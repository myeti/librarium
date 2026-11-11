//! One thread per MTP device, owning its session.
//!
//! Every operation on the device is a job sent to that thread over a
//! channel ([`WorkerHandle::call`]); the thread runs them one at a time.
//! This replaces the `feat/mtp` branch's session behind a mutex, which a
//! transfer held for minutes while each detection poll parked another
//! thread behind it:
//! - **Detection never waits**: it reads the worker's [`Status`], kept
//!   current by the thread, and at most asks for a refresh.
//! - **Health**: a job that fails with a transport error (or panics) drops
//!   the session; the thread reconnects right away. A failed connection is
//!   retried every `retry_after`, or as soon as a job needs the device.
//!   Unplugging is noticed by enumeration ([`super::state`]), which stops
//!   the worker.
//! - **Partial files** left by a failed or cancelled send are deleted once
//!   reconnected ([`Orphan`]).
//! - **Windows COM** needs every call on a thread that initialised COM and
//!   no COM object outliving it: the backend's per-thread setup runs first
//!   on this thread and is dropped last, after the session.
//! - **Long jobs** report progress and see cancellation through a
//!   [`Ticker`] ([`WorkerHandle::call_with_progress`]); the calling thread
//!   relays both, so callers keep their non-`Send` callbacks.

use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use super::devfs::DeviceFs;
use super::session::{Backend, Candidate, ClaimKind, MtpError, MtpSession, Orphan, StorageId};
use crate::error::panic_message;

/// Where worker threads log (the app's log in production, a collector in tests).
pub(crate) type Logger = Arc<dyn Fn(String) + Send + Sync>;

/// How often an idle worker checks whether it was stopped or a retry is due.
const TICK: Duration = Duration::from_millis(250);

/// How often a caller waiting for a long job checks for cancellation.
const CANCEL_POLL: Duration = Duration::from_millis(100);

/// What a job gets: the session and the path cache that goes with it.
pub(crate) struct Connection {
    pub session: Box<dyn MtpSession>,
    pub fs: DeviceFs,
    /// Set by a job that changed the free space (a send, a delete): the
    /// worker's [`Ready`] status is updated before the job's reply goes out.
    pub space: Option<(Option<u64>, Option<u64>)>,
}

/// A connected device, as detection reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Ready {
    /// `KindleDevice.id`: the UUID in the device manifest (never a serial).
    pub device_id: String,
    pub name: String,
    pub model: Option<String>,
    /// The storage books go to.
    pub storage: StorageId,
    pub free_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
}

/// Why a worker has no connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Failure {
    /// Another program holds the device, or no permission to open it.
    ClaimFailed,
    /// The device keeps answering "busy".
    Busy,
    Error,
}

/// A worker's state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Status {
    Connecting,
    Ready(Ready),
    Failed { kind: Failure, message: String },
}

/// Run on each new connection, before it is used: reads what [`Ready`]
/// needs (the device id from the manifest, the model, the space).
pub(crate) type ConnectHook =
    Arc<dyn Fn(&Candidate, &mut Connection) -> Result<Ready, MtpError> + Send + Sync>;

/// A job: runs with the connection (or why there is none); returns `false`
/// when the connection must be dropped.
type Job = Box<dyn FnOnce(Result<&mut Connection, MtpError>) -> bool + Send>;

enum Msg {
    Run(Job),
    Refresh,
}

