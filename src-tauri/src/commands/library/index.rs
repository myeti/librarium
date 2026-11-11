//! The library index: `<library>/.librarium/library.json`.
//!
//! The index holds each book's record (stable id, metadata, date added…).
//! Part of that data exists nowhere else — edits to PDF/MOBI metadata,
//! ratings, categories, the ids the covers are cached under — so the file is
//! handled with care.
//!
//! # Files
//!
//! | File | Who writes it | Role |
//! |---|---|---|
//! | `library.json` | this version | the index (format v2, below) |
//! | `library.json.bak` | this version | the previous good `library.json` |
//! | `books.json`, `books.json.bak` | an older format only | the **older** index (format v1): read once, to create `library.json`; **never written** |
//! | `books.v1.json.bak` | this version, once | byte copy of the v1 file a migration started from |
//!
//! Once `library.json` exists it is the only index read: `books.json` is left
//! untouched, and changes made to it afterwards are **not** picked up (books
//! added there are found by the next scan as new files, but metadata edited
//! there is not imported). A note is logged when `books.json` is newer than
//! `library.json`.
//!
//! # Format (version 2)
//!
//! ```json
//! { "version": 2,
//!   "libraryId": "<uuid>",
//!   "books":   { "<relKey>": Book, … },
//!   "ignored": [ "<relKey>", … ],
//!   "missing": { "<relKey>": { "book": Book, "missingSince": "<rfc3339>" }, … } }
//! ```
//!
//! - **Keys are relative to the library folder** (`Herbert/Dune.epub`, `/`
//!   separators, Unicode NFC; see [`super::relkey`]), so a library that is
//!   moved or mounted under another path keeps working. In the file,
//!   `Book.path` is the same relative path (in the spelling found on disk);
//!   in memory and over IPC it is absolute.
//! - **`ignored`**: files the user removed from the library without deleting
//!   them (`forget_books`). Scans skip them; importing one un-ignores it.
//! - **`missing`**: books whose file a complete scan didn't find. They are
//!   not shown, and are re-linked (same id, metadata and date added) when a
//!   scan finds a new file with the same name and size — a book moved to
//!   another folder, or a NAS folder that was briefly unreadable. They are
//!   dropped after [`MISSING_RETENTION_DAYS`]. Missing keys are the last
//!   known relKey; for entries migrated from v1 that couldn't be placed, the
//!   old absolute path (such keys are never turned into a path).
//! - Top-level fields written by a newer version are kept as they are.
//!
//! Version 1 (`books.json`, an older format) is a plain object keyed by
//! absolute path. When there is no `library.json` (nor its backup) yet, that
//! file (or its own backup) is migrated in memory
//! ([`super::migrate`]) and the result is written to `library.json`; before
//! that first write the v1 file is also copied to `books.v1.json.bak` (never
//! overwriting an existing copy). The v1 file itself stays as it was.
//!
//! # Safety rules
//!
//! - **Writes are atomic** ([`fsutil::atomic_write`]: temp file, fsync,
//!   rename) and the previous good version is kept as `library.json.bak`.
//! - **A damaged index is never overwritten.** When `library.json` can't be
//!   parsed (or isn't text), its backup is used instead (the damaged file is
//!   kept aside as `library.json.damaged-<timestamp>` on the next save). When
//!   the backup is unusable too, loading fails, the command returns the error
//!   and nothing is written — there is deliberately no fallback to the legacy
//!   `books.json`, which may be months older. The same holds for a damaged
//!   legacy file when there is nothing else to migrate from. A file written
//!   by a *newer* version is refused outright (no fallback, nothing written).
//! - **A file that can't be *read* is not damaged.** An I/O error on a share
//!   (a timeout, a dropped SMB session) or a refused access (macOS privacy
//!   protection on `/Volumes`) fails the command — `LIBRARY_UNAVAILABLE`,
//!   `PERMISSION_DENIED` or `IO` — without looking at the backup: falling
//!   back would set the last save aside as "damaged" because of a network
//!   hiccup.
//! - **The legacy `books.json` is only ever read.**
//! - **Read-modify-write is serialised.** [`with_index`] holds a per-file lock
//!   ([`fsutil::with_file_lock`]) from load to save, so two commands running
//!   at the same time can't silently drop each other's changes. Slow work
//!   (walking the library, parsing new files, rewriting EPUBs, trashing) is
//!   done *outside* the lock by the commands, which lock again to merge the
//!   result and re-check what may have changed meanwhile — so a scan of a
//!   large NAS library doesn't hold every cover and edit for its duration.
//! - **A library folder that isn't there is never created** (an unmounted
//!   NAS share must not silently become an empty local folder): every access
//!   fails with `LIBRARY_UNAVAILABLE` ([`ensure_library_available`]).
//! - Every save also refreshes `.librarium/lock.json` ([`super::lock`]).
//!
//! # Cache
//!
//! Parsing a few thousand records on every command (from a NAS) adds up, so
//! an index that was loaded and left unchanged is kept in memory, keyed by
//! library path. It is reused only while `library.json` has the same size,
//! modification time (and inode, on Unix) as when it was read, so a change
//! made by another computer is picked up by the next command. An index that
//! was modified — saved or not — or whose command failed is dropped and read
//! again next time, so memory never drifts from the file.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::files::ensure_library_available;
use super::lock;
use super::migrate::migrate_v1;
use super::relkey::{is_valid_key, nfc, normalize_key, relative_to, resolve};
use crate::error::{AppError, AppResult};
use crate::fsutil;
use crate::logging::Log;
use crate::models::book::Book;

/// Folder holding Librarium's own files, relative to the library root.
pub(crate) const INDEX_DIR: &str = ".librarium";
/// The index this version reads and writes.
const INDEX_FILE: &str = "library.json";
const BACKUP_FILE: &str = "library.json.bak";
/// The older index (format v1), and its backup: read to migrate, never
/// written (see the module docs).
const LEGACY_FILE: &str = "books.json";
const LEGACY_BACKUP_FILE: &str = "books.json.bak";
/// Copy of a v1 index, made before it is first migrated.
pub(crate) const V1_BACKUP_FILE: &str = "books.v1.json.bak";

/// The format this version reads and writes.
pub(crate) const FORMAT_VERSION: u32 = 2;

/// How long a missing book is remembered before it is dropped.
pub(crate) const MISSING_RETENTION_DAYS: i64 = 60;

/// `<library>/.librarium/library.json`.
pub(crate) fn index_path(library: &Path) -> PathBuf {
    library.join(INDEX_DIR).join(INDEX_FILE)
}

fn backup_path(library: &Path) -> PathBuf {
    library.join(INDEX_DIR).join(BACKUP_FILE)
}

/// `<library>/.librarium/books.json`, the legacy (v1) index. Read only.
pub(crate) fn legacy_path(library: &Path) -> PathBuf {
    library.join(INDEX_DIR).join(LEGACY_FILE)
}

fn legacy_backup_path(library: &Path) -> PathBuf {
    library.join(INDEX_DIR).join(LEGACY_BACKUP_FILE)
}

// ── On-disk format ───────────────────────────────────────────────────────────

/// `library.json`, format v2, as stored.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IndexFile {
    version: u32,
    #[serde(default)]
    library_id: String,
    /// relKey → book, `Book.path` relative.
    #[serde(default)]
    books: BTreeMap<String, Book>,
    #[serde(default)]
    ignored: BTreeSet<String>,
    #[serde(default)]
    missing: BTreeMap<String, MissingBook>,
    /// Top-level fields this version doesn't know (written by a newer one).
    #[serde(flatten)]
    extra: serde_json::Map<String, serde_json::Value>,
}

/// A book whose file a complete scan didn't find.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MissingBook {
    pub(crate) book: Book,
    /// When a scan first noticed the file was gone (RFC 3339).
    pub(crate) missing_since: String,
}

/// Why a candidate index file couldn't be used.
#[derive(Debug)]
enum ReadError {
    Missing,
    /// Damaged (not valid JSON, not text, not an index): the backup may be
    /// used instead.
    Unusable(String),
    /// Written by a newer Librarium: never fall back, never overwrite.
    TooNew(u32),
}

/// An index file, parsed and normalised in memory (see [`parse_index`]).
#[derive(Debug, Default)]
struct Parsed {
    library_id: String,
    books: HashMap<String, Book>,
    ids: HashMap<String, String>,
    ignored: BTreeSet<String>,
    missing: BTreeMap<String, MissingBook>,
    extra: serde_json::Map<String, serde_json::Value>,
    /// The data in memory differs from the file (migration, repairs) and
    /// should be saved.
    changed: bool,
    /// What loading did, for the debug log.
    notes: Vec<String>,
}

