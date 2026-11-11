//! Bringing files into the library: copying dropped or picked files
//! ([`import_files`]) and saving downloads ([`download_to_temp`], then
//! [`import_download`]).
//!
//! Both recognise a re-import: a file can reach the library twice — the
//! Windows drag-drop event may deliver the same payload more than once for a
//! single gesture, and the same book may be dropped from two different
//! folders. Comparing bytes tells a re-import (skip it, return the existing
//! book) from a different book that merely shares a filename (keep both,
//! under a suffixed name).
//!
//! Neither holds the library index lock while it walks the library, copies
//! or reads files — on a NAS that is minutes during which every cover and
//! edit would wait. Imports wait for each other instead
//! ([`with_import_lock`]), stage their copies in hidden temp files, and only
//! take the index lock to move them to their final names and index them.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use uuid::Uuid;

use super::failed::{FailedFiles, Fingerprint};
use super::files::{ensure_library_available, make_readable};
use super::index::{self, with_index, LibraryIndex};
use super::scan::{
    build_book_from_file, index_new_file, index_read_file, is_supported, record_failure,
    walk_library, NewFile, ReadFailure,
};
use super::sniff;
use crate::error::{display_name, AppError, AppResult};
use crate::fsutil;
use crate::logging::Log;
use crate::models::book::Book;

// ── Re-import detection ───────────────────────────────────────────────────────

/// Index every supported file already in the library by byte size. Sizes rarely
/// collide, so this keeps the content comparison below to a handful of reads
/// even for a large library.
fn library_size_index(lib_dir: &Path) -> HashMap<u64, Vec<PathBuf>> {
    let mut index: HashMap<u64, Vec<PathBuf>> = HashMap::new();
    for entry in walk_library(lib_dir)
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file() && is_supported(e.path()))
    {
        if let Ok(len) = entry.metadata().map(|m| m.len()) {
            index
                .entry(len)
                .or_default()
                .push(entry.path().to_path_buf());
        }
    }
    index
}

/// Where an incoming file belongs inside the library.
enum ImportDest {
    /// The library already holds a byte-identical file at this path — nothing
    /// to copy.
    AlreadyPresent(PathBuf),
    /// The file should be copied to this path.
    CopyTo(PathBuf),
}

/// Decide where `src` should land in `lib_dir`. Files keep their original
/// filename; a UUID suffix is appended only when that name is taken by a
/// *different* book.
fn resolve_import_dest(
    src: &Path,
    lib_dir: &Path,
    index: &HashMap<u64, Vec<PathBuf>>,
) -> ImportDest {
    let name = src
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    resolve_import_dest_named(src, &name, lib_dir, index)
}

/// [`resolve_import_dest`] for a file whose library name differs from its
/// current one (a download sitting in a temp file).
fn resolve_import_dest_named(
    src: &Path,
    name: &str,
    lib_dir: &Path,
    index: &HashMap<u64, Vec<PathBuf>>,
) -> ImportDest {
    if let Some(existing) = fs::metadata(src)
        .ok()
        .and_then(|m| index.get(&m.len()))
        .and_then(|candidates| candidates.iter().find(|c| fsutil::files_identical(src, c)))
    {
        return ImportDest::AlreadyPresent(existing.clone());
    }

    ImportDest::CopyTo(free_dest(&lib_dir.join(name)))
}

/// `dest`, or — when a file already has that name — the same name with a
/// UUID suffix (`Dune_<uuid>.epub`), for a different book that merely shares
/// a file name.
fn free_dest(dest: &Path) -> PathBuf {
    if !dest.exists() {
        return dest.to_path_buf();
    }
    let dir = dest.parent().unwrap_or(Path::new("."));
    let stem = dest.file_stem().and_then(|s| s.to_str()).unwrap_or("book");
    let ext = dest.extension().and_then(|e| e.to_str()).unwrap_or("");
    dir.join(format!("{}_{}.{}", stem, Uuid::new_v4(), ext))
}

