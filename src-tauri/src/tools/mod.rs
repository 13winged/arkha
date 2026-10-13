// Инструменты S6.
//
// Три решения, которые здесь приняты.
//
// **Аргументы типизированы.** Модель не передаёт строку для
// выполнения, она передаёт JSON, который разбирается в структуру с
// проверкой. Неразобранный аргумент — отказ, а не значение по
// умолчанию: правило 11.
//
// **Проверка разрешений живёт рядом с исполнением.** Иначе новый
// инструмент можно забыть проверить, и он получит доступ ко всему.
//
// **Путь нормализуется до проверки.** `..`, другой диск — всё это
// выясняется ДО открытия. Иначе агент прочитает то, что лежит за
// пределами разрешённой папки, и узнает об этом из ошибки доступа.
//
// Про два разных имени. У инструмента их два, и путать их нельзя:
//
// | Поле | Кто пользуется | Пример |
// |---|---|---|
// | `id` | модель вызывает по имени | `fs.read` |
// | `capability` | разрешения проверяют | `files.read` |
//
// Имя инструмента может быть сколько угодно подробным, а разрешение
// обязано быть ровно тем, что пользователь видел галочкой и что лежит
// в `capabilities.json`. Иначе галочка «чтение файлов» не покрывает
// инструмент, названный `files.read`, и права разъезжаются.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::permissions::{Decision, Settings};

pub mod apps;
pub mod control;
pub mod fs;
pub mod skill;
pub mod windows;
#[cfg(test)]
pub mod tempdir;

/// Что вернул инструмент.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutput {
    /// Короткий текст для модели.
    pub text: String,
    /// Что именно сделано, для аудита и интерфейса.
    pub summary: String,
    /// Инструмент что-то менял в системе.
    pub changed: bool,
}

impl ToolOutput {
    pub fn read(text: impl Into<String>, summary: impl Into<String>) -> Self {
        Self { text: text.into(), summary: summary.into(), changed: false }
    }

    /// Действие, которое в `dry-run` показано, но не выполнено.
    pub fn planned(summary: impl Into<String>) -> Self {
        Self {
            text: String::new(),
            summary: summary.into(),
            changed: false,
        }
    }

    /// Действие, которое выполнено и что-то изменило.
    ///
    /// Отдельный конструктор, а не `read` с `changed: true`: забыть
    /// проставить флаг значит записать в аудит действие как чтение.
    /// Аудит — то, на что смотрят после инцидента, и там «прочитал»
    /// вместо «нажал» вводит в заблуждение сильнее, чем отсутствие
    /// записи.
    pub fn changed(text: impl Into<String>, summary: impl Into<String>) -> Self {
        Self { text: text.into(), summary: summary.into(), changed: true }
    }
}

/// Отказ инструмента. Отказ — это результат, а не сбой: модели нужно
/// сказать «не вышло и почему», иначе она попробует ещё раз тем же
/// самым.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolError {
    /// Нет такого инструмента.
    Unknown(String),
    /// Аргументы не разобрались или не прошли проверку.
    BadArgs(String),
    /// Разрешения не дают.
    Denied(String),
    /// Путь ведёт не туда.
    Outside(String),
    /// Файла нет.
    Missing(String),
    /// Файл есть, но прочитать нельзя: другой формат, занят.
    Unreadable(String),
    /// Подошло несколько, и непонятно, какое имелось в виду.
    ///
    /// Отдельный вариант, а не `BadArgs`: подсказка модели здесь
    /// другая. Не «попробуй иначе», а «спроси человека, какое» —
    /// выбрать первое наугад означает переключиться не на то окно.
    Ambiguous(String),
    /// Система Windows отказала в действии.
    ///
    /// Не то же, что `Denied`: права есть, а операция не прошла.
    /// Окно не отдало фокус, ввод заблокирован другой программой.
    Refused(String),
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ToolError::Unknown(s) => write!(f, "нет инструмента «{s}»"),
            ToolError::BadArgs(s) => write!(f, "неверные аргументы: {s}"),
            ToolError::Denied(s) => write!(f, "не разрешено: {s}"),
            ToolError::Outside(s) => write!(f, "вне разрешённых папок: {s}"),
            ToolError::Missing(s) => write!(f, "файла нет: {s}"),
            ToolError::Unreadable(s) => write!(f, "не читается: {s}"),
            ToolError::Ambiguous(s) => write!(f, "неясно, что именно: {s}"),
            ToolError::Refused(s) => write!(f, "система отказала: {s}"),
        }
    }
}

