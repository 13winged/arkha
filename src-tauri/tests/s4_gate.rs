// Гейт S4: знакомство на живом железе.
//
// Проверяется то, что сломается у пользователя: агент знает свои
// границы, собирает настройки из разговора и не применяет их раньше
// времени.
//
// Пропуск не считается успехом. Знакомство — первая точка остановки
// проекта, и «прошло на трёх из пяти шагов» здесь не подходит.

use arkha_lib::onboarding::{Step, State};
use arkha_lib::onboarding_commands::{
    check_applicable, onboarding_folders, preview_of, say_to, view_of,
};
use arkha_lib::profile::{detect_and_persist, Tier};

fn tier() -> Tier {
    detect_and_persist().effective()
}

/// Промпт первого шага обязан содержать границы из настоящего
/// реестра, а не выдуманные. Иначе модель будет обещать то, чего
/// в приложении нет, и это станет правдой с точки зрения модели.
#[test]
fn гейт_s4_промпт_знает_реальные_границы() {
    let s = State::start();
    let v = view_of(&s, tier());
    let prompt = &v.prompt;

    eprintln!("профиль: {}", v.prompt.lines().next().unwrap_or(""));
    assert!(prompt.contains("chat.local"), "нет разговора в списке доступного");
    assert!(prompt.contains("Недоступно"), "нет раздела недоступного");
    assert!(
        prompt.contains("Не обещай"),
        "в промпте нет запрета обещать лишнего"
    );
    assert!(prompt.contains("ничего не отправляется"), "нет обещания приватности");
}

/// На шаге папок предлагаются только папки. Имена файлов на этом
/// шаге показывать нельзя: пользователь решит, что доступ уже
/// получен, и это будет неверно.
#[test]
fn гейт_s4_шаг_папок_не_раскрывает_содержимое() {
    let mut s = State::start();
    s.step = Step::Folders;
    let prompt = s.step.prompt(tier());

    for ext in [".txt", ".docx", ".pdf", ".xlsx"] {
        assert!(
            !prompt.contains(ext),
            "в подсказках не должно быть имён файлов: найдено {ext}"
        );
    }
    eprintln!("подсказки папок корректны");
}

