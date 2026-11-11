//! MTP Kindles in the device commands: detection, listing, deleting,
//! importing, ejecting (here), sending ([`send`]), the manifest
//! ([`manifest`]) and reading progress ([`progress`]). The transport and
//! the per-device worker are in `crate::mtp`.
//!
//! Everything here runs as jobs on the device's worker thread
//! ([`WorkerHandle::call`]), so operations on one device never overlap,
//! and the caller (a Tauri command on the blocking pool) just waits.
//!
//! Paths from the webview (`mtp://<port>/documents/…`) are checked like
//! paths on a drive ([`check_book_path`]): a plain relative path, inside
//! `documents/`, to an ebook file, on a connected MTP Kindle. They are
//! resolved freshly on the device before anything is deleted or copied.
//!
//! What differs from a drive, and why:
//! - **Titles** come from the device manifest for the books Librarium sent,
//!   else from the file names: reading each book's metadata would mean
//!   downloading every book (or a request per header) at each listing.
//! - **Library ids** come from the device manifest only, for the same reason.
//! - **Leftover temp files** (`.librarium-*.part`, from an interrupted
//!   replace) are removed when the books are listed: nothing else can be
//!   writing to the device at that moment.

pub(crate) mod manifest;
pub(crate) mod progress;
pub(crate) mod send;

use std::collections::HashMap;
use std::fs::File;
use std::path::Path;
use std::sync::Arc;

use super::listing::{is_ebook, parse_filename_title_author, DeviceBook};
use super::manifest::{is_plain_relative, path_key, ManifestEntry, ManifestState};
use super::transfer::is_partial_name;
use super::DeviceSpace;
use crate::error::{AppError, AppResult};
use crate::fsutil;
use crate::models::device::{KindleDevice, CONNECTION_MTP};
use crate::mtp::devfs::{
    join, name_of, parent_of, same_name, DeviceFs, Folder, DOCUMENTS, MAX_DEPTH,
};
use crate::mtp::session::{Candidate, MtpError, MtpSession, Object};
use crate::mtp::worker::{ConnectHook, Connection, Ready, WorkerHandle};
use crate::mtp::{self, MtpState};

// ── Connecting ────────────────────────────────────────────────────────────────

/// What a worker runs on each new connection: the device id from the
/// manifest (created on first sight, like on a drive), the model, the
/// space of the storage books go to.
///
/// A device whose manifest can't be written (a Kindle that refuses the
/// `.librarium` folder?) gets the id `kindle:mtp://<port>`: stable while it
/// stays in that port.
pub(crate) fn connect_hook() -> ConnectHook {
    Arc::new(|candidate: &Candidate, c: &mut Connection| {
        let primary = c.fs.layout(&mut *c.session)?.primary().storage.clone();
        let device_id = match manifest::ensure_device_id(&mut c.fs, &mut *c.session) {
            Ok(id) => id,
            Err(e) if e.is_fatal() => return Err(e),
            Err(_) => format!("kindle:{}", mtp::mount_path(&candidate.key)),
        };
        let model = model_name(&c.session.description().model);
        Ok(Ready {
            device_id,
            name: model.clone().unwrap_or_else(|| candidate.label.clone()),
            model,
            storage: primary.id,
            free_bytes: primary.free_bytes,
            total_bytes: primary.capacity_bytes,
        })
    })
}

/// The model to show for an MTP model string: "Kindle Paperwhite" as is,
/// "Paperwhite" as "Kindle Paperwhite", nothing for an empty one.
/// TODO(device-validation): see what a 2024 Kindle reports as its MTP
/// model (and whether it tells a Paperwhite from a Colorsoft).
fn model_name(model: &str) -> Option<String> {
    let model = model.trim();
    match model {
        "" => None,
        m if m.to_lowercase().contains("kindle") => Some(m.to_string()),
        m => Some(format!("Kindle {m}")),
    }
}

/// The [`KindleDevice`] of a connected MTP Kindle.
fn kindle_device(key: &str, ready: &Ready) -> KindleDevice {
    KindleDevice {
        id: ready.device_id.clone(),
        name: ready.name.clone(),
        mount_path: mtp::mount_path(key),
        device_type: "kindle".into(),
        connection: CONNECTION_MTP.into(),
        // TODO(device-validation): check which formats a 2024 Kindle opens
        // when sent over MTP (AZW3 is assumed, as for USB-drive Kindles).
        preferred_format: "azw3".into(),
        model: ready.model.clone(),
        free_space_bytes: ready.free_bytes,
        total_space_bytes: ready.total_bytes,
    }
}

