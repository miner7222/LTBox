//! Operation lifecycle, flash progress and the busy/exec surfaces.

use crate::*;

impl App {
    pub(crate) fn begin_op(&mut self, view: View) {
        self.begin_silent_op(view);
        let label = self.t("log_separator_start").to_string();
        self.log_separator(Some(&label));
    }

    pub(crate) fn begin_phased_op(
        &mut self,
        view: View,
        kind: OperationPhaseKind,
    ) -> PhaseReporter {
        debug_assert!(OperationPhaseKind::all().contains(&kind));
        let reporter = PhaseReporter::from_labels(
            kind.keys()
                .iter()
                .map(|key| self.t(key).to_string())
                .collect(),
        );
        self.reset_operation_feedback();
        self.operation.start(
            Some(view),
            OperationKind::Phased(kind),
            Some(reporter.clone()),
        );
        let label = self.t("log_separator_start").to_string();
        self.log_separator(Some(&label));
        reporter
    }

    /// Snapshot localized log strings for use across thread boundaries.
    pub(crate) fn live_labels(&self) -> LiveLabels {
        LiveLabels::new(|key| self.t(key).to_owned())
    }

    pub(crate) fn end_op(&mut self) {
        self.log_history.finish_progress();
        self.log_dirty = true;
        self.operation.finish(true);
        self.clear_flash_progress();
    }

    pub(crate) fn fail_op(&mut self) {
        self.log_history.finish_progress();
        self.log_dirty = true;
        self.operation.finish(false);
        self.clear_flash_progress();
    }

    pub(crate) fn reset_operation_feedback(&mut self) {
        self.error_msg = None;
        self.operation_error = None;
        self.clear_flash_progress();
        // Both operation entry points funnel through here, which is the last
        // moment the connected model is reliably readable: the device is about
        // to re-enumerate into EDL, and a re-enumeration can blank the polled
        // snapshot outright.
        self.arm_loader_memory();
    }

    pub(crate) fn begin_silent_op(&mut self, view: View) {
        self.reset_operation_feedback();
        self.operation
            .start(Some(view), OperationKind::Unphased, None);
    }

    pub(crate) fn end_silent_op(&mut self) {
        self.fail_op();
    }

    pub(crate) fn clear_flash_progress(&mut self) {
        self.flash_progress = None;
        ltbox_device::edl::clear_flash_progress();
    }

    /// True only while a busy op is on the exact firmware-write progress phase.
    pub(crate) fn firmware_write_progress_phase_active(&self) -> bool {
        if !self.operation.is_running() {
            return false;
        }
        let Some(kind) = self.operation.phase_kind() else {
            return false;
        };
        // Overflow-safe: current operation step is zero-based; policies use
        // the one-based phase numbers printed in the live log.
        self.operation
            .current_step()
            .checked_add(1)
            .is_some_and(|step| kind.is_firmware_progress_step(step))
    }

    pub(crate) fn refresh_flash_progress_snapshot(&mut self) {
        self.flash_progress = if self.firmware_write_progress_phase_active() {
            ltbox_device::edl::flash_progress()
        } else {
            None
        };
    }

    /// Secondary line under the firmware-write phase label, e.g. `super (42%)`.
    pub(crate) fn firmware_flash_progress_label(&self) -> Option<String> {
        if !self.firmware_write_progress_phase_active() || self.operation_error.is_some() {
            return None;
        }
        let progress = self.flash_progress.as_ref()?;
        if progress.partition.is_empty() {
            return None;
        }
        Some(format!("{} ({}%)", progress.partition, progress.percent))
    }

    pub(crate) fn advanced_inline_exec_surface_active(&self) -> bool {
        if self.advanced_wizard_open.is_flash_parts() {
            return self.flash_parts.step >= 3;
        }
        if self.advanced_wizard_open.is_dump_parts() {
            return self.dump_parts.step >= 2;
        }
        if self.advanced_wizard_open.is_dump_phys() {
            return self.dump_phys.step >= 2;
        }
        if self.advanced_wizard_open.is_flash_phys() {
            return self.flash_phys.step >= 3;
        }
        if self.advanced_wizard_open.is_simple_flash() {
            return self.simple_flash.step >= 2;
        }
        self.adv_wizard.action.is_some() && self.adv_wizard.step == self.adv_wizard.exec_step()
    }

