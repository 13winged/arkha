//! Подготовка контекста агента перед каждым обращением к модели.

use super::compaction;
use super::executor::Message;

#[derive(Debug, Clone)]
pub struct PreparedContext {
    pub messages: Vec<Message>,
    pub compression_note: String,
}

/// Ограничить историю перед model turn. Вызывается не только в начале
/// цикла: tool results могут разрастись уже после первого запроса.
pub fn prepare(messages: Vec<Message>, limit: usize) -> PreparedContext {
    let compacted = compaction::сжать(messages, limit);
    PreparedContext {
        messages: compacted.messages,
        compression_note: compacted.причина,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepares_context_without_changing_small_history() {
        let prepared = prepare(vec![Message::user("hello")], 1000);
        assert_eq!(prepared.messages.len(), 1);
        assert!(prepared.compression_note.is_empty());
    }
}
