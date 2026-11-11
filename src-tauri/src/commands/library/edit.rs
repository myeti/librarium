//! Changes to books already in the library: rename ([`rename_book_file`]),
//! metadata edits for one book ([`update_book_metadata`]) or several
//! ([`batch_update`]), reset ([`reset_book`]), delete
//! ([`delete_books_by_id`]) and "remove from library" ([`forget_books_by_id`]).
//!
//! Every operation looks books up by id under the index lock
//! ([`with_index`]) — paths, sizes and dates come from the index, never from
//! the client — refuses files outside the library folder
//! ([`ensure_in_library`]), and changes the file before the index, so a
//! failure leaves the two in agreement.
//!
//! What can be slow on a NAS — rewriting EPUBs for a metadata edit, moving
//! files to the trash — runs *between* two short holds of the index lock:
//! the books are looked up, the files changed without the lock, then the
//! index is updated under the lock again, after checking that each book is
//! still the one that was changed ([`edit_books`], [`delete_books_by_id`]).
//! Renames and resets only touch one file quickly and stay under the lock.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use super::covers::remove_cached_covers;
use super::epub_edit;
use super::files::{ensure_in_library, library_file_name, strip_hidden_prefix, Trash};
use super::index::{not_in_library, with_index};
use super::metadata::normalize_lang_code;
use super::scan::build_book_from_path;
use crate::error::{AppError, AppResult};
use crate::fsutil::{self, TrashOutcome};
use crate::logging::Log;
use crate::models::book::{
    BatchUpdateResult, Book, BookFormat, BookMetadata, BookMetadataPatch, DeleteResult, Failure,
    ForgetResult,
};

// ── Rename ────────────────────────────────────────────────────────────────────

/// [`rename_book`](super::rename_book). The name is sanitised for every
/// filesystem ([`fsutil::sanitize_filename`]), keeps the book's extension
/// (the format is detected from it), and gets a `_2`-style suffix when
/// another file already has it. Only the index entry's path and file name
/// change: the book keeps its id, metadata and date added.
pub(crate) fn rename_book_file(
    lib: &Path,
    id: &str,
    new_filename: &str,
    log: Log,
) -> AppResult<Book> {
    with_index(lib, log, |index| {
        let old = index.by_id(id).cloned().ok_or_else(|| not_in_library(id))?;
        let old_path = PathBuf::from(&old.path);
        ensure_in_library(lib, &old_path)?;
        if !old_path.is_file() {
            return Err(AppError::NotFound(format!(
                "{} is missing from the library folder; rescan the library",
                old.filename
            )));
        }

        let ext = old_path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let safe_name = safe_rename_name(new_filename, ext);
        let dir = old_path.parent().unwrap_or(lib);
        let target = rename_target(dir, &old_path, &safe_name);
        if target == old_path {
            return Ok(old);
        }

        // Under the file's lock: a cover being embedded right now would
        // otherwise re-create the file under its old name once done.
        fsutil::with_file_lock(&old_path, || fs::rename(&old_path, &target))
            .map_err(|e| AppError::io(format!("Could not rename {}", old.filename), &e))?;
        let book = Book {
            path: target.to_string_lossy().to_string(),
            filename: target
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            size_bytes: fs::metadata(&target)
                .map(|m| m.len())
                .unwrap_or(old.size_bytes),
            ..old.clone()
        };
        index.remove(&old.path);
        index.insert(book.clone());
        if let Err(e) = index.save() {
            // The index on disk still has the old path: put the file back so
            // the two keep matching (otherwise the next scan would see a new
            // book and drop this one's metadata).
            if let Err(undo) = fs::rename(&target, &old_path) {
                log(format!(
                    "[rename_book] could not undo the rename after a failed save: {}",
                    undo
                ));
            }
            return Err(e);
        }
        log(format!(
            "[rename_book] {} → {}",
            old.filename, book.filename
        ));
        Ok(book)
    })
}

/// The file name a rename to `requested` should use: the book's extension
/// `ext` is kept (appended unless `requested` already ends with it, in any
/// case), and the rest is made safe and visible as in [`library_file_name`].
fn safe_rename_name(requested: &str, ext: &str) -> String {
    let requested = strip_hidden_prefix(requested.trim()).trim_end();
    if ext.is_empty() {
        return library_file_name(requested);
    }
    let suffix = format!(".{ext}");
    let stem = requested
        .len()
        .checked_sub(suffix.len())
        .filter(|&i| requested.is_char_boundary(i) && requested[i..].eq_ignore_ascii_case(&suffix))
        .map_or(requested, |i| &requested[..i]);
    let ext = ext.to_lowercase();
    let budget = fsutil::MAX_FILENAME_BYTES.saturating_sub(ext.len() + 1);
    format!("{}.{}", fsutil::sanitize_stem(stem, budget), ext)
}

/// Where renaming `old` to `name` (in `dir`) should land: `dir/name`, or a
/// `_2`-style variant when another file has that name. A change of letter
/// case only (`dune.epub` → `Dune.epub`) on a case-insensitive volume names
/// the same file, and is kept as is.
fn rename_target(dir: &Path, old: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if candidate == old || (candidate.exists() && same_file(&candidate, old)) {
        return candidate;
    }
    fsutil::unique_path(dir, name)
}

/// Whether two paths name the same file on disk.
#[cfg(unix)]
fn same_file(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (fs::metadata(a), fs::metadata(b)) {
        (Ok(x), Ok(y)) => x.dev() == y.dev() && x.ino() == y.ino(),
        _ => false,
    }
}

/// Whether two paths name the same file on disk.
#[cfg(not(unix))]
fn same_file(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x.to_string_lossy().to_lowercase() == y.to_string_lossy().to_lowercase(),
        _ => false,
    }
}

// ── Metadata edits ────────────────────────────────────────────────────────────

