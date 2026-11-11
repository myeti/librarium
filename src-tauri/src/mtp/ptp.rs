//! A PTP/MTP session over a pair of USB bulk pipes ([`BulkPipe`]): the
//! transaction logic shared by every raw-USB transport, independent of the
//! USB library. [`super::usb`] provides the pipes (nusb); the tests below
//! drive it against a simulated PTP responder.
//!
//! What this layer guarantees (each point fixes a problem of the
//! first `feat/mtp` prototype):
//! - **Transaction ids per spec**: OpenSession carries 0, the next operation
//!   1, and so on; a response with another id is a protocol error.
//! - **Framing**: bulk OUT transfers are at most [`CHUNK`] bytes, every one
//!   but the last a multiple of the packet size, and a zero-length packet
//!   follows a data phase that ends exactly on a packet boundary
//!   ([`protocol::needs_zero_length_packet`]). IN transfers are multiples of
//!   the packet size (nusb requires it) and capped at [`CHUNK`] too, so no
//!   buffer is ever the size of a file (Linux usbfs caps all transfers in
//!   flight at 16 MB by default).
//! - **Streaming**: files are sent from a reader and received into a writer
//!   chunk by chunk, with progress and cancellation between chunks. Reads
//!   use GetPartialObject (1 MB per transaction, cleanly cancellable) when
//!   the device supports it, else one streamed GetObject.
//! - **Device_Busy** gets a few retries with a growing delay, for operations
//!   whose data is in memory (all but SendObject).
//! - **Health**: a transport error, a malformed reply, or a cancel in the
//!   middle of a data phase *poisons* the session ([`MtpSession::is_healthy`]
//!   turns false); the worker drops it and reconnects instead of sending
//!   more commands into a confused device.

use std::io::{self, Read, Write};
use std::time::Duration;

use super::protocol::{self, op, rc, ContainerKind, Reader, HEADER_LEN};
use super::session::{DeviceDescription, MtpError, MtpSession, Object, Orphan, Progress, Storage};

/// Largest single bulk transfer, in either direction (1 MiB: a multiple of
/// every bulk packet size).
pub(crate) const CHUNK: usize = 1 << 20;

/// Size of the first IN transfer of a reply. Most replies (a response, a
/// small dataset) fit; bigger data phases continue with [`CHUNK`] reads.
const FIRST_READ: usize = 16 * 1024;

/// Largest dataset (a listing, ObjectInfo…) accepted in memory.
const MAX_DATASET: usize = 16 << 20;

/// How long the device may take for each part of a transaction.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Timeouts {
    /// Writing a command container.
    pub command: Duration,
    /// Each chunk of a data phase.
    pub data: Duration,
    /// The start of the reply: a device writing a whole file to its flash
    /// answers SendObject only once it is done.
    pub response: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            command: Duration::from_secs(5),
            data: Duration::from_secs(30),
            response: Duration::from_secs(60),
        }
    }
}

/// The response timeout while opening a session.
const OPEN_TIMEOUT: Duration = Duration::from_secs(10);

/// Delays between retries of an operation the device answered with
/// Device_Busy (about 3.5 s in all).
const BUSY_BACKOFF: [Duration; 5] = [
    Duration::from_millis(100),
    Duration::from_millis(250),
    Duration::from_millis(500),
    Duration::from_secs(1),
    Duration::from_millis(1500),
];

/// The two bulk endpoints of a PTP interface, and its class requests.
pub(crate) trait BulkPipe: Send {
    /// The bulk endpoints' max packet size (64, 512 or 1024 bytes).
    fn max_packet_size(&self) -> usize;

    /// One bulk OUT transfer of exactly `data` (empty: a zero-length packet).
    fn write(&mut self, data: &[u8], timeout: Duration) -> Result<(), MtpError>;

    /// One bulk IN transfer of at most `max_len` bytes — a non-zero multiple
    /// of the packet size — ending early at a short packet. Empty when the
    /// device sent a zero-length packet.
    fn read(&mut self, max_len: usize, timeout: Duration) -> Result<Vec<u8>, MtpError>;

    /// Abort transaction `transaction_id` in the middle of its data phase
    /// (USB SIC Cancel request, then wait until the device is idle).
    fn cancel(&mut self, transaction_id: u32) -> Result<(), MtpError>;

    /// Return the device's PTP responder to its initial state (USB SIC
    /// Device Reset request; closes the session).
    fn reset(&mut self) -> Result<(), MtpError>;
}

/// The data phase a transaction sends, if any.
enum DataOut<'a, 'p> {
    None,
    Bytes(&'a [u8]),
    Stream {
        len: u64,
        source: &'a mut dyn Read,
        progress: Progress<'p>,
    },
}

/// Receives the data phase a transaction reads, chunk by chunk; `Ok(false)`
/// cancels.
type Sink<'a> = &'a mut dyn FnMut(&[u8]) -> Result<bool, MtpError>;

/// See the module docs.
pub(crate) struct PtpSession<P: BulkPipe> {
    pipe: P,
    /// The id of the last transaction started.
    last_tid: u32,
    session_open: bool,
    info: protocol::DeviceInfo,
    description: DeviceDescription,
    poisoned: bool,
    /// Bytes read past the end of a container (a non-compliant device that
    /// sent a reply in the same transfer as the end of its data phase).
    leftover: Vec<u8>,
    orphans: Vec<Orphan>,
    busy_backoff: Vec<Duration>,
    timeouts: Timeouts,
}

impl<P: BulkPipe> PtpSession<P> {
    /// Open a session on `pipe`: OpenSession (transaction 0), then
    /// GetDeviceInfo. A device that says its session is already open (left
    /// over by a crashed app) is used as is.
    pub(crate) fn open(pipe: P) -> Result<Self, MtpError> {
        Self::open_with(pipe, Timeouts::default(), BUSY_BACKOFF.to_vec())
    }

    pub(crate) fn open_with(
        pipe: P,
        timeouts: Timeouts,
        busy_backoff: Vec<Duration>,
    ) -> Result<Self, MtpError> {
        let mut session = Self {
            pipe,
            last_tid: protocol::OPEN_SESSION_TRANSACTION_ID,
            session_open: false,
            info: protocol::DeviceInfo::default(),
            description: DeviceDescription::default(),
            poisoned: false,
            leftover: Vec::new(),
            orphans: Vec::new(),
            busy_backoff,
            // A device that doesn't answer while opening is reported soon,
            // not after the long wait SendObject's response may need.
            timeouts: Timeouts {
                response: timeouts.response.min(OPEN_TIMEOUT),
                ..timeouts
            },
        };
        session.handshake()?;
        session.timeouts = timeouts;
        Ok(session)
    }

    /// OpenSession then GetDeviceInfo (see [`Self::open`]).
    fn handshake(&mut self) -> Result<(), MtpError> {
        let response = self.run(
            op::OPEN_SESSION,
            protocol::OPEN_SESSION_TRANSACTION_ID,
            &[protocol::SESSION_ID],
            DataOut::None,
            None,
        )?;
        match response.code {
            rc::OK | rc::SESSION_ALREADY_OPEN => self.session_open = true,
            // Retried by the worker (reported as "busy" meanwhile).
            rc::DEVICE_BUSY => return Err(MtpError::Busy),
            code => {
                return Err(MtpError::Response {
                    operation: op::OPEN_SESSION,
                    code,
                })
            }
        }
        let data = self.collect(op::GET_DEVICE_INFO, &[])?;
        self.info = protocol::parse_device_info(&data)?;
        self.description = DeviceDescription {
            manufacturer: self.info.manufacturer.trim().to_string(),
            model: self.info.model.trim().to_string(),
        };
        Ok(())
    }

