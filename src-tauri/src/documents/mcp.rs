// Клиент MCP поверх stdio.
//
// Решение и числа — в docs/31-документы-через-mcp.md. Здесь только
// транспорт: запустить `*-cli mcp`, поздороваться, попросить список
// инструментов, вызвать один. Ни сетевого порта, ни библиотеки MCP:
// протокол простой, а лишняя зависимость в дереве, где ценность —
// вес, дороже сотни строк.
//
// Три решения, которые здесь приняты.
//
// **Чтение идёт в отдельном потоке.** У пайпа нет таймаута чтения, а
// зависший CLI не должен вешать агент навсегда. Поток складывает
// строки в канал, вызывающий ждёт с ограничением.
//
// **Отказ инструмента — результат, а не сбой.** `isError` в ответе
// значит «не вышло и почему»; это то, что нужно сказать модели, а не
// ошибка транспорта.
//
// **Процесс умирает вместе с клиентом.** `Drop` убивает ребёнка:
// иначе осиротевший CLI держит документы открытыми и переживает
// выход из приложения.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

/// Версия протокола, которую объявляем. Совпадает с той, что 11.11.2026
/// отдали все четыре CLI.
pub const PROTOCOL: &str = "2025-06-18";

/// Сколько ждём рукопожатия. Дольше — значит процесс не тот или завис.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

/// Сколько ждём ответа инструмента: OCR и большие PDF не мгновенны.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(60);

/// Инструмент на той стороне. Схема нам нужна не для показа модели, а
/// для проверки, что нужное имя вообще есть.
#[derive(Debug, Clone, PartialEq)]
pub struct RemoteTool {
    pub name: String,
    pub description: String,
    pub schema: Value,
}

/// Ответ инструмента в том виде, в каком его понимает модель.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolReply {
    pub text: String,
    pub is_error: bool,
}

/// Шов между протоколом и процессом. Отдельный типаж, чтобы проверять
/// рукопожатие без запуска чужих бинарей.
pub(crate) trait Transport {
    fn send(&mut self, line: &str) -> Result<(), String>;
    fn recv(&mut self, timeout: Duration) -> Result<String, String>;
    fn diagnostics(&self) -> String {
        String::new()
    }
    fn stop(&mut self) {}
}

/// Сессия JSON-RPC. Вызовы последовательные: MCP по stdio отвечает по
/// одному, а параллельные запросы в один пайп только путают порядок.
pub(crate) struct Session<T: Transport> {
    transport: T,
    next_id: u64,
}

impl<T: Transport> Session<T> {
    pub(crate) fn new(transport: T) -> Self {
        Self { transport, next_id: 1 }
    }

    pub(crate) fn initialize(&mut self) -> Result<Value, String> {
        let result = self.request(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL,
                "capabilities": {},
                "clientInfo": {
                    "name": "arkha",
                    "version": env!("CARGO_PKG_VERSION"),
                },
            }),
            HANDSHAKE_TIMEOUT,
        )?;
        // Уведомление обязательное: без него сервер считает сессию
        // незавершённой и может не ответить на tools/list.
        self.notify("notifications/initialized", json!({}))?;
        Ok(result)
    }

    pub(crate) fn tools(&mut self) -> Result<Vec<RemoteTool>, String> {
        let result = self.request("tools/list", json!({}), HANDSHAKE_TIMEOUT)?;
        let items = result.get("tools").and_then(Value::as_array).cloned().unwrap_or_default();
        Ok(items
            .into_iter()
            .filter_map(|tool| {
                let name = tool.get("name").and_then(Value::as_str)?.to_string();
                Some(RemoteTool {
                    name,
                    description: tool
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    schema: tool.get("inputSchema").cloned().unwrap_or(Value::Null),
                })
            })
            .collect())
    }

    pub(crate) fn call(&mut self, name: &str, args: Value) -> Result<ToolReply, String> {
        let result = self.request(
            "tools/call",
            json!({ "name": name, "arguments": args }),
            CALL_TIMEOUT,
        )?;
        Ok(reply_from(result))
    }

    pub(crate) fn stop(&mut self) {
        self.transport.stop();
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        let payload = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        self.transport.send(&payload.to_string())
    }

    fn request(&mut self, method: &str, params: Value, timeout: Duration) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        let payload = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        self.transport.send(&payload.to_string())?;

        loop {
            let line = match self.transport.recv(timeout) {
                Ok(line) => line,
                Err(error) => return Err(with_diagnostics(method, error, &self.transport)),
            };
            let value: Value = match serde_json::from_str(&line) {
                Ok(value) => value,
                Err(error) => {
                    return Err(format!("{method}: ответ не JSON: {error}"));
                }
            };
            // Уведомление или ответ на прошлый запрос. Пропускаем, а не
            // падаем: сервер имеет право присылать свои события.
            if value.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = value.get("error") {
                return Err(format!("{method}: {}", describe_error(error)));
            }
            return Ok(value.get("result").cloned().unwrap_or(Value::Null));
        }
    }
}

