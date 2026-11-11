//! Windows: MTP Kindles through Windows Portable Devices (WPD), the OS's
//! own MTP stack. Windows binds its MTP driver to the device, so claiming
//! the USB interface (as on macOS/Linux) would fight it; WPD's COM API is
//! the way in.
//!
//! **NOT COMPILED OR RUN on the development machine** (macOS only): it was
//! written against the `windows` 0.62.2 sources (signatures checked one by
//! one) and is compiled by the Windows CI job. TODO(device-validation): a
//! full pass on Windows with a 2024 Kindle (detection, send, list, delete,
//! import, progress, unplug mid-transfer, another app holding it).
//!
//! **Which devices**: the WPD devices whose PnP id is an Amazon USB device
//! ([`super::is_amazon_pnp_id`]: VID 0x1949, not a mass-storage volume)
//! and whose description isn't a Fire tablet's. Never the friendly name
//! (the user can rename it). The PnP id carries the serial number, so it
//! stays inside this backend ([`Candidate::native`]); the key is `wpd-<n>`.
//!
//! **COM**: each device's worker thread enters the multithreaded apartment
//! before opening the session and leaves it after the session's last COM
//! object is released ([`WpdBackend::thread_setup`]; this replaces the
//! `feat/mtp` branch's per-call guard, which uninitialised COM while
//! objects were still alive). Enumeration runs on Tauri's blocking pool in
//! a [`ComScope`] that outlives every object it creates.
//!
//! Object ids are WPD's own strings; a storage's root folder is the
//! storage object itself (mapped to `parent: None`).

use std::any::Any;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::Mutex;

use windows::core::{Interface, GUID, HRESULT, HSTRING, PWSTR};
use windows::Win32::Devices::PortableDevices::{
    IEnumPortableDeviceObjectIDs, IPortableDevice, IPortableDeviceContent,
    IPortableDeviceDataStream, IPortableDeviceKeyCollection, IPortableDeviceManager,
    IPortableDevicePropVariantCollection, IPortableDeviceProperties, IPortableDeviceResources,
    IPortableDeviceValues, PortableDeviceFTM, PortableDeviceKeyCollection, PortableDeviceManager,
    PortableDevicePropVariantCollection, PortableDeviceValues, PORTABLE_DEVICE_DELETE_NO_RECURSION,
    WPD_CLIENT_MAJOR_VERSION, WPD_CLIENT_MINOR_VERSION, WPD_CLIENT_NAME, WPD_CLIENT_REVISION,
    WPD_CONTENT_TYPE_FOLDER, WPD_CONTENT_TYPE_FUNCTIONAL_OBJECT, WPD_CONTENT_TYPE_GENERIC_FILE,
    WPD_DEVICE_MANUFACTURER, WPD_DEVICE_MODEL, WPD_DEVICE_OBJECT_ID,
    WPD_FUNCTIONAL_CATEGORY_STORAGE, WPD_FUNCTIONAL_OBJECT_CATEGORY,
    WPD_OBJECT_CONTAINER_FUNCTIONAL_OBJECT_ID, WPD_OBJECT_CONTENT_TYPE, WPD_OBJECT_DATE_MODIFIED,
    WPD_OBJECT_FORMAT, WPD_OBJECT_FORMAT_PROPERTIES_ONLY, WPD_OBJECT_FORMAT_UNSPECIFIED,
    WPD_OBJECT_NAME, WPD_OBJECT_ORIGINAL_FILE_NAME, WPD_OBJECT_PARENT_ID, WPD_OBJECT_SIZE,
    WPD_RESOURCE_DEFAULT, WPD_STORAGE_ACCESS_CAPABILITY, WPD_STORAGE_CAPACITY,
    WPD_STORAGE_DESCRIPTION, WPD_STORAGE_FREE_SPACE_IN_BYTES,
};
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::System::Com::StructuredStorage::{PropVariantClear, PROPVARIANT};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemAlloc, CoTaskMemFree, CoUninitialize, IStream,
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, STGC_DEFAULT, STGM_READ,
};
use windows::Win32::System::Variant::{VT_DATE, VT_LPWSTR};

