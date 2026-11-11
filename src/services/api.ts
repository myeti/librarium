/**
 * The backend, as typed functions: one per Tauri command, named after it
 * (`scan_library` → `api.scanLibrary`). This is the only module that calls
 * `invoke`; stores and components call these instead, so every command's
 * arguments and result are checked by TypeScript.
 *
 * The contract (arguments, results, errors, side effects) is documented in
 * docs/IPC.md; keep both in sync. Argument names are the camelCase spelling
 * of the Rust parameters (Tauri converts them). `scripts/check-ipc.mjs`
 * cross-checks each invoke call below against the Rust signatures, so keep
 * their literal style: the command name as a string literal, the arguments
 * as an object literal without nested braces.
 *
 * Errors are not handled here: a failed command rejects with the backend's
 * `AppError`, `{ code, message }` (docs/IPC.md, "Errors"). Callers show it
 * with `errorMessage(e)` and branch on `errorCode(e)` (`@/utils/errors`),
 * which also read the plain strings older backends sent. Per-item failures
 * inside a result (`Failure.error`…) are plain strings.
 *
 * Arrays are copied before they are sent: callers often pass reactive arrays
 * (Vue proxies), and a plain copy is what the IPC layer expects.
 */
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import type {
    BatchUpdateResult, Book, BookMetadata, BookMetadataPatch,
    ConnectedDevice, CoverInfo, DeleteResult, DeviceBook, EpubStructure, ExportFormat,
    ForgetResult, HttpGetResponse, LockStatus, MtpStatus, OpdsStatus, TransferRequest,
} from '@/types'
import type {
    BackupExported, BackupManifest, BackupProgress, BackupRestored,
    CalibreBook, CalibreImportProgress, CalibreImportResult, ReaderSource,
} from '@/types'

/** What every command here rejects with (re-exported for callers that type their catch). */
export type { AppError } from '@/utils/errors'

// ── Library ──────────────────────────────────────────────────────────────────

/** Lists (and indexes) the books of the library at `path`. Rejects with `LIBRARY_UNAVAILABLE` if it doesn't exist. */
export function scanLibrary(path: string): Promise<Book[]> {
    return invoke<Book[]>('scan_library', { path })
}

/** Copies files into the library; a file it already holds comes back as is. */
export function importBooks(paths: string[], libraryPath: string): Promise<Book[]> {
    const files = [...paths]
    return invoke<Book[]>('import_books', { paths: files, libraryPath })
}

/** "Remove from library (keep file)": the books leave the index; their files are not touched. */
export function forgetBooks(libraryPath: string, bookIds: string[]): Promise<ForgetResult> {
    const ids = [...bookIds]
    return invoke<ForgetResult>('forget_books', { libraryPath, bookIds: ids })
}

/** Stars (or unstars) books; resolves with them as saved (ids not in the library are left out). */
export function setFavorite(libraryPath: string, bookIds: string[], favorite: boolean): Promise<Book[]> {
    const ids = [...bookIds]
    return invoke<Book[]>('set_favorite', { libraryPath, bookIds: ids, favorite })
}

/** Marks the library as open by this instance and reports another recent one (advisory). */
export function libraryLockStatus(libraryPath: string): Promise<LockStatus> {
    return invoke<LockStatus>('library_lock_status', { libraryPath })
}

/** Replaces one book's metadata (an absent optional field is cleared) and writes EPUBs. */
export function updateMetadata(libraryPath: string, bookId: string, metadata: BookMetadata): Promise<Book> {
    return invoke<Book>('update_metadata', { libraryPath, bookId, metadata })
}

/** Merges `patch` into each book's own metadata; per-book failures are listed, not thrown. */
/**
 * Applies `patch` to every book; `seriesIndexes` (book id → volume) then
 * gives books their own volume number (a series numbered in one edit).
 */
export function batchUpdateMetadata(
    libraryPath: string,
    bookIds: string[],
    patch: BookMetadataPatch,
    seriesIndexes?: Readonly<Record<string, number>>,
): Promise<BatchUpdateResult> {
    const ids = [...bookIds]
    const args: Record<string, unknown> = { libraryPath, bookIds: ids, patch }
    if (seriesIndexes && Object.keys(seriesIndexes).length > 0) { args.seriesIndexes = { ...seriesIndexes } }
    return invoke<BatchUpdateResult>('batch_update_metadata', args)
}

/** Renames the file (sanitised, extension kept); id, metadata and date added are kept. */
export function renameBook(libraryPath: string, bookId: string, newFilename: string): Promise<Book> {
    return invoke<Book>('rename_book', { libraryPath, bookId, newFilename })
}

/** Moves the files to the trash (system Trash, or `<library>/.librarium/trash/…`). */
export function deleteBooks(libraryPath: string, bookIds: string[]): Promise<DeleteResult> {
    const ids = [...bookIds]
    return invoke<DeleteResult>('delete_books', { libraryPath, bookIds: ids })
}

