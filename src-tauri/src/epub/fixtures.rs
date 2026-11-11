//! Test fixtures: EPUBs and OPFs modelled on what real tools produce
//! (Calibre, Sigil/EPUB 3, hand-made single-line files), built in code with
//! the `zip` crate. Compiled for tests only.

use std::io::{Cursor, Read, Write};
use std::path::Path;

use zip::write::SimpleFileOptions;
use zip::CompressionMethod;

use crate::models::book::BookMetadata;

use super::metadata::EpubMetadata;

/// `container.xml` pointing at `opf_path`.
pub(crate) fn container(opf_path: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="{opf_path}" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>
"#
    )
}

/// An EPUB archive: a stored `mimetype` first, then `entries` (deflated).
pub(crate) fn epub_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    zip.start_file("mimetype", stored).unwrap();
    zip.write_all(b"application/epub+zip").unwrap();
    for (name, content) in entries {
        zip.start_file(*name, deflated).unwrap();
        zip.write_all(content).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

/// Write an EPUB whose OPF is `opf` at `opf_path`, plus `extra` entries.
pub(crate) fn write_epub(path: &Path, opf_path: &str, opf: &str, extra: &[(&str, &[u8])]) {
    let container = container(opf_path);
    let mut entries: Vec<(&str, &[u8])> = vec![
        ("META-INF/container.xml", container.as_bytes()),
        (opf_path, opf.as_bytes()),
    ];
    entries.extend_from_slice(extra);
    std::fs::write(path, epub_bytes(&entries)).unwrap();
}

/// Content of one archive entry, if present.
pub(crate) fn entry(path: &Path, name: &str) -> Option<Vec<u8>> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(path).ok()?).ok()?;
    let mut file = archive.by_name(name).ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

/// An entry as text.
pub(crate) fn entry_text(path: &Path, name: &str) -> String {
    String::from_utf8(entry(path, name).unwrap_or_else(|| panic!("{name} missing"))).unwrap()
}

/// `meta` as the library would store it (the reverse of the library's
/// mapping, minus language normalisation).
pub(crate) fn book_metadata(meta: &EpubMetadata) -> BookMetadata {
    BookMetadata {
        title: meta.title.clone().unwrap_or_default(),
        author: meta.authors.join(" & "),
        subtitle: meta.subtitle.clone(),
        series: meta.series.clone(),
        series_index: meta.series_index,
        language: meta.language.clone(),
        description: meta.description.clone(),
        isbn: meta.isbn.clone(),
        rating: meta.rating,
        categories: meta.categories.clone(),
        goodreads_url: None,
    }
}

pub(crate) const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 0, 0x10, b'J', b'F', b'I', b'F'];
pub(crate) const PNG: &[u8] = b"\x89PNG\r\n\x1a\n-png-bytes";

