//! Writing metadata into an OPF: [`apply_edit`] rewrites only the elements
//! Librarium manages, in place, and leaves every other byte alone.
//!
//! Managed elements, and what an edit does to them:
//!
//! | Field        | Elements | Empty value |
//! |--------------|----------|-------------|
//! | title        | the main `dc:title` (subtitles and other titles are kept) | file left as is |
//! | authors      | `dc:creator`s with role `aut` or no role (editors, translators… are kept) | file left as is |
//! | language     | the first `dc:language` | file left as is (the element is mandatory) |
//! | description  | `dc:description` | removed |
//! | categories   | `dc:subject` | removed |
//! | series       | `calibre:series` + `calibre:series_index`, and an existing EPUB 3 series collection + `group-position` | removed |
//! | ISBN         | an ISBN `dc:identifier` that is **not** the unique identifier, or a new one | removed (except the unique identifier) |
//! | rating       | `calibre:rating` | removed |
//!
//! The `dc:identifier` named by `package@unique-identifier` is never
//! touched: readers key their data (positions, highlights) on it, and it is
//! the key that de-obfuscates embedded fonts.
//!
//! Elements whose current value already says what the edit asks for are not
//! rewritten (a description stored as HTML stays HTML until it is actually
//! changed). New elements follow the file's conventions: its namespace
//! prefixes, its indentation, its line breaks.
//!
//! Every edit is checked before it is returned ([`check_edit`]): the result
//! must parse, everything outside `<metadata>` must be byte-identical, the
//! unique identifier must be unchanged, and reading the result must give the
//! edited values back. Any failure is an error, and the caller writes nothing.

use std::collections::BTreeSet;
use std::ops::Range;

use super::isbn::{self, IsbnForm};
use super::metadata::{
    self, author_entries, clean_text, dc_entries, description_text, is_meta_named, isbn_entries,
    main_title, parse_index, refined, refinements, series_collection, subtitle, CALIBRE_RATING,
    CALIBRE_SERIES, CALIBRE_SERIES_INDEX,
};
use super::opf::{Element, EntryKind, Package, Span, DC_NS};
use super::xml::{
    escape_attr, escape_text, indentation_at, line_break, removal_range, scan_start_tag, Splicer,
};
use crate::models::book::BookMetadata;

// ── What to change ────────────────────────────────────────────────────────────

/// A change to one optional field.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) enum Change<T> {
    /// Leave the file as it is.
    #[default]
    Keep,
    /// Make the file say this.
    Set(T),
    /// Remove the field from the file.
    Clear,
}

/// The metadata changes to make to an OPF. `None` / [`Change::Keep`] leaves
/// a field as the file has it.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct MetadataEdit {
    pub title: Option<String>,
    /// Never cleared: a book always keeps its authors.
    pub authors: Option<Vec<String>>,
    /// Never cleared: `dc:language` is mandatory in EPUB.
    pub language: Option<String>,
    pub description: Change<String>,
    pub categories: Change<Vec<String>>,
    /// Series name and position.
    pub series: Change<(String, Option<f32>)>,
    /// Bare ISBN digits, checksum verified.
    pub isbn: Change<String>,
    /// 1–5 stars.
    pub rating: Change<u8>,
}

/// Values an older index may hold instead of "unknown" (unknown is `""`, and
/// the index clears these on load: `BookMetadata::clear_placeholders`).
/// Still refused here, as a last line of defence: they must never be
/// written into a book.
const PLACEHOLDERS: [&str; 2] = ["Unknown title", "Unknown author"];

/// `s` trimmed, unless it is blank or a placeholder.
fn known_text(s: &str) -> Option<String> {
    let s = clean_text(s);
    (!s.is_empty() && !PLACEHOLDERS.contains(&s.as_str())).then_some(s)
}

/// `Set` for a non-blank value, `Clear` otherwise.
fn text_change(value: &Option<String>) -> Change<String> {
    match value.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        Some(v) => Change::Set(v.to_string()),
        None => Change::Clear,
    }
}

/// The individual authors of an author string: Librarium (like Calibre)
/// joins several authors with `" & "`.
pub(crate) fn split_authors(author: &str) -> Vec<String> {
    clean_text(author)
        .split(" & ")
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .map(str::to_string)
        .collect()
}

impl MetadataEdit {
    /// Make the file say exactly `meta`: every field is written, and empty
    /// optional fields are removed from the file. A blank (or placeholder)
    /// title or author leaves the file's value; so does an ISBN that doesn't
    /// pass the checksum (it stays in the library only).
    pub(crate) fn full(meta: &BookMetadata) -> Self {
        let categories: Vec<String> = meta
            .categories
            .iter()
            .map(|c| clean_text(c))
            .filter(|c| !c.is_empty())
            .collect();
        let isbn_text = meta.isbn.as_deref().map(str::trim).unwrap_or_default();
        Self {
            title: known_text(&meta.title),
            authors: known_text(&meta.author)
                .map(|a| split_authors(&a))
                .filter(|a| !a.is_empty()),
            language: meta
                .language
                .as_deref()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_string),
            description: text_change(&meta.description),
            categories: if categories.is_empty() {
                Change::Clear
            } else {
                Change::Set(categories)
            },
            series: match meta
                .series
                .as_deref()
                .map(clean_text)
                .filter(|s| !s.is_empty())
            {
                Some(name) => Change::Set((name, meta.series_index.filter(|i| i.is_finite()))),
                None => Change::Clear,
            },
            isbn: match isbn::clean(isbn_text).filter(|i| isbn::is_valid(i)) {
                Some(valid) => Change::Set(valid),
                None if isbn_text.is_empty() => Change::Clear,
                None => Change::Keep,
            },
            rating: match meta.rating {
                Some(r) if r > 0 => Change::Set(r.min(5)),
                _ => Change::Clear,
            },
        }
    }

    /// Like [`MetadataEdit::full`], but nothing is ever removed: empty
    /// fields leave the file's value. For writing the library's metadata
    /// into a file whose own metadata the library never read (a freshly
    /// converted book).
    pub(crate) fn overlay(meta: &BookMetadata) -> Self {
        fn set_only<T>(change: Change<T>) -> Change<T> {
            match change {
                Change::Clear => Change::Keep,
                other => other,
            }
        }
        let full = Self::full(meta);
        Self {
            description: set_only(full.description),
            categories: set_only(full.categories),
            series: set_only(full.series),
            isbn: set_only(full.isbn),
            rating: set_only(full.rating),
            ..full
        }
    }

    /// Only what changed between `before` (what the library knew) and
    /// `after` (the edited metadata), as [`MetadataEdit::full`] would write
    /// it. Fields the user didn't touch are left as the file has them — so
    /// values the library never read from the file (an EPUB 3 series, a
    /// Calibre rating, a second author, in books indexed from an older format)
    /// survive an unrelated edit.
    pub(crate) fn changes(before: &BookMetadata, after: &BookMetadata) -> Self {
        let (old, new) = (Self::full(before), Self::full(after));
        fn keep_same<T: PartialEq>(old: Change<T>, new: Change<T>) -> Change<T> {
            if old == new {
                Change::Keep
            } else {
                new
            }
        }
        Self {
            title: new.title.filter(|t| old.title.as_ref() != Some(t)),
            authors: new.authors.filter(|a| old.authors.as_ref() != Some(a)),
            language: new.language.filter(|l| old.language.as_ref() != Some(l)),
            description: keep_same(old.description, new.description),
            categories: keep_same(old.categories, new.categories),
            series: keep_same(old.series, new.series),
            isbn: keep_same(old.isbn, new.isbn),
            rating: keep_same(old.rating, new.rating),
        }
    }

    /// Whether this edit changes nothing at all.
    pub(crate) fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

