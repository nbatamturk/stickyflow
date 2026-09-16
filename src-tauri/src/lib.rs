use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use argon2::{
    password_hash::{
        rand_core::{OsRng, RngCore},
        PasswordHash, PasswordHasher, PasswordVerifier, SaltString,
    },
    Argon2,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
#[cfg(target_os = "linux")]
use gtk::prelude::*;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use tauri::{window::WindowBuilder, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_window_state::{AppHandleExt, StateFlags};
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

const MASTER_KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;
const KDF_SALT_LEN: usize = 16;

#[derive(Default)]
struct AppState {
    master_key: Mutex<Option<Zeroizing<Vec<u8>>>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct SecurityConfig {
    enabled: bool,
    password_hash: Option<String>,
    #[serde(default)]
    kdf_salt: Option<String>,
    #[serde(default)]
    wrapped_master_key: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SecurityStatus {
    configured: bool,
    enabled: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StickyPreferences {
    compact: bool,
    compact_x: Option<i64>,
    compact_y: Option<i64>,
    expanded_x: Option<i64>,
    expanded_y: Option<i64>,
    expanded_width: i64,
    expanded_height: i64,
    opacity: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Note {
    id: String,
    title: String,
    content: String,
    color: String,
    note_type: String,
    pinned: bool,
    created_at: i64,
    updated_at: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateNoteInput {
    title: String,
    content: String,
    color: String,
    note_type: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateNoteInput {
    id: String,
    title: String,
    content: String,
    color: String,
    note_type: String,
    pinned: bool,
}

fn app_data_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    fs::create_dir_all(&app_data_dir).map_err(|error| error.to_string())?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&app_data_dir, fs::Permissions::from_mode(0o700))
            .map_err(|error| error.to_string())?;
    }

    Ok(app_data_dir)
}

fn security_config_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(app_data_dir(app)?.join("security.json"))
}

fn local_key_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(app_data_dir(app)?.join("local.key"))
}

fn database_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(app_data_dir(app)?.join("stickyflow.db"))
}

fn read_security_config(app: &tauri::AppHandle) -> Result<Option<SecurityConfig>, String> {
    let path = security_config_path(app)?;

    if !path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let config = serde_json::from_str(&content).map_err(|error| error.to_string())?;
    Ok(Some(config))
}

fn write_security_config(app: &tauri::AppHandle, config: &SecurityConfig) -> Result<(), String> {
    let path = security_config_path(app)?;
    let content = serde_json::to_vec_pretty(config).map_err(|error| error.to_string())?;
    write_private_file(&path, &content)
}

fn write_private_file(path: &Path, contents: &[u8]) -> Result<(), String> {
    let temporary_path = path.with_extension("tmp");
    fs::write(&temporary_path, contents).map_err(|error| error.to_string())?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temporary_path, fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
    }

    fs::rename(&temporary_path, path).map_err(|error| error.to_string())
}

fn generate_random_bytes(length: usize) -> Vec<u8> {
    let mut bytes = vec![0_u8; length];
    OsRng.fill_bytes(&mut bytes);
    bytes
}

fn derive_kek(password: &str, salt: &[u8]) -> Result<Zeroizing<Vec<u8>>, String> {
    let mut key = Zeroizing::new(vec![0_u8; MASTER_KEY_LEN]);
    Argon2::default()
        .hash_password_into(password.as_bytes(), salt, key.as_mut_slice())
        .map_err(|error| error.to_string())?;
    Ok(key)
}

fn encrypt_bytes(key: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, String> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|error| error.to_string())?;
    let nonce_bytes = generate_random_bytes(NONCE_LEN);
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), plaintext)
        .map_err(|_| "Encryption failed.".to_string())?;

    let mut payload = nonce_bytes;
    payload.extend_from_slice(&ciphertext);
    Ok(payload)
}

fn decrypt_bytes(key: &[u8], payload: &[u8]) -> Result<Vec<u8>, String> {
    if payload.len() <= NONCE_LEN {
        return Err("Encrypted payload is invalid.".into());
    }

    let cipher = Aes256Gcm::new_from_slice(key).map_err(|error| error.to_string())?;
    let (nonce_bytes, ciphertext) = payload.split_at(NONCE_LEN);
    cipher
        .decrypt(Nonce::from_slice(nonce_bytes), ciphertext)
        .map_err(|_| "Unable to decrypt local data.".to_string())
}

fn encrypt_text(key: &[u8], plaintext: &str) -> Result<Vec<u8>, String> {
    encrypt_bytes(key, plaintext.as_bytes())
}

fn decrypt_text(key: &[u8], payload: &[u8]) -> Result<String, String> {
    let plaintext = decrypt_bytes(key, payload)?;
    String::from_utf8(plaintext).map_err(|error| error.to_string())
}

fn wrap_master_key(password: &str, master_key: &[u8]) -> Result<(String, String), String> {
    let salt = generate_random_bytes(KDF_SALT_LEN);
    let kek = derive_kek(password, &salt)?;
    let wrapped = encrypt_bytes(&kek, master_key)?;
    Ok((BASE64.encode(salt), BASE64.encode(wrapped)))
}

fn unwrap_master_key(password: &str, salt_b64: &str, wrapped_b64: &str) -> Result<Vec<u8>, String> {
    let salt = BASE64.decode(salt_b64).map_err(|error| error.to_string())?;
    let wrapped = BASE64
        .decode(wrapped_b64)
        .map_err(|error| error.to_string())?;
    let kek = derive_kek(password, &salt)?;
    let master_key = decrypt_bytes(&kek, &wrapped)?;

    if master_key.len() != MASTER_KEY_LEN {
        return Err("Stored encryption key has an invalid length.".into());
    }

    Ok(master_key)
}

fn set_master_key(state: &AppState, master_key: Vec<u8>) -> Result<(), String> {
    if master_key.len() != MASTER_KEY_LEN {
        return Err("Encryption key has an invalid length.".into());
    }

    let mut guard = state
        .master_key
        .lock()
        .map_err(|_| "Encryption state is unavailable.")?;
    *guard = Some(Zeroizing::new(master_key));
    Ok(())
}

fn clear_master_key(state: &AppState) -> Result<(), String> {
    let mut guard = state
        .master_key
        .lock()
        .map_err(|_| "Encryption state is unavailable.")?;
    *guard = None;
    Ok(())
}

fn current_master_key(state: &AppState) -> Result<Zeroizing<Vec<u8>>, String> {
    let guard = state
        .master_key
        .lock()
        .map_err(|_| "Encryption state is unavailable.")?;
    guard
        .as_ref()
        .cloned()
        .ok_or_else(|| "StickyFlow is locked. Unlock the application first.".to_string())
}

