//! Локальная аутентификация ArkHa.
//!
//! Этот модуль owns local accounts, Argon2id password hashes, session
//! lifecycle, and the backend gate used by private-data commands.

use argon2::{password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString}, Argon2};
use rand_core::{OsRng, RngCore};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Manager, State};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthUser {
    pub id: String,
    pub login: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthSession {
    pub user: AuthUser,
    pub token: String,
    pub remember_me: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthProfileSummary {
    pub login: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthProfileDeleteResult {
    pub login: String,
    pub private_data_removed: bool,
}

#[derive(Debug, Clone)]
pub struct AuthenticatedUser {
    pub user: AuthUser,
    pub guest: bool,
    pub(crate) session_token_hash: String,
}

struct ActiveSession {
    user: AuthenticatedUser,
    token: String,
    remember_me: bool,
}

static ACTIVE_USER: OnceLock<Mutex<Option<ActiveSession>>> = OnceLock::new();

fn active_user_lock() -> &'static Mutex<Option<ActiveSession>> {
    ACTIVE_USER.get_or_init(|| Mutex::new(None))
}

#[derive(Default)]
pub struct AuthState;

impl AuthState {
    fn activate(&self, session: &AuthSession, guest: bool) -> Result<(), String> {
        let mut active = active_user_lock().lock().map_err(|_| "auth state lock poisoned".to_string())?;
        if active.is_some() {
            return Err("сначала завершите текущую сессию".into());
        }
        *active = Some(ActiveSession {
            user: AuthenticatedUser {
                user: session.user.clone(),
                guest,
                session_token_hash: token_hash(&session.token),
            },
            token: session.token.clone(),
            remember_me: session.remember_me,
        });
        Ok(())
    }

    pub fn current(&self) -> Result<AuthenticatedUser, String> {
        active_user_lock()
            .lock()
            .map_err(|_| "auth state lock poisoned".to_string())?
            .as_ref()
            .map(|active| active.user.clone())
            .ok_or_else(|| "требуется вход в ArkHa".to_string())
    }

    fn active_session(&self) -> Result<Option<AuthSession>, String> {
        let active = active_user_lock()
            .lock()
            .map_err(|_| "auth state lock poisoned".to_string())?;
        Ok(active.as_ref().map(|current| AuthSession {
            user: current.user.user.clone(),
            token: current.token.clone(),
            remember_me: current.remember_me,
        }))
    }

    fn clear_token(&self, token: &str) -> Result<(), String> {
        let mut active = active_user_lock().lock().map_err(|_| "auth state lock poisoned".to_string())?;
        if active.as_ref().is_some_and(|session| session.user.session_token_hash == token_hash(token)) {
            *active = None;
            return Ok(());
        }
        Err("токен не соответствует активной сессии".into())
    }

    fn matches_token(&self, token: &str) -> Result<bool, String> {
        let active = active_user_lock().lock().map_err(|_| "auth state lock poisoned".to_string())?;
        Ok(active.as_ref().is_some_and(|session| session.user.session_token_hash == token_hash(token)))
    }

    fn ensure_no_active_session(&self) -> Result<(), String> {
        if active_user_lock().lock().map_err(|_| "auth state lock poisoned".to_string())?.is_some() {
            return Err("сначала завершите текущую сессию".into());
        }
        Ok(())
    }
}

fn activate_for_app(
    app: &AppHandle,
    auth: &AuthState,
    session: &AuthSession,
    guest: bool,
) -> Result<(), String> {
    auth.activate(session, guest)?;
    let result = (|| {
        let user = auth.current()?;
        let roots = crate::user_data::roots_for(&user)?;
        crate::user_data::ensure_private_roots(&roots)?;
        app.state::<crate::agent_commands::SettingsState>().reload_for_active_user()?;
        let settings = crate::settings::load_existing()?.unwrap_or_default();
        let mode = crate::agent_modes::AgentMode::from_str(&settings.agent_mode)
            .unwrap_or(crate::agent_modes::AgentMode::Build);
        crate::agent_modes::set_mode(mode);
        Ok(())
    })();
    if result.is_err() {
        let _ = auth.clear_token(&session.token);
    }
    result
}

/// Backend authorization gate for commands that touch private user data.
pub fn require_authenticated() -> Result<AuthenticatedUser, String> {
    AuthState.current()
}

pub fn current_user() -> Result<AuthenticatedUser, String> {
    AuthState.current()
}

fn database_path() -> PathBuf {
    crate::profile::config_dir().join("auth.db")
}

fn database() -> Result<Connection, String> {
    let path = database_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let connection = Connection::open(path).map_err(|error| error.to_string())?;
    connection
        .execute_batch(include_str!("auth_schema.sql"))
        .map_err(|error| error.to_string())?;
    connection
        .execute(
            "DELETE FROM sessions WHERE expires_at IS NULL OR expires_at <= datetime('now')",
            [],
        )
        .map_err(|error| error.to_string())?;
    Ok(connection)
}

fn delete_persisted_session(connection: &Connection, token: &str) -> Result<(), String> {
    connection
        .execute("DELETE FROM sessions WHERE token_hash = ?1", params![token_hash(token)])
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn move_profile_to_tombstone(
    roots: &crate::user_data::UserDataRoots,
    app_root: &PathBuf,
    user_id: &str,
) -> Result<Option<PathBuf>, String> {
    let metadata = match std::fs::symlink_metadata(&roots.root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("не удалось проверить папку профиля: {error}")),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("папка профиля является ссылкой или не является каталогом; удаление остановлено".into());
    }

    let users_root = app_root.join("users");
    let users_metadata = std::fs::symlink_metadata(&users_root)
        .map_err(|error| format!("не удалось проверить каталог пользователей: {error}"))?;
    if users_metadata.file_type().is_symlink() || !users_metadata.is_dir() {
        return Err("каталог пользователей является ссылкой или не является каталогом".into());
    }
    let canonical_users = users_root
        .canonicalize()
        .map_err(|error| format!("каталог пользователей недоступен: {error}"))?;
    let canonical_profile = roots
        .root
        .canonicalize()
        .map_err(|error| format!("папка профиля недоступна: {error}"))?;
    if canonical_profile.parent() != Some(canonical_users.as_path()) {
        return Err("папка профиля находится вне каталога пользователей".into());
    }

    let tombstone = users_root.join(format!(".deleting-{user_id}-{}", new_id()));
    std::fs::rename(&roots.root, &tombstone)
        .map_err(|error| format!("не удалось подготовить приватные данные к удалению: {error}"))?;
    Ok(Some(tombstone))
}

fn delete_profile_with_password(
    connection: &mut Connection,
    app_root: &PathBuf,
    login: &str,
    password: &str,
) -> Result<AuthProfileDeleteResult, String> {
    let login = validate_login(login)?;
    let row: Option<(String, String, String)> = connection
        .query_row(
            "SELECT id, display_name, password_hash FROM users WHERE login = ?1",
            params![login],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some((user_id, display_name, password_hash)) = row else {
        return Err("неверный login или пароль".into());
    };
    if !verify_password(password, &password_hash)? {
        return Err("неверный login или пароль".into());
    }

    let roots = crate::user_data::roots_for_at(
        &AuthenticatedUser {
            user: AuthUser { id: user_id.clone(), login: login.clone(), display_name },
            guest: false,
            session_token_hash: String::new(),
        },
        app_root,
    )?;
    let tombstone = move_profile_to_tombstone(&roots, app_root, &user_id)?;
    let delete_result = (|| -> Result<(), String> {
        let transaction = connection.transaction().map_err(|error| error.to_string())?;
        transaction
            .execute("DELETE FROM sessions WHERE user_id = ?1", params![user_id])
            .map_err(|error| error.to_string())?;
        let removed = transaction
            .execute("DELETE FROM users WHERE id = ?1", params![user_id])
            .map_err(|error| error.to_string())?;
        if removed != 1 {
            return Err("профиль уже удалён или изменился".into());
        }
        transaction.commit().map_err(|error| error.to_string())
    })();

    if let Err(error) = delete_result {
        if let Some(tombstone) = tombstone.as_ref() {
            if let Err(restore_error) = std::fs::rename(tombstone, &roots.root) {
                return Err(format!("{error}; приватная папка не восстановилась: {restore_error}"));
            }
        }
        return Err(error);
    }

    let private_data_removed = match tombstone {
        Some(tombstone) => match std::fs::remove_dir_all(tombstone) {
            Ok(()) => true,
            Err(error) => {
                tracing::error!("профиль {login} удалён, но временную папку не удалось очистить: {error}");
                false
            }
        },
        None => true,
    };
    Ok(AuthProfileDeleteResult { login, private_data_removed })
}

fn validate_login(login: &str) -> Result<String, String> {
    let login = login.trim().to_ascii_lowercase();
    if login.len() < 3 || login.len() > 64 {
        return Err("login должен содержать от 3 до 64 символов".into());
    }
    if !login.chars().all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')) {
        return Err("login может содержать только латинские буквы, цифры, точку, дефис и _".into());
    }
    Ok(login)
}

fn validate_password(password: &str) -> Result<(), String> {
    if password.chars().count() < 8 {
        return Err("пароль должен содержать минимум 8 символов".into());
    }
    Ok(())
}

fn hash_password(password: &str) -> Result<String, String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|error| format!("не удалось захешировать пароль: {error}"))
}

