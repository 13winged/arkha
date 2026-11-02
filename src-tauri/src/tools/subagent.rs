// Субагенты: запуск отдельного агента для подзадачи.
//
// Паттерн из OpenCode: субагент работает в отдельном контексте,
// имеет свой набор инструментов и возвращает результат родительскому агенту.
//
// Ограничения:
//   - Субагент не может запускать другие субагенты (глубина 1)
//   - Субагент работает в том же режиме, что и родитель
//   - Результат возвращается как текст для модели

use serde_json::{json, Value};

use super::{Tool, ToolError, ToolOutput, ToolSpec};
use crate::permissions::Settings;

/// Запуск субагента для подзадачи.
pub struct RunSubagent;

impl Tool for RunSubagent {
    fn spec(&self, _s: &Settings) -> ToolSpec {
        ToolSpec {
            id: "subagent.run".into(),
            capability: "subagent.run".into(),
            name: "Запустить субагента".into(),
            description: "Запускает отдельного агента для выполнения подзадачи. \
                          Субагент работает в отдельном контексте и возвращает результат. \
                          Подходит для параллельных задач: поиск, анализ, синтез."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "task": {
                        "type": "string",
                        "description": "описание подзадачи для субагента"
                    },
                    "context": {
                        "type": "string",
                        "description": "дополнительный контекст для субагента (опционально)"
                    }
                },
                "required": ["task"]
            }),
            side_effecting: true,
        }
    }

    fn call(&self, args: &Value, _s: &Settings) -> Result<ToolOutput, ToolError> {
        let Some(task) = args.get("task").and_then(|v| v.as_str()) else {
            return Err(ToolError::BadArgs("нет поля task".into()));
        };

        if task.is_empty() {
            return Err(ToolError::BadArgs("подзадача пустая".into()));
        }

        let context = args.get("context").and_then(|v| v.as_str()).unwrap_or("");

        // Субагент пока возвращает заглушку: полная реализация требует
        // отдельного цикла агента с собственным контекстом.
        // Архитектура готова: capability, инструмент, режим.
        let результат = format!(
            "Субагент получил задачу: {}\n\
             Контекст: {}\n\
             (Реализация цикла субагента — следующий этап)",
            task, context
        );

        Ok(ToolOutput::read(
            результат,
            format!("субагент получил задачу: {}", task),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_required() {
        let e = RunSubagent
            .call(&serde_json::json!({}), &Settings::default())
            .unwrap_err();
        assert!(matches!(e, ToolError::BadArgs(_)), "{e:?}");
    }

    #[test]
    fn empty_task_rejected() {
        let e = RunSubagent
            .call(&serde_json::json!({"task": ""}), &Settings::default())
            .unwrap_err();
        assert!(matches!(e, ToolError::BadArgs(_)), "{e:?}");
    }
}