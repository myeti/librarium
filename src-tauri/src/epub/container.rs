//! `META-INF/container.xml` (which names the OPF) and archive paths.
//!
//! Hrefs in EPUB files are URLs relative to the file that contains them
//! (`../Images/cover%20art.jpg`); ZIP entries are plain paths from the
//! archive root (`OEBPS/Images/cover art.jpg`). [`resolve_href`] converts.

use std::borrow::Cow;

use quick_xml::events::Event;
use quick_xml::reader::Reader;

/// Where every EPUB keeps its container file.
pub(crate) const CONTAINER_PATH: &str = "META-INF/container.xml";

/// Media type of an OPF package document in `container.xml`.
pub(crate) const OPF_MEDIA_TYPE: &str = "application/oebps-package+xml";

/// The archive paths of the package documents (OPFs) listed in
/// `container.xml`, in order of preference: rootfiles declared with the OPF
/// media type first (an EPUB may also list e.g. a PDF rendition), then the
/// others, each in document order. Attribute quoting and namespace prefixes
/// don't matter; `full-path` is percent-decoded.
pub(crate) fn rootfiles(container_xml: &str) -> Result<Vec<String>, String> {
    let mut reader = Reader::from_str(container_xml);
    let mut preferred = Vec::new();
    let mut others = Vec::new();
    loop {
        let event = reader.read_event().map_err(|e| {
            format!(
                "META-INF/container.xml is not valid XML ({e}, at byte {})",
                reader.error_position()
            )
        })?;
        match event {
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == "rootfile" => {
                let mut path = None;
                let mut media_type = None;
                for attr in e.attributes() {
                    let attr = attr.map_err(|e| {
                        format!("META-INF/container.xml has a malformed attribute: {e}")
                    })?;
                    let value = super::opf::attribute_value(&attr);
                    match attr.key.local_name().as_ref() {
                        "full-path" => path = Some(value),
                        "media-type" => media_type = Some(value),
                        _ => {}
                    }
                }
                let Some(path) =
                    path.map(|p| percent_decode(p.trim()).trim_start_matches('/').to_string())
                else {
                    continue;
                };
                if path.is_empty() {
                    continue;
                }
                if media_type
                    .as_deref()
                    .is_some_and(|m| m.trim().eq_ignore_ascii_case(OPF_MEDIA_TYPE))
                {
                    preferred.push(path);
                } else {
                    others.push(path);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    preferred.extend(others);
    Ok(preferred)
}

/// Decode `%XX` escapes. Invalid escapes are kept as written; if the decoded
/// bytes aren't UTF-8, the input is returned unchanged.
pub(crate) fn percent_decode(s: &str) -> Cow<'_, str> {
    if !s.contains('%') {
        return Cow::Borrowed(s);
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    match String::from_utf8(out) {
        Ok(decoded) => Cow::Owned(decoded),
        Err(_) => Cow::Borrowed(s),
    }
}

/// The directory part of an archive path (`OEBPS` for `OEBPS/content.opf`,
/// `""` at the root).
pub(crate) fn parent_dir(path: &str) -> &str {
    path.rfind('/').map_or("", |i| &path[..i])
}

/// The archive path an `href` found in a file of directory `base_dir` points
/// to: fragment removed, percent-decoded, `.`/`..` resolved. `None` for
/// anything that isn't a path inside the archive: absolute URLs
/// (`http://…`, `data:`), empty hrefs, or a path climbing above the root.
pub(crate) fn resolve_href(base_dir: &str, href: &str) -> Option<String> {
    let href = href.trim();
    let href = href.split(['#', '?']).next().unwrap_or("");
    let has_scheme = href.split_once(':').is_some_and(|(scheme, _)| {
        !scheme.is_empty()
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
    });
    if href.is_empty() || has_scheme {
        return None;
    }
    let href = percent_decode(href);
    let (base, rel) = match href.strip_prefix('/') {
        Some(rel) => ("", rel),
        None => (base_dir, href.as_ref()),
    };
    let mut parts: Vec<&str> = base.split('/').filter(|p| !p.is_empty()).collect();
    for segment in rel.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rootfiles_accept_either_quote_prefixes_and_percent_escapes() {
        let xml = r#"<?xml version="1.0"?>
<c:container version="1.0" xmlns:c="urn:oasis:names:tc:opendocument:xmlns:container">
  <c:rootfiles>
    <c:rootfile full-path='book.pdf' media-type='application/pdf'/>
    <c:rootfile media-type='application/oebps-package+xml' full-path='OEBPS/My%20Book.opf'/>
  </c:rootfiles>
</c:container>"#;
        assert_eq!(
            rootfiles(xml).unwrap(),
            vec!["OEBPS/My Book.opf".to_string(), "book.pdf".to_string()]
        );
    }

    #[test]
    fn rootfiles_of_a_broken_container() {
        assert!(
            rootfiles("<container><rootfiles><rootfile full-path=\"a.opf\"></container>").is_err()
        );
        assert_eq!(rootfiles("<container/>").unwrap(), Vec::<String>::new());
        assert_eq!(
            rootfiles("<container><rootfile full-path=\"/content.opf\"/></container>").unwrap(),
            vec!["content.opf"]
        );
    }

    #[test]
    fn percent_decoding() {
        assert_eq!(percent_decode("a%20b%C3%A9"), "a bé");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz%4"), "%zz%4");
        assert_eq!(percent_decode("%FF"), "%FF", "not UTF-8 once decoded: kept");
    }

    #[test]
    fn hrefs_resolve_to_archive_paths() {
        assert_eq!(
            resolve_href("OEBPS", "images/cover.jpg").as_deref(),
            Some("OEBPS/images/cover.jpg")
        );
        assert_eq!(
            resolve_href("OEBPS/Text", "../Images/a%20b.jpg#frag").as_deref(),
            Some("OEBPS/Images/a b.jpg")
        );
        assert_eq!(
            resolve_href("", "./cover.jpg").as_deref(),
            Some("cover.jpg")
        );
        assert_eq!(
            resolve_href("OEBPS", "/cover.jpg").as_deref(),
            Some("cover.jpg")
        );
        assert_eq!(resolve_href("OEBPS", "../../escape.jpg"), None);
        assert_eq!(resolve_href("OEBPS", "http://example.com/a.jpg"), None);
        assert_eq!(resolve_href("OEBPS", "data:image/png;base64,AAAA"), None);
        assert_eq!(resolve_href("OEBPS", ""), None);
        assert_eq!(parent_dir("OEBPS/content.opf"), "OEBPS");
        assert_eq!(parent_dir("content.opf"), "");
    }
}