fn verify_password(password: &str, hash: &str) -> Result<bool, String> {
    let parsed = PasswordHash::new(hash).map_err(|error| error.to_string())?;
    Ok(Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok())
}

fn token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn token_hash(value: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(value.as_bytes());
    digest.finalize().iter().map(|byte| format!("{byte:02x}")).collect()
}

fn new_id() -> String {
    token()
}

#[tauri::command]
pub fn auth_register(app: AppHandle, auth: State<'_, AuthState>, login: String, password: String, display_name: String) -> Result<AuthSession, String> {
    auth.ensure_no_active_session()?;
    crate::secure_storage::clear_token()?;
    let login = validate_login(&login)?;
    validate_password(&password)?;
    let display_name = display_name.trim().to_string();
    if display_name.is_empty() || display_name.chars().count() > 120 {
        return Err("укажите имя пользователя длиной до 120 символов".into());
    }
    let connection = database()?;
    let id = new_id();
    let hash = hash_password(&password)?;
    connection
        .execute(
            "INSERT INTO users (id, login, display_name, password_hash) VALUES (?1, ?2, ?3, ?4)",
            params![id, login, display_name, hash],
        )
        .map_err(|error| format!("не удалось создать пользователя: {error}"))?;
    let session = create_session(&connection, &id, &login, &display_name, false)?;
    activate_for_app(&app, &auth, &session, false)?;
    Ok(session)
}