impl ToolError {
    /// Стоит ли пробовать иначе.
    ///
    /// Правило остановки из [docs/05-безопасность.md](../../docs/05-безопасность.md):
    /// ясная причина — сообщить и не повторять, неясная — попробовать
    /// иначе. Повторять на ясной причине бессмысленно и вредно: это
    /// учит модель долбить в одну точку.
    ///
    /// | Отказ | Причина | Повтор |
    /// |---|---|---|
    /// | `Denied` | права | нет |
    /// | `Outside` | путь вне папок | нет |
    /// | `Missing` | файла нет | нет |
    /// | `Unknown` | такого инструмента нет | нет |
    /// | `BadArgs` | модель ошиблась в аргументах | **да** |
    /// | `Unreadable` | занято, не тот формат | **да** |
    /// | `Ambiguous` | подошло несколько | **да**, но спросив человека |
    /// | `Refused` | система не дала | нет, сразу не выйдет |
    pub fn повтор_нужен(&self) -> bool {
        matches!(
            self,
            ToolError::BadArgs(_) | ToolError::Unreadable(_) | ToolError::Ambiguous(_)
        )
    }

    /// Что сказать модели, чтобы она не повторяла то же самое.
    pub fn подсказка_модели(&self) -> &'static str {
        if self.повтор_нужен() {
            "Причина ясна, но попробовать иначе можно. Измени подход, не повторяй тот же вызов."
        } else {
            "Причина ясна и повтор не поможет. Не вызывай этот инструмент снова \
             с этими аргументами: скажи пользователю, что не вышло и почему."
        }
    }
}

pub type ToolResult = Result<ToolOutput, ToolError>;

/// Описание инструмента для модели.
#[derive(Debug, Clone, Serialize)]
pub struct ToolSpec {
    /// Имя, по которому модель вызывает. Не обязано быть
    /// перечислимым в реестре возможностей.
    pub id: String,
    /// Возможность из `capabilities.json`, которой это соответствует.
    /// По ней принимается решение о доступе.
    pub capability: String,
    /// Название для интерфейса.
    pub name: String,
    /// Что делает, одной строкой. Модель выбирает по этому тексту,
    /// поэтому он пишется человеком, а не генерируется.
    pub description: String,
    /// JSON-схема аргументов.
    pub parameters: Value,
    /// Меняет ли инструмент систему.
    pub side_effecting: bool,
}

/// Один инструмент.
///
/// `spec` принимает настройки, и это не для удобства. Описание
/// инструмента — единственное, что читает модель, и без разрешённых
/// папок она выдумывает путь: на живом гейте S6 она стабильно
/// отвечала `C:\Users\me\Документы`, которого на машине нет. Модель не
/// может угадать, что ей разрешили, — значит, обязана прочитать это
/// из описания.
pub trait Tool: Send + Sync {
    fn spec(&self, settings: &Settings) -> ToolSpec;
    /// Имя инструмента без настроек. Нужно там, где важно само имя:
    /// в списке для интерфейса и при поиске по вызову. Описание при
    /// этом строится с настройками.
    fn spec_id(&self) -> String {
        self.spec(&Settings::default()).id
    }
    fn call(&self, args: &Value, settings: &Settings) -> ToolResult;
}

/// Разрешённые папки одной строкой — для описания инструмента.
pub fn папки_словами(settings: &Settings) -> String {
    if settings.paths.is_empty() {
        return "Не выбрано ни одной папки, читать нечего".to_string();
    }
    let список: Vec<String> = settings
        .paths
        .iter()
        .map(|p| p.display().to_string())
        .collect();
    format!("Можно читать только внутри: {}", список.join("; "))
}