fn local_key_exists_or_database_is_empty(app: &tauri::AppHandle) -> Result<(), String> {
    let db_path = database_path(app)?;
    if !db_path.exists() {
        return Ok(());
    }

    let connection = Connection::open(db_path).map_err(|error| error.to_string())?;
    let table_exists: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='notes'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;

    if table_exists == 0 {
        return Ok(());
    }

    let count: i64 = connection
        .query_row("SELECT COUNT(*) FROM notes", [], |row| row.get(0))
        .map_err(|error| error.to_string())?;

    if count > 0 {
        Err("The local encryption key is missing, so existing notes cannot be decrypted.".into())
    } else {
        Ok(())
    }
}

fn ensure_local_master_key(app: &tauri::AppHandle, state: &AppState) -> Result<(), String> {
    let path = local_key_path(app)?;

    let key = if path.exists() {
        let key = fs::read(&path).map_err(|error| error.to_string())?;
        if key.len() != MASTER_KEY_LEN {
            return Err("The local encryption key file is invalid.".into());
        }
        key
    } else {
        local_key_exists_or_database_is_empty(app)?;
        let key = generate_random_bytes(MASTER_KEY_LEN);
        write_private_file(&path, &key)?;
        key
    };

    set_master_key(state, key)
}

fn open_database(app: &tauri::AppHandle) -> Result<Connection, String> {
    let path = database_path(app)?;
    let connection = Connection::open(&path).map_err(|error| error.to_string())?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
    }

    connection
        .busy_timeout(Duration::from_secs(3))
        .map_err(|error| error.to_string())?;

    connection
        .execute_batch(
            "
            PRAGMA journal_mode = WAL;
            PRAGMA foreign_keys = ON;
            ",
        )
        .map_err(|error| error.to_string())?;

    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|error| error.to_string())?;

    match version {
        0 => {
            connection
                .execute_batch(
                    "
                    CREATE TABLE IF NOT EXISTS notes (
                        id TEXT PRIMARY KEY,
                        title_cipher BLOB NOT NULL,
                        content_cipher BLOB NOT NULL,
                        color TEXT NOT NULL,
                        note_type TEXT NOT NULL,
                        pinned INTEGER NOT NULL DEFAULT 0,
                        sticky_open INTEGER NOT NULL DEFAULT 0,
                        sticky_compact INTEGER NOT NULL DEFAULT 0,
                        compact_x INTEGER,
                        compact_y INTEGER,
                        expanded_x INTEGER,
                        expanded_y INTEGER,
                        expanded_width INTEGER NOT NULL DEFAULT 360,
                        expanded_height INTEGER NOT NULL DEFAULT 320,
                        opacity INTEGER NOT NULL DEFAULT 100,
                        created_at INTEGER NOT NULL,
                        updated_at INTEGER NOT NULL
                    );
                    PRAGMA user_version = 5;
                    ",
                )
                .map_err(|error| error.to_string())?;
        }
        1 => {
            connection
                .execute_batch(
                    "
                    BEGIN IMMEDIATE;
                    ALTER TABLE notes ADD COLUMN sticky_open INTEGER NOT NULL DEFAULT 0;
                    ALTER TABLE notes ADD COLUMN sticky_compact INTEGER NOT NULL DEFAULT 0;
                    ALTER TABLE notes ADD COLUMN compact_x INTEGER;
                    ALTER TABLE notes ADD COLUMN compact_y INTEGER;
                    ALTER TABLE notes ADD COLUMN expanded_x INTEGER;
                    ALTER TABLE notes ADD COLUMN expanded_y INTEGER;
                    ALTER TABLE notes ADD COLUMN expanded_width INTEGER NOT NULL DEFAULT 360;
                    ALTER TABLE notes ADD COLUMN expanded_height INTEGER NOT NULL DEFAULT 320;
                    ALTER TABLE notes ADD COLUMN opacity INTEGER NOT NULL DEFAULT 100;
                    PRAGMA user_version = 5;
                    COMMIT;
                    ",
                )
                .map_err(|error| error.to_string())?;
        }
        2 => {
            connection
                .execute_batch(
                    "
                    BEGIN IMMEDIATE;
                    ALTER TABLE notes ADD COLUMN sticky_compact INTEGER NOT NULL DEFAULT 0;
                    ALTER TABLE notes ADD COLUMN compact_x INTEGER;
                    ALTER TABLE notes ADD COLUMN compact_y INTEGER;
                    ALTER TABLE notes ADD COLUMN expanded_x INTEGER;
                    ALTER TABLE notes ADD COLUMN expanded_y INTEGER;
                    ALTER TABLE notes ADD COLUMN expanded_width INTEGER NOT NULL DEFAULT 360;
                    ALTER TABLE notes ADD COLUMN expanded_height INTEGER NOT NULL DEFAULT 320;
                    ALTER TABLE notes ADD COLUMN opacity INTEGER NOT NULL DEFAULT 100;
                    PRAGMA user_version = 5;
                    COMMIT;
                    ",
                )
                .map_err(|error| error.to_string())?;
        }
        3 => {
            connection
                .execute_batch(
                    "
                    BEGIN IMMEDIATE;
                    ALTER TABLE notes ADD COLUMN compact_x INTEGER;
                    ALTER TABLE notes ADD COLUMN compact_y INTEGER;
                    ALTER TABLE notes ADD COLUMN expanded_x INTEGER;
                    ALTER TABLE notes ADD COLUMN expanded_y INTEGER;
                    ALTER TABLE notes ADD COLUMN opacity INTEGER NOT NULL DEFAULT 100;
                    PRAGMA user_version = 5;
                    COMMIT;
                    ",
                )
                .map_err(|error| error.to_string())?;
        }
        4 => {
            connection
                .execute_batch(
                    "
                    BEGIN IMMEDIATE;
                    ALTER TABLE notes
                    ADD COLUMN opacity INTEGER NOT NULL DEFAULT 100;
                    PRAGMA user_version = 5;
                    COMMIT;
                    ",
                )
                .map_err(|error| error.to_string())?;
        }
        5 => {}
        other => {
            return Err(format!(
                "Unsupported StickyFlow database schema version: {other}"
            ));
        }
    }

    Ok(connection)
}

fn now_millis() -> Result<i64, String> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?;
    i64::try_from(duration.as_millis()).map_err(|error| error.to_string())
}

fn validate_note_fields(
    title: &str,
    content: &str,
    color: &str,
    note_type: &str,
) -> Result<(), String> {
    if title.chars().count() > 200 {
        return Err("Note title cannot exceed 200 characters.".into());
    }
    if content.len() > 1_048_576 {
        return Err("Note content cannot exceed 1 MiB.".into());
    }
    if color.is_empty() || color.chars().count() > 32 {
        return Err("Note color is invalid.".into());
    }
    if !matches!(note_type, "note" | "snippet" | "todo") {
        return Err("Note type is invalid.".into());
    }
    Ok(())
}

