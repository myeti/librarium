//! An in-memory MTP device for tests ([`FakeDevice`]), the session onto it
//! ([`FakeSession`]) and a backend that "enumerates" fake devices
//! ([`FakeBackend`]). Tests share the device with the code under test and
//! change it mid-way: unplug it, make the next claim fail, make it busy,
//! make a send fail half-way, turn renaming off.
//!
//! It behaves like the PTP transport where that matters: a send creates
//! the object before its data arrives (so a failed send leaves a partial
//! object, reported as an orphan once the session is unusable), and a
//! transport failure makes the session unhealthy.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex, MutexGuard};

use super::session::{
    Backend, Candidate, ClaimKind, DeviceDescription, MtpError, MtpSession, Object, Orphan,
    Progress, Storage,
};

/// The storage id of a fake device's only storage.
pub(crate) const STORAGE: &str = "65537";

#[derive(Debug, Clone)]
pub(crate) struct Entry {
    pub parent: Option<u64>,
    pub name: String,
    pub data: Vec<u8>,
    pub folder: bool,
    /// Bumped on every write: the object's "modification date".
    pub version: u32,
}

#[derive(Debug)]
pub(crate) struct DeviceState {
    pub entries: BTreeMap<u64, Entry>,
    next_id: u64,
    pub capacity: u64,
    pub plugged: bool,
    /// Every `open` fails with this (until cleared).
    pub claim_error: Option<MtpError>,
    /// The next N operations answer "busy".
    pub busy: u32,
    /// The next send fails after this many bytes, as if the cable was pulled.
    pub fail_send_after: Option<u64>,
    /// A pause after each 64 KB of a send (a slow device, for cancel tests).
    pub chunk_delay: std::time::Duration,
    pub can_rename: bool,
    /// Every session opened, and every mutating operation, in order.
    pub opens: usize,
    /// Folder listings served (to check caching).
    pub listings: usize,
    pub log: Vec<String>,
    pub model: String,
}

impl DeviceState {
    fn used(&self) -> u64 {
        self.entries.values().map(|e| e.data.len() as u64).sum()
    }

    fn object(&self, id: u64) -> Option<Object> {
        let e = self.entries.get(&id)?;
        Some(Object {
            id: id.to_string(),
            storage: STORAGE.to_string(),
            parent: e.parent.map(|p| p.to_string()),
            name: e.name.clone(),
            size: e.data.len() as u64,
            is_folder: e.folder,
            modified: format!("v{}", e.version),
        })
    }

    fn insert(&mut self, parent: Option<u64>, name: &str, data: Option<Vec<u8>>) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let folder = data.is_none();
        self.entries.insert(
            id,
            Entry {
                parent,
                name: name.to_string(),
                data: data.unwrap_or_default(),
                folder,
                version: 1,
            },
        );
        id
    }
}

/// A shared handle on a fake device.
#[derive(Clone)]
pub(crate) struct FakeDevice(pub Arc<Mutex<DeviceState>>);

impl FakeDevice {
    /// An empty device (not even `documents/`).
    pub(crate) fn blank() -> Self {
        FakeDevice(Arc::new(Mutex::new(DeviceState {
            entries: BTreeMap::new(),
            next_id: 100,
            capacity: 8 << 30,
            plugged: true,
            claim_error: None,
            busy: 0,
            fail_send_after: None,
            chunk_delay: std::time::Duration::ZERO,
            can_rename: true,
            opens: 0,
            listings: 0,
            log: Vec::new(),
            model: "Kindle Paperwhite".into(),
        })))
    }

    /// A device with a `documents/` folder, like a Kindle.
    pub(crate) fn kindle() -> Self {
        let device = Self::blank();
        device.mkdir("documents");
        device
    }

