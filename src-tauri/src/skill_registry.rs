//! Реестр builtin/local skills для UI и агентного контекста.

use crate::settings;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillRegistryItem {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub source: String,
    pub path: String,
    pub tools: Vec<String>,
    pub permissions: Vec<String>,
    pub enabled: bool,
    pub manifest: bool,
}

#[derive(Debug, Deserialize)]
struct SkillManifest {
    id: Option<String>,
    name: Option<String>,
    version: Option<String>,
    description: String,
    #[serde(default)]
    tools: Vec<String>,
    #[serde(default)]
    permissions: Vec<String>,
    #[serde(default = "default_entrypoint")]
    entrypoint: String,
}

fn default_entrypoint() -> String {
    "SKILL.md".to_string()
}

fn roots() -> Vec<PathBuf> {
    let mut result = vec![crate::skills::папка_данных(), crate::skills::папка()];
    if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
        if let Some(root) = Path::new(&manifest).parent() {
            result.push(root.join("skills"));
        }
    }
    result.push(PathBuf::from("skills"));
    result.sort();
    result.dedup();
    result
}

fn persisted_enabled() -> Result<Vec<String>, String> {
    Ok(settings::settings_get()?.enabled_skills)
}

fn is_enabled(id: &str, enabled: &[String]) -> bool {
    enabled.is_empty() || enabled.iter().any(|value| value == id)
}

fn legacy_item(path: &Path, enabled: &[String]) -> Option<SkillRegistryItem> {
    let skill = crate::skills::разобрать_файл(path).ok()?;
    let id = skill.name.clone();
    Some(SkillRegistryItem {
        id: id.clone(),
        name: skill.name,
        version: skill.version,
        description: skill.description,
        source: "legacy".to_string(),
        path: path.to_string_lossy().to_string(),
        tools: skill.tools.clone(),
        permissions: skill.tools,
        enabled: is_enabled(&id, enabled),
        manifest: false,
    })
}

fn manifest_item(dir: &Path, enabled: &[String]) -> Option<SkillRegistryItem> {
    let manifest_path = dir.join("skill.json");
    let manifest: SkillManifest = serde_json::from_str(&fs::read_to_string(&manifest_path).ok()?).ok()?;
    let entrypoint = dir.join(&manifest.entrypoint);
    if !entrypoint.is_file() {
        return None;
    }
    let id = manifest
        .id
        .or(manifest.name.clone())
        .or_else(|| dir.file_name().and_then(|name| name.to_str()).map(str::to_string))?;
    Some(SkillRegistryItem {
        name: manifest.name.unwrap_or_else(|| id.clone()),
        version: manifest.version.unwrap_or_else(|| "1.0.0".to_string()),
        description: manifest.description,
        source: "manifest".to_string(),
        path: dir.to_string_lossy().to_string(),
        tools: manifest.tools,
        permissions: manifest.permissions,
        enabled: is_enabled(&id, enabled),
        manifest: true,
        id,
    })
}

fn discover(enabled: &[String]) -> Vec<SkillRegistryItem> {
    let mut items = Vec::new();
    for root in roots() {
        let Ok(entries) = fs::read_dir(root) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|ext| ext.to_str()) == Some("md") {
                if let Some(item) = legacy_item(&path, enabled) { items.push(item); }
            } else if path.is_dir() && path.join("skill.json").is_file() {
                if let Some(item) = manifest_item(&path, enabled) { items.push(item); }
            }
        }
    }
    items.sort_by(|left, right| left.id.cmp(&right.id).then(left.path.cmp(&right.path)));
    let mut seen = BTreeSet::new();
    items.retain(|item| seen.insert(item.id.clone()));
    items
}

#[tauri::command]
pub fn skills_list() -> Result<Vec<SkillRegistryItem>, String> {
    Ok(discover(&persisted_enabled()?))
}

fn set_enabled(id: String, enabled: bool) -> Result<Vec<SkillRegistryItem>, String> {
    let available = discover(&[]);
    if !available.iter().any(|item| item.id == id) {
        return Err(format!("skill не найден: {id}"));
    }
    let mut settings = settings::settings_get()?;
    if settings.enabled_skills.is_empty() {
        settings.enabled_skills = available.iter().map(|item| item.id.clone()).collect();
    }
    if enabled {
        if !settings.enabled_skills.contains(&id) { settings.enabled_skills.push(id); }
    } else {
        settings.enabled_skills.retain(|value| value != &id);
    }
    settings::settings_save(settings)?;
    let _ = crate::tools::перечитать_навыки();
    Ok(discover(&persisted_enabled()?))
}

#[tauri::command]
pub fn skills_enable(id: String) -> Result<Vec<SkillRegistryItem>, String> {
    set_enabled(id, true)
}

#[tauri::command]
pub fn skills_disable(id: String) -> Result<Vec<SkillRegistryItem>, String> {
    set_enabled(id, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_persisted_selection_means_all_enabled() {
        assert!(is_enabled("files", &[]));
        assert!(is_enabled("web", &["web".to_string()]));
        assert!(!is_enabled("files", &["web".to_string()]));
    }
}
