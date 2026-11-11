//! Reading a Calibre library's database (`metadata.db`), read-only.
//!
//! The tables and columns used, a small subset of Calibre's schema
//! (`resources/metadata_sqlite.sql` in Calibre's sources):
//!
//! | Table(s) | Columns read | Notes |
//! |---|---|---|
//! | `books` | `id, title, path, series_index, has_cover` | `path` is the book's folder, relative to the library, `/`-separated (`Frank Herbert/Dune (12)`). `series_index` is set (1.0 by default) for *every* book, in a series or not. |
//! | `data` | `book, format, name` | One row per file: `<path>/<name>.<format in lower case>`. |
//! | `authors` + `books_authors_link` | `name` | Commas in names are stored as `\|`. |
//! | `series` + `books_series_link` | `name` | At most one per book. |
//! | `tags` + `books_tags_link` | `name` | |
//! | `ratings` + `books_ratings_link` | `rating` | 0–10: two points per star, odd values are half stars. |
//! | `comments` | `text` | HTML. |
//! | `identifiers` | `type, val` | `isbn`, `goodreads` (any case). |
//! | `languages` + `books_languages_link` | `lang_code` | ISO 639-2 codes (`fra`). The link table's column is also called `lang_code`. |
//!
//! Lists (authors, tags, languages) are read in the order of the link
//! table's `id`, which is the order Calibre itself loads and shows them in:
//! without it, SQLite may return "Herbert & Anderson" one day and "Anderson
//! & Herbert" the next.
//!
//! Only `books` and `data` are required. Every other table is optional: if
//! it is missing (very old libraries) the field stays empty; if a query on
//! it fails (an unexpected schema), the field stays empty and a warning is
//! logged once. One odd table never stops a whole import.
//!
//! [`CalibreDb::list`] reads the whole library in four queries whatever its
//! size (no query per book), so a 10 000-book library lists in well under a
//! second. [`CalibreDb::record`] reads one book (for the import) with
//! prepared, cached statements on indexed columns.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{params, Connection, OpenFlags, Params, Row};

use crate::error::{AppError, AppResult};
use crate::logging::Log;

/// The database file of a Calibre library.
pub(crate) const DB_FILE: &str = "metadata.db";

/// How long a query waits while Calibre (running meanwhile) holds a write
/// lock on its database, before giving up.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// One file of a Calibre book.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CalibreFile {
    /// Lower case, as in the file's extension: `epub`, `azw3`, `txt`…
    pub format: String,
    /// The file's name without its extension.
    pub name: String,
}

impl CalibreFile {
    /// The file's name in the book's folder: `<name>.<format>`.
    pub(crate) fn file_name(&self) -> String {
        format!("{}.{}", self.name, self.format)
    }
}

/// What Librarium reads about one Calibre book, as Calibre stores it (the
/// mapping to Librarium's metadata is in `import`).
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct CalibreRecord {
    pub id: i64,
    pub title: String,
    /// In Calibre's order, commas restored (`|` in the database).
    pub authors: Vec<String>,
    /// The book's folder, relative to the library root, `/`-separated.
    pub folder: String,
    pub has_cover: bool,
    pub series: Option<String>,
    /// `books.series_index`: meaningful only with a `series`.
    pub series_index: Option<f64>,
    /// In the order Calibre added them.
    pub files: Vec<CalibreFile>,
    pub tags: Vec<String>,
    /// Calibre's 0–10 value.
    pub rating: Option<f64>,
    /// HTML.
    pub comments: Option<String>,
    pub isbn: Option<String>,
    /// The Goodreads book id.
    pub goodreads_id: Option<String>,
    /// ISO 639-2 codes, first is the main language.
    pub languages: Vec<String>,
}

/// An open, read-only Calibre database.
pub(crate) struct CalibreDb {
    /// The Calibre library folder (where `metadata.db` is; the books'
    /// folders are relative to it).
    root: PathBuf,
    conn: Connection,
    /// The tables (and views) the database has.
    tables: HashSet<String>,
    /// Queries that already logged a warning (each warns once).
    warned: RefCell<HashSet<&'static str>>,
}