/// Clean up edited metadata before storing it: title and author are trimmed
/// (empty means unknown; the UI shows a placeholder), blank optional fields
/// become absent (so an emptied form field clears the value), categories
/// lose blank entries, and language codes become ISO 639-1 (`fre` → `fr`).
fn normalize_metadata(metadata: &mut BookMetadata) {
    metadata.title = metadata.title.trim().to_string();
    metadata.author = metadata.author.trim().to_string();
    for field in [
        &mut metadata.subtitle,
        &mut metadata.series,
        &mut metadata.language,
        &mut metadata.description,
        &mut metadata.isbn,
        &mut metadata.goodreads_url,
    ] {
        if field.as_deref().is_some_and(|v| v.trim().is_empty()) {
            *field = None;
        }
    }
    metadata.categories.retain(|c| !c.trim().is_empty());
    metadata.language = metadata.language.as_deref().map(normalize_lang_code);
}

/// Give `book` the metadata `metadata`. For EPUBs, what changed is written
/// into the file first (cleared fields are removed from it; fields the edit
/// didn't touch stay as the file has them); if that fails, the error is
/// returned and neither the file nor the book changes. Returns the updated
/// record — storing it in the index is the caller's job.
fn apply_book_metadata(
    lib: &Path,
    book: Book,
    metadata: BookMetadata,
    log: Log,
) -> AppResult<Book> {
    let path = PathBuf::from(&book.path);
    if book.format == BookFormat::Epub {
        ensure_in_library(lib, &path)?;
        let changed =
            epub_edit::write_metadata_changes(&path, &book.metadata, &metadata).map_err(|e| {
                AppError::Other(format!(
                    "Could not write the metadata into {}: {}",
                    book.filename, e
                ))
            })?;
        if changed {
            log(format!("[metadata] written into {}", book.filename));
        }
    }
    let size_bytes = fs::metadata(&path)
        .map(|m| m.len())
        .unwrap_or(book.size_bytes);
    Ok(Book {
        metadata,
        size_bytes,
        ..book
    })
}

/// [`update_metadata`](super::update_metadata). The book is looked up by
/// id — the path, size and date added come from the index, never from the
/// client. For EPUBs the metadata is written into the file first, and the
/// index only changes if that succeeded ([`edit_books`]).
pub(crate) fn update_book_metadata(
    lib: &Path,
    id: &str,
    mut metadata: BookMetadata,
    log: Log,
) -> AppResult<Book> {
    normalize_metadata(&mut metadata);
    log(format!(
        "[update_metadata] id={} title=\"{}\" author=\"{}\"",
        id, metadata.title, metadata.author
    ));
    let mut edited = edit_books(
        lib,
        std::slice::from_ref(&id.to_string()),
        &|_, _| metadata.clone(),
        log,
    )?;
    if let Some((_, error)) = edited.failures.pop() {
        return Err(error);
    }
    let book = edited.books.pop().ok_or_else(|| not_in_library(id))?;
    log(format!(
        "[update_metadata] saved — \"{}\" ({:?})",
        book.metadata.title, book.format
    ));
    Ok(book)
}

/// [`batch_update_metadata`](super::batch_update_metadata): merge `patch`
/// into **each** book's own metadata (absent fields stay as they are), then
/// give each book listed in `series_indexes` its own volume number (the
/// books of a series numbered in one edit), write EPUBs, save the index
/// once ([`edit_books`]). A book that fails (missing, unwritable EPUB) is
/// reported in `failures` and left unchanged; the others are still updated.
pub(crate) fn batch_update(
    lib: &Path,
    ids: &[String],
    patch: &BookMetadataPatch,
    series_indexes: &HashMap<String, f32>,
    log: Log,
) -> AppResult<BatchUpdateResult> {
    log(format!(
        "[batch_update_metadata] {} book(s) — patch={:?}, {} volume number(s)",
        ids.len(),
        patch,
        series_indexes.len()
    ));
    let patched = |id: &str, current: &BookMetadata| {
        let mut metadata = current.clone();
        patch.apply_to(&mut metadata);
        if let Some(index) = series_indexes
            .get(id)
            .filter(|i| i.is_finite() && **i >= 0.0)
        {
            metadata.series_index = Some(*index);
        }
        normalize_metadata(&mut metadata);
        metadata
    };
    let edited = edit_books(lib, ids, &patched, log)?;
    for book in &edited.books {
        log(format!(
            "[batch_update_metadata] patched id={} ({:?})",
            book.id, book.format
        ));
    }
    for (id, error) in &edited.failures {
        log(format!(
            "[batch_update_metadata] failed id={}: {}",
            id, error
        ));
    }
    let result = BatchUpdateResult {
        books: edited.books,
        failures: edited
            .failures
            .into_iter()
            .map(|(id, error)| Failure {
                id,
                error: error.to_string(),
            })
            .collect(),
    };
    log(format!(
        "[batch_update_metadata] done — {} book(s) updated, {} failed",
        result.books.len(),
        result.failures.len()
    ));
    Ok(result)
}

/// What [`edit_books`] did, in the order of the requested ids.
struct Edited {
    books: Vec<Book>,
    failures: Vec<(String, AppError)>,
}

