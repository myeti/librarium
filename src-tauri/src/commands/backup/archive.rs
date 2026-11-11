//! What a backup holds, and how it is written and restored (no Tauri here:
//! [`super`] wires it to commands and events).
//!
//! **A backup** is a zip file of the library root's users: every user
//! folder ([`users::list_user_dirs`]) with its books, its `.librarium`
//! folder (the index) and its `.covers`, under `library/<user>/…`, and a
//! manifest, [`MANIFEST`] (what made it, when, which users, how much). Not
//! in it: anything else in the root (a NAS's `#recycle`, unrelated files),
//! the library trash (`.librarium/trash`), lock files, temp files of
//! transfers and conversions, and system clutter (`.DS_Store`, `._*`…).
//! Books (already compressed: EPUB, PDF, CBZ, MOBI…) and images are stored
//! as they are; text files (the index, OPFs) are deflated. Each file keeps
//! its modification date (to the even second, zip dates being so).
//!
//! **Restoring** replaces the root's users with the backup's, in an order
//! that never leaves the library half-gone:
//! 1. every file is extracted into a hidden staging folder of the root
//!    (`.librarium-restore-<id>`), after checking there is room for it: a
//!    damaged or cancelled backup stops here, and nothing was touched;
//! 2. the current users are moved aside (renamed into
//!    `.librarium-restore-<id>-previous`, instant on one volume), then the
//!    backup's users are moved in; if a move fails, what was moved is put
//!    back;
//! 3. the previous users are deleted (that is the "erase"), and the staging
//!    folder. A folder that can't be deleted stays hidden, logged.
//!
//! Entry names are checked before anything is written: only `library/` +
//! a user folder name + a path that stays inside it (no `..`, no absolute
//! path: `ZipFile::enclosed_name`); anything else in the zip is skipped.

use std::fs::{self, File};
use std::io::{self, BufWriter, Read, Seek, Write};
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Datelike, Local, NaiveDate, Timelike, Utc};
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::commands::library::users;
use crate::error::{AppError, AppResult};
use crate::fsutil;
use crate::logging::Log;

/// The manifest's name, at the root of the zip.
pub(crate) const MANIFEST: &str = "librarium-backup.json";

/// Where the users are in the zip.
const LIBRARY_PREFIX: &str = "library/";

/// The backup format this version writes, and the newest it reads.
pub(crate) const FORMAT: u32 = 1;

/// Copies go by chunks of this size, checking for a cancel in between.
const CHUNK: usize = 1 << 20;

/// Room kept free on top of the backup's size (the zip's own headers, the
/// file system's overhead).
const SPACE_MARGIN: u64 = 32 * 1024 * 1024;

/// The manifest of a backup.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupManifest {
    /// Always "Librarium": what tells a backup from any other zip.
    pub app: String,
    pub format: u32,
    pub app_version: String,
    /// RFC 3339.
    pub created_at: String,
    pub users: Vec<String>,
    pub files: u64,
    /// The files' total size, uncompressed.
    pub bytes: u64,
}

/// Where an export or a restore is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    /// Listing the files (export) or reading the backup (restore).
    Scanning,
    /// Writing the zip (export) or extracting it (restore).
    Copying,
    /// Restore: moving the backup's users in place of the current ones.
    Replacing,
}

/// Progress of an export or a restore.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub phase: Phase,
    pub files_done: u64,
    pub files_total: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
}

/// What a running export or restore reports to, and asks.
pub(crate) struct Hooks<'a> {
    pub progress: &'a dyn Fn(Progress),
    pub is_cancelled: &'a dyn Fn() -> bool,
    pub log: Log<'a>,
}

/// A finished (or cancelled) export.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Exported {
    /// The zip written; `None` when cancelled (nothing is left behind).
    pub path: Option<String>,
    pub users: Vec<String>,
    pub files: u64,
    pub bytes: u64,
    pub cancelled: bool,
}

/// A finished (or cancelled) restore.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Restored {
    /// The users now in the library (the backup's).
    pub users: Vec<String>,
    pub files: u64,
    /// Cancelled while extracting: the library wasn't touched.
    pub cancelled: bool,
}

// ── Export ────────────────────────────────────────────────────────────────────

