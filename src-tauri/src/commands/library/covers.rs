//! Book covers: extraction from the book file (EPUB, CBZ, PDF), the cover
//! cache with its thumbnails, and setting or removing a cover.
//!
//! ## On disk: `<library>/.covers/`
//!
//! | File | Content |
//! |---|---|
//! | `<id>.<jpg\|png\|webp>` | the full-size cover, as found in the book or set by the user; the OPDS server serves it |
//! | `thumbs/<id>.jpg` | the thumbnail the app shows: at most [`THUMB_MAX`] px on each side, JPEG quality [`THUMB_QUALITY`] |
//! | `<id>.none` | "this book has no cover": the negative cache, see below |
//!
//! Every file is written atomically ([`fsutil::atomic_write`]). Writes for one
//! book are serialised by a per-book lock ([`with_cover_lock`]); lock order
//! is index → cover (`edit.rs` removes covers while holding the index lock),
//! so nothing here takes the index lock while holding a cover lock.
//!
//! ## How covers reach the webview
//!
//! As files, never as data URLs (those used to cost 1–2 GB of strings per
//! launch for a 5k-book library). The frontend shows
//! `convertFileSrc(coverPath)` through Tauri's asset protocol, whose scope
//! the command layer opens for the open library's `.covers` folder only. A
//! [`CoverInfo`] carries the path (the thumbnail, or the full cover when no
//! thumbnail could be made) and its modification time (`coverVersion`),
//! which the frontend appends to the URL so a replaced cover isn't served
//! from the webview's cache.
//!
//! ## When work happens
//!
//! - [`attach_cached_covers`], during `scan_library`: one listing of
//!   `thumbs/`. No image decoding, no book parsing.
//! - [`ensure_cover`], the `ensure_cover` command, which the frontend calls
//!   lazily for cards that become visible: thumbnail → full cover (make the
//!   thumbnail) → negative cache → look inside the book.
//! - [`persist_cover`] (the user sets a cover), [`delete_cover`] and
//!   [`remove_cached_covers`] (reset, delete).
//!
//! ## Negative cache
//!
//! A book in which no usable cover was found gets `<id>.none`, recording the
//! size and modification time of the book file (and its path, relative to the
//! library). While the file is unchanged, [`ensure_cover`] answers "no cover"
//! from that marker without opening the book or reading the index. When the
//! file changes (a cover was added in another app, the book was replaced),
//! the marker no longer matches and the book is looked at again. Removing a
//! cover in Librarium writes a marker too, so a cover the book file still
//! holds (a PDF's first page, a CBZ's first image) doesn't come back.

use std::collections::HashMap;
use std::fs;
use std::io::{self, Cursor, Read};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use base64::prelude::*;
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader};
use serde::{Deserialize, Serialize};

use super::epub_edit;
use super::files::ensure_in_library;
use super::index::find_book;
use super::sniff::ImageKind;
use crate::error::{AppError, AppResult};
use crate::fsutil;
use crate::logging::Log;
use crate::models::book::{Book, BookFormat};
use crate::net;

/// Largest cover accepted from the file picker or a URL.
const MAX_COVER_BYTES: usize = net::COVER_LIMITS.max_bytes as usize;
/// Largest image read out of a book file. Scanned PDFs carry big page
/// images; anything larger is not a cover worth decoding (or a zip bomb).
const MAX_EXTRACTED_BYTES: u64 = 50 * 1024 * 1024;

/// Thumbnails fit in a `THUMB_MAX` × `THUMB_MAX` box (a 2:3 cover becomes
/// 400 × 600): sharp on a high-DPI screen for the largest card and the
/// detail panel, about 40–80 KB each.
pub(crate) const THUMB_MAX: u32 = 600;
/// JPEG quality of the thumbnails.
pub(crate) const THUMB_QUALITY: u8 = 82;

/// A cover the webview can show (`Book.coverPath` / `coverVersion`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverInfo {
    /// Absolute path of the thumbnail, or of the full cover if no thumbnail
    /// could be made.
    pub cover_path: String,
    /// Modification time of that file in milliseconds since the epoch, used
    /// as a cache buster (`?v=`).
    pub cover_version: u64,
}

// ── Paths ─────────────────────────────────────────────────────────────────────

/// `<library>/.covers`: cover images cached (or set by the user) per book id.
pub(crate) fn covers_dir(library: &Path) -> PathBuf {
    library.join(".covers")
}

/// `<library>/.covers/thumbs`.
pub(crate) fn thumbs_dir(library: &Path) -> PathBuf {
    covers_dir(library).join("thumbs")
}

/// `<library>/.covers/<id>.<jpg|png|webp>`: the full-size cover.
pub(crate) fn cover_cache_path(library: &Path, id: &str, kind: ImageKind) -> PathBuf {
    covers_dir(library).join(format!("{}.{}", id, kind.extension()))
}

/// `<library>/.covers/thumbs/<id>.jpg`.
pub(crate) fn thumb_path(library: &Path, id: &str) -> PathBuf {
    thumbs_dir(library).join(format!("{id}.jpg"))
}

/// `<library>/.covers/<id>.none`: the negative cache.
fn marker_path(library: &Path, id: &str) -> PathBuf {
    covers_dir(library).join(format!("{id}.none"))
}

/// Whether `id` can safely be used as a file name. Ids are UUIDs generated by
/// the backend; anything else (`../x`) coming from the webview is refused so
/// it can't be joined into a path outside `.covers/`.
pub(crate) fn is_safe_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn check_id(id: &str) -> AppResult<()> {
    if is_safe_id(id) {
        Ok(())
    } else {
        Err(AppError::InvalidInput(format!("Invalid book id: {id:?}")))
    }
}

/// Run `f` while holding the cover lock of book `id` (in-process; see the
/// module docs for the lock order).
fn with_cover_lock<T>(library: &Path, id: &str, f: impl FnOnce() -> T) -> T {
    fsutil::with_file_lock(&covers_dir(library).join(format!("{id}.lock")), f)
}

/// Modification time in milliseconds since the epoch (0 if unknown).
fn mtime_ms(meta: &fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as u64)
}

/// The [`CoverInfo`] of an existing file, `None` if it doesn't exist.
fn cover_info(path: &Path) -> Option<CoverInfo> {
    let meta = fs::metadata(path).ok().filter(fs::Metadata::is_file)?;
    Some(CoverInfo {
        cover_path: path.to_string_lossy().into_owned(),
        cover_version: mtime_ms(&meta),
    })
}

