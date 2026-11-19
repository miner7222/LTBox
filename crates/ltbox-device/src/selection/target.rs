//! Session target: which attached device the app works on, and the proof that a device
//! that changed mode is still that target.
//!
//! Nothing here opens a transport. [`resolve`] takes the USB census and runs the pure
//! [`decide`] under one lock; the transports trust its answer.

use std::sync::{Mutex, MutexGuard, PoisonError};

use super::{DeviceMode, UsbCandidate, UsbLocation, UsbSelectionError};

/// The device chosen for this session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetIdentity {
    /// Chosen in ADB or Fastboot; matched by its trimmed USB iSerial, exactly.
    Serial { serial: String },
    /// Chosen while in EDL, where no serial is read; matched by its USB location.
    EdlOnly { location: UsbLocation },
}

#[derive(Debug, Clone)]
struct TargetState {
    identity: TargetIdentity,
    /// The last census in which the target was seen by its identity.
    witness: Vec<UsbCandidate>,
    /// Location proven to hold a `Serial` target during its current EDL stay.
    edl_binding: Option<UsbLocation>,
}

impl TargetState {
    fn new(identity: TargetIdentity) -> Self {
        Self {
            identity,
            witness: Vec::new(),
            edl_binding: None,
        }
    }
}

static TARGET: Mutex<Option<TargetState>> = Mutex::new(None);

fn lock_state(
    state: &Mutex<Option<TargetState>>,
) -> Result<MutexGuard<'_, Option<TargetState>>, UsbSelectionError> {
    state.lock().map_err(|_| UsbSelectionError::StatePoisoned)
}

/// Overwrites the whole state, so a poisoned lock is safe to recover here.
fn replace_state(state: &Mutex<Option<TargetState>>, value: Option<TargetState>) {
    let mut guard = state.lock().unwrap_or_else(PoisonError::into_inner);
    *guard = value;
    state.clear_poison();
}

/// Reads the stored identity even from a poisoned lock: reporting "no target" there
/// would be a fallback to any device.
fn peek_identity(state: &Mutex<Option<TargetState>>) -> Option<TargetIdentity> {
    state
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .map(|target| target.identity.clone())
}

/// Chooses the session target. The witness and the EDL binding start empty.
pub fn set_target(target: TargetIdentity) {
    replace_state(&TARGET, Some(TargetState::new(target)));
}

/// Drops the session target; transports return to the single-device rule.
pub fn clear_target() {
    replace_state(&TARGET, None);
}

pub fn current_target() -> Option<TargetIdentity> {
    peek_identity(&TARGET)
}

/// The candidate that is the target in `mode`, or `None` when it is not there (yet).
/// Without a target this is the single-device rule. See [`decide`].
pub fn resolve(mode: DeviceMode) -> Result<Option<UsbCandidate>, UsbSelectionError> {
    // Enumerate before locking: `list_candidates` reads string descriptors, which can be
    // slow, and `set_target` runs on the GUI thread and takes this lock.
    let census = super::list_candidates()?;
    let mut state = lock_state(&TARGET)?;
    let index = decide(&census, &mut state, mode)?;
    Ok(index.map(|index| census[index].clone()))
}

/// Whether the target is attached in any mode, for the idle poll. `false` without a target.
/// Never binds an EDL device and never changes the identity. See [`visible`].
pub fn target_visible() -> Result<bool, UsbSelectionError> {
    // The lock is not held while enumerating (see `resolve`): check for a target, release,
    // enumerate, then lock again for `visible`.
    if lock_state(&TARGET)?.is_none() {
        return Ok(false);
    }
    let census = super::list_candidates()?;
    let mut state = lock_state(&TARGET)?;
    visible(&census, &mut state)
}

/// Without a target: the strict count rule of [`super::ensure_single_usb_target`].
/// With a target the other devices are ignored, so this is `Ok`.
pub fn ensure_unambiguous_target() -> Result<(), UsbSelectionError> {
    let state = lock_state(&TARGET)?;
    if state.is_some() {
        Ok(())
    } else {
        super::ensure_single_usb_target()
    }
}

/// For transports that took the untargeted path: the strict count rule of
/// [`super::ensure_single_usb_target`], under the same lock that reads the target, so a
/// target chosen while the caller was already running cannot make it skip the guard.
///
/// A target set at this point is `AmbiguousTarget`: the caller decided on the single-device
/// rule earlier, and which device it may open is no longer settled. It is a hard error, never
/// a fallback to any device; the caller's next call takes the targeted path.
pub fn ensure_untargeted_single() -> Result<(), UsbSelectionError> {
    let state = lock_state(&TARGET)?;
    if state.is_some() {
        Err(UsbSelectionError::AmbiguousTarget)
    } else {
        super::ensure_single_usb_target()
    }
}

