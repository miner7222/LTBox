//! Canoe efisp ARB-overlay provisioning: pick the efisp asset variant
//! and build the per-LUN overlay set. Extracted from main.rs.

use crate::*;

/// GBL EFI asset suffix for a TB323FU target firmware, by region (`is_prc`) and
/// whether the anti-rollback build is needed (`arb`). Picks the
/// `*_prc.efi` / `*_row.efi` asset (or `*_prc_arb.efi` / `*_row_arb.efi`) from
/// the gbl_root_baldur release. The `_arb` GBL roots trust at the testkey so it
/// accepts the testkey-re-signed AVB chain LTBox stages on a downgrade. The
/// region comes from the vendor_boot `product_region` DTB marker — TB323FU's AVB
/// fingerprint carries no `_PRC`/`_ROW` token.
pub(crate) fn efisp_asset_suffix(is_prc: bool, arb: bool) -> &'static str {
    match (is_prc, arb) {
        (true, false) => "_prc.efi",
        (true, true) => "_prc_arb.efi",
        (false, false) => "_row.efi",
        (false, true) => "_row_arb.efi",
    }
}

/// A dumped `efisp` partition counts as empty (un-provisioned) when every byte
/// is zero and the dump is nonempty — the stock/erased state.
/// A GBL-provisioned efisp carries the EFI
/// payload, so it has non-zero bytes. The TB323FU root flow uses an empty result
/// to provision the appropriate region GBL before continuing.
pub(crate) fn efisp_is_empty(data: &[u8]) -> bool {
    !data.is_empty() && data.iter().all(|&b| b == 0)
}

/// A missing, unreadable or zero-length dump cannot establish provisioning state.
fn read_efisp_is_empty(path: &std::path::Path) -> Result<bool, String> {
    let data = std::fs::read(path).map_err(|error| {
        tr_args!(
            "err_efisp_read_failed",
            path = path.display(),
            error = error
        )
    })?;
    if data.is_empty() {
        return Err(ltbox_core::i18n::tr("err_efisp_dump_empty"));
    }
    Ok(efisp_is_empty(&data))
}

/// Select the vendor_boot region's GBL, defaulting to PRC when no marker exists
/// (as on TB324ZC). Missing, unreadable and empty images still fail.
pub(crate) fn efisp_suffix_for_vendor_boot(
    path: &std::path::Path,
    arb: bool,
) -> Result<&'static str, String> {
    use ltbox_patch::region::RegionTarget;
    match ltbox_patch::region::detect_product_region(path) {
        Some(RegionTarget::Prc) => Ok(efisp_asset_suffix(true, arb)),
        Some(RegionTarget::Row) => Ok(efisp_asset_suffix(false, arb)),
        None => {
            if std::fs::read(path).is_ok_and(|data| !data.is_empty()) {
                Ok(efisp_asset_suffix(true, arb))
            } else {
                Err(tr_args!("err_efisp_region_unknown", path = path.display()))
            }
        }
    }
}

/// Require positive evidence that this ABL loads efisp.
pub(crate) fn verify_abl_efisp(path: &std::path::Path) -> Result<(), String> {
    use ltbox_patch::efisp_load::{EfispLoad, detect};
    let data =
        std::fs::read(path).map_err(|_| ltbox_core::i18n::tr("err_abl_efisp_undetermined"))?;
    match detect(&data) {
        EfispLoad::Yes => Ok(()),
        EfispLoad::No => Err(ltbox_core::i18n::tr("err_abl_efisp_not_loaded")),
        EfispLoad::Undetermined => Err(ltbox_core::i18n::tr("err_abl_efisp_undetermined")),
    }
}

pub(crate) fn dump_verified_active_abl(
    session: &mut ltbox_device::edl::EdlSession,
    slot_suffix: &str,
    destination: &std::path::Path,
    log: &mut Vec<String>,
) -> Result<(), String> {
    let partition = format!("abl{}", active_slot_suffix(Some(slot_suffix)));
    let lun = ltbox_core::partition_lun::lun_for_partition(&partition)
        .ok_or_else(|| tr_args!("err_no_hardcoded_lun", partition = partition))?;
    session
        .dump_partition(&partition, destination, 0, lun, log)
        .map_err(|error| {
            tr_args!(
                "err_root_dump_partition_failed",
                partition = partition,
                error = error
            )
        })?;
    verify_abl_efisp(destination)
}

