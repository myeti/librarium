//! Plain text from the HTML found in book descriptions.
//!
//! Descriptions (`dc:description`, Calibre comments, PDF subjects) are often
//! HTML fragments: `<p>…</p>`, `<br/>`, `&eacute;`. [`strip_html`] turns them
//! into readable plain text. It is deliberately small — a description is a
//! short fragment, not a web page — and never fails: anything it doesn't
//! recognise as markup is kept as text.

/// Plain text from an HTML fragment:
/// - tags are removed; `<script>`/`<style>` contents and comments are dropped;
/// - `<br>` becomes a line break, the end of a paragraph (`</p>`) a blank
///   line, and the end of other blocks (`</div>`, `</li>`, headings,
///   `</tr>`, `</blockquote>`) a line break;
/// - named (`&eacute;`) and numeric (`&#233;`, `&#xE9;`) entities are decoded;
///   unknown ones are kept as written;
/// - whitespace is normalised: runs of spaces collapse to one, lines are
///   trimmed, at most one blank line separates paragraphs, and the result is
///   trimmed.
///
/// A `<` that doesn't start a tag (`a < b`) is kept as text.
pub(crate) fn strip_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(c) = rest.chars().next() {
        match c {
            '<' => {
                if let Some(after) = rest.strip_prefix("<!--") {
                    rest = after.find("-->").map_or("", |i| &after[i + 3..]);
                    continue;
                }
                if let Some(after) = rest.strip_prefix("<![CDATA[") {
                    let end = after.find("]]>").unwrap_or(after.len());
                    push_text(&mut out, &after[..end]);
                    rest = after.get(end + 3..).unwrap_or("");
                    continue;
                }
                match parse_tag(rest) {
                    Some((tag, len)) => {
                        rest = &rest[len..];
                        if !tag.closing && (tag.name == "script" || tag.name == "style") {
                            rest = skip_raw_text(rest, &tag.name);
                        }
                        match (tag.name.as_str(), tag.closing) {
                            ("br", _) => out.push('\n'),
                            ("p", true) => out.push_str("\n\n"),
                            ("p", false) => out.push('\n'),
                            (
                                "div" | "li" | "tr" | "blockquote" | "h1" | "h2" | "h3" | "h4"
                                | "h5" | "h6",
                                true,
                            ) => out.push('\n'),
                            _ => {}
                        }
                    }
                    None => {
                        out.push('<');
                        rest = &rest[1..];
                    }
                }
            }
            '&' => {
                let (text, len) = decode_entity(rest);
                out.push_str(&text);
                rest = &rest[len..];
            }
            _ => {
                let end = rest.find(['<', '&']).unwrap_or(rest.len());
                push_text(&mut out, &rest[..end]);
                rest = &rest[end..];
            }
        }
    }
    normalize_whitespace(&out)
}

/// Append source text: in HTML, line breaks and tabs in the source are just
/// spaces (only tags make line breaks).
fn push_text(out: &mut String, text: &str) {
    out.extend(
        text.chars()
            .map(|c| if c.is_whitespace() { ' ' } else { c }),
    );
}

struct Tag {
    /// Lower-cased element name.
    name: String,
    closing: bool,
}

/// The tag at the start of `s` (which starts with `<`) and its length, if it
/// is one: `<name …>`, `</name>`, `<name/>`, or a declaration/processing
/// instruction (`<!DOCTYPE…>`, `<?xml…?>`, returned with an empty name).
/// Quoted attribute values may contain `>`.
fn parse_tag(s: &str) -> Option<(Tag, usize)> {
    let bytes = s.as_bytes();
    let mut i = 1;
    let closing = bytes.get(i) == Some(&b'/');
    if closing {
        i += 1;
    }
    let declaration = matches!(bytes.get(i), Some(b'!' | b'?'));
    if !declaration && !bytes.get(i).is_some_and(|b| b.is_ascii_alphabetic()) {
        return None;
    }
    let name_start = i;
    while i < bytes.len()
        && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b':' | b'-' | b'_'))
    {
        i += 1;
    }
    let name = if declaration {
        String::new()
    } else {
        s[name_start..i].to_ascii_lowercase()
    };
    let mut quote = None;
    while i < bytes.len() {
        match (bytes[i], quote) {
            (b'"' | b'\'', None) => quote = Some(bytes[i]),
            (q, Some(open)) if q == open => quote = None,
            (b'>', None) => return Some((Tag { name, closing }, i + 1)),
            _ => {}
        }
        i += 1;
    }
    None
}

