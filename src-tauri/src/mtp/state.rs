//! [`MtpState`] (Tauri-managed): the MTP Kindles plugged in, one worker
//! each, and the experimental setting that gates them.
//!
//! Every detection poll calls [`MtpState::poll`]:
//! 1. **Enumerate** (cheap, nothing claimed). A worker whose device is no
//!    longer listed is stopped: that is how unplugging is noticed.
//! 2. With **MTP disabled** (the default), stop there: each Kindle found is
//!    reported as [`DeviceState::MtpDisabled`], so the UI can suggest the
//!    setting, and nothing is opened or claimed.
//! 3. Otherwise start a worker for each new device (waiting briefly for its
//!    first connection, so the first poll can already show it) and ask the
//!    others to refresh their free space.
//!
//! A device that was ejected stays released (no worker) until it is
//! unplugged.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde::Serialize;

use super::session::Backend;
use super::worker::{ConnectHook, Failure, Logger, Ready, Status, Worker, WorkerHandle};
use super::{mount_path, MtpError};

/// How long a poll waits for a new device's first connection.
const FIRST_CONNECT_WAIT: Duration = Duration::from_secs(3);

/// How often a failed connection is retried.
const RETRY_AFTER: Duration = Duration::from_secs(10);

/// How long ejecting waits for the session to close.
const EJECT_WAIT: Duration = Duration::from_secs(15);

/// What an MTP Kindle is doing (`MtpDevice.state` over IPC).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceState {
    /// Connected: it is in `detect_devices` too.
    Ready,
    /// Opening the session (first plug, or reconnecting after an error).
    Connecting,
    /// MTP support is off (Settings): nothing was opened.
    MtpDisabled,
    /// The device keeps answering "busy"; retried.
    Busy,
    /// Another program holds the device, or no permission: the message says what to do.
    ClaimFailed,
    /// Anything else; retried.
    Error,
    /// Ejected (released): safe to unplug.
    Ejected,
}

/// One MTP Kindle, as `get_mtp_status` reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MtpDevice {
    /// `mtp://<port>`: the `KindleDevice.mountPath` once ready.
    pub mount_path: String,
    /// The model once connected, else the USB product name ("Kindle").
    pub name: String,
    pub state: DeviceState,
    /// What to do (or what went wrong), for the user; absent when ready.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// `KindleDevice.id`, when ready.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
}

/// The message shown for a Kindle while MTP support is off.
pub const DISABLED_MESSAGE: &str =
    "This Kindle connects over MTP, which Librarium supports experimentally. \
     Turn on \"MTP Kindles (experimental)\" in Settings to use it.";

struct Inner {
    enabled: AtomicBool,
    backend: Arc<dyn Backend>,
    hook: ConnectHook,
    log: Logger,
    workers: Mutex<HashMap<String, Worker>>,
    ejected: Mutex<HashSet<String>>,
    first_wait: Duration,
    retry_after: Duration,
}

