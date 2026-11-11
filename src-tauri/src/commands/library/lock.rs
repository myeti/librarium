//! `.librarium/lock.json`: noticing that another Librarium has the same
//! library open.
//!
//! A library on a NAS can be opened from two computers at once (or, before
//! the single-instance plugin, twice on the same one). Each instance edits
//! `library.json` with its own read-modify-write; the in-process lock doesn't
//! reach across computers, so simultaneous edits can overwrite each other.
//! The lock file doesn't *prevent* that — a crashed instance would otherwise
//! lock the library forever — it lets the app *warn* the user.
//!
//! ```json
//! { "instanceId": "<uuid of this app process>", "host": "studio-mac",
//!   "pid": 4242, "updatedAt": "<rfc3339>" }
//! ```
//!
//! - The file is (re)written when a library is opened ([`claim`]: scans and
//!   `library_lock_status`) and refreshed on every index save ([`refresh`]).
//! - Before overwriting it, the previous holder is remembered. If it was
//!   another instance *on another computer*, active less than
//!   [`FRESH_FOR_MINUTES`] ago, `library_lock_status` reports it. An entry
//!   written on this computer (same host name) by another instance is this
//!   computer's previous run — a crash, a quit whose cleanup didn't finish, a
//!   relaunch after an update — since the single-instance plugin (lib.rs)
//!   never lets two instances run side by side here (otherwise every restart
//!   would trigger the warning for five minutes). (Two macOS accounts using
//!   the same library on one Mac at the same time are not detected.)
//! - On exit, the files this instance still holds are removed
//!   ([`release_all`]), so quitting on one computer and opening on the other
//!   right away doesn't warn.
//!
//! Everything here is best effort: a read-only share or a failed write never
//! fails a command.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use super::index::INDEX_DIR;
use crate::fsutil;

/// Name of the lock file inside `.librarium/`.
pub(crate) const LOCK_FILE: &str = "lock.json";

/// How recent another instance's update must be to count as "still open".
pub(crate) const FRESH_FOR_MINUTES: i64 = 5;

/// The content of `lock.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LockInfo {
    pub(crate) instance_id: String,
    pub(crate) host: String,
    pub(crate) pid: u32,
    pub(crate) updated_at: String,
}

/// Another instance using the library, as reported to the frontend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OtherInstance {
    pub host: String,
    pub updated_at: String,
}

/// Result of `library_lock_status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LockStatus {
    pub other_instance: Option<OtherInstance>,
}

/// Who this process is: a random id per launch, the host name, the pid.
fn me(now: DateTime<Utc>) -> LockInfo {
    static INSTANCE_ID: OnceLock<String> = OnceLock::new();
    LockInfo {
        instance_id: INSTANCE_ID
            .get_or_init(|| uuid::Uuid::new_v4().to_string())
            .clone(),
        host: gethostname::gethostname().to_string_lossy().into_owned(),
        pid: std::process::id(),
        updated_at: now.to_rfc3339(),
    }
}

fn lock_path(library: &Path) -> PathBuf {
    library.join(INDEX_DIR).join(LOCK_FILE)
}

/// Per library: the last *other* instance seen in its lock file, and the
/// libraries whose lock file this process wrote (to clean up on exit).
#[derive(Default)]
struct Registry {
    others: HashMap<PathBuf, LockInfo>,
    held: HashSet<PathBuf>,
}

fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(Default::default)
}

fn read_lock(library: &Path) -> Option<LockInfo> {
    serde_json::from_str(&fs::read_to_string(lock_path(library)).ok()?).ok()
}

/// Whether the lock file entry `previous` was written by another Librarium
/// than `who` that may still be running: another instance on another
/// computer. One from this computer can only be a previous run (see the
/// module docs: the single-instance plugin).
fn is_other_instance(previous: &LockInfo, who: &LockInfo) -> bool {
    previous.instance_id != who.instance_id && previous.host != who.host
}

/// Write `who` into the lock file of `library`, first remembering the
/// previous holder if it was another instance ([`is_other_instance`]).
fn write_lock(library: &Path, who: &LockInfo) -> std::io::Result<()> {
    let previous = read_lock(library);
    {
        let mut reg = registry().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(other) = previous.filter(|p| is_other_instance(p, who)) {
            reg.others.insert(library.to_path_buf(), other);
        }
        reg.held.insert(library.to_path_buf());
    }
    let dir = library.join(INDEX_DIR);
    fs::create_dir_all(&dir)?;
    let json = serde_json::to_vec_pretty(who).map_err(std::io::Error::other)?;
    fsutil::atomic_write(&lock_path(library), &json)
}

/// Mark `library` as open by this instance (when it is opened). Only call
/// it on an existing library folder.
pub(crate) fn claim(library: &Path) -> std::io::Result<()> {
    write_lock(library, &me(Utc::now()))
}

/// Refresh this instance's timestamp (after each index save).
pub(crate) fn refresh(library: &Path) -> std::io::Result<()> {
    claim(library)
}