/// Skip the raw text of a `<script>`/`<style>` element, up to and including
/// its end tag.
fn skip_raw_text<'a>(s: &'a str, name: &str) -> &'a str {
    let end_tag = format!("</{name}");
    let lower = s.to_ascii_lowercase();
    match lower.find(&end_tag) {
        Some(i) => s[i..].find('>').map_or("", |j| &s[i + j + 1..]),
        None => "",
    }
}

/// Decode the entity at the start of `s` (which starts with `&`). Returns the
/// text and how many bytes it used; an unknown or malformed entity is kept
/// as a literal `&`.
fn decode_entity(s: &str) -> (String, usize) {
    // Entity names are short: only look at the next few characters (cut on
    // a character boundary, the text may be anything).
    let after_amp = &s[1..];
    let candidate = after_amp
        .char_indices()
        .nth(40)
        .map_or(after_amp, |(i, _)| &after_amp[..i]);
    if let Some(end) = candidate.find(';') {
        let name = &candidate[..end];
        if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '#') {
            if let Some(text) = super::xml::resolve_entity(name) {
                return (text.into_owned(), end + 2);
            }
        }
    }
    ("&".to_string(), 1)
}

/// Collapse spaces, trim lines, keep at most one blank line in a row, trim.
pub(crate) fn normalize_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank_lines = 0;
    for line in text.split('\n') {
        let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.is_empty() {
            blank_lines += 1;
            continue;
        }
        if !out.is_empty() {
            out.push_str(if blank_lines > 1 { "\n\n" } else { "\n" });
        }
        blank_lines = 0;
        out.push_str(&line);
    }
    out
}

