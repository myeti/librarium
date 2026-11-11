//! The reader's cache of Kindle books (MOBI, AZW, AZW3) converted to EPUB.
//!
//! The in-app reader displays EPUB and PDF, so a Kindle book is converted
//! to EPUB first ([`crate::convert`]). That takes seconds for a novel and
//! more for a big illustrated book, so the result is kept in
//! `<app cache>/reader/`, and reopening the book is instant.
//!
//! - **Key.** An entry is named `<bookId>-<size>-<mtime>.epub`, after the
//!   source file's size in bytes and modification time (ms since the Unix
//!   epoch). A book whose file changed (edited, replaced, re-downloaded)
//!   therefore misses and is converted again; its older entries can never
//!   be used again and are removed at that point.
//! - **Size cap.** After each conversion, the least recently used entries
//!   are removed until the cache holds at most `max_bytes` (the app uses
//!   [`MAX_BYTES`], 500 MB). "Used" is the entry's modification time,
//!   refreshed on every hit. The entry just made is never removed, even if
//!   it alone is larger than the cap (the reader is about to open it).
//! - **Crash safety.** A conversion writes a hidden temp file in the cache
//!   folder (`.librarium-convert-*`), renamed to its entry name only once
//!   validated: an entry is always a complete, valid EPUB. Temp files left
//!   by a crash are removed by the next conversion once they are an hour
//!   old.
//! - **One lock** (in-process) for the whole cache. Conversions are rare and
//!   one at a time is fine; it also keeps an eviction from racing an
//!   insertion.
//!
//! It is only a cache: deleting the folder is always safe.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::convert::TEMP_PREFIX;
use crate::error::{display_name, AppError, AppResult};
use crate::fsutil;
use crate::logging::Log;

/// The cache's size cap in the app.
pub(crate) const MAX_BYTES: u64 = 500 * 1024 * 1024;

/// Extension of the cache entries.
const ENTRY_EXTENSION: &str = "epub";

/// Temp files of conversions older than this are leftovers of a crash.
const STALE_TEMP_AGE: Duration = Duration::from_secs(60 * 60);

/// Converts the book at its first argument to EPUB, into a temp file in the
/// folder given as second argument (the app passes
/// [`crate::convert::convert_to_temp`]; tests pass fakes).
pub(crate) type Convert<'a> = &'a dyn Fn(&Path, &Path) -> AppResult<tempfile::NamedTempFile>;

/// A cache of converted books in one folder, capped in size.
pub(crate) struct ReaderCache {
    dir: PathBuf,
    max_bytes: u64,
}

/// The EPUB for a book, found in or just added to the cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CachedEpub {
    pub path: PathBuf,
    /// Whether it was already there (no conversion).
    pub hit: bool,
}

