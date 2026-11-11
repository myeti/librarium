//! The catalog: the library as the feeds see it, and the cache that keeps it.
//!
//! A [`Catalog`] is built from a **read-only** snapshot of the library index
//! ([`read_books_snapshot`]: `library.json`, its backup, or the legacy
//! `books.json`; never locked, never written) plus one listing each of
//! `.covers/` and `.covers/thumbs/`. Everything the feeds need is computed
//! once per snapshot: display titles, individual authors, plain-text
//! summaries, the title, newest-first, author and series orders, and the
//! folded text searches run against.
//!
//! [`CatalogCache`] keeps the last catalog and rebuilds it when it is stale:
//! - **explicitly**, through [`CatalogCache::invalidate`] (the
//!   `invalidate_opds_cache` command, a server restart, a user switch);
//! - **automatically**, when one of the files it was built from changed: the
//!   index files and the two cover folders are `stat`ed (size, modification
//!   time, and file id on Unix) at most once per [`RECHECK_AFTER`], and any
//!   difference triggers a reload. A change made by another computer sharing
//!   a NAS library, or by an older Librarium writing `books.json`, shows up
//!   without anyone calling the command.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, PoisonError, RwLock as StdRwLock};
use std::time::{Duration, Instant, SystemTime};

use chrono::{DateTime, Utc};
use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

use crate::commands::library::covers::{covers_dir, thumbs_dir};
use crate::commands::library::index::{read_books_snapshot, snapshot_sources};
use crate::epub::edit::split_authors;
use crate::epub::metadata::description_text;
use crate::models::book::Book;

/// Feed page size when the client doesn't ask for one.
pub(super) const DEFAULT_PAGE_SIZE: usize = 50;
/// Largest page a client may ask for (`?limit=`).
pub(super) const MAX_PAGE_SIZE: usize = 200;
/// Longer search queries are cut (by characters) before matching.
pub(super) const MAX_QUERY_CHARS: usize = 200;
/// Words of a search query beyond this many are ignored.
const MAX_QUERY_TERMS: usize = 10;
/// How long a catalog is served before its source files are `stat`ed again.
/// Short enough to feel immediate, long enough that a reader loading 50
/// thumbnails doesn't cost 300 `stat`s on a NAS.
pub(super) const RECHECK_AFTER: Duration = Duration::from_secs(1);

// ── Entries and groups ───────────────────────────────────────────────────────

/// One book, with what the feeds need precomputed.
#[derive(Debug, Clone)]
pub(super) struct Entry {
    pub book: Book,
    /// The title to show: the book's own, else its file name without the
    /// extension (Atom requires a title; an unknown one is empty in the index).
    pub title: String,
    /// The individual authors (`"A & B"` → `["A", "B"]`); empty when unknown.
    pub authors: Vec<String>,
    /// The description as plain text (HTML stripped).
    pub summary: Option<String>,
    /// When the book was added to the library, or the catalog's date when
    /// `date_added` isn't a valid RFC 3339 date.
    pub added: DateTime<Utc>,
    /// `.covers/<id>.(jpg|png|webp)`, when there is one.
    pub cover: Option<PathBuf>,
    /// `.covers/thumbs/<id>.jpg`, when there is one.
    pub thumbnail: Option<PathBuf>,
    /// Folded title: the catalog's sort order.
    sort_key: String,
    /// Folded title, subtitle, authors, series and ISBN: what searches match.
    haystack: String,
}

