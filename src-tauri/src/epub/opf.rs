//! The OPF package document, parsed with quick-xml into a [`Package`]:
//! version, `unique-identifier`, metadata entries, manifest, spine, guide.
//!
//! Namespaces are resolved properly: Dublin Core elements are recognised by
//! their namespace whatever prefix the file binds it to (`dc:`, `purl:`, or
//! the default namespace), and the OPF elements may be prefixed
//! (`<opf:metadata>`, `<opf:meta>`). Files that use a prefix without
//! declaring it are common enough that an unbound `dc:` or `opf:` prefix is
//! taken to mean what it always means.
//!
//! Every element keeps its byte [`Span`] in the original text: the editor
//! ([`super::edit`], [`super::cover`]) uses them to change exactly those
//! bytes and nothing else.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use quick_xml::events::attributes::Attribute as XmlAttribute;
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::{PrefixDeclaration, ResolveResult};
use quick_xml::reader::NsReader;
use quick_xml::XmlVersion;

use super::html::named_entity;
use super::xml::resolve_entity;

/// The OPF namespace (`<package xmlns="…">`, `opf:scheme`…).
pub(crate) const OPF_NS: &str = "http://www.idpf.org/2007/opf";
/// The Dublin Core elements namespace (`dc:title`…).
pub(crate) const DC_NS: &str = "http://purl.org/dc/elements/1.1/";
/// Namespaces of the pre-EPUB "Open eBook" 1.x packages, still found in old
/// files, treated like their modern equivalents.
const OEB_PACKAGE_NS: &str = "http://openebook.org/namespaces/oeb-package/1.0/";
const DC_NS_1_0: &str = "http://purl.org/dc/elements/1.0/";

// ── Model ─────────────────────────────────────────────────────────────────────

/// Byte positions of an element in the OPF text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Span {
    /// The `<` of the start tag.
    pub start: usize,
    /// Just after the `>` of the start tag (for `<x/>`, the end of the element).
    pub tag_end: usize,
    /// The `<` of the end tag (equal to `tag_end` for `<x/>`).
    pub content_end: usize,
    /// Just after the end tag.
    pub end: usize,
}

impl Span {
    /// Whether the element is written `<x …/>`.
    pub(crate) fn is_self_closing(&self) -> bool {
        self.tag_end == self.end
    }

    /// The whole element.
    pub(crate) fn range(&self) -> Range<usize> {
        self.start..self.end
    }

    /// The start tag.
    pub(crate) fn start_tag(&self) -> Range<usize> {
        self.start..self.tag_end
    }

    /// The content between the tags.
    pub(crate) fn content(&self) -> Range<usize> {
        self.tag_end..self.content_end
    }
}

/// The namespace of an element or attribute, as far as an OPF cares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Ns {
    /// The OPF namespace (or the OEB 1.x one, or an undeclared `opf:` prefix).
    Opf,
    /// Dublin Core (1.1 or 1.0, or an undeclared `dc:` prefix).
    Dc,
    /// No namespace.
    None,
    /// Any other namespace (or an unknown prefix).
    Other(String),
}

impl Ns {
    /// Whether an element in this namespace can be an OPF structural element:
    /// in the OPF namespace, or in no namespace at all (OPFs that never
    /// declare one).
    fn is_opf_or_none(&self) -> bool {
        matches!(self, Ns::Opf | Ns::None)
    }
}

/// One attribute, value decoded.
#[derive(Debug, Clone)]
pub(crate) struct Attribute {
    pub local: String,
    pub ns: Ns,
    pub value: String,
}

/// An element: its name, attributes and position.
#[derive(Debug, Clone)]
pub(crate) struct Element {
    /// The name as written (`dc:title`, `opf:meta`).
    pub qname: String,
    pub local: String,
    pub ns: Ns,
    pub attributes: Vec<Attribute>,
    pub span: Span,
}

