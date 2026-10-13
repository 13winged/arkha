// Цикл агента S6: модель попросила инструмент — мы выполнили —
// отдали результат обратно.
//
// Главное решение: ядро цикла не знает про HTTP.
//
// Оно работает через признак [`Model`], а не через `reqwest`. Иначе
// проверить его можно было бы только с живой моделью на своей
// машине, а проверять надо каждый раз при изменении: цикл с
// ограничением числа шагов, с проверкой разрешений и с защитой от
// бесконечного вызова одного и того же инструмента — это ровно то
// место, где ошибка проявится на пользователе, а не на тесте.
//
// Живой сайдкар подключается отдельно, тонким слоем в
// [`crate::chat::agent_chat`].
//
// Три ограничителя, и все три обязательны:
//
// | Ограничитель | От чего спасает |
// |---|---|
// | предел шагов | модель, которая вызывает инструмент по кругу |
// | запрет повтора | модель, которая повторяет один и тот же вызов после отказа |
// | подтверждение | модель, которая дерзнула без спроса |
//
// Первые два — технические. Третий — про доверие, и он по умолчанию
// включён.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::permissions::Settings;
use crate::tools::{Registry, ToolError};

use super::approvals::Решение;

/// Сколько раз подряд агент может вызвать инструменты.
///
/// Число выбрано по тому, что нужно обычной задаче: прочитать папку,
/// прочитать два файла, ответить. Двенадцать заходов — это заведомо
/// больше, чем нужно человеку, и заведомо меньше, чем нужно, чтобы
/// зациклиться и съесть память.
pub const MAX_TURNS: usize = 12;

/// Что сказать модели, когда она ответила словами, не вызвав ни одного
/// инструмента.
///
/// Формулировка проверена: сначала факт, потом действие. Просьба
/// «попробуй ещё раз» без указания, что именно, на этой модели не
/// работает — она и так считает, что всё перепробовала.
pub const ПОДСКАЗКА_ПРОВАЛА: &str = "\
Ни один инструмент не был вызван, а ответ уже готов. Если задача про файлы или \
папки — ты не мог её решить: ты ничего не читал. Сейчас доступны инструменты, \
перечисленные в описании. Вызови их и прочитай то, о чём тебя спрашивают.";

/// На чём основан ответ агента.
///
/// Это не проверка выдумки. Выдумку по тексту ответа проверить нельзя:
/// любая эвристика на словах даст ложные срабатывания на «привет», на
/// даты и на нумерации, и агент начнёт отклонять верные ответы.
///
/// Здесь ровно другое: показывается, что было сделано на самом деле.
/// Если ничего не прочитано — это написано, и пользователь видит
/// правдоподобный ответ рядом с пометкой «ничего не прочитано».
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Основание {
    /// Что реально прочитано, в человеческом виде.
    pub прочитано: Vec<String>,
    /// Сколько вызовов успешно отработало.
    pub успешных: usize,
    /// Сколько было отказов.
    pub отказов: usize,
}

impl Основание {
    /// Короткая строка для интерфейса.
    ///
    /// Пустое основание при наличии инструментов — это и есть
    /// сигнал. Формулировка намеренно не «возможно, ошибка», а
    /// «ничего не прочитано»: первое оставляет сомнение, второе
    /// говорит факт.
    pub fn строка(&self, инструменты_были: bool) -> String {
        if self.прочитано.is_empty() {
            if инструменты_были {
                return "ничего не прочитано: ответ не опирается на файлы".into();
            }
            return "инструменты не выданы, ответ из памяти".into();
        }
        if self.прочитано.len() == 1 {
            format!("прочитано: {}", self.прочитано[0])
        } else {
            format!("прочитано: {} и ещё {}", self.прочитано[0], self.прочитано.len() - 1)
        }
    }
}

/// Сообщение в разговоре.
///
/// Отдельный тип, а не переиспользование `ChatMessage` из чата: там
/// есть только роль и текст, а циклу нужно различать сообщение модели,
/// вызов инструмента и его результат. Смешать их — значит отдать
/// модели её же вызов обратно в виде текста.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Message {
    pub role: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub content: String,
    /// Вызовы инструментов, которые модель объявила.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Value>,
    /// Идентификатор вызова, для результата.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Имя инструмента в сообщении-результате.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl Message {
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: text.into(),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    /// Подсказка цикла модели: не от пользователя.
    ///
    /// Роль `user`, а не системная строка: системная идёт один раз в
    /// начале, а эта нужна посреди разговора, после конкретного
    /// провала. В переписке пользователя она не видна — сообщение
    /// создаётся циклом, а не приходит оттуда.
    pub fn подсказка() -> Self {
        Self {
            role: "user".into(),
            content: ПОДСКАЗКА_ПРОВАЛА.to_string(),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    pub fn assistant(text: impl Into<String>, calls: Option<Value>) -> Self {
        Self {
            role: "assistant".into(),
            content: text.into(),
            tool_calls: calls,
            tool_call_id: None,
            name: None,
        }
    }

    /// Результат инструмента. Текст отказа — это тоже результат, и
    /// он должен дойти до модели: иначе она не поймёт, что произошло,
    /// и попробует снова.
    pub fn tool_result(id: impl Into<String>, name: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            role: "tool".into(),
            content: text.into(),
            tool_calls: None,
            tool_call_id: Some(id.into()),
            name: Some(name.into()),
        }
    }
}

/// Ответ модели на один заход.
#[derive(Debug, Clone, Default)]
pub struct Turn {
    pub text: String,
    pub tool_calls: Option<Value>,
}

/// Что умеет модель для цикла.
pub trait Model {
    /// Один заход: текст и, возможно, вызовы инструментов.
    fn turn(
        &mut self,
        messages: &[Message],
        tools: &[Value],
        request_id: u64,
    ) -> Result<Turn, String>;
}

/// Тот, кого цикл спрашивает о разрешении.
///
/// Отдельная функция, а не флаг в настройках, потому что ответ
/// приходит из интерфейса и ждать приходится по-настоящему. Значит
/// граница проходит по потокам, и проверить её без интерфейса можно
/// только подставкой — что и делают тесты.
pub trait Спрашивающий {
    /// Что решил пользователь.
    ///
    /// Вызывается **до** выполнения инструмента. Возврат `Отказ`
    /// обязан быть безопасным: отсутствие ответа, ошибка и
    /// исключение — всё это `Отказ`, потому что действие, о котором
    /// не спросили, выполнено быть не должно.
    fn спросить(&mut self, вопрос: &PendingApproval) -> Решение;
}

