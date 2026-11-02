import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

interface TraceView {
  trace_id: string;
  name: string;
  metadata: Record<string, unknown>;
}

interface SpanView {
  span_id: string;
  trace_id: string;
  parent_span_id: string | null;
  name: string;
  input: Record<string, unknown>;
  output: Record<string, unknown>;
  metadata: Record<string, unknown>;
}

interface MetricView {
  name: string;
  value: number;
  unit: string | null;
  timestamp: string;
  tags: Record<string, unknown>;
}

interface DashboardSummary {
  total_traces: number;
  total_spans: number;
  avg_latency_ms: number | null;
  total_tokens: number;
  traces: TraceView[];
}

export default function Dashboard() {
  const [summary, setSummary] = useState<DashboardSummary | null>(null);
  const [spans, setSpans] = useState<SpanView[]>([]);
  const [metrics, setMetrics] = useState<MetricView[]>([]);
  const [selectedTrace, setSelectedTrace] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const loadDashboard = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const data = await invoke<DashboardSummary>("obs_dashboard", { limit: 20 });
      setSummary(data);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  const loadSpans = useCallback(async (traceId: string) => {
    try {
      const data = await invoke<SpanView[]>("obs_spans", { traceId });
      setSpans(data);
      setSelectedTrace(traceId);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  const loadMetrics = useCallback(async (name?: string) => {
    try {
      const data = await invoke<MetricView[]>("obs_metrics", { name: name || null, limit: 50 });
      setMetrics(data);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    loadDashboard();
    loadMetrics();
  }, [loadDashboard, loadMetrics]);

  if (loading && !summary) {
    return <section className="panel"><p className="muted">Загрузка дашборда…</p></section>;
  }

  if (error) {
    return <section className="panel"><p className="error">{error}</p></section>;
  }

  return (
    <section className="panel">
      <h2>Наблюдение (Observability)</h2>

      {/* Summary cards */}
      <div style={{ display: "flex", gap: "1rem", marginBottom: "1rem", flexWrap: "wrap" }}>
        <div className="panel" style={{ flex: "1", minWidth: "120px" }}>
          <strong>Трейсы</strong>
          <div style={{ fontSize: "2em" }}>{summary?.total_traces ?? 0}</div>
        </div>
        <div className="panel" style={{ flex: "1", minWidth: "120px" }}>
          <strong>Спаны</strong>
          <div style={{ fontSize: "2em" }}>{summary?.total_spans ?? 0}</div>
        </div>
        <div className="panel" style={{ flex: "1", minWidth: "120px" }}>
          <strong>Ср. задержка</strong>
          <div style={{ fontSize: "2em" }}>
            {summary?.avg_latency_ms != null ? `${summary.avg_latency_ms.toFixed(0)} мс` : "—"}
          </div>
        </div>
        <div className="panel" style={{ flex: "1", minWidth: "120px" }}>
          <strong>Токены</strong>
          <div style={{ fontSize: "2em" }}>{summary?.total_tokens ?? 0}</div>
        </div>
      </div>

      {/* Traces list */}
      <div style={{ display: "flex", gap: "1rem", flexWrap: "wrap" }}>
        <div style={{ flex: "1", minWidth: "300px" }}>
          <h3>Трейсы</h3>
          <div style={{ maxHeight: "300px", overflow: "auto" }}>
            {(summary?.traces ?? []).map((t) => (
              <button
                key={t.trace_id}
                type="button"
                className={selectedTrace === t.trace_id ? "tab active" : "tab"}
                onClick={() => loadSpans(t.trace_id)}
                style={{ display: "block", width: "100%", textAlign: "left", marginBottom: "0.25rem" }}
              >
                <strong>{t.name}</strong>
                <div style={{ fontSize: "0.8em", opacity: 0.7 }}>{t.trace_id}</div>
              </button>
            ))}
            {(summary?.traces ?? []).length === 0 && (
              <p className="muted">Нет трейсов. Запустите диалог или навык, чтобы данные появились.</p>
            )}
          </div>
        </div>

        <div style={{ flex: "2", minWidth: "400px" }}>
          <h3>Спаны{selectedTrace ? ` — ${selectedTrace}` : ""}</h3>
          <div style={{ maxHeight: "300px", overflow: "auto" }}>
            {spans.map((s) => (
              <div key={s.span_id} className="panel" style={{ marginBottom: "0.5rem" }}>
                <strong>{s.name}</strong>
                <div style={{ fontSize: "0.8em" }}>
                  {s.parent_span_id && <span>parent: {s.parent_span_id} </span>}
                  {s.input && Object.keys(s.input).length > 0 && (
                    <div>input: {JSON.stringify(s.input)}</div>
                  )}
                  {s.output && Object.keys(s.output).length > 0 && (
                    <div>output: {JSON.stringify(s.output)}</div>
                  )}
                </div>
              </div>
            ))}
            {spans.length === 0 && selectedTrace && (
              <p className="muted">Нет спанов для этого трейса.</p>
            )}
            {!selectedTrace && (
              <p className="muted">Выберите трейс слева, чтобы увидеть спаны.</p>
            )}
          </div>
        </div>
      </div>

      {/* Metrics */}
      <div style={{ marginTop: "1rem" }}>
        <h3>Метрики</h3>
        <div style={{ maxHeight: "200px", overflow: "auto" }}>
          {metrics.map((m, i) => (
            <div key={i} style={{ display: "flex", gap: "0.5rem", padding: "0.25rem 0" }}>
              <span style={{ minWidth: "150px" }}>{m.name}</span>
              <span style={{ minWidth: "80px", textAlign: "right" }}>
                {m.value.toFixed(2)} {m.unit || ""}
              </span>
              <span style={{ fontSize: "0.8em", opacity: 0.7 }}>{m.timestamp}</span>
            </div>
          ))}
          {metrics.length === 0 && (
            <p className="muted">Нет метрик. Метрики появляются при работе голоса, навыков и агента.</p>
          )}
        </div>
      </div>

      <button
        type="button"
        onClick={() => { loadDashboard(); loadMetrics(); }}
        style={{ marginTop: "1rem" }}
      >
        Обновить
      </button>
    </section>
  );
}