/// Give each book of `ids` (repeated ids count once) the metadata `edit`
/// makes of its current metadata (`edit` gets the book's id too), without
/// holding the index lock while
/// EPUBs are rewritten — a batch edit of a hundred books on a NAS takes
/// minutes, and covers, scans and other edits would all wait for it:
///
/// 1. under the index lock, briefly: the books as they are now;
/// 2. without it: each EPUB is rewritten ([`apply_book_metadata`], under the
///    file's own lock);
/// 3. under the index lock again: each new record is stored — if the book
///    is still what step 1 saw. A book changed meanwhile (renamed, edited,
///    its cover replaced) gets `edit` applied to its record *as it is now*,
///    written again under the lock, so neither change is lost; a book
///    removed meanwhile is reported as a failure. The index is saved once.
///
/// Fails as a whole only when the index can't be read or saved.
fn edit_books(
    lib: &Path,
    ids: &[String],
    edit: &dyn Fn(&str, &BookMetadata) -> BookMetadata,
    log: Log,
) -> AppResult<Edited> {
    // Results are sorted back into the order of `ids` at the end.
    let mut order = std::collections::HashMap::new();
    for id in ids {
        let next = order.len();
        order.entry(id.as_str()).or_insert(next);
    }
    let mut failures: Vec<(String, AppError)> = Vec::new();

    // 1. The books, as the index has them now.
    let current: Vec<Book> = with_index(lib, log, |index| {
        Ok(order
            .keys()
            .filter_map(|id| index.by_id(id).cloned())
            .collect())
    })?;
    for id in order.keys() {
        if !current.iter().any(|b| b.id == *id) {
            failures.push((id.to_string(), not_in_library(id)));
        }
    }

    // 2. Rewrite the files, without the index lock.
    let mut written: Vec<(Book, Book)> = Vec::new();
    for old in current {
        let metadata = edit(&old.id, &old.metadata);
        match apply_book_metadata(lib, old.clone(), metadata, log) {
            Ok(new) => {
                #[cfg(test)]
                super::test_support::hooks::run(
                    super::test_support::hooks::EDIT_WRITTEN,
                    Path::new(&new.path),
                );
                written.push((old, new));
            }
            Err(error) => failures.push((old.id, error)),
        }
    }

    // 3. Store what was written, re-checked against the index as it is now.
    let mut books: Vec<Book> = Vec::new();
    if !written.is_empty() {
        let (stored, late_failures) = with_index(lib, log, |index| {
            let mut stored = Vec::new();
            let mut late_failures = Vec::new();
            for (old, new) in written {
                match index.by_id(&old.id).cloned() {
                    Some(now) if now == old => {
                        index.insert(new.clone());
                        stored.push(new);
                    }
                    Some(now) => {
                        log(format!(
                            "[metadata] {} changed during the edit; applying the edit to its current record",
                            now.filename
                        ));
                        let metadata = edit(&now.id, &now.metadata);
                        match apply_book_metadata(lib, now, metadata, log) {
                            Ok(book) => {
                                index.insert(book.clone());
                                stored.push(book);
                            }
                            Err(error) => late_failures.push((old.id, error)),
                        }
                    }
                    None => {
                        let error = not_in_library(&old.id);
                        late_failures.push((old.id, error));
                    }
                }
            }
            if !stored.is_empty() {
                index.save()?;
            }
            Ok((stored, late_failures))
        })?;
        books = stored;
        failures.extend(late_failures);
    }

    let position = |id: &str| order.get(id).copied().unwrap_or(usize::MAX);
    books.sort_by_key(|b| position(&b.id));
    failures.sort_by_key(|(id, _)| position(id));
    Ok(Edited { books, failures })
}

// ── Reset ─────────────────────────────────────────────────────────────────────

/// [`reset_book_data`](super::reset_book_data). The book keeps its id,
/// date added and star — it is the same book, only the metadata Librarium
/// added is dropped — and the fresh record is saved.
pub(crate) fn reset_book(lib: &Path, id: &str, log: Log) -> AppResult<Book> {
    log(format!("[reset_book_data] id={}", id));
    with_index(lib, log, |index| {
        let old = index.by_id(id).cloned().ok_or_else(|| not_in_library(id))?;
        let fresh = build_book_from_path(Path::new(&old.path))
            .map_err(|e| e.into_app_error(&old.filename))?;
        let book = Book {
            id: old.id.clone(),
            date_added: old.date_added.clone(),
            cover_path: None,
            favorite: old.favorite,
            ..fresh
        };
        remove_cached_covers(lib, id)
            .map_err(|e| AppError::io("Could not remove the cached cover", &e))?;
        index.remove(&old.path);
        index.insert(book.clone());
        index.save()?;
        log(format!(
            "[reset_book_data] reparsed — \"{}\" by {}",
            book.metadata.title, book.metadata.author
        ));
        Ok(book)
    })
}

// ── Favorites ─────────────────────────────────────────────────────────────────

/// [`set_favorite`](super::set_favorite): star (or unstar) the books `ids`.
/// Only the index changes, never the files, so it is quick and works on
/// every format. Returns the books as they are now, in the order asked
/// (repeated ids once); an id that isn't in the library is left out. The
/// index is saved only when a book changed.
pub(crate) fn set_favorite(
    lib: &Path,
    ids: &[String],
    favorite: bool,
    log: Log,
) -> AppResult<Vec<Book>> {
    with_index(lib, log, |index| {
        let mut books = Vec::new();
        let mut changed = 0;
        let mut seen = HashSet::new();
        for id in ids {
            if !seen.insert(id.as_str()) {
                continue;
            }
            let Some(current) = index.by_id(id) else {
                log(format!("[set_favorite] {id} isn't in the library"));
                continue;
            };
            if current.favorite == favorite {
                books.push(current.clone());
                continue;
            }
            let book = Book {
                favorite,
                ..current.clone()
            };
            index.insert(book.clone());
            books.push(book);
            changed += 1;
        }
        if changed > 0 {
            index.save()?;
        }
        log(format!(
            "[set_favorite] {} {changed} book(s)",
            if favorite { "starred" } else { "unstarred" }
        ));
        Ok(books)
    })
}

// ── Delete ────────────────────────────────────────────────────────────────────