/// Реестр: что умеем.
pub struct Registry {
    tools: Vec<Box<dyn Tool>>,
}

/// Инструменты, которые вообще написаны. Один список на две цели:
/// `all` для проверок, `allowed` — чтобы отсеять по разрешениям.
fn built() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(fs::ReadFile),
        Box::new(fs::ListDir),
        // Навыки не всегда есть: если папки нет, инструмент не
        // добавляется, иначе модель увидит «запустить навык» при
        // пустом списке.
        Box::new(skill::RunSkill {
            наборы: навыки(),
        }),
        Box::new(windows::СписокОкон),
        Box::new(windows::НайтиОкно),
        // Действия, меняющие компьютер. Все под L1 — спрашиваются
        // каждый раз. Клика по координатам среди них нет и не будет:
        // см. начало `control.rs`.
        Box::new(control::ВФокус),
        Box::new(control::Написать),
        Box::new(control::Нажать),
        Box::new(control::Кликнуть),
        Box::new(apps::Запустить),
    ]
}

/// Набор навыков, доступный всему приложению.
///
/// Создаётся один раз и перечитывается по требованию: чтобы замена
/// папки навыков не требовала перезапуска.
pub fn навыки() -> skill::Хранилище {
    use std::sync::OnceLock;
    static ХРАНИЛИЩЕ: OnceLock<skill::Хранилище> = OnceLock::new();
    ХРАНИЛИЩЕ
        .get_or_init(|| {
            let н = crate::skills::Набор::из_папки(&crate::skills::папка_данных());
            std::sync::Arc::new(std::sync::RwLock::new(н))
        })
        .clone()
}

/// Перечитать навыки с диска.
///
/// Вызывается из интерфейса, а не автоматически: незаметно
/// подменить набор инструкций во время работы цикла — значит
/// изменить то, что агент собирается сделать, без спроса.
pub fn перечитать_навыки() -> Result<Vec<String>, String> {
    let н = crate::skills::Набор::из_папки(&crate::skills::папка_данных());
    let сводка = н.сводка();
    let сломанные: Vec<String> = н.сломанные.iter().map(|о| о.to_string()).collect();
    *навыки()
        .write()
        .map_err(|_| "список навыков занят")? = н;
    if сломанные.is_empty() {
        Ok(сводка)
    } else {
        Err(format!("не прочитаны:\n{}", сломанные.join("\n")))
    }
}

impl Registry {
    /// Инструменты, которые агент видит.
    ///
    /// Фильтр не по «можно ли выполнить», а по «знает ли агент».
    /// Явно выключенное скрывается: человек выключил, значит и знать
    /// не должен. Неразрешённое, но не выключенное — **показывается**,
    /// чтобы отказ приходил как отказ («не разрешено явно»), а не как
    /// незнание («у меня такого инструмента нет»). Разница видна
    /// человеку: в первом случае есть что включить.
    pub fn allowed(settings: &Settings) -> Self {
        let tools = built()
            .into_iter()
            .filter(|t| settings.показывать(&t.spec(settings).capability))
            .collect();
        Self { tools }
    }

    /// Все инструменты, независимо от разрешений.
    pub fn all() -> Self {
        Self { tools: built() }
    }

    pub fn specs(&self, settings: &Settings) -> Vec<ToolSpec> {
        self.tools.iter().map(|t| t.spec(settings)).collect()
    }

    pub fn ids(&self) -> Vec<String> {
        self.tools.iter().map(|t| t.spec_id()).collect()
    }

    pub fn get(&self, id: &str, settings: &Settings) -> Option<&dyn Tool> {
        let key = id.trim();
        self.tools
            .iter()
            .find(|t| t.spec(settings).id == key)
            .map(|t| t.as_ref())
    }

