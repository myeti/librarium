//! Converting books with boko: the one way the app turns a book into
//! another format.
//!
//! Four features convert, and all go through [`convert_to_temp`]:
//! - "Convert to EPUB" of a MOBI/AZW3 in the library (in place:
//!   `commands/library/convert.rs`);
//! - "Export as…" (`commands/library/convert.rs`);
//! - the reader, which opens MOBI/AZW3 as EPUB (`commands/reader.rs`);
//! - device transfers: AZW3 for a Kindle, EPUB for a Kobo
//!   (`commands/device.rs`).
//!
//! What every conversion gets:
//! 1. the source is opened by boko; a DRM-protected book is refused with a
//!    clear message;
//! 2. the result is written to a new, uniquely named hidden temp file
//!    (`.librarium-convert-XXXX.epub`) in the folder the caller picks — next
//!    to the final destination when it is to be renamed into place (the
//!    hidden name keeps library scans away from it);
//! 3. it is fsynced, then **validated** ([`validate`]): it must open again,
//!    have at least one chapter, have text (or images, for picture books),
//!    and keep most of the source's text. The converters this replaces
//!    produced empty books on decompression errors;
//! 4. a panic inside boko is caught and reported as a conversion error.
//!
//! The temp file is deleted when the returned handle is dropped — on
//! error, on cancellation, while unwinding from a panic — unless the caller
//! persists it. Every message ends with "Nothing was changed.": no caller
//! touches the original before the conversion succeeded.

use std::io::Write;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

use crate::error::{display_name, panic_message, AppError, AppResult};

/// A format boko can write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutputFormat {
    Epub,
    /// KF8, the format of current Kindles.
    Azw3,
}

impl OutputFormat {
    /// The format called `name` (`epub` or `azw3`, any case). Anything else
    /// — PDF, old MOBI, which boko can't write — is refused.
    pub(crate) fn parse(name: &str) -> AppResult<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "epub" => Ok(Self::Epub),
            "azw3" => Ok(Self::Azw3),
            other => Err(AppError::InvalidInput(format!(
                "Conversion to {} is not supported",
                other.to_uppercase()
            ))),
        }
    }

    /// The format for a device transfer's `target_format`: as [`parse`](Self::parse),
    /// except that `mobi` and `azw` also give AZW3 — what a Kindle reads
    /// (boko can't write old MOBI).
    pub(crate) fn for_device(target: &str) -> AppResult<Self> {
        match target.trim().to_ascii_lowercase().as_str() {
            "azw" | "mobi" => Ok(Self::Azw3),
            _ => Self::parse(target),
        }
    }

    /// File extension, without the dot.
    pub(crate) fn extension(self) -> &'static str {
        match self {
            Self::Epub => "epub",
            Self::Azw3 => "azw3",
        }
    }

    fn boko(self) -> boko::Format {
        match self {
            Self::Epub => boko::Format::Epub,
            Self::Azw3 => boko::Format::Azw3,
        }
    }
}

/// Prefix of the temp files conversions write (hidden: scans skip them).
pub(crate) const TEMP_PREFIX: &str = ".librarium-convert-";

/// Test hook: converting a file whose path contains this marker panics,
/// as boko crashing on a malformed book would.
#[cfg(test)]
use crate::error::PANIC_TEST_MARKER;

/// Per-conversion settings for [`convert_to_temp_with`].
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ConvertOptions<'a> {
    /// AZW3 only: the ASIN written to EXTH 113. Librarium passes the
    /// library book id, so every book it writes has its own ASIN (Kindle
    /// keys home-screen thumbnails on it) and a device copy can be traced
    /// back to its library book. `None` keeps boko's `EBOK000000`
    /// placeholder. Ignored for EPUB output.
    pub asin: Option<&'a str>,
    /// AZW3 only: the date written to the file's header (Unix seconds),
    /// which also seeds the key that obfuscates embedded fonts. With a fixed
    /// value, converting the same book twice gives byte-identical files, so
    /// a re-sent book that hasn't changed can be left alone on the device
    /// (and keeps its Kindle reading position). `None` uses the current time.
    pub timestamp: Option<u32>,
}

/// Convert the book at `src` to `format`, into a new hidden temp file in
/// `dir`, and validate the result (see the module docs). The file is
/// removed when the returned handle is dropped, unless it is persisted.
pub(crate) fn convert_to_temp(
    src: &Path,
    format: OutputFormat,
    dir: &Path,
) -> AppResult<tempfile::NamedTempFile> {
    convert_to_temp_with(src, format, dir, ConvertOptions::default())
}