impl Entry {
    fn new(
        book: Book,
        covers: &HashMap<String, PathBuf>,
        thumbs: &HashMap<String, PathBuf>,
        fallback_date: DateTime<Utc>,
    ) -> Self {
        let meta = &book.metadata;
        let title = Some(meta.title.trim())
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .or_else(|| {
                let name = if book.filename.is_empty() {
                    book.path.as_str()
                } else {
                    book.filename.as_str()
                };
                Path::new(name)
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .filter(|s| !s.is_empty())
            })
            .unwrap_or_else(|| book.id.clone());
        let authors = split_authors(&meta.author);
        let summary = meta
            .description
            .as_deref()
            .map(description_text)
            .filter(|s| !s.trim().is_empty());
        let added = DateTime::parse_from_rfc3339(book.date_added.trim())
            .map(|d| d.with_timezone(&Utc))
            .unwrap_or(fallback_date);
        let searchable = [
            title.as_str(),
            meta.subtitle.as_deref().unwrap_or(""),
            meta.author.as_str(),
            meta.series.as_deref().unwrap_or(""),
            meta.isbn.as_deref().unwrap_or(""),
        ];
        Entry {
            sort_key: fold(&title),
            haystack: fold(&searchable.join(" ")),
            cover: covers.get(&book.id).cloned(),
            thumbnail: thumbs.get(&book.id).cloned(),
            title,
            authors,
            summary,
            added,
            book,
        }
    }

    /// The series name, when the book belongs to one.
    pub fn series(&self) -> Option<&str> {
        self.book
            .metadata
            .series
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
    }

    /// The image served as the full-size cover: the cover, else the thumbnail.
    pub fn cover_file(&self) -> Option<&Path> {
        self.cover.as_deref().or(self.thumbnail.as_deref())
    }

    /// The image served as the thumbnail: the thumbnail, else the cover.
    pub fn thumbnail_file(&self) -> Option<&Path> {
        self.thumbnail.as_deref().or(self.cover.as_deref())
    }
}

/// The books of one author or one series.
#[derive(Debug, Clone)]
pub(super) struct Group {
    /// As first spelled in the library (in title order).
    pub name: String,
    /// Folded name: the groups' sort order, and what lookups compare.
    key: String,
    /// Indices into the catalog's entries, in reading order.
    books: Vec<usize>,
}

impl Group {
    pub fn len(&self) -> usize {
        self.books.len()
    }
}

// ── Catalog ──────────────────────────────────────────────────────────────────

/// The library as the feeds see it (see the module docs). Immutable: a
/// change means a new catalog.
#[derive(Debug, Default)]
pub(super) struct Catalog {
    /// Sorted by folded title, then id.
    entries: Vec<Entry>,
    by_id: HashMap<String, usize>,
    /// Entry indices, newest first.
    recent: Vec<usize>,
    /// Sorted by folded name.
    authors: Vec<Group>,
    author_keys: HashMap<String, usize>,
    series: Vec<Group>,
    series_keys: HashMap<String, usize>,
    /// When the index last changed (else when the catalog was built): the
    /// feeds' `<updated>`.
    pub updated: DateTime<Utc>,
}

impl Catalog {
    /// Read the catalog of the library at `root`. Blocking; never fails: a
    /// missing or unreadable index gives an empty catalog.
    pub fn load(root: &Path, updated: DateTime<Utc>) -> Self {
        let covers = image_files(&covers_dir(root), &["jpg", "jpeg", "png", "webp"]);
        let thumbs = image_files(&thumbs_dir(root), &["jpg"]);
        Self::from_books(read_books_snapshot(root), &covers, &thumbs, updated)
    }

