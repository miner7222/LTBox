//! Wizard model — the per-flow wizard state structs and their
//! navigation logic, extracted from `main.rs`.

use crate::pickers;
use crate::{
    AdvAction, ConnectionStatus, Family, LOADER_PICKER_EXTS, NightlySource, Provider, RootMode,
    SkrootFlavor, VerChoice, is_loader_file,
};
use std::collections::BTreeMap;

use ltbox_patch::konabess::{
    GpuGroup, GpuTable, GpuTableIssue, GpuTableValidation, KonaBessExport, VendorBootDtbInfo,
    build_gpu_level_from_template, chip_names_match, normalize_edited_gpu_table, parse_gpu_cell,
    validate_gpu_table,
};

// Internal steps: 0=Family, 1=Mode, 2=Provider, 3=Version,
// 4=NightlySource, 5=Folder, 6=Confirm, 7=Flash, 8=APatch KPM.
// Mode auto-skips for non-KSU. GKI: steps 3/4 collapse into a kernel
// zip picker at 2. MagiskForks: skip Version, APK picker at 3. Nightly
// inserts 4 between Version and Folder.
#[derive(Default)]
pub(crate) struct RootWizard {
    pub(crate) step: usize,
    pub(crate) family: Option<Family>,
    pub(crate) mode: Option<RootMode>,
    pub(crate) skroot_flavor: Option<SkrootFlavor>,
    pub(crate) provider: Option<Provider>,
    pub(crate) version: Option<VerChoice>,
    pub(crate) nightly_source: Option<NightlySource>,
    pub(crate) file_path: Option<String>, // GKI zip, MagiskForks APK, or manual nightly
    pub(crate) folder_path: Option<String>, // Firmware folder (loader + optional testkey)
    /// APatch: `.kpm` modules to embed. Multi-select + per-entry remove.
    pub(crate) kpm_paths: Vec<String>,
    /// APatch superkey. Secret — never echoed in confirm or any log.
    pub(crate) superkey: Option<String>,
    pub(crate) superkey_popup_open: bool,
    /// Buffer for the currently visible field in the superkey popup;
    /// reset between the first-entry and re-entry stages.
    pub(crate) superkey_buffer: String,
    /// First-entry value held while the popup waits for the user to
    /// re-enter their key on the second stage. `None` → still on the
    /// first-entry stage; `Some(v)` → on the verification stage and
    /// `superkey_buffer` will be compared against `v` on Confirm.
    pub(crate) superkey_first_entry: Option<String>,
    /// Nightly ManualInput: committed workflow run ID (1..=12 digits).
    /// Only meaningful when `nightly_source == Some(ManualInput)`.
    pub(crate) run_id: Option<String>,
    pub(crate) run_id_popup_open: bool,
    pub(crate) run_id_buffer: String,
    /// KernelSU LKM: normalized `major.minor` kernel version from ADB or manual popup.
    pub(crate) kernel_version: Option<String>,
    pub(crate) kernel_version_popup_open: bool,
    pub(crate) kernel_version_buffer: String,
}

pub(crate) const ROOT_STEPS: &[&str] = &[
    "root_step_type",
    "root_step_mode",
    "root_step_provider",
    "root_step_version",
    "edl_loader_label",
    "root_step_confirm",
    "root_step_flash",
];
pub(crate) const ROOT_STEPS_NIGHTLY: &[&str] = &[
    "root_step_type",
    "root_step_mode",
    "root_step_provider",
    "root_step_version",
    "root_step_source",
    "edl_loader_label",
    "root_step_confirm",
    "root_step_flash",
];
pub(crate) const ROOT_STEPS_GKI: &[&str] = &[
    "root_step_type",
    "root_step_mode",
    "root_step_kernel",
    "edl_loader_label",
    "root_step_confirm",
    "root_step_flash",
];
pub(crate) const ROOT_STEPS_NOMODE: &[&str] = &[
    "root_step_type",
    "root_step_provider",
    "root_step_version",
    "edl_loader_label",
    "root_step_confirm",
    "root_step_flash",
];
pub(crate) const ROOT_STEPS_NOMODE_NIGHTLY: &[&str] = &[
    "root_step_type",
    "root_step_provider",
    "root_step_version",
    "root_step_source",
    "edl_loader_label",
    "root_step_confirm",
    "root_step_flash",
];
pub(crate) const ROOT_STEPS_FORKS: &[&str] = &[
    "root_step_type",
    "root_step_provider",
    "root_step_apk",
    "edl_loader_label",
    "root_step_confirm",
    "root_step_flash",
];
pub(crate) const ROOT_STEPS_APATCH: &[&str] = &[
    "root_step_type",
    "root_step_provider",
    "root_step_version",
    "root_step_kpm",
    "edl_loader_label",
    "root_step_confirm",
    "root_step_flash",
];
pub(crate) const ROOT_STEPS_APATCH_NIGHTLY: &[&str] = &[
    "root_step_type",
    "root_step_provider",
    "root_step_version",
    "root_step_source",
    "root_step_kpm",
    "edl_loader_label",
    "root_step_confirm",
    "root_step_flash",
];
pub(crate) const ROOT_STEPS_SKROOT: &[&str] = &[
    "root_step_type",
    "root_step_skroot_flavor",
    "edl_loader_label",
    "root_step_confirm",
    "root_step_flash",
];

impl RootWizard {
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    /// True on the final (flash/exec) step. Used to skip wizard reset
    /// when the user sidebar-bounces mid-operation.
    pub(crate) fn is_in_exec(&self) -> bool {
        self.step == 7
    }
    /// True on the confirm screen (step 6, before Flash). A sidebar
    /// bounce here preserves the wizard instead of resetting to step 0.
    pub(crate) fn is_on_confirm_step(&self) -> bool {
        self.step == 6
    }

    pub(crate) fn is_gki(&self) -> bool {
        self.mode == Some(RootMode::Gki)
    }
    pub(crate) fn is_forks(&self) -> bool {
        self.provider == Some(Provider::MagiskForks)
    }
    pub(crate) fn is_nightly(&self) -> bool {
        self.version == Some(VerChoice::Nightly)
    }
    pub(crate) fn is_apatch(&self) -> bool {
        self.family == Some(Family::APatch)
    }
    pub(crate) fn is_skroot(&self) -> bool {
        self.family == Some(Family::Skroot)
    }

    pub(crate) fn is_ksu_lkm(&self) -> bool {
        self.family == Some(Family::KernelSU) && self.mode == Some(RootMode::Lkm)
    }

    pub(crate) fn needs_ksu_lkm_kernel_version(&self) -> bool {
        self.is_ksu_lkm() && self.kernel_version.is_none()
    }

    pub(crate) fn active_steps(&self) -> &'static [&'static str] {
        if self.is_skroot() {
            return ROOT_STEPS_SKROOT;
        }
        if self.is_gki() {
            return ROOT_STEPS_GKI;
        }
        let has_modes = self.family.map(|f| f.has_modes()).unwrap_or(false);
        if self.is_forks() {
            return ROOT_STEPS_FORKS;
        }
        if self.is_apatch() {
            // APatch route: Version → KPM → Folder. Superkey popup
            // lives on the KPM→Folder edge, not as its own step.
            return if self.is_nightly() {
                ROOT_STEPS_APATCH_NIGHTLY
            } else {
                ROOT_STEPS_APATCH
            };
        }
        match (has_modes, self.is_nightly()) {
            (true, true) => ROOT_STEPS_NIGHTLY,
            (true, false) => ROOT_STEPS,
            (false, true) => ROOT_STEPS_NOMODE_NIGHTLY,
            (false, false) => ROOT_STEPS_NOMODE,
        }
    }

    pub(crate) fn display_step(&self) -> usize {
        // Map internal step index into the position within the active
        // route's label array. Comments at each branch show the mapping.
        let has_modes = self.family.map(|f| f.has_modes()).unwrap_or(false);
        if self.is_skroot() {
            // 0,1,5,6,7 → 0..4
            return match self.step {
                0 => 0,
                1 => 1,
                5 => 2,
                6 => 3,
                7 => 4,
                _ => self.step,
            };
        }
        if self.is_gki() {
            // 0,1,2,5,6,7 → 0..5
            return match self.step {
                0 => 0,
                1 => 1,
                2 => 2,
                5 => 3,
                6 => 4,
                7 => 5,
                _ => self.step,
            };
        }
        if self.is_forks() {
            // 0,2,3,5,6,7 → 0..5
            return match self.step {
                0 => 0,
                2 => 1,
                3 => 2,
                5 => 3,
                6 => 4,
                7 => 5,
                _ => self.step,
            };
        }
        if self.is_apatch() {
            // Stable: 0,2,3,8,5,6,7 → 0..6. Nightly: add 4 → 0..7.
            if self.is_nightly() {
                return match self.step {
                    0 => 0,
                    2 => 1,
                    3 => 2,
                    4 => 3,
                    8 => 4,
                    5 => 5,
                    6 => 6,
                    7 => 7,
                    _ => self.step,
                };
            }
            return match self.step {
                0 => 0,
                2 => 1,
                3 => 2,
                8 => 3,
                5 => 4,
                6 => 5,
                7 => 6,
                _ => self.step,
            };
        }
        if !has_modes {
            if self.is_nightly() {
                // 0,2,3,4,5,6,7 → 0..6
                return match self.step {
                    0 => 0,
                    2 => 1,
                    3 => 2,
                    4 => 3,
                    5 => 4,
                    6 => 5,
                    7 => 6,
                    _ => self.step,
                };
            }
            // 0,2,3,5,6,7 → 0..5
            return match self.step {
                0 => 0,
                2 => 1,
                3 => 2,
                5 => 3,
                6 => 4,
                7 => 5,
                _ => self.step,
            };
        }
        if self.is_nightly() {
            self.step
        } else {
            // 0,1,2,3,5,6,7 → 0..6
            match self.step {
                5 => 4,
                6 => 5,
                7 => 6,
                s => s,
            }
        }
    }

    pub(crate) fn next(&mut self) {
        match self.step {
            0 => {
                if let Some(f) = self.family
                    && !f.has_modes()
                {
                    self.mode = None;
                    self.step = 2;
                    return;
                }
                self.step = 1;
            }
            1 => {
                if self.is_skroot() {
                    self.step = 5;
                    return;
                }
                self.step = 2;
            }
            2 => {
                if self.is_gki() {
                    self.step = 5;
                    return;
                }
                self.step = 3;
            }
            3 => {
                if self.is_forks() {
                    self.step = 5;
                    return;
                }
                if self.is_nightly() {
                    self.step = 4;
                    return;
                }
                if self.is_apatch() {
                    self.step = 8;
                    return;
                }
                self.step = 5;
            }
            4 => {
                if self.is_apatch() {
                    self.step = 8;
                    return;
                }
                self.step = 5;
            }
            // Exit gated by superkey popup — caller sets step = 5 on confirm.
            8 => self.step = 5,
            5 => self.step = 6,
            6 => self.step = 7,
            _ => {}
        }
    }

    pub(crate) fn back(&mut self) {
        match self.step {
            1 => self.step = 0,
            2 => {
                if let Some(f) = self.family
                    && !f.has_modes()
                {
                    self.step = 0;
                    return;
                }
                self.step = 1;
            }
            3 => self.step = 2,
            4 => self.step = 3,
            5 => {
                // Folder → whichever sub-step populated the source.
                if self.is_skroot() {
                    self.step = 1;
                    return;
                }
                if self.is_gki() {
                    self.step = 2;
                    return;
                }
                if self.is_forks() {
                    self.step = 3;
                    return;
                }
                if self.is_apatch() {
                    self.step = 8;
                    return;
                }
                if self.is_nightly() {
                    self.step = 4;
                    return;
                }
                self.step = 3;
            }
            6 => self.step = 5,
            7 => self.step = 6,
            8 => {
                self.step = if self.is_nightly() { 4 } else { 3 };
            }
            _ => {}
        }
    }

    pub(crate) fn can_next(&self) -> bool {
        match self.step {
            0 => self.family.is_some(),
            1 => {
                if self.is_skroot() {
                    return self.skroot_flavor == Some(SkrootFlavor::Lite);
                }
                self.mode.is_some()
            }
            2 => {
                if self.is_gki() {
                    self.file_path.is_some()
                } else {
                    self.provider.is_some()
                }
            }
            3 => {
                if self.is_forks() {
                    self.file_path.is_some()
                } else {
                    self.version.is_some()
                }
            }
            4 => match self.nightly_source {
                // ManualInput also needs the popup's run ID committed.
                Some(NightlySource::AutoDetect) => true,
                Some(NightlySource::ManualInput) => {
                    self.run_id.as_deref().is_some_and(|s| !s.is_empty())
                }
                None => false,
            },
            5 => self.folder_path.is_some(),
            6 => true,
            // KPM embedding is optional — the actual gate is the
            // superkey popup on Next.
            8 => true,
            _ => false,
        }
    }
}