/// Parse an index file's text for the library at `library`: v2 as is
/// (validated), v1 migrated. Paths in the result are absolute.
fn parse_index(library: &Path, text: &str, now: DateTime<Utc>) -> Result<Parsed, ReadError> {
    let mut parsed = match serde_json::from_str::<IndexFile>(text) {
        Ok(file) if file.version == FORMAT_VERSION => from_v2(library, file, now),
        Ok(file) if file.version > FORMAT_VERSION => return Err(ReadError::TooNew(file.version)),
        Ok(file) => {
            return Err(ReadError::Unusable(format!(
                "has an unknown format version ({})",
                file.version
            )))
        }
        Err(typed_err) => match serde_json::from_str::<serde_json::Value>(text) {
            Err(e) => return Err(ReadError::Unusable(format!("is not valid JSON: {e}"))),
            Ok(serde_json::Value::Object(map)) if !map.contains_key("version") => {
                from_v1(library, map, now)
            }
            Ok(serde_json::Value::Object(map)) => {
                if let Some(version) = map["version"]
                    .as_u64()
                    .filter(|&v| v > u64::from(FORMAT_VERSION))
                {
                    return Err(ReadError::TooNew(
                        u32::try_from(version).unwrap_or(u32::MAX),
                    ));
                }
                return Err(ReadError::Unusable(format!("is not valid: {typed_err}")));
            }
            Ok(_) => return Err(ReadError::Unusable("is not a JSON object".into())),
        },
    };
    clear_placeholders(&mut parsed);
    Ok(parsed)
}

/// Load a v2 file: resolve paths, and quarantine anything that could point
/// outside the library or break the one-record-per-id rule.
fn from_v2(library: &Path, file: IndexFile, now: DateTime<Utc>) -> Parsed {
    let mut parsed = Parsed {
        library_id: file.library_id,
        ignored: file
            .ignored
            .iter()
            .map(|k| normalize_key(k))
            .filter(|k| is_valid_key(k))
            .collect(),
        missing: file.missing,
        extra: file.extra,
        ..Default::default()
    };
    if parsed.ignored.len() != file.ignored.len() {
        parsed.changed = true;
    }
    for (stored_key, mut book) in file.books {
        let key = normalize_key(&stored_key);
        if key != stored_key {
            parsed.changed = true;
        }
        if !is_valid_key(&key) || parsed.books.contains_key(&key) {
            // Hand-edited or corrupted: never resolve it to a path. Kept (as
            // missing) so a scan can re-link the book if its file is found.
            parsed.notes.push(format!(
                "entry {stored_key:?} can't be a path inside the library; kept as missing"
            ));
            parsed.changed = true;
            quarantine(&mut parsed.missing, stored_key, book, now);
            continue;
        }
        // The stored relative path is the on-disk spelling of the key; fall
        // back on the key itself if it doesn't match.
        let rel = if is_valid_key(&book.path) && normalize_key(&book.path) == key {
            std::mem::take(&mut book.path)
        } else {
            parsed.changed = true;
            key.clone()
        };
        book.path = resolve(library, &rel)
            .expect("validated key")
            .to_string_lossy()
            .into_owned();
        if parsed.ids.contains_key(&book.id) || book.id.is_empty() {
            let fresh = Uuid::new_v4().to_string();
            parsed.notes.push(format!(
                "duplicate book id {:?} at {key:?}; given the new id {fresh}",
                book.id
            ));
            book.id = fresh;
            parsed.changed = true;
        }
        parsed.ids.insert(book.id.clone(), key.clone());
        parsed.books.insert(key, book);
    }
    // A missing record must not share an id with a live one: re-linking it
    // would replace the live book.
    for entry in parsed.missing.values_mut() {
        if parsed.ids.contains_key(&entry.book.id) || entry.book.id.is_empty() {
            entry.book.id = Uuid::new_v4().to_string();
            parsed.changed = true;
        }
        if DateTime::parse_from_rfc3339(&entry.missing_since).is_err() {
            entry.missing_since = now.to_rfc3339();
            parsed.changed = true;
        }
    }
    if parsed.library_id.is_empty() {
        parsed.library_id = Uuid::new_v4().to_string();
        parsed.changed = true;
    }
    parsed
}

/// Migrate a v1 file (see [`super::migrate`]).
fn from_v1(
    library: &Path,
    entries: serde_json::Map<String, serde_json::Value>,
    now: DateTime<Utc>,
) -> Parsed {
    let migration = migrate_v1(library, entries, &|p| p.exists());
    let mut parsed = Parsed {
        library_id: Uuid::new_v4().to_string(),
        changed: true,
        notes: vec![format!(
            "migrating the index from format v1: {}",
            migration.report.summary()
        )],
        ..Default::default()
    };
    for (key, mut book) in migration.books {
        if parsed.ids.contains_key(&book.id) || book.id.is_empty() {
            book.id = Uuid::new_v4().to_string();
        }
        parsed.ids.insert(book.id.clone(), key.clone());
        parsed.books.insert(key, book);
    }
    for (old_key, mut book) in migration.missing {
        if parsed.ids.contains_key(&book.id) || book.id.is_empty() {
            book.id = Uuid::new_v4().to_string();
        }
        parsed.missing.insert(
            old_key,
            MissingBook {
                book,
                missing_since: now.to_rfc3339(),
            },
        );
    }
    parsed
}

/// A copy of `book` as written to the index file: the cover fields are
/// output only (filled per request from the cover cache), never stored.
fn stored_book(book: &Book) -> Book {
    Book {
        cover_path: None,
        cover_version: None,
        ..book.clone()
    }
}

/// Put `book` in `missing` under `key`, without replacing another record.
fn quarantine(
    missing: &mut BTreeMap<String, MissingBook>,
    key: String,
    book: Book,
    now: DateTime<Utc>,
) {
    let key = if missing.contains_key(&key) {
        format!("{key}#{}", book.id)
    } else {
        key
    };
    missing.insert(
        key,
        MissingBook {
            book,
            missing_since: now.to_rfc3339(),
        },
    );
}

/// Older versions stored "Unknown title" / "Unknown author" as data (and
/// wrote them into EPUBs). An unknown value is now an empty string; the UI
/// shows a localised placeholder.
fn clear_placeholders(parsed: &mut Parsed) {
    let mut cleared = 0usize;
    let books = parsed
        .books
        .values_mut()
        .chain(parsed.missing.values_mut().map(|m| &mut m.book));
    for book in books {
        if book.metadata.clear_placeholders() {
            cleared += 1;
        }
    }
    if cleared > 0 {
        parsed
            .notes
            .push(format!("{cleared} placeholder title(s)/author(s) cleared"));
        parsed.changed = true;
    }
}

// ── The index in memory ──────────────────────────────────────────────────────

/// Where the loaded entries came from. Decides what [`LibraryIndex::save`]
/// does with the existing files before writing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Origin {
    /// No index at all yet (new library).
    Missing,
    /// `library.json`, parsed successfully.
    Main,
    /// `library.json.bak`, because `library.json` was missing or damaged.
    Backup,
    /// The legacy `books.json` (or its backup), migrated in memory: there is
    /// no `library.json` yet. The legacy file is never written.
    Legacy,
}

/// What identifies one version of `library.json` on disk, to validate the cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileStamp {
    len: u64,
    modified: Option<SystemTime>,
    #[cfg(unix)]
    dev: u64,
    #[cfg(unix)]
    ino: u64,
}

fn file_stamp(path: &Path) -> Option<FileStamp> {
    let meta = fs::metadata(path).ok()?;
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;
    Some(FileStamp {
        len: meta.len(),
        modified: meta.modified().ok(),
        #[cfg(unix)]
        dev: meta.dev(),
        #[cfg(unix)]
        ino: meta.ino(),
    })
}

/// The library index, loaded from disk. Obtain one with [`with_index`], which
/// guarantees the index lock is held while you use it; nothing is written
/// until you call [`LibraryIndex::save`].
///
/// Books are stored by key (path relative to the library, NFC), with an
/// id → key map for lookups by id. In memory, `Book.path` is absolute.
#[derive(Debug)]
pub(crate) struct LibraryIndex {
    library: PathBuf,
    library_id: String,
    books: HashMap<String, Book>,
    /// Book id → key in `books`.
    ids: HashMap<String, String>,
    ignored: BTreeSet<String>,
    missing: BTreeMap<String, MissingBook>,
    extra: serde_json::Map<String, serde_json::Value>,
    origin: Origin,
    /// Whether `save` already rotated the backup during this command, so
    /// several saves under one lock keep the *pre-command* state as the backup.
    prepared: bool,
    /// The legacy file this index was migrated from, until its byte copy
    /// (`books.v1.json.bak`) is made by the first save.
    legacy_source: Option<PathBuf>,
    /// Memory differs from `library.json`.
    dirty: bool,
    /// The state of `library.json` when it was read; `None` once anything was
    /// written (the cache then no longer trusts this copy).
    stamp: Option<FileStamp>,
    /// Lines for the debug log, collected while loading.
    notes: Vec<String>,
    /// Human-readable explanation when the index was recovered from backup.
    recovery_note: Option<String>,
}

/// Read and parse the index file at `path`. The outer error is a failed
/// *read* (a share that stopped answering, access refused), which says
/// nothing about the content; the inner one is about the file itself.
fn read_file(
    path: &Path,
    library: &Path,
    now: DateTime<Utc>,
) -> io::Result<Result<Parsed, ReadError>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Err(ReadError::Missing)),
        // Not UTF-8: the content is damaged.
        Err(e) if e.kind() == io::ErrorKind::InvalidData => {
            return Ok(Err(ReadError::Unusable(format!("can't be read: {e}"))))
        }
        Err(e) => return Err(e),
    };
    Ok(parse_index(library, &text, now))
}