    /// Build a catalog from index records and the cover files found (book id
    /// → file).
    pub fn from_books(
        books: Vec<Book>,
        covers: &HashMap<String, PathBuf>,
        thumbs: &HashMap<String, PathBuf>,
        updated: DateTime<Utc>,
    ) -> Self {
        let mut entries: Vec<Entry> = books
            .into_iter()
            .map(|b| Entry::new(b, covers, thumbs, updated))
            .collect();
        entries.sort_by(|a, b| {
            a.sort_key
                .cmp(&b.sort_key)
                .then_with(|| a.book.id.cmp(&b.book.id))
        });

        let by_id = entries
            .iter()
            .enumerate()
            .map(|(i, e)| (e.book.id.clone(), i))
            .collect();
        let mut recent: Vec<usize> = (0..entries.len()).collect();
        recent.sort_by(|&a, &b| entries[b].added.cmp(&entries[a].added).then(a.cmp(&b)));

        let (mut authors, author_keys) =
            group_by(&entries, |e| e.authors.iter().map(String::as_str).collect());
        // An author's books: series together and in order, among the
        // standalone titles (a series sorts under its name).
        for group in &mut authors {
            group.books.sort_by(|&a, &b| {
                let key = |i: usize| {
                    let e = &entries[i];
                    (
                        e.series().map(fold).unwrap_or_else(|| e.sort_key.clone()),
                        series_position(e),
                    )
                };
                let ((name_a, pos_a), (name_b, pos_b)) = (key(a), key(b));
                name_a
                    .cmp(&name_b)
                    .then(pos_a.total_cmp(&pos_b))
                    .then(a.cmp(&b))
            });
        }
        let (mut series, series_keys) = group_by(&entries, |e| e.series().into_iter().collect());
        for group in &mut series {
            group.books.sort_by(|&a, &b| {
                series_position(&entries[a])
                    .total_cmp(&series_position(&entries[b]))
                    .then(a.cmp(&b))
            });
        }

        Catalog {
            entries,
            by_id,
            recent,
            authors,
            author_keys,
            series,
            series_keys,
            updated,
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Every book, by title.
    pub fn all(&self) -> Vec<&Entry> {
        self.entries.iter().collect()
    }

    /// Every book, newest first.
    pub fn recent(&self) -> Vec<&Entry> {
        self.recent.iter().map(|&i| &self.entries[i]).collect()
    }

    pub fn get(&self, id: &str) -> Option<&Entry> {
        self.by_id.get(id).map(|&i| &self.entries[i])
    }

    /// Every author, by name.
    pub fn authors(&self) -> &[Group] {
        &self.authors
    }

    /// Every series, by name.
    pub fn series_list(&self) -> &[Group] {
        &self.series
    }

    /// The author called `name` (compared folded: case, accents and spacing
    /// don't matter).
    pub fn author(&self, name: &str) -> Option<&Group> {
        self.author_keys.get(&fold(name)).map(|&i| &self.authors[i])
    }

    /// The series called `name` (compared folded).
    pub fn series(&self, name: &str) -> Option<&Group> {
        self.series_keys.get(&fold(name)).map(|&i| &self.series[i])
    }

    /// The books of a group, in its order.
    pub fn books_of(&self, group: &Group) -> Vec<&Entry> {
        group.books.iter().map(|&i| &self.entries[i]).collect()
    }

    /// Books whose title, subtitle, authors, series or ISBN contain every
    /// word of `query` (folded: `etranger` finds *L'Étranger*), by title. An
    /// empty query finds nothing.
    pub fn search(&self, query: &str) -> Vec<&Entry> {
        let query: String = query.chars().take(MAX_QUERY_CHARS).collect();
        let folded = fold(&query);
        let terms: Vec<&str> = folded
            .split(' ')
            .filter(|t| !t.is_empty())
            .take(MAX_QUERY_TERMS)
            .collect();
        if terms.is_empty() {
            return Vec::new();
        }
        self.entries
            .iter()
            .filter(|e| terms.iter().all(|t| e.haystack.contains(t)))
            .collect()
    }
}

/// Where a book sits in its series; books without an index go last.
fn series_position(entry: &Entry) -> f32 {
    entry
        .book
        .metadata
        .series_index
        .filter(|i| i.is_finite())
        .unwrap_or(f32::MAX)
}

/// Group entries by the names `names_of` gives each (folded), keeping the
/// first spelling met. Returns the groups sorted by folded name, and the
/// folded name → group index map.
fn group_by<'a>(
    entries: &'a [Entry],
    names_of: impl Fn(&'a Entry) -> Vec<&'a str>,
) -> (Vec<Group>, HashMap<String, usize>) {
    let mut groups: Vec<Group> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for (i, entry) in entries.iter().enumerate() {
        for name in names_of(entry) {
            let key = fold(name);
            if key.is_empty() {
                continue;
            }
            let g = *index.entry(key.clone()).or_insert_with(|| {
                groups.push(Group {
                    name: name.trim().to_string(),
                    key,
                    books: Vec::new(),
                });
                groups.len() - 1
            });
            // The same author listed twice on one book counts once.
            if groups[g].books.last() != Some(&i) {
                groups[g].books.push(i);
            }
        }
    }
    groups.sort_by(|a, b| a.key.cmp(&b.key));
    let index = groups
        .iter()
        .enumerate()
        .map(|(i, g)| (g.key.clone(), i))
        .collect();
    (groups, index)
}

/// Text folded for comparing and sorting: accents removed (decomposed, then
/// combining marks dropped), lowercase, `œ æ ß` spelled out, whitespace
/// collapsed to single spaces.
pub(super) fn fold(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.nfd().filter(|c| !is_combining_mark(*c)) {
        for lower in c.to_lowercase() {
            match lower {
                'œ' => out.push_str("oe"),
                'æ' => out.push_str("ae"),
                'ß' => out.push_str("ss"),
                other => out.push(other),
            }
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `book id → file` for the images directly in `dir` with one of the
/// `extensions` (lowercase). The id is the file name without extension.
fn image_files(dir: &Path, extensions: &[&str]) -> HashMap<String, PathBuf> {
    let mut map = HashMap::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase);
        if !ext.as_deref().is_some_and(|e| extensions.contains(&e)) {
            continue;
        }
        if let Some(id) = path.file_stem().and_then(|s| s.to_str()) {
            map.insert(id.to_string(), path.clone());
        }
    }
    map
}

// ── Pagination ───────────────────────────────────────────────────────────────

/// The page a client asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PageRequest {
    /// 1-based.
    pub number: usize,
    /// Between 1 and [`MAX_PAGE_SIZE`].
    pub size: usize,
}

impl PageRequest {
    /// From the `page` and `limit` query parameters, leniently: a missing,
    /// unparsable or zero value gives the default (page 1,
    /// [`DEFAULT_PAGE_SIZE`]); a limit above [`MAX_PAGE_SIZE`] is capped.
    pub fn parse(page: Option<&str>, limit: Option<&str>) -> Self {
        let positive = |v: Option<&str>| {
            v.and_then(|v| v.trim().parse::<usize>().ok())
                .filter(|&n| n > 0)
        };
        PageRequest {
            number: positive(page).unwrap_or(1),
            size: positive(limit).map_or(DEFAULT_PAGE_SIZE, |n| n.min(MAX_PAGE_SIZE)),
        }
    }
}

/// Where a page sits in its list, for the pagination links.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PageInfo {
    pub number: usize,
    pub size: usize,
    pub total: usize,
}

impl PageInfo {
    /// The last page (1 for an empty list).
    pub fn last(&self) -> usize {
        self.total.div_ceil(self.size).max(1)
    }

