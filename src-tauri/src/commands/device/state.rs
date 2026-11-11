//! What the device commands share while the app runs ([`TransferState`],
//! Tauri-managed): cancelled transfer jobs, and which devices are busy
//! receiving books or being ejected.
//!
//! The busy/ejecting bookkeeping is what makes these safe:
//! - `eject_device` refuses while a transfer to that device runs, and a
//!   transfer refuses to start while it is being ejected (one lock checks
//!   and marks both, so they can't race);
//! - stale `.part` files are only cleaned from a device nothing is being
//!   sent to.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::error::{display_name, AppError, AppResult};

/// See the module docs. Cloning is cheap and shares the same state (so a
/// blocking job can hold one).
#[derive(Default, Clone)]
pub struct TransferState {
    /// Ids of jobs the user asked to cancel. `send_books` checks it before
    /// a job starts, after its conversion, and between chunks of the copy;
    /// ids are forgotten when their batch ends.
    cancelled: Arc<Mutex<HashSet<String>>>,
    activity: Arc<Mutex<Activity>>,
}

#[derive(Default)]
struct Activity {
    /// Devices (by [`mount_key`]) with running `send_books` batches, and how many.
    sending: HashMap<PathBuf, usize>,
    /// Devices being ejected.
    ejecting: HashSet<PathBuf>,
}

/// Lock, recovering from poisoning (a panicked job must not make every
/// later command panic too).
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// How a device's mount path is compared: canonicalised when possible (the
/// frontend sends back the path detection gave it, but spellings may
/// differ by a trailing separator or a link).
pub(crate) fn mount_key(mount: &Path) -> PathBuf {
    fs::canonicalize(mount).unwrap_or_else(|_| mount.components().collect())
}

impl TransferState {
    /// Mark `job_id` as cancelled.
    pub fn cancel(&self, job_id: &str) {
        lock(&self.cancelled).insert(job_id.to_string());
    }

    /// Whether `job_id` was cancelled.
    pub fn is_cancelled(&self, job_id: &str) -> bool {
        lock(&self.cancelled).contains(job_id)
    }

    /// Forget the cancellations of a finished batch.
    pub(crate) fn forget(&self, job_ids: &[String]) {
        let mut set = lock(&self.cancelled);
        for id in job_ids {
            set.remove(id);
        }
    }

    /// Mark the devices at `mounts` as receiving books until the guard is
    /// dropped. Refused (`DEVICE`) if one of them is being ejected.
    pub(crate) fn begin_sending(&self, mounts: &[PathBuf]) -> AppResult<SendingGuard> {
        let keys: Vec<PathBuf> = mounts
            .iter()
            .map(|m| mount_key(m))
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let mut activity = lock(&self.activity);
        if let Some(k) = keys.iter().find(|k| activity.ejecting.contains(*k)) {
            return Err(AppError::Device(format!(
                "{} is being ejected",
                display_name(k)
            )));
        }
        for k in &keys {
            *activity.sending.entry(k.clone()).or_default() += 1;
        }
        Ok(SendingGuard {
            state: self.clone(),
            keys,
        })
    }

    /// Whether books are being sent to the device at `mount`.
    pub(crate) fn is_sending(&self, mount: &Path) -> bool {
        lock(&self.activity).sending.contains_key(&mount_key(mount))
    }

    /// Mark the device at `mount` as being ejected until the guard is
    /// dropped. Refused (`CONFLICT`) while books are being sent to it or it
    /// is already being ejected.
    pub(crate) fn begin_eject(&self, mount: &Path) -> AppResult<EjectGuard> {
        let key = mount_key(mount);
        let mut activity = lock(&self.activity);
        let name = display_name(mount);
        if activity.sending.contains_key(&key) {
            return Err(AppError::Conflict(format!(
                "Books are being sent to {name}. Wait for the transfer to finish (or cancel it), then eject."
            )));
        }
        if !activity.ejecting.insert(key.clone()) {
            return Err(AppError::Conflict(format!(
                "{name} is already being ejected"
            )));
        }
        Ok(EjectGuard {
            state: self.clone(),
            key,
        })
    }
}

/// Keeps devices marked as receiving books (see [`TransferState::begin_sending`]).
pub(crate) struct SendingGuard {
    state: TransferState,
    keys: Vec<PathBuf>,
}

impl Drop for SendingGuard {
    fn drop(&mut self) {
        let mut activity = lock(&self.state.activity);
        for k in &self.keys {
            if let Some(n) = activity.sending.get_mut(k) {
                *n -= 1;
                if *n == 0 {
                    activity.sending.remove(k);
                }
            }
        }
    }
}

/// Keeps a device marked as being ejected (see [`TransferState::begin_eject`]).
pub(crate) struct EjectGuard {
    state: TransferState,
    key: PathBuf,
}

impl Drop for EjectGuard {
    fn drop(&mut self) {
        lock(&self.state.activity).ejecting.remove(&self.key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracks_cancellations() {
        let state = TransferState::default();
        assert!(!state.is_cancelled("a"));
        state.cancel("a");
        assert!(state.is_cancelled("a"));
        state.forget(&["a".to_string()]);
        assert!(!state.is_cancelled("a"));
    }

    #[test]
    fn eject_and_transfers_exclude_each_other() {
        let state = TransferState::default();
        let device = tempfile::tempdir().unwrap();
        let mount = device.path().to_path_buf();
        // Another spelling of the same mount.
        let spelled = PathBuf::from(format!("{}/", mount.display()));

        let batch_a = state
            .begin_sending(&[mount.clone(), mount.clone()])
            .unwrap();
        let batch_b = state.begin_sending(std::slice::from_ref(&spelled)).unwrap();
        assert!(state.is_sending(&mount));
        assert_eq!(
            state.begin_eject(&mount).err().map(|e| e.code()),
            Some("CONFLICT")
        );
        drop(batch_a);
        assert!(state.is_sending(&mount), "batch b still runs");
        drop(batch_b);
        assert!(!state.is_sending(&mount));

        let eject = state.begin_eject(&spelled).unwrap();
        assert_eq!(
            state.begin_eject(&mount).err().map(|e| e.code()),
            Some("CONFLICT")
        );
        assert_eq!(
            state
                .begin_sending(std::slice::from_ref(&mount))
                .err()
                .map(|e| e.code()),
            Some("DEVICE")
        );
        drop(eject);
        assert!(state.begin_sending(&[mount]).is_ok());
    }
}
