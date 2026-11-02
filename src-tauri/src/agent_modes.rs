// Режимы агента: plan, build, think.
//
// Каждый режим определяет, какие инструменты доступны агенту.
// Это аналог OpenCode agent modes, адаптированный для ArkHa.
//
//   plan  — только чтение и планирование, без побочных эффектов
//   build — полный доступ к инструментам (с учётом разрешений)
//   think — рассуждение и анализ, без выполнения действий

use serde::{Deserialize, Serialize};
use std::sync::Mutex;

use crate::permissions::Settings;
use crate::tools::{Tool, ToolSpec, Registry};

/// Режим работы агента.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentMode {
    /// План: только чтение, анализ и планирование. Без побочных эффектов.
    Plan,
    /// Сборка: полный доступ к инструментам (с учётом разрешений пользователя).
    Build,
    /// Рассуждение: анализ и рассуждение, без выполнения действий.
    Think,
}

impl AgentMode {
    /// Название режима для интерфейса.
    pub fn title(self) -> &'static str {
        match self {
            AgentMode::Plan => "План",
            AgentMode::Build => "Сборка",
            AgentMode::Think => "Рассуждение",
        }
    }

    /// Описание режима для интерфейса.
    pub fn description(self) -> &'static str {
        match self {
            AgentMode::Plan => "Агент только читает, анализирует и планирует. Никаких изменений в системе.",
            AgentMode::Build => "Агент выполняет действия: читает, пишет, запускает программы, управляет окнами.",
            AgentMode::Think => "Агент рассуждает и анализирует, но не выполняет никаких действий.",
        }
    }

    /// Цвет для UI (gold, blue, green).
    pub fn color(self) -> &'static str {
        match self {
            AgentMode::Plan => "gold",
            AgentMode::Build => "blue",
            AgentMode::Think => "green",
        }
    }

    /// Какие возможности (capabilities) разрешены в этом режиме.
    /// Возвращает список префиксов capability, которые разрешены.
    pub fn allowed_capability_prefixes(self) -> &'static [&'static str] {
        match self {
            AgentMode::Plan => &[
                "files.read",
                "windows.read",
                "web.fetch",
                "web.search",
                "memory.recall",
                "memory.search",
                "voice.stt",
                "voice.tts",
                "chat.local",
                "skill.execute",
            ],
            AgentMode::Build => &[
                "files.read",
                "files.write",
                "windows.read",
                "windows.control",
                "apps.launch",
                "web.fetch",
                "web.search",
                "skill.execute",
                "subagent.run",
                "memory.recall",
                "memory.search",
                "memory.write",
                "voice.stt",
                "voice.tts",
                "chat.local",
                "docs.office",
                "vision.read",
                "browser.dom",
                "workflow.run",
            ],
            AgentMode::Think => &[
                "files.read",
                "windows.read",
                "web.fetch",
                "web.search",
                "memory.recall",
                "memory.search",
                "chat.local",
                "skill.execute",
            ],
        }
    }

    /// Проверяет, разрешена ли возможность в этом режиме.
    pub fn allows_capability(&self, capability: &str) -> bool {
        self.allowed_capability_prefixes()
            .iter()
            .any(|prefix| capability == *prefix || capability.starts_with(prefix))
    }

    /// Фильтрует список инструментов по режиму.
    pub fn filter_tools(&self, tools: Vec<Box<dyn Tool>>) -> Vec<Box<dyn Tool>> {
        tools
            .into_iter()
            .filter(|t| {
                let spec = t.spec(&Settings::default());
                self.allows_capability(&spec.capability)
            })
            .collect()
    }

    /// Все режимы для UI.
    pub fn all() -> &'static [AgentMode] {
        &[AgentMode::Plan, AgentMode::Build, AgentMode::Think]
    }

    /// Разбор из строки (для десериализации).
    pub fn from_str(s: &str) -> Option<AgentMode> {
        match s.to_lowercase().as_str() {
            "plan" => Some(AgentMode::Plan),
            "build" => Some(AgentMode::Build),
            "think" => Some(AgentMode::Think),
            _ => None,
        }
    }

    /// Строковое представление.
    pub fn as_str(self) -> &'static str {
        match self {
            AgentMode::Plan => "plan",
            AgentMode::Build => "build",
            AgentMode::Think => "think",
        }
    }
}

