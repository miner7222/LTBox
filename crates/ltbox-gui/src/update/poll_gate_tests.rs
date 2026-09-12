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
