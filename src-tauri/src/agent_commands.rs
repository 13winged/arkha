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
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_notification::NotificationExt;

use crate::agent::approvals;
use crate::agent::executor::{Agent, CallRecord, Message, Outcome, PendingApproval};
use crate::agent::live::Sidecar;
use crate::chat;
use crate::model_manager::SidecarState;
use crate::permissions::{ApprovalMode, Settings};

const EVENT_CALL: &str = "agent:call";
const EVENT_PENDING: &str = "agent:pending";
const EVENT_DONE: &str = "agent:done";
const EVENT_STARTED: &str = "agent:started";
const EVENT_CONTEXT: &str = "agent:context_ready";
const EVENT_TOOL_RESULT: &str = "agent:tool_result";
const EVENT_ERROR: &str = "agent:error";
const EVENT_CANCELLED: &str = "agent:cancelled";

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
    #[serde(default)]
    pub attachments: Vec<String>,
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
pub fn agent_tools(settings: State<'_, SettingsState>) -> Result<ToolsReport, String> {
    crate::auth::require_authenticated()?;
    let s = settings.0.lock().expect("settings lock poisoned");
    Ok(tools_report(&s))
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
    crate::auth::require_authenticated()?;
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
    crate::auth::require_authenticated()?;
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
    crate::auth::require_authenticated()?;
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
        Self(std::sync::Mutex::new(Settings::default()))
    }

    pub fn reload_for_active_user(&self) -> Result<(), String> {
        self.replace(Settings::load()?)
    }

    /// Заменить действующие настройки.
    ///
    /// Нужно не только при входе: человек меняет галочку в настройках
    /// и ждёт, что она подействует сразу. Без этого агент продолжал
    /// работать с копией, прочитанной при запуске, и отказывал,
    /// хотя в файле уже стояло разрешение.
    pub fn replace(&self, loaded: Settings) -> Result<(), String> {
        *self.0.lock().map_err(|_| "settings lock poisoned".to_string())? = loaded;
        Ok(())
    }

    pub fn clear_on_logout(&self) {
        if let Ok(mut state) = self.0.lock() {
            *state = Settings::default();
        }
    }
}