/// The connected MTP Kindles, for `detect_devices` (see [`MtpState::poll`]).
pub(crate) fn detect(state: &MtpState) -> Vec<KindleDevice> {
    state
        .poll()
        .into_iter()
        .filter_map(|(device, ready)| {
            let (key, _) = mtp::split_path(&device.mount_path)?;
            Some(kindle_device(key, &ready?))
        })
        .collect()
}

// ── Paths and errors ──────────────────────────────────────────────────────────

/// A failure, for the user: `context` plus what went wrong.
fn device_error(context: &str, e: &MtpError) -> AppError {
    AppError::Device(format!("{context}. {}", e.user_message()))
}

/// The worker of the MTP Kindle `key`, or a `DEVICE` error saying why not.
fn worker_for(state: &MtpState, key: &str) -> AppResult<WorkerHandle> {
    state.worker(key).map_err(|e| match e {
        MtpError::Refused(message) => AppError::Device(message),
        _ => AppError::Device("The Kindle (MTP) isn't connected. Plug it in, unlock it and wait until Librarium shows it.".into()),
    })
}

/// The device key of an `mtp://` mount path.
fn device_key(device_path: &str) -> AppResult<&str> {
    mtp::split_path(device_path)
        .map(|(key, _)| key)
        .ok_or_else(|| AppError::InvalidInput(format!("Not an MTP device: {device_path}")))
}

/// Whether a path component is hidden.
fn is_hidden(name: &str) -> bool {
    name.starts_with('.')
}

/// Check a book path from the webview: `mtp://<key>/<rel>` with `rel` a
/// plain relative path into `documents/` (of any storage), to an ebook
/// file, through no hidden folder. Returns `(key, rel)`.
pub(crate) fn check_book_path(path: &str) -> AppResult<(&str, &str)> {
    let invalid =
        || AppError::InvalidInput(format!("Not a book on an MTP Kindle: {}", name_of(path)));
    let (key, rel) = mtp::split_path(path).ok_or_else(invalid)?;
    if !in_documents(rel) || !is_ebook(Path::new(rel)) || rel.split('/').any(is_hidden) {
        return Err(invalid());
    }
    Ok((key, rel))
}

/// Whether the device path `rel` is a plain relative path to `documents/`
/// (of any storage) or something inside it.
pub(crate) fn in_documents(rel: &str) -> bool {
    if !is_plain_relative(rel) {
        return false;
    }
    let parts: Vec<&str> = rel.split('/').collect();
    let start = usize::from(parts.len() > 1 && parts[0].starts_with("storage-"));
    parts.get(start).is_some_and(|p| same_name(p, DOCUMENTS))
}

// ── Listing ───────────────────────────────────────────────────────────────────

/// What [`book_files`] finds.
struct BookFiles {
    /// Device path and object of each book.
    books: Vec<(String, Object)>,
    /// Leftover `.librarium-*.part` files.
    partials: Vec<Object>,
}

/// The book files under every storage's `documents/` (not in hidden or
/// `.sdr` folders), and the leftover `.librarium-*.part` files found there.
fn book_files(fs: &mut DeviceFs, s: &mut dyn MtpSession) -> Result<BookFiles, MtpError> {
    let layout = fs.layout(s)?.clone();
    let mut books = Vec::new();
    let mut partials = Vec::new();
    for volume in &layout.volumes {
        let Some(documents) = &volume.documents else {
            continue;
        };
        let start = format!("{}{}", volume.prefix, documents.name);
        let descend = |o: &Object| !is_hidden(&o.name) && !o.name.to_lowercase().ends_with(".sdr");
        for (rel, object) in fs.walk(s, &Folder::of(documents), &start, MAX_DEPTH, &descend)? {
            if object.is_folder {
                continue;
            }
            if is_partial_name(&object.name) {
                partials.push(object);
            } else if !is_hidden(&object.name) && is_ebook(Path::new(&object.name)) {
                books.push((rel, object));
            }
        }
    }
    Ok(BookFiles { books, partials })
}

