//! What the metadata entries of a [`Package`] mean: title, authors, ISBN,
//! series… ([`read`] → [`EpubMetadata`]).
//!
//! The functions that *find* the elements (`main_title`, `author_entries`,
//! `isbn_entries`…) are shared with the editor ([`super::edit`]), so that what
//! gets written is exactly what gets read back.

use super::html::{normalize_whitespace, strip_html};
use super::isbn::{self, IsbnForm};
use super::opf::{EntryKind, MetadataEntry, Package};

/// The descriptive metadata of an EPUB, as the file states it. Everything is
/// optional: a value the file doesn't have is `None` (or empty), never a
/// placeholder.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct EpubMetadata {
    /// The main title (whitespace collapsed).
    pub title: Option<String>,
    /// Only when the file says so: an EPUB 3 title refined with
    /// `title-type="subtitle"`.
    pub subtitle: Option<String>,
    /// Authors (creators with role `aut`, or no role), in document order.
    pub authors: Vec<String>,
    /// The first `dc:language`, as written (`en`, `fre`, `pt-BR`).
    pub language: Option<String>,
    /// Plain text: HTML descriptions are stripped ([`strip_html`]).
    pub description: Option<String>,
    /// Bare digits (`9780441013593`).
    pub isbn: Option<String>,
    pub series: Option<String>,
    pub series_index: Option<f32>,
    /// `dc:subject` values.
    pub categories: Vec<String>,
    /// 1–5 stars (from Calibre's 0–10 `calibre:rating`).
    pub rating: Option<u8>,
}

/// EPUB 2 Calibre series name / position and rating (`<meta name content/>`).
pub(crate) const CALIBRE_SERIES: &str = "calibre:series";
pub(crate) const CALIBRE_SERIES_INDEX: &str = "calibre:series_index";
pub(crate) const CALIBRE_RATING: &str = "calibre:rating";

/// Read the metadata of a package.
pub(crate) fn read(pkg: &Package) -> EpubMetadata {
    let text_of = |i: usize| clean_text(&pkg.metadata[i].text);
    let non_empty = |s: String| (!s.is_empty()).then_some(s);

    let (series, series_index) = series(pkg);
    EpubMetadata {
        title: main_title(pkg).map(text_of).and_then(non_empty),
        subtitle: subtitle(pkg).map(text_of).and_then(non_empty),
        authors: author_entries(pkg)
            .into_iter()
            .map(text_of)
            .filter(|a| !a.is_empty())
            .collect(),
        language: first_dc(pkg, "language").map(text_of),
        description: first_dc(pkg, "description")
            .map(|i| description_text(&pkg.metadata[i].text))
            .and_then(non_empty),
        isbn: preferred_isbn(pkg).map(|(_, isbn)| isbn),
        series,
        series_index,
        categories: subject_entries(pkg).into_iter().map(text_of).collect(),
        rating: rating(pkg).map(|(_, r)| r),
    }
}

