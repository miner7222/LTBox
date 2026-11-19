//! ADB client via `adb_client` crate's `usb` feature — direct libusb
//! transport, no background `adb.exe` daemon on `localhost:5037`.

use adb_client::usb::{ADBUSBDevice, find_all_connected_adb_devices};
use adb_client::{ADBDeviceExt, RebootType};
use rsa::pkcs8::{DecodePrivateKey, EncodePrivateKey, LineEnding};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;
use thiserror::Error;

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
    /// Failed to read or write the LTBox-owned ADB RSA private key.
    #[error("Failed to prepare ADB key: {0}")]
    Key(String),
}

type Result<T> = std::result::Result<T, AdbError>;

/// Map a failed `connect_device` to the state token `check_device_state`
/// reports. The multi-device guard is its own error, never a token: a token
/// would send the UI to "unauthorized" or "kill adb server".
fn connect_failure_state(err: &AdbError, adb_server_running: bool) -> Result<Option<&'static str>> {
    match err {
        AdbError::MultipleDevices => Err(AdbError::MultipleDevices),
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
}

impl AdbManager {
    pub fn new() -> Self {
        Self {
            serial: None,
            device: None,
            cached_bootmode: None,
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
    /// `self.serial` from `getprop ro.serialno`, since VID:PID cannot tell
    /// two units of the same model apart.
    fn connect_device(&mut self) -> Result<&mut ADBUSBDevice> {
        crate::selection::ensure_single_usb_target().map_err(|e| match e {
            crate::selection::UsbSelectionError::MultipleDevices => AdbError::MultipleDevices,
            crate::selection::UsbSelectionError::Enumeration(_) => AdbError::Client(e.to_string()),
            other => AdbError::Client(other.to_string()),
        })?;
        if self.device.is_none() {
            let key_path = Self::ensure_key_path()?;
            // See `CONNECT_RETRY_ATTEMPTS`. Re-autodetecting per attempt also
            // picks up a replug between attempts.
            let mut last_err: Option<String> = None;
            for attempt in 0..CONNECT_RETRY_ATTEMPTS {
                match ADBUSBDevice::autodetect_with_custom_private_key(key_path.clone()) {
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
                        if !serial.is_empty() {
                            self.serial = Some(serial);
                        }
                        self.cached_bootmode = None;
                        self.device = Some(dev);
                        return Ok(self.device.as_mut().expect("device just set"));
                    }
                    Err(e) => {
                        last_err = Some(e.to_string());
                        if attempt + 1 < CONNECT_RETRY_ATTEMPTS {
                            std::thread::sleep(CONNECT_RETRY_BACKOFF);
                        }
                    }
                }
            }
            return Err(AdbError::Client(
                last_err.unwrap_or_else(|| "ADB USB open failed".into()),
            ));
        }
        Ok(self.device.as_mut().expect("device cached"))
    }

    /// Drop the cached handle so the next probe re-authenticates against a
    /// freshly enumerated device (replug, adbd restart).
    fn drop_device(&mut self) {
        self.device = None;
        self.cached_bootmode = None;
    }

    /// Probe for a *fully-authorized* ADB device; updates stored serial.
    ///
    /// `true` only when an Android USB device is visible and the auth
    /// handshake succeeds; the boot mode is not checked.
    pub fn check_device(&mut self) -> Result<bool> {
        // Cheap presence probe before an auth attempt.
        let infos =
            find_all_connected_adb_devices().map_err(|e| AdbError::Client(e.to_string()))?;
        if infos.is_empty() {
            self.drop_device();
            return Ok(false);
        }
        match self.connect_device() {
            Ok(_) => Ok(true),
            Err(err) => {
                self.drop_device();
                if matches!(err, AdbError::MultipleDevices) {
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
    /// | More than one Android/EDL USB device attached | `Err(AdbError::MultipleDevices)` |
    ///
    /// `adb_client` discards the connect banner, so sideload is inferred from
    /// minadbd's exact `CLSE` shell refusal; rescue mode answers the same way
    /// and may be reported as sideload. Other shell errors are returned, so a
    /// transport failure cannot pass as sideload.
    pub fn check_device_state(&mut self) -> Result<Option<&'static str>> {
        let infos =
            find_all_connected_adb_devices().map_err(|e| AdbError::Client(e.to_string()))?;
        if infos.is_empty() {
            self.drop_device();
            return Ok(None);
        }
        match self.connect_device() {
            Ok(_) => {
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
            Err(err) => {
                self.drop_device();
                // Skip the TCP probe when the answer is already an error.
                let server_running =
                    !matches!(err, AdbError::MultipleDevices) && adb_server_running();
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
        AdbError, connect_failure_state, ensure_key_at_path, is_adbd_dropped_after_reboot,
        is_sideload_shell_refusal, remove_corrupt_key, validate_key_file, write_key_atomic,
    };
    use rsa::pkcs8::{EncodePrivateKey, LineEnding};
    use std::path::PathBuf;

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
