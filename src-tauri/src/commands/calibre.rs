//! Calibre import: the IPC layer.
//!
//! The commands here are thin wrappers that run the work on the blocking
//! pool; the work itself lives in the submodules, where it is unit-tested:
//!
//! | Module     | Responsibility |
//! |------------|----------------|
//! | [`detect`] | finding the user's Calibre library (Calibre's own settings, then the usual folders) |
//! | [`db`]     | reading `metadata.db`, read-only: the schema subset used, Calibre's order for authors and tags, a few queries whatever the library's size |
//! | [`import`] | copying books into the library with their Calibre metadata: duplicates, unique file names, batches, progress, cancellation |
//!
//! One import runs at a time ([`CalibreImportState`]). It reports progress
//! through the [`PROGRESS_EVENT`] event and stops between two books when
//! [`cancel_calibre_import`] is called.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::commands::library;
use crate::error::{AppError, AppResult};
use crate::logging::logger;
use crate::models::book::Book;

mod db;
mod detect;
mod import;

/// The event carrying [`ImportProgress`] while `import_from_calibre` runs.
pub const PROGRESS_EVENT: &str = "calibre-import-progress";

/// Progress events are sent at most this often (plus the last one): a
/// library full of duplicates is skimmed through at thousands of books per
/// second, and the UI only needs a few updates a second.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

// ── IPC types ─────────────────────────────────────────────────────────────────

/// A book of the Calibre library, as the picker lists it.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CalibreBook {
    pub id: i64,
    /// `''` when unknown (Calibre's "Unknown" placeholder).
    pub title: String,
    /// Authors in Calibre's order, joined with `" & "`; `''` when unknown.
    pub author: String,
    pub series: Option<String>,
    /// Only with a `series`.
    pub series_index: Option<f32>,
    /// Lower case (`epub`, `pdf`, `txt`…), in the order Calibre added them.
    pub available_formats: Vec<String>,
    /// The format that would be imported; `None` when the library supports
    /// none of them.
    pub best_format: Option<String>,
    pub has_cover: bool,
}

/// A Calibre book that was deliberately not imported.
#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SkippedBook {
    pub title: String,
    pub author: String,
    pub reason: SkipReason,
    /// For `alreadyInLibrary`: the file name of the library's copy.
    pub existing_filename: Option<String>,
}

/// Why a book was skipped (a code, so the UI can translate it).
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SkipReason {
    /// The library already has it: a book with the same title and authors,
    /// or a byte-identical file.
    AlreadyInLibrary,
    /// Calibre only has formats the library doesn't support (TXT, RTF…).
    UnsupportedFormat,
    /// Calibre has no file for this book (a metadata-only entry).
    NoFile,
}

/// A Calibre book that couldn't be imported.
#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FailedBook {
    pub title: String,
    /// A message for the user.
    pub reason: String,
}

/// What `import_from_calibre` did.
#[derive(Serialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    /// Saved in the library index (also when the import was cancelled).
    pub imported: Vec<Book>,
    pub skipped: Vec<SkippedBook>,
    pub failed: Vec<FailedBook>,
    /// The user cancelled ([`cancel_calibre_import`]).
    pub cancelled: bool,
    /// Why the import stopped before the end (the library became
    /// unreachable, its index can't be saved), when some books had already
    /// been imported; `None` otherwise.
    pub error: Option<String>,
    /// How many of the requested books were not looked at, because the
    /// import was cancelled or stopped.
    pub remaining: usize,
}

/// Payload of [`PROGRESS_EVENT`].
#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ImportProgress {
    /// Books finished so far (imported, skipped or failed).
    pub done: usize,
    /// Books requested (repeated ids counted once).
    pub total: usize,
    /// The book being imported now; `None` in the last event, sent when the
    /// import has ended (`done` is then the number of books looked at).
    pub current_title: Option<String>,
}

// ── One import at a time, cancellable ─────────────────────────────────────────

/// Tauri-managed state: the cancel flag of the running Calibre import, if
/// any. Only one import runs at a time: two imports into the same library
/// would race for the same file names and duplicates.
#[derive(Default)]
pub struct CalibreImportState {
    running: Mutex<Option<Arc<AtomicBool>>>,
}

/// The running import's slot in [`CalibreImportState`], freed when dropped
/// — at the end of the import, on error and on panic alike.
struct RunningImport<'a> {
    state: &'a CalibreImportState,
    cancelled: Arc<AtomicBool>,
}

impl CalibreImportState {
    fn slot(&self) -> std::sync::MutexGuard<'_, Option<Arc<AtomicBool>>> {
        self.running.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Register a new import; `CONFLICT` if one is already running.
    fn begin(&self) -> AppResult<RunningImport<'_>> {
        let mut slot = self.slot();
        if slot.is_some() {
            return Err(AppError::Conflict(
                "A Calibre import is already running".into(),
            ));
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        *slot = Some(cancelled.clone());
        Ok(RunningImport {
            state: self,
            cancelled,
        })
    }