/** Re-reads metadata from the file, keeping the id and date added; drops the cached cover. */
export function resetBookData(libraryPath: string, bookId: string): Promise<Book> {
    return invoke<Book>('reset_book_data', { libraryPath, bookId })
}

/** MOBI/AZW/AZW3 → EPUB; the original goes to the trash once the EPUB is validated. */
export function convertToEpub(libraryPath: string, bookId: string): Promise<Book> {
    return invoke<Book>('convert_to_epub', { libraryPath, bookId })
}

/** The cover to show, made on demand; `null` when the book has none. */
export function ensureCover(libraryPath: string, bookId: string): Promise<CoverInfo | null> {
    return invoke<CoverInfo | null>('ensure_cover', { libraryPath, bookId })
}

/** Downloads an image (JPEG/PNG/WebP only) and makes it the book's cover. */
export function saveCoverFromUrl(libraryPath: string, bookId: string, url: string): Promise<CoverInfo> {
    return invoke<CoverInfo>('save_cover_from_url', { libraryPath, bookId, url })
}

/** Makes an image (base64 or `data:` URL; JPEG/PNG/WebP) the book's cover. */
export function setBookCover(libraryPath: string, bookId: string, imageBase64: string): Promise<CoverInfo> {
    return invoke<CoverInfo>('set_book_cover', { libraryPath, bookId, imageBase64 })
}

/** Removes the cover (from the EPUB too) and remembers that the book has none. */
export function deleteBookCover(libraryPath: string, bookId: string): Promise<void> {
    return invoke<void>('delete_book_cover', { libraryPath, bookId })
}

/** Downloads a book (http/https, checked against its extension) into the library. */
export function downloadAndImport(url: string, filename: string, libraryPath: string): Promise<Book> {
    return invoke<Book>('download_and_import', { url, filename, libraryPath })
}

/**
 * Writes a converted copy of a library book to `destPath` (chosen in a save
 * dialog): EPUB or AZW3, from an EPUB, MOBI, AZW or AZW3 book. A failed
 * export leaves `destPath` as it was.
 */
export function exportBook(libraryPath: string, bookId: string, targetFormat: ExportFormat, destPath: string): Promise<void> {
    return invoke<void>('export_book', { libraryPath, bookId, targetFormat, destPath })
}

/**
 * The structure of an EPUB (manifest, spine, cover declaration): a developer
 * tool, registered in debug builds only (release builds reject the command).
 */
export function inspectEpub(path: string): Promise<EpubStructure> {
    return invoke<EpubStructure>('inspect_epub', { path })
}

/** The language of a short text (ISO 639-1), or `null` when it can't tell. */
export function detectLanguage(text: string): Promise<string | null> {
    return invoke<string | null>('detect_language', { text })
}

// ── Reader ───────────────────────────────────────────────────────────────────

/**
 * The file the reader displays for a book: the book itself (EPUB, PDF) or
 * its cached EPUB conversion (MOBI/AZW/AZW3; the first open converts, which
 * takes a while). Load it with `convertFileSrc(source.path)`; nothing else
 * travels over IPC. Rejects with `INVALID_INPUT` for CBZ or a file that
 * isn't what its format says, `CONVERSION` if a Kindle book can't be converted.
 */
export function openForReading(libraryPath: string, bookId: string): Promise<ReaderSource> {
    return invoke<ReaderSource>('open_for_reading', { libraryPath, bookId })
}

// ── Calibre ──────────────────────────────────────────────────────────────────

/** The Calibre library found on this computer (the one Calibre's settings name, else a usual folder), or `null`. */
export function detectCalibreLibrary(): Promise<string | null> {
    return invoke<string | null>('detect_calibre_library')
}

/** The books of a Calibre library (read from its `metadata.db`), sorted by title. */
export function listCalibreBooks(calibrePath: string): Promise<CalibreBook[]> {
    return invoke<CalibreBook[]>('list_calibre_books', { calibrePath })
}

/**
 * Imports Calibre books into the library and saves them in its index,
 * skipping those it already holds. Reports progress through
 * `calibre-import-progress` ({@link onCalibreImportProgress}); stops between
 * two books after {@link cancelCalibreImport}, resolving with
 * `cancelled: true`. Rejects with `CONFLICT` while another import runs.
 */
export function importFromCalibre(calibrePath: string, bookIds: number[], libraryPath: string): Promise<CalibreImportResult> {
    const ids = [...bookIds]
    return invoke<CalibreImportResult>('import_from_calibre', { calibrePath, bookIds: ids, libraryPath })
}

/** Asks the running Calibre import to stop before its next book; whether one was running. */
export function cancelCalibreImport(): Promise<boolean> {
    return invoke<boolean>('cancel_calibre_import')
}

