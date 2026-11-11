//! Rewriting EPUB files: metadata write-back, cover embedding and removal,
//! and the library id in copies sent to e-readers.
//!
//! What changes in the OPF is decided by [`crate::epub`] (a real XML parser,
//! edits spliced into the original text, every result re-parsed and
//! checked). This module applies those changes to the file. Because it
//! modifies the user's books, each rewrite:
//! - holds the EPUB's lock ([`fsutil::with_file_lock`]) from read to replace,
//!   so a metadata save and a cover change on the same book can't interleave
//!   and silently drop one another;
//! - writes the new archive to a temp file next to the original, re-opens it
//!   and checks it (same entries in the same order, the OPF found through
//!   `container.xml` parses, the unique identifier unchanged), and only then
//!   renames it over the original ([`fsutil::atomic_write_with`]). Any
//!   failure — unreadable archive, malformed OPF, full disk, crash — leaves
//!   the original file untouched;
//! - copies untouched entries as-is (`raw_copy_file`: no recompression, the
//!   stored `mimetype` entry stays first).

use std::fs;
use std::io::{self, Cursor, Seek, Write};
use std::path::Path;

use zip::write::SimpleFileOptions;
use zip::ZipArchive;

use super::sniff::ImageKind;
use crate::epub::archive::{locate_opf, read_entry};
use crate::epub::{cover, edit, identity, EpubArchive, Locate, MetadataEdit, Opf};
use crate::fsutil;
use crate::models::book::BookMetadata;

// ── Public operations ─────────────────────────────────────────────────────────

/// Write the library's `metadata` into the EPUB at `path`, without removing
/// anything: every known value is written (title, authors, language,
/// description, categories, series, ISBN, rating), empty fields leave the
/// file's value. Used for freshly converted books, whose file may carry
/// metadata the library never read. Returns whether the file changed.
pub(crate) fn write_metadata(path: &Path, metadata: &BookMetadata) -> Result<bool, String> {
    write_edit(path, &MetadataEdit::overlay(metadata))
}

/// Write an edit of a book's metadata (`before` → `after`) into the EPUB at
/// `path`: the fields that changed are written, fields that were cleared are
/// removed from the file, and fields the user didn't touch stay exactly as
/// the file has them ([`MetadataEdit::changes`]). Returns whether the file
/// changed; an EPUB that already says all this is not rewritten.
pub(crate) fn write_metadata_changes(
    path: &Path,
    before: &BookMetadata,
    after: &BookMetadata,
) -> Result<bool, String> {
    write_edit(path, &MetadataEdit::changes(before, after))
}

fn write_edit(path: &Path, metadata_edit: &MetadataEdit) -> Result<bool, String> {
    if metadata_edit.is_empty() {
        return Ok(false);
    }
    fsutil::with_file_lock(path, || {
        let epub = load(path)?;
        let Some(new_opf) = edit::apply_edit(epub.opf.text(), &epub.opf.package, metadata_edit)?
        else {
            return Ok(false);
        };
        let opf_bytes = epub.opf.xml.encode(&new_opf);
        let edits = ZipEdits {
            replace: Some((&epub.opf.path, &opf_bytes)),
            ..Default::default()
        };
        rewrite(path, &epub, &edits)?;
        Ok(true)
    })
}

/// Embed `image` as the cover of the EPUB at `path` ([`cover::set_cover`]):
/// - if the OPF declares a cover, that image is replaced and its manifest
///   `media-type` updated to the new image's type;
/// - otherwise the image is added under a free name (`cover.jpg`,
///   `cover-1.jpg`…) and declared both the EPUB 3 way (in EPUB 3 packages)
///   and the EPUB 2 way.
///
/// Images the OPF doesn't declare as the cover are never overwritten.
pub(crate) fn embed_cover(path: &Path, image: &[u8], kind: ImageKind) -> Result<(), String> {
    fsutil::with_file_lock(path, || {
        let epub = load(path)?;
        let exists = |p: &str| epub.entry_name(p).is_some();
        let update = cover::set_cover(
            epub.opf.text(),
            &epub.opf.package,
            &epub.opf.dir,
            kind.mime(),
            kind.extension(),
            &exists,
        )?;
        // Replace the entry under its stored spelling (case may differ).
        let image_entry = epub
            .entry_name(&update.image_path)
            .unwrap_or(update.image_path);
        let opf_bytes = epub.opf.xml.encode(&update.opf);
        let edits = ZipEdits {
            replace: Some((&epub.opf.path, &opf_bytes)),
            // `add` replaces an existing entry of the same name, and also
            // repairs EPUBs whose declared cover file is missing.
            add: Some((&image_entry, image)),
            ..Default::default()
        };
        rewrite(path, &epub, &edits)
    })
}