/// Whether a file (`rel`: its path inside its user folder) stays out of a
/// backup: the library trash, lock and temp files, system clutter.
fn left_out(rel: &Path) -> bool {
    let parts: Vec<&str> = rel.iter().filter_map(|c| c.to_str()).collect();
    if parts.len() >= 2
        && parts[0] == ".librarium"
        && (parts[1] == "trash" || parts[1] == "lock.json")
    {
        return true;
    }
    parts.iter().any(|name| {
        *name == ".DS_Store"
            || *name == "Thumbs.db"
            || *name == "desktop.ini"
            || name.starts_with("._")
            || name.starts_with(".librarium-")
            || name.ends_with(".part")
    })
}

/// How a file is stored: text deflated, the rest (books, images) as is.
fn compression_for(path: &Path) -> CompressionMethod {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    match ext.as_str() {
        "json" | "xml" | "opf" | "ncx" | "txt" | "html" | "xhtml" | "css" | "md" => {
            CompressionMethod::Deflated
        }
        _ => CompressionMethod::Stored,
    }
}

/// A file to back up: where it is, its name in the zip, its size and date.
struct Entry {
    path: PathBuf,
    name: String,
    size: u64,
    modified: Option<SystemTime>,
}

/// Every file of every user of `root`, in a stable order.
fn collect(root: &Path, user_names: &[String]) -> AppResult<Vec<Entry>> {
    let mut entries = Vec::new();
    for user in user_names {
        let dir = root.join(user);
        for item in WalkDir::new(&dir).follow_links(false).sort_by_file_name() {
            let item = item.map_err(|e| {
                let io = io::Error::other(e.to_string());
                AppError::io(format!("Could not read {}", dir.display()), &io)
            })?;
            if !item.file_type().is_file() {
                continue;
            }
            let Ok(rel) = item.path().strip_prefix(&dir) else {
                continue;
            };
            if left_out(rel) {
                continue;
            }
            let meta = item.metadata().ok();
            entries.push(Entry {
                path: item.path().to_path_buf(),
                name: zip_name(user, rel),
                size: meta.as_ref().map_or(0, |m| m.len()),
                modified: meta.and_then(|m| m.modified().ok()),
            });
        }
    }
    Ok(entries)
}

/// `library/<user>/<rel>`, with `/` separators on every system.
fn zip_name(user: &str, rel: &Path) -> String {
    let parts: Vec<String> = rel
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    format!("{LIBRARY_PREFIX}{user}/{}", parts.join("/"))
}

/// A file date as a zip date: local time, to the even second; `None`
/// before 1980 (zip dates start then).
fn zip_time(time: SystemTime) -> Option<zip::DateTime> {
    let t: DateTime<Local> = time.into();
    zip::DateTime::from_date_and_time(
        u16::try_from(t.year()).ok()?,
        t.month() as u8,
        t.day() as u8,
        t.hour() as u8,
        t.minute() as u8,
        t.second() as u8,
    )
    .ok()
}

/// A zip date back as a file date (`None` if it isn't a real date).
fn file_time(t: zip::DateTime) -> Option<SystemTime> {
    let date = NaiveDate::from_ymd_opt(t.year().into(), t.month().into(), t.day().into())?;
    let local = date
        .and_hms_opt(t.hour().into(), t.minute().into(), t.second().into())?
        .and_local_timezone(Local)
        .earliest()?;
    Some(local.into())
}

/// Options dated `time` (now when unknown: zip's default is 1980).
fn dated(options: SimpleFileOptions, time: Option<SystemTime>) -> SimpleFileOptions {
    match zip_time(time.unwrap_or_else(SystemTime::now)) {
        Some(t) => options.last_modified_time(t),
        None => options,
    }
}

/// The user folder of `root` that `path` is in, if any (compared after
/// resolving links). A backup there would be in the next backups, and
/// erased by a restore.
fn user_folder_holding(root: &Path, path: &Path) -> Option<String> {
    let root = fs::canonicalize(root).ok()?;
    let path = fs::canonicalize(path).ok()?;
    let Some(Component::Normal(name)) = path.strip_prefix(&root).ok()?.components().next() else {
        return None;
    };
    let name = name.to_str()?;
    (users::is_user_folder_name(name) && root.join(name).is_dir()).then(|| name.to_owned())
}

/// Free space at `dir`, when the system says.
fn available_space(dir: &Path) -> Option<u64> {
    fs4::statvfs(dir).ok().map(|s| s.available_space())
}

/// Fails when `dir` hasn't room for `needed` bytes (and the margin).
fn check_space(dir: &Path, needed: u64, what: &str) -> AppResult<()> {
    if let Some(free) = available_space(dir) {
        if free < needed.saturating_add(SPACE_MARGIN) {
            return Err(AppError::Io(format!(
                "Not enough space for {what}: it needs {}, {} is free.",
                human_size(needed + SPACE_MARGIN),
                human_size(free)
            )));
        }
    }
    Ok(())
}