use super::session::{
    Backend, Candidate, ClaimKind, DeviceDescription, MtpError, MtpSession, Object, Orphan,
    Progress, Storage,
};
use super::{is_amazon_pnp_id, is_fire_tablet};

/// The device object's id (`WPD_DEVICE_OBJECT_ID`), as a string.
const DEVICE: &str = "DEVICE";

/// Bounds of a stream chunk (the device suggests a size within them).
const MIN_CHUNK: usize = 64 * 1024;
const MAX_CHUNK: usize = 1 << 20;

// ── COM ───────────────────────────────────────────────────────────────────────

/// COM entered (multithreaded apartment) on this thread while the guard
/// lives. A thread already in another apartment (`RPC_E_CHANGED_MODE`)
/// can still use COM, but that initialisation isn't ours to undo.
struct ComScope {
    owned: bool,
}

impl ComScope {
    fn enter() -> Self {
        // S_OK or S_FALSE (already initialised): both must be balanced.
        let owned = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
        ComScope { owned }
    }
}

impl Drop for ComScope {
    fn drop(&mut self) {
        if self.owned {
            unsafe { CoUninitialize() };
        }
    }
}

/// A string WPD allocated, copied and freed.
unsafe fn take_pwstr(p: PWSTR) -> Option<String> {
    if p.is_null() {
        return None;
    }
    let text = unsafe { p.to_string() }.ok();
    unsafe { CoTaskMemFree(Some(p.0 as *const _)) };
    text
}

// HRESULTs that mean the device is gone (HRESULT_FROM_WIN32 of
// ERROR_DEVICE_NOT_CONNECTED, ERROR_GEN_FAILURE, ERROR_NOT_READY,
// ERROR_DEV_NOT_EXIST), busy, cancelled, timed out, refused.
const HR_DEVICE_NOT_CONNECTED: i32 = 0x8007_048F_u32 as i32;
const HR_GEN_FAILURE: i32 = 0x8007_001F_u32 as i32;
const HR_NOT_READY: i32 = 0x8007_0015_u32 as i32;
const HR_DEV_NOT_EXIST: i32 = 0x8007_0037_u32 as i32;
const HR_BUSY: i32 = 0x8007_00AA_u32 as i32;
const HR_CANCELLED: i32 = 0x8007_04C7_u32 as i32;
const HR_SEM_TIMEOUT: i32 = 0x8007_0079_u32 as i32;
const HR_ACCESS_DENIED: i32 = 0x8007_0005_u32 as i32;
const HR_DISK_FULL: i32 = 0x8007_0070_u32 as i32;

/// A WPD failure while doing `what`. Only lost-device codes and timeouts
/// are fatal (the worker reconnects); the rest fail the operation alone.
fn wpd_error(what: &str, e: &windows::core::Error) -> MtpError {
    match e.code().0 {
        HR_DEVICE_NOT_CONNECTED | HR_GEN_FAILURE | HR_NOT_READY | HR_DEV_NOT_EXIST => {
            MtpError::Disconnected
        }
        HR_BUSY => MtpError::Busy,
        HR_CANCELLED => MtpError::Cancelled,
        HR_SEM_TIMEOUT => MtpError::Transport(format!("{what}: the Kindle stopped responding")),
        HR_DISK_FULL => MtpError::Refused("The Kindle is full.".into()),
        HR_ACCESS_DENIED => MtpError::Refused(format!("The Kindle refused to {what}.")),
        _ => MtpError::Refused(format!("Windows couldn't {what} on the Kindle ({e}).")),
    }
}

/// `?` for WPD calls: the error with what was being done.
trait Wpd<T> {
    fn wpd(self, what: &str) -> Result<T, MtpError>;
}

impl<T> Wpd<T> for windows::core::Result<T> {
    fn wpd(self, what: &str) -> Result<T, MtpError> {
        self.map_err(|e| wpd_error(what, &e))
    }
}

fn hresult(what: &str, hr: HRESULT) -> Result<(), MtpError> {
    hr.ok().wpd(what)
}

// ── Enumeration ───────────────────────────────────────────────────────────────

