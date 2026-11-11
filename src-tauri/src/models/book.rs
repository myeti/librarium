use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum BookFormat {
    Epub,
    Pdf,
    Mobi,
    Azw3,
    Cbz,
    Unknown,
}

impl BookFormat {
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            "epub" => Self::Epub,
            "pdf" => Self::Pdf,
            "mobi" | "azw" => Self::Mobi,
            "azw3" => Self::Azw3,
            "cbz" => Self::Cbz,
            _ => Self::Unknown,
        }
    }

    #[allow(dead_code)]
    pub fn extension(&self) -> &str {
        match self {
            Self::Epub => "epub",
            Self::Pdf => "pdf",
            Self::Mobi => "mobi",
            Self::Azw3 => "azw3",
            Self::Cbz => "cbz",
            Self::Unknown => "bin",
        }
    }

    #[allow(dead_code)]
    pub fn is_kindle_native(&self) -> bool {
        matches!(self, Self::Mobi | Self::Azw3 | Self::Pdf)
    }
}

/// Descriptive metadata of a book, as shown and edited in the UI.
///
/// An unknown title or author is an **empty string**: the UI shows a
/// localised placeholder ("Unknown title", "Titre inconnu"…). Placeholders
/// are never stored as data, nor written into files. (An older index may
/// hold the English literals; the index clears them on load, see
/// [`BookMetadata::clear_placeholders`].)
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BookMetadata {
    /// Empty when unknown.
    pub title: String,
    /// Empty when unknown.
    pub author: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series_index: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub isbn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rating: Option<u8>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goodreads_url: Option<String>,
}

/// The placeholders an older index may hold as if they were data.
const LEGACY_TITLE_PLACEHOLDER: &str = "Unknown title";
const LEGACY_AUTHOR_PLACEHOLDER: &str = "Unknown author";

impl BookMetadata {
    /// Replace the placeholders an older index may hold ("Unknown title",
    /// "Unknown author") with empty strings. Returns whether anything changed.
    pub fn clear_placeholders(&mut self) -> bool {
        let mut changed = false;
        if self.title.trim() == LEGACY_TITLE_PLACEHOLDER {
            self.title.clear();
            changed = true;
        }
        if self.author.trim() == LEGACY_AUTHOR_PLACEHOLDER {
            self.author.clear();
            changed = true;
        }
        changed
    }
}

/// One book of the library, as stored in the index (`library.json`) and sent
/// to the frontend.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Book {
    pub id: String,
    /// The book's file. Absolute in memory and over IPC; relative to the
    /// library folder inside `library.json` (format v2).
    pub path: String,
    pub filename: String,
    pub format: BookFormat,
    pub size_bytes: u64,
    pub date_added: String,
    /// The cover to show (thumbnail, or full cover if no thumbnail could be
    /// made), as an absolute path for `convertFileSrc`. Output only: filled
    /// by `scan_library` from the cover cache (`commands::library::covers`),
    /// never stored in the index and never read back from JSON.
    #[serde(default, skip_deserializing, skip_serializing_if = "Option::is_none")]
    pub cover_path: Option<String>,
    /// Modification time (ms) of the `cover_path` file: the frontend appends
    /// it to the URL so a replaced cover isn't served from the webview cache.
    /// Output only, like `cover_path`.
    #[serde(default, skip_deserializing, skip_serializing_if = "Option::is_none")]
    pub cover_version: Option<u64>,
    /// Starred by the user (`set_favorite`): listed first. Kept in the index
    /// only, never written into the book's file; absent when false.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub favorite: bool,
    pub metadata: BookMetadata,
}

/// A partial metadata update for batch edits (`batch_update_metadata`).
///
/// Every field is optional: an absent (or `null`) field leaves the book's
/// value unchanged, so one patch can be merged into many books that each keep
/// their own title, author, ISBN… This is the fix for 0.1, where the whole
/// metadata of the first selected book was copied onto every other one.
///
/// To clear an optional text field (series, subtitle…) send an empty string.
/// `title` and `author` can't be cleared: an empty value is ignored.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BookMetadataPatch {
    pub title: Option<String>,
    pub author: Option<String>,
    pub subtitle: Option<String>,
    pub series: Option<String>,
    pub series_index: Option<f32>,
    pub language: Option<String>,
    pub description: Option<String>,
    pub isbn: Option<String>,
    pub rating: Option<u8>,
    /// Replaces the whole list (an empty list clears it).
    pub categories: Option<Vec<String>>,
    pub goodreads_url: Option<String>,
}

