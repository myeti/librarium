# Librarium — Specification

Tauri v2 + Vue 3 desktop app to manage an ebook library and send books to Kindle or Kobo.

---

## Tech Stack

| Layer     | Technology                                                          |
|-----------|---------------------------------------------------------------------|
| Frontend  | Vue 3.5 (Options API via `defineComponent`), TypeScript 5           |
| Bundler   | Vite 8                                                              |
| State     | Pinia 2, options stores only                                        |
| Router    | Vue Router 4                                                        |
| i18n      | Own module `src/i18n` (typed keys, plurals with `Intl.PluralRules`); English and French. No vue-i18n |
| Styling   | SCSS (`lang="scss" scoped`), CSS design tokens (`assets/themes.css`, `assets/main.css`), UI kit (`components/ui`); fonts bundled (`@fontsource`: Inter for the interface; Cinzel, Poiret One, Fraunces, Italiana and Cormorant Garamond for the themes' lettering) |
| Icons     | `vue-feather` (globally registered)                                 |
| EPUB reader | epub.js 0.3.93 (`@xmldom/xmldom` pinned to a patched 0.8 by `overrides`); PDFs in the web view's own viewer |
| Tests     | vitest + happy-dom (`*.test.ts` next to the code); Rust: `cargo test` |
| Backend   | Rust (toolchain pinned in `rust-toolchain.toml`), Tauri 2.12        |
| Tauri plugins | dialog, updater, opener (links, reveal), process (relaunch), log (log file), single-instance |
| Backend libraries | boko 0.6.0, vendored and patched (`src-tauri/vendor/boko`: conversion); quick-xml (EPUB OPF); lopdf (PDF metadata, covers); zip; image (thumbnails); axum 0.8 + tower-http (OPDS server); reqwest 0.13 on rustls (HTTP); rusqlite 0.41, bundled SQLite (Calibre, Kobo); nusb 0.2 (MTP on macOS/Linux) and windows 0.62 (WPD, MTP on Windows); trash; whatlang (language detection) |
| IPC       | Typed wrappers in `src/services/api.ts` (over `invoke`); events via `listen` — contract in `docs/IPC.md` |
| Analytics | Umami, opt-in (see "Feature: Analytics" and PRIVACY.md)           |
| Tooling   | Node ≥ 20.19 (`.nvmrc`), ESLint (`eslint-plugin-vue` + type-checked `typescript-eslint`), vue-tsc, clippy, cargo-deny |

---

## Conventions (frontend)

**Options API everywhere.** ESLint enforces the
component part (`vue/component-api-style: options`); the rest is review.

- **Components** are `defineComponent({ props, emits, data, computed, watch,
  methods, lifecycle hooks })`. No `setup()`, no `<script setup>`, no
  `ref()`/`computed()`/`onMounted()` inside a component. Give `data()` an
  explicit return type when a field holds a union or a nullable value
  (`data(): { step: Step, … }`) rather than `'pick' as Step` casts.
- **Stores** are Pinia *options* stores: `defineStore('id', { state, getters,
  actions })`, with an interface typing `state` (`state: (): DeviceState => …`).
  No setup stores.
- **Reaching stores from components:** `...mapStores(useXStore)` in
  `computed` (then `this.xStore.foo`), or `mapState` / `mapActions` for a few
  fields or actions. Don't call `useXStore()` in `data()`, lifecycle hooks or
  methods. Stores and services may call `useXStore()` (that is how they talk
  to each other).
- **Shared logic** goes in a store (shared *state*), a plugin (app-wide
  template helpers such as `$t`), or a plain module of functions under
  `src/utils/` (pure helpers: formatting, collation, ISBNs, debounce, sounds,
  the fly-to-device animation) or `src/services/` (talking to the outside:
  the backend, metadata providers, analytics). No composables (`useXxx()`
  functions holding refs).
- **Backend calls** only through `src/services/api.ts` (one typed function
  per command; `scripts/check-ipc.mjs` checks them against the Rust
  signatures). Errors are read with `errorMessage()` / `errorCode()` from
  `src/utils/errors.ts`, never `String(e)`.
- **Text** shown to the user comes from `src/i18n` (`$t` in templates, `t` in
  code), with keys checked at compile time; counts use plural messages
  (`{ one, other }` with `n`), never "(s)". Dates and language names go
  through `src/utils/format.ts`, text sorting through `src/utils/collator.ts`.
- **UI:** screens are built from the kit in `src/components/ui` (see "UI
  kit"): icon-only buttons are `IconButton`, dialogs are `BaseModal`, menus
  are `DropdownMenu`, global shortcuts go through the shortcuts store.
  Colours, sizes and spacing come from the tokens ("Design System"); the
  contrast test (`themes.test.ts`) must keep passing.
- **Formatting:** 4-space indent, no semicolons, braces on every block
  (`npm run lint:fix` fixes most of it).
- **Promises:** never leave one floating (`@typescript-eslint/no-floating-promises`):
  `await` it, or write `void promise` when it is deliberately fire-and-forget
  and handles its own errors.

---

## Project Structure

```
librarium/
├── index.html
├── vite.config.ts / vitest.config.ts / eslint.config.js
├── tsconfig.json / tsconfig.node.json
├── package.json / package-lock.json  # the single version source (tauri.conf.json reads it)
├── .nvmrc / rust-toolchain.toml      # Node and Rust versions used by CI
├── .env.example / .env.mock          # build-time settings (VITE_*); the mock-backend mode
├── deny.toml                         # cargo-deny: advisories, allowed licences (boko is the one GPL exception), sources
├── README.md, CHANGELOG.md, PRIVACY.md, COMPATIBILITY.md, DEPLOY.md, SPECS.md
├── THIRD_PARTY_NOTICES.md            # Generated by `npm run notices`
├── LICENSE                           # Proprietary — Magalon, all rights reserved
├── docs/IPC.md                       # The frontend ↔ backend contract
├── docs/DESIGN.md, docs/GUIDE.md     # The design; every feature, installing, development (the README stays short)
├── docs/images/                      # The README's screenshot (mock backend, `?showcase=<user>`)
├── scripts/                          # bump, release, versions (check), check-ipc, notices, tauri (the CLI with the build's servers in the CSP); ci/ helpers for the release workflow
├── .github/workflows/                # ci.yml (checks), release.yml (build, sign, publish) — see DEPLOY.md
├── src/
│   ├── main.ts                       # App entry point
│   ├── App.vue                       # Root layout (sidebar + router-view + DebugPanel)
│   ├── assets/
│   │   ├── main.css                  # Scales, base styles, global classes (@import themes.css)
│   │   ├── themes.css                # Tokens of the 4 (dark) themes: colours, fonts, ornaments, cover cloths
│   │   ├── themes.test.ts            # WCAG contrast of every theme, read from the CSS
│   │   ├── sounds/                   # UI sound effects (intro, new-user, transfer-done)
│   │   └── icons/
│   │       ├── library.svg           # App icon source: Nocturne's moon (also the About modal's and the favicon)
│   │       ├── library-square.svg
│   │       ├── divider-top.svg
│   │       └── divider-bottom.svg
│   ├── types/index.ts                # Shared TypeScript types
│   ├── router/index.ts               # Vue Router: /library, /discover, /settings (+ redirects from /options, /store, /transfers, /device)
│   ├── stores/
│   │   ├── library.ts                # Pinia: books, filters, sort, selection, view mode
│   │   ├── bookActions.ts            # Pinia: library screen actions and their dialogs (edit, send, tidy up, trash…)
│   │   ├── imports.ts                # Pinia: imports (picker, ⌘O, files dropped on any view), one at a time
│   │   ├── device.ts                 # Pinia: device, transfer queue, reading progress
│   │   ├── users.ts                  # Pinia: current user / user switching
│   │   ├── settings.ts               # Pinia: app-wide settings (app config): sounds, effects, folders, keys, MTP
│   │   ├── saveStatus.ts             # Pinia: "Saved" / "Not saved: why" feedback of the Settings rows
│   │   ├── system.ts                 # Pinia: live OS preferences (prefers-color-scheme, prefers-reduced-motion)
│   │   ├── theme.ts                  # Pinia: the theme (older names read as their successors); applies data-theme on <html>
│   │   ├── opds.ts                   # Pinia: OPDS server enabled/port/running state
│   │   ├── opdsSources.ts            # Pinia: user-registered OPDS catalog sources (config `opdsCatalogs`)
│   │   ├── notifications.ts          # Pinia: in-app toast notifications (push/dismiss)
│   │   ├── modals.ts                 # Pinia: stack of open dialogs (top one gets the keys)
│   │   ├── shortcuts.ts              # Pinia + plugin: the one global keydown listener
│   │   ├── analytics.ts              # Pinia: analytics consent (opt-in; wraps the analytics service)
│   │   ├── calibre.ts                # Pinia: Calibre import (progress events, cancel, one at a time)
│   │   ├── updates.ts                # Pinia: in-app updater (check, install, relaunch) and "What's new" messages
│   │   └── log.ts                    # Pinia: debug log entries (console + Rust events; secrets redacted)
│   ├── i18n/
│   │   ├── index.ts                  # t() / $t: typed keys, plurals (Intl.PluralRules), categoryLabel()
│   │   ├── types.ts                  # Message tree, plural message and key types
│   │   ├── en.ts                     # English strings (the reference: defines the keys)
│   │   └── fr.ts                     # French strings (same keys, plus category names)
│   ├── constants/
│   │   ├── categories.ts             # Book category list
│   │   └── shortcuts.ts              # Every command with a shortcut or a menu item (keys, description, group): the single list
│   ├── services/
│   │   ├── api.ts                    # One typed function per Tauri command (the only `invoke` caller)
│   │   ├── analytics.ts              # Usage analytics (Umami, opt-in): consent flag, queue, bucketing helpers
│   │   ├── librarium-api.ts          # The Librarium API: "What's new" messages, error reports (consent-gated), feedback
│   │   ├── privacy.ts                # Scrubbing of outgoing data: paths/e-mails in error messages, serial-like device models
│   │   ├── appConfig.ts              # get_config/set_config with an in-memory cache; the AppConfig keys
│   │   ├── http.ts                   # httpGet/httpGetJson over the backend's `http_get` (hosts the CSP doesn't allow)
│   │   ├── reader.ts                 # open_for_reading + asset URL: what the reader displays
│   │   ├── duplicates.ts             # Duplicate detection (ISBN-13, Unicode-aware title/author keys)
│   │   ├── duplicateReview.ts        # Duplicates review: per-copy comparison, suggested copy to keep
│   │   ├── deviceMatch.ts            # "On device" / "in library" matching (library id first, then title + author, then file name)
│   │   ├── shelf.ts                  # The library's books and the e-reader's, merged: presence, scope, filters, sort
│   │   ├── tidyup/                   # Metadata lookup (Google Books, Open Library): timeouts, retries, ISBN → title fallback; batch.ts: unreviewed batch rules
│   │   ├── readingProgress.ts        # Device reading progress per library book (by library id first)
│   │   └── store/                    # Discover's catalogs (merge.ts: one result per book, best matches first)
│   │       ├── providers/
│   │       │   ├── gutendex.ts       # Project Gutenberg, through gutendex.com
│   │       │   ├── openLibrary.ts    # Open Library search; download links from archive.org (through `http_get`)
│   │       │   ├── googleBooks.ts    # Google Books free ebooks (not registered in STORE_PROVIDERS: unused)
│   │       │   └── opdsCatalog.ts    # Factory: a StoreProvider from an OpdsCatalogSource (through `http_get`)
│   │       ├── index.ts              # STORE_PROVIDERS static array (gutendex + openLibrary)
│   │       └── types.ts              # StoreBook, StoreProvider interfaces
│   ├── utils/
│   │   ├── format.ts                 # fmtDate, fmtTime, fmtSize, fmtLanguage (Intl, UI language)
│   │   ├── collator.ts               # Shared Intl.Collator: locale-aware, numeric, case-insensitive sorting
│   │   ├── debounce.ts               # Debounced functions (search fields)
│   │   ├── dom.ts                    # Focusable elements, typing detection, unique ids
│   │   ├── isbn.ts                   # ISBN cleaning, check digits, ISBN-10 ↔ 13
│   │   ├── sounds.ts                 # Sound effects
│   │   ├── flyAnimation.ts           # "Flying cover" animation when sending to the device
│   │   ├── virtualWindow.ts          # Virtual scrolling maths of the library grid/list (pure)
│   │   ├── collectionNav.ts          # Keyboard / click / range-selection model of the library collection (pure)
│   │   └── …                         # errors, covers, bookDisplay, text, safeText, taskQueue, visibility, mtp (mtp:// locators)…
│   ├── components/
│   │   ├── ui/                       # UI kit: BaseButton, IconButton, BaseModal, DropdownMenu, fields… (see "UI kit")
│   │   ├── AppSidebar.vue            # Sidebar: nav (Device explains itself when nothing is connected), Activity, device block, user chip
│   │   ├── ActivityPopover.vue       # Sidebar item + popover: transfers to/from the e-reader (progress, cancel, clear finished)
│   │   ├── AppCommands.vue           # App-level commands (shortcuts + menu bar events), About and Keyboard shortcuts dialogs
│   │   ├── ShortcutsDialog.vue       # Help › Keyboard shortcuts (⌘/), built from constants/shortcuts.ts
│   │   ├── settings/                 # Settings sections (LibrarySettings, DeviceSettings…), SettingsSection, SaveStatus, ChoiceGroup, sections.ts
│   │   ├── effects/                  # BackgroundEffects + MagicParticles, FirefliesLayer, StarfieldLayer (utils/effects: CanvasLoop, scenes)
│   │   ├── library/                  # The library screen's parts (see "Library View")
│   │   │   ├── BookCollection.vue    # Virtualised, keyboard-driven listbox of books (grid or list)
│   │   │   ├── LibraryGrid.vue       # BookCollection + BookCard (GRID_LAYOUT)
│   │   │   ├── LibraryList.vue       # BookCollection + BookRow + sticky column header (LIST_LAYOUT)
│   │   │   ├── BookRow.vue           # One list row
│   │   │   ├── BookCover.vue         # Cover or placeholder; lazy cover request
│   │   │   ├── SelectCheckbox.vue    # The card/row selection checkbox
│   │   │   ├── BookContextMenu.vue   # Right-click / ContextMenu-key menu (DropdownMenu)
│   │   │   ├── FilterPopover.vue     # Filter fields under the filter button
│   │   │   ├── BookDetailModal.vue   # A book's details, a dialog over the books: cover + send/read, details, actions
│   │   │   ├── BookDetailHeader.vue  # Details: series, title, author, badges
│   │   │   ├── BookMetadataTable.vue # Details: language, series, ISBN, size, added, progress (a grid)
│   │   │   ├── BookDetailActions.vue # Details' action bar: edit, find metadata, location, export ▾, more ▾ + dialogs
│   │   │   ├── DeviceFileDetails.vue # Details of a file only on the e-reader
│   │   │   ├── FavoriteStar.vue      # The star of a card or row
│   │   │   ├── LibraryDialogs.vue    # The dialogs the bookActions store opens
│   │   │   ├── FileDropTarget.vue    # App-wide file drops (mounted by App.vue) + EPUB conversion offer
│   │   │   └── collection.ts         # Shared types and layouts (GRID_LAYOUT, LIST_LAYOUT)
│   │   ├── BookCard.vue              # Grid card: cover, then series / title (2 lines) / author, always visible
│   │   ├── ReaderModal.vue           # Full-screen ebook reader (EPUB device frame + PDF iframe)
│   │   ├── FilterSortBar.vue         # Search, filters, sort menu, grid/list toggle, duplicates
│   │   ├── DropZone.vue              # Full-screen drag-and-drop overlay (any view)
│   │   ├── DuplicatesModal.vue       # Duplicate detection modal
│   │   ├── MetadataModal.vue         # Single/batch metadata editor modal (batch: books in an order, volumes numbered in it)
│   │   ├── TidyupReviewModal.vue     # Field-picker modal for single-book tidy-up
│   │   ├── EpubConvertModal.vue      # Modal to convert MOBI/AZW/AZW3 library books to EPUB
│   │   ├── SelectionToolbar.vue      # Replaces the filter bar while books are selected
│   │   ├── AppNotifications.vue      # Toast overlay (TransitionGroup, auto-dismiss, error/info/success)
│   │   ├── DebugPanel.vue            # Ctrl+Shift+D floating log panel (release-enabled)
│   │   └── UserPicker.vue            # User selection screen (each avatar in the user's colour)
│   └── views/
│       ├── LibraryView.vue           # Main library page (/library)
│       ├── DiscoverView.vue          # Free-ebook catalogs (/discover; /store redirects)
│       └── SettingsView.vue          # Settings, one section at a time (/settings#section; /options redirects)
└── src-tauri/
    ├── Cargo.toml                    # Dependencies (each one commented), release profile (panic = "unwind", opt-level 3, split debug info)
    ├── tauri.conf.json               # Version read from ../package.json; CSP; asset protocol (empty scope); bundles; updater key
    ├── capabilities/default.json     # core, dialog, updater, opener (http/https/mailto URLs), process:allow-restart
    ├── build.rs
    ├── icons/                        # Generated by `npx tauri icon src/assets/icons/library.svg`
    ├── linux/70-librarium-kindle.rules  # udev rule for MTP Kindles, installed by the .deb
    ├── vendor/boko/                  # boko 0.6.0, vendored; dated "Modified for Librarium" notes (KF8 chunking, per-book ASIN)
    └── src/
        ├── main.rs
        ├── lib.rs                    # Plugins, managed state, the command list (inspect_epub in debug builds only)
        ├── error.rs                  # AppError: { code, message } for every command (codes in docs/IPC.md)
        ├── logging.rs                # Debug panel events (`app:log`) + Librarium.log (tauri-plugin-log)
        ├── fsutil.rs                 # Atomic writes (temp + fsync + rename), sync_file (F_FULLFSYNC → fsync fallback), per-file locks, trash with fallback (macOS: NSFileManager), FAT-safe names
        ├── net.rs                    # The one HTTP client (reqwest + rustls): timeouts, redirect and byte caps
        ├── convert.rs                # Every conversion (boko): DRM refused, output validated, temp files cleaned
        ├── epub/                     # EPUB read/edit with quick-xml: container, OPF model, metadata, covers, ISBNs, the librarium id
        ├── mtp/                      # MTP transport for 2024+ Kindles: protocol (pure), ptp session, usb (nusb), wpd (Windows), worker thread per device, fake device for tests
        ├── models/
        │   ├── book.rs               # Book, BookMetadata, BookFormat
        │   ├── device.rs             # KindleDevice (Kindles and Kobos), TransferRequest
        │   └── mod.rs
        └── commands/
            ├── library.rs            # Library commands (the IPC layer) …
            ├── library/              # … and their modules: index (library.json), migrate (older index format), relkey, lock, scan,
            │                         #   failed (failed.json), import, sniff (magic bytes), metadata, edit, epub_edit,
            │                         #   covers, convert, files, users, os (reveal, log folder), inspect (debug)
            ├── device/               # USB-drive e-readers: detect, identity (id inside copies), manifest (device.json),
            │                         #   transfer, listing, kobo (KoboReader.sqlite), eject, state; mtp/ (MTP Kindles)
            ├── progress.rs           # Reading progress: Kobo database, Kindle .sdr (.mbs + .apnx), old .mbp, system/*.db
            ├── calibre.rs, calibre/  # Calibre import: detect (global.py.json), db (metadata.db, read-only), import
            ├── reader.rs, reader/    # open_for_reading (asset protocol); cache.rs: converted-EPUB cache
            ├── opds.rs, opds/        # OPDS server (axum): catalog cache, atom (1.2), json (2.0), files (streamed), access (Host + Basic auth), limits, urls, qr
            ├── http.rs               # http_get: GET proxy for hosts the CSP doesn't allow
            ├── config.rs             # get_config / set_config: librarium-config.json in the app data folder
            └── mod.rs
```

---

## Data Models

The reference is docs/IPC.md ("Shared types", "Device commands"); the
frontend types are in `src/types/index.ts`, the Rust ones in
`src-tauri/src/models/`. In short:

### Book (Rust → TypeScript)

```typescript
interface Book {
  id: string            // UUID v4, stable: kept by rename, conversion, reset, re-link of a missing book
  path: string          // absolute over IPC; relative to the library folder inside library.json
  filename: string
  format: BookFormat    // 'epub' | 'pdf' | 'mobi' | 'azw3' | 'cbz' | 'unknown'  (.azw files are classified as 'mobi')
  sizeBytes: number
  dateAdded: string     // ISO 8601
  coverPath?: string    // output only, never stored: the thumbnail (or full cover) to show, see "Covers" in docs/IPC.md
  coverVersion?: number // output only: the cover file's mtime in ms (cache buster)
  favorite?: boolean    // starred (set_favorite): listed first. In the index only, never in the file; absent when not
  metadata: BookMetadata
}

interface BookMetadata {
  title: string         // '' when unknown: the UI shows "Unknown title" (localised); never a stored placeholder
  author: string        // '' when unknown; several authors joined with ' & '
  subtitle?: string
  series?: string
  seriesIndex?: number
  language?: string     // ISO 639-1
  description?: string  // plain text
  isbn?: string
  rating?: number       // 0–5
  categories?: string[] // the backend always sends an array
  goodreadsUrl?: string
}
```

Batch edits send a `BookMetadataPatch` (every field optional; an absent
field is left as is), and optionally `seriesIndexes` (book id → volume):
the batch dialog lists the selected books on the left (`BatchOrderList`:
series by name and volume, then the others by title; reordered by
dragging a handle, or ↑ ↓ Home End on a focused handle, moves announced)
beside the form, and "Number the volumes in this order" (enabled once a
series is filled in) gives each book its place as its volume, from a
"First volume" number (1 by default; 0 or 2.5 allowed). `publisher` and `publishedDate` were intentionally
removed — not surfaced in the UI. `tags` was replaced by `categories`
(predefined list from `src/constants/categories.ts`; Calibre tags are
imported as categories).

### ConnectedDevice (Rust `KindleDevice`, which also describes Kobos)

```typescript
interface ConnectedDevice {
  id: string               // UUID kept in <mount>/.librarium/device.json; "<type>:<mountPath>" when the device can't be written to. Another id at the same mountPath = a swap
  name: string             // the model, else the volume name ("Kindle 1"); never a serial number
  mountPath: string        // '/Volumes/Kindle', 'E:\\', '/media/me/KOBOeReader', or 'mtp://usb-20-1.2' for an MTP Kindle
  deviceType: 'kindle' | 'kobo'
  connection: 'massStorage' | 'mtp'
  preferredFormat: string  // the conversion target: always 'azw3' for a Kindle, 'epub' for a Kobo
  model?: string           // "Kindle Paperwhite 5" (guessed from the firmware), "Kobo Clara HD"
  freeSpaceBytes?: number
  totalSpaceBytes?: number
}
```

### DeviceBook

```typescript
interface DeviceBook {
  path: string           // absolute; an mtp:// locator on an MTP Kindle (never shown)
  filename: string
  title: string          // Kobo database, else EPUB metadata, else "Title - Author" from the file name
  author: string
  libraryId?: string     // the library Book.id (books Librarium sent): "on device" matches by it first
  sizeBytes: number
  percentRead?: number   // Kobo only (KoboReader.sqlite)
  readStatus?: 'unread' | 'reading' | 'finished'   // Kobo only
}
```

### TransferJob (frontend only, not persisted)

```typescript
interface TransferJob {
  id: string
  direction: 'to-device' | 'from-device'
  book?: Book           // present for to-device jobs
  deviceTitle?: string  // present for from-device jobs
  status: 'queued' | 'converting' | 'transferring' | 'done' | 'error' | 'cancelled'
  progress: number      // 0–100 (byte progress during the copy)
  errorMessage?: string
  needsConversion: boolean
  targetFormat: BookFormat
  batchId?: string          // jobs sent together: the device is rescanned once per batch
  cancelRequested?: boolean // a cancel is in flight ("Cancelling…")
  devicePath?: string       // the device's mountPath (eject waits for its jobs)
}
```

`cancelled` is only set by the backend, once the job has really stopped.
What gets converted: a Kindle receives MOBI, AZW3 and PDF as they are and
everything else converted to AZW3; a Kobo receives EPUB, PDF and CBZ as they
are and everything else converted to EPUB (`deviceStore.sendBooks`).

> **Serde note:** `KindleDevice` uses `#[serde(rename_all = "camelCase")]` (Rust → frontend). `TransferRequest` field names are snake_case (`book_id`, `library_path`, `device_path`…; camelCase is accepted too). Books are sent by library id; see docs/IPC.md.

---

## Tauri Commands

**The contract is docs/IPC.md**: arguments, results, error codes, events
and on-disk formats, kept in sync with the code. Each command has one typed
wrapper in `src/services/api.ts`, and `npm run check:ipc` (part of
`npm run validate` and CI) checks the wrappers against the Rust signatures.
Common rules: every command returns `Result<T, AppError>` (the promise
rejects with `{ code, message }`); every command runs off the main thread;
writes are id-based (the backend looks paths up in its index), atomic and
serialised; paths from the frontend are confined to the library.

| Area | Commands |
|------|----------|
| Library | `scan_library`, `import_books`, `set_favorite`, `forget_books`, `library_lock_status`, `update_metadata`, `batch_update_metadata`, `rename_book`, `delete_books`, `reset_book_data`, `convert_to_epub`, `export_book`, `download_and_import`, `inspect_epub` (debug builds only) |
| Covers | `ensure_cover`, `save_cover_from_url`, `set_book_cover`, `delete_book_cover` |
| Users and the OS | `list_users`, `create_user`, `reveal_in_explorer`, `open_log_folder`, `detect_language`, `check_path_accessible` |
| Devices | `detect_devices`, `detect_device`, `get_mtp_status`, `list_device_books`, `scan_device_library`, `send_books`, `cancel_transfer`, `delete_device_book`, `eject_device`, `get_device_reading_progress` |
| Calibre | `detect_calibre_library`, `list_calibre_books`, `import_from_calibre`, `cancel_calibre_import` |
| Backup | `export_backup`, `read_backup`, `restore_backup`, `cancel_backup`, `reveal_backup` |
| Reader | `open_for_reading` |
| OPDS server | `start_opds`, `stop_opds`, `opds_status`, `invalidate_opds_cache`, `opds_qr_svg` |
| Network | `http_get` |
| Settings | `get_config`, `set_config` |

### Events (Rust → Frontend)

| Event | Payload | Description |
|-------|---------|-------------|
| `transfer-progress` | `{ jobId, status, progress, errorMessage? }` | During each transfer job (`converting`, `transferring`); every job ends with exactly one `done`, `error` or `cancelled` |
| `device-space` | `{ mountPath, freeSpaceBytes?, totalSpaceBytes? }` | After each transfer job, whatever its outcome |
| `calibre-import-progress` | `{ done, total, currentTitle }` | During a Calibre import (at most every 100 ms); `currentTitle` is `null` in the last one |
| `backup-progress` | `{ phase, filesDone, filesTotal, bytesDone, bytesTotal }` | During a backup or a restore (at most every 150 ms, and at each change of phase) |
| `app:log` | `string` | One debug-log line from Rust (picked up by the log store; also written to the log file) |

---

## Persistence

| What | Where | Notes |
|------|-------|-------|
| Library index | `<library>/.librarium/library.json` (+ `.bak`) | Format 2: books keyed by their path relative to the library folder (NFC), plus `ignored` and `missing`. Only the backend writes it (atomic, locked). See docs/IPC.md, "Library index" |
| Older index | `<library>/.librarium/books.json` | An older format (format 1). Read once, when there is no `library.json` yet, to create it (a byte copy goes to `books.v1.json.bak`); **never written**, and not read again afterwards |
| Library side files | `<library>/.librarium/failed.json`, `lock.json`, `trash/`; `<library>/.covers/` (`<id>.jpg\|png\|webp`, `thumbs/<id>.jpg`, `<id>.none`) | Unreadable files; the "open elsewhere" warning; fallback trash; covers |
| Device manifest | `<mount>/.librarium/device.json` (MTP: `device-1.json` / `device-2.json`) | Device id and the books Librarium sent (by library id) |
| Settings | `<app data>/librarium-config.json` | `get_config` / `set_config`: keys below. Atomic; a damaged file reads as `{}` and is never overwritten |
| UI state | The webview's `localStorage` | Keys below |
| Log | `<app log dir>/Librarium.log` | 5 MB × 5 files; never sent |
| Reader cache | `<app cache>/reader/` | MOBI/AZW/AZW3 → EPUB conversions, ≤ 500 MB |

`<library>` is a user's folder: `<library root>/<user name>/`, the root
being `~/Documents/Librarium` unless changed in the settings. The app
folders are named after the bundle identifier `com.librarium.desktop`: data
in `~/Library/Application Support/` (macOS), `%APPDATA%\` (Windows),
`~/.local/share/` (Linux); logs in `~/Library/Logs/` (macOS),
`%LOCALAPPDATA%\…\logs\` (Windows), `~/.local/share/…/logs/` (Linux); cache
in `~/Library/Caches/` (macOS), `%LOCALAPPDATA%\` (Windows), `~/.cache/`
(Linux).

**Settings file keys** (`AppConfig` in `src/services/appConfig.ts`; every
key optional, a missing key means the default):

| Key | Type | Default | Read by |
|-----|------|---------|---------|
| `theme` | `'nocturne' \| 'saphir' \| 'folklore' \| 'cerulean'` | `'nocturne'` | theme store |
| `lang` | `'en' \| 'fr'` | the system language (`fr*` → French, else English) | i18n |
| `soundVolume` | number, 0–1 | `0.8` | settings store |
| `libraryRootOverride` | string \| null | `null` (→ `~/Documents/Librarium`) | settings, users stores |
| `deviceSubfolder` | string | `''` (books go to `documents/`) | settings store |
| `googleBooksApiKey` | string | `''` (Google Books not used) | settings store |
| `opdsCatalogs` | `OpdsCatalogSource[]` | `[]` | opdsSources store |
| `opdsShareOnLan` | boolean | `false` | opds store |
| `opdsPassword` | string | generated when LAN sharing first needs one | opds store |
| `mtpEnabled` | boolean | `true` (anything but `false`) | settings store **and the backend** (applied at once; saved with `saveConfigOrThrow`) |
| `autoCheckUpdates` | boolean | `true` (anything but `false`) | settings store → updates store `checkAtStartup()` |
| `showNews` | boolean | `true` (anything but `false`) | settings store → updates store `checkAtStartup()` |
| `analyticsConsent` | `'granted' \| 'denied'` | absent = not asked yet (the consent dialog shows) | analytics store |
| `analyticsOptOut` | boolean | — | older setting: `true` is read as `analyticsConsent: 'denied'` |
| `appUid` | string \| null | — | older setting: a random id, never sent. Never written; the analytics store sets one it finds to `null` at startup (`set_config` merges keys, it can't delete one) |

**localStorage keys:** `librarium:currentUser`, `viewMode` (`grid`, the default, or `list`),
`opdsEnabled`, `opdsPort` (default 8042),
`lastMessageId` (the last "What's new" message dismissed), `appUuid` (the
installation ID sent with error reports and feedback: see "Installation
ID" under "Feature: Librarium API Integration").

---

## UI Layout

### Sidebar (`AppSidebar.vue`)

- The wordmark in the theme's metal between two of its ornaments, at the top
- **Nav items** (`<nav aria-label="Main">`): Library (`/library`), **Activity** (a button: see "Activity popover"), Discover (`/discover`), Settings (`/settings`, a dot when an update is available). There is no Device view: the e-reader's books are in the library view (see "Shelf").
- **Device block:** shows brand (`Kindle` / `Kobo`) on the first line and model detail (e.g. `Paperwhite 3`) on a smaller second line when a device is connected; that name is a link to the library view showing the device's books (`/library?scope=device`). Without a device it reads "No device", with a tooltip saying how to connect one. Also shows free space bar and refresh button.
  - "Send to" buttons elsewhere in the app show brand only (not model)
  - **MTP Kindles** (2024+, experimental) plugged in but not usable are listed below it (`deviceStore.mtpNotices`, from `get_mtp_status`, polled with detection), one line per Kindle translated by state: `mtpDisabled` → "enable experimental MTP support in Settings" + a link to `/settings#mtp-setting`; `connecting` → spinner; `claimFailed` / `busy` / `error` → a short headline with the backend's message (what to do) as detail; `ejected` → "safe to unplug". With no usable device, the first line names that Kindle instead of "No device".
- **Device block buttons** are IconButtons: "Look for an e-reader" (⌘K) and Eject.
- **User chip:** avatar in the user's colour (`utils/avatar.ts`: one of the theme's cover cloths, `var(--cover-N)`, dealt at random for the session from a shuffled deck of the six, reshuffled when empty, so six users in a row never share one; initials in `--cover-ink`, readable on every cloth, checked by `themes.test.ts`) + name, click to switch user
- **Language:** FR / EN toggle buttons, named in their language, `aria-pressed`
- **About:** bottom, opens the About dialog (the `app.about` command, like the menu bar's)

### Library View (`LibraryView.vue`)

The view renders the library store and wires its parts; the work lives in
`components/library/`, the `bookActions` and `imports` stores and two pure
modules (`utils/virtualWindow.ts`, `utils/collectionNav.ts`).

- **Toolbar slot** (fixed height, 68 px): `FilterSortBar` (the view's
  title, the scope switch All · Library · Kindle while an e-reader is
  connected, SearchInput, filter popover, sort DropdownMenu, grid/list
  IconButtons, duplicates), or, while books are selected,
  `SelectionToolbar` *in its place* (count, clear, select all, and the
  actions that apply to what is selected: edit, tidy up, send, Move to
  Trash, ⋮ Remove from library for library books; Add to library for files
  only on the e-reader; Remove from Kindle for anything on it). Nothing
  moves or overlaps when they swap. Narrow: the toolbar buttons show icons
  only (labels kept for screen readers).
- **Books**: the **shelf** (library books and the e-reader's, see "Shelf"),
  as `LibraryGrid` or `LibraryList`, both a `BookCollection`:
  - **virtualised**: only the rows on screen ± 2 are rendered; the scroller
    is measured with a ResizeObserver; row heights follow from the width
    (grid: columns ≥ 170 px, cover + 92 px of text; list: 68 px rows). When
    the list changes (search, filter, sort) or the layout does (window
    size), the book at the top of the screen (or the active one) stays in
    place;
  - **one focusable listbox** (`role="listbox"`, `aria-multiselectable`,
    options with `aria-selected`, `aria-posinset` / `aria-setsize`, a name
    "Title, Author, Series #n, on Kindle, 42% read"), roving tabindex: keys in
    "Keyboard Shortcuts"; the active book's row stays rendered when
    scrolled away, and focus is put back after a re-layout;
  - **titles always visible** under the covers (2 lines, then "…", full text
    as a tooltip); authors on one line with a tooltip; the series reads
    "Saga #2", or "Saga" without a volume (`seriesLabel`);
  - **real checkboxes** (`SelectCheckbox`, out of the Tab order: Space does
    the same), visible on hover, on keyboard focus, when selected and on
    every card while anything is selected;
  - **the star** (`FavoriteStar`, out of the Tab order) in the cover's
    top-right corner, after the title in the list: always shown on a
    favorite (filled, in the accent), on hover and focus otherwise; a click
    stars or unstars the book (`bookActions.toggleFavorite`). So do the
    context menu ("Add to favorites" / "Remove from favorites", for a
    selection: unstars only when all are favorites) and the details' star
    toggle. **Favorites come first** in every sort (`visibleShelf`),
    each group in the sort's order; the book's name says "favorite". The
    list changes at once and is put back if saving fails (a toast);
  - **where a book is**, when it is on the e-reader: a badge on the cover's
    bottom-left corner (a book and a tablet: in both places; a tablet alone:
    only on the e-reader), the same icons after the title in the list;
  - **reading progress** on the cover (a "42%" label in the bottom-right
    corner; "0%" for a book never opened) and in a list column, when the connected
    device reports progress (`readingProgress.deviceBookProgress`; a book
    with several copies takes the furthest). The view asks the device store
    to load it when a device is selected.
- **Details**, opened by a click, Enter or ⌘I: `BookDetailModal` (see
  below), for a library book or a file only on the e-reader (what the
  device says about it, Add to library, Remove from Kindle). When a file is
  added to the library, the details follow it to its new library book.
  **Context menu** (`BookContextMenu`,
  DropdownMenu.openAt), by where the books are: Show details; for library
  books Edit, Send, Remove from library, Move to Trash; for files only on
  the e-reader Add to library; for anything on it Remove from Kindle. On a
  book that is part of a selection of several, it acts on the whole
  selection, each action on the items it applies to (with their count).
- **Import**: the round FAB (IconButton, no animation) or ⌘O open the file
  picker; files dropped on **any view** are imported (`FileDropTarget` in
  App.vue: Tauri's drag-drop events → `imports` store, one import at a time,
  repeat payloads from WebView2 ignored, "nothing to import" said). After an
  import, MOBI/AZW3 books are offered for EPUB conversion.
- **Dialogs** (`LibraryDialogs`, state in the `bookActions` store): metadata
  editor, tidy-up review and batch confirmation, Trash / remove / "remove
  from Kindle" / "send anyway" confirmations, duplicates. An action on "the
  selection" only reaches visible books. Every delete is confirmed and goes
  to the Trash; removing from the e-reader is its own action (always
  confirmed: e-readers have no Trash) and never part of "Move to Trash".
  The Delete key moves library books to the Trash, and offers to remove a
  file only on the e-reader from it.
- Empty states use `EmptyState`: library unreachable (Retry), not allowed
  to read it (`PERMISSION_DENIED`: the backend's message, which on macOS
  points to System Settings › Privacy & Security › Files and Folders; Retry),
  unreadable, first scan, empty library (Import), no books on the device
  (the Kindle scope), no results.
- User picker: when the users can't be listed (root missing, not allowed,
  I/O error) it says why, with Retry and without "Add user"; the remembered
  user is kept (only a missing *default* root means "no users yet").

### Book details (`BookDetailModal.vue`)

- **A dialog over the books** (BaseModal, `bare`, 960 × 720 px at most):
  the books don't move or shrink. Escape, × or ⌘I close it; focus starts
  on the title and comes back to the last book shown.
- **Left: the book.** The cover, and under it what to do with the book:
  Send to [Device] (primary) and Read (not CBZ; ReaderModal). A file only
  on the e-reader: Add to library (primary), Remove from Kindle.
- **Right: the details.** A bar with the star (favorite toggle), previous /
  next book ("6 / 32"; ← → do the same; the collection's active book
  follows) and close; then `BookDetailHeader` (series, title heading,
  author, badges: format, On Kindle, categories), `BookMetadataTable` (a
  `<dl>`, the facts side by side, label over value), the description; a
  file only on the e-reader: `DeviceFileDetails` (file, format, size,
  progress). Delete / ⌫ asks to move the book to the Trash (a device file:
  to remove it from the device).
- **The action bar** at the bottom (`BookDetailActions`): Edit, Find
  metadata, Open file location, Export ▾ (EPUB / AZW3, when the book can
  be converted) on the left; More ▾ on the right: Rename file from
  metadata, Convert to EPUB, Reload details from the file (confirmed
  first), Inspect EPUB (dev), then Remove from Kindle (a copy is on it),
  Remove from library, Move to Trash. Export is the book-actions store's
  (`exportBook`), shared with the menu bar's File › Export… (the book shown,
  else the only selected one; it and ⌘I also work over the details, and
  only when no other dialog is open over them).
- A narrow window (under 640 px of dialog): the cover beside the two main
  buttons, above the details, in one scroll.

### Shelf: the library and the e-reader together (`services/shelf.ts`)

There is no Device view (`/device` redirects to `/library?scope=device`):
while an e-reader is connected, its books are in the library view, with the
library's. `buildShelf` merges the library store's books and the device
store's `deviceBooks` into **shelf items**:

| Presence | What | Item id |
|---|---|---|
| `libraryOnly` | a library book not on the device | the book's id |
| `both` | a library book, with its copies on the device (one item, never two) | the book's id |
| `deviceOnly` | a file no library book matches: title, author, file name, size and progress only | `device:` + its path |

Files are matched like the "already on device" marks (`deviceMatch.ts`):
by the library id the copies Librarium sends carry, else by the same title
AND author, or the same file name; a file may be a copy of several library
books. The library store holds the shelf (`shelf`, `shelfById`), the
**scope** (`scope`: `all`, `library`, `device`; `effectiveScope` is `all`
without a device) and `visibleItems` (scope, search, filters, sort;
`filteredBooks` is its library books). The selection holds item ids of
both kinds; `selectedBooks` (library actions) and `selectedItems` split it.

- A device-only item has a generated cover, its file name as title when
  the device gives none, its format from the extension; filters on what it
  doesn't have (series, language, category) leave it out, sorts on it
  (series, date added) put it last. The "Where" filter (with a device
  connected) keeps books only in the library ("not sent yet"), in both, or
  only on the device; the sort menu gets **Reading progress**.
- The scope switch shows while an e-reader is connected; when it goes away
  with the Kindle scope on, the view shows everything again and says why.
  The selection is pruned when the shelf changes (a device unplugged).
- **Add to library** copies device files into the library one at a time
  (`deviceStore.importFromDevice`, a job in Activity); the file then
  matches its new library book. **Remove from Kindle** deletes the files
  (a device-only file, or every copy of a library book) after
  confirmation; on a failure the dialog stays with the files still there.
- **MTP Kindle** (`connection: 'mtp'`): its books are listed once per
  refresh (slow: a request per book) and carry no progress; `mtp://` paths
  are backend locators, never shown or revealed (`src/utils/mtp.ts`).

#### Titles and authors of the device's books

`list_device_books` reads them, in order, from: the Kobo's database; the
file's metadata (an EPUB's OPF; a MOBI/AZW3's header, the first few KB:
EXTH 503 or the MOBI full name for the title, EXTH 100 for each author, in
the file's text encoding, UTF-8 or Windows-1252); what the device manifest
recorded when Librarium sent the book; the file name, split at the *last*
" - " (`001 - Réglez-lui son compte ! (1949) - San-Antonio` is a title,
then an author), with underscores read as spaces in a name without spaces.
An MTP Kindle's files aren't opened (a request each): the manifest, else
the file name. Checked on a Kindle Paperwhite 3 (2026-11-10): 308 books
listed in 0.7 s, every one with a title and an author.

#### Kindle filename author extraction (importing)

Kindle devices store books with filenames like `Title _ - Author.mobi`. `title_from_filename` in Rust replaces all `-` and `_` with spaces, producing e.g. `"Dungeon Crawler Carl T1     Matt Dinniman"` (multi-space run where the separator was).

After `import_books` returns, `importFromDevice` in `device.ts` splits the title on `\s{2,}` (2+ consecutive whitespace characters). If this yields 2+ parts, the first part becomes the title and the last part becomes the author. The correction is applied with `api.updateMetadata(libraryPath, book.id, …)` directly (not through the library store) because the book is not yet in the store's list at that point; a failure only leaves the title uncorrected (logged).

### Activity popover (`ActivityPopover.vue`) — in the sidebar

Replaces the Transfers page (`/transfers` now redirects to the library).

- **Sidebar item** "Activity": a button (`aria-haspopup="dialog"`,
  `aria-expanded`) with the number of running transfers, or an alert icon
  when one failed; its name says both ("Activity, 2 transfers in progress,
  1 transfer failed").
- **Popover**: a non-modal dialog next to the item (teleported to <body>,
  `--z-popover`), focused when it opens; Escape (focus back on the item),
  × or a click outside close it.
- **List**: both directions (library → device, device → library), each job
  with its state in words, the conversion (EPUB → AZW3), a ProgressBar
  while it runs (indeterminate while queued), the reason when it failed,
  and Cancel while the backend can still stop it (queued or converting).
  "Clear finished" forgets done, failed and cancelled jobs
  (`deviceStore.clearFinished`); running jobs always stay.

### About Modal

- Shown via `<Teleport to="body">` from `AppSidebar`
- Displays: version, build date, author, library path (full, no truncation)
- **Compatible devices section:** grid of the supported Kindle and Kobo models (kept in step with COMPATIBILITY.md), each with a format chip (`EPUB` in green, `AZW3` in amber) indicating the preferred format. The 2024 Kindles (Paperwhite 12th gen, Colorsoft, Kindle 2024, Scribe 2024) are listed apart, under "From 2024: MTP, experimental", with a note pointing to the MTP setting. Placed before credits.
- **Credits section:** lists all third-party libraries and assets requiring attribution (see [Credits](#credits))

---

## Tidy-up / Open Library Integration

Service: `src/services/tidyup/` (`index.ts` runs the providers,
`providers/googleBooks.ts` and `providers/openLibrary.ts`, `merge.ts`,
`batch.ts`, `fetchJson.ts`).

**Providers, in order:** Google Books (only when the user has set a Google
Books API key; the key is redacted from logs), then Open Library. Each is
asked by ISBN first (cleaned, ISBN-10 ↔ 13), then by title and author
(quoted `intitle:`/`inauthor:` on Google Books), with the book's language
(detected locally with `detect_language`) to narrow title searches. The
first result whose title is similar enough (`MIN_TITLE_SIMILARITY`, 0.6)
wins; otherwise the most similar one is offered for review (single book
only: a batch skips it). Requests time out after 10 s and are retried (at most 2 times) on
429/503, honouring `Retry-After`; "service unreachable" (every provider
failed) is told apart from "no match". Open Library covers are requested
with `?default=false`, so a missing cover isn't a placeholder image.

**Single book (from detail panel):**
1. Query the providers as above
2. Open `TidyupReviewModal` — user can uncheck fields they want to keep
3. Only fields where the incoming value **differs** from the current value are shown and pre-checked
4. On confirm: apply only selected fields

**Multi-select (from selection toolbar)** (`services/tidyup/batch.ts`, applied without review):
1. Show a confirmation naming the services and the rules
2. Only confident title matches are used; only **empty** fields are filled
3. A cover is only added to a book that has none (never replaces one)
4. One summary toast (updated / already complete / no confident match / unreachable / save failed)

**Fields fetched:** title, author (the first one), language, description
(HTML stripped), ISBN, categories (mapped onto the predefined list); plus
subtitle from Google Books, Goodreads URL from Open Library, and a cover URL.
**Fields intentionally excluded:** publisher, publishedDate

---

## Debug Panel (`DebugPanel.vue`)

- Toggle: **Ctrl+Shift+D** (works in release builds)
- Fixed floating panel, bottom center, 860px wide, 300px tall, z-index 9999
- Sources: `RS` = Rust (orange), `JS` = frontend (blue); `error` = red, `warn` = yellow
- Captures: `console.log/warn/error` interception + `app:log` Tauri events
- Max 500 entries, auto-scrolls on new entries
- Buttons: copy all to clipboard, clear, close
- Log store (`src/stores/log.ts`): installed in `App.vue` on mount

---

## Device Support Matrix

Full details for users in `COMPATIBILITY.md`; the commands in docs/IPC.md
("Device commands", "MTP Kindles"). Summary:

| Device | Connection | Sent as is | Converted to | Reading progress |
|--------|------------|------------|--------------|------------------|
| Kindle (before 2024) | USB mass storage | MOBI (incl. `.azw`), AZW3, PDF | AZW3 (from EPUB; CBZ fails: boko can't read it) | `.sdr` (`.mbs` + `.apnx`) when the page map exists; old firmware `.mbp` (unreliable); `system/*.db` |
| Kindle (2024+) | MTP, experimental, behind `mtpEnabled` (on by default) | same | same | Backend mirrors `.sdr` over MTP (unverified); the UI doesn't show it |
| Kobo | USB mass storage | EPUB, PDF, CBZ | EPUB (from MOBI, AZW3) | `.kobo/KoboReader.sqlite` |

**Kindles always get AZW3.** A Kindle can't open an EPUB copied over USB
(Amazon's EPUB support is Send-to-Kindle only, converted server-side), so
`preferredFormat` is `azw3` for every Kindle and `epub` for every Kobo
(`DeviceKind::preferred_format`). The frontend decides per book what needs
converting (`deviceStore.sendBooks`); the backend converts with boko
(`convert.rs`, `OutputFormat::for_device`: `mobi`/`azw` targets also give
AZW3, since boko can't write MOBI).

**Identity.** Each copy Librarium sends carries the library book id: as the
ASIN (EXTH 113, cdetype `EBOK`) in an AZW3 it converts, as an extra
`<dc:identifier>urn:librarium:<id></dc:identifier>` in an EPUB; books sent
in their own format are left byte-identical. The device manifest
(`<mount>/.librarium/device.json`) maps library ids to device paths. "On
device", replacing a re-sent book (same folder, so a Kindle's `.sdr` stays
paired) and deleting all rely on the id and the manifest, never on titles.

**Transfers** (`commands/device/transfer.rs`): a hidden `.part` file,
fsynced, then renamed; free space checked first; FAT-safe names; 1 MB chunks
with byte progress and cancellation between chunks; one device rescan per
batch; a `device-space` event after each job. Leftover `.librarium-*.part`
files older than 5 minutes are removed when a device appears.

---

## Device Detection (platform-specific)

`commands/device/detect.rs`. Detects **every** connected Kindle and Kobo
(several at once). The frontend polls `detect_devices` (every 5 s, 10 s
while a device is connected) together with `get_mtp_status`.

A volume counts only if its **name** contains "kindle" or "kobo" (so
unrelated volumes, a sleeping NAS share in particular, are never looked
into) **and** it has the device's structure: `documents/` for a Kindle,
`.kobo/` for a Kobo (so the "Kindle Previewer" installer image isn't taken
for a Kindle). A renamed Kindle or Kobo volume isn't found.
TODO(device-validation): confirm those folders exist on every model, even
freshly reset.

| Platform | Mount-point discovery |
|----------|-----------------------|
| macOS | Reads `/Volumes/` directly |
| Windows | PowerShell `Get-Volume` (labels containing kindle/kobo); falls back to `wmic logicaldisk` |
| Linux | `lsblk -J` (mounted disks and partitions, by label) |

Each device gets a stable **id**: a UUID written to
`<mount>/.librarium/device.json` the first time Librarium sees it, so a swap
(another id at the same mount path) is noticed; `"<type>:<mount>"` when the
device can't be written to. Ids and names never contain a serial number.

### Model names (filesystem-based)

**Kindle** — `{mount}/system/version.txt` (`Kindle 5.16.2 (…)`): the firmware
minor version gives a family name: 5.6–5.9 "Paperwhite 3", 5.10–5.12
"Paperwhite 4 / Oasis", 5.13–5.14 "Paperwhite 5", 5.15 "Scribe", 5.16+
"Paperwhite 6". This is a guess: an older Kindle updated to 5.16+ also reads
as "Paperwhite 6" (TODO 2.3: use the serial or USB product id, without
storing it). The model only feeds the UI and analytics; it doesn't change
the format sent.

**Kobo** — `{mount}/.kobo/Kobo/Kobo eReader.conf`, `model=` line → "Kobo
<model>"; else the firmware version from `.kobo/version` ("Kobo fw 4.20.…").
The first field of `.kobo/version` is the device's serial number: it is
never kept.

`deviceType` is derived from which detection path succeeded.

### MTP Kindles (2024+, experimental)

`src-tauri/src/mtp/` and `commands/device/mtp/`; on unless the
`mtpEnabled` setting is `false` (with it off, Librarium only enumerates USB
devices to say that one is plugged in, and never opens it). Matching:
Amazon's vendor id `0x1949` with an MTP/PTP interface (Windows: a WPD device
whose PnP id is an Amazon USB device). Transport: raw PTP/MTP over USB with
nusb on macOS and Linux, Windows Portable Devices on Windows; one worker
thread per device owns the session. Paths are `mtp://<port>/…` locators,
never shown to the user. `get_mtp_status` reports each Kindle's state
(`ready`, `connecting`, `mtpDisabled`, `claimFailed`, `busy`, `error`,
`ejected`) with a message saying what to do. Linux needs the udev rule
`src-tauri/linux/70-librarium-kindle.rules`. Validated against simulated
devices only (`TODO(device-validation)`). Details: docs/IPC.md, "MTP Kindles".

### Eject

`eject_device`: macOS `diskutil eject`; Linux `udisksctl unmount` (else
`gio mount -u`), then `udisksctl power-off`; Windows not yet (the error says
to use Safely Remove Hardware); an MTP Kindle is released. Refused while
books are being sent to that device.

---

## Reading Progress — Kindle Paperwhite FW 5.x

Reverse-engineered from a Kindle Paperwhite running firmware 5.x. Implemented in `src-tauri/src/commands/progress.rs` (`get_device_reading_progress`), which never rejects: a device that can't be read gives `{}`. Map keys are the book's **file name without its last extension** (`Dune` for `Dune.azw3`), which is what `services/readingProgress.ts` looks up (for the library view's shelf).

Per `.sdr` folder, the strategies are tried in order: a known SQLite file inside it (`data.db`, `YJBookmarkStorage.db`, `BookmarkStorage.db`, `KindleBookmarkStorage.db`, also one level down), then `.mbs` + `.apnx` (below). Then the `system/*.db` databases are queried, and, for books still without a value, the old-firmware `.mbp` files are read. Every path is kept until fixtures from real Kindles prove it useless. Without an `.apnx` page map the `.mbs` position can't be turned into a percentage, and Librarium doesn't write page maps yet, so some books show no progress.

### File layout on device

For every book file `documents/Foo.azw3`, Kindle creates a companion directory `documents/Foo.azw3.sdr/` containing:

```
Foo.azw3.sdr/
  Foo.apnx          ← Amazon Page Number eXchange (page count + byte offsets per page)
  Foo.mbs           ← MobipocketBookmarkState (reading position)
  Foo.mbp1          ← annotation cache (bookmarks, highlights) — NOT used for position
```

The `.sdr` directory stem (`Foo`) is used as the map key when returning progress (matched against `filename` in the frontend).

> **Filename coupling:** The link between a book and its `.sdr` is purely by filename convention — Kindle does no UUID or hash lookup. Library books are never renamed automatically, and a re-sent book replaces its previous copy in the same folder (found through the device manifest), so the `.sdr` folder stays paired with the file. Deleting a book from the device also deletes its `.sdr` (`<name>.sdr`, and `<stem>.sdr` unless another book with that stem remains).

---

### `.mbs` — MobipocketBookmarkState

Binary key-value map with 5 entries. Relevant keys:

| Key | Meaning |
|-----|---------|
| `fpr` | Furthest Position Read — **byte offset** into uncompressed book text (ASCII integer) |
| `lpr` | Last Position Read — complex string with base64-encoded CFI; NOT used |

**Binary encoding per entry:**

```
[fe|ff fe] 00 00   ← entry separator (1 or 2 bytes)
03                 ← type: string key follows
[3-byte BE len]    ← key length
[key bytes]        ← e.g. 66 70 72 = "fpr"
03                 ← type: string value follows
[3-byte BE len]    ← value length
[value bytes]      ← ASCII integer, e.g. "18463"
```

The `fpr` value is a **byte offset** in the book's uncompressed text content (same coordinate space as the APNX page offset array). Example: `18463` at page 9 of 443 → 2%.

**Parser:** `parse_mbs_fpr` searches for the bytes `[0x66, 0x70, 0x72]` ("fpr"), then expects `0x03` (string type) and a ≤10-byte ASCII body parseable as `u32`.

---

### `.apnx` — Amazon Page Number eXchange

Binary file mapping page numbers to byte offsets in book text. Confirmed on Kindle Paperwhite FW 5.x.

**Full layout (verified on 3 books):**

```
Offset    Size  Description
──────────────────────────────────────────────────────────
0         4     Magic: 00 01 00 01
4         4     Total header size, BE u32 (always 122 = 0x7A)
8         4     Main JSON length, BE u32 (always 110 = 0x6E)
12        110   Main JSON: {"contentGuid":"…","asin":"…","cdeType":"EBOK","fileRevisionId":"1"}
                (no pageCount field)

122       2     Section type: 00 01
124       2     Section JSON length, BE u16 (always 67 = 0x43)
126       2     ★ Total page count, BE u16  ← e.g. 0x01BB = 443
128       2     Reserved: 00 20
130       67    Section JSON: {"asin":"<uuid>"}
197       4×N   Page byte-offsets, each BE u32, one per page (N = total_pages)
```

**Verification (3 books):**
- Machine à Présages (443 pages): file = 1969 bytes = 197 + 443×4 ✓  
- Le Fantôme (698 pages): file = 2989 bytes = 197 + 698×4 ✓  
- Le Crépuscule (480 pages): file = 2117 bytes = 197 + 480×4 ✓

---

### Progress computation

```rust
// 1. Parse .apnx → (total_pages: u32, offsets: Vec<u32>)
//    offsets[i] = byte offset where page i+1 starts in book text

// 2. Parse .mbs → fpr: u32 (byte offset of furthest position read)

// 3. Binary search: count how many page offsets are ≤ fpr
let current_page = offsets.partition_point(|&o| o <= fpr) as u32;

// 4. Percentage (clamped to 1–100)
let pct = ((current_page * 100) / total_pages).clamp(1, 100) as u8;
```

Books never read past the cover have `fpr ≈ 109` (byte offset of the cover/title), which maps to page 1 → 1%.

---

### Reading progress — Kobo

Kobo stores progress in a SQLite database at `.kobo/KoboReader.sqlite`, table `content` (books: `ContentType = 6`), column `___PercentRead` (integer 0–100) and `ReadStatus`. `ContentID` ends with the filename. Librarium reads a private copy of the database and its `-wal` (so a WAL-mode or locked database works, and nothing is written to the device); `list_device_books` uses the same snapshot for titles, authors, progress and read status (`commands/device/kobo.rs`).

---

### Reading progress — Old Kindle (FW 1–3)

`.mbp` (Mobipocket Bookmark) files stored alongside each book. No reliable schema; position is extracted by scanning all 4-byte-aligned windows and looking for a value in range `(0, book_size)`. First plausible match wins, which probably often picks a header field (≈ 0 %): to be validated against real `.mbp` samples before it is trusted or replaced. Its key is the file stem, like the other paths.

---

## Format Conversion

Every conversion goes through one module, `src-tauri/src/convert.rs`, on
[boko](https://github.com/zacharydenton/boko) **0.6.0, vendored** in
`src-tauri/vendor/boko` (GPL-3.0-or-later; see "Credits") with two dated
"Modified for Librarium" patches: the KF8 chunker never splits a chapter
mid-element (upstream does, and Kindles drop those fragments, about three
pages at a time),
and `Azw3Config::asin` writes a per-book ASIN instead of the `EBOK000000`
placeholder. boko is built without its default features (no CLI, no image
re-compression), with `parallel`. It reads EPUB, AZW3, MOBI/AZW (and KFX),
and writes EPUB and AZW3 (and KFX, unused); it can't read PDF or CBZ.

What every conversion gets (details in docs/IPC.md, "Conversions"):
1. the source is opened by boko; a DRM-protected book is refused;
2. the result goes to a hidden, uniquely named temp file
   (`.librarium-convert-*`) in the folder the caller picks;
3. it is fsynced and **validated**: it must open again, have a chapter,
   have text (or images), and keep at least a tenth of the source's text;
4. a panic inside boko is a `CONVERSION` error; the temp file is removed
   unless the caller keeps it; the original is never touched before success.

| Use | Source → target | Where |
|-----|-----------------|-------|
| Send to a Kindle | EPUB (and anything not MOBI/AZW3/PDF) → AZW3, with the book id as ASIN | `send_books` (`commands/device/transfer.rs`) |
| Send to a Kobo | MOBI, AZW, AZW3 → EPUB, with an extra `urn:librarium:` identifier | `send_books` |
| Convert to EPUB (in place) | MOBI, AZW, AZW3 → EPUB; keeps id, metadata and date added; the original goes to the trash | `convert_to_epub` (`commands/library/convert.rs`); UI: `EpubConvertModal` (per-book state, retry) |
| Export as… | EPUB, MOBI, AZW, AZW3 → EPUB or AZW3 (its own format excluded), written next to the chosen destination and renamed over it | `export_book` |
| Reader | MOBI, AZW, AZW3 → EPUB, cached in `<app cache>/reader/` (≤ 500 MB, LRU) | `open_for_reading` (`commands/reader.rs`, `reader/cache.rs`) |

No external tool is needed. The old pipeline (`mobi`, `epub-builder`,
`html5ever`, `markup5ever_rcdom`), which produced empty books on
decompression errors and deleted the original, is gone.

---

## Metadata Extraction

`src-tauri/src/commands/library/metadata.rs`. Nothing there writes; write-back
is `commands/library/epub_edit.rs` on top of `src-tauri/src/epub/`.

| Format | How | Fields extracted |
|--------|-----|-----------------|
| EPUB | Own OPF module (`src-tauri/src/epub/`, quick-xml) | title, authors (joined with ` & `), subtitle, series and index (Calibre and EPUB 3 forms), language, description (HTML → text), ISBN (told apart from other identifiers), rating (`calibre:rating`), categories (subjects) |
| PDF | `lopdf` (Info dictionary, UTF-16 and PDFDocEncoding text decoded) | Title, Author, Subject (as the description), Keywords (as categories); title from the file name when missing |
| MOBI / AZW / AZW3 / CBZ | File name | title from the file name stem (`-` and `_` become spaces); nothing else |

A readable EPUB without a title keeps `''` (shown as "Unknown title"). An
EPUB or PDF whose metadata can't be parsed is still added, titled after its
file name; a file whose content doesn't match its extension isn't added
(see `failed.json` in docs/IPC.md).

**Language normalization:** `normalize_lang_code()` maps MARC 21 / ISO 639-2/3
codes (e.g. `eng`, `fre`, `ger`) to ISO 639-1 (e.g. `en`, `fr`, `de`). Applied
when reading EPUBs, on `detect_language` results, on Calibre imports and on
every metadata save, so all books store 2-letter codes. `detect_language`
(whatlang) returns a code only above 75 % confidence.

**Write-back** (EPUB only): only the fields an edit changes are written, by
splicing the OPF (never re-serialised); the package's unique identifier is
never modified; the ISBN is its own `dc:identifier`; rating and series as
Calibre's `<meta>`s. Rules in docs/IPC.md, "How edits reach EPUB files".

---

## Design System

The design (themes as book covers, type, metal, ornaments, the page's
layers, books and generated covers) is explained in
[docs/DESIGN.md](docs/DESIGN.md). Two stylesheets, imported once by
`main.ts`:

- `src/assets/themes.css`: the **tokens of each theme**: four themes
  (Nocturne, Saphir, Folklore, Cerulean), all dark. `:root` is the
  default (Nocturne, no `data-theme`); the three others are
  `[data-theme="X"]` blocks, which come after it and weigh the same, so
  they override it. Each theme has two blocks: its fonts, ornaments and
  cover cloths, then its palette. The blocks apply to any element carrying
  the attribute: the theme picker shows each theme in its own colours that
  way (`data-theme=""` is Nocturne). Every theme defines every token: none
  inherits Nocturne's by accident.
- `src/assets/main.css`: the **theme-independent tokens** (spacing, type,
  radii, layers, motion, focus), the few colour tokens derived from the
  palette, the page's layers (sky, glows, motif, grain), the cover
  lettering and ornament utilities (`.display-title`, `.heading-title`,
  `.foil-text`, `.ornament`), the base styles (a small reset) and the
  global classes some templates use (`.button`, `.input`, `.select`,
  `.tag`, `.delete`, `.modal*`, `.form-*`, `.progress-*`, `.spinning`).
  Those classes go away as screens move to the UI kit (below).

No CSS framework: the few classes needed are in `main.css`, on the tokens;
`themes.test.ts` fails if Bulma or an `!important` appears.

### Tokens (per theme)

| Token | Use |
|-------|-----|
| `--bg`, `--bg-elevated`, `--bg-hover` | Surface levels: page, raised (cards, panels, menus), hover/pressed |
| `--bg-modal`, `--bg-card`, `--bg-card-hover`, `--bg-input`, `--bg-cover-placeholder`, `--bg-filter-bar`, `--bg-header-sticky`, `--glass` | Specific surfaces (some are gradients; `--glass` is the translucent pane of the sidebar and the user picker) |
| `--sky`, `--body-overlay-1`, `--body-overlay-2`, `--motif`, `--grain-opacity` | The page: its gradient, two glows, its pattern (two tiles), the paper grain |
| `--modal-backdrop` | The dimmed backdrop behind dialogs |
| `--border`, `--border-hover`, `--hairline` | Borders; `--hairline` is the gilt line around panels, dialogs and menus |
| `--foil` | The theme's metal (a gradient): the wordmark, ornaments, progress bars. **Never text** |
| `--text-primary`, `--text-secondary`, `--text-muted` | Text, strongest to faintest; muted is the faintest allowed (4.5:1) |
| `--accent`, `--accent-muted`, `--accent-dark` | The theme's hue: borders, tints, glows, fills **without text** |
| `--accent-text` | Accent-coloured **text** (readable on surfaces and on `--accent-muted`): the active nav item, series names |
| `--accent-strong` + `--on-accent` | A solid accent fill **with text on it** (primary button); `--accent-hover` is its hover (derived, slightly darker). `--on-accent` is dark on the gold and silver themes |
| `--secondary`, `--secondary-muted`, `--secondary-text` | The second hue (chips, badges), same rules |
| `--danger`, `--success`, `--warning` | Status **text** (readable on surfaces and on their `-muted` tint) |
| `--danger-muted`, `--success-muted`, `--warning-muted` | Status tints (backgrounds) |
| `--danger-strong` + `--on-danger` | A solid danger fill with text (danger button on hover) |
| `--fmt-epub`, `--fmt-pdf`, `--fmt-mobi`, `--fmt-azw3`, `--fmt-cbz` | Format badges |
| `--focus-ring-color` | The keyboard focus ring (derived: `--accent-text`) |
| `--shadow-sm`, `--shadow`, `--shadow-lg`, `--shadow-popover`, `--glow-card`, `--glow-focus` | Elevation; `--glow-card` is a lifted book (a gilt edge and a halo) |
| `--scrollbar-color(-hover)`, `--select-arrow`, `--color-on-device` | Details (`--color-on-device` is derived: `--success`) |
| *Type:* `--font-display`, `--font-heading`, `--font-serif`, `--display-weight`, `--display-tracking`, `--heading-weight`, `--heading-tracking`, `--serif-weight`, `--serif-scale` | The theme's lettering (see docs/DESIGN.md › Type) |
| *Ornaments:* `--ornament-rule`, `--ornament-corner`, `--cover-art` | Ornament masks (SVG data URIs): a divider, a frame corner, the emblem |
| *Covers:* `--cover-1` … `--cover-6`, `--cover-ink`, `--cover-ink-soft`, `--cover-foil` | Generated covers: six cloths, the title ink, the series/author ink, the gilding |

**Contrast is tested.** `src/assets/themes.test.ts` parses both stylesheets,
resolves every theme's tokens (`var()`, `color-mix()`, gradients; helper in
`src/test/themeTokens.ts`, which also reads selector lists) and checks WCAG
AA: 4.5:1 for text (primary, secondary and muted text on every surface
including the sky, the glows and input backgrounds; accent and status text
on surfaces and on their tints; labels on the primary and danger fills, at
rest and on hover; the active nav item over the glass; format and neutral
badges; the generated covers' inks on every cloth) and 3:1 for the focus
ring and the accent. It also checks that secondary text stays visibly
stronger than muted text. A palette edit that breaks a pair fails the test.

Lowest ratio over the surfaces each pair is drawn on:

| Theme | primary text | secondary text | muted text | accent-coloured text | primary button label (rest, dark end, hover) | cover title / author ink | focus ring |
|-------|-------------|----------------|-----------|----------------------|----------------------------------------------|--------------------------|------------|
| saphir | 12.1 | 8.5 | 6.0 | 9.1 | 5.7 | 8.8 / 6.6 | 8.9 |
| nocturne | 11.9 | 8.0 | 6.3 | 7.3 | 5.2 | 9.3 / 6.3 | 7.3 |
| folklore | 12.8 | 8.9 | 5.9 | 7.2 | 5.0 | 9.0 / 6.5 | 6.8 |
| cerulean | 12.5 | 8.5 | 5.1 | 8.9 | 7.9 | 10.0 / 7.4 | 8.9 |

### Scales (theme-independent, `main.css`)

| Tokens | Values |
|--------|--------|
| `--space-1` … `--space-7` | 4, 8, 12, 16, 24, 32, 48 px |
| `--font-size-xs` … `--font-size-3xl` | 12, 13, 14 (body), 16, 20, 24, 32 px. **12px is the minimum** for any text |
| `--font-weight-regular/medium/semibold/bold`, `--line-height`, `--line-height-tight` | 400/500/600/700, 1.5, 1.25 |
| `--font-sans` (Inter), `--font-mono` | Families (the display, heading and serif families are per theme) |
| `--radius-cover`, `--radius-xs`, `--radius-sm`, `--radius-control`, `--radius`, `--radius-lg`, `--radius-full` | 3 (books), 4, 8, 10 (buttons, fields), 14, 22, 999 px |
| `--z-effects` 0, `--z-raised` 2, `--z-sticky` 10, `--z-drop-zone` 50, `--z-modal` 100 (dialogs outside BaseModal), `--z-popover` 200, `--z-overlay` 300, `--z-reader` 500, `--z-toast` 9000, `--z-debug` 9999 | Stacking layers. Native dialogs (BaseModal) are in the browser's top layer, above all of them |
| `--duration-fast`, `--duration`, `--duration-slow`, `--duration-sheen`, `--ease-out`, `--ease-in`, `--ease-lift`, `--transition` | 100 / 160 / 260 / 900 ms (the foil glint); `--ease-lift` overshoots a little (a lifted book); the durations drop to 1 ms under `prefers-reduced-motion` |
| `--focus-ring-width`, `--focus-ring-offset` | 2px / 2px: `:focus-visible` draws one ring everywhere |
| `--control-height`, `--control-height-sm`, `--sidebar-width`, `--topbar-height`, `--cover-ratio` | Layout |

`color-mix(in srgb, var(--accent) X%, transparent)` is used for tints throughout the UI.

---

## UI kit (`src/components/ui/`)

Shared components, Options API like the rest of the app, each tested next to
it (`*.test.ts`, mounted with `src/test/mountUi.ts`). New screens use them;
existing screens move onto them as they are reworked. Prop types
live in `ui/types.ts`.

| Component | What it is |
|-----------|-----------|
| `BaseButton` | Text button: `variant` primary / secondary / ghost / danger, `size` sm / md, `icon`, `loading` (spinner, `aria-busy`, stays focusable, ignores clicks), `disabled`, `block`. `type="button"` by default |
| `IconButton` | Icon-only button. **`label` is required** (aria-label + tooltip); `pressed` makes it a toggle (`aria-pressed`) |
| `BaseModal` | Every dialog: native `<dialog>` + `showModal()` (top layer, inert page), registered in the modals stack, Escape for the top dialog only, focus moved in / kept in / given back, `aria-labelledby`, optional `closeOnBackdrop`, `dirty` → "Discard your changes?" guard, sizes sm / md / lg / xl / full / screen (edge to edge: the reader) or `width`, `bare` layout. Mount it with `v-if`; `close` is a request the parent grants |
| `ConfirmDialog` (in `components/`) | Confirmation built on BaseModal (`danger`, `busy`, `error`) |
| `DropdownMenu` + `MenuItem` + `MenuSeparator` | The one menu: a dropdown (trigger slot) or a context menu (`openFromEvent`); `role="menu"`, arrows / Home / End / type-ahead, Escape and Tab close it, an outside click closes it, kept inside the window, rendered inside the open dialog when there is one |
| `SearchInput` | Search field with a label for screen readers, debounced `v-model`, Enter commits, × and Escape clear |
| `TextField`, `SelectField`, `Checkbox` | Form controls with a label linked by id, `hint` and `error` (read with the field, `aria-invalid`; the error has an icon, not just a colour). TextField and SelectField share the internal `FormField` |
| `Badge` | Small label: neutral / accent / success / warning / danger, and one tone per book format |
| `EmptyState` | Nothing to show: icon, title, text, `actions` slot, `loading`, `alert` |
| `Spinner` | The spinning loader icon; with a `label` it is announced (`role="status"`) |
| `ProgressBar` | Track + fill, `role="progressbar"` with `aria-valuenow` / `aria-valuemax`, indeterminate when `value` is null |
| `SettingRow` | One setting: name and description beside (or above) its control, with ids for labelling it |

Rules:

- **Icon-only buttons use `IconButton`**, never a bare `<button>` with an
  icon (it would have no name for screen readers).
- **Every dialog uses `BaseModal`**: no hand-made overlays, focus traps or
  Escape handlers. **Every menu uses `DropdownMenu`.**
- **Colours come from tokens.** Text uses the text tokens, the `-text`
  tokens or the status tokens; `--accent` and `--secondary` are never text
  colours; a solid fill with text uses `-strong` + `--on-*`. No hard-coded
  colours.
- **Sizes come from the scales**; no text below 12px.
- **Status is never shown by colour alone**: add text or an icon.
- **Global shortcuts go through the shortcuts store** (below), never a
  component's own `keydown` listener on `document` or `window`.

### Plumbing (Pinia options stores)

- **`modals` store** (`stores/modals.ts`): the stack of open dialogs.
  BaseModal pushes itself on mount and removes itself (by id) on unmount;
  `isTop(id)` tells a dialog whether it gets the keys; `hasOpenModal` turns
  global shortcuts (and menu commands) off. Every dialog is a BaseModal.
- **`shortcuts` store** (`stores/shortcuts.ts`) and `shortcutsPlugin`
  (installed in `main.ts`): **one** global keydown listener. The commands
  are declared in `constants/shortcuts.ts` (keys, description, group,
  `whileTyping`, `whileModal`); components register a handler by id while
  they are on screen (`register(id, fn)` / `unregister(id, fn)`). The
  listener skips keys a focused widget already handled, keys typed in text
  fields, and everything while a dialog is open. A handler returning `false`
  passes the key on. `runCommand(id, 'menu' | 'button')` runs the same
  handlers for the menu bar and buttons (same dialog rule);
  `runWhenRegistered(id)` runs a command once its screen registers it
  (within 3 s).
- **`saveStatus` store** (`stores/saveStatus.ts`) and
  `settings/SaveStatus.vue`: `track(key, savePromise)` shows "Saved" next to
  a setting for 2.5 s, or "Not saved: <reason>" until the next attempt
  (polite live region). Settings setters return the save's promise
  (`saveConfigOrThrow`); the value stays applied for the session either way.
- **`system` store** (`stores/system.ts`): `prefersReducedMotion` (the
  background effects' default), read at creation and followed live once
  `install()`ed (App.vue).
- **`notifications` store** (`stores/notifications.ts`) and
  `AppNotifications`: the toasts. `push(type, message, { durationMs,
  action: { label, run } })` returns an id (`dismiss(id)`); a number as the
  third argument is the duration. Errors stay longest, a toast with a button
  at least 10 s; the same message isn't stacked twice; at most 5 at once.
  Timers stop while the pointer or focus is on the toasts or the window is
  hidden. The toasts are a `popover="manual"` element moved into the top
  open dialog, so they are drawn above dialogs and stay clickable and
  announced (polite live region; errors in an assertive one). A failure
  inside a dialog is shown in the dialog, not as a toast.

---

## Keyboard Shortcuts

The list of commands lives in **`src/constants/shortcuts.ts`**, in one
place (keys, what they do, their group, when they apply): it is the single
list the shortcuts store, the native menu bar and the Keyboard shortcuts
dialog (Help › Keyboard shortcuts, ⌘/) use. `Mod` is ⌘ on macOS and Ctrl
elsewhere.

| Shortcut | Action | Handled by |
|----------|--------|-----------|
| `Mod+O` | Import books (any screen) | AppCommands |
| `Mod+K` | Look for a connected e-reader (any screen) | AppCommands |
| `Mod+,` | Settings | AppCommands |
| `Mod+/` | Keyboard shortcuts dialog | AppCommands |
| `Mod+1` / `Mod+2` | Books as a grid / list (opens the library) | AppCommands |
| `Mod+I` | Show or hide the active (or only selected) book's details | LibraryView (elsewhere: opens the library first) |
| `Mod+F` | Focus the library search | LibraryView (elsewhere: opens the library first) |
| `Mod+A` | Select all books (again: none) | LibraryView |
| `Delete`, `Backspace`, `Mod+Backspace` | Move the selection (else the focused book; in the details, the book shown) to the Trash, after confirming | LibraryView, BookDetailModal |
| `ArrowLeft` / `ArrowRight` | Previous / next book, in the details | BookDetailModal |
| `Escape` | Clear the selection; close the details | LibraryView, BookDetailModal |
| `Ctrl+Shift+D` | Show or hide the debug console (also inside dialogs) | DebugPanel |
| — | About, Export…, Find duplicates, Open log folder, Privacy: menu only | AppCommands / LibraryView |

### Native menu bar (`src-tauri/src/menu.rs`, macOS)

App menu (About Librarium, Settings… ⌘,, Services, Hide, Quit), File
(Import Books… ⌘O, Export…, Close Window), Edit (standard items), View
(Grid ⌘1, List ⌘2, Show or Hide Details ⌘I, Search ⌘F, Full Screen),
Library (Detect E-reader ⌘K, Find Duplicates…), Window, Help (Keyboard
Shortcuts ⌘/, Open Log Folder, Privacy), in English or French
(`set_menu_language`, called by App.vue at startup and on each language
change).

- **The accelerators shown in the menu mirror `constants/shortcuts.ts`**:
  `constants/menu.test.ts` reads menu.rs and fails when an accelerator
  differs from the command's first key, or when a menu id isn't a command.
- A click emits `menu-command` with the command id; AppCommands routes it
  through `shortcutsStore.runCommand(id, 'menu')`, so the menu runs **the
  same handlers as the keys**, with the same rule (nothing runs behind a
  dialog). When a key and the menu's accelerator both fire for one press,
  the menu's echo (same id within 500 ms) is dropped.
- Not installed on Windows and Linux (it would add a native strip to the
  window); there the Keyboard shortcuts dialog lists the commands.

Not global, handled by the focused widget: Escape in a dialog (closes the top
one only), arrows / Home / End / letters in a menu, Enter / Escape in a search
field, ArrowLeft / ArrowRight in the reader (EPUB pages), and the library's
books (`BookCollection`, model in `utils/collectionNav.ts`):

| Key / click | In the grid or list |
|-------------|---------------------|
| ← → ↑ ↓ | Previous / next book, the book above / below (grid-aware) |
| Home / End, PageUp / PageDown | First / last book, one screen of rows |
| Shift + those | Extend the selection from the anchor (and shrink it back) |
| Space (Shift+Space) | Toggle the book (range) |
| Enter, click | Open the book's details |
| ⌘-click (Ctrl-click), Shift-click | Toggle the book, select a range |
| ContextMenu, Shift+F10, right-click | The book's menu |

The library view registers its shortcuts with the store while it is shown
(it has no `keydown` listener of its own; neither has the filter bar, whose
⌘F is `library.search`).

---

## CI/CD (GitHub Actions)

The full procedure (secrets, signing key, troubleshooting) is in DEPLOY.md.

- **`.github/workflows/ci.yml`** — every push and pull request to `main` and
  `overhaul/**`, on macOS (arm64), Windows and Ubuntu 24.04: `npm ci`,
  vue-tsc, ESLint, vitest, the IPC contract check, the version check, a
  frontend build, `cargo clippy --locked --all-targets -- -D warnings`,
  `cargo test --locked`; plus a `cargo deny check` job
  (`deny.toml`, at the repository root). `cargo fmt --check` isn't enabled yet.
- **`.github/workflows/release.yml`** — `v*` tags only (no manual trigger),
  one release at a time:
  - `check`: the tag equals `package.json`, `Cargo.toml` and `Cargo.lock`
    versions; the commit is on `main`; the annotated tag's message is the
    release notes;
  - `build` (unsigned, in parallel, `npm ci`, `cargo --locked`, pinned
    toolchain, no caches): macOS aarch64 and x86_64 (`app`, `dmg`), Windows
    x64 (NSIS `.exe`, `.msi`), Linux x64 (`.AppImage`, `.deb`); debug
    symbols kept as artifacts. Only the `VITE_*` secrets are visible here;
  - `sign`: signs the updater artifacts with the minisign key (password
    supported) in a job that runs no project dependency code, then
    verifies every signature;
  - `publish`: writes `latest.json` (`darwin-aarch64`, `darwin-x86_64`,
    `windows-x86_64[-nsis|-msi]`, `linux-x86_64-appimage`) and
    `SHA256SUMS`, creates a **draft** release on this repository (the
    workflow's own token, `contents: write` for this job only; no other
    repository), uploads, re-downloads and verifies, then publishes.
- The build secrets compiled into the frontend are `VITE_API_URL`,
  `VITE_API_KEY`, `VITE_UMAMI_URL` and `VITE_UMAMI_WEBSITE_ID` (all public
  once built: see `.env.example`). The build runs the Tauri CLI through
  `scripts/tauri.mjs`, which adds the hosts of `VITE_API_URL` and
  `VITE_UMAMI_URL` to the CSP's `connect-src` (`tauri.conf.json` lists only
  the services every build uses).
- Versions: `npm run bump X.Y.Z` updates `package.json`, `package-lock.json`,
  `Cargo.toml` and `Cargo.lock` (`tauri.conf.json` reads `package.json`);
  `npm run release` checks, tags with the release notes and pushes.

---

## Setup

### Prerequisites

1. **Node.js** 20.19 or newer (`engines` in package.json; `.nvmrc` has the
   version CI uses).
2. **Rust** through rustup (https://rustup.rs): the toolchain version is
   pinned in `rust-toolchain.toml` (with clippy and rustfmt) and installed
   on first use.
3. The [Tauri prerequisites](https://tauri.app/start/prerequisites/) for
   the OS (on Linux: WebKitGTK 4.1 and friends; the list CI installs is in
   `.github/workflows/ci.yml`).

No external tools are needed for conversion (boko), MTP (nusb is pure Rust;
WPD is part of Windows) or SQLite (bundled).

### Install & Run

```bash
npm ci                  # exact versions from package-lock.json
npm run dev             # tauri dev (through scripts/tauri.mjs): Vite on port 1420 + the Rust app
npm run vue:dev:mock    # UI only, in a browser, against src/dev/mockBackend.ts (port 5173)
```

Build-time settings: copy `.env.example` to `.env.local`.

### Checks

```bash
npm test                # vitest
npm run validate        # vue-tsc, eslint src, vitest, check:ipc, clippy, cargo test
npm run lint:fix        # ESLint --fix
npm run check:version   # every copy of the version agrees
npm run notices         # regenerate THIRD_PARTY_NOTICES.md
```

### Build for production

```bash
npm run build           # = tauri build: unsigned, for the current platform → src-tauri/target/release/bundle/
```

Local builds don't create updater artifacts (`createUpdaterArtifacts:
false`), so no signing key is needed. Releases are built by CI only.

### Regenerate app icons

```bash
npx tauri icon src/assets/icons/library.svg
```

---

## Settings (`SettingsView.vue`) — `/settings`

`/options` redirects here (keeping its `#hash`). A side navigation lists the
sections; the one shown is the address's hash, so each has a deep link:
`#library`, `#devices`, `#metadata`, `#opds`, `#catalogs`, `#appearance`,
`#privacy` (the menu bar's Help › Privacy), `#updates`, `#backup`,
`#advanced`. Single
settings can have their own anchor: `#mtp-setting` (the sidebar's MTP
notice) opens Devices, scrolled to it (`components/settings/sections.ts`).
The navigation's links replace the history entry.

| Section | Component | Settings and actions |
|---------|-----------|----------------------|
| Library | `LibrarySettings` | Library folder (Change…, "Use the default folder" after confirming; reachable or not), Rescan, Users (who is signed in, Switch user), Import from Calibre |
| Devices | `DeviceSettings` | The connected e-reader (Look for one), the folder on the e-reader, MTP Kindles (experimental) |
| Metadata | `MetadataSettings` | Where "Find metadata online" looks (Google Books when it has a key, then Open Library), the Google Books API key (shown on demand) |
| OPDS server | `OpdsSettings` | On/off and real state, addresses (copy), port (checked: 1024–65535, an error in the field otherwise; never NaN), sharing on the network (warning, LAN addresses, QR code, user name and password, new password after confirming), reading-app hints |
| Catalogs | `CatalogSettings` | OPDS 2.0 catalogs for Discover: remove (confirmed), add (http(s) address checked) |
| Appearance | `AppearanceSettings` | Theme (`ThemePicker`: a title page per theme, in its own colours; "Saved" beside the row's title, `SettingRow`'s `status` slot), background effects Off / Subtle / On, language, sound volume, launch sound |
| Privacy & analytics | `PrivacySettings` | The opt-in statistics, what they send and never send (`AnalyticsDisclosure`, the same component as the consent dialog), the other connections (the two launch checks as set, metadata/catalog services on demand, feedback), the privacy policy (`PrivacyPolicyDialog`: PRIVACY.md) |
| Updates | `UpdateSettings` | This version, a new one (release notes as safe text, Download & install), Check now; Check for updates automatically, Show news from Librarium (both on by default, from the next launch) |
| Backup | `BackupSettings` | Back up the library (a folder picker, then a zip of every user with progress and Cancel; a toast with Show), Restore a backup (a zip picker, then `RestoreBackupDialog`: what is erased, what the backup holds, "I understand…" to tick, then progress). See docs/IPC.md, "Backup" |
| Advanced | `AdvancedSettings` | Send feedback, Show the log file (`open_log_folder`), the debug console |

Every setting applies as soon as it changes (text fields: on blur or
Enter) and its row says "Saved", or "Not saved: <reason>"
(`stores/saveStatus.ts`). Settings are global, not per user, and saved in
`app_data_dir/config.json` through `appConfig.ts` (restored by App.vue
after `loadConfig()`), except the OPDS server's on/off and port
(localStorage).

| Setting | Storage | Default | Description |
|---------|---------|---------|-------------|
| Sound effects | appConfig `soundVolume` | `0.8` | Volume 0–1 (saved when the slider is released). 0 mutes UI sounds. |
| Launch sound | appConfig `launchSound` | `false` | The intro sound at startup; off unless turned on. |
| Background effects | appConfig `effects` | absent → Subtle, or Off when the system asks to reduce motion (followed live) | `off`, `subtle` (fewer particles, ~30 fps) or `on`. |
| Library folder | appConfig `libraryRootOverride` | `null` (→ `~/Documents/Librarium`) | Override default library root; supports NAS paths. Reachability tested via `check_path_accessible` (3 s timeout). |
| Device subfolder | appConfig `deviceSubfolder` | `""` | Subfolder inside `/documents` on device. Empty = root. |
| MTP Kindles (experimental) | **appConfig** `mtpEnabled` | `true` (anything but `false`) | Read by the backend too, which applies a change at once. Saved with `saveConfigOrThrow`: a failed save puts the switch back and shows a toast. |
| Google Books API key | appConfig `googleBooksApiKey` | `""` | When set, Google Books is preferred over Open Library for metadata. |
| Theme | **appConfig** `theme` | `"nocturne"` | One of `nocturne`, `saphir`, `folklore`, `cerulean`. Older names are read as their successors: `autumn` → `folklore`, `winter` → `cerulean`, `spring` → `nocturne`, `summer` and `seasonal` → `saphir` (written back the next time the theme changes). |
| Language | **appConfig** `lang` | browser language | `"en"` or `"fr"`. Defaults to `navigator.language`; overridden by config on mount. |
| OPDS enabled | localStorage `opdsEnabled` | `"false"` | Whether the OPDS catalog server auto-starts on launch. |
| OPDS port | localStorage `opdsPort` | `"8042"` | TCP port for the OPDS HTTP server. Whole numbers 1024–65535 (`parsePort`); a damaged saved value reads as 8042. |
| OPDS catalog sources | appConfig `opdsCatalogs` | `[]` | `OpdsCatalogSource` objects registered by the user. |
| Analytics | **appConfig** `analyticsConsent` | not asked yet | Opt-in (`granted` / `denied`). Settings › Privacy & analytics. |
| Check for updates automatically | appConfig `autoCheckUpdates` | `true` | The update check (GitHub `latest.json`) at launch. "Check now" works either way. |
| Show news from Librarium | appConfig `showNews` | `true` | The "What's new" messages request (Librarium API) at launch. |

**Actions on this page** (not persisted settings):

| Action | Description |
|--------|-------------|
| Rescan library | Calls `scan_library` on the current user's library path — picks up new files, prunes deleted ones, updates the live book list. Shows a spinner while running. |
| Import from Calibre | Opens the Calibre import modal (see [Feature: Import from Calibre](#feature-import-from-calibre)). |
| Back up the library | `export_backup` into a chosen folder: every user, their books, covers and metadata (not the settings). |
| Restore a backup | `restore_backup`, after a warning: **every user of the library root is erased** and replaced by the backup's. Extracted aside first, so a cancel, a damaged file or a lack of space changes nothing. |

---

## Feature: In-App Ebook Reader

### Summary

Full-screen reader for EPUB, PDF, MOBI, and AZW3 books, accessible from the **Read** button under the cover in `BookDetailModal`. MOBI/AZW3 are converted to EPUB on the fly before rendering. CBZ is excluded.

### Architecture

| Step | Detail |
|------|--------|
| Entry | "Read" button in `BookDetailModal` (shown only for `epub`, `pdf`, `mobi`, `azw3`) |
| Data transfer | `open_for_reading(libraryPath, bookId)` returns `{ format, path, sizeBytes, converted }` and adds that one file to the asset protocol's scope; `src/services/reader.ts` turns it into a URL with `convertFileSrc`. Nothing goes through IPC as bytes (the base64 `prepare_for_reading` is gone). The book is looked up by id and must be inside the library with its format's magic bytes. MOBI/AZW/AZW3 are converted to EPUB once (`convert.rs`) and cached in `<app cache>/reader/` (≤ 500 MB, least recently used removed first). |
| EPUB rendering | `epub.js` 0.3.93 (`epubjs`, unmaintained; `@xmldom/xmldom` pinned to a patched 0.8 by `overrides`, checked by `services/epubjs.test.ts`) — the file is `fetch`ed from its asset URL into an `ArrayBuffer` → `ePub(data)` → `renderTo(container, { width, height, spread: 'none' })` |
| PDF rendering | The web view's own PDF viewer in an `<iframe>` pointed at the asset URL (Range requests: only what is shown is read) |
| Progress | Always starts at page 1 — no sync with device reading progress |
| Font size | Fixed at 15px — no user control |
| Theme | Inherits app warm-beige palette via `getComputedStyle` on `--bg`, `--text-primary`, `--text-secondary` (CSS vars don't cross iframe boundaries; values are read and injected as literals) |

### UI

- Full-screen overlay (`position: fixed; inset: 0; z-index: 500`)
- **Header:** book title + close button (also closes on `Escape`)
- **EPUB area:** dark radial-gradient backdrop + a fake e-reader device frame containing the epub.js render area
  - Device frame: dark graphite shell, rounded corners (32px), layered box-shadows for depth, decorative side power button via `::before`
  - Screen area (`overflow: hidden`, `background: var(--bg)`) prevents flash on page turn
  - Navigation: circular icon-only prev/next buttons + centered page counter (`X / Y`) in the device bezel
- **PDF area:** full-height `<iframe>` with blob URL
- **Loading state:** spinner + "Converting to EPUB…" label for MOBI/AZW3
- **Error state:** alert icon + error message

### EPUB Typography

Injected via `rendition.themes.default()`:

| Property | Value |
|----------|-------|
| `font-family` | `Baskerville, 'Book Antiqua', Georgia, Palatino, serif` |
| `font-size` | `15px` |
| `line-height` | `1.75` |
| `text-align` | `justify` |

### Page tracking

`rendition.on('relocated', location => ...)` — reads `location.start.displayed.page` / `.total`. Displayed as `X / Y` in the device nav bar.

---

## Feature: Import from Calibre

### Summary

Copy-based migration from a Calibre library into Librarium. The user picks
the Calibre library (the one Calibre itself uses is offered), browses its
books, all pre-selected, deselects what they don't want, then confirms.
Files are copied into the current user's library and metadata is read from
Calibre's database. Contract: docs/IPC.md, "Calibre import".

### Decisions

| Question | Decision |
|----------|----------|
| Copy vs reference | **Copy** — files are copied into the Librarium library folder |
| Selective vs all | **Selective, all pre-checked** — user can deselect individual books |
| Tags | **Imported as categories**, in Calibre's order |
| Multiple formats | **Auto-pick best** — EPUB > AZW3 > MOBI > AZW > PDF > CBZ |
| Entry point | **Settings › Library** (`/settings#library`) |
| Duplicates | **Checked by the backend** against the library index; skipped with a reason |

### Calibre library structure

```
Calibre Library/
  metadata.db              ← SQLite: all metadata + file locations
  Author Name/
    Book Title (42)/
      Book Title - Author.epub
      Book Title - Author.mobi
      cover.jpg
```

`metadata.db` key tables:

| Table | Relevant columns |
|-------|-----------------|
| `books` | `id`, `title`, `path` (relative, e.g. `Author/Title (42)`), `series_index`, `has_cover` |
| `authors` + `books_authors_link` | author name (link-table order = Calibre's order) |
| `series` + `books_series_link` | series name |
| `tags` + `books_tags_link` | tag names |
| `comments` | `text` — HTML description |
| `identifiers` | `type` (`isbn`, `goodreads`, …), `val` |
| `ratings` + `books_ratings_link` | `rating` 0–10 (2 per star) |
| `languages` + `books_languages_link` | `lang_code` (ISO 639-2) |
| `data` | `format`, `name` — actual file is `{library_root}/{path}/{name}.{format}` |

### Metadata mapping

| Calibre | Librarium | Transform |
|---------|-----------|-----------|
| `books.title` | `title` | Calibre's "Unknown" → `''` |
| `authors` (linked) | `author` | Calibre's order, joined with ` & `; `\|` restored to `,`; "Unknown" dropped |
| `series` (linked) | `series` | direct |
| `books.series_index` | `seriesIndex` | only when the book has a series (Calibre gives every book 1) |
| `tags` (linked) | `categories` | in Calibre's order |
| `comments.text` | `description` | HTML → text, paragraphs kept |
| `identifiers` where type=`isbn` | `isbn` | |
| `identifiers` where type=`goodreads` | `goodreadsUrl` | construct URL |
| `languages.lang_code` | `language` | first language, ISO 639-2 → 639-1 |
| `ratings.rating` | `rating` | 0–10 → 1–5 stars, half stars rounded up (7 → 4); 0 = not rated |
| `cover.jpg` | cover | copied to `<library>/.covers/<id>.jpg` (thumbnail made on first display) |

### Format selection

The best available file per book, in this order: **EPUB → AZW3 → MOBI → AZW
→ PDF → CBZ**. Other formats (TXT, RTF, DOCX, `ORIGINAL_*` backups…) are
never imported; a book with none of these is skipped as `unsupportedFormat`.
The chosen file's first bytes must match its format.

### UX flow (`CalibreImportModal`, `stores/calibre.ts`)

1. **Import from Calibre** in **Settings › Library** opens the dialog.
2. Pick: the library found by `detect_calibre_library` ("Use"), or a folder
   picker. `metadata.db` must exist at the root.
3. Browse: `list_calibre_books` lists every book (title, author, series,
   formats, best format; sorted by title); all pre-checked; select/deselect
   individually or all; search across title and author.
4. **Import N books**: `import_from_calibre`, with a progress bar fed by
   `calibre-import-progress` ("x of n", current title) and a Cancel button
   (stops before the next book; closing the dialog cancels too).
5. Summary: imported, skipped (with the reason: already in the library as
   *file*, unsupported format, no file), failed, and what a cancellation or
   an early stop left out. Errors are shown in the dialog, on the step where
   they happened.

### Rust backend

`commands/calibre.rs` (IPC) and `commands/calibre/{detect,db,import}.rs`:

- `detect_calibre_library() → string | null`: `library_path` from Calibre's
  `global.py.json` (in `$CALIBRE_CONFIG_DIRECTORY`, else the OS's Calibre
  config folder), else the first of `~/Calibre Library`,
  `~/Documents/Calibre Library`, `~/Books` holding a `metadata.db`.
- `list_calibre_books(calibrePath) → CalibreBook[]`: four queries whatever
  the library's size.
- `import_from_calibre(calibrePath, bookIds, libraryPath) → ImportResult`
  (`imported`, `skipped`, `failed`, `cancelled`, `error`, `remaining`); one
  import at a time (`CONFLICT` otherwise); `cancel_calibre_import()`.
- `metadata.db` is opened read-only (Calibre may be running; a lock is
  waited for up to 5 s); missing optional tables leave fields empty.
- Books are saved to the library index in batches (50 books or 5 s), each
  under the index lock, so a crash loses at most one batch's entries.
- Runs on the blocking pool; folder and file names from the database must
  be plain relative names.

### Duplicate detection

Done by the backend against the library index (the frontend sends no book
list). A book is skipped as `alreadyInLibrary` (with the library
copy's file name) when the library has a book with the same title and
authors — case-, whitespace- and Unicode-normalisation-insensitive, authors
in any order — or a byte-identical file. A *different* book whose file name
is taken gets a free one (`Name_2.ext`). Books imported earlier in the same
run count as library books.

---

## Feature: Theme System

### Themes

Four themes, each a book cover (see [docs/DESIGN.md](docs/DESIGN.md)), all
dark (there is no light mode), chosen in Settings › Appearance
(`ThemePicker`). The theme is saved in the app config and applied by
setting `data-theme` on `<html>`: App.vue re-applies it whenever
`themeStore.dataTheme` changes.

| Theme | Colours | Accent | Lettering |
|-------|---------|--------|-----------|
| **Nocturne** (default) | Midnight, coral | `#f2836f` coral | Poiret One, Cormorant Garamond |
| **Saphir** | Sapphire, gold leaf, ivory | `#d8b25e` gold | Cinzel, Cormorant Garamond |
| **Folklore** | Ink, vermilion, gold | `#e85a3c` vermilion | Fraunces |
| **Cerulean** | Moonlit silver, mist | `#9ec2ff` silver-blue | Italiana, Cormorant Garamond |

`data-theme` values: `""` (no attribute: Nocturne), `"saphir"`,
`"folklore"`, `"cerulean"`. Older theme names are read as their successors
(`themeNameFrom`: autumn → Folklore, winter → Cerulean, spring → Nocturne,
summer and seasonal → Saphir); an older light/dark mode or
southern-hemisphere setting is ignored.

### Background Animations

`App.vue` renders `components/effects/BackgroundEffects.vue` (theme, level) unless the level is **Off**. Only one effect is active at a time; every root is `aria-hidden`.

**Level** (Settings › Appearance, `settings.effectsLevel`): **Off**, **Subtle** (about half the particles, canvas capped at ~30 fps) or **On** (full count, display rate). Default: Off when the system asks to reduce motion, else Subtle, until the user chooses.

| Condition | Component | Subtle / On |
|-----------|-----------|-------------|
| Nocturne | `StarfieldLayer` (`nocturne`) | Twinkling stars (warm white, a few coral and blue): 90 / 180; a shooting star every ~20 / ~10 s |
| Cerulean | `StarfieldLayer` (`cerulean`) | Silver and lilac moon dust drifting up and right: 90 / 180; a shooting star every ~44 / ~22 s |
| Folklore | `FirefliesLayer` | Embers, hue 12–44: 8 / 16 |
| Saphir | `MagicParticles` | Gold dust, embers and sparkles in the theme's `--accent` hue (re-read on `data-theme` changes): 45 / 100 |

The canvas effects run on `utils/effects/canvasLoop.ts` (`CanvasLoop`): time-based motion (the same speed at 60 and 120 Hz, a step clamped to 100 ms), the fps cap, no frames while the window is hidden (`visibilitychange`), a backing store sized for the device pixel ratio (capped at 2). The particles are plain objects held with `markRaw`, outside Vue's reactivity; the scenes (`particles.ts`, `fireflies.ts`, `starfield.ts`) are pure and tested. A shooting star is a Poisson event (the chance per frame is the frame's duration over the average interval), so it comes as often at any frame rate.

### Theme Store (`src/stores/theme.ts`)

```ts
type ThemeName = 'nocturne' | 'saphir' | 'folklore' | 'cerulean'

// themeNameFrom(saved) — a current name as is, an older name as its successor, else null
// dataTheme      — the data-theme attribute ('' for Nocturne, else the name)
// applyTheme()   — sets data-theme on document.documentElement
// setTheme       — applies, saves; rejects when not saved
```

### UI Transitions

- **Route transitions:** `fade-y` via `<RouterView v-slot="{ Component }">` + `<Transition name="fade-y" mode="out-in">`. Enter: 160ms fade + translateY(10px→0). Leave: 100ms fade + translateY(0→-6px).
- **Book details:** BaseModal's own entrance (`ui-modal-in`: a short rise, scale and fade).

---

## Feature: OPDS Catalog Server

### Summary

An optional HTTP server that exposes the current user's library as an OPDS
catalog — **OPDS 1.2 (Atom)**, which KOReader, Moon+ Reader and most
reading apps need, and **OPDS 2.0 (JSON)** for apps such as Thorium.
Enabled from **Settings › OPDS server**; off by default. Local-only by default;
LAN sharing is opt-in and password-protected. The full reference (commands,
feeds, URLs, limits, what to type in each app) is docs/IPC.md, "OPDS
server".

### Architecture

- **Backend:** `axum` 0.8 server in a `tokio` task inside the Tauri process
  (`commands/opds.rs` + `opds/`: `catalog` (the library as the feeds see
  it, and its cache), `atom` (OPDS 1.2, quick-xml), `json` (OPDS 2.0),
  `files` (downloads and covers, streamed with `tower-http`'s `ServeFile`),
  `access` (Host check, Basic auth), `limits`, `urls`, `qr`).
- **Commands:** `start_opds(port, libraryRoot, shareOnLan?, password?) →
  OpdsStatus` (restarts only when the settings changed, waiting for the old
  server to release the port), `stop_opds`, `opds_status`,
  `invalidate_opds_cache`, `opds_qr_svg(url)`.
  `OpdsStatus = { running, port, localUrl, lanUrls }`.
- **Access:** binds `127.0.0.1` unless `shareOnLan`; then every request
  needs HTTP Basic auth (user `librarium`, the password; loopback
  included). No CORS headers. The `Host` header must be `localhost`, a
  loopback IP or (LAN mode) the IP the client connected to: host names such
  as `mymac.local` are refused, which stops DNS-rebinding pages.
- **Catalog:** built from the index, read only (`library.json`, its
  backup, or an older `books.json`), and one listing of `.covers/`. Rebuilt
  on `invalidate_opds_cache`, on every (re)start, and by itself when those
  files change (checked at most once a second).
- **Limits:** 64 connections, 60 s idle timeout, 16 requests in flight
  (then up to 10 s of queueing, then `503`), 30 s to start a response.
- **State (frontend, `stores/opds.ts`):** `enabled` and `port` (default
  8042) in localStorage; `shareOnLan` and `password` in the config file
  (the password is generated when LAN sharing first needs one, and can be
  regenerated). The users store points the server at the current user's
  folder (`setLibraryRoot`), restarting it on a user switch and stopping it
  while no user is selected. The settings show the local and LAN URLs, a
  QR code of the first LAN URL (the address only, never the password, shown
  as an `<img>`), and hints for KOReader and Thorium.

### Feeds

| Path | Content |
|------|---------|
| `/opds` (also `/`) | Root: All books, Recently added, By author, By series, search links. Atom, or OPDS 2 JSON if `Accept` prefers it |
| `/opds/all`, `/opds/recent` | Every book, by title / newest first (Atom acquisition feeds) |
| `/opds/authors`, `/opds/authors/{name}` | Authors with book counts; one author's books |
| `/opds/series`, `/opds/series/{name}` | Series with book counts; one series in order |
| `/opds/search?q=`, `/opds/opensearch.xml` | Search (title, subtitle, authors, series, ISBN; case- and accent-insensitive); OpenSearch description |
| `/opds/books/{id}` | One book (Atom entry) |
| `/opds/v2`, `/opds/publications?q=`, `/opds/publications/{id}` | OPDS 2.0 root, publications (or search results), one publication |
| `/opds/publications/{id}/download`, `/cover`, `/thumbnail` | The book file (Range, `Content-Disposition`), the full cover, the thumbnail |

Feeds are paginated (`?page`, `?limit`, default 50, at most 200).

### MIME types per format

| Format | MIME |
|--------|------|
| epub | `application/epub+zip` |
| pdf | `application/pdf` |
| mobi (and azw) | `application/x-mobipocket-ebook` |
| azw3 | `application/vnd.amazon.mobi8-ebook` |
| cbz | `application/vnd.comicbook+zip` |

---

## Feature: OPDS Catalog Sources (External)

### Summary

Users can register external OPDS 2.0 catalog URLs in **Settings › Catalogs**.
Each registered source appears as an additional provider in **Discover**
(the free-book search) alongside Project Gutenberg and Open Library.

### Data model

```ts
interface OpdsCatalogSource {
    id:     string   // crypto.randomUUID() — generated on add
    url:    string   // base URL for the OPDS 2.0 publications feed (e.g. http://host/opds/publications)
    name:   string   // display name; if empty, hostname is used
    apiKey: string   // optional; sent as Authorization: Bearer <key>
}
```

Persisted in the config file under `opdsCatalogs` (JSON array; API keys in
plain text).

### Architecture

- **Store:** `src/stores/opdsSources.ts` — Pinia options store; `sources[]`, `initFromConfig()`, `add()`, `remove()`, `update()`, `displayName()` getter.
- **Provider factory:** `src/services/store/providers/opdsCatalog.ts` — `createOpdsCatalogProvider(source)` returns a `StoreProvider` with `id = "opds-<uuid>"`.
- **Requests:** through the backend's `http_get` (`services/http.ts`), since a user's catalog can be on any host, which the webview CSP can't allow in advance. A failure is shown as "catalogue unreachable", not as "no results".
- **DiscoverView integration:** `allProviders` computed combines the static `STORE_PROVIDERS` array with dynamically generated providers from `opdsSourcesStore.sources`. Provider badges and search both use this combined list. Answers are merged by `services/store/merge.ts`: one card per book (same title and author; the EPUB, then the copy with a cover, then the first catalog's), best matches first (the title is the search, starts with it, contains it, then the author), then by title; unknown authors and titles are `''` in the providers and translated in the view; a catalog's failure is explained in the UI language.

### OPDS query format

The provider sets `q=<query>` on the registered URL and parses the OPDS 2.0 JSON response:

- `data.publications[]` → array of publication objects
- Per publication: `metadata.title`, `metadata.author` (string | `{name}` | array), `metadata.language`
- Links: acquisition links (`rel` containing `acquisition` or `open-access`, or typed `application/epub+zip`) in a format the library imports, picked by format: EPUB, then AZW3, MOBI, PDF, CBZ. The format comes from the media type (`application/epub+zip`, `application/pdf`, `application/x-mobipocket-ebook`, `application/vnd.amazon.ebook`, `application/vnd.comicbook+zip`…; parameters ignored) or, for an untyped link, the URL's extension; it names the downloaded file (`download_and_import` checks the bytes against it). Links in other formats (FB2, HTML…) are not offered. Cover: `images[]` (OPDS 2.0) or `links[]`, `rel` containing `opds-spec.org/image` or any `image/*` type.
- Relative `href`s (download and cover) are resolved against the URL the feed was served from (after redirects: `http_get`'s `finalUrl`); only http(s) results are kept.
- API key sent as `Authorization: Bearer <key>` header when set (searches only: the download through `download_and_import` doesn't carry it)

### Settings UI (Settings › Catalogs)

- List of registered catalogs: name + URL, with a Remove IconButton per entry (confirmed first)
- "Add catalog" inline form: URL (required), Name (optional → hostname fallback), API key (optional, password input with show/hide toggle)
- Adding clears the form; URL field is required to enable the Add button

---

## Feature: Analytics

### Summary

Opt-in usage statistics sent to a self-hosted **Umami** server. Nothing is sent, or even queued, until the user says yes in
the first-run consent dialog. What users are told is in PRIVACY.md (shown
in the app by `PrivacyPolicyDialog`, from the consent dialog and Settings ›
Privacy & analytics) and in `AnalyticsDisclosure.vue` (i18n `consent.usage`,
`consent.errors`, `consent.never`), which the consent dialog and the
settings both render: keep them in sync with the code.

**Privacy constraints (hard rules):**
- Never include book titles, filenames, authors, paths, user names, error
  messages or any user-generated text.
- Never include exact numbers: `track()` only takes strings and booleans
  (`EventData`); counts and durations go through the bucketing helpers.
- Never include machine identifiers or device serial numbers (`safeDeviceModel`
  in `services/privacy.ts` turns a serial-looking model into `unknown`).
- Events carry no identifier: no installation ID, and a user OPDS catalog is
  sent as `opds`, not `opds-<its id>`.

### Architecture

- **Service:** `src/services/analytics.ts` — singleton `Analytics` class
  (`analytics`); holds the in-memory consent so `track()` stays
  synchronous; `resolveStoredConsent()` reads the config.
- **Store:** `src/stores/analytics.ts` — `consent` (`'granted' | 'denied' |
  null`), `loaded`, `needsConsent` (App.vue shows the dialog while it's
  true), `grant()`, `deny()`, `toggle()`; persists `analyticsConsent`.
- **Credentials:** `VITE_UMAMI_URL` and `VITE_UMAMI_WEBSITE_ID`, compiled in
  at build time (`.env.local` locally, GitHub secrets in CI). Empty: no-op.
- **Endpoint:** `POST ${VITE_UMAMI_URL}/api/send` (Umami's event API); its
  host is added to the CSP at build time (`scripts/tauri.mjs`). Payload: `website`,
  fixed `hostname` `desktop.librarium.app`, `language`
  (`navigator.language`), `url` `/<event>` (page views: `/<route>`),
  `title`, `name` and `data` (the event's values); page views also send
  `screen` (`<width>x<height>`).

### Consent

| Stored config | Consent | Effect |
|---------------|---------|--------|
| `analyticsConsent: 'granted'` | granted | Events sent |
| `analyticsConsent: 'denied'` | denied | Nothing sent |
| no `analyticsConsent`, `analyticsOptOut: true` (an older opt-out) | denied (saved as such) | Nothing sent; never asked |
| neither | `null` | Nothing sent; the dialog asks (first run) |

Escape or "No thanks" denies. Withdrawing consent clears the queue and the
pending user properties and deletes the installation ID; it sends nothing
(no `analytics_opt_out` event). Granting sends `analytics_opt_in`. Error
reports (`reportError`) follow the same consent. At startup, while no
choice is recorded (`analyticsConsent` absent), the installation ID is
deleted too, and an older `appUid` in the config is set to `null`.

### Event delivery

- Events are **queued** and flushed after a 2-second debounce via `fetch`.
- On app close (`beforeunload`): `app_close`, then a synchronous flush via
  `navigator.sendBeacon`.
- Without consent, or without a website id: every call is a no-op.

### Events tracked

| Event | Trigger | Key params |
|-------|---------|------------|
| `app_start` | App mount | `theme` (+ the user properties below) |
| `app_close` | `beforeunload` | `session_duration_bucket` |
| `view_open` (+ a page view) | Router navigation | `view` (route name) |
| `library_scan` | Library opened or rescanned | `book_count_bucket`, `duration_bucket`, `triggered_by` (`startup` / `manual`) |
| `book_edit` | Metadata save | `format` |
| `book_edit_batch` | Batch metadata save | `book_count_bucket` |
| `tidy_up_search` | Provider lookup | `service`, `found`, `confident` |
| `tidy_up_applied` | User confirms tidy-up | `service`, `field_count_bucket`, `fields` (key names only) |
| `book_convert` | EPUB conversion batch | `book_count_bucket`, `from_formats` |
| `reader_open` | Read button click | `format` |
| `device_connected` | A new device id is selected | `brand`, `model` |
| `transfer_to_device` | Transfer job done | `format`, `converted_from`, `brand`, `model` |
| `transfer_from_device` | Import from device | `format`, `brand`, `model` |
| `store_search` | Discover search | `provider_count_bucket`, `has_opds_source`, `result_count_bucket` |
| `store_download` | Book download | `format`, `source` (provider id; `opds` for any user catalog) |
| `calibre_import` | Calibre import done | `book_count_bucket` |
| `opds_catalog_add` | OPDS catalog registered | — |
| `analytics_opt_in` | User opts in | — |
| `error` | Some caught errors | `component`, `error_type` (fixed strings, never the message) |

### Bucketing helpers (privacy-safe coarse values)

| Helper | Buckets |
|--------|---------|
| `bucketBookCount(n)` (and `bucketCount` in the library store) | `<10`, `10-50`, `50-200`, `200+` |
| `bucketDurationMs(ms)` | `<1s`, `1-5s`, `5-30s`, `30s+` |
| `bucketResultCount(n)` | `0`, `1-5`, `6-20`, `20+` |
| `bucketCount(n)` (`services/privacy.ts`) | `0`, `1`, `2-5`, `6-20`, `21-100`, `100+` (more than 100): books converted or imported, fields applied, sources searched |
| `bucketMinutes(min)` (`services/privacy.ts`) | `<5`, `5-15`, `15-60`, `60+`: the session length |

No event value is an exact number. The sound volume isn't sent.

### User properties (once per session, merged into the next event)

| Property | Value |
|----------|-------|
| `os` | `macos` / `windows` / `linux` |
| `app_version` | from `getVersion()` (Tauri) |
| `language` | `navigator.language` prefix (e.g. `fr`) |

---

## Credits

The main third-party libraries and assets, shown in the app's **About →
Credits** (`CREDITS` in `AboutModal.vue`). The complete list — every Rust
crate linked into the app and every npm package bundled in the frontend,
with versions and licences — is `THIRD_PARTY_NOTICES.md`, generated by `npm
run notices` from the lockfiles.

| Name | License | Author / Source | Notes |
|------|---------|-----------------|-------|
| Vue.js, Pinia | MIT | Evan You; Eduardo San Martin Morote | Frontend framework, state |
| Tauri | MIT / Apache-2.0 | Tauri Contributors | Desktop shell |
| Feather Icons | MIT | Cole Bemis | Icon set via `vue-feather` |
| epub.js | BSD-2-Clause | FuturePress | EPUB rendering in the reader |
| boko | GPL-3.0-or-later | github.com/zacharydenton/boko | Conversion (EPUB ↔ AZW3, MOBI → EPUB); vendored and modified |
| lopdf | MIT | Junfeng Liu | PDF metadata and covers |
| quick-xml | MIT | Johann Tuffe & contributors | EPUB OPF parsing and editing |
| zip | MIT | zip-rs contributors | EPUB and CBZ archives |
| axum | MIT | Tokio contributors | HTTP server for the OPDS catalog |
| whatlang | MIT | Serhii Potapov | Language detection |
| Inter, Cinzel, Poiret One, Fraunces, Italiana, Cormorant Garamond | OFL-1.1 | Rasmus Andersson; Natanael Gama; Denis Masharov; Undercase Type; Santiago Orozco; Christian Thalmann | Bundled fonts (`@fontsource`) |
| Reading background | Unsplash | Pavan Trikutam | Photo |
| Sound effects | — | ZapSplat | UI sound design |

> **Licensing note:** `boko` (GPL-3.0-or-later) is statically linked into
> the binary; it is the only GPL dependency. `deny.toml` lists it as the
> one licence exception, so any new copyleft crate fails CI. *(Not legal
> advice.)*

---

## Feature: Librarium API Integration

Service: `src/services/librarium-api.ts`

Talks to the `librarium-website` backend at `VITE_API_URL` (set by the
release build; its host is added to the CSP at build time by
`scripts/tauri.mjs`). Requests carry
`X-API-Key: VITE_API_KEY` when set; that key ships in the app, so it is
public and grants nothing privileged. Without `VITE_API_URL`, messages and
error reports are no-ops and feedback rejects. What is sent is documented
for users in PRIVACY.md.

### Error Reporting

```typescript
reportError(code: string, context: ErrorReportContext, cause?: unknown): void
```

Fire-and-forget `POST /api/app/report/error` (5 s timeout; never throws).
**Only when the user granted analytics consent.** **No free text:** the
error's message is never sent (messages name files, and paths outside known
folders can't be scrubbed reliably). Body (`errorReportBody()`):

| Field | Value |
|-------|-------|
| `errorCode` | `code`, e.g. `IMPORT_FAILED` (`safeErrorCode`: anything but `[A-Z][A-Z0-9_]*` becomes `UNKNOWN`) |
| `errorContent` | the context key (`ErrorReportContext`: `app.startup`, `users.init`, `import.files`, `library.trash`, `library.forget`, `duplicates.trash`, `duplicates.forget`, `details.rename`, `tidyUp.book`, `tidyUp.batch`, `device.view`, `device.list`, `device.delete`, `calibre.detect`; `safeContextKey`), plus ` (<CATEGORY>)` when `cause` is a backend error whose code is in `ERROR_CODES` (`IO`, `NOT_FOUND`…), e.g. `library.trash (IO)` |
| `appUid` | the installation ID (below) |
| `version` | from `cacheAppVersion()` |
| `os`, `osVersion` | from the user agent |

The server's field names are unchanged; `errorContent` now only holds the
context and category.

### Installation ID

`src/services/installationId.ts`: one random UUID in localStorage
(`appUuid`), sent as `appUid` with error reports and feedback, never with
analytics events. Created on first need (`getInstallationId()`): the first
error report (consent required) or feedback (user-initiated, no consent
needed); never at startup. Deleted (`resetInstallationId()`) by the
analytics store whenever consent is set to `denied` (dialog or switch) and
at startup while no choice is recorded, so an older ID is never reused.

### Feedback

`sendFeedback(message, device?)` — `POST /api/app/report/feedback` (8 s
timeout), only when the user sends the feedback dialog; rejects on failure so
the dialog can say so. Body: `message`, the same `appUid`, then
`feedbackAttachments(device)`: `version`, `os`, `osVersion`, `deviceBrand`,
`deviceModel` (through `safeDeviceModel`; `null` without an e-reader). The
dialog lists those values before sending (it calls the same function).

### Update Checks and Messages

Managed by `src/stores/updates.ts` (Pinia). `checkAtStartup()` runs once on
app startup from `App.vue`: `checkReleases()` if the setting
`autoCheckUpdates` is on, `checkMessages()` if `showNews` is on (both on by
default; Settings › Updates). With both off, nothing is requested at
launch.

| Action | Mechanism | Result |
|--------|-----------|--------|
| `checkReleases()` | `tauri-plugin-updater` `check()` against `latest.json` on GitHub (see DEPLOY.md) | Sets `newRelease: { version, changelog, date }` and keeps the `Update` (`markRaw`) if a newer release exists |
| `downloadAndInstall()` | `update.downloadAndInstall()`, then `relaunch()` (`tauri-plugin-process`) | Errors shown (toast and `installError`) |
| `checkMessages()` | `GET /api/app/messages?from=<lastId>` (5 s timeout; `[]` on any error) | Filters by OS constraint, sets `pendingMessages` |

**New release badge:** `AppSidebar` shows a small green dot on the Settings nav item (and Settings on its Updates section) when `updatesStore.hasNewRelease` is true.

**Settings › Updates** shows the version, its release notes (safe text) and Download & install; "Check now" runs `checkReleases()` again.

**Messages modal:** `MessagesModal` is rendered in `App.vue` (inside the `v-if="usersStore.currentUser"` block) when `updatesStore.pendingMessages.length > 0`. Pressing "Got it" or Escape calls `dismissMessages()`, which saves the highest message ID to `localStorage` key `lastMessageId` so those messages are never shown again.

**Message translations format:** Each `ApiMessage` carries a `translations` JSON object keyed by BCP-47 language code (e.g. `en`, `fr`, `de`). Each value is `{ title: string, content: string }`. The `en` entry is always required. `MessagesModal` resolves the display copy via `translations[currentLang] ?? translations['en']` — falling back to English when the user's language is not present in the message.

### Env Variables

See `.env.example`. All four are compiled into the bundle (public).

| Variable | Description |
|----------|-------------|
| `VITE_API_URL` | Base URL of the `librarium-website` server. Leave empty to disable messages, error reports and feedback. Its host is added to the CSP's `connect-src` by `scripts/tauri.mjs`. |
| `VITE_API_KEY` | Sent as `X-API-Key`. Public. |
| `VITE_UMAMI_URL`, `VITE_UMAMI_WEBSITE_ID` | Umami server (its host is added to the CSP the same way) and website id. Leave empty to disable analytics. |

---

## Known Limitations & Future Work

- **MTP Kindles (2024+)** are experimental (on by default), and validated
  against simulated devices only; their reading progress isn't shown. Code
  paths awaiting a real device are marked `TODO(device-validation)`.
- **Kindle reading progress** needs the book's `.apnx` page map; Librarium
  doesn't generate page maps yet. Old-firmware `.mbp`
  parsing is unvalidated. Kindle model names are guessed from the firmware.
- **Eject** isn't implemented on Windows. The transfer list offers Cancel
  until a job's copy starts (the backend could also stop between 1 MB
  chunks of the copy).
- **Linux** builds haven't been checked on real hardware (detection still
  uses `lsblk`, WebKitGTK rendering, fonts, Trash).
- **Builds aren't OS-signed**: macOS ad-hoc, Windows
  unsigned; the updater's minisign key is the only protection of updates.
- Default library path: `~/Documents/Librarium/` (macOS, Linux) /
  `Documents\Librarium\` (Windows). Overridable globally in Settings › Library.
- No cloud sync.
- The OPDS server serves the current user's library only, over plain HTTP.
- Metadata write-back covers **EPUB only**. PDF, MOBI, AZW3 and CBZ
  metadata lives in the index only; MOBI/AZW3 metadata isn't read from the
  files either (title from the file name). Subtitle and Goodreads URL are
  never written into files.
- A book downloaded from a user OPDS catalog doesn't carry the catalog's API
  key.
- Tidy-up quality depends on title/author accuracy; ISBN lookup is
  preferred when available.
- Reader has no font size control, no light/dark theme toggle, and no
  reading progress sync — always opens at page 1. epub.js 0.3.93 is
  unmaintained.
- CBZ is not supported in the in-app reader, and can't be sent to a Kindle.
- Bundle identifier `com.librarium.desktop` is a domain the project doesn't
  own; changing it needs a migration of the app data (open).
- Privacy: who runs the two servers, how long reports and statistics are
  kept and how to ask for their deletion aren't written down yet (the
  `TODO(owner)` in PRIVACY.md). An older `appUid` is set to `null` rather
  than removed from the settings file (`set_config` can only merge keys).
