//! Explicit, opt-in import of single-user data from pre-account ArkHa.

use rusqlite::{backup::Backup, params, Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Manager};

static MIGRATION_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryPreview {
    pub id: String,
    pub title: String,
    pub description: String,
    pub files: usize,
    pub bytes: u64,
    pub conflicts: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationPreview {
    pub pending: bool,
    pub already_decided: bool,
    pub already_claimed: bool,
    pub guest: bool,
    pub browser_preferences: bool,
    pub categories: Vec<CategoryPreview>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationResult {
    pub imported: Vec<String>,
    pub skipped: bool,
}

#[derive(Clone, Copy)]
enum ArtifactKind {
    File,
    Sqlite,
    Directory,
}

#[derive(Clone)]
struct Artifact {
    source: PathBuf,
    destination: PathBuf,
    kind: ArtifactKind,
}

struct Category {
    id: &'static str,
    title: &'static str,
    description: &'static str,
    artifacts: Vec<Artifact>,
}

#[derive(Clone)]
struct SourceFile {
    source: PathBuf,
    destination: PathBuf,
    bytes: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Receipt {
    skipped: bool,
    imported: Vec<String>,
    decided_at: String,
}

fn categories(app_root: &Path, roots: &crate::user_data::UserDataRoots) -> Vec<Category> {
    vec![
        Category {
            id: "settings",
            title: "Настройки приложения",
            description: "Язык, интерфейс, выбранные модель и режим агента.",
            artifacts: vec![Artifact {
                source: app_root.join("data").join("secretary.db"),
                destination: roots.settings_db.clone(),
                kind: ArtifactKind::Sqlite,
            }],
        },
        Category {
            id: "memory",
            title: "Память, RAG и вложения",
            description: "Локальная память, индекс документов и связанные файлы вложений.",
            artifacts: vec![
                Artifact {
                    source: app_root.join("memory.db"),
                    destination: roots.memory_db.clone(),
                    kind: ArtifactKind::Sqlite,
                },
                Artifact {
                    source: app_root.join("attachments"),
                    destination: roots.attachments.clone(),
                    kind: ArtifactKind::Directory,
                },
            ],
        },
        Category {
            id: "permissions",
            title: "Разрешения агента",
            description: "Выбранные папки, инструменты и режим подтверждений.",
            artifacts: vec![Artifact {
                source: app_root.join("permissions.json"),
                destination: roots.permissions.clone(),
                kind: ArtifactKind::File,
            }],
        },
        Category {
            id: "profile",
            title: "Профиль устройства",
            description: "Ручной профиль железа и выбранный размер контекста.",
            artifacts: vec![
                Artifact {
                    source: app_root.join("profile.json"),
                    destination: roots.profile.clone(),
                    kind: ArtifactKind::File,
                },
                Artifact {
                    source: app_root.join("context"),
                    destination: roots.root.join("context"),
                    kind: ArtifactKind::File,
                },
            ],
        },
        Category {
            id: "preferences",
            title: "Звук и тема",
            description: "Выбранные аудиоустройства и оформление интерфейса.",
            artifacts: vec![
                Artifact {
                    source: app_root.join("audio.json"),
                    destination: roots.root.join("audio.json"),
                    kind: ArtifactKind::File,
                },
                Artifact {
                    source: app_root.join("theme.json"),
                    destination: roots.root.join("theme.json"),
                    kind: ArtifactKind::File,
                },
            ],
        },
        Category {
            id: "skills",
            title: "Пользовательские навыки",
            description: "Навыки, добавленные в старую пользовательскую папку skills.",
            artifacts: vec![Artifact {
                source: app_root.join("skills"),
                destination: roots.skills.clone(),
                kind: ArtifactKind::Directory,
            }],
        },
    ]
}

fn profile_storage_root(app_root: &Path, candidate: &Path) -> bool {
    let (Ok(app_root), Ok(candidate)) = (app_root.canonicalize(), candidate.canonicalize()) else {
        return false;
    };
    ["users", "guest-sessions"].iter().any(|name| {
        app_root
            .join(name)
            .canonicalize()
            .is_ok_and(|root| candidate.starts_with(root))
    })
}

fn collect_artifact_files(
    artifact: &Artifact,
    app_root: &Path,
) -> Result<Vec<SourceFile>, String> {
    let metadata = match fs::symlink_metadata(&artifact.source) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("не удалось проверить legacy-путь: {error}")),
    };
    if metadata.file_type().is_symlink() || profile_storage_root(app_root, &artifact.source) {
        return Ok(Vec::new());
    }

    match artifact.kind {
        ArtifactKind::File | ArtifactKind::Sqlite => {
            if !metadata.is_file() {
                return Ok(Vec::new());
            }
            let canonical = artifact
                .source
                .canonicalize()
                .map_err(|error| format!("legacy-файл недоступен: {error}"))?;
            let app_canonical = app_root
                .canonicalize()
                .map_err(|error| format!("каталог ArkHa недоступен: {error}"))?;
            if !canonical.starts_with(&app_canonical) || profile_storage_root(app_root, &canonical) {
                return Ok(Vec::new());
            }
            Ok(vec![SourceFile {
                source: artifact.source.clone(),
                destination: artifact.destination.clone(),
                bytes: metadata.len(),
            }])
        }
        ArtifactKind::Directory => {
            if !metadata.is_dir() {
                return Ok(Vec::new());
            }
            let canonical_root = artifact
                .source
                .canonicalize()
                .map_err(|error| format!("legacy-папка недоступна: {error}"))?;
            let app_canonical = app_root
                .canonicalize()
                .map_err(|error| format!("каталог ArkHa недоступен: {error}"))?;
            if !canonical_root.starts_with(&app_canonical)
                || profile_storage_root(app_root, &canonical_root)
            {
                return Ok(Vec::new());
            }
            let mut result = Vec::new();
            collect_directory_files(
                &artifact.source,
                &canonical_root,
                &artifact.destination,
                &mut result,
            )?;
            Ok(result)
        }
    }
}

fn collect_directory_files(
    source_root: &Path,
    canonical_root: &Path,
    destination_root: &Path,
    result: &mut Vec<SourceFile>,
) -> Result<(), String> {
    for entry in fs::read_dir(source_root).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let source = entry.path();
        let metadata = fs::symlink_metadata(&source).map_err(|error| error.to_string())?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            collect_directory_files(&source, canonical_root, destination_root, result)?;
        } else if metadata.is_file() {
            let canonical = source.canonicalize().map_err(|error| error.to_string())?;
            if !canonical.starts_with(canonical_root) {
                continue;
            }
            let relative = canonical
                .strip_prefix(canonical_root)
                .map_err(|error| error.to_string())?;
            if !safe_relative(relative) {
                continue;
            }
            result.push(SourceFile {
                source,
                destination: destination_root.join(relative),
                bytes: metadata.len(),
            });
        }
    }
    Ok(())
}

