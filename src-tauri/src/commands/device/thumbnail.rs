//! Home-screen covers for books on a Kindle that connects as a USB drive.
//!
//! A Kindle shows a book's cover from
//! `system/thumbnails/thumbnail_<ASIN>_<cdetype>_portrait.jpg`. For a book
//! with an ASIN and cdetype `EBOK` (what Librarium's AZW3 conversions carry,
//! like Calibre's), it does NOT make that file from the cover inside the
//! book: it tries to fetch it, leaves an empty `….jpg.partial` behind, and
//! shows a blank cover. Calibre writes the thumbnail itself when it sends a
//! book; so does Librarium, from the library's cover of that book.
//!
//! Seen on a Kindle Paperwhite 3 (2026-11-10): the books Librarium had sent
//! only had `.partial` files, and the thumbnails Calibre had written there
//! are JPEGs 470 px tall.
//!
//! Best effort everywhere: a missing cover or a write error is logged, never
//! a reason to fail a transfer or a delete.

use std::fs;
use std::path::{Path, PathBuf};

use super::identity::kindle_ids;
use super::manifest;
use crate::commands::library::covers;
use crate::fsutil;
use crate::logging::Log;

/// Height of the thumbnails Calibre writes for these Kindles.
pub(crate) const THUMBNAIL_HEIGHT: u32 = 470;
/// Widest a thumbnail may be (covers are portrait; this only bounds odd ones).
const THUMBNAIL_MAX_WIDTH: u32 = 940;
/// JPEG quality of the thumbnails (Calibre's are 20–35 KB at 470 px).
const THUMBNAIL_QUALITY: u8 = 85;

/// The Kindle's thumbnail folder.
pub(crate) fn thumbnails_dir(mount: &Path) -> PathBuf {
    mount.join("system").join("thumbnails")
}

/// The file name a Kindle looks for: `thumbnail_<asin>_<cdetype>_portrait.jpg`.
pub(crate) fn thumbnail_name(asin: &str, cdetype: &str) -> String {
    format!("thumbnail_{asin}_{cdetype}_portrait.jpg")
}

/// An ASIN or cdetype safe to put in a file name: letters, digits and `-`,
/// as Amazon ASINs, UUIDs and cdetypes are. Anything else is ignored.
fn is_safe_part(part: &str) -> bool {
    !part.is_empty()
        && part.len() <= 64
        && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Where the thumbnail of the Kindle book at `book` goes, if it has an ASIN
/// and a cdetype and the device has a thumbnail folder (a Kindle; a Kobo has
/// none, and the folder is never created).
fn thumbnail_path(mount: &Path, book: &Path) -> Option<PathBuf> {
    let (asin, cdetype) = kindle_ids(book)?;
    if !is_safe_part(&asin) || !is_safe_part(&cdetype) {
        return None;
    }
    let dir = thumbnails_dir(mount);
    dir.is_dir()
        .then(|| dir.join(thumbnail_name(&asin, &cdetype)))
}

/// Write the home-screen thumbnail of the book at `book` (a file on the
/// device) from `cover` (image bytes), and remove the Kindle's failed
/// `.partial` attempt. `Ok(false)` when the book needs none (no ASIN, or not
/// a Kindle).
pub(crate) fn write_for_book(mount: &Path, book: &Path, cover: &[u8]) -> Result<bool, String> {
    let Some(path) = thumbnail_path(mount, book) else {
        return Ok(false);
    };
    let jpeg = covers::fit_as_jpeg(
        cover,
        THUMBNAIL_MAX_WIDTH,
        THUMBNAIL_HEIGHT,
        THUMBNAIL_QUALITY,
    )?;
    fsutil::atomic_write(&path, &jpeg)
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    let _ = fs::remove_file(partial_of(&path));
    Ok(true)
}

/// Remove the thumbnail (and any `.partial`) of the book at `book`, before
/// that book is deleted from the device.
pub(crate) fn remove_for_book(mount: &Path, book: &Path) {
    if let Some(path) = thumbnail_path(mount, book) {
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(partial_of(&path));
    }
}

fn partial_of(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".partial");
    PathBuf::from(name)
}

/// Give a cover to the books Librarium sent (device manifest) that have none
/// on the Kindle yet: books sent before this existed, or whose thumbnail
/// write failed. Uses the covers of `library` (the books of other libraries
/// are skipped). Returns how many thumbnails were written.
pub(crate) fn repair(mount: &Path, library: &Path, log: Log) -> usize {
    if !thumbnails_dir(mount).is_dir() {
        return 0;
    }
    let loaded = manifest::load(mount);
    let mut written = 0;
    for (book_id, entry) in &loaded.manifest.books {
        let Some(book) = manifest::resolve(mount, &entry.path) else {
            continue;
        };
        let Some(path) = thumbnail_path(mount, &book) else {
            continue;
        };
        if path.is_file() {
            continue;
        }
        let Some(cover) = covers::cover_image(library, book_id, log) else {
            continue;
        };
        match write_for_book(mount, &book, &cover) {
            Ok(true) => written += 1,
            Ok(false) => {}
            Err(e) => log(format!("[device covers] WARNING — {}: {e}", entry.path)),
        }
    }
    if written > 0 {
        log(format!(
            "[device covers] wrote {written} missing Kindle cover(s)"
        ));
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_the_kindle_convention() {
        assert_eq!(
            thumbnail_name("625e0efd-d766-45c2-ad2d-d39d5bb7177d", "EBOK"),
            "thumbnail_625e0efd-d766-45c2-ad2d-d39d5bb7177d_EBOK_portrait.jpg"
        );
        assert!(is_safe_part("B00ABCDEF1") && is_safe_part("EBOK"));
        assert!(!is_safe_part("../x") && !is_safe_part("") && !is_safe_part("a b"));
        assert_eq!(
            partial_of(Path::new("/k/t.jpg")),
            PathBuf::from("/k/t.jpg.partial")
        );
    }
}
