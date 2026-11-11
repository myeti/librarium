//! The error type of every Tauri command.
//!
//! Commands return `Result<T, AppError>`. Over IPC an error is a plain object
//!
//! ```json
//! { "code": "LIBRARY_UNAVAILABLE", "message": "The library folder \"Alice\" can't be reached. …" }
//! ```
//!
//! - `code` is stable and machine-readable ([`AppError::code`]): the frontend
//!   branches on it (a Retry button for `LIBRARY_UNAVAILABLE` and
//!   `PERMISSION_DENIED`, for example) and may use it to pick a translated message. Codes are only ever added,
//!   never renamed: they are part of the IPC contract (docs/IPC.md, "Errors").
//! - `message` is one or two English sentences for the user. Messages name
//!   files by their file name, not their full path, where they can: they
//!   end up in toasts, screenshots and bug reports. (The debug log, which
//!   stays on this computer, has the paths.)
//!
//! Deep helpers (the EPUB module, cover extraction…) still return
//! `Result<T, String>`; `?` turns those messages into [`AppError::Other`].
//! The failures the UI needs to tell apart have their own variants and are
//! built where they happen (`files::ensure_library_available`,
//! `index::not_in_library`, `net`, `convert`…).

use std::any::Any;
use std::io;
use std::path::Path;

use serde::ser::{Serialize, SerializeStruct, Serializer};

/// A command failure: a stable code plus a message for the user.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// The library folder (or the library root) doesn't exist: a NAS share
    /// that isn't mounted, an unplugged drive, a folder that was moved.
    /// Nothing was created. The UI offers Retry.
    #[error("The library folder \"{folder}\" can't be reached. Is the drive or NAS connected?")]
    LibraryUnavailable {
        /// The folder's name (not its full path).
        folder: String,
    },

    /// The library folder (or the library root) is there, but the system
    /// refuses to let Librarium read it. On macOS this is almost always
    /// privacy protection (TCC): an app must be allowed to read network and
    /// removable volumes (System Settings › Privacy & Security › Files and
    /// Folders), and until it is, reading anything under `/Volumes` fails
    /// with "Operation not permitted". Not a missing NAS: the share is fine,
    /// and the user (not a reconnect) has to fix it. The UI shows the
    /// message, which says where, and offers Retry.
    #[error("{}", permission_denied_message(folder))]
    PermissionDenied {
        /// The folder's name (not its full path).
        folder: String,
    },

    /// The library index (`.librarium/library.json`) and its backup are both
    /// unreadable. Nothing was written; the message says how to recover.
    #[error("{0}")]
    IndexDamaged(String),

    /// The library index was written by a newer version of Librarium.
    /// Nothing was written; updating the app fixes it.
    #[error("{0}")]
    IndexTooNew(String),

    /// The book, file, folder or device the command is about isn't there
    /// (any more): an unknown book id, a file deleted behind the app's back.
    /// Usually fixed by rescanning.
    #[error("{0}")]
    NotFound(String),

    /// The request itself is invalid: a malformed id or URL, an unsupported
    /// format, an empty name, a path outside the library. Retrying the same
    /// request won't help.
    #[error("{0}")]
    InvalidInput(String),

    /// Reading or writing a file failed (permissions, disk full, I/O error
    /// on a share).
    #[error("{0}")]
    Io(String),

    /// A download or HTTP request failed: no connection, timeout, an error
    /// status, a response that is too large or of the wrong kind.
    #[error("{0}")]
    Network(String),

    /// A book couldn't be converted (DRM, a file boko can't read, an output
    /// that failed validation). The original is untouched.
    #[error("{0}")]
    Conversion(String),

    /// The e-reader is missing, full, or isn't a Kindle/Kobo.
    #[error("{0}")]
    Device(String),

    /// Something changed under the command (the book was renamed meanwhile,
    /// a name is already taken, a port is in use). Trying again may work.
    #[error("{0}")]
    Conflict(String),

    /// A bug: a panic, a task that died. Logged with details.
    #[error("Something went wrong inside Librarium ({0}). The log file has the details.")]
    Internal(String),

    /// Not classified yet: a message from a helper that still returns a
    /// plain `String`. Show the message.
    #[error("{0}")]
    Other(String),
}

