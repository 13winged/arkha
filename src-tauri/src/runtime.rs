//! Реестр установленных runtime-пакетов.
//!
//! На первом этапе реестр только перечисляет пакеты. Запуск, скачивание и
//! переключение backend будут подключены после стабилизации формата manifest.

use crate::{engine, model_manager};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimePackage {
    pub id: String,
    pub name: String,
    pub engine: String,
    pub backend: String,
    pub version: String,
    pub path: String,
    pub executable: Option<String>,
    pub installed: bool,
    pub active: bool,
    pub compatible: bool,
    pub status: String,
    pub notes: Vec<String>,
    pub managed: bool,
}

pub struct ActiveRuntime {
    pub executable: PathBuf,
    pub root: PathBuf,
    pub path_dirs: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeCompatibility {
    pub runtime_id: String,
    pub compatible: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub checked_files: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RuntimeManifest {
    pub id: String,
    pub name: String,
    pub engine: String,
    pub backend: String,
    pub version: String,
    pub executable: Option<String>,
    #[serde(default)]
    pub required_files: Vec<String>,
    #[serde(default)]
    pub requires_cuda: bool,
}

fn runtime_root() -> PathBuf {
    crate::profile::config_dir().join("data").join("runtimes")
}

fn package_from_manifest(
    package_dir: &Path,
    manifest_path: &Path,
    active_id: Option<&str>,
) -> Result<RuntimePackage, String> {
    let manifest_text = fs::read_to_string(manifest_path).map_err(|error| error.to_string())?;
    let manifest: RuntimeManifest =
        serde_json::from_str(&manifest_text).map_err(|error| error.to_string())?;
    let executable = manifest
        .executable
        .as_ref()
        .map(|relative| package_dir.join(relative));
    let mut missing = Vec::new();
    if let Some(path) = &executable {
        if !path.exists() {
            missing.push(path.to_string_lossy().to_string());
        }
    }
    for relative in &manifest.required_files {
        let path = package_dir.join(relative);
        if !path.exists() {
            missing.push(relative.clone());
        }
    }
    let cuda_missing = manifest.requires_cuda && !engine::cuda_есть();
    let installed = missing.is_empty();
    let compatible = installed && !cuda_missing;
    let status = if !installed {
        "corrupted"
    } else if active_id == Some(manifest.id.as_str()) {
        "active"
    } else {
        "installed"
    };

    Ok(RuntimePackage {
        id: manifest.id,
        name: manifest.name,
        engine: manifest.engine,
        backend: manifest.backend,
        version: manifest.version,
        path: package_dir.to_string_lossy().to_string(),
        executable: executable.map(|path| path.to_string_lossy().to_string()),
        installed,
        active: status == "active",
        compatible,
        status: status.to_string(),
        notes: missing
            .into_iter()
            .map(|path| format!("не найден обязательный файл: {path}"))
            .chain(cuda_missing.then_some("требуются библиотеки CUDA".to_string()))
            .collect(),
        managed: true,
    })
}

fn discover_manifest_packages(active_id: Option<&str>) -> Vec<RuntimePackage> {
    let root = runtime_root();
    let mut packages = Vec::new();
    let Ok(engines) = fs::read_dir(root) else {
        return packages;
    };

    for engine_entry in engines.flatten() {
        let Ok(backends) = fs::read_dir(engine_entry.path()) else {
            continue;
        };
        for backend_entry in backends.flatten() {
            let Ok(versions) = fs::read_dir(backend_entry.path()) else {
                continue;
            };
            for version_entry in versions.flatten() {
                let package_dir = version_entry.path();
                let manifest_path = package_dir.join("manifest.json");
                if !manifest_path.is_file() {
                    continue;
                }
                match package_from_manifest(&package_dir, &manifest_path, active_id) {
                    Ok(package) => packages.push(package),
                    Err(error) => packages.push(RuntimePackage {
                        id: format!("invalid-{}", package_dir.display()),
                        name: package_dir
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("invalid runtime")
                            .to_string(),
                        engine: "unknown".to_string(),
                        backend: "unknown".to_string(),
                        version: "unknown".to_string(),
                        path: package_dir.to_string_lossy().to_string(),
                        executable: None,
                        installed: false,
                        active: false,
                        compatible: false,
                        status: "corrupted".to_string(),
                        notes: vec![error],
                        managed: true,
                    }),
                }
            }
        }
    }
    packages
}

fn legacy_package(state: &engine::Состояние, active_id: Option<&str>) -> RuntimePackage {
    let id = format!("llama-cpp-legacy-{}", engine::ТЕГ);
    let path = model_manager::bin_dir();
    let mut notes = vec![
        "legacy runtime без manifest.json".to_string(),
        "переключение версии будет доступно после миграции пакета".to_string(),
    ];
    if state.cudaNeeded {
        notes.push("для CUDA требуется установить библиотеки NVIDIA".to_string());
    }
    RuntimePackage {
        id: id.clone(),
        name: "llama.cpp (legacy)".to_string(),
        engine: "llama.cpp".to_string(),
        backend: "auto".to_string(),
        version: engine::ТЕГ.to_string(),
        path: path.to_string_lossy().to_string(),
        executable: state.path.clone(),
        installed: state.found,
        active: active_id == Some(id.as_str()),
        compatible: state.found && !state.cudaNeeded,
        status: if !state.found {
            "missing".to_string()
        } else if active_id == Some(id.as_str()) {
            "active".to_string()
        } else {
            "installed".to_string()
        },
        notes,
        managed: false,
    }
}

/// Перечислить установленные runtime и legacy engine ArkHa.
#[tauri::command]
pub fn runtime_list() -> Result<Vec<RuntimePackage>, String> {
    let settings = crate::settings::settings_get()?;
    let active_id = settings.active_runtime.as_deref();
    let state = engine::состояние();
    let mut packages = discover_manifest_packages(active_id);

    // Legacy engine показываем даже до миграции: пользователь должен видеть,
    // почему текущий launcher работает и какой runtime ещё не оформлен как pack.
    let legacy = legacy_package(&state, active_id);
    packages.push(legacy);
    packages.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(packages)
}

fn compatibility_for_package(package: &RuntimePackage) -> RuntimeCompatibility {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut checked_files = Vec::new();
    if let Some(executable) = &package.executable {
        checked_files.push(executable.clone());
        if !Path::new(executable).is_file() {
            errors.push(format!("executable не найден: {executable}"));
        }
    }
    if !package.managed {
        if package.status == "missing" {
            errors.push("legacy llama-server.exe не найден".to_string());
        }
        if package.notes.iter().any(|note| note.contains("CUDA")) {
            warnings.push("CUDA-библиотеки не установлены".to_string());
        }
    }
    RuntimeCompatibility {
        runtime_id: package.id.clone(),
        compatible: errors.is_empty() && package.compatible,
        errors,
        warnings,
        checked_files,
    }
}

#[tauri::command]
pub fn runtime_check(id: Option<String>) -> Result<Vec<RuntimeCompatibility>, String> {
    let packages = runtime_list()?;
    Ok(packages
        .iter()
        .filter(|package| id.as_deref().is_none_or(|value| value == package.id))
        .map(compatibility_for_package)
        .collect())
}

fn managed_package_dir(id: &str) -> Result<PathBuf, String> {
    let package = runtime_list()?
        .into_iter()
        .find(|package| package.id == id)
        .ok_or_else(|| format!("runtime не найден: {id}"))?;
    if !package.managed {
        return Err("legacy runtime нельзя удалить этим способом".to_string());
    }
    let root = runtime_root();
    let path = PathBuf::from(package.path);
    let canonical_root = root
        .canonicalize()
        .unwrap_or_else(|_| root.clone());
    let canonical_path = path
        .canonicalize()
        .map_err(|error| format!("папка runtime недоступна: {error}"))?;
    if !canonical_path.starts_with(&canonical_root) {
        return Err("runtime находится вне защищённой папки".to_string());
    }
    Ok(path)
}

#[tauri::command]
pub fn runtime_delete(id: String) -> Result<Vec<RuntimePackage>, String> {
    let settings = crate::settings::settings_get()?;
    if settings.active_runtime.as_deref() == Some(id.as_str()) {
        return Err("нельзя удалить активный runtime".to_string());
    }
    let path = managed_package_dir(&id)?;
    fs::remove_dir_all(path).map_err(|error| error.to_string())?;
    runtime_list()
}

#[tauri::command]
pub fn runtime_reveal(id: String) -> Result<(), String> {
    let path = managed_package_dir(&id)?;
    std::process::Command::new("explorer.exe")
        .arg(path)
        .spawn()
        .map_err(|error| format!("не удалось открыть папку runtime: {error}"))?;
    Ok(())
}

fn safe_component(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.contains(['/', '\\', ':'])
        || value.chars().any(|character| character.is_control())
    {
        return Err(format!("недопустимый компонент {label}"));
    }
    Ok(())
}

fn safe_zip_path(path: &Path) -> Result<PathBuf, String> {
    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::Normal(value) => clean.push(value),
            _ => return Err("архив содержит небезопасный путь".to_string()),
        }
    }
    Ok(clean)
}