    #[cfg(test)]
    fn pipe(&self) -> &P {
        &self.pipe
    }

    // ── Transactions ─────────────────────────────────────────────────────────

    /// Run one transaction with the next transaction id. A fatal error
    /// poisons the session; a poisoned session runs nothing.
    fn transaction(
        &mut self,
        code: u16,
        params: &[u32],
        out: DataOut,
        sink: Option<Sink>,
    ) -> Result<protocol::Response, MtpError> {
        if self.poisoned {
            return Err(MtpError::Transport(
                "the connection must be reopened after an error".into(),
            ));
        }
        let tid = protocol::next_transaction_id(self.last_tid);
        let result = self.run(code, tid, params, out, sink);
        if matches!(&result, Err(e) if e.is_fatal()) {
            self.poisoned = true;
        }
        result
    }

    /// Command, optional data out, optional data in, response.
    fn run(
        &mut self,
        code: u16,
        tid: u32,
        params: &[u32],
        out: DataOut,
        sink: Option<Sink>,
    ) -> Result<protocol::Response, MtpError> {
        self.last_tid = tid;
        self.pipe.write(
            &protocol::encode_command(code, tid, params),
            self.timeouts.command,
        )?;
        match out {
            DataOut::None => {}
            DataOut::Bytes(bytes) => {
                self.send_data(code, tid, bytes.len() as u64, &mut &bytes[..], &mut |_| {
                    true
                })?
            }
            DataOut::Stream {
                len,
                source,
                progress,
            } => self.send_data(code, tid, len, source, progress)?,
        }
        self.receive(tid, sink)
    }

    /// Send a data container of `len` payload bytes read from `source`, in
    /// [`CHUNK`]-sized transfers (the header travels with the first bytes),
    /// then a zero-length packet if the container ends on a packet boundary.
    fn send_data(
        &mut self,
        code: u16,
        tid: u32,
        len: u64,
        source: &mut dyn Read,
        progress: Progress,
    ) -> Result<(), MtpError> {
        let header = protocol::encode_data_header(code, tid, len).ok_or_else(|| {
            MtpError::Refused("Files of 4 GB or more can't be sent over MTP.".into())
        })?;
        let mut buf = Vec::with_capacity(CHUNK.min(HEADER_LEN + len as usize));
        buf.extend_from_slice(&header);
        let mut read_total = 0u64;
        loop {
            while buf.len() < CHUNK && read_total < len {
                let start = buf.len();
                let room = (CHUNK - start).min((len - read_total) as usize);
                buf.resize(start + room, 0);
                match source.read(&mut buf[start..]) {
                    Ok(0) => {
                        buf.truncate(start);
                        return Err(self.abort(
                            tid,
                            MtpError::Local("The file got shorter while it was being sent.".into()),
                        ));
                    }
                    Ok(n) => {
                        buf.truncate(start + n);
                        read_total += n as u64;
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => buf.truncate(start),
                    Err(e) => {
                        buf.truncate(start);
                        return Err(self.abort(
                            tid,
                            MtpError::Local(format!("Could not read the file being sent: {e}")),
                        ));
                    }
                }
            }
            self.pipe.write(&buf, self.timeouts.data)?;
            buf.clear();
            let finished = read_total == len;
            if !progress(read_total) && !finished {
                return Err(self.abort(tid, MtpError::Cancelled));
            }
            if finished {
                break;
            }
        }
        if protocol::needs_zero_length_packet(HEADER_LEN as u64 + len, self.pipe.max_packet_size())
        {
            self.pipe.write(&[], self.timeouts.data)?;
        }
        Ok(())
    }

    /// Stop a transaction in the middle of its data phase: ask the device
    /// to cancel it, and poison the session (the device's state is unknown;
    /// the worker reconnects). Returns `err`.
    ///
    /// TODO(device-validation): check how a Kindle handles a SIC Cancel in
    /// the middle of SendObject (and that the partial object can then be
    /// deleted after reconnecting).
    fn abort(&mut self, tid: u32, err: MtpError) -> MtpError {
        let _ = self.pipe.cancel(tid);
        self.poisoned = true;
        self.leftover.clear();
        err
    }

    /// One IN transfer for (up to) `want` bytes: the leftover of an earlier
    /// read first, else a transfer of `want` rounded up to the packet size,
    /// capped at [`CHUNK`].
    fn read_some(&mut self, want: usize, timeout: Duration) -> Result<Vec<u8>, MtpError> {
        if !self.leftover.is_empty() {
            return Ok(std::mem::take(&mut self.leftover));
        }
        let mps = self.pipe.max_packet_size().max(1);
        let len = want.clamp(1, CHUNK).div_ceil(mps) * mps;
        self.pipe.read(len, timeout)
    }

    /// At least the header of the next container. Up to two zero-length
    /// packets are skipped: the one ending a data phase on a packet
    /// boundary arrives on its own when the last read was exactly full.
    fn read_head(&mut self, timeout: Duration) -> Result<Vec<u8>, MtpError> {
        let mut buf = Vec::new();
        let mut empty = 0;
        while buf.len() < HEADER_LEN {
            let chunk = self.read_some(FIRST_READ, timeout)?;
            if chunk.is_empty() {
                empty += 1;
                if empty > 2 {
                    return Err(MtpError::Protocol("the device sent no reply".into()));
                }
            }
            buf.extend_from_slice(&chunk);
        }
        Ok(buf)
    }

    /// The reply of transaction `tid`: an optional data phase (into `sink`),
    /// then the response.
    fn receive(
        &mut self,
        tid: u32,
        mut sink: Option<Sink>,
    ) -> Result<protocol::Response, MtpError> {
        let mut buf = self.read_head(self.timeouts.response)?;
        let mut header = protocol::decode_header(&buf)?;
        if header.transaction_id != tid {
            return Err(MtpError::Protocol(format!(
                "reply to transaction {} while waiting for {tid}",
                header.transaction_id
            )));
        }
        if header.kind == ContainerKind::Data {
            if header.length == u32::MAX {
                return Err(MtpError::Unsupported("files of 4 GB or more".into()));
            }
            let payload_len = header.length as usize - HEADER_LEN;
            let mut chunk = buf.split_off(HEADER_LEN);
            let mut received = 0usize;
            loop {
                if chunk.len() > payload_len - received {
                    self.leftover = chunk.split_off(payload_len - received);
                }
                received += chunk.len();
                if let Some(sink) = sink.as_mut() {
                    let keep_going = match sink(&chunk) {
                        Ok(keep_going) => keep_going,
                        Err(e) => return Err(self.abort(tid, e)),
                    };
                    if !keep_going && received < payload_len {
                        return Err(self.abort(tid, MtpError::Cancelled));
                    }
                }
                if received >= payload_len {
                    break;
                }
                chunk = self.read_some(payload_len - received, self.timeouts.data)?;
                if chunk.is_empty() {
                    return Err(MtpError::Protocol("the data phase ended early".into()));
                }
            }
            buf = self.read_head(self.timeouts.response)?;
            header = protocol::decode_header(&buf)?;
        }
        if header.kind != ContainerKind::Response {
            return Err(MtpError::Protocol(format!(
                "expected a response, got a {:?} container",
                header.kind
            )));
        }
        let len = header.length as usize;
        while buf.len() < len {
            let more = self.read_some(len - buf.len(), self.timeouts.response)?;
            if more.is_empty() {
                return Err(MtpError::Protocol("truncated response".into()));
            }
            buf.extend_from_slice(&more);
        }
        if buf.len() > len {
            self.leftover = buf.split_off(len);
        }
        let response = protocol::decode_response(&buf)?;
        if response.transaction_id != tid {
            return Err(MtpError::Protocol(format!(
                "response to transaction {} while waiting for {tid}",
                response.transaction_id
            )));
        }
        Ok(response)
    }

    /// Run `code` with in-memory data (out and in), retrying while the
    /// device is busy; the response parameters and the data read.
    fn call(
        &mut self,
        code: u16,
        params: &[u32],
        data_out: Option<&[u8]>,
    ) -> Result<(Vec<u32>, Vec<u8>), MtpError> {
        let mut attempt = 0;
        loop {
            let mut data = Vec::new();
            let response = {
                let mut sink = |chunk: &[u8]| {
                    if data.len() + chunk.len() > MAX_DATASET {
                        return Err(MtpError::Protocol("reply too large".into()));
                    }
                    data.extend_from_slice(chunk);
                    Ok(true)
                };
                let out = data_out.map_or(DataOut::None, DataOut::Bytes);
                self.transaction(code, params, out, Some(&mut sink))?
            };
            match response.code {
                rc::OK => return Ok((response.params, data)),
                rc::DEVICE_BUSY if attempt < self.busy_backoff.len() => {
                    std::thread::sleep(self.busy_backoff[attempt]);
                    attempt += 1;
                }
                rc::DEVICE_BUSY => return Err(MtpError::Busy),
                other => {
                    return Err(MtpError::Response {
                        operation: code,
                        code: other,
                    })
                }
            }
        }
    }

    /// [`Self::call`] without data out; the data read.
    fn collect(&mut self, code: u16, params: &[u32]) -> Result<Vec<u8>, MtpError> {
        self.call(code, params, None).map(|(_, data)| data)
    }

    // ── Objects ──────────────────────────────────────────────────────────────

    fn object_info(&mut self, handle: u32) -> Result<Object, MtpError> {
        let info = protocol::parse_object_info(&self.collect(op::GET_OBJECT_INFO, &[handle])?)?;
        Ok(Object {
            id: handle.to_string(),
            storage: info.storage_id.to_string(),
            parent: (!is_root_parent(info.parent)).then(|| info.parent.to_string()),
            size: if info.compressed_size == u32::MAX {
                u64::MAX
            } else {
                u64::from(info.compressed_size)
            },
            is_folder: info.is_folder(),
            modified: info.modification_date,
            name: info.filename,
        })
    }

    /// Delete `handle`, or remember it as an orphan if that fails.
    fn delete_or_orphan(&mut self, orphan: Orphan) {
        let deleted = self.is_healthy()
            && parse_id(&orphan.id)
                .and_then(|h| self.call(op::DELETE_OBJECT, &[h, 0], None))
                .is_ok();
        if !deleted {
            self.orphans.push(orphan);
        }
    }
}

/// Whether an ObjectInfo `ParentObject` means "at the root" (0 per spec;
/// some devices say 0xFFFF_FFFF).
fn is_root_parent(parent: u32) -> bool {
    parent == protocol::NO_PARENT || parent == protocol::PARENT_ROOT
}

/// A PTP handle or storage id from its [`ObjectId`](super::session::ObjectId) form.
fn parse_id(id: &str) -> Result<u32, MtpError> {
    id.parse()
        .map_err(|_| MtpError::NotFound(format!("Object {id:?}")))
}

impl<P: BulkPipe> MtpSession for PtpSession<P> {
    fn description(&self) -> &DeviceDescription {
        &self.description
    }

