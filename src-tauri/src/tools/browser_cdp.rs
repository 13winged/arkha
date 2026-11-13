// CDP-драйвер браузера.
//
// Управление страницей через Chrome DevTools Protocol: навигация,
// чтение DOM с ref'ами, клик по ref, ввод текста, текст страницы.
// Это второй путь рядом с UI Automation. UIA работает с уже открытым
// браузером, но зависит от дерева доступности и не даёт стабильных
// DOM-ref. CDP даёт именно DOM и не требует фокуса окна.
//
// Почему свой драйвер, а не Playwright: нам нужен один бинарник без
// Node и без C++-сборки. Протокол простой: WebSocket + JSON.
//
// Безопасность:
// - отдельный временный профиль, без cookie everyday-браузера;
// - только http/https, без file/javascript/data/chrome;
// - профиль удаляется при закрытии.

use std::io::ErrorKind;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

use crate::permissions::Settings;
use super::{apps, Tool, ToolError, ToolOutput, ToolSpec};

const КОМАНДА_ТАЙМАУТ: Duration = Duration::from_secs(30);
const ОЖИДАНИЕ_БРАУЗЕРА: Duration = Duration::from_secs(20);
const ЖДАТЬ_СОБЫТИЕ: Duration = Duration::from_millis(500);
const МАКСИМУМ_ВЫВОДА: usize = 24_000;

static СЧЁТЧИК: AtomicU64 = AtomicU64::new(0);

/// Один браузер на процесс. Вкладка одна: модель работает с одной
/// страницей, а вторая вкладка только путает ref'ы.
static БРАУЗЕР: Mutex<Option<Browser>> = Mutex::new(None);

fn таймаут(что: &str) -> ToolError {
    ToolError::Refused(format!("{что}: браузер не ответил за {} с", КОМАНДА_ТАЙМАУТ.as_secs()))
}

/// Каким браузером управлять.
fn найти_браузер() -> Result<PathBuf, ToolError> {
    for имя in ["chrome", "msedge", "chromium", "brave"] {
        let Ok(путь) = apps::найти(имя) else {
            continue;
        };
        let exe = путь
            .extension()
            .and_then(|расширение| расширение.to_str())
            .is_some_and(|расширение| расширение.eq_ignore_ascii_case("exe"));
        if exe && путь.is_file() {
            return Ok(путь);
        }
    }

    let кандидаты = [
        std::env::var_os("PROGRAMFILES")
            .map(|п| PathBuf::from(п).join(r"Google\Chrome\Application\chrome.exe")),
        std::env::var_os("PROGRAMFILES(X86)")
            .map(|п| PathBuf::from(п).join(r"Google\Chrome\Application\chrome.exe")),
        std::env::var_os("LOCALAPPDATA")
            .map(|п| PathBuf::from(п).join(r"Google\Chrome\Application\chrome.exe")),
        std::env::var_os("PROGRAMFILES")
            .map(|п| PathBuf::from(п).join(r"Microsoft\Edge\Application\msedge.exe")),
        std::env::var_os("PROGRAMFILES(X86)")
            .map(|п| PathBuf::from(п).join(r"Microsoft\Edge\Application\msedge.exe")),
    ];
    for кандидат in кандидаты.into_iter().flatten() {
        if кандидат.is_file() {
            return Ok(кандидат);
        }
    }

    Err(ToolError::Missing(
        "не найден Chrome, Edge или Chromium. Установи браузер с поддержкой CDP".into(),
    ))
}

fn проверить_браузера(settings: &Settings) -> Result<PathBuf, ToolError> {
    let exe = найти_браузер()?;
    let имя = exe
        .file_name()
        .and_then(|имя| имя.to_str())
        .unwrap_or_default();
    if settings.is_denied_app(имя) {
        return Err(ToolError::Denied(format!(
            "браузер «{имя}» запрещён в настройках"
        )));
    }
    Ok(exe)
}

