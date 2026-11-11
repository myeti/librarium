//! Talking to the operating system outside the library: showing a file in
//! the platform's file manager, and probing whether a folder (possibly on a
//! NAS) is reachable.

use std::path::Path;

use crate::error::{display_name, AppError, AppResult};
use crate::fsutil;

/// [`reveal_in_explorer`](super::reveal_in_explorer): show `path` in the
/// platform's file manager, selected — Finder, Explorer, or on Linux the
/// file manager's `ShowItems` D-Bus call (falling back to opening the
/// folder). Done by `tauri-plugin-opener` ([`reveal`]), which builds
/// Explorer's selection through the shell API instead of a hand-quoted
/// `explorer /select,"…"` command line (broken by commas and spaces in
/// paths). Returns once the file manager was asked, not when it is done.
///
/// Only paths inside `library` are shown (a book, or a folder such as the
/// library's fallback trash), and they must exist: the frontend has no
/// business pointing the file manager anywhere else.
pub(crate) fn reveal_in_file_manager(library: &Path, path: &Path) -> AppResult<()> {
    check_revealable(library, path)?;
    reveal(path)
}

/// The confinement rules of [`reveal_in_file_manager`], without revealing.
fn check_revealable(library: &Path, path: &Path) -> AppResult<()> {
    if !fsutil::is_within(library, path) {
        return Err(AppError::InvalidInput(format!(
            "{} is not inside the library folder",
            display_name(path)
        )));
    }
    if !path.exists() {
        return Err(AppError::NotFound(format!(
            "{} no longer exists; rescan the library",
            display_name(path)
        )));
    }
    Ok(())
}

/// Show `path` (a file or a folder) selected in the file manager.
pub(crate) fn reveal(path: &Path) -> AppResult<()> {
    tauri_plugin_opener::reveal_item_in_dir(path).map_err(|e| {
        AppError::Other(format!(
            "Could not show {} in the file manager: {e}",
            display_name(path)
        ))
    })
}

/// [`check_path_accessible`](super::check_path_accessible): whether `path`
/// is an accessible directory. Gives up after 3 seconds (a sleeping NAS can
/// block a filesystem call for much longer) and answers `false`.
pub(crate) async fn is_accessible_dir(path: String) -> bool {
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        tokio::task::spawn_blocking(move || std::path::Path::new(&path).is_dir()),
    )
    .await
    .unwrap_or(Ok(false))
    .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_existing_paths_inside_the_library_are_revealed() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path().join("lib");
        std::fs::create_dir_all(&lib).unwrap();
        let outside = dir.path().join("secret.txt");
        std::fs::write(&outside, b"").unwrap();
        let err = reveal_in_file_manager(&lib, &outside).unwrap_err();
        assert_eq!(err.code(), "INVALID_INPUT");
        assert!(err.to_string().contains("not inside the library"), "{err}");
        assert!(
            !err.to_string().contains(&dir.path().display().to_string()),
            "no full path: {err}"
        );
        let err = reveal_in_file_manager(&lib, &lib.join("../secret.txt")).unwrap_err();
        assert!(err.to_string().contains("not inside the library"), "{err}");
        let err = reveal_in_file_manager(&lib, &lib.join("gone.epub")).unwrap_err();
        assert_eq!(err.code(), "NOT_FOUND");
        // An existing path inside the library passes the checks (revealing
        // it would open the file manager, which a test shouldn't do).
        std::fs::write(lib.join("Dune.epub"), b"").unwrap();
        assert!(check_revealable(&lib, &lib.join("Dune.epub")).is_ok());
    }
}
