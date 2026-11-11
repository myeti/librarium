//! OPDS 1.2 documents (Atom): catalog feeds, entry documents and the
//! OpenSearch description.
//!
//! This is the format KOReader, Moon+ Reader, Librera, Aldiko, Thorium and
//! most other reading apps understand. Documents are written with quick-xml,
//! so every text node and attribute value is escaped; characters XML 1.0
//! forbids even escaped (control characters in a hand-edited title) are
//! dropped first, because a single one makes the whole feed unparsable.
//!
//! What an entry carries:
//!
//! | Element | From |
//! |---|---|
//! | `title`, `id` (`urn:uuid:<book id>`), `updated` and `published` (date added) | the index record |
//! | `author/name`, one per author (`"A & B"` is two) | `metadata.author` |
//! | `dc:language`, `dc:identifier` (`urn:isbn:…`) | `metadata.language`, `metadata.isbn` |
//! | `calibre_md:series`, `calibre_md:series_index` (Calibre's own OPDS vocabulary) | `metadata.series`, `metadata.seriesIndex` |
//! | `category term label`, one per category | `metadata.categories` |
//! | `summary type="text"` (HTML stripped) | `metadata.description` |
//! | links: `http://opds-spec.org/image` and `…/image/thumbnail`, `http://opds-spec.org/acquisition` (typed per format, with `length`), `alternate` (entry document), `related` (author and series feeds) | |
//!
//! Every feed has a feed-level `author` (Librarium), which makes entries of
//! unknown authorship valid Atom, plus `self`, `start`, `up` and `search`
//! links; paginated feeds add `first`/`previous`/`next`/`last` and the
//! OpenSearch counts.

use std::borrow::Cow;
use std::io;

use chrono::{DateTime, SecondsFormat, Utc};
use quick_xml::events::{BytesDecl, BytesText, Event};
use quick_xml::Writer;

use super::catalog::{Entry, PageInfo};
use super::files::{format_label, format_mime, image_mime_from_name};
use super::urls;

const ATOM_NS: &str = "http://www.w3.org/2005/Atom";
const DC_NS: &str = "http://purl.org/dc/terms/";
const OPDS_NS: &str = "http://opds-spec.org/2010/catalog";
const OPENSEARCH_NS: &str = "http://a9.com/-/spec/opensearch/1.1/";
const CALIBRE_NS: &str = "http://calibre.kovidgoyal.net/2009/metadata";
/// `thr:count` on navigation links: how many books are behind them.
const THREAD_NS: &str = "http://purl.org/syndication/thread/1.0";

pub(super) const REL_SUBSECTION: &str = "subsection";
pub(super) const REL_SORT_NEW: &str = "http://opds-spec.org/sort/new";
const REL_ACQUISITION: &str = "http://opds-spec.org/acquisition";
const REL_IMAGE: &str = "http://opds-spec.org/image";
const REL_THUMBNAIL: &str = "http://opds-spec.org/image/thumbnail";

/// Who the feeds are by (Atom requires an author at feed or entry level).
const FEED_AUTHOR: &str = "Librarium";

// ── Document model ───────────────────────────────────────────────────────────

/// What a feed lists: other feeds, or books.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Navigation,
    Acquisition,
}

impl Kind {
    pub fn media_type(self) -> &'static str {
        match self {
            Kind::Navigation => urls::NAVIGATION_TYPE,
            Kind::Acquisition => urls::ACQUISITION_TYPE,
        }
    }
}

/// An entry of a navigation feed: a link to another feed.
#[derive(Debug, Clone)]
pub(super) struct NavEntry {
    pub id: String,
    pub title: String,
    pub href: String,
    /// What the linked feed lists.
    pub kind: Kind,
    /// [`REL_SUBSECTION`], or [`REL_SORT_NEW`] for "Recently added".
    pub rel: &'static str,
    /// A short description (`"12 books"`).
    pub content: Option<String>,
    /// How many books the linked feed holds (`thr:count`).
    pub count: Option<usize>,
}