impl Element {
    /// The value of the unprefixed attribute `local` (`id`, `name`,
    /// `property`, `refines`…).
    pub(crate) fn attr(&self, local: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|a| a.ns == Ns::None && a.local == local)
            .map(|a| a.value.as_str())
    }

    /// The value of the OPF attribute `local` (`opf:scheme`, `opf:role`,
    /// `opf:file-as`): in the OPF namespace under any prefix, or — as many
    /// files write it — unprefixed.
    pub(crate) fn opf_attr(&self, local: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|a| a.ns == Ns::Opf && a.local == local)
            .or_else(|| {
                self.attributes
                    .iter()
                    .find(|a| a.ns == Ns::None && a.local == local)
            })
            .map(|a| a.value.as_str())
    }

    pub(crate) fn id(&self) -> Option<&str> {
        self.attr("id")
    }
}

/// What kind of `<metadata>` child an entry is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EntryKind {
    /// A Dublin Core element (`dc:title`, `dc:creator`…).
    Dc,
    /// An OPF `<meta>`: EPUB 2 `<meta name content/>` or EPUB 3
    /// `<meta property refines>value</meta>`.
    Meta,
    /// Anything else (EPUB 3 `<link>`, vendor elements).
    Other,
}

/// One child of `<metadata>`.
#[derive(Debug, Clone)]
pub(crate) struct MetadataEntry {
    pub kind: EntryKind,
    /// For Dublin Core elements, the lower-cased local name (`title`,
    /// `creator`; OEB 1.x wrote `dc:Title`); otherwise the local name.
    pub name: String,
    pub element: Element,
    /// The text content with entities and CDATA decoded, not trimmed.
    pub text: String,
}

impl MetadataEntry {
    /// Whether this is the Dublin Core element `name` (lower case).
    pub(crate) fn is_dc(&self, name: &str) -> bool {
        self.kind == EntryKind::Dc && self.name == name
    }

    /// The `name` of an EPUB 2 `<meta name="…" content="…"/>`.
    pub(crate) fn meta_name(&self) -> Option<&str> {
        if self.kind == EntryKind::Meta {
            self.element.attr("name")
        } else {
            None
        }
    }

    /// The `content` of an EPUB 2 `<meta>`.
    pub(crate) fn meta_content(&self) -> Option<&str> {
        if self.kind == EntryKind::Meta {
            self.element.attr("content")
        } else {
            None
        }
    }

    /// The `property` of an EPUB 3 `<meta>`.
    pub(crate) fn property(&self) -> Option<&str> {
        if self.kind == EntryKind::Meta {
            self.element.attr("property").map(str::trim)
        } else {
            None
        }
    }

    /// The id an EPUB 3 `<meta refines="#id">` refines.
    pub(crate) fn refines(&self) -> Option<&str> {
        self.element
            .attr("refines")
            .map(|r| r.trim().trim_start_matches('#'))
    }
}

/// One `<item>` of the manifest.
#[derive(Debug, Clone)]
pub(crate) struct ManifestItem {
    pub element: Element,
    pub id: String,
    /// The `href` as written (decoded XML, still a relative URL).
    pub href: String,
    pub media_type: String,
    /// EPUB 3 `properties`, split on whitespace (`cover-image`, `nav`…).
    pub properties: Vec<String>,
}

impl ManifestItem {
    pub(crate) fn has_property(&self, property: &str) -> bool {
        self.properties.iter().any(|p| p == property)
    }

    pub(crate) fn is_image(&self) -> bool {
        self.media_type
            .trim()
            .to_ascii_lowercase()
            .starts_with("image/")
    }
}

/// One `<itemref>` of the spine (the reading order).
#[derive(Debug, Clone)]
pub(crate) struct SpineItem {
    pub idref: String,
}

/// One EPUB 2 `<guide><reference type href/>`.
#[derive(Debug, Clone)]
pub(crate) struct GuideReference {
    pub element: Element,
    /// The `type` (`cover`, `toc`, `text`…).
    pub kind: String,
    pub href: String,
}

/// A top-level section of the package (`<metadata>`, `<manifest>`, `<guide>`).
#[derive(Debug, Clone)]
pub(crate) struct Section {
    /// The element name as written (`metadata`, `opf:metadata`).
    pub qname: String,
    pub span: Span,
}

