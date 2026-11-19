//! ADB client via `adb_client` crate's `usb` feature — direct libusb
//! transport, no background `adb.exe` daemon on `localhost:5037`.

use adb_client::usb::{ADBUSBDevice, USBTransport, find_all_connected_adb_devices};
use adb_client::{ADBDeviceExt, RebootType};
use rsa::pkcs8::{DecodePrivateKey, EncodePrivateKey, LineEnding};
use rusb::UsbContext;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;
use thiserror::Error;

use crate::selection::UsbCandidate;

const ADB_SERVER_ADDR: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 5037);
const ADB_SERVER_PROBE_TIMEOUT: Duration = Duration::from_millis(150);

/// Whether anything listens on `127.0.0.1:5037`. An adb server there holds the
/// Android USB interface, so our claim fails with `LIBUSB_ERROR_BUSY`; this
/// lets `check_device_state` report that instead of "unauthorized".
pub fn adb_server_running() -> bool {
    TcpStream::connect_timeout(&ADB_SERVER_ADDR.into(), ADB_SERVER_PROBE_TIMEOUT).is_ok()
}

/// Stop a running adb server by sending raw `host:kill` (4-byte ASCII-hex
/// length + command) to `127.0.0.1:5037`, with no `adb.exe` needed.
pub fn kill_adb_server() -> Result<()> {
    let mut stream = TcpStream::connect_timeout(&ADB_SERVER_ADDR.into(), Duration::from_secs(2))
        .map_err(|e| AdbError::Client(format!("adb server not reachable: {e}")))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|e| AdbError::Client(format!("set_write_timeout: {e}")))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|e| AdbError::Client(format!("set_read_timeout: {e}")))?;
    let payload = b"host:kill";
    let header = format!("{:04x}", payload.len());
    stream
        .write_all(header.as_bytes())
        .and_then(|_| stream.write_all(payload))
        .map_err(|e| AdbError::Client(format!("write host:kill: {e}")))?;
    // The exiting server may close without replying, so EOF / reset counts
    // as success; only `FAIL` is a rejection.
    let mut reply = [0u8; 4];
    match stream.read_exact(&mut reply) {
        Ok(()) => match &reply {
            b"OKAY" => Ok(()),
            b"FAIL" => {
                let mut len_buf = [0u8; 4];
                let _ = stream.read_exact(&mut len_buf);
                let n = std::str::from_utf8(&len_buf)
                    .ok()
                    .and_then(|s| usize::from_str_radix(s, 16).ok())
                    .unwrap_or(0);
                let mut msg_buf = vec![0u8; n.min(1024)];
                let _ = stream.read_exact(&mut msg_buf);
                Err(AdbError::Client(format!(
                    "adb server rejected host:kill: {}",
                    String::from_utf8_lossy(&msg_buf)
                )))
            }
            other => Err(AdbError::Client(format!(
                "unexpected reply to host:kill: {:?}",
                other
            ))),
        },
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::ConnectionReset
            ) =>
        {
            Ok(())
        }
        Err(e) => Err(AdbError::Client(format!("read host:kill reply: {e}"))),
    }
}

#[derive(Error, Debug)]
pub enum AdbError {
    #[error("ADB error: {0}")]
    Client(String),
    #[error("Command failed: {0}")]
    CommandFailed(String),
    #[error("Timeout waiting for device")]
    Timeout,
    /// More than one Android/EDL USB device is attached, so no single target
    /// can be chosen. Never a connect or authorization failure.
    #[error("Multiple Android/EDL USB devices found. Disconnect other devices and try again.")]
    MultipleDevices,
    /// Two attached ADB devices report the chosen device's serial number.
    #[error(
        "Two attached devices report the chosen device's serial number. Disconnect one and try again."
    )]
    AmbiguousTarget,
    /// The device that answered is not the chosen one; it was dropped unused.
    #[error("Wrong device: expected serial {expected}, found {found}")]
    TargetMismatch { expected: String, found: String },
    /// The chosen device could not be identified (no provable EDL successor, a
    /// chosen device without a USB serial, a poisoned target lock). Never a
    /// connect or authorization state.
    #[error("Cannot identify the chosen device: {0}")]
    Selection(String),
    /// Failed to read or write the LTBox-owned ADB RSA private key.
    #[error("Failed to prepare ADB key: {0}")]
    Key(String),
}

type Result<T> = std::result::Result<T, AdbError>;

/// Errors about which device to talk to. They are never a connect or
/// authorization state, so they are reported as errors instead of tokens.
fn is_selection_error(err: &AdbError) -> bool {
    matches!(
        err,
        AdbError::MultipleDevices
            | AdbError::AmbiguousTarget
            | AdbError::TargetMismatch { .. }
            | AdbError::Selection(_)
    )
}

/// Why one connect attempt failed, split by what the failure says about the device.
#[derive(Debug)]
enum ConnectFailure {
    /// Any other failure, including a failed auth handshake on the identified device.
    Error(AdbError),
    /// The chosen device could not be identified before any auth attempt: the target
    /// resolve or the rusb enumeration failed, the target is not among the rusb devices,
    /// or its iSerial cannot be read. Says nothing about authorization.
    Unidentified(String),
}

impl From<AdbError> for ConnectFailure {
    fn from(err: AdbError) -> Self {
        Self::Error(err)
    }
}

impl ConnectFailure {
    /// For callers that have no state token to report.
    fn into_error(self) -> AdbError {
        match self {
            Self::Error(err) => err,
            Self::Unidentified(reason) => AdbError::Selection(reason),
        }
    }
}

/// `targeted`: a session target is set. Enumeration failures are then a selection failure
/// (the chosen device cannot be identified); untargeted they stay a client error.
fn map_selection_error(err: crate::selection::UsbSelectionError, targeted: bool) -> AdbError {
    use crate::selection::UsbSelectionError;
    match err {
        UsbSelectionError::MultipleDevices => AdbError::MultipleDevices,
        UsbSelectionError::AmbiguousTarget => AdbError::AmbiguousTarget,
        UsbSelectionError::TargetMismatch { expected, found } => {
            AdbError::TargetMismatch { expected, found }
        }
        error @ UsbSelectionError::Enumeration(_) if !targeted => {
            AdbError::Client(error.to_string())
        }
        other => AdbError::Selection(other.to_string()),
    }
}

/// What `connect_device` must open.
enum AdbPick {
    /// No session target: today's autodetect of the single attached device.
    Any,
    /// The ADB device whose USB iSerial equals `serial`; `candidate` is what the target
    /// resolve saw (vendor/product ids and location), used to narrow the rusb devices.
    Serial {
        serial: String,
        candidate: UsbCandidate,
    },
}