/// Calibre stores commas in author names as `|` (the comma separates
/// "Last, First" sort names): `Doe| Jr.` is `Doe, Jr.`.
fn author_name(raw: &str) -> String {
    raw.replace('|', ",").trim().to_string()
}

/// A trimmed, non-empty text, or `None`.
fn non_empty(text: Option<String>) -> Option<String> {
    text.map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

/// All rows of `sql`, mapped by `map`; the first failing row fails the query.
fn rows<T>(
    conn: &Connection,
    sql: &str,
    params: impl Params,
    map: impl FnMut(&Row) -> rusqlite::Result<T>,
) -> rusqlite::Result<Vec<T>> {
    let mut stmt = conn.prepare_cached(sql)?;
    let rows = stmt.query_map(params, map)?;
    rows.collect()
}

/// A `data` row as a file. The format is lower-cased: Calibre stores it in
/// upper case, and the file's extension is the lower-case form.
fn file_of(format: String, name: String) -> CalibreFile {
    CalibreFile {
        format: format.trim().to_lowercase(),
        name,
    }
}

impl CalibreDb {
    /// Open the database of the Calibre library in `root`, read-only.
    ///
    /// `NOT_FOUND` if there is no `metadata.db`; `INVALID_INPUT` if it isn't
    /// a Calibre database (no `books` or `data` table).
    pub(crate) fn open(root: &Path) -> AppResult<Self> {
        let path = root.join(DB_FILE);
        if !path.is_file() {
            return Err(AppError::NotFound(format!(
                "There is no {DB_FILE} in this folder — is it a Calibre library?"
            )));
        }
        let conn = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|e| AppError::Io(format!("Could not open the Calibre database: {e}")))?;
        conn.busy_timeout(BUSY_TIMEOUT).map_err(db_error)?;
        let not_calibre = || {
            AppError::InvalidInput(format!(
                "{DB_FILE} in this folder is not a Calibre database"
            ))
        };
        // The first read: SQLite only notices here that a file isn't a database at all.
        let tables = rows(
            &conn,
            "SELECT name FROM sqlite_master WHERE type IN ('table', 'view')",
            [],
            |r| r.get::<_, String>(0),
        )
        .map_err(|e| {
            if e.sqlite_error_code() == Some(rusqlite::ErrorCode::NotADatabase) {
                not_calibre()
            } else {
                db_error(e)
            }
        })?
        .into_iter()
        .collect::<HashSet<_>>();
        if !tables.contains("books") || !tables.contains("data") {
            return Err(not_calibre());
        }
        Ok(Self {
            root: root.to_path_buf(),
            conn,
            tables,
            warned: RefCell::default(),
        })
    }

    /// The Calibre library folder.
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// Whether the database has all of `tables`.
    fn has(&self, tables: &[&str]) -> bool {
        tables.iter().all(|t| self.tables.contains(*t))
    }

    /// The rows of a query on optional `tables`: none if a table is
    /// missing; none, with a warning (once per query), if the query fails.
    fn optional<T>(
        &self,
        tables: &[&str],
        what: &'static str,
        log: Log,
        query: impl FnOnce(&Connection) -> rusqlite::Result<Vec<T>>,
    ) -> Vec<T> {
        if !self.has(tables) {
            return vec![];
        }
        query(&self.conn).unwrap_or_else(|e| {
            if self.warned.borrow_mut().insert(what) {
                log(format!("[calibre] WARNING — could not read the {what} of the Calibre library (left empty): {e}"));
            }
            vec![]
        })
    }

    /// Every book of the library, with what the picker shows: title,
    /// authors, series, files, cover flag. Four queries in all.
    pub(crate) fn list(&self, log: Log) -> AppResult<Vec<CalibreRecord>> {
        let mut books = rows(
            &self.conn,
            "SELECT id, title, path, series_index, has_cover FROM books ORDER BY id",
            [],
            |r| {
                Ok(CalibreRecord {
                    id: r.get(0)?,
                    title: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    folder: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    series_index: r.get(3)?,
                    has_cover: r.get::<_, Option<i64>>(4)?.unwrap_or(0) != 0,
                    ..CalibreRecord::default()
                })
            },
        )
        .map_err(db_error)?;
        let position: HashMap<i64, usize> =
            books.iter().enumerate().map(|(i, b)| (b.id, i)).collect();
        let index_of = |id: i64| position.get(&id).copied();

        let authors = self.optional(&["authors", "books_authors_link"], "authors", log, |c| {
            rows(c, "SELECT l.book, a.name FROM books_authors_link l JOIN authors a ON a.id = l.author ORDER BY l.id", [], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
            })
        });
        for (id, name) in authors {
            if let Some(i) = index_of(id) {
                books[i].authors.push(author_name(&name));
            }
        }

        let series = self.optional(&["series", "books_series_link"], "series", log, |c| {
            rows(c, "SELECT l.book, s.name FROM books_series_link l JOIN series s ON s.id = l.series ORDER BY l.id", [], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
            })
        });
        for (id, name) in series {
            if let Some(i) = index_of(id) {
                // Calibre allows one series per book; keep the first if not.
                books[i].series = books[i].series.take().or_else(|| non_empty(Some(name)));
            }
        }

        let files = rows(
            &self.conn,
            "SELECT book, format, name FROM data ORDER BY id",
            [],
            |r| Ok((r.get::<_, i64>(0)?, file_of(r.get(1)?, r.get(2)?))),
        )
        .map_err(db_error)?;
        for (id, file) in files {
            if let Some(i) = index_of(id) {
                books[i].files.push(file);
            }
        }
        Ok(books)
    }

    /// Everything the import reads about book `id`; `None` if the library
    /// has no such book (deleted in Calibre since it was listed).
    pub(crate) fn record(&self, id: i64, log: Log) -> AppResult<Option<CalibreRecord>> {
        let found = rows(
            &self.conn,
            "SELECT title, path, series_index, has_cover FROM books WHERE id = ?1",
            params![id],
            |r| {
                Ok(CalibreRecord {
                    id,
                    title: r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                    folder: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    series_index: r.get(2)?,
                    has_cover: r.get::<_, Option<i64>>(3)?.unwrap_or(0) != 0,
                    ..CalibreRecord::default()
                })
            },
        )
        .map_err(db_error)?;
        let Some(mut book) = found.into_iter().next() else {
            return Ok(None);
        };

        book.files = rows(
            &self.conn,
            "SELECT format, name FROM data WHERE book = ?1 ORDER BY id",
            params![id],
            |r| Ok(file_of(r.get(0)?, r.get(1)?)),
        )
        .map_err(db_error)?;

        let texts = |tables: &[&str], what: &'static str, sql: &str| -> Vec<String> {
            self.optional(tables, what, log, |c| {
                rows(c, sql, params![id], |r| r.get::<_, Option<String>>(0))
            })
            .into_iter()
            .filter_map(non_empty)
            .collect()
        };

        book.authors = texts(
            &["authors", "books_authors_link"],
            "authors",
            "SELECT a.name FROM books_authors_link l JOIN authors a ON a.id = l.author WHERE l.book = ?1 ORDER BY l.id",
        )
        .iter()
        .map(|a| author_name(a))
        .collect();
        book.series = texts(
            &["series", "books_series_link"],
            "series",
            "SELECT s.name FROM books_series_link l JOIN series s ON s.id = l.series WHERE l.book = ?1 ORDER BY l.id",
        )
        .into_iter()
        .next();
        book.tags = texts(
            &["tags", "books_tags_link"],
            "tags",
            "SELECT t.name FROM books_tags_link l JOIN tags t ON t.id = l.tag WHERE l.book = ?1 ORDER BY l.id",
        );
        book.languages = texts(
            &["languages", "books_languages_link"],
            "languages",
            "SELECT g.lang_code FROM books_languages_link l JOIN languages g ON g.id = l.lang_code WHERE l.book = ?1 ORDER BY l.id",
        );
        book.comments = texts(
            &["comments"],
            "comments",
            "SELECT text FROM comments WHERE book = ?1 ORDER BY id",
        )
        .into_iter()
        .next();

        book.rating = self
            .optional(&["ratings", "books_ratings_link"], "ratings", log, |c| {
                rows(
                    c,
                    "SELECT r.rating FROM books_ratings_link l JOIN ratings r ON r.id = l.rating WHERE l.book = ?1 ORDER BY l.id",
                    params![id],
                    |r| r.get::<_, Option<f64>>(0),
                )
            })
            .into_iter()
            .flatten()
            .next();

        let identifiers = self.optional(&["identifiers"], "identifiers", log, |c| {
            rows(
                c,
                "SELECT lower(type), val FROM identifiers WHERE book = ?1 ORDER BY id",
                params![id],
                |r| {
                    Ok((
                        r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                        r.get::<_, Option<String>>(1)?,
                    ))
                },
            )
        });
        for (kind, value) in identifiers {
            let slot = match kind.trim() {
                "isbn" => &mut book.isbn,
                "goodreads" => &mut book.goodreads_id,
                _ => continue,
            };
            if slot.is_none() {
                *slot = non_empty(value);
            }
        }
        Ok(Some(book))
    }
}

