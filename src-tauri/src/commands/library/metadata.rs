//! Reading a book's metadata from its file: the OPF for EPUB
//! ([`crate::epub`]), the Info dictionary for PDF, the file name for
//! everything else. Also the language helpers: code normalisation (`fre` →
//! `fr`) and text-based detection.
//!
//! Nothing here writes: metadata write-back lives in [`super::epub_edit`].

use std::path::Path;

use crate::epub::{strip_html, EpubMetadata};
use crate::models::book::{BookFormat, BookMetadata};

/// The metadata stored in the file at `path`, read according to its
/// `format`. Never fails. Unknown values are empty (`""`, `None`), never
/// placeholders: the UI shows a localised "Unknown title". The title comes
/// from the file name ([`title_from_filename`]) only for files that carry no
/// title field Librarium can read: PDFs without one, MOBI/AZW3/CBZ, and
/// EPUBs that can't be opened. A readable EPUB without a title keeps `""`.
#[cfg(test)]
pub(crate) fn read_metadata(path: &Path, format: &BookFormat) -> BookMetadata {
    read_metadata_as(path, path, format)
}

/// [`read_metadata`] of the file at `read`, which is (or is about to be) in
/// the library as `named`: a title taken from the file name comes from
/// `named` — an import reads its copy while it is still a hidden temp file.
pub(crate) fn read_metadata_as(read: &Path, named: &Path, format: &BookFormat) -> BookMetadata {
    match format {
        BookFormat::Epub => match crate::epub::read_metadata(read) {
            Ok(epub) => from_epub(epub),
            Err(_) => BookMetadata {
                title: title_from_filename(named),
                ..Default::default()
            },
        },
        BookFormat::Pdf => extract_pdf_metadata(read, named),
        _ => BookMetadata {
            title: title_from_filename(named),
            ..Default::default()
        },
    }
}

/// Extract the human-readable title from a filename stem as a last resort.
/// Replaces hyphens and underscores with spaces and trims. A name that isn't
/// valid Unicode is decoded lossily; a path without a file name gives `""`
/// (an unknown title), never a placeholder.
fn title_from_filename(path: &Path) -> String {
    path.file_stem()
        .map(|s| {
            s.to_string_lossy()
                .replace(['-', '_'], " ")
                .trim()
                .to_string()
        })
        .unwrap_or_default()
}

/// The library's view of an EPUB's metadata: several authors are joined
/// with `" & "` (the separator the editor splits on when writing back), the
/// language code is normalised, a missing title or author is `""`.
fn from_epub(epub: EpubMetadata) -> BookMetadata {
    BookMetadata {
        title: epub.title.unwrap_or_default(),
        author: epub.authors.join(" & "),
        subtitle: epub.subtitle,
        series: epub.series,
        series_index: epub.series_index,
        language: epub.language.map(|l| normalize_lang_code(&l)),
        description: epub.description,
        isbn: epub.isbn,
        rating: epub.rating,
        categories: epub.categories,
        goodreads_url: None,
    }
}

// ── PDF ───────────────────────────────────────────────────────────────────────

/// Title, author, subject (as the description) and keywords (as categories)
/// from the PDF at `read`'s Info dictionary. Falls back to the file name
/// (`named`'s) for the title.
fn extract_pdf_metadata(read: &Path, named: &Path) -> BookMetadata {
    let info = lopdf::Document::load(read)
        .ok()
        .map(|doc| pdf_info(&doc))
        .unwrap_or_default();
    let defaults = BookMetadata::default();
    BookMetadata {
        title: info.title.unwrap_or_else(|| title_from_filename(named)),
        author: info.author.unwrap_or(defaults.author),
        description: info
            .subject
            .map(|s| if s.contains('<') { strip_html(&s) } else { s }),
        categories: info
            .keywords
            .map(|k| {
                k.split([',', ';'])
                    .map(|c| c.trim().to_string())
                    .filter(|c| !c.is_empty())
                    .collect()
            })
            .unwrap_or_default(),
        ..defaults
    }
}

/// The text entries of a PDF Info dictionary that Librarium uses.
#[derive(Debug, Default, PartialEq)]
struct PdfInfo {
    title: Option<String>,
    author: Option<String>,
    subject: Option<String>,
    keywords: Option<String>,
}

/// Read the Info dictionary. Both the dictionary and each value may be
/// indirect references (`/Title 12 0 R`); they are followed.
fn pdf_info(doc: &lopdf::Document) -> PdfInfo {
    let info = doc
        .trailer
        .get(b"Info")
        .ok()
        .and_then(|o| doc.dereference(o).ok())
        .and_then(|(_, o)| o.as_dict().ok());
    let Some(info) = info else {
        return PdfInfo::default();
    };
    let text = |key: &[u8]| {
        let value = info.get(key).ok()?;
        let (_, value) = doc.dereference(value).ok()?;
        let decoded = decode_pdf_text(value.as_str().ok()?);
        let cleaned = decoded.replace('\0', "").trim().to_string();
        (!cleaned.is_empty()).then_some(cleaned)
    };
    PdfInfo {
        title: text(b"Title"),
        author: text(b"Author"),
        subject: text(b"Subject"),
        keywords: text(b"Keywords"),
    }
}

