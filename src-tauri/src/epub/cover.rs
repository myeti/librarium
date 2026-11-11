//! The cover image: finding it to show it ([`find_cover`]), and changing the
//! one the OPF declares ([`set_cover`], [`remove_cover`]).
//!
//! An OPF *declares* its cover in two standard ways:
//! - EPUB 3: the manifest item with `properties="cover-image"`;
//! - EPUB 2: `<meta name="cover" content="<manifest item id>"/>`.
//!
//! Many files do neither, so to *show* a cover Librarium also accepts weaker
//! signals: a `<meta name="cover">` naming the image's href instead of its
//! id, a guide `<reference type="cover">` pointing straight at an image, an
//! image item whose id is `cover` or whose file is `cover.*`. Those guesses
//! are only ever used for display. Replacing or removing a cover touches
//! nothing but a declared cover, so an interior illustration is never
//! overwritten or deleted because it happens to look like one.

use std::collections::BTreeSet;

use super::container::resolve_href;
use super::edit::{
    attribute_removal, attribute_replacement, insert_after, insert_into, meta_qname,
};
use super::metadata::{is_meta_named, refinements};
use super::opf::{EntryKind, Package};
use super::xml::{escape_attr, removal_range, Splicer};

/// How a cover was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CoverSource {
    /// EPUB 3 `properties="cover-image"`.
    Epub3,
    /// EPUB 2 `<meta name="cover" content="<item id>"/>`.
    Epub2,
    /// Display only: `<meta name="cover">` naming the image's href.
    Epub2Href,
    /// Display only: a guide `<reference type="cover">` pointing at an image.
    Guide,
    /// Display only: an image item with id `cover`, or a file `cover.*`.
    Named,
}

/// A cover image found in the OPF.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Cover {
    /// Archive path of the image (`OEBPS/images/cover.jpg`).
    pub path: String,
    /// Its manifest item (index into [`Package::manifest`]), if it has one.
    pub item: Option<usize>,
    /// The declared media type (empty if unknown).
    pub media_type: String,
    pub source: CoverSource,
}

/// The manifest items the OPF declares as its cover, EPUB 3 declarations
/// first. Image items only: a `<meta name="cover">` pointing at the XHTML
/// cover *page* is a common mistake, and that page must never be replaced
/// by image bytes.
fn declared_items(pkg: &Package) -> Vec<(usize, CoverSource)> {
    let mut items: Vec<(usize, CoverSource)> = pkg
        .manifest
        .iter()
        .enumerate()
        .filter(|(_, item)| item.has_property("cover-image") && item.is_image())
        .map(|(i, _)| (i, CoverSource::Epub3))
        .collect();
    for entry in pkg.metadata.iter().filter(|e| is_meta_named(e, "cover")) {
        let Some(id) = entry.meta_content().map(str::trim) else {
            continue;
        };
        if let Some(i) = pkg
            .manifest
            .iter()
            .position(|item| item.id == id && item.is_image())
        {
            if !items.iter().any(|(j, _)| *j == i) {
                items.push((i, CoverSource::Epub2));
            }
        }
    }
    items
}

fn cover_for(pkg: &Package, opf_dir: &str, item: usize, source: CoverSource) -> Option<Cover> {
    let manifest_item = &pkg.manifest[item];
    let path = resolve_href(opf_dir, &manifest_item.href)?;
    Some(Cover {
        path,
        item: Some(item),
        media_type: manifest_item.media_type.clone(),
        source,
    })
}

/// The cover the OPF declares (EPUB 3, else EPUB 2). `opf_dir` is the OPF's
/// directory in the archive, which hrefs are relative to.
pub(crate) fn declared_cover(pkg: &Package, opf_dir: &str) -> Option<Cover> {
    declared_items(pkg)
        .into_iter()
        .find_map(|(i, source)| cover_for(pkg, opf_dir, i, source))
}