/// Index of the census entry that is the target in `mode`. Pure apart from `state`.
///
/// - No target: two or more candidates in total is `MultipleDevices`; otherwise the sole
///   candidate if its mode matches.
/// - Serials match exactly (trimmed target, no case folding) and only on non-EDL entries.
///   Two matches in any mode is `AmbiguousTarget`. An empty target serial is
///   `UnprovenSuccessor` in every mode.
/// - `Serial` target, any request: one match refreshes the witness and drops the EDL
///   binding. An EDL request checks the old witness first: if a 9008 device not in the old
///   witness is present while the target is still visible by serial, that is
///   `UnprovenSuccessor`; otherwise `None`.
/// - `Serial` target, EDL request, binding present: the one 9008 device at the bound
///   location, `None` if absent (a `reset_to_edl` bounce keeps the binding), `AmbiguousTarget`
///   if two.
/// - `Serial` target, EDL request, no binding: the successor proof. No new 9008 device is
///   `None` (wait). Sole-device rule: the only candidate is a new 9008 device and the witness
///   holds no device other than the target (or is empty), so bind it. Otherwise an empty
///   witness, two or more new 9008 devices, a new device without a usable location, or
///   another witness device that is gone (matched by serial, or EDL by location) is
///   `UnprovenSuccessor`; else bind the one new device.
/// - `EdlOnly` target: an EDL request finds the one 9008 device at its location (refreshes
///   the witness). It never adopts a serial: ADB and Fastboot requests are `None`.
/// - A location with an empty bus id or port chain identifies nothing and is never bound.
///   Locations are compared EDL to EDL only; equal old/new locations are logged, never
///   required.
fn decide(
    census: &[UsbCandidate],
    state: &mut Option<TargetState>,
    mode: DeviceMode,
) -> Result<Option<usize>, UsbSelectionError> {
    let Some(target) = state.as_mut() else {
        return no_target(census, mode);
    };
    let found = find(census, target, mode == DeviceMode::Edl)?;
    Ok(found.filter(|&index| census[index].mode == mode))
}

/// Whether the target is in `census` in any mode: the idle-poll view of [`decide`].
///
/// Persists only direct sightings: a serial match refreshes the witness and drops the
/// binding, and a bound location that holds no 9008 device drops the binding, so the binding
/// does not outlive a socket swap between polls. The successor proof is only evaluated; this
/// never binds and never changes the identity.
fn visible(
    census: &[UsbCandidate],
    state: &mut Option<TargetState>,
) -> Result<bool, UsbSelectionError> {
    let Some(target) = state.as_mut() else {
        return Ok(false);
    };
    let serial = match &target.identity {
        TargetIdentity::EdlOnly { .. } => return Ok(find(census, target, true)?.is_some()),
        TargetIdentity::Serial { serial } => serial.trim().to_owned(),
    };
    if find_serial(census, target, &serial, false)?.is_some() {
        return Ok(true);
    }
    if let Some(location) = &target.edl_binding {
        if edl_at(census, location)?.is_some() {
            return Ok(true);
        }
        target.edl_binding = None;
    }
    Ok(prove_edl_successor(census, target, &serial)?.is_some())
}

fn no_target(
    census: &[UsbCandidate],
    mode: DeviceMode,
) -> Result<Option<usize>, UsbSelectionError> {
    match census {
        [] => Ok(None),
        [only] => Ok((only.mode == mode).then_some(0)),
        _ => Err(UsbSelectionError::MultipleDevices),
    }
}

fn find(
    census: &[UsbCandidate],
    target: &mut TargetState,
    edl: bool,
) -> Result<Option<usize>, UsbSelectionError> {
    match target.identity.clone() {
        TargetIdentity::Serial { serial } => find_serial(census, target, serial.trim(), edl),
        TargetIdentity::EdlOnly { location } => find_edl_only(census, target, &location, edl),
    }
}

fn unproven(reason: impl Into<String>) -> UsbSelectionError {
    UsbSelectionError::UnprovenSuccessor(reason.into())
}

fn location_usable(location: &UsbLocation) -> bool {
    !location.bus_id.is_empty() && !location.port_chain.is_empty()
}

fn has_serial(candidate: &UsbCandidate, serial: &str) -> bool {
    candidate.mode != DeviceMode::Edl && candidate.serial.as_deref() == Some(serial)
}

fn serial_hits(census: &[UsbCandidate], serial: &str) -> Vec<usize> {
    (0..census.len())
        .filter(|&index| has_serial(&census[index], serial))
        .collect()
}

/// The same physical device across two inventories: equal serial outside EDL, equal usable
/// location inside EDL. Never across the EDL boundary.
fn same_device(old: &UsbCandidate, new: &UsbCandidate) -> bool {
    match (old.mode == DeviceMode::Edl, new.mode == DeviceMode::Edl) {
        (false, false) => old.serial.is_some() && old.serial == new.serial,
        (true, true) => location_usable(&old.location) && old.location == new.location,
        _ => false,
    }
}