/// The namespace prefixes in scope inside `<metadata>`, so that elements the
/// editor adds use the same spelling as the rest of the file.
#[derive(Debug, Clone, Default)]
pub(crate) struct MetadataScope {
    /// Prefix bound to Dublin Core: `Some("dc")`, `Some("")` when Dublin Core
    /// is the default namespace, `None` when it isn't declared at all.
    pub dc_prefix: Option<String>,
    /// A named prefix bound to the OPF namespace (`opf`), for `opf:scheme`.
    pub opf_prefix: Option<String>,
    /// Whether unprefixed elements are in the OPF namespace.
    pub default_is_opf: bool,
}

/// A parsed OPF package document.
#[derive(Debug, Clone, Default)]
pub(crate) struct Package {
    /// The `version` attribute (`2.0`, `3.0`), as written.
    pub version: String,
    /// `unique-identifier`: the id of the `dc:identifier` that identifies the
    /// book. Readers key their data on it, and embedded fonts are obfuscated
    /// with it, so it must never change.
    pub unique_identifier: Option<String>,
    pub metadata: Vec<MetadataEntry>,
    pub manifest: Vec<ManifestItem>,
    pub spine: Vec<SpineItem>,
    /// The spine's `toc` attribute (the NCX item id, EPUB 2).
    pub spine_toc: Option<String>,
    pub guide: Vec<GuideReference>,
    pub metadata_section: Option<Section>,
    pub manifest_section: Option<Section>,
    pub guide_section: Option<Section>,
    pub spine_section: Option<Section>,
    pub scope: MetadataScope,
    /// Every `id` attribute in the document (new ids must avoid them all).
    pub ids: HashSet<String>,
}

impl Package {
    /// Parse an OPF. Fails on anything that isn't well-formed XML with a
    /// `<package>` root: a broken file is reported, never half-read or edited.
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        Parser::default().run(text)
    }

    /// Whether this is an EPUB 3 package (`version="3.…"`).
    pub(crate) fn is_epub3(&self) -> bool {
        self.version.trim().starts_with('3')
    }

    /// Index (in [`Package::metadata`]) of the `dc:identifier` named by
    /// `unique-identifier`.
    pub(crate) fn unique_identifier_entry(&self) -> Option<usize> {
        let id = self.unique_identifier.as_deref()?.trim();
        self.metadata
            .iter()
            .position(|e| e.is_dc("identifier") && e.element.id() == Some(id))
    }
}

// ── Parser ────────────────────────────────────────────────────────────────────

/// What an open element is to the parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Package,
    Metadata,
    /// OEB 1.x `<dc-metadata>` / `<x-metadata>` wrappers inside `<metadata>`:
    /// their children are metadata entries.
    MetadataWrapper,
    /// A metadata entry (`index` into `Package::metadata`).
    Entry,
    Manifest,
    Item,
    Spine,
    Itemref,
    Guide,
    Reference,
    Other,
}

#[derive(Debug)]
struct Frame {
    role: Role,
    /// Index into the package list matching the role (entries, items, references).
    index: usize,
    start: usize,
    tag_end: usize,
}

#[derive(Default)]
struct Parser {
    package: Package,
    stack: Vec<Frame>,
    /// The metadata entry whose text is being collected.
    open_entry: Option<usize>,
    root_seen: bool,
}

impl Parser {
    fn run(mut self, text: &str) -> Result<Package, String> {
        let mut reader = NsReader::from_str(text);
        loop {
            let start = reader.buffer_position() as usize;
            let event = reader.read_event().map_err(|e| {
                format!(
                    "The OPF is not well-formed XML: {e} (at byte {})",
                    reader.error_position()
                )
            })?;
            let end = reader.buffer_position() as usize;
            match event {
                Event::Start(e) => self.open(&reader, &e, start, end, false)?,
                Event::Empty(e) => self.open(&reader, &e, start, end, true)?,
                Event::End(_) => {
                    let frame = self
                        .stack
                        .pop()
                        .ok_or("The OPF is not well-formed XML: unexpected end tag")?;
                    let span = Span {
                        start: frame.start,
                        tag_end: frame.tag_end,
                        content_end: start,
                        end,
                    };
                    self.close(frame.role, frame.index, span);
                }
                Event::Text(t) => self.push_text(&t.xml10_content()),
                Event::CData(t) => self.push_text(&t.xml10_content()),
                Event::GeneralRef(r) => match resolve_entity(&r) {
                    Some(text) => self.push_text(&text),
                    None => self.push_text(&format!("&{};", &*r)),
                },
                Event::Eof => break,
                _ => {}
            }
        }
        if let Some(frame) = self.stack.last() {
            let name = &text[frame.start..frame.tag_end];
            return Err(format!("The OPF is truncated: {name} is never closed"));
        }
        if !self.root_seen {
            return Err("The OPF is empty".to_string());
        }
        Ok(self.package)
    }

