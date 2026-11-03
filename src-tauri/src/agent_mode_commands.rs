// Команды Tauri для режимов агента.
//
// Режимы: plan, build, think. Каждый определяет, какие инструменты
// доступны агенту. Отдельное окно для выбора режима.

use serde::{Deserialize, Serialize};

use crate::agent_modes;

#[derive(Serialize)]
pub struct ModeView {
    pub id: String,
    pub title: String,
    pub description: String,
    pub color: String,
    pub active: bool,
    pub allows_side_effects: bool,
}

#[derive(Serialize)]
pub struct AgentSelectView {
    pub current_mode: String,
    pub modes: Vec<ModeView>,
}

/// Открыть окно выбора агента.
#[tauri::command]
pub async fn agent_select_open_window(app: tauri::AppHandle) -> Result<(), String> {
    use tauri::Manager;

    if let Some(window) = app.get_webview_window("agent_select") {
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }

    let _window = tauri::WebviewWindowBuilder::new(
        &app,
        "agent_select",
        tauri::WebviewUrl::App("index.html".into()),
    )
    .title("ArkHa — Выбор режима агента")
    .inner_size(480.0, 520.0)
    .min_inner_size(400.0, 400.0)
    .build()
    .map_err(|e| format!("не удалось открыть окно выбора агента: {e}"))?;

    Ok(())
}

/// Получить текущий режим и список доступных.
#[tauri::command]
pub fn agent_mode_list() -> AgentSelectView {
    let current = agent_modes::current_mode();
    let modes: Vec<ModeView> = agent_modes::AgentMode::all()
        .iter()
        .map(|m| ModeView {
            id: m.as_str().to_string(),
            title: m.title().to_string(),
            description: m.description().to_string(),
            color: m.color().to_string(),
            active: *m == current,
            allows_side_effects: *m == agent_modes::AgentMode::Build,
        })
        .collect();

    AgentSelectView {
        current_mode: current.as_str().to_string(),
        modes,
    }
}

/// Установить режим агента.
#[tauri::command]
pub fn agent_mode_set(mode: String) -> Result<AgentSelectView, String> {
    let mode = agent_modes::AgentMode::from_str(&mode)
        .ok_or_else(|| format!("неизвестный режим: {mode}"))?;

    agent_modes::set_mode(mode);
    tracing::info!("режим агента: {}", mode.as_str());

    Ok(agent_mode_list())
}

/// Получить текущий режим.
#[tauri::command]
pub fn agent_mode_current() -> String {
    agent_modes::current_mode().as_str().to_string()
}

/// Проверить, разрешено ли действие в текущем режиме.
#[tauri::command]
pub fn agent_mode_allows(capability: String) -> bool {
    agent_modes::current_mode().allows_capability(&capability)
}