/// Decide what to open. `Ok(None)`: a target is set and it is not in ADB.
fn pick_target() -> std::result::Result<Option<AdbPick>, ConnectFailure> {
    use crate::selection::{
        DeviceMode, UsbSelectionError, current_target, ensure_untargeted_single, resolve,
    };
    if current_target().is_none() {
        // One read of the target under the guard's own lock: a target chosen in between is
        // an error here, never a skipped guard.
        ensure_untargeted_single().map_err(|e| map_selection_error(e, false))?;
        return Ok(Some(AdbPick::Any));
    }
    let resolved = match resolve(DeviceMode::Adb) {
        Ok(resolved) => resolved,
        Err(UsbSelectionError::Enumeration(reason)) => {
            return Err(ConnectFailure::Unidentified(format!(
                "USB device enumeration failed: {reason}"
            )));
        }
        Err(other) => return Err(map_selection_error(other, true).into()),
    };
    match resolved {
        None => Ok(None),
        Some(candidate) => match candidate.serial.clone() {
            Some(serial) => Ok(Some(AdbPick::Serial { serial, candidate })),
            None => Err(AdbError::Selection(
                "the chosen ADB device reports no USB serial number".into(),
            )
            .into()),
        },
    }
}

/// The one device whose serial equals `serial`. Two or more is an error, never
/// the first; devices with no readable serial never match, nor does an empty
/// `serial`.
fn pick_adb_device<T>(
    devices: impl IntoIterator<Item = (T, Option<String>)>,
    serial: &str,
) -> Result<Option<T>> {
    let wanted = serial.trim();
    if wanted.is_empty() {
        return Ok(None);
    }
    let mut found = None;
    for (device, device_serial) in devices {
        if device_serial.as_deref().map(str::trim) == Some(wanted) {
            if found.is_some() {
                return Err(AdbError::AmbiguousTarget);
            }
            found = Some(device);
        }
    }
    Ok(found)
}

/// Which rusb devices may be opened to read an iSerial.
#[derive(Debug, PartialEq, Eq)]
struct RusbNarrowing {
    /// Exactly one same-vid/pid device sits on the candidate's port chain: read its iSerial
    /// first. A match ends the search; a mismatch falls back to `same_ids`, because rusb
    /// and nusb may number ports differently. Never a pick by port alone.
    by_port: Option<usize>,
    /// Every same-vid/pid device (it includes `by_port`); the iSerial decides.
    same_ids: Vec<usize>,
}

/// Narrow `(vendor_id, product_id, port_numbers)` entries to the ones worth opening for the
/// resolved candidate. Pure. Only same-vid/pid entries are ever returned. A port chain
/// equal to the candidate's gives `by_port` only when exactly one entry has it and it is
/// not empty (an empty chain identifies nothing).
fn narrow_rusb_devices(
    devices: &[(u16, u16, Vec<u8>)],
    vendor_id: u16,
    product_id: u16,
    port_chain: &[u8],
) -> RusbNarrowing {
    let same_ids: Vec<usize> = devices
        .iter()
        .enumerate()
        .filter(|(_, (vid, pid, _))| *vid == vendor_id && *pid == product_id)
        .map(|(index, _)| index)
        .collect();
    let mut by_port = None;
    if !port_chain.is_empty() {
        let mut on_port = same_ids
            .iter()
            .copied()
            .filter(|&index| devices[index].2 == port_chain);
        if let (Some(only), None) = (on_port.next(), on_port.next()) {
            by_port = Some(only);
        }
    }
    RusbNarrowing { by_port, same_ids }
}

fn has_adb_interface(
    device: &rusb::Device<rusb::Context>,
    descriptor: &rusb::DeviceDescriptor,
) -> bool {
    (0..descriptor.num_configurations()).any(|n| {
        device.config_descriptor(n).is_ok_and(|config| {
            config.interfaces().any(|interface| {
                interface.descriptors().any(|alt| {
                    alt.class_code() == 0xff
                        && alt.sub_class_code() == 0x42
                        && alt.protocol_code() == 0x01
                })
            })
        })
    })
}

