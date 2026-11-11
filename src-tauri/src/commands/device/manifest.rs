//! The device manifest, `<mount>/.librarium/device.json`: which library book
//! each file Librarium put on this e-reader is.
//!
//! ```jsonc
//! {
//!   "version": 1,
//!   "deviceId": "<uuid>",                  // this device's stable id (see detect.rs)
//!   "books": {
//!     "<libraryBookId>": {
//!       "path": "documents/Dune.azw3",     // relative to the mount, '/' separators
//!       "title": "Dune", "author": "Frank Herbert",
//!       "format": "azw3", "sizeBytes": 123456,
//!       "sentAt": "2026-11-06T10:00:00+00:00"
//!     }
//!   }
//! }
//! ```
//!
//! It is what makes device operations safe without title matching:
//! - re-sending a book replaces *that* book's file (and only deletes the old
//!   one when the manifest says it is this book's, see `transfer.rs`);
//! - the library view knows which library book each file is (`libraryId`);
//! - deleting a book from the device forgets its entry.
//!
//! Rules:
//! - Written atomically ([`fsutil::atomic_write`]) under this process's lock
//!   for the file ([`update`]), after each successful transfer and delete.
//! - Never trusted blindly: an entry whose path is absolute, contains `..`
//!   or isn't a plain relative path is dropped on load, and every path is
//!   checked again ([`fsutil::is_within`]) before anything is deleted.
//! - A manifest written by a newer Librarium (`version` > 1) is read but
//!   never rewritten. A damaged one is moved aside
//!   (`device.json.damaged-<time>`) before a new one is written: it only
//!   describes files, which stay on the device either way.
//! - Unknown top-level fields are kept.
//!
//! Paths are compared with [`path_key`] (NFC, lower case): e-readers are
//! FAT/exFAT volumes, where `Dune.azw3` and `dune.azw3` are the same file.
//!
//! Validated on a Kindle Paperwhite 3 over USB on macOS (FAT32), 2026-11-09: the Kindle doesn't show the hidden `.librarium/` folder (it only
//! indexes `documents/`).
//! TODO(device-validation): a Kobo (it should skip dot-folders).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use unicode_normalization::UnicodeNormalization;

use crate::commands::library::covers::is_safe_id;
use crate::fsutil;
use crate::logging::Log;

/// Folder (at the device root) holding Librarium's files.
pub(crate) const MANIFEST_DIR: &str = ".librarium";
/// The manifest's file name in [`MANIFEST_DIR`].
pub(crate) const MANIFEST_FILE: &str = "device.json";
/// The format version this code writes.
pub(crate) const MANIFEST_VERSION: u64 = 1;

/// `<mount>/.librarium/device.json`.
pub(crate) fn manifest_path(mount: &Path) -> PathBuf {
    mount.join(MANIFEST_DIR).join(MANIFEST_FILE)
}

/// What Librarium knows about one file it sent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManifestEntry {
    /// Relative to the mount, `/` separators.
    pub path: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub author: String,
    /// The file's extension (`azw3`, `epub`…).
    #[serde(default)]
    pub format: String,
    #[serde(default)]
    pub size_bytes: u64,
    /// RFC 3339.
    #[serde(default)]
    pub sent_at: String,
}

/// The manifest's content (see the module docs).
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct DeviceManifest {
    /// This device's id: a UUID created the first time Librarium saw it.
    pub device_id: Option<String>,
    /// By library book id.
    pub books: BTreeMap<String, ManifestEntry>,
    /// Top-level fields this version doesn't know, written back unchanged.
    pub extra: Map<String, Value>,
}

impl DeviceManifest {
    /// The entry of library book `id`.
    pub(crate) fn entry(&self, id: &str) -> Option<&ManifestEntry> {
        self.books.get(id)
    }

    /// The id of the library book whose file is at `rel` (any spelling of
    /// it: see [`path_key`]).
    pub(crate) fn owner_of(&self, rel: &str) -> Option<&str> {
        let key = path_key(rel);
        self.books
            .iter()
            .find(|(_, e)| path_key(&e.path) == key)
            .map(|(id, _)| id.as_str())
    }

