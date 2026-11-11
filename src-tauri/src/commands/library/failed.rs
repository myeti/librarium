//! `.librarium/failed.json`: the files of the library that couldn't be read
//! as books, so that scans stop retrying them.
//!
//! A scan parses every new file it finds (the OPF of an EPUB, the Info
//! dictionary of a PDF). Two kinds of files would otherwise be parsed again
//! on every scan, forever:
//! - a file that makes a parser **panic**: before this list, the panic failed
//!   the whole scan, so one bad book stopped the library from opening at all
//!   (and did so again on every launch);
//! - a file that **isn't the format its name claims** (an HTML error page
//!   saved as `.epub`, a 0-byte or truncated download).
//!
//! Such a file is recorded here with its size and modification time, logged,
//! and left out of the library. Scans skip it — without opening it — until
//! its size or modification time changes: a repaired or replaced file is
//! tried again. Errors that may be temporary (a read error on a NAS share)
//! are never recorded; the next scan simply retries.
//!
//! ```json
//! { "Herbert/Dune.epub": { "size": 123456, "mtime": 1730000000000, "error": "…" } }
//! ```
//!
//! - Keys are index keys (paths relative to the library, NFC; see
//!   [`super::relkey`]). `mtime` is in milliseconds since the Unix epoch.
//! - Entries for files that are no longer on disk, or that became books, are
//!   dropped by the next complete scan.
//! - The file is a cache, not user data: a damaged one is treated as empty
//!   (and logged), and deleting it only makes the next scan retry every
//!   failed file once.
//! - It is written atomically ([`fsutil::atomic_write`]), only when something
//!   changed, and only under the library's index lock
//!   ([`with_index`](super::index::with_index)), which serialises the scans
//!   and imports of a library.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use super::index::INDEX_DIR;
use crate::error::{AppError, AppResult};
use crate::fsutil;
use crate::logging::Log;

/// File name of the list, in `<library>/.librarium/`.
pub(crate) const FAILED_FILE: &str = "failed.json";

/// What identifies one version of a file: if either value changes, the file
/// is tried again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Fingerprint {
    pub(crate) size: u64,
    /// Modification time in milliseconds since the Unix epoch (0 when the
    /// filesystem doesn't report one).
    pub(crate) mtime: i64,
}

impl Fingerprint {
    /// The fingerprint of a file from its metadata.
    pub(crate) fn of(meta: &fs::Metadata) -> Self {
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
            .unwrap_or(0);
        Self {
            size: meta.len(),
            mtime,
        }
    }

    /// The fingerprint of the file at `path`.
    pub(crate) fn read(path: &Path) -> io::Result<Self> {
        fs::metadata(path).map(|meta| Self::of(&meta))
    }
}

/// One entry of `failed.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct FailedEntry {
    pub(crate) size: u64,
    pub(crate) mtime: i64,
    /// Why the file was left out, for the log and (later) the UI.
    pub(crate) error: String,
}

impl FailedEntry {
    fn matches(&self, fp: Fingerprint) -> bool {
        self.size == fp.size && self.mtime == fp.mtime
    }
}

/// The list of a library, loaded in memory; [`save`](Self::save) writes it
/// back if it changed.
#[derive(Debug)]
pub(crate) struct FailedFiles {
    path: PathBuf,
    entries: BTreeMap<String, FailedEntry>,
    dirty: bool,
}