/// The cover to show: the declared one, else the display-only guesses
/// described in the module docs.
pub(crate) fn find_cover(pkg: &Package, opf_dir: &str) -> Option<Cover> {
    if let Some(cover) = declared_cover(pkg, opf_dir) {
        return Some(cover);
    }
    let image_item_at = |path: &str| {
        pkg.manifest.iter().position(|item| {
            item.is_image() && resolve_href(opf_dir, &item.href).as_deref() == Some(path)
        })
    };
    for entry in pkg.metadata.iter().filter(|e| is_meta_named(e, "cover")) {
        let path = entry.meta_content().and_then(|c| resolve_href(opf_dir, c));
        if let Some(i) = path.as_deref().and_then(image_item_at) {
            return cover_for(pkg, opf_dir, i, CoverSource::Epub2Href);
        }
    }
    for reference in pkg
        .guide
        .iter()
        .filter(|r| r.kind.eq_ignore_ascii_case("cover"))
    {
        let Some(path) = resolve_href(opf_dir, &reference.href) else {
            continue;
        };
        if let Some(i) = image_item_at(&path) {
            return cover_for(pkg, opf_dir, i, CoverSource::Guide);
        }
        if has_image_extension(&path) {
            return Some(Cover {
                path,
                item: None,
                media_type: String::new(),
                source: CoverSource::Guide,
            });
        }
    }
    let named = pkg.manifest.iter().position(|item| {
        item.is_image()
            && (item.id.eq_ignore_ascii_case("cover")
                || file_stem(&item.href).eq_ignore_ascii_case("cover"))
    })?;
    cover_for(pkg, opf_dir, named, CoverSource::Named)
}

/// The file name of an href without its extension (`cover` for
/// `images/cover.jpg`).
fn file_stem(href: &str) -> &str {
    let name = href.rsplit('/').next().unwrap_or(href);
    name.rsplit_once('.').map_or(name, |(stem, _)| stem)
}

fn has_image_extension(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    [".jpg", ".jpeg", ".png", ".gif", ".webp", ".svg"]
        .iter()
        .any(|ext| lower.ends_with(ext))
}

// ── Setting the cover ─────────────────────────────────────────────────────────

/// The result of [`set_cover`].
#[derive(Debug, Clone)]
pub(crate) struct CoverUpdate {
    /// The new OPF text.
    pub opf: String,
    /// Archive path to write the image bytes to (replacing any entry there).
    pub image_path: String,
}

/// Make an image of type `media_type` (file extension `extension`) the
/// cover of the OPF `text` (parsed as `pkg`, in archive directory `opf_dir`):
/// - with a declared cover, that image is the one replaced: its media type
///   is updated, and both declarations (EPUB 3 property in an EPUB 3 package,
///   EPUB 2 `<meta>`) are made to point at it;
/// - without one, a new manifest item is added under a free name
///   (`cover.jpg`, `cover-1.jpg`… — `entry_exists` tells which archive paths
///   are taken, so an undeclared `cover.jpg` illustration is left alone),
///   declared both ways.
pub(crate) fn set_cover(
    text: &str,
    pkg: &Package,
    opf_dir: &str,
    media_type: &str,
    extension: &str,
    entry_exists: &dyn Fn(&str) -> bool,
) -> Result<CoverUpdate, String> {
    let mut splicer = Splicer::new(text);
    let (image_path, item_id) = match declared_cover(pkg, opf_dir) {
        Some(cover) => {
            let item = &pkg.manifest[cover.item.expect("a declared cover is a manifest item")];
            if !item.media_type.eq_ignore_ascii_case(media_type) {
                let (range, value) =
                    attribute_replacement(text, &item.element, "media-type", media_type);
                splicer.replace(range, value);
            }
            if pkg.is_epub3() && !item.has_property("cover-image") {
                let properties: Vec<&str> = item
                    .properties
                    .iter()
                    .map(String::as_str)
                    .chain(std::iter::once("cover-image"))
                    .collect();
                let (range, value) =
                    attribute_replacement(text, &item.element, "properties", &properties.join(" "));
                splicer.replace(range, value);
            }
            (cover.path, item.id.clone())
        }
        None => {
            let manifest = pkg
                .manifest_section
                .as_ref()
                .ok_or("The OPF has no <manifest>")?;
            if manifest.span.is_self_closing() {
                return Err("The OPF's <manifest> is empty".to_string());
            }
            let (href, path) = free_cover_href(pkg, opf_dir, extension, entry_exists);
            let id = free_id(pkg);
            let qname = match pkg.manifest.first() {
                Some(item) => item.element.qname.clone(),
                None => sibling_name(&manifest.qname, "item"),
            };
            let properties = if pkg.is_epub3() {
                " properties=\"cover-image\""
            } else {
                ""
            };
            let xml = format!(
                "<{qname} id=\"{}\" href=\"{}\" media-type=\"{}\"{properties}/>",
                escape_attr(&id, '"'),
                escape_attr(&href, '"'),
                escape_attr(media_type, '"')
            );
            match pkg.manifest.last() {
                Some(last) => insert_after(&mut splicer, last.element.span, &xml),
                None => insert_into(&mut splicer, manifest.span, &xml),
            }
            (path, id)
        }
    };
    declare_epub2_cover(&mut splicer, pkg, &item_id)?;
    let opf = splicer.finish()?;
    check_cover_edit(text, pkg, &opf, |new| {
        declared_cover(new, opf_dir)
            .is_some_and(|c| c.path == image_path && c.media_type.eq_ignore_ascii_case(media_type))
    })?;
    Ok(CoverUpdate { opf, image_path })
}

