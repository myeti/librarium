//! The PTP/MTP wire format: containers, datasets, strings, codes. Pure
//! functions on byte slices, no I/O, unit-tested below.
//!
//! References: ISO 15740 (PTP, "PIMA 15740:2000"), the USB Still Image
//! Capture Device Definition 1.0 (the USB transport of PTP: containers,
//! class requests, zero-length packets) and the USB-IF MTP 1.1 spec (MTP
//! is PTP plus object properties).
//!
//! Every multi-byte value is little-endian. A **container** is a 12-byte
//! header (`length: u32` including the header, `kind: u16`, `code: u16`,
//! `transaction_id: u32`) followed by its payload: up to five `u32`
//! parameters for commands and responses, a dataset or file bytes for data.
//!
//! One operation (a *transaction*) is: a command container, an optional data
//! phase (a data container, host → device or device → host), then a response
//! container carrying the same transaction id.

use std::fmt;

// ── Codes ─────────────────────────────────────────────────────────────────────

/// Operation codes (PTP 1.0 §10.4, MTP 1.1 appendix D).
pub(crate) mod op {
    pub const GET_DEVICE_INFO: u16 = 0x1001;
    pub const OPEN_SESSION: u16 = 0x1002;
    pub const CLOSE_SESSION: u16 = 0x1003;
    pub const GET_STORAGE_IDS: u16 = 0x1004;
    pub const GET_STORAGE_INFO: u16 = 0x1005;
    pub const GET_OBJECT_HANDLES: u16 = 0x1007;
    pub const GET_OBJECT_INFO: u16 = 0x1008;
    pub const GET_OBJECT: u16 = 0x1009;
    /// 0x100B. (The `feat/mtp` branch used 0x1010, which is ResetDevice.)
    pub const DELETE_OBJECT: u16 = 0x100B;
    pub const SEND_OBJECT_INFO: u16 = 0x100C;
    pub const SEND_OBJECT: u16 = 0x100D;
    pub const GET_PARTIAL_OBJECT: u16 = 0x101B;
    /// MTP: set one property of an object (used to rename: ObjectFileName).
    pub const SET_OBJECT_PROP_VALUE: u16 = 0x9804;
}

/// Response codes (PTP 1.0 §11, MTP 1.1 appendix F).
pub(crate) mod rc {
    pub const OK: u16 = 0x2001;
    pub const GENERAL_ERROR: u16 = 0x2002;
    pub const SESSION_NOT_OPEN: u16 = 0x2003;
    pub const INVALID_TRANSACTION_ID: u16 = 0x2004;
    pub const OPERATION_NOT_SUPPORTED: u16 = 0x2005;
    pub const PARAMETER_NOT_SUPPORTED: u16 = 0x2006;
    pub const INCOMPLETE_TRANSFER: u16 = 0x2007;
    pub const INVALID_STORAGE_ID: u16 = 0x2008;
    pub const INVALID_OBJECT_HANDLE: u16 = 0x2009;
    pub const STORE_FULL: u16 = 0x200C;
    pub const OBJECT_WRITE_PROTECTED: u16 = 0x200D;
    pub const STORE_READ_ONLY: u16 = 0x200E;
    pub const ACCESS_DENIED: u16 = 0x200F;
    pub const PARTIAL_DELETION: u16 = 0x2012;
    pub const STORE_NOT_AVAILABLE: u16 = 0x2013;
    pub const DEVICE_BUSY: u16 = 0x2019;
    pub const INVALID_PARENT_OBJECT: u16 = 0x201A;
    pub const INVALID_PARAMETER: u16 = 0x201D;
    pub const SESSION_ALREADY_OPEN: u16 = 0x201E;
    pub const TRANSACTION_CANCELLED: u16 = 0x201F;
    pub const OBJECT_TOO_LARGE: u16 = 0xA809;
}

/// Object format codes.
pub(crate) mod format {
    /// "Undefined" (a plain file): what ebooks are sent as.
    /// TODO(device-validation): confirm a Kindle accepts AZW3/EPUB/PDF sent
    /// with this format (some responders want a specific code per type).
    pub const UNDEFINED: u16 = 0x3000;
    /// An association, i.e. a folder.
    pub const ASSOCIATION: u16 = 0x3001;
}

/// AssociationType of a folder.
pub(crate) const ASSOCIATION_GENERIC_FOLDER: u16 = 0x0001;

/// MTP object property codes.
pub(crate) mod prop {
    pub const OBJECT_FILE_NAME: u16 = 0xDC07;
}

/// USB Still Image class requests (USB SIC 1.0 §5.2), sent on the control
/// endpoint to the PTP interface.
pub(crate) mod class_request {
    /// Abort the current transaction (its data phase).
    pub const CANCEL: u8 = 0x64;
    /// Return the responder to its idle state; closes the session.
    pub const DEVICE_RESET: u8 = 0x66;
    /// `wLength`, code (OK / busy / cancelled…), stalled endpoints.
    pub const GET_DEVICE_STATUS: u8 = 0x67;
}

