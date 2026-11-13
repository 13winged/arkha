//! Живой гейт интеграции OpenCode Browse.
//!
//! Проверяется не «MCP ответил», а доступ к настоящему Chrome:
//! статус relay/расширения и выполнение Playwright-кода на странице.
//!
//! Запуск (нужен установленный Browse и подключённый Chrome):
//! ```text
//! cargo test --test browse_gate -- --ignored --nocapture --test-threads=1
//! ```

use std::time::Duration;

use serde_json::json;

use arkha_lib::permissions::Settings;
use arkha_lib::tools::Tool;
use arkha_lib::tools::browse;

struct Закрыть;

impl Drop for Закрыть {
    fn drop(&mut self) {
        browse::закрыть();
    }
}

#[test]
#[ignore = "нужен установленный Browse и подключённый Chrome"]
fn статус_и_страница_в_настоящем_chrome() {
    assert!(
        browse::найти().is_some(),
        "browse.exe не найден: установите irm https://opencode.ai/browse/install.ps1 | iex"
    );
    let _закрыть = Закрыть;
    let настройки = Settings::default();

    let статус = browse::BrowseStatus
        .call(&json!({}), &настройки)
        .expect("browse status должен ответить");
    println!("{}", статус.text);
    assert!(
        статус.text.contains("\"connected\": true") || статус.text.contains("\"connected\":true"),
        "нет подключения к Chrome:\n{}",
        статус.text
    );

    let ответ = browse::BrowseExecute
        .call(
            &json!({
                "code": "await page.goto('https://example.com', { waitUntil: 'domcontentloaded', timeout: 20000 }); return { url: page.url(), title: await page.title() };",
                "timeout_ms": 45000
            }),
            &настройки,
        )
        .expect("browse execute должен выполниться");
    println!("{}", ответ.text);
    assert!(ответ.text.contains("example.com"), "{}", ответ.text);
    assert!(
        ответ.text.contains("Example Domain"),
        "страница не прочиталась: {}",
        ответ.text
    );

    // Не оставляем вкладку после проверки.
    let _ = browse::BrowseSession
        .call(&json!({"action": "reset"}), &настройки);
    std::thread::sleep(Duration::from_millis(200));
}
