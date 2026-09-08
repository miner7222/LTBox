//! EDL (Emergency Download) — Qualcomm 9008 USB device detection and
//! session management (Sahara → Firehose configure → operations).
//!
//! Transport follows the configured Qualcomm driver mode:
//!
//! * userspace driver mode: `QdlBackend::Usb` (WinUSB stub on Windows via
//!   `qcom-usb-userspace-drivers` / libusb-style access on Linux/macOS)
//! * kernel driver mode: `QdlBackend::Serial` (Windows COM / Linux tty exposed
//!   by `qcom-usb-kernel-drivers`)

use std::io::{Cursor, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use thiserror::Error;

mod partition_flash;
pub use partition_flash::{PartitionFlash, PartitionFlashError};

use crate::driver::{QcomDriverMode, qcom_driver_mode};
use ltbox_core::i18n::tr;

use qdl::types::{
    FirehoseConfiguration, FirehoseResetMode, FirehoseStorageType, QdlBackend, QdlChan, QdlDevice,
    QdlReadWrite,
};

const QUALCOMM_VID: u16 = 0x05C6;
const QUALCOMM_EDL_PID: u16 = 0x9008;

/// Stable identifier returned by [`find_edl_device`] / [`wait_for_edl_device`]
/// when the EDL endpoint is visible via libusb. The actual `nusb::Device`
/// is opened later by qdl's USB backend (`qdl::usb::setup_usb_device`);
/// LTBox only needs a presence marker for logging + post-reset stability
/// checks, so we return a small synthetic string instead of plumbing a
/// raw `nusb` handle through the wait loop (which would tie the abstract
/// `wait_for_stable_port_with` helper to a concrete USB type and break
/// the unit tests that drive it with `String` ports).
const EDL_DEVICE_MARKER: &str = "USB:VID_05C6&PID_9008";

/// Build + send a Firehose `<erase>` XML to the device.
///
/// Inlined here instead of calling a `qdl::firehose_erase_storage`
/// wrapper so the dependency surface stays on the upstream-portable
/// `qdl::firehose_write_getack` primitive. Mirrors the v2 Python
/// flow that hand-writes a `FHLoaderErase.xml` and feeds it into
/// the Firehose pass: same XML payload, same end behaviour.
fn send_firehose_erase(
    dev: &mut QdlDevice<dyn QdlReadWrite>,
    num_sectors: usize,
    lun: u8,
    start_sector: &str,
) -> std::result::Result<(), String> {
    let sector_size = dev.fh_cfg.storage_sector_size;
    // Self-closed `<erase>` inside a `<data>` root with the XML
    // declaration matches what xmltree emits in qdl's internal
    // `firehose_xml_setup`. Firehose's parser is lenient about
    // whitespace but strict about attribute names + spelling.
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
/// Published by the rawprogram flash path (`EdlSession::flash_program_node`)
/// without parsing terminal `pbr` output. The last value remains available
/// until [`clear_flash_progress`] so short gaps between partitions do not
/// flicker empty in the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlashProgress {
    pub partition: String,
    pub percent: u8,
    pub completed_bytes: u64,
    pub total_bytes: u64,
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
    let next = FlashProgress {
        partition: partition.to_string(),
        percent,
        completed_bytes,
        total_bytes,
    };
    match flash_progress_slot().lock() {
        Ok(mut guard) => *guard = Some(next),
        // Poisoning should not panic production flash/GUI polling paths.
        Err(poisoned) => *poisoned.into_inner() = Some(next),
    }
}

/// Integer percent in `0..=100`, overflow-safe for large byte totals.
fn flash_percent(completed: u64, total: u64) -> u8 {
    if total == 0 {
        return 100;
    }
    let completed = completed.min(total);
    // Prefer saturating math over `completed * 100` which can overflow u64.
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

/// Scan for the Qualcomm 9008 EDL endpoint via libusb. Returns
/// [`EDL_DEVICE_MARKER`] when exactly one matching device is enumerated,
/// otherwise [`EdlError::PortNotFound`].
///
/// Implemented with `nusb::list_devices()` to mirror the discovery path
/// `qdl::usb::setup_usb_device` will take when actually opening the
/// transport — keeping both probes on the same enumeration source avoids
/// "visible to probe, invisible to open" mismatch (the classic failure
/// mode of the previous code, which probed `serialport::available_ports`
/// and then opened a totally different transport on top of it).
pub fn find_edl_device() -> Result<String> {
    use nusb::MaybeFuture;
    let devices = nusb::list_devices()
        .wait()
        .map_err(|e| EdlError::Serial(format!("USB device enumeration failed: {e}")))?;
    match devices
        .filter(|d| d.vendor_id() == QUALCOMM_VID && d.product_id() == QUALCOMM_EDL_PID)
        .count()
    {
        0 => Err(EdlError::PortNotFound),
        1 => Ok(EDL_DEVICE_MARKER.to_string()),
        _ => Err(EdlError::MultipleDevices),
    }
}

/// Scan for the Qualcomm 9008 serial port exposed by the kernel-driver mode.
pub fn find_edl_port() -> Result<String> {
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
    candidates.extend(find_qcom_edl_ports_windows()?);
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

/// Windows-only: resolve the EDL 9008 serial COM port by hardware ID via
/// SetupAPI, bypassing `serialport`'s USB classification (which fails on the
/// WDF kernel driver's non-standard metadata). Enumerates only currently
/// present devices in the Ports class (`DIGCF_PRESENT`), matches the hardware
/// ID `VID_05C6&PID_9008`, and reads the assigned `PortName` (e.g. `COM3`) from
/// the device's registry key. Returns an empty list when no such port exists — the
/// device is absent, or bound to a non-serial driver that created no COM port.
#[cfg(windows)]
fn find_qcom_edl_ports_windows() -> Result<Vec<String>> {
    use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
        DICS_FLAG_GLOBAL, DIGCF_PRESENT, DIREG_DEV, SP_DEVINFO_DATA, SPDRP_HARDWAREID,
        SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo, SetupDiGetClassDevsW,
        SetupDiGetDeviceRegistryPropertyW, SetupDiOpenDevRegKey,
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
                    found.push(port);
                }
            }
        }
        SetupDiDestroyDeviceInfoList(hdev);
        Ok(found)
    }
}

