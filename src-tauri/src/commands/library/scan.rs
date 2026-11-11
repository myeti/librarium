//! Finding the books in the library folder: which files count as books
//! ([`is_supported_extension`]), how the folder is walked
//! ([`walk_library`]), how a new file becomes a [`Book`] record
//! ([`build_book_from_path`], [`index_new_file`]), and the scan itself
//! ([`scan_library_at`]), which reconciles the index with what is on disk.
//!
//! Reading a new file is isolated: a parser that panics on one malformed
//! book, or a file that isn't what its extension says, only costs that file.
//! It is recorded in `.librarium/failed.json` and skipped by later scans
//! until it changes ([`super::failed`]).
//!
//! The scan holds the index lock only twice, briefly: to note what the
//! index knows, and to merge what the walk found. Walking the library and
//! reading new files — minutes, for a large library on a NAS — happen in
//! between, so covers, edits and the OPDS server keep working meanwhile.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, Read};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use uuid::Uuid;
use walkdir::WalkDir;

use super::failed::{FailedFiles, Fingerprint};
use super::files::{ensure_library_available, is_system_folder_name};
use super::index::{with_index, LibraryIndex};
use super::lock;
use super::metadata;
use super::relkey::{nfc, normalize_key, relative_to};
use super::sniff;
use crate::error::{panic_message, AppError, AppResult};
use crate::logging::Log;
use crate::models::book::{Book, BookFormat};

/// File extensions (lowercase, no dot) of the formats the library holds.
static SUPPORTED_EXTENSIONS: &[&str] = &["epub", "pdf", "mobi", "azw", "azw3", "cbz"];

/// Whether files with this extension (any case, no dot) can be in the library.
pub(crate) fn is_supported_extension(ext: &str) -> bool {
    SUPPORTED_EXTENSIONS.contains(&ext.to_lowercase().as_str())
}

/// Whether the file at `path` has a supported extension (its content is not
/// checked).
pub(crate) fn is_supported(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(is_supported_extension)
}

/// Walk the library folder, skipping hidden entries (`.librarium`, `.covers`,
/// macOS `._Dune.epub` resource forks…) and not descending into hidden
/// folders, nor into the folders a NAS or an OS keeps in shares (Synology's
/// `#recycle` and `@eaDir`, `$RECYCLE.BIN`…; [`is_system_folder_name`]):
/// deleted books in a NAS recycle bin are not books of the library. Only
/// names *below* the root are checked, so a library that itself lives under
/// a dotted folder (`~/.books`) is still walked (checking every component of
/// the absolute path would make it scan as empty).
pub(crate) fn walk_library(dir: &Path) -> impl Iterator<Item = walkdir::Result<walkdir::DirEntry>> {
    WalkDir::new(dir).into_iter().filter_entry(|e| {
        if e.depth() == 0 {
            return true;
        }
        let name = e.file_name().to_string_lossy();
        !(name.starts_with('.') || (e.file_type().is_dir() && is_system_folder_name(&name)))
    })
}

/// Why a file couldn't become a [`Book`].
#[derive(Debug)]
pub(crate) enum ReadFailure {
    /// Reading the file failed: it vanished, permissions, an I/O error on a
    /// share. Possibly temporary, so it is not recorded; the next scan
    /// retries.
    Io(io::Error),
    /// The file isn't a book of its format (its content doesn't match its
    /// extension). Recorded in `failed.json`.
    Invalid(String),
    /// A parser panicked on the file. Recorded in `failed.json`.
    Panicked(String),
}

impl ReadFailure {
    /// Whether the failure is a property of the file (recorded and skipped
    /// until the file changes) rather than of the moment (retried).
    pub(crate) fn is_permanent(&self) -> bool {
        !matches!(self, Self::Io(_))
    }

    /// The failure as a command error about the file called `name`.
    pub(crate) fn into_app_error(self, name: &str) -> AppError {
        match self {
            Self::Io(e) => AppError::io(format!("Could not read {name}"), &e),
            Self::Invalid(reason) => {
                AppError::InvalidInput(format!("{name} can't be added to the library: {reason}"))
            }
            Self::Panicked(reason) => {
                AppError::Internal(format!("reading {name} failed: {reason}"))
            }
        }
    }
}

impl std::fmt::Display for ReadFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "could not be read ({e})"),
            Self::Invalid(reason) | Self::Panicked(reason) => f.write_str(reason),
        }
    }
}

/// Check that a file whose metadata Librarium parses (EPUB, PDF) has the
/// content its extension claims ([`sniff::ebook_matches_extension`]): an
/// HTML error page saved as `.epub`, or an empty file, is not a book. Only
/// the first KB is read.
///
/// Other formats aren't checked: their metadata isn't parsed, and some
/// valid files have unusual headers (old PalmDOC `.mobi`, Topaz `.azw`,
/// RAR archives named `.cbz`) that readers still open.
fn check_content(path: &Path, format: &BookFormat) -> Result<(), ReadFailure> {
    let (ext, a_kind) = match format {
        BookFormat::Epub => ("epub", "an EPUB"),
        BookFormat::Pdf => ("pdf", "a PDF"),
        _ => return Ok(()),
    };
    let mut head = Vec::with_capacity(sniff::EBOOK_SNIFF_LEN);
    fs::File::open(path)
        .and_then(|file| {
            file.take(sniff::EBOOK_SNIFF_LEN as u64)
                .read_to_end(&mut head)
        })
        .map_err(ReadFailure::Io)?;
    if sniff::ebook_matches_extension(ext, &head) {
        Ok(())
    } else if head.is_empty() {
        Err(ReadFailure::Invalid("the file is empty".into()))
    } else {
        Err(ReadFailure::Invalid(format!(
            "it is not {a_kind} file (its content doesn't match its extension)"
        )))
    }
}