/// Inspect a Canoe target's `efisp` and, when it is still all-zero, stage the
/// matching region GBL used by the Root and KonaBess device workers. The
/// caller performs the actual efisp write with [`provision_canoe_efisp`] at
/// the safest point in its own operation.
pub(crate) fn prepare_canoe_efisp(
    session: &mut ltbox_device::edl::EdlSession,
    slot_suffix: &str,
    dumped_vendor_boot: Option<&std::path::Path>,
    work_dir: &std::path::Path,
    efi_dir: &std::path::Path,
    log: &mut Vec<String>,
) -> std::result::Result<Option<std::path::PathBuf>, String> {
    prepare_canoe_efisp_with(
        slot_suffix,
        dumped_vendor_boot,
        work_dir,
        efi_dir,
        log,
        &mut |partition, path, lun, log| {
            session
                .dump_partition(partition, path, 0, lun, log)
                .map_err(|error| {
                    tr_args!(
                        "err_root_dump_partition_failed",
                        partition = partition,
                        error = error
                    )
                })
        },
        &mut |suffix, dir, log| fetch_efisp_asset(suffix, dir, "[Root]", log),
    )
}

fn prepare_canoe_efisp_with(
    slot_suffix: &str,
    dumped_vendor_boot: Option<&std::path::Path>,
    work_dir: &std::path::Path,
    efi_dir: &std::path::Path,
    log: &mut Vec<String>,
    dump: &mut impl FnMut(&str, &std::path::Path, u8, &mut Vec<String>) -> Result<(), String>,
    fetch: &mut impl FnMut(
        &str,
        &std::path::Path,
        &mut Vec<String>,
    ) -> Result<std::path::PathBuf, String>,
) -> Result<Option<std::path::PathBuf>, String> {
    live!(
        log,
        "[Root] {}",
        ltbox_core::i18n::tr("log_root_efisp_check")
    );
    let abl_partition = format!("abl{}", active_slot_suffix(Some(slot_suffix)));
    let abl_lun = ltbox_core::partition_lun::lun_for_partition(&abl_partition)
        .ok_or_else(|| tr_args!("err_no_hardcoded_lun", partition = abl_partition))?;
    let abl_path = work_dir.join("abl.img");
    dump(&abl_partition, &abl_path, abl_lun, log)?;
    verify_abl_efisp(&abl_path)?;
    let dumped_efisp = work_dir.join("efisp.img");
    let efisp_lun = ltbox_core::partition_lun::lun_for_partition("efisp").unwrap_or(4);
    dump("efisp", &dumped_efisp, efisp_lun, log)?;
    let efisp_empty = read_efisp_is_empty(&dumped_efisp)?;
    if !efisp_empty {
        live!(log, "[Root] {}", ltbox_core::i18n::tr("log_root_efisp_ok"));
        return Ok(None);
    }

    // Empty efisp is the stock, GBL-unprovisioned state. Region comes from
    // vendor_boot's product_region marker because the AVB fingerprint carries
    // no PRC/ROW token on TB323FU.
    let suffix = if let Some(path) = dumped_vendor_boot {
        efisp_suffix_for_vendor_boot(path, false)?
    } else {
        let partition = format!("vendor_boot{slot_suffix}");
        let path = work_dir.join("vendor_boot.img");
        let lun = ltbox_core::partition_lun::lun_for_partition(&partition)
            .ok_or_else(|| tr_args!("err_no_hardcoded_lun", partition = partition))?;
        dump(&partition, &path, lun, log)?;
        efisp_suffix_for_vendor_boot(&path, false)?
    };
    fetch(suffix, efi_dir, log).map(Some)
}

/// Download the pinned efisp GBL for `suffix` into `efi_dir` and verify it.
///
/// Every caller writes the result to the device's `efisp` partition, so the
/// release tag is pinned, the asset name must be one of a fixed set, and the
/// bytes are SHA-256 checked before the path is handed back: an unpinned
/// release or an unverified download could stage an unexpected image onto a
/// rollback-protected partition.
pub(crate) fn fetch_efisp_asset(
    suffix: &str,
    efi_dir: &std::path::Path,
    log_tag: &str,
    log: &mut Vec<String>,
) -> std::result::Result<std::path::PathBuf, String> {
    let expected_name = efisp_expected_asset(suffix).ok_or_else(|| {
        tr_args!(
            "err_efisp_asset_unknown",
            asset = suffix,
            tag = EFISP_GBL_RELEASE_TAG
        )
    })?;
    live!(
        log,
        "{} {}",
        log_tag,
        tr_args!("live_flash_efisp_fetch", variant = suffix)
    );
    let gh = ltbox_core::github::GitHubClient::from_url("github.com/miner7222/gbl_root_baldur")
        .map_err(|e| tr_args!("err_efisp_github_failed", error = e))?;
    let assets = gh
        .release_by_tag(EFISP_GBL_RELEASE_TAG)
        .map_err(|e| tr_args!("err_efisp_github_failed", error = e))?;
    let (asset_name, asset_url) = assets
        .into_iter()
        .find(|(name, _)| name == expected_name)
        .ok_or_else(|| {
            tr_args!(
                "err_efisp_asset_missing",
                suffix = suffix,
                tag = EFISP_GBL_RELEASE_TAG
            )
        })?;
    let _ = std::fs::remove_dir_all(efi_dir);
    std::fs::create_dir_all(efi_dir)
        .map_err(|e| tr_args!("err_efisp_work_dir_failed", error = e))?;
    let efi_path = efi_dir.join(&asset_name);
    ltbox_core::downloader::download_to_file(&asset_url, &efi_path, log)
        .map_err(|e| tr_args!("err_efisp_download_failed", asset = asset_name, error = e))?;
    verify_efisp_asset(&efi_path, &asset_name)?;
    live!(
        log,
        "{} {}",
        log_tag,
        tr_args!("live_flash_efisp_fetched", name = asset_name)
    );
    Ok(efi_path)
}

