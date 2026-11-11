//! Test fixtures shared by the library modules' tests: tiny but well-formed
//! EPUBs built with the `zip` crate, helpers to look inside them, and
//! shortcuts for scanning a temp library and reading its index. Compiled for
//! tests only.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use zip::write::SimpleFileOptions;
use zip::CompressionMethod;

use super::index::with_index;
use super::scan::scan_library_at;
use crate::fsutil::{self, TrashOutcome};
use crate::logging::no_log;
use crate::models::book::Book;

/// Where the fixtures put their OPF.
pub(crate) const OPF_PATH: &str = "OEBPS/content.opf";

const CONTAINER_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>
"#;

const CHAPTER: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml">
<head><title>Chapter 1</title></head>
<body>
  <h1>Chapter 1</h1>
  <p>A beginning is the time for taking the most delicate care that the balances are correct.</p>
  <p>This every sister of the Bene Gesserit knows.</p>
</body>
</html>
"#;

const NCX: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <head><meta name="dtb:uid" content="urn:uuid:librarium-test"/></head>
  <docTitle><text>Test</text></docTitle>
  <navMap>
    <navPoint id="np1" playOrder="1"><navLabel><text>Chapter 1</text></navLabel><content src="ch1.xhtml"/></navPoint>
  </navMap>
</ncx>
"#;

/// An EPUB 2 OPF with the given title and author, a `urn:uuid` unique
/// identifier, one chapter, and optional extra metadata/manifest lines.
pub(crate) fn opf(title: &str, author: &str, extra_metadata: &str, extra_manifest: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="BookId">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:opf="http://www.idpf.org/2007/opf">
    <dc:title>{title}</dc:title>
    <dc:creator>{author}</dc:creator>
    <dc:identifier id="BookId">urn:uuid:librarium-test</dc:identifier>
    <dc:language>en</dc:language>
{extra_metadata}  </metadata>
  <manifest>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
{extra_manifest}  </manifest>
  <spine toc="ncx">
    <itemref idref="ch1"/>
  </spine>
</package>
"#
    )
}

/// Write an EPUB with the given OPF and extra entries to `path`.
pub(crate) fn write_epub_with(path: &Path, opf: &str, extra: &[(&str, &[u8])]) {
    let file = std::fs::File::create(path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    zip.start_file("mimetype", stored).unwrap();
    zip.write_all(b"application/epub+zip").unwrap();
    for (name, content) in [
        ("META-INF/container.xml", CONTAINER_XML.as_bytes()),
        (OPF_PATH, opf.as_bytes()),
        ("OEBPS/toc.ncx", NCX.as_bytes()),
        ("OEBPS/ch1.xhtml", CHAPTER.as_bytes()),
    ] {
        zip.start_file(name, deflated).unwrap();
        zip.write_all(content).unwrap();
    }
    for (name, content) in extra {
        zip.start_file(*name, deflated).unwrap();
        zip.write_all(content).unwrap();
    }
    zip.finish().unwrap();
}

/// A plain EPUB with this title and author.
pub(crate) fn write_epub(path: &Path, title: &str, author: &str) {
    write_epub_with(path, &opf(title, author, "", ""), &[]);
}

/// An EPUB whose cover (`OEBPS/images/cover.jpg`) is declared the EPUB 2 and
/// EPUB 3 ways.
pub(crate) fn epub_with_cover(path: &Path, image: &[u8]) {
    let opf = opf(
        "Dune",
        "Frank Herbert",
        "    <meta name=\"cover\" content=\"cover-img\"/>\n",
        "    <item id=\"cover-img\" href=\"images/cover.jpg\" media-type=\"image/jpeg\" properties=\"cover-image\"/>\n",
    );
    write_epub_with(path, &opf, &[("OEBPS/images/cover.jpg", image)]);
}

/// Names of the archive entries, in archive order.
pub(crate) fn entry_names(path: &Path) -> Vec<String> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    (0..archive.len())
        .map(|i| archive.by_index(i).unwrap().name().unwrap().into_owned())
        .collect()
}