/// Decode a PDF text string (PDF 32000 §7.9.2.2): UTF-16BE after a `FE FF`
/// byte-order mark, UTF-8 after `EF BB BF` (PDF 2.0), otherwise
/// PDFDocEncoding (via lopdf). A little-endian `FF FE` mark isn't valid PDF
/// but some writers produce it, so it is honoured too. Invalid sequences
/// become U+FFFD rather than failing.
fn decode_pdf_text(bytes: &[u8]) -> String {
    let utf16 = |rest: &[u8], big_endian: bool| {
        // A trailing odd byte is dropped (`from_utf16_lossy` marks any
        // broken pair with U+FFFD).
        let units: Vec<u16> = rest
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&pair| {
                if big_endian {
                    u16::from_be_bytes(pair)
                } else {
                    u16::from_le_bytes(pair)
                }
            })
            .collect();
        String::from_utf16_lossy(&units)
    };
    if let Some(rest) = bytes.strip_prefix(b"\xFE\xFF") {
        utf16(rest, true)
    } else if let Some(rest) = bytes.strip_prefix(b"\xFF\xFE") {
        utf16(rest, false)
    } else if let Some(rest) = bytes.strip_prefix(b"\xEF\xBB\xBF") {
        String::from_utf8_lossy(rest).into_owned()
    } else {
        let literal = lopdf::Object::String(bytes.to_vec(), lopdf::StringFormat::Literal);
        lopdf::decode_text_string(&literal)
            .unwrap_or_else(|_| String::from_utf8_lossy(bytes).into_owned())
    }
}

// ── Languages ─────────────────────────────────────────────────────────────────

/// Normalise any language code to ISO 639-1 (2-letter).
/// Accepts ISO 639-1 (pass-through), ISO 639-3, and MARC 21 codes.
pub(crate) fn normalize_lang_code(code: &str) -> String {
    let lc = code.trim().to_lowercase();
    // Already a 2-letter ISO 639-1 code — pass through.
    if lc.len() == 2 && lc.chars().all(|c| c.is_ascii_alphabetic()) {
        return lc;
    }
    match lc.as_str() {
        // ISO 639-3 / MARC 21 → ISO 639-1
        "fra" | "fre" => "fr".into(),
        "deu" | "ger" => "de".into(),
        "nld" | "dut" => "nl".into(),
        "spa" => "es".into(),
        "ita" => "it".into(),
        "por" => "pt".into(),
        "rus" => "ru".into(),
        "jpn" => "ja".into(),
        "zho" | "chi" | "cmn" => "zh".into(),
        "ara" => "ar".into(),
        "kor" => "ko".into(),
        "pol" => "pl".into(),
        "swe" => "sv".into(),
        "nor" => "no".into(),
        "dan" => "da".into(),
        "fin" => "fi".into(),
        "ces" | "cze" => "cs".into(),
        "slk" | "slo" => "sk".into(),
        "ron" | "rum" => "ro".into(),
        "hye" | "arm" => "hy".into(),
        "mkd" | "mac" => "mk".into(),
        "tur" => "tr".into(),
        "heb" => "he".into(),
        "hin" => "hi".into(),
        "vie" => "vi".into(),
        "cat" => "ca".into(),
        "hun" => "hu".into(),
        "ell" | "gre" => "el".into(),
        "ukr" => "uk".into(),
        "ind" => "id".into(),
        "tha" => "th".into(),
        _ => lc,
    }
}

