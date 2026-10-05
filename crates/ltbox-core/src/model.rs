//! Device-model identity shared across LTBox crates.

mod capabilities;
pub use capabilities::{
    ModelCapabilities, RollbackPolicy, SUPPORTED_MODELS, capabilities,
    capabilities_from_fingerprint, fingerprint_capabilities, fingerprint_model_lacking,
    fingerprint_models, fingerprint_names_known_model, known_model,
};

/// Model token reported by Legion Tab Y700 (2023) firmware.
pub const TB320FC_MODEL: &str = "TB320FC";

/// Model token reported by LAVIE Tab 9QHD1 firmware.
pub const LAVIE_TAB_9QHD1_MODEL: &str = "LAVIETab9QHD1";

/// Model token reported by Lenovo Y700 5G firmware.
pub const TB324ZC_MODEL: &str = "TB324ZC";

/// Model token reported by Xiaoxin Pro 13 firmware.
pub const TB376FC_MODEL: &str = "TB376FC";

/// Model token reported by Idea Tab Pro Gen 2 firmware.
pub const TB390FU_MODEL: &str = "TB390FU";

/// Model token reported by Zhaoyang K13 firmware.
pub const TB391FC_MODEL: &str = "TB391FC";

/// The hardware-equivalent Xiaoxin Pro 13 / Idea Tab Pro Gen 2 / Zhaoyang K13
/// SKUs. Cross-SKU flashes and every model gate treat all members identically.
pub const XIAOXIN_PRO13_MODELS: [&str; 3] = [TB376FC_MODEL, TB390FU_MODEL, TB391FC_MODEL];

/// Whether `model` follows the TB320FC hardware-specific paths.
///
/// LAVIE Tab 9QHD1 reports its domestic model token despite using the same
/// hardware behavior, so it must inherit every TB320FC-only gate.
pub fn is_tb320fc_model(model: &str) -> bool {
    model.eq_ignore_ascii_case(TB320FC_MODEL) || model.eq_ignore_ascii_case(LAVIE_TAB_9QHD1_MODEL)
}

/// Whether `model` is one of the hardware-equivalent Xiaoxin Pro 13 /
/// Idea Tab Pro Gen 2 / Zhaoyang K13 SKUs.
pub fn is_xiaoxin_pro13_model(model: &str) -> bool {
    XIAOXIN_PRO13_MODELS
        .iter()
        .any(|m| model.eq_ignore_ascii_case(m))
}

/// Match a model token inside a fingerprint or probe string.
///
/// Matches keep alphanumeric word boundaries so a future suffixed model cannot
/// collide. The TB320FC ↔ LAVIE Tab 9QHD1 pair and the Xiaoxin Pro 13 SKU
/// group are the equivalences handled here. A fingerprint also names a model
/// through the codename a custom ROM uses in its place (`baldur` → TB323FU;
/// the full table is `CODENAMES` in `capabilities.rs`).
pub fn fingerprint_model_match(haystack: &str, model: &str) -> bool {
    let names =
        |m: &str| token_match(haystack, m) || capabilities::codename_names_model(haystack, m);
    if names(model) {
        return true;
    }

    if model == TB320FC_MODEL {
        names(LAVIE_TAB_9QHD1_MODEL)
    } else if model == LAVIE_TAB_9QHD1_MODEL {
        names(TB320FC_MODEL)
    } else if XIAOXIN_PRO13_MODELS.contains(&model) {
        XIAOXIN_PRO13_MODELS.iter().any(|m| names(m))
    } else {
        false
    }
}