/// [`AppError::IndexTooNew`] for the file at `path`. (Index messages keep
/// the file's path: the user needs it to restore or move the file.)
fn too_new(path: &Path, version: u32) -> AppError {
    AppError::IndexTooNew(format!(
        "The library index {} was written by a newer version of Librarium (format {version}; \
         this version reads format {FORMAT_VERSION}). Update Librarium to open this library. \
         Nothing was changed.",
        path.display()
    ))
}

fn describe(err: ReadError) -> String {
    match err {
        ReadError::Missing => "does not exist".to_string(),
        ReadError::Unusable(e) => e,
        ReadError::TooNew(v) => format!("was written by a newer version (format {v})"),
    }
}

/// [`read_file`], with a failure to *read* the file (as opposed to a file
/// that isn't a good index) turned into the command's error right away, so
/// the fallback rules of [`read_sources`] only ever see missing, damaged or
/// too-new files. Access refused is `PERMISSION_DENIED`, a share that
/// stopped answering `LIBRARY_UNAVAILABLE`, anything else `IO`.
fn read_candidate(
    path: &Path,
    library: &Path,
    now: DateTime<Utc>,
) -> AppResult<Result<Parsed, ReadError>> {
    read_file(path, library, now).map_err(|e| match AppError::folder(library, &e) {
        AppError::Io(_) => AppError::io(
            format!(
                "Could not read the library index {} (nothing was changed)",
                path.display()
            ),
            &e,
        ),
        other => other,
    })
}

/// An index read from one of the candidate files.
struct Loaded {
    parsed: Parsed,
    origin: Origin,
    /// The file the data came from.
    source: PathBuf,
    /// Explanation of a fallback, for the log.
    note: Option<String>,
}

/// Find and parse the index of `library`: `library.json`, else its backup,
/// else — only when this version never wrote an index here — the legacy
/// `books.json` or its backup ([`read_legacy`]). Nothing is written.
fn read_sources(library: &Path, now: DateTime<Utc>) -> AppResult<Loaded> {
    let main = index_path(library);
    let backup = backup_path(library);
    match read_candidate(&main, library, now)? {
        Ok(parsed) => Ok(Loaded {
            parsed,
            origin: Origin::Main,
            source: main,
            note: None,
        }),
        Err(ReadError::TooNew(v)) => Err(too_new(&main, v)),
        Err(ReadError::Missing) => match read_candidate(&backup, library, now)? {
            // The main file was deleted or lost, but a backup survived: keep its data.
            Ok(parsed) => {
                let note = format!(
                    "{} is missing; restored {} book(s) from {BACKUP_FILE}",
                    main.display(),
                    parsed.books.len()
                );
                Ok(Loaded {
                    parsed,
                    origin: Origin::Backup,
                    source: backup,
                    note: Some(note),
                })
            }
            Err(ReadError::TooNew(v)) => Err(too_new(&backup, v)),
            // Neither file: this version never indexed this library.
            Err(ReadError::Missing) => read_legacy(library, now),
            Err(ReadError::Unusable(backup_err)) => Err(AppError::IndexDamaged(format!(
                "The library index {} is missing, and its backup {BACKUP_FILE} {backup_err}. \
                 Nothing was changed. Restore the file from a backup, or move {BACKUP_FILE} away \
                 to start a new index (metadata edited in Librarium would be lost).",
                main.display()
            ))),
        },
        // Damaged: the backup, never the (older) legacy file.
        Err(ReadError::Unusable(main_err)) => match read_candidate(&backup, library, now)? {
            Ok(parsed) => {
                let note = format!(
                    "{} {main_err}; using {BACKUP_FILE} instead ({} book(s)). \
                     The damaged file will be kept aside on the next save.",
                    main.display(),
                    parsed.books.len()
                );
                Ok(Loaded {
                    parsed,
                    origin: Origin::Backup,
                    source: backup,
                    note: Some(note),
                })
            }
            Err(ReadError::TooNew(v)) => Err(too_new(&backup, v)),
            Err(backup_err) => Err(AppError::IndexDamaged(format!(
                "The library index {} {main_err}, and its backup {BACKUP_FILE} {}. \
                 Nothing was changed. Restore the file from a backup, or move it away \
                 to start a new index (metadata edited in Librarium would be lost).",
                main.display(),
                describe(backup_err)
            ))),
        },
    }
}

/// [`read_sources`] when there is no `library.json` yet: the legacy index
/// (`books.json`, else `books.json.bak`), to be migrated. No legacy file
/// either: a new, empty library. The legacy files are only read.
fn read_legacy(library: &Path, now: DateTime<Utc>) -> AppResult<Loaded> {
    let legacy = legacy_path(library);
    let legacy_backup = legacy_backup_path(library);
    let empty = || Loaded {
        parsed: Parsed::default(),
        origin: Origin::Missing,
        source: index_path(library),
        note: None,
    };
    match read_candidate(&legacy, library, now)? {
        Ok(parsed) => Ok(Loaded {
            parsed,
            origin: Origin::Legacy,
            source: legacy,
            note: None,
        }),
        Err(ReadError::TooNew(v)) => Err(too_new(&legacy, v)),
        Err(ReadError::Missing) => match read_candidate(&legacy_backup, library, now)? {
            Ok(parsed) => {
                let note = format!(
                    "{} is missing; migrating its backup {LEGACY_BACKUP_FILE}",
                    legacy.display()
                );
                Ok(Loaded {
                    parsed,
                    origin: Origin::Legacy,
                    source: legacy_backup,
                    note: Some(note),
                })
            }
            Err(ReadError::TooNew(v)) => Err(too_new(&legacy_backup, v)),
            Err(ReadError::Missing) => Ok(empty()),
            Err(ReadError::Unusable(e)) => {
                // A stray, damaged backup of an index that no longer exists:
                // nothing to migrate (and nothing is ever written over it).
                let mut loaded = empty();
                loaded.note = Some(format!(
                    "{} {e}; starting a new index (the file is left as it is)",
                    legacy_backup.display()
                ));
                Ok(loaded)
            }
        },
        Err(ReadError::Unusable(legacy_err)) => match read_candidate(&legacy_backup, library, now)?
        {
            Ok(parsed) => {
                let note = format!(
                    "{} {legacy_err}; migrating its backup {LEGACY_BACKUP_FILE} instead (the damaged file is left as it is)",
                    legacy.display()
                );
                Ok(Loaded {
                    parsed,
                    origin: Origin::Legacy,
                    source: legacy_backup,
                    note: Some(note),
                })
            }
            Err(ReadError::TooNew(v)) => Err(too_new(&legacy_backup, v)),
            Err(backup_err) => Err(AppError::IndexDamaged(format!(
                "The old library index {} {legacy_err}, and its backup {LEGACY_BACKUP_FILE} {}. \
                 Nothing was changed. Restore the file from a backup, or move it away \
                 to start a new index (metadata edited in Librarium would be lost).",
                legacy.display(),
                describe(backup_err)
            ))),
        },
    }
}

/// A note for the log when the older `books.json` was modified after
/// `library.json`: something (here or on another computer) still writes it,
/// and its changes aren't picked up.
fn legacy_newer_note(library: &Path) -> Option<String> {
    let modified = |p: &Path| fs::metadata(p).and_then(|m| m.modified()).ok();
    let (legacy, main) = (
        modified(&legacy_path(library))?,
        modified(&index_path(library))?,
    );
    (legacy > main).then(|| {
        format!(
            "{LEGACY_FILE} was changed after {INDEX_FILE}, probably by an older Librarium \
             (on this or another computer). Its changes are not used: {INDEX_FILE} is this \
             version's index. Books added there are found by the next scan; metadata edited \
             there is not imported."
        )
    })
}

impl LibraryIndex {
    /// Load the index of `library` (see the module docs for the fallback
    /// rules). Prefer [`with_index`], which also takes the lock.
    pub(crate) fn load(library: &Path) -> AppResult<Self> {
        Self::load_at(library, Utc::now())
    }

    fn load_at(library: &Path, now: DateTime<Utc>) -> AppResult<Self> {
        // Taken *before* reading: if the file changes in between, the stamp
        // is the old one and the cache reloads, never the other way round.
        let stamp = file_stamp(&index_path(library));
        let Loaded {
            mut parsed,
            origin,
            source,
            note,
        } = read_sources(library, now)?;
        if origin == Origin::Main {
            parsed.notes.extend(legacy_newer_note(library));
        }
        let library_id = if origin == Origin::Missing {
            Uuid::new_v4().to_string()
        } else {
            parsed.library_id
        };
        Ok(Self {
            library: library.to_path_buf(),
            library_id,
            books: parsed.books,
            ids: parsed.ids,
            ignored: parsed.ignored,
            missing: parsed.missing,
            extra: parsed.extra,
            origin,
            prepared: false,
            legacy_source: (origin == Origin::Legacy).then_some(source),
            // A migrated legacy index is always written to library.json once.
            dirty: parsed.changed || origin == Origin::Legacy,
            stamp: if origin == Origin::Main { stamp } else { None },
            notes: parsed.notes,
            recovery_note: note,
        })
    }

