//! The books on a device, and deleting one.
//!
//! Where books are looked for:
//! - **Kindle**: `documents/` (and its subfolders), the only place a Kindle
//!   indexes. `.sdr` folders (Kindle's per-book reading state) are skipped.
//! - **Kobo**: the **whole volume**. A Kobo indexes books wherever they are,
//!   and people sideload into `Books/`, the root or anywhere else. Hidden
//!   folders (`.kobo`, `.adobe-digital-editions`…) and Windows' system
//!   folders are skipped. The titles, authors, progress and read status the
//!   Kobo itself shows come from `KoboReader.sqlite` ([`super::kobo`]).
//!
//! Each file is matched to its library book (`libraryId`) through the
//! device manifest ([`super::manifest`]) first, then through the id inside
//! the file ([`super::identity`]); books Librarium didn't send have none,
//! and the frontend falls back on title/author/file-name matching.
//!
//! A book's title and author come from, in order: the Kobo's database; the
//! file itself (an EPUB's metadata, a MOBI/AZW3's header: only its first
//! few KB are read, and remembered between listings); what Librarium
//! recorded when it sent it (the manifest); the file name
//! ([`parse_filename_title_author`]).
//!
//! Hidden files are never listed: `.librarium-*.part` transfers in progress
//! and macOS's `._Book.epub` AppleDouble files (which carry an ebook
//! extension but aren't books).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime};

use walkdir::{DirEntry, WalkDir};

use super::detect::{kind_at, DeviceKind};
use super::identity::{epub_details, kindle_details};
use super::kobo::{self, KoboBook, ReadStatus};
use super::manifest::{self, path_key, relative_path, ManifestEntry};
use crate::error::{display_name, AppError, AppResult};
use crate::fsutil;
use crate::logging::Log;

/// File extensions listed as books (lower case).
pub(crate) static EBOOK_EXTENSIONS: &[&str] = &["epub", "mobi", "azw3", "pdf", "cbz"];

/// Whether a file with this (lower-case) extension is a MOBI/AZW3 book,
/// whose header gives its title, authors and library id.
fn is_kindle_book(ext: &str) -> bool {
    matches!(ext, "azw3" | "mobi")
}

/// Whether `path` has an ebook extension.
pub(crate) fn is_ebook(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EBOOK_EXTENSIONS.contains(&e.to_lowercase().as_str()))
}

/// How deep the walks go: far deeper than any real folder layout, but a
/// bound on a pathological (or looping) tree.
const MAX_DEPTH: usize = 12;

/// A book file found on the device (see [`list_books`]).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceBook {
    /// Absolute path.
    pub path: String,
    pub filename: String,
    pub title: String,
    pub author: String,
    /// The library book this file is: from the device manifest, else from
    /// the id embedded in the file. Absent for books Librarium didn't send.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub library_id: Option<String>,
    pub size_bytes: u64,
    /// Kobo only: progress the Kobo recorded (0–100).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub percent_read: Option<u8>,
    /// Kobo only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_status: Option<ReadStatus>,
}

/// Whether a walk should go into (or list) this entry: not hidden (`.x`),
/// not a Windows system folder, not a Kindle `.sdr` folder.
fn keep(entry: &DirEntry) -> bool {
    if entry.depth() == 0 {
        return true;
    }
    let name = entry.file_name().to_string_lossy();
    if name.starts_with('.') {
        return false;
    }
    if entry.file_type().is_dir() {
        let lower = name.to_lowercase();
        return !(lower.starts_with('$')
            || lower == "system volume information"
            || lower.ends_with(".sdr"));
    }
    true
}

/// Every book file on the device at `mount` (see the module docs for where
/// each kind of device is searched), sorted by path.
pub(crate) fn book_files(mount: &Path) -> Vec<PathBuf> {
    let root = match kind_at(mount) {
        Some(DeviceKind::Kobo) => mount.to_path_buf(),
        _ => mount.join("documents"),
    };
    if !root.is_dir() {
        return vec![];
    }
    let mut files: Vec<PathBuf> = WalkDir::new(&root)
        .max_depth(MAX_DEPTH)
        .into_iter()
        .filter_entry(keep)
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file() && is_ebook(e.path()))
        .map(|e| e.into_path())
        .collect();
    files.sort();
    files
}