/** Calls `handler` with each `calibre-import-progress` event; resolves with the function that stops listening. */
export function onCalibreImportProgress(handler: (progress: CalibreImportProgress) => void): Promise<UnlistenFn> {
    return listen<CalibreImportProgress>('calibre-import-progress', event => handler(event.payload))
}

// ── Backup ───────────────────────────────────────────────────────────────────

/**
 * Zips every user of `libraryRoot` (books and Librarium's data) into a new
 * file in `destDir`. `libraryRoot` must be the settings' library folder.
 * Progress: `backup-progress` ({@link onBackupProgress}); stops after
 * {@link cancelBackup}, resolving with `cancelled: true` and no file.
 */
export function exportBackup(libraryRoot: string, destDir: string): Promise<BackupExported> {
    return invoke<BackupExported>('export_backup', { libraryRoot, destDir })
}

/** What the backup at `path` holds; rejects `INVALID_INPUT` when it isn't a Librarium backup. */
export function readBackup(path: string): Promise<BackupManifest> {
    return invoke<BackupManifest>('read_backup', { path })
}

/**
 * Replaces every user of `libraryRoot` with the backup's (the current ones
 * are erased). Extracted aside first: a cancel, a damaged backup or a lack
 * of space leaves the library as it was.
 */
export function restoreBackup(libraryRoot: string, path: string): Promise<BackupRestored> {
    return invoke<BackupRestored>('restore_backup', { libraryRoot, path })
}

/** Shows the last backup written in Finder/Explorer (the backend only ever reveals that file). */
export function revealBackup(): Promise<void> {
    return invoke<void>('reveal_backup')
}

/** Asks the running export or restore to stop; whether one was running. */
export function cancelBackup(): Promise<boolean> {
    return invoke<boolean>('cancel_backup')
}

/** Calls `handler` with each `backup-progress` event; resolves with the function that stops listening. */
export function onBackupProgress(handler: (progress: BackupProgress) => void): Promise<UnlistenFn> {
    return listen<BackupProgress>('backup-progress', event => handler(event.payload))
}

// ── Users and the file manager ───────────────────────────────────────────────

/** The user folders of the library root. Never creates the root. */
export function listUsers(libraryPath: string): Promise<string[]> {
    return invoke<string[]>('list_users', { libraryPath })
}

/** Creates a user folder; resolves with its (sanitised) name. */
export function createUser(libraryPath: string, name: string, createRoot: boolean): Promise<string> {
    return invoke<string>('create_user', { libraryPath, name, createRoot })
}

/** Shows a file of the library in Finder/Explorer (Linux: opens its folder). */
export function revealInExplorer(libraryPath: string, path: string): Promise<void> {
    return invoke<void>('reveal_in_explorer', { libraryPath, path })
}

/** Whether `path` is a reachable folder (`false` after 3 s: sleeping NAS). */
export function checkPathAccessible(path: string): Promise<boolean> {
    return invoke<boolean>('check_path_accessible', { path })
}

/** Shows the log folder in Finder/Explorer with `Librarium.log` selected (for "Report a problem"). */
export function openLogFolder(): Promise<void> {
    return invoke<void>('open_log_folder')
}

// ── Settings ─────────────────────────────────────────────────────────────────

/** The settings file (`{}` when it is damaged). */
export function getConfig(): Promise<Record<string, unknown>> {
    return invoke<Record<string, unknown>>('get_config')
}

/** Merges `updates` into the settings file, atomically. */
export function setConfig(updates: Record<string, unknown>): Promise<void> {
    return invoke<void>('set_config', { updates })
}

// ── Menu bar (macOS) ─────────────────────────────────────────────────────────

/** Rebuilds the native menu bar in `lang` (`en`, `fr`); does nothing off macOS. */
export function setMenuLanguage(lang: string): Promise<void> {
    return invoke<void>('set_menu_language', { lang })
}

/**
 * Calls `handler` with the command id of each menu item clicked (event
 * `menu-command`, e.g. `library.import`); resolves with the function that
 * stops listening.
 */
export function onMenuCommand(handler: (id: string) => void): Promise<UnlistenFn> {
    return listen<string>('menu-command', event => handler(event.payload))
}

// ── Devices ──────────────────────────────────────────────────────────────────

/**
 * Every connected e-reader, each with a stable `id` and its current free
 * space. Polled; a new device appearing also gets its leftover `.part`
 * files from interrupted transfers cleaned.
 */
export function detectDevices(): Promise<ConnectedDevice[]> {
    return invoke<ConnectedDevice[]>('detect_devices')
}

/** The first connected e-reader, or `null` (single-device callers). */
export function detectDevice(): Promise<ConnectedDevice | null> {
    return invoke<ConnectedDevice | null>('detect_device')
}

