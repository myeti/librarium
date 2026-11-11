//! User folders. Each user is a subdirectory of the library root (e.g.
//! `~/Documents/Librarium/Alice`) holding that user's books; the frontend
//! opens `<root>/<user>` as the library.
//!
//! Listing never creates anything: a root that isn't there is reported as
//! `LIBRARY_UNAVAILABLE` (an unmounted NAS share must not become an empty
//! local folder), one that can't be read as `PERMISSION_DENIED` (on macOS,
//! Librarium not yet allowed to read network volumes). Creating a user
//! creates its folder — and the root only when the caller explicitly asks
//! (the first run, with the default root).

use std::fs;
use std::path::Path;

use super::files::{ensure_library_available, is_system_folder_name, strip_hidden_prefix};
use crate::error::{display_name, AppError, AppResult};
use crate::fsutil;

/// Longest user name (bytes), well within every filesystem's limit.
const MAX_USER_NAME_BYTES: usize = 120;

/// [`list_users`](super::list_users): the names of the user folders of
/// `root` ([`is_user_folder_name`]), sorted case-insensitively.
///
/// Fails rather than return a partial list — the frontend forgets a
/// remembered user that isn't listed — with `LIBRARY_UNAVAILABLE` when
/// `root` doesn't exist (or the share stopped answering), `PERMISSION_DENIED`
/// when it can't be read, `IO` for another read error ([`AppError::folder`]).
pub(crate) fn list_user_dirs(root: &Path) -> AppResult<Vec<String>> {
    ensure_library_available(root)?;
    let entries = fs::read_dir(root).map_err(|e| AppError::folder(root, &e))?;
    let mut users = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| AppError::folder(root, &e))?;
        // An entry whose type can't be read is not a usable user folder.
        if !entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if is_user_folder_name(&name) {
            users.push(name);
        }
    }
    users.sort_by_key(|name| name.to_lowercase());
    Ok(users)
}

/// Whether a folder of the library root called `name` is a user. Not:
/// - hidden folders (`.librarium`, `.covers`, `.DS_Store`…);
/// - folders a NAS creates in its shares: every name starting with `#` or
///   `@` (Synology's `#recycle`, `#snapshot`, `@eaDir`; QNAP's `@Recycle`,
///   `@Recently-Snapshot`…). [`user_folder_name`] never creates such a name;
/// - the system folders of [`is_system_folder_name`] (`$RECYCLE.BIN`,
///   `System Volume Information`, `lost+found`…).
pub(crate) fn is_user_folder_name(name: &str) -> bool {
    !(name.starts_with(['.', '#', '@']) || is_system_folder_name(name))
}

/// The folder name for a user called `name`: trimmed, without what would
/// hide it from [`list_user_dirs`] (a leading `.`, `#` or `@`), and safe on
/// every filesystem ([`fsutil::sanitize_stem`], the same rules as book file
/// names). `None` when nothing usable is left.
fn user_folder_name(name: &str) -> Option<String> {
    let name =
        name.trim_start_matches(|c: char| c == '#' || c == '@' || c == '.' || c.is_whitespace());
    let name = strip_hidden_prefix(name).trim();
    if name.is_empty() {
        return None;
    }
    Some(fsutil::sanitize_stem(name, MAX_USER_NAME_BYTES))
}

