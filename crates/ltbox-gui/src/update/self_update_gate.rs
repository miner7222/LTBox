//! Keep direct updates exclusive with work that must survive until completion.
use crate::*;

impl App {
    pub(crate) fn self_update_blocked_reason(&self) -> Option<String> {
        if self.software_fix.closing {
            return Some(self.t("software_fix_closing").to_string());
        }
        if self.installing_drivers {
            return Some(self.t("driver_installing_btn").to_string());
        }
        if self.cleaning_temp {
            return Some(self.t("settings_cleanup_busy").to_string());
        }
        let operation = if self.operation.is_running() {
            self.busy_operation_label()
        } else if self.konabess.prepared.is_some() {
            // Inspection has ended, but the user still owns a staged EDL workflow.
            self.t("nav_konabess").to_string()
        } else {
            return None;
        };
        Some(ltbox_core::tr_args!(
            "progress_dialog_body",
            operation = operation
        ))
    }

    pub(crate) fn can_install_self_update(&self) -> bool {
        self.update_dialog_source == Some(ltbox_core::install_source::InstallSource::Direct)
            && self.update_available.is_some()
            && !self.operation.direct_update.is_active()
            && self.self_update_blocked_reason().is_none()
    }

    pub(super) fn can_exit_after_self_update(&self) -> bool {
        self.operation.direct_update == DirectUpdateState::Restarting
            && self.self_update_blocked_reason().is_none()
    }
}

