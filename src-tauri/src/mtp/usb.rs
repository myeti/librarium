//! macOS and Linux: MTP Kindles over raw USB, with nusb (pure Rust: IOKit
//! on macOS, usbfs on Linux; no libusb). Neither OS has an MTP driver, so
//! Librarium claims the device's MTP interface and speaks PTP itself
//! ([`super::ptp`]).
//!
//! **Which devices.** Amazon's vendor id (0x1949) *and* an MTP interface:
//! the PTP "Still Image" class (0x06/0x01/0x01) or a vendor-specific
//! interface named "MTP" (how Android-style responders announce it).
//! Mass-storage Kindles (an 0x08 interface) are left alone, and so are
//! Fire tablets (same vendor, "Fire"/"KF…" product names). Nothing is
//! opened to find out: interface classes come with the enumeration.
//! TODO(device-validation): record a 2024 Kindle's product id, product
//! string and interface class/subclass/protocol/name, and narrow the match.
//!
//! **Claim failures** are reported with what to do ([`super::session::claim_message`]):
//! on macOS, Image Capture's `ptpcamerad` grabs PTP-class devices, and
//! Android File Transfer or OpenMTP may hold the interface; on Linux,
//! gvfs-mtp or kio-mtp may, and without Librarium's udev rule
//! (`src-tauri/linux/70-librarium-kindle.rules`) the device can't be opened
//! at all. TODO(device-validation): check which of these actually grab a
//! 2024 Kindle, and the errors macOS/Linux return.

use std::time::{Duration, Instant};

use nusb::descriptors::TransferType;
use nusb::transfer::{
    Buffer, Bulk, ControlIn, ControlOut, ControlType, Direction, In, Out, Recipient, TransferError,
};
use nusb::{DeviceInfo, Endpoint, Interface, MaybeFuture};

use super::protocol::{self, class_request, rc};
use super::ptp::{BulkPipe, PtpSession};
use super::session::{Backend, Candidate, ClaimKind, MtpError, MtpSession};
use super::{is_fire_tablet, AMAZON_VENDOR_ID};

/// Timeout of class requests on the control endpoint.
const CONTROL_TIMEOUT: Duration = Duration::from_secs(2);

/// How long to wait for a device to become idle after a cancel or reset.
const IDLE_TIMEOUT: Duration = Duration::from_secs(5);

/// Whether a USB interface speaks PTP/MTP (see the module docs).
pub(crate) fn is_mtp_interface(class: u8, subclass: u8, protocol: u8, name: Option<&str>) -> bool {
    let still_image = class == 0x06 && subclass == 0x01 && protocol == 0x01;
    let named_mtp = class == 0xFF && name.is_some_and(|n| n.to_ascii_uppercase().contains("MTP"));
    still_image || named_mtp
}

/// The device key: the bus and port path it is plugged into
/// (`usb-20-1.2`), stable across replugs into the same port, and never
/// the serial number.
pub(crate) fn location_key(bus: &str, ports: &[u8], address: u8) -> String {
    let bus: String = bus.chars().filter(char::is_ascii_alphanumeric).collect();
    let place = if ports.is_empty() {
        format!("a{address}")
    } else {
        ports
            .iter()
            .map(u8::to_string)
            .collect::<Vec<_>>()
            .join(".")
    };
    format!("usb-{}-{place}", if bus.is_empty() { "0" } else { &bus })
}

fn key_of(info: &DeviceInfo) -> String {
    location_key(info.bus_id(), info.port_chain(), info.device_address())
}

fn candidate(info: &DeviceInfo) -> Option<Candidate> {
    if info.vendor_id() != AMAZON_VENDOR_ID {
        return None;
    }
    let mtp = info
        .interfaces()
        .any(|i| is_mtp_interface(i.class(), i.subclass(), i.protocol(), i.interface_string()));
    if !mtp || is_fire_tablet(info.product_string()) {
        return None;
    }
    let key = key_of(info);
    let label = info
        .product_string()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .unwrap_or("Kindle")
        .to_string();
    Some(Candidate {
        native: key.clone(),
        key,
        label,
    })
}

/// The macOS/Linux [`Backend`].
pub(crate) struct UsbBackend;

