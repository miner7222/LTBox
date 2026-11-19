use crate::{
    App, ConnectionStatus, DevicePollResult, KonaBessPrepared, Message, OperationExecution, View,
};

fn poll(serial: &str, model: &str) -> DevicePollResult {
    DevicePollResult {
        status: ConnectionStatus::Adb,
        model: model.to_string(),
        serial: serial.to_string(),
        ..DevicePollResult::default()
    }
}

fn prepared() -> KonaBessPrepared {
    KonaBessPrepared {
        work_dir: Default::default(),
        vendor_boot: Default::default(),
        vbmeta: Default::default(),
        backup_dir: Default::default(),
        slot_suffix: "_a".to_string(),
        probable_dtb_index: None,
        device_model: "TB323FU".into(),
    }
}

#[test]
fn poll_gate_allocates_one_id_and_drops_duplicate_poll_requests() {
    let mut app = App::default();

    let first = app.update(Message::PollDevice);
    assert_eq!(first.units(), 1);
    assert_eq!(app.queries.sequence(), 1);
    assert_eq!(app.queries.poll_in_flight, Some(1));

    let second = app.update(Message::PollDevice);
    assert_eq!(second.units(), 0);
    assert_eq!(app.queries.sequence(), 1);
    assert_eq!(app.queries.poll_in_flight, Some(1));
}

#[test]
fn stale_poll_completion_keeps_lease_and_snapshot_unchanged() {
    let mut app = App::default();
    let _ = app.update(Message::DevicePolled(poll("old", "old-model")));
    let _ = app.update(Message::PollDevice);

    let task = app.update(Message::DevicePollFinished(
        2,
        Some(poll("new", "new-model")),
    ));
    assert_eq!(task.units(), 0);
    assert_eq!(app.queries.poll_in_flight, Some(1));
    assert_eq!(app.device.serial, "old");
    assert_eq!(app.device.model, "old-model");
}

#[test]
fn empty_matching_completion_releases_gate_and_preserves_snapshot() {
    let mut app = App::default();
    let _ = app.update(Message::DevicePolled(poll("old", "old-model")));
    let _ = app.update(Message::PollDevice);

    let task = app.update(Message::DevicePollFinished(1, None));
    assert_eq!(task.units(), 0);
    assert_eq!(app.queries.poll_in_flight, None);
    assert_eq!(app.device.serial, "old");
    assert_eq!(app.device.model, "old-model");
    assert!(app.can_poll_device());

    let task = app.update(Message::PollDevice);
    assert_eq!(task.units(), 1);
    assert_eq!(app.queries.poll_in_flight, Some(2));
}

#[test]
fn workflow_blockers_prevent_a_new_poll() {
    let mut app = App {
        operation: OperationExecution::fixture(true, None, Vec::new(), 0, None),
        ..App::default()
    };
    assert_eq!(app.update(Message::PollDevice).units(), 0);
    assert_eq!(app.queries.sequence(), 0);

    app.end_silent_op();
    app.installing_drivers = true;
    assert_eq!(app.update(Message::PollDevice).units(), 0);
    assert_eq!(app.queries.sequence(), 0);

    app.installing_drivers = false;
    app.konabess.prepared = Some(prepared());
    assert_eq!(app.update(Message::PollDevice).units(), 0);
    assert_eq!(app.queries.sequence(), 0);
}

#[test]
fn navigation_and_ui_only_selection_apply_without_waiting_for_poll() {
    let mut app = App {
        current_view: View::Dashboard,
        root: crate::RootWizard {
            step: 3,
            family: Some(crate::Family::Magisk),
            ..crate::RootWizard::default()
        },
        ..App::default()
    };
    let _ = app.update(Message::PollDevice);
    let _ = app.update(Message::Navigate(View::Root));
    let _ = app.update(Message::Root(crate::RootMsg::RootFamily(
        crate::Family::Magisk,
    )));
    let _ = app.update(Message::StartOver);

    assert_eq!(app.current_view, View::Root);
    assert!(app.queries.poll_deferred.is_empty());
    assert_eq!(app.root.step, 0);
    assert!(app.root.family.is_none());
    assert_eq!(app.device.serial, "");

    let task = app.update(Message::DevicePollFinished(
        1,
        Some(poll("stale", "stale-model")),
    ));
    assert_eq!(task.units(), 0);
    assert_eq!(app.current_view, View::Root);
    assert_eq!(app.device.serial, "stale");
    assert_eq!(app.device.model, "stale-model");
    assert!(app.queries.poll_deferred.is_empty());
    assert_eq!(app.root.step, 0);
    assert!(app.root.family.is_none());
}

