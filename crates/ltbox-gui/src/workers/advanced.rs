//! Advanced-menu single-file workers: region convert, devinfo/country
//! patch, ARB patch, vbmeta rebuild, xml convert. Each takes one input
//! image and writes patched output. Extracted from the update_adv handler.

use crate::{AdvAction, DeviceRegion, PhaseReporter};
use ltbox_core::tr_args;

/// Prepare both images before modifying firmware. Backups are never overwritten.
pub(crate) fn patch_firmware_rollback(
    folder: &std::path::Path,
    target: crate::ManualRollbackIndices,
    phases: &PhaseReporter,
    log: &mut Vec<String>,
) -> Result<(), String> {
    use crate::workers::flash::manual::{
        build_manual_rollback_overlays, prepare_manual_rollback_plan,
    };
    use ltbox_patch::rollback::RollbackIndices;
    let names = ["boot.img", "vbmeta_system.img"];
    ltbox_core::live!(log, "[ARB] {}", phases.marker(1));
    for name in names {
        if folder.join(format!("{name}.bak")).exists() {
            return Err(format!("Backup already exists: {name}.bak"));
        }
    }
    let plan = prepare_manual_rollback_plan(
        folder,
        RollbackIndices {
            boot: 0,
            vbmeta_system: 0,
        },
        Some(RollbackIndices {
            boot: target.boot,
            vbmeta_system: target.vbmeta_system,
        }),
    )?;
    let temporary = tempfile::tempdir_in(folder).map_err(|e| e.to_string())?;
    ltbox_core::live!(log, "[ARB] {}", phases.marker(2));
    let overlays =
        build_manual_rollback_overlays(folder, &temporary.path().join("patched"), plan, log)?;
    // Reserve/copy both backups before either original can be changed.
    ltbox_core::live!(log, "[ARB] {}", phases.marker(3));
    for name in names {
        let mut source = std::fs::File::open(folder.join(name)).map_err(|e| e.to_string())?;
        let mut backup = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(folder.join(format!("{name}.bak")))
            .map_err(|e| e.to_string())?;
        std::io::copy(&mut source, &mut backup).map_err(|e| e.to_string())?;
        backup.sync_all().map_err(|e| e.to_string())?;
    }
    ltbox_core::live!(log, "[ARB] {}", phases.marker(4));
    for (partition, _, patched) in overlays {
        let name = match partition.as_str() {
            "boot_a" => "boot.img",
            "vbmeta_system_a" => "vbmeta_system.img",
            _ => return Err(format!("Unexpected rollback overlay: {partition}")),
        };
        if let Err(error) = std::fs::copy(&patched, folder.join(name)) {
            let mut failures = Vec::new();
            for original in names {
                if let Err(restore) = std::fs::copy(
                    folder.join(format!("{original}.bak")),
                    folder.join(original),
                ) {
                    failures.push(format!("{original}: {restore}"));
                }
            }
            return Err(format!(
                "Replace {name}: {error}; restore errors: {failures:?}. Original backups remain in the firmware folder."
            ));
        }
    }
    ltbox_core::live!(
        log,
        "[ARB] {}",
        tr_args!("live_advanced_output_folder", path = folder.display())
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn advanced_file_worker(
    input_path: String,
    action: AdvAction,
    adv_country: Option<String>,
    adv_region_target: Option<DeviceRegion>,
    adv_arb_index: Option<crate::ManualRollbackIndices>,
    output_dir: std::path::PathBuf,
    action_label: String,
    phases: PhaseReporter,
) -> Result<Vec<String>, String> {
    let mut log = Vec::new();
    let input = std::path::Path::new(&input_path);
    let parent = input.parent().unwrap_or(std::path::Path::new("."));
    // Created eagerly so a no-op exec still
    // leaves a folder for the user to find.
    if action.produces_output() && action != AdvAction::PatchArb {
        let _ = std::fs::create_dir_all(&output_dir);
        ltbox_core::live!(
            log,
            "[Advanced] {}",
            tr_args!(
                "live_advanced_output_folder",
                path = output_dir.display().to_string()
            )
        );
    }
    match action {
        AdvAction::ImageInfo => {
            return Err(ltbox_core::i18n::tr("err_advanced_image_info_dedicated"));
        }
        AdvAction::ConvertXml => {
            ltbox_core::live!(log, "[Crypto] {}", phases.marker(1));
            // `input` is now the folder holding the encrypted
            // `*.x` pack (picker moved from file→folder so
            // users don't have to repeat the dialog for each
            // file). Iterate every `*.x`, decrypt to `*.xml`
            // in `output_dir`.
            let mut entries: Vec<std::path::PathBuf> = std::fs::read_dir(input)
                .map_err(|e| {
                    tr_args!(
                        "err_read_dir_failed",
                        path = input.display().to_string(),
                        error = e.to_string()
                    )
                })?
                .filter_map(|r| r.ok().map(|e| e.path()))
                .filter(|p| {
                    p.is_file()
                        && p.extension()
                            .and_then(|s| s.to_str())
                            .map(|s| s.eq_ignore_ascii_case("x"))
                            .unwrap_or(false)
                })
                .collect();
            entries.sort();
            if entries.is_empty() {
                return Err(tr_args!(
                    "err_xml_no_x_files",
                    path = input.display().to_string()
                ));
            }
            ltbox_core::live!(log, "[Crypto] {}", phases.marker(2));
            for src in entries {
                let stem = src.file_stem().unwrap_or_default();
                let output = output_dir.join(stem).with_extension("xml");
                match ltbox_core::crypto::decrypt_file(&src, &output) {
                    Ok(size) => ltbox_core::live!(
                        log,
                        "[Crypto] {}",
                        tr_args!("live_crypto_decrypted", bytes = size.to_string())
                    ),
                    Err(e) => {
                        return Err(tr_args!(
                            "err_decrypt_file_failed",
                            path = src.display().to_string(),
                            error = e.to_string()
                        ));
                    }
                }
            }
            ltbox_core::live!(log, "[Crypto] {}", phases.marker(3));
        }
        AdvAction::DetectArb => {
            // DetectArb routes through its dedicated
            // `AdvDetectArbExecStart` worker, not the
            // generic file-selected pipeline. Reaching
            // this arm means a stale code path triggered
            // it; surface a clear error instead of a
            // silent no-op.
            return Err(ltbox_core::i18n::tr("err_advanced_detect_arb_dedicated"));
        }
        AdvAction::FlashPartitions
        | AdvAction::DumpPartitions
        | AdvAction::FlashPhysical
        | AdvAction::DumpPhysical
        | AdvAction::SimpleFlash => {
            ltbox_core::live!(
                log,
                "[Advanced] {}",
                ltbox_core::i18n::tr("live_advanced_use_dedicated")
            );
        }
        AdvAction::RegionConvert => {
            ltbox_core::live!(log, "[Region] {}", phases.marker(1));
            let Some(target_region) = adv_region_target else {
                return Err(ltbox_core::i18n::tr("err_region_target_missing"));
            };
            if input
                .file_name()
                .and_then(|s| s.to_str())
                .map(|s| !s.eq_ignore_ascii_case("vendor_boot.img"))
                .unwrap_or(true)
            {
                return Err(ltbox_core::i18n::tr("err_region_vendor_boot_expected"));
            }
            let firmware_dir = parent;
            let sibling_vbmeta = firmware_dir.join("vbmeta.img");
            if !sibling_vbmeta.is_file() {
                return Err(tr_args!(
                    "err_region_vbmeta_missing",
                    path = sibling_vbmeta.display().to_string()
                ));
            }
            let target = target_region.to_region_target();
            match ltbox_patch::region::build_region_converted_avb_images_with_progress(
                firmware_dir,
                &output_dir,
                target,
                &ltbox_patch::region::RegionPatternSet::default(),
                None,
                |stage| {
                    let phase = match stage {
                        ltbox_patch::region::RegionBuildStage::Inspect => 2,
                        ltbox_patch::region::RegionBuildStage::PatchVendorBoot => 3,
                        ltbox_patch::region::RegionBuildStage::RebuildVbmeta => 4,
                    };
                    ltbox_core::live!(log, "[Region] {}", phases.marker(phase));
                },
            ) {
                Ok(ltbox_patch::region::RegionAvbBuild::Built(output)) => {
                    ltbox_core::live!(
                        log,
                        "[Region] {}",
                        tr_args!(
                            "live_region_source_target",
                            source = format!("{:?}", output.source_region),
                            target = format!("{:?}", output.target)
                        )
                    );
                    ltbox_core::live!(
                        log,
                        "[Region] {}",
                        tr_args!(
                            "live_region_patched",
                            count = output.replacement_count.to_string(),
                            path = output.vendor_boot.display().to_string()
                        )
                    );
                    ltbox_core::live!(
                        log,
                        "[Region] {}",
                        tr_args!(
                            "live_region_final_vbmeta_written",
                            path = output.vbmeta.display().to_string()
                        )
                    );
                }
                Ok(ltbox_patch::region::RegionAvbBuild::Skipped {
                    source_region,
                    target,
                }) => {
                    // Inspection can prove that no patch is needed. Skip the
                    // write phase but still finish on the stable final phase.
                    ltbox_core::live!(log, "[Region] {}", phases.marker(4));
                    ltbox_core::live!(
                        log,
                        "[Region] {}",
                        tr_args!(
                            "live_region_source_target",
                            source = format!("{:?}", source_region),
                            target = format!("{:?}", target)
                        )
                    );
                    ltbox_core::live!(
                        log,
                        "[Region] {}",
                        ltbox_core::i18n::tr("live_region_source_matches_target")
                    );
                }
                Err(e) => {
                    return Err(tr_args!(
                        "err_region_conversion_failed",
                        error = e.to_string()
                    ));
                }
            }
        }
        AdvAction::PatchDevinfo => {
            // Country code lives in both devinfo.img
            // + persist.img — folder picker, at
            // least one must exist.
            use ltbox_patch::region::{EU_COUNTRY_CODES as EU, KNOWN_COUNTRY_CODES as KNOWN};
            let Some(new_code) = adv_country.as_deref() else {
                return Err(ltbox_core::i18n::tr("err_country_target_missing"));
            };
            if !input.is_dir() {
                return Err(tr_args!(
                    "err_country_folder_expected",
                    path = input.display().to_string()
                ));
            }
            let mut any_written = false;
            let mut any_found = false;
            for name in ["devinfo.img", "persist.img", "oemowninfo.img"] {
                let src = input.join(name);
                if !src.exists() {
                    ltbox_core::live!(
                        log,
                        "[Country] {}",
                        tr_args!("live_country_name_missing", name = name)
                    );
                    continue;
                }
                any_found = true;
                ltbox_core::live!(
                    log,
                    "[Country] {}",
                    tr_args!("live_country_processing", path = src.display().to_string())
                );
                let detected =
                    ltbox_patch::region::detect_country_code(&src, KNOWN, name == "persist.img")
                        .map_err(|e| {
                            tr_args!(
                                "err_country_detect_failed",
                                name = name,
                                error = e.to_string()
                            )
                        })?;
                let Some(old_code) = detected else {
                    ltbox_core::live!(
                        log,
                        "[Country] {}",
                        tr_args!("live_country_no_code_detected", name = name)
                    );
                    continue;
                };
                ltbox_core::live!(
                    log,
                    "[Country] {}",
                    tr_args!("live_country_detected", name = name, old_code = old_code)
                );
                let stem = std::path::Path::new(name)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| name.to_string());
                // v2 naming: `<stem>_modified.img`.
                let output = output_dir.join(format!("{stem}_modified.img"));
                match ltbox_patch::region::patch_country_code(
                    &src,
                    &output,
                    &old_code,
                    new_code,
                    EU,
                    name == "persist.img",
                ) {
                    Ok(true) => {
                        ltbox_core::live!(
                            log,
                            "[Country] {}",
                            tr_args!(
                                "live_country_written",
                                name = name,
                                old_code = old_code,
                                new_code = new_code,
                                path = output.display().to_string()
                            )
                        );
                        any_written = true;
                    }
                    Ok(false) => ltbox_core::live!(
                        log,
                        "[Country] {}",
                        tr_args!("live_country_no_replacements", name = name)
                    ),
                    Err(e) => {
                        return Err(tr_args!(
                            "err_country_patch_failed",
                            name = name,
                            error = e.to_string()
                        ));
                    }
                }
            }
            if !any_found {
                return Err(tr_args!(
                    "err_country_images_missing",
                    path = input.display().to_string()
                ));
            }
            if !any_written {
                ltbox_core::live!(
                    log,
                    "[Country] {}",
                    ltbox_core::i18n::tr("live_country_already_matches")
                );
            }
        }
        AdvAction::PatchArb => {
            let target = adv_arb_index
                .ok_or_else(|| ltbox_core::i18n::tr("err_patch_arb_target_missing"))?;
            patch_firmware_rollback(input, target, &phases, &mut log)?;
        }
        AdvAction::RebuildVbmeta => {
            ltbox_core::live!(log, "[AVB] {}", phases.marker(1));
            // `resign_image` alone won't work — matching Hash/Hashtree
            // descriptors go stale when dtbo / init_boot / vendor_boot change.
            let info = ltbox_patch::avb::extract_image_avb_info(input)
                .map_err(|e| tr_args!("err_vbmeta_inspect_failed", error = e.to_string()))?;
            // Only the two stock test keys embedded in
            // avbtool-rs are supported.
            let key_spec =
                ltbox_patch::key_map::key_spec_for_pubkey(info.public_key_sha1.as_deref())
                    .ok_or_else(|| {
                        ltbox_patch::key_map::unresolved_signing_key_error(
                            "vbmeta.img",
                            info.public_key_sha1.as_deref().unwrap_or_default(),
                        )
                    })?;
            let alg: Option<&str> = if info.algorithm == "NONE" {
                // NONE → infer from the resolved key spec.
                Some(if key_spec.contains("2048") {
                    "SHA256_RSA2048"
                } else {
                    "SHA256_RSA4096"
                })
            } else {
                Some(info.algorithm.as_str())
            };

            // Advanced is file-only — user supplies the partition images whose
            // embedded descriptors should be imported (v2 dumps them).
            let partition_image_candidates: &[&str] = &[
                "dtbo.img",
                "dtbo_a.img",
                "dtbo_b.img",
                "init_boot.img",
                "init_boot_a.img",
                "init_boot_b.img",
                "vendor_boot.img",
                "vendor_boot_a.img",
                "vendor_boot_b.img",
                "boot.img",
                "boot_a.img",
                "boot_b.img",
            ];
            let mut partition_images: Vec<std::path::PathBuf> = Vec::new();
            for name in partition_image_candidates {
                let p = parent.join(name);
                if p.exists() {
                    partition_images.push(p);
                }
            }
            ltbox_core::live!(log, "[AVB] {}", phases.marker(2));
            if partition_images.is_empty() {
                ltbox_core::live!(
                    log,
                    "[AVB] {}",
                    ltbox_core::i18n::tr("live_avb_no_partition_images_fallback")
                );
                if let Err(e) = ltbox_patch::avb::resign_image(
                    input,
                    key_spec,
                    alg.unwrap_or("SHA256_RSA4096"),
                    Some(info.rollback_index),
                ) {
                    return Err(tr_args!(
                        "err_vbmeta_rebuild_fallback_failed",
                        error = e.to_string()
                    ));
                }
            } else {
                if partition_images.iter().any(|p| {
                    p.file_name()
                        .and_then(|s| s.to_str())
                        .map(|s| s.starts_with("vendor_boot"))
                        .unwrap_or(false)
                }) {
                    ltbox_core::live!(
                        log,
                        "[AVB] {}",
                        ltbox_core::i18n::tr("live_avb_rebuild_warning")
                    );
                }
                let output = output_dir.join("vbmeta.rebuilt.img");
                let partition_image_refs: Vec<&std::path::Path> =
                    partition_images.iter().map(|p| p.as_path()).collect();
                let partition_image_names = partition_images
                    .iter()
                    .map(|p| p.file_name().and_then(|s| s.to_str()).unwrap_or(""))
                    .collect::<Vec<_>>()
                    .join(", ");
                ltbox_core::live!(
                    log,
                    "[AVB] {}",
                    tr_args!(
                        "live_avb_rebuild_partition_images",
                        count = partition_images.len().to_string(),
                        names = partition_image_names
                    )
                );
                ltbox_core::live!(
                    log,
                    "[AVB] {}",
                    tr_args!(
                        "live_avb_rebuild_key_alg",
                        key = key_spec,
                        alg = alg.unwrap_or("(from original vbmeta)")
                    )
                );
                if let Err(e) = ltbox_patch::avb::rebuild_vbmeta_with_partition_descriptors(
                    &output,
                    input,
                    &partition_image_refs,
                    key_spec,
                    alg,
                ) {
                    return Err(tr_args!("err_vbmeta_rebuild_failed", error = e.to_string()));
                }
                ltbox_core::live!(
                    log,
                    "[AVB] {}",
                    tr_args!(
                        "live_avb_rebuilt_written",
                        path = output.display().to_string()
                    )
                );
            }
            ltbox_core::live!(log, "[AVB] {}", phases.marker(3));
        }
    }
    ltbox_core::live!(
        log,
        "[Advanced] {}",
        tr_args!("live_advanced_completed", action = action_label)
    );
    Ok(log)
}