/// Текущий режим агента.
static CURRENT_MODE: Mutex<AgentMode> = Mutex::new(AgentMode::Build);

/// Получить текущий режим.
pub fn current_mode() -> AgentMode {
    *CURRENT_MODE.lock().unwrap()
}

/// Установить режим агента.
pub fn set_mode(mode: AgentMode) {
    let mut current = CURRENT_MODE.lock().unwrap();
    *current = mode;
}

/// Возвращает true, если режим разрешает выполнение действий (side effects).
pub fn allows_side_effects() -> bool {
    current_mode() == AgentMode::Build
}

/// Возвращает true, если режим разрешает чтение.
pub fn allows_read() -> bool {
    true // Все режимы разрешают чтение
}

/// Возвращает true, если режим разрешает запись/изменение.
pub fn allows_write() -> bool {
    current_mode() == AgentMode::Build
}

/// Возвращает true, если режим разрешает запуск программ.
pub fn allows_launch() -> bool {
    current_mode() == AgentMode::Build
}

/// Возвращает true, если режим разрешает управление окнами.
pub fn allows_control() -> bool {
    current_mode() == AgentMode::Build
}

/// Возвращает true, если режим разрешает выполнение навыков.
pub fn allows_skills() -> bool {
    current_mode() == AgentMode::Build
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_from_str() {
        assert_eq!(AgentMode::from_str("plan"), Some(AgentMode::Plan));
        assert_eq!(AgentMode::from_str("build"), Some(AgentMode::Build));
        assert_eq!(AgentMode::from_str("think"), Some(AgentMode::Think));
        assert_eq!(AgentMode::from_str("unknown"), None);
    }

    #[test]
    fn plan_mode_allows_read() {
        assert!(AgentMode::Plan.allows_capability("files.read"));
        assert!(AgentMode::Plan.allows_capability("windows.read"));
        assert!(AgentMode::Plan.allows_capability("web.fetch"));
        assert!(AgentMode::Plan.allows_capability("memory.recall"));
    }

    #[test]
    fn plan_mode_blocks_write() {
        assert!(!AgentMode::Plan.allows_capability("files.write"));
        assert!(!AgentMode::Plan.allows_capability("apps.launch"));
        assert!(!AgentMode::Plan.allows_capability("windows.control"));
        // skill.execute теперь разрешён в Plan mode (навыки включены)
    }

    #[test]
    fn build_mode_allows_everything() {
        assert!(AgentMode::Build.allows_capability("files.read"));
        assert!(AgentMode::Build.allows_capability("files.write"));
        assert!(AgentMode::Build.allows_capability("apps.launch"));
        assert!(AgentMode::Build.allows_capability("windows.control"));
        assert!(AgentMode::Build.allows_capability("skill.execute"));
        assert!(AgentMode::Build.allows_capability("web.search"));
    }

    #[test]
    fn think_mode_allows_read_only() {
        assert!(AgentMode::Think.allows_capability("files.read"));
        assert!(AgentMode::Think.allows_capability("web.fetch"));
        assert!(AgentMode::Think.allows_capability("memory.recall"));
        assert!(!AgentMode::Think.allows_capability("files.write"));
        assert!(!AgentMode::Think.allows_capability("apps.launch"));
        assert!(!AgentMode::Think.allows_capability("windows.control"));
    }

    #[test]
    fn side_effects_only_in_build() {
        // Simulate mode switching
        set_mode(AgentMode::Plan);
        assert!(!allows_side_effects());
        assert!(!allows_write());
        assert!(!allows_launch());
        assert!(!allows_control());
        assert!(!allows_skills());

        set_mode(AgentMode::Build);
        assert!(allows_side_effects());
        assert!(allows_write());
        assert!(allows_launch());
        assert!(allows_control());
        assert!(allows_skills());

        set_mode(AgentMode::Think);
        assert!(!allows_side_effects());
        assert!(!allows_write());
        assert!(!allows_launch());
        assert!(!allows_control());
        assert!(!allows_skills());
    }
}