pub fn check_device() -> bool {
    match qcom_driver_mode() {
        QcomDriverMode::Userspace => find_edl_device().is_ok(),
        QcomDriverMode::Kernel => find_edl_port().is_ok(),
    }
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
fn wait_for_stable_port_with<F, S, D>(
    mut find_port: F,
    mut sleep: S,
    mut deadline_exceeded: D,
    timeout: Duration,
) -> Result<String>
where
    F: FnMut() -> Result<String>,
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

    // Phase 2: wait for post-reset port to appear + stabilize.
    let mut last: Option<String> = None;
    while !deadline_exceeded() {
        sleep(EDL_STABILITY_INTERVAL);
        match find_port() {
            Ok(port) => {
                if last.as_deref() == Some(port.as_str()) {
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

fn wait_for_stable_port(mode: QcomDriverMode) -> Result<String> {
    let deadline = Instant::now() + EDL_SESSION_OPEN_TIMEOUT;
    let finder: fn() -> Result<String> = match mode {
        QcomDriverMode::Userspace => find_edl_device,
        QcomDriverMode::Kernel => find_edl_port,
    };
    wait_for_stable_port_with(
        finder,
        std::thread::sleep,
        move || Instant::now() >= deadline,
        EDL_SESSION_OPEN_TIMEOUT,
    )
}

/// Wait for an EDL device; returns the stable-device marker string.
pub fn wait_for_device() -> Result<String> {
    wait_for_stable_port(qcom_driver_mode())
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
pub struct EdlSession {
    dev: QdlDevice<dyn QdlReadWrite>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WipeErasePlanEntry {
    label: String,
    lun: u8,
    start_sector: String,
    num_sectors: usize,
}

impl WipeErasePlanEntry {
    fn log_line(&self) -> String {
        self.log_line_with_template(&tr("log_edl_pre_erase_cmd"))
    }

    fn log_line_with_template(&self, template: &str) -> String {
        format!(
            "[EDL] {}",
            template
                .replace("{label}", &self.label)
                .replace("{lun}", &self.lun.to_string())
                .replace("{start}", &self.start_sector)
                .replace("{sectors}", &self.num_sectors.to_string())
        )
    }
}

impl EdlSession {
    /// Open: find port → Sahara upload → Firehose configure.
    pub fn open(loader_path: &Path, log: &mut Vec<String>) -> Result<Self> {
        crate::selection::ensure_single_usb_target()
            .map_err(|e| EdlError::Session(e.to_string()))?;
        let mode = qcom_driver_mode();
        ltbox_core::live!(log, "[EDL] {}", tr("log_edl_scanning"));
        let port = wait_for_stable_port(mode)?;
        // `port` is now a libusb marker string ("USB:VID_05C6&PID_9008"),
        // not a COM port name — the log line wording is generic enough
        // ("found on …") that the swap doesn't require an i18n update.
        ltbox_core::live!(log, "[EDL] {} {port}", tr("log_edl_found_on"));

        // An encrypted manifest (`qsahara_device_programmer.x`) is decrypted
        // to the plaintext `.xml` beside it before use, so its per-id images
        // (referenced relative to that directory) still resolve. Some Lenovo
        // firmware packs ship the manifest only in this encrypted form. This
        // is the single point loaders are consumed, so decrypting here covers
        // every caller (Flash / Unroot / Rescue / DetectArb / Dump).
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

        // Manifest (TB323FU): slot array indexed by Sahara image-id.
        // Single loader (.melf/.mbn/.elf): one-element slice at slot 0.
        let mut slots: Vec<Option<Vec<u8>>> =
            if ltbox_core::sahara_xml::is_manifest_filename(loader_path) {
                ltbox_core::live!(
                    log,
                    "[EDL] {} {}",
                    tr("log_edl_loading_programmer"),
                    loader_path.display()
                );
                let (slots, paths) = ltbox_core::sahara_xml::load_image_slots(loader_path)
                    .map_err(|e| EdlError::Session(format!("Sahara manifest: {e}")))?;
                let total: usize = slots
                    .iter()
                    .filter_map(|s| s.as_ref().map(|b| b.len()))
                    .sum();
                ltbox_core::live!(
                    log,
                    "[EDL] {} ({} images, {} bytes total)",
                    tr("log_edl_programmer_size"),
                    paths.len(),
                    total
                );
                slots
            } else {
                ltbox_core::live!(
                    log,
                    "[EDL] {} {}",
                    tr("log_edl_loading_programmer"),
                    loader_path.display()
                );
                let mbn = std::fs::read(loader_path)
                    .map_err(|e| EdlError::Session(format!("Failed to read loader: {e}")))?;
                ltbox_core::live!(
                    log,
                    "[EDL] {} {} bytes",
                    tr("log_edl_programmer_size"),
                    mbn.len()
                );
                vec![Some(mbn)]
            };

        ltbox_core::live!(log, "[EDL] {}", tr("log_edl_transport_setup"));
        let backend = match mode {
            QcomDriverMode::Userspace => QdlBackend::Usb,
            QcomDriverMode::Kernel => QdlBackend::Serial,
        };
        crate::selection::ensure_single_usb_target()
            .map_err(|e| EdlError::Session(e.to_string()))?;
        if mode == QcomDriverMode::Kernel && find_edl_port()? != port {
            return Err(EdlError::Session(
                "EDL port changed before transport setup; retry the operation".into(),
            ));
        }
        let rw = match mode {
            QcomDriverMode::Userspace => qdl::setup_target_device(backend, None, None)
                .map_err(|e| EdlError::Session(format!("Transport setup failed: {e}")))?,
            QcomDriverMode::Kernel => {
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
        // First attempt: trust PBL to emit Sahara HELLO. `sahara_run`'s loop
        // blocks on the first `channel.read` waiting for the HELLO packet,
        // then replies with HELLO_RESP (qdl `sahara.rs:489-509`).
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
                // Skip-HELLO fallback: mirrors qdl CLI's
                // `--skip-hello-wait` (cli/src/main.rs:246-248): send an
                // unsolicited HELLO_RESP to nudge the PBL state machine
                // forward, then retry `sahara_run`. Only triggered on a
                // timeout-shaped error so happy-path devices keep using the
                // standard handshake (no behavior change unless the first
                // attempt already failed).
                //
                // Primary check: structured downcast to
                // `io::ErrorKind::TimedOut` (qdl-rs propagates the
                // serialport read timeout as an `io::Error` wrapped in
                // `anyhow::Error`). String fallback is defense-in-depth in
                // case a future qdl-rs revision changes the wrapping.
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

        // Keep reset_on_drop false to dodge qdl's recursive reset.
        dev.reset_on_drop = false;

        ltbox_core::live!(log, "[EDL] {}", tr("log_edl_firehose_configuring"));
        qdl::firehose_read_greeting(&mut dev)
            .map_err(|e| EdlError::Session(format!("Firehose read failed: {e}")))?;
        qdl::firehose_configure(&mut dev, false)
            .map_err(|e| EdlError::Session(format!("Firehose configure failed: {e}")))?;
        qdl::firehose_read(&mut dev, qdl::parsers::firehose_parser_configure_response)
            .map_err(|e| EdlError::Session(format!("Firehose config response failed: {e}")))?;
        ltbox_core::live!(log, "[EDL] {}", tr("log_edl_firehose_configured"));

        Ok(Self { dev })
    }

    /// Max plausible GPT metadata span (protective MBR + header + entry
    /// array) in sectors. A malformed or hostile GPT can report a huge
    /// `first_usable_lba`; passing it straight to `firehose_read_storage`
    /// as a sector count would allocate/read gigabytes. Real GPTs fit in a
    /// few dozen sectors, so 2048 is a generous ceiling.
    const MAX_GPT_METADATA_SECTORS: u64 = 2048;

    /// Extra sectors appended to a GPT metadata read so the partition-entry
    /// array is never the last sector delivered. The USB Firehose read can
    /// return the final sector of a transfer short/garbled; a fully-populated
    /// GPT (TB323FU LUN 4 carries 128 entries filling LBA 2..=5, i.e. up to
    /// `first_usable_lba`) reads its entry array right up to the buffer's last
    /// byte, so the flaky tail corrupts it and `gptman` fails to parse. Reading
    /// a couple of slack sectors keeps the entry array in the reliable middle;
    /// `gptman` ignores the trailing padding.
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

    /// Read the GPT of a single LUN. Returns the parsed `gptman::GPT`.
    /// Used by `scan_partitions` and `find_partition`.
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
    /// partitions. GPT placeholder slots (empty name or zero size) are
    /// dropped. Per-LUN failures are logged but do not abort the scan —
    /// devices often expose only a subset of the 0..=5 LUN range.
    pub fn scan_partitions(
        &mut self,
        lun_range: std::ops::RangeInclusive<u8>,
        log: &mut Vec<String>,
    ) -> Result<Vec<GptPartitionInfo>> {
        let sector_size = self.dev.fh_config().storage_sector_size as u64;
        let mut out = Vec::new();
        for lun in lun_range {
            ltbox_core::live!(log, "[EDL] {} LUN {lun}", tr("log_edl_reading_gpt"));
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
                            size_bytes: num_sectors * sector_size,
                        });
                    }
                }
                Err(e) => {
                    ltbox_core::live!(
                        log,
                        "[EDL] {}",
                        tr("log_edl_lun_gpt_read_failed")
                            .replace("{lun}", &lun.to_string())
                            .replace("{error}", &e.to_string())
                    );
                }
            }
        }
        Ok(out)
    }

    /// Resolve a partition's UFS LUN: the static [`ltbox_core::partition_lun`]
    /// table first (no I/O), then a GPT scan across LUNs `0..=5` as a fallback
    /// for partitions not in the table (a model whose rawprogram wasn't compared,
    /// or a future layout). Slot suffix / case are normalised. Errors when the
    /// label is on no LUN's GPT.
    pub fn lun_for(&mut self, label: &str, log: &mut Vec<String>) -> Result<u8> {
        if let Some(lun) = ltbox_core::partition_lun::lun_for_partition(label) {
            return Ok(lun);
        }
        let parts = self.scan_partitions(0..=5, log)?;
        // Resolve from the device GPT: an EXACT label match first, then the
        // slot-stripped base. Within each tier the matches MUST agree on one LUN.
        // A deliberately multi-LUN label (e.g. `xbl` on LUN 1+2, `last_parti` on
        // every LUN) is omitted from the static map precisely because it has no
        // single LUN, so returning an arbitrary first hit would defeat that
        // omission — error on ambiguity instead. A slot-suffixed `xbl_a` / `xbl_b`
        // resolves through the exact tier to its own entry.
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

    /// Collapse the LUNs of GPT entries that matched a label into a single LUN.
    /// `Ok(None)` = no match (caller tries the next tier or reports not-found);
    /// `Ok(Some(lun))` = every match shares one LUN; `Err(AmbiguousPartitionLun)`
    /// = the label spans more than one distinct LUN and cannot be targeted
    /// unambiguously (a multi-LUN label deliberately absent from the static map).
    fn single_matching_lun(luns: &[u8], label: &str) -> Result<Option<u8>> {
        match luns.first() {
            None => Ok(None),
            Some(&first) if luns.iter().all(|&l| l == first) => Ok(Some(first)),
            Some(_) => Err(EdlError::AmbiguousPartitionLun(label.to_string())),
        }
    }

    /// GPT lookup by partition name.
    fn find_partition(&mut self, part_name: &str, slot: u8, lun: u8) -> Result<(u64, u64)> {
        let mut buf = Cursor::new(Vec::<u8>::new());
        qdl::firehose_read_storage(&mut self.dev, &mut buf, 1, slot, lun, 1)
            .map_err(|e| EdlError::Session(format!("GPT probe failed: {e}")))?;
        buf.rewind()?;
        let header = gptman::GPTHeader::read_from(&mut buf)
            .map_err(|e| EdlError::Session(format!("GPT header parse failed: {e}")))?;
        let gpt_len = Self::gpt_read_sectors(header.first_usable_lba)?;
        // Over-read past the GPT metadata — see `GPT_READ_TAIL_MARGIN`.
        let read_len =
            (gpt_len + Self::GPT_READ_TAIL_MARGIN).min(Self::MAX_GPT_METADATA_SECTORS as usize);
        buf.rewind()?;
        qdl::firehose_read_storage(&mut self.dev, &mut buf, read_len, slot, lun, 0)
            .map_err(|e| EdlError::Session(format!("GPT read failed: {e}")))?;
        buf.set_position(self.dev.fh_config().storage_sector_size as u64);
        let gpt = gptman::GPT::read_from(&mut buf, self.dev.fh_config().storage_sector_size as u64)
            .map_err(|e| EdlError::Session(format!("GPT parse failed: {e}")))?;

        let part = gpt
            .iter()
            .find(|(_, p)| p.partition_name.as_str() == part_name)
            .ok_or_else(|| EdlError::PartitionNotFound(part_name.to_string()))?
            .1;
        Ok((part.starting_lba, part.ending_lba))
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
        ltbox_core::live!(
            log,
            "[EDL] {} '{part_name}' on LUN {lun}...",
            tr("log_edl_lookup_partition")
        );
        let (start, end) = self.find_partition(part_name, slot, lun)?;
        // Reject degenerate GPT entries up-front: end<start wraps under
        // u64, end==start is a zero-sector partition (valid but nothing to
        // dump), and a sector count past usize::MAX cannot be allocated.
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
        ltbox_core::live!(
            log,
            "[EDL] {} {part_name}: LBA {start}-{end} ({sectors} sectors)",
            tr("log_edl_found_partition")
        );

        let mut out_file = std::fs::File::create(output)?;
        ltbox_core::live!(
            log,
            "[EDL] {} {part_name} → {}",
            tr("log_edl_dump_cmd"),
            output.display()
        );
        qdl::firehose_read_storage(&mut self.dev, &mut out_file, sectors, slot, lun, start)
            .map_err(|e| EdlError::Session(format!("Partition read failed: {e}")))?;
        ltbox_core::live!(log, "[EDL] {} {part_name}", tr("log_edl_dumped"));
        Ok(())
    }

    /// Dump with pre-resolved (LUN, start, length); skips GPT lookup.
    /// Needed because some Lenovo devices put boot/init_boot on non-zero
    /// LUNs that the LUN-0 GPT can't describe. Mirrors v2
    /// `EdlPartitionService.dump_partition`.
    pub fn dump_partition_at(
        &mut self,
        part_name: &str,
        output: &Path,
        lun: u8,
        start_sector: u64,
        num_sectors: usize,
        log: &mut Vec<String>,
    ) -> Result<()> {
        let mut out_file = std::fs::File::create(output)?;
        ltbox_core::live!(
            log,
            "[EDL] {} {part_name} → {} (LUN {lun}, start {start_sector}, {num_sectors} sectors)",
            tr("log_edl_dump_cmd"),
            output.display()
        );
        qdl::firehose_read_storage(
            &mut self.dev,
            &mut out_file,
            num_sectors,
            0,
            lun,
            start_sector,
        )
        .map_err(|e| EdlError::Session(format!("Partition read failed: {e}")))?;
        ltbox_core::live!(log, "[EDL] {} {part_name}", tr("log_edl_dumped"));
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
        let file_len = file.metadata()?.len();
        let sector_size = self.dev.fh_config().storage_sector_size as u64;
        let image_sectors = file_len.div_ceil(sector_size);
        // Refuse to program past the partition — an oversized image would
        // spill into the next partition and brick the device. The by-name
        // `flash_partition` resolves the span from the GPT; this explicit-
        // start variant relies on the caller's prior partition scan, passed
        // in as `partition_sectors`.
        if image_sectors > partition_sectors {
            return Err(EdlError::Session(format!(
                "Flash {part_name}: image is {image_sectors} sectors but the partition spans only {partition_sectors}"
            )));
        }
        let num_sectors = image_sectors as usize;
        ltbox_core::live!(
            log,
            "[EDL] {} {part_name} ← {} ({file_len} bytes, {num_sectors} sectors, LUN {lun})",
            tr("log_edl_flash_cmd"),
            image.display()
        );
        qdl::firehose_program_storage(
            &mut self.dev,
            &mut file,
            part_name,
            num_sectors,
            0,
            lun,
            start_sector,
        )
        .map_err(|e| EdlError::Session(format!("Partition write failed: {e}")))?;
        ltbox_core::live!(log, "[EDL] {} {part_name}", tr("log_edl_flashed"));
        Ok(())
    }

    /// Total sector count of a physical LUN. Probes the primary GPT
    /// header (sector 1) and returns `backup_lba + 1`, since
    /// `backup_lba` points at the backup header (last sector of the
    /// disk). Used by the Physical Storage Dump wizard to size a
    /// whole-LUN read without an explicit disk-size query.
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
            tr("log_edl_lun_total_sectors")
                .replace("{lun}", &lun.to_string())
                .replace("{total}", &total.to_string())
        );
        Ok(total)
    }

    /// Whole-LUN dump to a file. Reads every sector from LUN `lun`
    /// (count derived from the GPT header's `alternate_lba`) straight
    /// into `output`. Mirrors qdlrs `Dump` but without GPT decoding —
    /// the caller gets a raw physical image.
    pub fn dump_physical_storage(
        &mut self,
        lun: u8,
        output: &Path,
        log: &mut Vec<String>,
    ) -> Result<()> {
        let total = self.physical_lun_sector_count(lun, log)?;
        let mut out_file = std::fs::File::create(output)?;
        ltbox_core::live!(
            log,
            "[EDL] {}",
            tr("log_edl_dump_lun_cmd")
                .replace("{lun}", &lun.to_string())
                .replace("{path}", &output.display().to_string())
                .replace("{total}", &total.to_string())
        );
        qdl::firehose_read_storage(&mut self.dev, &mut out_file, total as usize, 0, lun, 0)
            .map_err(|e| EdlError::Session(format!("Physical LUN read failed: {e}")))?;
        ltbox_core::live!(
            log,
            "[EDL] {}",
            tr("log_edl_dumped_lun").replace("{lun}", &lun.to_string())
        );
        Ok(())
    }

    /// Whole-LUN raw flash. Mirrors qdlrs `OverwriteStorage`: empty
    /// partition name + start sector "0", file sector count derived
    /// from the image length. Queries the target LUN capacity first and
    /// refuses images that would program past the end of the disk.
    pub fn flash_physical_storage(
        &mut self,
        lun: u8,
        image: &Path,
        log: &mut Vec<String>,
    ) -> Result<()> {
        let mut file = std::fs::File::open(image)?;
        let file_len = file.metadata()?.len();
        let sector_size = self.dev.fh_config().storage_sector_size as u64;
        let image_sectors = file_len.div_ceil(sector_size);
        // Capacity probe before the write: an oversized image would spill
        // past the last LBA (and on some Firehose implementations wrap or
        // corrupt adjacent storage). Mirrors the partition-span check in
        // `flash_partition` / `flash_partition_at`.
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
            tr("log_edl_flash_lun_cmd")
                .replace("{lun}", &lun.to_string())
                .replace("{path}", &image.display().to_string())
                .replace("{bytes}", &file_len.to_string())
                .replace("{sectors}", &num_sectors.to_string())
        );
        qdl::firehose_program_storage(&mut self.dev, &mut file, "", num_sectors, 0, lun, "0")
            .map_err(|e| EdlError::Session(format!("Physical LUN write failed: {e}")))?;
        ltbox_core::live!(
            log,
            "[EDL] {}",
            tr("log_edl_flashed_lun").replace("{lun}", &lun.to_string())
        );
        Ok(())
    }

    /// Erase a sector range. Used by the Flash Partitions wizard when
    /// the user flags a row as "erase". `start_sector` is passed through
    /// Firehose as a string so negative offsets (e.g. "-1") stay valid.
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
            tr("log_edl_erase_part_cmd")
                .replace("{part}", part_name)
                .replace("{lun}", &lun.to_string())
                .replace("{start}", start_sector)
                .replace("{sectors}", &num_sectors.to_string())
        );
        send_firehose_erase(&mut self.dev, num_sectors, lun, start_sector)
            .map_err(|e| EdlError::Session(format!("Erase {part_name} failed: {e}")))?;
        ltbox_core::live!(
            log,
            "[EDL] {}",
            tr("log_edl_erased_part").replace("{part}", part_name)
        );
        Ok(())
    }

    /// Sectors spanned by a GPT partition (`end` is the inclusive last LBA).
    /// Errors on an inverted range so brick-critical erase/flash refuse bad
    /// geometry rather than silently touching a wrong, tiny span.
    fn partition_span_sectors(part_name: &str, start: u64, end: u64) -> Result<usize> {
        end.checked_sub(start)
            .map(|delta| delta as usize + 1)
            .ok_or_else(|| {
                EdlError::Session(format!(
                    "{part_name}: invalid GPT range (start {start} > end {end})"
                ))
            })
    }

    /// Erase a whole partition resolved by name from the device GPT. Looks up
    /// the partition's start/end LBA (like [`Self::flash_partition`]) and erases
    /// every sector it spans.
    pub fn erase_partition_by_name(
        &mut self,
        part_name: &str,
        slot: u8,
        lun: u8,
        log: &mut Vec<String>,
    ) -> Result<()> {
        ltbox_core::live!(
            log,
            "[EDL] {} '{part_name}' on LUN {lun}...",
            tr("log_edl_lookup_partition")
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
        ltbox_core::live!(
            log,
            "[EDL] {} '{part_name}' on LUN {lun}...",
            tr("log_edl_lookup_partition")
        );
        let (start, end) = self.find_partition(part_name, slot, lun)?;
        let span = Self::partition_span_sectors(part_name, start, end)?;

        let mut file = std::fs::File::open(image)?;
        let file_len = file.metadata()?.len();
        let sector_size = self.dev.fh_config().storage_sector_size as u64;
        let num_sectors = file_len.div_ceil(sector_size) as usize;
        // Refuse to program past the partition — an oversized image would
        // spill into the next partition and brick the device.
        if num_sectors > span {
            return Err(EdlError::Session(format!(
                "Flash {part_name}: image is {num_sectors} sectors but the partition spans only {span}"
            )));
        }
        ltbox_core::live!(
            log,
            "[EDL] {} {part_name} ← {} ({file_len} bytes, {num_sectors} sectors)",
            tr("log_edl_flash_cmd"),
            image.display()
        );

        qdl::firehose_program_storage(
            &mut self.dev,
            &mut file,
            part_name,
            num_sectors,
            slot,
            lun,
            &start.to_string(),
        )
        .map_err(|e| EdlError::Session(format!("Partition write failed: {e}")))?;
        ltbox_core::live!(log, "[EDL] {} {part_name}", tr("log_edl_flashed"));
        Ok(())
    }

    pub fn reset(&mut self, log: &mut Vec<String>) -> Result<()> {
        ltbox_core::live!(log, "[EDL] {}", tr("log_edl_reset_cmd"));
        qdl::firehose_reset(&mut self.dev, &FirehoseResetMode::Reset, 2)
            .map_err(|e| EdlError::Session(format!("Reset failed: {e}")))?;
        ltbox_core::live!(log, "[EDL] {}", tr("log_edl_reset_initiated"));
        Ok(())
    }

    /// Best-effort system reset for end-of-flow cleanup.
    ///
    /// v2 called qdl reset with `check=False`; some devices successfully
    /// reset the USB endpoint while qdl still reports an error. Use this
    /// after all destructive writes have already completed and the only
    /// remaining action is booting the device back to system.
    pub fn reset_tolerant(&mut self, log: &mut Vec<String>) {
        if let Err(e) = self.reset(log) {
            ltbox_core::live!(
                log,
                "[EDL] {}",
                tr("log_edl_reboot_handoff_error").replace("{error}", &e.to_string())
            );
        }
    }

    /// Bounce back to Sahara (does NOT boot system). Required after a
    /// dump-only session so the next `open()` gets a fresh Hello —
    /// otherwise Sahara times out. Mirrors v2 qdl-rs default behavior.
    pub fn reset_to_edl(&mut self, log: &mut Vec<String>) -> Result<()> {
        ltbox_core::live!(log, "[EDL] {}", tr("log_edl_reset_to_edl_cmd"));
        qdl::firehose_reset(&mut self.dev, &FirehoseResetMode::ResetToEdl, 0)
            .map_err(|e| EdlError::Session(format!("reset_to_edl failed: {e}")))?;
        ltbox_core::live!(log, "[EDL] {}", tr("log_edl_reset_to_edl_sent"));
        Ok(())
    }

    /// Mark `xbl_a`'s LUN as the boot drive (Firehose
    /// `<setbootablestoragedrive value="1"/>`, equivalent to fh_loader's
    /// `setactivepartition=1`). LUN 1 is hardcoded — every supported
    /// Lenovo Qualcomm tablet (TB320FC / TB321FU / TB322FC / TB323FU /
    /// TB520FU / TB710FU) places `xbl_a` on LUN 1.
    ///
    /// Lenovo firmware rawprograms only target `_a`, so a full firmware
    /// flash always lands on `_a`. Call this after the flash so the SoC
    /// boots from the freshly-written `_a` on the next reset; without
    /// it a device that was previously running on `_b` would continue
    /// booting `_b`'s pre-flash firmware.
    pub fn set_active_slot_a(&mut self, log: &mut Vec<String>) -> Result<()> {
        const XBL_A_LUN: u8 = 1;
        ltbox_core::live!(
            log,
            "[Flash] {}",
            tr("live_flash_set_bootable").replace("{lun}", &XBL_A_LUN.to_string())
        );
        qdl::firehose_set_bootable(&mut self.dev, XBL_A_LUN)
            .map_err(|e| EdlError::Session(format!("setbootablestoragedrive failed: {e}")))
    }

    /// Erased on wipe=true (matches v2 `_ERASE_LABELS`).
    const WIPE_ERASE_BASES: &'static [&'static str] = &["userdata", "metadata", "frp"];

    /// Skipped on wipe=false. Narrower than `WIPE_ERASE_BASES`: frp is
    /// not user state. Matches v2 `_patch_xml_for_wipe` (wipe=0).
    const KEEP_DATA_SKIP_BASES: &'static [&'static str] = &["userdata", "metadata"];

    /// Match `label` against bases, with or without `_a`/`_b` suffix.
    fn label_matches_base(label: &str, bases: &[&str]) -> bool {
        let l = label.to_ascii_lowercase();
        bases
            .iter()
            .any(|b| l == *b || l.starts_with(&format!("{b}_")))
    }

    fn wipe_labels(label: &str) -> bool {
        Self::label_matches_base(label, Self::WIPE_ERASE_BASES)
    }

    fn keep_data_skip_labels(label: &str) -> bool {
        Self::label_matches_base(label, Self::KEEP_DATA_SKIP_BASES)
    }

    fn is_split_super_image(filename: &str) -> bool {
        Path::new(filename).file_name().is_some_and(|name| {
            let name = name.to_string_lossy().to_ascii_lowercase();
            name.starts_with("super_") && name.ends_with(".img")
        })
    }

    /// Require every referenced split-super image before any device mutation.
    /// Other missing images keep the existing warning-and-skip behavior in
    /// `flash_program_node`.
    fn preflight_rawprogram_super_images(program_xmls: &[PathBuf]) -> Result<()> {
        for xml_path in program_xmls {
            let xml_content = std::fs::read_to_string(xml_path)?;
            let doc = roxmltree::Document::parse(&xml_content).map_err(|e| {
                EdlError::Session(format!("XML parse error in {}: {e}", xml_path.display()))
            })?;
            let xml_dir = xml_path.parent().unwrap_or(Path::new("."));

            for node in doc
                .descendants()
                .filter(|node| node.tag_name().name().eq_ignore_ascii_case("program"))
            {
                let filename = node.attribute("filename").unwrap_or("").trim();
                if filename.is_empty() || !Self::is_split_super_image(filename) {
                    continue;
                }

                let label = node.attribute("label").unwrap_or("").trim();
                let ctx = format!("{} <program label={label}>", xml_path.display());
                let num_sectors: usize =
                    parse_xml_attr(&node, "num_partition_sectors", 0usize, &ctx)?;
                if num_sectors == 0 {
                    continue;
                }

                let image_path =
                    ltbox_core::safe_path::safe_join(xml_dir, filename).map_err(|e| {
                        EdlError::Session(format!(
                            "invalid split-super image reference `{filename}` in {}: {e}",
                            xml_path.display()
                        ))
                    })?;
                if !image_path.exists() {
                    return Err(EdlError::Session(format!(
                        "required split-super image is missing: {} (referenced by {})",
                        image_path.display(),
                        xml_path.display()
                    )));
                }
            }
        }
        Ok(())
    }

    /// Flash with explicit user-data mode.
    ///
    /// `wipe=true` (v2 `pre_erase=True`): erase userdata/metadata/frp
    /// (+ slot variants) before flashing, then flash rawprograms, then
    /// apply patches.
    ///
    /// `wipe=false` (v2 `_patch_xml_for_wipe(wipe=0)`): skip
    /// userdata/metadata entries during the flash pass.
    pub fn flash_rawprogram_with_wipe(
        &mut self,
        program_xmls: &[PathBuf],
        patch_xmls: &[PathBuf],
        wipe: bool,
        log: &mut Vec<String>,
    ) -> Result<()> {
        Self::preflight_rawprogram_super_images(program_xmls)?;

        if wipe {
            ltbox_core::live!(log, "[Flash] {}", tr("log_flash_wipe_enabled"));
            self.pre_erase_wipe_labels(program_xmls, log)?;
        } else {
            ltbox_core::live!(log, "[Flash] {}", tr("log_flash_wipe_disabled"));
        }

        for xml_path in program_xmls {
            ltbox_core::live!(
                log,
                "[EDL] {} {}",
                tr("log_edl_flash_cmd"),
                xml_path.display()
            );
            self.flash_one_rawprogram(xml_path, wipe, log)?;
        }
        for xml_path in patch_xmls {
            // Show file name only — the full disk path was noisy and added
            // nothing the user couldn't already see in the firmware folder.
            let display_name = xml_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| xml_path.display().to_string());
            ltbox_core::live!(
                log,
                "[EDL] {}",
                tr("log_edl_patch_xml_cmd").replace("{path}", &display_name)
            );
            self.apply_patch_xml(xml_path, log)?;
        }
        Ok(())
    }

    /// Flash every `<program>` / `<erase>` node exactly as the rawprogram
    /// XMLs list them, then apply patch XMLs — no keep-data skipping and no
    /// pre-erase pass.
    ///
    /// This mirrors a stock Lenovo flash script as closely as possible: the
    /// data-wipe outcome is decided entirely by which rawprogram the catalog
    /// selected (e.g. a persist-preserving `save_persist` variant vs a
    /// `write_persist` one), not by any LTBox-side keep/wipe policy. Callers
    /// that want the userdata/metadata keep-skip or the userdata/metadata/frp
    /// pre-erase must use [`Self::flash_rawprogram_with_wipe`] instead.
    pub fn flash_rawprogram_verbatim(
        &mut self,
        program_xmls: &[PathBuf],
        patch_xmls: &[PathBuf],
        log: &mut Vec<String>,
    ) -> Result<()> {
        Self::preflight_rawprogram_super_images(program_xmls)?;

        for xml_path in program_xmls {
            ltbox_core::live!(
                log,
                "[EDL] {} {}",
                tr("log_edl_flash_cmd"),
                xml_path.display()
            );
            // `wipe = true` here only means "do not skip userdata/metadata":
            // `flash_one_rawprogram` writes every node verbatim and the
            // separate pre-erase pass is intentionally not run.
            self.flash_one_rawprogram(xml_path, true, log)?;
        }
        for xml_path in patch_xmls {
            let display_name = xml_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| xml_path.display().to_string());
            ltbox_core::live!(
                log,
                "[EDL] {}",
                tr("log_edl_patch_xml_cmd").replace("{path}", &display_name)
            );
            self.apply_patch_xml(xml_path, log)?;
        }
        Ok(())
    }

    /// Erase every `<program>` whose label is in `WIPE_ERASE_BASES`
    /// using XML-reported coordinates. Skips partitions absent from the XMLs.
    fn pre_erase_wipe_labels(
        &mut self,
        program_xmls: &[PathBuf],
        log: &mut Vec<String>,
    ) -> Result<()> {
        for entry in Self::collect_wipe_erase_plan(program_xmls)? {
            ltbox_core::live!(log, "{}", entry.log_line());
            send_firehose_erase(
                &mut self.dev,
                entry.num_sectors,
                entry.lun,
                &entry.start_sector,
            )
            .map_err(|e| EdlError::Session(format!("Erase {} failed: {e}", entry.label)))?;
        }
        Ok(())
    }

    fn collect_wipe_erase_plan(program_xmls: &[PathBuf]) -> Result<Vec<WipeErasePlanEntry>> {
        let mut plan = Vec::new();
        for xml_path in program_xmls {
            let xml_content = std::fs::read_to_string(xml_path)?;
            let doc = roxmltree::Document::parse(&xml_content).map_err(|e| {
                EdlError::Session(format!("XML parse error in {}: {e}", xml_path.display()))
            })?;
            for node in doc.descendants() {
                if !node.tag_name().name().eq_ignore_ascii_case("program") {
                    continue;
                }
                let label = node.attribute("label").unwrap_or("").trim();
                if !Self::wipe_labels(label) {
                    continue;
                }
                let ctx = format!("{} <program label={label}>", xml_path.display());
                let num_sectors: usize =
                    parse_xml_attr(&node, "num_partition_sectors", 0usize, &ctx)?;
                if num_sectors == 0 {
                    continue;
                }
                // Same coordinate gate as `<program>` / `<erase>` flash paths:
                // missing LUN or start would silently default to LUN0/LBA0
                // (primary GPT) via `parse_xml_attr` / `unwrap_or("0")`.
                require_destructive_coords(&node, &ctx)?;
                let lun: u8 = parse_xml_attr(&node, "physical_partition_number", 0u8, &ctx)?;
                let start_sector = node.attribute("start_sector").unwrap_or("0");
                plan.push(WipeErasePlanEntry {
                    label: label.to_string(),
                    lun,
                    start_sector: start_sector.to_string(),
                    num_sectors,
                });
            }
        }
        Ok(plan)
    }

    fn flash_one_rawprogram(
        &mut self,
        xml_path: &Path,
        wipe: bool,
        log: &mut Vec<String>,
    ) -> Result<()> {
        let xml_content = std::fs::read_to_string(xml_path)?;
        let doc = roxmltree::Document::parse(&xml_content).map_err(|e| {
            EdlError::Session(format!("XML parse error in {}: {e}", xml_path.display()))
        })?;
        let xml_dir = xml_path.parent().unwrap_or(Path::new("."));

        for node in doc.descendants() {
            match node.tag_name().name().to_lowercase().as_str() {
                "program" => {
                    // Keep-data: skip userdata/metadata entries.
                    if !wipe {
                        let label = node.attribute("label").unwrap_or("").trim();
                        if Self::keep_data_skip_labels(label) {
                            ltbox_core::live!(
                                log,
                                "[EDL] {}",
                                tr("log_edl_skip_keep_data").replace("{label}", label)
                            );
                            continue;
                        }
                    }
                    self.flash_program_node(&node, xml_dir, log)?;
                }
                "erase" => self.erase_program_node(&node, log)?,
                _ => continue,
            }
        }
        Ok(())
    }

    fn flash_program_node(
        &mut self,
        node: &roxmltree::Node<'_, '_>,
        xml_dir: &Path,
        log: &mut Vec<String>,
    ) -> Result<()> {
        let label = node.attribute("label").unwrap_or("").trim().to_string();
        let filename = node.attribute("filename").unwrap_or("").trim().to_string();
        let ctx = format!("<program label={label}>");
        let num_sectors: usize = parse_xml_attr(node, "num_partition_sectors", 0usize, &ctx)?;

        // Skip GPT placeholders / empty entries (qdl CLI `parse_program_cmd`).
        if filename.is_empty() || num_sectors == 0 {
            return Ok(());
        }

        require_destructive_coords(node, &ctx)?;
        let lun: u8 = parse_xml_attr(node, "physical_partition_number", 0u8, &ctx)?;
        let slot: u8 = parse_xml_attr(node, "slot", 0u8, &ctx)?;
        let start_sector = node.attribute("start_sector").unwrap_or("0");
        let file_sector_offset: u64 = parse_xml_attr(node, "file_sector_offset", 0u64, &ctx)?;
        let sector_size: u64 = self.dev.fh_config().storage_sector_size as u64;

        let image_path = ltbox_core::safe_path::safe_join(xml_dir, &filename)
            .map_err(|e| EdlError::Session(e.to_string()))?;
        if !image_path.exists() {
            ltbox_core::live!(
                log,
                "[EDL] {}",
                tr("log_edl_skip_image_missing")
                    .replace("{label}", &label)
                    .replace("{path}", &image_path.display().to_string())
            );
            return Ok(());
        }

        let mut file = std::fs::File::open(&image_path)?;
        if file_sector_offset > 0 {
            // `file_sector_offset * sector_size` can overflow u64 if the
            // rawprogram XML carries a hostile or corrupted value (untrusted
            // input — the same XML that names the partition). On overflow the
            // wrap-around lands at a tiny offset and we'd flash bytes from the
            // wrong region of `image_path` to the device. Reject overflow and
            // require the resulting byte offset to fit inside the image file
            // so we never seek past EOF and feed Firehose stale read data.
            let byte_offset = sector_size.checked_mul(file_sector_offset).ok_or_else(|| {
                EdlError::Session(format!(
                    "{ctx}: file_sector_offset {file_sector_offset} \
                         × sector_size {sector_size} overflows u64"
                ))
            })?;
            let file_len = file.metadata().map(|m| m.len()).unwrap_or(0);
            if byte_offset >= file_len {
                return Err(EdlError::Session(format!(
                    "{ctx}: file_sector_offset {file_sector_offset} \
                     (byte offset {byte_offset}) >= image length {file_len} \
                     for {}",
                    image_path.display(),
                )));
            }
            file.seek(SeekFrom::Start(byte_offset))?;
        }

        ltbox_core::live!(
            log,
            "[EDL] {}",
            tr("log_edl_flash_program_cmd")
                .replace("{label}", &label)
                .replace(
                    "{image}",
                    image_path
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or(""),
                )
                .replace("{lun}", &lun.to_string())
                .replace("{start}", start_sector)
                .replace("{sectors}", &num_sectors.to_string())
        );

        // Publish only when the integer percentage changes so the process-wide
        // slot is not locked/allocated on every Firehose chunk.
        let mut last_percent: Option<u8> = None;
        qdl::firehose_program_storage_with_progress(
            &mut self.dev,
            &mut file,
            &label,
            num_sectors,
            slot,
            lun,
            start_sector,
            |completed, total| update_flash_progress(&mut last_percent, &label, completed, total),
        )
        .map_err(|e| EdlError::Session(format!("Program {label} failed: {e}")))?;
        Ok(())
    }

    fn erase_program_node(
        &mut self,
        node: &roxmltree::Node<'_, '_>,
        log: &mut Vec<String>,
    ) -> Result<()> {
        let ctx = "<erase>";
        let num_sectors: usize = parse_xml_attr(node, "num_partition_sectors", 0usize, ctx)?;
        if num_sectors == 0 {
            return Ok(());
        }
        require_destructive_coords(node, ctx)?;
        let lun: u8 = parse_xml_attr(node, "physical_partition_number", 0u8, ctx)?;
        let start_sector = node.attribute("start_sector").unwrap_or("0");

        ltbox_core::live!(
            log,
            "[EDL] {}",
            tr("log_edl_erase_lun_cmd")
                .replace("{lun}", &lun.to_string())
                .replace("{start}", start_sector)
                .replace("{sectors}", &num_sectors.to_string())
        );
        send_firehose_erase(&mut self.dev, num_sectors, lun, start_sector)
            .map_err(|e| EdlError::Session(format!("Erase failed: {e}")))?;
        Ok(())
    }

    fn apply_patch_xml(&mut self, xml_path: &Path, log: &mut Vec<String>) -> Result<()> {
        let xml_content = std::fs::read_to_string(xml_path)?;
        let doc = roxmltree::Document::parse(&xml_content).map_err(|e| {
            EdlError::Session(format!("XML parse error in {}: {e}", xml_path.display()))
        })?;

        for node in doc.descendants() {
            if !node.tag_name().name().eq_ignore_ascii_case("patch") {
                continue;
            }
            // Non-DISK patches target files, not storage.
            let filename = node.attribute("filename").unwrap_or("");
            if filename != "DISK" {
                continue;
            }
            let ctx = "<patch>";
            require_destructive_coords(&node, ctx)?;
            let byte_off: u64 = parse_xml_attr(&node, "byte_offset", 0u64, ctx)?;
            let lun: u8 = parse_xml_attr(&node, "physical_partition_number", 0u8, ctx)?;
            let slot: u8 = parse_xml_attr(&node, "slot", 0u8, ctx)?;
            let size: u64 = parse_xml_attr(&node, "size_in_bytes", 0u64, ctx)?;
            let start_sector = node.attribute("start_sector").unwrap_or("0");
            let value = node.attribute("value").unwrap_or("");

            ltbox_core::live!(
                log,
                "[EDL] {}",
                tr("log_edl_patch_lun_cmd")
                    .replace("{lun}", &lun.to_string())
                    .replace("{start}", start_sector)
                    .replace("{offset}", &byte_off.to_string())
                    .replace("{bytes}", &size.to_string())
                    .replace("{value}", value)
            );
            qdl::firehose_patch(
                &mut self.dev,
                byte_off,
                slot,
                lun,
                size,
                start_sector,
                value,
            )
            .map_err(|e| EdlError::Session(format!("Patch failed: {e}")))?;
        }
        Ok(())
    }
}

