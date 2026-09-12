use std::path::Path;

use crate::active_slot_suffix;
use ltbox_patch::rollback::{ManualRollbackPlan, RollbackIndices, plan_manual_rollback};

/// Validate the optional manual targets against the device floors and the
/// firmware indices read from the two rollback-protected AVB images.
pub(crate) fn prepare_manual_rollback_plan(
    fw_dir: &Path,
    device_floors: RollbackIndices,
    requested: Option<RollbackIndices>,
) -> Result<ManualRollbackPlan, String> {
    let requested =
        requested.ok_or_else(|| ltbox_core::i18n::tr("rollback_manual_error_missing"))?;
    let firmware = RollbackIndices {
        boot: inspect_firmware_index(fw_dir, "boot", "boot.img")?,
        vbmeta_system: inspect_firmware_index(fw_dir, "vbmeta_system", "vbmeta_system.img")?,
    };

    plan_manual_rollback(firmware, device_floors, requested).map_err(|error| {
        ltbox_core::tr_args!(
            "err_flash_manual_rollback_below_floor",
            partition = error.partition,
            requested = error.requested.to_string(),
            floor = error.floor.to_string()
        )
    })
}

fn inspect_firmware_index(fw_dir: &Path, image: &str, filename: &str) -> Result<u64, String> {
    let source = fw_dir.join(filename);
    ltbox_patch::avb::extract_image_avb_info(&source)
        .map(|info| info.rollback_index)
        .map_err(|error| {
            ltbox_core::tr_args!(
                "err_patch_arb_inspect_failed",
                image = image,
                error = error.to_string()
            )
        })
}

