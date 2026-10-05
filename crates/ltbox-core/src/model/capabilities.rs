//! Shared behavior profiles, independent of device and firmware validation.
//!
//! The generic profile is the fallback for unrecognized model strings; it does
//! not whitelist unknown physical devices. Connection and firmware
//! compatibility checks still apply.

use super::{LAVIE_TAB_9QHD1_MODEL, token_match};

/// Models in the existing GUI and firmware resolver order.
pub const SUPPORTED_MODELS: [&str; 10] = [
    "TB320FC", "TB321FU", "TB322FC", "TB323FU", "TB324ZC", "TB376FC", "TB390FU", "TB391FC",
    "TB520FU", "TB710FU",
];

/// Rollback protection and supported rollback-index operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RollbackPolicy {
    /// Standard rollback protection.
    Standard,
    /// Device does not enforce rollback protection.
    Unprotected,
    /// Rollback protection uses the GBL path.
    Gbl,
    /// Rollback information can be inspected but indices cannot be edited.
    ReadOnly,
}

impl RollbackPolicy {
    /// Whether rollback protection must be respected.
    pub const fn is_protected(self) -> bool {
        !matches!(self, Self::Unprotected)
    }

    /// Whether the profile permits rollback-index editing.
    pub const fn permits_index_edit(self) -> bool {
        !matches!(self, Self::ReadOnly)
    }
}

/// Model-specific operation availability and boot behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelCapabilities {
    /// Supports the root workflow.
    pub root: bool,
    /// Supports GKI rooting.
    pub gki_root: bool,
    /// Supports restoring stock root partitions.
    pub unroot: bool,
    /// Supports KonaBess patching.
    pub konabess: bool,
    /// Supports rescue operations.
    pub rescue: bool,
    /// Ramdisk rooting patches boot rather than init_boot.
    pub ramdisk_root_uses_boot: bool,
    /// The boot VBMeta descriptor uses a hash.
    pub boot_vbmeta_is_hash: bool,
    /// Rooting uses GBL.
    pub root_uses_gbl: bool,
    /// Sahara flashing requires a manifest.
    pub requires_sahara_manifest: bool,
    /// Hardware has two USB ports.
    pub dual_usb: bool,
    /// Firmware is PRC-only.
    pub prc_only: bool,
    /// Supports AVB conversion between firmware regions.
    pub region_avb_conversion: bool,
    /// The bootloader reports its committed rollback floors through
    /// `fastboot getvar`. Where it does not, the floor has to be read by
    /// dumping AVB metadata over EDL, which needs a programmer.
    pub rollback_floor_via_fastboot: bool,
    /// Rollback protection and editing policy.
    pub rollback: RollbackPolicy,
    /// Partitions that carry the device country code, in dump/patch order.
    pub country_partitions: &'static [&'static str],
}

const GENERIC: ModelCapabilities = ModelCapabilities {
    root: true,
    gki_root: true,
    unroot: true,
    konabess: true,
    rescue: true,
    ramdisk_root_uses_boot: false,
    boot_vbmeta_is_hash: false,
    root_uses_gbl: false,
    requires_sahara_manifest: false,
    dual_usb: false,
    prc_only: false,
    region_avb_conversion: true,
    rollback_floor_via_fastboot: false,
    rollback: RollbackPolicy::Standard,
    country_partitions: &["devinfo", "persist"],
};