    /// The file as it will be written: format v2, relative paths.
    fn to_json(&self) -> Result<Vec<u8>, String> {
        let books = self
            .books
            .iter()
            .map(|(key, book)| {
                let mut stored = stored_book(book);
                // The on-disk spelling of the key (NFD names stay NFD), or the key.
                stored.path = relative_to(&self.library, Path::new(&book.path))
                    .filter(|rel| normalize_key(rel) == *key)
                    .unwrap_or_else(|| key.clone());
                (key.clone(), stored)
            })
            .collect();
        let missing = self
            .missing
            .iter()
            .map(|(key, m)| {
                (
                    key.clone(),
                    MissingBook {
                        book: stored_book(&m.book),
                        ..m.clone()
                    },
                )
            })
            .collect();
        let file = IndexFile {
            version: FORMAT_VERSION,
            library_id: self.library_id.clone(),
            books,
            ignored: self.ignored.clone(),
            missing,
            extra: self.extra.clone(),
        };
        serde_json::to_vec_pretty(&file)
            .map_err(|e| format!("Could not serialise the library index: {e}"))
    }

    /// Write the index to `library.json`, atomically. The legacy
    /// `books.json` is never written.
    ///
    /// The first save of a command also protects the previous state:
    /// - migrated from the legacy index: the file it came from is first
    ///   copied to `books.v1.json.bak` (or `books.v1.json_2.bak`… if that
    ///   exists); if the copy fails, nothing is written. (The v1 file
    ///   itself stays in place.)
    /// - loaded from a good `library.json`: it is copied to
    ///   `library.json.bak` first, so the backup is always the last version
    ///   that parsed;
    /// - loaded from the backup: the damaged `library.json` (if any) is
    ///   renamed to `library.json.damaged-<timestamp>` rather than
    ///   overwritten, and the good backup is left alone.
    ///
    /// Then `.librarium/lock.json` is refreshed (best effort).
    pub(crate) fn save(&mut self) -> AppResult<()> {
        let main = index_path(&self.library);
        let dir = self.library.join(INDEX_DIR);
        fs::create_dir_all(&dir)
            .map_err(|e| AppError::io(format!("Could not create the {INDEX_DIR} folder"), &e))?;
        let json = self.to_json().map_err(AppError::Internal)?;
        // From here on the file may change: this copy is no longer what's on disk.
        self.stamp = None;

        if !self.prepared {
            if let Some(source) = &self.legacy_source {
                let dest = fsutil::unique_path(&dir, V1_BACKUP_FILE);
                fsutil::atomic_copy(source, &dest).map_err(|e| {
                    AppError::io("Could not back up the old library index before migrating it (nothing was changed)", &e)
                })?;
            }
            match self.origin {
                Origin::Main => {
                    fsutil::atomic_copy(&main, &backup_path(&self.library))
                        .map_err(|e| AppError::io("Could not back up the library index", &e))?;
                }
                Origin::Backup if main.exists() => {
                    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
                    let aside = fsutil::unique_path(&dir, &format!("{INDEX_FILE}.damaged-{stamp}"));
                    fs::rename(&main, &aside).map_err(|e| {
                        AppError::io("Could not move the damaged library index aside", &e)
                    })?;
                }
                // Legacy: nothing of ours to rotate yet, and the legacy
                // files are left exactly as they are.
                Origin::Backup | Origin::Missing | Origin::Legacy => {}
            }
            self.legacy_source = None;
            self.prepared = true;
        }

        fsutil::atomic_write(&main, &json)
            .map_err(|e| AppError::io("Could not save the library index", &e))?;
        self.origin = Origin::Main;
        self.dirty = false;
        if let Err(e) = lock::refresh(&self.library) {
            eprintln!("[library index] could not refresh {}: {e}", lock::LOCK_FILE);
        }
        Ok(())
    }

    // ── Reading ──────────────────────────────────────────────────────────────

    /// The library's stable id (`libraryId`).
    #[allow(dead_code)] // for the device manifest
    pub(crate) fn library_id(&self) -> &str {
        &self.library_id
    }

    /// Number of books in the index (missing ones excluded).
    pub(crate) fn len(&self) -> usize {
        self.books.len()
    }

    /// The key of a path inside the library (`None` outside it).
    pub(crate) fn key_of(&self, path: &Path) -> Option<String> {
        relative_to(&self.library, path)
            .map(|rel| normalize_key(&rel))
            .filter(|k| is_valid_key(k))
    }

    /// The book stored for `path` (an absolute path in the library), if any.
    pub(crate) fn get(&self, path: &str) -> Option<&Book> {
        self.books.get(&self.key_of(Path::new(path))?)
    }

    /// The book stored under `key`, if any.
    pub(crate) fn get_key(&self, key: &str) -> Option<&Book> {
        self.books.get(key)
    }

    /// The book with this id, if any.
    pub(crate) fn by_id(&self, id: &str) -> Option<&Book> {
        self.ids.get(id).and_then(|key| self.books.get(key))
    }

    /// All books, in no particular order (missing ones excluded).
    pub(crate) fn books(&self) -> impl Iterator<Item = &Book> {
        self.books.values()
    }

    /// Whether the file at `key` was removed from the library by the user.
    pub(crate) fn is_ignored(&self, key: &str) -> bool {
        self.ignored.contains(key)
    }

    /// The missing books, by key.
    #[cfg(test)]
    pub(crate) fn missing(&self) -> &BTreeMap<String, MissingBook> {
        &self.missing
    }

    /// The ignored keys.
    #[cfg(test)]
    pub(crate) fn ignored(&self) -> &BTreeSet<String> {
        &self.ignored
    }

    // ── Changing ─────────────────────────────────────────────────────────────

    /// Store `book` under the key of its (absolute) path, replacing whatever
    /// was there. A book has one place: any other record with the same id
    /// (in the books or the missing ones) is removed, and the key stops being
    /// ignored (importing an ignored file brings it back).
    ///
    /// A path outside the library can't be keyed. That is a bug in the
    /// caller; rather than lose the record it is kept as missing (and logged
    /// to stderr).
    pub(crate) fn insert(&mut self, book: Book) {
        match self.key_of(Path::new(&book.path)) {
            Some(key) => self.insert_at(key, book),
            None => {
                eprintln!(
                    "[library index] {} is outside the library {}; kept as missing",
                    book.path,
                    self.library.display()
                );
                self.remove_id(&book.id);
                quarantine(&mut self.missing, book.path.clone(), book, Utc::now());
                self.dirty = true;
            }
        }
    }

    fn insert_at(&mut self, key: String, book: Book) {
        if let Some(old_key) = self.ids.get(&book.id).cloned() {
            if old_key != key {
                self.books.remove(&old_key);
            }
        }
        if let Some(previous) = self.books.get(&key) {
            if previous.id != book.id {
                let previous_id = previous.id.clone();
                self.ids.remove(&previous_id);
            }
        }
        self.missing.retain(|_, m| m.book.id != book.id);
        self.ignored.remove(&key);
        self.ids.insert(book.id.clone(), key.clone());
        self.books.insert(key, book);
        self.dirty = true;
    }

    /// Remove and return the entry stored for `path` (an absolute path).
    pub(crate) fn remove(&mut self, path: &str) -> Option<Book> {
        let key = self.key_of(Path::new(path))?;
        self.remove_key(&key)
    }

    fn remove_key(&mut self, key: &str) -> Option<Book> {
        let book = self.books.remove(key)?;
        self.ids.remove(&book.id);
        self.dirty = true;
        Some(book)
    }

    /// Remove and return the book with this id.
    pub(crate) fn remove_id(&mut self, id: &str) -> Option<Book> {
        let key = self.ids.get(id)?.clone();
        self.remove_key(&key)
    }

    /// Remove the book with this id and ignore its file from now on (the
    /// file itself is not touched). Returns the removed book.
    pub(crate) fn forget(&mut self, id: &str) -> Option<Book> {
        let key = self.ids.get(id)?.clone();
        let book = self.remove_key(&key)?;
        self.ignored.insert(key);
        Some(book)
    }

    /// Record that the file of the book at `key` is now at `path` — the same
    /// file, spelled differently on disk (Unicode normalisation).
    pub(crate) fn set_path(&mut self, key: &str, path: &Path) {
        if let Some(book) = self.books.get_mut(key) {
            book.path = path.to_string_lossy().into_owned();
            self.dirty = true;
        }
    }

    /// Move every book whose key isn't in `seen` to `missing`, noting `now`
    /// as the time it went missing. Only call this after a *complete* walk
    /// of the library. Returns how many books were moved.
    #[cfg(test)]
    pub(crate) fn mark_missing_except(
        &mut self,
        seen: &HashSet<String>,
        now: DateTime<Utc>,
    ) -> usize {
        let gone: Vec<String> = self
            .books
            .keys()
            .filter(|k| !seen.contains(*k))
            .cloned()
            .collect();
        self.mark_missing(&gone, now)
    }

    /// Move the books stored under `keys` to `missing`, noting `now` as the
    /// time they went missing (keys that aren't books are skipped). Returns
    /// how many books were moved.
    pub(crate) fn mark_missing(&mut self, keys: &[String], now: DateTime<Utc>) -> usize {
        let mut moved = 0;
        for key in keys {
            let Some(mut book) = self.remove_key(key) else {
                continue;
            };
            book.path =
                relative_to(&self.library, Path::new(&book.path)).unwrap_or_else(|| key.clone());
            quarantine(&mut self.missing, key.clone(), book, now);
            moved += 1;
        }
        moved
    }

