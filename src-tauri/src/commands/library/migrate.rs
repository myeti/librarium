//! Migrating a v1 library index to v2.
//!
//! **v1** (an older format) is a JSON object mapping each book's *absolute*
//! path to its record: `{ "/Volumes/books/Alice/Dune.epub": Book, … }`.
//! It breaks as soon as the library is reached through another path: the
//! share mounted as `/Volumes/books-1`, the folder moved, the same NAS
//! library opened from a Windows PC (`\\nas\books\Alice\…`).
//!
//! **v2** keys each book by its path relative to the library folder (see
//! [`super::relkey`]). Migrating means finding, for every v1 entry, where its
//! file is relative to the library *now*:
//!
//! 1. **Under the root**: the old path starts with the current library path
//!    (compared component by component, NFC-normalised, `/` and `\` both
//!    accepted, case-insensitively as a fallback for Windows drive letters
//!    and case-insensitive volumes). The rest of the path is the key.
//! 2. **Relocated**: otherwise, the library folder's own name (`Alice`)
//!    appears in the old path (`/Volumes/books/Alice/Herbert/Dune.epub`
//!    seen from `/Volumes/books-1/Alice`): the part after it is the key.
//!    If the name appears several times, the occurrence whose file exists
//!    wins (deepest first); without any, the deepest one.
//! 3. **Missing**: otherwise the entry can't be placed. It is kept in the
//!    index's `missing` section (under its old path), so the next scan can
//!    re-link it if a file with the same name and size shows up in the
//!    library — and it stays in the v1 backup file in any case.
//!
//! Two v1 entries can land on the same key (the same file indexed under two
//! mount names). The better match is kept: "under the
//! root" beats "relocated", then the most recently added. The other one is
//! kept as missing rather than dropped.
//!
//! Entries that aren't book records at all (hand edits, truncated values)
//! are skipped and reported; the caller backs the v1 file up before writing
//! v2, so nothing is lost.
//!
//! This module is pure apart from the `exists` probe, so every case is unit
//! tested without a real NAS.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::path::Path;

use super::relkey::{foreign_components, normalize_key, resolve};
use crate::models::book::Book;

/// How a v1 entry was placed; the lower, the more trustworthy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Placement {
    UnderRoot,
    Relocated,
}

/// What happened to the v1 entries, for the log.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct MigrationReport {
    /// Entries whose old path was under the current library path.
    pub(crate) under_root: usize,
    /// Entries mapped through the library folder's name (library moved, or
    /// mounted under another path).
    pub(crate) relocated: usize,
    /// Entries that couldn't be placed in the library (kept as missing).
    pub(crate) unplaced: usize,
    /// Extra entries for a file another entry already claimed (kept as missing).
    pub(crate) duplicates: usize,
    /// `old key: reason` for each entry that isn't a book record (skipped).
    pub(crate) skipped: Vec<String>,
}

impl MigrationReport {
    /// One line for the debug log.
    pub(crate) fn summary(&self) -> String {
        let mut line = format!(
            "{} book(s) migrated ({} at the same place, {} relocated), {} kept as missing",
            self.under_root + self.relocated,
            self.under_root,
            self.relocated,
            self.unplaced + self.duplicates,
        );
        if self.duplicates > 0 {
            line.push_str(&format!(" ({} duplicate entries)", self.duplicates));
        }
        if !self.skipped.is_empty() {
            line.push_str(&format!(
                ", {} invalid entries skipped (still in the v1 backup): {}",
                self.skipped.len(),
                self.skipped.join("; ")
            ));
        }
        line
    }
}

/// The result of migrating a v1 index.
#[derive(Debug, Default)]
pub(crate) struct V1Migration {
    /// Key → book; `Book.path` is the absolute path under the current root.
    pub(crate) books: BTreeMap<String, Book>,
    /// Old v1 key → book, for entries that couldn't be placed (or lost a
    /// key collision). `Book.path` is left as it was.
    pub(crate) missing: BTreeMap<String, Book>,
    pub(crate) report: MigrationReport,
}

