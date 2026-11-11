//! KF8 Skeleton Chunking
//!
//! Implements the algorithm for breaking HTML content into skeleton + chunks
//! as used by Amazon's KF8/AZW3 format.
//!
//! The skeleton is the HTML structure with large content removed.
//! Chunks are the removed content pieces, each with an insert position.
//!
//! Modified for Librarium on 2026-11-05 (from boko 0.6.0): chunks are now
//! whole sibling nodes inserted under their real parent element (calibre's
//! algorithm) instead of byte ranges cut at any end tag, and `rebuild()`
//! handles chunks at arbitrary skeleton positions.

use std::collections::HashMap;

/// Tags that can receive aid attributes (based on Calibre's list)
pub const AID_ABLE_TAGS: &[&str] = &[
    "a",
    "abbr",
    "address",
    "article",
    "aside",
    "audio",
    "b",
    "bdo",
    "blockquote",
    "body",
    "button",
    "cite",
    "code",
    "dd",
    "del",
    "details",
    "dfn",
    "div",
    "dl",
    "dt",
    "em",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hgroup",
    "i",
    "ins",
    "kbd",
    "label",
    "legend",
    "li",
    "map",
    "mark",
    "meter",
    "nav",
    "ol",
    "output",
    "p",
    "pre",
    "progress",
    "q",
    "rp",
    "rt",
    "samp",
    "section",
    "select",
    "small",
    "span",
    "strong",
    "sub",
    "summary",
    "sup",
    "textarea",
    "time",
    "ul",
    "var",
    "video",
];

/// A chunk of content extracted from the skeleton
#[derive(Debug, Clone)]
pub struct Chunk {
    /// The raw bytes of this chunk
    pub raw: Vec<u8>,
    /// Insert position in the skeleton (absolute byte offset)
    pub insert_pos: usize,
    /// Selector for this chunk (e.g., "P-//*[@aid='xxx']")
    pub selector: String,
    /// File number this chunk belongs to
    pub file_number: usize,
    /// Sequence number within all chunks
    pub sequence_number: usize,
    /// Start position within the file's chunks
    pub start_pos: usize,
}

/// A skeleton file with its associated chunks
#[derive(Debug)]
pub struct Skeleton {
    /// File number (index in spine)
    pub file_number: usize,
    /// The skeleton HTML (structure with content removed)
    pub skeleton: Vec<u8>,
    /// Chunks extracted from this skeleton
    pub chunks: Vec<Chunk>,
    /// Start position of this skeleton in the combined text
    pub start_pos: usize,
}

impl Skeleton {
    /// Total length of skeleton + all chunks
    pub fn len(&self) -> usize {
        self.skeleton.len() + self.chunks.iter().map(|c| c.raw.len()).sum::<usize>()
    }

    /// Get the raw text (skeleton + chunks concatenated). This is the
    /// on-disk layout that ends up in rawML records.
    pub fn raw_text(&self) -> Vec<u8> {
        let mut result = self.skeleton.clone();
        for chunk in &self.chunks {
            result.extend_from_slice(&chunk.raw);
        }
        result
    }

    /// Get the reassembled file content (chunks inserted into the skeleton
    /// at their `insert_pos`). This is what Kindle materialises when
    /// rendering a part; `chunk_table.insert_pos` and `pos_fid` offsets are
    /// in *this* coordinate space, not in the rawML layout.
    pub fn rebuild(&self) -> Vec<u8> {
        // Chunks are inserted in order, and `insert_pos - start_pos` is each
        // chunk's offset in the *reassembled* file (earlier chunks already
        // inserted). So copy skeleton bytes until the output reaches that
        // offset, then append the chunk. Chunks may sit at different points
        // of the skeleton (inside elements whose tags stay in the skeleton),
        // or back to back.
        let mut chunks: Vec<&Chunk> = self.chunks.iter().collect();
        chunks.sort_by_key(|c| c.insert_pos);

        let mut result = Vec::with_capacity(self.len());
        let mut skel_cursor = 0;
        for chunk in chunks {
            let target = chunk.insert_pos.saturating_sub(self.start_pos);
            let take = target
                .saturating_sub(result.len())
                .min(self.skeleton.len() - skel_cursor);
            result.extend_from_slice(&self.skeleton[skel_cursor..skel_cursor + take]);
            skel_cursor += take;
            result.extend_from_slice(&chunk.raw);
        }
        result.extend_from_slice(&self.skeleton[skel_cursor..]);
        result
    }
}

/// SKEL table entry (for INDX record)
#[derive(Debug, Clone)]
pub struct SkelEntry {
    pub name: String,
    pub chunk_count: usize,
    pub start_pos: usize,
    pub length: usize,
}

/// Chunk table entry (for INDX record)
#[derive(Debug, Clone)]
pub struct ChunkEntry {
    pub insert_pos: usize,
    pub selector: String,
    pub file_number: usize,
    pub sequence_number: usize,
    pub start_pos: usize,
    pub length: usize,
}