fn with_diagnostics(method: &str, error: String, transport: &impl Transport) -> String {
    let tail = transport.diagnostics();
    if tail.is_empty() {
        format!("{method}: {error}")
    } else {
        format!("{method}: {error} ({tail})")
    }
}

fn describe_error(error: &Value) -> String {
    let code = error.get("code").and_then(Value::as_i64);
    let message = error.get("message").and_then(Value::as_str).unwrap_or("без текста");
    match code {
        Some(code) => format!("{message} (код {code})"),
        None => message.to_string(),
    }
}

/// Текст ответа и признак отказа. Текст собирается из блоков
/// `content[].text`: у всех четырёх CLI он там.
fn reply_from(result: Value) -> ToolReply {
    let is_error = result.get("isError").and_then(Value::as_bool).unwrap_or(false);
    let mut parts: Vec<String> = Vec::new();
    if let Some(items) = result.get("content").and_then(Value::as_array) {
        for item in items {
            if let Some(text) = item.get("text").and_then(Value::as_str) {
                parts.push(text.to_string());
            }
        }
    }
    let text = if parts.is_empty() {
        // Ничего не сказал — показываем хотя бы структуру, иначе отказ
        // выглядит как пустая строка и модель пробует то же самое.
        result.to_string()
    } else {
        parts.join("\n")
    };
    ToolReply { text, is_error }
}

/// Процесс CLI, поднятый с пайпами.
pub struct ProcessTransport {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
    stderr: Arc<Mutex<VecDeque<String>>>,
}

impl ProcessTransport {
    pub fn spawn(exe: &Path, args: &[String]) -> Result<Self, String> {
        let mut command = Command::new(exe);
        command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        hide_console(&mut command);

        let mut child = command
            .spawn()
            .map_err(|error| format!("не удалось запустить {}: {error}", exe.display()))?;
        let stdin = child.stdin.take().ok_or("у процесса нет stdin")?;
        let stdout = child.stdout.take().ok_or("у процесса нет stdout")?;
        let stderr_pipe = child.stderr.take().ok_or("у процесса нет stderr")?;

        let (sender, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if sender.send(line).is_err() {
                    break;
                }
            }
        });

        let stderr = Arc::new(Mutex::new(VecDeque::new()));
        let sink = stderr.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr_pipe).lines() {
                let Ok(line) = line else { break };
                let mut sink = sink.lock().expect("stderr lock poisoned");
                if sink.len() == 20 {
                    sink.pop_front();
                }
                sink.push_back(line);
            }
        });

        Ok(Self { child, stdin, lines, stderr })
    }
}

impl Transport for ProcessTransport {
    fn send(&mut self, line: &str) -> Result<(), String> {
        writeln!(self.stdin, "{line}")
            .and_then(|()| self.stdin.flush())
            .map_err(|error| format!("процесс не принял запрос: {error}"))
    }

    fn recv(&mut self, timeout: Duration) -> Result<String, String> {
        match self.lines.recv_timeout(timeout) {
            Ok(line) => Ok(line),
            Err(RecvTimeoutError::Timeout) => {
                Err(format!("нет ответа за {} с", timeout.as_secs()))
            }
            Err(RecvTimeoutError::Disconnected) => Err("процесс закрыл поток".into()),
        }
    }

    fn diagnostics(&self) -> String {
        let sink = self.stderr.lock().expect("stderr lock poisoned");
        sink.iter().cloned().collect::<Vec<_>>().join(" | ")
    }

    fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for ProcessTransport {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Готовое подключение: рукопожатие уже прошло, инструменты прочитаны.
pub struct Mcp {
    session: Session<ProcessTransport>,
    tools: Vec<RemoteTool>,
}

impl Mcp {
    pub fn connect(exe: &Path, args: &[String]) -> Result<Self, String> {
        let transport = ProcessTransport::spawn(exe, args)?;
        let mut session = Session::new(transport);
        let info = session.initialize()?;
        let server = info.get("serverInfo").cloned().unwrap_or(Value::Null);
        tracing::info!(server = %server, "MCP: рукопожатие прошло");
        let tools = session.tools()?;
        Ok(Self { session, tools })
    }

    pub fn tools(&self) -> &[RemoteTool] {
        &self.tools
    }

    pub fn has_tool(&self, name: &str) -> bool {
        self.tools.iter().any(|tool| tool.name == name)
    }

    pub fn call(&mut self, name: &str, args: Value) -> Result<ToolReply, String> {
        self.session.call(name, args)
    }

    pub fn stop(&mut self) {
        self.session.stop();
    }
}

#[cfg(windows)]
fn hide_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_console(_command: &mut Command) {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Подставной сервер: запоминает, что у него спросили, и отдаёт
    /// заранее заготовленные строки.
    struct Scripted {
        sent: Vec<Value>,
        replies: VecDeque<String>,
    }

    impl Scripted {
        fn new(replies: &[Value]) -> Self {
            Self {
                sent: Vec::new(),
                replies: replies.iter().map(Value::to_string).collect(),
            }
        }
    }

    impl Transport for Scripted {
        fn send(&mut self, line: &str) -> Result<(), String> {
            let value: Value = serde_json::from_str(line).map_err(|error| error.to_string())?;
            self.sent.push(value);
            Ok(())
        }

        fn recv(&mut self, _timeout: Duration) -> Result<String, String> {
            self.replies.pop_front().ok_or_else(|| "процесс закрыл поток".into())
        }
    }

    fn handshake_and_tools(tools: Value) -> Scripted {
        Scripted::new(&[
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": {
                    "protocolVersion": PROTOCOL,
                    "serverInfo": { "name": "wordcraft", "version": "0.17.0" },
                    "capabilities": { "tools": {} }
                }
            }),
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "result": { "tools": tools }
            }),
        ])
    }

    /// Порядок запросов важен: сначала initialize, потом уведомление,
    /// потом уже список. Иначе сервер отвечает отказом на tools/list.
    #[test]
    fn рукопожатие_идёт_первым_и_завершается_уведомлением() {
        let mut session = Session::new(handshake_and_tools(json!([])));
        session.initialize().expect("рукопожатие");
        session.tools().expect("список инструментов");

        let sent = &session.transport.sent;
        assert_eq!(sent[0]["method"], "initialize");
        assert_eq!(sent[0]["params"]["protocolVersion"], PROTOCOL);
        assert_eq!(sent[1]["method"], "notifications/initialized");
        assert!(sent[1].get("id").is_none(), "уведомление не имеет id");
        assert_eq!(sent[2]["method"], "tools/list");
    }

    #[test]
    fn инструменты_разбираются_с_именем_описанием_и_схемой() {
        let tools = json!([
            {
                "name": "get_text",
                "description": "Return the document text",
                "inputSchema": { "type": "object", "properties": {} }
            },
            { "description": "без имени" }
        ]);
        let mut session = Session::new(handshake_and_tools(tools));
        session.initialize().unwrap();
        let tools = session.tools().unwrap();

        assert_eq!(tools.len(), 1, "инструмент без имени не должен появляться");
        assert_eq!(tools[0].name, "get_text");
        assert_eq!(tools[0].description, "Return the document text");
        assert_eq!(tools[0].schema["type"], "object");
    }

    /// Ошибка протокола — это сбой, а не ответ инструмента. Модели
    /// нужно знать имя метода и код, иначе она не поймёт, что менять.
    #[test]
    fn ошибка_протокола_называет_метод_и_код() {
        let mut session = Session::new(Scripted::new(&[json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": { "code": -32601, "message": "unknown method" }
        })]));

        let error = session.initialize().expect_err("должен быть отказ");
        assert!(error.contains("initialize"), "{error}");
        assert!(error.contains("unknown method"), "{error}");
        assert!(error.contains("-32601"), "{error}");
    }

    /// Отказ инструмента приходит с `isError: true` и текстом. Это
    /// результат, который надо показать модели, а не падение клиента.
    #[test]
    fn отказ_инструмента_остаётся_результатом() {
        let mut session = Session::new(Scripted::new(&[json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "isError": true,
                "content": [{ "type": "text", "text": "outside the read root" }]
            }
        })]));

        let reply = session.call("open_document", json!({ "path": "C:\\Windows\\win.ini" })).unwrap();
        assert!(reply.is_error);
        assert_eq!(reply.text, "outside the read root");
    }

    #[test]
    fn вызов_передаёт_имя_и_аргументы() {
        let mut session = Session::new(Scripted::new(&[json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": { "content": [{ "type": "text", "text": "ok" }] }
        })]));

        session.call("read_range", json!({ "range": "A1:B2" })).unwrap();
        let sent = &session.transport.sent[0];
        assert_eq!(sent["method"], "tools/call");
        assert_eq!(sent["params"]["name"], "read_range");
        assert_eq!(sent["params"]["arguments"]["range"], "A1:B2");
    }

    /// Сервер имеет право присылать свои уведомления. Ответ на прошлый
    /// запрос тоже может прийти — пропускаем, а не принимаем за свой.
    #[test]
    fn чужие_сообщения_пропускаются() {
        let mut session = Session::new(Scripted::new(&[
            json!({ "jsonrpc": "2.0", "method": "notifications/progress", "params": {} }),
            json!({ "jsonrpc": "2.0", "id": 999, "result": { "чужой": true } }),
            json!({ "jsonrpc": "2.0", "id": 1, "result": { "tools": [] } }),
        ]));

        let tools = session.tools().expect("список после чужих сообщений");
        assert!(tools.is_empty());
    }

    #[test]
    fn закрытый_поток_не_превращается_в_пустой_ответ() {
        let mut session = Session::new(Scripted::new(&[]));
        let error = session.initialize().expect_err("пустой поток — отказ");
        assert!(error.contains("закрыл поток"), "{error}");
    }
}