    pub(crate) fn state(&self) -> MutexGuard<'_, DeviceState> {
        self.0.lock().unwrap()
    }

    /// The id of the object at `path` (`/`-separated, exact names).
    pub(crate) fn find(&self, path: &str) -> Option<u64> {
        let state = self.state();
        let mut parent = None;
        let mut found = None;
        for part in path.split('/') {
            let id = state
                .entries
                .iter()
                .find(|(_, e)| e.parent == parent && e.name == part)
                .map(|(id, _)| *id)?;
            parent = Some(id);
            found = Some(id);
        }
        found
    }

    /// Create the folders of `path` as needed; the last one's id.
    pub(crate) fn mkdir(&self, path: &str) -> u64 {
        let mut parent = None;
        for (i, _) in path.split('/').enumerate() {
            let prefix = path.split('/').take(i + 1).collect::<Vec<_>>().join("/");
            let id = match self.find(&prefix) {
                Some(id) => id,
                None => {
                    let name = prefix.rsplit('/').next().unwrap().to_string();
                    self.state().insert(parent, &name, None)
                }
            };
            parent = Some(id);
        }
        parent.expect("a non-empty path")
    }

    /// Put a file at `path`, creating its folders.
    pub(crate) fn put(&self, path: &str, data: &[u8]) -> u64 {
        let (dir, name) = path
            .rsplit_once('/')
            .map_or((None, path), |(d, n)| (Some(d), n));
        let parent = dir.map(|d| self.mkdir(d));
        self.state().insert(parent, name, Some(data.to_vec()))
    }

    pub(crate) fn read(&self, path: &str) -> Option<Vec<u8>> {
        let id = self.find(path)?;
        Some(self.state().entries[&id].data.clone())
    }

    /// Every file and folder path, sorted.
    pub(crate) fn paths(&self) -> Vec<String> {
        let state = self.state();
        let path_of = |mut id: u64| {
            let mut parts = vec![];
            loop {
                let e = &state.entries[&id];
                parts.push(e.name.clone());
                match e.parent {
                    Some(p) => id = p,
                    None => break,
                }
            }
            parts.reverse();
            parts.join("/")
        };
        let mut out: Vec<String> = state.entries.keys().map(|&id| path_of(id)).collect();
        out.sort();
        out
    }

    pub(crate) fn unplug(&self) {
        self.state().plugged = false;
    }

    pub(crate) fn plug(&self) {
        self.state().plugged = true;
    }

    pub(crate) fn session(&self) -> FakeSession {
        self.state().opens += 1;
        let model = self.state().model.clone();
        FakeSession {
            device: self.clone(),
            healthy: true,
            orphans: vec![],
            description: DeviceDescription {
                manufacturer: "Amazon".into(),
                model,
            },
        }
    }
}

/// A session onto a [`FakeDevice`].
pub(crate) struct FakeSession {
    device: FakeDevice,
    healthy: bool,
    orphans: Vec<Orphan>,
    description: DeviceDescription,
}

fn parse(id: &str) -> Result<u64, MtpError> {
    id.parse()
        .map_err(|_| MtpError::NotFound(format!("Object {id:?}")))
}

impl FakeSession {
    /// Common checks: unhealthy, unplugged, busy.
    fn check(&mut self) -> Result<MutexGuard<'_, DeviceState>, MtpError> {
        if !self.healthy {
            return Err(MtpError::Transport(
                "the connection must be reopened after an error".into(),
            ));
        }
        let mut state = self.device.0.lock().unwrap();
        if !state.plugged {
            self.healthy = false;
            return Err(MtpError::Disconnected);
        }
        if state.busy > 0 {
            state.busy -= 1;
            return Err(MtpError::Busy);
        }
        Ok(state)
    }

    /// What the PTP transport reports for an unknown object.
    fn missing(_id: &str) -> MtpError {
        MtpError::Response {
            operation: 0x1008,
            code: super::protocol::rc::INVALID_OBJECT_HANDLE,
        }
    }
}

impl MtpSession for FakeSession {
    fn description(&self) -> &DeviceDescription {
        &self.description
    }