fn describe(candidate: &UsbCandidate) -> String {
    match &candidate.serial {
        Some(serial) if candidate.mode != DeviceMode::Edl => {
            format!("{:?} device {serial}", candidate.mode)
        }
        _ => format!(
            "{:?} device at bus {:?} port {:?}",
            candidate.mode, candidate.location.bus_id, candidate.location.port_chain
        ),
    }
}

/// Census indices of `mode_edl` devices (9008 or non-EDL) that do not pair one-to-one with a
/// witness entry. Entries without a serial (non-EDL) or a usable location (EDL) never pair.
fn new_devices(census: &[UsbCandidate], witness: &[UsbCandidate], mode_edl: bool) -> Vec<usize> {
    let mut paired = vec![false; witness.len()];
    let mut fresh = Vec::new();
    for (index, candidate) in census.iter().enumerate() {
        if (candidate.mode == DeviceMode::Edl) != mode_edl {
            continue;
        }
        let pair = (0..witness.len()).find(|&w| !paired[w] && same_device(&witness[w], candidate));
        match pair {
            Some(w) => paired[w] = true,
            None => fresh.push(index),
        }
    }
    fresh
}

/// Proof rule 3: every `others` entry pairs one-to-one with a census entry other than the
/// successor.
fn others_present(
    census: &[UsbCandidate],
    others: &[&UsbCandidate],
    successor: usize,
) -> Result<(), UsbSelectionError> {
    let mut used = vec![false; census.len()];
    used[successor] = true;
    for &old in others {
        let slot =
            (0..census.len()).find(|&index| !used[index] && same_device(old, &census[index]));
        match slot {
            Some(index) => used[index] = true,
            None => {
                return Err(unproven(format!(
                    "{} was attached with the chosen device and is gone",
                    describe(old)
                )));
            }
        }
    }
    Ok(())
}

fn log_location_corroboration(old: Option<&UsbCandidate>, new: &UsbCandidate) {
    let same_port = old.is_some_and(|old| old.location == new.location);
    tracing::info!(
        same_port,
        bus_id = %new.location.bus_id,
        port_chain = ?new.location.port_chain,
        "device succession proven"
    );
}

fn edl_at(
    census: &[UsbCandidate],
    location: &UsbLocation,
) -> Result<Option<usize>, UsbSelectionError> {
    let hits: Vec<usize> = (0..census.len())
        .filter(|&index| {
            census[index].mode == DeviceMode::Edl && census[index].location == *location
        })
        .collect();
    match hits.as_slice() {
        [] => Ok(None),
        [only] => Ok(Some(*only)),
        _ => Err(UsbSelectionError::AmbiguousTarget),
    }
}

fn find_serial(
    census: &[UsbCandidate],
    target: &mut TargetState,
    serial: &str,
    edl: bool,
) -> Result<Option<usize>, UsbSelectionError> {
    if serial.is_empty() {
        return Err(unproven("the chosen device has no serial number"));
    }
    let hits = serial_hits(census, serial);
    if hits.len() > 1 {
        return Err(UsbSelectionError::AmbiguousTarget);
    }
    if let Some(&index) = hits.first() {
        // Judge against the old witness before this census replaces it.
        let stranger_in_edl = edl
            && !target.witness.is_empty()
            && !new_devices(census, &target.witness, true).is_empty();
        target.witness = census.to_vec();
        target.edl_binding = None;
        if stranger_in_edl {
            return Err(unproven(
                "a new EDL device appeared while the chosen device is still attached by serial",
            ));
        }
        return Ok((!edl).then_some(index));
    }
    if !edl {
        return Ok(None);
    }
    if let Some(location) = &target.edl_binding {
        return edl_at(census, location);
    }
    let Some(successor) = prove_edl_successor(census, target, serial)? else {
        return Ok(None);
    };
    let old = target.witness.iter().find(|old| has_serial(old, serial));
    log_location_corroboration(old, &census[successor]);
    target.edl_binding = Some(census[successor].location.clone());
    Ok(Some(successor))
}

/// The census index of the proven EDL successor of a `Serial` target, `None` while no new
/// 9008 device is present. Binding is left to the caller.
fn prove_edl_successor(
    census: &[UsbCandidate],
    target: &TargetState,
    serial: &str,
) -> Result<Option<usize>, UsbSelectionError> {
    let fresh = new_devices(census, &target.witness, true);
    if fresh.is_empty() {
        return Ok(None);
    }
    // A serial target exists only because a picker saw several devices, so an empty witness
    // proves nothing: the sole EDL device may be one of those others.
    if target.witness.is_empty() {
        return Err(unproven(
            "the chosen device has not been seen by serial since it was chosen",
        ));
    }
    // Today's single-device behaviour, but only when nothing else was attached with the
    // target: otherwise the sole device may be one of those others, now in EDL.
    let sole_device = census.len() == 1 && target.witness.iter().all(|old| has_serial(old, serial));
    if !sole_device && fresh.len() > 1 {
        return Err(unproven(format!(
            "{} new EDL devices appeared",
            fresh.len()
        )));
    }
    let successor = fresh[0];
    if !location_usable(&census[successor].location) {
        return Err(unproven("the new EDL device has no USB location"));
    }
    let others: Vec<&UsbCandidate> = target
        .witness
        .iter()
        .filter(|old| !has_serial(old, serial))
        .collect();
    others_present(census, &others, successor)?;
    Ok(Some(successor))
}