/// Отказ по умолчанию: пользователя нет, значит разрешения нет.
///
/// Это не параноидальность, а отсутствие выбора. Если спрашивающего не
/// задали, единственный честный ответ — «не разрешено»: выполнить
/// действие без спроса нельзя, и «на всякий случай разрешить» здесь
/// означало бы ровно то, от чего защищает подтверждение.
pub struct БезСогласия;

impl Спрашивающий for БезСогласия {
    fn спросить(&mut self, _вопрос: &PendingApproval) -> Решение {
        Решение::Отказ
    }
}

/// Что сделал один вызов инструмента. Для журнала и интерфейса.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CallRecord {
    pub tool: String,
    /// Отказ или успех.
    pub ok: bool,
    /// Коротко: что получилось.
    pub summary: String,
    /// Отказ — это не поломка, поэтому отдельным флагом.
    pub denied: bool,
}

/// Что агент хочет сделать, но ждёт разрешения.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PendingApproval {
    pub call_id: String,
    pub tool: String,
    pub capability: String,
    pub arguments: Value,
    pub reason: String,
    /// Уровень опасности строкой: `L1`, `L2`, `L3`. Интерфейсу
    /// показывать не нужно — он берёт `reason`, где по-человечески.
    pub level: String,
}

/// Итог цикла.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Outcome {
    /// Текст последнего ответа без вызовов.
    pub answer: String,
    pub calls: Vec<CallRecord>,
    /// Разговор целиком, чтобы интерфейс показал историю.
    pub messages: Vec<Message>,
    /// Заходов сделано.
    pub turns: usize,
    /// Причина остановки, словами для пользователя.
    pub stop_reason: String,
    /// На чём основан ответ. Показывается рядом с ответом.
    pub основание: Основание,
    /// Сработала ли страховка: ответ не опирался ни на что, и цикл
    /// попросил модель попробовать ещё раз.
    pub была_страховка: bool,
    /// Были ли инструменты в этом запуске.
    ///
    /// Без этого поля основание не отличить «нечем было читать» от
    /// «было чем, но не прочитали». Для пользователя это разные
    /// вещи: в первом случае он что-то отключил, во втором от него
    /// скрыли результат.
    pub инструменты_были: bool,
}

/// Агент: цикл вызовов инструментов.
pub struct Agent<'a> {
    model: &'a mut dyn Model,
    registry: Registry,
    settings: Settings,
    /// Вызовы, уже выполненные в этом цикле: «инструмент + аргументы»
    /// попарно. Повтор запрещаем.
    done: Vec<(String, String)>,
    pub calls: Vec<CallRecord>,
    /// Отмена пользователя. Проверяется на границе заходов и перед
    /// каждым вызовом инструмента: остановиться «между словами» нельзя,
    /// но остановиться до того, как что-то сделано, — можно.
    ///
    /// Отдельный флаг, а не глобальный: тест должен уметь отменить
    /// один цикл, не задев остальные.
    cancel: Arc<AtomicBool>,
    /// Страховка сработала уже раз. Повтор ровно один: второй не
    /// помогает, а время тратит.
    страховка_сработала: bool,
    /// Разрешения, выданные на текущую задачу.
    ///
    /// Пустой при каждом новом запуске. Постоянного разрешения здесь
    /// быть не может: выданный доступ — это уже не подтверждение, а
    /// решение о правах, и принимать его молча нельзя.
    разрешения: crate::agent::approvals::Разрешения,
    /// Кого спрашиваем. `None` — никого, отказ.
    спрашивающий: Option<&'a mut dyn Спрашивающий>,
    /// При последнем сжатии. Показывается пользователю.
    ///
    /// Отдельная строка, а не флаг: пользователь должен видеть, что
    /// секретарь забыл часть разговора. Молчаливое сжатие выглядит
    /// как «он просто не помнит», и это ложь.
    pub сжатие_текст: String,
    /// Сколько символов считать переполнением.
    ///
    /// Задаётся извне, а не берётся из профиля: в тестах нужен
    /// маленький порог, а в жизни — большой.
    pub порог_контекста: usize,
}

impl<'a> Agent<'a> {
    pub fn new(model: &'a mut dyn Model, settings: Settings) -> Self {
        Self::with_cancel(model, settings, Arc::new(AtomicBool::new(false)))
    }

    /// Цикл с явной отменой.
    pub fn with_cancel(
        model: &'a mut dyn Model,
        settings: Settings,
        cancel: Arc<AtomicBool>,
    ) -> Self {
        let registry = Registry::allowed(&settings);
        Self {
            model,
            registry,
            settings,
            done: Vec::new(),
            calls: Vec::new(),
            cancel,
            страховка_сработала: false,
            разрешения: crate::agent::approvals::Разрешения::пусто(),
            спрашивающий: None,
            сжатие_текст: String::new(),
            порог_контекста: super::compaction::ПОРОГ,
        }
    }

    /// Цикл, который умеет спрашивать пользователя.
    pub fn со_спросом(
        model: &'a mut dyn Model,
        settings: Settings,
        cancel: Arc<AtomicBool>,
        спрашивающий: &'a mut dyn Спрашивающий,
    ) -> Self {
        let mut a = Self::with_cancel(model, settings, cancel);
        a.спрашивающий = Some(спрашивающий);
        a
    }

    /// Спросить и получить ответ. Без спрашивающего — отказ.
    fn спросить(&mut self, вопрос: PendingApproval) -> Решение {
        match self.спрашивающий.as_mut() {
            Some(с) => с.спросить(&вопрос),
            None => Решение::Отказ,
        }
    }