// ── Applying an edit ──────────────────────────────────────────────────────────

/// Apply `edit` to the OPF `text` (already parsed as `pkg`). Returns the new
/// text, or `None` if the file already says everything the edit asks for.
pub(crate) fn apply_edit(
    text: &str,
    pkg: &Package,
    edit: &MetadataEdit,
) -> Result<Option<String>, String> {
    let section = pkg
        .metadata_section
        .as_ref()
        .ok_or("The OPF has no <metadata> section")?;
    if section.span.is_self_closing() {
        return Err("The OPF's <metadata> section is empty".to_string());
    }
    let current = metadata::read(pkg);
    let mut plan = Plan::new(text, pkg);

    if let Some(title) = &edit.title {
        plan.title(&current, title);
    }
    if let Some(authors) = &edit.authors {
        plan.authors(&current, authors);
    }
    if let Some(language) = &edit.language {
        plan.language(&current, language);
    }
    plan.description(&current, &edit.description);
    plan.categories(&current, &edit.categories);
    plan.series(&current, &edit.series);
    plan.isbn(&current, &edit.isbn);
    plan.rating(&current, &edit.rating);

    if plan.is_empty() {
        return Ok(None);
    }
    let new_text = plan.finish()?;
    check_edit(text, pkg, &new_text, edit)?;
    Ok(Some(new_text))
}

/// [`apply_edit`] with [`MetadataEdit::full`]: make the OPF say exactly
/// `meta`. (The app itself writes edits with [`MetadataEdit::changes`] and
/// conversions with [`MetadataEdit::overlay`]; this is the reference
/// behaviour the tests check round-trips against.)
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn apply_metadata(
    text: &str,
    pkg: &Package,
    meta: &BookMetadata,
) -> Result<Option<String>, String> {
    apply_edit(text, pkg, &MetadataEdit::full(meta))
}

/// Where a new element goes.
#[derive(Debug, Clone, Copy)]
enum Near {
    /// After the last Dublin Core element of this name.
    Dc(&'static str),
    /// After the last `<meta name="…">` with this name.
    MetaNamed(&'static str),
    /// After the last `<meta>`.
    AnyMeta,
    /// After this entry.
    Entry(usize),
}

/// The edits planned for one OPF, resolved into byte splices by
/// [`Plan::finish`].
struct Plan<'a> {
    text: &'a str,
    pkg: &'a Package,
    /// Entries to delete.
    removed: BTreeSet<usize>,
    /// Replacements of whole elements or parts of them (text ranges).
    replacements: Vec<(Range<usize>, String)>,
    /// New elements: placement preferences, then the element.
    inserts: Vec<(Vec<Near>, bool, String)>,
}

impl<'a> Plan<'a> {
    fn new(text: &'a str, pkg: &'a Package) -> Self {
        Self {
            text,
            pkg,
            removed: BTreeSet::new(),
            replacements: Vec::new(),
            inserts: Vec::new(),
        }
    }

    fn is_empty(&self) -> bool {
        self.removed.is_empty() && self.replacements.is_empty() && self.inserts.is_empty()
    }

    fn element(&self, i: usize) -> &'a Element {
        &self.pkg.metadata[i].element
    }

    // ── Primitive operations ────────────────────────────────────────────────

    /// Remove entry `i`, and the EPUB 3 refinements that point at it (a
    /// creator's `role` and `file-as`…), which would otherwise dangle.
    fn remove(&mut self, i: usize) {
        let mut pending = vec![i];
        while let Some(i) = pending.pop() {
            if self.removed.insert(i) {
                if let Some(id) = self.element(i).id() {
                    pending.extend(refinements(self.pkg, id.trim()));
                }
            }
        }
    }

    /// Replace the text content of entry `i` with `value`.
    fn set_content(&mut self, i: usize, value: &str) {
        let element = self.element(i);
        self.replacements
            .push(content_replacement(self.text, element, value));
    }

    /// Set the unprefixed attribute `name` of entry `i`.
    fn set_attr(&mut self, i: usize, name: &str, value: &str) {
        let element = self.element(i);
        self.replacements
            .push(attribute_replacement(self.text, element, name, value));
    }

    fn insert(&mut self, near: &[Near], at_start: bool, xml: String) {
        self.inserts.push((near.to_vec(), at_start, xml));
    }

    /// Make the entries `existing` say `desired`, in that order. Entries
    /// whose text already matches a desired value are kept verbatim (with
    /// their attributes: `opf:file-as`, ids…); the others are dropped; a
    /// desired value without a matching entry gets `new_elements[k]` (one
    /// new element per desired value, built by the caller). The list takes
    /// the place of the first existing entry.
    fn replace_list(
        &mut self,
        existing: &[usize],
        desired: &[String],
        new_elements: Vec<String>,
        near: &[Near],
    ) {
        let Some(&first) = existing.first() else {
            for xml in new_elements {
                self.insert(near, false, xml);
            }
            return;
        };
        let mut used = vec![false; existing.len()];
        let mut pieces = Vec::new();
        for (value, new_element) in desired.iter().zip(new_elements) {
            let reused = existing
                .iter()
                .enumerate()
                .find(|(k, &i)| !used[*k] && clean_text(&self.pkg.metadata[i].text) == *value)
                .map(|(k, _)| k);
            match reused {
                Some(k) => {
                    used[k] = true;
                    pieces.push(self.text[self.element(existing[k]).span.range()].to_string());
                }
                None => pieces.push(new_element),
            }
        }
        let separator = separator_before(self.text, self.element(first).span.start);
        self.replacements
            .push((self.element(first).span.range(), pieces.join(&separator)));
        for (k, &i) in existing.iter().enumerate() {
            match (k, used[k]) {
                // The first entry's place now holds the list. If it was
                // reused it keeps its id there, so its refinements stay.
                (0, true) => {}
                (0, false) => {
                    if let Some(id) = self.element(i).id() {
                        for r in refinements(self.pkg, id.trim()).collect::<Vec<_>>() {
                            self.remove(r);
                        }
                    }
                }
                // Moved into the list: delete the old copy, keep refinements.
                (_, true) => {
                    self.removed.insert(i);
                }
                (_, false) => self.remove(i),
            }
        }
    }

    // ── New elements ────────────────────────────────────────────────────────

