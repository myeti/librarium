//! Reading-progress extraction for Kindle and Kobo devices.
//!
//! Each reader platform stores progress differently:
//!   - Kobo:         SQLite database at `.kobo/KoboReader.sqlite`
//!   - Kindle FW 5+: Binary `.mbs` file + `.apnx` page-offset table in per-book `.sdr` dirs
//!   - Kindle FW 1–3: Binary `.mbp` (Mobipocket Bookmark) files, brute-force scanned
//!   - MTP Kindles (2024+): the `.sdr` strategy on local copies of the
//!     device's `.sdr` files (`commands/device/mtp/progress.rs`; unverified)
//!
//! Every map returned here is keyed by [`progress_key`] — the book's file
//! name without its last extension — because that is what the library view
//! looks up (`services/readingProgress.ts`, docs/IPC.md).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, State};
use walkdir::WalkDir;

use crate::commands::device::kobo;
use crate::error::AppError;
use crate::logging::{log, Log};
use crate::mtp::{is_mtp_path, MtpState};

/// Key under which a book's progress is reported: its file name without the
/// last extension (`Dune.azw3` → `Dune`, `Book.kepub.epub` → `Book.kepub`).
///
/// Mirrors the frontend's `filename.replace(/\.[^.]+$/, '')` exactly,
/// including its edge cases (`.hidden` → empty, `name.` → unchanged), so the
/// two sides always agree.
pub fn progress_key(file_name: &str) -> String {
    match file_name.rfind('.') {
        Some(i) if i + 1 < file_name.len() => file_name[..i].to_string(),
        _ => file_name.to_string(),
    }
}

// ── Public command ────────────────────────────────────────────────────────────

/// Return a map of `book key → reading percentage (0–100)` for every book
/// with recorded progress on the device (keys: see [`progress_key`]).
/// Dispatches to the platform-specific implementation based on `device_type`
/// ("kindle" | "kobo"). Never rejects: a device that can't be read gives
/// an empty map (logged).
#[tauri::command]
pub async fn get_device_reading_progress(
    app: AppHandle,
    mtp_state: State<'_, MtpState>,
    device_path: String,
    device_type: String,
) -> Result<HashMap<String, u8>, AppError> {
    log(
        &app,
        format!(
            "[progress] device_path={} type={}",
            device_path, device_type
        ),
    );
    if is_mtp_path(&device_path) {
        // `.sdr` folders mirrored over MTP (unverified on a device; see the module).
        let mtp_state = MtpState::clone(&mtp_state);
        return Ok(tokio::task::spawn_blocking(move || {
            crate::commands::device::mtp::progress::reading_progress(&mtp_state, &device_path)
        })
        .await
        .unwrap_or_default());
    }
    let app2 = app.clone();

    // All progress extraction is synchronous I/O — run off the async executor.
    Ok(tokio::task::spawn_blocking(move || {
        let log_line = |m: String| log(&app2, m);
        reading_progress(Path::new(&device_path), &device_type, &log_line)
    })
    .await
    .unwrap_or_default())
}

/// Blocking implementation of [`get_device_reading_progress`].
fn reading_progress(mount: &Path, device_type: &str, log: Log) -> HashMap<String, u8> {
    match device_type {
        "kobo" => kobo_reading_progress(mount, log),
        "kindle" => {
            // Merge new-firmware results (authoritative) with old-firmware results (fallback).
            let mut map = kindle_new_reading_progress(mount, log);
            for (k, v) in kindle_old_reading_progress(mount, log) {
                map.entry(k).or_insert(v);
            }
            log(format!("[progress] kindle total entries={}", map.len()));
            map
        }
        _ => HashMap::new(),
    }
}

// ── Kobo ──────────────────────────────────────────────────────────────────────