/// The other instance that had `library` open less than
/// [`FRESH_FOR_MINUTES`] before `now`, among those seen so far.
fn other_instance(library: &Path, now: DateTime<Utc>) -> Option<OtherInstance> {
    let reg = registry().lock().unwrap_or_else(|e| e.into_inner());
    let other = reg.others.get(library)?;
    let updated = DateTime::parse_from_rfc3339(&other.updated_at).ok()?;
    let age = now.signed_duration_since(updated);
    // A clock slightly ahead on the other computer gives a negative age:
    // that is "just now", not "never".
    (age < Duration::minutes(FRESH_FOR_MINUTES)).then(|| OtherInstance {
        host: other.host.clone(),
        updated_at: other.updated_at.clone(),
    })
}

/// [`library_lock_status`](super::library_lock_status): claim `library` and
/// report another instance that was using it recently. Never fails: an
/// unwritable lock file only means nothing can be detected.
pub(crate) fn status(library: &Path) -> LockStatus {
    let _ = claim(library);
    LockStatus {
        other_instance: other_instance(library, Utc::now()),
    }
}

/// Remove the lock files this instance still holds (on exit). A file that
/// another instance has taken over since is left alone.
pub(crate) fn release_all() {
    let held: Vec<PathBuf> = {
        let mut reg = registry().lock().unwrap_or_else(|e| e.into_inner());
        reg.held.drain().collect()
    };
    release(&held, &me(Utc::now()).instance_id);
}

/// Remove the lock file of each of `libraries` that still names `instance_id`.
fn release(libraries: &[PathBuf], instance_id: &str) {
    for library in libraries {
        if read_lock(library).is_some_and(|lock| lock.instance_id == instance_id) {
            let _ = fs::remove_file(lock_path(library));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn other(host: &str, updated_at: DateTime<Utc>) -> LockInfo {
        LockInfo {
            instance_id: format!("other-{host}"),
            host: host.into(),
            pid: 1,
            updated_at: updated_at.to_rfc3339(),
        }
    }

    #[test]
    fn claiming_writes_this_instance() {
        let dir = tempfile::tempdir().unwrap();
        claim(dir.path()).unwrap();
        let lock = read_lock(dir.path()).unwrap();
        assert_eq!(
            lock,
            LockInfo {
                updated_at: lock.updated_at.clone(),
                ..me(Utc::now())
            }
        );
        assert_eq!(
            status(dir.path()).other_instance,
            None,
            "our own lock is not another instance"
        );
    }

    #[test]
    fn another_recent_instance_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        let now = Utc::now();
        write_lock(lib, &other("pc-salon", now - Duration::minutes(2))).unwrap();
        let status = status(lib);
        assert_eq!(status.other_instance.unwrap().host, "pc-salon");
        // We hold the file now.
        assert_eq!(read_lock(lib).unwrap().instance_id, me(now).instance_id);
        // Still reported on the next check (until it gets old).
        assert!(other_instance(lib, now).is_some());
        assert!(
            other_instance(lib, now + Duration::minutes(4)).is_none(),
            "older than 5 minutes"
        );
    }

    /// After a crash (or a quit that couldn't clean up in time) this
    /// computer's previous run is still in the lock file: relaunching within
    /// five minutes must not warn about "another Librarium".
    #[test]
    fn this_computers_previous_run_is_not_another_instance() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path();
        let previous_run = LockInfo {
            instance_id: "a-previous-launch".into(),
            pid: 999_999,
            ..me(Utc::now() - Duration::minutes(1))
        };
        write_lock(lib, &previous_run).unwrap();
        assert_eq!(status(lib).other_instance, None);
    }

    #[test]
    fn a_stale_lock_is_not_reported() {
        let dir = tempfile::tempdir().unwrap();
        write_lock(
            dir.path(),
            &other("old-laptop", Utc::now() - Duration::minutes(30)),
        )
        .unwrap();
        assert_eq!(status(dir.path()).other_instance, None);
    }

    #[test]
    fn a_damaged_lock_file_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(INDEX_DIR)).unwrap();
        fs::write(lock_path(dir.path()), b"{ nope").unwrap();
        assert_eq!(status(dir.path()).other_instance, None);
        assert!(read_lock(dir.path()).is_some());
    }

    #[test]
    fn release_removes_only_our_own_locks() {
        let mine = tempfile::tempdir().unwrap();
        let taken = tempfile::tempdir().unwrap();
        claim(mine.path()).unwrap();
        claim(taken.path()).unwrap();
        // Another instance took this one over meanwhile.
        fsutil::atomic_write(
            &lock_path(taken.path()),
            &serde_json::to_vec(&other("x", Utc::now())).unwrap(),
        )
        .unwrap();
        // (Not `release_all`: it would also release the other tests' libraries.)
        release(
            &[mine.path().to_path_buf(), taken.path().to_path_buf()],
            &me(Utc::now()).instance_id,
        );
        assert!(!lock_path(mine.path()).exists());
        assert!(lock_path(taken.path()).exists());
    }
}
