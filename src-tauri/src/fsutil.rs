//! Filesystem helpers shared by every command that writes user data.
//!
//! Libraries live on local disks *and* on NAS shares, and devices are FAT/exFAT
//! volumes that can be unplugged at any moment. Every write of user data must
//! therefore be crash-safe and serialised:
//!
//! - [`atomic_write`] / [`atomic_write_with`]: write to a temp file in the same
//!   directory, fsync, then rename over the target. A crash leaves either the
//!   old file or the new one — never a truncated mix.
//! - [`sync_file`]: the one way to fsync a file, also on volumes that refuse
//!   macOS's "full" fsync (SMB shares, FAT e-readers).
//! - [`with_file_lock`]: serialise writers of the same path inside this
//!   process (e.g. a metadata save and a cover update on the same EPUB).
//! - [`move_to_trash`]: delete recoverably — OS trash first, then a
//!   `.librarium/trash/` folder for volumes without one (most SMB shares).
//!   Never asks anything, never deletes for good.
//! - [`sanitize_filename`]: names that are valid on FAT32/exFAT/NTFS/APFS/ext4.
//! - [`is_within`]: confine paths received from the webview to a root folder.
//!
//! All functions here are blocking; call them from `spawn_blocking` (or other
//! synchronous code), never directly on the async runtime in hot paths.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

// ── Atomic writes ────────────────────────────────────────────────────────────

/// Atomically replace `path` with `data` (see [`atomic_write_with`]).
pub fn atomic_write(path: &Path, data: &[u8]) -> io::Result<()> {
    atomic_write_with(path, |file| file.write_all(data))
}

/// Atomically replace `path` with whatever `write` produces.
///
/// The content is written to a hidden `.part` temp file next to `path` (same
/// directory, hence same volume, so the final rename is atomic), flushed to
/// disk, given the original file's permissions, and renamed over `path`.
/// If `write` fails, the temp file is removed and `path` is untouched.
pub fn atomic_write_with<F>(path: &Path, write: F) -> io::Result<()>
where
    F: FnOnce(&mut File) -> io::Result<()>,
{
    let dir = parent_dir(path);
    let mut tmp = tempfile::Builder::new()
        .prefix(".librarium-")
        .suffix(".part")
        .tempfile_in(dir)?;
    write(tmp.as_file_mut())?;
    sync_file(tmp.as_file())?;
    // tempfile creates files with owner-only permissions (0600). Keep the
    // original file's mode, or use the usual 0644 for a new file, so other
    // users of a shared library (or the NAS's own services) can still read it.
    match fs::metadata(path) {
        Ok(meta) => {
            let _ = fs::set_permissions(tmp.path(), meta.permissions());
        }
        Err(_) => set_default_file_mode(tmp.path()),
    }
    tmp.persist(path).map_err(|e| e.error)?;
    sync_dir(dir);
    Ok(())
}

/// Copy `src` to `dest` atomically: `dest` only appears once fully written.
pub fn atomic_copy(src: &Path, dest: &Path) -> io::Result<u64> {
    let mut copied = 0;
    atomic_write_with(dest, |out| {
        let mut input = File::open(src)?;
        copied = io::copy(&mut input, out)?;
        Ok(())
    })?;
    Ok(copied)
}

/// Give a newly created file the conventional `rw-r--r--` mode (Unix only;
/// Windows ACLs are inherited from the folder).
fn set_default_file_mode(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o644));
    }
    #[cfg(not(unix))]
    let _ = path;
}

fn parent_dir(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

/// Make a rename durable by syncing the directory entry. Best effort: not
/// supported on Windows or on some network filesystems.
fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    if let Ok(d) = File::open(dir) {
        let _ = sync_file(&d);
    }
    #[cfg(not(unix))]
    let _ = dir;
}

// ── Flushing to disk ─────────────────────────────────────────────────────────

