//! Reject ambiguous Android USB targets before mode transitions or opening a transport.

use nusb::MaybeFuture;

mod target;

pub use target::{
    TargetIdentity, clear_target, current_target, ensure_unambiguous_target,
    ensure_untargeted_single, resolve, set_target, target_visible,
};

fn is_android_interface(class: u8, subclass: u8, protocol: u8) -> bool {
    (class == 0xff && subclass == 0x42 && matches!(protocol, 1 | 3))
        || (class == 0xdc && subclass == 2 && protocol == 1)
}

/// Why automatic USB target selection cannot proceed.
#[derive(Debug, thiserror::Error)]
pub enum UsbSelectionError {
    #[error("Multiple Android/EDL USB devices found. Disconnect other devices and try again.")]
    MultipleDevices,
    #[error("USB device enumeration failed: {0}")]
    Enumeration(String),
    #[error(
        "Two attached devices report the chosen device's serial number. Disconnect one and try again."
    )]
    AmbiguousTarget,
    #[error("Cannot prove which attached device is the chosen one: {0}")]
    UnprovenSuccessor(String),
    #[error("Wrong device: expected serial {expected}, found {found}")]
    TargetMismatch { expected: String, found: String },
    #[error("The EDL COM port of the chosen device cannot be found.")]
    UnmappedEdlPort,
    #[error("Device selection state is unusable after an internal error.")]
    StatePoisoned,
}

/// Which protocol a listed USB device speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceMode {
    Adb,
    Fastboot,
    Edl,
    /// Android-related but neither ADB, Fastboot nor EDL (e.g. a Qualcomm diag port).
    Other,
}

/// Physical USB port of a device: bus id plus hub port chain.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct UsbLocation {
    pub bus_id: String,
    pub port_chain: Vec<u8>,
}

/// One Android/EDL USB device as seen by enumeration, without opening a transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbCandidate {
    pub mode: DeviceMode,
    /// Trimmed USB serial; `None` when absent, empty, unreadable, or in EDL.
    pub serial: Option<String>,
    pub location: UsbLocation,
    pub vendor_id: u16,
    pub product_id: u16,
    pub product: Option<String>,
    /// Windows device instance id; `None` on other platforms.
    pub instance_id: Option<String>,
}

fn check_candidate_count(count: usize) -> Result<(), UsbSelectionError> {
    if count > 1 {
        Err(UsbSelectionError::MultipleDevices)
    } else {
        Ok(())
    }
}

/// Counts physical device entries once even if they expose several Android
/// interfaces. Zero devices is allowed while a mode transition is in progress.
/// Transport-specific discovery still checks its own complete candidate set.
/// Callers outside this module go through `ensure_unambiguous_target` or
/// `ensure_untargeted_single`, which also read the session target.
fn ensure_single_usb_target() -> Result<(), UsbSelectionError> {
    // The guard only needs the count, so it skips the string descriptor reads.
    let candidates = collect_candidates(false)?;
    check_candidate_count(candidates.len())
}

/// Lists every Android/EDL USB device with its mode, serial and location.
/// Nothing is claimed; sorted by bus id, then port chain.
pub fn list_candidates() -> Result<Vec<UsbCandidate>, UsbSelectionError> {
    collect_candidates(true)
}

/// Pure mode classification from IDs and `(class, subclass, protocol)` triples.
/// `None` means the device is not an Android/EDL candidate.
fn classify(vendor_id: u16, product_id: u16, interfaces: &[(u8, u8, u8)]) -> Option<DeviceMode> {
    if vendor_id == 0x05c6 {
        match product_id {
            0x9008 => return Some(DeviceMode::Edl),
            0x900e => return Some(DeviceMode::Other),
            _ => {}
        }
    }
    let has = |protocol: u8| interfaces.contains(&(0xff, 0x42, protocol));
    if has(3) {
        Some(DeviceMode::Fastboot)
    } else if has(1) {
        Some(DeviceMode::Adb)
    } else if interfaces
        .iter()
        .any(|&(class, subclass, protocol)| is_android_interface(class, subclass, protocol))
    {
        Some(DeviceMode::Other)
    } else {
        None
    }
}

fn collect_candidates(read_strings: bool) -> Result<Vec<UsbCandidate>, UsbSelectionError> {
    let devices = nusb::list_devices()
        .wait()
        .map_err(|error| UsbSelectionError::Enumeration(error.to_string()))?;
    let mut candidates: Vec<UsbCandidate> = devices
        .filter_map(|device| candidate_from(&device, read_strings))
        .collect();
    candidates.sort_by(|a, b| a.location.cmp(&b.location));
    Ok(candidates)
}