#[test]
fn device_work_start_still_waits_for_poll_completion() {
    let mut app = App::default();
    let _ = app.update(Message::PollDevice);

    let task = app.update(Message::Unroot(crate::UnrootMsg::UnrootExecStart));

    assert_eq!(task.units(), 0);
    assert_eq!(app.queries.poll_deferred.len(), 1);
}

#[test]
fn ordinary_wizard_next_is_immediate_but_final_next_waits_for_poll() {
    let mut app = App::default();
    let _ = app.update(Message::PollDevice);

    let _ = app.update(Message::Unroot(crate::UnrootMsg::UnrootNext));
    assert_eq!(app.unroot.step, 1);
    assert!(app.queries.poll_deferred.is_empty());

    app.unroot.step = 3;
    let task = app.update(Message::Unroot(crate::UnrootMsg::UnrootNext));
    assert_eq!(task.units(), 0);
    assert_eq!(app.unroot.step, 3);
    assert_eq!(app.queries.poll_deferred.len(), 1);
}

#[test]
fn old_completion_cannot_release_or_apply_after_a_new_poll_starts() {
    let mut app = App::default();
    let _ = app.update(Message::DevicePollFinished(
        0,
        Some(poll("ignored", "ignored-model")),
    ));
    let _ = app.update(Message::DevicePolled(poll("stable", "stable-model")));
    let _ = app.update(Message::PollDevice);
    let _ = app.update(Message::DevicePollFinished(1, None));
    let _ = app.update(Message::PollDevice);

    assert_eq!(app.queries.poll_in_flight, Some(2));
    let task = app.update(Message::DevicePollFinished(
        1,
        Some(poll("old", "old-model")),
    ));
    assert_eq!(task.units(), 0);
    assert_eq!(app.queries.poll_in_flight, Some(2));
    assert_eq!(app.device.serial, "stable");
    assert_eq!(app.device.model, "stable-model");
}

#[test]
fn queued_kill_server_does_not_delay_navigation() {
    let mut app = App::default();
    let _ = app.update(Message::PollDevice);
    let _ = app.update(Message::KillAdbServer);
    let _ = app.update(Message::Navigate(View::Settings));

    assert_eq!(app.queries.poll_deferred.len(), 1);
    assert!(!app.adb_server_kill_in_flight);
    assert_eq!(app.current_view, View::Settings);

    let task = app.update(Message::DevicePollFinished(
        1,
        Some(poll("stale", "stale-model")),
    ));
    assert!(task.units() > 0);
    assert!(app.adb_server_kill_in_flight);
    assert_eq!(app.queries.poll_deferred.len(), 0);
    assert_eq!(app.current_view, View::Settings);
    assert_eq!(app.device.serial, "");

    let task = app.update(Message::AdbServerKillFinished(Ok(())));
    assert!(task.units() > 0);
    assert!(!app.adb_server_kill_in_flight);
    assert_eq!(app.queries.poll_deferred.len(), 0);
    assert_eq!(app.current_view, View::Settings);
}

#[test]
fn matching_poll_applies_once_then_rejects_duplicate_completion() {
    let mut app = App::default();
    let _ = app.update(Message::PollDevice);
    let _ = app.update(Message::DevicePollFinished(
        1,
        Some(poll("new", "new-model")),
    ));
    assert_eq!(app.device.serial, "new");
    assert_eq!(app.device.model, "new-model");
    assert!(app.queries.poll_in_flight.is_none());
    let _ = app.update(Message::DevicePollFinished(
        1,
        Some(poll("old", "old-model")),
    ));
    assert_eq!(app.device.serial, "new");
}

