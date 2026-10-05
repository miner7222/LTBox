//! Async device-poll results, popup UI state and their parsing helpers.

use crate::*;

#[derive(Debug, Clone, Default)]
pub(crate) struct DevicePollResult {
    pub(crate) status: ConnectionStatus,
    /// `status == Fastboot` and the endpoint answered `is-userspace: yes`,
    /// i.e. fastbootd rather than the bootloader. Display only — every
    /// behavioural branch treats the two the same.
    pub(crate) fastboot_userspace: bool,
    pub(crate) model: String,
    /// Trimmed `ro.build.version.release`. Only populated by ADB polls;
    /// fastboot and EDL do not expose Android system properties.
    pub(crate) android_version: String,
    pub(crate) slot: String,
    /// Trimmed `ro.build.display.id` — leading device-model prefix
    /// stripped so the dashboard cell stays readable.
    pub(crate) firmware: String,
    /// Untrimmed `ro.build.display.id` exactly as the device reports
    /// it. Required by Lenovo's OTA `querynewfirmware` endpoint —
    /// passing the trimmed form returns an empty `<firmwareupdate/>`
    /// because the upstream key matches the full string.
    pub(crate) firmware_full: String,
    pub(crate) arb: String,
    /// `boot` / `vbmeta_system` rollback floors classified from the
    /// fastboot `stored_rollback_index:N` vars. Only ever `Some` on a
    /// bootloader-mode poll — no other transport reports them.
    pub(crate) rollback_floors: Option<ltbox_patch::rollback::FastbootRollbackFloors>,
    pub(crate) ram: String,
    pub(crate) storage: String,
    pub(crate) market_name: String,
    /// Device serial captured from ADB or fastboot. Empty when no
    /// connected device produced a serial (EDL/Sahara never reports
    /// one). Used by the device-info popup to query the Lenovo PTSTPD
    /// API. Reset to empty whenever the device disconnects so a stale
    /// serial does not bleed across hardware swaps mid-session.
    pub(crate) serial: String,
    pub(crate) platform_supported: Option<bool>, // None = unknown, Some(true) = qcom, Some(false) = unsupported
}

/// Loading state for the device-info popup. The popup view branches on
/// this to render a progress indicator / table / error banner while keeping the
/// modal open so the user has a clear target to dismiss.
#[derive(Debug, Clone)]
pub(crate) enum DeviceInfoState {
    /// Fetch is in flight; render a progress indicator and disable retry.
    Loading,
    /// `device_info_cache[serial]` is populated; render the table.
    Ready,
    /// Fetch failed; render the message + a retry pill.
    Error(String),
}

/// Loading state for the firmware-OTA popup. Mirrors `DeviceInfoState`
/// but adds a `NoUpdate` arm — the upstream `<firmwareupdate/>` empty
/// payload means "no OTA staged for this firmware id" and renders as a
/// single placeholder line, not as an error banner.
#[derive(Debug, Clone)]
pub(crate) enum OtaPopupState {
    Loading,
    NoUpdate,
    Ready(ltbox_core::lenovo_ota::OtaUpdate),
    Error(String),
}

/// Worker result for the QFIL-firmware lookup: a global (non-CN) device, a CN
/// device whose MTM has no published package, or the resolved package.
#[derive(Debug, Clone)]
pub(crate) enum QfilOutcome {
    /// `SaleArea != CN` — point the user at Lenovo Software Fix instead.
    Global,
    /// CN device, but the MTM resolved to no flashing-machine package.
    NoPackage,
    /// Resolved official QFIL package.
    Package(ltbox_core::lenovo_qfil::QfilPackage),
}

/// Loading state for the QFIL-firmware popup. Mirrors [`OtaPopupState`] with a
/// `Global` arm (non-CN device) and a `NoPackage` arm (CN, MTM unmatched).
#[derive(Debug, Clone)]
pub(crate) enum QfilPopupState {
    Loading,
    Global,
    NoPackage,
    Ready(ltbox_core::lenovo_qfil::QfilPackage),
    Error(String),
}

/// Parse hwboardid: `"SM8750P_16+512_13"` → `("16 GB", "512 GB")`.
pub(crate) fn parse_hwboardid_ram_storage(hwboardid: &str) -> (String, String) {
    let parts: Vec<&str> = hwboardid.split('_').collect();
    for part in &parts {
        if let Some((ram, storage)) = part.split_once('+')
            && ram.chars().all(|c| c.is_ascii_digit())
            && storage.chars().all(|c| c.is_ascii_digit())
        {
            return (
                ltbox_core::tr_args!("unit_gigabytes", value = ram.to_string()),
                ltbox_core::tr_args!("unit_gigabytes", value = storage.to_string()),
            );
        }
    }
    (String::new(), String::new())
}

