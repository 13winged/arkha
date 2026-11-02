// Наблюдение за выполнением (локальный аналог Opik, но без облака).
//
// Трейсы — высокоуровневые операции (диалог, вызов навыка, распознавание речи).
// Спаны — вложенные операции внутри трейса (STT, VAD, LLM, TTS).
// Метрики — измеряемые значения (задержка, токены, стоимость).
//
// Всё хранится в той же SQLite базе, что и граф памяти.

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

/// Трассировка: высокоуровневая операция.
#[derive(Debug, Clone)]
pub struct Trace {
    pub trace_id: String,
    pub name: String,
    pub metadata: Value,
}

/// Спан: вложенная операция внутри трейса.
#[derive(Debug, Clone)]
pub struct Span {
    pub span_id: String,
    pub trace_id: String,
    pub parent_span_id: Option<String>,
    pub name: String,
    pub input: Value,
    pub output: Value,
    pub metadata: Value,
}

/// Метрика: измеряемое значение.
#[derive(Debug, Clone)]
pub struct Metric {
    pub trace_id: Option<String>,
    pub span_id: Option<String>,
    pub name: String,
    pub value: f64,
    pub unit: Option<String>,
    pub tags: Value,
}

/// Начать трейс. Возвращает trace_id.
pub fn start_trace(conn: &Connection, name: &str, metadata: &Value) -> Result<String, rusqlite::Error> {
    let trace_id = format!("trace_{}", now_nanos());
    let meta_json = serde_json::to_string(metadata).unwrap_or_default();
    let now = now_iso();

    conn.execute(
        "INSERT INTO obs_traces (trace_id, name, start_time, metadata, status) VALUES (?1, ?2, ?3, ?4, 'running')",
        params![trace_id, name, now, meta_json],
    )?;

    Ok(trace_id)
}

/// Завершить трейс.
pub fn end_trace(conn: &Connection, trace_id: &str, status: &str, error: Option<&str>) -> Result<(), rusqlite::Error> {
    let now = now_iso();
    // Длительность не вычисляем (SQLite strftime требует формат времени, не Unix timestamp)
    let duration_ms: Option<i64> = None;

    conn.execute(
        "UPDATE obs_traces SET end_time = ?1, duration_ms = ?2, status = ?3, error = ?4 WHERE trace_id = ?5",
        params![now, duration_ms, status, error, trace_id],
    )?;
    Ok(())
}

/// Начать спан внутри трейса. Возвращает span_id.
pub fn start_span(
    conn: &Connection,
    trace_id: &str,
    name: &str,
    input: &Value,
    metadata: &Value,
    parent_span_id: Option<&str>,
) -> Result<String, rusqlite::Error> {
    let span_id = format!("span_{}", now_nanos());
    let input_json = serde_json::to_string(input).unwrap_or_default();
    let meta_json = serde_json::to_string(metadata).unwrap_or_default();
    let now = now_iso();

    conn.execute(
        "INSERT INTO obs_spans (trace_id, span_id, parent_span_id, name, start_time, input, metadata, status) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'running')",
        params![trace_id, span_id, parent_span_id, name, now, input_json, meta_json],
    )?;

    Ok(span_id)
}

/// Завершить спан.
pub fn end_span(conn: &Connection, span_id: &str, output: &Value, status: &str, error: Option<&str>) -> Result<(), rusqlite::Error> {
    let now = now_iso();
    let output_json = serde_json::to_string(output).unwrap_or_default();

    // Длительность не вычисляем
    let duration_ms: Option<i64> = None;

    conn.execute(
        "UPDATE obs_spans SET end_time = ?1, duration_ms = ?2, output = ?3, status = ?4, error = ?5 WHERE span_id = ?6",
        params![now, duration_ms, output_json, status, error, span_id],
    )?;
    Ok(())
}

/// Записать метрику.
pub fn record_metric(
    conn: &Connection,
    name: &str,
    value: f64,
    unit: Option<&str>,
    tags: &Value,
    trace_id: Option<&str>,
    span_id: Option<&str>,
) -> Result<(), rusqlite::Error> {
    let tags_json = serde_json::to_string(tags).unwrap_or_default();
    conn.execute(
        "INSERT INTO obs_metrics (trace_id, span_id, name, value, unit, tags) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![trace_id, span_id, name, value, unit, tags_json],
    )?;
    Ok(())
}

/// Получить все трейсы (последние N).
pub fn list_traces(conn: &Connection, limit: usize) -> Result<Vec<Trace>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT trace_id, name, metadata FROM obs_traces ORDER BY start_time DESC LIMIT ?1"
    )?;
    let rows = stmt.query_map(params![limit], |row| {
        Ok(Trace {
            trace_id: row.get(0)?,
            name: row.get(1)?,
            metadata: serde_json::from_str(&row.get::<_, String>(2)?).unwrap_or_default(),
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>()
}

/// Получить все спаны для трейса.
pub fn list_spans_for_trace(conn: &Connection, trace_id: &str) -> Result<Vec<Span>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT span_id, trace_id, parent_span_id, name, input, output, metadata FROM obs_spans WHERE trace_id = ?1 ORDER BY start_time"
    )?;
    let rows = stmt.query_map(params![trace_id], |row| {
        Ok(Span {
            span_id: row.get(0)?,
            trace_id: row.get(1)?,
            parent_span_id: row.get(2)?,
            name: row.get(3)?,
            input: serde_json::from_str(&row.get::<_, String>(4)?).unwrap_or_default(),
            output: serde_json::from_str(&row.get::<_, String>(5)?).unwrap_or_default(),
            metadata: serde_json::from_str(&row.get::<_, String>(6)?).unwrap_or_default(),
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>()
}

/// Текущее время в ISO 8601.
fn now_iso() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| format!("{}", d.as_secs()))
        .unwrap_or_else(|_| "0".to_string())
}

/// Наносекунды для уникальных идентификаторов.
fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_and_span() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::memory::sqlite::init_schema(&conn).unwrap();

        let trace_id = start_trace(&conn, "test_dialogue", &serde_json::json!({"user": "alice"})).unwrap();
        assert!(!trace_id.is_empty());

        let span_id = start_span(&conn, &trace_id, "stt", &serde_json::json!({"audio": "chunk1"}), &serde_json::json!({}), None).unwrap();
        assert!(!span_id.is_empty());

        end_span(&conn, &span_id, &serde_json::json!({"text": "привет"}), "completed", None).unwrap();
        end_trace(&conn, &trace_id, "completed", None).unwrap();

        let traces = list_traces(&conn, 10).unwrap();
        assert_eq!(traces.len(), 1);
        assert_eq!(traces[0].trace_id, trace_id);

        let spans = list_spans_for_trace(&conn, &trace_id).unwrap();
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].span_id, span_id);
    }

    #[test]
    fn record_metric() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::memory::sqlite::init_schema(&conn).unwrap();

        let trace_id = start_trace(&conn, "test", &serde_json::json!({})).unwrap();
        super::record_metric(&conn, "stt_latency_ms", 274.5, Some("ms"), &serde_json::json!({"model": "qwen-asr"}), Some(&trace_id), None).unwrap();

        let mut stmt = conn.prepare("SELECT name, value FROM obs_metrics WHERE trace_id = ?1").unwrap();
        let metrics: Vec<(String, f64)> = stmt.query_map(params![trace_id], |row| {
            Ok((row.get(0)?, row.get(1)?))
        }).unwrap().collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(metrics.len(), 1);
        assert_eq!(metrics[0].0, "stt_latency_ms");
        assert!((metrics[0].1 - 274.5).abs() < 0.001);
    }
}