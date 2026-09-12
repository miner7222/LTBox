//! Firmware flash worker: validate the firmware folder, route to EDL,
//! apply region / rollback / country / wipe modifications, and flash every
//! image (incl. the TB323FU ARB-overlay path). Extracted from the
//! update_flash handler.

use crate::{
    ConnectionStatus, CountryPatchProgress, FirmwareIdentity, LiveLabels, PhaseReporter,
    WorkflowConfig, active_slot_suffix, build_testkey_arb_overlays, efisp_suffix_for_vendor_boot,
    fetch_efisp_asset, find_firmware_loader, fingerprint_token_match, is_rollback_protected_model,
    open_edl_session, read_device_rollback_index_via_edl, transition_to_edl,
};
use ltbox_core::{live, tr_args};

fn require_firmware_loader(
    loader: Option<std::path::PathBuf>,
) -> std::result::Result<std::path::PathBuf, String> {
    loader.ok_or_else(|| ltbox_core::i18n::tr("live_edl_loader_missing"))
}

fn should_reboot_fastboot_to_system_after_pre_edl_abort(started_in_fastboot: bool) -> bool {
    !started_in_fastboot
}

fn reboot_fastboot_to_system_after_pre_edl_abort(log: &mut Vec<String>, started_in_fastboot: bool) {
    if !should_reboot_fastboot_to_system_after_pre_edl_abort(started_in_fastboot) {
        return;
    }
    if !ltbox_device::fastboot::FastbootDevice::check_device() {
        return;
    }
    ltbox_core::live!(
        log,
        "[Fastboot] {}",
        ltbox_core::i18n::tr("live_fastboot_rebooting_system")
    );
    match ltbox_device::fastboot::FastbootDevice::open() {
        Ok(mut dev) => {
            if let Err(e) = dev.reboot() {
                ltbox_core::live!(
                    log,
                    "[Fastboot] {}",
                    tr_args!("live_fastboot_reboot_failed", error = e.to_string())
                );
            }
        }
        Err(e) => {
            ltbox_core::live!(
                log,
                "[Fastboot] {}",
                tr_args!("err_fastboot_open_failed", error = e.to_string())
            );
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LenovoFirmwareDevicePolicy {
    AbortUnknown,
    KeepLenovo,
    ResignTestkey,
}

/// Decide how a Lenovo-key firmware should be handled from the device's
/// active-slot vbmeta_system key class. vbmeta_system may use either bundled
/// testkey size while the root vbmeta still uses testkey_rsa4096, so the exact
/// vbmeta_system key spec must not gate the testkey re-sign path. A user-supplied
/// testkey ABL selects that path regardless of the device key class.
fn lenovo_firmware_device_policy(
    device_vbs_class: ltbox_patch::key_map::KeyClass,
    user_abl_supplied: bool,
) -> LenovoFirmwareDevicePolicy {
    if user_abl_supplied {
        return LenovoFirmwareDevicePolicy::ResignTestkey;
    }
    match device_vbs_class {
        ltbox_patch::key_map::KeyClass::Unknown => LenovoFirmwareDevicePolicy::AbortUnknown,
        ltbox_patch::key_map::KeyClass::Lenovo => LenovoFirmwareDevicePolicy::KeepLenovo,
        ltbox_patch::key_map::KeyClass::Testkey => LenovoFirmwareDevicePolicy::ResignTestkey,
    }
}

fn effective_flash_rollback_mode(
    mode: ltbox_patch::rollback::RollbackMode,
    target_is_canoe: bool,
    xiaoxin_pro13_flash: bool,
) -> ltbox_patch::rollback::RollbackMode {
    use ltbox_patch::rollback::RollbackMode;
    if xiaoxin_pro13_flash || (target_is_canoe && mode == RollbackMode::On) {
        RollbackMode::Auto
    } else {
        mode
    }
}

/// Never lose a known stored floor when also reading fallback image indices.
/// Unclassified Fastboot layouts retain the existing conservative aggregate.
fn manual_device_floors(
    edl: Option<(u64, u64)>,
    fastboot: Option<ltbox_patch::rollback::FastbootRollbackFloors>,
    aggregate: Option<u64>,
) -> Option<ltbox_patch::rollback::RollbackIndices> {
    let stored = fastboot
        .map(|f| (f.boot_index, f.vbmeta_system_index))
        .or_else(|| aggregate.map(|index| (index, index)));
    let (boot, vbmeta_system) = match (edl, stored) {
        (Some((b, v)), Some((sb, sv))) => (b.max(sb), v.max(sv)),
        (Some(floors), None) | (None, Some(floors)) => floors,
        (None, None) => return None,
    };
    Some(ltbox_patch::rollback::RollbackIndices {
        boot,
        vbmeta_system,
    })
}

/// Supported Lenovo SKUs, matched as fingerprint tokens to recover the device
/// model from an EDL-dumped vendor_boot when fastboot/ADB never ran.
/// LAVIE Tab 9QHD1 is represented by TB320FC because the shared matcher treats
/// its reported token as equivalent and the recovered value is internal only.
use ltbox_core::model::{RollbackPolicy, SUPPORTED_MODELS, capabilities, fingerprint_capabilities};

fn xiaoxin_pro13_token(text: &str) -> Option<&'static str> {
    [
        ltbox_core::model::TB376FC_MODEL,
        ltbox_core::model::TB390FU_MODEL,
    ]
    .into_iter()
    .find(|model| {
        text.split(|c: char| !c.is_ascii_alphanumeric())
            .any(|token| token.eq_ignore_ascii_case(model))
    })
}

fn xiaoxin_pro13_cross_model(device_model: &str, firmware_fingerprint: Option<&str>) -> bool {
    matches!(
        (
            xiaoxin_pro13_token(device_model),
            firmware_fingerprint.and_then(xiaoxin_pro13_token),
        ),
        (Some(device), Some(firmware)) if !device.eq_ignore_ascii_case(firmware)
    )
}

fn supported_model_identity_match(fingerprint: &str, model: &str) -> bool {
    if ltbox_core::model::is_xiaoxin_pro13_model(model) {
        xiaoxin_pro13_token(fingerprint).is_some_and(|token| token.eq_ignore_ascii_case(model))
    } else {
        fingerprint_token_match(fingerprint, model)
    }
}

/// EDL-start device identity + rollback floor, read over the open EDL session.
struct EdlStartProbe {
    /// Device model token recovered from the vbmeta_system fingerprint, or empty
    /// when no supported SKU token matched.
    model_token: String,
    /// Component-wise device rollback floors `(boot, vbmeta_system)` — the MAX
    /// of each location across both slots — or `None` for the no-ARB TB322FC.
    rollback_floors: Option<(u64, u64)>,
}

/// Read the device model + committed rollback index on an EDL-start flash by
/// dumping BOTH slots over the open session. The active slot is unknown in
/// EDL-start and the inactive slot's images may not carry parseable AVB info,
/// so each partition is dumped from `_a` and `_b` and the valid side is used
/// (the higher-index slot when both parse). Returns `Err` (the caller resets
/// back into EDL and aborts) when neither slot yields a valid vbmeta_system,
/// when the recovered model does not match the target firmware, or — for a
/// rollback-protected model — when neither slot yields a valid boot +
/// vbmeta_system pair.
fn read_edl_start_device(
    session: &mut ltbox_device::edl::EdlSession,
    firmware_fp: Option<&str>,
    log: &mut Vec<String>,
) -> std::result::Result<EdlStartProbe, String> {
    let work_dir = ltbox_core::app_paths::work_dir_for("flash_edl_probe");
    let _ = std::fs::remove_dir_all(&work_dir);
    std::fs::create_dir_all(&work_dir)
        .map_err(|e| tr_args!("err_edl_probe_work_dir_failed", error = e))?;

    // 1. Model + vbmeta_system rollback index — dump vbmeta_system from BOTH
    //    slots. It carries the device build fingerprint (system.fingerprint) AND
    //    the per-location rollback index, so one dump identifies the model and
    //    yields the vbmeta_system floor; vendor_boot / vbmeta need no separate
    //    dump. A device left mid-cross-flash can have one slot on a different
    //    SKU, so keep every fingerprint that parses.
    let mut device_fps: Vec<String> = Vec::new();
    let mut vbs_idx: [Option<u64>; 2] = [None, None];
    for (i, slot) in ["_a", "_b"].into_iter().enumerate() {
        let part = format!("vbmeta_system{slot}");
        let Some(lun) = ltbox_core::partition_lun::lun_for_partition(&part) else {
            continue;
        };
        let out = work_dir.join(format!("dev_{part}.img"));
        if let Some(info) = session
            .dump_partition(&part, &out, 0, lun, log)
            .ok()
            .and_then(|_| ltbox_patch::avb::extract_image_avb_info(&out).ok())
        {
            if let Some(fp) = ltbox_patch::avb::build_fingerprint(&info) {
                device_fps.push(fp);
            }
            vbs_idx[i] = Some(info.rollback_index);
        }
        let _ = std::fs::remove_file(&out);
    }
    if device_fps.is_empty() {
        return Err(ltbox_core::i18n::tr(
            if firmware_fp.and_then(xiaoxin_pro13_token).is_some() {
                "err_flash_xiaoxin_arb_floor_unreadable"
            } else {
                "err_flash_edl_avb_invalid"
            },
        ));
    }

    // Recover the model token, preferring a slot whose SKU the target firmware
    // also names — a device left mid-cross-flash can carry a stale non-matching
    // slot alongside the matching one. On EDL-start there is no other model
    // source, so an unrecognized device, a firmware image with no vendor_boot
    // fingerprint, or a genuine mismatch all abort (the caller resets back into
    // EDL) rather than flash blind — TB323FU especially, since its ARB/region
    // gates key off the model and would silently stay off on an unidentified
    // device.
    let mut model_token = String::new();
    'find: for fp in &device_fps {
        for m in SUPPORTED_MODELS {
            if supported_model_identity_match(fp, m) {
                let matches_fw = firmware_fp
                    .map(|fw| fingerprint_token_match(fw, m))
                    .unwrap_or(false);
                if matches_fw {
                    model_token = m.to_string();
                    break 'find;
                }
                if model_token.is_empty() {
                    model_token = m.to_string();
                }
            }
        }
    }
    let firmware_matches = firmware_fp
        .map(|fw| fingerprint_token_match(fw, &model_token))
        .unwrap_or(false);
    if model_token.is_empty() || !firmware_matches {
        ltbox_core::live!(
            log,
            "[Flash] {}",
            tr_args!(
                "live_rescue_model_mismatch_abort",
                device = if model_token.is_empty() {
                    device_fps.join(", ")
                } else {
                    model_token.clone()
                },
                fingerprint = firmware_fp.unwrap_or("").to_string()
            )
        );
        return Err(ltbox_core::i18n::tr("err_flash_model_mismatch_pre_edl"));
    }
    ltbox_core::live!(
        log,
        "[Flash] {}",
        ltbox_core::i18n::tr("live_rescue_model_check_ok")
    );

    // 2. TB322FC has no rollback protection — skip the index read.
    if !capabilities(&model_token).rollback.is_protected() {
        return Ok(EdlStartProbe {
            model_token,
            rollback_floors: None,
        });
    }

    // 3. Rollback floor — vbmeta_system was read above; add the boot floor by
    //    dumping boot from both slots, then take the per-location MAX across
    //    slots. AVB rollback indices are per-location and the two slots can hold
    //    different images, so a single slot can underestimate one location (and
    //    TB323FU re-signs each location to its own floor). Each location must
    //    parse on at least one slot.
    ltbox_core::live!(log, "[ARB] {}", ltbox_core::i18n::tr("live_arb_edl_dump"));
    let mut boot_idx: [Option<u64>; 2] = [None, None];
    for (i, slot) in ["_a", "_b"].into_iter().enumerate() {
        let boot = format!("boot{slot}");
        let Some(boot_lun) = ltbox_core::partition_lun::lun_for_partition(&boot) else {
            continue;
        };
        let boot_img = work_dir.join(format!("dev_{boot}.img"));
        boot_idx[i] = session
            .dump_partition(&boot, &boot_img, 0, boot_lun, log)
            .ok()
            .and_then(|_| ltbox_patch::avb::extract_image_avb_info(&boot_img).ok())
            .map(|info| info.rollback_index);
        let _ = std::fs::remove_file(&boot_img);
    }
    let Some(floors) = rollback_floors(boot_idx, vbs_idx) else {
        return Err(ltbox_core::i18n::tr(
            if capabilities(&model_token).rollback == RollbackPolicy::ReadOnly {
                "err_flash_xiaoxin_arb_floor_unreadable"
            } else {
                "err_flash_edl_avb_invalid"
            },
        ));
    };
    Ok(EdlStartProbe {
        model_token,
        rollback_floors: Some(floors),
    })
}

/// Component-wise device rollback floors from per-slot boot / vbmeta_system
/// indices (`None` = that slot's image did not parse). Each location's floor is
/// the MAX across both slots, because AVB rollback indices are per-location and
/// the two slots can hold different images. Returns `None` (the caller aborts)
/// when a location parsed on neither slot.
fn rollback_floors(boot: [Option<u64>; 2], vbs: [Option<u64>; 2]) -> Option<(u64, u64)> {
    let boot_floor = boot.into_iter().flatten().max()?;
    let vbs_floor = vbs.into_iter().flatten().max()?;
    Some((boot_floor, vbs_floor))
}

/// Dump a partition over EDL and parse its AVB info, cleaning up the temp file.
/// `None` on any dump/parse failure. Shared with `workers::unroot`, which
/// compares the device image against the backup folder before writing.
pub(super) fn dump_avb_info(
    session: &mut ltbox_device::edl::EdlSession,
    part: &str,
    lun: u8,
    work_dir: &std::path::Path,
    log: &mut Vec<String>,
) -> Option<ltbox_patch::avb::AvbImageInfo> {
    let out = work_dir.join(format!("dev_{part}.img"));
    let info = session
        .dump_partition(part, &out, 0, lun, log)
        .ok()
        .and_then(|_| ltbox_patch::avb::extract_image_avb_info(&out).ok());
    let _ = std::fs::remove_file(&out);
    info
}

/// Device active-slot identity (read from vbmeta_system), for the AVB key-class
/// policy.
struct DeviceVbmeta {
    /// Active slot suffix (`_a` / `_b`).
    slot: &'static str,
    /// Device key class indicated by the active-slot vbmeta_system signer.
    class: ltbox_patch::key_map::KeyClass,
    /// Device-committed per-location rollback floors — the MAX of each location
    /// across BOTH slots (AVB indices are per-location; the slots can differ).
    boot_floor: u64,
    vbs_floor: u64,
}

/// Read the device's active-slot key class + committed rollback floors from
/// vbmeta_system — the unified identity source: its signer identifies the device
/// key class, and it carries the build fingerprint + per-location rollback index,
/// so vbmeta / vendor_boot need no separate dump here. Its exact signer is not
/// necessarily the root vbmeta signer. The active slot is
/// `known_active` (fastboot `current-slot`) when set; otherwise (EDL-start) it
/// is the valid vbmeta_system side, or — when both parse — the slot with the
/// higher vbmeta_system rollback index (ties favour `_a`). The floors are
/// component-wise maxima of `boot` + `vbmeta_system` across BOTH slots (the
/// device's true committed floor — a single slot can understate one location).
/// Errors (the caller resets to the device's start mode and aborts) when no
/// slot's vbmeta_system / floors parse.
fn read_device_vbmeta(
    session: &mut ltbox_device::edl::EdlSession,
    known_active: Option<&str>,
    work_dir: &std::path::Path,
    log: &mut Vec<String>,
) -> std::result::Result<DeviceVbmeta, String> {
    let vbs_lun = ltbox_core::partition_lun::lun_for_partition("vbmeta_system")
        .ok_or_else(|| "no LUN for vbmeta_system".to_string())?;
    let boot_lun = ltbox_core::partition_lun::lun_for_partition("boot")
        .ok_or_else(|| "no LUN for boot".to_string())?;

    // Per slot: full vbmeta_system AVB info (validity + pubkey + rollback index)
    // and the boot rollback index.
    let mut vbs_info: [Option<ltbox_patch::avb::AvbImageInfo>; 2] = [None, None];
    let mut boot_idx: [Option<u64>; 2] = [None, None];
    for (i, slot) in ["_a", "_b"].into_iter().enumerate() {
        vbs_info[i] = dump_avb_info(
            session,
            &format!("vbmeta_system{slot}"),
            vbs_lun,
            work_dir,
            log,
        );
        boot_idx[i] = dump_avb_info(session, &format!("boot{slot}"), boot_lun, work_dir, log)
            .map(|info| info.rollback_index);
    }

    // Active slot: fastboot's when known; otherwise the valid vbmeta_system side,
    // or (both valid) the higher vbmeta_system rollback index, ties favour `_a`.
    let active = match known_active {
        Some(s) => usize::from(active_slot_suffix(Some(s)) == "_b"),
        None => match (vbs_info[0].as_ref(), vbs_info[1].as_ref()) {
            (Some(_), None) => 0,
            (None, Some(_)) => 1,
            (Some(a), Some(b)) => usize::from(b.rollback_index > a.rollback_index),
            (None, None) => {
                return Err("no device vbmeta_system parsed on either slot".to_string());
            }
        },
    };
    let slot = if active == 1 { "_b" } else { "_a" };

    // Device key class from the active-slot vbmeta_system pubkey. Do not retain
    // its exact testkey spec: vbmeta_system may use RSA-2048 while root vbmeta
    // uses RSA-4096, and Lenovo-key-firmware re-sign targets that RSA-4096 root.
    let active_info = vbs_info[active]
        .as_ref()
        .ok_or_else(|| format!("device vbmeta_system{slot} AVB unreadable"))?;
    let pubkey = active_info.public_key_sha1.as_deref();
    let class = ltbox_patch::key_map::classify_pubkey(pubkey);

    // Component-wise device rollback floor (max per location across both slots).
    let vbs_idx = [
        vbs_info[0].as_ref().map(|i| i.rollback_index),
        vbs_info[1].as_ref().map(|i| i.rollback_index),
    ];
    let (boot_floor, vbs_floor) = rollback_floors(boot_idx, vbs_idx)
        .ok_or_else(|| "device boot/vbmeta_system AVB unreadable on both slots".to_string())?;

    Ok(DeviceVbmeta {
        slot,
        class,
        boot_floor,
        vbs_floor,
    })
}

/// Back up the device's active-slot `abl` (the bootloader) for later restore.
/// Returns `(lun, backup_path)` — the LUN holds both `abl_a`/`abl_b`, and the
/// restore later targets `abl_a` (firmware always lands on `_a`).
fn backup_device_abl(
    session: &mut ltbox_device::edl::EdlSession,
    slot: &str,
    work_dir: &std::path::Path,
    log: &mut Vec<String>,
) -> std::result::Result<(u8, std::path::PathBuf), String> {
    let abl_part = format!("abl{slot}");
    // Static LUN map (abl is mapped to LUN 4) with a GPT-scan fallback.
    let lun = session
        .lun_for(&abl_part, log)
        .map_err(|e| format!("resolve LUN for {abl_part}: {e}"))?;
    let out = work_dir.join("dev_abl_backup.img");
    session
        .dump_partition(&abl_part, &out, 0, lun, log)
        .map_err(|e| format!("dump device {abl_part}: {e}"))?;
    Ok((lun, out))
}

/// Best-effort restore of the backed-up device `abl` onto `abl_a` for the flash
/// error paths: once the firmware's own (Lenovo-key) abl may have been written,
/// the original testkey abl must go back even on failure, or the device is left
/// with a Lenovo-key bootloader on a testkey-resigned chain. Failures here are
/// logged and swallowed — the caller is already returning an error and leaves
/// the device in EDL for retry.
fn restore_abl_best_effort(
    session: &mut ltbox_device::edl::EdlSession,
    abl_restore: &Option<(u8, std::path::PathBuf)>,
    log: &mut Vec<String>,
) {
    if let Some((lun, abl_img)) = abl_restore {
        ltbox_core::live!(
            log,
            "[ARB] {}",
            ltbox_core::i18n::tr("live_flash_abl_restore")
        );
        let _ = session.flash_partition("abl_a", abl_img, 0, *lun, log);
    }
}

/// Decrypt every shipped rawprogram `.x` in `fw_dir` in place, writing each to
/// a sibling `<stem>.xml` so the catalog scan can resolve image paths. The
/// encrypted Sahara manifest (`qsahara_device_programmer.x`) is a loader, not a
/// flash image, so it is left for `EdlSession::open` to decrypt at load time.
/// Logs the decrypted count and returns it. Unpacks the firmware as shipped —
/// not a content modification.
fn decrypt_rawprogram_x_files(
    fw_dir: &std::path::Path,
    log: &mut Vec<String>,
) -> std::result::Result<usize, String> {
    let x_entries: Vec<std::path::PathBuf> = std::fs::read_dir(fw_dir)
        .map_err(|e| {
            tr_args!(
                "err_read_dir_failed",
                path = fw_dir.display().to_string(),
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
                && !ltbox_core::sahara_xml::is_encrypted_manifest_filename(p)
        })
        .collect();
    if x_entries.is_empty() {
        return Ok(0);
    }
    let mut decrypted = 0usize;
    for src in &x_entries {
        let stem = src.file_stem().unwrap_or_default();
        let output = fw_dir.join(stem).with_extension("xml");
        ltbox_core::crypto::decrypt_file(src, &output).map_err(|e| {
            tr_args!(
                "err_decrypt_file_failed",
                path = src.display().to_string(),
                error = e.to_string()
            )
        })?;
        decrypted += 1;
    }
    live!(
        log,
        "[XML] {}",
        tr_args!("live_xml_decrypt_done", count = decrypted.to_string())
    );
    Ok(decrypted)
}

/// Recursively collect `*.zst` files under `dir` (bounded depth so a symlink
/// loop can't spin), appending to `out`. A firmware tree is shallow; the depth
/// cap is a safety net, not a real limit.
fn collect_zst_files(
    dir: &std::path::Path,
    depth: u32,
    out: &mut Vec<std::path::PathBuf>,
) -> std::result::Result<(), String> {
    const MAX_DEPTH: u32 = 8;
    if depth > MAX_DEPTH {
        return Ok(());
    }
    let entries = std::fs::read_dir(dir).map_err(|e| {
        tr_args!(
            "err_read_dir_failed",
            path = dir.display().to_string(),
            error = e.to_string()
        )
    })?;
    for path in entries.filter_map(|r| r.ok().map(|e| e.path())) {
        if path.is_dir() {
            collect_zst_files(&path, depth + 1, out)?;
        } else if path.is_file()
            && path
                .extension()
                .and_then(|s| s.to_str())
                .map(|s| s.eq_ignore_ascii_case("zst"))
                .unwrap_or(false)
        {
            out.push(path);
        }
    }
    Ok(())
}

/// Decompress any `*.zst` partition images in place so the rawprogram
/// references resolve. Ported ROMs ship e.g. `super.img.zst` (zstd-compressed)
/// while the rawprogram XML names `super.img`. The scan recurses into
/// subdirectories because a rawprogram can reference an image through a relative
/// subdir (`images/super.img`). A `*.zst` whose decompressed target already
/// exists is skipped, and the source `.zst` is kept (LTBox never deletes user
/// files). Streamed — the output can be tens of GB — with periodic progress in
/// the live log. Returns the number of files decompressed.
fn decompress_zst_images(
    fw_dir: &std::path::Path,
    log: &mut Vec<String>,
) -> std::result::Result<usize, String> {
    let mut zst_entries: Vec<std::path::PathBuf> = Vec::new();
    collect_zst_files(fw_dir, 0, &mut zst_entries)?;
    if zst_entries.is_empty() {
        return Ok(0);
    }
    let mut count = 0usize;
    for src in &zst_entries {
        // `super.img.zst` → `super.img` (drop the trailing `.zst`).
        let target = src.with_extension("");
        if target.exists() {
            continue;
        }
        let name = src
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        live!(
            log,
            "[Flash] {}",
            tr_args!("live_flash_zst_decompress", name = &name)
        );
        decompress_zst_file(src, &target, &name, log).map_err(|e| {
            tr_args!(
                "err_flash_zst_decompress_failed",
                path = src.display().to_string(),
                error = e
            )
        })?;
        count += 1;
    }
    Ok(count)
}

/// Stream-decompress one zstd file, logging progress every ~1 GiB written.
///
/// Writes to a temporary sibling and renames onto `dst` only after a clean
/// flush, so an interrupted run (kill / power loss) never leaves a
/// complete-looking but truncated `*.img` that a later run would skip
/// (`target.exists()`) and then flash.
const ZSTD_FREE_SPACE_RESERVE_BYTES: u64 = 1024 * 1024 * 1024;
const ZSTD_PROGRESS_INTERVAL_BYTES: u64 = 1 << 30;

fn zstd_output_limit(available_space: u64) -> std::result::Result<u64, String> {
    available_space
        .checked_sub(ZSTD_FREE_SPACE_RESERVE_BYTES)
        .filter(|limit| *limit > 0)
        .ok_or_else(|| {
            format!(
                "need more than {} GiB free before decompressing a zstd image",
                ZSTD_FREE_SPACE_RESERVE_BYTES / (1024 * 1024 * 1024)
            )
        })
}

fn stream_zstd_decoder<R: std::io::Read, W: std::io::Write>(
    mut decoder: R,
    mut out: W,
    name: &str,
    log: &mut Vec<String>,
    output_limit: u64,
) -> std::result::Result<W, String> {
    let mut buf = vec![0u8; 4 * 1024 * 1024];
    let mut total = 0u64;
    let mut next_mark = ZSTD_PROGRESS_INTERVAL_BYTES;
    loop {
        let n = decoder.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        let next_total = total
            .checked_add(u64::try_from(n).map_err(|e| e.to_string())?)
            .ok_or_else(|| "zstd output size overflowed u64".to_string())?;
        if next_total > output_limit {
            return Err(format!(
                "zstd output limit of {} GiB reached while decompressing {name}",
                output_limit / (1024 * 1024 * 1024)
            ));
        }
        out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        total = next_total;
        if total >= next_mark {
            live!(
                log,
                "[Flash] {}",
                tr_args!(
                    "live_flash_zst_progress",
                    name = name,
                    gb = format!("{:.1}", total as f64 / 1_073_741_824.0)
                )
            );
            next_mark = next_mark.saturating_add(ZSTD_PROGRESS_INTERVAL_BYTES);
        }
    }
    Ok(out)
}

fn decompress_zst_file(
    src: &std::path::Path,
    dst: &std::path::Path,
    name: &str,
    log: &mut Vec<String>,
) -> std::result::Result<(), String> {
    // `<dst>.ltbox-zst-tmp`, a sibling so the rename stays on one filesystem.
    let tmp = {
        let mut s = dst.as_os_str().to_owned();
        s.push(".ltbox-zst-tmp");
        std::path::PathBuf::from(s)
    };
    let result = (|| -> std::result::Result<(), String> {
        let dst_parent = dst.parent().ok_or_else(|| {
            format!(
                "cannot determine destination directory for {}",
                dst.display()
            )
        })?;
        // Preserve enough room for LTBox, logs, and recovery files.  This is a
        // per-output limit instead of a fixed image-size cap because firmware
        // images legitimately span many tens of GiB.
        let output_limit =
            zstd_output_limit(fs2::available_space(dst_parent).map_err(|e| e.to_string())?)?;
        let input = std::fs::File::open(src).map_err(|e| e.to_string())?;
        // `Decoder::new` wraps the reader in its own `BufReader`.
        let decoder = zstd::stream::Decoder::new(input).map_err(|e| e.to_string())?;
        let out = std::io::BufWriter::new(std::fs::File::create(&tmp).map_err(|e| e.to_string())?);
        let out = stream_zstd_decoder(decoder, out, name, log, output_limit)?;
        // Drain the buffer, then fsync the data BEFORE publishing the name —
        // `flush` only reaches the OS cache, so without `sync_all` a crash after
        // the rename could leave `dst` pointing at unflushed (truncated) bytes
        // that a later run would `target.exists()`-skip and then flash.
        let file = out.into_inner().map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        // Atomic publish: the final name only ever appears fully written.
        std::fs::rename(&tmp, dst).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Country-code partitions to dump/patch/flash for a model. The TB320FC hardware
/// path and TB323FU keep the code ONLY in `oemowninfo` (LUN 0); every other model
/// keeps it in `devinfo` + `persist`. The model is matched against the
/// vendor_boot AVB fingerprint (works on an EDL-start flash) or the
/// probe-reported model name.
fn country_partitions_for(
    device_model: &str,
    firmware_fingerprint: Option<&str>,
) -> &'static [&'static str] {
    let xiaoxin_pro13_sku = [
        ltbox_core::model::TB376FC_MODEL,
        ltbox_core::model::TB390FU_MODEL,
    ]
    .iter()
    .any(|m| {
        firmware_fingerprint
            .map(|fp| fingerprint_token_match(fp, m))
            .unwrap_or(false)
            || fingerprint_token_match(device_model, m)
    });
    if xiaoxin_pro13_sku {
        return &["proinfo", "persist"];
    }
    let oemowninfo_sku = ["TB320FC", "TB323FU"].iter().any(|m| {
        firmware_fingerprint
            .map(|fp| fingerprint_token_match(fp, m))
            .unwrap_or(false)
            || fingerprint_token_match(device_model, m)
    });
    if oemowninfo_sku {
        &["oemowninfo"]
    } else {
        &["devinfo", "persist"]
    }
}

fn is_field_only_country_partition(label: &str) -> bool {
    matches!(label, "persist" | "proinfo")
}

/// Rewrite the device's country code in the model's country partitions over an
/// open EDL session: the TB320FC hardware path and TB323FU use `oemowninfo`;
/// every other model uses `devinfo` + `persist`. Best-effort per partition
/// (logs + continues on failure). Shared by `flash_worker`'s post-flash country
/// phase and the standalone `change_country_worker`.
#[allow(clippy::too_many_arguments)]
fn run_country_change(
    session: &mut ltbox_device::edl::EdlSession,
    work_dir: &std::path::Path,
    critical_backup: &std::path::Path,
    backup_operation: &str,
    backup_fingerprint: Option<&str>,
    device_model: &str,
    firmware_fingerprint: Option<&str>,
    target_code: Option<&str>,
    flip_proinfo_channel: bool,
    only_partitions: Option<&'static [&'static str]>,
    ll: &LiveLabels,
    log: &mut Vec<String>,
    phases: Option<&PhaseReporter>,
) -> Result<(), String> {
    use ltbox_patch::region::{EU_COUNTRY_CODES as EU_CODES, KNOWN_COUNTRY_CODES as KNOWN_CODES};

    struct CountryPartitionWork {
        label: &'static str,
        lun: u8,
        dump_path: std::path::PathBuf,
        detected: Option<String>,
    }

    let country_partitions = only_partitions
        .unwrap_or_else(|| country_partitions_for(device_model, firmware_fingerprint));
    let mut country_progress = CountryPatchProgress::new(country_partitions);
    if let Some(phases) = phases {
        live!(log, "[Country] {}", phases.marker(3));
    }
    let mut staged = Vec::with_capacity(country_partitions.len());
    for label in country_partitions.iter().copied() {
        let Some(lun) = ltbox_core::partition_lun::lun_for_partition(label) else {
            let reason = ltbox_core::i18n::tr("country_reason_no_lun");
            ltbox_core::live!(
                log,
                "[Country] {}",
                tr_args!(
                    "live_country_partition_status",
                    label = label,
                    reason = reason
                )
            );
            country_progress.mark_failed(label, reason);
            continue;
        };
        let dump_path = work_dir.join(format!("{label}.img"));
        live!(
            log,
            "[Country] {}",
            tr_args!(
                "live_country_dump_partition",
                label = label,
                lun = lun.to_string(),
                start = "?",
                sectors = "?"
            )
        );
        if let Err(e) = session.dump_partition(label, &dump_path, 0, lun, log) {
            let reason = tr_args!("country_reason_dump_failed", error = e);
            ltbox_core::live!(
                log,
                "[Country] {}",
                tr_args!(
                    "live_country_partition_status",
                    label = label,
                    reason = reason
                )
            );
            country_progress.mark_failed(label, reason);
            continue;
        }
        // Backup before any patch touches it.
        if let Err(e) = std::fs::copy(&dump_path, critical_backup.join(format!("{label}.img"))) {
            let reason = tr_args!("country_reason_backup_failed", error = e);
            ltbox_core::live!(
                log,
                "[Country] {}",
                tr_args!(
                    "live_country_partition_status",
                    label = label,
                    reason = reason
                )
            );
            country_progress.mark_failed(label, reason);
            continue;
        }
        let detected = if target_code.is_some() {
            match ltbox_patch::region::detect_country_code(
                &dump_path,
                KNOWN_CODES,
                is_field_only_country_partition(label),
            ) {
                Ok(c) => c,
                Err(e) => {
                    let reason = tr_args!("country_reason_detect_failed", error = e);
                    ltbox_core::live!(
                        log,
                        "[Country] {}",
                        tr_args!(
                            "live_country_partition_status",
                            label = label,
                            reason = reason
                        )
                    );
                    country_progress.mark_failed(label, reason);
                    continue;
                }
            }
        } else {
            None
        };

        staged.push(CountryPartitionWork {
            label,
            lun,
            dump_path,
            detected,
        });
    }

    if let Err(error) = crate::backup::write_backup_manifest(
        critical_backup,
        backup_operation,
        device_model,
        backup_fingerprint,
        None,
    ) {
        live!(log, "[Backup] Could not record manifest: {error}");
    }

    if let Some(phases) = phases {
        live!(log, "[Country] {}", phases.marker(4));
    }
    for work in staged {
        let CountryPartitionWork {
            label,
            lun,
            dump_path,
            detected,
        } = work;
        let patched_path = work_dir.join(format!("{label}.patched.img"));
        // Patch the country code when the partition carries
        // one. `persist` has no real country code (its only
        // matches live in captured logs), so it is a no-op
        // pass-through here — dump + backup already happened.
        let mut changed = false;
        if let Some(target_code) = target_code {
            match detected {
                Some(ref old_code) => {
                    live!(
                        log,
                        "[Country] {}",
                        tr_args!(
                            "live_country_patch_transition",
                            label = label,
                            from = old_code,
                            to = target_code
                        )
                    );
                    match ltbox_patch::region::patch_country_code(
                        &dump_path,
                        &patched_path,
                        old_code,
                        target_code,
                        EU_CODES,
                        is_field_only_country_partition(label),
                    ) {
                        Ok(c) => changed |= c,
                        Err(e) => {
                            let reason = tr_args!("country_reason_patch_failed", error = e);
                            ltbox_core::live!(
                                log,
                                "[Country] {}",
                                tr_args!(
                                    "live_country_partition_status",
                                    label = label,
                                    reason = reason
                                )
                            );
                            country_progress.mark_failed(label, reason);
                            continue;
                        }
                    }
                }
                None => {
                    if label != "persist" {
                        let reason = ltbox_core::i18n::tr("country_reason_no_known_code");
                        ltbox_core::live!(
                            log,
                            "[Country] {}",
                            tr_args!(
                                "live_country_partition_status",
                                label = label,
                                reason = reason
                            )
                        );
                        country_progress.mark_failed(label, reason);
                        continue;
                    }
                    // persist has no country code — nothing to patch;
                    // `changed` stays false and it is marked handled below.
                }
            }
        }

        if flip_proinfo_channel && label == "proinfo" {
            let channel_input = if target_code.is_some() {
                patched_path.as_path()
            } else {
                dump_path.as_path()
            };
            match ltbox_patch::region::patch_proinfo_channel(channel_input, &patched_path) {
                Ok(channel_changed) => {
                    changed |= channel_changed;
                    ltbox_core::live!(
                        log,
                        "[Country] {}",
                        ltbox_core::i18n::tr(if channel_changed {
                            "live_proinfo_channel_transition"
                        } else {
                            "live_proinfo_channel_already"
                        })
                    );
                }
                Err(e) => {
                    let reason = tr_args!("country_reason_channel_patch_failed", error = e);
                    ltbox_core::live!(
                        log,
                        "[Country] {}",
                        tr_args!(
                            "live_country_partition_status",
                            label = label,
                            reason = reason
                        )
                    );
                    country_progress.mark_failed(label, reason);
                    continue;
                }
            }
        }

        // Flash once if the country code changed.
        if changed {
            if let Some(phases) = phases {
                phases.mark_writes_started();
            }
            if let Err(e) = session.flash_partition(label, &patched_path, 0, lun, log) {
                ltbox_core::live!(
                    log,
                    "[Country] {}",
                    tr_args!(
                        "live_country_flash_failed",
                        label = label,
                        error = e.to_string()
                    )
                );
                country_progress
                    .mark_failed(label, tr_args!("country_reason_flash_failed", error = e));
            } else {
                live!(
                    log,
                    "[Country] {}",
                    tr_args!("live_country_patched_flashed", label = label)
                );
                country_progress.mark_flashed(label);
            }
        } else if target_code.is_some_and(|target| detected.as_deref() == Some(target)) {
            ltbox_core::live!(
                log,
                "[Country] {}",
                tr_args!(
                    "live_country_partition_already",
                    label = label,
                    target = target_code.unwrap_or_default()
                )
            );
            country_progress.mark_flashed(label);
        } else if target_code.is_none() && flip_proinfo_channel && label == "proinfo" {
            country_progress.mark_flashed(label);
        } else if label == "persist" {
            // persist carries no country code — nothing to
            // patch; treat as handled so the run still
            // resets to system.
            country_progress.mark_flashed(label);
        } else {
            let reason = "no replacements";
            ltbox_core::live!(
                log,
                "[Country] {}",
                tr_args!(
                    "live_country_partition_status",
                    label = label,
                    reason = reason
                )
            );
            country_progress.mark_failed(label, reason);
        }
    }
    // Aggregate per-partition outcome. The caller decides severity: the
    // post-flash phase treats it as best-effort (warn + still reset), the
    // standalone change-country op propagates it as the operation's result.
    let outcome = country_progress.finish();
    // Surface the backup location once per run. Empty dir = every label skipped.
    if std::fs::read_dir(critical_backup)
        .map(|mut it| it.next().is_some())
        .unwrap_or(false)
    {
        live!(
            log,
            "[Country] {} {}",
            ll.backup_saved_prefix,
            critical_backup.display()
        );
    }
    outcome
}

mod country;
mod full;
pub(crate) mod manual;
mod simple;

pub(crate) use country::change_country_worker;
pub(crate) use full::flash_worker;
pub(crate) use simple::simple_flash_worker;

#[cfg(test)]
mod tests {
    use super::{
        LenovoFirmwareDevicePolicy, ZSTD_FREE_SPACE_RESERVE_BYTES, lenovo_firmware_device_policy,
        require_firmware_loader, should_reboot_fastboot_to_system_after_pre_edl_abort,
        stream_zstd_decoder, zstd_output_limit,
    };

    #[test]
    fn country_partitions_select_by_model() {
        use super::country_partitions_for;
        // TB320FC / TB323FU keep the country code only in oemowninfo.
        assert_eq!(country_partitions_for("TB320FC", None), &["oemowninfo"][..]);
        assert_eq!(
            country_partitions_for("LAVIETab9QHD1", None),
            &["oemowninfo"][..]
        );
        assert_eq!(country_partitions_for("TB323FU", None), &["oemowninfo"][..]);
        assert_eq!(
            country_partitions_for("TB376FC", None),
            &["proinfo", "persist"][..]
        );
        assert_eq!(
            country_partitions_for("TB390FU", None),
            &["proinfo", "persist"][..]
        );
        // Every other model uses devinfo + persist.
        assert_eq!(
            country_partitions_for("TB330FU", None),
            &["devinfo", "persist"][..]
        );
        // The AVB fingerprint (EDL-start, no probed model) also selects it.
        assert_eq!(
            country_partitions_for("", Some("Lenovo/TB323FU/TB323FU:14/build")),
            &["oemowninfo"][..]
        );
        assert_eq!(
            country_partitions_for("", Some("qti/LAVIETab9QHD1/LAVIETab9QHD1:15/build_NEC")),
            &["oemowninfo"][..]
        );
        assert_eq!(
            country_partitions_for("", Some("Lenovo/TB390FU/TB390FU:15/build")),
            &["proinfo", "persist"][..]
        );
    }

    #[test]
    fn manual_mode_survives_tb323fu_but_never_enables_xiaoxin_index_edits() {
        use ltbox_patch::rollback::RollbackMode::{Auto, Manual, Off, On};
        assert_eq!(
            super::effective_flash_rollback_mode(Manual, true, false),
            Manual
        );
        assert_eq!(super::effective_flash_rollback_mode(On, true, false), Auto);
        assert_eq!(super::effective_flash_rollback_mode(Off, true, false), Off);
        for mode in [On, Auto, Manual, Off] {
            for model in ["TB376FC", "TB390FU"] {
                let restricted = super::xiaoxin_pro13_token(model).is_some();
                assert_eq!(
                    super::effective_flash_rollback_mode(mode, false, restricted),
                    Auto
                );
            }
        }
    }

    #[test]
    fn manual_floors_preserve_each_known_stored_minimum() {
        use ltbox_patch::rollback::{FastbootRollbackFloors, RollbackIndices};
        let fastboot = Some(FastbootRollbackFloors {
            boot_location: 3,
            boot_index: 100,
            vbmeta_system_location: 2,
            vbmeta_system_index: 20,
        });
        assert_eq!(
            super::manual_device_floors(None, fastboot, Some(100)),
            Some(RollbackIndices {
                boot: 100,
                vbmeta_system: 20
            })
        );
        assert_eq!(
            super::manual_device_floors(Some((90, 30)), fastboot, Some(100)),
            Some(RollbackIndices {
                boot: 100,
                vbmeta_system: 30
            })
        );
        assert_eq!(super::manual_device_floors(None, None, None), None);
        assert_eq!(
            super::manual_device_floors(None, None, Some(99)),
            Some(RollbackIndices {
                boot: 99,
                vbmeta_system: 99
            })
        );
        assert_eq!(
            super::manual_device_floors(Some((0, 0)), None, None),
            Some(RollbackIndices {
                boot: 0,
                vbmeta_system: 0
            })
        );
    }

    #[test]
    fn supported_models_include_both_xiaoxin_pro13_tokens() {
        assert!(super::SUPPORTED_MODELS.contains(&"TB376FC"));
        assert!(super::SUPPORTED_MODELS.contains(&"TB390FU"));
    }

    #[test]
    fn xiaoxin_channel_flip_requires_different_pair_tokens() {
        use super::{supported_model_identity_match, xiaoxin_pro13_cross_model};
        assert!(xiaoxin_pro13_cross_model(
            "TB376FC",
            Some("Lenovo/TB390FU/TB390FU:15/build")
        ));
        assert!(xiaoxin_pro13_cross_model(
            "TB390FU",
            Some("Lenovo/TB376FC_PRC/TB376FC:15/build")
        ));
        assert!(!xiaoxin_pro13_cross_model(
            "TB376FC",
            Some("Lenovo/TB376FC_PRC/TB376FC:15/build")
        ));
        assert!(!xiaoxin_pro13_cross_model(
            "TB390FU",
            Some("Lenovo/TB390FU/TB390FU:15/build")
        ));
        assert!(supported_model_identity_match(
            "Lenovo/TB390FU/TB390FU:15/build",
            "TB390FU"
        ));
        assert!(!supported_model_identity_match(
            "Lenovo/TB390FU/TB390FU:15/build",
            "TB376FC"
        ));
    }

    #[test]
    fn pre_edl_abort_reboots_when_flash_flow_entered_fastboot() {
        assert!(should_reboot_fastboot_to_system_after_pre_edl_abort(false));
    }

    #[test]
    fn pre_edl_abort_keeps_fastboot_when_flash_started_in_fastboot() {
        assert!(!should_reboot_fastboot_to_system_after_pre_edl_abort(true));
    }

    #[test]
    fn missing_firmware_loader_is_an_error() {
        assert!(require_firmware_loader(None).is_err());
    }

    #[test]
    fn zstd_output_limit_preserves_the_disk_reserve() {
        let free = ZSTD_FREE_SPACE_RESERVE_BYTES * 3;
        assert_eq!(
            zstd_output_limit(free).unwrap(),
            free - ZSTD_FREE_SPACE_RESERVE_BYTES
        );
        assert!(zstd_output_limit(ZSTD_FREE_SPACE_RESERVE_BYTES).is_err());
    }

    #[test]
    fn zstd_stream_rejects_an_output_larger_than_its_limit() {
        let original = vec![0xAB; 1024];
        let compressed = zstd::stream::encode_all(&original[..], 1).unwrap();
        let decoder = zstd::stream::Decoder::new(&compressed[..]).unwrap();
        let mut log = Vec::new();

        let err = stream_zstd_decoder(decoder, Vec::new(), "test.img.zst", &mut log, 1023)
            .expect_err("an over-limit decompression must fail");

        assert!(err.contains("output limit"), "got: {err}");
    }

    #[test]
    fn edl_start_rollback_floors_are_component_wise() {
        use super::rollback_floors;
        // Per-location max across slots: _a=(boot 10, vbs 1), _b=(boot 9, vbs 8)
        // -> (10, 8); a single-slot pick would have missed vbs 8.
        assert_eq!(
            rollback_floors([Some(10), Some(9)], [Some(1), Some(8)]),
            Some((10, 8))
        );
        // A location that parsed on only one slot uses that slot's value.
        assert_eq!(
            rollback_floors([Some(7), None], [None, Some(3)]),
            Some((7, 3))
        );
        // A location that parsed on neither slot -> abort.
        assert_eq!(rollback_floors([None, None], [Some(5), Some(6)]), None);
        assert_eq!(rollback_floors([Some(5), Some(6)], [None, None]), None);
        assert_eq!(rollback_floors([None, None], [None, None]), None);
    }

    #[test]
    fn lenovo_firmware_policy_uses_device_key_unless_user_abl_is_supplied() {
        let rsa2048 =
            ltbox_patch::key_map::classify_pubkey(Some("cdbb77177f731920bbe0a0f94f84d9038ae0617d"));
        let rsa4096 =
            ltbox_patch::key_map::classify_pubkey(Some("2597c218aae470a130f61162feaae70afd97f011"));
        let lenovo =
            ltbox_patch::key_map::classify_pubkey(Some("8fcb864f11f53ed11284615fb67685522085d3a2"));

        assert_eq!(
            lenovo_firmware_device_policy(rsa2048, false),
            LenovoFirmwareDevicePolicy::ResignTestkey
        );
        assert_eq!(
            lenovo_firmware_device_policy(rsa4096, false),
            LenovoFirmwareDevicePolicy::ResignTestkey
        );
        assert_eq!(
            lenovo_firmware_device_policy(lenovo, false),
            LenovoFirmwareDevicePolicy::KeepLenovo
        );
        assert_eq!(
            lenovo_firmware_device_policy(ltbox_patch::key_map::KeyClass::Unknown, false),
            LenovoFirmwareDevicePolicy::AbortUnknown
        );
        assert_eq!(
            lenovo_firmware_device_policy(ltbox_patch::key_map::KeyClass::Lenovo, true),
            LenovoFirmwareDevicePolicy::ResignTestkey
        );
        assert_eq!(
            lenovo_firmware_device_policy(ltbox_patch::key_map::KeyClass::Unknown, true),
            LenovoFirmwareDevicePolicy::ResignTestkey
        );
    }
}
