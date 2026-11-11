//! Finding the user's Calibre library without asking.
//!
//! Calibre remembers the library it last opened in its own settings: the
//! `library_path` key of `global.py.json`, in its configuration folder. That
//! is the library the user actually works with, wherever it lives (another
//! disk, a NAS share), so it is tried first. Only when Calibre's settings
//! can't be found or point to a folder without a `metadata.db` do we fall
//! back to the folders Calibre suggests on first run ("Calibre Library" in
//! the home or Documents folder).
//!
//! Everything here takes the environment as a function (`env`), so tests can
//! describe any machine without touching the real one.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// How a test (or the app, with [`real_env`]) answers "what is the value of
/// this environment variable?".
pub(crate) type Env<'a> = &'a dyn Fn(&str) -> Option<OsString>;

/// The environment of the running app.
pub(crate) fn real_env(name: &str) -> Option<OsString> {
    std::env::var_os(name)
}

/// The conventions that decide where Calibre keeps its settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Platform {
    MacOs,
    Windows,
    /// Linux and the BSDs: the XDG base directory rules.
    Unix,
}

impl Platform {
    /// The platform the app was built for.
    pub(crate) fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(windows) {
            Self::Windows
        } else {
            Self::Unix
        }
    }
}

/// Calibre's settings file holding the current library (`library_path`).
const GLOBAL_PREFS_FILE: &str = "global.py.json";

/// `global.py.json` is a few kilobytes; anything much larger isn't it.
const MAX_PREFS_BYTES: u64 = 4 * 1024 * 1024;

/// The value of `name` as a path, if it is set to an absolute path. (Relative
/// values are ignored: they would be resolved against whatever the app's
/// working directory happens to be.)
fn env_path(env: Env, name: &str) -> Option<PathBuf> {
    env(name).map(PathBuf::from).filter(|p| p.is_absolute())
}

/// The user's home folder (`USERPROFILE` on Windows, `HOME` elsewhere).
fn home(platform: Platform, env: Env) -> Option<PathBuf> {
    match platform {
        Platform::Windows => env_path(env, "USERPROFILE").or_else(|| env_path(env, "HOME")),
        Platform::MacOs | Platform::Unix => env_path(env, "HOME"),
    }
}

/// Where Calibre may keep its settings, in the order Calibre itself looks:
/// `CALIBRE_CONFIG_DIRECTORY` when set (Calibre honours it, for example in
/// portable setups), then the platform's folder:
///
/// | Platform | Folder |
/// |---|---|
/// | macOS | `~/Library/Preferences/calibre` |
/// | Windows | `%APPDATA%\calibre` |
/// | Linux | `$XDG_CONFIG_HOME/calibre`, else `~/.config/calibre` |
pub(crate) fn config_dirs(platform: Platform, env: Env) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = env_path(env, "CALIBRE_CONFIG_DIRECTORY") {
        dirs.push(dir);
    }
    let platform_dir = match platform {
        Platform::MacOs => {
            home(platform, env).map(|h| h.join("Library").join("Preferences").join("calibre"))
        }
        Platform::Windows => env_path(env, "APPDATA").map(|d| d.join("calibre")),
        Platform::Unix => env_path(env, "XDG_CONFIG_HOME")
            .or_else(|| home(platform, env).map(|h| h.join(".config")))
            .map(|d| d.join("calibre")),
    };
    dirs.extend(platform_dir);
    dirs
}

/// The library Calibre last opened, as recorded in `global.py.json` in
/// `config_dir`: its `library_path`, if the file can be read and the value
/// is an absolute path. Whether a library is really there is checked by the
/// caller.
pub(crate) fn configured_library(config_dir: &Path) -> Option<PathBuf> {
    let file = config_dir.join(GLOBAL_PREFS_FILE);
    if std::fs::metadata(&file).ok()?.len() > MAX_PREFS_BYTES {
        return None;
    }
    let text = std::fs::read_to_string(&file).ok()?;
    let prefs: serde_json::Value = serde_json::from_str(&text).ok()?;
    let path = prefs.get("library_path")?.as_str()?.trim();
    Some(PathBuf::from(path)).filter(|p| !path.is_empty() && p.is_absolute())
}

/// The folders Calibre proposes for a new library, used when its settings
/// don't name one: `~/Calibre Library`, `~/Documents/Calibre Library`, and
/// `~/Books` (an older default).
pub(crate) fn usual_library_folders(platform: Platform, env: Env) -> Vec<PathBuf> {
    let Some(home) = home(platform, env) else {
        return vec![];
    };
    vec![
        home.join("Calibre Library"),
        home.join("Documents").join("Calibre Library"),
        home.join("Books"),
    ]
}

/// Whether `folder` holds a Calibre library (its `metadata.db`).
pub(crate) fn is_calibre_library(folder: &Path) -> bool {
    folder.join(super::db::DB_FILE).is_file()
}

