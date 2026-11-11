//! Persistent app configuration stored in `app_data_dir/librarium-config.json`.
//! This location is independent of the active library folder, so it survives
//! any library path change without a chicken-and-egg bootstrap problem.
//!
//! The file holds the library location and user settings, so it gets the
//! same care as the library index: writes are atomic ([`fsutil::atomic_write`])
//! and serialised ([`fsutil::with_file_lock`]), `set_config` only changes the
//! keys it is given, and a damaged file is never overwritten — `set_config`
//! fails with the parse error instead, so the user (or a backup) can repair it.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

use crate::commands::library::blocking;
use crate::error::{AppError, AppResult};
use crate::fsutil;
use crate::logging::log;
use crate::mtp::MtpState;

/// The configuration: a flat JSON object of setting name → value.
type ConfigMap = HashMap<String, serde_json::Value>;

fn config_path(app: &AppHandle) -> AppResult<PathBuf> {
    app.path()
        .app_data_dir()
        .map(|d| d.join("librarium-config.json"))
        .map_err(|e| AppError::Internal(format!("cannot resolve the app data folder: {e}")))
}

/// Read the configuration at `path`. A missing file is an empty
/// configuration (first launch); an unreadable or invalid one is an error.
fn read_config(path: &Path) -> AppResult<ConfigMap> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(ConfigMap::new()),
        Err(e) => {
            return Err(AppError::io(
                format!("Could not read the settings file {}", path.display()),
                &e,
            ))
        }
    };
    serde_json::from_str(&text).map_err(|e| {
        AppError::Io(format!(
            "The settings file {} is damaged ({e}). It was not changed; fix or remove it to save settings again.",
            path.display()
        ))
    })
}

/// Merge `updates` into the configuration at `path` and save it atomically,
/// keeping every other key. Fails without writing if the current file can't
/// be read or parsed.
fn update_config(path: &Path, updates: ConfigMap) -> AppResult<()> {
    fsutil::with_file_lock(path, || {
        let mut map = read_config(path)?;
        map.extend(updates);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| AppError::io("Could not create the settings folder", &e))?;
        }
        let json =
            serde_json::to_vec_pretty(&map).map_err(|e| AppError::Internal(e.to_string()))?;
        fsutil::atomic_write(path, &json)
            .map_err(|e| AppError::io("Could not save the settings", &e))
    })
}

/// The library root the settings name: `libraryRootOverride`, else the
/// default `Documents/Librarium` (as the frontend's users store resolves
/// it). For commands that must not trust a root sent by the webview (a
/// restore erases its users).
pub(crate) fn library_root(app: &AppHandle) -> AppResult<PathBuf> {
    let map = read_config(&config_path(app)?)?;
    if let Some(root) = map
        .get("libraryRootOverride")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Ok(PathBuf::from(root));
    }
    app.path()
        .document_dir()
        .map(|d| d.join("Librarium"))
        .map_err(|e| AppError::Internal(format!("cannot resolve the Documents folder: {e}")))
}

/// The whole configuration. A damaged file reads as empty (the app starts
/// with defaults) and is logged; it stays on disk untouched, because
/// `set_config` refuses to overwrite it. Read on the blocking pool: Tauri
/// runs synchronous commands on the main thread, which a slow disk would
/// freeze.
#[tauri::command]
pub async fn get_config(app: AppHandle) -> serde_json::Value {
    let map = match config_path(&app) {
        // `blocking` logs a failure itself.
        Ok(path) => blocking(move || read_config(&path))
            .await
            .unwrap_or_default(),
        Err(e) => {
            log(&app, format!("[config] ERROR — {e}"));
            ConfigMap::new()
        }
    };
    serde_json::Value::Object(map.into_iter().collect())
}

/// Set the given keys, leaving the others unchanged (on the blocking pool,
/// like [`get_config`]). Setting [`MTP_ENABLED`] turns MTP support on or
/// off right away.
#[tauri::command]
pub async fn set_config(
    app: AppHandle,
    updates: HashMap<String, serde_json::Value>,
) -> AppResult<()> {
    let path = config_path(&app)?;
    // Like `read_flag`: anything but `false` is on.
    let mtp = updates.get(MTP_ENABLED).map(|v| v.as_bool() != Some(false));
    blocking(move || update_config(&path, updates)).await?;
    if let (Some(enabled), Some(state)) = (mtp, app.try_state::<MtpState>()) {
        state.set_enabled(enabled);
    }
    Ok(())
}

/// The setting for MTP Kindles (2024+, experimental): on unless it is
/// `false`. Off, Librarium only enumerates USB devices (to say that one is
/// there) and never claims one.
pub(crate) const MTP_ENABLED: &str = "mtpEnabled";

/// The boolean setting `key` (at startup): its value when it is a boolean,
/// else `default` (absent, not a boolean, or a missing or damaged file).
pub(crate) fn read_flag(app: &AppHandle, key: &str, default: bool) -> bool {
    config_path(app)
        .ok()
        .and_then(|path| read_config(&path).ok())
        .and_then(|map| map.get(key).and_then(serde_json::Value::as_bool))
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn updates(pairs: &[(&str, serde_json::Value)]) -> ConfigMap {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn missing_config_is_empty_and_gets_created() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app").join("librarium-config.json");
        assert!(read_config(&path).unwrap().is_empty());
        update_config(&path, updates(&[("theme", json!("dark"))])).unwrap();
        assert_eq!(read_config(&path).unwrap()["theme"], json!("dark"));
    }

    #[test]
    fn updates_keep_the_other_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("librarium-config.json");
        update_config(
            &path,
            updates(&[
                ("libraryPath", json!("/Volumes/books")),
                ("theme", json!("dark")),
            ]),
        )
        .unwrap();
        update_config(&path, updates(&[("theme", json!("light"))])).unwrap();
        let map = read_config(&path).unwrap();
        assert_eq!(map["libraryPath"], json!("/Volumes/books"));
        assert_eq!(map["theme"], json!("light"));
    }

    #[test]
    fn a_damaged_config_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("librarium-config.json");
        std::fs::write(&path, b"{\"libraryPath\": \"/Volu").unwrap();
        let err = update_config(&path, updates(&[("theme", json!("dark"))])).unwrap_err();
        assert!(err.to_string().contains("damaged"), "{err}");
        assert_eq!(std::fs::read(&path).unwrap(), b"{\"libraryPath\": \"/Volu");
    }
}
