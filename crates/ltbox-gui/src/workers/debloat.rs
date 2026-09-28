//! Debloat worker: remove or restore catalogued apps for user 0 over ADB.
//!
//! Every change is reversible: uninstalled apps keep their APK on the system
//! partition (`install-existing` brings them back) and disabled apps only
//! change state.

use crate::debloat::{DebloatMethod, PackageState, is_valid_package_id, package_states};
use crate::{
    ConnectionStatus, DebloatAction, DebloatTarget, PhaseReporter, package_reinstall_succeeded,
};
use ltbox_core::tr_args;
use std::collections::BTreeMap;

/// What one package command did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PackageOutcome {
    Applied,
    /// The app was already in the requested state.
    Unchanged,
    Failed,
}

fn uninstall_outcome(output: &str) -> PackageOutcome {
    let output = output.trim();
    if output == "Success" {
        PackageOutcome::Applied
    } else if output.contains("not installed for") {
        PackageOutcome::Unchanged
    } else {
        PackageOutcome::Failed
    }
}

fn state_change_outcome(output: &str, package: &str, state: &str) -> PackageOutcome {
    let expected = format!("Package {package} new state: {state}");
    if output.lines().any(|line| line.trim() == expected) {
        PackageOutcome::Applied
    } else {
        PackageOutcome::Failed
    }
}

/// `disable-user` on a package user 0 no longer has (uninstalled for the
/// user, or absent from this firmware) leaves nothing to disable.
fn disable_outcome(output: &str, package: &str) -> PackageOutcome {
    if output.contains(&format!("Unknown package: {package}")) {
        PackageOutcome::Unchanged
    } else {
        state_change_outcome(output, package, "disabled-user")
    }
}

/// One line for the log: the exception message rather than the Java stack
/// trace `pm` prints under it.
fn concise_pm_error(output: &str) -> String {
    let mut lines = output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());
    let first = lines.next().unwrap_or_default();
    let reason = std::iter::once(first)
        .chain(lines)
        .find(|line| line.starts_with("java.") || line.starts_with("Error:"))
        .unwrap_or(first);
    reason
        .split_once("Exception: ")
        .map_or(reason, |(_, message)| message)
        .to_string()
}

/// The `pm` command one target needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PackageCommand {
    Uninstall,
    Disable,
    InstallExisting,
    Enable,
}

impl PackageCommand {
    /// Removal follows the catalogue's method; restoring follows the state the
    /// app was read in, so an app a method-agnostic tool uninstalled comes back
    /// even when the catalogue only disables it.
    fn for_target(action: DebloatAction, target: &DebloatTarget) -> Self {
        match (action, target.package.method, target.state) {
            (DebloatAction::Remove, DebloatMethod::Uninstall, _) => Self::Uninstall,
            (DebloatAction::Remove, DebloatMethod::Disable, _) => Self::Disable,
            (DebloatAction::Restore, _, Some(PackageState::Removed)) => Self::InstallExisting,
            (DebloatAction::Restore, _, Some(PackageState::Disabled)) => Self::Enable,
            (DebloatAction::Restore, DebloatMethod::Uninstall, _) => Self::InstallExisting,
            (DebloatAction::Restore, DebloatMethod::Disable, _) => Self::Enable,
        }
    }

    fn shell(self, id: &str) -> String {
        match self {
            Self::Uninstall => format!("pm uninstall -k --user 0 {id}"),
            Self::Disable => format!("pm disable-user --user 0 {id}"),
            Self::InstallExisting => format!("cmd package install-existing {id}"),
            Self::Enable => format!("pm enable {id}"),
        }
    }

    fn outcome(self, output: &str, id: &str) -> PackageOutcome {
        match self {
            Self::Uninstall => uninstall_outcome(output),
            Self::Disable => disable_outcome(output, id),
            Self::InstallExisting if package_reinstall_succeeded(output, id) => {
                PackageOutcome::Applied
            }
            Self::InstallExisting => PackageOutcome::Failed,
            Self::Enable => state_change_outcome(output, id, "enabled"),
        }
    }
}