/// The entries of a feed.
#[derive(Debug)]
pub(super) enum Entries<'a> {
    Navigation(Vec<NavEntry>),
    Books(Vec<&'a Entry>),
}

/// A catalog feed (see the module docs).
#[derive(Debug)]
pub(super) struct Feed<'a> {
    pub id: String,
    pub title: String,
    pub updated: DateTime<Utc>,
    /// The feed's path, without query.
    pub path: String,
    /// Query parameters every link to this feed keeps (`q`, a non-default
    /// `limit`); `page` is added per link.
    pub params: Vec<(&'static str, String)>,
    /// The parent feed; `None` for the root.
    pub up: Option<(String, Kind)>,
    /// Set for paginated feeds.
    pub page: Option<PageInfo>,
    pub entries: Entries<'a>,
}

impl Feed<'_> {
    fn kind(&self) -> Kind {
        match self.entries {
            Entries::Navigation(_) => Kind::Navigation,
            Entries::Books(_) => Kind::Acquisition,
        }
    }

    /// This feed's URL at `page` (page 1 is the bare URL).
    fn href(&self, page: Option<usize>) -> String {
        let page = page
            .filter(|&p| p > 1)
            .map(|p| p.to_string())
            .unwrap_or_default();
        let mut params: Vec<(&str, &str)> =
            self.params.iter().map(|(k, v)| (*k, v.as_str())).collect();
        params.push(("page", &page));
        urls::with_query(&self.path, &params)
    }
}

// ── Writing ──────────────────────────────────────────────────────────────────

type XmlWriter = Writer<Vec<u8>>;

/// The namespaces declared on every root element.
fn namespaces() -> [(&'static str, &'static str); 6] {
    [
        ("xmlns", ATOM_NS),
        ("xmlns:dc", DC_NS),
        ("xmlns:opds", OPDS_NS),
        ("xmlns:opensearch", OPENSEARCH_NS),
        ("xmlns:calibre_md", CALIBRE_NS),
        ("xmlns:thr", THREAD_NS),
    ]
}

fn new_document() -> io::Result<XmlWriter> {
    let mut w = Writer::new_with_indent(Vec::new(), b' ', 1);
    w.write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)))?;
    Ok(w)
}

/// A catalog feed document.
pub(super) fn feed(feed: &Feed) -> io::Result<Vec<u8>> {
    let mut w = new_document()?;
    w.create_element("feed")
        .with_attributes(namespaces())
        .write_inner_content(|w| {
            text(w, "id", &feed.id)?;
            text(w, "title", &feed.title)?;
            text(w, "updated", &timestamp(feed.updated))?;
            w.create_element("author")
                .write_inner_content(|w| text(w, "name", FEED_AUTHOR))?;

            let kind = feed.kind();
            link(
                w,
                "self",
                &feed.href(feed.page.map(|p| p.number)),
                kind.media_type(),
                &[],
            )?;
            link(w, "start", urls::ROOT, urls::NAVIGATION_TYPE, &[])?;
            if let Some((href, up_kind)) = &feed.up {
                link(w, "up", href, up_kind.media_type(), &[])?;
            }
            link(w, "search", urls::OPENSEARCH, urls::OPENSEARCH_TYPE, &[])?;
            link(w, "search", &urls::search_template(), urls::ATOM_TYPE, &[])?;

            if let Some(page) = feed.page {
                link(w, "first", &feed.href(None), kind.media_type(), &[])?;
                if let Some(previous) = page.previous() {
                    link(
                        w,
                        "previous",
                        &feed.href(Some(previous)),
                        kind.media_type(),
                        &[],
                    )?;
                }
                if let Some(next) = page.next() {
                    link(w, "next", &feed.href(Some(next)), kind.media_type(), &[])?;
                }
                link(
                    w,
                    "last",
                    &feed.href(Some(page.last())),
                    kind.media_type(),
                    &[],
                )?;
                text(w, "opensearch:totalResults", &page.total.to_string())?;
                text(w, "opensearch:itemsPerPage", &page.size.to_string())?;
                text(w, "opensearch:startIndex", &page.start_index().to_string())?;
            }

            match &feed.entries {
                Entries::Navigation(entries) => {
                    for entry in entries {
                        nav_entry(w, entry, feed.updated)?;
                    }
                }
                Entries::Books(entries) => {
                    for entry in entries {
                        w.create_element("entry")
                            .write_inner_content(|w| book(w, entry))?;
                    }
                }
            }
            Ok(())
        })?;
    Ok(w.into_inner())
}

