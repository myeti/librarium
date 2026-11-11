use serde::{Deserialize, Serialize};

/// `KindleDevice.connection` of a USB drive (every Kobo, Kindles up to 2023).
pub const CONNECTION_MASS_STORAGE: &str = "massStorage";
/// `KindleDevice.connection` of an MTP Kindle (2024+, experimental).
pub const CONNECTION_MTP: &str = "mtp";

/// A connected e-reader, as returned by `detect_devices` / `detect_device`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KindleDevice {
    /// Stable per device: a UUID kept in the device's manifest
    /// (`.librarium/device.json`), or `"<type>:<mount path>"` for a device
    /// Librarium can't write to. A different id at the same mount path means
    /// another device was plugged in. Never contains a serial number.
    pub id: String,
    pub name: String,
    /// The volume (`/Volumes/Kindle`, `E:\`), or `mtp://<port>` for an MTP Kindle.
    pub mount_path: String,
    /// "kindle" | "kobo"
    pub device_type: String,
    /// How it is connected: [`CONNECTION_MASS_STORAGE`] or [`CONNECTION_MTP`].
    pub connection: String,
    pub preferred_format: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub free_space_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_space_bytes: Option<u64>,
}

/// One book to send to the device (an element of `send_books`' `jobs`).
///
/// The frontend has always sent these fields in snake_case; the camelCase
/// aliases let it move to the usual IPC spelling without breaking anything.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferRequest {
    /// Job id, echoed in every `transfer-progress` event and used by `cancel_transfer`.
    pub id: String,
    /// Absolute path of the book in the library. Only used without
    /// `book_id`/`library_path` (older frontends); may then be omitted.
    #[serde(default, alias = "bookPath")]
    pub book_path: String,
    #[serde(alias = "needsConversion")]
    pub needs_conversion: bool,
    /// Conversion target when `needs_conversion`: "epub" (Kobo) or "azw3" (Kindle).
    #[serde(alias = "targetFormat")]
    pub target_format: String,
    /// Mount point of the device (`KindleDevice::mount_path`).
    #[serde(alias = "devicePath")]
    pub device_path: String,
    /// Optional folder under `documents/`, from the user's settings. Must be
    /// relative; it is validated and sanitised before use.
    #[serde(default)]
    pub subfolder: Option<String>,
    /// Full path of the book already on the device, if the frontend thinks it
    /// is there (matched by title, so it may be a different book). Only used
    /// to place the new file in the same folder, which preserves the adjacent
    /// `.sdr` folder (Kindle reading state), and only when the device
    /// manifest doesn't know where the book is; it is never deleted.
    #[serde(default, alias = "existingDevicePath")]
    pub existing_device_path: Option<String>,
    /// The library book being sent (`Book.id`), with `library_path`. With
    /// both, the backend reads the book's file, title and author from the
    /// library index, embeds the id in the copy, records it in the device
    /// manifest, and replaces the copy it sent before. One without the
    /// other fails the job.
    #[serde(default, alias = "bookId")]
    pub book_id: Option<String>,
    /// The library folder holding `book_id` (the current user's library).
    #[serde(default, alias = "libraryPath")]
    pub library_path: Option<String>,
}