/// `Result` with an [`AppError`].
pub type AppResult<T> = Result<T, AppError>;

/// Test hook shared by the parsers' panic guards (scan, conversion): working
/// on a file whose path contains this marker panics, as a parser crashing
/// on a malformed book would.
#[cfg(test)]
pub(crate) const PANIC_TEST_MARKER: &str = "librarium-test-panic";

impl AppError {
    /// The stable, machine-readable code sent to the frontend
    /// (SCREAMING_SNAKE_CASE, one per variant).
    pub fn code(&self) -> &'static str {
        match self {
            Self::LibraryUnavailable { .. } => "LIBRARY_UNAVAILABLE",
            Self::PermissionDenied { .. } => "PERMISSION_DENIED",
            Self::IndexDamaged(_) => "INDEX_DAMAGED",
            Self::IndexTooNew(_) => "INDEX_TOO_NEW",
            Self::NotFound(_) => "NOT_FOUND",
            Self::InvalidInput(_) => "INVALID_INPUT",
            Self::Io(_) => "IO",
            Self::Network(_) => "NETWORK",
            Self::Conversion(_) => "CONVERSION",
            Self::Device(_) => "DEVICE",
            Self::Conflict(_) => "CONFLICT",
            Self::Internal(_) => "INTERNAL",
            Self::Other(_) => "OTHER",
        }
    }

    /// [`AppError::LibraryUnavailable`] for the folder at `path` (only its
    /// name goes into the message).
    pub fn library_unavailable(path: &Path) -> Self {
        Self::LibraryUnavailable {
            folder: display_name(path),
        }
    }

    /// [`AppError::PermissionDenied`] for the folder at `path` (only its
    /// name goes into the message).
    pub fn permission_denied(path: &Path) -> Self {
        Self::PermissionDenied {
            folder: display_name(path),
        }
    }

    /// The error for a library folder (or the library root) at `path` that
    /// couldn't be opened or listed because of `err`:
    /// - access refused ([`is_permission_denied`]) → [`AppError::PermissionDenied`]
    ///   (on macOS, the privacy settings; see the variant);
    /// - not there, or the share under it is gone (not found, timed out,
    ///   not connected, a stale network handle…; [`is_unreachable`]) →
    ///   [`AppError::LibraryUnavailable`], answered with Retry;
    /// - anything else (an I/O error on the share) → [`AppError::Io`], with
    ///   the reason.
    pub fn folder(path: &Path, err: &io::Error) -> Self {
        if is_permission_denied(err) {
            Self::permission_denied(path)
        } else if is_unreachable(err) {
            Self::library_unavailable(path)
        } else {
            Self::io(
                format!("Could not read the folder {}", display_name(path)),
                err,
            )
        }
    }

    /// A failed file operation: `"{context}: {err}"`, e.g. `Could not read
    /// Dune.epub: Permission denied`. A "not found" error becomes
    /// [`AppError::NotFound`], everything else [`AppError::Io`].
    pub fn io(context: impl std::fmt::Display, err: &io::Error) -> Self {
        let message = format!("{context}: {}", io_reason(err));
        if err.kind() == io::ErrorKind::NotFound {
            Self::NotFound(message)
        } else {
            Self::Io(message)
        }
    }

    /// A panic caught while running `what` (a command, a scan of one file),
    /// with the panic's own message.
    pub fn panicked(what: &str, payload: &(dyn Any + Send)) -> Self {
        Self::Internal(format!("{what} crashed: {}", panic_message(payload)))
    }
}

/// The message of [`AppError::PermissionDenied`]: where to fix it, which
/// depends on the platform.
fn permission_denied_message(folder: &str) -> String {
    if cfg!(target_os = "macos") {
        format!(
            "Librarium isn't allowed to open the library folder \"{folder}\". In System Settings › \
             Privacy & Security › Files and Folders, allow Librarium to access Network Volumes \
             (or Removable Volumes, for a USB drive), then try again."
        )
    } else {
        format!(
            "Librarium isn't allowed to open the library folder \"{folder}\". Check the \
             permissions of the folder (or of the network share), then try again."
        )
    }
}