/// Make the EPUB 2 `<meta name="cover">` point at item `id`: an existing one
/// is updated (it may point at a page or a missing item), else one is added.
fn declare_epub2_cover(splicer: &mut Splicer, pkg: &Package, id: &str) -> Result<(), String> {
    let text = splicer.text();
    let metas: Vec<usize> = (0..pkg.metadata.len())
        .filter(|&i| is_meta_named(&pkg.metadata[i], "cover"))
        .collect();
    if metas
        .iter()
        .any(|&i| pkg.metadata[i].meta_content().map(str::trim) == Some(id))
    {
        return Ok(());
    }
    if let Some(&first) = metas.first() {
        let (range, value) =
            attribute_replacement(text, &pkg.metadata[first].element, "content", id);
        splicer.replace(range, value);
        return Ok(());
    }
    let section = pkg
        .metadata_section
        .as_ref()
        .ok_or("The OPF has no <metadata>")?;
    if section.span.is_self_closing() {
        return Err("The OPF's <metadata> section is empty".to_string());
    }
    let xml = format!(
        "<{} name=\"cover\" content=\"{}\"/>",
        meta_qname(pkg),
        escape_attr(id, '"')
    );
    let anchor = pkg
        .metadata
        .iter()
        .rev()
        .find(|e| e.kind == EntryKind::Meta)
        .or(pkg.metadata.last());
    match anchor {
        Some(entry) => insert_after(splicer, entry.element.span, &xml),
        None => insert_into(splicer, section.span, &xml),
    }
    Ok(())
}

/// `cover.<ext>`, `cover-1.<ext>`… next to the OPF: the first name that is
/// neither an archive entry nor a manifest item. Returns (href, archive path).
fn free_cover_href(
    pkg: &Package,
    opf_dir: &str,
    extension: &str,
    entry_exists: &dyn Fn(&str) -> bool,
) -> (String, String) {
    (0..)
        .map(|i| {
            if i == 0 {
                format!("cover.{extension}")
            } else {
                format!("cover-{i}.{extension}")
            }
        })
        .find_map(|href| {
            let path = resolve_href(opf_dir, &href)?;
            let in_manifest = pkg.manifest.iter().any(|item| {
                resolve_href(opf_dir, &item.href).is_some_and(|p| p.eq_ignore_ascii_case(&path))
            });
            (!entry_exists(&path) && !in_manifest).then_some((href, path))
        })
        .expect("an unbounded range always finds a free name")
}

