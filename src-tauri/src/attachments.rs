//! Локальное хранилище вложений: SQLite metadata + файлы на диске.

use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentView {
    pub id: String,
    pub name: String,
    pub mime: String,
    pub size_bytes: u64,
    pub path: String,
    pub indexed: bool,
}

fn database() -> Result<rusqlite::Connection, String> {
    crate::memory::sqlite::open(&crate::profile::config_dir().join("memory.db"))
        .map_err(|error| error.to_string())
}

fn root() -> PathBuf {
    crate::profile::config_dir().join("attachments")
}

fn safe_name(name: &str) -> String {
    let leaf = name.rsplit(['/', '\\']).next().unwrap_or("attachment");
    let clean: String = leaf
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_' | ' ') {
                character
            } else {
                '_'
            }
        })
        .collect();
    if clean.is_empty() || clean == "." || clean == ".." { "attachment".into() } else { clean }
}

#[tauri::command]
pub fn attachment_add(name: String, mime: String, data: Vec<u8>) -> Result<AttachmentView, String> {
    const MAX_BYTES: usize = 50 * 1024 * 1024;
    if data.len() > MAX_BYTES {
        return Err("вложение больше лимита 50 МБ".into());
    }
    let mut digest = Sha256::new();
    digest.update(&data);
    let id = digest.finalize().iter().map(|byte| format!("{byte:02x}")).collect::<String>();
    let directory = root().join(&id);
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let filename = safe_name(&name);
    let path = directory.join(&filename);
    std::fs::write(&path, &data).map_err(|error| error.to_string())?;

    let indexed = matches!(
        path.extension().and_then(|extension| extension.to_str()).map(|extension| extension.to_ascii_lowercase()).as_deref(),
        Some("txt" | "md" | "json" | "csv" | "log" | "toml" | "yaml" | "yml")
    );
    let connection = database()?;
    connection
        .execute(
            "INSERT OR REPLACE INTO attachments (id, name, mime, path, size_bytes, sha256, indexed)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![id, filename, mime, path.to_string_lossy(), data.len() as i64, id, indexed as i64],
        )
        .map_err(|error| error.to_string())?;

    let size_bytes = data.len() as u64;
    if indexed {
        if let Ok(text) = String::from_utf8(data.clone()) {
            let chunks = crate::memory::fts::chunk_text(&text, 1000);
            for (index, chunk) in chunks.into_iter().enumerate() {
                crate::memory::fts::add_document(&connection, &filename, &path.to_string_lossy(), &chunk, index as i32)
                    .map_err(|error| error.to_string())?;
            }
        }
    }
    Ok(AttachmentView {
        id: id.clone(),
        name: filename,
        mime,
        size_bytes,
        path: format!("attachment://{id}"),
        indexed,
    })
}

pub fn context(ids: &[String]) -> Result<Option<String>, String> {
    if ids.is_empty() { return Ok(None); }
    let connection = database()?;
    let mut statement = connection
        .prepare("SELECT id, name, mime, size_bytes FROM attachments WHERE id = ?1")
        .map_err(|error| error.to_string())?;
    let mut lines = Vec::new();
    for id in ids {
        let row = statement.query_row(params![id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, i64>(3)?))
        });
        if let Ok((id, name, mime, size)) = row {
            lines.push(format!("- {name} ({mime}, {size} bytes): attachment://{id}"));
        }
    }
    if lines.is_empty() { Ok(None) } else { Ok(Some(format!("## Вложения текущего запроса\n{}", lines.join("\n")))) }
}

#[tauri::command]
pub fn attachments_delete(id: String) -> Result<(), String> {
    let connection = database()?;
    let path: Option<String> = connection
        .query_row("SELECT path FROM attachments WHERE id = ?1", params![id], |row| row.get(0))
        .optional()
        .map_err(|error| error.to_string())?;
    if let Some(path) = path { let _ = std::fs::remove_file(path); }
    connection.execute("DELETE FROM attachments WHERE id = ?1", params![id]).map_err(|error| error.to_string())?;
    Ok(())
}
