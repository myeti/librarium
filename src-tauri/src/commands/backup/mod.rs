//! Backups of the whole library: every user, their books and Librarium's
//! data about them, as one zip; and restoring one, which replaces them all.
//! The format and the safe order of a restore are in [`archive`].
//!
//! One export or restore runs at a time ([`BackupState`]); both report
//! [`archive::Progress`] through [`PROGRESS_EVENT`] (throttled) and can be
//! cancelled ([`cancel_backup`]): a cancelled export leaves no file, a
//! cancelled restore leaves the library as it was (the cancel is only
//! heard while extracting, before anything is replaced).
//!
//! The library root is checked against the settings' (`library_root`): a
//! restore erases the users of the folder it is given, which must not be a
//! folder the webview chose.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, State};

pub(crate) mod archive;

pub use archive::{BackupManifest, Exported, Progress, Restored};

use crate::commands::config::library_root;
use crate::commands::library::{blocking, index, os};
use crate::error::{AppError, AppResult};
use crate::logging::logger;

/// The event carrying [`Progress`] while a backup or a restore runs.
pub const PROGRESS_EVENT: &str = "backup-progress";

/// Progress events go out at most this often (and at every phase change).
const PROGRESS_INTERVAL: Duration = Duration::from_millis(150);

/// Tauri-managed state: the cancel flag of the running export or restore,
/// and the last backup written (the only file [`reveal_backup`] shows).
#[derive(Default)]
pub struct BackupState {
    running: Mutex<Option<Arc<AtomicBool>>>,
    last_export: Mutex<Option<PathBuf>>,
}

/// The running operation's slot, freed when dropped (end, error, panic).
struct Running<'a> {
    state: &'a BackupState,
    cancelled: Arc<AtomicBool>,
}

impl BackupState {
    fn slot(&self) -> std::sync::MutexGuard<'_, Option<Arc<AtomicBool>>> {
        self.running.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn begin(&self) -> AppResult<Running<'_>> {
        let mut slot = self.slot();
        if slot.is_some() {
            return Err(AppError::Conflict(
                "A backup or a restore is already running".into(),
            ));
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        *slot = Some(cancelled.clone());
        Ok(Running {
            state: self,
            cancelled,
        })
    }

    fn cancel(&self) -> bool {
        match &*self.slot() {
            Some(flag) => {
                flag.store(true, Ordering::Relaxed);
                true
            }
            None => false,
        }
    }
}

impl Drop for Running<'_> {
    fn drop(&mut self) {
        *self.state.slot() = None;
    }
}

/// Sends progress at most every [`PROGRESS_INTERVAL`], and always when the
/// phase changes.
fn throttled(app: AppHandle) -> impl Fn(Progress) {
    let last: Cell<Option<(Instant, archive::Phase)>> = Cell::new(None);
    move |p: Progress| {
        let now = Instant::now();
        let due = match last.get() {
            None => true,
            Some((at, phase)) => phase != p.phase || now.duration_since(at) >= PROGRESS_INTERVAL,
        };
        if due {
            last.set(Some((now, p.phase)));
            let _ = app.emit(PROGRESS_EVENT, p);
        }
    }
}

/// Whether `given` is the settings' library root (the same folder).
fn is_configured_root(app: &AppHandle, given: &Path) -> AppResult<bool> {
    let configured = library_root(app)?;
    let same = match (
        std::fs::canonicalize(given),
        std::fs::canonicalize(&configured),
    ) {
        (Ok(a), Ok(b)) => a == b,
        _ => given == configured,
    };
    Ok(same)
}

fn check_root(app: &AppHandle, root: &Path) -> AppResult<()> {
    if is_configured_root(app, root)? {
        Ok(())
    } else {
        Err(AppError::InvalidInput(format!(
            "{} isn't the library folder of the settings",
            root.display()
        )))
    }
}

/// Write a backup of the library at `library_root` (every user) into a new
/// zip in `dest_dir` ([`archive::export`]).
#[tauri::command]
pub async fn export_backup(
    app: AppHandle,
    state: State<'_, BackupState>,
    library_root: String,
    dest_dir: String,
) -> AppResult<Exported> {
    let root = PathBuf::from(&library_root);
    check_root(&app, &root)?;
    let running = state.begin()?;
    let cancelled = running.cancelled.clone();
    let log = logger(&app);
    let version = app.package_info().version.to_string();
    let result = blocking(move || {
        let progress = throttled(app);
        let is_cancelled = || cancelled.load(Ordering::Relaxed);
        let hooks = archive::Hooks {
            progress: &progress,
            is_cancelled: &is_cancelled,
            log: &log,
        };
        archive::export(&root, Path::new(&dest_dir), &version, &hooks)
    })
    .await;
    drop(running);
    if let Ok(Exported {
        path: Some(path), ..
    }) = &result
    {
        *state.last_export.lock().unwrap_or_else(|e| e.into_inner()) = Some(PathBuf::from(path));
    }
    result
}

/// Show the last backup written in Finder/Explorer (the export's "Show"
/// button): only that file, never a path from the webview.
#[tauri::command]
pub async fn reveal_backup(state: State<'_, BackupState>) -> AppResult<()> {
    let last = state
        .last_export
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let path = last.ok_or_else(|| AppError::NotFound("No backup was made yet".into()))?;
    blocking(move || os::reveal(&path)).await
}

/// What the backup at `path` holds (its manifest), to say before restoring;
/// `INVALID_INPUT` when it isn't a Librarium backup.
#[tauri::command]
pub async fn read_backup(path: String) -> AppResult<BackupManifest> {
    blocking(move || archive::read_manifest(Path::new(&path))).await
}

/// Replace every user of the library at `library_root` with the backup's
/// ([`archive::restore`]: extracted aside first, then swapped).
#[tauri::command]
pub async fn restore_backup(
    app: AppHandle,
    state: State<'_, BackupState>,
    library_root: String,
    path: String,
) -> AppResult<Restored> {
    let root = PathBuf::from(&library_root);
    check_root(&app, &root)?;
    let running = state.begin()?;
    let cancelled = running.cancelled.clone();
    let log = logger(&app);
    let result = blocking(move || {
        let progress = throttled(app);
        let is_cancelled = || cancelled.load(Ordering::Relaxed);
        let hooks = archive::Hooks {
            progress: &progress,
            is_cancelled: &is_cancelled,
            log: &log,
        };
        let restored = archive::restore(&root, Path::new(&path), &hooks);
        index::clear_cache();
        restored
    })
    .await;
    drop(running);
    result
}

/// Stop the running export or restore (a restore only while it extracts).
/// Returns whether one was running.
#[tauri::command]
pub async fn cancel_backup(state: State<'_, BackupState>) -> AppResult<bool> {
    Ok(state.cancel())
}