impl Backend for UsbBackend {
    fn enumerate(&self) -> Vec<Candidate> {
        let mut found: Vec<Candidate> = match nusb::list_devices().wait() {
            Ok(devices) => devices.filter_map(|d| candidate(&d)).collect(),
            Err(_) => Vec::new(),
        };
        found.sort_by(|a, b| a.key.cmp(&b.key));
        found
    }

    fn open(&self, candidate: &Candidate) -> Result<Box<dyn MtpSession>, MtpError> {
        let find = || {
            nusb::list_devices()
                .wait()
                .ok()
                .and_then(|mut devices| {
                    devices.find(|d| {
                        d.vendor_id() == AMAZON_VENDOR_ID && key_of(d) == candidate.native
                    })
                })
                .ok_or(MtpError::Disconnected)
        };
        match PtpSession::open(UsbPipe::open(&find()?)?) {
            Ok(session) => Ok(Box::new(session)),
            // A device left in the middle of a transaction (a crashed app, a
            // pulled cable): reset its responder and try once more.
            Err(e) if e.is_fatal() => {
                let mut pipe = UsbPipe::open(&find()?)?;
                let _ = pipe.reset();
                Ok(Box::new(PtpSession::open(pipe)?))
            }
            Err(e) => Err(e),
        }
    }
}

/// The claimed MTP interface and its two bulk endpoints.
struct UsbPipe {
    interface: Interface,
    bulk_in: Endpoint<Bulk, In>,
    bulk_out: Endpoint<Bulk, Out>,
}

/// A failure to open or claim the device.
fn open_error(e: nusb::Error) -> MtpError {
    let detail = e.to_string();
    match e.kind() {
        nusb::ErrorKind::Disconnected => MtpError::Disconnected,
        nusb::ErrorKind::Busy => MtpError::Claim {
            kind: ClaimKind::Busy,
            detail,
        },
        nusb::ErrorKind::PermissionDenied => MtpError::Claim {
            kind: ClaimKind::PermissionDenied,
            detail,
        },
        _ => MtpError::Claim {
            kind: ClaimKind::Other,
            detail,
        },
    }
}

fn transfer_error(e: TransferError) -> MtpError {
    match e {
        TransferError::Disconnected => MtpError::Disconnected,
        TransferError::Cancelled => {
            MtpError::Transport("the Kindle stopped responding (timeout)".into())
        }
        TransferError::Stall => MtpError::Transport("the Kindle stalled the transfer".into()),
        other => MtpError::Transport(format!("USB error: {other}")),
    }
}

impl UsbPipe {
    fn open(info: &DeviceInfo) -> Result<Self, MtpError> {
        let device = info.open().wait().map_err(open_error)?;
        let config = device.active_configuration().map_err(|e| {
            MtpError::Transport(format!("the device has no active USB configuration ({e})"))
        })?;
        let mut found = None;
        for alt in config.interface_alt_settings() {
            if alt.alternate_setting() != 0 {
                continue;
            }
            let name = info
                .interfaces()
                .find(|i| i.interface_number() == alt.interface_number())
                .and_then(|i| i.interface_string());
            if !is_mtp_interface(alt.class(), alt.subclass(), alt.protocol(), name) {
                continue;
            }
            let (mut ep_in, mut ep_out) = (None, None);
            for ep in alt
                .endpoints()
                .filter(|ep| ep.transfer_type() == TransferType::Bulk)
            {
                match ep.direction() {
                    Direction::In => {
                        ep_in.get_or_insert(ep.address());
                    }
                    Direction::Out => {
                        ep_out.get_or_insert(ep.address());
                    }
                }
            }
            if let (Some(i), Some(o)) = (ep_in, ep_out) {
                found = Some((alt.interface_number(), i, o));
                break;
            }
        }
        let (number, ep_in, ep_out) = found.ok_or_else(|| {
            MtpError::Unsupported(
                "file transfers (it has no MTP interface with bulk endpoints)".into(),
            )
        })?;
        let interface = device.claim_interface(number).wait().map_err(open_error)?;
        let bulk_in = interface.endpoint::<Bulk, In>(ep_in).map_err(open_error)?;
        let bulk_out = interface
            .endpoint::<Bulk, Out>(ep_out)
            .map_err(open_error)?;
        Ok(Self {
            interface,
            bulk_in,
            bulk_out,
        })
    }