/// Flush `file`'s content (and metadata) to the storage device. Use this
/// instead of `File::sync_all` everywhere.
///
/// On Apple platforms `sync_all` is `fcntl(F_FULLFSYNC)`, which also flushes
/// the drive's own cache — and which some filesystems simply don't
/// implement: smbfs (a NAS library), msdosfs/exfat (a Kindle or Kobo), some
/// FUSE and AFP volumes answer `ENOTSUP`, `ENOTTY` or `EINVAL`. With no
/// fallback, every write there would fail (index saves, covers, imports,
/// EPUB write-back, transfers). When that happens the file is flushed with
/// plain `fsync(2)`, which is what those filesystems do support; if even
/// that is "unsupported", the volume can't flush on request at all and the
/// write stands as it is (refusing it would make the library read-only).
///
/// Elsewhere this is `sync_all`.
pub fn sync_file(file: &File) -> io::Result<()> {
    #[cfg(target_vendor = "apple")]
    {
        match file.sync_all() {
            Err(e) if sync_unsupported(&e) => plain_fsync(file),
            other => other,
        }
    }
    #[cfg(not(target_vendor = "apple"))]
    {
        file.sync_all()
    }
}

/// Whether a failed sync means "this filesystem doesn't do that kind of
/// sync" rather than a real I/O error (which must fail the write): `ENOTSUP`
/// / `EOPNOTSUPP`, `ENOTTY` (no such `fcntl` for this file), `EINVAL`
/// (the descriptor doesn't support syncing), or anything Rust already
/// classifies as [`io::ErrorKind::Unsupported`]. (Compiled on every Unix for
/// the tests; only Apple platforms need it.)
#[cfg(all(unix, any(target_vendor = "apple", test)))]
fn sync_unsupported(err: &io::Error) -> bool {
    if err.kind() == io::ErrorKind::Unsupported {
        return true;
    }
    matches!(
        err.raw_os_error(),
        Some(code) if code == libc::ENOTSUP
            || code == libc::EOPNOTSUPP
            || code == libc::ENOTTY
            || code == libc::EINVAL
    )
}

/// `fsync(2)`, for volumes that refused `F_FULLFSYNC` ([`sync_file`]).
#[cfg(target_vendor = "apple")]
fn plain_fsync(file: &File) -> io::Result<()> {
    use std::os::unix::io::AsRawFd;
    // SAFETY: `fsync` only reads the descriptor, which `file` keeps open for
    // the duration of the call.
    let result = unsafe { libc::fsync(file.as_raw_fd()) };
    if result == 0 {
        return Ok(());
    }
    let err = io::Error::last_os_error();
    if sync_unsupported(&err) {
        // The volume can't flush on request at all: the data was written,
        // and closing the file hands it to the server.
        Ok(())
    } else {
        Err(err)
    }
}

// ── Per-path locks ───────────────────────────────────────────────────────────

type LockMap = Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>;

fn locks() -> &'static LockMap {
    static LOCKS: OnceLock<LockMap> = OnceLock::new();
    LOCKS.get_or_init(Default::default)
}

/// Key used to identify a file across differently spelled paths.
fn lock_key(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Run `f` while holding this process's lock for `path`.
///
/// Every read-modify-write of a user file (EPUB metadata, cover embedding,
/// the library index) must go through this so concurrent commands can't
/// interleave and silently drop each other's changes. The lock is blocking:
/// call this from synchronous code or `spawn_blocking`.
pub fn with_file_lock<T>(path: &Path, f: impl FnOnce() -> T) -> T {
    let key = lock_key(path);
    let lock = {
        let mut map = locks().lock().unwrap_or_else(|e| e.into_inner());
        map.entry(key.clone()).or_default().clone()
    };
    let result = {
        let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
        f()
    };
    // Drop the map entry once nobody else holds or waits for it.
    let mut map = locks().lock().unwrap_or_else(|e| e.into_inner());
    if Arc::strong_count(&lock) == 2 {
        map.remove(&key);
    }
    result
}

/// Whether some thread holds [`with_file_lock`]'s lock for `path` right now.
/// Tests use it to check that slow work runs *outside* a lock.
#[cfg(test)]
pub fn is_file_locked(path: &Path) -> bool {
    let lock = locks()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&lock_key(path))
        .cloned();
    lock.is_some_and(|lock| lock.try_lock().is_err())
}

