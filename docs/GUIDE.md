# Librarium in detail

Everything the [README](../README.md) leaves out: what each feature does,
how to install on each system, and how to work on the code.

- [Features](#features)
- [Installing](#installing)
- [Development](#development)
- [Documentation](#documentation)
- [Licence](#licence)

## Features

**Library**
- Import by file picker, ⌘O / Ctrl+O, or by dropping files anywhere in the
  window. EPUB, PDF, MOBI (and `.azw`), AZW3 and CBZ.
- Grid and list views that stay fast with thousands of books, fully usable
  from the keyboard; search, filters (format, author, series, language,
  category), sorting, and each book's details in a dialog (← → to the next).
- Favorites: star a book and it comes first, whatever the sort.
- Metadata read from EPUB and PDF files, edited one book at a time or in
  batch (with a series numbered in the order you choose), and written back
  into EPUB files (only the fields you changed).
- Covers taken from the book, downloaded, or chosen from an image file.
- "Find metadata": missing details and covers looked up online (Open
  Library, and Google Books with your own API key), with a review of every
  change.
- Duplicates review: compares the copies and suggests which one to keep.
- Convert MOBI/AZW/AZW3 to EPUB; export a book as EPUB or AZW3.
- Built-in reader for EPUB, PDF, MOBI and AZW3.
- Several users, each with their own folder.
- Import from a Calibre library (metadata, tags, ratings and covers).
- Backup and restore: the whole library (every user, their books, covers
  and metadata) in one zip file.
- Find free books: search Project Gutenberg, Open Library and your own OPDS
  catalogs, and download straight into the library.

**Safe with your files**
- The index (`.librarium/library.json` in the library folder) is written
  atomically, with a backup, and never reset when something goes wrong.
- Deleting moves files to the system Trash (or to `.librarium/trash/` on a
  drive without one). "Remove from library" forgets a book but keeps its file.
- A library on a NAS that isn't mounted is reported as unreachable, never
  silently replaced by an empty folder. A warning appears when another
  computer has the same library open.
- A restore extracts the backup aside first: a cancel, a damaged file or a
  full disk leaves the library as it was.

**E-readers**
- Kindle and Kobo connected over USB, several at once: the device's books
  show in the library with your own (where each book is, its reading
  progress), and a switch shows everything, the library's books or the
  device's. Send books, copy books back into the library, remove them from
  the device, eject it (macOS and Linux).
- Kindles receive AZW3 (EPUB books are converted); Kobos receive EPUB (MOBI
  and AZW3 books are converted). See [COMPATIBILITY.md](../COMPATIBILITY.md).
- Kindles from 2024 on, which connect over MTP rather than as a USB drive:
  experimental support, off by default (a switch in the device settings).

**Reading apps**
- A built-in OPDS catalog server, so reading apps such as KOReader,
  Moon+ Reader or Thorium can browse and download your library. It is
  reachable from this computer only, unless you turn on sharing on the
  local network, which requires a password (with a QR code of the address).

**Everything else**
- Four dark themes, each dressed like a book cover (Nocturne, Saphir,
  Folklore, Cerulean); English and French.
- Usage statistics are opt-in: nothing is sent unless you allow it. See
  [PRIVACY.md](../PRIVACY.md).
- Updates are offered in the app and verified with Librarium's signing key.

## Installing

Download the installer for your system from the latest release on the
[Librarium releases page](https://github.com/myeti/librarium/releases/latest):

| System | File |
|---|---|
| macOS, Apple Silicon | `Librarium_X.Y.Z_aarch64.dmg` |
| macOS, Intel | `Librarium_X.Y.Z_x64.dmg` |
| Windows 10/11, 64-bit | `Librarium_X.Y.Z_x64-setup.exe` (recommended) or the `.msi` |
| Linux x86_64, any distribution | `Librarium_X.Y.Z_amd64.AppImage` |
| Linux x86_64, Debian/Ubuntu | `Librarium_X.Y.Z_amd64.deb` |

### The first launch

The builds are not signed with an Apple or Microsoft developer certificate
yet, so your system warns you the first time:

- **macOS:** open the `.dmg`, drag Librarium into Applications and open it.
  If macOS says it can't verify the developer, go to *System Settings ›
  Privacy & Security*, click **Open Anyway**, and confirm. After an update,
  macOS may ask again for access to your library folder and to removable
  volumes.
- **Windows:** if SmartScreen says "Windows protected your PC", click
  **More info › Run anyway**.
- **Linux, AppImage:** `chmod +x Librarium_X.Y.Z_amd64.AppImage`, then run
  it. If it doesn't start, install FUSE 2 (`sudo apt install libfuse2t64` on
  Ubuntu). The AppImage updates itself from the app.
- **Linux, Debian/Ubuntu:** `sudo apt install ./Librarium_X.Y.Z_amd64.deb`.
  The `.deb` doesn't update itself: install the next release's `.deb` the
  same way.

The Linux builds need glibc 2.39 or newer (Ubuntu 24.04, Debian 13,
Fedora 40 or later). macOS 10.15 or later.

### Kindles over MTP on Linux

The experimental MTP support needs a udev rule that lets you open Amazon
USB devices without root. The `.deb` installs it. With the AppImage,
download `70-librarium-kindle.rules` from the release page and install it
once:

```sh
sudo install -m 644 70-librarium-kindle.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules && sudo udevadm trigger
```

then reconnect the Kindle.

## Development

Built with [Tauri v2](https://tauri.app) (Rust) and
[Vue 3](https://vuejs.org) (Options API, TypeScript).

### Prerequisites

- [Node.js](https://nodejs.org) 20.19 or newer (`.nvmrc` pins the version CI uses).
- [Rust](https://rustup.rs) through rustup. The toolchain version is pinned
  in `rust-toolchain.toml`; rustup installs it on first use.
- The Tauri prerequisites for your system:
  [tauri.app/start/prerequisites](https://tauri.app/start/prerequisites/).
  On Debian/Ubuntu that is `build-essential pkg-config file
  libwebkit2gtk-4.1-dev libsoup-3.0-dev libjavascriptcoregtk-4.1-dev
  libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libssl-dev
  libxdo-dev` (the list CI installs).

### Commands

```bash
npm ci                  # install the exact dependency versions from package-lock.json
npm run dev             # run the app (Vite dev server on port 1420 + the Rust backend)
npm run vue:dev:mock    # the UI alone in a browser, against a fake backend (http://localhost:5173)
npm test                # frontend tests (vitest)
npm run validate        # everything CI checks: vue-tsc, eslint, vitest, IPC contract, clippy, cargo test
npm run build           # an unsigned build for this computer, in src-tauri/target/release/bundle/
```

Also: `npm run lint:fix` (fix formatting), `npm run check:ipc` (frontend
`invoke` calls against the Rust commands), `npm run notices` (regenerate
`THIRD_PARTY_NOTICES.md`), `npm run bump` (set the version everywhere), and
`cargo test --manifest-path src-tauri/Cargo.toml` for the Rust tests alone.

`vue:dev:mock` needs no Rust toolchain: it loads `src/dev/mockBackend.ts`
(two users, a few dozen books, a connected Kindle; `?mtp=<state>` adds an MTP
Kindle) so every screen can be opened and screenshotted in a browser.
`?showcase=<user>` shows a real library instead, with its covers, from a
copy of its index and cover thumbnails in `debug/showcase/<user>/`
(`library.json` and `.covers/thumbs/`; `debug/` is git-ignored): that is
how the README's screenshot is taken.

The server addresses used for messages, error reports, feedback and
statistics are build-time settings: copy `.env.example` to `.env.local` to
set them. Left empty, those features are off.

Releases are built and signed by GitHub Actions from a version tag: see
[DEPLOY.md](../DEPLOY.md).

### Project structure

```
src/                     Vue 3 frontend (Options API, TypeScript)
  components/            Screens' parts; ui/ is the shared component kit, library/ the library screen
  views/                 One component per page
  stores/                Pinia stores (library, devices, users, settings, …)
  services/              The backend (api.ts: one typed function per command), metadata and
                         book providers, analytics, the Librarium API
  i18n/                  English and French strings, typed keys
  utils/                 Pure helpers (formatting, sorting, ISBNs, virtual scrolling, …)
  dev/mockBackend.ts     Fake backend for `npm run vue:dev:mock`
src-tauri/               Rust backend
  src/commands/          Tauri commands: library/, device/ (incl. MTP), opds/, calibre/, backup/, reader/, config, http
  src/epub/              EPUB reading and editing (quick-xml)
  src/mtp/               MTP transport for 2024+ Kindles (USB on macOS/Linux, WPD on Windows)
  src/convert.rs         Format conversion (boko)
  src/net.rs, fsutil.rs  The one HTTP client; atomic writes, locks, trash
  vendor/boko/           boko 0.6.0, vendored and patched (Kindle text fix, per-book ASIN, fixed dates)
  linux/                 udev rule for MTP Kindles
scripts/                 Version bump, release, IPC check, notices, CI helpers
```

## Documentation

| File | What it covers |
|---|---|
| [docs/IPC.md](IPC.md) | The contract between frontend and backend: every command, event, error code and on-disk format |
| [docs/DESIGN.md](DESIGN.md) | The design: themes as book covers, type, ornaments, generated covers, how to add a theme |
| [SPECS.md](../SPECS.md) | How the app is built: conventions, structure, features, design system |
| [COMPATIBILITY.md](../COMPATIBILITY.md) | E-readers, formats, reading apps and platforms |
| [PRIVACY.md](../PRIVACY.md) | What the app sends, when, and what it stores |
| [CHANGELOG.md](../CHANGELOG.md) | What changed in each version |
| [DEPLOY.md](../DEPLOY.md) | Releasing: CI, signing, secrets, the updater |
| [THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md) | Every third-party component and its licence |

## Licence

Librarium is proprietary software: see [LICENSE](../LICENSE).

It includes open-source components, listed with their licences in
[THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md). One of them, the
[boko](https://github.com/zacharydenton/boko) converter, is licensed
GPL-3.0-or-later; it is vendored in `src-tauri/vendor/boko` with dated
modifications and statically linked into the app. Librarium's own licence
is being made compatible with it.
