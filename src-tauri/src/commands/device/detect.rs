//! Finding the e-readers connected as USB mass storage: every Kindle and
//! Kobo volume (several at once), with its model, a stable id and its free
//! and total space.
//!
//! A volume counts only if its **name** says "kindle" or "kobo" *and* it has
//! the matching **structure** ([`has_device_structure`]: `documents/` for a
//! Kindle, `.kobo/` for a Kobo). The name filter means unrelated volumes —
//! a sleeping NAS share in particular — are never even looked into; the
//! structure check rejects look-alikes such as the "Kindle Previewer"
//! installer image. (A Kindle or Kobo volume the user renamed isn't found.)
//!
//! Ids ([`device_id`]): each device gets a UUID, stored in its manifest
//! (`.librarium/device.json`) the first time Librarium sees it, so a device
//! keeps its id across reconnections and a different device at the same
//! mount point has a different one — which is how the frontend notices a
//! swap. A device that can't be written to gets `"<type>:<mount path>"`.
//! Ids never contain a serial number (the Kobo's is in `.kobo/version`).

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use super::manifest;
use crate::logging::Log;
use crate::models::device::{KindleDevice, CONNECTION_MASS_STORAGE};

/// The kinds of e-reader supported over mass storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum DeviceKind {
    Kindle,
    Kobo,
}

impl DeviceKind {
    /// `KindleDevice::device_type`.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Kindle => "kindle",
            Self::Kobo => "kobo",
        }
    }

    /// The kind a volume *name* suggests ("Kindle", "KOBOeReader").
    pub(crate) fn from_volume_name(name: &str) -> Option<Self> {
        let lower = name.to_lowercase();
        if lower.contains("kindle") {
            Some(Self::Kindle)
        } else if lower.contains("kobo") {
            Some(Self::Kobo)
        } else {
            None
        }
    }

    /// The format books are converted to for this device.
    fn preferred_format(self) -> &'static str {
        match self {
            // Kindle never reads EPUB from its filesystem.
            Self::Kindle => "azw3",
            Self::Kobo => "epub",
        }
    }
}

/// Structural sanity check for a volume whose *name* looks like an e-reader:
/// a Kindle has a `documents/` folder at its root, a Kobo has `.kobo/`.
///
/// Validated on a Kindle Paperwhite 3 over USB on macOS (FAT32), 2026-11-09.
/// TODO(device-validation): Scribe, a freshly reset Kindle, and Kobos.
pub(crate) fn has_device_structure(mount: &Path, kind: DeviceKind) -> bool {
    match kind {
        DeviceKind::Kindle => mount.join("documents").is_dir(),
        DeviceKind::Kobo => mount.join(".kobo").is_dir(),
    }
}

/// What kind of e-reader the folder `mount` is the root of, from its
/// structure alone (a Kobo may also have a `documents/` folder that
/// Librarium created, so `.kobo/` wins). Used on paths received from the
/// webview, before writing to them.
pub(crate) fn kind_at(mount: &Path) -> Option<DeviceKind> {
    [DeviceKind::Kobo, DeviceKind::Kindle]
        .into_iter()
        .find(|&k| has_device_structure(mount, k))
}

/// Free (for this user) and total bytes of the volume holding `path`;
/// `None` when the OS can't say.
pub(crate) fn space_info(path: &Path) -> (Option<u64>, Option<u64>) {
    match fs4::statvfs(path) {
        Ok(stats) => (Some(stats.available_space()), Some(stats.total_space())),
        Err(_) => (None, None),
    }
}

// ── All connected devices ─────────────────────────────────────────────────────

/// A volume that looks like an e-reader: where it is mounted, what it seems
/// to be, and the name to show if the model is unknown.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Candidate {
    pub mount: PathBuf,
    pub kind: DeviceKind,
    pub label: String,
}

/// Every connected e-reader (blocking: volume listing, a few small reads,
/// and on first sight a small write of the manifest's device id).
pub(crate) fn detect_all(log: Log) -> Vec<KindleDevice> {
    let mut seen_mounts = HashSet::new();
    let mut seen_ids = HashSet::new();
    platform_candidates()
        .into_iter()
        .filter(|c| has_device_structure(&c.mount, c.kind) && seen_mounts.insert(c.mount.clone()))
        .map(|c| {
            let mut device = describe(&c, log);
            // Two devices can't share an id (a `.librarium` folder copied
            // from one device to another): the second one gets its mount.
            if !seen_ids.insert(device.id.clone()) {
                device.id = fallback_id(c.kind, &c.mount);
            }
            device
        })
        .collect()
}