#[cfg(test)]
use crate::error::PANIC_TEST_MARKER;

/// [`metadata::read_metadata_as`] of the file at `read`, in the library as
/// `named`, with the parser's panics caught: a malformed book that crashes
/// the EPUB or PDF parser only fails itself.
fn read_metadata_isolated(
    read: &Path,
    named: &Path,
    format: &BookFormat,
) -> Result<crate::models::book::BookMetadata, ReadFailure> {
    catch_unwind(AssertUnwindSafe(|| {
        #[cfg(test)]
        if [read, named]
            .iter()
            .any(|p| p.to_string_lossy().contains(PANIC_TEST_MARKER))
        {
            panic!("forced panic (test hook)");
        }
        metadata::read_metadata_as(read, named, format)
    }))
    .map_err(|payload| {
        ReadFailure::Panicked(format!(
            "reading it crashed the parser ({})",
            panic_message(payload.as_ref())
        ))
    })
}

/// Build a `Book` struct by reading the file at `path` and extracting metadata
/// ([`metadata::read_metadata`]). Generates a fresh UUID — callers should
/// check the store before calling this (or use [`index_new_file`]).
///
/// Fails when the file can't be read ([`ReadFailure::Io`]), isn't a book of
/// its format ([`check_content`]), or makes the parser panic. Metadata that
/// can't be read from a file that *is* of its format (a damaged OPF, a PDF
/// lopdf can't parse) is not a failure: the book gets its file name as
/// title, as before.
///
/// An unknown title or author is left empty (the UI shows a placeholder).
/// For formats without embedded metadata (and PDFs without a title), the
/// file name stands in for the title, as before.
pub(crate) fn build_book_from_path(path: &Path) -> Result<Book, ReadFailure> {
    build_book_from_file(path, path)
}

/// [`build_book_from_path`] for a file read at `read` that is (or will be)
/// in the library as `named` — an import staged in a hidden temp file: the
/// content comes from `read`; the format, path, file name and the title
/// that stands in for a missing one come from `named`.
pub(crate) fn build_book_from_file(read: &Path, named: &Path) -> Result<Book, ReadFailure> {
    // Tests act here, while a scan or an import reads a file (no lock held).
    #[cfg(test)]
    super::test_support::hooks::run(super::test_support::hooks::READ, read);
    let ext = named.extension().and_then(|e| e.to_str()).unwrap_or("");
    let format = BookFormat::from_extension(ext);
    let size_bytes = fs::metadata(read).map_err(ReadFailure::Io)?.len();
    let filename = named
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();
    check_content(read, &format)?;
    let mut metadata = read_metadata_isolated(read, named, &format)?;
    // Readers that still produce the old English placeholders.
    metadata.clear_placeholders();
    metadata.title = metadata.title.trim().to_string();
    metadata.author = metadata.author.trim().to_string();
    if metadata.title.is_empty() && format != BookFormat::Epub {
        metadata.title = named
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
    }

    Ok(Book {
        id: Uuid::new_v4().to_string(),
        path: named.to_string_lossy().to_string(),
        filename,
        format,
        size_bytes,
        date_added: Utc::now().to_rfc3339(),
        cover_path: None,
        cover_version: None,
        favorite: false,
        metadata,
    })
}

/// How [`index_new_file`] got the book record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NewFile {
    /// A missing book with the same file name and size: re-linked, keeping
    /// its id, metadata and date added.
    Relinked,
    /// A book never seen before, read from the file.
    Parsed,
}

/// Add the file at `path` (inside the library, not yet indexed) to `index`
/// and return its record. If a *missing* book has the same file name and
/// size, the file is taken to be that book, moved: it gets the old record
/// back (same id, metadata, date added) instead of a new one.
pub(crate) fn index_new_file(
    index: &mut LibraryIndex,
    path: &Path,
) -> Result<(Book, NewFile), ReadFailure> {
    index_read_file(index, path, None)
}

/// [`index_new_file`] for a file that was already read, outside the index
/// lock ([`build_book_from_file`]): `read` is used unless a missing book
/// claims the file. Without it (`None`) the file is read here, under the
/// lock — only for files that turned up between the read and the merge.
/// The record gets `path` and its file name, whatever the read called it.
pub(crate) fn index_read_file(
    index: &mut LibraryIndex,
    path: &Path,
    read: Option<Result<Book, ReadFailure>>,
) -> Result<(Book, NewFile), ReadFailure> {
    let size = fs::metadata(path).map_err(ReadFailure::Io)?.len();
    let filename = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let key = index.key_of(path).unwrap_or_default();
    if let Some(missing) = index.take_missing_match(&filename, size, &key) {
        let book = Book {
            path: path.to_string_lossy().into_owned(),
            filename,
            ..missing.book
        };
        index.insert(book.clone());
        return Ok((book, NewFile::Relinked));
    }
    let book = match read {
        Some(read) => Book {
            path: path.to_string_lossy().into_owned(),
            filename,
            ..read?
        },
        None => build_book_from_path(path)?,
    };
    index.insert(book.clone());
    Ok((book, NewFile::Parsed))
}

/// What one scan changed, for the log (and the tests).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct ScanSummary {
    pub(crate) listed: usize,
    pub(crate) new: usize,
    pub(crate) relinked: usize,
    pub(crate) now_missing: usize,
    pub(crate) expired: usize,
    pub(crate) ignored: usize,
    pub(crate) walk_errors: usize,
    /// New files that couldn't be read this time (recorded in `failed.json`
    /// when the failure is permanent).
    pub(crate) failed: usize,
    /// Files skipped without being opened: they already failed in the same
    /// version (`failed.json`).
    pub(crate) known_failures: usize,
}