fn human_size(bytes: u64) -> String {
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else {
        format!("{:.0} MB", (b / MB).ceil())
    }
}

/// The file name of a backup made at `now`: `Librarium backup 2026-11-10 14.32.zip`
/// (no `:`, which Windows refuses).
pub(crate) fn backup_file_name(now: DateTime<Local>) -> String {
    format!("Librarium backup {}.zip", now.format("%Y-%m-%d %H.%M"))
}

/// Writes a backup of `root`'s users into a new zip in `dest_dir`, through a
/// hidden temp file renamed at the end (a cancel or a failure leaves
/// nothing). Checks first that `dest_dir` has room for it.
pub(crate) fn export(
    root: &Path,
    dest_dir: &Path,
    app_version: &str,
    hooks: &Hooks,
) -> AppResult<Exported> {
    let user_names = users::list_user_dirs(root)?;
    (hooks.progress)(Progress {
        phase: Phase::Scanning,
        files_done: 0,
        files_total: 0,
        bytes_done: 0,
        bytes_total: 0,
    });
    let entries = collect(root, &user_names)?;
    let files_total = entries.len() as u64;
    let bytes_total: u64 = entries.iter().map(|e| e.size).sum();
    (hooks.log)(format!(
        "[export_backup] {} user(s), {files_total} file(s), {} → {}",
        user_names.len(),
        human_size(bytes_total),
        dest_dir.display()
    ));
    if !dest_dir.is_dir() {
        return Err(AppError::NotFound(format!(
            "The folder {} isn't there",
            dest_dir.display()
        )));
    }
    if let Some(user) = user_folder_holding(root, dest_dir) {
        return Err(AppError::InvalidInput(format!(
            "The backup can't go in {user}'s library folder: choose a folder outside the library."
        )));
    }
    check_space(dest_dir, bytes_total, "the backup")?;

    let temp = tempfile::Builder::new()
        .prefix(".librarium-backup-")
        .suffix(".zip.part")
        .tempfile_in(dest_dir)
        .map_err(|e| AppError::io("Could not create the backup file", &e))?;
    let mut zip = ZipWriter::new(BufWriter::new(
        temp.as_file()
            .try_clone()
            .map_err(|e| AppError::io("Could not create the backup file", &e))?,
    ));
    let fail = |e: &dyn std::fmt::Display| AppError::Io(format!("Could not write the backup: {e}"));

    // Each user's folder, even an empty one, is restored.
    for user in &user_names {
        zip.add_directory(
            format!("{LIBRARY_PREFIX}{user}/"),
            dated(SimpleFileOptions::default(), None),
        )
        .map_err(|e| fail(&e))?;
    }
    let mut progress = Progress {
        phase: Phase::Copying,
        files_done: 0,
        files_total,
        bytes_done: 0,
        bytes_total,
    };
    let mut buf = vec![0u8; CHUNK];
    for entry in &entries {
        if (hooks.is_cancelled)() {
            return Ok(cancelled_export(&user_names));
        }
        let options = SimpleFileOptions::default()
            .compression_method(compression_for(&entry.path))
            .large_file(entry.size >= u64::from(u32::MAX));
        let options = dated(options, entry.modified);
        zip.start_file(entry.name.as_str(), options)
            .map_err(|e| fail(&e))?;
        let mut file = File::open(&entry.path)
            .map_err(|e| AppError::io(format!("Could not read {}", entry.path.display()), &e))?;
        loop {
            if (hooks.is_cancelled)() {
                return Ok(cancelled_export(&user_names));
            }
            let n = match file.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    return Err(AppError::io(
                        format!("Could not read {}", entry.path.display()),
                        &e,
                    ))
                }
            };
            zip.write_all(&buf[..n]).map_err(|e| fail(&e))?;
            progress.bytes_done += n as u64;
            (hooks.progress)(progress.clone());
        }
        progress.files_done += 1;
        (hooks.progress)(progress.clone());
    }

    let manifest = BackupManifest {
        app: "Librarium".into(),
        format: FORMAT,
        app_version: app_version.into(),
        created_at: Utc::now().to_rfc3339(),
        users: user_names.clone(),
        files: files_total,
        bytes: bytes_total,
    };
    zip.start_file(
        MANIFEST,
        dated(
            SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
            None,
        ),
    )
    .map_err(|e| fail(&e))?;
    let json =
        serde_json::to_vec_pretty(&manifest).map_err(|e| AppError::Internal(e.to_string()))?;
    zip.write_all(&json).map_err(|e| fail(&e))?;
    let mut out = zip.finish().map_err(|e| fail(&e))?;
    out.flush().map_err(|e| fail(&e))?;
    drop(out);
    fsutil::sync_file(temp.as_file()).map_err(|e| fail(&e))?;

    let dest = fsutil::unique_path(dest_dir, &backup_file_name(Local::now()));
    temp.persist(&dest)
        .map_err(|e| AppError::io(format!("Could not save {}", dest.display()), &e.error))?;
    (hooks.log)(format!("[export_backup] saved {}", dest.display()));
    Ok(Exported {
        path: Some(dest.to_string_lossy().into_owned()),
        users: user_names,
        files: files_total,
        bytes: bytes_total,
        cancelled: false,
    })
}