/// The [`KindleDevice`] for a candidate that passed the structure check.
fn describe(c: &Candidate, log: Log) -> KindleDevice {
    let model = match c.kind {
        DeviceKind::Kindle => kindle_model(&c.mount),
        DeviceKind::Kobo => kobo_model(&c.mount),
    };
    // Without a model, the volume's name ("Kindle 1" tells two Kindles apart).
    let name = model
        .clone()
        .unwrap_or_else(|| match (c.label.trim(), c.kind) {
            ("", DeviceKind::Kindle) => "Kindle".to_string(),
            ("", DeviceKind::Kobo) => "Kobo".to_string(),
            (label, _) => label.to_string(),
        });
    let (free, total) = space_info(&c.mount);
    KindleDevice {
        id: device_id(&c.mount, c.kind, log),
        name,
        mount_path: c.mount.to_string_lossy().into_owned(),
        device_type: c.kind.as_str().to_string(),
        connection: CONNECTION_MASS_STORAGE.to_string(),
        preferred_format: c.kind.preferred_format().to_string(),
        model,
        free_space_bytes: free,
        total_space_bytes: total,
    }
}

/// The device's stable id (see the module docs).
pub(crate) fn device_id(mount: &Path, kind: DeviceKind, log: Log) -> String {
    match manifest::ensure_device_id(mount, log) {
        Ok(id) => id,
        Err(_) => fallback_id(kind, mount),
    }
}

/// The id of a device without a usable manifest: stable while it stays
/// mounted at the same place.
fn fallback_id(kind: DeviceKind, mount: &Path) -> String {
    format!("{}:{}", kind.as_str(), mount.display())
}

#[cfg(target_os = "macos")]
fn platform_candidates() -> Vec<Candidate> {
    volumes_in(Path::new("/Volumes"))
}

#[cfg(target_os = "windows")]
fn platform_candidates() -> Vec<Candidate> {
    windows::candidates()
}

