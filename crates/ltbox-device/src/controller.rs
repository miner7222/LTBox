//! Active-slot resolution across the ADB and Fastboot transports.

use crate::adb::AdbManager;
use crate::fastboot::FastbootDevice;
use ltbox_core::{i18n::tr, tr_args};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ControllerError {
    #[error("{0}")]
    SlotResolve(String),
}

/// Poll ADB then Fastboot for the active slot suffix until one returns `_a`
/// or `_b`, or the deadline expires.
///
/// Errors instead of defaulting to `_a`: a wrong-slot write to `boot_*` /
/// `vbmeta_*` / `init_boot_*` either fails AVB or leaves the device on the
/// unmodified slot while reporting success.
///
/// ADB answers in normal / recovery, Fastboot in bootloader. EDL has no
/// slot query, so callers must probe BEFORE entering EDL. `log` gets the
/// per-attempt lines.
pub fn poll_active_slot(
    timeout: std::time::Duration,
    log: &mut Vec<String>,
) -> std::result::Result<String, ControllerError> {
    let started = std::time::Instant::now();
    poll_active_slot_with(
        timeout,
        log,
        &mut DeviceSlotProbe {
            adb: AdbManager::new(),
            fastboot: None,
        },
        || started.elapsed(),
        || std::thread::sleep(std::time::Duration::from_millis(500)),
    )
}

// Keep transport construction and wall-clock waits outside the resolution policy.
trait SlotProbe {
    fn adb_state(&mut self) -> Result<Option<&'static str>, crate::adb::AdbError>;
    fn adb_slot(&mut self) -> Result<Option<String>, crate::adb::AdbError>;
    fn fastboot_open(&mut self) -> Result<(), crate::fastboot::FastbootError>;
    fn fastboot_slot(&mut self) -> Result<Option<String>, crate::fastboot::FastbootError>;
}

struct DeviceSlotProbe {
    adb: AdbManager,
    fastboot: Option<FastbootDevice>,
}

impl SlotProbe for DeviceSlotProbe {
    fn adb_state(&mut self) -> Result<Option<&'static str>, crate::adb::AdbError> {
        self.adb = AdbManager::new();
        self.adb.check_device_state()
    }
    fn adb_slot(&mut self) -> Result<Option<String>, crate::adb::AdbError> {
        self.adb.get_slot_suffix()
    }
    fn fastboot_open(&mut self) -> Result<(), crate::fastboot::FastbootError> {
        self.fastboot = None;
        self.fastboot = Some(FastbootDevice::open()?);
        Ok(())
    }
    fn fastboot_slot(&mut self) -> Result<Option<String>, crate::fastboot::FastbootError> {
        self.fastboot
            .take()
            .expect("opened fastboot device")
            .get_slot_suffix()
    }
}