/// The Windows [`Backend`].
#[derive(Default)]
pub(crate) struct WpdBackend {
    keys: Mutex<Keys>,
}

/// The `wpd-<n>` key of each PnP id seen since the app started (a device
/// keeps its key while the app runs).
#[derive(Default)]
struct Keys {
    by_id: HashMap<String, String>,
    last: u32,
}

impl Keys {
    fn key_for(&mut self, pnp_id: &str) -> String {
        if let Some(key) = self.by_id.get(pnp_id) {
            return key.clone();
        }
        self.last += 1;
        let key = format!("wpd-{}", self.last);
        self.by_id.insert(pnp_id.to_string(), key.clone());
        key
    }
}

/// Every WPD device's PnP id.
unsafe fn device_ids(manager: &IPortableDeviceManager) -> windows::core::Result<Vec<String>> {
    let mut count = 0u32;
    unsafe { manager.GetDevices(std::ptr::null_mut(), &mut count) }?;
    if count == 0 {
        return Ok(Vec::new());
    }
    let mut ids = vec![PWSTR::null(); count as usize];
    unsafe { manager.GetDevices(ids.as_mut_ptr(), &mut count) }?;
    Ok(ids
        .into_iter()
        .take(count as usize)
        .filter_map(|p| unsafe { take_pwstr(p) })
        .collect())
}

/// One of the manager's string getters (`GetDeviceDescription`…), which
/// take a buffer and its length in characters.
unsafe fn manager_string(
    get: impl Fn(PWSTR, *mut u32) -> windows::core::Result<()>,
) -> Option<String> {
    let mut len = 0u32;
    let _ = get(PWSTR::null(), &mut len);
    if len == 0 {
        return None;
    }
    let mut buf = vec![0u16; len as usize];
    get(PWSTR(buf.as_mut_ptr()), &mut len).ok()?;
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..end]))
}

impl Backend for WpdBackend {
    fn enumerate(&self) -> Vec<Candidate> {
        let _com = ComScope::enter();
        let found = unsafe {
            let Ok(manager) = CoCreateInstance::<_, IPortableDeviceManager>(
                &PortableDeviceManager,
                None,
                CLSCTX_INPROC_SERVER,
            ) else {
                return Vec::new();
            };
            let ids = device_ids(&manager).unwrap_or_default();
            let mut found = Vec::new();
            for id in ids.into_iter().filter(|id| is_amazon_pnp_id(id)) {
                let h = HSTRING::from(id.as_str());
                let description =
                    manager_string(|buf, len| manager.GetDeviceDescription(&h, buf, len));
                if is_fire_tablet(description.as_deref()) {
                    continue;
                }
                found.push((id, description));
            }
            found
            // `manager` is released here, before `_com`.
        };
        let mut keys = self.keys.lock().unwrap_or_else(|e| e.into_inner());
        let mut out: Vec<Candidate> = found
            .into_iter()
            .map(|(id, description)| Candidate {
                key: keys.key_for(&id),
                label: description
                    .filter(|d| !d.trim().is_empty())
                    .unwrap_or_else(|| "Kindle".into()),
                native: id,
            })
            .collect();
        out.sort_by(|a, b| a.key.cmp(&b.key));
        out
    }

    fn open(&self, candidate: &Candidate) -> Result<Box<dyn MtpSession>, MtpError> {
        Ok(Box::new(unsafe { WpdSession::open(&candidate.native)? }))
    }

    fn thread_setup(&self) -> Box<dyn Any> {
        Box::new(ComScope::enter())
    }
}

// ── Session ───────────────────────────────────────────────────────────────────

struct WpdSession {
    device: IPortableDevice,
    content: IPortableDeviceContent,
    properties: IPortableDeviceProperties,
    resources: IPortableDeviceResources,
    /// The properties read for each object.
    object_keys: IPortableDeviceKeyCollection,
    /// The properties read for each functional object (storages).
    storage_keys: IPortableDeviceKeyCollection,
    description: DeviceDescription,
    healthy: bool,
}