/// Pinned gbl_root_baldur release used for TB323FU efisp GBL images.
pub(crate) const EFISP_GBL_RELEASE_TAG: &str = "6.2.192-mod2";

/// Exact asset names accepted for the pinned efisp release.
pub(crate) const EFISP_EXPECTED_ASSETS: &[(&str, &str)] = &[
    (
        "generic_superfastboot_prc.efi",
        "b91db7974da80976bc73add671f4adfb833aea4fd0d9d7604b75cb63415fed84",
    ),
    (
        "generic_superfastboot_prc_arb.efi",
        "00fb9c97b5ab2c7754dc13d0761e7ac753b94ceaf9e2068122c16ade2d80f7c4",
    ),
    (
        "generic_superfastboot_row.efi",
        "e21f6d5ffe29113943092ac4a2a75e4fa03960831cc8855316c784b472999abd",
    ),
    (
        "generic_superfastboot_row_arb.efi",
        "e67b3778189080f1d14c7b511b7fed3f253e7b13f9b661898b6c9b0293e2383e",
    ),
];

/// Map a region/ARB suffix to the exact pinned asset name for the
/// `6.2.192-mod2` gbl_root_baldur release. Unknown suffixes refuse.
pub(crate) fn efisp_expected_asset(suffix: &str) -> Option<&'static str> {
    match suffix {
        "_prc.efi" => Some("generic_superfastboot_prc.efi"),
        "_prc_arb.efi" => Some("generic_superfastboot_prc_arb.efi"),
        "_row.efi" => Some("generic_superfastboot_row.efi"),
        "_row_arb.efi" => Some("generic_superfastboot_row_arb.efi"),
        _ => None,
    }
}

/// SHA-256 hex for a pinned efisp asset name. Unknown names refuse.
pub(crate) fn efisp_expected_sha256(asset_name: &str) -> Option<&'static str> {
    EFISP_EXPECTED_ASSETS
        .iter()
        .find(|(name, _)| *name == asset_name)
        .map(|(_, hash)| *hash)
}

/// Verify a downloaded efisp EFI against the pinned name → SHA-256 map.
/// Unknown names and hash mismatches both refuse.
pub(crate) fn verify_efisp_asset(path: &std::path::Path, asset_name: &str) -> Result<(), String> {
    let expected = efisp_expected_sha256(asset_name).ok_or_else(|| {
        tr_args!(
            "err_efisp_asset_unknown",
            asset = asset_name,
            tag = EFISP_GBL_RELEASE_TAG
        )
    })?;
    let actual = ltbox_core::crypto::sha256_file_hex(path).map_err(|error| error.to_string())?;
    if actual != expected {
        return Err(tr_args!(
            "err_efisp_hash_mismatch",
            asset = asset_name,
            expected = expected,
            actual = actual
        ));
    }
    Ok(())
}

/// Provision a staged region GBL for a Canoe target. `None` is the
/// already-provisioned path and deliberately performs no device write.
pub(crate) fn provision_canoe_efisp(
    session: &mut ltbox_device::edl::EdlSession,
    efi: Option<&std::path::Path>,
    log: &mut Vec<String>,
) -> std::result::Result<bool, String> {
    let Some(efi) = efi else {
        return Ok(false);
    };
    let efisp_lun = ltbox_core::partition_lun::lun_for_partition("efisp").unwrap_or(4);
    live!(
        log,
        "[Root] {}",
        ltbox_core::i18n::tr("live_flash_efisp_flash")
    );
    session
        .flash_partition("efisp", efi, 0, efisp_lun, log)
        .map_err(|e| tr_args!("err_root_efisp_provision_failed", error = e))?;
    ltbox_core::live_debug!(
        log,
        "[Root] {}",
        ltbox_core::i18n::tr("live_flash_efisp_flashed")
    );
    Ok(true)
}

