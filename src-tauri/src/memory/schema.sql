-- Схема локальной памяти Arkha.
-- Alles в одном файле SQLite. Никакого облака.

-- ============================================================
-- Граф памяти (аналог Graphiti, но локально)
-- ============================================================

-- Сущности (узлы графа): люди, организации, понятия, файлы, программы
CREATE TABLE IF NOT EXISTS memory_entities (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    name         TEXT NOT NULL,
    entity_type  TEXT NOT NULL DEFAULT 'concept',  -- person, org, concept, file, program, place
    properties   TEXT NOT NULL DEFAULT '{}',       -- JSON с дополнительными атрибутами
    confidence   REAL NOT NULL DEFAULT 1.0,        -- уверенность 0.0..1.0
    created_at   TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at   TEXT NOT NULL DEFAULT (datetime('now')),
    source       TEXT                              -- откуда learnt: 'user', 'observation', 'inference'
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_memory_entities_name_type
    ON memory_entities(name, entity_type);

-- Отношения (рёбра графа): кто с кем, как связано
CREATE TABLE IF NOT EXISTS memory_relationships (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    source_entity_id INTEGER NOT NULL,
    target_entity_id INTEGER NOT NULL,
    relationship     TEXT NOT NULL,                  -- works_at, knows, likes, uses, located_in, etc.
    properties       TEXT NOT NULL DEFAULT '{}',     -- JSON с дополнительными атрибутами
    confidence       REAL NOT NULL DEFAULT 1.0,
    created_at       TEXT NOT NULL DEFAULT (datetime('now')),
    source           TEXT,
    FOREIGN KEY (source_entity_id) REFERENCES memory_entities(id) ON DELETE CASCADE,
    FOREIGN KEY (target_entity_id) REFERENCES memory_entities(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_memory_relationships_source
    ON memory_relationships(source_entity_id);
CREATE INDEX IF NOT EXISTS idx_memory_relationships_target
    ON memory_relationships(target_entity_id);
CREATE INDEX IF NOT EXISTS idx_memory_relationships_type
    ON memory_relationships(relationship);

-- Факты (утверждения о сущности)
CREATE TABLE IF NOT EXISTS memory_facts (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    entity_id    INTEGER NOT NULL,
    fact_text    TEXT NOT NULL,
    source       TEXT,                              -- откуда learnt
    confidence   REAL NOT NULL DEFAULT 1.0,
    created_at   TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (entity_id) REFERENCES memory_entities(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_memory_facts_entity
    ON memory_facts(entity_id);

-- ============================================================
-- Наблюдение (аналог Opik, но локально)
-- ============================================================

-- Трейсы (высокоуровневые операции: диалог, вызов навыка, распознавание речи)
CREATE TABLE IF NOT EXISTS obs_traces (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    trace_id     TEXT NOT NULL UNIQUE,   -- внешний идентификатор трейса
    name         TEXT NOT NULL,          -- например 'voice_dictation', 'skill.run'
    start_time   TEXT NOT NULL DEFAULT (datetime('now')),
    end_time     TEXT,
    duration_ms  INTEGER,
    metadata     TEXT NOT NULL DEFAULT '{}',  -- JSON
    status       TEXT NOT NULL DEFAULT 'running',  -- running, completed, error
    error        TEXT
);

CREATE INDEX IF NOT EXISTS idx_obs_traces_trace_id ON obs_traces(trace_id);
CREATE INDEX IF NOT EXISTS idx_obs_traces_start_time ON obs_traces(start_time);

-- Спаны (вложенные операции внутри трейса: STT, VAD, LLM, TTS)
CREATE TABLE IF NOT EXISTS obs_spans (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    trace_id     TEXT NOT NULL,
    span_id      TEXT NOT NULL UNIQUE,
    parent_span_id TEXT,
    name         TEXT NOT NULL,          -- например 'silero_vad', 'llama_stt', 'skill.obzor-papki'
    start_time   TEXT NOT NULL DEFAULT (datetime('now')),
    end_time     TEXT,
    duration_ms  INTEGER,
    input        TEXT NOT NULL DEFAULT '{}',   -- JSON
    output       TEXT NOT NULL DEFAULT '{}',   -- JSON
    metadata     TEXT NOT NULL DEFAULT '{}',   -- JSON
    status       TEXT NOT NULL DEFAULT 'running',
    error        TEXT,
    FOREIGN KEY (trace_id) REFERENCES obs_traces(trace_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_obs_spans_trace_id ON obs_spans(trace_id);
CREATE INDEX IF NOT EXISTS idx_obs_spans_span_id ON obs_spans(span_id);
CREATE INDEX IF NOT EXISTS idx_obs_spans_parent ON obs_spans(parent_span_id);
CREATE INDEX IF NOT EXISTS idx_obs_spans_name ON obs_spans(name);

-- Метрики (измеряемые значения: задержка, использование токенов, стоимость)
CREATE TABLE IF NOT EXISTS obs_metrics (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    trace_id     TEXT,
    span_id      TEXT,
    name         TEXT NOT NULL,          -- например 'stt_latency_ms', 'tokens_used', 'tts_rtf'
    value        REAL NOT NULL,
    unit         TEXT,                   -- ms, tokens, rtf, etc.
    timestamp    TEXT NOT NULL DEFAULT (datetime('now')),
    tags         TEXT NOT NULL DEFAULT '{}',  -- JSON
    FOREIGN KEY (trace_id) REFERENCES obs_traces(trace_id) ON DELETE CASCADE,
    FOREIGN KEY (span_id) REFERENCES obs_spans(span_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_obs_metrics_trace_id ON obs_metrics(trace_id);
CREATE INDEX IF NOT EXISTS idx_obs_metrics_span_id ON obs_metrics(span_id);
CREATE INDEX IF NOT EXISTS idx_obs_metrics_name ON obs_metrics(name);
CREATE INDEX IF NOT EXISTS idx_obs_metrics_timestamp ON obs_metrics(timestamp);

-- ============================================================
-- Полнотекстовый поиск (FTS5, пользовательский RAG)
-- ============================================================

CREATE VIRTUAL TABLE IF NOT EXISTS memory_fts USING fts5(
    title, content, source, tokenize='unicode61'
);

CREATE TABLE IF NOT EXISTS memory_documents (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL,
    source TEXT NOT NULL,
    content TEXT NOT NULL,
    chunk_index INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_memory_documents_source ON memory_documents(source);