unsafe fn key_collection(
    keys: &[PROPERTYKEY],
) -> windows::core::Result<IPortableDeviceKeyCollection> {
    let collection: IPortableDeviceKeyCollection =
        unsafe { CoCreateInstance(&PortableDeviceKeyCollection, None, CLSCTX_INPROC_SERVER) }?;
    for key in keys {
        unsafe { collection.Add(key) }?;
    }
    Ok(collection)
}

unsafe fn new_values() -> windows::core::Result<IPortableDeviceValues> {
    unsafe { CoCreateInstance(&PortableDeviceValues, None, CLSCTX_INPROC_SERVER) }
}

unsafe fn string_value(values: &IPortableDeviceValues, key: &PROPERTYKEY) -> Option<String> {
    unsafe { values.GetStringValue(key).ok().and_then(|p| take_pwstr(p)) }
}

/// A date property as text (only compared, as a cache key).
unsafe fn date_value(values: &IPortableDeviceValues, key: &PROPERTYKEY) -> String {
    let Ok(mut value) = (unsafe { values.GetValue(key) }) else {
        return String::new();
    };
    let text = unsafe {
        let inner = &value.Anonymous.Anonymous;
        if inner.vt == VT_DATE {
            format!("{}", inner.Anonymous.date)
        } else {
            String::new()
        }
    };
    let _ = unsafe { PropVariantClear(&mut value) };
    text
}

impl WpdSession {
    unsafe fn open(pnp_id: &str) -> Result<Self, MtpError> {
        unsafe {
            let client = new_values().wpd("prepare the connection")?;
            client
                .SetStringValue(&WPD_CLIENT_NAME, windows::core::w!("Librarium"))
                .wpd("prepare the connection")?;
            client
                .SetUnsignedIntegerValue(&WPD_CLIENT_MAJOR_VERSION, 1)
                .wpd("prepare the connection")?;
            client
                .SetUnsignedIntegerValue(&WPD_CLIENT_MINOR_VERSION, 0)
                .wpd("prepare the connection")?;
            client
                .SetUnsignedIntegerValue(&WPD_CLIENT_REVISION, 0)
                .wpd("prepare the connection")?;
            let device: IPortableDevice =
                CoCreateInstance(&PortableDeviceFTM, None, CLSCTX_INPROC_SERVER)
                    .wpd("prepare the connection")?;
            let id = HSTRING::from(pnp_id);
            device
                .Open(&id, &client)
                .map_err(|e| match wpd_error("open the device", &e) {
                    MtpError::Disconnected => MtpError::Disconnected,
                    MtpError::Busy => MtpError::Claim {
                        kind: ClaimKind::Busy,
                        detail: e.to_string(),
                    },
                    _ if e.code().0 == HR_ACCESS_DENIED => MtpError::Claim {
                        kind: ClaimKind::PermissionDenied,
                        detail: e.to_string(),
                    },
                    _ => MtpError::Claim {
                        kind: ClaimKind::Other,
                        detail: e.to_string(),
                    },
                })?;
            let content = device.Content().wpd("open the device")?;
            let properties = content.Properties().wpd("open the device")?;
            let resources = content.Transfer().wpd("open the device")?;
            let object_keys = key_collection(&[
                WPD_OBJECT_PARENT_ID,
                WPD_OBJECT_NAME,
                WPD_OBJECT_ORIGINAL_FILE_NAME,
                WPD_OBJECT_SIZE,
                WPD_OBJECT_CONTENT_TYPE,
                WPD_OBJECT_DATE_MODIFIED,
                WPD_OBJECT_CONTAINER_FUNCTIONAL_OBJECT_ID,
            ])
            .wpd("open the device")?;
            let storage_keys = key_collection(&[
                WPD_OBJECT_CONTENT_TYPE,
                WPD_FUNCTIONAL_OBJECT_CATEGORY,
                WPD_STORAGE_DESCRIPTION,
                WPD_STORAGE_FREE_SPACE_IN_BYTES,
                WPD_STORAGE_CAPACITY,
                WPD_STORAGE_ACCESS_CAPABILITY,
            ])
            .wpd("open the device")?;
            // Model and manufacturer only: WPD_DEVICE_SERIAL_NUMBER is never read.
            let device_keys = key_collection(&[WPD_DEVICE_MODEL, WPD_DEVICE_MANUFACTURER])
                .wpd("open the device")?;
            let description = match properties.GetValues(WPD_DEVICE_OBJECT_ID, &device_keys) {
                Ok(values) => DeviceDescription {
                    manufacturer: string_value(&values, &WPD_DEVICE_MANUFACTURER)
                        .unwrap_or_default(),
                    model: string_value(&values, &WPD_DEVICE_MODEL).unwrap_or_default(),
                },
                Err(_) => DeviceDescription::default(),
            };
            Ok(WpdSession {
                device,
                content,
                properties,
                resources,
                object_keys,
                storage_keys,
                description,
                healthy: true,
            })
        }
    }

