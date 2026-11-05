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
    let messages = add_rag_context(compacted.messages);
    PreparedContext {
        messages,
        compression_note: compacted.причина,
    }
}

fn add_rag_context(mut messages: Vec<Message>) -> Vec<Message> {
    if messages.iter().any(|message| {
        message.role == "system" && message.content.starts_with("## Локальный RAG-контекст")
    }) {
        return messages;
    }
    let Some(query) = messages
        .iter()
        .rev()
        .find(|message| message.role == "user" && !message.content.trim().is_empty())
        .map(|message| message.content.trim().to_string())
    else {
        return messages;
    };
    if query.chars().count() < 3 {
        return messages;
    }

    let Ok(roots) = crate::user_data::current_roots() else { return messages };
    let database = roots.memory_db;
    let Ok(connection) = crate::memory::sqlite::open(&database) else {
        return messages;
    };
    let Ok(results) = crate::memory::fts::search(&connection, &query, 3) else {
        return messages;
    };
    if results.is_empty() {
        return messages;
    }

    let mut context = String::from("## Локальный RAG-контекст\n\n");
    for result in results {
        let content: String = result.content.chars().take(1800).collect();
        context.push_str(&format!("- {}: {}\n", result.title, content));
    }
    messages.insert(0, Message {
        role: "system".to_string(),
        content: context,
        tool_calls: None,
        tool_call_id: None,
        name: None,
    });
    messages
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