    /// A new Dublin Core element, spelled with the file's prefix (declaring
    /// the namespace on the element itself if the file never declared it).
    fn new_dc(&self, local: &str, attrs: &str, value: &str) -> String {
        let (qname, declaration) = match self.pkg.scope.dc_prefix.as_deref() {
            Some("") => (local.to_string(), String::new()),
            Some(prefix) => (format!("{prefix}:{local}"), String::new()),
            None => (format!("dc:{local}"), format!(" xmlns:dc=\"{DC_NS}\"")),
        };
        format!(
            "<{qname}{declaration}{attrs}>{}</{qname}>",
            escape_text(value)
        )
    }

    /// `<meta name="…" content="…"/>` (EPUB 2 style, also valid in EPUB 3).
    fn new_meta_named(&self, name: &str, content: &str) -> String {
        format!(
            "<{} name=\"{}\" content=\"{}\"/>",
            meta_qname(self.pkg),
            escape_attr(name, '"'),
            escape_attr(content, '"')
        )
    }

    /// `<meta refines="#id" property="…">value</meta>` (EPUB 3).
    fn new_refinement(&self, id: &str, property: &str, value: &str) -> String {
        let q = meta_qname(self.pkg);
        format!(
            "<{q} refines=\"#{}\" property=\"{property}\">{}</{q}>",
            escape_attr(id, '"'),
            escape_text(value)
        )
    }

    // ── Fields ──────────────────────────────────────────────────────────────

    fn title(&mut self, current: &metadata::EpubMetadata, title: &str) {
        let title = clean_text(title);
        if current.title.as_deref() == Some(title.as_str()) {
            return;
        }
        // The first title that isn't a subtitle, even if it's empty.
        let slot = main_title(self.pkg)
            .or_else(|| dc_entries(self.pkg, "title").find(|&i| Some(i) != subtitle(self.pkg)));
        match slot {
            Some(i) => self.set_content(i, &title),
            None => {
                let xml = self.new_dc("title", "", &title);
                self.insert(&[Near::Dc("title")], true, xml);
            }
        }
    }

    fn authors(&mut self, current: &metadata::EpubMetadata, authors: &[String]) {
        let desired: Vec<String> = authors
            .iter()
            .map(|a| clean_text(a))
            .filter(|a| !a.is_empty())
            .collect();
        if desired.is_empty() || current.authors == desired {
            return;
        }
        // EPUB 2 states the role on the element; EPUB 3 would need a
        // refinement, and a creator without a role already counts as an author.
        let role = match (&self.pkg.scope.opf_prefix, self.pkg.is_epub3()) {
            (Some(prefix), false) => format!(" {prefix}:role=\"aut\""),
            _ => String::new(),
        };
        let new_elements = desired
            .iter()
            .map(|name| self.new_dc("creator", &role, name))
            .collect();
        self.replace_list(
            &author_entries(self.pkg),
            &desired,
            new_elements,
            &[Near::Dc("creator"), Near::Dc("title")],
        );
    }

    fn language(&mut self, current: &metadata::EpubMetadata, language: &str) {
        let language = language.trim();
        if current
            .language
            .as_deref()
            .is_some_and(|l| l.eq_ignore_ascii_case(language))
        {
            return;
        }
        match dc_entries(self.pkg, "language").next() {
            Some(i) => self.set_content(i, language),
            None => {
                let xml = self.new_dc("language", "", language);
                self.insert(
                    &[
                        Near::Dc("language"),
                        Near::Dc("identifier"),
                        Near::Dc("title"),
                    ],
                    false,
                    xml,
                );
            }
        }
    }

    fn description(&mut self, current: &metadata::EpubMetadata, change: &Change<String>) {
        let entries: Vec<usize> = dc_entries(self.pkg, "description").collect();
        match change {
            Change::Keep => {}
            Change::Set(description) => {
                let unchanged = current.description.as_deref().map(clean_text)
                    == Some(clean_text(&description_text(description)));
                if unchanged {
                    return;
                }
                match entries.first() {
                    Some(&i) => self.set_content(i, description.trim()),
                    None => {
                        let xml = self.new_dc("description", "", description.trim());
                        self.insert(&[Near::Dc("description"), Near::Dc("language")], false, xml);
                    }
                }
            }
            Change::Clear => entries.into_iter().for_each(|i| self.remove(i)),
        }
    }

    fn categories(&mut self, current: &metadata::EpubMetadata, change: &Change<Vec<String>>) {
        let entries: Vec<usize> = dc_entries(self.pkg, "subject").collect();
        match change {
            Change::Keep => {}
            Change::Set(categories) if !categories.is_empty() => {
                if current.categories == *categories {
                    return;
                }
                let new_elements = categories
                    .iter()
                    .map(|c| self.new_dc("subject", "", c))
                    .collect();
                let near = [
                    Near::Dc("subject"),
                    Near::Dc("description"),
                    Near::Dc("language"),
                ];
                self.replace_list(&entries, categories, new_elements, &near);
            }
            Change::Set(_) | Change::Clear => entries.into_iter().for_each(|i| self.remove(i)),
        }
    }

    fn series(&mut self, current: &metadata::EpubMetadata, change: &Change<(String, Option<f32>)>) {
        let pkg = self.pkg;
        let calibre_series = metas_named(pkg, CALIBRE_SERIES);
        let calibre_index = metas_named(pkg, CALIBRE_SERIES_INDEX);
        let collection = series_collection(pkg);
        match change {
            Change::Keep => {}
            Change::Set((name, index)) => {
                if current.series.as_deref() == Some(name.as_str())
                    && same_index(current.series_index, *index)
                {
                    return;
                }
                let content_of = |i: usize| pkg.metadata[i].meta_content().map(clean_text);
                // Calibre's representation: written unless the file only has
                // an EPUB 3 collection (which is then updated instead).
                if let Some(&s) = calibre_series.first() {
                    if content_of(s).as_deref() != Some(name.as_str()) {
                        self.set_attr(s, "content", name);
                    }
                }
                if !calibre_series.is_empty() || collection.is_none() {
                    if calibre_series.is_empty() {
                        let xml = self.new_meta_named(CALIBRE_SERIES, name);
                        self.insert(
                            &[Near::MetaNamed(CALIBRE_SERIES_INDEX), Near::AnyMeta],
                            false,
                            xml,
                        );
                    }
                    match (index, calibre_index.first()) {
                        (Some(v), Some(&i)) => {
                            if !same_index(content_of(i).as_deref().and_then(parse_index), Some(*v))
                            {
                                self.set_attr(i, "content", &format_index(*v));
                            }
                        }
                        (Some(v), None) => {
                            let xml = self.new_meta_named(CALIBRE_SERIES_INDEX, &format_index(*v));
                            self.insert(
                                &[Near::MetaNamed(CALIBRE_SERIES), Near::AnyMeta],
                                false,
                                xml,
                            );
                        }
                        (None, _) => calibre_index.iter().for_each(|&i| self.remove(i)),
                    }
                }
                // An existing EPUB 3 series collection is kept in step.
                if let Some(c) = collection {
                    if clean_text(&self.pkg.metadata[c].text) != *name {
                        self.set_content(c, name);
                    }
                    let position = refined(self.pkg, c, "group-position");
                    match (index, position) {
                        (Some(v), Some((p, value))) => {
                            if !same_index(parse_index(&value), Some(*v)) {
                                self.set_content(p, &format_index(*v));
                            }
                        }
                        (Some(v), None) => {
                            if let Some(id) = self.element(c).id() {
                                let xml = self.new_refinement(
                                    id.trim(),
                                    "group-position",
                                    &format_index(*v),
                                );
                                self.insert(&[Near::Entry(c)], false, xml);
                            }
                        }
                        (None, Some((p, _))) => self.remove(p),
                        (None, None) => {}
                    }
                }
            }
            Change::Clear => {
                for i in calibre_series
                    .into_iter()
                    .chain(calibre_index)
                    .chain(collection)
                {
                    self.remove(i);
                }
            }
        }
    }

