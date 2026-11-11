//! The dev-mode EPUB inspector: a JSON summary of an EPUB's internal
//! structure (OPF path, cover declaration, manifest, spine, ZIP entries),
//! shown by the EPUB inspector in the UI to debug books that misbehave.
//! Read-only. The archive is opened once and the OPF parsed by
//! [`crate::epub`], so the inspector shows exactly what the rest of the
//! app sees.

use std::collections::HashSet;
use std::path::Path;

use serde_json::{json, Value};

use crate::epub::container::resolve_href;
use crate::epub::cover::{declared_cover, find_cover, CoverSource};
use crate::epub::EpubArchive;

/// [`inspect_epub`](super::inspect_epub): inspect the EPUB at `path` and
/// return `{ opfPath, opfDir, version, uniqueIdentifier, coverDeclaration,
/// coverFallback, manifest, spine, zipEntries }`.
/// - `coverDeclaration` is the cover the OPF declares (`method` `epub3` or
///   `epub2`) — the only one Librarium ever replaces or removes — or `null`;
/// - `coverFallback` is the image shown when nothing is declared (`method`
///   `epub2-href`, `guide` or `name`), or `null`;
/// - each manifest item says whether its file exists in the archive.
pub(crate) fn inspect_epub_file(path: &str) -> Result<Value, String> {
    if !path.to_lowercase().ends_with(".epub") {
        return Err("Not an EPUB file".to_string());
    }
    let mut epub = EpubArchive::open(Path::new(path))?;
    let zip_entries: Vec<Value> = {
        let zip = epub.zip();
        (0..zip.len())
            .filter_map(|i| {
                let entry = zip.by_index_raw(i).ok()?;
                // Lossy: an inspector shows even names that don't decode.
                let name = entry
                    .name()
                    .map(|n| n.into_owned())
                    .unwrap_or_else(|_| String::from_utf8_lossy(entry.name_raw()).into_owned());
                Some(json!({ "name": name, "size": entry.size() }))
            })
            .collect()
    };
    let names: HashSet<String> = epub
        .zip()
        .file_names()
        .filter_map(Result::ok)
        .map(|n| n.into_owned())
        .collect();
    let opf = epub.opf();
    let pkg = &opf.package;

    let describe = |cover: &crate::epub::cover::Cover| {
        let method = match cover.source {
            CoverSource::Epub3 => "epub3",
            CoverSource::Epub2 => "epub2",
            CoverSource::Epub2Href => "epub2-href",
            CoverSource::Guide => "guide",
            CoverSource::Named => "name",
        };
        let item = cover.item.map(|i| &pkg.manifest[i]);
        json!({
            "method": method,
            "href": item.map_or(cover.path.as_str(), |item| item.href.as_str()),
            "mediaType": cover.media_type,
            "itemId": item.map(|item| item.id.as_str()),
            "zipPath": cover.path,
        })
    };
    let declared = declared_cover(pkg, &opf.dir);
    let fallback = if declared.is_none() {
        find_cover(pkg, &opf.dir)
    } else {
        None
    };

    let manifest: Vec<Value> = pkg
        .manifest
        .iter()
        .map(|item| {
            let zip_path = resolve_href(&opf.dir, &item.href);
            let exists = zip_path.as_ref().is_some_and(|p| {
                names.contains(p) || names.iter().any(|n| n.eq_ignore_ascii_case(p))
            });
            json!({
                "id": item.id,
                "href": item.href,
                "mediaType": item.media_type,
                "properties": (!item.properties.is_empty()).then(|| item.properties.join(" ")),
                "zipPath": zip_path.unwrap_or_default(),
                "exists": exists,
            })
        })
        .collect();
    let spine: Vec<&str> = pkg.spine.iter().map(|s| s.idref.as_str()).collect();

    Ok(json!({
        "opfPath": opf.path,
        "opfDir": opf.dir,
        "version": pkg.version,
        "uniqueIdentifier": pkg.unique_identifier_entry().map(|i| pkg.metadata[i].text.trim().to_string()),
        "coverDeclaration": declared.as_ref().map(describe),
        "coverFallback": fallback.as_ref().map(describe),
        "manifest": manifest,
        "spine": spine,
        "zipEntries": zip_entries,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::epub::fixtures;

    #[test]
    fn inspects_an_epub3_book() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("foundation.epub");
        fixtures::write_epub3(&path);
        let report = inspect_epub_file(path.to_str().unwrap()).unwrap();
        assert_eq!(report["opfPath"], "OEBPS/package.opf");
        assert_eq!(report["opfDir"], "OEBPS");
        assert_eq!(report["version"], "3.0");
        assert_eq!(
            report["uniqueIdentifier"],
            "urn:uuid:0b6c1a9e-5f0d-4c1e-9a43-3f1f3c0f9b11"
        );
        assert_eq!(report["coverDeclaration"]["method"], "epub3");
        assert_eq!(report["coverDeclaration"]["href"], "images/cover.png");
        assert_eq!(report["coverDeclaration"]["itemId"], "cover-img");
        assert!(report["coverFallback"].is_null());
        let manifest = report["manifest"].as_array().unwrap();
        assert_eq!(manifest.len(), 4);
        assert_eq!(manifest[1]["zipPath"], "OEBPS/images/cover.png");
        assert_eq!(manifest[1]["properties"], "cover-image");
        assert!(manifest.iter().all(|m| m["exists"] == true));
        assert_eq!(report["spine"], json!(["c1"]));
        assert_eq!(report["zipEntries"][0]["name"], "mimetype");
    }

    #[test]
    fn reports_display_only_covers_and_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plain.epub");
        let opf = r#"<package xmlns="http://www.idpf.org/2007/opf" version="2.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>T</dc:title></metadata>
<manifest><item id="c" href="Images/Cover.JPG" media-type="image/jpeg"/><item id="gone" href="missing.xhtml" media-type="application/xhtml+xml"/></manifest><spine/></package>"#;
        fixtures::write_epub(
            &path,
            "content.opf",
            opf,
            &[("Images/Cover.JPG", fixtures::JPEG)],
        );
        let report = inspect_epub_file(path.to_str().unwrap()).unwrap();
        assert!(report["coverDeclaration"].is_null());
        assert_eq!(report["coverFallback"]["method"], "name");
        assert_eq!(report["manifest"][0]["exists"], true);
        assert_eq!(report["manifest"][1]["exists"], false);
        assert!(inspect_epub_file("/tmp/x.pdf").is_err());
    }
}
