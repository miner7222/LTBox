//! Unroot wizard state.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnrootType {
    MagiskLkm,
    APatchGki,
}
impl UnrootType {
    pub(crate) fn label_key(&self) -> &'static str {
        match self {
            Self::MagiskLkm => "unroottype_magisk_lkm",
            Self::APatchGki => "unroottype_apatch_gki",
        }
    }
    pub(crate) fn desc_key(&self) -> &'static str {
        match self {
            Self::MagiskLkm => "unroottype_magisk_lkm_desc",
            Self::APatchGki => "unroottype_apatch_gki_desc",
        }
    }
}

#[derive(Default)]
pub(crate) struct UnrootWizard {
    pub(crate) step: usize,
    pub(crate) unroot_type: Option<UnrootType>,
    pub(crate) folder_path: Option<String>,
    /// Loader file (`xbl_s_devprg_ns.melf`) for the EDL flash, on its own step.
    /// A loader remembered for the connected model is filled in and the step
    /// skipped on Next from the method step; otherwise the picker shows.
    pub(crate) loader_path: Option<String>,
    /// Loader-resolution failure, kept apart from any scan error so a
    /// refused pick does not overwrite why the last scan failed.
    pub(crate) loader_error: Option<String>,
    /// LTBox-owned root snapshots, refreshed whenever the folder step is entered.
    pub(crate) backup_folders: Vec<crate::backup::BackupFolderEntry>,
    pub(crate) backup_scan_error: Option<String>,
    pub(crate) backup_scan_request: Option<std::time::Instant>,
    pub(crate) backup_manifest_request: Option<std::time::Instant>,
    pub(crate) backup_manifest_dialog: Option<crate::backup::BackupManifestDialog>,
}

pub(crate) const UNROOT_STEPS: &[&str] = &[
    "unroot_step_method",
    "edl_loader_label",
    "unroot_step_folder",
    "unroot_step_confirm",
    "unroot_step_restore",
];

impl Wizard for UnrootWizard {
    fn step(&self) -> usize {
        self.step
    }
    fn step_mut(&mut self) -> &mut usize {
        &mut self.step
    }
    fn step_count(&self) -> usize {
        UNROOT_STEPS.len()
    }
    fn can_next(&self) -> bool {
        // Indexes match `UNROOT_STEPS`: loader (1) and folder (2) are separate steps.
        match self.step {
            0 => self.unroot_type.is_some(),
            1 => self.loader_path.is_some() && self.loader_error.is_none(),
            2 => self.folder_path.is_some(),
            3 => true,
            _ => false,
        }
    }
}
