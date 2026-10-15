// Команды знакомства.
//
// Знакомство — единственный сценарий, который меняет систему, и
// меняет не сразу. Порядок вызовов: `onboarding_step` → `onboarding_say`
// → `onboarding_apply`. Третий вызов требует отдельного нажатия и
// ничего не применяет сам.
//
// Почему не «собрал и сразу записал»: разрешения нельзя отменить
// задним числом. Человек должен увидеть, что подтверждает.

use serde::Serialize;
use std::sync::Mutex;

use crate::onboarding::{self, Step, State};
use crate::permissions::{ApprovalMode, Executor};
use crate::permissions_commands::view;

/// Знание о текущем шаге — для интерфейса и для промпта.
#[derive(Debug, Clone, Serialize)]
pub struct StepView {
    pub step: String,
    pub title: String,
    pub index: usize,
    pub total: usize,
    /// Что агент соберёт на этом шаге.
    pub what: &'static str,
    /// Готовый текст для модели. Отдаётся из Rust, чтобы промпт
    /// жил рядом с данными возможностей, а не в разметке.
    pub prompt: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Progress {
    pub step: String,
    pub title: String,
    pub index: usize,
    pub total: usize,
    pub finished: bool,
    /// Что агент понял, словами. Показывается пользователю.
    pub summary: String,
    /// Настройки, которые получатся, если применить.
    pub preview: crate::permissions_commands::SettingsView,
}

fn describe(step: Step) -> &'static str {
    describe_lang(step, false)
}

/// Подпись шага на выбранном языке.
///
/// Тот же список, что и `Step::title`, но словами проще: заголовок
/// называет шаг, а подпись — что именно на нём решается.
fn describe_lang(step: Step, en: bool) -> &'static str {
    if en {
        return match step {
            Step::Introduction => "the name you answer to",
            Step::Tools => "which tools to turn on",
            Step::Folders => "which folders to search for documents",
            Step::Levels => "what needs no confirmation",
            Step::Summary => "check it and finish",
        };
    }
    match step {
        Step::Introduction => "имя, которым тебя зовут",
        Step::Tools => "какие инструменты включать",
        Step::Folders => "в каких папках искать документы",
        Step::Levels => "что можно без подтверждения",
        Step::Summary => "проверить и закончить",
    }
}

/// Название шага на выбранном языке.
fn title_lang(step: Step, en: bool) -> String {
    if en {
        match step {
            Step::Introduction => "Welcome".to_string(),
            Step::Tools => "Tools".to_string(),
            Step::Folders => "Folders".to_string(),
            Step::Levels => "Access levels".to_string(),
            Step::Summary => "Summary".to_string(),
        }
    } else {
        step.title().to_string()
    }
}

/// Состояние живёт в приложении, а не в файле: незаконченное
/// знакомство не должно переживать перезапуск. Пользователь, который
/// закрыл окно на середине, начнёт заново — это меньше, чем
/// разбираться, что он успел разрешить.
fn state() -> &'static Mutex<Option<State>> {
    static STATE: std::sync::OnceLock<Mutex<Option<State>>> = std::sync::OnceLock::new();
    STATE.get_or_init(|| Mutex::new(None))
}

// --- чистое ядро ---------------------------------------------------------
// Логика отделена от глобального состояния, иначе тесты делят одно
// на всех и падают в зависимости от порядка. Команды ниже — тонкая
// обёртка над этими функциями.

pub fn view_of(s: &State, tier: crate::profile::Tier) -> StepView {
    view_of_lang(s, tier, false)
}

/// Вид шага на выбранном языке.
///
/// Промпт модели остаётся русским при любом языке интерфейса: модель
/// работает по-русски, и перевод промпта означал бы, что на вопросы
/// человека ей приходит инструкция на другом языке. Переводится то,
/// что читает человек.
pub fn view_of_lang(s: &State, tier: crate::profile::Tier, en: bool) -> StepView {
    let index = Step::ALL.iter().position(|x| *x == s.step).unwrap_or(0);
    StepView {
        step: s.step.as_str().to_string(),
        // Название берётся из самого шага: копия в интерфейсе
        // рано или поздно разъехалась бы с логикой.
        title: title_lang(s.step, en),
        index: index + 1,
        total: Step::ALL.len(),
        what: describe_lang(s.step, en),
        prompt: s.step.prompt(tier),
    }
}