    /// Сколько раз за эту задачу уже разрешали.
    ///
    /// Текст показывается рядом с вопросом: пользователь должен видеть,
    /// что это не первый раз и что после «на всю задачу» следующие
    /// такие действия вопросов не вызовут.
    fn основание_повторов(&self) -> String {
        if self.разрешения.возможности.is_empty() {
            String::new()
        } else {
            format!(
                "Уже разрешено на эту задачу: {}.",
                self.разрешения.возможности.iter().cloned().collect::<Vec<_>>().join(", ")
            )
        }
    }

    /// Флаг отмены этого цикла. Отдаётся наружу, чтобы команда могла
    /// его поставить: пользователь нажал «стоп», а цикл это видит.
    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel)
    }

    fn отменено(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    /// На чём основан ответ: что реально было сделано.
    ///
    /// Считается из фактических вызовов, а не из намерения модели.
    fn основание(&self) -> Основание {
        let mut о = Основание::default();
        for c in &self.calls {
            if c.ok {
                о.успешных += 1;
            } else {
                о.отказов += 1;
            }
            // Название файла в сводке инструмента начинается после
            // двоеточия: «прочитан заметка.txt: 48 байт». Брать надо
            // его, а не весь текст.
            if c.ok && c.tool.starts_with("fs.read") {
                let имя = c
                    .summary
                    .split_once(':')
                    .map(|(а, _)| а.trim().to_string())
                    .unwrap_or_else(|| c.summary.clone());
                о.прочитано.push(имя);
            }
        }
        о
    }

    /// Запустить цикл до ответа, отказа или лимита.
    pub fn run(&mut self, mut messages: Vec<Message>, request_id: u64) -> Outcome {
        // Сжатие — до первого обращения к модели. Иначе запрос может
        // просто не уйти: контекст конечен, а разговор нет.
        let сжатие = super::compaction::сжать(messages, self.порог_контекста);
        self.сжатие_текст = сжатие.причина.clone();
        messages = сжатие.messages;

        let tools = self.registry.as_request_json(&self.settings);

        for turn_no in 0..MAX_TURNS {
            // Отмена проверяется ДО обращения к модели: если
            // пользователь нажал «стоп», ждать ещё и его ответ незачем.
            if self.отменено() {
                return Outcome {
                    answer: String::new(),
                    calls: self.calls.clone(),
                    messages,
                    turns: turn_no,
                    stop_reason: "остановлено вами".into(),
                    основание: self.основание(),
                    была_страховка: self.страховка_сработала,
                    инструменты_были: !tools.is_empty(),
                };
            }

            let answer = match self.model.turn(&messages, &tools, request_id) {
                Ok(t) => t,
                Err(e) => {
                    return Outcome {
                        answer: String::new(),
                        calls: self.calls.clone(),
                        messages,
                        turns: turn_no,
                        stop_reason: format!("модель не ответила: {e}"),
                        основание: self.основание(),
                        была_страховка: self.страховка_сработала,
                        инструменты_были: !tools.is_empty(),
                    }
                }
            };

            let Some(calls) = answer.tool_calls.as_ref() else {
                // Страховка A: инструменты были, а модель ответила
                // словами, не вызвав ни одного.
                //
                // На замере 12 октября это был ровно каждый второй
                // провал: 8 из 16 ответов «не могу найти» шли без
                // единого вызова. Детектор точный, а не эвристика:
                // пустой список вызовов при непустом списке
                // инструментов — это факт.
                //
                // Повтор ровно один: не нашёл со второго раза — значит
                // не найдёт, и дальше будет только расход времени.
                if !tools.is_empty() && self.calls.is_empty() && !self.страховка_сработала {
                    self.страховка_сработала = true;
                    messages.push(Message::assistant(answer.text.clone(), None));
                    messages.push(Message::подсказка());
                    continue;
                }

                // Обычный ответ. Цикл закончен.
                let answer = answer.text.clone();
                messages.push(Message::assistant(answer.clone(), None));
                return Outcome {
                    answer,
                    calls: self.calls.clone(),
                    messages,
                    turns: turn_no + 1,
                    stop_reason: "ответ готов".into(),
                    основание: self.основание(),
                    была_страховка: self.страховка_сработала,
                    инструменты_были: !tools.is_empty(),
                };
            };

            let calls = calls.clone();
            messages.push(Message::assistant(answer.text.clone(), Some(calls.clone())));

            for call in calls.as_array().cloned().unwrap_or_default() {
                // Отмена и здесь: модель могла вернуть сразу десять
                // вызовов, и выполнять их все после «стоп» нельзя.
                if self.отменено() {
                    return Outcome {
                        answer: String::new(),
                        calls: self.calls.clone(),
                        messages,
                        turns: turn_no + 1,
                        stop_reason: "остановлено вами".into(),
                        основание: self.основание(),
                        была_страховка: self.страховка_сработала,
                        инструменты_были: !tools.is_empty(),
                    };
                }
                let id = call
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let name = call
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|n| n.as_str())
                    .unwrap_or_default()
                    .to_string();
                let raw_args = call
                    .get("function")
                    .and_then(|f| f.get("arguments"))
                    .cloned()
                    .unwrap_or(Value::Null);

                // Аргументы приходят строкой. Разбираются здесь, а не
                // в инструменте: неразобранный JSON — это поломка
                // вызова, а не отказ инструмента.
                let args = match разобрать_аргументы(&raw_args) {
                    Ok(a) => a,
                    Err(e) => {
                        messages.push(Message::tool_result(
                            &id,
                            &name,
                            format!("вызов не разобран: {e}. Нужны аргументы объектом"),
                        ));
                        self.calls.push(CallRecord {
                            tool: name,
                            ok: false,
                            summary: "аргументы не разобрались".into(),
                            denied: false,
                        });
                        continue;
                    }
                };

                // Подтверждение — до выполнения, и до проверки повтора.
                // Нечего подтверждать то, что уже отклонено.
                let capability = self
                    .registry
                    .get(&name, &self.settings)
                    .map(|t| t.spec(&self.settings).capability)
                    .unwrap_or_default();
                let уровень = crate::agent::approvals::уровень(&capability, &args);
                let вопрос = Some(PendingApproval {
                    call_id: id.clone(),
                    tool: name.clone(),
                    capability: capability.clone(),
                    arguments: args.clone(),
                    reason: match self.settings.allows(&capability) {
                        crate::permissions::Decision { allowed: true, .. } => format!(
                            "{} — {}. {}",
                            уровень.название(),
                            уровень.пояснение(),
                            self.основание_повторов()
                        ),
                        d => d.reason,
                    },
                    level: format!("{:?}", уровень),
                });
                if let Some(вопрос) = вопрос {
                    if crate::agent::approvals::спрашивать(
                        &capability,
                        &args,
                        &self.settings,
                        &self.разрешения,
                    )
                    .is_some()
                    {
                        match self.спросить(вопрос) {
                            crate::agent::approvals::Решение::Отказ => {
                                let текст =
                                    "пользователь запретил это действие. Не предлагай его снова \
                                     и скажи пользователю, что не выполнено.";
                                self.calls.push(CallRecord {
                                    tool: name.clone(),
                                    ok: false,
                                    summary: "запрещено пользователем".into(),
                                    denied: true,
                                });
                                messages.push(Message::tool_result(&id, &name, текст));
                                continue;
                            }
                            решение => {
                                self.разрешения.запомнить(&capability, решение);
                            }
                        }
                    }
                }

                // Повтор того же вызова запрещён. Модель после отказа
                // часто пробует снова то же самое, и без этого
                // ограничения цикл съедает все шаги на одно и то же.
                let key = (name.clone(), аргументы_как_текст(&args));
                if self.done.contains(&key) {
                    let reason = "тот же вызов уже был, повтор запрещён";
                    messages.push(Message::tool_result(&id, &name, reason));
                    self.calls.push(CallRecord {
                        tool: name,
                        ok: false,
                        summary: reason.into(),
                        denied: true,
                    });
                    continue;
                }
                self.done.push(key);

                match self.registry.call(&name, &args, &self.settings) {
                    Ok(out) => {
                        let summary = out.summary.clone();
                        let text = out.text.clone();
                        self.calls.push(CallRecord {
                            tool: name.clone(),
                            ok: true,
                            summary,
                            denied: false,
                        });
                        messages.push(Message::tool_result(&id, &name, text));
                    }
                    Err(e) => {
                        // Отказ — это результат, и модель должна знать,
                        // что с ним делать. Правило остановки из
                        // docs/05: ясная причина — не повторять, неясная
                        // — попробовать иначе.
                        let denied = matches!(e, ToolError::Denied(_) | ToolError::Outside(_));
                        let reason = e.to_string();
                        let текст = format!("{reason}. {}", e.подсказка_модели());
                        self.calls.push(CallRecord {
                            tool: name.clone(),
                            ok: false,
                            summary: reason.clone(),
                            denied,
                        });
                        messages.push(Message::tool_result(&id, &name, &текст));
                    }
                }
            }
        }

        Outcome {
            answer: String::new(),
            calls: self.calls.clone(),
            messages,
            turns: MAX_TURNS,
            stop_reason: format!(
                "остановлено на {MAX_TURNS} шагах: агент не закончил. Возможно, модель зациклилась"
            ),
            основание: self.основание(),
            была_страховка: self.страховка_сработала,
            инструменты_были: !tools.is_empty(),
        }
    }
}