/// Полный проход: пять шагов, разбор ответов, применение только в
/// конце. Модель тут не участвует — проверяется логика сбора.
#[test]
fn гейт_s4_знакомство_собирает_настройки_целиком() {
    let tier = tier();
    let mut s = State::start();

    say_to(&mut s, tier, "Зовут Артём");
    assert_eq!(s.step, Step::Tools);

    say_to(&mut s, tier, r#"{"enable":["chat.local"]}"#);
    assert_eq!(s.step, Step::Folders);

    say_to(&mut s, tier, r#"{"paths":["Загрузки","Рабочий стол"]}"#);
    assert_eq!(s.step, Step::Levels);

    let r = say_to(
        &mut s,
        tier,
        r#"{"executor":"dry-run","approval_mode":"actions"}"#,
    );
    assert!(r.understood);
    assert_eq!(s.step, Step::Summary);

    // Что собрано, показывается пользователю словами.
    eprintln!("итог: {}", r.summary);
    assert!(r.summary.contains("Понял"));

    // Названия папок должны стать настоящими путями.
    let settings = s.to_settings();
    for p in &settings.paths {
        assert!(p.is_dir(), "путь не существует: {}", p.display());
        assert!(p.is_absolute(), "путь не абсолютный: {}", p.display());
    }
    eprintln!("пути: {:?}", settings.paths);

    let done = say_to(&mut s, tier, "всё верно");
    assert!(done.finished);

    // Безопасные значения сохранились: dry-run и подтверждение
    // изменений. Знакомство не имеет права расширять права молча.
    assert_eq!(settings.executor, arkha_lib::permissions::Executor::DryRun);
    assert_eq!(
        settings.approval_mode,
        arkha_lib::permissions::ApprovalMode::Risky
    );
}

/// Неразобранный ответ не должен двигать знакомство. Угадывать
/// разрешения опаснее, чем спросить ещё раз.
#[test]
fn гейт_s4_мусорный_ответ_не_двигает_шаг() {
    let tier = tier();
    let mut s = State::start();
    s.step = Step::Tools;

    let r = say_to(&mut s, tier, "ну включай всё что можно");
    assert!(!r.understood, "мусорный ответ принят как ответ");
    assert_eq!(s.step, Step::Tools, "шаг сдвинулся на мусоре");
    assert!(s.collected.enable.is_empty(), "включено что-то без ответа");
}

/// Несуществующие папки не должны попасть в разрешения. Путь, которого
/// нет, всплывёт через месяц и будет выглядеть как поломка.
#[test]
fn гейт_s4_несуществующие_пути_отбрасываются() {
    let tier = tier();
    let mut s = State::start();
    s.step = Step::Folders;

    say_to(
        &mut s,
        tier,
        r#"{"paths":["Загрузки","D:\\точно\\нет\\такой","Несуществующая"]}"#,
    );

    for p in &s.collected.paths {
        assert!(p.is_dir(), "несуществующий путь попал в разрешения: {p:?}");
    }
    eprintln!("осталось путей: {}", s.collected.paths.len());
}

/// Выбор папок кнопками доходит до разрешений настоящими путями.
#[test]
fn гейт_s4_выбор_папок_доходит_до_разрешений() {
    let mut s = State::start();
    s.step = Step::Folders;

    s.set_paths(vec!["Загрузки".into(), "D:\\нет\\такой".into()]);
    assert_eq!(s.step, Step::Levels, "шаг должен сдвинуться");

    let settings = s.to_settings();
    assert!(!settings.paths.is_empty(), "ни одной папки не записалось");
    for p in &settings.paths {
        assert!(p.is_dir(), "{p:?} не существует");
        assert!(p.is_absolute(), "{p:?} не абсолютный");
    }
    eprintln!("выбрано папок: {}", settings.paths.len());
}

/// Весь диск — решение человека, и оно доходит до разрешений
/// отдельным флагом, а не подменой корня диска в списке папок.
#[test]
fn гейт_s4_весь_диск_доходит_до_разрешений_отдельным_флагом() {
    let mut s = State::start();
    s.step = Step::Folders;

    s.set_whole_disk(true);
    assert_eq!(s.step, Step::Levels, "включение должно закончить шаг");

    let settings = s.to_settings();
    assert!(settings.читать_весь_диск, "флаг не дошёл до разрешений");
    assert!(
        settings.paths.is_empty(),
        "папки не должны подменяться корнем диска: {:?}",
        settings.paths
    );
}

/// Выключение — отказ от варианта, а не ответ. Шаг после него не
/// двигается: человек снял галочку, чтобы поставить другую.
#[test]
fn гейт_s4_снятие_всего_диска_не_двигает_шаг() {
    let mut s = State::start();
    s.step = Step::Folders;

    s.set_whole_disk(false);
    assert_eq!(s.step, Step::Folders, "снятие галочки не должно двигать шаг");
    assert!(!s.to_settings().читать_весь_диск);
}

/// Папок на диске может не оказаться. Тогда выбор пустой и шаг всё
/// равно должен доехать до конца, а не зависнуть.
#[test]
fn гейт_s4_пустой_выбор_папок_не_ломает_знакомство() {
    let mut s = State::start();
    s.step = Step::Folders;
    s.set_paths(vec![]);
    assert_eq!(s.step, Step::Levels);
    assert!(s.to_settings().paths.is_empty());
}

/// Подсказки содержат только папки: ни одного имени файла.
#[test]
fn гейт_s4_подсказки_папок_не_содержат_файлов() {
    for f in onboarding_folders() {
        assert!(!f.label.is_empty());
        assert!(!f.path.is_empty());
        assert!(
            std::path::Path::new(&f.path).is_dir(),
            "подсказка указывает на несуществующее: {}",
            f.path
        );
    }
}

/// Предпросмотр не должен ничего применять и не должен пугать,
/// пока режим безопасный.
#[test]
fn гейт_s4_предпросмотр_безопасен() {
    let mut s = State::start();
    s.step = Step::Summary;
    s.collected.enable = vec!["chat.local".into()];

    let p = preview_of(&s);
    assert_eq!(p.settings.executor.value, "dry-run");
    assert!(p.settings.warning.is_none(), "в безопасном режиме пугать нечем");
    assert!(
        p.settings.deny_apps.contains(&"local.secretary.app".to_string()),
        "секретарь обязан остаться в запрете после знакомства"
    );
}

/// Применение до конца знакомства отклоняется: конфигурацию
/// разрешений невозможно откатить задним числом.
///
/// Проверяется `check_applicable`, а не `apply`: применение пишет в
/// файл настроек, и гейт не должен трогать диск пользователя.
#[test]
fn гейт_s4_применение_до_итога_отклоняется() {
    let mut s = State::start();
    let err = check_applicable(&s).expect_err("знакомство не начато");
    assert!(err.contains("не закончено"), "{err}");

    // Доходим до конца — теперь применять можно.
    let tier = tier();
    say_to(&mut s, tier, "Зовут Артём");
    say_to(&mut s, tier, r#"{"enable":["chat.local"]}"#);
    say_to(&mut s, tier, r#"{"paths":[]}"#);
    say_to(&mut s, tier, r#"{"executor":"dry-run","approval_mode":"actions"}"#);
    say_to(&mut s, tier, "всё верно");
    assert!(s.finished);
    assert!(check_applicable(&s).is_ok());
}

/// Выбор инструментов кнопками доходит до настроек.
///
/// Проверяется главное: включённое записывается, а не теряется по
/// дороге. Раньше `to_settings` писал только запреты, и отметка
/// пользователя ничего бы не меняла.
#[test]
fn гейт_s4_выбор_кнопками_доходит_до_настроек() {
    let mut s = State::start();
    s.step = Step::Tools;

    s.set_tools(vec!["chat.local".into()], vec!["files.write".into()]);
    assert_eq!(s.step, Step::Folders,  "шаг должен сдвинуться");

    let settings = s.to_settings();
    assert_eq!(
        settings.capabilities.get("chat.local").map(|r| r.enabled),
        Some(true),
        "отмеченное потерялось"
    );
    assert_eq!(
        settings.capabilities.get("files.write").map(|r| r.enabled),
        Some(false),
        "снятое не записалось"
    );
}

/// Идентификаторы из интерфейса проверяются. Их источник — кнопки,
/// но значение извне есть значение извне.
#[test]
fn гейт_s4_выдуманные_идентификаторы_не_попадают_в_настройки() {
    let mut s = State::start();
    s.step = Step::Tools;

    s.set_tools(
        vec!["chat.local".into(), "выдуманное.нет".into()],
        vec!["тоже.нет".into()],
    );

    let settings = s.to_settings();
    assert!(!settings.capabilities.contains_key("выдуманное.нет"));
    assert!(!settings.capabilities.contains_key("тоже.нет"));
    assert!(settings.capabilities.contains_key("chat.local"));
}

/// Явно разрешённое действие должно быть разрешено, иначе кнопка
/// «включить» ничего не включает.
#[test]
fn гейт_s4_включённая_кнопка_действительно_разрешает() {
    use arkha_lib::permissions::Executor;

    let mut s = State::start();
    s.step = Step::Tools;
    s.set_tools(vec!["files.write".into()], vec![]);

    let mut settings = s.to_settings();
    settings.executor = Executor::Live;

    assert!(
        settings.allows("files.write").allowed,
        "кнопка включила, а разрешение не сработало"
    );
}

/// Без явной отметки действовать нельзя. Это то, что обещает
/// знакомство, и то, что должно быть правдой.
#[test]
fn гейт_s4_неотмеченное_действие_запрещено() {
    use arkha_lib::permissions::Executor;

    let mut s = State::start();
    s.step = Step::Tools;
    s.set_tools(vec![], vec!["files.write".into()]);

    let mut settings = s.to_settings();
    settings.executor = Executor::Live;

    assert!(!settings.allows("files.write").allowed);
    assert!(
        settings.allows("files.read").allowed,
        "чтение не должно требовать явной отметки"
    );
}