pub fn preview_of(s: &State) -> Preview {
    preview_of_lang(s, false)
}

pub fn preview_of_lang(s: &State, en: bool) -> Preview {
    // Настройки читаются один раз и передаются дальше: `summary`
    // и `to_settings` по отдельности читали бы файл дважды, и
    // между этими двумя чтениями он мог бы измениться.
    let база = crate::permissions::Settings::load().unwrap_or_default();
    Preview {
        summary: s.summary_из(база.clone()),
        settings: crate::permissions_commands::view_lang(&s.to_settings_из(база), en),
    }
}

/// Предпросмотр на заданной базе — для тестов, которые не должны
/// смотреть в настоящий файл человека.
pub fn preview_из(s: &State, база: crate::permissions::Settings) -> Preview {
    Preview {
        summary: s.summary_из(база.clone()),
        settings: crate::permissions_commands::view_lang(&s.to_settings_из(база), false),
    }
}

pub fn say_to(s: &mut State, tier: crate::profile::Tier, text: &str) -> SayResult {
    say_to_lang(s, tier, text, false)
}

pub fn say_to_lang(s: &mut State, tier: crate::profile::Tier, text: &str, en: bool) -> SayResult {
    let understood = s.apply(text);
    SayResult {
        understood,
        // При неудаче шаг не сдвинулся, поэтому повторяется тот же
        // вопрос, а не следующий.
        next: view_of_lang(s, tier, en),
        summary: s.summary(),
        finished: s.finished,
    }
}

/// Можно ли применять. Отдельно от `apply`, потому что проверка
/// безопасна, а применение пишет в файл, и проверять её на живом
/// гейте нельзя.
pub fn check_applicable(s: &State) -> Result<(), String> {
    if !s.finished {
        return Err("знакомство не закончено: сначала дойди до итога".into());
    }
    Ok(())
}

/// Команда: подтвердить итог и применить.
///
/// Кнопка «сохранить» на итоге ведёт сюда. Отдельная команда, а не
/// `onboarding_apply`, потому что применение требует законченного
/// знакомства, а закончить его должен человек — кнопкой.
///
/// Замечено 15.10.2026: кнопка не работала вовсе, знакомство нельзя
/// было закончить, и настройки не сохранялись. Из-за этого у
/// секретаря не было доступа ни к одной папке, хотя в знакомстве
/// они были выбраны.
#[tauri::command]
pub fn onboarding_confirm(lang: Option<String>) -> Result<Preview, String> {
    let _ = lang;
    let mut guard = state().lock().map_err(|e| format!("состояние занято: {e}"))?;
    let s = guard.as_mut().ok_or("знакомство не начато")?;
    s.подтвердить_итог()?;
    let settings = s.to_settings();
    onboarding::save(&settings)?;
    Ok(Preview {
        summary: s.summary(),
        settings: view(&settings),
    })
}

/// Применение разрешений. Отказ, пока знакомство не закончено.
pub fn apply(s: &State) -> Result<Preview, String> {
    check_applicable(s)?;
    let settings = s.to_settings();
    onboarding::save(&settings)?;
    Ok(Preview { summary: s.summary(), settings: view(&settings) })
}

// --- команды -------------------------------------------------------------

/// Начать знакомство с нуля.
#[tauri::command]
pub fn onboarding_start(lang: Option<String>) -> Result<StepView, String> {
    let en = lang.as_deref() == Some("en");
    let tier = crate::profile::detect_and_persist().effective();
    let s = State::start();
    let v = view_of_lang(&s, tier, en);
    *state().lock().map_err(|e| format!("состояние занято: {e}"))? = Some(s);
    Ok(v)
}

