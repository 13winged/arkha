// HTTP и поиск в интернете.
//
// Инструменты для многоагентного deep researcher:
//   - web.fetch — открыть страницу по адресу и вернуть её текст
//   - web.search — поиск в интернете через SearXNG-инстанс
//
// Всё через reqwest (уже в зависимостях), rustls (TLS без OpenSSL).

use reqwest::blocking::Client;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{Tool, ToolError, ToolOutput, ToolSpec};
use crate::permissions::Settings;

/// HTTP клиент для чтения страниц. Один на вызов — без состояния.
fn http_client() -> Result<Client, ToolError> {
    Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| ToolError::Refused(format!("не удалось создать HTTP-клиент: {e}")))
}

/// Открыть страницу и вернуть её текст.
pub struct FetchPage;

impl Tool for FetchPage {
    fn spec(&self, s: &Settings) -> ToolSpec {
        let папки = super::папки_словами(s);
        ToolSpec {
            id: "web.fetch".into(),
            capability: "web.fetch".into(),
            name: "Прочитать страницу".into(),
            description: "Открывает страницу по адресу и возвращает её текст. \
                          Полезно для поиска информации, проверки документации. \
                          Адрес должен быть полным (https://...)."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "полный адрес страницы, например https://example.com"
                    }
                },
                "required": ["url"]
            }),
            side_effecting: false,
        }
    }

    fn call(&self, args: &Value, _s: &Settings) -> Result<ToolOutput, ToolError> {
        let Some(url) = args.get("url").and_then(|v| v.as_str()) else {
            return Err(ToolError::BadArgs("нет поля url".into()));
        };

        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(ToolError::BadArgs(
                "адрес должен начинаться с http:// или https://".into(),
            ));
        }

        let client = http_client()?;
        let response = client
            .get(url)
            .send()
            .map_err(|e| ToolError::Refused(format!("не удалось открыть страницу: {e}")))?;

        if !response.status().is_success() {
            return Err(ToolError::Refused(format!(
                "страница вернула статус {}: {}",
                response.status(),
                response.status().canonical_reason().unwrap_or("неизвестно")
            )));
        }

        let text = response
            .text()
            .map_err(|e| ToolError::Unreadable(format!("текст не читается: {e}")))?;

        // Обрезаем слишком длинные страницы
        let text = if text.len() > 50_000 {
            let mut truncated = text[..50_000].to_string();
            truncated.push_str("\n\n[... страница обрезана, слишком длинная ...]");
            truncated
        } else {
            text
        };

        let len = text.len();
        Ok(ToolOutput::read(
            text,
            format!("прочитана страница {} ({} байт)", url, len),
        ))
    }
}

/// Результат поиска.
#[derive(Debug, Clone, serde::Deserialize)]
struct SearchResult {
    title: String,
    url: String,
    content: String,
}

/// Ответ SearXNG.
#[derive(Debug, Clone, serde::Deserialize)]
struct SearXNGResponse {
    results: Vec<SearchResult>,
}

/// Поиск в интернете через SearXNG.
pub struct SearchWeb;

impl Tool for SearchWeb {
    fn spec(&self, _s: &Settings) -> ToolSpec {
        ToolSpec {
            id: "web.search".into(),
            capability: "web.search".into(),
            name: "Поиск в интернете".into(),
            description: "Ищет в интернете через SearXNG-инстанс. \
                          Нужен адрес инстанса (настраивается в настройках). \
                          Возвращает заголовки, ссылки и краткое описание."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "поисковый запрос"
                    },
                    "searxng_url": {
                        "type": "string",
                        "description": "адрес SearXNG-инстанса (если не задан, используется по умолчанию)"
                    }
                },
                "required": ["query"]
            }),
            side_effecting: false,
        }
    }

    fn call(&self, args: &Value, _s: &Settings) -> Result<ToolOutput, ToolError> {
        let Some(query) = args.get("query").and_then(|v| v.as_str()) else {
            return Err(ToolError::BadArgs("нет поля query".into()));
        };

        if query.is_empty() {
            return Err(ToolError::BadArgs("запрос пустой".into()));
        }

        let searxng_url = args
            .get("searxng_url")
            .and_then(|v| v.as_str())
            .unwrap_or("http://localhost:8080");

        let client = http_client()?;
        let encoded_query = query
            .replace(' ', "+")
            .replace('&', "%26")
            .replace('=', "%3D")
            .replace('?', "%3F");
        let url = format!("{}/search?q={}&format=json", searxng_url, encoded_query);
        let response = client
            .get(&url)
            .send()
            .map_err(|e| ToolError::Refused(format!("поиск не удался: {e}")))?;

        if !response.status().is_success() {
            return Err(ToolError::Refused(format!(
                "SearXNG вернул статус {}: {}",
                response.status(),
                response.status().canonical_reason().unwrap_or("неизвестно")
            )));
        }

        let searxng: SearXNGResponse = response
            .json()
            .map_err(|e| ToolError::Unreadable(format!("ответ не разобрать: {e}")))?;

        if searxng.results.is_empty() {
            return Ok(ToolOutput::read(
                "Ничего не найдено.".to_string(),
                "поиск не дал результатов".to_string(),
            ));
        }

        let mut текст = String::new();
        for (i, r) in searxng.results.iter().take(10).enumerate() {
            текст.push_str(&format!(
                "{}. **{}**\n   {}\n   {}\n\n",
                i + 1,
                r.title,
                r.url,
                r.content
            ));
        }

        Ok(ToolOutput::read(
            текст,
            format!("найдено {} результатов", searxng.results.len()),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetch_page_bad_url() {
        let e = FetchPage
            .call(
                &serde_json::json!({"url": "not-a-url"}),
                &Settings::default(),
            )
            .unwrap_err();
        assert!(matches!(e, ToolError::BadArgs(_)), "{e:?}");
    }

    #[test]
    fn search_empty_query() {
        let e = SearchWeb
            .call(
                &serde_json::json!({"query": ""}),
                &Settings::default(),
            )
            .unwrap_err();
        assert!(matches!(e, ToolError::BadArgs(_)), "{e:?}");
    }

    #[test]
    fn search_missing_query() {
        let e = SearchWeb
            .call(
                &serde_json::json!({}),
                &Settings::default(),
            )
            .unwrap_err();
        assert!(matches!(e, ToolError::BadArgs(_)), "{e:?}");
    }
}