// Команды Tauri для пользовательского RAG (дообучение corporа).
//
// Пользователь добавляет свои документы в локальный корпус, и секретарь
// использует их при ответах. Это «дообучение» без изменения весов модели:
// документы разбиваются на фрагменты, индексируются в FTS5, и при
// вопросе секретарь ищет релевантные фрагменты и добавляет их в контекст.

use serde::Serialize;
use std::path::PathBuf;
use tauri::Emitter;

use crate::memory::fts;
use crate::memory::sqlite;

#[derive(Serialize, Clone)]
pub struct DocumentView {
    pub id: i64,
    pub title: String,
    pub source: String,
    pub chunk_index: i32,
    pub content: String,
}

#[derive(Serialize, Clone)]
pub struct SearchHitView {
    pub id: i64,
    pub title: String,
    pub source: String,
    pub content: String,
    pub rank: f64,
}

#[derive(Serialize)]
pub struct CorpusStatsView {
    pub total_documents: usize,
    pub total_chunks: usize,
    pub unique_sources: usize,
}

/// Открыть окно RAG (дообучения корпусом).
#[tauri::command]
pub async fn rag_open_window(app: tauri::AppHandle) -> Result<(), String> {
    use tauri::Manager;

    if let Some(window) = app.get_webview_window("rag") {
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }

    let _window = tauri::WebviewWindowBuilder::new(
        &app,
        "rag",
        // Vite собирает единый entry point — index.html. Раньше здесь
        // указывался rag.html, которого в dist не было, поэтому WebView2
        // показывал пустое белое окно.
        tauri::WebviewUrl::App("index.html".into()),
    )
    .title("ArkHa — RAG (дообучение корпусом)")
    .inner_size(720.0, 560.0)
    .min_inner_size(600.0, 400.0)
    .build()
    .map_err(|e| format!("не удалось открыть окно RAG: {e}"))?;

    Ok(())
}

/// Добавить текстовый файл в корпус.
#[tauri::command]
pub fn rag_add_file(path: String) -> Result<usize, String> {
    let path = std::path::Path::new(&path);
    let title = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "без названия".to_string());

    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("не удалось прочитать файл: {e}"))?;

    rag_add_text_inner(&title, &path.to_string_lossy(), &content)
}

/// Добавить произвольный текст в корпус.
#[tauri::command]
pub fn rag_add_text(title: String, text: String, source: Option<String>) -> Result<usize, String> {
    let source = source.unwrap_or_else(|| "вручную".to_string());
    rag_add_text_inner(&title, &source, &text)
}

fn rag_add_text_inner(title: &str, source: &str, text: &str) -> Result<usize, String> {
    let conn = sqlite::open(&crate::profile::config_dir().join("memory.db"))
        .map_err(|e| format!("не удалось открыть базу: {e}"))?;

    fts::init_fts(&conn).map_err(|e| format!("ошибка инициализации FTS: {e}"))?;

    let chunks = fts::chunk_text(text, 1000);
    let count = chunks.len();

    for chunk in chunks {
        fts::add_document(&conn, title, source, &chunk, 0)
            .map_err(|e| format!("ошибка добавления фрагмента: {e}"))?;
    }

    Ok(count)
}

/// Поиск по корпусу.
#[tauri::command]
pub fn rag_search(query: String, limit: Option<usize>) -> Result<Vec<SearchHitView>, String> {
    let conn = sqlite::open(&crate::profile::config_dir().join("memory.db"))
        .map_err(|e| format!("не удалось открыть базу: {e}"))?;

    fts::init_fts(&conn).map_err(|e| format!("ошибка инициализации FTS: {e}"))?;

    let limit = limit.unwrap_or(20);
    let results = fts::search(&conn, &query, limit)
        .map_err(|e| format!("ошибка поиска: {e}"))?;

    Ok(results
        .into_iter()
        .map(|r| SearchHitView {
            id: r.id,
            title: r.title,
            source: r.source,
            content: r.content,
            rank: r.rank,
        })
        .collect())
}

/// Удалить документ из корпуса по источнику.
#[tauri::command]
pub fn rag_remove(source: String) -> Result<usize, String> {
    let conn = sqlite::open(&crate::profile::config_dir().join("memory.db"))
        .map_err(|e| format!("не удалось открыть базу: {e}"))?;

    fts::init_fts(&conn).map_err(|e| format!("ошибка инициализации FTS: {e}"))?;

    fts::remove_document(&conn, &source)
        .map_err(|e| format!("ошибка удаления: {e}"))
}

/// Статистика корпуса.
#[tauri::command]
pub fn rag_stats() -> Result<CorpusStatsView, String> {
    let conn = sqlite::open(&crate::profile::config_dir().join("memory.db"))
        .map_err(|e| format!("не удалось открыть базу: {e}"))?;

    fts::init_fts(&conn).map_err(|e| format!("ошибка инициализации FTS: {e}"))?;

    let stats = fts::corpus_stats(&conn).map_err(|e| format!("ошибка статистики: {e}"))?;

    Ok(CorpusStatsView {
        total_documents: stats.total_documents,
        total_chunks: stats.total_chunks,
        unique_sources: stats.unique_sources,
    })
}

/// Список всех документов в корпусе.
#[tauri::command]
pub fn rag_list() -> Result<Vec<DocumentView>, String> {
    let conn = sqlite::open(&crate::profile::config_dir().join("memory.db"))
        .map_err(|e| format!("не удалось открыть базу: {e}"))?;

    fts::init_fts(&conn).map_err(|e| format!("ошибка инициализации FTS: {e}"))?;

    let mut stmt = conn
        .prepare("SELECT id, title, source, chunk_index, content FROM memory_documents ORDER BY source, chunk_index")
        .map_err(|e| format!("ошибка SQL: {e}"))?;

    let rows = stmt
        .query_map([], |row| {
            Ok(DocumentView {
                id: row.get(0)?,
                title: row.get(1)?,
                source: row.get(2)?,
                chunk_index: row.get(3)?,
                content: row.get(4)?,
            })
        })
        .map_err(|e| format!("ошибка запроса: {e}"))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("ошибка чтения: {e}"))
}

/// Отправить текст в чат главного окна.
#[tauri::command]
pub fn rag_ask(app: tauri::AppHandle, text: String) -> Result<(), String> {
    use tauri::Manager;

    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }

    app.emit("rag:ask", text)
        .map_err(|e| format!("не удалось отправить вопрос: {e}"))
}