/// Текущий шаг, без сброса. Нужен при перерисовке окна.
#[tauri::command]
pub fn onboarding_current(lang: Option<String>) -> Result<StepView, String> {
    let en = lang.as_deref() == Some("en");
    let tier = crate::profile::detect_and_persist().effective();
    let mut guard = state().lock().map_err(|e| format!("состояние занято: {e}"))?;
    let s = guard.get_or_insert_with(State::start);
    Ok(view_of_lang(s, tier, en))
}

/// Ответ агента на текущем шаге.
///
/// `understood: false` означает, что разобрать ответ не удалось и
/// вопрос надо повторить. Это не ошибка: модель ответила словами, и
/// угадывать разрешения опаснее, чем спросить ещё раз.
#[derive(Debug, Clone, Serialize)]
pub struct SayResult {
    pub understood: bool,
    /// Что агент говорит дальше: следующий вопрос или итог.
    pub next: StepView,
    pub summary: String,
    pub finished: bool,
}

/// Выбор инструментов кнопками.
///
/// Отдельная команда, а не разбор текста: человек не должен писать
/// JSON, чтобы отметить галочки. Текстовый путь остаётся для тех,
/// кто сформулирует ответ словами, но кнопки — основной способ.
#[tauri::command]
pub fn onboarding_set_tools(
    enable: Vec<String>,
    disable: Vec<String>,
    lang: Option<String>,
) -> Result<SayResult, String> {
    let en = lang.as_deref() == Some("en");
    let tier = crate::profile::detect_and_persist().effective();
    let mut guard = state().lock().map_err(|e| format!("состояние занято: {e}"))?;
    let s = guard.get_or_insert_with(State::start);
    s.set_tools(enable, disable);
    Ok(SayResult {
        understood: true,
        next: view_of_lang(s, tier, en),
        summary: s.summary(),
        finished: s.finished,
    })
}

/// Папки, которые имеет смысл предложить.
///
/// Только имена и пути. Содержимое не читается и не показывается:
/// на шаге разрешения список файлов выглядел бы как уже полученный
/// доступ.
#[derive(Debug, Clone, Serialize)]
pub struct FolderView {
    pub label: String,
    pub path: String,
}

#[tauri::command]
pub fn onboarding_folders() -> Vec<FolderView> {
    onboarding::folder_suggestions()
        .into_iter()
        .map(|(label, path)| FolderView {
            label,
            path: path.display().to_string(),
        })
        .collect()
}

/// Выбор папок кнопками. Названия, а не пути: именно названия видит
/// человек, а пути он мог и не знать.
#[tauri::command]
pub fn onboarding_set_paths(paths: Vec<String>, lang: Option<String>) -> Result<SayResult, String> {
    let en = lang.as_deref() == Some("en");
    let tier = crate::profile::detect_and_persist().effective();
    let mut guard = state().lock().map_err(|e| format!("состояние занято: {e}"))?;
    let s = guard.get_or_insert_with(State::start);
    s.set_paths(paths);
    Ok(SayResult {
        understood: true,
        next: view_of_lang(s, tier, en),
        summary: s.summary(),
        finished: s.finished,
    })
}

/// Выбор: весь диск или отдельные папки.
///
/// Отдельная команда, а не значение в списке папок. Разрешение на
/// весь диск — широкое, и человек должен видеть его как отдельное
/// решение, а не как «выбрана папка C:\».
#[tauri::command]
pub fn onboarding_set_whole_disk(enabled: bool, lang: Option<String>) -> Result<SayResult, String> {
    let en = lang.as_deref() == Some("en");
    let tier = crate::profile::detect_and_persist().effective();
    let mut guard = state().lock().map_err(|e| format!("состояние занято: {e}"))?;
    let s = guard.get_or_insert_with(State::start);
    s.set_whole_disk(enabled);
    Ok(SayResult {
        understood: true,
        next: view_of_lang(s, tier, en),
        summary: s.summary(),
        finished: s.finished,
    })
}

