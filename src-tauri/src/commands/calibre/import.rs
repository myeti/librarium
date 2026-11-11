//! Importing Calibre books into the library: copying their best file, their
//! cover and their Calibre metadata, and saving them in the library index.
//!
//! For each requested book, in order:
//! 1. its record is read from `metadata.db` ([`CalibreDb::record`]);
//! 2. a book the library already has is skipped ([`Holdings`]): same title
//!    and authors, or a byte-identical file;
//! 3. its best supported file is picked ([`FORMAT_PRIORITY`]); a book with
//!    only other formats (TXT, RTF, DOCX…) or no file at all is skipped;
//! 4. the file is copied atomically into the library root under its Calibre
//!    file name — or, if a *different* book already has that name,
//!    `Name_2.epub`, `Name_3.epub`… ([`fsutil::unique_path`]);
//! 5. its cover goes to the cover cache (`.covers/<id>.jpg`), and the book
//!    is added to the index with Calibre's metadata ([`book_metadata`]).
//!
//! **Batches.** The index lock is taken for one batch of books at a time
//! ([`BATCH_MAX_BOOKS`] or [`BATCH_MAX_TIME`], whichever comes first), and
//! each batch is saved before the lock is released. So the library stays
//! usable during a long import (a scan or an edit waits for one batch at
//! most), a crash loses at most one batch's index entries (their files are
//! then found by the next scan), and a scan can never see a copied file
//! before its Calibre record is in the index.
//!
//! **Cancellation** is checked before each book: the books already imported
//! stay imported (and are saved), the rest are counted in `remaining`.
//!
//! **Errors.** A problem with one book (file missing on disk, copy failed)
//! fails that book only (`failed`). A problem with the library itself (it
//! became unreachable, its index can't be saved) stops the import: as the
//! command's error if nothing had been imported yet, otherwise in
//! `ImportResult.error`, alongside what was imported before.

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::Utc;
use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;

use super::db::{CalibreDb, CalibreFile, CalibreRecord};
use super::{FailedBook, ImportProgress, ImportResult, SkipReason, SkippedBook};
use crate::commands::library::{self, index::LibraryIndex, sniff};
use crate::epub::edit::split_authors;
use crate::error::{AppError, AppResult};
use crate::fsutil;
use crate::logging::Log;
use crate::models::book::{Book, BookFormat, BookMetadata};

/// Formats Librarium can import, best first. Calibre also stores formats
/// the library can't hold (TXT, RTF, DOCX, CBR, ORIGINAL_EPUB backups…);
/// those are never picked.
pub(crate) const FORMAT_PRIORITY: &[&str] = &["epub", "azw3", "mobi", "azw", "pdf", "cbz"];

/// At most this many books per batch (see the module docs).
const BATCH_MAX_BOOKS: usize = 50;

/// At most this long per batch: copying from and to a NAS can be slow, and
/// other commands wait for the index meanwhile.
const BATCH_MAX_TIME: Duration = Duration::from_secs(5);

/// Calibre's placeholder for an unknown title or author. (Calibre writes it
/// in the language of its interface at the time; only the English one can
/// be recognised.) Unknown values are empty in Librarium.
const CALIBRE_UNKNOWN: &str = "Unknown";

/// What the import reports back while it runs, and how it learns that the
/// user cancelled. The command wires these to Tauri events and state; tests
/// pass closures.
pub(crate) struct Hooks<'a> {
    pub log: Log<'a>,
    /// Called when a book starts (`current_title` set) and once at the end
    /// (`current_title: None`).
    pub progress: &'a dyn Fn(ImportProgress),
    /// Checked before each book.
    pub is_cancelled: &'a dyn Fn() -> bool,
}

// ── From Calibre's metadata to Librarium's ────────────────────────────────────

/// The best file of `files` that the library supports, if any.
pub(crate) fn best_file(files: &[CalibreFile]) -> Option<&CalibreFile> {
    FORMAT_PRIORITY
        .iter()
        .find_map(|wanted| files.iter().find(|f| f.format == *wanted))
}

/// Calibre's 0–10 rating as Librarium's 1–5 stars: two points per star,
/// half stars (odd values) rounded **up** (7 = 3½ stars → 4; 1 = ½ star →
/// 1), so a rated book never reads as unrated. 0 (or less) means "not
/// rated". This is the rule used for `calibre:rating` in EPUB files too
/// (`epub::metadata::rating`), so a book gets the same stars whether they
/// come from Calibre's database or from its OPF.
pub(crate) fn stars(rating: f64) -> Option<u8> {
    (rating.is_finite() && rating > 0.0).then(|| (rating / 2.0).round().clamp(1.0, 5.0) as u8)
}