    pub(crate) fn current_view_has_inline_exec_surface(&self) -> bool {
        match self.current_view {
            View::Flash => self.flash.is_in_exec(),
            View::SystemUpdate => self.sysupdate.is_in_exec(),
            View::Debloat => self.debloat.is_in_exec(),
            View::Root => self.root.is_in_exec(),
            View::Unroot => self.unroot.is_in_exec(),
            View::KonaBess => self.konabess.step >= 3,
            View::Advanced => self.advanced_inline_exec_surface_active(),
            View::Dashboard | View::Reboot | View::Settings | View::About => false,
        }
    }

    pub(crate) fn current_view_shows_shared_exec_surface(&self) -> bool {
        self.current_view_has_inline_exec_surface() && !self.image_info_exec_active()
    }

    pub(crate) fn should_show_error_banner(&self) -> bool {
        let shared_surface_owns_error = self.current_view_shows_shared_exec_surface()
            && self.operation_error.is_some()
            && self.operation_error.as_deref() == self.error_msg.as_deref();
        self.error_msg.is_some() && !shared_surface_owns_error
    }

    pub(crate) fn blocking_popup_open(&self) -> bool {
        self.country_popup_open
            || self.konabess.target_popup_open
            || self.reboot_confirm_target.is_some()
            || self.sysupdate.rescue_region_popup_open
            || self.root.run_id_popup_open
            || self.root.kernel_version_popup_open
    }

    /// True when the Advanced view holds wizard state the user would lose to a
    /// sidebar bounce: an op's exec/result surface (running, or its Done screen
    /// still up), a generic op sitting on its confirm step, or a partition
    /// read/write whose GPT table is still valid (device still in EDL). The
    /// `Navigate` handler consults this to skip the entry-time reset so
    /// navigating away and back keeps the user's place.
    pub(crate) fn advanced_in_progress(&self) -> bool {
        // The exec/result surface must survive a sidebar bounce until the user
        // hits 'start over' — mirrors the `is_in_exec()` gate the Root / Flash /
        // etc. views use in `Navigate`. `busy` already covers a running op; this
        // also keeps the result on screen after the op finishes.
        if self.advanced_inline_exec_surface_active() {
            return true;
        }
        use AdvancedWizardOpen as W;
        match self.advanced_wizard_open {
            // Generic advanced op (PatchArb / PatchDevinfo / DetectArb / ...):
            // preserve only on the confirm step (waiting to start).
            W::None => self.adv_wizard.action.is_some() && self.adv_wizard.is_confirm_step(),
            // Read/Write Partitions: a rendered GPT table — or the confirm
            // screen after it — survives as long as the device stays in EDL,
            // since the table reflects the live partition layout.
            W::FlashParts => {
                self.device.connection == ConnectionStatus::Edl && !self.flash_parts.rows.is_empty()
            }
            W::DumpParts => {
                self.device.connection == ConnectionStatus::Edl && !self.dump_parts.rows.is_empty()
            }
            // Physical storage: preserve the confirm screen (FlashPhys);
            // DumpPhys runs Select → Exec with no confirm screen to preserve.
            W::FlashPhys => self.flash_phys.step + 2 == FLASH_PHYS_STEPS.len(),
            W::DumpPhys => false,
            // Simple Flash: preserve the confirm screen (folder already
            // picked) so a sidebar bounce returns the user to it.
            W::SimpleFlash => self.simple_flash.step == 1,
        }
    }

    /// True while KonaBess owns a prepared EDL workspace whose table or
    /// confirm screen must survive a sidebar bounce. This mirrors the
    /// partition-table branch of `advanced_in_progress`.
    pub(crate) fn konabess_in_progress(&self) -> bool {
        self.device.connection == ConnectionStatus::Edl
            && self.konabess.prepared.is_some()
            && matches!(self.konabess.step, 1 | 2)
    }