/// Query `.kobo/KoboReader.sqlite`. The `content` table has a `___PercentRead`
/// column (0–100) and a `ContentID` that ends with the book's file name for
/// sideloaded books (e.g. `file:///mnt/onboard/Books/Dune.kepub.epub`).
///
/// Only `ContentType = 6` rows are books; the others are chapters, bookmarks
/// and the like, whose ContentIDs end in internal file names and would
/// produce bogus keys. (`= 6` also matches a TEXT column holding `'6'`: SQLite
/// applies the column's affinity to the literal.)
///
/// The database is read from a snapshot copy ([`kobo::open_snapshot`]), so
/// a WAL-mode or locked database still reads, and nothing is ever written
/// to the device.
///
/// TODO(device-validation): check against a real KoboReader.sqlite that
/// sideloaded ContentIDs are not percent-encoded (spaces, accents).
fn kobo_reading_progress(mount_path: &Path, log: Log) -> HashMap<String, u8> {
    log(format!(
        "[progress/kobo] db={}",
        kobo::database_path(mount_path).display()
    ));
    let snapshot = match kobo::open_snapshot(mount_path) {
        Ok(Some(snapshot)) => snapshot,
        Ok(None) => {
            log("[progress/kobo] db not found".to_string());
            return HashMap::new();
        }
        Err(e) => {
            log(format!("[progress/kobo] failed to open db: {e}"));
            return HashMap::new();
        }
    };
    let conn = &snapshot.conn;

    let mut map = HashMap::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT ContentID, ___PercentRead FROM content WHERE ContentType = 6 AND ___PercentRead > 0",
    ) {
        // f64 accepts both INTEGER and REAL storage of ___PercentRead.
        if let Ok(rows) = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?))
        }) {
            for (content_id, pct) in rows.flatten() {
                // ContentID is a path-like string; the file name is the last segment.
                let fname = content_id.rsplit('/').next().unwrap_or("");
                if fname.is_empty() {
                    continue;
                }
                let pct = pct.clamp(0.0, 100.0) as u8;
                // The same file name in two folders: keep the furthest progress.
                let entry = map.entry(progress_key(fname)).or_insert(pct);
                *entry = (*entry).max(pct);
            }
        }
    }

    log(format!("[progress/kobo] {} entries", map.len()));
    map
}

// ── Kindle FW 5+ (Paperwhite and newer) ──────────────────────────────────────

/// Two-pass strategy for Kindle FW 5.x+:
///   Pass 1 — `.sdr` companion directories alongside each ebook in `documents/`
///   Pass 2 — SQLite databases anywhere under `system/` (older Paperwhite variants)
fn kindle_new_reading_progress(mount_path: &Path, log: Log) -> HashMap<String, u8> {
    let mut map = HashMap::new();
    let base = mount_path.to_path_buf();

    // Pass 1: scan .sdr directories
    let docs = base.join("documents");
    if docs.exists() {
        let sdr_dirs: Vec<_> = WalkDir::new(&docs)
            .max_depth(3)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_type().is_dir()
                    && e.path().extension().and_then(|x| x.to_str()) == Some("sdr")
            })
            .collect();

        log(format!(
            "[progress/new-kindle] found {} .sdr dirs",
            sdr_dirs.len()
        ));

        for entry in &sdr_dirs {
            let path = entry.path();
            let Some(key) = path.file_name().and_then(|n| n.to_str()).and_then(sdr_key) else {
                continue;
            };
            if let Some(pct) = sdr_progress(path, log) {
                map.insert(key, pct);
            }
        }

        log(format!(
            "[progress/new-kindle] .sdr scan done → {} entries",
            map.len()
        ));
    }

    // Pass 2: SQLite databases under system/
    let system = base.join("system");
    if system.exists() {
        for entry in WalkDir::new(&system)
            .max_depth(4)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("db") {
                kindle_query_db(path, &mut map);
            }
        }
    }

    map
}

/// Ebook extensions a Kindle might keep in a `.sdr` folder name.
const SDR_BOOK_EXTENSIONS: &[&str] = &["azw3", "azw", "mobi", "prc", "pdf", "kfx", "epub", "txt"];