/// A standalone entry document (`/opds/books/{id}`).
pub(super) fn entry_document(entry: &Entry) -> io::Result<Vec<u8>> {
    let mut w = new_document()?;
    w.create_element("entry")
        .with_attributes(namespaces())
        .write_inner_content(|w| book(w, entry))?;
    Ok(w.into_inner())
}

/// The OpenSearch description of the search feed (`/opds/opensearch.xml`).
pub(super) fn opensearch_description() -> io::Result<Vec<u8>> {
    let mut w = new_document()?;
    let template = urls::search_template();
    w.create_element("OpenSearchDescription")
        .with_attribute(("xmlns", OPENSEARCH_NS))
        .write_inner_content(|w| {
            text(w, "ShortName", "Librarium")?;
            text(
                w,
                "Description",
                "Search the library by title, author or series",
            )?;
            text(w, "InputEncoding", "UTF-8")?;
            text(w, "OutputEncoding", "UTF-8")?;
            // Plain Atom first: some clients match the type exactly.
            for media_type in [urls::ATOM_TYPE, urls::ACQUISITION_TYPE] {
                w.create_element("Url")
                    .with_attribute(("type", media_type))
                    .with_attribute(("template", template.as_str()))
                    .write_empty()?;
            }
            Ok(())
        })?;
    Ok(w.into_inner())
}

fn nav_entry(w: &mut XmlWriter, entry: &NavEntry, updated: DateTime<Utc>) -> io::Result<()> {
    w.create_element("entry").write_inner_content(|w| {
        text(w, "title", &entry.title)?;
        text(w, "id", &entry.id)?;
        text(w, "updated", &timestamp(updated))?;
        if let Some(content) = &entry.content {
            w.create_element("content")
                .with_attribute(("type", "text"))
                .write_text_content(BytesText::new(&xml_safe(content)))?;
        }
        let count = entry.count.map(|c| c.to_string());
        let extra: Vec<(&str, &str)> = count.iter().map(|c| ("thr:count", c.as_str())).collect();
        link(w, entry.rel, &entry.href, entry.kind.media_type(), &extra)
    })?;
    Ok(())
}

/// The content of a book's `<entry>`.
fn book(w: &mut XmlWriter, entry: &Entry) -> io::Result<()> {
    let book = &entry.book;
    let meta = &book.metadata;
    let id = book.id.as_str();

    text(w, "title", &entry.title)?;
    text(w, "id", &entry_id(id))?;
    text(w, "updated", &timestamp(entry.added))?;
    text(w, "published", &timestamp(entry.added))?;
    for author in &entry.authors {
        w.create_element("author")
            .write_inner_content(|w| text(w, "name", author))?;
    }
    if let Some(language) = non_empty(&meta.language) {
        text(w, "dc:language", language)?;
    }
    if let Some(isbn) = non_empty(&meta.isbn) {
        text(w, "dc:identifier", &format!("urn:isbn:{isbn}"))?;
    }
    if let Some(series) = entry.series() {
        text(w, "calibre_md:series", series)?;
        if let Some(index) = meta.series_index.filter(|i| i.is_finite()) {
            // `2` rather than `2.0`; `2.5` stays.
            text(w, "calibre_md:series_index", &index.to_string())?;
        }
    }
    for category in meta
        .categories
        .iter()
        .map(|c| c.trim())
        .filter(|c| !c.is_empty())
    {
        let category = xml_safe(category);
        w.create_element("category")
            .with_attribute(("term", category.as_ref()))
            .with_attribute(("label", category.as_ref()))
            .write_empty()?;
    }
    if let Some(summary) = &entry.summary {
        w.create_element("summary")
            .with_attribute(("type", "text"))
            .write_text_content(BytesText::new(&xml_safe(summary)))?;
    }

    if let Some(cover) = entry.cover_file() {
        link(
            w,
            REL_IMAGE,
            &urls::cover(id),
            image_mime_from_name(cover),
            &[],
        )?;
    }
    if let Some(thumbnail) = entry.thumbnail_file() {
        link(
            w,
            REL_THUMBNAIL,
            &urls::thumbnail(id),
            image_mime_from_name(thumbnail),
            &[],
        )?;
    }
    let length = book.size_bytes.to_string();
    let mut acquisition = vec![("title", format_label(&book.format))];
    if book.size_bytes > 0 {
        acquisition.push(("length", length.as_str()));
    }
    link(
        w,
        REL_ACQUISITION,
        &urls::download(id),
        format_mime(&book.format),
        &acquisition,
    )?;
    link(w, "alternate", &urls::entry(id), urls::ENTRY_TYPE, &[])?;
    for author in &entry.authors {
        link(
            w,
            "related",
            &urls::author(author),
            urls::ACQUISITION_TYPE,
            &[("title", author.as_str())],
        )?;
    }
    if let Some(series) = entry.series() {
        link(
            w,
            "related",
            &urls::series(series),
            urls::ACQUISITION_TYPE,
            &[("title", series)],
        )?;
    }
    Ok(())
}