/// The cancellation code of a Cancel request's data.
const CANCELLATION_CODE: u16 = 0x4001;

/// `GetObjectHandles` third parameter: objects at the root of the storage.
/// (0 would mean every object of the storage, at any depth.)
pub(crate) const PARENT_ROOT: u32 = 0xFFFF_FFFF;

/// `GetObjectHandles` second parameter: every format.
pub(crate) const ALL_FORMATS: u32 = 0;

/// The `ParentObject` of an object at the root of a storage, in ObjectInfo.
pub(crate) const NO_PARENT: u32 = 0;

/// The session id Librarium opens. Any non-zero value works (0 means "no
/// session"); there is only ever one session per device connection.
pub(crate) const SESSION_ID: u32 = 1;

/// The transaction id of OpenSession. PTP requires OpenSession to carry
/// transaction id 0, and the first operation inside the session to carry 1
/// (ISO 15740 §9.3.1, "transaction IDs"; libgphoto2's `ptp_opensession`
/// does the same). The `feat/mtp` branch started at 1.
pub(crate) const OPEN_SESSION_TRANSACTION_ID: u32 = 0;

/// The transaction id after `id`: 1, 2, … 0xFFFF_FFFE, 0xFFFF_FFFF, then 1
/// again (0 is reserved for OpenSession).
pub(crate) fn next_transaction_id(id: u32) -> u32 {
    match id.wrapping_add(1) {
        0 => 1,
        next => next,
    }
}

/// A short, human-readable name for a response code, for messages and logs.
pub(crate) fn response_name(code: u16) -> &'static str {
    match code {
        rc::OK => "OK",
        rc::GENERAL_ERROR => "general error",
        rc::SESSION_NOT_OPEN => "session not open",
        rc::INVALID_TRANSACTION_ID => "invalid transaction id",
        rc::OPERATION_NOT_SUPPORTED => "operation not supported",
        rc::PARAMETER_NOT_SUPPORTED => "parameter not supported",
        rc::INCOMPLETE_TRANSFER => "incomplete transfer",
        rc::INVALID_STORAGE_ID => "invalid storage",
        rc::INVALID_OBJECT_HANDLE => "no such object",
        rc::STORE_FULL => "storage full",
        rc::OBJECT_WRITE_PROTECTED => "object write-protected",
        rc::STORE_READ_ONLY => "storage read-only",
        rc::ACCESS_DENIED => "access denied",
        rc::PARTIAL_DELETION => "partial deletion",
        rc::STORE_NOT_AVAILABLE => "storage not available",
        rc::DEVICE_BUSY => "device busy",
        rc::INVALID_PARENT_OBJECT => "invalid parent folder",
        rc::INVALID_PARAMETER => "invalid parameter",
        rc::SESSION_ALREADY_OPEN => "session already open",
        rc::TRANSACTION_CANCELLED => "transaction cancelled",
        rc::OBJECT_TOO_LARGE => "object too large",
        _ => "error",
    }
}

// ── Containers ────────────────────────────────────────────────────────────────

/// Size of a container header.
pub(crate) const HEADER_LEN: usize = 12;

/// Most parameters a command or response container carries.
pub(crate) const MAX_PARAMS: usize = 5;

/// Container types (the header's `kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContainerKind {
    Command,
    Data,
    Response,
    Event,
}

impl ContainerKind {
    fn code(self) -> u16 {
        match self {
            Self::Command => 1,
            Self::Data => 2,
            Self::Response => 3,
            Self::Event => 4,
        }
    }

    fn from_code(code: u16) -> Option<Self> {
        match code {
            1 => Some(Self::Command),
            2 => Some(Self::Data),
            3 => Some(Self::Response),
            4 => Some(Self::Event),
            _ => None,
        }
    }
}

/// A container header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Header {
    /// Whole container, header included. 0xFFFF_FFFF in a data container
    /// means "more than 4 GB, read until a short packet" (never sent by
    /// Librarium; files that large are refused).
    pub length: u32,
    pub kind: ContainerKind,
    /// The operation code (command, data) or response code (response).
    pub code: u16,
    pub transaction_id: u32,
}

/// Bytes that don't form a valid container or dataset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Malformed(pub &'static str);

impl fmt::Display for Malformed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "malformed MTP data: {}", self.0)
    }
}

fn header_bytes(
    length: u32,
    kind: ContainerKind,
    code: u16,
    transaction_id: u32,
) -> [u8; HEADER_LEN] {
    let mut out = [0u8; HEADER_LEN];
    out[0..4].copy_from_slice(&length.to_le_bytes());
    out[4..6].copy_from_slice(&kind.code().to_le_bytes());
    out[6..8].copy_from_slice(&code.to_le_bytes());
    out[8..12].copy_from_slice(&transaction_id.to_le_bytes());
    out
}