/// Result of chunking operation
pub struct ChunkerResult {
    pub skel_table: Vec<SkelEntry>,
    pub chunk_table: Vec<ChunkEntry>,
    pub text: Vec<u8>,
    /// Maps (file_href, anchor_id) -> aid for link resolution
    pub id_map: HashMap<(String, String), String>,
    /// Maps aid -> (chunk_sequence_number, offset_in_chunk, offset_in_text)
    pub aid_offset_map: HashMap<String, (usize, usize, usize)>,
    /// Maps file_href -> [(original_position, aid)] for filepos resolution
    pub filepos_map: HashMap<String, Vec<(usize, String)>>,
}

/// Chunker - breaks HTML files into skeletons and chunks
pub struct Chunker {
    aid_counter: u32,
    /// Mapping of (file, id) -> aid built during processing
    id_map: HashMap<(String, String), String>,
    /// Mapping of file_href -> [(original_position, aid)] for filepos resolution
    filepos_map: HashMap<String, Vec<(usize, String)>>,
}

impl Chunker {
    pub fn new() -> Self {
        Self {
            aid_counter: 0,
            id_map: HashMap::new(),
            filepos_map: HashMap::new(),
        }
    }

    /// Process multiple HTML files into skeletons and chunks
    pub fn process(&mut self, html_files: &[(String, Vec<u8>)]) -> ChunkerResult {
        let mut skeletons = Vec::new();
        let mut start_pos = 0;

        for (i, (file_href, html)) in html_files.iter().enumerate() {
            let skeleton = self.process_file(i, file_href, html, start_pos);
            start_pos += skeleton.len();
            skeletons.push(skeleton);
        }

        // Build SKEL and Chunk tables directly from the per-file chunks
        // produced by process_file. SKEL.length is the skeleton's bytes (0
        // in this writer); chunks' insert_pos and length describe the
        // content slice that lives in the rawML right after the skeleton.
        let mut chunk_table: Vec<ChunkEntry> = Vec::new();
        let mut seq_num = 0usize;
        for skel in &mut skeletons {
            for chunk in &mut skel.chunks {
                chunk.sequence_number = seq_num;
                chunk_table.push(ChunkEntry {
                    insert_pos: chunk.insert_pos,
                    selector: chunk.selector.clone(),
                    file_number: chunk.file_number,
                    sequence_number: seq_num,
                    start_pos: chunk.start_pos,
                    length: chunk.raw.len(),
                });
                seq_num += 1;
            }
        }

        let skel_table: Vec<SkelEntry> = skeletons
            .iter()
            .map(|s| SkelEntry {
                name: format!("SKEL{:010}", s.file_number),
                chunk_count: s.chunks.len(),
                start_pos: s.start_pos,
                length: s.skeleton.len(),
            })
            .collect();

        // `text` is the on-disk rawML layout: per skel, the skeleton bytes
        // followed by its chunk bytes.
        let text: Vec<u8> = skeletons.iter().flat_map(|s| s.raw_text()).collect();

        // Build aid_offset_map by scanning the *reassembled* book — chunks
        // inserted into their skeletons — because `chunk_table.insert_pos`
        // is in reassembled coordinates. Calibre does the same in
        // `Skeleton.set_internal_links` (it iterates `rebuilt_text`, not
        // the rawML).
        let rebuilt: Vec<u8> = skeletons.iter().flat_map(|s| s.rebuild()).collect();
        let aid_offset_map = self.build_aid_offset_map(&rebuilt, &chunk_table);

        ChunkerResult {
            skel_table,
            chunk_table,
            text,
            id_map: std::mem::take(&mut self.id_map),
            aid_offset_map,
            filepos_map: std::mem::take(&mut self.filepos_map),
        }
    }