impl BookMetadataPatch {
    /// Merge this patch into `meta`, leaving absent fields untouched.
    /// Text values are trimmed. Language codes are *not* normalised here
    /// (that lives with the library code); callers do it after merging.
    pub fn apply_to(&self, meta: &mut BookMetadata) {
        if let Some(title) = non_empty(&self.title) {
            meta.title = title;
        }
        if let Some(author) = non_empty(&self.author) {
            meta.author = author;
        }
        set_optional_text(&mut meta.subtitle, &self.subtitle);
        set_optional_text(&mut meta.series, &self.series);
        set_optional_text(&mut meta.language, &self.language);
        set_optional_text(&mut meta.description, &self.description);
        set_optional_text(&mut meta.isbn, &self.isbn);
        set_optional_text(&mut meta.goodreads_url, &self.goodreads_url);
        if let Some(index) = self.series_index {
            meta.series_index = Some(index);
        }
        if let Some(rating) = self.rating {
            meta.rating = Some(rating);
        }
        if let Some(categories) = &self.categories {
            meta.categories = categories
                .iter()
                .map(|c| c.trim().to_string())
                .filter(|c| !c.is_empty())
                .collect();
        }
    }
}

/// The trimmed value of `value`, unless it is absent or blank.
fn non_empty(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

/// Patch semantics for an optional text field: `None` keeps the current
/// value, a blank string clears it, anything else replaces it.
fn set_optional_text(target: &mut Option<String>, value: &Option<String>) {
    if let Some(v) = value {
        let v = v.trim();
        *target = if v.is_empty() {
            None
        } else {
            Some(v.to_string())
        };
    }
}

/// A per-book error inside a batch command: the batch carries on with the
/// other books and reports this one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Failure {
    pub id: String,
    pub error: String,
}

/// Result of `batch_update_metadata`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchUpdateResult {
    /// The books that were updated, with their new metadata.
    pub books: Vec<Book>,
    pub failures: Vec<Failure>,
}

/// Result of `forget_books` ("Remove from library": the files are kept).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForgetResult {
    /// Ids of the books removed from the library.
    pub forgotten: Vec<String>,
    pub failures: Vec<Failure>,
}

/// Result of `delete_books`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteResult {
    /// Ids of the books removed from the library (their files are in the trash).
    pub deleted: Vec<String>,
    pub failures: Vec<Failure>,
    /// Set when at least one file went to the fallback trash folder
    /// (`<library>/.librarium/trash/<timestamp>/`) because the volume has no
    /// OS trash — typical of NAS shares. The UI should tell the user where.
    pub library_trash: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dune() -> BookMetadata {
        BookMetadata {
            title: "Dune".into(),
            author: "Frank Herbert".into(),
            series: Some("Dune".into()),
            series_index: Some(1.0),
            isbn: Some("9780441013593".into()),
            categories: vec!["SF".into()],
            ..Default::default()
        }
    }

    #[test]
    fn empty_patch_changes_nothing() {
        let mut meta = dune();
        BookMetadataPatch::default().apply_to(&mut meta);
        assert_eq!(meta, dune());
    }

    #[test]
    fn patch_only_touches_present_fields() {
        let mut meta = dune();
        let patch = BookMetadataPatch {
            language: Some(" fr ".into()),
            rating: Some(4),
            ..Default::default()
        };
        patch.apply_to(&mut meta);
        assert_eq!(meta.title, "Dune");
        assert_eq!(meta.isbn.as_deref(), Some("9780441013593"));
        assert_eq!(meta.language.as_deref(), Some("fr"));
        assert_eq!(meta.rating, Some(4));
    }

    #[test]
    fn blank_values_clear_optional_fields_but_not_title() {
        let mut meta = dune();
        let patch = BookMetadataPatch {
            title: Some("  ".into()),
            series: Some(String::new()),
            categories: Some(vec![]),
            ..Default::default()
        };
        patch.apply_to(&mut meta);
        assert_eq!(meta.title, "Dune");
        assert_eq!(meta.series, None);
        assert!(meta.categories.is_empty());
    }

    #[test]
    fn patch_deserialises_from_partial_camel_case_json() {
        let patch: BookMetadataPatch =
            serde_json::from_str(r#"{"seriesIndex": 2, "goodreadsUrl": null}"#).unwrap();
        assert_eq!(patch.series_index, Some(2.0));
        assert_eq!(patch.goodreads_url, None);
        assert_eq!(patch.title, None);
    }

    #[test]
    fn unknown_title_and_author_are_empty() {
        let meta = BookMetadata::default();
        assert_eq!((meta.title.as_str(), meta.author.as_str()), ("", ""));
    }

    #[test]
    fn legacy_placeholders_are_cleared() {
        let mut meta = BookMetadata {
            title: "Unknown title".into(),
            author: " Unknown author ".into(),
            ..Default::default()
        };
        assert!(meta.clear_placeholders());
        assert_eq!((meta.title.as_str(), meta.author.as_str()), ("", ""));
        assert!(!meta.clear_placeholders(), "nothing left to clear");

        // Real titles that merely contain the words are kept.
        let mut real = BookMetadata {
            title: "Unknown title: a memoir".into(),
            author: "Unknown".into(),
            ..Default::default()
        };
        assert!(!real.clear_placeholders());
        assert_eq!(real.title, "Unknown title: a memoir");
    }

    #[test]
    fn delete_result_serialises_library_trash_as_null() {
        let result = DeleteResult {
            deleted: vec![],
            failures: vec![],
            library_trash: None,
        };
        let json = serde_json::to_value(&result).unwrap();
        assert!(json.get("libraryTrash").unwrap().is_null());
    }
}
