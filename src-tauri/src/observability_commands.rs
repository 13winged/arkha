// Команды Tauri для наблюдения (дашборд метрик).
//
// Обёртка над crate::memory::observability. Отдаёт трейсы, спаны и
// метрики во фронтенд для визуализации.

use serde::Serialize;

use crate::memory::observability;
use crate::memory::sqlite;

#[derive(Serialize)]
pub struct TraceView {
    pub trace_id: String,
    pub name: String,
    pub metadata: serde_json::Value,
}

#[derive(Serialize)]
pub struct SpanView {
    pub span_id: String,
    pub trace_id: String,
    pub parent_span_id: Option<String>,
    pub name: String,
    pub input: serde_json::Value,
    pub output: serde_json::Value,
    pub metadata: serde_json::Value,
}

#[derive(Serialize)]
pub struct MetricView {
    pub name: String,
    pub value: f64,
    pub unit: Option<String>,
    pub timestamp: String,
    pub tags: serde_json::Value,
}

#[derive(Serialize)]
pub struct DashboardSummary {
    pub total_traces: usize,
    pub total_spans: usize,
    pub avg_latency_ms: Option<f64>,
    pub total_tokens: f64,
    pub traces: Vec<TraceView>,
}

/// Получить сводку наблюдения для дашборда.
#[tauri::command]
pub fn obs_dashboard(limit: Option<usize>) -> Result<DashboardSummary, String> {
    crate::auth::require_authenticated()?;
    let conn = sqlite::open(&crate::user_data::current_roots()?.memory_db)
        .map_err(|e| format!("не удалось открыть базу: {e}"))?;

    let limit = limit.unwrap_or(20);
    let traces = observability::list_traces(&conn, limit)
        .map_err(|e| format!("ошибка чтения трейсов: {e}"))?;

    let mut total_spans = 0usize;
    let mut latencies: Vec<f64> = Vec::new();
    let mut total_tokens = 0.0;

    for t in &traces {
        let spans = observability::list_spans_for_trace(&conn, &t.trace_id)
            .map_err(|e| format!("ошибка чтения спанов: {e}"))?;
        total_spans += spans.len();
        for s in &spans {
            // Ищем метрики задержки
            if s.name.contains("latency") || s.name.contains("ms") {
                if let Some(v) = s.input.get("latency_ms").and_then(|v| v.as_f64()) {
                    latencies.push(v);
                }
                if let Some(v) = s.output.get("latency_ms").and_then(|v| v.as_f64()) {
                    latencies.push(v);
                }
            }
            // Ищем метрики токенов
            if let Some(v) = s.input.get("tokens").and_then(|v| v.as_f64()) {
                total_tokens += v;
            }
            if let Some(v) = s.output.get("tokens").and_then(|v| v.as_f64()) {
                total_tokens += v;
            }
        }
    }

    let avg_latency_ms = if latencies.is_empty() {
        None
    } else {
        Some(latencies.iter().sum::<f64>() / latencies.len() as f64)
    };

    let traces_view: Vec<TraceView> = traces
        .into_iter()
        .map(|t| TraceView {
            trace_id: t.trace_id,
            name: t.name,
            metadata: t.metadata,
        })
        .collect();

    Ok(DashboardSummary {
        total_traces: traces_view.len(),
        total_spans,
        avg_latency_ms,
        total_tokens,
        traces: traces_view,
    })
}

/// Получить спаны для трейса.
#[tauri::command]
pub fn obs_spans(trace_id: String) -> Result<Vec<SpanView>, String> {
    crate::auth::require_authenticated()?;
    let conn = sqlite::open(&crate::user_data::current_roots()?.memory_db)
        .map_err(|e| format!("не удалось открыть базу: {e}"))?;

    let spans = observability::list_spans_for_trace(&conn, &trace_id)
        .map_err(|e| format!("ошибка чтения спанов: {e}"))?;

    Ok(spans
        .into_iter()
        .map(|s| SpanView {
            span_id: s.span_id,
            trace_id: s.trace_id,
            parent_span_id: s.parent_span_id,
            name: s.name,
            input: s.input,
            output: s.output,
            metadata: s.metadata,
        })
        .collect())
}

/// Точка временной шкалы использования токенов.
#[derive(Serialize, Clone)]
pub struct TimelinePoint {
    pub label: String,
    pub tokens: f64,
    pub traces: usize,
}