    /// The keys of the books (missing ones excluded).
    pub(crate) fn keys(&self) -> impl Iterator<Item = &String> {
        self.books.keys()
    }

    /// The keys of the files the user removed from the library (`ignored`).
    pub(crate) fn ignored_keys(&self) -> impl Iterator<Item = &String> {
        self.ignored.iter()
    }

    /// The file name (NFC) and size of every missing book: a new file with
    /// one of these would be re-linked to that book
    /// ([`Self::take_missing_match`]) rather than read.
    pub(crate) fn missing_signatures(&self) -> HashSet<(String, u64)> {
        self.missing
            .values()
            .map(|m| (nfc(&m.book.filename), m.book.size_bytes))
            .collect()
    }

    /// Take the missing book that a new file named `file_name`, of `size`
    /// bytes, found at `key`, most likely is: same file name (NFC) and size.
    /// A record last seen at `key` itself is preferred; otherwise the first
    /// in key order.
    pub(crate) fn take_missing_match(
        &mut self,
        file_name: &str,
        size: u64,
        key: &str,
    ) -> Option<MissingBook> {
        let name = nfc(file_name);
        let matches = |m: &MissingBook| m.book.size_bytes == size && nfc(&m.book.filename) == name;
        let found = match self.missing.get(key) {
            Some(m) if matches(m) => Some(key.to_string()),
            _ => self
                .missing
                .iter()
                .find(|(_, m)| matches(m))
                .map(|(k, _)| k.clone()),
        }?;
        self.dirty = true;
        self.missing.remove(&found)
    }

    /// Drop the missing books that went missing more than
    /// [`MISSING_RETENTION_DAYS`] before `now`. Returns how many.
    pub(crate) fn expire_missing(&mut self, now: DateTime<Utc>) -> usize {
        let limit = now - chrono::Duration::days(MISSING_RETENTION_DAYS);
        let before = self.missing.len();
        self.missing.retain(|_, m| {
            DateTime::parse_from_rfc3339(&m.missing_since).map_or(true, |since| since >= limit)
        });
        let expired = before - self.missing.len();
        if expired > 0 {
            self.dirty = true;
        }
        expired
    }

    /// Keep only the ignored keys for which `keep` is true; returns how many
    /// were removed.
    pub(crate) fn retain_ignored(&mut self, mut keep: impl FnMut(&str) -> bool) -> usize {
        let before = self.ignored.len();
        self.ignored.retain(|k| keep(k));
        let removed = before - self.ignored.len();
        if removed > 0 {
            self.dirty = true;
        }
        removed
    }

    /// Whether the index changed since it was loaded or last saved.
    pub(crate) fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Whether this copy can be kept in the cache: read from `books.json`,
    /// unchanged since.
    fn cacheable(&self) -> bool {
        self.origin == Origin::Main && !self.dirty && self.stamp.is_some()
    }
}

// ── Cache ────────────────────────────────────────────────────────────────────

type Cache = Mutex<HashMap<PathBuf, LibraryIndex>>;

fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// The cached index of `library`, if `books.json` hasn't changed since it
/// was read. The entry is removed either way: the caller owns it now.
fn cache_take(library: &Path) -> Option<LibraryIndex> {
    let cached = cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(library)?;
    let current = file_stamp(&index_path(library));
    (cached.stamp.is_some() && cached.stamp == current).then_some(cached)
}

/// Keep `index` for the next command, if it is still what's on disk.
fn cache_put(index: LibraryIndex) {
    if index.cacheable() {
        cache()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(index.library.clone(), index);
    }
}

/// Forget every cached index (after a backup was restored over the
/// libraries: none of them is what's on disk any more).
pub(crate) fn clear_cache() {
    cache().lock().unwrap_or_else(|e| e.into_inner()).clear();
}

#[cfg(test)]
fn cache_contains(library: &Path) -> bool {
    cache().lock().unwrap().contains_key(library)
}

// ── Access ───────────────────────────────────────────────────────────────────

/// The path the index lock is keyed on. Built from the canonical library
/// folder so it is the same whether or not `books.json` exists yet (the lock
/// key of a missing file can't be canonicalised).
fn lock_path(library: &Path) -> PathBuf {
    index_path(&fs::canonicalize(library).unwrap_or_else(|_| library.to_path_buf()))
}

/// Load the index of `library`, run `f` on it, and release the lock.
///
/// This is the only way commands touch the index: the lock is held for the
/// whole call, so a load → modify → [`LibraryIndex::save`] sequence inside `f`
/// can't interleave with another command's. `f` decides whether to save.
///
/// Fails with [`AppError::LibraryUnavailable`] when the library folder
/// doesn't exist (an unmounted share), [`AppError::PermissionDenied`] when
/// it can't be read. A recovery from backup and a migration are reported
/// through `log`; a migration (or repair) is saved right after loading, so it
/// happens once — if that save fails, the command still runs on the migrated
/// copy and the save is retried on the next change.
///
/// Keep `f` short: every command of the library waits for it (the covers
/// of the visible cards, edits, the OPDS reindex…). Walking the library,
/// parsing files, rewriting EPUBs, copying and trashing files are done
/// before or after, with the result merged under a second, short lock that
/// re-checks what may have changed meanwhile (see `scan`, `import`, `edit`).
///
/// Lock order: the import lock (`import::with_import_lock`) before the index
/// lock; the index lock before an EPUB's lock (rare re-writes under the
/// index lock); never the other way round.
pub(crate) fn with_index<T>(
    library: &Path,
    log: Log,
    f: impl FnOnce(&mut LibraryIndex) -> AppResult<T>,
) -> AppResult<T> {
    ensure_library_available(library)?;
    fsutil::with_file_lock(&lock_path(library), || {
        let mut index = match cache_take(library) {
            Some(index) => index,
            None => {
                let mut index = LibraryIndex::load(library)?;
                if let Some(note) = &index.recovery_note {
                    eprintln!("[library index] {note}");
                    log(format!("[library index] WARNING — {note}"));
                }
                for note in std::mem::take(&mut index.notes) {
                    log(format!("[library index] {note}"));
                }
                if index.dirty {
                    match index.save() {
                        Ok(()) => log(format!("[library index] saved as {INDEX_FILE} (format v2)")),
                        Err(e) => log(format!(
                            "[library index] WARNING — could not save the updated index yet: {e}"
                        )),
                    }
                }
                index
            }
        };
        index.prepared = false;
        let result = f(&mut index);
        if result.is_ok() {
            cache_put(index);
        }
        result
    })
}

/// Whether some thread holds the index lock of `library` right now (tests:
/// slow work must run outside it).
#[cfg(test)]
pub(crate) fn is_index_locked(library: &Path) -> bool {
    fsutil::is_file_locked(&lock_path(library))
}

/// A copy of the book with this id, looked up under the index lock.
pub(crate) fn find_book(library: &Path, id: &str, log: Log) -> AppResult<Book> {
    with_index(library, log, |index| {
        index.by_id(id).cloned().ok_or_else(|| not_in_library(id))
    })
}

/// The error for an id that isn't in the index ([`AppError::NotFound`]).
pub(crate) fn not_in_library(id: &str) -> AppError {
    AppError::NotFound(format!(
        "Book {id} is not in the library (it may have been removed or renamed; rescan the library)"
    ))
}

/// The books of `library` as stored right now, for readers that must not
/// lock or write (the OPDS server), found as [`with_index`] would:
/// `library.json`, else its backup, else (never indexed by this version) the
/// legacy `books.json` migrated in memory. Paths are absolute; missing and
/// ignored entries are left out. An unreadable index gives an empty list.
pub(crate) fn read_books_snapshot(library: &Path) -> Vec<Book> {
    read_sources(library, Utc::now())
        .map(|loaded| loaded.parsed.books.into_values().collect())
        .unwrap_or_default()
}