struct Shared {
    status: Mutex<Status>,
    changed: Condvar,
    refresh_pending: AtomicBool,
    stop: AtomicBool,
    finished: Mutex<bool>,
    finished_cv: Condvar,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Shared {
    fn set_status(&self, status: Status) {
        *lock(&self.status) = status;
        self.changed.notify_all();
    }

    fn update_space(&self, free: Option<u64>, total: Option<u64>) {
        if let Status::Ready(ready) = &mut *lock(&self.status) {
            ready.free_bytes = free;
            ready.total_bytes = total;
        }
    }
}

/// Reports progress from inside a job, and tells it the caller cancelled.
pub(crate) struct Ticker {
    report: Box<dyn Fn(u64, u64) + Send>,
    cancel: Arc<AtomicBool>,
}

impl Ticker {
    pub(crate) fn progress(&self, done: u64, total: u64) {
        (self.report)(done, total);
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

enum Event<T> {
    Progress(u64, u64),
    Done(Result<T, MtpError>),
}

/// A cheap, cloneable way to send jobs to a worker.
#[derive(Clone)]
pub(crate) struct WorkerHandle {
    tx: mpsc::Sender<Msg>,
    shared: Arc<Shared>,
}

impl WorkerHandle {
    pub(crate) fn status(&self) -> Status {
        lock(&self.shared.status).clone()
    }

    /// The status once it is no longer [`Status::Connecting`], or after `timeout`.
    pub(crate) fn wait_settled(&self, timeout: Duration) -> Status {
        let guard = lock(&self.shared.status);
        let (guard, _) = self
            .shared
            .changed
            .wait_timeout_while(guard, timeout, |s| *s == Status::Connecting)
            .unwrap_or_else(|e| e.into_inner());
        guard.clone()
    }

    /// Ask the worker to check the device (free space; a dead link is
    /// noticed and reconnected). Doesn't wait; requests coalesce while one
    /// is queued (behind a long transfer, say).
    pub(crate) fn refresh(&self) {
        if !self.shared.refresh_pending.swap(true, Ordering::AcqRel) {
            let _ = self.tx.send(Msg::Refresh);
        }
    }

    /// Run `f` on the worker thread and wait for its result.
    pub(crate) fn call<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Connection) -> Result<T, MtpError> + Send + 'static,
    ) -> Result<T, MtpError> {
        let (tx, rx) = mpsc::sync_channel(1);
        let shared = Arc::clone(&self.shared);
        let job: Job = Box::new(move |conn| {
            let (result, keep) = run_job(conn, &shared, f);
            let _ = tx.send(result);
            keep
        });
        self.tx
            .send(Msg::Run(job))
            .map_err(|_| MtpError::WorkerGone)?;
        // The job is dropped unrun if the worker stops first.
        rx.recv().unwrap_or(Err(MtpError::WorkerGone))
    }

    /// [`Self::call`] for a long job: `on_progress` receives its
    /// [`Ticker::progress`] reports, and `is_cancelled` (polled here, on
    /// the calling thread) is relayed to [`Ticker::is_cancelled`].
    pub(crate) fn call_with_progress<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Connection, &Ticker) -> Result<T, MtpError> + Send + 'static,
        on_progress: &mut dyn FnMut(u64, u64),
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<T, MtpError> {
        let (tx, rx) = mpsc::channel::<Event<T>>();
        let cancel = Arc::new(AtomicBool::new(is_cancelled()));
        let progress_tx = tx.clone();
        let ticker = Ticker {
            report: Box::new(move |done, total| {
                let _ = progress_tx.send(Event::Progress(done, total));
            }),
            cancel: Arc::clone(&cancel),
        };
        let shared = Arc::clone(&self.shared);
        let job: Job = Box::new(move |conn| {
            let (result, keep) = run_job(conn, &shared, |c| f(c, &ticker));
            let _ = tx.send(Event::Done(result));
            keep
        });
        self.tx
            .send(Msg::Run(job))
            .map_err(|_| MtpError::WorkerGone)?;
        loop {
            match rx.recv_timeout(CANCEL_POLL) {
                Ok(Event::Progress(done, total)) => on_progress(done, total),
                Ok(Event::Done(result)) => return result,
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Err(MtpError::WorkerGone),
            }
            if !cancel.load(Ordering::Relaxed) && is_cancelled() {
                cancel.store(true, Ordering::Relaxed);
            }
        }
    }
}

