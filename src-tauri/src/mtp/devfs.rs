//! Paths on an MTP device. Librarium names device files by their path
//! relative to the device root — `documents/SF/Dune.azw3`, as on a
//! mass-storage Kindle and in the device manifest — while MTP knows only
//! opaque object ids, folder by folder. [`DeviceFs`] maps one onto the
//! other, with a short-lived cache of folder listings.
//!
//! - **Storages.** Every storage is used. The *primary* one (the first
//!   writable storage with a `documents` folder; a Kindle has one storage)
//!   has plain paths; the others' paths start with `storage-<id>/`.
//!   TODO(device-validation): confirm a 2024 Kindle shows one storage with
//!   `documents/` at its root over MTP, and that books placed there (or in
//!   subfolders) are indexed like on a USB drive.
//! - **Names** are compared like on FAT: case-insensitively, Unicode-NFC
//!   ([`path_key`]), so `Documents` is `documents`.
//! - **Cache.** A folder's listing is kept for [`LISTING_TTL`] and updated
//!   in place by Librarium's own changes ([`DeviceFs::note_added`] and
//!   co.), so a batch of transfers lists each folder once. Listing the books
//!   forces a fresh listing ([`DeviceFs::invalidate`]). A 2024 Kindle stays
//!   usable while plugged in over MTP, so the device can change behind the
//!   cache: before anything is deleted, replaced or copied, the cached
//!   object is checked with one GetObjectInfo ([`DeviceFs::resolve_checked`],
//!   [`DeviceFs::find_child`]) and looked up again freshly if it changed.
//!   (Listing `documents/` freshly instead would cost one request per book,
//!   for every book sent.)
//!
//! The cache belongs to one session: the worker drops it with the session.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::session::{MtpError, MtpSession, Object, ObjectId, Storage, StorageId};
use crate::commands::device::manifest::path_key;

/// The folder a Kindle reads books from.
pub(crate) const DOCUMENTS: &str = "documents";

/// How long a folder listing is trusted.
pub(crate) const LISTING_TTL: Duration = Duration::from_secs(30);

/// How deep walks go (as on mass storage).
pub(crate) const MAX_DEPTH: usize = 12;

/// A folder to list or write into: `id` `None` is the storage root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Folder {
    pub storage: StorageId,
    pub id: Option<ObjectId>,
}

impl Folder {
    pub(crate) fn of(object: &Object) -> Self {
        Folder {
            storage: object.storage.clone(),
            id: Some(object.id.clone()),
        }
    }
}

/// One storage and its `documents` folder.
#[derive(Debug, Clone)]
pub(crate) struct Volume {
    pub storage: Storage,
    /// "" for the primary storage, `storage-<id>/` for the others.
    pub prefix: String,
    pub documents: Option<Object>,
}

/// The device's storages (see the module docs).
#[derive(Debug, Clone)]
pub(crate) struct Layout {
    pub volumes: Vec<Volume>,
    pub primary: usize,
}

impl Layout {
    pub(crate) fn primary(&self) -> &Volume {
        &self.volumes[self.primary]
    }
}

/// Whether two device names are the same file name (see the module docs).
pub(crate) fn same_name(a: &str, b: &str) -> bool {
    path_key(a) == path_key(b)
}

/// `dir/name`, or `name` when `dir` is empty.
pub(crate) fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

/// The parent of a relative path (`""` at the root).
pub(crate) fn parent_of(rel: &str) -> &str {
    rel.rsplit_once('/').map_or("", |(dir, _)| dir)
}

/// The last component of a relative path.
pub(crate) fn name_of(rel: &str) -> &str {
    rel.rsplit_once('/').map_or(rel, |(_, name)| name)
}

/// A storage id as a path component: letters and digits only.
fn storage_tag(id: &str) -> String {
    id.chars().filter(char::is_ascii_alphanumeric).collect()
}

/// See the module docs.
#[derive(Default)]
pub(crate) struct DeviceFs {
    layout: Option<Layout>,
    listings: HashMap<(StorageId, Option<ObjectId>), (Instant, Vec<Object>)>,
}

impl DeviceFs {
    /// Forget every listing and the layout: the next operations ask the device.
    pub(crate) fn invalidate(&mut self) {
        self.layout = None;
        self.listings.clear();
    }