/// Progress key for a Kindle `.sdr` folder name.
///
/// Kindle names the folder after the book's stem (`documents/Dune.azw3` →
/// `documents/Dune.sdr`), so the key is the name minus `.sdr`. If a firmware
/// keeps the book extension (`Dune.azw3.sdr`), that is stripped too — only
/// for known ebook extensions, so a title like `Vol. 1` is left intact.
///
/// Validated on a Kindle Paperwhite 3 over USB on macOS (FAT32), 2026-11-09: progress of 105 books read through their `.sdr` folders
/// (`.mbs` + `.apnx`), matching what the Kindle shows.
pub(crate) fn sdr_key(dir_name: &str) -> Option<String> {
    let stem = dir_name.strip_suffix(".sdr")?;
    if stem.is_empty() {
        return None;
    }
    let inner = progress_key(stem);
    let inner_ext = stem[inner.len()..]
        .trim_start_matches('.')
        .to_ascii_lowercase();
    if inner != stem && !inner.is_empty() && SDR_BOOK_EXTENSIONS.contains(&inner_ext.as_str()) {
        Some(inner)
    } else {
        Some(stem.to_string())
    }
}

/// Try to extract a progress percentage from a `.sdr` companion directory.
/// Attempts several strategies in order of reliability:
///   1. Known SQLite file names inside the .sdr dir
///   2. SQLite files inside sub-directories of the .sdr dir
///   3. `.mbs` binary file + `.apnx` page-offset table (Paperwhite FW 5.x)
pub(crate) fn sdr_progress(sdr_dir: &Path, log: Log) -> Option<u8> {
    // Strategy 1 & 2: known SQLite database names
    let db_candidates = [
        "data.db",
        "YJBookmarkStorage.db",
        "BookmarkStorage.db",
        "KindleBookmarkStorage.db",
    ];
    for name in &db_candidates {
        if let Some(pct) = try_kindle_db(&sdr_dir.join(name)) {
            return Some(pct);
        }
    }
    for sub in std::fs::read_dir(sdr_dir).into_iter().flatten().flatten() {
        let sub_path = sub.path();
        if sub_path.is_dir() {
            for name in &db_candidates {
                if let Some(pct) = try_kindle_db(&sub_path.join(name)) {
                    return Some(pct);
                }
            }
        }
    }

    // Strategy 3: .mbs (furthest-position byte offset) + .apnx (page offsets)
    let stem = sdr_dir.file_stem()?.to_str()?;

    // Load APNX page table once; it's needed to convert a byte offset to a percentage.
    let apnx_path = sdr_dir.join(format!("{}.apnx", stem));
    let apnx = std::fs::read(&apnx_path)
        .ok()
        .as_deref()
        .and_then(parse_apnx);

    for entry in std::fs::read_dir(sdr_dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("mbs") {
            continue;
        }
        let Ok(data) = std::fs::read(&path) else {
            continue;
        };
        let Some(fpr) = parse_mbs_fpr(&data) else {
            continue;
        };

        if let Some((total_pages, ref offsets)) = apnx {
            if total_pages > 0 && !offsets.is_empty() {
                // Count page offsets ≤ fpr → current page (1-indexed)
                let current_page = offsets.partition_point(|&o| o <= fpr) as u32;
                let pct = ((current_page * 100) / total_pages).clamp(1, 100) as u8;
                log(format!(
                    "[progress/mbs] {} fpr={} → page {}/{} = {}%",
                    stem, fpr, current_page, total_pages, pct
                ));
                return Some(pct);
            }
        }
    }

    None
}

