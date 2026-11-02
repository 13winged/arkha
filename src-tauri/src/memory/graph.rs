// Граф памяти (локальный аналог Graphiti).
//
// Сущности (узлы), отношения (рёбра) и факты (утверждения).
// Всё хранится в SQLite. Никакого облака.

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use std::time::SystemTime;

use super::sqlite::open as open_db;
use crate::profile::config_dir;

/// Сущность графа памяти.
#[derive(Debug, Clone)]
pub struct Entity {
    pub id: Option<i64>,
    pub name: String,
    pub entity_type: String,
    pub properties: Value,
    pub confidence: f64,
    pub source: Option<String>,
}

/// Отношение между двумя сущностями.
#[derive(Debug, Clone)]
pub struct Relationship {
    pub id: Option<i64>,
    pub source_entity_id: i64,
    pub target_entity_id: i64,
    pub relationship: String,
    pub properties: Value,
    pub confidence: f64,
    pub source: Option<String>,
}

/// Факт (утверждение о сущности).
#[derive(Debug, Clone)]
pub struct Fact {
    pub id: Option<i64>,
    pub entity_id: i64,
    pub fact_text: String,
    pub source: Option<String>,
    pub confidence: f64,
}

/// Результат поиска связанных сущностей.
#[derive(Debug, Clone)]
pub struct RelatedEntity {
    pub entity: Entity,
    pub relationship: String,
    pub relationship_confidence: f64,
}

/// Открыть базу памяти по умолчанию (в конфигурационной папке).
pub fn default_connection() -> Result<Connection, rusqlite::Error> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|e| rusqlite::Error::InvalidPath(e.to_string().into()))?;
    let path = dir.join("memory.db");
    super::sqlite::open(&path)
}

/// Создать или обновить сущность.
///
/// Если сущность с таким именем и типом уже существует — обновляем её.
pub fn upsert_entity(conn: &Connection, entity: &Entity) -> Result<i64, rusqlite::Error> {
    // Проверяем, существует ли уже такая сущность
    let existing: Option<i64> = conn.query_row(
        "SELECT id FROM memory_entities WHERE name = ?1 AND entity_type = ?2",
        params![entity.name, entity.entity_type],
        |row| row.get(0),
    ).optional()?;

    let props_json = serde_json::to_string(&entity.properties)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    let now = now_iso();

    match existing {
        Some(id) => {
            conn.execute(
                "UPDATE memory_entities SET properties = ?1, confidence = ?2, updated_at = ?3, source = COALESCE(?4, source) WHERE id = ?5",
                params![props_json, entity.confidence, now, entity.source, id],
            )?;
            Ok(id)
        }
        None => {
            let props_json = serde_json::to_string(&entity.properties)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            conn.execute(
                "INSERT INTO memory_entities (name, entity_type, properties, confidence, source) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![entity.name, entity.entity_type, props_json, entity.confidence, entity.source],
            )?;
            Ok(conn.last_insert_rowid())
        }
    }
}

