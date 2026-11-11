// Документы через MCP.
//
// Решение и числа — в docs/31-документы-через-mcp.md: четыре чужих CLI
// (WordCraft, PdfCraft, GridCraft, DeckCraft) читают и пишут docx, pdf,
// xlsx и pptx. Своего разбора Office-форматов не пишем.
//
// Здесь транспорт (`mcp`), установка (`install`) и инструменты
// (`tools`). Подключение к реестру — в `tools::built`.

pub mod install;
pub mod mcp;
pub mod tools;

pub use tools::{DocsInfo, DocsRead, DocsWrite};
