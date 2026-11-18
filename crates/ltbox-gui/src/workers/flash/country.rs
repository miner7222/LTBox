use super::*;

/// Advanced "Change Country Code": rewrite the device's country code over EDL and reset to
/// system. Mirrors `flash_worker`'s country phase without a firmware package: only a user-picked
/// country + EDL loader. Partitions come from `ModelCapabilities::country_partitions`.
pub(crate) fn change_country_worker(
    conn: ConnectionStatus,
    device_model: String,
    target_code: String,
    loader: std::path::PathBuf,
    ll: LiveLabels,
    phases: PhaseReporter,
) -> Result<Vec<String>, String> {
    let mut log = Vec::new();
    live!(log, "[Country] {}", phases.marker(1));
    live!(
        log,
        "[Country] {}",
        tr_args!(
            "live_flash_country_patch_target",
            target = target_code.as_str()
        )
    );
    // Create scratch + backup dirs BEFORE entering EDL: a setup failure here
    // must not strand the device in EDL before any session exists to reset it.
    let work_dir = ltbox_core::app_paths::work_dir_for("change_country");
    let _ = std::fs::remove_dir_all(&work_dir);
    std::fs::create_dir_all(&work_dir)
        .map_err(|e| tr_args!("err_country_work_dir_failed", error = e.to_string()))?;
    let critical_backup = crate::backup::create_backup_dir("change_country", &device_model)
        .map_err(|e| tr_args!("err_country_backup_dir_failed", error = e.to_string()))?;
    // Country partitions do not carry AVB build properties. Capture an
    // available Android fingerprint before the mode transition, for the
    // informational manifest only. EDL/Fastboot starts leave it unknown.
    let backup_fingerprint = if conn.skip_adb() {
        None
    } else {
        ltbox_device::adb::AdbManager::new_if_connected()
            .and_then(|mut adb| adb.shell("getprop ro.build.fingerprint").ok())
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    live!(log, "[Country] {}", phases.marker(2));
    transition_to_edl(conn, &mut log)?;
    let mut session = open_edl_session(&loader, &mut log)?;
    let outcome = run_country_change(
        &mut session,
        &work_dir,
        &critical_backup,
        "change_country",
        backup_fingerprint.as_deref(),
        &device_model,
        None,
        Some(&target_code),
        false,
        None,
        &ll,
        &mut log,
        Some(&phases),
    );
    // Reset to system regardless (don't strand the device in EDL). On a country-write failure keep
    // phase 4, so the failed card names the stage that failed rather than the cleanup reboot.
    if let Err(error) = outcome {
        session.reset_tolerant(&mut log);
        return Err(error);
    }
    live!(log, "[Country] {}", phases.marker(5));
    session.reset_tolerant(&mut log);
    live!(
        log,
        "[Country] {}",
        ltbox_core::i18n::tr("live_country_change_done")
    );
    ltbox_core::app_paths::clean_work_dirs();
    Ok(log)
}
