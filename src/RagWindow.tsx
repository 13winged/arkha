import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

interface DocumentView {
  id: number;
  title: string;
  source: string;
  chunk_index: number;
  content: string;
}

interface SearchHitView {
  id: number;
  title: string;
  source: string;
  content: string;
  rank: number;
}

interface CorpusStatsView {
  total_documents: number;
  total_chunks: number;
  unique_sources: number;
}

export default function RagWindow() {
  const [stats, setStats] = useState<CorpusStatsView | null>(null);
  const [documents, setDocuments] = useState<DocumentView[]>([]);
  const [searchQuery, setSearchQuery] = useState("");
  const [searchResults, setSearchResults] = useState<SearchHitView[]>([]);
  const [textContent, setTextContent] = useState("");
  const [textTitle, setTextTitle] = useState("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);

  const loadStats = useCallback(async () => {
    try {
      const data = await invoke<CorpusStatsView>("rag_stats");
      setStats(data);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  const loadDocuments = useCallback(async () => {
    try {
      const data = await invoke<DocumentView[]>("rag_list");
      setDocuments(data);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    loadStats();
    loadDocuments();
  }, [loadStats, loadDocuments]);

  const [fileInput, setFileInput] = useState<HTMLInputElement | null>(null);

  const handleFileChange = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    setLoading(true);
    setMessage(null);
    setError(null);
    try {
      // Read file content as text
      const text = await file.text();
      const count = await invoke<number>("rag_add_text", {
        title: file.name,
        text: text,
        source: file.name,
      });
      setMessage(`Добавлено: ${count} фрагментов из ${file.name}`);
      await loadStats();
      await loadDocuments();
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
      // Reset input so same file can be selected again
      if (e.target) e.target.value = "";
    }
  };

  const addFile = () => {
    fileInput?.click();
  };

  const addText = async () => {
    if (!textTitle.trim() || !textContent.trim()) {
      setError("Укажите название и текст");
      return;
    }
    setLoading(true);
    setMessage(null);
    setError(null);
    try {
      const count = await invoke<number>("rag_add_text", {
        title: textTitle,
        text: textContent,
        source: "вручную",
      });
      setMessage(`Добавлено: ${count} фрагментов`);
      setTextTitle("");
      setTextContent("");
      await loadStats();
      await loadDocuments();
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  const search = async () => {
    if (!searchQuery.trim()) return;
    setLoading(true);
    setError(null);
    try {
      const results = await invoke<SearchHitView[]>("rag_search", { query: searchQuery, limit: 10 });
      setSearchResults(results);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  const removeDoc = async (source: string) => {
    setLoading(true);
    setError(null);
    try {
      const count = await invoke<number>("rag_remove", { source });
      setMessage(`Удалено: ${count} фрагментов`);
      await loadStats();
      await loadDocuments();
      setSearchResults([]);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  return (
    <div style={{ padding: "1rem", fontFamily: "inherit", color: "inherit", background: "inherit", minHeight: "100vh" }}>
      <h1 style={{ marginBottom: "0.5rem" }}>ArkHa — RAG (дообучение корпусом)</h1>
      <p style={{ opacity: 0.7, marginBottom: "1rem", fontSize: "0.9em" }}>
        Добавьте свои документы, и секретарь будет использовать их при ответах.
        Это «дообучение» без изменения весов модели — секретарь ищет релевантные
        фрагменты в ваших документах и добавляет их в контекст.
      </p>

      {error && <p style={{ color: "#c00", marginBottom: "0.5rem" }}>{error}</p>}
      {message && <p style={{ color: "#0a0", marginBottom: "0.5rem" }}>{message}</p>}

      {/* Stats */}
      <div style={{ display: "flex", gap: "1rem", marginBottom: "1rem", flexWrap: "wrap" }}>
        <div style={{ padding: "0.5rem", border: "1px solid #ccc", borderRadius: "4px", minWidth: "100px" }}>
          <strong>Документов</strong>
          <div style={{ fontSize: "1.5em" }}>{stats?.total_documents ?? 0}</div>
        </div>
        <div style={{ padding: "0.5rem", border: "1px solid #ccc", borderRadius: "4px", minWidth: "100px" }}>
          <strong>Фрагментов</strong>
          <div style={{ fontSize: "1.5em" }}>{stats?.total_chunks ?? 0}</div>
        </div>
        <div style={{ padding: "0.5rem", border: "1px solid #ccc", borderRadius: "4px", minWidth: "100px" }}>
          <strong>Источников</strong>
          <div style={{ fontSize: "1.5em" }}>{stats?.unique_sources ?? 0}</div>
        </div>
      </div>

      {/* Add document */}
      <div style={{ marginBottom: "1rem" }}>
        <h3>Добавить документ</h3>
        <div style={{ display: "flex", gap: "0.5rem", marginBottom: "0.5rem" }}>
          <button onClick={addFile} disabled={loading}>
            Выбрать файл
          </button>
          <input
            type="file"
            accept=".txt,.md,.csv,.json,.log,.html,.xml"
            ref={(ref) => setFileInput(ref)}
            onChange={handleFileChange}
            style={{ display: "none" }}
          />
        </div>
        <div>
          <input
            type="text"
            placeholder="Название документа"
            value={textTitle}
            onChange={(e) => setTextTitle(e.target.value)}
            style={{ width: "100%", marginBottom: "0.25rem", padding: "0.25rem" }}
          />
          <textarea
            placeholder="Вставьте текст документа..."
            value={textContent}
            onChange={(e) => setTextContent(e.target.value)}
            rows={4}
            style={{ width: "100%", marginBottom: "0.25rem", padding: "0.25rem" }}
          />
          <button onClick={addText} disabled={loading || !textTitle.trim() || !textContent.trim()}>
            Добавить текст
          </button>
        </div>
      </div>

      {/* Search */}
      <div style={{ marginBottom: "1rem" }}>
        <h3>Поиск по корпусу</h3>
        <div style={{ display: "flex", gap: "0.5rem", marginBottom: "0.5rem" }}>
          <input
            type="text"
            placeholder="Поисковый запрос"
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && search()}
            style={{ flex: "1", padding: "0.25rem" }}
          />
          <button onClick={search} disabled={loading || !searchQuery.trim()}>
            Искать
          </button>
        </div>
        <div style={{ maxHeight: "200px", overflow: "auto" }}>
          {searchResults.map((r) => (
            <div key={r.id} style={{ padding: "0.5rem", borderBottom: "1px solid #eee", marginBottom: "0.25rem" }}>
              <strong>{r.title}</strong>
              <div style={{ fontSize: "0.8em", opacity: 0.7 }}>{r.source}</div>
              <div style={{ fontSize: "0.9em", marginTop: "0.25rem" }}>
                {r.content.length > 200 ? r.content.slice(0, 200) + "..." : r.content}
              </div>
            </div>
          ))}
          {searchResults.length === 0 && searchQuery && (
            <p style={{ opacity: 0.6 }}>Ничего не найдено.</p>
          )}
        </div>
      </div>

      {/* Document list */}
      <div>
        <h3>Документы в корпусе</h3>
        <div style={{ maxHeight: "200px", overflow: "auto" }}>
          {documents.length === 0 ? (
            <p style={{ opacity: 0.6 }}>Корпус пуст. Добавьте документы выше.</p>
          ) : (
            documents.map((d) => (
              <div key={d.id} style={{ display: "flex", justifyContent: "space-between", padding: "0.25rem 0", borderBottom: "1px solid #eee" }}>
                <div>
                  <strong>{d.title}</strong>
                  <span style={{ fontSize: "0.8em", opacity: 0.7, marginLeft: "0.5rem" }}>
                    {d.source} (фрагмент {d.chunk_index})
                  </span>
                </div>
                <button
                  onClick={() => removeDoc(d.source)}
                  disabled={loading}
                  style={{ color: "#c00", border: "none", background: "transparent", cursor: "pointer" }}
                >
                  Удалить
                </button>
              </div>
            ))
          )}
        </div>
      </div>
    </div>
  );
}