    /// Ask the running import to stop; whether one was running.
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

impl Drop for RunningImport<'_> {
    fn drop(&mut self) {
        *self.state.slot() = None;
    }
}

/// Lets an event through if [`PROGRESS_INTERVAL`] has passed since the last
/// one it let through, or if it must not be dropped (the last one).
struct Throttle {
    last: Cell<Option<Instant>>,
}

impl Throttle {
    fn new() -> Self {
        Self {
            last: Cell::new(None),
        }
    }

    fn ready(&self, now: Instant, must_send: bool) -> bool {
        let due = self
            .last
            .get()
            .is_none_or(|last| now.duration_since(last) >= PROGRESS_INTERVAL);
        if due || must_send {
            self.last.set(Some(now));
        }
        due || must_send
    }
}

// ── Commands ──────────────────────────────────────────────────────────────────

/// The user's Calibre library: the one Calibre's settings (`global.py.json`)
/// name, else the first usual folder holding a `metadata.db`; `null` if none
/// ([`detect::detect`]). On the blocking pool: the home folder, or the
/// library, may be on a sleeping network share.
#[tauri::command]
pub async fn detect_calibre_library() -> Option<String> {
    library::blocking(|| {
        Ok(
            detect::detect(detect::Platform::current(), &detect::real_env)
                .map(|p| p.to_string_lossy().into_owned()),
        )
    })
    .await
    .ok()
    .flatten()
}

/// The books of the Calibre library at `calibre_path`, sorted by title, with
/// their formats and the one Librarium would import. A few queries whatever
/// the library's size ([`db::CalibreDb::list`]).
#[tauri::command]
pub async fn list_calibre_books(
    app: AppHandle,
    calibre_path: String,
) -> AppResult<Vec<CalibreBook>> {
    let log = logger(&app);
    library::blocking(move || list_books(Path::new(&calibre_path), &log)).await
}

/// [`list_calibre_books`].
fn list_books(calibre_root: &Path, log: crate::logging::Log) -> AppResult<Vec<CalibreBook>> {
    let db = db::CalibreDb::open(calibre_root)?;
    let mut books: Vec<CalibreBook> = db
        .list(log)?
        .into_iter()
        .map(|record| {
            let meta = import::book_metadata(&record);
            CalibreBook {
                id: record.id,
                best_format: import::best_file(&record.files).map(|f| f.format.clone()),
                available_formats: record.files.into_iter().map(|f| f.format).collect(),
                title: meta.title,
                author: meta.author,
                series: meta.series,
                series_index: meta.series_index,
                has_cover: record.has_cover,
            }
        })
        .collect();
    // Untitled books last, as in the library.
    books.sort_by_cached_key(|b| (b.title.is_empty(), b.title.to_lowercase(), b.id));
    Ok(books)
}

/// Copy the Calibre books `book_ids` into the library at `library_path`,
/// with their covers and Calibre metadata, and save them in the library
/// index ([`import::import_books`]). Progress goes out as
/// [`PROGRESS_EVENT`] events; [`cancel_calibre_import`] stops the import
/// between two books (what was imported stays imported). Duplicates are
/// found from the library index itself, never from a list the frontend
/// sends.
#[tauri::command]
pub async fn import_from_calibre(
    app: AppHandle,
    state: State<'_, CalibreImportState>,
    calibre_path: String,
    book_ids: Vec<i64>,
    library_path: String,
) -> AppResult<ImportResult> {
    let running = state.begin()?;
    let cancelled = running.cancelled.clone();
    let log = logger(&app);
    let result = library::blocking(move || {
        let throttle = Throttle::new();
        let progress = |p: ImportProgress| {
            if throttle.ready(Instant::now(), p.current_title.is_none()) {
                let _ = app.emit(PROGRESS_EVENT, p);
            }
        };
        let is_cancelled = || cancelled.load(Ordering::Relaxed);
        let hooks = import::Hooks {
            log: &log,
            progress: &progress,
            is_cancelled: &is_cancelled,
        };
        import::import_books(
            &PathBuf::from(calibre_path),
            &book_ids,
            &PathBuf::from(library_path),
            &hooks,
        )
    })
    .await;
    drop(running);
    result
}

/// Stop the running Calibre import before its next book. Returns whether an
/// import was running. The import then resolves normally, with
/// `cancelled: true`; the books it had imported are kept and saved.
#[tauri::command]
pub async fn cancel_calibre_import(state: State<'_, CalibreImportState>) -> AppResult<bool> {
    Ok(state.cancel())
}

