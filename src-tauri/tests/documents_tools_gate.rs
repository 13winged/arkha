//! Гейт инструментов документов: запись, чтение, отказы.
//!
//! Юнит-тесты проверяют маршрутизацию и разбор, но не отвечают на
//! главный вопрос: **создаст ли `docs.write` файл, который `docs.read`
//! прочитает обратно**. Здесь это делается по-настоящему, на живых CLI.
//!
//! Запуск:
//! ```text
//! ARKHA_DOCS_CLI=<wordcraft-cli.exe> \
//! ARKHA_DOCS_GRID=<gridcraft-cli.exe> \
//! ARKHA_DOCS_DECK=<deckcraft-cli.exe> \
//!   cargo test --test documents_tools_gate -- --test-threads=1 --nocapture
//! ```
//!
//! Таблицы и презентации проверяются, только если заданы их CLI:
//! без них эти тесты пропускаются, а не притворяются зелёными.

use std::path::{Path, PathBuf};

use arkha_lib::documents::install::{self, App};
use arkha_lib::documents::mcp::Mcp;
use arkha_lib::documents::tools::{DocsInfo, DocsRead, DocsWrite};
use arkha_lib::permissions::{ApprovalMode, CapabilityRule, Executor, Settings};
use arkha_lib::tools::{Tool, ToolError};
use serde_json::json;

fn sandbox(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "arkha-docs-tools-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("время")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("песочница");
    dir
}

/// Переменная с путём к CLI. Пропуск — только осознанный.
fn cli_from_env(name: &str) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(name).map(PathBuf::from) {
        assert!(path.is_file(), "нет файла {}", path.display());
        return Some(path);
    }
    let message = format!(
        "нет {name}: укажите путь к CLI или ARKHA_ALLOW_SKIP=1, чтобы пропустить"
    );
    if std::env::var("ARKHA_ALLOW_SKIP").is_ok() {
        eprintln!("ПРОПУЩЕНО по требованию: {message}");
        return None;
    }
    panic!("{message}");
}

/// Положить CLI туда, откуда его берёт `install::ensure`, и вернуть
/// настройки с песочницей в разрешённых папках.
///
/// Копия делается, потому что `ensure` ищет закреплённую версию в
/// нашем дереве: подсовывать ей путь через отдельную переменную
/// значило бы проверять не тот код, который работает у человека.
fn prepare(app: App, source: &Path, label: &str) -> (PathBuf, Settings) {
    let root = sandbox(label);
    let pin = install::pinned(app);
    let folder = root.join(app.title()).join(pin.version);
    std::fs::create_dir_all(&folder).expect("папка версии");
    std::fs::copy(source, folder.join(app.exe_name())).expect("копия CLI");

    // `ensure` читает корень из окружения. Гейты идут с
    // `--test-threads=1`; другого способа не трогать настоящий профиль
    // у `install` нет.
    unsafe {
        std::env::set_var("ARKHA_DOCS_CLI_ROOT", &root);
    }

    let mut settings = Settings::default();
    settings.paths = vec![root.clone()];
    settings.executor = Executor::Live;
    settings.capabilities.insert(
        "files.write".into(),
        CapabilityRule { enabled: true, mode: Some(ApprovalMode::Auto) },
    );
    (root, settings)
}

fn text_of(error: ToolError) -> String {
    error.to_string()
}

