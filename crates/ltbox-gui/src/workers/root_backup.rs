//! Root-backup contents shared by the root and unroot workers.

use std::path::Path;

use crate::UnrootType;
use ltbox_patch::avb::{self, VbmetaOwner};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BackupRootTarget {
    Boot,
    InitBoot,
}

impl BackupRootTarget {
    pub(super) const fn partition_base(self) -> &'static str {
        match self {
            Self::Boot => "boot",
            Self::InitBoot => "init_boot",
        }
    }

    pub(super) const fn filename(self) -> &'static str {
        match self {
            Self::Boot => "boot.img",
            Self::InitBoot => "init_boot.img",
        }
    }
}

/// What a backup folder holds, and therefore what Unroot must restore.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct BackupContents {
    pub(super) root_target: BackupRootTarget,
    /// The one descriptor-matched parent to restore, if this route uses one.
    pub(super) vbmeta_owner: Option<VbmetaOwner>,
}

/// Resolve the images from their on-disk names.
///
/// The optional backup manifest is informational metadata. It is deliberately
/// not parsed here: an old, malformed, or mismatching manifest must not block
/// installation of otherwise valid stock images. The filename convention,
/// selected unroot route, and model's vbmeta participation determine the files.
pub(super) fn resolve_backup_contents(
    backup_dir: &Path,
    unroot_type: UnrootType,
    device_model: &str,
) -> Result<BackupContents, String> {
    // A Magisk LKM backup can contain either target. Prefer boot when
    // present, falling back to the init_boot missing-file diagnostic when
    // neither candidate exists. APatch GKI always restores boot.
    let root_target = match unroot_type {
        UnrootType::MagiskLkm => {
            if backup_dir.join(BackupRootTarget::Boot.filename()).is_file() {
                BackupRootTarget::Boot
            } else {
                BackupRootTarget::InitBoot
            }
        }
        UnrootType::APatchGki => BackupRootTarget::Boot,
    };
    let target = match root_target {
        BackupRootTarget::Boot => ltbox_patch::root_pipeline::RootImageTarget::Boot,
        BackupRootTarget::InitBoot => ltbox_patch::root_pipeline::RootImageTarget::InitBoot,
    };
    // Old fixed backup folders can retain a vbmeta from another run. Apply the same participation
    // rule as Root rather than trusting its presence or the old manifest. Chained boot and GBL-route
    // models leave it alone.
    let restore_vbmeta = ltbox_patch::root_pipeline::root_run_rebuilds_vbmeta(target, device_model)
        && !ltbox_core::model::capabilities(device_model).root_uses_gbl;
    validate_backup_image(backup_dir, root_target.filename())?;
    let vbmeta_owner = if restore_vbmeta {
        Some(resolve_backup_owner(backup_dir, root_target)?)
    } else {
        None
    };
    Ok(BackupContents {
        root_target,
        vbmeta_owner,
    })
}

fn resolve_backup_owner(dir: &Path, target: BackupRootTarget) -> Result<VbmetaOwner, String> {
    let image = dir.join(target.filename());
    let invalid = |name: &str, error: String| {
        ltbox_core::tr_args!("err_unroot_image_avb_failed", image = name, error = error)
    };
    avb::verify_root_backup(&image, None, target.partition_base())
        .map_err(|e| invalid(target.filename(), e.to_string()))?;
    let mut matched = None;
    for owner in [VbmetaOwner::Vbmeta, VbmetaOwner::VbmetaSystem] {
        let candidate = dir.join(owner.filename());
        if !candidate
            .try_exists()
            .map_err(|e| invalid(owner.filename(), e.to_string()))?
        {
            continue;
        }
        validate_backup_image(dir, owner.filename())?;
        // Parse even nonmatching candidates: unreadable and duplicate metadata
        // are not evidence that a candidate is safe to ignore.
        let count = avb::root_hash_descriptor_count(&candidate, target.partition_base())
            .map_err(|e| invalid(owner.filename(), e.to_string()))?;
        if count == 0 {
            continue;
        }
        if count != 1 {
            return Err(ltbox_core::i18n::tr("err_unroot_vbmeta_ambiguous"));
        }
        if avb::verify_root_backup(&image, Some(&candidate), target.partition_base()).is_ok() {
            if matched.is_some() {
                return Err(ltbox_core::i18n::tr("err_unroot_vbmeta_ambiguous"));
            }
            matched = Some(owner);
        }
    }
    matched.ok_or_else(|| ltbox_core::i18n::tr("err_unroot_vbmeta_missing"))
}