/// One staged ARB overlay: (GPT label, UFS LUN, patched image path).
pub(crate) type ArbOverlay = (String, u8, std::path::PathBuf);

/// Select the rollback indices written into the re-signed `boot` and
/// `vbmeta_system` images, and report whether the selection requires the
/// TB323FU ARB GBL. This is deliberately independent of USB and filesystem
/// state so manual-target policy can be tested without a device session.
fn select_arb_rollback_targets(
    firmware: ltbox_patch::rollback::RollbackIndices,
    device_floors: ltbox_patch::rollback::RollbackIndices,
    manual_targets: Option<ltbox_patch::rollback::RollbackIndices>,
) -> std::result::Result<(ltbox_patch::rollback::RollbackIndices, bool), String> {
    if let Some(requested) = manual_targets {
        let plan = ltbox_patch::rollback::plan_manual_rollback(firmware, device_floors, requested)
            .map_err(|error| {
                tr_args!(
                    "err_flash_manual_rollback_below_floor",
                    partition = error.partition,
                    requested = error.requested.to_string(),
                    floor = error.floor.to_string()
                )
            })?;
        return Ok((plan.targets, plan.changes_indices()));
    }

    let targets = ltbox_patch::rollback::RollbackIndices {
        boot: firmware.boot.max(device_floors.boot),
        vbmeta_system: firmware.vbmeta_system.max(device_floors.vbmeta_system),
    };
    let need =
        firmware.boot < device_floors.boot || firmware.vbmeta_system < device_floors.vbmeta_system;
    Ok((targets, need))
}

/// Build the overlay files from already-read firmware/device state. This
/// helper performs no device I/O, so fixture tests can verify AVB signatures,
/// chain keys, and exact rollback indices without an `EdlSession`.
fn build_testkey_arb_overlays_from_images(
    fw_dir: &std::path::Path,
    work_dir: &std::path::Path,
    inst_vbmeta: &std::path::Path,
    chain_descriptors: &[ltbox_patch::avb::ChainPartitionDescriptor],
    rollback_targets: ltbox_patch::rollback::RollbackIndices,
    log: &mut Vec<String>,
) -> std::result::Result<Vec<ArbOverlay>, String> {
    const KEY: &str = "testkey_rsa4096";
    const ALGO: &str = "SHA256_RSA4096";

    let lun_of = |label: &str| -> std::result::Result<u8, String> {
        ltbox_core::partition_lun::lun_for_partition(label)
            .ok_or_else(|| format!("no hardcoded LUN for {label}"))
    };
    let inst_img = |p: &str| fw_dir.join(format!("{p}.img"));
    // GPT label for a chained partition: `_a` for A/B, the unsuffixed name for
    // a `DO_NOT_USE_AB` chain (AVB verifies the unsuffixed partition).
    let label_of = |c: &ltbox_patch::avb::ChainPartitionDescriptor| -> String {
        if c.do_not_use_ab {
            c.name.clone()
        } else {
            format!("{}_a", c.name)
        }
    };

    // Validate + select the chained partitions to re-sign. A chained image
    // missing from the package is unsafe because the rebuilt root would
    // delegate to an image that the flash cannot replace. Partitions with no
    // static LUN (for example vbmeta_vendor) retain their stock descriptor and
    // image, as in the original layout-aware policy.
    let mut to_resign: Vec<&ltbox_patch::avb::ChainPartitionDescriptor> = Vec::new();
    for c in chain_descriptors {
        if c.name.is_empty()
            || !c
                .name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Err(format!(
                "unsafe chain partition name in vbmeta: {:?}",
                c.name
            ));
        }
        if !inst_img(&c.name).exists() {
            return Err(format!(
                "vbmeta chains {} but its install image is missing: {}",
                c.name,
                inst_img(&c.name).display()
            ));
        }
        if ltbox_core::partition_lun::lun_for_partition(&label_of(c)).is_some() {
            to_resign.push(c);
        } else if c.name == "boot" || c.name == "vbmeta_system" {
            return Err(format!(
                "rollback-protected {} is chained but its LUN is unknown",
                c.name
            ));
        }
    }

    // Re-sign each handled chained partition to the testkey. The rollback
    // protected images receive the selected target exactly; other chained
    // partitions are re-signed without changing their own rollback index.
    let key_algo = ltbox_patch::avb::algorithm_for_key_spec(KEY)
        .ok_or_else(|| format!("unknown AVB algorithm for {KEY}"))?;
    let mut ordered = to_resign.clone();
    ordered.sort_by_key(|c| u8::from(c.name == "boot"));
    let mut overlays: Vec<ArbOverlay> = Vec::new();
    for &c in &ordered {
        let name = c.name.as_str();
        let out = work_dir.join(format!("{name}.arb.img"));
        std::fs::copy(inst_img(name), &out).map_err(|e| format!("copy {name}: {e}"))?;
        let target = match name {
            "boot" => Some(rollback_targets.boot),
            "vbmeta_system" => Some(rollback_targets.vbmeta_system),
            _ => None,
        };
        ltbox_patch::avb::resign_image(&out, KEY, &key_algo, target)
            .map_err(|e| format!("resign {name}: {e}"))?;
        let label = label_of(c);
        let lun = lun_of(&label)?;
        overlays.push((label, lun, out));
    }

    // Rebuild vbmeta on the base, updating chain partition descriptor public
    // keys for the re-signed chained partitions to the testkey; flash vbmeta
    // last because it ties the chain together.
    let out_vbmeta = work_dir.join("vbmeta.arb.img");
    let chain_partition_names: Vec<&str> = to_resign.iter().map(|c| c.name.as_str()).collect();
    ltbox_patch::avb::rebuild_vbmeta_with_chain_key_overrides(
        &out_vbmeta,
        inst_vbmeta,
        &chain_partition_names,
        KEY,
        KEY,
        ALGO,
    )
    .map_err(|e| format!("rebuild vbmeta: {e}"))?;
    overlays.push(("vbmeta_a".to_string(), lun_of("vbmeta_a")?, out_vbmeta));
    ltbox_core::live!(
        log,
        "[ARB] {}",
        tr_args!(
            "live_arb_tb323_resigned",
            boot = rollback_targets.boot.to_string(),
            vbs = rollback_targets.vbmeta_system.to_string()
        )
    );

    Ok(overlays)
}

