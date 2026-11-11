//! "Safe eject": unmounting an e-reader (and, where the OS can, powering
//! its USB storage off) so it can be unplugged without losing writes.
//!
//! | OS      | How |
//! |---------|-----|
//! | macOS   | `diskutil eject <mount>` (unmounts and ejects the whole disk) |
//! | Linux   | `udisksctl unmount -b <device>`, else `gio mount -u <mount>`; then `udisksctl power-off -b <disk>` (best effort: once unmounted, unplugging is safe anyway) |
//! | Windows | not done yet: the user is told to use "Safely Remove Hardware" (TODO(windows-eject): `CM_Request_Device_EjectW` through the `windows` crate, which 2.3b brings for MTP, needs a Windows machine to test) |
//!
//! The commands run with a timeout ([`EJECT_TIMEOUT`]): `diskutil` can wait
//! a long time for a busy volume. The caller (`eject_device`) makes sure
//! the mount is a detected e-reader and that no transfer is running.
//!
//! The OS-specific steps go through a [`Runner`] so they can be tested
//! everywhere with a fake one.
//!
//! Validated on a Kindle Paperwhite 3 over USB on macOS (FAT32), 2026-11-09: the Kindle leaves USB mode cleanly.
//! TODO(device-validation): a Kobo, and Linux (GNOME and KDE).

use std::io;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::detect::linux::{mounted, LSBLK_ARGS};
use crate::error::{display_name, AppError, AppResult};
use crate::logging::Log;

/// How long an eject command may take.
pub(crate) const EJECT_TIMEOUT: Duration = Duration::from_secs(60);

/// What a command printed, and whether it succeeded.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Output {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    /// The first line that explains a failure (stderr first), shortened.
    fn reason(&self) -> String {
        let line = [&self.stderr, &self.stdout]
            .iter()
            .flat_map(|s| s.lines())
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("no details");
        line.chars().take(200).collect()
    }
}

/// Runs `program` with `args`.
pub(crate) type Runner<'a> = &'a dyn Fn(&str, &[&str]) -> io::Result<Output>;

/// Which OS's way of ejecting to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Platform {
    MacOs,
    Linux,
    Windows,
    Other,
}

impl Platform {
    pub(crate) fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(target_os = "linux") {
            Self::Linux
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else {
            Self::Other
        }
    }
}

/// Eject the e-reader mounted at `mount` (see the module docs).
pub(crate) fn eject(mount: &Path, log: Log) -> AppResult<()> {
    let run = |program: &str, args: &[&str]| run_with_timeout(program, args, EJECT_TIMEOUT);
    eject_with(Platform::current(), mount, &run, log)
}

/// [`eject`] for `platform`, running commands through `run`.
pub(crate) fn eject_with(platform: Platform, mount: &Path, run: Runner, log: Log) -> AppResult<()> {
    let name = display_name(mount);
    let failed = |reason: String| {
        AppError::Device(format!(
            "Could not eject {name} ({reason}). Close any file or window that uses it, then try again."
        ))
    };
    let mount_str = mount.to_string_lossy();
    match platform {
        Platform::MacOs => {
            let out = run("diskutil", &["eject", &mount_str]).map_err(|e| failed(e.to_string()))?;
            if !out.success {
                return Err(failed(out.reason()));
            }
            log(format!("[eject_device] {name} ejected"));
            Ok(())
        }
        Platform::Linux => eject_linux(mount, run, log).map_err(failed),
        Platform::Windows => Err(AppError::Device(format!(
            "Librarium can't eject {name} on Windows yet: use \"Safely Remove Hardware\" in the system tray."
        ))),
        Platform::Other => Err(AppError::Device(format!("Librarium can't eject {name} on this system."))),
    }
}

