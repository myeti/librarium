//! Rules for the book files themselves, shared by every operation that
//! writes into the library folder: whether the library is reachable at all
//! ([`ensure_library_available`]), where a write may happen
//! ([`ensure_in_library`]), what a book file may be called
//! ([`library_file_name`]), the permissions of files Librarium creates
//! ([`make_readable`]), how files are deleted ([`Trash`]), and which folders
//! belong to the NAS or the OS rather than to the user
//! ([`is_system_folder_name`]).

use std::fs;
use std::path::Path;

use crate::error::{display_name, AppError, AppResult};
use crate::fsutil::{self, TrashOutcome};

/// How a helper moves a file to the trash: [`fsutil::move_to_trash`] in the
/// app, a fake in tests (so tests never touch the real OS trash).
pub(crate) type Trash<'a> = &'a dyn Fn(&Path) -> std::io::Result<TrashOutcome>;

/// Fail unless `library` is an existing folder that can be looked at:
/// - not there (or not a folder, or the share under it is gone) →
///   [`AppError::LibraryUnavailable`] (code `LIBRARY_UNAVAILABLE`, which the
///   frontend answers with "library not reachable" and a Retry button);
/// - there, but access refused → [`AppError::PermissionDenied`] (code
///   `PERMISSION_DENIED`: on macOS, Librarium hasn't been allowed to read
///   network or removable volumes yet).
///
/// `fs::metadata` rather than `Path::is_dir`, which answers `false` for any
/// error and so turned a permission problem into "is the NAS connected?".
/// (macOS privacy protection usually lets `stat` through and refuses the
/// reads that follow: those are classified where they happen, with
/// [`AppError::folder`] — listing the users, reading the index, walking the
/// library.)
///
/// A library that isn't there is almost always a NAS share that isn't
/// mounted (or an unplugged drive). It must never be created on the spot:
/// the books would then go to an empty local folder under `/Volumes`, and
/// the next scan would see an empty library. Only the explicit first-run and
/// create-user flows create folders.
pub(crate) fn ensure_library_available(library: &Path) -> AppResult<()> {
    match fs::metadata(library) {
        Ok(meta) if meta.is_dir() => Ok(()),
        Ok(_) => Err(AppError::library_unavailable(library)),
        Err(e) => Err(match AppError::folder(library, &e) {
            // Neither refused nor unreachable (an odd I/O error): still
            // "can't be reached", the answer every caller expects here.
            AppError::Io(_) => AppError::library_unavailable(library),
            other => other,
        }),
    }
}

/// Folders that a NAS or an operating system puts in shared folders, which
/// are never users nor book folders: Synology's `#recycle`, `#snapshot` and
/// `@eaDir` (thumbnails, in every folder), QNAP's `@Recycle` and
/// `@Recently-Snapshot`, Windows' `$RECYCLE.BIN` and `System Volume
/// Information`, ext4's `lost+found`, AFP's `Network Trash Folder` and
/// `Temporary Items`. Compared without regard to case (SMB shares are
/// case-insensitive). Names starting with a dot are hidden anyway.
const SYSTEM_FOLDERS: &[&str] = &[
    "#recycle",
    "#snapshot",
    "@eaDir",
    "@Recycle",
    "@Recently-Snapshot",
    "$RECYCLE.BIN",
    "System Volume Information",
    "lost+found",
    "Network Trash Folder",
    "Temporary Items",
];

/// Whether a folder called `name` belongs to the NAS or the OS
/// ([`SYSTEM_FOLDERS`]): scans don't walk into it.
pub(crate) fn is_system_folder_name(name: &str) -> bool {
    SYSTEM_FOLDERS.iter().any(|s| s.eq_ignore_ascii_case(name))
}

/// Refuse to touch a file outside the library folder. Index paths are
/// written by the backend, but they are absolute and can predate a move of
/// the library (or a bug); writes and deletes must stay inside the root.
pub(crate) fn ensure_in_library(library: &Path, path: &Path) -> AppResult<()> {
    if fsutil::is_within(library, path) {
        Ok(())
    } else {
        Err(AppError::InvalidInput(format!(
            "{} is outside the library folder; rescan the library",
            display_name(path)
        )))
    }
}

/// A file name for a book file inside the library: safe on every filesystem
/// ([`fsutil::sanitize_filename`]) and not hidden — scans skip names that
/// start with a dot, so a `.Dune.epub` would silently drop out of the library.
pub(crate) fn library_file_name(name: &str) -> String {
    fsutil::sanitize_filename(strip_hidden_prefix(name))
}

/// `name` without leading dots and whitespace.
pub(crate) fn strip_hidden_prefix(name: &str) -> &str {
    name.trim_start_matches(|c: char| c == '.' || c.is_whitespace())
}

/// Give a file created through `tempfile` (owner-only permissions) the usual
/// `rw-r--r--`, like files copied into the library, so other accounts sharing
/// the library can still read it.
pub(crate) fn make_readable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644));
    }
    #[cfg(not(unix))]
    let _ = path;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_library_is_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        assert!(ensure_library_available(dir.path()).is_ok());
        let err = ensure_library_available(&dir.path().join("unmounted")).unwrap_err();
        assert_eq!(err.code(), "LIBRARY_UNAVAILABLE");
        assert!(err.to_string().contains("\"unmounted\""), "{err}");
        let file = dir.path().join("file");
        std::fs::write(&file, b"").unwrap();
        assert!(
            ensure_library_available(&file).is_err(),
            "a file is not a library"
        );
    }

    /// A folder that is there but can't be looked into is a permission
    /// problem, not a missing NAS (never "is the drive or NAS connected?").
    #[cfg(unix)]
    #[test]
    fn a_refused_library_is_a_permission_problem() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let share = dir.path().join("share");
        let lib = share.join("Alice");
        fs::create_dir_all(&lib).unwrap();
        fs::set_permissions(&share, fs::Permissions::from_mode(0o000)).unwrap();
        let result = ensure_library_available(&lib);
        fs::set_permissions(&share, fs::Permissions::from_mode(0o755)).unwrap();
        let err = result.unwrap_err();
        assert_eq!(err.code(), "PERMISSION_DENIED", "{err}");
        assert!(err.to_string().contains("\"Alice\""), "{err}");
    }

    #[test]
    fn nas_and_os_folders_are_recognised() {
        for name in [
            "#recycle",
            "#Recycle",
            "@eaDir",
            "$RECYCLE.BIN",
            "System Volume Information",
            "lost+found",
        ] {
            assert!(is_system_folder_name(name), "{name}");
        }
        for name in ["Alice", "#1 Bestsellers", "@home", "Recycle"] {
            assert!(!is_system_folder_name(name), "{name}");
        }
    }
}