/// How the rollback-index popup renders a stored floor. Clicking a value
/// steps to the next form and wraps back around.
///
/// A rollback index is a unix timestamp, but fastboot reports it base-16
/// (`stored_rollback_index:N = 41B7A200`), so the raw form a user sees in
/// `fastboot getvar all` is hex. The cycle walks outward from that raw
/// value to progressively more readable renderings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum RollbackValueFormat {
    /// As `fastboot getvar all` prints it — base-16.
    #[default]
    Raw,
    /// The same number in decimal, i.e. a plain unix timestamp.
    Unix,
    /// `YYYY-MM-DD`, UTC.
    Date,
}

impl RollbackValueFormat {
    pub(crate) fn next(self) -> Self {
        match self {
            Self::Raw => Self::Unix,
            Self::Unix => Self::Date,
            Self::Date => Self::Raw,
        }
    }

    /// Render `index` in this form. The returned string is exactly what
    /// the copy button puts on the clipboard.
    pub(crate) fn render(self, index: u64) -> String {
        match self {
            Self::Raw => format!("0x{index:X}"),
            Self::Unix => index.to_string(),
            Self::Date => format_unix_date_utc(index),
        }
    }

    /// i18n key naming the current form, shown beside the value so the
    /// cycle is self-explanatory rather than a guessing game.
    pub(crate) const fn label_key(self) -> &'static str {
        match self {
            Self::Raw => "rollback_format_raw",
            Self::Unix => "rollback_format_unix",
            Self::Date => "rollback_format_date",
        }
    }

    /// Parse user input using the same convention used for rendering:
    /// raw is `0x…` hexadecimal, Unix is decimal, and Date is a UTC
    /// calendar day represented at midnight.
    pub(crate) fn parse(self, input: &str) -> Result<u64, String> {
        let trimmed = input.trim();
        match self {
            Self::Raw => {
                let digits = trimmed
                    .strip_prefix("0x")
                    .or_else(|| trimmed.strip_prefix("0X"))
                    .ok_or_else(|| "rollback_manual_error_prefix".to_string())?;
                u64::from_str_radix(digits, 16).map_err(|_| "rollback_manual_error_hex".to_string())
            }
            Self::Unix => trimmed
                .parse::<u64>()
                .map_err(|_| "rollback_manual_error_decimal".to_string()),
            Self::Date => {
                let bytes = trimmed.as_bytes();
                if trimmed.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
                    return Err("rollback_manual_error_date_shape".to_string());
                }
                let (year, rest) = trimmed.split_at(4);
                if rest.len() != 6 || !rest.starts_with('-') {
                    return Err("rollback_manual_error_date_shape".to_string());
                }
                let month = &rest[1..3];
                let day = &rest[4..6];
                let year: i32 = year
                    .parse()
                    .map_err(|_| "rollback_manual_error_date_value".to_string())?;
                let month: u32 = month
                    .parse()
                    .ok()
                    .filter(|month| (1..=12).contains(month))
                    .ok_or_else(|| "rollback_manual_error_date_value".to_string())?;
                let day: u32 = day
                    .parse()
                    .ok()
                    .filter(|day| (1..=31).contains(day))
                    .ok_or_else(|| "rollback_manual_error_date_value".to_string())?;

                let days_since_epoch = civil_from_days_ordinal(year, month, day)
                    .ok_or_else(|| "rollback_manual_error_date_value".to_string())?;
                let timestamp = days_since_epoch * 86_400;
                u64::try_from(timestamp).map_err(|_| "rollback_manual_error_date_value".to_string())
            }
        }
    }
}

/// Convert a proleptic Gregorian UTC date to days since the Unix epoch.
/// Returns `None` for impossible dates such as February 30.
pub(crate) fn civil_from_days_ordinal(year: i32, month: u32, day: u32) -> Option<i64> {
    let date = chrono::NaiveDate::from_ymd_opt(year, month, day)?;
    Some(
        date.signed_duration_since(chrono::DateTime::UNIX_EPOCH.date_naive())
            .num_days(),
    )
}

/// Current Unix timestamp in whole seconds.
pub(crate) fn current_unix_timestamp() -> Option<u64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs())
}

/// Rollback-protection answer for a model, or `""` when the model is
/// unknown.
///
/// `is_rollback_protected_model` is a deny-list (only TB322FC is exempt), so
/// an unhandled empty model would read as protected — the Dashboard would
/// assert "Yes" for a device it can't identify instead of falling back to
/// the em dash every other field shows for an empty string.
pub(crate) fn arb_from_model(model: &str) -> &'static str {
    if model.trim().is_empty() {
        ""
    } else if is_rollback_protected_model(model) {
        "arb_yes"
    } else {
        "arb_no"
    }
}

/// Normalize an optional fastboot `current-slot` to a partition suffix
/// (`_a`/`_b`), defaulting to `_a` when unknown (e.g. EDL-start with no
/// fastboot probe).
pub(crate) fn active_slot_suffix(slot: Option<&str>) -> &'static str {
    match slot {
        Some(s) if s.eq_ignore_ascii_case("_b") || s.eq_ignore_ascii_case("b") => "_b",
        _ => "_a",
    }
}

