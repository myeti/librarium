export type BookFormat = 'epub' | 'pdf' | 'mobi' | 'azw3' | 'cbz' | 'unknown'

export interface BookMetadata {
  /** Empty when unknown: display it with `displayTitle` (utils/bookDisplay). */
  title: string
  /** Empty when unknown: display it with `displayAuthor` (utils/bookDisplay). */
  author: string
  subtitle?: string
  series?: string
  seriesIndex?: number
  language?: string
  description?: string
  isbn?: string
  rating?: number
  categories?: string[]
  goodreadsUrl?: string
}

export interface Book {
  id: string
  path: string
  filename: string
  format: BookFormat
  sizeBytes: number
  dateAdded: string
  /** Cover file to show (absolute path, for `coverUrl()`); unset until known. */
  coverPath?: string
  /** Modification time of the cover file, a cache buster for `coverUrl()`. */
  coverVersion?: number
  /** Starred by the user (`set_favorite`): listed first. Absent when not. */
  favorite?: boolean
  metadata: BookMetadata
}

/** A cover file for the webview (`ensure_cover`, `set_book_cover`…; see docs/IPC.md). */
export interface CoverInfo {
  coverPath: string
  coverVersion: number
}

/** `progress`: reading progress on the connected e-reader (books without any last). */
export type SortField = 'title' | 'author' | 'series' | 'format' | 'dateAdded' | 'sizeBytes' | 'progress'

/** Where a book of the library screen is: only in the library, in both, only on the e-reader (services/shelf.ts). */
export type Presence = 'libraryOnly' | 'both' | 'deviceOnly'
export type SortOrder = 'asc' | 'desc'

export interface SortConfig {
  field: SortField
  order: SortOrder
}

export interface FilterConfig {
  search: string
  formats: BookFormat[]
  authors: string[]
  series: string[]
  languages: string[]
  categories: string[]
  /** Where the book is (library only, both, device only); empty: anywhere. */
  presence: Presence[]
}

/** `detect_devices` / `detect_device` result (the Rust `KindleDevice`, which also describes Kobos). */
export interface ConnectedDevice {
  /**
   * Stable per device (a UUID kept on the device in `.librarium/device.json`,
   * or `"<type>:<mountPath>"` for a device Librarium can't write to): another
   * id at the same `mountPath` means another device was plugged in.
   */
  id: string
  name: string
  /** A folder (`/Volumes/Kindle`, `E:\`), or `mtp://<port>` for an MTP Kindle (see `connection`). */
  mountPath: string
  deviceType: 'kindle' | 'kobo'
  /**
   * `mtp`: a 2024+ Kindle reached over MTP (experimental). Its `mountPath`
   * and its books' paths are then `mtp://…` locators, not files: never show
   * them as paths or hand them to the file manager (see `utils/mtp.ts`).
   */
  connection: DeviceConnection
  preferredFormat: string
  model?: string
  freeSpaceBytes?: number
  totalSpaceBytes?: number
}

/** How an e-reader is connected: as a USB drive, or over MTP (2024+ Kindles). */
export type DeviceConnection = 'massStorage' | 'mtp'

/**
 * What an MTP Kindle is doing (`get_mtp_status`; docs/IPC.md, "MTP Kindles"):
 * `ready` (it is in `detect_devices` too), `connecting`, `mtpDisabled` (the
 * setting is off), `claimFailed` (another app holds it, or no permission),
 * `busy`, `error`, `ejected` (released: safe to unplug).
 */
export type MtpDeviceState = 'ready' | 'connecting' | 'mtpDisabled' | 'claimFailed' | 'busy' | 'error' | 'ejected'

/** An MTP Kindle plugged in, whatever its state (`get_mtp_status`). */
export interface MtpDevice {
  /** `mtp://<port>`: the `ConnectedDevice.mountPath` once ready. */
  mountPath: string
  /** The model once connected, else the USB product name ("Kindle"). */
  name: string
  state: MtpDeviceState
  /** What to do, or what went wrong (English, from the backend); absent when ready. */
  message?: string
  /** `ConnectedDevice.id`, when ready. */
  deviceId?: string
}

/** `get_mtp_status` result. */
export interface MtpStatus {
  /** The `mtpEnabled` setting, as the backend applies it. */
  enabled: boolean
  devices: MtpDevice[]
}

/**
 * `cancelled` is only ever set by the backend (through a `transfer-progress`
 * event): cancelling a job asks Rust to stop it, and the job is shown as
 * cancelled once Rust confirms it actually stopped (see docs/IPC.md).
 */
export type TransferStatus = 'queued' | 'converting' | 'transferring' | 'done' | 'error' | 'cancelled'
export type TransferDirection = 'to-device' | 'from-device'

