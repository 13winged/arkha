// Дерево доступности: кроссплатформенная абстракция.
//
// Паттерн из agent-desktop (vercel-labs): ядро определяет типы и
// трейт, а платформенные адаптеры реализуют его под каждую ОС:
// Windows — UI Automation, Linux — AT-SPI2, macOS — AXAPI.
//
// Контракт с агентом:
//   1. `snapshot` — снять дерево доступности окна, каждый элемент
//      получает ref (@e1, @e2, ...).
//   2. `click_ref` — нажать на элемент по ref, без координат.
//   3. `list_apps` / `list_windows` — перечислить приложения и окна.
//
// Клик только по nommé элементу: ни координат, ни угадывания.

#[cfg(target_os = "windows")]
pub mod windows;

#[cfg(target_os = "linux")]
pub mod linux;

use serde::{Deserialize, Serialize};

/// Тип элемента доступности.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Role {
    Button,
    CheckBox,
    ComboBox,
    Edit,
    Hyperlink,
    ListItem,
    MenuItem,
    RadioButton,
    TabItem,
    Text,
    Window,
    Other(String),
}

impl Role {
    pub fn is_interactive(&self) -> bool {
        matches!(
            self,
            Role::Button
                | Role::CheckBox
                | Role::ComboBox
                | Role::Hyperlink
                | Role::ListItem
                | Role::MenuItem
                | Role::RadioButton
                | Role::TabItem
                | Role::Edit
        )
    }

    pub fn takes_text(&self) -> bool {
        matches!(self, Role::Edit)
    }
}

/// Прямоугольник в экранных координатах.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn width(&self) -> i32 {
        self.right - self.left
    }

    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }

    pub fn center(&self) -> (i32, i32) {
        ((self.left + self.right) / 2, (self.top + self.bottom) / 2)
    }

    pub fn is_empty(&self) -> bool {
        self.width() <= 0 || self.height() <= 0
    }
}

/// Точка на экране.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

/// Состояние элемента.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ElementState {
    pub enabled: Option<bool>,
    pub visible: Option<bool>,
    pub offscreen: Option<bool>,
    pub focused: Option<bool>,
}

/// Идентичность элемента: имя, значение, описание.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeIdentity {
    pub name: Option<String>,
    pub value: Option<String>,
    pub description: Option<String>,
}

/// Презентация элемента: подсказки, состояния, доступные действия.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodePresentation {
    pub hint: Option<String>,
    pub states: Vec<String>,
    pub available_actions: Vec<String>,
}

/// Узел дерева доступности.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub ref_id: Option<String>,
    pub role: Role,
    pub identity: NodeIdentity,
    pub presentation: NodePresentation,
    pub bounds: Option<Rect>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub subtree_truncated: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Node>,
}

/// Снимок дерева доступности окна.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub refs: Vec<RefEntry>,
    pub nodes_observed: usize,
    pub complete: bool,
}

/// Запись ref: связь между ref_id и элементом.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefEntry {
    pub ref_id: String,
    pub role: Role,
    pub name: Option<String>,
    pub bounds: Option<Rect>,
    pub available_actions: Vec<String>,
}

/// Ошибка доступа к дереву доступности.
#[derive(Debug, Clone, PartialEq)]
pub enum AccessibilityError {
    WindowNotFound(String),
    TreeUnavailable(String),
    ElementNotFound(String),
    Ambiguous(String),
    Platform(String),
}

impl std::fmt::Display for AccessibilityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AccessibilityError::WindowNotFound(s) => write!(f, "окно не найдено: {s}"),
            AccessibilityError::TreeUnavailable(s) => write!(f, "дерево недоступно: {s}"),
            AccessibilityError::ElementNotFound(s) => write!(f, "элемент не найден: {s}"),
            AccessibilityError::Ambiguous(s) => write!(f, "несколько элементов: {s}"),
            AccessibilityError::Platform(s) => write!(f, "платформенная ошибка: {s}"),
        }
    }
}

impl std::error::Error for AccessibilityError {}

/// Трейт доступа к дереву доступности.
pub trait Accessibility: Send + Sync {
    fn snapshot(&self, target: &Target) -> Result<Snapshot, AccessibilityError>;
    fn click_ref(&self, ref_id: &str) -> Result<(), AccessibilityError>;
    fn list_apps(&self) -> Result<Vec<String>, AccessibilityError>;
    fn list_windows(&self) -> Result<Vec<String>, AccessibilityError>;
}

/// Цель для snapshot: окно по идентификатору.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    Window(i64),
    Desktop,
}

/// Создать адаптер для текущей платформы.
pub fn create() -> Box<dyn Accessibility> {
    #[cfg(target_os = "windows")]
    return windows::WindowsAccessibility::new();
    #[cfg(target_os = "linux")]
    return linux::LinuxAccessibility::new();
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    unimplemented!("доступность не реализована для этой платформы");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_center() {
        let r = Rect { left: 0, top: 0, right: 100, bottom: 50 };
        assert_eq!(r.center(), (50, 25));
    }

    #[test]
    fn rect_is_empty() {
        let r = Rect { left: 0, top: 0, right: 0, bottom: 0 };
        assert!(r.is_empty());
        let r = Rect { left: 0, top: 0, right: 10, bottom: 10 };
        assert!(!r.is_empty());
    }

    #[test]
    fn role_is_interactive() {
        assert!(Role::Button.is_interactive());
        assert!(Role::Edit.is_interactive());
        assert!(!Role::Text.is_interactive());
        assert!(!Role::Window.is_interactive());
    }
}