/// Detect the language of a text sample using `whatlang`.
/// Returns an ISO 639-1 (2-letter) code, or `None` when confidence is below 75%.
pub(crate) fn detect_text_language(text: &str) -> Option<String> {
    let info = whatlang::detect(text)?;
    if info.confidence() < 0.75 {
        return None;
    }
    Some(normalize_lang_code(info.lang().code()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::epub::fixtures;
    use lopdf::{dictionary, Object, StringFormat};

    #[test]
    fn epub_metadata_is_mapped_for_the_library() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dune.epub");
        fixtures::write_calibre_epub(&path);
        let meta = read_metadata(&path, &BookFormat::Epub);
        assert_eq!(meta.title, "Dune");
        assert_eq!(meta.author, "Frank Herbert");
        assert_eq!(meta.isbn.as_deref(), Some("9780441013593"));
        assert_eq!(meta.language.as_deref(), Some("en"));
        assert_eq!(
            meta.description.as_deref(),
            Some("Set on the desert planet Arrakis.\n\nA classic.")
        );
        assert_eq!(meta.series.as_deref(), Some("Dune"));
        assert_eq!(meta.series_index, Some(1.0));
        assert_eq!(meta.rating, Some(4));
        assert_eq!(meta.categories, vec!["Science Fiction", "Classics"]);
    }

    #[test]
    fn unknown_epub_titles_are_empty_and_unreadable_files_use_the_file_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("my-great_book.epub");
        let opf = r#"<package xmlns="http://www.idpf.org/2007/opf" version="2.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title> </dc:title><dc:language>fre</dc:language></metadata><manifest/><spine/></package>"#;
        fixtures::write_epub(&path, "content.opf", opf, &[]);
        let meta = read_metadata(&path, &BookFormat::Epub);
        assert_eq!(
            (meta.title.as_str(), meta.author.as_str()),
            ("", ""),
            "unknown, not a placeholder"
        );
        assert_eq!(meta.language.as_deref(), Some("fr"));

        std::fs::write(&path, b"not a zip").unwrap();
        assert_eq!(
            read_metadata(&path, &BookFormat::Epub).title,
            "my great book"
        );
        assert_eq!(
            read_metadata(&path, &BookFormat::Mobi).title,
            "my great book"
        );
    }

    #[test]
    fn several_authors_are_joined() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("b.epub");
        let opf = r#"<package xmlns="http://www.idpf.org/2007/opf" version="2.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>T</dc:title><dc:creator>A</dc:creator><dc:creator>B</dc:creator></metadata><manifest/><spine/></package>"#;
        fixtures::write_epub(&path, "content.opf", opf, &[]);
        assert_eq!(read_metadata(&path, &BookFormat::Epub).author, "A & B");
    }

    #[test]
    fn pdf_text_strings_are_decoded() {
        let utf16: Vec<u8> = [0xFE, 0xFF]
            .into_iter()
            .chain("Été – 東京".encode_utf16().flat_map(u16::to_be_bytes))
            .collect();
        assert_eq!(decode_pdf_text(&utf16), "Été – 東京");
        let utf16le: Vec<u8> = [0xFF, 0xFE]
            .into_iter()
            .chain("Ok".encode_utf16().flat_map(u16::to_le_bytes))
            .collect();
        assert_eq!(decode_pdf_text(&utf16le), "Ok");
        assert_eq!(decode_pdf_text(b"\xEF\xBB\xBFcaf\xC3\xA9"), "café");
        // PDFDocEncoding: 0xE9 is é (as in Latin-1), 0x84 is an em dash.
        assert_eq!(decode_pdf_text(b"Caf\xE9 \x84 ok"), "Café — ok");
        // An odd number of UTF-16 bytes or a lone surrogate doesn't fail.
        assert!(decode_pdf_text(&[0xFE, 0xFF, 0xD8, 0x00, 0x00]).contains('\u{FFFD}'));
    }

    #[test]
    fn pdf_info_follows_indirect_references() {
        let mut doc = lopdf::Document::with_version("1.5");
        let title_bytes: Vec<u8> = [0xFE, 0xFF]
            .into_iter()
            .chain("Les Misérables".encode_utf16().flat_map(u16::to_be_bytes))
            .collect();
        let author_id = doc.add_object(Object::String(
            b"Victor Hugo".to_vec(),
            StringFormat::Literal,
        ));
        let info_id = doc.add_object(dictionary! {
            "Title" => Object::String(title_bytes, StringFormat::Hexadecimal),
            "Author" => Object::Reference(author_id),
            "Keywords" => Object::String(b"Classics; France".to_vec(), StringFormat::Literal),
            "Subject" => Object::String(b"   ".to_vec(), StringFormat::Literal),
        });
        let name = |n: &str| Object::Name(n.as_bytes().to_vec());
        let pages_id = doc.add_object(
            dictionary! { "Type" => name("Pages"), "Kids" => Object::Array(vec![]), "Count" => 0 },
        );
        let catalog_id =
            doc.add_object(dictionary! { "Type" => name("Catalog"), "Pages" => pages_id });
        doc.trailer.set("Root", catalog_id);
        doc.trailer.set("Info", info_id);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hugo.pdf");
        doc.save(&path).unwrap();

        let meta = read_metadata(&path, &BookFormat::Pdf);
        assert_eq!(meta.title, "Les Misérables");
        assert_eq!(meta.author, "Victor Hugo");
        assert_eq!(meta.categories, vec!["Classics", "France"]);
        assert_eq!(meta.description, None, "a blank subject is no description");
    }

    #[test]
    fn unreadable_pdf_falls_back_to_the_file_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("some_report.pdf");
        std::fs::write(&path, b"%PDF-1.4 garbage").unwrap();
        let meta = read_metadata(&path, &BookFormat::Pdf);
        assert_eq!(meta.title, "some report");
        assert_eq!(meta.author, "", "unknown, not a placeholder");
    }

    #[test]
    fn language_codes_are_normalised() {
        assert_eq!(normalize_lang_code("fre"), "fr");
        assert_eq!(normalize_lang_code(" EN "), "en");
        assert_eq!(normalize_lang_code("pt-BR"), "pt-br");
    }
}
