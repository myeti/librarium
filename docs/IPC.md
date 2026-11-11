# Librarium IPC reference

The Vue frontend talks to the Rust backend only through Tauri commands
(`invoke('command', { ...args })`) and events (`listen('event', ...)`).
Tauri converts camelCase argument names to the snake_case Rust parameters.
Every command that can fail returns `Result<T, AppError>`: the promise
rejects with `{ code, message }` (see [Errors](#errors)), which the frontend
must surface (toast or inline error), never swallow. Every command runs off
the main thread (async, blocking work on Tauri's blocking pool), so a slow
disk or a sleeping NAS never freezes the window.

This file documents the contract. Keep it in sync when a command changes.
On the frontend, each command has one typed wrapper in `src/services/api.ts`
(`scan_library` → `api.scanLibrary`), the only place that calls `invoke`;
its argument and result types live in `src/types/`.

## Principles

- **Mutations are id-based.** The frontend sends `bookId`s; the backend
  looks paths, dates and sizes up in its own index (`library.json`). It never
  trusts a client-supplied path for writes.
- **Writes are atomic and serialised.** Every file the backend rewrites
  (the index, EPUBs, covers, device files) goes through `fsutil`
  (temp file + fsync + rename, per-path lock). On macOS the fsync is
  `F_FULLFSYNC`, falling back to plain `fsync` on volumes that refuse it
  (SMB shares, FAT e-readers) instead of failing the write
  (`fsutil::sync_file`).
- **Deletes are recoverable.** Files go to the OS trash, or to
  `<library>/.librarium/trash/<timestamp>/` on volumes without a trash
  (most SMB shares). Nothing ever asks for confirmation or deletes for good:
  on macOS the OS trash is `NSFileManager` (not Finder's AppleScript, which
  needed an Automation permission and, on a share without a Trash, asked
  "delete immediately?"), and a file is only reported as in the OS trash once
  it is really gone from the library. Trashing runs outside the index lock.
- **Slow work doesn't hold the library.** Walking the library, reading new
  files, rewriting EPUBs, copying imports and trashing run between two short
  holds of the index lock; the second re-checks what changed meanwhile, so
  covers, edits and the OPDS server keep answering during a long scan of a
  NAS library.
- **A missing library folder is never created.** An unmounted NAS share or
  an unplugged drive must not silently become an empty local folder. Every
  command that works in a library (scan, import, edits, users…) rejects
  instead, with code `LIBRARY_UNAVAILABLE` (see [Errors](#errors)). Only
  creating a user creates folders. A folder that is there but can't be read
  is `PERMISSION_DENIED` instead — not "is the NAS connected?".
- **Paths from the frontend are confined.** A path the frontend sends for a
  write, delete or reveal must be inside the library folder (or is looked up
  in the index by id). The reader opens books by id, and only exposes a
  file whose extension and first bytes match the book's format.
- **One bad file costs only itself.** Parsing a book (scan, import,
  conversion) is panic-isolated; a file that can't be read is skipped and
  remembered (`.librarium/failed.json`), never a reason for a scan to fail.

## Errors

A rejected command gives an object (the Rust `AppError`, `src-tauri/src/error.rs`):

```ts
interface AppError {
  code: string      // stable, SCREAMING_SNAKE_CASE; new codes may be added, never renamed
  message: string   // English sentence(s) for the user; names files, not full paths
}
```

Read errors with `errorMessage(e)` / `errorCode(e)` (`src/utils/errors.ts`):
they also accept the plain strings older backends sent (`LIBRARY_UNAVAILABLE:
…` included), `Error` objects and `{ message }` objects.

| Code | Meaning | UI |
|---|---|---|
| `LIBRARY_UNAVAILABLE` | The library folder (or the library root, for `list_users`/`create_user`; the OPDS root for `start_opds`) doesn't exist, or the share under it stopped answering (timeout, not connected, stale handle): share not mounted, drive unplugged, folder moved. Nothing was created. The message names the folder. | "Library folder not reachable — is the drive/NAS connected?" with Retry (library view or user picker). |
| `PERMISSION_DENIED` | The library folder (or root) is there, but the system refuses to let Librarium read it (`EPERM`/`EACCES`). On macOS this is privacy protection: until Librarium is allowed in System Settings › Privacy & Security › Files and Folders (Network Volumes, or Removable Volumes), every read under `/Volumes` fails. Returned by `list_users`, `scan_library` (the folder can't be listed) and any command whose index read is refused. Nothing was written; the index is *not* treated as damaged. The message names the folder and says where to allow access (macOS), or to check the permissions (elsewhere). | Show the message, distinctly from "not reachable", with Retry (library view or user picker). The user picker keeps the current user. |
| `INDEX_DAMAGED` | `library.json` and its backup are both unreadable (see "Library index"). Nothing was written; the message says how to recover. | Show the message; no automatic retry. |
| `INDEX_TOO_NEW` | The index was written by a newer Librarium. Nothing was written. | "Update Librarium". |
| `NOT_FOUND` | Unknown book id, a file deleted behind the app's back, no Calibre `metadata.db`. | Usually "rescan the library". |
| `INVALID_INPUT` | The request is wrong: malformed id or URL, unsupported format, empty name, path outside the library, a file that isn't a book. | Show the message. |
| `IO` | Reading or writing a file failed (permissions of one file, disk full, share error). A failure to *read* `library.json` is `IO` (or `PERMISSION_DENIED`/`LIBRARY_UNAVAILABLE`), never a reason to fall back to the backup. | Show the message; retry may help. |
| `NETWORK` | Download / HTTP failure: offline, timeout, HTTP error status, too large, not the expected content. | Show the message; retry may help. |
| `CONVERSION` | boko couldn't convert (DRM, unreadable book, output failed validation). The original is untouched. | Show the message. |
| `DEVICE` | E-reader missing or not a Kindle/Kobo; a path that isn't on a connected e-reader; `send_books` while the device is being ejected; ejecting failed (the message says why) or isn't possible (Windows: "use Safely Remove Hardware"). | Show the message. |
| `CONFLICT` | Something changed meanwhile (book renamed during a conversion; in `delete_books.failures`, a book renamed or converted while its file was being trashed — it is kept), a name is taken (user exists, export over another book), port in use, `eject_device` while books are being sent to that device. | Show the message; retry may help. |
| `INTERNAL` | A bug (a panic, a dead task). Details are in the log file. | Show the message; suggest "Open log folder". |
| `OTHER` | Not classified yet (a message from a helper deep in the backend). | Show the message. |

Per-item failures inside a successful result (`Failure.error`,
`ImportResult.failed[].reason`, `transfer-progress.errorMessage`) are plain
message strings.

Every error returned by a command is also written to the debug log and the
log file, as `[command] ERROR (CODE) — message`.

## Logging

Every line of the debug panel (event `app:log`, payload: the line) also goes
to the log file through `tauri-plugin-log`: `Librarium.log` in the app's log
folder (macOS `~/Library/Logs/com.librarium.desktop/`, Windows
`%LOCALAPPDATA%\com.librarium.desktop\logs\`, Linux
`~/.local/share/com.librarium.desktop/logs/`). A line containing `ERROR` is
logged as an error, `WARNING` as a warning, the rest as info; panics are
logged with their location. Release builds write info and above (other
crates: warnings), debug builds debug and above and echo to stdout. Times
are UTC. The file rotates at 5 MB; 5 files are kept. It never leaves the
computer.

| Command | Arguments | Returns | Notes |
|---|---|---|---|
| `open_log_folder` | — | `void` | Shows the log folder in Finder/Explorer, `Librarium.log` selected (creates the folder if needed). Settings › Advanced › Show the log file, and the menu bar's Help › Open Log Folder. |

## Plugins used from the frontend

| Plugin | Permission (`capabilities/default.json`) | Use |
|---|---|---|
| `@tauri-apps/plugin-opener` | `opener:allow-open-url` for `https://*`, `http://*`, `mailto:*` | `openUrl(url)`. Links with `target="_blank"` (and ctrl/shift-clicked links) are opened in the browser by the plugin itself (a click handler it injects); nothing to add in the app. Revealing files goes through `reveal_in_explorer` (confined to the library), not the plugin's `revealItemInDir`, which isn't granted. |
| `@tauri-apps/plugin-process` | `process:allow-restart` | `relaunch()` after the updater installed an update. |
| `@tauri-apps/plugin-dialog`, `@tauri-apps/plugin-updater` | `dialog:default`, `updater:default` | As before. |

## Shared types

```ts
interface Book {
  id: string
  path: string      // absolute (relative only inside the index file, see below)
  filename: string; format: BookFormat
  sizeBytes: number; dateAdded: string
  coverPath?: string; coverVersion?: number   // output only, never stored; see "Covers" below
  favorite?: boolean   // starred (set_favorite); absent when not. Index only, never in the file
  metadata: BookMetadata
}

/** A cover file the webview can show. */
interface CoverInfo {
  coverPath: string      // absolute path: the thumbnail, or the full cover if no thumbnail could be made
  coverVersion: number   // the file's mtime in ms, appended as `?v=` (cache buster)
}

interface BookMetadata {
  title: string     // '' when unknown: the UI shows "Unknown title" (localised)
  author: string    // '' when unknown: the UI shows "Unknown author"
  subtitle?: string
  series?: string; seriesIndex?: number; language?: string
  description?: string; isbn?: string; rating?: number
  categories: string[]; goodreadsUrl?: string
}

/** Batch edit: every field optional; an absent (or null) field is left
 *  unchanged. An empty string clears an optional text field (series,
 *  subtitle, language, description, isbn, goodreadsUrl); a blank title or
 *  author is ignored. `categories` replaces the whole list. */
type BookMetadataPatch = Partial<BookMetadata>

interface Failure { id: string; error: string }

interface ForgetResult { forgotten: string[]; failures: Failure[] }

interface LockStatus { otherInstance: { host: string; updatedAt: string } | null }
```

Unknown titles and authors are empty strings, never placeholders: nothing
like "Unknown author" is stored in the index or written into files. Display
them with `displayTitle` / `displayAuthor` (`src/utils/bookDisplay.ts`);
sorting puts empty values last.

**How edits reach EPUB files.** Only the fields an edit changes are written
into the file; fields it doesn't touch stay exactly as the file has them
(including values the index never read, such as a second author or a
Calibre rating). A field cleared in the app is removed from the file,
except title, author and language, which are left as they are (`dc:language`
is mandatory). Several authors (`"A & B"`) become several `dc:creator`s;
editors, translators and other non-author creators are kept. The ISBN is
written as its own `dc:identifier` (`opf:scheme="ISBN"` in EPUB 2,
`urn:isbn:` in EPUB 3), only if its checksum is valid; the identifier named
by the package's `unique-identifier` is never modified (readers key their
data on it, and embedded fonts are de-obfuscated with it). The rating is
written as Calibre's `calibre:rating` (0–10); the series as
`calibre:series`/`calibre:series_index`, plus an EPUB 3 series collection if
the file already has one. Subtitle and Goodreads URL stay in the index. An
OPF that isn't well-formed XML (or isn't UTF-8) is never rewritten: the
command fails and the file is left as it was.

## Library commands

| Command | Arguments | Returns | Notes |
|---|---|---|---|
| `scan_library` | `path` | `Book[]` | `LIBRARY_UNAVAILABLE` if the folder doesn't exist (never created); `PERMISSION_DENIED` if it can't be listed (never an empty library). Rejects (and never overwrites the index) if `library.json` is unreadable and no valid `.bak` exists, or was written by a newer version. New files are indexed — or **re-linked** to a missing book with the same file name and size (same id, metadata, date added). Files the user removed from the library (`ignored`) are skipped. A new file that can't be read as a book — a parser panics on it, or an EPUB/PDF whose content doesn't match its extension (an HTML page saved as `.epub`, an empty file) — is left out, logged and recorded in `failed.json`; later scans skip it without opening it until its size or mtime changes (see "Library index"). An EPUB/PDF whose metadata merely can't be parsed is still added, titled after its file name. Indexed files that are gone become **missing** (not returned; kept 60 days) — only when the whole tree was walked without errors, and only books that were indexed before the walk started and whose file is really gone (or inside a skipped folder). Hidden folders and NAS/OS folders (`#recycle`, `#snapshot`, `@eaDir`, `@Recycle`, `$RECYCLE.BIN`, `System Volume Information`, `lost+found`…) are not walked. The walk and the reading of new files run without the index lock (other commands go on meanwhile); the result is merged under the lock into the index as it is then, keeping changes made during the scan, and books added meanwhile are returned too. Also marks the library as open by this instance (`lock.json`). Books whose thumbnail exists come with `coverPath`/`coverVersion` (one directory listing, no image work); the others have none until `ensure_cover` is asked. Also opens the asset-protocol scope for `<path>/.covers`. |
| `import_books` | `paths, libraryPath` | `Book[]` | `LIBRARY_UNAVAILABLE` if the library folder doesn't exist. A file the library already holds is returned as is (and is un-ignored if it had been removed from the library); a new file matching a missing book (name and size) gets that book's record back. A file that can't be read as a book (as in `scan_library`) isn't returned: its copy is removed again (the original is untouched) and the reason is logged; the other files are still imported. Books on an MTP Kindle (`mtp://` paths from `list_device_books`) are first copied from the device into private temp folders (removed afterwards); if one can't be copied the command rejects (`DEVICE`, with the reason) and nothing is imported. Copies are staged as hidden temp files in the library and read there, without the index lock; imports of one library wait for each other (so a payload delivered twice is still a re-import), and the copies get their final names under the index lock. |
| `set_favorite` | `libraryPath, bookIds, favorite` | `Book[]` | Stars (`true`) or unstars books: `favorite` in the index, **never written into the files**, so any format, and quick. Returns the books as saved, in the order asked (a repeated id once); an id that isn't in the library is left out (logged). The index is saved only if a book changed. Kept by edits, renames, conversions, resets and re-links. |
| `forget_books` | `libraryPath, bookIds` | `ForgetResult` | "Remove from library (keep file)": the books leave the index, their files are **not touched**, and their places are added to `ignored` so scans don't re-add them. Cached covers are removed. Importing the file again brings it back (as a new book). Unknown ids go to `failures`. |
| `library_lock_status` | `libraryPath` | `LockStatus` | Call when a library is opened. Marks it as open by this instance, and reports another Librarium on another computer that used it less than 5 minutes ago (`lock.json`), so the UI can warn. An entry from this computer (same host name) is its own previous run — the single-instance plugin never lets two run side by side — and is ignored, so relaunching after a crash doesn't warn. Advisory: never blocks anything. `LIBRARY_UNAVAILABLE` if the folder doesn't exist. |
| `update_metadata` | `libraryPath, bookId, metadata: BookMetadata` | `Book` | Single-book edit: replaces the metadata (empty optional fields clear them). For EPUBs the backend also writes what changed into the file (see "How edits reach EPUB files" above); the index only changes if that write succeeds. |
| `batch_update_metadata` | `libraryPath, bookIds, patch: BookMetadataPatch, seriesIndexes?: Record<string, number>` | `{ books: Book[], failures: Failure[] }` | Merges the patch into **each** book's own metadata, then gives each book listed in `seriesIndexes` (book id → volume, ≥ 0; others ignored) its own volume number, overriding the patch's `seriesIndex` (the metadata dialog numbers a series in the order the user chose), and writes EPUBs. Books that fail (unknown id, unwritable file) are listed in `failures` and left unchanged; the others are still updated. The EPUBs are rewritten without the index lock; a book whose record changed meanwhile gets the patch on top of that change. `books` and `failures` follow the order of `bookIds`. |
| `rename_book` | `libraryPath, bookId, newFilename` | `Book` | Keeps id, metadata and `dateAdded`. The name is sanitised (no path, no leading dot) and keeps the book's extension (appended if missing); a `_2`-style suffix avoids collisions. |
| `delete_books` | `libraryPath, bookIds` | `{ deleted: string[], failures: Failure[], libraryTrash: string \| null }` | Moves files to the trash; `libraryTrash` is the fallback folder (`<library>/.librarium/trash/<timestamp>`) when it was used (a volume without an OS trash, like most SMB shares). Never asks anything and never deletes for good (macOS: `NSFileManager`, not Finder). Also removes cached covers. Carries on past failures. A book whose file is already gone is just removed from the index. Files are trashed outside the index lock; a book renamed or converted meanwhile is kept and reported in `failures` (a `CONFLICT` message). `deleted` and `failures` follow the order of `bookIds`. |
| `reset_book_data` | `libraryPath, bookId` | `Book` | Re-reads metadata from the file; keeps the id and `dateAdded`; removes the cached cover, its thumbnail and the "no cover" marker (call `ensure_cover` to get the file's own cover back). |
| `convert_to_epub` | `libraryPath, bookId` | `Book` | MOBI/AZW/AZW3 → EPUB with boko. The EPUB keeps id, metadata and `dateAdded`; the original goes to the trash only after the EPUB has been validated (see "Conversions"). |
| `ensure_cover` | `libraryPath, bookId` | `CoverInfo \| null` | The cover to show, made on demand (see "Covers"). `null`: the book has no usable cover. Rejects an id other than letters, digits and `-` (max 64), and an id that has no cache and isn't in the index. Called lazily by the frontend for visible cards, 4 at a time. |
| `save_cover_from_url` | `libraryPath, bookId, url` | `CoverInfo` | Rejects anything that isn't a JPEG/PNG/WebP image (placeholder GIFs, error pages), or that can't be decoded, before anything changes. 30 s timeout, 20 MB cap. Writes the full cover and its thumbnail; embedding into the EPUB is best effort (logged). |
| `set_book_cover` | `libraryPath, bookId, imageBase64` | `CoverInfo` | Plain base64 or a `data:` URL; JPEG/PNG/WebP only, must decode, 20 MB cap. Same effects as `save_cover_from_url`. |
| `delete_book_cover` | `libraryPath, bookId` | `void` | Rejects if the cover can't be removed from the EPUB file (the cache is then kept too). Removes the full cover and the thumbnail, then writes the "no cover" marker, so an image the file still holds (a PDF's first page, a CBZ's first image) doesn't come back until the file changes or the book is reset. |
| `download_and_import` | `url, filename, libraryPath` | `Book` | `LIBRARY_UNAVAILABLE` if the library folder doesn't exist (checked before downloading). `http`/`https` only; 300 MB cap, 60 s idle timeout, 30 min overall. The extension must be supported and the bytes must match it (ZIP for EPUB/CBZ, `%PDF`, `BOOKMOBI`). Re-downloading a book already in the library returns the existing book. |
| `export_book` | `libraryPath, bookId, targetFormat, destPath` | `void` | `targetFormat`: `epub` or `azw3` (anything else, PDF included, is `INVALID_INPUT`); the source must be EPUB, MOBI, AZW or AZW3 (boko can't read PDF/CBZ: `CONVERSION`). The book is looked up by id. `destPath` is where the user chose in a save dialog: the converted, validated file is written to a hidden temp file in that folder and renamed over it, so a failed export leaves nothing (and whatever was there) untouched. Refused: `destPath` being the book itself (compared canonicalised: links, `..`, case) and, inside the library, another book of the index (`CONFLICT`). An AZW3 carries the book id as its ASIN (EXTH 113), like the copies sent to a Kindle. |
| `inspect_epub` | `path` | JSON summary | **Debug builds only** (not registered in release builds; the UI only offers it when `import.meta.env.DEV`). Reads any `.epub` path: OPF path, cover declaration, manifest, spine, ZIP entries. |

Calibre import and the reader have their own sections: [Calibre
import](#calibre-import), [Reader](#reader).

Metadata is written into EPUB files by `update_metadata` /
`batch_update_metadata`; covers are made on demand by `ensure_cover`.

### Conversions

Every conversion — `convert_to_epub`, `export_book`, the reader's
MOBI/AZW/AZW3 → EPUB (cached, see [Reader](#reader)), device transfers (`send_books` with `needs_conversion`) — goes
through one module (`src-tauri/src/convert.rs`): boko opens the source (a
DRM-protected book is refused), writes a hidden `.librarium-convert-*` temp
file, fsyncs it, and **validates** it before anything uses it: it must open
again, have a chapter, have text (or images) and keep at least a tenth of
the source's text. A panic inside boko is a `CONVERSION` error. The temp
file is always removed unless kept. Output formats: EPUB and AZW3 (device
targets `mobi`/`azw` also give AZW3). An AZW3 written for a library book
(export, device transfer) has the book id as its ASIN (EXTH 113, cdetype
`EBOK`) rather than boko's shared `EBOK000000` placeholder (vendored boko,
`Azw3Config::asin`). An AZW3 for a device transfer is dated
with the book's `dateAdded` (the PDB header dates, which also seed the
embedded-font obfuscation key: `Azw3Config::timestamp`), so converting an
unchanged book again gives the same bytes; other conversions use the
current time.

### Covers

Covers are files, never data URLs: the webview shows
`convertFileSrc(coverPath) + '?v=' + coverVersion` (`coverUrl()` in
`src/utils/covers.ts`) through Tauri's asset protocol. Its scope is empty in
`tauri.conf.json`; `scan_library`, `ensure_cover`, `set_book_cover` and
`save_cover_from_url` add the open library's `.covers` folder (and only that)
at runtime (`open_for_reading` adds the one file the reader opens, see
[Reader](#reader)). The CSP allows `asset:` and `http://asset.localhost`
(Windows) in `img-src` (covers), `connect-src` (the reader's `fetch`),
`font-src` and `frame-src` (the PDF viewer's iframe). `connect-src` also has
`ipc: http://ipc.localhost` (Tauri 2's IPC; without it the first `invoke`
is a CSP violation and falls back to the slower postMessage IPC). epub.js
serves a book's own CSS and fonts as `blob:` URLs inside its iframe, hence
`blob:` in `style-src` and `font-src`; `script-src` stays `'self'`.

| File in `<library>/.covers/` | Content |
|---|---|
| `<id>.jpg \| .png \| .webp` | Full-size cover, as found in the book or set by the user. Served by OPDS. |
| `thumbs/<id>.jpg` | Thumbnail shown by the app: fits in 600 × 600 px (never enlarged), JPEG quality 82, transparency on white. Served by OPDS as the thumbnail. |
| `<id>.none` | "No cover" marker (JSON: the book file's path relative to the library, size and mtime). |

`ensure_cover` works in order of cost: the thumbnail if it exists; else a
thumbnail made from the cached full cover (or that full cover, if it can't be
decoded); else `null` while the marker matches the book file's current size
and mtime; else the cover is looked for inside the book (EPUB manifest cover,
CBZ first image by name, PDF largest JPEG of page 1), cached with its
thumbnail, or the marker is written. Only that last step reads the index and
the book. A GIF or other non-JPEG/PNG/WebP image counts as no cover; MOBI and
AZW3 have no extraction (a cover set by the user or imported from Calibre is
still served). `coverPath`/`coverVersion` are never stored in the index.

In EPUB files the cover shown is the one the OPF *declares*
(`properties="cover-image"`, else `<meta name="cover">`), else a display-only
guess (a `<meta name="cover">` naming the image's href, a guide reference to
an image, an image whose id or file name is `cover`). Setting a cover
(`set_book_cover`, `save_cover_from_url`) replaces only the declared image,
updating its media type; without one, a new `cover.<ext>` (or
`cover-1.<ext>`…) is added and declared both ways. Removing a cover
(`delete_book_cover`) removes its declarations, and the image file only if
nothing else in the book (a cover page, a style sheet) still refers to it. A
guessed cover may be *shown* but is never overwritten or deleted.

## Calibre import

Code: `src-tauri/src/commands/calibre.rs` (commands) and `calibre/`
(`detect`, `db`, `import`).

| Command | Arguments | Returns | Notes |
|---|---|---|---|
| `detect_calibre_library` | — | `string \| null` | The library Calibre itself uses: `library_path` in Calibre's `global.py.json`, in `$CALIBRE_CONFIG_DIRECTORY` if set, else macOS `~/Library/Preferences/calibre/`, Windows `%APPDATA%\calibre\`, Linux `$XDG_CONFIG_HOME/calibre/` or `~/.config/calibre/`. If that's missing, unreadable or points to a folder without `metadata.db` (an unmounted share): the first of `~/Calibre Library`, `~/Documents/Calibre Library`, `~/Books` holding a `metadata.db`. Never fails (`null`). |
| `list_calibre_books` | `calibrePath` | `CalibreBook[]` | Sorted by title (case-insensitive, untitled last). Four queries whatever the library's size (10 000 books list in well under a second). `NOT_FOUND` without `metadata.db`; `INVALID_INPUT` if it isn't a Calibre database. |
| `import_from_calibre` | `calibrePath, bookIds, libraryPath` | `ImportResult` | See below. Progress: `calibre-import-progress` events. One import at a time: a second call while one runs is `CONFLICT`. `LIBRARY_UNAVAILABLE` if the library folder doesn't exist (nothing is created); `NOT_FOUND`/`INVALID_INPUT` as for `list_calibre_books`. Duplicates are found from the library index (the frontend sends no book list). |
| `cancel_calibre_import` | — | `boolean` | Stops the running import before its next book (a file being copied is finished first); `false` if none was running. The import then resolves normally with `cancelled: true`; what it had imported is kept and saved. |

```ts
interface CalibreBook {
  id: number                 // Calibre's book id, for bookIds
  title: string              // '' when unknown (Calibre's "Unknown")
  author: string             // Calibre's order, joined with ' & '; '' when unknown
  series: string | null
  seriesIndex: number | null // only with a series
  availableFormats: string[] // lower case: 'epub', 'pdf', 'txt'…
  bestFormat: string | null  // what would be imported; null: nothing the library supports
  hasCover: boolean
}

interface ImportResult {
  imported: Book[]           // saved in the library index (also when cancelled or stopped)
  skipped: SkippedBook[]
  failed: { title: string; reason: string }[]   // reason: a message for the user
  cancelled: boolean         // cancel_calibre_import was called
  error: string | null       // why the import stopped early, when some books were already imported
  remaining: number          // requested books not looked at (cancelled or stopped)
}

interface SkippedBook {
  title: string; author: string
  reason: 'alreadyInLibrary' | 'unsupportedFormat' | 'noFile'
  existingFilename: string | null   // alreadyInLibrary: the library's copy
}

/** Payload of the `calibre-import-progress` event. */
interface CalibreImportProgress {
  done: number               // books finished (imported, skipped or failed)
  total: number              // books requested (a repeated id counts once)
  currentTitle: string | null  // the book being imported; null in the last event
}
```

Events are sent when a book starts, at most every 100 ms, plus a last one
(`currentTitle: null`) when the import ends, cancelled or not. A failed
book's `title` is `#<id>` when its record couldn't be read.

**What is imported.** For each book, in the order of `bookIds`: its best
file among EPUB, AZW3, MOBI, AZW, PDF, CBZ (in that order; TXT, RTF, DOCX,
`ORIGINAL_*` backups… are never imported), checked like any file entering
the library (its first bytes must match its format), copied atomically into
the library root under Calibre's file name; its `cover.jpg` into the cover
cache (`<library>/.covers/<id>.jpg`; `coverPath` stays unset, the thumbnail
is made by the first `ensure_cover`); and this metadata:

| Librarium | From Calibre |
|---|---|
| `title` | `title` (Calibre's "Unknown" placeholder → `''`) |
| `author` | the authors in Calibre's order (its link table's order, so always the same), joined with `' & '`; commas restored (Calibre stores them as `\|`); "Unknown" dropped |
| `series`, `seriesIndex` | the series; its index **only if** the book has a series (Calibre gives every book an index of 1) |
| `categories` | the tags, in Calibre's order |
| `rating` | Calibre's 0–10 (2 per star) as 1–5 stars, half stars rounded **up** (7 → 4, 1 → 1); 0 = not rated. Same rule as `calibre:rating` read from EPUB files. |
| `language` | the first language, ISO 639-2 → 639-1 (`fra` → `fr`) |
| `description` | the comments: HTML → text, paragraphs kept |
| `isbn`, `goodreadsUrl` | the `isbn` and `goodreads` identifiers |

**Duplicates and names.** A book is skipped (`alreadyInLibrary`, with the
library copy's file name) when the library already has a book with the same
title and authors — compared case-insensitively, whitespace-insensitively,
in Unicode NFC, authors in any order — or a byte-identical file (an indexed
file of the same size, or a not-yet-indexed file with the same name). A
*different* book whose file name is taken gets a free one (`Poems_2.pdf`).
Books imported earlier in the same run count as library books.

**Saving.** The import works in batches of up to 50 books or 5 seconds:
each batch holds the library index lock, then is saved. Other commands
(a scan, an edit) wait for one batch at most, and a crash loses at most one
batch's index entries (their files are found by the next scan, without
Calibre's metadata). A problem with one book (file missing, copy failed)
lands in `failed`; a problem with the library itself (unreachable, index
can't be saved) stops the import — rejected as an error if nothing had been
imported yet, otherwise resolved with `error` set (that batch's books, copied
but not indexed, are listed in `failed`). The frontend (`stores/calibre.ts`)
adds `imported` to its book list (which also calls `invalidate_opds_cache`),
and rescans the library after `error` or a rejection that may have copied
files (any code but `CONFLICT`, `NOT_FOUND`, `INVALID_INPUT`). Closing the
import dialog cancels the import; the dialog closes once it has stopped.

The database is opened read-only (Calibre may be running; a lock is waited
for up to 5 s). Missing optional tables (old libraries) leave fields empty;
a query that fails on an unexpected schema leaves its field empty and logs
one warning. Book folders and file names from the database must be plain
relative names (no `..`, no absolute path), or the book fails.

## Backup

Code: `src-tauri/src/commands/backup/mod.rs` (commands, progress, the
one-at-a-time slot) and `backup/archive.rs` (the format, export, restore).
Frontend: `stores/backup.ts`, Settings › Backup.

| Command | Arguments | Returns | Notes |
|---|---|---|---|
| `export_backup` | `libraryRoot, destDir` | `BackupExported` | Zips every user of the library root into a new file in `destDir`: `Librarium backup 2026-11-10 14.32.zip` (`_2` added if taken). Written to a hidden `.librarium-backup-*.zip.part` in `destDir`, renamed at the end: a cancel or a failure leaves no file. `IO` when `destDir` hasn't room for the books plus 32 MB; `NOT_FOUND` when it isn't there; `INVALID_INPUT` when it is inside a user folder (the backup would be in the next ones, and erased by a restore; the root itself is fine). |
| `read_backup` | `path` | `BackupManifest` | What a backup holds, to say before restoring. `INVALID_INPUT` when the file isn't a zip or has no Librarium manifest, or was made by a newer Librarium (a newer `format`). |
| `restore_backup` | `libraryRoot, path` | `BackupRestored` | **Replaces every user of the root** with the backup's; see below. `INVALID_INPUT` as `read_backup`, and when the backup is inside a user folder (the restore would erase it). |
| `cancel_backup` | — | `boolean` | Stops the running export, or the running restore while it extracts (afterwards the users are being swapped, which isn't stopped); `false` if none was running. The command then resolves normally with `cancelled: true`. |
| `reveal_backup` | — | `void` | Shows the last backup `export_backup` wrote in Finder/Explorer (the export toast's Show). Only that file: never a path from the webview. `NOT_FOUND` before the first export. |

`libraryRoot` must be the settings' library folder (`libraryRootOverride`,
else `~/Documents/Librarium`; compared after resolving links), or the
command is `INVALID_INPUT`: a restore erases the users of the folder it is
given, which the webview doesn't get to choose. One export or restore at a
time: a second is `CONFLICT`.

```ts
interface BackupManifest {   // librarium-backup.json, at the root of the zip
  app: 'Librarium'
  format: number             // 1
  appVersion: string         // the Librarium that made it
  createdAt: string          // RFC 3339
  users: string[]            // the user folders in it
  files: number
  bytes: number              // the files' total size
}

interface BackupExported {
  path: string | null        // the zip written; null when cancelled
  users: string[]; files: number; bytes: number
  cancelled: boolean
}

interface BackupRestored {
  users: string[]            // the users now in the root
  files: number
  cancelled: boolean         // nothing was replaced
}

/** Payload of the `backup-progress` event. */
interface BackupProgress {
  phase: 'scanning' | 'copying' | 'replacing'
  filesDone: number; filesTotal: number
  bytesDone: number; bytesTotal: number
}
```

Progress events go out at most every 150 ms and at every change of phase:
`scanning` (listing the files; for a restore, reading the zip and checking
the space), `copying` (zipping, or extracting), `replacing` (a restore
swapping the users).

**What a backup holds.** Every user folder of the root (the same as
`list_users`; empty ones too) with its books, `.librarium/` (the index,
`failed.json`) and `.covers/`, under `library/<user>/…`, and the manifest,
written last (a zip without one isn't a backup). Left out: anything else in
the root (a NAS's `#recycle`, unrelated files), `.librarium/trash/`,
`.librarium/lock.json`, temp files (`.librarium-*`, `*.part`), and system
files (`.DS_Store`, `._*`, `Thumbs.db`, `desktop.ini`). The settings
(`librarium-config.json`) aren't in it. Books and images are stored as
they are (already compressed); text files (`json`, `opf`, `xml`…) are
deflated. Each file keeps its modification date, to the even second (zip
dates are so; ZIP64 past 4 GB).

**Restoring**, in an order that never leaves the library half gone:

1. The space is checked (the backup's size plus 32 MB on the root's
   volume; `IO` otherwise), then every entry is extracted into a hidden
   staging folder of the root, `.librarium-restore-<id>`; each file is
   synced. Entries outside `library/<user name>/` (an invalid user folder
   name, `..`, an absolute path) are skipped and logged, never written. A
   cancel, a damaged zip (a CRC mismatch) or a full disk stops here: the
   staging folder is deleted and nothing was touched.
2. The current users are renamed into `.librarium-restore-<id>-previous`,
   then the backup's are renamed in (instant on one volume). If a rename
   fails, what was moved is put back, and the command fails.
3. The previous users are deleted (that is the "erase"), then the staging
   folder; one that can't be deleted stays hidden, logged.

The root is created if it is missing (a new computer). The backend's cache
of library indexes is cleared. The frontend closes the library before the
restore (and stops the OPDS server) without signing the user out, refuses
it while books are being imported or transferred, then reloads the users
like a fresh start (`users.init()`: with several users, the picker). After
a cancel or a failure it reopens the library as it was.

## Reader

Code: `src-tauri/src/commands/reader.rs`, `reader/cache.rs`.

| Command | Arguments | Returns | Notes |
|---|---|---|---|
| `open_for_reading` | `libraryPath, bookId` | `ReaderSource` | The file the reader should display, added to the asset protocol's scope (that file only). EPUB and PDF: the book's own file. MOBI, AZW, AZW3: its EPUB conversion, from the reader cache (converted on first open, see "Conversions"; `CONVERSION` if that fails). The book is looked up by id; its file must be inside the library and have its format's extension and first bytes (ZIP, `%PDF-`, `BOOKMOBI`), else `INVALID_INPUT` (also for CBZ, which has no reader). `NOT_FOUND` for an unknown id or a missing file. |

```ts
interface ReaderSource {
  format: 'epub' | 'pdf'   // how to display path
  path: string             // absolute: the book, or its cached EPUB conversion
  sizeBytes: number        // of that file
  converted: boolean       // path is a MOBI/AZW/AZW3 → EPUB conversion
}
```

**Why a path and not bytes.** The webview reads the file itself through
the asset protocol: nothing goes through IPC, Rust never holds the book in
memory, and the protocol answers Range requests, so the PDF viewer can
fetch only what it shows. (Raw bytes through `tauri::ipc::Response` would
avoid base64, but still load the whole book in Rust and copy it into the
JS heap before anything is displayed.) The frontend (`src/services/reader.ts`,
used by `ReaderModal`) does:

```ts
const source = await api.openForReading(libraryPath, book.id)
const url = convertFileSrc(source.path)
if (source.format === 'pdf') {
  iframe.src = url            // the webview's PDF viewer reads it (CSP: frame-src asset:)
} else {
  const data = await (await fetch(url)).arrayBuffer()   // CSP: connect-src asset:
  ePub(data)                  // epub.js
}
```

(`prepare_for_reading`, which sent the whole book as base64 inside JSON —
several GB of memory for a 300 MB PDF — is gone.) The EPUB renderer is
epub.js 0.3.93, unmaintained; its `@xmldom/xmldom` dependency is pinned to a
patched 0.8.x by `overrides` in package.json (`npm audit --omit=dev` is
clean), and `src/services/epubjs.test.ts` checks that epub.js still opens a
book with it. In the webview epub.js parses with the DOM's own `DOMParser`;
xmldom is only its fallback.

**Conversion cache** (`<app cache>/reader/`, e.g. macOS
`~/Library/Caches/com.librarium.desktop/reader/`): entries are named
`<bookId>-<size>-<mtime ms>.epub` after the *source* file, so a book whose
file changed is converted again (and its outdated entries removed). After
each conversion, least recently used entries (by modification time, which
a hit refreshes) are removed until the cache is at most 500 MB; the entry
just made is always kept. Conversions are written to a hidden temp file and
renamed into place once validated; leftover temp files are removed after an
hour. Deleting the folder is always safe.

Files added to the asset scope stay readable by the webview until the app
quits (Tauri can't narrow a scope again without also refusing the file
later); they are only book files the user opened, or their conversions.

## Users and the file manager

| Command | Arguments | Returns | Notes |
|---|---|---|---|
| `list_users` | `libraryPath` (the library root) | `string[]` | The user folders of the root, sorted: not hidden ones, not names starting with `#` or `@` (Synology/QNAP: `#recycle`, `@eaDir`…), not `$RECYCLE.BIN`, `System Volume Information`, `lost+found`… `LIBRARY_UNAVAILABLE` when the root doesn't exist (it is never created here), `PERMISSION_DENIED` when it can't be read, `IO` on another read error — never a partial list. (The frontend treats `LIBRARY_UNAVAILABLE` for the default root `~/Documents/Librarium` as "no users yet" — a first run — and any other failure as "users unknown": the picker shows the reason with Retry and keeps the current user.) |
| `create_user` | `libraryPath, name, createRoot?: boolean` | `string` (the folder name) | The name is made safe for every filesystem (same rules as book file names; no leading `.`, `#` or `@`). Rejects if it is empty, a system folder name (`lost+found`…) or the folder exists. The root itself is only created with `createRoot: true` (first run, default root); otherwise a missing root is `LIBRARY_UNAVAILABLE`. |
| `reveal_in_explorer` | `libraryPath, path` | `void` | Shows `path` selected in Finder/Explorer/the Linux file manager (through `tauri-plugin-opener`'s reveal API, which doesn't hand-quote an `explorer /select` command line). Rejects unless `path` exists (`NOT_FOUND`) and is inside `libraryPath` (`INVALID_INPUT`). |
| `detect_language` | `text` | `string \| null` | ISO 639-1 code from `whatlang`, `null` below 75 % confidence. |
| `check_path_accessible` | `path` | `boolean` | Whether `path` is a reachable folder; `false` after 3 s (sleeping NAS). |

## Library index (`<library>/.librarium/library.json`)

| File in `.librarium/` | Written by | Role |
|---|---|---|
| `library.json` | this version | the index, format 2 (below) |
| `library.json.bak` | this version | the previous good `library.json` |
| `books.json`, `books.json.bak` | an older format **only** | the older index (format 1). Read once, to create `library.json`, and **never written**. |
| `books.v1.json.bak` | this version, once | byte copy of the older file a migration started from |
| `failed.json` | this version | files that couldn't be read as books (below); absent in a healthy library |

`failed.json` maps index keys (relative paths, as in `library.json`) to
`{ size, mtime, error }` (`mtime` in ms since the Unix epoch): files a scan
or an import left out because a parser panicked on them or their content
doesn't match their extension. Scans skip a listed file without opening it
while its size and mtime are unchanged, and drop entries whose file is gone
or became a book. Temporary read errors (a NAS hiccup) are never listed.
It's a cache: written atomically under the index lock, treated as empty if
damaged, safe to delete (every listed file is then tried once more).

Once `library.json` exists it is the only index read: a `books.json`
beside it is left alone, and changes made to it afterwards are **not picked
up** (books added there are found by the next scan as new files; edited
metadata isn't imported). The debug log says so when `books.json` is newer
than `library.json`.

The backend is the only reader and writer of `library.json` (the OPDS server
reads it, never writes it). Format version 2:

```jsonc
{
  "version": 2,
  "libraryId": "<uuid>",                    // stable id of this library
  "books":   { "<relKey>": Book },          // Book.path = the same relative path
  "ignored": [ "<relKey>" ],                // removed from the library, files kept (forget_books)
  "missing": { "<relKey>": { "book": Book, "missingSince": "<rfc3339>" } }
}
```

- **relKey**: the path relative to the library folder, `/` separators,
  Unicode NFC (macOS and SMB can report NFD names: both spellings find the
  same book). In the file, `Book.path` is that relative path in the spelling
  found on disk; over IPC it is absolute. A library that is moved, or
  mounted under another path, keeps working.
- Keys that could point outside the library (`..`, absolute, drive letters)
  are never turned into paths: on load they are kept aside as missing.
- **missing**: books a complete scan didn't find. Not returned to the
  frontend; re-linked when a file with the same name and size appears;
  dropped after 60 days. For entries converted from format 1 that couldn't
  be placed, the key is their absolute path.
- Unknown top-level fields (written by a newer version) are preserved. A file
  whose `version` is newer than 2 is refused without being touched.
- `coverPath`/`coverVersion` are output only: never written to the file.
- Writes are atomic (temp file, fsync, rename); the previous version is kept
  as `library.json.bak`; a damaged `library.json` is never overwritten (the
  backup is used, the damaged file kept aside as
  `library.json.damaged-<time>`). With both unusable, nothing is written
  and the error is shown — there is no fallback to an older `books.json`.
- **Format 1** (`books.json`, one object mapping absolute paths to books)
  is converted on first access when there is no `library.json`
  nor `library.json.bak`: `books.json` (or, if it is damaged or missing,
  `books.json.bak`) is read and migrated in memory, a byte copy goes to
  `books.v1.json.bak` (or `books.v1.json_2.bak`… — an existing copy is never
  overwritten), and the result is written to `library.json`. The older
  files are left exactly as they were. A damaged `books.json` without a
  usable backup is refused (nothing written). Each entry is mapped to a relKey: the old path under
  the current library folder; else the part after the library folder's name
  (a moved mount point, a Windows `\\nas\…` path); else it is kept as
  missing so a scan can re-link it. Two entries for one file keep the best
  match. Ids, metadata and dates are kept; old "Unknown title"/"Unknown
  author" placeholders become `''`; entries that aren't book records are
  skipped (they remain in `books.json` and its copy). If the migrated index
  can't be written (read-only share), the library still opens and the
  migration is retried on the next change.

`<library>/.librarium/lock.json` — `{ instanceId, host, pid, updatedAt }` —
is written when a library is opened (scan, `library_lock_status`) and
refreshed on every index save; it is removed on exit if this instance still
holds it. It only serves to warn; it never prevents opening. A second launch
of the app on the same computer focuses the running window instead
(single-instance plugin).

## Settings

| Command | Arguments | Returns | Notes |
|---|---|---|---|
| `get_config` | — | `Record<string, unknown>` | A damaged settings file reads as `{}` (and is logged); it is left on disk untouched. |
| `set_config` | `updates: Record<string, unknown>` | `void` | Merges the keys into the file (others are kept), atomically. Rejects, without writing, if the current file is damaged. Setting `mtpEnabled` takes effect at once (see [MTP Kindles](#mtp-kindles-2024-experimental)). |

Settings the backend itself reads:

| Key | Type | Default | Effect |
|---|---|---|---|
| `mtpEnabled` | `boolean` | `false` | Experimental support for MTP Kindles (2024+). Off: Librarium only enumerates USB devices to say that one is plugged in, and never opens or claims it. Read at startup and on every `set_config` that contains it; turning it off releases every MTP device once its current operation ends. |

## Menu bar

macOS only (`src-tauri/src/menu.rs`): the app's own menu bar, installed at
startup in English, replacing Tauri's default one (it keeps the standard
Edit and Window items, so copy/paste and ⌘M still work). On Windows and
Linux there is none — a menu bar would add a native strip to the window —
and the Keyboard shortcuts dialog lists the same commands.

| Command | Arguments | Returns | Notes |
|---|---|---|---|
| `set_menu_language` | `lang: string` | `void` | Rebuilds the menu bar with `'en'` or `'fr'` labels (`fr-CA` counts as French; anything else gives English). The frontend calls it at startup and when the user switches language. Does nothing outside macOS. Built on the main thread; a failure is `INTERNAL` (and logged), the previous menu stays. |

Events:

- `menu-command`: payload the chosen item's id (a `string`), for the
  custom items below only. Standard items (Copy, Hide, Quit, Minimize…)
  are handled by the OS and send nothing. The frontend routes the id
  through the shortcuts store, as if the shortcut had been pressed.

The accelerators mirror `src/constants/shortcuts.ts` (`Mod` there is
`CmdOrCtrl` in the menu); a frontend test reads `menu.rs` and checks them,
and a Rust test checks that every id below is listed here.

| Id | Menu | Item (EN / FR) | Accelerator |
|---|---|---|---|
| `app.about` | Librarium | About Librarium / À propos de Librarium (the app's own dialog) | — |
| `app.settings` | Librarium | Settings… / Réglages… | ⌘, |
| `library.import` | File | Import Books… / Importer des livres… | ⌘O |
| `library.export` | File | Export… / Exporter… | — |
| `view.grid` | View | Grid / Grille | ⌘1 |
| `view.list` | View | List / Liste | ⌘2 |
| `view.toggleDetails` | View | Show or Hide Details / Afficher ou masquer les détails | ⌘I |
| `library.search` | View | Search / Rechercher | ⌘F |
| `library.detectDevice` | Library | Detect E-reader / Détecter une liseuse | ⌘K |
| `library.findDuplicates` | Library | Find Duplicates… / Rechercher les doublons… | — |
| `help.shortcuts` | Help | Keyboard Shortcuts / Raccourcis clavier | ⌘/ |
| `help.logs` | Help | Open Log Folder / Ouvrir le dossier des journaux | — |
| `help.privacy` | Help | Privacy / Confidentialité | — |

The other items are standard: Services, Hide, Hide Others, Show All, Quit
(Librarium menu); Close Window (File); Undo, Redo, Cut, Copy, Paste, Select
All (Edit); Enter Full Screen (View); Minimize, Zoom (Window).

## Device commands

Kindles and Kobos connected as USB mass storage (`src-tauri/src/commands/device/`),
and — experimental, behind the `mtpEnabled` setting — MTP Kindles (2024+),
which appear in the same commands with `connection: "mtp"` and an
`mtp://` mount path (see [MTP Kindles](#mtp-kindles-2024-experimental)).

| Command | Arguments | Returns | Notes |
|---|---|---|---|
| `detect_devices` | — | `KindleDevice[]` | **Every** connected e-reader, polled by the frontend. A volume counts only if its name says "kindle"/"kobo" *and* it has a `documents/` folder (Kindle) or `.kobo/` (Kobo): look-alikes such as the "Kindle Previewer" installer image are ignored, and unrelated volumes (a sleeping NAS) are never looked into. Each device has a stable `id` (see "Device identity") and its current free/total space, so a poll shows a swap (another `id` at the same `mountPath`) and space changes. When a device appears, the `.librarium-*.part` files an interrupted transfer left on it (older than 5 minutes) are removed, unless something is being sent to it. The first time Librarium sees a device it writes `.librarium/device.json` (its id). Names never include a serial number. Then, with `mtpEnabled`, the **connected** MTP Kindles (`connection: "mtp"`); the others (MTP off, held by another app…) are only in `get_mtp_status`. |
| `detect_device` | — | `KindleDevice \| null` | The first of `detect_devices` (kept for single-device code). |
| `get_mtp_status` | — | `MtpStatus` | The MTP Kindles plugged in, whatever their state, and whether MTP support is on. Cheap; poll it with `detect_devices` to show why a Kindle isn't usable (see [MTP Kindles](#mtp-kindles-2024-experimental)). |
| `send_books` | `jobs: TransferRequest[]` | `void` | Progress through `transfer-progress` events (byte progress during the copy), and a `device-space` event after each job. Rejects (`DEVICE`) while a target device is being ejected. Per job: with `bookId` + `libraryPath` the book is read from the library index and the copy carries its id (AZW3 ASIN, EPUB `urn:librarium:` identifier — the library file is never modified), the transfer is recorded in the device manifest, and a re-sent book **replaces its previous copy**: it goes to the folder that copy is in (keeping a Kindle's `.sdr` next to it), and the previous copy is deleted if it had another name — only when the manifest says it is this book's, it is a book file inside the device and it isn't the file just written. A file with the same (sanitised) name is replaced, unless the manifest says it is *another* library book: the copy is then named `Name_2.ext`. A re-sent book whose copy on the device is already byte-identical is **not written again** (a rewritten file loses its Kindle reading position; conversions of a library book are reproducible, see [Conversions](#conversions)); the transfer is still recorded. On a Kindle, after the copy, the book's home-screen cover is written from the library's cover (`system/thumbnails/thumbnail_<ASIN>_<cdetype>_portrait.jpg`, JPEG, 470 px tall, as Calibre does; the Kindle's empty `.jpg.partial` attempt is removed): a Kindle doesn't make it for books with an ASIN. Only if `system/thumbnails/` exists; a failure is logged, never fails the job. Files are written as a hidden `.part` file, fsynced, then renamed; free space is checked first (`Not enough space on the device (needs X, free Y)`); names are FAT-safe. Without `bookId`/`libraryPath` (older frontends) the book is copied as before, with no identity and nothing deleted. |
| `cancel_transfer` | `jobId` | `void` | Real cancellation: a queued job is skipped; a running one stops at its next checkpoint — after the conversion (which itself can't be interrupted) or between 1 MB chunks of the copy. Its `.part` and conversion temp files are removed and any previous copy on the device is kept. The job then emits status `cancelled`. A job that already finished is unaffected. |
| `list_device_books` | `devicePath` | `DeviceBook[]` | Kindle: `documents/` and its subfolders (`.sdr` folders skipped). **Kobo: the whole volume** (hidden folders such as `.kobo`, and Windows' `$RECYCLE.BIN` / `System Volume Information`, skipped), merged with `.kobo/KoboReader.sqlite` (title, author, progress and read status as the Kobo shows them; read from a private copy of the database and its `-wal`, so a WAL-mode or locked database works and nothing is written to the device). Hidden files (`.librarium-*.part`, macOS `._*` files) are never listed. `libraryId` comes from the device manifest, else from the id inside the file. **Title and author**, in order: the Kobo's database; the file's metadata (an EPUB's OPF; a MOBI/AZW3's header, record 0 only: EXTH 503 or the full name, EXTH 100 per author, in its UTF-8 or Windows-1252 text encoding; cached by path, size and date between listings); the title and author the manifest recorded when Librarium sent it; the file name (`Title - Author`, split at the *last* " - ", underscores read as spaces when there are no spaces). An MTP Kindle's files aren't opened: the manifest, else the file name. Sorted by path. An MTP Kindle is listed over MTP (rejects with `DEVICE` when it can't be). (Rejects only for MTP.) |
| `scan_device_library` | `devicePath` | `string[]` | File names of the same books (rejects like `list_device_books`). |
| `delete_device_book` | `path` | `void` | Rejects unless `path` is an ebook file (`INVALID_INPUT`) inside a currently connected device (`DEVICE`). Deletes the file (permanently: device volumes have no usable trash), then on a Kindle its `.sdr` folder (reading position, notes: `<name>.sdr` and `<stem>.sdr` next to it, inside `documents/`; `<stem>.sdr` is kept if another book with that stem is still there) and its home-screen thumbnail, then its manifest entry. An `mtp://` path is checked the same way (an ebook under `documents/`, no hidden folder, on a connected MTP Kindle). |
| `repair_device_covers` | `libraryPath, devicePath` | `number` | Writes the missing Kindle home-screen covers (see `send_books`) of the books the device manifest says were sent, from `libraryPath`'s covers: books whose cover write failed, or sent without one. Returns how many were written; 0 on a Kobo or an MTP Kindle. Never rejects. The device store runs it once per Kindle and library per session, after the first listing. |
| `eject_device` | `mountPath` | `void` | Safe eject. Rejects (`DEVICE`) unless `mountPath` is a connected e-reader (it never ejects another disk), and (`CONFLICT`) while books are being sent to it; `send_books` to it is refused meanwhile. macOS: `diskutil eject`. Linux: `udisksctl unmount`, else `gio mount -u`, then `udisksctl power-off` (best effort). 60 s timeout. Windows: not yet — rejects with `DEVICE` "use Safely Remove Hardware in the system tray". On failure the message gives the system's reason (a file open on the device…). An MTP Kindle is *released* (its session closed): it shows as `ejected` in `get_mtp_status`, out of `detect_devices`, until it is unplugged. |
| `get_device_reading_progress` | `devicePath, deviceType` | `Record<string, number>` | Keys are the **file name without its last extension** (`Dune` for `Dune.azw3`, `Book.kepub` for `Book.kepub.epub`), what the frontend looks up (`services/readingProgress.ts`, for the library view's shelf). Kobo: from a private copy of `KoboReader.sqlite` (WAL included), like `list_device_books`. Kindle: `.sdr` folders (`.mbs` + `.apnx`), the old-firmware `.mbp` files and `system/*.db`, all kept. MTP Kindle: the `.sdr` folders only, copied over MTP (cached; **unverified** on a device). Never rejects: a device that can't be read gives `{}`. |

```ts
interface KindleDevice {
  id: string               // stable per device (see "Device identity"); never a serial number
  name: string             // the model ("Kindle Paperwhite 5", "Kobo Clara HD"), else the volume name ("Kindle 1")
  mountPath: string        // '/Volumes/Kindle', 'E:\\', '/media/me/KOBOeReader', 'mtp://usb-20-1.2'
  deviceType: 'kindle' | 'kobo'
  connection: 'massStorage' | 'mtp'
  preferredFormat: string  // conversion target: 'azw3' (Kindle) | 'epub' (Kobo)
  model?: string
  freeSpaceBytes?: number  // as of this detection
  totalSpaceBytes?: number
}

interface DeviceBook {
  path: string             // absolute; 'mtp://usb-20-1.2/documents/Dune.azw3' on an MTP Kindle
  filename: string
  title: string            // Kobo database, else EPUB metadata, else "Title - Author" from the file name
  author: string
  libraryId?: string       // the library Book.id this file is; absent for books Librarium didn't send
  sizeBytes: number
  percentRead?: number     // Kobo only, 0–100
  readStatus?: 'unread' | 'reading' | 'finished'   // Kobo only
}
```

Events:

- `transfer-progress`: `{ jobId, status, progress, errorMessage? }` with
  `status` one of `queued | converting | transferring | done | error | cancelled`
  (`queued` is never emitted: the frontend sets it when it creates the job).
  Every job ends with exactly one `done`, `error` or `cancelled` event.
- `device-space`: `{ mountPath, freeSpaceBytes?, totalSpaceBytes? }`, after
  each job (whatever its outcome), so the free space shown is current
  without waiting for the next poll.

```ts
// Field names are snake_case (as the frontend has always sent them);
// the camelCase spellings are accepted too.
interface TransferRequest {
  id: string                 // job id, echoed in events and used by cancel_transfer
  book_id?: string           // the library Book.id — send it with library_path (one without the other fails the job)
  library_path?: string      // the library folder holding it (the current user's)
  book_path?: string         // only used without book_id/library_path (older frontends); otherwise the index's path is used
  needs_conversion: boolean
  target_format: string      // conversion target: 'epub' (Kobo) | 'azw3' (Kindle; 'mobi' also gives AZW3)
  device_path: string        // KindleDevice.mountPath; must contain documents/ or .kobo/
  subfolder?: string | null  // under documents/; relative, no '..', each part sanitised
  // Title-matched copy already on the device (may be another book!). Only
  // used to put the new file in the same folder (keeps Kindle's .sdr), only
  // if that folder is inside the device, and only when the manifest doesn't
  // know where the book is. Never deleted. Not needed with book_id.
  existing_device_path?: string | null
}
```

### Device identity (`<mount>/.librarium/device.json`)

Librarium keeps a manifest on each device it writes to. It is what makes
replacing and deleting safe without matching titles:

```jsonc
{
  "version": 1,
  "deviceId": "<uuid>",                     // KindleDevice.id
  "books": {
    "<library Book.id>": {
      "path": "documents/Dune.azw3",        // relative to the mount, '/' separators
      "title": "Dune", "author": "Frank Herbert",
      "format": "azw3", "sizeBytes": 123456,
      "sentAt": "2026-11-06T10:00:00+00:00"
    }
  }
}
```

- Written atomically after each successful transfer and delete, and once
  when a device is first seen (its `deviceId`). A device that can't be
  written to (read-only, damaged manifest) gets the id `"<type>:<mountPath>"`.
- Paths are compared case- and Unicode-normalisation-insensitively (FAT and
  exFAT don't distinguish `Dune.azw3` from `dune.azw3`). Entries whose path
  is absolute or contains `..`, or whose key isn't a valid id, are ignored;
  every path is checked to be inside the device again before anything is
  deleted.
- A manifest written by a newer Librarium (`version` > 1) is read but never
  rewritten (transfers still work; they just aren't recorded). A damaged one
  is moved aside (`device.json.damaged-<time>`) before a new one is written.
  Unknown fields are kept.
- The id is also inside the files Librarium sends, so it survives a lost
  manifest: an AZW3 converted for a Kindle has it as its ASIN (EXTH 113,
  cdetype `EBOK`; Kindle names home-screen thumbnails after it); an EPUB
  copy gets an extra `<dc:identifier id="librarium-id">urn:librarium:<id></dc:identifier>`
  (the package's unique identifier is never changed; an EPUB whose OPF
  can't be edited safely goes as it is, logged). Books sent in their own
  format (AZW3, MOBI, PDF, CBZ from the library) are not modified: only the
  manifest knows them.

### MTP Kindles (2024+, experimental)

Kindles from 2024 on don't mount as a USB drive: they speak MTP.
Librarium talks MTP itself (`src-tauri/src/mtp/`: raw USB with nusb on
macOS and Linux, Windows Portable Devices on Windows), **only when the
`mtpEnabled` setting is on** (off by default). Nothing here has been
validated on a real 2024 Kindle yet (`TODO(device-validation)` in the code).

**How they appear.** A connected MTP Kindle is a `KindleDevice` like any
other, with `connection: "mtp"`, `deviceType: "kindle"`, `preferredFormat:
"azw3"` and `mountPath: "mtp://<port>"` (`mtp://usb-20-1.2`,
`mtp://wpd-1`): where it is plugged in, never who it is (no serial number,
in paths, ids, names or logs). Its `id` is the UUID in its device manifest,
like a drive's (`kindle:mtp://<port>` if the manifest can't be written).
Its books' `DeviceBook.path` is `mtp://<port>/documents/…`. Every device
command accepts these paths: `list_device_books`, `scan_device_library`,
`send_books` (same `transfer-progress` events, byte progress, cancel),
`delete_device_book`, `import_books`, `get_device_reading_progress`,
`eject_device`, and `device-space` events carry the `mtp://` mount path.

**Which devices.** Amazon's USB vendor id (0x1949) with an MTP/PTP
interface (Windows: a WPD device whose PnP id is an Amazon USB device);
Fire tablets and mass-storage Kindles are left alone.

**Why one isn't usable.** `detect_devices` lists only connected ones;
`get_mtp_status` lists all of them with a state and, unless ready, a
`message` saying what to do (English; the UI may translate by `state`):

| `state` | Meaning | UI |
|---|---|---|
| `ready` | Connected: also in `detect_devices`. | — |
| `connecting` | Opening the session (first plug, or reconnecting after an error). | Spinner. |
| `mtpDisabled` | MTP support is off; nothing was opened. | "A Kindle using MTP is connected — enable experimental MTP support in Settings", with a link to the setting. |
| `claimFailed` | Another program holds the Kindle (macOS: Image Capture/Photos/Android File Transfer/OpenMTP, `ptpcamerad`; Linux: gvfs-mtp, kio-mtp), or no permission (Linux without the udev rule `70-librarium-kindle.rules`). Retried every 10 s. | Show `message`. |
| `busy` | The Kindle keeps answering "busy". Retried. | Show `message`. |
| `error` | Anything else (the message says what). Retried. | Show `message`. |
| `ejected` | Released by `eject_device`: safe to unplug; stays so until unplugged. | "Safe to unplug". |

```ts
interface MtpStatus {
  enabled: boolean           // the mtpEnabled setting
  devices: MtpDevice[]
}

interface MtpDevice {
  mountPath: string          // 'mtp://usb-20-1.2' (KindleDevice.mountPath once ready)
  name: string               // the model once connected, else the USB product name ('Kindle')
  state: 'ready' | 'connecting' | 'mtpDisabled' | 'claimFailed' | 'busy' | 'error' | 'ejected'
  message?: string           // what to do / what went wrong; absent when ready
  deviceId?: string          // KindleDevice.id, when ready
}
```

**How it behaves** (differences from a drive):
- One worker thread per device owns its session, so operations on one
  device run one at a time (a listing waits for the transfer in progress);
  detection never waits. After a USB error the session is dropped and
  reopened; partial files left by a failed or cancelled transfer are
  deleted once reconnected.
- Replacing a book can't be atomic over MTP: the new copy is written under a
  hidden temp name, then the old one is deleted and the new one renamed;
  on a Kindle that can't rename, the old copy is deleted first (if the copy
  then fails, send it again; the library always has the book).
- Titles come from file names (`Title - Author.ext`), and `libraryId` from
  the device manifest only: reading each file would mean downloading it.
- The device manifest is kept as two copies, `.librarium/device-1.json` and
  `device-2.json` (same content as `device.json` on a drive, plus a
  `generation` counter), written alternately, so an interrupted write never
  loses it.
- Reading progress: the `.sdr` strategy on local copies of the device's
  `.sdr` files, re-downloaded only when they change. Whether 2024 Kindles
  expose `.sdr` over MTP is unverified.

## Network

| Command | Arguments | Returns | Notes |
|---|---|---|---|
| `http_get` | `url, headers?: Record<string,string>, maxBytes?: number` | `{ status, contentType: string \| null, body: string, finalUrl: string }` | For hosts the webview's CSP doesn't allow (user OPDS catalogs, archive.org). `http`/`https` only, 20 s timeout (overall, and for silences), 5 MB default cap (`maxBytes` is clamped to 1 B–20 MB). Invalid header names/values reject; `Host` is ignored. `body` is decoded as UTF-8 (invalid bytes become U+FFFD). Non-2xx statuses resolve; network errors, timeouts and oversized bodies reject. Private/LAN addresses are allowed (a Calibre server on the LAN is a valid catalog). |

Every request the backend makes (this proxy, cover and book downloads) goes
through one shared client (`src-tauri/src/net.rs`): reqwest 0.13 on rustls
(the same stack as the updater), certificates checked against the OS store,
15 s connect timeout, at most 10 redirects, User-Agent `Librarium/<version>`,
and per-request limits (overall deadline, idle timeout, byte cap enforced
while streaming). Error messages name the host, never the full URL (query
strings can carry keys).

## OPDS server

| Command | Arguments | Returns | Notes |
|---|---|---|---|
| `start_opds` | `port, libraryRoot, shareOnLan?: boolean, password?: string \| null` | `OpdsStatus` | Binds `127.0.0.1` unless `shareOnLan` (default `false`, so callers that omit it get local-only mode). In LAN mode, HTTP Basic auth is required on every request, loopback included (user `librarium`, the given password; rejects if empty or blank). No CORS headers. The Host header must be `localhost` or a loopback IP, or — in LAN mode — the IP the client connected to: LAN clients must use an address from `lanUrls`, not a host name like `mymac.local`. Rejects if `libraryRoot` isn't a folder (`LIBRARY_UNAVAILABLE`), the password is missing in LAN mode (`INVALID_INPUT`) or the port is taken (`CONFLICT`). Restarts only when the settings changed (waiting for the old server to release the port); always clears the catalog cache. |
| `stop_opds` | — | `void` | Waits (up to 3 s, then aborts) until the port is released. |
| `opds_status` | — | `OpdsStatus \| null` | `null` when never started or stopped; `running: false` if the server task died. |
| `invalidate_opds_cache` | — | `void` | Makes a library change visible to readers at once. Optional: the server also notices changes to the index and to `.covers/` by itself (checked at most once a second). |
| `opds_qr_svg` | `url: string` | `string` | SVG markup of a QR code of `url` (meant for a `lanUrls` entry): black on white with a quiet zone, ≥ 256 px, scalable (`viewBox`). Rejects empty text or more than 512 bytes (`INVALID_INPUT`). Show it as an image (`<img src="data:image/svg+xml;charset=utf-8,…">`), not with `v-html`. |

```ts
interface OpdsStatus { running: boolean; port: number; localUrl: string; lanUrls: string[] }
```

`localUrl` is `http://127.0.0.1:<port>/opds`. `lanUrls` is empty in local mode,
and also in LAN mode when the machine has no network route.

### What to type in a reading app

The catalog address is the `localUrl` (an app on this computer) or a
`lanUrls` entry (a phone, tablet or e-reader on the same network, with
"Share on local network" on), e.g. `http://192.168.1.20:8042/opds`. Use the
IP address shown: host names such as `mymac.local` are refused (that is what
protects the catalog from web pages). In LAN mode the app asks for a user
name and password: `librarium` and the password shown in Settings › OPDS server.

| App | Where | Notes |
|---|---|---|
| KOReader (Kobo, Kindle, PocketBook, Android) | Search ▸ OPDS catalog ▸ `+` | Catalog URL, Username, Password. Search appears as its own item. |
| Moon+ Reader, Librera, Aldiko, FBReader | "Net library" / "OPDS catalogs" ▸ add | Use the same URL. |
| Thorium Reader (desktop) | Catalogs ▸ Add | Reads OPDS 1.2 and 2.0; `/opds/v2` gives the JSON catalog. |

`http://<ip>:<port>/` (without `/opds`) serves the same root, for apps that
only ask for a server address.

### Feeds and files served

Both OPDS versions share the book files, covers and thumbnails. Feeds link
with path-absolute URLs (`/opds/all`), so they work under any address.

| Path | Content | Type |
|---|---|---|
| `/opds` (also `/`, `/opds/`) | Root: All books, Recently added, By author, By series; `rel="search"` links | Atom navigation feed, or OPDS 2 JSON if `Accept` rates `application/opds+json` (or `application/json`) above Atom; `Vary: Accept` |
| `/opds/all` | Every book, by title | Atom acquisition feed |
| `/opds/recent` | Every book, newest first (`rel="http://opds-spec.org/sort/new"` from the root) | Atom acquisition feed |
| `/opds/authors` | Authors (`"A & B"` counts as two), with book counts (`thr:count`) | Atom navigation feed |
| `/opds/authors/{name}` | One author's books, series grouped and in order | Atom acquisition feed |
| `/opds/series` | Series, with book counts | Atom navigation feed |
| `/opds/series/{name}` | One series, by series index | Atom acquisition feed |
| `/opds/search?q=` | Books whose title, subtitle, authors, series or ISBN contain every word of `q` (case- and accent-insensitive; first 200 characters, 10 words) | Atom acquisition feed |
| `/opds/opensearch.xml` | OpenSearch description; template `/opds/search?q={searchTerms}` | `application/opensearchdescription+xml` |
| `/opds/books/{id}` | One book | Atom entry document |
| `/opds/v2` | OPDS 2 root | `application/opds+json` |
| `/opds/publications?q=` | Every book, or the search results (same search) | `application/opds+json` |
| `/opds/publications/{id}` | One publication | `application/opds-publication+json` |
| `/opds/publications/{id}/download` | The book file | per format, below |
| `/opds/publications/{id}/cover` | `.covers/<id>.(jpg\|png\|webp)`, else the thumbnail | sniffed from the bytes |
| `/opds/publications/{id}/thumbnail` | `.covers/thumbs/<id>.jpg`, else the full cover | sniffed from the bytes |

`{name}` is percent-encoded (`AC%2FDC`) and compared case- and
accent-insensitively. Unknown names and ids give `404`.

**Pagination.** Every feed but the root, the OpenSearch description and
single entries is paginated: `?page=N` (from 1) and `?limit=N` (default 50,
capped at 200; nonsense values fall back to the defaults). Atom feeds carry
`first`/`previous`/`next`/`last` links and `opensearch:totalResults`,
`itemsPerPage`, `startIndex`; the OPDS 2 feed has the same links and
`numberOfItems`, `itemsPerPage`, `currentPage`. A page past the end is empty,
not an error.

**Entries** (Atom) carry `title` (the file name when the book has none),
`id` (`urn:uuid:<book id>`), `updated` and `published` (date added), one
`author` per author, `dc:language`, `dc:identifier` (`urn:isbn:…`),
`calibre_md:series` / `calibre_md:series_index`, one `category` per category,
`summary type="text"` (HTML stripped), cover and thumbnail links
(`http://opds-spec.org/image`, `…/image/thumbnail`), the acquisition link
(`http://opds-spec.org/acquisition`, typed per format, with `length`), an
`alternate` link to the entry document and `related` links to the author
and series feeds. All text is escaped by the XML writer; characters XML
forbids (control characters) are dropped.

**Downloads** are streamed from disk (never read whole into memory), with
`Content-Length`, `Accept-Ranges: bytes`, single `Range` requests (`206`,
`416` when unsatisfiable or multi-range), `Last-Modified` /
`If-Modified-Since`, `HEAD`, and `Content-Disposition: attachment` with the
file name (RFC 6266: ASCII `filename` plus UTF-8 `filename*`). Types:
EPUB `application/epub+zip`, PDF `application/pdf`, MOBI/AZW
`application/x-mobipocket-ebook`, AZW3 `application/vnd.amazon.mobi8-ebook`,
CBZ `application/vnd.comicbook+zip`. Only files inside the library folder
are served (symlinks resolved); covers only from `.covers/`.

**Limits.** At most 64 open connections (more wait in the OS queue); a
connection with no byte moving either way for 60 s is closed (stalled
downloads, sleeping e-readers, idle keep-alives); at most 16 requests are
answered at once, a download counting until it ends (more wait up to 10 s,
then `503` with `Retry-After: 5`); a response not started within 30 s is a
`503`. Requests refused by the Host or password check never take a slot.

**Freshness.** The catalog is built from the index (read only, never
written: `library.json`, its backup, or an older `books.json`) and one
listing of `.covers/` and `.covers/thumbs/`. It is rebuilt after
`invalidate_opds_cache`, on every (re)start, and by itself when one of those
files or folders changes size, modification time or identity, which the
server checks at most once a second.