    /// Build map of aid -> (chunk_sequence_number, offset_in_chunk, offset_in_text)
    fn build_aid_offset_map(
        &self,
        text: &[u8],
        chunk_table: &[ChunkEntry],
    ) -> HashMap<String, (usize, usize, usize)> {
        use memchr::memmem;

        let mut aid_offset_map = HashMap::new();
        let finder = memmem::Finder::new(b" aid=\"");
        let mut search_pos = 0;
        // Aids are discovered in ascending offset order and chunks are sorted
        // by insert_pos, so a persistent cursor resolves each aid without
        // rescanning the chunk table (previously O(aids × chunks)).
        let mut chunk_cursor = 0usize;

        while let Some(rel_pos) = finder.find(&text[search_pos..]) {
            let offset = search_pos + rel_pos;
            let val_start = offset + 6; // len(" aid=\"") is 6

            // Validate we have enough bytes for 4-char ID + quote
            if val_start + 5 <= text.len() {
                // Extract 4-byte aid
                let aid_bytes = &text[val_start..val_start + 4];
                let quote = text[val_start + 4];

                if quote == b'"' {
                    let aid = String::from_utf8_lossy(aid_bytes).to_string();
                    // Link targets must point at the element's `<`, not at
                    // the ` aid="` attribute inside the tag (calibre uses
                    // `match.start()` of `<[^>]+? aid=`). Kindle cannot
                    // resolve a position in the middle of a start tag.
                    let offset = memchr::memrchr(b'<', &text[..offset]).unwrap_or(offset);

                    // `offset` is in reassembled coordinates. Find the chunk
                    // whose [insert_pos, insert_pos+length) range contains
                    // it. Calibre falls back to "the chunk immediately
                    // after" when the aid is in the skeleton (e.g. on
                    // `<body aid="0000">`), with an in-chunk offset of 0.
                    let (seq_num, offset_in_chunk) = if chunk_table.is_empty() {
                        (0usize, offset)
                    } else {
                        // Advance to the first chunk that ends after `offset`.
                        while chunk_cursor < chunk_table.len() {
                            let c = &chunk_table[chunk_cursor];
                            if c.insert_pos + c.length > offset {
                                break;
                            }
                            chunk_cursor += 1;
                        }
                        match chunk_table.get(chunk_cursor) {
                            // Chunk contains the aid.
                            Some(chunk) if offset >= chunk.insert_pos => {
                                (chunk.sequence_number, offset - chunk.insert_pos)
                            }
                            // Aid is in a skeleton before this chunk — use
                            // this chunk with in-chunk offset 0.
                            Some(chunk) => (chunk.sequence_number, 0),
                            // Past every chunk: point to the last chunk's end.
                            None => {
                                let last = chunk_table.last().expect("chunk_table non-empty here");
                                (last.sequence_number, last.length.saturating_sub(1))
                            }
                        }
                    };

                    aid_offset_map.insert(aid, (seq_num, offset_in_chunk, offset));
                }
            }
            search_pos = val_start;
        }

        aid_offset_map
    }

    /// Process a single HTML file into a skeleton + chunks.
    ///
    /// Layout mirrors calibre's `writer8/skeleton.py`: the skeleton is the
    /// file with the chunked content cut out, and every chunk is a run of
    /// *complete* sibling nodes (elements and text) of one parent element,
    /// whose aid is the chunk's selector. A child too large for one chunk is
    /// stepped into: its start and end tags stay in the skeleton and its own
    /// children are chunked. So no chunk starts or ends inside an element.
    ///
    /// Kindle relies on that: it places each fragment under its selector
    /// element, and a fragment that opens a `<p>` it doesn't close, or that
    /// starts with a stray `</p>` or mid-sentence text, is dropped on the
    /// device — losing a whole fragment (~3 pages) of text.
    ///
    /// Earlier versions packed the whole file into a single chunk and used
    /// an empty skeleton, which freezes the device's renderer: Kindle's
    /// layout engine seems to require real HTML scaffolding bytes per skel.
    fn process_file(
        &mut self,
        file_number: usize,
        file_href: &str,
        html: &[u8],
        start_pos: usize,
    ) -> Skeleton {
        // Kindle's HTML5 parser chokes on EPUB3 namespace decorations
        // (`xmlns:epub`, `epub:type`, `epub:prefix`, `xml:lang`, etc.) so
        // strip them before aid annotation.
        let cleaned = super::writer_transform::strip_xml_namespaces(html);
        let result = super::writer_transform::add_aid_attributes_fast(
            &cleaned,
            file_href,
            &mut self.aid_counter,
            &mut self.id_map,
        );

        if !result.position_map.is_empty() {
            self.filepos_map
                .insert(file_href.to_string(), result.position_map);
        }

        let doc = &result.html;
        let (skel_prefix, body_content, _) = split_body(doc);
        let body_start = skel_prefix.len();
        let body_end = body_start + body_content.len();

        // Chunk selectors name the element a chunk is inserted into. Kindle's
        // renderer uses them to map a chunk's content back to its enclosing
        // DOM element; aids are unique across the book, which also keeps
        // selectors unique per file (without that Kindle conflates positions
        // across files when laying out and locks up).
        let body_aid = extract_body_aid(skel_prefix).unwrap_or_else(|| "0000".to_string());

        let mut pieces = match parse_nodes(doc, body_start, body_end) {
            Some(nodes) => {
                let mut pieces = Vec::new();
                chunk_children(doc, &nodes, &body_aid, &mut pieces);
                pieces
            }
            // Unbalanced markup: element boundaries are unknown, so keep the
            // whole body as one (possibly oversized) chunk rather than risk
            // cutting through an element.
            None => vec![Piece {
                start: body_start,
                end: body_end,
                parent_aid: body_aid.clone(),
            }],
        };
        if pieces.is_empty() {
            // Every skeleton gets at least one chunk.
            pieces.push(Piece {
                start: body_start,
                end: body_start,
                parent_aid: body_aid,
            });
        }

        // The skeleton is the file minus the chunk bytes. Chunks are inserted
        // in order, so each one's `insert_pos` (reassembled coordinates) is
        // simply its offset in the original file.
        let mut skeleton_bytes = Vec::with_capacity(doc.len());
        let mut chunks = Vec::with_capacity(pieces.len());
        let mut skel_cursor = 0;
        let mut cumulative = 0usize;
        for piece in pieces {
            skeleton_bytes.extend_from_slice(&doc[skel_cursor..piece.start]);
            skel_cursor = piece.end;
            let raw = doc[piece.start..piece.end].to_vec();
            let len = raw.len();
            chunks.push(Chunk {
                raw,
                insert_pos: start_pos + piece.start,
                selector: format!("P-//*[@aid='{}']", piece.parent_aid),
                file_number,
                sequence_number: 0, // assigned by Chunker::process
                start_pos: cumulative,
            });
            cumulative += len;
        }
        skeleton_bytes.extend_from_slice(&doc[skel_cursor..]);

        Skeleton {
            file_number,
            skeleton: skeleton_bytes,
            chunks,
            start_pos,
        }
    }
}