/// [`convert_to_temp`] with [`ConvertOptions`].
pub(crate) fn convert_to_temp_with(
    src: &Path,
    format: OutputFormat,
    dir: &Path,
    options: ConvertOptions,
) -> AppResult<tempfile::NamedTempFile> {
    let name = display_name(src);
    match catch_unwind(AssertUnwindSafe(|| {
        convert_unguarded(src, format, dir, &name, options)
    })) {
        Ok(result) => result,
        // The temp file was dropped (and removed) while unwinding.
        Err(payload) => Err(AppError::Conversion(format!(
            "Converting {name} crashed the converter ({}). Nothing was changed.",
            panic_message(payload.as_ref())
        ))),
    }
}

fn convert_unguarded(
    src: &Path,
    format: OutputFormat,
    dir: &Path,
    name: &str,
    options: ConvertOptions,
) -> AppResult<tempfile::NamedTempFile> {
    #[cfg(test)]
    if src.to_string_lossy().contains(PANIC_TEST_MARKER) {
        panic!("forced panic (test hook)");
    }
    let source = boko::Book::open(src).map_err(|e| source_error(name, e))?;
    let mut tmp = tempfile::Builder::new()
        .prefix(TEMP_PREFIX)
        .suffix(&format!(".{}", format.extension()))
        .tempfile_in(dir)
        .map_err(|e| {
            AppError::io(
                format!("Could not create a temporary file for converting {name}"),
                &e,
            )
        })?;
    let exported = match format {
        // The exporter itself rather than `Book::export`, to pass the ASIN
        // and the timestamp; apart from that it is exactly what
        // `Book::export` does.
        OutputFormat::Azw3 => {
            let config = boko::Azw3Config {
                asin: options.asin.map(str::to_string),
                timestamp: options.timestamp,
                ..Default::default()
            };
            boko::Exporter::export(
                &boko::Azw3Exporter::new().with_config(config),
                &source,
                tmp.as_file_mut(),
            )
        }
        OutputFormat::Epub => source.export(format.boko(), tmp.as_file_mut()),
    };
    exported.map_err(|e| {
        AppError::Conversion(format!(
            "Converting {name} failed ({e}). Nothing was changed."
        ))
    })?;
    tmp.as_file_mut()
        .flush()
        .and_then(|()| crate::fsutil::sync_file(tmp.as_file()))
        .map_err(|e| AppError::io(format!("Could not write the converted {name}"), &e))?;
    validate(tmp.path(), format, visible_text_chars(&source))?;
    Ok(tmp)
}

/// The message for a boko error on the *source* book.
fn source_error(name: &str, e: boko::Error) -> AppError {
    match e {
        boko::Error::DrmProtected(_) => AppError::Conversion(format!(
            "{name} is DRM-protected, so Librarium can't convert it. Nothing was changed."
        )),
        other => AppError::Conversion(format!(
            "Could not read {name} for conversion ({other}). Nothing was changed."
        )),
    }
}

/// Check a freshly converted book before anything uses it: it must open,
/// have at least one chapter, contain text (or images, for picture books),
/// and keep most of the source's text (`source_text` visible characters).
pub(crate) fn validate(path: &Path, format: OutputFormat, source_text: usize) -> AppResult<()> {
    let fail = |what: String| {
        Err(AppError::Conversion(format!(
            "{what}. Nothing was changed."
        )))
    };
    let out = match boko::Book::open_format(path, format.boko()) {
        Ok(out) => out,
        Err(e) => return fail(format!("The converted book can't be read back ({e})")),
    };
    if out.spine().is_empty() {
        return fail("The conversion produced an empty book".into());
    }
    let text = visible_text_chars(&out);
    let has_images = out.list_assets().iter().any(|a| {
        let a = a.to_lowercase();
        [".jpg", ".jpeg", ".png", ".gif", ".webp", ".svg"]
            .iter()
            .any(|ext| a.ends_with(ext))
    });
    if text == 0 && !has_images {
        return fail("The conversion produced a book without any text".into());
    }
    // A conversion that keeps under a tenth of the text is a failed one.
    if source_text >= 1000 && text < source_text / 10 {
        return fail(format!(
            "The conversion lost most of the text ({text} of about {source_text} characters)"
        ));
    }
    Ok(())
}

/// Rough count of a book's visible text: non-whitespace characters outside
/// tags, in the `<body>` of each chapter.
fn visible_text_chars(book: &boko::Book) -> usize {
    let ids: Vec<_> = book.spine().iter().map(|entry| entry.id).collect();
    ids.into_iter()
        .filter_map(|id| book.load_raw(id).ok())
        .map(|raw| html_text_chars(&String::from_utf8_lossy(&raw)))
        .sum()
}

