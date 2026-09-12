//! Serialize background USB polling only with actions that can acquire the
//! same device. Navigation, selection, and ordinary wizard transitions stay
//! synchronous so a periodic poll never makes the GUI feel unresponsive.
use crate::*;
use iced::Task;

pub(super) fn defers_message(app: &App, message: &Message) -> bool {
    match message {
        Message::Flash(FlashMsg::FlashNext) => app.flash.current_step() == FlashStep::Confirm,
        Message::Root(RootMsg::RootNext) => app.root.step == 6,
        Message::Unroot(UnrootMsg::UnrootNext) => app.unroot.step == 3,
        Message::Sys(SysMsg::SysNext) => {
            app.sysupdate.step
                == if app.sysupdate.action == Some(SysUpdateAction::Rescue) {
                    2
                } else {
                    1
                }
        }
        Message::FlashParts(FlashPartsMsg::FlashPartsNext) => app.flash_parts.step != 1,
        Message::FlashParts(FlashPartsMsg::FlashPartsBack) => app.flash_parts.step == 1,
        Message::DumpParts(DumpPartsMsg::DumpPartsNext) => app.dump_parts.step == 0,
        Message::DumpParts(DumpPartsMsg::DumpPartsBack) => app.dump_parts.step == 1,
        Message::FlashPhys(FlashPhysMsg::FlashPhysNext) => app.flash_phys.step == 2,
        Message::SimpleFlash(SimpleFlashMsg::SimpleFlashNext) => app.simple_flash.step == 1,
        Message::KonaBess(KonaBessMsg::KonaBessNext) => app.konabess.step != 1,
        Message::KonaBess(KonaBessMsg::KonaBessBack) => {
            app.konabess.step == 1 && app.konabess.prepared.is_some()
        }
        Message::Adv(AdvMsg::AdvWizNext) => {
            matches!(app.adv_wizard.action, Some(AdvAction::DetectArb))
                || (matches!(app.adv_wizard.action, Some(AdvAction::PatchDevinfo))
                    && app.adv_wizard.is_confirm_step())
        }
        _ => matches!(
            message,
            Message::InstallSelfUpdate
                | Message::KillAdbServer
                | Message::InstallDrivers
                | Message::ConfirmCloseSoftwareFix
                | Message::ForceCloseSoftwareFix
                | Message::Flash(FlashMsg::FlashExecStart)
                | Message::Root(RootMsg::RootExecStart)
                | Message::Unroot(UnrootMsg::UnrootExecStart)
                | Message::Sys(SysMsg::SysExecStart)
                | Message::Adv(AdvMsg::AdvDetectArbExecStart)
                | Message::FlashParts(
                    FlashPartsMsg::FlashPartsScanStart | FlashPartsMsg::FlashPartsExecStart
                )
                | Message::DumpParts(
                    DumpPartsMsg::DumpPartsScanStart | DumpPartsMsg::DumpPartsFolderChosen(Some(_))
                )
                | Message::DumpPhys(DumpPhysMsg::DumpPhysFolderChosen(Some(_)))
                | Message::FlashPhys(FlashPhysMsg::FlashPhysExecStart)
                | Message::SimpleFlash(SimpleFlashMsg::SimpleFlashExecStart)
                | Message::Reboot(
                    RebootMsg::RebootConfirm
                        | RebootMsg::RebootTo(_)
                        | RebootMsg::RebootEdlWithLoader(..)
                )
                | Message::Settings(SettingsMsg::SetQcomDriverMode(_))
        ),
    }
}

impl App {
    pub(super) fn can_poll_device(&self) -> bool {
        self.queries.poll_in_flight.is_none()
            && self.queries.poll_deferred.is_empty()
            && !self.operation.is_running()
            && !self.installing_drivers
            && !self.adb_server_kill_in_flight
            && !self.software_fix.closing
            && !self.operation.direct_update.is_active()
            && self.konabess.prepared.is_none()
    }

    pub(super) fn finish_device_poll(
        &mut self,
        id: u64,
        result: Option<DevicePollResult>,
    ) -> Task<Message> {
        // A duplicate or older completion must not release the current lease.
        if !self.queries.finish_poll(id) {
            return Task::none();
        }
        let apply = self.can_poll_device();
        let task = if apply && let Some(result) = result {
            self.update(Message::DevicePolled(result))
        } else {
            Task::none()
        };
        // The blocking worker has returned and dropped its USB handles before
        // workflow input is replayed. Discard the snapshot if input was queued.
        task.chain(self.resume_after_device_poll())
    }

    pub(crate) fn resume_after_device_poll(&mut self) -> Task<Message> {
        if self.queries.poll_in_flight.is_some()
            || self.adb_server_kill_in_flight
            || self.software_fix.closing
        {
            return Task::none();
        }
        let mut tasks = Vec::new();
        while self.queries.poll_in_flight.is_none()
            && !self.adb_server_kill_in_flight
            && !self.software_fix.closing
        {
            let Some(message) = self.queries.poll_deferred.pop_front() else {
                break;
            };
            // Dispatch synchronously in input order so busy reservations and
            // wizard edits are applied before the next deferred action.
            tasks.push(self.update(message));
        }
        Task::batch(tasks)
    }
}