/// Создать отношение между двумя сущностями.
pub fn add_relationship(conn: &Connection, rel: &Relationship) -> Result<i64, rusqlite::Error> {
    let props_json = serde_json::to_string(&rel.properties)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    conn.execute(
        "INSERT INTO memory_relationships (source_entity_id, target_entity_id, relationship, properties, confidence, source) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![rel.source_entity_id, rel.target_entity_id, rel.relationship, props_json, rel.confidence, rel.source],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Добавить факт о сущности.
pub fn add_fact(conn: &Connection, fact: &Fact) -> Result<i64, rusqlite::Error> {
    conn.execute(
        "INSERT INTO memory_facts (entity_id, fact_text, source, confidence) VALUES (?1, ?2, ?3, ?4)",
        params![fact.entity_id, fact.fact_text, fact.source, fact.confidence],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Найти сущность по имени и типу.
pub fn find_entity(conn: &Connection, name: &str, entity_type: &str) -> Result<Option<Entity>, rusqlite::Error> {
    conn.query_row(
        "SELECT id, name, entity_type, properties, confidence, source FROM memory_entities WHERE name = ?1 AND entity_type = ?2",
        params![name, entity_type],
        |row| {
            Ok(Entity {
                id: row.get(0)?,
                name: row.get(1)?,
                entity_type: row.get(2)?,
                properties: serde_json::from_str(&row.get::<_, String>(3)?).unwrap_or_default(),
                confidence: row.get(4)?,
                source: row.get(5)?,
            })
        }
    ).optional()
}

/// Найти сущность по ID.
pub fn get_entity(conn: &Connection, id: i64) -> Result<Option<Entity>, rusqlite::Error> {
    conn.query_row(
        "SELECT id, name, entity_type, properties, confidence, source FROM memory_entities WHERE id = ?1",
        params![id],
        |row| {
            Ok(Entity {
                id: row.get(0)?,
                name: row.get(1)?,
                entity_type: row.get(2)?,
                properties: serde_json::from_str(&row.get::<_, String>(3)?).unwrap_or_default(),
                confidence: row.get(4)?,
                source: row.get(5)?,
            })
        }
    ).optional()
}

/// Поиск сущностей по подстроке имени.
pub fn search_entities(conn: &Connection, query: &str, limit: usize) -> Result<Vec<Entity>, rusqlite::Error> {
    let like = format!("%{}%", query);
    let mut stmt = conn.prepare(
        "SELECT id, name, entity_type, properties, confidence, source FROM memory_entities WHERE name LIKE ?1 ORDER BY confidence DESC LIMIT ?2"
    )?;
    let rows = stmt.query_map(params![like, limit], |row| {
        Ok(Entity {
            id: row.get(0)?,
            name: row.get(1)?,
            entity_type: row.get(2)?,
            properties: serde_json::from_str(&row.get::<_, String>(3)?).unwrap_or_default(),
            confidence: row.get(4)?,
            source: row.get(5)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>()
}

/// Получить все сущности, связанные с данной.
pub fn get_related(conn: &Connection, entity_id: i64) -> Result<Vec<RelatedEntity>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT r.relationship, r.confidence, t.id, t.name, t.entity_type, t.properties, t.confidence, t.source
         FROM memory_relationships r
         JOIN memory_entities t ON t.id = r.target_entity_id
         WHERE r.source_entity_id = ?1
         UNION ALL
         SELECT r.relationship, r.confidence, s.id, s.name, s.entity_type, s.properties, s.confidence, s.source
         FROM memory_relationships r
         JOIN memory_entities s ON s.id = r.source_entity_id
         WHERE r.target_entity_id = ?1"
    )?;
    let rows = stmt.query_map(params![entity_id], |row| {
        Ok(RelatedEntity {
            relationship: row.get(0)?,
            relationship_confidence: row.get(1)?,
            entity: Entity {
                id: row.get(2)?,
                name: row.get(3)?,
                entity_type: row.get(4)?,
                properties: serde_json::from_str(&row.get::<_, String>(5)?).unwrap_or_default(),
                confidence: row.get(6)?,
                source: row.get(7)?,
            },
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>()
}

/// Получить все факты о сущности.
pub fn get_facts(conn: &Connection, entity_id: i64) -> Result<Vec<Fact>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT id, entity_id, fact_text, source, confidence FROM memory_facts WHERE entity_id = ?1 ORDER BY created_at DESC"
    )?;
    let rows = stmt.query_map(params![entity_id], |row| {
        Ok(Fact {
            id: row.get(0)?,
            entity_id: row.get(1)?,
            fact_text: row.get(2)?,
            source: row.get(3)?,
            confidence: row.get(4)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>()
}

/// Текущее время в ISO 8601.
fn now_iso() -> String {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| format!("{}", d.as_secs()))
        .unwrap_or_else(|_| "0".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_and_find_entity() {
        let conn = open_db(std::path::Path::new(":memory:")).unwrap();
        crate::memory::sqlite::init_schema(&conn).unwrap();

        let id = upsert_entity(&conn, &Entity {
            id: None,
            name: "Иван Иванов".into(),
            entity_type: "person".into(),
            properties: serde_json::json!({"age": 30}),
            confidence: 0.9,
            source: Some("user".into()),
        }).unwrap();

        let found = find_entity(&conn, "Иван Иванов", "person").unwrap();
        assert!(found.is_some());
        let e = found.unwrap();
        assert_eq!(e.id, Some(id));
        assert_eq!(e.confidence, 0.9);
    }

    #[test]
    fn add_relationship_and_get_related() {
        let conn = open_db(std::path::Path::new(":memory:")).unwrap();
        crate::memory::sqlite::init_schema(&conn).unwrap();

        let a = upsert_entity(&conn, &Entity { id: None, name: "Алиса".into(), entity_type: "person".into(), properties: serde_json::json!({}), confidence: 1.0, source: None }).unwrap();
        let b = upsert_entity(&conn, &Entity { id: None, name: "Боб".into(), entity_type: "person".into(), properties: serde_json::json!({}), confidence: 1.0, source: None }).unwrap();

        add_relationship(&conn, &Relationship { id: None, source_entity_id: a, target_entity_id: b, relationship: "knows".into(), properties: serde_json::json!({}), confidence: 1.0, source: None }).unwrap();

        let related = get_related(&conn, a).unwrap();
        assert_eq!(related.len(), 1);
        assert_eq!(related[0].relationship, "knows");
        assert_eq!(related[0].entity.name, "Боб");
    }

    #[test]
    fn add_and_get_facts() {
        let conn = open_db(std::path::Path::new(":memory:")).unwrap();
        crate::memory::sqlite::init_schema(&conn).unwrap();

        let id = upsert_entity(&conn, &Entity { id: None, name: "Тест".into(), entity_type: "concept".into(), properties: serde_json::json!({}), confidence: 1.0, source: None }).unwrap();
        add_fact(&conn, &Fact { id: None, entity_id: id, fact_text: "Тестовый факт".into(), source: None, confidence: 0.8 }).unwrap();

        let facts = get_facts(&conn, id).unwrap();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].fact_text, "Тестовый факт");
    }
}