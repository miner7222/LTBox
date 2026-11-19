//! EDL (Emergency Download) — Qualcomm 9008 USB device detection and
//! session management (Sahara → Firehose configure → operations).
//!
//! Transport follows the configured Qualcomm driver mode:
//!
//! * userspace driver mode: `QdlBackend::Usb` (`nusb`; WinUSB stub on Windows
//!   via `qcom-usb-userspace-drivers`)
//! * kernel driver mode: `QdlBackend::Serial` (Windows COM / Linux tty exposed
//!   by `qcom-usb-kernel-drivers`)

use std::io::{Cursor, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use thiserror::Error;

mod atomic_dump;
mod firmware_xml;
mod partition_flash;
mod rawprogram;
#[cfg(test)]
mod test_support;
pub use firmware_xml::collect_firmware_xmls_for_flash;
pub use partition_flash::{PartitionFlash, PartitionFlashError, PartitionOperation};

use crate::driver::{QcomDriverMode, qcom_driver_mode};
use crate::selection::{DeviceMode, UsbCandidate, UsbLocation, UsbSelectionError};
use ltbox_core::i18n::tr;

use qdl::types::{
    FirehoseConfiguration, FirehoseResetMode, FirehoseStorageType, QdlBackend, QdlChan, QdlDevice,
    QdlReadWrite,
};

const QUALCOMM_VID: u16 = 0x05C6;
const QUALCOMM_EDL_PID: u16 = 0x9008;

/// Presence marker returned by [`find_edl_device`] / [`wait_for_device`]
/// in userspace mode. qdl's USB backend opens the real `nusb` device later; a
/// string keeps `wait_for_stable_with` generic and testable with `String` ports.
/// With a session target the marker also names the bound location, see
/// [`edl_location_marker`].
const EDL_DEVICE_MARKER: &str = "USB:VID_05C6&PID_9008";

/// Send a Firehose `<erase>` command. Built on `qdl::firehose_write_getack`
/// so the qdl dependency surface stays on upstream-portable primitives.
fn send_firehose_erase(
    dev: &mut QdlDevice<dyn QdlReadWrite>,
    num_sectors: usize,
    lun: u8,
    start_sector: &str,
) -> std::result::Result<(), String> {
    let sector_size = dev.fh_cfg.storage_sector_size;
    // Same shape xmltree emits in qdl's `firehose_xml_setup`. Firehose is
    // lenient about whitespace but strict about attribute names.
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><data><erase SECTOR_SIZE_IN_BYTES="{sector_size}" num_partition_sectors="{num_sectors}" physical_partition_number="{lun}" start_sector="{start_sector}" /></data>"#
    );
    let mut buf = xml.into_bytes();
    qdl::firehose_write_getack(
        dev,
        &mut buf,
        format!("erase sectors {start_sector}..+{num_sectors}"),
    )
    .map_err(|e| format!("{e}"))
}

const EDL_STABILITY_INTERVAL: Duration = Duration::from_secs(1);
const EDL_DISCONNECT_OBSERVE: Duration = Duration::from_secs(5);
const EDL_SESSION_OPEN_TIMEOUT: Duration = Duration::from_secs(45);

/// Latest per-partition firmware write progress for GUI consumers.
///
/// The last value stays until [`clear_flash_progress`] so gaps between
/// partitions do not flicker empty in the UI.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlashProgress {
    pub partition: String,
    pub percent: u8,
    pub completed_bytes: u64,
    pub total_bytes: u64,
    /// Bytes acknowledged across every write in the current operation.
    pub operation_completed_bytes: u64,
    /// Planned transfer bytes, including sector padding. Later generated
    /// overlays register their sizes before they are written.
    pub operation_total_bytes: u64,
}

impl FlashProgress {
    fn register(&mut self, bytes: u64) {
        self.operation_total_bytes = self.operation_total_bytes.saturating_add(bytes);
    }

    fn begin_partition(&mut self, partition: &str, total: u64) {
        self.partition = partition.to_string();
        self.percent = 0;
        self.completed_bytes = 0;
        self.total_bytes = total;
    }

    fn update_partition(&mut self, partition: &str, completed: u64, total: u64) {
        let delta = completed.saturating_sub(self.completed_bytes);
        self.operation_completed_bytes = self.operation_completed_bytes.saturating_add(delta);
        self.partition = partition.to_string();
        self.percent = flash_percent(completed, total);
        self.completed_bytes = completed;
        self.total_bytes = total;
    }
}

fn uploaded_loader_slot() -> &'static Mutex<Option<std::path::PathBuf>> {
    static SLOT: OnceLock<Mutex<Option<std::path::PathBuf>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// Take the loader whose Sahara upload most recently succeeded, clearing it.
///
/// A completed upload is the only proof a file is the right programmer for the
/// device; the GUI records it against the connected model. Take-once, so a
/// later read cannot credit the upload to a different device.
pub fn take_uploaded_loader() -> Option<std::path::PathBuf> {
    match uploaded_loader_slot().lock() {
        Ok(mut guard) => guard.take(),
        Err(poisoned) => poisoned.into_inner().take(),
    }
}

/// Drop any unread upload record. Called when an operation starts so a stale
/// success cannot be credited to the device this operation is about to touch.
pub fn clear_uploaded_loader() {
    let _ = take_uploaded_loader();
}

fn publish_uploaded_loader(loader: &Path) {
    let mut guard = match uploaded_loader_slot().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    *guard = Some(loader.to_path_buf());
}

fn flash_progress_slot() -> &'static Mutex<Option<FlashProgress>> {
    static SLOT: OnceLock<Mutex<Option<FlashProgress>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// Latest process-wide flash progress snapshot, if any.
