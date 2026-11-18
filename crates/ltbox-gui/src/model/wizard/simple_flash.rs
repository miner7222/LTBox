//! Simple Firmware Flash wizard state (Advanced → EDL ops).

use super::*;

/// Minimal flash wizard for the "Firmware Simple Flasher" advanced op: pick a
/// firmware folder, review a read-only confirm screen, flash. No region /
/// rollback / data choices: the firmware's own rawprogram runs verbatim.
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