#[test]
fn poll_cannot_run_across_an_operation_boundary() {
    let mut app = App::default();
    let _ = app.update(Message::PollDevice);
    let _ = app.update(Message::DevicePollFinished(
        1,
        Some(poll("before", "before")),
    ));
    app.begin_op(View::Root);
    assert_eq!(app.update(Message::PollDevice).units(), 0);
    app.end_op();
    let _ = app.update(Message::PollDevice);
    assert_eq!(app.queries.poll_in_flight, Some(2));
    let _ = app.update(Message::DevicePollFinished(1, Some(poll("stale", "stale"))));
    assert_eq!(app.device.serial, "before");
    assert_eq!(app.queries.poll_in_flight, Some(2));
    let _ = app.update(Message::DevicePollFinished(2, Some(poll("after", "after"))));
    assert_eq!(app.device.serial, "after");
}

mod device_choice_activation {
    use super::super::device_poll_gate::{PollUsb, run_device_poll};
    use crate::device_choice::test_target::{self, Call};
    use crate::device_choice::{ChoiceId, DeviceChoice, DeviceChoiceState, MultiDeviceState};
    use crate::{
        App, ConnectionStatus, DeviceChoiceMsg, DevicePollResult, Message, OperationExecution,
    };
    use ltbox_device::selection::{
        DeviceMode, TargetIdentity, UsbCandidate, UsbLocation, UsbSelectionError,
    };

    fn candidate(mode: DeviceMode, serial: Option<&str>, port: u8) -> UsbCandidate {
        UsbCandidate {
            mode,
            serial: serial.map(str::to_owned),
            location: UsbLocation {
                bus_id: "1".to_string(),
                port_chain: vec![port],
            },
            vendor_id: 0x18d1,
            product_id: 0x4ee0,
            product: None,
            instance_id: None,
        }
    }

    fn adb(serial: &str, port: u8) -> UsbCandidate {
        candidate(DeviceMode::Adb, Some(serial), port)
    }

    fn edl(port: u8) -> UsbCandidate {
        candidate(DeviceMode::Edl, None, port)
    }

    fn polled(candidates: &[UsbCandidate]) -> Message {
        let inventory: Vec<DeviceChoice> = candidates
            .iter()
            .map(DeviceChoice::from_candidate)
            .collect();
        let status = if inventory.len() == 1 {
            ConnectionStatus::Adb
        } else {
            ConnectionStatus::None
        };
        Message::DevicePolled(DevicePollResult {
            status,
            inventory,
            ..DevicePollResult::default()
        })
    }

    fn pick(serial: &str) -> Message {
        Message::DeviceChoice(DeviceChoiceMsg::PickNow(ChoiceId::Serial(
            serial.to_string(),
        )))
    }

    fn serial_target(serial: &str) -> TargetIdentity {
        TargetIdentity::Serial {
            serial: serial.to_string(),
        }
    }

    #[test]
    fn device_choice_pick_sets_the_target_exactly_once() {
        let mut app = App::default();
        let _ = app.update(polled(&[adb("A", 1), adb("B", 2)]));
        assert!(test_target::calls().is_empty());
        let _ = app.update(pick("A"));
        assert_eq!(test_target::calls(), vec![Call::Set(serial_target("A"))]);
        assert_eq!(test_target::current(), Some(serial_target("A")));
        // Unchanged polls and a repeated pick of the same device keep it.
        let _ = app.update(polled(&[adb("A", 1), adb("B", 2)]));
        let _ = app.update(pick("A"));
        let _ = app.update(polled(&[adb("B", 2)]));
        assert_eq!(app.device_choice.state, MultiDeviceState::Chosen);
        assert_eq!(test_target::set_calls(), 1);
    }