/// The books of the manifest (library id, and the entry: title, author…),
/// by [`path_key`]; with a warning when it can't be used.
fn owners(
    fs: &mut DeviceFs,
    s: &mut dyn MtpSession,
    notes: &mut Vec<String>,
) -> Result<HashMap<String, (String, ManifestEntry)>, MtpError> {
    match manifest::load(fs, s) {
        Ok(stored) => {
            if let ManifestState::Damaged(reason) = &stored.loaded.state {
                notes.push(format!(
                    "WARNING — the device manifest {reason}; books are matched by name only"
                ));
            }
            Ok(stored
                .manifest()
                .books
                .iter()
                .map(|(id, e)| (path_key(&e.path), (id.clone(), e.clone())))
                .collect())
        }
        Err(e) if e.is_fatal() => Err(e),
        Err(e) => {
            notes.push(format!(
                "WARNING — the device manifest can't be read ({e}); books are matched by name only"
            ));
            Ok(HashMap::new())
        }
    }
}

/// The books on the MTP Kindle at `device_path` (fresh listings).
pub(crate) fn list_books(state: &MtpState, device_path: &str) -> AppResult<Vec<DeviceBook>> {
    let key = device_key(device_path)?.to_string();
    let worker = worker_for(state, &key)?;
    let log = state.logger();
    let job_key = key.clone();
    let (books, notes) = worker
        .call(move |c| {
            c.fs.invalidate();
            books_on(c, &job_key)
        })
        .map_err(|e| device_error("Could not list the books on the Kindle", &e))?;
    for note in notes {
        log(format!("[list_device_books] {key}: {note}"));
    }
    Ok(books)
}

fn books_on(c: &mut Connection, key: &str) -> Result<(Vec<DeviceBook>, Vec<String>), MtpError> {
    let s = &mut *c.session;
    let fs = &mut c.fs;
    let mut notes = Vec::new();
    let owners = owners(fs, s, &mut notes)?;
    let BookFiles {
        books: files,
        partials,
    } = book_files(fs, s)?;
    for partial in partials {
        match s.delete(&partial.id) {
            Ok(()) => {
                fs.note_removed(&partial);
                notes.push(format!(
                    "removed {}, left by an interrupted transfer",
                    partial.name
                ));
            }
            Err(e) => notes.push(format!(
                "WARNING — could not remove the leftover {}: {e}",
                partial.name
            )),
        }
    }
    let books = files
        .into_iter()
        .map(|(rel, object)| {
            let stem = Path::new(&object.name)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            // Files aren't opened over MTP (a request each): what Librarium
            // recorded when it sent the book, else what the name says.
            let sent = owners.get(&path_key(&rel));
            let (title, author) = match sent {
                Some((_, entry)) if !entry.title.trim().is_empty() => {
                    (entry.title.clone(), entry.author.clone())
                }
                _ => parse_filename_title_author(&stem),
            };
            DeviceBook {
                path: mtp::file_path(key, &rel),
                library_id: sent.map(|(id, _)| id.clone()),
                filename: object.name,
                title,
                author,
                size_bytes: object.size,
                percent_read: None,
                read_status: None,
            }
        })
        .collect();
    Ok((books, notes))
}

// ── Deleting ──────────────────────────────────────────────────────────────────

/// Delete one book from an MTP Kindle: the file, its `.sdr` folders (as on
/// a drive: `<name>.sdr`, and `<stem>.sdr` unless another book has that
/// stem), its manifest entry.
pub(crate) fn delete_book(state: &MtpState, path: &str) -> AppResult<()> {
    let (key, rel) = check_book_path(path)?;
    let worker = worker_for(state, key)?;
    let log = state.logger();
    let job_rel = rel.to_string();
    let notes = worker.call(move |c| delete_on(c, &job_rel)).map_err(|e| {
        device_error(
            &format!("Could not delete {} from the Kindle", name_of(rel)),
            &e,
        )
    })?;
    log(format!("[delete_device_book] deleted {path}"));
    for note in notes {
        log(format!("[delete_device_book] {note}"));
    }
    Ok(())
}