    /// Note a fatal error (the worker then reconnects).
    fn check<T>(&mut self, result: Result<T, MtpError>) -> Result<T, MtpError> {
        if matches!(&result, Err(e) if e.is_fatal()) {
            self.healthy = false;
        }
        result
    }

    /// The ids of the objects directly in `parent`.
    unsafe fn child_ids(&self, parent: &str) -> Result<Vec<String>, MtpError> {
        unsafe {
            let parent = HSTRING::from(parent);
            let ids: IEnumPortableDeviceObjectIDs = self
                .content
                .EnumObjects(0, &parent, None::<&IPortableDeviceValues>)
                .wpd("list a folder")?;
            let mut out = Vec::new();
            loop {
                let mut batch = [PWSTR::null(); 64];
                let mut fetched = 0u32;
                let hr = ids.Next(&mut batch, &mut fetched);
                for p in batch.iter().take(fetched as usize) {
                    if let Some(id) = take_pwstr(*p) {
                        out.push(id);
                    }
                }
                hresult("list a folder", hr)?;
                // S_FALSE: fewer than asked, the enumeration is over.
                if hr.0 != 0 || fetched == 0 {
                    break;
                }
            }
            Ok(out)
        }
    }

    /// One object's properties as an [`Object`]. `storage_hint` is the
    /// storage being listed, when known.
    unsafe fn read_object(&self, id: &str, storage_hint: Option<&str>) -> Result<Object, MtpError> {
        unsafe {
            let h = HSTRING::from(id);
            let values = self
                .properties
                .GetValues(&h, &self.object_keys)
                .wpd("read a file's details")?;
            let storage = string_value(&values, &WPD_OBJECT_CONTAINER_FUNCTIONAL_OBJECT_ID)
                .or_else(|| storage_hint.map(str::to_string))
                .unwrap_or_default();
            let parent = string_value(&values, &WPD_OBJECT_PARENT_ID)
                .filter(|p| !p.is_empty() && *p != storage);
            let name = string_value(&values, &WPD_OBJECT_ORIGINAL_FILE_NAME)
                .filter(|n| !n.is_empty())
                .or_else(|| string_value(&values, &WPD_OBJECT_NAME))
                .unwrap_or_default();
            let content_type = values
                .GetGuidValue(&WPD_OBJECT_CONTENT_TYPE)
                .unwrap_or(GUID::zeroed());
            Ok(Object {
                id: id.to_string(),
                storage,
                parent,
                name,
                size: values
                    .GetUnsignedLargeIntegerValue(&WPD_OBJECT_SIZE)
                    .unwrap_or(0),
                is_folder: content_type == WPD_CONTENT_TYPE_FOLDER
                    || content_type == WPD_CONTENT_TYPE_FUNCTIONAL_OBJECT,
                modified: date_value(&values, &WPD_OBJECT_DATE_MODIFIED),
            })
        }
    }

