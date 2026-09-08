//! Reboot workers: issue the reboot command over ADB / Fastboot / EDL.
//! Each runs on a blocking thread; extracted from the update_reboot handler.

use crate::{ConnectionStatus, RebootTarget, ensure_edl, open_edl_session};
use ltbox_core::{i18n::tr, tr_args};
use std::path::PathBuf;

pub(crate) fn reboot_worker(
    conn: ConnectionStatus,
    target: RebootTarget,
    reboot_cmd_sent: String,
) -> Result<Vec<String>, String> {
    let mut log = Vec::new();
    match (conn, target) {
        (ConnectionStatus::Adb | ConnectionStatus::AdbRecovery, RebootTarget::Edl) => {
            // Unlike the other one-shot reboot targets, EDL entry is not done
            // until the 9008 port appears. Reuse the shared transition worker
            // so the tracked Reboot operation stays live for the wait dialog.
            ensure_edl(conn, "Reboot", &mut log)
                .map_err(|()| ltbox_core::i18n::tr("err_edl_transition_failed"))?;
        }
        (
            ConnectionStatus::Adb | ConnectionStatus::AdbRecovery | ConnectionStatus::AdbSideload,
            t,
        ) => {
            let mut adb = ltbox_device::adb::AdbManager::new();
            // `AdbManager::reboot` needs the serial
            // from a prior `check_device` call.
            if !adb.check_device().unwrap_or(false) {
                return Err(tr("err_no_adb_device_replug"));
            }
            let arg = match t {
                RebootTarget::System => "",
                RebootTarget::Recovery => "recovery",
                RebootTarget::Bootloader => "bootloader",
                RebootTarget::Fastbootd => "fastboot",
                RebootTarget::Edl => "edl",
            };
            if let Err(e) = adb.reboot(arg) {
                return Err(tr_args!("err_reboot_adb_failed", error = e));
            }
        }
        (ConnectionStatus::Fastboot, t) => {
            let mut dev = ltbox_device::fastboot::FastbootDevice::open()
                .map_err(|e| tr_args!("err_fastboot_open_failed", error = e))?;
            match t {
                RebootTarget::System => {
                    dev.reboot().map_err(|e| {
                        tr_args!("err_fastboot_command_failed", command = "reboot", error = e)
                    })?;
                }
                RebootTarget::Bootloader => {
                    dev.reboot_bootloader().map_err(|e| {
                        tr_args!(
                            "err_fastboot_command_failed",
                            command = "reboot-bootloader",
                            error = e
                        )
                    })?;
                }
                RebootTarget::Fastbootd => {
                    dev.reboot_fastboot().map_err(|e| {
                        tr_args!(
                            "err_fastboot_command_failed",
                            command = "reboot-fastboot",
                            error = e
                        )
                    })?;
                }
                RebootTarget::Edl => {
                    drop(dev);
                    ensure_edl(ConnectionStatus::Fastboot, "Reboot", &mut log)
                        .map_err(|()| ltbox_core::i18n::tr("err_edl_transition_failed"))?;
                }
                RebootTarget::Recovery => {
                    return Err(tr("err_fastboot_recovery_unsupported"));
                }
            }
        }
        (ConnectionStatus::Edl, _) => {
            // RebootTo routes EDL through
            // RebootEdlWithLoader, never here.
            unreachable!("EDL reboot goes through RebootEdlWithLoader");
        }
        (ConnectionStatus::None, _) => {
            return Err(tr("err_no_device_connected"));
        }
        (ConnectionStatus::AdbUnauthorized, _) => {
            return Err(tr("err_usb_debugging_unauthorized"));
        }
        (ConnectionStatus::AdbServerBlocking, _) => {
            return Err(tr("err_adb_server_blocking_retry"));
        }
    }
    ltbox_core::live!(log, "[Reboot] {}", reboot_cmd_sent);
    Ok(log)
}

pub(crate) fn reboot_edl_with_loader_worker(
    loader: PathBuf,
    target: RebootTarget,
    reboot_cmd_sent: String,
) -> Result<Vec<String>, String> {
    let mut log = Vec::new();
    let mut session = open_edl_session(&loader, &mut log)?;
    match target {
        RebootTarget::System => {
            // Reboot-to-system is the user's intent here; the inner EDL
            // reset log lines duplicate the surrounding `[Reboot]` start +
            // `command sent` lines, so swallow them into a scratch log.
            let mut quiet = Vec::new();
            session.reset_tolerant(&mut quiet);
        }
        RebootTarget::Edl => {
            session
                .reset_to_edl(&mut log)
                .map_err(|e| tr_args!("err_reboot_edl_reset_failed", error = e))?;
        }
        other => {
            return Err(tr_args!(
                "err_reboot_edl_target_unsupported",
                target = format!("{other:?}")
            ));
        }
    }
    ltbox_core::live!(log, "[Reboot] {}", reboot_cmd_sent);
    Ok(log)
}