/// The title and author a file name gives, for files whose content says
/// nothing (PDFs, files without metadata, every file on an MTP Kindle).
///
/// Names like Calibre's `Title - Author`: split at the *last* " - ", since
/// the author comes last and a title may hold one itself ("001 - Réglez-lui
/// son compte ! (1949) - San-Antonio" is a title, then San-Antonio). A name
/// written with underscores for spaces (`Le_calamity_club_-_Kathryn_Stockett`)
/// is read with spaces. Without a separator, it is all title.
pub(crate) fn parse_filename_title_author(stem: &str) -> (String, String) {
    let spaced = if stem.contains(' ') {
        stem.to_string()
    } else {
        stem.replace('_', " ")
    };
    // One space between words; no stray underscores or spaces at the ends.
    let name = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    let clean = |s: &str| {
        s.trim_matches(|c: char| c == '_' || c == '-' || c.is_whitespace())
            .to_string()
    };
    if let Some(pos) = name.rfind(" - ") {
        let (title, author) = (clean(&name[..pos]), clean(&name[pos + 3..]));
        if !title.is_empty() && !author.is_empty() {
            return (title, author);
        }
    }
    (clean(&name), String::new())
}

/// Title, author(s) and embedded library id read from a device file.
type FileDetails = Option<(String, String, Option<String>)>;

/// What was read from device files, kept between listings and keyed by
/// path, size and modification time: opening every book over USB again on
/// each refresh (the view opening, a transfer finishing) made listings slow.
type DetailsCache = Mutex<HashMap<PathBuf, (u64, Option<SystemTime>, FileDetails)>>;

/// Upper bound on cached files (several devices' worth), so the cache can't grow without limit.
const MAX_CACHED_FILES: usize = 20_000;

fn details_cache() -> &'static DetailsCache {
    static CACHE: OnceLock<DetailsCache> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// `read()`'s result for `path`, from the cache when the file is unchanged.
/// `opened` counts the files actually read.
fn cached_details(
    path: &Path,
    size: u64,
    modified: Option<SystemTime>,
    opened: &mut usize,
    read: impl FnOnce() -> FileDetails,
) -> FileDetails {
    let lock = || details_cache().lock().unwrap_or_else(|e| e.into_inner());
    if let Some((s, m, details)) = lock().get(path) {
        if *s == size && *m == modified && modified.is_some() {
            return details.clone();
        }
    }
    *opened += 1;
    let details = read();
    let mut cache = lock();
    if cache.len() >= MAX_CACHED_FILES {
        cache.clear();
    }
    cache.insert(path.to_path_buf(), (size, modified, details.clone()));
    details
}

