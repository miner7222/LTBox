//! Minimal Fastboot over nusb — only the commands LTBox uses
//! (getvar, reboot, reboot-bootloader, reboot-fastboot, detect). Protocol:
//! ASCII command → bulk write → read OKAY/FAIL/DATA/INFO/TEXT.

use nusb::Endpoint;
use nusb::transfer::{Buffer, Bulk, In, Out};
use std::time::Duration;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum FastbootError {
    #[error("USB error: {0}")]
    Usb(String),
    #[error("Device not found")]
    DeviceNotFound,
    #[error("Multiple fastboot devices found; disconnect all other devices and try again")]
    MultipleDevices,
    #[error("Command failed: {0}")]
    CommandFailed(String),
}

type Result<T> = std::result::Result<T, FastbootError>;

const FASTBOOT_USB_CLASS: u8 = 0xFF;
const FASTBOOT_USB_SUBCLASS: u8 = 0x42;
const FASTBOOT_USB_PROTOCOL: u8 = 0x03;

const WAIT_SLEEP: Duration = Duration::from_secs(2);
/// One bulk transfer. A wedged bootloader or half-detached cable must not
/// block forever, or the callers' own polling deadlines could never fire.
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(10);
/// One command, however many INFO lines it streams.
const COMMAND_DEADLINE: Duration = Duration::from_secs(60);

#[derive(Debug, Default, Clone)]
pub struct FastbootVars {
    pub model: Option<String>,
    pub product: Option<String>,
    pub serialno: Option<String>,
    pub current_slot: Option<String>,
    pub build_display_id: Option<String>,
    pub ram_gb: Option<String>,
    pub storage_gb: Option<String>,
    pub rollback_indices: std::collections::HashMap<u32, u64>,
    /// Raw `getvar:all` dump (the INFO lines joined), with a `serialno:` line
    /// guaranteed. Saved as `getvar.txt` in the Flash critical backup.
    pub raw_getvar_all: String,
}

pub struct FastbootDevice {
    // Kept alive so the endpoints below stay bound to the claim.
    _interface: nusb::Interface,
    ep_in: Endpoint<Bulk, In>,
    ep_out: Endpoint<Bulk, Out>,
    poisoned: bool,
}

struct FastbootCandidate {
    device_info: nusb::DeviceInfo,
    interface_numbers: Vec<u8>,
}

impl FastbootDevice {
    pub fn open() -> Result<Self> {
        use nusb::MaybeFuture;

        crate::selection::ensure_single_usb_target().map_err(map_target_selection_error)?;

        let devices = nusb::list_devices()
            .wait()
            .map_err(|e| FastbootError::Usb(e.to_string()))?;
        let mut candidates = Vec::new();
        for device_info in devices {
            // Windows whole-device WinUSB bindings have no interface summary.
            // Descriptor reads may open a handle, but no interface is claimed
            // until every physical candidate has been identified.
            let mut interface_numbers = device_info
                .interfaces()
                .filter(|iface| {
                    is_fastboot_interface(iface.class(), iface.subclass(), iface.protocol())
                })
                .map(|iface| iface.interface_number())
                .collect::<Vec<_>>();
            if interface_numbers.is_empty() {
                if let Ok(device) = device_info.open().wait() {
                    for config in device.configurations() {
                        for iface in config.interfaces() {
                            if iface.alt_settings().any(|alt| {
                                is_fastboot_interface(alt.class(), alt.subclass(), alt.protocol())
                            }) {
                                interface_numbers.push(iface.interface_number());
                            }
                        }
                    }
                } else if is_fastboot_interface(
                    device_info.class(),
                    device_info.subclass(),
                    device_info.protocol(),
                ) {
                    // Known but inaccessible candidates must still count.
                    interface_numbers.push(0);
                }
            }
            if !interface_numbers.is_empty() {
                candidates.push(FastbootCandidate {
                    device_info,
                    interface_numbers,
                });
            }
        }

        claim_unique_candidate(
            candidates,
            |candidate| candidate.device_info.id(),
            Self::open_candidate,
        )
        .map(|device| device.ok_or(FastbootError::DeviceNotFound))?
    }