/// The ADB-interface device (class ff, subclass 42, protocol 01) whose USB iSerial equals
/// `serial`, among the rusb devices that can be the resolved `candidate`. Only handles of
/// the narrowed devices are opened (to read the string); no interface is claimed. Not
/// finding it is `Unidentified`.
fn find_adb_device(
    serial: &str,
    candidate: &UsbCandidate,
) -> std::result::Result<rusb::Device<rusb::Context>, ConnectFailure> {
    let unidentified = |what: &str, e: rusb::Error| {
        ConnectFailure::Unidentified(format!("libusb {what} failed: {e}"))
    };
    let context = rusb::Context::new().map_err(|e| unidentified("context", e))?;
    let list = context
        .devices()
        .map_err(|e| unidentified("device list", e))?;
    let mut adb = Vec::new();
    for device in list.iter() {
        let Ok(descriptor) = device.device_descriptor() else {
            continue;
        };
        if !has_adb_interface(&device, &descriptor) {
            continue;
        }
        let ports = device.port_numbers().unwrap_or_default();
        adb.push((device, descriptor, ports));
    }
    let ids: Vec<(u16, u16, Vec<u8>)> = adb
        .iter()
        .map(|(_, descriptor, ports)| {
            (
                descriptor.vendor_id(),
                descriptor.product_id(),
                ports.clone(),
            )
        })
        .collect();
    let narrowing = narrow_rusb_devices(
        &ids,
        candidate.vendor_id,
        candidate.product_id,
        &candidate.location.port_chain,
    );
    let read_serial = |index: usize| -> Option<String> {
        let (device, descriptor, _) = &adb[index];
        device
            .open()
            .ok()
            .and_then(|handle| handle.read_serial_number_string_ascii(descriptor).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    // The device on the candidate's port chain, when its iSerial agrees, is the one.
    let mut known: Option<(usize, Option<String>)> = None;
    if let Some(index) = narrowing.by_port {
        let device_serial = read_serial(index);
        if !serial.trim().is_empty() && device_serial.as_deref() == Some(serial.trim()) {
            return Ok(adb.swap_remove(index).0);
        }
        known = Some((index, device_serial));
    }
    // No port match, or its iSerial differs (rusb and nusb may number ports differently):
    // every same-vid/pid device, with the iSerial still required to match.
    tracing::debug!(
        port_chain = ?candidate.location.port_chain,
        same_ids = narrowing.same_ids.len(),
        port_match_rejected = known.is_some(),
        "ADB port did not identify the device; reading iSerial of same-vid/pid devices only"
    );
    let with_serial: Vec<(usize, Option<String>)> = narrowing
        .same_ids
        .iter()
        .map(|&index| match &known {
            Some((known_index, device_serial)) if *known_index == index => {
                (index, device_serial.clone())
            }
            _ => (index, read_serial(index)),
        })
        .collect();
    match pick_adb_device(with_serial, serial).map_err(ConnectFailure::Error)? {
        Some(index) => Ok(adb.swap_remove(index).0),
        None => Err(ConnectFailure::Unidentified(format!(
            "no ADB device with USB serial {serial} could be identified among the libusb devices"
        ))),
    }
}

/// Open the ADB device with USB iSerial `serial`. Identification failures are
/// `Unidentified` (the caller retries); two matches are `AmbiguousTarget`; only the auth
/// handshake on the identified device is a plain `Client` error.
fn open_adb_by_serial(
    serial: &str,
    candidate: &UsbCandidate,
    key_path: &Path,
) -> std::result::Result<ADBUSBDevice, ConnectFailure> {
    let device = find_adb_device(serial, candidate)?;
    ADBUSBDevice::new_from_transport(
        USBTransport::new_from_device(device),
        Some(key_path.to_path_buf()),
    )
    .map_err(|e| {
        if is_usb_device_unavailable(&e) {
            ConnectFailure::Unidentified(format!("the chosen ADB device could not be opened: {e}"))
        } else {
            AdbError::Client(e.to_string()).into()
        }
    })
}

/// Whether a failed transport open says the device vanished or cannot be accessed (libusb
/// NotFound, NoDevice, Access), as opposed to a failed handshake. Only these are
/// `Unidentified`: with a target set the device is still being identified, so the caller
/// retries. Timeout and every auth-related error are not: a pending auth prompt can surface
/// as a USB Timeout.
fn is_usb_device_unavailable(err: &adb_client::RustADBError) -> bool {
    matches!(
        err,
        adb_client::RustADBError::UsbError(
            rusb::Error::NotFound | rusb::Error::NoDevice | rusb::Error::Access
        )
    )
}

/// The last unidentified reason that was logged at warn, so a poll that fails the same way
/// every second logs it once.
static LAST_UNIDENTIFIED_WARNING: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Log `reason` at warn unless it is the one logged last; `None` re-arms the warning.
fn warn_unidentified_once(reason: Option<&str>) {
    let mut last = LAST_UNIDENTIFIED_WARNING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match reason {
        Some(reason) if last.as_deref() != Some(reason) => {
            tracing::warn!(%reason, "chosen ADB device not identifiable yet");
            *last = Some(reason.to_owned());
        }
        Some(_) => {}
        None => *last = None,
    }
}

/// State token for a device that could not be identified. An adb server holding the
/// interface explains the unreadable iSerial; otherwise it is a selection error, never
/// "unauthorized", because no auth was attempted.
fn unidentified_state(reason: String, adb_server_running: bool) -> Result<Option<&'static str>> {
    if adb_server_running {
        Ok(Some("adb_server_blocking"))
    } else {
        Err(AdbError::Selection(reason))
    }
}

/// Map a failed `connect_device` to the state token `check_device_state`
/// reports. The selection errors are their own errors, never a token: a token
/// would send the UI to "unauthorized" or "kill adb server".
fn connect_failure_state(err: &AdbError, adb_server_running: bool) -> Result<Option<&'static str>> {
    match err {
        AdbError::MultipleDevices => Err(AdbError::MultipleDevices),
        AdbError::AmbiguousTarget => Err(AdbError::AmbiguousTarget),
        AdbError::TargetMismatch { expected, found } => Err(AdbError::TargetMismatch {
            expected: expected.clone(),
            found: found.clone(),
        }),
        AdbError::Selection(reason) => Err(AdbError::Selection(reason.clone())),
        // An external adb server holds the interface; a distinct state lets
        // the UI offer "kill server" instead.
        _ if adb_server_running => Ok(Some("adb_server_blocking")),
        _ => Ok(Some("unauthorized")),
    }
}

/// Upper bound on `wait_for_device`. Re-enumeration after a reboot can take
/// well over a minute on a cold boot; a shorter bound fails working devices.
const WAIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// libusb interface-claim retries. Two `AdbManager`s racing for the endpoint
/// (a Dashboard poll still running when the user clicks Reboot) get
/// `LIBUSB_ERROR_BUSY` until the first release propagates. 10 × 150 ms
/// outlasts a polling cycle (under 1 s in practice).
const CONNECT_RETRY_ATTEMPTS: u32 = 10;
const CONNECT_RETRY_BACKOFF: std::time::Duration = std::time::Duration::from_millis(150);

pub struct AdbManager {
    serial: Option<String>,
    /// Cached USB handle, so the RSA auth handshake runs once. Dropped on any
    /// I/O error so a replug recovers.
    device: Option<ADBUSBDevice>,
    /// State derived from `getprop ro.bootmode` on the first state probe
    /// (`"device"`, `"recovery"` or `"sideload"`). Cleared with `device`.
    cached_bootmode: Option<&'static str>,
    /// The target serial `device` was opened for (`None`: opened by autodetect
    /// with no session target). A change of target drops the cached handle.
    opened_for: Option<String>,
}

impl AdbManager {
    pub fn new() -> Self {
        Self {
            serial: None,
            device: None,
            cached_bootmode: None,
            opened_for: None,
        }
    }

    /// `Some(adb)` when a fully-authorized device is reachable; `None`
    /// otherwise, including when the probe errored.
    pub fn new_if_connected() -> Option<Self> {
        let mut adb = Self::new();
        if adb.check_device().unwrap_or(false) {
            Some(adb)
        } else {
            None
        }
    }

    /// Last-known serial captured by `check_device` /
    /// `check_device_state` / `wait_for_device`. `None` until the first
    /// successful probe; never cleared by `AdbManager`.
    pub fn serial(&self) -> Option<&str> {
        self.serial.as_deref()
    }

    /// Resolve LTBox's ADB private key, generating a fresh 2048-bit PKCS8 PEM
    /// (what adbd requires) if the file is missing or corrupt.
    fn ensure_key_path() -> Result<PathBuf> {
        ensure_key_at_path(ltbox_core::app_paths::adb_key_path())
    }

    /// Open (or reuse) the cached `ADBUSBDevice`. A fresh open sets
    /// `self.serial` to the target serial when opened by serial, else from
    /// `getprop ro.serialno`, since VID:PID cannot tell two units of the same
    /// model apart.
    fn connect_device(&mut self) -> Result<&mut ADBUSBDevice> {
        if !self.try_connect().map_err(ConnectFailure::into_error)? {
            return Err(AdbError::Client(
                "the chosen device is not attached in ADB mode".into(),
            ));
        }
        Ok(self.device.as_mut().expect("device just connected"))
    }

