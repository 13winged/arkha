// Команды разрешений для интерфейса.
//
// Настройки читаются, но не пишутся. Запись — это отдельное решение
// с подтверждением, и делать её одной строкой значило бы обойти
// собственную осторожность: агент, который может и читать, и менять
// настройки разрешений, способен выдать себе права сам.

use serde::Serialize;

use crate::permissions::{ApprovalMode, Executor, Settings};

/// Разрешение, готовое к показать: значение, название и объяснение.
#[derive(Debug, Clone, Serialize)]
pub struct ModeView {
    pub value: String,
    pub title: String,
    pub explain: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SettingsView {
    pub executor: ModeView,
    pub approval_mode: ModeView,
    pub cloud: bool,
    pub deny_apps: Vec<String>,
    pub paths: Vec<String>,
    /// Какие возможности выключены вручную.
    pub disabled: Vec<String>,
    /// Предупреждение, если режим не тот, что безопаснее всего.
    pub warning: Option<String>,
}

fn executor_view(e: Executor) -> ModeView {
    ModeView {
        value: match e {
            Executor::DryRun => "dry-run",
            Executor::Live => "live",
        }
        .to_string(),
        title: e.title().to_string(),
        explain: match e {
            Executor::DryRun => "Показывать, что агент собирается сделать, и ничего не выполнять.",
            Executor::Live => "Выполнять действия по-настоящему.",
        }
        .to_string(),
    }
}

fn approval_view(m: ApprovalMode) -> ModeView {
    ModeView {
        value: match m {
            ApprovalMode::Risky => "actions",
            ApprovalMode::Confirm => "all",
            ApprovalMode::Auto => "never",
        }
        .to_string(),
        title: m.title().to_string(),
        explain: m.explain().to_string(),
    }
}

#[tauri::command]
pub fn permission_settings() -> Result<SettingsView, String> {
    let s = Settings::load()?;
    Ok(view(&s))
}

pub fn view(s: &Settings) -> SettingsView {
    // Предупреждение не оценочное суждение, а конкретное следствие
    // режима: пользователь должен понимать, что именно он разрешил.
    let warning = match s.executor {
        Executor::Live => Some(
            "Режим выполнения включён: агент будет менять файлы и запускать программы \
             без вашего участия. Подтверждения остаются, но без них он всё равно \
             может начать действовать."
                .to_string(),
        ),
        Executor::DryRun => None,
    };

    let mut disabled: Vec<String> = s
        .capabilities
        .iter()
        .filter(|(_, r)| !r.enabled)
        .map(|(id, _)| id.clone())
        .collect();
    disabled.sort();

    SettingsView {
        executor: executor_view(s.executor),
        approval_mode: approval_view(s.approval_mode),
        cloud: s.cloud,
        deny_apps: s.deny_apps.clone(),
        paths: s.paths.iter().map(|p| p.display().to_string()).collect(),
        disabled,
        warning,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn по_умолчанию_никаких_предупреждений() {
        let v = view(&Settings::default());
        assert!(v.warning.is_none(), "в безопасном режиме пугать нечем");
        assert_eq!(v.executor.value, "dry-run");
        assert_eq!(v.approval_mode.value, "actions");
    }

    /// Если пользователь включил выполнение, интерфейс обязан
    /// объяснить, что именно он разрешил. Молчаливый переключатель
    /// опаснее отсутствия переключателя.
    #[test]
    fn режим_выполнения_даёт_предупреждение() {
        let mut s = Settings::default();
        s.executor = Executor::Live;
        let v = view(&s);
        let w = v.warning.expect("режим выполнения обязан предупреждать");
        assert!(w.contains("подтверждения") || w.contains("Подтверждения"));
    }

    /// Название значения в файле нечитаемо, поэтому показываем своё.
    #[test]
    fn значения_переводятся_на_человеческий() {
        let v = view(&Settings::default());
        assert!(!v.approval_mode.title.is_empty());
        assert!(v.approval_mode.title.len() > "actions".len());
        for m in [ApprovalMode::Risky, ApprovalMode::Confirm, ApprovalMode::Auto] {
            let mv = approval_view(m);
            assert!(!mv.explain.is_empty(), "у режима должно быть объяснение");
        }
    }

    #[test]
    fn выключенные_возможности_видны_списком() {
        let mut s = Settings::default();
        s.capabilities.insert(
            "files.delete".into(),
            crate::permissions::CapabilityRule { enabled: false, mode: None },
        );
        s.capabilities.insert(
            "apps.launch".into(),
            crate::permissions::CapabilityRule { enabled: false, mode: None },
        );
        let v = view(&s);
        assert_eq!(v.disabled, vec!["apps.launch", "files.delete"]);
    }

    #[test]
    fn настоящие_настройки_читаются() {
        let s = Settings::load().expect("настройки должны читаться");
        let v = view(&s);
        assert!(!v.deny_apps.is_empty());
        assert!(v.deny_apps.contains(&"local.secretary.app".to_string()));
    }
}
