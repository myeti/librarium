//! Diagnostics: the in-app debug panel and the log file, fed by the same
//! lines.
//!
//! Every line goes to two places:
//! - the **debug panel**: the frontend listens to the [`LOG_EVENT`]
//!   (`app:log`) event, whose payload is the line itself (a plain string);
//! - the **log file**, through the `log` crate and `tauri-plugin-log`
//!   ([`plugin`]): `Librarium.log` in the app's log folder (macOS
//!   `~/Library/Logs/com.librarium.desktop/`, Windows
//!   `%LOCALAPPDATA%\com.librarium.desktop\logs\`, Linux
//!   `~/.local/share/com.librarium.desktop/logs/`), rotated by size. The
//!   `open_log_folder` command shows that folder. The file stays on this
//!   computer: it is what a user attaches to a bug report.
//!
//! The level of a line comes from its text ([`level_of`]): lines saying
//! `ERROR` are errors, `WARNING` warnings, the rest information. Release
//! builds write `Info` and above, debug builds `Debug` and above (other
//! crates: `Warn`/`Info`, so HTTP and webview internals don't drown ours).
//!
//! Commands that hold an [`AppHandle`] call [`log`] directly. Code below the
//! commands takes a [`Log`] sink instead, so it can run without a Tauri app:
//! commands pass [`logger`], tests pass `no_log` or collect the lines with
//! `LogLines` (both test-only). Shared plumbing with no handle at hand (the
//! `blocking` wrapper of the library commands) uses [`log_global`], which
//! reaches the panel once [`install`] has run at startup.

use std::cell::Cell;
use std::path::PathBuf;
use std::sync::OnceLock;

use tauri::{AppHandle, Emitter, Manager, Runtime};

/// Event the debug panel listens to; the payload is the log line (a string).
pub(crate) const LOG_EVENT: &str = "app:log";

/// Name of the log file (without `.log`) in the app's log folder.
pub(crate) const LOG_FILE_NAME: &str = "Librarium";

/// Size at which the log file is rotated: the current file is renamed with
/// a date, and a new one started.
const MAX_LOG_FILE_BYTES: u128 = 5 * 1024 * 1024;

/// How many log files are kept, the current one included (5 × 5 MB at most).
const KEPT_LOG_FILES: usize = 5;

/// The `tauri-plugin-log` plugin, writing [`LOG_FILE_NAME`]`.log` in the
/// app's log folder (and, in debug builds, to stdout too). Register it
/// before anything logs.
pub(crate) fn plugin<R: Runtime>() -> tauri::plugin::TauriPlugin<R> {
    use tauri_plugin_log::{Builder, RotationStrategy, Target, TargetKind, TimezoneStrategy};

    let (ours, others) = if cfg!(debug_assertions) {
        (log::LevelFilter::Debug, log::LevelFilter::Info)
    } else {
        (log::LevelFilter::Info, log::LevelFilter::Warn)
    };
    let mut builder = Builder::new()
        .clear_targets()
        .target(Target::new(TargetKind::LogDir {
            file_name: Some(LOG_FILE_NAME.into()),
        }))
        .max_file_size(MAX_LOG_FILE_BYTES)
        .rotation_strategy(RotationStrategy::KeepSome(KEPT_LOG_FILES))
        // UTC: the `time` crate can't read the local offset once the app has
        // several threads (it would silently fall back to UTC anyway).
        .timezone_strategy(TimezoneStrategy::UseUtc)
        .level(others)
        .level_for(env!("CARGO_CRATE_NAME"), ours);
    if cfg!(debug_assertions) {
        builder = builder.target(Target::new(TargetKind::Stdout));
    }
    builder.build()
}

/// The folder the log file is written to.
pub(crate) fn log_dir(app: &AppHandle) -> Result<PathBuf, tauri::Error> {
    app.path().app_log_dir()
}