    fn storages(&mut self) -> Result<Vec<Storage>, MtpError> {
        let state = self.check()?;
        Ok(vec![Storage {
            id: STORAGE.into(),
            description: "Internal Storage".into(),
            free_bytes: Some(state.capacity.saturating_sub(state.used())),
            capacity_bytes: Some(state.capacity),
            writable: true,
        }])
    }

    fn children(&mut self, storage: &str, parent: Option<&str>) -> Result<Vec<Object>, MtpError> {
        let parent = parent.map(parse).transpose()?;
        let mut state = self.check()?;
        state.listings += 1;
        if storage != STORAGE {
            return Err(MtpError::Response {
                operation: 0x1007,
                code: super::protocol::rc::INVALID_STORAGE_ID,
            });
        }
        if parent.is_some_and(|p| !state.entries.contains_key(&p)) {
            return Err(MtpError::Response {
                operation: 0x1007,
                code: super::protocol::rc::INVALID_OBJECT_HANDLE,
            });
        }
        Ok(state
            .entries
            .iter()
            .filter(|(_, e)| e.parent == parent)
            .filter_map(|(id, _)| state.object(*id))
            .collect())
    }

    fn object(&mut self, id: &str) -> Result<Object, MtpError> {
        let n = parse(id)?;
        let state = self.check()?;
        state.object(n).ok_or_else(|| Self::missing(id))
    }

    fn create_folder(
        &mut self,
        storage: &str,
        parent: Option<&str>,
        name: &str,
    ) -> Result<Object, MtpError> {
        let parent = parent.map(parse).transpose()?;
        let mut state = self.check()?;
        assert_eq!(storage, STORAGE);
        let id = state.insert(parent, name, None);
        state.log.push(format!("mkdir {name}"));
        Ok(state.object(id).unwrap())
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
        let parent_id = parent.map(parse).transpose()?;
        let (id, fail_after) = {
            let mut state = self.check()?;
            assert_eq!(storage, STORAGE);
            if state.used() + size > state.capacity {
                return Err(MtpError::Response {
                    operation: 0x100C,
                    code: super::protocol::rc::STORE_FULL,
                });
            }
            let id = state.insert(parent_id, name, Some(vec![]));
            state.log.push(format!("send {name}"));
            (id, state.fail_send_after.take())
        };
        let orphan = Orphan {
            id: id.to_string(),
            storage: storage.into(),
            parent: parent.map(str::to_string),
            name: name.into(),
        };
        let mut data = Vec::new();
        let mut buf = vec![0u8; 64 * 1024];
        while (data.len() as u64) < size {
            let want = buf.len().min((size - data.len() as u64) as usize);
            let n = source
                .read(&mut buf[..want])
                .map_err(|e| MtpError::Local(e.to_string()))?;
            if n == 0 {
                self.healthy = false;
                self.orphans.push(orphan);
                return Err(MtpError::Local("short source".into()));
            }
            data.extend_from_slice(&buf[..n]);
            if fail_after.is_some_and(|limit| data.len() as u64 >= limit)
                || !self.device.state().plugged
            {
                self.healthy = false;
                self.device.state().entries.get_mut(&id).unwrap().data = data;
                self.orphans.push(orphan);
                return Err(MtpError::Disconnected);
            }
            let delay = self.device.state().chunk_delay;
            if !delay.is_zero() {
                std::thread::sleep(delay);
            }
            let done = data.len() as u64;
            if !progress(done) && done < size {
                // Like the PTP transport: the data phase is aborted, the
                // session must be reopened, the partial object stays.
                self.healthy = false;
                self.device.state().entries.get_mut(&id).unwrap().data = data;
                self.orphans.push(orphan);
                return Err(MtpError::Cancelled);
            }
        }
        progress(size);
        let mut state = self.device.state();
        state.entries.get_mut(&id).unwrap().data = data;
        Ok(state.object(id).unwrap())
    }