    fn storages(&mut self) -> Result<Vec<Storage>, MtpError> {
        let ids = Reader::new(&self.collect(op::GET_STORAGE_IDS, &[])?).u32_array()?;
        let mut out = Vec::new();
        for id in ids {
            // The low half 0 is a removable slot with nothing in it (PTP §5.5.3).
            if id & 0xFFFF == 0 {
                continue;
            }
            let info = match self.collect(op::GET_STORAGE_INFO, &[id]) {
                Ok(data) => protocol::parse_storage_info(&data)?,
                Err(MtpError::Response {
                    code: rc::STORE_NOT_AVAILABLE | rc::INVALID_STORAGE_ID,
                    ..
                }) => continue,
                Err(e) => return Err(e),
            };
            let known = |v: u64| (v != u64::MAX).then_some(v);
            out.push(Storage {
                id: id.to_string(),
                description: if info.description.is_empty() {
                    info.volume_label
                } else {
                    info.description
                },
                free_bytes: known(info.free_space),
                capacity_bytes: known(info.max_capacity),
                writable: info.access == protocol::ACCESS_READ_WRITE,
            });
        }
        Ok(out)
    }

    /// The handles GetObjectHandles returns for the folder, each looked up
    /// with GetObjectInfo. Objects whose ParentObject says they are
    /// elsewhere are dropped (a device that ignores the folder filter would
    /// otherwise return every object of the storage), unless that drops
    /// them all (a device that reports parents oddly).
    /// TODO(device-validation): check what a Kindle reports as the parent
    /// of root objects, and that it honours the folder filter.
    fn children(&mut self, storage: &str, parent: Option<&str>) -> Result<Vec<Object>, MtpError> {
        let storage_id = parse_id(storage)?;
        let parent_handle = parent.map(parse_id).transpose()?;
        let filter = parent_handle.unwrap_or(protocol::PARENT_ROOT);
        let data = self.collect(
            op::GET_OBJECT_HANDLES,
            &[storage_id, protocol::ALL_FORMATS, filter],
        )?;
        let handles = Reader::new(&data).u32_array()?;
        let mut objects = Vec::with_capacity(handles.len());
        for handle in handles {
            match self.object_info(handle) {
                Ok(object) => objects.push(object),
                // Deleted between the two calls.
                Err(MtpError::Response {
                    code: rc::INVALID_OBJECT_HANDLE,
                    ..
                }) => {}
                Err(e) => return Err(e),
            }
        }
        let wanted = parent.map(str::to_string);
        let belongs: Vec<Object> = objects
            .iter()
            .filter(|o| o.parent == wanted && o.storage == storage)
            .cloned()
            .collect();
        Ok(if belongs.is_empty() { objects } else { belongs })
    }

    fn object(&mut self, id: &str) -> Result<Object, MtpError> {
        let handle = parse_id(id)?;
        self.object_info(handle)
    }

