//! Root-manager APK install helpers (push/install the Magisk/KernelSU/APatch
//! manager app over ADB).

use crate::*;

pub(crate) fn install_root_manager_apk(
    manager_apk: &std::path::Path,
    log: &mut Vec<String>,
) -> std::result::Result<(), String> {
    let mut adb = ltbox_device::adb::AdbManager::new();
    if !adb.check_device().unwrap_or(false) {
        return Err("ADB device is not connected".to_string());
    }
    let path = manager_apk.to_string_lossy().to_string();
    live!(
        log,
        "[Root] {}",
        tr_args!(
            "log_root_installing_manager_apk",
            path = manager_apk.display().to_string()
        )
    );
    adb.install(&path)
        .map_err(|e| format!("Manager APK install failed: {e}"))?;
    live!(
        log,
        "[Root] {}",
        ltbox_core::i18n::tr("log_root_manager_apk_installed")
    );
    Ok(())
}

pub(crate) fn wait_and_install_root_manager_apk(
    manager_apk: &std::path::Path,
    timeout: std::time::Duration,
    log: &mut Vec<String>,
) -> std::result::Result<(), String> {
    let deadline = std::time::Instant::now() + timeout;
    live!(
        log,
        "[Root] {}",
        tr_args!(
            "live_root_wait_adb_for_apk",
            seconds = timeout.as_secs().to_string()
        )
    );
    loop {
        match install_root_manager_apk(manager_apk, log) {
            Ok(()) => return Ok(()),
            // Return the raw error: the caller adds the manual-install hint.
            Err(last) if std::time::Instant::now() >= deadline => return Err(last),
            Err(_) => std::thread::sleep(std::time::Duration::from_secs(1)),
        }
    }
}

/// After the manager APK fails to auto-install, copy it onto the device at
/// `/sdcard/manager.apk` so the user can install it there by hand.
///
/// Returns the path to surface in the manual-install reminder plus whether
/// the local staging copy must be kept:
/// - `(/sdcard/manager.apk, false)` — the push succeeded, so the on-device
///   copy is enough and the staging dir can be cleaned up.
/// - `(local apk path, true)` — the push also failed, so the user needs the
///   local file and the caller must keep the staging dir.
///
/// A fresh [`AdbManager`] is used so a transport dropped by the failed
/// install is re-claimed cleanly.
pub(crate) fn stage_manager_apk_for_manual_install(
    apk: &std::path::Path,
    log: &mut Vec<String>,
) -> (std::path::PathBuf, bool) {
    const REMOTE: &str = "/sdcard/manager.apk";
    let mut adb = ltbox_device::adb::AdbManager::new();
    if adb.check_device().unwrap_or(false) && adb.push_file(apk, REMOTE).is_ok() {
        live!(
            log,
            "[Root] {}",
            tr_args!("log_root_manager_apk_pushed", path = REMOTE)
        );
        (std::path::PathBuf::from(REMOTE), false)
    } else {
        live!(
            log,
            "[Root] {}",
            ltbox_core::i18n::tr("log_root_manager_apk_push_failed")
        );
        (apk.to_path_buf(), true)
    }
}