/// Calibre's authors as Librarium's author string: joined with `" & "`
/// (what the editor splits on when writing a book back); Calibre's
/// "Unknown" placeholder is dropped.
pub(crate) fn author_string(authors: &[String]) -> String {
    authors
        .iter()
        .map(|a| a.trim())
        .filter(|a| !a.is_empty() && *a != CALIBRE_UNKNOWN)
        .collect::<Vec<_>>()
        .join(" & ")
}

/// Librarium's metadata for a Calibre book:
/// - title and authors in Calibre's order (placeholders become empty);
/// - series, and its position only when the book has a series (Calibre
///   gives every book a `series_index` of 1);
/// - tags become categories;
/// - rating: [`stars`];
/// - language: the first one, as ISO 639-1 (`fra` → `fr`);
/// - comments: Calibre stores HTML; the text is kept, with its paragraphs;
/// - ISBN and Goodreads id (as a link) from the identifiers.
pub(crate) fn book_metadata(record: &CalibreRecord) -> BookMetadata {
    let title = record.title.trim();
    let series = record.series.clone();
    BookMetadata {
        title: if title == CALIBRE_UNKNOWN {
            String::new()
        } else {
            title.to_string()
        },
        author: author_string(&record.authors),
        subtitle: None,
        series_index: series
            .as_ref()
            .and(record.series_index)
            .filter(|i| i.is_finite())
            .map(|i| i as f32),
        series,
        language: record
            .languages
            .first()
            .map(|code| library::metadata::normalize_lang_code(code)),
        description: record
            .comments
            .as_deref()
            .map(crate::epub::strip_html)
            .filter(|d| !d.is_empty()),
        isbn: record.isbn.clone(),
        rating: record.rating.and_then(stars),
        categories: record.tags.clone(),
        goodreads_url: record
            .goodreads_id
            .as_ref()
            .map(|id| format!("https://www.goodreads.com/book/show/{id}")),
    }
}

// ── What the library already holds ────────────────────────────────────────────

