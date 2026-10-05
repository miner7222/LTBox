//! System-update worker: disable/enable Lenovo OTA packages over ADB,
//! or run the Rescue OTA (EDL dump + region patch + reflash).

use crate::{
    ConnectionStatus, PhaseReporter, RescueRegion, SysUpdateAction, open_edl_session,
    transition_to_edl,
};
use ltbox_core::tr_args;

fn package_reinstall_succeeded(output: &str, package: &str) -> bool {
    let prefix = format!("Package {package} installed for user: ");
    output.lines().any(|line| {
        line.trim()
            .strip_prefix(&prefix)
            .is_some_and(|user| user.parse::<u32>().is_ok())
    })
}

pub(crate) fn sysupdate_worker(
    action: SysUpdateAction,
    rescue_folder: Option<String>,
    rescue_region: Option<RescueRegion>,
    device_model: String,
    conn: ConnectionStatus,
    phases: PhaseReporter,
) -> Result<Vec<String>, String> {
    let mut log = Vec::new();
    if action == SysUpdateAction::Rescue
        && let Some(error) = rescue_capability_error(&device_model)
    {
        return Err(error);
    }
    // Disable/Enable need a running Android shell; Rescue needs EDL. A
    // device sitting in Fastboot or EDL is still recoverable, so bridge it:
    //   * Disable/Enable: from Fastboot, `fastboot continue` and wait for
    //     ADB; from EDL there is no automatic system-boot path, so the user
    //     must reboot manually.
    //   * Rescue: hand off to `transition_to_edl`, which already handles
    //     all three source modes via `ensure_edl`.
    if action != SysUpdateAction::Rescue {
        ltbox_core::live!(log, "[SysUpdate] {}", phases.marker(1));
    }
    if action != SysUpdateAction::Rescue && matches!(conn, ConnectionStatus::Fastboot) {
        ltbox_core::live!(
            log,
            "[SysUpdate] {}",
            ltbox_core::i18n::tr("live_sysupdate_fastboot_to_adb")
        );
        if let Ok(mut dev) = ltbox_device::fastboot::FastbootDevice::open() {
            let _ = dev.reboot();
        }
    }
    let mut adb = ltbox_device::adb::AdbManager::new();
    // Disable/Enable need ADB. Wait up to 120 s
    // (matches `AdbManager::wait_for_device`'s
    // internal cap) so a fastboot→system reboot
    // has time to land before we surface a hard
    // failure. Rescue skips this — its own bridge
    // below routes to EDL, where ADB isn't needed.
    if action != SysUpdateAction::Rescue {
        ltbox_core::live!(
            log,
            "[ADB] {}",
            ltbox_core::i18n::tr("live_adb_checking_device")
        );
        if !adb.check_device().unwrap_or(false) {
            if matches!(conn, ConnectionStatus::Fastboot) {
                if let Err(e) = adb.wait_for_device() {
                    return Err(tr_args!("err_sysupdate_no_adb", error = e.to_string()));
                }
            } else {
                return Err(tr_args!(
                    "err_sysupdate_no_adb",
                    error = "device not in ADB"
                ));
            }
        }
        ltbox_core::live!(
            log,
            "[ADB] {}",
            ltbox_core::i18n::tr("live_adb_device_connected")
        );
    }
    let packages = [
        "com.lenovo.ota",
        "com.tblenovo.lenovowhatsnew",
        "com.lenovo.tbengine",
    ];
    match action {
        SysUpdateAction::Disable => {
            // Command echoes (`$ settings put …` / `$ pm clear …`)
            // were noise — the user only needs to see the outcome
            // (Uninstalled / Reinstalled / failure). Suppressed.
            ltbox_core::live!(log, "[SysUpdate] {}", phases.marker(2));
            phases.mark_writes_started();
            adb.shell("settings put global ota_disable_automatic_update 1")
                .map_err(|e| e.to_string())?;
            adb.shell("settings put secure lenovo_ota_new_version_found 0")
                .map_err(|e| e.to_string())?;

            ltbox_core::live!(log, "[SysUpdate] {}", phases.marker(3));
            let mut succeeded = 0;
            for pkg in &packages {
                let _ = adb.shell(&format!("pm clear {pkg}"));

                match adb.shell(&format!("pm uninstall -k --user 0 {pkg}")) {
                    Ok(out) if out.trim() == "Success" => {
                        succeeded += 1;
                        ltbox_core::live!(
                            log,
                            "[ADB] {}",
                            tr_args!("live_adb_uninstalled", package = pkg)
                        );
                    }
                    Ok(out) => ltbox_core::live!(
                        log,
                        "[ADB] {}",
                        tr_args!("live_adb_uninstall_failed", package = pkg, error = out)
                    ),
                    Err(e) => ltbox_core::live!(
                        log,
                        "[ADB] {}",
                        tr_args!("live_adb_uninstall_failed", package = pkg, error = e)
                    ),
                }
            }
            ltbox_core::live!(
                log,
                "[SysUpdate] {}",
                tr_args!(
                    "live_sysupdate_disabled",
                    success = succeeded,
                    total = packages.len()
                )
            );
            Ok(log)
        }
        SysUpdateAction::Enable => {
            // Command echoes suppressed — same rationale as Disable.
            ltbox_core::live!(log, "[SysUpdate] {}", phases.marker(2));
            phases.mark_writes_started();
            adb.shell("settings put global ota_disable_automatic_update 0")
                .map_err(|e| e.to_string())?;

            ltbox_core::live!(log, "[SysUpdate] {}", phases.marker(3));
            let mut succeeded = 0;
            for pkg in &packages {
                match adb.shell(&format!("cmd package install-existing {pkg}")) {
                    Ok(out) if package_reinstall_succeeded(&out, pkg) => {
                        succeeded += 1;
                        ltbox_core::live!(
                            log,
                            "[ADB] {}",
                            tr_args!("live_adb_reinstalled", package = pkg)
                        );
                    }
                    Ok(out) => ltbox_core::live!(
                        log,
                        "[ADB] {}",
                        tr_args!("live_adb_reinstall_failed", package = pkg, error = out)
                    ),
                    Err(e) => ltbox_core::live!(
                        log,
                        "[ADB] {}",
                        tr_args!("live_adb_reinstall_failed", package = pkg, error = e)
                    ),
                }
            }
            ltbox_core::live!(
                log,
                "[SysUpdate] {}",
                tr_args!(
                    "live_sysupdate_enabled",
                    success = succeeded,
                    total = packages.len()
                )
            );
            Ok(log)
        }
        SysUpdateAction::Rescue => {
            ltbox_core::live!(log, "[Rescue] {}", phases.marker(1));
            // Precondition: loader file + region
            // picked in the wizard.
            let Some(loader_path) = rescue_folder else {
                return Err(ltbox_core::i18n::tr("err_rescue_loader_not_selected"));
            };
            let Some(region) = rescue_region else {
                return Err(ltbox_core::i18n::tr("err_rescue_region_not_selected"));
            };
            let loader = std::path::PathBuf::from(&loader_path);
            if !loader.is_file() {
                return Err(tr_args!(
                    "err_rescue_loader_missing",
                    path = loader.display().to_string()
                ));
            }
            // Extension-only check — accept `.melf` /
            // `.mbn` / `.elf` single-blob loaders, the
            // `.xml` multi-image manifest, or its
            // encrypted `.x` form (decrypted in
            // `EdlSession::open`). Filename is free-form.
            let ext_ok = loader
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| {
                    let l = e.to_ascii_lowercase();
                    l == "melf" || l == "mbn" || l == "elf" || l == "xml"
                })
                || ltbox_core::sahara_xml::is_encrypted_manifest_filename(&loader);
            if !ext_ok {
                return Err(tr_args!(
                    "err_rescue_loader_invalid",
                    path = loader.display().to_string()
                ));
            }
            let loader_dir = loader
                .parent()
                .map(std::path::Path::to_path_buf)
                .unwrap_or_else(|| std::path::PathBuf::from("."));
            ltbox_core::live!(
                log,
                "[Rescue] {}",
                tr_args!("live_rescue_loader", path = loader.display().to_string())
            );
            ltbox_core::live!(
                log,
                "[Rescue] {}",
                tr_args!(
                    "live_rescue_target_region",
                    target = match region {
                        RescueRegion::Prc => "PRC",
                        RescueRegion::Row => "ROW",
                    }
                )
            );

            // Stage dumps + patched outputs in a
            // timestamped temp dir next to the
            // loader so the user's loader directory
            // doesn't get cluttered with rescue
            // intermediates.
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let work_dir = loader_dir.join(format!("rescue_{ts}"));
            if let Err(e) = std::fs::create_dir_all(&work_dir) {
                return Err(tr_args!(
                    "err_rescue_work_dir_failed",
                    error = e.to_string()
                ));
            }
            ltbox_core::live!(
                log,
                "[Rescue] {}",
                tr_args!(
                    "live_rescue_work_dir",
                    path = work_dir.display().to_string()
                )
            );

            ltbox_core::live!(log, "[Rescue] {}", phases.marker(2));
            ltbox_core::live!(
                log,
                "[Rescue] {}",
                ltbox_core::i18n::tr("live_rescue_transitioning")
            );
            // Use the shared `transition_to_edl` helper so Rescue handles
            // every source mode (ADB / Fastboot / EDL) the same way
            // Flash / Root / Unroot already do.
            transition_to_edl(conn, &mut log)?;

            ltbox_core::live!(log, "[Rescue] {}", phases.marker(3));
            let mut session = open_edl_session(&loader, &mut log)?;

            // vendor_boot + vbmeta resolve through the
            // shared partition LUN map (LUN 4 on supported
            // models). GPT-by-name resolves sector geometry,
            // no rawprogram*.xml needed.
            let slots = ["a", "b"];
            let mut dumped: Vec<(String, String, std::path::PathBuf)> = Vec::new();
            ltbox_core::live!(log, "[Rescue] {}", phases.marker(4));
            for slot in &slots {
                for base in &["vendor_boot", "vbmeta"] {
                    let part_name = format!("{base}_{slot}");
                    let out = work_dir.join(format!("{part_name}.img"));
                    let Some(lun) = rescue_partition_lun(&part_name) else {
                        ltbox_core::live!(
                            log,
                            "[Rescue] {}",
                            tr_args!(
                                "live_rescue_skip_dump",
                                name = part_name,
                                error = tr_args!("err_no_hardcoded_lun", partition = part_name)
                            )
                        );
                        continue;
                    };
                    ltbox_core::live!(
                        log,
                        "[Rescue] {}",
                        tr_args!("live_rescue_dumping", name = part_name)
                    );
                    if let Err(e) = session.dump_partition(&part_name, &out, 0, lun, &mut log) {
                        ltbox_core::live!(
                            log,
                            "[Rescue] {}",
                            tr_args!(
                                "live_rescue_skip_dump",
                                name = part_name,
                                error = e.to_string()
                            )
                        );
                        continue;
                    }
                    dumped.push(((*base).to_string(), (*slot).to_string(), out));
                }
            }

            // Model-agnostic safety net for the EDL-first
            // path where `device_model` is unknown so the
            // TB323FU action gate can't fire: if none of
            // vendor_boot/vbmeta resolved via the shared
            // LUN map + GPT, the device doesn't have the
            // layout Boot Recovery assumes. Abort before
            // any write — nothing was flashed.
            if dumped.is_empty() {
                return Err(ltbox_core::i18n::tr("err_rescue_unsupported_layout"));
            }

            // Cross-check firmware against device
            // model via AVB vendor_boot fingerprint —
            // aborts the whole rescue if the dumped
            // image was built for another model. Uses
            // the first available vendor_boot dump;
            // slot A/B carry the same fingerprint.
            if let Some(vb_probe) = dumped.iter().find(|(b, _, _)| b == "vendor_boot") {
                match ltbox_patch::avb::extract_image_avb_info(&vb_probe.2) {
                    Ok(info) => {
                        if let Some(error) = ltbox_patch::avb::build_fingerprint(&info)
                            .as_deref()
                            .and_then(|fp| {
                                if ltbox_core::model::fingerprint_names_known_model(fp) {
                                    ltbox_core::model::fingerprint_models(fp)
                                        .find_map(rescue_capability_error)
                                } else {
                                    // A fingerprint that names no model cannot
                                    // vouch for one: gate on the detected model.
                                    ltbox_core::model::known_model(&device_model)
                                        .and_then(rescue_capability_error)
                                }
                            })
                        {
                            session.reset_tolerant(&mut log);
                            return Err(error);
                        }
                        use ltbox_patch::region::{ModelValidation, validate_device_model};
                        match validate_device_model(&info, &device_model) {
                            ModelValidation::Match { .. } => {
                                ltbox_core::live!(
                                    log,
                                    "[Rescue] {}",
                                    ltbox_core::i18n::tr("live_rescue_model_check_ok")
                                );
                            }
                            ModelValidation::Unidentified {
                                fingerprint,
                                device_model,
                            } => {
                                ltbox_core::live!(
                                    log,
                                    "[Rescue] {}",
                                    tr_args!(
                                        "live_image_fingerprint_unidentified",
                                        fingerprint = fingerprint,
                                        model = device_model
                                    )
                                );
                            }
                            ModelValidation::Missing => {
                                ltbox_core::live!(
                                    log,
                                    "[Rescue] {}",
                                    ltbox_core::i18n::tr("live_rescue_no_fingerprint_skip")
                                );
                            }
                            ModelValidation::Mismatch {
                                fingerprint,
                                device_model,
                            } => {
                                ltbox_core::live!(
                                    log,
                                    "[Rescue] {}",
                                    tr_args!(
                                        "live_rescue_model_mismatch_abort",
                                        device = device_model,
                                        fingerprint = fingerprint
                                    )
                                );
                                session.reset_tolerant(&mut log);
                                return Err(ltbox_core::i18n::tr("err_rescue_model_mismatch"));
                            }
                        }
                    }
                    Err(e) => {
                        ltbox_core::live!(
                            log,
                            "[Rescue] {}",
                            tr_args!("live_rescue_avb_inspect_skip", error = e.to_string())
                        );
                    }
                }
            }

            // Patch vendor_boot per region, rebuild its footer, then refresh
            // the matching vbmeta descriptor per slot.
            let target = region.to_target();
            let prc_dot = vec![0x2E, 0x50, 0x52, 0x43]; // ".PRC"
            let prc_i = vec![0x49, 0x50, 0x52, 0x43]; // "IPRC"
            let row_dot = vec![0x2E, 0x52, 0x4F, 0x57]; // ".ROW"
            let row_i = vec![0x49, 0x52, 0x4F, 0x57]; // "IROW"
            let prc_patterns: Vec<(Vec<u8>, Vec<u8>)> = vec![
                (prc_dot.clone(), row_dot.clone()),
                (prc_i.clone(), row_i.clone()),
            ];
            let row_patterns: Vec<(Vec<u8>, Vec<u8>)> = vec![
                (row_dot.clone(), prc_dot.clone()),
                (row_i.clone(), prc_i.clone()),
            ];

            ltbox_core::live!(log, "[Rescue] {}", phases.marker(5));
            let mut flash_plan: Vec<(String, std::path::PathBuf)> = Vec::new();
            for slot in &slots {
                let vb_src = dumped
                    .iter()
                    .find(|(b, s, _)| b == "vendor_boot" && s == slot);
                let vbm_src = dumped.iter().find(|(b, s, _)| b == "vbmeta" && s == slot);
                let (Some(vb_src), Some(vbm_src)) = (vb_src, vbm_src) else {
                    ltbox_core::live!(
                        log,
                        "[Rescue] {}",
                        tr_args!("live_rescue_slot_missing_dump", slot = slot)
                    );
                    continue;
                };

                let vb_patched = work_dir.join(format!("vendor_boot_{slot}.patched.img"));
                ltbox_core::live!(
                    log,
                    "[Rescue] {}",
                    tr_args!(
                        "live_rescue_patching_vendor_boot",
                        slot = slot,
                        target = match region {
                            RescueRegion::Prc => "PRC",
                            RescueRegion::Row => "ROW",
                        }
                    )
                );
                let n = match ltbox_patch::region::patch_vendor_boot(
                    &vb_src.2,
                    &vb_patched,
                    target,
                    &prc_patterns,
                    &row_patterns,
                ) {
                    Ok(n) => n,
                    Err(e) => {
                        ltbox_core::live!(
                            log,
                            "[Rescue] {}",
                            tr_args!(
                                "live_rescue_region_patch_failed",
                                slot = slot,
                                error = e.to_string()
                            )
                        );
                        continue;
                    }
                };
                if n == 0 {
                    ltbox_core::live!(
                        log,
                        "[Rescue] {}",
                        tr_args!("live_rescue_no_region_bytes_changed", slot = slot)
                    );
                } else {
                    ltbox_core::live!(
                        log,
                        "[Rescue] {}",
                        tr_args!(
                            "live_rescue_occurrences_patched",
                            slot = slot,
                            count = n.to_string()
                        )
                    );
                }

                // Rebuild AVB hash footer on the
                // patched vendor_boot using metadata
                // from the original.
                let vb_info = match ltbox_patch::avb::extract_image_avb_info(&vb_src.2) {
                    Ok(i) => i,
                    Err(e) => {
                        ltbox_core::live!(
                            log,
                            "[Rescue] {}",
                            tr_args!(
                                "live_rescue_vendor_boot_avb_failed",
                                slot = slot,
                                error = e.to_string()
                            )
                        );
                        continue;
                    }
                };
                // Only the two stock test keys embedded in
                // avbtool-rs are supported.
                let vb_key_spec =
                    ltbox_patch::key_map::key_spec_for_pubkey(vb_info.public_key_sha1.as_deref());
                if let Err(e) =
                    ltbox_patch::avb::add_hash_footer(&vb_patched, &vb_info, vb_key_spec, None)
                {
                    ltbox_core::live!(
                        log,
                        "[Rescue] {}",
                        tr_args!(
                            "live_rescue_add_hash_footer_failed",
                            slot = slot,
                            error = e.to_string()
                        )
                    );
                    continue;
                }

                // Refresh matching vbmeta descriptors from those embedded in
                // the patched vendor_boot. Key fallback:
                // algorithm comes from the original
                // vbmeta header.
                let vbm_info = match ltbox_patch::avb::extract_image_avb_info(&vbm_src.2) {
                    Ok(i) => i,
                    Err(e) => {
                        ltbox_core::live!(
                            log,
                            "[Rescue] {}",
                            tr_args!(
                                "live_rescue_vbmeta_inspect_failed",
                                slot = slot,
                                error = e.to_string()
                            )
                        );
                        continue;
                    }
                };
                let Some(vbm_key) =
                    ltbox_patch::key_map::key_spec_for_pubkey(vbm_info.public_key_sha1.as_deref())
                else {
                    ltbox_core::live!(
                        log,
                        "[Rescue] {}",
                        tr_args!("live_rescue_no_testkey", slot = slot)
                    );
                    continue;
                };
                let vbm_rebuilt = work_dir.join(format!("vbmeta_{slot}.rebuilt.img"));
                let partition_images: [&std::path::Path; 1] = [vb_patched.as_path()];
                if let Err(e) = ltbox_patch::avb::rebuild_vbmeta_with_partition_descriptors(
                    &vbm_rebuilt,
                    &vbm_src.2,
                    &partition_images,
                    vbm_key,
                    Some(vbm_info.algorithm.as_str()),
                ) {
                    ltbox_core::live!(
                        log,
                        "[Rescue] {}",
                        tr_args!(
                            "live_rescue_rebuild_vbmeta_failed",
                            slot = slot,
                            error = e.to_string()
                        )
                    );
                    continue;
                }

                flash_plan.push((format!("vendor_boot_{slot}"), vb_patched));
                flash_plan.push((format!("vbmeta_{slot}"), vbm_rebuilt));
            }

            if flash_plan.is_empty() {
                return Err(ltbox_core::i18n::tr("err_rescue_nothing_to_flash"));
            }

            ltbox_core::live!(log, "[Rescue] {}", phases.marker(6));
            ltbox_core::live!(
                log,
                "[Rescue] {}",
                tr_args!(
                    "live_rescue_flashing_targets",
                    count = flash_plan.len().to_string()
                )
            );
            let requests = flash_plan
                .iter()
                .map(|(part_name, image)| {
                    let lun = rescue_partition_lun(part_name)
                        .ok_or_else(|| tr_args!("err_no_hardcoded_lun", partition = part_name))?;
                    Ok(ltbox_device::edl::PartitionFlash {
                        label: part_name,
                        image,
                        slot: 0,
                        lun,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            session
                .flash_partition_batch(
                    &requests,
                    &mut log,
                    |_, _, _| {},
                    || {
                        phases.mark_writes_started();
                    },
                )
                .map_err(|e| {
                    ltbox_core::live!(
                        log,
                        "[Rescue] {}",
                        tr_args!(
                            "live_rescue_flash_failed",
                            name = e.partition,
                            error = e.source
                        )
                    );
                    tr_args!(
                        "err_rescue_flash_failed",
                        name = e.partition,
                        error = e.source
                    )
                })?;

            ltbox_core::live!(log, "[Rescue] {}", phases.marker(7));
            ltbox_core::live!(
                log,
                "[Rescue] {}",
                ltbox_core::i18n::tr("live_rescue_resetting")
            );
            session.reset_tolerant(&mut log);
            ltbox_core::live!(
                log,
                "[Rescue] {}",
                ltbox_core::i18n::tr("live_rescue_complete")
            );
            Ok(log)
        }
    }
}

/// Resolve Boot Recovery partition LUN via the shared map.
/// Expected LUN 4 for vendor_boot / vbmeta on supported models.
fn rescue_partition_lun(part_name: &str) -> Option<u8> {
    ltbox_core::partition_lun::lun_for_partition(part_name)
}

/// Refuse Rescue on a model whose profile disables it, naming that model.
fn rescue_capability_error(model: &str) -> Option<String> {
    (!ltbox_core::model::capabilities(model).rescue)
        .then(|| tr_args!("model_unsupported", model = model))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_result_does_not_treat_not_installed_as_success() {
        let package = "com.lenovo.ota";
        assert!(package_reinstall_succeeded(
            "Package com.lenovo.ota installed for user: 0\r\n",
            package
        ));
        assert!(package_reinstall_succeeded(
            "Package com.lenovo.ota installed for user: 10",
            package
        ));
        for output in [
            "Package com.lenovo.ota not installed",
            "Package other installed for user: 0",
            "Error: package not installed for 0",
            "",
        ] {
            assert!(!package_reinstall_succeeded(output, package), "{output}");
        }
    }

    #[test]
    fn disabled_rescue_models_are_rejected_before_device_access() {
        for model in ["TB323FU", "TB324ZC", "TB376FC", "TB390FU", "TB391FC"] {
            let phases = PhaseReporter::from_labels(vec!["unused".into()]);
            let error = sysupdate_worker(
                SysUpdateAction::Rescue,
                None,
                None,
                model.into(),
                ConnectionStatus::None,
                phases,
            )
            .expect_err("unsupported Rescue must fail before loader/device access");
            assert_eq!(error, rescue_capability_error(model).unwrap());
        }
    }

    #[test]
    fn rescue_vendor_boot_and_vbmeta_use_lun4() {
        for name in [
            "vendor_boot",
            "vendor_boot_a",
            "vendor_boot_b",
            "vbmeta",
            "vbmeta_a",
            "vbmeta_b",
        ] {
            assert_eq!(rescue_partition_lun(name), Some(4), "{name}");
        }
    }

    #[test]
    fn rescue_unknown_partition_returns_none() {
        assert_eq!(rescue_partition_lun("nonexistent_part"), None);
    }
}