/// The user's Calibre library: the one Calibre's settings name, else the
/// first of the usual folders that holds a library; `None` if there is none.
pub(crate) fn detect(platform: Platform, env: Env) -> Option<PathBuf> {
    config_dirs(platform, env)
        .iter()
        .filter_map(|dir| configured_library(dir))
        .chain(usual_library_folders(platform, env))
        .find(|folder| is_calibre_library(folder))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// An environment holding exactly `vars`.
    fn env_of(vars: &[(&str, &Path)]) -> impl Fn(&str) -> Option<OsString> {
        let vars: HashMap<String, OsString> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.as_os_str().to_owned()))
            .collect();
        move |name| vars.get(name).cloned()
    }

    fn make_library(folder: &Path) {
        std::fs::create_dir_all(folder).unwrap();
        std::fs::write(folder.join("metadata.db"), b"").unwrap();
    }

    fn write_prefs(config_dir: &Path, library: &Path) {
        std::fs::create_dir_all(config_dir).unwrap();
        let json = serde_json::json!({ "library_path": library, "database_path": "ignored" });
        std::fs::write(config_dir.join(GLOBAL_PREFS_FILE), json.to_string()).unwrap();
    }

    #[test]
    fn config_folders_follow_each_platform() {
        // Absolute on every OS the tests run on (a bare "/home/me" isn't, on Windows).
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let appdata = root.path().join("appdata");
        let env = env_of(&[("HOME", &home), ("APPDATA", &appdata)]);
        assert_eq!(
            config_dirs(Platform::MacOs, &env),
            [home.join("Library/Preferences/calibre")]
        );
        assert_eq!(
            config_dirs(Platform::Windows, &env),
            [appdata.join("calibre")]
        );
        assert_eq!(
            config_dirs(Platform::Unix, &env),
            [home.join(".config/calibre")]
        );

        let xdg_dir = root.path().join("xdg");
        let xdg = env_of(&[("HOME", &home), ("XDG_CONFIG_HOME", &xdg_dir)]);
        assert_eq!(config_dirs(Platform::Unix, &xdg), [xdg_dir.join("calibre")]);

        // Calibre's own override comes first; relative values are ignored.
        let portable = root.path().join("portable/settings");
        let custom = env_of(&[
            ("HOME", &home),
            ("CALIBRE_CONFIG_DIRECTORY", &portable),
            ("XDG_CONFIG_HOME", Path::new("relative")),
        ]);
        assert_eq!(
            config_dirs(Platform::Unix, &custom),
            [portable, home.join(".config/calibre")]
        );
        assert!(config_dirs(Platform::Windows, &env_of(&[])).is_empty());
    }

    #[test]
    fn the_library_named_in_calibre_settings_wins() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        // A library in the usual place, and the one Calibre actually uses, elsewhere.
        make_library(&home.join("Calibre Library"));
        let nas = dir.path().join("Volumes/books/Calibre");
        make_library(&nas);
        write_prefs(&home.join(".config/calibre"), &nas);

        let env = env_of(&[("HOME", &home)]);
        assert_eq!(detect(Platform::Unix, &env), Some(nas));
    }

    #[test]
    fn falls_back_to_the_usual_folders() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        make_library(&home.join("Documents/Calibre Library"));
        let env = env_of(&[("HOME", &home)]);

        // No settings at all.
        assert_eq!(
            detect(Platform::MacOs, &env),
            Some(home.join("Documents/Calibre Library"))
        );

        // Settings naming a library that is gone (an unmounted share).
        write_prefs(
            &home.join("Library/Preferences/calibre"),
            &dir.path().join("Volumes/gone"),
        );
        assert_eq!(
            detect(Platform::MacOs, &env),
            Some(home.join("Documents/Calibre Library"))
        );

        // Damaged settings.
        std::fs::write(
            home.join("Library/Preferences/calibre")
                .join(GLOBAL_PREFS_FILE),
            b"{ not json",
        )
        .unwrap();
        assert_eq!(
            detect(Platform::MacOs, &env),
            Some(home.join("Documents/Calibre Library"))
        );
    }

    #[test]
    fn nothing_found_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let env = env_of(&[("HOME", dir.path())]);
        assert_eq!(detect(Platform::Unix, &env), None);
        assert_eq!(detect(Platform::Unix, &env_of(&[])), None);
    }

    #[test]
    fn only_an_absolute_library_path_is_taken() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(GLOBAL_PREFS_FILE),
            r#"{"library_path": "Calibre Library"}"#,
        )
        .unwrap();
        assert_eq!(configured_library(dir.path()), None);
        std::fs::write(
            dir.path().join(GLOBAL_PREFS_FILE),
            r#"{"library_path": null}"#,
        )
        .unwrap();
        assert_eq!(configured_library(dir.path()), None);
        std::fs::write(
            dir.path().join(GLOBAL_PREFS_FILE),
            r#"{"library_path": "  "}"#,
        )
        .unwrap();
        assert_eq!(configured_library(dir.path()), None);
    }
}
