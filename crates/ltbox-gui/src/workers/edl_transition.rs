//! EDL-entry helpers: route the device into EDL/9008 from ADB,
//! Fastboot, or a manual trigger.

use crate::{ConnectionStatus, EdlEntryAction, edl_entry_action};
use ltbox_core::tr_args;
use ltbox_device::adb::AdbError;
use ltbox_device::fastboot::FastbootError;
use ltbox_device::selection::{DeviceMode, UsbSelectionError};

/// Which device to talk to is unsettled: abort, never wait for a manual EDL
/// reboot of some device.
fn is_adb_selection_error(error: &AdbError) -> bool {
    matches!(
        error,
        AdbError::MultipleDevices
            | AdbError::AmbiguousTarget
            | AdbError::TargetMismatch { .. }
            | AdbError::Selection(_)
    )
}

pub(crate) fn is_fastboot_selection_error(error: &FastbootError) -> bool {
    matches!(
        error,
        FastbootError::MultipleDevices
            | FastbootError::AmbiguousTarget
            | FastbootError::TargetMismatch { .. }
            | FastbootError::Selection(_)
    )
}

/// Whether the target has entered EDL, for the ADB wait after a Fastboot reboot.
/// Without a target this is `edl::check_device()` as before. With one it is the
/// EDL resolve; a refused proof is stored in `refused` and ends the wait at once
/// instead of reading as "not yet".
fn target_reached_edl(targeted: bool, refused: &mut Option<String>) -> bool {
    if !targeted {
        return ltbox_device::edl::check_device();
    }
    match ltbox_device::selection::resolve(DeviceMode::Edl) {
        Ok(found) => found.is_some(),
        Err(UsbSelectionError::Enumeration(error)) => {
            tracing::debug!("USB enumeration failed while waiting for EDL: {error}");
            false
        }
        Err(error) => {
            *refused = Some(error.to_string());
            true
        }
    }
}

/// Wait for the EDL (9008) port to appear and log the outcome. `Err` means it never did; the
/// caller aborts after the no-port notice.
pub(crate) fn wait_for_edl_ready(tag: &str, log: &mut Vec<String>) -> Result<(), ()> {
    ltbox_core::live!(
        log,
        "[{tag}] {}",
        ltbox_core::i18n::tr("live_wait_edl_port")
    );
    match ltbox_device::edl::wait_for_device() {
        Ok(_) => {
            ltbox_core::live!(log, "[{tag}] {}", ltbox_core::i18n::tr("live_edl_ready"));
            Ok(())
        }
        Err(e) => {
            // Localize the common wait failures so the detail isn't raw
            // English; rarer variants keep their Display text.
            let detail = match &e {
                ltbox_device::edl::EdlError::PortTimeout(d) => {
                    tr_args!("live_edl_wait_timeout", seconds = d.as_secs().to_string())
                }
                ltbox_device::edl::EdlError::PortNotFound => {
                    ltbox_core::i18n::tr("live_edl_port_not_found")
                }
                other => other.to_string(),
            };
            ltbox_core::live!(
                log,
                "[{tag}] {}",
                tr_args!("live_edl_not_found", error = detail)
            );
            // Only the reboot / manual-wait paths reach here, so the device was sent toward EDL but its port
            // never appeared. No write has happened yet; reassure the user before the caller aborts.
            ltbox_core::live!(
                log,
                "{}",
                ltbox_core::i18n::tr("live_edl_reboot_no_port_notice")
            );
            Err(())
        }
    }
}

/// Open the first EDL session after a successful transition. On failure the
/// device reached EDL and its port was found, but the Sahara/Firehose handshake
/// did not complete — nothing has been written yet, so reassure the user and
/// point them to a manual reboot (mirrors `wait_for_edl_ready`'s no-port notice)
/// before the caller aborts. Use for the FIRST session open of any operation.
pub(crate) fn open_edl_session(
    loader: &std::path::Path,
    log: &mut Vec<String>,
) -> Result<ltbox_device::edl::EdlSession, String> {
    match ltbox_device::edl::EdlSession::open(loader, log) {
        Ok(session) => Ok(session),
        Err(e) => {
            ltbox_core::live!(
                log,
                "{}",
                ltbox_core::i18n::tr("live_edl_open_failed_reboot_notice")
            );
            Err(tr_args!(
                "err_edl_session_open_failed",
                error = e.to_string()
            ))
        }
    }
}

pub(crate) fn wait_for_manual_edl(tag: &str, log: &mut Vec<String>) -> Result<(), ()> {
    ltbox_core::live!(
        log,
        "[{tag}] {}",
        ltbox_core::i18n::tr("live_manual_reboot_edl_wait")
    );
    wait_for_edl_ready(tag, log)
}