    /// The storages and their `documents` folders (cached for the session;
    /// see [`Self::invalidate`]).
    pub(crate) fn layout(&mut self, s: &mut dyn MtpSession) -> Result<&Layout, MtpError> {
        if self.layout.is_none() {
            let storages = s.storages()?;
            if storages.is_empty() {
                // TODO(device-validation): check whether a locked Kindle hides
                // its storage over MTP, as a locked Android phone does.
                return Err(MtpError::Refused(
                    "The Kindle shows no storage. Unlock it, then unplug and replug it.".into(),
                ));
            }
            let mut volumes = Vec::new();
            for storage in storages {
                let root = self.children(s, &storage.id, None)?;
                let documents = root
                    .into_iter()
                    .find(|o| o.is_folder && same_name(&o.name, DOCUMENTS));
                volumes.push(Volume {
                    prefix: String::new(),
                    storage,
                    documents,
                });
            }
            let primary = volumes
                .iter()
                .position(|v| v.storage.writable && v.documents.is_some())
                .or_else(|| volumes.iter().position(|v| v.documents.is_some()))
                .or_else(|| volumes.iter().position(|v| v.storage.writable))
                .unwrap_or(0);
            for (i, v) in volumes.iter_mut().enumerate() {
                if i != primary {
                    v.prefix = format!("storage-{}/", storage_tag(&v.storage.id));
                }
            }
            self.layout = Some(Layout { volumes, primary });
        }
        Ok(self.layout.as_ref().expect("just set"))
    }

    /// The objects in `folder` (cached; see the module docs).
    pub(crate) fn children(
        &mut self,
        s: &mut dyn MtpSession,
        storage: &str,
        parent: Option<&str>,
    ) -> Result<Vec<Object>, MtpError> {
        let key = (storage.to_string(), parent.map(str::to_string));
        if let Some((at, objects)) = self.listings.get(&key) {
            if at.elapsed() < LISTING_TTL {
                return Ok(objects.clone());
            }
        }
        let objects = s.children(storage, parent)?;
        self.listings.insert(key, (Instant::now(), objects.clone()));
        Ok(objects)
    }

    /// [`Self::children`] of a [`Folder`].
    pub(crate) fn list(
        &mut self,
        s: &mut dyn MtpSession,
        folder: &Folder,
    ) -> Result<Vec<Object>, MtpError> {
        self.children(s, &folder.storage, folder.id.as_deref())
    }

    /// Forget the listing of `folder` (the next one asks the device).
    pub(crate) fn forget(&mut self, folder: &Folder) {
        self.listings
            .remove(&(folder.storage.clone(), folder.id.clone()));
    }

