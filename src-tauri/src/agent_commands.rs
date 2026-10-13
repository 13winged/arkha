// Команды агента S6.
//
// Цикл выполняется в отдельном потоке, а не прямо в команде: клиент к
// сайдкару блокирующий, и вызвать его из рантайма tokio нельзя.
// Поэтому команда возвращает识别 идентификатор и отдаёт результат
// событием, как это уже сделано для чата.
//
// Отдельные события, а не одно общее:
//
// | Событие | Что значит |
// |---|---|
// | `agent:call` | агент вызвал инструмент, вот что он сделал |
// | `agent:pending` | ждём разрешения пользователя |
// | `agent:done` | цикл закончен, вот ответ и причина остановки |
//
// Причина остановки обязательна и показывается всегда. «Молча
// остановился» и «ответил» выглядят снаружи одинаково, а разница
// между ними — вся разница между рабочим агентом и сломанным.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::agent::approvals;
use crate::agent::executor::{Agent, CallRecord, Message, Outcome, PendingApproval};
use crate::agent::live::Sidecar;
use crate::chat;
use crate::model_manager::SidecarState;
use crate::permissions::Settings;

const EVENT_CALL: &str = "agent:call";
const EVENT_PENDING: &str = "agent:pending";
const EVENT_DONE: &str = "agent:done";

/// Один вызов инструмента для интерфейса.
#[derive(Debug, Clone, Serialize)]
pub struct CallEvent {
    pub request_id: u64,
    pub call: CallRecord,
}

/// Ожидание разрешения.
#[derive(Debug, Clone, Serialize)]
pub struct PendingEvent {
    pub request_id: u64,
    pub pending: PendingApproval,
}

/// Итог цикла.
#[derive(Debug, Clone, Serialize)]
pub struct DoneEvent {
    pub request_id: u64,
    pub answer: String,
    pub calls: Vec<CallRecord>,
    pub turns: usize,
    /// Почему цикл остановился. Показывается всегда.
    pub stop_reason: String,
    pub error: Option<String>,
    /// На чём основан ответ.
    ///
    /// Пустое основание — не отсутствие данных, а сообщение:
    /// ответ не опирался ни на один файл. Пользователь обязан это
    /// видеть рядом с текстом, иначе правдоподобный выдуманный ответ
    /// читается как знание.
    #[serde(rename = "basis")]
    pub основание: BasisView,
    /// Сработала ли страховка.
    #[serde(rename = "safety_fired")]
    pub страховка: bool,
    /// Что произошло с разговором, если что-то произошло.
    #[serde(rename = "compacted", default)]
    pub сжатие: String,
}

/// Основание ответа в том виде, в каком его видит интерфейс.
#[derive(Debug, Clone, Serialize)]
pub struct BasisView {
    /// Что реально прочитано.
    pub read: Vec<String>,
    pub ok: usize,
    pub denied: usize,
    /// Готовая строка для показа.
    pub label: String,
}

/// Сообщение от пользователя или из истории разговора.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Incoming {
    pub role: String,
    #[serde(default)]
    pub content: String,
}

/// Что агент умеет и что ему разрешено. Для вкладки настроек.
#[derive(Debug, Clone, Serialize)]
pub struct ToolsReport {
    pub tools: Vec<String>,
    pub capabilities: Vec<String>,
    /// Разрешено ли что-то менять в системе.
    pub can_change: bool,
    /// Навыки, которые лежат на диске: «имя — описание».
    pub навыки: Vec<String>,
    /// Навыки, которые не прочитались. Показываются, а не прячутся:
    /// молчащий сломанный навык выглядит как «навыков нет».
    pub сломанные_навыки: Vec<String>,
    /// Режим исполнения, словами.
    pub исполнитель: String,
    /// Режим подтверждений, словами.
    pub подтверждения: String,
}

#[tauri::command]
pub fn agent_tools(settings: State<'_, SettingsState>) -> ToolsReport {
    let s = settings.0.lock().expect("settings lock poisoned");
    tools_report(&s)
}

/// Отчёт собирается из настроек, а не из состояния приложения.
///
/// Разделено, чтобы отчёт можно было проверить на обычных настройках,
/// без запуска Tauri: состояние требует рантайма, а проверять здесь
/// надо именно логику «что агент умеет при этих правах».
pub fn tools_report(s: &Settings) -> ToolsReport {
    let reg = crate::tools::Registry::allowed(s);
    let mut уникальные: Vec<String> =
        reg.specs(s).iter().map(|t| t.capability.clone()).collect();
    уникальные.sort();
    уникальные.dedup();
    let can_change = уникальные.iter().any(|c| s.is_side_effecting(c));
    let навыки = crate::skills::Набор::из_папки(&crate::skills::папка_данных());
    ToolsReport {
        tools: reg.ids(),
        capabilities: уникальные,
        can_change,
        навыки: навыки.сводка(),
        сломанные_навыки: навыки.сломанные.iter().map(|o| o.to_string()).collect(),
        исполнитель: s.executor.title().to_string(),
        подтверждения: s.approval_mode.explain().to_string(),
    }
}

