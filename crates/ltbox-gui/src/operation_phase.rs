use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use crate::AdvAction;
use ltbox_core::tr_args;
use ltbox_patch::konabess::KonaBessBuildStage;

pub(crate) fn phase_marker(phase: usize, total: usize, label: impl AsRef<str>) -> String {
    tr_args!(
        "live_phase_marker",
        phase = phase.to_string(),
        total = total.to_string(),
        label = label.as_ref()
    )
}

/// Overall determinate completion from a zero-based phase and optional
/// within-phase percentage. A completed operation always fills the track.
pub(crate) fn operation_progress_fraction(
    current_step: usize,
    total_steps: usize,
    phase_percent: Option<u8>,
    complete: bool,
) -> f32 {
    if complete {
        return 1.0;
    }
    if total_steps == 0 {
        return 0.0;
    }
    let step = current_step.min(total_steps.saturating_sub(1)) as f32;
    let within_step = f32::from(phase_percent.unwrap_or(0).min(100)) / 100.0;
    ((step + within_step) / total_steps as f32).clamp(0.0, 1.0)
}

#[derive(Debug, Clone)]
pub(crate) struct OpStep {
    pub(crate) label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OperationPhaseKind {
    Flash,
    Root,
    Unroot,
    SysUpdateDisable,
    SysUpdateEnable,
    BootRecovery,
    ChangeCountry,
    DetectArb,
    SimpleFlash,
    FlashPartitions,
    DumpPartitions,
    FlashPhysical,
    DumpPhysical,
    OfflineConvertXml,
    RegionConversion,
    PatchArb,
    RebuildVbmeta,
    KonaBess,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OperationTransportHint {
    Current,
    Adb,
    Fastboot,
    Edl,
    Disconnected,
}

impl OperationPhaseKind {
    /// One-based phases doing the substantive device work and their share of
    /// the track. Transport setup and teardown share the remaining weight.
    fn progress_policy(self) -> Option<(&'static [usize], f32)> {
        match self {
            // The stock rawprogram pass and LTBox's generated overlays are
            // both firmware writes and together own 90% of the full flash.
            Self::Flash => Some((&[7, 8], 0.9)),
            Self::SimpleFlash => Some((&[3], 0.9)),
            Self::Root => Some((&[4, 5, 6], 0.8)),
            Self::Unroot => Some((&[4, 5], 0.8)),
            Self::BootRecovery => Some((&[4, 5, 6], 0.8)),
            Self::ChangeCountry => Some((&[3, 4], 0.8)),
            Self::DetectArb => Some((&[2, 4], 0.8)),
            Self::FlashPartitions | Self::DumpPartitions => Some((&[2], 0.8)),
            Self::FlashPhysical | Self::DumpPhysical => Some((&[3], 0.8)),
            Self::KonaBess => Some((&[2, 3, 4, 5, 6], 0.8)),
            Self::SysUpdateDisable
            | Self::SysUpdateEnable
            | Self::OfflineConvertXml
            | Self::RegionConversion
            | Self::PatchArb
            | Self::RebuildVbmeta => None,
        }
    }

    pub(crate) fn progress_fraction(
        self,
        current_step: usize,
        phase_percent: Option<u8>,
        complete: bool,
    ) -> f32 {
        let total = self.keys().len();
        let Some((work, share)) = self.progress_policy() else {
            return operation_progress_fraction(current_step, total, phase_percent, complete);
        };
        if complete {
            return 1.0;
        }
        let weight = |index: usize| {
            if work.contains(&(index + 1)) {
                share / work.len() as f32
            } else {
                (1.0 - share) / (total - work.len()) as f32
            }
        };
        let current = current_step.min(total - 1);
        let before: f32 = (0..current).map(weight).sum();
        let within = f32::from(phase_percent.unwrap_or(0).min(100)) / 100.0;
        (before + weight(current) * within).clamp(0.0, 1.0)
    }

    /// Transport implied by a worker phase. Device polling is intentionally
    /// paused while a workflow owns USB, so the execution view follows the
    /// worker's phase boundaries instead of a stale pre-operation poll.
    pub(crate) const fn transport_hint(self, current_step: usize) -> OperationTransportHint {
        use OperationTransportHint::{Adb, Current, Disconnected, Edl, Fastboot};
        match (self, current_step) {
            (Self::Flash, 0..=3) | (Self::Root | Self::Unroot, 0..=1) => Current,
            (Self::Flash, 4..=7)
            | (Self::Root, 2..=5)
            | (Self::Unroot, 2..=4)
            | (Self::BootRecovery, 1..=5)
            | (Self::ChangeCountry, 1..=3)
            | (Self::DetectArb, 2..=3)
            | (Self::SimpleFlash, 1..=3)
            | (Self::FlashPartitions, 0..=1)
            | (Self::DumpPartitions, 0..=2)
            | (Self::FlashPhysical, 0..=2)
            | (Self::DumpPhysical, 0..=3)
            | (Self::KonaBess, 1..=5) => Edl,
            (Self::Root, 7) | (Self::SysUpdateDisable | Self::SysUpdateEnable, _) => Adb,
            (Self::DetectArb, 0..=1) => Fastboot,
            (Self::BootRecovery | Self::KonaBess, 0) => Current,
            (Self::Flash, 8)
            | (Self::Root, 6)
            | (Self::Unroot, 5)
            | (Self::BootRecovery, 6)
            | (Self::ChangeCountry, 4)
            | (Self::DetectArb, 4)
            | (Self::SimpleFlash, 4)
            | (Self::FlashPartitions, 2)
            | (Self::DumpPartitions, 3)
            | (Self::FlashPhysical, 3)
            | (Self::DumpPhysical, 4)
            | (Self::KonaBess, 6)
            | (
                Self::OfflineConvertXml
                | Self::RegionConversion
                | Self::PatchArb
                | Self::RebuildVbmeta,
                _,
            ) => Disconnected,
            _ => Current,
        }
    }

    pub(crate) const fn all() -> &'static [Self] {
        &[
            Self::Flash,
            Self::Root,
            Self::Unroot,
            Self::SysUpdateDisable,
            Self::SysUpdateEnable,
            Self::BootRecovery,
            Self::ChangeCountry,
            Self::DetectArb,
            Self::SimpleFlash,
            Self::FlashPartitions,
            Self::DumpPartitions,
            Self::FlashPhysical,
            Self::DumpPhysical,
            Self::OfflineConvertXml,
            Self::RegionConversion,
            Self::PatchArb,
            Self::RebuildVbmeta,
            Self::KonaBess,
        ]
    }

    pub(crate) const fn for_advanced_file(action: AdvAction) -> Option<Self> {
        match action {
            AdvAction::ConvertXml => Some(Self::OfflineConvertXml),
            AdvAction::RegionConvert => Some(Self::RegionConversion),
            AdvAction::PatchArb => Some(Self::PatchArb),
            AdvAction::RebuildVbmeta => Some(Self::RebuildVbmeta),
            _ => None,
        }
    }

    pub(crate) const fn is_firmware_progress_step(self, one_based: usize) -> bool {
        match self {
            Self::Flash => matches!(one_based, 7 | 8),
            Self::SimpleFlash => one_based == 3,
            _ => false,
        }
    }

    pub(crate) const fn keys(self) -> &'static [&'static str] {
        match self {
            Self::Flash => &[
                "op_flash_phase_1",
                "op_flash_phase_2",
                "op_flash_phase_3",
                "op_flash_phase_4",
                "op_flash_phase_5",
                "op_flash_phase_6",
                "op_flash_phase_7",
                "op_flash_phase_8",
                "op_flash_phase_9",
            ],
            Self::Root => &[
                "op_root_phase_1",
                "op_root_phase_2",
                "op_root_phase_3",
                "op_root_phase_4",
                "op_root_phase_5",
                "op_root_phase_6",
                "op_root_phase_7",
                "op_root_phase_8",
            ],
            Self::Unroot => &[
                "op_unroot_phase_1",
                "op_unroot_phase_2",
                "op_unroot_phase_3",
                "op_unroot_phase_4",
                "op_unroot_phase_5",
                "op_unroot_phase_6",
            ],
            Self::SysUpdateDisable => &[
                "op_sys_phase_adb",
                "op_sys_disable_phase_policy",
                "op_sys_disable_phase_packages",
            ],
            Self::SysUpdateEnable => &[
                "op_sys_phase_adb",
                "op_sys_enable_phase_policy",
                "op_sys_enable_phase_packages",
            ],
            Self::BootRecovery => &[
                "op_rescue_phase_1",
                "op_rescue_phase_2",
                "op_rescue_phase_3",
                "op_rescue_phase_4",
                "op_rescue_phase_5",
                "op_rescue_phase_6",
                "op_rescue_phase_7",
            ],
            Self::ChangeCountry => &[
                "op_country_phase_validate",
                "op_phase_enter_edl_firehose",
                "op_country_phase_backup",
                "op_country_phase_apply",
                "op_phase_reboot_system",
            ],
            Self::DetectArb => &[
                "op_arb_phase_fastboot",
                "op_arb_phase_read",
                "op_arb_phase_edl",
                "op_arb_phase_result",
                "op_phase_reboot_system",
            ],
            Self::SimpleFlash => &[
                "op_simple_phase_prepare",
                "op_phase_enter_edl_firehose",
                "op_simple_phase_write",
                "op_simple_phase_slot",
                "op_phase_reboot_system",
            ],
            Self::FlashPartitions => &[
                "op_phase_open_firehose",
                "op_flashparts_phase_write",
                "op_phase_reboot_system",
            ],
            Self::DumpPartitions => &[
                "op_phase_open_firehose",
                "op_dumpparts_phase_read",
                "op_phase_stabilize_usb",
                "op_phase_reboot_system",
            ],
            Self::FlashPhysical => &[
                "op_phase_enter_edl",
                "op_phase_open_firehose",
                "op_flashphys_phase_write",
                "op_phase_reboot_system",
            ],
            Self::DumpPhysical => &[
                "op_phase_enter_edl",
                "op_phase_open_firehose",
                "op_dumpphys_phase_read",
                "op_phase_stabilize_usb",
                "op_phase_reboot_system",
            ],
            Self::OfflineConvertXml => &[
                "op_xml_phase_scan",
                "op_xml_phase_decrypt",
                "op_offline_phase_finalize",
            ],
            Self::RegionConversion => &[
                "op_region_phase_validate",
                "op_region_phase_inspect",
                "op_region_phase_patch",
                "op_region_phase_finalize",
            ],
            Self::PatchArb => &[
                "op_patch_arb_phase_inspect",
                "op_patch_arb_phase_keys",
                "op_patch_arb_phase_boot",
                "op_patch_arb_phase_vbmeta",
            ],
            Self::RebuildVbmeta => &[
                "op_vbmeta_phase_inspect",
                "op_vbmeta_phase_rebuild",
                "op_offline_phase_finalize",
            ],
            Self::KonaBess => &[
                "op_konabess_phase_prepare",
                "op_konabess_phase_dump",
                "op_konabess_phase_inspect",
                "op_konabess_phase_patch",
                "op_konabess_phase_rebuild",
                "op_konabess_phase_flash",
                "op_phase_reboot_system",
            ],
        }
    }
}