/// The full-size cover of book `id`, if cached.
fn full_cover(library: &Path, id: &str) -> Option<PathBuf> {
    ImageKind::ALL
        .into_iter()
        .map(|kind| cover_cache_path(library, id, kind))
        .find(|path| path.is_file())
}

/// Remove `path`; a missing file is fine.
fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Every cache file of book `id`: full covers of each type, thumbnail, marker.
fn remove_cover_files(library: &Path, id: &str) -> io::Result<()> {
    for kind in ImageKind::ALL {
        remove_if_present(&cover_cache_path(library, id, kind))?;
    }
    remove_if_present(&thumb_path(library, id))?;
    remove_if_present(&marker_path(library, id))
}

/// Forget everything cached about book `id`'s cover (full cover, thumbnail,
/// negative cache): used by reset (the cover is extracted again from the
/// file) and delete. A missing file is fine. An id that isn't a safe file
/// name can't have cache files, so nothing is touched.
pub(crate) fn remove_cached_covers(library: &Path, id: &str) -> io::Result<()> {
    if !is_safe_id(id) {
        return Ok(());
    }
    with_cover_lock(library, id, || remove_cover_files(library, id))
}

// ── Negative cache ────────────────────────────────────────────────────────────

/// Content of `<id>.none` (JSON): which file had no cover, and its state then.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NoCoverMarker {
    /// The book file, relative to the library with `/` separators (or
    /// absolute if it lies outside), so the marker survives a move of the
    /// whole library and is shared by Mac and Windows clients of a NAS.
    path: String,
    size: u64,
    mtime_ms: u64,
}

impl NoCoverMarker {
    /// The marker describing `book_path` as it is now.
    fn for_file(library: &Path, book_path: &Path) -> io::Result<Self> {
        let meta = fs::metadata(book_path)?;
        let path = match book_path.strip_prefix(library) {
            Ok(rel) => rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/"),
            Err(_) => book_path.to_string_lossy().into_owned(),
        };
        Ok(Self {
            path,
            size: meta.len(),
            mtime_ms: mtime_ms(&meta),
        })
    }

    /// The book file this marker describes. A relative path may not climb
    /// out of the library (a hand-edited marker is only ever stat'ed, but
    /// stay strict anyway).
    fn book_path(&self, library: &Path) -> Option<PathBuf> {
        let path = Path::new(&self.path);
        if path.is_absolute() {
            return Some(path.to_path_buf());
        }
        let mut resolved = library.to_path_buf();
        for part in self.path.split('/') {
            // Exactly one plain name: no "", ".", "..", drive prefix or `\`.
            let mut components = Path::new(part).components();
            match (components.next(), components.next()) {
                (Some(Component::Normal(name)), None) => resolved.push(name),
                _ => return None,
            }
        }
        Some(resolved)
    }

    /// Whether the book file is still exactly as recorded.
    fn is_current(&self, library: &Path) -> bool {
        self.book_path(library)
            .and_then(|path| Self::for_file(library, &path).ok())
            .is_some_and(|now| now.size == self.size && now.mtime_ms == self.mtime_ms)
    }
}

/// Whether book `id` has a marker that still matches its file.
fn negative_cache_is_current(library: &Path, id: &str) -> bool {
    fs::read(marker_path(library, id))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<NoCoverMarker>(&bytes).ok())
        .is_some_and(|marker| marker.is_current(library))
}

/// Record that `book_path` (book `id`) has no cover. Best effort: without
/// the marker the book is only looked at again next time.
fn write_negative_cache(library: &Path, id: &str, marker: &NoCoverMarker, log: Log) {
    let written = fs::create_dir_all(covers_dir(library)).and_then(|()| {
        let json = serde_json::to_vec(marker).map_err(io::Error::other)?;
        fsutil::atomic_write(&marker_path(library, id), &json)
    });
    if let Err(e) = written {
        log(format!(
            "[covers] could not record that {id} has no cover: {e}"
        ));
    }
}

// ── Thumbnails ────────────────────────────────────────────────────────────────

/// Decoder limits: covers are at most a few thousand pixels on a side; this
/// refuses decompression bombs before anything big is allocated.
fn decode_limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(300 * 1024 * 1024);
    limits
}

fn image_format(kind: ImageKind) -> ImageFormat {
    match kind {
        ImageKind::Jpeg => ImageFormat::Jpeg,
        ImageKind::Png => ImageFormat::Png,
        ImageKind::Webp => ImageFormat::WebP,
    }
}

/// Decode a JPEG, PNG or WebP image, upright (EXIF orientation applied, as
/// the webview does when it shows the full image).
fn decode_image(bytes: &[u8]) -> Result<DynamicImage, String> {
    let kind = ImageKind::sniff(bytes).ok_or("not a JPEG, PNG or WebP image")?;
    let mut reader = ImageReader::with_format(Cursor::new(bytes), image_format(kind));
    reader.limits(decode_limits());
    let mut decoder = reader.into_decoder().map_err(|e| e.to_string())?;
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut img = DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    img.apply_orientation(orientation);
    Ok(img)
}

/// The thumbnail of a cover image: fitted in [`THUMB_MAX`] × [`THUMB_MAX`]
/// (never enlarged), transparency flattened onto white, encoded as JPEG at
/// [`THUMB_QUALITY`]. Fails on anything that isn't a decodable JPEG, PNG or
/// WebP image.
pub(crate) fn make_thumbnail(bytes: &[u8]) -> Result<Vec<u8>, String> {
    fit_as_jpeg(bytes, THUMB_MAX, THUMB_MAX, THUMB_QUALITY)
}

/// A cover image fitted in `max_width` × `max_height` (aspect ratio kept,
/// never enlarged), transparency flattened onto white, encoded as JPEG at
/// `quality`. Used for the library's thumbnails and for the e-reader's own
/// home-screen thumbnails (see `commands::device::thumbnail`).
pub(crate) fn fit_as_jpeg(
    bytes: &[u8],
    max_width: u32,
    max_height: u32,
    quality: u8,
) -> Result<Vec<u8>, String> {
    let img = decode_image(bytes)?;
    let img = if img.width() > max_width || img.height() > max_height {
        // Catmull-Rom: sharp enough for the lettering on covers, and the
        // filter support scales with the ratio, so big reductions don't alias.
        img.resize(
            max_width,
            max_height,
            image::imageops::FilterType::CatmullRom,
        )
    } else {
        img
    };
    let rgb = flatten_on_white(&img);
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality)
        .encode_image(&rgb)
        .map_err(|e| e.to_string())?;
    Ok(out)
}