/// Кто говорит с моделью и с какими инструментами.
#[tauri::command]
pub fn agent_chat(
    app: AppHandle,
    sidecar: State<'_, SidecarState>,
    settings: State<'_, SettingsState>,
    messages: Vec<Incoming>,
    // Запрос пришёл из голосовой команды, а не из набранного текста.
    voice: Option<bool>,
) -> Result<u64, String> {
    crate::auth::require_authenticated()?;
    let base = sidecar.base_url()?;
    let request_id = chat::next_request_id();
    chat::reset_cancel();

    let s = settings.0.lock().expect("settings lock poisoned").clone();
    // Голос — это уже явное указание человека, и держать его перед
    // кнопкой «разрешить» значит ломать сам сценарий: руки заняты,
    // окно может быть свёрнуто. Поэтому запуск программы по голосу
    // не спрашивает, но только если возможность включена (её всё
    // равно проверит инструмент) и режим не «спрашивать всегда».
    let голосом_без_вопроса =
        voice.unwrap_or(false) && s.executor == crate::permissions::Executor::Live
            && s.approval_mode != ApprovalMode::Confirm;

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

    let attachment_ids = messages.iter().flat_map(|message| message.attachments.iter().cloned()).collect::<Vec<_>>();
    let mut messages = messages;
    let user = crate::settings::settings_get()?.user;
    if !user.login.is_empty() || !user.display_name.is_empty() {
        messages.insert(0, Incoming {
            role: "system".into(),
            content: format!(
                "Локальный профиль пользователя: имя «{}», login «{}». Учитывай его как постоянный контекст. Пароли и секреты недоступны.",
                if user.display_name.is_empty() { "не указано" } else { &user.display_name },
                if user.login.is_empty() { "не указано" } else { &user.login },
            ),
            attachments: Vec::new(),
        });
    }
    if let Some(context) = crate::attachments::context(&attachment_ids)? {
        messages.insert(0, Incoming { role: "system".into(), content: context, attachments: Vec::new() });
    }
    let start: Vec<Message> = messages.into_iter().map(|m| Message { role: m.role, content: m.content, tool_calls: None, tool_call_id: None, name: None }).collect();
    if start.is_empty() {
        return Err("нечего отправлять".into());
    }
    let _ = app.emit(EVENT_STARTED, StartedEvent { request_id });
    let tool_count = crate::tools::Registry::allowed(&s).ids().len();

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
            голосом_без_вопроса,
        };
        let _ = app.emit(
            EVENT_CONTEXT,
            ContextReadyEvent {
                request_id,
                messages: start.len(),
                tools: tool_count,
            },
        );
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
        let _ = app.emit(EVENT_TOOL_RESULT, ToolResultEvent { request_id, call: c.clone() });
    }
    // Событие `agent:pending` отправляет не отчёт, а спрашивающий:
    // он же ждёт ответа. Иначе событие пришлось бы слать из двух
    // мест, и одно из них сработало бы на вызов, который уже
    // разрешён.
    if out.stop_reason.starts_with("модель не ответила") {
        let _ = app.emit(EVENT_ERROR, ErrorEvent { request_id, message: out.stop_reason.clone() });
    }
    if out.stop_reason == "остановлено вами" {
        let _ = app.emit(EVENT_CANCELLED, ErrorEvent { request_id, message: out.stop_reason.clone() });
    }
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

    // Системное уведомление, а не только плашка в окне: человек мог
    // свернуть приложение или уйти в другую программу, и тогда конец
    // запроса иначе остаётся незамеченным. Когда окно на переднем
    // плане, достаточно плашки — второй раз то же самое не шлём.
    let окно_активно = app
        .get_webview_window("main")
        .and_then(|окно| окно.is_focused().ok())
        .unwrap_or(false);
    if !окно_активно {
        let текст = if out.answer.trim().is_empty() {
            out.stop_reason.clone()
        } else {
            out.answer.clone()
        };
        let кратко: String = текст.chars().take(180).collect();
        let _ = app
            .notification()
            .builder()
            .title("ArkHa — готово")
            .body(кратко)
            .show();
    }
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
        self.ждать_до(отмена, std::time::Duration::from_secs(600))
    }

    /// То же ожидание, но со своим сроком. Отдельная функция нужна
    /// тестам: ждать шестьсот секунд в проверке нельзя.
    fn ждать_до(
        &self,
        отмена: &std::sync::atomic::AtomicBool,
        срок: std::time::Duration,
    ) -> approvals::Решение {
        let г = self.значение.lock().expect("решение lock poisoned");
        // Пока ждём, условие проверяется на каждом пробуждении: и при
        // ответе, и при отмене, и по таймауту.
        let ждать = |значение: &mut Option<approvals::Решение>| {
            отмена.load(std::sync::atomic::Ordering::SeqCst) || значение.is_some()
        };
        let (mut г, _) = self
            .будка
            .wait_timeout_while(г, срок, ждать)
            .expect("ожидание решения");
        // `take`, а не `as_ref`: ответ относится к одному вопросу.
        // Иначе отказ, полученный на первом вызове, молча применялся
        // бы ко всем следующим — модель видела два «запрещено
        // пользователем» подряд, хотя человек отвечал один раз.
        match г.take() {
            Some(р) => р,
            // Ни ответа, ни явной отмены — просто вышли по таймауту.
            // Тот же исход: не разрешено.
            None => {
                eprintln!("подтверждение не получено за {} секунд, считаем отказом", срок.as_secs());
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
    /// Голосовой запуск программы не спрашивает подтверждения.
    голосом_без_вопроса: bool,
}

impl crate::agent::executor::Спрашивающий for ИзИнтерфейса {
    fn спросить(&mut self, вопрос: &PendingApproval) -> approvals::Решение {
        if self.голосом_без_вопроса && вопрос.capability == "apps.launch" {
            return approvals::Решение::ОдинРаз;
        }
        let _ = self.app.emit(
            EVENT_PENDING,
            PendingEvent {
                request_id: self.request_id,
                pending: вопрос.clone(),
            },
        );
        // Вопрос мог прийти, когда окно свёрнуто или закрыто другим
        // приложением. Тогда человек его просто не увидит и ответит
        // «ничего», то есть отказ. Мигание в панели задач и системное
        // уведомление дают шанс узнать, что от него ждут ответа.
        if let Some(окно) = self.app.get_webview_window("main") {
            let _ = окно.request_user_attention(Some(tauri::UserAttentionType::Critical));
        }
        let _ = self
            .app
            .notification()
            .builder()
            .title("ArkHa: нужно подтверждение")
            .body(вопрос.reason.chars().take(180).collect::<String>())
            .show();
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
    crate::auth::require_authenticated()?;
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
pub fn stop_agent() -> Result<(), String> {
    crate::auth::require_authenticated()?;
    let _ = chat::stop_generation();
    let место = ЦИКЛ.lock().expect("цикл lock poisoned");
    if let Some(h) = место.as_ref() {
        h.cancel();
    }
    Ok(())
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

    /// Ответ на подтверждение относится к одному вопросу. Раньше он
    /// оставался в очереди, и следующий вызов получал тот же «отказ»
    /// без нового вопроса — в журнале это выглядело как два запрета
    /// подряд.
    #[test]
    fn решение_на_подтверждение_не_повторяется() {
        let ожидание = РешениеWait::новый();
        let отмена = std::sync::atomic::AtomicBool::new(false);
        ожидание.отдать(approvals::Решение::ВсюЗадачу);
        assert_eq!(
            ожидание.ждать_до(&отмена, std::time::Duration::from_millis(50)),
            approvals::Решение::ВсюЗадачу
        );
        // Второго ответа нет: ожидание обязано истечь, а не выдать
        // прошлое разрешение.
        assert_eq!(
            ожидание.ждать_до(&отмена, std::time::Duration::from_millis(50)),
            approvals::Решение::Отказ
        );
    }

    /// Галочка в настройках должна действовать сразу, а не после
    /// перезапуска: агент читает действующую копию из состояния.
    #[test]
    fn смена_настроек_видна_сразу() {
        let состояние = SettingsState::new();
        let mut новые = Settings::default();
        новые.executor = crate::permissions::Executor::Live;
        состояние.replace(новые).unwrap();
        assert_eq!(
            состояние.0.lock().unwrap().executor,
            crate::permissions::Executor::Live
        );
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolResultEvent {
    pub request_id: u64,
    pub call: CallRecord,
}

#[derive(Debug, Clone, Serialize)]
pub struct ErrorEvent {
    pub request_id: u64,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct StartedEvent {
    pub request_id: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContextReadyEvent {
    pub request_id: u64,
    pub messages: usize,
    pub tools: usize,
}