#[cfg(target_os = "linux")]
fn platform_candidates() -> Vec<Candidate> {
    linux::candidates()
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn platform_candidates() -> Vec<Candidate> {
    Vec::new()
}

/// The volumes in `volumes_dir` (`/Volumes` on macOS) whose name says
/// "kindle" or "kobo", in name order. Reads the directory itself (no `ls`).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn volumes_in(volumes_dir: &Path) -> Vec<Candidate> {
    let Ok(entries) = fs::read_dir(volumes_dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .filter(|name| !name.starts_with('.'))
        .collect();
    names.sort();
    names
        .into_iter()
        .filter_map(|name| {
            let kind = DeviceKind::from_volume_name(&name)?;
            Some(Candidate {
                mount: volumes_dir.join(&name),
                kind,
                label: name,
            })
        })
        .collect()
}

// ── Windows ───────────────────────────────────────────────────────────────────

/// Windows volume enumeration: PowerShell's `Get-Volume` (Windows 10/11),
/// else `wmic` (deprecated, still present for compatibility). The parsers
/// are plain functions, tested on every platform.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) mod windows {
    use super::{Candidate, DeviceKind};
    use std::path::PathBuf;

    /// `(drive letter, label)` pairs from `Get-Volume | Select-Object
    /// DriveLetter,FileSystemLabel | ConvertTo-Json`: one object for one
    /// volume, an array for several. Volumes without a letter are skipped.
    pub(crate) fn parse_powershell(json: &[u8]) -> Vec<(String, String)> {
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(json) else {
            return Vec::new();
        };
        let items = match value {
            serde_json::Value::Array(items) => items,
            other => vec![other],
        };
        items
            .iter()
            .filter_map(|v| {
                let letter = v.get("DriveLetter")?.as_str()?.trim().to_string();
                let label = v
                    .get("FileSystemLabel")
                    .and_then(|l| l.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                (letter.len() == 1 && letter.chars().all(|c| c.is_ascii_alphabetic()))
                    .then_some((letter, label))
            })
            .collect()
    }

    /// `(drive, label)` pairs from `wmic logicaldisk get DeviceID,VolumeName`
    /// (a header line, then `E:  Kindle`).
    pub(crate) fn parse_wmic(text: &str) -> Vec<(String, String)> {
        text.lines()
            .skip(1)
            .filter_map(|line| {
                let mut parts = line.split_whitespace();
                let drive = parts.next()?.trim_end_matches(':').to_string();
                let label = parts.collect::<Vec<_>>().join(" ");
                (!label.is_empty() && drive.len() == 1).then_some((drive, label))
            })
            .collect()
    }

    /// Candidates from `(letter, label)` pairs, kept when the label names an e-reader.
    pub(crate) fn to_candidates(volumes: Vec<(String, String)>) -> Vec<Candidate> {
        volumes
            .into_iter()
            .filter_map(|(letter, label)| {
                let kind = DeviceKind::from_volume_name(&label)?;
                Some(Candidate {
                    mount: PathBuf::from(format!("{letter}:\\")),
                    kind,
                    label,
                })
            })
            .collect()
    }

    #[cfg(target_os = "windows")]
    pub(super) fn candidates() -> Vec<Candidate> {
        use super::has_device_structure;
        let real = |c: &Candidate| has_device_structure(&c.mount, c.kind);
        let powershell = silent_cmd("powershell")
            .args([
                "-NoProfile",
                "-Command",
                "Get-Volume | Where-Object { $_.FileSystemLabel -like '*kindle*' -or $_.FileSystemLabel -like '*kobo*' } \
                 | Select-Object DriveLetter,FileSystemLabel | ConvertTo-Json",
            ])
            .output()
            .map(|o| to_candidates(parse_powershell(&o.stdout)))
            .unwrap_or_default();
        if powershell.iter().any(real) {
            return powershell;
        }
        // Only when PowerShell found nothing real: a look-alike volume it
        // found must not hide the device from the fallback.
        silent_cmd("wmic")
            .args(["logicaldisk", "get", "DeviceID,VolumeName"])
            .output()
            .map(|o| to_candidates(parse_wmic(&String::from_utf8_lossy(&o.stdout))))
            .unwrap_or_default()
    }

    /// A command that doesn't open a console window.
    #[cfg(target_os = "windows")]
    pub(crate) fn silent_cmd(program: &str) -> std::process::Command {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let mut cmd = std::process::Command::new(program);
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd
    }
}

// ── Linux ─────────────────────────────────────────────────────────────────────

/// Linux volume enumeration with `lsblk -J`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) mod linux {
    use super::{Candidate, DeviceKind};
    use serde_json::Value;
    use std::path::PathBuf;

    /// The arguments `lsblk` is run with (shared with eject, which needs
    /// the device names too).
    pub(crate) const LSBLK_ARGS: [&str; 4] = ["-J", "-p", "-o", "NAME,PKNAME,LABEL,MOUNTPOINT"];

    /// One mounted block device from `lsblk -J`.
    #[derive(Debug, Clone, PartialEq)]
    pub(crate) struct BlockDevice {
        /// `/dev/sdb1` (with `-p`).
        pub name: String,
        /// The parent disk (`/dev/sdb`), when it is a partition.
        pub parent: Option<String>,
        pub label: String,
        pub mount: String,
    }

    /// Every mounted block device in `lsblk -J` output, disks and partitions
    /// alike (an e-reader may expose a filesystem on the whole disk, without
    /// a partition table). Accepts lsblk's `mountpoint` and the newer
    /// `mountpoints` array.
    pub(crate) fn mounted(json: &[u8]) -> Vec<BlockDevice> {
        fn walk(node: &Value, out: &mut Vec<BlockDevice>) {
            let text = |key: &str| {
                node.get(key)
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string()
            };
            let mount = node
                .get("mountpoint")
                .and_then(Value::as_str)
                .or_else(|| {
                    node.get("mountpoints")?
                        .as_array()?
                        .iter()
                        .find_map(Value::as_str)
                })
                .unwrap_or("");
            if !mount.is_empty() {
                let parent = Some(text("pkname")).filter(|p| !p.is_empty());
                out.push(BlockDevice {
                    name: text("name"),
                    parent,
                    label: text("label"),
                    mount: mount.to_string(),
                });
            }
            for child in node
                .get("children")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                walk(child, out);
            }
        }
        let mut out = Vec::new();
        if let Ok(value) = serde_json::from_slice::<Value>(json) {
            for dev in value
                .get("blockdevices")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                walk(dev, &mut out);
            }
        }
        out
    }

    /// Candidates from `lsblk` output: mounted volumes whose label names an e-reader.
    pub(crate) fn to_candidates(json: &[u8]) -> Vec<Candidate> {
        mounted(json)
            .into_iter()
            .filter_map(|d| {
                let kind = DeviceKind::from_volume_name(&d.label)?;
                Some(Candidate {
                    mount: PathBuf::from(&d.mount),
                    kind,
                    label: d.label,
                })
            })
            .collect()
    }

    #[cfg(target_os = "linux")]
    pub(super) fn candidates() -> Vec<Candidate> {
        std::process::Command::new("lsblk")
            .args(LSBLK_ARGS)
            .output()
            .map(|o| to_candidates(&o.stdout))
            .unwrap_or_default()
    }
}

