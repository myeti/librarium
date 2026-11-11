//! Sending one book to a device ([`transfer_one`]), and cleaning up after
//! interrupted transfers ([`clean_stale_parts`]).
//!
//! A job, in order:
//! 1. **Source.** With `bookId` + `libraryPath` the book is looked up in the
//!    library index (its file, title and author); otherwise `book_path` is
//!    used as is and the copy has no identity (older frontends).
//! 2. **Folder.** Where the device manifest says this book already is (so a
//!    Kindle's `.sdr` reading state next to it stays valid), else next to
//!    the title-matched `existing_device_path`, else `documents/<subfolder>`.
//! 3. **Payload.** Converted if needed ([`crate::convert`]: validated, temp
//!    file always removed); an AZW3 carries the book id as its ASIN, an EPUB
//!    copy an `urn:librarium:` identifier ([`super::identity`]). The library
//!    file is never modified.
//! 4. **Name.** FAT-safe. A file of that name is replaced if it is this book
//!    (per the manifest) or unknown to the manifest (copied there by hand,
//!    or by another tool); if it is *another* library
//!    book, the copy gets a `_2` name instead.
//! 5. **Copy.** Free space checked, then written through
//!    [`fsutil::atomic_write_with`] (hidden `.part`, fsync, rename) in 1 MB
//!    chunks with byte progress and cancellation between chunks: a pulled
//!    cable or a cancel leaves the previous file (or nothing), never a
//!    truncated book under its final name.
//! 6. **Manifest.** The entry is recorded; then the copy this book replaced,
//!    if it was elsewhere (another folder, another format), is deleted —
//!    only if the manifest said it was this book's, it is a book file inside
//!    the device, and it isn't the file just written ([`superseded_file`]).
//!    It goes by the manifest, never by title, so it can't remove another
//!    book.
//!
//! Everything here is blocking and `AppHandle`-free: the device is a
//! directory, so the tests use temp folders as fake devices.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use tempfile::TempPath;
use walkdir::WalkDir;

use super::detect::{kind_at, DeviceKind};
use super::listing::is_ebook;
use super::manifest::{self, path_key, relative_path, DeviceManifest, ManifestEntry, MANIFEST_DIR};
use crate::commands::library::covers::{self, is_safe_id};
use crate::commands::library::epub_edit::write_library_id;
use crate::commands::library::index::find_book;
use crate::convert::{convert_to_temp_with, ConvertOptions, OutputFormat};
use crate::error::{display_name, AppError};
use crate::fsutil;
use crate::logging::Log;
use crate::models::device::TransferRequest;

// ── Events and outcomes ───────────────────────────────────────────────────────

/// `status` values of the `transfer-progress` event (docs/IPC.md). `queued`
/// is set by the frontend when it creates the job, so it is never emitted here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TransferStatus {
    Converting,
    Transferring,
    Done,
    Error,
    Cancelled,
}

/// Payload of the `transfer-progress` event.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferProgress {
    pub job_id: String,
    pub status: TransferStatus,
    /// 0–100.
    pub progress: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

/// How a transfer job ended.
#[derive(Debug, PartialEq, Eq)]
pub enum JobOutcome {
    /// The book was written to this path on the device.
    Done(PathBuf),
    Cancelled,
    Failed(String),
}

/// Everything [`transfer_one`] needs from the outside world, injectable so
/// tests can run it against a temp directory acting as a fake device.
pub struct JobContext<'a> {
    /// Polled at every checkpoint; `true` stops the job.
    pub is_cancelled: &'a dyn Fn() -> bool,
    /// Receives every progress update, including the final one.
    pub emit: &'a dyn Fn(TransferProgress),
    pub log: Log<'a>,
    /// Free bytes on the volume holding a directory; `None` when unknown.
    pub free_space: &'a dyn Fn(&Path) -> Option<u64>,
    /// Where conversion output and EPUB copies are prepared before they are
    /// copied to the device.
    pub temp_dir: &'a Path,
}

impl JobContext<'_> {
    pub(super) fn report(
        &self,
        job: &TransferRequest,
        status: TransferStatus,
        progress: u8,
        error: Option<String>,
    ) {
        (self.emit)(TransferProgress {
            job_id: job.id.clone(),
            status,
            progress,
            error_message: error,
        });
    }
}

/// Why a job stopped early (internal; see [`JobOutcome`]).
pub(super) enum Stop {
    Cancelled,
    Failed(String),
}

impl From<String> for Stop {
    fn from(msg: String) -> Self {
        Stop::Failed(msg)
    }
}

/// A conversion or lookup failure: the job fails with its message (the
/// `transfer-progress` event carries a plain `errorMessage`).
impl From<AppError> for Stop {
    fn from(err: AppError) -> Self {
        Stop::Failed(err.to_string())
    }
}

/// Size of each read/write while copying to the device; cancellation and
/// progress are checked between chunks.
pub(crate) const COPY_CHUNK: usize = 1 << 20;

/// Extra free space required on top of the file size: FAT rounds files up to
/// whole clusters, and the device needs a little room for its own index.
pub(crate) const SPACE_MARGIN: u64 = 1 << 20;

// ── One job ───────────────────────────────────────────────────────────────────

/// Convert (if needed) and copy one book to the device, reporting progress
/// through `ctx.emit` and always ending with a `done`, `error` or `cancelled`
/// update. Blocking: run it on the blocking thread pool. See the module
/// docs for the steps.
pub fn transfer_one(job: &TransferRequest, ctx: &JobContext) -> JobOutcome {
    finish(job, ctx, run_job(job, ctx))
}

/// Log and report how a job ended (the last `transfer-progress` event of a
/// job); shared with MTP transfers (`mtp/send.rs`).
pub(super) fn finish(
    job: &TransferRequest,
    ctx: &JobContext,
    result: Result<PathBuf, Stop>,
) -> JobOutcome {
    let outcome = match result {
        Ok(dest) => JobOutcome::Done(dest),
        Err(Stop::Cancelled) => JobOutcome::Cancelled,
        Err(Stop::Failed(msg)) => JobOutcome::Failed(msg),
    };
    match &outcome {
        JobOutcome::Done(dest) => {
            (ctx.log)(format!(
                "[send_books] job {} done → {}",
                job.id,
                dest.display()
            ));
            ctx.report(job, TransferStatus::Done, 100, None);
        }
        JobOutcome::Cancelled => {
            (ctx.log)(format!("[send_books] job {} cancelled", job.id));
            ctx.report(job, TransferStatus::Cancelled, 0, None);
        }
        JobOutcome::Failed(msg) => {
            (ctx.log)(format!("[send_books] ERROR job {}: {}", job.id, msg));
            ctx.report(job, TransferStatus::Error, 0, Some(msg.clone()));
        }
    }
    outcome
}

/// The library book a job sends, when the frontend said which one.
pub(super) struct Identity {
    pub book_id: String,
    pub title: String,
    pub author: String,
    /// When the book was added to the library (Unix seconds): the fixed
    /// date of its converted copies (see [`ConvertOptions::timestamp`]).
    pub added: Option<u32>,
}

/// The file a job sends, and which library book it is.
pub(super) struct Source {
    pub path: PathBuf,
    pub identity: Option<Identity>,
}

