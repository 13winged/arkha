//! Разделение приватных данных по authenticated user id.
//!
//! Этот модуль вычисляет каталоги, но не переносит legacy-файлы. Миграция
//! будет отдельной операцией после явного подтверждения пользователя.

use crate::auth::AuthenticatedUser;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone)]
pub struct UserDataRoots {
    pub root: PathBuf,
    pub settings_db: PathBuf,
    pub memory_db: PathBuf,
    pub attachments: PathBuf,
    pub skills: PathBuf,
    pub permissions: PathBuf,
    pub profile: PathBuf,
    pub legacy_migration: PathBuf,
    /// GGUF files are shared read-only; active model selection is user-specific.
    pub shared_models: PathBuf,
    /// Runtime binaries are shared; active runtime selection is user-specific.
    pub shared_runtimes: PathBuf,
}

fn safe_id(value: &str) -> Result<&str, String> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err("недопустимый идентификатор профиля".to_string());
    }
    Ok(value)
}

pub fn roots_for(user: &AuthenticatedUser) -> Result<UserDataRoots, String> {
    let id = safe_id(&user.user.id)?;
    let app_root = crate::profile::config_dir();
    let root = if user.guest {
        app_root.join("guest-sessions").join(id)
    } else {
        app_root.join("users").join(id)
    };
    Ok(UserDataRoots {
        settings_db: root.join("data").join("secretary.db"),
        memory_db: root.join("memory.db"),
        attachments: root.join("attachments"),
        skills: root.join("skills"),
        permissions: root.join("permissions.json"),
        profile: root.join("profile.json"),
        legacy_migration: root.join("legacy-migration.json"),
        shared_models: app_root.join("models"),
        shared_runtimes: app_root.join("data").join("runtimes"),
        root,
    })
}

pub fn current_roots() -> Result<UserDataRoots, String> {
    roots_for(&crate::auth::require_authenticated()?)
}

pub fn ensure_private_roots(roots: &UserDataRoots) -> Result<(), String> {
    for path in [
        roots.root.as_path(),
        roots.settings_db.parent().unwrap_or(Path::new(".")),
        roots.attachments.as_path(),
        roots.skills.as_path(),
    ] {
        std::fs::create_dir_all(path)
            .map_err(|error| format!("не удалось создать каталог профиля: {error}"))?;
    }
    Ok(())
}

pub fn contains_private_path(roots: &UserDataRoots, candidate: &Path) -> bool {
    let Ok(root) = roots.root.canonicalize() else { return false };
    let Ok(candidate) = candidate.canonicalize() else { return false };
    candidate.starts_with(root)
        && candidate
            .components()
            .all(|component| !matches!(component, Component::ParentDir))
}

/// Detect paths into another ArkHa account's private storage, even when a
/// user has granted the agent broad filesystem access.
pub fn is_other_profile_path(roots: &UserDataRoots, candidate: &Path) -> bool {
    is_other_profile_path_at(roots, candidate, &crate::profile::config_dir())
}

/// Deny direct access to global account/auth files and unclaimed pre-account
/// user data. These files are not shared assets and remain in place after copy.
pub fn is_protected_app_data_path(candidate: &Path) -> bool {
    is_protected_app_data_path_at(candidate, &crate::profile::config_dir())
}

fn is_protected_app_data_path_at(candidate: &Path, app_root: &Path) -> bool {
    let (Ok(candidate), Ok(app_root)) = (candidate.canonicalize(), app_root.canonicalize()) else {
        return false;
    };
    let protected_files = [
        "auth.db",
        "auth.db-wal",
        "auth.db-shm",
        "remembered-session.dpapi",
        "memory.db",
        "memory.db-wal",
        "memory.db-shm",
        "permissions.json",
        "profile.json",
        "audio.json",
        "theme.json",
        "context",
        "data/secretary.db",
        "data/secretary.db-wal",
        "data/secretary.db-shm",
    ];
    if protected_files
        .iter()
        .filter_map(|relative| app_root.join(relative).canonicalize().ok())
        .any(|protected| candidate == protected)
    {
        return true;
    }
    ["attachments", "skills"].iter().any(|relative| {
        app_root
            .join(relative)
            .canonicalize()
            .is_ok_and(|protected| candidate.starts_with(protected))
    })
}

