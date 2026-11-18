//! Per-flow wizard state structs and their navigation logic.

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

mod advanced;
mod flash;
mod konabess;
mod partitions;
mod physical;
mod root;
mod simple_flash;
mod sysupdate;
mod unroot;

pub(crate) use advanced::*;
pub(crate) use flash::*;
pub(crate) use konabess::*;
pub(crate) use partitions::*;
pub(crate) use physical::*;
pub(crate) use root::*;
pub(crate) use simple_flash::*;
pub(crate) use sysupdate::*;
pub(crate) use unroot::*;

/// Linear-step wizard contract: `next` / `back` walk `0..step_count`, and
/// implementors supply only `step`, `step_mut`, `step_count` and `can_next`.
///
/// Not implemented for `RootWizard`: its step numbering skips around by
/// family/mode and needs custom navigation.
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
    /// True on the confirm/start screen, the step just before exec. A sidebar
    /// bounce here keeps the wizard instead of resetting to step 0.
    fn is_on_confirm_step(&self) -> bool {
        let n = self.step_count();
        n >= 2 && self.step() == n - 2
    }
}