fn run_job(job: &TransferRequest, ctx: &JobContext) -> Result<PathBuf, Stop> {
    let check_cancel = || {
        if (ctx.is_cancelled)() {
            Err(Stop::Cancelled)
        } else {
            Ok(())
        }
    };
    check_cancel()?;

    let source = resolve_source(job, ctx.log)?;
    let src = &source.path;
    (ctx.log)(format!(
        "[send_books] job {} — src: {:?} book id: {} needs_conversion: {} target: {}",
        job.id,
        src,
        source
            .identity
            .as_ref()
            .map_or("none (sent without identity)", |i| i.book_id.as_str()),
        job.needs_conversion,
        job.target_format
    ));
    if !src.is_file() {
        return Err(format!("Source file not found: {}", display_name(src)).into());
    }

    let device_root = PathBuf::from(&job.device_path);
    if kind_at(&device_root).is_none() {
        return Err(format!(
            "{} doesn't look like a Kindle or Kobo (no documents or .kobo folder)",
            device_root.display()
        )
        .into());
    }

    // What the manifest knows, for planning. It is read again, under its
    // lock, when the transfer is recorded.
    let manifest = manifest::load(&device_root).manifest;
    let book_id = source.identity.as_ref().map(|i| i.book_id.as_str());
    let previous = book_id
        .and_then(|id| manifest.entry(id))
        .and_then(|e| manifest::resolve(&device_root, &e.path));

    let dest_dir = resolve_dest_dir(job, &device_root, previous.as_deref(), ctx.log)?;
    fs::create_dir_all(&dest_dir).map_err(|e| {
        format!(
            "Cannot create folder {} on the device: {}",
            dest_dir.display(),
            e
        )
    })?;

    // A temp payload is deleted when dropped: on success, error,
    // cancellation or panic.
    let (payload, dest_name) = prepare_payload(&source, job, ctx)?;
    check_cancel()?;

    let dest = choose_dest(&dest_dir, &dest_name, &device_root, book_id, &manifest);
    if !fsutil::is_within(&device_root, &dest) {
        return Err(format!("Refusing to write outside the device: {}", dest.display()).into());
    }

    let size = fs::metadata(payload.path())
        .map_err(|e| format!("Cannot read {}: {}", payload.path().display(), e))?
        .len();

    // A re-sent book that hasn't changed is left alone: rewriting it, even
    // with the same bytes, makes a Kindle forget the reading position.
    // (Conversions of a library book are byte-identical, see
    // `ConvertOptions::timestamp`.)
    if dest.is_file() && fsutil::files_identical(payload.path(), &dest) {
        (ctx.log)(format!(
            "[send_books] {} is already on the device, unchanged: not copied again",
            display_name(&dest)
        ));
    } else {
        check_free_space(size, (ctx.free_space)(&dest_dir))?;
        let start = if job.needs_conversion { 60 } else { 30 };
        (ctx.log)(format!(
            "[send_books] copying {:?} → {:?}",
            payload.path(),
            dest
        ));
        ctx.report(job, TransferStatus::Transferring, start, None);
        copy_to_device(payload.path(), &dest, size, start, job, ctx)?;
    }
    drop(payload);

    if let Some(identity) = &source.identity {
        record_sent(&device_root, identity, &dest, size, ctx.log);
        write_kindle_cover(job, identity, &device_root, &dest, ctx.log);
    }
    Ok(dest)
}

/// Give a book sent to a Kindle its home-screen cover, from the library's
/// cover of that book (see `thumbnail.rs`: the Kindle doesn't make one for
/// books with an ASIN). Best effort: logged, never fails the transfer.
fn write_kindle_cover(
    job: &TransferRequest,
    identity: &Identity,
    mount: &Path,
    dest: &Path,
    log: Log,
) {
    if kind_at(mount) != Some(DeviceKind::Kindle) {
        return;
    }
    let Some(library) = job.library_path.as_deref().map(Path::new) else {
        return;
    };
    let Some(cover) = covers::cover_image(library, &identity.book_id, log) else {
        log(format!(
            "[send_books] no cover for {} in the library: the Kindle shows a blank one",
            display_name(dest)
        ));
        return;
    };
    match super::thumbnail::write_for_book(mount, dest, &cover) {
        Ok(true) => log(format!(
            "[send_books] Kindle cover written for {}",
            display_name(dest)
        )),
        Ok(false) => {}
        Err(e) => log(format!(
            "[send_books] WARNING — Kindle cover for {}: {e}",
            display_name(dest)
        )),
    }
}

/// The file to send: from the library index when the job names the book
/// (`book_id` + `library_path`), else `book_path` (no identity).
pub(super) fn resolve_source(job: &TransferRequest, log: Log) -> Result<Source, Stop> {
    let id = job
        .book_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let library = job
        .library_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    match (id, library) {
        (Some(id), Some(library)) => {
            if !is_safe_id(id) {
                return Err(format!("Invalid book id: {id:?}").into());
            }
            let book = find_book(Path::new(library), id, log)?;
            if !job.book_path.is_empty() && Path::new(&job.book_path) != Path::new(&book.path) {
                log(format!(
                    "[send_books] job {}: the library index has {} at {} (not {}); sending that file",
                    job.id, id, book.path, job.book_path
                ));
            }
            Ok(Source {
                path: PathBuf::from(&book.path),
                identity: Some(Identity {
                    added: unix_seconds(&book.date_added),
                    book_id: book.id,
                    title: book.metadata.title,
                    author: book.metadata.author,
                }),
            })
        }
        (None, None) => Ok(Source {
            path: PathBuf::from(&job.book_path),
            identity: None,
        }),
        _ => Err("A transfer needs both bookId and libraryPath, or neither"
            .to_string()
            .into()),
    }
}

/// An RFC 3339 date (a book's `dateAdded`) as Unix seconds, if it is one
/// that fits a MOBI header (1970–2106).
fn unix_seconds(date: &str) -> Option<u32> {
    let date = chrono::DateTime::parse_from_rfc3339(date.trim()).ok()?;
    u32::try_from(date.timestamp()).ok()
}

/// What gets copied to the device: the library file itself, or a temp file
/// (a conversion, or an EPUB copy carrying the library id).
pub(super) enum Payload {
    Library(PathBuf),
    Temp(TempPath),
}

impl Payload {
    pub(super) fn path(&self) -> &Path {
        match self {
            Payload::Library(p) => p,
            Payload::Temp(t) => t,
        }
    }
}

/// Prepare what to copy (see [`Payload`]) and the file name it gets on the
/// device.
pub(super) fn prepare_payload(
    source: &Source,
    job: &TransferRequest,
    ctx: &JobContext,
) -> Result<(Payload, String), Stop> {
    let src = &source.path;
    let book_id = source.identity.as_ref().map(|i| i.book_id.as_str());
    let options = ConvertOptions {
        asin: book_id,
        timestamp: source.identity.as_ref().and_then(|i| i.added),
    };
    let file_name = src
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();

    if job.needs_conversion {
        let format = OutputFormat::for_device(&job.target_format)?;
        ctx.report(job, TransferStatus::Converting, 10, None);
        let converted = convert_to_temp_with(src, format, ctx.temp_dir, options)?.into_temp_path();
        if let (OutputFormat::Epub, Some(id)) = (format, book_id) {
            tag_epub(&converted, id, ctx.log);
        }
        let stem = src
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        return Ok((
            Payload::Temp(converted),
            format!("{stem}.{}", format.extension()),
        ));
    }

    let is_epub = src
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("epub"));
    if let (true, Some(id)) = (is_epub, book_id) {
        let copy = copy_to_temp(src, ctx.temp_dir).map_err(|e| {
            format!(
                "Could not prepare {} for the device: {e}",
                display_name(src)
            )
        })?;
        tag_epub(&copy, id, ctx.log);
        return Ok((Payload::Temp(copy), file_name));
    }
    Ok((Payload::Library(src.clone()), file_name))
}

/// Copy `src` into a new hidden temp file in `dir`.
fn copy_to_temp(src: &Path, dir: &Path) -> io::Result<TempPath> {
    let mut tmp = tempfile::Builder::new()
        .prefix(".librarium-send-")
        .suffix(".epub")
        .tempfile_in(dir)?;
    io::copy(&mut File::open(src)?, tmp.as_file_mut())?;
    fsutil::sync_file(tmp.as_file())?;
    Ok(tmp.into_temp_path())
}

/// Add the library id to an EPUB about to be sent. Best effort: a book whose
/// OPF Librarium can't edit safely goes as it is (the manifest still knows
/// which book it is), with a warning in the log.
fn tag_epub(path: &Path, book_id: &str, log: Log) {
    if let Err(e) = write_library_id(path, book_id) {
        log(format!("[send_books] WARNING — the copy goes without its library id inside ({e}); the device manifest still has it"));
    }
}