    unsafe fn storages_inner(&self) -> Result<Vec<Storage>, MtpError> {
        unsafe {
            let mut out = Vec::new();
            for id in self.child_ids(DEVICE)? {
                let h = HSTRING::from(id.as_str());
                let Ok(values) = self.properties.GetValues(&h, &self.storage_keys) else {
                    continue;
                };
                if values
                    .GetGuidValue(&WPD_FUNCTIONAL_OBJECT_CATEGORY)
                    .unwrap_or(GUID::zeroed())
                    != WPD_FUNCTIONAL_CATEGORY_STORAGE
                {
                    continue;
                }
                out.push(Storage {
                    description: string_value(&values, &WPD_STORAGE_DESCRIPTION)
                        .unwrap_or_default(),
                    free_bytes: values
                        .GetUnsignedLargeIntegerValue(&WPD_STORAGE_FREE_SPACE_IN_BYTES)
                        .ok(),
                    capacity_bytes: values
                        .GetUnsignedLargeIntegerValue(&WPD_STORAGE_CAPACITY)
                        .ok(),
                    // WPD_STORAGE_ACCESS_CAPABILITY_READWRITE = 0.
                    writable: values
                        .GetUnsignedIntegerValue(&WPD_STORAGE_ACCESS_CAPABILITY)
                        .unwrap_or(0)
                        == 0,
                    id,
                });
            }
            Ok(out)
        }
    }

    unsafe fn create_folder_inner(
        &self,
        storage: &str,
        parent: Option<&str>,
        name: &str,
    ) -> Result<Object, MtpError> {
        unsafe {
            let values = new_values().wpd("create a folder")?;
            let parent_id = HSTRING::from(parent.unwrap_or(storage));
            let name_h = HSTRING::from(name);
            values
                .SetStringValue(&WPD_OBJECT_PARENT_ID, &parent_id)
                .wpd("create a folder")?;
            values
                .SetStringValue(&WPD_OBJECT_NAME, &name_h)
                .wpd("create a folder")?;
            values
                .SetStringValue(&WPD_OBJECT_ORIGINAL_FILE_NAME, &name_h)
                .wpd("create a folder")?;
            values
                .SetGuidValue(&WPD_OBJECT_CONTENT_TYPE, &WPD_CONTENT_TYPE_FOLDER)
                .wpd("create a folder")?;
            values
                .SetGuidValue(&WPD_OBJECT_FORMAT, &WPD_OBJECT_FORMAT_PROPERTIES_ONLY)
                .wpd("create a folder")?;
            let mut id = PWSTR::null();
            self.content
                .CreateObjectWithPropertiesOnly(&values, &mut id)
                .wpd("create a folder")?;
            let id = take_pwstr(id)
                .ok_or_else(|| MtpError::Protocol("the new folder has no id".into()))?;
            Ok(Object {
                id,
                storage: storage.to_string(),
                parent: parent.map(str::to_string),
                name: name.to_string(),
                size: 0,
                is_folder: true,
                modified: String::new(),
            })
        }
    }