/// Non-whitespace characters outside tags, from `<body` on (or the whole
/// string when there is no body tag).
fn html_text_chars(html: &str) -> usize {
    let start = html
        .find("<body")
        .or_else(|| html.find("<BODY"))
        .unwrap_or(0);
    let mut in_tag = false;
    let mut count = 0;
    for c in html[start..].chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag && !c.is_whitespace() => count += 1,
            _ => {}
        }
    }
    count
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::fs;
    use zip::write::SimpleFileOptions;

    /// Write a valid EPUB 2 at `path` with `chapters` chapters of about
    /// `chars` visible characters each.
    pub(crate) fn write_test_epub(path: &Path, title: &str, chapters: usize, chars: usize) {
        let file = fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let stored =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        let deflated = SimpleFileOptions::default();
        zip.start_file("mimetype", stored).unwrap();
        zip.write_all(b"application/epub+zip").unwrap();
        zip.start_file("META-INF/container.xml", deflated).unwrap();
        zip.write_all(br#"<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#).unwrap();
        let mut manifest = String::new();
        let mut spine = String::new();
        for i in 0..chapters {
            manifest.push_str(&format!(
                r#"<item id="c{i}" href="c{i}.xhtml" media-type="application/xhtml+xml"/>"#
            ));
            spine.push_str(&format!(r#"<itemref idref="c{i}"/>"#));
            let words = "lorem ipsum dolor sit amet ".repeat(chars / 22 + 1);
            zip.start_file(format!("OEBPS/c{i}.xhtml"), deflated)
                .unwrap();
            write!(
                zip,
                r#"<?xml version="1.0" encoding="utf-8"?><html xmlns="http://www.w3.org/1999/xhtml"><head><title>{i}</title></head><body><h1>Chapter {i}</h1><p>{words}</p></body></html>"#
            )
            .unwrap();
        }
        zip.start_file("OEBPS/content.opf", deflated).unwrap();
        write!(
            zip,
            r#"<?xml version="1.0" encoding="utf-8"?><package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>{title}</dc:title><dc:creator>Test</dc:creator><dc:language>en</dc:language><dc:identifier id="id">urn:uuid:12345678-1234-1234-1234-123456789abc</dc:identifier></metadata><manifest>{manifest}</manifest><spine>{spine}</spine></package>"#
        )
        .unwrap();
        zip.finish().unwrap();
    }

    /// Write an EPUB at `path` whose chapters use an embedded font, from
    /// `font` (bytes boko doesn't parse: any data will do).
    fn write_epub_with_font(path: &Path, font: &[u8]) {
        let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
        let stored =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        let deflated = SimpleFileOptions::default();
        zip.start_file("mimetype", stored).unwrap();
        zip.write_all(b"application/epub+zip").unwrap();
        zip.start_file("META-INF/container.xml", deflated).unwrap();
        zip.write_all(br#"<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#).unwrap();
        zip.start_file("OEBPS/content.opf", deflated).unwrap();
        zip.write_all(br#"<?xml version="1.0" encoding="utf-8"?><package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Fonts</dc:title><dc:creator>Test</dc:creator><dc:language>en</dc:language><dc:identifier id="id">urn:uuid:12345678-1234-1234-1234-123456789abc</dc:identifier></metadata><manifest><item id="css" href="style.css" media-type="text/css"/><item id="font" href="book.ttf" media-type="font/ttf"/><item id="c0" href="c0.xhtml" media-type="application/xhtml+xml"/><item id="c1" href="c1.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="c0"/><itemref idref="c1"/></spine></package>"#).unwrap();
        zip.start_file("OEBPS/style.css", deflated).unwrap();
        zip.write_all(br#"@font-face { font-family: "Book"; src: url(book.ttf); } p { font-family: "Book"; }"#).unwrap();
        zip.start_file("OEBPS/book.ttf", deflated).unwrap();
        zip.write_all(font).unwrap();
        for i in 0..2 {
            let words = "lorem ipsum dolor sit amet ".repeat(200);
            zip.start_file(format!("OEBPS/c{i}.xhtml"), deflated)
                .unwrap();
            write!(
                zip,
                r#"<?xml version="1.0" encoding="utf-8"?><html xmlns="http://www.w3.org/1999/xhtml"><head><title>{i}</title><link rel="stylesheet" type="text/css" href="style.css"/></head><body><h1>Chapter {i}</h1><p>{words}</p></body></html>"#
            )
            .unwrap();
        }
        zip.finish().unwrap();
    }

    fn files_in(dir: &Path) -> Vec<String> {
        fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn formats_by_name() {
        assert_eq!(OutputFormat::parse("EPUB").unwrap(), OutputFormat::Epub);
        assert_eq!(OutputFormat::parse("azw3").unwrap(), OutputFormat::Azw3);
        assert_eq!(
            OutputFormat::parse("pdf").unwrap_err().code(),
            "INVALID_INPUT"
        );
        assert!(
            OutputFormat::parse("mobi").is_err(),
            "an export to .mobi would be an AZW3 under a lying name"
        );
        assert_eq!(
            OutputFormat::for_device("mobi").unwrap(),
            OutputFormat::Azw3
        );
        assert_eq!(OutputFormat::for_device("azw").unwrap(), OutputFormat::Azw3);
        assert_eq!(
            OutputFormat::for_device("epub").unwrap(),
            OutputFormat::Epub
        );
        assert!(OutputFormat::for_device("pdf").is_err());
    }

    #[test]
    fn round_trips_through_azw3_and_validates_both_ways() {
        let dir = tempfile::tempdir().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let epub = dir.path().join("Long.epub");
        // Enough text for the "kept most of the text" check to apply.
        write_test_epub(&epub, "Long", 12, 3000);

        let azw3 = convert_to_temp(&epub, OutputFormat::Azw3, temp.path()).unwrap();
        let name = azw3
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert!(
            name.starts_with(TEMP_PREFIX) && name.ends_with(".azw3"),
            "{name}"
        );
        let kept = dir.path().join("Long.azw3");
        azw3.persist(&kept).unwrap();

        let back = convert_to_temp(&kept, OutputFormat::Epub, temp.path()).unwrap();
        let bytes = fs::read(back.path()).unwrap();
        assert!(bytes.starts_with(b"PK"), "an EPUB is a ZIP file");
        drop(back);
        assert!(
            files_in(temp.path()).is_empty(),
            "the temp file goes away with its handle"
        );
    }

    #[test]
    fn an_azw3_with_a_fixed_timestamp_is_the_same_file_every_time() {
        let dir = tempfile::tempdir().unwrap();
        let epub = dir.path().join("Fonts.epub");
        // Data that doesn't compress, so the font record is obfuscated with
        // a key: the other thing upstream boko draws at random.
        let mut x = 1u32;
        let font: Vec<u8> = (0..8_000)
            .map(|_| {
                x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                (x >> 16) as u8
            })
            .collect();
        write_epub_with_font(&epub, &font);

        let convert = |timestamp| {
            let options = ConvertOptions {
                asin: Some("3f2b1c3e-6e0a-4c1b-9a43-3f1f3c0f9b11"),
                timestamp,
            };
            let out = convert_to_temp_with(&epub, OutputFormat::Azw3, dir.path(), options).unwrap();
            fs::read(out.path()).unwrap()
        };
        let first = convert(Some(1_700_000_000));
        assert!(
            first.windows(4).any(|w| w == b"FONT"),
            "the font is embedded"
        );
        // The PDB header's creation and modification dates.
        assert_eq!(first[36..40], 1_700_000_000u32.to_be_bytes());
        assert_eq!(first[40..44], 1_700_000_000u32.to_be_bytes());
        assert!(
            first == convert(Some(1_700_000_000)),
            "converting again gives the same bytes"
        );
        assert!(first != convert(Some(1_700_000_001)));
    }

    #[test]
    fn failures_leave_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let broken = dir.path().join("broken.azw3");
        fs::write(&broken, b"not a kindle book").unwrap();
        let err = convert_to_temp(&broken, OutputFormat::Epub, temp.path()).unwrap_err();
        assert_eq!(err.code(), "CONVERSION");
        assert!(
            err.to_string().contains("broken.azw3")
                && err.to_string().ends_with("Nothing was changed."),
            "{err}"
        );
        assert!(files_in(temp.path()).is_empty());
    }

    #[test]
    fn a_converter_panic_is_a_conversion_error() {
        let dir = tempfile::tempdir().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let src = dir.path().join(format!("{PANIC_TEST_MARKER}.epub"));
        write_test_epub(&src, "Crash", 1, 100);
        let err = convert_to_temp(&src, OutputFormat::Azw3, temp.path()).unwrap_err();
        assert_eq!(err.code(), "CONVERSION");
        assert!(err.to_string().contains("crashed the converter"), "{err}");
        assert!(files_in(temp.path()).is_empty());
    }

    #[test]
    fn validation_rejects_books_that_lost_their_text() {
        let dir = tempfile::tempdir().unwrap();
        let epub = dir.path().join("Short.epub");
        write_test_epub(&epub, "Short", 1, 200);
        assert!(validate(&epub, OutputFormat::Epub, 200).is_ok());
        let err = validate(&epub, OutputFormat::Epub, 1_000_000).unwrap_err();
        assert!(err.to_string().contains("lost most of the text"), "{err}");
        let junk = dir.path().join("junk.epub");
        fs::write(&junk, b"PK\x03\x04 junk").unwrap();
        assert!(validate(&junk, OutputFormat::Epub, 0)
            .unwrap_err()
            .to_string()
            .contains("can't be read back"));
    }

    #[test]
    fn text_counting_ignores_markup_and_head() {
        assert_eq!(
            html_text_chars(
                "<html><head><title>Long title</title></head><body><p>ab c</p></body></html>"
            ),
            3
        );
        assert_eq!(html_text_chars("plain <b>text</b>"), 9);
    }
}
