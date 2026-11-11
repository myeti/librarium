//! Reading a Kobo's library database, `.kobo/KoboReader.sqlite`.
//!
//! The database is never opened in place. It may be in WAL mode with
//! recent changes still in `KoboReader.sqlite-wal`, may carry a hot
//! `-journal` after a crash, or be locked; and opening it on the device —
//! even read-only — can make SQLite create `-shm`/`-journal` files there.
//! [`open_snapshot`] therefore copies the database and its `-wal` /
//! `-journal` into a temp folder and opens the copy, where SQLite may
//! replay or roll back whatever it needs to. The device is only read.
//!
//! [`read_books`] lists the sideloaded books (`ContentType = 6`) with their
//! title, author, progress and read status. Kobo store purchases (content
//! ids that aren't `file:///mnt/onboard/…` paths) live in `.kobo/kepub/`
//! and aren't files the user manages, so they are left out.
//!
//! TODO(device-validation): check against a real KoboReader.sqlite (current
//! firmware) that the WAL copy gives the same rows as the Kobo shows, and
//! whether sideloaded ContentIDs are percent-encoded (both spellings are
//! tried here).

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};

/// `<mount>/.kobo/KoboReader.sqlite`.
pub(crate) fn database_path(mount: &Path) -> PathBuf {
    mount.join(".kobo").join("KoboReader.sqlite")
}

/// SQLite's companion files that hold data the main file may not have yet.
const SIDECARS: [&str; 2] = ["-wal", "-journal"];

/// A private copy of the database, open. The copy is deleted with it.
pub(crate) struct Snapshot {
    // Declared first so the connection closes before the folder goes.
    pub conn: Connection,
    _dir: tempfile::TempDir,
}

/// Copy the Kobo database of the device at `mount` (with its `-wal` and
/// `-journal`, if any) into a temp folder and open the copy. `Ok(None)` when
/// the device has no database.
pub(crate) fn open_snapshot(mount: &Path) -> Result<Option<Snapshot>, String> {
    let db = database_path(mount);
    if !db.is_file() {
        return Ok(None);
    }
    let dir = tempfile::Builder::new()
        .prefix("librarium-kobo-")
        .tempdir()
        .map_err(|e| format!("could not create a temp folder for the Kobo database: {e}"))?;
    let copy = dir.path().join("KoboReader.sqlite");
    fs::copy(&db, &copy).map_err(|e| format!("could not copy the Kobo database: {e}"))?;
    for suffix in SIDECARS {
        let sidecar = sibling(&db, suffix);
        if sidecar.is_file() {
            fs::copy(&sidecar, sibling(&copy, suffix))
                .map_err(|e| format!("could not copy the Kobo database's {suffix} file: {e}"))?;
        }
    }
    // Read-write on the copy, so SQLite can apply the WAL or roll back a
    // hot journal; `query_only` then makes sure this code never writes.
    let conn = Connection::open_with_flags(
        &copy,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| format!("could not open the Kobo database: {e}"))?;
    conn.pragma_update(None, "query_only", true)
        .map_err(|e| format!("could not open the Kobo database read-only: {e}"))?;
    Ok(Some(Snapshot { conn, _dir: dir }))
}

/// `path` with `suffix` appended to its file name (`x.sqlite` → `x.sqlite-wal`).
fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

/// Kobo `ReadStatus`: 0 unread, 1 reading, 2 finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ReadStatus {
    Unread,
    Reading,
    Finished,
}

impl ReadStatus {
    fn from_kobo(value: i64) -> Option<Self> {
        match value {
            0 => Some(Self::Unread),
            1 => Some(Self::Reading),
            2 => Some(Self::Finished),
            _ => None,
        }
    }
}

/// One sideloaded book of the Kobo database.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct KoboBook {
    /// The file, relative to the mount (`/` separators), as the ContentID
    /// spells it.
    pub rel_path: String,
    /// The same, percent-decoded, when that differs.
    pub decoded_rel_path: Option<String>,
    pub title: String,
    /// `Attribution`: the author(s), as the Kobo shows them.
    pub author: String,
    /// `___PercentRead`, 0–100.
    pub percent_read: Option<u8>,
    pub read_status: Option<ReadStatus>,
}

/// Where sideloaded books live, as Kobo writes their ContentID.
const ONBOARD_PREFIX: &str = "file:///mnt/onboard/";