/// [`delete_books`](super::delete_books). Paths come from the index (never
/// from the client) and must lie inside the library. Each book is handled on
/// its own: a failure is reported and the others are still deleted (the old
/// code stopped at the first error without saving, so already-deleted files
/// stayed in the index).
///
/// The files are trashed *outside* the index lock — asking the OS trash,
/// then maybe copying into the library's trash folder, can take a while on a
/// NAS, and every library command would wait — in three steps:
/// 1. under the lock, briefly: which file each book has;
/// 2. without it: each file goes to the trash (under the file's own lock,
///    like a rename);
/// 3. under the lock again: the trashed books are removed from the index —
///    only if their record still names the file that was trashed. A book
///    renamed or converted meanwhile is reported as a failure (`CONFLICT`)
///    and kept; one removed meanwhile counts as deleted. The index is saved
///    once, then the cached covers are removed.
pub(crate) fn delete_books_by_id(
    lib: &Path,
    ids: &[String],
    trash: Trash,
    log: Log,
) -> AppResult<DeleteResult> {
    log(format!("[delete_books] deleting {} book(s)", ids.len()));
    let mut result = DeleteResult {
        deleted: Vec::new(),
        failures: Vec::new(),
        library_trash: None,
    };
    let fail = |result: &mut DeleteResult, id: &str, error: AppError| {
        log(format!("[delete_books] failed id={}: {}", id, error));
        result.failures.push(Failure {
            id: id.to_string(),
            error: error.to_string(),
        });
    };

    // 1. The books' files, as the index has them now.
    let planned: Vec<Result<Book, (String, AppError)>> = with_index(lib, log, |index| {
        let mut seen = HashSet::new();
        Ok(ids
            .iter()
            .filter(|id| seen.insert(id.as_str()))
            .map(|id| {
                index
                    .by_id(id)
                    .cloned()
                    .ok_or_else(|| (id.clone(), not_in_library(id)))
            })
            .collect())
    })?;

    // 2. Trash the files, without the index lock — only inside the library
    //    (index paths are absolute and may predate a move of the library).
    let mut trashed: Vec<(Book, Option<TrashOutcome>)> = Vec::new();
    for planned in planned {
        let done = planned.and_then(|book| {
            ensure_in_library(lib, Path::new(&book.path))
                .and_then(|()| trash_file(&book, trash, log))
                .map(|outcome| (book.clone(), outcome))
                .map_err(|e| (book.id, e))
        });
        match done {
            Ok(done) => trashed.push(done),
            Err((id, error)) => fail(&mut result, &id, error),
        }
    }

    // 3. Forget the books whose file was trashed.
    if !trashed.is_empty() {
        let outcome = with_index(lib, log, |index| {
            let mut deleted = Vec::new();
            let mut conflicts = Vec::new();
            for (book, outcome) in trashed {
                match index.by_id(&book.id) {
                    Some(now) if now.path == book.path => {
                        index.remove(&book.path);
                        deleted.push((book.id, outcome));
                    }
                    Some(now) => {
                        let error = AppError::Conflict(format!(
                            "{} was renamed or converted while it was being deleted; it is still in \
                             the library (now {}). Try again.",
                            book.filename, now.filename
                        ));
                        conflicts.push((book.id, error));
                    }
                    // Deleted or removed meanwhile: gone either way.
                    None => deleted.push((book.id, outcome)),
                }
            }
            if !deleted.is_empty() {
                index.save().map_err(|e| {
                    AppError::Io(format!(
                        "{} book(s) were moved to the trash, but the library index could not be \
                         updated (rescan the library): {}",
                        deleted.len(),
                        e
                    ))
                })?;
            }
            Ok((deleted, conflicts))
        });
        let (deleted, conflicts) = outcome?;
        for (id, error) in conflicts {
            fail(&mut result, &id, error);
        }
        for (id, outcome) in deleted {
            if let Some(TrashOutcome::LibraryTrash(dest)) = outcome {
                result.library_trash = dest.parent().map(|p| p.to_string_lossy().into_owned());
            }
            if let Err(e) = remove_cached_covers(lib, &id) {
                log(format!(
                    "[delete_books] could not remove the cached cover of {}: {}",
                    id, e
                ));
            }
            result.deleted.push(id);
        }
    }

    // In the order of the request.
    let position = |id: &str| ids.iter().position(|i| i == id).unwrap_or(usize::MAX);
    result.deleted.sort_by_key(|id| position(id));
    result.failures.sort_by_key(|f| position(&f.id));
    log(format!(
        "[delete_books] done — {} deleted, {} failed",
        result.deleted.len(),
        result.failures.len()
    ));
    Ok(result)
}

// ── Remove from library (keep the file) ──────────────────────────────────────

/// [`forget_books`](super::forget_books): remove books from the library
/// without touching their files. Each file's place is added to the index's
/// `ignored` list, so the next scan doesn't add it back; importing the file
/// again brings it back (as a new book). Cached covers are removed once the
/// index is saved. Unknown ids are reported in `failures`; the others are
/// still forgotten.
pub(crate) fn forget_books_by_id(lib: &Path, ids: &[String], log: Log) -> AppResult<ForgetResult> {
    log(format!(
        "[forget_books] removing {} book(s) from the library (files kept)",
        ids.len()
    ));
    with_index(lib, log, |index| {
        let mut result = ForgetResult {
            forgotten: Vec::new(),
            failures: Vec::new(),
        };
        let mut seen = HashSet::new();
        for id in ids {
            if !seen.insert(id.as_str()) {
                continue;
            }
            match index.forget(id) {
                Some(book) => {
                    log(format!("[forget_books] forgot {} (file kept)", book.path));
                    result.forgotten.push(id.clone());
                }
                None => result.failures.push(Failure {
                    id: id.clone(),
                    error: not_in_library(id).to_string(),
                }),
            }
        }
        if !result.forgotten.is_empty() {
            index.save()?;
            for id in &result.forgotten {
                if let Err(e) = remove_cached_covers(lib, id) {
                    log(format!(
                        "[forget_books] could not remove the cached cover of {}: {}",
                        id, e
                    ));
                }
            }
        }
        log(format!(
            "[forget_books] done — {} removed from the library, {} failed",
            result.forgotten.len(),
            result.failures.len()
        ));
        Ok(result)
    })
}

