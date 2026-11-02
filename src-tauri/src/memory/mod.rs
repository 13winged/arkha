// S10-S11  Память и наблюдение.
//
// Модули:
//   - sqlite    — подключение и схема (основа для всего)
//   - graph     — граф памяти: сущности, отношения, факты (локальный аналог Graphiti)
//   - observability — трейсы, спаны, метрики (локальный аналог Opik)
//   - embeddings — BGE-M3, ONNX (заглушка, S10)
//   - fts        — FTS5, поиск по точным словам (заглушка, S10)
//   - vec        — sqlite-vec, векторный поиск (заглушка, S10)

pub mod graph;
pub mod observability;
pub mod sqlite;

pub mod embeddings;
pub mod fts;
pub mod vec;