/// Remove the declared cover from the EPUB at `path` ([`cover::remove_cover`]):
/// its declarations, and the image file itself when nothing else in the
/// book refers to it (a cover page usually shows the same image, which then
/// stays). Returns whether the file changed; a book without a declared cover
/// is left alone (an image that merely looks like a cover is not removed).
pub(crate) fn strip_cover(path: &Path) -> Result<bool, String> {
    fsutil::with_file_lock(path, || {
        let epub = load(path)?;
        let Some(removal) = cover::remove_cover(epub.opf.text(), &epub.opf.package, &epub.opf.dir)?
        else {
            return Ok(false);
        };
        let mut remove = Vec::new();
        for image in &removal.unlisted_images {
            if let Some(name) = epub.entry_name(image) {
                if !epub.is_referenced(&name, &removal.opf) {
                    remove.push(name);
                }
            }
        }
        let opf_bytes = epub.opf.xml.encode(&removal.opf);
        let edits = ZipEdits {
            replace: Some((&epub.opf.path, &opf_bytes)),
            remove: &remove,
            ..Default::default()
        };
        rewrite(path, &epub, &edits)?;
        Ok(true)
    })
}

/// Make the EPUB at `path` carry the library book id `book_id` as an extra
/// `urn:librarium:` identifier ([`identity::set_library_id`]). Only ever
/// called on the temporary copy of a book about to be sent to an e-reader,
/// never on a library file. Returns whether the file changed (it already
/// carries the id otherwise). Same safety as every rewrite here: the
/// archive is verified before it replaces the file, and a failure leaves
/// the file as it was.
pub(crate) fn write_library_id(path: &Path, book_id: &str) -> Result<bool, String> {
    fsutil::with_file_lock(path, || {
        let epub = load(path)?;
        let Some(new_opf) = identity::set_library_id(epub.opf.text(), &epub.opf.package, book_id)?
        else {
            return Ok(false);
        };
        let opf_bytes = epub.opf.xml.encode(&new_opf);
        let edits = ZipEdits {
            replace: Some((&epub.opf.path, &opf_bytes)),
            ..Default::default()
        };
        rewrite(path, &epub, &edits)?;
        Ok(true)
    })
}

// ── Reading and rewriting the archive ─────────────────────────────────────────

/// An EPUB loaded in memory, with its OPF located (strictly, through
/// `container.xml`) and parsed.
struct LoadedEpub {
    bytes: Vec<u8>,
    /// Names of all archive entries, in archive order.
    names: Vec<String>,
    opf: Opf,
}

/// Largest content document scanned when checking whether an image is still
/// used; a bigger one counts as "uses it" (keep the image).
const MAX_SCANNED_BYTES: u64 = 16 << 20;

impl LoadedEpub {
    /// The stored name of the entry at `path` (exact, else case-insensitive).
    fn entry_name(&self, path: &str) -> Option<String> {
        self.names
            .iter()
            .find(|n| *n == path)
            .or_else(|| self.names.iter().find(|n| n.eq_ignore_ascii_case(path)))
            .cloned()
    }

