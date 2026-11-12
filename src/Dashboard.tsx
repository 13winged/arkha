import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import { t } from "./i18n";

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

interface MemoryView {
  ramUsedMb: number;
  ramTotalMb: number;
  vramUsedMb: number | null;
  vramTotalMb: number | null;
  vramНеизвестна: boolean;
}

interface DashboardSummary {
  total_traces: number;
  total_spans: number;
  avg_latency_ms: number | null;
  total_tokens: number;
  traces: TraceView[];
}

interface Sample {
  cpu: number;
  ram: number;
  vram: number | null;
}

const HISTORY = 60; // две минуты при опросе раз в две секунды
const SVG_W = 600;
const SVG_H = 150;

export default function Dashboard() {
  const [summary, setSummary] = useState<DashboardSummary | null>(null);
  const [spans, setSpans] = useState<SpanView[]>([]);
  const [metrics, setMetrics] = useState<MetricView[]>([]);
  const [budget, setBudget] = useState<TokenBudget | null>(null);
  const [cpu, setCpu] = useState<CpuLoad | null>(null);
  const [memory, setMemory] = useState<MemoryView | null>(null);
  const [history, setHistory] = useState<Sample[]>([]);
  const [selectedTrace, setSelectedTrace] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const timer = useRef<number | null>(null);

  const sample = useCallback(async () => {
    try {
      const [load, mem] = await Promise.all([
        invoke<CpuLoad>("cpu_load_сейчас"),
        invoke<MemoryView>("память_сейчас"),
      ]);
      setCpu(load);
      setMemory(mem);
      const ram = mem.ramTotalMb > 0 ? (mem.ramUsedMb * 100) / mem.ramTotalMb : 0;
      const vram =
        !mem.vramНеизвестна && mem.vramTotalMb
          ? ((mem.vramUsedMb ?? 0) * 100) / mem.vramTotalMb
          : null;
      setHistory((current) => {
        const next = [...current, { cpu: load.loadPercent, ram, vram }];
        return next.length > HISTORY ? next.slice(next.length - HISTORY) : next;
      });
    } catch {
      // Нет замера — график просто не двигается. Ошибка здесь
      // повторялась бы каждые две секунды и забила бы панель.
    }
  }, []);

  const loadDashboard = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setSummary(await invoke<DashboardSummary>("obs_dashboard", { limit: 20 }));
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  const loadMetrics = useCallback(async () => {
    try {
      setMetrics(await invoke<MetricView[]>("obs_metrics", { name: null, limit: 50 }));
    } catch {
      setMetrics([]);
    }
  }, []);

  const loadBudget = useCallback(async () => {
    try {
      setBudget(await invoke<TokenBudget>("obs_budget", { limit: 100000.0 }));
    } catch {
      setBudget(null);
    }
  }, []);

  const loadSpans = useCallback(async (traceId: string) => {
    try {
      setSpans(await invoke<SpanView[]>("obs_spans", { traceId }));
      setSelectedTrace(traceId);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    void sample();
    void loadDashboard();
    void loadMetrics();
    void loadBudget();
    timer.current = window.setInterval(() => void sample(), 2000);
    return () => {
      if (timer.current !== null) window.clearInterval(timer.current);
    };
  }, [sample, loadDashboard, loadMetrics, loadBudget]);

  const refresh = () => {
    void sample();
    void loadDashboard();
    void loadMetrics();
    void loadBudget();
  };

  const ramPercent = memory && memory.ramTotalMb > 0
    ? Math.min(100, (memory.ramUsedMb * 100) / memory.ramTotalMb)
    : 0;
  const vramPercent = memory && !memory.vramНеизвестна && memory.vramTotalMb
    ? Math.min(100, ((memory.vramUsedMb ?? 0) * 100) / memory.vramTotalMb)
    : null;

  return (
    <section className="panel dashboard">
      <div className="dash-head">
        <h2>{t("Наблюдение")}</h2>
        <button type="button" className="chip" onClick={refresh} disabled={loading}>
          {t("Обновить")}
        </button>
      </div>

      {error && <p className="error">{error}</p>}

      {/* Бюджет: кольцо на всю ширину панели. */}
      <div className="dash-budget">
        <BudgetRing budget={budget} />
        <div className="dash-budget-facts">
          <div>
            <span>{t("Лимит")}</span>
            <b>{fmt(budget?.limit ?? 0)}</b>
          </div>
          <div>
            <span>{t("Потрачено")}</span>
            <b>{fmt(budget?.spent ?? 0)}</b>
          </div>
          <div>
            <span>{t("Остаток")}</span>
            <b>{fmt(Math.max(0, (budget?.limit ?? 0) - (budget?.spent ?? 0)))}</b>
          </div>
        </div>
      </div>

      {/* Текущие значения. */}
      <div className="dash-cards">
        <div className="dash-card">
          <span className="dash-card-label">CPU</span>
          <b className="dash-card-value">{cpu ? `${cpu.loadPercent}%` : "—"}</b>
          <span className="dash-card-note">{cpu ? `${cpu.logicalCores} ${t("ядер")}` : t("нет данных")}</span>
        </div>
        <div className="dash-card">
          <span className="dash-card-label">{t("ОЗУ")}</span>
          <b className="dash-card-value">{memory ? `${Math.round(ramPercent)}%` : "—"}</b>
          <span className="dash-card-note">
            {memory ? `${gb(memory.ramUsedMb)} / ${gb(memory.ramTotalMb)}` : t("нет данных")}
          </span>
        </div>
        <div className="dash-card">
          <span className="dash-card-label">VRAM</span>
          <b className="dash-card-value">{vramPercent !== null ? `${Math.round(vramPercent)}%` : "—"}</b>
          <span className="dash-card-note">
            {memory && memory.vramTotalMb
              ? `${gb(memory.vramUsedMb ?? 0)} / ${gb(memory.vramTotalMb)}`
              : t("нет данных")}
          </span>
        </div>
      </div>

      <div className="dash-graph">
        <div className="dash-graph-head">
          <h3>{t("Загрузка за последние две минуты")}</h3>
          <div className="dash-legend">
            <span className="dash-key cpu">CPU</span>
            <span className="dash-key ram">{t("ОЗУ")}</span>
            <span className="dash-key vram">VRAM</span>
          </div>
        </div>
        <UsageGraph history={history} />
      </div>

      <div className="dash-cards">
        <div className="dash-card small">
          <span className="dash-card-label">{t("Трейсы")}</span>
          <b className="dash-card-value">{summary?.total_traces ?? 0}</b>
        </div>
        <div className="dash-card small">
          <span className="dash-card-label">{t("Спаны")}</span>
          <b className="dash-card-value">{summary?.total_spans ?? 0}</b>
        </div>
        <div className="dash-card small">
          <span className="dash-card-label">{t("Ср. задержка")}</span>
          <b className="dash-card-value">
            {summary?.avg_latency_ms != null ? `${summary.avg_latency_ms.toFixed(0)} ${t("мс")}` : "—"}
          </b>
        </div>
        <div className="dash-card small">
          <span className="dash-card-label">{t("Токены")}</span>
          <b className="dash-card-value">{fmt(summary?.total_tokens ?? 0)}</b>
        </div>
      </div>

      <div className="dash-split">
        <div>
          <h3>{t("Трейсы")}</h3>
          <div className="dash-scroll">
            {(summary?.traces ?? []).map((trace) => (
              <button
                key={trace.trace_id}
                type="button"
                className={selectedTrace === trace.trace_id ? "dash-trace active" : "dash-trace"}
                onClick={() => void loadSpans(trace.trace_id)}
              >
                <strong>{trace.name}</strong>
                <span>{trace.trace_id}</span>
              </button>
            ))}
            {(summary?.traces ?? []).length === 0 && (
              <p className="muted">{t("Нет трейсов. Запустите диалог или навык, чтобы данные появились.")}</p>
            )}
          </div>
        </div>
        <div>
          <h3>{t("Спаны")}{selectedTrace ? ` — ${selectedTrace}` : ""}</h3>
          <div className="dash-scroll">
            {spans.map((span) => (
              <div key={span.span_id} className="dash-span">
                <strong>{span.name}</strong>
                {span.parent_span_id && <span className="muted">parent: {span.parent_span_id}</span>}
                {span.input && Object.keys(span.input).length > 0 && (
                  <code>{JSON.stringify(span.input)}</code>
                )}
                {span.output && Object.keys(span.output).length > 0 && (
                  <code>{JSON.stringify(span.output)}</code>
                )}
              </div>
            ))}
            {spans.length === 0 && (
              <p className="muted">
                {selectedTrace ? t("Нет спанов для этого трейса.") : t("Выберите трейс, чтобы увидеть спаны.")}
              </p>
            )}
          </div>
        </div>
      </div>

      <h3>{t("Метрики")}</h3>
      <div className="dash-scroll">
        {metrics.map((metric, index) => (
          <div key={index} className="dash-metric">
            <span>{metric.name}</span>
            <b>{metric.value.toFixed(2)} {metric.unit ?? ""}</b>
            <span className="muted">{metric.timestamp}</span>
          </div>
        ))}
        {metrics.length === 0 && <p className="muted">{t("Метрик пока нет.")}</p>}
      </div>
    </section>
  );
}

function BudgetRing({ budget }: { budget: TokenBudget | null }) {
  const percent = Math.max(0, Math.min(100, budget?.percent ?? 0));
  const radius = 54;
  const circumference = 2 * Math.PI * radius;
  const offset = circumference * (1 - percent / 100);
  const color = budget?.status === "critical"
    ? "var(--cp-neon-red)"
    : budget?.status === "warning"
      ? "var(--gold-lit)"
      : "var(--cp-neon-green)";

  return (
    <div className="dash-ring" role="img" aria-label={`${t("бюджет токенов")}: ${percent.toFixed(1)}%`}>
      <svg viewBox="0 0 120 120">
        <circle cx="60" cy="60" r={radius} className="dash-ring-track" />
        <circle
          cx="60"
          cy="60"
          r={radius}
          className="dash-ring-fill"
          stroke={color}
          strokeDasharray={circumference}
          strokeDashoffset={offset}
          transform="rotate(-90 60 60)"
        />
        <text x="60" y="58" textAnchor="middle" className="dash-ring-value">
          {percent.toFixed(1)}%
        </text>
        <text x="60" y="76" textAnchor="middle" className="dash-ring-caption">
          {t("бюджет")}
        </text>
      </svg>
    </div>
  );
}

function UsageGraph({ history }: { history: Sample[] }) {
  if (history.length < 2) {
    return <p className="muted">{t("Собираем данные…")}</p>;
  }
  const x = (index: number) => (index / (HISTORY - 1)) * SVG_W;
  const y = (value: number) => SVG_H - (Math.max(0, Math.min(100, value)) / 100) * SVG_H;
  const line = (pick: (sample: Sample) => number | null) =>
    history
      .map((sample, index) => {
        const value = pick(sample);
        return value === null ? null : `${x(index)},${y(value)}`;
      })
      .filter((point): point is string => point !== null)
      .join(" ");

  return (
    <svg className="dash-graph-svg" viewBox={`0 0 ${SVG_W} ${SVG_H}`} preserveAspectRatio="none">
      {[25, 50, 75].map((value) => (
        <line key={value} x1={0} y1={y(value)} x2={SVG_W} y2={y(value)} className="dash-grid" />
      ))}
      <polyline points={line((s) => s.cpu)} className="dash-line cpu" />
      <polyline points={line((s) => s.ram)} className="dash-line ram" />
      <polyline points={line((s) => s.vram)} className="dash-line vram" />
    </svg>
  );
}

function fmt(value: number): string {
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)}k`;
  return value.toFixed(0);
}

function gb(megabytes: number): string {
  return `${(megabytes / 1024).toFixed(1)} ГиБ`;
}