    pub fn next(&self) -> Option<usize> {
        (self.number < self.last()).then_some(self.number + 1)
    }

    /// The page before; for a page past the end, the last page.
    pub fn previous(&self) -> Option<usize> {
        (self.number > 1).then(|| (self.number - 1).min(self.last()))
    }

    /// 1-based position of the page's first item (OpenSearch `startIndex`).
    pub fn start_index(&self) -> usize {
        (self.number - 1)
            .saturating_mul(self.size)
            .saturating_add(1)
    }
}

/// The requested page of `items` (empty past the end).
pub(super) fn paginate<T>(items: &[T], request: PageRequest) -> (&[T], PageInfo) {
    let start = (request.number - 1)
        .saturating_mul(request.size)
        .min(items.len());
    let end = start.saturating_add(request.size).min(items.len());
    (
        &items[start..end],
        PageInfo {
            number: request.number,
            size: request.size,
            total: items.len(),
        },
    )
}

// ── Cache ────────────────────────────────────────────────────────────────────

/// Size, modification time and (on Unix) file id of one file or folder;
/// `None` when it doesn't exist.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FileStamp {
    len: u64,
    modified: Option<SystemTime>,
    #[cfg(unix)]
    ino: u64,
}

fn file_stamp(path: &Path) -> Option<FileStamp> {
    let meta = std::fs::metadata(path).ok()?;
    Some(FileStamp {
        len: meta.len(),
        modified: meta.modified().ok(),
        #[cfg(unix)]
        ino: std::os::unix::fs::MetadataExt::ino(&meta),
    })
}