impl FailedFiles {
    /// The list of `library` (empty when there is none). A file that can't
    /// be read or parsed is logged and treated as empty: it is rebuilt by the
    /// next scan.
    pub(crate) fn load(library: &Path, log: Log) -> Self {
        let path = library.join(INDEX_DIR).join(FAILED_FILE);
        let entries = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                log(format!("[library] WARNING — {FAILED_FILE} is damaged ({e}); every file that failed before is tried again"));
                BTreeMap::new()
            }),
            Err(e) if e.kind() == io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => {
                log(format!("[library] WARNING — could not read {FAILED_FILE} ({e}); every file that failed before is tried again"));
                BTreeMap::new()
            }
        };
        Self {
            path,
            entries,
            dirty: false,
        }
    }

    /// Whether the file at `key`, in the version `fp`, already failed: then
    /// it is skipped without being opened.
    pub(crate) fn is_known_failure(&self, key: &str, fp: Fingerprint) -> bool {
        self.entries.get(key).is_some_and(|entry| entry.matches(fp))
    }

    /// The recorded failure of `key`, whatever the version of the file.
    pub(crate) fn get(&self, key: &str) -> Option<&FailedEntry> {
        self.entries.get(key)
    }

    /// Remember that the file at `key`, in the version `fp`, can't be read.
    pub(crate) fn record(&mut self, key: &str, fp: Fingerprint, error: &str) {
        let entry = FailedEntry {
            size: fp.size,
            mtime: fp.mtime,
            error: error.to_string(),
        };
        if self.entries.get(key) != Some(&entry) {
            self.entries.insert(key.to_string(), entry);
            self.dirty = true;
        }
    }

    /// Forget the failure of `key` (the file was read after all).
    pub(crate) fn clear(&mut self, key: &str) {
        if self.entries.remove(key).is_some() {
            self.dirty = true;
        }
    }

    /// Keep only the entries `keep` accepts (after a complete scan: the files
    /// still on disk and still not in the library). Returns how many were
    /// dropped.
    pub(crate) fn retain(&mut self, mut keep: impl FnMut(&str) -> bool) -> usize {
        let before = self.entries.len();
        self.entries.retain(|key, _| keep(key));
        let dropped = before - self.entries.len();
        if dropped > 0 {
            self.dirty = true;
        }
        dropped
    }

    /// Number of recorded failures.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// Write the list if it changed: atomically, or by removing the file
    /// when the list is empty (no `failed.json` in a healthy library).
    pub(crate) fn save(&mut self) -> AppResult<()> {
        if !self.dirty {
            return Ok(());
        }
        if self.entries.is_empty() {
            match fs::remove_file(&self.path) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(AppError::io(format!("Could not update {FAILED_FILE}"), &e)),
            }
        } else {
            if let Some(dir) = self.path.parent() {
                fs::create_dir_all(dir).map_err(|e| {
                    AppError::io(format!("Could not create the {INDEX_DIR} folder"), &e)
                })?;
            }
            let json = serde_json::to_vec_pretty(&self.entries)
                .map_err(|e| AppError::Internal(e.to_string()))?;
            fsutil::atomic_write(&self.path, &json)
                .map_err(|e| AppError::io(format!("Could not save {FAILED_FILE}"), &e))?;
        }
        self.dirty = false;
        Ok(())
    }

    /// [`save`](Self::save), logging a failure instead of returning it: the
    /// list is a cache, and a scan or import must not fail because of it.
    pub(crate) fn save_or_log(&mut self, log: Log) {
        if let Err(e) = self.save() {
            log(format!("[library] WARNING — {e}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging::{no_log, LogLines};

    fn fp(size: u64, mtime: i64) -> Fingerprint {
        Fingerprint { size, mtime }
    }

    #[test]
    fn records_round_trip_and_match_only_the_same_version() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        let mut failed = FailedFiles::load(lib, &no_log);
        assert_eq!(failed.len(), 0);
        failed.record("Herbert/Dune.epub", fp(10, 1_000), "not an EPUB file");
        failed.save().unwrap();

        let path = lib.join(INDEX_DIR).join(FAILED_FILE);
        let json: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "Herbert/Dune.epub": { "size": 10, "mtime": 1000, "error": "not an EPUB file" } })
        );

        let failed = FailedFiles::load(lib, &no_log);
        assert!(failed.is_known_failure("Herbert/Dune.epub", fp(10, 1_000)));
        assert!(
            !failed.is_known_failure("Herbert/Dune.epub", fp(11, 1_000)),
            "a new size is tried again"
        );
        assert!(
            !failed.is_known_failure("Herbert/Dune.epub", fp(10, 2_000)),
            "a new mtime is tried again"
        );
        assert!(!failed.is_known_failure("Other.epub", fp(10, 1_000)));
    }

    #[test]
    fn an_empty_list_removes_the_file_and_unchanged_lists_are_not_written() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        let path = lib.join(INDEX_DIR).join(FAILED_FILE);
        let mut failed = FailedFiles::load(lib, &no_log);
        failed.save().unwrap();
        assert!(!path.exists(), "nothing to write, nothing written");

        failed.record("a.epub", fp(1, 1), "bad");
        failed.save().unwrap();
        assert!(path.exists());
        failed.record("a.epub", fp(1, 1), "bad");
        assert!(
            !failed.dirty,
            "recording the same failure again changes nothing"
        );

        assert_eq!(failed.retain(|_| true), 0);
        failed.clear("a.epub");
        failed.save().unwrap();
        assert!(!path.exists(), "a healthy library has no failed.json");
    }

    #[test]
    fn a_damaged_list_is_treated_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        fs::create_dir_all(lib.join(INDEX_DIR)).unwrap();
        fs::write(lib.join(INDEX_DIR).join(FAILED_FILE), b"{ not json").unwrap();
        let lines = LogLines::default();
        let failed = FailedFiles::load(lib, &lines.sink());
        assert_eq!(failed.len(), 0);
        assert!(lines.contains("damaged"));
    }

    #[test]
    fn fingerprints_follow_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.epub");
        fs::write(&path, b"12345").unwrap();
        let first = Fingerprint::read(&path).unwrap();
        assert_eq!(first.size, 5);
        assert!(first.mtime > 0);
        fs::write(&path, b"123456").unwrap();
        assert_ne!(Fingerprint::read(&path).unwrap(), first);
    }
}