/// The sideloaded books (`ContentType = 6`) in the database. Columns that a
/// firmware lacks read as empty (so an older database still lists its
/// books); a database without a `content` table is an error.
pub(crate) fn read_books(conn: &Connection) -> Result<Vec<KoboBook>, String> {
    let columns: Vec<String> = conn
        .prepare("SELECT name FROM pragma_table_info('content')")
        .and_then(|mut s| s.query_map([], |r| r.get::<_, String>(0))?.collect())
        .map_err(|e| format!("the Kobo database can't be read: {e}"))?;
    if !columns.iter().any(|c| c == "ContentID") || !columns.iter().any(|c| c == "ContentType") {
        return Err("the Kobo database has no book table".to_string());
    }
    let col = |name: &str| {
        if columns.iter().any(|c| c == name) {
            name.to_string()
        } else {
            "NULL".to_string()
        }
    };
    // `= 6` also matches a TEXT column holding '6' (SQLite applies the
    // column's affinity to the literal; ContentType is TEXT on device).
    let sql = format!(
        "SELECT ContentID, {}, {}, {}, {} FROM content WHERE ContentType = 6",
        col("Title"),
        col("Attribution"),
        col("___PercentRead"),
        col("ReadStatus"),
    );
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("the Kobo database can't be read: {e}"))?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
                // f64 / i64 accept both INTEGER and REAL storage.
                r.get::<_, Option<f64>>(3)?,
                r.get::<_, Option<i64>>(4)?,
            ))
        })
        .map_err(|e| format!("the Kobo database can't be read: {e}"))?;
    let mut books = Vec::new();
    for row in rows {
        // A row with unexpected types costs only itself.
        let Ok((Some(content_id), title, author, percent, status)) = row else {
            continue;
        };
        let Some(rel_path) = content_id
            .strip_prefix(ONBOARD_PREFIX)
            .filter(|p| !p.is_empty())
        else {
            continue;
        };
        let decoded = percent_decode(rel_path).filter(|d| d != rel_path);
        books.push(KoboBook {
            rel_path: rel_path.to_string(),
            decoded_rel_path: decoded,
            title: title.unwrap_or_default().trim().to_string(),
            author: author.unwrap_or_default().trim().to_string(),
            percent_read: percent
                .filter(|p| p.is_finite())
                .map(|p| p.clamp(0.0, 100.0) as u8),
            read_status: status.and_then(ReadStatus::from_kobo),
        });
    }
    Ok(books)
}