/// The best cover image of a library book, as bytes: the full-size cached
/// cover, else the library thumbnail; extracted from the book first if it
/// was never looked at ([`ensure_cover`]). `None` when the book has none.
pub(crate) fn cover_image(library: &Path, id: &str, log: Log) -> Option<Vec<u8>> {
    ensure_cover(library, id, log).ok().flatten()?;
    let path = full_cover(library, id).unwrap_or_else(|| thumb_path(library, id));
    fs::read(path).ok()
}

/// RGB pixels, with transparent areas composited onto white (JPEG has no
/// alpha; dropping it would turn them black).
fn flatten_on_white(img: &DynamicImage) -> image::RgbImage {
    if !img.color().has_alpha() {
        return img.to_rgb8();
    }
    let rgba = img.to_rgba8();
    image::RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let [r, g, b, a] = rgba.get_pixel(x, y).0;
        let over_white = |c: u8| ((c as u32 * a as u32 + 255 * (255 - a as u32) + 127) / 255) as u8;
        image::Rgb([over_white(r), over_white(g), over_white(b)])
    })
}

/// Write the thumbnail of book `id`.
fn write_thumbnail(library: &Path, id: &str, jpeg: &[u8]) -> io::Result<CoverInfo> {
    fs::create_dir_all(thumbs_dir(library))?;
    let path = thumb_path(library, id);
    fsutil::atomic_write(&path, jpeg)?;
    cover_info(&path).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "thumbnail vanished"))
}

/// Store `bytes` (a `kind` image) as book `id`'s cover: the full-size file,
/// then the thumbnail (`thumb`, already made or the reason it couldn't be).
/// Clears what the new cover supersedes: a full cover of another type (it
/// would win the lookup), a stale thumbnail, the negative cache. Call with
/// the cover lock held.
fn store_cover(
    library: &Path,
    id: &str,
    bytes: &[u8],
    kind: ImageKind,
    thumb: Result<Vec<u8>, String>,
    log: Log,
) -> AppResult<CoverInfo> {
    let fail = |e: io::Error| AppError::io("Could not save the cover", &e);
    fs::create_dir_all(covers_dir(library)).map_err(fail)?;
    let full = cover_cache_path(library, id, kind);
    fsutil::atomic_write(&full, bytes).map_err(fail)?;
    for other in ImageKind::ALL.into_iter().filter(|k| *k != kind) {
        remove_if_present(&cover_cache_path(library, id, other)).map_err(fail)?;
    }
    remove_if_present(&marker_path(library, id)).map_err(fail)?;

    let thumbnail =
        thumb.and_then(|jpeg| write_thumbnail(library, id, &jpeg).map_err(|e| e.to_string()));
    match thumbnail {
        Ok(info) => Ok(info),
        Err(e) => {
            log(format!(
                "[covers] no thumbnail for {id} ({e}); the full-size cover is used"
            ));
            // A thumbnail of the previous cover would hide the new one.
            remove_if_present(&thumb_path(library, id)).map_err(fail)?;
            cover_info(&full)
                .ok_or_else(|| AppError::Io("The cover could not be saved".to_string()))
        }
    }
}

// ── Scans ─────────────────────────────────────────────────────────────────────

/// `book id → thumbnail` for every thumbnail in `thumbs/` (one directory
/// listing).
fn list_thumbnails(library: &Path) -> HashMap<String, CoverInfo> {
    let mut map = HashMap::new();
    for entry in fs::read_dir(thumbs_dir(library))
        .into_iter()
        .flatten()
        .flatten()
    {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jpg") {
            continue; // temp files of an interrupted write (`.librarium-*.part`)
        }
        let Some(id) = path
            .file_stem()
            .and_then(|s| s.to_str())
            .filter(|id| is_safe_id(id))
        else {
            continue;
        };
        if let Ok(meta) = entry.metadata() {
            let info = CoverInfo {
                cover_path: path.to_string_lossy().into_owned(),
                cover_version: mtime_ms(&meta),
            };
            map.insert(id.to_string(), info);
        }
    }
    map
}

/// Fill `cover_path` / `cover_version` of scanned `books` from the
/// thumbnails already made. Cheap (one listing, no decoding): books without
/// a thumbnail are left without a cover, and the frontend asks
/// [`ensure_cover`] for them when they become visible. Values that may have
/// been stored in an old index are overwritten.
pub(crate) fn attach_cached_covers(library: &Path, books: &mut [Book]) {
    let thumbs = list_thumbnails(library);
    for book in books {
        let info = thumbs.get(&book.id);
        book.cover_path = info.map(|i| i.cover_path.clone());
        book.cover_version = info.map(|i| i.cover_version);
    }
}

// ── On demand ─────────────────────────────────────────────────────────────────

/// [`ensure_cover`](super::ensure_cover): the cover of book `id` for the
/// webview, making it if needed. `Ok(None)` when the book has no usable
/// cover. In order of cost:
///
/// 1. the thumbnail, if made already (one `stat`);
/// 2. a full-size cover without a thumbnail (copied by the Calibre import,
///    or one that couldn't be thumbnailed):
///    make the thumbnail, or fall back to the full cover;
/// 3. the negative cache, if the book file is unchanged;
/// 4. otherwise look inside the book (EPUB, CBZ, PDF), cache what is found
///    (full cover + thumbnail), or record that there is nothing.
///
/// Only step 4 reads the index and the book file. An image that isn't JPEG,
/// PNG or WebP (GIF…) counts as no cover. A panic while parsing a malformed
/// book is contained and counts as no cover too, so it isn't retried on
/// every launch.
pub(crate) fn ensure_cover(library: &Path, id: &str, log: Log) -> AppResult<Option<CoverInfo>> {
    check_id(id)?;
    if let Some(info) = cover_info(&thumb_path(library, id)) {
        return Ok(Some(info));
    }
    if full_cover(library, id).is_some() {
        return Ok(with_cover_lock(library, id, || {
            thumbnail_cached_cover(library, id, log)
        }));
    }
    if negative_cache_is_current(library, id) {
        return Ok(None);
    }

    let book = find_book(library, id, log)?;
    let book_path = PathBuf::from(&book.path);
    // Stamped before reading: if the file changes meanwhile, the marker won't
    // match it and the book is looked at again next time.
    let marker = NoCoverMarker::for_file(library, &book_path)
        .map_err(|e| AppError::io(format!("Could not read {}", book.filename), &e))?;
    let found = extract_cover_image(&book_path, &book.format, log);

    with_cover_lock(library, id, || {
        // A cover set or removed while the book was being read wins over
        // what was read.
        if let Some(info) = cover_info(&thumb_path(library, id)) {
            return Ok(Some(info));
        }
        if full_cover(library, id).is_some() {
            return Ok(thumbnail_cached_cover(library, id, log));
        }
        if negative_cache_is_current(library, id) {
            return Ok(None);
        }
        match found {
            Some((bytes, kind)) => {
                log(format!(
                    "[ensure_cover] {} bytes ({}) found in {}",
                    bytes.len(),
                    kind.mime(),
                    book.filename
                ));
                let thumb = make_thumbnail(&bytes);
                store_cover(library, id, &bytes, kind, thumb, log).map(Some)
            }
            None => {
                log(format!("[ensure_cover] no cover in {}", book.filename));
                write_negative_cache(library, id, &marker, log);
                Ok(None)
            }
        }
    })
}