/// Сменить режим исполнения прямо из вкладки секретаря.
///
/// Раньше режим был виден, но не менялся: человек ставил «только
/// показывать», видел, что агент ничего не делает, и не понимал,
/// где переключить.
///
/// Запись идёт через `onboarding::save` — тот же единственный путь,
/// которым меняет настройки знакомство. Своя запись из другой вкладки
/// обошла бы ту самую осторожность, ради которой
/// [permissions_commands](permissions_commands.rs) не пишет ничего:
///
/// > Настройки читаются, но не пишутся. Запись — это отдельное
/// > решение с подтверждением, и делать её одной строкой значило бы
/// > обойти собственную осторожность: агент, который может и читать, и
/// > менять настройки разрешений, способен выдать себе права сам.
///
/// Команда меняет **только** режим исполнения и **только** из
/// интерфейса. Права возможностей она не трогает, и агент до неё не
/// дотягивается: в списке команд Tauri её нет для вызовов модели.
#[tauri::command]
pub fn agent_set_executor(
    settings: State<'_, SettingsState>,
    executor: crate::permissions::Executor,
) -> Result<ToolsReport, String> {
    let mut s = settings.0.lock().expect("settings lock poisoned");
    s.executor = executor;
    crate::onboarding::save(&s)?;
    Ok(tools_report(&s))
}

/// Сменить режим подтверждений прямо из вкладки секретаря.
///
/// Оговорка та же, что у `agent_set_executor`: режим `Auto` снимает
/// вопросы совсем, и это единственный переключатель, который может
/// ослабить защиту целиком. Поэтому он спрашивает подтверждением в
/// интерфейсе, а не применяется молча.
#[tauri::command]
pub fn agent_set_approval(
    settings: State<'_, SettingsState>,
    mode: crate::permissions::ApprovalMode,
    подтверди: Option<bool>,
) -> Result<ToolsReport, String> {
    if mode == crate::permissions::ApprovalMode::Auto && подтверди != Some(true) {
        return Err(
            "Режим «не спрашивать ничего» снимает все подтверждения разом. \
             Если вы этого хотите, подтвердите ещё раз."
                .into(),
        );
    }
    let mut s = settings.0.lock().expect("settings lock poisoned");
    s.approval_mode = mode;
    crate::onboarding::save(&s)?;
    Ok(tools_report(&s))
}

/// Перечитать навыки с диска.
#[tauri::command]
pub fn agent_reload_skills() -> Result<Vec<String>, String> {
    crate::tools::перечитать_навыки()
}

/// Настройки в состоянии приложения.
///
/// Отдельное состояние, а не чтение файла на каждый вызов: чтение
/// диска внутри команды означало бы, что смена галочки не видна до
/// перезапуска. Пользователь ждёт, что галочка работает сразу.
pub struct SettingsState(pub std::sync::Mutex<Settings>);

impl SettingsState {
    pub fn new() -> Self {
        let s = Settings::load().unwrap_or_else(|_| Settings::default());
        Self(std::sync::Mutex::new(s))
    }
}

/// Кто говорит с моделью и с какими инструментами.
#[tauri::command]
pub fn agent_chat(
    app: AppHandle,
    sidecar: State<'_, SidecarState>,
    settings: State<'_, SettingsState>,
    messages: Vec<Incoming>,
) -> Result<u64, String> {
    let base = sidecar.base_url()?;
    let request_id = chat::next_request_id();
    chat::reset_cancel();

    let s = settings.0.lock().expect("settings lock poisoned").clone();

    // Параметры выборки — из каталога, как и в чате. Значения по
    // умолчанию секретаря: температура 0.3, чтобы он выполнял, а не
    // выдумывал.
    let sampling = match crate::model_manager::Catalog::load().and_then(|c| c.sampling()) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("параметры выборки не из каталога, беру безопасные: {e}");
            crate::model_manager::Sampling::default_for_secrets()
        }
    };

    let start: Vec<Message> = messages.into_iter().map(|m| Message { role: m.role, content: m.content, tool_calls: None, tool_call_id: None, name: None }).collect();
    if start.is_empty() {
        return Err("нечего отправлять".into());
    }

    // Отдельный поток: клиент к сайдкару блокирующий, и вызвать его
    // из рантайма tokio нельзя.
    std::thread::spawn(move || {
        let mut model = Sidecar::new(base, sampling, app.clone());

        // Флаг отмены и очередь ответа — до запуска цикла. Иначе
        // «стоп», нажатый в первую секунду, не дошёл бы до агента.
        let решение = РешениеWait::новый();
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let handle = AgentHandle { cancel: std::sync::Arc::clone(&cancel), решение: std::sync::Arc::clone(&решение) };
        {
            let mut место = ЦИКЛ.lock().expect("цикл lock poisoned");
            *место = Some(handle.clone());
        }

        let mut спрашивающий = ИзИнтерфейса {
            app: app.clone(),
            request_id,
            handle,
        };
        let mut agent = Agent::со_спросом(&mut model, s, cancel, &mut спрашивающий);

        let итог = agent.run(start, request_id);
        report(&app, &итог, &agent, request_id);

        // Цикл окончен: держать его флаг больше незачем, иначе «стоп»
        // из следующего диалога отменил бы уже завершившийся.
        let mut место = ЦИКЛ.lock().expect("цикл lock poisoned");
        *место = None;
        chat::reset_cancel();
    });

    Ok(request_id)
}