/// Testkey re-sign overlays for an AVB flash — used by the TB323FU anti-rollback
/// path and the non-TB323FU Lenovo-key / cross-region re-sign. The device-committed
/// boot + vbmeta_system indices come from `device_floors` (component-wise across
/// both slots on EDL-start) or are read from the active slot here.
///
/// Layout-aware: it re-signs exactly the partitions the (base) vbmeta chains —
/// each needs a matching `<part>.img` — so packages without recovery (or with a
/// different set of chained partitions) work too. In automatic mode,
/// boot / vbmeta_system bump to the device floor (`max`, never lowered); manual
/// mode writes the validated requested targets exactly. Other chained partitions
/// (e.g. recovery) are re-signed only; the vbmeta is rebuilt with selected chain
/// partition descriptor public keys updated to the testkey and flashed LAST (it
/// ties the chain together — shrinks the partial-write brick window). For a Lenovo-key cross-region
/// install, `vbmeta_base` overrides the rebuild base with the region-converted,
/// testkey vbmeta so its recomputed vendor_boot hash is preserved; otherwise it
/// uses the firmware's own `vbmeta.img`.
///
/// `force_resign` re-signs even without a downgrade (Lenovo-key firmware on a testkey
/// device). Returns `(overlays, need)`; `need` is true when an automatic downgrade
/// or a manual index change requires the `_arb` (testkey-root) GBL.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_testkey_arb_overlays(
    session: &mut ltbox_device::edl::EdlSession,
    fw_dir: &std::path::Path,
    work_dir: &std::path::Path,
    slot: Option<&str>,
    device_floors: Option<(u64, u64)>,
    manual_targets: Option<ltbox_patch::rollback::RollbackIndices>,
    force_resign: bool,
    vbmeta_base: Option<&std::path::Path>,
    log: &mut Vec<String>,
) -> std::result::Result<(Vec<ArbOverlay>, bool), String> {
    let lun_of = |label: &str| -> std::result::Result<u8, String> {
        ltbox_core::partition_lun::lun_for_partition(label)
            .ok_or_else(|| format!("no hardcoded LUN for {label}"))
    };
    let idx_of = |path: &std::path::Path| -> std::result::Result<u64, String> {
        Ok(ltbox_patch::avb::extract_image_avb_info(path)
            .map_err(|e| format!("AVB inspect {}: {e}", path.display()))?
            .rollback_index)
    };

    // Device-committed per-location indices (boot + vbmeta_system), passed to
    // `build_testkey_arb_overlays_for_floors` as fixed floors. On an EDL-start
    // flash the caller passes component-wise maxima already read across BOTH
    // slots; otherwise read the ACTIVE slot here (a first-time user may still
    // be on `_b`, so don't assume `_a`).
    let (dev_boot_idx, dev_vbs_idx) = match device_floors {
        Some(floors) => floors,
        None => {
            let dev_boot = format!("boot{}", active_slot_suffix(slot));
            let dev_vbs = format!("vbmeta_system{}", active_slot_suffix(slot));
            let dev_boot_img = work_dir.join(format!("dev_{dev_boot}.img"));
            let dev_vbs_img = work_dir.join(format!("dev_{dev_vbs}.img"));
            session
                .dump_partition(&dev_boot, &dev_boot_img, 0, lun_of(&dev_boot)?, log)
                .map_err(|e| format!("dump device {dev_boot}: {e}"))?;
            session
                .dump_partition(&dev_vbs, &dev_vbs_img, 0, lun_of(&dev_vbs)?, log)
                .map_err(|e| format!("dump device {dev_vbs}: {e}"))?;
            let b = idx_of(&dev_boot_img)?;
            let v = idx_of(&dev_vbs_img)?;
            let _ = std::fs::remove_file(&dev_boot_img);
            let _ = std::fs::remove_file(&dev_vbs_img);
            (b, v)
        }
    };

    build_testkey_arb_overlays_for_floors(
        fw_dir,
        work_dir,
        (dev_boot_idx, dev_vbs_idx),
        manual_targets,
        force_resign,
        vbmeta_base,
        log,
    )
}