fn find_edl_only(
    census: &[UsbCandidate],
    target: &mut TargetState,
    location: &UsbLocation,
    edl: bool,
) -> Result<Option<usize>, UsbSelectionError> {
    if !location_usable(location) {
        return Err(unproven("the chosen EDL device has no USB location"));
    }
    // Chosen in EDL, the target has no serial to be recognised by once it leaves EDL: a new
    // ADB or Fastboot device could be any device plugged in meanwhile.
    if !edl {
        return Ok(None);
    }
    let found = edl_at(census, location)?;
    if found.is_some() {
        target.witness = census.to_vec();
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use DeviceMode::{Adb, Edl, Fastboot};

    fn loc(bus: &str, chain: &[u8]) -> UsbLocation {
        UsbLocation {
            bus_id: bus.to_owned(),
            port_chain: chain.to_vec(),
        }
    }

    fn device(mode: DeviceMode, serial: Option<&str>, location: UsbLocation) -> UsbCandidate {
        let (vendor_id, product_id) = match mode {
            Edl => (0x05c6, 0x9008),
            _ => (0x17ef, 0x7a3e),
        };
        UsbCandidate {
            mode,
            serial: serial.map(str::to_owned),
            location,
            vendor_id,
            product_id,
            product: None,
            instance_id: None,
        }
    }

    fn adb(serial: &str, port: u8) -> UsbCandidate {
        device(Adb, Some(serial), loc("2", &[port]))
    }

    fn fastboot(serial: &str, port: u8) -> UsbCandidate {
        device(Fastboot, Some(serial), loc("1", &[port]))
    }

    fn edl(port: u8) -> UsbCandidate {
        device(Edl, None, loc("1", &[port]))
    }

    fn by_serial(serial: &str) -> Option<TargetState> {
        Some(TargetState::new(TargetIdentity::Serial {
            serial: serial.to_owned(),
        }))
    }

    fn by_location(port: u8) -> Option<TargetState> {
        Some(TargetState::new(TargetIdentity::EdlOnly {
            location: loc("1", &[port]),
        }))
    }

    fn binding(state: &Option<TargetState>) -> Option<UsbLocation> {
        state.as_ref().unwrap().edl_binding.clone()
    }

    fn identity(state: &Option<TargetState>) -> TargetIdentity {
        state.as_ref().unwrap().identity.clone()
    }

    fn is_unproven(result: Result<Option<usize>, UsbSelectionError>) -> bool {
        matches!(result, Err(UsbSelectionError::UnprovenSuccessor(_)))
    }

    fn is_ambiguous(result: Result<Option<usize>, UsbSelectionError>) -> bool {
        matches!(result, Err(UsbSelectionError::AmbiguousTarget))
    }

    /// Target A in ADB beside B in ADB, witnessed by one ADB resolve.
    fn witnessed_a_beside_b() -> Option<TargetState> {
        let mut state = by_serial("A");
        let census = [adb("A", 1), adb("B", 2)];
        assert_eq!(decide(&census, &mut state, Adb).unwrap(), Some(0));
        state
    }

    /// `witnessed_a_beside_b`, then A re-enumerated in EDL and was bound.
    fn bound_a_beside_b() -> Option<TargetState> {
        let mut state = witnessed_a_beside_b();
        let census = [adb("B", 2), edl(1)];
        assert_eq!(decide(&census, &mut state, Edl).unwrap(), Some(1));
        assert_eq!(binding(&state), Some(loc("1", &[1])));
        state
    }

    #[test]
    fn no_target_keeps_the_single_device_rule() {
        let mut state = None;
        assert_eq!(decide(&[], &mut state, Adb).unwrap(), None);
        assert_eq!(decide(&[adb("A", 1)], &mut state, Adb).unwrap(), Some(0));
        assert_eq!(decide(&[edl(1)], &mut state, Edl).unwrap(), Some(0));
        assert_eq!(decide(&[adb("A", 1)], &mut state, Fastboot).unwrap(), None);
        assert_eq!(decide(&[edl(1)], &mut state, Adb).unwrap(), None);
        for mode in [Adb, Fastboot, Edl] {
            assert!(matches!(
                decide(&[adb("A", 1), adb("B", 2)], &mut state, mode),
                Err(UsbSelectionError::MultipleDevices)
            ));
            assert!(matches!(
                decide(&[adb("A", 1), edl(2)], &mut state, mode),
                Err(UsbSelectionError::MultipleDevices)
            ));
        }
        assert!(state.is_none());
    }

    #[test]
    fn serial_target_is_found_among_two_adb_devices_in_either_order() {
        let mut state = by_serial("A");
        assert_eq!(
            decide(&[adb("A", 1), adb("B", 2)], &mut state, Adb).unwrap(),
            Some(0)
        );
        let mut state = by_serial("A");
        assert_eq!(
            decide(&[adb("B", 2), adb("A", 1)], &mut state, Adb).unwrap(),
            Some(1)
        );
        let mut state = by_serial("  A ");
        assert_eq!(
            decide(&[adb("B", 2), adb("A", 1)], &mut state, Adb).unwrap(),
            Some(1)
        );
    }

    #[test]
    fn serial_match_is_exact_and_mode_filtered() {
        let mut state = by_serial("a");
        assert_eq!(decide(&[adb("A", 1)], &mut state, Adb).unwrap(), None);
        let mut state = by_serial("A");
        assert_eq!(decide(&[fastboot("A", 1)], &mut state, Adb).unwrap(), None);
        assert_eq!(
            decide(&[fastboot("A", 1)], &mut state, Fastboot).unwrap(),
            Some(0)
        );
        assert_eq!(decide(&[adb("B", 2)], &mut state, Adb).unwrap(), None);
    }

    #[test]
    fn duplicate_serial_is_ambiguous_in_every_mode() {
        for census in [
            vec![adb("A", 1), adb("A", 2)],
            vec![adb("A", 1), fastboot("A", 2)],
        ] {
            for mode in [Adb, Fastboot, Edl] {
                let mut state = by_serial("A");
                assert!(is_ambiguous(decide(&census, &mut state, mode)));
            }
        }
    }

    #[test]
    fn empty_target_serial_is_refused_even_for_a_sole_device() {
        for census in [vec![adb("A", 1)], vec![edl(1)], vec![]] {
            for mode in [Adb, Edl] {
                let mut state = by_serial(" ");
                assert!(is_unproven(decide(&census, &mut state, mode)));
            }
        }
    }

    #[test]
    fn adb_to_edl_proof_passes_with_the_other_device_unchanged() {
        let mut state = bound_a_beside_b();
        // The binding then answers directly.
        assert_eq!(
            decide(&[adb("B", 2), edl(1)], &mut state, Edl).unwrap(),
            Some(1)
        );
        assert_eq!(
            decide(&[adb("B", 2), edl(1)], &mut state, Adb).unwrap(),
            None
        );
    }

    #[test]
    fn proof_passes_when_the_port_chain_differs_between_speeds() {
        let mut state = by_serial("A");
        // ADB at SuperSpeed on bus 3, EDL at high speed on bus 1 behind another hub port.
        let a = device(Adb, Some("A"), loc("3", &[2]));
        let b = device(Adb, Some("B"), loc("3", &[4]));
        decide(&[a, b.clone()], &mut state, Adb).unwrap();
        let successor = device(Edl, None, loc("1", &[5, 2]));
        let census = [b, successor];
        assert_eq!(decide(&census, &mut state, Edl).unwrap(), Some(1));
        assert_eq!(binding(&state), Some(loc("1", &[5, 2])));
    }

    #[test]
    fn refuses_two_new_edl_devices() {
        let mut state = witnessed_a_beside_b();
        assert!(is_unproven(decide(
            &[adb("B", 2), edl(1), edl(3)],
            &mut state,
            Edl
        )));
        assert_eq!(binding(&state), None);
    }

    #[test]
    fn refuses_while_the_target_is_still_visible_in_adb() {
        let mut state = witnessed_a_beside_b();
        let census = [adb("A", 1), adb("B", 2), edl(3)];
        assert!(is_unproven(decide(&census, &mut state, Edl)));
        assert_eq!(binding(&state), None);
        // The refused census became the witness, so that EDL device is now known not to be
        // the target: once A is gone it is not new, and the app waits.
        assert_eq!(
            decide(&[adb("B", 2), edl(3)], &mut state, Edl).unwrap(),
            None
        );
        // A's own EDL device then binds.
        assert_eq!(
            decide(&[adb("B", 2), edl(3), edl(1)], &mut state, Edl).unwrap(),
            Some(2)
        );
    }

    #[test]
    fn target_visible_without_a_new_edl_device_waits() {
        let mut state = witnessed_a_beside_b();
        assert_eq!(
            decide(&[adb("A", 1), adb("B", 2)], &mut state, Edl).unwrap(),
            None
        );
    }

    #[test]
    fn refuses_when_another_witness_device_vanished() {
        let mut state = by_serial("A");
        decide(&[adb("A", 1), adb("B", 2), adb("C", 3)], &mut state, Adb).unwrap();
        assert!(is_unproven(decide(&[edl(1), adb("C", 3)], &mut state, Edl)));

        // B vanished and an unknown D appeared in its place.
        let mut state = witnessed_a_beside_b();
        assert!(is_unproven(decide(&[edl(1), adb("D", 2)], &mut state, Edl)));

        // B left ADB for EDL: two new EDL devices.
        let mut state = witnessed_a_beside_b();
        assert!(is_unproven(decide(&[edl(1), edl(2)], &mut state, Edl)));

        // Two other devices shared serial B; one of them vanished.
        let mut state = by_serial("A");
        decide(&[adb("A", 1), adb("B", 2), adb("B", 3)], &mut state, Adb).unwrap();
        assert!(is_unproven(decide(&[edl(1), adb("B", 3)], &mut state, Edl)));
        assert_eq!(binding(&state), None);
    }

    #[test]
    fn refuses_when_a_witness_device_cannot_be_recognised() {
        // An ADB device without a serial cannot be shown to still be there.
        let mut state = by_serial("A");
        decide(
            &[adb("A", 1), device(Adb, None, loc("2", &[2]))],
            &mut state,
            Adb,
        )
        .unwrap();
        let census = [edl(1), device(Adb, None, loc("2", &[2]))];
        assert!(is_unproven(decide(&census, &mut state, Edl)));

        // Neither can an EDL device without a location.
        let mut state = by_serial("A");
        let nowhere = device(Edl, None, loc("", &[]));
        decide(&[adb("A", 1), nowhere.clone()], &mut state, Adb).unwrap();
        assert!(is_unproven(decide(&[edl(1), nowhere], &mut state, Edl)));
    }

    #[test]
    fn waits_while_no_new_edl_device_has_appeared() {
        let mut state = witnessed_a_beside_b();
        assert_eq!(decide(&[adb("B", 2)], &mut state, Edl).unwrap(), None);
        assert_eq!(binding(&state), None);
    }

    #[test]
    fn sole_edl_device_is_accepted_when_only_the_target_was_witnessed() {
        let mut state = by_serial("A");
        decide(&[adb("A", 1)], &mut state, Adb).unwrap();
        assert_eq!(decide(&[edl(1)], &mut state, Edl).unwrap(), Some(0));
        assert_eq!(binding(&state), Some(loc("1", &[1])));
    }

    #[test]
    fn sole_edl_device_is_refused_before_any_witness() {
        // Picked from [A, B] in ADB, then the first census shows only an EDL device: it may be B.
        let mut state = by_serial("A");
        assert!(is_unproven(decide(&[edl(4)], &mut state, Edl)));
        assert_eq!(binding(&state), None);
        assert!(matches!(
            visible(&[edl(4)], &mut state),
            Err(UsbSelectionError::UnprovenSuccessor(_))
        ));
    }

    #[test]
    fn sole_edl_device_is_refused_when_another_device_was_witnessed() {
        // A and B in ADB; B entered EDL while A is slow to enumerate. The sole EDL device is
        // new, but it may be B, and B is gone by serial.
        let mut state = witnessed_a_beside_b();
        assert!(is_unproven(decide(&[edl(2)], &mut state, Edl)));
        assert_eq!(binding(&state), None);
        // The same holds when the other device was simply unplugged.
        let mut state = witnessed_a_beside_b();
        assert!(is_unproven(decide(&[edl(1)], &mut state, Edl)));
        assert_eq!(binding(&state), None);
    }

    #[test]
    fn sole_edl_device_already_in_the_witness_is_not_the_target() {
        let mut state = by_serial("A");
        decide(&[adb("A", 1), edl(2)], &mut state, Adb).unwrap();
        // A left ADB; only the device that sat beside it in EDL remains.
        assert_eq!(decide(&[edl(2)], &mut state, Edl).unwrap(), None);
        assert_eq!(binding(&state), None);
        assert_eq!(decide(&[edl(2), edl(1)], &mut state, Edl).unwrap(), Some(1));
    }

    #[test]
    fn edl_device_without_a_location_is_never_bound() {
        let nowhere = device(Edl, None, loc("", &[]));
        let no_chain = device(Edl, None, loc("1", &[]));
        for successor in [nowhere, no_chain] {
            // Sole device.
            let mut state = by_serial("A");
            assert!(is_unproven(decide(
                std::slice::from_ref(&successor),
                &mut state,
                Edl
            )));
            // Proven successor otherwise.
            let mut state = witnessed_a_beside_b();
            assert!(is_unproven(decide(
                &[adb("B", 2), successor],
                &mut state,
                Edl
            )));
            assert_eq!(binding(&state), None);
        }
    }

    #[test]
    fn empty_witness_proves_nothing_beside_another_device() {
        let mut state = by_serial("A");
        assert!(is_unproven(decide(&[adb("B", 2), edl(1)], &mut state, Edl)));
        assert_eq!(binding(&state), None);
    }

    #[test]
    fn first_sighting_with_an_edl_device_present_only_fills_the_witness() {
        let mut state = by_serial("A");
        assert_eq!(
            decide(&[adb("A", 1), edl(2)], &mut state, Edl).unwrap(),
            None
        );
        assert_eq!(state.as_ref().unwrap().witness, vec![adb("A", 1), edl(2)]);
    }

    #[test]
    fn binding_is_dropped_once_the_target_is_seen_again_by_serial() {
        let mut state = bound_a_beside_b();
        assert_eq!(
            decide(&[adb("A", 1), adb("B", 2)], &mut state, Adb).unwrap(),
            Some(0)
        );
        assert_eq!(binding(&state), None);
        // The old binding would answer port 1; a new EDL entry needs its own proof.
        assert!(is_unproven(decide(
            &[adb("B", 2), edl(1), edl(3)],
            &mut state,
            Edl
        )));
        assert_eq!(
            decide(&[adb("B", 2), edl(1)], &mut state, Edl).unwrap(),
            Some(1)
        );
    }

    #[test]
    fn binding_is_dropped_by_a_serial_sighting_during_an_edl_request() {
        let mut state = bound_a_beside_b();
        assert_eq!(
            decide(&[adb("A", 1), adb("B", 2)], &mut state, Edl).unwrap(),
            None
        );
        assert_eq!(binding(&state), None);
    }

    #[test]
    fn reset_to_edl_bounce_keeps_the_binding() {
        let mut state = bound_a_beside_b();
        assert_eq!(decide(&[adb("B", 2)], &mut state, Edl).unwrap(), None);
        assert_eq!(binding(&state), Some(loc("1", &[1])));
        assert_eq!(
            decide(&[adb("B", 2), edl(1)], &mut state, Edl).unwrap(),
            Some(1)
        );
        assert_eq!(binding(&state), Some(loc("1", &[1])));
    }

    #[test]
    fn second_edl_device_elsewhere_is_ignored_while_bound() {
        let mut state = bound_a_beside_b();
        let census = [adb("B", 2), edl(3), edl(1)];
        assert_eq!(decide(&census, &mut state, Edl).unwrap(), Some(2));
        assert_eq!(binding(&state), Some(loc("1", &[1])));
    }

    #[test]
    fn two_edl_devices_at_the_bound_location_are_ambiguous() {
        let mut state = bound_a_beside_b();
        assert!(is_ambiguous(decide(
            &[adb("B", 2), edl(1), edl(1)],
            &mut state,
            Edl
        )));
        let mut state = by_location(1);
        assert!(is_ambiguous(decide(&[edl(1), edl(1)], &mut state, Edl)));
    }

    #[test]
    fn edl_only_target_resolves_by_location() {
        let mut state = by_location(1);
        assert_eq!(decide(&[edl(3), edl(1)], &mut state, Edl).unwrap(), Some(1));
        assert_eq!(decide(&[edl(3)], &mut state, Edl).unwrap(), None);
        assert_eq!(decide(&[edl(3), edl(1)], &mut state, Adb).unwrap(), None);
    }

    #[test]
    fn edl_only_target_never_adopts_a_serial() {
        let witnessed = || {
            let mut state = by_location(1);
            assert_eq!(
                decide(&[edl(1), adb("B", 2)], &mut state, Edl).unwrap(),
                Some(0)
            );
            state
        };
        // A phone plugged in while the target reboots out of EDL.
        let mut state = witnessed();
        let census = [adb("B", 2), adb("P", 3)];
        for mode in [Adb, Fastboot] {
            assert_eq!(decide(&census, &mut state, mode).unwrap(), None);
        }
        assert_eq!(identity(&state), by_location(1).unwrap().identity);
        assert_eq!(binding(&state), None);
        // Even the target's own new serial is not adopted.
        let census = [adb("B", 2), fastboot("A", 1)];
        for mode in [Adb, Fastboot] {
            assert_eq!(decide(&census, &mut state, mode).unwrap(), None);
        }
        assert_eq!(identity(&state), by_location(1).unwrap().identity);
        assert!(!visible(&census, &mut state).unwrap());
        // Back in EDL at its location it is found again.
        assert_eq!(
            decide(&[adb("B", 2), edl(1)], &mut state, Edl).unwrap(),
            Some(1)
        );
    }

    #[test]
    fn edl_only_target_without_a_location_is_refused() {
        let mut state = Some(TargetState::new(TargetIdentity::EdlOnly {
            location: loc("", &[]),
        }));
        for mode in [Adb, Fastboot, Edl] {
            assert!(is_unproven(decide(
                &[device(Edl, None, loc("", &[]))],
                &mut state,
                mode
            )));
        }
        assert!(matches!(
            visible(&[device(Edl, None, loc("", &[]))], &mut state),
            Err(UsbSelectionError::UnprovenSuccessor(_))
        ));
    }

    #[test]
    fn visible_finds_the_target_in_whichever_mode_it_is_in() {
        let mut state = None;
        assert!(!visible(&[adb("A", 1)], &mut state).unwrap());

        let mut state = witnessed_a_beside_b();
        assert!(visible(&[adb("B", 2), adb("A", 1)], &mut state).unwrap());
        assert!(visible(&[fastboot("A", 1), adb("B", 2)], &mut state).unwrap());
        assert!(!visible(&[adb("B", 2)], &mut state).unwrap());
        assert!(visible(&[adb("B", 2), edl(1)], &mut state).unwrap());

        let mut state = bound_a_beside_b();
        assert!(visible(&[adb("B", 2), edl(1)], &mut state).unwrap());
        assert_eq!(binding(&state), Some(loc("1", &[1])));

        let mut state = by_location(1);
        assert!(visible(&[edl(3), edl(1), adb("B", 2)], &mut state).unwrap());
        assert!(!visible(&[edl(3), adb("B", 2)], &mut state).unwrap());

        let mut state = witnessed_a_beside_b();
        assert!(matches!(
            visible(&[adb("A", 1), adb("A", 3)], &mut state),
            Err(UsbSelectionError::AmbiguousTarget)
        ));
    }

    #[test]
    fn visible_never_binds_or_rewrites_the_identity() {
        let mut state = witnessed_a_beside_b();
        let witness = state.as_ref().unwrap().witness.clone();
        // The proof would pass, so the target counts as visible, but nothing is bound.
        assert!(visible(&[adb("B", 2), edl(1)], &mut state).unwrap());
        assert_eq!(binding(&state), None);
        assert_eq!(state.as_ref().unwrap().witness, witness);
        assert_eq!(identity(&state), by_serial("A").unwrap().identity);
        // A refused proof is reported, and still binds nothing.
        assert!(matches!(
            visible(&[adb("B", 2), edl(1), edl(3)], &mut state),
            Err(UsbSelectionError::UnprovenSuccessor(_))
        ));
        assert_eq!(binding(&state), None);
        // The sole-device rule binds nothing from the poll either.
        let mut state = by_serial("A");
        decide(&[adb("A", 1)], &mut state, Adb).unwrap();
        assert!(visible(&[edl(4)], &mut state).unwrap());
        assert_eq!(binding(&state), None);
        // The operation's own resolve binds.
        assert_eq!(
            decide(&[adb("B", 2), edl(1)], &mut witnessed_a_beside_b(), Edl).unwrap(),
            Some(1)
        );
    }

    #[test]
    fn visible_refreshes_the_witness_on_a_serial_sighting() {
        let mut state = bound_a_beside_b();
        let census = [adb("A", 1), adb("B", 2), adb("C", 3)];
        assert!(visible(&census, &mut state).unwrap());
        assert_eq!(binding(&state), None);
        assert_eq!(state.as_ref().unwrap().witness, census.to_vec());
    }

    #[test]
    fn idle_poll_drops_the_binding_after_a_socket_swap() {
        let mut state = bound_a_beside_b();
        // A was unplugged from port 1 while idle.
        assert!(!visible(&[adb("B", 2)], &mut state).unwrap());
        assert_eq!(binding(&state), None);
        // Another EDL device in port 1 is no longer the target by location alone; with B gone
        // as well it fails the proof.
        assert!(matches!(
            visible(&[edl(1)], &mut state),
            Err(UsbSelectionError::UnprovenSuccessor(_))
        ));
        assert!(is_unproven(decide(&[edl(1)], &mut state, Edl)));
        assert_eq!(binding(&state), None);
        // A third device in EDL beside it needs a new proof, where the binding would have
        // answered port 1.
        assert!(is_unproven(decide(
            &[adb("B", 2), edl(1), edl(3)],
            &mut state,
            Edl
        )));
        assert_eq!(binding(&state), None);
    }

    #[test]
    fn idle_poll_keeps_the_binding_while_the_bound_device_is_there() {
        let mut state = bound_a_beside_b();
        assert!(visible(&[adb("B", 2), edl(3), edl(1)], &mut state).unwrap());
        assert_eq!(binding(&state), Some(loc("1", &[1])));
        assert!(matches!(
            visible(&[adb("B", 2), edl(1), edl(1)], &mut state),
            Err(UsbSelectionError::AmbiguousTarget)
        ));
        assert_eq!(binding(&state), Some(loc("1", &[1])));
    }

    #[test]
    fn set_target_replaces_the_whole_state() {
        let cell = Mutex::new(witnessed_a_beside_b());
        replace_state(
            &cell,
            Some(TargetState::new(TargetIdentity::Serial {
                serial: "B".to_owned(),
            })),
        );
        let guard = lock_state(&cell).unwrap();
        let state = guard.as_ref().unwrap();
        assert!(state.witness.is_empty());
        assert_eq!(state.edl_binding, None);
    }

    #[test]
    fn poisoned_state_is_an_error_not_an_empty_target() {
        let cell = Mutex::new(by_serial("A"));
        let _ = std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    let _guard = cell.lock().unwrap();
                    panic!("poison the target state");
                })
                .join()
        });
        assert!(matches!(
            lock_state(&cell),
            Err(UsbSelectionError::StatePoisoned)
        ));
        assert_eq!(
            peek_identity(&cell),
            Some(TargetIdentity::Serial {
                serial: "A".to_owned()
            })
        );
        replace_state(&cell, None);
        assert!(lock_state(&cell).unwrap().is_none());
    }
}