    #[test]
    fn device_choice_one_device_all_session_never_sets_a_target() {
        let mut app = App::default();
        for inventory in [
            vec![adb("A", 1)],
            vec![],
            vec![adb("A", 3)],
            vec![edl(3)],
            vec![adb("A", 1)],
        ] {
            let _ = app.update(polled(&inventory));
            assert_eq!(app.device_choice.state, MultiDeviceState::Single);
            for message in [
                DeviceChoiceMsg::Open,
                DeviceChoiceMsg::Pick(ChoiceId::Serial("A".to_string())),
                DeviceChoiceMsg::Confirm,
                DeviceChoiceMsg::PickNow(ChoiceId::Serial("A".to_string())),
            ] {
                let _ = app.update(Message::DeviceChoice(message));
            }
        }
        assert_eq!(test_target::set_calls(), 0);
        assert!(test_target::calls().is_empty());
    }

    #[test]
    fn device_choice_others_changing_clears_the_target() {
        for after in [
            vec![adb("A", 1), adb("B", 2), adb("C", 3)],
            vec![adb("A", 1)],
            vec![adb("A", 1), adb("C", 2)],
            vec![],
        ] {
            let mut app = App::default();
            let _ = app.update(polled(&[adb("A", 1), adb("B", 2)]));
            let _ = app.update(pick("A"));
            let _ = app.update(polled(&after));
            assert_eq!(
                test_target::calls(),
                vec![Call::Set(serial_target("A")), Call::Clear],
                "{after:?}"
            );
            assert_eq!(test_target::current(), None);
            assert!(app.device_choice.chosen.is_none());
            if !after.is_empty() {
                assert_eq!(app.device_choice.state, MultiDeviceState::NeedsChoice);
                assert!(app.device_choice.rechoose);
            }
            test_target::reset();
        }
    }

    #[test]
    fn device_choice_during_a_running_operation_is_a_no_op() {
        let mut app = App::default();
        let _ = app.update(polled(&[adb("A", 1), adb("B", 2)]));
        app.operation = OperationExecution::fixture(true, None, Vec::new(), 0, None);
        let before = app.device_choice.clone();
        for message in [
            DeviceChoiceMsg::Open,
            DeviceChoiceMsg::PickNow(ChoiceId::Serial("A".to_string())),
        ] {
            let _ = app.update(Message::DeviceChoice(message));
        }
        assert_eq!(app.device_choice.state, before.state);
        assert_eq!(app.device_choice.chosen, None);
        assert_eq!(app.device_choice.dialog, None);
        assert!(test_target::calls().is_empty());
    }

    #[test]
    fn device_choice_waits_for_a_poll_in_flight() {
        let mut app = App::default();
        let _ = app.update(polled(&[adb("A", 1), adb("B", 2)]));
        let _ = app.update(Message::PollDevice);
        let poll = app.queries.poll_in_flight.expect("poll started");
        let _ = app.update(pick("A"));
        assert!(test_target::calls().is_empty());
        assert_eq!(app.device_choice.chosen, None);
        // The deferred pick runs once the poll has finished.
        let _ = app.update(Message::DevicePollFinished(poll, None));
        assert_eq!(test_target::calls(), vec![Call::Set(serial_target("A"))]);
    }

    /// Records every USB call; panics if a transport opens when `allow_probe`
    /// is false.
    struct FakeUsb {
        candidates: Result<Vec<UsbCandidate>, ()>,
        visible: Result<bool, ()>,
        allow_probe: bool,
        visible_calls: usize,
        probes: Vec<bool>,
    }

    impl FakeUsb {
        fn new(candidates: Vec<UsbCandidate>) -> Self {
            Self {
                candidates: Ok(candidates),
                visible: Ok(true),
                allow_probe: true,
                visible_calls: 0,
                probes: Vec::new(),
            }
        }

        fn sealed(candidates: Vec<UsbCandidate>) -> Self {
            Self {
                allow_probe: false,
                ..Self::new(candidates)
            }
        }
    }

    impl PollUsb for FakeUsb {
        fn list_candidates(&mut self) -> Result<Vec<UsbCandidate>, UsbSelectionError> {
            self.candidates
                .clone()
                .map_err(|()| UsbSelectionError::Enumeration("test".to_string()))
        }

