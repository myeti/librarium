//! What the rest of Librarium sees of an MTP device: an open session
//! ([`MtpSession`]) with file-level operations, the devices a platform
//! [`Backend`] finds, and the errors ([`MtpError`]).
//!
//! Two transports implement [`MtpSession`]: PTP over raw USB
//! ([`super::ptp`] + [`super::usb`], macOS and Linux) and Windows Portable
//! Devices ([`super::wpd`]). Tests use an in-memory device
//! ([`super::fake`]).
//!
//! Every method blocks. A session is opened on, used by and closed on the
//! one worker thread that owns it ([`super::worker`]): it never crosses
//! threads, so it needs neither `Send` nor `Sync` (WPD's COM objects are
//! neither).

use std::any::Any;
use std::fmt;
use std::io::{Read, Write};

use super::protocol::{self, rc};

/// An object (file or folder) on the device: opaque, transport-specific
/// (a PTP handle in decimal, a WPD object id). Never parsed outside the
/// transport that made it.
pub type ObjectId = String;

/// A storage of the device (a Kindle has one, "Internal Storage").
pub type StorageId = String;

/// One storage, from `storages()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Storage {
    pub id: StorageId,
    /// "Internal Storage".
    pub description: String,
    pub free_bytes: Option<u64>,
    pub capacity_bytes: Option<u64>,
    /// Whether files can be written and deleted.
    pub writable: bool,
}

/// One file or folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Object {
    pub id: ObjectId,
    pub storage: StorageId,
    /// `None` at the root of the storage.
    pub parent: Option<ObjectId>,
    pub name: String,
    pub size: u64,
    pub is_folder: bool,
    /// As the device reports it (MTP: `YYYYMMDDThhmmss`); empty if it
    /// doesn't. Only compared for equality (cache keys).
    pub modified: String,
}

/// What the device says it is. Never its serial number: the transports
/// don't keep it (see [`protocol::parse_device_info`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceDescription {
    pub manufacturer: String,
    /// "Kindle Paperwhite"; empty when unknown.
    pub model: String,
}

/// A partially written object left behind by a failed or cancelled send
/// whose session then had to be dropped: the worker deletes it once
/// reconnected, after checking it is still that object (same name, same
/// folder).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Orphan {
    pub id: ObjectId,
    pub storage: StorageId,
    pub parent: Option<ObjectId>,
    pub name: String,
}

/// Called with the number of bytes transferred so far; returning `false`
/// cancels the transfer.
pub type Progress<'a> = &'a mut dyn FnMut(u64) -> bool;

/// An open connection to one MTP device. See the module docs.
pub trait MtpSession {
    fn description(&self) -> &DeviceDescription;

    /// Every storage, with its free and total space (also the cheap health
    /// check the worker runs when detection polls).
    fn storages(&mut self) -> Result<Vec<Storage>, MtpError>;

    /// The files and folders directly in `parent` (`None`: the storage root).
    fn children(&mut self, storage: &str, parent: Option<&str>) -> Result<Vec<Object>, MtpError>;

    /// One object, by id (to check that an id from an earlier listing still
    /// is the same file).
    fn object(&mut self, id: &str) -> Result<Object, MtpError>;

    /// Create the folder `name` in `parent`.
    fn create_folder(
        &mut self,
        storage: &str,
        parent: Option<&str>,
        name: &str,
    ) -> Result<Object, MtpError>;

    /// Create the file `name` in `parent` with the `size` bytes read from
    /// `source`, streamed (never held in memory whole), reporting progress
    /// and stopping when `progress` returns `false` ([`MtpError::Cancelled`]).
    /// On failure the partial object is deleted, or reported through
    /// [`MtpSession::take_orphans`] when the session can't do it any more.
    fn send_file(
        &mut self,
        storage: &str,
        parent: Option<&str>,
        name: &str,
        size: u64,
        source: &mut dyn Read,
        progress: Progress,
    ) -> Result<Object, MtpError>;

    /// Stream the content of `object` into `sink`; returns the bytes written.
    fn read_file(
        &mut self,
        object: &Object,
        sink: &mut dyn Write,
        progress: Progress,
    ) -> Result<u64, MtpError>;

    /// Delete one file (or an empty folder).
    fn delete(&mut self, id: &str) -> Result<(), MtpError>;

    /// Whether [`MtpSession::rename`] is supported (MTP SetObjectPropValue).
    fn can_rename(&self) -> bool;

    /// Rename a file in place.
    fn rename(&mut self, id: &str, new_name: &str) -> Result<(), MtpError>;

    /// `false` once a transport error left the session unusable: the worker
    /// then drops it and reconnects.
    fn is_healthy(&self) -> bool;

