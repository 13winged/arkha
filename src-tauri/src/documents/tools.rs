// Инструменты документов для модели.
//
// Три, а не 18 и не 200. Модель видит `docs.read`, `docs.info` и
// `docs.write`; выбор CLI по расширению — наша работа, а не её.
// Спека и числа: docs/31-документы-через-mcp.md.
//
// Почему процесс поднимается на один вызов, а не живёт всё время.
// Первая версия выбирает простоту: нет общего состояния между
// профилями, нет осиротевшего процесса после выхода, нет вопроса
// «к какому профилю относится этот CLI». Цена — рукопожатие на каждый
// вызов: 10 мс у WordCraft, до 250 мс у DeckCraft. Для чтения файла,
// где разбор занимает больше, это не та величина, ради которой стоит
// держать чужой процесс.

use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use super::install::{self, App};
use super::mcp::{Mcp, ToolReply};
use crate::permissions::Settings;
use crate::tools::fs::{arg_str, resolve_inside};
use crate::tools::{ensure_allowed, Tool, ToolError, ToolOutput, ToolResult, ToolSpec};

/// Сколько байт текста отдаём модели за один раз. Как у `fs.read`:
/// примерно двадцать страниц. Дальше — начало и честная пометка.
const MAX_TEXT: usize = 64 * 1024;

const WORD_EXTENSIONS: &[&str] = &["docx", "doc", "odt", "rtf", "md", "txt", "tex", "html", "htm", "json"];
const PDF_EXTENSIONS: &[&str] = &["pdf"];
const GRID_EXTENSIONS: &[&str] = &["xlsx", "xlsm", "csv", "tsv"];
const DECK_EXTENSIONS: &[&str] = &["pptx", "deckcraft"];

/// Форматы, в которые умеем писать. Всё это делает WordCraft: у него
/// `save_document` выбирает формат по расширению.
const WRITE_EXTENSIONS: &[&str] = &["docx", "pdf", "odt", "rtf", "html", "htm", "md", "txt", "tex"];