/// [`scan_library`](super::scan_library): reconcile the index with the files.
///
/// - Indexed files that are still there are listed as they are (their
///   metadata isn't re-read).
/// - New files are indexed, or re-linked to a missing book with the same
///   file name and size (a book moved to another folder keeps its id and
///   metadata).
/// - Files the user removed from the library (`ignored`) are skipped.
/// - New files that can't be read as books (a parser panics, or the content
///   doesn't match the extension) are left out and recorded in
///   `.librarium/failed.json`; later scans skip them without opening them
///   until their size or modification time changes ([`super::failed`]).
/// - Indexed files that are gone become *missing* (hidden, kept for
///   [`MISSING_RETENTION_DAYS`](super::index::MISSING_RETENTION_DAYS) days in
///   case they come back) — but only after a walk without a single error: an
///   unreadable folder or a NAS hiccup must not be mistaken for deleted books.
///
/// The library folder must exist (`LIBRARY_UNAVAILABLE` otherwise; a folder
/// that can't be listed is `PERMISSION_DENIED` or `LIBRARY_UNAVAILABLE`, not
/// an empty library): a scan never creates it. Opening the library also
/// marks it as open by this instance (`.librarium/lock.json`).
pub(crate) fn scan_library_at(dir: &Path, log: Log) -> AppResult<Vec<Book>> {
    scan_at(dir, Utc::now(), log).map(|(books, _)| books)
}

/// What the index knew when a scan started ([`scan_at`], step 1).
struct Known {
    /// Key → file name (NFC) and size of each indexed book. A book not
    /// found at its key may turn up elsewhere (moved): a new file with the
    /// same name and size will be re-linked to it, so it isn't read.
    books: HashMap<String, (String, u64)>,
    ignored: HashSet<String>,
    /// File name (NFC) and size of each missing book.
    missing: HashSet<(String, u64)>,
}

impl Known {
    fn of(index: &LibraryIndex) -> Self {
        Self {
            books: index
                .keys()
                .filter_map(|key| {
                    let book = index.get_key(key)?;
                    Some((key.clone(), (nfc(&book.filename), book.size_bytes)))
                })
                .collect(),
            ignored: index.ignored_keys().cloned().collect(),
            missing: index.missing_signatures(),
        }
    }

    /// Whether the scan treats `key` as a file the index doesn't have yet.
    fn is_new(&self, key: &str) -> bool {
        !self.books.contains_key(key) && !self.ignored.contains(key)
    }
}

/// A file of a supported format found by the walk.
struct Found {
    key: String,
    path: PathBuf,
    /// Size and modification time, read for new files only (`None` for
    /// indexed and ignored files, or when the file couldn't be looked at).
    fingerprint: Option<Fingerprint>,
}

/// What [`walk_files`] found.
struct Walk {
    files: Vec<Found>,
    /// Folders (or entries) that couldn't be read: the walk is incomplete.
    errors: usize,
}

/// Step 2 of [`scan_at`], without the index lock: list the library's files
/// of a supported format. Only new files (per `known`) are looked at
/// (`stat`), for their fingerprint: on a NAS each one is a round trip.
///
/// A library folder that can't be listed at all fails the scan
/// ([`AppError::folder`]: `PERMISSION_DENIED` on macOS when Librarium may
/// not read network volumes yet, `LIBRARY_UNAVAILABLE` when the share went
/// away) instead of looking like an empty library.
fn walk_files(dir: &Path, known: &Known, log: Log) -> AppResult<Walk> {
    let mut walk = Walk {
        files: Vec::new(),
        errors: 0,
    };
    for entry in walk_library(dir) {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) if e.depth() == 0 => {
                return Err(match e.io_error() {
                    Some(io) => AppError::folder(dir, io),
                    None => AppError::Io(format!("Could not read the library folder: {e}")),
                });
            }
            Err(e) => {
                walk.errors += 1;
                log(format!(
                    "[scan_library] WARNING — could not read part of the library: {}",
                    e
                ));
                continue;
            }
        };
        if !entry.file_type().is_file() || !is_supported(entry.path()) {
            continue;
        }
        let Some(rel) = relative_to(dir, entry.path()) else {
            continue;
        };
        let key = normalize_key(&rel);
        let fingerprint = if known.is_new(&key) {
            entry.metadata().ok().map(|meta| Fingerprint::of(&meta))
        } else {
            None
        };
        walk.files.push(Found {
            key,
            path: entry.path().to_path_buf(),
            fingerprint,
        });
    }
    Ok(walk)
}

/// What step 3 of [`scan_at`] made of a new file, without the index lock.
enum Prepared {
    /// It already failed in this very version, unchanged: not opened again.
    KnownFailure,
    /// A missing (or moved) book has its name and size: it will be re-linked,
    /// so there is nothing to read.
    Relink,
    /// Read: the record, or why it couldn't be.
    Read(Box<Result<Book, ReadFailure>>),
}

/// Whether the file of the book stored under `key`, which the walk didn't
/// find, is really gone: not on disk (`symlink_metadata` says "not found"),
/// or inside a folder scans skip (hidden, or a NAS/OS folder such as
/// `#recycle`). Any other answer (the file is there, or can't be looked at)
/// keeps the book: the walk may have missed a file being replaced at that
/// moment (an EPUB rewrite renames a new file over the old one).
fn is_gone(key: &str, path: &Path) -> bool {
    let mut folders = key.split('/').rev().skip(1);
    if folders.any(|name| name.starts_with('.') || is_system_folder_name(name)) {
        return true;
    }
    matches!(fs::symlink_metadata(path), Err(e) if e.kind() == io::ErrorKind::NotFound)
}