    /// Partial objects to delete after reconnecting (see [`Orphan`]).
    fn take_orphans(&mut self) -> Vec<Orphan>;
}

// ── Devices before a session ──────────────────────────────────────────────────

/// An MTP Kindle found by enumeration, before anything is claimed.
#[derive(Clone, PartialEq, Eq)]
pub struct Candidate {
    /// Where it is plugged in, not who it is (`usb-20-1.2`, `wpd-1`):
    /// stable while it stays plugged into the same port, and never
    /// sensitive. Becomes the `mtp://<key>` mount path.
    pub key: String,
    /// The USB product string ("Kindle"), for messages before a session exists.
    pub label: String,
    /// What the backend needs to open this device again. May contain the
    /// serial number (a Windows PnP id does), so it is never logged,
    /// serialised or shown — `Debug` hides it.
    pub(crate) native: String,
}

impl fmt::Debug for Candidate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Candidate")
            .field("key", &self.key)
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

/// How a platform finds and opens MTP Kindles.
pub trait Backend: Send + Sync {
    /// The MTP Kindles plugged in. Cheap and side-effect free: nothing is
    /// opened or claimed, so this is all that runs while MTP support is
    /// disabled.
    fn enumerate(&self) -> Vec<Candidate>;

    /// Claim the device and open a session. Called on the worker thread
    /// that will own the session.
    fn open(&self, candidate: &Candidate) -> Result<Box<dyn MtpSession>, MtpError>;

    /// Per-thread setup for the worker thread that will own this backend's
    /// sessions, kept alive (and dropped) after every session on it: COM on
    /// Windows, nothing elsewhere.
    fn thread_setup(&self) -> Box<dyn Any> {
        Box::new(())
    }
}

// ── Errors ────────────────────────────────────────────────────────────────────

/// Why a device couldn't be claimed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimKind {
    /// Another program (or a system service) holds the device.
    Busy,
    /// The OS doesn't let this user open it (Linux without the udev rule).
    PermissionDenied,
    Other,
}

/// What can go wrong talking to an MTP device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MtpError {
    /// The device is gone: unplugged, or its USB link dropped.
    Disconnected,
    /// A USB transfer failed or timed out (the text says how).
    Transport(String),
    /// The device sent something that isn't valid PTP/MTP.
    Protocol(String),
    /// The device refused an operation with this response code. Built by
    /// the USB transport (ptp.rs), which Windows builds only for tests: WPD
    /// reports its own errors.
    #[cfg_attr(windows, allow(dead_code))]
    Response { operation: u16, code: u16 },
    /// The device kept answering "busy".
    Busy,
    /// The device couldn't be claimed (see [`ClaimKind`]).
    Claim { kind: ClaimKind, detail: String },
    /// The user cancelled.
    Cancelled,
    /// A local file couldn't be read or written.
    Local(String),
    /// A file or folder isn't (or is no longer) on the device.
    NotFound(String),
    /// Librarium won't do it; the message says why, for the user (not
    /// enough space, a file too large…).
    Refused(String),
    /// The device or transport doesn't support this. Built by the USB
    /// transport only (ptp.rs, usb.rs), as `Response`.
    #[cfg_attr(windows, allow(dead_code))]
    Unsupported(String),
    /// The device's worker has stopped (unplugged, ejected, MTP disabled).
    WorkerGone,
}

impl MtpError {
    /// Whether the session can't be trusted after this error (the worker
    /// drops it and reconnects). Response codes are not fatal, except the
    /// two that mean the device and Librarium disagree on the session.
    pub fn is_fatal(&self) -> bool {
        match self {
            Self::Disconnected | Self::Transport(_) | Self::Protocol(_) => true,
            Self::Response { code, .. } => {
                matches!(*code, rc::SESSION_NOT_OPEN | rc::INVALID_TRANSACTION_ID)
            }
            _ => false,
        }
    }

    /// One or two sentences for the user (a toast, a job's error).
    pub fn user_message(&self) -> String {
        match self {
            Self::Disconnected | Self::WorkerGone => {
                "The Kindle was disconnected. Plug it in again (and unlock it), then retry.".to_string()
            }
            Self::Transport(detail) => format!("The connection to the Kindle failed ({detail}). Librarium reconnects automatically; retry in a moment."),
            Self::Protocol(detail) => format!("The Kindle sent an unexpected reply ({detail}). Librarium reconnects automatically; retry in a moment."),
            Self::Response { code: rc::STORE_FULL, .. } => "The Kindle is full.".to_string(),
            Self::Response { code: rc::STORE_READ_ONLY | rc::OBJECT_WRITE_PROTECTED | rc::ACCESS_DENIED, .. } => {
                "The Kindle refused to change this file (it is read-only or protected).".to_string()
            }
            Self::Response { code: rc::INVALID_OBJECT_HANDLE, .. } => {
                "The file is no longer on the Kindle. Refresh the Kindle's books.".to_string()
            }
            Self::Response { operation, code } => format!(
                "The Kindle refused the operation ({}, code 0x{code:04X}, operation 0x{operation:04X}).",
                protocol::response_name(*code)
            ),
            Self::Busy => "The Kindle is busy. Wait a moment and retry.".to_string(),
            Self::Claim { kind, .. } => claim_message(*kind),
            Self::Cancelled => "Cancelled.".to_string(),
            Self::Local(detail) => detail.clone(),
            Self::NotFound(what) => format!("{what} is not on the Kindle (any more). Refresh the Kindle's books."),
            Self::Refused(message) => message.clone(),
            Self::Unsupported(what) => format!("This Kindle doesn't support {what} over MTP."),
        }
    }
}