/// The key two records of the same book share: the title and the set of
/// authors, compared case-insensitively, in Unicode NFC, with runs of
/// whitespace as one space, authors in any order. `None` without a title
/// (two untitled books aren't the same book).
fn identity_key(title: &str, author: &str) -> Option<String> {
    let norm = |s: &str| {
        s.nfc()
            .collect::<String>()
            .to_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    let title = norm(title);
    if title.is_empty() {
        return None;
    }
    let mut authors: Vec<String> = split_authors(author).iter().map(|a| norm(a)).collect();
    authors.sort();
    Some(format!("{title}\u{1f}{}", authors.join(" & ")))
}

/// What the library holds, to recognise a book it already has. Built from
/// the index at the start of each batch (so books added meanwhile by other
/// commands count too) and kept up to date with the books this import adds.
struct Holdings {
    /// [`identity_key`] → file name of the library's book.
    by_identity: HashMap<String, String>,
    /// File size → the library's files of that size (as indexed).
    by_size: HashMap<u64, Vec<PathBuf>>,
}

impl Holdings {
    fn of(index: &LibraryIndex) -> Self {
        let mut holdings = Self {
            by_identity: HashMap::new(),
            by_size: HashMap::new(),
        };
        for book in index.books() {
            holdings.add(book);
        }
        holdings
    }

    fn add(&mut self, book: &Book) {
        if let Some(key) = identity_key(&book.metadata.title, &book.metadata.author) {
            self.by_identity
                .entry(key)
                .or_insert_with(|| book.filename.clone());
        }
        self.by_size
            .entry(book.size_bytes)
            .or_default()
            .push(PathBuf::from(&book.path));
    }

    /// The file name of a library book with this title and these authors.
    fn same_book(&self, meta: &BookMetadata) -> Option<&str> {
        self.by_identity
            .get(&identity_key(&meta.title, &meta.author)?)
            .map(String::as_str)
    }

    /// A library file with exactly the bytes of `src` (`size` bytes long).
    /// Sizes rarely collide, so this reads files only for real candidates.
    fn same_file(&self, src: &Path, size: u64) -> Option<&Path> {
        self.by_size
            .get(&size)?
            .iter()
            .find(|candidate| fsutil::files_identical(src, candidate))
            .map(PathBuf::as_path)
    }
}

// ── The import ────────────────────────────────────────────────────────────────

/// Whether `relative` (a folder or file name from Calibre's database) stays
/// inside the folder it is joined to: plain names only, no `..`, no root.
/// The database is the user's, but nothing read from a file may make the
/// import copy from elsewhere on the disk.
fn is_plain_relative(relative: &str) -> bool {
    !relative.is_empty()
        && Path::new(relative)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

/// What became of one book.
enum Outcome {
    Imported(Box<Book>),
    Skipped(SkipReason, Option<String>),
    Failed(String),
}

/// Import the Calibre books `ids` (in this order; repeated ids once) from
/// the library at `calibre_root` into the Librarium library `library` — see
/// the module docs.
pub(crate) fn import_books(
    calibre_root: &Path,
    ids: &[i64],
    library: &Path,
    hooks: &Hooks,
) -> AppResult<ImportResult> {
    let log = hooks.log;
    let db = CalibreDb::open(calibre_root)?;
    // An unmounted share is not created here (nor anywhere).
    library::files::ensure_library_available(library)?;

    let mut seen = HashSet::new();
    let ids: Vec<i64> = ids.iter().copied().filter(|id| seen.insert(*id)).collect();
    let total = ids.len();
    log(format!(
        "[import_from_calibre] {total} book(s) from {}",
        calibre_root.display()
    ));

    let mut result = ImportResult::default();
    let mut next = 0;
    while next < total && !result.cancelled {
        let before_batch = result.imported.len();
        let batch = library::index::with_index(library, log, |index| {
            let started = Instant::now();
            let mut holdings = Holdings::of(index);
            let mut in_batch = 0;
            while next < total && in_batch < BATCH_MAX_BOOKS && started.elapsed() < BATCH_MAX_TIME {
                if (hooks.is_cancelled)() {
                    result.cancelled = true;
                    break;
                }
                let (title, author, outcome) =
                    import_one(&db, ids[next], library, &mut holdings, hooks, (next, total));
                match outcome {
                    Outcome::Imported(book) => {
                        log(format!(
                            "[import_from_calibre] imported \"{}\" → {}",
                            book.metadata.title, book.filename
                        ));
                        index.insert((*book).clone());
                        result.imported.push(*book);
                    }
                    Outcome::Skipped(reason, existing_filename) => {
                        log(format!(
                            "[import_from_calibre] skipped \"{title}\" ({reason:?})"
                        ));
                        result.skipped.push(SkippedBook {
                            title,
                            author,
                            reason,
                            existing_filename,
                        });
                    }
                    Outcome::Failed(reason) => {
                        log(format!(
                            "[import_from_calibre] WARNING — \"{title}\" not imported: {reason}"
                        ));
                        result.failed.push(FailedBook { title, reason });
                    }
                }
                next += 1;
                in_batch += 1;
            }
            if index.is_dirty() {
                index.save()?;
            }
            Ok(())
        });

        if let Err(e) = batch {
            // This batch's books are copied but not in the saved index: a
            // rescan finds their files (without Calibre's metadata).
            let unsaved = result.imported.split_off(before_batch);
            let message = if unsaved.is_empty() {
                e.to_string()
            } else {
                format!(
                    "{} book(s) were copied into the library, but the library index could not be updated \
                     (rescan the library to see them): {e}",
                    unsaved.len()
                )
            };
            log(format!(
                "[import_from_calibre] ERROR — import stopped: {message}"
            ));
            if result.imported.is_empty() {
                // Nothing is saved: the command fails, with the library's
                // own error (its code tells the UI what happened) unless
                // books were copied without being indexed.
                return Err(if unsaved.is_empty() {
                    e
                } else {
                    AppError::Io(message)
                });
            }
            result
                .failed
                .extend(unsaved.into_iter().map(|b| FailedBook {
                    title: b.metadata.title,
                    reason: message.clone(),
                }));
            result.error = Some(message);
            break;
        }
    }

    result.remaining = total - next;
    (hooks.progress)(ImportProgress {
        done: next,
        total,
        current_title: None,
    });
    log(format!(
        "[import_from_calibre] {} — {} imported, {} skipped, {} failed, {} not processed",
        if result.cancelled {
            "cancelled"
        } else if result.error.is_some() {
            "stopped"
        } else {
            "done"
        },
        result.imported.len(),
        result.skipped.len(),
        result.failed.len(),
        result.remaining
    ));
    Ok(result)
}

/// Import Calibre book `id`, the `done`-th of `total` (`position`): returns
/// its title and author (for the result lists) and what became of it.
/// Doesn't touch the index; the caller inserts an imported book.
fn import_one(
    db: &CalibreDb,
    id: i64,
    library: &Path,
    holdings: &mut Holdings,
    hooks: &Hooks,
    (done, total): (usize, usize),
) -> (String, String, Outcome) {
    let progress = |title: &str| {
        (hooks.progress)(ImportProgress {
            done,
            total,
            current_title: Some(title.to_string()),
        })
    };
    let record = match db.record(id, hooks.log) {
        Ok(Some(record)) => record,
        Ok(None) => {
            let title = format!("#{id}");
            progress(&title);
            return (
                title,
                String::new(),
                Outcome::Failed("This book is no longer in the Calibre library".into()),
            );
        }
        Err(e) => {
            let title = format!("#{id}");
            progress(&title);
            return (title, String::new(), Outcome::Failed(e.to_string()));
        }
    };
    progress(&record.title);

    let meta = book_metadata(&record);
    let (title, author) = (meta.title.clone(), meta.author.clone());
    let outcome = import_record(&record, meta, db.root(), library, holdings, hooks.log);
    if let Outcome::Imported(book) = &outcome {
        holdings.add(book);
    }
    (title, author, outcome)
}

/// Steps 2–5 of the module docs for one book.
fn import_record(
    record: &CalibreRecord,
    meta: BookMetadata,
    calibre_root: &Path,
    library: &Path,
    holdings: &Holdings,
    log: Log,
) -> Outcome {
    if let Some(existing) = holdings.same_book(&meta) {
        return Outcome::Skipped(SkipReason::AlreadyInLibrary, Some(existing.to_string()));
    }
    if record.files.is_empty() {
        return Outcome::Skipped(SkipReason::NoFile, None);
    }
    let Some(file) = best_file(&record.files) else {
        return Outcome::Skipped(SkipReason::UnsupportedFormat, None);
    };
    let file_name = file.file_name();
    if !is_plain_relative(&record.folder) || !is_plain_relative(&file_name) {
        return Outcome::Failed(format!(
            "Calibre's database gives this book an invalid location ({}/{file_name})",
            record.folder
        ));
    }
    let src = calibre_root.join(&record.folder).join(&file_name);
    let size = match std::fs::metadata(&src) {
        Ok(m) if m.is_file() => m.len(),
        Ok(_) => {
            return Outcome::Failed(format!("{file_name} in the Calibre library is not a file"))
        }
        Err(e) => {
            return Outcome::Failed(
                AppError::io(
                    format!("Could not read {file_name} in the Calibre library"),
                    &e,
                )
                .to_string(),
            )
        }
    };
    if !looks_like(&src, &file.format) {
        return Outcome::Failed(format!(
            "{file_name} in the Calibre library is not a valid {} file",
            file.format.to_uppercase()
        ));
    }
    if let Some(existing) = holdings.same_file(&src, size) {
        return Outcome::Skipped(
            SkipReason::AlreadyInLibrary,
            Some(crate::error::display_name(existing)),
        );
    }

    // A file of that name may already be in the library: if it is this very
    // file (not indexed yet), it is a duplicate; otherwise it is another
    // book, and this one gets a free name next to it.
    let name = library::files::library_file_name(&file_name);
    let wanted = library.join(&name);
    if wanted.is_file() && fsutil::files_identical(&src, &wanted) {
        return Outcome::Skipped(SkipReason::AlreadyInLibrary, Some(name));
    }
    let dest = fsutil::unique_path(library, &name);
    if dest != wanted {
        log(format!(
            "[import_from_calibre] {name} is taken by another book; importing as {}",
            crate::error::display_name(&dest)
        ));
    }
    let size_bytes = match fsutil::atomic_copy(&src, &dest) {
        Ok(bytes) => bytes,
        Err(e) => {
            return Outcome::Failed(
                AppError::io(format!("Could not copy {file_name} into the library"), &e)
                    .to_string(),
            )
        }
    };

    let id = Uuid::new_v4().to_string();
    if record.has_cover {
        copy_cover(
            &calibre_root.join(&record.folder),
            library,
            &id,
            &meta.title,
            log,
        );
    }
    Outcome::Imported(Box::new(Book {
        id,
        path: dest.to_string_lossy().into_owned(),
        filename: crate::error::display_name(&dest),
        format: BookFormat::from_extension(&file.format),
        size_bytes,
        date_added: Utc::now().to_rfc3339(),
        cover_path: None,
        cover_version: None,
        favorite: false,
        metadata: meta,
    }))
}

/// Whether the file at `path` starts like a `format` ebook should (ZIP for
/// EPUB/CBZ, `%PDF-`, `BOOKMOBI`): the same check as for any file entering
/// the library, so a damaged or mislabelled file isn't indexed as a book.
fn looks_like(path: &Path, format: &str) -> bool {
    use std::io::Read;
    let mut head = Vec::with_capacity(sniff::EBOOK_SNIFF_LEN);
    std::fs::File::open(path)
        .and_then(|f| f.take(sniff::EBOOK_SNIFF_LEN as u64).read_to_end(&mut head))
        .is_ok_and(|_| sniff::ebook_matches_extension(format, &head))
}

/// Copy Calibre's `cover.jpg` from `book_folder` into the library's cover
/// cache as `.covers/<id>.jpg`, where `ensure_cover` finds it (and makes
/// its thumbnail). Best effort: a book without its cover is still imported.
fn copy_cover(book_folder: &Path, library: &Path, id: &str, title: &str, log: Log) {
    let src = book_folder.join("cover.jpg");
    if !src.is_file() {
        return;
    }
    let covers = library::covers::covers_dir(library);
    let copied = std::fs::create_dir_all(&covers)
        .and_then(|()| fsutil::atomic_copy(&src, &covers.join(format!("{id}.jpg"))));
    if let Err(e) = copied {
        log(format!(
            "[import_from_calibre] WARNING — the cover of \"{title}\" was not copied: {e}"
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::super::db::fixture::{self, add_book, NewBook};
    use super::*;
    use crate::commands::library::index::with_index;
    use crate::commands::library::test_support::write_epub;
    use crate::logging::no_log;
    use std::cell::{Cell, RefCell};

    /// A Calibre library and an (empty) Librarium library side by side.
    struct Libraries {
        _dir: tempfile::TempDir,
        calibre: PathBuf,
        library: PathBuf,
        conn: rusqlite::Connection,
    }

    impl Libraries {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let calibre = dir.path().join("Calibre Library");
            let library = dir.path().join("Librarium");
            std::fs::create_dir_all(&library).unwrap();
            let conn = fixture::create(&calibre);
            Self {
                _dir: dir,
                calibre,
                library,
                conn,
            }
        }

        /// Add `book` to Calibre, writing each of its files: EPUBs as real
        /// EPUBs (titled `epub_title`), other formats as `content`.
        fn add(&self, book: &NewBook, content: &[u8]) -> i64 {
            let id = add_book(&self.conn, book);
            let folder = self.calibre.join(fixture::folder_of(&self.conn, id));
            std::fs::create_dir_all(&folder).unwrap();
            for (format, name) in book.files {
                let path = folder.join(format!("{name}.{}", format.to_lowercase()));
                if format.eq_ignore_ascii_case("epub") {
                    write_epub(
                        &path,
                        book.title,
                        book.authors.first().copied().unwrap_or(""),
                    );
                } else {
                    std::fs::write(&path, content).unwrap();
                }
            }
            if book.has_cover {
                std::fs::write(folder.join("cover.jpg"), [0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3])
                    .unwrap();
            }
            id
        }

        fn import(&self, ids: &[i64]) -> ImportResult {
            let hooks = Hooks {
                log: &no_log,
                progress: &|_| {},
                is_cancelled: &|| false,
            };
            import_books(&self.calibre, ids, &self.library, &hooks).unwrap()
        }

        fn indexed(&self) -> Vec<Book> {
            with_index(&self.library, &no_log, |index| {
                Ok(index.books().cloned().collect())
            })
            .unwrap()
        }
    }

    fn dune() -> NewBook<'static> {
        NewBook {
            title: "Dune",
            authors: &["Frank Herbert"],
            series: Some(("Dune Chronicles", 1.0)),
            files: &[
                ("EPUB", "Dune - Frank Herbert"),
                ("TXT", "Dune - Frank Herbert"),
            ],
            tags: &["Science Fiction", "Classics"],
            rating: Some(9),
            comments: Some("<div><p>Melange &amp; sandworms.</p><p>Arrakis.</p></div>"),
            identifiers: &[("isbn", "9780441013593"), ("goodreads", "234225")],
            languages: &["fra"],
            has_cover: true,
        }
    }

    #[test]
    fn calibre_metadata_is_mapped_and_saved() {
        let libs = Libraries::new();
        let id = libs.add(&dune(), b"");

        let result = libs.import(&[id]);
        assert_eq!(
            (
                result.imported.len(),
                result.skipped.len(),
                result.failed.len()
            ),
            (1, 0, 0)
        );
        assert!(!result.cancelled && result.error.is_none() && result.remaining == 0);

        let book = &result.imported[0];
        assert_eq!(
            book.filename, "Dune - Frank Herbert.epub",
            "EPUB preferred over TXT"
        );
        assert_eq!(book.format, BookFormat::Epub);
        let m = &book.metadata;
        assert_eq!(
            (m.title.as_str(), m.author.as_str()),
            ("Dune", "Frank Herbert")
        );
        assert_eq!(
            (m.series.as_deref(), m.series_index),
            (Some("Dune Chronicles"), Some(1.0))
        );
        assert_eq!(
            m.categories,
            ["Science Fiction", "Classics"],
            "tags become categories"
        );
        assert_eq!(m.rating, Some(5), "9/10 = 4½ stars, rounded up");
        assert_eq!(m.language.as_deref(), Some("fr"));
        assert_eq!(
            m.description.as_deref(),
            Some("Melange & sandworms.\n\nArrakis.")
        );
        assert_eq!(m.isbn.as_deref(), Some("9780441013593"));
        assert_eq!(
            m.goodreads_url.as_deref(),
            Some("https://www.goodreads.com/book/show/234225")
        );

        assert!(libs.library.join("Dune - Frank Herbert.epub").is_file());
        assert!(library::covers::covers_dir(&libs.library)
            .join(format!("{}.jpg", book.id))
            .is_file());

        // Saved: the index has the book, and a rescan keeps its id and metadata.
        assert_eq!(libs.indexed(), result.imported);
        let scanned = library::scan::scan_library_at(&libs.library, &no_log).unwrap();
        assert_eq!(scanned.len(), 1);
        assert_eq!(
            (scanned[0].id.as_str(), &scanned[0].metadata),
            (book.id.as_str(), m)
        );
    }

    #[test]
    fn several_authors_keep_calibre_order() {
        let libs = Libraries::new();
        let id = libs.add(
            &NewBook {
                title: "Good Omens",
                authors: &["Terry Pratchett", "Neil Gaiman", "Doe, Jr."],
                files: &[("EPUB", "Good Omens")],
                ..NewBook::default()
            },
            b"",
        );
        let result = libs.import(&[id]);
        assert_eq!(
            result.imported[0].metadata.author,
            "Terry Pratchett & Neil Gaiman & Doe, Jr."
        );
    }

    #[test]
    fn series_index_only_with_a_series_and_placeholders_dropped() {
        let libs = Libraries::new();
        let id = libs.add(
            &NewBook {
                title: "Unknown",
                authors: &["Unknown"],
                files: &[("PDF", "Unknown")],
                ..NewBook::default()
            },
            b"%PDF-1.4\n",
        );
        let result = libs.import(&[id]);
        let m = &result.imported[0].metadata;
        assert_eq!((m.title.as_str(), m.author.as_str()), ("", ""));
        assert_eq!(
            (m.series.as_deref(), m.series_index),
            (None, None),
            "Calibre's default 1.0 isn't a position"
        );
        assert_eq!(
            (m.rating, m.language.as_deref(), m.description.as_deref()),
            (None, None, None)
        );
    }

    #[test]
    fn ratings_map_to_stars() {
        let cases = [
            (0.0, None),
            (-1.0, None),
            (1.0, Some(1)),
            (2.0, Some(1)),
            (3.0, Some(2)),
            (6.0, Some(3)),
            (7.0, Some(4)),
            (10.0, Some(5)),
            (12.0, Some(5)),
        ];
        for (calibre, expected) in cases {
            assert_eq!(stars(calibre), expected, "Calibre rating {calibre}");
        }
        assert_eq!(stars(f64::NAN), None);
    }

    #[test]
    fn only_supported_formats_are_picked() {
        let files = |formats: &[&str]| {
            formats
                .iter()
                .map(|f| CalibreFile {
                    format: f.to_string(),
                    name: "x".into(),
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(best_file(&files(&["pdf", "epub"])).unwrap().format, "epub");
        assert_eq!(best_file(&files(&["txt", "cbz"])).unwrap().format, "cbz");
        assert_eq!(best_file(&files(&["txt", "original_epub"])), None);
    }

    #[test]
    fn unsupported_and_fileless_books_are_skipped_with_a_reason() {
        let libs = Libraries::new();
        let txt = libs.add(
            &NewBook {
                title: "Notes",
                authors: &["Someone"],
                files: &[("TXT", "Notes")],
                ..NewBook::default()
            },
            b"text",
        );
        let none = libs.add(
            &NewBook {
                title: "Wishlist",
                authors: &["Someone"],
                ..NewBook::default()
            },
            b"",
        );
        let result = libs.import(&[txt, none]);
        let reasons: Vec<_> = result
            .skipped
            .iter()
            .map(|s| (s.title.as_str(), s.reason))
            .collect();
        assert_eq!(
            reasons,
            [
                ("Notes", SkipReason::UnsupportedFormat),
                ("Wishlist", SkipReason::NoFile)
            ]
        );
        assert!(result.imported.is_empty() && result.failed.is_empty());
    }

    #[test]
    fn a_different_book_with_the_same_file_name_gets_a_unique_name() {
        let libs = Libraries::new();
        // Two different books Calibre happens to name alike.
        let a = libs.add(
            &NewBook {
                title: "Poems",
                authors: &["Emily Dickinson"],
                files: &[("PDF", "Poems")],
                ..NewBook::default()
            },
            b"%PDF-1.4 Dickinson",
        );
        let b = libs.add(
            &NewBook {
                title: "Poems",
                authors: &["John Keats"],
                files: &[("PDF", "Poems")],
                ..NewBook::default()
            },
            b"%PDF-1.4 Keats",
        );
        let result = libs.import(&[a, b]);
        assert!(
            result.skipped.is_empty(),
            "not duplicates: {:?}",
            result.skipped
        );
        let names: Vec<_> = result
            .imported
            .iter()
            .map(|b| b.filename.as_str())
            .collect();
        assert_eq!(names, ["Poems.pdf", "Poems_2.pdf"]);
        assert_eq!(
            std::fs::read(libs.library.join("Poems_2.pdf")).unwrap(),
            b"%PDF-1.4 Keats"
        );
        assert_eq!(libs.indexed().len(), 2);
    }

    #[test]
    fn real_duplicates_are_skipped() {
        let libs = Libraries::new();
        let first = libs.add(&dune(), b"");
        assert_eq!(libs.import(&[first]).imported.len(), 1);

        // Imported again: same title and author (case and spacing aside).
        let again = libs.add(
            &NewBook {
                title: "  DUNE ",
                authors: &["frank  herbert"],
                files: &[("PDF", "Dune other edition")],
                ..NewBook::default()
            },
            b"%PDF-1.4 other",
        );
        // Same bytes as a library file, under another title.
        let copy_src = libs.add(
            &NewBook {
                title: "Report",
                authors: &["A"],
                files: &[("PDF", "Report")],
                ..NewBook::default()
            },
            b"%PDF-1.4 report",
        );
        assert_eq!(libs.import(&[copy_src]).imported.len(), 1);
        let same_bytes = libs.add(
            &NewBook {
                title: "Report (scan)",
                authors: &["B"],
                files: &[("PDF", "Scan")],
                ..NewBook::default()
            },
            b"%PDF-1.4 report",
        );
        // Authors in another order are the same authors.
        let duo = libs.add(
            &NewBook {
                title: "Omens",
                authors: &["A", "B"],
                files: &[("PDF", "Omens")],
                ..NewBook::default()
            },
            b"%PDF-1.4 omens",
        );
        let duo_again = libs.add(
            &NewBook {
                title: "Omens",
                authors: &["B", "A"],
                files: &[("PDF", "Omens 2")],
                ..NewBook::default()
            },
            b"%PDF-1.4 omens 2",
        );

        let result = libs.import(&[again, same_bytes, duo, duo_again]);
        let skipped: Vec<_> = result
            .skipped
            .iter()
            .map(|s| (s.title.as_str(), s.reason, s.existing_filename.as_deref()))
            .collect();
        assert_eq!(
            skipped,
            [
                (
                    "DUNE",
                    SkipReason::AlreadyInLibrary,
                    Some("Dune - Frank Herbert.epub")
                ),
                (
                    "Report (scan)",
                    SkipReason::AlreadyInLibrary,
                    Some("Report.pdf")
                ),
                ("Omens", SkipReason::AlreadyInLibrary, Some("Omens.pdf")),
            ]
        );
        assert_eq!(result.imported.len(), 1, "only the first Omens");
        assert_eq!(libs.indexed().len(), 3, "Dune, Report, Omens");
    }

    #[test]
    fn an_unindexed_identical_file_is_a_duplicate_not_a_second_copy() {
        let libs = Libraries::new();
        let id = libs.add(
            &NewBook {
                title: "Report",
                authors: &["A"],
                files: &[("PDF", "Report")],
                ..NewBook::default()
            },
            b"%PDF-1.4 report",
        );
        // Put there by hand, never scanned.
        std::fs::write(libs.library.join("Report.pdf"), b"%PDF-1.4 report").unwrap();
        let result = libs.import(&[id]);
        assert_eq!(result.skipped[0].reason, SkipReason::AlreadyInLibrary);
        assert!(!libs.library.join("Report_2.pdf").exists());
    }

    #[test]
    fn a_missing_file_fails_that_book_only() {
        let libs = Libraries::new();
        let gone = libs.add(
            &NewBook {
                title: "Gone",
                authors: &["A"],
                files: &[("EPUB", "Gone")],
                ..NewBook::default()
            },
            b"",
        );
        std::fs::remove_file(
            libs.calibre
                .join(fixture::folder_of(&libs.conn, gone))
                .join("Gone.epub"),
        )
        .unwrap();
        let ok = libs.add(&dune(), b"");
        let result = libs.import(&[gone, ok, 9999]);
        let failed: Vec<_> = result.failed.iter().map(|f| f.title.as_str()).collect();
        assert_eq!(failed, ["Gone", "#9999"]);
        assert_eq!(result.imported.len(), 1);
    }

    #[test]
    fn locations_outside_the_calibre_library_are_refused() {
        let libs = Libraries::new();
        let id = libs.add(
            &NewBook {
                title: "Evil",
                authors: &["A"],
                files: &[("PDF", "Evil")],
                ..NewBook::default()
            },
            b"%PDF-1.4",
        );
        libs.conn
            .execute("UPDATE books SET path = '../../etc' WHERE id = ?1", [id])
            .unwrap();
        let result = libs.import(&[id]);
        assert!(result.failed[0].reason.contains("invalid location"));
        assert!(is_plain_relative("Frank Herbert/Dune (1)"));
        assert!(
            !is_plain_relative("/etc") && !is_plain_relative("a/../b") && !is_plain_relative("")
        );
    }

    #[test]
    fn progress_is_reported_for_every_book_and_at_the_end() {
        let libs = Libraries::new();
        let a = libs.add(&dune(), b"");
        let b = libs.add(
            &NewBook {
                title: "Notes",
                authors: &["Someone"],
                files: &[("TXT", "Notes")],
                ..NewBook::default()
            },
            b"x",
        );
        let events = RefCell::new(Vec::new());
        let progress =
            |p: ImportProgress| events.borrow_mut().push((p.done, p.total, p.current_title));
        let hooks = Hooks {
            log: &no_log,
            progress: &progress,
            is_cancelled: &|| false,
        };
        import_books(&libs.calibre, &[a, b, a], &libs.library, &hooks).unwrap();
        assert_eq!(
            events.into_inner(),
            [
                (0, 2, Some("Dune".to_string())),
                (1, 2, Some("Notes".to_string())),
                (2, 2, None)
            ],
            "a repeated id is imported once"
        );
    }

    #[test]
    fn cancelling_keeps_what_was_imported() {
        let libs = Libraries::new();
        let ids: Vec<i64> = (0..4)
            .map(|i| {
                let title = format!("Book {i}");
                let book = NewBook {
                    title: &title,
                    authors: &["A"],
                    files: &[("PDF", &title)],
                    ..NewBook::default()
                };
                libs.add(&book, format!("%PDF-1.4 {i}").as_bytes())
            })
            .collect();
        // The user presses Cancel while the second book is being imported.
        let checks = Cell::new(0);
        let is_cancelled = || {
            checks.set(checks.get() + 1);
            checks.get() > 2
        };
        let hooks = Hooks {
            log: &no_log,
            progress: &|_| {},
            is_cancelled: &is_cancelled,
        };
        let result = import_books(&libs.calibre, &ids, &libs.library, &hooks).unwrap();

        assert!(result.cancelled);
        assert_eq!((result.imported.len(), result.remaining), (2, 2));
        assert_eq!(
            libs.indexed().len(),
            2,
            "the books imported before the cancel are saved"
        );
        assert!(!libs.library.join("Book 2.pdf").exists());
    }

    #[test]
    fn long_imports_are_saved_in_batches() {
        let libs = Libraries::new();
        let ids: Vec<i64> = (0..BATCH_MAX_BOOKS + 3)
            .map(|i| {
                let title = format!("Book {i}");
                let book = NewBook {
                    title: &title,
                    authors: &["A"],
                    files: &[("CBZ", &title)],
                    ..NewBook::default()
                };
                libs.add(&book, format!("PK\x03\x04 {i}").as_bytes())
            })
            .collect();
        // As each book starts: how many books it follows, and how many the
        // index *on disk* holds.
        let seen = RefCell::new(Vec::new());
        let progress = |p: ImportProgress| {
            if p.current_title.is_some() {
                let on_disk = std::fs::read_to_string(library::index::index_path(&libs.library))
                    .unwrap_or_default();
                seen.borrow_mut()
                    .push((p.done, on_disk.matches("\"dateAdded\"").count()));
            }
        };
        let hooks = Hooks {
            log: &no_log,
            progress: &progress,
            is_cancelled: &|| false,
        };
        let result = import_books(&libs.calibre, &ids, &libs.library, &hooks).unwrap();
        assert_eq!(result.imported.len(), BATCH_MAX_BOOKS + 3);
        let seen = seen.into_inner();
        // The first batch (BATCH_MAX_BOOKS books, fewer on a very slow
        // machine) was saved while the import went on.
        assert!(
            seen.iter().any(|&(_, saved)| saved > 0),
            "saved during the import: {seen:?}"
        );
        assert!(seen.iter().all(|&(done, saved)| saved <= done));
        assert!(seen.windows(2).all(|w| w[0].1 <= w[1].1));
        assert_eq!(libs.indexed().len(), BATCH_MAX_BOOKS + 3);
    }

    #[test]
    fn importing_into_an_unmounted_library_creates_nothing() {
        let libs = Libraries::new();
        let id = libs.add(&dune(), b"");
        let missing = libs.library.parent().unwrap().join("Volumes").join("books");
        let hooks = Hooks {
            log: &no_log,
            progress: &|_| {},
            is_cancelled: &|| false,
        };
        let err = import_books(&libs.calibre, &[id], &missing, &hooks)
            .err()
            .unwrap();
        assert_eq!(err.code(), "LIBRARY_UNAVAILABLE");
        assert!(!libs.library.parent().unwrap().join("Volumes").exists());
    }

    #[test]
    fn identity_keys_ignore_case_spacing_and_author_order() {
        assert_eq!(
            identity_key("Good  Omens", "Neil Gaiman & Terry Pratchett"),
            identity_key("good omens", "Terry Pratchett & Neil Gaiman")
        );
        assert_ne!(
            identity_key("Poems", "Emily Dickinson"),
            identity_key("Poems", "John Keats")
        );
        assert_eq!(identity_key("  ", "Someone"), None);
        // NFC: a decomposed "é" (as macOS file names have it) is the same letter.
        assert_eq!(
            identity_key("Les Mise\u{301}rables", "Hugo"),
            identity_key("Les Misérables", "Hugo")
        );
    }
}