    fn read_file(
        &mut self,
        object: &Object,
        sink: &mut dyn Write,
        progress: Progress,
    ) -> Result<u64, MtpError> {
        let id = parse(&object.id)?;
        let data = {
            let state = self.check()?;
            state
                .entries
                .get(&id)
                .ok_or_else(|| Self::missing(&object.id))?
                .data
                .clone()
        };
        let mut done = 0u64;
        for chunk in data.chunks(64 * 1024) {
            sink.write_all(chunk)
                .map_err(|e| MtpError::Local(e.to_string()))?;
            done += chunk.len() as u64;
            if !progress(done) && done < data.len() as u64 {
                return Err(MtpError::Cancelled);
            }
        }
        Ok(done)
    }

    fn delete(&mut self, id: &str) -> Result<(), MtpError> {
        let n = parse(id)?;
        let mut state = self.check()?;
        if state.entries.values().any(|e| e.parent == Some(n)) {
            return Err(MtpError::Response {
                operation: 0x100B,
                code: super::protocol::rc::PARTIAL_DELETION,
            });
        }
        let entry = state.entries.remove(&n).ok_or_else(|| Self::missing(id))?;
        state.log.push(format!("delete {}", entry.name));
        Ok(())
    }

    fn can_rename(&self) -> bool {
        self.device.state().can_rename
    }

    fn rename(&mut self, id: &str, new_name: &str) -> Result<(), MtpError> {
        let n = parse(id)?;
        let mut state = self.check()?;
        if !state.can_rename {
            return Err(MtpError::Response {
                operation: 0x9804,
                code: super::protocol::rc::OPERATION_NOT_SUPPORTED,
            });
        }
        let entry = state.entries.get_mut(&n).ok_or_else(|| Self::missing(id))?;
        let old = std::mem::replace(&mut entry.name, new_name.to_string());
        entry.version += 1;
        state.log.push(format!("rename {old} -> {new_name}"));
        Ok(())
    }

    fn is_healthy(&self) -> bool {
        self.healthy
    }

    fn take_orphans(&mut self) -> Vec<Orphan> {
        std::mem::take(&mut self.orphans)
    }
}

/// A backend whose devices are [`FakeDevice`]s, one per key.
#[derive(Default)]
pub(crate) struct FakeBackend {
    pub devices: Mutex<Vec<(String, FakeDevice)>>,
}

impl FakeBackend {
    pub(crate) fn with(devices: &[(&str, &FakeDevice)]) -> Arc<Self> {
        Arc::new(Self {
            devices: Mutex::new(
                devices
                    .iter()
                    .map(|(k, d)| (k.to_string(), (*d).clone()))
                    .collect(),
            ),
        })
    }
}

impl Backend for FakeBackend {
    fn enumerate(&self) -> Vec<Candidate> {
        self.devices
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, d)| d.state().plugged)
            .map(|(key, _)| Candidate {
                key: key.clone(),
                label: "Kindle".into(),
                native: format!("native-{key}-SERIAL"),
            })
            .collect()
    }

    fn open(&self, candidate: &Candidate) -> Result<Box<dyn MtpSession>, MtpError> {
        let device = self
            .devices
            .lock()
            .unwrap()
            .iter()
            .find(|(k, _)| *k == candidate.key)
            .map(|(_, d)| d.clone())
            .ok_or(MtpError::Disconnected)?;
        if !device.state().plugged {
            return Err(MtpError::Disconnected);
        }
        // Until the test clears it, like another app holding the device.
        if let Some(err) = device.state().claim_error.clone() {
            return Err(err);
        }
        Ok(Box::new(device.session()))
    }
}

/// A claim failure as the USB backend reports one.
pub(crate) fn claimed_by_another_app() -> MtpError {
    MtpError::Claim {
        kind: ClaimKind::Busy,
        detail: "could not open interface for exclusive access".into(),
    }
}
