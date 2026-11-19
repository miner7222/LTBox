//! System Update wizard state.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SysUpdateAction {
    Disable,
    Enable,
    Rescue,
}
impl SysUpdateAction {
    pub(crate) fn label_key(&self) -> &'static str {
        match self {
            Self::Disable => "sysupdate_disable",
            Self::Enable => "sysupdate_enable",
            Self::Rescue => "sysupdate_rescue",
        }
    }
    pub(crate) fn desc_key(&self) -> &'static str {
        match self {
            Self::Disable => "sysupdate_disable_desc",
            Self::Enable => "sysupdate_enable_desc",
            Self::Rescue => "sysupdate_rescue_desc",
        }
    }
}

/// Region target for Boot Recovery (Rescue). PRC/ROW hardware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RescueRegion {
    Prc,
    Row,
}

impl RescueRegion {
    pub(crate) fn label_key(self) -> &'static str {
        match self {
            Self::Prc => "rescue_region_prc",
            Self::Row => "rescue_region_row",
        }
    }
    pub(crate) fn to_target(self) -> ltbox_patch::region::RegionTarget {
        match self {
            Self::Prc => ltbox_patch::region::RegionTarget::Prc,
            Self::Row => ltbox_patch::region::RegionTarget::Row,
        }
    }
}

#[derive(Default)]
pub(crate) struct SysUpdateWizard {
    pub(crate) step: usize,
    pub(crate) action: Option<SysUpdateAction>,
    /// Rescue: selected EDL loader file (`xbl_s_devprg_ns.melf`) or Sahara manifest.
    pub(crate) rescue_folder: Option<String>,
    /// Rescue: target region, set only when the user confirms the popup between
    /// the loader and Confirm steps. This is the value the worker receives.
    pub(crate) rescue_region: Option<RescueRegion>,
    /// Rescue: row selected in the popup but not yet confirmed. Starts empty on
    /// every open; the detected hardware region is only shown as a hint.
    pub(crate) rescue_region_pending: Option<RescueRegion>,
    /// Rescue: region popup. Opens on Next from the loader step until a region
    /// is confirmed.
    pub(crate) rescue_region_popup_open: bool,
    /// Rescue: true once the user confirms a region in the popup.
    pub(crate) rescue_region_confirmed: bool,
}

pub(crate) const SYSUPDATE_STEPS_COMPACT: &[&str] = &[
    "sysupdate_step_action",
    "sysupdate_step_confirm",
    "sysupdate_step_execute",
];

pub(crate) const SYSUPDATE_STEPS_RESCUE: &[&str] = &[
    "sysupdate_step_action",
    "edl_loader_label",
    "sysupdate_step_confirm",
    "sysupdate_step_execute",
];

impl SysUpdateWizard {
    /// Rescue has an extra loader step; Disable/Enable keep the short 3-step flow.
    pub(crate) fn steps(&self) -> &'static [&'static str] {
        if matches!(self.action, Some(SysUpdateAction::Rescue)) {
            SYSUPDATE_STEPS_RESCUE
        } else {
            SYSUPDATE_STEPS_COMPACT
        }
    }
    pub(crate) fn is_rescue(&self) -> bool {
        matches!(self.action, Some(SysUpdateAction::Rescue))
    }
}

impl Wizard for SysUpdateWizard {
    fn step(&self) -> usize {
        self.step
    }
    fn step_mut(&mut self) -> &mut usize {
        &mut self.step
    }
    fn step_count(&self) -> usize {
        self.steps().len()
    }
    fn can_next(&self) -> bool {
        if self.is_rescue() {
            // Rescue flow: Action → Loader → Confirm → Exec.
            match self.step {
                0 => self.action.is_some(),
                1 => self
                    .rescue_folder
                    .as_deref()
                    .map(std::path::Path::new)
                    .is_some_and(|p| {
                        is_loader_file(p)
                            || ltbox_core::sahara_xml::is_encrypted_manifest_filename(p)
                    }),
                2 => self.rescue_region.is_some(),
                _ => false,
            }
        } else {
            match self.step {
                0 => self.action.is_some(),
                1 => true,
                _ => false,
            }
        }
    }
}