fn cancelled_export(users: &[String]) -> Exported {
    Exported {
        path: None,
        users: users.to_vec(),
        files: 0,
        bytes: 0,
        cancelled: true,
    }
}

// ── Reading ───────────────────────────────────────────────────────────────────

fn not_a_backup(path: &Path, why: impl std::fmt::Display) -> AppError {
    AppError::InvalidInput(format!(
        "{} isn't a Librarium backup ({why}).",
        crate::error::display_name(path)
    ))
}

fn open_archive(path: &Path) -> AppResult<ZipArchive<File>> {
    let file = File::open(path)
        .map_err(|e| AppError::io(format!("Could not open {}", path.display()), &e))?;
    ZipArchive::new(file).map_err(|e| not_a_backup(path, e))
}

fn manifest_of<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    path: &Path,
) -> AppResult<BackupManifest> {
    let mut text = String::new();
    archive
        .by_name(MANIFEST)
        .map_err(|_| not_a_backup(path, "no manifest"))?
        .read_to_string(&mut text)
        .map_err(|e| not_a_backup(path, e))?;
    let manifest: BackupManifest =
        serde_json::from_str(&text).map_err(|e| not_a_backup(path, e))?;
    if manifest.app != "Librarium" {
        return Err(not_a_backup(path, "made by another app"));
    }
    if manifest.format > FORMAT {
        return Err(AppError::InvalidInput(format!(
            "{} was made by a newer Librarium ({}): update Librarium to restore it.",
            crate::error::display_name(path),
            manifest.app_version
        )));
    }
    Ok(manifest)
}

/// What the backup at `path` holds (its manifest), to say before restoring.
pub(crate) fn read_manifest(path: &Path) -> AppResult<BackupManifest> {
    manifest_of(&mut open_archive(path)?, path)
}

// ── Restore ───────────────────────────────────────────────────────────────────

/// Where an entry of the zip goes under the staging folder: `user/rel`, or
/// `None` for anything that isn't a user's file (the manifest, a name that
/// would leave the folder, a hidden or system top folder).
fn restore_target(enclosed: &Path) -> Option<PathBuf> {
    let rel = enclosed
        .strip_prefix(LIBRARY_PREFIX.trim_end_matches('/'))
        .ok()?;
    let mut parts = rel.components();
    let Some(Component::Normal(user)) = parts.next() else {
        return None;
    };
    if !users::is_user_folder_name(user.to_str()?) {
        return None;
    }
    if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
        return None;
    }
    Some(rel.to_path_buf())
}