    fn push_text(&mut self, text: &str) {
        if let Some(i) = self.open_entry {
            self.package.metadata[i].text.push_str(text);
        }
    }

    fn open(
        &mut self,
        reader: &NsReader<&[u8]>,
        e: &BytesStart,
        start: usize,
        tag_end: usize,
        self_closing: bool,
    ) -> Result<(), String> {
        let span = Span {
            start,
            tag_end,
            content_end: tag_end,
            end: tag_end,
        };
        let element = read_element(reader, e, span)?;
        if let Some(id) = element.id() {
            self.package.ids.insert(id.trim().to_string());
        }
        let parent = self.stack.last().map(|f| f.role);
        let pkg = &mut self.package;
        let (role, index) = match parent {
            None => {
                if self.root_seen {
                    return Err(
                        "The OPF is not well-formed XML: more than one root element".to_string()
                    );
                }
                if element.local != "package" || !(element.ns.is_opf_or_none()) {
                    return Err(format!(
                        "This is not an OPF package document (its root is <{}>)",
                        element.qname
                    ));
                }
                self.root_seen = true;
                pkg.version = element
                    .attr("version")
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                pkg.unique_identifier = element
                    .attr("unique-identifier")
                    .map(|s| s.trim().to_string());
                (Role::Package, 0)
            }
            Some(Role::Package) if element.ns.is_opf_or_none() => {
                let section = Some(Section {
                    qname: element.qname.clone(),
                    span,
                });
                match element.local.as_str() {
                    "metadata" if pkg.metadata_section.is_none() => {
                        pkg.scope = metadata_scope(reader);
                        pkg.metadata_section = section;
                        (Role::Metadata, 0)
                    }
                    "manifest" if pkg.manifest_section.is_none() => {
                        pkg.manifest_section = section;
                        (Role::Manifest, 0)
                    }
                    "spine" if pkg.spine_section.is_none() => {
                        pkg.spine_toc = element.attr("toc").map(|s| s.trim().to_string());
                        pkg.spine_section = section;
                        (Role::Spine, 0)
                    }
                    "guide" if pkg.guide_section.is_none() => {
                        pkg.guide_section = section;
                        (Role::Guide, 0)
                    }
                    _ => (Role::Other, 0),
                }
            }
            Some(Role::Metadata | Role::MetadataWrapper) => {
                if matches!(element.local.as_str(), "dc-metadata" | "x-metadata") {
                    (Role::MetadataWrapper, 0)
                } else {
                    let kind = match element.ns {
                        Ns::Dc => EntryKind::Dc,
                        _ if element.local == "meta" && element.ns.is_opf_or_none() => {
                            EntryKind::Meta
                        }
                        _ => EntryKind::Other,
                    };
                    let name = if kind == EntryKind::Dc {
                        element.local.to_ascii_lowercase()
                    } else {
                        element.local.clone()
                    };
                    pkg.metadata.push(MetadataEntry {
                        kind,
                        name,
                        element,
                        text: String::new(),
                    });
                    let index = pkg.metadata.len() - 1;
                    self.open_entry = Some(index);
                    (Role::Entry, index)
                }
            }
            Some(Role::Manifest) if element.local == "item" && element.ns.is_opf_or_none() => {
                let get = |name| element.attr(name).unwrap_or_default().trim().to_string();
                let (id, href, media_type) = (get("id"), get("href"), get("media-type"));
                let properties = element
                    .attr("properties")
                    .unwrap_or_default()
                    .split_whitespace()
                    .map(str::to_string)
                    .collect();
                pkg.manifest.push(ManifestItem {
                    element,
                    id,
                    href,
                    media_type,
                    properties,
                });
                (Role::Item, pkg.manifest.len() - 1)
            }
            Some(Role::Spine) if element.local == "itemref" && element.ns.is_opf_or_none() => {
                let idref = element.attr("idref").unwrap_or_default().trim().to_string();
                pkg.spine.push(SpineItem { idref });
                (Role::Itemref, 0)
            }
            Some(Role::Guide) if element.local == "reference" && element.ns.is_opf_or_none() => {
                let kind = element.attr("type").unwrap_or_default().trim().to_string();
                let href = element.attr("href").unwrap_or_default().trim().to_string();
                pkg.guide.push(GuideReference {
                    element,
                    kind,
                    href,
                });
                (Role::Reference, pkg.guide.len() - 1)
            }
            _ => (Role::Other, 0),
        };
        if self_closing {
            self.close(role, index, span);
        } else {
            self.stack.push(Frame {
                role,
                index,
                start,
                tag_end,
            });
        }
        Ok(())
    }