    unsafe fn send_inner(
        &self,
        storage: &str,
        parent: Option<&str>,
        name: &str,
        size: u64,
        source: &mut dyn Read,
        progress: Progress,
    ) -> Result<Object, MtpError> {
        unsafe {
            let values = new_values().wpd("send a file")?;
            let parent_id = HSTRING::from(parent.unwrap_or(storage));
            let name_h = HSTRING::from(name);
            values
                .SetStringValue(&WPD_OBJECT_PARENT_ID, &parent_id)
                .wpd("send a file")?;
            values
                .SetStringValue(&WPD_OBJECT_NAME, &name_h)
                .wpd("send a file")?;
            values
                .SetStringValue(&WPD_OBJECT_ORIGINAL_FILE_NAME, &name_h)
                .wpd("send a file")?;
            values
                .SetUnsignedLargeIntegerValue(&WPD_OBJECT_SIZE, size)
                .wpd("send a file")?;
            values
                .SetGuidValue(&WPD_OBJECT_CONTENT_TYPE, &WPD_CONTENT_TYPE_GENERIC_FILE)
                .wpd("send a file")?;
            values
                .SetGuidValue(&WPD_OBJECT_FORMAT, &WPD_OBJECT_FORMAT_UNSPECIFIED)
                .wpd("send a file")?;

            let mut stream: Option<IStream> = None;
            let mut optimal = 0u32;
            let mut cookie = PWSTR::null();
            self.content
                .CreateObjectWithPropertiesAndData(&values, &mut stream, &mut optimal, &mut cookie)
                .wpd("send a file")?;
            let _ = take_pwstr(cookie);
            let stream = stream.ok_or_else(|| {
                MtpError::Protocol("Windows returned no stream to write the file".into())
            })?;
            let data: IPortableDeviceDataStream = stream.cast().wpd("send a file")?;

            // Until Commit, nothing is created: a cancel or error leaves nothing behind.
            let abort = |err: MtpError| {
                let _ = data.Cancel();
                err
            };
            let mut buf = vec![0u8; (optimal as usize).clamp(MIN_CHUNK, MAX_CHUNK)];
            let mut sent = 0u64;
            while sent < size {
                let want = buf.len().min((size - sent) as usize);
                let n = match source.read(&mut buf[..want]) {
                    Ok(0) => {
                        return Err(abort(MtpError::Local(
                            "The file got shorter while it was being sent.".into(),
                        )))
                    }
                    Ok(n) => n,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(e) => {
                        return Err(abort(MtpError::Local(format!(
                            "Could not read the file being sent: {e}"
                        ))))
                    }
                };
                let mut written = 0u32;
                if let Err(e) = hresult(
                    "send a file",
                    stream.Write(
                        buf.as_ptr() as *const _,
                        n as u32,
                        Some(&mut written as *mut u32),
                    ),
                ) {
                    return Err(abort(e));
                }
                if written as usize != n {
                    return Err(abort(MtpError::Transport(
                        "the Kindle accepted only part of a chunk".into(),
                    )));
                }
                sent += n as u64;
                if !progress(sent) && sent < size {
                    return Err(abort(MtpError::Cancelled));
                }
            }
            stream.Commit(STGC_DEFAULT).wpd("send a file")?;
            let id = take_pwstr(data.GetObjectID().wpd("send a file")?)
                .ok_or_else(|| MtpError::Protocol("the new file has no id".into()))?;
            Ok(Object {
                id,
                storage: storage.to_string(),
                parent: parent.map(str::to_string),
                name: name.to_string(),
                size,
                is_folder: false,
                modified: String::new(),
            })
        }
    }

    unsafe fn read_inner(
        &self,
        object: &Object,
        sink: &mut dyn Write,
        progress: Progress,
    ) -> Result<u64, MtpError> {
        unsafe {
            let h = HSTRING::from(object.id.as_str());
            let mut optimal = 0u32;
            let mut stream: Option<IStream> = None;
            self.resources
                .GetStream(
                    &h,
                    &WPD_RESOURCE_DEFAULT,
                    STGM_READ.0,
                    &mut optimal,
                    &mut stream,
                )
                .wpd("read a file")?;
            let stream = stream.ok_or_else(|| {
                MtpError::Protocol("Windows returned no stream to read the file".into())
            })?;
            let mut buf = vec![0u8; (optimal as usize).clamp(MIN_CHUNK, MAX_CHUNK)];
            let mut done = 0u64;
            loop {
                let mut read = 0u32;
                hresult(
                    "read a file",
                    stream.Read(
                        buf.as_mut_ptr() as *mut _,
                        buf.len() as u32,
                        Some(&mut read as *mut u32),
                    ),
                )?;
                if read == 0 {
                    break;
                }
                sink.write_all(&buf[..read as usize]).map_err(|e| {
                    MtpError::Local(format!(
                        "Could not write the file copied from the Kindle: {e}"
                    ))
                })?;
                done += u64::from(read);
                if !progress(done) && done < object.size {
                    // Dropping the stream ends the transfer.
                    return Err(MtpError::Cancelled);
                }
            }
            Ok(done)
        }
    }