#[test]
fn гейт_документов_запись_чтение_и_отказы() {
    let Some(cli) = cli_from_env("ARKHA_DOCS_CLI") else {
        return;
    };
    let (root, settings) = prepare(App::Wordcraft, &cli, "wordcraft");

    // 1. Текстовый файл читается тем же путём, что и docx.
    let plain = root.join("заметка.txt");
    std::fs::write(&plain, "Первая строка\nВторая строка").unwrap();
    let read = DocsRead.call(&json!({ "path": plain }), &settings).expect("чтение txt");
    assert!(read.text.contains("Первая строка"), "{}", read.text);
    assert!(read.text.contains("Вторая строка"), "{}", read.text);

    // 2. Создание docx и чтение его обратно.
    let docx = root.join("письмо.docx");
    let body = "Первый абзац про документы.\nВторой абзац: цифры 42 и слово «тест».";
    let written = DocsWrite
        .call(&json!({ "path": docx, "text": body }), &settings)
        .expect("создание docx");
    assert!(written.changed);
    assert!(docx.is_file(), "файл не появился");

    let back = DocsRead.call(&json!({ "path": docx }), &settings).expect("чтение docx");
    println!("прочитано обратно: {}", back.text);
    assert!(back.text.contains("Первый абзац"), "{}", back.text);
    assert!(back.text.contains("42"), "{}", back.text);
    assert!(back.text.contains("тест"), "{}", back.text);

    // 3. Сведения о документе отвечают и содержат страницы.
    let info = DocsInfo.call(&json!({ "path": docx }), &settings).expect("сведения");
    assert!(info.text.contains("pages"), "{}", info.text);

    // 4. Поверх существующего файла не пишем.
    let overwrite = DocsWrite.call(&json!({ "path": docx, "text": "другое" }), &settings);
    let error = text_of(overwrite.expect_err("перезапись должна быть отклонена"));
    assert!(error.contains("уже существует"), "{error}");

    // 5. Путь вне разрешённых папок отклоняется до запуска CLI.
    let outside = std::env::temp_dir().join("arkha-docs-tools-outside.txt");
    std::fs::write(&outside, "снаружи").unwrap();
    let denied = DocsRead.call(&json!({ "path": outside }), &settings);
    let error = text_of(denied.expect_err("путь снаружи должен быть отклонён"));
    assert!(error.contains("вне разрешённых"), "{error}");

    let _ = std::fs::remove_file(outside);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn гейт_документов_таблица_читается() {
    let Some(cli) = cli_from_env("ARKHA_DOCS_GRID") else {
        return;
    };
    let (root, settings) = prepare(App::Gridcraft, &cli, "gridcraft");

    // Книгу создаём чужим CLI напрямую: наш `docs.write` таблицы пока
    // не пишет, и делать вид, что проверяем запись, нельзя.
    let book = root.join("продажи.xlsx");
    let exe = root
        .join(App::Gridcraft.title())
        .join(install::pinned(App::Gridcraft).version)
        .join(App::Gridcraft.exe_name());
    let mut mcp = Mcp::connect(&exe, &["mcp".to_string()]).expect("gridcraft mcp");
    for (cell, input) in [
        ("A1", json!("Регион")),
        ("B1", json!("Продажи")),
        ("A2", json!("Москва")),
        ("B2", json!(1234)),
        ("A3", json!("Казань")),
        ("B3", json!(987)),
    ] {
        let reply = mcp.call("set_cell", json!({ "cell": cell, "input": input })).expect("set_cell");
        assert!(!reply.is_error, "set_cell {cell}: {}", reply.text);
    }
    let saved = mcp
        .call("save_workbook", json!({ "path": book.display().to_string() }))
        .expect("save_workbook");
    assert!(!saved.is_error, "{}", saved.text);
    mcp.stop();

    let read = DocsRead.call(&json!({ "path": book }), &settings).expect("чтение xlsx");
    println!("таблица: {}", read.text);
    assert!(read.text.contains("Москва"), "{}", read.text);
    assert!(read.text.contains("1234"), "{}", read.text);
    assert!(read.text.contains("Казань"), "{}", read.text);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn гейт_документов_презентация_читается() {
    let Some(cli) = cli_from_env("ARKHA_DOCS_DECK") else {
        return;
    };
    let (root, settings) = prepare(App::Deckcraft, &cli, "deckcraft");

    let deck = root.join("итоги.pptx");
    let exe = root
        .join(App::Deckcraft.title())
        .join(install::pinned(App::Deckcraft).version)
        .join(App::Deckcraft.exe_name());
    let args = vec![
        "mcp".to_string(),
        "--automation-write-root".to_string(),
        root.display().to_string(),
    ];
    let mut mcp = Mcp::connect(&exe, &args).expect("deckcraft mcp");
    assert!(!mcp.call("new_presentation", json!({})).expect("new").is_error);
    let added = mcp
        .call(
            "add_slide",
            json!({ "layout": "titleOnly", "title": "Итоги квартала", "body": "Выручка выросла на 12 процентов" }),
        )
        .expect("add_slide");
    assert!(!added.is_error, "{}", added.text);
    let saved = mcp
        .call("save_presentation", json!({ "path": deck.display().to_string() }))
        .expect("save_presentation");
    assert!(!saved.is_error, "{}", saved.text);
    mcp.stop();

    let read = DocsRead.call(&json!({ "path": deck }), &settings).expect("чтение pptx");
    println!("презентация: {}", read.text);
    assert!(read.text.contains("Итоги квартала"), "{}", read.text);

    let _ = std::fs::remove_dir_all(root);
}
