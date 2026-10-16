// ArkHa — локальный ИИ-секретарь для Windows.
//
// S0: каркас. Модули объявлены, но пусты. Содержимое появляется
// по этапам S1–S13 согласно docs/03-дорожная-карта.md.

pub mod agent;
pub mod agent_commands;
pub mod audit;
pub mod capabilities;
pub mod capabilities_commands;
pub mod chat;
pub mod downloader;
pub mod engine;
pub mod memory;
pub mod model_manager;
pub mod onboarding;
pub mod onboarding_commands;
pub mod permissions;
pub mod permissions_commands;
pub mod profile;
pub mod profile_commands;
pub mod resources;
pub mod skills;
pub mod state;
pub mod tools;
pub mod voice;

// S1: профиль. S2: модель и чат. S3: возможности и разрешения.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Плагин открытия ссылок здесь стоял и был мёртвым: он
    // регистрировался, но интерфейс его ни разу не вызывал, то есть
    // функции «открыть ссылку в браузере» в приложении не было, а
    // вес крейта оставался. Убран вместе с зависимостями.
    let app = tauri::Builder::default()
        .manage(model_manager::SidecarState::new())
        .manage(chat::ModelState::new())
        .manage(agent_commands::SettingsState::new())
        .manage(voice::Голос::новый())
        .invoke_handler(tauri::generate_handler![
            profile_commands::get_profile,
            profile_commands::set_profile_tier,
            profile_commands::set_context,
            resources::память_сейчас,
            voice::voice_status,
            voice::voice_enable,
            voice::voice_disable,
            voice::voice_say,
            voice::voice_hear,
            chat::start_model,
            chat::stop_model,
            chat::model_status,
            chat::model_log,
            chat::chat,
            agent_commands::agent_chat,
            agent_commands::agent_tools,
            agent_commands::stop_agent,
            agent_commands::agent_decide,
            agent_commands::agent_reload_skills,
            agent_commands::agent_set_executor,
            agent_commands::agent_set_approval,
            chat::stop_generation,
            chat::preflight_model,
            chat::repo_files,
            chat::search_models,
            chat::download_hosts,
            chat::download_model,
            chat::cancel_download,
            engine::engine_status,
            engine::engine_download_cuda,
            capabilities_commands::capabilities_report,
            capabilities_commands::capabilities_for_model,
            permissions_commands::permission_settings,
            onboarding_commands::onboarding_start,
            onboarding_commands::onboarding_current,
            onboarding_commands::onboarding_say,
            onboarding_commands::onboarding_confirm,
            onboarding_commands::onboarding_set_tools,
            onboarding_commands::onboarding_folders,
            onboarding_commands::onboarding_set_paths,
            onboarding_commands::onboarding_set_whole_disk,
            onboarding_commands::onboarding_set_modes,
            onboarding_commands::onboarding_preview,
            onboarding_commands::onboarding_apply,
        ]);

    app.run(tauri::generate_context!())
        .expect("не удалось запустить приложение");
}