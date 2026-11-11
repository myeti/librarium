//! EPUB files, read and edited with a real XML parser (quick-xml).
//!
//! An EPUB is a ZIP archive. `META-INF/container.xml` names the *package
//! document* (the OPF), which describes the book: its metadata (Dublin Core
//! elements and `<meta>` entries), its manifest (every file, with its media
//! type), the reading order (spine) and, in EPUB 2, a guide.
//!
//! | Module        | Responsibility |
//! |---------------|----------------|
//! | [`archive`]   | open an EPUB once, locate and parse its OPF, read entries with size caps |
//! | [`container`] | `container.xml` → OPF path; hrefs → archive paths |
//! | [`opf`]       | parse an OPF into a [`Package`], keeping each element's byte positions |
//! | [`metadata`]  | what the entries mean: title, authors, ISBN, series… → [`EpubMetadata`] |
//! | [`edit`]      | write metadata back, touching only the managed elements |
//! | [`cover`]     | find the cover; set, replace or remove the declared one |
//! | [`identity`]  | Librarium's `urn:librarium:` identifier in copies sent to e-readers |
//! | [`html`]      | plain text from HTML descriptions ([`strip_html`]) |
//! | [`isbn`]      | telling ISBNs apart from other identifiers |
//! | [`xml`]       | decoding, entities, escaping, start tags, splicing |
//!
//! Two rules keep the user's books safe:
//! - **Edits are surgical.** The OPF is never re-serialised: new bytes are
//!   spliced into the original text at the positions the parser recorded,
//!   so everything Librarium doesn't manage (comments, formatting, unknown
//!   elements, other `<meta>`s) stays byte-for-byte as it was. Each edited
//!   OPF is parsed again and checked — well-formed, the package's unique
//!   identifier unchanged, nothing changed outside the edited section, the
//!   new values read back — before it can be written.
//! - **Reading may guess, writing may not.** Finding a cover to *show* may
//!   use heuristics; replacing or removing one only ever touches the image
//!   the OPF declares as its cover.
//!
//! This module works on bytes and text only. Writing files (locks, atomic
//! replace, ZIP rewriting) lives in `commands::library::epub_edit`.

use std::path::Path;

pub(crate) mod archive;
pub(crate) mod container;
pub(crate) mod cover;
pub(crate) mod edit;
mod html;
pub(crate) mod identity;
mod isbn;
pub(crate) mod metadata;
pub(crate) mod opf;
mod xml;

#[cfg(test)]
pub(crate) mod fixtures;

pub(crate) use archive::{EpubArchive, Locate, Opf};
pub(crate) use edit::MetadataEdit;
pub(crate) use html::strip_html;
pub(crate) use metadata::EpubMetadata;

/// The metadata of the EPUB at `path`.
pub(crate) fn read_metadata(path: &Path) -> Result<EpubMetadata, String> {
    EpubArchive::open(path).map(|epub| epub.metadata())
}

/// The cover image of the EPUB at `path` (the declared one, else the best
/// guess; see [`cover::find_cover`]), with its declared media type (may be
/// empty). `None` if it has none, it can't be read, or it is larger than
/// [`archive::MAX_IMAGE_BYTES`].
pub(crate) fn read_cover_image(path: &Path) -> Option<(Vec<u8>, String)> {
    let mut epub = EpubArchive::open(path).ok()?;
    epub.cover_image()
        .map(|(cover, bytes)| (bytes, cover.media_type))
}