/// A Calibre-made EPUB 2 OPF (at the archive root, as Calibre does): a
/// `uuid` unique identifier plus Calibre and ISBN identifiers, HTML
/// description, series, rating, the cover declared the EPUB 2 way and shown
/// by `titlepage.xhtml`, and an interior illustration whose name contains
/// "cover".
pub(crate) const CALIBRE_OPF: &str = r#"<?xml version='1.0' encoding='utf-8'?>
<package xmlns="http://www.idpf.org/2007/opf" unique-identifier="uuid_id" version="2.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:opf="http://www.idpf.org/2007/opf">
    <dc:title>Dune</dc:title>
    <dc:creator opf:file-as="Herbert, Frank" opf:role="aut">Frank Herbert</dc:creator>
    <dc:contributor opf:file-as="calibre" opf:role="bkp">calibre (6.29.0) [https://calibre-ebook.com]</dc:contributor>
    <dc:date>1965-08-01T00:00:00+00:00</dc:date>
    <dc:identifier id="uuid_id" opf:scheme="uuid">5f2b1c3e-6e0a-4c1b-9a43-3f1f3c0f9b11</dc:identifier>
    <dc:identifier opf:scheme="calibre">5f2b1c3e-6e0a-4c1b-9a43-3f1f3c0f9b11</dc:identifier>
    <dc:identifier opf:scheme="ISBN">9780441013593</dc:identifier>
    <dc:language>en</dc:language>
    <dc:description>&lt;p&gt;Set on the desert planet &lt;b&gt;Arrakis&lt;/b&gt;.&lt;/p&gt;&lt;p&gt;A classic.&lt;/p&gt;</dc:description>
    <dc:publisher>Ace</dc:publisher>
    <dc:subject>Science Fiction</dc:subject>
    <dc:subject>Classics</dc:subject>
    <!-- calibre metadata -->
    <meta name="calibre:series" content="Dune"/>
    <meta name="calibre:series_index" content="1.0"/>
    <meta name="calibre:rating" content="8.0"/>
    <meta name="calibre:timestamp" content="2023-01-01T00:00:00+00:00"/>
    <meta name="calibre:title_sort" content="Dune"/>
    <meta name="cover" content="cover"/>
  </metadata>
  <manifest>
    <item href="cover.jpeg" id="cover" media-type="image/jpeg"/>
    <item href="titlepage.xhtml" id="titlepage" media-type="application/xhtml+xml"/>
    <item href="text/ch1.xhtml" id="ch1" media-type="application/xhtml+xml"/>
    <item href="images/map-cover.jpg" id="map" media-type="image/jpeg"/>
    <item href="toc.ncx" id="ncx" media-type="application/x-dtbncx+xml"/>
  </manifest>
  <spine toc="ncx">
    <itemref idref="titlepage"/>
    <itemref idref="ch1"/>
  </spine>
  <guide>
    <reference href="titlepage.xhtml" title="Cover" type="cover"/>
  </guide>
</package>
"#;

pub(crate) const CALIBRE_TITLEPAGE: &str = r#"<?xml version='1.0' encoding='utf-8'?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Cover</title></head>
<body><svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" version="1.1">
<image width="600" height="800" xlink:href="cover.jpeg"/></svg></body></html>
"#;

/// The Calibre EPUB, written to `path`.
pub(crate) fn write_calibre_epub(path: &Path) {
    write_epub(
        path,
        "content.opf",
        CALIBRE_OPF,
        &[
            ("cover.jpeg", JPEG),
            ("titlepage.xhtml", CALIBRE_TITLEPAGE.as_bytes()),
            ("text/ch1.xhtml", b"<html xmlns=\"http://www.w3.org/1999/xhtml\"><body><img src=\"../images/map-cover.jpg\"/></body></html>"),
            ("images/map-cover.jpg", b"\xFF\xD8\xFF-map"),
            ("toc.ncx", b"<ncx/>"),
        ],
    );
}

/// An EPUB 3 OPF (Sigil style) in `OEBPS/`: `urn:uuid` unique identifier,
/// refined title and creator, a series collection, the cover declared with
/// `properties="cover-image"`, and an interior image named like a cover.
pub(crate) const EPUB3_OPF: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="pub-id">urn:uuid:0b6c1a9e-5f0d-4c1e-9a43-3f1f3c0f9b11</dc:identifier>
    <dc:title id="title">Foundation</dc:title>
    <meta refines="#title" property="title-type">main</meta>
    <dc:creator id="creator01">Isaac Asimov</dc:creator>
    <meta refines="#creator01" property="role" scheme="marc:relators">aut</meta>
    <meta refines="#creator01" property="file-as">Asimov, Isaac</meta>
    <dc:language>en</dc:language>
    <meta property="dcterms:modified">2024-05-01T10:00:00Z</meta>
    <meta property="belongs-to-collection" id="series">Foundation</meta>
    <meta refines="#series" property="collection-type">series</meta>
    <meta refines="#series" property="group-position">1</meta>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="cover-img" href="images/cover.png" media-type="image/png" properties="cover-image"/>
    <item id="ill" href="images/cover-illustration.jpg" media-type="image/jpeg"/>
    <item id="c1" href="text/c1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="c1"/>
  </spine>
</package>
"##;

/// The EPUB 3 book, written to `path`.
pub(crate) fn write_epub3(path: &Path) {
    write_epub(
        path,
        "OEBPS/package.opf",
        EPUB3_OPF,
        &[
            ("OEBPS/nav.xhtml", b"<html xmlns=\"http://www.w3.org/1999/xhtml\"><body><nav/></body></html>"),
            ("OEBPS/images/cover.png", PNG),
            ("OEBPS/images/cover-illustration.jpg", b"\xFF\xD8\xFF-illustration"),
            ("OEBPS/text/c1.xhtml", b"<html xmlns=\"http://www.w3.org/1999/xhtml\"><body><img src=\"../images/cover-illustration.jpg\"/></body></html>"),
        ],
    );
}

/// A minimal EPUB 2 OPF entirely on one line, with `opf:`-prefixed package
/// elements and Dublin Core bound to the prefix `purl`.
pub(crate) const PREFIXED_ONE_LINE_OPF: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8"?>"#,
    r#"<opf:package xmlns:opf="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="id">"#,
    r#"<opf:metadata xmlns:purl="http://purl.org/dc/elements/1.1/">"#,
    r#"<purl:title>Tom &amp; Jerry &#8211; caf&eacute; <![CDATA[<Deluxe>]]></purl:title>"#,
    r#"<purl:creator opf:role="aut">Anonymous</purl:creator>"#,
    r#"<purl:identifier id="id">urn:uuid:11111111-2222-3333-4444-555555555555</purl:identifier>"#,
    r#"<purl:language>en</purl:language>"#,
    r#"<opf:meta name="cover" content="img"/>"#,
    r#"</opf:metadata>"#,
    r#"<opf:manifest><opf:item id="img" href="cover%20image.jpg" media-type="image/jpeg"/><opf:item id="t" href="t.xhtml" media-type="application/xhtml+xml"/></opf:manifest>"#,
    r#"<opf:spine><opf:itemref idref="t"/></opf:spine>"#,
    r#"</opf:package>"#
);
