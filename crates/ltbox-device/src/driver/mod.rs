//! Cross-platform device-driver / udev-rule status + installer.
//!
//! Used by the GUI's startup driver-check banner + the Settings
//! "Install Drivers" action. Public surface stays the same regardless
//! of host OS so the GUI never has to branch by `cfg!(windows)`; the
//! platform impl picks how to interpret `Present` / `Missing` /
//! `NotWindows` for the local OS.
//!
//! ## Platform impl mapping
//!
//! | Host    | Module                  | Strategy |
//! |---------|-------------------------|----------|
//! | Windows | `driver::windows`       | Mode-aware `pnputil /enum-drivers` + DriverStore probe (`qcserlib.inf` for userspace / `qcwdfser.inf` for kernel); install by downloading the selected signed per-arch Qualcomm `.exe` and launching it via UAC (`Start-Process -Verb RunAs`). The installer self-elevates, so LTBox itself does not need to run as Administrator. |
//! | Linux   | `driver::linux`         | Userspace mode detects the LTBox udev rules at `/etc/udev/rules.d/51-ltbox-qcom.rules` and installs them through `pkexec … --install-udev`. Kernel mode probes the `qud` Debian package and installs the latest `qud_*_all.zip` release through elevated `apt-get install`. |
//! | macOS   | `driver::unsupported`   | No-op `NotWindows` — macOS is forced to userspace mode because Qualcomm publishes no macOS kernel driver and libusb claims the device directly. |
//!
//! The shared `DriverStatus` / `DriverError` / `Result` types live here so
//! the GUI never branches by `cfg`; the per-OS module decides which variants
//! it can produce.

use std::sync::atomic::{AtomicU8, Ordering};

/// Qualcomm USB driver family LTBox should use for EDL.
///
/// `Userspace` uses WinUSB on Windows and direct USB plus udev rules on Linux.
/// `Kernel` uses Qualcomm's kernel driver packages and the qdl serial backend,
/// so it is unavailable on macOS. Windows and Debian-style Linux default to
/// `Kernel` (see [`kernel_mode_supported`]); other Linux distros and macOS
/// default to / are forced to `Userspace`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum QcomDriverMode {
    #[default]
    Userspace,
    Kernel,
}

impl QcomDriverMode {
    pub const fn code(self) -> &'static str {
        match self {
            Self::Userspace => "userspace",
            Self::Kernel => "kernel",
        }
    }

    pub fn from_code(code: &str) -> Self {
        match code {
            "kernel" => Self::Kernel,
            _ => Self::Userspace,
        }
    }

    pub const fn is_kernel(self) -> bool {
        matches!(self, Self::Kernel)
    }
}

/// Whether the Qualcomm kernel driver mode is usable on this host. Drives both
/// the first-run driver-mode default and the Settings UI gate (the picker is
/// locked to userspace when this is `false`). Windows ships the signed kernel
/// driver; Linux only automates the kernel-driver (`qud`) install on
/// Debian-style hosts where `dpkg-query` exists, so non-Debian distros are
/// userspace-only; macOS has no kernel driver at all. Mirrors the support gate
/// in the Linux backend's `check_kernel_driver`.
pub fn kernel_mode_supported() -> bool {
    #[cfg(windows)]
    {
        true
    }
    #[cfg(target_os = "linux")]
    {
        self::linux::dpkg_available()
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        false
    }
}

static QCOM_DRIVER_MODE: AtomicU8 = AtomicU8::new(0);

pub fn set_qcom_driver_mode(mode: QcomDriverMode) {
    QCOM_DRIVER_MODE.store(
        match mode {
            QcomDriverMode::Userspace => 0,
            QcomDriverMode::Kernel => 1,
        },
        Ordering::Relaxed,
    );
}

pub fn qcom_driver_mode() -> QcomDriverMode {
    match QCOM_DRIVER_MODE.load(Ordering::Relaxed) {
        1 => QcomDriverMode::Kernel,
        _ => QcomDriverMode::Userspace,
    }
}