// ── Recoverable deletion ─────────────────────────────────────────────────────

/// Where [`move_to_trash`] put a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrashOutcome {
    /// The OS trash (macOS Trash, Windows Recycle Bin, freedesktop trash).
    SystemTrash,
    /// The fallback folder, for volumes without a usable OS trash.
    LibraryTrash(PathBuf),
}

/// Name of the fallback trash folder, relative to the library root.
pub const LIBRARY_TRASH_DIR: &str = ".librarium/trash";

/// Move `path` to the OS trash, or — when the volume has none (typical for
/// SMB/NFS shares) — into `<fallback_root>/.librarium/trash/<timestamp>/`.
/// Either way the file stays recoverable; nothing is deleted permanently.
///
/// Blocking, and possibly slow on a NAS: never call it while holding the
/// library index lock (commands resolve paths under the lock, trash outside
/// it, then lock again to update the index).
pub fn move_to_trash(path: &Path, fallback_root: &Path) -> io::Result<TrashOutcome> {
    move_to_trash_with(path, fallback_root, |p| {
        system_trash_context().delete(p).map_err(|e| e.to_string())
    })
}

/// How the OS trash is asked. On macOS, `NSFileManager.trashItemAtURL`
/// rather than the `trash` crate's default, which runs an AppleScript
/// telling Finder to delete the file: that needs an Automation permission
/// (a prompt, the first time), has no timeout, and — on a volume without a
/// Trash, like most SMB shares — makes Finder ask "delete immediately?",
/// where a confirmation deletes the file for good while the call reports
/// success. `NSFileManager` never asks anything: on such a volume it fails
/// at once, and the library's own trash folder takes the file instead. (The
/// price: Finder's "Put Back" may not be offered for those files; dragging
/// them out of the Trash works.)
#[cfg(target_os = "macos")]
fn system_trash_context() -> trash::TrashContext {
    use trash::macos::{DeleteMethod, TrashContextExtMacos};
    let mut context = trash::TrashContext::default();
    context.set_delete_method(DeleteMethod::NsFileManager);
    context
}

/// How the OS trash is asked: the `trash` crate's defaults (the Recycle Bin
/// through the shell, without UI; the freedesktop trash).
#[cfg(not(target_os = "macos"))]
fn system_trash_context() -> trash::TrashContext {
    trash::TrashContext::default()
}

/// [`move_to_trash`] with an injectable OS-trash implementation (for tests).
///
/// The OS trash's answer isn't taken on trust: the outcome is
/// [`TrashOutcome::SystemTrash`] only once `path` is really gone (checked
/// with `symlink_metadata`, so neither a dangling link nor an unreadable
/// share passes for "gone"). A call that reports success while the file is
/// still there falls back to the library's trash folder, like a failed one.
pub fn move_to_trash_with(
    path: &Path,
    fallback_root: &Path,
    system_trash: impl FnOnce(&Path) -> Result<(), String>,
) -> io::Result<TrashOutcome> {
    if !still_there(path)? {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} not found", path.display()),
        ));
    }
    let reported = system_trash(path);
    if !still_there(path)? {
        // Gone through the OS trash call — even one that reported an error
        // after moving it: the file isn't in the library any more either way.
        return Ok(TrashOutcome::SystemTrash);
    }
    if let Err(e) = &reported {
        log::debug!(
            "OS trash refused {} ({e}); using the library's trash folder",
            path.display()
        );
    }

    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let dir = fallback_root.join(LIBRARY_TRASH_DIR).join(stamp);
    fs::create_dir_all(&dir)?;
    let name = path
        .file_name()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("file"));
    let dest = unique_path(&dir, &name.to_string_lossy());
    if fs::rename(path, &dest).is_err() {
        // Different volume: copy, then remove the original.
        atomic_copy(path, &dest)?;
        fs::remove_file(path)?;
    }
    Ok(TrashOutcome::LibraryTrash(dest))
}