pub(crate) fn reboot_adb_to_edl(
    tag: &str,
    log: &mut Vec<String>,
    mgr: &mut ltbox_device::adb::AdbManager,
) -> Result<(), ()> {
    // The command echo (`adb reboot edl`) is suppressed; the user only sees the outcome (waiting /
    // reached EDL / failure).
    //
    // `mgr` is caller-provided so the Fastboot->ADB->EDL path reuses the `AdbManager` it already
    // opened in `wait_for_device`. A second one would be another libusb claimer on the same endpoint
    // while the cached `ADBUSBDevice` is alive: `LIBUSB_ERROR_BUSY`, bucketed by `check_device_state`
    // as `Some("unauthorized")`, so the worker would bail to `wait_for_manual_edl` for an authorized
    // device. `check_device_state` (not `check_device`, which accepts only `Device`) lets recovery-state
    // ADB also seed the serial before `reboot edl`.
    let state = match mgr.check_device_state() {
        Ok(s) => s,
        Err(e) => {
            ltbox_core::live!(
                log,
                "[{tag}] {}",
                tr_args!("live_adb_state_probe_failed", error = e.to_string())
            );
            if is_adb_selection_error(&e) {
                return Err(());
            }
            return wait_for_manual_edl(tag, log);
        }
    };
    match state {
        Some("device") | Some("recovery") => {}
        Some(other) => {
            ltbox_core::live!(
                log,
                "[{tag}] {}",
                tr_args!("live_adb_state_cannot_reboot_edl", state = other)
            );
            return wait_for_manual_edl(tag, log);
        }
        None => {
            ltbox_core::live!(
                log,
                "[{tag}] {}",
                ltbox_core::i18n::tr("live_no_adb_device_found")
            );
            return wait_for_manual_edl(tag, log);
        }
    }
    match mgr.reboot("edl") {
        Ok(_) => wait_for_edl_ready(tag, log),
        Err(e) => {
            ltbox_core::live!(
                log,
                "[{tag}] {}",
                tr_args!("live_adb_reboot_edl_failed", error = e.to_string())
            );
            if is_adb_selection_error(&e) {
                return Err(());
            }
            wait_for_manual_edl(tag, log)
        }
    }
}

pub(crate) fn fastboot_reboot_then_adb_edl(tag: &str, log: &mut Vec<String>) -> Result<(), ()> {
    // The device boots into the OS so ADB comes up for the following `adb reboot edl`. `oem edl` is
    // deliberately not used (it misbehaves on some devices).
    match ltbox_device::fastboot::FastbootDevice::open() {
        Ok(mut dev) => {
            if let Err(e) = dev.reboot() {
                ltbox_core::live!(
                    log,
                    "[{tag}] {}",
                    tr_args!("live_fastboot_reboot_failed", error = e.to_string())
                );
                if is_fastboot_selection_error(&e) {
                    return Err(());
                }
                return wait_for_manual_edl(tag, log);
            }
        }
        Err(e) => {
            ltbox_core::live!(
                log,
                "[{tag}] {}",
                tr_args!("err_fastboot_open_failed", error = e.to_string())
            );
            if is_fastboot_selection_error(&e) {
                return Err(());
            }
            return wait_for_manual_edl(tag, log);
        }
    }

    ltbox_core::live!(
        log,
        "[{tag}] {}",
        ltbox_core::i18n::tr("live_adb_wait_after_fastboot")
    );
    let mut mgr = ltbox_device::adb::AdbManager::new();
    // Some devices reboot straight into EDL/9008 and never bring ADB up. Poll
    // the EDL port alongside the ADB wait so the moment 9008 appears we skip the
    // (otherwise full-timeout) ADB wait and go straight to EDL.
    let targeted = ltbox_device::selection::current_target().is_some();
    let mut refused = None;
    let waited = mgr.wait_for_device_or(|| target_reached_edl(targeted, &mut refused));
    if let Some(error) = refused {
        // The EDL device that appeared cannot be proven to be the chosen one.
        ltbox_core::live!(log, "[{tag}] {error}");
        return Err(());
    }
    match waited {
        // ADB device came up — reuse the same `mgr` (it already holds the
        // cached `ADBUSBDevice`) for `reboot_adb_to_edl` instead of opening a
        // second one — see that fn's doc comment for the USB-claim race.
        Ok(true) => reboot_adb_to_edl(tag, log, &mut mgr),
        // EDL appeared before ADB — skip the ADB wait entirely.
        Ok(false) => {
            ltbox_core::live!(
                log,
                "[{tag}] {}",
                ltbox_core::i18n::tr("live_edl_detected_skip_adb")
            );
            wait_for_edl_ready(tag, log)
        }
        Err(e) => {
            ltbox_core::live!(
                log,
                "[{tag}] {}",
                tr_args!("live_adb_wait_after_fastboot_failed", error = e.to_string())
            );
            if is_adb_selection_error(&e) {
                return Err(());
            }
            wait_for_manual_edl(tag, log)
        }
    }
}

pub(crate) fn ensure_edl(
    conn: ConnectionStatus,
    tag: &str,
    log: &mut Vec<String>,
) -> Result<(), ()> {
    if let Err(error) = ltbox_device::selection::ensure_unambiguous_target() {
        ltbox_core::live!(log, "[{tag}] {error}");
        return Err(());
    }
    match edl_entry_action(conn) {
        EdlEntryAction::AlreadyEdl => {
            ltbox_core::live!(log, "[{tag}] {}", ltbox_core::i18n::tr("live_edl_already"));
            Ok(())
        }
        EdlEntryAction::AdbReboot => {
            let mut mgr = ltbox_device::adb::AdbManager::new();
            reboot_adb_to_edl(tag, log, &mut mgr)
        }
        EdlEntryAction::FastbootRebootThenAdb => fastboot_reboot_then_adb_edl(tag, log),
        EdlEntryAction::ManualWait => wait_for_manual_edl(tag, log),
    }
}