/// The stored book for `path` (a file already in the library), indexed
/// first when the index has no entry for it yet — re-linked to a missing
/// book with the same name and size, or parsed ([`index_new_file`]). Keeps
/// IDs stable so a re-import never invents a second book. A file the user
/// had removed from the library (ignored) is brought back.
///
/// The file is already in the library, so a failure to read it is handled
/// as in a scan: recorded in `failed` when permanent ([`record_failure`]),
/// and a file that already failed in this version isn't opened again.
fn book_at_path(
    index: &mut LibraryIndex,
    failed: &mut FailedFiles,
    path: &Path,
    label: &str,
    log: Log,
) -> AppResult<Book> {
    if let Some(book) = index.get(&path.to_string_lossy()) {
        return Ok(book.clone());
    }
    let name = display_name(path);
    let key = index.key_of(path).unwrap_or_default();
    if let Some(entry) = failed.get(&key) {
        if Fingerprint::read(path).is_ok_and(|fp| failed.is_known_failure(&key, fp)) {
            return Err(AppError::InvalidInput(format!(
                "{name} can't be added to the library: {}",
                entry.error
            )));
        }
    }
    match index_new_file(index, path) {
        Ok((book, _)) => {
            failed.clear(&key);
            Ok(book)
        }
        Err(failure) => {
            record_failure(failed, &key, path, &failure, label, log);
            Err(failure.into_app_error(&name))
        }
    }
}

/// Put a copy staged by this import (`temp`, read as `read`) into the library
/// as `dest` — or under a suffixed name if `dest` was taken since it was
/// chosen — and index it (re-linked to a missing book with the same name and
/// size, or added as read). Under the index lock: the final rename is quick,
/// and nothing else that names files in the library (renames, conversions)
/// can take the name in between.
///
/// A copy that can't be read as a book is removed again (the original is
/// untouched) — unless the failure may be temporary, in which case it stays
/// for the next scan to retry.
fn place_copy(
    index: &mut LibraryIndex,
    temp: tempfile::TempPath,
    dest: &Path,
    read: Result<Book, ReadFailure>,
    log: Log,
) -> AppResult<(Book, NewFile)> {
    let dest = free_dest(dest);
    let name = display_name(&dest);
    temp.persist(&dest)
        .map_err(|e| AppError::io(format!("Could not save {name}"), &e.error))?;
    index_read_file(index, &dest, Some(read)).map_err(|failure| {
        if failure.is_permanent() {
            match fs::remove_file(&dest) {
                Ok(()) => log(format!("[import] {name} can't be read as a book ({failure}); its copy was removed")),
                Err(e) => log(format!("[import] WARNING — {name} can't be read as a book ({failure}), and its copy could not be removed: {e}")),
            }
        }
        failure.into_app_error(&name)
    })
}

/// Copy `src` into a new hidden temp file in `dir` (where it is to be
/// imported, so keeping it is a rename), flushed to disk and readable by the
/// other accounts sharing the library. Deleted when dropped. Hidden, so a
/// scan running meanwhile doesn't pick it up.
fn stage_copy(src: &Path, dir: &Path) -> std::io::Result<(tempfile::TempPath, u64)> {
    let mut tmp = tempfile::Builder::new()
        .prefix(".librarium-import-")
        .suffix(".part")
        .tempfile_in(dir)?;
    let copied = std::io::copy(&mut fs::File::open(src)?, tmp.as_file_mut())?;
    fsutil::sync_file(tmp.as_file())?;
    let path = tmp.into_temp_path();
    make_readable(&path);
    Ok((path, copied))
}

/// Run `f` while no other import into `library` (files, a download) runs in
/// this process.
///
/// Imports are serialised among themselves — rather than under the index
/// lock, which their walk of the library and their copies would hold for
/// minutes on a NAS — so each one sees the files the previous one added: the
/// same payload delivered twice (the Windows drag-drop event) is recognised
/// as a re-import instead of being copied twice. Lock order: this lock, then
/// the index lock.
fn with_import_lock<T>(library: &Path, f: impl FnOnce() -> T) -> T {
    let library = fs::canonicalize(library).unwrap_or_else(|_| library.to_path_buf());
    fsutil::with_file_lock(&library.join(index::INDEX_DIR).join(".import"), f)
}

// ── Importing files ───────────────────────────────────────────────────────────

/// One file of an import, prepared without the index lock ([`import_files`]).
enum Staged {
    /// The library already holds a byte-identical file at this path.
    Existing(PathBuf),
    /// The same bytes as an earlier file of this import (its position).
    SameAs(usize),
    /// Copied into a hidden temp file next to `dest`, and read.
    Copied {
        temp: tempfile::TempPath,
        dest: PathBuf,
        read: Box<Result<Book, ReadFailure>>,
    },
}