    /// Whether anything other than the image entry `image` itself — the new
    /// OPF `new_opf`, content documents, style sheets, the NCX — mentions its
    /// file name. Deliberately generous: a false "yes" only keeps a file.
    fn is_referenced(&self, image: &str, new_opf: &str) -> bool {
        let file_name = image
            .rsplit('/')
            .next()
            .unwrap_or(image)
            .to_ascii_lowercase();
        let needles = [file_name.clone(), percent_encode(&file_name)];
        let mentions = |haystack: &[u8]| {
            let haystack = String::from_utf8_lossy(haystack).to_ascii_lowercase();
            needles.iter().any(|n| haystack.contains(n.as_str()))
        };
        if mentions(new_opf.as_bytes()) {
            return true;
        }
        let Ok(mut zip) = ZipArchive::new(Cursor::new(self.bytes.as_slice())) else {
            return true;
        };
        let textual = [
            "xhtml", "html", "htm", "xml", "svg", "css", "ncx", "smil", "xpgt",
        ];
        for name in &self.names {
            let extension = name
                .rsplit_once('.')
                .map(|(_, e)| e.to_ascii_lowercase())
                .unwrap_or_default();
            if name == image || *name == self.opf.path || !textual.contains(&extension.as_str()) {
                continue;
            }
            match read_entry(&mut zip, name, MAX_SCANNED_BYTES) {
                Ok(bytes) if !mentions(&bytes) => {}
                _ => return true,
            }
        }
        false
    }
}

/// `s` with the characters a URL can't contain as-is percent-encoded (how an
/// href may spell a file name with spaces or accents).
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~!$&'()*+,;=:@".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02x}"));
        }
    }
    out
}

fn load(path: &Path) -> Result<LoadedEpub, String> {
    let bytes = fs::read(path).map_err(|e| format!("Could not read {}: {e}", path.display()))?;
    let (names, opf) = {
        let mut epub = EpubArchive::new(Cursor::new(bytes.as_slice()), Locate::Strict)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let names = epub
            .zip()
            .file_names()
            .map(|name| name.map(|n| n.into_owned()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("{}: an entry name can't be read ({e})", path.display()))?;
        (names, epub.opf().clone())
    };
    Ok(LoadedEpub { bytes, names, opf })
}

/// Changes applied while copying an EPUB archive entry by entry.
#[derive(Default)]
struct ZipEdits<'a> {
    /// Entry whose content is replaced (keeping its position and compression).
    replace: Option<(&'a str, &'a [u8])>,
    /// Entries to drop.
    remove: &'a [String],
    /// Entry written at the end, replacing any entry of the same name.
    /// Stored uncompressed: it is an image, already compressed.
    add: Option<(&'a str, &'a [u8])>,
}

impl ZipEdits<'_> {
    /// The entry names the rewritten archive must have, in order.
    fn expected_names(&self, names: &[String]) -> Vec<String> {
        let added = self.add.map(|(n, _)| n);
        let mut expected: Vec<String> = names
            .iter()
            .filter(|n| !self.remove.contains(n) && Some(n.as_str()) != added)
            .cloned()
            .collect();
        expected.extend(added.map(str::to_string));
        expected
    }
}

/// Copy `original` into `out`, applying `edits`.
fn write_archive<W: Write + Seek>(
    original: &[u8],
    edits: &ZipEdits,
    out: W,
) -> zip::result::ZipResult<W> {
    let mut archive = ZipArchive::new(Cursor::new(original))?;
    let mut writer = zip::ZipWriter::new(out);
    for i in 0..archive.len() {
        let entry = archive.by_index_raw(i)?;
        let name = entry.name()?.into_owned();
        if edits.remove.contains(&name) || edits.add.is_some_and(|(n, _)| n == name) {
            continue;
        }
        match edits.replace {
            Some((replaced, content)) if replaced == name => {
                let options = SimpleFileOptions::default().compression_method(entry.compression());
                drop(entry);
                writer.start_file(name, options)?;
                writer.write_all(content)?;
            }
            _ => writer.raw_copy_file(entry)?,
        }
    }
    if let Some((name, content)) = edits.add {
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        writer.start_file(name, options)?;
        writer.write_all(content)?;
    }
    writer.finish()
}