    pub(crate) fn should_show_busy_progress_dialog(&self) -> bool {
        let dedicated_reboot_wait =
            self.reboot_wait_transition && self.operation.view() == Some(View::Reboot);
        self.operation.is_running()
            // The temp-file cleanup borrows `busy` only to lock out racing
            // device ops; it's a sub-second maintenance action with its own
            // in-button "Cleaning…" state, so it gets no full-screen dialog.
            && !self.cleaning_temp
            && self.current_view != View::Dashboard
            && !self.blocking_popup_open()
            && !self.current_view_has_inline_exec_surface()
            // The EDL transition has its own truthful checklist. Keep the
            // generic spinner suppressed even after that modeless dialog is
            // closed; the tracked operation itself continues unchanged.
            && !dedicated_reboot_wait
    }

    pub(crate) fn advanced_operation_label(&self) -> Option<String> {
        if self.advanced_wizard_open.is_flash_parts() {
            return Some(self.t(AdvAction::FlashPartitions.label_key()).to_string());
        }
        if self.advanced_wizard_open.is_dump_parts() {
            return Some(self.t(AdvAction::DumpPartitions.label_key()).to_string());
        }
        if self.advanced_wizard_open.is_dump_phys() {
            return Some(self.t(AdvAction::DumpPhysical.label_key()).to_string());
        }
        if self.advanced_wizard_open.is_flash_phys() {
            return Some(self.t(AdvAction::FlashPhysical.label_key()).to_string());
        }
        if self.advanced_wizard_open.is_simple_flash() {
            return Some(self.t(AdvAction::SimpleFlash.label_key()).to_string());
        }
        self.adv_wizard
            .action
            .map(|action| self.t(action.label_key()).to_string())
    }

    pub(crate) fn busy_operation_label(&self) -> String {
        if self.operation.view() == Some(View::Advanced)
            && let Some(label) = self.advanced_operation_label()
        {
            return label;
        }
        self.operation
            .view()
            .map(|view| self.t(view.label_key()).to_string())
            .unwrap_or_else(|| self.t("status_working").to_string())
    }

    /// Override busy-dialog body for the four Advanced partition/physical
    /// flows during their reboot → loader → GPT-scan preamble. Gated on
    /// `busy_view == Advanced` so a stale wizard doesn't hijack unrelated ops.
    ///
    /// When the Advanced view is busy but no specific sub-action labels
    /// itself, the default template "{operation} 중입니다." substitutes
    /// in `nav_advanced` ("고급") and reads awkwardly across all four
    /// locales — "고급 중입니다." / "Advanced is in progress." /
    /// "高级 正在进行中。" / "Дополнительно выполняется." — because
    /// the operation token is a section noun, not a verb phrase. The
    /// `busy_advanced_generic` key carries a per-locale full sentence
    /// for this fallback.
    pub(crate) fn busy_body_override(&self) -> Option<String> {
        if self.operation.view() == Some(View::KonaBess) {
            let key = if self.konabess.prepared.is_some() {
                "busy_konabess_cancel"
            } else {
                "busy_konabess_inspection"
            };
            return Some(self.t(key).to_string());
        }
        if self.operation.view() != Some(View::Advanced) {
            return None;
        }
        // Simple Flash is a full firmware flash, not a partition scan/write —
        // let it fall through to the default "{operation} in progress" template
        // (operation = its own label) instead of the partition-scan body.
        if self.advanced_wizard_open.is_open() && !self.advanced_wizard_open.is_simple_flash() {
            // Write Partitions' exec phase is a partition *write*; the loader-
            // upload + GPT scan preamble (and the other advanced flows) keep
            // the scan label.
            let key = if self.advanced_wizard_open.is_flash_parts() && self.flash_parts.is_in_exec()
            {
                "busy_partition_write"
            } else {
                "busy_partition_scan"
            };
            return Some(self.t(key).to_string());
        }
        if self.advanced_operation_label().is_none() {
            return Some(self.t("busy_advanced_generic").to_string());
        }
        None
    }
}