    /// Forget every entry whose file is at `rel`; returns the ids removed.
    pub(crate) fn remove_path(&mut self, rel: &str) -> Vec<String> {
        let key = path_key(rel);
        let ids: Vec<String> = self
            .books
            .iter()
            .filter(|(_, e)| path_key(&e.path) == key)
            .map(|(id, _)| id.clone())
            .collect();
        for id in &ids {
            self.books.remove(id);
        }
        ids
    }

    fn to_json(&self) -> Value {
        let mut map = self.extra.clone();
        map.insert("version".into(), Value::from(MANIFEST_VERSION));
        if let Some(id) = &self.device_id {
            map.insert("deviceId".into(), Value::from(id.clone()));
        }
        let books = self
            .books
            .iter()
            .map(|(id, e)| {
                (
                    id.clone(),
                    serde_json::to_value(e).expect("a manifest entry always serialises"),
                )
            })
            .collect();
        map.insert("books".into(), Value::Object(books));
        Value::Object(map)
    }
}

/// How the manifest file was found.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ManifestState {
    /// No manifest yet (a device Librarium never wrote to).
    Absent,
    /// Read normally.
    Valid,
    /// Not readable as a manifest (the reason). Treated as empty; moved
    /// aside before a new one is written.
    Damaged(String),
    /// Written by a newer Librarium (its version): read, never rewritten.
    TooNew(u64),
}

/// A manifest as loaded by [`load`].
#[derive(Debug, Clone)]
pub(crate) struct Loaded {
    pub manifest: DeviceManifest,
    pub state: ManifestState,
    /// Entries left out because their id or path isn't safe to use.
    pub dropped: usize,
}

