// Интеграция OpenCode Browse: реальный Chrome пользователя.
//
// Browse — это MCP-сервер + расширение Chrome. Сервер говорит по
// stdio и управляет вкладками через relay и chrome.debugger. Для нас
// это второй браузерный путь:
//
//   browser.cdp_*   — отдельный временный профиль, без логинов;
//   browser.browse_* — настоящий Chrome, с логинами и расширениями.
//
// Переписывать Browse на Rust не нужно: у нас уже есть MCP-клиент,
// а сам Browse распространяется под MIT.
//
// Чего модуль не делает: не хранит пароли, не обходит капчу и не
// решает, что считать успехом. Это делает сценарий, который присылает
// модель; проверять результат она обязана сама.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{json, Map, Value};

use crate::documents::mcp::{Mcp, ToolReply};
use crate::permissions::Settings;
use super::{Tool, ToolError, ToolOutput, ToolSpec};

/// Сколько ждём обычный вызов Browse.
const ТАЙМАУТ: Duration = Duration::from_secs(30);
/// Сколько ждём `execute`: там может быть навигация и несколько шагов.
const ТАЙМАУТ_ВЫПОЛНЕНИЯ: Duration = Duration::from_secs(120);
const МАКСИМУМ_ВЫВОДА: usize = 24_000;

/// Один MCP-процесс на приложение. Перезапускать его на каждый вызов
/// значит терять текущую сессию и каждый раз ждать рукопожатия.
static MCP: Mutex<Option<Mcp>> = Mutex::new(None);

/// Где лежит `browse`. Порядок: явная переменная, обычные места
/// установки, PATH. Явная переменная нужна тестам и нестандартным
/// установкам.
pub fn найти() -> Option<PathBuf> {
    if let Some(путь) = std::env::var_os("ARKHA_BROWSE_EXE") {
        let путь = PathBuf::from(путь);
        if путь.is_file() {
            return Some(путь);
        }
    }
    let имя = if cfg!(windows) { "browse.exe" } else { "browse" };
    let кандидаты = [
        std::env::var_os("USERPROFILE")
            .map(|п| PathBuf::from(п).join(".local").join("bin").join(имя)),
        std::env::var_os("LOCALAPPDATA")
            .map(|п| PathBuf::from(п).join("Programs").join("browse").join(имя)),
    ];
    for кандидат in кандидаты.into_iter().flatten() {
        if кандидат.is_file() {
            return Some(кандидат);
        }
    }
    let path = std::env::var_os("PATH")?;
    for папка in std::env::split_paths(&path) {
        let кандидат = папка.join(имя);
        if кандидат.is_file() {
            return Some(кандидат);
        }
    }
    None
}

fn нет_browse() -> ToolError {
    ToolError::Missing(
        "Browse не установлен. Установите: irm https://opencode.ai/browse/install.ps1 | iex, \
         затем добавьте расширение в Chrome"
            .into(),
    )
}

/// Вызвать инструмент Browse. Если процесс умер, один раз поднимаем
/// заново: MCP-сервер — это отдельный процесс, и его смерть не должна
/// требовать перезапуска всего приложения.
fn вызвать(
    имя: &str,
    аргументы: Value,
    таймаут: Duration,
) -> Result<ToolReply, ToolError> {
    let exe = найти().ok_or_else(нет_browse)?;
    let mut замок = MCP
        .lock()
        .map_err(|_| ToolError::Refused("Browse занят".into()))?;
    if замок.is_none() {
        let mcp = Mcp::connect(&exe, &["mcp".to_string()]).map_err(|ошибка| {
            ToolError::Refused(format!("Browse не запустился: {ошибка}"))
        })?;
        *замок = Some(mcp);
    }

    let первый = {
        let mcp = замок.as_mut().expect("Browse только что создан");
        mcp.call_timeout(имя, аргументы.clone(), таймаут)
    };
    match первый {
        Ok(ответ) => Ok(ответ),
        Err(первая) => {
            *замок = None;
            let mut mcp = Mcp::connect(&exe, &["mcp".to_string()]).map_err(|ошибка| {
                ToolError::Refused(format!(
                    "Browse не перезапустился после «{первая}»: {ошибка}"
                ))
            })?;
            let ответ = mcp.call_timeout(имя, аргументы, таймаут);
            *замок = Some(mcp);
            ответ.map_err(|ошибка| {
                ToolError::Refused(format!("Browse: {первая}; повтор: {ошибка}"))
            })
        }
    }
}

/// Закрыть MCP-процесс. Relay и расширение остаются: они общие с
/// другими клиентами Browse.
pub fn закрыть() {
    if let Ok(mut замок) = MCP.lock() {
        if let Some(mut mcp) = замок.take() {
            mcp.stop();
        }
    }
}

