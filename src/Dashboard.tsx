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

interface TimelinePoint {
  label: string;
  tokens: number;
  traces: number;
}

interface TokenBudget {
  spent: number;
  limit: number;
  percent: number;
  status: string;
}

interface CpuLoad {
  loadPercent: number;
  logicalCores: number;
}

interface DashboardSummary {
  total_traces: number;
  total_spans: number;
  avg_latency_ms: number | null;
  total_tokens: number;
  traces: TraceView[];
}

// CPU приходит из `resources::cpu_load_сейчас` (camelCase, как и память).

export default function Dashboard() {
  const [summary, setSummary] = useState<DashboardSummary | null>(null);
  const [spans, setSpans] = useState<SpanView[]>([]);
  const [metrics, setMetrics] = useState<MetricView[]>([]);
  const [timeline, setTimeline] = useState<TimelinePoint[]>([]);
  const [budget, setBudget] = useState<TokenBudget | null>(null);
  const [cpu, setCpu] = useState<CpuLoad | null>(null);
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

  const loadTimeline = useCallback(async () => {
    try {
      const data = await invoke<TimelinePoint[]>("obs_timeline", { hours: 24 });
      setTimeline(data);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  const loadBudget = useCallback(async () => {
    try {
      const data = await invoke<TokenBudget>("obs_budget", { limit: 100000.0 });
      setBudget(data);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  const loadCpu = useCallback(async () => {
    try {
      const data = await invoke<CpuLoad>("cpu_load_сейчас");
      setCpu(data);
    } catch {
      // Нет загрузки CPU — не повод прятать весь дашборд.
      setCpu(null);
    }
  }, []);

  useEffect(() => {
    loadDashboard();
    loadMetrics();
    loadTimeline();
    loadBudget();
    loadCpu();
  }, [loadDashboard, loadMetrics, loadTimeline, loadBudget, loadCpu]);

  const refresh = () => {
    loadDashboard();
    loadMetrics();
    loadTimeline();
    loadBudget();
    loadCpu();
  };

  if (loading && !summary) {
    return <section className="panel"><p className="muted">Загрузка дашборда…</p></section>;
  }

  if (error) {
    return <section className="panel"><p className="error">{error}</p></section>;
  }

  // Timeline chart data
  const maxTokens = Math.max(...timeline.map((t) => t.tokens), 1);
  const chartWidth = 600;
  const chartHeight = 120;
  const pointSpacing = timeline.length > 1 ? chartWidth / (timeline.length - 1) : 0;

  // Per-conversation breakdown
  const conversationMap = new Map<string, { tokens: number; traces: number; latency: number[] }>();
  for (const t of summary?.traces ?? []) {
    const name = t.name;
    if (!conversationMap.has(name)) {
      conversationMap.set(name, { tokens: 0, traces: 0, latency: [] });
    }
    const entry = conversationMap.get(name)!;
    entry.traces += 1;
  }
  // Add token data from spans
  for (const m of metrics) {
    if (m.name === "tokens_used") {
      // Try to match by trace_id (not available in MetricView, so just aggregate)
    }
  }

  const conversations = Array.from(conversationMap.entries()).map(([name, data]) => ({
    name,
    traces: data.traces,
    tokens: data.tokens,
  }));

  const budgetColor = budget
    ? budget.status === "critical"
      ? "#c00"
      : budget.status === "warning"
      ? "#d4a830"
      : "#0a0"
    : "#888";

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
        <div className="panel" style={{ flex: "1", minWidth: "120px" }}>
          <strong>CPU</strong>
          <div style={{ fontSize: "2em" }}>
            {cpu ? `${cpu.loadPercent}%` : "—"}
          </div>
          {cpu && <div style={{ fontSize: "0.8em", opacity: 0.7 }}>{cpu.logicalCores} ядер</div>}
        </div>
      </div>

      {/* Budget card */}
      {budget && (
        <div className="panel" style={{ marginBottom: "1rem" }}>
          <h3>Бюджет токенов</h3>
          <div style={{ display: "flex", alignItems: "center", gap: "1rem" }}>
            <div style={{ flex: "1" }}>
              <div style={{ display: "flex", justifyContent: "space-between", marginBottom: "0.25rem" }}>
                <span>Потрачено: {budget.spent.toFixed(0)}</span>
                <span>Лимит: {budget.limit.toFixed(0)}</span>
              </div>
              <div style={{ height: "8px", background: "#eee", borderRadius: "4px", overflow: "hidden" }}>
                <div
                  style={{
                    height: "100%",
                    width: `${Math.min(budget.percent, 100)}%`,
                    background: budgetColor,
                    transition: "width 0.3s",
                  }}
                />
              </div>
            </div>
            <div style={{ fontSize: "1.2em", fontWeight: "bold", color: budgetColor }}>
              {budget.percent.toFixed(1)}%
            </div>
          </div>
        </div>
      )}

      {/* Timeline chart */}
      {timeline.length > 0 && (
        <div className="panel" style={{ marginBottom: "1rem" }}>
          <h3>Использование токенов (24ч)</h3>
          <svg width={chartWidth} height={chartHeight} style={{ width: "100%", height: "auto" }}>
            {/* Grid lines */}
            <line x1={0} y1={chartHeight} x2={chartWidth} y2={chartHeight} stroke="#ccc" strokeWidth={1} />
            <line x1={0} y1={0} x2={0} y2={chartHeight} stroke="#ccc" strokeWidth={1} />
            {/* Line chart */}
            <polyline
              points={timeline
                .map((t, i) => `${i * pointSpacing},${chartHeight - (t.tokens / maxTokens) * (chartHeight - 10)}`)
                .join(" ")}
              fill="none"
              stroke="#3b82f6"
              strokeWidth={2}
            />
            {/* Points */}
            {timeline.map((t, i) => (
              <circle
                key={i}
                cx={i * pointSpacing}
                cy={chartHeight - (t.tokens / maxTokens) * (chartHeight - 10)}
                r={3}
                fill="#3b82f6"
              />
            ))}
            {/* Labels */}
            {timeline.map((t, i) => (
              <text
                key={`label-${i}`}
                x={i * pointSpacing}
                y={chartHeight + 12}
                textAnchor="middle"
                fontSize={8}
                fill="currentColor"
                opacity={0.6}
              >
                {t.label.split(" ")[1] || ""}
              </text>
            ))}
          </svg>
        </div>
      )}

      {/* Per-conversation breakdown */}
      {conversations.length > 0 && (
        <div className="panel" style={{ marginBottom: "1rem" }}>
          <h3>Разбивка по разговорам</h3>
          <div style={{ maxHeight: "200px", overflow: "auto" }}>
            <table style={{ width: "100%", borderCollapse: "collapse" }}>
              <thead>
                <tr>
                  <th style={{ textAlign: "left", padding: "0.25rem" }}>Название</th>
                  <th style={{ textAlign: "right", padding: "0.25rem" }}>Трейсы</th>
                  <th style={{ textAlign: "right", padding: "0.25rem" }}>Токены</th>
                </tr>
              </thead>
              <tbody>
                {conversations.map((c) => (
                  <tr key={c.name}>
                    <td style={{ padding: "0.25rem" }}>{c.name}</td>
                    <td style={{ textAlign: "right", padding: "0.25rem" }}>{c.traces}</td>
                    <td style={{ textAlign: "right", padding: "0.25rem" }}>{c.tokens}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      )}

      {/* Traces and spans */}
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
        onClick={refresh}
        style={{ marginTop: "1rem" }}
      >
        Обновить
      </button>
    </section>
  );
}