/**
 * The MTP Kindles plugged in (2024+, experimental), whatever their state,
 * and whether MTP support is on. Cheap: polled with `detectDevices`, to say
 * why a Kindle isn't usable (MTP off, held by another app…). The ready ones
 * are also in `detectDevices`, with `connection: 'mtp'`.
 */
export function getMtpStatus(): Promise<MtpStatus> {
    return invoke<MtpStatus>('get_mtp_status')
}

/** The ebook file names on the device (Kindle `documents/`, the whole Kobo volume). */
export function scanDeviceLibrary(devicePath: string): Promise<string[]> {
    return invoke<string[]>('scan_device_library', { devicePath })
}

/**
 * The books on the device: title and author (Kobo: as the Kobo shows them),
 * `libraryId` for the books Librarium sent, Kobo progress and read status.
 * Rejects (`DEVICE`) only for an MTP Kindle that can't be listed; listing
 * one is slow (a request per book) and waits for a transfer in progress.
 */
export function listDeviceBooks(devicePath: string): Promise<DeviceBook[]> {
    return invoke<DeviceBook[]>('list_device_books', { devicePath })
}

/**
 * Queues transfers to the device. Progress arrives as `transfer-progress`
 * events; every job ends with exactly one `done`, `error` or `cancelled`.
 */
export function sendBooks(jobs: TransferRequest[]): Promise<void> {
    const requests = [...jobs]
    return invoke<void>('send_books', { jobs: requests })
}

/** Asks the backend to stop a job; it confirms with a `cancelled` event (or `done` if too late). */
export function cancelTransfer(jobId: string): Promise<void> {
    return invoke<void>('cancel_transfer', { jobId })
}

/**
 * Deletes an ebook from the device, for good (e-readers have no Trash),
 * with its Kindle `.sdr` reading-state folder and its manifest entry.
 */
export function deleteDeviceBook(path: string): Promise<void> {
    return invoke<void>('delete_device_book', { path })
}

/**
 * Safely ejects the e-reader at `mountPath`. Rejects with `CONFLICT` while
 * books are being sent to it, and `DEVICE` when ejecting failed (the
 * message says why) or isn't supported (Windows: the message says to use
 * the system tray).
 */
export function ejectDevice(mountPath: string): Promise<void> {
    return invoke<void>('eject_device', { mountPath })
}

/**
 * Writes the Kindle home-screen covers missing for books Librarium sent to
 * the device at `devicePath` (a Kindle only shows a cover for them if that
 * file exists), from `libraryPath`'s covers. Resolves to how many were
 * written; 0 on a Kobo or an MTP Kindle.
 */
export function repairDeviceCovers(libraryPath: string, devicePath: string): Promise<number> {
    return invoke<number>('repair_device_covers', { libraryPath, devicePath })
}

/** Reading progress (0–100) by file name without its last extension. */
export function getDeviceReadingProgress(devicePath: string, deviceType: string): Promise<Record<string, number>> {
    return invoke<Record<string, number>>('get_device_reading_progress', { devicePath, deviceType })
}

// ── Network ──────────────────────────────────────────────────────────────────

/**
 * A GET request made by Rust, for hosts the webview's CSP doesn't allow.
 * Non-2xx statuses resolve; network errors, timeouts and oversized bodies
 * reject. Prefer `httpGet`/`httpGetJson` from `@/services/http`.
 */
export function httpGet(url: string, headers?: Record<string, string>, maxBytes?: number): Promise<HttpGetResponse> {
    return invoke<HttpGetResponse>('http_get', { url, headers, maxBytes })
}

// ── OPDS server ──────────────────────────────────────────────────────────────

/** (Re)starts the OPDS server; local-only unless `shareOnLan` (which needs a password). */
export function startOpds(port: number, libraryRoot: string, shareOnLan: boolean, password: string | null): Promise<OpdsStatus> {
    return invoke<OpdsStatus>('start_opds', { port, libraryRoot, shareOnLan, password })
}

/** Stops the OPDS server and waits until the port is released. */
export function stopOpds(): Promise<void> {
    return invoke<void>('stop_opds')
}

/** The OPDS server's state, or `null` when it was never started (or stopped). */
export function opdsStatus(): Promise<OpdsStatus | null> {
    return invoke<OpdsStatus | null>('opds_status')
}

/**
 * Tells the OPDS server the library changed, so readers see it at once (the
 * server also notices index and cover changes by itself within a second).
 */
export function invalidateOpdsCache(): Promise<void> {
    return invoke<void>('invalidate_opds_cache')
}

/**
 * SVG markup of a QR code of `url` (black on white, scalable). Show it as an
 * `<img>` data URL (`svgDataUrl`), never with `v-html`. Rejects empty text or
 * more than 512 bytes.
 */
export function opdsQrSvg(url: string): Promise<string> {
    return invoke<string>('opds_qr_svg', { url })
}