#[tauri::command]
pub fn auth_login(app: AppHandle, auth: State<'_, AuthState>, login: String, password: String, remember_me: bool) -> Result<AuthSession, String> {
    auth.ensure_no_active_session()?;
    let login = validate_login(&login)?;
    let connection = database()?;
    let row: Option<(String, String, String, String)> = connection
        .query_row(
            "SELECT id, login, display_name, password_hash FROM users WHERE login = ?1",
            params![login],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some((id, login, display_name, hash)) = row else {
        return Err("неверный login или пароль".into());
    };
    if !verify_password(&password, &hash)? {
        return Err("неверный login или пароль".into());
    }
    connection
        .execute("UPDATE users SET last_login_at = datetime('now') WHERE id = ?1", params![id])
        .map_err(|error| error.to_string())?;
    crate::secure_storage::clear_token()?;
    let session = create_session(&connection, &id, &login, &display_name, remember_me)?;
    if remember_me {
        if let Err(error) = crate::secure_storage::store_token(&session.token) {
            let _ = delete_persisted_session(&connection, &session.token);
            return Err(error);
        }
    }
    if let Err(error) = activate_for_app(&app, &auth, &session, false) {
        let _ = crate::secure_storage::clear_token();
        let _ = delete_persisted_session(&connection, &session.token);
        return Err(error);
    }
    Ok(session)
}

fn create_session(
    connection: &Connection,
    user_id: &str,
    login: &str,
    display_name: &str,
    remember_me: bool,
) -> Result<AuthSession, String> {
    let raw_token = token();
    if remember_me {
        connection
            .execute(
                "INSERT INTO sessions (id, user_id, token_hash, remember_me, expires_at)
                 VALUES (?1, ?2, ?3, 1, datetime('now', '+30 days'))",
                params![new_id(), user_id, token_hash(&raw_token)],
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(AuthSession {
        user: AuthUser { id: user_id.to_string(), login: login.to_string(), display_name: display_name.to_string() },
        token: raw_token,
        remember_me,
    })
}

#[tauri::command]
pub fn auth_guest(app: AppHandle, auth: State<'_, AuthState>) -> Result<AuthSession, String> {
    auth.ensure_no_active_session()?;
    crate::secure_storage::clear_token()?;
    let session = AuthSession {
        user: AuthUser { id: format!("guest-{}", new_id()), login: "guest".into(), display_name: "Гость".into() },
        token: token(),
        remember_me: false,
    };
    activate_for_app(&app, &auth, &session, true)?;
    Ok(session)
}

#[tauri::command]
pub fn auth_logout(app: AppHandle, auth: State<'_, AuthState>, token: String) -> Result<(), String> {
    if !auth.matches_token(&token)? {
        return Err("токен не соответствует активной сессии".into());
    }
    let active = auth.current()?;
    let _ = crate::agent_commands::stop_agent();
    crate::agent_commands::сбросить_автоподтверждение();
    crate::tools::browser_cdp::закрыть();
    crate::chat::stop_generation()?;
    crate::voice::hotkey::снять();
    crate::voice::stt::выключить(&app.state::<crate::voice::Голос>());
    if let Some(sidecar) = app.try_state::<crate::model_manager::SidecarState>() {
        sidecar.stop();
    }
    if let Some(status) = app.try_state::<crate::chat::ModelState>() {
        if let Ok(mut current) = status.0.lock() {
            *current = None;
        }
    }
    crate::secure_storage::clear_token()?;
    let delete_result = database().and_then(|connection| delete_persisted_session(&connection, &token));
    auth.clear_token(&token)?;
    if active.guest {
        let roots = crate::user_data::roots_for(&active)?;
        let _ = std::fs::remove_dir_all(roots.root);
    }
    app.state::<crate::agent_commands::SettingsState>().clear_on_logout();
    crate::onboarding_commands::clear_transient_state();
    crate::agent_modes::set_mode(crate::agent_modes::AgentMode::Build);
    if let Err(error) = delete_result {
        tracing::warn!("не удалось удалить запись завершённой сессии: {error}");
    }
    Ok(())
}

#[tauri::command]
pub fn auth_session(app: AppHandle, auth: State<'_, AuthState>) -> Result<AuthSession, String> {
    if let Some(session) = auth.active_session()? {
        return Ok(session);
    }
    let token = match crate::secure_storage::load_token() {
        Ok(Some(token)) => token,
        Ok(None) => return Err("сохранённая сессия не найдена".into()),
        Err(error) => {
            let _ = crate::secure_storage::clear_token();
            return Err(error);
        }
    };
    let connection = database()?;
    let row: Option<(String, String, String, i64)> = connection
        .query_row(
            "SELECT u.id, u.login, u.display_name, s.remember_me
             FROM sessions s JOIN users u ON u.id = s.user_id
             WHERE s.token_hash = ?1 AND s.expires_at > datetime('now')",
            params![token_hash(&token)],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some((id, login, display_name, remember_me)) = row else {
        let _ = crate::secure_storage::clear_token();
        let _ = delete_persisted_session(&connection, &token);
        return Err("сессия недействительна".into());
    };
    if remember_me != 1 {
        let _ = crate::secure_storage::clear_token();
        return Err("для этой сессии не включён автоматический вход".into());
    }
    let session = AuthSession {
        user: AuthUser { id, login, display_name },
        token: token.clone(),
        remember_me: true,
    };
    if let Err(error) = activate_for_app(&app, &auth, &session, false) {
        let _ = crate::secure_storage::clear_token();
        let _ = delete_persisted_session(&connection, &session.token);
        return Err(error);
    }
    Ok(session)
}

#[tauri::command]
pub fn auth_remember_supported() -> bool {
    crate::secure_storage::supported()
}

#[tauri::command]
pub fn auth_profiles_list(auth: State<'_, AuthState>) -> Result<Vec<AuthProfileSummary>, String> {
    auth.ensure_no_active_session()?;
    let connection = database()?;
    let mut statement = connection
        .prepare("SELECT login, display_name FROM users ORDER BY login COLLATE NOCASE")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok(AuthProfileSummary {
                login: row.get(0)?,
                display_name: row.get(1)?,
            })
        })
        .map_err(|error| error.to_string())?;
    rows.map(|row| row.map_err(|error| error.to_string())).collect()
}

#[tauri::command]
pub fn auth_profile_delete(
    auth: State<'_, AuthState>,
    login: String,
    password: String,
) -> Result<AuthProfileDeleteResult, String> {
    auth.ensure_no_active_session()?;
    let mut connection = database()?;
    delete_profile_with_password(
        &mut connection,
        &crate::profile::config_dir(),
        &login,
        &password,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn password_hash_is_not_plaintext_and_verifies() {
        let hash = hash_password("correct horse battery staple").unwrap();
        assert_ne!(hash, "correct horse battery staple");
        assert!(verify_password("correct horse battery staple", &hash).unwrap());
        assert!(!verify_password("wrong password", &hash).unwrap());
    }

    #[test]
    fn login_is_normalized_and_validated() {
        assert_eq!(validate_login(" Alice_1 ").unwrap(), "alice_1");
        assert!(validate_login("а" ).is_err());
        assert!(validate_password("short").is_err());
    }

    #[test]
    fn auth_state_requires_session_and_clears_only_matching_token() {
        let _lock = TEST_LOCK.lock().unwrap();
        let state = AuthState::default();
        assert!(state.current().is_err());
        let session = AuthSession {
            user: AuthUser { id: "user-1".into(), login: "alice".into(), display_name: "Alice".into() },
            token: "opaque-token".into(),
            remember_me: false,
        };
        state.activate(&session, false).unwrap();
        assert_eq!(state.current().unwrap().user.id, "user-1");
        let shared_window_session = state.active_session().unwrap().unwrap();
        assert_eq!(shared_window_session.token, "opaque-token");
        assert_eq!(shared_window_session.user.id, "user-1");
        let other_session = AuthSession {
            user: AuthUser { id: "user-2".into(), login: "bob".into(), display_name: "Bob".into() },
            token: "other-session-token".into(),
            remember_me: false,
        };
        assert!(state.activate(&other_session, false).is_err());
        assert_eq!(state.current().unwrap().user.id, "user-1");
        assert!(state.clear_token("other-token").is_err());
        assert_eq!(state.current().unwrap().user.id, "user-1");
        state.clear_token("opaque-token").unwrap();
        assert!(state.current().is_err());
    }

    #[test]
    fn guest_is_marked_as_guest_in_authenticated_state() {
        let _lock = TEST_LOCK.lock().unwrap();
        let state = AuthState::default();
        let session = AuthSession {
            user: AuthUser { id: "guest-123".into(), login: "guest".into(), display_name: "Гость".into() },
            token: "guest-token".into(),
            remember_me: false,
        };
        state.activate(&session, true).unwrap();
        assert!(state.current().unwrap().guest);
        state.clear_token("guest-token").unwrap();
    }

    #[test]
    fn deleting_a_profile_requires_its_password_and_removes_only_private_data() {
        let base = std::env::temp_dir().join(format!(
            "arkha-profile-delete-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let app_root = base.join("ArkHa");
        let id = "ab".repeat(32);
        let private_root = app_root.join("users").join(&id);
        let shared_models = app_root.join("models");
        std::fs::create_dir_all(&private_root).unwrap();
        std::fs::create_dir_all(&shared_models).unwrap();
        std::fs::write(private_root.join("memory.db"), b"private").unwrap();
        std::fs::write(shared_models.join("model.gguf"), b"shared").unwrap();

        let mut connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(include_str!("auth_schema.sql")).unwrap();
        let password_hash = hash_password("correct password").unwrap();
        connection.execute(
            "INSERT INTO users (id, login, display_name, password_hash) VALUES (?1, ?2, ?3, ?4)",
            params![id, "alice", "Alice", password_hash],
        ).unwrap();

        assert!(delete_profile_with_password(&mut connection, &app_root, "alice", "wrong password").is_err());
        let count: i64 = connection.query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0)).unwrap();
        assert_eq!(count, 1);
        assert!(private_root.join("memory.db").is_file());

        let result = delete_profile_with_password(&mut connection, &app_root, "alice", "correct password").unwrap();
        assert_eq!(result.login, "alice");
        assert!(result.private_data_removed);
        let count: i64 = connection.query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0)).unwrap();
        assert_eq!(count, 0);
        assert!(!private_root.exists());
        assert!(shared_models.join("model.gguf").is_file());

        drop(connection);
        std::fs::remove_dir_all(base).unwrap();
    }
}
