// ArkHa — локальный ИИ-секретарь для Windows.
//
// S0: каркас. Модули объявлены, но пусты. Содержимое появляется
// по этапам S1–S13 согласно docs/03-дорожная-карта.md.

pub mod agent;
pub mod auth;
pub mod attachments;
pub mod agent_commands;
pub mod agent_modes;
pub mod agent_mode_commands;
pub mod audit;
pub mod capabilities;
pub mod capabilities_commands;
pub mod chat;
pub mod documents;
pub mod downloader;
pub mod engine;
pub mod memory;
pub mod legacy_migration;
pub mod model_manager;
pub mod observability_commands;
pub mod onboarding;
pub mod rag_commands;
pub mod onboarding_commands;
pub mod permissions;
pub mod permissions_commands;
pub mod profile;
pub mod profile_commands;
pub mod resources;
pub mod runtime;
pub mod settings;
mod secure_storage;
pub mod skill_registry;
pub mod skills;
pub mod state;
pub mod tools;
pub mod voice;
pub mod user_data;
pub mod webview_orphans;

// S1: профиль. S2: модель и чат. S3: возможности и разрешения.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Трейсинг для диагностики задержек в цепочке запись → VAD → STT → LLM → TTS.
    // Пишет в stderr; уровень берется из переменной ARKHA_LOG (по умолчанию info).
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    tracing::info!("аркха: запуск");

    // Хвосты от прошлых запусков: обработчики сбоев WebView2, чьи окна уже
    // закрыты. Убираем до создания первого окна, иначе они копятся.
    webview_orphans::reap_orphans();
    webview_orphans::start_sweeper();

    // Плагин открытия ссылок здесь стоял и был мёртвым: он
    // регистрировался, но интерфейс его ни разу не вызывал, то есть
    // функции «открыть ссылку в браузере» в приложении не было, а
    // вес крейта оставался. Убран вместе с зависимостями.
    let app = tauri::Builder::default()
        .manage(auth::AuthState::default())
        .manage(model_manager::SidecarState::new())
        .manage(chat::ModelState::new())
        .manage(agent_commands::SettingsState::new())
        .manage(voice::Голос::новый())
        .invoke_handler(tauri::generate_handler![
            profile_commands::get_profile,
            auth::auth_register,
            auth::auth_login,
            auth::auth_guest,
            auth::auth_logout,
            auth::auth_session,
            auth::auth_remember_supported,
            auth::auth_profiles_list,
            auth::auth_profile_delete,
            legacy_migration::legacy_migration_preview,
            legacy_migration::legacy_migration_import,
            legacy_migration::legacy_migration_skip,
            profile_commands::set_profile_tier,
            profile_commands::set_context,
            resources::память_сейчас,
            resources::cpu_load_сейчас,
            resources::cpu_load_сейчас,
            voice::voice_status,
            voice::voice_enable,
            voice::voice_disable,
            voice::voice_say,
            voice::voice_hear,
            voice::voice_record_start,
            voice::voice_record_stop,
            voice::voice_hotkey_enable,
            voice::voice_hotkey_disable,
            voice::voice_hotkey_set,
            voice::voice_devices,
            voice::voice_audio_settings,
            voice::voice_audio_set,
            voice::voice_theme_settings,
            voice::voice_theme_set,
            settings::settings_get,
            settings::settings_save,
            settings::settings_reset,
            skill_registry::skills_list,
            skill_registry::skills_enable,
            skill_registry::skills_disable,
            skill_registry::skills_read,
            runtime::runtime_list,
            runtime::runtime_set_active,
            runtime::runtime_check,
            runtime::runtime_delete,
            runtime::runtime_reveal,
            runtime::runtime_install,
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
            agent_mode_commands::agent_select_open_window,
            agent_mode_commands::agent_mode_list,
            agent_mode_commands::agent_mode_set,
            agent_mode_commands::agent_mode_current,
            agent_mode_commands::agent_mode_allows,
            chat::stop_generation,
            chat::preflight_model,
            chat::repo_files,
            chat::models_list,
            chat::model_set_active,
            chat::model_delete,
            attachments::attachment_add,
            attachments::attachments_delete,
            chat::models_open_window,
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
            observability_commands::obs_dashboard,
            observability_commands::obs_spans,
            observability_commands::obs_metrics,
            observability_commands::obs_timeline,
            observability_commands::obs_budget,
            rag_commands::rag_open_window,
            rag_commands::rag_add_file,
            rag_commands::rag_add_text,
            rag_commands::rag_search,
            rag_commands::rag_remove,
            rag_commands::rag_stats,
            rag_commands::rag_list,
            rag_commands::rag_ask,
        ]);

    app.run(tauri::generate_context!())
        .expect("не удалось запустить приложение");
}