/// Trim and collapse whitespace runs (pretty-printed OPFs wrap long titles).
pub(crate) fn clean_text(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A description as plain text: HTML is stripped; plain text keeps its line
/// breaks (they are meaningful) but loses the indentation a pretty-printed
/// OPF puts on continuation lines. Both get the same whitespace rules
/// ([`normalize_whitespace`]).
pub(crate) fn description_text(raw: &str) -> String {
    if looks_like_html(raw) {
        strip_html(raw)
    } else {
        normalize_whitespace(raw)
    }
}

/// Whether a description is HTML rather than plain text that happens to
/// contain a `<` ("a <3 b", or what the app itself writes): it has an end
/// tag, a self-closing tag, or a paragraph or line break.
fn looks_like_html(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    lower.contains("</")
        || lower.contains("/>")
        || lower.contains("<br")
        || lower.contains("<p>")
        || lower.contains("<p ")
}

// ── Finding entries ───────────────────────────────────────────────────────────

/// Indices of the Dublin Core elements `name`, in document order.
pub(crate) fn dc_entries<'a>(pkg: &'a Package, name: &'a str) -> impl Iterator<Item = usize> + 'a {
    pkg.metadata
        .iter()
        .enumerate()
        .filter(move |(_, e)| e.is_dc(name))
        .map(|(i, _)| i)
}

/// The first `name` element with some text.
fn first_dc(pkg: &Package, name: &str) -> Option<usize> {
    dc_entries(pkg, name).find(|&i| !pkg.metadata[i].text.trim().is_empty())
}

/// Indices of the EPUB 3 `<meta>` entries refining the element with id `id`.
pub(crate) fn refinements<'a>(pkg: &'a Package, id: &'a str) -> impl Iterator<Item = usize> + 'a {
    pkg.metadata
        .iter()
        .enumerate()
        .filter(move |(_, e)| e.kind == EntryKind::Meta && e.refines() == Some(id))
        .map(|(i, _)| i)
}

/// The refinement `property` of entry `i` (e.g. the `role` of a creator):
/// index of the `<meta>` and its trimmed value.
pub(crate) fn refined(pkg: &Package, i: usize, property: &str) -> Option<(usize, String)> {
    let id = pkg.metadata[i].element.id()?.trim();
    refinements(pkg, id)
        .find(|&m| pkg.metadata[m].property() == Some(property))
        .map(|m| (m, pkg.metadata[m].text.trim().to_string()))
}

/// The EPUB 2 `<meta name="name">` entry.
pub(crate) fn meta_named(pkg: &Package, name: &str) -> Option<usize> {
    pkg.metadata
        .iter()
        .position(|e| e.meta_name().map(str::trim) == Some(name))
}

/// The main title: the first `dc:title` not refined as a subtitle (or
/// another secondary title type).
pub(crate) fn main_title(pkg: &Package) -> Option<usize> {
    let secondary = |i: usize| {
        refined(pkg, i, "title-type")
            .is_some_and(|(_, t)| !t.eq_ignore_ascii_case("main") && !t.is_empty())
    };
    dc_entries(pkg, "title").find(|&i| !secondary(i) && !pkg.metadata[i].text.trim().is_empty())
}

/// An EPUB 3 `dc:title` refined with `title-type="subtitle"`.
pub(crate) fn subtitle(pkg: &Package) -> Option<usize> {
    dc_entries(pkg, "title").find(|&i| {
        refined(pkg, i, "title-type").is_some_and(|(_, t)| t.eq_ignore_ascii_case("subtitle"))
    })
}

/// The role of a `dc:creator`: EPUB 2 `opf:role`, or an EPUB 3 `role`
/// refinement. `None` when unspecified.
fn creator_role(pkg: &Package, i: usize) -> Option<String> {
    let entry = &pkg.metadata[i];
    entry
        .element
        .opf_attr("role")
        .map(|r| r.trim().to_string())
        .or_else(|| refined(pkg, i, "role").map(|(_, r)| r))
        .filter(|r| !r.is_empty())
}

/// The `dc:creator` entries that are authors: role `aut`, or no role at all
/// (most files without roles list only authors). Editors, translators,
/// illustrators… are not authors and are never edited.
pub(crate) fn author_entries(pkg: &Package) -> Vec<usize> {
    dc_entries(pkg, "creator")
        .filter(|&i| creator_role(pkg, i).is_none_or(|r| r.eq_ignore_ascii_case("aut")))
        .collect()
}

/// The `dc:subject` entries with some text.
pub(crate) fn subject_entries(pkg: &Package) -> Vec<usize> {
    dc_entries(pkg, "subject")
        .filter(|&i| !pkg.metadata[i].text.trim().is_empty())
        .collect()
}

/// An identifier that holds an ISBN.
#[derive(Debug, Clone)]
pub(crate) struct IsbnEntry {
    pub index: usize,
    pub isbn: String,
    pub form: IsbnForm,
    /// Whether this is the package's unique identifier (never edited).
    pub is_unique_identifier: bool,
}

/// Every `dc:identifier` that holds an ISBN, in document order.
pub(crate) fn isbn_entries(pkg: &Package) -> Vec<IsbnEntry> {
    let unique = pkg.unique_identifier_entry();
    dc_entries(pkg, "identifier")
        .filter_map(|i| {
            let entry = &pkg.metadata[i];
            let scheme = identifier_scheme(pkg, i);
            let declared = scheme
                .as_deref()
                .is_some_and(|s| s.eq_ignore_ascii_case("isbn"));
            let other_scheme = scheme.is_some() && !declared;
            let (isbn, form) = isbn::from_identifier(&entry.text, declared, other_scheme)?;
            Some(IsbnEntry {
                index: i,
                isbn,
                form,
                is_unique_identifier: Some(i) == unique,
            })
        })
        .collect()
}

/// The scheme an identifier declares: EPUB 2 `opf:scheme`, or an EPUB 3
/// `identifier-type` refinement (ONIX code list 5: `15` is ISBN-13, `02`
/// ISBN-10), normalised to `ISBN` for ISBNs.
fn identifier_scheme(pkg: &Package, i: usize) -> Option<String> {
    if let Some(scheme) = pkg.metadata[i]
        .element
        .opf_attr("scheme")
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Some(scheme.to_string());
    }
    let (m, value) = refined(pkg, i, "identifier-type")?;
    let onix = pkg.metadata[m]
        .element
        .attr("scheme")
        .is_some_and(|s| s.trim().eq_ignore_ascii_case("onix:codelist5"));
    if (onix && (value == "15" || value == "02")) || value.eq_ignore_ascii_case("isbn") {
        Some("ISBN".to_string())
    } else {
        Some(value)
    }
}

