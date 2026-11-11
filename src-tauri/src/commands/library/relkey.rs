//! Index keys ("relKeys"): where a book lives, relative to the library root.
//!
//! The index (`.librarium/library.json`, format v2) identifies a book's file by
//! its path *relative to the library folder*, so a library that is moved, or
//! mounted under another name (`/Volumes/books` today, `/Volumes/books-1`
//! tomorrow, `\\nas\books` on the Windows PC), keeps working.
//!
//! A key is built from the path's components joined with `/`, whatever the
//! platform, and normalised to Unicode NFC: macOS and SMB shares can report
//! the same name decomposed (NFD, `e` + U+0301) or precomposed (NFC, `é`),
//! and both spellings must find the same book.
//!
//! Keys are identities, not spellings. The path used to open the file is the
//! one the filesystem reported (kept in `Book.path`), because a
//! normalisation-sensitive filesystem (ext4 behind a NAS) only opens the
//! exact bytes it stores.

use std::path::{Component, Path, PathBuf};

use unicode_normalization::UnicodeNormalization;

/// The key for a relative path made of `/`-separated components: its NFC
/// normalisation. (Backslashes are *not* separators here: on macOS and Linux
/// `\` is a valid character in a file name. Old Windows paths are split by
/// the v1 migration, which knows they are paths from another system.)
pub(crate) fn normalize_key(rel: &str) -> String {
    rel.nfc().collect()
}

/// NFC form of any string (file names, path components).
pub(crate) fn nfc(s: &str) -> String {
    s.nfc().collect()
}

/// `path` relative to `root`, as `/`-separated components in the spelling
/// the path uses (not normalised: see [`normalize_key`]). `None` when `path`
/// is `root` itself, isn't under it, or climbs out of it with `..`.
///
/// The comparison is lexical (no filesystem access): paths built by joining
/// onto `root` always match. A root spelled in another Unicode normalisation
/// than the path still matches.
pub(crate) fn relative_to(root: &Path, path: &Path) -> Option<String> {
    if let Ok(rest) = path.strip_prefix(root) {
        return join_normal_components(rest);
    }
    // Same folder, other normalisation (an NFD path from the filesystem
    // under an NFC root typed or stored elsewhere).
    if root.has_root() != path.has_root() {
        return None;
    }
    let root_parts = normal_components(root)?;
    let path_parts = normal_components(path)?;
    if path_parts.len() <= root_parts.len() {
        return None;
    }
    let same_prefix = root_parts
        .iter()
        .zip(&path_parts)
        .all(|(r, p)| nfc(r) == nfc(p));
    same_prefix.then(|| path_parts[root_parts.len()..].join("/"))
}

/// The plain components of `path` (no root, prefix, `.` or `..`); `None` if
/// it contains `..`.
fn normal_components(path: &Path) -> Option<Vec<String>> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(name) => parts.push(name.to_string_lossy().into_owned()),
            Component::CurDir | Component::RootDir | Component::Prefix(_) => {}
            Component::ParentDir => return None,
        }
    }
    Some(parts)
}

