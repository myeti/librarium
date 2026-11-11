# Librarium privacy policy

*Applies to Librarium 0.6.0. Last updated 2026-11-11.*

Librarium is a desktop app. Your books, their metadata and your reading
progress stay on your computer, on the drives or NAS where you keep your
library, and on your e-readers. Librarium has no account and no cloud
library.

It does talk to a few servers. This page lists every one of them: what is
sent, when, and how to turn it off. It is written from the app's source code
(`src/services/analytics.ts`, `src/services/librarium-api.ts`,
`src/services/privacy.ts`, `src/services/installationId.ts`,
`src/stores/updates.ts` and the metadata and catalog providers); when those
change, this page must change with them. The app shows this page too: *Read
the privacy policy* in the statistics dialog and in Settings › Privacy &
analytics.

<!-- TODO(owner): add who operates the Librarium server and the analytics
server (name, contact address), how long error reports, feedback and
statistics are kept, and how to ask for their deletion. None of that is
defined in the app's code. -->

## At a glance

| What | When | Sent to | Can you turn it off? |
|---|---|---|---|
| [Usage statistics](#usage-statistics) | Only if you allow it | The developer's analytics server (Umami) | Off unless you say yes |
| [Error reports](#error-reports) | Only if you allow statistics | The Librarium server | Same switch as statistics |
| [Feedback](#feedback) | When you click *Send feedback* | The Librarium server | Only sent when you send it |
| [What's new messages](#whats-new-messages) | At each launch | The Librarium server | Yes: *Show news from Librarium* (Settings › Updates), on by default |
| [Update check](#updates) | At each launch | GitHub | Yes: *Check for updates automatically* (Settings › Updates), on by default |
| [Looking up metadata](#looking-up-metadata-tidy-up) | When you tidy up books | Open Library; Google Books if you set an API key | Only when you use it |
| [Finding free books](#finding-free-books) | When you search or download | Gutendex, Project Gutenberg, Open Library, Internet Archive, your OPDS catalogs | Only when you use it |
| [OPDS server](#the-opds-server) | If you turn it on | Reading apps on this computer, or on your network | Off by default |

Apart from the OPDS server, which hands your books to the reading apps you
connect to it, nothing on this list sends the contents of your books.

## Usage statistics

**Off unless you allow it.** The first time you open Librarium a dialog
asks *"Help improve Librarium?"*. Until you answer, and if you answer *No
thanks* or press Escape, nothing is sent and nothing is even queued.

You can change your mind at any time with the switch **Send anonymous usage
statistics and error reports** in Settings › Privacy & analytics. Switching
it off saves the choice, throws away any event not yet sent, deletes the
[installation ID](#error-reports), and sends nothing more, not even an
"opted out" event. Switching it on sends one `analytics_opt_in` event.

**Where it goes.** To an [Umami](https://umami.is) analytics server run by
the developer. Librarium sets no cookie. Like any
web server, it receives your IP address and the user agent of the app's web
view (which names your operating system); according to Umami's
documentation, Umami doesn't store IP addresses and uses the request to
estimate a rough location and to group the events of one visit.

**What is sent.** Each event has a name and a few values. No value is an
exact number: counts are sent as ranges (`0`, `1`, `2-5`, `6-20`, `21-100`,
`100+`) unless the table gives other ranges, and durations as ranges too.

| Event | When | Values |
|---|---|---|
| `app_start` | The app opens | theme; with the first event of a session: OS family (`macos`, `windows`, `linux`), app version, language (two letters, e.g. `fr`) |
| `app_close` | The app closes | session length, as a range: `<5`, `5-15`, `15-60` or `60+` minutes |
| `view_open` (and a page view) | You open a screen | the screen's name (`library`, `device`, `discover`, `settings`) |
| `library_scan` | The library is read | number of books (`<10`, `10-50`, `50-200`, `200+`); duration (`<1s`, `1-5s`, `5-30s`, `30s+`); `startup` or `manual` |
| `book_edit`, `book_edit_batch` | You save metadata | the book's format; for a batch, the number of books (`<10` … `200+`) |
| `tidy_up_search`, `tidy_up_applied` | You tidy up a book | the service (`googlebooks`, `openlibrary`), whether it found a match; the number of fields applied (a range) and their names (e.g. `title,isbn`) |
| `book_convert` | You convert books to EPUB | number of books (a range), their original formats |
| `reader_open` | You open the reader | the book's format |
| `device_connected` | An e-reader is detected | brand (`kindle`, `kobo`), model (e.g. `Kindle Paperwhite 5`) |
| `transfer_to_device`, `transfer_from_device` | A book is sent to or copied from the e-reader | format, the format it was converted from, brand, model |
| `store_search`, `store_download` | You search for or download a free book | number of sources searched (a range), whether you have OPDS catalogs, number of results (`0`, `1-5`, `6-20`, `20+`); the format and the source (`gutendex`, `openlibrary`, or `opds` for any of your catalogs) |
| `calibre_import` | A Calibre import ends | number of books imported (a range) |
| `opds_catalog_add` | You add an OPDS catalog | — |
| `analytics_opt_in` | You allow statistics | — |
| `error` | Something fails | where (e.g. `transfer`, `library_scan`) and a fixed error type (e.g. `transfer_failed`); never the error message |

Every event also carries your system's language setting (e.g. `fr-FR`) and a
fixed host name (`desktop.librarium.app`). Page views also carry your screen
resolution (e.g. `2560x1440`).

**What is never sent with statistics:** book titles, authors, file names,
folder paths, error messages, user names, or anything you typed. No
installation ID or other persistent identifier is attached to the events. A
device model that contains something looking like a serial number is
replaced with `unknown`.

## Error reports

**Only if you allowed statistics** (same switch). When certain operations
fail, Librarium sends a report to the Librarium server, run by the
developer. A report holds no free text, only fixed values:

- an error code, such as `IMPORT_FAILED`;
- where it happened, a fixed name: `app.startup` and `users.init` (loading
  the list of users), `import.files` (importing), `library.trash` and
  `duplicates.trash` (moving to the Trash), `library.forget` and
  `duplicates.forget` (removing from the library), `details.rename`
  (renaming), `tidyUp.book` and `tidyUp.batch` (tidying up), `device.view`
  and `device.list` (reading the books on an e-reader), `device.delete`
  (deleting one), `calibre.detect` (finding a Calibre library);
- for errors from the app's own core, their category, from a fixed list
  (such as `IO`, `NOT_FOUND` or `LIBRARY_UNAVAILABLE`);
- the app version, the operating system and its version as the web view
  reports them;
- a random installation ID (see below).

**Never the error message.** Messages can name a book or a folder ("Dune.epub
can't be added to the library"), so they stay on your computer.

**The installation ID** is a random identifier. It links reports from the
same installation together, and to feedback you send; it says nothing about
who you are. It is created the first time a report or a feedback message
needs it (never at startup, and never for statistics, which carry no
identifier) and kept in the app's web storage. Sending feedback creates it
too, even with statistics off. It is deleted each time you say no to
statistics: *No thanks* in the dialog, or switching them off; the next one
will be unrelated.

Each request also carries an API key built into the app. It identifies the
app, not you.

## Feedback

Only when you write a message and click *Send feedback*. Sent to the
Librarium server: your message (up to 2,000 characters), the same
random [installation ID](#error-reports) as error reports (created if there
is none yet), the app version, the operating system and its version, and
the brand and model of the connected e-reader, if one is connected (a model
that looks like it contains a serial number is sent as `unknown`). The
feedback dialog lists these, with their values, before you send. Feedback
doesn't depend on the statistics setting, since you choose to send it.
Don't put anything in it you don't want the developer to read.

## What's new messages

At each launch, Librarium asks the Librarium server for announcements
("What's new") newer than the last one you dismissed. The request carries
the number of that last message and the app's API key; no installation ID.
The server sees your IP address, as with any request. Messages are shown as
plain text (links open in your browser); they can't run code in the app.

To stop it, turn off **Show news from Librarium** in Settings › Updates
(from the next launch).

## Updates

At each launch, Librarium downloads a small file (`latest.json`) from GitHub
(`github.com/myeti/librarium`) to see whether a new version exists.
If you install an update, it is downloaded from GitHub too, and installed
only if its signature matches Librarium's signing key. No identifier is
sent; GitHub sees your IP address.

To stop the check at launch, turn off **Check for updates automatically**
in Settings › Updates (from the next launch). *Check now*, on the same
page, checks once whatever that setting.

## Looking up metadata (Tidy up)

Only when you ask Librarium to tidy up a book or a selection of books. For
each book it sends the ISBN, or the title, author and language (detected on
your computer), to:

- **Google Books** (`www.googleapis.com`), only if you entered your own
  Google Books API key in the settings. The key is sent with each request,
  so Google can link these searches to your Google Cloud project.
- **Open Library** (`openlibrary.org`).

The covers they propose are shown straight from their servers
(`covers.openlibrary.org`, Google's image servers). A cover you accept is
then downloaded by Librarium.

## Finding free books

Only when you search or download:

- your search words go to **Gutendex** (`gutendex.com`, a catalog of Project
  Gutenberg) and **Open Library** (`openlibrary.org`); for Open Library
  results, Librarium asks the **Internet Archive** (`archive.org`) which
  files can be downloaded;
- the covers of the results are shown straight from those services;
- a book you download comes from Project Gutenberg (`gutenberg.org`) or the
  Internet Archive, fetched by the app itself (user agent
  `Librarium/<version>`);
- for each **OPDS catalog you added**, your search words go to that catalog's
  server, with its API key if you entered one. Catalogs are only contacted
  when you search them.

## The OPDS server

Off by default. When you turn it on, reading apps can browse and download
your library (the current user's books):

- **This computer only** (the default): the server answers on `127.0.0.1`,
  so only apps running on your computer can reach it. Web pages open in
  your browser can't read it (it sends no CORS headers, and it refuses
  requests made under any host name but `localhost` or the address it
  listens on).
- **Share on local network**: other devices on your network can reach it,
  with the user name `librarium` and a password Librarium generates (you can
  make a new one). The connection is plain HTTP: on the network, the
  password and the books are not encrypted. Turn it on only at home or on a
  network you trust. The QR code shown in the settings contains the address,
  never the password.

The server never sends anything to the internet by itself.

## What Librarium stores on your computer

**In your library folder** (by default `~/Documents/Librarium`, with one
folder per user; each user's folder holds):

| File | Content |
|---|---|
| Your book files | As you imported them |
| `.librarium/library.json` (and `.bak`) | The index: each book's metadata, date added, file name and size |
| `.librarium/failed.json` | Files that couldn't be read as books |
| `.librarium/lock.json` | The **computer name**, process number and time of the Librarium that has the library open. On a shared drive or NAS, anyone who can read the folder can see it |
| `.librarium/trash/` | Books you deleted, on drives without a system Trash |
| `.covers/` | Cover images and thumbnails |

A backup (Settings › Backup) is a zip of these folders, every user's,
written where you choose. It is never sent anywhere; anyone who can read
the file can read your books and their metadata.

**In the app's data folders** (`com.librarium.desktop`):

| What | Where |
|---|---|
| Settings, `librarium-config.json`: library folder, theme, language, your statistics choice, the update and news settings, the OPDS sharing password and your **Google Books API key** (in plain text), your OPDS catalogs (with their API keys) | macOS `~/Library/Application Support/com.librarium.desktop/`, Windows `%APPDATA%\com.librarium.desktop\`, Linux `~/.local/share/com.librarium.desktop/` |
| Log file, `Librarium.log` (five files of 5 MB at most): what the debug console shows, including file names and paths. It never leaves your computer | macOS `~/Library/Logs/com.librarium.desktop/`, Windows `%LOCALAPPDATA%\com.librarium.desktop\logs\`, Linux `~/.local/share/com.librarium.desktop/logs/` |
| Reader cache: Kindle books converted to EPUB for the reader, 500 MB at most, safe to delete | macOS `~/Library/Caches/com.librarium.desktop/reader/`, Windows `%LOCALAPPDATA%\com.librarium.desktop\reader\`, Linux `~/.cache/com.librarium.desktop/reader/` |
| Web storage: the current user, view preferences, the OPDS server switch and port, the last message seen, and the installation ID once a report or feedback needed it | Managed by the system's web view |

The debug console (Ctrl+Shift+D) has a *Copy all* button. What you copy is
yours to share; keys and passwords in web addresses (such as the Google
Books API key) are hidden in it.

**On your e-readers**, in `.librarium/` at the root of the device
(`device.json`, or `device-1.json` and `device-2.json` on a Kindle connected
over MTP): a random device identifier and, for each book Librarium sent, its
place on the device, title, author, format, size and the date it was sent. The copies
Librarium sends carry the random identifier of the book in your library
(inside the EPUB, or as the AZW3's ASIN); it isn't linked to you.

Librarium never stores a device's serial number.

## Removing your data

- **From your computer:** uninstall Librarium, then delete the folders in
  the table above. Delete your library folder only if you no longer want
  your books.
- **From an e-reader:** delete its `.librarium` folder.
- **From the servers:** switching statistics off stops everything that
  depends on it, and deletes the installation ID from your computer, so
  what was sent before can no longer be linked to what you send later.

## Builds you make yourself

The server addresses are set when the app is built (`.env.example`). A build
without them sends no statistics, error reports, feedback or message
requests.