impl fmt::Display for MtpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Claim { detail, .. } => write!(f, "{} ({detail})", self.user_message()),
            Self::Response { operation, code } => {
                write!(
                    f,
                    "operation 0x{operation:04X} failed: {} (0x{code:04X})",
                    protocol::response_name(*code)
                )
            }
            other => f.write_str(&other.user_message()),
        }
    }
}

impl From<protocol::Malformed> for MtpError {
    fn from(err: protocol::Malformed) -> Self {
        Self::Protocol(err.0.to_string())
    }
}

/// What to do when the device can't be claimed, per platform.
pub fn claim_message(kind: ClaimKind) -> String {
    match kind {
        ClaimKind::PermissionDenied if cfg!(target_os = "linux") => "Librarium isn't allowed to open the Kindle. Install Librarium's udev rule \
             (70-librarium-kindle.rules: the .deb package installs it; for the AppImage, copy it to /etc/udev/rules.d/ \
             and run `sudo udevadm control --reload-rules && sudo udevadm trigger`), then unplug and replug the Kindle."
            .to_string(),
        ClaimKind::PermissionDenied => {
            "Librarium isn't allowed to open the Kindle. Unplug it, plug it in again and retry.".to_string()
        }
        _ if cfg!(target_os = "macos") => "Another app is using the Kindle. Quit Image Capture, Photos, Android File Transfer \
             or OpenMTP, then unplug and replug the Kindle. If it still fails, macOS's camera service may be holding it: \
             run `killall -9 ptpcamerad` in Terminal and replug."
            .to_string(),
        _ if cfg!(target_os = "linux") => "Another program is using the Kindle, usually the file manager's MTP support \
             (gvfs-mtp on GNOME, kio-mtp on KDE). Unmount the Kindle in the file manager (or quit the program), then \
             unplug and replug it."
            .to_string(),
        _ => "Windows couldn't open the Kindle; another program may be using it. Close it, then unplug and replug the Kindle."
            .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_transport_level_errors_drop_the_session() {
        assert!(MtpError::Disconnected.is_fatal());
        assert!(MtpError::Transport("timeout".into()).is_fatal());
        assert!(MtpError::Protocol("bad".into()).is_fatal());
        assert!(MtpError::Response {
            operation: 0x1008,
            code: rc::SESSION_NOT_OPEN
        }
        .is_fatal());
        assert!(!MtpError::Response {
            operation: 0x1008,
            code: rc::INVALID_OBJECT_HANDLE
        }
        .is_fatal());
        assert!(!MtpError::Busy.is_fatal());
        assert!(!MtpError::Cancelled.is_fatal());
        assert!(!MtpError::Refused("full".into()).is_fatal());
    }

    #[test]
    fn messages_are_for_people() {
        assert_eq!(
            MtpError::Response {
                operation: 0x100D,
                code: rc::STORE_FULL
            }
            .user_message(),
            "The Kindle is full."
        );
        let generic = MtpError::Response {
            operation: 0x100B,
            code: rc::GENERAL_ERROR,
        }
        .user_message();
        assert!(
            generic.contains("general error")
                && generic.contains("0x2002")
                && generic.contains("0x100B"),
            "{generic}"
        );
        let claim = MtpError::Claim {
            kind: ClaimKind::Busy,
            detail: "exclusive access".into(),
        };
        assert!(claim.user_message().contains("replug"), "{claim}");
        assert!(
            claim.to_string().contains("exclusive access"),
            "details stay in the log text"
        );
    }

    #[test]
    fn candidates_never_show_their_native_id() {
        let c = Candidate {
            key: "wpd-1".into(),
            label: "Kindle".into(),
            native: r"\\?\usb#vid_1949&pid_9981#G000SERIAL#{x}".into(),
        };
        let shown = format!("{c:?}");
        assert!(
            shown.contains("wpd-1") && !shown.contains("G000SERIAL"),
            "{shown}"
        );
    }
}
