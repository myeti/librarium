//! Low-level XML helpers shared by the container and OPF parsers and by the
//! OPF editor: decoding the raw bytes, resolving entity references, escaping,
//! locating attributes inside a start tag, and splicing edits into the
//! original text.
//!
//! The editor never re-serialises a document. It records byte positions while
//! parsing (see [`super::opf::Span`]) and later replaces exactly those ranges
//! ([`Splicer`]), so everything it doesn't touch stays byte-for-byte as the
//! publisher wrote it.

use std::borrow::Cow;
use std::ops::Range;

use super::html::named_entity;

// ── Decoding ──────────────────────────────────────────────────────────────────

/// An XML document decoded to text.
#[derive(Debug, Clone)]
pub(crate) struct XmlText {
    /// The document, without its byte-order mark.
    pub text: String,
    /// Whether the original bytes started with a UTF-8 byte-order mark (kept
    /// when the edited text is encoded again).
    pub utf8_bom: bool,
    /// Whether the original was UTF-8, so `text` *is* the original bytes and
    /// may be edited and written back. Anything else (UTF-16, Latin-1) is
    /// decoded for reading only: writing it back would change its encoding.
    pub editable: bool,
}

impl XmlText {
    /// Decode the raw bytes of an XML file. UTF-8 is the norm for EPUB; UTF-16
    /// (with a byte-order mark) is allowed by the spec; anything that is
    /// neither is read as Latin-1, the usual culprit in old files, so a
    /// misencoded OPF still yields readable metadata.
    pub(crate) fn decode(bytes: &[u8]) -> Self {
        let (body, utf8_bom) = match bytes.strip_prefix(b"\xEF\xBB\xBF") {
            Some(rest) => (rest, true),
            None => (bytes, false),
        };
        if !utf8_bom && (body.starts_with(b"\xFE\xFF") || body.starts_with(b"\xFF\xFE")) {
            let big_endian = body[0] == 0xFE;
            let units: Vec<u16> = body[2..]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&pair| {
                    if big_endian {
                        u16::from_be_bytes(pair)
                    } else {
                        u16::from_le_bytes(pair)
                    }
                })
                .collect();
            return Self {
                text: String::from_utf16_lossy(&units),
                utf8_bom: false,
                editable: false,
            };
        }
        match std::str::from_utf8(body) {
            Ok(text) => Self {
                text: text.to_string(),
                utf8_bom,
                editable: true,
            },
            Err(_) => Self {
                text: body.iter().map(|&b| b as char).collect(),
                utf8_bom,
                editable: false,
            },
        }
    }

    /// The bytes of `text` (an edited version of this document), with the
    /// original byte-order mark if there was one.
    pub(crate) fn encode(&self, text: &str) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(text.len() + 3);
        if self.utf8_bom {
            bytes.extend_from_slice(b"\xEF\xBB\xBF");
        }
        bytes.extend_from_slice(text.as_bytes());
        bytes
    }
}

// ── Entities and escaping ─────────────────────────────────────────────────────

/// The text an entity or character reference stands for. `name` is what
/// sits between `&` and `;`: `amp`, `#233`, `#xE9`, `eacute`.
///
/// Besides XML's five predefined entities this accepts HTML's named ones:
/// they are not valid in an OPF without a DTD declaring them, but they do
/// occur, and their meaning is not in doubt. Unknown names give `None`.
pub(crate) fn resolve_entity(name: &str) -> Option<Cow<'static, str>> {
    if let Some(number) = name.strip_prefix('#') {
        let code = match number.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => number.parse::<u32>().ok()?,
        };
        return char::from_u32(code)
            .filter(|&c| c != '\0')
            .map(|c| Cow::Owned(c.to_string()));
    }
    named_entity(name).map(Cow::Borrowed)
}

/// Escape `s` for use as element content.
pub(crate) fn escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
}

/// Escape `s` for use as an attribute value delimited by `quote`.
/// Tabs and line breaks are written as character references, since a
/// parser would otherwise turn them into spaces.
pub(crate) fn escape_attr(s: &str, quote: char) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if quote == '"' => out.push_str("&quot;"),
            '\'' if quote == '\'' => out.push_str("&apos;"),
            '\t' => out.push_str("&#9;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            _ => out.push(c),
        }
    }
    out
}

// ── Start tags ────────────────────────────────────────────────────────────────

/// One attribute found by [`scan_start_tag`].
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TagAttr {
    /// The attribute name as written (`opf:role`).
    pub name: String,
    /// Byte position of the name inside the tag.
    pub name_start: usize,
    /// Byte range of the raw (escaped) value inside the tag, quotes excluded.
    pub value: Range<usize>,
    pub quote: char,
}