pub fn flash_progress() -> Option<FlashProgress> {
    match flash_progress_slot().lock() {
        Ok(guard) => guard.clone(),
        // Poisoning should not panic production flash/GUI polling paths.
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

/// Clear the process-wide flash progress snapshot.
pub fn clear_flash_progress() {
    match flash_progress_slot().lock() {
        Ok(mut guard) => *guard = None,
        Err(poisoned) => *poisoned.into_inner() = None,
    }
}

fn publish_flash_progress(partition: &str, percent: u8, completed_bytes: u64, total_bytes: u64) {
    let mut guard = flash_progress_slot()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let snapshot = guard.get_or_insert_with(FlashProgress::default);
    snapshot.update_partition(partition, completed_bytes, total_bytes);
    debug_assert_eq!(snapshot.percent, percent);
}

fn register_flash_bytes(bytes: u64) {
    let mut guard = flash_progress_slot()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let snapshot = guard.get_or_insert_with(FlashProgress::default);
    snapshot.register(bytes);
}

fn begin_partition_progress(partition: &str, total: u64, register: bool) {
    if register {
        register_flash_bytes(total);
    }
    let mut guard = flash_progress_slot()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let snapshot = guard.get_or_insert_with(FlashProgress::default);
    snapshot.begin_partition(partition, total);
}

fn padded_transfer_bytes(num_sectors: usize, sector_size: usize) -> Result<u64> {
    let bytes = num_sectors
        .checked_mul(sector_size)
        .ok_or_else(|| EdlError::Session("Padded image transfer size exceeds usize".to_string()))?;
    u64::try_from(bytes)
        .map_err(|_| EdlError::Session("Padded image transfer size exceeds u64".to_string()))
}

/// Integer percent in `0..=100`, overflow-safe for large byte totals.
fn flash_percent(completed: u64, total: u64) -> u8 {
    if total == 0 {
        return 100;
    }
    let completed = completed.min(total);
    // Widen to u128: `completed * 100` can overflow u64.
    let pct = ((completed as u128) * 100 / (total as u128)) as u64;
    pct.min(100) as u8
}

/// Publish progress only when the integer percentage changes.
fn update_flash_progress(
    last_percent: &mut Option<u8>,
    partition: &str,
    completed: u64,
    total: u64,
) {
    let percent = flash_percent(completed, total);
    if *last_percent != Some(percent) {
        *last_percent = Some(percent);
        publish_flash_progress(partition, percent, completed, total);
    }
}

#[derive(Error, Debug)]
pub enum EdlError {
    #[error("EDL port not found")]
    PortNotFound,
    #[error("Multiple EDL devices found. Disconnect other devices and try again.")]
    MultipleDevices,
    #[error("Timed out after {0:?} waiting for a stable EDL port")]
    PortTimeout(Duration),
    #[error("Cannot prove which EDL device is the chosen one: {0}")]
    TargetNotProven(String),
    #[error(
        "More than one attached device matches the chosen device. Disconnect the others and try again."
    )]
    AmbiguousTarget,
    #[error("Serial error: {0}")]
    Serial(String),
    #[error("EDL session error: {0}")]
    Session(String),
    #[error("Partition not found: {0}")]
    PartitionNotFound(String),
    #[error("Partition {0} resolves to multiple LUNs (ambiguous; cannot target one)")]
    AmbiguousPartitionLun(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

type Result<T> = std::result::Result<T, EdlError>;

/// One EDL probe result. `location` is the bound USB location and is `Some` exactly when a
/// session target is set; the stable-port wait compares the whole value, so with a target a
/// location change counts as a change in both driver modes.
#[derive(Debug, Clone, PartialEq, Eq)]
struct EdlProbe {
    /// Userspace: the presence marker. Kernel: the COM port name.
    port: String,
    location: Option<UsbLocation>,
}

/// Userspace marker for a device at a bound location. `Debug` quoting keeps distinct
/// locations distinct.
fn edl_location_marker(location: &UsbLocation) -> String {
    format!(
        "{EDL_DEVICE_MARKER}@bus {:?} port {:?}",
        location.bus_id, location.port_chain
    )
}

/// The wait/fail decision for one targeted EDL probe, pure over the `resolve` answer.
///
/// [`EdlError::PortNotFound`] is the only "keep waiting" answer of
/// [`wait_for_stable_with`], and only `Ok(None)` (the target is not in EDL yet) maps to
/// it. A refused proof or two matches stop the wait at once.
fn edl_target_outcome(
    resolved: std::result::Result<Option<UsbCandidate>, UsbSelectionError>,
) -> Result<UsbLocation> {
    edl_target_candidate(resolved).map(|candidate| candidate.location)
}

/// [`edl_target_outcome`] keeping the whole candidate, for the kernel-mode port mapping.
fn edl_target_candidate(
    resolved: std::result::Result<Option<UsbCandidate>, UsbSelectionError>,
) -> Result<UsbCandidate> {
    match resolved {
        Ok(Some(candidate)) => Ok(candidate),
        Ok(None) => Err(EdlError::PortNotFound),
        Err(UsbSelectionError::UnprovenSuccessor(reason)) => Err(EdlError::TargetNotProven(reason)),
        Err(UsbSelectionError::AmbiguousTarget) => Err(EdlError::AmbiguousTarget),
        Err(UsbSelectionError::MultipleDevices) => Err(EdlError::MultipleDevices),
        Err(error @ UsbSelectionError::Enumeration(_)) => Err(EdlError::Serial(error.to_string())),
        Err(
            error @ (UsbSelectionError::UnmappedEdlPort
            | UsbSelectionError::StatePoisoned
            | UsbSelectionError::TargetMismatch { .. }),
        ) => Err(EdlError::Session(error.to_string())),
    }
}

/// Kernel mode with a target where ports cannot be mapped to a USB device (non-Windows): the
/// port is accepted only when exactly one 9008 USB device and exactly one 9008 COM port
/// exist in total. Any other count is [`UsbSelectionError::UnmappedEdlPort`], except one USB
/// device whose COM port is not created yet, which keeps waiting.
#[cfg_attr(windows, allow(dead_code))]
fn kernel_target_port(edl_usb_devices: usize, port: Result<String>) -> Result<String> {
    let unmapped = || EdlError::Session(UsbSelectionError::UnmappedEdlPort.to_string());
    if edl_usb_devices != 1 {
        return Err(unmapped());
    }
    match port {
        Ok(port) => Ok(port),
        Err(EdlError::PortNotFound) => Err(EdlError::PortNotFound),
        Err(EdlError::MultipleDevices) => Err(unmapped()),
        Err(error) => Err(error),
    }
}

/// The COM port whose device instance id is `instance_id` (the bound USB device, from
/// `UsbCandidate::instance_id`). `ports` pairs each port with an instance id of its devnode or
/// of that devnode's parent. ASCII case-insensitive; an empty id never matches. The same port
/// listed under both of its ids is one match. No match, or two distinct ports, is
/// [`UsbSelectionError::UnmappedEdlPort`]; there is no "first match".
#[cfg_attr(not(windows), allow(dead_code))]
fn select_port_for_instance(ports: &[(String, String)], instance_id: &str) -> Result<String> {
    let unmapped = || EdlError::Session(UsbSelectionError::UnmappedEdlPort.to_string());
    if instance_id.is_empty() {
        return Err(unmapped());
    }
    let mut matched: Vec<&str> = Vec::new();
    for (port, id) in ports {
        if !id.is_empty()
            && id.eq_ignore_ascii_case(instance_id)
            && !matched.iter().any(|seen| seen.eq_ignore_ascii_case(port))
        {
            matched.push(port);
        }
    }
    match matched.as_slice() {
        [port] => Ok((*port).to_owned()),
        _ => Err(unmapped()),
    }
}

/// Windows kernel mode with a target: the COM port of the bound USB device, found by device
/// instance id among every 9008 COM port. One USB device whose COM port does not exist yet
/// (`ports` empty) keeps waiting, as before; every other failure to map is a hard error.
#[cfg_attr(not(windows), allow(dead_code))]
fn kernel_target_port_mapped(
    edl_usb_devices: usize,
    instance_id: Option<&str>,
    ports: &[(String, String)],
) -> Result<String> {
    if edl_usb_devices == 1 && ports.is_empty() {
        return Err(EdlError::PortNotFound);
    }
    select_port_for_instance(ports, instance_id.unwrap_or_default())
}

/// The kernel-mode COM port of the resolved target.
#[cfg(windows)]
fn kernel_port_of_target(candidate: &UsbCandidate) -> Result<String> {
    kernel_target_port_mapped(
        count_edl_usb_devices()?,
        candidate.instance_id.as_deref(),
        &find_qcom_edl_ports_windows()?,
    )
}

/// The kernel-mode COM port of the resolved target. Ports cannot be mapped here, so only the
/// one-device-one-port case is accepted (see [`kernel_target_port`]).
#[cfg(not(windows))]
fn kernel_port_of_target(_candidate: &UsbCandidate) -> Result<String> {
    kernel_target_port(count_edl_usb_devices()?, find_edl_port_untargeted())
}

/// The transport may be set up only on the device the wait settled on. A device that is
/// gone, moved, or renamed since then is an error; a refused proof keeps its own error.
fn ensure_probe_unchanged(waited: &EdlProbe, now: Result<EdlProbe>) -> Result<()> {
    match now {
        Ok(now) if now == *waited => Ok(()),
        Ok(_) | Err(EdlError::PortNotFound) => Err(EdlError::Session(
            "EDL device changed before transport setup; retry the operation".into(),
        )),
        Err(error) => Err(error),
    }
}

/// 9008 devices on the USB bus, counted like `qdl::usb::setup_usb_device` sees them.
fn count_edl_usb_devices() -> Result<usize> {
    use nusb::MaybeFuture;
    let devices = nusb::list_devices()
        .wait()
        .map_err(|e| EdlError::Serial(format!("USB device enumeration failed: {e}")))?;
    Ok(devices
        .filter(|d| d.vendor_id() == QUALCOMM_VID && d.product_id() == QUALCOMM_EDL_PID)
        .count())
}

fn resolve_edl_target() -> Result<UsbLocation> {
    edl_target_outcome(crate::selection::resolve(DeviceMode::Edl))
}

fn resolve_edl_candidate() -> Result<UsbCandidate> {
    edl_target_candidate(crate::selection::resolve(DeviceMode::Edl))
}

/// One probe in `mode`. Without a target this is today's count rule; with one, every probe
/// goes through `selection::resolve(DeviceMode::Edl)`.
fn probe_edl(mode: QcomDriverMode, targeted: bool) -> Result<EdlProbe> {
    match (mode, targeted) {
        (QcomDriverMode::Userspace, false) => Ok(EdlProbe {
            port: find_edl_device_untargeted()?,
            location: None,
        }),
        (QcomDriverMode::Kernel, false) => Ok(EdlProbe {
            port: find_edl_port_untargeted()?,
            location: None,
        }),
        (QcomDriverMode::Userspace, true) => {
            let location = resolve_edl_target()?;
            Ok(EdlProbe {
                port: edl_location_marker(&location),
                location: Some(location),
            })
        }
        (QcomDriverMode::Kernel, true) => {
            let candidate = resolve_edl_candidate()?;
            let port = kernel_port_of_target(&candidate)?;
            Ok(EdlProbe {
                port,
                location: Some(candidate.location),
            })
        }
    }
}

/// The open-time guard for the rule `targeted` was read under. Untargeted, the strict count
/// rule is checked together with "still no target" under one lock, so a target set mid-open
/// cannot let the untargeted path skip it. Targeted, a target cleared mid-open is an error.
fn edl_open_guard(targeted: bool) -> Result<()> {
    if targeted {
        if !has_target() {
            return Err(EdlError::Session(
                "the chosen device was cleared during the EDL open; retry the operation".into(),
            ));
        }
        crate::selection::ensure_unambiguous_target()
    } else {
        crate::selection::ensure_untargeted_single()
    }
    .map_err(|e| EdlError::Session(e.to_string()))
}

fn has_target() -> bool {
    crate::selection::current_target().is_some()
}

/// Scan for the Qualcomm 9008 EDL endpoint via `nusb`. Returns
/// [`EDL_DEVICE_MARKER`] for exactly one match, [`EdlError::PortNotFound`] for
/// none, [`EdlError::MultipleDevices`] for several, and [`EdlError::Serial`]
/// when enumeration fails.
///
/// With a session target only the device `selection::resolve` proves is the target counts;
/// the marker then names its location. A refused proof is [`EdlError::TargetNotProven`].
pub fn find_edl_device() -> Result<String> {
    probe_edl(QcomDriverMode::Userspace, has_target()).map(|probe| probe.port)
}

/// Uses the same enumeration as `qdl::usb::setup_usb_device`, so a device the
/// probe sees is one the transport can open.
fn find_edl_device_untargeted() -> Result<String> {
    match count_edl_usb_devices()? {
        0 => Err(EdlError::PortNotFound),
        1 => Ok(EDL_DEVICE_MARKER.to_string()),
        _ => Err(EdlError::MultipleDevices),
    }
}

/// Scan for the Qualcomm 9008 serial port exposed by the kernel-driver mode.
/// With a session target see [`kernel_port_of_target`].
pub fn find_edl_port() -> Result<String> {
    probe_edl(QcomDriverMode::Kernel, has_target()).map(|probe| probe.port)
}

fn find_edl_port_untargeted() -> Result<String> {
    let ports = serialport::available_ports().map_err(|e| EdlError::Serial(e.to_string()))?;
    let mut candidates: Vec<String> = ports
        .into_iter()
        .filter_map(|port| match port.port_type {
            serialport::SerialPortType::UsbPort(usb)
                if usb.vid == QUALCOMM_VID && usb.pid == QUALCOMM_EDL_PID =>
            {
                Some(port.port_name)
            }
            _ => None,
        })
        .collect();
    // Merge even when serialport found a target: another kernel-driver port
    // can have Unknown metadata and only appear through SetupAPI.
    #[cfg(windows)]
    candidates.extend(
        find_qcom_edl_ports_windows()?
            .into_iter()
            .map(|(port, _)| port),
    );
    select_edl_port(&mut candidates)
}

fn select_edl_port(candidates: &mut Vec<String>) -> Result<String> {
    #[cfg(windows)]
    candidates
        .iter_mut()
        .for_each(|port| port.make_ascii_uppercase());
    candidates.sort();
    candidates.dedup();
    match candidates.as_slice() {
        [] => Err(EdlError::PortNotFound),
        [port] => Ok(port.clone()),
        _ => Err(EdlError::MultipleDevices),
    }
}

/// Resolve EDL 9008 COM ports by hardware ID via SetupAPI, because
/// `serialport`'s USB classification fails on the WDF kernel driver's
/// metadata. Reads `PortName` of present Ports-class devices matching
/// `VID_05C6&PID_9008`; empty when the device is absent or has no COM port.
///
/// Each port comes with the Windows device instance id of its devnode, plus a second entry
/// with the instance id of the devnode's parent when there is one, so a port maps to the USB
/// device whether the Ports devnode is the USB devnode itself or a child of it
/// ([`select_port_for_instance`]). A failed id lookup yields an empty id, which never
/// matches, so untargeted callers still get the port.
#[cfg(windows)]
fn find_qcom_edl_ports_windows() -> Result<Vec<(String, String)>> {
    use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
        CM_Get_Device_IDW, CM_Get_Parent, CR_SUCCESS, DICS_FLAG_GLOBAL, DIGCF_PRESENT, DIREG_DEV,
        SP_DEVINFO_DATA, SPDRP_HARDWAREID, SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo,
        SetupDiGetClassDevsW, SetupDiGetDeviceInstanceIdW, SetupDiGetDeviceRegistryPropertyW,
        SetupDiOpenDevRegKey,
    };
    use windows_sys::Win32::Foundation::{ERROR_NO_MORE_ITEMS, GetLastError, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Registry::{KEY_READ, RegCloseKey, RegQueryValueExW};
    use windows_sys::core::GUID;

    // GUID_DEVCLASS_PORTS {4d36e978-e325-11ce-bfc1-08002be10318}
    const GUID_DEVCLASS_PORTS: GUID = GUID {
        data1: 0x4d36_e978,
        data2: 0xe325,
        data3: 0x11ce,
        data4: [0xbf, 0xc1, 0x08, 0x00, 0x2b, 0xe1, 0x03, 0x18],
    };
    const HWID_NEEDLE: &str = "VID_05C6&PID_9008";

    /// A NUL-terminated UTF-16 id from `buf`; empty when there is no terminator.
    fn id_from_buf(buf: &[u16]) -> String {
        buf.iter()
            .position(|&c| c == 0)
            .map(|end| String::from_utf16_lossy(&buf[..end]))
            .unwrap_or_default()
    }

    // SAFETY: textbook SetupAPI enumeration. The device-info-set handle is
    // always destroyed before returning; every buffer is fixed-size and the
    // returned length is clamped to it; only device registry values are read
    // (no writes, no device changes).
    unsafe {
        let hdev = SetupDiGetClassDevsW(
            &GUID_DEVCLASS_PORTS,
            std::ptr::null(),
            std::ptr::null_mut(),
            DIGCF_PRESENT,
        );
        if hdev == INVALID_HANDLE_VALUE as isize {
            return Err(EdlError::Serial(
                std::io::Error::last_os_error().to_string(),
            ));
        }

        let mut found = Vec::new();
        let mut index = 0u32;
        loop {
            let mut info: SP_DEVINFO_DATA = std::mem::zeroed();
            info.cbSize = std::mem::size_of::<SP_DEVINFO_DATA>() as u32;
            if SetupDiEnumDeviceInfo(hdev, index, &mut info) == 0 {
                let error = GetLastError();
                if error != ERROR_NO_MORE_ITEMS {
                    SetupDiDestroyDeviceInfoList(hdev);
                    return Err(EdlError::Serial(format!(
                        "EDL port enumeration failed: {error}"
                    )));
                }
                break;
            }
            index += 1;

            // Hardware IDs: REG_MULTI_SZ of UTF-16. Zero-init so the lossy
            // decode of the unused tail is just NULs; overflow → skip.
            let mut hwid_buf = [0u16; 1024];
            let mut hwid_len = 0u32;
            let got = SetupDiGetDeviceRegistryPropertyW(
                hdev,
                &info,
                SPDRP_HARDWAREID,
                std::ptr::null_mut(),
                hwid_buf.as_mut_ptr() as *mut u8,
                std::mem::size_of_val(&hwid_buf) as u32,
                &mut hwid_len,
            );
            if got == 0 {
                continue;
            }
            let hwids = String::from_utf16_lossy(&hwid_buf).to_ascii_uppercase();
            if !hwids.contains(HWID_NEEDLE) {
                continue;
            }

            // Instance ids (buffer sizes are in WCHARs). No early return from here to the
            // end of the iteration, so `hdev` is still destroyed on every path.
            let mut id_buf = [0u16; 512];
            let mut required = 0u32;
            let own_id = if SetupDiGetDeviceInstanceIdW(
                hdev,
                &info,
                id_buf.as_mut_ptr(),
                id_buf.len() as u32,
                &mut required,
            ) != 0
            {
                id_from_buf(&id_buf)
            } else {
                String::new()
            };
            let mut parent = 0u32;
            let parent_id = if CM_Get_Parent(&mut parent, info.DevInst, 0) == CR_SUCCESS {
                let mut parent_buf = [0u16; 512];
                if CM_Get_Device_IDW(parent, parent_buf.as_mut_ptr(), parent_buf.len() as u32, 0)
                    == CR_SUCCESS
                {
                    id_from_buf(&parent_buf)
                } else {
                    String::new()
                }
            } else {
                String::new()
            };

            let hkey = SetupDiOpenDevRegKey(hdev, &info, DICS_FLAG_GLOBAL, 0, DIREG_DEV, KEY_READ);
            if hkey == INVALID_HANDLE_VALUE {
                continue;
            }
            // Read PortName (REG_SZ, UTF-16) → e.g. "COM3".
            let value: Vec<u16> = "PortName"
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let mut port_buf = [0u16; 64];
            let mut port_len = std::mem::size_of_val(&port_buf) as u32;
            let rc = RegQueryValueExW(
                hkey,
                value.as_ptr(),
                std::ptr::null(),
                std::ptr::null_mut(),
                port_buf.as_mut_ptr() as *mut u8,
                &mut port_len,
            );
            RegCloseKey(hkey);
            if rc == 0 {
                let count = (port_len as usize / 2).min(port_buf.len());
                let end = port_buf[..count]
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(count);
                let port = String::from_utf16_lossy(&port_buf[..end]);
                if !port.is_empty() {
                    if !parent_id.is_empty() {
                        found.push((port.clone(), parent_id));
                    }
                    found.push((port, own_id));
                }
            }
        }
        SetupDiDestroyDeviceInfoList(hdev);
        Ok(found)
    }
}

pub fn check_device() -> bool {
    probe_edl(qcom_driver_mode(), has_target()).is_ok()
}

fn serial_open_error_hint(port: &str, err: &anyhow::Error) -> String {
    let raw = err.to_string();
    let lower = raw.to_ascii_lowercase();
    let base = format!("Transport setup failed: {raw}");

    #[cfg(target_os = "linux")]
    {
        let busy = lower.contains("resource busy")
            || lower.contains("device or resource busy")
            || lower.contains("os error 16");
        let denied = lower.contains("permission denied") || lower.contains("os error 13");
        if busy {
            return format!(
                "{base}\nhint: another process is holding {port}. Re-plug the device after installing the Qualcomm kernel driver, or stop ModemManager temporarily."
            );
        }
        if denied {
            return format!(
                "{base}\nhint: the desktop user cannot open {port}. Check udev/group permissions or run the kernel-driver install again."
            );
        }
    }
    let _ = (&lower, port);
    base
}

/// Wait for a stable EDL port after a reset. Two phases:
/// 1. If a port is visible, observe for `EDL_DISCONNECT_OBSERVE`; treat a
///    name change or disconnect as stale and fall through.
/// 2. Otherwise, return when the same port is seen twice in a row
///    (`EDL_STABILITY_INTERVAL` apart).
///
/// Returns [`EdlError::PortTimeout`] once `deadline_exceeded()` trips.
/// Only [`EdlError::PortNotFound`] keeps waiting; every other error returns at once.
fn wait_for_stable_with<P, F, S, D>(
    mut find_port: F,
    mut sleep: S,
    mut deadline_exceeded: D,
    timeout: Duration,
) -> Result<P>
where
    P: PartialEq,
    F: FnMut() -> Result<P>,
    S: FnMut(Duration),
    D: FnMut() -> bool,
{
    // Phase 1: watch for disconnect OR name change; the OS can keep the
    // old COM handle alive briefly while the new one enumerates.
    let initial = match find_port() {
        Ok(port) => Some(port),
        Err(EdlError::PortNotFound) => None,
        Err(error) => return Err(error),
    };
    if let Some(initial) = initial {
        let mut observed = Duration::ZERO;
        let mut saw_disconnect = false;
        while observed < EDL_DISCONNECT_OBSERVE && !deadline_exceeded() {
            sleep(EDL_STABILITY_INTERVAL);
            observed += EDL_STABILITY_INTERVAL;
            match find_port() {
                Ok(current) if current == initial => {}
                // Name changed → old handle stale. Phase 2 tolerates the move.
                Ok(_) => {
                    saw_disconnect = true;
                    break;
                }
                Err(EdlError::PortNotFound) => {
                    saw_disconnect = true;
                    break;
                }
                Err(e) => return Err(e),
            }
        }
        if !saw_disconnect {
            return Ok(initial);
        }
    }

    // Phase 2: wait for the post-reset port to appear and stabilize.
    let mut last: Option<P> = None;
    while !deadline_exceeded() {
        sleep(EDL_STABILITY_INTERVAL);
        match find_port() {
            Ok(port) => {
                if last.as_ref() == Some(&port) {
                    return Ok(port);
                }
                last = Some(port);
            }
            Err(EdlError::PortNotFound) => last = None,
            Err(e) => return Err(e),
        }
    }

    Err(EdlError::PortTimeout(timeout))
}

/// [`wait_for_stable_with`] over plain port names, for the port-wait tests.
#[cfg(test)]
fn wait_for_stable_port_with<F, S, D>(
    find_port: F,
    sleep: S,
    deadline_exceeded: D,
    timeout: Duration,
) -> Result<String>
where
    F: FnMut() -> Result<String>,
    S: FnMut(Duration),
    D: FnMut() -> bool,
{
    wait_for_stable_with(find_port, sleep, deadline_exceeded, timeout)
}

/// `targeted` is read once by the caller, so one wait never mixes the two rules.
fn wait_for_stable_port(mode: QcomDriverMode, targeted: bool) -> Result<EdlProbe> {
    let deadline = Instant::now() + EDL_SESSION_OPEN_TIMEOUT;
    wait_for_stable_with(
        || probe_edl(mode, targeted),
        std::thread::sleep,
        move || Instant::now() >= deadline,
        EDL_SESSION_OPEN_TIMEOUT,
    )
}

/// Wait for an EDL device; returns the stable-device marker string.
pub fn wait_for_device() -> Result<String> {
    wait_for_stable_port(qcom_driver_mode(), has_target()).map(|probe| probe.port)
}

/// One partition entry surfaced by [`EdlSession::scan_partitions`].
#[derive(Debug, Clone)]
pub struct GptPartitionInfo {
    pub lun: u8,
    pub name: String,
    pub start_sector: u64,
    pub num_sectors: u64,
    pub size_bytes: u64,
}

/// EDL session: owns `QdlDevice`, exposes partition ops.
///
/// A session must leave Firehose through [`Self::reset`],
/// [`Self::reset_tolerant`] or [`Self::reset_to_edl`]. One dropped without
/// any of them (an early `?`, a forgotten error branch, a worker panic) sends
/// `reset_to_edl` itself: left in Firehose, the device ignores the next
/// Sahara Hello until it is power-cycled, and resetting to EDL never boots a
/// possibly half-written slot.
pub struct EdlSession {
    dev: QdlDevice<dyn QdlReadWrite>,
    /// Physical sector count per LUN, memoised for the rawprogram bounds
    /// check. The disk size does not change while a session is open, so a
    /// value stays valid even after the XML rewrites the GPT.
    lun_sectors: std::collections::BTreeMap<u8, Option<u64>>,
    /// An explicit reset has been attempted, successful or not, so `Drop`
    /// must not send a second one.
    exited: bool,
}

fn observe_qdl_log(event: qdl::operation_log::Event<'_>) {
    use ltbox_core::{live_sink, log_format, tr_args};
    match event {
        qdl::operation_log::Event::Diagnostic(message) => {
            live_sink::emit(live_sink::Entry::debug(format!("[EDL/qdl] {message}")));
        }
        qdl::operation_log::Event::Transfer {
            id,
            write,
            target,
            completed,
            total,
        } => {
            let key = if write {
                "live_transfer_write"
            } else {
                "live_transfer_read"
            };
            let line = tr_args!(
                key,
                target = target,
                pct = flash_percent(completed, total),
                done = log_format::bytes(completed),
                total = log_format::bytes(total)
            );
            live_sink::progress(&format!("edl:{id}"), format!("[EDL] {line}"));
        }
    }
}

impl EdlSession {
    /// Open: find port → Sahara upload → Firehose configure.
    pub fn open(loader_path: &Path, log: &mut Vec<String>) -> Result<Self> {
        qdl::operation_log::set_observer(observe_qdl_log);
        // Read once: the target changes only between operations, and one open must not mix
        // the targeted and untargeted rules. Without a target this guard is the strict
        // single-device count; with one, the other devices are ignored.
        let targeted = has_target();
        edl_open_guard(targeted)?;
        let mode = qcom_driver_mode();
        ltbox_core::live!(log, "[EDL] {}", tr("log_edl_scanning"));
        let probe = wait_for_stable_port(mode, targeted)?;
        // In userspace mode `port` is a presence marker, not a COM port.
        let port = probe.port.clone();
        ltbox_core::live!(
            log,
            "[EDL] {}",
            ltbox_core::tr_args!("log_edl_found_on", port = port)
        );

        // Some Lenovo packs ship the Sahara manifest only encrypted (`.x`). It
        // is decrypted to the `.xml` beside it so its relative image paths
        // resolve; every loader consumer passes through here. The caller's
        // original pick is what gets remembered, not the decrypted sibling.
        let picked_loader = loader_path;
        let decrypted_holder;
        let loader_path: &Path =
            if ltbox_core::sahara_xml::is_encrypted_manifest_filename(loader_path) {
                let out = loader_path.with_file_name(ltbox_core::sahara_xml::MANIFEST_FILENAME);
                ltbox_core::crypto::decrypt_file(loader_path, &out).map_err(|e| {
                    EdlError::Session(format!(
                        "Decrypt loader manifest {}: {e}",
                        loader_path.display()
                    ))
                })?;
                decrypted_holder = out;
                decrypted_holder.as_path()
            } else {
                loader_path
            };

        // Manifest: slot array indexed by Sahara image-id.
        // Single loader (.melf/.mbn/.elf): one-element slice at slot 0.
        let mut slots: Vec<Option<Vec<u8>>> =
            if ltbox_core::sahara_xml::is_manifest_filename(loader_path) {
                ltbox_core::live!(
                    log,
                    "[EDL] {}",
                    ltbox_core::tr_args!(
                        "log_edl_loading_programmer",
                        path = loader_path.display()
                    )
                );
                let (slots, paths) = ltbox_core::sahara_xml::load_image_slots(loader_path)
                    .map_err(|e| EdlError::Session(format!("Sahara manifest: {e}")))?;
                let total: usize = slots
                    .iter()
                    .filter_map(|s| s.as_ref().map(|b| b.len()))
                    .sum();
                ltbox_core::live_debug!(
                    log,
                    "[EDL] {}",
                    ltbox_core::tr_args!(
                        "log_edl_programmer_size",
                        size = ltbox_core::log_format::bytes(total as u64),
                        count = paths.len()
                    )
                );
                slots
            } else {
                ltbox_core::live!(
                    log,
                    "[EDL] {}",
                    ltbox_core::tr_args!(
                        "log_edl_loading_programmer",
                        path = loader_path.display()
                    )
                );
                let mbn = std::fs::read(loader_path)
                    .map_err(|e| EdlError::Session(format!("Failed to read loader: {e}")))?;
                ltbox_core::live_debug!(
                    log,
                    "[EDL] {}",
                    ltbox_core::tr_args!(
                        "log_edl_programmer_size",
                        size = ltbox_core::log_format::bytes(mbn.len() as u64),
                        count = 1
                    )
                );
                vec![Some(mbn)]
            };

        ltbox_core::live!(log, "[EDL] {}", tr("log_edl_transport_setup"));
        let backend = match mode {
            QcomDriverMode::Userspace => QdlBackend::Usb,
            QcomDriverMode::Kernel => QdlBackend::Serial,
        };
        edl_open_guard(targeted)?;
        if targeted {
            // Resolve again right before the claim: the proof must still name the device
            // (and location) the wait settled on.
            ensure_probe_unchanged(&probe, probe_edl(mode, true))?;
        } else if mode == QcomDriverMode::Kernel && find_edl_port_untargeted()? != port {
            return Err(EdlError::Session(
                "EDL port changed before transport setup; retry the operation".into(),
            ));
        }
        let rw: Box<dyn QdlReadWrite> = match (mode, targeted, &probe.location) {
            (QcomDriverMode::Userspace, true, Some(location)) => Box::new(
                qdl::usb::setup_usb_device_at(&location.bus_id, &location.port_chain)
                    .map_err(|e| EdlError::Session(format!("Transport setup failed: {e}")))?,
            ),
            (QcomDriverMode::Userspace, true, None) => {
                return Err(EdlError::Session(
                    "EDL device has no bound USB location; retry the operation".into(),
                ));
            }
            (QcomDriverMode::Userspace, false, _) => qdl::setup_target_device(backend, None, None)
                .map_err(|e| EdlError::Session(format!("Transport setup failed: {e}")))?,
            (QcomDriverMode::Kernel, _, _) => {
                match qdl::setup_target_device(backend, None, Some(port.clone())) {
                    Ok(rw) => rw,
                    Err(e) => {
                        let msg = serial_open_error_hint(&port, &e);
                        ltbox_core::live!(log, "[EDL] {msg}");
                        return Err(EdlError::Session(msg));
                    }
                }
            }
        };

        let mut dev = QdlDevice {
            rw,
            fh_cfg: FirehoseConfiguration {
                storage_type: FirehoseStorageType::Ufs,
                storage_sector_size: 4096,
                bypass_storage: false,
                backend,
                skip_firehose_log: true,
                verbose_firehose: false,
                ..Default::default()
            },
            reset_on_drop: false,
        };

        ltbox_core::live!(log, "[EDL] {}", tr("log_edl_sahara_uploading"));
        // qdl `sahara_run` indexes `img_arr` by image-id when len>1, else slot 0.
        // First attempt waits for the PBL's HELLO and answers with HELLO_RESP.
        let first_attempt = qdl::sahara::sahara_run(
            &mut dev,
            qdl::sahara::SaharaMode::WaitingForImage,
            None,
            &mut slots,
            vec![],
            false,
        );
        match first_attempt {
            Ok(_) => {}
            Err(e) => {
                // On a timeout only, mirror upstream qdl CLI's
                // `--skip-hello-wait`: send an unsolicited HELLO_RESP to nudge
                // the PBL forward, then retry. The string match backs up the
                // `io::ErrorKind::TimedOut` downcast in case qdl's wrapping changes.
                let timed_out = e
                    .downcast_ref::<std::io::Error>()
                    .map(|io| io.kind() == std::io::ErrorKind::TimedOut)
                    .unwrap_or(false)
                    || {
                        let emsg = e.to_string();
                        emsg.contains("timed out")
                            || emsg.contains("TimedOut")
                            || emsg.contains("timeout")
                    };
                if !timed_out {
                    return Err(EdlError::Session(format!("Sahara failed: {e}")));
                }
                ltbox_core::live!(log, "[EDL] {}", tr("log_edl_sahara_skip_hello_retry"));
                qdl::sahara::sahara_send_hello_rsp(
                    &mut dev,
                    qdl::sahara::SaharaMode::WaitingForImage,
                )
                .map_err(|e| {
                    EdlError::Session(format!("Sahara skip-hello HELLO_RESP send failed: {e}"))
                })?;
                qdl::sahara::sahara_run(
                    &mut dev,
                    qdl::sahara::SaharaMode::WaitingForImage,
                    None,
                    &mut slots,
                    vec![],
                    false,
                )
                .map_err(|e| {
                    EdlError::Session(format!("Sahara failed after skip-hello retry: {e}"))
                })?;
            }
        }
        ltbox_core::live!(log, "[EDL] {}", tr("log_edl_sahara_uploaded"));
        // Published here, not on `Ok(Self)`: a later Firehose failure says
        // nothing about whether the loader was right.
        publish_uploaded_loader(picked_loader);

        // Keep qdl's own reset_on_drop false to dodge its recursive reset;
        // `EdlSession`'s `Drop` owns the missed-exit cleanup instead.
        dev.reset_on_drop = false;

        ltbox_core::live!(log, "[EDL] {}", tr("log_edl_firehose_configuring"));
        qdl::firehose_read_greeting(&mut dev)
            .map_err(|e| EdlError::Session(format!("Firehose read failed: {e}")))?;
        qdl::firehose_configure(&mut dev, false)
            .map_err(|e| EdlError::Session(format!("Firehose configure failed: {e}")))?;
        qdl::firehose_read(&mut dev, qdl::parsers::firehose_parser_configure_response)
            .map_err(|e| EdlError::Session(format!("Firehose config response failed: {e}")))?;
        ltbox_core::live!(log, "[EDL] {}", tr("log_edl_firehose_configured"));

        Ok(Self {
            dev,
            lun_sectors: std::collections::BTreeMap::new(),
            exited: false,
        })
    }

    /// Max GPT metadata span in sectors. A hostile `first_usable_lba` used
    /// as a read count would allocate gigabytes; real GPTs fit in a few dozen.
    const MAX_GPT_METADATA_SECTORS: u64 = 2048;

    /// Slack sectors after a GPT metadata read. A USB Firehose read can garble
    /// its final sector, and a full entry array (TB323FU LUN 4: 128 entries in
    /// LBA 2..=5) ends exactly there; `gptman` ignores the trailing padding.
    const GPT_READ_TAIL_MARGIN: usize = 2;

    /// Clamp a GPT header's `first_usable_lba` to a sane Firehose read
    /// length, rejecting implausible values instead of over-reading.
    fn gpt_read_sectors(first_usable_lba: u64) -> Result<usize> {
        if first_usable_lba == 0 || first_usable_lba > Self::MAX_GPT_METADATA_SECTORS {
            return Err(EdlError::Session(format!(
                "GPT first_usable_lba {first_usable_lba} outside plausible range 1..={}",
                Self::MAX_GPT_METADATA_SECTORS
            )));
        }
        Ok(first_usable_lba as usize)
    }

    /// Read and parse the GPT of a single LUN.
    fn read_gpt_for_lun(&mut self, slot: u8, lun: u8) -> Result<gptman::GPT> {
        let mut buf = Cursor::new(Vec::<u8>::new());
        qdl::firehose_read_storage(&mut self.dev, &mut buf, 1, slot, lun, 1)
            .map_err(|e| EdlError::Session(format!("GPT probe failed: {e}")))?;
        buf.rewind()?;
        let header = gptman::GPTHeader::read_from(&mut buf)
            .map_err(|e| EdlError::Session(format!("GPT header parse failed: {e}")))?;
        let gpt_len = Self::gpt_read_sectors(header.first_usable_lba)?;
        let read_len =
            (gpt_len + Self::GPT_READ_TAIL_MARGIN).min(Self::MAX_GPT_METADATA_SECTORS as usize);
        let mut buf = Cursor::new(Vec::<u8>::new());
        qdl::firehose_read_storage(&mut self.dev, &mut buf, read_len, slot, lun, 0)
            .map_err(|e| EdlError::Session(format!("GPT read failed: {e}")))?;
        buf.set_position(self.dev.fh_config().storage_sector_size as u64);
        gptman::GPT::read_from(&mut buf, self.dev.fh_config().storage_sector_size as u64)
            .map_err(|e| EdlError::Session(format!("GPT parse failed: {e}")))
    }

    /// Read the GPT of every LUN in `lun_range` and flatten the named
    /// partitions, dropping empty-name or zero-size slots. Per-LUN failures
    /// are logged, not fatal: devices often expose only some of LUNs 0..=5.
    pub fn scan_partitions(
        &mut self,
        lun_range: std::ops::RangeInclusive<u8>,
        log: &mut Vec<String>,
    ) -> Result<Vec<GptPartitionInfo>> {
        let sector_size = self.dev.fh_config().storage_sector_size as u64;
        let mut out = Vec::new();
        for lun in lun_range {
            ltbox_core::live_debug!(
                log,
                "[EDL] {}",
                ltbox_core::tr_args!("log_edl_reading_gpt", lun = lun)
            );
            match self.read_gpt_for_lun(0, lun) {
                Ok(gpt) => {
                    for (_idx, part) in gpt.iter() {
                        let name = part.partition_name.as_str().to_string();
                        let Ok(num_sectors) = part.size() else {
                            continue;
                        };
                        if num_sectors == 0 || name.is_empty() {
                            continue;
                        }
                        out.push(GptPartitionInfo {
                            lun,
                            name,
                            start_sector: part.starting_lba,
                            num_sectors,
                            size_bytes: num_sectors.checked_mul(sector_size).ok_or_else(|| {
                                EdlError::Session("partition byte size overflow".into())
                            })?,
                        });
                    }
                }
                Err(e) => {
                    ltbox_core::live!(
                        log,
                        "[EDL] {}",
                        ltbox_core::tr_args!("log_edl_lun_gpt_read_failed", lun = lun, error = e)
                    );
                }
            }
        }
        Ok(out)
    }

    /// Resolve a partition's UFS LUN: the static [`ltbox_core::partition_lun`]
    /// table first (no I/O), else a GPT scan of LUNs `0..=5` for labels not in
    /// the table. Errors when no GPT has the label or it spans several LUNs.
    pub fn lun_for(&mut self, label: &str, log: &mut Vec<String>) -> Result<u8> {
        if let Some(lun) = ltbox_core::partition_lun::lun_for_partition(label) {
            return Ok(lun);
        }
        let parts = self.scan_partitions(0..=5, log)?;
        // Exact label first, then the slot-stripped base; each tier must agree
        // on one LUN. Multi-LUN labels (`xbl` on LUNs 1+2, `last_parti` on every
        // LUN) are absent from the static map for that reason, so error on
        // ambiguity rather than take a first hit. `xbl_a` resolves exactly.
        let exact: Vec<u8> = parts
            .iter()
            .filter(|p| p.name.eq_ignore_ascii_case(label))
            .map(|p| p.lun)
            .collect();
        if let Some(lun) = Self::single_matching_lun(&exact, label)? {
            return Ok(lun);
        }
        let want = ltbox_core::partition_lun::strip_slot_suffix(label).to_ascii_lowercase();
        let stripped: Vec<u8> = parts
            .iter()
            .filter(|p| {
                ltbox_core::partition_lun::strip_slot_suffix(&p.name).to_ascii_lowercase() == want
            })
            .map(|p| p.lun)
            .collect();
        Self::single_matching_lun(&stripped, label)?
            .ok_or_else(|| EdlError::PartitionNotFound(label.to_string()))
    }

    /// Collapse the LUNs of GPT entries that matched a label: `Ok(None)` for no
    /// match, `Ok(Some(lun))` when all agree, `Err(AmbiguousPartitionLun)` when
    /// the label spans several LUNs.
    fn single_matching_lun(luns: &[u8], label: &str) -> Result<Option<u8>> {
        match luns.first() {
            None => Ok(None),
            Some(&first) if luns.iter().all(|&l| l == first) => Ok(Some(first)),
            Some(_) => Err(EdlError::AmbiguousPartitionLun(label.to_string())),
        }
    }

    /// GPT lookup by partition name.
    fn find_partition(&mut self, part_name: &str, slot: u8, lun: u8) -> Result<(u64, u64)> {
        let gpt = self.read_gpt_for_lun(slot, lun)?;

        let part = gpt
            .iter()
            .find(|(_, p)| p.partition_name.as_str() == part_name)
            .ok_or_else(|| EdlError::PartitionNotFound(part_name.to_string()))?
            .1;
        Ok((part.starting_lba, part.ending_lba))
    }

    /// Revalidate all selected geometry before any transfer in this session.
    /// Identical cloned GPTs still require a device-specific identity check.
    pub fn validate_partition_snapshot(&mut self, expected: &[GptPartitionInfo]) -> Result<()> {
        let sector_size = self.dev.fh_config().storage_sector_size as u64;
        let mut seen = std::collections::BTreeSet::new();
        for row in expected {
            if !seen.insert((row.lun, row.name.as_str())) {
                return Err(EdlError::Session(format!(
                    "Duplicate partition selection: {}",
                    row.name
                )));
            }
            let (start, end) = self.find_partition(&row.name, 0, row.lun)?;
            let sectors = Self::partition_span_sectors(&row.name, start, end)? as u64;
            if start != row.start_sector
                || sectors != row.num_sectors
                || sector_size == 0
                || sectors.checked_mul(sector_size) != Some(row.size_bytes)
            {
                return Err(EdlError::Session(format!(
                    "Partition layout changed for {} on LUN {}; scan the device again",
                    row.name, row.lun
                )));
            }
        }
        Ok(())
    }

    /// Dump a partition (GPT-by-name) to a file.
    pub fn dump_partition(
        &mut self,
        part_name: &str,
        output: &Path,
        slot: u8,
        lun: u8,
        log: &mut Vec<String>,
    ) -> Result<()> {
        ltbox_core::live_debug!(
            log,
            "[EDL] {}",
            ltbox_core::tr_args!("log_edl_lookup_partition", part = part_name, lun = lun)
        );
        let (start, end) = self.find_partition(part_name, slot, lun)?;
        // GPT bounds are inclusive: end == start spans one sector.
        // Reject inverted ranges and counts that cannot be represented.
        let span = end
            .checked_sub(start)
            .and_then(|d| d.checked_add(1))
            .ok_or_else(|| {
                EdlError::Session(format!(
                    "Partition {part_name} GPT range invalid: start={start} end={end}"
                ))
            })?;
        let sectors = usize::try_from(span).map_err(|_| {
            EdlError::Session(format!("Partition {part_name} span {span} exceeds usize"))
        })?;
        ltbox_core::live_debug!(
            log,
            "[EDL] {}",
            ltbox_core::tr_args!(
                "log_edl_found_partition",
                part = part_name,
                start = start,
                end = end,
                sectors = sectors
            )
        );

        ltbox_core::live!(
            log,
            "[EDL] {}",
            ltbox_core::tr_args!(
                "log_edl_read_image",
                part = part_name,
                path = output.display(),
                lun = lun
            )
        );
        let expected = padded_transfer_bytes(sectors, self.dev.fh_config().storage_sector_size)?;
        atomic_dump::write_dump(output, expected, |file| {
            qdl::firehose_read_storage(&mut self.dev, file, sectors, slot, lun, start)
                .map_err(|e| EdlError::Session(format!("Partition read failed: {e}")))
        })?;
        ltbox_core::live!(
            log,
            "[EDL] {}",
            ltbox_core::tr_args!("log_edl_dumped", part = part_name)
        );
        Ok(())
    }

    /// Dump with pre-resolved (LUN, start, length); skips the GPT lookup.
    pub fn dump_partition_at(
        &mut self,
        part_name: &str,
        output: &Path,
        lun: u8,
        start_sector: u64,
        num_sectors: usize,
        log: &mut Vec<String>,
    ) -> Result<()> {
        ltbox_core::live!(
            log,
            "[EDL] {}",
            ltbox_core::tr_args!(
                "log_edl_read_image",
                part = part_name,
                path = output.display(),
                lun = lun
            )
        );
        let expected =
            padded_transfer_bytes(num_sectors, self.dev.fh_config().storage_sector_size)?;
        atomic_dump::write_dump(output, expected, |file| {
            qdl::firehose_read_storage(&mut self.dev, file, num_sectors, 0, lun, start_sector)
                .map_err(|e| EdlError::Session(format!("Partition read failed: {e}")))
        })?;
        ltbox_core::live!(
            log,
            "[EDL] {}",
            ltbox_core::tr_args!("log_edl_dumped", part = part_name)
        );
        Ok(())
    }

    /// Flash with pre-resolved (LUN, start). Counterpart to
    /// [`dump_partition_at`]; skips the GPT lookup used by [`flash_partition`].
    pub fn flash_partition_at(
        &mut self,
        part_name: &str,
        image: &Path,
        lun: u8,
        start_sector: &str,
        partition_sectors: u64,
        log: &mut Vec<String>,
    ) -> Result<()> {
        let mut file = std::fs::File::open(image)?;
        let metadata = file.metadata()?;
        let file_len = metadata.len();
        let sector_size = self.dev.fh_config().storage_sector_size as u64;
        if !metadata.is_file() || file_len == 0 || sector_size == 0 {
            return Err(EdlError::Session(
                "Flash requires a nonempty regular image and nonzero sector size".into(),
            ));
        }
        let image_sectors = file_len.div_ceil(sector_size);
        // An oversized image would spill into the next partition and brick
        // the device. `partition_sectors` comes from the caller's prior scan.
        if image_sectors > partition_sectors {
            return Err(EdlError::Session(format!(
                "Flash {part_name}: image is {image_sectors} sectors but the partition spans only {partition_sectors}"
            )));
        }
        let num_sectors = usize::try_from(image_sectors)
            .map_err(|_| EdlError::Session("Image sector count exceeds usize".into()))?;
        ltbox_core::live!(
            log,
            "[EDL] {}",
            ltbox_core::tr_args!(
                "log_edl_flash_image",
                part = part_name,
                path = image.display(),
                size = ltbox_core::log_format::bytes(file_len),
                sectors = num_sectors,
                lun = lun
            )
        );
        let transfer_bytes =
            padded_transfer_bytes(num_sectors, self.dev.fh_config().storage_sector_size)?;
        begin_partition_progress(part_name, transfer_bytes, true);
        let mut last_percent = None;
        qdl::firehose_program_storage_with_progress(
            &mut self.dev,
            &mut file,
            part_name,
            num_sectors,
            0,
            lun,
            start_sector,
            |completed, total| {
                update_flash_progress(&mut last_percent, part_name, completed, total)
            },
        )
        .map_err(|e| EdlError::Session(format!("Partition write failed: {e}")))?;
        ltbox_core::live!(
            log,
            "[EDL] {}",
            ltbox_core::tr_args!("log_edl_flashed", part = part_name)
        );
        Ok(())
    }

    /// Total sector count of a physical LUN: the primary GPT header's
    /// `backup_lba + 1`, since the backup header is the disk's last sector.
    pub fn physical_lun_sector_count(&mut self, lun: u8, log: &mut Vec<String>) -> Result<u64> {
        let mut buf = Cursor::new(Vec::<u8>::new());
        qdl::firehose_read_storage(&mut self.dev, &mut buf, 1, 0, lun, 1)
            .map_err(|e| EdlError::Session(format!("GPT probe failed: {e}")))?;
        buf.rewind()?;
        let header = gptman::GPTHeader::read_from(&mut buf)
            .map_err(|e| EdlError::Session(format!("GPT header parse failed: {e}")))?;
        let total = header.backup_lba.checked_add(1).ok_or_else(|| {
            EdlError::Session(format!(
                "GPT backup_lba {} overflows when computing LUN sector count",
                header.backup_lba
            ))
        })?;
        ltbox_core::live!(
            log,
            "[EDL] {}",
            ltbox_core::tr_args!("log_edl_lun_total_sectors", lun = lun, total = total)
        );
        Ok(total)
    }

    /// Raw whole-LUN dump to `output`, sized by
    /// [`Self::physical_lun_sector_count`]; no GPT decoding.
    pub fn dump_physical_storage(
        &mut self,
        lun: u8,
        output: &Path,
        log: &mut Vec<String>,
    ) -> Result<()> {
        let total = self.physical_lun_sector_count(lun, log)?;
        ltbox_core::live!(
            log,
            "[EDL] {}",
            ltbox_core::tr_args!(
                "log_edl_dump_lun_cmd",
                lun = lun,
                path = output.display(),
                total = total
            )
        );
        let sectors = usize::try_from(total)
            .map_err(|_| EdlError::Session("LUN sector count exceeds usize".into()))?;
        let expected = padded_transfer_bytes(sectors, self.dev.fh_config().storage_sector_size)?;
        atomic_dump::write_dump(output, expected, |file| {
            qdl::firehose_read_storage(&mut self.dev, file, sectors, 0, lun, 0)
                .map_err(|e| EdlError::Session(format!("Physical LUN read failed: {e}")))
        })?;
        ltbox_core::live!(
            log,
            "[EDL] {}",
            ltbox_core::tr_args!("log_edl_dumped_lun", lun = lun)
        );
        Ok(())
    }

    /// Whole-LUN raw flash from sector 0, like qdlrs `OverwriteStorage`.
    /// Refuses an image larger than the LUN's capacity.
    pub fn flash_physical_storage(
        &mut self,
        lun: u8,
        image: &Path,
        log: &mut Vec<String>,
    ) -> Result<()> {
        let mut file = std::fs::File::open(image)?;
        let metadata = file.metadata()?;
        let file_len = metadata.len();
        let sector_size = self.dev.fh_config().storage_sector_size as u64;
        if !metadata.is_file() || file_len == 0 || sector_size == 0 {
            return Err(EdlError::Session(
                "Flash requires a nonempty regular image and nonzero sector size".into(),
            ));
        }
        let image_sectors = file_len.div_ceil(sector_size);
        // An oversized image would spill past the last LBA (some Firehose
        // implementations wrap or corrupt adjacent storage).
        let lun_capacity = self.physical_lun_sector_count(lun, log)?;
        ensure_image_fits_physical_lun(image_sectors, lun_capacity, lun)?;
        let num_sectors = usize::try_from(image_sectors).map_err(|_| {
            EdlError::Session(format!(
                "Flash LUN {lun}: image sector count {image_sectors} exceeds usize"
            ))
        })?;
        ltbox_core::live!(
            log,
            "[EDL] {}",
            ltbox_core::tr_args!(
                "log_edl_flash_lun_cmd",
                lun = lun,
                path = image.display(),
                bytes = file_len,
                sectors = num_sectors
            )
        );
        let transfer_bytes =
            padded_transfer_bytes(num_sectors, self.dev.fh_config().storage_sector_size)?;
        let label = format!("LUN {lun}");
        begin_partition_progress(&label, transfer_bytes, true);
        let mut last_percent = None;
        qdl::firehose_program_storage_with_progress(
            &mut self.dev,
            &mut file,
            &label,
            num_sectors,
            0,
            lun,
            "0",
            |completed, total| update_flash_progress(&mut last_percent, &label, completed, total),
        )
        .map_err(|e| EdlError::Session(format!("Physical LUN write failed: {e}")))?;
        ltbox_core::live!(
            log,
            "[EDL] {}",
            ltbox_core::tr_args!("log_edl_flashed_lun", lun = lun)
        );
        Ok(())
    }

    /// Erase a sector range (`efisp` is zero-filled instead). `start_sector`
    /// is passed to Firehose as a string so negative offsets (e.g. "-1") stay valid.
    pub fn erase_partition_at(
        &mut self,
        part_name: &str,
        lun: u8,
        start_sector: &str,
        num_sectors: usize,
        log: &mut Vec<String>,
    ) -> Result<()> {
        ltbox_core::live!(
            log,
            "[EDL] {}",
            ltbox_core::tr_args!(
                "log_edl_erase_part_cmd",
                part = part_name,
                lun = lun,
                start = start_sector,
                sectors = num_sectors
            )
        );
        if part_name == "efisp" {
            // On GBL devices a Firehose <erase> can read back as zero in-session
            // yet expose the old EFISP image after reboot, so program zeros,
            // streamed to avoid a partition-sized allocation. No readback.
            let sector_size = self.dev.fh_config().storage_sector_size;
            if num_sectors == 0
                || sector_size == 0
                || self.dev.fh_config().send_buffer_size < sector_size
            {
                return Err(EdlError::Session(
                    "Invalid EFISP zero-write geometry".into(),
                ));
            }
            let bytes = padded_transfer_bytes(num_sectors, sector_size)?;
            let mut zeros = std::io::Read::take(std::io::repeat(0), bytes);
            register_flash_bytes(bytes);
            begin_partition_progress(part_name, bytes, false);
            let mut last_percent = None;
            ltbox_core::live!(
                log,
                "[EDL] {}",
                ltbox_core::tr_args!(
                    "log_edl_zero_fill",
                    part = part_name,
                    size = ltbox_core::log_format::bytes(bytes),
                    lun = lun,
                    start = start_sector
                )
            );
            qdl::firehose_program_storage_with_progress(
                &mut self.dev,
                &mut zeros,
                part_name,
                num_sectors,
                0,
                lun,
                start_sector,
                |completed, total| {
                    update_flash_progress(&mut last_percent, part_name, completed, total)
                },
            )
            .map_err(|e| EdlError::Session(format!("Zero-fill {part_name} failed: {e}")))?;
        } else {
            send_firehose_erase(&mut self.dev, num_sectors, lun, start_sector)
                .map_err(|e| EdlError::Session(format!("Erase {part_name} failed: {e}")))?;
        }
        ltbox_core::live!(
            log,
            "[EDL] {}",
            ltbox_core::tr_args!("log_edl_erased_part", part = part_name)
        );
        Ok(())
    }

    /// Sectors spanned by a GPT partition (`end` is the inclusive last LBA).
    /// Errors on an inverted range so brick-critical erase/flash refuse bad
    /// geometry rather than silently touching a wrong, tiny span.
    fn partition_span_sectors(part_name: &str, start: u64, end: u64) -> Result<usize> {
        end.checked_sub(start)
            .and_then(|delta| delta.checked_add(1))
            .and_then(|span| usize::try_from(span).ok())
            .ok_or_else(|| {
                EdlError::Session(format!(
                    "{part_name}: invalid GPT range (start {start} > end {end})"
                ))
            })
    }

    /// Erase every sector of a partition resolved by name from the device GPT.
    pub fn erase_partition_by_name(
        &mut self,
        part_name: &str,
        slot: u8,
        lun: u8,
        log: &mut Vec<String>,
    ) -> Result<()> {
        ltbox_core::live_debug!(
            log,
            "[EDL] {}",
            ltbox_core::tr_args!("log_edl_lookup_partition", part = part_name, lun = lun)
        );
        let (start, end) = self.find_partition(part_name, slot, lun)?;
        let num_sectors = Self::partition_span_sectors(part_name, start, end)?;
        self.erase_partition_at(part_name, lun, &start.to_string(), num_sectors, log)
    }

    /// Flash image to a partition (GPT-by-name).
    pub fn flash_partition(
        &mut self,
        part_name: &str,
        image: &Path,
        slot: u8,
        lun: u8,
        log: &mut Vec<String>,
    ) -> Result<()> {
        ltbox_core::live_debug!(
            log,
            "[EDL] {}",
            ltbox_core::tr_args!("log_edl_lookup_partition", part = part_name, lun = lun)
        );
        let (start, end) = self.find_partition(part_name, slot, lun)?;
        let span = Self::partition_span_sectors(part_name, start, end)?;

        let mut file = std::fs::File::open(image)?;
        let metadata = file.metadata()?;
        let file_len = metadata.len();
        let sector_size = self.dev.fh_config().storage_sector_size as u64;
        if !metadata.is_file() || file_len == 0 || sector_size == 0 {
            return Err(EdlError::Session(
                "Flash requires a nonempty regular image and nonzero sector size".into(),
            ));
        }
        let num_sectors = usize::try_from(file_len.div_ceil(sector_size))
            .map_err(|_| EdlError::Session("Image sector count exceeds usize".into()))?;
        // An oversized image would spill into the next partition and brick
        // the device.
        if num_sectors > span {
            return Err(EdlError::Session(format!(
                "Flash {part_name}: image is {num_sectors} sectors but the partition spans only {span}"
            )));
        }
        ltbox_core::live!(
            log,
            "[EDL] {}",
            ltbox_core::tr_args!(
                "log_edl_flash_image",
                part = part_name,
                path = image.display(),
                size = ltbox_core::log_format::bytes(file_len),
                sectors = num_sectors,
                lun = lun
            )
        );

        let transfer_bytes =
            padded_transfer_bytes(num_sectors, self.dev.fh_config().storage_sector_size)?;
        begin_partition_progress(part_name, transfer_bytes, true);
        let mut last_percent = None;
        qdl::firehose_program_storage_with_progress(
            &mut self.dev,
            &mut file,
            part_name,
            num_sectors,
            slot,
            lun,
            &start.to_string(),
            |completed, total| {
                update_flash_progress(&mut last_percent, part_name, completed, total)
            },
        )
        .map_err(|e| EdlError::Session(format!("Partition write failed: {e}")))?;
        ltbox_core::live!(
            log,
            "[EDL] {}",
            ltbox_core::tr_args!("log_edl_flashed", part = part_name)
        );
        Ok(())
    }

    pub fn reset(&mut self, log: &mut Vec<String>) -> Result<()> {
        self.exited = true;
        ltbox_core::live!(log, "[EDL] {}", tr("log_edl_reset_cmd"));
        qdl::firehose_reset(&mut self.dev, &FirehoseResetMode::Reset, 2)
            .map_err(|e| EdlError::Session(format!("Reset failed: {e}")))?;
        ltbox_core::live!(log, "[EDL] {}", tr("log_edl_reset_initiated"));
        Ok(())
    }

    /// Best-effort system reset for end-of-flow cleanup: some devices reset
    /// while qdl still reports an error. Use only after every write is done.
    pub fn reset_tolerant(&mut self, log: &mut Vec<String>) {
        if let Err(e) = self.reset(log) {
            ltbox_core::live!(
                log,
                "[EDL] {}",
                ltbox_core::tr_args!("log_edl_reboot_handoff_error", error = e)
            );
        }
    }

    /// Bounce back to Sahara (does NOT boot system). Required after a
    /// dump-only session so the next `open()` gets a fresh Hello;
    /// otherwise Sahara times out.
    pub fn reset_to_edl(&mut self, log: &mut Vec<String>) -> Result<()> {
        self.exited = true;
        ltbox_core::live!(log, "[EDL] {}", tr("log_edl_reset_to_edl_cmd"));
        qdl::firehose_reset(&mut self.dev, &FirehoseResetMode::ResetToEdl, 0)
            .map_err(|e| EdlError::Session(format!("reset_to_edl failed: {e}")))?;
        ltbox_core::live!(log, "[EDL] {}", tr("log_edl_reset_to_edl_sent"));
        Ok(())
    }

    /// Mark LUN 1 as the boot drive: every model in
    /// `ltbox_core::model::SUPPORTED_MODELS` places `xbl_a` there.
    /// Call after flashing Lenovo's `_a` firmware so the next reset boots it.
    pub fn set_active_slot_a(&mut self, log: &mut Vec<String>) -> Result<()> {
        const XBL_A_LUN: u8 = 1;
        ltbox_core::live!(
            log,
            "[Flash] {}",
            ltbox_core::tr_args!("live_flash_set_bootable", lun = XBL_A_LUN)
        );
        qdl::firehose_set_bootable(&mut self.dev, XBL_A_LUN)
            .map_err(|e| EdlError::Session(format!("setbootablestoragedrive failed: {e}")))
    }
}

impl Drop for EdlSession {
    fn drop(&mut self) {
        if self.exited {
            return;
        }
        // No caller log survives a drop; the live sink still carries these.
        let mut log = Vec::new();
        ltbox_core::live_debug!(
            log,
            "[EDL] session dropped without an explicit reset; returning the device to EDL"
        );
        if let Err(e) = self.reset_to_edl(&mut log) {
            ltbox_core::live_debug!(log, "[EDL] drop-time reset_to_edl failed: {e}");
        }
    }
}

/// Refuse a whole-LUN flash whose image exceeds the target LUN capacity.
/// An oversized image would program past the end of the disk.
fn ensure_image_fits_physical_lun(
    image_sectors: u64,
    lun_capacity_sectors: u64,
    lun: u8,
) -> Result<()> {
    if image_sectors > lun_capacity_sectors {
        return Err(EdlError::Session(format!(
            "Flash LUN {lun}: image is {image_sectors} sectors but the LUN holds only {lun_capacity_sectors}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    #[test]
    fn flash_percent_clamps_and_avoids_overflow() {
        assert_eq!(flash_percent(0, 0), 100);
        assert_eq!(flash_percent(0, 100), 0);
        assert_eq!(flash_percent(50, 100), 50);
        assert_eq!(flash_percent(100, 100), 100);
        assert_eq!(flash_percent(150, 100), 100);
        // Large even totals that would overflow `completed * 100` in u64.
        let total = 1u64 << 63; // even, so half is exact
        assert_eq!(flash_percent(total / 2, total), 50);
        assert_eq!(flash_percent(total, total), 100);
        assert_eq!(flash_percent(u64::MAX, u64::MAX), 100);
    }

    #[test]
    fn flash_progress_accumulates_partitions_without_losing_current_details() {
        let mut progress = FlashProgress::default();
        progress.register(200);
        progress.register(400);
        progress.begin_partition("boot", 200);
        progress.update_partition("boot", 1, 200);
        assert_eq!(progress.percent, 0);
        progress.update_partition("boot", 200, 200);
        progress.begin_partition("system", 400);
        progress.update_partition("system", 0, 400);
        assert_eq!(
            progress,
            FlashProgress {
                partition: "system".into(),
                percent: 0,
                completed_bytes: 0,
                total_bytes: 400,
                operation_completed_bytes: 200,
                operation_total_bytes: 600,
            }
        );
    }

    #[test]
    fn ensure_image_fits_physical_lun_rejects_oversize() {
        // Exact fit and undersize are accepted.
        assert!(ensure_image_fits_physical_lun(100, 100, 0).is_ok());
        assert!(ensure_image_fits_physical_lun(1, 100, 1).is_ok());
        assert!(ensure_image_fits_physical_lun(0, 0, 0).is_ok());
        // One sector over capacity must refuse before any Firehose write.
        let err = ensure_image_fits_physical_lun(101, 100, 2).expect_err("oversize image");
        let msg = err.to_string();
        assert!(msg.contains("LUN 2"), "{msg}");
        assert!(msg.contains("101"), "{msg}");
        assert!(msg.contains("100"), "{msg}");
    }

    #[test]
    fn partition_span_sectors_counts_inclusive_and_rejects_inverted() {
        // end is the inclusive last LBA, so span = end - start + 1.
        assert_eq!(
            EdlSession::partition_span_sectors("p", 100, 199).unwrap(),
            100
        );
        assert_eq!(EdlSession::partition_span_sectors("p", 5, 5).unwrap(), 1);
        // Start LBAs past u32::MAX must not truncate.
        let start = u64::from(u32::MAX) + 1;
        assert_eq!(
            EdlSession::partition_span_sectors("p", start, start + 99).unwrap(),
            100
        );
        // Inverted range must error, never produce a tiny bogus span.
        assert!(EdlSession::partition_span_sectors("p", 200, 100).is_err());
    }

    #[test]
    fn single_matching_lun_resolves_unique_and_rejects_ambiguous() {
        // No GPT match -> None (caller falls through to the next tier / not-found).
        assert_eq!(EdlSession::single_matching_lun(&[], "boot").unwrap(), None);
        // One entry, or several all on the same LUN -> that LUN.
        assert_eq!(
            EdlSession::single_matching_lun(&[4], "boot").unwrap(),
            Some(4)
        );
        assert_eq!(
            EdlSession::single_matching_lun(&[2, 2, 2], "apdpb").unwrap(),
            Some(2)
        );
        // A label spanning >1 LUN (e.g. `xbl` on LUN 1+2) is ambiguous, never an
        // arbitrary first hit.
        assert!(matches!(
            EdlSession::single_matching_lun(&[1, 2], "xbl"),
            Err(EdlError::AmbiguousPartitionLun(_))
        ));
    }

    #[test]
    fn gpt_read_sectors_bounds_first_usable_lba() {
        // Plausible GPT metadata spans pass through unchanged.
        assert_eq!(EdlSession::gpt_read_sectors(6).unwrap(), 6);
        assert_eq!(EdlSession::gpt_read_sectors(34).unwrap(), 34);
        assert_eq!(
            EdlSession::gpt_read_sectors(EdlSession::MAX_GPT_METADATA_SECTORS).unwrap(),
            EdlSession::MAX_GPT_METADATA_SECTORS as usize
        );
        // Zero and hostile-GPT values must not drive an unbounded read.
        assert!(EdlSession::gpt_read_sectors(0).is_err());
        assert!(EdlSession::gpt_read_sectors(EdlSession::MAX_GPT_METADATA_SECTORS + 1).is_err());
        assert!(EdlSession::gpt_read_sectors(u64::MAX).is_err());
    }

    fn run_with_deadline_never(ports: Vec<Result<String>>) -> Result<String> {
        let mut queue: VecDeque<Result<String>> = ports.into();
        wait_for_stable_port_with(
            || queue.pop_front().unwrap_or(Err(EdlError::PortNotFound)),
            |_| {},
            || false,
            EDL_SESSION_OPEN_TIMEOUT,
        )
    }

    #[test]
    fn stale_port_disconnects_then_new_port_stabilizes() {
        // Phase 1: Ok(COM6) → disconnect. Phase 2: COM7 twice → return.
        let port = run_with_deadline_never(vec![
            Ok("COM6".to_string()),
            Err(EdlError::PortNotFound),
            Ok("COM7".to_string()),
            Ok("COM7".to_string()),
        ])
        .expect("stable port");
        assert_eq!(port, "COM7");
    }

    #[test]
    fn visible_port_without_disconnect_is_trusted() {
        // Port visible for full 5-poll observation window.
        let port = run_with_deadline_never(vec![
            Ok("COM6".to_string()),
            Ok("COM6".to_string()),
            Ok("COM6".to_string()),
            Ok("COM6".to_string()),
            Ok("COM6".to_string()),
            Ok("COM6".to_string()),
        ])
        .expect("stable port");
        assert_eq!(port, "COM6");
    }

    #[test]
    fn phase1_name_change_forces_phase2() {
        // A presence-only Phase 1 would latch the dead COM6 handle across a
        // reset-to-EDL re-enumeration.
        let port = run_with_deadline_never(vec![
            Ok("COM6".to_string()),
            Ok("COM7".to_string()), // phase 1 name change → fall through
            Ok("COM7".to_string()),
            Ok("COM7".to_string()),
        ])
        .expect("stable port");
        assert_eq!(port, "COM7");
    }

    #[test]
    fn timeout_returns_port_timeout_error() {
        let mut polls_remaining: u32 = 3;
        let err = wait_for_stable_port_with(
            || Err(EdlError::PortNotFound),
            |_| {},
            move || {
                if polls_remaining == 0 {
                    true
                } else {
                    polls_remaining -= 1;
                    false
                }
            },
            EDL_SESSION_OPEN_TIMEOUT,
        )
        .expect_err("should time out");
        assert!(matches!(err, EdlError::PortTimeout(_)));
    }
}

#[cfg(test)]
mod selection_tests {
    use super::*;
    #[test]
    fn edl_ports_merge_duplicates_but_reject_distinct_targets() {
        assert!(matches!(
            select_edl_port(&mut vec![]),
            Err(EdlError::PortNotFound)
        ));
        assert_eq!(
            select_edl_port(&mut vec!["COM3".into(), "COM3".into()]).unwrap(),
            "COM3"
        );
        for ports in [["COM3", "COM4"], ["COM4", "COM3"]] {
            assert!(matches!(
                select_edl_port(&mut ports.map(String::from).to_vec()),
                Err(EdlError::MultipleDevices)
            ));
        }
    }
    #[test]
    fn ambiguity_on_initial_stability_probe_is_fatal() {
        let result = wait_for_stable_port_with(
            || Err(EdlError::MultipleDevices),
            |_| panic!("must not wait after ambiguous discovery"),
            || false,
            Duration::from_secs(1),
        );
        assert!(matches!(result, Err(EdlError::MultipleDevices)));
    }
}

#[cfg(test)]
mod target_tests {
    use super::*;
    use std::cell::Cell;
    use std::collections::VecDeque;

    fn loc(bus: &str, chain: &[u8]) -> UsbLocation {
        UsbLocation {
            bus_id: bus.to_owned(),
            port_chain: chain.to_vec(),
        }
    }

    fn edl_at(location: UsbLocation) -> UsbCandidate {
        UsbCandidate {
            mode: DeviceMode::Edl,
            serial: None,
            location,
            vendor_id: QUALCOMM_VID,
            product_id: QUALCOMM_EDL_PID,
            product: None,
            instance_id: None,
        }
    }

    type Resolved = std::result::Result<Option<UsbCandidate>, UsbSelectionError>;

    #[test]
    fn resolve_outcome_waits_only_while_the_target_is_not_in_edl() {
        let bound = loc("1", &[4]);
        assert_eq!(
            edl_target_outcome(Ok(Some(edl_at(bound.clone())))).unwrap(),
            bound
        );
        assert!(matches!(
            edl_target_outcome(Ok(None)),
            Err(EdlError::PortNotFound)
        ));
        assert!(matches!(
            edl_target_outcome(Err(UsbSelectionError::UnprovenSuccessor("two new".into()))),
            Err(EdlError::TargetNotProven(reason)) if reason == "two new"
        ));
        assert!(matches!(
            edl_target_outcome(Err(UsbSelectionError::AmbiguousTarget)),
            Err(EdlError::AmbiguousTarget)
        ));
        assert!(matches!(
            edl_target_outcome(Err(UsbSelectionError::MultipleDevices)),
            Err(EdlError::MultipleDevices)
        ));
        assert!(matches!(
            edl_target_outcome(Err(UsbSelectionError::Enumeration("busy".into()))),
            Err(EdlError::Serial(message)) if message.contains("busy")
        ));
        for error in [
            UsbSelectionError::UnmappedEdlPort,
            UsbSelectionError::StatePoisoned,
            UsbSelectionError::TargetMismatch {
                expected: "A".into(),
                found: "B".into(),
            },
        ] {
            assert!(matches!(
                edl_target_outcome(Err(error)),
                Err(EdlError::Session(_))
            ));
        }
    }

    #[test]
    fn no_selection_error_reads_as_keep_waiting() {
        let errors = [
            UsbSelectionError::MultipleDevices,
            UsbSelectionError::Enumeration(String::new()),
            UsbSelectionError::AmbiguousTarget,
            UsbSelectionError::UnprovenSuccessor(String::new()),
            UsbSelectionError::TargetMismatch {
                expected: String::new(),
                found: String::new(),
            },
            UsbSelectionError::UnmappedEdlPort,
            UsbSelectionError::StatePoisoned,
        ];
        for error in errors {
            let label = format!("{error:?}");
            assert!(
                !matches!(edl_target_outcome(Err(error)), Err(EdlError::PortNotFound)),
                "{label} must stop the wait"
            );
        }
    }

    #[test]
    fn kernel_target_needs_one_usb_device_and_one_port() {
        assert_eq!(kernel_target_port(1, Ok("COM5".into())).unwrap(), "COM5");
        // The USB node can appear before the kernel driver creates its COM port.
        assert!(matches!(
            kernel_target_port(1, Err(EdlError::PortNotFound)),
            Err(EdlError::PortNotFound)
        ));
        let unmapped = UsbSelectionError::UnmappedEdlPort.to_string();
        for (usb, port) in [
            (1, Err(EdlError::MultipleDevices)),
            (2, Ok("COM5".to_string())),
            (2, Err(EdlError::PortNotFound)),
            (0, Ok("COM5".to_string())),
            (0, Err(EdlError::PortNotFound)),
        ] {
            assert!(matches!(
                kernel_target_port(usb, port),
                Err(EdlError::Session(message)) if message == unmapped
            ));
        }
        assert!(matches!(
            kernel_target_port(1, Err(EdlError::Serial("enum".into()))),
            Err(EdlError::Serial(_))
        ));
    }

    fn port_ids(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(port, id)| (port.to_string(), id.to_string()))
            .collect()
    }

    fn is_unmapped<T: std::fmt::Debug>(result: Result<T>) -> bool {
        matches!(result, Err(EdlError::Session(message))
            if message == UsbSelectionError::UnmappedEdlPort.to_string())
    }

    const USB_A: &str = r"USB\VID_05C6&PID_9008\5&2A3B4C5D&0&3";
    const USB_B: &str = r"USB\VID_05C6&PID_9008\5&2A3B4C5D&0&4";

    #[test]
    fn select_port_for_instance_picks_the_bound_device() {
        let ports = port_ids(&[("COM5", USB_A), ("COM6", USB_B)]);
        assert_eq!(select_port_for_instance(&ports, USB_A).unwrap(), "COM5");
        assert_eq!(select_port_for_instance(&ports, USB_B).unwrap(), "COM6");
    }

    #[test]
    fn select_port_for_instance_ignores_ascii_case() {
        let ports = port_ids(&[("COM5", USB_A)]);
        assert_eq!(
            select_port_for_instance(&ports, &USB_A.to_ascii_lowercase()).unwrap(),
            "COM5"
        );
    }

    #[test]
    fn select_port_for_instance_refuses_no_match_and_two_ports() {
        let other = r"USB\VID_05C6&PID_9008\5&FFFF&0&9";
        assert!(is_unmapped(select_port_for_instance(&[], USB_A)));
        assert!(is_unmapped(select_port_for_instance(
            &port_ids(&[("COM5", USB_B)]),
            USB_A
        )));
        assert!(is_unmapped(select_port_for_instance(
            &port_ids(&[("COM5", USB_A), ("COM6", USB_A)]),
            USB_A
        )));
        assert!(is_unmapped(select_port_for_instance(
            &port_ids(&[("COM5", other), ("COM6", USB_B)]),
            USB_A
        )));
    }

    #[test]
    fn select_port_for_instance_counts_one_port_under_two_ids_once() {
        // Own devnode id plus parent id of the same port.
        let hub = r"USB\ROOT_HUB30\4&1&0&0";
        let ports = port_ids(&[("COM5", USB_A), ("COM5", hub), ("com5", USB_A)]);
        assert_eq!(select_port_for_instance(&ports, USB_A).unwrap(), "COM5");
        // A child devnode: its own id differs, its parent is the bound USB device.
        let child = port_ids(&[
            ("COM5", r"USB\VID_05C6&PID_9008&MI_00\6&1&0&0000"),
            ("COM5", USB_A),
            ("COM6", r"USB\VID_05C6&PID_9008&MI_00\6&2&0&0000"),
            ("COM6", USB_B),
        ]);
        assert_eq!(select_port_for_instance(&child, USB_B).unwrap(), "COM6");
    }

    #[test]
    fn select_port_for_instance_never_matches_an_empty_id() {
        let ports = port_ids(&[("COM5", ""), ("COM6", USB_A)]);
        assert!(is_unmapped(select_port_for_instance(&ports, "")));
        assert!(is_unmapped(select_port_for_instance(
            &port_ids(&[("COM5", "")]),
            USB_A
        )));
        assert_eq!(select_port_for_instance(&ports, USB_A).unwrap(), "COM6");
    }

    #[test]
    fn kernel_target_port_mapped_waits_only_for_the_single_device_without_a_port() {
        // One USB device, COM port not created yet: keep waiting.
        assert!(matches!(
            kernel_target_port_mapped(1, Some(USB_A), &[]),
            Err(EdlError::PortNotFound)
        ));
        // One USB device whose only port belongs to something else, or has no id: refused.
        assert!(is_unmapped(kernel_target_port_mapped(
            1,
            Some(USB_A),
            &port_ids(&[("COM5", USB_B)])
        )));
        assert!(is_unmapped(kernel_target_port_mapped(
            1,
            Some(USB_A),
            &port_ids(&[("COM5", "")])
        )));
        // The bound device has no instance id: refused, not guessed.
        assert!(is_unmapped(kernel_target_port_mapped(
            1,
            None,
            &port_ids(&[("COM5", USB_A)])
        )));
        // Several USB devices: the matching port is found; a missing one is refused.
        let ports = port_ids(&[("COM5", USB_A), ("COM6", USB_B)]);
        assert_eq!(
            kernel_target_port_mapped(2, Some(USB_B), &ports).unwrap(),
            "COM6"
        );
        assert!(is_unmapped(kernel_target_port_mapped(2, Some(USB_A), &[])));
        assert!(is_unmapped(kernel_target_port_mapped(
            2,
            Some(USB_A),
            &port_ids(&[("COM6", USB_B)])
        )));
    }

    #[test]
    fn transport_setup_requires_the_waited_device() {
        let at = |chain: &[u8], port: &str| EdlProbe {
            port: port.to_owned(),
            location: Some(loc("1", chain)),
        };
        let waited = at(&[4], "COM5");
        assert!(ensure_probe_unchanged(&waited, Ok(waited.clone())).is_ok());
        for now in [
            Ok(at(&[5], "COM5")),
            Ok(at(&[4], "COM6")),
            Err(EdlError::PortNotFound),
        ] {
            assert!(matches!(
                ensure_probe_unchanged(&waited, now),
                Err(EdlError::Session(message)) if message.starts_with("EDL device changed before transport setup")
            ));
        }
        assert!(matches!(
            ensure_probe_unchanged(&waited, Err(EdlError::TargetNotProven("gone".into()))),
            Err(EdlError::TargetNotProven(_))
        ));
        assert!(matches!(
            ensure_probe_unchanged(&waited, Err(EdlError::AmbiguousTarget)),
            Err(EdlError::AmbiguousTarget)
        ));
    }

    #[test]
    fn location_marker_names_the_location() {
        let a = edl_location_marker(&loc("1", &[4]));
        assert!(a.starts_with(EDL_DEVICE_MARKER));
        assert_eq!(a, edl_location_marker(&loc("1", &[4])));
        assert_ne!(a, edl_location_marker(&loc("1", &[5])));
        assert_ne!(a, edl_location_marker(&loc("2", &[4])));
        assert_ne!(
            edl_location_marker(&loc("1", &[1, 4])),
            edl_location_marker(&loc("1", &[14]))
        );
        assert_ne!(a, EDL_DEVICE_MARKER);
    }

    /// Runs the real wait loop over scripted `resolve` answers, mapped as the userspace
    /// targeted finder does. Panics if the loop sleeps more than `max_sleeps` times.
    fn wait_on_resolves(answers: Vec<Resolved>, max_sleeps: usize) -> Result<EdlProbe> {
        let mut queue: VecDeque<Resolved> = answers.into();
        let sleeps = Cell::new(0usize);
        wait_for_stable_with(
            || {
                let location = edl_target_outcome(queue.pop_front().unwrap_or(Ok(None)))?;
                Ok(EdlProbe {
                    port: edl_location_marker(&location),
                    location: Some(location),
                })
            },
            |_| {
                sleeps.set(sleeps.get() + 1);
                assert!(sleeps.get() <= max_sleeps, "wait kept polling after a stop");
            },
            || false,
            EDL_SESSION_OPEN_TIMEOUT,
        )
    }

    #[test]
    fn wait_keeps_polling_until_the_successor_is_proven() {
        let bound = loc("1", &[4]);
        let probe = wait_on_resolves(
            vec![
                Ok(None),
                Ok(None),
                Ok(Some(edl_at(bound.clone()))),
                Ok(Some(edl_at(bound.clone()))),
            ],
            3,
        )
        .expect("bound device");
        assert_eq!(probe.location, Some(bound.clone()));
        assert_eq!(probe.port, edl_location_marker(&bound));
    }

    #[test]
    fn refused_proof_stops_the_wait_at_once() {
        // Initial probe.
        assert!(matches!(
            wait_on_resolves(
                vec![Err(UsbSelectionError::UnprovenSuccessor("x".into()))],
                0
            ),
            Err(EdlError::TargetNotProven(_))
        ));
        assert!(matches!(
            wait_on_resolves(vec![Err(UsbSelectionError::AmbiguousTarget)], 0),
            Err(EdlError::AmbiguousTarget)
        ));
        // Phase 1: seen once, then refused.
        let bound = loc("1", &[4]);
        assert!(matches!(
            wait_on_resolves(
                vec![
                    Ok(Some(edl_at(bound.clone()))),
                    Err(UsbSelectionError::UnprovenSuccessor("x".into())),
                ],
                1
            ),
            Err(EdlError::TargetNotProven(_))
        ));
        // Phase 2: not there yet, then refused.
        assert!(matches!(
            wait_on_resolves(vec![Ok(None), Err(UsbSelectionError::AmbiguousTarget)], 1),
            Err(EdlError::AmbiguousTarget)
        ));
    }

    #[test]
    fn location_change_restarts_stabilisation() {
        let first = loc("1", &[4]);
        let second = loc("1", &[5]);
        let probe = wait_on_resolves(
            vec![
                Ok(Some(edl_at(first.clone()))),
                Ok(Some(edl_at(second.clone()))), // phase 1 change → phase 2
                Ok(Some(edl_at(second.clone()))),
                Ok(Some(edl_at(second.clone()))),
            ],
            3,
        )
        .expect("stable location");
        assert_eq!(probe.location, Some(second));
    }
}
