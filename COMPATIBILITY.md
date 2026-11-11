# Compatibility

E-readers Librarium can send books to, the formats it sends them in, reading
apps that can use its OPDS catalog server, and the systems it runs on.

## At a glance

| Device | Connection | Librarium sends | Reading progress |
|---|---|---|---|
| Kindle (before 2024) | USB drive | **AZW3** (EPUB books are converted); MOBI, AZW3 and PDF as they are | When the Kindle has a page map for the book ([details](#reading-progress-on-a-kindle)) |
| Kindle (2024 and later) | MTP, **experimental**, off by default | Same as above | Not shown yet |
| Kobo | USB drive | **EPUB** (MOBI and AZW3 books are converted); EPUB, PDF and CBZ as they are | Yes |
| Any reading app (KOReader, Moon+ Reader, Thorium…) | OPDS over the network | The book files as they are in your library | — |

A CBZ can't be sent to a Kindle: Librarium can't convert comics, so the
transfer fails with an error. Books protected by DRM can't be converted.

## Kindle

### Formats

Librarium **never sends EPUB to a Kindle**: EPUB books are converted to
AZW3, because Kindles can't open an EPUB file copied over USB. The "EPUB
support" Amazon added in 2022 only applies to books sent through Amazon's
Send to Kindle service, which converts them on Amazon's servers. So:

| Book in your library | Sent to a Kindle as |
|---|---|
| EPUB | AZW3, converted by Librarium |
| AZW3 | AZW3, as is |
| MOBI, AZW | As is |
| PDF | As is |
| CBZ | Not supported (the conversion fails with an error) |

Each AZW3 that Librarium converts carries the book's own identifier as its
ASIN. Librarium
uses it to recognise its copies on the device; the Kindle names cover
thumbnails after it.

A Kindle doesn't make the home-screen cover of such a book itself (it shows
a blank one), so Librarium writes it, from the book's cover in your library,
into the Kindle's `system/thumbnails/` folder, as Calibre does. A book whose
cover is missing there gets it the next time the Kindle is connected.

Sending a book again when it hasn't changed leaves the copy on the Kindle
alone, so the Kindle keeps your place in it. A book that changed in your
library (edited metadata, a new cover) replaces the copy on the Kindle,
which then opens it at the beginning.

### Kindles that connect as a USB drive

Kindles released before 2024 show up as a drive named "Kindle" when plugged
in. Librarium supports them all: send, list, copy back into the library,
delete (with the book's reading data, its `.sdr` folder), and eject on
macOS and Linux. For example:

| Family | Models |
|---|---|
| Paperwhite | Paperwhite 3 (7th gen, 2015), Paperwhite 4 (10th gen, 2018), Paperwhite 5 and Signature Edition (11th gen, 2021) |
| Oasis | Oasis 2 (9th gen, 2017), Oasis 3 (10th gen, 2019) |
| Kindle | Kindle 10th gen (2019), Kindle 11th gen (2022) |
| Scribe | Kindle Scribe, 1st gen (2022) |

**Tested on real hardware** (Kindle Paperwhite 3, macOS, 2026-11-09): device
detection, listing 300+ books (under a second), sending converted books,
re-sending a book (replaced in place, no duplicate), deleting a book with
its `.sdr` folder, ejecting, reading progress, the model name, the
hidden `.librarium/` folder staying out of the Kindle's library, and
(2026-11-10) home-screen covers. Other
models, a Kobo, and Windows/Linux haven't been tried on a device yet.

The model name Librarium shows is guessed from the firmware version, so it
can be wrong on a Kindle that received recent updates (any Kindle on
firmware 5.16 or later shows as "Paperwhite 6"). It doesn't change what is
sent: every Kindle gets AZW3.

### Kindles that connect over MTP (2024 and later), experimental

Kindles released from 2024 on no longer appear as a USB drive: they use MTP,
like a phone. If your Kindle doesn't appear as a drive when you plug it in,
it is one of them. For example:

| Family | Models |
|---|---|
| Paperwhite | Paperwhite 12th gen (2024), and its Signature Edition |
| Colorsoft | Kindle Colorsoft (2024 on) |
| Kindle | Kindle (2024) |
| Scribe | Kindle Scribe (2024) |

The About dialog lists them apart from the USB Kindles, as experimental.

Librarium talks MTP itself (no other software needed), but this support is
**experimental and off by default**: turn on *MTP Kindles (experimental)* in
the device settings. It has only been tested against simulated devices so
far, not on a real 2024 Kindle. On such a Kindle:

- sending, listing, copying back, deleting and releasing ("eject") work like
  on a USB Kindle, but listing is slower;
- titles in the device list come from the file names;
- reading progress isn't shown.

If the Kindle is plugged in but can't be used, the sidebar says why. The
usual causes:

- **macOS:** another app holds it. Quit Image Capture, Photos, Android File
  Transfer and OpenMTP.
- **Linux:** the udev rule `70-librarium-kindle.rules` is missing (the
  `.deb` installs it; with the AppImage, see [the guide](docs/GUIDE.md#kindles-over-mtp-on-linux)), or the file
  manager (gvfs-mtp, kio-mtp) holds the Kindle: unmount it there first.

### Reading progress on a Kindle

Librarium reads the reading position a Kindle keeps next to each book (in
`documents/<book>.sdr/`) and turns it into a percentage with the book's page
map (`.apnx`). When the Kindle has no page map for a book, no progress is
shown for it; Librarium doesn't create page maps yet. Very old Kindles
(firmware 1 to 3) keep the position in `.mbp` files, which Librarium reads
by guesswork: treat that percentage as approximate.

## Kobo

Kobos read EPUB natively, so Librarium sends:

| Book in your library | Sent to a Kobo as |
|---|---|
| EPUB | As is (as `.epub`, not converted to Kobo's `.kepub`) |
| PDF, CBZ | As is |
| MOBI, AZW, AZW3 | EPUB, converted by Librarium |

Kobos show up as a USB drive. Librarium lists the books anywhere on the
device, together with what Kobo's own database (`.kobo/KoboReader.sqlite`)
says about them: title, author, reading progress and "finished" status. It
reads a private copy of that database and never writes to it. Eject works on
macOS and Linux.

Every Kobo with that layout is supported, for example the Clara HD, Clara
2E, Clara Colour, Libra H2O, Libra 2, Libra Colour, Forma, Sage, Elipsa,
Elipsa 2E and Nia.

## Reading apps (OPDS)

Librarium can serve your library as an OPDS catalog that reading apps
browse and download from. Turn it on in Settings › OPDS server. It is
reachable from this computer only, unless you turn on *Share on local
network*; then use the network address shown (an IP address such as
`http://192.168.1.20:8042/opds`, not a name like `my-mac.local`), with the
user name `librarium` and the password shown.

| App | Where to add the catalog | Address |
|---|---|---|
| KOReader (Kobo, Kindle, PocketBook, Android) | Search ▸ OPDS catalog ▸ `+` | `…/opds` (OPDS 1.2) |
| Moon+ Reader, Librera, Aldiko, FBReader | "Net library" or "OPDS catalogs" ▸ add | `…/opds` |
| Thorium Reader (desktop) | Catalogs ▸ Add | `…/opds/v2` (OPDS 2.0), or `…/opds` |

The catalog has: all books, recently added, by author, by series, and
search. Downloads can resume. Details:
[docs/IPC.md, "OPDS server"](docs/IPC.md#opds-server).

## Conversion

Conversions are done by Librarium itself (with the
[boko](https://github.com/zacharydenton/boko) library); no other software is
needed.

| From | To | Used for |
|---|---|---|
| EPUB | AZW3 | Sending to a Kindle; *Export as AZW3* |
| MOBI, AZW | AZW3 | *Export as AZW3* |
| MOBI, AZW, AZW3 | EPUB | Sending to a Kobo; *Convert to EPUB*; *Export as EPUB*; opening in the reader |

Every conversion is checked before it is used: the result must open, have
content, and keep most of the original's text. A failed conversion changes
nothing. *Convert to EPUB* keeps the book's metadata and moves the original
to the Trash. PDF and CBZ are never converted; DRM-protected books are
refused.

## Built-in reader

EPUB and PDF open directly; MOBI, AZW and AZW3 are converted to EPUB the
first time and cached. CBZ isn't supported by the reader.

## Systems

| System | Build | Notes |
|---|---|---|
| macOS 10.15 or later, Apple Silicon | `aarch64.dmg` | |
| macOS 10.15 or later, Intel | `x64.dmg` | |
| Windows 10 and 11, 64-bit | `x64-setup.exe` or `.msi` | Eject isn't available yet: use *Safely Remove Hardware* |
| Linux x86_64 | `.AppImage` (any distribution) or `.deb` (Debian, Ubuntu) | Needs glibc 2.39 or newer (Ubuntu 24.04, Debian 13, Fedora 40 and later). The AppImage needs FUSE 2 |

The macOS minimum comes from the app's configuration
(`bundle.macOS.minimumSystemVersion`); the Linux one from the system the
builds are made on (Ubuntu 24.04). Linux support hasn't been checked on
real hardware yet (e-reader detection, rendering, the
Trash).