/// The attributes of a start tag such as `<item id="a" href='b.jpg'/>`, with
/// the position of each value inside `tag`, and the position right after the
/// last attribute (or after the element name), where a new attribute can be
/// inserted.
///
/// Only called on tags that quick-xml has already parsed successfully, so the
/// syntax is known to be well-formed; on anything unexpected it simply stops.
pub(crate) fn scan_start_tag(tag: &str) -> (Vec<TagAttr>, usize) {
    let bytes = tag.as_bytes();
    let is_space = |b: u8| matches!(b, b' ' | b'\t' | b'\r' | b'\n');
    let mut i = 1; // skip '<'
    while i < bytes.len() && !is_space(bytes[i]) && bytes[i] != b'/' && bytes[i] != b'>' {
        i += 1;
    }
    let mut append_at = i;
    let mut attrs = Vec::new();
    loop {
        while i < bytes.len() && is_space(bytes[i]) {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] == b'/' || bytes[i] == b'>' {
            break;
        }
        let name_start = i;
        while i < bytes.len() && bytes[i] != b'=' && !is_space(bytes[i]) {
            i += 1;
        }
        let name = tag[name_start..i].to_string();
        while i < bytes.len() && is_space(bytes[i]) {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'=' {
            break;
        }
        i += 1;
        while i < bytes.len() && is_space(bytes[i]) {
            i += 1;
        }
        let Some(&quote) = bytes.get(i).filter(|&&b| b == b'"' || b == b'\'') else {
            break;
        };
        let value_start = i + 1;
        let Some(len) = tag[value_start..].find(quote as char) else {
            break;
        };
        let value_end = value_start + len;
        attrs.push(TagAttr {
            name,
            name_start,
            value: value_start..value_end,
            quote: quote as char,
        });
        i = value_end + 1;
        append_at = i;
    }
    (attrs, append_at)
}

// ── Lines ─────────────────────────────────────────────────────────────────────

/// The line break used by `text` (`"\r\n"` if it has any, else `"\n"`), so
/// inserted lines match the file's convention.
pub(crate) fn line_break(text: &str) -> &'static str {
    if text.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

/// The indentation of the line `pos` is on, if `pos` is the first
/// non-whitespace position of its line; `None` if something else precedes it
/// on the same line (e.g. a single-line OPF).
pub(crate) fn indentation_at(text: &str, pos: usize) -> Option<&str> {
    let line_start = text[..pos].rfind('\n').map_or(0, |i| i + 1);
    let indent = &text[line_start..pos];
    (text[..pos].contains('\n') && indent.chars().all(|c| c == ' ' || c == '\t')).then_some(indent)
}

/// The range to delete to remove `range` (an element) from `text`: its whole
/// line, indentation and line break included, when nothing else is on that
/// line; otherwise just `range`, so neighbours on the same line survive.
pub(crate) fn removal_range(text: &str, range: Range<usize>) -> Range<usize> {
    let line_start = text[..range.start].rfind('\n').map_or(0, |i| i + 1);
    let before_is_blank = text[line_start..range.start]
        .chars()
        .all(|c| c == ' ' || c == '\t');
    let rest = &text[range.end..];
    let line_end = rest.find('\n').map(|i| range.end + i + 1);
    match line_end {
        Some(end)
            if before_is_blank && line_start > 0 && text[range.end..end].trim().is_empty() =>
        {
            line_start..end
        }
        _ => range,
    }
}

// ── Splicing ──────────────────────────────────────────────────────────────────

/// A set of replacements to apply to one text in a single pass.
///
/// Edits refer to byte ranges of the *original* text, so they can be added in
/// any order. Insertions at the same position keep the order they were added
/// in. Overlapping edits are a bug in the caller and make [`Splicer::finish`]
/// fail rather than produce a mangled document.
pub(crate) struct Splicer<'a> {
    text: &'a str,
    edits: Vec<(Range<usize>, String)>,
}

impl<'a> Splicer<'a> {
    pub(crate) fn new(text: &'a str) -> Self {
        Self {
            text,
            edits: Vec::new(),
        }
    }

