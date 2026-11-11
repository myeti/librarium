# Changelog

All notable changes to Librarium. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[Semantic Versioning](https://semver.org).

## [Unreleased] — 0.6.0

The first release.

### Your library

- **A library that is just a folder**, on your computer, an external drive
  or a NAS: EPUB, PDF, MOBI (and `.azw`), AZW3 and CBZ. Import with the file
  picker, ⌘O / Ctrl+O, or by dropping files anywhere in the window.
- **A grid of covers or a list**, fast with thousands of books: search
  (accents ignored), filters (format, author, series, language, category,
  where a book is), sorting that follows the language's rules.
- **Favorites**: star a book and it comes first, whatever the sort.
- **Book details** in a dialog over the library: the cover with Send and
  Read under it, the details beside it, previous / next book with ← →, and
  every action on the book in one bar.
- **Metadata** read from the files, edited one book at a time or several at
  once (a series numbered in the order you choose), and written back into
  EPUB files. Covers from the book, downloaded, or chosen from an image.
- **Find metadata** online (Open Library, and Google Books with your own
  key), with a review of every change before it is saved.
- **A duplicates review** that compares the copies and suggests which one
  to keep.
- **Convert** MOBI and AZW3 books to EPUB; **export** a book as EPUB or AZW3.
- **A built-in reader** for EPUB, PDF, MOBI and AZW3.
- **Several users**, each with their own library.
- **Import from Calibre**: books, authors in their order, tags, ratings,
  languages and covers.
- **Backup and restore**: the whole library, every user with their books,
  covers and metadata, in one zip file.
- **Safe with your files**: deleted books go to the Trash, "Remove from
  library" keeps the file, the index is written atomically with a backup,
  a library on an unmounted NAS is reported as unreachable rather than
  replaced by an empty one, and a book whose file moves keeps its metadata.

### Kindle and Kobo

- **Your books and the e-reader's in one view.** While a Kindle or Kobo is
  connected, its books join the library: a switch shows everything, your
  library or the device, a badge tells where each book is, and the cover
  shows how far you have read.
- **Send in one click**, in the format the device needs: AZW3 for Kindles,
  EPUB for Kobos, converted on the way, with their covers on the Kindle's
  home screen.
- Copy books from the device into the library, remove them from it, eject
  it (macOS and Linux). Several e-readers at once.
- **Experimental:** Kindles from 2024 on, which connect over MTP. Off by
  default.

### More books

- **Discover**: search Project Gutenberg, Open Library and your own OPDS
  catalogs, and download straight into the library.
- **An OPDS server** (1.2 and 2.0) for reading apps such as KOReader,
  Moon+ Reader or Thorium: this computer only by default, or your local
  network with a password and a QR code of the address.

### Look and feel

- **Four dark themes, each dressed like a book cover**: Nocturne, Saphir,
  Folklore and Cerulean, each with its own lettering, ornaments, generated
  covers and background effect (a starfield, gold dust, embers, moon dust).
- **Made for the keyboard**: the grid, the list, menus and dialogs, a
  shortcuts dialog (⌘/ or Ctrl+/), and a native menu bar on macOS.
- Readable in every theme (WCAG AA contrast), with names for screen readers.
- English and French.

### Privacy

- **No account.** Usage statistics and error reports are opt-in: nothing is
  sent unless you say yes. The privacy policy is in the app, with every
  connection Librarium makes and a switch for each.

### Platforms

- macOS (Apple Silicon and Intel), Windows (installer or MSI) and Linux
  (AppImage or `.deb`).
- Updates offered in the app, and installed only if they are signed with
  Librarium's key.