/// [`import_books`](super::import_books): copy each of `paths` into
/// `lib_dir` and index it. Missing, repeated and unsupported paths are
/// skipped (and logged); a file the library already holds is not copied
/// again — its existing book is returned instead (and stops being ignored if
/// the user had removed it from the library). Copies are atomic: a book
/// only appears under its final name once fully written, so an interrupted
/// import never leaves a truncated file for the next scan to index.
///
/// A file that can't be read as a book (a parser panics, or its content
/// doesn't match its extension) is not imported: its copy is removed again
/// and the failure is logged; the other files are still imported. (A file
/// that was already in the library is recorded in `failed.json` instead, as
/// a scan would.)
///
/// The slow part — walking the library for re-imports, copying (to a NAS),
/// reading each book — runs without the index lock (imports wait for each
/// other instead, [`with_import_lock`]); the copies are then moved to their
/// final names and indexed under the lock, in one short step.
///
/// The library folder must exist (`LIBRARY_UNAVAILABLE` otherwise): an
/// import never creates it, so books can't land in an empty local folder
/// while the NAS is unmounted.
pub(crate) fn import_files(lib_dir: &Path, paths: &[String], log: Log) -> AppResult<Vec<Book>> {
    log(format!(
        "[import_books] start — {} file(s) to import",
        paths.len()
    ));
    ensure_library_available(lib_dir)?;

    with_import_lock(lib_dir, || {
        // 1. Without the index lock: find re-imports, copy, read.
        let mut size_index = library_size_index(lib_dir);
        // Staged copies by temp path, for files repeated within this import.
        let mut staged_at: HashMap<PathBuf, usize> = HashMap::new();
        let mut staged: Vec<Staged> = Vec::new();
        // The same path can appear more than once in a single payload.
        let mut seen_sources = HashSet::new();

        for src_path in paths {
            if !seen_sources.insert(src_path.clone()) {
                log(format!(
                    "[import_books] skip — repeated path in payload: {}",
                    src_path
                ));
                continue;
            }
            let src = PathBuf::from(src_path);
            if !src.exists() {
                log(format!("[import_books] skip — not found: {}", src_path));
                continue;
            }
            if !is_supported(&src) {
                log(format!(
                    "[import_books] skip — unsupported format: {}",
                    src_path
                ));
                continue;
            }

            let dest = match resolve_import_dest(&src, lib_dir, &size_index) {
                ImportDest::AlreadyPresent(existing) => {
                    staged.push(match staged_at.get(&existing) {
                        Some(&earlier) => Staged::SameAs(earlier),
                        None => Staged::Existing(existing),
                    });
                    continue;
                }
                ImportDest::CopyTo(dest) => dest,
            };

            log(format!(
                "[import_books] copy {} → {}",
                src_path,
                dest.display()
            ));
            let dir = dest.parent().unwrap_or(lib_dir);
            match stage_copy(&src, dir) {
                Ok((temp, bytes)) => {
                    log(format!("[import_books] copied {} bytes", bytes));
                    let read = build_book_from_file(&temp, &dest);
                    // So the same book dropped twice in one batch is
                    // recognised as a re-import too.
                    size_index
                        .entry(bytes)
                        .or_default()
                        .push(temp.to_path_buf());
                    staged_at.insert(temp.to_path_buf(), staged.len());
                    staged.push(Staged::Copied {
                        temp,
                        dest,
                        read: Box::new(read),
                    });
                }
                Err(e) => log(format!("[import_books] copy failed: {}", e)),
            }
        }

        // 2. Under the index lock: put the copies in place and index them.
        with_index(lib_dir, log, |index| {
            let mut failed = FailedFiles::load(lib_dir, log);
            let mut imported = Vec::new();
            let mut results: Vec<Option<Book>> = Vec::with_capacity(staged.len());
            for item in staged {
                let book = match item {
                    Staged::Existing(existing) => {
                        log(format!(
                            "[import_books] already in library — {}",
                            existing.display()
                        ));
                        book_at_path(index, &mut failed, &existing, "import_books", log)
                            .map_err(|e| log(format!("[import_books] skip — {e}")))
                            .ok()
                    }
                    Staged::SameAs(earlier) => {
                        log("[import_books] already in this import — the same file twice".into());
                        results.get(earlier).cloned().flatten()
                    }
                    Staged::Copied { temp, dest, read } => {
                        match place_copy(index, temp, &dest, *read, log) {
                            Ok((book, how)) => {
                                let how = if how == NewFile::Relinked {
                                    "re-linked to a missing book"
                                } else {
                                    "imported"
                                };
                                log(format!(
                                    "[import_books] {how} — \"{}\" ({:?}) id={}",
                                    book.metadata.title, book.format, book.id
                                ));
                                Some(book)
                            }
                            Err(e) => {
                                log(format!("[import_books] skip — {e}"));
                                None
                            }
                        }
                    }
                };
                imported.extend(book.clone());
                results.push(book);
            }

            if !imported.is_empty() {
                index.save()?;
            }
            failed.save_or_log(log);
            log(format!(
                "[import_books] done — {} book(s) imported",
                imported.len()
            ));
            Ok(imported)
        })
    })
}