/// Migrate the entries of a v1 index (`old absolute path → book`) to keys
/// relative to `root`. `exists` tells whether a file exists (only used to
/// choose between several possible places; pass `|p| p.exists()`).
///
/// Placeholders stored as data ("Unknown title", "Unknown author") are
/// **not** cleared here; the index loader does that for every
/// format.
pub(crate) fn migrate_v1(
    root: &Path,
    entries: serde_json::Map<String, serde_json::Value>,
    exists: &dyn Fn(&Path) -> bool,
) -> V1Migration {
    let root_parts = foreign_components(&root.to_string_lossy());
    let mut out = V1Migration::default();
    // key → (placement, v1 key) of the entry currently holding that key.
    let mut claimed: BTreeMap<String, (Placement, String)> = BTreeMap::new();

    for (old_key, value) in entries {
        let mut book: Book = match serde_json::from_value(value) {
            Ok(book) => book,
            Err(e) => {
                out.report.skipped.push(format!("{old_key}: {e}"));
                continue;
            }
        };
        // v1 stored the path both as the key and in the record; they are
        // always equal, but fall back on the key if the record has none.
        let old_path = if book.path.trim().is_empty() {
            old_key.clone()
        } else {
            book.path.clone()
        };

        let Some((rel, placement)) = locate(root, &root_parts, &old_path, exists) else {
            out.report.unplaced += 1;
            out.missing.insert(old_key, book);
            continue;
        };
        let key = normalize_key(&rel);
        let Some(absolute) = resolve(root, &rel) else {
            // `locate` only returns valid keys; be defensive anyway.
            out.report.unplaced += 1;
            out.missing.insert(old_key, book);
            continue;
        };
        book.path = absolute.to_string_lossy().into_owned();

        match claimed.get(&key) {
            None => {
                count(&mut out.report, placement);
                claimed.insert(key.clone(), (placement, old_key));
                out.books.insert(key, book);
            }
            Some((held_placement, held_old_key)) => {
                let held = &out.books[&key];
                if beats(placement, &book, *held_placement, held) {
                    // The new entry wins: the one it displaces becomes missing.
                    let (held_placement, held_old_key) = (*held_placement, held_old_key.clone());
                    let mut loser = out.books.remove(&key).expect("claimed keys are in books");
                    uncount(&mut out.report, held_placement);
                    loser.path = held_old_key.clone();
                    out.missing.insert(held_old_key, loser);
                    count(&mut out.report, placement);
                    claimed.insert(key.clone(), (placement, old_key));
                    out.books.insert(key, book);
                } else {
                    book.path = old_path;
                    out.missing.insert(old_key, book);
                }
                out.report.duplicates += 1;
            }
        }
    }
    out
}

fn count(report: &mut MigrationReport, placement: Placement) {
    match placement {
        Placement::UnderRoot => report.under_root += 1,
        Placement::Relocated => report.relocated += 1,
    }
}

fn uncount(report: &mut MigrationReport, placement: Placement) {
    match placement {
        Placement::UnderRoot => report.under_root -= 1,
        Placement::Relocated => report.relocated -= 1,
    }
}

/// Whether entry `a` should hold a key rather than `b`: the better
/// placement, then the most recently added (the record a recent version of
/// the app was using).
fn beats(a_placement: Placement, a: &Book, b_placement: Placement, b: &Book) -> bool {
    match a_placement.cmp(&b_placement) {
        Ordering::Less => true,
        Ordering::Greater => false,
        Ordering::Equal => date_added(a) > date_added(b),
    }
}

fn date_added(book: &Book) -> Option<chrono::DateTime<chrono::FixedOffset>> {
    chrono::DateTime::parse_from_rfc3339(&book.date_added).ok()
}