/// Unmount with udisks (the desktop's own way, no root needed), else gio;
/// then power the drive off. The error is the reason(s) unmounting failed.
fn eject_linux(mount: &Path, run: Runner, log: Log) -> Result<(), String> {
    let name = display_name(mount);
    let device = run("lsblk", &LSBLK_ARGS)
        .ok()
        .filter(|o| o.success)
        .and_then(|o| {
            mounted(o.stdout.as_bytes())
                .into_iter()
                .find(|d| Path::new(&d.mount) == mount)
        });
    let mut reasons = Vec::new();
    let mut attempt = |program: &str, args: &[&str]| match run(program, args) {
        Ok(out) if out.success => true,
        Ok(out) => {
            reasons.push(format!("{program}: {}", out.reason()));
            false
        }
        Err(e) => {
            reasons.push(format!("{program}: {e}"));
            false
        }
    };
    let mount_str = mount.to_string_lossy();
    let unmounted = device
        .as_ref()
        .is_some_and(|d| attempt("udisksctl", &["unmount", "-b", &d.name]))
        || attempt("gio", &["mount", "-u", &mount_str]);
    if !unmounted {
        return Err(reasons.join("; "));
    }
    log(format!("[eject_device] {name} unmounted"));
    if let Some(d) = &device {
        let disk = d.parent.as_deref().unwrap_or(&d.name);
        match run("udisksctl", &["power-off", "-b", disk]) {
            Ok(out) if out.success => log(format!("[eject_device] {name} powered off")),
            Ok(out) => log(format!(
                "[eject_device] {name} is unmounted (safe to unplug) but wasn't powered off: {}",
                out.reason()
            )),
            Err(e) => log(format!(
                "[eject_device] {name} is unmounted (safe to unplug) but wasn't powered off: {e}"
            )),
        }
    }
    Ok(())
}

