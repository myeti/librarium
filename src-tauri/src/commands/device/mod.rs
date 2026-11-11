//! E-readers connected over USB mass storage (Kindle, Kobo): detection,
//! listing, sending books, deleting them, ejecting.
//!
//! Each `#[tauri::command]` here is a thin wrapper running blocking,
//! `AppHandle`-free code from the submodules, where it is unit-tested
//! against temp folders acting as fake devices:
//!
//! | Module       | Responsibility |
//! |--------------|----------------|
//! | [`detect`]   | finding every connected e-reader: structure check, model, stable id, free/total space |
//! | [`manifest`] | `.librarium/device.json`: which library book each file Librarium sent is |
//! | [`transfer`] | sending one book: conversion, the id in the copy, atomic copy, replacing its previous copy; `.part` cleanup |
//! | [`identity`] | the library id inside device files (AZW3 ASIN, EPUB `urn:librarium:`) |
//! | [`listing`]  | the books on a device (Kindle `documents/`, the whole Kobo volume), deleting one |
//! | [`kobo`]     | reading `KoboReader.sqlite` safely (a snapshot copy, WAL included) |
//! | [`eject`]    | safe eject (macOS `diskutil`, Linux `udisksctl`/`gio`) |
//! | [`state`]    | cancelled jobs; devices busy sending or ejecting |
//!
//! | [`mtp`]      | MTP Kindles (2024+, experimental): the same commands over MTP (transport in `crate::mtp`) |
//!
//! Reading progress lives in `commands/progress.rs`.
//!
//! E-readers are FAT/exFAT volumes that can be unplugged at any moment:
//! every write goes through [`crate::fsutil`] (hidden `.part`, fsync,
//! rename), every path from the webview is confined to a detected device
//! ([`crate::fsutil::is_within`]), and the only file a transfer ever deletes
//! is the previous copy of the same book, when the manifest says so (see
//! `transfer.rs`).
//!
//! **MTP Kindles** (behind the `mtpEnabled` setting) have `mtp://<port>`
//! mount paths and `connection: "mtp"`; every command below dispatches
//! on [`crate::mtp::is_mtp_path`] to [`mtp`], which runs the same steps
//! as jobs on the device's worker thread. `get_mtp_status` reports the MTP
//! Kindles that aren't usable (MTP disabled, held by another app…) and why.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tauri::{AppHandle, Emitter, State};

use crate::commands::library::blocking;
use crate::error::{display_name, AppError, AppResult};
use crate::fsutil;
use crate::logging::{log, logger};
use crate::models::device::{KindleDevice, TransferRequest};
use crate::mtp::{is_mtp_path, MtpDevice, MtpState};

pub(crate) mod detect;
pub(crate) mod eject;
pub(crate) mod identity;
pub(crate) mod kobo;
pub(crate) mod listing;
pub(crate) mod manifest;
pub(crate) mod mtp;
mod state;
pub(crate) mod thumbnail;
pub(crate) mod transfer;

pub use listing::DeviceBook;
use state::mount_key;
pub use state::TransferState;
pub use transfer::{JobContext, TransferProgress, TransferStatus};

/// Event sent after each transfer job with the device's new free space.
pub(crate) const DEVICE_SPACE_EVENT: &str = "device-space";

/// Payload of [`DEVICE_SPACE_EVENT`].
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceSpace {
    pub mount_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub free_space_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_space_bytes: Option<u64>,
}

// ── Detection ─────────────────────────────────────────────────────────────────