fn delete_on(c: &mut Connection, rel: &str) -> Result<Vec<String>, MtpError> {
    let s = &mut *c.session;
    let fs = &mut c.fs;
    let name = name_of(rel).to_string();
    let book = fs
        .resolve_checked(s, rel)?
        .filter(|o| !o.is_folder)
        .ok_or_else(|| MtpError::NotFound(name.clone()))?;
    s.delete(&book.id)?;
    fs.note_removed(&book);
    let mut notes = Vec::new();

    // The reading state, next to the book (only inside documents/: checked by the caller).
    let folder = Folder {
        storage: book.storage.clone(),
        id: book.parent.clone(),
    };
    let siblings = fs.list(s, &folder)?;
    let stem = Path::new(&book.name)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem_in_use = siblings.iter().any(|o| {
        !o.is_folder
            && is_ebook(Path::new(&o.name))
            && Path::new(&o.name)
                .file_stem()
                .is_some_and(|x| same_name(&x.to_string_lossy(), &stem))
    });
    let mut sdr_names = vec![format!("{}.sdr", book.name)];
    if !stem_in_use {
        sdr_names.push(format!("{stem}.sdr"));
    }
    for sdr in siblings
        .iter()
        .filter(|o| o.is_folder && sdr_names.iter().any(|n| same_name(n, &o.name)))
    {
        match fs.delete_tree(s, sdr) {
            Ok(()) => notes.push(format!("removed {}", join(parent_of(rel), &sdr.name))),
            Err(e) if e.is_fatal() => return Ok(notes),
            Err(e) => notes.push(format!("WARNING — could not remove {}: {e}", sdr.name)),
        }
    }

    match manifest::update(fs, s, |m| m.remove_path(rel)) {
        Ok(ids) if !ids.is_empty() => notes.push(format!("forgot {}", ids.join(", "))),
        Ok(_) => {}
        Err(e) => notes.push(format!("WARNING — {}", e.user_message())),
    }
    c.space = fs.space(s, &book.storage).ok();
    Ok(notes)
}

// ── Importing ─────────────────────────────────────────────────────────────────

/// The local files to import for `paths`: `mtp://` books are copied (fresh
/// path lookup, streamed) into private temp folders, which are removed
/// when the returned guards are dropped; other paths are kept as they are.
/// A copy that fails fails the whole import (nothing is imported), with
/// the reason.
pub(crate) fn fetch_for_import(
    state: &MtpState,
    paths: &[String],
) -> AppResult<(Vec<String>, Vec<tempfile::TempDir>)> {
    let mut local = Vec::with_capacity(paths.len());
    let mut guards = Vec::new();
    for path in paths {
        if !mtp::is_mtp_path(path) {
            local.push(path.clone());
            continue;
        }
        let (key, rel) = check_book_path(path)?;
        let worker = worker_for(state, key)?;
        let job_rel = rel.to_string();
        let (dir, file) = worker.call(move |c| fetch(c, &job_rel)).map_err(|e| {
            device_error(
                &format!("Could not copy {} from the Kindle", name_of(rel)),
                &e,
            )
        })?;
        local.push(file);
        guards.push(dir);
    }
    Ok((local, guards))
}

fn fetch(c: &mut Connection, rel: &str) -> Result<(tempfile::TempDir, String), MtpError> {
    let s = &mut *c.session;
    let book =
        c.fs.resolve_checked(s, rel)?
            .filter(|o| !o.is_folder)
            .ok_or_else(|| MtpError::NotFound(name_of(rel).to_string()))?;
    let local = |e: std::io::Error| MtpError::Local(format!("Could not save the copy: {e}"));
    let dir = tempfile::Builder::new()
        .prefix(".librarium-mtp-")
        .tempdir()
        .map_err(local)?;
    let path = dir.path().join(fsutil::sanitize_filename(&book.name));
    let mut file = File::create(&path).map_err(local)?;
    s.read_file(&book, &mut file, &mut |_| true)?;
    fsutil::sync_file(&file).map_err(local)?;
    Ok((dir, path.to_string_lossy().into_owned()))
}

// ── Ejecting, space ───────────────────────────────────────────────────────────