/// The ISBN shown for the book: the first one that isn't the unique
/// identifier (an ISBN the user or a tool added on purpose), else the unique
/// identifier's own. Returns its entry index and digits.
pub(crate) fn preferred_isbn(pkg: &Package) -> Option<(usize, String)> {
    let entries = isbn_entries(pkg);
    entries
        .iter()
        .find(|e| !e.is_unique_identifier)
        .or_else(|| entries.first())
        .map(|e| (e.index, e.isbn.clone()))
}

/// EPUB 3 `<meta property="belongs-to-collection">` that is a series:
/// `collection-type="series"`, or no collection type (an untyped collection
/// is almost always a series). Collections typed otherwise (`set`) are not.
pub(crate) fn series_collection(pkg: &Package) -> Option<usize> {
    let collections: Vec<usize> = pkg
        .metadata
        .iter()
        .enumerate()
        .filter(|(_, e)| {
            e.property() == Some("belongs-to-collection")
                && e.refines().is_none()
                && !e.text.trim().is_empty()
        })
        .map(|(i, _)| i)
        .collect();
    let kind = |i: usize| refined(pkg, i, "collection-type").map(|(_, t)| t);
    collections
        .iter()
        .copied()
        .find(|&i| kind(i).is_some_and(|t| t.eq_ignore_ascii_case("series")))
        .or_else(|| collections.iter().copied().find(|&i| kind(i).is_none()))
}

/// Parse a series position (`1`, `1.0`, `2.5`).
pub(crate) fn parse_index(s: &str) -> Option<f32> {
    s.trim().parse::<f32>().ok().filter(|v| v.is_finite())
}

/// Series name and position: Calibre's `calibre:series` /
/// `calibre:series_index` first (what Calibre and Kobo use), else an EPUB 3
/// series collection with its `group-position`.
fn series(pkg: &Package) -> (Option<String>, Option<f32>) {
    let content = |i: usize| {
        pkg.metadata[i]
            .meta_content()
            .map(clean_text)
            .filter(|s| !s.is_empty())
    };
    if let Some(name) = meta_named(pkg, CALIBRE_SERIES).and_then(content) {
        let index = meta_named(pkg, CALIBRE_SERIES_INDEX)
            .and_then(content)
            .and_then(|s| parse_index(&s));
        return (Some(name), index);
    }
    if let Some(c) = series_collection(pkg) {
        let index = refined(pkg, c, "group-position").and_then(|(_, v)| parse_index(&v));
        return (Some(clean_text(&pkg.metadata[c].text)), index);
    }
    (None, None)
}

