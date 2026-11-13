//! Гейт CDP-драйвера на живой странице.
//!
//! Проверяется не «функция вернула объект», а DOM: навигация, чтение
//! с ref, ввод в поле, клик по ref и смена заголовка. Браузер —
//! Chrome/Edge, запускается headless, чтобы тест не отвлекал.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;

use arkha_lib::permissions::Settings;
use arkha_lib::tools::Tool;
use arkha_lib::tools::browser_cdp;

const СТРАНИЦА: &str = r#"<!doctype html>
<html lang="ru">
<head><meta charset="utf-8"><title>ArkHa CDP</title></head>
<body>
  <h1>ArkHa CDP</h1>
  <input id="name" aria-label="Название" value="старое">
  <button onclick="document.title = document.getElementById('name').value + ' — сохранено'">Сохранить</button>
</body>
</html>"#;

struct Сервер {
    адрес: String,
    стоп: Arc<AtomicBool>,
    поток: Option<std::thread::JoinHandle<()>>,
}

impl Сервер {
    fn запустить() -> Self {
        let слушатель = TcpListener::bind("127.0.0.1:0").expect("порт для теста");
        слушатель.set_nonblocking(true).expect("неблокирующий слушатель");
        let адрес = слушатель.local_addr().expect("адрес").to_string();
        let стоп = Arc::new(AtomicBool::new(false));
        let флаг = Arc::clone(&стоп);
        let поток = std::thread::spawn(move || {
            while !флаг.load(Ordering::SeqCst) {
                match слушатель.accept() {
                    Ok((mut поток, _)) => {
                        let _ = поток.set_read_timeout(Some(Duration::from_secs(1)));
                        let mut буфер = [0u8; 4096];
                        let _ = поток.read(&mut буфер);
                        let тело = СТРАНИЦА.as_bytes();
                        let ответ = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
                             Content-Length: {}\r\nConnection: close\r\n\r\n",
                            тело.len()
                        );
                        let _ = поток.write_all(ответ.as_bytes());
                        let _ = поток.write_all(тело);
                        let _ = поток.flush();
                    }
                    Err(ошибка) if ошибка.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(30));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            адрес: format!("http://{адрес}/"),
            стоп,
            поток: Some(поток),
        }
    }
}

impl Drop for Сервер {
    fn drop(&mut self) {
        self.стоп.store(true, Ordering::SeqCst);
        if let Some(поток) = self.поток.take() {
            let _ = поток.join();
        }
    }
}

struct Закрыть;

impl Drop for Закрыть {
    fn drop(&mut self) {
        browser_cdp::закрыть();
    }
}

fn ссылка(текст: &str, подпись: &str) -> String {
    let строка = текст
        .lines()
        .find(|строка| строка.contains(подпись) && строка.contains('['))
        .unwrap_or_else(|| panic!("в снимке нет «{подпись}»:\n{текст}"));
    строка
        .split('[')
        .nth(1)
        .and_then(|часть| часть.split(']').next())
        .expect("ref в квадратных скобках")
        .to_string()
}

#[test]
fn навигация_чтение_ввод_и_клик() {
    // Безопасно: тест один, окружение процесса больше никто не читает
    // одновременно. Так драйвер не показывает окно на машине.
    unsafe {
        std::env::set_var("ARKHA_CDP_HEADLESS", "1");
    }
    let _закрыть = Закрыть;
    let сервер = Сервер::запустить();
    let настройки = Settings::default();

    let переход = browser_cdp::CdpNavigate
        .call(&json!({ "url": сервер.адрес }), &настройки)
        .expect("навигация должна пройти");
    println!("{}", переход.text);
    assert!(переход.text.contains("ArkHa CDP"), "{}", переход.text);

    let снимок = browser_cdp::CdpReadPage
        .call(&json!({}), &настройки)
        .expect("чтение страницы должно пройти");
    println!("{}", снимок.text);
    assert!(снимок.text.contains("Название"), "{}", снимок.text);
    assert!(снимок.text.contains("Сохранить"), "{}", снимок.text);

    let текст = browser_cdp::CdpText
        .call(&json!({}), &настройки)
        .expect("текст страницы должен прочитаться");
    println!("{}", текст.text);
    assert!(текст.text.contains("ArkHa CDP"), "{}", текст.text);

    let поле = ссылка(&снимок.text, "Название");
    browser_cdp::CdpType
        .call(
            &json!({ "ref": поле, "text": "Титан квест Release", "replace": true }),
            &настройки,
        )
        .expect("ввод должен пройти");

    let кнопка = ссылка(&снимок.text, "Сохранить");
    browser_cdp::CdpClick
        .call(&json!({ "ref": кнопка }), &настройки)
        .expect("клик должен пройти");

    let край = Instant::now() + Duration::from_secs(10);
    let mut состояние = String::new();
    while Instant::now() < край {
        состояние = browser_cdp::CdpState
            .call(&json!({}), &настройки)
            .expect("состояние должно читаться")
            .text;
        if состояние.contains("Титан квест Release") {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    println!("{состояние}");
    assert!(
        состояние.contains("Титан квест Release"),
        "заголовок не сменился: {состояние}"
    );
}