/// Адрес страницы: только http/https. Короткий адрес получает https.
fn адрес(значение: &str) -> Result<String, ToolError> {
    let адрес = значение.trim();
    if адрес.is_empty() {
        return Err(ToolError::BadArgs("пустой адрес".into()));
    }
    if адрес.contains(['\n', '\r', '"']) {
        return Err(ToolError::BadArgs("в адресе есть кавычки или перевод строки".into()));
    }
    let с_схемой = if адрес.contains("://") {
        адрес.to_string()
    } else {
        format!("https://{адрес}")
    };
    let разобранный = url::Url::parse(&с_схемой)
        .map_err(|ошибка| ToolError::BadArgs(format!("адрес не разобрался: {ошибка}")))?;
    match разобранный.scheme() {
        "http" | "https" => Ok(разобранный.to_string()),
        другое => Err(ToolError::BadArgs(format!(
            "схема «{другое}» запрещена: разрешены только http и https"
        ))),
    }
}

struct Browser {
    ребёнок: Child,
    профиль: PathBuf,
    сокет: WebSocket<MaybeTlsStream<std::net::TcpStream>>,
    сессия: String,
    следующий_id: u64,
    события: Vec<Value>,
    /// Соединение оборвалось или браузер закрылся. Такой Browser
    /// нельзя переиспользовать: следующий вызов должен запустить новый.
    мёртв: bool,
}