/// A command container. Panics on more than [`MAX_PARAMS`] parameters (a
/// programming error: every operation here has at most three).
pub(crate) fn encode_command(code: u16, transaction_id: u32, params: &[u32]) -> Vec<u8> {
    assert!(
        params.len() <= MAX_PARAMS,
        "PTP commands carry at most 5 parameters"
    );
    let len = HEADER_LEN + 4 * params.len();
    let mut out = header_bytes(len as u32, ContainerKind::Command, code, transaction_id).to_vec();
    for p in params {
        out.extend_from_slice(&p.to_le_bytes());
    }
    out
}

/// A response container (only devices send these; used by the test
/// responder).
#[cfg(test)]
pub(crate) fn encode_response(code: u16, transaction_id: u32, params: &[u32]) -> Vec<u8> {
    let len = HEADER_LEN + 4 * params.len();
    let mut out = header_bytes(len as u32, ContainerKind::Response, code, transaction_id).to_vec();
    for p in params {
        out.extend_from_slice(&p.to_le_bytes());
    }
    out
}

/// The largest payload a data container can announce (its length field is
/// a `u32` that includes the header).
pub(crate) const MAX_DATA_PAYLOAD: u64 = u32::MAX as u64 - HEADER_LEN as u64 - 1;

/// The header of a data container whose payload is `payload_len` bytes;
/// `None` if that doesn't fit the length field ([`MAX_DATA_PAYLOAD`]).
pub(crate) fn encode_data_header(
    code: u16,
    transaction_id: u32,
    payload_len: u64,
) -> Option<[u8; HEADER_LEN]> {
    if payload_len > MAX_DATA_PAYLOAD {
        return None;
    }
    Some(header_bytes(
        payload_len as u32 + HEADER_LEN as u32,
        ContainerKind::Data,
        code,
        transaction_id,
    ))
}

/// The header at the start of `buf`.
pub(crate) fn decode_header(buf: &[u8]) -> Result<Header, Malformed> {
    if buf.len() < HEADER_LEN {
        return Err(Malformed("container shorter than its header"));
    }
    let u32_at = |i: usize| u32::from_le_bytes([buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]);
    let u16_at = |i: usize| u16::from_le_bytes([buf[i], buf[i + 1]]);
    let length = u32_at(0);
    if (length as usize) < HEADER_LEN {
        return Err(Malformed("container length smaller than its header"));
    }
    let kind = ContainerKind::from_code(u16_at(4)).ok_or(Malformed("unknown container type"))?;
    Ok(Header {
        length,
        kind,
        code: u16_at(6),
        transaction_id: u32_at(8),
    })
}

/// A decoded response container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Response {
    pub code: u16,
    pub transaction_id: u32,
    pub params: Vec<u32>,
}

/// Decode the response container `buf` (exactly one container: its
/// declared length must match).
pub(crate) fn decode_response(buf: &[u8]) -> Result<Response, Malformed> {
    let header = decode_header(buf)?;
    if header.kind != ContainerKind::Response {
        return Err(Malformed("expected a response container"));
    }
    let len = header.length as usize;
    if len != buf.len()
        || !(len - HEADER_LEN).is_multiple_of(4)
        || (len - HEADER_LEN) / 4 > MAX_PARAMS
    {
        return Err(Malformed("bad response length"));
    }
    let params = buf[HEADER_LEN..]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| u32::from_le_bytes(*c))
        .collect();
    Ok(Response {
        code: header.code,
        transaction_id: header.transaction_id,
        params,
    })
}

/// Whether a bulk OUT transfer of `total_len` bytes must be followed by a
/// zero-length packet: when it is an exact multiple of the endpoint's max
/// packet size, the device can't otherwise tell that the phase ended (USB
/// SIC 1.0 §7.2). Without it the device waits, the response never comes
/// and the operation times out — one file in 512 (or 64, or 1024, with the
/// endpoint's packet size). nusb doesn't add it to raw transfers.
///
/// This holds only if every earlier transfer of the phase was itself a
/// multiple of the packet size (a short packet ends the phase early): the
/// USB transport writes in chunks of [`super::ptp::CHUNK`], a multiple of
/// every bulk packet size (64, 512, 1024).
pub(crate) fn needs_zero_length_packet(total_len: u64, max_packet_size: usize) -> bool {
    max_packet_size > 0 && total_len > 0 && total_len.is_multiple_of(max_packet_size as u64)
}

/// The data of a USB SIC Cancel request for transaction `transaction_id`.
pub(crate) fn encode_cancel_request(transaction_id: u32) -> [u8; 6] {
    let mut out = [0u8; 6];
    out[0..2].copy_from_slice(&CANCELLATION_CODE.to_le_bytes());
    out[2..6].copy_from_slice(&transaction_id.to_le_bytes());
    out
}