const CHUNK_SIZE: usize = 8192;

/// Extract the `aid="…"` value from the `<body…>` tag inside the scaffold
/// prefix. Calibre's chunker tracks this naturally via DOM walk; in our
/// byte-level chunker we recover it by scanning the prefix bytes for the
/// already-inserted aid attribute on the body opening tag.
fn extract_body_aid(skel_prefix: &[u8]) -> Option<String> {
    let body_pos = memchr::memmem::find(skel_prefix, b"<body")?;
    let tag_end_rel = memchr::memchr(b'>', &skel_prefix[body_pos..])?;
    tag_aid(&skel_prefix[body_pos..body_pos + tag_end_rel])
}

/// The `aid="…"` value of a start tag, if it has one.
fn tag_aid(tag: &[u8]) -> Option<String> {
    use memchr::memmem;
    let aid_pos = memmem::find(tag, b" aid=\"")?;
    let val_start = aid_pos + 6;
    let val_end = memchr::memchr(b'"', &tag[val_start..])?;
    Some(
        std::str::from_utf8(&tag[val_start..val_start + val_end])
            .ok()?
            .to_string(),
    )
}

/// HTML elements that never have an end tag.
const VOID_TAGS: &[&[u8]] = &[
    b"area", b"base", b"br", b"col", b"embed", b"hr", b"img", b"input", b"link", b"meta",
    b"param", b"source", b"track", b"wbr",
];

/// A node of a file's body, as a byte range of the file.
struct Node {
    start: usize,
    end: usize,
    kind: NodeKind,
}

enum NodeKind {
    /// Character data between tags.
    Text,
    /// Something that can't be split: a comment, CDATA section, processing
    /// instruction, void or self-closing element, or `<script>`/`<style>`.
    Leaf,
    /// An element with an end tag.
    Element {
        aid: Option<String>,
        children: Vec<Node>,
    },
}

/// A run of the file to emit as one chunk, and the aid of the element it is
/// inserted into.
struct Piece {
    start: usize,
    end: usize,
    parent_aid: String,
}

/// Index just past the `>` closing the tag that starts at `lt`, ignoring
/// `>` inside quoted attribute values.
fn find_tag_end(html: &[u8], lt: usize, end: usize) -> Option<usize> {
    let mut in_quote: Option<u8> = None;
    for (j, &c) in html[lt + 1..end].iter().enumerate() {
        match in_quote {
            Some(q) if c == q => in_quote = None,
            Some(_) => {}
            None => match c {
                b'"' | b'\'' => in_quote = Some(c),
                b'>' => return Some(lt + 1 + j + 1),
                _ => {}
            },
        }
    }
    None
}