fn extension(path: &Path) -> String {
    path.extension()
        .map(|value| value.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

/// Какой CLI читает этот файл. Ошибка называет, что поддерживается:
/// иначе модель начнёт перебирать расширения по одному.
fn app_for(path: &Path) -> Result<App, ToolError> {
    let ext = extension(path);
    if WORD_EXTENSIONS.contains(&ext.as_str()) {
        Ok(App::Wordcraft)
    } else if PDF_EXTENSIONS.contains(&ext.as_str()) {
        Ok(App::Pdfcraft)
    } else if GRID_EXTENSIONS.contains(&ext.as_str()) {
        Ok(App::Gridcraft)
    } else if DECK_EXTENSIONS.contains(&ext.as_str()) {
        Ok(App::Deckcraft)
    } else {
        Err(ToolError::Unreadable(format!(
            "формат «.{ext}» не читается. Поддерживаются: docx, doc, odt, rtf, md, txt, tex, \
             html, pdf, xlsx, xlsm, csv, tsv, pptx"
        )))
    }
}

/// Готовый к запуску CLI. Скачивать здесь нельзя: загрузка 120 МБ
/// внутри вызова инструмента выглядит как зависший агент, поэтому её
/// делает человек кнопкой в настройках. Отказ называет, куда идти.
fn exe_for(app: App) -> Result<PathBuf, ToolError> {
    match install::status(app) {
        Ok(install::State::Ready(path)) => Ok(path),
        Ok(install::State::Missing) => {
            let pin = install::pinned(app);
            Err(ToolError::Missing(format!(
                "{} {} не установлен. Откройте «Настройки» → «Документы» \
                 и нажмите «Скачать» ({} МБ)",
                app.title(),
                pin.version,
                pin.archive_bytes.div_ceil(1024 * 1024)
            )))
        }
        Err(error) => Err(ToolError::Refused(error)),
    }
}

/// Поднять CLI на чтение. Корень — папка самого файла, а не все
/// разрешённые папки: чем уже право, тем меньше последствий ошибки.
fn connect_read(app: App, path: &Path) -> Result<Mcp, ToolError> {
    let exe = exe_for(app)?;
    let parent = path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .display()
        .to_string();

    let args = match app {
        // У GridCraft нет ограничения папок, поэтому файл открываем мы,
        // а модели достаются только инструменты без аргумента-пути.
        App::Gridcraft => vec!["mcp".into(), "--in".into(), path.display().to_string()],
        _ => vec!["mcp".into(), "--automation-read-root".into(), parent],
    };

    Mcp::connect(&exe, &args)
        .map_err(|error| ToolError::Refused(format!("{} не запустился: {error}", app.title())))
}

fn connect_write(app: App, path: &Path) -> Result<Mcp, ToolError> {
    let exe = exe_for(app)?;
    let parent = path
        .parent()
        .ok_or_else(|| ToolError::BadArgs("у пути нет папки".into()))?
        .display()
        .to_string();
    let args = vec!["mcp".into(), "--automation-write-root".into(), parent];
    Mcp::connect(&exe, &args)
        .map_err(|error| ToolError::Refused(format!("{} не запустился: {error}", app.title())))
}

/// Ответ инструмента, разобранный как JSON. Отказ CLI — это результат
/// с текстом, а не сбой транспорта, поэтому он превращается в
/// `Refused`, а не в панику и не в пустую строку.
fn parse(reply: ToolReply, tool: &str) -> Result<Value, ToolError> {
    if reply.is_error {
        return Err(ToolError::Refused(format!("{tool}: {}", reply.text.trim())));
    }
    serde_json::from_str(reply.text.trim()).map_err(|error| {
        ToolError::Unreadable(format!(
            "{tool} вернул не JSON: {error}. Начало ответа: {}",
            first_chars(&reply.text, 200)
        ))
    })
}

/// Вызов инструмента CLI с переводом сбоя транспорта в отказ.
///
/// Ошибка клиента — это «CLI не ответил / процесс упал», а не отказ
/// инструмента. Модели всё равно нужен текст причины, поэтому оба
/// случая приходят как `Refused`, но с разными началами.
fn call(mcp: &mut Mcp, name: &str, args: Value) -> Result<ToolReply, ToolError> {
    mcp.call(name, args)
        .map_err(|error| ToolError::Refused(format!("{name}: {error}")))
}

fn id_of(value: &Value) -> Result<u64, ToolError> {
    value
        .get("id")
        .or_else(|| value.get("doc"))
        .and_then(Value::as_u64)
        .ok_or_else(|| ToolError::Unreadable("инструмент не вернул id документа".into()))
}

/// Обрезка по границе символа: резать UTF-8 посередине нельзя, иначе
/// модель получит битый текст и решит, что документ повреждён.
fn first_chars(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_string();
    }
    let mut end = limit;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

fn clip(text: String) -> (String, bool) {
    if text.len() <= MAX_TEXT {
        return (text, false);
    }
    let mut end = MAX_TEXT;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    (
        format!(
            "{}\n\n… обрезано: показано {end} байт из {}",
            &text[..end],
            text.len()
        ),
        true,
    )
}

fn path_string(path: &Path) -> String {
    path.display().to_string()
}

fn read_word(mcp: &mut Mcp, path: &Path) -> Result<String, ToolError> {
    let opened = parse(
        call(mcp, "open_document", json!({ "path": path_string(path) }))?,
        "open_document",
    )?;
    let doc = id_of(&opened)?;
    let text = parse(call(mcp, "get_text", json!({ "doc": doc }))?, "get_text")?;
    Ok(text
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string())
}

fn read_pdf(mcp: &mut Mcp, path: &Path) -> Result<String, ToolError> {
    let opened = parse(
        call(mcp, "doc_open", json!({ "path": path_string(path) }))?,
        "doc_open",
    )?;
    let doc = id_of(&opened)?;
    let extracted = parse(
        call(mcp, "text_extract", json!({ "doc": doc }))?,
        "text_extract",
    )?;

    let mut out = String::new();
    for page in extracted
        .get("pages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let Some(text) = page.get("text").and_then(Value::as_str) else {
            continue;
        };
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(text);
    }
    Ok(out)
}

fn read_grid(mcp: &mut Mcp) -> Result<String, ToolError> {
    let info = parse(call(mcp, "inspect_workbook", json!({}))?, "inspect_workbook")?;
    let sheets = info
        .get("sheets")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut out = String::new();
    for sheet in sheets {
        let name = sheet
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("Лист")
            .to_string();
        let range = sheet
            .get("usedRange")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        out.push_str(&format!("== {name} ==\n"));
        if range.is_empty() {
            out.push_str("(пусто)\n\n");
            continue;
        }

        let values = parse(
            call(mcp, "read_range", json!({ "sheet": name, "range": range }))?,
            "read_range",
        )?;
        for row in values
            .get("values")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
        {
            let cells: Vec<String> = row
                .as_array()
                .map(|cells| {
                    cells
                        .iter()
                        .map(|cell| match cell {
                            Value::Null => String::new(),
                            Value::String(text) => text.clone(),
                            other => other.to_string(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            out.push_str(cells.join("\t").trim_end());
            out.push('\n');
        }
        out.push('\n');
    }
    Ok(out)
}

fn read_deck(mcp: &mut Mcp, path: &Path) -> Result<String, ToolError> {
    let opened = parse(
        call(mcp, "open_presentation", json!({ "path": path_string(path) }))?,
        "open_presentation",
    )?;
    let slides = opened
        .get("slides")
        .and_then(Value::as_u64)
        .unwrap_or_default();

    let mut out = String::new();
    for index in 0..slides {
        let slide = parse(
            call(mcp, "inspect_slide", json!({ "index": index }))?,
            "inspect_slide",
        )?;
        out.push_str(&format!("## Слайд {}\n", index + 1));

        if let Some(title) = slide.get("title").and_then(Value::as_str) {
            if !title.trim().is_empty() {
                out.push_str(title.trim());
                out.push('\n');
            }
        }
        for shape in slide
            .get("shapes")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
        {
            if let Some(text) = shape.get("text").and_then(Value::as_str) {
                let text = text.trim();
                if !text.is_empty() {
                    out.push_str(text);
                    out.push('\n');
                }
            }
        }
        out.push('\n');
    }
    Ok(out)
}

/// Прочитать документ любого поддерживаемого формата.
pub struct DocsRead;

impl Tool for DocsRead {
    fn spec(&self, settings: &Settings) -> ToolSpec {
        ToolSpec {
            id: "docs.read".into(),
            capability: "docs.office".into(),
            name: "Прочитать документ".into(),
            description: format!(
                "Прочитать текст документа. Понимает docx, doc, odt, rtf, md, txt, tex, html, \
                 pdf, xlsx, xlsm, csv, tsv, pptx. Путь полный, файл должен лежать внутри \
                 разрешённых папок. Для больших документов возвращает начало. {}",
                crate::tools::папки_словами(settings)
            ),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Полный путь к документу, например C:\\Users\\me\\Документы\\отчёт.docx"
                    }
                },
                "required": ["path"]
            }),
            side_effecting: false,
        }
    }

    fn call(&self, args: &Value, settings: &Settings) -> ToolResult {
        ensure_allowed(settings, "files.read")?;
        let raw = arg_str(args, "path")?;
        let path = resolve_inside(&raw, settings)?;

        if path.is_dir() {
            return Err(ToolError::BadArgs(format!(
                "{raw} это папка. Список файлов: fs.list"
            )));
        }
        if !path.is_file() {
            return Err(ToolError::Missing(raw));
        }

        let app = app_for(&path)?;
        let mut mcp = connect_read(app, &path)?;
        let text = match app {
            App::Wordcraft => read_word(&mut mcp, &path),
            App::Pdfcraft => read_pdf(&mut mcp, &path),
            App::Gridcraft => read_grid(&mut mcp),
            App::Deckcraft => read_deck(&mut mcp, &path),
        }?;
        mcp.stop();

        let empty = text.trim().is_empty();
        let (text, truncated) = clip(text);
        let text = if empty {
            "(текста в документе нет)".to_string()
        } else {
            text
        };
        let note = if truncated { ", начало" } else { "" };
        let name = path
            .file_name()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_default();
        Ok(ToolOutput::read(
            text,
            format!("прочитан {name} через {}{note}", app.title()),
        ))
    }
}

/// Структура и свойства документа: страницы, листы, слайды.
pub struct DocsInfo;

impl Tool for DocsInfo {
    fn spec(&self, settings: &Settings) -> ToolSpec {
        ToolSpec {
            id: "docs.info".into(),
            capability: "docs.office".into(),
            name: "Сведения о документе".into(),
            description: format!(
                "Показать структуру документа: страницы и разделы у docx и pdf, листы и \
                 использованные диапазоны у xlsx, слайды и макеты у pptx. {}",
                crate::tools::папки_словами(settings)
            ),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Полный путь к документу" }
                },
                "required": ["path"]
            }),
            side_effecting: false,
        }
    }

    fn call(&self, args: &Value, settings: &Settings) -> ToolResult {
        ensure_allowed(settings, "files.read")?;
        let raw = arg_str(args, "path")?;
        let path = resolve_inside(&raw, settings)?;
        if !path.is_file() {
            return Err(ToolError::Missing(raw));
        }

        let app = app_for(&path)?;
        let mut mcp = connect_read(app, &path)?;
        let info = match app {
            App::Wordcraft => {
                let opened = parse(
                    call(&mut mcp, "open_document", json!({ "path": path_string(&path) }))?,
                    "open_document",
                )?;
                let doc = id_of(&opened)?;
                parse(
                    call(&mut mcp, "inspect_document", json!({ "doc": doc }))?,
                    "inspect_document",
                )?
            }
            App::Pdfcraft => {
                let opened = parse(
                    call(&mut mcp, "doc_open", json!({ "path": path_string(&path) }))?,
                    "doc_open",
                )?;
                let doc = id_of(&opened)?;
                parse(call(&mut mcp, "doc_info", json!({ "doc": doc }))?, "doc_info")?
            }
            App::Gridcraft => parse(call(&mut mcp, "inspect_workbook", json!({}))?, "inspect_workbook")?,
            App::Deckcraft => {
                parse(
                    call(&mut mcp, "open_presentation", json!({ "path": path_string(&path) }))?,
                    "open_presentation",
                )?;
                parse(call(&mut mcp, "inspect_document", json!({}))?, "inspect_document")?
            }
        };
        mcp.stop();

        let text = serde_json::to_string_pretty(&info).unwrap_or_else(|_| info.to_string());
        let (text, truncated) = clip(text);
        let note = if truncated { " (начало)" } else { "" };
        Ok(ToolOutput::read(
            text,
            format!("сведения о документе через {}{note}", app.title()),
        ))
    }
}