/// Extract the "furthest position reached" (fpr) byte offset from a `.mbs` binary file.
///
/// Binary encoding: key `"fpr"` (bytes 0x66 0x70 0x72) → type 0x03 (string)
/// → 3-byte big-endian length → ASCII decimal digits.
fn parse_mbs_fpr(data: &[u8]) -> Option<u32> {
    let pos = data.windows(3).position(|w| w == [0x66, 0x70, 0x72])?;
    let after = pos + 3;
    if after + 4 > data.len() || data[after] != 0x03 {
        return None;
    }
    let len = ((data[after + 1] as usize) << 16)
        | ((data[after + 2] as usize) << 8)
        | (data[after + 3] as usize);
    if len == 0 || len > 10 || after + 4 + len > data.len() {
        return None;
    }
    std::str::from_utf8(&data[after + 4..after + 4 + len])
        .ok()?
        .parse::<u32>()
        .ok()
}

/// Parse a Kindle `.apnx` file and return `(total_pages, sorted byte-offsets)`.
///
/// APNX layout:
/// ```text
///  0..11   fixed header (magic, total_header_size, main_json_len)
///  12..122 main JSON (contentGuid, asin, cdeType, fileRevisionId)
///  122..   page section
///    +0..+1  section type (00 01)
///    +2..+3  section JSON len (BE u16)
///    +4..+5  total page count (BE u16)   ← the key field
///    +6..+7  reserved
///    +8..    section JSON ("asin": "…")
///    rest    BE u32 page byte-offsets, one per page
/// ```
fn parse_apnx(data: &[u8]) -> Option<(u32, Vec<u32>)> {
    if data.len() < 12 {
        return None;
    }
    let main_json_len = u32::from_be_bytes([data[8], data[9], data[10], data[11]]) as usize;
    let sec = 12usize.checked_add(main_json_len)?; // page section start
    if sec.checked_add(8)? > data.len() {
        return None;
    }

    let sec_json_len = u16::from_be_bytes([data[sec + 2], data[sec + 3]]) as usize;
    let total_pages = u16::from_be_bytes([data[sec + 4], data[sec + 5]]) as u32;
    let offsets_start = sec + 8 + sec_json_len;

    if offsets_start > data.len() || total_pages == 0 {
        return None;
    }

    let (chunks, _) = data[offsets_start..].as_chunks::<4>();
    let offsets: Vec<u32> = chunks.iter().map(|c| u32::from_be_bytes(*c)).collect();

    if offsets.is_empty() {
        None
    } else {
        Some((total_pages, offsets))
    }
}

/// Open a single SQLite file and try known Kindle schema variants.
/// Returns `None` if the file doesn't match any known schema.
fn try_kindle_db(path: &Path) -> Option<u8> {
    if !path.exists() {
        return None;
    }
    let conn = rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;

    // Schema A: BOOK_LOCATION — position stored as fraction 0.0–1.0
    if let Ok(pos) = conn.query_row(
        "SELECT position FROM BOOK_LOCATION WHERE position > 0 LIMIT 1",
        [],
        |r| r.get::<_, f64>(0),
    ) {
        return Some((pos * 100.0).clamp(0.0, 100.0) as u8);
    }

    // Schema B: Bookmark — ChapterProgress (fraction OR raw 0–100, variant per firmware)
    if let Ok(pct) = conn.query_row(
        "SELECT MAX(ChapterProgress) FROM Bookmark WHERE ChapterProgress > 0",
        [],
        |r| r.get::<_, f64>(0),
    ) {
        let v = if pct <= 1.0 { pct * 100.0 } else { pct };
        return Some(v.clamp(0.0, 100.0) as u8);
    }

    None
}