/// The books on the device at `mount`, with their library id when known.
pub(crate) fn list_books(mount: &Path, log: Log) -> Vec<DeviceBook> {
    let started = Instant::now();
    let mut opened = 0usize;
    let loaded = manifest::load(mount);
    if let manifest::ManifestState::Damaged(reason) = &loaded.state {
        log(format!("[list_device_books] WARNING — the device manifest {reason}; books are matched by name only"));
    }
    if loaded.dropped > 0 {
        log(format!(
            "[list_device_books] WARNING — {} unusable entries in the device manifest were ignored",
            loaded.dropped
        ));
    }
    // What Librarium recorded when it sent each book: its id, title, author.
    let owners: HashMap<String, (&str, &ManifestEntry)> = loaded
        .manifest
        .books
        .iter()
        .map(|(id, entry)| (path_key(&entry.path), (id.as_str(), entry)))
        .collect();

    let kobo_rows = if kind_at(mount) == Some(DeviceKind::Kobo) {
        kobo_books(mount, log)
    } else {
        vec![]
    };
    let mut by_path: HashMap<String, &KoboBook> = HashMap::new();
    for row in &kobo_rows {
        by_path.insert(path_key(&row.rel_path), row);
        if let Some(decoded) = &row.decoded_rel_path {
            by_path.insert(path_key(decoded), row);
        }
    }

    let books: Vec<DeviceBook> = book_files(mount)
        .into_iter()
        .filter_map(|path| {
            let filename = path.file_name()?.to_str()?.to_string();
            let stem = path.file_stem()?.to_str()?.to_string();
            let ext = path
                .extension()
                .and_then(|x| x.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let key = relative_path(mount, &path)
                .map(|rel| path_key(&rel))
                .unwrap_or_default();
            let meta = fs::metadata(&path).ok();
            let size_bytes = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            let modified = meta.and_then(|m| m.modified().ok());
            let kobo = by_path.get(&key).copied();

            let sent = owners.get(&key);
            let mut library_id = sent.map(|(id, _)| id.to_string());
            let mut title = kobo.map(|k| k.title.clone()).unwrap_or_default();
            let mut author = kobo.map(|k| k.author.clone()).unwrap_or_default();
            // Open the file only for what is still unknown.
            if ext == "epub" && (title.is_empty() || library_id.is_none()) {
                if let Some((t, a, id)) =
                    cached_details(&path, size_bytes, modified, &mut opened, || {
                        epub_details(&path)
                    })
                {
                    if title.is_empty() && !t.is_empty() {
                        (title, author) = (t, a);
                    }
                    library_id = library_id.or(id);
                }
            } else if is_kindle_book(&ext) && (title.is_empty() || library_id.is_none()) {
                // The header (record 0) only: a few KB, not the whole book.
                if let Some((t, a, id)) =
                    cached_details(&path, size_bytes, modified, &mut opened, || {
                        kindle_details(&path).map(|d| (d.title, d.author, d.library_id))
                    })
                {
                    if title.is_empty() && !t.is_empty() {
                        (title, author) = (t, a);
                    }
                    library_id = library_id.or(id);
                }
            }
            // Nothing in the file: what Librarium recorded when it sent it,
            // else what the file name says.
            if title.is_empty() {
                if let Some((_, entry)) = sent.filter(|(_, e)| !e.title.trim().is_empty()) {
                    (title, author) = (entry.title.clone(), entry.author.clone());
                }
            }
            if title.is_empty() {
                (title, author) = parse_filename_title_author(&stem);
            }

            Some(DeviceBook {
                path: path.to_string_lossy().into_owned(),
                filename,
                title,
                author,
                library_id,
                size_bytes,
                percent_read: kobo.and_then(|k| k.percent_read),
                read_status: kobo.and_then(|k| k.read_status),
            })
        })
        .collect();
    log(format!(
        "[list_device_books] {} books in {} ms ({opened} files read, the rest from cache)",
        books.len(),
        started.elapsed().as_millis()
    ));
    books
}

/// The sideloaded books of the Kobo database; empty (logged) when it can't
/// be read, so the listing still shows the files.
fn kobo_books(mount: &Path, log: Log) -> Vec<KoboBook> {
    match kobo::open_snapshot(mount).and_then(|s| s.map(|s| kobo::read_books(&s.conn)).transpose())
    {
        Ok(rows) => rows.unwrap_or_default(),
        Err(e) => {
            log(format!(
                "[list_device_books] WARNING — {e}; titles come from the files"
            ));
            vec![]
        }
    }
}

// ── Deleting ──────────────────────────────────────────────────────────────────

/// What [`delete_book`] removed besides the file.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Deleted {
    /// Kindle `.sdr` folders (reading position, notes) of the book.
    pub sdr: Vec<PathBuf>,
    /// Library ids whose manifest entries were removed.
    pub forgotten: Vec<String>,
}

