// FTS5, поиск по точным словам.
//
// Полнотекстовый поиск по корпусу документов пользователя.
// Использует SQLite FTS5 для быстрого поиска по ключевым словам.
// Работает локально, без эмбеддингов и без облака.

use rusqlite::{params, Connection};
use serde_json::Value;

/// Инициализировать FTS5 таблицу (если не существует).
pub fn init_fts(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS memory_fts USING fts5(
            title, content, source, tokenize='unicode61'
        );
        CREATE TABLE IF NOT EXISTS memory_documents (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            title TEXT NOT NULL,
            source TEXT NOT NULL,      -- путь к файлу или URL
            content TEXT NOT NULL,
            chunk_index INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_memory_documents_source ON memory_documents(source);",
    )?;
    Ok(())
}

/// Добавить документ (или его фрагмент) в индекс.
pub fn add_document(
    conn: &Connection,
    title: &str,
    source: &str,
    content: &str,
    chunk_index: i32,
) -> Result<i64, rusqlite::Error> {
    conn.execute(
        "INSERT INTO memory_documents (title, source, content, chunk_index) VALUES (?1, ?2, ?3, ?4)",
        params![title, source, content, chunk_index],
    )?;
    let doc_id = conn.last_insert_rowid();

    // Добавляем в FTS-индекс
    conn.execute(
        "INSERT INTO memory_fts (rowid, title, content, source) VALUES (?1, ?2, ?3, ?4)",
        params![doc_id, title, content, source],
    )?;

    Ok(doc_id)
}

/// Результат поиска.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub id: i64,
    pub title: String,
    pub source: String,
    pub content: String,
    pub chunk_index: i32,
    pub rank: f64,
}

/// Полнотекстовый поиск по корпусу.
pub fn search(conn: &Connection, query: &str, limit: usize) -> Result<Vec<SearchResult>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT d.id, d.title, d.source, d.content, d.chunk_index,
                bm25(memory_fts) AS rank
         FROM memory_fts
         JOIN memory_documents d ON d.id = memory_fts.rowid
         WHERE memory_fts MATCH ?1
         ORDER BY rank
         LIMIT ?2"
    )?;

    let rows = stmt.query_map(params![query, limit], |row| {
        Ok(SearchResult {
            id: row.get(0)?,
            title: row.get(1)?,
            source: row.get(2)?,
            content: row.get(3)?,
            chunk_index: row.get(4)?,
            rank: row.get(5)?,
        })
    })?;

    rows.collect::<Result<Vec<_>, _>>()
}

/// Удалить документ и его фрагменты из индекса.
pub fn remove_document(conn: &Connection, source: &str) -> Result<usize, rusqlite::Error> {
    // Находим все id документов с этим источником
    let mut stmt = conn.prepare("SELECT id FROM memory_documents WHERE source = ?1")?;
    let ids: Vec<i64> = stmt.query_map(params![source], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?;

    // Удаляем из FTS и документов
    for id in &ids {
        conn.execute("DELETE FROM memory_fts WHERE rowid = ?1", params![id])?;
        conn.execute("DELETE FROM memory_documents WHERE id = ?1", params![id])?;
    }

    Ok(ids.len())
}

/// Разбить текст на фрагменты (chunks) для индексации.
///
/// Простой разбиение по абзацам с ограничением размера.
pub fn chunk_text(text: &str, max_chunk_size: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();

    for paragraph in text.split("\n\n") {
        let trimmed = paragraph.trim();
        if trimmed.is_empty() {
            continue;
        }

        if current.len() + trimmed.len() > max_chunk_size && !current.is_empty() {
            chunks.push(current.trim().to_string());
            current.clear();
        }

        if current.is_empty() {
            current.push_str(trimmed);
        } else {
            current.push_str("\n\n");
            current.push_str(trimmed);
        }
    }

    if !current.trim().is_empty() {
        chunks.push(current.trim().to_string());
    }

    if chunks.is_empty() {
        chunks.push(text.to_string());
    }

    chunks
}

/// Статистика корпуса.
pub struct CorpusStats {
    pub total_documents: usize,
    pub total_chunks: usize,
    pub unique_sources: usize,
}

pub fn corpus_stats(conn: &Connection) -> Result<CorpusStats, rusqlite::Error> {
    let total_chunks: usize = conn.query_row(
        "SELECT COUNT(*) FROM memory_documents",
        [],
        |row| row.get(0),
    )?;
    let total_documents: usize = conn.query_row(
        "SELECT COUNT(DISTINCT source) FROM memory_documents",
        [],
        |row| row.get(0),
    )?;
    Ok(CorpusStats {
        total_documents,
        total_chunks,
        unique_sources: total_documents,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        init_fts(&conn).unwrap();
        conn
    }

    #[test]
    fn add_and_search() {
        let conn = make_conn();
        add_document(&conn, "test.txt", "test.txt", "Hello world, this is a test document about Rust programming.", 0).unwrap();

        let results = search(&conn, "Rust", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "test.txt");
        assert!(results[0].content.contains("Rust"));
    }

    #[test]
    fn chunk_text_works() {
        let text = "Paragraph one.\n\nParagraph two.\n\nParagraph three.";
        let chunks = chunk_text(text, 1000);
        assert_eq!(chunks.len(), 1);

        let chunks = chunk_text(text, 20);
        assert!(chunks.len() >= 2);
    }

    #[test]
    fn remove_document_works() {
        let conn = make_conn();
        add_document(&conn, "a.txt", "a.txt", "Content A", 0).unwrap();
        add_document(&conn, "b.txt", "b.txt", "Content B", 0).unwrap();

        let removed = remove_document(&conn, "a.txt").unwrap();
        assert_eq!(removed, 1);

        let results = search(&conn, "Content", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].source, "b.txt");
    }

    #[test]
    fn corpus_stats_works() {
        let conn = make_conn();
        add_document(&conn, "a.txt", "a.txt", "Content A", 0).unwrap();
        add_document(&conn, "a.txt", "a.txt", "Content A chunk 2", 1).unwrap();
        add_document(&conn, "b.txt", "b.txt", "Content B", 0).unwrap();

        let stats = corpus_stats(&conn).unwrap();
        assert_eq!(stats.total_chunks, 3);
        assert_eq!(stats.total_documents, 2);
    }
}