/// Linear-step wizard contract. Wizards whose `next` / `back` simply
/// walk a 0..step_count range share `reset` / `next` / `back` /
/// `is_in_exec` via this trait's default impls; only `step`,
/// `step_mut`, `step_count`, and `can_next` need per-impl bodies.
///
/// Not implemented for `RootWizard` because its non-linear step
/// numbering (steps skip around depending on family/mode) requires
/// custom navigation logic.
pub(crate) trait Wizard: Default {
    fn step(&self) -> usize;
    fn step_mut(&mut self) -> &mut usize;
    fn step_count(&self) -> usize;
    fn can_next(&self) -> bool;

    fn reset(&mut self) {
        *self = Self::default();
    }
    fn next(&mut self) {
        if self.step() < self.step_count() - 1 {
            *self.step_mut() += 1;
        }
    }
    fn back(&mut self) {
        if self.step() > 0 {
            *self.step_mut() -= 1;
        }
    }
    fn is_in_exec(&self) -> bool {
        self.step() == self.step_count() - 1
    }
    /// True on the confirm/start screen — the step immediately before
    /// exec. A sidebar bounce here preserves the wizard (the user returns
    /// to the confirm screen) instead of resetting to step 0.
    fn is_on_confirm_step(&self) -> bool {
        let n = self.step_count();
        n >= 2 && self.step() == n - 2
    }
}

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
    /// Loader file (`xbl_s_devprg_ns.melf`) for the EDL flash. Has
    /// its own wizard step. The Settings-level default loader
    /// auto-fills + auto-advances the loader step on Next from the
    /// method step (mirrors the Root wizard's step-5 fold-through);
    /// anyone without a default sees the explicit loader picker.
    pub(crate) loader_path: Option<String>,
    /// Loader-resolution failure, kept apart from any scan error so a
    /// refused pick does not overwrite why the last scan failed.
    pub(crate) loader_error: Option<String>,
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
        // Step indexes match `UNROOT_STEPS` — loader is its own step
        // (#1) so the folder step (#2) only gates on the backup folder
        // pick and doesn't have to bundle a loader sub-row.
        match self.step {
            0 => self.unroot_type.is_some(),
            1 => self.loader_path.is_some() && self.loader_error.is_none(),
            2 => self.folder_path.is_some(),
            3 => true,
            _ => false,
        }
    }
}

// =========================================================================
// Flash wizard state
// =========================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeviceRegion {
    Prc,
    Row,
}
impl DeviceRegion {
    pub(crate) fn label_key(&self) -> &'static str {
        match self {
            Self::Prc => "deviceregion_prc",
            Self::Row => "deviceregion_row",
        }
    }

    pub(crate) fn to_region_target(self) -> ltbox_patch::region::RegionTarget {
        match self {
            Self::Prc => ltbox_patch::region::RegionTarget::Prc,
            Self::Row => ltbox_patch::region::RegionTarget::Row,
        }
    }
}