fn candidate_from(device: &nusb::DeviceInfo, read_strings: bool) -> Option<UsbCandidate> {
    let mut triples = vec![(device.class(), device.subclass(), device.protocol())];
    triples.extend(device.interfaces().map(|interface| {
        (
            interface.class(),
            interface.subclass(),
            interface.protocol(),
        )
    }));
    let mode = match classify(device.vendor_id(), device.product_id(), &triples) {
        Some(mode) => mode,
        None => {
            // Windows only exposes interface summaries for composite usbccgp bindings.
            // Whole-device WinUSB bindings require reading configuration descriptors.
            let handle = device.open().wait().ok()?;
            let descriptor_triples: Vec<(u8, u8, u8)> = handle
                .configurations()
                .flat_map(|config| {
                    config
                        .interfaces()
                        .flat_map(|interface| {
                            interface
                                .alt_settings()
                                .map(|alt| (alt.class(), alt.subclass(), alt.protocol()))
                                .collect::<Vec<_>>()
                        })
                        .collect::<Vec<_>>()
                })
                .collect();
            classify(device.vendor_id(), device.product_id(), &descriptor_triples)?
        }
    };

    let serial = if read_strings && mode != DeviceMode::Edl {
        read_serial(device)
    } else {
        None
    };
    // EDL is skipped like the serial: a cache miss sends a string request that could reach a
    // 9008 device in the middle of a Sahara/Firehose session.
    let product = if read_strings && mode != DeviceMode::Edl {
        device
            .product_string()
            .map(str::to_owned)
            .or_else(|| device.get_product_string().wait().ok().flatten())
            .map(|text| text.trim().to_owned())
            .filter(|text| !text.is_empty())
    } else {
        None
    };
    let location = UsbLocation {
        bus_id: device.bus_id().to_owned(),
        port_chain: device.port_chain().to_vec(),
    };
    if location.bus_id.is_empty() || location.port_chain.is_empty() {
        tracing::debug!(
            vendor_id = device.vendor_id(),
            product_id = device.product_id(),
            "USB candidate has no bus location"
        );
    }
    Some(UsbCandidate {
        mode,
        serial,
        location,
        vendor_id: device.vendor_id(),
        product_id: device.product_id(),
        product,
        instance_id: instance_id_of(device),
    })
}

fn read_serial(device: &nusb::DeviceInfo) -> Option<String> {
    device
        .serial_number()
        .map(str::to_owned)
        .or_else(|| device.get_serial_number_string().wait().ok().flatten())
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
}

#[cfg(target_os = "windows")]
fn instance_id_of(device: &nusb::DeviceInfo) -> Option<String> {
    Some(device.instance_id().to_string_lossy().into_owned()).filter(|id| !id.is_empty())
}

#[cfg(not(target_os = "windows"))]
fn instance_id_of(_device: &nusb::DeviceInfo) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn android_modes_are_candidates_but_normal_usb_peripherals_are_not() {
        assert!(is_android_interface(0xff, 0x42, 1));
        assert!(is_android_interface(0xff, 0x42, 3));
        assert!(is_android_interface(0xdc, 2, 1));
        assert!(!is_android_interface(3, 1, 1));
        assert!(!is_android_interface(8, 6, 0x50));
        assert!(!is_android_interface(0xff, 0x42, 2));
    }

    #[test]
    fn mixed_modes_and_same_mode_pairs_are_both_ambiguous() {
        assert!(check_candidate_count(0).is_ok());
        assert!(check_candidate_count(1).is_ok());
        assert!(check_candidate_count(2).is_err());
        assert!(check_candidate_count(3).is_err());
    }

    #[test]
    fn classify_maps_ids_and_interfaces_to_modes() {
        assert_eq!(classify(0x05c6, 0x9008, &[]), Some(DeviceMode::Edl));
        assert_eq!(classify(0x05c6, 0x900e, &[]), Some(DeviceMode::Other));
        assert_eq!(classify(0x1234, 0x9008, &[]), None);
        assert_eq!(
            classify(0x18d1, 0x4ee0, &[(0xff, 0x42, 3)]),
            Some(DeviceMode::Fastboot)
        );
        assert_eq!(
            classify(0x18d1, 0x4ee2, &[(0xff, 0x42, 1)]),
            Some(DeviceMode::Adb)
        );
        assert_eq!(
            classify(0x18d1, 0x4ee0, &[(0xff, 0x42, 1), (0xff, 0x42, 3)]),
            Some(DeviceMode::Fastboot)
        );
        assert_eq!(
            classify(0x1234, 0x1, &[(0xdc, 2, 1)]),
            Some(DeviceMode::Other)
        );
        assert_eq!(classify(0x1234, 0x1, &[(3, 1, 1), (0xff, 0x42, 2)]), None);
        assert_eq!(classify(0x1234, 0x1, &[]), None);
    }
}