/// Parse an XML attribute, distinguishing three cases:
///   - attribute absent → returns `default`
///   - attribute present and parseable → returns the parsed value
///   - attribute present but malformed → returns
///     `EdlError::Session` with context
///
/// Silently defaulting a malformed value (e.g. `num_partition_sectors="bogus"`
/// → 0) lets a corrupt rawprogram XML steer a flash at sector 0 or skip a
/// real partition entirely. Values that are legitimately optional (e.g.
/// `slot`, `file_sector_offset`) still default cleanly when absent.
fn parse_xml_attr<T>(
    node: &roxmltree::Node<'_, '_>,
    attr: &str,
    default: T,
    context: &str,
) -> Result<T>
where
    T: std::str::FromStr,
    <T as std::str::FromStr>::Err: std::fmt::Display,
{
    match node.attribute(attr) {
        None => Ok(default),
        Some(raw) => raw
            .trim()
            .parse::<T>()
            .map_err(|e| EdlError::Session(format!("{context}: invalid {attr}='{raw}': {e}"))),
    }
}

/// Refuse a destructive node (`<program>`, `<erase>`, or a DISK `<patch>`)
/// that omits its target coordinates. Unlike `slot` / `file_sector_offset`,
/// `physical_partition_number` and `start_sector` are not optional here:
/// silently defaulting either to 0 would steer the write at LUN 0 / sector 0
/// — the primary GPT. Presence-only, since a `start_sector` may be a formula
/// string (e.g. `NUM_DISK_SECTORS-33.`) rather than a parseable integer.
fn require_destructive_coords(node: &roxmltree::Node<'_, '_>, context: &str) -> Result<()> {
    for attr in ["physical_partition_number", "start_sector"] {
        if node.attribute(attr).is_none() {
            return Err(EdlError::Session(format!(
                "{context}: missing required {attr} on a destructive node"
            )));
        }
    }
    Ok(())
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RawprogramFamily {
    Other,
    Persist,
    Devinfo,
}

fn rawprogram_family(name_lower: &str) -> RawprogramFamily {
    match name_lower {
        "rawprogram_unsparse0.xml"
        | "rawprogram_unsparse0-half.xml"
        | "rawprogram_write_persist_unsparse0.xml"
        | "rawprogram_save_persist_unsparse0.xml"
        | "rawprogram_save_persist_ota_unsparse0.xml" => RawprogramFamily::Persist,
        "rawprogram4.xml" | "rawprogram4_write_devinfo.xml" | "rawprogram_unsparse4.xml" => {
            RawprogramFamily::Devinfo
        }
        _ => RawprogramFamily::Other,
    }
}

fn filename_rank(path: &Path, preferred: &[&str]) -> usize {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    preferred
        .iter()
        .position(|candidate| name == *candidate)
        .unwrap_or(preferred.len())
}

fn select_one_by_name_priority(mut paths: Vec<PathBuf>, preferred: &[&str]) -> Option<PathBuf> {
    paths.sort_by(|a, b| {
        filename_rank(a, preferred)
            .cmp(&filename_rank(b, preferred))
            .then_with(|| a.file_name().cmp(&b.file_name()))
    });
    paths.into_iter().next()
}

fn select_persist_xml(paths: Vec<PathBuf>, allow_dp_filenames: bool) -> Option<PathBuf> {
    if allow_dp_filenames {
        select_one_by_name_priority(
            paths,
            &[
                "rawprogram_write_persist_unsparse0.xml",
                "rawprogram_unsparse0.xml",
                "rawprogram_save_persist_ota_unsparse0.xml",
                "rawprogram_save_persist_unsparse0.xml",
                "rawprogram_unsparse0-half.xml",
            ],
        )
    } else {
        select_one_by_name_priority(
            paths,
            &[
                "rawprogram_save_persist_ota_unsparse0.xml",
                "rawprogram_save_persist_unsparse0.xml",
                "rawprogram_unsparse0-half.xml",
                "rawprogram_unsparse0.xml",
                "rawprogram_write_persist_unsparse0.xml",
            ],
        )
    }
}

fn select_devinfo_xml(paths: Vec<PathBuf>, allow_dp_filenames: bool) -> Option<PathBuf> {
    if allow_dp_filenames {
        select_one_by_name_priority(
            paths,
            &[
                "rawprogram4_write_devinfo.xml",
                "rawprogram4.xml",
                "rawprogram_unsparse4.xml",
            ],
        )
    } else {
        select_one_by_name_priority(
            paths,
            &[
                "rawprogram4.xml",
                "rawprogram_unsparse4.xml",
                "rawprogram4_write_devinfo.xml",
            ],
        )
    }
}

fn validate_dp_filename_usage(raw_xmls: &[PathBuf], allow_dp_filenames: bool) -> Result<()> {
    for xml_path in raw_xmls {
        let xml_content = std::fs::read_to_string(xml_path)?;
        let doc = roxmltree::Document::parse(&xml_content).map_err(|e| {
            EdlError::Session(format!("XML parse error in {}: {e}", xml_path.display()))
        })?;
        let xml_dir = xml_path.parent().unwrap_or(Path::new("."));
        for node in doc.descendants() {
            if !node.tag_name().name().eq_ignore_ascii_case("program") {
                continue;
            }
            let filename = node.attribute("filename").unwrap_or("").trim();
            let lower = filename.to_ascii_lowercase();
            if lower != "persist.img" && lower != "devinfo.img" {
                continue;
            }
            if !allow_dp_filenames {
                return Err(EdlError::Session(format!(
                    "{} references {filename}, but devinfo/persist image flashing is disabled",
                    xml_path.display()
                )));
            }
            let image_path = xml_dir.join(filename);
            if !image_path.exists() {
                return Err(EdlError::Session(format!(
                    "{} references {filename}, but {} is missing",
                    xml_path.display(),
                    image_path.display()
                )));
            }
        }
    }
    Ok(())
}

/// Collect `rawprogram*.xml` and `patch*.xml` from `dir` for firmware
/// flashing. Drops v2 filter targets (WIPE/BLANK variants,
/// `rawprogram0.xml` GPT programmer) and treats devinfo/persist XML
/// variants as mutually exclusive families.
///
/// `allow_dp_filenames=false` is the normal v3 firmware path: country-code
/// images are dumped, patched, and flashed explicitly after rawprogram
/// flashing, so a selected rawprogram must not reference `persist.img` or
/// `devinfo.img`.
pub fn collect_firmware_xmls_for_flash(
    dir: &Path,
    allow_dp_filenames: bool,
) -> Result<(Vec<PathBuf>, Vec<PathBuf>)> {
    let mut raw_xmls = Vec::new();
    let mut patch_xmls = Vec::new();
    let mut persist_xmls = Vec::new();
    let mut devinfo_xmls = Vec::new();
    // Plain `rawprogram0.xml`, deferred: used as the LUN0 source only when no
    // persist/`_unsparse0` variant supersedes it (see below).
    let mut rawprogram0: Option<PathBuf> = None;
    let entries = std::fs::read_dir(dir)?;

    for entry in entries.flatten() {
        let path = entry.path();
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        let lower = name.to_lowercase();
        if !lower.ends_with(".xml") {
            continue;
        }
        if lower.starts_with("rawprogram") {
            // v2 skips WIPE / zero-GPT variants.
            if name.contains("WIPE_PARTITIONS") || name.contains("BLANK_GPT") {
                continue;
            }
            if name == "rawprogram0.xml" {
                // Lenovo stock ships a persist/`_unsparse0` LUN0 variant that
                // supersedes the plain `rawprogram0.xml`, so defer it and use it
                // only as a fallback when no such variant exists — ported ROMs
                // ship only `rawprogram0.xml`, with `super` (the OS) on LUN0, so
                // dropping it unconditionally left `super` unflashed.
                rawprogram0 = Some(path);
                continue;
            }
            match rawprogram_family(&lower) {
                RawprogramFamily::Other => raw_xmls.push(path),
                RawprogramFamily::Persist => persist_xmls.push(path),
                RawprogramFamily::Devinfo => devinfo_xmls.push(path),
            }
        } else if lower.starts_with("patch") {
            patch_xmls.push(path);
        }
    }

    // LUN0 source: prefer the persist/`_unsparse0` variant; otherwise fall back
    // to the plain `rawprogram0.xml`. Either way `flash_rawprogram_with_wipe`'s
    // per-label keep/wipe skip still preserves userdata/metadata.
    match select_persist_xml(persist_xmls, allow_dp_filenames) {
        Some(path) => raw_xmls.push(path),
        None => {
            if let Some(path) = rawprogram0 {
                raw_xmls.push(path);
            }
        }
    }
    if let Some(path) = select_devinfo_xml(devinfo_xmls, allow_dp_filenames) {
        raw_xmls.push(path);
    }

    raw_xmls.sort();
    patch_xmls.sort();
    validate_dp_filename_usage(&raw_xmls, allow_dp_filenames)?;
    Ok((raw_xmls, patch_xmls))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    /// Process-unique suffix for temp files/dirs. `{pid}-{nanos}` alone is not
    /// enough: `cargo test` runs these in parallel within one process (shared
    /// pid), and the macOS x86_64 runner's `SystemTime` is coarse enough that
    /// two helpers entering the same tick got identical names — then one's
    /// `Drop` `remove_dir_all` deleted the dir the other was still writing,
    /// panicking with `NotFound`. A process-global counter makes the name
    /// unique even within a single clock tick.
    fn unique_temp_id() -> String {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        format!("{}-{nanos}-{seq}", std::process::id())
    }

    fn first_node<'a>(doc: &'a roxmltree::Document<'a>, tag: &str) -> roxmltree::Node<'a, 'a> {
        doc.descendants().find(|n| n.has_tag_name(tag)).unwrap()
    }

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
    fn flash_progress_latest_value_and_clear() {
        // Keep process-wide static state isolated for parallel cargo test.
        clear_flash_progress();
        let mut last = None;
        update_flash_progress(&mut last, "boot", 0, 200); // initial 0
        assert_eq!(last, Some(0));
        assert_eq!(flash_progress().unwrap().percent, 0);

        update_flash_progress(&mut last, "boot", 1, 200); // unchanged integer %
        assert_eq!(last, Some(0));
        assert_eq!(flash_progress().unwrap().percent, 0);

        update_flash_progress(&mut last, "boot", 200, 200); // final 100
        assert_eq!(last, Some(100));
        assert_eq!(flash_progress().unwrap().percent, 100);

        last = None;
        update_flash_progress(&mut last, "system", 0, 400); // new partition
        assert_eq!(
            flash_progress(),
            Some(FlashProgress {
                partition: "system".into(),
                percent: 0,
                completed_bytes: 0,
                total_bytes: 400,
            })
        );
        clear_flash_progress();
        assert_eq!(flash_progress(), None);
    }

    #[test]
    fn require_destructive_coords_demands_lun_and_start_sector() {
        // Both present (start_sector may be a formula string) → accepted.
        let ok = roxmltree::Document::parse(
            r#"<data><program physical_partition_number="0" start_sector="NUM_DISK_SECTORS-33."/></data>"#,
        )
        .unwrap();
        assert!(require_destructive_coords(&first_node(&ok, "program"), "<program>").is_ok());

        // Missing start_sector → rejected (would default to sector 0 = GPT).
        let no_start =
            roxmltree::Document::parse(r#"<data><program physical_partition_number="2"/></data>"#)
                .unwrap();
        assert!(
            require_destructive_coords(&first_node(&no_start, "program"), "<program>").is_err()
        );

        // Missing physical_partition_number → rejected (would default to LUN 0).
        let no_lun =
            roxmltree::Document::parse(r#"<data><erase start_sector="34"/></data>"#).unwrap();
        assert!(require_destructive_coords(&first_node(&no_lun, "erase"), "<erase>").is_err());
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
        // Start LBAs past the historical Firehose u32 limit must still
        // compute an inclusive span correctly (no hardware / no truncation).
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
        // Zero and implausibly large values (a malformed or hostile GPT) are
        // rejected before they can drive an unbounded Firehose read.
        assert!(EdlSession::gpt_read_sectors(0).is_err());
        assert!(EdlSession::gpt_read_sectors(EdlSession::MAX_GPT_METADATA_SECTORS + 1).is_err());
        assert!(EdlSession::gpt_read_sectors(u64::MAX).is_err());
    }

    struct TempXml(PathBuf);

    impl TempXml {
        fn new(contents: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("ltbox-edl-wipe-plan-{}.xml", unique_temp_id()));
            std::fs::write(&path, contents).expect("write temp rawprogram");
            Self(path)
        }

        fn path(&self) -> PathBuf {
            self.0.clone()
        }
    }

    struct TempFirmwareDir(PathBuf);

    impl TempFirmwareDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("ltbox-edl-fw-{}", unique_temp_id()));
            std::fs::create_dir_all(&path).expect("create temp firmware dir");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }

        fn write(&self, name: &str, contents: &str) {
            std::fs::write(self.0.join(name), contents).expect("write temp firmware file");
        }

        fn write_bytes(&self, name: &str, contents: &[u8]) {
            std::fs::write(self.0.join(name), contents).expect("write temp firmware image");
        }
    }

    impl Drop for TempFirmwareDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn xml_names(paths: &[PathBuf]) -> Vec<String> {
        paths
            .iter()
            .filter_map(|p| p.file_name().and_then(|n| n.to_str()).map(str::to_string))
            .collect()
    }

    impl Drop for TempXml {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
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
        // Regression: Phase 1 that only checks Ok/Err presence would latch
        // the dead COM6 handle across a reset-to-EDL renumeration.
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

    #[test]
    fn wipe_erase_plan_keeps_xml_order_and_log_geometry() {
        let rawprogram = TempXml::new(
            r#"
            <data>
              <program label="metadata" physical_partition_number="0" start_sector="8192" num_partition_sectors="2048" />
              <program label="super" physical_partition_number="0" start_sector="16384" num_partition_sectors="4096" />
              <program label="frp" physical_partition_number="1" start_sector="24576" num_partition_sectors="128" />
              <program label="userdata_a" physical_partition_number="2" start_sector="32768" num_partition_sectors="0" />
              <program label="userdata_b" physical_partition_number="3" start_sector="65536" num_partition_sectors="8192" />
            </data>
            "#,
        );

        let plan = EdlSession::collect_wipe_erase_plan(&[rawprogram.path()])
            .expect("collect wipe erase plan");

        assert_eq!(
            plan.iter()
                .map(|entry| entry.label.as_str())
                .collect::<Vec<_>>(),
            ["metadata", "frp", "userdata_b"]
        );
        let template = "erase {label} (LUN {lun}, start {start}, {sectors} sectors)";
        assert_eq!(
            plan[0].log_line_with_template(template),
            "[EDL] erase metadata (LUN 0, start 8192, 2048 sectors)"
        );
        assert_eq!(
            plan[1].log_line_with_template(template),
            "[EDL] erase frp (LUN 1, start 24576, 128 sectors)"
        );
        assert_eq!(
            plan[2].log_line_with_template(template),
            "[EDL] erase userdata_b (LUN 3, start 65536, 8192 sectors)"
        );
    }

    #[test]
    fn wipe_erase_plan_rejects_missing_destructive_coords() {
        // Missing start_sector would previously default to LBA 0 (primary GPT).
        let missing_start = TempXml::new(
            r#"
            <data>
              <program label="metadata" physical_partition_number="0" num_partition_sectors="2048" />
            </data>
            "#,
        );
        let err = EdlSession::collect_wipe_erase_plan(&[missing_start.path()])
            .expect_err("missing start_sector must fail");
        assert!(
            err.to_string().contains("start_sector"),
            "unexpected error: {err}"
        );

        // Missing physical_partition_number would previously default to LUN 0.
        let missing_lun = TempXml::new(
            r#"
            <data>
              <program label="frp" start_sector="24576" num_partition_sectors="128" />
            </data>
            "#,
        );
        let err = EdlSession::collect_wipe_erase_plan(&[missing_lun.path()])
            .expect_err("missing physical_partition_number must fail");
        assert!(
            err.to_string().contains("physical_partition_number"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rawprogram_super_image_preflight_rejects_missing_image() {
        let fw = TempFirmwareDir::new();
        fw.write(
            "rawprogram0.xml",
            r#"<data><program label="super" filename="images/SUPER_1.IMG" num_partition_sectors="1" /></data>"#,
        );

        let rawprogram = fw.path().join("rawprogram0.xml");
        let err = EdlSession::preflight_rawprogram_super_images(&[rawprogram])
            .expect_err("missing split-super image must fail preflight");

        assert!(matches!(err, EdlError::Session(_)));
        let message = err.to_string();
        assert!(
            message.contains("SUPER_1.IMG"),
            "unexpected error: {message}"
        );
        assert!(
            message.contains("rawprogram0.xml"),
            "unexpected error: {message}"
        );
    }

    #[test]
    fn rawprogram_super_image_preflight_allows_present_and_optional_missing_images() {
        let fw = TempFirmwareDir::new();
        let image_dir = fw.path().join("images");
        std::fs::create_dir_all(&image_dir).expect("create split-super image directory");
        std::fs::write(image_dir.join("SuPeR_2.ImG"), b"super").expect("write split-super image");
        fw.write(
            "rawprogram0.xml",
            r#"
            <data>
              <program label="super" filename="images/SuPeR_2.ImG" num_partition_sectors="1" />
              <program label="boot" filename="boot.img" num_partition_sectors="1" />
              <program label="super" filename="super.img" num_partition_sectors="1" />
              <program label="super" filename="super_ignored.img" num_partition_sectors="0" />
              <program label="placeholder" filename="" num_partition_sectors="1" />
            </data>
            "#,
        );

        let rawprogram = fw.path().join("rawprogram0.xml");
        EdlSession::preflight_rawprogram_super_images(&[rawprogram])
            .expect("present split-super and optional missing images should pass preflight");
    }

    #[test]
    fn collect_firmware_xmls_chooses_safe_dp_variants_once() {
        let fw = TempFirmwareDir::new();
        fw.write("rawprogram1.xml", "<data/>");
        fw.write(
            "rawprogram_unsparse0.xml",
            r#"<data><program label="persist" filename="persist.img" num_partition_sectors="1"/></data>"#,
        );
        fw.write(
            "rawprogram_unsparse0-half.xml",
            r#"<data><program label="persist" filename="" num_partition_sectors="1"/></data>"#,
        );
        fw.write(
            "rawprogram4.xml",
            r#"<data><program label="devinfo" filename="" num_partition_sectors="1"/></data>"#,
        );
        fw.write(
            "rawprogram4_write_devinfo.xml",
            r#"<data><program label="devinfo" filename="devinfo.img" num_partition_sectors="1"/></data>"#,
        );
        fw.write("patch0.xml", "<data/>");

        let (raw, patch) =
            collect_firmware_xmls_for_flash(fw.path(), false).expect("collect safe XMLs");
        let names = xml_names(&raw);

        assert!(names.contains(&"rawprogram1.xml".to_string()));
        assert!(names.contains(&"rawprogram_unsparse0-half.xml".to_string()));
        assert!(names.contains(&"rawprogram4.xml".to_string()));
        assert!(!names.contains(&"rawprogram_unsparse0.xml".to_string()));
        assert!(!names.contains(&"rawprogram4_write_devinfo.xml".to_string()));
        assert_eq!(
            names.len(),
            names.iter().collect::<std::collections::HashSet<_>>().len()
        );
        assert_eq!(xml_names(&patch), vec!["patch0.xml".to_string()]);
    }

    #[test]
    fn collect_firmware_xmls_prefers_persistless_lun0_for_simple_flash() {
        // Simple Flash reuses `collect_firmware_xmls_for_flash(dir, false)`:
        // when a firmware ships both a persist-less LUN0 rawprogram
        // (save_persist, empty persist filename) and a persist-writing one
        // (write_persist), the persist-less variant must win and be the *only*
        // LUN0 rawprogram kept.
        let fw = TempFirmwareDir::new();
        fw.write(
            "rawprogram_save_persist_unsparse0.xml",
            r#"<data><program label="persist" filename="" num_partition_sectors="1"/></data>"#,
        );
        fw.write(
            "rawprogram_write_persist_unsparse0.xml",
            r#"<data><program label="persist" filename="persist.img" num_partition_sectors="1"/></data>"#,
        );

        let (raw, _) =
            collect_firmware_xmls_for_flash(fw.path(), false).expect("collect persist-less LUN0");
        let names = xml_names(&raw);

        assert!(names.contains(&"rawprogram_save_persist_unsparse0.xml".to_string()));
        assert!(!names.contains(&"rawprogram_write_persist_unsparse0.xml".to_string()));
        // Exactly one LUN0 rawprogram is kept.
        let lun0 = names
            .iter()
            .filter(|n| n.contains("persist_unsparse0"))
            .count();
        assert_eq!(lun0, 1);
    }

    #[test]
    fn collect_firmware_xmls_allows_dp_xmls_when_images_exist() {
        let fw = TempFirmwareDir::new();
        fw.write("rawprogram1.xml", "<data/>");
        fw.write(
            "rawprogram_save_persist_unsparse0.xml",
            r#"<data><program label="persist" filename="" num_partition_sectors="1"/></data>"#,
        );
        fw.write(
            "rawprogram_write_persist_unsparse0.xml",
            r#"<data><program label="persist" filename="persist.img" num_partition_sectors="1"/></data>"#,
        );
        fw.write(
            "rawprogram4.xml",
            r#"<data><program label="devinfo" filename="" num_partition_sectors="1"/></data>"#,
        );
        fw.write(
            "rawprogram4_write_devinfo.xml",
            r#"<data><program label="devinfo" filename="devinfo.img" num_partition_sectors="1"/></data>"#,
        );
        fw.write_bytes("persist.img", b"persist");
        fw.write_bytes("devinfo.img", b"devinfo");

        let (raw, _) = collect_firmware_xmls_for_flash(fw.path(), true).expect("collect DP XMLs");
        let names = xml_names(&raw);

        assert!(names.contains(&"rawprogram_write_persist_unsparse0.xml".to_string()));
        assert!(names.contains(&"rawprogram4_write_devinfo.xml".to_string()));
        assert!(!names.contains(&"rawprogram_save_persist_unsparse0.xml".to_string()));
        assert!(!names.contains(&"rawprogram4.xml".to_string()));
        assert_eq!(
            names.len(),
            names.iter().collect::<std::collections::HashSet<_>>().len()
        );
    }

    #[test]
    fn collect_firmware_xmls_rejects_disabled_dp_references() {
        let fw = TempFirmwareDir::new();
        fw.write(
            "rawprogram_unsparse0.xml",
            r#"<data><program label="persist" filename="persist.img" num_partition_sectors="1"/></data>"#,
        );

        let err = collect_firmware_xmls_for_flash(fw.path(), false)
            .expect_err("persist.img should be rejected");
        assert!(err.to_string().contains("persist.img"));
        assert!(err.to_string().contains("disabled"));
    }

    #[test]
    fn collect_firmware_xmls_rejects_allowed_dp_when_image_missing() {
        let fw = TempFirmwareDir::new();
        fw.write(
            "rawprogram_write_persist_unsparse0.xml",
            r#"<data><program label="persist" filename="persist.img" num_partition_sectors="1"/></data>"#,
        );

        let err = collect_firmware_xmls_for_flash(fw.path(), true)
            .expect_err("persist.img image should be required");
        assert!(err.to_string().contains("persist.img"));
        assert!(err.to_string().contains("missing"));
    }

    #[test]
    fn real_firmware_xml_matrix_when_available() {
        let Some(dir) = std::env::var_os("LTBOX_REAL_FIRMWARE_DIR") else {
            return;
        };
        let dir = PathBuf::from(dir);
        if !dir.join("rawprogram_unsparse0-half.xml").exists() {
            return;
        }

        let (raw, patch) =
            collect_firmware_xmls_for_flash(&dir, false).expect("collect real safe XMLs");
        let names = xml_names(&raw);

        assert!(names.contains(&"rawprogram_unsparse0-half.xml".to_string()));
        assert!(!names.contains(&"rawprogram_unsparse0.xml".to_string()));
        assert!(names.contains(&"rawprogram4.xml".to_string()));
        assert_eq!(
            names.len(),
            names.iter().collect::<std::collections::HashSet<_>>().len()
        );
        assert!(!patch.is_empty());

        let wipe_plan = EdlSession::collect_wipe_erase_plan(&raw).expect("collect wipe plan");
        assert_eq!(
            wipe_plan
                .iter()
                .filter(|entry| entry.label == "metadata")
                .count(),
            7
        );
        assert_eq!(
            wipe_plan
                .iter()
                .filter(|entry| entry.label == "userdata")
                .count(),
            10
        );
        assert!(wipe_plan.iter().any(|entry| entry.label == "frp"));
    }

    #[test]
    fn rawprogram0_is_lun0_fallback_only_without_persist_variant() {
        let base = std::env::temp_dir().join(format!(
            "ltbox_xmlsel_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&base).unwrap();
        let has = |raw: &[PathBuf], n: &str| raw.iter().any(|p| p.file_name().unwrap() == n);
        let prog = |lun: u32, file: &str| {
            format!(
                r#"<data><program physical_partition_number="{lun}" start_sector="0" filename="{file}"/></data>"#
            )
        };
        std::fs::write(base.join("rawprogram0.xml"), prog(0, "super.img")).unwrap();
        std::fs::write(base.join("rawprogram1.xml"), prog(1, "xbl.img")).unwrap();

        // No persist / `_unsparse0` variant → rawprogram0.xml is the LUN0 source
        // (the ported-ROM case where `super` would otherwise never flash).
        let (raw, _) = collect_firmware_xmls_for_flash(&base, false).unwrap();
        assert!(has(&raw, "rawprogram0.xml"));

        // A persist variant supersedes the plain rawprogram0.xml (Lenovo stock).
        std::fs::write(base.join("rawprogram_unsparse0.xml"), prog(0, "super.img")).unwrap();
        let (raw2, _) = collect_firmware_xmls_for_flash(&base, false).unwrap();
        assert!(!has(&raw2, "rawprogram0.xml"));
        assert!(has(&raw2, "rawprogram_unsparse0.xml"));

        let _ = std::fs::remove_dir_all(&base);
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