/// The level a debug-panel line is written to the log file with: `Error`
/// when it says `ERROR`, `Warn` when it says `WARNING`, `Info` otherwise.
/// (The lines have always carried these words; the panel shows them as is.)
pub(crate) fn level_of(msg: &str) -> log::Level {
    if msg.contains("ERROR") {
        log::Level::Error
    } else if msg.contains("WARNING") {
        log::Level::Warn
    } else {
        log::Level::Info
    }
}

/// Write one line to the log file (no-op until the log plugin is set up,
/// and in tests).
fn to_file(msg: &str) {
    log::log!(level_of(msg), "{msg}");
}

/// Send one line to the in-app debug panel and to the log file. Best
/// effort: an emit error is ignored (logging must never fail a command).
pub(crate) fn log(app: &AppHandle, msg: impl Into<String>) {
    let msg = msg.into();
    to_file(&msg);
    let _ = app.emit(LOG_EVENT, msg);
}

/// The app handle [`log_global`] emits through, set once by [`install`].
static GLOBAL: OnceLock<AppHandle> = OnceLock::new();

/// Make [`log_global`] reach the debug panel, and send panics to the log
/// file (call once, at startup).
pub(crate) fn install(app: &AppHandle) {
    let _ = GLOBAL.set(app.clone());
    install_panic_hook();
}

/// Send one line to the debug panel and the log file without an
/// `AppHandle` at hand. Before [`install`] (and in tests) the line goes to
/// stderr only.
pub(crate) fn log_global(msg: String) {
    match GLOBAL.get() {
        Some(app) => log(app, msg),
        None => eprintln!("{msg}"),
    }
}

/// Write every panic — with where it happened — to the log file, then let
/// the default hook print it as before. Panics in commands are caught and
/// returned as `INTERNAL` errors, but only this hook sees their location.
fn install_panic_hook() {
    thread_local! {
        // A panic while logging a panic must not loop.
        static IN_HOOK: Cell<bool> = const { Cell::new(false) };
    }
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if !IN_HOOK.with(|flag| flag.replace(true)) {
            log::error!("panic: {info}");
            IN_HOOK.with(|flag| flag.set(false));
        }
        default_hook(info);
    }));
}

/// Where AppHandle-free code sends its diagnostic lines: [`logger`] in the
/// app, `no_log` or `LogLines::sink` in tests. Keeping `AppHandle` out of
/// that code is what makes it unit-testable.
pub(crate) type Log<'a> = &'a dyn Fn(String);

/// A [`Log`] sink that forwards to the debug panel and the log file
/// ([`log`]). It owns a clone of the handle, so it can be moved onto the
/// blocking thread pool.
pub(crate) fn logger(app: &AppHandle) -> impl Fn(String) + Send + Sync + 'static {
    let app = app.clone();
    move |msg| log(&app, msg)
}

/// A [`Log`] sink that discards every line (tests that don't check logging).
#[cfg(test)]
pub(crate) fn no_log(_: String) {}

/// A [`Log`] sink that keeps every line, for tests that check what was logged.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct LogLines(std::cell::RefCell<Vec<String>>);

#[cfg(test)]
impl LogLines {
    /// The sink to pass as a [`Log`]: `&lines.sink()`.
    pub(crate) fn sink(&self) -> impl Fn(String) + '_ {
        move |msg| self.0.borrow_mut().push(msg)
    }

    /// Whether any line logged so far contains `needle`.
    pub(crate) fn contains(&self, needle: &str) -> bool {
        self.0.borrow().iter().any(|line| line.contains(needle))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_follow_the_words_the_lines_carry() {
        assert_eq!(level_of("[scan_library] ERROR — boom"), log::Level::Error);
        assert_eq!(
            level_of("[send_books] ERROR job 1: full"),
            log::Level::Error
        );
        assert_eq!(
            level_of("[covers] WARNING — can't be shown"),
            log::Level::Warn
        );
        assert_eq!(level_of("[scan_library] done — 3 books"), log::Level::Info);
    }

    #[test]
    fn logging_without_a_logger_is_harmless() {
        // No plugin in tests: the `log` facade drops the line.
        to_file("[test] ERROR — nothing listens");
        log_global("[test] a line before install".into());
    }
}
