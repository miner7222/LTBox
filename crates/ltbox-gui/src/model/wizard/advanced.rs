//! Advanced single-action wizard state.

use super::*;

/// Wizard for every non-FlashPartitions Advanced action. Steps are
/// [source, confirm, exec], plus country step between for `PatchDevinfo`.
#[derive(Default, Debug, Clone)]
pub(crate) struct AdvWizard {
    pub(crate) action: Option<AdvAction>,
    pub(crate) step: usize,
    pub(crate) file_path: Option<String>,
    pub(crate) file_paths: Vec<String>,
    pub(crate) country: Option<String>,
    /// User-picked target region for `RegionConvert`. Explicit target so
    /// confirm can echo it and exec can short-circuit on no-op.
    pub(crate) region_target: Option<DeviceRegion>,
    /// Output folder; set on Confirm → Exec.
    pub(crate) output_dir: Option<std::path::PathBuf>,
    pub(crate) arb_targets: Option<crate::ManualRollbackIndices>,
    /// PatchArb: `(boot, vbmeta_system)` rollback indices from the picked firmware.
    pub(crate) arb_inspect: Option<(u64, u64)>,
}

impl AdvWizard {
    pub(crate) fn open(&mut self, a: AdvAction) {
        *self = Self::default();
        self.action = Some(a);
    }
    pub(crate) fn needs_country(&self) -> bool {
        matches!(self.action, Some(AdvAction::PatchDevinfo))
    }
    pub(crate) fn needs_region_target(&self) -> bool {
        matches!(self.action, Some(AdvAction::RegionConvert))
    }
    pub(crate) fn is_image_info(&self) -> bool {
        matches!(self.action, Some(AdvAction::ImageInfo))
    }
    pub(crate) fn steps(&self) -> &'static [&'static str] {
        if self.is_image_info() {
            return &["adv_step_source", "adv_step_info"];
        }
        if self.needs_country() {
            // The country is chosen in a popup before this wizard opens.
            &["edl_loader_label", "flash_step_confirm", "flash_step_flash"]
        } else if self.needs_region_target() {
            &[
                "adv_step_source",
                "adv_step_region_target",
                "flash_step_confirm",
                "flash_step_flash",
            ]
        } else if matches!(self.action, Some(AdvAction::PatchArb)) {
            &["adv_step_source", "flash_step_confirm", "flash_step_flash"]
        } else if matches!(self.action, Some(AdvAction::DetectArb)) {
            // The model and transport decide whether a loader is needed; Start
            // on the source step runs directly.
            &["adv_step_source", "flash_step_flash"]
        } else {
            &["adv_step_source", "flash_step_confirm", "flash_step_flash"]
        }
    }
    pub(crate) fn exec_step(&self) -> usize {
        self.steps().len() - 1
    }
    pub(crate) fn is_confirm_step(&self) -> bool {
        !self.is_image_info() && self.step + 1 == self.exec_step()
    }
}

impl Wizard for AdvWizard {
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
        if self.needs_country() {
            return self.country.is_some() && self.file_path.is_some();
        }
        if self.step == 0 {
            if self.is_image_info() {
                return !self.file_paths.is_empty();
            }
            return self.file_path.is_some();
        }
        if self.needs_region_target() && self.step == 1 {
            return self.region_target.is_some();
        }
        // Offline editing requires inspected sources and both confirmed targets.
        if matches!(self.action, Some(AdvAction::PatchArb)) && self.step == 1 {
            return self.arb_inspect.is_some() && self.arb_targets.is_some();
        }
        true
    }
}

impl AdvWizard {
    /// Folder-vs-file dispatch for Browse on step 0.
    pub(crate) fn is_folder_op(&self) -> bool {
        matches!(
            self.action,
            // ConvertXml: folder holds the encrypted `*.x` pack.
            // PatchArb: folder holds boot.img + vbmeta_system.img.
            // Change Country Code picks an EDL loader file, not a folder.
            Some(AdvAction::ConvertXml) | Some(AdvAction::PatchArb)
        )
    }
    /// Extension whitelist for `rfd::AsyncFileDialog::add_filter`.
    /// Empty slice = no constraint.
    pub(crate) fn accepted_exts(&self) -> (&'static str, &'static [&'static str]) {
        match self.action {
            Some(AdvAction::RegionConvert)
            | Some(AdvAction::ImageInfo)
            | Some(AdvAction::RebuildVbmeta) => ("Android partition image (*.img)", &["img"]),
            Some(AdvAction::DetectArb) | Some(AdvAction::PatchDevinfo) => {
                ("EDL loader (.melf / .xml / .x)", LOADER_PICKER_EXTS)
            }
            _ => ("", &[]),
        }
    }

    /// Recents bucket for the current action: a source-folder category for
    /// folder actions, the shared `File` bucket for file actions. Must agree
    /// with [`Self::is_folder_op`], or recents are orphaned or corrupted.
    pub(crate) fn picker_kind(&self) -> pickers::PickerKind {
        use pickers::PickerKind;
        match self.action {
            // Source folders (existing payloads).
            Some(AdvAction::ConvertXml) => PickerKind::EncryptedRawprogramFolder,
            Some(AdvAction::PatchArb) => PickerKind::QfilFirmwareFolder,
            // File-picking actions - all share the unified File bucket.
            Some(AdvAction::RegionConvert)
            | Some(AdvAction::ImageInfo)
            | Some(AdvAction::DetectArb)
            | Some(AdvAction::PatchDevinfo)
            | Some(AdvAction::RebuildVbmeta) => PickerKind::File,
            // Remaining actions have dedicated wizards and no Browse dialog on
            // step 0; `File` keeps `storage_key()` valid.
            _ => PickerKind::File,
        }
    }
}
