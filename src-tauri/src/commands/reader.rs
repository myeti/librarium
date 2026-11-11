//! The in-app reader: which file the webview should display for a book.
//!
//! **Transport.** The reader gets a *path*, not the book's bytes: the
//! webview loads the file itself through Tauri's asset protocol
//! (`convertFileSrc(path)`), and [`open_for_reading`] adds exactly that file
//! to the protocol's scope. A book never travels as base64 inside the JSON
//! answer of an IPC call: a 300 MB PDF would become a 400 MB string, decoded
//! into more copies on both sides — several GB for one book. Raw bytes through
//! `tauri::ipc::Response` would avoid base64, but still hold the whole book
//! in Rust's memory and copy all of it into the JavaScript heap before
//! anything shows. With the asset protocol, the file is read straight from
//! disk when the webview asks for it, and Range requests let the PDF viewer
//! fetch only the parts it needs. (The covers already reach the webview that
//! way.)
//!
//! **Formats.** EPUB and PDF are displayed as they are. MOBI, AZW and AZW3
//! are converted to EPUB first, and the conversion is cached ([`cache`]), so
//! reopening a Kindle book is instant. CBZ has no reader.
//!
//! **Checks.** The book is looked up by id in the library index; its file
//! must be inside the library, have its format's extension and start like
//! that format (ZIP, `%PDF-`, `BOOKMOBI`): the reader never exposes any
//! other file to the webview.

use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::commands::library::{self, blocking, sniff};
use crate::convert::{convert_to_temp, OutputFormat};
use crate::error::{display_name, AppError, AppResult};
use crate::logging::{logger, Log};
use crate::models::book::BookFormat;

pub(crate) mod cache;

use cache::ReaderCache;

/// Folder of the conversion cache, in the app's cache folder.
const CACHE_FOLDER: &str = "reader";

/// What the reader should display, from [`open_for_reading`].
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReaderSource {
    /// How to display `path`: `"epub"` or `"pdf"`.
    pub format: &'static str,
    /// The file to load with `convertFileSrc` (absolute): the book itself,
    /// or its cached EPUB conversion.
    pub path: String,
    /// The size of that file in bytes.
    pub size_bytes: u64,
    /// Whether `path` is a conversion (MOBI/AZW/AZW3 → EPUB).
    pub converted: bool,
}

/// How a book reaches the reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Delivery {
    /// EPUB and PDF: the file as it is.
    AsIs(&'static str),
    /// MOBI, AZW and AZW3: converted to EPUB first ([`crate::convert`]).
    ConvertToEpub,
}

impl Delivery {
    /// How a book in `format` (`epub`, `pdf`, `mobi`, `azw`, `azw3`, any
    /// case) is read; `INVALID_INPUT` for the others.
    fn of(format: &str) -> AppResult<Self> {
        match format.to_ascii_lowercase().as_str() {
            "epub" => Ok(Self::AsIs("epub")),
            "pdf" => Ok(Self::AsIs("pdf")),
            "mobi" | "azw" | "azw3" => Ok(Self::ConvertToEpub),
            other => Err(AppError::InvalidInput(format!(
                "The reader can't open {} files",
                other.to_uppercase()
            ))),
        }
    }
}

/// The format name of an indexed book, as [`Delivery::of`] and
/// [`check_book_file`] take it.
fn format_name(format: &BookFormat) -> &'static str {
    match format {
        BookFormat::Epub => "epub",
        BookFormat::Pdf => "pdf",
        // MOBI and AZW (old Kindle format) share this variant.
        BookFormat::Mobi => "mobi",
        BookFormat::Azw3 => "azw3",
        BookFormat::Cbz => "cbz",
        BookFormat::Unknown => "unknown",
    }
}

/// Open book `book_id` of the library at `library_path` in the reader:
/// where the webview finds the file to display, and how to display it
/// ([`reader_source`]). The file is added to the asset protocol's scope, so
/// `convertFileSrc(path)` can load it.
#[tauri::command]
pub async fn open_for_reading(
    app: AppHandle,
    library_path: String,
    book_id: String,
) -> AppResult<ReaderSource> {
    let log = logger(&app);
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|e| AppError::Io(format!("The app's cache folder is unknown: {e}")))?
        .join(CACHE_FOLDER);
    let source = blocking(move || {
        let cache = ReaderCache::new(cache_dir, cache::MAX_BYTES);
        reader_source(Path::new(&library_path), &book_id, &cache, &log)
    })
    .await?;
    // Only this file: the scope never opens a whole folder for the reader.
    app.asset_protocol_scope()
        .allow_file(&source.path)
        .map_err(|e| AppError::Internal(format!("could not let the reader load the book: {e}")))?;
    Ok(source)
}