/// Разобрать аргументы вызова.
///
/// llama.cpp присылает `arguments` строкой. Пустая строка — это `{}`,
/// а вот неразобранный JSON — поломка вызова, и молча считать его
/// пустым значит вызвать инструмент не с теми аргументами, о которых
/// думала модель.
fn разобрать_аргументы(raw: &Value) -> Result<Value, String> {
    match raw {
        Value::Null => Ok(Value::Object(serde_json::Map::new())),
        Value::String(s) => {
            let t = s.trim();
            if t.is_empty() {
                return Ok(Value::Object(serde_json::Map::new()));
            }
            serde_json::from_str::<Value>(t).map_err(|e| e.to_string())
        }
        // Уже объект: некоторые серверы не оборачивают в строку.
        Value::Object(_) => Ok(raw.clone()),
        other => Err(format!("ожидался объект аргументов, получено {other}")),
    }
}

/// Ключ для сравнения вызовов. Аргументы приводятся к канонической
/// строке, чтобы `{"a":1,"b":2}` и `{"b":2,"a":1}` считались одним и
/// тем же вызовом: иначе запрет на повтор обходится перестановкой.
fn аргументы_как_текст(v: &Value) -> String {
    fn канон(v: &Value) -> Value {
        match v {
            Value::Object(m) => {
                let mut keys: Vec<&String> = m.keys().collect();
                keys.sort();
                let mut out = serde_json::Map::new();
                for k in keys {
                    out.insert(k.clone(), канон(&m[k]));
                }
                Value::Object(out)
            }
            Value::Array(a) => Value::Array(a.iter().map(канон).collect()),
            other => other.clone(),
        }
    }
    канон(v).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permissions::{CapabilityRule, Executor};
    use serde_json::json;
    use crate::tools::tempdir::TempRoot;

    /// Модель, которая возвращает заготовленные ответы по очереди.
    struct Сценарий {
        ходы: Vec<Result<Turn, String>>,
        взято: usize,
        /// Какие инструменты были видны в каждом заходе.
        видел: Vec<Vec<Value>>,
    }

    impl Сценарий {
        fn нов(ходы: Vec<Result<Turn, String>>) -> Self {
            Self { ходы, взято: 0, видел: Vec::new() }
        }

        /// Ответ без вызовов.
        fn текст(t: &str) -> Result<Turn, String> {
            Ok(Turn { text: t.into(), tool_calls: None })
        }

        /// Вызов инструмента.
        fn вызов(имя: &str, аргументы: Value) -> Result<Turn, String> {
            Ok(Turn {
                text: String::new(),
                tool_calls: Some(json!([
                    {"id": "call-1", "type": "function",
                     "function": {"name": имя, "arguments": аргументы.to_string()}}
                ])),
            })
        }
    }

    /// Подставной пользователь: отвечает заранее выбранным решением
    /// и запоминает заданные вопросы.
    ///
    /// Именно подставка нужна для проверки: настоящий ответ приходит
    /// из интерфейса и по потокам, поэтому без неё подтверждения не
    /// проверить вовсе.
    struct Ответ {
        решение: Решение,
        вопросы: Vec<PendingApproval>,
    }

    impl Ответ {
        fn разрешающий() -> Self {
            Self { решение: Решение::ОдинРаз, вопросы: Vec::new() }
        }
        fn разрешающий_навсегда() -> Self {
            Self { решение: Решение::ВсюЗадачу, вопросы: Vec::new() }
        }
        fn отказывающий() -> Self {
            Self { решение: Решение::Отказ, вопросы: Vec::new() }
        }
    }

    impl Спрашивающий for Ответ {
        fn спросить(&mut self, вопрос: &PendingApproval) -> Решение {
            self.вопросы.push(вопрос.clone());
            self.решение
        }
    }

    impl Model for Сценарий {
        fn turn(&mut self, _messages: &[Message], tools: &[Value], _id: u64) -> Result<Turn, String> {
            self.видел.push(tools.to_vec());
            let r = self
                .ходы
                .get(self.взято)
                .cloned()
                .unwrap_or_else(|| Сценарий::текст("всё"));
            self.взято += 1;
            r
        }
    }

    fn настройки(корень: &TempRoot) -> Settings {
        let mut s = Settings::default();
        s.paths.push(корень.path().to_path_buf());
        s
    }

    #[test]
    fn ответ_без_инструментов_доходит_сразу() {
        // Два ответа: первый без вызовов, второй после подсказки
        // цикла. Ответ один и тот же — важно не это, а то, что он
        // дойдёт, и что повтор был ровно один.
        let mut m = Сценарий::нов(vec![
            Сценарий::текст("Привет. Что у тебя есть?"),
            Сценарий::текст("Привет. Что у тебя есть?"),
        ]);
        let root = TempRoot::new();
        let mut a = Agent::new(&mut m, настройки(&root));
        let out = a.run(vec![Message::user("привет")], 1);
        assert_eq!(out.answer, "Привет. Что у тебя есть?");
        assert!(out.calls.is_empty());
        assert_eq!(out.stop_reason, "ответ готов");
        // Два захода, а не один: страховка спросила ещё раз, потому
        // что инструменты были, а вызовов не было. Это ровно то
        // поведение, ради которого страховка и написана.
        assert_eq!(out.turns, 2, "страховка должна повторить один раз");
        assert!(out.была_страховка);
    }

    /// Страховка A: ответ без единого вызова при доступных
    /// инструментах — это половина всех провалов на замере.
    #[test]
    fn страховка_спрашивает_ещё_раз_без_повтора_бесконечно() {
        let root = TempRoot::new();
        // Модель, которая четыре раза отвечает словами и не зовёт
        // ничего. Повтор должен быть ровно один, иначе цикл съест
        // все двенадцать шагов на пустом месте.
        let mut m = Сценарий::нов(vec![
            Сценарий::текст("не нашёл"),
            Сценарий::текст("не нашёл"),
            Сценарий::текст("не нашёл"),
            Сценарий::текст("не нашёл"),
        ]);
        let mut a = Agent::new(&mut m, настройки(&root));
        let out = a.run(vec![Message::user("найди файл")], 1);

        assert_eq!(out.turns, 2, "больше одного повтора не делаем");
        assert!(out.была_страховка);
        assert!(out.calls.is_empty());
        assert!(out.основание.прочитано.is_empty());
    }

    /// Страховка B: при пустом основании интерфейс получает честную
    /// строку, а не «возможно, ошибка».
    #[test]
    fn основание_говорит_что_ничего_не_прочитано() {
        let root = TempRoot::new();
        let mut m = Сценарий::нов(vec![Сценарий::текст("внутри: помидоры")]);
        let mut a = Agent::new(&mut m, настройки(&root));
        let out = a.run(vec![Message::user("что в файле?")], 1);

        let строка = out.основание.строка(true);
        assert!(строка.contains("ничего не прочитано"), "{строка}");
        // И обязательно без смягчения: «возможно» оставляет сомнение.
        assert!(!строка.contains("возможно"), "{строка}");
    }

    /// При успешном чтении основание называет файл. Это и есть
    /// обратная сторона: пользователь видит, на чём стоит ответ.
    #[test]
    fn основание_называет_прочитанный_файл() {
        let root = TempRoot::new();
        let p = root.write("заметка.txt", "данные".as_bytes());
        let mut m = Сценарий::нов(vec![
            Сценарий::вызов("fs.read", json!({"path": p.to_string_lossy()})),
            Сценарий::текст("там «данные»"),
        ]);
        let mut a = Agent::new(&mut m, настройки(&root));
        let out = a.run(vec![Message::user("прочитай заметка.txt")], 1);

        assert_eq!(out.основание.прочитано.len(), 1, "{:?}", out.основание);
        assert!(
            out.основание.прочитано[0].contains("заметка.txt"),
            "{:?}",
            out.основание.прочитано
        );
        assert_eq!(out.основание.успешных, 1);
        assert!(out.основание.строка(true).contains("прочитано"));
    }

    /// Страховка не должна срабатывать, если инструменты уже
    /// использовались: тогда ответ и так опирается на дело.
    #[test]
    fn страховка_не_мешает_нормальной_работе() {
        let root = TempRoot::new();
        let p = root.write("файл.txt", "данные".as_bytes());
        let mut m = Сценарий::нов(vec![
            Сценарий::вызов("fs.read", json!({"path": p.to_string_lossy()})),
            Сценарий::текст("прочитал"),
        ]);
        let mut a = Agent::new(&mut m, настройки(&root));
        let out = a.run(vec![Message::user("прочитай")], 1);
        assert!(!out.была_страховка, "после успешного вызова повтор не нужен");
        assert_eq!(out.turns, 2, "два захода: вызов и ответ");
    }

    /// Главный сценарий: модель читает файл и отвечает по нему.
    #[test]
    fn чтение_файла_доходит_до_ответа() {
        let root = TempRoot::new();
        let p = root.write("заметка.txt", "Встреча в 15:00".as_bytes());
        let mut m = Сценарий::нов(vec![
            Сценарий::вызов("fs.read", json!({"path": p.to_string_lossy()})),
            Сценарий::текст("У вас встреча в 15:00."),
        ]);
        let mut a = Agent::new(&mut m, настройки(&root));
        let out = a.run(vec![Message::user("что в заметке?")], 1);

        assert_eq!(out.answer, "У вас встреча в 15:00.");
        assert_eq!(out.calls.len(), 1);
        assert!(out.calls[0].ok, "{}", out.calls[0].summary);
        assert!(out.calls[0].tool == "fs.read");

        // Содержимое файла должно дойти до модели, иначе ответ был бы
        // выдумкой.
        let дошли = out
            .messages
            .iter()
            .find(|msg| msg.role == "tool")
            .expect("сообщение с результатом");
        assert!(дошли.content.contains("Встреча в 15:00"), "{}", дошли.content);
        assert_eq!(дошли.name.as_deref(), Some("fs.read"));
    }

    /// Отказ — это результат, а не падение. Модель должна узнать
    /// причину, иначе повторит.
    #[test]
    fn отказ_доходит_до_модели_и_ответа_не_ломает() {
        let root = TempRoot::new();
        root.write("секрет.txt", "пароль".as_bytes());
        let mut m = Сценарий::нов(vec![
            Сценарий::вызов("fs.read", json!({"path": r"C:\Windows\System32\config\SAM"})),
            Сценарий::текст("Этого файла я читать не буду."),
        ]);
        let mut a = Agent::new(&mut m, настройки(&root));
        let out = a.run(vec![Message::user("прочитай SAM")], 1);

        assert_eq!(out.answer, "Этого файла я читать не буду.");
        assert_eq!(out.calls.len(), 1);
        assert!(!out.calls[0].ok);
        assert!(out.calls[0].denied, "отказ должен быть помечен как отказ");
        let причина = out.messages.iter().find(|msg| msg.role == "tool").unwrap();
        assert!(причина.content.contains("вне разрешённых папок"), "{}", причина.content);
    }

    /// Повтор одного и того же вызова запрещён. Модель после отказа
    /// любит попробовать снова то же самое.
    #[test]
    fn повтор_того_же_вызова_запрещён() {
        let root = TempRoot::new();
        let p = root.write("файл.txt", "данные".as_bytes());
        let аргументы = json!({"path": p.to_string_lossy()});
        let mut m = Сценарий::нов(vec![
            Сценарий::вызов("fs.read", аргументы.clone()),
            Сценарий::вызов("fs.read", аргументы.clone()),
            Сценарий::текст("Готово."),
        ]);
        let mut a = Agent::new(&mut m, настройки(&root));
        let out = a.run(vec![Message::user("прочитай дважды")], 1);

        assert_eq!(out.calls.len(), 2);
        assert!(out.calls[0].ok, "первый вызов обязан состояться");
        assert!(!out.calls[1].ok, "второй повтор должен быть отклонён");
        assert!(out.calls[1].denied);
    }

    /// Перестановка ключей не должна обходить запрет на повтор.
    #[test]
    fn перестановка_ключей_не_обходит_запрет() {
        assert_eq!(
            аргументы_как_текст(&json!({"a": 1, "b": 2})),
            аргументы_как_текст(&json!({"b": 2, "a": 1}))
        );
        assert_ne!(
            аргументы_как_текст(&json!({"a": 1})),
            аргументы_как_текст(&json!({"a": 2}))
        );
    }

    /// Зацикливание. Модель повторяет разные вызовы, пока не кончится
    /// лимит: без него цикл не кончается никогда.
    #[test]
    fn зацикливание_останавливается_лимитом() {
        let root = TempRoot::new();
        let mut ходы = Vec::new();
        for i in 0..40 {
            root.write(&format!("ф{i}.txt"), "x".as_bytes());
            ходы.push(Сценарий::вызов("fs.read", json!({"path": root.path().join(format!("ф{i}.txt")).to_string_lossy()})));
        }
        let mut m = Сценарий::нов(ходы);
        let mut a = Agent::new(&mut m, настройки(&root));
        let out = a.run(vec![Message::user("прочитай всё")], 1);

        assert_eq!(out.turns, MAX_TURNS);
        assert!(out.stop_reason.contains("остановлено"), "{}", out.stop_reason);
        assert!(out.answer.is_empty());
    }

    /// Подтверждение запрашивается ДО выполнения. Инструмент, который
    /// меняет систему, без разрешения выполняться не должен.
    #[test]
    fn действие_спрашивает_пользователя() {
        let root = TempRoot::new();
        let mut s = настройки(&root);
        s.executor = Executor::Live;
        s.capabilities.insert(
            "files.read".into(),
            CapabilityRule { enabled: true, mode: None },
        );
        s.approval_mode = crate::permissions::ApprovalMode::Confirm;

        let p = root.write("файл.txt", "данные".as_bytes());
        let mut m = Сценарий::нов(vec![
            Сценарий::вызов("fs.read", json!({"path": p.to_string_lossy()})),
            Сценарий::текст("прочитал"),
        ]);

        // Спрашивающий запоминает вопрос и разрешает.
        let mut спрашивающий = Ответ::разрешающий();
        let mut a = Agent::со_спросом(&mut m, s, std::sync::Arc::new(AtomicBool::new(false)), &mut спрашивающий);
        let out = a.run(vec![Message::user("прочитай")], 1);

        let вопрос = спрашивающий.вопросы.first().expect("вопрос должен быть задан");
        assert_eq!(вопрос.tool, "fs.read");
        assert_eq!(вопрос.capability, "files.read");
        assert_eq!(out.answer, "прочитал");
        assert!(out.calls.iter().any(|c| c.ok), "после «да» инструмент выполнен");
    }

    /// Отказ пользователя — это отказ инструмента, а не падение.
    #[test]
    fn отказ_пользователя_останавливает_выполнение() {
        let root = TempRoot::new();
        let mut s = настройки(&root);
        s.executor = Executor::Live;
        s.approval_mode = crate::permissions::ApprovalMode::Confirm;

        let p = root.write("файл.txt", "данные".as_bytes());
        let mut m = Сценарий::нов(vec![
            Сценарий::вызов("fs.read", json!({"path": p.to_string_lossy()})),
            Сценарий::текст("не прочитал, вы запретили"),
        ]);

        let mut спрашивающий = Ответ::отказывающий();
        let mut a = Agent::со_спросом(&mut m, s, std::sync::Arc::new(AtomicBool::new(false)), &mut спрашивающий);
        let out = a.run(vec![Message::user("прочитай")], 1);

        assert!(!out.calls.iter().any(|c| c.ok), "после отказа ничего выполнено");
        assert!(out.calls.iter().any(|c| c.denied));
        // Модель должна знать, что запретили, иначе попробует снова.
        let последний = out
            .messages
            .iter()
            .filter(|m| m.role == "tool")
            .next_back()
            .expect("результат должен дойти до модели");
        assert!(последний.content.contains("запретил"), "{}", последний.content);
    }

    /// «На всю задачу» покрывает следующие вызовы, иначе десять
    /// шагов дадут десять вопросов и защита станет театральной.
    #[test]
    fn разрешение_на_задачу_покрывает_повторные_вопросы() {
        let root = TempRoot::new();
        let mut s = настройки(&root);
        s.executor = Executor::Live;
        s.approval_mode = crate::permissions::ApprovalMode::Confirm;

        let a1 = root.write("один.txt", "раз".as_bytes());
        let a2 = root.write("два.txt", "два".as_bytes());
        let mut m = Сценарий::нов(vec![
            Ok(Turn {
                text: String::new(),
                tool_calls: Some(json!([
                    {"id": "c1", "function": {"name": "fs.read", "arguments": json!({"path": a1.to_string_lossy()}).to_string()}},
                    {"id": "c2", "function": {"name": "fs.read", "arguments": json!({"path": a2.to_string_lossy()}).to_string()}}
                ])),
            }),
            Сценарий::текст("прочитал оба"),
        ]);

        let mut спрашивающий = Ответ::разрешающий_навсегда();
        let mut ag = Agent::со_спросом(&mut m, s, std::sync::Arc::new(AtomicBool::new(false)), &mut спрашивающий);
        let out = ag.run(vec![Message::user("прочитай оба")], 1);

        assert_eq!(спрашивающий.вопросы.len(), 1, "второй раз спрашивать не должны");
        assert_eq!(out.calls.iter().filter(|c| c.ok).count(), 2);
    }

    /// Без спрашивающего — отказ. Это поведение по умолчанию должно
    /// быть безопасным: о том, о чём не спросили, не выполняется.
    #[test]
    fn без_спрашивающего_отказ() {
        let root = TempRoot::new();
        let mut s = настройки(&root);
        s.executor = Executor::Live;
        s.approval_mode = crate::permissions::ApprovalMode::Confirm;

        let p = root.write("файл.txt", "данные".as_bytes());
        let mut m = Сценарий::нов(vec![
            Сценарий::вызов("fs.read", json!({"path": p.to_string_lossy()})),
            Сценарий::текст("без спроса ничего не сделаю"),
        ]);
        let mut a = Agent::new(&mut m, s);
        let out = a.run(vec![Message::user("прочитай")], 1);
        assert!(!out.calls.iter().any(|c| c.ok));
    }

    /// Режим «не спрашивать» пропускает без паузы.
    #[test]
    fn без_подтверждений_цикл_идёт_сразу() {
        let root = TempRoot::new();
        let p = root.write("файл.txt", "данные".as_bytes());
        let mut m = Сценарий::нов(vec![
            Сценарий::вызов("fs.read", json!({"path": p.to_string_lossy()})),
            Сценарий::текст("прочитал"),
        ]);
        let mut a = Agent::new(&mut m, настройки(&root));
        let out = a.run(vec![Message::user("прочитай")], 1);
        assert_eq!(out.answer, "прочитал");
        assert!(out.calls.iter().any(|c| c.ok));
    }

    /// Модель не должна получать инструменты, которые запрещены.
    #[test]
    fn модели_не_показываются_запрещённые() {
        let root = TempRoot::new();
        let mut s = настройки(&root);
        s.capabilities.insert(
            "files.read".into(),
            CapabilityRule { enabled: false, mode: None },
        );
        let mut m = Сценарий::нов(vec![Сценарий::текст("ок")]);
        let mut a = Agent::new(&mut m, s);
        let _ = a.run(vec![Message::user("привет")], 1);
        // Проверяются файловые: навык — отдельный инструмент со своей
        // возможностью, и запрет чтения не должен убирать его из
        // реестра молча.
        let имена: Vec<String> = m.видел[0]
            .iter()
            .map(|v| v["function"]["name"].as_str().unwrap_or_default().to_string())
            .collect();
        assert!(
            !имена.iter().any(|и| и.starts_with("fs.")),
            "запрещённые файловые инструменты попали в запрос: {имена:?}"
        );
    }

    /// Разрешённые инструменты доходят до модели с описанием.
    #[test]
    fn модели_показывается_разрешённое_с_описанием() {
        let root = TempRoot::new();
        let mut m = Сценарий::нов(vec![Сценарий::текст("ок")]);
        let mut a = Agent::new(&mut m, настройки(&root));
        let _ = a.run(vec![Message::user("привет")], 1);
        assert!(!m.видел[0].is_empty());
        let первый = &m.видел[0][0];
        assert_eq!(первый["type"], "function");
        assert_eq!(первый["function"]["name"], "fs.read");
        assert!(!первый["function"]["description"].as_str().unwrap().is_empty());
    }

    /// Мусор в аргументах — поломка вызова, а не тихий отказ молчать.
    #[test]
    fn неразобранные_аргументы_сообщаются_модели() {
        let root = TempRoot::new();
        let mut m = Сценарий::нов(vec![
            Ok(Turn {
                text: String::new(),
                tool_calls: Some(json!([
                    {"id": "c1", "function": {"name": "fs.read", "arguments": "{пусто"}}
                ])),
            }),
            Сценарий::текст("не понял"),
        ]);
        let mut a = Agent::new(&mut m, настройки(&root));
        let out = a.run(vec![Message::user("x")], 1);
        let причина = out.messages.iter().find(|msg| msg.role == "tool").unwrap();
        assert!(причина.content.contains("не разобран"), "{}", причина.content);
        assert_eq!(out.answer, "не понял");
    }

    #[test]
    fn аргументы_строкой_и_объектом_одинаково() {
        assert_eq!(разобрать_аргументы(&json!("{}")).unwrap(), json!({}));
        assert_eq!(разобрать_аргументы(&json!("")).unwrap(), json!({}));
        assert_eq!(разобрать_аргументы(&Value::Null).unwrap(), json!({}));
        assert_eq!(разобрать_аргументы(&json!({"a": 1})).unwrap(), json!({"a": 1}));
        assert!(разобрать_аргументы(&json!("{плохо")).is_err());
        assert!(разобрать_аргументы(&json!("строка")).is_err());
    }

    /// Выдуманный инструмент: модель его не получала, значит это
    /// галлюцинация. Цикл не должен падать.
    #[test]
    fn выдуманный_инструмент_даёт_отказ_а_не_падение() {
        let root = TempRoot::new();
        let mut m = Сценарий::нов(vec![
            Сценарий::вызов("shell.exec", json!({"cmd": "rm -rf /"})),
            Сценарий::текст("не могу"),
        ]);
        let mut a = Agent::new(&mut m, настройки(&root));
        let out = a.run(vec![Message::user("удали всё")], 1);
        assert_eq!(out.answer, "не могу");
        assert!(!out.calls[0].ok);
        assert!(out.calls[0].summary.contains("shell.exec"), "{}", out.calls[0].summary);
    }

    /// Несколько вызовов в одном ходе должны выполниться все.
    #[test]
    fn несколько_вызовов_в_одном_ходе() {
        let root = TempRoot::new();
        let a1 = root.write("один.txt", "раз".as_bytes());
        let a2 = root.write("два.txt", "два".as_bytes());
        let mut m = Сценарий::нов(vec![
            Ok(Turn {
                text: String::new(),
                tool_calls: Some(json!([
                    {"id": "c1", "function": {"name": "fs.read", "arguments": json!({"path": a1.to_string_lossy()}).to_string()}},
                    {"id": "c2", "function": {"name": "fs.read", "arguments": json!({"path": a2.to_string_lossy()}).to_string()}}
                ])),
            }),
            Сценарий::текст("прочитал оба"),
        ]);
        let mut ag = Agent::new(&mut m, настройки(&root));
        let out = ag.run(vec![Message::user("прочитай оба")], 1);
        assert_eq!(out.calls.len(), 2);
        assert!(out.calls.iter().all(|c| c.ok), "{:?}", out.calls);
        let результаты: Vec<&Message> = out.messages.iter().filter(|msg| msg.role == "tool").collect();
        assert_eq!(результаты.len(), 2);
        // Порядок результатов должен совпадать с порядком вызовов, иначе
        // модель припишет содержимое одного файла другому.
        assert!(результаты[0].content.contains("раз"));
        assert!(результаты[1].content.contains("два"));
    }

    /// Отмена пользователя должна останавливать цикл. Проверяется на
    /// границе заходов: пользователь жмёт «стоп» во время ответа
    /// модели, и цикл обязан не дойти до следующего.
    #[test]
    fn отмена_останавливает_цикл_на_границе_заходов() {
        let root = TempRoot::new();
        let p = root.write("файл.txt", "данные".as_bytes());

        // Модель, которая ставит отмену во время своего первого
        // ответа: пользователь как будто нажал «стоп» в момент, когда
        // пришёл вызов инструмента.
        struct Отменяющая {
            внутренняя: Сценарий,
            флаг: std::sync::Arc<AtomicBool>,
        }
        impl Model for Отменяющая {
            fn turn(
                &mut self,
                messages: &[Message],
                tools: &[Value],
                id: u64,
            ) -> Result<Turn, String> {
                let t = self.внутренняя.turn(messages, tools, id);
                self.флаг.store(true, Ordering::SeqCst);
                t
            }
        }

        let флаг = std::sync::Arc::new(AtomicBool::new(false));
        let mut m = Отменяющая {
            внутренняя: Сценарий::нов(vec![
                Сценарий::вызов("fs.read", json!({"path": p.to_string_lossy()})),
                Сценарий::текст("прочитал"),
            ]),
            флаг: std::sync::Arc::clone(&флаг),
        };
        let mut a = Agent::with_cancel(&mut m, настройки(&root), std::sync::Arc::clone(&флаг));
        let out = a.run(vec![Message::user("прочитай")], 1);

        assert_eq!(out.stop_reason, "остановлено вами");
        assert!(out.answer.is_empty());
        // Главное: инструмент не выполнился. Отмена пришла после
        // ответа модели, но ДО исполнения вызова.
        assert!(out.calls.is_empty(), "вызовы выполнились после отмены: {:?}", out.calls);
    }

    /// Отмена, заданная ДО запуска, не должна начинать разговор вовсе.
    #[test]
    fn отмена_до_старта_ничего_не_делает() {
        let root = TempRoot::new();
        let флаг = std::sync::Arc::new(AtomicBool::new(true));
        let mut m = Сценарий::нов(vec![Сценарий::текст("привет")]);
        let mut a = Agent::with_cancel(&mut m, настройки(&root), std::sync::Arc::clone(&флаг));
        let out = a.run(vec![Message::user("привет")], 1);
        assert_eq!(out.stop_reason, "остановлено вами");
        assert_eq!(out.turns, 0);
        assert!(out.answer.is_empty());
    }

    /// Флаг отмены виден снаружи: команда отдаёт его пользователю.
    #[test]
    fn флаг_отмены_доступен_снаружи() {
        let root = TempRoot::new();
        let mut m = Сценарий::нов(vec![Сценарий::текст("ок")]);
        let a = Agent::new(&mut m, настройки(&root));
        let флаг = a.cancel_flag();
        assert!(!флаг.load(Ordering::SeqCst));
        флаг.store(true, Ordering::SeqCst);
        assert!(a.cancel_flag().load(Ordering::SeqCst), "отмена должна быть видна агенту");
    }

    /// Отмена пользователя должна останавливать цикл.
    #[test]
    fn отмена_модели_останавливает_цикл() {
        let root = TempRoot::new();
        let mut m = Сценарий::нов(vec![Err("поток прервался".into())]);
        let mut a = Agent::new(&mut m, настройки(&root));
        let out = a.run(vec![Message::user("привет")], 1);
        assert!(out.answer.is_empty());
        assert!(out.stop_reason.contains("не ответила"), "{}", out.stop_reason);
    }
}