    fn create_folder(
        &mut self,
        storage: &str,
        parent: Option<&str>,
        name: &str,
    ) -> Result<Object, MtpError> {
        let storage_id = parse_id(storage)?;
        let parent_handle = parent.map(parse_id).transpose()?;
        let dataset = protocol::encode_object_info(
            storage_id,
            parent_handle.unwrap_or(protocol::NO_PARENT),
            name,
            0,
            true,
        )
        .map_err(|e| {
            MtpError::Refused(format!(
                "Can't create the folder \"{name}\" on the Kindle ({})",
                e.0
            ))
        })?;
        let (params, _) = self.call(
            op::SEND_OBJECT_INFO,
            &[storage_id, parent_handle.unwrap_or(protocol::PARENT_ROOT)],
            Some(&dataset),
        )?;
        let handle = *params
            .get(2)
            .ok_or_else(|| MtpError::Protocol("SendObjectInfo returned no object handle".into()))?;
        Ok(Object {
            id: handle.to_string(),
            storage: storage.to_string(),
            parent: parent.map(str::to_string),
            name: name.to_string(),
            size: 0,
            is_folder: true,
            modified: String::new(),
        })
    }

    fn send_file(
        &mut self,
        storage: &str,
        parent: Option<&str>,
        name: &str,
        size: u64,
        source: &mut dyn Read,
        progress: Progress,
    ) -> Result<Object, MtpError> {
        let storage_id = parse_id(storage)?;
        let parent_handle = parent.map(parse_id).transpose()?;
        let dataset = protocol::encode_object_info(
            storage_id,
            parent_handle.unwrap_or(protocol::NO_PARENT),
            name,
            size,
            false,
        )
        .map_err(|e| MtpError::Refused(format!("Can't send \"{name}\" to the Kindle ({})", e.0)))?;
        let (params, _) = self.call(
            op::SEND_OBJECT_INFO,
            &[storage_id, parent_handle.unwrap_or(protocol::PARENT_ROOT)],
            Some(&dataset),
        )?;
        let handle = *params
            .get(2)
            .ok_or_else(|| MtpError::Protocol("SendObjectInfo returned no object handle".into()))?;
        let orphan = Orphan {
            id: handle.to_string(),
            storage: storage.to_string(),
            parent: parent.map(str::to_string),
            name: name.to_string(),
        };

        // SendObject must directly follow SendObjectInfo: no retry here
        // (the data comes from a stream that can't be rewound).
        let result = self.transaction(
            op::SEND_OBJECT,
            &[],
            DataOut::Stream {
                len: size,
                source,
                progress,
            },
            None,
        );
        match result {
            Ok(response) if response.code == rc::OK => Ok(Object {
                id: handle.to_string(),
                storage: storage.to_string(),
                parent: parent.map(str::to_string),
                name: name.to_string(),
                size,
                is_folder: false,
                modified: String::new(),
            }),
            Ok(response) => {
                self.delete_or_orphan(orphan);
                Err(if response.code == rc::DEVICE_BUSY {
                    MtpError::Busy
                } else {
                    MtpError::Response {
                        operation: op::SEND_OBJECT,
                        code: response.code,
                    }
                })
            }
            Err(e) => {
                self.delete_or_orphan(orphan);
                Err(e)
            }
        }
    }

    fn read_file(
        &mut self,
        object: &Object,
        sink: &mut dyn Write,
        progress: Progress,
    ) -> Result<u64, MtpError> {
        let handle = parse_id(&object.id)?;
        if object.size == u64::MAX {
            return Err(MtpError::Unsupported("files of 4 GB or more".into()));
        }
        let local = |e: io::Error| {
            MtpError::Local(format!(
                "Could not write the file copied from the Kindle: {e}"
            ))
        };

        // A size of 0 may mean "unknown" (some responders): GetObject sends
        // whatever there is.
        if !self.info.supports(op::GET_PARTIAL_OBJECT) || object.size == 0 {
            // One streamed data phase; a cancel aborts it (and the session).
            let mut written = 0u64;
            let response = {
                let mut chunks = |chunk: &[u8]| {
                    sink.write_all(chunk).map_err(local)?;
                    written += chunk.len() as u64;
                    Ok(progress(written))
                };
                self.transaction(op::GET_OBJECT, &[handle], DataOut::None, Some(&mut chunks))?
            };
            return match response.code {
                rc::OK => Ok(written),
                code => Err(MtpError::Response {
                    operation: op::GET_OBJECT,
                    code,
                }),
            };
        }

        // 1 MB per transaction: a cancel between two is clean.
        let mut offset = 0u64;
        let mut busy = 0;
        while offset < object.size {
            let want = (object.size - offset).min(CHUNK as u64) as u32;
            let mut got = 0u64;
            let response = {
                let mut chunks = |chunk: &[u8]| {
                    sink.write_all(chunk).map_err(local)?;
                    got += chunk.len() as u64;
                    Ok(true)
                };
                self.transaction(
                    op::GET_PARTIAL_OBJECT,
                    &[handle, offset as u32, want],
                    DataOut::None,
                    Some(&mut chunks),
                )?
            };
            match response.code {
                rc::OK => {}
                rc::DEVICE_BUSY if got == 0 && busy < self.busy_backoff.len() => {
                    std::thread::sleep(self.busy_backoff[busy]);
                    busy += 1;
                    continue;
                }
                code => {
                    return Err(MtpError::Response {
                        operation: op::GET_PARTIAL_OBJECT,
                        code,
                    })
                }
            }
            if got == 0 {
                return Err(MtpError::Protocol(
                    "the device sent no data for a part of the file".into(),
                ));
            }
            offset += got;
            if !progress(offset) && offset < object.size {
                return Err(MtpError::Cancelled);
            }
        }
        Ok(offset)
    }

    fn delete(&mut self, id: &str) -> Result<(), MtpError> {
        let handle = parse_id(id)?;
        self.call(op::DELETE_OBJECT, &[handle, 0], None).map(|_| ())
    }

    fn can_rename(&self) -> bool {
        self.info.supports(op::SET_OBJECT_PROP_VALUE)
    }

    fn rename(&mut self, id: &str, new_name: &str) -> Result<(), MtpError> {
        let handle = parse_id(id)?;
        let data = protocol::encode_string(new_name).map_err(|e| {
            MtpError::Refused(format!("Can't name a file \"{new_name}\" ({})", e.0))
        })?;
        self.call(
            op::SET_OBJECT_PROP_VALUE,
            &[handle, u32::from(protocol::prop::OBJECT_FILE_NAME)],
            Some(&data),
        )
        .map(|_| ())
    }

    fn is_healthy(&self) -> bool {
        !self.poisoned
    }