/// An id no element of the OPF uses yet (duplicate ids make it invalid).
fn free_id(pkg: &Package) -> String {
    std::iter::once("cover-image".to_string())
        .chain((1..).map(|i| format!("librarium-cover-{i}")))
        .find(|id| !pkg.ids.contains(id))
        .expect("an unbounded range always finds a free id")
}

/// `opf:item` next to `opf:manifest`, `item` next to `manifest`.
fn sibling_name(qname: &str, local: &str) -> String {
    match qname.split_once(':') {
        Some((prefix, _)) => format!("{prefix}:{local}"),
        None => local.to_string(),
    }
}

// ── Removing the cover ────────────────────────────────────────────────────────

/// The result of [`remove_cover`].
#[derive(Debug, Clone)]
pub(crate) struct CoverRemoval {
    /// The new OPF text.
    pub opf: String,
    /// Archive paths of the cover images whose manifest item was removed.
    /// The caller deletes each one only if nothing else in the book still
    /// refers to it (a cover page usually shows the same image).
    pub unlisted_images: Vec<String>,
}

/// Remove the declared cover(s) of the OPF `text`: the manifest items, their
/// EPUB 2 `<meta name="cover">`, and guide references pointing straight at
/// the image. An item the rest of the package still uses (spine, fallbacks)
/// keeps its manifest entry and only loses its `cover-image` property.
/// Returns `None` when nothing is declared as the cover — display-only
/// guesses are never removed.
pub(crate) fn remove_cover(
    text: &str,
    pkg: &Package,
    opf_dir: &str,
) -> Result<Option<CoverRemoval>, String> {
    let declared = declared_items(pkg);
    if declared.is_empty() {
        return Ok(None);
    }
    let mut splicer = Splicer::new(text);
    let mut metadata_removals = BTreeSet::new();
    let mut guide_removals = BTreeSet::new();
    let mut unlisted_images = Vec::new();
    for (i, _) in declared {
        let item = &pkg.manifest[i];
        let path = resolve_href(opf_dir, &item.href);
        if used_elsewhere(pkg, &item.id) {
            if item.has_property("cover-image") {
                let rest: Vec<&str> = item
                    .properties
                    .iter()
                    .map(String::as_str)
                    .filter(|p| *p != "cover-image")
                    .collect();
                if rest.is_empty() {
                    if let Some(range) = attribute_removal(text, &item.element, "properties") {
                        splicer.delete(range);
                    }
                } else {
                    let (range, value) =
                        attribute_replacement(text, &item.element, "properties", &rest.join(" "));
                    splicer.replace(range, value);
                }
            }
        } else {
            splicer.delete(removal_range(text, item.element.span.range()));
            metadata_removals.extend(refinements(pkg, &item.id));
            unlisted_images.extend(path.clone());
        }
        for (k, entry) in pkg.metadata.iter().enumerate() {
            let content = entry.meta_content().map(str::trim);
            if is_meta_named(entry, "cover")
                && (content == Some(item.id.as_str()) || content == Some(item.href.as_str()))
            {
                metadata_removals.insert(k);
            }
        }
        for (k, reference) in pkg.guide.iter().enumerate() {
            if reference.kind.eq_ignore_ascii_case("cover")
                && path.is_some()
                && resolve_href(opf_dir, &reference.href) == path
            {
                guide_removals.insert(k);
            }
        }
    }
    for k in metadata_removals {
        splicer.delete(removal_range(text, pkg.metadata[k].element.span.range()));
    }
    for k in guide_removals {
        splicer.delete(removal_range(text, pkg.guide[k].element.span.range()));
    }
    let opf = splicer.finish()?;
    check_cover_edit(text, pkg, &opf, |new| {
        declared_cover(new, opf_dir).is_none()
    })?;
    Ok(Some(CoverRemoval {
        opf,
        unlisted_images,
    }))
}