/// [`open_for_reading`], without the asset scope: look book `id` up in the
/// index of `library`, check its file ([`check_book_file`]), and return
/// the file itself (EPUB, PDF) or its EPUB conversion from `cache` (MOBI,
/// AZW, AZW3).
pub(crate) fn reader_source(
    library: &Path,
    id: &str,
    cache: &ReaderCache,
    log: Log,
) -> AppResult<ReaderSource> {
    let book = library::index::find_book(library, id, log)?;
    let format = format_name(&book.format);
    let delivery = Delivery::of(format)?;
    let src = PathBuf::from(&book.path);
    library::files::ensure_in_library(library, &src)?;
    check_book_file(&src, format)?;

    let (format, path, converted) = match delivery {
        Delivery::AsIs(format) => (format, src, false),
        Delivery::ConvertToEpub => {
            let convert = |src: &Path, dir: &Path| convert_to_temp(src, OutputFormat::Epub, dir);
            let epub = cache.epub_for(&book.id, &src, &convert, log)?;
            let how = if epub.hit {
                "cached EPUB"
            } else {
                "converted to EPUB"
            };
            log(format!("[open_for_reading] {} — {how}", display_name(&src)));
            ("epub", epub.path, true)
        }
    };
    let size_bytes = std::fs::metadata(&path)
        .map_err(|e| AppError::io(format!("Could not read {}", display_name(&path)), &e))?
        .len();
    Ok(ReaderSource {
        format,
        path: path.to_string_lossy().into_owned(),
        size_bytes,
        converted,
    })
}