    /// `Ok(true)`: `self.device` holds the device to use. `Ok(false)`: a
    /// session target is set and it is not in ADB (the cache is dropped, and
    /// other ADB devices are never opened).
    fn try_connect(&mut self) -> std::result::Result<bool, ConnectFailure> {
        let Some(pick) = pick_target()? else {
            self.drop_device();
            return Ok(false);
        };
        let wanted = match &pick {
            AdbPick::Any => None,
            AdbPick::Serial { serial, .. } => Some(serial.clone()),
        };
        if self.device.is_some() && self.opened_for != wanted {
            self.drop_device();
        }
        if self.device.is_none() {
            let key_path = Self::ensure_key_path()?;
            // See `CONNECT_RETRY_ATTEMPTS`. Re-enumerating per attempt also
            // picks up a replug between attempts.
            let mut last_err: Option<String> = None;
            let mut last_unidentified = false;
            for attempt in 0..CONNECT_RETRY_ATTEMPTS {
                let opened = match &pick {
                    AdbPick::Any => {
                        ADBUSBDevice::autodetect_with_custom_private_key(key_path.clone())
                            .map_err(|e| ConnectFailure::Error(AdbError::Client(e.to_string())))
                    }
                    AdbPick::Serial { serial, candidate } => {
                        open_adb_by_serial(serial, candidate, &key_path)
                    }
                };
                match opened {
                    Ok(mut dev) => {
                        // Not `shell_inner`: its failure path drops the
                        // device, and a recovery adbd that authenticates but
                        // refuses shell would lose the handle just opened.
                        let mut stdout = Vec::new();
                        let _ = dev.shell_command(
                            &"getprop ro.serialno",
                            Some(&mut stdout as &mut dyn std::io::Write),
                            None,
                        );
                        let serial = String::from_utf8_lossy(&stdout).trim().to_string();
                        if let Some(expected) = wanted.as_deref()
                            && !serial.is_empty()
                            && serial != expected
                        {
                            // Not the chosen device: release it unused.
                            drop(dev);
                            return Err(AdbError::TargetMismatch {
                                expected: expected.to_string(),
                                found: serial,
                            }
                            .into());
                        }
                        // Opened by serial: the device is the target, so the stored
                        // serial is the target's even when ro.serialno was unreadable.
                        if let Some(expected) = wanted.as_deref() {
                            self.serial = Some(expected.to_string());
                        } else if !serial.is_empty() {
                            self.serial = Some(serial);
                        }
                        self.cached_bootmode = None;
                        self.opened_for = wanted;
                        self.device = Some(dev);
                        return Ok(true);
                    }
                    Err(ConnectFailure::Unidentified(msg)) => {
                        last_err = Some(msg);
                        last_unidentified = true;
                        if attempt + 1 < CONNECT_RETRY_ATTEMPTS {
                            std::thread::sleep(CONNECT_RETRY_BACKOFF);
                        }
                    }
                    Err(ConnectFailure::Error(AdbError::Client(msg))) => {
                        last_err = Some(msg);
                        last_unidentified = false;
                        if attempt + 1 < CONNECT_RETRY_ATTEMPTS {
                            std::thread::sleep(CONNECT_RETRY_BACKOFF);
                        }
                    }
                    Err(other) => return Err(other),
                }
            }
            let reason = last_err.unwrap_or_else(|| "ADB USB open failed".into());
            return Err(if last_unidentified {
                ConnectFailure::Unidentified(reason)
            } else {
                AdbError::Client(reason).into()
            });
        }
        Ok(true)
    }

    /// Drop the cached handle so the next probe re-authenticates against a
    /// freshly enumerated device (replug, adbd restart).
    fn drop_device(&mut self) {
        self.device = None;
        self.cached_bootmode = None;
        self.opened_for = None;
    }

    /// Probe for a *fully-authorized* ADB device; updates stored serial.
    ///
    /// `true` only when an Android USB device is visible and the auth
    /// handshake succeeds; the boot mode is not checked.
    pub fn check_device(&mut self) -> Result<bool> {
        // Cheap presence probe before an auth attempt. With a session target
        // `try_connect` decides presence itself: other ADB devices are ignored.
        if crate::selection::current_target().is_none() {
            let infos =
                find_all_connected_adb_devices().map_err(|e| AdbError::Client(e.to_string()))?;
            if infos.is_empty() {
                self.drop_device();
                return Ok(false);
            }
        }
        match self.try_connect() {
            Ok(present) => {
                if present {
                    warn_unidentified_once(None);
                }
                Ok(present)
            }
            Err(ConnectFailure::Unidentified(reason)) => {
                self.drop_device();
                // Not ready, so `false`: `wait_for_device_or` keeps polling through a
                // re-enumeration and ends in `Timeout`. No other device is opened.
                warn_unidentified_once(Some(&reason));
                Ok(false)
            }
            Err(ConnectFailure::Error(err)) => {
                self.drop_device();
                if is_selection_error(&err) {
                    return Err(err);
                }
                Ok(false)
            }
        }
    }