/// Shape returned by [`check_required_drivers`]. Windows produces
/// `Present` / `Missing`; Linux produces `Present` / `UdevRules*`; macOS
/// (and other hosts) produce `NotWindows`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriverStatus {
    /// Host has no driver-action concept (macOS and other non-Windows,
    /// non-Linux hosts).
    NotWindows,
    /// Every required driver / rule is in place.
    Present,
    /// Windows: list of `.inf` filenames not yet installed.
    Missing(Vec<&'static str>),
    /// Linux: the LTBox udev rules file is not installed.
    UdevRulesMissing,
    /// Linux: a udev rules file is installed but its content differs from the
    /// rules LTBox bundles (an older LTBox wrote it, or the user edited it).
    UdevRulesStale,
    /// Linux: the udev rules file exists but could not be read to verify it
    /// (e.g. permission denied) — surfaced as a repairable state.
    UdevRulesNoPermission,
    /// Kernel-driver mode: the Qualcomm kernel driver package is not present.
    KernelDriverMissing,
    /// Kernel-driver mode is selected on a host where LTBox cannot automate it.
    KernelDriverUnsupported,
}

/// Result of comparing the locally-installed Qualcomm driver version
/// against the latest signed release on GitHub. Only produced when a
/// driver is already present AND a strictly-newer release exists — the
/// GUI uses its presence to decide whether to show the optional
/// "update available" banner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriverUpdate {
    /// Dotted version parsed from the selected installed driver package.
    pub current: String,
    /// Dotted version parsed from the latest selected driver release tag.
    pub latest: String,
}