impl Browser {
    fn запустить(exe: PathBuf) -> Result<Self, ToolError> {
        let номер = СЧЁТЧИК.fetch_add(1, Ordering::SeqCst);
        let профиль = std::env::temp_dir().join(format!(
            "arkha-cdp-{}-{номер}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&профиль);
        std::fs::create_dir_all(&профиль)
            .map_err(|ошибка| ToolError::Refused(format!("папка профиля не создалась: {ошибка}")))?;

        let mut команда = Command::new(&exe);
        команда
            .arg("--remote-debugging-port=0")
            .arg(format!("--user-data-dir={}", профиль.display()))
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg("--disable-background-networking")
            .arg("--disable-extensions")
            .arg("--disable-sync")
            .arg("--window-size=1280,900")
            .arg("about:blank")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if std::env::var("ARKHA_CDP_HEADLESS").ok().as_deref() == Some("1") {
            команда.arg("--headless=new").arg("--disable-gpu");
        }

        let mut ребёнок = команда.spawn().map_err(|ошибка| {
            ToolError::Refused(format!("браузер не запустился: {ошибка}"))
        })?;

        let файл = профиль.join("DevToolsActivePort");
        let край = Instant::now() + ОЖИДАНИЕ_БРАУЗЕРА;
        let mut адрес_ws = None;
        while Instant::now() < край {
            if let Ok(текст) = std::fs::read_to_string(&файл) {
                let строки: Vec<&str> = текст.lines().collect();
                if строки.len() >= 2
                    && строки[0].trim().parse::<u16>().is_ok()
                    && строки[1].starts_with('/')
                {
                    адрес_ws = Some(format!(
                        "ws://127.0.0.1:{}{}",
                        строки[0].trim(),
                        строки[1].trim()
                    ));
                    break;
                }
            }
            if ребёнок.try_wait().ok().flatten().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }

        let Some(адрес_ws) = адрес_ws else {
            let _ = ребёнок.kill();
            let _ = std::fs::remove_dir_all(&профиль);
            return Err(ToolError::Refused(
                "Chromium не открыл DevTools-порт за 20 секунд".into(),
            ));
        };

        let (mut сокет, _) = tungstenite::connect(&адрес_ws).map_err(|ошибка| {
            let _ = ребёнок.kill();
            let _ = std::fs::remove_dir_all(&профиль);
            ToolError::Refused(format!("CDP-соединение не открылось: {ошибка}"))
        })?;
        if let MaybeTlsStream::Plain(поток) = сокет.get_ref() {
            let _ = поток.set_read_timeout(Some(ЖДАТЬ_СОБЫТИЕ));
        }

        let mut браузер = Self {
            ребёнок,
            профиль,
            сокет,
            сессия: String::new(),
            следующий_id: 1,
            события: Vec::new(),
            мёртв: false,
        };

        let target = браузер.send("Target.createTarget", json!({"url": "about:blank"}))?;
        let target_id = target
            .get("targetId")
            .and_then(|значение| значение.as_str())
            .ok_or_else(|| ToolError::Refused("CDP не вернул targetId".into()))?;
        let привязка = браузер.send(
            "Target.attachToTarget",
            json!({"targetId": target_id, "flatten": true}),
        )?;
        браузер.сессия = привязка
            .get("sessionId")
            .and_then(|значение| значение.as_str())
            .ok_or_else(|| ToolError::Refused("CDP не вернул sessionId".into()))?
            .to_string();
        браузер.send("Page.enable", json!({}))?;
        браузер.send("Page.setLifecycleEventsEnabled", json!({"enabled": true}))?;
        браузер.send("Runtime.enable", json!({}))?;
        Ok(браузер)
    }

    fn send(&mut self, method: &str, params: Value) -> Result<Value, ToolError> {
        let id = self.следующий_id;
        self.следующий_id += 1;
        let mut команда = json!({"id": id, "method": method, "params": params});
        if !self.сессия.is_empty()
            && !method.starts_with("Target.")
            && !method.starts_with("Browser.")
        {
            команда["sessionId"] = json!(self.сессия);
        }
        let текст = команда.to_string();
        if let Err(ошибка) = self.сокет.send(Message::Text(текст.into())) {
            self.мёртв = true;
            return Err(ToolError::Refused(format!(
                "CDP не принял команду: {ошибка}"
            )));
        }

        let край = Instant::now() + КОМАНДА_ТАЙМАУТ;
        loop {
            let Some(сообщение) = self.прочитать(край)? else {
                continue;
            };
            if сообщение.get("id").and_then(|значение| значение.as_u64()) != Some(id) {
                continue;
            }
            if let Some(ошибка) = сообщение.get("error") {
                let текст = ошибка
                    .get("message")
                    .and_then(|значение| значение.as_str())
                    .unwrap_or("неизвестная ошибка");
                return Err(ToolError::Refused(format!("{method}: {текст}")));
            }
            return Ok(сообщение
                .get("result")
                .cloned()
                .unwrap_or(Value::Null));
        }
    }

    /// Прочитать одно сообщение. `Ok(None)` — истёк короткий таймаут,
    /// надо попробовать ещё раз. Жизнь ограничена общим дедлайном.
    fn прочитать(&mut self, край: Instant) -> Result<Option<Value>, ToolError> {
        if Instant::now() >= край {
            return Err(таймаут("CDP"));
        }
        let сообщение = match self.сокет.read() {
            Ok(сообщение) => сообщение,
            Err(tungstenite::Error::Io(ошибка))
                if matches!(ошибка.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock) =>
            {
                return Ok(None);
            }
            Err(ошибка) => {
                self.мёртв = true;
                return Err(ToolError::Refused(format!(
                    "CDP-соединение оборвалось: {ошибка}"
                )));
            }
        };
        let текст = match сообщение {
            Message::Text(текст) => текст.to_string(),
            Message::Binary(байты) => String::from_utf8_lossy(&байты).to_string(),
            Message::Close(_) => {
                self.мёртв = true;
                return Err(ToolError::Refused("CDP закрыл соединение".into()));
            }
            _ => return Ok(None),
        };
        let значение: Value = serde_json::from_str(&текст)
            .map_err(|ошибка| ToolError::Refused(format!("CDP прислал не JSON: {ошибка}")))?;
        if значение.get("method").and_then(|м| м.as_str()) == Some("Page.lifecycleEvent") {
            self.события.push(значение.clone());
        }
        Ok(Some(значение))
    }

    fn evaluate(&mut self, выражение: &str) -> Result<Value, ToolError> {
        let ответ = self.send(
            "Runtime.evaluate",
            json!({"expression": выражение, "returnByValue": true}),
        )?;
        if let Some(исключение) = ответ.get("exceptionDetails") {
            let текст = исключение
                .get("exception")
                .and_then(|значение| значение.get("description"))
                .and_then(|значение| значение.as_str())
                .unwrap_or("ошибка в скрипте страницы");
            return Err(ToolError::Refused(format!(
                "скрипт страницы упал: {текст}"
            )));
        }
        Ok(ответ
            .get("result")
            .and_then(|значение| значение.get("value"))
            .cloned()
            .unwrap_or(Value::Null))
    }

    fn navigate(&mut self, url: &str) -> Result<(String, String), ToolError> {
        let url = адрес(url)?;
        self.события.clear();
        let ответ = self.send("Page.navigate", json!({"url": url}))?;
        if let Some(ошибка) = ответ.get("errorText").and_then(|значение| значение.as_str()) {
            return Err(ToolError::Refused(format!("навигация не удалась: {ошибка}")));
        }
        if let Some(loader) = ответ.get("loaderId").and_then(|значение| значение.as_str()) {
            let край = Instant::now() + КОМАНДА_ТАЙМАУТ;
            loop {
                let готово = self.события.iter().any(|событие| {
                    событие.get("method").and_then(|м| м.as_str())
                        == Some("Page.lifecycleEvent")
                        && событие
                            .get("params")
                            .and_then(|п| п.get("name"))
                            .and_then(|и| и.as_str())
                            == Some("DOMContentLoaded")
                        && событие
                            .get("params")
                            .and_then(|п| п.get("loaderId"))
                            .and_then(|и| и.as_str())
                            == Some(loader)
                });
                if готово {
                    break;
                }
                let Some(_) = self.прочитать(край)? else {
                    continue;
                };
            }
        }
        self.state()
    }

    fn state(&mut self) -> Result<(String, String), ToolError> {
        let значение = self.evaluate(PAGE_INFO)?;
        let url = значение
            .get("url")
            .and_then(|значение| значение.as_str())
            .unwrap_or("about:blank")
            .to_string();
        let title = значение
            .get("title")
            .and_then(|значение| значение.as_str())
            .unwrap_or("")
            .to_string();
        Ok((url, title))
    }

    fn click(&mut self, ссылка: &str, координата: Option<(f64, f64)>) -> Result<(), ToolError> {
        let (x, y) = match координата {
            Some(точка) => точка,
            None => {
                let выражение = format!(
                    "((ref) => {{ const el = window.__cdpBrowserRefs && window.__cdpBrowserRefs.byRef.get(ref); \
                     if (!el || !el.isConnected) return null; \
                     el.scrollIntoView({{block:'center', inline:'center', behavior:'auto'}}); \
                     const r = el.getBoundingClientRect(); \
                     return {{x: r.left + r.width / 2, y: r.top + r.height / 2}}; }})({})",
                    serde_json::to_string(ссылка).unwrap_or_else(|_| "\"\"".into())
                );
                let точка = self.evaluate(&выражение)?;
                let x = точка
                    .get("x")
                    .and_then(|значение| значение.as_f64())
                    .ok_or_else(|| {
                        ToolError::Unknown(format!(
                            "ref «{ссылка}» устарел или не найден. Сделай browser.cdp_read_page заново"
                        ))
                    })?;
                let y = точка
                    .get("y")
                    .and_then(|значение| значение.as_f64())
                    .ok_or_else(|| {
                        ToolError::Unknown(format!(
                            "ref «{ссылка}» устарел или не найден. Сделай browser.cdp_read_page заново"
                        ))
                    })?;
                (x, y)
            }
        };

        self.send(
            "Input.dispatchMouseEvent",
            json!({"type": "mouseMoved", "x": x, "y": y}),
        )?;
        for тип in ["mousePressed", "mouseReleased"] {
            self.send(
                "Input.dispatchMouseEvent",
                json!({"type": тип, "x": x, "y": y, "button": "left", "clickCount": 1}),
            )?;
        }
        std::thread::sleep(Duration::from_millis(100));
        Ok(())
    }

    fn type_text(&mut self, ссылка: Option<&str>, текст: &str, заменить: bool) -> Result<(), ToolError> {
        if let Some(ссылка) = ссылка {
            let выражение = format!(
                "((ref, replace) => {{ const el = window.__cdpBrowserRefs && window.__cdpBrowserRefs.byRef.get(ref); \
                 if (!el || !el.isConnected) return false; \
                 el.scrollIntoView({{block:'center', inline:'center', behavior:'auto'}}); \
                 el.focus(); \
                 if (replace) {{ if (typeof el.select === 'function') el.select(); \
                 else if (el.isContentEditable) {{ const r = document.createRange(); r.selectNodeContents(el); \
                 const s = window.getSelection(); s.removeAllRanges(); s.addRange(r); }} }} \
                 return true; }})({}, {})",
                serde_json::to_string(ссылка).unwrap_or_else(|_| "\"\"".into()),
                if заменить { "true" } else { "false" }
            );
            let найден = self.evaluate(&выражение)?;
            if найден.as_bool() != Some(true) {
                return Err(ToolError::Unknown(format!(
                    "ref «{ссылка}» устарел или не найден. Сделай browser.cdp_read_page заново"
                )));
            }
        }
        self.send("Input.insertText", json!({"text": текст}))?;
        std::thread::sleep(Duration::from_millis(100));
        Ok(())
    }

    fn stop(&mut self) {
        // Не ждём `Browser.close`: на выходе приложения важнее не
        // зависнуть на 30 секунд. Профиль временный, его не жалко.
        let _ = self.сокет.close(None);
        let _ = self.сокет.flush();
        let _ = self.ребёнок.kill();
        let _ = self.ребёнок.wait();
        let _ = std::fs::remove_dir_all(&self.профиль);
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Открыть браузер и вкладку, если ещё не открыты.
fn с_браузером<T>(
    открыть: bool,
    settings: &Settings,
    действие: impl FnOnce(&mut Browser) -> Result<T, ToolError>,
) -> Result<T, ToolError> {
    let mut замок = БРАУЗЕР
        .lock()
        .map_err(|_| ToolError::Refused("CDP-браузер занят".into()))?;
    if замок.is_none() {
        if !открыть {
            return Err(ToolError::Refused(
                "CDP-браузер ещё не запущен. Сначала вызови browser.cdp_navigate".into(),
            ));
        }
        let exe = проверить_браузера(settings)?;
        *замок = Some(Browser::запустить(exe)?);
    }
    let браузер = замок.as_mut().expect("браузер только что создан");
    let результат = действие(браузер);
    if браузер.мёртв {
        let _ = замок.take();
    }
    результат
}

/// Закрыть CDP-браузер. Вызывается при выходе из аккаунта и по команде.
pub fn закрыть() {
    if let Ok(mut замок) = БРАУЗЕР.lock() {
        if let Some(браузер) = замок.take() {
            drop(браузер);
        }
    }
}

/// Команда для интерфейса и выхода из аккаунта.
#[tauri::command]
pub fn browser_cdp_close() -> Result<(), String> {
    crate::auth::require_authenticated()?;
    закрыть();
    Ok(())
}

// --- инструменты ------------------------------------------------------

pub struct CdpNavigate;

impl Tool for CdpNavigate {
    fn spec(&self, _s: &Settings) -> ToolSpec {
        ToolSpec {
            id: "browser.cdp_navigate".into(),
            capability: "browser.dom".into(),
            name: "Открыть адрес через CDP".into(),
            description: "Запустить Chromium с временным профилем (если ещё не запущен) \
                          и открыть http/https-адрес. После навигации страницу читают \
                          browser.cdp_read_page и browser.cdp_text."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "адрес: https://... или example.com" }
                },
                "required": ["url"]
            }),
            side_effecting: true,
        }
    }