const TB320FC: ModelCapabilities = ModelCapabilities {
    ramdisk_root_uses_boot: true,
    boot_vbmeta_is_hash: true,
    dual_usb: true,
    country_partitions: &["oemowninfo", "persist"],
    ..GENERIC
};
const TB321FU: ModelCapabilities = ModelCapabilities {
    dual_usb: true,
    rollback_floor_via_fastboot: true,
    ..GENERIC
};
/// TB520FU behaves like the generic profile except that its bootloader
/// reports rollback floors over fastboot, so no EDL loader is needed to read
/// them.
const TB520FU: ModelCapabilities = ModelCapabilities {
    rollback_floor_via_fastboot: true,
    ..GENERIC
};
const TB322FC: ModelCapabilities = ModelCapabilities {
    dual_usb: true,
    prc_only: true,
    rollback: RollbackPolicy::Unprotected,
    ..GENERIC
};
const TB323FU: ModelCapabilities = ModelCapabilities {
    rescue: false,
    root_uses_gbl: true,
    requires_sahara_manifest: true,
    dual_usb: true,
    region_avb_conversion: false,
    rollback: RollbackPolicy::Gbl,
    country_partitions: &["oemowninfo", "persist"],
    ..GENERIC
};
/// TB324ZC — Y700 5G. Shares TB323FU's efisp/GBL route, multi-image Sahara
/// manifest and dual USB-C ports, but ships PRC-only firmware.
const TB324ZC: ModelCapabilities = ModelCapabilities {
    rescue: false,
    root_uses_gbl: true,
    requires_sahara_manifest: true,
    dual_usb: true,
    prc_only: true,
    region_avb_conversion: false,
    rollback: RollbackPolicy::Gbl,
    country_partitions: &["proinfo", "persist"],
    ..GENERIC
};
const XIAOXIN_PRO13: ModelCapabilities = ModelCapabilities {
    root: false,
    gki_root: false,
    unroot: false,
    konabess: false,
    rescue: false,
    region_avb_conversion: false,
    rollback: RollbackPolicy::ReadOnly,
    country_partitions: &["proinfo", "persist"],
    ..GENERIC
};

/// Device codenames that custom ROMs put in build fingerprints in place of the
/// model token, as `(codename, canonical model)`. A ROM built for TB323FU
/// reports `lineage_baldur/baldur` rather than `TB323FU`, so the model token
/// alone cannot identify it. Matching uses the same alphanumeric-boundary rule
/// as the model tokens.
const CODENAMES: [(&str, &str); 7] = [
    ("asphalt", "TB320FC"),
    ("kirby", "TB321FU"),
    ("elden", "TB322FC"),
    ("baldur", "TB323FU"),
    ("wuji", "TB324ZC"),
    ("lapis", "TB520FU"),
    ("topaz", "TB710FU"),
];

const PROFILES: [(&str, &ModelCapabilities); 11] = [
    (SUPPORTED_MODELS[0], &TB320FC),
    (SUPPORTED_MODELS[1], &TB321FU),
    (SUPPORTED_MODELS[2], &TB322FC),
    (SUPPORTED_MODELS[3], &TB323FU),
    (SUPPORTED_MODELS[4], &TB324ZC),
    (SUPPORTED_MODELS[5], &XIAOXIN_PRO13),
    (SUPPORTED_MODELS[6], &XIAOXIN_PRO13),
    (SUPPORTED_MODELS[7], &XIAOXIN_PRO13),
    (SUPPORTED_MODELS[8], &TB520FU),
    (SUPPORTED_MODELS[9], &GENERIC),
    (LAVIE_TAB_9QHD1_MODEL, &TB320FC),
];

/// Whether a fingerprint names the codename of model `model`.
pub(super) fn codename_names_model(fp: &str, model: &str) -> bool {
    CODENAMES
        .iter()
        .any(|(codename, canonical)| *canonical == model && token_match(fp, codename))
}

/// Whether a fingerprint names a profile, by model token or by codename.
fn names_profile(fp: &str, name: &str) -> bool {
    token_match(fp, name) || codename_names_model(fp, name)
}

/// Canonical name of a known model, ignoring ASCII case and spaces (so a
/// `ro.product.model` read as `TB 320FC` resolves). `None` for an empty or
/// unrecognized name; unlike [`capabilities`] there is no generic fallback.
pub fn known_model(name: &str) -> Option<&'static str> {
    let name = name.replace(' ', "");
    if name.is_empty() {
        return None;
    }
    PROFILES
        .iter()
        .find(|(profile, _)| profile.eq_ignore_ascii_case(&name))
        .map(|(profile, _)| *profile)
}

/// Whether a fingerprint names at least one known model, by token or codename.
pub fn fingerprint_names_known_model(fp: &str) -> bool {
    fingerprint_models(fp).next().is_some()
}

/// Resolve an exact model name, ignoring ASCII case, or use the generic profile.
///
/// This fallback does not authorize an unknown physical device for operations.
pub fn capabilities(model: &str) -> &'static ModelCapabilities {
    PROFILES
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(model))
        .map_or(&GENERIC, |(_, profile)| *profile)
}