#[derive(thiserror::Error, Debug)]
pub enum DriverError {
    #[error("Driver install is not supported on this host")]
    NotWindows,
    // ureq has no thiserror-friendly root error, so collapse transport + status.
    #[error("Network error: {0}")]
    Http(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Zip extraction error: {0}")]
    Zip(#[from] zip::result::ZipError),
    /// GitHub release JSON could not be parsed.
    #[error("GitHub release parse error: {0}")]
    Parse(String),
    /// The latest selected Qualcomm driver release shipped no installer
    /// asset for the host architecture / package type.
    #[error("No matching driver asset found in the latest release")]
    NoAsset,
    /// The user dismissed the elevation prompt (Windows UAC for the signed
    /// installer, or polkit for the Linux `pkexec --install-udev` call), so
    /// the privileged step never ran. Distinct from `InstallerFailed` so the
    /// GUI can say "approve the prompt and try again" instead of a generic
    /// error. Neither path needs LTBox itself to run elevated — the prompt is
    /// the only elevation step.
    #[error("Driver install was cancelled at the elevation prompt.")]
    InstallCancelled,
    /// The selected driver installer exited with a non-zero status.
    #[error("Driver installer exited with code {exit_code}.")]
    InstallerFailed { exit_code: i32 },
}

impl From<ureq::Error> for DriverError {
    fn from(e: ureq::Error) -> Self {
        DriverError::Http(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, DriverError>;

#[cfg(any(windows, target_os = "linux"))]
fn version_from_tag(tag: &str) -> Option<String> {
    let seg = tag.rsplit('-').next()?;
    let v = seg.strip_prefix(['v', 'V'])?;
    parse_version(v)?;
    Some(v.to_string())
}

#[cfg(any(windows, target_os = "linux"))]
fn parse_version(v: &str) -> Option<Vec<u64>> {
    v.split('.')
        .map(|p| {
            if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
                None
            } else {
                p.parse::<u64>().ok()
            }
        })
        .collect()
}

#[cfg(any(windows, target_os = "linux"))]
fn version_lt(a: &str, b: &str) -> bool {
    let (a, b) = (
        parse_version(a).unwrap_or_default(),
        parse_version(b).unwrap_or_default(),
    );
    let n = a.len().max(b.len());
    for i in 0..n {
        let (x, y) = (
            a.get(i).copied().unwrap_or(0),
            b.get(i).copied().unwrap_or(0),
        );
        if x != y {
            return x < y;
        }
    }
    false
}

#[cfg(all(test, any(windows, target_os = "linux")))]
mod version_tests {
    use super::{version_from_tag, version_lt};

    #[test]
    fn version_from_tag_extracts_dotted() {
        assert_eq!(
            version_from_tag("release-win-v1.0.2.0").as_deref(),
            Some("1.0.2.0")
        );
        assert_eq!(
            version_from_tag("release-lnx-v1.0.6.4").as_deref(),
            Some("1.0.6.4")
        );
        assert_eq!(version_from_tag("v2.3.4").as_deref(), Some("2.3.4"));
        assert_eq!(version_from_tag("release-linux").as_deref(), None);
        assert_eq!(version_from_tag("").as_deref(), None);
        assert_eq!(version_from_tag("release-win-v1..2").as_deref(), None);
        assert_eq!(version_from_tag("release-lnx-v1..6").as_deref(), None);
        assert_eq!(version_from_tag("release-win-v1.").as_deref(), None);
        assert_eq!(version_from_tag("release-win-v1.x").as_deref(), None);
        assert_eq!(version_from_tag("release-win-v1.+2").as_deref(), None);
        assert_eq!(version_from_tag("release-win-+1.2").as_deref(), None);
        assert_eq!(version_from_tag("release-win-v+1.2").as_deref(), None);
        assert_eq!(version_from_tag("release-win-v-1.2").as_deref(), None);
        assert_eq!(version_from_tag("release-win-1.0.2.0").as_deref(), None);
        assert_eq!(version_from_tag("release-lnx-1.0.6.4").as_deref(), None);
    }

    #[test]
    fn userspace_version_namespaces_do_not_mix() {
        assert!(version_lt("1.0.2.1", "1.0.2.2"));
        assert!(!version_lt("1.0.2.2", "1.0.2.2"));
        assert!(version_lt("1.0.2.2", "1.0.2.3"));
    }

    #[test]
    fn kernel_version_namespaces_do_not_mix() {
        assert!(version_lt("1.0.3.6", "1.00.94.6"));
        assert!(!version_lt("1.00.94.6", "1.00.94.6"));
        assert!(version_lt("1.00.94.6", "1.00.94.7"));
        assert!(!version_lt("1.00.94.6", "1.0.94.6"));
    }

    #[test]
    fn version_lt_compares_componentwise() {
        assert!(version_lt("1.0.1.0", "1.0.2.0"));
        assert!(version_lt("1.0", "1.0.0.1"));
        assert!(!version_lt("1.0.2.0", "1.0.2.0"));
        assert!(!version_lt("1.0.0", "1.0"));
        assert!(!version_lt("2.0.0.0", "1.9.9.9"));
    }
}

/// Best-effort reachability probe for the GitHub host LTBox downloads driver
/// assets from. Used to pre-disable install / update buttons (with an
/// "internet required" tooltip) instead of letting the user click into a
/// download that can only fail. A short timeout keeps a dead network from
/// stalling startup; any transport / non-2xx result reads as offline.
#[cfg(any(windows, target_os = "linux"))]
fn github_reachable() -> bool {
    // Bespoke agent: an 8s global timeout keeps a dead network from stalling
    // startup, so this probe does not reuse the shared pooled agent (which has
    // no global cap). It does reuse the shared user-agent string.
    let agent = ureq::Agent::config_builder()
        .user_agent(ltbox_core::downloader::USER_AGENT)
        .timeout_global(Some(std::time::Duration::from_secs(8)))
        .build()
        .new_agent();
    agent.get("https://api.github.com/").call().is_ok()
}

#[cfg(windows)]
pub fn probe_connectivity() -> bool {
    github_reachable()
}

#[cfg(target_os = "linux")]
pub fn probe_connectivity() -> bool {
    if qcom_driver_mode().is_kernel() {
        github_reachable()
    } else {
        true
    }
}

/// macOS has no driver download path, so do not spend startup time probing
/// GitHub just to gate a button that will never be shown.
#[cfg(not(any(windows, target_os = "linux")))]
pub fn probe_connectivity() -> bool {
    true
}

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use self::windows::{check_driver_update, check_required_drivers, download_and_install};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use self::linux::{check_driver_update, check_required_drivers, download_and_install};

// macOS (and any other non-Windows, non-Linux target) has no driver / udev-rule
// concept — a no-op stub keeps the public surface identical so the GUI never
// branches by `cfg`.
#[cfg(not(any(windows, target_os = "linux")))]
mod unsupported;
#[cfg(not(any(windows, target_os = "linux")))]
pub use self::unsupported::{check_driver_update, check_required_drivers, download_and_install};

/// Canonical LTBox udev rules, embedded so the Linux probe can tell an
/// up-to-date install apart from a missing, stale, or hand-edited one. Same
/// source file the GUI's `--install-udev` writer embeds.
#[cfg(any(target_os = "linux", test))]
pub(crate) const UDEV_RULES: &str = include_str!("../../../../misc/udev/51-ltbox-qcom.rules");

/// Classify installed udev rules against [`UDEV_RULES`]. `installed` is the
/// file's content (`None` = file absent). Pure, so it is unit-tested on any
/// host even though the filesystem read that feeds it is Linux-only.
#[cfg(any(target_os = "linux", test))]
pub(crate) fn classify_udev_rules(installed: Option<&str>) -> DriverStatus {
    match installed {
        None => DriverStatus::UdevRulesMissing,
        Some(content) if content == UDEV_RULES => DriverStatus::Present,
        Some(_) => DriverStatus::UdevRulesStale,
    }
}

/// Classify the effective LTBox rules across udev's administrator and vendor
/// package directories. A same-named file in `/etc/udev/rules.d` overrides the
/// packaged copy in `/usr/lib/udev/rules.d`, so its content wins when present.
#[cfg(any(target_os = "linux", test))]
pub(crate) fn classify_udev_rule_locations(
    admin_rules: Option<&str>,
    package_rules: Option<&str>,
) -> DriverStatus {
    classify_udev_rules(admin_rules.or(package_rules))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn driver_mode_codes_round_trip() {
        assert_eq!(QcomDriverMode::from_code("kernel"), QcomDriverMode::Kernel);
        assert_eq!(
            QcomDriverMode::from_code("userspace"),
            QcomDriverMode::Userspace
        );
        assert_eq!(
            QcomDriverMode::from_code("unknown"),
            QcomDriverMode::Userspace
        );
        assert_eq!(QcomDriverMode::Kernel.code(), "kernel");
        assert_eq!(QcomDriverMode::Userspace.code(), "userspace");
    }

    #[test]
    fn classify_udev_rules_states() {
        assert_eq!(classify_udev_rules(None), DriverStatus::UdevRulesMissing);
        assert_eq!(classify_udev_rules(Some(UDEV_RULES)), DriverStatus::Present);
        assert_eq!(
            classify_udev_rules(Some("# hand-edited or older rules\n")),
            DriverStatus::UdevRulesStale
        );
    }

    /// systemd-udevd discards a whole rule, `TAG+="uaccess"` included, when
    /// its `GROUP=` names a group the host lacks (no `plugdev` on Arch), so
    /// the shipped rules must not name one. Issue #119.
    #[test]
    fn shipped_udev_rules_grant_uaccess_without_naming_a_group() {
        let rules: Vec<String> = UDEV_RULES
            .replace("\r\n", "\n")
            .replace("\\\n", " ")
            .lines()
            .filter(|line| !line.trim_start().starts_with('#') && !line.trim().is_empty())
            .map(str::to_owned)
            .collect();
        assert_eq!(rules.len(), 4, "{rules:#?}");
        for rule in &rules {
            assert!(rule.contains("TAG+=\"uaccess\""), "{rule}");
            assert!(!rule.contains("GROUP="), "{rule}");
        }
    }

    #[test]
    fn packaged_udev_rules_are_present_without_an_admin_copy() {
        assert_eq!(
            classify_udev_rule_locations(None, Some(UDEV_RULES)),
            DriverStatus::Present
        );
        assert_eq!(
            classify_udev_rule_locations(Some("# stale admin override\n"), Some(UDEV_RULES)),
            DriverStatus::UdevRulesStale
        );
    }
}