export interface TransferJob {
  id: string
  direction: TransferDirection
  book?: Book           // to-device jobs
  deviceTitle?: string  // from-device jobs
  status: TransferStatus
  progress: number
  errorMessage?: string
  needsConversion: boolean
  targetFormat: BookFormat
  /** Jobs sent together share a batch id, so the device is rescanned once per batch. */
  batchId?: string
  /** Set while a cancel request is in flight, so the UI can say "Cancelling…". */
  cancelRequested?: boolean
  /** `ConnectedDevice.mountPath` of the device the job sends to or reads from (eject waits for it). */
  devicePath?: string
}

/** A book file on the e-reader (`list_device_books`; docs/IPC.md). */
export interface DeviceBook {
  /** Absolute path; an `mtp://…` locator on an MTP Kindle (not a file: never shown). */
  path: string
  filename: string
  /**
   * In order: as the Kobo shows it; the file's own metadata (an EPUB's, a
   * MOBI/AZW3's header); what Librarium recorded when it sent the book;
   * the file name ("Title - Author", split at the last " - "). On an MTP
   * Kindle files aren't opened: the record, else the file name.
   */
  title: string
  author: string
  /**
   * The library `Book.id` this file is (from the device manifest, or the id
   * Librarium embeds in the copies it sends). Absent for books Librarium
   * didn't send. It may be another user's library book.
   */
  libraryId?: string
  sizeBytes?: number
  /** Kobo only: progress the Kobo recorded, 0–100. */
  percentRead?: number
  /** Kobo only. */
  readStatus?: DeviceReadStatus
}

/** Kobo `ReadStatus`. */
export type DeviceReadStatus = 'unread' | 'reading' | 'finished'

// ── IPC result types (see docs/IPC.md) ───────────────────────────────────────

/** Batch edit: every field optional; an absent field is left unchanged on each book. */
export type BookMetadataPatch = Partial<BookMetadata>

/** One book a batch command could not process, with the backend's reason. */
export interface Failure {
  id: string
  error: string
}

/** `batch_update_metadata` result. */
export interface BatchUpdateResult {
  books: Book[]
  failures: Failure[]
}

/** `delete_books` result. */
export interface DeleteResult {
  /** Ids of the books whose files were moved to a trash. */
  deleted: string[]
  failures: Failure[]
  /**
   * Set when the volume has no system Trash (typical on SMB/NAS shares): the
   * files were moved to this folder (`<library>/.librarium/trash/<timestamp>/`)
   * instead, so the user knows where to find them.
   */
  libraryTrash: string | null
}

/** `forget_books` result ("Remove from library": the files are kept). */
export interface ForgetResult {
  /** Ids of the books removed from the library. */
  forgotten: string[]
  failures: Failure[]
}

/** `library_lock_status` result. */
export interface LockStatus {
  /** Another Librarium (this computer or another) used the library in the last 5 minutes. */
  otherInstance: { host: string, updatedAt: string } | null
}

/** `start_opds` / `opds_status` result. */
export interface OpdsStatus {
  running: boolean
  port: number
  localUrl: string
  /** Only non-empty when the server was started with `shareOnLan`. */
  lanUrls: string[]
}

/** `http_get` result. Non-2xx statuses resolve; only network errors reject. */
export interface HttpGetResponse {
  status: number
  contentType: string | null
  body: string
  finalUrl: string
}

// ── Device transfers (see docs/IPC.md, "Device commands") ───────────────────

/**
 * One job of `send_books`. TransferRequest is a nested struct, so Tauri
 * doesn't convert its field names like it does for command arguments: Rust
 * reads these snake_case names (and accepts camelCase aliases).
 */
export interface TransferRequest {
  /** Job id, echoed in `transfer-progress` events and used by `cancel_transfer`. */
  id: string
  /**
   * The library book (`Book.id`), sent with `library_path`: the backend reads
   * the file from the library index, embeds the id in the copy, records it
   * in the device manifest and replaces the copy it sent earlier. One
   * without the other fails the job.
   */
  book_id?: string
  /** The library folder holding `book_id` (the current user's). */
  library_path?: string
  /** Only used without `book_id`/`library_path`. */
  book_path: string
  needs_conversion: boolean
  /** Conversion target: 'epub' (Kobo) | 'azw3' (Kindle; 'mobi' also gives AZW3). */
  target_format: string
  /** `ConnectedDevice.mountPath`; must contain `documents/` or `.kobo/`. */
  device_path: string
  /** Under `documents/`; relative, no '..', each part sanitised. */
  subfolder?: string | null
}

/** `device-space` event payload: the device's space after a transfer job. */
export interface DeviceSpaceEvent {
  mountPath: string
  freeSpaceBytes?: number
  totalSpaceBytes?: number
}

/**
 * `transfer-progress` event payload. docs/IPC.md specifies camelCase; older
 * backends sent snake_case, so both spellings are read.
 */
export interface TransferProgressEvent {
  jobId?: string
  job_id?: string
  status: TransferStatus
  progress: number
  errorMessage?: string | null
  error_message?: string | null
}

/** What `export_book` can write. */
export type ExportFormat = 'epub' | 'azw3'