/// Временная шкала использования токенов.
#[tauri::command]
pub fn obs_timeline(hours: Option<usize>) -> Result<Vec<TimelinePoint>, String> {
    crate::auth::require_authenticated()?;
    let conn = sqlite::open(&crate::user_data::current_roots()?.memory_db)
        .map_err(|e| format!("не удалось открыть базу: {e}"))?;

    let hours = hours.unwrap_or(24);

    let mut stmt = conn
        .prepare(
            "SELECT strftime('%Y-%m-%d %H:00', start_time) AS hour,
                    COUNT(*) AS trace_count,
                    SUM(COALESCE((SELECT SUM(value) FROM obs_metrics WHERE trace_id = t.trace_id AND name = 'tokens_used'), 0)) AS tokens
             FROM obs_traces t
             WHERE start_time >= datetime('now', ?1)
             GROUP BY hour
             ORDER BY hour",
        )
        .map_err(|e| format!("ошибка SQL: {e}"))?;

    let offset = format!("-{} hours", hours);
    let rows: Vec<TimelinePoint> = stmt
        .query_map(rusqlite::params![offset], |row| {
            Ok(TimelinePoint {
                label: row.get(0)?,
                tokens: row.get::<_, Option<f64>>(2)?.unwrap_or(0.0),
                traces: row.get::<_, i64>(1)?.max(0) as usize,
            })
        })
        .map_err(|e| format!("ошибка запроса: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("ошибка чтения: {e}"))?;

    Ok(rows)
}

/// Бюджет токенов.
#[derive(Serialize)]
pub struct TokenBudget {
    pub spent: f64,
    pub limit: f64,
    pub percent: f64,
    pub status: String, // "ok", "warning", "critical"
}

/// Получить бюджет токенов.
#[tauri::command]
pub fn obs_budget(limit: Option<f64>) -> Result<TokenBudget, String> {
    crate::auth::require_authenticated()?;
    let conn = sqlite::open(&crate::user_data::current_roots()?.memory_db)
        .map_err(|e| format!("не удалось открыть базу: {e}"))?;

    let spent: f64 = conn
        .query_row(
            "SELECT COALESCE(SUM(value), 0) FROM obs_metrics WHERE name = 'tokens_used'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0.0);

    let limit = limit.unwrap_or(100_000.0);
    let percent = if limit > 0.0 { (spent / limit) * 100.0 } else { 0.0 };
    let status = if percent >= 80.0 {
        "critical"
    } else if percent >= 50.0 {
        "warning"
    } else {
        "ok"
    };

    Ok(TokenBudget {
        spent,
        limit,
        percent,
        status: status.to_string(),
    })
}

/// Получить метрики (по имени, если указано).
#[tauri::command]
pub fn obs_metrics(name: Option<String>, limit: Option<usize>) -> Result<Vec<MetricView>, String> {
    crate::auth::require_authenticated()?;
    let conn = sqlite::open(&crate::user_data::current_roots()?.memory_db)
        .map_err(|e| format!("не удалось открыть базу: {e}"))?;

    let mut stmt = if let Some(ref n) = name {
        conn.prepare("SELECT name, value, unit, timestamp, tags FROM obs_metrics WHERE name = ?1 ORDER BY timestamp DESC LIMIT ?2")
            .map_err(|e| format!("ошибка SQL: {e}"))?
    } else {
        conn.prepare("SELECT name, value, unit, timestamp, tags FROM obs_metrics ORDER BY timestamp DESC LIMIT ?1")
            .map_err(|e| format!("ошибка SQL: {e}"))?
    };

    let limit = limit.unwrap_or(50);

    let rows: Vec<MetricView> = if let Some(ref n) = name {
        stmt.query_map(rusqlite::params![n, limit], |row| {
            Ok(MetricView {
                name: row.get(0)?,
                value: row.get(1)?,
                unit: row.get(2)?,
                timestamp: row.get(3)?,
                tags: serde_json::from_str(&row.get::<_, String>(4)?).unwrap_or_default(),
            })
        })
        .map_err(|e| format!("ошибка запроса: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("ошибка чтения: {e}"))?
    } else {
        stmt.query_map(rusqlite::params![limit], |row| {
            Ok(MetricView {
                name: row.get(0)?,
                value: row.get(1)?,
                unit: row.get(2)?,
                timestamp: row.get(3)?,
                tags: serde_json::from_str(&row.get::<_, String>(4)?).unwrap_or_default(),
            })
        })
        .map_err(|e| format!("ошибка запроса: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("ошибка чтения: {e}"))?
    };

    Ok(rows)
}