/// The response code a USB SIC Get Device Status request returned (`OK`
/// once the device is idle again, `DEVICE_BUSY` while it still processes a
/// cancel), and the endpoints it reports stalled (to clear).
pub(crate) fn decode_device_status(buf: &[u8]) -> Result<(u16, Vec<u8>), Malformed> {
    if buf.len() < 4 {
        return Err(Malformed("device status shorter than 4 bytes"));
    }
    let len = (u16::from_le_bytes([buf[0], buf[1]]) as usize).min(buf.len());
    let code = u16::from_le_bytes([buf[2], buf[3]]);
    // Each stalled endpoint is a u32 holding the endpoint address.
    let stalled = buf[4..len.max(4)]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| c[0])
        .collect();
    Ok((code, stalled))
}

// ── Datasets ──────────────────────────────────────────────────────────────────

/// Reads a dataset; every read is bounds-checked (a truncated dataset is an
/// error, never a silent zero).
pub(crate) struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], Malformed> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|&e| e <= self.buf.len())
            .ok_or(Malformed("truncated dataset"))?;
        let out = &self.buf[self.pos..end];
        self.pos = end;
        Ok(out)
    }

    pub(crate) fn u8(&mut self) -> Result<u8, Malformed> {
        Ok(self.take(1)?[0])
    }

    pub(crate) fn u16(&mut self) -> Result<u16, Malformed> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    pub(crate) fn u32(&mut self) -> Result<u32, Malformed> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub(crate) fn u64(&mut self) -> Result<u64, Malformed> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes(b.try_into().expect("8 bytes")))
    }

    /// A PTP string: a `u8` count of UTF-16 code units *including* the
    /// terminating NUL (0 for an empty string, which then has no NUL), then
    /// the code units. Invalid UTF-16 is replaced (U+FFFD).
    pub(crate) fn string(&mut self) -> Result<String, Malformed> {
        let count = self.u8()? as usize;
        let bytes = self.take(count * 2)?;
        let mut units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        while units.last() == Some(&0) {
            units.pop();
        }
        Ok(String::from_utf16_lossy(&units))
    }

    /// An array of `u16`: a `u32` count, then the elements.
    pub(crate) fn u16_array(&mut self) -> Result<Vec<u16>, Malformed> {
        let count = self.u32()? as usize;
        let bytes = self.take(count.checked_mul(2).ok_or(Malformed("array too long"))?)?;
        Ok(bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect())
    }

    /// An array of `u32`: a `u32` count, then the elements.
    pub(crate) fn u32_array(&mut self) -> Result<Vec<u32>, Malformed> {
        let count = self.u32()? as usize;
        let bytes = self.take(count.checked_mul(4).ok_or(Malformed("array too long"))?)?;
        Ok(bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| u32::from_le_bytes(*c))
            .collect())
    }
}

/// Builds a dataset.
#[derive(Default)]
pub(crate) struct Writer {
    pub buf: Vec<u8>,
}

/// Longest PTP string, in UTF-16 code units, without its NUL.
pub(crate) const MAX_STRING_UNITS: usize = 254;