#[tauri::command]
fn security_status(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<SecurityStatus, String> {
    let config = read_security_config(&app)?;

    match config {
        Some(config) => {
            if !config.enabled {
                ensure_local_master_key(&app, &state)?;
                restore_persisted_sticky_windows(&app, &state)?;
            }
            Ok(SecurityStatus {
                configured: true,
                enabled: config.enabled,
            })
        }
        None => Ok(SecurityStatus {
            configured: false,
            enabled: false,
        }),
    }
}

#[tauri::command]
fn setup_password(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    mut password: String,
) -> Result<(), String> {
    let result = (|| {
        if read_security_config(&app)?.is_some() {
            return Err("Security has already been configured.".into());
        }

        if password.chars().count() < 8 {
            return Err("Password must be at least 8 characters.".into());
        }

        let password_salt = SaltString::generate(&mut OsRng);
        let password_hash = Argon2::default()
            .hash_password(password.as_bytes(), &password_salt)
            .map_err(|error| error.to_string())?
            .to_string();

        let master_key = generate_random_bytes(MASTER_KEY_LEN);
        let (kdf_salt, wrapped_master_key) = wrap_master_key(&password, &master_key)?;

        write_security_config(
            &app,
            &SecurityConfig {
                enabled: true,
                password_hash: Some(password_hash),
                kdf_salt: Some(kdf_salt),
                wrapped_master_key: Some(wrapped_master_key),
            },
        )?;
        set_master_key(&state, master_key)
    })();

    password.zeroize();
    result
}

#[tauri::command]
fn skip_password_setup(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    if read_security_config(&app)?.is_some() {
        return Err("Security has already been configured.".into());
    }

    ensure_local_master_key(&app, &state)?;
    write_security_config(
        &app,
        &SecurityConfig {
            enabled: false,
            password_hash: None,
            kdf_salt: None,
            wrapped_master_key: None,
        },
    )
}

#[tauri::command]
fn verify_password(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    mut password: String,
) -> Result<bool, String> {
    let result = (|| {
        let Some(mut config) = read_security_config(&app)? else {
            return Ok(false);
        };

        if !config.enabled {
            ensure_local_master_key(&app, &state)?;
            return Ok(true);
        }

        let Some(stored_hash) = config.password_hash.as_deref() else {
            return Err("Password lock is enabled but no password hash is stored.".into());
        };

        let parsed_hash = PasswordHash::new(stored_hash).map_err(|error| error.to_string())?;
        if Argon2::default()
            .verify_password(password.as_bytes(), &parsed_hash)
            .is_err()
        {
            return Ok(false);
        }

        let master_key = match (
            config.kdf_salt.as_deref(),
            config.wrapped_master_key.as_deref(),
        ) {
            (Some(salt), Some(wrapped)) => unwrap_master_key(&password, salt, wrapped)?,
            _ => {
                // Migration path for v0.1 installations that had a password hash
                // before encrypted note storage existed.
                let master_key = generate_random_bytes(MASTER_KEY_LEN);
                let (salt, wrapped) = wrap_master_key(&password, &master_key)?;
                config.kdf_salt = Some(salt);
                config.wrapped_master_key = Some(wrapped);
                write_security_config(&app, &config)?;
                master_key
            }
        };

        set_master_key(&state, master_key)?;
        restore_persisted_sticky_windows(&app, &state)?;
        Ok(true)
    })();

    password.zeroize();
    result
}

#[tauri::command]
fn lock_session(app: tauri::AppHandle, state: tauri::State<'_, AppState>) -> Result<(), String> {
    // Persist our own per-mode geometry before closing any sticky window.
    // Do not depend only on frontend move/resize events: the user may move
    // a chip and immediately lock without ever expanding it.
    if let Ok(ids) = persisted_sticky_ids(&app) {
        for id in ids {
            let _ = save_window_geometry(&app, &id, "expanded");
            let _ = save_window_geometry(&app, &id, "chip");
        }
    }

    // Keep the Tauri window-state file as an additional fallback.
    // A state-save failure must never prevent the security lock.
    let _ = app.save_window_state(StateFlags::POSITION | StateFlags::SIZE);

    for (label, window) in app.windows() {
        if label.starts_with("sticky-") {
            let _ = window.close();
        }
    }

    clear_master_key(&state)
}

fn sticky_base_label(id: &str) -> String {
    format!("sticky-{}", id.replace('-', ""))
}

fn sticky_expanded_label(id: &str) -> String {
    format!("{}-expanded", sticky_base_label(id))
}

fn sticky_chip_label(id: &str) -> String {
    format!("{}-chip", sticky_base_label(id))
}

fn load_note_by_id(app: &tauri::AppHandle, state: &AppState, id: &str) -> Result<Note, String> {
    let key = current_master_key(state)?;
    let connection = open_database(app)?;

    let (title_cipher, content_cipher, color, note_type, pinned, created_at, updated_at): (
        Vec<u8>,
        Vec<u8>,
        String,
        String,
        i64,
        i64,
        i64,
    ) = connection
        .query_row(
            "SELECT title_cipher, content_cipher, color, note_type, pinned, created_at, updated_at
             FROM notes
             WHERE id = ?1",
            params![id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .map_err(|error| {
            if matches!(error, rusqlite::Error::QueryReturnedNoRows) {
                "Note not found.".to_string()
            } else {
                error.to_string()
            }
        })?;

    Ok(Note {
        id: id.to_string(),
        title: decrypt_text(&key, &title_cipher)?,
        content: decrypt_text(&key, &content_cipher)?,
        color,
        note_type,
        pinned: pinned != 0,
        created_at,
        updated_at,
    })
}

#[tauri::command]
fn get_note(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<Note, String> {
    load_note_by_id(&app, &state, &id)
}

fn load_sticky_preferences(app: &tauri::AppHandle, id: &str) -> Result<StickyPreferences, String> {
    let connection = open_database(app)?;

    connection
        .query_row(
            "SELECT
                sticky_compact,
                compact_x,
                compact_y,
                expanded_x,
                expanded_y,
                expanded_width,
                expanded_height,
                opacity
             FROM notes
             WHERE id = ?1",
            params![id],
            |row| {
                Ok(StickyPreferences {
                    compact: row.get::<_, i64>(0)? != 0,
                    compact_x: row.get(1)?,
                    compact_y: row.get(2)?,
                    expanded_x: row.get(3)?,
                    expanded_y: row.get(4)?,
                    expanded_width: row.get(5)?,
                    expanded_height: row.get(6)?,
                    opacity: row.get(7)?,
                })
            },
        )
        .map_err(|error| error.to_string())
}

fn configure_widget_window(window: &tauri::WebviewWindow) -> Result<(), String> {
    window
        .set_skip_taskbar(true)
        .map_err(|error| error.to_string())?;
    window
        .set_focusable(false)
        .map_err(|error| error.to_string())?;
    window
        .set_always_on_top(true)
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn apply_position(window: &tauri::WebviewWindow, x: Option<i64>, y: Option<i64>) {
    if let (Some(x), Some(y)) = (x, y) {
        let _ = window.set_position(tauri::LogicalPosition::new(x as f64, y as f64));
    }
}

fn apply_native_window_opacity(app: &tauri::AppHandle, label: &str, opacity: i64) {
    #[cfg(target_os = "linux")]
    {
        if let Some(window) = app.get_window(label) {
            if let Ok(gtk_window) = window.gtk_window() {
                gtk_window.set_opacity(opacity.clamp(40, 100) as f64 / 100.0);
            }
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = (app, label, opacity);
    }
}

#[cfg(target_os = "linux")]
fn install_expanded_opacity_map_handler(app: &tauri::AppHandle, note_id: &str, label: &str) {
    let Some(native_window) = app.get_window(label) else {
        return;
    };

    let Ok(gtk_window) = native_window.gtk_window() else {
        return;
    };

    let app_for_map = app.clone();
    let id_for_map = note_id.to_string();

    gtk_window.connect_map_event(move |window, _| {
        if let Ok(preferences) = load_sticky_preferences(&app_for_map, &id_for_map) {
            let alpha = preferences.opacity.clamp(40, 100) as f64 / 100.0;

            // Window is now genuinely mapped. Reapply the
            // persisted opacity after WebKitGTK/XWayland remap.
            window.set_opacity(alpha);

            if let Some(gdk_window) = window.window() {
                gdk_window.set_opaque_region(None);
                gdk_window.set_opacity(alpha);
            }

            // One more pass on the next GTK loop iteration.
            let window_retry = window.clone();

            gtk::glib::idle_add_local_once(move || {
                window_retry.set_opacity(alpha);

                if let Some(gdk_window) = window_retry.window() {
                    gdk_window.set_opaque_region(None);
                    gdk_window.set_opacity(alpha);
                }
            });
        }

        gtk::glib::Propagation::Proceed
    });
}

fn show_expanded_window(
    app: &tauri::AppHandle,
    note: &Note,
    preferences: &StickyPreferences,
) -> Result<(), String> {
    let label = sticky_expanded_label(&note.id);

    if let Some(window) = app.get_webview_window(&label) {
        configure_widget_window(&window)?;

        window
            .set_size(tauri::LogicalSize::new(
                preferences.expanded_width as f64,
                preferences.expanded_height as f64,
            ))
            .map_err(|error| error.to_string())?;

        apply_position(&window, preferences.expanded_x, preferences.expanded_y);

        window.show().map_err(|error| error.to_string())?;

        // Apply real native window opacity after the window is mapped.
        apply_native_window_opacity(app, &label, preferences.opacity);

        return Ok(());
    }

    let window = WebviewWindowBuilder::new(
        app,
        &label,
        WebviewUrl::App(format!("?sticky={}&mode=expanded", note.id).into()),
    )
    .title(&note.title)
    .inner_size(
        preferences.expanded_width as f64,
        preferences.expanded_height as f64,
    )
    .min_inner_size(260.0, 180.0)
    .resizable(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .focused(false)
    .focusable(false)
    .visible(false)
    .build()
    .map_err(|error| error.to_string())?;

    #[cfg(target_os = "linux")]
    install_expanded_opacity_map_handler(app, &note.id, &label);

    apply_position(&window, preferences.expanded_x, preferences.expanded_y);

    window.show().map_err(|error| error.to_string())?;

    // WebView opacity must be applied at the native GTK window level.
    apply_native_window_opacity(app, &label, preferences.opacity);

    Ok(())
}

const CHIP_DEFAULT_HEIGHT: i32 = 26;
const CHIP_DEFAULT_MIN_WIDTH: i32 = 60;
const CHIP_DEFAULT_MAX_WIDTH: i32 = 180;
const CHIP_DEFAULT_FIXED_WIDTH: i32 = 90;
const CHIP_DEFAULT_FONT_SIZE: i32 = 11;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChipSettings {
    auto_width: bool,
    fixed_width: i32,
    height: i32,
    min_width: i32,
    max_width: i32,
    font_size: i32,
}

impl Default for ChipSettings {
    fn default() -> Self {
        Self {
            auto_width: true,
            fixed_width: CHIP_DEFAULT_FIXED_WIDTH,
            height: CHIP_DEFAULT_HEIGHT,
            min_width: CHIP_DEFAULT_MIN_WIDTH,
            max_width: CHIP_DEFAULT_MAX_WIDTH,
            font_size: CHIP_DEFAULT_FONT_SIZE,
        }
    }
}

fn normalize_chip_settings(mut settings: ChipSettings) -> ChipSettings {
    settings.height = settings.height.clamp(20, 64);
    settings.min_width = settings.min_width.clamp(48, 480);
    settings.max_width = settings.max_width.clamp(settings.min_width, 640);

    settings.fixed_width = settings
        .fixed_width
        .clamp(settings.min_width, settings.max_width);

    settings.font_size = settings.font_size.clamp(8, 24);

    settings
}

fn ensure_app_settings_table(connection: &rusqlite::Connection) -> Result<(), String> {
    connection
        .execute_batch(
            "
            CREATE TABLE IF NOT EXISTS app_settings (
                key TEXT PRIMARY KEY NOT NULL,
                value TEXT NOT NULL
            );
            ",
        )
        .map_err(|error| error.to_string())
}

fn read_app_setting(connection: &rusqlite::Connection, key: &str) -> Option<String> {
    connection
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .ok()
}

fn write_app_setting(
    connection: &rusqlite::Connection,
    key: &str,
    value: &str,
) -> Result<(), String> {
    connection
        .execute(
            "
            INSERT INTO app_settings (key, value)
            VALUES (?1, ?2)
            ON CONFLICT(key)
            DO UPDATE SET value = excluded.value
            ",
            params![key, value],
        )
        .map_err(|error| error.to_string())?;

    Ok(())
}

fn load_chip_settings(app: &tauri::AppHandle) -> Result<ChipSettings, String> {
    let connection = open_database(app)?;
    ensure_app_settings_table(&connection)?;

    let defaults = ChipSettings::default();

    let auto_width = read_app_setting(&connection, "chip_auto_width")
        .and_then(|value| value.parse::<bool>().ok())
        .unwrap_or(defaults.auto_width);

    let fixed_width = read_app_setting(&connection, "chip_fixed_width")
        .and_then(|value| value.parse::<i32>().ok())
        .unwrap_or(defaults.fixed_width);

    let height = read_app_setting(&connection, "chip_height")
        .and_then(|value| value.parse::<i32>().ok())
        .unwrap_or(defaults.height);

    let min_width = read_app_setting(&connection, "chip_min_width")
        .and_then(|value| value.parse::<i32>().ok())
        .unwrap_or(defaults.min_width);

    let max_width = read_app_setting(&connection, "chip_max_width")
        .and_then(|value| value.parse::<i32>().ok())
        .unwrap_or(defaults.max_width);

    let font_size = read_app_setting(&connection, "chip_font_size")
        .and_then(|value| value.parse::<i32>().ok())
        .unwrap_or(defaults.font_size);

    Ok(normalize_chip_settings(ChipSettings {
        auto_width,
        fixed_width,
        height,
        min_width,
        max_width,
        font_size,
    }))
}

fn persist_chip_settings(app: &tauri::AppHandle, settings: &ChipSettings) -> Result<(), String> {
    let connection = open_database(app)?;
    ensure_app_settings_table(&connection)?;

    write_app_setting(
        &connection,
        "chip_auto_width",
        &settings.auto_width.to_string(),
    )?;

    write_app_setting(
        &connection,
        "chip_fixed_width",
        &settings.fixed_width.to_string(),
    )?;

    write_app_setting(&connection, "chip_height", &settings.height.to_string())?;

    write_app_setting(
        &connection,
        "chip_min_width",
        &settings.min_width.to_string(),
    )?;

    write_app_setting(
        &connection,
        "chip_max_width",
        &settings.max_width.to_string(),
    )?;

    write_app_setting(
        &connection,
        "chip_font_size",
        &settings.font_size.to_string(),
    )?;

    Ok(())
}

fn compact_chip_title(title: &str) -> String {
    let source = title.trim();

    let source = if source.is_empty() {
        "Untitled"
    } else {
        source
    };

    let mut chars = source.chars();
    let short: String = chars.by_ref().take(22).collect();

    if chars.next().is_some() {
        format!("{short}…")
    } else {
        short
    }
}

#[cfg(target_os = "linux")]
fn install_chip_css(settings: &ChipSettings) -> Result<(), String> {
    let provider = gtk::CssProvider::new();

    let css = format!(
        "
        #stickyflow-chip-root {{
            background-color: #fff1a8;
            border-radius: 7px;
        }}

        #stickyflow-chip-drag {{
            color: #756c46;
            font-size: {}px;
        }}

        #stickyflow-chip-title {{
            color: #29261e;
            font-size: {}px;
            font-weight: 600;
        }}

        #stickyflow-chip-arrow {{
            color: #756c46;
            font-size: {}px;
        }}
        ",
        settings.font_size,
        settings.font_size,
        settings.font_size + 1,
    );

    provider
        .load_from_data(css.as_bytes())
        .map_err(|error| error.to_string())?;

    if let Some(screen) = gtk::gdk::Screen::default() {
        gtk::StyleContext::add_provider_for_screen(
            &screen,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn calculate_chip_width(title: &str, settings: &ChipSettings) -> i32 {
    if !settings.auto_width {
        return settings
            .fixed_width
            .clamp(settings.min_width, settings.max_width);
    }

    let display_title = compact_chip_title(title);

    let probe = gtk::Label::new(Some(&display_title));
    probe.set_widget_name("stickyflow-chip-title");
    probe.show();

    let (_, natural_width) = probe.preferred_width();

    (natural_width + 42).clamp(settings.min_width, settings.max_width)
}

#[cfg(target_os = "linux")]
fn update_named_gtk_label(widget: &gtk::Widget, target_name: &str, text: &str) -> bool {
    if widget.widget_name().as_str() == target_name {
        if let Ok(label) = widget.clone().downcast::<gtk::Label>() {
            label.set_text(text);
            return true;
        }
    }

    if let Ok(container) = widget.clone().downcast::<gtk::Container>() {
        for child in container.children() {
            if update_named_gtk_label(&child, target_name, text) {
                return true;
            }
        }
    }

    false
}

#[cfg(target_os = "linux")]
fn show_chip_window(
    app: &tauri::AppHandle,
    note: &Note,
    preferences: &StickyPreferences,
) -> Result<(), String> {
    let label = sticky_chip_label(&note.id);

    let settings = load_chip_settings(app)?;
    install_chip_css(&settings)?;

    let chip_width = calculate_chip_width(&note.title, &settings);

    // Existing native chip: reuse the same native window.
    // Refresh its GTK title in-place so Quick Edit changes are reflected
    // without closing/recreating the raw Tauri window.
    if let Some(window) = app.get_window(&label) {
        let display_title = compact_chip_title(&note.title);

        if let Ok(default_vbox) = window.default_vbox() {
            for child in default_vbox.children() {
                if update_named_gtk_label(&child, "stickyflow-chip-title", &display_title) {
                    break;
                }
            }
        }

        let _ = window.set_title(&note.title);

        window
            .set_size(tauri::LogicalSize::new(
                chip_width as f64,
                settings.height as f64,
            ))
            .map_err(|error| error.to_string())?;

        if let Ok(gtk_window) = window.gtk_window() {
            gtk_window.resize(chip_width, settings.height);
            gtk_window.set_opacity(preferences.opacity.clamp(40, 100) as f64 / 100.0);
        }

        if let (Some(x), Some(y)) = (preferences.compact_x, preferences.compact_y) {
            let _ = window.set_position(tauri::LogicalPosition::new(x as f64, y as f64));
        }

        window.show().map_err(|error| error.to_string())?;

        // Re-apply after mapping; Mutter/XWayland can otherwise retain
        // the previous native opacity state.
        apply_native_window_opacity(app, &label, preferences.opacity);

        return Ok(());
    }

    let builder_min_width = if settings.auto_width {
        settings.min_width
    } else {
        chip_width
    };

    let builder_max_width = if settings.auto_width {
        settings.max_width
    } else {
        chip_width
    };

    // Raw Tauri Window: no WebKitGTK / 200x200 WebView minimum.
    let window = WindowBuilder::new(app, &label)
        .title(&note.title)
        .inner_size(chip_width as f64, settings.height as f64)
        .min_inner_size(builder_min_width as f64, settings.height as f64)
        .max_inner_size(builder_max_width as f64, settings.height as f64)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .focusable(false)
        .visible(false)
        .build()
        .map_err(|error| error.to_string())?;

    let default_vbox = window.default_vbox().map_err(|error| error.to_string())?;

    let gtk_window = window.gtk_window().map_err(|error| error.to_string())?;

    gtk_window.set_opacity(preferences.opacity.clamp(40, 100) as f64 / 100.0);

    let root_event = gtk::EventBox::new();
    root_event.set_visible_window(true);
    root_event.set_widget_name("stickyflow-chip-root");

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);

    // Dedicated draggable area.
    let drag_event = gtk::EventBox::new();
    drag_event.set_visible_window(false);
    drag_event.set_size_request(16, settings.height);

    drag_event.add_events(gtk::gdk::EventMask::BUTTON_PRESS_MASK);

    let drag_label = gtk::Label::new(Some("⋮"));
    drag_label.set_widget_name("stickyflow-chip-drag");
    drag_label.set_xalign(0.5);
    drag_event.add(&drag_label);

    // Clickable body opens expanded sticky.
    let body_event = gtk::EventBox::new();
    body_event.set_visible_window(false);

    body_event.add_events(gtk::gdk::EventMask::BUTTON_PRESS_MASK);

    let body = gtk::Box::new(gtk::Orientation::Horizontal, 3);

    let display_title = compact_chip_title(&note.title);

    let title_label = gtk::Label::new(Some(&display_title));
    title_label.set_widget_name("stickyflow-chip-title");
    title_label.set_xalign(0.0);

    let arrow = gtk::Label::new(Some("›"));
    arrow.set_widget_name("stickyflow-chip-arrow");

    body.pack_start(&title_label, true, true, 5);

    body.pack_end(&arrow, false, false, 5);

    body_event.add(&body);

    row.pack_start(&drag_event, false, false, 0);

    row.pack_start(&body_event, true, true, 0);

    root_event.add(&row);

    default_vbox.pack_start(&root_event, true, true, 0);

    root_event.show_all();

    window
        .set_size(tauri::LogicalSize::new(
            chip_width as f64,
            settings.height as f64,
        ))
        .map_err(|error| error.to_string())?;

    gtk_window.resize(chip_width, settings.height);

    if let (Some(x), Some(y)) = (preferences.compact_x, preferences.compact_y) {
        window
            .set_position(tauri::LogicalPosition::new(x as f64, y as f64))
            .map_err(|error| error.to_string())?;
    }

    let window_for_drag = window.clone();

    drag_event.connect_button_press_event(move |_, event| {
        if event.button() == 1 {
            let _ = window_for_drag.start_dragging();
        }

        gtk::glib::Propagation::Stop
    });

    let app_for_expand = app.clone();
    let id_for_expand = note.id.clone();

    body_event.connect_button_press_event(move |_, event| {
        if event.button() == 1 {
            let state = app_for_expand.state::<AppState>();

            let _ = set_sticky_compact_inner(&app_for_expand, state.inner(), &id_for_expand, false);
        }

        gtk::glib::Propagation::Stop
    });

    window.show().map_err(|error| error.to_string())?;

    gtk_window.resize(chip_width, settings.height);

    apply_native_window_opacity(app, &label, preferences.opacity);

    if let (Some(x), Some(y)) = (preferences.compact_x, preferences.compact_y) {
        let _ = window.set_position(tauri::LogicalPosition::new(x as f64, y as f64));
    }

    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn show_chip_window(
    app: &tauri::AppHandle,
    note: &Note,
    preferences: &StickyPreferences,
) -> Result<(), String> {
    let label = sticky_chip_label(&note.id);

    if let Some(window) = app.get_webview_window(&label) {
        if let (Some(x), Some(y)) = (preferences.compact_x, preferences.compact_y) {
            let _ = window.set_position(tauri::LogicalPosition::new(x as f64, y as f64));
        }

        window.show().map_err(|error| error.to_string())?;
        return Ok(());
    }

    let window = WebviewWindowBuilder::new(
        app,
        &label,
        WebviewUrl::App(format!("?sticky={}&mode=chip", note.id).into()),
    )
    .title(&note.title)
    .inner_size(90.0, 26.0)
    .resizable(false)
    .decorations(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .focused(false)
    .focusable(false)
    .visible(false)
    .build()
    .map_err(|error| error.to_string())?;

    if let (Some(x), Some(y)) = (preferences.compact_x, preferences.compact_y) {
        let _ = window.set_position(tauri::LogicalPosition::new(x as f64, y as f64));
    }

    window.show().map_err(|error| error.to_string())?;
    Ok(())
}

fn show_sticky_window(app: &tauri::AppHandle, note: &Note) -> Result<(), String> {
    let preferences = load_sticky_preferences(app, &note.id)?;

    if preferences.compact {
        show_chip_window(app, note, &preferences)
    } else {
        show_expanded_window(app, note, &preferences)
    }
}

fn save_window_geometry(app: &tauri::AppHandle, id: &str, mode: &str) -> Result<(), String> {
    let label = match mode {
        "chip" => sticky_chip_label(id),
        "expanded" => sticky_expanded_label(id),
        _ => return Err("Invalid sticky mode.".into()),
    };

    let Some(window) = app.get_window(&label) else {
        return Ok(());
    };

    let scale = window.scale_factor().map_err(|error| error.to_string())?;

    let physical_position = window.outer_position().map_err(|error| error.to_string())?;

    let position = physical_position.to_logical::<f64>(scale);

    let connection = open_database(app)?;

    if mode == "chip" {
        connection
            .execute(
                "UPDATE notes
                 SET compact_x = ?2,
                     compact_y = ?3
                 WHERE id = ?1",
                params![id, position.x.round() as i64, position.y.round() as i64,],
            )
            .map_err(|error| error.to_string())?;
    } else {
        let physical_size = window.inner_size().map_err(|error| error.to_string())?;

        let size = physical_size.to_logical::<f64>(scale);

        connection
            .execute(
                "UPDATE notes
                 SET expanded_x = ?2,
                     expanded_y = ?3,
                     expanded_width = ?4,
                     expanded_height = ?5
                 WHERE id = ?1",
                params![
                    id,
                    position.x.round() as i64,
                    position.y.round() as i64,
                    size.width.round().max(260.0) as i64,
                    size.height.round().max(180.0) as i64,
                ],
            )
            .map_err(|error| error.to_string())?;
    }

    Ok(())
}

#[tauri::command]
fn get_chip_settings(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<ChipSettings, String> {
    let _key = current_master_key(&state)?;
    load_chip_settings(&app)
}

fn refresh_visible_compact_chips(app: &tauri::AppHandle, state: &AppState) -> Result<(), String> {
    for id in persisted_sticky_ids(app)? {
        let preferences = load_sticky_preferences(app, &id)?;

        if !preferences.compact {
            continue;
        }

        if let Ok(note) = load_note_by_id(app, state, &id) {
            let _ = show_chip_window(app, &note, &preferences);
        }
    }

    Ok(())
}

#[tauri::command]
fn save_chip_settings(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    input: ChipSettings,
) -> Result<ChipSettings, String> {
    let _key = current_master_key(&state)?;

    let settings = normalize_chip_settings(input);

    persist_chip_settings(&app, &settings)?;

    refresh_visible_compact_chips(&app, state.inner())?;

    Ok(settings)
}

#[tauri::command]
fn reset_chip_settings(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<ChipSettings, String> {
    let _key = current_master_key(&state)?;

    let settings = ChipSettings::default();

    persist_chip_settings(&app, &settings)?;

    refresh_visible_compact_chips(&app, state.inner())?;

    Ok(settings)
}

#[tauri::command]
fn save_sticky_geometry(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    id: String,
    mode: String,
) -> Result<(), String> {
    let _key = current_master_key(&state)?;
    save_window_geometry(&app, &id, &mode)
}

#[tauri::command]
fn get_sticky_opacity(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<i64, String> {
    let _key = current_master_key(&state)?;
    let connection = open_database(&app)?;

    let opacity = connection
        .query_row(
            "SELECT opacity FROM notes WHERE id = ?1",
            params![id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| {
            if matches!(error, rusqlite::Error::QueryReturnedNoRows) {
                "Note not found.".to_string()
            } else {
                error.to_string()
            }
        })?;

    Ok(opacity.clamp(40, 100))
}

#[tauri::command]
fn set_sticky_opacity(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    id: String,
    opacity: i64,
) -> Result<i64, String> {
    let _key = current_master_key(&state)?;
    let opacity = opacity.clamp(40, 100);
    let connection = open_database(&app)?;

    let affected = connection
        .execute(
            "UPDATE notes
             SET opacity = ?2
             WHERE id = ?1",
            params![&id, opacity],
        )
        .map_err(|error| error.to_string())?;

    if affected == 0 {
        return Err("Note not found.".into());
    }

    // Apply the value to both companion windows immediately.
    // Expanded is a WebViewWindow, chip is a raw native Window,
    // but both have an underlying GTK toplevel on Linux.
    apply_native_window_opacity(&app, &sticky_expanded_label(&id), opacity);

    apply_native_window_opacity(&app, &sticky_chip_label(&id), opacity);

    Ok(opacity)
}

fn set_sticky_compact_inner(
    app: &tauri::AppHandle,
    state: &AppState,
    id: &str,
    compact: bool,
) -> Result<(), String> {
    let _key = current_master_key(state)?;
    let note = load_note_by_id(app, state, id)?;

    if compact {
        // Preserve expanded geometry first.
        let _ = save_window_geometry(app, id, "expanded");

        let connection = open_database(app)?;

        connection
            .execute(
                "UPDATE notes
                 SET sticky_compact = 1,
                     compact_x = COALESCE(compact_x, expanded_x),
                     compact_y = COALESCE(compact_y, expanded_y)
                 WHERE id = ?1",
                params![id],
            )
            .map_err(|error| error.to_string())?;

        let preferences = load_sticky_preferences(app, id)?;

        // IMPORTANT:
        // Do NOT hide expanded until the native chip is definitely visible.
        match show_chip_window(app, &note, &preferences) {
            Ok(()) => {
                if let Some(window) = app.get_window(&sticky_expanded_label(id)) {
                    let _ = window.hide();
                }

                eprintln!("[stickyflow] compact transition OK: {}", id);

                Ok(())
            }

            Err(error) => {
                eprintln!(
                    "[stickyflow] compact transition FAILED for {}: {}",
                    id, error
                );

                // Roll back persisted mode because expanded is still visible.
                let _ = connection.execute(
                    "UPDATE notes
                     SET sticky_compact = 0
                     WHERE id = ?1",
                    params![id],
                );

                Err(format!("Failed to show compact chip: {error}"))
            }
        }
    } else {
        // Preserve compact geometry first.
        let _ = save_window_geometry(app, id, "chip");

        let connection = open_database(app)?;

        connection
            .execute(
                "UPDATE notes
                 SET sticky_compact = 0,
                     expanded_x = COALESCE(expanded_x, compact_x),
                     expanded_y = COALESCE(expanded_y, compact_y)
                 WHERE id = ?1",
                params![id],
            )
            .map_err(|error| error.to_string())?;

        let preferences = load_sticky_preferences(app, id)?;

        // Same rule in the opposite direction:
        // don't hide chip until expanded is confirmed visible.
        match show_expanded_window(app, &note, &preferences) {
            Ok(()) => {
                if let Some(window) = app.get_window(&sticky_chip_label(id)) {
                    let _ = window.hide();
                }

                eprintln!("[stickyflow] expanded transition OK: {}", id);

                Ok(())
            }

            Err(error) => {
                eprintln!(
                    "[stickyflow] expanded transition FAILED for {}: {}",
                    id, error
                );

                // Roll back persisted mode because chip is still visible.
                let _ = connection.execute(
                    "UPDATE notes
                     SET sticky_compact = 1
                     WHERE id = ?1",
                    params![id],
                );

                Err(format!("Failed to show expanded sticky: {error}"))
            }
        }
    }
}

#[tauri::command]
fn set_sticky_compact(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    id: String,
    compact: bool,
) -> Result<(), String> {
    set_sticky_compact_inner(&app, state.inner(), &id, compact)
}

fn set_sticky_open(app: &tauri::AppHandle, id: &str, open: bool) -> Result<(), String> {
    let connection = open_database(app)?;

    let affected = connection
        .execute(
            "UPDATE notes SET sticky_open = ?2 WHERE id = ?1",
            params![id, if open { 1 } else { 0 }],
        )
        .map_err(|error| error.to_string())?;

    if affected == 0 {
        return Err("Note not found.".into());
    }

    Ok(())
}

fn persisted_sticky_ids(app: &tauri::AppHandle) -> Result<Vec<String>, String> {
    let connection = open_database(app)?;

    let mut statement = connection
        .prepare(
            "SELECT id
             FROM notes
             WHERE sticky_open = 1
             ORDER BY updated_at DESC",
        )
        .map_err(|error| error.to_string())?;

    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?;

    let mut ids = Vec::new();

    for row in rows {
        ids.push(row.map_err(|error| error.to_string())?);
    }

    Ok(ids)
}

fn restore_persisted_sticky_windows(
    app: &tauri::AppHandle,
    state: &AppState,
) -> Result<(), String> {
    for id in persisted_sticky_ids(app)? {
        if let Ok(note) = load_note_by_id(app, state, &id) {
            let _ = show_sticky_window(app, &note);
        }
    }

    Ok(())
}

#[tauri::command]
fn open_sticky_window(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let note = load_note_by_id(&app, &state, &id)?;
    show_sticky_window(&app, &note)?;
    set_sticky_open(&app, &id, true)
}

#[tauri::command]
fn close_sticky_window(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let _key = current_master_key(&state)?;

    let _ = save_window_geometry(&app, &id, "expanded");
    let _ = save_window_geometry(&app, &id, "chip");

    set_sticky_open(&app, &id, false)?;

    if let Some(window) = app.get_webview_window(&sticky_expanded_label(&id)) {
        let _ = window.close();
    }

    if let Some(window) = app.get_window(&sticky_chip_label(&id)) {
        let _ = window.close();
    }

    Ok(())
}

#[tauri::command]
fn list_notes(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Note>, String> {
    let key = current_master_key(&state)?;
    let connection = open_database(&app)?;
    let mut statement = connection
        .prepare(
            "SELECT id, title_cipher, content_cipher, color, note_type, pinned, created_at, updated_at
             FROM notes
             ORDER BY pinned DESC, updated_at DESC",
        )
        .map_err(|error| error.to_string())?;
    let mut rows = statement.query([]).map_err(|error| error.to_string())?;
    let mut notes = Vec::new();

    while let Some(row) = rows.next().map_err(|error| error.to_string())? {
        let title_cipher: Vec<u8> = row.get(1).map_err(|error| error.to_string())?;
        let content_cipher: Vec<u8> = row.get(2).map_err(|error| error.to_string())?;
        notes.push(Note {
            id: row.get(0).map_err(|error| error.to_string())?,
            title: decrypt_text(&key, &title_cipher)?,
            content: decrypt_text(&key, &content_cipher)?,
            color: row.get(3).map_err(|error| error.to_string())?,
            note_type: row.get(4).map_err(|error| error.to_string())?,
            pinned: row.get::<_, i64>(5).map_err(|error| error.to_string())? != 0,
            created_at: row.get(6).map_err(|error| error.to_string())?,
            updated_at: row.get(7).map_err(|error| error.to_string())?,
        });
    }

    Ok(notes)
}

#[tauri::command]
fn create_note(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    input: CreateNoteInput,
) -> Result<Note, String> {
    validate_note_fields(&input.title, &input.content, &input.color, &input.note_type)?;
    let key = current_master_key(&state)?;
    let connection = open_database(&app)?;
    let id = Uuid::new_v4().to_string();
    let now = now_millis()?;
    let title_cipher = encrypt_text(&key, &input.title)?;
    let content_cipher = encrypt_text(&key, &input.content)?;

    connection
        .execute(
            "INSERT INTO notes
             (id, title_cipher, content_cipher, color, note_type, pinned, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?6)",
            params![
                id,
                title_cipher,
                content_cipher,
                input.color,
                input.note_type,
                now
            ],
        )
        .map_err(|error| error.to_string())?;

    Ok(Note {
        id,
        title: input.title,
        content: input.content,
        color: input.color,
        note_type: input.note_type,
        pinned: false,
        created_at: now,
        updated_at: now,
    })
}

#[tauri::command]
fn update_note(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    input: UpdateNoteInput,
) -> Result<Note, String> {
    validate_note_fields(&input.title, &input.content, &input.color, &input.note_type)?;
    let key = current_master_key(&state)?;
    let connection = open_database(&app)?;
    let now = now_millis()?;
    let title_cipher = encrypt_text(&key, &input.title)?;
    let content_cipher = encrypt_text(&key, &input.content)?;

    let affected = connection
        .execute(
            "UPDATE notes
             SET title_cipher = ?2,
                 content_cipher = ?3,
                 color = ?4,
                 note_type = ?5,
                 pinned = ?6,
                 updated_at = ?7
             WHERE id = ?1",
            params![
                input.id,
                title_cipher,
                content_cipher,
                input.color,
                input.note_type,
                if input.pinned { 1 } else { 0 },
                now
            ],
        )
        .map_err(|error| error.to_string())?;

    if affected == 0 {
        return Err("Note not found.".into());
    }

    let created_at: i64 = connection
        .query_row(
            "SELECT created_at FROM notes WHERE id = ?1",
            params![input.id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;

    let note = Note {
        id: input.id,
        title: input.title,
        content: input.content,
        color: input.color,
        note_type: input.note_type,
        pinned: input.pinned,
        created_at,
        updated_at: now,
    };

    // If a native compact chip is currently alive, refresh its
    // title/size immediately without opening a closed sticky.
    let chip_label = sticky_chip_label(&note.id);

    if app.get_window(&chip_label).is_some() {
        if let Ok(preferences) = load_sticky_preferences(&app, &note.id) {
            let _ = show_chip_window(&app, &note, &preferences);
        }
    }

    // Only the note id crosses the event bus. Each UI retrieves
    // the current decrypted value through the normal guarded command.
    if let Err(error) = app.emit("stickyflow-note-updated", note.id.clone()) {
        eprintln!("[stickyflow] note update event failed: {}", error);
    }

    Ok(note)
}

#[tauri::command]
fn delete_note(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let _key = current_master_key(&state)?;
    let connection = open_database(&app)?;
    let expanded_label = sticky_expanded_label(&id);
    let chip_label = sticky_chip_label(&id);

    let affected = connection
        .execute("DELETE FROM notes WHERE id = ?1", params![&id])
        .map_err(|error| error.to_string())?;

    if affected == 0 {
        return Err("Note not found.".into());
    }

    if let Some(window) = app.get_webview_window(&expanded_label) {
        let _ = window.close();
    }

    if let Some(window) = app.get_window(&chip_label) {
        let _ = window.close();
    }

    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::default())
        .plugin(
            tauri_plugin_window_state::Builder::default()
                // StickyFlow persists sticky/chip geometry in SQLite.
                // Do not let the window-state plugin restore stale sizes
                // or positions for sticky companion windows.
                .with_filter(|label| !label.starts_with("sticky-"))
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .invoke_handler(tauri::generate_handler![
            security_status,
            setup_password,
            skip_password_setup,
            verify_password,
            lock_session,
            list_notes,
            get_note,
            open_sticky_window,
            close_sticky_window,
            set_sticky_compact,
            get_chip_settings,
            save_chip_settings,
            reset_chip_settings,
            get_sticky_opacity,
            set_sticky_opacity,
            save_sticky_geometry,
            create_note,
            update_note,
            delete_note
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