/// Resolve a case-sensitive model token with alphanumeric boundaries.
///
/// If multiple tokens occur, the first entry in [`SUPPORTED_MODELS`] wins, followed
/// by the explicit LAVIE token. Firmware cross-SKU aliases are not applied.
pub fn capabilities_from_fingerprint(fp: &str) -> Option<&'static ModelCapabilities> {
    fingerprint_capabilities(fp).next()
}

/// All explicitly named model profiles in a fingerprint. Safety restrictions
/// must consider every token when a malformed/mixed fingerprint names multiple
/// devices; a permissive token must not hide a restricted one.
pub fn fingerprint_capabilities(fp: &str) -> impl Iterator<Item = &'static ModelCapabilities> + '_ {
    PROFILES
        .iter()
        .filter(move |(name, _)| names_profile(fp, name))
        .map(|(_, profile)| *profile)
}

/// Model names explicitly named in a fingerprint (canonical names, never
/// codenames), in [`fingerprint_capabilities`] order. Use these to name the
/// model in a message instead of inferring it from a profile's fields.
pub fn fingerprint_models(fp: &str) -> impl Iterator<Item = &'static str> + '_ {
    PROFILES
        .iter()
        .filter(move |(name, _)| names_profile(fp, name))
        .map(|(name, _)| *name)
}