    fn open_candidate(candidate: FastbootCandidate) -> Result<Self> {
        use nusb::MaybeFuture;

        let device = candidate
            .device_info
            .open()
            .wait()
            .map_err(|e| FastbootError::Usb(e.to_string()))?;

        for config in device.configurations() {
            for iface in config.interfaces() {
                if !candidate
                    .interface_numbers
                    .contains(&iface.interface_number())
                {
                    continue;
                }
                for alt in iface.alt_settings() {
                    if !is_fastboot_interface(alt.class(), alt.subclass(), alt.protocol()) {
                        continue;
                    }
                    let mut in_addr: u8 = 0;
                    let mut out_addr: u8 = 0;
                    for ep in alt.endpoints() {
                        match ep.direction() {
                            nusb::transfer::Direction::In => in_addr = ep.address(),
                            nusb::transfer::Direction::Out => out_addr = ep.address(),
                        }
                    }
                    if in_addr == 0 || out_addr == 0 {
                        continue;
                    }
                    let interface_number = iface.interface_number();
                    let interface = device
                        .claim_interface(interface_number)
                        .wait()
                        .map_err(|e| FastbootError::Usb(e.to_string()))?;
                    let ep_in = interface
                        .endpoint::<Bulk, In>(in_addr)
                        .map_err(|e| FastbootError::Usb(e.to_string()))?;
                    let ep_out = interface
                        .endpoint::<Bulk, Out>(out_addr)
                        .map_err(|e| FastbootError::Usb(e.to_string()))?;
                    return Ok(Self {
                        _interface: interface,
                        ep_in,
                        ep_out,
                        poisoned: false,
                    });
                }
            }
        }

        Err(FastbootError::DeviceNotFound)
    }

    pub fn check_device() -> bool {
        Self::open().is_ok()
    }

    /// Wait at most `timeout` for a Fastboot device to appear.
    pub fn wait_for_device_within(timeout: Duration) -> Result<Self> {
        wait_for_device_with(
            timeout,
            Self::open,
            std::thread::sleep,
            std::time::Instant::now,
        )
    }

    /// Submit `buf` on the OUT endpoint and wait up to [`TRANSFER_TIMEOUT`].
    fn bulk_write(&mut self, buf: Vec<u8>) -> Result<()> {
        ensure_transport_ready(self.poisoned)?;
        let completion = self
            .ep_out
            .transfer_blocking(Buffer::from(buf), TRANSFER_TIMEOUT);
        completion
            .status
            .map_err(|e| transfer_error(e, &mut self.poisoned))?;
        Ok(())
    }

    /// Submit a 4 KiB read on the IN endpoint and wait up to
    /// [`TRANSFER_TIMEOUT`]. Returns the initialized prefix of the buffer.
    fn bulk_read(&mut self) -> Result<Vec<u8>> {
        ensure_transport_ready(self.poisoned)?;
        let completion = self
            .ep_in
            .transfer_blocking(Buffer::new(4096), TRANSFER_TIMEOUT);
        completion
            .status
            .map_err(|e| transfer_error(e, &mut self.poisoned))?;
        let len = completion.actual_len;
        let mut out = completion.buffer.into_vec();
        out.truncate(len);
        Ok(out)
    }

    /// Send command and read until OKAY/FAIL.
    fn command(&mut self, cmd: &str) -> Result<String> {
        self.bulk_write(cmd.as_bytes().to_vec())?;

        let deadline = std::time::Instant::now() + COMMAND_DEADLINE;
        loop {
            check_deadline(deadline, cmd)?;
            let data = self.bulk_read()?;

            match classify_packet(&data)? {
                Packet::Okay(payload) | Packet::Data(payload) => return Ok(payload.to_string()),
                Packet::Fail(payload) => {
                    return Err(FastbootError::CommandFailed(payload.to_string()));
                }
                Packet::Info(_) | Packet::Text(_) => continue,
            }
        }
    }

    /// Send command, collect all INFO lines.
    fn command_all(&mut self, cmd: &str) -> Result<Vec<String>> {
        self.bulk_write(cmd.as_bytes().to_vec())?;

        let mut lines = Vec::new();
        let deadline = std::time::Instant::now() + COMMAND_DEADLINE;
        loop {
            check_deadline(deadline, cmd)?;
            let data = self.bulk_read()?;
            match classify_packet(&data)? {
                Packet::Info(payload) => lines.push(payload.to_string()),
                Packet::Text(_) => continue,
                Packet::Okay(_) | Packet::Data(_) => break,
                Packet::Fail(payload) => {
                    return Err(FastbootError::CommandFailed(payload.to_string()));
                }
            }
        }
        Ok(lines)
    }

