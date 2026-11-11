//! Library commands: the IPC layer for everything that happens inside the
//! library folder.
//!
//! Each `#[tauri::command]` here is a thin wrapper: it turns its arguments
//! into paths, picks a log sink ([`logger`]) and, for blocking work, runs a
//! plain, `AppHandle`-free function on the blocking pool ([`blocking`]). The
//! work itself lives in the submodules, where it is unit-tested:
//!
//! | Module          | Responsibility |
//! |-----------------|----------------|
//! | [`scan`]        | supported formats, walking the library, building `Book` records (panic-isolated), scan and prune |
//! | [`failed`]      | `.librarium/failed.json`: files that couldn't be read, skipped until they change |
//! | [`import`]      | copying files and downloads into the library, re-import detection |
//! | [`metadata`]    | reading metadata from EPUB/PDF files, language codes and detection |
//! | [`edit`]        | rename, metadata edits (single and batch), reset, delete |
//! | [`covers`]      | cover extraction, the cover cache and thumbnails, setting and removing covers |
//! | [`convert`]     | MOBI/AZW/AZW3 → EPUB conversion in place, export to another format (with [`crate::convert`]) |
//! | [`users`]       | user folders (list, create) |
//! | [`os`]          | reveal in the file manager, folder reachability |
//! | [`inspect`]     | the dev-mode EPUB inspector (debug builds only) |
//! | [`index`]       | the library index (`.librarium/library.json`, format v2): load, save, backups, locking, cache |
//! | [`relkey`]      | index keys: paths relative to the library root, NFC, confined to the root |
//! | [`migrate`]     | migrating the legacy v1 index (`books.json`, absolute paths; never written) to v2 |
//! | [`lock`]        | `.librarium/lock.json`: noticing another instance using the library |
//! | [`epub_edit`]   | rewriting EPUB files: metadata write-back, cover embedding/removal |
//! | [`files`]       | rules for book files: stay inside the library, safe names, permissions, trash |
//! | [`sniff`]       | recognising images and ebooks by their magic bytes |
//!
//! HTTP downloads (covers, books) go through the shared [`crate::net`] layer.
//!
//! Commands identify books by id; paths come from the index, never from the
//! client (except where a command documents otherwise). Every command
//! returns [`AppError`] on failure (`{ code, message }` over IPC).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tauri::{AppHandle, Manager};

use crate::convert::OutputFormat;
use crate::error::{AppError, AppResult};
use crate::fsutil;
use crate::logging::{log, log_global, logger};
use crate::models::book::{
    BatchUpdateResult, Book, BookMetadata, BookMetadataPatch, DeleteResult, ForgetResult,
};
use crate::net;
use covers::CoverInfo;

mod convert;
pub(crate) mod covers;
mod edit;
pub(crate) mod epub_edit;
pub(crate) mod failed;
pub(crate) mod files;
pub(crate) mod import;
pub(crate) mod index;
#[cfg(debug_assertions)]
mod inspect;
mod lock;
pub(crate) mod metadata;
mod migrate;
pub(crate) mod os;
mod relkey;
pub(crate) mod scan;
pub(crate) mod sniff;
#[cfg(test)]
pub(crate) mod test_support;
pub(crate) mod users;

use index::find_book;

/// Run blocking work (filesystem, ZIP, locks, SQLite) on the blocking thread
/// pool instead of an async worker or the main thread. A panic inside `f` (a
/// malformed book) becomes an [`AppError::Internal`] for this command
/// instead of taking the app down.
///
/// Every error is also written to the debug log and the log file, with its
/// code and labelled with the command it came from, so "Copy all" in the
/// debug panel (or the log file) has the whole story even when the UI only
/// showed a short toast.
pub(crate) async fn blocking<T, F>(f: F) -> AppResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> AppResult<T> + Send + 'static,
{
    let label = command_label::<F>();
    let result = match tokio::task::spawn_blocking(f).await {
        Ok(result) => result,
        Err(e) => Err(AppError::from(e)),
    };
    if let Err(e) = &result {
        log_global(format!("[{label}] ERROR ({}) — {e}", e.code()));
    }
    result
}