/// Choose the destination folder for a job: the folder of this book's
/// previous copy (`previous`, from the manifest), else of the title-matched
/// `existing_device_path` the frontend sent, else `documents/<subfolder>`.
/// A previous folder is only used if it exists inside the device. Keeping a
/// re-sent book where it was keeps the Kindle's adjacent `.sdr` folder
/// (reading position, notes) valid.
fn resolve_dest_dir(
    job: &TransferRequest,
    device_root: &Path,
    previous: Option<&Path>,
    log: Log,
) -> Result<PathBuf, String> {
    let hint = job
        .existing_device_path
        .as_deref()
        .filter(|p| !p.is_empty())
        .map(Path::new);
    for (what, path) in [("previous copy", previous), ("existing_device_path", hint)] {
        let Some(path) = path else { continue };
        match path.parent() {
            Some(dir) if dir.is_dir() && fsutil::is_within(device_root, dir) => {
                return Ok(dir.to_path_buf())
            }
            _ => log(format!(
                "[send_books] ignoring the {what}'s folder (gone, or outside the device): {}",
                path.display()
            )),
        }
    }
    let mut dir = device_root.join("documents");
    if let Some(sub) = validate_subfolder(job.subfolder.as_deref())? {
        dir.push(sub);
    }
    Ok(dir)
}

/// Where in `dest_dir` the book goes: its FAT-safe name, unless a file of
/// that name is there and the manifest says it is *another* library book —
/// then the first free `name_2.ext`, `name_3.ext`… (another book is never
/// overwritten). A file that is this book, or that the manifest doesn't
/// know, is replaced.
fn choose_dest(
    dest_dir: &Path,
    name: &str,
    mount: &Path,
    book_id: Option<&str>,
    manifest: &DeviceManifest,
) -> PathBuf {
    let name = fsutil::sanitize_filename(name);
    let candidate = dest_dir.join(&name);
    let Some(id) = book_id else { return candidate };
    let another_book = relative_path(mount, &candidate)
        .and_then(|rel| manifest.owner_of(&rel).map(|owner| owner != id))
        .unwrap_or(false);
    if another_book && candidate.exists() {
        fsutil::unique_path(dest_dir, &name)
    } else {
        candidate
    }
}

/// Record the transfer in the device manifest, then delete the copy it
/// superseded (see [`superseded_file`]). Failures are logged, not fatal:
/// the book is on the device either way.
///
/// The superseded copy's Kindle `.sdr` folder (named after its old file
/// name) is left where it is: an orphan is harmless, a reading position
/// moved onto a different file may not be.
///
/// Checked on a Kindle Paperwhite 3 (2026-11-09): a re-sent book that is
/// rewritten (same name, `.sdr` left in place) LOSES its reading position.
/// So a book that hasn't changed is not rewritten at all (see [`run_job`]);
/// one that changed in the library still opens at the beginning.
/// TODO(kindle-resend-position): confirm on the Kindle that re-sending an
/// unchanged book keeps its position.
fn record_sent(mount: &Path, identity: &Identity, dest: &Path, size: u64, log: Log) {
    let Some(rel) = relative_path(mount, dest) else {
        log(format!(
            "[send_books] WARNING — {} isn't under the device root; not recorded",
            dest.display()
        ));
        return;
    };
    let entry = ManifestEntry {
        path: rel.clone(),
        title: identity.title.clone(),
        author: identity.author.clone(),
        format: dest
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default(),
        size_bytes: size,
        sent_at: chrono::Utc::now().to_rfc3339(),
    };
    let recorded = manifest::update(mount, log, |m| {
        let previous = m.books.remove(&identity.book_id);
        // Whatever else was recorded at this path was just overwritten.
        let replaced = m.remove_path(&rel);
        m.books.insert(identity.book_id.clone(), entry);
        // Another entry still pointing at the old copy: it isn't only ours.
        let shared = previous
            .as_ref()
            .is_some_and(|p| m.owner_of(&p.path).is_some_and(|o| o != identity.book_id));
        (previous, replaced, shared)
    });
    let (previous, replaced, shared) = match recorded {
        Ok(r) => r,
        Err(e) => {
            log(format!(
                "[send_books] WARNING — {} is on the device, but {e}; its previous copy (if any) was left in place",
                display_name(dest)
            ));
            return;
        }
    };
    if !replaced.is_empty() {
        log(format!(
            "[send_books] {} replaced the file recorded for {}",
            display_name(dest),
            replaced.join(", ")
        ));
    }
    let Some(old) = previous
        .filter(|_| !shared)
        .and_then(|p| superseded_file(mount, &p, &rel, dest))
    else {
        return;
    };
    match fs::remove_file(&old) {
        Ok(()) => log(format!(
            "[send_books] removed the previous copy {}",
            old.display()
        )),
        Err(e) => log(format!(
            "[send_books] WARNING — could not remove the previous copy {}: {e}",
            old.display()
        )),
    }
}

/// The previous copy of a re-sent book, if it must be deleted: the manifest
/// recorded it for this book (`previous`), its path is a plain relative path
/// inside the device, it is a book file (not a folder or link), and it is
/// not the file just written (`new_rel`/`new_dest`, compared both by name
/// as FAT does — case-insensitively — and by file identity). Anything
/// uncertain keeps the file: a leftover duplicate is harmless, deleting the
/// wrong file is not.
fn superseded_file(
    mount: &Path,
    previous: &ManifestEntry,
    new_rel: &str,
    new_dest: &Path,
) -> Option<PathBuf> {
    if path_key(&previous.path) == path_key(new_rel) {
        return None;
    }
    let old = manifest::resolve(mount, &previous.path)?;
    let meta = fs::symlink_metadata(&old).ok()?;
    let deletable = meta.is_file()
        && is_ebook(&old)
        && fsutil::is_within(mount, &old)
        && distinct_files(&old, new_dest);
    deletable.then_some(old)
}

/// Whether `a` and `b` are certainly two different files (`false` when it
/// can't be told).
fn distinct_files(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match (fs::metadata(a), fs::metadata(b)) {
            (Ok(x), Ok(y)) => (x.dev(), x.ino()) != (y.dev(), y.ino()),
            _ => false,
        }
    }
    #[cfg(not(unix))]
    {
        // Canonical paths carry the on-disk spelling on Windows.
        match (fs::canonicalize(a), fs::canonicalize(b)) {
            (Ok(x), Ok(y)) => {
                x.to_string_lossy().to_lowercase() != y.to_string_lossy().to_lowercase()
            }
            _ => false,
        }
    }
}

// ── Checks and helpers ────────────────────────────────────────────────────────

/// Validate the user's "device subfolder" setting and make it FAT-safe.
///
/// It must be relative (no leading `/` or `\`, no drive letter) and may not
/// contain `..`; `/` and `\` both separate components, empty and `.`
/// components are dropped, and each component is sanitised. Returns `None`
/// for an empty setting.
pub fn validate_subfolder(sub: Option<&str>) -> Result<Option<PathBuf>, String> {
    let Some(raw) = sub.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let has_drive =
        raw.len() >= 2 && raw.as_bytes()[1] == b':' && raw.as_bytes()[0].is_ascii_alphabetic();
    if raw.starts_with('/') || raw.starts_with('\\') || has_drive || Path::new(raw).is_absolute() {
        return Err(format!(
            "The device subfolder must be a relative path, not \"{raw}\""
        ));
    }
    let mut out = PathBuf::new();
    for comp in raw.split(['/', '\\']).map(str::trim) {
        if comp.is_empty() || comp == "." {
            continue;
        }
        if comp.chars().all(|c| c == '.') {
            return Err(format!("The device subfolder can't contain \"{comp}\""));
        }
        out.push(fsutil::sanitize_stem(comp, fsutil::MAX_FILENAME_BYTES));
    }
    Ok((!out.as_os_str().is_empty()).then_some(out))
}

