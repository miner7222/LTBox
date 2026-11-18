//! Anti-rollback — per-location rollback-index detection and patching.
//!
//! Fastboot `stored_rollback_index:N` entries are classified only for a
//! two-entry non-recovery layout. Other layouts require device image reads.
//! [`RollbackMode`] selects automatic or device-floor-driven patching;
//! `Off` skips, and `Manual` uses caller-supplied targets.

use std::collections::HashMap;
use std::path::Path;

use ltbox_core::Result;
use tracing::info;

use crate::avb::{self, AvbImageInfo};

/// Rollback-patch mode: `On` patches whenever a device floor exists, `Auto`
/// only when image index < device floor, `Off` skips, and `Manual` uses
/// validated caller-supplied targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RollbackMode {
    On,
    Auto,
    Off,
    Manual,
}

/// Fastboot rollback floors classified from two meaningful non-recovery
/// locations. Lenovo layouts place `vbmeta_system` at the lower location and
/// `boot` at the higher location; numeric ordering also supports shifted pairs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FastbootRollbackFloors {
    /// Persistent rollback location classified as `vbmeta_system`.
    pub vbmeta_system_location: u32,
    /// Device floor stored at [`Self::vbmeta_system_location`].
    pub vbmeta_system_index: u64,
    /// Persistent rollback location classified as `boot`.
    pub boot_location: u32,
    /// Device floor stored at [`Self::boot_location`].
    pub boot_index: u64,
}

/// Rollback indices selected for the `boot` and `vbmeta_system` partitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RollbackIndices {
    pub boot: u64,
    pub vbmeta_system: u64,
}

/// Manual rollback targets and whether each target differs from firmware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ManualRollbackPlan {
    pub targets: RollbackIndices,
    pub boot_changed: bool,
    pub vbmeta_system_changed: bool,
}

impl ManualRollbackPlan {
    /// Returns whether either partition target differs from its firmware index.
    pub fn changes_indices(&self) -> bool {
        self.boot_changed || self.vbmeta_system_changed
    }
}

/// A requested manual target that is below the device floor for one partition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ManualRollbackBelowFloor {
    pub partition: &'static str,
    pub requested: u64,
    pub floor: u64,
}

/// Validate manual rollback targets against device floors and report changes.
pub fn plan_manual_rollback(
    firmware: RollbackIndices,
    device_floors: RollbackIndices,
    requested: RollbackIndices,
) -> std::result::Result<ManualRollbackPlan, ManualRollbackBelowFloor> {
    if requested.boot < device_floors.boot {
        return Err(ManualRollbackBelowFloor {
            partition: "boot",
            requested: requested.boot,
            floor: device_floors.boot,
        });
    }
    if requested.vbmeta_system < device_floors.vbmeta_system {
        return Err(ManualRollbackBelowFloor {
            partition: "vbmeta_system",
            requested: requested.vbmeta_system,
            floor: device_floors.vbmeta_system,
        });
    }

    Ok(ManualRollbackPlan {
        targets: requested,
        boot_changed: firmware.boot != requested.boot,
        vbmeta_system_changed: firmware.vbmeta_system != requested.vbmeta_system,
    })
}

/// Classify exactly two meaningful fastboot rollback entries excluding
/// location 1 (`recovery`). The lower location is `vbmeta_system`; the higher
/// location is `boot`. Returns `None` for every other shape; callers then read
/// the per-location floors from device images.
pub fn classify_fastboot_rollback_floors(
    stored: &HashMap<u32, u64>,
) -> Option<FastbootRollbackFloors> {
    let mut candidates = stored.iter().filter_map(|(&location, &index)| {
        (location != 1 && index > 1).then_some((location, index))
    });
    let first = candidates.next()?;
    let second = candidates.next()?;
    if candidates.next().is_some() {
        return None;
    }

    let (vbmeta_system, boot) = if first.0 < second.0 {
        (first, second)
    } else {
        (second, first)
    };
    Some(FastbootRollbackFloors {
        vbmeta_system_location: vbmeta_system.0,
        vbmeta_system_index: vbmeta_system.1,
        boot_location: boot.0,
        boot_index: boot.1,
    })
}

/// Decide whether to patch given mode, image index, and device index.
/// `device_index = None` (no non-stock value committed) never patches. `Off`
/// and `Manual` never patch here.
pub fn needs_patch(mode: RollbackMode, image_index: u64, device_index: Option<u64>) -> bool {
    match mode {
        RollbackMode::Off | RollbackMode::Manual => false,
        RollbackMode::On => device_index.is_some(),
        RollbackMode::Auto => match device_index {
            Some(d) => image_index < d,
            None => false,
        },
    }
}