/// Bring the device to a running Android shell. Fastboot can hand over with
/// `fastboot continue`; EDL has no automatic way back to system.
fn connect_adb(
    conn: ConnectionStatus,
    log: &mut Vec<String>,
) -> Result<ltbox_device::adb::AdbManager, String> {
    if matches!(conn, ConnectionStatus::Fastboot) {
        ltbox_core::live!(
            log,
            "[Debloat] {}",
            ltbox_core::i18n::tr("live_sysupdate_fastboot_to_adb")
        );
        if let Ok(mut dev) = ltbox_device::fastboot::FastbootDevice::open() {
            let _ = dev.reboot();
        }
    }
    let mut adb = ltbox_device::adb::AdbManager::new();
    ltbox_core::live!(
        log,
        "[ADB] {}",
        ltbox_core::i18n::tr("live_adb_checking_device")
    );
    if !adb.check_device().unwrap_or(false) {
        if matches!(conn, ConnectionStatus::Fastboot) {
            if let Err(e) = adb.wait_for_device() {
                return Err(tr_args!("err_debloat_no_adb", error = e.to_string()));
            }
        } else {
            return Err(tr_args!("err_debloat_no_adb", error = "device not in ADB"));
        }
    }
    ltbox_core::live!(
        log,
        "[ADB] {}",
        ltbox_core::i18n::tr("live_adb_device_connected")
    );
    Ok(adb)
}

/// Result of reading where each catalogued app stands on the device.
#[derive(Debug, Clone)]
pub(crate) struct DebloatScanResult {
    pub(crate) logs: Vec<String>,
    pub(crate) states: Option<BTreeMap<String, PackageState>>,
    pub(crate) error: Option<String>,
}

/// Read user 0's package listings so the Apps step offers only apps the
/// chosen action would change. Read-only; needs Android running.
pub(crate) fn debloat_scan(conn: ConnectionStatus, ids: Vec<String>) -> DebloatScanResult {
    let mut logs = Vec::new();
    ltbox_core::live!(
        logs,
        "[Debloat] {}",
        ltbox_core::i18n::tr("live_debloat_reading_states")
    );
    let read = (|| {
        if conn != ConnectionStatus::Adb {
            return Err(tr_args!("err_debloat_no_adb", error = "device not in ADB"));
        }
        let mut adb = ltbox_device::adb::AdbManager::new();
        let mut list = |flags: &str| {
            adb.shell(&format!("pm list packages {flags} --user 0"))
                .map_err(|e| tr_args!("err_debloat_no_adb", error = e.to_string()))
        };
        let all = list("-u")?;
        let installed = list("")?;
        let disabled = list("-d")?;
        Ok(package_states(
            ids.iter().map(String::as_str),
            &all,
            &installed,
            &disabled,
        ))
    })();
    match read {
        Ok(states) => DebloatScanResult {
            logs,
            states: Some(states),
            error: None,
        },
        Err(error) => DebloatScanResult {
            logs,
            states: None,
            error: Some(error),
        },
    }
}

