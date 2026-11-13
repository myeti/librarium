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
//!
//! On macOS and Linux, what other code writes to stderr goes to the log file
//! too ([`capture_stderr`]): GTK, GLib, GStreamer and the web view's own
//! process (`WebKitWebProcess`) print their errors there, not through us.

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

/// Copy every line written to stderr into the log file too, as
/// `[stderr] …`, and still to the terminal. That covers what never goes
/// through [`log`]: messages from GTK, GLib and GStreamer, and from
/// WebKitGTK's `WebKitWebProcess`, which inherits our stderr (a missing
/// GStreamer element, a GLib CRITICAL). Call first thing, before any of them
/// starts, so the web view's process inherits the pipe. Lines read before
/// the log plugin is up reach the terminal only. Best effort: if anything
/// fails, stderr is left as it was. Unix only (Windows' web view doesn't
/// print there).
#[cfg(unix)]
pub(crate) fn capture_stderr() {
    use std::fs::File;
    use std::io::{BufRead, BufReader, Write};
    use std::os::fd::FromRawFd;

    // SAFETY: plain fd juggling on descriptors this function creates; each
    // is closed on every failure path, and the two kept are owned by `File`s
    // moved into the reader thread.
    unsafe {
        let mut fds = [0; 2];
        if libc::pipe(fds.as_mut_ptr()) != 0 {
            return;
        }
        let [read_end, write_end] = fds;
        let original = libc::dup(libc::STDERR_FILENO);
        if original < 0 {
            libc::close(read_end);
            libc::close(write_end);
            return;
        }
        let reader = File::from_raw_fd(read_end);
        let mut terminal = File::from_raw_fd(original);
        // The reader runs first: stderr only becomes the pipe once someone
        // drains it (a full pipe nobody reads would block every writer).
        let spawned = std::thread::Builder::new()
            .name("stderr-to-log".into())
            .spawn(move || {
                for line in BufReader::new(reader).split(b'\n') {
                    let Ok(bytes) = line else { break };
                    let _ = terminal.write_all(&bytes);
                    let _ = terminal.write_all(b"\n");
                    let text = String::from_utf8_lossy(&bytes);
                    let text = text.trim_end();
                    if !text.is_empty() {
                        log::log!(stderr_level(text), "[stderr] {text}");
                    }
                }
            });
        if spawned.is_err() || libc::dup2(write_end, libc::STDERR_FILENO) < 0 {
            // Without a reader, keep stderr as it is (the thread, if any,
            // ends when the write end closes).
            libc::close(write_end);
            return;
        }
        libc::close(write_end);
    }
}

/// The level a stderr line is logged with: GLib's CRITICAL and errors are
/// errors, the rest (GTK messages, GStreamer warnings…) warnings.
#[cfg(unix)]
fn stderr_level(line: &str) -> log::Level {
    let upper = line.to_ascii_uppercase();
    if upper.contains("CRITICAL") || upper.contains("ERROR") || upper.contains("FATAL") {
        log::Level::Error
    } else {
        log::Level::Warn
    }
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

    /// Run in a child process (it takes over that process's stderr): with
    /// the capture on, writing far more than a pipe holds neither blocks nor
    /// loses a byte on the way to the real stderr.
    #[cfg(unix)]
    #[test]
    fn captured_stderr_still_reaches_the_terminal_in_full() {
        const CHILD: &str = "LIBRARIUM_STDERR_CHILD";
        let line = "x".repeat(99);
        if std::env::var_os(CHILD).is_some() {
            capture_stderr();
            for _ in 0..2000 {
                eprintln!("{line}");
            }
            // Let the reader thread drain the pipe before the process exits.
            std::thread::sleep(std::time::Duration::from_millis(500));
            return;
        }
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "logging::tests::captured_stderr_still_reaches_the_terminal_in_full",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(out.status.success());
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(stderr.lines().filter(|l| *l == line).count(), 2000);
    }

    #[cfg(unix)]
    #[test]
    fn stderr_lines_are_errors_or_warnings() {
        let critical =
            "(WebKitWebProcess:31150): GLib-GObject-CRITICAL **: invalid (NULL) pointer instance";
        assert_eq!(stderr_level(critical), log::Level::Error);
        assert_eq!(
            stderr_level("GStreamer element appsink not found. Please install it."),
            log::Level::Warn
        );
        assert_eq!(
            stderr_level("Gtk-Message: Failed to load module \"canberra-gtk-module\""),
            log::Level::Warn
        );
    }

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