        fn target_visible(&mut self) -> Result<bool, UsbSelectionError> {
            self.visible_calls += 1;
            self.visible
                .map_err(|()| UsbSelectionError::UnprovenSuccessor("test".to_string()))
        }

        fn probe_transports(&mut self, result: &mut DevicePollResult, probe_edl: bool) {
            assert!(self.allow_probe, "a transport opened");
            self.probes.push(probe_edl);
            result.status = ConnectionStatus::Adb;
        }
    }

    fn state_after(candidates: &[UsbCandidate]) -> DeviceChoiceState {
        let mut state = DeviceChoiceState::default();
        state.apply(
            candidates
                .iter()
                .map(DeviceChoice::from_candidate)
                .collect(),
            false,
        );
        state
    }

    fn chosen_a() -> DeviceChoiceState {
        let mut chosen = state_after(&[adb("A", 1), adb("B", 2)]);
        let _ = chosen.handle(DeviceChoiceMsg::PickNow(ChoiceId::Serial("A".to_string())));
        assert_eq!(chosen.state, MultiDeviceState::Chosen);
        chosen
    }

    #[test]
    fn device_choice_poll_in_a_blocking_state_opens_no_transport() {
        for (inventory, expected) in [
            (
                vec![adb("A", 1), adb("B", 2)],
                MultiDeviceState::NeedsChoice,
            ),
            (vec![edl(1), edl(2)], MultiDeviceState::MultipleEdl),
            (
                vec![adb("A", 1), adb("A", 2)],
                MultiDeviceState::Unselectable,
            ),
            (
                vec![adb("A", 1), candidate(DeviceMode::Other, None, 2)],
                MultiDeviceState::Unselectable,
            ),
        ] {
            // From a fresh session and from the state it already shows.
            for prev in [DeviceChoiceState::default(), state_after(&inventory)] {
                let mut usb = FakeUsb::sealed(inventory.clone());
                let result = run_device_poll(&prev, None, &mut usb).expect("a result");
                assert_eq!(result.status, ConnectionStatus::None);
                assert_eq!(result.inventory.len(), inventory.len());
                assert_eq!(usb.visible_calls, 0);
                let mut next = prev.clone();
                let _ = next.apply(result.inventory, false);
                assert_eq!(next.state, expected);
            }
        }
        // One device left after a choice is a re-prompt: still nothing opens.
        let chosen = chosen_a();
        for target in [Some(serial_target("A")), None] {
            let mut usb = FakeUsb::sealed(vec![adb("A", 1)]);
            let result = run_device_poll(&chosen, target.as_ref(), &mut usb).expect("a result");
            assert_eq!(result.status, ConnectionStatus::None);
            assert_eq!(usb.visible_calls, 0);
        }
        // A held target that no choice backs opens nothing either.
        let mut usb = FakeUsb::sealed(vec![adb("A", 1)]);
        let target = serial_target("A");
        let _ = run_device_poll(&DeviceChoiceState::default(), Some(&target), &mut usb);
        assert_eq!(usb.visible_calls, 0);
    }

    #[test]
    fn device_choice_poll_probes_the_single_device_as_before() {
        for inventory in [vec![], vec![adb("A", 1)], vec![edl(1)]] {
            let mut usb = FakeUsb::new(inventory);
            let result =
                run_device_poll(&DeviceChoiceState::default(), None, &mut usb).expect("a result");
            assert_eq!(usb.probes, vec![true], "EDL is probed as before");
            assert_eq!(usb.visible_calls, 0);
            assert_eq!(result.status, ConnectionStatus::Adb);
        }
        // Enumeration failure keeps the single-device probe of before.
        let mut usb = FakeUsb::new(Vec::new());
        usb.candidates = Err(());
        assert!(run_device_poll(&DeviceChoiceState::default(), None, &mut usb).is_some());
        assert_eq!(usb.probes, vec![true]);
    }

