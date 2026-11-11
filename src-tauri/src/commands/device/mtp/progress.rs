//! Reading progress of an MTP Kindle, from the `.sdr` folders next to its
//! books: the strategy that works on USB-drive Kindles (`.mbs` furthest
//! position + `.apnx` page table, or the bookmark databases;
//! `commands/progress.rs`), run on local copies of those files.
//!
//! **Unverified**: nobody knows yet whether a 2024 Kindle exposes `.sdr`
//! folders over MTP at all. TODO(device-validation): check, and drop this
//! (or find where the progress lives) if it doesn't. Without `.sdr`
//! folders this finds nothing, after one (cached) walk of `documents/`.
//!
//! **Cost.** Each refresh walks `documents/` (three levels, like on a
//! drive) and lists each `.sdr` folder, but only downloads files that
//! changed: the copies are kept per device in a private temp folder, keyed
//! by object id, size and modification date. (If a device doesn't report
//! modification dates, a file rewritten at the same size isn't noticed
//! until the app restarts.) Files over [`MAX_FILE`] are skipped.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::commands::progress::{sdr_key, sdr_progress};
use crate::fsutil;
use crate::mtp::devfs::Folder;
use crate::mtp::session::{MtpError, Object};
use crate::mtp::worker::Connection;
use crate::mtp::{self, MtpState};

/// Largest `.sdr` file copied (a bookmark database is a few hundred KB).
const MAX_FILE: u64 = 16 << 20;

/// How deep `.sdr` folders are looked for under `documents/` (as on a drive).
const SDR_DEPTH: usize = 3;

/// The extensions of the files progress is read from.
const WANTED: &[&str] = &["mbs", "apnx", "db"];

/// The local copies of one device's `.sdr` files.
struct Mirror {
    dir: tempfile::TempDir,
    /// Device path → (object id, size, modification date) of the copy.
    files: HashMap<String, (String, u64, String)>,
}

/// Mirrors by device id, kept for the app's lifetime.
static MIRRORS: Mutex<Option<HashMap<String, Mirror>>> = Mutex::new(None);

/// `book key → percent` for the MTP Kindle at `device_path` (keys as in
/// `get_device_reading_progress`). Empty, logged, on any failure.
pub(crate) fn reading_progress(state: &MtpState, device_path: &str) -> HashMap<String, u8> {
    let log = state.logger();
    let Some((key, _)) = mtp::split_path(device_path) else {
        return HashMap::new();
    };
    let (Ok(worker), Some(ready)) = (state.worker(key), state.ready(key)) else {
        log(format!("[progress/mtp] {key}: not connected"));
        return HashMap::new();
    };
    let taken = MIRRORS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(HashMap::new)
        .remove(&ready.device_id);
    let mirror = match taken {
        Some(m) => m,
        None => match tempfile::Builder::new().prefix(".librarium-sdr-").tempdir() {
            Ok(dir) => Mirror {
                dir,
                files: HashMap::new(),
            },
            Err(e) => {
                log(format!(
                    "[progress/mtp] WARNING — no temp folder for the copies: {e}"
                ));
                return HashMap::new();
            }
        },
    };
    let result = worker.call(move |c| {
        let mut mirror = mirror;
        let outcome = refresh(c, &mut mirror);
        Ok((mirror, outcome))
    });
    match result {
        Ok((mirror, outcome)) => {
            MIRRORS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get_or_insert_with(HashMap::new)
                .insert(ready.device_id, mirror);
            match outcome {
                Ok((map, downloaded, lines)) => {
                    for line in lines {
                        log(format!("[progress/mtp] {line}"));
                    }
                    log(format!(
                        "[progress/mtp] {key}: {} entries ({downloaded} file(s) copied)",
                        map.len()
                    ));
                    map
                }
                Err(e) => {
                    log(format!("[progress/mtp] {key}: WARNING — {e}"));
                    HashMap::new()
                }
            }
        }
        Err(e) => {
            log(format!("[progress/mtp] {key}: WARNING — {e}"));
            HashMap::new()
        }
    }
}