/// Scan a Kindle SQLite database and populate `map` with `key → percent` entries.
fn kindle_query_db(path: &Path, map: &mut HashMap<String, u8>) {
    let Ok(conn) = rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) else {
        return;
    };

    // Schema A: BOOK_LOCATION(uuid, position 0.0–1.0). Keyed by uuid, which
    // the UI can't match to a file yet; kept until fixtures show whether this
    // schema ever appears.
    let schema_a: Vec<(String, f64)> = conn
        .prepare("SELECT uuid, position FROM BOOK_LOCATION WHERE position > 0")
        .and_then(|mut s| {
            s.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?)))
                .map(|rows| rows.flatten().collect())
        })
        .unwrap_or_default();

    for (uuid, pos) in schema_a {
        map.entry(uuid)
            .or_insert((pos * 100.0).clamp(0.0, 100.0) as u8);
    }

    // Schema B: Bookmark(ContentID, ChapterProgress)
    let schema_b: Vec<(String, f64)> = conn
        .prepare("SELECT ContentID, ChapterProgress FROM Bookmark WHERE ChapterProgress > 0")
        .and_then(|mut s| {
            s.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?)))
                .map(|rows| rows.flatten().collect())
        })
        .unwrap_or_default();

    for (content_id, raw) in schema_b {
        let fname = content_id.rsplit('/').next().unwrap_or("");
        if fname.is_empty() {
            continue;
        }
        let pct = if raw <= 1.0 { raw * 100.0 } else { raw };
        map.entry(progress_key(fname))
            .or_insert(pct.clamp(0.0, 100.0) as u8);
    }
}

// ── Kindle FW 1–3 (old firmware, Kindle 1 / 2 / DX) ─────────────────────────

/// Read `.mbp` (Mobipocket Bookmark) files. Scans both `system/` and `documents/`
/// directories since old Kindles store these files in either location.
fn kindle_old_reading_progress(mount_path: &Path, log: Log) -> HashMap<String, u8> {
    let mut map = HashMap::new();
    let base = mount_path.to_path_buf();
    let system_dir = base.join("system");
    let docs_dir = base.join("documents");

    // Log counts up front rather than per-file to avoid flooding the debug panel.
    let mbp_in_system = WalkDir::new(&system_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("mbp"))
        .count();
    let mbp_in_docs = WalkDir::new(&docs_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("mbp"))
        .count();
    log(format!(
        "[progress/old-kindle] .mbp files: system/={} documents/={}",
        mbp_in_system, mbp_in_docs
    ));

    for dir in [&system_dir, &docs_dir] {
        for entry in WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("mbp") {
                log(format!("[progress/old-kindle] mbp: {}", path.display()));
                parse_mbp_file(path, &base, &mut map, log);
            }
        }
    }

    log(format!("[progress/old-kindle] {} entries found", map.len()));
    map
}

/// Extract progress from a single `.mbp` file and insert into `map`.
///
/// Strategy: find the book file that matches the `.mbp` stem (sibling or in
/// `documents/`), then brute-force scan the binary data for a plausible byte
/// offset within that book's size.
///
/// TODO(device-validation): the brute-force scan probably returns a header
/// field (≈0%) on real files; validate against `.mbp` samples.
fn parse_mbp_file(mbp_path: &Path, device_root: &Path, map: &mut HashMap<String, u8>, log: Log) {
    let stem = match mbp_path.file_stem().and_then(|s| s.to_str()) {
        Some(s) => s.to_string(),
        None => return,
    };

    let data = match std::fs::read(mbp_path) {
        Ok(d) => d,
        Err(e) => {
            log(format!(
                "[progress/old-kindle] read error {}: {}",
                mbp_path.display(),
                e
            ));
            return;
        }
    };

    // Locate the corresponding book file (sibling first, then documents/).
    let parent = mbp_path.parent().unwrap_or(device_root);
    let book_path: Option<PathBuf> =
        ["mobi", "azw3", "azw", "prc", "epub"]
            .iter()
            .find_map(|ext| {
                let sibling = parent.join(format!("{}.{}", stem, ext));
                let in_docs = device_root
                    .join("documents")
                    .join(format!("{}.{}", stem, ext));
                if sibling.exists() {
                    Some(sibling)
                } else if in_docs.exists() {
                    Some(in_docs)
                } else {
                    None
                }
            });

    let Some(book_path) = book_path else { return };
    let book_size = match book_path.metadata() {
        Ok(m) => m.len(),
        Err(_) => return,
    };

    if let Some(pct) = parse_mbp_percent(&data, book_size) {
        // Key by the *book* file (`MyBook.mobi` → `MyBook`), like every other
        // source (`MyBook.mobi` would never match in the UI).
        let book_name = book_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&stem);
        map.insert(progress_key(book_name), pct);
    }
}