/// Run a job's function: panics are caught; the free space it noted is
/// published; a fatal error (or a panic) asks for the connection to be dropped.
fn run_job<T>(
    conn: Result<&mut Connection, MtpError>,
    shared: &Shared,
    f: impl FnOnce(&mut Connection) -> Result<T, MtpError>,
) -> (Result<T, MtpError>, bool) {
    let c = match conn {
        Ok(c) => c,
        Err(e) => return (Err(e), true),
    };
    match panic::catch_unwind(AssertUnwindSafe(|| f(&mut *c))) {
        Ok(result) => {
            if let Some((free, total)) = c.space.take() {
                shared.update_space(free, total);
            }
            let keep = !matches!(&result, Err(e) if e.is_fatal());
            (result, keep)
        }
        Err(payload) => (
            Err(MtpError::Local(format!(
                "Internal error while talking to the Kindle ({}). The log file has the details.",
                panic_message(payload.as_ref())
            ))),
            false,
        ),
    }
}

/// Owns a worker thread. Dropping it stops the thread once its current job
/// is done (the session is then closed and the device released).
pub(crate) struct Worker {
    handle: WorkerHandle,
}

impl Worker {
    pub(crate) fn spawn(
        candidate: Candidate,
        backend: Arc<dyn Backend>,
        hook: ConnectHook,
        log: Logger,
        retry_after: Duration,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        let shared = Arc::new(Shared {
            status: Mutex::new(Status::Connecting),
            changed: Condvar::new(),
            refresh_pending: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            finished: Mutex::new(false),
            finished_cv: Condvar::new(),
        });
        let runner = Runner {
            candidate,
            backend,
            hook,
            log,
            retry_after,
            shared: Arc::clone(&shared),
            orphans: Vec::new(),
            retry_at: None,
            last_error: MtpError::WorkerGone,
            last_logged: None,
        };
        let name = format!("mtp-{}", runner.candidate.key);
        let spawned = thread::Builder::new()
            .name(name)
            .spawn(move || runner.run(rx));
        if let Err(e) = spawned {
            shared.set_status(Status::Failed {
                kind: Failure::Error,
                message: format!("Could not start the MTP worker: {e}"),
            });
        }
        Worker {
            handle: WorkerHandle { tx, shared },
        }
    }

    pub(crate) fn handle(&self) -> WorkerHandle {
        self.handle.clone()
    }

    /// Stop the worker and wait (up to `timeout`) until its session is
    /// closed and the device released. `false` on timeout (a long job is
    /// still running; it stops when that job ends).
    pub(crate) fn stop_and_wait(self, timeout: Duration) -> bool {
        let shared = Arc::clone(&self.handle.shared);
        drop(self);
        let guard = lock(&shared.finished);
        let (done, _) = shared
            .finished_cv
            .wait_timeout_while(guard, timeout, |done| !*done)
            .unwrap_or_else(|e| e.into_inner());
        *done
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.handle.shared.stop.store(true, Ordering::Release);
    }
}

/// The worker thread's state.
struct Runner {
    candidate: Candidate,
    backend: Arc<dyn Backend>,
    hook: ConnectHook,
    log: Logger,
    retry_after: Duration,
    shared: Arc<Shared>,
    orphans: Vec<Orphan>,
    /// When to try connecting again after a failure.
    retry_at: Option<Instant>,
    /// Why there is no connection (the reply to jobs meanwhile).
    last_error: MtpError,
    /// The last failure logged, so a retry every few seconds doesn't flood the log.
    last_logged: Option<String>,
}