    unsafe fn delete_inner(&self, id: &str) -> Result<(), MtpError> {
        unsafe {
            let ids: IPortableDevicePropVariantCollection = CoCreateInstance(
                &PortableDevicePropVariantCollection,
                None,
                CLSCTX_INPROC_SERVER,
            )
            .wpd("delete a file")?;
            // A VT_LPWSTR PROPVARIANT holding a CoTaskMemAlloc'd copy of the id;
            // Add copies it, PropVariantClear frees ours.
            let wide: Vec<u16> = id.encode_utf16().chain(std::iter::once(0)).collect();
            let copy = CoTaskMemAlloc(wide.len() * 2) as *mut u16;
            if copy.is_null() {
                return Err(MtpError::Local("out of memory".into()));
            }
            std::ptr::copy_nonoverlapping(wide.as_ptr(), copy, wide.len());
            let mut value = PROPVARIANT::default();
            (*value.Anonymous.Anonymous).vt = VT_LPWSTR;
            (*value.Anonymous.Anonymous).Anonymous.pwszVal = PWSTR(copy);
            let added = ids.Add(&value).wpd("delete a file");
            let _ = PropVariantClear(&mut value);
            added?;
            self.content
                .Delete(
                    PORTABLE_DEVICE_DELETE_NO_RECURSION.0 as u32,
                    &ids,
                    std::ptr::null_mut(),
                )
                .wpd("delete a file")
        }
    }

    unsafe fn rename_inner(&self, id: &str, new_name: &str) -> Result<(), MtpError> {
        unsafe {
            let values = new_values().wpd("rename a file")?;
            let name = HSTRING::from(new_name);
            values
                .SetStringValue(&WPD_OBJECT_ORIGINAL_FILE_NAME, &name)
                .wpd("rename a file")?;
            let h = HSTRING::from(id);
            let results = self
                .properties
                .SetValues(&h, &values)
                .wpd("rename a file")?;
            // SetValues succeeds overall and reports each property's outcome.
            match results.GetErrorValue(&WPD_OBJECT_ORIGINAL_FILE_NAME) {
                Ok(hr) => hresult("rename a file", hr),
                Err(_) => Ok(()),
            }
        }
    }
}

impl MtpSession for WpdSession {
    fn description(&self) -> &DeviceDescription {
        &self.description
    }

    fn storages(&mut self) -> Result<Vec<Storage>, MtpError> {
        let result = unsafe { self.storages_inner() };
        self.check(result)
    }

    fn children(&mut self, storage: &str, parent: Option<&str>) -> Result<Vec<Object>, MtpError> {
        let result = unsafe {
            self.child_ids(parent.unwrap_or(storage)).and_then(|ids| {
                let mut objects = Vec::with_capacity(ids.len());
                for id in ids {
                    let mut object = self.read_object(&id, Some(storage))?;
                    // Listed in this folder: that is its parent, whatever WPD says.
                    object.storage = storage.to_string();
                    object.parent = parent.map(str::to_string);
                    objects.push(object);
                }
                Ok(objects)
            })
        };
        self.check(result)
    }

    fn object(&mut self, id: &str) -> Result<Object, MtpError> {
        let result = unsafe { self.read_object(id, None) };
        self.check(result)
    }

    fn create_folder(
        &mut self,
        storage: &str,
        parent: Option<&str>,
        name: &str,
    ) -> Result<Object, MtpError> {
        let result = unsafe { self.create_folder_inner(storage, parent, name) };
        self.check(result)
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
        let result = unsafe { self.send_inner(storage, parent, name, size, source, progress) };
        self.check(result)
    }

    fn read_file(
        &mut self,
        object: &Object,
        sink: &mut dyn Write,
        progress: Progress,
    ) -> Result<u64, MtpError> {
        let result = unsafe { self.read_inner(object, sink, progress) };
        self.check(result)
    }

    fn delete(&mut self, id: &str) -> Result<(), MtpError> {
        let result = unsafe { self.delete_inner(id) };
        self.check(result)
    }

    /// WPD renames by setting the original file name; whether a Kindle
    /// allows it is unknown (a refusal makes the transfer fall back).
    fn can_rename(&self) -> bool {
        true
    }

    fn rename(&mut self, id: &str, new_name: &str) -> Result<(), MtpError> {
        let result = unsafe { self.rename_inner(id, new_name) };
        self.check(result)
    }

    fn is_healthy(&self) -> bool {
        self.healthy
    }

    /// Nothing is left behind: an uncommitted WPD object doesn't exist.
    fn take_orphans(&mut self) -> Vec<Orphan> {
        Vec::new()
    }
}

impl Drop for WpdSession {
    fn drop(&mut self) {
        let _ = unsafe { self.device.Close() };
    }
}