fn is_other_profile_path_at(roots: &UserDataRoots, candidate: &Path, app_root: &Path) -> bool {
    let Ok(candidate) = candidate.canonicalize() else {
        return false;
    };
    ["users", "guest-sessions"].iter().any(|directory| {
        app_root
            .join(directory)
            .canonicalize()
            .is_ok_and(|profiles_root| {
                if !candidate.starts_with(&profiles_root) {
                    return false;
                }
                roots
                    .root
                    .canonicalize()
                    .is_ok_and(|own_root| !candidate.starts_with(own_root))
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(id: &str, guest: bool) -> AuthenticatedUser {
        AuthenticatedUser {
            user: crate::auth::AuthUser {
                id: id.to_string(),
                login: "test".into(),
                display_name: "Test".into(),
            },
            guest,
            session_token_hash: "test-token-hash".into(),
        }
    }

    #[test]
    fn users_have_distinct_private_roots_but_share_models() {
        let alice = roots_for(&user("alice-id", false)).unwrap();
        let bob = roots_for(&user("bob-id", false)).unwrap();
        assert_ne!(alice.root, bob.root);
        assert_ne!(alice.memory_db, bob.memory_db);
        assert_eq!(alice.shared_models, bob.shared_models);
        assert_eq!(alice.shared_runtimes, bob.shared_runtimes);
    }

    #[test]
    fn guest_root_is_separate() {
        let guest = roots_for(&user("session-123", true)).unwrap();
        let registered = roots_for(&user("session-123", false)).unwrap();
        assert_ne!(guest.root, registered.root);
        assert!(guest.root.to_string_lossy().contains("guest-sessions"));
    }

    #[test]
    fn path_like_ids_are_rejected() {
        assert!(roots_for(&user("../alice", false)).is_err());
        assert!(roots_for(&user("C:\\alice", false)).is_err());
    }

    #[test]
    fn another_profiles_files_stay_blocked_even_when_paths_are_granted() {
        let base = std::env::temp_dir().join(format!(
            "arkha-user-data-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let own = base.join("users").join("alice");
        let other = base.join("users").join("bob");
        std::fs::create_dir_all(&own).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        let own_file = own.join("memory.db");
        let other_file = other.join("memory.db");
        std::fs::write(&own_file, b"own").unwrap();
        std::fs::write(&other_file, b"other").unwrap();
        let roots = UserDataRoots {
            root: own.clone(),
            settings_db: own.join("data").join("secretary.db"),
            memory_db: own_file.clone(),
            attachments: own.join("attachments"),
            skills: own.join("skills"),
            permissions: own.join("permissions.json"),
            profile: own.join("profile.json"),
            legacy_migration: own.join("legacy-migration.json"),
            shared_models: base.join("models"),
            shared_runtimes: base.join("runtimes"),
        };

        assert!(!is_other_profile_path_at(&roots, &own_file, &base));
        assert!(is_other_profile_path_at(&roots, &other_file, &base));
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn app_private_and_legacy_data_are_blocked_but_shared_models_are_not() {
        let base = std::env::temp_dir().join(format!(
            "arkha-protected-path-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let attachments = base.join("attachments");
        let models = base.join("models");
        std::fs::create_dir_all(&attachments).unwrap();
        std::fs::create_dir_all(&models).unwrap();
        let auth_db = base.join("auth.db");
        let old_memory = base.join("memory.db");
        let attachment = attachments.join("old.txt");
        let model = models.join("shared.gguf");
        std::fs::write(&auth_db, b"accounts").unwrap();
        std::fs::write(&old_memory, b"legacy").unwrap();
        std::fs::write(&attachment, b"legacy attachment").unwrap();
        std::fs::write(&model, b"shared asset").unwrap();

        assert!(is_protected_app_data_path_at(&auth_db, &base));
        assert!(is_protected_app_data_path_at(&old_memory, &base));
        assert!(is_protected_app_data_path_at(&attachment, &base));
        assert!(!is_protected_app_data_path_at(&model, &base));
        std::fs::remove_dir_all(base).unwrap();
    }
}
