//! Physical Storage wizards (Advanced → Dump/Flash Physical).
//!
//! LUN-level counterparts to the partition wizards: no GPT scan; the user picks
//! a LUN (0..=5) and the exec pass reads or writes the whole LUN. Mirrors qdlrs
//! `Dump` (whole-disk variant) and `OverwriteStorage`.

use super::*;

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
