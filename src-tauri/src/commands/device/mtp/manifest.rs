//! The device manifest on an MTP Kindle: the same content as on a USB drive
//! (`commands/device/manifest.rs`: the device id, which library book each
//! file Librarium sent is), stored differently.
//!
//! On a drive the manifest is rewritten atomically (temp file, rename).
//! MTP can't rename a file over another, so instead there are **two
//! copies**, `.librarium/device-1.json` and `.librarium/device-2.json`,
//! each with a `generation` counter:
//! - reading takes the valid copy with the highest generation;
//! - writing replaces the *other* copy, with the next generation.
//!
//! An interrupted write (cable pulled) damages only the copy being written:
//! the previous one still reads, one change behind. A copy written by a
//! newer Librarium is read but never rewritten; damaged copies are simply
//! overwritten (they only describe files, which stay on the device).
//!
//! No lock is needed: every access to one device goes through its worker
//! thread, one job at a time.
//!
//! TODO(device-validation): confirm a Kindle lets MTP create the hidden
//! `.librarium` folder at the storage root (if not, the device gets a
//! fallback id and transfers aren't recorded; see `connect_hook`).

use serde_json::Value;

use crate::commands::device::manifest::{
    self, DeviceManifest, Loaded, ManifestState, MANIFEST_DIR,
};
use crate::mtp::devfs::{same_name, DeviceFs, Folder};
use crate::mtp::session::{MtpError, MtpSession};

/// The two copies, in `.librarium/`.
pub(crate) const SLOTS: [&str; 2] = ["device-1.json", "device-2.json"];

/// The counter each copy carries (a top-level field the drive format
/// doesn't use, so it is kept as an unknown field there).
const GENERATION: &str = "generation";

/// A copy larger than this isn't a manifest Librarium wrote.
const MAX_BYTES: u64 = 8 << 20;

/// A manifest as read from the device.
pub(crate) struct Stored {
    pub loaded: Loaded,
    generation: u64,
    /// The slot the manifest was read from.
    slot: Option<usize>,
}

impl Stored {
    fn absent() -> Self {
        Stored {
            loaded: Loaded::empty(ManifestState::Absent),
            generation: 0,
            slot: None,
        }
    }

    pub(crate) fn manifest(&self) -> &DeviceManifest {
        &self.loaded.manifest
    }
}

/// Read the manifest (see the module docs). Listings are fresh.
pub(crate) fn load(fs: &mut DeviceFs, s: &mut dyn MtpSession) -> Result<Stored, MtpError> {
    let Some(dir) = fs.resolve(s, MANIFEST_DIR, true)?.filter(|o| o.is_folder) else {
        return Ok(Stored::absent());
    };
    let folder = Folder::of(&dir);
    fs.forget(&folder);
    let children = fs.list(s, &folder)?;
    let mut best: Option<Stored> = None;
    let mut damaged: Option<String> = None;
    for (slot, name) in SLOTS.iter().enumerate() {
        let Some(object) = children
            .iter()
            .find(|o| !o.is_folder && same_name(&o.name, name))
        else {
            continue;
        };
        if object.size > MAX_BYTES {
            damaged = Some("is too large".into());
            continue;
        }
        let mut bytes = Vec::new();
        s.read_file(object, &mut bytes, &mut |_| true)?;
        let loaded = manifest::parse(&bytes);
        if let ManifestState::Damaged(reason) = &loaded.state {
            damaged = Some(reason.clone());
            continue;
        }
        let generation = loaded
            .manifest
            .extra
            .get(GENERATION)
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if best.as_ref().is_none_or(|b| generation > b.generation) {
            best = Some(Stored {
                loaded,
                generation,
                slot: Some(slot),
            });
        }
    }
    Ok(match (best, damaged) {
        (Some(stored), _) => stored,
        (None, Some(reason)) => Stored {
            loaded: Loaded::empty(ManifestState::Damaged(reason)),
            generation: 0,
            slot: None,
        },
        (None, None) => Stored::absent(),
    })
}

