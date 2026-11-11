//! The server's URL space and media types.
//!
//! Every path a feed links to is built here, next to the route patterns the
//! router registers, so the feeds and the router can't drift apart. Links are
//! path-absolute (`/opds/all`), which every OPDS client resolves against the
//! catalog's address: the server never needs to know the name or IP it was
//! reached by.

// ── Paths (OPDS 1.2, Atom) ───────────────────────────────────────────────────

/// Root navigation feed (Atom, or OPDS 2 JSON when asked for; see
/// `prefers_opds2`). The URL users type in their reading app.
pub(super) const ROOT: &str = "/opds";
pub(super) const ALL: &str = "/opds/all";
pub(super) const RECENT: &str = "/opds/recent";
pub(super) const AUTHORS: &str = "/opds/authors";
pub(super) const AUTHOR_ROUTE: &str = "/opds/authors/{name}";
pub(super) const SERIES: &str = "/opds/series";
pub(super) const SERIES_ROUTE: &str = "/opds/series/{name}";
pub(super) const SEARCH: &str = "/opds/search";
pub(super) const OPENSEARCH: &str = "/opds/opensearch.xml";
pub(super) const ENTRY_ROUTE: &str = "/opds/books/{id}";

// ── Paths (OPDS 2.0, JSON) ───────────────────────────────────────────────────

pub(super) const V2_ROOT: &str = "/opds/v2";
pub(super) const PUBLICATIONS: &str = "/opds/publications";
pub(super) const PUBLICATION_ROUTE: &str = "/opds/publications/{id}";

// ── Files (shared by both versions) ──────────────────────────────────────────

pub(super) const DOWNLOAD_ROUTE: &str = "/opds/publications/{id}/download";
pub(super) const COVER_ROUTE: &str = "/opds/publications/{id}/cover";
pub(super) const THUMBNAIL_ROUTE: &str = "/opds/publications/{id}/thumbnail";

// ── Media types ──────────────────────────────────────────────────────────────

pub(super) const NAVIGATION_TYPE: &str =
    "application/atom+xml;profile=opds-catalog;kind=navigation";
pub(super) const ACQUISITION_TYPE: &str =
    "application/atom+xml;profile=opds-catalog;kind=acquisition";
pub(super) const ENTRY_TYPE: &str = "application/atom+xml;type=entry;profile=opds-catalog";
pub(super) const OPENSEARCH_TYPE: &str = "application/opensearchdescription+xml";
/// What a `rel="search"` link to a templated Atom URL is typed as. Some
/// clients (KOReader among them) look for exactly this prefix.
pub(super) const ATOM_TYPE: &str = "application/atom+xml";
pub(super) const OPDS2_TYPE: &str = "application/opds+json";
pub(super) const OPDS2_PUBLICATION_TYPE: &str = "application/opds-publication+json";

// ── Builders ─────────────────────────────────────────────────────────────────

/// Acquisition feed of one author's books.
pub(super) fn author(name: &str) -> String {
    format!("{AUTHORS}/{}", encode(name))
}

/// Acquisition feed of one series.
pub(super) fn series(name: &str) -> String {
    format!("{SERIES}/{}", encode(name))
}

/// The Atom entry document of one book.
pub(super) fn entry(id: &str) -> String {
    format!("/opds/books/{}", encode(id))
}

/// The OPDS 2 publication of one book.
pub(super) fn publication(id: &str) -> String {
    format!("{PUBLICATIONS}/{}", encode(id))
}

pub(super) fn download(id: &str) -> String {
    format!("{PUBLICATIONS}/{}/download", encode(id))
}

pub(super) fn cover(id: &str) -> String {
    format!("{PUBLICATIONS}/{}/cover", encode(id))
}

pub(super) fn thumbnail(id: &str) -> String {
    format!("{PUBLICATIONS}/{}/thumbnail", encode(id))
}

/// The OpenSearch URL template of the Atom search feed. Only
/// `{searchTerms}`: clients that substitute nothing else (KOReader) would
/// send an optional parameter like `{startPage?}` literally.
pub(super) fn search_template() -> String {
    format!("{SEARCH}?q={{searchTerms}}")
}

/// The OPDS 2 (RFC 6570) search template.
pub(super) fn publications_search_template() -> String {
    format!("{PUBLICATIONS}{{?q}}")
}

/// `path?k=v&…`, with each value percent-encoded; empty values are left out.
pub(super) fn with_query(path: &str, params: &[(&str, &str)]) -> String {
    let mut out = path.to_string();
    let mut separator = '?';
    for (key, value) in params.iter().filter(|(_, v)| !v.is_empty()) {
        out.push(separator);
        out.push_str(key);
        out.push('=');
        out.push_str(&encode(value));
        separator = '&';
    }
    out
}

/// Percent-encode everything but RFC 3986 "unreserved" characters, so any
/// text (an author called `AC/DC`, a series with `?`, `#`, `&` or `%`) can be
/// one path segment or one query value, and decodes back to itself.
pub(super) fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_encoded_as_one_segment() {
        assert_eq!(author("AC/DC"), "/opds/authors/AC%2FDC");
        assert_eq!(
            series("Tintin & Milou #1?"),
            "/opds/series/Tintin%20%26%20Milou%20%231%3F"
        );
        assert_eq!(encode("été"), "%C3%A9t%C3%A9");
        assert_eq!(encode("a-b.c_d~"), "a-b.c_d~");
    }

    #[test]
    fn queries_skip_empty_values() {
        assert_eq!(
            with_query(SEARCH, &[("q", "le petit"), ("page", "2")]),
            "/opds/search?q=le%20petit&page=2"
        );
        assert_eq!(
            with_query(ALL, &[("page", ""), ("limit", "10")]),
            "/opds/all?limit=10"
        );
        assert_eq!(with_query(ALL, &[]), "/opds/all");
    }

    #[test]
    fn route_patterns_match_the_builders() {
        // The `{param}` patterns the router registers produce the same paths
        // as the builders, so a link always reaches its route.
        let fill =
            |pattern: &str, value: &str| pattern.replace("{name}", value).replace("{id}", value);
        assert_eq!(fill(AUTHOR_ROUTE, "x"), author("x"));
        assert_eq!(fill(SERIES_ROUTE, "x"), series("x"));
        assert_eq!(fill(ENTRY_ROUTE, "x"), entry("x"));
        assert_eq!(fill(PUBLICATION_ROUTE, "x"), publication("x"));
        assert_eq!(fill(DOWNLOAD_ROUTE, "x"), download("x"));
        assert_eq!(fill(COVER_ROUTE, "x"), cover("x"));
        assert_eq!(fill(THUMBNAIL_ROUTE, "x"), thumbnail("x"));
    }
}
