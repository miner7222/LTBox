use super::*;

/// Simple firmware flash: flash a firmware folder exactly like a stock Lenovo flash script,
/// skipping every LTBox-side check and modification.
///
/// Unlike [`flash_worker`] this performs **no** fingerprint/model check, **no** signing-key check,
/// and **no** region / rollback / country / wipe handling. It decrypts the firmware's own
/// `rawprogram*.x` pack to `.xml`, transitions to EDL per the current connection mode, and flashes
/// the selected rawprogram + patch XMLs verbatim. The XML selection is the same
/// [`collect_firmware_xmls_for_flash`](ltbox_device::edl::collect_firmware_xmls_for_flash) the full
/// flash uses, so only the persist-less LUN0 rawprogram is included, first. Whether user data is
/// wiped is decided solely by the firmware package.
pub(crate) fn simple_flash_worker(
    conn: ConnectionStatus,
    fw_folder: String,
    phases: PhaseReporter,
) -> Result<Vec<String>, String> {
    let mut log = Vec::new();
    live!(log, "[SimpleFlash] {}", phases.marker(1));
    let fw_dir = std::path::Path::new(&fw_folder);

    if !fw_dir.exists() {
        return Err(tr_args!(
            "err_flash_firmware_folder_missing",
            path = fw_folder
        ));
    }
    live!(
        log,
        "[SimpleFlash] {}",
        tr_args!("live_flash_firmware_folder", path = fw_folder)
    );

    // Decrypt the firmware's own `rawprogram*.x` pack to `.xml` so the catalog scan below can read
    // it. The encrypted Sahara manifest (`qsahara_device_programmer.x`) is a loader, not a flash
    // image; `EdlSession::open` decrypts it. Unpacking is not a content modification.
    decrypt_rawprogram_x_files(fw_dir, &mut log)?;

    // Decompress `*.zst` images (e.g. `super.img.zst`) so rawprogram references resolve, as in the
    // full flash, and before EDL so a multi-GB decompress doesn't hold the session open.
    decompress_zst_images(fw_dir, &mut log)?;

    // Locate the EDL loader in the firmware folder (or its parent). A missing loader is a hard
    // error: nothing can be flashed, so the run must fail rather than report success.
    let loader = require_firmware_loader(find_firmware_loader(fw_dir))?;

    // Same XML selection as the full flash, so the persist-less rawprogram0 stays first and only
    // it is included.
    let (raw_xmls, patch_xmls) = ltbox_device::edl::collect_firmware_xmls_for_flash(fw_dir, false)
        .map_err(|e| tr_args!("err_flash_xml_selection_failed", error = e.to_string()))?;
    if raw_xmls.is_empty() {
        return Err(tr_args!("err_flash_no_rawprogram_xml", path = fw_folder));
    }

    // Transition to EDL via the shared live-probe path (re-probes the current transport rather than
    // trusting the captured snapshot), then open the session, as normal firmware flashing does.
    live!(log, "[SimpleFlash] {}", phases.marker(2));
    transition_to_edl(conn, &mut log)?;
    let mut session = open_edl_session(&loader, &mut log)?;

    // Flash verbatim: no fingerprint or signing-key check, no region / ARB / country edits, no
    // keep-data skip.
    live!(log, "[SimpleFlash] {}", phases.marker(3));
    live!(
        log,
        "[SimpleFlash] {}",
        tr_args!(
            "live_flash_phase3_xml_counts",
            raw = raw_xmls.len().to_string(),
            patch = patch_xmls.len().to_string()
        )
    );
    phases.mark_writes_started();
    session
        .flash_rawprogram_verbatim(&raw_xmls, &patch_xmls, &mut log)
        .map_err(|e| tr_args!("err_flash_firmware_failed", error = e.to_string()))?;

    // Mark `_a` active before reset (Lenovo rawprograms only target `_a`), as in the stock script
    // and the full flash, so the device boots the freshly written slot.
    live!(log, "[SimpleFlash] {}", phases.marker(4));
    phases.mark_writes_started();
    if let Err(e) = session.set_active_slot_a(&mut log) {
        return Err(tr_args!(
            "err_flash_set_bootable_lun_failed",
            error = e.to_string()
        ));
    }
    live!(log, "[SimpleFlash] {}", phases.marker(5));
    session.reset_tolerant(&mut log);
    live!(
        log,
        "[SimpleFlash] {}",
        ltbox_core::i18n::tr("live_flash_completed")
    );
    ltbox_core::app_paths::clean_work_dirs();
    Ok(log)
}