    pub(crate) fn text(&self) -> &'a str {
        self.text
    }

    /// Replace `range` with `with`.
    pub(crate) fn replace(&mut self, range: Range<usize>, with: impl Into<String>) {
        self.edits.push((range, with.into()));
    }

    /// Insert `s` at `at`.
    pub(crate) fn insert(&mut self, at: usize, s: impl Into<String>) {
        self.edits.push((at..at, s.into()));
    }

    /// Delete `range`.
    pub(crate) fn delete(&mut self, range: Range<usize>) {
        self.edits.push((range, String::new()));
    }

    /// The edited text.
    pub(crate) fn finish(self) -> Result<String, String> {
        let mut edits: Vec<(usize, Range<usize>, String)> = self
            .edits
            .into_iter()
            .enumerate()
            .map(|(seq, (r, s))| (seq, r, s))
            .collect();
        // Stable order: by position, then insertions before the replacement
        // that starts at the same place, then in the order they were added.
        edits.sort_by_key(|(seq, r, _)| (r.start, r.end != r.start, *seq));
        let mut out = String::with_capacity(self.text.len() + 256);
        let mut pos = 0;
        for (_, range, with) in edits {
            if range.start < pos || range.end < range.start || range.end > self.text.len() {
                return Err("internal error: overlapping OPF edits".to_string());
            }
            out.push_str(&self.text[pos..range.start]);
            out.push_str(&with);
            pos = range.end;
        }
        out.push_str(&self.text[pos..]);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoding_keeps_utf8_editable_and_reads_the_rest() {
        let utf8 = XmlText::decode("<a>é</a>".as_bytes());
        assert!(utf8.editable && !utf8.utf8_bom);
        assert_eq!(utf8.text, "<a>é</a>");

        let bom = XmlText::decode(b"\xEF\xBB\xBF<a/>");
        assert!(bom.editable && bom.utf8_bom);
        assert_eq!(bom.text, "<a/>");
        assert_eq!(bom.encode("<b/>"), b"\xEF\xBB\xBF<b/>");

        let utf16: Vec<u8> = [0xFE, 0xFF]
            .into_iter()
            .chain("<a>é</a>".encode_utf16().flat_map(u16::to_be_bytes))
            .collect();
        let decoded = XmlText::decode(&utf16);
        assert_eq!(decoded.text, "<a>é</a>");
        assert!(!decoded.editable);

        let latin1 = XmlText::decode(b"<a>\xE9t\xE9</a>");
        assert_eq!(latin1.text, "<a>été</a>");
        assert!(!latin1.editable);
    }

    #[test]
    fn entities_resolve_numeric_predefined_and_html_names() {
        assert_eq!(resolve_entity("amp").as_deref(), Some("&"));
        assert_eq!(resolve_entity("#233").as_deref(), Some("é"));
        assert_eq!(resolve_entity("#xE9").as_deref(), Some("é"));
        assert_eq!(resolve_entity("eacute").as_deref(), Some("é"));
        assert_eq!(resolve_entity("nbsp").as_deref(), Some("\u{a0}"));
        assert_eq!(resolve_entity("#0"), None);
        assert_eq!(resolve_entity("#xD800"), None);
        assert_eq!(resolve_entity("bogus"), None);
    }

    #[test]
    fn escaping() {
        assert_eq!(escape_text("Tom & Jerry <3>"), "Tom &amp; Jerry &lt;3&gt;");
        assert_eq!(escape_attr("a\"b'c", '"'), "a&quot;b'c");
        assert_eq!(escape_attr("a\"b'c", '\''), "a\"b&apos;c");
    }

    #[test]
    fn start_tags_are_scanned_with_either_quote() {
        let tag = r#"<item id="a" href = 'b c.jpg' media-type="image/jpeg"/>"#;
        let (attrs, append_at) = scan_start_tag(tag);
        let found: Vec<(&str, &str, char)> = attrs
            .iter()
            .map(|a| (a.name.as_str(), &tag[a.value.clone()], a.quote))
            .collect();
        assert_eq!(
            found,
            vec![
                ("id", "a", '"'),
                ("href", "b c.jpg", '\''),
                ("media-type", "image/jpeg", '"')
            ]
        );
        assert_eq!(&tag[append_at..], "/>");

        let (attrs, append_at) = scan_start_tag("<metadata>");
        assert!(attrs.is_empty());
        assert_eq!(append_at, "<metadata".len());
    }

    #[test]
    fn removal_takes_the_whole_line_only_when_alone_on_it() {
        let text = "<m>\n  <a/>\n  <b/>\n</m>";
        let a = text.find("<a/>").unwrap();
        let r = removal_range(text, a..a + 4);
        assert_eq!(
            format!("{}{}", &text[..r.start], &text[r.end..]),
            "<m>\n  <b/>\n</m>"
        );

        let one_line = "<m><a/><b/></m>";
        let a = one_line.find("<a/>").unwrap();
        assert_eq!(removal_range(one_line, a..a + 4), a..a + 4);
    }

    #[test]
    fn indentation_is_found_only_at_line_starts() {
        let text = "<m>\n    <a/><b/>\n</m>";
        assert_eq!(
            indentation_at(text, text.find("<a/>").unwrap()),
            Some("    ")
        );
        assert_eq!(indentation_at(text, text.find("<b/>").unwrap()), None);
        assert_eq!(indentation_at("<a/>", 0), None);
        assert_eq!(line_break("a\r\nb"), "\r\n");
        assert_eq!(line_break("a\nb"), "\n");
    }

    #[test]
    fn splicer_applies_edits_in_position_order_and_refuses_overlaps() {
        let mut s = Splicer::new("0123456789");
        s.replace(7..9, "x");
        s.insert(2, "a");
        s.insert(2, "b");
        s.delete(4..5);
        assert_eq!(s.finish().unwrap(), "01ab2356x9");

        let mut s = Splicer::new("0123456789");
        s.replace(2..6, "x");
        s.insert(4, "y");
        assert!(s.finish().is_err());
    }
}