    pub fn getvar(&mut self, variable: &str) -> Result<String> {
        self.command(&format!("getvar:{variable}"))
    }

    pub fn get_model(&mut self) -> Result<Option<String>> {
        match self.getvar("product") {
            Ok(v) if !v.is_empty() => Ok(Some(v)),
            _ => Ok(None),
        }
    }

    /// Active slot suffix (`_a` / `_b`), else `None`, matching
    /// [`crate::adb::AdbManager::get_slot_suffix`]. Garbage `current-slot`
    /// values would otherwise build bogus partition names.
    pub fn get_slot_suffix(&mut self) -> Result<Option<String>> {
        match self.getvar("current-slot") {
            Ok(slot) => Ok(normalize_slot_suffix(&slot)),
            _ => Ok(None),
        }
    }

    /// Parse vars from `getvar:all` INFO lines.
    pub fn get_all_vars(&mut self) -> Result<FastbootVars> {
        let mut vars = FastbootVars::default();
        // Best-effort: a transient failure here must not discard the
        // getvar:all fields (an Err would blank the whole poll).
        if let Ok(slot) = self.get_slot_suffix() {
            vars.current_slot = slot;
        }
        if let Ok(sn) = self.getvar("serialno")
            && !sn.is_empty()
        {
            vars.serialno = Some(sn);
        }
        if let Ok(lines) = self.command_all("getvar:all") {
            for line in &lines {
                apply_getvar_all_line(&mut vars, line);
            }
            // Some bootloaders omit `serialno` from `getvar:all`; prepend the
            // separately fetched one so `getvar.txt` always records it.
            let mut raw = lines.join("\n");
            if let Some(sn) = &vars.serialno
                && !lines.iter().any(|l| l.starts_with("serialno:"))
            {
                raw = format!("serialno:{sn}\n{raw}");
            }
            vars.raw_getvar_all = raw;
        }
        // product = market_name in GUI
        if let Ok(p) = self.get_model() {
            vars.product = p;
        }
        Ok(vars)
    }

    pub fn reboot(&mut self) -> Result<()> {
        self.command("reboot").map(|_| ())
    }

    pub fn reboot_bootloader(&mut self) -> Result<()> {
        self.command("reboot-bootloader").map(|_| ())
    }

    /// Reboot into userspace fastboot. Bootloaders that predate fastbootd
    /// answer this with FAIL, which surfaces as a command error.
    pub fn reboot_fastboot(&mut self) -> Result<()> {
        self.command("reboot-fastboot").map(|_| ())
    }

    /// Whether this endpoint is fastbootd rather than the bootloader's
    /// own fastboot. Both enumerate identically, so only `is-userspace`
    /// separates them; a bootloader that predates fastbootd does not know
    /// the variable and fails the getvar, which reads as `false`.
    pub fn is_userspace(&mut self) -> bool {
        self.getvar("is-userspace")
            .map(|v| v.trim().eq_ignore_ascii_case("yes"))
            .unwrap_or(false)
    }
}