impl Writer {
    pub(crate) fn u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub(crate) fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    #[cfg(test)]
    pub(crate) fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    /// A PTP string (see [`Reader::string`]). Fails for strings longer than
    /// [`MAX_STRING_UNITS`] UTF-16 code units: a file name is never
    /// truncated silently (it would be another name).
    pub(crate) fn string(&mut self, s: &str) -> Result<(), Malformed> {
        if s.is_empty() {
            self.buf.push(0);
            return Ok(());
        }
        let units: Vec<u16> = s.encode_utf16().collect();
        if units.len() > MAX_STRING_UNITS {
            return Err(Malformed("string longer than 254 characters"));
        }
        self.buf.push((units.len() + 1) as u8);
        for u in units.iter().chain(std::iter::once(&0)) {
            self.buf.extend_from_slice(&u.to_le_bytes());
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn u16_array(&mut self, items: &[u16]) {
        self.u32(items.len() as u32);
        for &v in items {
            self.u16(v);
        }
    }

    #[cfg(test)]
    pub(crate) fn u32_array(&mut self, items: &[u32]) {
        self.u32(items.len() as u32);
        for &v in items {
            self.u32(v);
        }
    }
}

/// The parts of the DeviceInfo dataset Librarium uses.
///
/// The dataset also carries the device's **serial number**. It is read
/// (the fields after it are in a fixed order) but never kept: Librarium
/// doesn't need it, and it must never reach the logs, the UI or analytics.
/// A model string that contains it is dropped.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct DeviceInfo {
    pub vendor_extension_id: u32,
    /// Operation codes the device supports.
    pub operations: Vec<u16>,
    pub manufacturer: String,
    pub model: String,
    pub device_version: String,
}

impl DeviceInfo {
    pub(crate) fn supports(&self, operation: u16) -> bool {
        self.operations.contains(&operation)
    }
}

/// Parse a DeviceInfo dataset (PTP 1.0 §5.5.1).
pub(crate) fn parse_device_info(data: &[u8]) -> Result<DeviceInfo, Malformed> {
    let mut r = Reader::new(data);
    let _standard_version = r.u16()?;
    let vendor_extension_id = r.u32()?;
    let _vendor_extension_version = r.u16()?;
    let _vendor_extension_desc = r.string()?;
    let _functional_mode = r.u16()?;
    let operations = r.u16_array()?;
    let _events = r.u16_array()?;
    let _device_properties = r.u16_array()?;
    let _capture_formats = r.u16_array()?;
    let _playback_formats = r.u16_array()?;
    let manufacturer = r.string()?;
    let mut model = r.string()?;
    let device_version = r.string()?;
    // The serial number: read to check the model string, then dropped.
    let serial = r.string().unwrap_or_default();
    if !serial.trim().is_empty() && model.contains(serial.trim()) {
        model.clear();
    }
    Ok(DeviceInfo {
        vendor_extension_id,
        operations,
        manufacturer,
        model,
        device_version,
    })
}

/// A storage's AccessCapability.
pub(crate) const ACCESS_READ_WRITE: u16 = 0;

/// The StorageInfo dataset (PTP 1.0 §5.5.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StorageInfo {
    pub storage_type: u16,
    pub filesystem_type: u16,
    /// 0 read-write, 1 read-only, 2 read-only with deletion.
    pub access: u16,
    pub max_capacity: u64,
    pub free_space: u64,
    pub description: String,
    pub volume_label: String,
}

/// Parse a StorageInfo dataset.
pub(crate) fn parse_storage_info(data: &[u8]) -> Result<StorageInfo, Malformed> {
    let mut r = Reader::new(data);
    let storage_type = r.u16()?;
    let filesystem_type = r.u16()?;
    let access = r.u16()?;
    let max_capacity = r.u64()?;
    let free_space = r.u64()?;
    let _free_space_in_objects = r.u32()?;
    // Some responders end the dataset early: the strings are optional.
    let description = r.string().unwrap_or_default();
    let volume_label = r.string().unwrap_or_default();
    Ok(StorageInfo {
        storage_type,
        filesystem_type,
        access,
        max_capacity,
        free_space,
        description,
        volume_label,
    })
}

/// The ObjectInfo dataset (PTP 1.0 §5.5.2), the fields Librarium uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ObjectInfo {
    pub storage_id: u32,
    pub format: u16,
    /// Bytes; 0xFFFF_FFFF for objects of 4 GB or more.
    pub compressed_size: u32,
    /// [`NO_PARENT`] at the root of the storage.
    pub parent: u32,
    pub association_type: u16,
    pub filename: String,
    /// `YYYYMMDDThhmmss[.s][Z|±hhmm]`, or empty.
    pub modification_date: String,
}

impl ObjectInfo {
    pub(crate) fn is_folder(&self) -> bool {
        self.format == format::ASSOCIATION
    }
}

/// Parse an ObjectInfo dataset.
pub(crate) fn parse_object_info(data: &[u8]) -> Result<ObjectInfo, Malformed> {
    let mut r = Reader::new(data);
    let storage_id = r.u32()?;
    let format = r.u16()?;
    let _protection_status = r.u16()?;
    let compressed_size = r.u32()?;
    let _thumb_format = r.u16()?;
    let _thumb_compressed_size = r.u32()?;
    let _thumb_pix_width = r.u32()?;
    let _thumb_pix_height = r.u32()?;
    let _image_pix_width = r.u32()?;
    let _image_pix_height = r.u32()?;
    let _image_bit_depth = r.u32()?;
    let parent = r.u32()?;
    let association_type = r.u16()?;
    let _association_desc = r.u32()?;
    let _sequence_number = r.u32()?;
    let filename = r.string()?;
    let _capture_date = r.string().unwrap_or_default();
    let modification_date = r.string().unwrap_or_default();
    Ok(ObjectInfo {
        storage_id,
        format,
        compressed_size,
        parent,
        association_type,
        filename,
        modification_date,
    })
}

