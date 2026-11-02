// Инструмент S6+: снять дерево доступности окна.
//
// Паттерн из agent-desktop: агент сначала снимает дерево доступности,
// получает ref'ы (@e1, @e2, ...) и затем нажимает на конкретный ref.
// Это делает действия воспроизводимыми и безопасными.

use serde_json::{json, Value};

use crate::permissions::Settings;
use super::accessibility::{self, Role};

use super::{Tool, ToolError, ToolOutput, ToolSpec};

/// Снять дерево доступности окна.
pub struct SnapshotTool;

impl Tool for SnapshotTool {
    fn spec(&self, _s: &Settings) -> ToolSpec {
        ToolSpec {
            id: "accessibility.snapshot".into(),
            capability: "windows.read".into(),
            name: "Снять дерево доступности".into(),
            description: "Снять дерево доступности окна. Каждый интерактивный \
                          элемент получает ref (@e1, @e2, ...). Используйте ref \
                          в инструменте click_ref, чтобы нажать на элемент."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "title": {
                        "type": "string",
                        "description": "часть заголовка окна (опционально)"
                    }
                },
                "required": []
            }),
            side_effecting: false,
        }
    }

    fn call(&self, args: &Value, _s: &Settings) -> Result<ToolOutput, ToolError> {
        let title = args.get("title").and_then(|v| v.as_str());

        let target = match title {
            Some(t) if !t.is_empty() => {
                let windows_list = crate::tools::windows::окна();
                let found: Vec<_> = windows_list
                    .iter()
                    .filter(|w| w.заголовок.to_lowercase().contains(&t.to_lowercase()))
                    .collect();

                match found.len() {
                    0 => {
                        return Err(ToolError::Unknown(format!(
                            "видимого окна с «{t}» в названии нет"
                        )));
                    }
                    1 => accessibility::Target::Window(found[0].hwnd as i64),
                    _ => {
                        let list: Vec<String> = found
                            .iter()
                            .map(|w| format!("[{}] {}", w.hwnd, w.заголовок))
                            .collect();
                        return Err(ToolError::Ambiguous(format!(
                            "под «{t}» подходит {} окон: {}. Уточни, какое",
                            found.len(),
                            list.join("; ")
                        )));
                    }
                }
            }
            _ => accessibility::Target::Desktop,
        };

        let adapter = accessibility::create();
        let snapshot = adapter.snapshot(&target).map_err(|e| match e {
            accessibility::AccessibilityError::WindowNotFound(_) => ToolError::Unknown(e.to_string()),
            accessibility::AccessibilityError::TreeUnavailable(_) => ToolError::Refused(e.to_string()),
            accessibility::AccessibilityError::ElementNotFound(_) => ToolError::Unknown(e.to_string()),
            accessibility::AccessibilityError::Ambiguous(_) => ToolError::Ambiguous(e.to_string()),
            accessibility::AccessibilityError::Platform(_) => ToolError::Refused(e.to_string()),
        })?;

        // Формируем текст для модели: список ref'ов с описаниями
        let mut text = String::new();
        if !snapshot.refs.is_empty() {
            text.push_str("Дерево доступности окна. Элементы:\n");
            for entry in &snapshot.refs {
                let name = entry.name.as_deref().unwrap_or("(без названия)");
                text.push_str(&format!(
                    "  {} — {} «{}» (действия: {})\n",
                    entry.ref_id,
                    role_to_words(&entry.role),
                    name,
                    entry.available_actions.join(", ")
                ));
            }
        } else {
            text.push_str("Интерактивных элементов не найдено.\n");
        }

        Ok(ToolOutput::read(
            text,
            format!("снято дерево доступности, {} элементов", snapshot.refs.len()),
        ))
    }
}

/// Роль элемента на человеческом языке.
fn role_to_words(role: &Role) -> &'static str {
    match role {
        Role::Button => "кнопка",
        Role::CheckBox => "флажок",
        Role::ComboBox => "выпадающий список",
        Role::Edit => "поле ввода",
        Role::Hyperlink => "ссылка",
        Role::ListItem => "элемент списка",
        Role::MenuItem => "пункт меню",
        Role::RadioButton => "переключатель",
        Role::TabItem => "вкладка",
        Role::Text => "текст",
        Role::Window => "окно",
        Role::Other(_) => "элемент",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_to_words_known() {
        assert_eq!(role_to_words(&Role::Button), "кнопка");
        assert_eq!(role_to_words(&Role::Edit), "поле ввода");
        assert_eq!(role_to_words(&Role::Window), "окно");
    }
}