// ── Importing downloads ───────────────────────────────────────────────────────

/// Download `url` into a uniquely named temp file in `<lib>/.librarium/`
/// (same volume as the library, so the final move is a rename), bounded by
/// [`crate::net::BOOK_LIMITS`] and fsynced ([`crate::net::download_to_temp`]).
/// The temp file is deleted when the returned path is dropped, unless
/// [`import_download`] keeps it.
///
/// Generic over the sink (rather than a [`Log`]) because it is held across
/// `await`s, so it must be `Sync` for the command's future to be `Send`.
pub(crate) async fn download_to_temp(
    lib: &Path,
    url: &str,
    log: &(impl Fn(String) + Sync),
) -> AppResult<tempfile::TempPath> {
    log("[download_and_import] fetching…".into());
    let temp_dir = lib.join(index::INDEX_DIR);
    let download = crate::net::download_to_temp(url, &temp_dir, crate::net::BOOK_LIMITS).await?;
    log(format!(
        "[download_and_import] downloaded {} bytes",
        download.bytes
    ));
    Ok(download.path)
}

/// Import a downloaded file (`temp`, deleted unless imported) into the
/// library as `name`. The content must match the extension's magic bytes —
/// a server answering with an HTML error page must not become a "book".
/// Uses the same collision and re-import handling as [`import_files`], and
/// like it only takes the index lock to put the file in place.
pub(crate) fn import_download(
    lib: &Path,
    temp: tempfile::TempPath,
    name: &str,
    log: Log,
) -> AppResult<Book> {
    let ext = Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    let mut head = vec![0u8; sniff::EBOOK_SNIFF_LEN];
    let mut file =
        fs::File::open(&temp).map_err(|e| AppError::io("Could not read the download", &e))?;
    let n = fsutil::fill(&mut file, &mut head)
        .map_err(|e| AppError::io("Could not read the download", &e))?;
    drop(file);
    if !sniff::ebook_matches_extension(ext, &head[..n]) {
        return Err(AppError::Network(format!(
            "The download is not a valid {} file (the server may have returned an error page).",
            ext.to_uppercase()
        )));
    }
    ensure_library_available(lib)?;

    with_import_lock(lib, || {
        let size_index = library_size_index(lib);
        let book = match resolve_import_dest_named(&temp, name, lib, &size_index) {
            ImportDest::AlreadyPresent(existing) => {
                log(format!(
                    "[download_and_import] already in library — {}",
                    existing.display()
                ));
                with_index(lib, log, |index| {
                    let mut failed = FailedFiles::load(lib, log);
                    let found =
                        book_at_path(index, &mut failed, &existing, "download_and_import", log);
                    failed.save_or_log(log);
                    let book = found?;
                    if index.is_dirty() {
                        index.save()?;
                    }
                    Ok(book)
                })?
            }
            ImportDest::CopyTo(dest) => {
                log(format!(
                    "[download_and_import] saving to {}",
                    dest.display()
                ));
                // Read without the index lock; put in place under it.
                let read = build_book_from_file(&temp, &dest);
                with_index(lib, log, |index| {
                    let (book, _) = place_copy(index, temp, &dest, read, log)?;
                    index.save()?;
                    Ok(book)
                })?
            }
        };
        log(format!(
            "[download_and_import] imported — \"{}\" by {} ({:?})",
            book.metadata.title, book.metadata.author, book.format
        ));
        Ok(book)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::library::test_support::{
        index_len, indexed, scan, temp_dir, write, write_epub,
    };
    use crate::logging::no_log;

    // ── Re-import detection ───────────────────────────────────────────────────

    #[test]
    fn re_importing_the_same_file_does_not_copy_it_again() {
        let lib = temp_dir();
        let src_dir = temp_dir();
        let src = src_dir.join("book.epub");
        write(&src, b"ebook bytes");

        // First import: nothing in the library yet, so the file is copied
        // under its own name.
        let index = library_size_index(&lib);
        let dest = match resolve_import_dest(&src, &lib, &index) {
            ImportDest::CopyTo(dest) => dest,
            ImportDest::AlreadyPresent(p) => panic!("unexpected re-import of {}", p.display()),
        };
        assert_eq!(dest, lib.join("book.epub"));
        fs::copy(&src, &dest).unwrap();

        // Second import of the same file (Windows can repeat a drag-drop
        // event): it must not produce a second copy.
        let index = library_size_index(&lib);
        match resolve_import_dest(&src, &lib, &index) {
            ImportDest::AlreadyPresent(existing) => assert_eq!(existing, dest),
            ImportDest::CopyTo(p) => panic!("would have duplicated as {}", p.display()),
        }

        fs::remove_dir_all(&lib).unwrap();
        fs::remove_dir_all(&src_dir).unwrap();
    }

    #[test]
    fn same_filename_different_book_is_kept_under_a_suffixed_name() {
        let lib = temp_dir();
        let src_dir = temp_dir();
        let src = src_dir.join("book.epub");
        write(&src, b"second book");
        write(&lib.join("book.epub"), b"first book!"); // same length, different bytes

        let index = library_size_index(&lib);
        match resolve_import_dest(&src, &lib, &index) {
            ImportDest::CopyTo(dest) => {
                assert_ne!(dest, lib.join("book.epub"));
                let name = dest.file_name().unwrap().to_str().unwrap();
                assert!(
                    name.starts_with("book_") && name.ends_with(".epub"),
                    "got {}",
                    name
                );
            }
            ImportDest::AlreadyPresent(p) => {
                panic!("wrongly treated as re-import of {}", p.display())
            }
        }

        fs::remove_dir_all(&lib).unwrap();
        fs::remove_dir_all(&src_dir).unwrap();
    }

    #[test]
    fn size_index_covers_subdirectories_and_skips_hidden_ones() {
        let lib = temp_dir();
        write(&lib.join("a.epub"), b"12345");
        write(&lib.join("nested").join("b.epub"), b"12345");
        write(&lib.join(".librarium").join("library.json"), b"12345");
        write(&lib.join("notes.txt"), b"12345");

        let index = library_size_index(&lib);
        let at_size = index.get(&5).cloned().unwrap_or_default();
        assert_eq!(at_size.len(), 2, "indexed: {:?}", at_size);
        assert!(at_size.contains(&lib.join("a.epub")));
        assert!(at_size.contains(&lib.join("nested").join("b.epub")));

        fs::remove_dir_all(&lib).unwrap();
    }

    // ── Import ────────────────────────────────────────────────────────────────

    #[test]
    fn imported_books_are_saved_and_a_reimport_returns_the_same_book() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path().join("lib");
        let src = dir.path().join("Dune.epub");
        write_epub(&src, "Dune", "Frank Herbert");
        let paths = vec![src.to_string_lossy().to_string()];

        fs::create_dir_all(&lib).unwrap();
        let first = import_files(&lib, &paths, &no_log).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(
            fs::read(lib.join("Dune.epub")).unwrap(),
            fs::read(&src).unwrap()
        );
        assert!(indexed(&lib, &first[0].id).is_some());

        let again = import_files(&lib, &paths, &no_log).unwrap();
        assert_eq!(again[0].id, first[0].id);
        assert_eq!(index_len(&lib), 1);
    }

    #[test]
    fn importing_into_an_unmounted_library_creates_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path().join("Volumes").join("books").join("Alice");
        let src = dir.path().join("Dune.epub");
        write_epub(&src, "Dune", "Frank Herbert");
        let err = import_files(&lib, &[src.to_string_lossy().to_string()], &no_log).unwrap_err();
        assert_eq!(err.code(), "LIBRARY_UNAVAILABLE");
        assert!(!dir.path().join("Volumes").exists());
    }

    #[test]
    fn a_file_that_cannot_be_read_is_not_imported_and_leaves_no_copy() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path().join("lib");
        fs::create_dir_all(&lib).unwrap();
        let good = dir.path().join("Dune.epub");
        write_epub(&good, "Dune", "Frank Herbert");
        let fake = dir.path().join("Fake.epub");
        write(&fake, b"<html>not a book</html>");
        let crashing = dir
            .path()
            .join(format!("{}.epub", crate::error::PANIC_TEST_MARKER));
        write_epub(&crashing, "Crash", "X");
        let paths: Vec<String> = [&good, &fake, &crashing]
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();

        let imported = import_files(&lib, &paths, &no_log).unwrap();
        assert_eq!(
            imported
                .iter()
                .map(|b| b.filename.as_str())
                .collect::<Vec<_>>(),
            ["Dune.epub"]
        );
        assert!(
            !lib.join("Fake.epub").exists(),
            "the unreadable copy is removed"
        );
        assert!(!lib.join(crashing.file_name().unwrap()).exists());
        assert!(
            fake.exists() && crashing.exists(),
            "the originals are untouched"
        );
        assert_eq!(index_len(&lib), 1);
    }

    #[test]
    fn a_library_file_that_cannot_be_read_is_recorded_like_in_a_scan() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path().join("lib");
        fs::create_dir_all(&lib).unwrap();
        let fake = lib.join("Fake.pdf");
        write(&fake, b"not a pdf");
        scan(&lib);
        // Dropping the library's own file again: it is not re-parsed, and stays.
        let imported = import_files(&lib, &[fake.to_string_lossy().to_string()], &no_log).unwrap();
        assert!(imported.is_empty());
        assert!(
            fake.exists(),
            "a file that was in the library is never removed"
        );
        let failed = FailedFiles::load(&lib, &no_log);
        assert!(failed.get("Fake.pdf").is_some());
    }

    #[test]
    fn re_importing_a_forgotten_file_brings_it_back() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path().join("lib");
        fs::create_dir_all(&lib).unwrap();
        write_epub(&lib.join("Dune.epub"), "Dune", "Frank Herbert");
        let book = scan(&lib).remove("Dune.epub").unwrap();
        with_index(&lib, &no_log, |index| {
            index.forget(&book.id).unwrap();
            index.save()
        })
        .unwrap();
        assert!(scan(&lib).is_empty(), "ignored by scans");

        // The user drops the same file again (here: the copy in the library itself).
        let again = import_files(
            &lib,
            &[lib.join("Dune.epub").to_string_lossy().to_string()],
            &no_log,
        )
        .unwrap();
        assert_eq!(again.len(), 1);
        assert_eq!(scan(&lib).len(), 1, "no longer ignored");
    }

    #[test]
    fn importing_a_lost_book_again_restores_its_record() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path().join("lib");
        fs::create_dir_all(&lib).unwrap();
        let backup = dir.path().join("backup");
        fs::create_dir_all(&backup).unwrap();
        write_epub(&lib.join("Dune.epub"), "Dune", "Frank Herbert");
        fs::copy(lib.join("Dune.epub"), backup.join("Dune.epub")).unwrap();
        let book = scan(&lib).remove("Dune.epub").unwrap();
        fs::remove_file(lib.join("Dune.epub")).unwrap();
        assert!(scan(&lib).is_empty(), "missing now");

        let restored = import_files(
            &lib,
            &[backup.join("Dune.epub").to_string_lossy().to_string()],
            &no_log,
        )
        .unwrap();
        assert_eq!(
            restored[0].id, book.id,
            "same book: id, metadata and date added are back"
        );
        assert_eq!(restored[0].date_added, book.date_added);
    }

    // ── Download import (11) ──────────────────────────────────────────────────

    fn temp_download(lib: &Path, bytes: &[u8]) -> tempfile::TempPath {
        let dir = lib.join(index::INDEX_DIR);
        fs::create_dir_all(&dir).unwrap();
        let mut tmp = tempfile::Builder::new()
            .prefix(".download-")
            .tempfile_in(dir)
            .unwrap();
        std::io::Write::write_all(&mut tmp, bytes).unwrap();
        tmp.into_temp_path()
    }

    #[test]
    fn downloads_must_match_their_format() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        let temp = temp_download(lib, b"<!DOCTYPE html><html>Sign in</html>");
        let temp_path = temp.to_path_buf();
        let err = import_download(lib, temp, "Dune.epub", &no_log)
            .unwrap_err()
            .to_string();
        assert!(err.contains("not a valid EPUB"), "{err}");
        assert!(!temp_path.exists(), "the rejected download is removed");
        assert!(!lib.join("Dune.epub").exists());
    }

    #[test]
    fn downloads_are_imported_once() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        let fixture = dir.path().join("fixture.epub");
        write_epub(&fixture, "Dune", "Frank Herbert");
        let bytes = fs::read(&fixture).unwrap();
        fs::remove_file(&fixture).unwrap();

        let book = import_download(lib, temp_download(lib, &bytes), "Dune.epub", &no_log).unwrap();
        assert_eq!(book.filename, "Dune.epub");
        assert_eq!(book.metadata.title, "Dune");
        assert_eq!(fs::read(lib.join("Dune.epub")).unwrap(), bytes);
        // The same bytes again: the existing book, no copy.
        let again = import_download(lib, temp_download(lib, &bytes), "Dune.epub", &no_log).unwrap();
        assert_eq!(again.id, book.id);
        assert_eq!(scan(lib).len(), 1);
        let leftovers = fs::read_dir(lib.join(index::INDEX_DIR))
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".download-")
            })
            .count();
        assert_eq!(leftovers, 0);
    }

    // ── Imports and the index lock ────────────────────────────────────────────

    use crate::commands::library::index::is_index_locked;
    use crate::commands::library::test_support::{hooks, within};

    /// Copying (to a NAS) and reading the new books happen without the
    /// index lock: the library keeps answering during a large import.
    #[test]
    fn imports_copy_and_read_without_holding_the_index() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path().join("lib");
        fs::create_dir_all(&lib).unwrap();
        let src = dir.path().join("Dune.epub");
        write_epub(&src, "Dune", "Frank Herbert");

        let _hook = hooks::on(hooks::READ, &lib, {
            let lib = lib.clone();
            move |path| {
                assert!(
                    !is_index_locked(&lib),
                    "the index is locked while importing"
                );
                assert!(
                    path.file_name().unwrap().to_string_lossy().starts_with('.'),
                    "the copy is read while it is still a hidden temp file"
                );
                let lib = lib.clone();
                within(20, move || index_len(&lib));
            }
        });
        let imported = import_files(&lib, &[src.to_string_lossy().to_string()], &no_log).unwrap();
        assert_eq!(imported.len(), 1);
        assert_eq!(imported[0].filename, "Dune.epub");
        assert_eq!(imported[0].path, lib.join("Dune.epub").to_string_lossy());
        assert_eq!(imported[0].metadata.title, "Dune");
        let names: Vec<_> = fs::read_dir(&lib)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| !n.starts_with('.'))
            .collect();
        assert_eq!(names, ["Dune.epub"], "no temp file left behind");
    }

    /// The same payload delivered twice at once (the Windows drag-drop
    /// event) is still recognised as a re-import: imports wait for each
    /// other, so the second sees the first one's copy.
    #[test]
    fn the_same_payload_imported_twice_at_once_is_copied_once() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path().join("lib");
        fs::create_dir_all(&lib).unwrap();
        let src = dir.path().join("Dune.epub");
        write_epub(&src, "Dune", "Frank Herbert");
        let paths = vec![src.to_string_lossy().to_string()];

        let handles: Vec<_> = (0..2)
            .map(|_| {
                let (lib, paths) = (lib.clone(), paths.clone());
                std::thread::spawn(move || import_files(&lib, &paths, &no_log).unwrap())
            })
            .collect();
        let results: Vec<Vec<Book>> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(results[0].len(), 1);
        assert_eq!(results[0][0].id, results[1][0].id, "the same book");
        assert_eq!(index_len(&lib), 1);
        let copies = fs::read_dir(&lib)
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .path()
                    .extension()
                    .is_some_and(|x| x == "epub")
            })
            .count();
        assert_eq!(copies, 1);
    }

    /// A non-EPUB without a title is named after the file it becomes in the
    /// library, not after the temp file it was read from.
    #[test]
    fn a_staged_copy_is_titled_after_its_final_name() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path().join("lib");
        fs::create_dir_all(&lib).unwrap();
        let src = dir.path().join("Old Comic.cbz");
        write(&src, b"PK\x03\x04 not really a zip");
        let imported = import_files(&lib, &[src.to_string_lossy().to_string()], &no_log).unwrap();
        assert_eq!(imported[0].metadata.title, "Old Comic");
        assert_eq!(imported[0].filename, "Old Comic.cbz");
    }
}