/// `<name>text</name>`, escaped.
fn text(w: &mut XmlWriter, name: &str, value: &str) -> io::Result<()> {
    w.create_element(name)
        .write_text_content(BytesText::new(&xml_safe(value)))?;
    Ok(())
}

/// `<link rel href type …/>`; `extra` attributes are escaped too.
fn link(
    w: &mut XmlWriter,
    rel: &str,
    href: &str,
    media_type: &str,
    extra: &[(&str, &str)],
) -> io::Result<()> {
    let mut element = w
        .create_element("link")
        .with_attribute(("rel", rel))
        .with_attribute(("href", xml_safe(href).as_ref()))
        .with_attribute(("type", media_type));
    for (name, value) in extra {
        element = element.with_attribute((*name, xml_safe(value).as_ref()));
    }
    element.write_empty()?;
    Ok(())
}

/// A book's Atom id: `urn:uuid:<id>` (ids are UUIDs), or a Librarium URN
/// for an id that isn't one.
fn entry_id(id: &str) -> String {
    match uuid::Uuid::parse_str(id) {
        Ok(uuid) => format!("urn:uuid:{uuid}"),
        Err(_) => format!("urn:librarium:book:{}", urls::encode(id)),
    }
}

/// RFC 3339, to the second, in UTC (`2026-01-31T12:00:00Z`).
fn timestamp(date: DateTime<Utc>) -> String {
    date.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn non_empty(value: &Option<String>) -> Option<&str> {
    value.as_deref().map(str::trim).filter(|v| !v.is_empty())
}

/// `text` without the characters XML 1.0 doesn't allow at all, not even as
/// character references: C0 controls other than tab, line feed and carriage
/// return, and U+FFFE/U+FFFF.
fn xml_safe(text: &str) -> Cow<'_, str> {
    let allowed = |c: char| {
        matches!(c, '\t' | '\n' | '\r') || (c >= ' ' && c != '\u{FFFE}' && c != '\u{FFFF}')
    };
    if text.chars().all(allowed) {
        Cow::Borrowed(text)
    } else {
        Cow::Owned(text.chars().filter(|&c| allowed(c)).collect())
    }
}

// ── Test helper: a strict parser ─────────────────────────────────────────────

/// Parses what this module writes back into a small tree, with namespaces
/// resolved, so tests check documents as a client sees them.
#[cfg(test)]
pub(super) mod xml_tree {
    use quick_xml::events::Event;
    use quick_xml::name::ResolveResult;
    use quick_xml::{NsReader, XmlVersion};