/// Whether manifest item `id` is used by the package other than as the
/// cover: in the spine, as the NCX, or as another item's fallback.
fn used_elsewhere(pkg: &Package, id: &str) -> bool {
    pkg.spine.iter().any(|s| s.idref == id)
        || pkg.spine_toc.as_deref() == Some(id)
        || pkg.manifest.iter().any(|item| {
            item.element.attr("fallback").map(str::trim) == Some(id)
                || item.element.attr("media-overlay").map(str::trim) == Some(id)
        })
}

/// Check an OPF after a cover edit: it parses, the unique identifier and
/// the spine are byte-identical, and `ok` holds for the new package.
fn check_cover_edit(
    old_text: &str,
    old: &Package,
    new_text: &str,
    ok: impl Fn(&Package) -> bool,
) -> Result<(), String> {
    let fail = |what: &str| {
        Err(format!(
            "Internal check failed after editing the cover ({what}); the file was not changed"
        ))
    };
    let new = match Package::parse(new_text) {
        Ok(pkg) => pkg,
        Err(e) => return fail(&format!("the result doesn't parse: {e}")),
    };
    if let Some(u) = old.unique_identifier_entry() {
        let same = new.unique_identifier_entry().is_some_and(|v| {
            old_text[old.metadata[u].element.span.range()]
                == new_text[new.metadata[v].element.span.range()]
        });
        if !same {
            return fail("the unique identifier changed");
        }
    }
    let spine = |text: &str, pkg: &Package| {
        pkg.spine_section
            .as_ref()
            .map(|s| text[s.span.range()].to_string())
    };
    if spine(old_text, old) != spine(new_text, &new) {
        return fail("the spine changed");
    }
    if !ok(&new) {
        return fail("the cover isn't declared as expected");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::epub::fixtures::{CALIBRE_OPF, EPUB3_OPF, PREFIXED_ONE_LINE_OPF};

    fn parse(text: &str) -> Package {
        Package::parse(text).unwrap()
    }

    fn opf(metadata: &str, manifest: &str, rest: &str) -> String {
        format!(
            r#"<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:opf="http://www.idpf.org/2007/opf">
    <dc:identifier id="id">urn:uuid:1</dc:identifier>{metadata}
  </metadata>
  <manifest>
    <item id="t" href="Text/t.xhtml" media-type="application/xhtml+xml"/>{manifest}
  </manifest>
  <spine><itemref idref="t"/></spine>{rest}
</package>"#
        )
    }

    fn found(text: &str) -> Option<(String, CoverSource)> {
        find_cover(&parse(text), "OEBPS").map(|c| (c.path, c.source))
    }

    #[test]
    fn declared_covers_are_found_epub3_first() {
        assert_eq!(
            found(EPUB3_OPF),
            Some(("OEBPS/images/cover.png".into(), CoverSource::Epub3))
        );
        assert_eq!(
            find_cover(&parse(CALIBRE_OPF), "").map(|c| (c.path, c.source)),
            Some(("cover.jpeg".into(), CoverSource::Epub2))
        );
        assert_eq!(
            find_cover(&parse(PREFIXED_ONE_LINE_OPF), "").map(|c| c.path),
            Some("cover image.jpg".into()),
            "hrefs are percent-decoded"
        );
    }

    #[test]
    fn display_only_fallbacks() {
        let meta_href = opf(
            r#"<meta name="cover" content="Images/front.jpg"/>"#,
            r#"<item id="f" href="Images/front.jpg" media-type="image/jpeg"/>"#,
            "",
        );
        assert_eq!(
            found(&meta_href),
            Some(("OEBPS/Images/front.jpg".into(), CoverSource::Epub2Href))
        );

        let guide = opf(
            "",
            r#"<item id="f" href="Images/front.jpg" media-type="image/jpeg"/>"#,
            r#"<guide><reference type="cover" href="Images/front.jpg"/></guide>"#,
        );
        assert_eq!(
            found(&guide),
            Some(("OEBPS/Images/front.jpg".into(), CoverSource::Guide))
        );

        let named = opf(
            "",
            r#"<item id="x" href="Images/Cover.JPG" media-type="image/jpeg"/>"#,
            "",
        );
        assert_eq!(
            found(&named),
            Some(("OEBPS/Images/Cover.JPG".into(), CoverSource::Named))
        );

        // A page called "cover", an illustration whose name merely contains
        // "cover", a meta pointing at the cover *page*: none is a cover image.
        let none = opf(
            r#"<meta name="cover" content="cover"/>"#,
            r#"<item id="cover" href="Text/cover.xhtml" media-type="application/xhtml+xml"/><item id="i" href="Images/cover-art-detail.jpg" media-type="image/jpeg"/>"#,
            "",
        );
        assert_eq!(found(&none), None);
        assert_eq!(
            declared_cover(&parse(&meta_href), "OEBPS"),
            None,
            "fallbacks are not declarations"
        );
    }

    #[test]
    fn replacing_a_declared_cover_updates_its_type_and_both_declarations() {
        let pkg = parse(EPUB3_OPF);
        let update = set_cover(EPUB3_OPF, &pkg, "OEBPS", "image/jpeg", "jpg", &|_| true).unwrap();
        assert_eq!(update.image_path, "OEBPS/images/cover.png");
        assert!(update.opf.contains(r#"<item id="cover-img" href="images/cover.png" media-type="image/jpeg" properties="cover-image"/>"#));
        assert!(
            update
                .opf
                .contains(r#"<meta name="cover" content="cover-img"/>"#),
            "EPUB 2 readers get a declaration too"
        );
        assert!(update.opf.contains(
            r#"<item id="ill" href="images/cover-illustration.jpg" media-type="image/jpeg"/>"#
        ));

        // EPUB 2 declaration only, in an EPUB 3 package: the property is added.
        let epub2_only = EPUB3_OPF
            .replace(r#" properties="cover-image""#, "")
            .replace("    <meta property=\"dcterms:modified\">", "    <meta name=\"cover\" content=\"cover-img\"/>\n    <meta property=\"dcterms:modified\">");
        let update = set_cover(
            &epub2_only,
            &parse(&epub2_only),
            "OEBPS",
            "image/png",
            "png",
            &|_| true,
        )
        .unwrap();
        assert!(update.opf.contains(r#"<item id="cover-img" href="images/cover.png" media-type="image/png" properties="cover-image"/>"#), "{}", update.opf);
        assert_eq!(update.opf.matches("name=\"cover\"").count(), 1);
    }

    #[test]
    fn a_book_without_a_declared_cover_gets_a_new_item_never_an_existing_image() {
        let text = opf(
            r#"<meta name="cover" content="cover-page"/>"#,
            r#"<item id="cover-page" href="Text/cover.xhtml" media-type="application/xhtml+xml"/><item id="cover-image" href="cover.jpg" media-type="image/jpeg"/>"#,
            "",
        );
        // An undeclared image "cover.jpg" (with the id we would like) exists.
        let taken = |p: &str| p == "OEBPS/cover.jpg";
        let update = set_cover(&text, &parse(&text), "OEBPS", "image/png", "png", &taken).unwrap();
        assert_eq!(update.image_path, "OEBPS/cover.png");
        assert!(
            update.opf.contains(
                r#"<item id="librarium-cover-1" href="cover.png" media-type="image/png"/>"#
            ),
            "{}",
            update.opf
        );
        assert!(
            update
                .opf
                .contains(r#"<meta name="cover" content="librarium-cover-1"/>"#),
            "the broken meta now points at the image"
        );
        assert!(
            update
                .opf
                .contains(r#"<item id="cover-image" href="cover.jpg" media-type="image/jpeg"/>"#),
            "the other image is untouched"
        );

        let jpg = set_cover(&text, &parse(&text), "OEBPS", "image/jpeg", "jpg", &taken).unwrap();
        assert_eq!(jpg.image_path, "OEBPS/cover-1.jpg", "cover.jpg is taken");
    }

    #[test]
    fn new_covers_follow_the_files_spelling() {
        let pkg = parse(PREFIXED_ONE_LINE_OPF);
        let without =
            PREFIXED_ONE_LINE_OPF.replace(r#"<opf:meta name="cover" content="img"/>"#, "");
        let update = set_cover(&without, &parse(&without), "", "image/jpeg", "jpg", &|_| {
            false
        })
        .unwrap();
        assert!(update.opf.contains(r#"<opf:item id="cover-image" href="cover.jpg" media-type="image/jpeg"/></opf:manifest>"#), "{}", update.opf);
        assert!(
            update.opf.contains(
                r#"<purl:language>en</purl:language><opf:meta name="cover" content="cover-image"/>"#
            ),
            "{}",
            update.opf
        );
        assert!(!update.opf.contains('\n'));
        assert_eq!(pkg.unique_identifier, parse(&update.opf).unique_identifier);
    }

    #[test]
    fn removing_the_cover_keeps_everything_else() {
        let removal = remove_cover(EPUB3_OPF, &parse(EPUB3_OPF), "OEBPS")
            .unwrap()
            .unwrap();
        assert_eq!(
            removal.unlisted_images,
            vec!["OEBPS/images/cover.png".to_string()]
        );
        assert!(!removal.opf.contains("cover-img"));
        assert!(removal.opf.contains(
            r#"<item id="ill" href="images/cover-illustration.jpg" media-type="image/jpeg"/>"#
        ));
        assert_eq!(declared_cover(&parse(&removal.opf), "OEBPS"), None);

        let calibre = remove_cover(CALIBRE_OPF, &parse(CALIBRE_OPF), "")
            .unwrap()
            .unwrap();
        assert!(!calibre.opf.contains(r#"name="cover""#) && !calibre.opf.contains(r#"id="cover""#));
        assert!(
            calibre
                .opf
                .contains(r#"<reference href="titlepage.xhtml" title="Cover" type="cover"/>"#),
            "a guide entry for the cover page stays"
        );
        assert!(calibre
            .opf
            .contains(r#"<item href="images/map-cover.jpg" id="map" media-type="image/jpeg"/>"#));
        assert!(!calibre.opf.contains("\n\n"));
    }

    #[test]
    fn removal_also_drops_a_guide_entry_pointing_at_the_image() {
        let text = opf(
            r#"<meta name="cover" content="c"/>"#,
            r#"<item id="c" href="Images/c.jpg" media-type="image/jpeg"/>"#,
            r#"<guide><reference type="cover" href="Images/c.jpg"/><reference type="toc" href="Text/t.xhtml"/></guide>"#,
        );
        let removal = remove_cover(&text, &parse(&text), "OEBPS")
            .unwrap()
            .unwrap();
        assert!(!removal.opf.contains("Images/c.jpg"), "{}", removal.opf);
        assert!(removal
            .opf
            .contains(r#"<reference type="toc" href="Text/t.xhtml"/>"#));
    }

    #[test]
    fn an_image_used_by_the_spine_keeps_its_item_and_loses_only_the_property() {
        let text = EPUB3_OPF.replace(
            r#"<itemref idref="c1"/>"#,
            r#"<itemref idref="cover-img"/><itemref idref="c1"/>"#,
        );
        let removal = remove_cover(&text, &parse(&text), "OEBPS")
            .unwrap()
            .unwrap();
        assert!(removal.unlisted_images.is_empty());
        assert!(
            removal.opf.contains(
                r#"<item id="cover-img" href="images/cover.png" media-type="image/png"/>"#
            ),
            "{}",
            removal.opf
        );
    }

    #[test]
    fn nothing_declared_nothing_removed() {
        let named = opf(
            "",
            r#"<item id="cover" href="Images/cover.jpg" media-type="image/jpeg"/>"#,
            "",
        );
        assert!(
            remove_cover(&named, &parse(&named), "OEBPS")
                .unwrap()
                .is_none(),
            "a guessed cover is never removed"
        );
    }
}
