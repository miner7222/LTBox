//! Qualcomm regulator-vote identifiers used by KonaBess GPU tables.

const DIWALI_VOTES: [u32; 17] = [
    16, 48, 56, 64, 80, 96, 128, 144, 192, 224, 256, 320, 336, 352, 384, 400, 416,
];
const DIWALI_NAMES: [&str; 17] = [
    "RETENTION",
    "MIN_SVS",
    "LOW_SVS_D1",
    "LOW_SVS",
    "LOW_SVS_L1",
    "LOW_SVS_L2",
    "SVS",
    "SVS_L0",
    "SVS_L1",
    "SVS_L2",
    "NOM",
    "NOM_L1",
    "NOM_L2",
    "NOM_L3",
    "TURBO",
    "TURBO_L0",
    "TURBO_L1",
];

const PINEAPPLE_VOTES: [u32; 24] = [
    16, 48, 52, 56, 60, 64, 72, 80, 96, 128, 144, 192, 224, 256, 288, 320, 336, 384, 400, 416, 432,
    448, 464, 480,
];
const PINEAPPLE_NAMES: [&str; 24] = [
    "RETENTION",
    "MIN_SVS",
    "LOW_SVS_D2",
    "LOW_SVS_D1",
    "LOW_SVS_D0",
    "LOW_SVS",
    "LOW_SVS_P1",
    "LOW_SVS_L1",
    "LOW_SVS_L2",
    "SVS",
    "SVS_L0",
    "SVS_L1",
    "SVS_L2",
    "NOM",
    "NOM_L0",
    "NOM_L1",
    "NOM_L2",
    "TURBO",
    "TURBO_L0",
    "TURBO_L1",
    "TURBO_L2",
    "TURBO_L3",
    "SUPER_TURBO",
    "SUPER_TURBO_NO_CPR",
];

const SUN_VOTES: [u32; 26] = [
    16, 48, 50, 52, 56, 60, 64, 72, 80, 96, 128, 144, 192, 224, 256, 288, 320, 336, 384, 400, 416,
    432, 448, 452, 464, 480,
];
const SUN_NAMES: [&str; 26] = [
    "RETENTION",
    "MIN_SVS",
    "LOW_SVS_D3",
    "LOW_SVS_D2",
    "LOW_SVS_D1",
    "LOW_SVS_D0",
    "LOW_SVS",
    "LOW_SVS_P1",
    "LOW_SVS_L1",
    "LOW_SVS_L2",
    "SVS",
    "SVS_L0",
    "SVS_L1",
    "SVS_L2",
    "NOM",
    "NOM_L0",
    "NOM_L1",
    "NOM_L2",
    "TURBO",
    "TURBO_L0",
    "TURBO_L1",
    "TURBO_L2",
    "TURBO_L3",
    "TURBO_L4",
    "SUPER_TURBO",
    "SUPER_TURBO_NO_CPR",
];

// Qualcomm's SM8850 binding names vote 76 LOW_SVS_L0 (distinct from P1 at 72).
// OnePlusOSS/android_kernel_oneplus_sm8850, fc30e54174d254ff7f33622a9278e4435f6718d2:
// include/dt-bindings/regulator/qcom,rpmh-regulator-levels.h
const CANOE_VOTES: [u32; 29] = [
    16, 48, 50, 51, 52, 54, 56, 60, 64, 72, 76, 80, 96, 128, 144, 192, 224, 256, 288, 320, 336,
    384, 400, 416, 432, 448, 452, 464, 480,
];
const CANOE_NAMES: [&str; 29] = [
    "RETENTION",
    "MIN_SVS",
    "LOW_SVS_D3",
    "LOW_SVS_D2_5",
    "LOW_SVS_D2",
    "LOW_SVS_D1_5",
    "LOW_SVS_D1",
    "LOW_SVS_D0",
    "LOW_SVS",
    "LOW_SVS_P1",
    "LOW_SVS_L0",
    "LOW_SVS_L1",
    "LOW_SVS_L2",
    "SVS",
    "SVS_L0",
    "SVS_L1",
    "SVS_L2",
    "NOM",
    "NOM_L0",
    "NOM_L1",
    "NOM_L2",
    "TURBO",
    "TURBO_L0",
    "TURBO_L1",
    "TURBO_L2",
    "TURBO_L3",
    "TURBO_L4",
    "SUPER_TURBO",
    "SUPER_TURBO_NO_CPR",
];

fn chip_levels(chip: &str) -> Option<(&'static [u32], &'static [&'static str])> {
    match chip {
        "diwali" => Some((&DIWALI_VOTES, &DIWALI_NAMES)),
        "pineapple" => Some((&PINEAPPLE_VOTES, &PINEAPPLE_NAMES)),
        "sun" => Some((&SUN_VOTES, &SUN_NAMES)),
        "canoe" => Some((&CANOE_VOTES, &CANOE_NAMES)),
        _ => None,
    }
}

/// All known encoded regulator votes for `chip`, in upstream order.
pub fn regulator_level_votes(chip: &str) -> Option<&'static [u32]> {
    chip_levels(chip).map(|(votes, _)| votes)
}

/// The upstream identifier for an exact encoded regulator vote.
pub fn regulator_level_name(chip: &str, vote: u32) -> Option<&'static str> {
    let (votes, names) = chip_levels(chip)?;
    votes
        .iter()
        .position(|candidate| *candidate == vote)
        .map(|index| names[index])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vote_and_name_arrays_have_identical_upstream_lengths() {
        for (votes, names, expected) in [
            (&DIWALI_VOTES[..], &DIWALI_NAMES[..], 17),
            (&PINEAPPLE_VOTES[..], &PINEAPPLE_NAMES[..], 24),
            (&SUN_VOTES[..], &SUN_NAMES[..], 26),
            (&CANOE_VOTES[..], &CANOE_NAMES[..], 29),
        ] {
            assert_eq!(votes.len(), expected);
            assert_eq!(names.len(), expected);
            assert_eq!(votes.len(), names.len());
        }
    }

    #[test]
    fn exact_votes_resolve_without_aliasing_unknown_chips_or_values() {
        assert_eq!(regulator_level_name("diwali", 384), Some("TURBO"));
        assert_eq!(regulator_level_name("pineapple", 256), Some("NOM"));
        assert_eq!(regulator_level_name("sun", 452), Some("TURBO_L4"));
        assert_eq!(regulator_level_name("canoe", 51), Some("LOW_SVS_D2_5"));
        assert_eq!(regulator_level_name("sun", 51), None);
    }

    #[test]
    fn canoe_low_svs_l0_is_distinct_from_its_neighbors() {
        assert_eq!(regulator_level_name("canoe", 72), Some("LOW_SVS_P1"));
        assert_eq!(regulator_level_name("canoe", 76), Some("LOW_SVS_L0"));
        assert_eq!(regulator_level_name("canoe", 80), Some("LOW_SVS_L1"));
        let votes = regulator_level_votes("canoe").unwrap();
        assert!(votes.windows(3).any(|votes| votes == [72, 76, 80]));
        for chip in ["diwali", "pineapple", "sun", "unknown"] {
            assert_eq!(regulator_level_name(chip, 76), None);
        }
    }
}
