use argon2::{
    password_hash::{
        rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString,
    },
    Argon2,
};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};
use tauri::Manager;

#[derive(Debug, Serialize, Deserialize)]
struct SecurityConfig {
    enabled: bool,
    password_hash: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SecurityStatus {
    configured: bool,
    enabled: bool,
}

fn security_config_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let app_data_dir = app.path().app_data_dir().map_err(|error| error.to_string())?;
    fs::create_dir_all(&app_data_dir).map_err(|error| error.to_string())?;
    Ok(app_data_dir.join("security.json"))
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
    let content = serde_json::to_string_pretty(config).map_err(|error| error.to_string())?;
    fs::write(path, content).map_err(|error| error.to_string())
}

#[tauri::command]
fn security_status(app: tauri::AppHandle) -> Result<SecurityStatus, String> {
    let config = read_security_config(&app)?;

    Ok(match config {
        Some(config) => SecurityStatus {
            configured: true,
            enabled: config.enabled,
        },
        None => SecurityStatus {
            configured: false,
            enabled: false,
        },
    })
}

#[tauri::command]
fn setup_password(app: tauri::AppHandle, password: String) -> Result<(), String> {
    if read_security_config(&app)?.is_some() {
        return Err("Security has already been configured.".into());
    }

    if password.chars().count() < 8 {
        return Err("Password must be at least 8 characters.".into());
    }

    let salt = SaltString::generate(&mut OsRng);
    let password_hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|error| error.to_string())?
        .to_string();

    write_security_config(
        &app,
        &SecurityConfig {
            enabled: true,
            password_hash: Some(password_hash),
        },
    )
}

#[tauri::command]
fn skip_password_setup(app: tauri::AppHandle) -> Result<(), String> {
    if read_security_config(&app)?.is_some() {
        return Err("Security has already been configured.".into());
    }

    write_security_config(
        &app,
        &SecurityConfig {
            enabled: false,
            password_hash: None,
        },
    )
}

#[tauri::command]
fn verify_password(app: tauri::AppHandle, password: String) -> Result<bool, String> {
    let Some(config) = read_security_config(&app)? else {
        return Ok(false);
    };

    if !config.enabled {
        return Ok(true);
    }

    let Some(stored_hash) = config.password_hash else {
        return Err("Password lock is enabled but no password hash is stored.".into());
    };

    let parsed_hash = PasswordHash::new(&stored_hash).map_err(|error| error.to_string())?;

    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            security_status,
            setup_password,
            skip_password_setup,
            verify_password
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