/// Run `program`, killing it after `timeout`. For commands with little
/// output (it is read once the command has finished, so one that fills the
/// pipe would block until killed).
pub(crate) fn run_with_timeout(
    program: &str,
    args: &[&str],
    timeout: Duration,
) -> io::Result<Output> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let deadline = Instant::now() + timeout;
    while child.try_wait()?.is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("{program} didn't finish within {} s", timeout.as_secs()),
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let out = child.wait_with_output()?;
    Ok(Output {
        success: out.status.success(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging::{no_log, LogLines};
    use std::cell::RefCell;

    /// A fake [`Runner`]: records every command and answers from `answers`
    /// (`program` → output; an unknown program isn't installed).
    struct Fake {
        calls: RefCell<Vec<String>>,
        answers: Vec<(&'static str, Output)>,
    }

    impl Fake {
        fn new(answers: Vec<(&'static str, Output)>) -> Self {
            Self {
                calls: RefCell::new(vec![]),
                answers,
            }
        }

        fn run(&self, program: &str, args: &[&str]) -> io::Result<Output> {
            let call = format!("{program} {}", args.join(" "));
            self.calls.borrow_mut().push(call.clone());
            self.answers
                .iter()
                .find(|(prefix, _)| call.starts_with(prefix))
                .map(|(_, out)| out.clone())
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "not installed"))
        }

        fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }
    }

    fn ok(stdout: &str) -> Output {
        Output {
            success: true,
            stdout: stdout.into(),
            stderr: String::new(),
        }
    }

    fn fail(stderr: &str) -> Output {
        Output {
            success: false,
            stdout: String::new(),
            stderr: stderr.into(),
        }
    }

    const LSBLK: &str = r#"{"blockdevices":[{"name":"/dev/sdb","pkname":null,"label":null,"mountpoint":null,"children":[
        {"name":"/dev/sdb1","pkname":"/dev/sdb","label":"Kindle","mountpoint":"/media/me/Kindle"}]}]}"#;

    #[test]
    fn macos_uses_diskutil() {
        let fake = Fake::new(vec![("diskutil eject", ok("Disk /Volumes/Kindle ejected"))]);
        let run = |p: &str, a: &[&str]| fake.run(p, a);
        eject_with(Platform::MacOs, Path::new("/Volumes/Kindle"), &run, &no_log).unwrap();
        assert_eq!(fake.calls(), vec!["diskutil eject /Volumes/Kindle"]);

        let busy = Fake::new(vec![(
            "diskutil eject",
            fail("Volume Kindle on disk4s1 failed to unmount: dissented by PID 42"),
        )]);
        let run = |p: &str, a: &[&str]| busy.run(p, a);
        let err =
            eject_with(Platform::MacOs, Path::new("/Volumes/Kindle"), &run, &no_log).unwrap_err();
        assert_eq!(err.code(), "DEVICE");
        assert!(err.to_string().contains("dissented by PID 42"), "{err}");
    }

    #[test]
    fn linux_unmounts_then_powers_off_the_disk() {
        let fake = Fake::new(vec![
            ("lsblk", ok(LSBLK)),
            ("udisksctl unmount", ok("Unmounted")),
            ("udisksctl power-off", ok("")),
        ]);
        let run = |p: &str, a: &[&str]| fake.run(p, a);
        let lines = LogLines::default();
        eject_with(
            Platform::Linux,
            Path::new("/media/me/Kindle"),
            &run,
            &lines.sink(),
        )
        .unwrap();
        assert_eq!(
            fake.calls(),
            vec![
                "lsblk -J -p -o NAME,PKNAME,LABEL,MOUNTPOINT",
                "udisksctl unmount -b /dev/sdb1",
                "udisksctl power-off -b /dev/sdb",
            ]
        );
        assert!(lines.contains("powered off"));
    }

    #[test]
    fn linux_falls_back_on_gio_and_tolerates_no_power_off() {
        // No udisksctl at all: gio unmounts; power-off is then impossible, which is fine.
        let fake = Fake::new(vec![("lsblk", ok(LSBLK)), ("gio mount -u", ok(""))]);
        let run = |p: &str, a: &[&str]| fake.run(p, a);
        let lines = LogLines::default();
        eject_with(
            Platform::Linux,
            Path::new("/media/me/Kindle"),
            &run,
            &lines.sink(),
        )
        .unwrap();
        assert_eq!(
            fake.calls(),
            vec![
                "lsblk -J -p -o NAME,PKNAME,LABEL,MOUNTPOINT",
                "udisksctl unmount -b /dev/sdb1",
                "gio mount -u /media/me/Kindle",
                "udisksctl power-off -b /dev/sdb",
            ]
        );
        assert!(lines.contains("safe to unplug"));
    }

    #[test]
    fn linux_reports_why_unmounting_failed() {
        let fake = Fake::new(vec![
            ("lsblk", ok(LSBLK)),
            (
                "udisksctl unmount",
                fail("Error unmounting /dev/sdb1: target is busy"),
            ),
            ("gio", fail("gio: busy")),
        ]);
        let run = |p: &str, a: &[&str]| fake.run(p, a);
        let err = eject_with(
            Platform::Linux,
            Path::new("/media/me/Kindle"),
            &run,
            &no_log,
        )
        .unwrap_err();
        assert_eq!(err.code(), "DEVICE");
        assert!(
            err.to_string().contains("target is busy") && err.to_string().contains("gio: busy"),
            "{err}"
        );
        assert!(
            !fake.calls().iter().any(|c| c.contains("power-off")),
            "never powers off a mounted disk"
        );
    }

    #[test]
    fn windows_says_what_to_do_instead() {
        let fake = Fake::new(vec![]);
        let run = |p: &str, a: &[&str]| fake.run(p, a);
        let err = eject_with(Platform::Windows, Path::new("E:\\"), &run, &no_log).unwrap_err();
        assert_eq!(err.code(), "DEVICE");
        assert!(err.to_string().contains("Safely Remove Hardware"), "{err}");
        assert!(fake.calls().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn commands_are_killed_after_the_timeout() {
        let out = run_with_timeout(
            "sh",
            &["-c", "echo hello; echo oops >&2; exit 3"],
            Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!(
            out,
            Output {
                success: false,
                stdout: "hello\n".into(),
                stderr: "oops\n".into()
            }
        );
        let started = Instant::now();
        let err = run_with_timeout("sleep", &["30"], Duration::from_millis(200)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(
            run_with_timeout("librarium-no-such-program", &[], Duration::from_secs(1)).is_err()
        );
    }
}