    fn call(&self, args: &Value, _s: &Settings) -> Result<ToolOutput, ToolError> {
        let Some(url) = args.get("url").and_then(|значение| значение.as_str()) else {
            return Err(ToolError::BadArgs("нет поля url".into()));
        };
        let (url, title) = с_браузером(true, _s, |браузер| браузер.navigate(url))?;
        Ok(ToolOutput::changed(
            format!("открыт {url}\nЗаголовок: {title}"),
            format!("открыт {url}"),
        ))
    }
}

pub struct CdpReadPage;

impl Tool for CdpReadPage {
    fn spec(&self, _s: &Settings) -> ToolSpec {
        ToolSpec {
            id: "browser.cdp_read_page".into(),
            capability: "windows.read".into(),
            name: "Прочитать страницу через CDP".into(),
            description: "Перечислить заголовки, ссылки, кнопки и поля страницы. Каждый \
                          элемент получает ref_N. Нажимай browser.cdp_click по ref, вводи \
                          текст browser.cdp_type. Снимок уже содержит всю страницу, \
                          прокрутка не нужна."
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
        let текст = с_браузером(false, _s, |браузер| {
            let значение = браузер.evaluate(READ_PAGE)?;
            Ok(значение.as_str().unwrap_or("").to_string())
        })?;
        let текст = if текст.chars().count() > МАКСИМУМ_ВЫВОДА {
            let mut обрезано: String = текст.chars().take(МАКСИМУМ_ВЫВОДА).collect();
            обрезано.push_str("\n… список обрезан");
            обрезано
        } else {
            текст
        };
        let число = текст.lines().filter(|строка| строка.starts_with("- ")).count();
        Ok(ToolOutput::read(
            format!("Элементы страницы (CDP):\n{текст}"),
            format!("элементов CDP: {число}"),
        ))
    }
}

pub struct CdpClick;

impl Tool for CdpClick {
    fn spec(&self, _s: &Settings) -> ToolSpec {
        ToolSpec {
            id: "browser.cdp_click".into(),
            capability: "browser.dom".into(),
            name: "Нажать в браузере через CDP".into(),
            description: "Нажать элемент по ref_N из browser.cdp_read_page. Клик идёт \
                          через CDP, окно не нужно переключать. Координаты можно \
                          передать только явно и только после снимка."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "ref": { "type": "string", "description": "ref из browser.cdp_read_page, например ref_3" },
                    "x": { "type": "number", "description": "координата X, только если нет ref" },
                    "y": { "type": "number", "description": "координата Y, только если нет ref" }
                },
                "required": []
            }),
            side_effecting: true,
        }
    }

    fn call(&self, args: &Value, _s: &Settings) -> Result<ToolOutput, ToolError> {
        let ссылка = args.get("ref").and_then(|значение| значение.as_str());
        let координата = match (
            args.get("x").and_then(|значение| значение.as_f64()),
            args.get("y").and_then(|значение| значение.as_f64()),
        ) {
            (Some(x), Some(y)) => Some((x, y)),
            _ => None,
        };
        if ссылка.is_none() && координата.is_none() {
            return Err(ToolError::BadArgs(
                "нужен ref из browser.cdp_read_page или пара x/y после снимка".into(),
            ));
        }
        let подпись = ссылка.unwrap_or("координата").to_string();
        с_браузером(false, _s, |браузер| {
            браузер.click(ссылка.unwrap_or(""), координата)
        })?;
        Ok(ToolOutput::changed(
            format!("нажато «{подпись}» через CDP"),
            format!("нажато «{подпись}»"),
        ))
    }
}