fn poll_active_slot_with(
    timeout: std::time::Duration,
    log: &mut Vec<String>,
    probe: &mut impl SlotProbe,
    mut elapsed: impl FnMut() -> std::time::Duration,
    mut sleep: impl FnMut(),
) -> Result<String, ControllerError> {
    let mut adb_attempted = false;
    let mut fastboot_attempted = false;
    let mut last_adb_err = String::new();
    let mut last_fastboot_err = String::new();

    while elapsed() < timeout {
        // ADB only in a shell-capable state (device or recovery).
        match probe.adb_state() {
            Ok(Some(state @ ("device" | "recovery"))) => {
                adb_attempted = true;
                match probe.adb_slot() {
                    Ok(Some(s)) if s == "_a" || s == "_b" => {
                        ltbox_core::live!(
                            log,
                            "[Slot] {}",
                            ltbox_core::tr_args!("log_slot_resolved_adb", state = state, slot = s,)
                        );
                        return Ok(s);
                    }
                    Ok(Some(other)) => {
                        last_adb_err = tr_args!("slot_err_adb_unexpected", slot = other);
                    }
                    Ok(None) => {
                        last_adb_err = tr("slot_err_adb_empty");
                    }
                    Err(e) => {
                        last_adb_err = tr_args!("slot_err_adb_shell_failed", error = e);
                    }
                }
            }
            Ok(Some(state)) => {
                last_adb_err = tr_args!("slot_err_adb_state_no_shell", state = state);
            }
            Ok(None) => {
                last_adb_err = tr("slot_err_adb_no_device");
            }
            // Which device to talk to is not settled: retrying until the deadline
            // cannot fix it, so stop at once.
            Err(
                e @ (crate::adb::AdbError::AmbiguousTarget
                | crate::adb::AdbError::TargetMismatch { .. }
                | crate::adb::AdbError::Selection(_)),
            ) => {
                return Err(ControllerError::SlotResolve(e.to_string()));
            }
            Err(e) => {
                last_adb_err = tr_args!("slot_err_adb_probe_failed", error = e);
            }
        }

        // open() fails fast outside fastboot, so no separate state probe.
        match probe.fastboot_open() {
            Ok(()) => {
                fastboot_attempted = true;
                match probe.fastboot_slot() {
                    Ok(Some(s)) if s == "_a" || s == "_b" => {
                        ltbox_core::live!(
                            log,
                            "[Slot] {}",
                            ltbox_core::tr_args!("log_slot_resolved_fastboot", slot = s)
                        );
                        return Ok(s);
                    }
                    Ok(Some(other)) => {
                        last_fastboot_err = tr_args!("slot_err_fastboot_unexpected", slot = other);
                    }
                    Ok(None) => {
                        last_fastboot_err = tr("slot_err_fastboot_empty");
                    }
                    Err(e) => {
                        last_fastboot_err = tr_args!("slot_err_fastboot_getvar_failed", error = e);
                    }
                }
            }
            Err(
                e @ (crate::fastboot::FastbootError::MultipleDevices
                | crate::fastboot::FastbootError::AmbiguousTarget
                | crate::fastboot::FastbootError::TargetMismatch { .. }
                | crate::fastboot::FastbootError::Selection(_)),
            ) => {
                return Err(ControllerError::SlotResolve(e.to_string()));
            }
            Err(e) => {
                last_fastboot_err = tr_args!("slot_err_fastboot_open_failed", error = e);
            }
        }

        sleep();
    }

    // Report what was tried and each transport's last failure.
    let mut detail = String::new();
    if adb_attempted {
        detail.push_str(&tr_args!("slot_err_adb_detail", error = last_adb_err));
    } else {
        detail.push_str(&tr("slot_err_adb_never_shell"));
    }
    detail.push(' ');
    if fastboot_attempted {
        detail.push_str(&tr_args!(
            "slot_err_fastboot_detail",
            error = last_fastboot_err
        ));
    } else {
        detail.push_str(&tr("slot_err_fastboot_never"));
    }
    Err(ControllerError::SlotResolve(tr_args!(
        "err_active_slot_detect_failed",
        timeout = timeout.as_secs_f64().to_string(),
        detail = detail
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, collections::VecDeque, time::Duration};

    enum Reply {
        State(Option<&'static str>),
        StateErr(fn() -> crate::adb::AdbError),
        OpenErr(fn() -> crate::fastboot::FastbootError),
        Adb(Option<&'static str>),
        AdbFailure,
        Open(bool),
        Fastboot(Option<&'static str>),
    }
    struct Script(VecDeque<Reply>);
    impl SlotProbe for Script {
        fn adb_state(&mut self) -> Result<Option<&'static str>, crate::adb::AdbError> {
            match self.0.pop_front() {
                Some(Reply::State(state)) => Ok(state),
                Some(Reply::StateErr(error)) => Err(error()),
                _ => panic!("unexpected ADB probe"),
            }
        }
        fn adb_slot(&mut self) -> Result<Option<String>, crate::adb::AdbError> {
            match self.0.pop_front() {
                Some(Reply::Adb(slot)) => Ok(slot.map(str::to_owned)),
                Some(Reply::AdbFailure) => Err(crate::adb::AdbError::Timeout),
                _ => panic!("unexpected ADB slot query"),
            }
        }
        fn fastboot_open(&mut self) -> Result<(), crate::fastboot::FastbootError> {
            match self.0.pop_front() {
                Some(Reply::Open(true)) => Err(crate::fastboot::FastbootError::MultipleDevices),
                Some(Reply::Open(false)) => Ok(()),
                Some(Reply::OpenErr(error)) => Err(error()),
                _ => panic!("unexpected Fastboot open"),
            }
        }
        fn fastboot_slot(&mut self) -> Result<Option<String>, crate::fastboot::FastbootError> {
            let Some(Reply::Fastboot(slot)) = self.0.pop_front() else {
                panic!("unexpected Fastboot slot query")
            };
            Ok(slot.map(str::to_owned))
        }
    }

    fn resolve(replies: Vec<Reply>, polls: u64) -> (Result<String, ControllerError>, u64) {
        let mut script = Script(replies.into());
        let ticks = Cell::new(0);
        let result = poll_active_slot_with(
            Duration::from_secs(polls),
            &mut Vec::new(),
            &mut script,
            || Duration::from_secs(ticks.get()),
            || ticks.set(ticks.get() + 1),
        );
        assert!(
            script.0.is_empty(),
            "resolution skipped expected transport calls"
        );
        (result, ticks.get())
    }

    #[test]
    fn adb_device_and_recovery_preserve_active_b_without_fastboot() {
        for state in ["device", "recovery"] {
            let (result, waits) =
                resolve(vec![Reply::State(Some(state)), Reply::Adb(Some("_b"))], 1);
            assert_eq!(result.unwrap(), "_b");
            assert_eq!(waits, 0);
        }
    }

    #[test]
    fn invalid_adb_slot_falls_through_to_fastboot() {
        let (result, _) = resolve(
            vec![
                Reply::State(Some("device")),
                Reply::Adb(Some("garbage")),
                Reply::Open(false),
                Reply::Fastboot(Some("_b")),
            ],
            1,
        );
        assert_eq!(result.unwrap(), "_b");
    }

    #[test]
    fn adb_shell_error_still_allows_fastboot_b() {
        let (result, waits) = resolve(
            vec![
                Reply::State(Some("device")),
                Reply::AdbFailure,
                Reply::Open(false),
                Reply::Fastboot(Some("_b")),
            ],
            1,
        );
        assert_eq!(result.unwrap(), "_b");
        assert_eq!(waits, 0);
    }

    #[test]
    fn unavailable_slots_timeout_without_guessing_a() {
        let (result, waits) = resolve(
            vec![
                Reply::State(Some("unauthorized")),
                Reply::Open(false),
                Reply::Fastboot(None),
            ],
            1,
        );
        assert!(result.is_err());
        assert_eq!(waits, 1);
    }

    #[test]
    fn retries_probe_fresh_state_after_waiting() {
        let (result, waits) = resolve(
            vec![
                Reply::State(None),
                Reply::Open(false),
                Reply::Fastboot(None),
                Reply::State(Some("recovery")),
                Reply::Adb(Some("_b")),
            ],
            2,
        );
        assert_eq!(result.unwrap(), "_b");
        assert_eq!(waits, 1);
    }

    #[test]
    fn ambiguous_fastboot_stops_without_retrying() {
        let (result, waits) = resolve(vec![Reply::State(None), Reply::Open(true)], 5);
        assert!(result.is_err());
        assert_eq!(waits, 0);
    }

    #[test]
    fn adb_selection_errors_stop_without_retrying_or_trying_fastboot() {
        use crate::adb::AdbError;
        let errors: [fn() -> AdbError; 3] = [
            || AdbError::AmbiguousTarget,
            || AdbError::TargetMismatch {
                expected: "A".into(),
                found: "B".into(),
            },
            || AdbError::Selection("no successor".into()),
        ];
        for error in errors {
            let (result, waits) = resolve(vec![Reply::StateErr(error)], 5);
            assert!(result.is_err());
            assert_eq!(waits, 0);
        }
    }

    #[test]
    fn fastboot_selection_errors_stop_without_retrying() {
        use crate::fastboot::FastbootError;
        let errors: [fn() -> FastbootError; 4] = [
            || FastbootError::MultipleDevices,
            || FastbootError::AmbiguousTarget,
            || FastbootError::TargetMismatch {
                expected: "A".into(),
                found: "B".into(),
            },
            || FastbootError::Selection("no successor".into()),
        ];
        for error in errors {
            let (result, waits) = resolve(vec![Reply::State(None), Reply::OpenErr(error)], 5);
            assert!(result.is_err());
            assert_eq!(waits, 0);
        }
    }
}