/// Calibre's `calibre:rating` (0–10, half-stars allowed) as 1–5 stars; 0 or
/// an unreadable value means "not rated".
pub(crate) fn rating(pkg: &Package) -> Option<(usize, u8)> {
    let i = meta_named(pkg, CALIBRE_RATING)?;
    let value = pkg.metadata[i]
        .meta_content()?
        .trim()
        .parse::<f32>()
        .ok()
        .filter(|v| v.is_finite() && *v > 0.0)?;
    Some((i, (value / 2.0).round().clamp(1.0, 5.0) as u8))
}

/// Whether `entry` is an EPUB 2 `<meta>` named `name`.
pub(crate) fn is_meta_named(entry: &MetadataEntry, name: &str) -> bool {
    entry.meta_name().map(str::trim) == Some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_opf(metadata: &str, version: &str) -> EpubMetadata {
        let opf = format!(
            r#"<package xmlns="http://www.idpf.org/2007/opf" version="{version}" unique-identifier="uid">
<metadata xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:opf="http://www.idpf.org/2007/opf">{metadata}</metadata>
<manifest/><spine/></package>"#
        );
        read(&Package::parse(&opf).unwrap())
    }

    #[test]
    fn calibre_style_epub2() {
        let m = read_opf(
            r#"
    <dc:title>
        Dune
        Messiah
    </dc:title>
    <dc:creator opf:role="aut" opf:file-as="Herbert, Frank">Frank Herbert</dc:creator>
    <dc:creator opf:role="aut">Brian Herbert</dc:creator>
    <dc:creator opf:role="edt">An Editor</dc:creator>
    <dc:identifier id="uid" opf:scheme="uuid">5f2b1c3e-6e0a-4c1b-9a43-3f1f3c0f9b11</dc:identifier>
    <dc:identifier opf:scheme="calibre">1234</dc:identifier>
    <dc:identifier opf:scheme="ISBN">978-0-441-01359-3</dc:identifier>
    <dc:language>fre</dc:language>
    <dc:description>&lt;p&gt;Paul &amp;amp; his &lt;b&gt;sister&lt;/b&gt;.&lt;/p&gt;&lt;p&gt;Second.&lt;/p&gt;</dc:description>
    <dc:subject>Science Fiction</dc:subject>
    <dc:subject>  </dc:subject>
    <dc:subject>Classics</dc:subject>
    <meta name="calibre:series" content="Dune"/>
    <meta name="calibre:series_index" content="2.0"/>
    <meta name="calibre:rating" content="8.0"/>
    <meta name="calibre:title_sort" content="Dune Messiah"/>"#,
            "2.0",
        );
        assert_eq!(m.title.as_deref(), Some("Dune Messiah"));
        assert_eq!(m.authors, vec!["Frank Herbert", "Brian Herbert"]);
        assert_eq!(m.isbn.as_deref(), Some("9780441013593"));
        assert_eq!(m.language.as_deref(), Some("fre"));
        assert_eq!(
            m.description.as_deref(),
            Some("Paul & his sister.\n\nSecond.")
        );
        assert_eq!(m.categories, vec!["Science Fiction", "Classics"]);
        assert_eq!(m.series.as_deref(), Some("Dune"));
        assert_eq!(m.series_index, Some(2.0));
        assert_eq!(m.rating, Some(4));
        assert_eq!(m.subtitle, None, "EPUB 2 has no reliable subtitle signal");
    }

    #[test]
    fn epub3_refinements() {
        let m = read_opf(
            r##"
    <dc:identifier id="uid">urn:isbn:9780441013593</dc:identifier>
    <dc:identifier id="isbn10">0441013597</dc:identifier>
    <meta refines="#isbn10" property="identifier-type" scheme="onix:codelist5">02</meta>
    <dc:title id="t1">Dune</dc:title>
    <meta refines="#t1" property="title-type">main</meta>
    <dc:title id="t2">The Desert Planet</dc:title>
    <meta refines="#t2" property="title-type">subtitle</meta>
    <dc:creator id="c1">Frank Herbert</dc:creator>
    <meta refines="#c1" property="role" scheme="marc:relators">aut</meta>
    <dc:creator id="c2">Some Translator</dc:creator>
    <meta refines="#c2" property="role" scheme="marc:relators">trl</meta>
    <meta property="belongs-to-collection" id="set">Box set</meta>
    <meta refines="#set" property="collection-type">set</meta>
    <meta property="belongs-to-collection" id="s">Dune Chronicles</meta>
    <meta refines="#s" property="collection-type">series</meta>
    <meta refines="#s" property="group-position">1</meta>
    <dc:language>en-US</dc:language>
    <dc:description>Plain text
with a line break.</dc:description>"##,
            "3.0",
        );
        assert_eq!(m.title.as_deref(), Some("Dune"));
        assert_eq!(m.subtitle.as_deref(), Some("The Desert Planet"));
        assert_eq!(m.authors, vec!["Frank Herbert"]);
        assert_eq!(
            m.isbn.as_deref(),
            Some("0441013597"),
            "an ISBN other than the unique identifier wins"
        );
        assert_eq!(m.series.as_deref(), Some("Dune Chronicles"));
        assert_eq!(m.series_index, Some(1.0));
        assert_eq!(m.language.as_deref(), Some("en-US"));
        assert_eq!(
            m.description.as_deref(),
            Some("Plain text\nwith a line break.")
        );
    }

    #[test]
    fn isbn_detection_ignores_other_schemes_and_bad_checksums() {
        let isbn = |identifiers: &str| read_opf(identifiers, "2.0").isbn;
        assert_eq!(
            isbn(r#"<dc:identifier id="uid">urn:uuid:9780441013593</dc:identifier>"#),
            None
        );
        assert_eq!(
            isbn(r#"<dc:identifier opf:scheme="AMAZON">0441013597</dc:identifier>"#),
            None
        );
        assert_eq!(
            isbn(r#"<dc:identifier>9780441013594</dc:identifier>"#),
            None,
            "bad checksum"
        );
        assert_eq!(
            isbn(r#"<dc:identifier>9780441013593</dc:identifier>"#).as_deref(),
            Some("9780441013593")
        );
        assert_eq!(
            isbn(r#"<dc:identifier id="uid" opf:scheme="ISBN">9780441013593</dc:identifier>"#)
                .as_deref(),
            Some("9780441013593"),
            "the unique identifier's ISBN is used when it is the only one"
        );
    }

    #[test]
    fn untyped_collections_and_ratings() {
        let m = read_opf(
            r##"<meta property="belongs-to-collection" id="c">Foundation</meta><meta refines="#c" property="group-position">2.5</meta>
<meta name="calibre:rating" content="0"/>"##,
            "3.0",
        );
        assert_eq!(m.series.as_deref(), Some("Foundation"));
        assert_eq!(m.series_index, Some(2.5));
        assert_eq!(m.rating, None, "0 means not rated");

        let rated = |content: &str| {
            read_opf(
                &format!(r#"<meta name="calibre:rating" content="{content}"/>"#),
                "2.0",
            )
            .rating
        };
        assert_eq!(rated("10"), Some(5));
        assert_eq!(rated("7"), Some(4));
        assert_eq!(rated("1"), Some(1));
        assert_eq!(rated("x"), None);
    }

    #[test]
    fn creators_without_roles_are_authors_and_empty_values_are_absent() {
        let m = read_opf("<dc:title>  </dc:title><dc:creator>A</dc:creator><dc:creator/><dc:creator>B</dc:creator>", "2.0");
        assert_eq!(m.title, None);
        assert_eq!(m.authors, vec!["A", "B"]);
        assert_eq!(m.description, None);
        assert_eq!(m.series, None);
    }
}