/// Brute-force scan all 4-byte-aligned positions in the MBP data for a value
/// that falls within `(0, book_size)` — the first plausible byte offset wins.
/// Both little-endian and big-endian interpretations are tried at each position.
fn parse_mbp_percent(data: &[u8], book_size: u64) -> Option<u8> {
    if book_size == 0 {
        return None;
    }
    for i in (0..data.len().saturating_sub(3)).step_by(4) {
        let le = u32::from_le_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]) as u64;
        let be = u32::from_be_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]) as u64;
        for pos in [le, be] {
            if pos > 0 && pos < book_size {
                return Some(((pos * 100) / book_size) as u8);
            }
        }
    }
    None
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging::no_log;
    use std::fs;

    #[test]
    fn key_is_the_file_name_without_its_last_extension() {
        assert_eq!(progress_key("Dune.azw3"), "Dune");
        assert_eq!(progress_key("Book.kepub.epub"), "Book.kepub");
        assert_eq!(progress_key("Vol. 1 - Author.mobi"), "Vol. 1 - Author");
        assert_eq!(progress_key("NoExtension"), "NoExtension");
        // Same edge cases as the frontend regex /\.[^.]+$/.
        assert_eq!(progress_key("trailing."), "trailing.");
        assert_eq!(progress_key(".hidden"), "");
        assert_eq!(progress_key("L'été.epub"), "L'été");
    }

    #[test]
    fn sdr_keys() {
        assert_eq!(sdr_key("Dune.sdr").as_deref(), Some("Dune"));
        assert_eq!(sdr_key("Dune.azw3.sdr").as_deref(), Some("Dune"));
        assert_eq!(sdr_key("Vol. 1.sdr").as_deref(), Some("Vol. 1"));
        assert_eq!(sdr_key("Dr. No.sdr").as_deref(), Some("Dr. No"));
        assert_eq!(sdr_key(".sdr"), None);
        assert_eq!(sdr_key("Dune"), None);
    }

    /// A KoboReader.sqlite with the columns the query uses, typed like the
    /// real one (ContentType is TEXT on device).
    fn kobo_fixture(mount: &Path) {
        let dir = mount.join(".kobo");
        fs::create_dir_all(&dir).unwrap();
        let conn = rusqlite::Connection::open(dir.join("KoboReader.sqlite")).unwrap();
        conn.execute_batch(
            "CREATE TABLE content (
                ContentID TEXT NOT NULL, ContentType TEXT NOT NULL,
                BookTitle TEXT, ___PercentRead INTEGER
             );
             INSERT INTO content VALUES ('file:///mnt/onboard/Books/Dune.kepub.epub', '6', NULL, 42);
             INSERT INTO content VALUES ('file:///mnt/onboard/Foundation.epub', '6', NULL, 100);
             INSERT INTO content VALUES ('file:///mnt/onboard/Unread.epub', '6', NULL, 0);
             INSERT INTO content VALUES ('file:///mnt/onboard/Dune.kepub.epub!OEBPS!ch1.xhtml', '9', 'Dune', 77);
             INSERT INTO content VALUES ('file:///mnt/onboard/Foundation.epub#(3)OEBPS/ch03.xhtml', '899', 'Foundation', 12);",
        )
        .unwrap();
    }

    #[test]
    fn kobo_progress_is_keyed_like_the_ui_and_ignores_non_books() {
        let mount = tempfile::tempdir().unwrap();
        kobo_fixture(mount.path());

        let map = reading_progress(mount.path(), "kobo", &no_log);

        let expected: HashMap<String, u8> = [
            ("Dune.kepub".to_string(), 42),
            ("Foundation".to_string(), 100),
        ]
        .into();
        assert_eq!(map, expected);
    }

    #[test]
    fn kobo_progress_still_in_the_wal_is_read() {
        let mount = tempfile::tempdir().unwrap();
        kobo_fixture(mount.path());
        // A WAL-mode database whose latest change is only in the -wal file,
        // with the writer still holding it open.
        let writer =
            rusqlite::Connection::open(mount.path().join(".kobo/KoboReader.sqlite")).unwrap();
        writer.pragma_update(None, "journal_mode", "WAL").unwrap();
        writer.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
        writer
            .execute(
                "UPDATE content SET ___PercentRead = 64 WHERE ContentID LIKE '%Dune.kepub.epub'",
                [],
            )
            .unwrap();

        let map = reading_progress(mount.path(), "kobo", &no_log);
        assert_eq!(map.get("Dune.kepub"), Some(&64));
        drop(writer);
    }

    #[test]
    fn kobo_without_database_is_empty() {
        let mount = tempfile::tempdir().unwrap();
        assert!(reading_progress(mount.path(), "kobo", &no_log).is_empty());
    }

    #[test]
    fn old_kindle_mbp_progress_uses_the_book_key() {
        let mount = tempfile::tempdir().unwrap();
        let docs = mount.path().join("documents");
        fs::create_dir_all(&docs).unwrap();
        fs::write(docs.join("MyBook.mobi"), vec![0u8; 1000]).unwrap();
        // First plausible offset: 500 (little-endian) of a 1000-byte book.
        let mut mbp = 500u32.to_le_bytes().to_vec();
        mbp.extend_from_slice(&[0u8; 12]);
        fs::write(docs.join("MyBook.mbp"), mbp).unwrap();

        let map = reading_progress(mount.path(), "kindle", &no_log);

        assert_eq!(map.get("MyBook"), Some(&50));
        assert!(!map.contains_key("MyBook.mobi"));
    }

    #[test]
    fn new_kindle_sdr_progress_from_mbs_and_apnx() {
        let mount = tempfile::tempdir().unwrap();
        let sdr = mount.path().join("documents/Dune.sdr");
        fs::create_dir_all(&sdr).unwrap();
        fs::write(mount.path().join("documents/Dune.azw3"), b"book").unwrap();

        // .mbs: "fpr" + type 0x03 + 3-byte length + ASCII digits.
        let mut mbs = b"\x00\x00fpr\x03\x00\x00\x04".to_vec();
        mbs.extend_from_slice(b"2500");
        fs::write(sdr.join("Dune.mbs"), mbs).unwrap();

        // .apnx: 12-byte header with empty main JSON, then a page section of
        // 4 pages at offsets 0, 1000, 2000, 3000.
        let mut apnx = vec![0u8; 8];
        apnx.extend_from_slice(&0u32.to_be_bytes());
        apnx.extend_from_slice(&[0, 1, 0, 0, 0, 4, 0, 0]);
        for off in [0u32, 1000, 2000, 3000] {
            apnx.extend_from_slice(&off.to_be_bytes());
        }
        fs::write(sdr.join("Dune.apnx"), apnx).unwrap();

        let map = reading_progress(mount.path(), "kindle", &no_log);
        assert_eq!(map.get("Dune"), Some(&75), "page 3 of 4");
    }

    #[test]
    fn apnx_parser_rejects_truncated_or_absurd_headers() {
        assert_eq!(parse_apnx(&[]), None);
        let mut huge = vec![0u8; 8];
        huge.extend_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(parse_apnx(&huge), None, "no overflow on a huge JSON length");
    }

    #[test]
    fn unknown_device_type_is_empty() {
        let mount = tempfile::tempdir().unwrap();
        assert!(reading_progress(mount.path(), "nook", &no_log).is_empty());
    }
}