    /// Like `check_device` but returns the raw state token
    /// (`"device"`, `"recovery"`, `"sideload"`, `"unauthorized"`,
    /// `"adb_server_blocking"`) so callers can pattern-match without
    /// importing `adb_client::DeviceState`.
    ///
    /// | Probe outcome | Returned |
    /// |---------------|----------|
    /// | libusb enumeration empty | `Ok(None)` |
    /// | Device visible + auth ok + `ro.bootmode == "recovery"` | `Ok(Some("recovery"))` |
    /// | Device visible + auth ok + any other bootmode | `Ok(Some("device"))` |
    /// | Device visible + auth ok + the expected `CLSE` shell refusal | `Ok(Some("sideload"))` |
    /// | Device visible + auth fails AND a process holds `127.0.0.1:5037` | `Ok(Some("adb_server_blocking"))` |
    /// | Device visible + auth fails AND no adb server present | `Ok(Some("unauthorized"))` |
    /// | Session target set, device cannot be identified before auth (resolve/libusb failure, not among the libusb devices, iSerial unreadable) AND a process holds `127.0.0.1:5037` | `Ok(Some("adb_server_blocking"))` |
    /// | Same, no adb server present | `Err(AdbError::Selection)` |
    /// | More than one Android/EDL USB device attached, no session target | `Err(AdbError::MultipleDevices)` |
    /// | Session target set, not attached in ADB (other ADB devices ignored) | `Ok(None)` |
    /// | Two ADB devices carry the target serial | `Err(AdbError::AmbiguousTarget)` |
    /// | Opened device's `ro.serialno` differs from the target | `Err(AdbError::TargetMismatch)` |
    ///
    /// `adb_client` discards the connect banner, so sideload is inferred from
    /// minadbd's exact `CLSE` shell refusal; rescue mode answers the same way
    /// and may be reported as sideload. Other shell errors are returned, so a
    /// transport failure cannot pass as sideload.
    pub fn check_device_state(&mut self) -> Result<Option<&'static str>> {
        if crate::selection::current_target().is_none() {
            let infos =
                find_all_connected_adb_devices().map_err(|e| AdbError::Client(e.to_string()))?;
            if infos.is_empty() {
                self.drop_device();
                return Ok(None);
            }
        }
        match self.try_connect() {
            Ok(false) => Ok(None),
            Ok(true) => {
                // Probe bootmode once per connection, not every poll.
                if self.cached_bootmode.is_none() {
                    self.cached_bootmode = Some(match self.shell_inner("getprop ro.bootmode") {
                        Ok(mode) if mode.trim() == "recovery" => "recovery",
                        Ok(_) => "device",
                        // Rescue mode is a known false positive (see doc).
                        Err(err) if is_sideload_shell_refusal(&err) => "sideload",
                        Err(err) => return Err(err),
                    });
                }
                Ok(self.cached_bootmode)
            }
            Err(ConnectFailure::Unidentified(reason)) => {
                self.drop_device();
                unidentified_state(reason, adb_server_running())
            }
            Err(ConnectFailure::Error(err)) => {
                self.drop_device();
                // Skip the TCP probe when the answer is already an error.
                let server_running = !is_selection_error(&err) && adb_server_running();
                connect_failure_state(&err, server_running)
            }
        }
    }

    /// Wait up to [`WAIT_TIMEOUT`] for a fully-authorized ADB device.
    ///
    /// Returns `AdbError::Timeout` on expiry instead of spinning forever.
    pub fn wait_for_device(&mut self) -> Result<()> {
        self.wait_for_device_or(|| false).map(|_| ())
    }

    /// Like [`wait_for_device`](Self::wait_for_device), but returns `Ok(false)`
    /// as soon as `bail` is true (e.g. the device went straight to EDL).
    /// `Ok(true)` = ADB ready; `Err(Timeout)` = neither in time. `bail` runs
    /// before each probe so it wins over a transient probe error.
    pub fn wait_for_device_or<F: FnMut() -> bool>(&mut self, mut bail: F) -> Result<bool> {
        let deadline = std::time::Instant::now() + WAIT_TIMEOUT;
        loop {
            if bail() {
                return Ok(false);
            }
            if self.check_device()? {
                return Ok(true);
            }
            if std::time::Instant::now() >= deadline {
                return Err(AdbError::Timeout);
            }
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }

    /// Run shell command; returns trimmed stdout.
    pub fn shell(&mut self, cmd: &str) -> Result<String> {
        self.shell_inner(cmd)
    }

    fn shell_inner(&mut self, cmd: &str) -> Result<String> {
        let dev = self.connect_device()?;
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let res = dev.shell_command(
            &cmd,
            Some(&mut stdout as &mut dyn std::io::Write),
            Some(&mut stderr as &mut dyn std::io::Write),
        );
        match res {
            Ok(_) => Ok(String::from_utf8_lossy(&stdout).trim().to_string()),
            Err(e) => {
                // Otherwise one broken pipe makes every later call fail.
                self.drop_device();
                Err(AdbError::CommandFailed(e.to_string()))
            }
        }
    }

    pub fn get_model(&mut self) -> Result<Option<String>> {
        match self.shell_inner("getprop ro.product.model") {
            Ok(m) if !m.is_empty() => Ok(Some(m)),
            _ => Ok(None),
        }
    }

    /// Active slot suffix — only `"_a"` or `"_b"`, else `None`. A garbage prop
    /// would otherwise build bogus partition names like `vendor_bootxyz`.
    pub fn get_slot_suffix(&mut self) -> Result<Option<String>> {
        match self.shell_inner("getprop ro.boot.slot_suffix") {
            Ok(s) if s == "_a" || s == "_b" => Ok(Some(s)),
            _ => Ok(None),
        }
    }

    /// The whole kernel release token from `/proc/version`, including the
    /// `-androidNN-...` GKI branch: LKM consumers need it (5.10 ships under two
    /// branches). Callers normalize to `major.minor` themselves.
    pub fn get_kernel_version(&mut self) -> Result<Option<String>> {
        const PREFIX: &str = "Linux version ";
        match self.shell_inner("cat /proc/version") {
            Ok(v) => {
                let Some(start) = v.find(PREFIX) else {
                    return Ok(None);
                };
                let Some(rest) = v[start..].strip_prefix(PREFIX) else {
                    return Ok(None);
                };
                let ver: String = rest.chars().take_while(|c| !c.is_whitespace()).collect();
                if !ver.is_empty() {
                    Ok(Some(ver))
                } else {
                    Ok(None)
                }
            }
            Err(_) => Ok(None),
        }
    }

    pub fn reboot(&mut self, target: &str) -> Result<()> {
        let reboot_type = match target {
            "bootloader" => RebootType::Bootloader,
            "recovery" => RebootType::Recovery,
            "sideload" => RebootType::Sideload,
            "fastboot" => RebootType::Fastboot,
            _ => RebootType::System,
        };
        // RebootType has no EDL variant, so use the shell. adbd dies as the
        // reboot fires, so a pipe error here means success.
        if target == "edl" {
            let res = self.shell_inner("reboot edl");
            self.drop_device();
            return match res {
                Ok(_) => Ok(()),
                Err(AdbError::CommandFailed(msg)) if is_adbd_dropped_after_reboot(&msg) => Ok(()),
                Err(e) => Err(e),
            };
        }
        let dev = self.connect_device()?;
        let res = dev
            .reboot(reboot_type)
            .map_err(|e| AdbError::CommandFailed(e.to_string()));
        self.drop_device();
        // adbd tears down USB as the reboot fires, so a pipe / EOF error
        // after the command is spurious.
        match res {
            Ok(()) => Ok(()),
            Err(AdbError::CommandFailed(msg)) if is_adbd_dropped_after_reboot(&msg) => Ok(()),
            Err(e) => Err(e),
        }
    }

    pub fn install(&mut self, apk_path: &str) -> Result<()> {
        let path = Path::new(apk_path);
        let remote = format!("/data/local/tmp/ltbox-install-{}.apk", std::process::id());

        let push_res = {
            let mut apk_file = std::fs::File::open(path)
                .map_err(|e| AdbError::CommandFailed(format!("open {}: {e}", path.display())))?;
            let dev = self.connect_device()?;
            // /data/local/tmp is shell-writable on stock Android; hardened
            // SELinux test images may still reject this staging path.
            dev.push(&mut apk_file, &remote)
                .map_err(|e| AdbError::CommandFailed(e.to_string()))
        };
        if let Err(e) = push_res {
            // A failed push leaves the handle half-drained; reclaim next time.
            self.drop_device();
            return Err(e);
        }

        let output = self.shell_inner(&format!("pm install -r '{remote}'; rm -f '{remote}'"))?;
        if output.contains("Success") {
            Ok(())
        } else if output.is_empty() {
            Err(AdbError::CommandFailed(
                "pm install failed with empty output".to_string(),
            ))
        } else {
            Err(AdbError::CommandFailed(output))
        }
    }

    /// Copy a local file to an on-device path (e.g. `/sdcard/manager.apk`)
    /// without `pm install`, so a user can install it by hand.
    pub fn push_file(&mut self, local: &Path, remote: &str) -> Result<()> {
        let mut file = std::fs::File::open(local)
            .map_err(|e| AdbError::CommandFailed(format!("open {}: {e}", local.display())))?;
        let dev = self.connect_device()?;
        if let Err(e) = dev.push(&mut file, &remote) {
            // Mirror `install`: a failed push can leave the USB handle
            // half-drained, so drop it for a clean re-claim next call.
            self.drop_device();
            return Err(AdbError::CommandFailed(e.to_string()));
        }
        Ok(())
    }
}