/// The ObjectInfo dataset of SendObjectInfo: a file of `size` bytes, or a
/// folder (`folder`, size 0), named `filename`, in `parent` of `storage_id`.
/// Fails if the name is too long for a PTP string or `size` doesn't fit
/// the dataset's 32-bit size field.
pub(crate) fn encode_object_info(
    storage_id: u32,
    parent: u32,
    filename: &str,
    size: u64,
    folder: bool,
) -> Result<Vec<u8>, Malformed> {
    let size = u32::try_from(size)
        .ok()
        .filter(|&s| s != u32::MAX)
        .ok_or(Malformed("object larger than 4 GB"))?;
    let mut w = Writer::default();
    w.u32(storage_id);
    w.u16(if folder {
        format::ASSOCIATION
    } else {
        format::UNDEFINED
    });
    w.u16(0); // ProtectionStatus: none
    w.u32(size); // ObjectCompressedSize
    w.u16(0); // ThumbFormat
    w.u32(0); // ThumbCompressedSize
    w.u32(0); // ThumbPixWidth
    w.u32(0); // ThumbPixHeight
    w.u32(0); // ImagePixWidth
    w.u32(0); // ImagePixHeight
    w.u32(0); // ImageBitDepth
    w.u32(parent); // ParentObject (ignored by most devices: SendObjectInfo's parameter decides)
    w.u16(if folder {
        ASSOCIATION_GENERIC_FOLDER
    } else {
        0
    });
    w.u32(0); // AssociationDesc
    w.u32(0); // SequenceNumber
    w.string(filename)?;
    w.string("")?; // CaptureDate
    w.string("")?; // ModificationDate
    w.string("")?; // Keywords
    Ok(w.buf)
}

/// The data of SetObjectPropValue(ObjectFileName): the new name as a PTP string.
pub(crate) fn encode_string(s: &str) -> Result<Vec<u8>, Malformed> {
    let mut w = Writer::default();
    w.string(s)?;
    Ok(w.buf)
}

/// A DeviceInfo dataset (tests: the fake responder's answer).
#[cfg(test)]
pub(crate) fn encode_device_info(
    operations: &[u16],
    manufacturer: &str,
    model: &str,
    serial: &str,
) -> Vec<u8> {
    let mut w = Writer::default();
    w.u16(100); // StandardVersion 1.00
    w.u32(6); // VendorExtensionID: Microsoft (MTP)
    w.u16(100);
    w.string("microsoft.com: 1.0;").unwrap();
    w.u16(0);
    w.u16_array(operations);
    w.u16_array(&[]);
    w.u16_array(&[]);
    w.u16_array(&[]);
    w.u16_array(&[format::UNDEFINED, format::ASSOCIATION]);
    w.string(manufacturer).unwrap();
    w.string(model).unwrap();
    w.string("1.0").unwrap();
    w.string(serial).unwrap();
    w.buf
}

/// A StorageInfo dataset (tests).
#[cfg(test)]
pub(crate) fn encode_storage_info(info: &StorageInfo) -> Vec<u8> {
    let mut w = Writer::default();
    w.u16(info.storage_type);
    w.u16(info.filesystem_type);
    w.u16(info.access);
    w.u64(info.max_capacity);
    w.u64(info.free_space);
    w.u32(0xFFFF_FFFF);
    w.string(&info.description).unwrap();
    w.string(&info.volume_label).unwrap();
    w.buf
}

