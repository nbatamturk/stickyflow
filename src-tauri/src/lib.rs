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
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_window_state::{AppHandleExt, StateFlags, WindowExt};
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
                        created_at INTEGER NOT NULL,
                        updated_at INTEGER NOT NULL
                    );
                    PRAGMA user_version = 2;
                    ",
                )
                .map_err(|error| error.to_string())?;
        }
        1 => {
            connection
                .execute_batch(
                    "
                    BEGIN IMMEDIATE;
                    ALTER TABLE notes
                    ADD COLUMN sticky_open INTEGER NOT NULL DEFAULT 0;
                    PRAGMA user_version = 2;
                    COMMIT;
                    ",
                )
                .map_err(|error| error.to_string())?;
        }
        2 => {}
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
    // Save geometry before destroying decrypted sticky windows.
    // A state-save failure must not prevent the security lock.
    let _ = app.save_window_state(StateFlags::POSITION | StateFlags::SIZE);

    for (label, window) in app.webview_windows() {
        if label.starts_with("sticky-") {
            let _ = window.close();
        }
    }

    clear_master_key(&state)
}

fn sticky_window_label(id: &str) -> String {
    format!("sticky-{}", id.replace('-', ""))
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

fn show_sticky_window(app: &tauri::AppHandle, note: &Note) -> Result<(), String> {
    let label = sticky_window_label(&note.id);

    if let Some(window) = app.get_webview_window(&label) {
        window.show().map_err(|error| error.to_string())?;
        window
            .set_always_on_top(true)
            .map_err(|error| error.to_string())?;
        window.set_focus().map_err(|error| error.to_string())?;
        return Ok(());
    }

    let window = WebviewWindowBuilder::new(
        app,
        &label,
        WebviewUrl::App(format!("?sticky={}", note.id).into()),
    )
    .title(&note.title)
    .inner_size(360.0, 320.0)
    .min_inner_size(260.0, 180.0)
    .resizable(true)
    .always_on_top(true)
    .visible(false)
    .build()
    .map_err(|error| error.to_string())?;

    // Restore geometry only. Visibility is controlled by StickyFlow so
    // decrypted content can never appear before unlock.
    let _ = window.restore_state(StateFlags::POSITION | StateFlags::SIZE);

    window.show().map_err(|error| error.to_string())?;

    Ok(())
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

    // Save last known position/size before destroying the window.
    let _ = app.save_window_state(StateFlags::POSITION | StateFlags::SIZE);

    set_sticky_open(&app, &id, false)?;

    if let Some(window) = app.get_webview_window(&sticky_window_label(&id)) {
        let _ = window.close();
    }

    Ok(())
}

#[tauri::command]
fn save_sticky_window_state(app: tauri::AppHandle) -> Result<(), String> {
    app.save_window_state(StateFlags::POSITION | StateFlags::SIZE)
        .map_err(|error| error.to_string())
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

    Ok(Note {
        id: input.id,
        title: input.title,
        content: input.content,
        color: input.color,
        note_type: input.note_type,
        pinned: input.pinned,
        created_at,
        updated_at: now,
    })
}

#[tauri::command]
fn delete_note(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let _key = current_master_key(&state)?;
    let connection = open_database(&app)?;
    let window_label = sticky_window_label(&id);

    let affected = connection
        .execute("DELETE FROM notes WHERE id = ?1", params![&id])
        .map_err(|error| error.to_string())?;

    if affected == 0 {
        return Err("Note not found.".into());
    }

    if let Some(window) = app.get_webview_window(&window_label) {
        let _ = window.close();
    }

    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::default())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_opener::init())
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
            save_sticky_window_state,
            create_note,
            update_note,
            delete_note
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