    fn take_orphans(&mut self) -> Vec<Orphan> {
        std::mem::take(&mut self.orphans)
    }
}

impl<P: BulkPipe> Drop for PtpSession<P> {
    /// Close the session politely when the device is still there.
    fn drop(&mut self) {
        if self.session_open && !self.poisoned {
            let _ = self.transaction(op::CLOSE_SESSION, &[], DataOut::None, None);
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
pub(crate) mod tests {
    //! A simulated PTP responder behind a [`BulkPipe`], strict where real
    //! devices are: transaction ids, the zero-length packet after a data
    //! phase on a packet boundary, IN transfer sizes, sessions.

    use super::*;
    use crate::mtp::protocol::{format, ObjectInfo, StorageInfo};
    use std::collections::{BTreeMap, VecDeque};
    use std::sync::{Arc, Mutex};

    pub(crate) const STORAGE: u32 = 0x0001_0001;

    #[derive(Debug, Clone)]
    pub(crate) struct FakeObject {
        pub parent: u32,
        pub name: String,
        pub data: Vec<u8>,
        pub folder: bool,
    }

    /// A command waiting for its data phase.
    struct Pending {
        code: u16,
        tid: u32,
        params: Vec<u32>,
    }

    #[derive(Default)]
    pub(crate) struct Responder {
        pub mps: usize,
        inbox: Vec<u8>,
        /// Device → host: each element ends at a short packet; an empty one
        /// is a zero-length packet.
        outbox: VecDeque<Vec<u8>>,
        pending: Option<Pending>,
        /// A complete data container ending on a packet boundary: the device
        /// waits for the host's zero-length packet before acting on it.
        awaiting_zlp: Option<Vec<u8>>,
        /// A data-phase write that wasn't a multiple of the packet size
        /// before the end of the container (a short packet mid-phase).
        short_mid_phase: bool,
        session: bool,
        last_tid: Option<u32>,
        pub objects: BTreeMap<u32, FakeObject>,
        next_handle: u32,
        send_target: Option<u32>,
        pub operations: Vec<u16>,
        /// Answer the next N commands with Device_Busy.
        pub busy: u32,
        /// Send the response in the same transfer as the end of the data
        /// phase (a non-compliant device).
        pub merge_reply: bool,
        /// Reply with the wrong transaction id once.
        pub wrong_tid_once: bool,
        pub unplugged: bool,
        /// Every OUT transfer's length, and every op code seen.
        pub writes: Vec<usize>,
        pub ops_seen: Vec<(u16, u32)>,
        pub cancels: Vec<u32>,
        pub resets: usize,
        pub closed: bool,
    }

    impl Responder {
        pub(crate) fn new(mps: usize, operations: &[u16]) -> Self {
            Self {
                mps,
                next_handle: 100,
                operations: operations.to_vec(),
                ..Default::default()
            }
        }

        pub(crate) fn add(&mut self, parent: u32, name: &str, data: Option<&[u8]>) -> u32 {
            let handle = self.next_handle;
            self.next_handle += 1;
            self.objects.insert(
                handle,
                FakeObject {
                    parent,
                    name: name.into(),
                    data: data.map(<[u8]>::to_vec).unwrap_or_default(),
                    folder: data.is_none(),
                },
            );
            handle
        }

        fn queue(&mut self, container: Vec<u8>) {
            let boundary = container.len().is_multiple_of(self.mps);
            self.outbox.push_back(container);
            if boundary {
                self.outbox.push_back(Vec::new());
            }
        }

        fn reply(&mut self, code: u16, tid: u32, data: Option<Vec<u8>>, params: &[u32]) {
            let tid_out = if std::mem::take(&mut self.wrong_tid_once) {
                tid.wrapping_add(7)
            } else {
                tid
            };
            let response = protocol::encode_response(code, tid_out, params);
            match data {
                Some(payload) => {
                    let mut container = protocol::encode_data_header(
                        self.pending_code(),
                        tid_out,
                        payload.len() as u64,
                    )
                    .unwrap()
                    .to_vec();
                    container.extend_from_slice(&payload);
                    if self.merge_reply {
                        container.extend_from_slice(&response);
                        self.outbox.push_back(container);
                    } else {
                        self.queue(container);
                        self.queue(response);
                    }
                }
                None => self.queue(response),
            }
        }

        fn pending_code(&self) -> u16 {
            self.ops_seen.last().map(|(c, _)| *c).unwrap_or(0)
        }

        fn write(&mut self, data: &[u8]) -> Result<(), MtpError> {
            if self.unplugged {
                return Err(MtpError::Disconnected);
            }
            self.writes.push(data.len());
            if data.is_empty() {
                if let Some(container) = self.awaiting_zlp.take() {
                    self.handle_container(container);
                }
                return Ok(());
            }
            self.inbox.extend_from_slice(data);
            while let Ok(header) = protocol::decode_header(&self.inbox) {
                let len = header.length as usize;
                if self.inbox.len() < len {
                    if !data.len().is_multiple_of(self.mps) {
                        self.short_mid_phase = true;
                    }
                    break;
                }
                let container: Vec<u8> = self.inbox.drain(..len).collect();
                if header.kind == protocol::ContainerKind::Data && len.is_multiple_of(self.mps) {
                    self.awaiting_zlp = Some(container);
                } else {
                    self.handle_container(container);
                }
            }
            Ok(())
        }

        fn read(&mut self, max_len: usize) -> Result<Vec<u8>, MtpError> {
            if self.unplugged {
                return Err(MtpError::Disconnected);
            }
            if max_len == 0 || !max_len.is_multiple_of(self.mps) {
                return Err(MtpError::Transport(format!(
                    "IN transfer of {max_len} bytes isn't a multiple of {}",
                    self.mps
                )));
            }
            if self.awaiting_zlp.is_some() {
                return Err(MtpError::Transport(
                    "timed out (the device is waiting for the end of the data phase)".into(),
                ));
            }
            let Some(front) = self.outbox.front_mut() else {
                return Err(MtpError::Transport("timed out (nothing to read)".into()));
            };
            if front.is_empty() {
                self.outbox.pop_front();
                return Ok(Vec::new());
            }
            let n = max_len.min(front.len());
            let out: Vec<u8> = front.drain(..n).collect();
            if front.is_empty() {
                self.outbox.pop_front();
                // A transfer with room left ends at the zero-length packet
                // and consumes it; a transfer that was exactly filled doesn't.
                if n < max_len && self.outbox.front().is_some_and(Vec::is_empty) {
                    self.outbox.pop_front();
                }
            }
            Ok(out)
        }

        fn handle_container(&mut self, container: Vec<u8>) {
            let header = protocol::decode_header(&container).unwrap();
            let payload = &container[HEADER_LEN..];
            match header.kind {
                protocol::ContainerKind::Command => {
                    let params: Vec<u32> = payload
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|c| u32::from_le_bytes(*c))
                        .collect();
                    self.ops_seen.push((header.code, header.transaction_id));
                    let expected = match self.last_tid {
                        _ if header.code == op::OPEN_SESSION => {
                            protocol::OPEN_SESSION_TRANSACTION_ID
                        }
                        Some(last) => protocol::next_transaction_id(last),
                        None => header.transaction_id,
                    };
                    self.last_tid = Some(header.transaction_id);
                    let needs_data = matches!(
                        header.code,
                        op::SEND_OBJECT_INFO | op::SEND_OBJECT | op::SET_OBJECT_PROP_VALUE
                    );
                    if header.transaction_id != expected {
                        if !needs_data {
                            self.reply(
                                rc::INVALID_TRANSACTION_ID,
                                header.transaction_id,
                                None,
                                &[],
                            );
                        }
                        return;
                    }
                    if needs_data {
                        self.pending = Some(Pending {
                            code: header.code,
                            tid: header.transaction_id,
                            params,
                        });
                    } else {
                        self.command(header.code, header.transaction_id, &params, None);
                    }
                }
                protocol::ContainerKind::Data => {
                    let Some(p) = self.pending.take() else { return };
                    assert_eq!(
                        p.tid, header.transaction_id,
                        "data phase of another transaction"
                    );
                    if std::mem::take(&mut self.short_mid_phase) {
                        self.reply(rc::INCOMPLETE_TRANSFER, p.tid, None, &[]);
                        return;
                    }
                    self.command(p.code, p.tid, &p.params, Some(payload));
                }
                _ => {}
            }
        }

        fn command(&mut self, code: u16, tid: u32, params: &[u32], data: Option<&[u8]>) {
            if self.busy > 0 && code != op::OPEN_SESSION {
                self.busy -= 1;
                return self.reply(rc::DEVICE_BUSY, tid, None, &[]);
            }
            if !self.session && !matches!(code, op::OPEN_SESSION | op::GET_DEVICE_INFO) {
                return self.reply(rc::SESSION_NOT_OPEN, tid, None, &[]);
            }
            let param = |i: usize| params.get(i).copied().unwrap_or(0);
            match code {
                op::OPEN_SESSION => {
                    let already = self.session;
                    self.session = true;
                    self.reply(
                        if already {
                            rc::SESSION_ALREADY_OPEN
                        } else {
                            rc::OK
                        },
                        tid,
                        None,
                        &[],
                    );
                }
                op::CLOSE_SESSION => {
                    self.session = false;
                    self.closed = true;
                    self.reply(rc::OK, tid, None, &[]);
                }
                op::GET_DEVICE_INFO => {
                    let info = protocol::encode_device_info(
                        &self.operations.clone(),
                        "Amazon",
                        "Kindle Paperwhite",
                        "G000FAKESERIAL",
                    );
                    self.reply(rc::OK, tid, Some(info), &[]);
                }
                op::GET_STORAGE_IDS => {
                    let mut w = protocol::Writer::default();
                    w.u32_array(&[STORAGE, 0x0002_0000]);
                    self.reply(rc::OK, tid, Some(w.buf), &[]);
                }
                op::GET_STORAGE_INFO => {
                    let info = StorageInfo {
                        storage_type: 3,
                        filesystem_type: 2,
                        access: 0,
                        max_capacity: 16 << 30,
                        free_space: 9 << 30,
                        description: "Internal Storage".into(),
                        volume_label: String::new(),
                    };
                    self.reply(rc::OK, tid, Some(protocol::encode_storage_info(&info)), &[]);
                }
                op::GET_OBJECT_HANDLES => {
                    let parent = if param(2) == protocol::PARENT_ROOT {
                        0
                    } else {
                        param(2)
                    };
                    let handles: Vec<u32> = self
                        .objects
                        .iter()
                        .filter(|(_, o)| o.parent == parent)
                        .map(|(h, _)| *h)
                        .collect();
                    let mut w = protocol::Writer::default();
                    w.u32_array(&handles);
                    self.reply(rc::OK, tid, Some(w.buf), &[]);
                }
                op::GET_OBJECT_INFO => match self.objects.get(&param(0)).cloned() {
                    Some(o) => {
                        let info = ObjectInfo {
                            storage_id: STORAGE,
                            format: if o.folder {
                                format::ASSOCIATION
                            } else {
                                format::UNDEFINED
                            },
                            compressed_size: o.data.len() as u32,
                            parent: o.parent,
                            association_type: if o.folder { 1 } else { 0 },
                            filename: o.name,
                            modification_date: "20261107T101500".into(),
                        };
                        self.reply(
                            rc::OK,
                            tid,
                            Some(protocol::encode_object_info_reply(&info)),
                            &[],
                        );
                    }
                    None => self.reply(rc::INVALID_OBJECT_HANDLE, tid, None, &[]),
                },
                op::GET_OBJECT => match self.objects.get(&param(0)).map(|o| o.data.clone()) {
                    Some(bytes) => self.reply(rc::OK, tid, Some(bytes), &[]),
                    None => self.reply(rc::INVALID_OBJECT_HANDLE, tid, None, &[]),
                },
                op::GET_PARTIAL_OBJECT => match self.objects.get(&param(0)).map(|o| o.data.clone())
                {
                    Some(bytes) => {
                        let start = (param(1) as usize).min(bytes.len());
                        let end = (start + param(2) as usize).min(bytes.len());
                        let part = bytes[start..end].to_vec();
                        let n = part.len() as u32;
                        self.reply(rc::OK, tid, Some(part), &[n]);
                    }
                    None => self.reply(rc::INVALID_OBJECT_HANDLE, tid, None, &[]),
                },
                op::DELETE_OBJECT => {
                    let has_children = self.objects.values().any(|o| o.parent == param(0));
                    match self.objects.contains_key(&param(0)) {
                        true if has_children => self.reply(rc::PARTIAL_DELETION, tid, None, &[]),
                        true => {
                            self.objects.remove(&param(0));
                            self.reply(rc::OK, tid, None, &[]);
                        }
                        false => self.reply(rc::INVALID_OBJECT_HANDLE, tid, None, &[]),
                    }
                }
                op::SEND_OBJECT_INFO => {
                    let info = protocol::parse_object_info(data.unwrap()).unwrap();
                    let parent = if param(1) == protocol::PARENT_ROOT {
                        0
                    } else {
                        param(1)
                    };
                    let handle = self.add(
                        parent,
                        &info.filename,
                        if info.is_folder() { None } else { Some(&[]) },
                    );
                    if !info.is_folder() {
                        self.send_target = Some(handle);
                    }
                    self.reply(rc::OK, tid, None, &[param(0), parent, handle]);
                }
                op::SEND_OBJECT => match self.send_target.take() {
                    Some(handle) => {
                        self.objects.get_mut(&handle).unwrap().data = data.unwrap().to_vec();
                        self.reply(rc::OK, tid, None, &[]);
                    }
                    None => self.reply(rc::GENERAL_ERROR, tid, None, &[]),
                },
                op::SET_OBJECT_PROP_VALUE => {
                    let name = Reader::new(data.unwrap()).string().unwrap();
                    match self.objects.get_mut(&param(0)) {
                        Some(o) if param(1) == u32::from(protocol::prop::OBJECT_FILE_NAME) => {
                            o.name = name;
                            self.reply(rc::OK, tid, None, &[]);
                        }
                        _ => self.reply(rc::INVALID_OBJECT_HANDLE, tid, None, &[]),
                    }
                }
                _ => self.reply(rc::OPERATION_NOT_SUPPORTED, tid, None, &[]),
            }
        }
    }

    /// A [`BulkPipe`] onto a shared [`Responder`].
    #[derive(Clone)]
    pub(crate) struct FakePipe(pub Arc<Mutex<Responder>>);

    impl FakePipe {
        pub(crate) fn device(&self) -> std::sync::MutexGuard<'_, Responder> {
            self.0.lock().unwrap()
        }
    }

    impl BulkPipe for FakePipe {
        fn max_packet_size(&self) -> usize {
            self.device().mps
        }
        fn write(&mut self, data: &[u8], _timeout: Duration) -> Result<(), MtpError> {
            assert!(data.len() <= CHUNK, "a bulk transfer larger than CHUNK");
            self.device().write(data)
        }
        fn read(&mut self, max_len: usize, _timeout: Duration) -> Result<Vec<u8>, MtpError> {
            assert!(max_len <= CHUNK, "a bulk transfer larger than CHUNK");
            self.device().read(max_len)
        }
        fn cancel(&mut self, transaction_id: u32) -> Result<(), MtpError> {
            let mut d = self.device();
            d.cancels.push(transaction_id);
            d.inbox.clear();
            d.pending = None;
            d.awaiting_zlp = None;
            d.outbox.clear();
            Ok(())
        }
        fn reset(&mut self) -> Result<(), MtpError> {
            let mut d = self.device();
            d.resets += 1;
            d.session = false;
            Ok(())
        }
    }

    const ALL_OPS: &[u16] = &[op::GET_PARTIAL_OBJECT, op::SET_OBJECT_PROP_VALUE];

    pub(crate) fn device(mps: usize, ops: &[u16]) -> FakePipe {
        FakePipe(Arc::new(Mutex::new(Responder::new(mps, ops))))
    }

    fn open(pipe: &FakePipe) -> PtpSession<FakePipe> {
        PtpSession::open_with(pipe.clone(), Timeouts::default(), vec![Duration::ZERO; 3]).unwrap()
    }

    fn send(
        session: &mut PtpSession<FakePipe>,
        parent: Option<&str>,
        name: &str,
        data: &[u8],
    ) -> Result<Object, MtpError> {
        let mut reported = Vec::new();
        let result = session.send_file(
            &STORAGE.to_string(),
            parent,
            name,
            data.len() as u64,
            &mut &data[..],
            &mut |n| {
                reported.push(n);
                true
            },
        );
        if result.is_ok() {
            assert_eq!(
                reported.last().copied(),
                Some(data.len() as u64),
                "progress reaches the size"
            );
            assert!(reported.windows(2).all(|w| w[0] <= w[1]));
        }
        result
    }

    fn read(session: &mut PtpSession<FakePipe>, object: &Object) -> Vec<u8> {
        let mut out = Vec::new();
        let n = session.read_file(object, &mut out, &mut |_| true).unwrap();
        assert_eq!(n as usize, out.len());
        out
    }

    #[test]
    fn a_session_opens_with_transaction_zero_then_counts_from_one() {
        let pipe = device(512, ALL_OPS);
        let session = open(&pipe);
        assert_eq!(
            pipe.device().ops_seen,
            vec![(op::OPEN_SESSION, 0), (op::GET_DEVICE_INFO, 1)]
        );
        assert_eq!(
            session.description(),
            &DeviceDescription {
                manufacturer: "Amazon".into(),
                model: "Kindle Paperwhite".into()
            }
        );
        assert!(session.can_rename());
        drop(session);
        assert!(pipe.device().closed, "the session is closed when dropped");
    }

    #[test]
    fn a_session_left_open_by_another_app_is_reused() {
        let pipe = device(512, ALL_OPS);
        pipe.device().session = true;
        let mut session = open(&pipe);
        assert!(session.storages().is_ok());
    }

    #[test]
    fn storages_skip_empty_slots_and_report_space() {
        let pipe = device(512, ALL_OPS);
        let mut session = open(&pipe);
        assert_eq!(
            session.storages().unwrap(),
            vec![Storage {
                id: STORAGE.to_string(),
                description: "Internal Storage".into(),
                free_bytes: Some(9 << 30),
                capacity_bytes: Some(16 << 30),
                writable: true,
            }]
        );
    }

    #[test]
    fn folders_are_listed_and_created() {
        let pipe = device(512, ALL_OPS);
        let docs = pipe.device().add(0, "documents", None);
        pipe.device().add(docs, "Dune.azw3", Some(b"dune"));
        let mut session = open(&pipe);
        let root = session.children(&STORAGE.to_string(), None).unwrap();
        assert_eq!(
            root.iter()
                .map(|o| (o.name.as_str(), o.is_folder, o.parent.clone()))
                .collect::<Vec<_>>(),
            vec![("documents", true, None)]
        );
        let books = session
            .children(&STORAGE.to_string(), Some(&docs.to_string()))
            .unwrap();
        assert_eq!(
            (
                books[0].name.as_str(),
                books[0].size,
                books[0].modified.as_str()
            ),
            ("Dune.azw3", 4, "20261107T101500")
        );

        let sf = session
            .create_folder(&STORAGE.to_string(), Some(&docs.to_string()), "SF")
            .unwrap();
        assert!(sf.is_folder);
        assert_eq!(
            session.object(&sf.id).unwrap().parent,
            Some(docs.to_string())
        );
    }

    #[test]
    fn sending_a_file_that_ends_on_a_packet_boundary_sends_a_zero_length_packet() {
        for mps in [64usize, 512, 1024] {
            let pipe = device(mps, ALL_OPS);
            let mut session = open(&pipe);
            // Header (12) + payload = exactly 4 packets: the responder waits
            // for the ZLP and times out without it (the old bug).
            let data = vec![7u8; 4 * mps - HEADER_LEN];
            let sent = send(&mut session, None, "Boundary.azw3", &data).unwrap();
            assert_eq!(
                pipe.device().objects[&sent.id.parse::<u32>().unwrap()].data,
                data
            );
            assert!(
                pipe.device().writes.contains(&0),
                "a zero-length packet was written"
            );
            // One byte more: no ZLP needed, none sent.
            pipe.device().writes.clear();
            send(
                &mut session,
                None,
                "Other.azw3",
                &vec![1u8; 4 * mps - HEADER_LEN + 1],
            )
            .unwrap();
            assert!(!pipe.device().writes.contains(&0));
        }
    }

    #[test]
    fn large_files_are_streamed_in_bounded_chunks() {
        let pipe = device(512, ALL_OPS);
        let mut session = open(&pipe);
        let data: Vec<u8> = (0..(3 * CHUNK + 5)).map(|i| (i % 251) as u8).collect();
        let sent = send(&mut session, None, "Big.pdf", &data).unwrap();
        let writes = pipe.device().writes.clone();
        // command, dataset, command, then the data phase in CHUNK writes.
        let phase: Vec<usize> = writes.iter().copied().skip_while(|&w| w != CHUNK).collect();
        assert_eq!(
            phase,
            vec![CHUNK, CHUNK, CHUNK, HEADER_LEN + 5],
            "{writes:?}"
        );
        assert_eq!(read(&mut session, &sent), data, "and read back in parts");
        let partial_reads = pipe
            .device()
            .ops_seen
            .iter()
            .filter(|(c, _)| *c == op::GET_PARTIAL_OBJECT)
            .count();
        assert_eq!(partial_reads, 4, "GetPartialObject, 1 MB at a time");
    }

    #[test]
    fn devices_without_partial_reads_stream_one_get_object() {
        for mps in [64usize, 512] {
            let pipe = device(mps, &[]);
            // 12 + payload on a packet boundary: the ZLP after the data phase
            // must be skipped before the response.
            let data = vec![3u8; 2 * CHUNK - HEADER_LEN];
            let h = pipe.device().add(0, "Book.epub", Some(&data));
            let mut session = open(&pipe);
            let object = session.object(&h.to_string()).unwrap();
            let mut seen = Vec::new();
            let mut out = Vec::new();
            session
                .read_file(&object, &mut out, &mut |n| {
                    seen.push(n);
                    true
                })
                .unwrap();
            assert_eq!(out, data);
            assert!(seen.len() > 1, "progress during the single data phase");
            assert!(
                session.storages().is_ok(),
                "the session is in sync afterwards"
            );
        }
    }

    #[test]
    fn an_object_reported_empty_is_read_whole() {
        let pipe = device(512, ALL_OPS);
        let h = pipe.device().add(0, "Odd.pdf", Some(b"content"));
        let mut session = open(&pipe);
        let mut object = session.object(&h.to_string()).unwrap();
        object.size = 0;
        assert_eq!(read(&mut session, &object), b"content");
    }

    #[test]
    fn a_busy_device_is_reported_busy_when_opening() {
        let pipe = device(512, ALL_OPS);
        // (The responder never answers OpenSession "busy"; GetDeviceInfo,
        // the next step of opening, stays busy beyond the retries.)
        pipe.device().busy = 10;
        assert_eq!(
            PtpSession::open_with(pipe.clone(), Timeouts::default(), vec![Duration::ZERO; 2]).err(),
            Some(MtpError::Busy)
        );
    }

    #[test]
    fn a_reply_merged_into_the_data_transfer_is_still_read() {
        let pipe = device(512, ALL_OPS);
        let h = pipe.device().add(0, "x.pdf", Some(&[9u8; 500]));
        let mut session = open(&pipe);
        pipe.device().merge_reply = true;
        let object = session.object(&h.to_string()).unwrap();
        assert_eq!(object.name, "x.pdf");
        assert_eq!(read(&mut session, &object), vec![9u8; 500]);
        pipe.device().merge_reply = false;
        assert!(session.storages().is_ok());
    }

    #[test]
    fn busy_devices_are_retried_then_reported() {
        let pipe = device(512, ALL_OPS);
        let mut session = open(&pipe);
        pipe.device().busy = 2;
        assert!(session.storages().is_ok(), "two busy answers, then OK");
        pipe.device().busy = 10;
        assert_eq!(session.storages().unwrap_err(), MtpError::Busy);
        assert!(session.is_healthy(), "busy isn't fatal");
    }

    #[test]
    fn transaction_ids_wrap_without_zero() {
        let pipe = device(512, ALL_OPS);
        let mut session = open(&pipe);
        session.last_tid = 0xFFFF_FFFD;
        pipe.device().last_tid = Some(0xFFFF_FFFD);
        for _ in 0..3 {
            session.storages().unwrap();
        }
        let tids: Vec<u32> = pipe
            .device()
            .ops_seen
            .iter()
            .map(|(_, t)| *t)
            .skip(2)
            .collect();
        assert_eq!(&tids[..3], &[0xFFFF_FFFE, 0xFFFF_FFFF, 1]);
    }

    #[test]
    fn cancelling_a_send_aborts_the_phase_and_leaves_an_orphan() {
        let pipe = device(512, ALL_OPS);
        let mut session = open(&pipe);
        let data = vec![5u8; 3 * CHUNK];
        let mut calls = 0;
        let err = session
            .send_file(
                &STORAGE.to_string(),
                None,
                "Cancelled.azw3",
                data.len() as u64,
                &mut &data[..],
                &mut |_| {
                    calls += 1;
                    calls < 2
                },
            )
            .unwrap_err();
        assert_eq!(err, MtpError::Cancelled);
        assert_eq!(pipe.device().cancels.len(), 1, "a SIC cancel was sent");
        assert!(!session.is_healthy(), "the session must be reopened");
        let orphans = session.take_orphans();
        assert_eq!(
            orphans.iter().map(|o| o.name.as_str()).collect::<Vec<_>>(),
            vec!["Cancelled.azw3"]
        );
        assert!(
            session.storages().is_err(),
            "nothing more is sent on a poisoned session"
        );
    }

    #[test]
    fn cancelling_a_partial_read_is_clean() {
        let pipe = device(512, ALL_OPS);
        let h = pipe.device().add(0, "Big.pdf", Some(&vec![1u8; 3 * CHUNK]));
        let mut session = open(&pipe);
        let object = session.object(&h.to_string()).unwrap();
        let mut out = Vec::new();
        assert_eq!(
            session
                .read_file(&object, &mut out, &mut |_| false)
                .unwrap_err(),
            MtpError::Cancelled
        );
        assert_eq!(out.len(), CHUNK, "stopped after the first part");
        assert!(
            session.is_healthy(),
            "between two transactions: nothing to abort"
        );
        assert!(pipe.device().cancels.is_empty());
    }

    #[test]
    fn a_source_shorter_than_announced_aborts() {
        let pipe = device(512, ALL_OPS);
        let mut session = open(&pipe);
        let data = [1u8; 100];
        let err = session
            .send_file(
                &STORAGE.to_string(),
                None,
                "Short.pdf",
                1000,
                &mut &data[..],
                &mut |_| true,
            )
            .unwrap_err();
        assert!(matches!(err, MtpError::Local(_)), "{err:?}");
        assert_eq!(session.take_orphans().len(), 1);
    }

    #[test]
    fn transport_errors_poison_the_session() {
        let pipe = device(512, ALL_OPS);
        let mut session = open(&pipe);
        pipe.device().unplugged = true;
        assert_eq!(session.storages().unwrap_err(), MtpError::Disconnected);
        assert!(!session.is_healthy());
        pipe.device().unplugged = false;
        assert!(session.storages().is_err(), "until reopened");
    }

    #[test]
    fn a_reply_to_another_transaction_is_a_protocol_error() {
        let pipe = device(512, ALL_OPS);
        let mut session = open(&pipe);
        pipe.device().wrong_tid_once = true;
        assert!(matches!(
            session.storages().unwrap_err(),
            MtpError::Protocol(_)
        ));
        assert!(!session.is_healthy());
    }

    #[test]
    fn rename_and_delete() {
        let pipe = device(512, ALL_OPS);
        let mut session = open(&pipe);
        let sent = send(&mut session, None, ".librarium-tmp.part", b"book").unwrap();
        session.rename(&sent.id, "Dune.azw3").unwrap();
        assert_eq!(session.object(&sent.id).unwrap().name, "Dune.azw3");
        session.delete(&sent.id).unwrap();
        assert!(matches!(
            session.object(&sent.id).unwrap_err(),
            MtpError::Response {
                code: rc::INVALID_OBJECT_HANDLE,
                ..
            }
        ));
        assert!(session.is_healthy(), "an error response isn't fatal");

        let plain = device(512, &[]);
        assert!(
            !open(&plain).can_rename(),
            "SetObjectPropValue not advertised"
        );
    }

    #[test]
    fn the_pipe_is_reachable_for_tests() {
        let pipe = device(64, &[]);
        let session = open(&pipe);
        assert_eq!(session.pipe().max_packet_size(), 64);
    }
}