/// Read the manifest of the device at `mount`. Never fails: a missing or
/// unreadable manifest reads as empty (see [`Loaded::state`]).
pub(crate) fn load(mount: &Path) -> Loaded {
    let path = manifest_path(mount);
    match fs::read(&path) {
        Ok(bytes) => parse(&bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Loaded::empty(ManifestState::Absent),
        Err(e) => Loaded::empty(ManifestState::Damaged(format!("can't be read: {e}"))),
    }
}

impl Loaded {
    pub(crate) fn empty(state: ManifestState) -> Self {
        Loaded {
            manifest: DeviceManifest::default(),
            state,
            dropped: 0,
        }
    }
}

/// Parse a manifest's bytes (see [`load`]; also used for the copy an MTP
/// device keeps, which has no mount to read it from).
pub(crate) fn parse(bytes: &[u8]) -> Loaded {
    let empty = Loaded::empty;
    let value: Value = match serde_json::from_slice(bytes) {
        Ok(value) => value,
        Err(e) => return empty(ManifestState::Damaged(format!("isn't valid JSON: {e}"))),
    };
    let Value::Object(mut map) = value else {
        return empty(ManifestState::Damaged("isn't a JSON object".into()));
    };
    let version = match map.remove("version") {
        None => MANIFEST_VERSION,
        Some(v) => match v.as_u64() {
            Some(v) => v,
            None => return empty(ManifestState::Damaged("has no valid version".into())),
        },
    };
    let device_id = map
        .remove("deviceId")
        .and_then(|v| v.as_str().map(str::to_string))
        .filter(|id| is_safe_id(id));
    let mut books = BTreeMap::new();
    let mut dropped = 0;
    match map.remove("books") {
        None => {}
        Some(Value::Object(entries)) => {
            for (id, raw) in entries {
                // One bad entry costs only itself.
                match serde_json::from_value::<ManifestEntry>(raw) {
                    Ok(entry) if is_safe_id(&id) && is_plain_relative(&entry.path) => {
                        books.insert(id, entry);
                    }
                    _ => dropped += 1,
                }
            }
        }
        Some(_) => {
            return empty(ManifestState::Damaged(
                "its \"books\" isn't an object".into(),
            ))
        }
    }
    let state = if version > MANIFEST_VERSION {
        ManifestState::TooNew(version)
    } else {
        ManifestState::Valid
    };
    Loaded {
        manifest: DeviceManifest {
            device_id,
            books,
            extra: map,
        },
        state,
        dropped,
    }
}

/// Read-modify-write the manifest of the device at `mount`, under this
/// process's lock for the file: `f` gets the current content (empty if
/// there was none, or it was damaged), and the result is written atomically
/// if `f` changed anything. Returns what `f` returned.
///
/// Fails without writing when the manifest was written by a newer
/// Librarium, or when the write fails (read-only or full volume, unplugged
/// device). A damaged manifest is moved aside first (logged).
pub(crate) fn update<T>(
    mount: &Path,
    log: Log,
    f: impl FnOnce(&mut DeviceManifest) -> T,
) -> Result<T, String> {
    let path = manifest_path(mount);
    fsutil::with_file_lock(&path, || {
        let loaded = load(mount);
        if let ManifestState::TooNew(version) = loaded.state {
            return Err(format!(
                "the device manifest was written by a newer Librarium (format {version}); update Librarium"
            ));
        }
        let mut manifest = loaded.manifest;
        let before = manifest.clone();
        let out = f(&mut manifest);
        if manifest == before {
            // Nothing to record: no manifest is created (or rewritten, or
            // moved aside) for nothing.
            return Ok(out);
        }
        if let ManifestState::Damaged(reason) = &loaded.state {
            let aside = path.with_file_name(format!(
                "{MANIFEST_FILE}.damaged-{}",
                chrono::Utc::now().format("%Y%m%dT%H%M%SZ")
            ));
            match fs::rename(&path, &aside) {
                Ok(()) => log(format!("[device manifest] WARNING — {MANIFEST_FILE} {reason}; kept aside as {}", aside.display())),
                Err(e) => log(format!("[device manifest] WARNING — {MANIFEST_FILE} {reason}, and could not be moved aside ({e})")),
            }
        }
        save(mount, &manifest).map_err(|e| format!("could not write the device manifest: {e}"))?;
        Ok(out)
    })
}

fn save(mount: &Path, manifest: &DeviceManifest) -> std::io::Result<()> {
    let path = manifest_path(mount);
    fs::create_dir_all(path.parent().expect("the manifest path has a parent"))?;
    fsutil::atomic_write(&path, &manifest.to_bytes())
}

impl DeviceManifest {
    /// The file's bytes: pretty JSON and a final newline.
    pub(crate) fn to_bytes(&self) -> Vec<u8> {
        let mut json =
            serde_json::to_vec_pretty(&self.to_json()).expect("a JSON value always serialises");
        json.push(b'\n');
        json
    }
}

/// The device id stored in the manifest of the device at `mount`, created
/// (a new UUID, written to the manifest) if it has none yet. Fails when the
/// manifest is damaged or too new and has no id (it is not rewritten just
/// for an id), or can't be written.
pub(crate) fn ensure_device_id(mount: &Path, log: Log) -> Result<String, String> {
    let loaded = load(mount);
    if let Some(id) = loaded.manifest.device_id {
        return Ok(id);
    }
    match loaded.state {
        ManifestState::Absent | ManifestState::Valid => {}
        ManifestState::Damaged(reason) => return Err(format!("the device manifest {reason}")),
        ManifestState::TooNew(v) => {
            return Err(format!(
                "the device manifest has format {v} and no device id"
            ))
        }
    }
    update(mount, log, |m| {
        m.device_id
            .get_or_insert_with(|| uuid::Uuid::new_v4().to_string())
            .clone()
    })
}

// ── Paths ─────────────────────────────────────────────────────────────────────

/// `path` relative to `mount`, with `/` separators (how the manifest stores
/// paths). `None` if `path` isn't under `mount` (compared as written; both
/// come from the same mount string).
pub(crate) fn relative_path(mount: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(mount).ok()?;
    let parts: Vec<String> = rel
        .components()
        .map(|c| match c {
            Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Option<_>>()?;
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// The absolute path of the manifest path `rel` on the device at `mount`,
/// or `None` if `rel` isn't a plain relative path (empty, absolute, a drive
/// letter, `.` or `..` components). Doesn't touch the disk: callers that
/// delete check [`fsutil::is_within`] as well.
pub(crate) fn resolve(mount: &Path, rel: &str) -> Option<PathBuf> {
    let rel = rel.trim();
    if rel.is_empty() || rel.starts_with('/') || rel.starts_with('\\') || rel.contains(':') {
        return None;
    }
    let mut path = mount.to_path_buf();
    for part in rel.split(['/', '\\']) {
        if part.is_empty() || part.chars().all(|c| c == '.') {
            return None;
        }
        path.push(part);
    }
    Some(path)
}

/// Whether `rel` is a plain relative path (what [`resolve`] accepts).
pub(crate) fn is_plain_relative(rel: &str) -> bool {
    resolve(Path::new(""), rel).is_some()
}

/// How two spellings of a device path are compared: `/` separators,
/// Unicode NFC (macOS can report NFD names) and lower case (FAT and exFAT
/// are case-insensitive).
pub(crate) fn path_key(rel: &str) -> String {
    rel.replace('\\', "/")
        .nfc()
        .collect::<String>()
        .to_lowercase()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging::{no_log, LogLines};

    fn entry(path: &str) -> ManifestEntry {
        ManifestEntry {
            path: path.into(),
            title: "Dune".into(),
            author: "Frank Herbert".into(),
            format: "azw3".into(),
            size_bytes: 42,
            sent_at: "2026-11-06T10:00:00+00:00".into(),
        }
    }

    const ID: &str = "3f2b1c3e-6e0a-4c1b-9a43-3f1f3c0f9b11";

    #[test]
    fn round_trips_and_matches_the_documented_format() {
        let device = tempfile::tempdir().unwrap();
        assert_eq!(load(device.path()).state, ManifestState::Absent);

        update(device.path(), &no_log, |m| {
            m.device_id = Some("dev-1".into());
            m.books.insert(ID.into(), entry("documents/Dune.azw3"));
        })
        .unwrap();

        let json: Value =
            serde_json::from_slice(&fs::read(manifest_path(device.path())).unwrap()).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "version": 1,
                "deviceId": "dev-1",
                "books": { ID: {
                    "path": "documents/Dune.azw3", "title": "Dune", "author": "Frank Herbert",
                    "format": "azw3", "sizeBytes": 42, "sentAt": "2026-11-06T10:00:00+00:00"
                } }
            })
        );
        let loaded = load(device.path());
        assert_eq!(loaded.state, ManifestState::Valid);
        assert_eq!(
            loaded.manifest.entry(ID),
            Some(&entry("documents/Dune.azw3"))
        );
        let names: Vec<_> = fs::read_dir(device.path().join(MANIFEST_DIR))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(
            names,
            vec![std::ffi::OsString::from(MANIFEST_FILE)],
            "no temp file left"
        );
    }

    #[test]
    fn unknown_fields_are_kept_and_bad_entries_dropped() {
        let device = tempfile::tempdir().unwrap();
        fs::create_dir_all(device.path().join(MANIFEST_DIR)).unwrap();
        fs::write(
            manifest_path(device.path()),
            serde_json::to_vec(&serde_json::json!({
                "version": 1,
                "futureField": { "x": 1 },
                "books": {
                    ID: { "path": "documents/Dune.azw3", "sizeBytes": 1 },
                    "escape": { "path": "../../etc/passwd" },
                    "absolute": { "path": "/Volumes/Other/book.epub" },
                    "../bad-id": { "path": "documents/x.epub" },
                    "wrong-type": { "path": 3 }
                }
            }))
            .unwrap(),
        )
        .unwrap();

        let loaded = load(device.path());
        assert_eq!(loaded.manifest.books.keys().collect::<Vec<_>>(), vec![ID]);
        assert_eq!(loaded.dropped, 4);

        update(device.path(), &no_log, |m| {
            m.books.get_mut(ID).unwrap().title = "Dune".into()
        })
        .unwrap();
        let json: Value =
            serde_json::from_slice(&fs::read(manifest_path(device.path())).unwrap()).unwrap();
        assert_eq!(json["futureField"], serde_json::json!({ "x": 1 }));
        assert_eq!(json["books"].as_object().unwrap().len(), 1);
    }

    #[test]
    fn a_newer_manifest_is_read_but_never_rewritten() {
        let device = tempfile::tempdir().unwrap();
        fs::create_dir_all(device.path().join(MANIFEST_DIR)).unwrap();
        let original = serde_json::to_vec(&serde_json::json!({
            "version": 7, "deviceId": "dev-7", "books": { ID: { "path": "documents/Dune.azw3" } }
        }))
        .unwrap();
        fs::write(manifest_path(device.path()), &original).unwrap();

        let loaded = load(device.path());
        assert_eq!(loaded.state, ManifestState::TooNew(7));
        assert!(loaded.manifest.entry(ID).is_some());
        assert!(update(device.path(), &no_log, |m| m.books.clear())
            .unwrap_err()
            .contains("newer Librarium"));
        assert_eq!(fs::read(manifest_path(device.path())).unwrap(), original);
        assert_eq!(ensure_device_id(device.path(), &no_log).unwrap(), "dev-7");
    }

    #[test]
    fn a_damaged_manifest_is_moved_aside_before_a_new_one_is_written() {
        let device = tempfile::tempdir().unwrap();
        fs::create_dir_all(device.path().join(MANIFEST_DIR)).unwrap();
        fs::write(manifest_path(device.path()), b"{ not json").unwrap();
        assert!(matches!(
            load(device.path()).state,
            ManifestState::Damaged(_)
        ));
        assert!(
            ensure_device_id(device.path(), &no_log).is_err(),
            "detection doesn't rewrite a damaged file"
        );

        let lines = LogLines::default();
        update(device.path(), &lines.sink(), |m| {
            m.books.insert(ID.into(), entry("documents/Dune.azw3"));
        })
        .unwrap();
        assert!(lines.contains("kept aside"));
        assert_eq!(load(device.path()).state, ManifestState::Valid);
        let aside: Vec<_> = fs::read_dir(device.path().join(MANIFEST_DIR))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("device.json.damaged-"))
            .collect();
        assert_eq!(aside.len(), 1);
        assert_eq!(
            fs::read(device.path().join(MANIFEST_DIR).join(&aside[0])).unwrap(),
            b"{ not json"
        );
    }

    #[test]
    fn nothing_is_written_when_nothing_changed() {
        let device = tempfile::tempdir().unwrap();
        update(device.path(), &no_log, |m| {
            m.remove_path("documents/x.epub")
        })
        .unwrap();
        assert!(
            !device.path().join(MANIFEST_DIR).exists(),
            "no manifest created for nothing"
        );
    }

    #[test]
    fn the_device_id_is_created_once() {
        let device = tempfile::tempdir().unwrap();
        let first = ensure_device_id(device.path(), &no_log).unwrap();
        assert!(uuid::Uuid::parse_str(&first).is_ok());
        assert_eq!(ensure_device_id(device.path(), &no_log).unwrap(), first);
        assert!(load(device.path()).manifest.books.is_empty());
    }

    #[test]
    fn paths_are_matched_case_and_normalisation_insensitively() {
        let mut m = DeviceManifest::default();
        m.books.insert(ID.into(), entry("documents/Café.azw3"));
        assert_eq!(
            m.owner_of("documents/cafe\u{301}.AZW3"),
            Some(ID),
            "NFD and upper case"
        );
        assert_eq!(m.owner_of("documents\\CAFÉ.azw3"), Some(ID), "backslashes");
        assert_eq!(m.owner_of("documents/Other.azw3"), None);
        assert_eq!(m.remove_path("DOCUMENTS/café.azw3"), vec![ID.to_string()]);
        assert!(m.books.is_empty());
    }

    #[test]
    fn relative_and_resolved_paths() {
        let mount = Path::new("/Volumes/Kindle");
        assert_eq!(
            relative_path(mount, &mount.join("documents/Sci-Fi/Dune.azw3")).as_deref(),
            Some("documents/Sci-Fi/Dune.azw3")
        );
        assert_eq!(
            relative_path(mount, Path::new("/Volumes/Other/Dune.azw3")),
            None
        );
        assert_eq!(relative_path(mount, mount), None);

        assert_eq!(
            resolve(mount, "documents/Dune.azw3"),
            Some(mount.join("documents").join("Dune.azw3"))
        );
        assert_eq!(
            resolve(mount, "documents\\Dune.azw3"),
            Some(mount.join("documents").join("Dune.azw3"))
        );
        for bad in [
            "",
            "  ",
            "/etc/passwd",
            "\\x",
            "C:\\x",
            "C:x",
            "../x",
            "documents/../../x",
            "documents//x",
            "./x",
            "documents/...",
        ] {
            assert_eq!(resolve(mount, bad), None, "{bad:?}");
        }
    }
}