/// Режим выполнения и уровень доступа кнопками.
///
/// Отдельная команда, а не часть произвольного текста: на шаге
/// «Уровни доступа» не было ни одной кнопки, и человек оказывался
/// перед пустым экраном с одним полем ввода. Спросить словами можно,
/// но по кнопке ответ не перепутать, а «спросить у модели, чего он
/// хочет» — значит отдать выбор модели вместо человека.
#[tauri::command]
pub fn onboarding_set_modes(
    executor: Option<String>,
    approval: Option<String>,
    lang: Option<String>,
) -> Result<SayResult, String> {
    let исп = match executor.as_deref() {
        Some(v) => Some(
            serde_json::from_value::<Executor>(serde_json::Value::String(v.to_string()))
                .map_err(|e| format!("не понял режим выполнения «{v}»: {e}"))?,
        ),
        None => None,
    };
    let ур = match approval.as_deref() {
        Some(v) => Some(
            serde_json::from_value::<ApprovalMode>(serde_json::Value::String(v.to_string()))
                .map_err(|e| format!("не понял уровень доступа «{v}»: {e}"))?,
        ),
        None => None,
    };

    let en = lang.as_deref() == Some("en");
    let tier = crate::profile::detect_and_persist().effective();
    let mut guard = state().lock().map_err(|e| format!("состояние занято: {e}"))?;
    let s = guard.get_or_insert_with(State::start);
    s.set_modes(исп, ур);
    Ok(SayResult {
        understood: true,
        next: view_of_lang(s, tier, en),
        summary: s.summary(),
        finished: s.finished,
    })
}

/// Что получится, если применить. Ничего не применяет.
#[derive(Debug, Clone, Serialize)]
pub struct Preview {
    pub summary: String,
    pub settings: crate::permissions_commands::SettingsView,
}

#[tauri::command]
pub fn onboarding_say(text: String, lang: Option<String>) -> Result<SayResult, String> {
    let en = lang.as_deref() == Some("en");
    let tier = crate::profile::detect_and_persist().effective();
    let mut guard = state().lock().map_err(|e| format!("состояние занято: {e}"))?;
    let s = guard.get_or_insert_with(State::start);
    Ok(say_to(s, tier, &text))
}

#[tauri::command]
pub fn onboarding_preview(lang: Option<String>) -> Result<Preview, String> {
    let guard = state().lock().map_err(|e| format!("состояние занято: {e}"))?;
    let s = guard.as_ref().ok_or("знакомство не начато")?;
    Ok(preview_of(s))
}