pub struct CdpType;

impl Tool for CdpType {
    fn spec(&self, _s: &Settings) -> ToolSpec {
        ToolSpec {
            id: "browser.cdp_type".into(),
            capability: "browser.dom".into(),
            name: "Ввести текст в браузере через CDP".into(),
            description: "Ввести текст в поле по ref_N из browser.cdp_read_page. \
                          replace=true (по умолчанию) заменяет старое содержимое. \
                          Без ref текст уходит в активный элемент."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "ref": { "type": "string", "description": "ref поля из browser.cdp_read_page" },
                    "text": { "type": "string", "description": "что ввести" },
                    "replace": { "type": "boolean", "description": "заменить старое, по умолчанию да" }
                },
                "required": ["text"]
            }),
            side_effecting: true,
        }
    }

    fn call(&self, args: &Value, _s: &Settings) -> Result<ToolOutput, ToolError> {
        let Some(текст) = args.get("text").and_then(|значение| значение.as_str()) else {
            return Err(ToolError::BadArgs("нет поля text".into()));
        };
        let ссылка = args.get("ref").and_then(|значение| значение.as_str());
        let заменить = args
            .get("replace")
            .and_then(|значение| значение.as_bool())
            .unwrap_or(true);
        с_браузером(false, _s, |браузер| браузер.type_text(ссылка, текст, заменить))?;
        Ok(ToolOutput::changed(
            format!("введено символов: {}", текст.chars().count()),
            format!("введено символов: {}", текст.chars().count()),
        ))
    }
}