/// Pure image-staging entry point shared by the transport wrapper and fixtures.
pub(crate) fn build_testkey_arb_overlays_for_floors(
    fw_dir: &std::path::Path,
    work_dir: &std::path::Path,
    device_floors: (u64, u64),
    manual_targets: Option<ltbox_patch::rollback::RollbackIndices>,
    force_resign: bool,
    vbmeta_base: Option<&std::path::Path>,
    log: &mut Vec<String>,
) -> std::result::Result<(Vec<ArbOverlay>, bool), String> {
    let idx_of = |path: &std::path::Path| -> std::result::Result<u64, String> {
        Ok(ltbox_patch::avb::extract_image_avb_info(path)
            .map_err(|e| format!("AVB inspect {}: {e}", path.display()))?
            .rollback_index)
    };
    let (dev_boot_idx, dev_vbs_idx) = device_floors;
    // 1. Inspect base vbmeta (caller override for cross-region, else firmware's)
    //    and the partitions it chains. Re-sign only the ones that qualify —
    //    a plain partition name, an install image, and a resolvable A/B GPT
    //    label/LUN — updating their chain partition descriptor public keys.
    //    Other chained partitions (e.g. vbmeta_vendor) keep their stock chain
    //    partition descriptor + stock image.
    let inst_vbmeta = vbmeta_base
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| fw_dir.join("vbmeta.img"));
    if !inst_vbmeta.exists() {
        return Err(format!("install image missing: {}", inst_vbmeta.display()));
    }
    let chain_descriptors = ltbox_patch::avb::chain_partition_descriptors(&inst_vbmeta)
        .map_err(|e| format!("vbmeta chain partition descriptors: {e}"))?;
    let inst_img = |p: &str| fw_dir.join(format!("{p}.img"));
    let has = |name: &str| chain_descriptors.iter().any(|c| c.name == name);
    if manual_targets.is_some() && (!has("boot") || !has("vbmeta_system")) {
        return Err(
            "manual rollback targets require vbmeta chains for both boot and vbmeta_system"
                .to_string(),
        );
    }
    // The floor read + index bump below assume A/B slots for the rollback-
    // protected partitions, so reject a non-A/B boot / vbmeta_system layout.
    if chain_descriptors
        .iter()
        .any(|c| (c.name == "boot" || c.name == "vbmeta_system") && c.do_not_use_ab)
    {
        return Err("non-A/B boot/vbmeta_system rollback layout is unsupported".to_string());
    }
    // 2. Rollback-protected install indices (boot + vbmeta_system when chained).
    let firmware_boot_idx = if has("boot") {
        idx_of(&inst_img("boot"))?
    } else {
        0
    };
    let firmware_vbs_idx = if has("vbmeta_system") {
        idx_of(&inst_img("vbmeta_system"))?
    } else {
        0
    };
    ltbox_core::live!(
        log,
        "[ARB] {}",
        tr_args!(
            "live_arb_tb323_indices",
            boot_i = firmware_boot_idx.to_string(),
            boot_d = dev_boot_idx.to_string(),
            vbs_i = firmware_vbs_idx.to_string(),
            vbs_d = dev_vbs_idx.to_string()
        )
    );

    // 3. Automatic mode preserves the max/floor behavior. Manual mode
    // validates each requested target against its corresponding device floor
    // and writes the exact requested value, including values below firmware.
    let (rollback_targets, need) = select_arb_rollback_targets(
        ltbox_patch::rollback::RollbackIndices {
            boot: firmware_boot_idx,
            vbmeta_system: firmware_vbs_idx,
        },
        ltbox_patch::rollback::RollbackIndices {
            // An absent chain must not independently trigger an Auto overlay.
            boot: if has("boot") { dev_boot_idx } else { 0 },
            vbmeta_system: if has("vbmeta_system") { dev_vbs_idx } else { 0 },
        },
        manual_targets,
    )?;
    if !need && !force_resign {
        ltbox_core::live!(
            log,
            "[ARB] {}",
            ltbox_core::i18n::tr("live_arb_tb323_skip_uptodate")
        );
        return Ok((Vec::new(), false));
    }

    let overlays = build_testkey_arb_overlays_from_images(
        fw_dir,
        work_dir,
        &inst_vbmeta,
        &chain_descriptors,
        rollback_targets,
        log,
    )?;
    Ok((overlays, need))
}