/// A database error while reading the Calibre library.
fn db_error(e: rusqlite::Error) -> AppError {
    AppError::Io(format!("Could not read the Calibre database: {e}"))
}

/// A Calibre-like `metadata.db` for tests: the schema subset this module
/// reads (with Calibre's real column names), and helpers to fill it.
#[cfg(test)]
pub(crate) mod fixture {
    use super::*;

    /// The tables of Calibre's schema that Librarium reads, as Calibre
    /// creates them (columns Librarium doesn't read are left out).
    pub(crate) const SCHEMA: &str = "
        CREATE TABLE books (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL DEFAULT 'Unknown',
            sort TEXT, path TEXT NOT NULL DEFAULT '', series_index REAL NOT NULL DEFAULT 1.0, has_cover BOOL DEFAULT 0);
        CREATE TABLE data (id INTEGER PRIMARY KEY, book INTEGER NOT NULL, format TEXT NOT NULL,
            uncompressed_size INTEGER NOT NULL DEFAULT 0, name TEXT NOT NULL, UNIQUE(book, format));
        CREATE TABLE authors (id INTEGER PRIMARY KEY, name TEXT NOT NULL, sort TEXT, UNIQUE(name));
        CREATE TABLE books_authors_link (id INTEGER PRIMARY KEY, book INTEGER NOT NULL, author INTEGER NOT NULL, UNIQUE(book, author));
        CREATE TABLE series (id INTEGER PRIMARY KEY, name TEXT NOT NULL, UNIQUE(name));
        CREATE TABLE books_series_link (id INTEGER PRIMARY KEY, book INTEGER NOT NULL, series INTEGER NOT NULL, UNIQUE(book));
        CREATE TABLE tags (id INTEGER PRIMARY KEY, name TEXT NOT NULL, UNIQUE(name));
        CREATE TABLE books_tags_link (id INTEGER PRIMARY KEY, book INTEGER NOT NULL, tag INTEGER NOT NULL, UNIQUE(book, tag));
        CREATE TABLE ratings (id INTEGER PRIMARY KEY, rating INTEGER CHECK(rating > -1 AND rating < 11), UNIQUE(rating));
        CREATE TABLE books_ratings_link (id INTEGER PRIMARY KEY, book INTEGER NOT NULL, rating INTEGER NOT NULL, UNIQUE(book, rating));
        CREATE TABLE comments (id INTEGER PRIMARY KEY, book INTEGER NOT NULL, text TEXT NOT NULL, UNIQUE(book));
        CREATE TABLE identifiers (id INTEGER PRIMARY KEY, book INTEGER NOT NULL, type TEXT NOT NULL DEFAULT 'isbn',
            val TEXT NOT NULL, UNIQUE(book, type));
        CREATE TABLE languages (id INTEGER PRIMARY KEY, lang_code TEXT NOT NULL, UNIQUE(lang_code));
        CREATE TABLE books_languages_link (id INTEGER PRIMARY KEY, book INTEGER NOT NULL, lang_code INTEGER NOT NULL,
            item_order INTEGER NOT NULL DEFAULT 0, UNIQUE(book, lang_code));
    ";