/// The state of every file a catalog is built from: the index files
/// ([`snapshot_sources`]), then `.covers/` and `.covers/thumbs/` (a folder's
/// modification time changes when a file is added, removed or replaced in
/// it, and covers are always written by rename).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Stamp {
    index: [Option<FileStamp>; 4],
    covers: [Option<FileStamp>; 2],
}

impl Stamp {
    fn of(root: &Path) -> Self {
        Stamp {
            index: snapshot_sources(root).map(|p| file_stamp(&p)),
            covers: [covers_dir(root), thumbs_dir(root)].map(|p| file_stamp(&p)),
        }
    }

    /// Modification time of the first index file that exists (the one read).
    fn index_modified(&self) -> Option<SystemTime> {
        self.index.iter().flatten().find_map(|s| s.modified)
    }
}

/// A catalog and the state of the files it was built from.
struct Cached {
    root: PathBuf,
    stamp: Stamp,
    catalog: Arc<Catalog>,
    /// When `stamp` was last compared with the files.
    checked: StdMutex<Instant>,
}

impl Cached {
    fn checked_within(&self, period: Duration) -> bool {
        self.checked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .elapsed()
            < period
    }

    fn mark_checked(&self) {
        *self.checked.lock().unwrap_or_else(PoisonError::into_inner) = Instant::now();
    }
}

/// The last catalog built, rebuilt when stale (see the module docs).
pub(super) struct CatalogCache {
    slot: StdRwLock<Option<Arc<Cached>>>,
    /// Bumped by every invalidation, so a load that started before one
    /// doesn't store its (possibly stale) result after it.
    generation: AtomicU64,
    /// One load at a time: requests arriving during a load wait for it and
    /// reuse its result instead of reading the index again.
    loading: tokio::sync::Mutex<()>,
    recheck_after: Duration,
}

impl Default for CatalogCache {
    fn default() -> Self {
        Self::new(RECHECK_AFTER)
    }
}

impl CatalogCache {
    pub fn new(recheck_after: Duration) -> Self {
        CatalogCache {
            slot: StdRwLock::new(None),
            generation: AtomicU64::new(0),
            loading: tokio::sync::Mutex::new(()),
            recheck_after,
        }
    }

    /// Forget the catalog; the next request rebuilds it.
    pub fn invalidate(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        *self.slot.write().unwrap_or_else(PoisonError::into_inner) = None;
    }

    fn current(&self, root: &Path) -> Option<Arc<Cached>> {
        self.slot
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .filter(|c| c.root == root)
            .cloned()
    }

    /// The catalog of the library at `root`, rebuilt first if it is stale.
    pub async fn get(self: &Arc<Self>, root: &Path) -> Arc<Catalog> {
        if let Some(cached) = self.current(root) {
            if cached.checked_within(self.recheck_after) {
                return Arc::clone(&cached.catalog);
            }
            // The library may be on a NAS: `stat` on the blocking pool.
            let dir = root.to_path_buf();
            if let Ok(stamp) = tokio::task::spawn_blocking(move || Stamp::of(&dir)).await {
                if stamp == cached.stamp {
                    cached.mark_checked();
                    return Arc::clone(&cached.catalog);
                }
            }
        }
        // Spawned, so a load whose request gave up (timeout, client gone)
        // still completes and fills the cache for the next request.
        let (this, root) = (Arc::clone(self), root.to_path_buf());
        tokio::spawn(async move { this.load(root).await })
            .await
            .unwrap_or_default()
    }

