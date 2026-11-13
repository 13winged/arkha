//! Гейт браузерного модуля на живой странице.
//!
//! Проверяется не «функция вернула объект», а результат на странице:
//! снимок видит поле и кнопку, ввод меняет значение поля, нажатие
//! меняет заголовок вкладки.
//!
//! Прогон:
//! ```text
//! cargo test --test browser_gate -- --nocapture --test-threads=1
//! ```
//!
//! Нужен Chrome, Edge или Firefox. Модель не участвует: это проверка
//! инструментов, а не выбора инструмента моделью.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use serde_json::json;

use arkha_lib::permissions::Settings;
use arkha_lib::tools::Tool;
use arkha_lib::tools::browser;

const ДО: &str = "ArkHa Browser";
const ТЕКСТ: &str = "Титан квест Release";

fn страница() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/browser.html")
}

fn браузер() -> Option<PathBuf> {
    let кандидаты = [
        std::env::var("PROGRAMFILES")
            .ok()
            .map(|p| PathBuf::from(p).join(r"Google\Chrome\Application\chrome.exe")),
        std::env::var("LOCALAPPDATA")
            .ok()
            .map(|p| PathBuf::from(p).join(r"Google\Chrome\Application\chrome.exe")),
        std::env::var("PROGRAMFILES")
            .ok()
            .map(|p| PathBuf::from(p).join(r"Microsoft\Edge\Application\msedge.exe")),
        std::env::var("PROGRAMFILES")
            .ok()
            .map(|p| PathBuf::from(p).join(r"Mozilla Firefox\firefox.exe")),
    ];
    кандидаты.into_iter().flatten().find(|путь| путь.exists())
}

struct Браузер {
    ребёнок: Child,
}

impl Drop for Браузер {
    fn drop(&mut self) {
        let _ = Command::new("taskkill")
            .args(["/PID", &self.ребёнок.id().to_string(), "/T", "/F"])
            .output();
        let _ = self.ребёнок.wait();
    }
}

struct Профиль(PathBuf);

impl Drop for Профиль {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn url_файла(путь: &Path) -> String {
    let текст = путь.display().to_string().replace('\\', "/");
    format!("file:///{}", текст.replace(' ', "%20"))
}

fn открыть() -> (Браузер, Профиль) {
    let exe = браузер().expect("не найден Chrome, Edge или Firefox");
    let профиль = std::env::temp_dir().join(format!("arkha-browser-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&профиль);

    let ребёнок = Command::new(&exe)
        .arg(format!("--user-data-dir={}", профиль.display()))
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--new-window")
        .arg(url_файла(&страница()))
        .spawn()
        .expect("не удалось запустить браузер");

    if ждать_окна(ДО, 40).is_none() {
        let видимые = arkha_lib::tools::windows::видимые()
            .into_iter()
            .map(|о| о.заголовок)
            .collect::<Vec<_>>();
        let _ = Command::new("taskkill")
            .args(["/PID", &ребёнок.id().to_string(), "/T", "/F"])
            .output();
        panic!("окно «{ДО}» не появилось. Сейчас: {видимые:?}");
    }

    (Браузер { ребёнок }, Профиль(профиль))
}

fn ждать_окна(часть: &str, секунд: u64) -> Option<isize> {
    let край = Instant::now() + Duration::from_secs(секунд);
    loop {
        if let Some(окно) = arkha_lib::tools::windows::найти(часть)
            .into_iter()
            .find(|окно| окно.видимое)
        {
            return Some(окно.hwnd);
        }
        if Instant::now() > край {
            return None;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn ждать_заголовка(часть: &str, секунд: u64) -> bool {
    let край = Instant::now() + Duration::from_secs(секунд);
    loop {
        if arkha_lib::tools::windows::видимые()
            .iter()
            .any(|окно| окно.заголовок.contains(часть))
        {
            return true;
        }
        if Instant::now() > край {
            return false;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

#[test]
fn снимок_видит_поле_и_кнопку() {
    let (_браузер, _профиль) = открыть();
    let s = Settings::default();
    let снимок = browser::Снимок
        .call(&json!({ "title": ДО }), &s)
        .expect("снимок должен сняться");
    println!("{}", снимок.text);
    assert!(снимок.text.contains("поле ввода"), "{}", снимок.text);
    assert!(снимок.text.contains("Сохранить"), "{}", снимок.text);
    assert!(снимок.text.contains("Название"), "{}", снимок.text);
}

#[test]
fn текст_страницы_читается() {
    let (_браузер, _профиль) = открыть();
    let s = Settings::default();
    let текст = browser::Текст
        .call(&json!({ "title": ДО }), &s)
        .expect("текст страницы должен читаться");
    println!("{}", текст.text);
    assert!(текст.text.contains("ArkHa Browser"), "{}", текст.text);
    assert!(текст.text.contains("Название"), "{}", текст.text);
}

#[test]
fn ввод_и_нажатие_меняют_страницу() {
    let (_браузер, _профиль) = открыть();
    let s = Settings::default();

    let введено = browser::Ввести
        .call(
            &json!({
                "title": ДО,
                "text": ТЕКСТ,
                "target": "Название",
                "replace": true
            }),
            &s,
        )
        .expect("ввод должен пройти");
    println!("{}", введено.text);

    let нажато = browser::Нажать
        .call(&json!({ "title": ДО, "text": "Сохранить" }), &s)
        .expect("нажатие должно пройти");
    println!("{}", нажато.text);

    assert!(
        ждать_заголовка(ТЕКСТ, 15),
        "заголовок не сменился. Ввод, видимо, не дошёл до поля. Сейчас: {:?}",
        arkha_lib::tools::windows::видимые()
            .into_iter()
            .map(|о| о.заголовок)
            .collect::<Vec<_>>()
    );
}