fn safe_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path.components().all(|component| matches!(component, Component::Normal(_)))
}

fn inspect_category(category: &Category, app_root: &Path) -> Result<CategoryPreview, String> {
    let mut files = 0usize;
    let mut bytes = 0u64;
    let mut conflicts = 0usize;
    for artifact in &category.artifacts {
        for file in collect_artifact_files(artifact, app_root)? {
            files += 1;
            bytes = bytes.saturating_add(file.bytes);
            if fs::symlink_metadata(&file.destination).is_ok() {
                conflicts += 1;
            }
        }
    }
    Ok(CategoryPreview {
        id: category.id.to_string(),
        title: category.title.to_string(),
        description: category.description.to_string(),
        files,
        bytes,
        conflicts,
    })
}

fn migration_receipt(roots: &crate::user_data::UserDataRoots) -> Result<Option<Receipt>, String> {
    match fs::read(&roots.legacy_migration) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|error| format!("файл состояния миграции повреждён: {error}")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("не удалось прочитать состояние миграции: {error}")),
    }
}

fn global_claim_path() -> PathBuf {
    crate::profile::config_dir().join("legacy-migration-claimed")
}

fn global_claimed() -> bool {
    global_claim_path().exists()
}

fn claim_global_legacy_data() -> Result<(), String> {
    claim_global_legacy_data_at(&global_claim_path())
}