    fn close(&mut self, role: Role, index: usize, span: Span) {
        let pkg = &mut self.package;
        match role {
            Role::Entry => {
                pkg.metadata[index].element.span = span;
                self.open_entry = None;
            }
            Role::Item => pkg.manifest[index].element.span = span,
            Role::Reference => pkg.guide[index].element.span = span,
            Role::Metadata => set_span(&mut pkg.metadata_section, span),
            Role::Manifest => set_span(&mut pkg.manifest_section, span),
            Role::Guide => set_span(&mut pkg.guide_section, span),
            Role::Spine => set_span(&mut pkg.spine_section, span),
            Role::Package | Role::MetadataWrapper | Role::Itemref | Role::Other => {}
        }
    }
}

fn set_span(section: &mut Option<Section>, span: Span) {
    if let Some(section) = section {
        section.span = span;
    }
}

/// Name, namespace and decoded attributes of a start tag.
fn read_element(reader: &NsReader<&[u8]>, e: &BytesStart, span: Span) -> Result<Element, String> {
    let qname = e.name().as_ref().to_string();
    let (resolved, local) = reader.resolver().resolve_element(e.name());
    let ns = classify(&resolved);
    let local = local.as_ref().to_string();
    let mut attributes = Vec::new();
    for attr in e.attributes() {
        let attr = attr.map_err(|err| {
            format!("The OPF is not well-formed XML: bad attribute in <{qname}>: {err}")
        })?;
        if attr.key.as_namespace_binding().is_some() {
            continue;
        }
        let (resolved, attr_local) = reader.resolver().resolve_attribute(attr.key);
        attributes.push(Attribute {
            local: attr_local.as_ref().to_string(),
            ns: classify(&resolved),
            value: attribute_value(&attr),
        });
    }
    Ok(Element {
        qname,
        local,
        ns,
        attributes,
        span,
    })
}

/// The decoded value of an attribute: entities and character references
/// resolved (HTML names included), whitespace normalised as XML requires. A
/// value quick-xml can't decode (an unknown entity) is returned as written.
pub(crate) fn attribute_value(attr: &XmlAttribute) -> String {
    attr.normalized_value_with(XmlVersion::Implicit1_0, 8, named_entity)
        .map(|v| v.into_owned())
        .unwrap_or_else(|_| attr.value.to_string())
}

fn classify(resolved: &ResolveResult) -> Ns {
    match resolved {
        ResolveResult::Bound(ns) => match ns.0 {
            OPF_NS | OEB_PACKAGE_NS => Ns::Opf,
            DC_NS | DC_NS_1_0 => Ns::Dc,
            other => Ns::Other(other.to_string()),
        },
        ResolveResult::Unbound => Ns::None,
        ResolveResult::Unknown(prefix) => match prefix.as_str() {
            "opf" => Ns::Opf,
            "dc" => Ns::Dc,
            other => Ns::Other(format!("{other}:")),
        },
    }
}