impl Runner {
    fn run(mut self, rx: mpsc::Receiver<Msg>) {
        // Dropped last: after the connection (COM on Windows).
        let _thread_setup = self.backend.thread_setup();
        let mut conn = self.connect();
        loop {
            if self.stopped() {
                break;
            }
            let msg = match rx.recv_timeout(TICK) {
                Ok(msg) => Some(msg),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            };
            if self.stopped() {
                break;
            }
            match msg {
                None => {
                    if conn.is_none() && self.retry_due() {
                        conn = self.connect();
                    }
                }
                Some(Msg::Refresh) => {
                    self.shared.refresh_pending.store(false, Ordering::Release);
                    match conn.as_mut() {
                        Some(c) => {
                            if !self.probe(c) {
                                self.drop_connection(conn.take());
                                conn = self.connect();
                            }
                        }
                        None if self.retry_due() => conn = self.connect(),
                        None => {}
                    }
                }
                Some(Msg::Run(job)) => {
                    if conn.is_none() {
                        conn = self.connect();
                    }
                    let keep = match conn.as_mut() {
                        Some(c) => job(Ok(c)),
                        None => job(Err(self.last_error.clone())),
                    };
                    let healthy = conn.as_ref().is_some_and(|c| c.session.is_healthy());
                    if conn.is_some() && !(keep && healthy) {
                        (self.log)(format!(
                            "[mtp] {}: the connection failed; reconnecting",
                            self.candidate.key
                        ));
                        self.drop_connection(conn.take());
                        conn = self.connect();
                    }
                }
            }
        }
        drop(conn);
        (self.log)(format!("[mtp] {}: released", self.candidate.key));
        *lock(&self.shared.finished) = true;
        self.shared.finished_cv.notify_all();
    }

    fn stopped(&self) -> bool {
        self.shared.stop.load(Ordering::Acquire)
    }

    fn retry_due(&self) -> bool {
        self.retry_at.is_some_and(|at| Instant::now() >= at)
    }

    /// Open a session and run the connect hook; the status follows.
    fn connect(&mut self) -> Option<Connection> {
        self.shared.set_status(Status::Connecting);
        let result = self.backend.open(&self.candidate).and_then(|session| {
            let mut conn = Connection {
                session,
                fs: DeviceFs::default(),
                space: None,
            };
            self.clean_orphans(&mut conn);
            let hook = Arc::clone(&self.hook);
            match panic::catch_unwind(AssertUnwindSafe(|| hook(&self.candidate, &mut conn))) {
                Ok(Ok(ready)) => Ok((conn, ready)),
                Ok(Err(e)) => Err(e),
                Err(payload) => Err(MtpError::Local(format!(
                    "Internal error while connecting ({})",
                    panic_message(payload.as_ref())
                ))),
            }
        });
        match result {
            Ok((conn, ready)) => {
                (self.log)(format!(
                    "[mtp] {}: connected — {} ({} free)",
                    self.candidate.key,
                    ready.name,
                    ready.free_bytes.map_or(
                        "unknown".to_string(),
                        crate::commands::device::transfer::format_size
                    )
                ));
                self.retry_at = None;
                self.last_logged = None;
                self.shared.set_status(Status::Ready(ready));
                Some(conn)
            }
            Err(e) => {
                let kind = match &e {
                    MtpError::Claim { .. } => Failure::ClaimFailed,
                    MtpError::Busy => Failure::Busy,
                    _ => Failure::Error,
                };
                let text = e.to_string();
                if self.last_logged.as_deref() != Some(text.as_str()) {
                    let level = if matches!(
                        e,
                        MtpError::Claim {
                            kind: ClaimKind::Busy,
                            ..
                        }
                    ) {
                        "WARNING"
                    } else {
                        "ERROR"
                    };
                    (self.log)(format!(
                        "[mtp] {}: {level} — could not connect: {text}",
                        self.candidate.key
                    ));
                    self.last_logged = Some(text);
                }
                self.shared.set_status(Status::Failed {
                    kind,
                    message: e.user_message(),
                });
                self.last_error = e;
                self.retry_at = Some(Instant::now() + self.retry_after);
                None
            }
        }
    }