// ── Model names ───────────────────────────────────────────────────────────────

/// The Kindle's model, from `system/version.txt` ("Kindle 5.16.2 (…)").
///
/// TODO(2.3): firmware only gives a generation (every 5.16+ Kindle reads as
/// "Paperwhite 6"); the serial or USB product id would give the model.
fn kindle_model(mount: &Path) -> Option<String> {
    let content = fs::read_to_string(mount.join("system").join("version.txt")).ok()?;
    let fw = content.split_whitespace().nth(1)?;
    Some(format!("Kindle {}", kindle_model_from_fw(fw)))
}

/// Map a firmware version string to a Kindle model name.
/// Kindle firmware minor versions correlate with device generations:
///   5.6–5.9   → Paperwhite 3 / Voyage (7th gen, 2014–2015)
///   5.10–5.12 → Paperwhite 4 / Oasis 2–3 (2018–2019)
///   5.13–5.14 → Paperwhite 5 / Kindle 11th gen (2021–2022)
///   5.15      → Scribe / Paperwhite SE (2022)
///   5.16+     → Paperwhite 6 / Colorsoft (2024+)
fn kindle_model_from_fw(fw: &str) -> &'static str {
    let parts: Vec<u32> = fw.split('.').filter_map(|p| p.parse().ok()).collect();
    if parts.len() < 2 || parts[0] != 5 {
        return "Kindle";
    }
    match parts[1] {
        0..=5 => "Kindle",
        6..=9 => "Paperwhite 3",
        10..=12 => "Paperwhite 4 / Oasis",
        13..=14 => "Paperwhite 5",
        15 => "Scribe",
        _ => "Paperwhite 6",
    }
}

/// The Kobo's model, from `.kobo/Kobo/Kobo eReader.conf` (`model=`), else
/// its firmware version from `.kobo/version`.
fn kobo_model(mount: &Path) -> Option<String> {
    let conf = mount.join(".kobo").join("Kobo").join("Kobo eReader.conf");
    if let Ok(content) = fs::read_to_string(&conf) {
        for line in content.lines() {
            if let Some(model) = line
                .strip_prefix("model=")
                .map(str::trim)
                .filter(|m| !m.is_empty())
            {
                return Some(format!("Kobo {model}"));
            }
        }
    }
    let version = fs::read_to_string(mount.join(".kobo").join("version")).ok()?;
    kobo_firmware(&version).map(|fw| format!("Kobo fw {fw}"))
}