/// Whether `err` means "access refused" rather than "not there": `EACCES`
/// (file permissions) and `EPERM` (on macOS, privacy protection on
/// `/Volumes`, `~/Documents`…). Rust maps both to
/// [`io::ErrorKind::PermissionDenied`]; the raw codes are checked as well so
/// the rule doesn't depend on that mapping.
pub fn is_permission_denied(err: &io::Error) -> bool {
    if err.kind() == io::ErrorKind::PermissionDenied {
        return true;
    }
    #[cfg(unix)]
    if let Some(code) = err.raw_os_error() {
        return code == libc::EPERM || code == libc::EACCES;
    }
    false
}

/// Whether `err` means the folder can't be reached at all: it doesn't exist
/// (an unmounted share leaves an empty mount point, or none), or the network
/// under it failed (timed out, not connected, a stale handle after the NAS
/// restarted). Retrying later may work.
pub fn is_unreachable(err: &io::Error) -> bool {
    use io::ErrorKind as K;
    matches!(
        err.kind(),
        K::NotFound
            | K::NotADirectory
            | K::TimedOut
            | K::NotConnected
            | K::HostUnreachable
            | K::NetworkUnreachable
            | K::NetworkDown
            | K::ConnectionAborted
            | K::ConnectionReset
            | K::StaleNetworkFileHandle
    )
}

/// The last component of `path` for a message (`/Volumes/books/Alice` →
/// `Alice`); the whole path when it has none (`/`, `C:\`).
pub fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// An I/O error as a short reason: the OS message without the
/// `(os error 13)` suffix, which means nothing to a user.
fn io_reason(err: &io::Error) -> String {
    let text = err.to_string();
    match text.rfind(" (os error ") {
        Some(i) if text.ends_with(')') => text[..i].to_string(),
        _ => text,
    }
}

/// The message of a panic payload (`panic!("…")` gives a `&str` or a
/// `String`; anything else is described generically).
pub fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}

/// `{ "code": "...", "message": "..." }`: what the frontend receives as the
/// rejection value of `invoke`.
impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut s = serializer.serialize_struct("AppError", 2)?;
        s.serialize_field("code", self.code())?;
        s.serialize_field("message", &self.to_string())?;
        s.end()
    }
}

/// A message from a helper that still returns `Result<T, String>`.
impl From<String> for AppError {
    fn from(message: String) -> Self {
        Self::Other(message)
    }
}

impl From<&str> for AppError {
    fn from(message: &str) -> Self {
        Self::Other(message.to_string())
    }
}

/// A file operation without more context. Prefer [`AppError::io`], which
/// says what was being done.
impl From<io::Error> for AppError {
    fn from(err: io::Error) -> Self {
        Self::io("A file operation failed", &err)
    }
}

/// Tauri's own errors (resolving the app's folders, the asset scope…).
impl From<tauri::Error> for AppError {
    fn from(err: tauri::Error) -> Self {
        Self::Internal(err.to_string())
    }
}

