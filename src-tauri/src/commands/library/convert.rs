//! Library operations that convert a book ([`crate::convert`] does the
//! converting): turning a MOBI/AZW/AZW3 book of the library into an EPUB in
//! place ([`convert_book_to_epub`]), and exporting a book to another format
//! at a path the user picked ([`export_book_by_id`]).
//!
//! Both only touch anything once the converted file has been validated. In
//! place, the original then goes to the trash, never deleted outright; an
//! export is written next to its destination and renamed over it.

use std::fs;
use std::path::{Path, PathBuf};

use super::covers::is_safe_id;
use super::epub_edit;
use super::files::{ensure_in_library, make_readable, Trash};
use super::index::{find_book, not_in_library, with_index};
use crate::convert::{convert_to_temp, convert_to_temp_with, ConvertOptions, OutputFormat};
use crate::error::{display_name, AppError, AppResult};
use crate::fsutil;
use crate::logging::Log;
use crate::models::book::{Book, BookFormat};

// ── MOBI → EPUB conversion ────────────────────────────────────────────────────

/// [`convert_to_epub`](super::convert_to_epub), in an order where every
/// failure leaves the library as it was:
/// 1. convert into a hidden, validated temp file next to the original
///    ([`convert_to_temp`]; same folder, so the final move is a rename) —
///    DRM, unreadable or empty output, or output that lost most of the text,
///    is an error;
/// 2. write the index metadata into it (so the file carries the user's edits);
/// 3. under the index lock: move it to its final, unique name and save the
///    index (on failure the new file is removed again);
/// 4. only then — after the index lock is released, since the OS trash can
///    take a while on a NAS — move the original to the trash, never deleted
///    outright.
pub(crate) fn convert_book_to_epub(
    lib: &Path,
    id: &str,
    trash: Trash,
    log: Log,
) -> AppResult<Book> {
    let book = find_book(lib, id, log)?;
    let src = PathBuf::from(&book.path);
    ensure_in_library(lib, &src)?;
    let ext = src
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    if !["mobi", "azw", "azw3"].contains(&ext.as_str()) {
        return Err(AppError::InvalidInput(format!(
            "Only MOBI, AZW and AZW3 books can be converted to EPUB (this one is {})",
            ext.to_uppercase()
        )));
    }
    if !src.is_file() {
        return Err(AppError::NotFound(format!(
            "{} is missing from the library folder; rescan the library",
            book.filename
        )));
    }
    let dir = src.parent().unwrap_or(lib).to_path_buf();
    log(format!("[convert_to_epub] start — id={} {}", id, book.path));

    let temp = convert_to_temp(&src, OutputFormat::Epub, &dir)?.into_temp_path();
    log("[convert_to_epub] converted and validated".into());
    if let Err(e) = epub_edit::write_metadata(&temp, &book.metadata) {
        // Not fatal: the index keeps the metadata either way.
        log(format!(
            "[convert_to_epub] could not write the metadata into the EPUB: {}",
            e
        ));
    }

    let new_book = with_index(lib, log, |index| {
        let current = index.by_id(id).cloned().ok_or_else(|| not_in_library(id))?;
        if current.path != book.path {
            return Err(AppError::Conflict(
                "The book was renamed or moved during the conversion; nothing was changed. Try again.".into(),
            ));
        }
        let stem = src.file_stem().and_then(|s| s.to_str()).unwrap_or("book");
        let dest = fsutil::unique_path(
            &dir,
            &format!(
                "{}.epub",
                fsutil::sanitize_stem(stem, fsutil::MAX_FILENAME_BYTES - 5)
            ),
        );
        temp.persist(&dest)
            .map_err(|e| AppError::io("Could not save the converted EPUB", &e.error))?;
        make_readable(&dest);

        let new_book = Book {
            path: dest.to_string_lossy().to_string(),
            filename: dest
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            format: BookFormat::Epub,
            size_bytes: fs::metadata(&dest).map(|m| m.len()).unwrap_or(0),
            ..current.clone()
        };
        index.remove(&current.path);
        index.insert(new_book.clone());
        if let Err(e) = index.save() {
            // Our own output, not user data: removing it restores the library.
            let _ = fs::remove_file(&dest);
            return Err(e);
        }
        Ok(new_book)
    })?;

    // The index no longer knows the original: trash it without the lock
    // (under the file's own lock, like a delete).
    match fsutil::with_file_lock(&src, || trash(&src)) {
        Ok(outcome) => log(format!(
            "[convert_to_epub] original moved to the trash ({:?})",
            outcome
        )),
        Err(e) => log(format!(
            "[convert_to_epub] WARNING — could not move the original {} to the trash ({}); \
             it stays in the library and will show up again on the next scan",
            book.filename, e
        )),
    }
    log(format!(
        "[convert_to_epub] done — new path={}",
        new_book.path
    ));
    Ok(new_book)
}

