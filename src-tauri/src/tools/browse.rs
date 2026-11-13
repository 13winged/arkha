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

/// Что сказать, если relay не отвечает.
const ПОДСКАЗКА_RELAY: &str = "Browse не отвечает: relay на 127.0.0.1:19988 не запущен. \
     Relay поднимается сам при первом вызове; если этого не случилось, откройте Chrome \
     с установленным расширением Browse или запустите `browse serve` для отладки.";

/// Что сказать, если relay есть, а Chrome не подключён.
const ПОДСКАЗКА_CHROME: &str = "Chrome с расширением Browse не подключён. Откройте Chrome, \
     где установлено расширение Browse, — оно переподключится само, и повторите вызов.";

fn объект() -> Map<String, Value> {
    Map::new()
}

/// Подключён ли браузер по тексту `status`.
///
/// Ответ Browse — JSON вида `{"status": {"connected": true, ...}}`.
/// `None` означает «формат незнаком», и тогда execute не блокируется:
/// пусть скажет своё, вместо того чтобы гадать.
fn состояние_подключения(текст: &str) -> Option<bool> {
    let значение: Value = serde_json::from_str(текст).ok()?;
    значение.get("status")?.get("connected")?.as_bool()
}

/// Причина отказа — недоступный relay, а не ошибка сценария.
fn это_ошибка_relay(текст: &str) -> bool {
    let нижний = текст.to_lowercase();
    нижний.contains("relay is not reachable") || нижний.contains("relay is not running")
}

/// Короткая выжимка из статуса: без простыни всего JSON.
fn краткий_статус(текст: &str) -> String {
    let Ok(значение) = serde_json::from_str::<Value>(текст) else {
        return текст.trim().to_string();
    };
    let статус = значение.get("status");
    let состояние = статус
        .and_then(|с| с.get("pairingState"))
        .and_then(|с| с.as_str())
        .unwrap_or("неизвестно");
    let шаг = статус
        .and_then(|с| с.get("nextStep"))
        .and_then(|с| с.as_str())
        .unwrap_or("");
    if шаг.is_empty() {
        format!("Состояние: {состояние}.")
    } else {
        format!("Состояние: {состояние}. {шаг}")
    }
}

/// Проверить связь до запуска сценария.
///
/// Без этого первый `execute` ждал бы таймаут и возвращал сырой текст
/// вроде «relay is not reachable». Проверка дешёвая и локальная.
fn проверить_подключение() -> Result<(), ToolError> {
    match вызвать("status", json!({}), ТАЙМАУТ) {
        Ok(ответ) => match подсказка_по_статусу(&ответ) {
            Some(подсказка) => Err(ToolError::Refused(подсказка)),
            None => Ok(()),
        },
        // MCP-процесс не поднялся: `вызвать` уже дал внятную причину.
        Err(_) => Ok(()),
    }
}

/// Что не так со связью по ответу `status`; `None` — можно работать.
fn подсказка_по_статусу(ответ: &ToolReply) -> Option<String> {
    if ответ.is_error {
        return это_ошибка_relay(&ответ.text).then(|| ПОДСКАЗКА_RELAY.to_string());
    }
    match состояние_подключения(&ответ.text) {
        Some(false) => Some(format!(
            "{ПОДСКАЗКА_CHROME}\n\n{}",
            краткий_статус(&ответ.text)
        )),
        // Незнакомый формат не блокирует execute: точную ошибку
        // даст сам сценарий.
        _ => None,
    }
}

/// Заменить сырой отказ Browse на подсказку, если причина — связь.
fn уточнить(ошибка: ToolError) -> ToolError {
    match &ошибка {
        ToolError::Refused(текст) if это_ошибка_relay(текст) => {
            ToolError::Refused(ПОДСКАЗКА_RELAY.into())
        }
        _ => ошибка,
    }
}

/// Аргументы MCP-инструмента `execute` из аргументов нашего инструмента.
/// Вынесено отдельно, чтобы проверять без запуска Browse.
fn аргументы_выполнения(args: &Value) -> Result<Value, ToolError> {
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
    Ok(Value::Object(аргументы))
}