/// A blocking task that panicked or was cancelled.
impl From<tokio::task::JoinError> for AppError {
    fn from(err: tokio::task::JoinError) -> Self {
        if err.is_panic() {
            Self::panicked("The operation", err.into_panic().as_ref())
        } else {
            Self::Internal(format!("the operation was cancelled ({err})"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn serialises_as_code_and_message() {
        let err = AppError::library_unavailable(Path::new("/Volumes/books/Alice"));
        assert_eq!(
            serde_json::to_value(&err).unwrap(),
            json!({
                "code": "LIBRARY_UNAVAILABLE",
                "message": "The library folder \"Alice\" can't be reached. Is the drive or NAS connected?",
            })
        );
        assert_eq!(
            serde_json::to_value(AppError::NotFound("Book x is not in the library".into()))
                .unwrap(),
            json!({ "code": "NOT_FOUND", "message": "Book x is not in the library" })
        );
    }

    #[test]
    fn every_code_is_screaming_snake_case() {
        let all = [
            AppError::library_unavailable(Path::new("x")),
            AppError::permission_denied(Path::new("x")),
            AppError::IndexDamaged(String::new()),
            AppError::IndexTooNew(String::new()),
            AppError::NotFound(String::new()),
            AppError::InvalidInput(String::new()),
            AppError::Io(String::new()),
            AppError::Network(String::new()),
            AppError::Conversion(String::new()),
            AppError::Device(String::new()),
            AppError::Conflict(String::new()),
            AppError::Internal(String::new()),
            AppError::Other(String::new()),
        ];
        let codes: std::collections::HashSet<_> = all.iter().map(AppError::code).collect();
        assert_eq!(codes.len(), all.len(), "one code per variant");
        for code in codes {
            assert!(
                code.chars().all(|c| c.is_ascii_uppercase() || c == '_'),
                "{code}"
            );
        }
    }

    #[test]
    fn the_unavailable_message_names_the_folder_not_the_path() {
        let err = AppError::library_unavailable(Path::new("/Volumes/books/Alice"));
        assert!(!err.to_string().contains("/Volumes"), "{err}");
        assert_eq!(display_name(Path::new("/")), "/");
    }

    #[test]
    fn io_errors_keep_context_and_drop_os_codes() {
        let denied = io::Error::from_raw_os_error(13);
        let err = AppError::io("Could not read Dune.epub", &denied);
        assert_eq!(err.code(), "IO");
        assert!(
            err.to_string().starts_with("Could not read Dune.epub: "),
            "{err}"
        );
        assert!(!err.to_string().contains("os error"), "{err}");

        let gone = io::Error::new(io::ErrorKind::NotFound, "gone");
        assert_eq!(
            AppError::io("Could not open Dune.epub", &gone).code(),
            "NOT_FOUND"
        );
    }

    #[test]
    fn folder_errors_tell_a_refusal_from_a_missing_share() {
        let lib = Path::new("/Volumes/books/Alice");
        // Access refused, in the platform's own codes: on Unix, EPERM (macOS
        // privacy protection, TCC) and EACCES (file modes); on Windows,
        // ERROR_ACCESS_DENIED (5). The same numbers mean other things there.
        #[cfg(unix)]
        let refusals = [libc::EPERM, libc::EACCES];
        #[cfg(windows)]
        let refusals = [5];
        for code in refusals {
            let err = AppError::folder(lib, &io::Error::from_raw_os_error(code));
            assert_eq!(err.code(), "PERMISSION_DENIED", "errno {code}");
            assert!(err.to_string().contains("\"Alice\""), "{err}");
            assert!(!err.to_string().contains("/Volumes"), "{err}");
        }
        #[cfg(target_os = "macos")]
        assert!(AppError::permission_denied(lib)
            .to_string()
            .contains("Privacy & Security › Files and Folders"));

        for kind in [
            io::ErrorKind::NotFound,
            io::ErrorKind::TimedOut,
            io::ErrorKind::NotConnected,
            io::ErrorKind::StaleNetworkFileHandle,
        ] {
            let err = AppError::folder(lib, &io::Error::from(kind));
            assert_eq!(err.code(), "LIBRARY_UNAVAILABLE", "{kind:?}");
        }
        // Any other I/O error keeps its reason.
        let err = AppError::folder(lib, &io::Error::other("bad sector"));
        assert_eq!(err.code(), "IO");
        assert!(err.to_string().contains("bad sector"), "{err}");
    }

    #[test]
    fn plain_messages_and_panics() {
        let err: AppError = "The OPF is not well-formed".into();
        assert_eq!(
            (err.code(), err.to_string().as_str()),
            ("OTHER", "The OPF is not well-formed")
        );

        let payload = std::panic::catch_unwind(|| panic!("index out of bounds")).unwrap_err();
        let err = AppError::panicked("Reading Dune.epub", payload.as_ref());
        assert_eq!(err.code(), "INTERNAL");
        assert!(
            err.to_string()
                .contains("Reading Dune.epub crashed: index out of bounds"),
            "{err}"
        );
        let formatted = std::panic::catch_unwind(|| panic!("bad {}", 42)).unwrap_err();
        assert_eq!(panic_message(formatted.as_ref()), "bad 42");
    }
}