/// Move one book's file to the trash (step 2 of [`delete_books_by_id`]; no
/// index lock). `Ok(None)` when the file was already gone from disk (the
/// book is then just removed from the index).
fn trash_file(book: &Book, trash: Trash, log: Log) -> AppResult<Option<TrashOutcome>> {
    let path = PathBuf::from(&book.path);
    // Under the file's lock, like rename (see `rename_book_file`) and EPUB
    // rewrites: neither can re-create the file under us.
    fsutil::with_file_lock(&path, || match fs::symlink_metadata(&path) {
        Ok(_) => {
            let outcome = trash(&path).map_err(|e| {
                AppError::io(format!("Could not move {} to the trash", book.filename), &e)
            })?;
            log(format!(
                "[delete_books] trashed {} ({:?})",
                book.path, outcome
            ));
            Ok(Some(outcome))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            log(format!(
                "[delete_books] file not on disk (index-only): {}",
                book.path
            ));
            Ok(None)
        }
        Err(e) => Err(AppError::io(
            format!("Could not check {}", book.filename),
            &e,
        )),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::library::covers::cover_cache_path;
    use crate::commands::library::sniff::ImageKind;
    use crate::commands::library::test_support::{
        index_len, indexed, opf_of, scan, test_trash, write, write_epub,
    };
    use crate::logging::no_log;

    // ── Rename ────────────────────────────────────────────────────────────────

    #[test]
    fn rename_keeps_id_metadata_and_date_added() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        write_epub(&lib.join("dune.epub"), "Dune", "Frank Herbert");
        let book = scan(lib).remove("dune.epub").unwrap();
        // Index-only metadata (not in the file) must survive the rename too.
        let mut meta = book.metadata.clone();
        meta.rating = Some(5);
        update_book_metadata(lib, &book.id, meta, &no_log).unwrap();

        let renamed =
            rename_book_file(lib, &book.id, "Frank Herbert - Dune: Book 1", &no_log).unwrap();
        assert_eq!(renamed.filename, "Frank Herbert - Dune_ Book 1.epub");
        assert_eq!(renamed.id, book.id);
        assert_eq!(renamed.date_added, book.date_added);
        assert_eq!(renamed.metadata.title, "Dune");
        assert_eq!(renamed.metadata.rating, Some(5));
        assert!(!lib.join("dune.epub").exists());
        assert!(Path::new(&renamed.path).is_file());

        let stored = indexed(lib, &book.id).unwrap();
        assert_eq!(stored.path, renamed.path);
        assert_eq!(stored.metadata.rating, Some(5));
        assert_eq!(index_len(lib), 1);
        // A rescan sees the same book, not a new one.
        assert_eq!(scan(lib)[&renamed.filename].id, book.id);
    }

    #[test]
    fn rename_avoids_collisions_and_path_tricks() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        write_epub(&lib.join("a.epub"), "A", "X");
        write_epub(&lib.join("b.epub"), "B", "X");
        let books = scan(lib);

        let renamed = rename_book_file(lib, &books["a.epub"].id, "b.epub", &no_log).unwrap();
        assert_eq!(renamed.filename, "b_2.epub");
        let renamed = rename_book_file(lib, &books["a.epub"].id, "../../escape", &no_log).unwrap();
        assert_eq!(Path::new(&renamed.path).parent().unwrap(), lib);
        assert_eq!(
            renamed.filename, "_.._escape.epub",
            "no path, and not a hidden file"
        );
        assert!(rename_book_file(lib, "unknown-id", "x", &no_log).is_err());
    }

    #[test]
    fn rename_to_the_same_name_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        write_epub(&lib.join("Dune.epub"), "Dune", "Frank Herbert");
        let book = scan(lib).remove("Dune.epub").unwrap();
        let same = rename_book_file(lib, &book.id, "Dune.epub", &no_log).unwrap();
        assert_eq!(same.path, book.path);
        // A case-only change is a real rename, not a "Dune_2" copy.
        let cased = rename_book_file(lib, &book.id, "DUNE", &no_log).unwrap();
        assert_eq!(cased.filename, "DUNE.epub");
    }

    #[test]
    fn safe_rename_names() {
        assert_eq!(safe_rename_name("Dune", "epub"), "Dune.epub");
        assert_eq!(safe_rename_name("Dune.EPUB", "epub"), "Dune.epub");
        assert_eq!(safe_rename_name("Mr. Smith", "pdf"), "Mr. Smith.pdf");
        assert_eq!(
            safe_rename_name("Dune.pdf", "epub"),
            "Dune.pdf.epub",
            "the format follows the file"
        );
        assert_eq!(safe_rename_name("  a/b  ", "mobi"), "a_b.mobi");
        assert_eq!(
            safe_rename_name(".hidden", "epub"),
            "hidden.epub",
            "scans skip hidden files"
        );
        assert_eq!(library_file_name("..Dune.epub"), "Dune.epub");
    }

    // ── Metadata (0.1, 0.11) ──────────────────────────────────────────────────

    #[test]
    fn a_batch_edit_numbers_each_book_of_a_series_in_the_chosen_order() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        write_epub(&lib.join("a.epub"), "Dune", "Frank Herbert");
        write_epub(&lib.join("b.epub"), "Dune Messiah", "Frank Herbert");
        write_epub(&lib.join("c.epub"), "Children of Dune", "Frank Herbert");
        let books = scan(lib);
        let id = |f: &str| books[f].id.clone();
        let patch = BookMetadataPatch {
            series: Some("Dune".into()),
            series_index: Some(9.0),
            ..Default::default()
        };
        // The user's order: Messiah first. "c" isn't numbered: the patch's
        // volume applies; a nonsense number is ignored.
        let indexes = HashMap::from([
            (id("b.epub"), 1.0),
            (id("a.epub"), 2.0),
            (id("c.epub"), f32::NAN),
        ]);
        let ids = vec![id("a.epub"), id("b.epub"), id("c.epub")];
        let result = batch_update(lib, &ids, &patch, &indexes, &no_log).unwrap();
        assert!(result.failures.is_empty(), "{:?}", result.failures);

        let volume = |f: &str| indexed(lib, &id(f)).unwrap().metadata.series_index;
        assert_eq!(volume("b.epub"), Some(1.0));
        assert_eq!(volume("a.epub"), Some(2.0));
        assert_eq!(volume("c.epub"), Some(9.0));
        // Written into the files too, not only the index.
        let opf = crate::epub::EpubArchive::open(&lib.join("b.epub")).unwrap();
        assert_eq!(opf.metadata().series_index, Some(1.0));
    }

    #[test]
    fn batch_edit_merges_the_patch_into_each_books_own_metadata() {
        // Each book keeps its own title, author, ISBN…: only the patch is
        // applied, never one book's metadata onto the others.
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        write_epub(&lib.join("dune.epub"), "Dune", "Frank Herbert");
        write_epub(&lib.join("messiah.epub"), "Dune Messiah", "Frank Herbert");
        write_epub(&lib.join("other.epub"), "Neuromancer", "William Gibson");
        let books = scan(lib);
        let (dune, messiah) = (&books["dune.epub"], &books["messiah.epub"]);
        let mut with_isbn = dune.metadata.clone();
        with_isbn.isbn = Some("9780441013593".into());
        update_book_metadata(lib, &dune.id, with_isbn, &no_log).unwrap();

        let patch = BookMetadataPatch {
            series: Some("Dune Chronicles".into()),
            language: Some("fre".into()),
            ..Default::default()
        };
        let ids = vec![dune.id.clone(), messiah.id.clone()];
        let result = batch_update(lib, &ids, &patch, &HashMap::new(), &no_log).unwrap();
        assert!(result.failures.is_empty(), "{:?}", result.failures);
        assert_eq!(result.books.len(), 2);

        let dune_after = indexed(lib, &dune.id).unwrap();
        let messiah_after = indexed(lib, &messiah.id).unwrap();
        assert_eq!(dune_after.metadata.title, "Dune");
        assert_eq!(dune_after.metadata.isbn.as_deref(), Some("9780441013593"));
        assert_eq!(messiah_after.metadata.title, "Dune Messiah");
        assert_eq!(
            messiah_after.metadata.isbn, None,
            "book #1's ISBN must not leak"
        );
        for book in [&dune_after, &messiah_after] {
            assert_eq!(book.metadata.series.as_deref(), Some("Dune Chronicles"));
            assert_eq!(
                book.metadata.language.as_deref(),
                Some("fr"),
                "language normalised"
            );
            assert_eq!(book.date_added, books[&book.filename].date_added);
        }
        // The files got their own titles plus the patched series.
        let opf = opf_of(&lib.join("messiah.epub"));
        assert!(opf.contains("<dc:title>Dune Messiah</dc:title>"), "{opf}");
        assert!(opf.contains("content=\"Dune Chronicles\""));
        // Books outside the selection are untouched.
        assert_eq!(
            indexed(lib, &books["other.epub"].id)
                .unwrap()
                .metadata
                .series,
            None
        );
    }

    #[test]
    fn batch_edit_reports_failures_and_updates_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        write_epub(&lib.join("good.epub"), "Good", "X");
        write_epub(&lib.join("broken.epub"), "Broken", "X");
        let books = scan(lib);
        // The file became unreadable after it was indexed.
        fs::write(lib.join("broken.epub"), b"not a zip any more").unwrap();

        let patch = BookMetadataPatch {
            author: Some("Y".into()),
            ..Default::default()
        };
        let ids = vec![
            books["good.epub"].id.clone(),
            books["broken.epub"].id.clone(),
            "ghost".into(),
        ];
        let result = batch_update(lib, &ids, &patch, &HashMap::new(), &no_log).unwrap();
        assert_eq!(result.books.len(), 1);
        let failed: Vec<_> = result.failures.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(failed, [books["broken.epub"].id.as_str(), "ghost"]);
        assert_eq!(
            indexed(lib, &books["good.epub"].id)
                .unwrap()
                .metadata
                .author,
            "Y"
        );
        assert_eq!(
            indexed(lib, &books["broken.epub"].id)
                .unwrap()
                .metadata
                .author,
            "X"
        );
    }

    #[test]
    fn update_metadata_writes_the_file_then_the_index() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        write_epub(&lib.join("dune.epub"), "Dune", "Frank Herbert");
        let book = scan(lib).remove("dune.epub").unwrap();

        let mut meta = book.metadata.clone();
        meta.title = "Dune (Deluxe)".into();
        meta.isbn = Some("9780593099322".into());
        meta.series = Some("  ".into()); // an emptied form field
        let updated = update_book_metadata(lib, &book.id, meta, &no_log).unwrap();
        assert_eq!(updated.path, book.path);
        assert_eq!(updated.metadata.series, None);
        assert_eq!(
            indexed(lib, &book.id).unwrap().metadata.title,
            "Dune (Deluxe)"
        );
        let opf = opf_of(&lib.join("dune.epub"));
        assert!(opf.contains("<dc:title>Dune (Deluxe)</dc:title>"));
        // 0.11 / 1.3: the ISBN is an extra identifier; the unique one is kept.
        assert!(
            opf.contains(r#"<dc:identifier opf:scheme="ISBN">9780593099322</dc:identifier>"#),
            "{opf}"
        );
        assert!(
            opf.contains(r#"<dc:identifier id="BookId">urn:uuid:librarium-test</dc:identifier>"#)
        );
    }

    #[test]
    fn update_metadata_leaves_the_index_alone_when_the_file_write_fails() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        write_epub(&lib.join("dune.epub"), "Dune", "Frank Herbert");
        let book = scan(lib).remove("dune.epub").unwrap();
        fs::write(lib.join("dune.epub"), b"corrupted").unwrap();

        let mut meta = book.metadata.clone();
        meta.title = "Changed".into();
        assert!(update_book_metadata(lib, &book.id, meta, &no_log).is_err());
        assert_eq!(indexed(lib, &book.id).unwrap().metadata.title, "Dune");
        assert_eq!(fs::read(lib.join("dune.epub")).unwrap(), b"corrupted");
    }

    // ── Delete ────────────────────────────────────────────────────────────────

    #[test]
    fn delete_moves_files_to_the_trash_and_continues_past_failures() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        write_epub(&lib.join("a.epub"), "A", "X");
        write_epub(&lib.join("b.epub"), "B", "X");
        write_epub(&lib.join("keep.epub"), "Keep", "X");
        let books = scan(lib);
        let (a, b) = (&books["a.epub"], &books["b.epub"]);
        write(&cover_cache_path(lib, &a.id, ImageKind::Jpeg), b"cover");
        // b's file is already gone: it is just forgotten.
        fs::remove_file(&b.path).unwrap();

        let ids = vec![a.id.clone(), "ghost".to_string(), b.id.clone()];
        let result = delete_books_by_id(lib, &ids, &test_trash(lib), &no_log).unwrap();
        assert_eq!(result.deleted, [a.id.clone(), b.id.clone()]);
        assert_eq!(result.failures.len(), 1);
        assert_eq!(result.failures[0].id, "ghost");

        let trash_dir = PathBuf::from(result.library_trash.expect("fallback trash used"));
        assert!(trash_dir.starts_with(lib.join(fsutil::LIBRARY_TRASH_DIR)));
        assert!(
            trash_dir.join("a.epub").is_file(),
            "the file is recoverable"
        );
        assert!(!lib.join("a.epub").exists());
        assert!(!cover_cache_path(lib, &a.id, ImageKind::Jpeg).exists());
        assert!(indexed(lib, &a.id).is_none() && indexed(lib, &b.id).is_none());
        assert!(indexed(lib, &books["keep.epub"].id).is_some());
    }

    #[test]
    fn delete_never_reaches_files_outside_the_library() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path().join("lib");
        fs::create_dir_all(lib.join(".librarium")).unwrap();
        let outside = dir.path().join("outside.epub");
        write_epub(&outside, "Outside", "X");
        // A hand-edited index pointing outside the library: the entry can't
        // be resolved to a path, so it isn't a book of this library at all.
        let entry = serde_json::json!({
            "id": "evil", "path": "../outside.epub", "filename": "outside.epub", "format": "epub",
            "sizeBytes": 1, "dateAdded": "2026-01-01T00:00:00Z", "metadata": { "title": "X", "author": "" },
        });
        let index_json = serde_json::json!({ "version": 2, "libraryId": "l", "books": { "../outside.epub": entry } });
        fs::write(
            crate::commands::library::index::index_path(&lib),
            index_json.to_string(),
        )
        .unwrap();

        let result =
            delete_books_by_id(&lib, &["evil".to_string()], &test_trash(&lib), &no_log).unwrap();
        assert!(result.deleted.is_empty());
        assert!(result.failures[0].error.contains("not in the library"));
        assert!(outside.exists());
    }

    #[cfg(unix)]
    #[test]
    fn delete_refuses_a_symlink_out_of_the_library() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path().join("lib");
        let elsewhere = dir.path().join("elsewhere");
        fs::create_dir_all(&lib).unwrap();
        fs::create_dir_all(&elsewhere).unwrap();
        write_epub(&elsewhere.join("b.epub"), "B", "X");
        std::os::unix::fs::symlink(&elsewhere, lib.join("link")).unwrap();
        // Scans don't follow links, but the index may still name a path
        // through one (made by hand, or by another tool).
        let book = build_book_from_path(&lib.join("link").join("b.epub")).unwrap();
        with_index(&lib, &no_log, |index| {
            index.insert(book.clone());
            index.save()
        })
        .unwrap();

        let result = delete_books_by_id(
            &lib,
            std::slice::from_ref(&book.id),
            &test_trash(&lib),
            &no_log,
        )
        .unwrap();
        assert!(result.deleted.is_empty());
        assert!(result.failures[0].error.contains("outside the library"));
        assert!(elsewhere.join("b.epub").exists());
    }

    // ── Remove from library ───────────────────────────────────────────────────

    #[test]
    fn forget_keeps_the_files_and_scans_skip_them() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        write_epub(&lib.join("a.epub"), "A", "X");
        write_epub(&lib.join("b.epub"), "B", "X");
        let books = scan(lib);
        let a = &books["a.epub"];
        write(&cover_cache_path(lib, &a.id, ImageKind::Jpeg), b"cover");

        let ids = vec![a.id.clone(), "ghost".to_string(), a.id.clone()];
        let result = forget_books_by_id(lib, &ids, &no_log).unwrap();
        assert_eq!(result.forgotten, std::slice::from_ref(&a.id));
        assert_eq!(result.failures.len(), 1);
        assert_eq!(result.failures[0].id, "ghost");

        assert!(lib.join("a.epub").is_file(), "the file is never touched");
        assert!(!cover_cache_path(lib, &a.id, ImageKind::Jpeg).exists());
        assert!(indexed(lib, &a.id).is_none());
        let rescanned = scan(lib);
        assert_eq!(
            rescanned.keys().collect::<Vec<_>>(),
            ["b.epub"],
            "not re-added by the next scan"
        );
        assert_eq!(rescanned["b.epub"].id, books["b.epub"].id);
    }

    // ── Reset ─────────────────────────────────────────────────────────────────

    #[test]
    fn reset_keeps_the_id_and_saves_the_fresh_record() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        write(&lib.join("notes.pdf"), b"%PDF-1.4 not really a pdf");
        let book = scan(lib).remove("notes.pdf").unwrap();
        let mut meta = book.metadata.clone();
        meta.title = "Edited".into();
        meta.rating = Some(3);
        update_book_metadata(lib, &book.id, meta, &no_log).unwrap();
        write(&cover_cache_path(lib, &book.id, ImageKind::Png), b"cover");

        let reset = reset_book(lib, &book.id, &no_log).unwrap();
        assert_eq!(reset.id, book.id);
        assert_eq!(reset.date_added, book.date_added);
        assert_eq!(reset.metadata.title, "notes");
        assert_eq!(reset.metadata.rating, None);
        assert_eq!(indexed(lib, &book.id).unwrap().metadata.title, "notes");
        assert_eq!(index_len(lib), 1);
        assert!(!cover_cache_path(lib, &book.id, ImageKind::Png).exists());
    }

    // ── Favorites ─────────────────────────────────────────────────────────────

    #[test]
    fn a_star_is_saved_in_the_index_and_survives_edits_renames_and_resets() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        write_epub(&lib.join("dune.epub"), "Dune", "Frank Herbert");
        write(&lib.join("notes.pdf"), b"%PDF-1.4 not really a pdf");
        let books = scan(lib);
        let (dune, notes) = (&books["dune.epub"], &books["notes.pdf"]);
        assert!(!dune.favorite);

        let ids = vec![
            dune.id.clone(),
            "nope".into(),
            dune.id.clone(),
            notes.id.clone(),
        ];
        let starred = set_favorite(lib, &ids, true, &no_log).unwrap();
        assert_eq!(
            starred.iter().map(|b| b.id.as_str()).collect::<Vec<_>>(),
            [dune.id.as_str(), notes.id.as_str()]
        );
        assert!(starred.iter().all(|b| b.favorite));
        assert!(indexed(lib, &dune.id).unwrap().favorite);
        // Only in the index: the file is as it was.
        assert!(!opf_of(&lib.join("dune.epub")).contains("favorite"));

        let mut meta = dune.metadata.clone();
        meta.title = "Dune (edited)".into();
        assert!(
            update_book_metadata(lib, &dune.id, meta, &no_log)
                .unwrap()
                .favorite
        );
        assert!(
            rename_book_file(lib, &dune.id, "Dune 1", &no_log)
                .unwrap()
                .favorite
        );
        assert!(reset_book(lib, &notes.id, &no_log).unwrap().favorite);
        // And a rescan.
        assert!(scan(lib)["Dune 1.epub"].favorite);

        let unstarred = set_favorite(lib, std::slice::from_ref(&notes.id), false, &no_log).unwrap();
        assert!(!unstarred[0].favorite);
        assert!(!indexed(lib, &notes.id).unwrap().favorite);
    }

    #[test]
    fn an_unstarred_book_has_no_favorite_field_in_the_index() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        write(&lib.join("notes.pdf"), b"%PDF-1.4 not really a pdf");
        let book = scan(lib).remove("notes.pdf").unwrap();
        let index = || fs::read_to_string(lib.join(".librarium/library.json")).unwrap();
        assert!(!index().contains("favorite"));
        set_favorite(lib, std::slice::from_ref(&book.id), true, &no_log).unwrap();
        assert!(index().contains("\"favorite\": true"), "{}", index());
        set_favorite(lib, std::slice::from_ref(&book.id), false, &no_log).unwrap();
        assert!(!index().contains("favorite"));
    }

    // ── Slow work outside the index lock ──────────────────────────────────────

    use crate::commands::library::index::{find_book, is_index_locked};
    use crate::commands::library::test_support::{hooks, within};

    /// The OS trash may take a while (a Finder dialog can make it wait): it
    /// runs without the index lock, so the library keeps answering meanwhile.
    #[test]
    fn delete_trashes_files_without_holding_the_index() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path().to_path_buf();
        write_epub(&lib.join("a.epub"), "A", "X");
        write_epub(&lib.join("b.epub"), "B", "X");
        let books = scan(&lib);
        let (a, b) = (books["a.epub"].clone(), books["b.epub"].clone());

        let trash = |path: &Path| {
            assert!(!is_index_locked(&lib), "the index is locked while trashing");
            let (other_lib, other) = (lib.clone(), b.id.clone());
            within(20, move || find_book(&other_lib, &other, &no_log).unwrap());
            test_trash(&lib)(path)
        };
        let result =
            delete_books_by_id(&lib, std::slice::from_ref(&a.id), &trash, &no_log).unwrap();
        assert_eq!(result.deleted, std::slice::from_ref(&a.id));
        assert!(indexed(&lib, &a.id).is_none());
        assert!(indexed(&lib, &b.id).is_some());
    }

    /// A book renamed (or converted) between the lookup and the trash keeps
    /// its record: the delete reports a conflict instead of forgetting a book
    /// whose current file wasn't the one trashed.
    #[test]
    fn a_book_renamed_while_it_is_deleted_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path().to_path_buf();
        write_epub(&lib.join("a.epub"), "A", "X");
        let a = scan(&lib).remove("a.epub").unwrap();

        let trash = |path: &Path| {
            // What a rename running meanwhile leaves behind.
            fs::copy(path, lib.join("renamed.epub")).unwrap();
            with_index(&lib, &no_log, |index| {
                let mut moved = index.by_id(&a.id).unwrap().clone();
                moved.path = lib.join("renamed.epub").to_string_lossy().into_owned();
                moved.filename = "renamed.epub".into();
                index.insert(moved);
                index.save()
            })
            .unwrap();
            test_trash(&lib)(path)
        };
        let result =
            delete_books_by_id(&lib, std::slice::from_ref(&a.id), &trash, &no_log).unwrap();
        assert!(result.deleted.is_empty());
        assert_eq!(result.failures.len(), 1);
        assert!(
            result.failures[0].error.contains("renamed or converted"),
            "{}",
            result.failures[0].error
        );
        assert_eq!(indexed(&lib, &a.id).unwrap().filename, "renamed.epub");
    }

    /// A batch edit rewrites the EPUBs without the index lock; a book whose
    /// record changed meanwhile (another edit) gets the patch on top of that
    /// change — neither is lost.
    #[test]
    fn a_batch_edit_writes_files_without_the_index_and_keeps_concurrent_changes() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path().to_path_buf();
        write_epub(&lib.join("dune.epub"), "Dune", "Frank Herbert");
        write_epub(&lib.join("messiah.epub"), "Dune Messiah", "Frank Herbert");
        let books = scan(&lib);
        let (dune, messiah) = (books["dune.epub"].clone(), books["messiah.epub"].clone());

        let _hook = hooks::on(hooks::EDIT_WRITTEN, &lib, {
            let (lib, dune_id) = (lib.clone(), dune.id.clone());
            move |path| {
                assert!(
                    !is_index_locked(&lib),
                    "the index is locked while an EPUB is rewritten"
                );
                if path.ends_with("dune.epub") {
                    // Another command rates the book meanwhile.
                    let (lib, id) = (lib.clone(), dune_id.clone());
                    within(20, move || {
                        with_index(&lib, &no_log, |index| {
                            let mut rated = index.by_id(&id).unwrap().clone();
                            rated.metadata.rating = Some(4);
                            index.insert(rated);
                            index.save()
                        })
                        .unwrap()
                    });
                }
            }
        });
        let patch = BookMetadataPatch {
            series: Some("Dune Chronicles".into()),
            ..Default::default()
        };
        let ids = vec![dune.id.clone(), messiah.id.clone()];
        let result = batch_update(&lib, &ids, &patch, &HashMap::new(), &no_log).unwrap();
        assert!(result.failures.is_empty(), "{:?}", result.failures);
        assert_eq!(
            result
                .books
                .iter()
                .map(|b| b.id.as_str())
                .collect::<Vec<_>>(),
            [dune.id.as_str(), messiah.id.as_str()],
            "in the order asked"
        );
        let dune_after = indexed(&lib, &dune.id).unwrap();
        assert_eq!(
            dune_after.metadata.rating,
            Some(4),
            "the concurrent change is kept"
        );
        assert_eq!(
            dune_after.metadata.series.as_deref(),
            Some("Dune Chronicles")
        );
        assert!(opf_of(&lib.join("dune.epub")).contains("Dune Chronicles"));
        assert_eq!(
            indexed(&lib, &messiah.id)
                .unwrap()
                .metadata
                .series
                .as_deref(),
            Some("Dune Chronicles")
        );
    }
}