/// Step 2 of [`ensure_cover`], with the cover lock held: thumbnail the cached
/// full cover, or serve it as is when it can't be decoded (a GIF saved as
/// `.jpg`, a damaged file — the webview may still manage).
fn thumbnail_cached_cover(library: &Path, id: &str, log: Log) -> Option<CoverInfo> {
    if let Some(info) = cover_info(&thumb_path(library, id)) {
        return Some(info); // made by another call meanwhile
    }
    let full = full_cover(library, id)?;
    let made = fs::read(&full)
        .map_err(|e| e.to_string())
        .and_then(|bytes| make_thumbnail(&bytes))
        .and_then(|jpeg| write_thumbnail(library, id, &jpeg).map_err(|e| e.to_string()));
    match made {
        Ok(info) => Some(info),
        Err(e) => {
            log(format!(
                "[ensure_cover] no thumbnail for {id} ({e}); the full-size cover is used"
            ));
            cover_info(&full)
        }
    }
}

// ── Extraction from book files ────────────────────────────────────────────────

/// The cover image inside the book file, if there is a usable one (JPEG,
/// PNG or WebP). Formats without cover extraction (MOBI, AZW3) give `None`.
fn extract_cover_image(path: &Path, format: &BookFormat, log: Log) -> Option<(Vec<u8>, ImageKind)> {
    let extracted = catch_unwind(AssertUnwindSafe(|| match format {
        BookFormat::Epub => extract_epub_cover(path),
        BookFormat::Cbz => extract_cbz_cover(path),
        BookFormat::Pdf => extract_pdf_cover(path),
        BookFormat::Mobi | BookFormat::Azw3 | BookFormat::Unknown => None,
    }));
    let bytes = match extracted {
        Ok(bytes) => bytes?,
        Err(_) => {
            log(format!(
                "[ensure_cover] WARNING — reading the cover of {} crashed; treated as no cover",
                path.display()
            ));
            return None;
        }
    };
    match ImageKind::sniff(&bytes) {
        Some(kind) => Some((bytes, kind)),
        None => {
            log(format!(
                "[ensure_cover] the cover of {} is not a JPEG, PNG or WebP image; ignored",
                path.display()
            ));
            None
        }
    }
}

/// Read a ZIP entry, refusing one larger than [`MAX_EXTRACTED_BYTES`]
/// (checked on the declared size, then while reading, since the header can lie).
fn read_zip_entry<R: Read + io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
) -> Option<Vec<u8>> {
    let entry = archive.by_name(name).ok()?;
    if entry.size() > MAX_EXTRACTED_BYTES {
        return None;
    }
    let mut data = Vec::new();
    entry
        .take(MAX_EXTRACTED_BYTES + 1)
        .read_to_end(&mut data)
        .ok()?;
    if data.is_empty() || data.len() as u64 > MAX_EXTRACTED_BYTES {
        None
    } else {
        Some(data)
    }
}

/// The cover image bytes of an EPUB: the cover its OPF declares, else the
/// best guess (an image named `cover`, the guide…), read by the OPF parser
/// ([`crate::epub::read_cover_image`]; container.xml → OPF → image entry,
/// size-capped).
pub(crate) fn extract_epub_cover(path: &Path) -> Option<Vec<u8>> {
    crate::epub::read_cover_image(path).map(|(bytes, _media_type)| bytes)
}

