//! Per-partition UFS LUN map for supported Lenovo Qualcomm tablets.
//!
//! LUN is a hardware property: the mapping holds across every firmware rev on
//! the same model. The table below was built by decrypting + comparing the
//! rawprogram catalogs of all six supported models — TB320FC / TB321FU /
//! TB322FC / TB323FU / TB520FU / TB710FU (`physical_partition_number` per
//! `label`). Every partition that appears on more than one of those models sits
//! on the SAME LUN on each (verified: zero conflicts), so the map is
//! model-agnostic — there is no per-model LUN override. Partitions present on
//! only some models (e.g. `init_boot`, `efisp`, `oemowninfo`) keep that same
//! consistent LUN and are listed too.
//!
//! A static hit lets ARB / country-code / unroot / root skip the rawprogram
//! catalog scan + `.x` decrypt — qdl-rs resolves start/length from the device
//! GPT once given LUN + name. A label NOT in the map (a partition unique to a
//! future model / layout not in that comparison) returns `None`; the caller
//! falls back to a device GPT scan (`EdlSession::lun_for`).
//!
//! Multi-LUN labels are deliberately omitted (they cannot map to one LUN):
//! `primarygpt` / `backupgpt` (a GPT table per LUN), `last_parti` (an end
//! marker per LUN), and the bootloader copies duplicated on LUN 1 + 2
//! (`xbl`, `xbl_config`, `multiimgqti`, `multiimgoem`, `tme_*`, `xbl_ac_config`).

/// UFS LUN for a partition LTBox may operate on individually. Slot suffixes
/// (`_a`/`_b`) and case are normalised. Returns `None` for labels not in the
/// static table — the caller falls back to a device GPT scan.
pub fn lun_for_partition(label: &str) -> Option<u8> {
    let base = strip_slot_suffix(label).to_ascii_lowercase();
    let lun = match base.as_str() {
        // LUN 0 — userdata / region / persist storage.
        "dataext" | "frp" | "keystore" | "lenovocust" | "lenovolock" | "lenovoraw" | "metadata"
        | "misc" | "oemowninfo" | "persist" | "proinfo" | "pstoredump" | "rawdump" | "ssd"
        | "super" | "userdata" | "vbmeta_system" => 0,

        // LUN 1 / 2 — single-copy DPP pair (the redundant bootloader images on
        // these LUNs are multi-LUN and intentionally excluded above).
        "apdp" => 1,
        "apdpb" => 2,

        // LUN 3 — board config.
        "align_to_128k_1" | "cdt" | "ddr" => 3,

        // LUN 4 — boot-related and AVB images, firmware, vendor.
        "abl" | "aop" | "aop_config" | "bluetooth" | "boot" | "connsec" | "cpucp" | "cpucp_dtb"
        | "dcp" | "devcfg" | "devinfo" | "dip" | "dpm" | "dsp" | "dtbo" | "efisp"
        | "featenabler" | "hyp" | "hyp_ac_config" | "imagefv" | "init_boot" | "keymaster"
        | "limits" | "limits-cdsp" | "logdump" | "logfs" | "mdcompress" | "mdtp" | "mdtpsecapp"
        | "modem" | "pdp" | "pdp_cdb" | "pvmfw" | "pvmfw_signed" | "qmcs" | "qtvm_dtbo"
        | "qupfw" | "qweslicstore" | "recovery" | "rtice" | "secdata" | "secretkeeper" | "shrm"
        | "soccp" | "soccp_dcd" | "soccp_debug" | "splash" | "spunvm" | "spuservice"
        | "storsec" | "toolsfv" | "tz" | "tz_ac_config" | "tz_qti_config" | "tzsc" | "uefi"
        | "uefisecapp" | "uefivarstore" | "vbmeta" | "vendor_boot" | "vm-bootsys" | "vm-data"
        | "vm-persist" | "xbl_ramdump" | "xbl_sc_logs" | "xbl_sc_test_mode" => 4,

        // LUN 5 — modem filesystem.
        "align_to_128k_2" | "fs_bkup" | "fsc" | "fsg" | "modemst1" | "modemst2" => 5,

        _ => return None,
    };
    Some(lun)
}