/// Whether something (a file, a folder, a link — even a dangling one) is at
/// `path`. Unlike `Path::exists`, an error other than "not found" (a share
/// that stopped answering, a permission problem) is an error, not `false`.
fn still_there(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

// ── File names ───────────────────────────────────────────────────────────────

/// Characters rejected by FAT32/exFAT/NTFS (and `/` everywhere).
const RESERVED_CHARS: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

/// Windows device names that can't be used as a file stem, whatever the extension.
const RESERVED_STEMS: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Maximum file name length in bytes. 255 is the common limit; staying well
/// below leaves room for `_2`-style suffixes and multi-byte characters.
pub const MAX_FILENAME_BYTES: usize = 200;

/// Make a file stem (name without extension) safe on every common filesystem:
/// reserved and control characters become `_`, whitespace runs collapse to one
/// space, leading/trailing spaces and trailing dots are removed, Windows device
/// names get a `_` prefix, and the result is cut to `max_bytes` on a character
/// boundary. Never returns an empty string.
pub fn sanitize_stem(stem: &str, max_bytes: usize) -> String {
    let mut out = String::with_capacity(stem.len());
    let mut last_space = false;
    for c in stem.chars() {
        // Whitespace first: tabs/newlines are control characters too, but
        // they should read as a space, not an underscore.
        let c = if c.is_whitespace() {
            ' '
        } else if RESERVED_CHARS.contains(&c) || c.is_control() {
            '_'
        } else {
            c
        };
        if c == ' ' {
            if !last_space {
                out.push(' ');
            }
            last_space = true;
        } else {
            out.push(c);
            last_space = false;
        }
    }
    let mut out = out.trim().trim_end_matches('.').trim_end().to_string();
    if RESERVED_STEMS.iter().any(|r| r.eq_ignore_ascii_case(&out)) {
        out.insert(0, '_');
    }
    truncate_bytes(&mut out, max_bytes);
    let out = out.trim_end().trim_end_matches('.').to_string();
    if out.is_empty() {
        "untitled".to_string()
    } else {
        out
    }
}

/// [`sanitize_stem`] applied to a full file name, keeping its extension.
pub fn sanitize_filename(name: &str) -> String {
    let (stem, ext) = split_extension(name);
    let ext: String = ext
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_lowercase();
    let budget = MAX_FILENAME_BYTES.saturating_sub(ext.len() + 1);
    let stem = sanitize_stem(stem, budget);
    if ext.is_empty() {
        stem
    } else {
        format!("{stem}.{ext}")
    }
}

/// Split `"Title.epub"` into `("Title", "epub")`. Names without a dot (or
/// starting with one, like `.hidden`) have no extension.
fn split_extension(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() => (&name[..i], &name[i + 1..]),
        _ => (name, ""),
    }
}

fn truncate_bytes(s: &mut String, max_bytes: usize) {
    if s.len() <= max_bytes {
        return;
    }
    let mut cut = max_bytes;
    while !s.is_char_boundary(cut) {
        cut -= 1;
    }
    s.truncate(cut);
}

/// `dir/name`, or `dir/stem_2.ext`, `dir/stem_3.ext`… if that already exists.
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(unique_name(name, |candidate| dir.join(candidate).exists()))
}