/// Delete the book file `path` from the device at `mount`: the file, then
/// (Kindle) its `.sdr` folder, then its manifest entry. The caller checked
/// that `path` is on this device; it is checked again here. Failing to
/// remove the `.sdr` or the entry is only logged: the book itself is gone.
///
/// On a Kobo, the book's row in `KoboReader.sqlite` is left for the Kobo
/// to clean up (it notices missing files when it re-scans after the USB
/// session); Librarium never writes to that database.
/// TODO(device-validation): confirm a real Kobo drops the row (and doesn't
/// show a ghost book) after a delete over USB.
pub(crate) fn delete_book(mount: &Path, path: &Path, log: Log) -> AppResult<Deleted> {
    let name = display_name(path);
    let is_file = fs::symlink_metadata(path)
        .map(|m| m.is_file())
        .unwrap_or(false);
    if !is_file || !is_ebook(path) {
        return Err(AppError::InvalidInput(format!(
            "Not an ebook file on the device: {name}"
        )));
    }
    if !fsutil::is_within(mount, path) {
        return Err(AppError::Device(format!("{name} isn't on this e-reader")));
    }
    // Read the book's ASIN before the file goes, to drop its home-screen
    // thumbnail too (a Kindle doesn't, and it would be left behind).
    super::thumbnail::remove_for_book(mount, path);
    fs::remove_file(path)
        .map_err(|e| AppError::io(format!("Could not delete {name} from the e-reader"), &e))?;
    let mut deleted = Deleted::default();

    if kind_at(mount) == Some(DeviceKind::Kindle) {
        for sdr in sdr_folders(mount, path) {
            match fs::remove_dir_all(&sdr) {
                Ok(()) => deleted.sdr.push(sdr),
                Err(e) => log(format!(
                    "[delete_device_book] WARNING — could not remove {}: {e}",
                    sdr.display()
                )),
            }
        }
    }

    if let Some(rel) = relative_path(mount, path) {
        match manifest::update(mount, log, |m| m.remove_path(&rel)) {
            Ok(ids) => deleted.forgotten = ids,
            Err(e) => log(format!("[delete_device_book] WARNING — {e}")),
        }
    }
    Ok(deleted)
}