/// Every file [`read_books_snapshot`] may read, in the order it tries them.
/// A reader that caches a snapshot (the OPDS server) compares their size and
/// modification time to notice that the index changed.
pub(crate) fn snapshot_sources(library: &Path) -> [PathBuf; 4] {
    [
        index_path(library),
        backup_path(library),
        legacy_path(library),
        legacy_backup_path(library),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging::{no_log, LogLines};
    use crate::models::book::{BookFormat, BookMetadata};
    use serde_json::json;

    fn book(lib: &Path, name: &str) -> Book {
        Book {
            id: format!("id-{name}"),
            path: lib.join(name).to_string_lossy().to_string(),
            filename: name.rsplit('/').next().unwrap().to_string(),
            format: BookFormat::Epub,
            size_bytes: 1,
            date_added: "2026-01-01T00:00:00Z".into(),
            cover_path: None,
            cover_version: None,
            favorite: false,
            metadata: BookMetadata {
                title: name.to_string(),
                ..Default::default()
            },
        }
    }

    fn save_books(lib: &Path, books: &[Book]) {
        with_index(lib, &no_log, |index| {
            for b in books {
                index.insert(b.clone());
            }
            index.save()
        })
        .unwrap();
    }

    fn titles(lib: &Path) -> Vec<String> {
        let mut titles = with_index(lib, &no_log, |index| {
            Ok(index
                .books()
                .map(|b| b.metadata.title.clone())
                .collect::<Vec<_>>())
        })
        .unwrap();
        titles.sort();
        titles
    }

    fn file_json(lib: &Path) -> serde_json::Value {
        serde_json::from_str(&fs::read_to_string(index_path(lib)).unwrap()).unwrap()
    }

    fn write_index(lib: &Path, value: &serde_json::Value) {
        fs::create_dir_all(lib.join(INDEX_DIR)).unwrap();
        fs::write(index_path(lib), serde_json::to_vec_pretty(value).unwrap()).unwrap();
    }

    /// Write an older `books.json` (format v1).
    fn write_legacy(lib: &Path, value: &serde_json::Value) {
        fs::create_dir_all(lib.join(INDEX_DIR)).unwrap();
        fs::write(legacy_path(lib), serde_json::to_vec_pretty(value).unwrap()).unwrap();
    }

    fn v1_record(id: &str, path: &Path, title: &str, author: &str) -> serde_json::Value {
        json!({
            "id": id, "path": path, "filename": path.file_name().unwrap().to_string_lossy(),
            "format": "epub", "sizeBytes": 3, "dateAdded": "2025-05-05T05:05:05Z",
            "metadata": { "title": title, "author": author, "rating": 4 },
        })
    }

    // ── Loading and saving ───────────────────────────────────────────────────

    #[test]
    fn missing_index_is_an_empty_library() {
        let dir = tempfile::tempdir().unwrap();
        let index = LibraryIndex::load(dir.path()).unwrap();
        assert_eq!(index.len(), 0);
        assert_eq!(index.origin, Origin::Missing);
    }

    #[test]
    fn save_round_trips_and_keeps_the_previous_version_as_backup() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        save_books(lib, &[book(lib, "a.epub")]);
        assert!(
            !backup_path(lib).exists(),
            "nothing to back up on the first save"
        );

        save_books(lib, &[book(lib, "b.epub")]);
        assert_eq!(titles(lib), ["a.epub", "b.epub"]);
        let backup: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(backup_path(lib)).unwrap()).unwrap();
        assert_eq!(
            backup["books"].as_object().unwrap().len(),
            1,
            "the backup is the state before the last save"
        );
    }

    #[test]
    fn corrupt_index_without_backup_is_an_error_and_is_left_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        fs::create_dir_all(lib.join(INDEX_DIR)).unwrap();
        let garbage = b"{\"truncated\": ";
        fs::write(index_path(lib), garbage).unwrap();

        let result = with_index(lib, &no_log, |index| {
            index.insert(book(lib, "new.epub"));
            index.save()
        });
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Nothing was changed"), "{err}");
        assert_eq!(fs::read(index_path(lib)).unwrap(), garbage);
        assert!(!backup_path(lib).exists());
    }

    /// A `library.json` that can't be *read* (access refused, as macOS does
    /// on `/Volumes` until Librarium is allowed; an I/O error on a share) is
    /// not damaged: the command fails, the backup isn't used, and nothing is
    /// set aside — the next command reads the same, intact file.
    #[cfg(unix)]
    #[test]
    fn an_unreadable_index_is_not_mistaken_for_a_damaged_one() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        save_books(lib, &[book(lib, "a.epub")]);
        save_books(lib, &[book(lib, "b.epub")]);
        assert!(backup_path(lib).exists());

        let main = index_path(lib);
        fs::set_permissions(&main, fs::Permissions::from_mode(0o000)).unwrap();
        let result = with_index(lib, &no_log, |index| {
            index.insert(book(lib, "c.epub"));
            index.save()
        });
        fs::set_permissions(&main, fs::Permissions::from_mode(0o644)).unwrap();

        assert_eq!(result.unwrap_err().code(), "PERMISSION_DENIED");
        let aside = fs::read_dir(lib.join(INDEX_DIR))
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains("damaged")
            })
            .count();
        assert_eq!(aside, 0, "nothing set aside as damaged");
        assert_eq!(titles(lib), ["a.epub", "b.epub"]);
    }

    #[test]
    fn corrupt_index_falls_back_to_backup_and_keeps_the_damaged_file_aside() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        save_books(lib, &[book(lib, "a.epub")]);
        save_books(lib, &[book(lib, "b.epub")]); // backup now holds just "a"
        fs::write(index_path(lib), b"").unwrap(); // e.g. a crash mid-write

        let notes = LogLines::default();
        let recovered = with_index(lib, &notes.sink(), |index| {
            assert_eq!(index.origin, Origin::Backup);
            let n = index.len();
            index.insert(book(lib, "c.epub"));
            index.save()?;
            Ok(n)
        })
        .unwrap();
        assert_eq!(recovered, 1);
        assert!(notes.contains("WARNING"));
        assert_eq!(titles(lib), ["a.epub", "c.epub"]);

        let damaged: Vec<_> = fs::read_dir(lib.join(INDEX_DIR))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .filter(|n| n.starts_with("library.json.damaged-"))
            .collect();
        assert_eq!(damaged.len(), 1, "the damaged index is kept for inspection");
        // The good backup was not replaced by the damaged file.
        let backup: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(backup_path(lib)).unwrap()).unwrap();
        assert_eq!(backup["books"].as_object().unwrap().len(), 1);
    }

    #[test]
    fn missing_index_with_a_backup_restores_the_backup() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        save_books(lib, &[book(lib, "a.epub")]);
        save_books(lib, &[book(lib, "b.epub")]);
        fs::remove_file(index_path(lib)).unwrap();
        assert_eq!(titles(lib), ["a.epub"]);
    }

    #[test]
    fn concurrent_updates_are_not_lost() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path().to_path_buf();
        let handles: Vec<_> = (0..8)
            .map(|t| {
                let lib = lib.clone();
                std::thread::spawn(move || {
                    for i in 0..10 {
                        save_books(&lib, &[book(&lib, &format!("t{t}-{i}.epub"))]);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(titles(&lib).len(), 80);
    }

    #[test]
    fn id_lookups() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        save_books(lib, &[book(lib, "a.epub"), book(lib, "b.epub")]);
        with_index(lib, &no_log, |index| {
            assert_eq!(index.by_id("id-a.epub").unwrap().filename, "a.epub");
            assert!(index.by_id("nope").is_none());
            let path = index.by_id("id-b.epub").unwrap().path.clone();
            assert_eq!(index.remove(&path).unwrap().filename, "b.epub");
            assert!(
                index.by_id("id-b.epub").is_none(),
                "the id map follows removals"
            );
            assert_eq!(index.len(), 1);
            Ok(())
        })
        .unwrap();
        assert!(find_book(lib, "nope", &no_log)
            .unwrap_err()
            .to_string()
            .contains("not in the library"));
    }

    // ── Format v2 ─────────────────────────────────────────────────────────────

    #[test]
    fn the_file_stores_relative_paths_and_memory_absolute_ones() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        save_books(lib, &[book(lib, "Herbert/Dune.epub")]);
        let json = file_json(lib);
        assert_eq!(json["version"], 2);
        assert!(Uuid::parse_str(json["libraryId"].as_str().unwrap()).is_ok());
        assert_eq!(
            json["books"]["Herbert/Dune.epub"]["path"],
            "Herbert/Dune.epub"
        );
        assert_eq!(json["ignored"], json!([]));
        assert_eq!(json["missing"], json!({}));

        let stored = find_book(lib, "id-Herbert/Dune.epub", &no_log).unwrap();
        assert_eq!(PathBuf::from(stored.path), lib.join("Herbert/Dune.epub"));
    }

    #[test]
    fn a_moved_library_keeps_working() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("old").join("Alice");
        fs::create_dir_all(&old).unwrap();
        save_books(&old, &[book(&old, "Dune.epub")]);
        let new = dir.path().join("new").join("Alice");
        fs::create_dir_all(new.parent().unwrap()).unwrap();
        fs::rename(&old, &new).unwrap();

        let stored = find_book(&new, "id-Dune.epub", &no_log).unwrap();
        assert_eq!(PathBuf::from(stored.path), new.join("Dune.epub"));
    }

    #[test]
    fn insert_keeps_one_record_per_id_and_unignores() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        with_index(lib, &no_log, |index| {
            index.insert(book(lib, "a.epub"));
            index.forget("id-a.epub").unwrap();
            assert!(index.is_ignored("a.epub"));
            // Same book, imported again at the same place: no longer ignored.
            index.insert(book(lib, "a.epub"));
            assert!(!index.is_ignored("a.epub"));
            // The same id at another path moves the record.
            let mut moved = book(lib, "a.epub");
            moved.path = lib.join("sub/a.epub").to_string_lossy().into_owned();
            index.insert(moved);
            assert_eq!(index.len(), 1);
            assert!(index.get(&lib.join("a.epub").to_string_lossy()).is_none());
            assert!(index.get_key("sub/a.epub").is_some());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_path_outside_the_library_is_never_keyed() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path().join("lib");
        fs::create_dir_all(&lib).unwrap();
        with_index(&lib, &no_log, |index| {
            index.insert(book(dir.path(), "outside.epub"));
            assert_eq!(index.len(), 0);
            assert_eq!(index.missing().len(), 1, "kept, not lost");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn hand_edited_keys_cannot_escape_the_library() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path().join("lib");
        fs::create_dir_all(&lib).unwrap();
        let entry = |id: &str, path: &str| {
            json!({
                "id": id, "path": path, "filename": "x.epub", "format": "epub",
                "sizeBytes": 1, "dateAdded": "2026-01-01T00:00:00Z", "metadata": { "title": "X", "author": "" },
            })
        };
        write_index(
            &lib,
            &json!({
                "version": 2, "libraryId": "lib-1",
                "books": {
                    "../outside.epub": entry("evil", "../outside.epub"),
                    "/etc/passwd": entry("abs", "/etc/passwd"),
                    "ok.epub": entry("ok", "../sneaky/path.epub"),
                },
            }),
        );
        with_index(&lib, &no_log, |index| {
            assert_eq!(index.len(), 1);
            assert!(index.by_id("evil").is_none() && index.by_id("abs").is_none());
            // A bad stored path falls back on its (valid) key.
            assert_eq!(
                PathBuf::from(&index.by_id("ok").unwrap().path),
                lib.join("ok.epub")
            );
            assert_eq!(index.missing().len(), 2);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn duplicate_ids_are_repaired_on_load() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        let entry = |path: &str| {
            json!({
                "id": "same", "path": path, "filename": path, "format": "epub",
                "sizeBytes": 1, "dateAdded": "2026-01-01T00:00:00Z", "metadata": { "title": path, "author": "" },
            })
        };
        write_index(
            lib,
            &json!({ "version": 2, "libraryId": "x", "books": { "a.epub": entry("a.epub"), "b.epub": entry("b.epub") } }),
        );
        let ids = with_index(lib, &no_log, |index| {
            Ok(index.books().map(|b| b.id.clone()).collect::<HashSet<_>>())
        })
        .unwrap();
        assert_eq!(ids.len(), 2, "each book is addressable by its own id");
        // …and the repair was saved, so ids are stable from now on.
        let again = with_index(lib, &no_log, |index| {
            Ok(index.books().map(|b| b.id.clone()).collect::<HashSet<_>>())
        })
        .unwrap();
        assert_eq!(ids, again);
    }

    #[test]
    fn a_newer_format_is_refused_and_left_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        let v3 = json!({ "version": 3, "libraryId": "x", "books": {}, "shelves": [] });
        write_index(lib, &v3);
        let before = fs::read(index_path(lib)).unwrap();
        let err = with_index(lib, &no_log, |index| index.save())
            .unwrap_err()
            .to_string();
        assert!(err.contains("newer version of Librarium"), "{err}");
        assert_eq!(fs::read(index_path(lib)).unwrap(), before);
    }

    #[test]
    fn unknown_top_level_fields_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        write_index(
            lib,
            &json!({ "version": 2, "libraryId": "x", "books": {}, "shelves": ["favourites"] }),
        );
        save_books(lib, &[book(lib, "a.epub")]);
        assert_eq!(file_json(lib)["shelves"], json!(["favourites"]));
        assert_eq!(file_json(lib)["libraryId"], "x", "the library id is stable");
    }

    #[test]
    fn nfd_spellings_are_kept_on_disk_and_found_by_nfc_key() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        let nfd = "Caf\u{65}\u{301}.epub";
        save_books(lib, &[book(lib, nfd)]);
        let json = file_json(lib);
        let (key, value) = json["books"].as_object().unwrap().iter().next().unwrap();
        assert_eq!(key, "Caf\u{e9}.epub", "keys are NFC");
        assert_eq!(
            value["path"], nfd,
            "the stored path is the spelling found on disk"
        );
        with_index(lib, &no_log, |index| {
            assert!(
                index
                    .get(&lib.join("Caf\u{e9}.epub").to_string_lossy())
                    .is_some(),
                "found by either spelling"
            );
            assert!(index.get(&lib.join(nfd).to_string_lossy()).is_some());
            Ok(())
        })
        .unwrap();
    }

    // ── Missing books ─────────────────────────────────────────────────────────

    #[test]
    fn missing_books_expire_after_the_retention_period() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        let now = Utc::now();
        with_index(lib, &no_log, |index| {
            index.insert(book(lib, "old.epub"));
            index.insert(book(lib, "recent.epub"));
            index.mark_missing_except(&HashSet::new(), now - chrono::Duration::days(61));
            // "recent" went missing later.
            let recent = index.missing.get_mut("recent.epub").unwrap();
            recent.missing_since = (now - chrono::Duration::days(59)).to_rfc3339();
            assert_eq!(index.expire_missing(now), 1);
            assert_eq!(index.missing().keys().collect::<Vec<_>>(), ["recent.epub"]);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn missing_matches_prefer_the_same_place() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        with_index(lib, &no_log, |index| {
            let mut a = book(lib, "A/x.epub");
            a.id = "a".into();
            let mut b = book(lib, "B/x.epub");
            b.id = "b".into();
            index.insert(a);
            index.insert(b);
            index.mark_missing_except(&HashSet::new(), Utc::now());
            assert!(
                index.take_missing_match("x.epub", 2, "C/x.epub").is_none(),
                "size must match"
            );
            assert_eq!(
                index
                    .take_missing_match("x.epub", 1, "B/x.epub")
                    .unwrap()
                    .book
                    .id,
                "b"
            );
            assert_eq!(
                index
                    .take_missing_match("x.epub", 1, "C/x.epub")
                    .unwrap()
                    .book
                    .id,
                "a"
            );
            assert!(index.missing().is_empty());
            Ok(())
        })
        .unwrap();
    }

    // ── Migration from the legacy v1 index ────────────────────────────────────

    #[test]
    fn a_legacy_index_is_migrated_to_library_json_and_left_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path().join("Alice");
        fs::create_dir_all(lib.join("Herbert")).unwrap();
        let dune = lib.join("Herbert").join("Dune.epub");
        let v1 = json!({
            dune.to_string_lossy(): v1_record("d", &dune, "Dune", "Unknown author"),
            "/old/mount/Alice/Asimov/Foundation.epub": v1_record("f", Path::new("/old/mount/Alice/Asimov/Foundation.epub"), "Unknown title", "Asimov"),
            "/elsewhere/lost.epub": v1_record("l", Path::new("/elsewhere/lost.epub"), "Lost", "X"),
        });
        write_legacy(&lib, &v1);
        let original = fs::read(legacy_path(&lib)).unwrap();

        let notes = LogLines::default();
        let books = with_index(&lib, &notes.sink(), |index| {
            Ok(index.books().cloned().collect::<Vec<_>>())
        })
        .unwrap();
        assert!(notes.contains("migrating the index from format v1"));
        assert_eq!(books.len(), 2);

        // The new index is library.json; books.json is exactly as it was
        // (an older Librarium can still open it), and a byte copy was kept.
        assert_eq!(
            fs::read(legacy_path(&lib)).unwrap(),
            original,
            "the legacy file is never written"
        );
        assert_eq!(
            fs::read(lib.join(INDEX_DIR).join(V1_BACKUP_FILE)).unwrap(),
            original
        );
        assert!(
            !legacy_backup_path(&lib).exists(),
            "no rotation of the legacy files"
        );
        let json = file_json(&lib);
        assert_eq!(json["version"], 2);
        let dune_entry = &json["books"]["Herbert/Dune.epub"];
        assert_eq!(dune_entry["id"], "d");
        assert_eq!(dune_entry["path"], "Herbert/Dune.epub");
        assert_eq!(dune_entry["metadata"]["rating"], 4, "metadata kept");
        assert_eq!(
            dune_entry["dateAdded"], "2025-05-05T05:05:05Z",
            "date added kept"
        );
        assert_eq!(dune_entry["metadata"]["author"], "", "placeholders cleared");
        assert_eq!(
            json["books"]["Asimov/Foundation.epub"]["metadata"]["title"],
            ""
        );
        assert_eq!(json["missing"]["/elsewhere/lost.epub"]["book"]["id"], "l");

        // Loaded again: no second migration, no second copy.
        let notes = LogLines::default();
        with_index(&lib, &notes.sink(), |_| Ok(())).unwrap();
        assert!(!notes.contains("migrating"));
        let v1_backups = fs::read_dir(lib.join(INDEX_DIR))
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with("books.v1")
            })
            .count();
        assert_eq!(v1_backups, 1);

        // Later changes go to library.json only.
        with_index(&lib, &no_log, |index| {
            let mut edited = index.by_id("d").unwrap().clone();
            edited.metadata.rating = Some(1);
            index.insert(edited);
            index.save()
        })
        .unwrap();
        assert_eq!(
            file_json(&lib)["books"]["Herbert/Dune.epub"]["metadata"]["rating"],
            1
        );
        assert_eq!(
            fs::read(legacy_path(&lib)).unwrap(),
            original,
            "still untouched"
        );
    }

    #[test]
    fn an_existing_v1_backup_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        fs::create_dir_all(lib.join(INDEX_DIR)).unwrap();
        fs::write(
            lib.join(INDEX_DIR).join(V1_BACKUP_FILE),
            b"an older v1 backup",
        )
        .unwrap();
        let a = lib.join("a.epub");
        write_legacy(
            lib,
            &json!({ a.to_string_lossy(): v1_record("a", &a, "A", "B") }),
        );
        let original = fs::read(legacy_path(lib)).unwrap();

        with_index(lib, &no_log, |_| Ok(())).unwrap();
        assert_eq!(
            fs::read(lib.join(INDEX_DIR).join(V1_BACKUP_FILE)).unwrap(),
            b"an older v1 backup"
        );
        assert_eq!(
            fs::read(lib.join(INDEX_DIR).join("books.v1.json_2.bak")).unwrap(),
            original
        );
        assert_eq!(file_json(lib)["version"], 2);
    }

    #[test]
    fn a_damaged_legacy_file_falls_back_to_its_backup_and_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        let a = lib.join("a.epub");
        fs::create_dir_all(lib.join(INDEX_DIR)).unwrap();
        let v1 = serde_json::to_vec(&json!({ a.to_string_lossy(): v1_record("a", &a, "A", "B") }))
            .unwrap();
        fs::write(legacy_backup_path(lib), &v1).unwrap();
        fs::write(legacy_path(lib), b"{ truncated").unwrap();

        assert_eq!(titles(lib), ["A"]);
        assert_eq!(
            fs::read(lib.join(INDEX_DIR).join(V1_BACKUP_FILE)).unwrap(),
            v1,
            "copy of the file migrated from"
        );
        assert_eq!(
            fs::read(legacy_path(lib)).unwrap(),
            b"{ truncated",
            "not moved aside, not rewritten"
        );
        assert_eq!(fs::read(legacy_backup_path(lib)).unwrap(), v1);
        assert_eq!(file_json(lib)["version"], 2);
    }

    #[test]
    fn a_damaged_legacy_index_without_backup_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        fs::create_dir_all(lib.join(INDEX_DIR)).unwrap();
        fs::write(legacy_path(lib), b"{ truncated").unwrap();
        let err = with_index(lib, &no_log, |index| index.save())
            .unwrap_err()
            .to_string();
        assert!(err.contains("Nothing was changed"), "{err}");
        assert!(
            !index_path(lib).exists(),
            "no empty index started over the old one"
        );
        assert_eq!(fs::read(legacy_path(lib)).unwrap(), b"{ truncated");
    }

    #[test]
    fn a_damaged_library_json_never_falls_back_to_the_older_legacy_file() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        let a = lib.join("a.epub");
        write_legacy(
            lib,
            &json!({ a.to_string_lossy(): v1_record("a", &a, "Old", "B") }),
        );
        fs::write(index_path(lib), b"{ truncated").unwrap();
        let err = with_index(lib, &no_log, |index| index.save())
            .unwrap_err()
            .to_string();
        assert!(err.contains("Nothing was changed"), "{err}");
        assert_eq!(fs::read(index_path(lib)).unwrap(), b"{ truncated");
    }

    #[test]
    fn a_lost_library_json_is_restored_from_its_backup_not_from_the_legacy_file() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        let old = lib.join("old.epub");
        write_legacy(
            lib,
            &json!({ old.to_string_lossy(): v1_record("old", &old, "Old", "B") }),
        );
        save_books(lib, &[book(lib, "a.epub")]); // migrates, then adds a
        save_books(lib, &[book(lib, "b.epub")]); // backup: old + a
        fs::remove_file(index_path(lib)).unwrap();
        assert_eq!(titles(lib), ["Old", "a.epub"]);
    }

    #[test]
    fn changes_an_old_version_makes_after_the_migration_are_not_used() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        let a = lib.join("a.epub");
        write_legacy(
            lib,
            &json!({ a.to_string_lossy(): v1_record("a", &a, "A", "B") }),
        );
        assert_eq!(titles(lib), ["A"]);

        // Something on another computer edits books.json later.
        let b = lib.join("b.epub");
        let newer = json!({
            a.to_string_lossy(): v1_record("a", &a, "A, edited elsewhere", "B"),
            b.to_string_lossy(): v1_record("b", &b, "B", "B"),
        });
        write_legacy(lib, &newer);
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(60);
        fs::OpenOptions::new()
            .write(true)
            .open(legacy_path(lib))
            .unwrap()
            .set_modified(later)
            .unwrap();

        let notes = LogLines::default();
        let titles = with_index(lib, &notes.sink(), |index| {
            let mut t: Vec<String> = index.books().map(|b| b.metadata.title.clone()).collect();
            t.sort();
            Ok(t)
        })
        .unwrap();
        assert_eq!(titles, ["A"], "library.json is the index");
        assert!(notes.contains("books.json was changed after library.json"));
    }

    #[cfg(unix)]
    #[test]
    fn a_migration_that_cannot_be_saved_changes_nothing_on_disk() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        let a = lib.join("a.epub");
        write_legacy(
            lib,
            &json!({ a.to_string_lossy(): v1_record("a", &a, "A", "B") }),
        );
        let original = fs::read(legacy_path(lib)).unwrap();
        let meta_dir = lib.join(INDEX_DIR);
        fs::set_permissions(&meta_dir, fs::Permissions::from_mode(0o555)).unwrap();

        let notes = LogLines::default();
        let result = with_index(lib, &notes.sink(), |index| {
            Ok(index.by_id("a").map(|b| b.metadata.title.clone()))
        });
        fs::set_permissions(&meta_dir, fs::Permissions::from_mode(0o755)).unwrap();

        assert_eq!(
            result.unwrap().as_deref(),
            Some("A"),
            "the library still opens"
        );
        assert!(notes.contains("could not save the updated index yet"));
        assert_eq!(
            fs::read(legacy_path(lib)).unwrap(),
            original,
            "the v1 file is untouched"
        );
        assert!(!index_path(lib).exists());
        // Writable again: the next access migrates it.
        with_index(lib, &no_log, |_| Ok(())).unwrap();
        assert_eq!(file_json(lib)["version"], 2);
        assert_eq!(fs::read(meta_dir.join(V1_BACKUP_FILE)).unwrap(), original);
        assert_eq!(fs::read(legacy_path(lib)).unwrap(), original);
    }

    #[test]
    fn cover_fields_are_never_stored() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        let mut shown = book(lib, "a.epub");
        shown.cover_path = Some(
            lib.join(".covers/thumbs/x.jpg")
                .to_string_lossy()
                .into_owned(),
        );
        shown.cover_version = Some(42);
        let mut gone = book(lib, "gone.epub");
        gone.cover_path = shown.cover_path.clone();
        gone.cover_version = Some(7);
        with_index(lib, &no_log, |index| {
            index.insert(shown.clone());
            index.insert(gone.clone());
            index.mark_missing_except(&HashSet::from(["a.epub".to_string()]), Utc::now());
            index.save()
        })
        .unwrap();
        let text = fs::read_to_string(index_path(lib)).unwrap();
        assert!(
            !text.contains("coverPath") && !text.contains("coverVersion"),
            "{text}"
        );
    }

    // ── Unavailable libraries ─────────────────────────────────────────────────

    #[test]
    fn a_missing_library_folder_is_unavailable_and_not_created() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path().join("not-mounted");
        let err = with_index(&lib, &no_log, |index| {
            index.insert(book(&lib, "a.epub"));
            index.save()
        })
        .unwrap_err();
        assert_eq!(err.code(), "LIBRARY_UNAVAILABLE");
        assert!(!lib.exists());
    }

    // ── Cache ─────────────────────────────────────────────────────────────────

    #[test]
    fn the_cache_follows_changes_made_by_another_computer() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        save_books(lib, &[book(lib, "a.epub")]);
        assert_eq!(titles(lib), ["a.epub"]);
        assert!(cache_contains(lib), "an unchanged index is cached");

        // Another computer rewrites the index (atomically, as Librarium does).
        let mut other = file_json(lib);
        other["books"]["b.epub"] = other["books"]["a.epub"].clone();
        other["books"]["b.epub"]["id"] = json!("id-b");
        other["books"]["b.epub"]["path"] = json!("b.epub");
        other["books"]["b.epub"]["metadata"]["title"] = json!("b.epub");
        fsutil::atomic_write(&index_path(lib), &serde_json::to_vec(&other).unwrap()).unwrap();
        assert_eq!(titles(lib), ["a.epub", "b.epub"]);
    }

    #[test]
    fn unsaved_or_failed_changes_never_reach_the_next_command() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        save_books(lib, &[book(lib, "a.epub")]);
        titles(lib); // cached

        let failed: AppResult<()> = with_index(lib, &no_log, |index| {
            index.insert(book(lib, "ghost.epub"));
            Err("the EPUB write failed".into())
        });
        assert!(failed.is_err());
        assert_eq!(titles(lib), ["a.epub"]);

        with_index(lib, &no_log, |index| {
            index.insert(book(lib, "unsaved.epub"));
            Ok(())
        })
        .unwrap();
        assert_eq!(titles(lib), ["a.epub"]);
    }

    #[test]
    fn snapshots_read_both_formats_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        let a = lib.join("a.epub");
        write_legacy(
            lib,
            &json!({ a.to_string_lossy(): v1_record("a", &a, "A", "B") }),
        );
        let before = fs::read(legacy_path(lib)).unwrap();
        let books = read_books_snapshot(lib);
        assert_eq!(books.len(), 1);
        assert_eq!(PathBuf::from(&books[0].path), a);
        assert_eq!(fs::read(legacy_path(lib)).unwrap(), before, "read-only");
        assert!(!index_path(lib).exists(), "a snapshot never migrates");

        save_books(lib, &[book(lib, "b.epub")]);
        assert_eq!(read_books_snapshot(lib).len(), 2, "from library.json now");
    }
}