/// [`scan_library_at`], with the clock as a parameter.
///
/// The index lock is only held for steps 1 and 4, which are quick; the
/// slow part — walking the folder and reading new files, minutes for a
/// large library on a NAS — happens in between, unlocked, so covers, edits
/// and the OPDS server don't wait for it. Step 4 works on the index as it is
/// *then*: changes made meanwhile (an edit, an import, a rename, a delete)
/// are kept, never overwritten with what step 1 saw.
pub(crate) fn scan_at(
    dir: &Path,
    now: DateTime<Utc>,
    log: Log,
) -> AppResult<(Vec<Book>, ScanSummary)> {
    log(format!("[scan_library] start — path={}", dir.display()));
    ensure_library_available(dir)?;
    if let Err(e) = lock::claim(dir) {
        log(format!(
            "[scan_library] could not write {}: {e}",
            lock::LOCK_FILE
        ));
    }

    // 1. What the index knows (under the lock, briefly).
    let known = with_index(dir, log, |index| {
        log(format!(
            "[scan_library] index loaded — {} known entries",
            index.len()
        ));
        Ok(Known::of(index))
    })?;

    // 2. Walk the library (no lock).
    let walk = walk_files(dir, &known, log)?;

    // 3. Read the new files (no lock). A file whose name and size match a
    //    missing book, or an indexed one the walk didn't find where it was
    //    (moved), will be re-linked: not read.
    let found_keys: HashSet<&str> = walk.files.iter().map(|f| f.key.as_str()).collect();
    let mut relinkable = known.missing.clone();
    relinkable.extend(
        known
            .books
            .iter()
            .filter(|(key, _)| !found_keys.contains(key.as_str()))
            .map(|(_, signature)| signature.clone()),
    );
    let failed_before = FailedFiles::load(dir, log);
    let mut prepared: HashMap<String, Prepared> = HashMap::new();
    for file in walk.files.iter().filter(|f| known.is_new(&f.key)) {
        let name = file
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let step = match file.fingerprint {
            Some(fp) if failed_before.is_known_failure(&file.key, fp) => Prepared::KnownFailure,
            Some(fp) if relinkable.contains(&(nfc(&name), fp.size)) => Prepared::Relink,
            _ => Prepared::Read(Box::new(build_book_from_path(&file.path))),
        };
        prepared.insert(file.key.clone(), step);
    }

    // 4. Merge into the index as it is now (under the lock, briefly).
    with_index(dir, log, |index| {
        let mut summary = ScanSummary {
            walk_errors: walk.errors,
            ..ScanSummary::default()
        };
        let mut failed = FailedFiles::load(dir, log);
        // Keys of the indexed books found on disk, of the ignored files seen,
        // and of the files on disk that aren't books of the library (yet).
        let mut seen: HashSet<String> = HashSet::new();
        let mut seen_ignored: HashSet<String> = HashSet::new();
        let mut seen_unindexed: HashSet<String> = HashSet::new();
        let mut new_files: Vec<&Found> = Vec::new();

        for file in &walk.files {
            if index.is_ignored(&file.key) {
                seen_ignored.insert(file.key.clone());
                continue;
            }
            match index.get_key(&file.key) {
                Some(book) => {
                    // The same file, but the disk spells its name in another
                    // Unicode normalisation: keep the spelling that opens it.
                    if Path::new(&book.path) != file.path {
                        index.set_path(&file.key, &file.path);
                    }
                    seen.insert(file.key.clone());
                }
                None => {
                    seen_unindexed.insert(file.key.clone());
                    new_files.push(file);
                }
            }
        }
        let complete = walk.errors == 0;

        // Files that are gone become missing — after a complete walk only,
        // and only books the index had before the walk started (one added
        // meanwhile, by an import or a rename, simply wasn't there to see).
        if complete {
            let gone: Vec<String> = index
                .keys()
                .filter(|key| !seen.contains(*key) && known.books.contains_key(*key))
                .filter(|key| {
                    index
                        .get_key(key)
                        .is_some_and(|book| is_gone(key, Path::new(&book.path)))
                })
                .cloned()
                .collect();
            summary.now_missing = index.mark_missing(&gone, now);
            // Ignored files no longer on disk are forgotten (not those
            // ignored meanwhile, which the walk may not have seen as such).
            let stale_ignored = index
                .retain_ignored(|key| seen_ignored.contains(key) || !known.ignored.contains(key));
            if stale_ignored > 0 {
                log(format!(
                    "[scan_library] {} ignored file(s) no longer on disk",
                    stale_ignored
                ));
            }
        } else {
            log(format!(
                "[scan_library] WARNING — {} folder(s) could not be read; keeping every known book \
                 (nothing is marked missing after an incomplete scan)",
                summary.walk_errors
            ));
        }
        summary.ignored = seen_ignored.len();

        // New files: re-linked to a missing book, or added as read in step 3.
        for file in new_files {
            let outcome = match prepared.remove(&file.key) {
                Some(Prepared::KnownFailure) => {
                    summary.known_failures += 1;
                    continue;
                }
                Some(Prepared::Read(read)) => index_read_file(index, &file.path, Some(*read)),
                // Re-linked — or, if the missing book was claimed meanwhile
                // (or the file is new since step 1), read now.
                Some(Prepared::Relink) | None => index_new_file(index, &file.path),
            };
            match outcome {
                Ok((book, how)) => {
                    if how == NewFile::Relinked {
                        log(format!(
                            "[scan_library] re-linked — {} (id {})",
                            file.path.display(),
                            book.id
                        ));
                        summary.relinked += 1;
                    } else {
                        log(format!(
                            "[scan_library] new file — {}: \"{}\" by {}",
                            file.path.display(),
                            book.metadata.title,
                            book.metadata.author
                        ));
                        summary.new += 1;
                    }
                    failed.clear(&file.key);
                    seen_unindexed.remove(&file.key);
                    seen.insert(file.key.clone());
                }
                Err(failure) => {
                    summary.failed += 1;
                    record_failure(
                        &mut failed,
                        &file.key,
                        &file.path,
                        &failure,
                        "scan_library",
                        log,
                    );
                }
            }
        }

        // Forget books missing for too long, and failures of files that are
        // gone (or became books).
        summary.expired = index.expire_missing(now);
        if summary.expired > 0 {
            log(format!(
                "[scan_library] {} book(s) missing for a long time forgotten",
                summary.expired
            ));
        }
        if complete {
            failed.retain(|key| seen_unindexed.contains(key));
        }
        failed.save_or_log(log);

        // After a complete walk, every book of the index (the ones found,
        // and any added or kept meanwhile). After an incomplete one, books
        // not seen stay indexed but aren't listed — except those added since
        // step 1, which the walk had no chance to see.
        let books: Vec<Book> = index
            .keys()
            .filter(|key| complete || seen.contains(*key) || !known.books.contains_key(*key))
            .filter_map(|key| index.get_key(key).cloned())
            .collect();
        summary.listed = books.len();

        // Anything above (new, re-linked, missing, expired, a path spelling,
        // stale ignored entries) marks the index as changed.
        if index.is_dirty() {
            index.save()?;
        }

        log(format!(
            "[scan_library] done — {} books ({} new, {} re-linked, {} now missing, {} ignored)",
            summary.listed, summary.new, summary.relinked, summary.now_missing, summary.ignored
        ));
        if summary.known_failures > 0 {
            log(format!(
                "[scan_library] {} file(s) skipped: they could not be read before and haven't changed \
                 (listed in .librarium/{})",
                summary.known_failures,
                super::failed::FAILED_FILE
            ));
        }
        Ok((books, summary))
    })
}