/// `%XX` sequences decoded (as UTF-8). `None` for a malformed escape or
/// bytes that aren't UTF-8.
pub(crate) fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The `content` columns Librarium reads, typed like a real
    /// KoboReader.sqlite (ContentType is TEXT on device).
    pub(crate) const CONTENT_TABLE: &str = "CREATE TABLE content (
        ContentID TEXT NOT NULL, ContentType TEXT NOT NULL, MimeType TEXT,
        Title TEXT, Attribution TEXT, ___PercentRead INTEGER, ReadStatus INTEGER,
        BookTitle TEXT
    );";

    /// A fake Kobo at `mount` (`.kobo/` and a database with `rows`:
    /// `(ContentID, ContentType, Title, Attribution, percent, status)`).
    pub(crate) fn kobo_db(mount: &Path, rows: &[(&str, &str, &str, &str, i64, i64)]) -> Connection {
        fs::create_dir_all(mount.join(".kobo")).unwrap();
        let conn = Connection::open(database_path(mount)).unwrap();
        conn.execute_batch(CONTENT_TABLE).unwrap();
        for (id, kind, title, author, pct, status) in rows {
            conn.execute(
                "INSERT INTO content (ContentID, ContentType, Title, Attribution, ___PercentRead, ReadStatus) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![id, kind, title, author, pct, status],
            )
            .unwrap();
        }
        conn
    }

    fn books_of(mount: &Path) -> Vec<KoboBook> {
        let snapshot = open_snapshot(mount).unwrap().unwrap();
        read_books(&snapshot.conn).unwrap()
    }

    #[test]
    fn lists_sideloaded_books_only() {
        let mount = tempfile::tempdir().unwrap();
        drop(kobo_db(
            mount.path(),
            &[
                (
                    "file:///mnt/onboard/Books/Dune.kepub.epub",
                    "6",
                    "Dune",
                    "Frank Herbert",
                    42,
                    1,
                ),
                (
                    "file:///mnt/onboard/Caf%C3%A9.epub",
                    "6",
                    "Café",
                    "Anon",
                    100,
                    2,
                ),
                (
                    "file:///mnt/onboard/Books/Dune.kepub.epub!OEBPS!ch1.xhtml",
                    "9",
                    "Ch 1",
                    "",
                    77,
                    0,
                ),
                ("0b6c1a9e-store-book", "6", "Store book", "Someone", 10, 1),
                (
                    "file:///mnt/sd/OnCard.epub",
                    "6",
                    "On the SD card",
                    "",
                    0,
                    0,
                ),
            ],
        ));

        let books = books_of(mount.path());
        assert_eq!(
            books,
            vec![
                KoboBook {
                    rel_path: "Books/Dune.kepub.epub".into(),
                    decoded_rel_path: None,
                    title: "Dune".into(),
                    author: "Frank Herbert".into(),
                    percent_read: Some(42),
                    read_status: Some(ReadStatus::Reading),
                },
                KoboBook {
                    rel_path: "Caf%C3%A9.epub".into(),
                    decoded_rel_path: Some("Café.epub".into()),
                    title: "Café".into(),
                    author: "Anon".into(),
                    percent_read: Some(100),
                    read_status: Some(ReadStatus::Finished),
                },
            ]
        );
    }

    #[test]
    fn reads_changes_still_in_the_wal_without_touching_the_device() {
        let mount = tempfile::tempdir().unwrap();
        let writer = kobo_db(mount.path(), &[]);
        // WAL mode with checkpoints off: new rows stay in KoboReader.sqlite-wal,
        // and the writer keeps its connection (and lock) open meanwhile.
        writer.pragma_update(None, "journal_mode", "WAL").unwrap();
        writer.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
        writer
            .execute(
                "INSERT INTO content (ContentID, ContentType, Title, Attribution, ___PercentRead, ReadStatus) VALUES ('file:///mnt/onboard/New.epub', '6', 'New', 'Me', 5, 1)",
                [],
            )
            .unwrap();
        let wal = sibling(&database_path(mount.path()), "-wal");
        assert!(
            fs::metadata(&wal).unwrap().len() > 0,
            "the row is in the WAL"
        );
        writer.execute_batch("BEGIN EXCLUSIVE").unwrap();

        let books = books_of(mount.path());
        assert_eq!(
            books.iter().map(|b| b.title.as_str()).collect::<Vec<_>>(),
            vec!["New"]
        );
        writer.execute_batch("COMMIT").unwrap();
    }

    #[test]
    fn reading_never_writes_to_the_device() {
        let mount = tempfile::tempdir().unwrap();
        drop(kobo_db(
            mount.path(),
            &[("file:///mnt/onboard/A.epub", "6", "A", "", 0, 0)],
        ));
        let db = database_path(mount.path());
        let before = fs::read(&db).unwrap();
        assert_eq!(books_of(mount.path()).len(), 1);
        assert_eq!(fs::read(&db).unwrap(), before);
        let files: Vec<_> = fs::read_dir(mount.path().join(".kobo"))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(
            files,
            vec![std::ffi::OsString::from("KoboReader.sqlite")],
            "no -shm/-journal created on the device"
        );
    }

    #[test]
    fn older_schemas_and_missing_databases() {
        let mount = tempfile::tempdir().unwrap();
        assert!(open_snapshot(mount.path()).unwrap().is_none());

        fs::create_dir_all(mount.path().join(".kobo")).unwrap();
        let conn = Connection::open(database_path(mount.path())).unwrap();
        conn.execute_batch(
            "CREATE TABLE content (ContentID TEXT, ContentType TEXT, Title TEXT);
             INSERT INTO content VALUES ('file:///mnt/onboard/Old.epub', '6', 'Old');",
        )
        .unwrap();
        drop(conn);
        let books = books_of(mount.path());
        assert_eq!(
            (
                books[0].title.as_str(),
                books[0].author.as_str(),
                books[0].percent_read
            ),
            ("Old", "", None)
        );

        let other = tempfile::tempdir().unwrap();
        fs::create_dir_all(other.path().join(".kobo")).unwrap();
        Connection::open(database_path(other.path()))
            .unwrap()
            .execute_batch("CREATE TABLE x (a)")
            .unwrap();
        let snapshot = open_snapshot(other.path()).unwrap().unwrap();
        assert!(read_books(&snapshot.conn).is_err());
    }

    #[test]
    fn percent_decoding() {
        assert_eq!(
            percent_decode("Caf%C3%A9%20au%20lait.epub").as_deref(),
            Some("Café au lait.epub")
        );
        assert_eq!(percent_decode("plain.epub").as_deref(), Some("plain.epub"));
        assert_eq!(percent_decode("bad%zz"), None);
        assert_eq!(percent_decode("cut%4"), None);
        assert_eq!(percent_decode("%FF"), None, "not UTF-8");
    }
}