// ── Reader, EPUB inspector ───────────────────────────────────────────────────

/**
 * `open_for_reading` result: the file the reader displays. The webview
 * loads it itself, from `convertFileSrc(path)` (the asset protocol; the
 * backend allowed that one file).
 */
export interface ReaderSource {
  /** How to display `path`. */
  format: 'epub' | 'pdf'
  /** Absolute: the book itself, or its cached EPUB conversion (MOBI/AZW/AZW3). */
  path: string
  /** Size of that file. */
  sizeBytes: number
  /** `path` is a MOBI/AZW/AZW3 → EPUB conversion. */
  converted: boolean
}

/** `inspect_epub` result (developer tool). */
export interface EpubStructure {
  opfPath: string
  opfDir: string
  coverDeclaration: EpubCoverDeclaration | null
  manifest: EpubManifestItem[]
  spine: string[]
  zipEntries: { name: string, size: number }[]
}

export interface EpubManifestItem {
  id: string
  href: string
  mediaType: string
  properties: string | null
  zipPath: string
  exists: boolean
}

export interface EpubCoverDeclaration {
  method: 'epub3' | 'epub2'
  href: string
  mediaType: string
  itemId?: string
}

// ── Calibre import ───────────────────────────────────────────────────────────

/** `list_calibre_books` entry. */
export interface CalibreBook {
  /** Calibre's id, for `import_from_calibre`. */
  id: number
  /** '' when unknown. */
  title: string
  /** Calibre's order, joined with ' & '; '' when unknown. */
  author: string
  series: string | null
  /** Only with a series. */
  seriesIndex: number | null
  /** Lower case: 'epub', 'pdf', 'txt'… */
  availableFormats: string[]
  /** What would be imported; `null` when the library supports none of the formats. */
  bestFormat: string | null
  hasCover: boolean
}

/** Why `import_from_calibre` skipped a book. */
export type CalibreSkipReason = 'alreadyInLibrary' | 'unsupportedFormat' | 'noFile'

export interface CalibreSkippedBook {
  title: string
  author: string
  reason: CalibreSkipReason
  /** `alreadyInLibrary`: the file name of the library's copy. */
  existingFilename: string | null
}

/** `import_from_calibre` result. */
export interface CalibreImportResult {
  /** Saved in the library index, also when the import was cancelled or stopped. */
  imported: Book[]
  skipped: CalibreSkippedBook[]
  /** `reason`: a message for the user. A book whose record couldn't be read is titled `#<id>`. */
  failed: { title: string, reason: string }[]
  /** `cancel_calibre_import` stopped it. */
  cancelled: boolean
  /** Why it stopped before the end, when some books had already been imported. */
  error: string | null
  /** Requested books not looked at (cancelled or stopped). */
  remaining: number
}

/** Payload of the `calibre-import-progress` event. */
export interface CalibreImportProgress {
  /** Books finished (imported, skipped or failed). */
  done: number
  /** Books requested. */
  total: number
  /** The book being imported; `null` in the last event. */
  currentTitle: string | null
}

// ── Backup ───────────────────────────────────────────────────────────────────

/** What a backup holds (`read_backup`): its manifest, `librarium-backup.json`. */
export interface BackupManifest {
  /** Always `Librarium`. */
  app: string
  /** The backup format (1). */
  format: number
  /** The Librarium that made it. */
  appVersion: string
  /** When it was made (RFC 3339). */
  createdAt: string
  /** The user folders in it. */
  users: string[]
  files: number
  /** The files' total size. */
  bytes: number
}

/** `export_backup` result. */
export interface BackupExported {
  /** The zip written; `null` when cancelled. */
  path: string | null
  users: string[]
  files: number
  bytes: number
  /** `cancel_backup` stopped it: no file was left. */
  cancelled: boolean
}

/** `restore_backup` result. */
export interface BackupRestored {
  /** The users now in the library root. */
  users: string[]
  files: number
  /** `cancel_backup` stopped it before anything was replaced. */
  cancelled: boolean
}

/** Payload of the `backup-progress` event. */
export interface BackupProgress {
  /** `scanning`: listing the files; `copying`: zipping or extracting them; `replacing`: swapping the users. */
  phase: 'scanning' | 'copying' | 'replacing'
  filesDone: number
  filesTotal: number
  bytesDone: number
  bytesTotal: number
}

// ── Metadata dialog ──────────────────────────────────────────────────────────

/** What the metadata dialog asks for the cover of the (single) edited book. */
export interface CoverChange {
  action: 'none' | 'delete' | 'replace'
  /** The new image, for 'replace'. */
  base64: string | null
}

/**
 * Applies a reviewed "Find metadata online" result (TidyupReviewModal's
 * `submit` prop): the ticked fields and the cover URL (or null). Resolves
 * once saved; rejects with a readable message when it failed, and the
 * review then stays open and shows why.
 */
export type TidyupSubmit = (metadata: Partial<BookMetadata>, coverUrl: string | null) => Promise<void>
