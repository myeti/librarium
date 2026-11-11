//! MTP Kindles: the 2024+ Kindles that don't mount as a USB drive but
//! speak MTP (Media Transfer Protocol, PTP's extension for files).
//! Experimental, behind the `mtpEnabled` setting (on by default).
//!
//! Layers, bottom up:
//!
//! | Module          | Responsibility |
//! |-----------------|----------------|
//! | [`protocol`]    | the PTP/MTP wire format: containers, datasets, codes (pure) |
//! | [`ptp`]         | a PTP session over two USB bulk pipes: transactions, framing, streaming, busy retries |
//! | `usb`           | macOS/Linux: finding Kindles and opening their pipes with nusb |
//! | `wpd`           | Windows: Windows Portable Devices, the OS's own MTP stack |
//! | [`session`]     | what the layers above see: [`MtpSession`](session::MtpSession), [`Backend`](session::Backend), [`MtpError`] |
//! | [`devfs`]       | device paths (`documents/Dune.azw3`) ↔ MTP object ids, cached listings |
//! | [`worker`]      | one thread per device owning its session: jobs, health, reconnection |
//! | [`state`]       | [`MtpState`]: the devices plugged in, their workers, the setting |
//!
//! What Librarium does with them (books, the manifest, transfers, reading
//! progress) is in `commands/device/mtp/`.
//!
//! **Paths.** Over IPC an MTP device is `mtp://<port>` (its
//! `KindleDevice.mountPath`, e.g. `mtp://usb-20-1.2`) and a book on it
//! `mtp://<port>/documents/Dune.azw3`. `<port>` says where the device is
//! plugged in, never who it is: no serial number anywhere.
//!
//! Hardware-dependent assumptions are marked `TODO(device-validation)`:
//! none of this has met a real 2024 Kindle yet.

pub(crate) mod devfs;
#[cfg(test)]
pub(crate) mod fake;
#[cfg_attr(windows, allow(dead_code))]
pub(crate) mod protocol;
#[cfg(any(not(windows), test))]
#[cfg_attr(windows, allow(dead_code))]
pub(crate) mod ptp;
pub(crate) mod session;
pub(crate) mod state;
#[cfg(not(windows))]
mod usb;
pub(crate) mod worker;
#[cfg(windows)]
mod wpd;

use std::sync::Arc;

pub(crate) use session::MtpError;
pub use state::{MtpDevice, MtpState};

/// The scheme of MTP device paths.
pub(crate) const SCHEME: &str = "mtp://";

/// Whether `path` names an MTP device or a file on one.
pub fn is_mtp_path(path: &str) -> bool {
    path.starts_with(SCHEME)
}

/// `mtp://<key>`: the mount path of the device at `key`.
pub(crate) fn mount_path(key: &str) -> String {
    format!("{SCHEME}{key}")
}

/// `mtp://<key>/<rel>`: the path of the file `rel` (relative to the device root).
pub(crate) fn file_path(key: &str, rel: &str) -> String {
    format!("{SCHEME}{key}/{rel}")
}

/// The device key and the relative path (`""` for the device itself) of
/// an MTP path; `None` if it isn't one, or its key isn't a plain key.
pub(crate) fn split_path(path: &str) -> Option<(&str, &str)> {
    let rest = path.strip_prefix(SCHEME)?;
    let (key, rel) = rest.split_once('/').unwrap_or((rest, ""));
    let plain = key.starts_with(|c: char| c.is_ascii_alphanumeric())
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.');
    plain.then_some((key, rel.trim_end_matches('/')))
}

/// Amazon's USB vendor id (Lab126): Kindles, and Fire tablets.
pub(crate) const AMAZON_VENDOR_ID: u16 = 0x1949;