/// Создать документ из текста.
pub struct DocsWrite;

impl Tool for DocsWrite {
    fn spec(&self, _settings: &Settings) -> ToolSpec {
        ToolSpec {
            id: "docs.write".into(),
            capability: "files.write".into(),
            name: "Создать документ".into(),
            // Список форматов — в описании: иначе модель пробует .docx
            // для таблицы и удивляется отказу.
            description: "Создать документ из текста. Формат выбирается по расширению: \
                          docx, pdf, odt, rtf, html, md, txt, tex. Существующий файл не \
                          перезаписывается: укажите новое имя."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Полный путь нового файла, например C:\\Users\\me\\Документы\\письмо.docx"
                    },
                    "text": {
                        "type": "string",
                        "description": "Текст документа. Пустая строка разделяет абзацы."
                    }
                },
                "required": ["path", "text"]
            }),
            side_effecting: true,
        }
    }

    fn call(&self, args: &Value, settings: &Settings) -> ToolResult {
        let raw = arg_str(args, "path")?;
        let path = resolve_inside(&raw, settings)?;
        let text = arg_str(args, "text")?;

        let ext = extension(&path);
        if !WRITE_EXTENSIONS.contains(&ext.as_str()) {
            return Err(ToolError::BadArgs(format!(
                "формат «.{ext}» не создаётся. Поддерживаются: docx, pdf, odt, rtf, html, md, txt, tex"
            )));
        }
        if path.exists() {
            return Err(ToolError::Refused(format!(
                "{raw} уже существует. Запись поверх не делаем: выберите новое имя"
            )));
        }
        if let Some(parent) = path.parent() {
            if !parent.is_dir() {
                return Err(ToolError::Missing(format!(
                    "папки {} нет",
                    parent.display()
                )));
            }
        }

        let mut mcp = connect_write(App::Wordcraft, &path)?;
        let created = parse(
            call(&mut mcp, "new_document", json!({ "template": "blank" }))?,
            "new_document",
        )?;
        let doc = id_of(&created)?;
        parse(
            call(&mut mcp, "type_text", json!({ "doc": doc, "text": text, "paragraphs": true }))?,
            "type_text",
        )?;
        let saved = call(
            &mut mcp,
            "save_document",
            json!({ "doc": doc, "path": path_string(&path) }),
        )?;
        mcp.stop();

        if saved.is_error {
            return Err(ToolError::Refused(format!(
                "save_document: {}",
                saved.text.trim()
            )));
        }

        let name = path
            .file_name()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_default();
        Ok(ToolOutput::changed(
            format!("создан {name}"),
            format!("создан документ {}", path.display()),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn формат_выбирает_правильный_cli() {
        let cases = [
            ("C:\\Документы\\отчёт.docx", App::Wordcraft),
            ("C:\\Документы\\заметка.md", App::Wordcraft),
            ("C:\\Документы\\счёт.pdf", App::Pdfcraft),
            ("C:\\Документы\\бюджет.xlsx", App::Gridcraft),
            ("C:\\Документы\\данные.csv", App::Gridcraft),
            ("C:\\Документы\\презентация.pptx", App::Deckcraft),
        ];
        for (path, expected) in cases {
            assert_eq!(app_for(Path::new(path)).unwrap(), expected, "{path}");
        }
    }

    /// Неизвестное расширение — отказ со списком, а не попытка открыть
    /// файл «на удачу».
    #[test]
    fn неизвестный_формат_называет_поддерживаемые() {
        let error = app_for(Path::new("C:\\Документы\\архив.7z")).expect_err("должен быть отказ");
        let text = error.to_string();
        assert!(text.contains("7z"), "{text}");
        assert!(text.contains("docx"), "{text}");
        assert!(text.contains("pptx"), "{text}");
    }

    #[test]
    fn обрезка_не_ломает_кириллицу() {
        let text = "я".repeat(MAX_TEXT); // каждый символ — два байта
        let (clipped, was) = clip(text);
        assert!(was);
        assert!(clipped.len() > MAX_TEXT);
        assert!(clipped.contains("обрезано"));
        // Главное: результат — валидная строка, а не обрубок.
        assert!(std::str::from_utf8(clipped.as_bytes()).is_ok());
    }

    #[test]
    fn короткий_текст_не_обрезается() {
        let (text, was) = clip("коротко".into());
        assert!(!was);
        assert_eq!(text, "коротко");
    }

    /// Отказ CLI — это `Refused` с текстом причины. Иначе модель
    /// получит «не удалось» без объяснения и повторит то же самое.
    #[test]
    fn отказ_cli_становится_причиной() {
        let reply = ToolReply {
            text: "outside the read root".into(),
            is_error: true,
        };
        let error = parse(reply, "open_document").expect_err("должен быть отказ");
        match error {
            ToolError::Refused(text) => {
                assert!(text.contains("open_document"), "{text}");
                assert!(text.contains("outside the read root"), "{text}");
            }
            other => panic!("ожидался Refused, получен {other:?}"),
        }
    }

    #[test]
    fn id_читается_из_поля_id_и_из_поля_doc() {
        assert_eq!(id_of(&json!({ "id": 3 })).unwrap(), 3);
        assert_eq!(id_of(&json!({ "doc": 7 })).unwrap(), 7);
        assert!(id_of(&json!({ "нет": 1 })).is_err());
    }

    #[test]
    fn первый_фрагмент_режется_по_границе_символа() {
        let text = "привет";
        let short = first_chars(text, 4);
        assert_eq!(short.chars().count(), 2, "два символа — четыре байта");
        assert_eq!(first_chars(text, 100), text);
    }
}