fn claim_global_legacy_data_at(path: &Path) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "не удалось определить каталог legacy-данных".to_string())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|error| {
            if error.kind() == io::ErrorKind::AlreadyExists {
                "legacy-данные уже назначены другому аккаунту".to_string()
            } else {
                format!("не удалось закрепить legacy-данные за аккаунтом: {error}")
            }
        })?;
    let write_result = file
        .write_all(b"ArkHa legacy data claimed\n")
        .and_then(|()| file.sync_all());
    if let Err(error) = write_result {
        let _ = fs::remove_file(path);
        return Err(error.to_string());
    }
    Ok(())
}

fn release_global_claim() {
    let _ = fs::remove_file(global_claim_path());
}

fn write_receipt(
    roots: &crate::user_data::UserDataRoots,
    receipt: &Receipt,
) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(receipt).map_err(|error| error.to_string())?;
    let temporary = roots.root.join(format!(
        "legacy-migration-{}-{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let result = (|| -> Result<(), String> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| format!("не удалось записать решение миграции: {error}"))?;
        file.write_all(&bytes).map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        fs::rename(&temporary, &roots.legacy_migration)
            .map_err(|error| format!("не удалось сохранить решение миграции: {error}"))
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

fn registered_roots() -> Result<crate::user_data::UserDataRoots, String> {
    let user = crate::auth::require_authenticated()?;
    if user.guest {
        return Err("legacy-данные можно переносить только в постоянный аккаунт".into());
    }
    crate::user_data::roots_for(&user)
}

#[tauri::command]
pub fn legacy_migration_preview(
    legacy_language: Option<String>,
    legacy_font_size: Option<u32>,
) -> Result<MigrationPreview, String> {
    let user = crate::auth::require_authenticated()?;
    if user.guest {
        return Ok(MigrationPreview {
            pending: false,
            already_decided: false,
            already_claimed: false,
            guest: true,
            browser_preferences: false,
            categories: Vec::new(),
        });
    }
    let roots = crate::user_data::roots_for(&user)?;
    if migration_receipt(&roots)?.is_some() {
        return Ok(MigrationPreview {
            pending: false,
            already_decided: true,
            already_claimed: global_claimed(),
            guest: false,
            browser_preferences: false,
            categories: Vec::new(),
        });
    }
    if global_claimed() {
        return Ok(MigrationPreview {
            pending: false,
            already_decided: true,
            already_claimed: true,
            guest: false,
            browser_preferences: false,
            categories: Vec::new(),
        });
    }
    let app_root = crate::profile::config_dir();
    let categories = categories(&app_root, &roots)
        .iter()
        .map(|category| inspect_category(category, &app_root))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|category| category.files > 0)
        .collect::<Vec<_>>();
    let browser_preferences = valid_language(legacy_language.as_deref())
        || valid_font_size(legacy_font_size);
    Ok(MigrationPreview {
        pending: !categories.is_empty() || browser_preferences,
        already_decided: false,
        already_claimed: false,
        guest: false,
        browser_preferences,
        categories,
    })
}

#[tauri::command]
pub fn legacy_migration_skip() -> Result<MigrationResult, String> {
    let _lock = MIGRATION_LOCK.lock().map_err(|_| "миграция занята".to_string())?;
    let roots = registered_roots()?;
    if global_claimed() {
        return Err("legacy-данные уже назначены другому аккаунту".into());
    }
    if migration_receipt(&roots)?.is_some() {
        return Err("решение о переносе уже сохранено".into());
    }
    write_receipt(
        &roots,
        &Receipt {
            skipped: true,
            imported: Vec::new(),
            decided_at: now_utc(),
        },
    )?;
    Ok(MigrationResult { imported: Vec::new(), skipped: true })
}

#[tauri::command]
pub fn legacy_migration_import(
    app: AppHandle,
    categories_selected: Vec<String>,
    legacy_language: Option<String>,
    legacy_font_size: Option<u32>,
) -> Result<MigrationResult, String> {
    let _lock = MIGRATION_LOCK.lock().map_err(|_| "миграция занята".to_string())?;
    let roots = registered_roots()?;
    if global_claimed() {
        return Err("legacy-данные уже назначены другому аккаунту".into());
    }
    if migration_receipt(&roots)?.is_some() {
        return Err("решение о переносе уже сохранено".into());
    }
    let mut selected = categories_selected.into_iter().collect::<BTreeSet<_>>();
    let import_browser_preferences = selected.remove("browser-preferences");
    if selected.is_empty() && !import_browser_preferences {
        return Err("выберите хотя бы одну категорию для переноса".into());
    }
    if import_browser_preferences
        && !valid_language(legacy_language.as_deref())
        && !valid_font_size(legacy_font_size)
    {
        return Err("нет допустимых старых настроек интерфейса для переноса".into());
    }
    let previous_settings = crate::settings::load_existing()?;
    let settings_db_existed = roots.settings_db.is_file();
    let app_root = crate::profile::config_dir();
    let available = categories(&app_root, &roots);
    let mut selected_categories = Vec::new();
    for category in available {
        if selected.contains(category.id) {
            let preview = inspect_category(&category, &app_root)?;
            if preview.files == 0 {
                return Err(format!("в категории «{}» больше нет legacy-файлов", category.title));
            }
            if preview.conflicts > 0 {
                return Err(format!("категория «{}» уже содержит файлы профиля; перезапись запрещена", category.title));
            }
            selected_categories.push(category);
        }
    }
    if selected_categories.len() != selected.len() {
        return Err("запрошена неизвестная категория переноса".into());
    }

    let mut created = Vec::new();
    let transfer = (|| -> Result<(), String> {
        for category in &selected_categories {
            for artifact in &category.artifacts {
                let files = collect_artifact_files(artifact, &app_root)?;
                if matches!(artifact.kind, ArtifactKind::Sqlite) {
                    if let Some(file) = files.first() {
                        backup_sqlite(&file.source, &file.destination, &roots, &mut created)?;
                    }
                } else {
                    for file in files {
                        copy_file(&file.source, &file.destination, &roots, &mut created)?;
                    }
                }
            }
        }
        if selected.contains("memory") {
            repair_attachment_paths(&roots.memory_db, &roots.attachments)?;
        }
        if import_browser_preferences {
            let mut settings = crate::settings::load_existing()?.unwrap_or_default();
            if valid_language(legacy_language.as_deref()) {
                let language = legacy_language.as_deref().unwrap_or("ru");
                settings.language = language.to_string();
                settings.voice.language = language.to_string();
            }
            if let Some(font_size) = legacy_font_size.filter(|size| valid_font_size(Some(*size))) {
                settings.font_size = font_size;
            }
            if !settings_db_existed && !created.contains(&roots.settings_db) {
                created.push(roots.settings_db.clone());
            }
            crate::settings::settings_save(settings)?;
        }
        Ok(())
    })();
    if let Err(error) = transfer {
        rollback_created(&created, &roots.root);
        restore_settings_after_rollback(&app, settings_db_existed, previous_settings);
        return Err(error);
    }

    let loaded = (|| -> Result<_, String> {
        app.state::<crate::agent_commands::SettingsState>()
            .reload_for_active_user()?;
        crate::settings::load_existing()
    })();
    let settings = match loaded {
        Ok(settings) => settings.unwrap_or_default(),
        Err(error) => {
            rollback_created(&created, &roots.root);
            restore_settings_after_rollback(&app, settings_db_existed, previous_settings);
            return Err(error);
        }
    };
    let mode = crate::agent_modes::AgentMode::from_str(&settings.agent_mode)
        .unwrap_or(crate::agent_modes::AgentMode::Build);
    crate::agent_modes::set_mode(mode);
    if let Err(error) = claim_global_legacy_data() {
        rollback_created(&created, &roots.root);
        restore_settings_after_rollback(&app, settings_db_existed, previous_settings);
        return Err(error);
    }
    let mut imported = selected_categories.iter().map(|category| category.id.to_string()).collect::<Vec<_>>();
    if import_browser_preferences {
        imported.push("browser-preferences".into());
    }
    if let Err(error) = write_receipt(
        &roots,
        &Receipt { skipped: false, imported, decided_at: now_utc() },
    ) {
        release_global_claim();
        rollback_created(&created, &roots.root);
        restore_settings_after_rollback(&app, settings_db_existed, previous_settings);
        return Err(error);
    }
    let mut imported = selected_categories.iter().map(|category| category.id.to_string()).collect::<Vec<_>>();
    if import_browser_preferences {
        imported.push("browser-preferences".into());
    }
    Ok(MigrationResult {
        imported,
        skipped: false,
    })
}

fn valid_language(language: Option<&str>) -> bool {
    matches!(language, Some("ru" | "en"))
}

fn valid_font_size(font_size: Option<u32>) -> bool {
    matches!(font_size, Some(13 | 14 | 15 | 16 | 18))
}

fn restore_settings_after_rollback(
    app: &AppHandle,
    settings_db_existed: bool,
    previous_settings: Option<crate::settings::AppSettings>,
) {
    if settings_db_existed {
        let _ = crate::settings::restore_optional(previous_settings.clone());
    }
    if let Some(state) = app.try_state::<crate::agent_commands::SettingsState>() {
        let _ = state.reload_for_active_user();
    }
    let previous = previous_settings.unwrap_or_default();
    let mode = crate::agent_modes::AgentMode::from_str(&previous.agent_mode)
        .unwrap_or(crate::agent_modes::AgentMode::Build);
    crate::agent_modes::set_mode(mode);
}

fn backup_sqlite(
    source: &Path,
    destination: &Path,
    roots: &crate::user_data::UserDataRoots,
    created: &mut Vec<PathBuf>,
) -> Result<(), String> {
    ensure_destination_parent(destination, roots)?;
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|error| format!("не удалось подготовить базу профиля: {error}"))?;
    created.push(destination.to_path_buf());

    let source = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| format!("legacy-база не открылась: {error}"))?;
    let mut destination_connection = Connection::open(destination)
        .map_err(|error| format!("новая база профиля не открылась: {error}"))?;
    let backup = Backup::new(&source, &mut destination_connection)
        .map_err(|error| format!("не удалось подготовить копию SQLite: {error}"))?;
    backup
        .run_to_completion(16, Duration::from_millis(50), None)
        .map_err(|error| format!("не удалось перенести базу SQLite: {error}"))
}