/// Whether a USB product name (or a WPD device description) is a Fire
/// tablet's: same vendor id as Kindles, also MTP, not an e-reader.
/// TODO(device-validation): record what a 2024 Kindle reports, to match
/// Kindles positively rather than excluding Fire tablets.
pub(crate) fn is_fire_tablet(product: Option<&str>) -> bool {
    product.is_some_and(|p| {
        let p = p.trim().to_ascii_lowercase();
        p.contains("fire") || p.starts_with("kf")
    })
}

/// Whether a Windows Portable Devices id (a PnP device interface path such
/// as `\\?\usb#vid_1949&pid_0004#<serial>#{6ac27878-…}`) is an Amazon USB
/// device's. Mass-storage volumes, which WPD lists too, have
/// `wpdbusenum`/`storage` paths and don't match. The id contains the serial
/// number: it is only ever compared, never logged.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn is_amazon_pnp_id(pnp_id: &str) -> bool {
    let id = pnp_id.to_ascii_lowercase();
    id.contains(&format!("usb#vid_{AMAZON_VENDOR_ID:04x}&")) && !id.contains("wpdbusenum")
}

/// This platform's backend: nusb on macOS/Linux, WPD on Windows.
pub(crate) fn platform_backend() -> Arc<dyn session::Backend> {
    #[cfg(not(windows))]
    {
        Arc::new(usb::UsbBackend)
    }
    #[cfg(windows)]
    {
        Arc::new(wpd::WpdBackend::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mtp_paths() {
        assert!(is_mtp_path("mtp://usb-20-1.2"));
        assert!(!is_mtp_path("/Volumes/Kindle"));
        assert_eq!(mount_path("usb-20-1.2"), "mtp://usb-20-1.2");
        assert_eq!(
            file_path("usb-20-1.2", "documents/Dune #1.azw3"),
            "mtp://usb-20-1.2/documents/Dune #1.azw3"
        );
        assert_eq!(split_path("mtp://usb-20-1.2"), Some(("usb-20-1.2", "")));
        assert_eq!(split_path("mtp://usb-20-1.2/"), Some(("usb-20-1.2", "")));
        assert_eq!(
            split_path("mtp://wpd-1/documents/SF/Dune.azw3"),
            Some(("wpd-1", "documents/SF/Dune.azw3"))
        );
        assert_eq!(split_path("mtp://"), None);
        assert_eq!(split_path("mtp://../x"), None, "keys are plain");
        assert_eq!(split_path("mtp://a b/x"), None);
        assert_eq!(split_path("/Volumes/Kindle"), None);
    }

    #[test]
    fn fire_tablets_are_not_kindles() {
        assert!(is_fire_tablet(Some("Fire")));
        assert!(is_fire_tablet(Some("KFTRWI")));
        assert!(is_fire_tablet(Some("Amazon Fire HD 10")));
        assert!(!is_fire_tablet(Some("Kindle")));
        assert!(!is_fire_tablet(Some("Amazon Kindle")));
        assert!(!is_fire_tablet(None));
    }

    #[test]
    fn windows_device_ids() {
        assert!(is_amazon_pnp_id(
            r"\\?\usb#vid_1949&pid_9981#G000XXXX#{6ac27878-a6fa-4155-ba85-f98f491d4f33}"
        ));
        assert!(is_amazon_pnp_id(
            r"\\?\USB#VID_1949&PID_0004&MI_00#7&1a2b#{6ac27878-a6fa-4155-ba85-f98f491d4f33}"
        ));
        assert!(
            !is_amazon_pnp_id(
                r"\\?\usb#vid_04e8&pid_6860#R58M#{6ac27878-a6fa-4155-ba85-f98f491d4f33}"
            ),
            "another vendor"
        );
        assert!(
            !is_amazon_pnp_id(
                r"\\?\swd#wpdbusenum#_??_usbstor#disk&ven_kindle&prod_internal_storage#vid_1949&#{6ac27878}"
            ),
            "a mass-storage volume"
        );
        assert!(
            !is_amazon_pnp_id(r"\\?\usb#vid_19490&pid_1#x"),
            "not a prefix match"
        );
    }
}