/// Whether `id` can be part of a file name: book ids are UUIDs; anything
/// else (a `../x` from a confused caller) is refused.
fn is_safe_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Modification time in ms since the Unix epoch (0 if the platform or the
/// filesystem doesn't say).
fn mtime_ms(meta: &fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// The entry name for book `id` whose file has metadata `source`.
fn entry_name(id: &str, source: &fs::Metadata) -> String {
    format!(
        "{id}-{}-{}.{ENTRY_EXTENSION}",
        source.len(),
        mtime_ms(source)
    )
}

/// The book id of an entry name made by [`entry_name`]; `None` for any other
/// file.
fn entry_id(name: &str) -> Option<&str> {
    let stem = name.strip_suffix(ENTRY_EXTENSION)?.strip_suffix('.')?;
    let mut parts = stem.rsplitn(3, '-');
    let (mtime, size, id) = (parts.next()?, parts.next()?, parts.next()?);
    let numeric = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    (numeric(mtime) && numeric(size) && is_safe_id(id)).then_some(id)
}

/// A file in the cache folder: an entry or a conversion's temp file.
struct CacheFile {
    path: PathBuf,
    name: String,
    size: u64,
    modified: SystemTime,
}

impl ReaderCache {
    pub(crate) fn new(dir: PathBuf, max_bytes: u64) -> Self {
        Self { dir, max_bytes }
    }

    /// The EPUB of book `id`, whose file is `src`: the cached one if `src`
    /// hasn't changed since it was converted (a hit, which also marks the
    /// entry as just used), else a new conversion (`convert`), stored for
    /// next time — then the cache is trimmed to its cap.
    pub(crate) fn epub_for(
        &self,
        id: &str,
        src: &Path,
        convert: Convert,
        log: Log,
    ) -> AppResult<CachedEpub> {
        if !is_safe_id(id) {
            return Err(AppError::InvalidInput(format!("Invalid book id: {id:?}")));
        }
        let source = fs::metadata(src)
            .map_err(|e| AppError::io(format!("Could not read {}", display_name(src)), &e))?;
        let path = self.dir.join(entry_name(id, &source));

        fsutil::with_file_lock(&self.dir.join(".lock"), || {
            if path.is_file() {
                touch(&path);
                return Ok(CachedEpub {
                    path: path.clone(),
                    hit: true,
                });
            }
            fs::create_dir_all(&self.dir)
                .map_err(|e| AppError::io("Could not create the reader's cache folder", &e))?;
            let converted = convert(src, &self.dir)?;
            converted.persist(&path).map_err(|e| {
                AppError::io(
                    "Could not store the converted book in the reader's cache",
                    &e.error,
                )
            })?;
            log(format!(
                "[reader cache] converted {} → {}",
                display_name(src),
                display_name(&path)
            ));
            self.trim(id, &path, log);
            Ok(CachedEpub {
                path: path.clone(),
                hit: false,
            })
        })
    }

    /// After `keep` was added for book `id`: remove `id`'s other entries
    /// (older versions of its file), stale temp files, then the least
    /// recently used entries until the cache is within its cap. Best effort:
    /// a file that can't be removed is logged and left.
    fn trim(&self, id: &str, keep: &Path, log: Log) {
        let now = SystemTime::now();
        let mut entries = Vec::new();
        for file in self.files() {
            let stale = if file.name.starts_with(TEMP_PREFIX) {
                now.duration_since(file.modified)
                    .is_ok_and(|age| age >= STALE_TEMP_AGE)
            } else if let Some(entry) = entry_id(&file.name) {
                let outdated = entry == id && file.path != keep;
                if !outdated {
                    entries.push(file);
                    continue;
                }
                true
            } else {
                false
            };
            if stale {
                remove(&file.path, log);
            }
        }

        let mut total: u64 = entries.iter().map(|e| e.size).sum();
        // Least recently used first; the name breaks ties, so the order is stable.
        entries.sort_by(|a, b| {
            a.modified
                .cmp(&b.modified)
                .then_with(|| a.name.cmp(&b.name))
        });
        for entry in &entries {
            if total <= self.max_bytes {
                break;
            }
            if entry.path != keep && remove(&entry.path, log) {
                total -= entry.size;
            }
        }
    }

    /// The files directly in the cache folder (an unreadable folder or
    /// entry is skipped).
    fn files(&self) -> Vec<CacheFile> {
        let Ok(dir) = fs::read_dir(&self.dir) else {
            return vec![];
        };
        dir.filter_map(Result::ok)
            .filter_map(|e| {
                let meta = e.metadata().ok().filter(fs::Metadata::is_file)?;
                Some(CacheFile {
                    path: e.path(),
                    name: e.file_name().to_string_lossy().into_owned(),
                    size: meta.len(),
                    modified: meta.modified().unwrap_or(UNIX_EPOCH),
                })
            })
            .collect()
    }
}

/// Mark the entry at `path` as just used (its modification time). Best
/// effort: if this fails, the entry is just evicted a bit early.
fn touch(path: &Path) {
    let _ = fs::File::options()
        .write(true)
        .open(path)
        .and_then(|f| f.set_modified(SystemTime::now()));
}

/// Remove a cache file; whether it is gone.
fn remove(path: &Path, log: Log) -> bool {
    match fs::remove_file(path) {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
        Err(e) => {
            log(format!(
                "[reader cache] WARNING — could not remove {}: {e}",
                display_name(path)
            ));
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging::no_log;
    use std::cell::Cell;
    use std::io::Write;

    /// A cache in a temp folder, a source "book", and a fake converter that
    /// writes `output_size` bytes and counts its calls.
    struct Setup {
        _dir: tempfile::TempDir,
        cache_dir: PathBuf,
        books: PathBuf,
        conversions: Cell<usize>,
        output_size: usize,
    }

    impl Setup {
        fn new(output_size: usize) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let books = dir.path().join("books");
            fs::create_dir_all(&books).unwrap();
            Self {
                cache_dir: dir.path().join("cache/reader"),
                books,
                _dir: dir,
                conversions: Cell::new(0),
                output_size,
            }
        }

        fn book(&self, name: &str, content: &[u8]) -> PathBuf {
            let path = self.books.join(name);
            fs::write(&path, content).unwrap();
            path
        }

        fn open(&self, cache: &ReaderCache, id: &str, src: &Path) -> CachedEpub {
            let convert = |_: &Path, dir: &Path| -> AppResult<tempfile::NamedTempFile> {
                self.conversions.set(self.conversions.get() + 1);
                let mut tmp = tempfile::Builder::new()
                    .prefix(TEMP_PREFIX)
                    .suffix(".epub")
                    .tempfile_in(dir)
                    .unwrap();
                tmp.write_all(&vec![b'x'; self.output_size]).unwrap();
                Ok(tmp)
            };
            cache.epub_for(id, src, &convert, &no_log).unwrap()
        }

        fn entries(&self) -> Vec<String> {
            let mut names: Vec<String> = fs::read_dir(&self.cache_dir)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        }
    }

    /// Set a file's modification time to `secs_ago` seconds ago.
    fn age(path: &Path, secs_ago: u64) {
        let file = fs::File::options().write(true).open(path).unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(secs_ago))
            .unwrap();
    }

    #[test]
    fn a_miss_converts_and_a_hit_reuses_the_conversion() {
        let s = Setup::new(100);
        let cache = ReaderCache::new(s.cache_dir.clone(), 10_000);
        let src = s.book("Dune.azw3", b"kindle book");

        let first = s.open(&cache, "book-1", &src);
        assert!(!first.hit);
        assert!(first.path.is_file());
        let second = s.open(&cache, "book-1", &src);
        assert_eq!(
            second,
            CachedEpub {
                path: first.path.clone(),
                hit: true
            }
        );
        assert_eq!(s.conversions.get(), 1, "reopening is instant");

        let name = display_name(&first.path);
        assert!(
            name.starts_with("book-1-11-") && name.ends_with(".epub"),
            "{name}"
        );
        assert_eq!(s.entries(), [name], "no temp file left behind");
    }

    #[test]
    fn a_changed_source_is_converted_again_and_the_old_entry_removed() {
        let s = Setup::new(100);
        let cache = ReaderCache::new(s.cache_dir.clone(), 10_000);
        let src = s.book("Dune.azw3", b"first version");
        let other = s.book("Emma.azw3", b"another book");
        let old = s.open(&cache, "book-1", &src);
        let emma = s.open(&cache, "book-2", &other);

        // Same size, new modification time (an edit in place).
        fs::write(&src, b"second versio").unwrap();
        let later = SystemTime::now() + Duration::from_secs(5);
        fs::File::options()
            .write(true)
            .open(&src)
            .unwrap()
            .set_modified(later)
            .unwrap();
        let new = s.open(&cache, "book-1", &src);
        assert!(!new.hit);
        assert_ne!(new.path, old.path);
        assert!(
            !old.path.exists(),
            "the outdated entry can never be used again"
        );
        assert!(emma.path.exists(), "other books' entries are kept");

        // A different size misses too.
        fs::write(&src, b"a longer third version").unwrap();
        assert!(!s.open(&cache, "book-1", &src).hit);
        assert_eq!(s.conversions.get(), 4);
    }

    #[test]
    fn the_least_recently_used_entries_go_first() {
        let s = Setup::new(100);
        // Room for two entries.
        let cache = ReaderCache::new(s.cache_dir.clone(), 250);
        let a = s.open(&cache, "a", &s.book("a.mobi", b"a"));
        let b = s.open(&cache, "b", &s.book("b.mobi", b"b"));
        age(&a.path, 300);
        age(&b.path, 200);
        // Reading A again makes B the least recently used.
        assert!(s.open(&cache, "a", &s.books.join("a.mobi")).hit);

        let c = s.open(&cache, "c", &s.book("c.mobi", b"c"));
        assert!(a.path.exists() && c.path.exists());
        assert!(!b.path.exists(), "B was the least recently used");
    }

    #[test]
    fn the_new_entry_is_kept_even_over_the_cap() {
        let s = Setup::new(100);
        let cache = ReaderCache::new(s.cache_dir.clone(), 50);
        let old = s.open(&cache, "old", &s.book("old.azw", b"o"));
        let big = s.open(&cache, "big", &s.book("big.azw3", b"b"));
        assert!(big.path.is_file(), "the reader is about to open it");
        assert!(!old.path.exists());
    }

    #[test]
    fn leftover_temp_files_are_removed_once_stale() {
        let s = Setup::new(10);
        let cache = ReaderCache::new(s.cache_dir.clone(), 10_000);
        fs::create_dir_all(&s.cache_dir).unwrap();
        let stale = s.cache_dir.join(format!("{TEMP_PREFIX}crash.epub"));
        let recent = s.cache_dir.join(format!("{TEMP_PREFIX}recent.epub"));
        let unrelated = s.cache_dir.join("notes.txt");
        for f in [&stale, &recent, &unrelated] {
            fs::write(f, b"x").unwrap();
        }
        age(&stale, 2 * 60 * 60);

        s.open(&cache, "book", &s.book("b.mobi", b"b"));
        assert!(!stale.exists());
        assert!(recent.exists() && unrelated.exists());
    }

    #[test]
    fn a_failed_conversion_stores_nothing() {
        let s = Setup::new(10);
        let cache = ReaderCache::new(s.cache_dir.clone(), 10_000);
        let src = s.book("broken.azw3", b"x");
        let fail = |_: &Path, _: &Path| -> AppResult<tempfile::NamedTempFile> {
            Err(AppError::Conversion("DRM".into()))
        };
        assert_eq!(
            cache
                .epub_for("book", &src, &fail, &no_log)
                .unwrap_err()
                .code(),
            "CONVERSION"
        );
        assert!(s.entries().is_empty());
        // Nothing poisoned: the next attempt converts.
        assert!(!s.open(&cache, "book", &src).hit);
    }

    #[test]
    fn ids_and_entry_names() {
        let s = Setup::new(10);
        let cache = ReaderCache::new(s.cache_dir.clone(), 10_000);
        let src = s.book("b.mobi", b"b");
        let never = |_: &Path, _: &Path| -> AppResult<tempfile::NamedTempFile> { unreachable!() };
        assert_eq!(
            cache
                .epub_for("../escape", &src, &never, &no_log)
                .unwrap_err()
                .code(),
            "INVALID_INPUT"
        );

        let uuid = "0f8fad5b-d9cb-469f-a165-70867728950e";
        assert_eq!(
            entry_id(&format!("{uuid}-1234-1730000000000.epub")),
            Some(uuid)
        );
        assert_eq!(entry_id("notes.txt"), None);
        assert_eq!(entry_id("x-12-abc.epub"), None);
        assert_eq!(entry_id(&format!("{TEMP_PREFIX}abc.epub")), None);
    }
}