/// Every connected Kindle and Kobo, in a stable order: the USB drives,
/// then the MTP Kindles that are connected (only with MTP enabled; see
/// [`get_mtp_status`] for the others). Polled by the frontend: each device
/// has a stable `id` (a different id at the same mount path means the
/// device was swapped) and its current free/total space.
///
/// When a drive appears, the `.librarium-*.part` files an interrupted
/// transfer left on it are removed (unless something is being sent to it).
/// Only logs when the result changes.
#[tauri::command]
pub async fn detect_devices(
    app: AppHandle,
    state: State<'_, TransferState>,
    mtp_state: State<'_, MtpState>,
) -> Result<Vec<KindleDevice>, AppError> {
    let transfers = TransferState::clone(&state);
    let mtp_state = MtpState::clone(&mtp_state);
    let log_line = logger(&app);
    let devices = tokio::task::spawn_blocking(move || {
        let mut devices = detect::detect_all(&log_line);
        for device in newly_connected(&devices) {
            let mount = Path::new(&device.mount_path);
            if transfers.is_sending(mount) {
                continue;
            }
            let removed = transfer::clean_stale_parts(mount, transfer::STALE_PART_AGE, &log_line);
            if !removed.is_empty() {
                log_line(format!(
                    "[detect_devices] removed {} file(s) left by an interrupted transfer from {}",
                    removed.len(),
                    device.name
                ));
            }
        }
        devices.extend(mtp::detect(&mtp_state));
        devices
    })
    .await
    .unwrap_or_default();

    let summary = if devices.is_empty() {
        "no device".to_string()
    } else {
        devices
            .iter()
            .map(|d| format!("{} ({}) at {}", d.name, d.device_type, d.mount_path))
            .collect::<Vec<_>>()
            .join(", ")
    };
    if detection_changed(&summary) {
        log(&app, format!("[detect_devices] {summary}"));
    }
    Ok(devices)
}

/// The first of [`detect_devices`], or `None`: for frontends that handle
/// one device.
#[tauri::command]
pub async fn detect_device(
    app: AppHandle,
    state: State<'_, TransferState>,
    mtp_state: State<'_, MtpState>,
) -> Result<Option<KindleDevice>, AppError> {
    Ok(detect_devices(app, state, mtp_state)
        .await?
        .into_iter()
        .next())
}

/// `get_mtp_status`: whether MTP support is on, and every MTP Kindle
/// plugged in with its state (see [`crate::mtp::state::DeviceState`]).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MtpStatus {
    /// The `mtpEnabled` setting.
    pub enabled: bool,
    pub devices: Vec<MtpDevice>,
}

/// The MTP Kindles plugged in and what each is doing: connected, or why
/// not — MTP support off (`mtpDisabled`: suggest the setting), held by
/// another app or not permitted (`claimFailed`, with what to do), busy,
/// connecting, ejected. Cheap (USB enumeration; nothing is claimed while
/// MTP is off): poll it with `detect_devices`.
#[tauri::command]
pub async fn get_mtp_status(mtp_state: State<'_, MtpState>) -> AppResult<MtpStatus> {
    let mtp_state = MtpState::clone(&mtp_state);
    blocking(move || {
        let devices = mtp_state
            .poll()
            .into_iter()
            .map(|(device, _)| device)
            .collect();
        Ok(MtpStatus {
            enabled: mtp_state.is_enabled(),
            devices,
        })
    })
    .await
}

/// Remember the last detection summary; `true` when `summary` differs from it.
fn detection_changed(summary: &str) -> bool {
    static LAST: Mutex<String> = Mutex::new(String::new());
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    if *last == summary {
        false
    } else {
        *last = summary.to_string();
        true
    }
}

/// The devices in `devices` that weren't there at the previous call.
fn newly_connected(devices: &[KindleDevice]) -> Vec<&KindleDevice> {
    static SEEN: Mutex<Option<HashSet<String>>> = Mutex::new(None);
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    take_new(seen.get_or_insert_with(HashSet::new), devices)
}

/// [`newly_connected`] with the memory as a parameter: a device is new when
/// its `(id, mount)` pair wasn't in `seen`; `seen` becomes the current set.
fn take_new<'a>(seen: &mut HashSet<String>, devices: &'a [KindleDevice]) -> Vec<&'a KindleDevice> {
    let key = |d: &KindleDevice| format!("{}\u{0}{}", d.id, d.mount_path);
    let new = devices.iter().filter(|d| !seen.contains(&key(d))).collect();
    *seen = devices.iter().map(key).collect();
    new
}

// ── Listing and deleting ──────────────────────────────────────────────────────

/// File names of the books on the device (see [`list_device_books`] for
/// where they are looked for).
#[tauri::command]
pub async fn scan_device_library(
    mtp_state: State<'_, MtpState>,
    device_path: String,
) -> AppResult<Vec<String>> {
    if is_mtp_path(&device_path) {
        let mtp_state = MtpState::clone(&mtp_state);
        let books = blocking(move || mtp::list_books(&mtp_state, &device_path)).await?;
        return Ok(books.into_iter().map(|b| b.filename).collect());
    }
    Ok(tokio::task::spawn_blocking(move || {
        listing::book_files(Path::new(&device_path))
            .iter()
            .filter_map(|p| p.file_name().and_then(|n| n.to_str()).map(str::to_string))
            .collect()
    })
    .await
    .unwrap_or_default())
}