/// Refuse a `path` that isn't a book of `format`: the extension must match
/// (`mobi`, `azw` and `azw3` are interchangeable: they're all Kindle books),
/// and so must the first bytes ([`sniff::ebook_matches_extension`]: ZIP for
/// EPUB, `%PDF-` for PDF, `BOOKMOBI` for MOBI/AZW/AZW3).
fn check_book_file(path: &Path, format: &str) -> AppResult<()> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let kindle = |f: &str| matches!(f, "mobi" | "azw" | "azw3");
    let ext_ok = ext == format || (kindle(format) && kindle(&ext));
    let not_a_book = || {
        AppError::InvalidInput(format!(
            "{} is not a {} book",
            display_name(path),
            format.to_uppercase()
        ))
    };
    if !ext_ok {
        return Err(not_a_book());
    }
    let mut head = Vec::with_capacity(sniff::EBOOK_SNIFF_LEN);
    std::fs::File::open(path)
        .and_then(|f| f.take(sniff::EBOOK_SNIFF_LEN as u64).read_to_end(&mut head))
        .map_err(|e| AppError::io(format!("Could not read {}", display_name(path)), &e))?;
    if sniff::ebook_matches_extension(format, &head) {
        Ok(())
    } else {
        Err(not_a_book())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::tests::write_test_epub;
    use crate::logging::no_log;

    /// An AZW3 made from a test EPUB (boko writes it), at `path`.
    fn write_azw3(path: &Path) {
        let dir = tempfile::tempdir().unwrap();
        let epub = dir.path().join("book.epub");
        write_test_epub(&epub, "Kindle Book", 3, 2000);
        let mut out = std::fs::File::create(path).unwrap();
        boko::Book::open(&epub)
            .unwrap()
            .export(boko::Format::Azw3, &mut out)
            .unwrap();
    }

    /// A library holding an EPUB, a PDF, an AZW3 and a CBZ, scanned into its
    /// index; returns the library folder and the books' ids by file name.
    fn library_with_books(dir: &Path) -> (PathBuf, std::collections::HashMap<String, String>) {
        let lib = dir.join("Library");
        std::fs::create_dir_all(&lib).unwrap();
        write_test_epub(&lib.join("Dune.epub"), "Dune", 2, 500);
        std::fs::write(
            lib.join("Scan.pdf"),
            b"%PDF-1.4\n1 0 obj << >> endobj\ntrailer << >>\n%%EOF\n",
        )
        .unwrap();
        write_azw3(&lib.join("Kindle.azw3"));
        let mut cbz = zip::ZipWriter::new(std::fs::File::create(lib.join("Comic.cbz")).unwrap());
        cbz.start_file("001.jpg", zip::write::SimpleFileOptions::default())
            .unwrap();
        std::io::Write::write_all(&mut cbz, &[0xFF, 0xD8, 0xFF, 0xE0]).unwrap();
        cbz.finish().unwrap();

        let books = library::scan::scan_library_at(&lib, &no_log).unwrap();
        let ids = books.into_iter().map(|b| (b.filename, b.id)).collect();
        (lib, ids)
    }

    #[test]
    fn epub_and_pdf_are_read_in_place_kindle_books_through_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let (lib, ids) = library_with_books(dir.path());
        let cache_dir = dir.path().join("cache");
        let cache = ReaderCache::new(cache_dir.clone(), cache::MAX_BYTES);

        let epub = reader_source(&lib, &ids["Dune.epub"], &cache, &no_log).unwrap();
        assert_eq!((epub.format, epub.converted), ("epub", false));
        assert_eq!(PathBuf::from(&epub.path), lib.join("Dune.epub"));
        assert_eq!(
            epub.size_bytes,
            std::fs::metadata(lib.join("Dune.epub")).unwrap().len()
        );

        let pdf = reader_source(&lib, &ids["Scan.pdf"], &cache, &no_log).unwrap();
        assert_eq!(
            (pdf.format, PathBuf::from(&pdf.path)),
            ("pdf", lib.join("Scan.pdf"))
        );

        let kindle = reader_source(&lib, &ids["Kindle.azw3"], &cache, &no_log).unwrap();
        assert_eq!((kindle.format, kindle.converted), ("epub", true));
        let converted = PathBuf::from(&kindle.path);
        assert!(
            converted.starts_with(&cache_dir),
            "the conversion lives in the cache, not the library"
        );
        assert!(
            std::fs::read(&converted).unwrap().starts_with(b"PK"),
            "an EPUB is a ZIP file"
        );
        assert_eq!(
            reader_source(&lib, &ids["Kindle.azw3"], &cache, &no_log).unwrap(),
            kindle,
            "reopened from the cache"
        );

        assert_eq!(
            reader_source(&lib, &ids["Comic.cbz"], &cache, &no_log)
                .unwrap_err()
                .code(),
            "INVALID_INPUT"
        );
        assert_eq!(
            reader_source(&lib, "no-such-book", &cache, &no_log)
                .unwrap_err()
                .code(),
            "NOT_FOUND"
        );
    }

    #[test]
    fn a_book_file_replaced_by_something_else_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (lib, ids) = library_with_books(dir.path());
        let cache = ReaderCache::new(dir.path().join("cache"), cache::MAX_BYTES);
        std::fs::write(lib.join("Scan.pdf"), b"<html>not a pdf</html>").unwrap();
        assert_eq!(
            reader_source(&lib, &ids["Scan.pdf"], &cache, &no_log)
                .unwrap_err()
                .code(),
            "INVALID_INPUT"
        );
    }

    #[test]
    fn a_failed_real_conversion_leaves_nothing_in_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let broken = dir.path().join("broken.azw3");
        std::fs::write(&broken, b"not a kindle book").unwrap();
        let cache_dir = dir.path().join("cache");
        let cache = ReaderCache::new(cache_dir.clone(), cache::MAX_BYTES);
        let convert = |src: &Path, dir: &Path| convert_to_temp(src, OutputFormat::Epub, dir);

        assert_eq!(
            cache
                .epub_for("book", &broken, &convert, &no_log)
                .unwrap_err()
                .code(),
            "CONVERSION"
        );
        assert!(
            std::fs::read_dir(&cache_dir).unwrap().next().is_none(),
            "no entry, no temp file"
        );
    }

    #[test]
    fn delivery_by_format() {
        assert_eq!(Delivery::of("EPUB").unwrap(), Delivery::AsIs("epub"));
        assert_eq!(Delivery::of("pdf").unwrap(), Delivery::AsIs("pdf"));
        for kindle in ["mobi", "azw", "azw3"] {
            assert_eq!(Delivery::of(kindle).unwrap(), Delivery::ConvertToEpub);
        }
        assert_eq!(Delivery::of("cbz").unwrap_err().code(), "INVALID_INPUT");
    }

    #[test]
    fn only_books_of_the_requested_format_are_read() {
        let dir = tempfile::tempdir().unwrap();
        let epub = dir.path().join("book.epub");
        write_test_epub(&epub, "Book", 1, 100);
        assert!(check_book_file(&epub, "epub").is_ok());
        assert!(check_book_file(&epub, "pdf").is_err(), "extension mismatch");

        let secret = dir.path().join("id_rsa");
        std::fs::write(&secret, b"-----BEGIN OPENSSH PRIVATE KEY-----").unwrap();
        assert_eq!(
            check_book_file(&secret, "pdf").unwrap_err().code(),
            "INVALID_INPUT"
        );
        let disguised = dir.path().join("passwords.pdf");
        std::fs::write(&disguised, b"hunter2").unwrap();
        assert!(
            check_book_file(&disguised, "pdf").is_err(),
            "a .pdf must start like a PDF"
        );
        let pdf = dir.path().join("real.pdf");
        std::fs::write(&pdf, b"%PDF-1.7\n...").unwrap();
        assert!(check_book_file(&pdf, "pdf").is_ok());

        // Kindle extensions are interchangeable, but the content must be a Kindle book.
        let azw = dir.path().join("old.azw");
        write_azw3(&azw);
        assert!(check_book_file(&azw, "mobi").is_ok());
        assert!(check_book_file(&azw, "azw3").is_ok());
        assert!(check_book_file(&epub, "mobi").is_err());
    }
}
