//! Book downloads, covers and thumbnails: files streamed from disk.
//!
//! Files are served by tower-http's [`ServeFile`]: read in 64 KiB chunks as
//! the client takes them (a 300 MB PDF never sits in memory), with
//! `Content-Length`, `Accept-Ranges: bytes`, single `Range` requests
//! (`206 Partial Content`, `416` when unsatisfiable; multi-range requests are
//! answered with `416` too), `Last-Modified` / `If-Modified-Since`, and
//! `HEAD`. The `Content-Type` is ours, not guessed from the file name.

use std::fmt::Write as _;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::extract::Request;
use axum::response::Response;
use tower::ServiceExt;
use tower_http::services::ServeFile;

use crate::models::book::BookFormat;

/// The MIME type of a book format, as reading apps expect it (KOReader picks
/// the file extension of a download from it).
pub(super) fn format_mime(format: &BookFormat) -> &'static str {
    match format {
        BookFormat::Epub => "application/epub+zip",
        BookFormat::Pdf => "application/pdf",
        BookFormat::Mobi => "application/x-mobipocket-ebook",
        // KF8. (`application/vnd.amazon.ebook` is the older AZW.)
        BookFormat::Azw3 => "application/vnd.amazon.mobi8-ebook",
        BookFormat::Cbz => "application/vnd.comicbook+zip",
        BookFormat::Unknown => "application/octet-stream",
    }
}

/// A short name for a format, shown on download buttons.
pub(super) fn format_label(format: &BookFormat) -> &'static str {
    match format {
        BookFormat::Epub => "EPUB",
        BookFormat::Pdf => "PDF",
        BookFormat::Mobi => "MOBI",
        BookFormat::Azw3 => "AZW3",
        BookFormat::Cbz => "CBZ",
        BookFormat::Unknown => "File",
    }
}

/// Image type from the file's magic bytes, falling back to its extension
/// (cover files written by old versions weren't always named after their
/// real format).
pub(super) fn image_mime(bytes: &[u8], path: &Path) -> &'static str {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else if bytes.starts_with(b"GIF8") {
        "image/gif"
    } else {
        image_mime_from_name(path)
    }
}

/// Image type from the extension alone: what the feeds announce (sniffing
/// every cover of a feed page would mean reading them all).
pub(super) fn image_mime_from_name(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        _ => "image/jpeg",
    }
}

/// The type of the image at `path`, from its first bytes. Blocking.
pub(super) fn sniff_image_file(path: &Path) -> &'static str {
    let mut head = [0u8; 16];
    let read = std::fs::File::open(path)
        .and_then(|mut f| f.read(&mut head))
        .unwrap_or(0);
    image_mime(&head[..read], path)
}

