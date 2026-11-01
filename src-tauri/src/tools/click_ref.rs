// Инструмент S6+: нажать на элемент по ref.
//
// Паттерн из agent-desktop: после snapshot агент получает ref'ы
// (@e1, @e2, ...) и нажимает на конкретный ref. Это делает действия
// воспроизводимыми: ref привязан к конкретному элементу в конкретном
// окне, без координат и без угадывания.

use serde_json::{json, Value};

use crate::permissions::Settings;
use super::accessibility::{self, Accessibility};

use super::{Tool, ToolError, ToolOutput, ToolSpec};

/// Нажать на элемент по ref.
///
/// ref получается из инструмента `accessibility.snapshot`. Пример:
/// ref "@e3" — третья кнопка в дереве доступности окна.
pub struct ClickRef;

impl Tool for ClickRef {
    fn spec(&self, _s: &Settings) -> ToolSpec {
        ToolSpec {
            id: "accessibility.click_ref".into(),
            capability: "windows.control".into(),
            name: "Нажать на элемент по ref".into(),
            description: "Нажать на элемент доступности по ref (например, @e3). \
                          ref получается из accessibility.snapshot. Клик по \
                          coordonataм не поддерживается — только по nommé элементу."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "ref": {
                        "type": "string",
                        "description": "ref элемента, например @e3"
                    }
                },
                "required": ["ref"]
            }),
            side_effecting: true,
        }
    }

    fn call(&self, args: &Value, _s: &Settings) -> Result<ToolOutput, ToolError> {
        let Some(ref_id) = args.get("ref").and_then(|v| v.as_str()) else {
            return Err(ToolError::BadArgs("нет поля ref".into()));
        };

        if ref_id.is_empty() {
            return Err(ToolError::BadArgs("ref пустой".into()));
        }

        let adapter = accessibility::create();
        adapter.click_ref(ref_id).map_err(|e| match e {
            accessibility::AccessibilityError::ElementNotFound(_) => ToolError::Unknown(e.to_string()),
            accessibility::AccessibilityError::Platform(_) => ToolError::Refused(e.to_string()),
            accessibility::AccessibilityError::WindowNotFound(_) => ToolError::Unknown(e.to_string()),
            accessibility::AccessibilityError::TreeUnavailable(_) => ToolError::Refused(e.to_string()),
            accessibility::AccessibilityError::Ambiguous(_) => ToolError::Ambiguous(e.to_string()),
        })?;

        Ok(ToolOutput::changed(
            format!("нажато «{ref_id}»"),
            format!("нажат элемент {ref_id}"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ref_is_required() {
        let e = ClickRef.call(&serde_json::json!({}), &Settings::default()).unwrap_err();
        assert!(matches!(e, ToolError::BadArgs(_)), "{e:?}");
    }

    #[test]
    fn empty_ref_is_rejected() {
        let e = ClickRef.call(&serde_json::json!({"ref": ""}), &Settings::default()).unwrap_err();
        assert!(matches!(e, ToolError::BadArgs(_)), "{e:?}");
    }
}