/// Freeze workflow input, including late picker replies that can launch work.
/// Preserve worker results so a defensive deferred exit can release its blockers.
/// Gate at the dispatcher: many starts are reached through Next or picker replies,
/// not just messages named ExecStart. No worker is cancelled by this gate.
pub(super) fn blocks_message(msg: &Message) -> bool {
    match msg {
        Message::Flash(m) => !matches!(
            m,
            FlashMsg::FlashExecDone(_)
                | FlashMsg::FlashAutoRegionFetched(..)
                | FlashMsg::FlashFirmwareIdentityInspected(..)
                | FlashMsg::FlashBootloaderAnalysed(..)
        ),
        Message::Root(m) => !matches!(
            m,
            RootMsg::RootExecDone(_) | RootMsg::RootKernelVersionProbeDone(_)
        ),
        Message::Unroot(m) => !matches!(m, UnrootMsg::UnrootExecDone(_)),
        Message::Sys(m) => !matches!(m, SysMsg::SysExecDone(_)),
        Message::Adv(m) => !matches!(
            m,
            AdvMsg::AdvExecDone(_)
                | AdvMsg::AdvImageInfoExecDone(_)
                | AdvMsg::AdvDetectArbExecDone(_)
        ),
        Message::KonaBess(m) => !matches!(
            m,
            KonaBessMsg::KonaBessInspectionReady(_)
                | KonaBessMsg::KonaBessInspectionFailed(_)
                | KonaBessMsg::KonaBessCancelDone(_)
                | KonaBessMsg::KonaBessFlashDone(_)
        ),
        Message::FlashParts(m) => !matches!(
            m,
            FlashPartsMsg::FlashPartsScanDone(_) | FlashPartsMsg::FlashPartsExecDone(_)
        ),
        Message::DumpParts(m) => !matches!(
            m,
            DumpPartsMsg::DumpPartsScanDone(_) | DumpPartsMsg::DumpPartsExecDone(_)
        ),
        Message::DumpPhys(m) => !matches!(m, DumpPhysMsg::DumpPhysExecDone(_)),
        Message::FlashPhys(m) => !matches!(m, FlashPhysMsg::FlashPhysExecDone(_)),
        Message::SimpleFlash(m) => !matches!(m, SimpleFlashMsg::SimpleFlashExecDone(_)),
        Message::Reboot(m) => !matches!(m, RebootMsg::RebootDone(_)),
        Message::Settings(SettingsMsg::CleanupTempFiles | SettingsMsg::SetQcomDriverMode(_))
        | Message::Navigate(_)
        | Message::StartOver
        | Message::PollDevice
        | Message::ConfirmCloseSoftwareFix
        | Message::ForceCloseSoftwareFix
        | Message::KillAdbServer
        | Message::InstallDrivers
        | Message::RecentFilePicked(..)
        | Message::RecentFolderPicked(..) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready_app() -> App {
        App {
            update_dialog_source: Some(ltbox_core::install_source::InstallSource::Direct),
            update_available: Some(ltbox_core::github::StableRelease {
                tag: "v99.0.0".into(),
                html_url: "https://example.invalid/release".into(),
            }),
            ..App::default()
        }
    }

    #[test]
    fn self_update_refuses_install_while_an_operation_is_busy() {
        for view in [
            View::Flash,
            View::Root,
            View::Unroot,
            View::Advanced,
            View::Reboot,
        ] {
            let mut app = ready_app();
            // The update dialog was opened before the device operation started.
            app.begin_silent_op(view);
            assert!(!app.can_install_self_update());
            assert_eq!(app.update(Message::InstallSelfUpdate).units(), 0);
            assert_eq!(app.operation.direct_update, DirectUpdateState::Ready);
            assert!(app.operation.is_running());
            app.end_silent_op();
            assert!(app.can_install_self_update());
        }
    }

    #[test]
    fn self_update_refuses_driver_cleanup_and_prepared_konabess_work() {
        let mut app = ready_app();
        app.installing_drivers = true;
        assert_eq!(app.update(Message::InstallSelfUpdate).units(), 0);
        app.installing_drivers = false;
        app.cleaning_temp = true;
        assert_eq!(app.update(Message::InstallSelfUpdate).units(), 0);
        app.cleaning_temp = false;
        app.konabess.prepared = Some(KonaBessPrepared {
            work_dir: Default::default(),
            vendor_boot: Default::default(),
            vbmeta: Default::default(),
            backup_dir: Default::default(),
            slot_suffix: "_a".into(),
            probable_dtb_index: None,
            device_model: "TB323FU".into(),
        });
        // Do not depend on the latest poll's connection status.
        assert_eq!(app.update(Message::InstallSelfUpdate).units(), 0);
        assert_eq!(app.operation.direct_update, DirectUpdateState::Ready);
        app.konabess.prepared = None;
        assert!(app.can_install_self_update());
    }

    #[test]
    fn self_update_start_is_exclusive_and_reopening_cannot_reset_it() {
        let mut app = ready_app();
        // Dropping the task does not run the network/filesystem worker.
        assert_eq!(app.update(Message::InstallSelfUpdate).units(), 1);
        for state in [DirectUpdateState::Updating, DirectUpdateState::Restarting] {
            app.operation.direct_update = state.clone();
            assert_eq!(app.update(Message::OpenUpdate).units(), 0);
            assert_eq!(app.update(Message::UpdateDialogClose).units(), 0);
            assert_eq!(app.update(Message::InstallSelfUpdate).units(), 0);
            assert_eq!(app.operation.direct_update, state);
            assert!(app.update_dialog_source.is_some());
        }
    }

    #[test]
    fn self_update_blocks_starts_next_steps_and_late_picker_replies() {
        let inputs = vec![
            Message::Flash(FlashMsg::FlashExecStart),
            Message::Root(RootMsg::RootExecStart),
            Message::Root(RootMsg::RootNext),
            Message::Unroot(UnrootMsg::UnrootExecStart),
            Message::Sys(SysMsg::SysExecStart),
            Message::Adv(AdvMsg::AdvWizNext),
            Message::Adv(AdvMsg::AdvDetectArbExecStart),
            Message::KonaBess(KonaBessMsg::KonaBessNext),
            Message::FlashParts(FlashPartsMsg::FlashPartsScanStart),
            Message::FlashParts(FlashPartsMsg::FlashPartsExecStart),
            Message::DumpParts(DumpPartsMsg::DumpPartsFolderChosen(Some("dump".into()))),
            Message::DumpPhys(DumpPhysMsg::DumpPhysFolderChosen(Some("dump".into()))),
            Message::FlashPhys(FlashPhysMsg::FlashPhysExecStart),
            Message::SimpleFlash(SimpleFlashMsg::SimpleFlashExecStart),
            Message::Reboot(RebootMsg::RebootConfirm),
            Message::Reboot(RebootMsg::RebootEdlWithLoader(
                RebootTarget::System,
                Some("loader.elf".into()),
            )),
            Message::Settings(SettingsMsg::CleanupTempFiles),
            Message::InstallDrivers,
            Message::KillAdbServer,
            Message::PollDevice,
            Message::Navigate(View::Flash),
            Message::StartOver,
        ];
        for state in [DirectUpdateState::Updating, DirectUpdateState::Restarting] {
            let mut app = ready_app();
            app.operation.direct_update = state.clone();
            for message in &inputs {
                assert!(blocks_message(message), "unguarded input: {message:?}");
                assert_eq!(app.update(message.clone()).units(), 0, "{message:?}");
                assert!(!app.operation.is_running());
                assert!(!app.installing_drivers);
                assert!(!app.cleaning_temp);
                assert_eq!(app.operation.direct_update, state);
            }
        }
    }

    #[test]
    fn self_update_failure_releases_gate_and_allows_retry() {
        let mut app = ready_app();
        app.operation.direct_update = DirectUpdateState::Updating;
        let failure = SelfUpdateFailure {
            kind: SelfUpdateFailureKind::Download,
            detail: "offline".into(),
        };
        drop(app.update(Message::SelfUpdateFinished(Err(failure.clone()))));
        assert_eq!(
            app.operation.direct_update,
            DirectUpdateState::Failed(failure)
        );
        assert!(app.can_install_self_update());
        drop(app.update(Message::Root(RootMsg::RootFamily(Family::Magisk))));
        assert_eq!(app.root.family, Some(Family::Magisk));
    }

    #[test]
    fn self_update_exit_requires_success_and_waits_for_existing_work() {
        let mut app = ready_app();
        assert_eq!(app.update(Message::ExitAfterUpdate).units(), 0);
        assert_eq!(app.update(Message::SelfUpdateFinished(Ok(()))).units(), 0);
        assert_eq!(app.operation.direct_update, DirectUpdateState::Ready);
        app.operation.direct_update = DirectUpdateState::Updating;
        assert_eq!(app.update(Message::ExitAfterUpdate).units(), 0);
        drop(app.update(Message::SelfUpdateFinished(Ok(()))));
        assert!(app.can_exit_after_self_update());
        app.begin_silent_op(View::Root);
        assert!(!app.can_exit_after_self_update());
        // This task reschedules the exit check; completion messages still run.
        drop(app.update(Message::ExitAfterUpdate));
        assert!(app.operation.is_running());
        drop(app.update(Message::Root(RootMsg::RootExecDone(
            crate::workers::root::RootWorkerResult {
                log: Vec::new(),
                skroot_root_key: None,
            },
        ))));
        assert!(!app.operation.is_running());
        assert!(app.can_exit_after_self_update());
        app.installing_drivers = true;
        assert!(!app.can_exit_after_self_update());
    }
}