/// KonaBess build callbacks map into the stable inspect/patch/rebuild portion
/// of the full EDL operation plan, matching the region worker's stage mapping.
pub(crate) const fn konabess_build_phase(stage: KonaBessBuildStage) -> usize {
    match stage {
        KonaBessBuildStage::Inspect => 3,
        KonaBessBuildStage::PatchVendorBoot => 4,
        KonaBessBuildStage::RebuildVbmeta => 5,
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PhaseReporter {
    labels: Arc<[String]>,
    progress: Arc<OperationProgress>,
}

#[derive(Debug, Default)]
struct OperationProgress {
    step: AtomicUsize,
    writes_started: AtomicBool,
}

impl PhaseReporter {
    pub(crate) fn from_labels(labels: Vec<String>) -> Self {
        assert!(
            !labels.is_empty(),
            "operation phase plans must not be empty"
        );
        Self {
            labels: labels.into(),
            progress: Arc::default(),
        }
    }

    pub(crate) fn steps(&self) -> Vec<OpStep> {
        self.labels
            .iter()
            .cloned()
            .map(|label| OpStep { label })
            .collect()
    }

    pub(crate) fn marker(&self, one_based: usize) -> String {
        let index = one_based
            .checked_sub(1)
            .expect("phase markers are one-based");
        let label = self
            .labels
            .get(index)
            .expect("worker marker must exist in its phase plan");
        self.progress.step.store(index, Ordering::Relaxed);
        phase_marker(one_based, self.labels.len(), label)
    }

    pub(crate) fn current_step(&self) -> usize {
        self.progress.step.load(Ordering::Relaxed)
    }

    /// Latches immediately before a device mutation is attempted. An error may
    /// still leave a partial write, so this is never cleared within a run.
    pub(crate) fn mark_writes_started(&self) {
        self.progress.writes_started.store(true, Ordering::Relaxed);
    }

    pub(crate) fn writes_started(&self) -> bool {
        self.progress.writes_started.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn konabess_build_stages_map_to_inspect_patch_rebuild_phases() {
        assert_eq!(konabess_build_phase(KonaBessBuildStage::Inspect), 3);
        assert_eq!(konabess_build_phase(KonaBessBuildStage::PatchVendorBoot), 4);
        assert_eq!(konabess_build_phase(KonaBessBuildStage::RebuildVbmeta), 5);
    }

    #[test]
    fn determinate_progress_combines_phase_and_within_phase_progress() {
        assert_eq!(operation_progress_fraction(0, 9, None, false), 0.0);
        assert!((operation_progress_fraction(6, 9, Some(50), false) - 6.5 / 9.0).abs() < 0.001);
        assert_eq!(operation_progress_fraction(8, 9, None, true), 1.0);
    }

    #[test]
    fn weighted_progress_is_continuous_at_every_phase_boundary() {
        for &kind in OperationPhaseKind::all() {
            let mut previous = 0.0;
            for step in 0..kind.keys().len() {
                let start = kind.progress_fraction(step, Some(0), false);
                assert!((start - previous).abs() < 0.00001, "{kind:?}, {step}");
                previous = kind.progress_fraction(step, Some(100), false);
                assert!(previous >= start);
            }
            assert!((previous - 1.0).abs() < 0.00001, "{kind:?}");
            assert_eq!(kind.progress_fraction(0, None, true), 1.0);
        }
    }

    #[test]
    fn firmware_write_owns_ninety_percent_of_progress() {
        for kind in [OperationPhaseKind::Flash, OperationPhaseKind::SimpleFlash] {
            let total: f32 = (0..kind.keys().len())
                .filter(|step| kind.is_firmware_progress_step(step + 1))
                .map(|step| {
                    kind.progress_fraction(step, Some(100), false)
                        - kind.progress_fraction(step, Some(0), false)
                })
                .sum();
            assert!((total - 0.9).abs() < 0.00001);
        }
    }

    #[test]
    fn full_flash_overlay_phase_starts_where_rawprogram_finishes() {
        let kind = OperationPhaseKind::Flash;
        assert_eq!(
            kind.progress_fraction(6, Some(100), false),
            kind.progress_fraction(7, None, false)
        );
    }

    #[test]
    fn transport_hints_follow_worker_owned_usb_transitions() {
        use OperationTransportHint::{Adb, Current, Disconnected, Edl};

        assert_eq!(OperationPhaseKind::Flash.transport_hint(0), Current);
        assert_eq!(OperationPhaseKind::Flash.transport_hint(6), Edl);
        assert_eq!(OperationPhaseKind::Flash.transport_hint(8), Disconnected);
        assert_eq!(OperationPhaseKind::Root.transport_hint(5), Edl);
        assert_eq!(OperationPhaseKind::Root.transport_hint(7), Adb);
    }
}
