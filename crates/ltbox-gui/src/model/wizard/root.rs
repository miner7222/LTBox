//! Root wizard state and its non-linear step navigation.

use super::*;

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
    pub(crate) release_tag: Option<String>,
    pub(crate) release_popup_open: bool,
    pub(crate) release_request: Option<std::time::Instant>,
    pub(crate) releases: Vec<ltbox_core::github::PublishedRelease>,
    pub(crate) release_selection: Option<usize>,
    pub(crate) release_error: Option<String>,
    pub(crate) nightly_source: Option<NightlySource>,
    pub(crate) ksuinit_path: Option<String>,
    pub(crate) module_path: Option<String>,
    pub(crate) file_path: Option<String>, // GKI image/ZIP or local manager APK
    pub(crate) folder_path: Option<String>, // Firmware folder (loader + optional testkey)
    /// APatch: `.kpm` modules to embed. Multi-select + per-entry remove.
    pub(crate) kpm_paths: Vec<String>,
    /// Committed nightly run ID from the build picker or manual entry.
    pub(crate) run_id: Option<String>,
    pub(crate) run_id_popup_open: bool,
    pub(crate) run_id_buffer: String,
    /// KernelSU LKM: normalized `major.minor` kernel version from ADB or manual popup.
    pub(crate) kernel_version: Option<String>,
    pub(crate) kernel_version_popup_open: bool,
    pub(crate) kernel_version_buffer: String,
    /// True when `kernel_version` came from the manual popup, not the ADB probe.
    /// Display only; the worker receives `kernel_version` either way.
    pub(crate) kernel_version_manual: bool,
    /// SKRoot key shown only after a successful run; never added to logs.
    pub(crate) skroot_root_key: Option<String>,
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
    pub(crate) fn is_local(&self) -> bool {
        matches!(
            self.provider,
            Some(Provider::MagiskForks | Provider::KernelSULocal)
        )
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
        self.is_ksu_lkm() && !self.is_local() && self.kernel_version.is_none()
    }

    pub(crate) fn active_steps(&self) -> &'static [&'static str] {
        if self.is_skroot() {
            return ROOT_STEPS_SKROOT;
        }
        if self.is_gki() {
            return ROOT_STEPS_GKI;
        }
        let has_modes = self.family.map(|f| f.has_modes()).unwrap_or(false);
        if self.provider == Some(Provider::KernelSULocal) {
            return &[
                "root_step_type",
                "root_step_mode",
                "root_step_provider",
                "root_step_files",
                "edl_loader_label",
                "root_step_confirm",
                "root_step_flash",
            ];
        }
        if self.is_local() {
            return ROOT_STEPS_FORKS;
        }
        if self.is_apatch() {
            // APatch family: Version → KPM → Folder, with no key prompt.
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

    /// True once no choice still ahead of the current step can change
    /// `active_steps()`. Family (step 0), Mode (1), Provider (2) and Version
    /// (3) pick the route; a Back to an earlier step re-opens that choice, so
    /// the current position counts, not only which fields are set.
    pub(crate) fn step_total_known(&self) -> bool {
        if self.is_skroot() {
            return true;
        }
        if self.family.is_none() {
            return false;
        }
        if self.is_gki() {
            return self.step >= 1;
        }
        if self.is_local() {
            return self.step >= 2;
        }
        // Step 8 (KPM) and 4.. all sit after the Version choice.
        self.step >= 3 && self.version.is_some()
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
        if self.provider == Some(Provider::KernelSULocal) {
            return if self.step >= 5 {
                self.step - 1
            } else {
                self.step
            };
        }
        if self.is_local() {
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
                if self.is_local() {
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
            // Both APatch providers advance directly after optional KPM selection.
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
                if self.is_local() {
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
                if self.is_local() {
                    self.file_path.is_some()
                        && (self.provider != Some(Provider::KernelSULocal)
                            || self.ksuinit_path.is_some() && self.module_path.is_some())
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
            // KPM embedding is optional.
            8 => true,
            _ => false,
        }
    }
}