fn wait_for_device_with<T>(
    timeout: Duration,
    mut open: impl FnMut() -> Result<T>,
    mut sleep: impl FnMut(Duration),
    mut now: impl FnMut() -> std::time::Instant,
) -> Result<T> {
    let started = now();
    loop {
        if now().saturating_duration_since(started) >= timeout {
            return Err(FastbootError::DeviceNotFound);
        }
        match open() {
            Err(FastbootError::DeviceNotFound) => {
                let elapsed = now().saturating_duration_since(started);
                sleep(WAIT_SLEEP.min(timeout.saturating_sub(elapsed)));
            }
            result => return result,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Packet<'a> {
    Okay(&'a str),
    Fail(&'a str),
    Info(&'a str),
    Text(&'a str),
    Data(&'a str),
}

fn classify_packet(data: &[u8]) -> Result<Packet<'_>> {
    if data.len() < 4 {
        return Err(FastbootError::CommandFailed("Short response".into()));
    }
    let status = std::str::from_utf8(&data[..4]).unwrap_or("");
    let payload = std::str::from_utf8(&data[4..]).unwrap_or("").trim();
    match status {
        "OKAY" => Ok(Packet::Okay(payload)),
        "FAIL" => Ok(Packet::Fail(payload)),
        "INFO" => Ok(Packet::Info(payload)),
        "TEXT" => Ok(Packet::Text(payload)),
        "DATA" => Ok(Packet::Data(payload)),
        _ => Err(FastbootError::CommandFailed(format!("Unknown: {status}"))),
    }
}

fn ensure_transport_ready(poisoned: bool) -> Result<()> {
    if poisoned {
        return Err(FastbootError::Usb(
            "fastboot transport timed out earlier; reopen the device".into(),
        ));
    }
    Ok(())
}

fn transfer_error(error: nusb::transfer::TransferError, poisoned: &mut bool) -> FastbootError {
    match error {
        nusb::transfer::TransferError::Cancelled => {
            *poisoned = true;
            FastbootError::Usb(format!(
                "no response within {} s",
                TRANSFER_TIMEOUT.as_secs()
            ))
        }
        other => FastbootError::Usb(other.to_string()),
    }
}

/// A bootloader streaming INFO lines forever must not hold the caller.
fn check_deadline(deadline: std::time::Instant, cmd: &str) -> Result<()> {
    if std::time::Instant::now() >= deadline {
        return Err(FastbootError::CommandFailed(format!(
            "{cmd}: no OKAY/FAIL within {} s",
            COMMAND_DEADLINE.as_secs()
        )));
    }
    Ok(())
}

fn is_fastboot_interface(class: u8, subclass: u8, protocol: u8) -> bool {
    class == FASTBOOT_USB_CLASS
        && subclass == FASTBOOT_USB_SUBCLASS
        && protocol == FASTBOOT_USB_PROTOCOL
}

fn map_target_selection_error(error: crate::selection::UsbSelectionError) -> FastbootError {
    match error {
        crate::selection::UsbSelectionError::MultipleDevices => FastbootError::MultipleDevices,
        crate::selection::UsbSelectionError::Enumeration(error) => FastbootError::Usb(error),
    }
}

fn select_unique_candidate<T, K, I, Key>(candidates: I, key: Key) -> Result<Option<T>>
where
    I: IntoIterator<Item = T>,
    Key: Fn(&T) -> K,
    K: Eq,
{
    let mut selected: Option<(K, T)> = None;
    for candidate in candidates {
        let candidate_key = key(&candidate);
        if let Some((selected_key, _)) = selected.as_ref() {
            if selected_key == &candidate_key {
                continue;
            }
            return Err(FastbootError::MultipleDevices);
        }
        selected = Some((candidate_key, candidate));
    }
    Ok(selected.map(|(_, candidate)| candidate))
}

fn claim_unique_candidate<T, K, I, Key, Claim, R>(
    candidates: I,
    key: Key,
    claim: Claim,
) -> Result<Option<R>>
where
    I: IntoIterator<Item = T>,
    Key: Fn(&T) -> K,
    K: Eq,
    Claim: FnOnce(T) -> Result<R>,
{
    let Some(candidate) = select_unique_candidate(candidates, key)? else {
        return Ok(None);
    };
    claim(candidate).map(Some)
}

/// Normalize a fastboot `current-slot` payload to `_a` / `_b`.
fn normalize_slot_suffix(slot: &str) -> Option<String> {
    match slot.trim() {
        "a" | "_a" => Some("_a".to_string()),
        "b" | "_b" => Some("_b".to_string()),
        _ => None,
    }
}

/// Apply one `getvar:all` INFO line into `vars`. Product and serialno are
/// separate getvar queries in [`FastbootDevice::get_all_vars`].
pub(crate) fn apply_getvar_all_line(vars: &mut FastbootVars, line: &str) {
    // RAM/storage only: the leading `hwboardid` token is the SoC on some
    // SKUs, so the model comes from `modelname:` below.
    if let Some(val) = line.strip_prefix("hwboardid:")
        && let Some((ram, storage)) = parse_hwboardid_ram_storage(val.trim())
    {
        vars.ram_gb = Some(ram);
        vars.storage_gb = Some(storage);
    }
    // `modelname:TB322FC` — the bootloader-published model.
    if let Some(val) = line.strip_prefix("modelname:") {
        let val = val.trim();
        if !val.is_empty() {
            vars.model = Some(val.to_string());
        }
    }
    // Prefer an explicit current-slot from the dump when present so a
    // failed standalone slot query still leaves the dashboard filled.
    if vars.current_slot.is_none()
        && let Some(val) = line.strip_prefix("current-slot:")
    {
        vars.current_slot = normalize_slot_suffix(val);
    }
    if let Some((slot, val)) = parse_stored_rollback_line(line) {
        vars.rollback_indices.insert(slot, val);
    }
    if let Some(val) = line.strip_prefix("build-display-id:") {
        let v = val.trim();
        if !v.is_empty() {
            vars.build_display_id = Some(v.to_string());
        }
    } else if let Some(val) = line.strip_prefix("build.display.id:") {
        let v = val.trim();
        if !v.is_empty() {
            vars.build_display_id = Some(v.to_string());
        }
    }
}

/// Parse `stored_rollback_index:N = HEX`. Value is always base-16.
/// Tolerates `(N)`/`N)` slot wrapping and optional `0x` prefix.
pub(crate) fn parse_stored_rollback_line(line: &str) -> Option<(u32, u64)> {
    let rest = line.strip_prefix("stored_rollback_index:")?;
    let (slot_str, val_str) = rest.split_once('=')?;
    let slot_str = slot_str
        .trim()
        .trim_start_matches('(')
        .trim_end_matches(')');
    let val_str = val_str.trim();
    let slot: u32 = slot_str.parse().ok()?;
    let hex = val_str.strip_prefix("0x").unwrap_or(val_str);
    let val = u64::from_str_radix(hex, 16).ok()?;
    Some((slot, val))
}

/// Pull RAM + storage out of a Lenovo `hwboardid` getvar value.
///
/// Returns the first `_`-separated `<digits>+<digits>` segment as
/// `("<ram> GB", "<storage> GB")`. Known shapes:
///
/// * `TB322FC_SM8750P_16+512` — model + SoC + spec
/// * `SM8750P_16+512`         — SoC + spec only
/// * `SM8650P_12+256_12`      — SoC + spec + trailing suffix
///
/// Returns `None` when no segment matches.
pub(crate) fn parse_hwboardid_ram_storage(val: &str) -> Option<(String, String)> {
    for part in val.split('_') {
        if let Some((ram, storage)) = part.split_once('+')
            && !ram.is_empty()
            && !storage.is_empty()
            && ram.chars().all(|c| c.is_ascii_digit())
            && storage.chars().all(|c| c.is_ascii_digit())
        {
            return Some((format!("{ram} GB"), format!("{storage} GB")));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq, Eq)]
    struct TestCandidate {
        device: u8,
        fastboot: bool,
    }

    #[test]
    fn bounded_wait_times_out_without_sleeping_past_deadline() {
        let now = std::cell::Cell::new(std::time::Instant::now());
        let started = now.get();
        let mut polls = 0;
        let result = wait_for_device_with::<()>(
            Duration::from_secs(5),
            || {
                polls += 1;
                Err(FastbootError::DeviceNotFound)
            },
            |duration| now.set(now.get() + duration),
            || now.get(),
        );
        assert!(matches!(result, Err(FastbootError::DeviceNotFound)));
        assert_eq!(polls, 3);
        assert_eq!(now.get().duration_since(started), Duration::from_secs(5));
    }

    #[test]
    fn bounded_wait_succeeds_after_three_polls() {
        let now = std::cell::Cell::new(std::time::Instant::now());
        let mut polls = 0;
        let result = wait_for_device_with(
            Duration::from_secs(10),
            || {
                polls += 1;
                if polls == 3 {
                    Ok(7)
                } else {
                    Err(FastbootError::DeviceNotFound)
                }
            },
            |duration| now.set(now.get() + duration),
            || now.get(),
        );
        assert_eq!(result.unwrap(), 7);
        assert_eq!(polls, 3);
    }

    #[test]
    fn bounded_wait_propagates_other_errors_without_sleep() {
        let result = wait_for_device_with::<()>(
            Duration::from_secs(10),
            || Err(FastbootError::MultipleDevices),
            |_| panic!("must not sleep"),
            std::time::Instant::now,
        );
        assert!(matches!(result, Err(FastbootError::MultipleDevices)));
    }

    #[test]
    fn packets_classify_all_statuses_and_reject_invalid_headers() {
        assert_eq!(
            classify_packet(b"OKAY value ").unwrap(),
            Packet::Okay("value")
        );
        assert_eq!(classify_packet(b"FAILbad").unwrap(), Packet::Fail("bad"));
        assert_eq!(classify_packet(b"INFOline").unwrap(), Packet::Info("line"));
        assert_eq!(classify_packet(b"TEXTline").unwrap(), Packet::Text("line"));
        assert_eq!(
            classify_packet(b"DATA00000010").unwrap(),
            Packet::Data("00000010")
        );
        assert!(matches!(
            classify_packet(b"OK"),
            Err(FastbootError::CommandFailed(_))
        ));
        assert!(matches!(
            classify_packet(b"WHAT"),
            Err(FastbootError::CommandFailed(_))
        ));
    }

    #[test]
    fn cancelled_transfer_poisons_transport() {
        let mut poisoned = false;
        assert!(ensure_transport_ready(poisoned).is_ok());
        let _ = transfer_error(nusb::transfer::TransferError::Cancelled, &mut poisoned);
        assert!(poisoned);
        assert!(
            matches!(ensure_transport_ready(poisoned), Err(FastbootError::Usb(message))
            if message == "fastboot transport timed out earlier; reopen the device")
        );
    }

    #[test]
    fn unique_selector_does_not_claim_when_no_device_is_present() {
        let mut claims = 0;
        let result = claim_unique_candidate(
            Vec::<u8>::new(),
            |device| *device,
            |_| {
                claims += 1;
                Ok(())
            },
        );

        assert_eq!(result.unwrap(), None);
        assert_eq!(claims, 0);
    }

    #[test]
    fn unique_selector_claims_the_only_device_once() {
        let mut claimed = Vec::new();
        let result = claim_unique_candidate(
            [7_u8],
            |device| *device,
            |device| {
                claimed.push(device);
                Ok(device + 1)
            },
        );

        assert_eq!(result.unwrap(), Some(8));
        assert_eq!(claimed, [7]);
    }

    #[test]
    fn unique_selector_rejects_multiple_devices_before_claim_in_either_order() {
        for candidates in [[1_u8, 2_u8], [2_u8, 1_u8]] {
            let mut claimed = Vec::new();
            let result = claim_unique_candidate(
                candidates,
                |device| *device,
                |device| {
                    claimed.push(device);
                    Ok(device)
                },
            );

            assert!(matches!(result, Err(FastbootError::MultipleDevices)));
            assert!(claimed.is_empty());
        }
    }

    #[test]
    fn unrelated_usb_devices_are_filtered_before_selection() {
        let result = select_unique_candidate(
            [
                TestCandidate {
                    device: 1,
                    fastboot: false,
                },
                TestCandidate {
                    device: 2,
                    fastboot: true,
                },
            ]
            .into_iter()
            .filter(|candidate| candidate.fastboot),
            |candidate| candidate.device,
        );

        assert_eq!(result.unwrap().map(|candidate| candidate.device), Some(2));
    }

    #[test]
    fn alternate_settings_on_one_physical_device_are_not_multiple_devices() {
        let result = select_unique_candidate(
            [
                TestCandidate {
                    device: 9,
                    fastboot: true,
                },
                TestCandidate {
                    device: 9,
                    fastboot: true,
                },
            ],
            |candidate| candidate.device,
        );

        assert_eq!(result.unwrap().map(|candidate| candidate.device), Some(9));
    }

    #[test]
    fn inaccessible_unique_candidate_returns_claim_error_without_fallback() {
        let result = claim_unique_candidate(
            [TestCandidate {
                device: 4,
                fastboot: true,
            }],
            |candidate| candidate.device,
            |_| Err::<(), _>(FastbootError::Usb("access denied".into())),
        );

        assert!(matches!(
            result,
            Err(FastbootError::Usb(message)) if message == "access denied"
        ));
    }

    #[test]
    fn bare_hex_parses_as_base16() {
        // Bare hex with no `0x` prefix must still parse as base-16, not fall through to 0.
        let out = parse_stored_rollback_line("stored_rollback_index:0 = 41B7A200");
        assert_eq!(out, Some((0, 0x41B7A200)));
    }

    #[test]
    fn prefixed_hex_still_parses() {
        let out = parse_stored_rollback_line("stored_rollback_index:1 = 0xDEADBEEF");
        assert_eq!(out, Some((1, 0xDEADBEEF)));
    }

    #[test]
    fn small_decimal_digits_parse_as_hex() {
        // Contract: always base-16. "100" → 0x100.
        let out = parse_stored_rollback_line("stored_rollback_index:0 = 100");
        assert_eq!(out, Some((0, 0x100)));
    }

    #[test]
    fn malformed_line_returns_none() {
        assert!(parse_stored_rollback_line("unrelated:0 = ff").is_none());
        assert!(parse_stored_rollback_line("stored_rollback_index:not_a_slot = ff").is_none());
        assert!(parse_stored_rollback_line("stored_rollback_index:0 = not_hex_ghi").is_none());
        assert!(parse_stored_rollback_line("stored_rollback_index:0").is_none());
    }

    #[test]
    fn trailing_paren_on_slot_is_stripped() {
        let out = parse_stored_rollback_line("stored_rollback_index:0) = 41B7A200");
        assert_eq!(out, Some((0, 0x41B7A200)));
    }

    #[test]
    fn both_parens_on_slot_are_stripped() {
        // `(0)` with both parens must still parse instead of silently skipping ARB.
        let out = parse_stored_rollback_line("stored_rollback_index:(0) = 41B7A200");
        assert_eq!(out, Some((0, 0x41B7A200)));
    }

    #[test]
    fn hwboardid_three_segment_soc_spec() {
        assert_eq!(
            parse_hwboardid_ram_storage("TB322FC_SM8750P_16+512"),
            Some(("16 GB".into(), "512 GB".into()))
        );
    }

    #[test]
    fn hwboardid_two_segment_soc_spec() {
        assert_eq!(
            parse_hwboardid_ram_storage("SM8750P_16+512"),
            Some(("16 GB".into(), "512 GB".into()))
        );
    }

    #[test]
    fn hwboardid_trailing_suffix_after_spec() {
        // A trailing numeric suffix must not be taken as the spec.
        assert_eq!(
            parse_hwboardid_ram_storage("SM8650P_12+256_12"),
            Some(("12 GB".into(), "256 GB".into()))
        );
    }

    #[test]
    fn hwboardid_no_spec_returns_none() {
        assert_eq!(parse_hwboardid_ram_storage("SM8750P_only"), None);
    }

    #[test]
    fn tb322fc_getvar_all_populates_dashboard_fields() {
        // Live TB322FC dump. The INFO lines alone must fill model/RAM/
        // storage/slot/firmware when the standalone slot query fails.
        let lines = [
            "modelname:TB322FC",
            "hwboardid:TB322FC_SM8750P_16+512",
            "current-slot:a",
            "build.display.id:TB322FC_CN_OPEN_USER_Q00041.1_W_ZUXOS_1.5.10.259_ST_2606119",
            "product:elden_prc_wifi",
            "serialno:HA289EBP",
        ];
        let mut vars = FastbootVars::default();
        for line in lines {
            apply_getvar_all_line(&mut vars, line);
        }
        assert_eq!(vars.model.as_deref(), Some("TB322FC"));
        assert_eq!(vars.ram_gb.as_deref(), Some("16 GB"));
        assert_eq!(vars.storage_gb.as_deref(), Some("512 GB"));
        assert_eq!(vars.current_slot.as_deref(), Some("_a"));
        assert_eq!(
            vars.build_display_id.as_deref(),
            Some("TB322FC_CN_OPEN_USER_Q00041.1_W_ZUXOS_1.5.10.259_ST_2606119")
        );
        // No stored_rollback_index here; TB322FC enforces no rollback
        // protection (its `ModelCapabilities` profile).
        assert!(vars.rollback_indices.is_empty());
        // product/serialno remain separate getvar queries in get_all_vars.
        assert!(vars.product.is_none());
        assert!(vars.serialno.is_none());
    }

    #[test]
    fn getvar_all_line_does_not_overwrite_existing_slot() {
        let mut vars = FastbootVars {
            current_slot: Some("_b".into()),
            ..FastbootVars::default()
        };
        apply_getvar_all_line(&mut vars, "current-slot:a");
        assert_eq!(vars.current_slot.as_deref(), Some("_b"));
    }
}