/// The prefixes in scope at `<metadata>` (its own declarations included).
fn metadata_scope(reader: &NsReader<&[u8]>) -> MetadataScope {
    // Later bindings shadow earlier ones with the same prefix.
    let mut bindings: HashMap<Option<String>, String> = HashMap::new();
    for (prefix, ns) in reader.resolver().bindings() {
        let prefix = match prefix {
            PrefixDeclaration::Default => None,
            PrefixDeclaration::Named(p) => Some(p.to_string()),
        };
        bindings.insert(prefix, ns.0.to_string());
    }
    let named_prefix_for = |uris: &[&str], preferred: &str| {
        let mut prefixes: Vec<&String> = bindings
            .iter()
            .filter_map(|(p, ns)| p.as_ref().filter(|_| uris.contains(&ns.as_str())))
            .collect();
        prefixes.sort_by_key(|p| (p.as_str() != preferred, p.len(), p.to_string()));
        prefixes.first().map(|p| p.to_string())
    };
    let default = bindings.get(&None).map(String::as_str);
    let dc_prefix = named_prefix_for(&[DC_NS, DC_NS_1_0], "dc").or_else(|| {
        default
            .filter(|d| [DC_NS, DC_NS_1_0].contains(d))
            .map(|_| String::new())
    });
    MetadataScope {
        dc_prefix,
        opf_prefix: named_prefix_for(&[OPF_NS, OEB_PACKAGE_NS], "opf"),
        default_is_opf: default.is_some_and(|d| d == OPF_NS || d == OEB_PACKAGE_NS),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPUB2: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="BookId">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:opf="http://www.idpf.org/2007/opf">
    <dc:title>Tom &amp; Jerry &#8211; caf&eacute; <![CDATA[<raw>]]></dc:title>
    <dc:creator opf:role="aut" opf:file-as="Herbert, Frank">Frank Herbert</dc:creator>
    <dc:identifier id="BookId" opf:scheme="UUID">urn:uuid:1234</dc:identifier>
    <meta name="cover" content="cover-img"/>
    <!-- a comment -->
  </metadata>
  <manifest>
    <item id="cover-img" href="images/cover%20art.jpg" media-type="image/jpeg"/>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav scripted"/>
  </manifest>
  <spine toc="ncx"><itemref idref="nav"/><itemref idref="notes" linear="no"/></spine>
  <guide><reference type="cover" title="Cover" href="cover.xhtml"/></guide>
</package>"#;

    #[test]
    fn parses_an_epub2_package() {
        let pkg = Package::parse(EPUB2).unwrap();
        assert_eq!(pkg.version, "2.0");
        assert!(!pkg.is_epub3());
        assert_eq!(pkg.unique_identifier.as_deref(), Some("BookId"));
        assert_eq!(pkg.metadata.len(), 4);

        let title = &pkg.metadata[0];
        assert!(title.is_dc("title"));
        assert_eq!(title.text, "Tom & Jerry – café <raw>");
        assert_eq!(&EPUB2[title.element.span.range()][..10], "<dc:title>");

        let creator = &pkg.metadata[1];
        assert_eq!(creator.element.opf_attr("role"), Some("aut"));
        assert_eq!(creator.element.opf_attr("file-as"), Some("Herbert, Frank"));
        assert_eq!(
            creator.element.attr("role"),
            None,
            "opf:role is not the unprefixed role"
        );

        assert_eq!(pkg.unique_identifier_entry(), Some(2));
        assert_eq!(pkg.metadata[3].meta_name(), Some("cover"));
        assert_eq!(pkg.metadata[3].meta_content(), Some("cover-img"));

        assert_eq!(pkg.manifest.len(), 2);
        assert_eq!(pkg.manifest[0].href, "images/cover%20art.jpg");
        assert!(pkg.manifest[0].is_image());
        assert!(pkg.manifest[1].has_property("nav") && pkg.manifest[1].has_property("scripted"));
        assert_eq!(
            pkg.spine
                .iter()
                .map(|s| s.idref.as_str())
                .collect::<Vec<_>>(),
            vec!["nav", "notes"]
        );
        assert_eq!(pkg.spine_toc.as_deref(), Some("ncx"));
        assert_eq!(pkg.guide[0].kind, "cover");
        assert_eq!(pkg.guide[0].href, "cover.xhtml");
        assert!(pkg.ids.contains("BookId") && pkg.ids.contains("nav"));
        assert_eq!(pkg.scope.dc_prefix.as_deref(), Some("dc"));
        assert_eq!(pkg.scope.opf_prefix.as_deref(), Some("opf"));
        assert!(pkg.scope.default_is_opf);

        let section = pkg.metadata_section.as_ref().unwrap();
        assert!(EPUB2[section.span.range()].starts_with("<metadata"));
        assert!(EPUB2[section.span.range()].ends_with("</metadata>"));
    }

    #[test]
    fn opf_prefixed_elements_and_an_unusual_dc_prefix() {
        let opf = r#"<opf:package xmlns:opf="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id">
<opf:metadata xmlns:purl="http://purl.org/dc/elements/1.1/">
<purl:title>Dune</purl:title><purl:identifier id="id">x</purl:identifier>
<opf:meta property="dcterms:modified">2020-01-01T00:00:00Z</opf:meta>
<dc:title>not Dublin Core: dc is bound to something else here</dc:title>
</opf:metadata>
<opf:manifest><opf:item id="a" href="a.xhtml" media-type="application/xhtml+xml"/></opf:manifest>
<opf:spine><opf:itemref idref="a"/></opf:spine>
</opf:package>"#
            .replace("<dc:title>", "<dc:title xmlns:dc=\"urn:other\">");
        let pkg = Package::parse(&opf).unwrap();
        assert!(pkg.is_epub3());
        assert!(pkg.metadata[0].is_dc("title"));
        assert_eq!(pkg.metadata[0].text, "Dune");
        assert_eq!(pkg.unique_identifier_entry(), Some(1));
        assert_eq!(pkg.metadata[2].kind, EntryKind::Meta);
        assert_eq!(pkg.metadata[2].property(), Some("dcterms:modified"));
        assert_eq!(pkg.metadata[3].kind, EntryKind::Other);
        assert_eq!(pkg.manifest.len(), 1);
        assert_eq!(pkg.spine.len(), 1);
        assert_eq!(pkg.scope.dc_prefix.as_deref(), Some("purl"));
        assert_eq!(pkg.scope.opf_prefix.as_deref(), Some("opf"));
        assert!(!pkg.scope.default_is_opf);
    }

    #[test]
    fn undeclared_prefixes_and_oeb_wrappers_are_tolerated() {
        let opf = r#"<package version="2.0"><metadata><dc-metadata>
<dc:Title>Old book</dc:Title></dc-metadata><x-metadata><meta name="cover" content="c"/></x-metadata>
</metadata><manifest/></package>"#;
        let pkg = Package::parse(opf).unwrap();
        assert!(pkg.metadata[0].is_dc("title"));
        assert_eq!(pkg.metadata[0].text, "Old book");
        assert_eq!(pkg.metadata[1].meta_name(), Some("cover"));
        assert_eq!(pkg.scope.dc_prefix, None);
    }

    #[test]
    fn malformed_documents_are_errors() {
        assert!(
            Package::parse("<package><metadata></package>").is_err(),
            "mismatched end tag"
        );
        assert!(Package::parse("<package><metadata>").is_err(), "truncated");
        assert!(
            Package::parse("<html><body/></html>").is_err(),
            "not a package"
        );
        assert!(Package::parse("").is_err());
        assert!(
            Package::parse("<package a=\"1\" a=\"2\"/>").is_err(),
            "duplicate attribute"
        );
        assert!(Package::parse("<package/><package/>").is_err(), "two roots");
    }

    #[test]
    fn attribute_values_are_decoded() {
        let pkg = Package::parse(
            r#"<package><metadata><meta name="calibre:series" content="Tom &amp; Jerry &#233;t&eacute;"/></metadata></package>"#,
        )
        .unwrap();
        assert_eq!(pkg.metadata[0].meta_content(), Some("Tom & Jerry été"));
    }
}
