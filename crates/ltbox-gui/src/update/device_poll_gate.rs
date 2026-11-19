//! Serialize background USB polling only with actions that can acquire the
//! same device. Navigation, selection, and ordinary wizard transitions stay
//! synchronous so a periodic poll never makes the GUI feel unresponsive.
use crate::device_choice::{
    MultiDeviceState, PollProbe, inventory_from_candidates, poll_probe, target_in_edl,
};
use crate::*;
use iced::Task;
use ltbox_device::selection::{TargetIdentity, UsbCandidate, UsbSelectionError};

/// The USB calls one device poll makes, so tests can show which states open
/// nothing.
pub(super) trait PollUsb {
    /// Enumeration only; claims nothing.
    fn list_candidates(&mut self) -> Result<Vec<UsbCandidate>, UsbSelectionError>;
    /// Enumeration only; never binds an EDL device.
    fn target_visible(&mut self) -> Result<bool, UsbSelectionError>;
    /// Opens transports: ADB, then Fastboot, then EDL when `probe_edl`.
    fn probe_transports(&mut self, result: &mut DevicePollResult, probe_edl: bool);
}

/// The real USB stack.
pub(super) struct UsbPoll;

impl PollUsb for UsbPoll {
    fn list_candidates(&mut self) -> Result<Vec<UsbCandidate>, UsbSelectionError> {
        ltbox_device::selection::list_candidates()
    }

    fn target_visible(&mut self) -> Result<bool, UsbSelectionError> {
        ltbox_device::selection::target_visible()
    }

    fn probe_transports(&mut self, result: &mut DevicePollResult, probe_edl: bool) {
        super::probe_transports(result, probe_edl);
    }
}

/// One device poll on the worker thread. `prev` and `target` are the update
/// loop's state when the poll started; the update loop folds the inventory in
/// with the same inputs, so the state predicted here is the one it computes.
///
/// Transports open only for the single device (no target, as before) or the
/// chosen one. In `NeedsChoice`, `MultipleEdl` and `Unselectable` nothing opens.
/// `None` keeps the last snapshot: enumeration failed while a choice matters.
pub(super) fn run_device_poll(
    prev: &device_choice::DeviceChoiceState,
    target: Option<&TargetIdentity>,
    usb: &mut impl PollUsb,
) -> Option<DevicePollResult> {
    let mut result = DevicePollResult::default();
    match usb.list_candidates() {
        Ok(candidates) => result.inventory = inventory_from_candidates(&candidates),
        Err(error) => {
            tracing::debug!("USB enumeration failed: {error}");
            // Without a device list nothing can be told apart. A session that
            // has only seen one device probes as before; any other keeps its state.
            if target.is_some() || prev.state != MultiDeviceState::Single {
                return None;
            }
            usb.probe_transports(&mut result, true);
            return Some(result);
        }
    }
    match poll_probe(prev, &result.inventory, target.is_some()) {
        PollProbe::Nothing => {}
        PollProbe::Untargeted => usb.probe_transports(&mut result, true),
        PollProbe::Target => {
            let Some(target) = target else {
                return Some(result);
            };
            match usb.target_visible() {
                // Never `edl::check_device()` here: with a target it resolves and
                // can bind. EDL presence comes from the inventory instead.
                Ok(true) if target_in_edl(target, &result.inventory) => {
                    result.status = ConnectionStatus::Edl;
                }
                Ok(true) => usb.probe_transports(&mut result, false),
                Ok(false) => {}
                // A refused proof (`UnprovenSuccessor`, `AmbiguousTarget`) or a
                // failed enumeration: the target is not visible, never present.
                Err(error) => tracing::debug!("chosen device treated as not visible: {error}"),
            }
        }
    }
    Some(result)
}

pub(super) fn defers_message(app: &App, message: &Message) -> bool {
    match message {
        Message::DeviceChoice(_) => true,
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
        Message::FlashParts(FlashPartsMsg::FlashPartsClose) => {
            matches!(app.flash_parts.step, 1 | 2)
        }
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