/// Parse `html[start..end]` into a tree of nodes covering every byte.
///
/// Returns `None` when the markup isn't balanced (an end tag that doesn't
/// match the open element, or elements left open): chunk boundaries can't be
/// placed safely then.
fn parse_nodes(html: &[u8], start: usize, end: usize) -> Option<Vec<Node>> {
    use memchr::memmem;

    struct Open {
        name: Vec<u8>,
        start: usize,
        aid: Option<String>,
        children: Vec<Node>,
    }

    fn push(root: &mut Vec<Node>, stack: &mut [Open], node: Node) {
        match stack.last_mut() {
            Some(open) => open.children.push(node),
            None => root.push(node),
        }
    }

    let mut root = Vec::new();
    let mut stack: Vec<Open> = Vec::new();
    let mut i = start;

    while i < end {
        if html[i] != b'<' {
            let next = memchr::memchr(b'<', &html[i..end]).map_or(end, |p| i + p);
            push(&mut root, &mut stack, Node { start: i, end: next, kind: NodeKind::Text });
            i = next;
            continue;
        }

        let rest = &html[i..end];
        let special_end = if rest.starts_with(b"<!--") {
            Some(memmem::find(&rest[4..], b"-->").map(|p| i + 4 + p + 3)?)
        } else if rest.starts_with(b"<![CDATA[") {
            Some(memmem::find(&rest[9..], b"]]>").map(|p| i + 9 + p + 3)?)
        } else if rest.starts_with(b"<!") || rest.starts_with(b"<?") {
            Some(memchr::memchr(b'>', rest).map(|p| i + p + 1)?)
        } else {
            None
        };
        if let Some(leaf_end) = special_end {
            push(&mut root, &mut stack, Node { start: i, end: leaf_end, kind: NodeKind::Leaf });
            i = leaf_end;
            continue;
        }

        let is_close = rest.get(1) == Some(&b'/');
        let name_start = i + if is_close { 2 } else { 1 };
        let name_len = html[name_start..end]
            .iter()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, b':' | b'-' | b'_' | b'.'))
            .count();
        if name_len == 0 {
            // A stray `<` that doesn't start a tag (`a < b`) is text.
            let next = memchr::memchr(b'<', &html[i + 1..end]).map_or(end, |p| i + 1 + p);
            push(&mut root, &mut stack, Node { start: i, end: next, kind: NodeKind::Text });
            i = next;
            continue;
        }
        let name = html[name_start..name_start + name_len].to_ascii_lowercase();
        let tag_end = find_tag_end(html, i, end)?;

        if is_close {
            let open = stack.pop()?;
            if open.name != name {
                return None;
            }
            let node = Node {
                start: open.start,
                end: tag_end,
                kind: NodeKind::Element { aid: open.aid, children: open.children },
            };
            push(&mut root, &mut stack, node);
        } else if html[tag_end - 2] == b'/' || VOID_TAGS.contains(&name.as_slice()) {
            push(&mut root, &mut stack, Node { start: i, end: tag_end, kind: NodeKind::Leaf });
        } else if name == b"script" || name == b"style" {
            // Raw text: its content may contain `<`.
            let close_tag = [b"</", name.as_slice()].concat();
            let close = tag_end + memmem::find(&html[tag_end..end], &close_tag)?;
            let leaf_end = find_tag_end(html, close, end)?;
            push(&mut root, &mut stack, Node { start: i, end: leaf_end, kind: NodeKind::Leaf });
            i = leaf_end;
            continue;
        } else {
            stack.push(Open {
                name,
                start: i,
                aid: tag_aid(&html[i..tag_end]),
                children: Vec::new(),
            });
        }
        i = tag_end;
    }

    stack.is_empty().then_some(root)
}

/// Group `children` (all children of the element whose aid is `parent_aid`)
/// into pieces of at most `CHUNK_SIZE` bytes made of whole nodes.
///
/// A child too large on its own is split instead: an element by recursing
/// into its children (its start and end tags stay in the skeleton), a text
/// node at character boundaries. Elements without an aid can't be named by a
/// selector and leaves can't be split, so those become oversized chunks
/// (calibre does the same).
fn chunk_children(html: &[u8], children: &[Node], parent_aid: &str, out: &mut Vec<Piece>) {
    let mut run: Option<(usize, usize)> = None;

    for child in children {
        if child.end - child.start > CHUNK_SIZE {
            match &child.kind {
                NodeKind::Element { aid: Some(aid), children: inner } if !inner.is_empty() => {
                    flush_run(html, run.take(), parent_aid, out);
                    chunk_children(html, inner, aid, out);
                    continue;
                }
                NodeKind::Text => {
                    flush_run(html, run.take(), parent_aid, out);
                    let mut parts = split_text(html, child.start, child.end);
                    // Leave the last part open so small following siblings
                    // can join it.
                    run = parts.pop();
                    for (start, end) in parts {
                        out.push(Piece { start, end, parent_aid: parent_aid.to_string() });
                    }
                    continue;
                }
                _ => {}
            }
        }

        run = match run {
            Some((start, _)) if child.end - start <= CHUNK_SIZE => Some((start, child.end)),
            full => {
                flush_run(html, full, parent_aid, out);
                Some((child.start, child.end))
            }
        };
    }

    flush_run(html, run, parent_aid, out);
}

fn flush_run(html: &[u8], run: Option<(usize, usize)>, parent_aid: &str, out: &mut Vec<Piece>) {
    if let Some((start, end)) = run {
        // Whitespace between stepped-into elements stays in the skeleton.
        if !html[start..end].iter().all(u8::is_ascii_whitespace) {
            out.push(Piece { start, end, parent_aid: parent_aid.to_string() });
        }
    }
}

/// Split the text node `html[start..end]` into ranges of at most
/// `CHUNK_SIZE` bytes, never inside a UTF-8 sequence or a character
/// reference (`&amp;`).
fn split_text(html: &[u8], start: usize, end: usize) -> Vec<(usize, usize)> {
    let mut parts = Vec::new();
    let mut s = start;
    while end - s > CHUNK_SIZE {
        let mut cut = s + CHUNK_SIZE;
        while cut > s && (html[cut] & 0xC0) == 0x80 {
            cut -= 1;
        }
        let window = cut.saturating_sub(32).max(s);
        if let Some(amp) = memchr::memrchr(b'&', &html[window..cut]).map(|p| window + p)
            && !html[amp..cut].contains(&b';')
        {
            cut = amp;
        }
        if cut == s {
            break;
        }
        parts.push((s, cut));
        s = cut;
    }
    parts.push((s, end));
    parts
}