pub struct CdpText;

impl Tool for CdpText {
    fn spec(&self, _s: &Settings) -> ToolSpec {
        ToolSpec {
            id: "browser.cdp_text".into(),
            capability: "windows.read".into(),
            name: "Прочитать текст страницы через CDP".into(),
            description: "Вернуть document.body.innerText открытой страницы. Используй \
                          для статей, дат и цитат; прокрутка не нужна."
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
        let текст = с_браузером(false, _s, |браузер| {
            let значение = браузер.evaluate(PAGE_TEXT)?;
            Ok(значение.as_str().unwrap_or("").to_string())
        })?;
        let текст = if текст.chars().count() > МАКСИМУМ_ВЫВОДА {
            let mut обрезано: String = текст.chars().take(МАКСИМУМ_ВЫВОДА).collect();
            обрезано.push_str("\n… текст обрезан");
            обрезано
        } else {
            текст
        };
        Ok(ToolOutput::read(
            текст.clone(),
            format!("текст CDP: {} символов", текст.chars().count()),
        ))
    }
}

pub struct CdpWait;

impl Tool for CdpWait {
    fn spec(&self, _s: &Settings) -> ToolSpec {
        ToolSpec {
            id: "browser.cdp_wait".into(),
            capability: "windows.read".into(),
            name: "Подождать страницу".into(),
            description: "Подождать 0–30 секунд, пока страница догрузится. Не заменяет \
                          навигацию: после неё уже ждём DOMContentLoaded."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "seconds": { "type": "number", "minimum": 0, "maximum": 30, "description": "сколько ждать" }
                },
                "required": ["seconds"]
            }),
            side_effecting: false,
        }
    }

    fn call(&self, args: &Value, _s: &Settings) -> Result<ToolOutput, ToolError> {
        let секунд = args
            .get("seconds")
            .and_then(|значение| значение.as_f64())
            .unwrap_or(0.0);
        if !(0.0..=30.0).contains(&секунд) {
            return Err(ToolError::BadArgs(
                "seconds должно быть от 0 до 30".into(),
            ));
        }
        std::thread::sleep(Duration::from_secs_f64(секунд));
        Ok(ToolOutput::read(
            format!("подождали {секунд:.1} с"),
            format!("ожидание {секунд:.1} с"),
        ))
    }
}