fn ответ_текстом(reply: ToolReply, инструмент: &str) -> Result<String, ToolError> {
    if reply.is_error {
        return Err(ToolError::Refused(format!(
            "{инструмент}: {}",
            reply.text.trim()
        )));
    }
    let текст = reply.text.trim();
    if текст.is_empty() {
        return Ok("(пустой ответ)".into());
    }
    let красивый = serde_json::from_str::<Value>(текст)
        .ok()
        .and_then(|значение| serde_json::to_string_pretty(&значение).ok())
        .unwrap_or_else(|| текст.to_string());
    if красивый.chars().count() <= МАКСИМУМ_ВЫВОДА {
        return Ok(красивый);
    }
    let mut обрезано: String = красивый.chars().take(МАКСИМУМ_ВЫВОДА).collect();
    обрезано.push_str("\n… ответ Browse обрезан");
    Ok(обрезано)
}

fn объект() -> Map<String, Value> {
    Map::new()
}

pub struct BrowseExecute;

impl Tool for BrowseExecute {
    fn spec(&self, _s: &Settings) -> ToolSpec {
        ToolSpec {
            id: "browser.browse_execute".into(),
            capability: "browser.dom".into(),
            name: "Выполнить в реальном Chrome".into(),
            description: "Выполнить Playwright-код в настоящем Chrome пользователя через Browse. \
                          Сначала осмотрись: `const s = await snapshot();` — в ответе будут ref вида \
                          `e1`. Действуй по ref: `await ref(\"e1\").click()`. Проверяй результат: \
                          `page.url()`, новый `snapshot()`, заголовок. Если нужен человек (логин, \
                          2FA, капча, оплата) — `await handoff(\"что сделать\")`. Не решай капчу. \
                          Не вставляй пароли в код; для сохранённых секретов есть `secrets.type`. \
                          Вкладки открываются в реальном браузере, логины уже на месте."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "code": {
                        "type": "string",
                        "description": "Playwright JS: page, context, snapshot(), ref(), handoff(), files, secrets"
                    },
                    "timeout_ms": {
                        "type": "integer",
                        "minimum": 0,
                        "maximum": 300000,
                        "description": "бюджет всего скрипта, по умолчанию 60000"
                    },
                    "session": {
                        "type": "string",
                        "description": "id сессии Browse; без него — текущая"
                    },
                    "target_url": {
                        "type": "string",
                        "description": "выбрать уже открытую вкладку по части адреса"
                    },
                    "target_index": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "выбрать вкладку по индексу"
                    }
                },
                "required": ["code"]
            }),
            side_effecting: true,
        }
    }

    fn call(&self, args: &Value, _s: &Settings) -> Result<ToolOutput, ToolError> {
        let Some(code) = args.get("code").and_then(|значение| значение.as_str()) else {
            return Err(ToolError::BadArgs("нет поля code".into()));
        };
        if code.trim().is_empty() {
            return Err(ToolError::BadArgs("code пустой".into()));
        }

        let mut аргументы = объект();
        аргументы.insert("code".into(), json!(code));
        if let Some(значение) = args.get("timeout_ms").and_then(|з| з.as_u64()) {
            аргументы.insert("timeoutMs".into(), json!(значение));
        }
        if let Some(значение) = args.get("session").and_then(|з| з.as_str()) {
            аргументы.insert("session".into(), json!(значение));
        }
        if let Some(значение) = args.get("target_url").and_then(|з| з.as_str()) {
            аргументы.insert("targetUrl".into(), json!(значение));
        }
        if let Some(значение) = args.get("target_index").and_then(|з| з.as_u64()) {
            аргументы.insert("targetIndex".into(), json!(значение));
        }

        let бюджёт = args
            .get("timeout_ms")
            .and_then(|з| з.as_u64())
            .unwrap_or(60_000);
        let таймаут = if бюджёт == 0 {
            Duration::from_secs(300)
        } else {
            Duration::from_millis(бюджёт.max(30_000) + 30_000)
        };
        let ответ = вызвать("execute", Value::Object(аргументы), таймаут)?;
        let текст = ответ_текстом(ответ, "execute")?;
        Ok(ToolOutput::changed(
            текст,
            "browse.execute выполнен".to_string(),
        ))
    }
}

pub struct BrowseStatus;

impl Tool for BrowseStatus {
    fn spec(&self, _s: &Settings) -> ToolSpec {
        ToolSpec {
            id: "browser.browse_status".into(),
            capability: "windows.read".into(),
            name: "Состояние Browse".into(),
            description: "Показать, подключены ли relay, расширение Chrome и какие браузеры \
                          доступны. Вызывай перед первым browser.browse_execute, если не уверен, \
                          что Browse работает."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
            side_effecting: false,
        }
    }

    fn call(&self, _args: &Value, _s: &Settings) -> Result<ToolOutput, ToolError> {
        let ответ = вызвать("status", json!({}), ТАЙМАУТ)?;
        let текст = ответ_текстом(ответ, "status")?;
        Ok(ToolOutput::read(текст, "browse: статус".to_string()))
    }
}

pub struct BrowseSession;