    /// Which volume a relative path is on, and its components there.
    fn split<'r>(
        &mut self,
        s: &mut dyn MtpSession,
        rel: &'r str,
    ) -> Result<(Volume, Vec<&'r str>), MtpError> {
        let layout = self.layout(s)?;
        let parts: Vec<&str> = rel.split('/').filter(|p| !p.is_empty()).collect();
        if let Some(first) = parts.first() {
            if let Some(v) = layout
                .volumes
                .iter()
                .find(|v| !v.prefix.is_empty() && same_name(v.prefix.trim_end_matches('/'), first))
            {
                return Ok((v.clone(), parts[1..].to_vec()));
            }
        }
        Ok((layout.primary().clone(), parts))
    }

    /// The object at `rel`, or `None` if there is none. `fresh` re-lists
    /// every folder on the way (before deleting or replacing).
    pub(crate) fn resolve(
        &mut self,
        s: &mut dyn MtpSession,
        rel: &str,
        fresh: bool,
    ) -> Result<Option<Object>, MtpError> {
        let (volume, parts) = self.split(s, rel)?;
        let mut folder = Folder {
            storage: volume.storage.id.clone(),
            id: None,
        };
        let mut found = None;
        for (i, part) in parts.iter().enumerate() {
            if fresh {
                self.forget(&folder);
            }
            let Some(object) = self
                .list(s, &folder)?
                .into_iter()
                .find(|o| same_name(&o.name, part))
            else {
                return Ok(None);
            };
            if i + 1 < parts.len() && !object.is_folder {
                return Ok(None);
            }
            folder = Folder::of(&object);
            found = Some(object);
        }
        Ok(found)
    }

    /// Whether `object` (from a cached listing) is still on the device, with
    /// the same name in the same folder: one GetObjectInfo.
    pub(crate) fn still_there(
        &mut self,
        s: &mut dyn MtpSession,
        object: &Object,
    ) -> Result<bool, MtpError> {
        match s.object(&object.id) {
            Ok(now) => Ok(now.name == object.name
                && now.parent == object.parent
                && now.is_folder == object.is_folder),
            Err(e) if e.is_fatal() => Err(e),
            Err(_) => Ok(false),
        }
    }

    /// [`Self::resolve`] for something about to be deleted, replaced or
    /// copied: the cached answer is used only once [`Self::still_there`]
    /// confirms it; otherwise (or when the cache doesn't have it) the path
    /// is resolved again with fresh listings. Cheaper than always listing
    /// freshly: a `documents/` listing costs one request per book.
    pub(crate) fn resolve_checked(
        &mut self,
        s: &mut dyn MtpSession,
        rel: &str,
    ) -> Result<Option<Object>, MtpError> {
        if let Some(cached) = self.resolve(s, rel, false)? {
            if self.still_there(s, &cached)? {
                return Ok(Some(cached));
            }
        }
        self.resolve(s, rel, true)
    }

    /// The object named `name` in `folder` (any case), checked like
    /// [`Self::resolve_checked`] when the cache has one; and the folder's
    /// listing it was found in.
    pub(crate) fn find_child(
        &mut self,
        s: &mut dyn MtpSession,
        folder: &Folder,
        name: &str,
    ) -> Result<(Vec<Object>, Option<Object>), MtpError> {
        let listing = self.list(s, folder)?;
        match listing.iter().find(|o| same_name(&o.name, name)).cloned() {
            Some(found) if !self.still_there(s, &found)? => {
                self.forget(folder);
                let fresh = self.list(s, folder)?;
                let found = fresh.iter().find(|o| same_name(&o.name, name)).cloned();
                Ok((fresh, found))
            }
            found => Ok((listing, found)),
        }
    }

    /// The folder at `rel_dir`, created (with its parents) if missing. Each
    /// component found in the cache is checked; one that isn't there is
    /// looked for in a fresh listing before being created.
    pub(crate) fn ensure_dir(
        &mut self,
        s: &mut dyn MtpSession,
        rel_dir: &str,
    ) -> Result<Folder, MtpError> {
        let (volume, parts) = self.split(s, rel_dir)?;
        let mut folder = Folder {
            storage: volume.storage.id.clone(),
            id: None,
        };
        for part in parts {
            let (_, mut existing) = self.find_child(s, &folder, part)?;
            if existing.is_none() {
                self.forget(&folder);
                existing = self
                    .list(s, &folder)?
                    .into_iter()
                    .find(|o| same_name(&o.name, part));
            }
            let object = match existing {
                Some(o) if o.is_folder => o,
                Some(_) => {
                    return Err(MtpError::Refused(format!(
                        "A file named \"{part}\" on the Kindle is where Librarium needs a folder."
                    )))
                }
                None => {
                    let created = s.create_folder(&folder.storage, folder.id.as_deref(), part)?;
                    self.note_added(&created);
                    created
                }
            };
            folder = Folder::of(&object);
        }
        Ok(folder)
    }

    /// Every object under `start` (at most `depth` levels down), with its
    /// path (`start_rel/...`); folders are entered when `descend` says so.
    pub(crate) fn walk(
        &mut self,
        s: &mut dyn MtpSession,
        start: &Folder,
        start_rel: &str,
        depth: usize,
        descend: &dyn Fn(&Object) -> bool,
    ) -> Result<Vec<(String, Object)>, MtpError> {
        let mut out = Vec::new();
        let mut stack = vec![(start.clone(), start_rel.to_string(), 0usize)];
        while let Some((folder, rel, level)) = stack.pop() {
            for object in self.list(s, &folder)? {
                let path = join(&rel, &object.name);
                if object.is_folder && level + 1 < depth && descend(&object) {
                    stack.push((Folder::of(&object), path.clone(), level + 1));
                }
                out.push((path, object));
            }
        }
        out.sort_by_cached_key(|(rel, _)| path_key(rel));
        Ok(out)
    }

    /// Delete `object`; a folder's content first (MTP doesn't promise to
    /// delete a non-empty folder).
    pub(crate) fn delete_tree(
        &mut self,
        s: &mut dyn MtpSession,
        object: &Object,
    ) -> Result<(), MtpError> {
        self.delete_tree_at(s, object, 0)
    }

    fn delete_tree_at(
        &mut self,
        s: &mut dyn MtpSession,
        object: &Object,
        level: usize,
    ) -> Result<(), MtpError> {
        if object.is_folder {
            if level >= MAX_DEPTH {
                return Err(MtpError::Refused(
                    "A folder on the Kindle is nested too deeply to delete.".into(),
                ));
            }
            let folder = Folder::of(object);
            self.forget(&folder);
            for child in self.list(s, &folder)? {
                self.delete_tree_at(s, &child, level + 1)?;
            }
        }
        s.delete(&object.id)?;
        self.note_removed(object);
        Ok(())
    }

    /// Free and total bytes of `storage`, asked freshly.
    pub(crate) fn space(
        &mut self,
        s: &mut dyn MtpSession,
        storage: &str,
    ) -> Result<(Option<u64>, Option<u64>), MtpError> {
        let found = s.storages()?.into_iter().find(|st| st.id == storage);
        Ok(found.map_or((None, None), |st| (st.free_bytes, st.capacity_bytes)))
    }

    /// Record an object Librarium created.
    pub(crate) fn note_added(&mut self, object: &Object) {
        if let Some((_, list)) = self
            .listings
            .get_mut(&(object.storage.clone(), object.parent.clone()))
        {
            list.retain(|o| o.id != object.id);
            list.push(object.clone());
        }
    }

    /// Record an object Librarium deleted.
    pub(crate) fn note_removed(&mut self, object: &Object) {
        if let Some((_, list)) = self
            .listings
            .get_mut(&(object.storage.clone(), object.parent.clone()))
        {
            list.retain(|o| o.id != object.id);
        }
        self.listings
            .remove(&(object.storage.clone(), Some(object.id.clone())));
    }

    /// Record a rename Librarium made; the renamed object.
    pub(crate) fn note_renamed(&mut self, object: &Object, new_name: &str) -> Object {
        let renamed = Object {
            name: new_name.to_string(),
            ..object.clone()
        };
        self.note_added(&renamed);
        renamed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mtp::fake::FakeDevice;

    fn names(objects: &[(String, Object)]) -> Vec<&str> {
        objects.iter().map(|(rel, _)| rel.as_str()).collect()
    }

    #[test]
    fn the_layout_finds_documents_whatever_its_case() {
        let device = FakeDevice::blank();
        device.mkdir("Documents");
        let mut s = device.session();
        let mut fs = DeviceFs::default();
        let layout = fs.layout(&mut s).unwrap();
        assert_eq!(
            layout.primary().documents.as_ref().map(|d| d.name.as_str()),
            Some("Documents")
        );
        assert_eq!(layout.primary().prefix, "");
    }

    #[test]
    fn paths_resolve_like_on_fat() {
        let device = FakeDevice::kindle();
        device.put("documents/Sci-Fi/Café.azw3", b"x");
        let mut s = device.session();
        let mut fs = DeviceFs::default();
        let found = fs
            .resolve(&mut s, "DOCUMENTS/sci-fi/cafe\u{301}.AZW3", false)
            .unwrap()
            .unwrap();
        assert_eq!(found.name, "Café.azw3");
        assert!(fs
            .resolve(&mut s, "documents/Nope.azw3", false)
            .unwrap()
            .is_none());
        assert!(
            fs.resolve(&mut s, "documents/Sci-Fi/Café.azw3/inside", false)
                .unwrap()
                .is_none(),
            "a file has no children"
        );
    }

    #[test]
    fn listings_are_cached_and_kept_in_sync_with_our_changes() {
        let device = FakeDevice::kindle();
        device.put("documents/A.azw3", b"a");
        let mut s = device.session();
        let mut fs = DeviceFs::default();
        fs.resolve(&mut s, "documents/A.azw3", false)
            .unwrap()
            .unwrap();
        let before = device.state().listings;
        fs.resolve(&mut s, "documents/A.azw3", false)
            .unwrap()
            .unwrap();
        assert_eq!(device.state().listings, before, "served from the cache");

        let docs = fs.resolve(&mut s, "documents", false).unwrap().unwrap();
        let sent = s
            .send_file(
                &docs.storage,
                Some(&docs.id),
                "B.azw3",
                1,
                &mut &b"b"[..],
                &mut |_| true,
            )
            .unwrap();
        fs.note_added(&sent);
        assert!(
            fs.resolve(&mut s, "documents/B.azw3", false)
                .unwrap()
                .is_some(),
            "our own send is visible without a refetch"
        );
        assert_eq!(device.state().listings, before);

        // Changed behind our back: a fresh resolve sees it.
        device.put("documents/C.azw3", b"c");
        assert!(
            fs.resolve(&mut s, "documents/C.azw3", false)
                .unwrap()
                .is_none(),
            "stale, as expected"
        );
        assert!(
            fs.resolve(&mut s, "documents/C.azw3", true)
                .unwrap()
                .is_some(),
            "fresh"
        );
    }

    #[test]
    fn folders_are_created_once() {
        let device = FakeDevice::kindle();
        let mut s = device.session();
        let mut fs = DeviceFs::default();
        let a = fs.ensure_dir(&mut s, "documents/Sci-Fi/Herbert").unwrap();
        let b = fs.ensure_dir(&mut s, "documents/sci-fi/HERBERT").unwrap();
        assert_eq!(a, b);
        assert_eq!(
            device.paths(),
            vec!["documents", "documents/Sci-Fi", "documents/Sci-Fi/Herbert"]
        );
        device.put("documents/Blocked", b"file");
        assert!(matches!(
            fs.ensure_dir(&mut s, "documents/Blocked/x"),
            Err(MtpError::Refused(_))
        ));
    }

    #[test]
    fn a_device_without_documents_gets_it_on_first_use() {
        let device = FakeDevice::blank();
        let mut s = device.session();
        let mut fs = DeviceFs::default();
        assert!(fs.layout(&mut s).unwrap().primary().documents.is_none());
        fs.ensure_dir(&mut s, DOCUMENTS).unwrap();
        assert_eq!(device.paths(), vec!["documents"]);
    }

    #[test]
    fn walks_are_bounded_and_filtered() {
        let device = FakeDevice::kindle();
        device.put("documents/A.azw3", b"a");
        device.put("documents/A.sdr/A.mbs", b"m");
        device.put("documents/Shelf/B.epub", b"b");
        let mut s = device.session();
        let mut fs = DeviceFs::default();
        let docs = fs.resolve(&mut s, "documents", false).unwrap().unwrap();
        let all = fs
            .walk(&mut s, &Folder::of(&docs), "documents", MAX_DEPTH, &|_| {
                true
            })
            .unwrap();
        assert_eq!(
            names(&all),
            vec![
                "documents/A.azw3",
                "documents/A.sdr",
                "documents/A.sdr/A.mbs",
                "documents/Shelf",
                "documents/Shelf/B.epub"
            ]
        );
        let no_sdr = fs
            .walk(&mut s, &Folder::of(&docs), "documents", MAX_DEPTH, &|o| {
                !o.name.ends_with(".sdr")
            })
            .unwrap();
        assert_eq!(
            names(&no_sdr),
            vec![
                "documents/A.azw3",
                "documents/A.sdr",
                "documents/Shelf",
                "documents/Shelf/B.epub"
            ]
        );
        let shallow = fs
            .walk(&mut s, &Folder::of(&docs), "documents", 1, &|_| true)
            .unwrap();
        assert_eq!(
            names(&shallow),
            vec!["documents/A.azw3", "documents/A.sdr", "documents/Shelf"]
        );
    }

    #[test]
    fn folders_are_deleted_with_their_content() {
        let device = FakeDevice::kindle();
        device.put("documents/A.sdr/A.mbs", b"m");
        device.put("documents/A.sdr/sub/x.db", b"d");
        let mut s = device.session();
        let mut fs = DeviceFs::default();
        let sdr = fs
            .resolve(&mut s, "documents/A.sdr", false)
            .unwrap()
            .unwrap();
        fs.delete_tree(&mut s, &sdr).unwrap();
        assert_eq!(device.paths(), vec!["documents"]);
        assert!(
            fs.resolve(&mut s, "documents/A.sdr", false)
                .unwrap()
                .is_none(),
            "the cache knows"
        );
    }

    #[test]
    fn path_helpers() {
        assert_eq!(join("", "documents"), "documents");
        assert_eq!(join("documents", "A.azw3"), "documents/A.azw3");
        assert_eq!(parent_of("documents/SF/A.azw3"), "documents/SF");
        assert_eq!(parent_of("A.azw3"), "");
        assert_eq!(name_of("documents/SF/A.azw3"), "A.azw3");
        assert_eq!(storage_tag("s1/0:01"), "s1001");
    }
}
