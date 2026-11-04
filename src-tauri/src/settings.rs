//! Persistent application settings stored in `%APPDATA%\ArkHa\data\secretary.db`.

use crate::profile::config_dir;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const SETTINGS_KEY: &str = "app";
const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VoiceSettings {
    pub enabled: bool,
    pub auto_load: bool,
    pub tts_enabled: bool,
    pub language: String,
    pub voice: Option<String>,
    pub speed: f32,
}

impl Default for VoiceSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            auto_load: false,
            tts_enabled: false,
            language: "ru".to_string(),
            voice: None,
            speed: 1.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppSettings {
    pub schema_version: u32,
    pub language: String,
    pub font_size: u32,
    pub active_model: Option<String>,
    pub active_runtime: Option<String>,
    pub voice: VoiceSettings,
    pub enabled_skills: Vec<String>,
    pub enabled_tools: Vec<String>,
    pub agent_mode: String,
    pub fallback_policy: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            language: "ru".to_string(),
            font_size: 14,
            active_model: None,
            active_runtime: None,
            voice: VoiceSettings::default(),
            enabled_skills: Vec::new(),
            enabled_tools: Vec::new(),
            agent_mode: "build".to_string(),
            fallback_policy: "cuda-vulkan-cpu".to_string(),
        }
    }
}

fn database_path() -> PathBuf {
    config_dir().join("data").join("secretary.db")
}

fn open() -> Result<Connection, String> {
    let path = database_path();
    let parent = path
        .parent()
        .ok_or_else(|| "не удалось определить папку настроек".to_string())?;
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let connection = Connection::open(path).map_err(|error| error.to_string())?;
    connection
        .execute_batch(include_str!("settings_schema.sql"))
        .map_err(|error| error.to_string())?;
    Ok(connection)
}

fn read_optional(connection: &Connection) -> Result<Option<AppSettings>, String> {
    let raw: Option<String> = connection
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            params![SETTINGS_KEY],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;

    raw.map(|value| serde_json::from_str(&value).map_err(|error| error.to_string()))
        .transpose()
}

fn normalize(mut settings: AppSettings) -> AppSettings {
    settings.schema_version = SCHEMA_VERSION;
    if settings.language != "ru" && settings.language != "en" {
        settings.language = "ru".to_string();
    }
    if ![13, 14, 15, 16, 18].contains(&settings.font_size) {
        settings.font_size = 14;
    }
    if !["plan", "think", "build"].contains(&settings.agent_mode.as_str()) {
        settings.agent_mode = "build".to_string();
    }
    if settings.voice.language != "ru" && settings.voice.language != "en" {
        settings.voice.language = settings.language.clone();
    }
    if !(0.5..=2.0).contains(&settings.voice.speed) {
        settings.voice.speed = 1.0;
    }
    settings.enabled_skills.sort();
    settings.enabled_skills.dedup();
    settings.enabled_tools.sort();
    settings.enabled_tools.dedup();
    settings
}

fn write(connection: &Connection, settings: &AppSettings) -> Result<(), String> {
    let value = serde_json::to_string(settings).map_err(|error| error.to_string())?;
    connection
        .execute(
            "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, ?2, datetime('now'))
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = datetime('now')",
            params![SETTINGS_KEY, value],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn settings_get() -> Result<AppSettings, String> {
    let connection = open()?;
    if let Some(settings) = read_optional(&connection)? {
        return Ok(normalize(settings));
    }
    let settings = AppSettings::default();
    write(&connection, &settings)?;
    Ok(settings)
}

#[tauri::command]
pub fn settings_save(settings: AppSettings) -> Result<AppSettings, String> {
    let connection = open()?;
    let settings = normalize(settings);
    write(&connection, &settings)?;
    Ok(settings)
}

#[tauri::command]
pub fn settings_migrate_legacy(
    language: Option<String>,
    font_size: Option<u32>,
) -> Result<AppSettings, String> {
    let connection = open()?;
    if let Some(settings) = read_optional(&connection)? {
        return Ok(normalize(settings));
    }

    let mut settings = AppSettings::default();
    if let Some(language) = language {
        settings.language = language;
        settings.voice.language = settings.language.clone();
    }
    if let Some(font_size) = font_size {
        settings.font_size = font_size;
    }
    let settings = normalize(settings);
    write(&connection, &settings)?;
    Ok(settings)
}

#[tauri::command]
pub fn settings_reset() -> Result<AppSettings, String> {
    let connection = open()?;
    let settings = AppSettings::default();
    write(&connection, &settings)?;
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_stable() {
        let settings = AppSettings::default();
        assert_eq!(settings.language, "ru");
        assert_eq!(settings.font_size, 14);
        assert_eq!(settings.agent_mode, "build");
    }

    #[test]
    fn normalize_rejects_invalid_values() {
        let settings = normalize(AppSettings {
            language: "xx".into(),
            font_size: 99,
            agent_mode: "unknown".into(),
            voice: VoiceSettings {
                language: "xx".into(),
                speed: 5.0,
                ..VoiceSettings::default()
            },
            ..AppSettings::default()
        });
        assert_eq!(settings.language, "ru");
        assert_eq!(settings.font_size, 14);
        assert_eq!(settings.agent_mode, "build");
        assert_eq!(settings.voice.speed, 1.0);
    }

    #[test]
    fn normalize_deduplicates_lists() {
        let settings = normalize(AppSettings {
            enabled_skills: vec!["web".into(), "files".into(), "web".into()],
            enabled_tools: vec!["fs.read".into(), "fs.read".into()],
            ..AppSettings::default()
        });
        assert_eq!(settings.enabled_skills, vec!["files", "web"]);
        assert_eq!(settings.enabled_tools, vec!["fs.read"]);
    }
}