    fn isbn(&mut self, current: &metadata::EpubMetadata, change: &Change<String>) {
        let entries = isbn_entries(self.pkg);
        match change {
            Change::Keep => {}
            Change::Set(isbn) => {
                if current.isbn.as_deref() == Some(isbn.as_str()) {
                    return;
                }
                // The ISBN the reader shows is the first one that isn't the
                // unique identifier: update that one, or add one.
                match entries.iter().find(|e| !e.is_unique_identifier) {
                    Some(entry) => {
                        let value =
                            isbn_value_like(&self.pkg.metadata[entry.index].text, entry.form, isbn);
                        self.set_content(entry.index, &value);
                    }
                    None => {
                        let xml = match (&self.pkg.scope.opf_prefix, self.pkg.is_epub3()) {
                            (Some(prefix), false) => self.new_dc(
                                "identifier",
                                &format!(" {prefix}:scheme=\"ISBN\""),
                                isbn,
                            ),
                            _ => self.new_dc("identifier", "", &format!("urn:isbn:{isbn}")),
                        };
                        self.insert(&[Near::Dc("identifier"), Near::Dc("title")], false, xml);
                    }
                }
            }
            Change::Clear => {
                for entry in entries.iter().filter(|e| !e.is_unique_identifier) {
                    self.remove(entry.index);
                }
            }
        }
    }

    fn rating(&mut self, current: &metadata::EpubMetadata, change: &Change<u8>) {
        let entries = metas_named(self.pkg, CALIBRE_RATING);
        match change {
            Change::Keep => {}
            Change::Set(stars) => {
                if current.rating == Some(*stars) {
                    return;
                }
                // Calibre's scale is 0–10.
                let content = (u32::from(*stars) * 2).to_string();
                match entries.first() {
                    Some(&i) => self.set_attr(i, "content", &content),
                    None => {
                        let xml = self.new_meta_named(CALIBRE_RATING, &content);
                        let near = [
                            Near::MetaNamed(CALIBRE_SERIES_INDEX),
                            Near::MetaNamed(CALIBRE_SERIES),
                            Near::AnyMeta,
                        ];
                        self.insert(&near, false, xml);
                    }
                }
            }
            Change::Clear => entries.into_iter().for_each(|i| self.remove(i)),
        }
    }

    // ── Resolution ──────────────────────────────────────────────────────────

    /// The last surviving entry matching `near`.
    fn last_matching(&self, near: Near) -> Option<usize> {
        let alive = |i: &usize| !self.removed.contains(i);
        let entries = &self.pkg.metadata;
        match near {
            Near::Dc(name) => dc_entries(self.pkg, name).filter(alive).last(),
            Near::MetaNamed(name) => (0..entries.len())
                .rev()
                .filter(alive)
                .find(|&i| is_meta_named(&entries[i], name)),
            Near::AnyMeta => (0..entries.len())
                .rev()
                .filter(alive)
                .find(|&i| entries[i].kind == EntryKind::Meta),
            Near::Entry(i) => Some(i).filter(alive),
        }
    }

    fn finish(self) -> Result<String, String> {
        let mut splicer = Splicer::new(self.text);
        for &i in &self.removed {
            splicer.delete(removal_range(self.text, self.element(i).span.range()));
        }
        for (range, with) in &self.replacements {
            splicer.replace(range.clone(), with.clone());
        }
        let section = self
            .pkg
            .metadata_section
            .as_ref()
            .expect("checked by apply_edit");
        let alive: Vec<usize> = (0..self.pkg.metadata.len())
            .filter(|i| !self.removed.contains(i))
            .collect();
        for (near, at_start, xml) in &self.inserts {
            let anchor = near.iter().find_map(|n| self.last_matching(*n));
            match (anchor, alive.first(), alive.last()) {
                (Some(after), _, _) => insert_after(&mut splicer, self.element(after).span, xml),
                (None, Some(&first), _) if *at_start => {
                    insert_before(&mut splicer, self.element(first).span, xml)
                }
                (None, _, Some(&last)) => insert_after(&mut splicer, self.element(last).span, xml),
                (None, _, None) => insert_into(&mut splicer, section.span, xml),
            }
        }
        splicer.finish()
    }
}

/// `isbn` written the way the identifier `old` (of form `form`) wrote its
/// ISBN: `urn:isbn:…`, `isbn:…` / `ISBN …` (prefix kept as written), or bare.
fn isbn_value_like(old: &str, form: IsbnForm, isbn: &str) -> String {
    let old = old.trim();
    match form {
        IsbnForm::Urn => format!("urn:isbn:{isbn}"),
        _ => match old.get(..5) {
            Some(prefix)
                if prefix.eq_ignore_ascii_case("isbn:") || prefix.eq_ignore_ascii_case("isbn ") =>
            {
                format!("{prefix}{isbn}")
            }
            _ => isbn.to_string(),
        },
    }
}

/// The EPUB 2 `<meta name="name">` entries.
fn metas_named(pkg: &Package, name: &str) -> Vec<usize> {
    pkg.metadata
        .iter()
        .enumerate()
        .filter(|(_, e)| is_meta_named(e, name))
        .map(|(i, _)| i)
        .collect()
}

/// Whether two series positions are the same (none, or equal).
fn same_index(a: Option<f32>, b: Option<f32>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => (a - b).abs() < 1e-4,
        _ => false,
    }
}

