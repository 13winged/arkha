// SQLite подключение и схема для локальной памяти.
//
// Хранит:
//   - граф памяти (сущности, отношения, факты) — аналог Graphiti, но локально
//   - наблюдение (трейсы, спаны, метрики) — аналог Opik, но локально
//
// Всё в одном файле SQLite рядом с данными пользователя. Никакого облака.

use rusqlite::{Connection, OpenFlags};
use std::path::Path;

/// Открыть (или создать) базу памяти.
///
/// Таблицы создаются автоматически при первом запуске.
pub fn open(path: &Path) -> Result<Connection, rusqlite::Error> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
    )?;
    conn.execute_batch(include_str!("schema.sql"))?;
    Ok(conn)
}

/// Проверить, что база жива (можно выполнить простой запрос).
pub fn health_check(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.query_row("SELECT 1", [], |_| Ok(()))
}

/// Применить схему (создать таблицы, если не существуют).
pub fn init_schema(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(include_str!("schema.sql"))?;
    Ok(())
}