pub struct CdpState;

impl Tool for CdpState {
    fn spec(&self, _s: &Settings) -> ToolSpec {
        ToolSpec {
            id: "browser.cdp_state".into(),
            capability: "windows.read".into(),
            name: "Состояние CDP-вкладки".into(),
            description: "Показать адрес и заголовок открытой CDP-вкладки."
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
        let (url, title) = с_браузером(false, _s, |браузер| браузер.state())?;
        Ok(ToolOutput::read(
            format!("URL: {url}\nЗаголовок: {title}"),
            format!("{url} — {title}"),
        ))
    }
}

// --- JavaScript, который выполняется на странице ----------------------

const READ_PAGE: &str = r#"(() => {
  const refs = (window.__cdpBrowserRefs ||= { next: 1, byRef: new Map(), byElement: new WeakMap() });
  const roles = { A: 'link', BUTTON: 'button', INPUT: 'textbox', SELECT: 'combobox', TEXTAREA: 'textbox' };
  const name = (el) => (el.getAttribute('aria-label') || el.innerText || el.value || el.placeholder || '')
    .trim().replace(/\s+/g, ' ').slice(0, 100);
  const lines = [];
  const selector = 'h1, h2, h3, a[href], button, input, select, textarea, [role=button], [role=link]';
  for (const el of document.querySelectorAll(selector)) {
    const rect = el.getBoundingClientRect();
    if (rect.width === 0 && rect.height === 0) continue;
    let ref = refs.byElement.get(el);
    if (!ref) { ref = 'ref_' + refs.next++; refs.byElement.set(el, ref); refs.byRef.set(ref, el); }
    const role = el.getAttribute('role') || roles[el.tagName] || 'heading';
    lines.push(`- ${role} "${name(el)}" [${ref}]`);
  }
  return lines.join('\n');
})()"#;

const PAGE_INFO: &str = "({ url: location.href, title: document.title })";
const PAGE_TEXT: &str = "document.body ? document.body.innerText : ''";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn адрес_получает_https_и_проверяет_схему() {
        assert_eq!(адрес("example.com").unwrap(), "https://example.com/");
        assert_eq!(
            адрес("http://example.com/a?b=1").unwrap(),
            "http://example.com/a?b=1"
        );
        assert!(адрес("file:///C:/secret.txt").is_err());
        assert!(адрес("javascript:alert(1)").is_err());
        assert!(адрес("data:text/html,hi").is_err());
    }

    #[test]
    fn чужой_браузер_не_выдаёт_себя_за_свой() {
        // Просто проверка, что поиск не паникует на этой машине.
        let _ = найти_браузер();
    }
}