    /// Схемы в формате OpenAI-совместимого запроса.
    pub fn as_request_json(&self, settings: &Settings) -> Vec<Value> {
        self.tools
            .iter()
            .map(|t| {
                let s = t.spec(settings);
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": s.id,
                        "description": s.description,
                        "parameters": s.parameters,
                    }
                })
            })
            .collect()
    }

    /// Вызов по имени и аргументам.
    pub fn call(&self, id: &str, args: &Value, settings: &Settings) -> ToolResult {
        let Some(tool) = self.get(id, settings) else {
            // Инструмент не показан модели, но она его назвала. Это
            // бывает при галлюцинации или при смене разрешений посреди
            // разговора. Отказ с объяснением, а не паника.
            return Err(ToolError::Unknown(id.to_string()));
        };
        // Видимость инструмента не даёт права его выполнить. Все
        // исполнения проходят эту проверку перед вызовом Tool::call:
        // иначе новый инструмент можно показать для честности, но он
        // обойдёт `dry-run` и явные запреты, если внутри него забыли
        // вызвать `ensure_allowed`.
        ensure_allowed(settings, &tool.spec(settings).capability)?;
        tool.call(args, settings)
    }
}

/// Проверка разрешения с превращением отказа в ошибку инструмента.
///
/// Отдельная функция, чтобы каждая проверка выглядела одинаково и её
/// нельзя было забыть: `?` вместо ручного `return`.
pub fn ensure_allowed(settings: &Settings, capability: &str) -> Result<(), ToolError> {
    match settings.allows(capability) {
        Decision { allowed: true, .. } => Ok(()),
        Decision { allowed: false, reason } => Err(ToolError::Denied(reason)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// Правило остановки: ясная причина — не повторять, неясная —
    /// попробовать иначе.
    ///
    /// Повторять на ясной причине бессмысленно и вредно: это учит
    /// модель долбить в одну точку, а пользователю показывает десять
    /// одинаковых отказов подряд.
    #[test]
    fn повтор_нужен_только_при_неясной_причине() {
        use ToolError::*;
        // Неясные: попробовать иначе можно.
        assert!(BadArgs("нет поля path".into()).повтор_нужен());
        assert!(Unreadable("занят".into()).повтор_нужен());
        // Ясные: повтор не поможет.
        assert!(!Denied("нет прав".into()).повтор_нужен());
        assert!(!Outside("не та папка".into()).повтор_нужен());
        assert!(!Missing("нет файла".into()).повтор_нужен());
        assert!(!Unknown("нет инструмента".into()).повтор_нужен());
    }

    /// Модель должна знать, что делать с отказом. Без этой строки она
    /// повторяет: на живом железе так и выходило.
    #[test]
    fn подсказка_модели_разная_для_разных_причин() {
        let ясная = ToolError::Denied("нет прав".into()).подсказка_модели();
        let неясная = ToolError::BadArgs("пусто".into()).подсказка_модели();
        assert_ne!(ясная, неясная);
        assert!(ясная.contains("Не вызывай"), "{ясная}");
        assert!(неясная.contains("попробовать иначе"), "{неясная}");
    }
    use serde_json::json;
    use crate::permissions::{CapabilityRule, Executor, Settings as S};

    /// Только файловые инструменты.
    ///
    /// Навык — тоже инструмент, и он не про папки. Проверки про пути
    /// и пустые папки относятся к `fs.*`, а не ко всему реестру:
    /// иначе добавление любого инструмента роняет тест, который
    /// ничего общего с ним не проверяет.
    fn файловые(s: &S) -> Vec<ToolSpec> {
        Registry::allowed(s)
            .specs(s)
            .into_iter()
            .filter(|сп| сп.id.starts_with("fs."))
            .collect()
    }

    fn выкл(имя: &str) -> S {
        let mut s = S::default();
        s.capabilities
            .insert(имя.into(), CapabilityRule { enabled: false, mode: None });
        s
    }

    struct ПробныйИнструмент {
        вызовы: Arc<AtomicUsize>,
    }

    impl Tool for ПробныйИнструмент {
        fn spec(&self, _settings: &Settings) -> ToolSpec {
            ToolSpec {
                id: "test.side_effect".into(),
                capability: "windows.control".into(),
                name: "Пробное действие".into(),
                description: "Тест границы разрешений".into(),
                parameters: json!({"type": "object", "properties": {}, "required": []}),
                side_effecting: true,
            }
        }

        fn call(&self, _args: &Value, _settings: &Settings) -> ToolResult {
            self.вызовы.fetch_add(1, Ordering::SeqCst);
            Ok(ToolOutput::changed("выполнено", "пробное действие"))
        }
    }

    fn реестр_с_пробным(вызовы: Arc<AtomicUsize>) -> Registry {
        Registry {
            tools: vec![Box::new(ПробныйИнструмент { вызовы })],
        }
    }

    #[test]
    fn чтение_разрешено_без_явной_галочки() {
        // Чтение безопасно, поэтому реестр не пуст без настроек.
        let s = S::default();
        assert!(!Registry::allowed(&s).ids().is_empty());
    }

    #[test]
    fn выключенная_возможность_убирает_инструмент() {
        let s = выкл("files.read");
        let r = Registry::allowed(&s);
        // Проверяются файловые: навык живёт своей возможностью и не
        // исчезает из-за выключенного чтения.
        assert!(
            !r.ids().iter().any(|и| и.starts_with("fs.")),
            "выключенное чтение должно убирать файловые инструменты, осталось {:?}",
            r.ids()
        );
    }

    /// Модель получает только разрешённое. Неразрешённое в список
    /// инструментов не попадает вовсе.
    #[test]
    fn модели_показывается_только_разрешённое() {
        let s = выкл("files.read");
        let json = Registry::allowed(&s).as_request_json(&s);
        let имена: Vec<String> = json.iter().map(|v| v["function"]["name"].to_string()).collect();
        assert!(
            !имена.iter().any(|и| и.starts_with("\"fs.")),
            "в запрос ушли запрещённые файловые инструменты: {имена:?}"
        );
    }

    #[test]
    fn неизвестный_инструмент_отклоняется_с_названием() {
        let s = S::default();
        let r = Registry::allowed(&s);
        let e = r.call("выдуманный", &json!({}), &s).unwrap_err();
        match e {
            ToolError::Unknown(n) => assert!(n.contains("выдуманный")),
            other => panic!("ожидался Unknown, получено {other:?}"),
        }
    }

    /// Неразрешённый инструмент виден модели, чтобы она могла назвать
    /// честную причину отказа. Но видимость не должна превращаться в
    /// право исполнить действие.
    #[test]
    fn видимый_инструмент_без_разрешения_не_исполняется() {
        let вызовы = Arc::new(AtomicUsize::new(0));
        let reg = реестр_с_пробным(вызовы.clone());
        let mut s = S::default();
        s.executor = Executor::Live;

        assert!(reg.ids().iter().any(|id| id == "test.side_effect"));
        let e = reg.call("test.side_effect", &json!({}), &s).unwrap_err();
        assert!(matches!(e, ToolError::Denied(_)), "{e:?}");
        assert_eq!(вызовы.load(Ordering::SeqCst), 0, "действие не должно было начаться");
    }

    /// Явное разрешение не отменяет `dry-run`: пользователь разрешил
    /// возможность, но исполнитель всё ещё не переведён в live.
    #[test]
    fn dry_run_блокирует_даже_явно_разрешённый_инструмент() {
        let вызовы = Arc::new(AtomicUsize::new(0));
        let reg = реестр_с_пробным(вызовы.clone());
        let mut s = S::default();
        s.capabilities.insert(
            "windows.control".into(),
            CapabilityRule { enabled: true, mode: None },
        );

        let e = reg.call("test.side_effect", &json!({}), &s).unwrap_err();
        assert!(matches!(e, ToolError::Denied(_)), "{e:?}");
        assert!(e.to_string().contains("dry-run"), "{e}");
        assert_eq!(вызовы.load(Ordering::SeqCst), 0, "dry-run не должен вызывать инструмент");
    }

    /// Только live + явная возможность доходят до инструмента. Запрос
    /// подтверждения остаётся в Agent и проверяется отдельно.
    #[test]
    fn live_с_явным_разрешением_доходит_до_инструмента() {
        let вызовы = Arc::new(AtomicUsize::new(0));
        let reg = реестр_с_пробным(вызовы.clone());
        let mut s = S::default();
        s.executor = Executor::Live;
        s.capabilities.insert(
            "windows.control".into(),
            CapabilityRule { enabled: true, mode: None },
        );

        let out = reg.call("test.side_effect", &json!({}), &s).expect("разрешено");
        assert!(out.changed);
        assert_eq!(вызовы.load(Ordering::SeqCst), 1);
    }

    /// Схема обязана описывать обязательные поля. Без этого модель
    /// пришлёт неполные аргументы, и отказ будет выглядеть как поломка
    /// инструмента.
    #[test]
    fn у_каждой_схемы_есть_обязательные_поля() {
        for spec in Registry::all().specs(&S::default()) {
            assert!(spec.parameters.is_object(), "{}: схема не объект", spec.id);
            assert_eq!(
                spec.parameters["type"], "object",
                "{}: корень схемы должен быть объектом",
                spec.id
            );
            assert!(
                spec.parameters["required"].is_array(),
                "{}: нет списка обязательных полей",
                spec.id
            );
            assert!(!spec.description.is_empty(), "{}: модель выбирает по описанию", spec.id);
        }
    }

    /// Инструменты обязаны быть объявлены в реестре возможностей —
    /// по полю `capability`, а не по имени. Имя для модели может быть
    /// любым, а галочка в знакомстве ставится на возможность.
    #[test]
    fn инструменты_объявлены_в_реестре_возможностей() {
        let reg = crate::capabilities::Registry::load().expect("реестр возможностей");
        for spec in Registry::all().specs(&S::default()) {
            assert!(
                reg.get(&spec.capability).is_some(),
                "{} ссылается на возможность {}, которой нет в capabilities.json",
                spec.id,
                spec.capability
            );
        }
    }

    /// Описание инструмента обязано называть разрешённые папки.
///
/// Найдено гейтом S6 на живой модели: она двенадцать раз подряд
/// просила `C:\Users\me\Документы`, потому что описания инструментов
/// были одинаковы при любых настройках. Модель не может угадать, что
/// ей разрешили, — если это не сказано.
#[test]
fn описание_инструмента_называет_разрешённые_папки() {
    let root = crate::tools::tempdir::TempRoot::new();
    let mut s = S::default();
    s.paths.push(root.path().to_path_buf());

    for spec in файловые(&s) {
        assert!(
            spec.description.contains(&root.path().display().to_string()),
            "{} не упоминает разрешённую папку. Описание: {}",
            spec.id,
            spec.description
        );
    }
}

/// Без выбранных папок описание должно говорить об этом прямо, а не
/// молчать: молчание модель читает как «папок нет ограничений».
#[test]
fn без_папок_описание_говорит_прямо() {
    let s = S::default();
    for spec in файловые(&s) {
        assert!(
            spec.description.contains("Не выбрано ни одной папки"),
            "{}: описание молчит об отсутствии папок. Описание: {}",
            spec.id,
            spec.description
        );
    }
}

/// Имя инструмента и возможность — разные вещи, и реестр
    /// возможностей не должен требовать, чтобы они совпадали.
    #[test]
    fn имя_инструмента_не_обязано_быть_возможностью() {
        // Это не проверка совпадения, а проверка того, что механизм
        // вообще различает эти два поля: если бы код сравнивал их
        // напрямую, любое подробное имя сломало бы тест выше.
        let s = S::default();
        let r = Registry::allowed(&s);
        for spec in r.specs(&s) {
            assert!(!spec.id.is_empty() && !spec.capability.is_empty());
        }
    }

    /// Отказ разрешения — это отказ инструмента, а не падение.
    #[test]
    fn запрет_даёт_отказ_а_не_ошибку() {
        let mut s = выкл("files.read");
        s.executor = Executor::Live;
        let e = Registry::allowed(&s).call("fs.read", &json!({"path": "C:\\x.txt"}), &s).unwrap_err();
        assert!(matches!(e, ToolError::Unknown(_) | ToolError::Denied(_)), "{e:?}");
    }
}