/// Result of rollback-index analysis against a device.
pub struct RollbackAnalysis {
    pub image_index: u64,
    pub needs_patch: bool,
    pub image_info: AvbImageInfo,
}

/// Rollback analysis with mode. `device_index = None` → no non-stock
/// value committed; never triggers a patch.
pub fn analyze_rollback_with_mode(
    image_path: &Path,
    device_index: Option<u64>,
    mode: RollbackMode,
) -> Result<RollbackAnalysis> {
    let image_info = avb::extract_image_avb_info(image_path)?;
    let image_index = image_info.rollback_index;
    let needs_patch = needs_patch(mode, image_index, device_index);
    info!(
        "Rollback analysis: mode={mode:?}, device={device_index:?}, image={image_index}, needs_patch={needs_patch}"
    );
    Ok(RollbackAnalysis {
        image_index,
        needs_patch,
        image_info,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_indices(values: &[(u32, u64)]) -> HashMap<u32, u64> {
        values.iter().copied().collect()
    }

    /// Real TB520FU (`lapis`) dump: 32 locations, only 2 and 3 carry a
    /// value, location 1 is the stock recovery `1`. Captured from
    /// `fastboot getvar all` on hardware.
    #[test]
    fn classify_real_tb520fu_dump() {
        let mut stored = HashMap::new();
        for loc in 0..=31u32 {
            stored.insert(loc, 0u64);
        }
        stored.insert(1, 1);
        stored.insert(2, 0x69D1_A600);
        stored.insert(3, 0x69D1_A600);
        assert_eq!(
            classify_fastboot_rollback_floors(&stored),
            Some(FastbootRollbackFloors {
                vbmeta_system_location: 2,
                vbmeta_system_index: 0x69D1_A600,
                boot_location: 3,
                boot_index: 0x69D1_A600,
            })
        );
    }

    #[test]
    fn classify_standard_non_recovery_locations() {
        let indices = make_indices(&[(0, 0), (1, 1), (2, 0x69D0A600), (3, 0x69D1A600)]);
        assert_eq!(
            classify_fastboot_rollback_floors(&indices),
            Some(FastbootRollbackFloors {
                vbmeta_system_location: 2,
                vbmeta_system_index: 0x69D0A600,
                boot_location: 3,
                boot_index: 0x69D1A600,
            })
        );
    }

    #[test]
    fn classify_shifted_locations_by_numeric_order() {
        let indices = make_indices(&[(17, 100), (18, 200)]);
        assert_eq!(
            classify_fastboot_rollback_floors(&indices),
            Some(FastbootRollbackFloors {
                vbmeta_system_location: 17,
                vbmeta_system_index: 100,
                boot_location: 18,
                boot_index: 200,
            })
        );
    }

    #[test]
    fn classify_counts_locations_when_values_match() {
        let indices = make_indices(&[(2, 0x69D1A600), (3, 0x69D1A600)]);
        assert_eq!(
            classify_fastboot_rollback_floors(&indices),
            Some(FastbootRollbackFloors {
                vbmeta_system_location: 2,
                vbmeta_system_index: 0x69D1A600,
                boot_location: 3,
                boot_index: 0x69D1A600,
            })
        );
    }

    #[test]
    fn classify_ignores_stock_values() {
        let indices = make_indices(&[(0, 0), (2, 10), (3, 11), (4, 1), (31, 0)]);
        assert_eq!(
            classify_fastboot_rollback_floors(&indices),
            Some(FastbootRollbackFloors {
                vbmeta_system_location: 2,
                vbmeta_system_index: 10,
                boot_location: 3,
                boot_index: 11,
            })
        );
    }

    #[test]
    fn classify_always_excludes_recovery_location() {
        let indices = make_indices(&[(1, 999), (8, 10), (9, 11)]);
        assert_eq!(
            classify_fastboot_rollback_floors(&indices),
            Some(FastbootRollbackFloors {
                vbmeta_system_location: 8,
                vbmeta_system_index: 10,
                boot_location: 9,
                boot_index: 11,
            })
        );
    }

    #[test]
    fn classify_requires_exactly_two_candidates() {
        assert_eq!(
            classify_fastboot_rollback_floors(&make_indices(&[(2, 10)])),
            None
        );
        assert_eq!(
            classify_fastboot_rollback_floors(&make_indices(&[(2, 10), (3, 11), (4, 12)])),
            None
        );
    }

    #[test]
    fn needs_patch_off_never() {
        assert!(!needs_patch(RollbackMode::Off, 0, Some(10)));
        assert!(!needs_patch(RollbackMode::Off, 100, Some(10)));
    }

    #[test]
    fn needs_patch_on_patches_when_device_committed() {
        assert!(needs_patch(RollbackMode::On, 0, Some(5)));
        assert!(needs_patch(RollbackMode::On, 5, Some(5)));
        assert!(needs_patch(RollbackMode::On, 100, Some(5)));
    }

    #[test]
    fn needs_patch_on_skipped_when_device_none() {
        assert!(!needs_patch(RollbackMode::On, 0, None));
    }

    #[test]
    fn needs_patch_auto_only_when_behind() {
        assert!(needs_patch(RollbackMode::Auto, 3, Some(5)));
        assert!(!needs_patch(RollbackMode::Auto, 5, Some(5)));
        assert!(!needs_patch(RollbackMode::Auto, 7, Some(5)));
        assert!(!needs_patch(RollbackMode::Auto, 0, None));
        // Manual never consults a device floor; its caller supplies targets.
        assert!(!needs_patch(RollbackMode::Manual, 0, Some(10)));
        assert!(!needs_patch(RollbackMode::Manual, 100, Some(10)));
        assert!(!needs_patch(RollbackMode::Manual, 0, None));
    }

    #[test]
    fn manual_plan_accepts_asymmetric_floor_boundaries_and_preserves_targets() {
        let firmware = RollbackIndices {
            boot: 50,
            vbmeta_system: 80,
        };
        let floors = RollbackIndices {
            boot: 10,
            vbmeta_system: 90,
        };
        let requested = RollbackIndices {
            boot: 10,
            vbmeta_system: 100,
        };

        let plan = plan_manual_rollback(firmware, floors, requested).unwrap();

        assert_eq!(plan.targets, requested);
        assert!(plan.boot_changed);
        assert!(plan.vbmeta_system_changed);
        assert!(plan.changes_indices());
    }

    #[test]
    fn manual_plan_rejects_boot_below_floor_before_unchanged_check() {
        let requested = RollbackIndices {
            boot: 9,
            vbmeta_system: 20,
        };
        let error = plan_manual_rollback(
            requested,
            RollbackIndices {
                boot: 10,
                vbmeta_system: 20,
            },
            requested,
        )
        .unwrap_err();

        assert_eq!(
            error,
            ManualRollbackBelowFloor {
                partition: "boot",
                requested: 9,
                floor: 10,
            }
        );
    }

    #[test]
    fn manual_plan_rejects_vbmeta_system_below_floor() {
        let error = plan_manual_rollback(
            RollbackIndices {
                boot: 10,
                vbmeta_system: 9,
            },
            RollbackIndices {
                boot: 10,
                vbmeta_system: 10,
            },
            RollbackIndices {
                boot: 10,
                vbmeta_system: 9,
            },
        )
        .unwrap_err();

        assert_eq!(
            error,
            ManualRollbackBelowFloor {
                partition: "vbmeta_system",
                requested: 9,
                floor: 10,
            }
        );
    }

    #[test]
    fn manual_plan_accepts_firmware_equal_to_requested_at_floor() {
        let requested = RollbackIndices {
            boot: 10,
            vbmeta_system: 20,
        };
        let plan = plan_manual_rollback(
            requested,
            RollbackIndices {
                boot: 10,
                vbmeta_system: 20,
            },
            requested,
        )
        .unwrap();

        assert_eq!(plan.targets, requested);
        assert!(!plan.boot_changed);
        assert!(!plan.vbmeta_system_changed);
        assert!(!plan.changes_indices());
    }

    #[test]
    fn manual_plan_preserves_targets_above_firmware_and_u64_max() {
        let requested = RollbackIndices {
            boot: u64::MAX,
            vbmeta_system: u64::MAX - 1,
        };
        let plan = plan_manual_rollback(
            RollbackIndices {
                boot: 1,
                vbmeta_system: 2,
            },
            RollbackIndices {
                boot: 0,
                vbmeta_system: 0,
            },
            requested,
        )
        .unwrap();

        assert_eq!(plan.targets, requested);
        assert_eq!(plan.targets.boot, u64::MAX);
        assert_eq!(plan.targets.vbmeta_system, u64::MAX - 1);
        assert!(plan.changes_indices());
    }
}