/// Log why the file at `path` (index key `key`) couldn't be read and, when
/// the failure is a property of the file, record it in `failed` so later
/// scans skip it until it changes. `label` (the command) prefixes the log
/// line.
pub(crate) fn record_failure(
    failed: &mut FailedFiles,
    key: &str,
    path: &Path,
    failure: &ReadFailure,
    label: &str,
    log: Log,
) {
    if !failure.is_permanent() {
        log(format!(
            "[{label}] WARNING — {} {failure}; it will be tried again next time",
            path.display()
        ));
        return;
    }
    log(format!(
        "[{label}] WARNING — {} was left out of the library: {failure}. It is skipped until the file changes.",
        path.display()
    ));
    match Fingerprint::read(path) {
        Ok(fp) => failed.record(key, fp, &failure.to_string()),
        // Gone already: nothing to skip next time.
        Err(_) => failed.clear(key),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::library::index;
    use crate::commands::library::test_support::{index_len, indexed, scan, write, write_epub};
    use crate::logging::{no_log, LogLines};

    #[test]
    fn a_library_under_a_dotted_folder_is_scanned() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path().join(".books").join("Alice");
        fs::create_dir_all(&lib).unwrap();
        write_epub(&lib.join("Dune.epub"), "Dune", "Frank Herbert");
        // Hidden entries below the root are still skipped.
        fs::create_dir_all(lib.join(".covers")).unwrap();
        write_epub(&lib.join(".covers").join("hidden.epub"), "Hidden", "Nobody");
        write(&lib.join("._Dune.epub"), b"AppleDouble resource fork");

        let books = scan(&lib);
        assert_eq!(books.keys().collect::<Vec<_>>(), ["Dune.epub"]);
        assert_eq!(books["Dune.epub"].metadata.title, "Dune");
        assert_eq!(index_len(&lib), 1);
    }

    #[test]
    fn deleted_files_become_missing_after_a_complete_scan() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        write_epub(&lib.join("a.epub"), "A", "X");
        write_epub(&lib.join("b.epub"), "B", "X");
        let books = scan(lib);
        assert_eq!(books.len(), 2);
        fs::remove_file(lib.join("b.epub")).unwrap();

        let (listed, summary) = scan_at(lib, Utc::now(), &no_log).unwrap();
        assert_eq!(listed.len(), 1, "missing books aren't listed");
        assert_eq!(summary.now_missing, 1);
        assert_eq!(index_len(lib), 1);
        assert!(
            indexed(lib, &books["b.epub"].id).is_none(),
            "not addressable while missing"
        );
        let missing = with_index(lib, &no_log, |i| Ok(i.missing().clone())).unwrap();
        assert_eq!(missing["b.epub"].book.id, books["b.epub"].id);
        assert_eq!(missing["b.epub"].book.path, "b.epub");
    }

    #[test]
    fn a_moved_file_is_re_linked_with_its_id_and_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        write_epub(&lib.join("Dune.epub"), "Dune", "Frank Herbert");
        let original = scan(lib).remove("Dune.epub").unwrap();
        with_index(lib, &no_log, |index| {
            let mut rated = index.by_id(&original.id).unwrap().clone();
            rated.metadata.rating = Some(5);
            index.insert(rated);
            index.save()
        })
        .unwrap();

        // Moved into a sub-folder (by Finder, another computer…): one scan
        // sees the old place gone and the new file at once.
        fs::create_dir_all(lib.join("Herbert")).unwrap();
        fs::rename(lib.join("Dune.epub"), lib.join("Herbert").join("Dune.epub")).unwrap();
        let (books, summary) = scan_at(lib, Utc::now(), &no_log).unwrap();
        assert_eq!((summary.relinked, summary.new), (1, 0));
        assert_eq!(books.len(), 1);
        let moved = &books[0];
        assert_eq!(moved.id, original.id);
        assert_eq!(moved.date_added, original.date_added);
        assert_eq!(
            moved.metadata.rating,
            Some(5),
            "index-only metadata survives the move"
        );
        assert_eq!(
            PathBuf::from(&moved.path),
            lib.join("Herbert").join("Dune.epub")
        );
    }

    #[test]
    fn a_file_that_comes_back_later_is_re_linked() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        write_epub(&lib.join("a.epub"), "A", "X");
        let id = scan(lib)["a.epub"].id.clone();
        let park = tempfile::tempdir().unwrap();
        let parked = park.path().join("a.epub");
        fs::rename(lib.join("a.epub"), &parked).unwrap();
        assert!(scan(lib).is_empty());
        fs::rename(&parked, lib.join("a.epub")).unwrap();
        assert_eq!(scan(lib)["a.epub"].id, id);
    }

    #[test]
    fn missing_books_are_forgotten_after_sixty_days() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        write_epub(&lib.join("a.epub"), "A", "X");
        scan(lib);
        let park = tempfile::tempdir().unwrap();
        let parked = park.path().join("a.epub");
        fs::rename(lib.join("a.epub"), &parked).unwrap();
        let now = Utc::now();
        scan_at(lib, now, &no_log).unwrap();
        let (_, summary) = scan_at(lib, now + chrono::Duration::days(59), &no_log).unwrap();
        assert_eq!(summary.expired, 0);
        let (_, summary) = scan_at(lib, now + chrono::Duration::days(61), &no_log).unwrap();
        assert_eq!(summary.expired, 1);
        // Back after that: a new book.
        fs::rename(&parked, lib.join("a.epub")).unwrap();
        let (_, summary) = scan_at(lib, now + chrono::Duration::days(62), &no_log).unwrap();
        assert_eq!((summary.new, summary.relinked), (1, 0));
    }

    #[test]
    fn a_different_file_with_the_same_name_is_a_new_book() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        write_epub(&lib.join("a.epub"), "A", "X");
        let id = scan(lib)["a.epub"].id.clone();
        fs::remove_file(lib.join("a.epub")).unwrap();
        assert!(scan(lib).is_empty(), "missing now");
        write_epub(&lib.join("a.epub"), "A completely different book", "Y");
        let book = scan(lib).remove("a.epub").unwrap();
        assert_ne!(book.id, id, "different size: not the same file");
        assert_eq!(book.metadata.title, "A completely different book");
    }

    #[cfg(unix)]
    #[test]
    fn an_incomplete_scan_marks_nothing_missing() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        write_epub(&lib.join("a.epub"), "A", "X");
        fs::create_dir(lib.join("Series")).unwrap();
        write_epub(&lib.join("Series").join("b.epub"), "B", "X");
        let ids: Vec<String> = scan(lib).values().map(|b| b.id.clone()).collect();
        assert_eq!(ids.len(), 2);

        // An unreadable folder (permissions, or a NAS hiccup) is a walk error.
        let series = lib.join("Series");
        fs::set_permissions(&series, fs::Permissions::from_mode(0o000)).unwrap();
        let books = scan_library_at(lib, &no_log);
        fs::set_permissions(&series, fs::Permissions::from_mode(0o755)).unwrap();

        assert_eq!(books.unwrap().len(), 1, "only the readable book is listed");
        for id in &ids {
            assert!(
                indexed(lib, id).is_some(),
                "book {id} must stay in the library"
            );
        }
        // Once readable again, the book is still the same one (same id).
        assert_eq!(scan(lib).len(), 2);
    }

    #[test]
    fn a_corrupt_index_fails_the_scan_without_touching_it() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        write_epub(&lib.join("a.epub"), "A", "X");
        write(&index::index_path(lib), b"{ not json");
        assert!(scan_library_at(lib, &no_log).is_err());
        assert_eq!(fs::read(index::index_path(lib)).unwrap(), b"{ not json");
    }

    #[test]
    fn an_unmounted_library_is_unavailable_and_not_created() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path().join("Volumes").join("books").join("Alice");
        let err = scan_library_at(&lib, &no_log).unwrap_err();
        assert_eq!(err.code(), "LIBRARY_UNAVAILABLE");
        assert!(!dir.path().join("Volumes").exists(), "nothing was created");
    }

    // ── Files that can't be read ──────────────────────────────────────────────

    use crate::commands::library::failed::FAILED_FILE;

    fn failed_json(lib: &Path) -> serde_json::Value {
        let path = lib.join(index::INDEX_DIR).join(FAILED_FILE);
        fs::read(path)
            .map(|b| serde_json::from_slice(&b).unwrap())
            .unwrap_or(serde_json::Value::Null)
    }

    #[test]
    fn a_file_that_is_not_its_format_is_recorded_and_skipped_until_it_changes() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        write_epub(&lib.join("good.epub"), "Good", "X");
        // A login page saved under a book's name.
        write(
            &lib.join("Sub").join("bad.epub"),
            b"<!DOCTYPE html><html>Sign in</html>",
        );

        let (books, summary) = scan_at(lib, Utc::now(), &no_log).unwrap();
        assert_eq!(
            books
                .iter()
                .map(|b| b.filename.as_str())
                .collect::<Vec<_>>(),
            ["good.epub"]
        );
        assert_eq!(
            (summary.new, summary.failed, summary.known_failures),
            (1, 1, 0)
        );
        let entry = &failed_json(lib)["Sub/bad.epub"];
        assert_eq!(entry["size"], 35);
        assert!(entry["mtime"].as_i64().unwrap() > 0);
        assert!(
            entry["error"]
                .as_str()
                .unwrap()
                .contains("it is not an EPUB file"),
            "{entry}"
        );

        // The next scan doesn't even open it.
        let lines = LogLines::default();
        let (_, summary) = scan_at(lib, Utc::now(), &lines.sink()).unwrap();
        assert_eq!((summary.failed, summary.known_failures), (0, 1));
        assert!(lines.contains("1 file(s) skipped"));

        // Replaced by a real book: tried again, indexed, and forgotten by the list.
        write_epub(&lib.join("Sub").join("bad.epub"), "Fixed", "Y");
        let (books, summary) = scan_at(lib, Utc::now(), &no_log).unwrap();
        assert_eq!((summary.new, summary.known_failures), (1, 0));
        assert_eq!(books.len(), 2);
        assert!(
            failed_json(lib).is_null(),
            "no failed.json in a healthy library"
        );
    }

    #[test]
    fn a_parser_panic_only_costs_its_file() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        write_epub(&lib.join("Dune.epub"), "Dune", "Frank Herbert");
        let crashing = format!("{PANIC_TEST_MARKER}.epub");
        write_epub(&lib.join(&crashing), "Crash", "X");

        let lines = LogLines::default();
        let (books, summary) = scan_at(lib, Utc::now(), &lines.sink()).unwrap();
        assert_eq!(books.len(), 1, "the scan survives and lists the other book");
        assert_eq!(books[0].filename, "Dune.epub");
        assert_eq!(summary.failed, 1);
        assert!(lines.contains("left out of the library"));
        let error = failed_json(lib)[&crashing]["error"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(
            error.contains("crashed the parser") && error.contains("forced panic"),
            "{error}"
        );

        // No crash loop: the next scans skip it without parsing it.
        let (_, summary) = scan_at(lib, Utc::now(), &no_log).unwrap();
        assert_eq!((summary.failed, summary.known_failures), (0, 1));
    }

    #[test]
    fn failures_of_deleted_files_are_forgotten() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        write(&lib.join("empty.pdf"), b"");
        scan(lib);
        assert_eq!(failed_json(lib)["empty.pdf"]["error"], "the file is empty");
        fs::remove_file(lib.join("empty.pdf")).unwrap();
        scan(lib);
        assert!(failed_json(lib).is_null());
    }

    #[test]
    fn content_checks_only_apply_to_parsed_formats() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        // Old PalmDOC / Topaz files and RAR "cbz"s are still books.
        write(&lib.join("Old.mobi"), b"not a BOOKMOBI header");
        write(&lib.join("Comic.cbz"), b"Rar!\x1a\x07\x00");
        let books = scan(lib);
        assert_eq!(books.len(), 2);
        assert_eq!(books["Old.mobi"].metadata.title, "Old");
    }

    #[test]
    fn ignored_files_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        write_epub(&lib.join("keep.epub"), "Keep", "X");
        write_epub(&lib.join("forget.epub"), "Forget", "X");
        let books = scan(lib);
        with_index(lib, &no_log, |index| {
            index.forget(&books["forget.epub"].id).unwrap();
            index.save()
        })
        .unwrap();

        let (listed, summary) = scan_at(lib, Utc::now(), &no_log).unwrap();
        assert_eq!(
            listed
                .iter()
                .map(|b| b.filename.as_str())
                .collect::<Vec<_>>(),
            ["keep.epub"]
        );
        assert_eq!(summary.ignored, 1);
        assert!(
            lib.join("forget.epub").exists(),
            "the file is never touched"
        );

        // Once the file is gone, a complete scan stops tracking it.
        fs::remove_file(lib.join("forget.epub")).unwrap();
        scan(lib);
        assert!(with_index(lib, &no_log, |i| Ok(i.ignored().is_empty())).unwrap());
    }

    #[test]
    fn nfd_names_on_disk_match_nfc_keys() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        let nfd = "Caf\u{65}\u{301}.epub";
        write_epub(&lib.join(nfd), "Café", "X");
        let first = scan(lib);
        assert_eq!(first.len(), 1);
        let again = scan(lib);
        assert_eq!(again.len(), 1, "the same book, not a new one on each scan");
        assert_eq!(
            first.values().next().unwrap().id,
            again.values().next().unwrap().id
        );
    }

    #[test]
    fn unknown_titles_are_empty_and_pdfs_fall_back_to_the_file_name() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        write_epub(&lib.join("untitled.epub"), "", "");
        write(
            &lib.join("Notes de cours.pdf"),
            b"%PDF-1.4 not really a pdf",
        );
        let books = scan(lib);
        let epub = &books["untitled.epub"].metadata;
        assert_eq!(
            (epub.title.as_str(), epub.author.as_str()),
            ("", ""),
            "no placeholder stored"
        );
        assert_eq!(books["Notes de cours.pdf"].metadata.title, "Notes de cours");
        assert_eq!(books["Notes de cours.pdf"].metadata.author, "");
    }

    // ── NAS folders, permissions ──────────────────────────────────────────────

    #[test]
    fn nas_recycle_bins_and_thumbnail_folders_are_not_part_of_the_library() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        for folder in ["#1 Picks", "#recycle", "Herbert/@eaDir"] {
            fs::create_dir_all(lib.join(folder)).unwrap();
        }
        write_epub(&lib.join("Dune.epub"), "Dune", "Frank Herbert");
        write_epub(&lib.join("#1 Picks").join("Good.epub"), "Good", "X");
        write_epub(&lib.join("#recycle").join("Deleted.epub"), "Deleted", "X");
        write_epub(
            &lib.join("Herbert").join("@eaDir").join("Thumb.epub"),
            "Thumb",
            "X",
        );
        let books = scan(lib);
        let mut names: Vec<_> = books.keys().map(String::as_str).collect();
        names.sort();
        assert_eq!(names, ["Dune.epub", "Good.epub"]);

        // A book indexed inside the recycle bin (by hand, or by another tool)
        // leaves the library, even though its file is still there.
        let old = build_book_from_path(&lib.join("#recycle").join("Deleted.epub")).unwrap();
        with_index(lib, &no_log, |index| {
            index.insert(old.clone());
            index.save()
        })
        .unwrap();
        let (_, summary) = scan_at(lib, Utc::now(), &no_log).unwrap();
        assert_eq!(summary.now_missing, 1);
        assert!(indexed(lib, &old.id).is_none());
    }

    /// A library folder that is there but can't be listed — on macOS, until
    /// Librarium may read network volumes — fails the scan with
    /// `PERMISSION_DENIED` instead of looking like an empty library.
    #[cfg(unix)]
    #[test]
    fn an_unlistable_library_is_a_permission_problem_not_an_empty_library() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path().join("Alice");
        write_epub_creating_dirs(&lib.join("Dune.epub"));
        let id = scan(&lib)["Dune.epub"].id.clone();
        // Files inside can still be opened (x), the folder can't be listed (no r).
        fs::set_permissions(&lib, fs::Permissions::from_mode(0o300)).unwrap();
        let result = scan_library_at(&lib, &no_log);
        fs::set_permissions(&lib, fs::Permissions::from_mode(0o755)).unwrap();
        let err = result.unwrap_err();
        assert_eq!(err.code(), "PERMISSION_DENIED", "{err}");
        assert_eq!(scan(&lib)["Dune.epub"].id, id, "nothing was marked missing");
    }

    // ── The index lock during a scan ──────────────────────────────────────────

    use crate::commands::library::covers;
    use crate::commands::library::edit::update_book_metadata;
    use crate::commands::library::test_support::{hooks, within};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    /// Reading new files — the slow part of a scan on a NAS — happens
    /// without the index lock: covers and edits go through meanwhile, and the
    /// scan's merge keeps them.
    #[test]
    fn a_scan_does_not_hold_the_index_while_it_reads_new_files() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path().to_path_buf();
        write_epub(&lib.join("Dune.epub"), "Dune", "Frank Herbert");
        let dune = scan(&lib).remove("Dune.epub").unwrap();
        write_epub(&lib.join("New.epub"), "New", "X");

        let reads = Arc::new(AtomicUsize::new(0));
        let _hook = hooks::on(hooks::READ, &lib, {
            let (lib, dune, reads) = (lib.clone(), dune.clone(), reads.clone());
            move |_| {
                assert!(
                    !index::is_index_locked(&lib),
                    "the index is locked while a new file is read"
                );
                let (lib, dune) = (lib.clone(), dune.clone());
                within(20, move || {
                    let mut rated = dune.metadata.clone();
                    rated.rating = Some(5);
                    update_book_metadata(&lib, &dune.id, rated, &no_log).unwrap();
                    covers::ensure_cover(&lib, &dune.id, &no_log).unwrap();
                });
                reads.fetch_add(1, Ordering::SeqCst);
            }
        });
        let (books, summary) = scan_at(&lib, Utc::now(), &no_log).unwrap();
        assert_eq!(reads.load(Ordering::SeqCst), 1, "only the new file is read");
        assert_eq!((summary.new, books.len()), (1, 2));
        let listed = books.iter().find(|b| b.id == dune.id).unwrap();
        assert_eq!(
            listed.metadata.rating,
            Some(5),
            "the edit made during the scan is kept"
        );
        assert_eq!(indexed(&lib, &dune.id).unwrap().metadata.rating, Some(5));
    }

    /// A book that reaches the index while the scan walks (an import, a
    /// rename) wasn't there for the walk to see: it must not be marked
    /// missing, and is listed.
    #[test]
    fn a_book_added_during_a_scan_is_listed_and_not_marked_missing() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path().to_path_buf();
        write_epub(&lib.join("a.epub"), "A", "X");
        scan(&lib);
        write_epub(&lib.join("b.epub"), "B", "X");

        let added: Arc<Mutex<Option<String>>> = Arc::default();
        let _hook = hooks::on(hooks::READ, &lib, {
            let (lib, added) = (lib.clone(), added.clone());
            move |path| {
                if !path.ends_with("b.epub") {
                    return;
                }
                // As an import running meanwhile would: a new file, indexed.
                let c = lib.join("Later").join("c.epub");
                write_epub_creating_dirs(&c);
                let book = build_book_from_path(&c).unwrap();
                *added.lock().unwrap() = Some(book.id.clone());
                with_index(&lib, &no_log, |index| {
                    index.insert(book);
                    index.save()
                })
                .unwrap();
            }
        });
        let (books, summary) = scan_at(&lib, Utc::now(), &no_log).unwrap();
        let c_id = added.lock().unwrap().clone().expect("the hook ran");
        assert_eq!(summary.now_missing, 0);
        assert!(books.iter().any(|b| b.id == c_id), "listed");
        assert_eq!(books.len(), 3);
        assert!(indexed(&lib, &c_id).is_some());
    }

    fn write_epub_creating_dirs(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        write_epub(path, "C", "X");
    }
}