/// Replaces `root`'s users with the backup's (see the module docs for the
/// order that keeps the library whole). A cancel while extracting leaves
/// the library as it was.
pub(crate) fn restore(root: &Path, path: &Path, hooks: &Hooks) -> AppResult<Restored> {
    if let Some(user) = user_folder_holding(root, path) {
        return Err(AppError::InvalidInput(format!(
            "This backup is in {user}'s library folder, which the restore would erase: move it out of the library first."
        )));
    }
    let mut archive = open_archive(path)?;
    let manifest = manifest_of(&mut archive, path)?;
    (hooks.progress)(Progress {
        phase: Phase::Scanning,
        files_done: 0,
        files_total: manifest.files,
        bytes_done: 0,
        bytes_total: manifest.bytes,
    });
    fs::create_dir_all(root).map_err(|e| AppError::folder(root, &e))?;
    // The current users must be readable before anything is decided.
    let current = users::list_user_dirs(root)?;
    let mut bytes_total = 0u64;
    for i in 0..archive.len() {
        if let Ok(entry) = archive.by_index_raw(i) {
            bytes_total = bytes_total.saturating_add(entry.size());
        }
    }
    check_space(root, bytes_total, "the restored library")?;
    (hooks.log)(format!(
        "[restore_backup] {} → {}: {} user(s), {} file(s); replacing {} user(s)",
        path.display(),
        root.display(),
        manifest.users.len(),
        manifest.files,
        current.len()
    ));

    let id = uuid::Uuid::new_v4().simple().to_string();
    let staging = root.join(format!(".librarium-restore-{id}"));
    fs::create_dir(&staging).map_err(|e| AppError::io("Could not prepare the restore", &e))?;
    let cleanup = |dir: &Path| {
        if dir.exists() {
            if let Err(e) = fs::remove_dir_all(dir) {
                (hooks.log)(format!(
                    "[restore_backup] WARNING — could not remove {}: {e}",
                    dir.display()
                ));
            }
        }
    };

    // 1. Everything into the staging folder.
    let extracted = extract(&mut archive, &staging, bytes_total, &manifest, hooks);
    let files = match extracted {
        Ok(Some(files)) => files,
        Ok(None) => {
            cleanup(&staging);
            (hooks.log)("[restore_backup] cancelled: nothing was changed".into());
            return Ok(Restored {
                users: current,
                files: 0,
                cancelled: true,
            });
        }
        Err(e) => {
            cleanup(&staging);
            return Err(e);
        }
    };
    let restored: Vec<String> = read_users(&staging)?;

    // 2. The current users aside, the backup's in.
    (hooks.progress)(Progress {
        phase: Phase::Replacing,
        files_done: files,
        files_total: files,
        bytes_done: bytes_total,
        bytes_total,
    });
    let previous = root.join(format!(".librarium-restore-{id}-previous"));
    if let Err(e) = swap(root, &staging, &previous, &current, &restored) {
        cleanup(&staging);
        return Err(e);
    }

    // 3. The previous users go.
    cleanup(&previous);
    cleanup(&staging);
    (hooks.log)(format!(
        "[restore_backup] done: {} user(s), {files} file(s)",
        restored.len()
    ));
    Ok(Restored {
        users: restored,
        files,
        cancelled: false,
    })
}

/// Extracts the users' files into `staging`; `None` when cancelled.
fn extract<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    staging: &Path,
    bytes_total: u64,
    manifest: &BackupManifest,
    hooks: &Hooks,
) -> AppResult<Option<u64>> {
    let mut progress = Progress {
        phase: Phase::Copying,
        files_done: 0,
        files_total: manifest.files,
        bytes_done: 0,
        bytes_total,
    };
    let mut buf = vec![0u8; CHUNK];
    for i in 0..archive.len() {
        if (hooks.is_cancelled)() {
            return Ok(None);
        }
        let mut entry = archive
            .by_index(i)
            .map_err(|e| AppError::Io(format!("The backup is damaged ({e})")))?;
        let Some(target) = entry.enclosed_name().as_deref().and_then(restore_target) else {
            let name = entry.name().map(|n| n.into_owned()).unwrap_or_default();
            if name != MANIFEST {
                (hooks.log)(format!("[restore_backup] skipped {name:?}"));
            }
            continue;
        };
        let dest = staging.join(&target);
        if entry.is_dir() {
            fs::create_dir_all(&dest)
                .map_err(|e| AppError::io("Could not restore a folder", &e))?;
            continue;
        }
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| AppError::io("Could not restore a folder", &e))?;
        }
        let mut out = File::create(&dest)
            .map_err(|e| AppError::io(format!("Could not restore {}", target.display()), &e))?;
        loop {
            if (hooks.is_cancelled)() {
                return Ok(None);
            }
            let n = match entry.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(AppError::Io(format!("The backup is damaged ({e})"))),
            };
            out.write_all(&buf[..n])
                .map_err(|e| AppError::io(format!("Could not restore {}", target.display()), &e))?;
            progress.bytes_done += n as u64;
            (hooks.progress)(progress.clone());
        }
        if let Some(time) = entry.last_modified().and_then(file_time) {
            // Best effort: a share may refuse it, the file is there anyway.
            let _ = out.set_modified(time);
        }
        fsutil::sync_file(&out)
            .map_err(|e| AppError::io(format!("Could not restore {}", target.display()), &e))?;
        progress.files_done += 1;
        (hooks.progress)(progress.clone());
    }
    Ok(Some(progress.files_done))
}

/// The user folders extracted into `staging`.
fn read_users(staging: &Path) -> AppResult<Vec<String>> {
    let mut names: Vec<String> = fs::read_dir(staging)
        .map_err(|e| AppError::io("Could not read the restored users", &e))?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    Ok(names)
}

