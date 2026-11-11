//! Librarium's own identifier in the EPUB copies it sends to e-readers.
//!
//! A copy sent to a device carries the library book id as an extra
//! identifier, so the device copy can be traced back to its library book
//! even when the device manifest (`.librarium/device.json`) is lost:
//!
//! ```xml
//! <dc:identifier id="librarium-id">urn:librarium:0b6c1a9e-5f0d-4c1e-9a43-3f1f3c0f9b11</dc:identifier>
//! ```
//!
//! Only the *copy* is changed, never the library file. The identifier named
//! by `package@unique-identifier` is never touched (readers key their data
//! on it, and embedded fonts are de-obfuscated with it): this is a second
//! `dc:identifier`, which EPUB 2 and 3 both allow.
//!
//! As everywhere in this module, the OPF is edited by splicing new bytes
//! into the original text, and the result is parsed again and checked
//! before it is returned.

use super::edit::{content_replacement, insert_after, insert_into};
use super::metadata::dc_entries;
use super::opf::{Package, DC_NS};
use super::xml::{escape_text, Splicer};

/// The `id` attribute of Librarium's identifier element.
pub(crate) const ID_ATTR: &str = "librarium-id";

/// The URN prefix of Librarium's identifier value.
pub(crate) const URN_PREFIX: &str = "urn:librarium:";

/// The `urn:librarium:` value for `book_id`.
pub(crate) fn urn(book_id: &str) -> String {
    format!("{URN_PREFIX}{book_id}")
}

/// Index (in [`Package::metadata`]) of Librarium's identifier: the first
/// `dc:identifier` with `id="librarium-id"` or a `urn:librarium:` value,
/// unless it is the package's unique identifier (which is never ours to
/// change).
fn entry(pkg: &Package) -> Option<usize> {
    let unique = pkg.unique_identifier_entry();
    dc_entries(pkg, "identifier")
        .filter(|&i| Some(i) != unique)
        .find(|&i| {
            let e = &pkg.metadata[i];
            e.element.id().map(str::trim) == Some(ID_ATTR) || e.text.trim().starts_with(URN_PREFIX)
        })
}

/// The library book id the package carries (the part after
/// `urn:librarium:`), if any.
pub(crate) fn library_id(pkg: &Package) -> Option<String> {
    let text = pkg.metadata[entry(pkg)?].text.trim();
    let id = text.strip_prefix(URN_PREFIX)?.trim();
    (!id.is_empty()).then(|| id.to_string())
}

/// Make the OPF `text` (parsed as `pkg`) carry `urn:librarium:<book_id>`:
/// the value of an existing Librarium identifier is replaced (a book copied
/// back from a device into another library carries the old id), otherwise
/// a new `dc:identifier` is added after the last one. Returns `None` when
/// the file already says so.
///
/// The new element uses the file's Dublin Core prefix, and gets
/// `id="librarium-id"` unless that id is already taken by another element
/// (the `urn:librarium:` value is enough to find it again).
pub(crate) fn set_library_id(
    text: &str,
    pkg: &Package,
    book_id: &str,
) -> Result<Option<String>, String> {
    let book_id = book_id.trim();
    if book_id.is_empty() {
        return Err("No book id to write".to_string());
    }
    if library_id(pkg).as_deref() == Some(book_id) {
        return Ok(None);
    }
    let section = pkg
        .metadata_section
        .as_ref()
        .ok_or("The OPF has no <metadata> section")?;
    if section.span.is_self_closing() {
        return Err("The OPF's <metadata> section is empty".to_string());
    }
    let value = urn(book_id);
    let mut splicer = Splicer::new(text);
    match entry(pkg) {
        Some(i) => {
            let (range, with) = content_replacement(text, &pkg.metadata[i].element, &value);
            splicer.replace(range, with);
        }
        None => {
            let element = new_identifier(pkg, &value);
            let after = dc_entries(pkg, "identifier")
                .last()
                .or_else(|| pkg.metadata.len().checked_sub(1));
            match after {
                Some(i) => insert_after(&mut splicer, pkg.metadata[i].element.span, &element),
                None => insert_into(&mut splicer, section.span, &element),
            }
        }
    }
    let new_text = splicer.finish()?;
    check(text, pkg, &new_text, book_id)?;
    Ok(Some(new_text))
}

/// `<dc:identifier id="librarium-id">value</dc:identifier>`, spelled with
/// the file's Dublin Core prefix (declaring the namespace on the element if
/// the file never declares it).
fn new_identifier(pkg: &Package, value: &str) -> String {
    let (qname, declaration) = match pkg.scope.dc_prefix.as_deref() {
        Some("") => ("identifier".to_string(), String::new()),
        Some(prefix) => (format!("{prefix}:identifier"), String::new()),
        None => (
            "dc:identifier".to_string(),
            format!(" xmlns:dc=\"{DC_NS}\""),
        ),
    };
    let id = if pkg.ids.contains(ID_ATTR) {
        String::new()
    } else {
        format!(" id=\"{ID_ATTR}\"")
    };
    format!("<{qname}{declaration}{id}>{}</{qname}>", escape_text(value))
}