/// Selection state for the Flash wizard's region step. Automatic lookup is a
/// method for resolving a [`DeviceRegion`], never a region passed downstream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FlashRegionSelection {
    Auto,
    Manual(DeviceRegion),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FlashTarget {
    OtherRegion,
    SameRegion,
}
impl FlashTarget {
    pub(crate) fn label_key(&self) -> &'static str {
        match self {
            Self::OtherRegion => "flashtarget_other",
            Self::SameRegion => "flashtarget_same",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DataMode {
    Keep,
    Wipe,
}
impl DataMode {
    pub(crate) fn label_key(&self) -> &'static str {
        match self {
            Self::Keep => "datamode_keep",
            Self::Wipe => "datamode_wipe",
        }
    }
}

/// Which Flash-confirm summary row the "hidden dropdown" editor targets.
/// `Country` is special-cased to reuse the existing country popup; the
/// rest open the shared `flash_confirm_edit_popup`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConfirmField {
    Region,
    Target,
    Data,
    RegionEdit,
    Rollback,
    Country,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FlashStep {
    Region,
    Target,
    Data,
    Folder,
    Bootloader,
    Confirm,
    Flash,
}

impl FlashStep {
    pub(crate) fn label_key(self) -> &'static str {
        match self {
            Self::Region => "flash_step_region",
            Self::Target => "flash_step_target",
            Self::Data => "flash_step_data",
            Self::Folder => "flash_step_folder",
            Self::Bootloader => "flash_step_bootloader",
            Self::Confirm => "flash_step_confirm",
            Self::Flash => "flash_step_flash",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FirmwareIdentity {
    pub(crate) efisp_load: ltbox_patch::efisp_load::EfispLoad,
    pub(crate) key_class: ltbox_patch::key_map::KeyClass,
    pub(crate) fingerprint: Option<String>,
    pub(crate) model_token: Option<String>,
}

impl FirmwareIdentity {
    pub(crate) fn uses_gbl(&self) -> bool {
        self.fingerprint.as_deref().is_some_and(|fp| {
            ltbox_core::model::fingerprint_capabilities(fp).any(|caps| caps.root_uses_gbl)
        }) || self
            .model_token
            .as_deref()
            .is_some_and(|model| ltbox_core::model::capabilities(model).root_uses_gbl)
    }

    pub(crate) fn needs_bootloader_step(&self) -> bool {
        if self.uses_gbl() {
            self.efisp_load != ltbox_patch::efisp_load::EfispLoad::Yes
        } else {
            firmware_needs_bootloader_step(
                self.key_class,
                self.fingerprint.as_deref(),
                self.efisp_load,
            )
        }
    }

    pub(crate) fn from_avb_info(info: &ltbox_patch::avb::AvbImageInfo) -> Self {
        let fingerprint = ltbox_patch::avb::build_fingerprint(info);
        let model_token = fingerprint.as_deref().and_then(fingerprint_model_token);
        Self {
            key_class: ltbox_patch::key_map::classify_pubkey(info.public_key_sha1.as_deref()),
            efisp_load: ltbox_patch::efisp_load::EfispLoad::Undetermined,
            fingerprint,
            model_token,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FirmwareIdentityDialog {
    Ready,
    Failed(String),
}

fn fingerprint_model_token(fingerprint: &str) -> Option<String> {
    fingerprint
        .split('/')
        .nth(1)
        .and_then(|product| product.split('_').next())
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
}

pub(crate) fn firmware_needs_bootloader_step(
    key_class: ltbox_patch::key_map::KeyClass,
    fingerprint: Option<&str>,
    efisp_load: ltbox_patch::efisp_load::EfispLoad,
) -> bool {
    if fingerprint.is_some_and(|fp| {
        ltbox_core::model::fingerprint_capabilities(fp).any(|caps| caps.root_uses_gbl)
    }) {
        return efisp_load != ltbox_patch::efisp_load::EfispLoad::Yes;
    }
    key_class == ltbox_patch::key_map::KeyClass::Lenovo
        && !fingerprint.is_some_and(|fp| {
            ltbox_core::model::fingerprint_capabilities(fp).any(|caps| {
                matches!(
                    caps.rollback,
                    ltbox_core::model::RollbackPolicy::Gbl
                        | ltbox_core::model::RollbackPolicy::ReadOnly
                )
            })
        })
}

pub(crate) const FLASH_STEPS: &[FlashStep] = &[
    FlashStep::Region,
    FlashStep::Target,
    FlashStep::Data,
    FlashStep::Folder,
    FlashStep::Confirm,
    FlashStep::Flash,
];

const FLASH_STEPS_WITH_BOOTLOADER: &[FlashStep] = &[
    FlashStep::Region,
    FlashStep::Target,
    FlashStep::Data,
    FlashStep::Folder,
    FlashStep::Bootloader,
    FlashStep::Confirm,
    FlashStep::Flash,
];

#[derive(Default)]
pub(crate) struct FlashWizard {
    pub(crate) step: usize,
    pub(crate) region_selection: Option<FlashRegionSelection>,
    pub(crate) device_region: Option<DeviceRegion>,
    pub(crate) region_auto_unknown: bool,
    pub(crate) target: Option<FlashTarget>,
    pub(crate) data_mode: Option<DataMode>,
    pub(crate) firmware_folder: Option<String>,
    /// Original `boot` / `vbmeta_system` rollback indices read from the
    /// selected firmware. Missing entries retain their reason for display.
    pub(crate) firmware_rollback_indices: Option<(Result<u64, String>, Result<u64, String>)>,
    /// `true` when the selected firmware folder ships no EDL loader, so the
    /// folder step requires a separately-picked loader before advancing.
    pub(crate) loader_required: bool,
    /// User-picked EDL loader (or the resolved Settings default) used when the
    /// firmware folder has none. `None` + `loader_required` blocks Next.
    pub(crate) loader_override: Option<String>,
    /// Reason the last picked loader was rejected (e.g. a standalone `.melf` on
    /// TB323FU), shown in the folder step.
    pub(crate) loader_error: Option<String>,
    pub(crate) firmware_identity: Option<FirmwareIdentity>,
    pub(crate) firmware_identity_pending: bool,
    pub(crate) firmware_identity_dialog: Option<FirmwareIdentityDialog>,
    pub(crate) user_abl_path: Option<String>,
    pub(crate) user_abl_key_class: Option<ltbox_patch::key_map::KeyClass>,
    pub(crate) user_abl_analyzing: bool,
    pub(crate) user_abl_efisp_load: ltbox_patch::efisp_load::EfispLoad,
    pub(crate) no_efisp_load: bool,
}

impl FlashWizard {
    pub(crate) fn set_firmware_rollback_indices(&mut self, folder: &str) {
        let read = |filename: &str| -> Result<u64, String> {
            ltbox_patch::avb::extract_image_avb_info(
                std::path::Path::new(folder).join(filename).as_path(),
            )
            .map(|info| info.rollback_index)
            .map_err(|error| error.to_string())
        };
        self.firmware_rollback_indices = Some((read("boot.img"), read("vbmeta_system.img")));
    }

    pub(crate) fn visible_steps(&self) -> &'static [FlashStep] {
        if self
            .firmware_identity
            .as_ref()
            .is_some_and(|identity| identity.needs_bootloader_step())
        {
            FLASH_STEPS_WITH_BOOTLOADER
        } else {
            FLASH_STEPS
        }
    }

    pub(crate) fn current_step(&self) -> FlashStep {
        self.visible_steps()
            .get(self.step)
            .copied()
            .unwrap_or(FlashStep::Flash)
    }

    pub(crate) fn set_step(&mut self, step: FlashStep) {
        self.step = self
            .visible_steps()
            .iter()
            .position(|candidate| *candidate == step)
            .unwrap_or(0);
    }

    pub(crate) fn reset_firmware_identity(&mut self) {
        self.firmware_identity = None;
        self.firmware_identity_pending = false;
        self.firmware_identity_dialog = None;
        self.clear_bootloader();
    }

    pub(crate) fn clear_bootloader(&mut self) {
        self.user_abl_path = None;
        self.user_abl_key_class = None;
        self.user_abl_analyzing = false;
        self.user_abl_efisp_load = ltbox_patch::efisp_load::EfispLoad::Undetermined;
        self.no_efisp_load = false;
    }

    pub(crate) fn uses_gbl(&self) -> bool {
        self.firmware_identity
            .as_ref()
            .is_some_and(FirmwareIdentity::uses_gbl)
    }

    pub(crate) fn bootloader_can_next(&self) -> bool {
        if self.user_abl_analyzing {
            return false;
        }
        if self.uses_gbl() {
            match self.user_abl_path {
                Some(_) => self.user_abl_efisp_load == ltbox_patch::efisp_load::EfispLoad::Yes,
                None => self.firmware_identity.as_ref().is_some_and(|identity| {
                    identity.efisp_load != ltbox_patch::efisp_load::EfispLoad::Undetermined
                }),
            }
        } else {
            self.user_abl_path.is_none()
                || self.user_abl_key_class == Some(ltbox_patch::key_map::KeyClass::Testkey)
        }
    }

    pub(crate) fn record_bootloader_decision(&mut self) {
        self.no_efisp_load = self.uses_gbl()
            && self.user_abl_path.is_none()
            && self.firmware_identity.as_ref().is_some_and(|identity| {
                identity.efisp_load == ltbox_patch::efisp_load::EfispLoad::No
            });
    }

    pub(crate) fn bootloader_execution_allowed(&self) -> bool {
        self.bootloader_can_next()
            && (!self.uses_gbl()
                || self.user_abl_path.is_some()
                || self.firmware_identity.as_ref().is_some_and(|identity| {
                    identity.efisp_load == ltbox_patch::efisp_load::EfispLoad::Yes
                        || (identity.efisp_load == ltbox_patch::efisp_load::EfispLoad::No
                            && self.no_efisp_load)
                }))
    }
}

impl Wizard for FlashWizard {
    fn step(&self) -> usize {
        self.step
    }
    fn step_mut(&mut self) -> &mut usize {
        &mut self.step
    }
    fn step_count(&self) -> usize {
        self.visible_steps().len()
    }
    fn can_next(&self) -> bool {
        match self.current_step() {
            FlashStep::Region => match self.region_selection {
                Some(FlashRegionSelection::Auto) => true,
                Some(FlashRegionSelection::Manual(region)) => self.device_region == Some(region),
                None => false,
            },
            FlashStep::Target => self.target.is_some(),
            FlashStep::Data => self.data_mode.is_some(),
            // Folder picked, and — when it ships no loader — a loader provided.
            FlashStep::Folder => {
                self.firmware_folder.is_some()
                    && (!self.loader_required || self.loader_override.is_some())
                    && !self.firmware_identity_pending
            }
            FlashStep::Bootloader => self.bootloader_can_next(),
            FlashStep::Confirm => {
                self.firmware_identity.is_some()
                    && self.bootloader_execution_allowed()
                    && self.firmware_folder.is_some()
                    && (!self.loader_required || self.loader_override.is_some())
            }
            FlashStep::Flash => false,
        }
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

// =========================================================================
// System Update wizard state
// =========================================================================

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
    /// Rescue: firmware folder containing loader (`xbl_s_devprg_ns.melf`).
    pub(crate) rescue_folder: Option<String>,
    /// Rescue: selected target region. Set via popup between Folder and
    /// Confirm steps. May be pre-seeded from `inferred_flash_region`
    /// (PTSTPD `SaleArea`) before the popup opens — `rescue_region_confirmed`
    /// tracks whether the user explicitly clicked through.
    pub(crate) rescue_region: Option<RescueRegion>,
    /// Rescue: region popup overlay flag. Opens on Next press from the
    /// Folder step when the user hasn't yet confirmed a region pick.
    pub(crate) rescue_region_popup_open: bool,
    /// Rescue: true once the user has clicked a region radio in the
    /// popup. Distinguishes a pre-seeded `rescue_region` (initial
    /// preselect from `inferred_flash_region`) from a user-confirmed
    /// pick — preselect alone shouldn't skip the popup.
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
    /// Rescue gets an extra Folder step — distinct step list keeps the
    /// other actions (Disable/Enable) on their short 3-step flow.
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
            // Rescue flow: Action → Folder → Confirm → Exec.
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

/// The only three actions a partition row can represent.
///
/// `Write` always owns a selected file, while `Skip` and `Erase` never do.
/// Keeping that invariant in [`FlashPartRow`] prevents stale images from being
/// written after a user changes a row to erase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum FlashRowState {
    #[default]
    Skip,
    Write,
    Erase,
}

/// One GPT entry surfaced in the wizard table.
#[derive(Debug, Clone)]
pub(crate) struct FlashPartRow {
    pub(crate) lun: u8,
    pub(crate) label: String,
    pub(crate) start_sector: u64,
    pub(crate) num_sectors: u64,
    pub(crate) size_bytes: u64,
    pub(crate) file_path: Option<String>,
    pub(crate) state: FlashRowState,
}

impl FlashPartRow {
    /// Assigning an image is the sole transition into `Write`.
    pub(crate) fn assign_file(&mut self, path: String) {
        self.file_path = Some(path);
        self.state = FlashRowState::Write;
    }

    /// Removing an assigned image returns the row to the assignable skip state.
    pub(crate) fn clear_file(&mut self) {
        self.file_path = None;
        self.state = FlashRowState::Skip;
    }

    /// Advance an already-actionable row. Entering erase always drops its file.
    pub(crate) fn advance_action(&mut self) {
        match self.state {
            FlashRowState::Skip => {}
            FlashRowState::Write => {
                self.file_path = None;
                self.state = FlashRowState::Erase;
            }
            FlashRowState::Erase => {
                self.file_path = None;
                self.state = FlashRowState::Skip;
            }
        }
    }
}

#[cfg(test)]
mod flash_part_row_tests {
    use super::*;

    fn row() -> FlashPartRow {
        FlashPartRow {
            lun: 0,
            label: "userdata".to_string(),
            start_sector: 0,
            num_sectors: 1,
            size_bytes: 512,
            file_path: None,
            state: FlashRowState::Skip,
        }
    }

    #[test]
    fn assignment_and_action_transitions_preserve_file_invariants() {
        let mut row = row();
        row.assign_file("userdata.img".to_string());
        assert_eq!(row.state, FlashRowState::Write);
        assert_eq!(row.file_path.as_deref(), Some("userdata.img"));

        row.advance_action();
        assert_eq!(row.state, FlashRowState::Erase);
        assert_eq!(row.file_path, None);

        row.advance_action();
        assert_eq!(row.state, FlashRowState::Skip);
        assert_eq!(row.file_path, None);
    }

    #[test]
    fn clearing_any_selected_file_returns_to_skip() {
        let mut row = row();
        row.assign_file("boot.img".to_string());
        row.clear_file();
        assert_eq!(row.state, FlashRowState::Skip);
        assert_eq!(row.file_path, None);
    }
}

/// Column the partition table is currently sorted by. Header click
/// fires `*SortBy(col)`; clicking the active column toggles direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum PartsSortColumn {
    #[default]
    Lun,
    Label,
    Start,
    Size,
    /// File-path column — only meaningful for FlashParts; DumpParts has
    /// no file-path column so this variant is never produced from its
    /// header buttons.
    File,
}

#[derive(Default)]
pub(crate) struct FlashPartsWizard {
    pub(crate) step: usize, // 0=Loader, 1=Select, 2=Confirm, 3=Exec
    pub(crate) loader_path: Option<String>,
    /// Loader-resolution failure, kept apart from any scan error so a
    /// refused pick does not overwrite why the last scan failed.
    pub(crate) loader_error: Option<String>,
    pub(crate) rows: Vec<FlashPartRow>,
    pub(crate) scanning: bool,
    pub(crate) scan_error: Option<String>,
    /// Connection state captured when the GPT scan started. The table-step
    /// leading action must not infer this from the live post-scan EDL state.
    pub(crate) entry_connection: Option<ConnectionStatus>,
    pub(crate) sort_col: PartsSortColumn,
    /// `true` → descending. Default `false` (ascending) on first scan
    /// so initial layout matches the device's GPT order well enough
    /// for LUN-then-label browsing.
    pub(crate) sort_desc: bool,
}

pub(crate) const FLASH_PARTS_STEPS: &[&str] = &[
    "edl_loader_label",
    "flash_parts_step_select",
    "flash_step_confirm",
    "flash_step_flash",
];

impl FlashPartsWizard {
    pub(crate) fn active_rows(&self) -> Vec<FlashPartRow> {
        self.rows
            .iter()
            .filter(|r| match r.state {
                FlashRowState::Write => r.file_path.is_some(),
                FlashRowState::Erase => true,
                FlashRowState::Skip => false,
            })
            .cloned()
            .collect()
    }

    /// Stable-sort `rows` by current `sort_col` / `sort_desc`. Tie-break
    /// on (lun, label) so identical primary keys land in a deterministic
    /// order.
    pub(crate) fn apply_sort(&mut self) {
        let col = self.sort_col;
        let desc = self.sort_desc;
        self.rows.sort_by(|a, b| {
            let ord = match col {
                PartsSortColumn::Lun => a.lun.cmp(&b.lun),
                // ASCII byte order — uppercase (A-Z, 0x41-0x5A) sorts
                // before lowercase (a-z, 0x61-0x7A) by user request.
                PartsSortColumn::Label => a.label.cmp(&b.label),
                PartsSortColumn::Start => a.start_sector.cmp(&b.start_sector),
                PartsSortColumn::Size => a.size_bytes.cmp(&b.size_bytes),
                PartsSortColumn::File => a
                    .file_path
                    .as_deref()
                    .unwrap_or("")
                    .cmp(b.file_path.as_deref().unwrap_or("")),
            };
            let ord = ord
                .then_with(|| a.lun.cmp(&b.lun))
                .then_with(|| a.label.cmp(&b.label));
            if desc { ord.reverse() } else { ord }
        });
    }

    /// Header click: toggle direction on the active column, otherwise
    /// switch to the new column ascending.
    pub(crate) fn toggle_sort(&mut self, col: PartsSortColumn) {
        if self.sort_col == col {
            self.sort_desc = !self.sort_desc;
        } else {
            self.sort_col = col;
            self.sort_desc = false;
        }
        self.apply_sort();
    }
}

impl Wizard for FlashPartsWizard {
    fn step(&self) -> usize {
        self.step
    }
    fn step_mut(&mut self) -> &mut usize {
        &mut self.step
    }
    fn step_count(&self) -> usize {
        FLASH_PARTS_STEPS.len()
    }
    fn can_next(&self) -> bool {
        match self.step {
            0 => self.loader_path.is_some() && self.loader_error.is_none() && !self.scanning,
            1 => self.rows.iter().any(|r| match r.state {
                FlashRowState::Write => r.file_path.is_some(),
                FlashRowState::Erase => true,
                FlashRowState::Skip => false,
            }),
            2 => true,
            _ => false,
        }
    }
}

/// Scan-phase result carried in a single message. Same shape as the
/// DumpParts variant but with the Flash row type.
#[derive(Debug, Clone, Default)]
pub(crate) struct FlashPartsScanResult {
    pub(crate) logs: Vec<String>,
    pub(crate) rows: Vec<FlashPartRow>,
    pub(crate) error: Option<String>,
}

// =========================================================================
// Dump Partitions wizard state (Advanced → Dump Partitions)
// =========================================================================

#[derive(Debug, Clone)]
pub(crate) struct DumpPartRow {
    pub(crate) lun: u8,
    pub(crate) label: String,
    pub(crate) start_sector: u64,
    pub(crate) num_sectors: u64,
    pub(crate) size_bytes: u64,
    pub(crate) selected: bool,
}

/// Scan-phase result carried in a single message.
#[derive(Debug, Clone, Default)]
pub(crate) struct DumpPartsScanResult {
    pub(crate) logs: Vec<String>,
    pub(crate) rows: Vec<DumpPartRow>,
    pub(crate) error: Option<String>,
}

#[derive(Default)]
pub(crate) struct DumpPartsWizard {
    pub(crate) step: usize, // 0=Loader, 1=Select, 2=Exec
    pub(crate) loader_path: Option<String>,
    /// Loader-resolution failure, kept apart from any scan error so a
    /// refused pick does not overwrite why the last scan failed.
    pub(crate) loader_error: Option<String>,
    pub(crate) rows: Vec<DumpPartRow>,
    pub(crate) output_dir: Option<String>,
    pub(crate) scanning: bool,
    pub(crate) scan_error: Option<String>,
    /// Connection state captured when the GPT scan started. The table-step
    /// leading action must not infer this from the live post-scan EDL state.
    pub(crate) entry_connection: Option<ConnectionStatus>,
    pub(crate) sort_col: PartsSortColumn,
    pub(crate) sort_desc: bool,
}

pub(crate) const DUMP_PARTS_STEPS: &[&str] = &[
    "edl_loader_label",
    "dump_parts_step_select",
    "dump_parts_step_dump",
];

impl DumpPartsWizard {
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }
    pub(crate) fn back(&mut self) {
        if self.step > 0 {
            self.step -= 1;
        }
    }
    pub(crate) fn can_next(&self) -> bool {
        match self.step {
            0 => self.loader_path.is_some() && self.loader_error.is_none() && !self.scanning,
            1 => self.rows.iter().any(|r| r.selected),
            _ => false,
        }
    }
    pub(crate) fn selected_rows(&self) -> Vec<DumpPartRow> {
        self.rows.iter().filter(|r| r.selected).cloned().collect()
    }

    pub(crate) fn apply_sort(&mut self) {
        let col = self.sort_col;
        let desc = self.sort_desc;
        self.rows.sort_by(|a, b| {
            let ord = match col {
                PartsSortColumn::Lun => a.lun.cmp(&b.lun),
                // ASCII byte order — uppercase (A-Z, 0x41-0x5A) sorts
                // before lowercase (a-z, 0x61-0x7A) by user request.
                PartsSortColumn::Label => a.label.cmp(&b.label),
                PartsSortColumn::Start => a.start_sector.cmp(&b.start_sector),
                PartsSortColumn::Size => a.size_bytes.cmp(&b.size_bytes),
                // DumpParts has no file column; behave as Lun fallback.
                PartsSortColumn::File => a.lun.cmp(&b.lun),
            };
            let ord = ord
                .then_with(|| a.lun.cmp(&b.lun))
                .then_with(|| a.label.cmp(&b.label));
            if desc { ord.reverse() } else { ord }
        });
    }

    pub(crate) fn toggle_sort(&mut self, col: PartsSortColumn) {
        if self.sort_col == col {
            self.sort_desc = !self.sort_desc;
        } else {
            self.sort_col = col;
            self.sort_desc = false;
        }
        self.apply_sort();
    }
}

// =========================================================================
// Physical Storage wizards (Advanced → Dump/Flash Physical)
//
// LUN-level counterparts to the partition wizards. No GPT scan — the
// user picks which of LUN 0..=5 to hit, and the exec pass reads/writes
// the whole LUN. Mirrors qdlrs `Dump` (whole-disk variant) and
// `OverwriteStorage` commands.
// =========================================================================

pub(crate) const PHYS_LUN_COUNT: usize = 6;

#[derive(Default)]
pub(crate) struct DumpPhysWizard {
    pub(crate) step: usize, // 0=Loader, 1=Select, 2=Exec
    pub(crate) loader_path: Option<String>,
    pub(crate) selected: [bool; PHYS_LUN_COUNT],
    pub(crate) output_dir: Option<String>,
    pub(crate) loader_error: Option<String>,
}

pub(crate) const DUMP_PHYS_STEPS: &[&str] = &[
    "edl_loader_label",
    "phys_step_select",
    "dump_parts_step_dump",
];

impl DumpPhysWizard {
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }
    pub(crate) fn back(&mut self) {
        if self.step > 0 {
            self.step -= 1;
        }
    }
    pub(crate) fn can_next(&self) -> bool {
        match self.step {
            0 => self.loader_path.is_some() && self.loader_error.is_none(),
            1 => self.selected.iter().any(|&s| s),
            _ => false,
        }
    }
    pub(crate) fn selected_luns(&self) -> Vec<u8> {
        self.selected
            .iter()
            .enumerate()
            .filter_map(|(i, &s)| if s { Some(i as u8) } else { None })
            .collect()
    }
}

#[derive(Default)]
pub(crate) struct FlashPhysWizard {
    pub(crate) step: usize, // 0=Loader, 1=Select, 2=Confirm, 3=Exec
    pub(crate) loader_path: Option<String>,
    pub(crate) selected: [bool; PHYS_LUN_COUNT],
    pub(crate) file_paths: [Option<String>; PHYS_LUN_COUNT],
    pub(crate) loader_error: Option<String>,
}

pub(crate) const FLASH_PHYS_STEPS: &[&str] = &[
    "edl_loader_label",
    "phys_step_select",
    "flash_step_confirm",
    "flash_step_flash",
];

impl FlashPhysWizard {
    /// (LUN, file_path) pairs for every selected, file-bound row.
    pub(crate) fn active_pairs(&self) -> Vec<(u8, String)> {
        (0..PHYS_LUN_COUNT)
            .filter_map(|i| {
                if self.selected[i] {
                    self.file_paths[i].clone().map(|p| (i as u8, p))
                } else {
                    None
                }
            })
            .collect()
    }
}

impl Wizard for FlashPhysWizard {
    fn step(&self) -> usize {
        self.step
    }
    fn step_mut(&mut self) -> &mut usize {
        &mut self.step
    }
    fn step_count(&self) -> usize {
        FLASH_PHYS_STEPS.len()
    }
    fn can_next(&self) -> bool {
        match self.step {
            0 => self.loader_path.is_some() && self.loader_error.is_none(),
            // At least one row selected AND every selected row has a file.
            1 => {
                let any = self.selected.iter().any(|&s| s);
                let all_have_file = self
                    .selected
                    .iter()
                    .zip(self.file_paths.iter())
                    .all(|(&s, f)| !s || f.is_some());
                any && all_have_file
            }
            2 => true,
            _ => false,
        }
    }
}

// =========================================================================
// Simple Firmware Flash wizard state (Advanced → EDL ops)
// =========================================================================

/// Minimal flash wizard for the "Firmware Simple Flasher" advanced op: pick a
/// firmware folder, review a read-only confirm screen, flash. No region /
/// rollback / data choices — the flash runs the
/// firmware's own rawprogram verbatim.
#[derive(Default)]
pub(crate) struct SimpleFlashWizard {
    pub(crate) step: usize, // 0=Intro, 1=Confirm, 2=Exec
    pub(crate) firmware_folder: Option<String>,
}

pub(crate) const SIMPLE_FLASH_STEPS: &[&str] =
    &["adv_step_source", "flash_step_confirm", "flash_step_flash"];

impl Wizard for SimpleFlashWizard {
    fn step(&self) -> usize {
        self.step
    }
    fn step_mut(&mut self) -> &mut usize {
        &mut self.step
    }
    fn step_count(&self) -> usize {
        SIMPLE_FLASH_STEPS.len()
    }
    fn can_next(&self) -> bool {
        // Source (0): require a firmware folder. Confirm (1): Start.
        // Exec (2) has no Next.
        match self.step {
            0 => self.firmware_folder.is_some(),
            1 => true,
            _ => false,
        }
    }
}

// =========================================================================
// KonaBess GPU-table wizard state
// =========================================================================

pub(crate) const KONABESS_STEPS: &[&str] = &[
    "edl_loader_label",
    "konabess_step_table",
    "konabess_step_confirm",
    "konabess_step_apply",
];

/// Device state retained across the inspection worker's UI selection pause.
/// Part 2 can consume these exact stock images and the already-resolved slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KonaBessPrepared {
    pub(crate) work_dir: std::path::PathBuf,
    pub(crate) vendor_boot: std::path::PathBuf,
    pub(crate) vbmeta: std::path::PathBuf,
    pub(crate) backup_dir: std::path::PathBuf,
    pub(crate) slot_suffix: String,
    /// Android's best-effort `ro.boot.dtb_idx` hint, captured before EDL.
    pub(crate) probable_dtb_index: Option<usize>,
}

/// KonaBess wizard state. The prepared workspace is populated only after the
/// non-destructive device inspection and remains live through target selection.
#[derive(Debug, Clone, Default)]
pub(crate) struct KonaBessWizard {
    pub(crate) step: usize,
    pub(crate) loader_path: Option<String>,
    pub(crate) loader_error: Option<String>,
    pub(crate) import_path: Option<String>,
    pub(crate) import_error: Option<String>,
    pub(crate) import_warnings: Vec<GpuTableIssue>,
    /// Device table retained for comparison and one-click revert.
    pub(crate) stock_table: Option<GpuTable>,
    /// In-memory table that will be passed directly to the AVB build path.
    pub(crate) edited_table: Option<GpuTable>,
    pub(crate) edited_dirty: bool,
    /// User-entered text is retained independently from the last parseable
    /// value committed to `edited_table`, so partial input never snaps back.
    pub(crate) cell_edits: BTreeMap<GpuCellKey, GpuCellEdit>,
    /// DTBs whose GPU table parsed from the dumped vendor_boot image.
    pub(crate) candidates: Vec<VendorBootDtbInfo>,
    /// The one DTB index passed to the existing single-target patch API.
    pub(crate) selected_target_index: Option<usize>,
    /// Upstream KonaBess's probable target, when it is one of the candidates.
    pub(crate) probable_target_index: Option<usize>,
    /// Modal ownership stays with the wizard rather than parallel App flags.
    pub(crate) target_popup_open: bool,
    /// Initial selection is part of the inspection pause; cancelling it abandons
    /// the prepared device flow. Reopening the picker from the table does not.
    pub(crate) target_popup_abandons_on_dismiss: bool,
    pub(crate) prepared: Option<KonaBessPrepared>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum GpuCellLocation {
    Level { level: usize, property: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GpuPropertyLocation {
    GroupHeader,
    Level,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GpuPropertyEditability {
    ReadOnly,
    Editable,
}

pub(crate) fn gpu_property_editability(
    location: GpuPropertyLocation,
    property_name: &str,
) -> GpuPropertyEditability {
    match location {
        GpuPropertyLocation::GroupHeader => GpuPropertyEditability::ReadOnly,
        GpuPropertyLocation::Level if property_name == "reg" => GpuPropertyEditability::ReadOnly,
        GpuPropertyLocation::Level => GpuPropertyEditability::Editable,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct GpuCellKey {
    pub(crate) group: usize,
    pub(crate) location: GpuCellLocation,
    pub(crate) cell: usize,
}

impl GpuCellKey {
    pub(crate) const fn level(group: usize, level: usize, property: usize, cell: usize) -> Self {
        Self {
            group,
            location: GpuCellLocation::Level { level, property },
            cell,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GpuCellEdit {
    pub(crate) text: String,
    pub(crate) has_error: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum KonaBessImportError {
    NoTarget,
    TargetChipUnknown,
    ChipMismatch { expected: String, actual: String },
}

impl KonaBessWizard {
    /// Accept an inspection result and require explicit single-target selection.
    /// Candidates are driven solely by GPU tables parsed from the device image.
    pub(crate) fn apply_inspection_result(
        &mut self,
        inspected: Vec<VendorBootDtbInfo>,
        probable_dtb_index: Option<usize>,
    ) {
        self.candidates = inspected
            .into_iter()
            .filter(|candidate| candidate.table.is_some())
            .collect();
        self.probable_target_index = probable_dtb_index.filter(|index| {
            self.candidates
                .iter()
                .any(|candidate| candidate.index == *index)
        });
        self.clear_table_selection();
        self.target_popup_open = true;
        self.target_popup_abandons_on_dismiss = true;
    }

    pub(crate) fn is_probable_target(&self, target_index: usize) -> bool {
        self.probable_target_index == Some(target_index)
    }

    /// Select one candidate by its stable vendor_boot DTB index.
    pub(crate) fn select_target(&mut self, target_index: usize) -> bool {
        let Some(table) = self
            .candidates
            .iter()
            .find(|candidate| candidate.index == target_index && candidate.chip.is_some())
            .and_then(|candidate| candidate.table.clone())
        else {
            return false;
        };
        self.selected_target_index = Some(target_index);
        self.edited_table = Some(table.clone());
        self.stock_table = Some(table);
        self.cell_edits.clear();
        self.edited_dirty = false;
        self.import_path = None;
        self.import_error = None;
        self.import_warnings.clear();
        true
    }

    /// Confirm target selection. The popup remains open until one DTB is set.
    pub(crate) fn confirm_target(&mut self) -> Option<usize> {
        let selected = self.selected_target_index?;
        self.stock_table.as_ref()?;
        self.edited_table.as_ref()?;
        self.target_popup_open = false;
        self.target_popup_abandons_on_dismiss = false;
        Some(selected)
    }

    /// Dismissal is a non-error state; a later Apply inspection can reopen it.
    pub(crate) fn dismiss_target_popup(&mut self) -> bool {
        let abandons = self.target_popup_abandons_on_dismiss;
        self.target_popup_open = false;
        self.target_popup_abandons_on_dismiss = false;
        abandons
    }

    pub(crate) fn open_target_popup(&mut self) {
        self.target_popup_open = true;
        self.target_popup_abandons_on_dismiss = false;
    }

    pub(crate) fn selected_target(&self) -> Option<&VendorBootDtbInfo> {
        let index = self.selected_target_index?;
        self.candidates
            .iter()
            .find(|candidate| candidate.index == index)
    }

    pub(crate) fn selected_chip(&self) -> Option<&str> {
        self.selected_target()?.chip.as_deref()
    }

    pub(crate) fn overwrite_edited_from_import(
        &mut self,
        export: KonaBessExport,
    ) -> Result<(), KonaBessImportError> {
        let Some(target) = self.selected_target() else {
            return Err(KonaBessImportError::NoTarget);
        };
        let Some(expected) = target.chip.as_deref() else {
            return Err(KonaBessImportError::TargetChipUnknown);
        };
        if !chip_names_match(&export.chip, expected) {
            return Err(KonaBessImportError::ChipMismatch {
                expected: expected.to_string(),
                actual: export.chip,
            });
        }
        let mut table = export.table;
        if let Some(stock) = self.stock_table.as_ref() {
            // KonaBess profiles provide editable level data. The selected
            // device remains authoritative for group identity and every bin
            // header property, including FDT cell metadata and SKU bindings.
            table.groups = stock
                .groups
                .iter()
                .map(|stock_group| {
                    table
                        .groups
                        .iter()
                        .find(|imported_group| imported_group.id == stock_group.id)
                        .map_or_else(
                            || stock_group.clone(),
                            |imported_group| GpuGroup {
                                id: stock_group.id,
                                header_properties: stock_group.header_properties.clone(),
                                levels: imported_group.levels.clone(),
                            },
                        )
                })
                .collect();
        }
        if let Some(stock) = self.stock_table.as_ref()
            && let Ok(normalized) = normalize_edited_gpu_table(stock, &table)
        {
            table = normalized.table;
        }
        self.edited_table = Some(table);
        self.cell_edits.clear();
        self.edited_dirty = self.edited_table != self.stock_table;
        self.import_warnings = export.import_warnings;
        Ok(())
    }

    /// Retain the exact text being typed and commit only values that parse.
    /// Frequencies are presented in MHz but stored as exact integer Hz.
    pub(crate) fn edit_cell(&mut self, key: GpuCellKey, text: String) -> bool {
        let Some(property_name) = self
            .cell_property(key)
            .map(|property| property.name.clone())
        else {
            return false;
        };
        if gpu_property_editability(GpuPropertyLocation::Level, &property_name)
            == GpuPropertyEditability::ReadOnly
        {
            return false;
        }
        let parsed = if property_name == "qcom,gpu-freq" {
            parse_gpu_frequency_mhz(&text)
        } else {
            parse_gpu_cell(&text).map_err(|_| ())
        };
        let has_error = parsed.is_err();
        self.cell_edits.insert(key, GpuCellEdit { text, has_error });
        let Ok(value) = parsed else {
            return false;
        };
        let Some(stock) = self.stock_table.as_ref() else {
            return false;
        };
        let Some(mut edited) = self.edited_table.clone() else {
            return false;
        };
        let Some(cell) = Self::cell_mut(&mut edited, key) else {
            return false;
        };
        *cell = value;
        let Ok(normalized) = normalize_edited_gpu_table(stock, &edited) else {
            return false;
        };
        self.edited_table = Some(normalized.table);
        self.edited_dirty = self.edited_table != self.stock_table;
        true
    }

    pub(crate) fn cell_text(&self, key: GpuCellKey, committed: u32, property: &str) -> String {
        self.cell_edits.get(&key).map_or_else(
            || {
                if property == "qcom,gpu-freq" {
                    format_gpu_frequency_mhz(committed)
                } else {
                    committed.to_string()
                }
            },
            |edit| edit.text.clone(),
        )
    }

    pub(crate) fn cell_has_input_error(&self, key: GpuCellKey) -> bool {
        self.cell_edits.get(&key).is_some_and(|edit| edit.has_error)
    }

    pub(crate) fn issue_matches_cell(&self, issue: &GpuTableIssue, key: GpuCellKey) -> bool {
        self.cell_property_path(key)
            .is_some_and(|path| issue.path == path)
    }

    /// Blocking parser/structural findings plus all non-blocking validation and
    /// retargeting advisories currently implied by the working table.
    pub(crate) fn editor_validation(&self) -> GpuTableValidation {
        let Some(edited) = self.edited_table.as_ref() else {
            return GpuTableValidation::default();
        };
        let mut validation = validate_gpu_table(edited);
        for (key, edit) in &self.cell_edits {
            if edit.has_error {
                validation.hard_errors.push(GpuTableIssue {
                    path: self.cell_property_path(*key).map_or_else(
                        || "table".to_string(),
                        |path| format!("{path}[{}]", key.cell),
                    ),
                    message: "cell input is not a parseable u32 value".to_string(),
                });
            }
        }
        if let Some(stock) = self.stock_table.as_ref()
            && let Ok(normalized) = normalize_edited_gpu_table(stock, edited)
        {
            validation.warnings = normalized.advisories;
        }
        validation
            .warnings
            .extend(self.import_warnings.iter().cloned());
        validation
    }

    /// Append a copy of the last sibling through the core's schema-preserving
    /// constructor, then apply the core's index/initial-target normalization.
    pub(crate) fn add_level(&mut self, group_position: usize) -> bool {
        if self.editor_validation().has_hard_errors() {
            return false;
        }
        let Some(stock) = self.stock_table.as_ref() else {
            return false;
        };
        let Some(mut edited) = self.edited_table.clone() else {
            return false;
        };
        let Some(group) = edited.groups.get(group_position) else {
            return false;
        };
        let Some(template) = group.levels.last() else {
            return false;
        };
        let Ok(new_level_id) = u32::try_from(group.levels.len()) else {
            return false;
        };
        let Ok(new_level) =
            build_gpu_level_from_template(group, template.id, new_level_id, |property| {
                property.cells.clone()
            })
        else {
            return false;
        };
        edited.groups[group_position].levels.push(new_level);
        let Ok(normalized) = normalize_edited_gpu_table(stock, &edited) else {
            return false;
        };
        self.edited_table = Some(normalized.table);
        self.cell_edits.clear();
        self.edited_dirty = self.edited_table != self.stock_table;
        true
    }

    /// Remove one row while refusing to create an invalid empty group.
    pub(crate) fn remove_level(&mut self, group_position: usize, level_position: usize) -> bool {
        if self.editor_validation().has_hard_errors() {
            return false;
        }
        let Some(stock) = self.stock_table.as_ref() else {
            return false;
        };
        let Some(mut edited) = self.edited_table.clone() else {
            return false;
        };
        let Some(group) = edited.groups.get_mut(group_position) else {
            return false;
        };
        if group.levels.len() <= 1 || level_position >= group.levels.len() {
            return false;
        }
        group.levels.remove(level_position);
        let Ok(normalized) = normalize_edited_gpu_table(stock, &edited) else {
            return false;
        };
        self.edited_table = Some(normalized.table);
        self.cell_edits.clear();
        self.edited_dirty = self.edited_table != self.stock_table;
        true
    }

    pub(crate) fn revert_edits(&mut self) -> bool {
        let Some(stock) = self.stock_table.clone() else {
            return false;
        };
        self.edited_table = Some(stock);
        self.cell_edits.clear();
        self.edited_dirty = false;
        self.import_path = None;
        self.import_error = None;
        self.import_warnings.clear();
        true
    }

    fn clear_table_selection(&mut self) {
        self.selected_target_index = None;
        self.stock_table = None;
        self.edited_table = None;
        self.cell_edits.clear();
        self.edited_dirty = false;
        self.import_path = None;
        self.import_error = None;
        self.import_warnings.clear();
    }

    /// Remove inspection scratch when the flow is closed or abandoned. Stock
    /// backups are intentionally retained; only the part-2 working copies go.
    pub(crate) fn cleanup_prepared(&mut self) {
        if let Some(prepared) = self.prepared.take() {
            let _ = std::fs::remove_dir_all(prepared.work_dir);
        }
        self.candidates.clear();
        self.probable_target_index = None;
        self.target_popup_open = false;
        self.target_popup_abandons_on_dismiss = false;
        self.clear_table_selection();
    }
}

impl Wizard for KonaBessWizard {
    fn reset(&mut self) {
        self.cleanup_prepared();
        *self = Self::default();
    }

    fn step(&self) -> usize {
        self.step
    }

    fn step_mut(&mut self) -> &mut usize {
        &mut self.step
    }

    fn step_count(&self) -> usize {
        KONABESS_STEPS.len()
    }

    fn can_next(&self) -> bool {
        match self.step {
            0 => self.loader_path.is_some() && self.loader_error.is_none(),
            1 => {
                self.prepared.is_some()
                    && self.selected_chip().is_some()
                    && self.stock_table.is_some()
                    && self.edited_table.is_some()
                    && !self.editor_validation().has_hard_errors()
            }
            2 => {
                self.loader_path.is_some()
                    && self.prepared.is_some()
                    && self.selected_chip().is_some()
                    && self.edited_table.is_some()
                    && !self.editor_validation().has_hard_errors()
            }
            3 => false,
            _ => false,
        }
    }
}

impl KonaBessWizard {
    fn cell_property(&self, key: GpuCellKey) -> Option<&ltbox_patch::konabess::GpuProperty> {
        let group = self.edited_table.as_ref()?.groups.get(key.group)?;
        let GpuCellLocation::Level { level, property } = key.location;
        group.levels.get(level)?.properties.get(property)
    }

    fn cell_mut(table: &mut GpuTable, key: GpuCellKey) -> Option<&mut u32> {
        let group = table.groups.get_mut(key.group)?;
        let GpuCellLocation::Level { level, property } = key.location;
        let property = group.levels.get_mut(level)?.properties.get_mut(property)?;
        property.cells.get_mut(key.cell)
    }

    fn cell_property_path(&self, key: GpuCellKey) -> Option<String> {
        let table = self.edited_table.as_ref()?;
        let group = table.groups.get(key.group)?;
        let property = self.cell_property(key)?;
        let GpuCellLocation::Level { level, .. } = key.location;
        Some(format!(
            "group {} / level {} / {}",
            group.id,
            group.levels.get(level)?.id,
            property.name
        ))
    }
}

fn parse_gpu_frequency_mhz(input: &str) -> Result<u32, ()> {
    const HZ_PER_MHZ: u32 = 1_000_000;
    let value = input.trim();
    let Some((whole, fraction)) = value.split_once('.') else {
        return parse_gpu_cell(value)
            .map_err(|_| ())?
            .checked_mul(HZ_PER_MHZ)
            .ok_or(());
    };
    if value.matches('.').count() != 1
        || fraction.is_empty()
        || whole.starts_with("0x")
        || whole.starts_with("0X")
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(());
    }
    let significant_fraction = fraction.trim_end_matches('0');
    if significant_fraction.len() > 6 {
        return Err(());
    }
    let whole_hz = parse_gpu_cell(whole)
        .map_err(|_| ())?
        .checked_mul(HZ_PER_MHZ)
        .ok_or(())?;
    let fraction_hz = if significant_fraction.is_empty() {
        0
    } else {
        let parsed = parse_gpu_cell(significant_fraction).map_err(|_| ())?;
        let scale = 10_u32
            .checked_pow(u32::try_from(6 - significant_fraction.len()).map_err(|_| ())?)
            .ok_or(())?;
        parsed.checked_mul(scale).ok_or(())?
    };
    whole_hz.checked_add(fraction_hz).ok_or(())
}

fn format_gpu_frequency_mhz(frequency_hz: u32) -> String {
    const HZ_PER_MHZ: u32 = 1_000_000;
    let whole = frequency_hz / HZ_PER_MHZ;
    let remainder = frequency_hz % HZ_PER_MHZ;
    if remainder == 0 {
        whole.to_string()
    } else {
        format!("{whole}.{remainder:06}")
            .trim_end_matches('0')
            .to_string()
    }
}

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
    /// `{exe_dir}/output_<action>/` — set on Confirm → Exec.
    pub(crate) output_dir: Option<std::path::PathBuf>,
    /// PatchArb: live-typing buffer for the unix-timestamp popup.
    pub(crate) arb_index_buffer: String,
    /// PatchArb: committed target rollback index. Gates inspect-step Next.
    pub(crate) arb_index_committed: Option<u64>,
    /// PatchArb: `(boot_rollback, vbmeta_rollback)` from picked firmware.
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
            // Change Country Code: country pick, then EDL loader, confirm, exec.
            &[
                "adv_step_country",
                "edl_loader_label",
                "flash_step_confirm",
                "flash_step_flash",
            ]
        } else if self.needs_region_target() {
            &[
                "adv_step_source",
                "adv_step_region_target",
                "flash_step_confirm",
                "flash_step_flash",
            ]
        } else if matches!(self.action, Some(AdvAction::PatchArb)) {
            &[
                "adv_step_source",
                "adv_step_arb_inspect",
                "flash_step_confirm",
                "flash_step_flash",
            ]
        } else if matches!(self.action, Some(AdvAction::DetectArb)) {
            // DetectArb: source step is either a loader picker (TB320FC
            // path) or a "Start" prompt; no separate confirm — Next on
            // the source step jumps straight to exec.
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
        // Change Country Code: step 0 picks the country, step 1 the EDL loader.
        if self.needs_country() {
            return match self.step {
                0 => self.country.is_some(),
                1 => self.file_path.is_some(),
                _ => true,
            };
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
        // PatchArb inspect step (step 1) requires the inspect read to
        // have completed successfully before the user can advance into
        // the timestamp popup → confirm step.
        if matches!(self.action, Some(AdvAction::PatchArb)) && self.step == 1 {
            return self.arb_inspect.is_some();
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
            // (Change Country Code now picks an EDL loader file, not a folder.)
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
                ("EDL loader (.melf / .xml)", LOADER_PICKER_EXTS)
            }
            _ => ("", &[]),
        }
    }

    /// Recents bucket for the current action. Folder actions bin into
    /// one of the 4 user-facing folder categories + `OutputFolder` for
    /// dump destinations; file actions share the `File` bucket per the
    /// unified-file-picker design.
    ///
    /// Kept close to [`Self::is_folder_op`] so they don't diverge -
    /// mismatches would either orphan recents (folder op writing to
    /// `File`) or corrupt them (file path shoved into a folder bucket).
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
            // Remaining actions don't open a Browse dialog on step 0
            // (DumpPartitions/DumpPhysical/Flash* have dedicated wizards);
            // return File defensively so storage_key() is always valid.
            _ => PickerKind::File,
        }
    }
}

#[cfg(test)]
mod flash_tests {
    use super::*;
    use ltbox_patch::key_map::KeyClass;

    fn identity(key_class: KeyClass, fingerprint: &str) -> FirmwareIdentity {
        FirmwareIdentity {
            key_class,
            efisp_load: ltbox_patch::efisp_load::EfispLoad::Undetermined,
            fingerprint: Some(fingerprint.to_string()),
            model_token: None,
        }
    }

    #[test]
    fn visible_flash_steps_include_bootloader_only_when_selected_by_gate() {
        let mut without = FlashWizard::default();
        assert_eq!(
            without.visible_steps(),
            &[
                FlashStep::Region,
                FlashStep::Target,
                FlashStep::Data,
                FlashStep::Folder,
                FlashStep::Confirm,
                FlashStep::Flash,
            ]
        );
        without.set_step(FlashStep::Folder);
        without.next();
        assert_eq!(without.current_step(), FlashStep::Confirm);
        without.back();
        assert_eq!(without.current_step(), FlashStep::Folder);

        let mut with = FlashWizard {
            firmware_identity: Some(identity(
                KeyClass::Lenovo,
                "qti/TB320FC/TB320FC:15/build:user/release-keys",
            )),
            ..FlashWizard::default()
        };
        assert_eq!(
            with.visible_steps(),
            &[
                FlashStep::Region,
                FlashStep::Target,
                FlashStep::Data,
                FlashStep::Folder,
                FlashStep::Bootloader,
                FlashStep::Confirm,
                FlashStep::Flash,
            ]
        );
        with.set_step(FlashStep::Folder);
        with.next();
        assert_eq!(with.current_step(), FlashStep::Bootloader);
        with.next();
        assert_eq!(with.current_step(), FlashStep::Confirm);
        with.back();
        assert_eq!(with.current_step(), FlashStep::Bootloader);
        with.back();
        assert_eq!(with.current_step(), FlashStep::Folder);
    }

    #[test]
    fn canoe_efisp_routes_and_candidate_gates() {
        use ltbox_patch::efisp_load::EfispLoad::{No, Undetermined, Yes};
        for model in ["TB323FU", "TB324ZC"] {
            for state in [Yes, No, Undetermined] {
                let mut identity = identity(
                    KeyClass::Unknown,
                    &format!("qti/{model}/{model}:15/build:user/release-keys"),
                );
                identity.efisp_load = state;
                let mut wizard = FlashWizard {
                    firmware_identity: Some(identity),
                    ..Default::default()
                };
                assert_eq!(
                    wizard.visible_steps().contains(&FlashStep::Bootloader),
                    state != Yes
                );
                assert_eq!(wizard.bootloader_can_next(), state != Undetermined);
                assert_eq!(wizard.bootloader_execution_allowed(), state == Yes);
                wizard.record_bootloader_decision();
                assert_eq!(wizard.no_efisp_load, state == No);
                assert_eq!(wizard.bootloader_execution_allowed(), state != Undetermined);
                wizard.user_abl_path = Some("candidate.elf".into());
                for candidate in [Yes, No, Undetermined] {
                    wizard.user_abl_efisp_load = candidate;
                    for key in [KeyClass::Testkey, KeyClass::Lenovo, KeyClass::Unknown] {
                        wizard.user_abl_key_class = Some(key);
                        assert_eq!(wizard.bootloader_can_next(), candidate == Yes);
                    }
                }
                wizard.user_abl_efisp_load = Yes;
                wizard.user_abl_analyzing = true;
                assert!(!wizard.bootloader_can_next());
                wizard.clear_bootloader();
                assert!(!wizard.no_efisp_load);
                assert_eq!(wizard.user_abl_efisp_load, Undetermined);
                wizard.reset_firmware_identity();
                assert!(wizard.firmware_identity.is_none());
            }
        }
    }

    #[test]
    fn other_models_still_require_testkey_candidates() {
        use ltbox_patch::efisp_load::EfispLoad::Yes;
        let mut wizard = FlashWizard {
            firmware_identity: Some(identity(
                KeyClass::Lenovo,
                "qti/TB320FC/TB320FC:15/build:user/release-keys",
            )),
            user_abl_path: Some("candidate.elf".into()),
            user_abl_efisp_load: Yes,
            ..Default::default()
        };
        for key in [KeyClass::Testkey, KeyClass::Lenovo, KeyClass::Unknown] {
            wizard.user_abl_key_class = Some(key);
            assert_eq!(wizard.bootloader_can_next(), key == KeyClass::Testkey);
        }
    }

    #[test]
    fn firmware_bootloader_gate_matches_key_and_model_rules() {
        let other = "qti/TB320FC/TB320FC:15/build:user/release-keys";
        assert!(firmware_needs_bootloader_step(
            KeyClass::Lenovo,
            Some(other),
            ltbox_patch::efisp_load::EfispLoad::Undetermined
        ));

        for model in ["TB323FU", "TB376FC", "TB390FU"] {
            let fingerprint = format!("qti/{model}/{model}:15/build:user/release-keys");
            assert!(!firmware_needs_bootloader_step(
                KeyClass::Lenovo,
                Some(&fingerprint),
                ltbox_patch::efisp_load::EfispLoad::Yes
            ));
        }

        assert!(!firmware_needs_bootloader_step(
            KeyClass::Testkey,
            Some(other),
            ltbox_patch::efisp_load::EfispLoad::Undetermined
        ));
        assert!(!firmware_needs_bootloader_step(
            KeyClass::Unknown,
            Some(other),
            ltbox_patch::efisp_load::EfispLoad::Undetermined
        ));
    }
}

#[cfg(test)]
mod konabess_tests {
    use super::*;
    use ltbox_patch::konabess::{GpuGroup, GpuLevel, GpuProperty, GpuTable};

    fn table(frequency: u32) -> GpuTable {
        GpuTable {
            groups: vec![GpuGroup {
                id: 0,
                header_properties: vec![],
                levels: vec![GpuLevel {
                    id: 0,
                    properties: vec![
                        GpuProperty {
                            name: "reg".into(),
                            cells: vec![0],
                        },
                        GpuProperty {
                            name: "qcom,gpu-freq".into(),
                            cells: vec![frequency],
                        },
                        GpuProperty {
                            name: "qcom,level".into(),
                            cells: vec![200],
                        },
                    ],
                }],
            }],
        }
    }

    fn prepared() -> KonaBessPrepared {
        KonaBessPrepared {
            work_dir: "work".into(),
            vendor_boot: "vendor_boot.img".into(),
            vbmeta: "vbmeta.img".into(),
            backup_dir: "backup".into(),
            slot_suffix: "_a".into(),
            probable_dtb_index: None,
        }
    }

    fn ready_wizard(frequency: u32) -> KonaBessWizard {
        let mut wizard = KonaBessWizard::default();
        wizard.apply_inspection_result(vec![candidate(1, Some("sun"), Some(frequency))], None);
        assert!(wizard.select_target(1));
        wizard.prepared = Some(prepared());
        wizard.step = 1;
        wizard
    }

    fn candidate(index: usize, chip: Option<&str>, frequency: Option<u32>) -> VendorBootDtbInfo {
        VendorBootDtbInfo {
            index,
            model: Some(format!("model-{index}")),
            chip: chip.map(str::to_owned),
            gpu_shape: None,
            table: frequency.map(table),
        }
    }

    #[test]
    fn inspection_without_export_offers_every_parsed_gpu_table() {
        let mut wizard = KonaBessWizard::default();

        wizard.apply_inspection_result(
            vec![
                candidate(1, None, Some(700_000_000)),
                candidate(2, Some("sun"), None),
                candidate(3, Some("sun"), Some(900_000_000)),
            ],
            None,
        );

        assert_eq!(
            wizard
                .candidates
                .iter()
                .map(|candidate| candidate.index)
                .collect::<Vec<_>>(),
            vec![1, 3]
        );
    }

    #[test]
    fn selecting_a_target_populates_stock_and_edited_tables() {
        let mut wizard = KonaBessWizard::default();
        wizard.apply_inspection_result(
            vec![
                candidate(2, Some("sun"), Some(700_000_000)),
                candidate(7, Some("sun"), Some(900_000_000)),
            ],
            None,
        );

        assert!(wizard.select_target(2));
        assert_eq!(wizard.selected_target_index, Some(2));
        assert_eq!(wizard.stock_table, Some(table(700_000_000)));
        assert_eq!(wizard.edited_table, wizard.stock_table);
        assert!(!wizard.edited_dirty);

        wizard
            .overwrite_edited_from_import(KonaBessExport {
                chip: "sun".into(),
                description: "import from first target".into(),
                table: table(800_000_000),
                import_warnings: vec![],
            })
            .unwrap();
        wizard.import_path = Some("first-target.txt".into());
        wizard.import_error = Some("stale error".into());
        assert!(wizard.edit_cell(GpuCellKey::level(0, 0, 1, 0), "801".to_string()));
        assert!(!wizard.cell_edits.is_empty());
        assert!(wizard.edited_dirty);

        assert!(wizard.select_target(7));
        assert_eq!(wizard.selected_target_index, Some(7));
        assert_eq!(wizard.stock_table, Some(table(900_000_000)));
        assert_eq!(wizard.edited_table, Some(table(900_000_000)));
        assert!(!wizard.edited_dirty);
        assert!(wizard.cell_edits.is_empty());
        assert!(wizard.import_path.is_none());
        assert!(wizard.import_error.is_none());
        assert!(!wizard.select_target(99));
        assert_eq!(wizard.selected_target_index, Some(7));
    }

    #[test]
    fn confirming_requires_a_selected_target() {
        let mut wizard = KonaBessWizard::default();
        wizard.apply_inspection_result(vec![candidate(3, Some("sun"), Some(700_000_000))], None);

        assert_eq!(wizard.confirm_target(), None);
        assert!(wizard.target_popup_open);
        assert!(wizard.select_target(3));
        assert_eq!(wizard.confirm_target(), Some(3));
        assert!(!wizard.target_popup_open);
    }

    #[test]
    fn initial_picker_cancel_abandons_but_reopened_picker_cancel_does_not() {
        let mut wizard = KonaBessWizard::default();
        wizard.apply_inspection_result(vec![candidate(3, Some("sun"), Some(700_000_000))], None);
        assert!(wizard.select_target(3));

        assert!(wizard.dismiss_target_popup());
        wizard.open_target_popup();
        assert!(!wizard.dismiss_target_popup());
    }

    #[test]
    fn import_overwrites_only_edited_table_and_tracks_dirty_state() {
        let mut wizard = KonaBessWizard::default();
        wizard.apply_inspection_result(vec![candidate(1, Some("sun"), Some(700_000_000))], None);
        assert!(wizard.select_target(1));
        let stock = wizard.stock_table.clone();

        wizard
            .overwrite_edited_from_import(KonaBessExport {
                chip: "sun".into(),
                description: "import".into(),
                table: table(950_000_000),
                import_warnings: vec![],
            })
            .unwrap();

        assert_eq!(wizard.stock_table, stock);
        assert_eq!(wizard.edited_table, Some(table(950_000_000)));
        assert!(wizard.edited_dirty);
        assert!(wizard.revert_edits());
        assert_eq!(wizard.edited_table, stock);
        assert!(!wizard.edited_dirty);
    }

    #[test]
    fn import_commits_the_core_normalized_table() {
        let mut wizard = ready_wizard(900_000_000);
        let group = &mut wizard.edited_table.as_mut().unwrap().groups[0];
        group.header_properties.push(GpuProperty {
            name: "qcom,initial-pwrlevel".into(),
            cells: vec![0],
        });
        let mut second = group.levels[0].clone();
        second.id = 1;
        second.properties[0].cells[0] = 1;
        second.properties[1].cells[0] = 700_000_000;
        group.levels.push(second);
        wizard.stock_table = wizard.edited_table.clone();
        let mut imported = wizard.edited_table.clone().unwrap();
        imported.groups[0].header_properties[0].cells[0] = 1;
        let expected = normalize_edited_gpu_table(wizard.stock_table.as_ref().unwrap(), &imported)
            .unwrap()
            .table;

        wizard
            .overwrite_edited_from_import(KonaBessExport {
                chip: "sun".into(),
                description: "normalization regression".into(),
                table: imported,
                import_warnings: vec![],
            })
            .unwrap();

        assert_eq!(wizard.edited_table, Some(expected));
        assert_eq!(
            wizard.edited_table.as_ref().unwrap().groups[0].header_properties[0].cells,
            [0]
        );
    }

    #[test]
    fn cell_edit_updates_only_working_copy_and_retains_invalid_text() {
        let mut wizard = ready_wizard(700_000_000);
        let stock = wizard.stock_table.clone();
        let frequency = GpuCellKey::level(0, 0, 1, 0);

        assert!(wizard.edit_cell(frequency, "812.345678".into()));
        assert_eq!(
            wizard.edited_table.as_ref().unwrap().groups[0].levels[0].properties[1].cells,
            [812_345_678]
        );
        assert_eq!(wizard.stock_table, stock);
        assert!(wizard.edited_dirty);

        assert!(!wizard.edit_cell(frequency, "812.".into()));
        assert_eq!(wizard.cell_edits[&frequency].text, "812.");
        assert!(wizard.cell_edits[&frequency].has_error);
        assert!(wizard.editor_validation().has_hard_errors());
        assert!(!wizard.can_next());
        assert_eq!(
            wizard.edited_table.as_ref().unwrap().groups[0].levels[0].properties[1].cells,
            [812_345_678]
        );
    }

    #[test]
    fn every_group_header_property_is_read_only_and_level_reg_stays_read_only() {
        for property_name in [
            "qcom,speed-bin",
            "qcom,sku-codes",
            "#address-cells",
            "#size-cells",
            "qcom,initial-pwrlevel",
            "qcom,initial-min-pwrlevel",
            "vendor,unknown-header",
        ] {
            assert_eq!(
                gpu_property_editability(GpuPropertyLocation::GroupHeader, property_name),
                GpuPropertyEditability::ReadOnly
            );
        }
        assert_eq!(
            gpu_property_editability(GpuPropertyLocation::Level, "reg"),
            GpuPropertyEditability::ReadOnly
        );
        assert_eq!(
            gpu_property_editability(GpuPropertyLocation::Level, "qcom,gpu-freq"),
            GpuPropertyEditability::Editable
        );

        let mut wizard = ready_wizard(700_000_000);
        let before = wizard.edited_table.clone();
        assert!(!wizard.edit_cell(GpuCellKey::level(0, 0, 0, 0), "99".into()));
        assert_eq!(wizard.edited_table, before);
        assert!(wizard.cell_edits.is_empty());
    }

    #[test]
    fn accepted_cell_edit_commits_the_core_normalized_table() {
        let mut wizard = ready_wizard(900_000_000);
        let group = &mut wizard.edited_table.as_mut().unwrap().groups[0];
        group.header_properties.push(GpuProperty {
            name: "qcom,initial-pwrlevel".into(),
            cells: vec![0],
        });
        let mut second = group.levels[0].clone();
        second.id = 1;
        second.properties[0].cells[0] = 1;
        second.properties[1].cells[0] = 700_000_000;
        group.levels.push(second);
        wizard.stock_table = wizard.edited_table.clone();
        wizard.edited_table.as_mut().unwrap().groups[0].header_properties[0].cells[0] = 1;

        let mut candidate = wizard.edited_table.clone().unwrap();
        candidate.groups[0].levels[0].properties[2].cells[0] = 300;
        let expected = normalize_edited_gpu_table(wizard.stock_table.as_ref().unwrap(), &candidate)
            .unwrap()
            .table;

        assert!(wizard.edit_cell(GpuCellKey::level(0, 0, 2, 0), "300".into()));
        assert_eq!(wizard.edited_table, Some(expected));
        assert_eq!(
            wizard.edited_table.as_ref().unwrap().groups[0].header_properties[0].cells,
            [0]
        );
    }

    #[test]
    fn advisory_is_visible_but_does_not_block_advancing() {
        let mut wizard = ready_wizard(700_000_000);
        let frequency = GpuCellKey::level(0, 0, 1, 0);

        assert!(wizard.edit_cell(frequency, "2000".into()));
        let validation = wizard.editor_validation();

        assert!(!validation.has_hard_errors());
        assert!(!validation.warnings.is_empty());
        assert!(wizard.can_next());
    }

    #[test]
    fn deleted_initial_target_advisory_does_not_block_advancing() {
        let mut wizard = ready_wizard(900_000_000);
        let group = &mut wizard.edited_table.as_mut().unwrap().groups[0];
        group.header_properties.push(GpuProperty {
            name: "qcom,initial-pwrlevel".into(),
            cells: vec![0],
        });
        let mut second = group.levels[0].clone();
        second.id = 1;
        second.properties[0].cells[0] = 1;
        second.properties[1].cells[0] = 700_000_000;
        group.levels.push(second);
        wizard.stock_table = wizard.edited_table.clone();
        let mut removed = wizard.stock_table.clone().unwrap();
        removed.groups[0].levels.remove(0);
        let expected = normalize_edited_gpu_table(wizard.stock_table.as_ref().unwrap(), &removed)
            .unwrap()
            .table;

        assert!(wizard.remove_level(0, 0));
        let validation = wizard.editor_validation();

        assert_eq!(wizard.edited_table, Some(expected));
        assert_eq!(
            wizard.edited_table.as_ref().unwrap().groups[0].header_properties[0].cells,
            [0]
        );
        assert!(!validation.has_hard_errors());
        assert!(
            validation
                .warnings
                .iter()
                .any(|warning| warning.message.contains("was deleted"))
        );
        assert!(wizard.can_next());
    }

    #[test]
    fn import_preserves_device_group_headers_and_ignores_foreign_groups() {
        let mut wizard = ready_wizard(700_000_000);
        let sku_codes = GpuProperty {
            name: "qcom,sku-codes".into(),
            cells: vec![1, 2, 3],
        };
        wizard.stock_table.as_mut().unwrap().groups[0]
            .header_properties
            .push(sku_codes.clone());
        wizard.edited_table.as_mut().unwrap().groups[0]
            .header_properties
            .push(sku_codes);
        let mut imported = wizard.edited_table.clone().unwrap();
        imported.groups[0].header_properties[0].cells = vec![9, 8, 7];
        imported.groups[0].levels[0].properties[1].cells[0] = 800_000_000;
        let mut foreign_group = imported.groups[0].clone();
        foreign_group.id = 99;
        imported.groups.push(foreign_group);

        wizard
            .overwrite_edited_from_import(KonaBessExport {
                chip: "sun".into(),
                description: "unsafe header replacement".into(),
                table: imported,
                import_warnings: vec![],
            })
            .unwrap();

        assert_eq!(
            wizard.edited_table.as_ref().unwrap().groups[0].header_properties[0].cells,
            [1, 2, 3]
        );
        assert_eq!(
            wizard.edited_table.as_ref().unwrap().groups[0].levels[0].properties[1].cells,
            [800_000_000]
        );
        assert_eq!(wizard.edited_table.as_ref().unwrap().groups.len(), 1);
    }

    #[test]
    fn added_level_uses_exact_ordered_schema_of_heterogeneous_sibling() {
        let mut wizard = ready_wizard(900_000_000);
        let group = &mut wizard.edited_table.as_mut().unwrap().groups[0];
        group.levels[0].properties.push(GpuProperty {
            name: "qcom,acd-level".into(),
            cells: vec![1],
        });
        group.levels.push(GpuLevel {
            id: 1,
            properties: vec![
                GpuProperty {
                    name: "reg".into(),
                    cells: vec![1],
                },
                GpuProperty {
                    name: "qcom,gpu-freq".into(),
                    cells: vec![700_000_000],
                },
                GpuProperty {
                    name: "qcom,level".into(),
                    cells: vec![150],
                },
                GpuProperty {
                    name: "qcom,bus-freq".into(),
                    cells: vec![4],
                },
            ],
        });
        wizard.stock_table = wizard.edited_table.clone();
        let first_names = wizard.edited_table.as_ref().unwrap().groups[0].levels[0]
            .properties
            .iter()
            .map(|property| property.name.clone())
            .collect::<Vec<_>>();
        let template_names = wizard.edited_table.as_ref().unwrap().groups[0].levels[1]
            .properties
            .iter()
            .map(|property| property.name.clone())
            .collect::<Vec<_>>();

        assert!(wizard.add_level(0));

        let added_names = wizard.edited_table.as_ref().unwrap().groups[0].levels[2]
            .properties
            .iter()
            .map(|property| property.name.clone())
            .collect::<Vec<_>>();
        assert_eq!(added_names, template_names);
        assert_ne!(added_names, first_names);
    }

    #[test]
    fn final_level_cannot_be_removed_and_revert_restores_exact_stock() {
        let mut wizard = ready_wizard(700_000_000);
        let stock = wizard.stock_table.clone();
        let vote = GpuCellKey::level(0, 0, 2, 0);

        assert!(!wizard.remove_level(0, 0));
        assert!(wizard.edit_cell(vote, "300".into()));
        wizard.import_path = Some("import.txt".into());
        assert!(wizard.revert_edits());

        assert_eq!(wizard.edited_table, stock);
        assert!(wizard.cell_edits.is_empty());
        assert!(wizard.import_path.is_none());
        assert!(!wizard.edited_dirty);
    }

    #[test]
    fn mhz_display_and_parser_round_trip_exact_hz() {
        for frequency in [750_000_000, 231_234_567, u32::MAX] {
            let display = format_gpu_frequency_mhz(frequency);
            assert_eq!(parse_gpu_frequency_mhz(&display), Ok(frequency));
        }
        assert_eq!(parse_gpu_frequency_mhz("231.2345670"), Ok(231_234_567));
        assert_eq!(parse_gpu_frequency_mhz("750.5"), Ok(750_500_000));
        assert_eq!(parse_gpu_frequency_mhz("0x2EE"), Ok(750_000_000));
        assert!(parse_gpu_frequency_mhz("0x2EE.5").is_err());
        assert!(parse_gpu_frequency_mhz("750.0x5").is_err());
        assert!(parse_gpu_frequency_mhz("231.2345671").is_err());
    }

    #[test]
    fn matching_import_that_does_not_change_values_stays_clean() {
        let mut wizard = KonaBessWizard::default();
        wizard.apply_inspection_result(vec![candidate(1, Some("sun"), Some(700_000_000))], None);
        assert!(wizard.select_target(1));

        wizard
            .overwrite_edited_from_import(KonaBessExport {
                chip: "sun".into(),
                description: String::new(),
                table: table(700_000_000),
                import_warnings: vec![],
            })
            .unwrap();

        assert!(!wizard.edited_dirty);
    }

    #[test]
    fn import_naming_an_existing_chip_alias_is_accepted() {
        let mut wizard = KonaBessWizard::default();
        wizard.apply_inspection_result(vec![candidate(1, Some("sun"), Some(700_000_000))], None);
        assert!(wizard.select_target(1));

        wizard
            .overwrite_edited_from_import(KonaBessExport {
                chip: "tuna".into(),
                description: String::new(),
                table: table(900_000_000),
                import_warnings: vec![],
            })
            .unwrap();

        assert_eq!(wizard.stock_table, Some(table(700_000_000)));
        assert_eq!(wizard.edited_table, Some(table(900_000_000)));
        assert!(wizard.edited_dirty);
    }

    #[test]
    fn chip_mismatch_does_not_overwrite_the_working_copy() {
        let mut wizard = KonaBessWizard::default();
        wizard.apply_inspection_result(vec![candidate(1, Some("sun"), Some(700_000_000))], None);
        assert!(wizard.select_target(1));
        let edited = wizard.edited_table.clone();

        let error = wizard
            .overwrite_edited_from_import(KonaBessExport {
                chip: "pineapple".into(),
                description: String::new(),
                table: table(900_000_000),
                import_warnings: vec![],
            })
            .unwrap_err();

        assert_eq!(
            error,
            KonaBessImportError::ChipMismatch {
                expected: "sun".into(),
                actual: "pineapple".into(),
            }
        );
        assert_eq!(wizard.edited_table, edited);
        assert!(!wizard.edited_dirty);
    }

    #[test]
    fn probable_dtb_match_is_only_a_hint_until_explicitly_selected() {
        let mut wizard = KonaBessWizard::default();
        wizard.apply_inspection_result(
            vec![
                candidate(2, Some("sun"), Some(700_000_000)),
                candidate(7, Some("sun"), Some(900_000_000)),
            ],
            Some(7),
        );

        assert!(wizard.target_popup_open);
        assert!(wizard.target_popup_abandons_on_dismiss);
        assert!(wizard.is_probable_target(7));
        assert_eq!(wizard.selected_target_index, None);
        assert_eq!(wizard.stock_table, None);
        assert_eq!(wizard.edited_table, None);
        assert_eq!(wizard.confirm_target(), None);
        assert!(wizard.target_popup_open);

        assert!(wizard.select_target(7));
        assert_eq!(wizard.selected_target_index, Some(7));
        assert_eq!(wizard.stock_table, Some(table(900_000_000)));

        let mut unknown_chip_wizard = KonaBessWizard::default();
        unknown_chip_wizard
            .apply_inspection_result(vec![candidate(7, None, Some(900_000_000))], Some(7));

        assert!(unknown_chip_wizard.target_popup_open);
        assert!(unknown_chip_wizard.is_probable_target(7));
        assert_eq!(unknown_chip_wizard.selected_target_index, None);
        assert!(!unknown_chip_wizard.select_target(7));
    }

    #[test]
    fn unknown_probable_dtb_requires_manual_selection() {
        let mut wizard = KonaBessWizard::default();
        wizard
            .apply_inspection_result(vec![candidate(2, Some("sun"), Some(700_000_000))], Some(99));

        assert!(wizard.target_popup_open);
        assert_eq!(wizard.probable_target_index, None);
        assert_eq!(wizard.selected_target_index, None);
    }

    #[test]
    fn reset_removes_workspace_and_table_state() {
        let root = tempfile::tempdir().unwrap();
        let work_dir = root.path().join("work");
        std::fs::create_dir_all(&work_dir).unwrap();
        let mut wizard = KonaBessWizard {
            prepared: Some(KonaBessPrepared {
                vendor_boot: work_dir.join("vendor_boot.img"),
                vbmeta: work_dir.join("vbmeta.img"),
                backup_dir: root.path().join("backup"),
                slot_suffix: "_a".into(),
                probable_dtb_index: None,
                work_dir: work_dir.clone(),
            }),
            candidates: vec![candidate(2, Some("sun"), Some(700_000_000))],
            ..KonaBessWizard::default()
        };
        assert!(wizard.select_target(2));

        wizard.reset();

        assert!(!work_dir.exists());
        assert!(wizard.candidates.is_empty());
        assert_eq!(wizard.selected_target_index, None);
        assert!(wizard.stock_table.is_none());
        assert!(wizard.edited_table.is_none());
        assert!(!wizard.edited_dirty);
    }
}