    pub const ATOM: &str = super::ATOM_NS;
    pub const DC: &str = super::DC_NS;
    pub const OPENSEARCH: &str = super::OPENSEARCH_NS;
    pub const CALIBRE: &str = super::CALIBRE_NS;

    #[derive(Debug, Clone, Default)]
    pub struct Node {
        /// Namespace URI ("" when unbound).
        pub ns: String,
        pub name: String,
        /// Qualified names as written, values unescaped.
        pub attrs: Vec<(String, String)>,
        /// Text content, references resolved.
        pub text: String,
        pub children: Vec<Node>,
    }

    impl Node {
        pub fn attr(&self, name: &str) -> Option<&str> {
            self.attrs
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.as_str())
        }

        pub fn all<'a>(
            &'a self,
            ns: &'a str,
            name: &'a str,
        ) -> impl Iterator<Item = &'a Node> + 'a {
            self.children
                .iter()
                .filter(move |c| c.ns == ns && c.name == name)
        }

        pub fn first(&self, ns: &str, name: &str) -> Option<&Node> {
            self.children.iter().find(|c| c.ns == ns && c.name == name)
        }

        /// Text of the first `ns:name` child.
        pub fn text_of(&self, ns: &str, name: &str) -> Option<&str> {
            self.first(ns, name).map(|n| n.text.as_str())
        }

        /// The Atom `<link>` children with this `rel`.
        pub fn links<'a>(&'a self, rel: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
            self.all(ATOM, "link")
                .filter(move |l| l.attr("rel") == Some(rel))
        }

        /// The `href` of the first Atom link with this `rel`.
        pub fn href(&self, rel: &str) -> Option<&str> {
            self.all(ATOM, "link")
                .find(|l| l.attr("rel") == Some(rel))
                .and_then(|l| l.attr("href"))
        }

        pub fn entries(&self) -> Vec<&Node> {
            self.all(ATOM, "entry").collect()
        }
    }

    /// Parse a whole document; panics (failing the test) on anything
    /// malformed: bad nesting, an unknown prefix, a bad reference, trailing
    /// garbage, not exactly one root.
    pub fn parse(bytes: &[u8]) -> Node {
        let text = std::str::from_utf8(bytes).expect("UTF-8");
        let mut reader = NsReader::from_str(text);
        let mut stack: Vec<Node> = vec![Node::default()];
        loop {
            let (resolved, event) = reader.read_resolved_event().expect("well-formed XML");
            let ns = match resolved {
                ResolveResult::Bound(ns) => ns.as_ref().to_string(),
                ResolveResult::Unbound => String::new(),
                ResolveResult::Unknown(prefix) => panic!("unknown namespace prefix {prefix}"),
            };
            let empty = matches!(event, Event::Empty(_));
            match event {
                Event::Start(e) | Event::Empty(e) => {
                    let mut node = Node {
                        ns,
                        name: e.local_name().as_ref().to_string(),
                        ..Default::default()
                    };
                    for attr in e.attributes() {
                        let attr = attr.expect("well-formed attribute");
                        let key = attr.key.as_ref().to_string();
                        let value = attr
                            .normalized_value(XmlVersion::Explicit1_0)
                            .expect("valid references")
                            .into_owned();
                        node.attrs.push((key, value));
                    }
                    if empty {
                        stack.last_mut().unwrap().children.push(node);
                    } else {
                        stack.push(node);
                    }
                }
                Event::End(_) => {
                    let node = stack.pop().unwrap();
                    stack.last_mut().expect("balanced").children.push(node);
                }
                Event::Text(t) => stack.last_mut().unwrap().text.push_str(&t.xml10_content()),
                Event::CData(t) => stack.last_mut().unwrap().text.push_str(&t.into_inner()),
                Event::GeneralRef(r) => {
                    let resolved = match r.resolve_char_ref().expect("valid character reference") {
                        Some(c) => c.to_string(),
                        None => {
                            let name = r.into_inner();
                            quick_xml::escape::resolve_predefined_entity(&name)
                                .expect("predefined entity")
                                .to_string()
                        }
                    };
                    stack.last_mut().unwrap().text.push_str(&resolved);
                }
                Event::Eof => break,
                _ => {}
            }
        }
        let mut document = stack.pop().unwrap();
        assert!(stack.is_empty(), "unclosed elements");
        assert_eq!(document.children.len(), 1, "exactly one root element");
        assert!(document.text.trim().is_empty(), "no text outside the root");
        document.children.remove(0)
    }
}