/// Route device into EDL (Qualcomm 9008). Shared by Root/Unroot/Flash.
///
/// Already-EDL: no-op. Fastboot live: continue system boot, wait for ADB,
/// then `adb reboot edl`. ADB live: `adb reboot edl`. If ADB is not
/// usable, ask the user to reboot manually and wait for 9008.
///
/// `conn` is the caller's captured `App.connection`, used only as a
/// fallback. The body re-probes EDL → Fastboot → ADB live because flows
/// (e.g. Flash) may reboot the device themselves between worker spawn
/// and the EDL transition (ADB → bootloader for variable query), making
/// the captured `conn` stale.
pub(crate) fn transition_to_edl(
    conn: ConnectionStatus,
    log: &mut Vec<String>,
) -> std::result::Result<(), String> {
    ltbox_device::selection::ensure_single_usb_target().map_err(|e| e.to_string())?;
    let live = probe_connection_for_edl().unwrap_or(conn);
    ensure_edl(live, "EDL", log).map_err(|()| ltbox_core::i18n::tr("err_edl_transition_failed"))
}

/// Quick EDL/Fastboot/ADB probe in that order. Returns `None` only when
/// every transport is silent (caller falls back to its captured conn).
pub(crate) fn probe_connection_for_edl() -> Option<ConnectionStatus> {
    if ltbox_device::edl::check_device() {
        return Some(ConnectionStatus::Edl);
    }
    if ltbox_device::fastboot::FastbootDevice::check_device() {
        return Some(ConnectionStatus::Fastboot);
    }
    let mut adb = ltbox_device::adb::AdbManager::new();
    match adb.check_device_state().ok().flatten() {
        Some("device" | "recovery") => Some(ConnectionStatus::Adb),
        Some("adb_server_blocking") => Some(ConnectionStatus::AdbServerBlocking),
        Some("unauthorized" | "authorizing") => Some(ConnectionStatus::AdbUnauthorized),
        Some("sideload") => Some(ConnectionStatus::AdbSideload),
        _ => None,
    }
}

/// Wrap a heavy blocking flow as a `Task<Message>`. Runs `f` on the
/// 64 MiB heavy-task pool via `spawn_blocking + run_heavy`, then sends
/// the result through `done`. Both `run_heavy` panics and the
/// `spawn_blocking` JoinError collapse to a single error string passed
/// to `fallback`, sparing callers a two-level `unwrap_or_else` chain.
pub(crate) fn task_heavy<T, F, G, D>(f: F, done: D, fallback: G) -> Task<Message>
where
    F: FnOnce() -> T + Send + 'static,
    G: FnOnce(String) -> T + Send + 'static,
    D: FnOnce(T) -> Message + Send + 'static,
    T: Send + 'static,
{
    Task::perform(
        async move {
            match tokio::task::spawn_blocking(move || ltbox_core::runtime::run_heavy(f)).await {
                Ok(Ok(v)) => v,
                Ok(Err(e)) => fallback(e),
                Err(_) => fallback("task panicked".to_string()),
            }
        },
        done,
    )
}

/// Map a PTSTPD `MachineInfo`'s `SaleArea` to a flash region: `"CN"` → PRC,
/// JSON `null` → ROW, anything else (or missing) → `None` (can't infer).
pub(crate) fn region_from_salearea(
    info: &ltbox_core::lenovo_info::MachineInfo,
) -> Option<DeviceRegion> {
    match info.field("SaleArea") {
        ltbox_core::lenovo_info::FieldValue::Value(s) if s.eq_ignore_ascii_case("CN") => {
            Some(DeviceRegion::Prc)
        }
        ltbox_core::lenovo_info::FieldValue::Null => Some(DeviceRegion::Row),
        _ => None,
    }
}

/// Resolve the QFIL-firmware outcome for a serial (blocking; runs in the
/// worker). `cached` supplies MTM + SaleArea when already known; otherwise
/// machine info is fetched here. Non-CN `SaleArea` short-circuits to
/// [`QfilOutcome::Global`]; a CN device queries the official package.
pub(crate) fn resolve_qfil(
    serial: &str,
    cached: Option<(String, String)>,
) -> Result<QfilOutcome, String> {
    use ltbox_core::lenovo_info::FieldValue;
    let (mtm, area) = match cached {
        Some(t) => t,
        None => {
            let info =
                ltbox_core::lenovo_info::fetch_machine_info(serial).map_err(|e| e.to_string())?;
            let field = |k: &str| match info.field(k) {
                FieldValue::Value(s) => s,
                _ => String::new(),
            };
            (field("MTM"), field("SaleArea"))
        }
    };
    // Global (non-CN) devices have no PTSTPD flashing-machine entry.
    if !area.eq_ignore_ascii_case("CN") {
        return Ok(QfilOutcome::Global);
    }
    if mtm.trim().is_empty() {
        return Ok(QfilOutcome::NoPackage);
    }
    match ltbox_core::lenovo_qfil::fetch_qfil_package(&mtm).map_err(|e| e.to_string())? {
        Some(pkg) => Ok(QfilOutcome::Package(pkg)),
        None => Ok(QfilOutcome::NoPackage),
    }
}