/// Moves the `current` users of `root` into `previous`, then the `restored`
/// ones from `staging` into `root`. On a failure, puts back what it moved.
fn swap(
    root: &Path,
    staging: &Path,
    previous: &Path,
    current: &[String],
    restored: &[String],
) -> AppResult<()> {
    fs::create_dir(previous).map_err(|e| AppError::io("Could not prepare the restore", &e))?;
    let mut moved_aside: Vec<&String> = Vec::new();
    for user in current {
        if let Err(e) = fs::rename(root.join(user), previous.join(user)) {
            for back in moved_aside {
                let _ = fs::rename(previous.join(back), root.join(back));
            }
            let _ = fs::remove_dir(previous);
            return Err(AppError::io(
                format!("Could not move the user {user} aside (nothing was changed)"),
                &e,
            ));
        }
        moved_aside.push(user);
    }
    let mut moved_in: Vec<&String> = Vec::new();
    for user in restored {
        if let Err(e) = fs::rename(staging.join(user), root.join(user)) {
            for done in moved_in {
                let _ = fs::rename(root.join(done), staging.join(done));
            }
            for back in moved_aside {
                let _ = fs::rename(previous.join(back), root.join(back));
            }
            let _ = fs::remove_dir(previous);
            return Err(AppError::io(
                format!("Could not put the user {user} in place (nothing was changed)"),
                &e,
            ));
        }
        moved_in.push(user);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging::no_log;

    fn hooks(cancel: &dyn Fn() -> bool) -> Hooks<'_> {
        Hooks {
            progress: &|_| {},
            is_cancelled: cancel,
            log: &no_log,
        }
    }

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    /// A library root: two users (one empty), Librarium's data, things a
    /// backup leaves out, and things in the root that aren't users.
    fn library() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        let r = root.path();
        write(&r.join("Alice/Dune.epub"), "dune");
        write(&r.join("Alice/SF/Hyperion.pdf"), "hyperion");
        write(&r.join("Alice/.librarium/library.json"), "{\"books\":1}");
        write(&r.join("Alice/.covers/x.jpg"), "jpg");
        write(&r.join("Alice/.librarium/trash/old.epub"), "trashed");
        write(&r.join("Alice/.librarium/lock.json"), "lock");
        write(&r.join("Alice/.DS_Store"), "junk");
        write(&r.join("Alice/.librarium-convert-1.azw3"), "temp");
        fs::create_dir(r.join("Clara")).unwrap();
        write(&r.join("#recycle/deleted.epub"), "nas trash");
        write(&r.join("notes.txt"), "not a user");
        root
    }

    fn names_in(zip: &Path) -> Vec<String> {
        let mut archive = ZipArchive::new(File::open(zip).unwrap()).unwrap();
        let mut names: Vec<String> = (0..archive.len())
            .map(|i| archive.by_index(i).unwrap().name().unwrap().into_owned())
            .collect();
        names.sort();
        names
    }

    fn never() -> bool {
        false
    }

    fn export_of(root: &Path, dest: &Path) -> PathBuf {
        let done = export(root, dest, "0.6.0", &hooks(&never)).unwrap();
        assert!(!done.cancelled);
        PathBuf::from(done.path.unwrap())
    }

    #[test]
    fn a_backup_holds_every_user_and_their_data_and_nothing_else() {
        let root = library();
        let dest = tempfile::tempdir().unwrap();
        let zip = export_of(root.path(), dest.path());
        assert!(zip
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("Librarium backup "));
        assert_eq!(
            names_in(&zip),
            vec![
                MANIFEST,
                "library/Alice/",
                "library/Alice/.covers/x.jpg",
                "library/Alice/.librarium/library.json",
                "library/Alice/Dune.epub",
                "library/Alice/SF/Hyperion.pdf",
                "library/Clara/",
            ]
        );
        let manifest = read_manifest(&zip).unwrap();
        assert_eq!(manifest.app, "Librarium");
        assert_eq!(manifest.format, FORMAT);
        assert_eq!(manifest.app_version, "0.6.0");
        assert_eq!(manifest.users, vec!["Alice", "Clara"]);
        assert_eq!(manifest.files, 4);
        assert_eq!(manifest.bytes, 4 + 8 + 11 + 3);
        // Nothing but the backup in the destination (no temp file).
        assert_eq!(fs::read_dir(dest.path()).unwrap().count(), 1);
    }

    #[test]
    fn a_restore_replaces_every_user_and_leaves_the_rest_of_the_root_alone() {
        let root = library();
        let dest = tempfile::tempdir().unwrap();
        let zip = export_of(root.path(), dest.path());
        let r = root.path();
        // After the backup: a book changed, one added, a new user.
        write(&r.join("Alice/Dune.epub"), "edited");
        write(&r.join("Alice/New.epub"), "new");
        write(&r.join("Zoé/Book.epub"), "zoé's");
        let restored = restore(r, &zip, &hooks(&never)).unwrap();
        assert!(!restored.cancelled);
        assert_eq!(restored.users, vec!["Alice", "Clara"]);
        assert_eq!(restored.files, 4);
        assert_eq!(
            fs::read_to_string(r.join("Alice/Dune.epub")).unwrap(),
            "dune"
        );
        assert_eq!(
            fs::read_to_string(r.join("Alice/SF/Hyperion.pdf")).unwrap(),
            "hyperion"
        );
        assert_eq!(
            fs::read_to_string(r.join("Alice/.librarium/library.json")).unwrap(),
            "{\"books\":1}"
        );
        assert!(!r.join("Alice/New.epub").exists());
        assert!(
            !r.join("Zoé").exists(),
            "a user not in the backup is erased"
        );
        assert!(r.join("Clara").is_dir(), "an empty user comes back");
        // Not users: untouched. No staging folder left.
        assert_eq!(
            fs::read_to_string(r.join("#recycle/deleted.epub")).unwrap(),
            "nas trash"
        );
        assert_eq!(
            fs::read_to_string(r.join("notes.txt")).unwrap(),
            "not a user"
        );
        let hidden: Vec<String> = fs::read_dir(r)
            .unwrap()
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter(|n| n.starts_with(".librarium-restore"))
            .collect();
        assert!(hidden.is_empty(), "{hidden:?}");
    }

    #[test]
    fn a_backup_never_goes_in_a_user_folder() {
        let root = library();
        let r = root.path();
        let err = export(r, &r.join("Alice/SF"), "0.6.0", &hooks(&never)).unwrap_err();
        assert_eq!(err.code(), "INVALID_INPUT");
        assert!(err.to_string().contains("Alice's library folder"), "{err}");

        // In the root itself (not a user's folder): fine, and a restore from
        // there leaves it, as the root's other files.
        let zip = export_of(r, r);
        restore(r, &zip, &hooks(&never)).unwrap();
        assert!(zip.is_file());

        let inside = r.join("Clara/backup.zip");
        fs::copy(&zip, &inside).unwrap();
        let err = restore(r, &inside, &hooks(&never)).unwrap_err();
        assert_eq!(err.code(), "INVALID_INPUT");
        assert!(
            err.to_string().contains("move it out of the library"),
            "{err}"
        );
        assert!(inside.is_file());
    }

    #[test]
    fn files_keep_their_dates() {
        let root = library();
        let dune = root.path().join("Alice/Dune.epub");
        let then = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        File::options()
            .write(true)
            .open(&dune)
            .unwrap()
            .set_modified(then)
            .unwrap();
        let dest = tempfile::tempdir().unwrap();
        let zip = export_of(root.path(), dest.path());
        write(&dune, "edited");
        restore(root.path(), &zip, &hooks(&never)).unwrap();
        // 1 700 000 000 is an even second: zip dates keep it exactly.
        assert_eq!(fs::metadata(&dune).unwrap().modified().unwrap(), then);
    }

    #[test]
    fn a_restore_onto_a_new_computer_creates_the_root() {
        let root = library();
        let dest = tempfile::tempdir().unwrap();
        let zip = export_of(root.path(), dest.path());
        let fresh = tempfile::tempdir().unwrap();
        let new_root = fresh.path().join("Librarium");
        restore(&new_root, &zip, &hooks(&never)).unwrap();
        assert_eq!(
            fs::read_to_string(new_root.join("Alice/Dune.epub")).unwrap(),
            "dune"
        );
    }

    #[test]
    fn cancelling_leaves_nothing_behind() {
        let root = library();
        let dest = tempfile::tempdir().unwrap();
        let cancel_now = || true;
        let done = export(root.path(), dest.path(), "0.6.0", &hooks(&cancel_now)).unwrap();
        assert!(done.cancelled);
        assert_eq!(done.path, None);
        assert_eq!(
            fs::read_dir(dest.path()).unwrap().count(),
            0,
            "no temp file left"
        );

        let zip = export_of(root.path(), dest.path());
        write(&root.path().join("Alice/Dune.epub"), "edited");
        let restored = restore(root.path(), &zip, &hooks(&cancel_now)).unwrap();
        assert!(restored.cancelled);
        assert_eq!(
            fs::read_to_string(root.path().join("Alice/Dune.epub")).unwrap(),
            "edited"
        );
        assert!(!fs::read_dir(root.path()).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".librarium-restore")));
    }

    #[test]
    fn only_librarium_backups_are_read() {
        let dir = tempfile::tempdir().unwrap();
        let not_zip = dir.path().join("notes.zip");
        fs::write(&not_zip, "not a zip").unwrap();
        assert_eq!(read_manifest(&not_zip).unwrap_err().code(), "INVALID_INPUT");

        let other = dir.path().join("other.zip");
        let mut zip = ZipWriter::new(File::create(&other).unwrap());
        zip.start_file("hello.txt", SimpleFileOptions::default())
            .unwrap();
        zip.finish().unwrap();
        let err = read_manifest(&other).unwrap_err();
        assert!(
            err.to_string().contains("isn't a Librarium backup"),
            "{err}"
        );

        let newer = dir.path().join("newer.zip");
        let mut zip = ZipWriter::new(File::create(&newer).unwrap());
        zip.start_file(MANIFEST, SimpleFileOptions::default())
            .unwrap();
        let manifest = BackupManifest {
            app: "Librarium".into(),
            format: FORMAT + 1,
            app_version: "9.0.0".into(),
            created_at: String::new(),
            users: vec![],
            files: 0,
            bytes: 0,
        };
        zip.write_all(&serde_json::to_vec(&manifest).unwrap())
            .unwrap();
        zip.finish().unwrap();
        let err = read_manifest(&newer).unwrap_err();
        assert!(err.to_string().contains("newer Librarium (9.0.0)"), "{err}");
    }

    #[test]
    fn entries_outside_a_user_folder_are_never_written() {
        assert_eq!(
            restore_target(Path::new("library/Alice/SF/x.epub")),
            Some(PathBuf::from("Alice/SF/x.epub"))
        );
        assert_eq!(
            restore_target(Path::new("library/Alice")),
            Some(PathBuf::from("Alice"))
        );
        for refused in [
            "library/.hidden/x",
            "library/#recycle/x",
            "library/@eaDir/x",
            "other/x",
            MANIFEST,
            "library",
        ] {
            assert_eq!(restore_target(Path::new(refused)), None, "{refused}");
        }

        // A hostile zip: names that climb out, beside a real backup's manifest.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        fs::create_dir(&root).unwrap();
        let evil = dir.path().join("evil.zip");
        let mut zip = ZipWriter::new(File::create(&evil).unwrap());
        for name in [
            "library/../escaped.txt",
            "library/Alice/../../escaped.txt",
            "/abs.txt",
            "library/Alice/ok.txt",
        ] {
            zip.start_file(name, SimpleFileOptions::default()).unwrap();
            zip.write_all(b"x").unwrap();
        }
        zip.start_file(MANIFEST, SimpleFileOptions::default())
            .unwrap();
        let manifest = BackupManifest {
            app: "Librarium".into(),
            format: FORMAT,
            app_version: "0.6.0".into(),
            created_at: String::new(),
            users: vec!["Alice".into()],
            files: 1,
            bytes: 1,
        };
        zip.write_all(&serde_json::to_vec(&manifest).unwrap())
            .unwrap();
        zip.finish().unwrap();
        restore(&root, &evil, &hooks(&never)).unwrap();
        assert!(!dir.path().join("escaped.txt").exists());
        assert!(!root.join("escaped.txt").exists());
        assert_eq!(fs::read_to_string(root.join("Alice/ok.txt")).unwrap(), "x");
    }

    #[test]
    fn file_names_and_storage() {
        let at = chrono::TimeZone::with_ymd_and_hms(&Local, 2026, 11, 10, 14, 32, 5).unwrap();
        assert_eq!(
            backup_file_name(at),
            "Librarium backup 2026-11-10 14.32.zip"
        );
        assert_eq!(
            compression_for(Path::new("a/library.json")),
            CompressionMethod::Deflated
        );
        assert_eq!(
            compression_for(Path::new("Dune.EPUB")),
            CompressionMethod::Stored
        );
        assert!(left_out(Path::new(".librarium/trash/x.epub")));
        assert!(left_out(Path::new("SF/._Dune.epub")));
        assert!(!left_out(Path::new(".librarium/library.json")));
        assert!(
            !left_out(Path::new("trash/Book.epub")),
            "a user's own folder named trash is kept"
        );
    }
}