/// Human-readable size for error messages ("12.3 MB").
pub(crate) fn format_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.0} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

/// Fail early, with a clear message, when the device can't hold `needed`
/// bytes (plus [`SPACE_MARGIN`]). An unknown free space lets the copy try.
pub(super) fn check_free_space(needed: u64, available: Option<u64>) -> Result<(), String> {
    let Some(free) = available else { return Ok(()) };
    let needed = needed.saturating_add(SPACE_MARGIN);
    if free < needed {
        Err(format!(
            "Not enough space on the device (needs {}, free {})",
            format_size(needed),
            format_size(free)
        ))
    } else {
        Ok(())
    }
}

/// Map `copied / total` onto the `start..=99` range of the progress bar
/// (100 is reserved for "done", after the rename).
pub(super) fn scale_progress(start: u8, copied: u64, total: u64) -> u8 {
    if total == 0 {
        return 99;
    }
    let span = u64::from(99u8.saturating_sub(start));
    let pct = u64::from(start) + span * copied.min(total) / total;
    pct.min(99) as u8
}

/// Copy `src` to `dest` through [`fsutil::atomic_write_with`], in chunks, so
/// a cancellation stops the copy between chunks and removes the `.part` file.
///
/// Validated on a Kindle Paperwhite 3 over USB on macOS (FAT32), 2026-11-09: new books and a re-sent book (replaced in place) were indexed like
/// plain copies.
/// TODO(device-validation): exFAT, Windows, and a Kobo.
fn copy_to_device(
    src: &Path,
    dest: &Path,
    total: u64,
    start: u8,
    job: &TransferRequest,
    ctx: &JobContext,
) -> Result<(), Stop> {
    let mut cancelled = false;
    let result = fsutil::atomic_write_with(dest, |out| {
        let mut input = File::open(src)?;
        let mut buf = vec![0u8; COPY_CHUNK];
        let mut copied = 0u64;
        let mut last = start;
        loop {
            if (ctx.is_cancelled)() {
                cancelled = true;
                return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
            }
            let n = match input.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            };
            out.write_all(&buf[..n])?;
            copied += n as u64;
            let pct = scale_progress(start, copied, total);
            if pct != last {
                ctx.report(job, TransferStatus::Transferring, pct, None);
                last = pct;
            }
        }
        Ok(())
    });
    match result {
        Ok(()) => Ok(()),
        Err(_) if cancelled => Err(Stop::Cancelled),
        Err(e) => Err(Stop::Failed(format!("Copy to device failed: {e}"))),
    }
}

// ── Interrupted transfers ─────────────────────────────────────────────────────

/// How old a `.part` file must be before it counts as left over by an
/// interrupted copy. A live copy rewrites its `.part` continuously, so this
/// only guards against a transfer that starts while the cleanup runs.
pub(crate) const STALE_PART_AGE: Duration = Duration::from_secs(5 * 60);

/// Whether `name` is a temp file of [`fsutil::atomic_write_with`]
/// (`.librarium-XXXXXX.part`), or macOS's `._` companion of one.
pub(super) fn is_partial_name(name: &str) -> bool {
    let name = name.strip_prefix("._").unwrap_or(name);
    name.starts_with(".librarium-") && name.ends_with(".part")
}

