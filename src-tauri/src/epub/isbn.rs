//! ISBN recognition: telling an ISBN apart from the other identifiers an EPUB
//! carries (UUIDs, Calibre ids, ASINs, URLs), and checking its checksum.

/// The ISBN in `value`, as bare digits (`9780441013593`, or an ISBN-10 that
/// may end in `X`), with hyphens and spaces removed: `None` unless it has
/// exactly 10 or 13 characters of digits (plus a final `X` for ISBN-10).
/// The checksum is *not* checked here (see [`is_valid`]).
pub(crate) fn clean(value: &str) -> Option<String> {
    let cleaned: String = value
        .trim()
        .chars()
        .filter(|c| !matches!(c, '-' | ' ' | '\u{a0}' | '\u{2010}' | '\u{2011}'))
        .map(|c| c.to_ascii_uppercase())
        .collect();
    let digits_then_check = |len: usize| {
        cleaned.len() == len
            && cleaned.chars().take(len - 1).all(|c| c.is_ascii_digit())
            && cleaned
                .chars()
                .last()
                .is_some_and(|c| c.is_ascii_digit() || (len == 10 && c == 'X'))
    };
    (digits_then_check(10) || digits_then_check(13)).then_some(cleaned)
}

/// Whether `isbn` (as returned by [`clean`]) has a valid ISBN-10 or ISBN-13
/// check digit.
pub(crate) fn is_valid(isbn: &str) -> bool {
    let digit = |c: char| if c == 'X' { Some(10) } else { c.to_digit(10) };
    match isbn.len() {
        10 => {
            let mut sum = 0;
            for (i, c) in isbn.chars().enumerate() {
                let Some(d) = digit(c) else { return false };
                if d == 10 && i != 9 {
                    return false;
                }
                sum += (10 - i as u32) * d;
            }
            sum % 11 == 0
        }
        13 => {
            let mut sum = 0;
            for (i, c) in isbn.chars().enumerate() {
                let Some(d) = c.to_digit(10) else {
                    return false;
                };
                sum += if i % 2 == 0 { d } else { 3 * d };
            }
            sum % 10 == 0
        }
        _ => false,
    }
}

/// `value` without `prefix`, compared ASCII-case-insensitively. Safe on any
/// text: it never slices inside a multi-byte character.
fn strip_prefix_ignore_case<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    let head = value.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &value[prefix.len()..])
}

/// How an identifier says it is an ISBN.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IsbnForm {
    /// `urn:isbn:…` (the EPUB 3 way, valid in EPUB 2 too).
    Urn,
    /// An `opf:scheme="ISBN"` attribute (EPUB 2), an EPUB 3
    /// `identifier-type` refinement, or an `isbn:` prefix.
    Declared,
    /// A bare value that happens to be a valid ISBN.
    Bare,
}

/// Whether a `dc:identifier` holds an ISBN, and which: `value` is its text;
/// `declared_isbn` says whether its scheme says ISBN (`opf:scheme="ISBN"`,
/// or an EPUB 3 `identifier-type` refinement); `other_scheme` whether it
/// declares a scheme that is *not* ISBN (`calibre`, `uuid`, `AMAZON`…).
///
/// Declared ISBNs (`urn:isbn:`, `isbn:`, a scheme) are trusted without a
/// checksum, but must still look like one (10 or 13 digits). A bare value
/// must have a valid checksum, and an ISBN-13 must start with 978/979, so a
/// random 10- or 13-digit number isn't mistaken for one. A value under
/// another scheme is never an ISBN unless it is a `urn:isbn:`.
pub(crate) fn from_identifier(
    value: &str,
    declared_isbn: bool,
    other_scheme: bool,
) -> Option<(String, IsbnForm)> {
    let value = value.trim();
    if let Some(rest) = strip_prefix_ignore_case(value, "urn:isbn:") {
        return clean(rest).map(|isbn| (isbn, IsbnForm::Urn));
    }
    for prefix in ["isbn:", "isbn "] {
        if let Some(rest) = strip_prefix_ignore_case(value, prefix) {
            return clean(rest).map(|isbn| (isbn, IsbnForm::Declared));
        }
    }
    if declared_isbn {
        return clean(value).map(|isbn| (isbn, IsbnForm::Declared));
    }
    if other_scheme {
        return None;
    }
    let isbn = clean(value).filter(|isbn| is_valid(isbn))?;
    let plausible = isbn.len() == 10 || isbn.starts_with("978") || isbn.starts_with("979");
    plausible.then_some((isbn, IsbnForm::Bare))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleaning_keeps_only_well_shaped_isbns() {
        assert_eq!(clean("978-0-441-01359-3").as_deref(), Some("9780441013593"));
        assert_eq!(clean(" 0 441 01359 7 ").as_deref(), Some("0441013597"));
        assert_eq!(clean("080442957x").as_deref(), Some("080442957X"));
        assert_eq!(clean("12345"), None);
        assert_eq!(
            clean("97804410135X3"),
            None,
            "X only as an ISBN-10 check digit"
        );
        assert_eq!(clean("urn:uuid:0b6c1a9e"), None);
        assert_eq!(
            clean("９７８０４４１０１３５９３"),
            None,
            "full-width digits are not ASCII digits"
        );
    }

    #[test]
    fn checksums() {
        assert!(is_valid("9780441013593"));
        assert!(is_valid("0441013597"));
        assert!(is_valid("080442957X"));
        assert!(!is_valid("9780441013594"));
        assert!(!is_valid("0441013598"));
        assert!(!is_valid("X441013597"));
    }

    #[test]
    fn identifiers() {
        let isbn = |v, declared, other| from_identifier(v, declared, other);
        assert_eq!(
            isbn("urn:isbn:978-0-441-01359-3", false, false),
            Some(("9780441013593".into(), IsbnForm::Urn))
        );
        assert_eq!(
            isbn("URN:ISBN:9780441013593", false, true),
            Some(("9780441013593".into(), IsbnForm::Urn))
        );
        assert_eq!(
            isbn("ISBN 0441013597", false, false),
            Some(("0441013597".into(), IsbnForm::Declared))
        );
        // Declared: no checksum required.
        assert_eq!(
            isbn("9780441013594", true, false),
            Some(("9780441013594".into(), IsbnForm::Declared))
        );
        assert_eq!(isbn("N/A", true, false), None);
        // Bare: checksum and prefix required.
        assert_eq!(
            isbn("9780441013593", false, false),
            Some(("9780441013593".into(), IsbnForm::Bare))
        );
        assert_eq!(isbn("9780441013594", false, false), None);
        assert_eq!(
            isbn("4006381333931", false, false),
            None,
            "a valid EAN that isn't an ISBN"
        );
        // Another scheme: not an ISBN.
        assert_eq!(isbn("0441013597", false, true), None);
        assert_eq!(
            isbn(
                "urn:uuid:5f2b1c3e-6e0a-4c1b-9a43-3f1f3c0f9b11",
                false,
                false
            ),
            None
        );
        // Non-ASCII text never panics.
        assert_eq!(isbn("ünïcödé", false, false), None);
        assert_eq!(isbn("é", false, false), None);
    }
}
