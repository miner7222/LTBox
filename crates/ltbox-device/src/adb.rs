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

/// Probe whether an `adb.exe` server (or any other process) is bound to
/// `127.0.0.1:5037`. A live listener there claims the Android USB
/// interface exclusively, so LTBox's `ADBUSBDevice` claim fails with
/// `LIBUSB_ERROR_BUSY` and `check_device_state` would otherwise bucket
/// the failure into `"unauthorized"` — misleading the user into tapping
/// "Allow USB debugging" again instead of killing the conflicting
/// server.
pub fn adb_server_running() -> bool {
    TcpStream::connect_timeout(&ADB_SERVER_ADDR.into(), ADB_SERVER_PROBE_TIMEOUT).is_ok()
}

/// Send the raw `host:kill` ADB protocol message to `127.0.0.1:5037` to
/// stop a running adb server without depending on an `adb.exe` binary
/// being on `PATH`. ADB host protocol: 4-byte ASCII-hex length prefix +
/// command. Server replies `OKAY` then exits the process.
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
    // `host:kill` is special: the server may reply `OKAY` and then close
    // the socket, or close immediately without replying because it's
    // exiting. Treat an EOF / connection-reset before 4 bytes as success
    // (server died = kill worked); only `FAIL` is a real rejection.
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
    /// Failed to read or write the LTBox-owned ADB RSA private key.
    #[error("Failed to prepare ADB key: {0}")]
    Key(String),
}

type Result<T> = std::result::Result<T, AdbError>;

/// Upper bound on `wait_for_device` before surfacing `Timeout`. 120 s is the
/// v2 expectation for post-reboot re-detection: a device re-enumerating after
/// `reboot bootloader` / `reboot edl` can take well over a minute on a cold
/// boot, and a shorter bound turned a slow-but-working device into a spurious
/// failure mid-flow.
const WAIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// libusb interface-claim retry budget. Two short-lived `AdbManager`
/// instances racing for the same Android USB endpoint (e.g. the
/// Dashboard's 3 s polling tick still draining one or two
/// `getprop` shells when the user clicks Reboot) hit
/// `LIBUSB_ERROR_BUSY` on the second claim because the first
/// holder's drop hasn't propagated through the kernel-side USB
/// release yet. The retry window has to comfortably exceed the
/// longest Dashboard polling cycle — empirically a few `getprop`
/// shells + model / slot / boardid reads finish under 1 s, so 10
/// attempts × 150 ms = 1.5 s gives a margin.
const CONNECT_RETRY_ATTEMPTS: u32 = 10;
const CONNECT_RETRY_BACKOFF: std::time::Duration = std::time::Duration::from_millis(150);

pub struct AdbManager {
    serial: Option<String>,
    /// Cached USB device handle — lazy, populated on first successful
    /// `connect_device`. Reused so the RSA auth handshake only fires
    /// once per `AdbManager` lifetime. Dropped on any I/O error so a
    /// replug always recovers.
    device: Option<ADBUSBDevice>,
    /// Cached `getprop ro.bootmode` from the first successful connect.
    /// `Some("recovery")` promotes the reported state from "device" to
    /// "recovery" so `poll_active_slot` and the Dashboard chip see the
    /// distinction. Cleared whenever `device` is dropped.
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

    /// Convenience for the `let mut adb = AdbManager::new(); if
    /// adb.check_device().unwrap_or(false) { ... }` pattern repeated
    /// across the GUI, root pipeline, and rescue flows. Returns
    /// `Some(adb)` when a fully-authorized device is reachable, `None`
    /// otherwise (including when the underlying probe errored — every
    /// existing caller swallowed the error via `unwrap_or(false)`).
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

    /// Resolve LTBox's owned ADB private key, generating + persisting
    /// a fresh PKCS8 PEM if the file is missing or corrupt. 2048-bit
    /// modulus is what modern adbd's public key validation requires.
    fn ensure_key_path() -> Result<PathBuf> {
        ensure_key_at_path(ltbox_core::app_paths::adb_key_path())
    }