/// Скачать и атомарно установить runtime ZIP.
#[tauri::command]
pub async fn runtime_install(
    url: String,
    sha256: String,
    manifest: RuntimeManifest,
) -> Result<Vec<RuntimePackage>, String> {
    if !url.starts_with("https://") {
        return Err("runtime можно загружать только по HTTPS".to_string());
    }
    safe_component(&manifest.id, "id")?;
    safe_component(&manifest.engine, "engine")?;
    safe_component(&manifest.backend, "backend")?;
    safe_component(&manifest.version, "version")?;
    let expected = sha256.trim().to_ascii_lowercase();
    if expected.len() != 64 || !expected.chars().all(|character| character.is_ascii_hexdigit()) {
        return Err("некорректный SHA-256 runtime".to_string());
    }

    let response = reqwest::Client::new()
        .get(&url)
        .header("User-Agent", concat!("arkha/", env!("CARGO_PKG_VERSION")))
        .send()
        .await
        .map_err(|error| format!("не удалось начать загрузку runtime: {error}"))?
        .error_for_status()
        .map_err(|error| format!("сервер runtime ответил ошибкой: {error}"))?;
    let temp = std::env::temp_dir().join(format!("arkha-runtime-{}.zip", manifest.id));
    let mut file = fs::File::create(&temp).map_err(|error| error.to_string())?;
    let mut hash = Sha256::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| format!("загрузка runtime прервана: {error}"))?;
        hash.update(&chunk);
        file.write_all(&chunk).map_err(|error| error.to_string())?;
    }
    file.flush().map_err(|error| error.to_string())?;
    let actual = hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if actual != expected {
        let _ = fs::remove_file(&temp);
        return Err(format!("SHA-256 не совпал: ожидался {expected}, получен {actual}"));
    }

    let root = runtime_root();
    let target = root
        .join(&manifest.engine)
        .join(&manifest.backend)
        .join(&manifest.version);
    let staging = root.join(format!(".staging-{}-{}", manifest.id, std::process::id()));
    fs::create_dir_all(&staging).map_err(|error| error.to_string())?;
    let extraction = (|| -> Result<(), String> {
        let archive_file = fs::File::open(&temp).map_err(|error| error.to_string())?;
        let mut archive = zip::ZipArchive::new(archive_file).map_err(|error| error.to_string())?;
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).map_err(|error| error.to_string())?;
            let relative = safe_zip_path(Path::new(entry.name()))?;
            let destination = staging.join(relative);
            if entry.is_dir() {
                fs::create_dir_all(&destination).map_err(|error| error.to_string())?;
            } else {
                if let Some(parent) = destination.parent() {
                    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
                }
                let mut output = fs::File::create(&destination).map_err(|error| error.to_string())?;
                std::io::copy(&mut entry, &mut output).map_err(|error| error.to_string())?;
            }
        }
        let manifest_path = staging.join("manifest.json");
        let text = serde_json::to_string_pretty(&manifest).map_err(|error| error.to_string())?;
        fs::write(manifest_path, text).map_err(|error| error.to_string())?;
        let executable = manifest
            .executable
            .as_ref()
            .ok_or_else(|| "manifest не содержит executable".to_string())?;
        if !staging.join(executable).is_file() {
            return Err("после распаковки executable не найден".to_string());
        }
        Ok(())
    })();
    let _ = fs::remove_file(&temp);
    if let Err(error) = extraction {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    fs::create_dir_all(target.parent().expect("runtime target has parent"))
        .map_err(|error| error.to_string())?;
    if target.exists() {
        fs::remove_dir_all(&target).map_err(|error| error.to_string())?;
    }
    fs::rename(staging, target).map_err(|error| error.to_string())?;
    runtime_list()
}