// ── Export ────────────────────────────────────────────────────────────────────

/// [`export_book`](super::export_book): convert the book `id` of the library
/// to `format` and write it to `dest`, a path the user picked in a save
/// dialog (anywhere; an existing file there is replaced).
///
/// The conversion goes to a hidden temp file in `dest`'s folder, validated,
/// then renamed over `dest`: a failed export never leaves a half-written or
/// empty file, and never damages what was at `dest` before. Refused: a
/// `dest` that is the book itself (or, through a link or another spelling,
/// the same file), and one that is another book of the library.
pub(crate) fn export_book_by_id(
    lib: &Path,
    id: &str,
    format: OutputFormat,
    dest: &Path,
    log: Log,
) -> AppResult<()> {
    let book = find_book(lib, id, log)?;
    let src = PathBuf::from(&book.path);
    if !src.is_file() {
        return Err(AppError::NotFound(format!(
            "{} is missing from the library folder; rescan the library",
            book.filename
        )));
    }
    check_export_dest(lib, &src, dest, log)?;
    let dir = dest
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    log(format!(
        "[export_book] {} → {} ({:?})",
        book.path,
        dest.display(),
        format
    ));

    // An AZW3 carries the book id as its ASIN (EXTH 113), like the copies
    // sent to a Kindle, rather than boko's shared placeholder.
    let options = ConvertOptions {
        asin: Some(book.id.as_str()).filter(|id| is_safe_id(id)),
        ..Default::default()
    };
    let temp = convert_to_temp_with(&src, format, dir, options)?;
    temp.persist(dest)
        .map_err(|e| AppError::io(format!("Could not save {}", display_name(dest)), &e.error))?;
    make_readable(dest);
    log(format!("[export_book] done — {}", dest.display()));
    Ok(())
}