/// Strip the trailing `_a` / `_b` slot suffix if present.
pub fn strip_slot_suffix(label: &str) -> &str {
    // Byte-slicing the last two bytes panicked on a non-ASCII tail (GPT names
    // are untrusted); `strip_suffix` only cuts at a matched ASCII suffix.
    ["_a", "_A", "_b", "_B"]
        .iter()
        .find_map(|suffix| label.strip_suffix(suffix))
        .unwrap_or(label)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_slot_suffix_handles_case_and_non_ascii_tails() {
        assert_eq!(strip_slot_suffix("boot_a"), "boot");
        assert_eq!(strip_slot_suffix("boot_B"), "boot");
        assert_eq!(strip_slot_suffix("boot"), "boot");
        assert_eq!(strip_slot_suffix("_a"), "");
        assert_eq!(strip_slot_suffix("a"), "a");
        assert_eq!(strip_slot_suffix("한"), "한");
        assert_eq!(strip_slot_suffix("boot_é"), "boot_é");
    }

    #[test]
    fn lun_0_partitions_resolve() {
        for name in [
            "persist",
            "frp",
            "userdata",
            "metadata",
            "vbmeta_system",
            "super",
            "oemowninfo",
            "misc",
        ] {
            assert_eq!(lun_for_partition(name), Some(0), "{name}");
        }
    }

    #[test]
    fn lun_4_partitions_resolve() {
        for name in [
            "boot",
            "init_boot",
            "vbmeta",
            "vendor_boot",
            "devinfo",
            "dtbo",
            "efisp",
            "recovery",
            "abl",
            "modem",
            "vm-bootsys",
        ] {
            assert_eq!(lun_for_partition(name), Some(4), "{name}");
        }
    }

    #[test]
    fn dpp_and_modemfs_luns_resolve() {
        assert_eq!(lun_for_partition("apdp"), Some(1));
        assert_eq!(lun_for_partition("apdpb"), Some(2));
        assert_eq!(lun_for_partition("cdt"), Some(3));
        assert_eq!(lun_for_partition("modemst1"), Some(5));
        assert_eq!(lun_for_partition("fsg"), Some(5));
    }

    #[test]
    fn slot_suffix_strips() {
        assert_eq!(lun_for_partition("vbmeta_a"), Some(4));
        assert_eq!(lun_for_partition("vbmeta_b"), Some(4));
        assert_eq!(lun_for_partition("vbmeta_system_a"), Some(0));
        assert_eq!(lun_for_partition("boot_a"), Some(4));
        assert_eq!(lun_for_partition("init_boot_b"), Some(4));
    }

    #[test]
    fn case_insensitive() {
        assert_eq!(lun_for_partition("VBMETA_A"), Some(4));
        assert_eq!(lun_for_partition("Persist"), Some(0));
    }

    #[test]
    fn unknown_returns_none() {
        // Multi-LUN labels are intentionally absent (caller GPT-scans).
        assert_eq!(lun_for_partition("xbl"), None);
        assert_eq!(lun_for_partition("primarygpt"), None);
        assert_eq!(lun_for_partition("last_parti"), None);
        // Truly unknown.
        assert_eq!(lun_for_partition("nonexistent_part"), None);
        assert_eq!(lun_for_partition(""), None);
    }

    #[test]
    fn strip_slot_suffix_leaves_unsuffixed_alone() {
        assert_eq!(strip_slot_suffix("vbmeta"), "vbmeta");
        assert_eq!(strip_slot_suffix("a"), "a"); // too short to match
        assert_eq!(strip_slot_suffix(""), "");
    }
}
