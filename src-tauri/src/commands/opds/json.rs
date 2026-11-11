//! OPDS 2.0 documents (JSON): the root navigation, the publications feed
//! (paginated, searchable) and single publications.
//!
//! The JSON catalog that existed before the Atom feeds; its URLs are kept.
//! Readium-based apps (Thorium) read it; most e-reader apps want OPDS 1.2.
//! serde_json does all the escaping.

use serde_json::{json, Value};

use super::catalog::{Entry, PageInfo, DEFAULT_PAGE_SIZE};
use super::files::{format_mime, image_mime_from_name};
use super::urls;

/// The root navigation (`/opds/v2`, or `/opds` when asked for JSON).
pub(super) fn root() -> Value {
    json!({
        "metadata": {"title": "Librarium"},
        "links": [
            {"rel": "self",  "href": urls::V2_ROOT, "type": urls::OPDS2_TYPE},
            {"rel": "start", "href": urls::V2_ROOT, "type": urls::OPDS2_TYPE},
            {"rel": "search", "href": urls::publications_search_template(), "type": urls::OPDS2_TYPE, "templated": true},
            {"rel": "alternate", "href": urls::ROOT, "type": urls::NAVIGATION_TYPE},
        ],
        "navigation": [{
            "title": "All books",
            "href": urls::PUBLICATIONS,
            "type": urls::OPDS2_TYPE,
            "rel": "subsection",
        }],
    })
}

/// One page of the publications feed: every book, or the results of
/// `query`. The links keep the query, and the page size when it isn't the
/// default.
pub(super) fn publications(entries: &[&Entry], page: PageInfo, query: Option<&str>) -> Value {
    let size = if page.size == DEFAULT_PAGE_SIZE {
        String::new()
    } else {
        page.size.to_string()
    };
    let href = |number: usize| {
        let number = if number > 1 {
            number.to_string()
        } else {
            String::new()
        };
        urls::with_query(
            urls::PUBLICATIONS,
            &[
                ("q", query.unwrap_or("")),
                ("limit", &size),
                ("page", &number),
            ],
        )
    };
    let link = |rel: &str, number: usize| json!({"rel": rel, "href": href(number), "type": urls::OPDS2_TYPE});

    let mut links = vec![link("self", page.number), link("first", 1)];
    if let Some(previous) = page.previous() {
        links.push(link("previous", previous));
    }
    if let Some(next) = page.next() {
        links.push(link("next", next));
    }
    links.push(link("last", page.last()));
    links.push(json!({
        "rel": "search", "href": urls::publications_search_template(), "type": urls::OPDS2_TYPE, "templated": true,
    }));

    json!({
        "metadata": {
            "title": if query.is_some() { "Search results" } else { "All books" },
            "numberOfItems": page.total,
            "itemsPerPage": page.size,
            "currentPage": page.number,
        },
        "links": links,
        "publications": entries.iter().map(|e| publication(e)).collect::<Vec<_>>(),
    })
}