/// The character(s) an HTML named entity stands for: XML's five predefined
/// entities plus the HTML entities that turn up in book metadata (Latin-1
/// letters and symbols, typographic punctuation). Not the full HTML5 list.
pub(crate) fn named_entity(name: &str) -> Option<&'static str> {
    Some(match name {
        // XML predefined
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        // Spaces and invisible characters
        "nbsp" => "\u{a0}",
        "ensp" => "\u{2002}",
        "emsp" => "\u{2003}",
        "thinsp" => "\u{2009}",
        "zwnj" => "\u{200c}",
        "zwj" => "\u{200d}",
        "lrm" => "\u{200e}",
        "rlm" => "\u{200f}",
        "shy" => "\u{ad}",
        // Punctuation
        "ndash" => "–",
        "mdash" => "—",
        "hellip" => "…",
        "lsquo" => "‘",
        "rsquo" => "’",
        "sbquo" => "‚",
        "ldquo" => "“",
        "rdquo" => "”",
        "bdquo" => "„",
        "laquo" => "«",
        "raquo" => "»",
        "lsaquo" => "‹",
        "rsaquo" => "›",
        "bull" => "•",
        "middot" => "·",
        "dagger" => "†",
        "Dagger" => "‡",
        "prime" => "′",
        "Prime" => "″",
        "permil" => "‰",
        "iexcl" => "¡",
        "iquest" => "¿",
        "sect" => "§",
        "para" => "¶",
        // Symbols
        "copy" => "©",
        "reg" => "®",
        "trade" => "™",
        "deg" => "°",
        "plusmn" => "±",
        "times" => "×",
        "divide" => "÷",
        "micro" => "µ",
        "not" => "¬",
        "euro" => "€",
        "pound" => "£",
        "yen" => "¥",
        "cent" => "¢",
        "curren" => "¤",
        "brvbar" => "¦",
        "uml" => "¨",
        "macr" => "¯",
        "acute" => "´",
        "cedil" => "¸",
        "ordf" => "ª",
        "ordm" => "º",
        "sup1" => "¹",
        "sup2" => "²",
        "sup3" => "³",
        "frac14" => "¼",
        "frac12" => "½",
        "frac34" => "¾",
        "circ" => "ˆ",
        "tilde" => "˜",
        "fnof" => "ƒ",
        // Latin letters
        "Agrave" => "À",
        "Aacute" => "Á",
        "Acirc" => "Â",
        "Atilde" => "Ã",
        "Auml" => "Ä",
        "Aring" => "Å",
        "AElig" => "Æ",
        "Ccedil" => "Ç",
        "Egrave" => "È",
        "Eacute" => "É",
        "Ecirc" => "Ê",
        "Euml" => "Ë",
        "Igrave" => "Ì",
        "Iacute" => "Í",
        "Icirc" => "Î",
        "Iuml" => "Ï",
        "ETH" => "Ð",
        "Ntilde" => "Ñ",
        "Ograve" => "Ò",
        "Oacute" => "Ó",
        "Ocirc" => "Ô",
        "Otilde" => "Õ",
        "Ouml" => "Ö",
        "Oslash" => "Ø",
        "Ugrave" => "Ù",
        "Uacute" => "Ú",
        "Ucirc" => "Û",
        "Uuml" => "Ü",
        "Yacute" => "Ý",
        "THORN" => "Þ",
        "szlig" => "ß",
        "agrave" => "à",
        "aacute" => "á",
        "acirc" => "â",
        "atilde" => "ã",
        "auml" => "ä",
        "aring" => "å",
        "aelig" => "æ",
        "ccedil" => "ç",
        "egrave" => "è",
        "eacute" => "é",
        "ecirc" => "ê",
        "euml" => "ë",
        "igrave" => "ì",
        "iacute" => "í",
        "icirc" => "î",
        "iuml" => "ï",
        "eth" => "ð",
        "ntilde" => "ñ",
        "ograve" => "ò",
        "oacute" => "ó",
        "ocirc" => "ô",
        "otilde" => "õ",
        "ouml" => "ö",
        "oslash" => "ø",
        "ugrave" => "ù",
        "uacute" => "ú",
        "ucirc" => "û",
        "uuml" => "ü",
        "yacute" => "ý",
        "thorn" => "þ",
        "yuml" => "ÿ",
        "OElig" => "Œ",
        "oelig" => "œ",
        "Scaron" => "Š",
        "scaron" => "š",
        "Yuml" => "Ÿ",
        "Zcaron" => "Ž",
        "zcaron" => "ž",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::strip_html;

    #[test]
    fn tags_are_removed_and_blocks_become_line_breaks() {
        assert_eq!(
            strip_html("<p>First <b>bold</b> line.</p><p>Second</p>"),
            "First bold line.\n\nSecond"
        );
        assert_eq!(
            strip_html("one<br>two<br/>three<BR />four"),
            "one\ntwo\nthree\nfour"
        );
        assert_eq!(strip_html("<div>a</div><div>b</div>"), "a\nb");
        assert_eq!(strip_html("<ul><li>x</li><li>y</li></ul>"), "x\ny");
        assert_eq!(strip_html("<h2>Title</h2>Body"), "Title\nBody");
    }

    #[test]
    fn entities_are_decoded() {
        assert_eq!(
            strip_html("<p>caf&eacute; &amp; th&#233; &#x2014; &quot;ok&quot;</p>"),
            "café & thé — \"ok\""
        );
        assert_eq!(
            strip_html("a&nbsp;b"),
            "a b",
            "a no-break space is whitespace"
        );
        assert_eq!(strip_html("AT&T &unknown; &"), "AT&T &unknown; &");
    }

    #[test]
    fn whitespace_is_normalised() {
        assert_eq!(
            strip_html("  <p>\n  lots   of\n\tspace  </p>\n\n\n<p>x</p>  "),
            "lots of space\n\nx"
        );
        assert_eq!(strip_html("<p></p><p></p><p>only</p>"), "only");
    }

    #[test]
    fn non_tags_comments_and_scripts() {
        assert_eq!(strip_html("1 < 2 and 3 > 2"), "1 < 2 and 3 > 2");
        assert_eq!(strip_html("a<!-- hidden <p> -->b"), "ab");
        assert_eq!(
            strip_html("x<script>if (a < b) {}</script>y<style>p{}</style>z"),
            "xyz"
        );
        assert_eq!(strip_html("<a href=\"x>y\">link</a>"), "link");
        assert_eq!(strip_html("unclosed <b"), "unclosed <b");
        assert_eq!(strip_html("<![CDATA[raw <b>]]>"), "raw <b>");
        assert_eq!(strip_html(""), "");
    }
}