impl Tool for BrowseSession {
    fn spec(&self, _s: &Settings) -> ToolSpec {
        ToolSpec {
            id: "browser.browse_session".into(),
            capability: "browser.dom".into(),
            name: "Сессия Browse".into(),
            description: "Управление сессиями Browse: list, new, use, reset, delete. \
                          Сессия — это набор вкладок; после задачи сделай reset или delete, \
                          чтобы не оставлять вкладки."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["list", "new", "use", "reset", "delete"]
                    },
                    "id": { "type": "string", "description": "id сессии" },
                    "allowed_origins": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "домены/источники, куда можно ходить"
                    },
                    "read_only": { "type": "boolean" },
                    "browser": { "type": "string", "description": "id браузера из status" }
                },
                "required": ["action"]
            }),
            side_effecting: true,
        }
    }

    fn call(&self, args: &Value, _s: &Settings) -> Result<ToolOutput, ToolError> {
        let Some(действие) = args.get("action").and_then(|з| з.as_str()) else {
            return Err(ToolError::BadArgs("нет поля action".into()));
        };
        let id = args.get("id").and_then(|з| з.as_str());
        let (имя, аргументы, изменение) = match действие {
            "list" => ("session_list", json!({}), false),
            "new" => {
                let mut аргументы = объект();
                if let Some(id) = id {
                    аргументы.insert("id".into(), json!(id));
                }
                if let Some(значения) = args.get("allowed_origins").and_then(|з| з.as_array()) {
                    аргументы.insert("allowedOrigins".into(), Value::Array(значения.clone()));
                }
                if let Some(значение) = args.get("read_only").and_then(|з| з.as_bool()) {
                    аргументы.insert("readOnly".into(), json!(значение));
                }
                if let Some(значение) = args.get("browser").and_then(|з| з.as_str()) {
                    аргументы.insert("browser".into(), json!(значение));
                }
                ("session_new", Value::Object(аргументы), true)
            }
            "use" => {
                let id = id.ok_or_else(|| ToolError::BadArgs("для use нужен id".into()))?;
                ("session_use", json!({"id": id}), true)
            }
            "reset" => {
                let mut аргументы = объект();
                if let Some(id) = id {
                    аргументы.insert("id".into(), json!(id));
                }
                ("session_reset", Value::Object(аргументы), true)
            }
            "delete" => {
                let mut аргументы = объект();
                if let Some(id) = id {
                    аргументы.insert("id".into(), json!(id));
                }
                ("session_delete", Value::Object(аргументы), true)
            }
            другое => {
                return Err(ToolError::BadArgs(format!(
                    "неизвестное действие «{другое}», нужно list/new/use/reset/delete"
                )))
            }
        };
        let ответ = вызвать(имя, аргументы, ТАЙМАУТ)?;
        let текст = ответ_текстом(ответ, имя)?;
        if изменение {
            Ok(ToolOutput::changed(текст, format!("browse: {имя}")))
        } else {
            Ok(ToolOutput::read(текст, format!("browse: {имя}")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn поиск_не_паникует() {
        let _ = найти();
    }

    /// Разрешения: выполнение — действие, статус — чтение. Если это
    /// перепутать, Browse начнёт работать без подтверждения или,
    /// наоборот, статус будет спрашивать разрешение.
    #[test]
    fn возможности_инструментов_правильные() {
        let s = Settings::default();
        let выполнение = BrowseExecute.spec(&s);
        assert_eq!(выполнение.capability, "browser.dom");
        assert!(выполнение.side_effecting);

        let статус = BrowseStatus.spec(&s);
        assert_eq!(статус.capability, "windows.read");
        assert!(!статус.side_effecting);

        let сессия = BrowseSession.spec(&s);
        assert_eq!(сессия.capability, "browser.dom");
        assert!(сессия.side_effecting);
    }

    /// При включённом browser.dom Browse разрешён, при выключенном —
    /// нет; статус остаётся чтением и работает.
    #[test]
    fn видимость_зависит_от_разрешения() {
        use crate::permissions::{CapabilityRule, Executor};

        let mut s = Settings::default();
        s.executor = Executor::Live;
        s.capabilities.insert(
            "browser.dom".into(),
            CapabilityRule { enabled: true, mode: None },
        );
        assert!(s.показывать("browser.dom"));
        assert!(s.allows("browser.dom").allowed);
        assert!(s.показывать("windows.read"));
        assert!(s.allows("windows.read").allowed);

        s.capabilities.insert(
            "browser.dom".into(),
            CapabilityRule { enabled: false, mode: None },
        );
        assert!(!s.показывать("browser.dom"));
        assert!(!s.allows("browser.dom").allowed);
        assert!(s.показывать("windows.read"));
        assert!(s.allows("windows.read").allowed);
    }

    #[test]
    fn пустой_код_отклоняется() {
        let ошибка = BrowseExecute
            .call(&json!({"code": "   "}), &Settings::default())
            .unwrap_err();
        assert!(matches!(ошибка, ToolError::BadArgs(_)), "{ошибка:?}");
    }

    #[test]
    fn неизвестное_действие_сессии_отклоняется() {
        let ошибка = BrowseSession
            .call(
                &json!({"action": "взорвать"}),
                &Settings::default(),
            )
            .unwrap_err();
        assert!(matches!(ошибка, ToolError::BadArgs(_)), "{ошибка:?}");
    }
}