/// One publication (also the body of `/opds/publications/{id}`).
pub(super) fn publication(entry: &Entry) -> Value {
    let book = &entry.book;
    let meta = &book.metadata;
    let mut metadata = json!({
        "@type": "http://schema.org/Book",
        "title": entry.title,
        "identifier": format!("urn:uuid:{}", book.id),
        "added": entry.added.to_rfc3339(),
    });
    if !entry.authors.is_empty() {
        metadata["author"] = entry.authors.iter().map(|a| json!({"name": a})).collect();
    }
    if let Some(language) = meta
        .language
        .as_deref()
        .map(str::trim)
        .filter(|l| !l.is_empty())
    {
        metadata["language"] = json!(language);
    }
    if let Some(summary) = &entry.summary {
        metadata["description"] = json!(summary);
    }
    let subjects: Vec<Value> = meta
        .categories
        .iter()
        .map(|c| c.trim())
        .filter(|c| !c.is_empty())
        .map(|c| json!({"name": c}))
        .collect();
    if !subjects.is_empty() {
        metadata["subject"] = json!(subjects);
    }
    if let Some(series) = entry.series() {
        let mut item = json!({"name": series});
        if let Some(position) = meta.series_index.filter(|i| i.is_finite()) {
            item["position"] = json!(position);
        }
        metadata["belongsTo"] = json!({"series": [item]});
    }

    let mut publication = json!({
        "metadata": metadata,
        "links": [
            {"rel": "self", "href": urls::publication(&book.id), "type": urls::OPDS2_PUBLICATION_TYPE},
            {"rel": "http://opds-spec.org/acquisition", "href": urls::download(&book.id), "type": format_mime(&book.format)},
        ],
    });
    let mut images = Vec::new();
    if let Some(cover) = entry.cover_file() {
        images.push(json!({"href": urls::cover(&book.id), "type": image_mime_from_name(cover)}));
    }
    if let Some(thumbnail) = entry.thumbnail_file() {
        images.push(
            json!({"href": urls::thumbnail(&book.id), "type": image_mime_from_name(thumbnail)}),
        );
    }
    if !images.is_empty() {
        publication["images"] = json!(images);
    }
    publication
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::opds::catalog::{paginate, Catalog, PageRequest};
    use crate::models::book::{Book, BookFormat, BookMetadata};
    use chrono::Utc;
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn catalog(n: usize) -> Catalog {
        let books = (0..n)
            .map(|i| Book {
                id: format!("id-{i}"),
                path: format!("/lib/{i}.cbz"),
                filename: format!("{i}.cbz"),
                format: BookFormat::Cbz,
                size_bytes: 1,
                date_added: "2026-01-01T00:00:00Z".into(),
                cover_path: None,
                cover_version: None,
                favorite: false,
                metadata: BookMetadata {
                    title: format!("Tome \"{i}\""),
                    author: "A & B".into(),
                    series: Some("Saga".into()),
                    series_index: Some(i as f32 + 0.5),
                    categories: vec!["BD".into()],
                    ..Default::default()
                },
            })
            .collect();
        let thumbs = HashMap::from([(
            "id-0".to_string(),
            PathBuf::from("/lib/.covers/thumbs/id-0.jpg"),
        )]);
        Catalog::from_books(books, &HashMap::new(), &thumbs, Utc::now())
    }

    #[test]
    fn publications_are_paginated_with_links() {
        let c = catalog(5);
        let all = c.all();
        let (entries, page) = paginate(&all, PageRequest { number: 2, size: 2 });
        let feed = publications(entries, page, Some("tome"));
        assert_eq!(feed["metadata"]["numberOfItems"], 5);
        assert_eq!(feed["metadata"]["currentPage"], 2);
        let href = |rel: &str| {
            feed["links"]
                .as_array()
                .unwrap()
                .iter()
                .find(|l| l["rel"] == rel)
                .map(|l| l["href"].as_str().unwrap().to_string())
        };
        assert_eq!(
            href("self").as_deref(),
            Some("/opds/publications?q=tome&limit=2&page=2")
        );
        assert_eq!(
            href("previous").as_deref(),
            Some("/opds/publications?q=tome&limit=2")
        );
        assert_eq!(
            href("next").as_deref(),
            Some("/opds/publications?q=tome&limit=2&page=3")
        );
        assert_eq!(
            href("last").as_deref(),
            Some("/opds/publications?q=tome&limit=2&page=3")
        );
        assert_eq!(href("search").as_deref(), Some("/opds/publications{?q}"));
        assert_eq!(feed["publications"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn publications_carry_metadata_and_images() {
        let c = catalog(1);
        let p = publication(c.all()[0]);
        assert_eq!(p["metadata"]["title"], "Tome \"0\"");
        assert_eq!(
            p["metadata"]["author"],
            json!([{"name": "A"}, {"name": "B"}])
        );
        assert_eq!(
            p["metadata"]["belongsTo"]["series"][0],
            json!({"name": "Saga", "position": 0.5})
        );
        assert_eq!(p["metadata"]["subject"], json!([{"name": "BD"}]));
        assert_eq!(p["links"][1]["type"], "application/vnd.comicbook+zip");
        // Only a thumbnail exists: it stands in for the cover too.
        assert_eq!(p["images"].as_array().unwrap().len(), 2);
        assert_eq!(p["images"][0]["href"], "/opds/publications/id-0/cover");
        assert_eq!(p["images"][0]["type"], "image/jpeg");
    }
}