#[cfg(test)]
mod tests {
    use super::xml_tree::{parse, ATOM, CALIBRE, DC, OPENSEARCH};
    use super::*;
    use crate::commands::opds::catalog::{paginate, Catalog, PageRequest};
    use crate::models::book::{Book, BookFormat, BookMetadata};
    use std::collections::HashMap;
    use std::path::PathBuf;

    const HOSTILE: &str =
        "<script>alert(\"x\")</script> & 'q' ]]> \u{1}\u{8}\u{B}\u{1F}\u{FFFE} \r\n\t🐉";

    fn hostile_book() -> Book {
        Book {
            id: "11111111-1111-1111-1111-111111111111".into(),
            path: "/lib/x.azw3".into(),
            filename: "x.azw3".into(),
            format: BookFormat::Azw3,
            size_bytes: 1234,
            date_added: "2026-01-02T03:04:05Z".into(),
            cover_path: None,
            cover_version: None,
            favorite: false,
            metadata: BookMetadata {
                title: HOSTILE.into(),
                // Two authors (" & " separates them), one with markup in its name.
                author: "Jean \"<Dupont>\" & Plain Author".into(),
                series: Some(format!("S{HOSTILE}")),
                series_index: Some(2.0),
                language: Some("fr".into()),
                isbn: Some("9782070612758".into()),
                description: Some("<p>Un <b>conte</b> &amp; plus</p>".into()),
                categories: vec![HOSTILE.into(), "  ".into(), "Conte".into()],
                ..Default::default()
            },
        }
    }