fn validate_backup_image(dir: &Path, name: &str) -> Result<(), String> {
    let path = dir.join(name);
    if !path.is_file() {
        return Err(if matches!(name, "vbmeta.img" | "vbmeta_system.img") {
            ltbox_core::i18n::tr("err_unroot_vbmeta_missing")
        } else {
            ltbox_core::tr_args!("err_unroot_image_missing", image = name)
        });
    }
    let file = std::fs::File::open(&path).map_err(|error| error.to_string())?;
    if file.metadata().map_err(|error| error.to_string())?.len() == 0 {
        return Err(ltbox_core::tr_args!(
            "err_unroot_image_avb_failed",
            image = name,
            error = "image is empty"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_pair(dir: &Path, target: &str, owner: VbmetaOwner) {
        let image = dir.join(format!("{target}.img"));
        std::fs::write(&image, vec![0x41; 4096]).unwrap();
        avbtool_rs::footer::add_hash_footer(
            &image,
            &avbtool_rs::footer::HashFooterArgs {
                partition_size: Some(128 * 1024),
                dynamic_partition_size: false,
                partition_name: target.into(),
                hash_algorithm: "sha256".into(),
                salt: Some(vec![1, 2]),
                chain_partitions: Vec::new(),
                algorithm_name: "NONE".into(),
                key_spec: None,
                public_key_metadata: None,
                rollback_index: 0,
                flags: 0,
                rollback_index_location: 0,
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
        std::fs::write(
            dir.join(owner.filename()),
            avbtool_rs::image::load_vbmeta_blob(&image).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn descriptor_matching_selects_legacy_and_a17_owners() {
        for owner in [VbmetaOwner::Vbmeta, VbmetaOwner::VbmetaSystem] {
            let temp = tempfile::tempdir().unwrap();
            write_pair(temp.path(), "init_boot", owner);
            let contents =
                resolve_backup_contents(temp.path(), UnrootType::MagiskLkm, "TB520FU").unwrap();
            assert_eq!(contents.vbmeta_owner, Some(owner));
        }
    }

    #[test]
    fn ambiguous_missing_and_unreadable_owners_abort() {
        let temp = tempfile::tempdir().unwrap();
        write_pair(temp.path(), "init_boot", VbmetaOwner::VbmetaSystem);
        std::fs::copy(
            temp.path().join("vbmeta_system.img"),
            temp.path().join("vbmeta.img"),
        )
        .unwrap();
        assert_eq!(
            resolve_backup_contents(temp.path(), UnrootType::MagiskLkm, "TB520FU").unwrap_err(),
            ltbox_core::i18n::tr("err_unroot_vbmeta_ambiguous")
        );
        std::fs::remove_file(temp.path().join("vbmeta.img")).unwrap();
        std::fs::remove_file(temp.path().join("vbmeta_system.img")).unwrap();
        assert_eq!(
            resolve_backup_contents(temp.path(), UnrootType::MagiskLkm, "TB520FU").unwrap_err(),
            ltbox_core::i18n::tr("err_unroot_vbmeta_missing")
        );
        write_pair(temp.path(), "init_boot", VbmetaOwner::VbmetaSystem);
        std::fs::write(temp.path().join("vbmeta.img"), b"unreadable").unwrap();
        assert!(resolve_backup_contents(temp.path(), UnrootType::MagiskLkm, "TB520FU").is_err());
    }

    #[test]
    fn mismatched_owner_hash_is_missing_not_a_restore_candidate() {
        let temp = tempfile::tempdir().unwrap();
        write_pair(temp.path(), "init_boot", VbmetaOwner::VbmetaSystem);
        let image = temp.path().join("init_boot.img");
        let info = avb::extract_image_avb_info(&image).unwrap();
        std::fs::write(&image, vec![0x42; 4096]).unwrap();
        avb::add_hash_footer(&image, &info, None, None).unwrap();
        assert_eq!(
            resolve_backup_contents(temp.path(), UnrootType::MagiskLkm, "TB520FU").unwrap_err(),
            ltbox_core::i18n::tr("err_unroot_vbmeta_missing")
        );
    }

    #[test]
    fn malformed_manifest_does_not_block_boot_selection() {
        let temp = tempfile::tempdir().unwrap();
        write_pair(temp.path(), "boot", VbmetaOwner::Vbmeta);
        std::fs::write(temp.path().join("root-backup.json"), b"not json").unwrap();
        std::fs::write(temp.path().join("manifest.json"), b"not json").unwrap();

        let contents =
            resolve_backup_contents(temp.path(), UnrootType::MagiskLkm, "TB320FC").unwrap();
        assert_eq!(contents.root_target, BackupRootTarget::Boot);
        assert!(contents.vbmeta_owner.is_some());
    }

    #[test]
    fn mismatching_manifest_does_not_change_init_boot_selection() {
        let temp = tempfile::tempdir().unwrap();
        write_pair(temp.path(), "init_boot", VbmetaOwner::Vbmeta);
        std::fs::write(
            temp.path().join("root-backup.json"),
            br#"{"root_partition":"boot","vbmeta":true}"#,
        )
        .unwrap();
        std::fs::write(
            temp.path().join("manifest.json"),
            br#"{"model":"other device","fingerprint":"other build","files":[{"filename":"boot.img","sha256":"wrong","rollback_index":999}]}"#,
        )
        .unwrap();

        let contents =
            resolve_backup_contents(temp.path(), UnrootType::MagiskLkm, "TB320FC").unwrap();
        assert_eq!(contents.root_target, BackupRootTarget::InitBoot);
        assert!(contents.vbmeta_owner.is_some());
    }

    #[test]
    fn manifest_free_magisk_backup_infers_init_boot_filename() {
        let temp = tempfile::tempdir().unwrap();
        write_pair(temp.path(), "init_boot", VbmetaOwner::Vbmeta);

        assert_eq!(
            resolve_backup_contents(temp.path(), UnrootType::MagiskLkm, "TB320FC")
                .unwrap()
                .root_target,
            BackupRootTarget::InitBoot
        );
    }

    #[test]
    fn manifest_free_magisk_backup_infers_boot_filename() {
        let temp = tempfile::tempdir().unwrap();
        write_pair(temp.path(), "boot", VbmetaOwner::Vbmeta);

        assert_eq!(
            resolve_backup_contents(temp.path(), UnrootType::MagiskLkm, "TB320FC")
                .unwrap()
                .root_target,
            BackupRootTarget::Boot
        );
    }

    #[test]
    fn required_vbmeta_is_restored_without_manifest() {
        let temp = tempfile::tempdir().unwrap();
        write_pair(temp.path(), "boot", VbmetaOwner::Vbmeta);

        let contents =
            resolve_backup_contents(temp.path(), UnrootType::MagiskLkm, "TB320FC").unwrap();
        assert_eq!(contents.root_target, BackupRootTarget::Boot);
        assert!(contents.vbmeta_owner.is_some());
    }

    #[test]
    fn stale_legacy_vbmeta_is_ignored_when_root_leaves_it_untouched() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("boot.img"), b"current root image").unwrap();
        std::fs::write(temp.path().join("vbmeta.img"), b"stale from another run").unwrap();
        for model in ["TB322FC", "TB323FU", "TB321FU"] {
            let contents =
                resolve_backup_contents(temp.path(), UnrootType::APatchGki, model).unwrap();
            assert_eq!(contents.root_target, BackupRootTarget::Boot);
            assert!(contents.vbmeta_owner.is_none(), "{model}");
        }
        write_pair(temp.path(), "boot", VbmetaOwner::Vbmeta);
        for model in ["TB320FC", "LAVIETab9QHD1"] {
            assert!(
                resolve_backup_contents(temp.path(), UnrootType::APatchGki, model)
                    .unwrap()
                    .vbmeta_owner
                    .is_some()
            );
        }
    }

    #[test]
    fn init_boot_restores_vbmeta_except_on_efisp_root_route() {
        let temp = tempfile::tempdir().unwrap();
        write_pair(temp.path(), "init_boot", VbmetaOwner::Vbmeta);
        assert!(
            resolve_backup_contents(temp.path(), UnrootType::MagiskLkm, "TB322FC")
                .unwrap()
                .vbmeta_owner
                .is_some()
        );
        assert!(
            resolve_backup_contents(temp.path(), UnrootType::MagiskLkm, "TB323FU")
                .unwrap()
                .vbmeta_owner
                .is_none()
        );
    }
    #[test]
    fn required_images_are_validated_without_downgrading_the_restore_plan() {
        for target in ["boot.img", "init_boot.img"] {
            let temp = tempfile::tempdir().unwrap();
            std::fs::write(temp.path().join(target), b"root image").unwrap();
            assert!(
                resolve_backup_contents(temp.path(), UnrootType::MagiskLkm, "TB320FC").is_err()
            );
            std::fs::write(temp.path().join("vbmeta.img"), []).unwrap();
            assert!(
                resolve_backup_contents(temp.path(), UnrootType::MagiskLkm, "TB320FC").is_err()
            );
            write_pair(
                temp.path(),
                target.trim_end_matches(".img"),
                VbmetaOwner::Vbmeta,
            );
            assert!(
                resolve_backup_contents(temp.path(), UnrootType::MagiskLkm, "TB320FC")
                    .unwrap()
                    .vbmeta_owner
                    .is_some()
            );
            std::fs::write(temp.path().join(target), []).unwrap();
            assert!(
                resolve_backup_contents(temp.path(), UnrootType::MagiskLkm, "TB320FC").is_err()
            );
        }
    }

    #[test]
    fn nonparticipating_model_accepts_root_image_without_vbmeta() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("boot.img"), b"boot").unwrap();
        assert!(
            resolve_backup_contents(temp.path(), UnrootType::APatchGki, "TB322FC")
                .unwrap()
                .vbmeta_owner
                .is_none()
        );
    }
}