    /// A new `metadata.db` in `root` with [`SCHEMA`], open for writing.
    pub(crate) fn create(root: &Path) -> Connection {
        std::fs::create_dir_all(root).unwrap();
        let conn = Connection::open(root.join(DB_FILE)).unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn
    }

    /// A book to add with [`add_book`].
    #[derive(Default, Clone)]
    pub(crate) struct NewBook<'a> {
        pub title: &'a str,
        /// In the order they should be listed.
        pub authors: &'a [&'a str],
        pub series: Option<(&'a str, f64)>,
        /// `(format, file name without extension)`, e.g. `("EPUB", "Dune - Frank Herbert")`.
        pub files: &'a [(&'a str, &'a str)],
        pub tags: &'a [&'a str],
        pub rating: Option<i64>,
        pub comments: Option<&'a str>,
        pub identifiers: &'a [(&'a str, &'a str)],
        pub languages: &'a [&'a str],
        pub has_cover: bool,
    }

    /// Insert `book` the way Calibre would; returns its id. The book's
    /// folder is `<first author>/<title> (<id>)`; no file is written.
    pub(crate) fn add_book(conn: &Connection, book: &NewBook) -> i64 {
        conn.execute(
            "INSERT INTO books (title, sort, has_cover) VALUES (?1, ?1, ?2)",
            params![book.title, book.has_cover],
        )
        .unwrap();
        let id = conn.last_insert_rowid();
        let folder = format!(
            "{}/{} ({id})",
            book.authors.first().copied().unwrap_or("Unknown"),
            book.title
        );
        conn.execute(
            "UPDATE books SET path = ?1 WHERE id = ?2",
            params![folder, id],
        )
        .unwrap();
        // `INSERT OR IGNORE` + lookup: authors, tags… are shared between books.
        let item = |table: &str, column: &str, value: &dyn rusqlite::ToSql| -> i64 {
            conn.execute(
                &format!("INSERT OR IGNORE INTO {table} ({column}) VALUES (?1)"),
                [value],
            )
            .unwrap();
            conn.query_row(
                &format!("SELECT id FROM {table} WHERE {column} = ?1"),
                [value],
                |r| r.get(0),
            )
            .unwrap()
        };
        for author in book.authors {
            let a = item("authors", "name", &author.replace(',', "|"));
            conn.execute(
                "INSERT INTO books_authors_link (book, author) VALUES (?1, ?2)",
                params![id, a],
            )
            .unwrap();
        }
        if let Some((name, index)) = book.series {
            let s = item("series", "name", &name);
            conn.execute(
                "INSERT INTO books_series_link (book, series) VALUES (?1, ?2)",
                params![id, s],
            )
            .unwrap();
            conn.execute(
                "UPDATE books SET series_index = ?1 WHERE id = ?2",
                params![index, id],
            )
            .unwrap();
        }
        for (format, name) in book.files {
            conn.execute(
                "INSERT INTO data (book, format, name) VALUES (?1, ?2, ?3)",
                params![id, format, name],
            )
            .unwrap();
        }
        for tag in book.tags {
            let t = item("tags", "name", tag);
            conn.execute(
                "INSERT INTO books_tags_link (book, tag) VALUES (?1, ?2)",
                params![id, t],
            )
            .unwrap();
        }
        if let Some(rating) = book.rating {
            let r = item("ratings", "rating", &rating);
            conn.execute(
                "INSERT INTO books_ratings_link (book, rating) VALUES (?1, ?2)",
                params![id, r],
            )
            .unwrap();
        }
        if let Some(text) = book.comments {
            conn.execute(
                "INSERT INTO comments (book, text) VALUES (?1, ?2)",
                params![id, text],
            )
            .unwrap();
        }
        for (kind, value) in book.identifiers {
            conn.execute(
                "INSERT INTO identifiers (book, type, val) VALUES (?1, ?2, ?3)",
                params![id, kind, value],
            )
            .unwrap();
        }
        for (order, code) in book.languages.iter().enumerate() {
            let l = item("languages", "lang_code", code);
            conn.execute(
                "INSERT INTO books_languages_link (book, lang_code, item_order) VALUES (?1, ?2, ?3)",
                params![id, l, order as i64],
            )
            .unwrap();
        }
        id
    }

    /// The folder [`add_book`] gave book `id`.
    pub(crate) fn folder_of(conn: &Connection, id: i64) -> String {
        conn.query_row("SELECT path FROM books WHERE id = ?1", [id], |r| r.get(0))
            .unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::{add_book, create, NewBook};
    use super::*;
    use crate::logging::{no_log, LogLines};

    #[test]
    fn a_folder_without_a_calibre_database_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            CalibreDb::open(dir.path()).err().unwrap().code(),
            "NOT_FOUND"
        );

        let conn = Connection::open(dir.path().join(DB_FILE)).unwrap();
        conn.execute_batch("CREATE TABLE notes (id INTEGER PRIMARY KEY)")
            .unwrap();
        drop(conn);
        assert_eq!(
            CalibreDb::open(dir.path()).err().unwrap().code(),
            "INVALID_INPUT",
            "another app's database"
        );

        std::fs::write(dir.path().join(DB_FILE), vec![b'x'; 4096]).unwrap();
        assert_eq!(
            CalibreDb::open(dir.path()).err().unwrap().code(),
            "INVALID_INPUT",
            "not a database at all"
        );
    }

    #[test]
    fn one_book_is_read_completely_in_calibre_order() {
        let dir = tempfile::tempdir().unwrap();
        let conn = create(dir.path());
        // Authors added in a non-alphabetical order, and one with a comma.
        let id = add_book(
            &conn,
            &NewBook {
                title: "The Road to Dune",
                authors: &["Kevin J. Anderson", "Brian Herbert", "Herbert, Frank"],
                series: Some(("Dune", 0.5)),
                files: &[("EPUB", "The Road to Dune"), ("TXT", "The Road to Dune")],
                tags: &["Science Fiction", "Essays"],
                rating: Some(7),
                comments: Some("<p>Unpublished chapters.</p>"),
                identifiers: &[
                    ("ISBN", "9780765353658"),
                    ("goodreads", "12345"),
                    ("amazon", "B000"),
                ],
                languages: &["eng", "fra"],
                has_cover: true,
            },
        );

        let db = CalibreDb::open(dir.path()).unwrap();
        let book = db.record(id, &no_log).unwrap().unwrap();
        assert_eq!(book.title, "The Road to Dune");
        assert_eq!(
            book.authors,
            ["Kevin J. Anderson", "Brian Herbert", "Herbert, Frank"]
        );
        assert_eq!(
            (book.series.as_deref(), book.series_index),
            (Some("Dune"), Some(0.5))
        );
        assert_eq!(
            book.files
                .iter()
                .map(CalibreFile::file_name)
                .collect::<Vec<_>>(),
            ["The Road to Dune.epub", "The Road to Dune.txt"]
        );
        assert_eq!(book.tags, ["Science Fiction", "Essays"]);
        assert_eq!(book.rating, Some(7.0));
        assert_eq!(
            book.comments.as_deref(),
            Some("<p>Unpublished chapters.</p>")
        );
        assert_eq!(
            (book.isbn.as_deref(), book.goodreads_id.as_deref()),
            (Some("9780765353658"), Some("12345"))
        );
        assert_eq!(book.languages, ["eng", "fra"]);
        assert!(book.has_cover);
        assert_eq!(
            book.folder,
            format!("Kevin J. Anderson/The Road to Dune ({id})")
        );

        assert_eq!(
            db.record(id + 100, &no_log).unwrap(),
            None,
            "an unknown id is no book, not an error"
        );
    }

    #[test]
    fn the_list_matches_the_records_in_four_queries() {
        let dir = tempfile::tempdir().unwrap();
        let conn = create(dir.path());
        let a = add_book(
            &conn,
            &NewBook {
                title: "Good Omens",
                authors: &["Terry Pratchett", "Neil Gaiman"],
                files: &[("PDF", "Good Omens"), ("EPUB", "Good Omens")],
                ..NewBook::default()
            },
        );
        let b = add_book(
            &conn,
            &NewBook {
                title: "Notes",
                authors: &["Someone"],
                series: Some(("Diaries", 3.0)),
                ..NewBook::default()
            },
        );
        // A second link for the same book, added later, is listed after.
        conn.execute("INSERT INTO authors (name) VALUES ('Alpha')", [])
            .unwrap();
        conn.execute("INSERT INTO books_authors_link (book, author) SELECT ?1, id FROM authors WHERE name = 'Alpha'", [b]).unwrap();

        let db = CalibreDb::open(dir.path()).unwrap();
        let list = db.list(&no_log).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].authors, ["Terry Pratchett", "Neil Gaiman"]);
        assert_eq!(
            list[0]
                .files
                .iter()
                .map(|f| f.format.as_str())
                .collect::<Vec<_>>(),
            ["pdf", "epub"]
        );
        assert_eq!(list[1].authors, ["Someone", "Alpha"]);
        assert_eq!(list[1].series.as_deref(), Some("Diaries"));
        for listed in &list {
            let full = db.record(listed.id, &no_log).unwrap().unwrap();
            assert_eq!(
                (&full.title, &full.authors, &full.series, &full.files),
                (
                    &listed.title,
                    &listed.authors,
                    &listed.series,
                    &listed.files
                )
            );
        }
        assert_eq!((list[0].id, list[1].id), (a, b));
    }

    #[test]
    fn missing_optional_tables_leave_fields_empty() {
        let dir = tempfile::tempdir().unwrap();
        let conn = Connection::open(dir.path().join(DB_FILE)).unwrap();
        // A very old library: no series, tags, ratings, identifiers, languages… at all.
        conn.execute_batch(
            "CREATE TABLE books (id INTEGER PRIMARY KEY, title TEXT, path TEXT, series_index REAL, has_cover BOOL);
             CREATE TABLE data (id INTEGER PRIMARY KEY, book INTEGER, format TEXT, name TEXT);
             INSERT INTO books VALUES (1, 'Old', 'Someone/Old (1)', 1.0, NULL);
             INSERT INTO data VALUES (1, 1, 'EPUB', 'Old');",
        )
        .unwrap();
        let db = CalibreDb::open(dir.path()).unwrap();
        let lines = LogLines::default();
        let book = db.record(1, &lines.sink()).unwrap().unwrap();
        assert_eq!(book.title, "Old");
        assert!(
            book.authors.is_empty()
                && book.series.is_none()
                && book.tags.is_empty()
                && book.languages.is_empty()
        );
        assert!(!book.has_cover);
        assert!(
            !lines.contains("WARNING"),
            "a missing table is normal, not a warning"
        );
        assert_eq!(db.list(&no_log).unwrap().len(), 1);
    }

    #[test]
    fn a_table_with_an_unexpected_shape_warns_once_and_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let conn = Connection::open(dir.path().join(DB_FILE)).unwrap();
        conn.execute_batch(
            "CREATE TABLE books (id INTEGER PRIMARY KEY, title TEXT, path TEXT, series_index REAL, has_cover BOOL);
             CREATE TABLE data (id INTEGER PRIMARY KEY, book INTEGER, format TEXT, name TEXT);
             CREATE TABLE tags (id INTEGER PRIMARY KEY, label TEXT);
             CREATE TABLE books_tags_link (id INTEGER PRIMARY KEY, book INTEGER, tag INTEGER);
             INSERT INTO books VALUES (1, 'A', 'x/A (1)', 1.0, 0), (2, 'B', 'x/B (2)', 1.0, 0);",
        )
        .unwrap();
        let db = CalibreDb::open(dir.path()).unwrap();
        let lines = RefCell::new(Vec::<String>::new());
        let sink = |line: String| lines.borrow_mut().push(line);
        assert!(db.record(1, &sink).unwrap().unwrap().tags.is_empty());
        assert!(db.record(2, &sink).unwrap().unwrap().tags.is_empty());
        let lines = lines.into_inner();
        assert_eq!(
            lines.len(),
            1,
            "one warning for the whole import, not one per book: {lines:?}"
        );
        assert!(lines[0].contains("WARNING — could not read the tags"));
    }
}