/// Where the file of `old_path` is relative to `root` (see the module docs),
/// and how sure we are.
fn locate(
    root: &Path,
    root_parts: &[String],
    old_path: &str,
    exists: &dyn Fn(&Path) -> bool,
) -> Option<(String, Placement)> {
    let old_parts = foreign_components(old_path);
    let valid = |rel: String| resolve(root, &rel).map(|_| rel);

    // 1. Under the current root.
    if old_parts.len() > root_parts.len() {
        let prefix = &old_parts[..root_parts.len()];
        let exact = prefix == root_parts;
        let folded = !exact
            && prefix
                .iter()
                .zip(root_parts)
                .all(|(a, b)| a.to_lowercase() == b.to_lowercase());
        if exact || folded {
            if let Some(rel) = valid(old_parts[root_parts.len()..].join("/")) {
                return Some((rel, Placement::UnderRoot));
            }
        }
    }

    // 2. Relocated: after an occurrence of the library folder's name.
    let name = root_parts.last()?;
    let last_dir = old_parts.len().checked_sub(1)?; // the file name itself doesn't count
    let mut occurrences: Vec<usize> = (0..last_dir).filter(|&i| &old_parts[i] == name).collect();
    if occurrences.is_empty() {
        let folded = name.to_lowercase();
        occurrences = (0..last_dir)
            .filter(|&i| old_parts[i].to_lowercase() == folded)
            .collect();
    }
    let candidates: Vec<String> = occurrences
        .iter()
        .rev()
        .filter_map(|&i| valid(old_parts[i + 1..].join("/")))
        .collect();
    let chosen = match candidates.len() {
        0 => return None,
        1 => candidates[0].clone(),
        _ => candidates
            .iter()
            .find(|rel| resolve(root, rel).is_some_and(|p| exists(&p)))
            .unwrap_or(&candidates[0])
            .clone(),
    };
    Some((chosen, Placement::Relocated))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use std::path::PathBuf;

    const NFD_E: &str = "e\u{301}";
    const NFC_E: &str = "\u{e9}";

    fn record(id: &str, path: &str, date: &str) -> Value {
        json!({
            "id": id, "path": path, "filename": path.rsplit(['/', '\\']).next().unwrap(),
            "format": "epub", "sizeBytes": 10, "dateAdded": date,
            "metadata": { "title": id, "author": "A" },
        })
    }

    fn v1(entries: &[(&str, &str)]) -> serde_json::Map<String, Value> {
        entries
            .iter()
            .map(|(id, path)| (path.to_string(), record(id, path, "2026-01-01T00:00:00Z")))
            .collect()
    }

    fn never(_: &Path) -> bool {
        false
    }

    fn keys(m: &V1Migration) -> Vec<&str> {
        m.books.keys().map(String::as_str).collect()
    }

    #[test]
    fn entries_under_the_root_keep_their_relative_path() {
        let root = Path::new("/Volumes/books/Alice");
        let m = migrate_v1(
            root,
            v1(&[
                ("a", "/Volumes/books/Alice/Dune.epub"),
                ("b", "/Volumes/books/Alice/Herbert/Messiah.epub"),
            ]),
            &never,
        );
        assert_eq!(keys(&m), ["Dune.epub", "Herbert/Messiah.epub"]);
        assert_eq!(m.books["Dune.epub"].id, "a");
        assert_eq!(
            PathBuf::from(&m.books["Herbert/Messiah.epub"].path),
            root.join("Herbert").join("Messiah.epub")
        );
        assert_eq!(m.report.under_root, 2);
        assert!(m.missing.is_empty());
    }

    #[test]
    fn a_moved_mount_point_is_followed_through_the_library_folder_name() {
        // macOS mounted the share as "books-1" because "books" was taken.
        let root = Path::new("/Volumes/books-1/Alice");
        let m = migrate_v1(
            root,
            v1(&[
                ("a", "/Volumes/books/Alice/Dune.epub"),
                ("b", "/Volumes/books/Alice/Saga/T1.epub"),
            ]),
            &never,
        );
        assert_eq!(keys(&m), ["Dune.epub", "Saga/T1.epub"]);
        assert_eq!(
            PathBuf::from(&m.books["Saga/T1.epub"].path),
            root.join("Saga").join("T1.epub")
        );
        assert_eq!(m.report.relocated, 2);
    }

    #[test]
    fn windows_and_mixed_separators() {
        // The same NAS library, indexed from a Windows PC, opened on a Mac.
        let root = Path::new("/Volumes/books/Alice");
        let m = migrate_v1(
            root,
            v1(&[
                ("unc", r"\\nas\books\Alice\Herbert\Dune.epub"),
                ("drive", r"Z:\Alice\Asimov\Foundation.epub"),
                ("mixed", r"\\nas\books/Alice\Mixed/Book.epub"),
            ]),
            &never,
        );
        assert_eq!(
            keys(&m),
            [
                "Asimov/Foundation.epub",
                "Herbert/Dune.epub",
                "Mixed/Book.epub"
            ]
        );
        assert_eq!(m.report.relocated, 3);
    }

    #[test]
    fn windows_drive_letters_match_case_insensitively() {
        let root = Path::new(r"C:\Users\me\Librarium\Alice");
        let m = migrate_v1(
            root,
            v1(&[("a", r"c:\users\ME\Librarium\Alice\Dune.epub")]),
            &never,
        );
        assert_eq!(keys(&m), ["Dune.epub"]);
        assert_eq!(m.report.under_root, 1);
    }

    #[test]
    fn nfd_names_become_nfc_keys() {
        // macOS/SMB reported the decomposed spelling; the root was typed precomposed.
        let root = PathBuf::from(format!("/Volumes/books/Ren{NFC_E}e"));
        let old = format!("/Volumes/books/Ren{NFD_E}e/Caf{NFD_E}/{NFD_E}t{NFD_E}.epub");
        let m = migrate_v1(&root, v1(&[("a", &old)]), &never);
        assert_eq!(
            keys(&m),
            [format!("Caf{NFC_E}/{NFC_E}t{NFC_E}.epub").as_str()]
        );
        assert_eq!(m.report.under_root, 1);
    }

    #[test]
    fn unplaceable_entries_are_kept_as_missing() {
        let root = Path::new("/Volumes/books/Alice");
        // Another user's book (mixed in by an old bug), and a path with no
        // trace of the library folder.
        let m = migrate_v1(
            root,
            v1(&[
                ("other", "/Volumes/books/Milo/x.epub"),
                ("lost", "/tmp/y.epub"),
                ("ok", "/Volumes/books/Alice/z.epub"),
            ]),
            &never,
        );
        assert_eq!(keys(&m), ["z.epub"]);
        assert_eq!(m.missing.len(), 2);
        assert_eq!(m.missing["/Volumes/books/Milo/x.epub"].id, "other");
        assert_eq!(
            m.missing["/tmp/y.epub"].path, "/tmp/y.epub",
            "the old path is kept for reference"
        );
        assert_eq!(m.report.unplaced, 2);
    }

    #[test]
    fn paths_climbing_out_of_the_library_are_not_trusted() {
        let root = Path::new("/Volumes/books/Alice");
        let m = migrate_v1(
            root,
            v1(&[("evil", "/Volumes/books/Alice/../Milo/x.epub")]),
            &never,
        );
        assert!(m.books.is_empty());
        assert_eq!(m.report.unplaced, 1);
    }

    #[test]
    fn garbage_entries_are_skipped_and_reported() {
        let root = Path::new("/lib/Alice");
        let mut entries = v1(&[("ok", "/lib/Alice/a.epub")]);
        entries.insert("/lib/Alice/broken.epub".into(), json!({ "id": "broken" }));
        entries.insert("version".into(), json!(42));
        let m = migrate_v1(root, entries, &never);
        assert_eq!(keys(&m), ["a.epub"]);
        assert_eq!(m.report.skipped.len(), 2);
        assert!(m
            .report
            .skipped
            .iter()
            .any(|s| s.starts_with("/lib/Alice/broken.epub:")));
        assert!(m.report.summary().contains("2 invalid entries skipped"));
    }

    #[test]
    fn several_occurrences_of_the_folder_name_prefer_the_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Alice");
        std::fs::create_dir_all(root.join("Alice")).unwrap();
        std::fs::write(root.join("Alice").join("notes.epub"), b"x").unwrap();
        // Old library: /old/Alice, holding a sub-folder also named Alice.
        let entries = v1(&[
            ("a", "/old/Alice/Alice/notes.epub"),
            ("b", "/old/Alice/Alice/gone.epub"),
        ]);
        let m = migrate_v1(&root, entries, &|p| p.exists());
        // notes.epub exists at Alice/notes.epub (the shallower occurrence);
        // gone.epub exists nowhere, so the deepest occurrence is used.
        assert_eq!(keys(&m), ["Alice/notes.epub", "gone.epub"]);
    }

    #[test]
    fn colliding_entries_keep_the_best_match_and_the_other_as_missing() {
        let root = Path::new("/Volumes/books-1/Alice");
        let mut entries = serde_json::Map::new();
        // The same file, indexed under the old and the current mount name.
        entries.insert(
            "/Volumes/books/Alice/Dune.epub".into(),
            record(
                "old",
                "/Volumes/books/Alice/Dune.epub",
                "2026-03-01T00:00:00Z",
            ),
        );
        entries.insert(
            "/Volumes/books-1/Alice/Dune.epub".into(),
            record(
                "current",
                "/Volumes/books-1/Alice/Dune.epub",
                "2026-01-01T00:00:00Z",
            ),
        );
        // Two relocated entries for one file: the most recently added wins.
        entries.insert(
            "/a/Alice/T.epub".into(),
            record("older", "/a/Alice/T.epub", "2025-01-01T00:00:00Z"),
        );
        entries.insert(
            "/b/Alice/T.epub".into(),
            record("newer", "/b/Alice/T.epub", "2026-06-01T00:00:00+02:00"),
        );
        let m = migrate_v1(root, entries, &never);

        assert_eq!(
            m.books["Dune.epub"].id, "current",
            "under the root beats relocated, whatever the dates"
        );
        assert_eq!(m.books["T.epub"].id, "newer");
        assert_eq!(m.missing["/Volumes/books/Alice/Dune.epub"].id, "old");
        assert_eq!(m.missing["/a/Alice/T.epub"].id, "older");
        assert_eq!(m.report.duplicates, 2);
        assert_eq!(
            m.report.under_root + m.report.relocated,
            2,
            "each key counted once"
        );
    }

    #[test]
    fn a_record_without_path_uses_its_key() {
        let root = Path::new("/lib/Alice");
        let mut entries = serde_json::Map::new();
        let mut rec = record("a", "/lib/Alice/a.epub", "2026-01-01T00:00:00Z");
        rec["path"] = json!("");
        entries.insert("/lib/Alice/a.epub".into(), rec);
        let m = migrate_v1(root, entries, &never);
        assert_eq!(keys(&m), ["a.epub"]);
    }
}
