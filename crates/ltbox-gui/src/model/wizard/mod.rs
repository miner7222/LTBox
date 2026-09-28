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

mod advanced;
mod debloat;
mod flash;
mod konabess;
mod partitions;
mod physical;
mod root;
mod simple_flash;
mod sysupdate;
mod unroot;

pub(crate) use advanced::*;
pub(crate) use debloat::*;
pub(crate) use flash::*;
pub(crate) use konabess::*;
pub(crate) use partitions::*;
pub(crate) use physical::*;
pub(crate) use root::*;
pub(crate) use simple_flash::*;
pub(crate) use sysupdate::*;
pub(crate) use unroot::*;

/// Linear-step wizard contract. Wizards whose `next` / `back` walk a
/// `0..step_count` range share `reset` / `next` / `back` /
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