/// Remove the `.librarium-*.part` files an interrupted transfer (unplugged
/// device, crash) left on the device at `mount`, if they haven't been
/// modified for `min_age`. Only where Librarium writes: `documents/` (all
/// levels) and `.librarium/`. Returns what was removed. The caller makes
/// sure no transfer to this device is running.
pub(crate) fn clean_stale_parts(mount: &Path, min_age: Duration, log: Log) -> Vec<PathBuf> {
    let now = SystemTime::now();
    let walks = [
        WalkDir::new(mount.join("documents")).max_depth(12),
        WalkDir::new(mount.join(MANIFEST_DIR)).max_depth(1),
    ];
    let mut removed = Vec::new();
    for entry in walks
        .into_iter()
        .flat_map(|w| w.into_iter().filter_map(|e| e.ok()))
    {
        if !entry.file_type().is_file() || !is_partial_name(&entry.file_name().to_string_lossy()) {
            continue;
        }
        let stale = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age >= min_age);
        if !stale {
            continue;
        }
        match fs::remove_file(entry.path()) {
            Ok(()) => removed.push(entry.into_path()),
            Err(e) => log(format!(
                "[device] WARNING — could not remove the leftover {}: {e}",
                entry.path().display()
            )),
        }
    }
    removed
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::device::identity::{epub_details, kindle_details};
    use crate::commands::device::tests::{fake_kindle, fake_kobo, minimal_epub};
    use crate::commands::library::test_support::{scan, write_epub, OPF_PATH};
    use crate::logging::{no_log, LogLines};
    use std::cell::{Cell, RefCell};

    fn job(src: &Path, device: &Path) -> TransferRequest {
        TransferRequest {
            id: "job-1".into(),
            book_path: src.to_string_lossy().into_owned(),
            needs_conversion: false,
            target_format: "azw3".into(),
            device_path: device.to_string_lossy().into_owned(),
            subfolder: None,
            existing_device_path: None,
            book_id: None,
            library_path: None,
        }
    }

    /// A job for library book `file_name` of `lib` (scanned), by id.
    fn library_job(lib: &Path, file_name: &str, device: &Path) -> (TransferRequest, String) {
        let book = scan(lib)
            .remove(file_name)
            .unwrap_or_else(|| panic!("{file_name} not in the library"));
        let mut j = job(Path::new(&book.path), device);
        j.book_id = Some(book.id.clone());
        j.library_path = Some(lib.to_string_lossy().into_owned());
        (j, book.id)
    }

    /// Runs `transfer_one` with recording callbacks.
    struct Harness {
        events: RefCell<Vec<TransferProgress>>,
        cancel_at: Option<usize>,
        checks: Cell<usize>,
        free: Option<u64>,
        temp: tempfile::TempDir,
        lines: LogLines,
    }

    impl Harness {
        fn new() -> Self {
            Harness {
                events: RefCell::new(vec![]),
                cancel_at: None,
                checks: Cell::new(0),
                free: None,
                temp: tempfile::tempdir().unwrap(),
                lines: LogLines::default(),
            }
        }

        /// Report "cancelled" from the `n`-th cancellation check onwards (0-based).
        fn cancel_at(mut self, n: usize) -> Self {
            self.cancel_at = Some(n);
            self
        }

        fn run(&self, job: &TransferRequest) -> JobOutcome {
            let is_cancelled = || {
                let n = self.checks.get();
                self.checks.set(n + 1);
                self.cancel_at.is_some_and(|at| n >= at)
            };
            let emit = |p: TransferProgress| self.events.borrow_mut().push(p);
            let free = |_: &Path| self.free;
            let log = self.lines.sink();
            let ctx = JobContext {
                is_cancelled: &is_cancelled,
                emit: &emit,
                log: &log,
                free_space: &free,
                temp_dir: self.temp.path(),
            };
            transfer_one(job, &ctx)
        }

        fn statuses(&self) -> Vec<TransferStatus> {
            self.events.borrow().iter().map(|e| e.status).collect()
        }

        fn temp_is_empty(&self) -> bool {
            fs::read_dir(self.temp.path()).unwrap().next().is_none()
        }
    }

    /// Every file name under `dir`, recursively, relative to it.
    fn files_under(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = WalkDir::new(dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .map(|e| {
                e.path()
                    .strip_prefix(dir)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        names.sort();
        names
    }

    fn done(outcome: JobOutcome) -> PathBuf {
        match outcome {
            JobOutcome::Done(p) => p,
            other => panic!("expected done, got {other:?}"),
        }
    }

    // ── Copying (no identity) ──────────────────────────────────────────────

    #[test]
    fn copies_with_a_fat_safe_name_and_reports_progress() {
        let device = fake_kindle();
        let lib = tempfile::tempdir().unwrap();
        let src = lib.path().join("Dune: Messiah?.azw3");
        fs::write(&src, vec![7u8; 3 * COPY_CHUNK + 10]).unwrap();

        let h = Harness::new();
        let outcome = h.run(&job(&src, device.path()));

        let dest = device.path().join("documents/Dune_ Messiah_.azw3");
        assert_eq!(outcome, JobOutcome::Done(dest.clone()));
        assert_eq!(fs::read(&dest).unwrap(), fs::read(&src).unwrap());
        assert_eq!(
            files_under(device.path()),
            vec!["documents/Dune_ Messiah_.azw3"],
            "no .part left, no manifest without an id"
        );
        let events = h.events.borrow();
        assert_eq!(events.first().unwrap().status, TransferStatus::Transferring);
        assert!(
            events.windows(2).all(|w| w[0].progress <= w[1].progress),
            "progress never goes back"
        );
        assert!(
            events.len() > 3,
            "byte progress is reported during the copy"
        );
        assert_eq!(
            *events.last().unwrap(),
            TransferProgress {
                job_id: "job-1".into(),
                status: TransferStatus::Done,
                progress: 100,
                error_message: None,
            }
        );
    }

    #[test]
    fn replaces_a_previous_copy_with_the_same_name() {
        let device = fake_kindle();
        let lib = tempfile::tempdir().unwrap();
        let src = lib.path().join("Dune.azw3");
        fs::write(&src, b"new").unwrap();
        fs::write(device.path().join("documents/Dune.azw3"), b"old").unwrap();

        done(Harness::new().run(&job(&src, device.path())));
        assert_eq!(
            fs::read(device.path().join("documents/Dune.azw3")).unwrap(),
            b"new"
        );
    }

    #[test]
    fn never_deletes_the_title_matched_book() {
        // The frontend matched "existing" by title; it may be another book.
        let device = fake_kindle();
        let shelf = device.path().join("documents/Shelf");
        fs::create_dir(&shelf).unwrap();
        let other = shelf.join("Dune - Someone Else.epub");
        fs::write(&other, b"other book").unwrap();
        let lib = tempfile::tempdir().unwrap();
        let src = lib.path().join("Dune.azw3");
        fs::write(&src, b"dune").unwrap();

        let mut j = job(&src, device.path());
        j.existing_device_path = Some(other.to_string_lossy().into_owned());
        assert_eq!(
            done(Harness::new().run(&j)),
            shelf.join("Dune.azw3"),
            "same folder as the existing book"
        );
        assert_eq!(
            fs::read(&other).unwrap(),
            b"other book",
            "the other book is untouched"
        );
    }

    #[test]
    fn existing_path_outside_the_device_is_ignored() {
        let device = fake_kindle();
        let elsewhere = tempfile::tempdir().unwrap();
        let outside = elsewhere.path().join("Dune.azw3");
        fs::write(&outside, b"not on device").unwrap();
        let lib = tempfile::tempdir().unwrap();
        let src = lib.path().join("Dune.azw3");
        fs::write(&src, b"dune").unwrap();

        let mut j = job(&src, device.path());
        j.existing_device_path = Some(outside.to_string_lossy().into_owned());
        assert_eq!(
            done(Harness::new().run(&j)),
            device.path().join("documents/Dune.azw3")
        );
        assert_eq!(fs::read(&outside).unwrap(), b"not on device");
    }

    #[test]
    fn uses_the_sanitised_subfolder() {
        let device = fake_kindle();
        let lib = tempfile::tempdir().unwrap();
        let src = lib.path().join("Dune.azw3");
        fs::write(&src, b"dune").unwrap();

        let mut j = job(&src, device.path());
        j.subfolder = Some("Sci-Fi/Herbert: Dune".into());
        assert_eq!(
            done(Harness::new().run(&j)),
            device
                .path()
                .join("documents/Sci-Fi/Herbert_ Dune/Dune.azw3")
        );

        j.subfolder = Some("../../escape".into());
        assert!(matches!(Harness::new().run(&j), JobOutcome::Failed(_)));
    }

    #[test]
    fn subfolder_validation() {
        assert_eq!(validate_subfolder(None), Ok(None));
        assert_eq!(validate_subfolder(Some("   ")), Ok(None));
        assert_eq!(validate_subfolder(Some("./ //")), Ok(None));
        assert_eq!(
            validate_subfolder(Some("Books/SF")),
            Ok(Some(PathBuf::from("Books").join("SF")))
        );
        assert_eq!(
            validate_subfolder(Some("Books\\SF\\")),
            Ok(Some(PathBuf::from("Books").join("SF")))
        );
        assert_eq!(
            validate_subfolder(Some("a*b / c?.")),
            Ok(Some(PathBuf::from("a_b").join("c_")))
        );
        assert!(validate_subfolder(Some("/abs")).is_err());
        assert!(validate_subfolder(Some("\\abs")).is_err());
        assert!(validate_subfolder(Some("C:\\Books")).is_err());
        assert!(validate_subfolder(Some("C:Books")).is_err());
        assert!(validate_subfolder(Some("..")).is_err());
        assert!(validate_subfolder(Some("a/../b")).is_err());
        assert!(validate_subfolder(Some("a/.../b")).is_err());
    }

    #[test]
    fn fails_when_the_device_is_full() {
        let device = fake_kindle();
        let lib = tempfile::tempdir().unwrap();
        let src = lib.path().join("Big.pdf");
        fs::write(&src, vec![0u8; 4096]).unwrap();

        let mut h = Harness::new();
        h.free = Some(1000);
        let JobOutcome::Failed(msg) = h.run(&job(&src, device.path())) else {
            panic!("expected failure")
        };
        assert!(
            msg.starts_with("Not enough space on the device (needs "),
            "{msg}"
        );
        assert!(msg.contains("free 1000 B"), "{msg}");
        assert!(files_under(device.path()).is_empty());
        assert_eq!(h.statuses(), vec![TransferStatus::Error]);
    }

    #[test]
    fn free_space_check() {
        assert!(
            check_free_space(10, None).is_ok(),
            "unknown free space doesn't block"
        );
        assert!(check_free_space(10, Some(SPACE_MARGIN + 10)).is_ok());
        assert!(check_free_space(10, Some(SPACE_MARGIN + 9)).is_err());
        assert!(
            check_free_space(u64::MAX, Some(u64::MAX - 1)).is_err(),
            "no overflow"
        );
    }

    #[test]
    fn sizes_are_human_readable() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(2048), "2 KB");
        assert_eq!(format_size(5 * 1024 * 1024 + 300 * 1024), "5.3 MB");
        assert_eq!(format_size(3 * 1024 * 1024 * 1024), "3.0 GB");
    }

    #[test]
    fn progress_scale_stays_in_range() {
        assert_eq!(scale_progress(30, 0, 100), 30);
        assert_eq!(scale_progress(30, 50, 100), 64);
        assert_eq!(scale_progress(30, 100, 100), 99);
        assert_eq!(
            scale_progress(60, 200, 100),
            99,
            "a file that grew mid-copy"
        );
        assert_eq!(scale_progress(30, 0, 0), 99);
    }

    #[test]
    fn cancelled_before_start_writes_nothing() {
        let device = fake_kindle();
        let lib = tempfile::tempdir().unwrap();
        let src = lib.path().join("Dune.azw3");
        fs::write(&src, b"dune").unwrap();

        let h = Harness::new().cancel_at(0);
        assert_eq!(h.run(&job(&src, device.path())), JobOutcome::Cancelled);
        assert_eq!(h.statuses(), vec![TransferStatus::Cancelled]);
        assert!(files_under(device.path()).is_empty());
    }

    #[test]
    fn cancel_during_copy_leaves_no_partial_file() {
        let device = fake_kindle();
        let lib = tempfile::tempdir().unwrap();
        let src = lib.path().join("Big.azw3");
        fs::write(&src, vec![1u8; 4 * COPY_CHUNK]).unwrap();
        // Keep a previous copy to check it survives a cancelled update.
        fs::write(device.path().join("documents/Big.azw3"), b"previous").unwrap();

        // Checks: 0 = start, 1 = after "conversion", 2.. = copy chunks.
        let h = Harness::new().cancel_at(4);
        assert_eq!(h.run(&job(&src, device.path())), JobOutcome::Cancelled);
        assert_eq!(
            files_under(device.path()),
            vec!["documents/Big.azw3"],
            "no .part left"
        );
        assert_eq!(
            fs::read(device.path().join("documents/Big.azw3")).unwrap(),
            b"previous"
        );
        assert_eq!(h.statuses().last(), Some(&TransferStatus::Cancelled));
    }

    #[test]
    fn converts_through_a_temp_file_that_is_always_removed() {
        let device = fake_kindle();
        let lib = tempfile::tempdir().unwrap();
        let src = lib.path().join("Test Book.epub");
        minimal_epub(&src);

        let mut j = job(&src, device.path());
        j.needs_conversion = true;
        let h = Harness::new();
        let dest = done(h.run(&j));

        assert_eq!(dest, device.path().join("documents/Test Book.azw3"));
        assert!(fs::metadata(&dest).unwrap().len() > 0);
        assert!(h.temp_is_empty(), "conversion temp file removed");
        assert_eq!(h.statuses().first(), Some(&TransferStatus::Converting));
    }

    #[test]
    fn cancel_after_conversion_removes_the_temp_file() {
        let device = fake_kindle();
        let lib = tempfile::tempdir().unwrap();
        let src = lib.path().join("Test Book.epub");
        minimal_epub(&src);

        let mut j = job(&src, device.path());
        j.needs_conversion = true;
        let h = Harness::new().cancel_at(1);
        assert_eq!(h.run(&j), JobOutcome::Cancelled);
        assert!(h.temp_is_empty());
        assert!(files_under(device.path()).is_empty());
        assert_eq!(
            h.statuses(),
            vec![TransferStatus::Converting, TransferStatus::Cancelled]
        );
    }

    #[test]
    fn failed_conversion_removes_the_temp_file() {
        let device = fake_kindle();
        let lib = tempfile::tempdir().unwrap();
        let src = lib.path().join("Broken.epub");
        fs::write(&src, b"this is not a zip file").unwrap();

        let mut j = job(&src, device.path());
        j.needs_conversion = true;
        let h = Harness::new();
        assert!(matches!(h.run(&j), JobOutcome::Failed(_)));
        assert!(h.temp_is_empty());
        assert!(files_under(device.path()).is_empty());
    }

    #[test]
    fn unknown_conversion_target_fails() {
        let device = fake_kindle();
        let lib = tempfile::tempdir().unwrap();
        let src = lib.path().join("Book.epub");
        minimal_epub(&src);
        let mut j = job(&src, device.path());
        j.needs_conversion = true;
        j.target_format = "docx".into();
        assert!(matches!(Harness::new().run(&j), JobOutcome::Failed(_)));
    }

    #[test]
    fn missing_source_or_non_device_fails() {
        let device = fake_kindle();
        let lib = tempfile::tempdir().unwrap();
        let missing = lib.path().join("missing.azw3");
        assert!(matches!(
            Harness::new().run(&job(&missing, device.path())),
            JobOutcome::Failed(_)
        ));

        let src = lib.path().join("Dune.azw3");
        fs::write(&src, b"dune").unwrap();
        let not_a_device = tempfile::tempdir().unwrap();
        let outcome = Harness::new().run(&job(&src, not_a_device.path()));
        assert!(
            matches!(outcome, JobOutcome::Failed(ref m) if m.contains("doesn't look like")),
            "{outcome:?}"
        );
        assert!(files_under(not_a_device.path()).is_empty());
    }

    #[test]
    fn progress_payload_matches_the_ipc_contract() {
        let p = TransferProgress {
            job_id: "j".into(),
            status: TransferStatus::Cancelled,
            progress: 0,
            error_message: None,
        };
        assert_eq!(
            serde_json::to_string(&p).unwrap(),
            r#"{"jobId":"j","status":"cancelled","progress":0}"#
        );
        let p = TransferProgress {
            status: TransferStatus::Error,
            error_message: Some("x".into()),
            ..p
        };
        assert_eq!(
            serde_json::to_string(&p).unwrap(),
            r#"{"jobId":"j","status":"error","progress":0,"errorMessage":"x"}"#
        );
    }

    // ── Identity and the manifest ──────────────────────────────────────────

    #[test]
    fn a_library_book_is_sent_with_its_id_and_recorded() {
        let lib = tempfile::tempdir().unwrap();
        write_epub(&lib.path().join("Dune.epub"), "Dune", "Frank Herbert");
        let library_copy = fs::read(lib.path().join("Dune.epub")).unwrap();
        let device = fake_kobo();
        let (j, id) = library_job(lib.path(), "Dune.epub", device.path());

        let h = Harness::new();
        let dest = done(h.run(&j));

        assert_eq!(dest, device.path().join("documents/Dune.epub"));
        assert_eq!(
            epub_details(&dest).unwrap().2.as_deref(),
            Some(id.as_str()),
            "the copy carries the id"
        );
        assert_eq!(
            fs::read(lib.path().join("Dune.epub")).unwrap(),
            library_copy,
            "the library file is untouched"
        );
        assert!(h.temp_is_empty(), "the EPUB copy is removed");
        let entry = manifest::load(device.path())
            .manifest
            .entry(&id)
            .cloned()
            .unwrap();
        assert_eq!(
            (
                entry.path.as_str(),
                entry.title.as_str(),
                entry.author.as_str(),
                entry.format.as_str(),
                entry.size_bytes
            ),
            (
                "documents/Dune.epub",
                "Dune",
                "Frank Herbert",
                "epub",
                fs::metadata(&dest).unwrap().len()
            )
        );
        assert!(chrono::DateTime::parse_from_rfc3339(&entry.sent_at).is_ok());
    }

    #[test]
    fn a_kindle_conversion_carries_the_id_as_its_asin() {
        let lib = tempfile::tempdir().unwrap();
        minimal_epub(&lib.path().join("Book.epub"));
        let device = fake_kindle();
        let (mut j, id) = library_job(lib.path(), "Book.epub", device.path());
        j.needs_conversion = true;

        let dest = done(Harness::new().run(&j));
        assert_eq!(dest, device.path().join("documents/Book.azw3"));
        assert_eq!(
            kindle_details(&dest).and_then(|d| d.library_id).as_deref(),
            Some(id.as_str())
        );
        assert_eq!(
            manifest::load(device.path())
                .manifest
                .entry(&id)
                .unwrap()
                .format,
            "azw3"
        );
    }

    /// A PNG cover image of `w` × `h` pixels.
    fn png_cover(w: u32, h: u32) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image::RgbImage::new(w, h))
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    #[test]
    fn a_kindle_gets_the_books_home_screen_cover() {
        let lib = tempfile::tempdir().unwrap();
        minimal_epub(&lib.path().join("Book.epub"));
        let device = fake_kindle();
        let thumbs = super::super::thumbnail::thumbnails_dir(device.path());
        fs::create_dir_all(&thumbs).unwrap();
        let (mut j, id) = library_job(lib.path(), "Book.epub", device.path());
        j.needs_conversion = true;
        covers::persist_cover(
            lib.path(),
            &id,
            &png_cover(600, 900),
            &crate::logging::no_log,
        )
        .unwrap();
        // What the Kindle leaves when it fails to get a cover itself.
        let name = super::super::thumbnail::thumbnail_name(&id, "EBOK");
        let partial = thumbs.join(format!("{name}.partial"));
        fs::write(&partial, b"").unwrap();

        let dest = done(Harness::new().run(&j));

        let thumb = thumbs.join(&name);
        let img = image::open(&thumb).expect("thumbnail written as a JPEG");
        assert_eq!(img.height(), super::super::thumbnail::THUMBNAIL_HEIGHT);
        assert_eq!(img.width(), 313, "aspect ratio kept (600×900 → 313×470)");
        assert!(!partial.exists(), "the failed attempt is cleaned up");

        // Books sent earlier get theirs back through the repair.
        fs::remove_file(&thumb).unwrap();
        assert_eq!(
            super::super::thumbnail::repair(device.path(), lib.path(), &crate::logging::no_log),
            1
        );
        assert!(thumb.is_file());
        assert_eq!(
            super::super::thumbnail::repair(device.path(), lib.path(), &crate::logging::no_log),
            0
        );

        // Deleting the book from the device removes its cover too.
        super::super::listing::delete_book(device.path(), &dest, &crate::logging::no_log).unwrap();
        assert!(!thumb.exists());
    }

    #[test]
    fn a_kobo_gets_no_kindle_thumbnail_folder() {
        let lib = tempfile::tempdir().unwrap();
        minimal_epub(&lib.path().join("Book.epub"));
        let device = fake_kobo();
        let (j, id) = library_job(lib.path(), "Book.epub", device.path());
        covers::persist_cover(lib.path(), &id, &png_cover(60, 90), &crate::logging::no_log)
            .unwrap();
        done(Harness::new().run(&j));
        assert!(!device.path().join("system").exists());
    }

    fn set_modified(path: &Path, when: SystemTime) {
        let file = File::options().write(true).open(path).unwrap();
        file.set_modified(when).unwrap();
    }

    fn modified(path: &Path) -> SystemTime {
        fs::metadata(path).unwrap().modified().unwrap()
    }

    #[test]
    fn dates_added_become_unix_seconds() {
        assert_eq!(unix_seconds("2026-01-01T00:00:00Z"), Some(1_767_225_600));
        assert_eq!(
            unix_seconds(" 2026-01-01T01:00:00+01:00 "),
            Some(1_767_225_600)
        );
        assert_eq!(unix_seconds(""), None);
        assert_eq!(unix_seconds("yesterday"), None);
        assert_eq!(unix_seconds("1969-12-31T23:59:59Z"), None, "before 1970");
    }

    #[test]
    fn re_sending_an_unchanged_book_leaves_the_device_copy_alone() {
        let lib = tempfile::tempdir().unwrap();
        minimal_epub(&lib.path().join("Book.epub"));
        let kindle = fake_kindle();
        let (mut j, id) = library_job(lib.path(), "Book.epub", kindle.path());
        j.needs_conversion = true;
        let dest = done(Harness::new().run(&j));
        // Dated when the book was added to the library, not when converted.
        let added = unix_seconds(&scan(lib.path())["Book.epub"].date_added).unwrap();
        assert_eq!(fs::read(&dest).unwrap()[36..40], added.to_be_bytes());
        let long_ago = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        set_modified(&dest, long_ago);

        // The same AZW3 again: nothing is written (a Kindle would forget
        // the reading position), the transfer is still recorded (here in a
        // manifest that was lost meanwhile).
        fs::remove_file(manifest::manifest_path(kindle.path())).unwrap();
        let h = Harness::new();
        assert_eq!(done(h.run(&j)), dest);
        assert!(h.lines.contains("not copied again"));
        assert_eq!(modified(&dest), long_ago);
        assert!(manifest::load(kindle.path()).manifest.entry(&id).is_some());

        // The book changed in the library: the new version replaces it.
        write_epub(&lib.path().join("Book.epub"), "Test Book", "Someone Else");
        let h = Harness::new();
        assert_eq!(done(h.run(&j)), dest);
        assert!(!h.lines.contains("not copied again"));
        assert_ne!(modified(&dest), long_ago);
    }

    #[test]
    fn re_sending_an_unchanged_epub_leaves_the_device_copy_alone() {
        let lib = tempfile::tempdir().unwrap();
        minimal_epub(&lib.path().join("Book.epub"));
        let kobo = fake_kobo();
        let (j, _) = library_job(lib.path(), "Book.epub", kobo.path());
        let dest = done(Harness::new().run(&j));
        let long_ago = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        set_modified(&dest, long_ago);
        let h = Harness::new();
        assert_eq!(done(h.run(&j)), dest);
        assert_eq!(modified(&dest), long_ago);
    }

    #[test]
    fn the_index_decides_which_file_is_sent() {
        let lib = tempfile::tempdir().unwrap();
        write_epub(&lib.path().join("Dune.epub"), "Dune", "Frank Herbert");
        fs::write(lib.path().join("Other.pdf"), b"%PDF-1.4 other").unwrap();
        let device = fake_kobo();
        let (mut j, _) = library_job(lib.path(), "Dune.epub", device.path());
        // A stale or wrong path from the frontend.
        j.book_path = lib.path().join("Other.pdf").to_string_lossy().into_owned();

        let h = Harness::new();
        assert_eq!(done(h.run(&j)), device.path().join("documents/Dune.epub"));
        assert!(h.lines.contains("sending that file"));
    }

    #[test]
    fn identity_errors_fail_the_job() {
        let lib = tempfile::tempdir().unwrap();
        write_epub(&lib.path().join("Dune.epub"), "Dune", "Frank Herbert");
        let device = fake_kobo();
        let (j, _) = library_job(lib.path(), "Dune.epub", device.path());

        let mut half = j.clone();
        half.library_path = None;
        assert!(
            matches!(Harness::new().run(&half), JobOutcome::Failed(m) if m.contains("both bookId and libraryPath"))
        );

        let mut bad = j.clone();
        bad.book_id = Some("../../etc".into());
        assert!(
            matches!(Harness::new().run(&bad), JobOutcome::Failed(m) if m.contains("Invalid book id"))
        );

        let mut unknown = j;
        unknown.book_id = Some("aaaaaaaa-0000-4000-8000-000000000000".into());
        assert!(
            matches!(Harness::new().run(&unknown), JobOutcome::Failed(m) if m.contains("not in the library"))
        );
        assert!(
            files_under(device.path()).is_empty(),
            "nothing sent, nothing recorded"
        );
    }

    #[test]
    fn resending_replaces_the_previous_copy_wherever_it_was() {
        let lib = tempfile::tempdir().unwrap();
        minimal_epub(&lib.path().join("Book.epub"));
        let device = fake_kindle();
        let (mut j, id) = library_job(lib.path(), "Book.epub", device.path());
        j.needs_conversion = true;
        // An earlier copy of this book, under its old name (the book was
        // renamed in the library since), with the Kindle's reading state.
        let shelf = device.path().join("documents/Shelf");
        fs::create_dir_all(shelf.join("Old name.sdr")).unwrap();
        fs::write(shelf.join("Old name.azw3"), b"old copy").unwrap();
        fs::write(shelf.join("Unrelated.azw3"), b"someone else").unwrap();
        manifest::update(device.path(), &no_log, |m| {
            m.books.insert(
                id.clone(),
                ManifestEntry {
                    path: "documents/Shelf/Old name.azw3".into(),
                    title: "Book".into(),
                    author: String::new(),
                    format: "azw3".into(),
                    size_bytes: 8,
                    sent_at: String::new(),
                },
            );
        })
        .unwrap();
        // The subfolder setting changed meanwhile: the book stays where it is.
        j.subfolder = Some("Elsewhere".into());

        let h = Harness::new();
        let dest = done(h.run(&j));
        assert_eq!(dest, shelf.join("Book.azw3"));
        assert!(
            !shelf.join("Old name.azw3").exists(),
            "the superseded copy is removed"
        );
        assert!(h.lines.contains("removed the previous copy"));
        assert_eq!(
            fs::read(shelf.join("Unrelated.azw3")).unwrap(),
            b"someone else"
        );
        assert!(
            shelf.join("Old name.sdr").is_dir(),
            ".sdr folders are left alone"
        );
        assert_eq!(
            manifest::load(device.path())
                .manifest
                .entry(&id)
                .unwrap()
                .path,
            "documents/Shelf/Book.azw3"
        );
    }

    #[test]
    fn resending_to_the_same_name_keeps_the_file() {
        let lib = tempfile::tempdir().unwrap();
        write_epub(&lib.path().join("Dune.epub"), "Dune", "Frank Herbert");
        let device = fake_kobo();
        let (j, id) = library_job(lib.path(), "Dune.epub", device.path());
        let first = done(Harness::new().run(&j));
        // The device (FAT) reports another case for the same file.
        manifest::update(device.path(), &no_log, |m| {
            m.books.get_mut(&id).unwrap().path = "DOCUMENTS/dune.EPUB".into()
        })
        .unwrap();

        let second = done(Harness::new().run(&j));
        assert!(
            second.is_file() && first.is_file(),
            "the file just written is never deleted as 'superseded'"
        );
        assert_eq!(
            files_under(&device.path().join("documents")),
            vec!["Dune.epub"]
        );
    }

    #[test]
    fn another_library_books_file_is_never_overwritten() {
        let lib = tempfile::tempdir().unwrap();
        fs::create_dir_all(lib.path().join("a")).unwrap();
        fs::create_dir_all(lib.path().join("b")).unwrap();
        write_epub(&lib.path().join("a/Dune.epub"), "Dune", "Frank Herbert");
        write_epub(&lib.path().join("b/Dune.epub"), "Dune", "Brian Herbert");
        let books: Vec<_> =
            crate::commands::library::scan::scan_library_at(lib.path(), &no_log).unwrap();
        let device = fake_kobo();
        let job_for = |book: &crate::models::book::Book| {
            let mut j = job(Path::new(&book.path), device.path());
            j.book_id = Some(book.id.clone());
            j.library_path = Some(lib.path().to_string_lossy().into_owned());
            j
        };

        let first = done(Harness::new().run(&job_for(&books[0])));
        let second = done(Harness::new().run(&job_for(&books[1])));
        assert_eq!(first, device.path().join("documents/Dune.epub"));
        assert_eq!(second, device.path().join("documents/Dune_2.epub"));
        assert_eq!(
            epub_details(&first).unwrap().2.as_deref(),
            Some(books[0].id.as_str())
        );
        let m = manifest::load(device.path()).manifest;
        assert_eq!(m.entry(&books[1].id).unwrap().path, "documents/Dune_2.epub");
        assert_eq!(m.books.len(), 2);
    }

    #[test]
    fn superseded_copies_are_only_deleted_when_certain() {
        let device = fake_kindle();
        let docs = device.path().join("documents");
        let new_dest = docs.join("New.azw3");
        fs::write(&new_dest, b"new").unwrap();
        let entry = |path: &str| ManifestEntry {
            path: path.into(),
            title: String::new(),
            author: String::new(),
            format: String::new(),
            size_bytes: 0,
            sent_at: String::new(),
        };
        let old = docs.join("Old.azw3");
        fs::write(&old, b"old").unwrap();
        assert_eq!(
            superseded_file(
                device.path(),
                &entry("documents/Old.azw3"),
                "documents/New.azw3",
                &new_dest
            ),
            Some(old.clone())
        );
        // The same file under another case.
        assert_eq!(
            superseded_file(
                device.path(),
                &entry("documents/new.AZW3"),
                "documents/New.azw3",
                &new_dest
            ),
            None
        );
        // Not a book, a folder, missing, outside the device.
        fs::write(docs.join("notes.txt"), b"x").unwrap();
        assert_eq!(
            superseded_file(
                device.path(),
                &entry("documents/notes.txt"),
                "documents/New.azw3",
                &new_dest
            ),
            None
        );
        fs::create_dir(docs.join("Folder.epub")).unwrap();
        assert_eq!(
            superseded_file(
                device.path(),
                &entry("documents/Folder.epub"),
                "documents/New.azw3",
                &new_dest
            ),
            None
        );
        assert_eq!(
            superseded_file(
                device.path(),
                &entry("documents/Gone.azw3"),
                "documents/New.azw3",
                &new_dest
            ),
            None
        );
        assert_eq!(
            superseded_file(
                device.path(),
                &entry("../Old.azw3"),
                "documents/New.azw3",
                &new_dest
            ),
            None
        );
        #[cfg(unix)]
        {
            // A link to the new file, or a link to a book outside the device.
            std::os::unix::fs::symlink(&new_dest, docs.join("Link.azw3")).unwrap();
            assert_eq!(
                superseded_file(
                    device.path(),
                    &entry("documents/Link.azw3"),
                    "documents/New.azw3",
                    &new_dest
                ),
                None
            );
        }
    }

    #[test]
    fn a_damaged_manifest_doesnt_stop_a_transfer() {
        let lib = tempfile::tempdir().unwrap();
        write_epub(&lib.path().join("Dune.epub"), "Dune", "Frank Herbert");
        let device = fake_kobo();
        fs::create_dir_all(device.path().join(MANIFEST_DIR)).unwrap();
        fs::write(manifest::manifest_path(device.path()), b"{{{").unwrap();
        let (j, id) = library_job(lib.path(), "Dune.epub", device.path());

        done(Harness::new().run(&j));
        assert!(
            manifest::load(device.path()).manifest.entry(&id).is_some(),
            "a fresh manifest records the book"
        );
    }

    #[test]
    fn an_epub_that_cant_be_tagged_is_still_sent() {
        let lib = tempfile::tempdir().unwrap();
        let odd = lib.path().join("Odd.epub");
        write_epub(&odd, "Odd", "Someone");
        let device = fake_kobo();
        let (j, id) = library_job(lib.path(), "Odd.epub", device.path());
        // Now without META-INF/container.xml: readable (leniently), but
        // Librarium only ever edits an OPF it finds the strict way.
        let opf = crate::commands::library::test_support::opf_of(&odd);
        fs::write(
            &odd,
            crate::epub::fixtures::epub_bytes(&[(OPF_PATH, opf.as_bytes())]),
        )
        .unwrap();

        let h = Harness::new();
        let dest = done(h.run(&j));
        assert!(h.lines.contains("without its library id inside"));
        assert_eq!(
            fs::read(&dest).unwrap(),
            fs::read(&odd).unwrap(),
            "sent unchanged"
        );
        assert!(
            manifest::load(device.path()).manifest.entry(&id).is_some(),
            "the manifest still knows it"
        );
    }

    // ── Leftover .part files ───────────────────────────────────────────────

    #[test]
    fn stale_partial_files_are_removed_and_fresh_ones_kept() {
        let device = fake_kindle();
        let docs = device.path().join("documents");
        fs::create_dir_all(docs.join("Shelf")).unwrap();
        fs::create_dir_all(device.path().join(MANIFEST_DIR)).unwrap();
        let old = SystemTime::now() - Duration::from_secs(3600);
        let stale = [
            docs.join(".librarium-a1b2c3.part"),
            docs.join("Shelf/.librarium-d4e5f6.part"),
            docs.join("Shelf/._.librarium-d4e5f6.part"),
            device
                .path()
                .join(MANIFEST_DIR)
                .join(".librarium-g7h8.part"),
        ];
        for p in &stale {
            fs::write(p, b"partial").unwrap();
            File::options()
                .write(true)
                .open(p)
                .unwrap()
                .set_modified(old)
                .unwrap();
        }
        let fresh = docs.join(".librarium-live.part");
        fs::write(&fresh, b"being written").unwrap();
        let kept = [
            docs.join("Book.azw3"),
            docs.join("notes.part"),
            docs.join(".librarium-x.txt"),
        ];
        for p in &kept {
            fs::write(p, b"x").unwrap();
            File::options()
                .write(true)
                .open(p)
                .unwrap()
                .set_modified(old)
                .unwrap();
        }

        let mut removed = clean_stale_parts(device.path(), STALE_PART_AGE, &no_log);
        removed.sort();
        let mut expected = stale.to_vec();
        expected.sort();
        assert_eq!(removed, expected);
        assert!(fresh.exists(), "a .part being written is kept");
        assert!(kept.iter().all(|p| p.exists()));
    }
}