    fn class_out(&self, request: u8, data: &[u8]) -> Result<(), MtpError> {
        let setup = ControlOut {
            control_type: ControlType::Class,
            recipient: Recipient::Interface,
            request,
            value: 0,
            index: u16::from(self.interface.interface_number()),
            data,
        };
        self.interface
            .control_out(setup, CONTROL_TIMEOUT)
            .wait()
            .map_err(transfer_error)
    }

    /// Poll Get Device Status until the device is no longer busy, then
    /// clear the halts it reports.
    fn wait_idle(&mut self) -> Result<(), MtpError> {
        let deadline = Instant::now() + IDLE_TIMEOUT;
        loop {
            let setup = ControlIn {
                control_type: ControlType::Class,
                recipient: Recipient::Interface,
                request: class_request::GET_DEVICE_STATUS,
                value: 0,
                index: u16::from(self.interface.interface_number()),
                length: 20,
            };
            let status = self
                .interface
                .control_in(setup, CONTROL_TIMEOUT)
                .wait()
                .map_err(transfer_error)?;
            let (code, stalled) = protocol::decode_device_status(&status)?;
            if code != rc::DEVICE_BUSY {
                if !stalled.is_empty() || code != rc::OK {
                    let _ = self.bulk_in.clear_halt().wait();
                    let _ = self.bulk_out.clear_halt().wait();
                }
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(MtpError::Busy);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

impl BulkPipe for UsbPipe {
    fn max_packet_size(&self) -> usize {
        self.bulk_out.max_packet_size()
    }

    fn write(&mut self, data: &[u8], timeout: Duration) -> Result<(), MtpError> {
        // An empty buffer is a zero-length packet.
        let buffer = if data.is_empty() {
            Buffer::new(0)
        } else {
            Buffer::from(data.to_vec())
        };
        let completion = self.bulk_out.transfer_blocking(buffer, timeout);
        completion.status.map_err(|e| {
            if e == TransferError::Stall {
                let _ = self.bulk_out.clear_halt().wait();
            }
            transfer_error(e)
        })
    }

    fn read(&mut self, max_len: usize, timeout: Duration) -> Result<Vec<u8>, MtpError> {
        // nusb wants IN lengths in whole packets of the IN endpoint.
        let mps = self.bulk_in.max_packet_size().max(1);
        let len = max_len.max(1).div_ceil(mps) * mps;
        let completion = self.bulk_in.transfer_blocking(Buffer::new(len), timeout);
        match completion.status {
            Ok(()) => {
                let mut data = completion.buffer.into_vec();
                data.truncate(completion.actual_len);
                Ok(data)
            }
            Err(e) => {
                if e == TransferError::Stall {
                    let _ = self.bulk_in.clear_halt().wait();
                }
                Err(transfer_error(e))
            }
        }
    }

    fn cancel(&mut self, transaction_id: u32) -> Result<(), MtpError> {
        self.class_out(
            class_request::CANCEL,
            &protocol::encode_cancel_request(transaction_id),
        )?;
        self.wait_idle()
    }

    fn reset(&mut self) -> Result<(), MtpError> {
        self.class_out(class_request::DEVICE_RESET, &[])?;
        self.wait_idle()?;
        let _ = self.bulk_in.clear_halt().wait();
        let _ = self.bulk_out.clear_halt().wait();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_ptp_or_mtp_interfaces_count() {
        assert!(is_mtp_interface(0x06, 0x01, 0x01, None), "PTP Still Image");
        assert!(
            is_mtp_interface(0xFF, 0xFF, 0x00, Some("MTP")),
            "Android-style"
        );
        assert!(is_mtp_interface(
            0xFF,
            0xFF,
            0x00,
            Some("Kindle mtp interface")
        ));
        assert!(!is_mtp_interface(0x08, 0x06, 0x50, None), "mass storage");
        assert!(
            !is_mtp_interface(0xFF, 0x42, 0x01, Some("ADB Interface")),
            "adb"
        );
        assert!(
            !is_mtp_interface(0xFF, 0xFF, 0x00, None),
            "an unnamed vendor interface"
        );
    }

    #[test]
    fn keys_are_port_paths() {
        assert_eq!(location_key("20", &[1, 2], 7), "usb-20-1.2");
        assert_eq!(location_key("1", &[], 7), "usb-1-a7");
        assert_eq!(location_key("", &[3], 1), "usb-0-3");
        assert_eq!(location_key("0x14/x", &[3], 1), "usb-0x14x-3");
    }
}