fn token_match(haystack: &str, model: &str) -> bool {
    if model.is_empty() {
        return false;
    }

    let bytes = haystack.as_bytes();
    let mut start = 0usize;
    while let Some(pos) = haystack[start..].find(model) {
        let absolute = start + pos;
        let before_ok = absolute == 0 || !bytes[absolute - 1].is_ascii_alphanumeric();
        let end = absolute + model.len();
        let after_ok = end == bytes.len() || !bytes[end].is_ascii_alphanumeric();
        if before_ok && after_ok {
            return true;
        }
        start = absolute + 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    const TB320FC_FINGERPRINT: &str =
        "qti/TB320FC/TB320FC:15/AQ3A.240812.002/ZUI_17.0.313_250808_ROW:user/release-keys";
    const LAVIE_TAB_9QHD1_FINGERPRINT: &str = "qti/LAVIETab9QHD1/LAVIETab9QHD1:15/\
         AQ3A.240812.002/S104127_260624_NEC:user/release-keys";
    const TB376FC_FINGERPRINT: &str =
        "Lenovo/TB376FC_PRC/TB376FC:15/build/TB376FC_CN_OPEN_USER:user/release-keys";
    const TB390FU_FINGERPRINT: &str =
        "Lenovo/TB390FU/TB390FU:15/build/TB390FU_ROW_OPEN_USER:user/release-keys";
    const TB391FC_FINGERPRINT: &str =
        "Lenovo/TB391FC_PRC/TB391FC:15/build/TB391FC_CN_OPEN_USER:user/release-keys";

    #[test]
    fn lavie_tab_9qhd1_device_accepts_tb320fc_firmware() {
        assert!(fingerprint_model_match(
            TB320FC_FINGERPRINT,
            LAVIE_TAB_9QHD1_MODEL
        ));
    }

    #[test]
    fn tb320fc_device_accepts_lavie_tab_9qhd1_firmware() {
        assert!(fingerprint_model_match(
            LAVIE_TAB_9QHD1_FINGERPRINT,
            TB320FC_MODEL
        ));
    }

    #[test]
    fn xiaoxin_pro13_models_are_bidirectionally_equivalent() {
        assert!(fingerprint_model_match(TB390FU_FINGERPRINT, TB376FC_MODEL));
        assert!(fingerprint_model_match(TB376FC_FINGERPRINT, TB390FU_MODEL));
        assert!(fingerprint_model_match(TB391FC_FINGERPRINT, TB376FC_MODEL));
        assert!(fingerprint_model_match(TB391FC_FINGERPRINT, TB390FU_MODEL));
        assert!(fingerprint_model_match(TB376FC_FINGERPRINT, TB391FC_MODEL));
        assert!(fingerprint_model_match(TB390FU_FINGERPRINT, TB391FC_MODEL));
        assert!(fingerprint_model_match(TB376FC_FINGERPRINT, TB376FC_MODEL));
        assert!(fingerprint_model_match(TB390FU_FINGERPRINT, TB390FU_MODEL));
        assert!(fingerprint_model_match(TB391FC_FINGERPRINT, TB391FC_MODEL));
        assert!(is_xiaoxin_pro13_model(TB376FC_MODEL));
        assert!(is_xiaoxin_pro13_model(TB390FU_MODEL));
        assert!(is_xiaoxin_pro13_model(TB391FC_MODEL));
    }

    #[test]
    fn codename_fingerprint_matches_its_model_only() {
        let baldur =
            "Lenovo/lineage_baldur/baldur:17/CP2A.260605.016/eng.androi:userdebug/test-keys";
        assert!(fingerprint_model_match(baldur, "TB323FU"));
        assert!(!fingerprint_model_match(baldur, "TB320FC"));
        assert!(!fingerprint_model_match(baldur, "TB324ZC"));
        // Codenames inherit the model's equivalences.
        let asphalt = "Lenovo/lineage_asphalt/asphalt:17/build:userdebug/test-keys";
        assert!(fingerprint_model_match(asphalt, TB320FC_MODEL));
        assert!(fingerprint_model_match(asphalt, LAVIE_TAB_9QHD1_MODEL));
        assert!(!fingerprint_model_match(asphalt, "TB323FU"));
        assert!(!fingerprint_model_match(
            "Lenovo/baldurx/baldurx:17/b",
            "TB323FU"
        ));
        assert!(!fingerprint_model_match(
            "Lenovo/xbaldur/xbaldur:17/b",
            "TB323FU"
        ));
    }

    #[test]
    fn fingerprint_model_match_rejects_unrelated_and_suffixed_models() {
        assert!(!fingerprint_model_match(
            "qti/TB323FU/TB323FU:15/build",
            TB320FC_MODEL
        ));
        assert!(!fingerprint_model_match(
            "qti/TB320FCX/build",
            TB320FC_MODEL
        ));
        assert!(!fingerprint_model_match(
            "qti/LAVIETab9QHD1X/build",
            TB320FC_MODEL
        ));
    }
}
