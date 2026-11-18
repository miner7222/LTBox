//! Unroot worker: restore a stock root image — plus vbmeta when the root run
//! rebuilt it — from a backup folder over EDL.

use crate::{
    ConnectionStatus, PhaseReporter, UnrootType, find_edl_loader, open_edl_session,
    transition_to_edl,
};
use ltbox_core::{i18n::tr, live, tr_args};

use std::path::{Path, PathBuf};

use ltbox_patch::avb::{self, VbmetaOwner};

use super::root_backup::{BackupContents, BackupRootTarget, resolve_backup_contents};

pub(crate) fn unroot_connection_ready(conn: ConnectionStatus) -> bool {
    matches!(
        conn,
        ConnectionStatus::Adb | ConnectionStatus::AdbRecovery | ConnectionStatus::Fastboot
    )
}

pub(crate) fn unroot_worker(
    folder: String,
    unroot_type: UnrootType,
    loader_override: Option<String>,
    device_model: String,
    conn: ConnectionStatus,
    phases: PhaseReporter,
) -> Result<Vec<String>, String> {
    let mut log = Vec::new();
    // Where an abort has to leave the device. Captured before `transition_to_edl`
    // moves it: a user who was already in EDL stays there, anyone else goes back
    // to system.
    let edl_start = matches!(conn, ConnectionStatus::Edl);
    if !ltbox_core::model::capabilities(&device_model).unroot {
        return Err(tr_args!("model_unsupported", model = device_model.as_str()));
    }
    if !unroot_connection_ready(conn) {
        return Err(tr("err_unroot_connection_required"));
    }
    let dir = std::path::Path::new(&folder);

    live!(log, "[Unroot] {}", phases.marker(1));

    let backup = resolve_backup_contents(dir, unroot_type, &device_model)?;
    let root_target = backup.root_target;
    let root_image_name = root_target.filename();
    let base_part = root_target.partition_base();
    let root_image_path = dir.join(root_image_name);
    // A root run that left vbmeta untouched backed none up, and the stock
    // vbmeta is still on the device — restoring the root image alone puts the
    // pair back in sync.
    let vbmeta_path = backup.vbmeta_owner.map(|owner| dir.join(owner.filename()));
    if !root_image_path.exists() {
        return Err(tr_args!(
            "err_unroot_image_missing",
            image = root_image_name
        ));
    }
    if vbmeta_path.as_ref().is_some_and(|path| !path.exists()) {
        return Err(tr("err_unroot_vbmeta_missing"));
    }
    if let Ok(info) = ltbox_patch::avb::extract_image_avb_info(&root_image_path)
        && let Some(fingerprint) = ltbox_patch::avb::build_fingerprint(&info)
        && let Some(model) =
            ltbox_core::model::fingerprint_model_lacking(&fingerprint, |c| c.unroot)
    {
        return Err(tr_args!("model_unsupported", model = model));
    }
    if ltbox_core::model::capabilities(&device_model).root_uses_gbl {
        // The device-side comparison below is impossible on the GBL route, but
        // the backup itself can still be checked before anything is written.
        check_gbl_backup(
            &root_image_path,
            vbmeta_path.as_deref(),
            root_image_name,
            base_part,
            &device_model,
        )?;
    } else {
        ltbox_patch::avb::verify_root_backup(&root_image_path, vbmeta_path.as_deref(), base_part)
            .map_err(|error| {
            tr_args!(
                "err_unroot_image_avb_failed",
                image = root_image_name,
                error = error
            )
        })?;
    }
    // Names the images this run will write, so the pre-flight line and the
    // Phase 4 header agree with what actually gets flashed.
    let restored_label = if vbmeta_path.is_some() {
        tr_args!(
            "live_unroot_backup_pair",
            root_image = root_image_name,
            image = backup
                .vbmeta_owner
                .ok_or_else(|| tr("err_unroot_vbmeta_missing"))?
                .filename()
        )
    } else {
        root_image_name.to_string()
    };
    live!(log, "[Unroot] {}", restored_label);

    // Slot resolution must succeed — unroot writes the filename-resolved root
    // target + vbmeta_<slot> from the user's backup folder. Defaulting to
    // `_a` when the device is on `_b` would restore stale stock blobs to the
    // wrong slot and leave the active slot still rooted, with no clear
    // signal to the user.
    let slot =
        ltbox_device::controller::poll_active_slot(std::time::Duration::from_secs(30), &mut log)
            .map_err(|e| tr_args!("err_unroot_slot_resolve_failed", error = e))?;

    // An explicit picker or remembered choice takes priority. Fall back to
    // scanning the backup folder only when no override was set, for users
    // who still ship a loader alongside the backup images.
    let loader = match loader_override.clone() {
        Some(p) => std::path::PathBuf::from(p),
        None => find_edl_loader(dir)
            .or_else(|| dir.parent().and_then(find_edl_loader))
            .ok_or_else(|| tr_args!("err_unroot_loader_missing_under", path = dir.display()))?,
    };
    live!(
        log,
        "[Unroot] {}",
        tr_args!(
            "live_unroot_loader_path",
            path = loader.display().to_string()
        )
    );

    // The root image + vbmeta resolve through the
    // hardcoded LUN map; GPT-by-name reads
    // the slot's start sector from the
    // device. No rawprogram parse needed —
    // the loader's parent dir may not even
    // contain a firmware XML pair.
    let root_image_label = format!("{base_part}{slot}");
    let vbm_label = backup
        .vbmeta_owner
        .map(|owner| format!("{}{slot}", owner.partition_base()))
        .unwrap_or_default();
    let root_image_lun = ltbox_core::partition_lun::lun_for_partition(base_part)
        .ok_or_else(|| tr_args!("err_no_hardcoded_lun", partition = base_part))?;
    let vbm_lun = backup
        .vbmeta_owner
        .map(VbmetaOwner::lun)
        .transpose()
        .map_err(|e| e.to_string())?;
    if vbmeta_path.is_some() {
        live!(
            log,
            "[Unroot] {}",
            tr_args!(
                "log_unroot_lun_resolved",
                root_image_label = root_image_label,
                root_image_lun = root_image_lun,
                vbm_label = vbm_label,
                vbm_lun = vbm_lun.ok_or_else(|| tr("err_unroot_vbmeta_missing"))?,
            )
        );
    } else {
        live!(
            log,
            "[Unroot] {}",
            tr_args!(
                "log_unroot_lun_resolved_root_only",
                root_image_label = root_image_label,
                root_image_lun = root_image_lun,
            )
        );
    }

    live!(log, "[Unroot] {}", phases.marker(2));
    transition_to_edl(conn, &mut log)?;

    live!(log, "[Unroot] {}", phases.marker(3));
    let mut session = open_edl_session(&loader, &mut log)?;

    live!(log, "[Unroot] {}", phases.marker(4));
    // Nothing to compare on the testkey efisp/GBL route: its root run never ran
    // AVB post-processing, so the backup images hold no AVB metadata and the
    // pre-verification restore path is the correct one.
    let (root_image_path, vbmeta_path) =
        if ltbox_core::model::capabilities(&device_model).root_uses_gbl {
            live!(log, "[Unroot] {}", tr("live_unroot_verify_skipped"));
            (root_image_path, vbmeta_path)
        } else {
            match verify_against_device(
                &mut session,
                backup,
                &slot,
                &root_image_path,
                vbmeta_path.as_deref(),
                &mut log,
            ) {
                Ok(verified) => verified,
                Err(error) => {
                    if edl_start {
                        let _ = session.reset_to_edl(&mut log);
                    } else {
                        let _ = session.reset(&mut log);
                    }
                    return Err(error);
                }
            }
        };

    live!(log, "[Unroot] {} ({restored_label})", phases.marker(5));
    let mut requests = vec![ltbox_device::edl::PartitionFlash {
        label: &root_image_label,
        image: &root_image_path,
        slot: 0,
        lun: root_image_lun,
    }];
    if let Some(path) = &vbmeta_path {
        requests.push(ltbox_device::edl::PartitionFlash {
            label: &vbm_label,
            image: path,
            slot: 0,
            lun: vbm_lun.ok_or_else(|| tr("err_unroot_vbmeta_missing"))?,
        });
    }
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
            tr_args!(
                "err_unroot_flash_failed",
                label = e.partition,
                error = e.source
            )
        })?;

    live!(log, "[Unroot] {}", phases.marker(6));
    session.reset_tolerant(&mut log);
    live!(
        log,
        "[Unroot] {}",
        ltbox_core::i18n::tr("live_image_flash_completed")
    );
    Ok(log)
}