/// Release the MTP Kindle at `mount_path` (close the session): it can then
/// be unplugged, and stays released until it is.
pub(crate) fn eject(state: &MtpState, mount_path: &str) -> AppResult<()> {
    let key = device_key(mount_path)?;
    state.eject(key).map_err(|e| match e {
        MtpError::WorkerGone => AppError::Device("The Kindle (MTP) isn't connected.".into()),
        other => device_error("Could not eject the Kindle", &other),
    })
}

/// The `device-space` payload of the MTP Kindle at `mount_path`.
pub(crate) fn space(state: &MtpState, mount_path: &str) -> DeviceSpace {
    let ready = mtp::split_path(mount_path).and_then(|(key, _)| state.ready(key));
    DeviceSpace {
        mount_path: mount_path.to_string(),
        free_space_bytes: ready.as_ref().and_then(|r| r.free_bytes),
        total_space_bytes: ready.as_ref().and_then(|r| r.total_bytes),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::commands::device::manifest::ManifestEntry;
    use crate::mtp::fake::{FakeBackend, FakeDevice};
    use crate::mtp::worker::tests::collect_logs;
    use std::sync::Mutex;
    use std::time::Duration;

    pub(crate) const KEY: &str = "usb-1-2";
    pub(crate) const ID: &str = "3f2b1c3e-6e0a-4c1b-9a43-3f1f3c0f9b11";

    /// An enabled MTP state with `device` plugged in as [`KEY`], polled once
    /// (connected), and its log lines.
    pub(crate) fn connected(device: &FakeDevice) -> (MtpState, Arc<Mutex<Vec<String>>>) {
        let (log, lines) = collect_logs();
        let state = MtpState::with_timing(
            FakeBackend::with(&[(KEY, device)]),
            connect_hook(),
            log,
            Duration::from_secs(5),
            Duration::ZERO,
        );
        state.set_enabled(true);
        state.poll();
        (state, lines)
    }

    /// Record `rel` as library book `id` in the device manifest.
    pub(crate) fn record(state: &MtpState, id: &str, rel: &str) {
        record_titled(state, id, rel, "", "");
    }

    /// Record `rel` as library book `id`, sent with this title and author.
    fn record_titled(state: &MtpState, id: &str, rel: &str, title: &str, author: &str) {
        let rel = rel.to_string();
        let id = id.to_string();
        let (title, author) = (title.to_string(), author.to_string());
        state
            .worker(KEY)
            .unwrap()
            .call(move |c| {
                manifest::update(&mut c.fs, &mut *c.session, |m| {
                    m.books.insert(
                        id,
                        ManifestEntry {
                            path: rel,
                            title,
                            author,
                            format: "azw3".into(),
                            size_bytes: 1,
                            sent_at: String::new(),
                        },
                    )
                })
            })
            .unwrap();
    }

    #[test]
    fn a_connected_kindle_is_detected_with_its_manifest_id() {
        let device = FakeDevice::kindle();
        let (state, lines) = connected(&device);
        let devices = detect(&state);
        assert_eq!(devices.len(), 1);
        let d = &devices[0];
        assert_eq!(
            (
                d.mount_path.as_str(),
                d.connection.as_str(),
                d.device_type.as_str()
            ),
            ("mtp://usb-1-2", "mtp", "kindle")
        );
        assert_eq!(
            (d.name.as_str(), d.model.as_deref()),
            ("Kindle Paperwhite", Some("Kindle Paperwhite"))
        );
        assert!(uuid::Uuid::parse_str(&d.id).is_ok(), "{}", d.id);
        assert!(d.free_space_bytes.is_some() && d.total_space_bytes.is_some());
        assert!(
            device.find(".librarium/device-1.json").is_some(),
            "the id is kept on the device"
        );
        // Stable across reconnections.
        state.set_enabled(false);
        state.set_enabled(true);
        assert_eq!(detect(&state)[0].id, d.id);
        assert!(
            !lines.lock().unwrap().iter().any(|l| l.contains("SERIAL")),
            "no native id in the log"
        );
    }

    #[test]
    fn a_kindle_that_refuses_the_manifest_gets_a_port_id() {
        let device = FakeDevice::kindle();
        device.put(".librarium", b"a file where the folder should be");
        let (state, _) = connected(&device);
        assert_eq!(detect(&state)[0].id, "kindle:mtp://usb-1-2");
    }

    #[test]
    fn nothing_is_detected_while_disabled() {
        let device = FakeDevice::kindle();
        let (log, _) = collect_logs();
        let state = MtpState::with_timing(
            FakeBackend::with(&[(KEY, &device)]),
            connect_hook(),
            log,
            Duration::from_secs(5),
            Duration::ZERO,
        );
        assert!(detect(&state).is_empty());
        assert_eq!(device.state().opens, 0);
        let err = list_books(&state, "mtp://usb-1-2").unwrap_err();
        assert!(err.to_string().contains("Settings"), "{err}");
    }

    #[test]
    fn books_are_listed_from_every_documents_folder_with_their_ids() {
        let device = FakeDevice::kindle();
        device.put("documents/Dune - Frank Herbert.azw3", b"dune");
        device.put("documents/Shelf/Emma.epub", b"emma");
        device.put("documents/notes.txt", b"x");
        device.put("documents/._Dune - Frank Herbert.azw3", b"appledouble");
        device.put("documents/Dune - Frank Herbert.sdr/Dune.pdf", b"not a book");
        device.put("documents/.hidden/x.epub", b"x");
        device.put("documents/.librarium-abc123.part", b"leftover");
        device.put("system/x.epub", b"not under documents");
        let (state, lines) = connected(&device);
        record(&state, ID, "documents/shelf/EMMA.epub");

        let books = list_books(&state, "mtp://usb-1-2").unwrap();
        let summary: Vec<_> = books
            .iter()
            .map(|b| {
                (
                    b.path.as_str(),
                    b.title.as_str(),
                    b.author.as_str(),
                    b.library_id.as_deref(),
                    b.size_bytes,
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                (
                    "mtp://usb-1-2/documents/Dune - Frank Herbert.azw3",
                    "Dune",
                    "Frank Herbert",
                    None,
                    4
                ),
                (
                    "mtp://usb-1-2/documents/Shelf/Emma.epub",
                    "Emma",
                    "",
                    Some(ID),
                    4
                ),
            ]
        );
        assert!(
            device.find("documents/.librarium-abc123.part").is_none(),
            "the leftover is removed"
        );
        assert!(lines
            .lock()
            .unwrap()
            .iter()
            .any(|l| l.contains("interrupted transfer")));
    }

    #[test]
    fn books_librarium_sent_are_listed_with_the_title_it_recorded() {
        let device = FakeDevice::kindle();
        device.put("documents/Dune_Frank_Herbert.azw3", b"dune");
        device.put("documents/Le_calamity_club_-_Kathryn_Stockett.pdf", b"pdf");
        let (state, _lines) = connected(&device);
        record_titled(
            &state,
            ID,
            "documents/Dune_Frank_Herbert.azw3",
            "Dune",
            "Frank Herbert",
        );

        let books = list_books(&state, "mtp://usb-1-2").unwrap();
        let summary: Vec<_> = books
            .iter()
            .map(|b| (b.filename.as_str(), b.title.as_str(), b.author.as_str()))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("Dune_Frank_Herbert.azw3", "Dune", "Frank Herbert"),
                (
                    "Le_calamity_club_-_Kathryn_Stockett.pdf",
                    "Le calamity club",
                    "Kathryn Stockett"
                ),
            ]
        );
    }

    #[test]
    fn book_paths_from_the_webview_are_confined() {
        assert_eq!(
            check_book_path("mtp://usb-1-2/documents/Dune.azw3").unwrap(),
            ("usb-1-2", "documents/Dune.azw3")
        );
        assert!(check_book_path("mtp://usb-1-2/Documents/SF/Dune.EPUB").is_ok());
        assert!(check_book_path("mtp://usb-1-2/storage-131073/documents/Dune.azw3").is_ok());
        for bad in [
            "mtp://usb-1-2/documents/../system/x.azw3",
            "mtp://usb-1-2/system/x.azw3",
            "mtp://usb-1-2/documents/notes.txt",
            "mtp://usb-1-2/documents",
            "mtp://usb-1-2/documents/.hidden/x.azw3",
            "mtp://usb-1-2/documents/._x.azw3",
            "mtp://usb-1-2//documents/x.azw3",
            "/Volumes/Kindle/documents/x.azw3",
        ] {
            assert_eq!(
                check_book_path(bad).map_err(|e| e.code()).err(),
                Some("INVALID_INPUT"),
                "{bad}"
            );
        }
    }

    #[test]
    fn deleting_removes_the_book_its_sdr_and_its_manifest_entry() {
        let device = FakeDevice::kindle();
        device.put("documents/Dune.azw3", b"x");
        device.put("documents/Dune.sdr/Dune.mbs", b"position");
        device.put("documents/Other.sdr/Other.mbs", b"keep");
        let (state, _) = connected(&device);
        record(&state, ID, "documents/Dune.azw3");

        delete_book(&state, "mtp://usb-1-2/documents/dune.AZW3").unwrap();
        let left: Vec<String> = device
            .paths()
            .into_iter()
            .filter(|p| p.starts_with("documents"))
            .collect();
        assert_eq!(
            left,
            vec![
                "documents",
                "documents/Other.sdr",
                "documents/Other.sdr/Other.mbs"
            ]
        );
        let manifest = state
            .worker(KEY)
            .unwrap()
            .call(|c| manifest::load(&mut c.fs, &mut *c.session))
            .unwrap();
        assert!(manifest.manifest().books.is_empty());
    }

    #[test]
    fn a_shared_stem_keeps_its_sdr() {
        let device = FakeDevice::kindle();
        device.put("documents/Dune.azw3", b"x");
        device.put("documents/Dune.pdf", b"%PDF");
        device.put("documents/Dune.sdr/Dune.mbs", b"maybe the azw3's");
        let (state, _) = connected(&device);
        delete_book(&state, "mtp://usb-1-2/documents/Dune.pdf").unwrap();
        assert!(device.find("documents/Dune.sdr").is_some());
        assert!(device.find("documents/Dune.pdf").is_none());
    }

    #[test]
    fn deleting_a_book_that_is_gone_says_so() {
        let device = FakeDevice::kindle();
        let (state, _) = connected(&device);
        let err = delete_book(&state, "mtp://usb-1-2/documents/Gone.azw3").unwrap_err();
        assert_eq!(err.code(), "DEVICE");
        assert!(err.to_string().contains("Gone.azw3"), "{err}");
    }

    #[test]
    fn importing_copies_to_a_private_temp_folder_that_goes_away() {
        let device = FakeDevice::kindle();
        let content: Vec<u8> = (0..200_000).map(|i| (i % 7) as u8).collect();
        device.put("documents/SF/Dune.azw3", &content);
        let (state, _) = connected(&device);
        let paths = vec![
            "/local/Book.epub".to_string(),
            "mtp://usb-1-2/documents/SF/Dune.azw3".to_string(),
        ];
        let (local, guards) = fetch_for_import(&state, &paths).unwrap();
        assert_eq!(local[0], "/local/Book.epub");
        assert!(local[1].ends_with("Dune.azw3"));
        assert_eq!(std::fs::read(&local[1]).unwrap(), content);
        drop(guards);
        assert!(!Path::new(&local[1]).exists(), "the temp copy is removed");

        let err = fetch_for_import(
            &state,
            &["mtp://usb-1-2/documents/Missing.azw3".to_string()],
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("Missing.azw3"),
            "a failed copy is an error, not a log line: {err}"
        );
        assert!(fetch_for_import(&state, &["mtp://usb-1-2/system/x.azw3".to_string()]).is_err());
    }

    #[test]
    fn ejecting_releases_and_reports_the_space() {
        let device = FakeDevice::kindle();
        let (state, _) = connected(&device);
        let space = space(&state, "mtp://usb-1-2");
        assert!(space.free_space_bytes.is_some());
        eject(&state, "mtp://usb-1-2").unwrap();
        assert!(detect(&state).is_empty(), "released until unplugged");
        assert_eq!(eject(&state, "mtp://usb-1-2").unwrap_err().code(), "DEVICE");
        assert_eq!(super::space(&state, "mtp://usb-1-2").free_space_bytes, None);
    }

    #[test]
    fn end_to_end_over_a_simulated_ptp_device() {
        use crate::commands::device::transfer::{JobContext, JobOutcome};
        use crate::models::device::TransferRequest;
        use crate::mtp::protocol::op;
        use crate::mtp::ptp::tests::{device as ptp_device, FakePipe};
        use crate::mtp::ptp::{PtpSession, Timeouts};
        use crate::mtp::session::Backend;

        /// The real PTP session, over the simulated responder.
        struct PtpBackend(FakePipe);
        impl Backend for PtpBackend {
            fn enumerate(&self) -> Vec<Candidate> {
                vec![Candidate {
                    key: KEY.into(),
                    label: "Kindle".into(),
                    native: "native-SERIAL".into(),
                }]
            }
            fn open(&self, _: &Candidate) -> Result<Box<dyn MtpSession>, MtpError> {
                Ok(Box::new(PtpSession::open_with(
                    self.0.clone(),
                    Timeouts::default(),
                    vec![Duration::ZERO; 3],
                )?))
            }
        }

        let pipe = ptp_device(512, &[op::GET_PARTIAL_OBJECT, op::SET_OBJECT_PROP_VALUE]);
        pipe.device().add(0, "documents", None);
        let (log, lines) = collect_logs();
        let state = MtpState::with_timing(
            Arc::new(PtpBackend(pipe.clone())),
            connect_hook(),
            log,
            Duration::from_secs(5),
            Duration::ZERO,
        );
        state.set_enabled(true);
        let devices = detect(&state);
        assert_eq!(devices.len(), 1, "{:?}", lines.lock().unwrap());
        assert_eq!(devices[0].name, "Kindle Paperwhite");

        // A book whose data container ends exactly on a packet boundary.
        let lib = tempfile::tempdir().unwrap();
        let src = lib.path().join("Dune.azw3");
        std::fs::write(&src, vec![7u8; 3 * 512 - 12]).unwrap();
        let job = TransferRequest {
            id: "job-1".into(),
            book_path: src.to_string_lossy().into_owned(),
            needs_conversion: false,
            target_format: "azw3".into(),
            device_path: mtp::mount_path(KEY),
            subfolder: Some("SF".into()),
            existing_device_path: None,
            book_id: None,
            library_path: None,
        };
        let temp = tempfile::tempdir().unwrap();
        let run = || {
            let ctx = JobContext {
                is_cancelled: &|| false,
                emit: &|_| {},
                log: &|_| {},
                free_space: &|_| None,
                temp_dir: temp.path(),
            };
            send::transfer_one(&job, &ctx, &state)
        };
        assert_eq!(
            run(),
            JobOutcome::Done("mtp://usb-1-2/documents/SF/Dune.azw3".into())
        );
        // Sent again: replaced through a temp copy and a rename.
        assert_eq!(
            run(),
            JobOutcome::Done("mtp://usb-1-2/documents/SF/Dune.azw3".into())
        );

        let books = list_books(&state, &mtp::mount_path(KEY)).unwrap();
        assert_eq!(
            books
                .iter()
                .map(|b| (b.path.as_str(), b.size_bytes))
                .collect::<Vec<_>>(),
            vec![("mtp://usb-1-2/documents/SF/Dune.azw3", 3 * 512 - 12),]
        );
        let (local, _guards) = fetch_for_import(&state, &[books[0].path.clone()]).unwrap();
        assert_eq!(std::fs::read(&local[0]).unwrap(), vec![7u8; 3 * 512 - 12]);
        delete_book(&state, &books[0].path).unwrap();
        assert!(list_books(&state, &mtp::mount_path(KEY))
            .unwrap()
            .is_empty());

        let objects: Vec<String> = pipe
            .device()
            .objects
            .values()
            .map(|o| o.name.clone())
            .collect();
        assert!(
            objects.iter().all(|n| !n.ends_with(".part")),
            "no temp copy left: {objects:?}"
        );
        let logged = lines.lock().unwrap().join("\n");
        assert!(
            !logged.contains("SERIAL"),
            "the serial never reaches the log:\n{logged}"
        );
    }

    #[test]
    fn model_names() {
        assert_eq!(
            model_name("Kindle Paperwhite"),
            Some("Kindle Paperwhite".into())
        );
        assert_eq!(model_name(" Colorsoft "), Some("Kindle Colorsoft".into()));
        assert_eq!(model_name(""), None);
    }
}