/// Bring the mirror up to date and read the progress from it: the map,
/// how many files were copied, the lines `sdr_progress` logged.
#[allow(clippy::type_complexity)]
fn refresh(
    c: &mut Connection,
    mirror: &mut Mirror,
) -> Result<(HashMap<String, u8>, usize, Vec<String>), MtpError> {
    let s = &mut *c.session;
    let fs = &mut c.fs;
    let layout = fs.layout(s)?.clone();
    let mut sdrs: Vec<(String, Object)> = Vec::new();
    for volume in &layout.volumes {
        let Some(documents) = &volume.documents else {
            continue;
        };
        let start = format!("{}{}", volume.prefix, documents.name);
        let descend = |o: &Object| !o.name.starts_with('.') && !is_sdr(&o.name);
        let found = fs.walk(s, &Folder::of(documents), &start, SDR_DEPTH, &descend)?;
        sdrs.extend(
            found
                .into_iter()
                .filter(|(_, o)| o.is_folder && is_sdr(&o.name)),
        );
    }

    let mut seen = HashSet::new();
    let mut downloaded = 0;
    for (rel, sdr) in &sdrs {
        // The files in the folder, and the databases one level down: listed
        // freshly (a few files each), since the Kindle rewrites them as you read.
        let mut files = Vec::new();
        fs.forget(&Folder::of(sdr));
        for object in fs.list(s, &Folder::of(sdr))? {
            let path = format!("{rel}/{}", object.name);
            if object.is_folder {
                fs.forget(&Folder::of(&object));
                for inner in fs.list(s, &Folder::of(&object))? {
                    if !inner.is_folder && has_extension(&inner.name, "db") {
                        files.push((format!("{path}/{}", inner.name), inner));
                    }
                }
            } else if WANTED.iter().any(|ext| has_extension(&object.name, ext)) {
                files.push((path, object));
            }
        }
        for (path, object) in files.into_iter().filter(|(_, o)| o.size <= MAX_FILE) {
            seen.insert(path.clone());
            let tag = (object.id.clone(), object.size, object.modified.clone());
            let local = local_path(mirror.dir.path(), &path);
            if mirror.files.get(&path) == Some(&tag) && local.is_file() {
                continue;
            }
            copy(s, &object, &local)?;
            mirror.files.insert(path, tag);
            downloaded += 1;
        }
    }
    // Files gone from the device: their copies too.
    let gone: Vec<String> = mirror
        .files
        .keys()
        .filter(|p| !seen.contains(*p))
        .cloned()
        .collect();
    for path in gone {
        let _ = fs::remove_file(local_path(mirror.dir.path(), &path));
        mirror.files.remove(&path);
    }

    let lines = std::cell::RefCell::new(Vec::new());
    let log = |line: String| lines.borrow_mut().push(line);
    let mut map = HashMap::new();
    for (rel, sdr) in &sdrs {
        let Some(key) = sdr_key(&sdr.name) else {
            continue;
        };
        if let Some(pct) = sdr_progress(&local_path(mirror.dir.path(), rel), &log) {
            map.insert(key, pct);
        }
    }
    Ok((map, downloaded, lines.into_inner()))
}

fn is_sdr(name: &str) -> bool {
    has_extension(name, "sdr")
}