/// Сделать установленный совместимый runtime активным.
#[tauri::command]
pub fn runtime_set_active(id: String) -> Result<Vec<RuntimePackage>, String> {
    let mut settings = crate::settings::settings_get()?;
    let packages = runtime_list()?;
    let package = packages
        .iter()
        .find(|package| package.id == id)
        .ok_or_else(|| format!("runtime не найден: {id}"))?;
    if !package.installed || !package.compatible {
        return Err(format!("runtime недоступен: {}", package.status));
    }
    settings.active_runtime = Some(id);
    crate::settings::settings_save(settings)?;
    runtime_list()
}

/// Resolve the selected runtime into paths used by the launcher.
pub fn active_runtime_paths() -> Result<ActiveRuntime, String> {
    let settings = crate::settings::settings_get()?;
    let packages = runtime_list()?;
    let selected = settings
        .active_runtime
        .as_deref()
        .and_then(|id| packages.iter().find(|package| package.id == id));

    let package = match selected {
        Some(package) => package,
        None if settings.active_runtime.is_some() => {
            return Err("выбранный runtime больше не найден или повреждён".to_string())
        }
        None => packages
            .iter()
            .find(|package| package.id.starts_with("llama-cpp-legacy-") && package.installed)
            .ok_or_else(|| "не установлен ни один совместимый runtime".to_string())?,
    };

    if !package.installed || !package.compatible {
        return Err(format!("выбранный runtime недоступен: {}", package.status));
    }
    let executable = package
        .executable
        .as_ref()
        .map(PathBuf::from)
        .ok_or_else(|| "у runtime отсутствует executable".to_string())?;
    let root = PathBuf::from(&package.path);
    let mut path_dirs = vec![root.clone()];
    for directory in [root.join("cudart"), root.join("lib")] {
        if directory.is_dir() {
            path_dirs.push(directory);
        }
    }

    Ok(ActiveRuntime {
        executable,
        root,
        path_dirs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_package_is_explicitly_marked() {
        let state = engine::Состояние {
            found: true,
            path: Some("C:\\ArkHa\\llama-server.exe".to_string()),
            searched: Vec::new(),
            cudaNeeded: false,
            cudaPresent: false,
            gpuName: String::new(),
        };
        let package = legacy_package(&state, None);
        assert_eq!(package.backend, "auto");
        assert_eq!(package.status, "installed");
        assert!(!package.active);
        assert!(package.notes[0].contains("legacy"));
        assert!(!package.managed);
    }
}