/// Write the Kindle home-screen covers that are missing for books Librarium
/// sent there (books sent before covers were written, or whose cover write
/// failed; see `thumbnail::repair`), from `library_path`'s covers. Returns
/// how many were written: 0 on a Kobo or an MTP Kindle.
#[tauri::command]
pub async fn repair_device_covers(
    app: AppHandle,
    library_path: String,
    device_path: String,
) -> AppResult<u32> {
    if is_mtp_path(&device_path) {
        return Ok(0);
    }
    let log_line = logger(&app);
    let written = tokio::task::spawn_blocking(move || {
        thumbnail::repair(Path::new(&device_path), Path::new(&library_path), &log_line)
    })
    .await
    .unwrap_or_default();
    Ok(u32::try_from(written).unwrap_or(u32::MAX))
}

/// The books on the device: Kindle `documents/`, or the whole Kobo volume
/// merged with `KoboReader.sqlite`; each with `libraryId` when Librarium
/// knows which library book it is (device manifest, or the id inside the
/// file). An MTP Kindle is listed over MTP (`DEVICE` when it can't be).
#[tauri::command]
pub async fn list_device_books(
    app: AppHandle,
    mtp_state: State<'_, MtpState>,
    device_path: String,
) -> AppResult<Vec<DeviceBook>> {
    if is_mtp_path(&device_path) {
        let mtp_state = MtpState::clone(&mtp_state);
        return blocking(move || mtp::list_books(&mtp_state, &device_path)).await;
    }
    let log_line = logger(&app);
    Ok(
        tokio::task::spawn_blocking(move || {
            listing::list_books(Path::new(&device_path), &log_line)
        })
        .await
        .unwrap_or_default(),
    )
}

/// Remove one ebook file from the device, with (Kindle) its `.sdr` reading
/// state folder and its manifest entry.
///
/// The path comes from the webview, so it must be an ebook file inside a
/// currently connected device; anything else is refused.
#[tauri::command]
pub async fn delete_device_book(
    app: AppHandle,
    mtp_state: State<'_, MtpState>,
    path: String,
) -> AppResult<()> {
    if is_mtp_path(&path) {
        let mtp_state = MtpState::clone(&mtp_state);
        return blocking(move || mtp::delete_book(&mtp_state, &path)).await;
    }
    let log_line = logger(&app);
    blocking(move || {
        let path = PathBuf::from(&path);
        let name = display_name(&path);
        if !path.is_file() || !listing::is_ebook(&path) {
            return Err(AppError::InvalidInput(format!(
                "Not an ebook file on the device: {name}"
            )));
        }
        let device = detect::detect_all(&log_line)
            .into_iter()
            .find(|d| fsutil::is_within(Path::new(&d.mount_path), &path))
            .ok_or_else(|| AppError::Device(format!("{name} isn't on a connected e-reader")))?;
        let deleted = listing::delete_book(Path::new(&device.mount_path), &path, &log_line)?;
        log_line(format!(
            "[delete_device_book] deleted {}{}{}",
            path.display(),
            if deleted.sdr.is_empty() {
                String::new()
            } else {
                format!(" and {} .sdr folder(s)", deleted.sdr.len())
            },
            if deleted.forgotten.is_empty() {
                String::new()
            } else {
                format!("; forgot {}", deleted.forgotten.join(", "))
            },
        ));
        Ok(())
    })
    .await
}

// ── Transfers ─────────────────────────────────────────────────────────────────