/// Build overlays only for partitions whose manual target changes its
/// firmware index. Unchanged partitions remain untouched so their original
/// bytes and signing keys stay in the ordinary firmware flash path.
pub(crate) fn build_manual_rollback_overlays(
    fw_dir: &Path,
    work_dir: &Path,
    plan: ManualRollbackPlan,
    log: &mut Vec<String>,
) -> Result<Vec<crate::arb_overlay::ArbOverlay>, String> {
    if !plan.changes_indices() {
        return Ok(Vec::new());
    }

    let _ = std::fs::remove_dir_all(work_dir);
    std::fs::create_dir_all(work_dir)
        .map_err(|error| ltbox_core::tr_args!("err_arb_work_dir_failed", error = error))?;

    let mut overlays = Vec::new();
    for (log_name, filename, target, changed) in [
        ("boot", "boot.img", plan.targets.boot, plan.boot_changed),
        (
            "vbmeta_system",
            "vbmeta_system.img",
            plan.targets.vbmeta_system,
            plan.vbmeta_system_changed,
        ),
    ] {
        if !changed {
            continue;
        }

        let source = fw_dir.join(filename);
        let image_info = ltbox_patch::avb::extract_image_avb_info(&source).map_err(|error| {
            ltbox_core::tr_args!(
                "err_patch_arb_inspect_failed",
                image = log_name,
                error = error.to_string()
            )
        })?;
        let key_from_map = match ltbox_patch::key_map::key_spec_for_signed_pubkey(
            image_info.public_key_sha1.as_deref(),
        ) {
            Ok(spec) => spec,
            Err(sha) => {
                return Err(ltbox_patch::key_map::unresolved_signing_key_error(
                    log_name, &sha,
                ));
            }
        };

        let patched = work_dir.join(format!("{log_name}.manual.img"));
        let patch_result: Result<(), String> = if log_name == "vbmeta_system" {
            match key_from_map {
                Some(spec) => {
                    std::fs::copy(&source, &patched)
                        .map_err(|error| format!("copy vbmeta_system: {error}"))?;
                    ltbox_patch::avb::resign_image(
                        &patched,
                        spec,
                        &image_info.algorithm,
                        Some(target),
                    )
                    .map_err(|error| format!("resign {log_name}: {error}"))
                }
                None => Err(ltbox_core::tr_args!(
                    "err_patch_arb_resign_failed",
                    image = log_name,
                    error = "unsigned image; cannot stage required ARB overlay"
                )),
            }
        } else if image_info.algorithm == "NONE" {
            std::fs::copy(&source, &patched).map_err(|error| format!("copy chained: {error}"))?;
            ltbox_patch::avb::add_hash_footer(&patched, &image_info, key_from_map, Some(target))
                .map_err(|error| format!("patch {log_name}: {error}"))
        } else if let Some(spec) = key_from_map {
            std::fs::copy(&source, &patched).map_err(|error| format!("copy chained: {error}"))?;
            ltbox_patch::avb::resign_image(&patched, spec, &image_info.algorithm, Some(target))
                .map_err(|error| format!("resign {log_name}: {error}"))
        } else {
            Err(ltbox_core::tr_args!(
                "err_patch_arb_resign_failed",
                image = log_name,
                error = "unsigned image; cannot stage required ARB overlay"
            ))
        };

        if let Err(error) = patch_result {
            let error =
                ltbox_core::tr_args!("live_arb_patch_failed", name = log_name, error = error);
            ltbox_core::live!(log, "[ARB] {error}");
            return Err(error);
        }

        let lun = ltbox_core::partition_lun::lun_for_partition(log_name)
            .ok_or_else(|| format!("no hardcoded LUN for {log_name}"))?;
        ltbox_core::live!(
            log,
            "[ARB] {}",
            ltbox_core::tr_args!(
                "live_arb_manual_prepared",
                name = log_name,
                path = patched.display(),
                target = target.to_string()
            )
        );
        overlays.push((
            format!("{log_name}{}", active_slot_suffix(None)),
            lun,
            patched,
        ));
    }

    Ok(overlays)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(path: &Path, index: u64, signed: bool) {
        avbtool_rs::builder::make_vbmeta_image(
            path,
            &avbtool_rs::builder::VbmetaImageArgs {
                algorithm_name: if signed { "SHA256_RSA2048" } else { "NONE" }.into(),
                key_spec: signed.then(|| "testkey_rsa2048".into()),
                public_key_metadata: None,
                rollback_index: index,
                flags: 0,
                rollback_index_location: 3,
                properties: vec![],
                kernel_cmdlines: vec![],
                extra_descriptors: vec![],
                include_descriptors_from_images: vec![],
                chain_partitions: vec![],
                release_string: None,
                append_to_release_string: None,
                padding_size: 4096,
            },
        )
        .unwrap();
    }

    #[test]
    fn manual_unchanged_images_skip_signing_and_preserve_workspace() {
        let dir = tempfile::tempdir().unwrap();
        // No private key exists for these unsigned fixtures. Unchanged Manual
        // values must return before key resolution or any output modification.
        image(&dir.path().join("boot.img"), 100, false);
        image(&dir.path().join("vbmeta_system.img"), 80, false);
        let original = std::fs::read(dir.path().join("boot.img")).unwrap();
        let targets = RollbackIndices {
            boot: 100,
            vbmeta_system: 80,
        };
        let plan = prepare_manual_rollback_plan(dir.path(), targets, Some(targets)).unwrap();
        let work = dir.path().join("work");
        std::fs::create_dir(&work).unwrap();
        std::fs::write(work.join("existing"), b"preserve").unwrap();
        assert!(
            build_manual_rollback_overlays(dir.path(), &work, plan, &mut vec![])
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            std::fs::read(dir.path().join("boot.img")).unwrap(),
            original
        );
        assert_eq!(std::fs::read(work.join("existing")).unwrap(), b"preserve");
    }

    #[test]
    fn offline_edit_keeps_exact_backups_and_distinct_targets() {
        let dir = tempfile::tempdir().unwrap();
        image(&dir.path().join("boot.img"), 100, true);
        image(&dir.path().join("vbmeta_system.img"), 80, true);
        let boot = std::fs::read(dir.path().join("boot.img")).unwrap();
        let vbmeta = std::fs::read(dir.path().join("vbmeta_system.img")).unwrap();
        let phases = crate::PhaseReporter::from_labels(vec!["test".into(); 4]);
        let targets = crate::ManualRollbackIndices {
            boot: 90,
            vbmeta_system: 75,
        };
        crate::workers::advanced::patch_firmware_rollback(
            dir.path(),
            targets,
            &phases,
            &mut vec![],
        )
        .unwrap();
        assert_eq!(
            std::fs::read(dir.path().join("boot.img.bak")).unwrap(),
            boot
        );
        assert_eq!(
            std::fs::read(dir.path().join("vbmeta_system.img.bak")).unwrap(),
            vbmeta
        );
        for (name, expected) in [("boot.img", 90), ("vbmeta_system.img", 75)] {
            let info = ltbox_patch::avb::extract_image_avb_info(&dir.path().join(name)).unwrap();
            assert_eq!(info.rollback_index, expected);
            assert_eq!(info.algorithm, "SHA256_RSA2048");
        }
        let edited = std::fs::read(dir.path().join("boot.img")).unwrap();
        assert!(
            crate::workers::advanced::patch_firmware_rollback(
                dir.path(),
                targets,
                &phases,
                &mut vec![]
            )
            .is_err()
        );
        assert_eq!(std::fs::read(dir.path().join("boot.img")).unwrap(), edited);
        assert_eq!(
            std::fs::read(dir.path().join("boot.img.bak")).unwrap(),
            boot
        );
    }

    #[test]
    fn offline_source_next_opens_shared_editor_before_confirmation() {
        use crate::{AdvAction, AdvMsg, App, FlashMsg, Message, View};
        let dir = tempfile::tempdir().unwrap();
        image(&dir.path().join("boot.img"), 100, true);
        image(&dir.path().join("vbmeta_system.img"), 80, true);
        let mut app = App {
            current_view: View::Advanced,
            ..App::default()
        };
        app.adv_wizard.open(AdvAction::PatchArb);
        app.adv_wizard.file_path = Some(dir.path().to_string_lossy().into_owned());
        let _ = app.update(Message::Adv(AdvMsg::AdvWizNext));
        assert_eq!(app.adv_wizard.arb_inspect, Some((100, 80)));
        assert_eq!(app.adv_wizard.step, 0);
        assert!(app.manual_rollback_editor.is_some());
        assert!(!app.operation.is_running());
        let _ = app.update(Message::Flash(FlashMsg::FlashManualRollbackConfirm));
        assert!(app.adv_wizard.is_confirm_step());
        assert!(!dir.path().join("boot.img.bak").exists());
    }

    #[test]
    fn offline_inspection_failure_leaves_both_sources_untouched() {
        let dir = tempfile::tempdir().unwrap();
        image(&dir.path().join("boot.img"), 100, true);
        std::fs::write(dir.path().join("vbmeta_system.img"), b"invalid").unwrap();
        let before = std::fs::read(dir.path().join("boot.img")).unwrap();
        let phases = crate::PhaseReporter::from_labels(vec!["test".into(); 4]);
        assert!(
            crate::workers::advanced::patch_firmware_rollback(
                dir.path(),
                crate::ManualRollbackIndices {
                    boot: 90,
                    vbmeta_system: 75
                },
                &phases,
                &mut vec![]
            )
            .is_err()
        );
        assert_eq!(std::fs::read(dir.path().join("boot.img")).unwrap(), before);
        assert!(!dir.path().join("boot.img.bak").exists());
    }

    #[test]
    fn manual_only_changed_partition_is_resigned_to_exact_target() {
        let dir = tempfile::tempdir().unwrap();
        image(&dir.path().join("boot.img"), 100, true);
        image(&dir.path().join("vbmeta_system.img"), 80, true);
        let original = std::fs::read(dir.path().join("vbmeta_system.img")).unwrap();
        let plan = prepare_manual_rollback_plan(
            dir.path(),
            RollbackIndices {
                boot: 90,
                vbmeta_system: 70,
            },
            Some(RollbackIndices {
                boot: 100,
                vbmeta_system: 75,
            }),
        )
        .unwrap();
        let overlays =
            build_manual_rollback_overlays(dir.path(), &dir.path().join("work"), plan, &mut vec![])
                .unwrap();
        assert_eq!(overlays.len(), 1);
        assert_eq!(overlays[0].0, "vbmeta_system_a");
        let output = ltbox_patch::avb::extract_image_avb_info(&overlays[0].2).unwrap();
        assert_eq!(output.rollback_index, 75);
        assert_eq!(output.algorithm, "SHA256_RSA2048");
        assert_eq!(
            std::fs::read(dir.path().join("vbmeta_system.img")).unwrap(),
            original
        );
    }

    #[test]
    fn manual_unchanged_but_below_floor_aborts_before_staging() {
        let dir = tempfile::tempdir().unwrap();
        image(&dir.path().join("boot.img"), 100, false);
        image(&dir.path().join("vbmeta_system.img"), 80, false);
        assert!(
            prepare_manual_rollback_plan(
                dir.path(),
                RollbackIndices {
                    boot: 101,
                    vbmeta_system: 80
                },
                Some(RollbackIndices {
                    boot: 100,
                    vbmeta_system: 80
                })
            )
            .is_err()
        );
        assert!(
            prepare_manual_rollback_plan(
                dir.path(),
                RollbackIndices {
                    boot: 100,
                    vbmeta_system: 80
                },
                None
            )
            .is_err()
        );
        std::fs::remove_file(dir.path().join("boot.img")).unwrap();
        assert!(
            prepare_manual_rollback_plan(
                dir.path(),
                RollbackIndices {
                    boot: 0,
                    vbmeta_system: 0
                },
                Some(RollbackIndices {
                    boot: 100,
                    vbmeta_system: 80
                })
            )
            .is_err()
        );
    }
}