/// Write `stored` back as the next generation, into the slot it wasn't
/// read from.
pub(crate) fn save(
    fs: &mut DeviceFs,
    s: &mut dyn MtpSession,
    stored: &mut Stored,
) -> Result<(), MtpError> {
    if let ManifestState::TooNew(version) = stored.loaded.state {
        return Err(MtpError::Refused(format!(
            "The device manifest was written by a newer Librarium (format {version}); update Librarium."
        )));
    }
    let folder = fs.ensure_dir(s, MANIFEST_DIR)?;
    let target = if stored.slot == Some(0) { 1 } else { 0 };
    let generation = stored.generation + 1;
    let mut manifest = stored.loaded.manifest.clone();
    manifest
        .extra
        .insert(GENERATION.into(), Value::from(generation));
    let bytes = manifest.to_bytes();

    fs.forget(&folder);
    if let Some(old) = fs
        .list(s, &folder)?
        .into_iter()
        .find(|o| same_name(&o.name, SLOTS[target]))
    {
        fs.delete_tree(s, &old)?;
    }
    let written = s.send_file(
        &folder.storage,
        folder.id.as_deref(),
        SLOTS[target],
        bytes.len() as u64,
        &mut &bytes[..],
        &mut |_| true,
    )?;
    fs.note_added(&written);
    *stored = Stored {
        loaded: Loaded {
            manifest,
            state: ManifestState::Valid,
            dropped: 0,
        },
        generation,
        slot: Some(target),
    };
    Ok(())
}

/// Read-modify-write: `f` gets the current manifest (empty if absent or
/// damaged); it is written only if `f` changed it. Refused for a manifest
/// written by a newer Librarium.
pub(crate) fn update<T>(
    fs: &mut DeviceFs,
    s: &mut dyn MtpSession,
    f: impl FnOnce(&mut DeviceManifest) -> T,
) -> Result<T, MtpError> {
    let mut stored = load(fs, s)?;
    if let ManifestState::TooNew(version) = stored.loaded.state {
        return Err(MtpError::Refused(format!(
            "The device manifest was written by a newer Librarium (format {version}); update Librarium."
        )));
    }
    let before = stored.loaded.manifest.clone();
    let out = f(&mut stored.loaded.manifest);
    if stored.loaded.manifest != before {
        save(fs, s, &mut stored)?;
    }
    Ok(out)
}