/// See the module docs. Cloning shares the same state.
#[derive(Clone)]
pub struct MtpState {
    inner: Arc<Inner>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl MtpState {
    pub(crate) fn new(backend: Arc<dyn Backend>, hook: ConnectHook, log: Logger) -> Self {
        Self::with_timing(backend, hook, log, FIRST_CONNECT_WAIT, RETRY_AFTER)
    }

    pub(crate) fn with_timing(
        backend: Arc<dyn Backend>,
        hook: ConnectHook,
        log: Logger,
        first_wait: Duration,
        retry_after: Duration,
    ) -> Self {
        MtpState {
            inner: Arc::new(Inner {
                enabled: AtomicBool::new(false),
                backend,
                hook,
                log,
                workers: Mutex::new(HashMap::new()),
                ejected: Mutex::new(HashSet::new()),
                first_wait,
                retry_after,
            }),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.inner.enabled.load(Ordering::Acquire)
    }

    /// Turn MTP support on or off (the `mtpEnabled` setting). Off stops
    /// every worker: devices are released once their current job ends.
    pub fn set_enabled(&self, enabled: bool) {
        let was = self.inner.enabled.swap(enabled, Ordering::AcqRel);
        if was != enabled {
            (self.inner.log)(format!(
                "[mtp] experimental MTP support {}",
                if enabled { "enabled" } else { "disabled" }
            ));
        }
        if !enabled {
            lock(&self.inner.workers).clear();
        }
    }

    /// See the module docs. Blocking (enumeration; up to a few seconds for
    /// a device seen for the first time).
    pub fn poll(&self) -> Vec<(MtpDevice, Option<Ready>)> {
        let candidates = self.inner.backend.enumerate();
        let present: HashSet<&str> = candidates.iter().map(|c| c.key.as_str()).collect();
        lock(&self.inner.ejected).retain(|k| present.contains(k.as_str()));

        let mut fresh = Vec::new();
        {
            let mut workers = lock(&self.inner.workers);
            let gone: Vec<String> = workers
                .keys()
                .filter(|k| !present.contains(k.as_str()))
                .cloned()
                .collect();
            for key in gone {
                (self.inner.log)(format!("[mtp] {key}: unplugged"));
                workers.remove(&key);
            }
            if !self.is_enabled() {
                workers.clear();
            } else {
                let ejected = lock(&self.inner.ejected);
                for c in candidates.iter().filter(|c| !ejected.contains(&c.key)) {
                    match workers.get(&c.key) {
                        Some(worker) => worker.handle().refresh(),
                        None => {
                            (self.inner.log)(format!("[mtp] {}: {} found", c.key, c.label));
                            let worker = Worker::spawn(
                                c.clone(),
                                Arc::clone(&self.inner.backend),
                                Arc::clone(&self.inner.hook),
                                Arc::clone(&self.inner.log),
                                self.inner.retry_after,
                            );
                            fresh.push(worker.handle());
                            workers.insert(c.key.clone(), worker);
                        }
                    }
                }
            }
        }
        // Outside the lock: other commands use the workers meanwhile.
        for handle in fresh {
            handle.wait_settled(self.inner.first_wait);
        }

        let enabled = self.is_enabled();
        let ejected = lock(&self.inner.ejected).clone();
        let workers = lock(&self.inner.workers);
        candidates
            .iter()
            .map(|c| {
                let base = MtpDevice {
                    mount_path: mount_path(&c.key),
                    name: c.label.clone(),
                    state: DeviceState::Error,
                    message: None,
                    device_id: None,
                };
                if !enabled {
                    return (
                        MtpDevice {
                            state: DeviceState::MtpDisabled,
                            message: Some(DISABLED_MESSAGE.into()),
                            ..base
                        },
                        None,
                    );
                }
                if ejected.contains(&c.key) {
                    return (
                        MtpDevice {
                            state: DeviceState::Ejected,
                            message: Some("Ejected: safe to unplug.".into()),
                            ..base
                        },
                        None,
                    );
                }
                match workers.get(&c.key).map(|w| w.handle().status()) {
                    Some(Status::Ready(ready)) => (
                        MtpDevice {
                            name: ready.name.clone(),
                            state: DeviceState::Ready,
                            device_id: Some(ready.device_id.clone()),
                            ..base
                        },
                        Some(ready),
                    ),
                    Some(Status::Failed { kind, message }) => {
                        let state = match kind {
                            Failure::ClaimFailed => DeviceState::ClaimFailed,
                            Failure::Busy => DeviceState::Busy,
                            Failure::Error => DeviceState::Error,
                        };
                        (
                            MtpDevice {
                                state,
                                message: Some(message),
                                ..base
                            },
                            None,
                        )
                    }
                    Some(Status::Connecting) | None => (
                        MtpDevice {
                            state: DeviceState::Connecting,
                            ..base
                        },
                        None,
                    ),
                }
            })
            .collect()
    }

    /// The worker of the device at `key`, if MTP is enabled and it is plugged in.
    pub(crate) fn worker(&self, key: &str) -> Result<WorkerHandle, MtpError> {
        if !self.is_enabled() {
            return Err(MtpError::Refused(DISABLED_MESSAGE.into()));
        }
        lock(&self.inner.workers)
            .get(key)
            .map(Worker::handle)
            .ok_or(MtpError::WorkerGone)
    }

    /// The latest status of the device at `key`.
    pub(crate) fn ready(&self, key: &str) -> Option<Ready> {
        match lock(&self.inner.workers).get(key)?.handle().status() {
            Status::Ready(ready) => Some(ready),
            _ => None,
        }
    }

    /// Release the device at `key` (close the session) so it can be
    /// unplugged; it stays released until then. Fails if it isn't
    /// connected, or if its session didn't close in time.
    pub(crate) fn eject(&self, key: &str) -> Result<(), MtpError> {
        let worker = lock(&self.inner.workers)
            .remove(key)
            .ok_or(MtpError::WorkerGone)?;
        lock(&self.inner.ejected).insert(key.to_string());
        if worker.stop_and_wait(EJECT_WAIT) {
            (self.inner.log)(format!("[mtp] {key}: ejected"));
            Ok(())
        } else {
            Err(MtpError::Refused("The Kindle is still busy; wait for the current operation to finish, then eject again.".into()))
        }
    }

    /// The log sink (for code running on worker threads).
    pub(crate) fn logger(&self) -> Logger {
        Arc::clone(&self.inner.log)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mtp::fake::{self, FakeBackend, FakeDevice};
    use crate::mtp::worker::tests::collect_logs;
    use std::time::Instant;

    fn hook() -> ConnectHook {
        Arc::new(|_c, conn| {
            let st = conn.session.storages()?.remove(0);
            Ok(Ready {
                device_id: "dev-1".into(),
                name: "Kindle Paperwhite".into(),
                model: Some("Kindle Paperwhite".into()),
                storage: st.id,
                free_bytes: st.free_bytes,
                total_bytes: st.capacity_bytes,
            })
        })
    }

    fn state(backend: Arc<FakeBackend>) -> MtpState {
        let (log, _) = collect_logs();
        MtpState::with_timing(backend, hook(), log, Duration::from_secs(5), Duration::ZERO)
    }

    fn states(state: &MtpState) -> Vec<(String, DeviceState)> {
        state
            .poll()
            .into_iter()
            .map(|(d, _)| (d.mount_path, d.state))
            .collect()
    }

    #[test]
    fn disabled_it_only_enumerates() {
        let device = FakeDevice::kindle();
        let state = state(FakeBackend::with(&[("usb-1-2", &device)]));
        let polled = state.poll();
        assert_eq!(polled.len(), 1);
        let (dev, ready) = &polled[0];
        assert_eq!(
            (dev.mount_path.as_str(), dev.state),
            ("mtp://usb-1-2", DeviceState::MtpDisabled)
        );
        assert!(dev.message.as_deref().unwrap().contains("Settings"));
        assert!(ready.is_none());
        assert_eq!(device.state().opens, 0, "nothing was opened or claimed");
        assert!(matches!(state.worker("usb-1-2"), Err(MtpError::Refused(_))));
    }

    #[test]
    fn enabled_it_connects_and_reports_ready() {
        let device = FakeDevice::kindle();
        let state = state(FakeBackend::with(&[("usb-1-2", &device)]));
        state.set_enabled(true);
        let polled = state.poll();
        let (dev, ready) = &polled[0];
        assert_eq!(dev.state, DeviceState::Ready, "{dev:?}");
        assert_eq!(dev.device_id.as_deref(), Some("dev-1"));
        assert_eq!(ready.as_ref().unwrap().name, "Kindle Paperwhite");
        assert_eq!(
            serde_json::to_value(dev).unwrap(),
            serde_json::json!({
                "mountPath": "mtp://usb-1-2", "name": "Kindle Paperwhite", "state": "ready", "deviceId": "dev-1",
            })
        );
        // Later polls reuse the worker.
        state.poll();
        assert_eq!(device.state().opens, 1);
    }

    #[test]
    fn a_device_held_by_another_app_is_reported_with_what_to_do() {
        let device = FakeDevice::kindle();
        device.state().claim_error = Some(fake::claimed_by_another_app());
        let state = state(FakeBackend::with(&[("usb-1-2", &device)]));
        state.set_enabled(true);
        let (dev, _) = state.poll().remove(0);
        assert_eq!(dev.state, DeviceState::ClaimFailed);
        assert!(dev.message.unwrap().contains("replug"));
        assert_eq!(
            serde_json::to_value(DeviceState::ClaimFailed).unwrap(),
            "claimFailed"
        );
        assert_eq!(
            serde_json::to_value(DeviceState::MtpDisabled).unwrap(),
            "mtpDisabled"
        );
    }

    #[test]
    fn unplugging_stops_the_worker_and_replugging_starts_another() {
        let device = FakeDevice::kindle();
        let state = state(FakeBackend::with(&[("usb-1-2", &device)]));
        state.set_enabled(true);
        state.poll();
        let handle = state.worker("usb-1-2").unwrap();
        device.unplug();
        assert!(state.poll().is_empty());
        assert!(matches!(state.worker("usb-1-2"), Err(MtpError::WorkerGone)));
        let deadline = Instant::now() + Duration::from_secs(5);
        while handle.call(|c| c.session.storages()) != Err(MtpError::WorkerGone) {
            assert!(Instant::now() < deadline, "the old worker never stopped");
            std::thread::sleep(Duration::from_millis(20));
        }
        device.plug();
        assert_eq!(
            states(&state),
            vec![("mtp://usb-1-2".to_string(), DeviceState::Ready)]
        );
        assert_eq!(device.state().opens, 2);
    }

    #[test]
    fn several_devices_each_get_a_worker() {
        let a = FakeDevice::kindle();
        let b = FakeDevice::kindle();
        let state = state(FakeBackend::with(&[("usb-1-2", &a), ("usb-1-3", &b)]));
        state.set_enabled(true);
        assert_eq!(
            states(&state),
            vec![
                ("mtp://usb-1-2".to_string(), DeviceState::Ready),
                ("mtp://usb-1-3".to_string(), DeviceState::Ready),
            ]
        );
    }

    #[test]
    fn disabling_releases_everything() {
        let device = FakeDevice::kindle();
        let state = state(FakeBackend::with(&[("usb-1-2", &device)]));
        state.set_enabled(true);
        state.poll();
        state.set_enabled(false);
        assert!(state.worker("usb-1-2").is_err());
        assert_eq!(
            states(&state),
            vec![("mtp://usb-1-2".to_string(), DeviceState::MtpDisabled)]
        );
    }

    #[test]
    fn an_ejected_device_stays_released_until_unplugged() {
        let device = FakeDevice::kindle();
        let state = state(FakeBackend::with(&[("usb-1-2", &device)]));
        state.set_enabled(true);
        state.poll();
        state.eject("usb-1-2").unwrap();
        assert_eq!(
            states(&state),
            vec![("mtp://usb-1-2".to_string(), DeviceState::Ejected)]
        );
        assert_eq!(device.state().opens, 1, "not reopened");
        assert!(state.eject("usb-1-2").is_err(), "nothing to eject");
        device.unplug();
        state.poll();
        device.plug();
        assert_eq!(
            states(&state),
            vec![("mtp://usb-1-2".to_string(), DeviceState::Ready)]
        );
    }
}