/// What the pre-flash comparison against the device concluded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnrootPrep {
    /// Device and backup agree — write the selected images untouched.
    FlashAsIs,
    /// Same firmware, but the device's `boot` sits at a different rollback
    /// index. Restore the backup image at the device's value instead.
    PatchRollback(u64),
    Abort(AbortReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AbortReason {
    /// A build fingerprint is missing on one side, so "same firmware" cannot
    /// be proven either way.
    FingerprintUnreadable,
    /// Both fingerprints read, and they name different builds.
    FingerprintMismatch,
}

/// Decide what to do with the selected backup image given what the device
/// actually holds. Pure, so the brick-critical policy is testable without an
/// EDL session.
///
/// A user who restores from *some* folder of the same firmware version rather
/// than their own backup gets images that never received this device's
/// rollback-bypass treatment; flashing those unchanged is what bricks the
/// device. The payload retains its existing Boot-only reconciliation policy;
/// the descriptor-detected owner is checked independently below.
/// Self-check a GBL-route backup, which cannot be compared with the device:
/// the patched image on the device carries no AVB metadata.
///
/// A backup with AVB metadata (every backup current Root writes: it requires
/// a fingerprinted dump) must pass its own hash check and must not name
/// another model. One without any AVB footer must at least be an Android boot
/// image.
fn check_gbl_backup(
    image: &Path,
    vbmeta: Option<&Path>,
    image_name: &str,
    partition: &str,
    device_model: &str,
) -> Result<(), String> {
    let failed = |error: String| {
        tr_args!(
            "err_unroot_image_avb_failed",
            image = image_name,
            error = error
        )
    };
    match avb::extract_image_avb_info(image) {
        Ok(info) => {
            avb::verify_root_backup(image, vbmeta, partition)
                .map_err(|error| failed(error.to_string()))?;
            // As Root does: a fingerprint naming no known model (a custom
            // ROM) cannot contradict the device; one naming another does.
            if let Some(fingerprint) = avb::build_fingerprint(&info)
                && ltbox_core::model::fingerprint_names_known_model(&fingerprint)
                && !crate::fingerprint_token_match(&fingerprint, device_model)
            {
                return Err(tr_args!(
                    "err_unroot_fingerprint_mismatch",
                    device = device_model,
                    image = fingerprint
                ));
            }
            Ok(())
        }
        Err(_) => {
            let mut magic = [0u8; 8];
            let is_boot_image = std::fs::File::open(image)
                .and_then(|mut file| std::io::Read::read_exact(&mut file, &mut magic))
                .is_ok()
                && &magic == b"ANDROID!";
            if is_boot_image {
                Ok(())
            } else {
                Err(failed("not an Android boot image".into()))
            }
        }
    }
}

fn unroot_prep(
    target: BackupRootTarget,
    device_fingerprint: Option<&str>,
    image_fingerprint: Option<&str>,
    device_index: u64,
    image_index: u64,
) -> UnrootPrep {
    let (Some(device_fingerprint), Some(image_fingerprint)) =
        (device_fingerprint, image_fingerprint)
    else {
        return UnrootPrep::Abort(AbortReason::FingerprintUnreadable);
    };
    if device_fingerprint != image_fingerprint {
        return UnrootPrep::Abort(AbortReason::FingerprintMismatch);
    }
    if target == BackupRootTarget::Boot && device_index != image_index {
        return UnrootPrep::PatchRollback(device_index);
    }
    UnrootPrep::FlashAsIs
}

/// Dump the partition this run would overwrite, compare it against the selected
/// backup, and return the images to actually flash — the originals when they
/// already agree with the device, patched copies when they do not.
///
/// Fails closed: a partition that will not dump or parse leaves the device's
/// rollback state unknown, which is exactly the case this check exists to keep
/// from being flashed blind.
fn verify_against_device(
    session: &mut ltbox_device::edl::EdlSession,
    backup: BackupContents,
    slot: &str,
    root_image_path: &Path,
    vbmeta_path: Option<&Path>,
    log: &mut Vec<String>,
) -> Result<(PathBuf, Option<PathBuf>), String> {
    let base = backup.root_target.partition_base();
    let device_label = format!("{base}{slot}");
    let lun = ltbox_core::partition_lun::lun_for_partition(base)
        .ok_or_else(|| tr_args!("err_no_hardcoded_lun", partition = base))?;

    let work_dir = ltbox_core::app_paths::work_dir_for("unroot_verify");
    let _ = std::fs::remove_dir_all(&work_dir);
    std::fs::create_dir_all(&work_dir)
        .map_err(|error| tr_args!("err_arb_work_dir_failed", error = error))?;

    live!(
        log,
        "[Unroot] {}",
        tr_args!("live_unroot_verify_dump", label = device_label)
    );
    let device_info = super::flash::dump_avb_info(session, &device_label, lun, &work_dir, log)
        .map_err(|error| {
            live!(log, "[Unroot] {error}");
            tr_args!("err_unroot_device_dump_failed", label = device_label)
        })?
        .ok_or_else(|| tr_args!("err_unroot_device_dump_failed", label = device_label))?;
    let image_info =
        ltbox_patch::avb::extract_image_avb_info(root_image_path).map_err(|error| {
            tr_args!(
                "err_unroot_image_avb_failed",
                image = backup.root_target.filename(),
                error = error.to_string()
            )
        })?;

    let device_fingerprint = ltbox_patch::avb::build_fingerprint(&device_info);
    let image_fingerprint = ltbox_patch::avb::build_fingerprint(&image_info);
    let target_index = match unroot_prep(
        backup.root_target,
        device_fingerprint.as_deref(),
        image_fingerprint.as_deref(),
        device_info.rollback_index,
        image_info.rollback_index,
    ) {
        UnrootPrep::Abort(AbortReason::FingerprintUnreadable) => {
            return Err(tr("err_unroot_fingerprint_unreadable"));
        }
        UnrootPrep::Abort(AbortReason::FingerprintMismatch) => {
            return Err(tr_args!(
                "err_unroot_fingerprint_mismatch",
                device = device_fingerprint.unwrap_or_default(),
                image = image_fingerprint.unwrap_or_default()
            ));
        }
        UnrootPrep::FlashAsIs => {
            live!(
                log,
                "[Unroot] {}",
                tr_args!(
                    "live_unroot_verify_match",
                    label = device_label,
                    index = device_info.rollback_index.to_string()
                )
            );
            None
        }
        UnrootPrep::PatchRollback(index) => Some(index),
    };

    let patched_root = if let Some(target_index) = target_index {
        patch_root_rollback(
            root_image_path,
            &image_info,
            backup.root_target,
            target_index,
            &work_dir,
            log,
        )?
    } else {
        root_image_path.to_path_buf()
    };
    let Some(owner) = backup.vbmeta_owner else {
        return Ok((patched_root, None));
    };
    let owner_src = vbmeta_path.ok_or_else(|| tr("err_unroot_vbmeta_missing"))?;
    let owner_label = format!("{}{slot}", owner.partition_base());
    live!(
        log,
        "[Unroot] {}",
        tr_args!("live_unroot_verify_dump", label = owner_label)
    );
    let device_owner = super::flash::dump_avb_info(
        session,
        &owner_label,
        owner.lun().map_err(|e| e.to_string())?,
        &work_dir,
        log,
    )
    .map_err(|error| {
        live!(log, "[Unroot] {error}");
        tr_args!("err_unroot_device_dump_failed", label = owner_label)
    })?
    .ok_or_else(|| tr_args!("err_unroot_device_dump_failed", label = owner_label))?;
    let restored_owner = reconcile_owner(
        owner,
        owner_src,
        &device_owner,
        target_index.map(|_| patched_root.as_path()),
        base,
        &work_dir,
        log,
    )?;
    Ok((patched_root, Some(restored_owner)))
}

fn patch_root_rollback(
    root_image_path: &Path,
    image_info: &avb::AvbImageInfo,
    target: BackupRootTarget,
    target_index: u64,
    work_dir: &Path,
    log: &mut Vec<String>,
) -> Result<PathBuf, String> {
    let base = target.partition_base();
    live!(
        log,
        "[Unroot] {}",
        tr_args!(
            "live_unroot_rollback_patch",
            image = target.filename(),
            from = image_info.rollback_index.to_string(),
            to = target_index.to_string()
        )
    );
    let patched = work_dir.join(target.filename());
    let patch_failed = |error: String| {
        tr_args!(
            "err_unroot_rollback_patch_failed",
            image = target.filename(),
            error = error
        )
    };
    std::fs::copy(root_image_path, &patched).map_err(|error| patch_failed(error.to_string()))?;
    // Same key policy the firmware flash uses for its ARB overlays: the stock
    // signer has to resolve in KEY_MAP, or the restored image is one nothing
    // verifies.
    let key =
        ltbox_patch::key_map::key_spec_for_signed_pubkey(image_info.public_key_sha1.as_deref())
            .map_err(|sha1| ltbox_patch::key_map::unresolved_signing_key_error(base, &sha1))?;
    // A `NONE` footer is hashed by vbmeta rather than self-signed; re-adding the
    // footer rewrites only the metadata appended after the payload.
    if image_info.algorithm == "NONE" {
        ltbox_patch::avb::add_hash_footer(&patched, image_info, key, Some(target_index))
            .map_err(|error| patch_failed(error.to_string()))?;
    } else if let Some(spec) = key {
        ltbox_patch::avb::resign_image(&patched, spec, &image_info.algorithm, Some(target_index))
            .map_err(|error| patch_failed(error.to_string()))?;
    } else {
        return Err(patch_failed("unsigned image".to_string()));
    }

    Ok(patched)
}

/// Owners must never be restored below the device's rollback index. A newer
/// backup is retained; payload Boot reconciliation intentionally stays exact.
fn unroot_owner_prep(
    device_fingerprint: Option<&str>,
    image_fingerprint: Option<&str>,
    device_index: u64,
    image_index: u64,
) -> UnrootPrep {
    match unroot_prep(
        BackupRootTarget::InitBoot,
        device_fingerprint,
        image_fingerprint,
        device_index,
        image_index,
    ) {
        UnrootPrep::FlashAsIs if device_index > image_index => {
            UnrootPrep::PatchRollback(device_index)
        }
        result => result,
    }
}

fn reconcile_owner(
    owner: VbmetaOwner,
    source: &Path,
    device_info: &avb::AvbImageInfo,
    patched_root: Option<&Path>,
    base: &str,
    work_dir: &Path,
    log: &mut Vec<String>,
) -> Result<PathBuf, String> {
    let invalid = |error: String| {
        tr_args!(
            "err_unroot_image_avb_failed",
            image = owner.filename(),
            error = error
        )
    };
    let info = avb::extract_image_avb_info(source).map_err(|e| invalid(e.to_string()))?;
    let device_fp = avb::build_fingerprint(device_info);
    let image_fp = avb::build_fingerprint(&info);
    let index = match unroot_owner_prep(
        device_fp.as_deref(),
        image_fp.as_deref(),
        device_info.rollback_index,
        info.rollback_index,
    ) {
        UnrootPrep::Abort(AbortReason::FingerprintUnreadable) => {
            return Err(tr("err_unroot_fingerprint_unreadable"));
        }
        UnrootPrep::Abort(AbortReason::FingerprintMismatch) => {
            return Err(tr_args!(
                "err_unroot_fingerprint_mismatch",
                device = device_fp.unwrap_or_default(),
                image = image_fp.unwrap_or_default()
            ));
        }
        UnrootPrep::PatchRollback(index) => Some(index),
        UnrootPrep::FlashAsIs => None,
    };
    if index.is_none() && patched_root.is_none() {
        return Ok(source.to_path_buf());
    }
    let output = work_dir.join(owner.filename());
    let failed = |error: String| {
        tr_args!(
            "err_unroot_rollback_patch_failed",
            image = owner.filename(),
            error = error
        )
    };
    let key = ltbox_patch::key_map::key_spec_for_signed_pubkey(info.public_key_sha1.as_deref())
        .map_err(|sha1| {
            ltbox_patch::key_map::unresolved_signing_key_error(owner.filename(), &sha1)
        })?;
    if let (Some(root), Some(key)) = (patched_root, key) {
        avb::rebuild_root_vbmeta_owner(&output, source, root, base, key).map_err(|e| {
            tr_args!(
                "err_unroot_vbmeta_rebuild_failed",
                image = owner.filename(),
                error = e
            )
        })?;
        live!(
            log,
            "[Unroot] {}",
            tr_args!("live_unroot_vbmeta_rebuilt", image = owner.filename())
        );
    } else {
        // Keep the unsigned-owner copy policy used by Root.
        std::fs::copy(source, &output).map_err(|e| failed(e.to_string()))?;
    }
    if let Some(index) = index {
        let key = key.ok_or_else(|| failed("unsigned image".into()))?;
        live!(
            log,
            "[Unroot] {}",
            tr_args!(
                "live_unroot_rollback_patch",
                image = owner.filename(),
                from = info.rollback_index,
                to = index
            )
        );
        // Re-sign metadata in place: never add a hash footer to a mixed owner.
        avb::resign_image(&output, key, &info.algorithm, Some(index))
            .map_err(|e| failed(e.to_string()))?;
    }
    let final_info = avb::extract_image_avb_info(&output).map_err(|e| failed(e.to_string()))?;
    if final_info.rollback_index < device_info.rollback_index
        || final_info.rollback_index_location != info.rollback_index_location
        || final_info.algorithm != info.algorithm
        || final_info.flags != info.flags
        || final_info.public_key_sha1 != info.public_key_sha1
    {
        return Err(failed(
            "restored owner failed rollback or metadata postcondition".into(),
        ));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FP: &str = "qti/TB320FC/TB320FC:14/UP1A/S000123:user/release-keys";

    #[test]
    fn boot_owner_reconcile_rebuilds_then_resigns_for_a_higher_device_index() {
        use avbtool_rs::builder::{PropertySpec, VbmetaImageArgs, make_vbmeta_image};
        let temp = tempfile::tempdir().unwrap();
        let work = temp.path().join("work");
        std::fs::create_dir(&work).unwrap();
        let boot = temp.path().join("boot.img");
        std::fs::write(&boot, vec![0x41; 4096]).unwrap();
        avbtool_rs::footer::add_hash_footer(
            &boot,
            &avbtool_rs::footer::HashFooterArgs {
                partition_size: Some(128 * 1024),
                dynamic_partition_size: false,
                partition_name: "boot".into(),
                hash_algorithm: "sha256".into(),
                salt: Some(vec![1, 2]),
                chain_partitions: Vec::new(),
                algorithm_name: "NONE".into(),
                key_spec: None,
                public_key_metadata: None,
                rollback_index: 4,
                flags: 0,
                rollback_index_location: 3,
                properties: Vec::new(),
                kernel_cmdlines: Vec::new(),
                include_descriptors_from_images: Vec::new(),
                release_string: None,
                append_to_release_string: None,
                output_vbmeta_image: None,
                do_not_append_vbmeta_image: false,
                use_persistent_digest: false,
                do_not_use_ab: false,
            },
        )
        .unwrap();
        let source = temp.path().join("vbmeta.img");
        make_vbmeta_image(
            &source,
            &VbmetaImageArgs {
                algorithm_name: "SHA512_RSA2048".into(),
                key_spec: Some("testkey_rsa2048".into()),
                public_key_metadata: None,
                rollback_index: 4,
                flags: 0,
                rollback_index_location: 0,
                properties: vec![PropertySpec {
                    key: "com.android.build.system.fingerprint".into(),
                    value: FP.as_bytes().to_vec(),
                }],
                kernel_cmdlines: Vec::new(),
                extra_descriptors: Vec::new(),
                include_descriptors_from_images: vec![boot.clone()],
                chain_partitions: Vec::new(),
                release_string: None,
                append_to_release_string: None,
                padding_size: 4096,
            },
        )
        .unwrap();
        let before = avb::extract_image_avb_info(&source).unwrap();
        let mut device = before.clone();
        device.rollback_index = 7;
        let root_info = avb::extract_image_avb_info(&boot).unwrap();
        let patched_root = work.join("boot.img");
        // A different payload makes adopting the supplied hash observable;
        // its unsigned Boot footer also carries the reconciled payload index.
        std::fs::write(&patched_root, vec![0x42; 4096]).unwrap();
        avb::add_hash_footer(&patched_root, &root_info, None, Some(9)).unwrap();
        assert_eq!(
            avb::extract_image_avb_info(&patched_root)
                .unwrap()
                .rollback_index,
            9
        );
        let output = reconcile_owner(
            VbmetaOwner::Vbmeta,
            &source,
            &device,
            Some(&patched_root),
            "boot",
            &work,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(
            avb::hash_descriptor(&output, "boot").unwrap(),
            avb::hash_descriptor(&patched_root, "boot").unwrap()
        );
        assert_ne!(
            avb::hash_descriptor(&output, "boot").unwrap(),
            avb::hash_descriptor(&source, "boot").unwrap()
        );
        let after = avb::extract_image_avb_info(&output).unwrap();
        assert_eq!(after.rollback_index, device.rollback_index);
        assert_eq!(after.algorithm, before.algorithm);
        avbtool_rs::verify::verify_image(
            &output,
            &avbtool_rs::verify::VerifyImageOptions {
                key_blob: Some(avbtool_rs::crypto::extract_public_key("testkey_rsa2048").unwrap()),
                expected_chain_partitions: Vec::new(),
                follow_chain_partitions: false,
                accept_zeroed_hashtree: false,
            },
        )
        .unwrap();
    }

    #[test]
    fn owner_rollback_policy_never_lowers_the_device_floor() {
        assert_eq!(
            unroot_owner_prep(Some(FP), Some(FP), 7, 4),
            UnrootPrep::PatchRollback(7)
        );
        assert_eq!(
            unroot_owner_prep(Some(FP), Some(FP), 4, 7),
            UnrootPrep::FlashAsIs
        );
        assert_eq!(
            unroot_owner_prep(Some(FP), Some(FP), 7, 7),
            UnrootPrep::FlashAsIs
        );
        assert_eq!(
            unroot_owner_prep(None, Some(FP), 7, 4),
            UnrootPrep::Abort(AbortReason::FingerprintUnreadable)
        );
        assert_eq!(
            unroot_owner_prep(Some(FP), Some("other"), 7, 4),
            UnrootPrep::Abort(AbortReason::FingerprintMismatch)
        );
    }

    #[test]
    fn owner_rollback_reconcile_resigns_mixed_metadata_with_original_algorithm() {
        use avbtool_rs::builder::{PropertySpec, VbmetaImageArgs, make_vbmeta_image};
        for owner in [VbmetaOwner::Vbmeta, VbmetaOwner::VbmetaSystem] {
            let temp = tempfile::tempdir().unwrap();
            let source = temp.path().join(owner.filename());
            let work = temp.path().join("work");
            std::fs::create_dir(&work).unwrap();
            make_vbmeta_image(
                &source,
                &VbmetaImageArgs {
                    algorithm_name: "SHA512_RSA2048".into(),
                    key_spec: Some("testkey_rsa2048".into()),
                    public_key_metadata: None,
                    rollback_index: 4,
                    flags: 0,
                    rollback_index_location: 0,
                    properties: vec![PropertySpec {
                        key: "com.android.build.system.fingerprint".into(),
                        value: FP.as_bytes().to_vec(),
                    }],
                    kernel_cmdlines: Vec::new(),
                    extra_descriptors: vec![avbtool_rs::info::DescriptorInfo::Hash {
                        image_size: 4096,
                        hash_algorithm: "sha256".into(),
                        partition_name: "pvmfw".into(),
                        salt: vec![1],
                        digest: vec![2; 32],
                        flags: 0,
                    }],
                    include_descriptors_from_images: Vec::new(),
                    chain_partitions: Vec::new(),
                    release_string: None,
                    append_to_release_string: None,
                    padding_size: 4096,
                },
            )
            .unwrap();
            let original = std::fs::read(&source).unwrap();
            let before = avbtool_rs::image::inspect_avb_image(&source).unwrap();
            let mut device = avb::extract_image_avb_info(&source).unwrap();
            device.rollback_index = 7;
            let output = reconcile_owner(
                owner,
                &source,
                &device,
                None,
                "init_boot",
                &work,
                &mut Vec::new(),
            )
            .unwrap();
            assert_eq!(output.file_name().unwrap(), owner.filename());
            let after = avbtool_rs::image::inspect_avb_image(&output).unwrap();
            assert_eq!(after.header.rollback_index, 7);
            assert_eq!(
                after.header.rollback_index_location,
                before.header.rollback_index_location
            );
            assert_eq!(after.algorithm_name, before.algorithm_name);
            assert_eq!(after.descriptors, before.descriptors);
            assert_eq!(after.public_key_sha1, before.public_key_sha1);
            assert_eq!(std::fs::read(&source).unwrap(), original);
            let blob = avbtool_rs::image::load_vbmeta_blob(&output).unwrap();
            let header = &after.header;
            let aux = 256 + header.authentication_data_block_size as usize;
            let mut signed = blob[..256].to_vec();
            signed.extend_from_slice(&blob[aux..aux + header.auxiliary_data_block_size as usize]);
            let sig = 256 + header.signature_offset as usize;
            let key = avbtool_rs::crypto::AvbPublicKey::decode(
                &avbtool_rs::crypto::extract_public_key("testkey_rsa2048").unwrap(),
            )
            .unwrap();
            assert!(
                key.verify(
                    avbtool_rs::crypto::lookup_algorithm_by_name(&after.algorithm_name).unwrap(),
                    &blob[sig..sig + header.signature_size as usize],
                    &signed
                )
                .unwrap()
            );
            device.rollback_index = 3;
            assert_eq!(
                reconcile_owner(
                    owner,
                    &source,
                    &device,
                    None,
                    "init_boot",
                    &work,
                    &mut Vec::new()
                )
                .unwrap(),
                source
            );
            device.props.clear();
            assert!(
                reconcile_owner(
                    owner,
                    &source,
                    &device,
                    None,
                    "init_boot",
                    &work,
                    &mut Vec::new()
                )
                .is_err()
            );
        }
    }

    fn gbl_backup(dir: &Path, fingerprint: Option<&str>) -> PathBuf {
        let image = dir.join("init_boot.img");
        let mut body = b"ANDROID!".to_vec();
        body.resize(8 * 1024, 0x5a);
        std::fs::write(&image, body).unwrap();
        avbtool_rs::footer::add_hash_footer(
            &image,
            &avbtool_rs::footer::HashFooterArgs {
                partition_size: Some(128 * 1024),
                dynamic_partition_size: false,
                partition_name: "init_boot".into(),
                hash_algorithm: "sha256".into(),
                salt: Some(vec![7; 32]),
                chain_partitions: Vec::new(),
                algorithm_name: "NONE".into(),
                key_spec: None,
                public_key_metadata: None,
                rollback_index: 0,
                flags: 0,
                rollback_index_location: 0,
                properties: fingerprint
                    .map(|fp| avbtool_rs::builder::PropertySpec {
                        key: "com.android.build.init_boot.fingerprint".into(),
                        value: fp.as_bytes().to_vec(),
                    })
                    .into_iter()
                    .collect(),
                kernel_cmdlines: Vec::new(),
                include_descriptors_from_images: Vec::new(),
                release_string: None,
                append_to_release_string: None,
                output_vbmeta_image: None,
                do_not_append_vbmeta_image: false,
                use_persistent_digest: false,
                do_not_use_ab: false,
            },
        )
        .unwrap();
        image
    }

    const BALDUR_FP: &str = "Lenovo/TB323FU/TB323FU:15/AQ3A/ZUXOS_1.1.10:user/release-keys";

    #[test]
    fn gbl_backup_check_accepts_this_models_intact_backup() {
        let temp = tempfile::tempdir().unwrap();
        for fp in [
            Some(BALDUR_FP),
            Some("lineage_custom/foo/foo:15/x:user/test-keys"),
            None,
        ] {
            let image = gbl_backup(temp.path(), fp);
            check_gbl_backup(&image, None, "init_boot.img", "init_boot", "TB323FU")
                .unwrap_or_else(|error| panic!("{fp:?}: {error}"));
        }
    }

    #[test]
    fn gbl_backup_check_rejects_another_model_and_a_corrupted_image() {
        let temp = tempfile::tempdir().unwrap();
        let image = gbl_backup(temp.path(), Some(BALDUR_FP));
        assert!(check_gbl_backup(&image, None, "init_boot.img", "init_boot", "TB324ZC").is_err());

        let mut bytes = std::fs::read(&image).unwrap();
        bytes[100] ^= 0xff;
        std::fs::write(&image, bytes).unwrap();
        assert!(check_gbl_backup(&image, None, "init_boot.img", "init_boot", "TB323FU").is_err());
    }

    #[test]
    fn gbl_backup_without_avb_must_still_be_a_boot_image() {
        let temp = tempfile::tempdir().unwrap();
        let image = temp.path().join("init_boot.img");
        let mut legacy = b"ANDROID!".to_vec();
        legacy.resize(4096, 0);
        std::fs::write(&image, &legacy).unwrap();
        check_gbl_backup(&image, None, "init_boot.img", "init_boot", "TB323FU").unwrap();
        std::fs::write(&image, vec![0u8; 4096]).unwrap();
        assert!(check_gbl_backup(&image, None, "init_boot.img", "init_boot", "TB323FU").is_err());
    }

    #[test]
    fn unroot_requires_a_connection_that_can_resolve_the_active_slot() {
        assert!(!unroot_connection_ready(ConnectionStatus::Edl));
        assert!(unroot_connection_ready(ConnectionStatus::Adb));
        assert!(unroot_connection_ready(ConnectionStatus::AdbRecovery));
        assert!(unroot_connection_ready(ConnectionStatus::Fastboot));
    }

    #[test]
    fn matching_fingerprint_and_index_flashes_the_backup_untouched() {
        for target in [BackupRootTarget::Boot, BackupRootTarget::InitBoot] {
            assert_eq!(
                unroot_prep(target, Some(FP), Some(FP), 4, 4),
                UnrootPrep::FlashAsIs
            );
        }
    }

    #[test]
    fn a_different_fingerprint_aborts() {
        assert_eq!(
            unroot_prep(
                BackupRootTarget::Boot,
                Some(FP),
                Some("qti/TB320FC/TB320FC:14/UP1A/S000999:user/release-keys"),
                4,
                4
            ),
            UnrootPrep::Abort(AbortReason::FingerprintMismatch)
        );
    }

    #[test]
    fn an_unreadable_fingerprint_aborts_on_either_side() {
        assert_eq!(
            unroot_prep(BackupRootTarget::Boot, None, Some(FP), 4, 4),
            UnrootPrep::Abort(AbortReason::FingerprintUnreadable)
        );
        assert_eq!(
            unroot_prep(BackupRootTarget::Boot, Some(FP), None, 4, 4),
            UnrootPrep::Abort(AbortReason::FingerprintUnreadable)
        );
    }

    #[test]
    fn a_boot_restore_adopts_the_device_rollback_index() {
        // The bricking case: a firmware-stock backup sitting below the device's
        // bypassed floor.
        assert_eq!(
            unroot_prep(BackupRootTarget::Boot, Some(FP), Some(FP), 7, 4),
            UnrootPrep::PatchRollback(7)
        );
        // And the reverse — the restored image must claim exactly what the
        // device holds, not merely "at least" it.
        assert_eq!(
            unroot_prep(BackupRootTarget::Boot, Some(FP), Some(FP), 4, 7),
            UnrootPrep::PatchRollback(4)
        );
    }

    #[test]
    fn an_init_boot_restore_never_patches() {
        // init_boot carries no rollback index; whatever the footers say, there
        // is nothing to reconcile.
        assert_eq!(
            unroot_prep(BackupRootTarget::InitBoot, Some(FP), Some(FP), 7, 4),
            UnrootPrep::FlashAsIs
        );
    }
}