/// The device id in the manifest, created (and written) if there is none.
/// Fails when it can't be written, or the manifest is damaged or too new
/// and has none (it isn't rewritten just for an id).
pub(crate) fn ensure_device_id(
    fs: &mut DeviceFs,
    s: &mut dyn MtpSession,
) -> Result<String, MtpError> {
    let mut stored = load(fs, s)?;
    if let Some(id) = &stored.loaded.manifest.device_id {
        return Ok(id.clone());
    }
    match &stored.loaded.state {
        ManifestState::Absent | ManifestState::Valid => {}
        ManifestState::Damaged(reason) => {
            return Err(MtpError::Refused(format!("the device manifest {reason}")))
        }
        ManifestState::TooNew(v) => {
            return Err(MtpError::Refused(format!(
                "the device manifest has format {v} and no device id"
            )))
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    stored.loaded.manifest.device_id = Some(id.clone());
    save(fs, s, &mut stored)?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::device::manifest::ManifestEntry;
    use crate::mtp::fake::FakeDevice;

    const ID: &str = "3f2b1c3e-6e0a-4c1b-9a43-3f1f3c0f9b11";

    fn entry(path: &str) -> ManifestEntry {
        ManifestEntry {
            path: path.into(),
            title: "Dune".into(),
            author: "Frank Herbert".into(),
            format: "azw3".into(),
            size_bytes: 3,
            sent_at: "2026-11-07T10:00:00+00:00".into(),
        }
    }

    fn generation_of(device: &FakeDevice, slot: &str) -> Option<u64> {
        let bytes = device.read(&format!(".librarium/{slot}"))?;
        serde_json::from_slice::<Value>(&bytes)
            .ok()?
            .get(GENERATION)?
            .as_u64()
    }

    #[test]
    fn writes_alternate_between_the_two_copies() {
        let device = FakeDevice::kindle();
        let mut s = device.session();
        let mut fs = DeviceFs::default();
        assert_eq!(
            load(&mut fs, &mut s).unwrap().loaded.state,
            ManifestState::Absent
        );

        update(&mut fs, &mut s, |m| {
            m.books.insert(ID.into(), entry("documents/Dune.azw3"))
        })
        .unwrap();
        assert_eq!(
            (
                generation_of(&device, SLOTS[0]),
                generation_of(&device, SLOTS[1])
            ),
            (Some(1), None)
        );
        update(&mut fs, &mut s, |m| {
            m.books.get_mut(ID).unwrap().title = "Dune (2)".into()
        })
        .unwrap();
        assert_eq!(
            (
                generation_of(&device, SLOTS[0]),
                generation_of(&device, SLOTS[1])
            ),
            (Some(1), Some(2))
        );
        update(&mut fs, &mut s, |m| {
            m.books.get_mut(ID).unwrap().title = "Dune (3)".into()
        })
        .unwrap();
        assert_eq!(
            (
                generation_of(&device, SLOTS[0]),
                generation_of(&device, SLOTS[1])
            ),
            (Some(3), Some(2))
        );

        let stored = load(&mut fs, &mut s).unwrap();
        assert_eq!(stored.loaded.state, ManifestState::Valid);
        assert_eq!(stored.manifest().entry(ID).unwrap().title, "Dune (3)");
        assert_eq!(
            device
                .paths()
                .iter()
                .filter(|p| p.starts_with(".librarium/"))
                .count(),
            2,
            "never more than two copies"
        );
    }

    #[test]
    fn an_interrupted_write_falls_back_to_the_previous_copy() {
        let device = FakeDevice::kindle();
        let mut s = device.session();
        let mut fs = DeviceFs::default();
        update(&mut fs, &mut s, |m| {
            m.books.insert(ID.into(), entry("documents/Dune.azw3"))
        })
        .unwrap();
        // The next write was cut off half-way.
        device.put(
            ".librarium/device-2.json",
            br#"{"version":1,"generation":2,"books":{"#,
        );
        let stored = load(&mut fs, &mut s).unwrap();
        assert_eq!(stored.loaded.state, ManifestState::Valid);
        assert!(stored.manifest().entry(ID).is_some());
        // And the next write replaces the broken copy.
        update(&mut fs, &mut s, |m| m.books.clear()).unwrap();
        assert_eq!(generation_of(&device, SLOTS[1]), Some(2));
        assert!(load(&mut fs, &mut s).unwrap().manifest().books.is_empty());
    }

    #[test]
    fn only_damaged_copies_read_as_damaged() {
        let device = FakeDevice::kindle();
        device.put(".librarium/device-1.json", b"garbage");
        let mut s = device.session();
        let mut fs = DeviceFs::default();
        assert!(matches!(
            load(&mut fs, &mut s).unwrap().loaded.state,
            ManifestState::Damaged(_)
        ));
        assert!(
            ensure_device_id(&mut fs, &mut s).is_err(),
            "not rewritten just for an id"
        );
        update(&mut fs, &mut s, |m| {
            m.books.insert(ID.into(), entry("documents/Dune.azw3"))
        })
        .unwrap();
        assert_eq!(
            load(&mut fs, &mut s).unwrap().loaded.state,
            ManifestState::Valid
        );
    }

    #[test]
    fn a_newer_manifest_is_never_rewritten() {
        let device = FakeDevice::kindle();
        let newer = br#"{"version":9,"generation":4,"deviceId":"dev-9","books":{}}"#;
        device.put(".librarium/device-2.json", newer);
        let mut s = device.session();
        let mut fs = DeviceFs::default();
        assert_eq!(
            load(&mut fs, &mut s).unwrap().loaded.state,
            ManifestState::TooNew(9)
        );
        assert!(
            matches!(update(&mut fs, &mut s, |m| m.device_id = None), Err(MtpError::Refused(m)) if m.contains("newer Librarium"))
        );
        assert_eq!(
            device.read(".librarium/device-2.json").unwrap(),
            newer.to_vec()
        );
        assert_eq!(ensure_device_id(&mut fs, &mut s).unwrap(), "dev-9");
    }

    #[test]
    fn the_device_id_is_created_once() {
        let device = FakeDevice::kindle();
        let mut s = device.session();
        let mut fs = DeviceFs::default();
        let id = ensure_device_id(&mut fs, &mut s).unwrap();
        assert!(uuid::Uuid::parse_str(&id).is_ok());
        assert_eq!(
            ensure_device_id(&mut DeviceFs::default(), &mut s).unwrap(),
            id,
            "a new connection reads it back"
        );
        assert_eq!(
            device.paths(),
            vec![".librarium", ".librarium/device-1.json", "documents"]
        );
    }
}