fn copy_file(
    source: &Path,
    destination: &Path,
    roots: &crate::user_data::UserDataRoots,
    created: &mut Vec<PathBuf>,
) -> Result<(), String> {
    ensure_destination_parent(destination, roots)?;
    let mut input = File::open(source).map_err(|error| format!("legacy-файл не читается: {error}"))?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|error| format!("файл назначения уже существует или недоступен: {error}"))?;
    created.push(destination.to_path_buf());
    io::copy(&mut input, &mut output).map_err(|error| format!("копирование файла не удалось: {error}"))?;
    output.sync_all().map_err(|error| error.to_string())
}

fn ensure_destination_parent(
    destination: &Path,
    roots: &crate::user_data::UserDataRoots,
) -> Result<(), String> {
    let parent = destination
        .parent()
        .ok_or_else(|| "не удалось определить папку назначения".to_string())?;
    let root_path = roots
        .root
        .canonicalize()
        .map_err(|error| format!("корень профиля недоступен: {error}"))?;
    if !destination.starts_with(&roots.root) {
        return Err("папка назначения вышла за пределы профиля".into());
    }
    let relative_parent = parent
        .strip_prefix(&roots.root)
        .map_err(|_| "папка назначения вышла за пределы профиля".to_string())?;
    let mut current = roots.root.clone();
    for component in relative_parent.components() {
        let Component::Normal(name) = component else {
            return Err("путь назначения содержит небезопасный компонент".into());
        };
        current.push(name);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => return Err("папка назначения содержит ссылку или не является каталогом".into()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                fs::create_dir(&current)
                    .map_err(|create_error| format!("не удалось создать папку профиля: {create_error}"))?;
            }
            Err(error) => return Err(format!("папка назначения недоступна: {error}")),
        }
        let canonical = current
            .canonicalize()
            .map_err(|error| format!("папка назначения недоступна: {error}"))?;
        if !canonical.starts_with(&root_path) {
            return Err("папка назначения вышла за пределы профиля".into());
        }
    }
    Ok(())
}