    #[test]
    fn device_choice_poll_in_chosen_probes_only_the_visible_target() {
        let chosen = chosen_a();
        let target = serial_target("A");

        // Visible outside EDL: the target-aware transports, never the EDL probe.
        let mut usb = FakeUsb::new(vec![adb("A", 1), adb("B", 2)]);
        let result = run_device_poll(&chosen, Some(&target), &mut usb).expect("a result");
        assert_eq!(usb.visible_calls, 1);
        assert_eq!(usb.probes, vec![false]);
        assert_eq!(result.status, ConnectionStatus::Adb);

        // The successor in EDL: read from the inventory, no transport opens.
        let mut usb = FakeUsb::sealed(vec![adb("B", 2), edl(5)]);
        let result = run_device_poll(&chosen, Some(&target), &mut usb).expect("a result");
        assert_eq!(result.status, ConnectionStatus::Edl);

        // Not visible, or a refused proof: not present, nothing opens.
        for visible in [Ok(false), Err(())] {
            let mut usb = FakeUsb::sealed(vec![adb("B", 2), edl(5)]);
            usb.visible = visible;
            let result = run_device_poll(&chosen, Some(&target), &mut usb).expect("a result");
            assert_eq!(result.status, ConnectionStatus::None);
            assert_eq!(usb.visible_calls, 1);
        }

        // Enumeration failure keeps the last snapshot instead of resetting the choice.
        let mut usb = FakeUsb::sealed(Vec::new());
        usb.candidates = Err(());
        assert!(run_device_poll(&chosen, Some(&target), &mut usb).is_none());
        let needs_choice = state_after(&[adb("A", 1), adb("B", 2)]);
        let mut usb = FakeUsb::sealed(Vec::new());
        usb.candidates = Err(());
        assert!(run_device_poll(&needs_choice, None, &mut usb).is_none());
    }

    #[test]
    fn device_choice_edl_row_sets_an_edl_only_target() {
        let mut app = App::default();
        let _ = app.update(polled(&[adb("A", 1), edl(2)]));
        let port = DeviceChoice::from_candidate(&edl(2)).id;
        let _ = app.update(Message::DeviceChoice(DeviceChoiceMsg::PickNow(port)));
        assert_eq!(
            test_target::calls(),
            vec![Call::Set(TargetIdentity::EdlOnly {
                location: edl(2).location
            })]
        );
    }

    /// A poll that read `serial` over ADB with `model`, alongside `candidates`.
    fn polled_as(candidates: &[UsbCandidate], serial: &str, model: &str) -> Message {
        Message::DevicePolled(DevicePollResult {
            status: ConnectionStatus::Adb,
            serial: serial.to_string(),
            model: model.to_string(),
            firmware: format!("{model}_fw"),
            inventory: candidates
                .iter()
                .map(DeviceChoice::from_candidate)
                .collect(),
            ..DevicePollResult::default()
        })
    }

    /// Two tablets, A chosen and read over ADB.
    fn chosen_a_and_read() -> App {
        let both = [adb("A", 1), adb("B", 2)];
        let mut app = App::default();
        let _ = app.update(polled(&both));
        let _ = app.update(pick("A"));
        let _ = app.update(polled_as(&both, "A", "TB-A"));
        assert_eq!(app.device.model, "TB-A");
        assert_eq!(app.device_block_reason(), None);
        app
    }