/// An ObjectInfo dataset as a device sends it (tests).
#[cfg(test)]
pub(crate) fn encode_object_info_reply(info: &ObjectInfo) -> Vec<u8> {
    let mut w = Writer::default();
    w.u32(info.storage_id);
    w.u16(info.format);
    w.u16(0);
    w.u32(info.compressed_size);
    w.u16(0);
    w.u32(0);
    w.u32(0);
    w.u32(0);
    w.u32(0);
    w.u32(0);
    w.u32(0);
    w.u32(info.parent);
    w.u16(info.association_type);
    w.u32(0);
    w.u32(0);
    w.string(&info.filename).unwrap();
    w.string("").unwrap();
    w.string(&info.modification_date).unwrap();
    w.string("").unwrap();
    w.buf
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_and_responses_round_trip() {
        let cmd = encode_command(
            op::GET_OBJECT_HANDLES,
            7,
            &[0x0001_0001, ALL_FORMATS, PARENT_ROOT],
        );
        assert_eq!(cmd.len(), 24);
        assert_eq!(&cmd[..12], &[24, 0, 0, 0, 1, 0, 0x07, 0x10, 7, 0, 0, 0]);
        let header = decode_header(&cmd).unwrap();
        assert_eq!(
            header,
            Header {
                length: 24,
                kind: ContainerKind::Command,
                code: op::GET_OBJECT_HANDLES,
                transaction_id: 7
            }
        );

        let resp = encode_response(rc::OK, 7, &[1, 2, 3]);
        assert_eq!(
            decode_response(&resp).unwrap(),
            Response {
                code: rc::OK,
                transaction_id: 7,
                params: vec![1, 2, 3]
            }
        );
        assert_eq!(
            decode_response(&encode_response(rc::DEVICE_BUSY, 9, &[]))
                .unwrap()
                .params,
            Vec::<u32>::new()
        );
    }

    #[test]
    fn malformed_containers_are_rejected() {
        assert!(decode_header(&[0u8; 11]).is_err(), "too short");
        let mut bad_len = encode_command(op::OPEN_SESSION, 0, &[1]);
        bad_len[0] = 4;
        assert!(decode_header(&bad_len).is_err(), "length < header");
        let mut bad_kind = encode_command(op::OPEN_SESSION, 0, &[1]);
        bad_kind[4] = 9;
        assert!(decode_header(&bad_kind).is_err(), "unknown type");
        // A command where a response is expected, a response with a length
        // that doesn't match its bytes, one with half a parameter, one with
        // six parameters.
        assert!(decode_response(&encode_command(op::OPEN_SESSION, 0, &[1])).is_err());
        let mut resp = encode_response(rc::OK, 1, &[5]);
        resp.push(0);
        assert!(decode_response(&resp).is_err());
        let mut odd = encode_response(rc::OK, 1, &[]);
        odd.extend_from_slice(&[1, 2]);
        odd[0] = 14;
        assert!(decode_response(&odd).is_err());
        assert!(decode_response(&encode_response(rc::OK, 1, &[0; 6])).is_err());
    }

    #[test]
    fn data_headers_announce_the_payload_length() {
        let h = encode_data_header(op::SEND_OBJECT, 3, 1000).unwrap();
        assert_eq!(
            decode_header(&h).unwrap(),
            Header {
                length: 1012,
                kind: ContainerKind::Data,
                code: op::SEND_OBJECT,
                transaction_id: 3
            }
        );
        assert!(encode_data_header(op::SEND_OBJECT, 3, MAX_DATA_PAYLOAD).is_some());
        assert!(
            encode_data_header(op::SEND_OBJECT, 3, MAX_DATA_PAYLOAD + 1).is_none(),
            "the 0xFFFFFFFF marker is never produced"
        );
        assert!(encode_data_header(op::SEND_OBJECT, 3, 5 << 30).is_none());
    }

    #[test]
    fn transaction_ids_follow_the_spec() {
        assert_eq!(OPEN_SESSION_TRANSACTION_ID, 0);
        assert_eq!(
            next_transaction_id(OPEN_SESSION_TRANSACTION_ID),
            1,
            "the first operation in a session is 1"
        );
        assert_eq!(next_transaction_id(41), 42);
        assert_eq!(next_transaction_id(0xFFFF_FFFE), 0xFFFF_FFFF);
        assert_eq!(
            next_transaction_id(0xFFFF_FFFF),
            1,
            "0 is skipped when wrapping"
        );
    }

    #[test]
    fn zero_length_packets_exactly_at_packet_boundaries() {
        for mps in [64usize, 512, 1024] {
            assert!(
                !needs_zero_length_packet(12, mps),
                "a short container ends itself"
            );
            assert!(needs_zero_length_packet(mps as u64, mps));
            assert!(!needs_zero_length_packet(mps as u64 + 1, mps));
            assert!(!needs_zero_length_packet(mps as u64 - 1, mps));
            assert!(
                needs_zero_length_packet(1 << 20, mps),
                "a whole number of chunks"
            );
        }
        // A 1 MiB + 500 byte file: header + payload = 1 048 588 + 500.
        let total = 12 + (1u64 << 20) + 500;
        assert_eq!(
            needs_zero_length_packet(total, 512),
            total.is_multiple_of(512)
        );
        // The 1-in-512 case: 12 + 500 = 512.
        assert!(needs_zero_length_packet(12 + 500, 512));
        assert!(!needs_zero_length_packet(0, 512));
        assert!(!needs_zero_length_packet(512, 0), "no packet size, no ZLP");
    }

    #[test]
    fn strings_round_trip_and_are_bounded() {
        for s in [
            "",
            "documents",
            "Café crème.azw3",
            "日本語の本.epub",
            "emoji 📚.pdf",
        ] {
            let bytes = encode_string(s).unwrap();
            assert_eq!(Reader::new(&bytes).string().unwrap(), s);
        }
        assert_eq!(
            encode_string("").unwrap(),
            vec![0],
            "empty: a zero count and no NUL"
        );
        assert_eq!(
            encode_string("ab").unwrap(),
            vec![3, b'a', 0, b'b', 0, 0, 0],
            "the count includes the NUL"
        );
        assert!(encode_string(&"x".repeat(MAX_STRING_UNITS)).is_ok());
        assert!(
            encode_string(&"x".repeat(MAX_STRING_UNITS + 1)).is_err(),
            "never truncated"
        );
        // A device string without its NUL, and invalid UTF-16.
        assert_eq!(Reader::new(&[2, b'h', 0, b'i', 0]).string().unwrap(), "hi");
        assert_eq!(Reader::new(&[1, 0x00, 0xD8]).string().unwrap(), "\u{FFFD}");
        assert!(Reader::new(&[5, b'a', 0]).string().is_err(), "truncated");
    }

    #[test]
    fn readers_never_read_past_the_end() {
        let mut r = Reader::new(&[1, 2, 3]);
        assert_eq!(r.u16().unwrap(), 0x0201);
        assert!(r.u16().is_err());
        assert!(
            Reader::new(&[0xFF, 0xFF, 0xFF, 0xFF]).u32_array().is_err(),
            "a huge count"
        );
        assert!(
            Reader::new(&[2, 0, 0, 0, 1, 0]).u16_array().is_err(),
            "fewer elements than announced"
        );
        let mut w = Writer::default();
        w.u32_array(&[7, 8]);
        assert_eq!(Reader::new(&w.buf).u32_array().unwrap(), vec![7, 8]);
    }

    #[test]
    fn device_info_round_trip_and_the_serial_is_dropped() {
        let data = encode_device_info(
            &[op::GET_PARTIAL_OBJECT, op::SET_OBJECT_PROP_VALUE],
            "Amazon",
            "Kindle Paperwhite",
            "G000AB1234567890",
        );
        let info = parse_device_info(&data).unwrap();
        assert_eq!(info.manufacturer, "Amazon");
        assert_eq!(info.model, "Kindle Paperwhite");
        assert_eq!(info.vendor_extension_id, 6);
        assert!(info.supports(op::GET_PARTIAL_OBJECT) && !info.supports(op::GET_OBJECT));
        assert!(
            !format!("{info:?}").contains("G000AB"),
            "the serial isn't kept anywhere"
        );

        let leaky = parse_device_info(&encode_device_info(
            &[],
            "Amazon",
            "Kindle G000AB1234567890",
            "G000AB1234567890",
        ))
        .unwrap();
        assert_eq!(leaky.model, "", "a model carrying the serial is dropped");
        assert!(parse_device_info(&data[..20]).is_err(), "truncated");
    }

    #[test]
    fn storage_info_round_trip() {
        let info = StorageInfo {
            storage_type: 3,
            filesystem_type: 2,
            access: ACCESS_READ_WRITE,
            max_capacity: 16 << 30,
            free_space: 12 << 30,
            description: "Internal Storage".into(),
            volume_label: "Kindle".into(),
        };
        assert_eq!(
            parse_storage_info(&encode_storage_info(&info)).unwrap(),
            info
        );
        // Without the optional strings.
        let short = &encode_storage_info(&info)[..26];
        assert_eq!(parse_storage_info(short).unwrap().description, "");
        assert!(parse_storage_info(&[0; 10]).is_err());
    }

    #[test]
    fn object_info_round_trip_through_send_object_info() {
        let sent = encode_object_info(0x0001_0001, 42, "Dune.azw3", 123_456, false).unwrap();
        let parsed = parse_object_info(&sent).unwrap();
        assert_eq!(
            (
                parsed.storage_id,
                parsed.format,
                parsed.compressed_size,
                parsed.parent,
                parsed.filename.as_str()
            ),
            (0x0001_0001, format::UNDEFINED, 123_456, 42, "Dune.azw3")
        );
        assert!(!parsed.is_folder());

        let folder = parse_object_info(
            &encode_object_info(0x0001_0001, NO_PARENT, "documents", 0, true).unwrap(),
        )
        .unwrap();
        assert!(folder.is_folder());
        assert_eq!(folder.association_type, ASSOCIATION_GENERIC_FOLDER);

        let reply = ObjectInfo {
            storage_id: 1,
            format: format::UNDEFINED,
            compressed_size: 9,
            parent: 3,
            association_type: 0,
            filename: "x.pdf".into(),
            modification_date: "20261107T101500".into(),
        };
        assert_eq!(
            parse_object_info(&encode_object_info_reply(&reply)).unwrap(),
            reply
        );
        assert!(
            encode_object_info(1, 0, "big.pdf", 4 << 30, false).is_err(),
            "over 4 GB"
        );
        assert!(
            encode_object_info(1, 0, &"x".repeat(300), 1, false).is_err(),
            "name too long"
        );
    }

    #[test]
    fn class_requests() {
        assert_eq!(
            encode_cancel_request(0x0102_0304),
            [0x01, 0x40, 0x04, 0x03, 0x02, 0x01]
        );
        assert_eq!(
            decode_device_status(&[4, 0, 0x01, 0x20]).unwrap(),
            (rc::OK, vec![])
        );
        assert_eq!(
            decode_device_status(&[12, 0, 0x19, 0x20, 0x81, 0, 0, 0, 0x02, 0, 0, 0]).unwrap(),
            (rc::DEVICE_BUSY, vec![0x81, 0x02])
        );
        assert!(decode_device_status(&[4, 0]).is_err());
    }

    #[test]
    fn response_names() {
        assert_eq!(response_name(rc::DEVICE_BUSY), "device busy");
        assert_eq!(response_name(rc::STORE_FULL), "storage full");
        assert_eq!(response_name(0x2FFF), "error");
    }
}