    /// Open (or reuse) the cached `ADBUSBDevice`. On a fresh open,
    /// also populates `self.serial` from `getprop ro.serialno` so
    /// device-info popups + log lines show the user-visible Android
    /// serial (`HA...` on Lenovo / Samsung, hex-string on Pixel) the
    /// way the legacy server path did. USB descriptor VID:PID is too
    /// coarse to identify a specific device when multiple of the same
    /// model are around.
    fn connect_device(&mut self) -> Result<&mut ADBUSBDevice> {
        crate::selection::ensure_single_usb_target()
            .map_err(|e| AdbError::Client(e.to_string()))?;
        if self.device.is_none() {
            let key_path = Self::ensure_key_path()?;
            // Retry the libusb claim a few times — see
            // `CONNECT_RETRY_ATTEMPTS` for the race rationale. A fresh
            // `autodetect_with_custom_private_key` per attempt also
            // re-runs Android-descriptor filtering so a replug between
            // attempts is picked up automatically.
            let mut last_err: Option<String> = None;
            for attempt in 0..CONNECT_RETRY_ATTEMPTS {
                match ADBUSBDevice::autodetect_with_custom_private_key(key_path.clone()) {
                    Ok(mut dev) => {
                        // Stamp the user-facing serial inline on the
                        // freshly opened handle. Calling out to
                        // `shell_inner` would route through
                        // `drop_device` on any shell-side failure
                        // (recovery adbd that auth-handshakes but
                        // doesn't respond to shell), which would
                        // invalidate the device we just opened before
                        // we'd had a chance to hand it back. Pulling
                        // the shell directly here keeps the
                        // `self.device = Some(dev)` move atomic with
                        // the `return Ok(...)`.
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

    /// Drop the cached USB device handle. Called after any I/O error
    /// so the next probe re-runs the auth handshake against a freshly
    /// enumerated device (handles replug and adbd restart).
    fn drop_device(&mut self) {
        self.device = None;
        self.cached_bootmode = None;
    }

    /// Probe for a *fully-authorized* ADB device; updates stored serial.
    ///
    /// Returns `true` only when libusb sees at least one Android device
    /// AND the auth handshake succeeds. Both shell-able states (the
    /// regular "device" boot mode and "recovery") map to `true`
    /// because every LTBox caller treats them identically.
    pub fn check_device(&mut self) -> Result<bool> {
        // Cheap presence probe before paying for an auth attempt — if
        // libusb sees nothing, no point handshaking.
        let infos =
            find_all_connected_adb_devices().map_err(|e| AdbError::Client(e.to_string()))?;
        if infos.is_empty() {
            self.drop_device();
            return Ok(false);
        }
        match self.connect_device() {
            Ok(_) => Ok(true),
            Err(_) => {
                self.drop_device();
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
    ///
    /// The `sideload` row is what separates "connected but useless" from
    /// "not authorized". `adb_client` gives no access to the connect
    /// banner that names the state outright, so the state is inferred from
    /// the exact `CLSE` response used when minadbd refuses a shell. Rescue
    /// mode uses the same response and may therefore be reported as
    /// sideload; the discarded connect banner is the only way to tell them
    /// apart. Other shell errors are returned so transport failures cannot
    /// masquerade as a connected sideload device.
    pub fn check_device_state(&mut self) -> Result<Option<&'static str>> {
        let infos =
            find_all_connected_adb_devices().map_err(|e| AdbError::Client(e.to_string()))?;
        if infos.is_empty() {
            self.drop_device();
            return Ok(None);
        }
        match self.connect_device() {
            Ok(_) => {
                // First successful connect probes bootmode once; cache
                // hit for subsequent state polls so the Dashboard's 3 s
                // heartbeat doesn't re-shell every tick.
                if self.cached_bootmode.is_none() {
                    self.cached_bootmode = Some(match self.shell_inner("getprop ro.bootmode") {
                        Ok(mode) if mode.trim() == "recovery" => "recovery",
                        Ok(_) => "device",
                        // The dependency discards the connect banner, so
                        // use only its exact unsupported-shell response as
                        // the minadbd-style sideload inference. Rescue uses
                        // the same response and is therefore an unavoidable
                        // false-positive until the banner is exposed.
                        Err(err) if is_sideload_shell_refusal(&err) => "sideload",
                        Err(err) => return Err(err),
                    });
                }
                Ok(self.cached_bootmode)
            }
            Err(_) => {
                // A live external adb server on 127.0.0.1:5037 holds the
                // Android USB interface exclusively → our libusb claim
                // returns `LIBUSB_ERROR_BUSY`. Surface that as a
                // distinct state so the dashboard can offer "kill
                // server" instead of misleading the user into tapping
                // "Allow USB debugging" again.
                if adb_server_running() {
                    Ok(Some("adb_server_blocking"))
                } else {
                    Ok(Some("unauthorized"))
                }
            }
        }
    }

    /// Wait up to [`WAIT_TIMEOUT`] for a fully-authorized ADB device.
    ///
    /// Returns `AdbError::Timeout` on expiry instead of spinning forever.
    pub fn wait_for_device(&mut self) -> Result<()> {
        self.wait_for_device_or(|| false).map(|_| ())
    }

    /// Like [`wait_for_device`](Self::wait_for_device), but abandon the wait
    /// early (returning `Ok(false)`) the moment `bail` returns true. Used to
    /// drop the post-fastboot ADB wait as soon as the device jumps straight to
    /// the EDL/9008 port, instead of blocking for the full timeout when ADB will
    /// never come up. `Ok(true)` = ADB device ready; `Err(Timeout)` = neither
    /// appeared in time. `bail` is checked before each device probe so EDL
    /// detection takes priority over a transient ADB probe error.
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
                // Drop the cached handle so the next probe re-runs the
                // auth handshake against the (possibly replugged)
                // device. Otherwise a single broken-pipe leaves every
                // subsequent shell call returning the same stale
                // error.
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

    /// Active slot suffix — only `"_a"` or `"_b"`, else `None`.
    ///
    /// v2 parity: `device/adb.py::get_slot_suffix` whitelisted `["_a", "_b"]`
    /// and returned `None` otherwise. Some bootloaders return an empty or
    /// garbage prop on non-A/B devices; feeding that into downstream
    /// `vendor_boot{suffix}` partition names produces lookups for e.g.
    /// `vendor_bootxyz` that fail with a misleading error.
    pub fn get_slot_suffix(&mut self) -> Result<Option<String>> {
        match self.shell_inner("getprop ro.boot.slot_suffix") {
            Ok(s) if s == "_a" || s == "_b" => Ok(Some(s)),
            _ => Ok(None),
        }
    }

    /// The kernel release token from `/proc/version`, whole — version *and*
    /// the `-androidNN-...` GKI branch that follows it. Callers normalize it
    /// down to `major.minor` themselves; the branch is what tells an LKM
    /// consumer which GKI the module has to match, and 5.10 ships under two
    /// different ones.
    pub fn get_kernel_version(&mut self) -> Result<Option<String>> {
        const PREFIX: &str = "Linux version ";
        match self.shell_inner("cat /proc/version") {
            Ok(v) => {
                // A hardcoded `start + 14` offset would drift silently if
                // `PREFIX`'s length ever changed; `find` + `strip_prefix`
                // on the trimmed tail keeps the offset in sync with the
                // literal automatically.
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
        // RebootType has no EDL variant; fall back to shell. adbd dies
        // immediately after issuing the reboot, so the shell call
        // itself may surface as a USB pipe error — treat that as
        // success since the reboot did fire.
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
        // adbd tears down the USB connection as the reboot fires, so
        // the in-flight ADB transaction often returns a pipe / EOF
        // error even though the command was already acknowledged.
        // The user-visible effect of a "successful" reboot and a
        // "failed-but-actually-rebooted" reboot is identical (device
        // re-enumerates), so suppress the spurious error path.
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
            // A push failure usually leaves the cached USB handle in a
            // partially-drained state — drop it so the next operation
            // re-runs the libusb claim instead of inheriting the half-
            // broken transport.
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

    /// Copy a local file to an arbitrary on-device path (e.g.
    /// `/sdcard/manager.apk`). Unlike [`install`], this only transfers the
    /// file — no `pm install` — so a caller whose auto-install failed can
    /// drop the APK somewhere the user can reach and install it by hand.
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
                // Corrupt / unreadable material would otherwise surface as a
                // mysterious USB-auth failure. Recover by regenerating when
                // the current PKCS8 decode path rejects the bytes.
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

/// Heuristic: did the most recent ADB transaction fail because adbd
/// disconnected mid-call (the typical signature of "reboot fired,
/// transport dropped before ack made it back")? Matches the wording
/// `adb_client` / `rusb` surface for `LIBUSB_ERROR_PIPE`,
/// `LIBUSB_ERROR_IO`, broken-pipe / EOF / NoDevice cases. Kept
/// narrow on purpose — generic strings like "not found" would also
/// match unrelated shell errors (e.g. `command not found` echoed back
/// to a getprop wrapper), which should not be silently swallowed as
/// success.
fn is_adbd_dropped_after_reboot(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    // Keep the pipe match exact: bare "pipe" also hits unrelated wording
    // (named-pipe helpers, "pipeline", etc.). Observed adbd teardown forms
    // use "broken pipe" or the rusb/libusb "pipe error" phrasing.
    lower.contains("broken pipe")
        || lower.contains("pipe error")
        || lower.contains("no device")
        || lower.contains("no such device")
        || lower.contains("device disconnected")
        || lower.contains("unexpected eof")
        || lower.contains("end of file")
        // `LIBUSB_ERROR_IO` rusb stringifies as "Input/Output Error" (or
        // "I/O error" in some versions) — observed on Linux (rusb 3.0.8)
        // when adbd tears down the USB endpoint slightly earlier than
        // usual, so the reboot's in-flight transaction surfaces as IO
        // instead of PIPE.
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
        AdbError, ensure_key_at_path, is_adbd_dropped_after_reboot, is_sideload_shell_refusal,
        remove_corrupt_key, validate_key_file, write_key_atomic,
    };
    use rsa::pkcs8::{EncodePrivateKey, LineEnding};
    use std::path::PathBuf;

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
        // Real shell-echoed `command not found` must NOT be swallowed
        // as a success — the doc comment on the matcher calls this out
        // explicitly, so guard with a test.
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
