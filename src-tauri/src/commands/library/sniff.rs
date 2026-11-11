//! Recognise files by their content (magic bytes), not by their name or by
//! what a server claims.
//!
//! Downloads are the main reason: a "cover" URL can return an HTML error page
//! or Open Library's 1×1 placeholder GIF, and a "book" URL can return a login
//! page. Saving those would replace a real cover, or add a broken book.

/// Image formats accepted as book covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImageKind {
    Jpeg,
    Png,
    Webp,
}

impl ImageKind {
    /// Every accepted kind (used to find or remove cached cover files).
    pub(crate) const ALL: [ImageKind; 3] = [ImageKind::Jpeg, ImageKind::Png, ImageKind::Webp];

    /// Identify an image by its first bytes. Anything other than JPEG, PNG
    /// or WebP — GIF, SVG, HTML, empty — is `None`.
    pub(crate) fn sniff(bytes: &[u8]) -> Option<Self> {
        if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            Some(Self::Jpeg)
        } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            Some(Self::Png)
        } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
            Some(Self::Webp)
        } else {
            None
        }
    }

    /// File extension used for cached covers and embedded cover entries.
    pub(crate) fn extension(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png => "png",
            Self::Webp => "webp",
        }
    }

    /// MIME type, for data URLs and the OPF manifest.
    pub(crate) fn mime(self) -> &'static str {
        match self {
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
            Self::Webp => "image/webp",
        }
    }
}

/// How many leading bytes [`ebook_matches_extension`] needs to decide.
pub(crate) const EBOOK_SNIFF_LEN: usize = 1024;

/// Whether `head` (the first bytes of a file, ideally [`EBOOK_SNIFF_LEN`])
/// looks like an ebook of the format its extension `ext` claims:
/// - EPUB, CBZ: a ZIP archive (`PK\x03\x04`);
/// - PDF: a `%PDF-` header within the first 1024 bytes (the spec tolerates
///   leading junk);
/// - MOBI, AZW, AZW3: a Palm database with type `BOOKMOBI` at offset 60.
///
/// Unknown extensions never match.
pub(crate) fn ebook_matches_extension(ext: &str, head: &[u8]) -> bool {
    match ext.to_ascii_lowercase().as_str() {
        "epub" | "cbz" => head.starts_with(b"PK\x03\x04"),
        "pdf" => head.windows(5).take(EBOOK_SNIFF_LEN).any(|w| w == b"%PDF-"),
        "mobi" | "azw" | "azw3" => head.get(60..68) == Some(b"BOOKMOBI".as_slice()),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_accepted_images() {
        assert_eq!(
            ImageKind::sniff(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 0]),
            Some(ImageKind::Jpeg)
        );
        assert_eq!(
            ImageKind::sniff(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR"),
            Some(ImageKind::Png)
        );
        assert_eq!(
            ImageKind::sniff(b"RIFF\x24\0\0\0WEBPVP8 "),
            Some(ImageKind::Webp)
        );
    }

    #[test]
    fn rejects_placeholders_and_error_pages() {
        // Open Library's "no cover" answer is a 1×1 GIF.
        assert_eq!(ImageKind::sniff(b"GIF89a\x01\0\x01\0\x80\0\0"), None);
        assert_eq!(ImageKind::sniff(b"<!DOCTYPE html><html>"), None);
        assert_eq!(ImageKind::sniff(b"<svg xmlns="), None);
        assert_eq!(ImageKind::sniff(b""), None);
        assert_eq!(ImageKind::sniff(b"RIFF\0\0\0\0WAVE"), None);
    }

    #[test]
    fn ebook_magic_bytes() {
        assert!(ebook_matches_extension("EPUB", b"PK\x03\x04\x14\0"));
        assert!(ebook_matches_extension("cbz", b"PK\x03\x04"));
        assert!(!ebook_matches_extension("epub", b"<html>"));
        assert!(ebook_matches_extension("pdf", b"%PDF-1.7\n"));
        assert!(ebook_matches_extension("pdf", b"\xEF\xBB\xBF\n%PDF-1.4"));
        assert!(!ebook_matches_extension("pdf", b"<html>%PDF"));

        let mut mobi = vec![0u8; 80];
        mobi[60..68].copy_from_slice(b"BOOKMOBI");
        assert!(ebook_matches_extension("mobi", &mobi));
        assert!(ebook_matches_extension("azw3", &mobi));
        assert!(
            !ebook_matches_extension("azw3", &mobi[..64]),
            "too short to tell"
        );
        assert!(!ebook_matches_extension("txt", b"hello"));
    }
}