/// Split an HTML document into `(scaffold_before_body_content, body_content,
/// scaffold_after_body_content)`. The split point is just past the `<body…>`
/// opening tag and just before the matching `</body>`. If either tag isn't
/// found, returns the whole document as scaffold with empty body content (so
/// minimal files don't break the writer).
fn split_body(html: &[u8]) -> (&[u8], &[u8], &[u8]) {
    use memchr::memmem;

    // Find `<body` case-insensitively. memmem is case-sensitive, but body
    // tags in serialised XHTML are reliably lowercase, so a direct search
    // is fine; fall back to a manual scan if needed.
    let body_open_start = match memmem::find(html, b"<body") {
        Some(i) => i,
        None => return (html, &[], &[]),
    };

    // Find the `>` that closes the opening tag, respecting quoted attribute
    // values.
    let mut p = body_open_start + 5;
    let mut in_quote: Option<u8> = None;
    while p < html.len() {
        let c = html[p];
        match in_quote {
            Some(q) if c == q => in_quote = None,
            None => match c {
                b'"' | b'\'' => in_quote = Some(c),
                b'>' => break,
                _ => {}
            },
            _ => {}
        }
        p += 1;
    }
    if p >= html.len() {
        return (html, &[], &[]);
    }
    let body_open_end = p + 1;

    let body_close_start = match memmem::rfind(html, b"</body>") {
        Some(i) if i >= body_open_end => i,
        _ => return (html, &[], &[]),
    };

    (
        &html[..body_open_end],
        &html[body_open_end..body_close_start],
        &html[body_close_start..],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse Kindle base32 encoding back to number
    /// Kindle uses custom base32: 0-9 (0-9), A-V (10-31)
    pub(super) fn from_base32(s: &str) -> u32 {
        let mut result = 0u32;
        for c in s.chars() {
            result = result.saturating_mul(32);
            let val = match c {
                '0'..='9' => c as u32 - '0' as u32,
                'A'..='V' => c as u32 - 'A' as u32 + 10,
                'a'..='v' => c as u32 - 'a' as u32 + 10,
                _ => continue,
            };
            result = result.saturating_add(val);
        }
        result
    }

    #[test]
    fn test_from_base32() {
        assert_eq!(from_base32("0000"), 0);
        assert_eq!(from_base32("0001"), 1);
        assert_eq!(from_base32("000V"), 31);
        assert_eq!(from_base32("0010"), 32);
    }

    #[test]
    fn test_add_aids() {
        use crate::mobi::writer_transform::add_aid_attributes_fast;
        let mut chunker = Chunker::new();
        let html = b"<html><body><p>Hello</p><div>World</div></body></html>";
        let result = add_aid_attributes_fast(
            html,
            "test.xhtml",
            &mut chunker.aid_counter,
            &mut chunker.id_map,
        );
        let result_str = String::from_utf8_lossy(&result.html);
        assert!(result_str.contains("aid=\"0000\""));
        assert!(result_str.contains("aid=\"0001\""));
    }
}

#[cfg(test)]
mod chunker_tests {
    use super::*;

    #[test]
    fn aid_offset_points_at_tag_start() {
        // kindle:pos:fid:off targets must land on the element's `<`, not on
        // the ` aid="` attribute inside the start tag (calibre uses the
        // match start of `<[^>]+? aid=`). The device can't resolve a position
        // in the middle of a tag, so footnote links jumped to the wrong place.
        let html = b"<html><head></head><body><p>x</p>\
            <p class=\"note\" id=\"target\">note</p></body></html>"
            .to_vec();
        let mut chunker = Chunker::new();
        let result = chunker.process(&[("a.xhtml".to_string(), html)]);
        let aid = &result.id_map[&("a.xhtml".to_string(), "target".to_string())];
        let (seq, off, _) = result.aid_offset_map[aid];
        let chunk = &result.chunk_table[seq];
        let skel_len = result.skel_table[0].length;
        let chunk_bytes = &result.text[skel_len..skel_len + chunk.length];
        assert!(
            chunk_bytes[off..].starts_with(b"<p class=\"note\""),
            "offset {off} lands on {:?}",
            String::from_utf8_lossy(&chunk_bytes[off..])
        );
    }

    /// `html` after the chunker's preprocessing (namespace stripping, aid
    /// annotation): what reassembling the skeleton and chunks must give back.
    fn annotated(html: &[u8]) -> Vec<u8> {
        use crate::mobi::writer_transform::{add_aid_attributes_fast, strip_xml_namespaces};
        let mut counter = 0;
        let mut ids = HashMap::new();
        add_aid_attributes_fast(&strip_xml_namespaces(html), "a.xhtml", &mut counter, &mut ids)
            .html
    }

    fn chunk_file(html: &[u8]) -> ChunkerResult {
        Chunker::new().process(&[("a.xhtml".to_string(), html.to_vec())])
    }

    /// Rebuild each file the way Kindle does: take the skeleton bytes from
    /// the rawML, then insert each following chunk at its `insert_pos`.
    /// Returns, per file, the document and the (offset, chunk index) of
    /// every insertion.
    fn reassemble(result: &ChunkerResult) -> Vec<(Vec<u8>, Vec<(usize, usize)>)> {
        let mut files = Vec::new();
        let mut ci = 0;
        for skel in &result.skel_table {
            let mut doc = result.text[skel.start_pos..skel.start_pos + skel.length].to_vec();
            let mut raw_pos = skel.start_pos + skel.length;
            let mut inserts = Vec::new();
            for _ in 0..skel.chunk_count {
                let chunk = &result.chunk_table[ci];
                let at = chunk.insert_pos - skel.start_pos;
                let raw = &result.text[raw_pos..raw_pos + chunk.length];
                doc.splice(at..at, raw.iter().copied());
                inserts.push((at, ci));
                raw_pos += chunk.length;
                ci += 1;
            }
            files.push((doc, inserts));
        }
        files
    }

    /// The aid of the innermost element open at `pos` in `doc`.
    fn enclosing_aid(doc: &[u8], pos: usize) -> Option<String> {
        let nodes = parse_nodes(doc, 0, doc.len()).expect("document is balanced");
        let mut aid = None;
        let mut level = &nodes;
        'descend: loop {
            for node in level {
                if let NodeKind::Element { aid: node_aid, children } = &node.kind
                    && node.start < pos
                    && pos < node.end
                {
                    aid = node_aid.clone();
                    level = children;
                    continue 'descend;
                }
            }
            return aid;
        }
    }

    /// A chapter shaped like real EPUBs: one `<section>` wrapping everything,
    /// so there is no body-level boundary to cut at, and paragraphs whose
    /// text runs across `<i>`/`<b>` and multibyte characters.
    fn wrapped_chapter() -> Vec<u8> {
        let para = "<p class=\"txt\">Le projectile s’enfonça dans son épaule avec un \
                    <i>smack</i> mouillé. Ça ne la fit même pas vaciller. <b>Merde</b>, \
                    j’allais devoir aller là-bas &amp; l’achever.</p>\n";
        let long_para = format!("<p>{}</p>", "Une très longue phrase sans fin &amp; ".repeat(400));
        format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<html xmlns=\"http://www.w3.org/1999/xhtml\">\
             <head><title>42</title></head><body><section class=\"chap\" id=\"chap-042\">\
             <h2>[42]</h2><div class=\"dev\"><span id=\"page_531\"/><!--debut-->{}{}{}</div>\
             </section></body></html>",
            para.repeat(60),
            long_para,
            para.repeat(30),
        )
        .into_bytes()
    }

    #[test]
    fn reassembly_reproduces_the_file() {
        let html = wrapped_chapter();
        let result = chunk_file(&html);
        assert!(result.chunk_table.len() > 3, "chapter should span several chunks");
        let files = reassemble(&result);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0, annotated(&html));
    }

    #[test]
    fn chunks_are_whole_children_of_their_selector() {
        // Regression: chunks used to be cut after any end tag once 8 KB was
        // reached, e.g. after an `</i>` mid-sentence, and all claimed the
        // body as parent. Kindle drops such fragments, losing ~3 pages.
        let result = chunk_file(&wrapped_chapter());
        let (doc, inserts) = &reassemble(&result)[0];
        for &(at, ci) in inserts {
            let chunk = &result.chunk_table[ci];
            let raw = &doc[at..at + chunk.length];
            assert!(chunk.length <= CHUNK_SIZE, "chunk {ci} is {} bytes", chunk.length);
            assert!(std::str::from_utf8(raw).is_ok(), "chunk {ci} splits a character");
            assert!(
                parse_nodes(raw, 0, raw.len()).is_some(),
                "chunk {ci} is not a run of whole nodes: {:?}",
                String::from_utf8_lossy(raw)
            );
            let parent = enclosing_aid(doc, at).expect("chunk is inside an element");
            assert_eq!(chunk.selector, format!("P-//*[@aid='{parent}']"), "chunk {ci}");
        }
        // The heading is chunked inside `<section>`, paragraphs inside
        // `div.dev`, and the long paragraph inside its own `<p>`.
        let selectors: std::collections::HashSet<&str> =
            result.chunk_table.iter().map(|c| c.selector.as_str()).collect();
        assert_eq!(selectors.len(), 3, "{selectors:?}");
    }

    #[test]
    fn long_text_is_split_between_characters_and_entities() {
        let text = "é’&amp;x ".repeat(2000);
        let html = format!("<html><head></head><body><p>{text}</p></body></html>");
        let result = chunk_file(html.as_bytes());
        assert!(result.chunk_table.len() > 1);
        let (doc, inserts) = &reassemble(&result)[0];
        assert_eq!(*doc, annotated(html.as_bytes()));
        for &(at, ci) in inserts {
            let raw = &doc[at..at + result.chunk_table[ci].length];
            let raw = std::str::from_utf8(raw).expect("chunk splits a character");
            assert!(!raw.starts_with("amp;") && !raw.starts_with("mp;") && !raw.starts_with("p;"));
        }
    }

    #[test]
    fn unbalanced_markup_stays_in_one_chunk() {
        let html = format!(
            "<html><head></head><body><div>{}</body></html>",
            "<p>never closed div</p>".repeat(1000)
        );
        let result = chunk_file(html.as_bytes());
        assert_eq!(result.chunk_table.len(), 1);
        assert_eq!(reassemble(&result)[0].0, annotated(html.as_bytes()));
    }

    #[test]
    fn chunker_under_target_yields_single_chunk() {
        let html = format!("<html><head></head><body>{}</body></html>", "<p>tiny</p>".repeat(10));
        let result = chunk_file(html.as_bytes());
        assert_eq!(result.chunk_table.len(), 1);
        let body_aid = extract_body_aid(&annotated(html.as_bytes())).unwrap();
        assert_eq!(result.chunk_table[0].selector, format!("P-//*[@aid='{body_aid}']"));
        assert_eq!(reassemble(&result)[0].0, annotated(html.as_bytes()));
    }

    #[test]
    fn chunker_empty_body() {
        let result = chunk_file(b"<html><head></head><body></body></html>");
        assert_eq!(result.chunk_table.len(), 1);
        assert_eq!(result.chunk_table[0].length, 0);
    }

    #[test]
    fn rebuild_keeps_multi_chunk_body_contiguous() {
        // A body larger than CHUNK_SIZE splits into several chunks. rebuild()
        // must reassemble them contiguously between the scaffold prefix and
        // suffix — it used to splice the </body></html> suffix between the
        // first and second chunk, corrupting every aid offset past ~8 KB.
        let prefix = b"<html><head></head><body aid=\"0000\">";
        let suffix = b"</body></html>";
        let chunk_a = vec![b'A'; CHUNK_SIZE]; // fills the first chunk
        let chunk_b = b"<p>tail paragraph</p>".to_vec();
        let start_pos = 0;

        let skeleton: Vec<u8> = prefix.iter().chain(suffix.iter()).copied().collect();
        let base = start_pos + prefix.len();
        let skel = Skeleton {
            file_number: 0,
            skeleton,
            start_pos,
            chunks: vec![
                Chunk {
                    raw: chunk_a.clone(),
                    insert_pos: base,
                    selector: String::new(),
                    file_number: 0,
                    sequence_number: 0,
                    start_pos: 0,
                },
                Chunk {
                    raw: chunk_b.clone(),
                    insert_pos: base + chunk_a.len(),
                    selector: String::new(),
                    file_number: 0,
                    sequence_number: 1,
                    start_pos: chunk_a.len(),
                },
            ],
        };

        let rebuilt = skel.rebuild();
        let expected: Vec<u8> = prefix
            .iter()
            .chain(chunk_a.iter())
            .chain(chunk_b.iter())
            .chain(suffix.iter())
            .copied()
            .collect();
        assert_eq!(rebuilt, expected, "suffix must follow all body chunks");
        // The tail paragraph must appear after the bulk body, not before the
        // suffix-that-used-to-be-spliced-early.
        let body_end = rebuilt
            .windows(suffix.len())
            .position(|w| w == suffix)
            .unwrap();
        let tail = rebuilt
            .windows(chunk_b.len())
            .position(|w| w == chunk_b.as_slice())
            .unwrap();
        assert!(
            tail < body_end,
            "tail chunk must precede the closing suffix"
        );
    }

    #[test]
    fn chunk_selectors_are_per_file_unique() {
        // Regression test for issue #10: every spine file's chunks must
        // reference *that file's* body aid in their selector, not a single
        // shared aid. With shared selectors Kindle conflates positions
        // across files during layout and locks up.
        let html = |n| {
            format!(
                "<html><head><title>F{n}</title></head><body>\
                 <section><p>hello from file {n}</p></section>\
                 </body></html>"
            )
            .into_bytes()
        };
        let files = vec![
            ("a.xhtml".to_string(), html(0)),
            ("b.xhtml".to_string(), html(1)),
            ("c.xhtml".to_string(), html(2)),
        ];
        let result = Chunker::new().process(&files);

        let selectors_per_file: std::collections::HashMap<usize, std::collections::HashSet<&str>> =
            result
                .chunk_table
                .iter()
                .fold(std::collections::HashMap::new(), |mut acc, c| {
                    acc.entry(c.file_number)
                        .or_default()
                        .insert(c.selector.as_str());
                    acc
                });

        let all_selectors: std::collections::HashSet<&str> = result
            .chunk_table
            .iter()
            .map(|c| c.selector.as_str())
            .collect();

        assert_eq!(
            all_selectors.len(),
            selectors_per_file.len(),
            "each spine file must have a distinct chunk selector; \
             got {:?}",
            all_selectors,
        );
    }
}
