import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

interface ModeView {
  id: string;
  title: string;
  description: string;
  color: string;
  active: boolean;
  allows_side_effects: boolean;
}

interface AgentSelectView {
  current_mode: string;
  modes: ModeView[];
}

export default function AgentSelect() {
  const [data, setData] = useState<AgentSelectView | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const result = await invoke<AgentSelectView>("agent_mode_list");
      setData(result);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const setMode = async (mode: string) => {
    setLoading(true);
    setError(null);
    try {
      const result = await invoke<AgentSelectView>("agent_mode_set", { mode });
      setData(result);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  const colorMap: Record<string, string> = {
    gold: "#d4a830",
    blue: "#3b82f6",
    green: "#22c55e",
  };

  return (
    <div style={{ padding: "1.5rem", fontFamily: "inherit", color: "inherit", background: "inherit", minHeight: "100vh" }}>
      <h1 style={{ marginBottom: "0.5rem" }}>Выбор режима агента</h1>
      <p style={{ opacity: 0.7, marginBottom: "1.5rem", fontSize: "0.9em" }}>
        Режим определяет, какие инструменты доступны секретарю. Выберите режим в зависимости
        от задачи: планирование, выполнение или рассуждение.
      </p>

      {error && <p style={{ color: "#c00", marginBottom: "0.5rem" }}>{error}</p>}

      <div style={{ display: "flex", flexDirection: "column", gap: "0.75rem" }}>
        {(data?.modes ?? []).map((m) => (
          <button
            key={m.id}
            type="button"
            onClick={() => setMode(m.id)}
            disabled={loading || m.active}
            style={{
              padding: "1rem",
              border: m.active ? `2px solid ${colorMap[m.color] || "#888"}` : "1px solid #ccc",
              borderRadius: "8px",
              background: m.active ? `${colorMap[m.color] || "#888"}22` : "transparent",
              cursor: m.active ? "default" : "pointer",
              textAlign: "left",
              color: "inherit",
            }}
          >
            <div style={{ display: "flex", alignItems: "center", gap: "0.5rem", marginBottom: "0.25rem" }}>
              <span
                style={{
                  display: "inline-block",
                  width: "10px",
                  height: "10px",
                  borderRadius: "50%",
                  background: colorMap[m.color] || "#888",
                }}
              />
              <strong>{m.title}</strong>
              {m.active && <span style={{ fontSize: "0.8em", opacity: 0.6 }}>(текущий)</span>}
            </div>
            <div style={{ fontSize: "0.85em", opacity: 0.8, marginBottom: "0.25rem" }}>
              {m.description}
            </div>
            <div style={{ fontSize: "0.75em", opacity: 0.6 }}>
              {m.allows_side_effects ? "Разрешает действия (запись, запуск, управление)" : "Только чтение и анализ"}
            </div>
          </button>
        ))}
      </div>

      {data && (
        <div style={{ marginTop: "1.5rem", padding: "0.75rem", border: "1px solid #ccc", borderRadius: "8px", fontSize: "0.85em" }}>
          <strong>Текущий режим:</strong> {data.modes.find((m) => m.active)?.title ?? "—"}
          <br />
          <span style={{ opacity: 0.7 }}>
            {data.modes.find((m) => m.active)?.allows_side_effects
              ? "Агент может выполнять действия (запись, запуск программ, управление окнами)"
              : "Агент может только читать и анализировать"}
          </span>
        </div>
      )}
    </div>
  );
}