/// Extract the firmware version from the contents of `.kobo/version`.
///
/// The file is one comma-separated line whose **first field is the device
/// serial number**, e.g. `N418170012345,3.0.35+,4.20.14622,3.0.35+,…`, which
/// must never reach the device name, the UI or analytics. Only the third field (firmware) is kept, and only if it
/// looks like a version number.
///
/// TODO(device-validation): confirm the field order on a current Kobo.
fn kobo_firmware(version_file: &str) -> Option<String> {
    let fw = version_file.trim().split(',').nth(2)?.trim();
    let looks_like_version = !fw.is_empty() && fw.chars().all(|c| c.is_ascii_digit() || c == '.');
    looks_like_version.then(|| fw.to_string())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging::no_log;

    fn kinds(candidates: &[Candidate]) -> Vec<(String, DeviceKind)> {
        candidates
            .iter()
            .map(|c| (c.label.clone(), c.kind))
            .collect()
    }

    #[test]
    fn volume_scan_finds_every_e_reader_by_name() {
        let volumes = tempfile::tempdir().unwrap();
        for name in [
            "Kindle",
            "Kindle 1",
            "KOBOeReader",
            "Macintosh HD",
            ".hidden kindle",
        ] {
            fs::create_dir(volumes.path().join(name)).unwrap();
        }
        assert_eq!(
            kinds(&volumes_in(volumes.path())),
            vec![
                ("KOBOeReader".into(), DeviceKind::Kobo),
                ("Kindle".into(), DeviceKind::Kindle),
                ("Kindle 1".into(), DeviceKind::Kindle),
            ]
        );
        assert!(volumes_in(&volumes.path().join("missing")).is_empty());
    }

    #[test]
    fn several_devices_each_get_their_own_stable_id_and_space() {
        let volumes = tempfile::tempdir().unwrap();
        // Two Kindles, one Kobo, and an installer image named like a Kindle.
        fs::create_dir_all(volumes.path().join("Kindle/documents")).unwrap();
        fs::create_dir_all(volumes.path().join("Kindle 1/documents")).unwrap();
        fs::create_dir_all(volumes.path().join("KOBOeReader/.kobo")).unwrap();
        fs::create_dir_all(volumes.path().join("Kindle Previewer 3")).unwrap();
        let candidates: Vec<Candidate> = volumes_in(volumes.path());

        let detect = || -> Vec<KindleDevice> {
            candidates
                .iter()
                .filter(|c| has_device_structure(&c.mount, c.kind))
                .map(|c| describe(c, &no_log))
                .collect()
        };
        let first = detect();
        assert_eq!(
            first
                .iter()
                .map(|d| d.device_type.as_str())
                .collect::<Vec<_>>(),
            vec!["kobo", "kindle", "kindle"]
        );
        let ids: HashSet<&str> = first.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids.len(), 3, "one id per device");
        assert!(
            first.iter().all(|d| uuid::Uuid::parse_str(&d.id).is_ok()),
            "{first:?}"
        );
        assert!(first
            .iter()
            .all(|d| d.free_space_bytes.is_some() && d.total_space_bytes.is_some()));
        let again = detect();
        assert_eq!(
            first.iter().map(|d| &d.id).collect::<Vec<_>>(),
            again.iter().map(|d| &d.id).collect::<Vec<_>>(),
            "stable across polls"
        );

        // Another Kindle mounted at the same place (a swap): another id.
        let swapped = volumes.path().join("Kindle");
        fs::remove_dir_all(&swapped).unwrap();
        fs::create_dir_all(swapped.join("documents")).unwrap();
        let after = detect();
        assert_ne!(after[1].id, first[1].id, "a swap is visible");
        assert_eq!(after[2].id, first[2].id);
    }

    #[test]
    fn a_read_only_device_gets_a_fallback_id() {
        let volumes = tempfile::tempdir().unwrap();
        let mount = volumes.path().join("Kindle");
        fs::create_dir_all(mount.join("documents")).unwrap();
        // A damaged manifest isn't rewritten by detection.
        fs::create_dir_all(mount.join(".librarium")).unwrap();
        fs::write(mount.join(".librarium/device.json"), b"garbage").unwrap();
        assert_eq!(
            device_id(&mount, DeviceKind::Kindle, &no_log),
            format!("kindle:{}", mount.display())
        );
        assert_eq!(
            fs::read(mount.join(".librarium/device.json")).unwrap(),
            b"garbage"
        );
    }

    #[test]
    fn structure_decides_the_kind_of_a_path_from_the_webview() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(kind_at(dir.path()), None);
        fs::create_dir(dir.path().join("documents")).unwrap();
        assert_eq!(kind_at(dir.path()), Some(DeviceKind::Kindle));
        fs::create_dir(dir.path().join(".kobo")).unwrap();
        assert_eq!(
            kind_at(dir.path()),
            Some(DeviceKind::Kobo),
            "a Kobo with a documents/ folder Librarium made"
        );
    }

    #[test]
    fn kindle_and_kobo_models() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("system")).unwrap();
        fs::write(
            dir.path().join("system/version.txt"),
            "Kindle 5.13.7 (123456)",
        )
        .unwrap();
        assert_eq!(
            kindle_model(dir.path()).as_deref(),
            Some("Kindle Paperwhite 5")
        );
        assert_eq!(kindle_model_from_fw("4.1"), "Kindle");
        assert_eq!(kindle_model_from_fw("5.16.2"), "Paperwhite 6");

        fs::create_dir_all(dir.path().join(".kobo/Kobo")).unwrap();
        fs::write(
            dir.path().join(".kobo/version"),
            "N418170012345,3.0.35+,4.20.14622,3.0.35+",
        )
        .unwrap();
        assert_eq!(
            kobo_model(dir.path()).as_deref(),
            Some("Kobo fw 4.20.14622")
        );
        fs::write(
            dir.path().join(".kobo/Kobo/Kobo eReader.conf"),
            "[x]\nmodel=Clara HD\n",
        )
        .unwrap();
        assert_eq!(kobo_model(dir.path()).as_deref(), Some("Kobo Clara HD"));
    }

    #[test]
    fn kobo_firmware_never_returns_the_serial() {
        assert_eq!(
            kobo_firmware("N418170012345,3.0.35+,4.20.14622,3.0.35+,3.0.35+,00000000-0000-0000-0000-000000000370\n"),
            Some("4.20.14622".to_string())
        );
        assert_eq!(kobo_firmware("N418170012345"), None);
        assert_eq!(kobo_firmware("N418170012345,a,b"), None);
        assert_eq!(kobo_firmware(""), None);
    }

    #[test]
    fn windows_volume_lists() {
        let one = br#"{"DriveLetter":"E","FileSystemLabel":"Kindle"}"#;
        assert_eq!(
            windows::parse_powershell(one),
            vec![("E".into(), "Kindle".into())]
        );
        let several = br#"[{"DriveLetter":"E","FileSystemLabel":"Kindle"},{"DriveLetter":null,"FileSystemLabel":"Kindle"},{"DriveLetter":"F","FileSystemLabel":"KOBOeReader"}]"#;
        let candidates = windows::to_candidates(windows::parse_powershell(several));
        assert_eq!(
            candidates,
            vec![
                Candidate {
                    mount: PathBuf::from("E:\\"),
                    kind: DeviceKind::Kindle,
                    label: "Kindle".into()
                },
                Candidate {
                    mount: PathBuf::from("F:\\"),
                    kind: DeviceKind::Kobo,
                    label: "KOBOeReader".into()
                },
            ]
        );
        assert!(windows::parse_powershell(b"").is_empty());

        let wmic = "DeviceID  VolumeName\r\nC:        Windows\r\nE:        Kindle\r\nF:\r\nG:        KOBO eReader\r\n";
        assert_eq!(
            kinds(&windows::to_candidates(windows::parse_wmic(wmic))),
            vec![
                ("Kindle".into(), DeviceKind::Kindle),
                ("KOBO eReader".into(), DeviceKind::Kobo),
            ]
        );
    }

    #[test]
    fn linux_block_devices_at_any_level() {
        let json = br#"{"blockdevices":[
            {"name":"/dev/sda","pkname":null,"label":null,"mountpoint":null,"children":[
                {"name":"/dev/sda1","pkname":"/dev/sda","label":"root","mountpoint":"/"}]},
            {"name":"/dev/sdb","pkname":null,"label":"Kindle","mountpoint":"/media/me/Kindle"},
            {"name":"/dev/sdc","pkname":null,"label":null,"mountpoint":null,"children":[
                {"name":"/dev/sdc1","pkname":"/dev/sdc","label":"KOBOeReader","mountpoints":["/media/me/KOBOeReader"]}]},
            {"name":"/dev/sdd","pkname":null,"label":"Kindle","mountpoint":null}
        ]}"#;
        let devices = linux::mounted(json);
        assert_eq!(
            devices[1],
            linux::BlockDevice {
                name: "/dev/sdb".into(),
                parent: None,
                label: "Kindle".into(),
                mount: "/media/me/Kindle".into(),
            }
        );
        assert_eq!(devices[2].parent.as_deref(), Some("/dev/sdc"));
        let candidates = linux::to_candidates(json);
        assert_eq!(
            candidates
                .iter()
                .map(|c| (c.mount.clone(), c.kind))
                .collect::<Vec<_>>(),
            vec![
                (PathBuf::from("/media/me/Kindle"), DeviceKind::Kindle),
                (PathBuf::from("/media/me/KOBOeReader"), DeviceKind::Kobo)
            ]
        );
        assert!(linux::mounted(b"not json").is_empty());
    }
}