/// Ensure `path` holds a usable PKCS8 PEM RSA private key.
///
/// Missing or corrupt material is replaced atomically via a sibling
/// temp file (`create_new` + rename) so concurrent writers cannot leave
/// a half-written key. On Unix the final file is mode `0600`.
fn ensure_key_at_path(path: PathBuf) -> Result<PathBuf> {
    if path.exists() {
        match validate_key_file(&path) {
            Ok(()) => {
                harden_key_permissions(&path)?;
                return Ok(path);
            }
            Err(err) => {
                // Corrupt material would surface as an opaque USB-auth
                // failure, so regenerate.
                tracing::warn!(
                    path = %path.display(),
                    error = %err,
                    "ADB private key unusable; regenerating"
                );
                remove_corrupt_key(&path)?;
            }
        }
    }

    let parent = path
        .parent()
        .ok_or_else(|| AdbError::Key(format!("adb key path has no parent: {}", path.display())))?;
    std::fs::create_dir_all(parent)
        .map_err(|e| AdbError::Key(format!("create_dir_all {}: {e}", parent.display())))?;

    let private_key = rsa::RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 2048)
        .map_err(|e| AdbError::Key(format!("RSA keygen: {e}")))?;
    let pem = private_key
        .to_pkcs8_pem(LineEnding::LF)
        .map_err(|e| AdbError::Key(format!("PKCS8 encode: {e}")))?;
    write_key_atomic(&path, pem.as_bytes())?;
    Ok(path)
}

fn validate_key_file(path: &Path) -> Result<()> {
    let pem = std::fs::read_to_string(path)
        .map_err(|e| AdbError::Key(format!("read {}: {e}", path.display())))?;
    rsa::RsaPrivateKey::from_pkcs8_pem(&pem)
        .map_err(|e| AdbError::Key(format!("decode {}: {e}", path.display())))?;
    Ok(())
}

fn harden_key_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| AdbError::Key(format!("chmod 0600 {}: {e}", path.display())))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn remove_corrupt_key(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(AdbError::Key(format!(
            "remove corrupt adb key {}: {e}",
            path.display()
        ))),
    }
}

fn write_key_atomic(path: &Path, pem: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| AdbError::Key(format!("adb key path has no parent: {}", path.display())))?;
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("adbkey");

    // A random, exclusively created sibling (0600 on Unix) keeps the rename on
    // one filesystem. Dropping it on any error path removes only the temp file.
    let mut tmp = tempfile::Builder::new()
        .prefix(&format!(".{file_name}.tmp-"))
        .tempfile_in(parent)
        .map_err(|e| AdbError::Key(format!("create temp key in {}: {e}", parent.display())))?;
    tmp.write_all(pem)
        .map_err(|e| AdbError::Key(format!("write temp key {}: {e}", tmp.path().display())))?;
    tmp.as_file()
        .sync_all()
        .map_err(|e| AdbError::Key(format!("sync temp key {}: {e}", tmp.path().display())))?;

    // Another process may have published a valid key while we generated
    // ours. Prefer the already-published key over clobbering it.
    if path.exists() {
        if validate_key_file(path).is_ok() {
            drop(tmp);
            harden_key_permissions(path)?;
            return Ok(());
        }
        // Still corrupt / empty — fall through and replace.
        remove_corrupt_key(path)?;
    }

    let tmp_display = tmp.path().display().to_string();
    tmp.persist(path).map_err(|e| {
        AdbError::Key(format!(
            "rename temp key {tmp_display} -> {}: {}",
            path.display(),
            e.error
        ))
    })?;

    harden_key_permissions(path)?;

    Ok(())
}

const ADB_SHELL_REFUSED: &str =
    "ADB request failed - Open session failed: got CLSE in response instead of OKAY";

/// Match only the `adb_client` response for an unsupported `shell:` service.
/// Transport errors are also wrapped in `CommandFailed` by `shell_inner`, so
/// broad matching here would mislabel a disconnected device as sideload.
fn is_sideload_shell_refusal(error: &AdbError) -> bool {
    matches!(error, AdbError::CommandFailed(message) if message == ADB_SHELL_REFUSED)
}

/// Heuristic: did adbd disconnect mid-call because the reboot fired? Matches
/// `adb_client` / `rusb` wording for pipe, I/O, EOF and no-device errors.
/// Kept narrow: "not found" would also swallow a shell `command not found`.
fn is_adbd_dropped_after_reboot(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    // Bare "pipe" would also hit "pipeline" etc.; teardown says "broken pipe"
    // or rusb's "pipe error".
    lower.contains("broken pipe")
        || lower.contains("pipe error")
        || lower.contains("no device")
        || lower.contains("no such device")
        || lower.contains("device disconnected")
        || lower.contains("unexpected eof")
        || lower.contains("end of file")
        // rusb's `LIBUSB_ERROR_IO` ("Input/Output Error" or "I/O error"),
        // seen on Linux (LTBox v3.0.8) when adbd tears down USB early.
        || lower.contains("input/output error")
        || lower.contains("i/o error")
}