/// `Content-Disposition` for a download (RFC 6266): an ASCII-only `filename`
/// for old clients plus the exact UTF-8 name in `filename*` (RFC 5987).
pub(super) fn content_disposition(filename: &str) -> String {
    let fallback: String = filename
        .chars()
        .map(|c| {
            if (c.is_ascii_graphic() || c == ' ') && !matches!(c, '"' | '\\' | '%') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let mut encoded = String::with_capacity(filename.len());
    for b in filename.bytes() {
        // RFC 5987 attr-char; everything else is percent-encoded.
        if b.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&b) {
            encoded.push(b as char);
        } else {
            let _ = write!(encoded, "%{b:02X}");
        }
    }
    format!("attachment; filename=\"{fallback}\"; filename*=UTF-8''{encoded}")
}

/// Answer `request` with the file at `path`, typed `content_type` (see the
/// module docs). The caller has checked that the file may be served.
pub(super) async fn serve_file(
    request: Request,
    path: PathBuf,
    content_type: &'static str,
) -> Response {
    // The types used here are constants, so this only falls back for a typo.
    let mime = content_type
        .parse()
        .unwrap_or(mime::APPLICATION_OCTET_STREAM);
    match ServeFile::new_with_mime(path, &mime).oneshot(request).await {
        Ok(response) => response.map(Body::new),
        Err(never) => match never {},
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{header, HeaderValue, StatusCode};

    #[test]
    fn content_disposition_is_rfc6266() {
        assert_eq!(
            content_disposition("Café: été.epub"),
            "attachment; filename=\"Caf_: _t_.epub\"; filename*=UTF-8''Caf%C3%A9%3A%20%C3%A9t%C3%A9.epub"
        );
        assert_eq!(
            content_disposition("a\"b\\c%d\n.pdf"),
            "attachment; filename=\"a_b_c_d_.pdf\"; filename*=UTF-8''a%22b%5Cc%25d%0A.pdf"
        );
        assert!(
            HeaderValue::from_str(&content_disposition("日本語.epub")).is_ok(),
            "always a valid header"
        );
    }

    #[test]
    fn image_types_are_sniffed() {
        let p = Path::new("x.jpg");
        assert_eq!(image_mime(b"\x89PNG\r\n\x1a\n...", p), "image/png");
        assert_eq!(
            image_mime(&[0xFF, 0xD8, 0xFF, 0xE0], Path::new("x.png")),
            "image/jpeg"
        );
        assert_eq!(image_mime(b"RIFF\0\0\0\0WEBPVP8 ", p), "image/webp");
        assert_eq!(image_mime(b"????", Path::new("x.png")), "image/png");
        assert_eq!(image_mime(b"????", p), "image/jpeg");
        assert_eq!(
            sniff_image_file(Path::new("/does/not/exist.webp")),
            "image/webp"
        );
    }

    #[test]
    fn every_format_has_a_type() {
        for (format, mime) in [
            (BookFormat::Epub, "application/epub+zip"),
            (BookFormat::Pdf, "application/pdf"),
            (BookFormat::Mobi, "application/x-mobipocket-ebook"),
            (BookFormat::Azw3, "application/vnd.amazon.mobi8-ebook"),
            (BookFormat::Cbz, "application/vnd.comicbook+zip"),
            (BookFormat::Unknown, "application/octet-stream"),
        ] {
            assert_eq!(format_mime(&format), mime);
            assert!(mime.parse::<mime::Mime>().is_ok());
        }
    }

    /// Serve a 10-byte file; returns status, headers and the whole body.
    async fn serve(range: Option<&str>, method: &str) -> (StatusCode, header::HeaderMap, Vec<u8>) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("book.pdf");
        std::fs::write(&path, b"0123456789").unwrap();
        let mut req = Request::builder().method(method).uri("/x");
        if let Some(r) = range {
            req = req.header(header::RANGE, r);
        }
        let resp = serve_file(req.body(Body::empty()).unwrap(), path, "application/pdf").await;
        let (parts, body) = resp.into_parts();
        let bytes = axum::body::to_bytes(body, usize::MAX)
            .await
            .unwrap()
            .to_vec();
        (parts.status, parts.headers, bytes)
    }

    #[tokio::test]
    async fn whole_files_are_streamed_with_their_length() {
        let (status, headers, body) = serve(None, "GET").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers[header::CONTENT_TYPE], "application/pdf");
        assert_eq!(headers[header::CONTENT_LENGTH], "10");
        assert_eq!(headers[header::ACCEPT_RANGES], "bytes");
        assert!(headers.contains_key(header::LAST_MODIFIED));
        assert_eq!(body, b"0123456789");
    }

    #[tokio::test]
    async fn ranges_are_served_partially() {
        let (status, headers, body) = serve(Some("bytes=2-4"), "GET").await;
        assert_eq!(status, StatusCode::PARTIAL_CONTENT);
        assert_eq!(headers[header::CONTENT_RANGE], "bytes 2-4/10");
        assert_eq!(headers[header::CONTENT_LENGTH], "3");
        assert_eq!(body, b"234");

        // Open-ended and suffix ranges (how download managers resume).
        assert_eq!(serve(Some("bytes=7-"), "GET").await.2, b"789");
        assert_eq!(serve(Some("bytes=-2"), "GET").await.2, b"89");

        let (status, headers, _) = serve(Some("bytes=50-60"), "GET").await;
        assert_eq!(status, StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(headers[header::CONTENT_RANGE], "bytes */10");
    }

    #[tokio::test]
    async fn head_sends_headers_only() {
        let (status, headers, body) = serve(None, "HEAD").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers[header::CONTENT_LENGTH], "10");
        assert!(body.is_empty());
    }
}