/// The name of the command a [`blocking`] closure was written in, from its
/// type name (`…::library::scan_library::{{closure}}::{{closure}}` →
/// `scan_library`). Only a label for the log.
fn command_label<F>() -> &'static str {
    let name = std::any::type_name::<F>();
    let mut path = name;
    while let Some(stripped) = path.strip_suffix("::{{closure}}") {
        path = stripped;
    }
    path.rsplit("::")
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(name)
}

/// How long quitting waits for the lock files to be removed.
const RELEASE_LOCKS_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(1500);

/// Remove the `.librarium/lock.json` files this instance holds (on exit).
///
/// Called from the event loop (the main thread) while the app quits. The
/// files may be on a NAS that stopped answering, so the work runs on its own
/// thread and quitting waits for it at most [`RELEASE_LOCKS_TIMEOUT`]. A
/// file left behind costs little: another computer warns for a few minutes,
/// and this one ignores its own entries on the next launch.
pub(crate) fn release_library_locks() {
    if !run_bounded(RELEASE_LOCKS_TIMEOUT, lock::release_all) {
        log::warn!("quitting without removing every library lock file (the library folder did not answer in time)");
    }
}

/// Run `f` on a new thread and wait for it at most `timeout`. Returns whether
/// it finished in time; if not, it carries on in the background (or is cut
/// short when the process exits).
fn run_bounded(timeout: std::time::Duration, f: impl FnOnce() + Send + 'static) -> bool {
    let (done, finished) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("librarium-release-locks".into())
        .spawn(move || {
            f();
            let _ = done.send(());
        });
    spawned.is_ok() && finished.recv_timeout(timeout).is_ok()
}

// ── Scan and import ───────────────────────────────────────────────────────────

/// Walk the library directory and return all known books. Books already in
/// the index are returned as-is; new files are parsed (or re-linked to a
/// missing book) and added; entries whose file is gone become "missing"
/// after a complete walk; files that can't be read are recorded in
/// `failed.json` and skipped until they change ([`scan::scan_library_at`]).
/// Books whose cover thumbnail exists get `coverPath`/`coverVersion`
/// ([`covers::attach_cached_covers`]: a directory listing, no image work);
/// the webview may then load the library's covers.
#[tauri::command]
pub async fn scan_library(app: AppHandle, path: String) -> AppResult<Vec<Book>> {
    let log = logger(&app);
    let lib = PathBuf::from(&path);
    let books = blocking(move || {
        let mut books = scan::scan_library_at(&lib, &log)?;
        covers::attach_cached_covers(&lib, &mut books);
        Ok(books)
    })
    .await?;
    allow_covers_dir(&app, Path::new(&path));
    Ok(books)
}

/// Copy the given file paths into the library directory and extract metadata.
/// Files keep their original filename; a UUID suffix is appended only when the
/// name is taken by a different book. A file the library already holds is not
/// copied again — its existing book is returned instead, so importing the same
/// file twice is a no-op ([`import::import_files`]).
///
/// `mtp://` paths (books on an MTP Kindle) are first copied from the device
/// into private temp folders, removed afterwards; if one can't be copied
/// the command fails (`DEVICE`, with the reason) and nothing is imported.
#[tauri::command]
pub async fn import_books(
    app: AppHandle,
    mtp_state: tauri::State<'_, crate::mtp::MtpState>,
    paths: Vec<String>,
    library_path: String,
) -> AppResult<Vec<Book>> {
    let log = logger(&app);
    let mtp_state = crate::mtp::MtpState::clone(&mtp_state);
    blocking(move || {
        // Before anything is copied from a device.
        files::ensure_library_available(Path::new(&library_path))?;
        // `_copies` keeps the temp copies alive until the import is done.
        let (local, _copies) = crate::commands::device::mtp::fetch_for_import(&mtp_state, &paths)?;
        import::import_files(Path::new(&library_path), &local, &log)
    })
    .await
}