fn rollback_created(created: &[PathBuf], profile_root: &Path) {
    for path in created.iter().rev() {
        let _ = fs::remove_file(path);
        let mut parent = path.parent();
        while let Some(directory) = parent {
            if directory == profile_root || !directory.starts_with(profile_root) {
                break;
            }
            if fs::remove_dir(directory).is_err() {
                break;
            }
            parent = directory.parent();
        }
    }
}

fn repair_attachment_paths(database: &Path, attachment_root: &Path) -> Result<(), String> {
    if !database.is_file() {
        return Ok(());
    }
    let connection = Connection::open(database).map_err(|error| error.to_string())?;
    let exists: Option<String> = connection
        .query_row(
            "SELECT name FROM sqlite_master WHERE type='table' AND name='attachments'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if exists.is_none() {
        return Ok(());
    }
    let rows = {
        let mut statement = connection
            .prepare("SELECT id, name FROM attachments")
            .map_err(|error| error.to_string())?;
        let mut rows = statement
            .query([])
            .map_err(|error| error.to_string())?;
        let mut values = Vec::new();
        while let Some(row) = rows.next().map_err(|error| error.to_string())? {
            values.push((row.get::<_, String>(0).map_err(|error| error.to_string())?, row.get::<_, String>(1).map_err(|error| error.to_string())?));
        }
        values
    };
    for (id, name) in rows {
        if id.len() != 64
            || !id.bytes().all(|byte| byte.is_ascii_hexdigit())
            || Path::new(&name).file_name().and_then(|file| file.to_str()) != Some(name.as_str())
        {
            connection
                .execute("DELETE FROM attachments WHERE id=?1", params![id])
                .map_err(|error| error.to_string())?;
            continue;
        }
        let path = attachment_root.join(&id).join(&name);
        if path.is_file() {
            connection
                .execute(
                    "UPDATE attachments SET path=?1 WHERE id=?2",
                    params![path.to_string_lossy(), id],
                )
                .map_err(|error| error.to_string())?;
        } else {
            connection
                .execute("DELETE FROM attachments WHERE id=?1", params![id])
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

fn now_utc() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_base() -> PathBuf {
        std::env::temp_dir().join(format!(
            "arkha-legacy-migration-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn test_roots(base: &Path) -> crate::user_data::UserDataRoots {
        let root = base.join("ArkHa").join("users").join("migration-test-user");
        fs::create_dir_all(&root).unwrap();
        crate::user_data::UserDataRoots {
            settings_db: root.join("data").join("secretary.db"),
            memory_db: root.join("memory.db"),
            attachments: root.join("attachments"),
            skills: root.join("skills"),
            permissions: root.join("permissions.json"),
            profile: root.join("profile.json"),
            legacy_migration: root.join("legacy-migration.json"),
            shared_models: base.join("ArkHa").join("models"),
            shared_runtimes: base.join("ArkHa").join("data").join("runtimes"),
            root,
        }
    }

    #[test]
    fn preview_finds_only_private_legacy_categories_not_shared_models() {
        let base = test_base();
        let app_root = base.join("ArkHa");
        let roots = test_roots(&base);
        fs::create_dir_all(app_root.join("data")).unwrap();
        fs::create_dir_all(app_root.join("models")).unwrap();
        fs::write(app_root.join("data").join("secretary.db"), b"legacy settings").unwrap();
        fs::write(app_root.join("models").join("shared.gguf"), b"shared model").unwrap();

        let all = categories(&app_root, &roots);
        let settings = all.iter().find(|category| category.id == "settings").unwrap();
        let preview = inspect_category(settings, &app_root).unwrap();
        assert_eq!(preview.files, 1);
        assert_eq!(preview.conflicts, 0);
        assert!(!all.iter().any(|category| category.id == "models"));

        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn online_sqlite_backup_preserves_legacy_rows() {
        let base = test_base();
        let app_root = base.join("ArkHa");
        let roots = test_roots(&base);
        fs::create_dir_all(&app_root).unwrap();
        let source = app_root.join("memory.db");
        let connection = Connection::open(&source).unwrap();
        connection.execute("CREATE TABLE sample (value TEXT)", []).unwrap();
        connection.execute("INSERT INTO sample (value) VALUES ('saved')", []).unwrap();
        drop(connection);

        let mut created = Vec::new();
        backup_sqlite(&source, &roots.memory_db, &roots, &mut created).unwrap();
        let imported = Connection::open(&roots.memory_db).unwrap();
        let value: String = imported.query_row("SELECT value FROM sample", [], |row| row.get(0)).unwrap();
        assert_eq!(value, "saved");
        assert!(created.contains(&roots.memory_db));
        drop(imported);

        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn imported_attachment_paths_are_rebased_and_orphans_removed() {
        let base = test_base();
        let roots = test_roots(&base);
        fs::create_dir_all(&roots.attachments).unwrap();
        let existing_id = "ab".repeat(32);
        let missing_id = "cd".repeat(32);
        let attachment = roots.attachments.join(&existing_id).join("note.txt");
        fs::create_dir_all(attachment.parent().unwrap()).unwrap();
        fs::write(&attachment, b"file").unwrap();
        let connection = Connection::open(&roots.memory_db).unwrap();
        connection.execute("CREATE TABLE attachments (id TEXT PRIMARY KEY, name TEXT, path TEXT)", []).unwrap();
        connection.execute(
            "INSERT INTO attachments VALUES (?1, 'note.txt', 'C:/old/attachments/note.txt')",
            params![existing_id],
        ).unwrap();
        connection.execute(
            "INSERT INTO attachments VALUES (?1, 'missing.txt', 'C:/old/attachments/missing.txt')",
            params![missing_id],
        ).unwrap();
        drop(connection);

        repair_attachment_paths(&roots.memory_db, &roots.attachments).unwrap();
        let imported = Connection::open(&roots.memory_db).unwrap();
        let path: String = imported.query_row(
            "SELECT path FROM attachments WHERE id=?1",
            params![existing_id],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(PathBuf::from(path), attachment);
        let remaining: i64 = imported.query_row("SELECT COUNT(*) FROM attachments", [], |row| row.get(0)).unwrap();
        assert_eq!(remaining, 1);
        drop(imported);

        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn one_installation_legacy_set_can_only_be_claimed_once() {
        let base = test_base();
        fs::create_dir_all(&base).unwrap();
        let claim = base.join("legacy-migration-claimed");

        claim_global_legacy_data_at(&claim).unwrap();
        assert!(claim_global_legacy_data_at(&claim).is_err());
        assert!(claim.exists());

        fs::remove_dir_all(base).unwrap();
    }
}