/// Check an edited OPF before anyone writes it: it parses, nothing outside
/// `<metadata>` changed, the unique identifier is byte-for-byte the same,
/// and the library id reads back.
fn check(old_text: &str, old: &Package, new_text: &str, book_id: &str) -> Result<(), String> {
    let fail = |what: &str| {
        Err(format!(
            "Internal check failed after adding the library id to the OPF ({what})"
        ))
    };
    let new = match Package::parse(new_text) {
        Ok(pkg) => pkg,
        Err(e) => return fail(&format!("the result doesn't parse: {e}")),
    };
    let (Some(o), Some(n)) = (&old.metadata_section, &new.metadata_section) else {
        return fail("<metadata> is missing");
    };
    if old_text[..o.span.start] != new_text[..n.span.start]
        || old_text[o.span.end..] != new_text[n.span.end..]
    {
        return fail("something outside <metadata> changed");
    }
    if let Some(u) = old.unique_identifier_entry() {
        let same = new.unique_identifier_entry().is_some_and(|v| {
            old_text[old.metadata[u].element.span.range()]
                == new_text[new.metadata[v].element.span.range()]
        });
        if !same {
            return fail("the unique identifier changed");
        }
    }
    if library_id(&new).as_deref() != Some(book_id) {
        return fail("the id doesn't read back");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::epub::fixtures::{CALIBRE_OPF, EPUB3_OPF, PREFIXED_ONE_LINE_OPF};
    use crate::epub::metadata;

    const ID: &str = "3f2b1c3e-6e0a-4c1b-9a43-3f1f3c0f9b11";

    fn set(text: &str, id: &str) -> Option<String> {
        set_library_id(text, &Package::parse(text).unwrap(), id).unwrap()
    }

    fn read(text: &str) -> Option<String> {
        library_id(&Package::parse(text).unwrap())
    }

    #[test]
    fn adds_an_identifier_after_the_last_one_and_keeps_everything_else() {
        let new = set(EPUB3_OPF, ID).unwrap();
        assert!(
            new.contains(&format!(
                "<dc:identifier id=\"pub-id\">urn:uuid:0b6c1a9e-5f0d-4c1e-9a43-3f1f3c0f9b11</dc:identifier>\n    <dc:identifier id=\"librarium-id\">urn:librarium:{ID}</dc:identifier>"
            )),
            "{new}"
        );
        assert_eq!(read(&new).as_deref(), Some(ID));
        // Nothing else moved: the rest of the metadata reads the same.
        let (before, after) = (
            Package::parse(EPUB3_OPF).unwrap(),
            Package::parse(&new).unwrap(),
        );
        assert_eq!(metadata::read(&before), metadata::read(&after));
        assert_eq!(
            new.replace(
                &format!(
                    "\n    <dc:identifier id=\"librarium-id\">urn:librarium:{ID}</dc:identifier>"
                ),
                ""
            ),
            EPUB3_OPF
        );
    }

    #[test]
    fn works_with_calibre_and_prefixed_single_line_opfs() {
        let calibre = set(CALIBRE_OPF, ID).unwrap();
        assert_eq!(read(&calibre).as_deref(), Some(ID));
        let one_line = set(PREFIXED_ONE_LINE_OPF, ID).unwrap();
        assert!(
            one_line.contains(&format!(
                "<purl:identifier id=\"librarium-id\">urn:librarium:{ID}</purl:identifier>"
            )),
            "{one_line}"
        );
        assert_eq!(read(&one_line).as_deref(), Some(ID));
    }

    #[test]
    fn an_existing_library_id_is_replaced_not_duplicated() {
        let first = set(EPUB3_OPF, "old-id").unwrap();
        let second = set(&first, ID).unwrap();
        assert_eq!(read(&second).as_deref(), Some(ID));
        assert_eq!(second.matches("urn:librarium:").count(), 1, "{second}");
        assert_eq!(set(&second, ID), None, "already carries this id: unchanged");
    }

    #[test]
    fn the_unique_identifier_is_never_used_or_changed() {
        // A package whose *unique identifier* looks like ours: not ours to
        // change, so a second identifier is added.
        let opf = EPUB3_OPF.replace(
            "urn:uuid:0b6c1a9e-5f0d-4c1e-9a43-3f1f3c0f9b11",
            "urn:librarium:someone-else",
        );
        assert_eq!(read(&opf), None);
        let new = set(&opf, ID).unwrap();
        assert!(
            new.contains("<dc:identifier id=\"pub-id\">urn:librarium:someone-else</dc:identifier>")
        );
        assert_eq!(read(&new).as_deref(), Some(ID));
    }

    #[test]
    fn a_taken_id_attribute_is_not_reused() {
        let opf = EPUB3_OPF
            .replace("<dc:title id=\"title\">", "<dc:title id=\"librarium-id\">")
            .replace("#title", "#librarium-id");
        let new = set(&opf, ID).unwrap();
        assert!(
            new.contains(&format!(
                "<dc:identifier>urn:librarium:{ID}</dc:identifier>"
            )),
            "{new}"
        );
        assert_eq!(read(&new).as_deref(), Some(ID));
    }

    #[test]
    fn refuses_what_it_cant_edit() {
        let empty = r#"<package xmlns="http://www.idpf.org/2007/opf" version="2.0"><metadata/><manifest/><spine/></package>"#;
        assert!(set_library_id(empty, &Package::parse(empty).unwrap(), ID).is_err());
        assert!(set_library_id(EPUB3_OPF, &Package::parse(EPUB3_OPF).unwrap(), "  ").is_err());
    }
}