/// Copy or convert books to the device's `documents/` directory (or
/// subfolder); see `transfer.rs`. Each job is processed independently — a
/// failure on one does not abort the others; the frontend is notified per
/// job via `transfer-progress`, and after each job with the device's free
/// space (`device-space`). Refused (`DEVICE`) while one of the devices is
/// being ejected. Jobs for an MTP Kindle go over MTP (`mtp/send.rs`).
#[tauri::command]
pub async fn send_books(
    app: AppHandle,
    state: State<'_, TransferState>,
    mtp_state: State<'_, MtpState>,
    jobs: Vec<TransferRequest>,
) -> AppResult<()> {
    log(&app, format!("[send_books] received {} job(s)", jobs.len()));
    let ids: Vec<String> = jobs.iter().map(|j| j.id.clone()).collect();
    let mounts: Vec<PathBuf> = jobs.iter().map(|j| PathBuf::from(&j.device_path)).collect();
    // Marks the devices busy (no eject, no `.part` cleanup) until the end.
    let _busy = state.begin_sending(&mounts)?;

    for job in jobs {
        let job_id = job.id.clone();
        let transfers = TransferState::clone(&state);
        let mtp_state = MtpState::clone(&mtp_state);
        let app2 = app.clone();
        // Conversion and copy are blocking; a panic inside (e.g. boko on a
        // malformed book) only fails this job.
        let joined = tokio::task::spawn_blocking(move || {
            let temp_dir = std::env::temp_dir();
            let is_cancelled = || transfers.is_cancelled(&job.id);
            let emit = |p: TransferProgress| {
                let _ = app2.emit("transfer-progress", p);
            };
            let log_line = |m: String| log(&app2, m);
            let free_space = |dir: &Path| detect::space_info(dir).0;
            let ctx = JobContext {
                is_cancelled: &is_cancelled,
                emit: &emit,
                log: &log_line,
                free_space: &free_space,
                temp_dir: &temp_dir,
            };
            if is_mtp_path(&job.device_path) {
                mtp::send::transfer_one(&job, &ctx, &mtp_state);
                mtp::space(&mtp_state, &job.device_path)
            } else {
                transfer::transfer_one(&job, &ctx);
                space_of(&job.device_path)
            }
        })
        .await;

        match joined {
            Ok(space) => {
                let _ = app.emit(DEVICE_SPACE_EVENT, space);
            }
            Err(e) => {
                let msg = format!("Transfer failed unexpectedly: {e}");
                log(&app, format!("[send_books] ERROR job {job_id}: {msg}"));
                let _ = app.emit(
                    "transfer-progress",
                    TransferProgress {
                        job_id,
                        status: TransferStatus::Error,
                        progress: 0,
                        error_message: Some(msg),
                    },
                );
            }
        }
    }

    state.forget(&ids);
    log(&app, "[send_books] all jobs processed");
    Ok(())
}

/// The [`DEVICE_SPACE_EVENT`] payload for the device at `mount`.
fn space_of(mount: &str) -> DeviceSpace {
    let (free, total) = detect::space_info(Path::new(mount));
    DeviceSpace {
        mount_path: mount.to_string(),
        free_space_bytes: free,
        total_space_bytes: total,
    }
}

/// Cancel a transfer job. A queued job is skipped; a running one stops at its
/// next checkpoint (after conversion, or between 1 MB chunks of the copy) and
/// its partial file is removed. The job then emits status `cancelled`. A job
/// that already finished is unaffected.
#[tauri::command]
pub async fn cancel_transfer(state: State<'_, TransferState>, job_id: String) -> AppResult<()> {
    state.cancel(&job_id);
    Ok(())
}

// ── Ejecting ──────────────────────────────────────────────────────────────────