    /// Check the device; `false` when the connection must be dropped.
    fn probe(&mut self, c: &mut Connection) -> bool {
        let Status::Ready(ready) = lock(&self.shared.status).clone() else {
            return true;
        };
        match c.fs.space(&mut *c.session, &ready.storage) {
            Ok((free, total)) => {
                self.shared.update_space(free, total);
                true
            }
            Err(e) => !e.is_fatal(),
        }
    }

    fn drop_connection(&mut self, conn: Option<Connection>) {
        if let Some(mut c) = conn {
            self.orphans.extend(c.session.take_orphans());
        }
    }

    /// Delete the partial objects of failed sends, each only if it still is
    /// that object (same name, same folder: an id could have been reused).
    /// TODO(device-validation): MTP says object handles persist across
    /// sessions; check a Kindle keeps them, else orphans are only found
    /// by the next listing's `.librarium-*.part` cleanup (temp names) or
    /// stay (a first send under its final name).
    fn clean_orphans(&mut self, c: &mut Connection) {
        for orphan in std::mem::take(&mut self.orphans) {
            let same = matches!(c.session.object(&orphan.id), Ok(o)
                if !o.is_folder && o.name == orphan.name && o.parent == orphan.parent && o.storage == orphan.storage);
            if same && c.session.delete(&orphan.id).is_ok() {
                (self.log)(format!(
                    "[mtp] {}: removed the partial file \"{}\" left by an interrupted transfer",
                    self.candidate.key, orphan.name
                ));
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::mtp::fake::{self, FakeBackend, FakeDevice};

    pub(crate) fn collect_logs() -> (Logger, Arc<Mutex<Vec<String>>>) {
        let lines = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&lines);
        (
            Arc::new(move |line: String| sink.lock().unwrap().push(line)),
            lines,
        )
    }

    fn simple_hook() -> ConnectHook {
        Arc::new(|_candidate, conn| {
            let storages = conn.session.storages()?;
            Ok(Ready {
                device_id: "dev-1".into(),
                name: conn.session.description().model.clone(),
                model: None,
                storage: storages[0].id.clone(),
                free_bytes: storages[0].free_bytes,
                total_bytes: storages[0].capacity_bytes,
            })
        })
    }

    fn spawn(device: &FakeDevice) -> Worker {
        let (log, _) = collect_logs();
        spawn_logged(device, log)
    }

    fn spawn_logged(device: &FakeDevice, log: Logger) -> Worker {
        let backend = FakeBackend::with(&[("usb-1-1", device)]);
        let candidate = backend.enumerate().remove(0);
        Worker::spawn(candidate, backend, simple_hook(), log, Duration::ZERO)
    }

    const SETTLE: Duration = Duration::from_secs(5);

    #[test]
    fn a_worker_connects_and_runs_jobs_in_order() {
        let device = FakeDevice::kindle();
        let worker = spawn(&device);
        let Status::Ready(ready) = worker.handle().wait_settled(SETTLE) else {
            panic!("not ready")
        };
        assert_eq!(
            (ready.device_id.as_str(), ready.name.as_str()),
            ("dev-1", "Kindle Paperwhite")
        );
        let names = worker
            .handle()
            .call(|c| {
                Ok(c.session
                    .children(fake::STORAGE, None)?
                    .into_iter()
                    .map(|o| o.name)
                    .collect::<Vec<_>>())
            })
            .unwrap();
        assert_eq!(names, vec!["documents"]);
    }

    #[test]
    fn a_claim_failure_is_reported_and_retried() {
        let device = FakeDevice::kindle();
        device.state().claim_error = Some(fake::claimed_by_another_app());
        let (log, lines) = collect_logs();
        let worker = spawn_logged(&device, log);
        let status = worker.handle().wait_settled(SETTLE);
        assert!(
            matches!(&status, Status::Failed { kind: Failure::ClaimFailed, message } if message.contains("replug")),
            "{status:?}"
        );
        assert!(lines
            .lock()
            .unwrap()
            .iter()
            .any(|l| l.contains("WARNING") && l.contains("exclusive access")));
        let failures = || {
            lines
                .lock()
                .unwrap()
                .iter()
                .filter(|l| l.contains("could not connect"))
                .count()
        };
        let logged = failures();
        thread::sleep(TICK * 3);
        assert_eq!(failures(), logged, "retries don't flood the log");
        // The other app quit: the next retry connects.
        device.state().claim_error = None;
        worker.handle().refresh();
        let deadline = Instant::now() + SETTLE;
        while !matches!(worker.handle().status(), Status::Ready(_)) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        assert!(matches!(worker.handle().status(), Status::Ready(_)));
    }

    #[test]
    fn a_fatal_error_drops_the_session_and_reconnects() {
        let device = FakeDevice::kindle();
        let worker = spawn(&device);
        worker.handle().wait_settled(SETTLE);
        // The link drops during a job…
        let err = worker.handle().call(move |c| {
            c.session.storages()?;
            Err::<(), _>(MtpError::Transport("timed out".into()))
        });
        assert_eq!(err.unwrap_err(), MtpError::Transport("timed out".into()));
        // …the next job runs on a new session.
        assert!(worker.handle().call(|c| c.session.storages()).is_ok());
        assert_eq!(device.state().opens, 2);
    }

    #[test]
    fn a_panicking_job_fails_alone() {
        let device = FakeDevice::kindle();
        let worker = spawn(&device);
        worker.handle().wait_settled(SETTLE);
        let err = worker
            .handle()
            .call(|_| -> Result<(), MtpError> { panic!("boom") })
            .unwrap_err();
        assert!(
            matches!(&err, MtpError::Local(m) if m.contains("boom")),
            "{err:?}"
        );
        assert!(
            worker.handle().call(|c| c.session.storages()).is_ok(),
            "the worker survived"
        );
    }

    #[test]
    fn partial_files_of_a_cancelled_send_are_removed_after_reconnecting() {
        let device = FakeDevice::kindle();
        let docs = device.find("documents").unwrap().to_string();
        let worker = spawn(&device);
        worker.handle().wait_settled(SETTLE);
        let data = vec![1u8; 300_000];
        let err = worker.handle().call(move |c| {
            c.session.send_file(
                fake::STORAGE,
                Some(&docs),
                "Big.azw3",
                data.len() as u64,
                &mut &data[..],
                &mut |n| n < 100_000,
            )
        });
        assert_eq!(err.unwrap_err(), MtpError::Cancelled);
        // The cancel left the session unusable: the worker reconnected and
        // removed the partial file.
        assert!(worker.handle().call(|c| c.session.storages()).is_ok());
        assert_eq!(device.paths(), vec!["documents"]);
        assert!(device.state().log.iter().any(|l| l == "delete Big.azw3"));
    }

    #[test]
    fn an_orphan_that_is_now_another_file_is_kept() {
        let device = FakeDevice::kindle();
        let docs = device.find("documents").unwrap().to_string();
        let worker = spawn(&device);
        worker.handle().wait_settled(SETTLE);
        let data = vec![1u8; 300_000];
        let dev = device.clone();
        let _ = worker.handle().call(move |c| {
            let r = c.session.send_file(
                fake::STORAGE,
                Some(&docs),
                "Big.azw3",
                data.len() as u64,
                &mut &data[..],
                &mut |n| n < 100_000,
            );
            // Meanwhile the partial file was renamed on the device.
            let id = dev.find("documents/Big.azw3").unwrap();
            dev.state().entries.get_mut(&id).unwrap().name = "Other.azw3".into();
            r
        });
        assert!(worker.handle().call(|c| c.session.storages()).is_ok());
        assert_eq!(device.paths(), vec!["documents", "documents/Other.azw3"]);
    }

    #[test]
    fn progress_and_cancellation_cross_threads() {
        let device = FakeDevice::kindle();
        let worker = spawn(&device);
        worker.handle().wait_settled(SETTLE);
        let mut seen = Vec::new();
        let result = worker.handle().call_with_progress(
            |_, ticker| {
                for i in 1..=3 {
                    ticker.progress(i, 3);
                }
                Ok(ticker.is_cancelled())
            },
            &mut |done, total| seen.push((done, total)),
            &|| false,
        );
        assert_eq!(result, Ok(false));
        assert_eq!(seen, vec![(1, 3), (2, 3), (3, 3)]);

        // Cancelled before it starts (queued behind nothing here): the job sees it.
        let result = worker.handle().call_with_progress(
            |_, ticker| Ok(ticker.is_cancelled()),
            &mut |_, _| {},
            &|| true,
        );
        assert_eq!(result, Ok(true));

        // Cancelled while running.
        let result = worker.handle().call_with_progress(
            |_, ticker| {
                let deadline = Instant::now() + SETTLE;
                while !ticker.is_cancelled() && Instant::now() < deadline {
                    thread::sleep(Duration::from_millis(10));
                }
                if ticker.is_cancelled() {
                    Err(MtpError::Cancelled)
                } else {
                    Ok(())
                }
            },
            &mut |_, _| {},
            &{
                let start = Instant::now();
                move || start.elapsed() > Duration::from_millis(150)
            },
        );
        assert_eq!(result, Err(MtpError::Cancelled));
    }

    #[test]
    fn refreshing_updates_the_free_space_and_notices_a_dead_link() {
        let device = FakeDevice::kindle();
        let worker = spawn(&device);
        let Status::Ready(before) = worker.handle().wait_settled(SETTLE) else {
            panic!()
        };
        device.put("documents/Big.pdf", &vec![0u8; 1 << 20]);
        worker.handle().refresh();
        let deadline = Instant::now() + SETTLE;
        loop {
            if let Status::Ready(now) = worker.handle().status() {
                if now.free_bytes != before.free_bytes {
                    assert_eq!(now.free_bytes, before.free_bytes.map(|f| f - (1 << 20)));
                    break;
                }
            }
            assert!(Instant::now() < deadline, "the space was never refreshed");
            thread::sleep(Duration::from_millis(10));
        }
        // Unplugged: the probe fails, the reconnection too.
        device.unplug();
        worker.handle().refresh();
        let deadline = Instant::now() + SETTLE;
        while matches!(worker.handle().status(), Status::Ready(_)) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(matches!(
            worker.handle().status(),
            Status::Failed { .. } | Status::Connecting
        ));
    }

    #[test]
    fn a_stopped_worker_releases_the_device_and_refuses_jobs() {
        let device = FakeDevice::kindle();
        let worker = spawn(&device);
        let handle = worker.handle();
        handle.wait_settled(SETTLE);
        assert!(worker.stop_and_wait(SETTLE), "released in time");
        assert_eq!(
            handle.call(|c| c.session.storages()).unwrap_err(),
            MtpError::WorkerGone
        );
    }

    #[test]
    fn jobs_get_the_connection_error_when_the_device_cant_be_opened() {
        let device = FakeDevice::kindle();
        device.state().claim_error = Some(MtpError::Claim {
            kind: ClaimKind::PermissionDenied,
            detail: "EACCES".into(),
        });
        let worker = spawn(&device);
        assert!(matches!(
            worker.handle().wait_settled(SETTLE),
            Status::Failed {
                kind: Failure::ClaimFailed,
                ..
            }
        ));
        let err = worker.handle().call(|c| c.session.storages()).unwrap_err();
        assert!(
            matches!(
                err,
                MtpError::Claim {
                    kind: ClaimKind::PermissionDenied,
                    ..
                }
            ),
            "{err:?}"
        );
        // Fixed (the udev rule installed): a job connects first.
        device.state().claim_error = None;
        assert!(worker.handle().call(|c| c.session.storages()).is_ok());
    }
}
