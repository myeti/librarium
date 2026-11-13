//! Running the system's own programs (`lsblk`, `udisksctl`, `gio`…).
//!
//! Inside an AppImage, the environment points into the image: its AppRun
//! sets `LD_LIBRARY_PATH`, GIO, GTK and GStreamer module paths and the like
//! to the bundled libraries. A system program started with that environment
//! loads the image's (older) GLib instead of its own and fails with
//! "undefined symbol" (seen with `udisksctl` and `gio` on Fedora).
//! [`host_command`] removes everything under the image's mount from the
//! child's environment, so system programs run as they would from a
//! terminal. Outside an AppImage it is `Command::new`.

use std::ffi::{OsStr, OsString};
use std::process::Command;

/// `Command::new(program)` with the AppImage's paths taken out of its
/// environment (see the module docs).
#[cfg_attr(windows, allow(dead_code))]
pub(crate) fn host_command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    if let Some(appdir) = std::env::var_os("APPDIR").filter(|d| !d.is_empty()) {
        for (key, change) in appimage_env_fixes(std::env::vars_os(), &appdir) {
            match change {
                Some(value) => cmd.env(&key, value),
                None => cmd.env_remove(&key),
            };
        }
    }
    cmd
}

/// The environment changes that take `appdir` out of `vars`: for a
/// `:`-separated list, the entries inside `appdir` are dropped (`Some` with
/// the rest, `None` when nothing is left); any other value inside `appdir`
/// is removed (`None`). Variables that don't mention it are left alone.
fn appimage_env_fixes(
    vars: impl IntoIterator<Item = (OsString, OsString)>,
    appdir: &OsStr,
) -> Vec<(OsString, Option<OsString>)> {
    let appdir = appdir.to_string_lossy();
    let appdir = appdir.trim_end_matches('/');
    let inside = |entry: &str| entry == appdir || entry.starts_with(&format!("{appdir}/"));
    let mut fixes = Vec::new();
    for (key, value) in vars {
        let Some(text) = value.to_str() else { continue };
        if !text.contains(appdir) {
            continue;
        }
        let kept: Vec<&str> = text
            .split(':')
            .filter(|e| !e.is_empty() && !inside(e))
            .collect();
        let all_inside = text.split(':').filter(|e| !e.is_empty()).all(inside);
        if kept.is_empty() || all_inside {
            fixes.push((key, None));
        } else if kept.len() != text.split(':').filter(|e| !e.is_empty()).count() {
            fixes.push((key, Some(OsString::from(kept.join(":")))));
        }
    }
    fixes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
        pairs
            .iter()
            .map(|(k, v)| (OsString::from(k), OsString::from(v)))
            .collect()
    }

    #[test]
    fn the_appimage_paths_leave_a_system_programs_environment() {
        let appdir = OsStr::new("/tmp/.mount_LibrarXYZ");
        let mut fixes = appimage_env_fixes(
            vars(&[
                (
                    "LD_LIBRARY_PATH",
                    "/tmp/.mount_LibrarXYZ/usr/lib:/tmp/.mount_LibrarXYZ/usr/lib64",
                ),
                (
                    "XDG_DATA_DIRS",
                    "/tmp/.mount_LibrarXYZ/usr/share:/usr/local/share:/usr/share",
                ),
                (
                    "GIO_MODULE_DIR",
                    "/tmp/.mount_LibrarXYZ/usr/lib/gio/modules",
                ),
                ("PATH", "/tmp/.mount_LibrarXYZ/usr/bin:/usr/bin:/bin"),
                ("HOME", "/home/ann"),
                ("APPIMAGE", "/home/ann/Librarium.AppImage"),
                // A neighbour whose name merely starts like the mount.
                ("OTHER", "/tmp/.mount_LibrarXYZ2/x"),
            ]),
            appdir,
        );
        fixes.sort();
        let fix = |k: &str| {
            fixes
                .iter()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.clone())
        };
        assert_eq!(fix("LD_LIBRARY_PATH"), Some(None));
        assert_eq!(fix("GIO_MODULE_DIR"), Some(None));
        assert_eq!(
            fix("XDG_DATA_DIRS"),
            Some(Some(OsString::from("/usr/local/share:/usr/share")))
        );
        assert_eq!(fix("PATH"), Some(Some(OsString::from("/usr/bin:/bin"))));
        assert_eq!(fix("HOME"), None);
        assert_eq!(fix("APPIMAGE"), None);
        assert_eq!(fix("OTHER"), None);
    }
}