/// `rest`'s components joined with `/`; `None` if empty or if it contains
/// anything but plain names.
fn join_normal_components(rest: &Path) -> Option<String> {
    let mut parts = Vec::new();
    for component in rest.components() {
        match component {
            Component::Normal(name) => parts.push(name.to_string_lossy().into_owned()),
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// The absolute path of `rel` (a key, or a relative path in the same
/// `/`-separated form) under `root`, or `None` when `rel` could point outside
/// the root: empty, absolute, with an empty, `.` or `..` component, or with a
/// component the platform reads as more than one plain name (a Windows drive
/// such as `C:x`, a `\` separator on Windows).
///
/// This is the confinement rule for everything read from the index file: a
/// hand-edited or corrupted index file can't make Librarium write outside
/// the library.
pub(crate) fn resolve(root: &Path, rel: &str) -> Option<PathBuf> {
    if rel.is_empty() || rel.starts_with('/') {
        return None;
    }
    let mut path = root.to_path_buf();
    for part in rel.split('/') {
        if !is_plain_name(part) {
            return None;
        }
        path.push(part);
    }
    Some(path)
}

/// Whether `rel` is a usable key (see [`resolve`]).
pub(crate) fn is_valid_key(rel: &str) -> bool {
    resolve(Path::new("root"), rel).is_some()
}

/// Whether `part` is exactly one ordinary path component on this platform.
fn is_plain_name(part: &str) -> bool {
    if part.is_empty() || part == "." || part == ".." {
        return false;
    }
    let mut components = Path::new(part).components();
    matches!(
        (components.next(), components.next()),
        (Some(Component::Normal(name)), None) if name == std::ffi::OsStr::new(part)
    )
}

/// The components of a path written on *any* system (`/` or `\`
/// separators, a Windows drive or UNC prefix), NFC-normalised, without
/// empty and `.` components. Used to map old absolute paths, which may come
/// from another computer, onto the current library.
pub(crate) fn foreign_components(path: &str) -> Vec<String> {
    path.split(['/', '\\'])
        .filter(|part| !part.is_empty() && *part != ".")
        .map(nfc)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NFD_E: &str = "e\u{301}"; // "é" decomposed
    const NFC_E: &str = "\u{e9}"; // "é" precomposed

    #[test]
    fn keys_are_nfc() {
        let nfd = format!("Caf{NFD_E}/{NFD_E}t{NFD_E}.epub");
        assert_eq!(
            normalize_key(&nfd),
            format!("Caf{NFC_E}/{NFC_E}t{NFC_E}.epub")
        );
        assert_eq!(normalize_key("plain/ascii.epub"), "plain/ascii.epub");
    }

    #[test]
    fn relative_to_keeps_the_paths_spelling() {
        let root = Path::new("/Volumes/books/Alice");
        assert_eq!(
            relative_to(root, &root.join("Dune.epub")).as_deref(),
            Some("Dune.epub")
        );
        assert_eq!(
            relative_to(root, &root.join("Herbert").join("Dune.epub")).as_deref(),
            Some("Herbert/Dune.epub")
        );
        let nfd_file = root.join(format!("Caf{NFD_E}.epub"));
        assert_eq!(
            relative_to(root, &nfd_file),
            Some(format!("Caf{NFD_E}.epub")),
            "spelling kept"
        );
        // A trailing slash on the root changes nothing.
        assert_eq!(
            relative_to(Path::new("/Volumes/books/Alice/"), &root.join("a.epub")).as_deref(),
            Some("a.epub")
        );
    }

    #[test]
    fn relative_to_matches_a_root_in_another_normalisation() {
        let nfc_root = PathBuf::from(format!("/lib/Ren{NFC_E}e"));
        let nfd_path = PathBuf::from(format!("/lib/Ren{NFD_E}e/Dune.epub"));
        assert_eq!(
            relative_to(&nfc_root, &nfd_path).as_deref(),
            Some("Dune.epub")
        );
    }

    #[test]
    fn relative_to_rejects_paths_outside_the_root() {
        let root = Path::new("/lib/Alice");
        assert_eq!(
            relative_to(root, root),
            None,
            "the root itself is not a book"
        );
        assert_eq!(relative_to(root, Path::new("/lib/Other/a.epub")), None);
        assert_eq!(
            relative_to(root, Path::new("/lib/Alice/../Other/a.epub")),
            None
        );
        assert_eq!(
            relative_to(root, Path::new("/lib/AliceX/a.epub")),
            None,
            "component-wise, not a string prefix"
        );
        assert_eq!(
            relative_to(root, Path::new("lib/Alice/a.epub")),
            None,
            "relative vs absolute"
        );
    }

    #[test]
    fn resolve_confines_keys_to_the_root() {
        let root = Path::new("/lib");
        assert_eq!(
            resolve(root, "a/b.epub"),
            Some(root.join("a").join("b.epub"))
        );
        assert_eq!(
            resolve(root, "Dune: Messiah.epub"),
            Some(root.join("Dune: Messiah.epub"))
        );
        for bad in [
            "",
            "/etc/passwd",
            "../outside.epub",
            "a/../../x",
            "a//b",
            "./a",
            "a/./b",
            "a/",
        ] {
            assert_eq!(resolve(root, bad), None, "{bad:?} must be rejected");
            assert!(!is_valid_key(bad));
        }
        assert!(is_valid_key("Série/Tome 1.epub"));
    }

    #[cfg(windows)]
    #[test]
    fn resolve_rejects_windows_prefixes_and_backslashes() {
        let root = Path::new(r"C:\lib");
        assert_eq!(resolve(root, "C:x.epub"), None);
        assert_eq!(resolve(root, r"a\..\..\x"), None);
    }

    #[test]
    fn foreign_components_split_any_separator() {
        assert_eq!(
            foreign_components(r"C:\Users\me\Librarium\Alice\a.epub"),
            ["C:", "Users", "me", "Librarium", "Alice", "a.epub"]
        );
        assert_eq!(
            foreign_components("/Volumes/books//Alice/./a.epub"),
            ["Volumes", "books", "Alice", "a.epub"]
        );
        assert_eq!(
            foreign_components(r"\\nas\books/Alice\sub/a.epub"),
            ["nas", "books", "Alice", "sub", "a.epub"]
        );
        assert_eq!(
            foreign_components(&format!("/x/Caf{NFD_E}")),
            ["x", &format!("Caf{NFC_E}")]
        );
    }
}