/// `name`, or the first of `stem_2.ext`, `stem_3.ext`… for which `taken`
/// is false: [`unique_path`] for places that aren't a local folder (an MTP
/// device's folder listing).
pub fn unique_name(name: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(name) {
        return name.to_string();
    }
    let (stem, ext) = split_extension(name);
    let ext = if ext.is_empty() {
        String::new()
    } else {
        format!(".{ext}")
    };
    (2..)
        .map(|i| format!("{stem}_{i}{ext}"))
        .find(|candidate| !taken(candidate))
        .expect("unbounded range always finds a free name")
}

// ── Path confinement ─────────────────────────────────────────────────────────

/// Whether `path` is `root` itself or lies inside it, after resolving `..`,
/// symlinks and case differences the filesystem resolves. A path that doesn't
/// exist yet is checked through its nearest existing ancestor.
///
/// Use this on every path that comes from the webview before writing or
/// deleting, so a buggy (or compromised) frontend can't touch files outside
/// the library or the device.
pub fn is_within(root: &Path, path: &Path) -> bool {
    let Ok(root) = fs::canonicalize(root) else {
        return false;
    };
    let mut existing = path.to_path_buf();
    let mut rest = Vec::new();
    while !existing.exists() {
        match (existing.file_name(), existing.parent()) {
            (Some(name), Some(parent)) => {
                rest.push(name.to_os_string());
                existing = parent.to_path_buf();
            }
            _ => return false,
        }
    }
    let Ok(mut resolved) = fs::canonicalize(&existing) else {
        return false;
    };
    for name in rest.into_iter().rev() {
        if name == ".." || name == "." {
            return false;
        }
        resolved.push(name);
    }
    resolved.starts_with(&root)
}

// ── Tests ────────────────────────────────────────────────────────────────────

/// Read up to `buf.len()` bytes, looping over short reads.
pub fn fill(reader: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(filled)
}

