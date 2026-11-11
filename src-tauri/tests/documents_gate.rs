//! Гейт документов: клиент MCP против живого CLI.
//!
//! Юнит-тесты клиента работают с подставным транспортом. Это правильно
//! для проверки протокола, но ничего не говорит о главном: **сойдётся
//! ли наше рукопожатие с настоящим сервером на закреплённой версии**.
//!
//! Гейт отвечает на три вопроса:
//!
//! 1. Рукопожатие проходит и совпадает версия протокола.
//! 2. `tools/list` отдаёт инструменты, на которые мы рассчитываем.
//! 3. Путь вне разрешённой папки отклоняется самим CLI.
//!
//! Третий пропускается для GridCraft: у него нет флага ограничения
//! папок, и ограничение держит наша сторона — это записано в docs/31.
//!
//! Запуск:
//! ```text
//! ARKHA_DOCS_CLI=<путь к wordcraft-cli.exe> \
//!   cargo test --test documents_gate -- --nocapture --test-threads=1
//! ```

use std::path::{Path, PathBuf};
use std::time::Duration;

use arkha_lib::documents::mcp::{Mcp, PROTOCOL};

/// Какой CLI проверяем. Имя файла — единственное, что известно
/// заранее: аргументы и набор инструментов у четырёх разные.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Wordcraft,
    Pdfcraft,
    Gridcraft,
    Deckcraft,
}

impl Kind {
    fn from_path(path: &Path) -> Option<Self> {
        let name = path.file_name()?.to_string_lossy().to_lowercase();
        if name.contains("wordcraft") {
            Some(Self::Wordcraft)
        } else if name.contains("pdfcraft") {
            Some(Self::Pdfcraft)
        } else if name.contains("gridcraft") {
            Some(Self::Gridcraft)
        } else if name.contains("deckcraft") {
            Some(Self::Deckcraft)
        } else {
            None
        }
    }

    /// Инструмент, без которого интеграция не имеет смысла.
    fn expected_tool(self) -> &'static str {
        match self {
            Self::Wordcraft => "get_text",
            Self::Pdfcraft => "text_extract",
            Self::Gridcraft => "read_range",
            Self::Deckcraft => "inspect_document",
        }
    }

    /// Инструмент, которым открывается файл, и имя аргумента-пути.
    /// `None` — CLI не умеет ограничивать папки, проверять нечего.
    fn open_call(self) -> Option<(&'static str, &'static str)> {
        match self {
            Self::Wordcraft => Some(("open_document", "path")),
            Self::Pdfcraft => Some(("doc_open", "path")),
            Self::Gridcraft => None,
            Self::Deckcraft => Some(("open_presentation", "path")),
        }
    }

    fn args(self, root: &Path) -> Vec<String> {
        let root = root.display().to_string();
        match self {
            // У PdfCraft ограничение чтения задаётся отдельным флагом:
            // `--root` разрешает и запись тоже.
            Self::Pdfcraft => vec!["mcp".into(), "--automation-read-root".into(), root],
            Self::Gridcraft => vec!["mcp".into()],
            _ => vec!["mcp".into(), "--automation-read-root".into(), root],
        }
    }
}

/// Путь к CLI. Пропуск гейта — это не успех: без переменной тест
/// проваливается, а `ARKHA_ALLOW_SKIP=1` пропускает осознанно.
fn cli() -> Option<(PathBuf, Kind)> {
    let path = std::env::var_os("ARKHA_DOCS_CLI").map(PathBuf::from);
    if let Some(path) = path {
        let kind = Kind::from_path(&path).unwrap_or_else(|| {
            panic!(
                "ARKHA_DOCS_CLI={} не похож ни на один из четырёх CLI",
                path.display()
            )
        });
        assert!(path.is_file(), "нет файла {}", path.display());
        return Some((path, kind));
    }

    let message = "нет ARKHA_DOCS_CLI: укажите путь к wordcraft-cli.exe, \
                   pdfcraft-cli.exe, gridcraft-cli.exe или deckcraft-cli.exe, \
                   либо поставьте ARKHA_ALLOW_SKIP=1, чтобы сознательно пропустить";
    if std::env::var("ARKHA_ALLOW_SKIP").is_ok() {
        eprintln!("ПРОПУЩЕНО по требованию: {message}");
        return None;
    }
    panic!("{message}");
}

fn sandbox(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "arkha-docs-gate-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("время")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("песочница");
    dir
}

#[test]
fn гейт_документов_рукопожатие_и_список_инструментов() {
    let Some((path, kind)) = cli() else {
        return;
    };
    let root = sandbox("list");
    let args = kind.args(&root);

    let mcp = Mcp::connect(&path, &args).unwrap_or_else(|error| {
        panic!("рукопожатие с {} не прошло: {error}", path.display())
    });

    let tools = mcp.tools();
    println!(
        "{}: {} инструментов: {}",
        path.display(),
        tools.len(),
        tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>().join(", ")
    );

    assert!(!tools.is_empty(), "сервер не отдал ни одного инструмента");
    assert_eq!(PROTOCOL, "2025-06-18", "версия протокола в клиенте изменилась");
    assert!(
        mcp.has_tool(kind.expected_tool()),
        "нет инструмента {}, без него читать нечем",
        kind.expected_tool()
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn гейт_документов_чужой_путь_отклоняется_cli() {
    let Some((path, kind)) = cli() else {
        return;
    };
    let Some((tool, argument)) = kind.open_call() else {
        eprintln!("ПРОПУЩЕНО: у {kind:?} нет ограничения папок, это держит наша сторона");
        return;
    };

    let root = sandbox("escape");
    let mut mcp = Mcp::connect(&path, &kind.args(&root))
        .unwrap_or_else(|error| panic!("рукопожатие с {} не прошло: {error}", path.display()));

    // Файл существует и читается нами — значит отказ идёт от CLI, а не
    // от того, что пути нет.
    let outside = std::env::temp_dir().join("arkha-docs-gate-outside.ini");
    std::fs::write(&outside, b"[gate]\nkey=value\n").expect("файл вне песочницы");

    let reply = mcp
        .call(tool, serde_json::json!({ argument: outside }))
        .unwrap_or_else(|error| panic!("{tool} не ответил: {error}"));

    println!("{tool} на чужой путь: isError={} | {}", reply.is_error, reply.text);
    assert!(
        reply.is_error,
        "CLI открыл файл вне разрешённой папки: {}",
        reply.text
    );

    mcp.stop();
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_file(outside);
}

/// Клиент не должен оставлять процесс после остановки: иначе CLI
/// держит документы открытыми и переживает выход из приложения.
#[test]
fn гейт_документов_остановка_не_оставляет_процесс() {
    let Some((path, kind)) = cli() else {
        return;
    };
    let root = sandbox("stop");
    let mut mcp = Mcp::connect(&path, &kind.args(&root))
        .unwrap_or_else(|error| panic!("рукопожатие с {} не прошло: {error}", path.display()));

    mcp.stop();
    std::thread::sleep(Duration::from_millis(200));
    // После остановки процесс не должен отвечать: следующий вызов обязан
    // провалиться, а не вернуть результат из мёртвого пайпа.
    let after = mcp.call(kind.expected_tool(), serde_json::json!({}));
    assert!(after.is_err(), "остановленный CLI всё ещё отвечает");

    let _ = std::fs::remove_dir_all(root);
}