#[cfg(test)]
mod provisioning_tests {
    use super::{efisp_suffix_for_vendor_boot, read_efisp_is_empty, select_arb_rollback_targets};

    #[test]
    fn efisp_dump_requires_readable_nonempty_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("efisp.img");
        assert!(read_efisp_is_empty(&path).is_err());
        assert!(read_efisp_is_empty(dir.path()).is_err());
        std::fs::write(&path, []).unwrap();
        assert!(read_efisp_is_empty(&path).is_err());
        std::fs::write(&path, [0; 4096]).unwrap();
        assert!(read_efisp_is_empty(&path).unwrap());
        std::fs::write(&path, [0, 0, 1, 0]).unwrap();
        assert!(!read_efisp_is_empty(&path).unwrap());
    }

    #[test]
    fn gbl_selection_defaults_to_prc_without_a_region_marker() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vendor_boot.img");
        for arb in [false, true] {
            assert!(efisp_suffix_for_vendor_boot(&path, arb).is_err());
            assert!(efisp_suffix_for_vendor_boot(dir.path(), arb).is_err());
        }
        std::fs::write(&path, b"").unwrap();
        for arb in [false, true] {
            assert!(efisp_suffix_for_vendor_boot(&path, arb).is_err());
        }
        for data in [b"no region marker".as_slice(), b"product_region\0model\0"] {
            std::fs::write(&path, data).unwrap();
            assert_eq!(
                efisp_suffix_for_vendor_boot(&path, false).unwrap(),
                "_prc.efi"
            );
            assert_eq!(
                efisp_suffix_for_vendor_boot(&path, true).unwrap(),
                "_prc_arb.efi"
            );
        }
        // Minimal product_region FDT property layout accepted by the detector.
        for (region, stock, arb) in [
            (b"PRC\0", "_prc.efi", "_prc_arb.efi"),
            (b"ROW\0", "_row.efi", "_row_arb.efi"),
        ] {
            let mut data = b"product_region\0\0".to_vec();
            data.extend_from_slice(&[0, 0, 0, 3, 0, 0, 0, 4, 0, 0, 0x26, 0xb7]);
            data.extend_from_slice(region);
            std::fs::write(&path, data).unwrap();
            assert_eq!(efisp_suffix_for_vendor_boot(&path, false).unwrap(), stock);
            assert_eq!(efisp_suffix_for_vendor_boot(&path, true).unwrap(), arb);
        }
    }

    #[test]
    fn automatic_targets_keep_firmware_index_or_raise_to_device_floor() {
        let (targets, need) = select_arb_rollback_targets(
            ltbox_patch::rollback::RollbackIndices {
                boot: 40,
                vbmeta_system: 80,
            },
            ltbox_patch::rollback::RollbackIndices {
                boot: 50,
                vbmeta_system: 70,
            },
            None,
        )
        .unwrap();

        assert_eq!(
            targets,
            ltbox_patch::rollback::RollbackIndices {
                boot: 50,
                vbmeta_system: 80,
            }
        );
        assert!(need);
    }

    #[test]
    fn manual_targets_are_exact_and_need_tracks_index_changes() {
        let (targets, need) = select_arb_rollback_targets(
            ltbox_patch::rollback::RollbackIndices {
                boot: 40,
                vbmeta_system: 80,
            },
            ltbox_patch::rollback::RollbackIndices {
                boot: 30,
                vbmeta_system: 70,
            },
            Some(ltbox_patch::rollback::RollbackIndices {
                boot: 35,
                vbmeta_system: 95,
            }),
        )
        .unwrap();

        assert_eq!(
            targets,
            ltbox_patch::rollback::RollbackIndices {
                boot: 35,
                vbmeta_system: 95,
            }
        );
        assert!(need);
    }

    #[test]
    fn unchanged_manual_targets_do_not_require_arb_gbl() {
        let requested = ltbox_patch::rollback::RollbackIndices {
            boot: 40,
            vbmeta_system: 80,
        };
        let (targets, need) = select_arb_rollback_targets(
            requested,
            ltbox_patch::rollback::RollbackIndices {
                boot: 30,
                vbmeta_system: 70,
            },
            Some(requested),
        )
        .unwrap();

        assert_eq!(targets, requested);
        assert!(!need);
    }

    #[test]
    fn manual_targets_below_device_floor_are_rejected() {
        let result = select_arb_rollback_targets(
            ltbox_patch::rollback::RollbackIndices {
                boot: 40,
                vbmeta_system: 80,
            },
            ltbox_patch::rollback::RollbackIndices {
                boot: 50,
                vbmeta_system: 70,
            },
            Some(ltbox_patch::rollback::RollbackIndices {
                boot: 49,
                vbmeta_system: 80,
            }),
        );

        assert!(result.is_err());
    }
}