    async fn load(&self, root: PathBuf) -> Arc<Catalog> {
        let _one_at_a_time = self.loading.lock().await;
        let generation = self.generation.load(Ordering::SeqCst);
        let previous = self.current(&root);
        let previous_stamp = previous.as_ref().map(|c| c.stamp.clone());
        let dir = root.clone();
        let read = tokio::task::spawn_blocking(move || {
            // Stamp first: a change made while the index is being read then
            // shows as a difference on the next check (one reload too many,
            // never a stale catalog kept).
            let stamp = Stamp::of(&dir);
            if previous_stamp.as_ref() == Some(&stamp) {
                return (stamp, None);
            }
            let updated = stamp
                .index_modified()
                .map_or_else(Utc::now, DateTime::<Utc>::from);
            let catalog = Catalog::load(&dir, updated);
            (stamp, Some(catalog))
        });
        // A panic while reading (a bug, not bad data: the index reader
        // doesn't panic on bad input) serves an empty catalog, not cached.
        let Ok((stamp, catalog)) = read.await else {
            return Arc::default();
        };
        let catalog = match (catalog, previous) {
            (Some(catalog), _) => Arc::new(catalog),
            // Unchanged: another request rebuilt it while this one waited.
            (None, Some(previous)) => {
                previous.mark_checked();
                return Arc::clone(&previous.catalog);
            }
            (None, None) => Arc::default(),
        };
        let mut slot = self.slot.write().unwrap_or_else(PoisonError::into_inner);
        if self.generation.load(Ordering::SeqCst) == generation {
            *slot = Some(Arc::new(Cached {
                root,
                stamp,
                catalog: Arc::clone(&catalog),
                checked: StdMutex::new(Instant::now()),
            }));
        }
        catalog
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::book::{BookFormat, BookMetadata};

    fn book(id: &str, title: &str, author: &str, series: Option<(&str, f32)>, added: &str) -> Book {
        Book {
            id: id.into(),
            path: format!("/lib/{id}.epub"),
            filename: format!("{id}.epub"),
            format: BookFormat::Epub,
            size_bytes: 1,
            date_added: added.into(),
            cover_path: None,
            cover_version: None,
            favorite: false,
            metadata: BookMetadata {
                title: title.into(),
                author: author.into(),
                series: series.map(|s| s.0.to_string()),
                series_index: series.map(|s| s.1),
                ..Default::default()
            },
        }
    }

    fn date(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn catalog() -> Catalog {
        let books = vec![
            book(
                "b3",
                "Les Misérables",
                "Victor Hugo",
                None,
                "2026-03-01T00:00:00Z",
            ),
            book(
                "b1",
                "L'Étranger",
                "Albert Camus",
                None,
                "2026-01-01T00:00:00Z",
            ),
            book(
                "b5",
                "Le Cycle de Fondation",
                "Isaac Asimov",
                Some(("Fondation", 1.0)),
                "bad date",
            ),
            book(
                "b4",
                "Seconde Fondation",
                "isaac  asimov",
                Some(("fondation", 3.0)),
                "2026-02-01T00:00:00Z",
            ),
            book(
                "b6",
                "Fondation et Empire",
                "Isaac Asimov & Victor Hugo",
                Some(("Fondation", 2.0)),
                "2026-02-02T00:00:00Z",
            ),
            book("b2", "", "", None, "2026-01-02T00:00:00Z"),
        ];
        let covers = HashMap::from([("b1".to_string(), PathBuf::from("/lib/.covers/b1.png"))]);
        let thumbs = HashMap::from([(
            "b3".to_string(),
            PathBuf::from("/lib/.covers/thumbs/b3.jpg"),
        )]);
        Catalog::from_books(books, &covers, &thumbs, date("2026-06-01T00:00:00Z"))
    }

    fn titles(entries: &[&Entry]) -> Vec<String> {
        entries.iter().map(|e| e.title.clone()).collect()
    }

    #[test]
    fn folding_ignores_case_accents_and_spacing() {
        assert_eq!(fold("  L'Étranger \t Œuvre  "), "l'etranger oeuvre");
        assert_eq!(fold("Straße ÆSIR"), "strasse aesir");
        assert_eq!(fold("日本語"), "日本語");
    }

    #[test]
    fn entries_are_sorted_by_title_with_fallbacks() {
        let c = catalog();
        // The untitled book shows its file name; the bad date falls back to
        // the catalog's date.
        assert_eq!(
            titles(&c.all()),
            [
                "b2",
                "Fondation et Empire",
                "L'Étranger",
                "Le Cycle de Fondation",
                "Les Misérables",
                "Seconde Fondation",
            ]
        );
        assert_eq!(c.get("b5").unwrap().added, date("2026-06-01T00:00:00Z"));
        assert_eq!(
            titles(&c.recent())[..2],
            ["Le Cycle de Fondation", "Les Misérables"]
        );
        assert!(c.get("nope").is_none());
    }

    #[test]
    fn covers_fall_back_to_each_other() {
        let c = catalog();
        let b1 = c.get("b1").unwrap();
        assert_eq!(b1.cover_file(), Some(Path::new("/lib/.covers/b1.png")));
        assert_eq!(b1.thumbnail_file(), Some(Path::new("/lib/.covers/b1.png")));
        let b3 = c.get("b3").unwrap();
        assert_eq!(
            b3.cover_file(),
            Some(Path::new("/lib/.covers/thumbs/b3.jpg"))
        );
        assert!(c.get("b2").unwrap().cover_file().is_none());
    }

    #[test]
    fn authors_are_split_and_grouped_case_insensitively() {
        let c = catalog();
        let names: Vec<&str> = c.authors().iter().map(|g| g.name.as_str()).collect();
        assert_eq!(
            names,
            ["Albert Camus", "Isaac Asimov", "Victor Hugo"],
            "unknown author left out"
        );
        let asimov = c.author("ISAAC ASIMOV").unwrap();
        assert_eq!(asimov.len(), 3);
        assert_eq!(
            titles(&c.books_of(asimov)),
            [
                "Le Cycle de Fondation",
                "Fondation et Empire",
                "Seconde Fondation"
            ]
        );
        let hugo = c.author("victor hugo").unwrap();
        assert_eq!(
            titles(&c.books_of(hugo)),
            ["Fondation et Empire", "Les Misérables"]
        );
        assert!(c.author("nobody").is_none());
    }

    #[test]
    fn series_are_in_reading_order() {
        let c = catalog();
        assert_eq!(c.series_list().len(), 1);
        let fondation = c.series("FONDATION").unwrap();
        assert_eq!(
            fondation.name, "Fondation",
            "first spelling met in title order"
        );
        assert_eq!(
            titles(&c.books_of(fondation)),
            [
                "Le Cycle de Fondation",
                "Fondation et Empire",
                "Seconde Fondation"
            ]
        );
    }

    #[test]
    fn search_matches_every_word_across_fields() {
        let c = catalog();
        assert_eq!(titles(&c.search("etranger")), ["L'Étranger"]);
        assert_eq!(titles(&c.search("hugo fondation")), ["Fondation et Empire"]);
        assert_eq!(c.search("FONDATION").len(), 3, "series and titles");
        assert_eq!(c.search("asimov").len(), 3);
        assert!(c.search("   ").is_empty());
        assert!(c.search("camus fondation").is_empty());
        let long = "a".repeat(10_000);
        assert!(c.search(&long).is_empty());
    }

    #[test]
    fn page_requests_are_lenient_and_capped() {
        assert_eq!(
            PageRequest::parse(None, None),
            PageRequest {
                number: 1,
                size: DEFAULT_PAGE_SIZE
            }
        );
        assert_eq!(
            PageRequest::parse(Some("3"), Some("10")),
            PageRequest {
                number: 3,
                size: 10
            }
        );
        assert_eq!(
            PageRequest::parse(Some("0"), Some("x")),
            PageRequest {
                number: 1,
                size: DEFAULT_PAGE_SIZE
            }
        );
        assert_eq!(
            PageRequest::parse(Some("-1"), Some("100000")),
            PageRequest {
                number: 1,
                size: MAX_PAGE_SIZE
            }
        );
        let huge = PageRequest::parse(Some(&usize::MAX.to_string()), None);
        let (items, info) = paginate(&[1, 2, 3], huge);
        assert!(items.is_empty());
        assert_eq!(info.previous(), Some(1));
        assert_eq!(info.start_index(), usize::MAX);
    }

    #[test]
    fn pagination_slices_and_links() {
        let items: Vec<u32> = (1..=5).collect();
        let (page, info) = paginate(&items, PageRequest { number: 1, size: 2 });
        assert_eq!(page, [1, 2]);
        assert_eq!(
            (
                info.previous(),
                info.next(),
                info.last(),
                info.start_index()
            ),
            (None, Some(2), 3, 1)
        );
        let (page, info) = paginate(&items, PageRequest { number: 3, size: 2 });
        assert_eq!(page, [5]);
        assert_eq!(
            (info.previous(), info.next(), info.start_index()),
            (Some(2), None, 5)
        );
        let (page, info) = paginate::<u32>(&[], PageRequest { number: 1, size: 2 });
        assert!(page.is_empty());
        assert_eq!((info.previous(), info.next(), info.last()), (None, None, 1));
    }

    /// A library with an index of `n` books.
    fn write_index(root: &Path, n: usize) {
        std::fs::create_dir_all(root.join(".librarium")).unwrap();
        let books: serde_json::Map<String, serde_json::Value> = (0..n)
            .map(|i| {
                let name = format!("b{i}.epub");
                std::fs::write(root.join(&name), b"EPUB").unwrap();
                let record = serde_json::json!({
                    "id": format!("id-{i}"), "path": name, "filename": name, "format": "epub",
                    "sizeBytes": 4, "dateAdded": "2026-01-01T00:00:00Z",
                    "metadata": {"title": format!("Book {i}"), "author": "A"},
                });
                (name, record)
            })
            .collect();
        let index = serde_json::json!({"version": 2, "libraryId": "t", "books": books});
        std::fs::write(root.join(".librarium/library.json"), index.to_string()).unwrap();
    }

    #[tokio::test]
    async fn cache_reloads_when_the_index_changes() {
        let lib = tempfile::tempdir().unwrap();
        write_index(lib.path(), 1);
        let cache = Arc::new(CatalogCache::new(Duration::ZERO));
        let first = cache.get(lib.path()).await;
        assert_eq!(first.len(), 1);
        assert!(
            Arc::ptr_eq(&first, &cache.get(lib.path()).await),
            "unchanged: the same catalog"
        );

        write_index(lib.path(), 3);
        assert_eq!(
            cache.get(lib.path()).await.len(),
            3,
            "noticed without an explicit invalidation"
        );

        std::fs::remove_file(lib.path().join(".librarium/library.json")).unwrap();
        assert_eq!(cache.get(lib.path()).await.len(), 0);
    }

    #[tokio::test]
    async fn cache_notices_new_covers() {
        let lib = tempfile::tempdir().unwrap();
        write_index(lib.path(), 1);
        std::fs::create_dir_all(lib.path().join(".covers/thumbs")).unwrap();
        let cache = Arc::new(CatalogCache::new(Duration::ZERO));
        assert!(cache
            .get(lib.path())
            .await
            .get("id-0")
            .unwrap()
            .thumbnail
            .is_none());
        std::fs::write(lib.path().join(".covers/thumbs/id-0.jpg"), b"\xFF\xD8\xFF").unwrap();
        assert!(cache
            .get(lib.path())
            .await
            .get("id-0")
            .unwrap()
            .thumbnail
            .is_some());
    }

    #[tokio::test]
    async fn cache_rechecks_at_most_once_per_period_unless_invalidated() {
        let lib = tempfile::tempdir().unwrap();
        write_index(lib.path(), 1);
        let cache = Arc::new(CatalogCache::new(Duration::from_secs(3600)));
        assert_eq!(cache.get(lib.path()).await.len(), 1);
        write_index(lib.path(), 2);
        assert_eq!(
            cache.get(lib.path()).await.len(),
            1,
            "checked moments ago: served from memory"
        );
        cache.invalidate();
        assert_eq!(cache.get(lib.path()).await.len(), 2);
    }

    #[tokio::test]
    async fn concurrent_requests_share_one_catalog() {
        let lib = tempfile::tempdir().unwrap();
        write_index(lib.path(), 2);
        let cache = Arc::new(CatalogCache::new(Duration::ZERO));
        let gets: Vec<_> = (0..8)
            .map(|_| {
                let (cache, root) = (Arc::clone(&cache), lib.path().to_path_buf());
                tokio::spawn(async move { cache.get(&root).await })
            })
            .collect();
        let mut catalogs = Vec::new();
        for g in gets {
            catalogs.push(g.await.unwrap());
        }
        assert!(
            catalogs.iter().all(|c| Arc::ptr_eq(c, &catalogs[0])),
            "loaded once, then reused"
        );
    }
}