/// Check a freshly written archive before it replaces the original: it
/// opens, has exactly `expected_names` in that order, its OPF (found through
/// `container.xml`) is `opf.path` and parses, and the unique identifier
/// still says the same.
fn verify_archive(file: &mut fs::File, expected_names: &[String], opf: &Opf) -> io::Result<()> {
    let invalid = |what: String| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("the rewritten EPUB {what}"),
        )
    };
    file.seek(io::SeekFrom::Start(0))?;
    let mut archive =
        ZipArchive::new(&mut *file).map_err(|e| invalid(format!("doesn't open: {e}")))?;
    let names = archive
        .file_names()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| invalid(format!("has an unreadable entry name: {e}")))?;
    if !names
        .iter()
        .map(|n| n.as_ref())
        .eq(expected_names.iter().map(String::as_str))
    {
        return Err(invalid("doesn't have the expected entries".into()));
    }
    drop(names);
    let new_opf = locate_opf(&mut archive, Locate::Strict)
        .map_err(|e| invalid(format!("has an unreadable OPF: {e}")))?;
    let unique_id = |o: &Opf| {
        o.package
            .unique_identifier_entry()
            .map(|i| o.package.metadata[i].text.clone())
    };
    if new_opf.path != opf.path || unique_id(&new_opf) != unique_id(opf) {
        return Err(invalid("lost its OPF or unique identifier".into()));
    }
    Ok(())
}