/// Download an ebook from a URL directly into the library.
/// The file is streamed to a temp file in `.librarium/` (same volume as the
/// library), checked, then moved into place using the same collision and
/// re-import handling as `import_books` ([`import::download_to_temp`],
/// [`import::import_download`]).
#[tauri::command]
pub async fn download_and_import(
    app: AppHandle,
    url: String,
    filename: String,
    library_path: String,
) -> AppResult<Book> {
    let log = logger(&app);
    let lib = PathBuf::from(&library_path);
    // Before anything is created in it (the download's temp folder).
    let check = lib.clone();
    blocking(move || files::ensure_library_available(&check)).await?;
    let name = files::library_file_name(&filename);
    log(format!("[download_and_import] start — filename={}", name));
    let ext = Path::new(&name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    if !scan::is_supported_extension(ext) {
        return Err(AppError::InvalidInput(format!(
            "Unsupported file type: {}",
            filename
        )));
    }

    let temp = import::download_to_temp(&lib, &url, &log).await?;
    blocking(move || import::import_download(&lib, temp, &name, &log)).await
}

// ── Editing books ─────────────────────────────────────────────────────────────

/// Rename a book's file to `new_filename` (a bare file name). Keeps the
/// book's id, metadata and date added ([`edit::rename_book_file`]).
#[tauri::command]
pub async fn rename_book(
    app: AppHandle,
    library_path: String,
    book_id: String,
    new_filename: String,
) -> AppResult<Book> {
    let log = logger(&app);
    blocking(move || {
        edit::rename_book_file(Path::new(&library_path), &book_id, &new_filename, &log)
    })
    .await
}

/// Replace the metadata of one book (the single-book editor;
/// [`edit::update_book_metadata`]).
#[tauri::command]
pub async fn update_metadata(
    app: AppHandle,
    library_path: String,
    book_id: String,
    metadata: BookMetadata,
) -> AppResult<Book> {
    let log = logger(&app);
    blocking(move || edit::update_book_metadata(Path::new(&library_path), &book_id, metadata, &log))
        .await
}

/// Apply one metadata patch to several books, and optionally give each its
/// own volume number (`series_indexes`: book id → volume, for numbering a
/// series in the order the user chose) ([`edit::batch_update`]).
#[tauri::command]
pub async fn batch_update_metadata(
    app: AppHandle,
    library_path: String,
    book_ids: Vec<String>,
    patch: BookMetadataPatch,
    series_indexes: Option<HashMap<String, f32>>,
) -> AppResult<BatchUpdateResult> {
    let log = logger(&app);
    let series_indexes = series_indexes.unwrap_or_default();
    blocking(move || {
        edit::batch_update(
            Path::new(&library_path),
            &book_ids,
            &patch,
            &series_indexes,
            &log,
        )
    })
    .await
}

/// Delete books: their files go to the trash (recoverable), their index
/// entries and cached covers are removed ([`edit::delete_books_by_id`]).
#[tauri::command]
pub async fn delete_books(
    app: AppHandle,
    library_path: String,
    book_ids: Vec<String>,
) -> AppResult<DeleteResult> {
    let log = logger(&app);
    blocking(move || {
        let lib = PathBuf::from(&library_path);
        edit::delete_books_by_id(
            &lib,
            &book_ids,
            &|path| fsutil::move_to_trash(path, &lib),
            &log,
        )
    })
    .await
}

/// Remove books from the library without touching their files: the index
/// forgets them and scans skip them from now on; importing a file again
/// brings it back ([`edit::forget_books_by_id`]).
#[tauri::command]
pub async fn forget_books(
    app: AppHandle,
    library_path: String,
    book_ids: Vec<String>,
) -> AppResult<ForgetResult> {
    let log = logger(&app);
    blocking(move || edit::forget_books_by_id(Path::new(&library_path), &book_ids, &log)).await
}

/// Star or unstar books: favorites are listed first ([`edit::set_favorite`]).
/// Only the library index changes. Returns the books as they are now (an
/// id that isn't in the library is left out).
#[tauri::command]
pub async fn set_favorite(
    app: AppHandle,
    library_path: String,
    book_ids: Vec<String>,
    favorite: bool,
) -> AppResult<Vec<Book>> {
    let log = logger(&app);
    blocking(move || edit::set_favorite(Path::new(&library_path), &book_ids, favorite, &log)).await
}

/// Mark the library as open by this instance (`.librarium/lock.json`) and
/// report another Librarium — on this computer or another one — that used it
/// in the last few minutes ([`lock::status`]). Fails only when the library
/// folder isn't there.
#[tauri::command]
pub async fn library_lock_status(library_path: String) -> AppResult<lock::LockStatus> {
    blocking(move || {
        let library = PathBuf::from(&library_path);
        files::ensure_library_available(&library)?;
        Ok(lock::status(&library))
    })
    .await
}

/// Remove all sideloaded data for a book (edited metadata, cached cover) and
/// re-read its metadata from the file, as if it had just been added
/// ([`edit::reset_book`]).
#[tauri::command]
pub async fn reset_book_data(
    app: AppHandle,
    library_path: String,
    book_id: String,
) -> AppResult<Book> {
    let log = logger(&app);
    blocking(move || edit::reset_book(Path::new(&library_path), &book_id, &log)).await
}

// ── Covers ────────────────────────────────────────────────────────────────────

/// Let the webview load `library`'s covers through the asset protocol
/// (`convertFileSrc`). Only `<library>/.covers` (and below) joins the scope,
/// never the books themselves; the scope starts empty (`tauri.conf.json`).
/// Each folder is added once per run. Best effort: a failure only costs the
/// covers, and is logged.
fn allow_covers_dir(app: &AppHandle, library: &Path) {
    static ALLOWED: Mutex<Option<HashSet<PathBuf>>> = Mutex::new(None);
    let dir = covers::covers_dir(library);
    let mut allowed = ALLOWED.lock().unwrap_or_else(|e| e.into_inner());
    if allowed.get_or_insert_with(HashSet::new).contains(&dir) {
        return;
    }
    match app.asset_protocol_scope().allow_directory(&dir, true) {
        Ok(()) => {
            allowed.get_or_insert_with(HashSet::new).insert(dir);
        }
        Err(e) => log(
            app,
            format!(
                "[covers] WARNING — covers in {} can't be shown: {e}",
                dir.display()
            ),
        ),
    }
}

/// The cover of a book for the webview, made on demand
/// ([`covers::ensure_cover`]): the cached thumbnail; or a thumbnail made from
/// the cached full-size cover; or `null` when the book is known to have no
/// cover; or else the cover found inside the book file, cached with its
/// thumbnail. The frontend calls this lazily, for cards that become visible.
#[tauri::command]
pub async fn ensure_cover(
    app: AppHandle,
    library_path: String,
    book_id: String,
) -> AppResult<Option<CoverInfo>> {
    let log = logger(&app);
    let lib = PathBuf::from(&library_path);
    let info = blocking(move || covers::ensure_cover(&lib, &book_id, &log)).await?;
    allow_covers_dir(&app, Path::new(&library_path));
    Ok(info)
}

/// Download a cover image from a URL, save it to the covers cache (with its
/// thumbnail), and embed it into the source EPUB file so it survives
/// transfer/conversion ([`covers::persist_cover`]). Returns the new cover so
/// the frontend can update immediately.
#[tauri::command]
pub async fn save_cover_from_url(
    app: AppHandle,
    library_path: String,
    book_id: String,
    url: String,
) -> AppResult<CoverInfo> {
    let log = logger(&app);
    let lib = PathBuf::from(&library_path);
    // The host only: query strings can carry keys, and this line reaches the log file.
    log(format!(
        "[save_cover_from_url] downloading cover for id={} from {}",
        book_id,
        net::host_of(&url)
    ));

    // Fail fast on an unknown book, before downloading anything.
    let (check_lib, check_id, check_log) = (lib.clone(), book_id.clone(), logger(&app));
    blocking(move || find_book(&check_lib, &check_id, &check_log).map(|_| ())).await?;

    let bytes = net::get_bytes(&url, net::COVER_LIMITS).await?;
    log(format!(
        "[save_cover_from_url] downloaded {} bytes",
        bytes.len()
    ));
    covers::check_downloaded_cover(&bytes)?;
    let info = blocking(move || covers::persist_cover(&lib, &book_id, &bytes, &log)).await?;
    allow_covers_dir(&app, Path::new(&library_path));
    Ok(info)
}

/// Save a user-supplied cover image (from file picker). The image is provided as
/// a base64-encoded string (a `data:` URL prefix is tolerated). Saves the
/// cover and its thumbnail, and embeds it into the EPUB if applicable
/// ([`covers::persist_cover`]). Returns the new cover.
#[tauri::command]
pub async fn set_book_cover(
    app: AppHandle,
    library_path: String,
    book_id: String,
    image_base64: String,
) -> AppResult<CoverInfo> {
    let log = logger(&app);
    log(format!("[set_book_cover] id={}", book_id));
    let lib = PathBuf::from(&library_path);
    let info = blocking(move || {
        let bytes = covers::decode_image_base64(&image_base64)?;
        covers::persist_cover(&lib, &book_id, &bytes, &log)
    })
    .await?;
    allow_covers_dir(&app, Path::new(&library_path));
    Ok(info)
}

/// Remove a book's cover: from the EPUB file (if any) and from the cache
/// (full cover and thumbnail), and remember that the book has none so an
/// image the file still holds isn't extracted again ([`covers::delete_cover`]).
#[tauri::command]
pub async fn delete_book_cover(
    app: AppHandle,
    library_path: String,
    book_id: String,
) -> AppResult<()> {
    let log = logger(&app);
    blocking(move || covers::delete_cover(Path::new(&library_path), &book_id, &log)).await
}

// ── Conversion ────────────────────────────────────────────────────────────────

/// Convert a MOBI/AZW/AZW3 book to EPUB in the library. The EPUB keeps the
/// book's id, metadata and date added; the original goes to the trash
/// ([`convert::convert_book_to_epub`]).
#[tauri::command]
pub async fn convert_to_epub(
    app: AppHandle,
    library_path: String,
    book_id: String,
) -> AppResult<Book> {
    let log = logger(&app);
    blocking(move || {
        let lib = PathBuf::from(&library_path);
        convert::convert_book_to_epub(
            &lib,
            &book_id,
            &|path| fsutil::move_to_trash(path, &lib),
            &log,
        )
    })
    .await
}

/// Convert book `book_id` to `target_format` (`epub` or `azw3`) and write it
/// to `dest_path`, a path the user picked in a save dialog. The book is
/// looked up by id; the export is validated, written next to `dest_path`
/// and renamed over it; `dest_path` may not be the book itself or another
/// book of the library ([`convert::export_book_by_id`]).
#[tauri::command]
pub async fn export_book(
    app: AppHandle,
    library_path: String,
    book_id: String,
    target_format: String,
    dest_path: String,
) -> AppResult<()> {
    let log = logger(&app);
    let format = OutputFormat::parse(&target_format)?;
    blocking(move || {
        convert::export_book_by_id(
            Path::new(&library_path),
            &book_id,
            format,
            Path::new(&dest_path),
            &log,
        )
    })
    .await
}

// ── EPUB inspector ────────────────────────────────────────────────────────────

/// Inspect the internal structure of an EPUB file and return a JSON-friendly
/// summary: OPF path, cover declaration, manifest items, and spine order.
/// Used by the dev-mode EPUB inspector in the UI ([`inspect::inspect_epub_file`]).
/// Debug builds only: it reads any `.epub` path the webview names, which a
/// release build has no reason to allow.
#[cfg(debug_assertions)]
#[tauri::command]
pub async fn inspect_epub(path: String) -> AppResult<serde_json::Value> {
    blocking(move || Ok(inspect::inspect_epub_file(&path)?)).await
}

// ── Users ─────────────────────────────────────────────────────────────────────

/// List user directories (non-hidden subdirectories of the library root;
/// [`users::list_user_dirs`]). A root that doesn't exist is
/// `LIBRARY_UNAVAILABLE`; it is never created here.
#[tauri::command]
pub async fn list_users(library_path: String) -> AppResult<Vec<String>> {
    blocking(move || users::list_user_dirs(Path::new(&library_path))).await
}

/// Create a new user directory, with a name made safe for every filesystem
/// ([`users::create_user_dir`]). The library root is only created when
/// `create_root` is true (first run, default library folder).
#[tauri::command]
pub async fn create_user(
    library_path: String,
    name: String,
    create_root: Option<bool>,
) -> AppResult<String> {
    blocking(move || {
        users::create_user_dir(
            Path::new(&library_path),
            &name,
            create_root.unwrap_or(false),
        )
    })
    .await
}

// ── Utilities ─────────────────────────────────────────────────────────────────

/// Detect the language of a text sample using `whatlang`.
/// Returns an ISO 639-1 (2-letter) code, or `None` when confidence is below 75%
/// ([`metadata::detect_text_language`]). Async (on the blocking pool) so a
/// long description never stalls the main thread, where Tauri runs
/// synchronous commands.
#[tauri::command]
pub async fn detect_language(text: String) -> Option<String> {
    blocking(move || Ok(metadata::detect_text_language(&text)))
        .await
        .ok()
        .flatten()
}

/// Show a file of the library in the platform's file manager. `path` must
/// be inside `library_path` ([`os::reveal_in_file_manager`]).
#[tauri::command]
pub async fn reveal_in_explorer(library_path: String, path: String) -> AppResult<()> {
    blocking(move || os::reveal_in_file_manager(Path::new(&library_path), Path::new(&path))).await
}

/// Test whether a path is an accessible directory, with a 3-second timeout.
/// Used to validate custom library paths that may live on a NAS
/// ([`os::is_accessible_dir`]).
#[tauri::command]
pub async fn check_path_accessible(path: String) -> bool {
    os::is_accessible_dir(path).await
}

/// Show the folder holding the log file (`Librarium.log`) in the file
/// manager, the file selected when it exists — what a user attaches to a
/// bug report.
#[tauri::command]
pub async fn open_log_folder(app: AppHandle) -> AppResult<()> {
    let dir = crate::logging::log_dir(&app)?;
    blocking(move || {
        std::fs::create_dir_all(&dir)
            .map_err(|e| AppError::io("Could not create the log folder", &e))?;
        let file = dir.join(format!("{}.log", crate::logging::LOG_FILE_NAME));
        os::reveal(if file.is_file() { &file } else { &dir })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn label_of<F>(_: &F) -> &'static str {
        command_label::<F>()
    }

    #[test]
    fn blocking_errors_are_labelled_with_their_command() {
        // As in a command: a closure inside the async fn's body.
        fn scan_library() -> &'static str {
            let outer = || {
                let inner = || -> Result<(), String> { Ok(()) };
                label_of(&inner)
            };
            outer()
        }
        assert_eq!(scan_library(), "scan_library");
    }

    #[test]
    fn bounded_work_never_holds_up_the_caller_for_long() {
        use std::time::{Duration, Instant};
        assert!(run_bounded(Duration::from_secs(5), || {}));
        // A share that stopped answering.
        let start = Instant::now();
        assert!(!run_bounded(Duration::from_millis(100), || {
            std::thread::sleep(Duration::from_secs(3))
        }));
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[tokio::test]
    async fn a_panic_in_blocking_work_is_an_internal_error() {
        let err = blocking(|| -> AppResult<()> { panic!("boom") })
            .await
            .unwrap_err();
        assert_eq!(err.code(), "INTERNAL");
        assert!(err.to_string().contains("boom"), "{err}");
        let err = blocking(|| -> AppResult<()> { Err(AppError::NotFound("gone".into())) })
            .await
            .unwrap_err();
        assert_eq!(err.code(), "NOT_FOUND");
    }
}