impl Default for AdbManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AdbError, RusbNarrowing, connect_failure_state, ensure_key_at_path,
        is_adbd_dropped_after_reboot, is_sideload_shell_refusal, map_selection_error,
        narrow_rusb_devices, pick_adb_device, remove_corrupt_key, unidentified_state,
        validate_key_file, write_key_atomic,
    };
    use rsa::pkcs8::{EncodePrivateKey, LineEnding};
    use std::path::PathBuf;

    #[test]
    fn unidentifiable_target_is_an_error_not_an_auth_state() {
        use crate::selection::UsbSelectionError;
        for error in [
            UsbSelectionError::UnprovenSuccessor("two new EDL devices".into()),
            UsbSelectionError::StatePoisoned,
            UsbSelectionError::UnmappedEdlPort,
        ] {
            let mapped = map_selection_error(error, true);
            assert!(matches!(mapped, AdbError::Selection(_)), "{mapped:?}");
            assert!(matches!(
                connect_failure_state(&mapped, true),
                Err(AdbError::Selection(_))
            ));
        }
        // With a target an enumeration failure means the chosen device cannot be
        // identified; without one it stays a plain client error.
        assert!(matches!(
            map_selection_error(UsbSelectionError::Enumeration("busy".into()), true),
            AdbError::Selection(_)
        ));
        assert!(matches!(
            map_selection_error(UsbSelectionError::Enumeration("busy".into()), false),
            AdbError::Client(_)
        ));
    }

    #[test]
    fn unidentified_device_is_blocking_or_an_error_never_unauthorized() {
        assert!(matches!(
            unidentified_state("no such device".into(), true),
            Ok(Some("adb_server_blocking"))
        ));
        assert!(matches!(
            unidentified_state("no such device".into(), false),
            Err(AdbError::Selection(reason)) if reason == "no such device"
        ));
    }

    fn rusb_devs(list: &[(u16, u16, &[u8])]) -> Vec<(u16, u16, Vec<u8>)> {
        list.iter()
            .map(|&(vid, pid, ports)| (vid, pid, ports.to_vec()))
            .collect()
    }

    #[test]
    fn narrowing_picks_the_one_same_ids_device_on_the_port_chain() {
        let devices = rusb_devs(&[
            (0x18d1, 0x4ee2, &[1, 2]),
            (0x18d1, 0x4ee2, &[1, 3]),
            (0x18d1, 0x4ee0, &[1, 3]),
        ]);
        assert_eq!(
            narrow_rusb_devices(&devices, 0x18d1, 0x4ee2, &[1, 3]),
            RusbNarrowing {
                by_port: Some(1),
                // The port match is read first; the rest is the fallback when its iSerial differs.
                same_ids: vec![0, 1],
            }
        );
    }

    #[test]
    fn narrowing_falls_back_to_same_ids_only_when_ports_differ() {
        let devices = rusb_devs(&[
            (0x18d1, 0x4ee2, &[1, 2]),
            (0x18d1, 0x4ee2, &[1, 3]),
            (0x2717, 0xff48, &[1, 4]),
        ]);
        // Port numbering parity failed: every same-vid/pid device, and nothing else.
        assert_eq!(
            narrow_rusb_devices(&devices, 0x18d1, 0x4ee2, &[7]),
            RusbNarrowing {
                by_port: None,
                same_ids: vec![0, 1],
            }
        );
        // A port chain held only by a device with other ids is never a pick.
        assert_eq!(
            narrow_rusb_devices(&devices, 0x18d1, 0x4ee2, &[1, 4]),
            RusbNarrowing {
                by_port: None,
                same_ids: vec![0, 1],
            }
        );
    }

    #[test]
    fn narrowing_never_picks_by_an_ambiguous_or_empty_port_chain() {
        // Two same-id devices with the same ports (different buses): not exactly one.
        let devices = rusb_devs(&[(1, 2, &[1, 3]), (1, 2, &[1, 3]), (1, 2, &[2])]);
        assert_eq!(
            narrow_rusb_devices(&devices, 1, 2, &[1, 3]),
            RusbNarrowing {
                by_port: None,
                same_ids: vec![0, 1, 2],
            }
        );
        // An empty chain identifies nothing, even when a root-level entry has none either.
        let devices = rusb_devs(&[(1, 2, &[]), (1, 2, &[5])]);
        assert_eq!(
            narrow_rusb_devices(&devices, 1, 2, &[]),
            RusbNarrowing {
                by_port: None,
                same_ids: vec![0, 1],
            }
        );
    }

    #[test]
    fn narrowing_with_no_same_ids_device_opens_nothing() {
        let devices = rusb_devs(&[(0x2717, 0xff48, &[1, 3])]);
        assert_eq!(
            narrow_rusb_devices(&devices, 0x18d1, 0x4ee2, &[1, 3]),
            RusbNarrowing {
                by_port: None,
                same_ids: vec![],
            }
        );
        assert_eq!(
            narrow_rusb_devices(&[], 1, 2, &[1]),
            RusbNarrowing {
                by_port: None,
                same_ids: vec![],
            }
        );
    }

    #[test]
    fn only_vanished_or_inaccessible_usb_devices_are_unavailable() {
        use super::is_usb_device_unavailable;
        use adb_client::RustADBError;
        for gone in [
            rusb::Error::NotFound,
            rusb::Error::NoDevice,
            rusb::Error::Access,
        ] {
            assert!(is_usb_device_unavailable(&RustADBError::UsbError(gone)));
        }
        // A pending auth prompt can surface as a USB timeout; busy is the adb-server case.
        for kept in [
            rusb::Error::Timeout,
            rusb::Error::Busy,
            rusb::Error::Io,
            rusb::Error::Pipe,
        ] {
            assert!(!is_usb_device_unavailable(&RustADBError::UsbError(kept)));
        }
        assert!(!is_usb_device_unavailable(&RustADBError::DeviceBusy));
        assert!(!is_usb_device_unavailable(
            &RustADBError::USBNoDescriptorFound
        ));
    }

    #[test]
    fn connect_failure_state_keeps_multiple_devices_an_error() {
        assert!(matches!(
            connect_failure_state(&AdbError::MultipleDevices, false),
            Err(AdbError::MultipleDevices)
        ));
        assert!(matches!(
            connect_failure_state(&AdbError::MultipleDevices, true),
            Err(AdbError::MultipleDevices)
        ));
        let other = AdbError::Client("USB Error: Busy".into());
        assert!(matches!(
            connect_failure_state(&other, true),
            Ok(Some("adb_server_blocking"))
        ));
        assert!(matches!(
            connect_failure_state(&other, false),
            Ok(Some("unauthorized"))
        ));
    }

    #[test]
    fn connect_failure_state_keeps_target_errors_an_error() {
        assert!(matches!(
            connect_failure_state(&AdbError::AmbiguousTarget, true),
            Err(AdbError::AmbiguousTarget)
        ));
        let mismatch = AdbError::TargetMismatch {
            expected: "A".into(),
            found: "B".into(),
        };
        assert!(matches!(
            connect_failure_state(&mismatch, false),
            Err(AdbError::TargetMismatch { expected, found }) if expected == "A" && found == "B"
        ));
    }

    fn devs(serials: &[Option<&str>]) -> Vec<(usize, Option<String>)> {
        serials
            .iter()
            .enumerate()
            .map(|(i, s)| (i, s.map(str::to_string)))
            .collect()
    }

    #[test]
    fn pick_adb_device_takes_the_single_exact_match() {
        let list = devs(&[Some("AAA"), Some("BBB"), None]);
        assert!(matches!(pick_adb_device(list, "BBB"), Ok(Some(1))));
    }

    #[test]
    fn pick_adb_device_none_when_absent_or_empty() {
        assert!(matches!(
            pick_adb_device(devs(&[Some("AAA"), None]), "BBB"),
            Ok(None)
        ));
        assert!(matches!(pick_adb_device(devs(&[]), "BBB"), Ok(None)));
        // A device without a serial never matches an empty target.
        assert!(matches!(
            pick_adb_device(devs(&[None, Some("")]), ""),
            Ok(None)
        ));
    }

    #[test]
    fn pick_adb_device_refuses_two_matches() {
        assert!(matches!(
            pick_adb_device(devs(&[Some("AAA"), Some("AAA")]), "AAA"),
            Err(AdbError::AmbiguousTarget)
        ));
    }

    #[test]
    fn pick_adb_device_is_exact_not_case_folded() {
        assert!(matches!(
            pick_adb_device(devs(&[Some("abc")]), "ABC"),
            Ok(None)
        ));
        // Surrounding whitespace is trimmed on both sides.
        assert!(matches!(
            pick_adb_device(devs(&[Some(" abc ")]), " abc"),
            Ok(Some(0))
        ));
    }

    #[test]
    fn only_exact_shell_refusal_is_sideload() {
        let refusal = adb_client::RustADBError::ADBRequestFailed(
            "Open session failed: got CLSE in response instead of OKAY".into(),
        );
        assert!(is_sideload_shell_refusal(&AdbError::CommandFailed(
            refusal.to_string(),
        )));

        for error in [
            AdbError::CommandFailed("ADB error: USB Error: Timeout".into()),
            AdbError::CommandFailed("ADB error: USB Error: No device".into()),
            AdbError::CommandFailed(
                "ADB request failed - Open session failed: got FAIL in response instead of OKAY"
                    .into(),
            ),
            AdbError::Timeout,
        ] {
            assert!(!is_sideload_shell_refusal(&error));
        }
    }

    #[test]
    fn matches_libusb_pipe() {
        assert!(is_adbd_dropped_after_reboot(
            "USB Error: LIBUSB_ERROR_PIPE: pipe error"
        ));
        assert!(is_adbd_dropped_after_reboot("write failed: broken pipe"));
        // Bare "pipe" alone is too broad and must not match.
        assert!(!is_adbd_dropped_after_reboot("named pipe helper failed"));
        assert!(!is_adbd_dropped_after_reboot("pipeline aborted"));
    }

    #[test]
    fn matches_libusb_io() {
        // Real string from a v3.0.8 Linux user report. `LIBUSB_ERROR_IO`
        // stringifies as "Input/Output Error" in rusb.
        assert!(is_adbd_dropped_after_reboot(
            "Command failed: USB Error: Input/Output Error"
        ));
        assert!(is_adbd_dropped_after_reboot("rusb: i/o error"));
    }

    #[test]
    fn matches_no_device_and_eof() {
        assert!(is_adbd_dropped_after_reboot(
            "USB Error: LIBUSB_ERROR_NO_DEVICE: no device"
        ));
        // Observed on Linux after a successful reboot: rusb surfaces
        // LIBUSB_ERROR_NO_DEVICE as "No such device (...)", which does
        // not contain the contiguous substring "no device".
        assert!(is_adbd_dropped_after_reboot(
            "Command failed: USB Error: No such device (it may have been disconnected)."
        ));
        assert!(is_adbd_dropped_after_reboot("Unexpected EOF on socket"));
        assert!(is_adbd_dropped_after_reboot("end of file reached"));
    }

    #[test]
    fn rejects_unrelated_errors() {
        // A shell-echoed `command not found` must NOT pass as success.
        assert!(!is_adbd_dropped_after_reboot("command not found"));
        assert!(!is_adbd_dropped_after_reboot("Protocol error: 4 bytes"));
        assert!(!is_adbd_dropped_after_reboot(""));
    }

    fn temp_key_path(label: &str) -> PathBuf {
        // Tests remove the directory themselves, so detach it from the guard.
        let dir = tempfile::Builder::new()
            .prefix(&format!("ltbox-adbkey-test-dir-{label}-"))
            .tempdir()
            .expect("temp key dir")
            .keep();
        dir.join("adbkey")
    }

    #[test]
    fn ensure_key_creates_valid_private_key() {
        let path = temp_key_path("create");
        let _ = std::fs::remove_file(&path);
        let got = ensure_key_at_path(path.clone()).expect("create key");
        assert_eq!(got, path);
        validate_key_file(&path).expect("created key must decode");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "owner-only adb key");
        }
        // Idempotent: second call keeps the same valid material.
        let pem_before = std::fs::read_to_string(&path).unwrap();
        ensure_key_at_path(path.clone()).expect("reuse key");
        let pem_after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(pem_before, pem_after);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn ensure_key_recovers_from_corrupt_material() {
        let path = temp_key_path("corrupt");
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, b"not-a-pkcs8-key").expect("seed corrupt key");
        ensure_key_at_path(path.clone()).expect("recover key");
        validate_key_file(&path).expect("recovered key must decode");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn corrupt_key_removal_tolerates_concurrent_remover() {
        let path = temp_key_path("concurrent-remove");
        remove_corrupt_key(&path).expect("already-removed key must be tolerated");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn ensure_key_hardens_existing_valid_key_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let path = temp_key_path("permissions");
        ensure_key_at_path(path.clone()).expect("create key");
        let pem_before = std::fs::read(&path).expect("read key");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
            .expect("make key permissive");

        ensure_key_at_path(path.clone()).expect("harden existing key");

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "existing valid key must be owner-only");
        assert_eq!(std::fs::read(&path).unwrap(), pem_before);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn write_key_atomic_does_not_leave_partial_on_success() {
        let path = temp_key_path("atomic");
        let parent = path.parent().unwrap().to_path_buf();
        let _ = std::fs::remove_file(&path);
        let private_key = rsa::RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 2048).unwrap();
        let pem = private_key.to_pkcs8_pem(LineEnding::LF).unwrap();
        write_key_atomic(&path, pem.as_bytes()).expect("atomic write");
        assert!(path.is_file());
        let leftovers: Vec<_> = std::fs::read_dir(&parent)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains("adbkey") && n.contains(".tmp-"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "temp siblings must be cleaned: {leftovers:?}"
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