/// A series position as Calibre writes it: `2`, `2.5`.
fn format_index(v: f32) -> String {
    if v.fract() == 0.0 && v.abs() < 1e9 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

// ── Splicing helpers (also used by the cover editor) ──────────────────────────

/// The name to use for a new `<meta>`: the one the file already uses
/// (`meta`, `opf:meta`), else one that is in the OPF namespace here.
pub(super) fn meta_qname(pkg: &Package) -> String {
    if let Some(meta) = pkg.metadata.iter().find(|e| e.kind == EntryKind::Meta) {
        return meta.element.qname.clone();
    }
    match (&pkg.scope.opf_prefix, pkg.scope.default_is_opf) {
        (Some(prefix), false) => format!("{prefix}:meta"),
        _ => "meta".to_string(),
    }
}

/// What separates a new element from the one at `pos`: a line break and the
/// same indentation when that element starts its own line, nothing in a
/// single-line file.
fn separator_before(text: &str, pos: usize) -> String {
    match indentation_at(text, pos) {
        Some(indent) => format!("{}{indent}", line_break(text)),
        None => String::new(),
    }
}

/// Insert `xml` as a sibling right after the element at `span`.
pub(super) fn insert_after(splicer: &mut Splicer, span: Span, xml: &str) {
    let separator = separator_before(splicer.text(), span.start);
    splicer.insert(span.end, format!("{separator}{xml}"));
}

/// Insert `xml` as a sibling right before the element at `span`.
fn insert_before(splicer: &mut Splicer, span: Span, xml: &str) {
    let separator = separator_before(splicer.text(), span.start);
    splicer.insert(span.start, format!("{xml}{separator}"));
}

/// Insert `xml` as the first child of an element (at `span`) that has no
/// children yet.
pub(super) fn insert_into(splicer: &mut Splicer, span: Span, xml: &str) {
    let text = splicer.text();
    let inserted = match indentation_at(text, span.start) {
        Some(indent) => format!("{}{indent}  {xml}", line_break(text)),
        None => xml.to_string(),
    };
    splicer.insert(span.tag_end, inserted);
}

/// The splice that makes `element`'s text content `value`. A self-closing
/// element (`<dc:title/>`) gets an end tag.
pub(super) fn content_replacement(
    text: &str,
    element: &Element,
    value: &str,
) -> (Range<usize>, String) {
    let span = element.span;
    if span.is_self_closing() {
        let tag = &text[span.start_tag()];
        let slash = tag.trim_end_matches('>').trim_end_matches('/').len();
        (
            span.start + slash..span.tag_end,
            format!(">{}</{}>", escape_text(value), element.qname),
        )
    } else {
        (span.content(), escape_text(value))
    }
}

/// The splice that sets the unprefixed attribute `name` of `element` to
/// `value`: its value is replaced (keeping its quotes), or the attribute is
/// added after the existing ones.
pub(super) fn attribute_replacement(
    text: &str,
    element: &Element,
    name: &str,
    value: &str,
) -> (Range<usize>, String) {
    let start = element.span.start;
    let tag = &text[element.span.start_tag()];
    let (attrs, append_at) = scan_start_tag(tag);
    match attrs.iter().find(|a| a.name == name) {
        Some(attr) => (
            start + attr.value.start..start + attr.value.end,
            escape_attr(value, attr.quote),
        ),
        None => (
            start + append_at..start + append_at,
            format!(" {name}=\"{}\"", escape_attr(value, '"')),
        ),
    }
}

/// The range to delete to remove the attribute `name` from `element` (from
/// the whitespace before it to its closing quote), if it has one.
pub(super) fn attribute_removal(text: &str, element: &Element, name: &str) -> Option<Range<usize>> {
    let start = element.span.start;
    let tag = &text[element.span.start_tag()];
    let (attrs, _) = scan_start_tag(tag);
    let attr = attrs.iter().find(|a| a.name == name)?;
    let from = tag[..attr.name_start].trim_end().len();
    Some(start + from..start + attr.value.end + 1)
}

// ── Verification ──────────────────────────────────────────────────────────────

/// Check an edited OPF before anyone writes it (see the module docs).
fn check_edit(
    old_text: &str,
    old: &Package,
    new_text: &str,
    edit: &MetadataEdit,
) -> Result<(), String> {
    let fail = |what: &str| {
        Err(format!(
            "Internal check failed after editing the OPF ({what}); the file was not changed"
        ))
    };
    let new = match Package::parse(new_text) {
        Ok(pkg) => pkg,
        Err(e) => return fail(&format!("the result doesn't parse: {e}")),
    };
    let (Some(o), Some(n)) = (&old.metadata_section, &new.metadata_section) else {
        return fail("<metadata> is missing");
    };
    if old_text[..o.span.start] != new_text[..n.span.start]
        || old_text[o.span.end..] != new_text[n.span.end..]
    {
        return fail("something outside <metadata> changed");
    }
    if let Some(u) = old.unique_identifier_entry() {
        let same = new.unique_identifier_entry().is_some_and(|v| {
            old_text[old.metadata[u].element.span.range()]
                == new_text[new.metadata[v].element.span.range()]
        });
        if !same {
            return fail("the unique identifier changed");
        }
    }

    let m = metadata::read(&new);
    let mut wrong = Vec::new();
    if let Some(title) = &edit.title {
        if m.title.as_deref() != Some(clean_text(title).as_str()) {
            wrong.push("title");
        }
    }
    if let Some(authors) = &edit.authors {
        let expected: Vec<String> = authors
            .iter()
            .map(|a| clean_text(a))
            .filter(|a| !a.is_empty())
            .collect();
        if !expected.is_empty() && m.authors != expected {
            wrong.push("authors");
        }
    }
    if let Some(language) = &edit.language {
        if !m
            .language
            .as_deref()
            .is_some_and(|l| l.eq_ignore_ascii_case(language.trim()))
        {
            wrong.push("language");
        }
    }
    let description_ok = match &edit.description {
        Change::Keep => true,
        Change::Set(d) => {
            m.description.as_deref().map(clean_text) == Some(clean_text(&description_text(d)))
        }
        Change::Clear => m.description.is_none(),
    };
    let categories_ok = match &edit.categories {
        Change::Keep => true,
        Change::Set(c) => m.categories == *c,
        Change::Clear => m.categories.is_empty(),
    };
    let series_ok = match &edit.series {
        Change::Keep => true,
        Change::Set((name, index)) => {
            m.series.as_deref() == Some(name.as_str()) && same_index(m.series_index, *index)
        }
        Change::Clear => m.series.is_none(),
    };
    let isbn_ok = match &edit.isbn {
        Change::Keep => true,
        Change::Set(isbn) => m.isbn.as_deref() == Some(isbn.as_str()),
        // Only the unique identifier may still carry an ISBN.
        Change::Clear => isbn_entries(&new).iter().all(|e| e.is_unique_identifier),
    };
    let rating_ok = match &edit.rating {
        Change::Keep => true,
        Change::Set(r) => m.rating == Some(*r),
        Change::Clear => m.rating.is_none(),
    };
    for (ok, field) in [
        (description_ok, "description"),
        (categories_ok, "categories"),
        (series_ok, "series"),
        (isbn_ok, "ISBN"),
        (rating_ok, "rating"),
    ] {
        if !ok {
            wrong.push(field);
        }
    }
    if !wrong.is_empty() {
        return fail(&format!("{} not read back as written", wrong.join(", ")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::epub::fixtures::{book_metadata, CALIBRE_OPF, EPUB3_OPF, PREFIXED_ONE_LINE_OPF};

    fn parse(text: &str) -> Package {
        Package::parse(text).unwrap()
    }

    /// Apply `meta` as a full edit; panics if nothing changed.
    fn full(text: &str, meta: &BookMetadata) -> String {
        apply_metadata(text, &parse(text), meta)
            .unwrap()
            .expect("the OPF should change")
    }

    fn read(text: &str) -> metadata::EpubMetadata {
        metadata::read(&parse(text))
    }

    fn unique_identifier(text: &str) -> String {
        let pkg = parse(text);
        let i = pkg.unique_identifier_entry().unwrap();
        text[pkg.metadata[i].element.span.range()].to_string()
    }

    #[test]
    fn reading_then_writing_the_same_metadata_changes_nothing() {
        for opf in [CALIBRE_OPF, EPUB3_OPF, PREFIXED_ONE_LINE_OPF] {
            let meta = book_metadata(&read(opf));
            assert_eq!(
                apply_metadata(opf, &parse(opf), &meta).unwrap(),
                None,
                "{opf}"
            );
        }
    }

    #[test]
    fn edits_are_read_back_and_everything_else_is_kept() {
        let mut meta = book_metadata(&read(CALIBRE_OPF));
        meta.title = "Dune Messiah".into();
        meta.author = "Frank Herbert & Brian Herbert".into();
        meta.language = Some("fr".into());
        meta.description = Some("New text\nwith <angle> & ampersand".into());
        meta.categories = vec!["Classics".into(), "Space opera".into()];
        meta.series_index = Some(2.0);
        meta.isbn = Some("0-441-01359-7".into());
        meta.rating = Some(3);
        let new = full(CALIBRE_OPF, &meta);

        let back = book_metadata(&read(&new));
        assert_eq!(back.title, "Dune Messiah");
        assert_eq!(back.author, "Frank Herbert & Brian Herbert");
        assert_eq!(back.language.as_deref(), Some("fr"));
        assert_eq!(
            back.description.as_deref(),
            Some("New text\nwith <angle> & ampersand")
        );
        assert_eq!(back.categories, vec!["Classics", "Space opera"]);
        assert_eq!(
            (back.series.as_deref(), back.series_index),
            (Some("Dune"), Some(2.0))
        );
        assert_eq!(back.isbn.as_deref(), Some("0441013597"));
        assert_eq!(back.rating, Some(3));
        // Writing it again is a no-op: the round trip is stable.
        assert_eq!(apply_metadata(&new, &parse(&new), &back).unwrap(), None);

        assert_eq!(unique_identifier(&new), unique_identifier(CALIBRE_OPF));
        for kept in [
            r#"<dc:creator opf:file-as="Herbert, Frank" opf:role="aut">Frank Herbert</dc:creator>"#,
            r#"<dc:contributor opf:file-as="calibre" opf:role="bkp">calibre (6.29.0) [https://calibre-ebook.com]</dc:contributor>"#,
            r#"<dc:identifier opf:scheme="calibre">5f2b1c3e-6e0a-4c1b-9a43-3f1f3c0f9b11</dc:identifier>"#,
            "<dc:publisher>Ace</dc:publisher>",
            "<!-- calibre metadata -->",
            r#"<meta name="calibre:timestamp" content="2023-01-01T00:00:00+00:00"/>"#,
            r#"<meta name="cover" content="cover"/>"#,
            "<dc:subject>Classics</dc:subject>",
        ] {
            assert!(new.contains(kept), "{kept} should be kept:\n{new}");
        }
        assert!(
            new.contains("    <dc:creator opf:role=\"aut\">Brian Herbert</dc:creator>\n"),
            "new lines are indented like their neighbours:\n{new}"
        );
        assert!(new.contains(
            "<dc:description>New text\nwith &lt;angle&gt; &amp; ampersand</dc:description>"
        ));
        assert!(
            new.contains(r#"<dc:identifier opf:scheme="ISBN">0441013597</dc:identifier>"#),
            "the existing ISBN element is updated"
        );
        let manifest_onwards = |t: &str| t[t.find("<manifest>").unwrap()..].to_string();
        assert_eq!(manifest_onwards(&new), manifest_onwards(CALIBRE_OPF));
    }

    #[test]
    fn clearing_fields_removes_their_elements_but_keeps_title_author_and_language() {
        let meta = BookMetadata {
            title: "  ".into(),
            author: String::new(),
            ..Default::default()
        };
        let new = full(CALIBRE_OPF, &meta);
        let back = read(&new);
        assert_eq!(back.title.as_deref(), Some("Dune"));
        assert_eq!(back.authors, vec!["Frank Herbert"]);
        assert_eq!(
            back.language.as_deref(),
            Some("en"),
            "dc:language is mandatory"
        );
        assert_eq!(
            (
                back.description,
                back.series,
                back.series_index,
                back.isbn,
                back.rating
            ),
            (None, None, None, None, None)
        );
        assert!(back.categories.is_empty());
        for gone in [
            "dc:description",
            "dc:subject",
            "calibre:series",
            "calibre:rating",
            "opf:scheme=\"ISBN\"",
        ] {
            assert!(!new.contains(gone), "{gone} should be removed:\n{new}");
        }
        assert!(
            !new.contains("\n\n"),
            "removed elements take their line with them"
        );
        assert_eq!(unique_identifier(&new), unique_identifier(CALIBRE_OPF));
        assert!(
            new.contains(r#"opf:scheme="calibre""#),
            "identifiers that aren't ISBNs stay"
        );
    }

    #[test]
    fn placeholders_are_never_written() {
        let meta = BookMetadata {
            title: "Unknown title".into(),
            author: "Unknown author".into(),
            ..book_metadata(&read(EPUB3_OPF))
        };
        assert_eq!(
            apply_metadata(EPUB3_OPF, &parse(EPUB3_OPF), &meta).unwrap(),
            None
        );
    }

    #[test]
    fn epub3_series_collections_are_updated_in_place() {
        let mut meta = book_metadata(&read(EPUB3_OPF));
        meta.series_index = Some(2.5);
        let new = full(EPUB3_OPF, &meta);
        assert!(
            new.contains(r##"<meta refines="#series" property="group-position">2.5</meta>"##),
            "{new}"
        );
        assert!(
            !new.contains("calibre:series"),
            "no second representation added"
        );

        meta.series = Some("Foundation Saga".into());
        meta.series_index = None;
        let new = full(&new, &meta);
        assert!(new.contains(
            r#"<meta property="belongs-to-collection" id="series">Foundation Saga</meta>"#
        ));
        assert!(!new.contains("group-position"));
        assert_eq!(read(&new).series_index, None);

        meta.series = None;
        let new = full(&new, &meta);
        assert!(
            !new.contains("belongs-to-collection") && !new.contains("collection-type"),
            "{new}"
        );
        assert!(new.contains(
            r##"<meta refines="#creator01" property="role" scheme="marc:relators">aut</meta>"##
        ));
    }

    #[test]
    fn a_series_without_any_is_added_the_calibre_way() {
        let opf = EPUB3_OPF.replace(
            r#"<meta property="belongs-to-collection" id="series">Foundation</meta>"#,
            "",
        );
        let mut meta = book_metadata(&read(&opf));
        assert_eq!(meta.series, None);
        meta.series = Some("Foundation".into());
        meta.series_index = Some(1.0);
        let new = full(&opf, &meta);
        assert!(
            new.contains(r#"<meta name="calibre:series" content="Foundation"/>"#),
            "{new}"
        );
        assert!(new.contains(r#"<meta name="calibre:series_index" content="1"/>"#));
        assert_eq!(read(&new).series_index, Some(1.0));
    }

    #[test]
    fn the_isbn_is_added_next_to_a_uuid_unique_identifier() {
        // EPUB 3: urn:isbn: form.
        let mut meta = book_metadata(&read(EPUB3_OPF));
        meta.isbn = Some("978-0-553-29335-7".into());
        let new = full(EPUB3_OPF, &meta);
        assert!(
            new.contains("<dc:identifier>urn:isbn:9780553293357</dc:identifier>"),
            "{new}"
        );
        assert_eq!(unique_identifier(&new), unique_identifier(EPUB3_OPF));
        assert_eq!(read(&new).isbn.as_deref(), Some("9780553293357"));

        // EPUB 2 with the opf prefix: opf:scheme="ISBN".
        let opf = CALIBRE_OPF.replace(
            "    <dc:identifier opf:scheme=\"ISBN\">9780441013593</dc:identifier>\n",
            "",
        );
        let mut meta = book_metadata(&read(&opf));
        meta.isbn = Some("9780441013593".into());
        let new = full(&opf, &meta);
        assert!(
            new.contains("<dc:identifier opf:scheme=\"ISBN\">9780441013593</dc:identifier>"),
            "{new}"
        );
        assert_eq!(unique_identifier(&new), unique_identifier(&opf));
    }

    #[test]
    fn an_isbn_unique_identifier_is_never_modified() {
        let opf = CALIBRE_OPF
            .replace(
                r#"<dc:identifier opf:scheme="ISBN">9780441013593</dc:identifier>"#,
                "",
            )
            .replace(
                r#"opf:scheme="uuid">5f2b1c3e-6e0a-4c1b-9a43-3f1f3c0f9b11<"#,
                r#"opf:scheme="ISBN">9780441013593<"#,
            );
        assert_eq!(read(&opf).isbn.as_deref(), Some("9780441013593"));
        let unique = unique_identifier(&opf);

        let mut meta = book_metadata(&read(&opf));
        meta.isbn = Some("0441013597".into());
        let new = full(&opf, &meta);
        assert_eq!(unique_identifier(&new), unique);
        assert_eq!(
            read(&new).isbn.as_deref(),
            Some("0441013597"),
            "a second identifier carries the new ISBN"
        );

        meta.isbn = None;
        let cleared = full(&new, &meta);
        assert_eq!(unique_identifier(&cleared), unique);
        assert!(!cleared.contains("0441013597"));
        assert_eq!(
            read(&cleared).isbn.as_deref(),
            Some("9780441013593"),
            "only the unique identifier is left"
        );
        assert_eq!(
            apply_metadata(&opf, &parse(&opf), &meta).unwrap(),
            None,
            "nothing to clear"
        );
    }

    #[test]
    fn invalid_isbns_are_not_written() {
        let mut meta = book_metadata(&read(EPUB3_OPF));
        meta.isbn = Some("1234567890".into());
        assert_eq!(
            apply_metadata(EPUB3_OPF, &parse(EPUB3_OPF), &meta).unwrap(),
            None
        );
    }

    #[test]
    fn several_authors_keep_other_creators_and_reuse_matching_elements() {
        let opf = CALIBRE_OPF.replace(
            "    <dc:contributor",
            "    <dc:creator opf:role=\"edt\">An Editor</dc:creator>\n    <dc:contributor",
        );
        let mut meta = book_metadata(&read(&opf));
        meta.author = "Kevin J. Anderson & Frank Herbert".into();
        let new = full(&opf, &meta);
        assert_eq!(
            read(&new).authors,
            vec!["Kevin J. Anderson", "Frank Herbert"]
        );
        assert!(new.contains(
            "    <dc:creator opf:role=\"aut\">Kevin J. Anderson</dc:creator>\n    <dc:creator opf:file-as=\"Herbert, Frank\" opf:role=\"aut\">Frank Herbert</dc:creator>"
        ), "{new}");
        assert!(
            new.contains("<dc:creator opf:role=\"edt\">An Editor</dc:creator>"),
            "the editor stays"
        );

        meta.author = "Brian Herbert".into();
        let new = full(&new, &meta);
        assert_eq!(read(&new).authors, vec!["Brian Herbert"]);
        assert!(!new.contains("Kevin") && !new.contains("Herbert, Frank"));
    }

    #[test]
    fn removed_epub3_creators_take_their_refinements_along() {
        let mut meta = book_metadata(&read(EPUB3_OPF));
        meta.author = "Someone Else".into();
        let new = full(EPUB3_OPF, &meta);
        assert!(
            !new.contains("#creator01"),
            "dangling refinements are removed:\n{new}"
        );
        assert!(new.contains("<dc:creator>Someone Else</dc:creator>"));
        assert_eq!(read(&new).authors, vec!["Someone Else"]);
    }

    #[test]
    fn prefixes_single_lines_entities_and_cdata() {
        let current = read(PREFIXED_ONE_LINE_OPF);
        assert_eq!(
            current.title.as_deref(),
            Some("Tom & Jerry – café <Deluxe>")
        );

        let mut meta = book_metadata(&current);
        meta.categories = vec!["Humour".into()];
        meta.series = Some("Cartoons".into());
        meta.isbn = Some("9780441013593".into());
        let new = full(PREFIXED_ONE_LINE_OPF, &meta);
        assert!(!new.contains('\n'), "a single-line OPF stays on one line");
        assert!(
            new.contains("<purl:subject>Humour</purl:subject>"),
            "Dublin Core keeps the file's prefix: {new}"
        );
        assert!(
            new.contains(r#"<opf:meta name="calibre:series" content="Cartoons"/>"#),
            "{new}"
        );
        assert!(
            new.contains(r#"<purl:identifier opf:scheme="ISBN">9780441013593</purl:identifier>"#),
            "{new}"
        );
        assert!(
            new.contains(
                "<purl:title>Tom &amp; Jerry &#8211; caf&eacute; <![CDATA[<Deluxe>]]></purl:title>"
            ),
            "an unchanged title keeps its entities and CDATA"
        );

        meta.title = "Tom & Jerry <Deluxe>".into();
        let new = full(&new, &meta);
        assert!(new.contains("<purl:title>Tom &amp; Jerry &lt;Deluxe&gt;</purl:title>"));
        assert_eq!(read(&new).title.as_deref(), Some("Tom & Jerry <Deluxe>"));
    }

    #[test]
    fn an_undeclared_dublin_core_namespace_is_declared_on_new_elements() {
        let opf = r#"<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="id">
  <metadata>
    <dc:title>Old</dc:title>
    <dc:identifier id="id">x</dc:identifier>
  </metadata>
  <manifest/>
</package>"#;
        let meta = BookMetadata {
            title: "Old".into(),
            author: "A".into(),
            categories: vec!["B".into()],
            ..Default::default()
        };
        let new = full(opf, &meta);
        assert!(
            new.contains(
                r#"<dc:creator xmlns:dc="http://purl.org/dc/elements/1.1/">A</dc:creator>"#
            ),
            "{new}"
        );
        assert_eq!(read(&new).authors, vec!["A"]);
        assert_eq!(read(&new).categories, vec!["B"]);
    }

    #[test]
    fn crlf_files_get_crlf_lines() {
        let opf = CALIBRE_OPF.replace('\n', "\r\n");
        let mut meta = book_metadata(&read(&opf));
        meta.author = "A & Frank Herbert".into();
        let new = full(&opf, &meta);
        assert!(
            !new.replace("\r\n", "").contains('\n'),
            "no bare line feed added"
        );
    }

    #[test]
    fn changes_only_touch_what_the_user_changed() {
        // An index that knows less than the file: one author (of two), no
        // rating, no EPUB 3 series.
        let opf = EPUB3_OPF.replace(
            "    <dc:language>",
            "    <dc:creator>Second Author</dc:creator>\n    <meta name=\"calibre:rating\" content=\"10\"/>\n    <dc:language>",
        );
        let before = BookMetadata {
            title: "Foundation".into(),
            author: "Isaac Asimov".into(),
            language: Some("en".into()),
            ..Default::default()
        };
        let after = BookMetadata {
            title: "Foundation (1951)".into(),
            ..before.clone()
        };
        let edit = MetadataEdit::changes(&before, &after);
        assert_eq!(
            edit,
            MetadataEdit {
                title: Some("Foundation (1951)".into()),
                ..Default::default()
            }
        );

        let new = apply_edit(&opf, &parse(&opf), &edit).unwrap().unwrap();
        let back = read(&new);
        assert_eq!(back.title.as_deref(), Some("Foundation (1951)"));
        assert_eq!(back.authors, vec!["Isaac Asimov", "Second Author"]);
        assert_eq!(back.rating, Some(5));
        assert_eq!(back.series.as_deref(), Some("Foundation"));

        // Clearing a field the user saw removes it.
        let cleared = BookMetadata {
            series: None,
            ..book_metadata(&read(&opf))
        };
        let edit = MetadataEdit::changes(&book_metadata(&read(&opf)), &cleared);
        assert_eq!(
            edit,
            MetadataEdit {
                series: Change::Clear,
                ..Default::default()
            }
        );
        assert!(MetadataEdit::changes(&before, &before).is_empty());
    }

    #[test]
    fn overlay_writes_known_values_and_removes_nothing() {
        let meta = BookMetadata {
            title: "Dune".into(),
            author: "Frank Herbert".into(),
            series: Some("Dune".into()),
            ..Default::default()
        };
        let edit = MetadataEdit::overlay(&meta);
        assert_eq!(edit.description, Change::Keep);
        assert_eq!(edit.isbn, Change::Keep);
        assert_eq!(edit.series, Change::Set(("Dune".into(), None)));
        let new = apply_edit(CALIBRE_OPF, &parse(CALIBRE_OPF), &edit)
            .unwrap()
            .unwrap();
        let back = read(&new);
        assert!(back.description.is_some() && back.isbn.is_some() && back.rating.is_some());
        assert_eq!(
            back.series_index, None,
            "the series position goes with the series"
        );
    }

    #[test]
    fn a_description_stored_as_html_is_kept_until_changed() {
        let meta = book_metadata(&read(CALIBRE_OPF));
        assert_eq!(
            meta.description.as_deref(),
            Some("Set on the desert planet Arrakis.\n\nA classic.")
        );
        // Same text with different whitespace (as an older index may hold it).
        let older = BookMetadata {
            description: Some("Set on the desert planet Arrakis. A classic.".into()),
            ..meta
        };
        assert_eq!(
            apply_metadata(CALIBRE_OPF, &parse(CALIBRE_OPF), &older).unwrap(),
            None
        );
    }

    #[test]
    fn missing_or_empty_metadata_sections_are_errors() {
        let no_metadata = r#"<package version="2.0"><manifest/></package>"#;
        let meta = BookMetadata {
            title: "x".into(),
            ..Default::default()
        };
        assert!(apply_metadata(no_metadata, &parse(no_metadata), &meta).is_err());
        let empty = r#"<package version="2.0"><metadata/><manifest/></package>"#;
        assert!(apply_metadata(empty, &parse(empty), &meta).is_err());
    }

    /// Modelled on a French publisher's EPUB 3: a translator listed first, the
    /// author without a role, a collection title, an `isbn:` identifier next
    /// to an ISBN unique identifier refined with ONIX code 15, and a plain
    /// description whose second paragraph is indented.
    const PUBLISHER_OPF: &str = r##"<?xml version='1.0' encoding='utf-8'?>
<package xmlns="http://www.idpf.org/2007/opf" unique-identifier="ean" prefix="tdm: http://www.w3.org/ns/tdmrep#" version="3.0" xml:lang="fr">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:opf="http://www.idpf.org/2007/opf">
    <dc:title id="t1">Le Livre, tome 2</dc:title>
    <dc:title id="t2">Le Cycle</dc:title>
    <dc:creator id="creator02">Une Traductrice</dc:creator>
    <dc:creator id="id">Un Auteur</dc:creator>
    <dc:identifier>isbn:9780441013593</dc:identifier>
    <dc:identifier>calibre:11</dc:identifier>
    <dc:identifier id="ean">urn:isbn:9780553293357</dc:identifier>
    <dc:language>fr</dc:language>
    <dc:description>Premier paragraphe.
      Second paragraphe, indenté.</dc:description>
    <meta refines="#creator02" property="role" scheme="marc:relators">trl</meta>
    <meta refines="#t1" property="title-type">main</meta>
    <meta refines="#t2" property="title-type">collection</meta>
    <meta refines="#ean" property="identifier-type" scheme="onix:codelist5">15</meta>
    <meta name="cover" content="cover-image"/>
  </metadata>
  <manifest>
    <item href="cover/cover.png" id="cover-image" properties="cover-image" media-type="image/png"/>
  </manifest>
  <spine/>
</package>"##;

    #[test]
    fn publisher_style_epub3() {
        let current = read(PUBLISHER_OPF);
        assert_eq!(current.title.as_deref(), Some("Le Livre, tome 2"));
        assert_eq!(
            current.authors,
            vec!["Un Auteur"],
            "the translator is not an author"
        );
        assert_eq!(current.isbn.as_deref(), Some("9780441013593"));
        assert_eq!(
            current.description.as_deref(),
            Some("Premier paragraphe.\nSecond paragraphe, indenté.")
        );

        let mut meta = book_metadata(&current);
        meta.isbn = Some("0441013597".into());
        meta.author = "Un Auteur & Une Autrice".into();
        let new = full(PUBLISHER_OPF, &meta);
        assert!(
            new.contains("<dc:identifier>isbn:0441013597</dc:identifier>"),
            "the identifier keeps its prefix:\n{new}"
        );
        assert!(new.contains(r#"<dc:identifier id="ean">urn:isbn:9780553293357</dc:identifier>"#));
        assert!(new.contains("<dc:creator id=\"creator02\">Une Traductrice</dc:creator>"));
        assert!(new.contains("<dc:creator id=\"id\">Un Auteur</dc:creator>\n    <dc:creator>Une Autrice</dc:creator>"), "{new}");
        assert!(new.contains("<dc:title id=\"t2\">Le Cycle</dc:title>"));
        assert_eq!(read(&new).authors, vec!["Un Auteur", "Une Autrice"]);
    }

    #[test]
    fn splitting_authors() {
        assert_eq!(split_authors(" A  &  B "), vec!["A", "B"]);
        assert_eq!(split_authors("Simon&Schuster"), vec!["Simon&Schuster"]);
        assert!(split_authors("  ").is_empty());
    }
}