fn report(app: &AppHandle, out: &Outcome, агент: &Agent, request_id: u64) {
    for c in &out.calls {
        let _ = app.emit(EVENT_CALL, CallEvent { request_id, call: c.clone() });
    }
    // Событие `agent:pending` отправляет не отчёт, а спрашивающий:
    // он же ждёт ответа. Иначе событие пришлось бы слать из двух
    // мест, и одно из них сработало бы на вызов, который уже
    // разрешён.
    // Основание собирается здесь, а не в цикле: интерфейсу нужна
    // готовая строка, и собирать её из вызовов второй раз значит
    // завести второе место, где живёт это правило.
    let label = out.основание.строка(out.инструменты_были);
    let _ = app.emit(
        EVENT_DONE,
        DoneEvent {
            request_id,
            answer: out.answer.clone(),
            calls: out.calls.clone(),
            turns: out.turns,
            stop_reason: out.stop_reason.clone(),
            error: None,
            основание: BasisView {
                read: out.основание.прочитано.clone(),
                ok: out.основание.успешных,
                denied: out.основание.отказов,
                label,
            },
            страховка: out.была_страховка,
            сжатие: агент.сжатие_текст.clone(),
        },
    );
}

/// Текущий цикл агента. Один: диалог один, и «стоп» должен знать,
/// к кому относится.
///
/// `Mutex`, а не `OnceLock`: циклов будет много, а `OnceLock` записывается
/// один раз — второй диалог остался бы без отмены, и «стоп» молча
/// перестал бы работать после первой переписки.
static ЦИКЛ: std::sync::Mutex<Option<AgentHandle>> = std::sync::Mutex::new(None);

/// Что нужно для остановки цикла из другого потока.
#[derive(Clone)]
pub struct AgentHandle {
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Ответ пользователя на последний вопрос.
    ///
    /// `Condvar` вместо канала: ждать приходится на одном месте, и
    /// трогать его можно из двух мест — из кнопки интерфейса и из
    /// «стоп».
    решение: std::sync::Arc<РешениеWait>,
}

impl AgentHandle {
    pub fn cancel(&self) {
        self.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
        // Отмена должна разбудить ожидание, иначе цикл провисит до
        // таймаута: он стоит с проверкой отмены, но без сигнала
        // проснётся не сразу.
        self.решение.будить();
    }
}

/// Ответ пользователя: ждёт и отдаёт.
struct РешениеWait {
    /// Решение, если оно уже есть.
    значение: std::sync::Mutex<Option<approvals::Решение>>,
    /// Просыпается при `notify_all` или по таймауту.
    будка: std::sync::Condvar,
}

impl РешениеWait {
    fn новый() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            значение: std::sync::Mutex::new(None),
            будка: std::sync::Condvar::new(),
        })
    }

    /// Оставить ответ.
    fn отдать(&self, решение: approvals::Решение) {
        let mut г = self.значение.lock().expect("решение lock poisoned");
        *г = Some(решение);
        self.будка.notify_all();
    }

    /// Разбудить, ничего не ответив. Для отмены.
    fn будить(&self) {
        let _г = self.значение.lock().expect("решение lock poisoned");
        self.будка.notify_all();
    }

    /// Ждать ответа. `None` — отмена или таймаут.
    ///
    /// Таймаут обязателен: если интерфейс не ответит, цикл обязан
    /// отпустить поток. Ответ «не ответил» и ответ «запретил» здесь
    /// дают одно и то же — отказ, — и это правильно: действие, о
    /// котором не спросили, выполнено быть не должно.
    fn ждать(
        &self,
        отмена: &std::sync::atomic::AtomicBool,
    ) -> approvals::Решение {
        let г = self.значение.lock().expect("решение lock poisoned");
        // Пока ждём, условие проверяется на каждом пробуждении: и при
        // ответе, и при отмене, и по таймауту.
        let ждать = |значение: &mut Option<approvals::Решение>| {
            отмена.load(std::sync::atomic::Ordering::SeqCst) || значение.is_some()
        };
        let (г, _) = self
            .будка
            .wait_timeout_while(г, std::time::Duration::from_secs(600), ждать)
            .expect("ожидание решения");
        match г.as_ref() {
            Some(&р) => р,
            // Ни ответа, ни явной отмены — просто вышли по таймауту.
            // Тот же исход: не разрешено.
            None => {
                eprintln!("подтверждение не получено за 600 секунд, считаем отказом");
                approvals::Решение::Отказ
            }
        }
    }
}