/// Применить. Единственный вызов, меняющий систему, и он требует
/// отдельного нажатия после того, как пользователь увидел итог.
#[tauri::command]
pub fn onboarding_apply(lang: Option<String>) -> Result<Preview, String> {
    let guard = state().lock().map_err(|e| format!("состояние занято: {e}"))?;
    let s = guard.as_ref().ok_or("знакомство не начато")?;
    apply(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn у_каждого_шага_есть_название_и_смысл() {
        for s in Step::ALL {
            assert!(!s.title().is_empty());
            assert!(!describe(s).is_empty(), "{} без описания", s.as_str());
        }
    }

    #[test]
    fn знакомство_начинается_с_знакомства() {
        let v = view_of(&State::start(), crate::profile::Tier::T3);
        assert_eq!(v.step, "introduction");
        assert_eq!(v.index, 1);
        assert_eq!(v.total, 5);
        assert!(v.prompt.contains("локальный"), "в промпте должно быть про локальность");
    }

    /// Промпт обязан содержать границы возможностей из реестра.
    /// Иначе модель будет обещать то, чего нет.
    #[test]
    fn промпт_содержит_реальные_границы() {
        let s = State::start();
        let v = view_of(&s, crate::profile::Tier::T3);
        assert!(v.prompt.contains("chat.local"), "{}", v.prompt);
        assert!(
            v.prompt.contains("Не обещай"),
            "в промпте должно быть запрет обещать лишнего"
        );
    }

    /// Шаг «папки» не должен предлагать содержимое.
    #[test]
    fn шаг_папок_предлагает_только_папки() {
        let mut s = State::start();
        s.step = Step::Folders;
        let prompt = s.step.prompt(crate::profile::Tier::T3);
        assert!(prompt.contains("Загрузки") || prompt.contains("папок по умолчанию"));
        assert!(!prompt.contains(".txt"), "в промпте не должно быть имён файлов");
    }

    /// На представлении ничего не применяется, поэтому любой ответ
    /// считается принятым: неразборчивым может оказаться только шаг,
    /// который меняет настройки.
    #[test]
    fn неразобранный_ответ_повторяет_вопрос() {
        let mut s = State::start();
        s.step = Step::Tools;
        let r = say_to(&mut s, crate::profile::Tier::T3, "Ну давай что-нибудь включим");
        assert!(!r.understood);
        assert_eq!(r.next.step, "tools", "шаг не должен сдвинуться");
    }

    #[test]
    fn полное_знакомство_доходит_до_итога() {
        let mut s = State::start();
        let tier = crate::profile::Tier::T3;

        assert!(say_to(&mut s, tier, "Зовут Артём").understood);
        assert_eq!(s.step, Step::Tools);

        assert!(say_to(&mut s, tier, r#"{"enable":["chat.local"]}"#).understood);
        assert_eq!(s.step, Step::Folders);

        assert!(say_to(&mut s, tier, r#"{"paths":["Загрузки"]}"#).understood);
        assert_eq!(s.step, Step::Levels);

        // После уровней наступает итог, но знакомство ещё не закончено:
        // пользователь должен увидеть собранное и подтвердить.
        let r = say_to(
            &mut s,
            tier,
            r#"{"executor":"dry-run","approval_mode":"actions"}"#,
        );
        assert!(r.understood);
        assert_eq!(s.step, Step::Summary);
        assert!(!r.finished, "итог ещё предстоит подтвердить");

        let done = say_to(&mut s, tier, "всё верно, закончить");
        assert!(done.finished, "после подтверждения итога знакомство закончено");
        assert!(done.summary.contains("Понял"), "{}", done.summary);
    }

    /// Кнопка «сохранить» на итоге обязана доходить до конца.
///
/// Тест на то, чего не было: раньше кнопка звала применение, а
/// применение требовало законченного знакомства, и знакомство
/// заканчивалось только ответом текстом. Кнопка не сработать не
/// могла, и настройки не сохранялись.
#[test]
    fn кнопка_сохранить_заканчивает_знакомство() {
        let mut s = State::start();
        let tier = crate::profile::Tier::T3;

        // Пять шагов: знакомство → инструменты → папки → уровни →
        // итог. Пропуск шага не «ускоряет» знакомство, а оставляет
        // его на предыдущем шаге.
        assert!(say_to(&mut s, tier, "Зовут Артём").understood);
        assert_eq!(s.step, Step::Tools);
        assert!(say_to(&mut s, tier, r#"{"enable":["chat.local"]}"#).understood);
        assert_eq!(s.step, Step::Folders);
        assert!(say_to(&mut s, tier, r#"{"paths":["Загрузки"]}"#).understood);
        assert_eq!(s.step, Step::Levels);
        assert!(say_to(
            &mut s,
            tier,
            r#"{"executor":"dry-run","approval_mode":"actions"}"#
        )
        .understood);

        assert_eq!(s.step, Step::Summary);
        assert!(!s.finished, "до подтверждения знакомство не закончено");

        // Вот ради чего тест: раньше здесь был тупик.
        s.подтвердить_итог().expect("итог подтверждается");
        assert!(s.finished);

        let settings = s.to_settings_из(crate::permissions::Settings::default());
        assert!(
            !settings.paths.is_empty(),
            "после сохранения папки обязаны попасть в настройки, а их нет"
        );
    }

    /// Подтвердить итог можно только на итоге.
    ///
    /// Иначе кнопка на любом шаге тихо завершила бы знакомство, и
    /// человек получил бы пустые разрешения, ничего об этом не
    /// зная.
    #[test]
    fn подтвердить_можно_только_на_итоге() {
        let mut s = State::start();
        let e = s.подтвердить_итог().unwrap_err();
        assert!(e.contains("итоге"), "{e}");
        assert!(!s.finished);
    }

    /// «Начать заново» действительно стирает собранное.
    ///
    /// Проверка идёт через саму команду, а не через `State::start()`:
    /// между ними есть присваивание в общее состояние, и именно оно
    /// может не сработать. Проверено человеком 15.10.2026 — после
    /// «начать заново» первый шаг возвращался, а собранное оставалось
    /// прежним.
    #[test]
    fn начать_заново_сбрасывает_собранное() {
        // Папка берётся временная: она есть на любой машине. «Desktop»
        // не годится — `resolve_folder` ищет его по подписям, а на
        // машине без такой подписи путь не находится и папка молча
        // отбрасывается. Проверено: с «Desktop» итог получался
        // «папки не открыты», и проверка проходила бы враньё.
        let папка = std::env::temp_dir().join("arkha-сброс-знакомства");
        std::fs::create_dir_all(&папка).expect("временная папка создана");
        let имя_папки = папка.file_name().unwrap().to_string_lossy().to_string();
        let путь = папка.to_string_lossy().to_string();

        let mut guard = state().lock().expect("состояние занято");
        let mut s = State::start();
        say_to(&mut s, crate::profile::Tier::T3, "Зовут Артём");
        say_to(&mut s, crate::profile::Tier::T3, r#"{"enable":["chat.local"]}"#);
        // Ответ строится сериализатором, а не склейкой строк: в пути
        // Windows есть обратные слэши, а `\U` в JSON — не escape.
        // Склейка молча даёт нечитаемый ответ, шаг не сдвигается,
        // и проверка падает не там, где причина.
        let с_папками = serde_json::json!({ "paths": [путь] }).to_string();
        say_to(&mut s, crate::profile::Tier::T3, &с_папками);
        say_to(
            &mut s,
            crate::profile::Tier::T3,
            r#"{"executor":"live","approval_mode":"actions"}"#,
        );
        assert_eq!(s.step, Step::Summary);
        let до = s.summary();
        assert!(
            до.contains(&имя_папки),
            "до сброса папка обязана быть в итоге, а итог: {до}"
        );
        assert!(до.contains("Выполнять"), "до сброса режим: {до}");
        *guard = Some(s);
        drop(guard);

        // Теперь команда, которой пользуется кнопка.
        let начало = onboarding_start(None).expect("сброс не должен падать");
        assert_eq!(начало.step, "introduction");
        assert_eq!(начало.index, 1);

        let после = onboarding_current(None).expect("текущий шаг");
        assert_eq!(после.step, "introduction");
        let summary = state().lock().expect("состояние занято").as_ref().unwrap().summary();
        assert!(
            !summary.contains(&имя_папки),
            "после сброса папка не должна упоминаться, а итог: {summary}"
        );
        assert!(
            summary.contains("инструменты выбраны не будут"),
            "после сброса инструменты должны быть пусты, а итог: {summary}"
        );
        assert!(
            summary.contains("папки не открыты"),
            "после сброса папки должны быть закрыты, а итог: {summary}"
        );
        // Режим выполнения в итоге остаётся: он приходит из
        // сохранённых настроек, а не из собранного. Чистое знакомство
        // показывает, что действует сейчас, — иначе человек увидел бы
        // «выполнять всё» вместо собственного выбора и не понял бы,
        // что настройки не сбросились, а просто не выбраны заново.
        assert!(
            summary.contains("подтверждение:"),
            "итог должен называть режим подтверждения: {summary}"
        );

        let _ = std::fs::remove_dir_all(&папка);
    }

    /// Применить раньше времени нельзя: конфигурацию разрешений
    /// невозможно откатить задним числом.
    #[test]
    fn применить_до_конца_нельзя() {
        let s = State::start();
        let err = apply(&s).expect_err("должно быть отказано");
        assert!(err.contains("не закончено"), "{err}");
    }

    /// Предпросмотр не должен пугать пользователя, пока он в dry-run.
    ///
    /// База задана явно: раньше предпросмотр смотрел в настоящий
    /// файл человека и требовал увидеть там `dry-run`, то есть
    /// требовал, чтобы мы запрещали выбор «выполнять».
    #[test]
    fn предпросмотр_ничего_не_меняет() {
        let s = State::start();
        let p = preview_из(&s, crate::permissions::Settings::default());
        assert_eq!(p.settings.executor.value, "dry-run");
        assert!(p.settings.warning.is_none());
    }
}