/// The first page image of a CBZ (by name). macOS metadata (`__MACOSX/`,
/// `._page.jpg`) and other hidden entries are skipped: they sort before the
/// real pages and aren't images.
fn extract_cbz_cover(path: &Path) -> Option<Vec<u8>> {
    let file = fs::File::open(path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;

    let first = archive
        .file_names()
        // Names that don't decode can't be looked up by name anyway.
        .filter_map(Result::ok)
        .filter(|name| {
            let lower = name.to_lowercase();
            let hidden = lower
                .split('/')
                .any(|part| part.starts_with('.') || part == "__macosx");
            let image = [".jpg", ".jpeg", ".png", ".webp"]
                .iter()
                .any(|ext| lower.ends_with(ext));
            image && !hidden
        })
        .min()?
        .to_string();
    read_zip_entry(&mut archive, &first)
}

/// Serialises PDF parsing: `lopdf` loads the whole document into memory, and
/// with four covers requested at a time, four big scanned PDFs at once would
/// cost gigabytes.
static PDF_PARSING: Mutex<()> = Mutex::new(());

/// Extract the largest JPEG image from the first page of a PDF.
/// Only handles DCTDecode (JPEG) image XObjects — raw/compressed pixel data
/// is skipped. Returns None if no qualifying image is found.
fn extract_pdf_cover(path: &Path) -> Option<Vec<u8>> {
    use lopdf::{Document, Object};

    let _one_at_a_time = PDF_PARSING.lock().unwrap_or_else(|e| e.into_inner());
    let doc = Document::load(path).ok()?;
    let pages = doc.get_pages();
    let page_id = *pages.get(&1)?;

    // Collect XObject reference IDs in a owned vec so we can re-borrow doc later.
    let xobj_ids: Vec<_> = {
        let page_dict = doc.get_object(page_id).ok()?.as_dict().ok()?;

        let resources_dict = {
            let res = page_dict.get(b"Resources").ok()?;
            match res {
                Object::Reference(id) => doc.get_object(*id).ok()?.as_dict().ok()?,
                _ => res.as_dict().ok()?,
            }
        };

        let xobj_dict = {
            let xobj = resources_dict.get(b"XObject").ok()?;
            match xobj {
                Object::Reference(id) => doc.get_object(*id).ok()?.as_dict().ok()?,
                _ => xobj.as_dict().ok()?,
            }
        };

        xobj_dict
            .iter()
            .filter_map(|(_, v)| {
                if let Object::Reference(id) = v {
                    Some(*id)
                } else {
                    None
                }
            })
            .collect()
    }; // all borrows from doc released here

    let mut best: Option<(u64, &[u8])> = None;

    for oid in xobj_ids {
        let Ok(stream) = doc.get_object(oid).and_then(Object::as_stream) else {
            continue;
        };

        // Must be an Image XObject.
        if stream
            .dict
            .get(b"Subtype")
            .ok()
            .and_then(|o| o.as_name().ok())
            != Some(b"Image".as_slice())
        {
            continue;
        }

        // Only handle single DCTDecode filter (JPEG) — content bytes are the JPEG file.
        let is_jpeg = match stream.dict.get(b"Filter") {
            Ok(Object::Name(n)) => n == b"DCTDecode",
            _ => false,
        };
        if !is_jpeg
            || stream.content.is_empty()
            || stream.content.len() as u64 > MAX_EXTRACTED_BYTES
        {
            continue;
        }

        let width = stream
            .dict
            .get(b"Width")
            .ok()
            .and_then(|o| o.as_i64().ok())
            .unwrap_or(0)
            .max(0) as u64;
        let height = stream
            .dict
            .get(b"Height")
            .ok()
            .and_then(|o| o.as_i64().ok())
            .unwrap_or(0)
            .max(0) as u64;
        let area = width.saturating_mul(height);

        if best.is_none_or(|(a, _)| area > a) {
            best = Some((area, stream.content.as_slice()));
        }
    }

    best.map(|(_, bytes)| bytes.to_vec())
}

// ── Setting and removing covers ───────────────────────────────────────────────

/// Save `bytes` as book `id`'s cover (set by the user from a file or a URL):
/// validated by content (JPEG/PNG/WebP only) and decoded *before* anything
/// changes — an image the app can't read must neither replace the cover nor
/// go into the book. Then cached (full size + thumbnail) and embedded into
/// the EPUB with the right media type.
///
/// Embedding is best effort: if it fails, the cover is still saved for the
/// app (and logged); the book file keeps its previous cover.
pub(crate) fn persist_cover(lib: &Path, id: &str, bytes: &[u8], log: Log) -> AppResult<CoverInfo> {
    check_id(id)?;
    let kind = ImageKind::sniff(bytes).ok_or_else(|| {
        AppError::InvalidInput(
            "This is not a JPEG, PNG or WebP image; the cover was not changed.".to_string(),
        )
    })?;
    let book = find_book(lib, id, log)?;
    let thumb = make_thumbnail(bytes).map_err(|e| {
        AppError::InvalidInput(format!(
            "The image could not be read ({e}); the cover was not changed."
        ))
    })?;

    let info = with_cover_lock(lib, id, || {
        store_cover(lib, id, bytes, kind, Ok(thumb), log)
    })?;
    log(format!(
        "[persist_cover] cover of {} saved ({} bytes, {})",
        id,
        bytes.len(),
        kind.mime()
    ));

    if book.format == BookFormat::Epub {
        let path = PathBuf::from(&book.path);
        let embedded = ensure_in_library(lib, &path)
            .and_then(|()| Ok(epub_edit::embed_cover(&path, bytes, kind)?));
        match embedded {
            Ok(()) => log(format!(
                "[persist_cover] cover embedded into EPUB: {}",
                book.path
            )),
            Err(e) => log(format!(
                "[persist_cover] WARNING — cover not embedded into the EPUB: {}",
                e
            )),
        }
    }
    Ok(info)
}

/// Refuse a downloaded "cover" that is not a JPEG, PNG or WebP image — Open
/// Library's placeholder GIF, an HTML error page — before anything changes.
pub(crate) fn check_downloaded_cover(bytes: &[u8]) -> AppResult<()> {
    if ImageKind::sniff(bytes).is_none() {
        return Err(AppError::Network(format!(
            "The URL did not return a JPEG, PNG or WebP image ({} bytes, possibly a placeholder or an error page); \
             the cover was not changed.",
            bytes.len()
        )));
    }
    Ok(())
}

/// Decode a base64 image (optionally a `data:…;base64,` URL), refusing
/// anything over [`MAX_COVER_BYTES`].
pub(crate) fn decode_image_base64(input: &str) -> AppResult<Vec<u8>> {
    let payload = match input.split_once(',') {
        Some((header, data)) if header.starts_with("data:") => data,
        _ => input,
    };
    // base64 is 4/3 the size of the bytes it encodes.
    if payload.len() / 4 * 3 > MAX_COVER_BYTES {
        return Err(AppError::InvalidInput(format!(
            "The image is too large (limit {} MB)",
            MAX_COVER_BYTES / (1024 * 1024)
        )));
    }
    BASE64_STANDARD
        .decode(payload.trim())
        .map_err(|e| AppError::InvalidInput(format!("Invalid image data: {}", e)))
}

/// [`delete_book_cover`](super::delete_book_cover). The EPUB is edited
/// first: if that fails the error is returned and nothing changes. Then the
/// cache is cleared (full cover, thumbnail) and a negative-cache marker is
/// written for the book file as it now is, so an image the file still holds
/// (a PDF's first page, a CBZ's first image, an undeclared EPUB cover) isn't
/// extracted again — until the file changes or the book is reset.
pub(crate) fn delete_cover(lib: &Path, id: &str, log: Log) -> AppResult<()> {
    check_id(id)?;
    log(format!("[delete_book_cover] id={}", id));
    let book = find_book(lib, id, log)?;
    let path = PathBuf::from(&book.path);
    if book.format == BookFormat::Epub {
        ensure_in_library(lib, &path)?;
        match epub_edit::strip_cover(&path) {
            Ok(true) => log(format!(
                "[delete_book_cover] stripped cover from EPUB: {}",
                book.path
            )),
            Ok(false) => log(format!(
                "[delete_book_cover] no cover declaration removed from {}",
                book.path
            )),
            Err(e) => {
                return Err(AppError::Other(format!(
                    "Could not remove the cover from {}: {}",
                    book.filename, e
                )))
            }
        }
    }
    with_cover_lock(lib, id, || {
        remove_cover_files(lib, id)
            .map_err(|e| AppError::io("Could not remove the cached cover", &e))?;
        match NoCoverMarker::for_file(lib, &path) {
            Ok(marker) => write_negative_cache(lib, id, &marker, log),
            Err(e) => log(format!(
                "[delete_book_cover] {} not stat'ed ({e}); no negative cache",
                book.filename
            )),
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::library::index;
    use crate::commands::library::test_support::{
        epub_with_cover, opf_of, scan, write, write_epub,
    };
    use crate::logging::no_log;
    use std::io::Write;
    use std::time::{Duration, SystemTime};

    // ── Fixtures ──────────────────────────────────────────────────────────

    /// A `w` × `h` gradient (not a flat colour, so encoders can't cheat).
    fn rgb(w: u32, h: u32) -> image::RgbImage {
        image::RgbImage::from_fn(w, h, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, 128])
        })
    }

    fn encode(img: &DynamicImage, format: ImageFormat) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        img.write_to(&mut out, format).unwrap();
        out.into_inner()
    }

    fn jpeg(w: u32, h: u32) -> Vec<u8> {
        encode(&DynamicImage::ImageRgb8(rgb(w, h)), ImageFormat::Jpeg)
    }

    fn png(w: u32, h: u32) -> Vec<u8> {
        encode(&DynamicImage::ImageRgb8(rgb(w, h)), ImageFormat::Png)
    }

    fn webp(w: u32, h: u32) -> Vec<u8> {
        encode(&DynamicImage::ImageRgb8(rgb(w, h)), ImageFormat::WebP)
    }

    fn dimensions(jpeg: &[u8]) -> (u32, u32) {
        assert_eq!(
            ImageKind::sniff(jpeg),
            Some(ImageKind::Jpeg),
            "thumbnails are JPEG"
        );
        let img = image::load_from_memory_with_format(jpeg, ImageFormat::Jpeg).unwrap();
        (img.width(), img.height())
    }

    /// A CBZ holding `entries` (name, bytes).
    fn write_cbz(path: &Path, entries: &[(&str, &[u8])]) {
        let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
        for (name, bytes) in entries {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
    }

    /// A one-page PDF whose page shows `jpeg` as a DCTDecode image XObject.
    fn write_pdf_with_image(path: &Path, jpeg: &[u8], w: i64, h: i64) {
        use lopdf::{dictionary, Document, Object, Stream};
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let image_id = doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Image", "Width" => w, "Height" => h,
                "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8, "Filter" => "DCTDecode",
            },
            jpeg.to_vec(),
        ));
        let content_id = doc.add_object(Stream::new(
            dictionary! {},
            b"q 100 0 0 150 0 0 cm /Im0 Do Q".to_vec(),
        ));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "MediaBox" => vec![0.into(), 0.into(), 100.into(), 150.into()],
            "Resources" => dictionary! { "XObject" => dictionary! { "Im0" => image_id } },
        });
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1,
            }),
        );
        let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog_id);
        doc.save(path).unwrap();
    }

    /// Give `path` a different modification time, as an edit in another app would.
    fn touch(path: &Path, seconds_later: u64) {
        let file = fs::File::options().write(true).open(path).unwrap();
        file.set_modified(SystemTime::now() + Duration::from_secs(seconds_later))
            .unwrap();
    }

    fn id_of(lib: &Path, name: &str) -> String {
        scan(lib).remove(name).unwrap().id
    }

    // ── Thumbnails ────────────────────────────────────────────────────────

    #[test]
    fn thumbnails_fit_the_box_and_keep_the_aspect_ratio() {
        assert_eq!(
            dimensions(&make_thumbnail(&jpeg(1200, 1800)).unwrap()),
            (400, 600)
        );
        assert_eq!(
            dimensions(&make_thumbnail(&png(1800, 1200)).unwrap()),
            (600, 400),
            "landscape"
        );
        assert_eq!(
            dimensions(&make_thumbnail(&webp(800, 1000)).unwrap()),
            (480, 600)
        );
        assert_eq!(
            dimensions(&make_thumbnail(&png(300, 450)).unwrap()),
            (300, 450),
            "never enlarged"
        );
    }

    #[test]
    fn transparency_is_flattened_onto_white() {
        let clear = DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            10,
            10,
            image::Rgba([0, 0, 0, 0]),
        ));
        let thumb = make_thumbnail(&encode(&clear, ImageFormat::Png)).unwrap();
        let px = image::load_from_memory(&thumb)
            .unwrap()
            .to_rgb8()
            .get_pixel(5, 5)
            .0;
        assert!(px.iter().all(|&c| c > 245), "white, not black: {px:?}");
    }

    #[test]
    fn unreadable_images_have_no_thumbnail() {
        assert!(
            make_thumbnail(b"GIF89a\x01\0\x01\0\x80\0\0").is_err(),
            "GIF is not supported"
        );
        assert!(make_thumbnail(b"<html>404</html>").is_err());
        let mut truncated = jpeg(400, 600);
        truncated.truncate(200);
        assert!(make_thumbnail(&truncated).is_err(), "truncated JPEG");
        let mut header_only = png(10, 10);
        header_only.truncate(16);
        assert!(
            make_thumbnail(&header_only).is_err(),
            "PNG signature without data"
        );
    }

    #[test]
    fn oversized_images_are_refused_before_decoding() {
        // A tiny file can declare a huge image (decompression bomb): the
        // dimension limit refuses it from the header alone.
        assert!(make_thumbnail(&png(16_385, 1)).is_err());
        assert_eq!(
            dimensions(&make_thumbnail(&png(16_000, 1)).unwrap()).0,
            600,
            "at the limit: fine"
        );
    }

    // ── ensure_cover ──────────────────────────────────────────────────────

    #[test]
    fn epub_covers_are_extracted_cached_and_thumbnailed() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        let cover = jpeg(900, 1350);
        epub_with_cover(&lib.join("dune.epub"), &cover);
        let id = id_of(lib, "dune.epub");

        let info = ensure_cover(lib, &id, &no_log).unwrap().expect("a cover");
        assert_eq!(PathBuf::from(&info.cover_path), thumb_path(lib, &id));
        assert!(info.cover_version > 0);
        assert_eq!(dimensions(&fs::read(&info.cover_path).unwrap()), (400, 600));
        assert_eq!(
            fs::read(cover_cache_path(lib, &id, ImageKind::Jpeg)).unwrap(),
            cover,
            "full size kept for OPDS"
        );

        // Served from the thumbnail from now on: no index, no book needed.
        fs::remove_file(index::index_path(lib)).unwrap();
        fs::remove_file(lib.join("dune.epub")).unwrap();
        assert_eq!(ensure_cover(lib, &id, &no_log).unwrap(), Some(info));
    }

    #[test]
    fn cbz_covers_are_the_first_real_page() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        let page1 = png(600, 900);
        write_cbz(
            &lib.join("comic.cbz"),
            &[
                ("__MACOSX/._001.png", b"AppleDouble junk"),
                ("pages/002.png", &png(10, 10)),
                ("pages/001.png", &page1),
                ("notes.txt", b"not an image"),
            ],
        );
        let id = id_of(lib, "comic.cbz");

        let info = ensure_cover(lib, &id, &no_log).unwrap().expect("a cover");
        assert_eq!(dimensions(&fs::read(&info.cover_path).unwrap()), (400, 600));
        assert_eq!(
            fs::read(cover_cache_path(lib, &id, ImageKind::Png)).unwrap(),
            page1
        );
    }

    #[test]
    fn pdf_covers_are_the_largest_jpeg_of_the_first_page() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        let cover = jpeg(200, 300);
        write_pdf_with_image(&lib.join("scan.pdf"), &cover, 200, 300);
        let id = id_of(lib, "scan.pdf");

        let info = ensure_cover(lib, &id, &no_log).unwrap().expect("a cover");
        assert_eq!(dimensions(&fs::read(&info.cover_path).unwrap()), (200, 300));
        assert_eq!(
            fs::read(cover_cache_path(lib, &id, ImageKind::Jpeg)).unwrap(),
            cover
        );
    }

    #[test]
    fn coverless_books_are_cached_until_the_file_changes() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        let book = lib.join("plain.epub");
        write_epub(&book, "Plain", "Nobody");
        let id = id_of(lib, "plain.epub");

        assert_eq!(ensure_cover(lib, &id, &no_log).unwrap(), None);
        assert!(marker_path(lib, &id).is_file(), "negative cache written");
        // While the file is unchanged the answer comes from the marker alone:
        // even without the index.
        let index_file = index::index_path(lib);
        let saved_index = fs::read(&index_file).unwrap();
        fs::remove_file(&index_file).unwrap();
        assert_eq!(ensure_cover(lib, &id, &no_log).unwrap(), None);
        fs::write(&index_file, saved_index).unwrap();

        // Another app adds a cover: the file changes, the marker is stale.
        epub_with_cover(&book, &jpeg(60, 90));
        touch(&book, 5);
        let info = ensure_cover(lib, &id, &no_log)
            .unwrap()
            .expect("the new cover is found");
        assert_eq!(dimensions(&fs::read(&info.cover_path).unwrap()), (60, 90));
        assert!(
            !marker_path(lib, &id).exists(),
            "marker cleared once a cover exists"
        );
    }

    #[test]
    fn concurrent_requests_for_one_book_agree() {
        // The card and the detail panel can ask at the same time.
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path().to_path_buf();
        epub_with_cover(&lib.join("dune.epub"), &jpeg(300, 450));
        let id = id_of(&lib, "dune.epub");
        let results: Vec<_> = (0..6)
            .map(|_| {
                let (lib, id) = (lib.clone(), id.clone());
                std::thread::spawn(move || {
                    ensure_cover(&lib, &id, &no_log)
                        .unwrap()
                        .unwrap()
                        .cover_path
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect();
        assert!(
            results
                .iter()
                .all(|p| Path::new(p) == thumb_path(&lib, &id)),
            "{results:?}"
        );
        assert_eq!(
            dimensions(&fs::read(thumb_path(&lib, &id)).unwrap()),
            (300, 450)
        );
    }

    #[test]
    fn formats_without_extraction_and_unsupported_images_count_as_no_cover() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        let mut mobi = vec![0u8; 80];
        mobi[60..68].copy_from_slice(b"BOOKMOBI");
        write(&lib.join("old.mobi"), &mobi);
        epub_with_cover(&lib.join("gif.epub"), b"GIF89a\x01\0\x01\0\x80\0\0");
        let books = scan(lib);

        for name in ["old.mobi", "gif.epub"] {
            let id = &books[name].id;
            assert_eq!(ensure_cover(lib, id, &no_log).unwrap(), None, "{name}");
            assert!(marker_path(lib, id).is_file(), "{name}: negative cache");
            assert!(full_cover(lib, id).is_none(), "{name}: nothing cached");
        }
    }

    #[test]
    fn cached_full_covers_get_their_thumbnail_or_are_served_as_is() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        // Full-size covers without their thumbnails (as the Calibre import leaves them).
        write(
            &cover_cache_path(lib, "calibre-1", ImageKind::Jpeg),
            &jpeg(1000, 1500),
        );
        let info = ensure_cover(lib, "calibre-1", &no_log).unwrap().unwrap();
        assert_eq!(
            PathBuf::from(&info.cover_path),
            thumb_path(lib, "calibre-1")
        );

        // Older versions cached GIFs as `.jpg`: no thumbnail, the file itself is served.
        let gif = cover_cache_path(lib, "old-gif", ImageKind::Jpeg);
        write(&gif, b"GIF89a\x01\0\x01\0\x80\0\0");
        let info = ensure_cover(lib, "old-gif", &no_log).unwrap().unwrap();
        assert_eq!(PathBuf::from(&info.cover_path), gif);
    }

    #[test]
    fn ids_are_validated() {
        let dir = tempfile::tempdir().unwrap();
        for bad in ["../../etc/passwd", "", "a/b", "x.none", &"a".repeat(65)] {
            assert!(
                ensure_cover(dir.path(), bad, &no_log)
                    .unwrap_err()
                    .to_string()
                    .contains("Invalid book id"),
                "{bad:?}"
            );
            assert!(persist_cover(dir.path(), bad, &jpeg(10, 10), &no_log).is_err());
            assert!(delete_cover(dir.path(), bad, &no_log).is_err());
        }
        assert!(is_safe_id("0b6c1a9e-5f0d-4c1e-9a43-3f1f3c0f9b11"));
        // Unknown (but well-formed) ids: no cache, not in the index.
        assert!(ensure_cover(dir.path(), "nope", &no_log)
            .unwrap_err()
            .to_string()
            .contains("not in the library"));
    }

    #[test]
    fn markers_cannot_point_outside_the_library() {
        let lib = Path::new("/lib");
        let marker = |path: &str| NoCoverMarker {
            path: path.into(),
            size: 0,
            mtime_ms: 0,
        };
        assert_eq!(
            marker("a/b.epub").book_path(lib),
            Some(PathBuf::from("/lib/a/b.epub"))
        );
        assert_eq!(marker("../x.epub").book_path(lib), None);
        assert_eq!(marker("a/../../x.epub").book_path(lib), None);
        assert_eq!(marker("a//b.epub").book_path(lib), None);
    }

    // ── Scans ─────────────────────────────────────────────────────────────

    #[test]
    fn scans_attach_existing_thumbnails_only() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        epub_with_cover(&lib.join("a.epub"), &jpeg(100, 150));
        write_epub(&lib.join("b.epub"), "B", "X");
        let mut books: Vec<Book> = scan(lib).into_values().collect();
        books.sort_by(|x, y| x.filename.cmp(&y.filename));
        ensure_cover(lib, &books[0].id, &no_log).unwrap();
        // A full cover without a thumbnail is left to ensure_cover.
        write(
            &cover_cache_path(lib, &books[1].id, ImageKind::Jpeg),
            &jpeg(10, 10),
        );
        books[1].cover_path = Some("/stale/value/from/an/old/index.jpg".into());

        attach_cached_covers(lib, &mut books);
        assert_eq!(
            books[0].cover_path.as_deref().map(PathBuf::from),
            Some(thumb_path(lib, &books[0].id))
        );
        assert!(books[0].cover_version.unwrap() > 0);
        assert_eq!(
            (books[1].cover_path.as_deref(), books[1].cover_version),
            (None, None)
        );
    }

    // ── Setting and removing ──────────────────────────────────────────────

    #[test]
    fn covers_are_validated_cached_thumbnailed_and_embedded() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        epub_with_cover(&lib.join("dune.epub"), &jpeg(100, 150));
        let id = id_of(lib, "dune.epub");
        ensure_cover(lib, &id, &no_log).unwrap();
        let old_thumb = fs::read(thumb_path(lib, &id)).unwrap();

        let err = persist_cover(lib, &id, b"GIF89a\x01\x00\x01\x00", &no_log)
            .unwrap_err()
            .to_string();
        assert!(err.contains("not a JPEG, PNG or WebP"), "{err}");
        assert!(persist_cover(lib, &id, b"<html>404</html>", &no_log).is_err());
        let mut broken = png(100, 100);
        broken.truncate(40);
        let err = persist_cover(lib, &id, &broken, &no_log)
            .unwrap_err()
            .to_string();
        assert!(err.contains("could not be read"), "{err}");
        assert_eq!(
            fs::read(thumb_path(lib, &id)).unwrap(),
            old_thumb,
            "a rejected image changes nothing"
        );

        let new_cover = png(1200, 1800);
        let info = persist_cover(lib, &id, &new_cover, &no_log).unwrap();
        assert_eq!(PathBuf::from(&info.cover_path), thumb_path(lib, &id));
        assert_eq!(dimensions(&fs::read(&info.cover_path).unwrap()), (400, 600));
        assert_eq!(
            fs::read(cover_cache_path(lib, &id, ImageKind::Png)).unwrap(),
            new_cover
        );
        assert!(
            !cover_cache_path(lib, &id, ImageKind::Jpeg).exists(),
            "stale cover removed"
        );
        assert_eq!(
            extract_epub_cover(&lib.join("dune.epub")).unwrap(),
            new_cover
        );
        assert!(opf_of(&lib.join("dune.epub")).contains("media-type=\"image/png\""));
    }

    #[test]
    fn setting_a_cover_clears_the_negative_cache() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        write_cbz(&lib.join("comic.cbz"), &[("readme.txt", b"no pages")]);
        let id = id_of(lib, "comic.cbz");
        assert_eq!(ensure_cover(lib, &id, &no_log).unwrap(), None);

        let info = persist_cover(lib, &id, &webp(300, 450), &no_log).unwrap();
        assert!(!marker_path(lib, &id).exists());
        assert_eq!(ensure_cover(lib, &id, &no_log).unwrap(), Some(info));
    }

    #[test]
    fn deleted_covers_stay_deleted_until_the_book_is_reset() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        write_cbz(&lib.join("comic.cbz"), &[("001.jpg", &jpeg(100, 150))]);
        let id = id_of(lib, "comic.cbz");
        ensure_cover(lib, &id, &no_log).unwrap().unwrap();

        // The CBZ keeps its first page, but the user said "no cover".
        delete_cover(lib, &id, &no_log).unwrap();
        assert!(full_cover(lib, &id).is_none());
        assert!(!thumb_path(lib, &id).exists());
        assert_eq!(ensure_cover(lib, &id, &no_log).unwrap(), None);

        // Reset forgets everything: the page is the cover again.
        remove_cached_covers(lib, &id).unwrap();
        assert!(!marker_path(lib, &id).exists());
        assert!(ensure_cover(lib, &id, &no_log).unwrap().is_some());
    }

    #[test]
    fn deleting_an_epub_cover_strips_it_from_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        epub_with_cover(&lib.join("dune.epub"), &jpeg(100, 150));
        let id = id_of(lib, "dune.epub");
        ensure_cover(lib, &id, &no_log).unwrap().unwrap();

        delete_cover(lib, &id, &no_log).unwrap();
        assert!(extract_epub_cover(&lib.join("dune.epub")).is_none());
        assert!(full_cover(lib, &id).is_none() && !thumb_path(lib, &id).exists());
        assert_eq!(ensure_cover(lib, &id, &no_log).unwrap(), None);
    }

    #[test]
    fn base64_covers_accept_data_urls() {
        assert_eq!(
            decode_image_base64("data:image/png;base64,iVBORw==").unwrap(),
            b"\x89PNG"
        );
        assert_eq!(decode_image_base64("iVBORw==").unwrap(), b"\x89PNG");
        assert!(decode_image_base64("not base64!").is_err());
    }
}