/// The Kindle `.sdr` folders that belong to the (just deleted) book at
/// `book`: `<stem>.sdr` and `<file name>.sdr` next to it, only inside
/// `documents/`, only real folders (not links). `<stem>.sdr` is kept when
/// another book with the same stem is still there (`Dune.pdf` next to
/// `Dune.azw3`): it may be that book's.
///
/// Validated on a Kindle Paperwhite 3 over USB on macOS (FAT32), 2026-11-09: `.sdr` folders are named after the stem, and deleting a book with
/// its `.sdr` left the Kindle's library consistent.
/// TODO(device-validation): older (pre-Paperwhite) firmware.
fn sdr_folders(mount: &Path, book: &Path) -> Vec<PathBuf> {
    let (Some(dir), Some(stem), Some(file_name)) =
        (book.parent(), book.file_stem(), book.file_name())
    else {
        return vec![];
    };
    if !fsutil::is_within(&mount.join("documents"), dir) {
        return vec![];
    }
    let stem = stem.to_string_lossy();
    let same_stem_left = fs::read_dir(dir).into_iter().flatten().flatten().any(|e| {
        let p = e.path();
        p.is_file()
            && is_ebook(&p)
            && p.file_stem()
                .is_some_and(|s| s.to_string_lossy().eq_ignore_ascii_case(&stem))
    });
    let mut candidates = vec![dir.join(format!("{}.sdr", file_name.to_string_lossy()))];
    if !same_stem_left {
        candidates.push(dir.join(format!("{stem}.sdr")));
    }
    candidates
        .into_iter()
        .filter(|c| {
            fs::symlink_metadata(c).map(|m| m.is_dir()).unwrap_or(false)
                && fsutil::is_within(&mount.join("documents"), c)
        })
        .collect()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Opt-in check against a real, mounted e-reader (read-only: it only
    /// lists files and reads headers). Run with
    /// `LIBRARIUM_DEVICE=/Volumes/Kindle cargo test real_device_listing -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs a mounted e-reader (LIBRARIUM_DEVICE)"]
    fn real_device_listing() {
        let Ok(mount) = std::env::var("LIBRARIUM_DEVICE") else {
            return;
        };
        let mount = std::path::PathBuf::from(mount);
        let started = std::time::Instant::now();
        let files = book_files(&mount);
        println!(
            "book_files: {} files in {:?}",
            files.len(),
            started.elapsed()
        );
        let started = std::time::Instant::now();
        let books = list_books(&mount, &|line| println!("{line}"));
        let with_id = books.iter().filter(|b| b.library_id.is_some()).count();
        println!(
            "list_books: {} books ({with_id} with a library id) in {:?}",
            books.len(),
            started.elapsed()
        );
        // Every book, to check titles and authors by eye.
        for b in &books {
            println!("  {} | {} | {}", b.filename, b.title, b.author);
        }
    }
    use crate::commands::device::kobo::tests::kobo_db;
    use crate::commands::device::manifest::ManifestEntry;
    use crate::commands::device::tests::{fake_kindle, fake_kobo, minimal_epub};
    use crate::logging::no_log;

    const ID: &str = "3f2b1c3e-6e0a-4c1b-9a43-3f1f3c0f9b11";

    fn record(mount: &Path, id: &str, rel: &str) {
        record_titled(mount, id, rel, "", "");
    }

    /// Record `rel` as library book `id`, sent with this title and author.
    fn record_titled(mount: &Path, id: &str, rel: &str, title: &str, author: &str) {
        manifest::update(mount, &no_log, |m| {
            m.books.insert(
                id.into(),
                ManifestEntry {
                    path: rel.into(),
                    title: title.into(),
                    author: author.into(),
                    format: "azw3".into(),
                    size_bytes: 1,
                    sent_at: String::new(),
                },
            );
        })
        .unwrap();
    }

    fn summary(books: &[DeviceBook]) -> Vec<(&str, &str, &str, Option<&str>)> {
        books
            .iter()
            .map(|b| {
                (
                    b.filename.as_str(),
                    b.title.as_str(),
                    b.author.as_str(),
                    b.library_id.as_deref(),
                )
            })
            .collect()
    }

    #[test]
    fn kindle_listing_parses_names_and_epub_metadata_and_skips_hidden_files() {
        let device = fake_kindle();
        let docs = device.path().join("documents");
        fs::write(docs.join("Dune - Frank Herbert.azw3"), b"x").unwrap();
        fs::write(docs.join("notes.txt"), b"x").unwrap();
        fs::write(docs.join("._Dune - Frank Herbert.azw3"), b"appledouble").unwrap();
        fs::write(docs.join(".librarium-abc123.part"), b"partial").unwrap();
        fs::create_dir(docs.join("Dune - Frank Herbert.sdr")).unwrap();
        fs::write(
            docs.join("Dune - Frank Herbert.sdr/Dune.pdf"),
            b"not a book",
        )
        .unwrap();
        minimal_epub(&docs.join("whatever.epub"));
        fs::write(docs.join("broken.epub"), b"not a zip").unwrap();

        let books = list_books(device.path(), &no_log);
        assert_eq!(
            summary(&books),
            vec![
                ("Dune - Frank Herbert.azw3", "Dune", "Frank Herbert", None),
                ("broken.epub", "broken", "", None),
                ("whatever.epub", "Test Book", "Tester", None),
            ]
        );
        assert_eq!(books[0].size_bytes, 1);
    }

    #[test]
    fn file_names_give_a_title_and_an_author_split_at_the_last_dash() {
        let pair = |t: &str, a: &str| (t.to_string(), a.to_string());
        let parsed = parse_filename_title_author;
        assert_eq!(
            parsed("Dune - Frank Herbert"),
            pair("Dune", "Frank Herbert")
        );
        // The author comes last; the title may hold a dash itself.
        assert_eq!(
            parsed("001 - Reglez-lui son compte ! (1949) - San-Antonio"),
            pair("001 - Reglez-lui son compte ! (1949)", "San-Antonio")
        );
        // Underscores for spaces.
        assert_eq!(
            parsed("_OceanofPDF.com_Le_calamity_club_French_Edition_-_Kathryn_Stockett"),
            pair(
                "OceanofPDF.com Le calamity club French Edition",
                "Kathryn Stockett"
            )
        );
        // Calibre's Kindle names ("Title _ - Author"); spaces kept as they are.
        assert_eq!(
            parsed("Dune _ - Frank Herbert"),
            pair("Dune", "Frank Herbert")
        );
        assert_eq!(parsed("my_notes and_more"), pair("my_notes and_more", ""));
        // No separator, or nothing on one side: all title.
        assert_eq!(parsed("Dune"), pair("Dune", ""));
        assert_eq!(parsed("Dune - "), pair("Dune", ""));
        assert_eq!(parsed("  - Frank Herbert"), pair("Frank Herbert", ""));
    }

    #[test]
    fn titles_come_from_the_header_then_the_manifest_then_the_file_name() {
        let device = fake_kindle();
        let docs = device.path().join("documents");
        // An AZW3 under an unhelpful name: its header knows better.
        let work = tempfile::tempdir().unwrap();
        minimal_epub(&work.path().join("Book.epub"));
        crate::convert::convert_to_temp(
            &work.path().join("Book.epub"),
            crate::convert::OutputFormat::Azw3,
            work.path(),
        )
        .unwrap()
        .persist(docs.join("001 - something.azw3"))
        .unwrap();
        // A PDF Librarium sent: the manifest knows its title and author.
        fs::write(docs.join("dune.pdf"), b"%PDF-1.4").unwrap();
        record_titled(
            device.path(),
            ID,
            "documents/dune.pdf",
            "Dune",
            "Frank Herbert",
        );
        // A PDF nobody knows: its name.
        fs::write(
            docs.join("Le_calamity_club_-_Kathryn_Stockett.pdf"),
            b"%PDF-1.4",
        )
        .unwrap();

        let books = list_books(device.path(), &no_log);
        assert_eq!(
            summary(&books),
            vec![
                ("001 - something.azw3", "Test Book", "Tester", None),
                (
                    "Le_calamity_club_-_Kathryn_Stockett.pdf",
                    "Le calamity club",
                    "Kathryn Stockett",
                    None
                ),
                ("dune.pdf", "Dune", "Frank Herbert", Some(ID)),
            ]
        );
    }

    #[test]
    fn library_ids_come_from_the_manifest_then_from_the_file() {
        let device = fake_kindle();
        let docs = device.path().join("documents");
        fs::create_dir(docs.join("SF")).unwrap();
        fs::write(docs.join("SF/Dune.azw3"), b"x").unwrap();
        minimal_epub(&docs.join("Tagged.epub"));
        crate::commands::library::epub_edit::write_library_id(
            &docs.join("Tagged.epub"),
            "aaaaaaaa-0000-4000-8000-000000000001",
        )
        .unwrap();
        // Recorded with another case: FAT doesn't care, neither does the lookup.
        record(device.path(), ID, "documents/sf/DUNE.azw3");

        let books = list_books(device.path(), &no_log);
        assert_eq!(
            summary(&books),
            vec![
                ("Dune.azw3", "Dune", "", Some(ID)),
                (
                    "Tagged.epub",
                    "Test Book",
                    "Tester",
                    Some("aaaaaaaa-0000-4000-8000-000000000001")
                ),
            ]
        );
    }

    #[test]
    fn kobo_listing_walks_the_whole_volume_and_merges_the_database() {
        let device = fake_kobo();
        let root = device.path();
        fs::create_dir_all(root.join("Books/SF")).unwrap();
        fs::write(
            root.join("Books/SF/Dune.kepub.epub"),
            b"not parsed: the database has the title",
        )
        .unwrap();
        minimal_epub(&root.join("At the root.epub"));
        fs::create_dir_all(root.join("documents")).unwrap();
        fs::write(root.join("documents/Sent - By Librarium.epub"), b"x").unwrap();
        fs::create_dir_all(root.join("Café")).unwrap();
        fs::write(root.join("Café/Crème.pdf"), b"%PDF").unwrap();
        // Never listed: Kobo's own folders, hidden folders, Windows folders.
        fs::create_dir_all(root.join(".kobo/kepub")).unwrap();
        fs::write(root.join(".kobo/kepub/store-book.epub"), b"x").unwrap();
        fs::create_dir_all(root.join(".adobe-digital-editions")).unwrap();
        fs::write(root.join(".adobe-digital-editions/x.epub"), b"x").unwrap();
        fs::create_dir_all(root.join("$RECYCLE.BIN")).unwrap();
        fs::write(root.join("$RECYCLE.BIN/old.epub"), b"x").unwrap();
        drop(kobo_db(
            root,
            &[
                (
                    "file:///mnt/onboard/Books/SF/Dune.kepub.epub",
                    "6",
                    "Dune",
                    "Frank Herbert",
                    42,
                    1,
                ),
                (
                    "file:///mnt/onboard/Caf%C3%A9/Cr%C3%A8me.pdf",
                    "6",
                    "Crème",
                    "Chef",
                    100,
                    2,
                ),
                (
                    "file:///mnt/onboard/Gone.epub",
                    "6",
                    "Deleted meanwhile",
                    "",
                    0,
                    0,
                ),
            ],
        ));
        record(root, ID, "documents/Sent - By Librarium.epub");

        let books = list_books(root, &no_log);
        assert_eq!(
            summary(&books),
            vec![
                ("At the root.epub", "Test Book", "Tester", None),
                ("Dune.kepub.epub", "Dune", "Frank Herbert", None),
                ("Crème.pdf", "Crème", "Chef", None),
                ("Sent - By Librarium.epub", "Sent", "By Librarium", Some(ID)),
            ]
        );
        let dune = &books[1];
        assert_eq!(
            (dune.percent_read, dune.read_status),
            (Some(42), Some(ReadStatus::Reading))
        );
        assert_eq!(books[2].read_status, Some(ReadStatus::Finished));
        assert_eq!(books[0].percent_read, None);
    }

    #[test]
    fn a_kobo_without_a_readable_database_still_lists_its_files() {
        let device = fake_kobo();
        fs::write(
            device.path().join(".kobo/KoboReader.sqlite"),
            b"not a database",
        )
        .unwrap();
        fs::write(device.path().join("Book - Author.pdf"), b"%PDF").unwrap();
        assert_eq!(
            summary(&list_books(device.path(), &no_log)),
            vec![("Book - Author.pdf", "Book", "Author", None)]
        );
    }

    #[test]
    fn serialises_like_the_ipc_contract() {
        let book = DeviceBook {
            path: "/Volumes/KOBOeReader/Dune.epub".into(),
            filename: "Dune.epub".into(),
            title: "Dune".into(),
            author: "Frank Herbert".into(),
            library_id: Some(ID.into()),
            size_bytes: 3,
            percent_read: Some(42),
            read_status: Some(ReadStatus::Reading),
        };
        assert_eq!(
            serde_json::to_value(&book).unwrap(),
            serde_json::json!({
                "path": "/Volumes/KOBOeReader/Dune.epub", "filename": "Dune.epub", "title": "Dune",
                "author": "Frank Herbert", "libraryId": ID, "sizeBytes": 3, "percentRead": 42, "readStatus": "reading",
            })
        );
        let plain = DeviceBook {
            library_id: None,
            percent_read: None,
            read_status: None,
            ..book
        };
        assert_eq!(
            serde_json::to_value(&plain)
                .unwrap()
                .as_object()
                .unwrap()
                .keys()
                .collect::<Vec<_>>(),
            vec!["author", "filename", "path", "sizeBytes", "title"]
        );
    }

    #[test]
    fn deleting_a_kindle_book_removes_its_sdr_and_manifest_entry() {
        let device = fake_kindle();
        let docs = device.path().join("documents");
        let book = docs.join("Dune.azw3");
        fs::write(&book, b"x").unwrap();
        fs::create_dir_all(docs.join("Dune.sdr")).unwrap();
        fs::write(docs.join("Dune.sdr/Dune.mbs"), b"position").unwrap();
        fs::create_dir_all(docs.join("Other.sdr")).unwrap();
        record(device.path(), ID, "documents/Dune.azw3");
        record(
            device.path(),
            "bbbbbbbb-0000-4000-8000-000000000002",
            "documents/Other.azw3",
        );

        let deleted = delete_book(device.path(), &book, &no_log).unwrap();
        assert_eq!(
            deleted,
            Deleted {
                sdr: vec![docs.join("Dune.sdr")],
                forgotten: vec![ID.into()]
            }
        );
        assert!(!book.exists() && !docs.join("Dune.sdr").exists());
        assert!(
            docs.join("Other.sdr").is_dir(),
            "other books' reading state is kept"
        );
        let left = manifest::load(device.path()).manifest;
        assert_eq!(
            left.books.keys().collect::<Vec<_>>(),
            vec!["bbbbbbbb-0000-4000-8000-000000000002"]
        );
    }

    #[test]
    fn a_shared_sdr_stem_is_kept_while_another_book_uses_it() {
        let device = fake_kindle();
        let docs = device.path().join("documents");
        fs::write(docs.join("Dune.azw3"), b"x").unwrap();
        fs::write(docs.join("Dune.pdf"), b"%PDF").unwrap();
        fs::create_dir_all(docs.join("Dune.sdr")).unwrap();
        fs::create_dir_all(docs.join("Dune.pdf.sdr")).unwrap();

        let deleted = delete_book(device.path(), &docs.join("Dune.pdf"), &no_log).unwrap();
        assert_eq!(
            deleted.sdr,
            vec![docs.join("Dune.pdf.sdr")],
            "Dune.sdr may be Dune.azw3's"
        );
        assert!(docs.join("Dune.sdr").is_dir());
        assert!(deleted.forgotten.is_empty());
        assert!(
            !device.path().join(".librarium").exists(),
            "no manifest created by a delete"
        );
    }

    #[test]
    fn delete_refuses_non_books_and_paths_off_the_device() {
        let device = fake_kindle();
        let notes = device.path().join("documents/notes.txt");
        fs::write(&notes, b"x").unwrap();
        assert_eq!(
            delete_book(device.path(), &notes, &no_log)
                .unwrap_err()
                .code(),
            "INVALID_INPUT"
        );
        assert!(notes.exists());

        let elsewhere = tempfile::tempdir().unwrap();
        let outside = elsewhere.path().join("Dune.azw3");
        fs::write(&outside, b"x").unwrap();
        assert_eq!(
            delete_book(device.path(), &outside, &no_log)
                .unwrap_err()
                .code(),
            "DEVICE"
        );
        assert!(outside.exists());
        let sdr_dir = device.path().join("documents/../Dune.sdr");
        assert!(sdr_folders(device.path(), &sdr_dir).is_empty());
    }

    #[test]
    fn kobo_deletes_keep_sdr_like_folders() {
        let device = fake_kobo();
        let book = device.path().join("Dune.epub");
        fs::write(&book, b"x").unwrap();
        fs::create_dir_all(device.path().join("Dune.sdr")).unwrap();
        delete_book(device.path(), &book, &no_log).unwrap();
        assert!(
            device.path().join("Dune.sdr").is_dir(),
            ".sdr is a Kindle thing"
        );
    }
}
