//! Opt-in GKI image parity, without a device or network access.
//!
//! Set LTBOX_GKI_STOCK_BOOT, LTBOX_GKI_KERNEL_ZIP, LTBOX_GKI_REFERENCE_BOOT,
//! and LTBOX_GKI_ABL (the matching active-slot ABL that loads efisp).
//! The reference must be produced independently from the same stock image and
//! kernel (for example by the official magiskboot using the ZIP's AK3 options).
//! Run: cargo test -p ltbox-patch --test gki_boot_fixtures -- --ignored
//! Image equality does not establish that the device boots or that its GBL,
//! init_boot, slot, and other partitions match the reference installation.

use std::path::PathBuf;

use ltbox_patch::root_pipeline::{
    RootFamily, RootImageTarget, RootPipelineConfig, RootProvider, RootVersion,
    build_patched_artifacts,
};

fn fixture(name: &str) -> PathBuf {
    let path = PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("set {name}")));
    assert!(path.is_file(), "missing fixture: {}", path.display());
    path.canonicalize().unwrap()
}

#[test]
#[ignore = "requires independently generated stock/kernel/reference GKI fixtures"]
fn gbl_gki_pipeline_matches_reference_image() {
    let stock = fixture("LTBOX_GKI_STOCK_BOOT");
    let kernel = fixture("LTBOX_GKI_KERNEL_ZIP");
    let reference = fixture("LTBOX_GKI_REFERENCE_BOOT");
    let abl = fixture("LTBOX_GKI_ABL");
    let temp = tempfile::tempdir().unwrap();
    let work = temp.path().join("work");
    std::fs::create_dir(&work).unwrap();
    std::fs::copy(stock, work.join("boot.img")).unwrap();
    std::fs::copy(abl, work.join("abl.img")).unwrap();
    let config = RootPipelineConfig {
        family: RootFamily::KernelSU,
        provider: RootProvider::KernelSU,
        version: RootVersion::Stable,
        root_image_target: RootImageTarget::Boot,
        rebuild_vbmeta: false,
        work_dir: work,
        output_dir: temp.path().join("out"),
        loader: PathBuf::new(),
        slot_suffix: "_b".into(),
        preinit_device: String::new(),
        gki_kernel_zip: Some(kernel),
        kernel_version: None,
        kernel_gki_branch: None,
        gki_mode: true,
        kpm_paths: Vec::new(),
        superkey: String::new(),
        magisk_forks_apk: None,
        nightly_run_id: None,
        release_tag: None,
    };
    let artifacts = build_patched_artifacts(&config, true, &mut Vec::new()).unwrap();
    assert_eq!(artifacts.root_partition, "boot_b");
    assert!(artifacts.patched_vbmeta.is_none());
    assert!(artifacts.vbmeta_partition.is_none());
    let actual = std::fs::read(artifacts.patched_root_image).unwrap();
    let expected = std::fs::read(reference).unwrap();
    assert_eq!(actual.len(), expected.len(), "image lengths differ");
    assert!(
        actual == expected,
        "first differing byte: {:?}",
        actual.iter().zip(&expected).position(|(a, b)| a != b)
    );
}

#[test]
fn skip_avb_rejects_missing_empty_and_unrecognized_abl_before_patching() {
    for contents in [None, Some(Vec::new()), Some(b"unrecognized ABL".to_vec())] {
        let temp = tempfile::tempdir().unwrap();
        let work = temp.path().join("work");
        std::fs::create_dir(&work).unwrap();
        std::fs::write(work.join("boot.img"), b"stock untouched").unwrap();
        if let Some(contents) = contents {
            std::fs::write(work.join("abl.img"), contents).unwrap();
        }
        let config = RootPipelineConfig {
            family: RootFamily::KernelSU,
            provider: RootProvider::KernelSU,
            version: RootVersion::Stable,
            root_image_target: RootImageTarget::Boot,
            rebuild_vbmeta: false,
            work_dir: work.clone(),
            output_dir: temp.path().join("out"),
            loader: PathBuf::new(),
            slot_suffix: "_b".into(),
            preinit_device: String::new(),
            gki_kernel_zip: None,
            kernel_version: None,
            kernel_gki_branch: None,
            gki_mode: true,
            kpm_paths: Vec::new(),
            superkey: String::new(),
            magisk_forks_apk: None,
            nightly_run_id: None,
            release_tag: None,
        };
        let error = match build_patched_artifacts(&config, true, &mut Vec::new()) {
            Ok(_) => panic!("unverified ABL accepted"),
            Err(error) => error,
        };
        assert_eq!(
            error.to_string(),
            ltbox_core::LtboxError::Patch(ltbox_core::i18n::tr("err_abl_efisp_undetermined"))
                .to_string()
        );
        assert_eq!(
            std::fs::read(work.join("boot.img")).unwrap(),
            b"stock untouched"
        );
        assert!(!config.output_dir.join("boot.img").exists());
    }
}