/// Имя MCP-инструмента сессии, его аргументы и меняет ли он состояние.
fn аргументы_сессии(args: &Value) -> Result<(&'static str, Value, bool), ToolError> {
    let Some(действие) = args.get("action").and_then(|з| з.as_str()) else {
        return Err(ToolError::BadArgs("нет поля action".into()));
    };
    let id = args.get("id").and_then(|з| з.as_str());
    match действие {
        "list" => Ok(("session_list", json!({}), false)),
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
            Ok(("session_new", Value::Object(аргументы), true))
        }
        "use" => {
            let id = id.ok_or_else(|| ToolError::BadArgs("для use нужен id".into()))?;
            Ok(("session_use", json!({"id": id}), true))
        }
        "reset" => {
            let mut аргументы = объект();
            if let Some(id) = id {
                аргументы.insert("id".into(), json!(id));
            }
            Ok(("session_reset", Value::Object(аргументы), true))
        }
        "delete" => {
            let mut аргументы = объект();
            if let Some(id) = id {
                аргументы.insert("id".into(), json!(id));
            }
            Ok(("session_delete", Value::Object(аргументы), true))
        }
        другое => Err(ToolError::BadArgs(format!(
            "неизвестное действие «{другое}», нужно list/new/use/reset/delete"
        ))),
    }
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
        let аргументы = аргументы_выполнения(args)?;
        let бюджёт = args
            .get("timeout_ms")
            .and_then(|з| з.as_u64())
            .unwrap_or(60_000);
        let таймаут = if бюджёт == 0 {
            Duration::from_secs(300)
        } else {
            Duration::from_millis(бюджёт.max(30_000) + 30_000)
        };
        проверить_подключение()?;
        let ответ = вызвать("execute", аргументы, таймаут).map_err(уточнить)?;
        let текст = ответ_текстом(ответ, "execute").map_err(уточнить)?;
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
        let (имя, аргументы, изменение) = аргументы_сессии(args)?;
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

    #[test]
    fn аргументы_выполнения_не_добавляют_лишнего() {
        let полный = аргументы_выполнения(&json!({
            "code": "return 1",
            "timeout_ms": 1000,
            "session": "s1",
            "target_url": "example.com",
            "target_index": 2
        }))
        .unwrap();
        let объект = полный.as_object().unwrap();
        assert_eq!(объект["code"], "return 1");
        assert_eq!(объект["timeoutMs"], 1000);
        assert_eq!(объект["session"], "s1");
        assert_eq!(объект["targetUrl"], "example.com");
        assert_eq!(объект["targetIndex"], 2);
        assert_eq!(объект.len(), 5);

        let короткий = аргументы_выполнения(&json!({"code": "return 1"})).unwrap();
        assert_eq!(короткий.as_object().unwrap().len(), 1);
        assert!(аргументы_выполнения(&json!({"code": "  "})).is_err());
    }

    #[test]
    fn сессии_переводятся_в_имена_mcp() {
        let (имя, _, изменение) = аргументы_сессии(&json!({"action": "list"})).unwrap();
        assert_eq!(имя, "session_list");
        assert!(!изменение);

        let (имя, аргументы, изменение) = аргументы_сессии(&json!({
            "action": "new",
            "id": "work",
            "allowed_origins": ["example.com"],
            "read_only": true,
            "browser": "chrome"
        }))
        .unwrap();
        assert_eq!(имя, "session_new");
        assert!(изменение);
        assert_eq!(аргументы["id"], "work");
        assert_eq!(аргументы["allowedOrigins"][0], "example.com");
        assert_eq!(аргументы["readOnly"], true);
        assert_eq!(аргументы["browser"], "chrome");

        let (имя, аргументы, _) = аргументы_сессии(&json!({"action": "use", "id": "work"})).unwrap();
        assert_eq!(имя, "session_use");
        assert_eq!(аргументы["id"], "work");

        assert!(аргументы_сессии(&json!({"action": "use"})).is_err());
        assert!(аргументы_сессии(&json!({"action": "??"})).is_err());
    }

    #[test]
    fn ответ_browse_не_теряет_ошибку_и_режет_длинное() {
        let ошибка = ответ_текстом(
            ToolReply { text: "нет связи".into(), is_error: true },
            "execute",
        )
        .unwrap_err();
        assert!(matches!(ошибка, ToolError::Refused(_)), "{ошибка:?}");

        let json = ответ_текстом(
            ToolReply { text: "{\"a\":1}".into(), is_error: false },
            "status",
        )
        .unwrap();
        assert!(json.contains("\"a\": 1"), "{json}");

        let длинный = "x".repeat(МАКСИМУМ_ВЫВОДА + 100);
        let обрезанный = ответ_текстом(
            ToolReply { text: длинный, is_error: false },
            "execute",
        )
        .unwrap();
        assert!(обрезанный.contains("ответ Browse обрезан"));
    }

    #[test]
    fn статус_подключения_читается() {
        assert_eq!(
            состояние_подключения(r#"{"status":{"connected":true}}"#),
            Some(true)
        );
        assert_eq!(
            состояние_подключения(r#"{"status":{"connected":false}}"#),
            Some(false)
        );
        assert_eq!(состояние_подключения("{}"), None);
        assert_eq!(состояние_подключения(r#"{"status":{}}"#), None);
        assert_eq!(состояние_подключения("не json"), None);
    }

    #[test]
    fn краткий_статус_берёт_шаг_и_состояние() {
        let текст = concat!(
            r#"{"status":{"pairingState":"paired-offline","#,
            r#""nextStep":"Open the browser profile"}}"#
        );
        let краткий = краткий_статус(текст);
        assert!(краткий.contains("paired-offline"), "{краткий}");
        assert!(краткий.contains("Open the browser profile"), "{краткий}");
        assert_eq!(краткий_статус("просто текст"), "просто текст");
    }

    #[test]
    fn ошибка_relay_узнаётся_в_любом_регистре() {
        assert!(это_ошибка_relay("BROWSE RELAY IS NOT REACHABLE"));
        assert!(это_ошибка_relay("relay is not running"));
        assert!(!это_ошибка_relay("element not found"));
    }

    #[test]
    fn отказ_relay_заменяется_подсказкой() {
        let сырой = ToolError::Refused(
            "status: browse relay is not reachable at http://127.0.0.1:19988".into(),
        );
        match уточнить(сырой) {
            ToolError::Refused(текст) => {
                assert!(текст.contains("relay на 127.0.0.1:19988"), "{текст}")
            }
            другое => panic!("ожидался Refused, а не {другое:?}"),
        }

        let сценарная = ToolError::Refused("execute: page.goto упал".into());
        match уточнить(сценарная) {
            ToolError::Refused(текст) => assert_eq!(текст, "execute: page.goto упал"),
            другое => panic!("ожидался Refused, а не {другое:?}"),
        }
    }

    #[test]
    fn подсказка_по_статусу_различает_три_случая() {
        // Relay мёртв: status вернул ошибку с известной причиной.
        let мёртв = ToolReply {
            text: "browse relay is not reachable at http://127.0.0.1:19988".into(),
            is_error: true,
        };
        let подсказка = подсказка_по_статусу(&мёртв).expect("должна быть подсказка");
        assert!(подсказка.contains("relay на 127.0.0.1:19988"), "{подсказка}");

        // Relay жив, Chrome не подключён.
        let offline = ToolReply {
            text: r#"{"status":{"connected":false,"pairingState":"paired-offline","nextStep":"Open the browser profile"}}"#
                .into(),
            is_error: false,
        };
        let подсказка = подсказка_по_статусу(&offline).expect("должна быть подсказка");
        assert!(подсказка.contains("расширением Browse"), "{подсказка}");
        assert!(подсказка.contains("paired-offline"), "{подсказка}");

        // Всё подключено и чужая ошибка status не блокируют execute.
        let готов = ToolReply {
            text: r#"{"status":{"connected":true}}"#.into(),
            is_error: false,
        };
        assert!(подсказка_по_статусу(&готов).is_none());
        let иная = ToolReply { text: "timeout".into(), is_error: true };
        assert!(подсказка_по_статусу(&иная).is_none());
    }
}