    fn catalog() -> Catalog {
        let covers = HashMap::from([(hostile_book().id, PathBuf::from("/lib/.covers/x.png"))]);
        let updated = DateTime::parse_from_rfc3339("2026-05-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        Catalog::from_books(vec![hostile_book()], &covers, &HashMap::new(), updated)
    }

    /// What the hostile text reads as after the round trip: the forbidden
    /// characters are gone, everything else intact (XML turns CR LF into LF).
    fn hostile_read_back() -> String {
        HOSTILE.replace(['\u{1}', '\u{8}', '\u{B}', '\u{1F}', '\u{FFFE}'], "")
    }

    fn acquisition_feed<'a>(
        catalog: &'a Catalog,
        page: PageInfo,
        entries: Vec<&'a Entry>,
    ) -> Feed<'a> {
        Feed {
            id: "urn:librarium:all".into(),
            title: format!("Search {HOSTILE}"),
            updated: catalog.updated,
            path: urls::SEARCH.into(),
            params: vec![("q", HOSTILE.to_string())],
            up: Some((urls::ROOT.into(), Kind::Navigation)),
            page: Some(page),
            entries: Entries::Books(entries),
        }
    }

    #[test]
    fn hostile_text_is_escaped_and_reads_back() {
        let catalog = catalog();
        let all = catalog.all();
        let (entries, page) = paginate(
            &all,
            PageRequest {
                number: 1,
                size: 10,
            },
        );
        let xml = feed(&acquisition_feed(&catalog, page, entries.to_vec())).unwrap();
        let text = String::from_utf8(xml.clone()).unwrap();
        assert!(!text.contains("<script>"), "{text}");
        assert!(!text.contains('\u{1}'));

        let root = parse(&xml);
        assert_eq!((root.ns.as_str(), root.name.as_str()), (ATOM, "feed"));
        assert_eq!(
            root.text_of(ATOM, "title"),
            Some(format!("Search {}", hostile_read_back()).as_str())
        );
        let entry = root.entries()[0];
        assert_eq!(
            entry.text_of(ATOM, "title"),
            Some(hostile_read_back().as_str())
        );
        let authors: Vec<&str> = entry
            .all(ATOM, "author")
            .filter_map(|a| a.text_of(ATOM, "name"))
            .collect();
        assert_eq!(authors, ["Jean \"<Dupont>\"", "Plain Author"]);
        let categories: Vec<&str> = entry
            .all(ATOM, "category")
            .filter_map(|c| c.attr("term"))
            .collect();
        assert_eq!(
            categories,
            [hostile_read_back().as_str(), "Conte"],
            "attributes too; blank categories are skipped"
        );
        let related: Vec<&str> = entry
            .links("related")
            .filter_map(|l| l.attr("title"))
            .collect();
        assert_eq!(
            related,
            [
                "Jean \"<Dupont>\"",
                "Plain Author",
                format!("S{}", hostile_read_back()).as_str()
            ]
        );
        // The search terms survive in the self link (percent-encoded).
        let self_href = root.href("self").unwrap();
        assert!(
            self_href.starts_with("/opds/search?q=%3Cscript%3E"),
            "{self_href}"
        );
    }

    #[test]
    fn book_entries_carry_the_metadata_and_links() {
        let catalog = catalog();
        let entry = catalog.all()[0];
        let root = parse(&entry_document(entry).unwrap());
        assert_eq!(root.name, "entry");
        assert_eq!(
            root.text_of(ATOM, "id"),
            Some("urn:uuid:11111111-1111-1111-1111-111111111111")
        );
        assert_eq!(root.text_of(ATOM, "updated"), Some("2026-01-02T03:04:05Z"));
        assert_eq!(
            root.text_of(ATOM, "published"),
            Some("2026-01-02T03:04:05Z")
        );
        assert_eq!(root.text_of(DC, "language"), Some("fr"));
        assert_eq!(
            root.text_of(DC, "identifier"),
            Some("urn:isbn:9782070612758")
        );
        assert_eq!(
            root.text_of(CALIBRE, "series"),
            Some(format!("S{}", hostile_read_back()).as_str())
        );
        assert_eq!(root.text_of(CALIBRE, "series_index"), Some("2"));
        let summary = root.first(ATOM, "summary").unwrap();
        assert_eq!(summary.attr("type"), Some("text"));
        assert_eq!(summary.text, "Un conte & plus", "plain text");

        let acquisition = root
            .links("http://opds-spec.org/acquisition")
            .next()
            .unwrap();
        assert_eq!(
            acquisition.attr("href"),
            Some("/opds/publications/11111111-1111-1111-1111-111111111111/download")
        );
        assert_eq!(
            acquisition.attr("type"),
            Some("application/vnd.amazon.mobi8-ebook")
        );
        assert_eq!(acquisition.attr("length"), Some("1234"));
        let image = root.links("http://opds-spec.org/image").next().unwrap();
        assert_eq!(image.attr("type"), Some("image/png"));
        assert!(image.attr("href").unwrap().ends_with("/cover"));
        let thumbnail = root
            .links("http://opds-spec.org/image/thumbnail")
            .next()
            .unwrap();
        assert!(thumbnail.attr("href").unwrap().ends_with("/thumbnail"));
        assert_eq!(
            root.links("related").count(),
            3,
            "two authors and the series"
        );
        assert_eq!(
            root.links("alternate").next().unwrap().attr("type"),
            Some(urls::ENTRY_TYPE)
        );
    }

    #[test]
    fn feeds_have_the_required_elements_and_pagination_links() {
        let catalog = catalog();
        let all = catalog.all();
        let three = [all[0]; 3];
        let (entries, page) = paginate(&three, PageRequest { number: 2, size: 1 });
        let mut f = acquisition_feed(&catalog, page, entries.to_vec());
        f.params = vec![("q", "a b".into()), ("limit", "1".into())];
        let root = parse(&feed(&f).unwrap());

        for required in ["id", "title", "updated"] {
            assert!(
                root.text_of(ATOM, required).is_some_and(|t| !t.is_empty()),
                "{required}"
            );
        }
        assert_eq!(
            root.first(ATOM, "author")
                .and_then(|a| a.text_of(ATOM, "name")),
            Some("Librarium")
        );
        assert_eq!(
            root.href("self"),
            Some("/opds/search?q=a%20b&limit=1&page=2")
        );
        assert_eq!(root.href("first"), Some("/opds/search?q=a%20b&limit=1"));
        assert_eq!(root.href("previous"), Some("/opds/search?q=a%20b&limit=1"));
        assert_eq!(
            root.href("next"),
            Some("/opds/search?q=a%20b&limit=1&page=3")
        );
        assert_eq!(
            root.href("last"),
            Some("/opds/search?q=a%20b&limit=1&page=3")
        );
        assert_eq!(root.href("start"), Some("/opds"));
        assert_eq!(root.href("up"), Some("/opds"));
        let searches: Vec<(&str, &str)> = root
            .links("search")
            .map(|l| (l.attr("type").unwrap(), l.attr("href").unwrap()))
            .collect();
        assert_eq!(
            searches,
            [
                (urls::OPENSEARCH_TYPE, "/opds/opensearch.xml"),
                ("application/atom+xml", "/opds/search?q={searchTerms}"),
            ]
        );
        assert_eq!(root.text_of(OPENSEARCH, "totalResults"), Some("3"));
        assert_eq!(root.text_of(OPENSEARCH, "itemsPerPage"), Some("1"));
        assert_eq!(root.text_of(OPENSEARCH, "startIndex"), Some("2"));
        for entry in root.entries() {
            for required in ["id", "title", "updated"] {
                assert!(entry.text_of(ATOM, required).is_some(), "entry {required}");
            }
        }
    }

    #[test]
    fn navigation_feeds_link_to_subsections() {
        let updated = Utc::now();
        let f = Feed {
            id: "urn:librarium:root".into(),
            title: "Librarium".into(),
            updated,
            path: urls::ROOT.into(),
            params: vec![],
            up: None,
            page: None,
            entries: Entries::Navigation(vec![NavEntry {
                id: "urn:librarium:recent".into(),
                title: "Recently added".into(),
                href: urls::RECENT.into(),
                kind: Kind::Acquisition,
                rel: REL_SORT_NEW,
                content: Some("3 books".into()),
                count: Some(3),
            }]),
        };
        let root = parse(&feed(&f).unwrap());
        assert!(root.href("up").is_none() && root.href("next").is_none());
        assert_eq!(
            root.links("self").next().unwrap().attr("type"),
            Some(urls::NAVIGATION_TYPE)
        );
        let entry = root.entries()[0];
        assert_eq!(entry.text_of(ATOM, "content"), Some("3 books"));
        let link = entry.links(REL_SORT_NEW).next().unwrap();
        assert_eq!(link.attr("type"), Some(urls::ACQUISITION_TYPE));
        assert_eq!(link.attr("thr:count"), Some("3"));
    }

    #[test]
    fn opensearch_description_has_atom_templates() {
        let root = parse(&opensearch_description().unwrap());
        assert_eq!(
            (root.ns.as_str(), root.name.as_str()),
            (OPENSEARCH, "OpenSearchDescription")
        );
        assert_eq!(root.text_of(OPENSEARCH, "ShortName"), Some("Librarium"));
        let templates: Vec<(&str, &str)> = root
            .all(OPENSEARCH, "Url")
            .map(|u| (u.attr("type").unwrap(), u.attr("template").unwrap()))
            .collect();
        assert_eq!(
            templates,
            [
                ("application/atom+xml", "/opds/search?q={searchTerms}"),
                (urls::ACQUISITION_TYPE, "/opds/search?q={searchTerms}"),
            ]
        );
    }

    #[test]
    fn non_uuid_ids_get_a_librarium_urn() {
        assert_eq!(
            entry_id("11111111-1111-1111-1111-111111111111"),
            "urn:uuid:11111111-1111-1111-1111-111111111111"
        );
        assert_eq!(entry_id("a b"), "urn:librarium:book:a%20b");
    }
}