pub(crate) fn debloat_worker(
    action: DebloatAction,
    targets: Vec<DebloatTarget>,
    conn: ConnectionStatus,
    phases: PhaseReporter,
) -> Result<Vec<String>, String> {
    let mut log = Vec::new();
    ltbox_core::live!(log, "[Debloat] {}", phases.marker(1));
    let mut adb = connect_adb(conn, &mut log)?;

    ltbox_core::live!(log, "[Debloat] {}", phases.marker(2));
    phases.mark_writes_started();
    let mut succeeded = 0;
    for target in &targets {
        let id = target.package.id.as_str();
        // The catalogue test already guarantees this; the worker still
        // refuses anything that could turn into shell syntax.
        if !is_valid_package_id(id) {
            ltbox_core::live!(
                log,
                "[ADB] {}",
                tr_args!("live_debloat_invalid_package", package = id)
            );
            continue;
        }
        let command = PackageCommand::for_target(action, target);
        let result = adb.shell(&command.shell(id)).map_err(|e| e.to_string());
        let outcome = match &result {
            Ok(out) => command.outcome(out, id),
            Err(_) => PackageOutcome::Failed,
        };
        let error = match &result {
            Ok(out) => concise_pm_error(out),
            Err(e) => e.clone(),
        };
        let line = match (outcome, command) {
            (PackageOutcome::Unchanged, _) => {
                tr_args!("live_debloat_already_removed", package = id)
            }
            (PackageOutcome::Applied, PackageCommand::Uninstall) => {
                tr_args!("live_adb_uninstalled", package = id)
            }
            (PackageOutcome::Failed, PackageCommand::Uninstall) => {
                tr_args!("live_adb_uninstall_failed", package = id, error = error)
            }
            (PackageOutcome::Applied, PackageCommand::Disable) => {
                tr_args!("live_debloat_disabled", package = id)
            }
            (PackageOutcome::Failed, PackageCommand::Disable) => {
                tr_args!("live_debloat_disable_failed", package = id, error = error)
            }
            (PackageOutcome::Applied, PackageCommand::InstallExisting) => {
                tr_args!("live_adb_reinstalled", package = id)
            }
            (PackageOutcome::Failed, PackageCommand::InstallExisting) => {
                tr_args!("live_adb_reinstall_failed", package = id, error = error)
            }
            (PackageOutcome::Applied, PackageCommand::Enable) => {
                tr_args!("live_debloat_enabled", package = id)
            }
            (PackageOutcome::Failed, PackageCommand::Enable) => {
                tr_args!("live_debloat_enable_failed", package = id, error = error)
            }
        };
        ltbox_core::live!(log, "[ADB] {}", line);
        if outcome != PackageOutcome::Failed {
            succeeded += 1;
        }
    }
    ltbox_core::live!(
        log,
        "[Debloat] {}",
        tr_args!(
            "live_debloat_done",
            success = succeeded,
            total = targets.len()
        )
    );
    if succeeded == 0 && !targets.is_empty() {
        return Err(ltbox_core::i18n::tr("err_debloat_nothing_applied"));
    }
    Ok(log)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_follows_the_state_the_app_was_read_in() {
        let package = crate::debloat::DebloatPackage {
            id: "com.zui.browser".into(),
            label: "ZUI Browser".into(),
            method: DebloatMethod::Disable,
            recommended: true,
        };
        let target = |state| DebloatTarget {
            package: package.clone(),
            state,
        };
        assert_eq!(
            PackageCommand::for_target(
                DebloatAction::Restore,
                &target(Some(PackageState::Removed))
            ),
            PackageCommand::InstallExisting
        );
        assert_eq!(
            PackageCommand::for_target(
                DebloatAction::Restore,
                &target(Some(PackageState::Disabled))
            ),
            PackageCommand::Enable
        );
        assert_eq!(
            PackageCommand::for_target(DebloatAction::Restore, &target(None)),
            PackageCommand::Enable
        );
        assert_eq!(
            PackageCommand::for_target(
                DebloatAction::Remove,
                &target(Some(PackageState::Installed))
            ),
            PackageCommand::Disable
        );
    }

    #[test]
    fn uninstall_output_separates_done_absent_and_failed() {
        assert_eq!(uninstall_outcome("Success\r\n"), PackageOutcome::Applied);
        assert_eq!(
            uninstall_outcome("Failure [not installed for 0]"),
            PackageOutcome::Unchanged
        );
        for output in ["Failure [DELETE_FAILED_INTERNAL_ERROR]", "", "Successful"] {
            assert_eq!(
                uninstall_outcome(output),
                PackageOutcome::Failed,
                "{output}"
            );
        }
    }

    #[test]
    fn disabling_a_package_user_0_lacks_is_not_a_failure() {
        let output = "Exception occurred while executing 'disable-user':\n\
            java.lang.IllegalArgumentException: Unknown package: com.zui.browser\n\
            \tat com.android.server.pm.PackageManagerService.setEnabledSettings(PackageManagerService.java:4163)";
        assert_eq!(
            disable_outcome(output, "com.zui.browser"),
            PackageOutcome::Unchanged
        );
        assert_eq!(
            disable_outcome(output, "com.zui.contacts"),
            PackageOutcome::Failed
        );
        assert_eq!(concise_pm_error(output), "Unknown package: com.zui.browser");
        assert_eq!(
            concise_pm_error("Failure [DELETE_FAILED_INTERNAL_ERROR]\n"),
            "Failure [DELETE_FAILED_INTERNAL_ERROR]"
        );
        assert_eq!(concise_pm_error(""), "");
    }

    #[test]
    fn state_change_requires_the_named_package_and_state() {
        let package = "com.zui.browser";
        assert_eq!(
            state_change_outcome(
                "Package com.zui.browser new state: disabled-user\n",
                package,
                "disabled-user"
            ),
            PackageOutcome::Applied
        );
        for output in [
            "Package com.zui.browser new state: enabled",
            "Package com.zui.other new state: disabled-user",
            "Exception occurred while executing 'disable-user': Unknown package: com.zui.browser",
            "",
        ] {
            assert_eq!(
                state_change_outcome(output, package, "disabled-user"),
                PackageOutcome::Failed,
                "{output}"
            );
        }
    }
}
