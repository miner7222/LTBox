//! System-update wizard handler (OTA disable/enable, Rescue).

use crate::*;
use iced::Task;
use ltbox_core::tr_args;

impl App {
    pub(crate) fn update_sys(&mut self, msg: SysMsg) -> Task<Message> {
        match msg {
            SysMsg::SysAction(a) => {
                // Models without the `rescue` capability can't run Boot
                // Recovery. The card is disabled; also drop a stale dispatch.
                if a == SysUpdateAction::Rescue && !self.model_capabilities().rescue {
                    return Task::none();
                }
                // Reset Rescue state so a stale folder/region can't leak.
                self.sysupdate.action = Some(a);
                self.sysupdate.rescue_folder = None;
                self.sysupdate.rescue_region = None;
                self.sysupdate.rescue_region_popup_open = false;
                self.sysupdate.rescue_region_confirmed = false;
                Task::none()
            }
            SysMsg::SysNext => {
                // Rescue flow: Action(0) → Folder(1) → Confirm(2) → Exec(3), with
                // the region popup between Folder and Confirm.
                if self.sysupdate.is_rescue() {
                    if self.sysupdate.step == 1 && !self.sysupdate.rescue_region_confirmed {
                        // Pre-select the region from the polled device's PTSTPD
                        // `SaleArea` (as Flash does via `inferred_flash_region`);
                        // the popup still opens for confirmation.
                        if self.sysupdate.rescue_region.is_none()
                            && let Some(inferred) = self.inferred_flash_region()
                        {
                            self.sysupdate.rescue_region = Some(match inferred {
                                DeviceRegion::Prc => RescueRegion::Prc,
                                DeviceRegion::Row => RescueRegion::Row,
                            });
                        }
                        self.sysupdate.rescue_region_popup_open = true;
                        return Task::none();
                    }
                    if self.sysupdate.step == 2 {
                        self.sysupdate.next();
                        return self.update(Message::Sys(SysMsg::SysExecStart));
                    }
                    self.sysupdate.next();
                } else {
                    // Disable/Enable: Action(0) → Confirm(1) → Exec(2).
                    if self.sysupdate.step == 1 {
                        self.sysupdate.next();
                        return self.update(Message::Sys(SysMsg::SysExecStart));
                    }
                    self.sysupdate.next();
                }
                Task::none()
            }
            SysMsg::SysBack => {
                self.sysupdate.back();
                Task::none()
            }
            SysMsg::SysRescueSelectFolder => {
                // Rescue resolves vendor_boot / vbmeta through the shared LUN
                // map and GPT-by-name, so only an EDL loader is picked, not a
                // firmware folder with `rawprogram*.xml`.
                self.pick_loader(|__v| Message::Sys(SysMsg::SysRescueFolderChosen(__v)))
            }
            SysMsg::SysRescueFolderChosen(path) => {
                if let Some(p) = path {
                    self.remember_recent(pickers::PickerKind::File, &p);
                    self.sysupdate.rescue_folder = Some(p);
                    // Re-pick the region when the loader changes; a stale one
                    // could target the wrong hardware.
                    self.sysupdate.rescue_region = None;
                    self.sysupdate.rescue_region_confirmed = false;
                }
                Task::none()
            }
            SysMsg::SysRescueRegion(r) => {
                self.sysupdate.rescue_region = Some(r);
                self.sysupdate.rescue_region_confirmed = true;
                self.sysupdate.rescue_region_popup_open = false;
                // Picking the region is the popup's implicit "Next".
                if self.sysupdate.step == 1 {
                    self.sysupdate.next();
                }
                Task::none()
            }
            SysMsg::SysRescueRegionPopupDismiss => {
                self.sysupdate.rescue_region_popup_open = false;
                Task::none()
            }
            SysMsg::SysExecStart => {
                let Some(action) = self.sysupdate.action else {
                    return Task::none();
                };
                // Final guard: never start Boot Recovery without the `rescue`
                // capability, even if a stale selection slipped past the card.
                if action == SysUpdateAction::Rescue && !self.model_capabilities().rescue {
                    self.error_msg = Some(tr_args!(
                        "model_unsupported",
                        model = self.device.model.as_str()
                    ));
                    return Task::none();
                }
                // Clone the Rescue inputs for the async task.
                let rescue_folder = self.sysupdate.rescue_folder.clone();
                let rescue_region = self.sysupdate.rescue_region;
                if action == SysUpdateAction::Rescue
                    && self.validate_loader_path(&rescue_folder).is_err()
                {
                    return Task::none();
                }
                // Model for AVB fingerprint validation, so firmware built for
                // another model is not flashed.
                let device_model = self.device.model.clone();
                let conn = self.device.connection;
                let phase_kind = match action {
                    SysUpdateAction::Disable => OperationPhaseKind::SysUpdateDisable,
                    SysUpdateAction::Enable => OperationPhaseKind::SysUpdateEnable,
                    SysUpdateAction::Rescue => OperationPhaseKind::BootRecovery,
                };
                let phases = self.begin_phased_op(View::SystemUpdate, phase_kind);
                self.error_msg = None;
                self.log_push(format!(
                    "[SysUpdate] {}",
                    tr_args!(
                        "log_sysupdate_starting",
                        action = self.t(action.label_key())
                    )
                ));
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            sysupdate_worker(
                                action,
                                rescue_folder,
                                rescue_region,
                                device_model,
                                conn,
                                phases,
                            )
                        })
                        .await
                        .unwrap_or_else(|_| Err(ltbox_core::i18n::tr("err_task_failed")))
                    },
                    |result| match result {
                        Ok(lines) => Message::Sys(SysMsg::SysExecDone(lines)),
                        Err(e) => Message::OperationError(e),
                    },
                )
            }
            SysMsg::SysExecDone(lines) => {
                self.flush_exec_done_log(lines);
                self.end_op();
                Task::none()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rescue_dispatch_obeys_model_capabilities() {
        for model in [
            "TB323FU",
            "TB376FC",
            "TB390FU",
            "TB391FC",
            "TB320FC",
            "LAVIETab9QHD1",
            "",
        ] {
            let mut app = App::default();
            app.device.model = model.into();
            let _ = app.update_sys(SysMsg::SysAction(SysUpdateAction::Rescue));
            assert_eq!(
                app.sysupdate.action == Some(SysUpdateAction::Rescue),
                app.model_capabilities().rescue,
                "{model}"
            );
        }
    }
}