/// Content of one archive entry, if present.
pub(crate) fn read_entry_bytes(path: &Path, name: &str) -> Option<Vec<u8>> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(path).ok()?).ok()?;
    let mut entry = archive.by_name(name).ok()?;
    let mut buf = Vec::new();
    entry.read_to_end(&mut buf).ok()?;
    Some(buf)
}

// ── Temp libraries ────────────────────────────────────────────────────────────

/// Fresh empty directory under the system temp dir, removed by the caller.
pub(crate) fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("librarium-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Write `contents` to `path`, creating the parent folders.
pub(crate) fn write(path: &Path, contents: &[u8]) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, contents).unwrap();
}

/// A trash that never touches the real OS trash: always the library's
/// fallback folder.
pub(crate) fn test_trash(lib: &Path) -> impl Fn(&Path) -> std::io::Result<TrashOutcome> + '_ {
    move |p| fsutil::move_to_trash_with(p, lib, |_| Err("no OS trash in tests".into()))
}

/// Scan `lib` and return its books keyed by file name.
pub(crate) fn scan(lib: &Path) -> HashMap<String, Book> {
    scan_library_at(lib, &no_log)
        .unwrap()
        .into_iter()
        .map(|b| (b.filename.clone(), b))
        .collect()
}

/// The indexed book with this id, read fresh from disk.
pub(crate) fn indexed(lib: &Path, id: &str) -> Option<Book> {
    with_index(lib, &no_log, |index| Ok(index.by_id(id).cloned())).unwrap()
}

/// Number of books in `lib`'s index.
pub(crate) fn index_len(lib: &Path) -> usize {
    with_index(lib, &no_log, |index| Ok(index.len())).unwrap()
}

/// The OPF of the EPUB at `path` (a fixture, so at [`OPF_PATH`]).
pub(crate) fn opf_of(path: &Path) -> String {
    String::from_utf8(read_entry_bytes(path, OPF_PATH).unwrap()).unwrap()
}

/// Run `f` on another thread and wait at most `secs` seconds for it: a
/// command that waits on a lock held by the caller times out (and fails the
/// test) instead of hanging it.
pub(crate) fn within<T: Send + 'static>(secs: u64, f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(std::time::Duration::from_secs(secs))
        .expect("blocked: the work waited on a lock held by the caller")
}

/// Points where the code under test calls back into a test, so it can act
/// *during* a command — between its short holds of the index lock — and
/// check what is (not) locked. Only compiled for tests.
pub(crate) mod hooks {
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    /// A scan or an import is about to read a file (`scan::build_book_from_file`).
    pub(crate) const READ: &str = "read";
    /// A metadata edit has just rewritten a book's file (`edit::edit_books`).
    pub(crate) const EDIT_WRITTEN: &str = "edit-written";

    type Hook = Arc<dyn Fn(&Path) + Send + Sync>;

    static HOOKS: Mutex<Vec<(&'static str, PathBuf, Hook)>> = Mutex::new(Vec::new());

    /// Removes its hook when dropped.
    pub(crate) struct Registered(&'static str, PathBuf);

    impl Drop for Registered {
        fn drop(&mut self) {
            HOOKS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .retain(|(point, dir, _)| !(*point == self.0 && *dir == self.1));
        }
    }

    /// Call `hook` with the file's path each time the code reaches `point`
    /// for a file under `dir`, until the guard is dropped. Keyed by folder,
    /// so tests running in parallel (each in its own temp folder) don't see
    /// each other's hooks.
    pub(crate) fn on(
        point: &'static str,
        dir: &Path,
        hook: impl Fn(&Path) + Send + Sync + 'static,
    ) -> Registered {
        HOOKS.lock().unwrap_or_else(|e| e.into_inner()).push((
            point,
            dir.to_path_buf(),
            Arc::new(hook),
        ));
        Registered(point, dir.to_path_buf())
    }

    /// Run the hooks registered for `point` and a folder holding `path`.
    pub(crate) fn run(point: &'static str, path: &Path) {
        let hooks: Vec<Hook> = HOOKS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|(p, dir, _)| *p == point && path.starts_with(dir))
            .map(|(_, _, hook)| hook.clone())
            .collect();
        for hook in hooks {
            hook(path);
        }
    }
}