/// First model named in a fingerprint whose profile does not satisfy
/// `supported`, for refusing an image and naming the model it was built for.
pub fn fingerprint_model_lacking(
    fp: &str,
    supported: impl Fn(&ModelCapabilities) -> bool,
) -> Option<&'static str> {
    fingerprint_models(fp).find(|model| !supported(capabilities(model)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// TB324ZC shares TB323FU's exploit route and its dual USB-C hardware but
    /// not its region, so it is pinned against that profile rather than
    /// described twice.
    #[test]
    fn tb324zc_shares_the_gbl_route_and_dual_usb_but_is_prc_only() {
        let tb324zc = capabilities("TB324ZC");
        let tb323fu = capabilities("TB323FU");

        assert!(tb324zc.root_uses_gbl);
        assert!(tb324zc.requires_sahara_manifest);
        assert_eq!(tb324zc.rollback, RollbackPolicy::Gbl);
        assert!(tb324zc.rollback.is_protected());
        assert!(tb324zc.gki_root);
        assert!(!tb324zc.rescue);
        assert!(!tb324zc.region_avb_conversion);

        // Dual USB-C is shared; the region is what separates it from TB323FU.
        assert!(tb324zc.dual_usb && tb323fu.dual_usb);
        assert!(tb324zc.prc_only && !tb323fu.prc_only);

        assert_eq!(
            capabilities_from_fingerprint("qti/TB324ZC/TB324ZC:16/build:user/release-keys"),
            Some(tb324zc)
        );
    }

    #[test]
    fn fingerprint_models_name_each_exact_token() {
        let names: Vec<_> =
            fingerprint_models("qti/TB324ZC/TB324ZC:16/build:user/release-keys").collect();
        assert_eq!(names, ["TB324ZC"]);
        let names: Vec<_> = fingerprint_models("Lenovo/TB390FU/TB390FU:15/build").collect();
        assert_eq!(names, ["TB390FU"]);
        assert_eq!(fingerprint_models("Lenovo/TB324ZCextra/x:15").count(), 0);
    }

    const BALDUR_FP: &str =
        "Lenovo/lineage_baldur/baldur:17/CP2A.260605.016/eng.androi:userdebug/test-keys";

    #[test]
    fn codename_fingerprint_resolves_to_its_model() {
        assert_eq!(capabilities_from_fingerprint(BALDUR_FP), Some(&TB323FU));
        let names: Vec<_> = fingerprint_models(BALDUR_FP).collect();
        assert_eq!(names, ["TB323FU"]);
        // Token and codename together yield the model once.
        let both = "Lenovo/TB323FU/baldur:17/build";
        assert_eq!(fingerprint_models(both).collect::<Vec<_>>(), ["TB323FU"]);
        assert_eq!(fingerprint_capabilities(both).count(), 1);
        assert_eq!(
            fingerprint_model_lacking(BALDUR_FP, |c| c.rescue),
            Some("TB323FU")
        );
        assert_eq!(fingerprint_model_lacking(BALDUR_FP, |c| c.root), None);
    }

    #[test]
    fn every_codename_resolves_to_its_own_profile() {
        for (codename, model) in CODENAMES {
            let fp = format!("Lenovo/lineage_{codename}/{codename}:17/build:userdebug/test-keys");
            assert_eq!(fingerprint_models(&fp).collect::<Vec<_>>(), [model], "{fp}");
            assert_eq!(
                capabilities_from_fingerprint(&fp),
                Some(capabilities(model)),
                "{fp}"
            );
            assert_eq!(known_model(model), Some(model), "{model}");
        }
    }

    #[test]
    fn codenames_keep_alphanumeric_boundaries() {
        for fp in [
            "Lenovo/baldurx/baldurx:17/build",
            "Lenovo/xbaldur/xbaldur:17/build",
            "Lenovo/Baldur/Baldur:17/build",
        ] {
            assert_eq!(capabilities_from_fingerprint(fp), None, "{fp}");
            assert!(!fingerprint_names_known_model(fp), "{fp}");
        }
    }

    #[test]
    fn fingerprint_names_known_model_covers_tokens_and_codenames() {
        assert!(fingerprint_names_known_model(
            "Lenovo/TB323FU/TB323FU:16/build"
        ));
        assert!(fingerprint_names_known_model(BALDUR_FP));
        assert!(!fingerprint_names_known_model(
            "Lenovo/lineage_foo/foo:17/CP2A/eng:userdebug/test-keys"
        ));
        assert!(!fingerprint_names_known_model(""));
    }

    #[test]
    fn known_model_resolves_canonical_names_only() {
        assert_eq!(known_model("tb323fu"), Some("TB323FU"));
        assert_eq!(known_model("TB323FU"), Some("TB323FU"));
        assert_eq!(known_model("lavietab9qhd1"), Some("LAVIETab9QHD1"));
        assert_eq!(known_model("LAVIE Tab 9QHD1"), Some("LAVIETab9QHD1"));
        assert_eq!(known_model(" "), None);
        assert_eq!(known_model(""), None);
        assert_eq!(known_model("Legion Y700"), None);
        assert_eq!(known_model("baldur"), None);
    }

    #[test]
    fn fingerprint_model_lacking_names_the_restricted_token() {
        let xiaoxin = "Lenovo/TB390FU/TB390FU:15/build";
        assert_eq!(
            fingerprint_model_lacking(xiaoxin, |c| c.root),
            Some("TB390FU")
        );
        assert_eq!(
            fingerprint_model_lacking(xiaoxin, |c| c.rescue),
            Some("TB390FU")
        );
        let tb323fu = "Lenovo/TB323FU/TB323FU:14/build";
        assert_eq!(fingerprint_model_lacking(tb323fu, |c| c.root), None);
        assert_eq!(
            fingerprint_model_lacking(tb323fu, |c| c.rescue),
            Some("TB323FU")
        );
        // A permissive token must not hide a restricted one.
        let mixed = "Lenovo/TB320FC/TB376FC:15/build";
        assert_eq!(
            fingerprint_model_lacking(mixed, |c| c.root),
            Some("TB376FC")
        );
        assert_eq!(fingerprint_model_lacking("unknown", |c| c.root), None);
    }

    #[test]
    fn exact_model_names_accept_case_and_lavie_but_reject_suffixes() {
        for model in ["TB320FC", "tb320fc", "LAVIETab9QHD1", "lavietab9qhd1"] {
            assert_eq!(capabilities(model), &TB320FC, "{model}");
        }
        for model in [
            "",
            "Generic",
            "unknown",
            "TB320FCX",
            " TB320FC",
            "TB376FC_ROW",
        ] {
            assert_eq!(capabilities(model), &GENERIC, "{model}");
        }
    }

    #[test]
    fn special_models_restrict_operations_and_select_boot_paths() {
        for model in ["TB376FC", "TB390FU", "TB391FC", "tb390fu", "tb391fc"] {
            let profile = capabilities(model);
            assert!(!profile.root && !profile.gki_root && !profile.unroot);
            assert!(!profile.konabess && !profile.rescue && !profile.region_avb_conversion);
            assert!(!profile.rollback.permits_index_edit());
        }
        let gbl = capabilities("TB323FU");
        assert!(gbl.root && gbl.unroot && gbl.konabess && gbl.dual_usb);
        assert!(gbl.root_uses_gbl && gbl.requires_sahara_manifest);
        assert!(gbl.gki_root && !gbl.rescue && !gbl.region_avb_conversion);
        assert_eq!(gbl.rollback, RollbackPolicy::Gbl);
        for model in ["TB320FC", LAVIE_TAB_9QHD1_MODEL] {
            let profile = capabilities(model);
            assert!(profile.ramdisk_root_uses_boot && profile.boot_vbmeta_is_hash);
            assert!(profile.dual_usb);
        }
        assert!(capabilities("TB321FU").dual_usb);
        assert!(capabilities("TB322FC").prc_only);
        assert!(!capabilities("TB322FC").rollback.is_protected());
        for model in ["TB710FU", "unknown"] {
            assert_eq!(capabilities(model), &GENERIC);
        }
        // TB520FU is generic apart from reporting its rollback floors over
        // fastboot, so the dashboard can read them without an EDL loader.
        assert_eq!(
            capabilities("TB520FU"),
            &ModelCapabilities {
                rollback_floor_via_fastboot: true,
                ..GENERIC
            }
        );
    }

    #[test]
    fn only_the_two_fastboot_reporting_models_skip_the_edl_floor_read() {
        let via_fastboot: Vec<&str> = SUPPORTED_MODELS
            .into_iter()
            .chain([LAVIE_TAB_9QHD1_MODEL])
            .filter(|model| capabilities(model).rollback_floor_via_fastboot)
            .collect();
        assert_eq!(via_fastboot, ["TB321FU", "TB520FU"]);
        // An unrecognised device must fall back to the EDL read rather than
        // trust a fastboot variable it may not publish.
        assert!(!capabilities("unknown").rollback_floor_via_fastboot);
    }

    #[test]
    fn rollback_policy_keeps_protection_separate_from_editing() {
        for (policy, protected, editable) in [
            (RollbackPolicy::Standard, true, true),
            (RollbackPolicy::Unprotected, false, true),
            (RollbackPolicy::Gbl, true, true),
            (RollbackPolicy::ReadOnly, true, false),
        ] {
            assert_eq!(policy.is_protected(), protected);
            assert_eq!(policy.permits_index_edit(), editable);
        }
    }

    #[test]
    fn fingerprint_lookup_requires_exact_case_sensitive_tokens() {
        for (model, profile) in PROFILES {
            let fingerprint = format!("vendor/{model}_ROW/{model}:15/build");
            assert_eq!(capabilities_from_fingerprint(&fingerprint), Some(profile));
            for invalid in [
                format!("vendor/{model}X/build"),
                format!("vendor/X{model}/build"),
                model.to_ascii_lowercase(),
            ] {
                assert_eq!(capabilities_from_fingerprint(&invalid), None, "{invalid}");
            }
        }
        assert_eq!(capabilities_from_fingerprint("vendor/unknown/build"), None);
    }

    #[test]
    fn mixed_fingerprints_keep_all_restrictions_visible() {
        let fp = "vendor/TB320FC/TB323FU/TB390FU:15/build";
        assert!(fingerprint_capabilities(fp).any(|p| !p.root));
        assert!(fingerprint_capabilities(fp).any(|p| p.root_uses_gbl));
        assert!(fingerprint_capabilities(fp).any(|p| p.rollback == RollbackPolicy::ReadOnly));
    }

    #[test]
    fn fingerprint_priority_uses_table_order_without_cross_sku_aliases() {
        assert_eq!(
            capabilities_from_fingerprint("TB323FU/TB322FC"),
            Some(&TB322FC)
        );
        assert_eq!(
            capabilities_from_fingerprint("LAVIETab9QHD1/TB323FU"),
            Some(&TB323FU)
        );
        assert_eq!(
            capabilities_from_fingerprint("TB320FCX/TB323FU"),
            Some(&TB323FU)
        );
    }
}