#[cfg(test)]
mod tests {
    use super::db::fixture::{add_book, create, NewBook};
    use super::*;
    use crate::logging::no_log;

    #[test]
    fn the_list_is_sorted_by_title_with_calibre_details() {
        let dir = tempfile::tempdir().unwrap();
        let conn = create(dir.path());
        add_book(
            &conn,
            &NewBook {
                title: "notes",
                authors: &["Someone"],
                files: &[("TXT", "notes")],
                ..NewBook::default()
            },
        );
        add_book(
            &conn,
            &NewBook {
                title: "Dune",
                authors: &["Frank Herbert", "Unknown"],
                series: Some(("Dune Chronicles", 1.0)),
                files: &[("PDF", "Dune"), ("EPUB", "Dune")],
                has_cover: true,
                ..NewBook::default()
            },
        );
        add_book(
            &conn,
            &NewBook {
                title: "Anthology",
                authors: &["A", "B"],
                files: &[("AZW3", "Anthology")],
                ..NewBook::default()
            },
        );

        let books = list_books(dir.path(), &no_log).unwrap();
        let titles: Vec<_> = books.iter().map(|b| b.title.as_str()).collect();
        assert_eq!(titles, ["Anthology", "Dune", "notes"]);
        assert_eq!(books[0].author, "A & B");
        assert_eq!(
            books[1],
            CalibreBook {
                id: books[1].id,
                title: "Dune".into(),
                author: "Frank Herbert".into(),
                series: Some("Dune Chronicles".into()),
                series_index: Some(1.0),
                available_formats: vec!["pdf".into(), "epub".into()],
                best_format: Some("epub".into()),
                has_cover: true,
            }
        );
        assert_eq!(
            (books[2].best_format.as_deref(), books[2].series_index),
            (None, None)
        );
    }

    #[test]
    fn a_large_library_lists_quickly() {
        let dir = tempfile::tempdir().unwrap();
        let conn = create(dir.path());
        // 10 000 books, two authors each (all of A's links before B's, so
        // Calibre's order is A then B), every third one in a series.
        conn.execute_batch(
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 10000)
                 INSERT INTO books (id, title, sort, path) SELECT i, 'Book ' || i, 'Book ' || i, 'A/Book ' || i FROM n;
             INSERT INTO authors (id, name) VALUES (1, 'Author B'), (2, 'Author A');
             INSERT INTO books_authors_link (book, author) SELECT id, 2 FROM books;
             INSERT INTO books_authors_link (book, author) SELECT id, 1 FROM books;
             INSERT INTO series (id, name) VALUES (1, 'Saga');
             INSERT INTO books_series_link (book, series) SELECT id, 1 FROM books WHERE id % 3 = 0;
             INSERT INTO data (book, format, name) SELECT id, 'PDF', 'file' FROM books;
             INSERT INTO data (book, format, name) SELECT id, 'EPUB', 'file' FROM books;",
        )
        .unwrap();

        let started = Instant::now();
        let books = list_books(dir.path(), &no_log).unwrap();
        let took = started.elapsed();
        assert_eq!(books.len(), 10_000);
        assert!(
            books
                .iter()
                .all(|b| b.author == "Author A & Author B"
                    && b.best_format.as_deref() == Some("epub"))
        );
        assert_eq!(books.iter().filter(|b| b.series.is_some()).count(), 3_333);
        // Four queries in all (no query per book): a fraction of a second
        // even in a debug build; generous for slow CI machines.
        assert!(
            took < Duration::from_secs(5),
            "listing 10 000 books took {took:?}"
        );
    }

    #[test]
    fn one_import_at_a_time_and_cancel_reaches_it() {
        let state = CalibreImportState::default();
        assert!(!state.cancel(), "nothing to cancel");
        let running = state.begin().unwrap();
        assert_eq!(state.begin().err().unwrap().code(), "CONFLICT");
        assert!(state.cancel());
        assert!(running.cancelled.load(Ordering::Relaxed));
        drop(running);
        // The slot is free again, with a fresh flag.
        let next = state.begin().unwrap();
        assert!(!next.cancelled.load(Ordering::Relaxed));
    }

    #[test]
    fn progress_is_throttled_but_the_last_event_always_goes_out() {
        let throttle = Throttle::new();
        let t0 = Instant::now();
        assert!(throttle.ready(t0, false), "the first event");
        assert!(!throttle.ready(t0 + Duration::from_millis(10), false));
        assert!(
            throttle.ready(t0 + Duration::from_millis(20), true),
            "the last event"
        );
        assert!(!throttle.ready(t0 + Duration::from_millis(50), false));
        assert!(throttle.ready(t0 + Duration::from_millis(130), false));
    }
}