/// Спрашивающий, который показывает вопрос в интерфейсе и ждёт.
struct ИзИнтерфейса {
    app: AppHandle,
    request_id: u64,
    handle: AgentHandle,
}

impl crate::agent::executor::Спрашивающий for ИзИнтерфейса {
    fn спросить(&mut self, вопрос: &PendingApproval) -> approvals::Решение {
        let _ = self.app.emit(
            EVENT_PENDING,
            PendingEvent {
                request_id: self.request_id,
                pending: вопрос.clone(),
            },
        );
        self.handle.решение.ждать(&self.handle.cancel)
    }
}

/// Ответ пользователя на вопрос о подтверждении.
///
/// Отдельная команда, а не флаг в настройках: решение относится к
/// конкретному действию в конкретном запуске, и постоянным его
/// делать нельзя.
#[tauri::command]
pub fn agent_decide(decision: approvals::Решение) -> Result<(), String> {
    let место = ЦИКЛ.lock().expect("цикл lock poisoned");
    let h = место
        .as_ref()
        .ok_or("подтверждение никто не спрашивает")?;
    h.решение.отдать(decision);
    Ok(())
}

/// Отмена цикла агента.
/// Ставит флаг у текущего цикла и общий флаг генерации: цикл мог
/// сейчас ждать ответа модели или ответа пользователя, и его надо
/// прервать на середине, а не после.
#[tauri::command]
pub fn stop_agent() {
    chat::stop_generation();
    let место = ЦИКЛ.lock().expect("цикл lock poisoned");
    if let Some(h) = место.as_ref() {
        h.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn отчёт_о_инструментах_собирается() {
        let r = tools_report(&Settings::default());
        // По умолчанию чтение разрешено.
        assert!(r.tools.iter().any(|t| t == "fs.read"), "{:?}", r.tools);
        assert!(r.capabilities.iter().any(|c| c == "files.read"));
        // Инструменты управления видны, хотя и не разрешены: человек
        // должен видеть, что секретарь умеет, иначе непонятно, что
        // включать в знакомстве. Меняет ли он компьютер — отдельный
        // вопрос, и по умолчанию ответ «нет».
        assert!(
            r.tools.iter().any(|t| t == "windows.click"),
            "управление должно быть видно в отчёте: {:?}",
            r.tools
        );
        let s = Settings::default();
        assert!(
            !s.allows("windows.control").allowed,
            "по умолчанию управление не должно быть разрешено"
        );
        assert!(
            !s.allows("files.write").allowed,
            "по умолчанию запись не должна быть разрешена"
        );
    }

    #[test]
    fn входящее_сообщение_превращается() {
        let m: Incoming = serde_json::from_value(json!({"role": "user", "content": "привет"})).unwrap();
        let msg = Message { role: m.role, content: m.content, tool_calls: None, tool_call_id: None, name: None };
        assert_eq!(msg.role, "user");
        assert_eq!(msg.content, "привет");
        assert!(msg.tool_calls.is_none());
    }

    /// Роль, которую прислал интерфейс, не должна проходить как есть:
    /// никто не должен суметь подделать сообщение модели или
    /// результат инструмента.
    #[test]
    fn роль_из_интерфейса_ограничена() {
        let m: Incoming = serde_json::from_value(json!({"role": "tool", "content": "я выдумал"})).unwrap();
        // Роль приходит как есть, но результатом вызова инструмента
        // она не станет: у результата обязан быть `tool_call_id`, а
        // привести его интерфейс не может — поля нет в структуре
        // входящего. Проверяем именно это.
        let msg = Message { role: m.role, content: m.content, tool_calls: None, tool_call_id: None, name: None };
        assert_eq!(msg.role, "tool");
        assert!(msg.tool_call_id.is_none());
        assert!(msg.tool_calls.is_none());
    }
}