#[cfg(test)]
mod abl_preparation_tests {
    use super::*;

    /// Weekly download-only verification for every pinned efisp GBL variant.
    /// The production suffix selector and hash verifier are used, but staged
    /// files live only in `tempfile`; no EDL session or provisioning call is
    /// made.
    #[test]
    #[ignore = "weekly_fetch: downloads and hashes pinned efisp GBL variants"]
    fn weekly_fetch_efisp_gbl_variants() {
        let mut failures = Vec::new();
        let client = match ltbox_core::github::GitHubClient::from_url(
            "github.com/miner7222/gbl_root_baldur",
        ) {
            Ok(client) => client,
            Err(error) => {
                failures.push(format!("create GitHub client: {error}"));
                assert!(
                    failures.is_empty(),
                    "weekly efisp fetch failures:\n{}",
                    failures.join("\n")
                );
                return;
            }
        };
        let assets = match client.release_by_tag(EFISP_GBL_RELEASE_TAG) {
            Ok(assets) => assets,
            Err(error) => {
                failures.push(format!("load {EFISP_GBL_RELEASE_TAG} assets: {error}"));
                assert!(
                    failures.is_empty(),
                    "weekly efisp fetch failures:\n{}",
                    failures.join("\n")
                );
                return;
            }
        };
        for suffix in ["_prc.efi", "_prc_arb.efi", "_row.efi", "_row_arb.efi"] {
            let result = (|| -> std::result::Result<(), String> {
                let asset_name = efisp_expected_asset(suffix)
                    .ok_or_else(|| "production efisp selector returned no asset".to_string())?;
                let asset_url = assets
                    .iter()
                    .find(|(name, _)| name == asset_name)
                    .map(|(_, url)| url)
                    .ok_or_else(|| format!("release lacks {asset_name}"))?;
                let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
                let path = temp.path().join(asset_name);
                ltbox_core::downloader::download_to_file(asset_url, &path, &mut Vec::new())
                    .map_err(|error| error.to_string())?;
                verify_efisp_asset(&path, asset_name)
            })();
            if let Err(error) = result {
                failures.push(format!("{suffix}: {error}"));
            }
        }
        assert!(
            failures.is_empty(),
            "weekly efisp fetch failures:\n{}",
            failures.join("\n")
        );
    }

    #[test]
    fn rejected_abl_prevents_efisp_read_and_fetch() {
        for slot in ["_a", "_b"] {
            let dir = tempfile::tempdir().unwrap();
            let mut reads = Vec::new();
            let result = prepare_canoe_efisp_with(
                slot,
                None,
                dir.path(),
                dir.path(),
                &mut Vec::new(),
                &mut |partition, path, lun, _| {
                    reads.push((partition.to_owned(), lun));
                    std::fs::write(path, b"unrecognized ABL").unwrap();
                    Ok(())
                },
                &mut |_, _, _| panic!("unverified ABL must not fetch GBL"),
            );
            assert!(result.is_err());
            let partition = format!("abl{slot}");
            assert_eq!(
                reads,
                vec![(
                    partition.clone(),
                    ltbox_core::partition_lun::lun_for_partition(&partition).unwrap()
                )]
            );
            assert!(!dir.path().join("efisp.img").exists());
        }
    }
}