/// Safely eject the e-reader at `mount_path` (see `eject.rs`). Refused unless
/// it is a connected e-reader (`DEVICE`: never any other disk), and while
/// books are being sent to it (`CONFLICT`). On Windows it is not done yet:
/// the error tells the user to use the system tray. An MTP Kindle is
/// released (its session closed) and stays so until it is unplugged.
#[tauri::command]
pub async fn eject_device(
    app: AppHandle,
    state: State<'_, TransferState>,
    mtp_state: State<'_, MtpState>,
    mount_path: String,
) -> AppResult<()> {
    let transfers = TransferState::clone(&state);
    if is_mtp_path(&mount_path) {
        let mtp_state = MtpState::clone(&mtp_state);
        return blocking(move || {
            let _ejecting = transfers.begin_eject(Path::new(&mount_path))?;
            mtp::eject(&mtp_state, &mount_path)
        })
        .await;
    }
    let log_line = logger(&app);
    blocking(move || {
        let mount = PathBuf::from(&mount_path);
        let device = detect::detect_all(&log_line)
            .into_iter()
            .find(|d| mount_key(Path::new(&d.mount_path)) == mount_key(&mount))
            .ok_or_else(|| {
                AppError::Device(format!(
                    "{} isn't a connected e-reader",
                    display_name(&mount)
                ))
            })?;
        let _ejecting = transfers.begin_eject(&mount)?;
        eject::eject(Path::new(&device.mount_path), &log_line)
    })
    .await
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::fs::{self, File};
    use std::io::Write;

    /// Write a minimal but valid EPUB 2 at `path` (one chapter), for tests
    /// that need boko to convert something.
    pub(crate) fn minimal_epub(path: &Path) {
        use zip::write::SimpleFileOptions;
        let file = File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let stored =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        zip.start_file("mimetype", stored).unwrap();
        zip.write_all(b"application/epub+zip").unwrap();
        zip.start_file("META-INF/container.xml", stored).unwrap();
        zip.write_all(br#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>"#).unwrap();
        zip.start_file("OEBPS/content.opf", stored).unwrap();
        zip.write_all(
            br#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Test Book</dc:title><dc:creator>Tester</dc:creator>
    <dc:identifier id="id">urn:uuid:00000000-0000-0000-0000-000000000001</dc:identifier>
    <dc:language>en</dc:language>
  </metadata>
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
  </manifest>
  <spine toc="ncx"><itemref idref="ch1"/></spine>
</package>"#,
        )
        .unwrap();
        zip.start_file("OEBPS/toc.ncx", stored).unwrap();
        zip.write_all(br#"<?xml version="1.0" encoding="UTF-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <head><meta name="dtb:uid" content="urn:uuid:00000000-0000-0000-0000-000000000001"/></head>
  <docTitle><text>Test Book</text></docTitle>
  <navMap><navPoint id="n1" playOrder="1"><navLabel><text>One</text></navLabel><content src="ch1.xhtml"/></navPoint></navMap>
</ncx>"#).unwrap();
        zip.start_file("OEBPS/ch1.xhtml", stored).unwrap();
        zip.write_all(
            br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>One</title></head>
<body><h1>Chapter one</h1><p>Hello, Kindle.</p></body></html>"#,
        )
        .unwrap();
        zip.finish().unwrap();
    }

    /// A fake Kindle: a temp dir with `documents/`.
    pub(crate) fn fake_kindle() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("documents")).unwrap();
        dir
    }

    /// A fake Kobo: a temp dir with `.kobo/` (no database).
    pub(crate) fn fake_kobo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join(".kobo")).unwrap();
        dir
    }

    fn device(id: &str, mount: &str) -> KindleDevice {
        KindleDevice {
            id: id.into(),
            name: "Kindle".into(),
            mount_path: mount.into(),
            device_type: "kindle".into(),
            connection: "massStorage".into(),
            preferred_format: "azw3".into(),
            model: None,
            free_space_bytes: None,
            total_space_bytes: None,
        }
    }

    #[test]
    fn a_device_is_new_when_it_appears_or_is_swapped() {
        let mut seen = HashSet::new();
        let a = [device("a", "/Volumes/Kindle")];
        assert_eq!(take_new(&mut seen, &a).len(), 1, "first sight");
        assert!(take_new(&mut seen, &a).is_empty(), "still there");
        let both = [
            device("a", "/Volumes/Kindle"),
            device("b", "/Volumes/Kindle 1"),
        ];
        assert_eq!(
            take_new(&mut seen, &both)
                .iter()
                .map(|d| d.id.as_str())
                .collect::<Vec<_>>(),
            vec!["b"]
        );
        let swapped = [device("c", "/Volumes/Kindle")];
        assert_eq!(
            take_new(&mut seen, &swapped)
                .iter()
                .map(|d| d.id.as_str())
                .collect::<Vec<_>>(),
            vec!["c"]
        );
        assert!(take_new(&mut seen, &[]).is_empty());
        assert_eq!(take_new(&mut seen, &a).len(), 1, "reconnected");
    }

    #[test]
    fn detection_logs_only_changes() {
        // Unique summaries so this test doesn't depend on other tests' calls.
        assert!(detection_changed("test: found A"));
        assert!(!detection_changed("test: found A"));
        assert!(detection_changed("test: no device"));
    }

    #[test]
    fn space_payload_matches_the_ipc_contract() {
        let dir = tempfile::tempdir().unwrap();
        let space = space_of(&dir.path().to_string_lossy());
        assert!(space.free_space_bytes.is_some() && space.total_space_bytes.is_some());
        let json = serde_json::to_value(DeviceSpace {
            mount_path: "/Volumes/Kindle".into(),
            free_space_bytes: Some(1),
            total_space_bytes: None,
        })
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "mountPath": "/Volumes/Kindle", "freeSpaceBytes": 1 })
        );
    }
}