/// Refuse an export destination that would overwrite the source book, or
/// another book of the library. Paths are compared canonicalised (symlinks,
/// `..`, case on case-insensitive volumes), the destination through its
/// folder since the file itself may not exist yet.
fn check_export_dest(lib: &Path, src: &Path, dest: &Path, log: Log) -> AppResult<()> {
    let name = dest
        .file_name()
        .ok_or_else(|| AppError::InvalidInput("The export destination has no file name".into()))?;
    let dest_dir = dest
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let dest_dir = fs::canonicalize(dest_dir).map_err(|e| {
        AppError::io(
            format!("The folder for {} can't be used", name.to_string_lossy()),
            &e,
        )
    })?;
    let dest_canon = fs::canonicalize(dest).unwrap_or_else(|_| dest_dir.join(name));
    let src_canon = fs::canonicalize(src).unwrap_or_else(|_| src.to_path_buf());
    if dest_canon == src_canon {
        return Err(AppError::InvalidInput(
            "The export would overwrite the book itself; choose another name or folder".into(),
        ));
    }
    let lib_canon = fs::canonicalize(lib).unwrap_or_else(|_| lib.to_path_buf());
    if fsutil::is_within(&lib_canon, &dest_canon) {
        // Inside the library: never over another book.
        let other = with_index(lib, log, |index| {
            Ok(index
                .books()
                .any(|b| fs::canonicalize(&b.path).is_ok_and(|p| p == dest_canon)))
        })?;
        if other {
            return Err(AppError::Conflict(format!(
                "{} is another book of the library; choose another name",
                name.to_string_lossy()
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::library::edit::update_book_metadata;
    use crate::commands::library::test_support::{
        indexed, opf_of, scan, test_trash, write, write_epub,
    };
    use crate::logging::no_log;

    /// Build an AZW3 in `lib` by converting an EPUB fixture with boko.
    fn azw3_fixture(lib: &Path, name: &str) -> PathBuf {
        let epub = lib.join(".fixture.epub");
        write_epub(&epub, "Dune", "Frank Herbert");
        let azw3 = lib.join(name);
        let mut out = fs::File::create(&azw3).unwrap();
        boko::Book::open(&epub)
            .unwrap()
            .export(boko::Format::Azw3, &mut out)
            .unwrap();
        fs::remove_file(&epub).unwrap();
        azw3
    }

    #[test]
    fn conversion_keeps_the_book_and_trashes_the_original() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        azw3_fixture(lib, "Dune.azw3");
        let book = scan(lib).remove("Dune.azw3").unwrap();
        let mut meta = book.metadata.clone();
        meta.author = "Frank Herbert".into();
        meta.series = Some("Dune".into());
        meta.rating = Some(5);
        update_book_metadata(lib, &book.id, meta, &no_log).unwrap();

        // The original is trashed once the index lock is released (the OS
        // trash can take a while on a NAS).
        let trash = |path: &Path| {
            assert!(
                !crate::commands::library::index::is_index_locked(lib),
                "the index is locked while the original is trashed"
            );
            test_trash(lib)(path)
        };
        let converted = convert_book_to_epub(lib, &book.id, &trash, &no_log).unwrap();
        assert_eq!(converted.id, book.id);
        assert_eq!(converted.filename, "Dune.epub");
        assert_eq!(converted.format, BookFormat::Epub);
        assert_eq!(converted.date_added, book.date_added);
        assert_eq!(converted.metadata.rating, Some(5));
        assert_eq!(converted.metadata.series.as_deref(), Some("Dune"));

        assert!(!lib.join("Dune.azw3").exists());
        let trashed: Vec<_> = walkdir::WalkDir::new(lib.join(fsutil::LIBRARY_TRASH_DIR))
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name() == "Dune.azw3")
            .collect();
        assert_eq!(
            trashed.len(),
            1,
            "the original is in the trash, not deleted"
        );

        // The index and a rescan agree: one book, same id, now an EPUB.
        let books = scan(lib);
        assert_eq!(books.len(), 1);
        assert_eq!(books["Dune.epub"].id, book.id);
        // The EPUB is readable and carries the metadata.
        let opf = opf_of(&lib.join("Dune.epub"));
        assert!(opf.contains("Frank Herbert"), "{opf}");
        assert!(opf.contains("calibre:series"));
        let leftovers = fs::read_dir(lib)
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".librarium-convert-")
            })
            .count();
        assert_eq!(leftovers, 0);
    }

    #[test]
    fn a_failed_conversion_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        // Looks like a MOBI from the outside, but is garbage.
        let mut junk = vec![0u8; 4096];
        junk[60..68].copy_from_slice(b"BOOKMOBI");
        write(&lib.join("Broken.mobi"), &junk);
        let book = scan(lib).remove("Broken.mobi").unwrap();

        let err = convert_book_to_epub(lib, &book.id, &test_trash(lib), &no_log).unwrap_err();
        assert_eq!(err.code(), "CONVERSION");
        assert_eq!(fs::read(lib.join("Broken.mobi")).unwrap(), junk);
        assert_eq!(indexed(lib, &book.id).unwrap().path, book.path);
        let names: Vec<_> = fs::read_dir(lib)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .filter(|n| !n.starts_with(".librarium"))
            .collect();
        assert_eq!(
            names,
            ["Broken.mobi"],
            "no EPUB and no temp file left behind"
        );
    }

    #[test]
    fn drm_protected_books_are_refused_with_a_clear_message() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        let azw3 = azw3_fixture(lib, "Locked.azw3");
        // Set the encryption type of the MOBI header (record 0, offset 12).
        let mut bytes = fs::read(&azw3).unwrap();
        let record0 = u32::from_be_bytes(bytes[78..82].try_into().unwrap()) as usize;
        bytes[record0 + 12..record0 + 14].copy_from_slice(&2u16.to_be_bytes());
        fs::write(&azw3, &bytes).unwrap();
        let book = scan(lib).remove("Locked.azw3").unwrap();

        let err = convert_book_to_epub(lib, &book.id, &test_trash(lib), &no_log).unwrap_err();
        assert!(err.to_string().contains("DRM"), "{err}");
        assert!(azw3.exists());
    }

    #[test]
    fn only_kindle_formats_are_converted() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        write_epub(&lib.join("a.epub"), "A", "X");
        let book = scan(lib).remove("a.epub").unwrap();
        let err = convert_book_to_epub(lib, &book.id, &test_trash(lib), &no_log).unwrap_err();
        assert_eq!(err.code(), "INVALID_INPUT");
    }

    // ── Export ────────────────────────────────────────────────────────────────

    #[test]
    fn export_writes_a_validated_file_and_replaces_an_existing_one() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path().join("lib");
        fs::create_dir_all(&lib).unwrap();
        write_epub(&lib.join("Dune.epub"), "Dune", "Frank Herbert");
        let book = scan(&lib).remove("Dune.epub").unwrap();
        let out = dir.path().join("out");
        fs::create_dir_all(&out).unwrap();
        let dest = out.join("Dune.azw3");
        fs::write(&dest, b"an older export").unwrap();

        export_book_by_id(&lib, &book.id, OutputFormat::Azw3, &dest, &no_log).unwrap();
        let bytes = fs::read(&dest).unwrap();
        assert_eq!(&bytes[60..68], b"BOOKMOBI", "an AZW3 replaced the old file");
        let names: Vec<_> = fs::read_dir(&out)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["Dune.azw3"], "no temp file left next to it");
    }

    #[test]
    fn a_failed_export_leaves_the_destination_alone() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path().join("lib");
        fs::create_dir_all(&lib).unwrap();
        let mut junk = vec![0u8; 4096];
        junk[60..68].copy_from_slice(b"BOOKMOBI");
        write(&lib.join("Broken.mobi"), &junk);
        let book = scan(&lib).remove("Broken.mobi").unwrap();
        let dest = dir.path().join("Broken.epub");
        fs::write(&dest, b"keep me").unwrap();

        assert!(export_book_by_id(&lib, &book.id, OutputFormat::Epub, &dest, &no_log).is_err());
        assert_eq!(fs::read(&dest).unwrap(), b"keep me");
        assert_eq!(
            fs::read_dir(dir.path()).unwrap().count(),
            2,
            "lib and the untouched file only"
        );
    }

    #[test]
    fn export_refuses_to_overwrite_the_book_or_another_book() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path().join("lib");
        fs::create_dir_all(lib.join("Sub")).unwrap();
        write_epub(&lib.join("Dune.epub"), "Dune", "Frank Herbert");
        write_epub(&lib.join("Emma.epub"), "Emma", "Jane Austen");
        let books = scan(&lib);
        let dune = &books["Dune.epub"];
        let before = fs::read(lib.join("Dune.epub")).unwrap();

        let err = export_book_by_id(
            &lib,
            &dune.id,
            OutputFormat::Epub,
            &lib.join("Dune.epub"),
            &no_log,
        )
        .unwrap_err();
        assert!(err.to_string().contains("the book itself"), "{err}");
        // The same file through another spelling.
        let err = export_book_by_id(
            &lib,
            &dune.id,
            OutputFormat::Epub,
            &lib.join("Sub/../Dune.epub"),
            &no_log,
        )
        .unwrap_err();
        assert!(err.to_string().contains("the book itself"), "{err}");
        let err = export_book_by_id(
            &lib,
            &dune.id,
            OutputFormat::Epub,
            &lib.join("Emma.epub"),
            &no_log,
        )
        .unwrap_err();
        assert_eq!(err.code(), "CONFLICT");
        assert_eq!(fs::read(lib.join("Dune.epub")).unwrap(), before);

        // A new file inside the library is fine (the next scan adds it).
        export_book_by_id(
            &lib,
            &dune.id,
            OutputFormat::Azw3,
            &lib.join("Sub/Dune.azw3"),
            &no_log,
        )
        .unwrap();
        assert!(lib.join("Sub/Dune.azw3").is_file());

        let err = export_book_by_id(
            &lib,
            "no-such-id",
            OutputFormat::Epub,
            &dir.path().join("x.epub"),
            &no_log,
        )
        .unwrap_err();
        assert_eq!(err.code(), "NOT_FOUND");
    }
}