fn has_extension(name: &str, ext: &str) -> bool {
    Path::new(name)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

/// Where the copy of device file `rel` goes: each component made safe for
/// the local file system (an `.sdr` name stays an `.sdr` name).
fn local_path(root: &Path, rel: &str) -> PathBuf {
    let mut path = root.to_path_buf();
    for part in rel.split('/').filter(|p| !p.is_empty()) {
        path.push(fsutil::sanitize_filename(part));
    }
    path
}

/// Download `object` to `local`, through a temp file (an interrupted copy
/// never looks complete).
fn copy(
    s: &mut dyn crate::mtp::session::MtpSession,
    object: &Object,
    local: &Path,
) -> Result<(), MtpError> {
    let io = |e: std::io::Error| {
        MtpError::Local(format!("Could not save a copy of {}: {e}", object.name))
    };
    let dir = local.parent().expect("a mirror path has a parent");
    fs::create_dir_all(dir).map_err(io)?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir).map_err(io)?;
    s.read_file(object, tmp.as_file_mut(), &mut |_| true)?;
    tmp.persist(local).map_err(|e| io(e.error))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::device::mtp::tests::{connected, KEY};
    use crate::mtp::fake::FakeDevice;

    /// A `.mbs` file whose furthest position is `fpr`.
    fn mbs(fpr: u32) -> Vec<u8> {
        let digits = fpr.to_string();
        let mut out = b"header".to_vec();
        out.extend_from_slice(b"fpr");
        out.push(0x03);
        out.extend_from_slice(&[0, 0, digits.len() as u8]);
        out.extend_from_slice(digits.as_bytes());
        out
    }

    /// An `.apnx` page table with pages at 0, 100, 200… (`pages` of them).
    fn apnx(pages: u16) -> Vec<u8> {
        let main = b"{}";
        let mut out = vec![0, 1, 0, 1, 0, 0, 0, 0];
        out.extend_from_slice(&(main.len() as u32).to_be_bytes());
        out.extend_from_slice(main);
        let section = b"{}";
        out.extend_from_slice(&[0, 1]);
        out.extend_from_slice(&(section.len() as u16).to_be_bytes());
        out.extend_from_slice(&pages.to_be_bytes());
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(section);
        for p in 0..u32::from(pages) {
            out.extend_from_slice(&(p * 100).to_be_bytes());
        }
        out
    }

    #[test]
    fn progress_comes_from_mirrored_sdr_folders_and_only_changes_are_copied() {
        let device = FakeDevice::kindle();
        device.put("documents/Dune.azw3", b"book");
        device.put("documents/Dune.sdr/Dune.mbs", &mbs(450));
        device.put("documents/Dune.sdr/Dune.apnx", &apnx(10));
        device.put("documents/Dune.sdr/notes.txt", b"ignored");
        let (state, lines) = connected(&device);
        let path = crate::mtp::mount_path(KEY);

        let first = reading_progress(&state, &path);
        assert_eq!(first, HashMap::from([("Dune".to_string(), 50)]));
        assert!(lines
            .lock()
            .unwrap()
            .iter()
            .any(|l| l.contains("(2 file(s) copied)")));

        let again = reading_progress(&state, &path);
        assert_eq!(again, first);
        assert!(
            lines
                .lock()
                .unwrap()
                .iter()
                .any(|l| l.contains("(0 file(s) copied)")),
            "cached"
        );

        // Read further: the .mbs is rewritten (a new version).
        let id = device.find("documents/Dune.sdr/Dune.mbs").unwrap();
        {
            let mut st = device.state();
            let entry = st.entries.get_mut(&id).unwrap();
            entry.data = mbs(950);
            entry.version += 1;
        }
        assert_eq!(reading_progress(&state, &path)["Dune"], 100);
        assert!(lines
            .lock()
            .unwrap()
            .iter()
            .any(|l| l.contains("(1 file(s) copied)")));
    }

    #[test]
    fn a_kindle_without_sdr_folders_has_no_progress() {
        let device = FakeDevice::kindle();
        device.put("documents/Dune.azw3", b"book");
        let (state, _) = connected(&device);
        assert!(reading_progress(&state, &crate::mtp::mount_path(KEY)).is_empty());
        assert!(
            reading_progress(&state, "mtp://usb-9-9").is_empty(),
            "not connected"
        );
    }

    #[test]
    fn local_copies_have_safe_names() {
        let root = Path::new("/tmp/mirror");
        assert_eq!(
            local_path(root, "documents/A: B.sdr/x.mbs"),
            root.join("documents").join("A_ B.sdr").join("x.mbs")
        );
    }
}