    #[test]
    fn device_choice_repick_resets_the_snapshot_and_blocks_start_until_b_is_polled() {
        let both = [adb("A", 1), adb("B", 2)];
        let mut app = chosen_a_and_read();
        // State read from A while it was the target.
        app.konabess.step = 1;
        app.konabess.prepared = Some(super::prepared());
        app.flash_parts.step = 1;
        app.dump_parts.step = 1;
        app.flash.step = 2;
        app.flash.device_region = Some(crate::DeviceRegion::Prc);
        app.root.kernel_version = Some("6.1".to_string());
        app.reboot_confirm_target = Some(crate::RebootTarget::System);

        let task = app.update(pick("B"));
        assert_eq!(task.units(), 1, "an immediate poll of B is requested");
        assert_eq!(test_target::current(), Some(serial_target("B")));
        assert_eq!(app.device.connection, ConnectionStatus::None);
        assert!(app.device.model.is_empty() && app.device.firmware.is_empty());
        assert!(app.device.serial.is_empty());
        assert!(app.device_block_reason().is_some());
        assert!(app.konabess.prepared.is_none());
        assert_eq!(app.konabess.step, 0);
        assert_eq!(app.flash_parts.step, 0);
        assert_eq!(app.dump_parts.step, 0);
        assert_eq!(app.flash.step, 0);
        assert_eq!(app.flash.device_region, None);
        assert_eq!(app.root.kernel_version, None);
        assert_eq!(app.reboot_confirm_target, None);
        assert!(app.can_poll_device());

        // A poll that has not been applied changes nothing.
        let _ = app.update(Message::PollDevice);
        let poll = app.queries.poll_in_flight.expect("poll started");
        assert!(app.device_block_reason().is_some());
        let _ = app.update(Message::DevicePollFinished(
            poll,
            Some(match polled_as(&both, "B", "TB-B") {
                Message::DevicePolled(result) => result,
                _ => unreachable!(),
            }),
        ));
        assert_eq!(app.device_block_reason(), None);
        assert_eq!(app.device.model, "TB-B");
        assert_eq!(app.device.serial, "B");
        assert_eq!(
            test_target::calls(),
            vec![Call::Set(serial_target("A")), Call::Set(serial_target("B"))]
        );
    }

    /// `pick_first` runs before the queued Start, both deferred behind a poll.
    fn deferred_reboot_after(pick_first: Option<&str>) -> App {
        let mut app = chosen_a_and_read();
        let _ = app.update(Message::PollDevice);
        let poll = app.queries.poll_in_flight.expect("poll started");
        if let Some(serial) = pick_first {
            let _ = app.update(pick(serial));
        }
        let _ = app.update(Message::Reboot(crate::RebootMsg::RebootTo(
            crate::RebootTarget::System,
        )));
        assert_eq!(
            app.queries.poll_deferred.len(),
            usize::from(pick_first.is_some()) + 1
        );
        assert!(!app.operation.is_running());
        let _ = app.update(Message::DevicePollFinished(poll, None));
        assert!(app.queries.poll_deferred.is_empty());
        app
    }

    #[test]
    fn device_choice_start_queued_before_the_swap_does_not_run() {
        // Control: without a swap the queued reboot reaches A.
        assert!(deferred_reboot_after(None).operation.is_running());
        test_target::reset();
        // Re-picking the same tablet changes nothing, so it still runs.
        assert!(deferred_reboot_after(Some("A")).operation.is_running());
        test_target::reset();
        // A swap drops it: not run, and not refused either (no error shown).
        let app = deferred_reboot_after(Some("B"));
        assert_eq!(test_target::current(), Some(serial_target("B")));
        assert!(!app.operation.is_running());
        assert_eq!(app.error_msg, None);
    }

    #[test]
    fn device_choice_scan_snapshot_keeps_the_choice_and_target() {
        let mut app = chosen_a_and_read();
        let before = app.device_choice.inventory.clone();
        let _ = app.update(Message::DumpParts(crate::DumpPartsMsg::DumpPartsScanDone(
            crate::DumpPartsScanResult {
                logs: Vec::new(),
                rows: vec![crate::DumpPartRow {
                    lun: 0,
                    label: "boot_a".to_string(),
                    start_sector: 0,
                    num_sectors: 1,
                    size_bytes: 4096,
                    selected: false,
                }],
                error: None,
            },
        )));
        assert_eq!(app.dump_parts.step, 1);
        assert_eq!(app.device.connection, ConnectionStatus::Edl);
        assert_eq!(app.device_choice.state, MultiDeviceState::Chosen);
        assert_eq!(
            app.device_choice.chosen,
            Some(ChoiceId::Serial("A".to_string()))
        );
        assert_eq!(app.device_choice.inventory, before);
        assert_eq!(test_target::current(), Some(serial_target("A")));
        assert_eq!(test_target::calls(), vec![Call::Set(serial_target("A"))]);
    }
}