/// Byte-for-byte comparison of two files. Returns false on any read error.
pub fn files_identical(a: &Path, b: &Path) -> bool {
    let (Ok(ma), Ok(mb)) = (fs::metadata(a), fs::metadata(b)) else {
        return false;
    };
    if ma.len() != mb.len() {
        return false;
    }
    let (Ok(fa), Ok(fb)) = (File::open(a), File::open(b)) else {
        return false;
    };
    let mut ra = io::BufReader::new(fa);
    let mut rb = io::BufReader::new(fb);
    let mut ba = [0u8; 16 * 1024];
    let mut bb = [0u8; 16 * 1024];
    loop {
        let (Ok(na), Ok(nb)) = (fill(&mut ra, &mut ba), fill(&mut rb, &mut bb)) else {
            return false;
        };
        if na != nb || ba[..na] != bb[..nb] {
            return false;
        }
        if na == 0 {
            return true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_identical_compares_contents_not_names() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path();
        let a = dir.join("a.epub");
        let b = dir.join("b.epub");
        let c = dir.join("c.epub");
        // Larger than the 16 KiB read buffer, to exercise the chunk loop.
        let body = vec![7u8; 40_000];
        fs::write(&a, &body).unwrap();
        fs::write(&b, &body).unwrap();
        let mut other = body.clone();
        *other.last_mut().unwrap() = 9; // same length, last byte differs
        fs::write(&c, &other).unwrap();

        assert!(files_identical(&a, &b));
        assert!(!files_identical(&a, &c));
        assert!(!files_identical(&a, &dir.join("missing.epub")));
    }

    #[test]
    fn atomic_write_replaces_content_and_leaves_no_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("books.json");
        atomic_write(&target, b"old").unwrap();
        atomic_write(&target, b"new").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new");
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("books.json")]);
    }

    #[test]
    fn failed_atomic_write_keeps_the_original() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("book.epub");
        fs::write(&target, b"original").unwrap();
        let err = atomic_write_with(&target, |f| {
            f.write_all(b"partial")?;
            Err(io::Error::other("disk full"))
        });
        assert!(err.is_err());
        assert_eq!(fs::read(&target).unwrap(), b"original");
        assert_eq!(
            fs::read_dir(dir.path()).unwrap().count(),
            1,
            "temp file must be cleaned up"
        );
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_keeps_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("shared.epub");
        fs::write(&target, b"x").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o644)).unwrap();
        atomic_write(&target, b"y").unwrap();
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o644
        );

        let fresh = dir.path().join("new.json");
        atomic_write(&fresh, b"{}").unwrap();
        assert_eq!(
            fs::metadata(&fresh).unwrap().permissions().mode() & 0o777,
            0o644,
            "new files aren't owner-only"
        );
    }

    #[test]
    fn sync_file_flushes_a_regular_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut file = File::create(dir.path().join("x")).unwrap();
        file.write_all(b"data").unwrap();
        sync_file(&file).unwrap();
    }

    /// Which failed syncs fall back to plain `fsync` (or are accepted) on
    /// Apple platforms, and which fail the write.
    #[cfg(unix)]
    #[test]
    fn only_unsupported_syncs_fall_back() {
        let os = io::Error::from_raw_os_error;
        for code in [libc::ENOTSUP, libc::EOPNOTSUPP, libc::ENOTTY, libc::EINVAL] {
            assert!(sync_unsupported(&os(code)), "errno {code}");
        }
        assert!(sync_unsupported(&io::Error::from(
            io::ErrorKind::Unsupported
        )));
        for code in [
            libc::EIO,
            libc::ENOSPC,
            libc::EDQUOT,
            libc::EBADF,
            libc::EINTR,
        ] {
            assert!(
                !sync_unsupported(&os(code)),
                "errno {code} is a real failure"
            );
        }
        assert!(!sync_unsupported(&io::Error::other("disk on fire")));
    }

    #[test]
    fn file_lock_serialises_writers() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("counter");
        fs::write(&target, b"0").unwrap();
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let target = target.clone();
                std::thread::spawn(move || {
                    for _ in 0..25 {
                        with_file_lock(&target, || {
                            let n: u32 = fs::read_to_string(&target).unwrap().parse().unwrap();
                            atomic_write(&target, (n + 1).to_string().as_bytes()).unwrap();
                        });
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(fs::read_to_string(&target).unwrap(), "200");
        assert!(
            !locks().lock().unwrap().contains_key(&lock_key(&target)),
            "lock entry is released"
        );
    }

    #[test]
    fn trash_falls_back_to_library_folder() {
        let dir = tempfile::tempdir().unwrap();
        let book = dir.path().join("Dune.epub");
        fs::write(&book, b"book").unwrap();
        let outcome =
            move_to_trash_with(&book, dir.path(), |_| Err("no trash on this volume".into()))
                .unwrap();
        let TrashOutcome::LibraryTrash(dest) = outcome else {
            panic!("expected fallback")
        };
        assert!(!book.exists());
        assert!(dest.starts_with(dir.path().join(LIBRARY_TRASH_DIR)));
        assert_eq!(fs::read(&dest).unwrap(), b"book");
    }

    #[test]
    fn trash_reports_system_trash_when_it_worked() {
        let dir = tempfile::tempdir().unwrap();
        let book = dir.path().join("a.pdf");
        fs::write(&book, b"x").unwrap();
        let outcome = move_to_trash_with(&book, dir.path(), |p| {
            fs::remove_file(p).map_err(|e| e.to_string())
        });
        assert_eq!(outcome.unwrap(), TrashOutcome::SystemTrash);
    }

    /// An OS trash that says "done" but left the file in place (Finder
    /// waiting on a dialog, a volume that ignored the request) must not be
    /// reported as trashed: the library's own trash takes the file.
    #[test]
    fn a_trash_that_reports_success_but_left_the_file_is_not_trusted() {
        let dir = tempfile::tempdir().unwrap();
        let book = dir.path().join("Dune.epub");
        fs::write(&book, b"book").unwrap();
        let outcome = move_to_trash_with(&book, dir.path(), |_| Ok(())).unwrap();
        let TrashOutcome::LibraryTrash(dest) = outcome else {
            panic!("must not claim the OS trash took it")
        };
        assert!(!book.exists());
        assert_eq!(fs::read(&dest).unwrap(), b"book");
    }

    /// A dangling link is still something to trash (`Path::exists` says
    /// false for it, which would make the delete fail as "not found").
    #[cfg(unix)]
    #[test]
    fn trash_handles_a_dangling_link() {
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join("gone.epub");
        std::os::unix::fs::symlink(dir.path().join("nowhere.epub"), &link).unwrap();
        let outcome = move_to_trash_with(&link, dir.path(), |_| Err("no trash".into())).unwrap();
        assert!(matches!(outcome, TrashOutcome::LibraryTrash(_)));
        assert!(fs::symlink_metadata(&link).is_err());
    }

    #[test]
    fn trash_missing_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(move_to_trash_with(&dir.path().join("nope"), dir.path(), |_| Ok(())).is_err());
    }

    #[test]
    fn sanitize_replaces_reserved_and_control_characters() {
        assert_eq!(
            sanitize_filename("Dune: Messiah?.epub"),
            "Dune_ Messiah_.epub"
        );
        assert_eq!(
            sanitize_filename("a/b\\c|d*e\"f<g>h\u{7}.pdf"),
            "a_b_c_d_e_f_g_h_.pdf"
        );
        assert_eq!(
            sanitize_filename("  lots   of\tspace  .mobi"),
            "lots of space.mobi"
        );
    }

    #[test]
    fn sanitize_handles_windows_quirks() {
        assert_eq!(
            sanitize_filename("trailing dots....epub"),
            "trailing dots.epub"
        );
        assert_eq!(sanitize_filename("con.epub"), "_con.epub");
        assert_eq!(sanitize_filename("LPT1.pdf"), "_LPT1.pdf");
        assert_eq!(sanitize_filename("...epub"), "untitled.epub");
        assert_eq!(sanitize_filename(""), "untitled");
    }

    #[test]
    fn sanitize_keeps_unicode_and_lowercases_extension() {
        assert_eq!(
            sanitize_filename("L'œil de la veuve – Tome 6.EPUB"),
            "L'œil de la veuve – Tome 6.epub"
        );
    }

    #[test]
    fn sanitize_truncates_on_char_boundaries() {
        let long = "é".repeat(300) + ".epub";
        let out = sanitize_filename(&long);
        assert!(out.len() <= MAX_FILENAME_BYTES);
        assert!(out.ends_with(".epub"));
        assert!(out.trim_end_matches(".epub").chars().all(|c| c == 'é'));
    }

    #[test]
    fn unique_path_adds_numeric_suffix() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(unique_path(dir.path(), "a.epub"), dir.path().join("a.epub"));
        fs::write(dir.path().join("a.epub"), b"").unwrap();
        fs::write(dir.path().join("a_2.epub"), b"").unwrap();
        assert_eq!(
            unique_path(dir.path(), "a.epub"),
            dir.path().join("a_3.epub")
        );
    }

    #[test]
    fn is_within_confines_paths() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("library");
        fs::create_dir_all(root.join("Alice")).unwrap();
        fs::write(root.join("Alice/book.epub"), b"").unwrap();
        assert!(is_within(&root, &root.join("Alice/book.epub")));
        assert!(
            is_within(&root, &root.join("Alice/new.epub")),
            "non-existent file inside root"
        );
        assert!(!is_within(&root, &root.join("../outside.epub")));
        assert!(!is_within(&root, &root.join("Alice/../../outside.epub")));
        assert!(!is_within(&root, dir.path()));
        assert!(!is_within(&root.join("missing-root"), &root));
    }
}