/// Atomically replace the EPUB at `path` with `epub` + `edits`.
/// The caller must hold `path`'s lock (taken by the public operations).
fn rewrite(path: &Path, epub: &LoadedEpub, edits: &ZipEdits) -> Result<(), String> {
    let expected = edits.expected_names(&epub.names);
    fsutil::atomic_write_with(path, |file| {
        write_archive(&epub.bytes, edits, &mut *file).map_err(io::Error::other)?;
        verify_archive(file, &expected, &epub.opf)
    })
    .map_err(|e| format!("Could not rewrite {}: {e}", path.display()))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::library::test_support::{
        entry_names, epub_with_cover, read_entry_bytes, write_epub, OPF_PATH,
    };
    use crate::epub::fixtures::{self, JPEG, PNG};

    fn meta(title: &str, author: &str) -> BookMetadata {
        BookMetadata {
            title: title.into(),
            author: author.into(),
            ..Default::default()
        }
    }

    fn opf_of(path: &Path) -> String {
        String::from_utf8(read_entry_bytes(path, OPF_PATH).unwrap()).unwrap()
    }

    fn cover_of(path: &Path) -> Option<Vec<u8>> {
        crate::epub::read_cover_image(path).map(|(bytes, _)| bytes)
    }

    #[test]
    fn metadata_is_written_and_the_unique_identifier_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("book.epub");
        write_epub(&path, "Old title", "Old author");
        let chapter_before = read_entry_bytes(&path, "OEBPS/ch1.xhtml").unwrap();

        let mut m = meta("Dune & Co", "Frank Herbert");
        m.isbn = Some("978-0-441-01359-3".into());
        m.series = Some("Dune".into());
        m.series_index = Some(1.0);
        assert!(write_metadata(&path, &m).unwrap());

        let opf = opf_of(&path);
        assert!(opf.contains("<dc:title>Dune &amp; Co</dc:title>"), "{opf}");
        assert!(
            opf.contains("<dc:creator opf:role=\"aut\">Frank Herbert</dc:creator>"),
            "{opf}"
        );
        assert!(!opf.contains("Old author"));
        assert!(opf.contains("name=\"calibre:series\" content=\"Dune\""));
        assert!(
            opf.contains("<dc:identifier id=\"BookId\">urn:uuid:librarium-test</dc:identifier>"),
            "unique identifier kept"
        );
        assert!(
            opf.contains("<dc:identifier opf:scheme=\"ISBN\">9780441013593</dc:identifier>"),
            "ISBN added: {opf}"
        );
        assert_eq!(
            entry_names(&path)[0],
            "mimetype",
            "mimetype stays the first entry"
        );
        assert_eq!(
            read_entry_bytes(&path, "OEBPS/ch1.xhtml").unwrap(),
            chapter_before
        );

        let read = crate::epub::read_metadata(&path).unwrap();
        assert_eq!(read.title.as_deref(), Some("Dune & Co"));
        assert_eq!(read.isbn.as_deref(), Some("9780441013593"));
    }

    #[test]
    fn unchanged_metadata_does_not_rewrite_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("book.epub");
        write_epub(&path, "Dune", "Frank Herbert");
        let before = fs::read(&path).unwrap();
        assert!(!write_metadata(&path, &meta("Dune", "Frank Herbert")).unwrap());
        assert!(!write_metadata_changes(&path, &meta("Dune", "A"), &meta("Dune", "A")).unwrap());
        assert_eq!(fs::read(&path).unwrap(), before);
    }

    #[test]
    fn edits_write_changes_and_remove_cleared_fields_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dune.epub");
        fixtures::write_calibre_epub(&path);
        let untouched = ["titlepage.xhtml", "cover.jpeg", "images/map-cover.jpg"]
            .map(|n| fixtures::entry(&path, n));

        // What an index without a rating knows.
        let before = BookMetadata {
            title: "Dune".into(),
            author: "Frank Herbert".into(),
            description: Some("Set on the desert planet Arrakis. A classic.".into()),
            series: Some("Dune".into()),
            series_index: Some(1.0),
            isbn: Some("9780441013593".into()),
            categories: vec!["Science Fiction".into(), "Classics".into()],
            language: Some("en".into()),
            ..Default::default()
        };
        let after = BookMetadata {
            title: "Dune (40th anniversary)".into(),
            description: None,
            series_index: Some(2.0),
            ..before.clone()
        };
        assert!(write_metadata_changes(&path, &before, &after).unwrap());

        let opf = fixtures::entry_text(&path, "content.opf");
        assert!(opf.contains("<dc:title>Dune (40th anniversary)</dc:title>"));
        assert!(
            !opf.contains("dc:description"),
            "the cleared description is removed: {opf}"
        );
        assert!(opf.contains("<meta name=\"calibre:series_index\" content=\"2\"/>"));
        assert!(
            opf.contains("<meta name=\"calibre:rating\" content=\"8.0\"/>"),
            "the rating the index never knew survives"
        );
        assert!(opf.contains(
            r#"<dc:creator opf:file-as="Herbert, Frank" opf:role="aut">Frank Herbert</dc:creator>"#
        ));
        assert!(opf.contains(r#"<dc:identifier id="uuid_id" opf:scheme="uuid">5f2b1c3e-6e0a-4c1b-9a43-3f1f3c0f9b11</dc:identifier>"#));
        for (name, bytes) in ["titlepage.xhtml", "cover.jpeg", "images/map-cover.jpg"]
            .iter()
            .zip(untouched)
        {
            assert_eq!(fixtures::entry(&path, name), bytes, "{name} copied as-is");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_rewrite_leaves_the_original_intact() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let books = dir.path().join("books");
        fs::create_dir(&books).unwrap();
        let path = books.join("book.epub");
        write_epub(&path, "Dune", "Frank Herbert");
        let before = fs::read(&path).unwrap();

        // A read-only folder: the temp file can't be created (like a full
        // or read-only share).
        fs::set_permissions(&books, fs::Permissions::from_mode(0o555)).unwrap();
        let result = write_metadata(&path, &meta("Changed", "Someone"));
        fs::set_permissions(&books, fs::Permissions::from_mode(0o755)).unwrap();

        assert!(result.is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(
            fs::read_dir(&books).unwrap().count(),
            1,
            "no temp file left behind"
        );
    }

    #[test]
    fn not_an_epub_is_an_error_and_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fake.epub");
        fs::write(&path, b"<html>not a zip</html>").unwrap();
        assert!(write_metadata(&path, &meta("a", "b")).is_err());
        assert!(embed_cover(&path, JPEG, ImageKind::Jpeg).is_err());
        assert!(strip_cover(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"<html>not a zip</html>");
    }

    #[test]
    fn a_malformed_opf_is_an_error_and_the_file_is_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("broken.epub");
        let broken = fixtures::CALIBRE_OPF.replace("</dc:title>", "</dc:titel>");
        fixtures::write_epub(&path, "content.opf", &broken, &[("cover.jpeg", JPEG)]);
        let before = fs::read(&path).unwrap();

        let err = write_metadata(&path, &meta("New", "Someone")).unwrap_err();
        assert!(err.contains("not well-formed"), "{err}");
        assert!(embed_cover(&path, PNG, ImageKind::Png).is_err());
        assert!(strip_cover(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(
            fs::read_dir(dir.path()).unwrap().count(),
            1,
            "no temp file left behind"
        );
    }

    #[test]
    fn a_missing_container_is_an_error_for_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("no-container.epub");
        fs::write(
            &path,
            fixtures::epub_bytes(&[("content.opf", fixtures::CALIBRE_OPF.as_bytes())]),
        )
        .unwrap();
        let before = fs::read(&path).unwrap();
        assert!(write_metadata(&path, &meta("New", "Someone")).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        // Reading still works.
        assert_eq!(
            crate::epub::read_metadata(&path).unwrap().title.as_deref(),
            Some("Dune")
        );
    }

    #[test]
    fn embedding_a_cover_into_a_book_without_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("book.epub");
        write_epub(&path, "Dune", "Frank Herbert");
        embed_cover(&path, JPEG, ImageKind::Jpeg).unwrap();

        let opf = opf_of(&path);
        assert!(
            opf.contains(r#"<item id="cover-image" href="cover.jpg" media-type="image/jpeg"/>"#),
            "{opf}"
        );
        assert!(opf.contains("<meta name=\"cover\" content=\"cover-image\"/>"));
        assert!(
            !opf.contains("properties="),
            "no EPUB 3 attribute in an EPUB 2 package"
        );
        assert_eq!(cover_of(&path).unwrap(), JPEG);
    }

    #[test]
    fn replacing_a_cover_updates_its_media_type() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("book.epub");
        epub_with_cover(&path, JPEG);
        embed_cover(&path, PNG, ImageKind::Png).unwrap();

        let opf = opf_of(&path);
        assert!(
            opf.contains("href=\"images/cover.jpg\" media-type=\"image/png\""),
            "{opf}"
        );
        assert_eq!(cover_of(&path).unwrap(), PNG);
        assert_eq!(
            entry_names(&path)
                .iter()
                .filter(|n| n.ends_with("cover.jpg"))
                .count(),
            1
        );
    }

    #[test]
    fn an_undeclared_cover_jpg_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("book.epub");
        crate::commands::library::test_support::write_epub_with(
            &path,
            &crate::commands::library::test_support::opf("Dune", "Frank Herbert", "", ""),
            &[("OEBPS/cover.jpg", b"an illustration")],
        );
        embed_cover(&path, JPEG, ImageKind::Jpeg).unwrap();
        assert_eq!(
            read_entry_bytes(&path, "OEBPS/cover.jpg").unwrap(),
            b"an illustration"
        );
        assert_eq!(read_entry_bytes(&path, "OEBPS/cover-1.jpg").unwrap(), JPEG);
    }

    #[test]
    fn epub3_cover_replacement_keeps_interior_images() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("foundation.epub");
        fixtures::write_epub3(&path);
        let illustration = fixtures::entry(&path, "OEBPS/images/cover-illustration.jpg");

        embed_cover(&path, JPEG, ImageKind::Jpeg).unwrap();
        assert_eq!(
            fixtures::entry(&path, "OEBPS/images/cover.png").as_deref(),
            Some(JPEG)
        );
        assert_eq!(
            fixtures::entry(&path, "OEBPS/images/cover-illustration.jpg"),
            illustration
        );
        let opf = fixtures::entry_text(&path, "OEBPS/package.opf");
        assert!(opf.contains(r#"<item id="cover-img" href="images/cover.png" media-type="image/jpeg" properties="cover-image"/>"#), "{opf}");

        assert!(strip_cover(&path).unwrap());
        let opf = fixtures::entry_text(&path, "OEBPS/package.opf");
        assert!(
            !opf.contains("cover-img") && !opf.contains("name=\"cover\""),
            "{opf}"
        );
        assert!(
            fixtures::entry(&path, "OEBPS/images/cover.png").is_none(),
            "an unreferenced cover image is deleted"
        );
        assert_eq!(
            fixtures::entry(&path, "OEBPS/images/cover-illustration.jpg"),
            illustration,
            "the illustration stays"
        );
        assert!(opf.contains(
            r#"<item id="ill" href="images/cover-illustration.jpg" media-type="image/jpeg"/>"#
        ));
        // The illustration may *look* like a cover, but it isn't declared:
        // a second removal changes nothing.
        let before = fs::read(&path).unwrap();
        assert!(!strip_cover(&path).unwrap());
        assert_eq!(fs::read(&path).unwrap(), before);
    }

    #[test]
    fn stripping_the_cover_removes_declarations_and_image() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("book.epub");
        epub_with_cover(&path, JPEG);
        assert!(strip_cover(&path).unwrap());

        let opf = opf_of(&path);
        assert!(
            !opf.contains("cover-image") && !opf.contains("name=\"cover\""),
            "{opf}"
        );
        assert!(cover_of(&path).is_none());
        assert!(!entry_names(&path).iter().any(|n| n.ends_with("cover.jpg")));
        crate::epub::read_metadata(&path).expect("still a readable EPUB");
        // Nothing left to strip.
        assert!(!strip_cover(&path).unwrap());
    }

    #[test]
    fn a_cover_shown_by_a_cover_page_is_kept_when_stripped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dune.epub");
        fixtures::write_calibre_epub(&path);
        assert!(strip_cover(&path).unwrap());

        let opf = fixtures::entry_text(&path, "content.opf");
        assert!(
            !opf.contains(r#"id="cover""#) && !opf.contains(r#"name="cover""#),
            "{opf}"
        );
        assert!(
            opf.contains(r#"<reference href="titlepage.xhtml" title="Cover" type="cover"/>"#),
            "the cover page stays"
        );
        assert_eq!(
            fixtures::entry(&path, "cover.jpeg").as_deref(),
            Some(JPEG),
            "titlepage.xhtml still shows it"
        );
        assert!(fixtures::entry(&path, "images/map-cover.jpg").is_some());
    }

    /// Run every rewrite on *copies* of real books: point
    /// `LIBRARIUM_SAMPLE_EPUBS` at a folder and run
    /// `cargo test real_books -- --ignored --nocapture`. The originals are
    /// only read. Each copy must survive a metadata edit, a cover change and
    /// a cover removal with all untouched entries byte-identical.
    #[test]
    #[ignore = "needs LIBRARIUM_SAMPLE_EPUBS=<folder of EPUBs>"]
    fn real_books() {
        let Ok(folder) = std::env::var("LIBRARIUM_SAMPLE_EPUBS") else {
            return;
        };
        let scratch = tempfile::tempdir().unwrap();
        let mut failures = Vec::new();
        let books: Vec<_> = walkdir::WalkDir::new(&folder)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| {
                e.path()
                    .extension()
                    .is_some_and(|x| x.eq_ignore_ascii_case("epub"))
            })
            .collect();
        for (n, entry) in books.iter().enumerate() {
            let copy = scratch.path().join(format!("{n}.epub"));
            fs::copy(entry.path(), &copy).unwrap();
            let outcome = std::panic::catch_unwind(|| check_real_book(&copy));
            let name = entry.path().display().to_string();
            match outcome {
                Ok(Ok(summary)) => println!("ok   {name}: {summary}"),
                Ok(Err(e)) => failures.push(format!("{name}: {e}")),
                Err(_) => failures.push(format!("{name}: panicked")),
            }
        }
        assert!(
            failures.is_empty(),
            "{} of {} books failed:\n{}",
            failures.len(),
            books.len(),
            failures.join("\n")
        );
    }

    fn check_real_book(path: &Path) -> Result<String, String> {
        let entries = |p: &Path| -> Vec<(String, Vec<u8>)> {
            let mut zip = ZipArchive::new(fs::File::open(p).unwrap()).unwrap();
            let names: Vec<String> = zip.file_names().map(|n| n.unwrap().into_owned()).collect();
            names
                .into_iter()
                .map(|n| {
                    let b = read_entry(&mut zip, &n, 1 << 32).unwrap();
                    (n, b)
                })
                .collect()
        };
        let before = crate::epub::read_metadata(path)?;
        let opf_path = load(path)?.opf.path;
        let original = entries(path);

        let old = fixtures::book_metadata(&before);
        let new = BookMetadata {
            title: format!("{} (edited)", old.title),
            series: Some("Librarium test".into()),
            series_index: Some(3.0),
            categories: vec!["Tested".into()],
            rating: Some(4),
            isbn: Some("9780441013593".into()),
            ..old.clone()
        };
        write_metadata_changes(path, &old, &new)?;
        let after = crate::epub::read_metadata(path)?;
        if after.title.as_deref() != Some(new.title.as_str())
            || after.series_index != Some(3.0)
            || after.rating != Some(4)
        {
            return Err(format!("edit not read back: {after:?}"));
        }
        if after.authors != before.authors
            || after.language != before.language
            || after.description != before.description
        {
            return Err("an untouched field changed".into());
        }
        let edited = entries(path);
        if original
            .iter()
            .filter(|(n, _)| *n != opf_path)
            .ne(edited.iter().filter(|(n, _)| *n != opf_path))
        {
            return Err("an entry other than the OPF changed".into());
        }

        embed_cover(path, PNG, ImageKind::Png)?;
        if cover_of(path).as_deref() != Some(PNG) {
            return Err("the new cover isn't the one read back".into());
        }
        strip_cover(path)?;
        let stripped = crate::epub::read_metadata(path)?;
        if stripped.title != after.title {
            return Err("cover removal changed the metadata".into());
        }
        Ok(format!(
            "{:?} by {:?}",
            before.title.unwrap_or_default(),
            before.authors
        ))
    }

    #[test]
    fn single_line_opfs_are_edited_too() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("book.epub");
        epub_with_cover(&path, JPEG);
        // Re-pack with the OPF on one line (common in generated EPUBs).
        let one_line = opf_of(&path).replace('\n', "");
        crate::commands::library::test_support::write_epub_with(
            &path,
            &one_line,
            &[("OEBPS/images/cover.jpg", JPEG)],
        );

        assert!(strip_cover(&path).unwrap());
        let opf = opf_of(&path);
        assert!(!opf.contains('\n'), "still on one line");
        assert!(
            !opf.contains("cover-image") && !opf.contains("name=\"cover\""),
            "{opf}"
        );
        assert!(read_entry_bytes(&path, "OEBPS/images/cover.jpg").is_none());

        let mut m = meta("Dune", "Frank Herbert");
        m.categories = vec!["SF".into()];
        assert!(write_metadata(&path, &m).unwrap());
        let opf = opf_of(&path);
        assert!(
            opf.contains("<dc:language>en</dc:language><dc:subject>SF</dc:subject>"),
            "{opf}"
        );
    }

    #[test]
    fn the_library_id_is_added_to_a_copy_and_nothing_else_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("copy.epub");
        epub_with_cover(&path, JPEG);
        let names_before = entry_names(&path);
        let chapter_before = read_entry_bytes(&path, "OEBPS/ch1.xhtml").unwrap();
        let meta_before = crate::epub::read_metadata(&path).unwrap();

        assert!(write_library_id(&path, "3f2b1c3e-6e0a-4c1b-9a43-3f1f3c0f9b11").unwrap());
        let opf = opf_of(&path);
        assert!(opf.contains(r#"<dc:identifier id="librarium-id">urn:librarium:3f2b1c3e-6e0a-4c1b-9a43-3f1f3c0f9b11</dc:identifier>"#), "{opf}");
        assert!(
            opf.contains(r#"<dc:identifier id="BookId">urn:uuid:librarium-test</dc:identifier>"#),
            "unique identifier kept"
        );
        assert_eq!(
            entry_names(&path),
            names_before,
            "same entries, same order (mimetype first)"
        );
        assert_eq!(
            read_entry_bytes(&path, "OEBPS/ch1.xhtml").unwrap(),
            chapter_before
        );
        assert_eq!(crate::epub::read_metadata(&path).unwrap(), meta_before);
        assert!(
            !write_library_id(&path, "3f2b1c3e-6e0a-4c1b-9a43-3f1f3c0f9b11").unwrap(),
            "already there: not rewritten"
        );
    }

    #[test]
    fn a_broken_copy_is_left_as_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("broken.epub");
        fs::write(&path, b"PK\x03\x04 not really a zip").unwrap();
        assert!(write_library_id(&path, "abc").is_err());
        assert_eq!(fs::read(&path).unwrap(), b"PK\x03\x04 not really a zip");
    }
}