/// [`create_user`](super::create_user): create the folder of a new user
/// under `root` and return its (sanitised) name. Fails when the name is
/// empty or the folder already exists.
///
/// `root` itself is only created when `create_root` is set — the first run,
/// when the default library folder doesn't exist yet. Otherwise a missing
/// root is `LIBRARY_UNAVAILABLE`, like everywhere else.
pub(crate) fn create_user_dir(root: &Path, name: &str, create_root: bool) -> AppResult<String> {
    let safe = user_folder_name(name)
        .ok_or_else(|| AppError::InvalidInput("Name cannot be empty".to_string()))?;
    if !is_user_folder_name(&safe) {
        // `lost+found`, `$RECYCLE.BIN`…: the folder would never be listed.
        return Err(AppError::InvalidInput(format!(
            "\"{safe}\" is a name the system uses for its own folders; choose another one"
        )));
    }
    if create_root {
        fs::create_dir_all(root).map_err(|e| {
            AppError::io(
                format!("Could not create the library folder {}", display_name(root)),
                &e,
            )
        })?;
    } else {
        ensure_library_available(root)?;
    }

    let user_dir = root.join(&safe);
    if user_dir.exists() {
        return Err(AppError::Conflict(format!(
            "User '{}' already exists",
            safe
        )));
    }
    fs::create_dir(&user_dir)
        .map_err(|e| AppError::io(format!("Could not create the folder for {safe}"), &e))?;
    Ok(safe)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listing_a_missing_root_creates_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Volumes").join("books");
        let err = list_user_dirs(&root).unwrap_err();
        assert_eq!(err.code(), "LIBRARY_UNAVAILABLE");
        assert!(!dir.path().join("Volumes").exists());
    }

    #[test]
    fn users_are_the_visible_folders_sorted() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["bob", "Alice", ".librarium", "Zoé"] {
            fs::create_dir(dir.path().join(name)).unwrap();
        }
        fs::write(dir.path().join("notes.txt"), b"").unwrap();
        assert_eq!(list_user_dirs(dir.path()).unwrap(), ["Alice", "bob", "Zoé"]);
    }

    #[test]
    fn nas_and_system_folders_are_not_users() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "Alice",
            "#recycle",
            "#snapshot",
            "@eaDir",
            "@Recycle",
            "$RECYCLE.BIN",
            "System Volume Information",
            "lost+found",
        ] {
            fs::create_dir(dir.path().join(name)).unwrap();
        }
        assert_eq!(list_user_dirs(dir.path()).unwrap(), ["Alice"]);
        // A typed name never becomes one of them.
        assert_eq!(user_folder_name("#Bob").as_deref(), Some("Bob"));
        assert_eq!(user_folder_name("@ Ann").as_deref(), Some("Ann"));
        let err = create_user_dir(dir.path(), "lost+found", false).unwrap_err();
        assert_eq!(err.code(), "INVALID_INPUT");
    }

    /// A root that is there but can't be read (on macOS: Librarium not yet
    /// allowed to read network volumes) is `PERMISSION_DENIED`, so the user
    /// picker can say what to do — not "no users", not "NAS not connected".
    #[cfg(unix)]
    #[test]
    fn an_unreadable_root_is_a_permission_problem() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("books");
        fs::create_dir_all(root.join("Alice")).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o000)).unwrap();
        let result = list_user_dirs(&root);
        fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(result.unwrap_err().code(), "PERMISSION_DENIED");
    }

    #[test]
    fn user_names_use_the_shared_sanitiser() {
        assert_eq!(user_folder_name("  Alice  ").as_deref(), Some("Alice"));
        assert_eq!(
            user_folder_name("Zoé O'Brien").as_deref(),
            Some("Zoé O'Brien")
        );
        assert_eq!(user_folder_name("a/b:c").as_deref(), Some("a_b_c"));
        assert_eq!(
            user_folder_name("../../etc").as_deref(),
            Some("_.._etc"),
            "no path, no hidden folder"
        );
        assert_eq!(user_folder_name("CON").as_deref(), Some("_CON"));
        assert_eq!(user_folder_name("   "), None);
        assert_eq!(user_folder_name("..."), None);
    }

    #[test]
    fn creating_a_user() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert_eq!(create_user_dir(root, "Ann/Bob", false).unwrap(), "Ann_Bob");
        assert!(root.join("Ann_Bob").is_dir());
        assert_eq!(
            create_user_dir(root, "Ann/Bob", false).unwrap_err().code(),
            "CONFLICT"
        );
        assert!(create_user_dir(root, " ", false).is_err());
    }

    #[test]
    fn the_root_is_only_created_on_request() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Librarium");
        let err = create_user_dir(&root, "Ann", false).unwrap_err();
        assert_eq!(err.code(), "LIBRARY_UNAVAILABLE");
        assert!(!root.exists());
        // First run, default library folder.
        assert_eq!(create_user_dir(&root, "Ann", true).unwrap(), "Ann");
        assert!(root.join("Ann").is_dir());
    }
}
