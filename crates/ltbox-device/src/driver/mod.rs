//! Cross-platform device-driver / udev-rule status + installer.
//!
//! The public surface is identical on every host so the GUI never branches
//! by `cfg`; each per-OS module decides which `DriverStatus` variants it
//! produces.
//!
//! | Host    | Module                  | Strategy |
//! |---------|-------------------------|----------|
//! | Windows | `driver::windows`       | Mode-aware `pnputil /enum-drivers` + DriverStore probe (`qcserlib.inf` userspace / `qcwdfser.inf` kernel); installs the signed per-arch Qualcomm `.exe` through a UAC prompt, so LTBox itself never runs elevated. |
//! | Linux   | `driver::linux`         | Userspace mode checks the LTBox udev rules (`/etc/udev/rules.d`, else `/usr/lib/udev/rules.d`) and installs them via `pkexec … --install-udev`. Kernel mode probes the `qud` Debian package and installs the latest `qud_*_all.zip` release via `pkexec apt-get install`. |
//! | macOS   | `driver::unsupported`   | No-op `NotWindows`: no Qualcomm kernel driver exists, so macOS is userspace-only and claims the device directly. |

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

/// Whether the Qualcomm kernel driver mode is usable on this host; drives the
/// first-run default and locks the Settings picker to userspace when `false`.
/// Windows: always. Linux: only where `dpkg-query` and a trusted `apt-get`
/// exist (same gate as `check_kernel_driver`). macOS: never.
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

/// Installed vs latest Qualcomm driver version; only produced when a driver
/// is present and a strictly newer release exists (the GUI's update banner).
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
    /// Release data, signature check or archive contents were rejected.
    #[error("GitHub release parse error: {0}")]
    Parse(String),
    /// The latest selected Qualcomm driver release shipped no installer
    /// asset for the host architecture / package type.
    #[error("No matching driver asset found in the latest release")]
    NoAsset,
    /// The user dismissed the elevation prompt (Windows UAC or Linux polkit),
    /// so the privileged step never ran. Distinct from `InstallerFailed` so the
    /// GUI can ask the user to approve the prompt and retry.
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

/// Best-effort GitHub reachability, used to pre-disable driver install /
/// update buttons. Any transport or non-2xx result reads as offline.
#[cfg(any(windows, target_os = "linux"))]
fn github_reachable() -> bool {
    // Own agent: the shared pooled one has no global timeout, and a dead
    // network must not stall startup.
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

/// macOS has no driver download, so there is nothing to gate.
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

// Other hosts get a no-op stub with the same public surface.
#[cfg(not(any(windows, target_os = "linux")))]
mod unsupported;
#[cfg(not(any(windows, target_os = "linux")))]
pub use self::unsupported::{check_driver_update, check_required_drivers, download_and_install};

/// Canonical LTBox udev rules (the same file `--install-udev` writes), so the
/// probe can tell a current install from a missing or stale one.
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
