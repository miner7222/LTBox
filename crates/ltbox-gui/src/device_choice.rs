//! Which USB devices are attached and whether LTBox can tell them apart.
//!
//! The poll worker lists candidates before opening any transport. It probes
//! only the single device or the chosen one; in every other state it opens
//! nothing, and the dashboard asks instead of guessing which device to read.

use std::collections::BTreeSet;

use crate::message::DeviceChoiceMsg;
use ltbox_device::selection::{DeviceMode, TargetIdentity, UsbCandidate, UsbLocation};

/// Stable name of one attached device: its serial, or its USB port when it has
/// none (EDL never reports one).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ChoiceId {
    Serial(String),
    Location(UsbLocation),
}

/// One attached device as the dashboard lists it. Built from enumeration alone,
/// so no model is known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeviceChoice {
    pub(crate) id: ChoiceId,
    pub(crate) mode: DeviceMode,
    pub(crate) serial: Option<String>,
    /// USB product string; empty when the device did not report one.
    pub(crate) product: String,
}

impl DeviceChoice {
    pub(crate) fn from_candidate(candidate: &UsbCandidate) -> Self {
        let id = match &candidate.serial {
            Some(serial) => ChoiceId::Serial(serial.clone()),
            None => ChoiceId::Location(candidate.location.clone()),
        };
        Self {
            id,
            mode: candidate.mode,
            serial: candidate.serial.clone(),
            product: candidate.product.clone().unwrap_or_default(),
        }
    }
}

pub(crate) fn inventory_from_candidates(candidates: &[UsbCandidate]) -> Vec<DeviceChoice> {
    candidates
        .iter()
        .map(DeviceChoice::from_candidate)
        .collect()
}

/// Whether production may leave the `Single`/`MultipleEdl`/`Unselectable`
/// classification. On: every transport honours the session target.
pub(crate) const SELECTION_ENABLED: bool = true;

/// What the attached devices allow the dashboard to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum MultiDeviceState {
    /// Zero or one device and nothing chosen: poll and show it as before.
    #[default]
    Single,
    /// Several selectable devices, or one left after a choice: ask which one.
    NeedsChoice,
    /// Two or more devices in EDL, which cannot be told apart.
    MultipleEdl,
    /// A device without a serial outside EDL, or two with the same serial.
    Unselectable,
    /// The user picked a device and the other devices have not changed since.
    Chosen,
}

impl MultiDeviceState {
    /// The classification used while selection is disabled.
    pub(crate) fn classify(inventory: &[DeviceChoice]) -> Self {
        if inventory.len() < 2 {
            return Self::Single;
        }
        let edl = inventory
            .iter()
            .filter(|device| device.mode == DeviceMode::Edl)
            .count();
        if edl >= 2 {
            Self::MultipleEdl
        } else {
            Self::Unselectable
        }
    }
}

/// What the device layer's session target must do. Only
/// `App::apply_target_action` carries it out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TargetAction {
    Keep,
    Clear,
    Set(TargetIdentity),
}

/// The last polled inventory, the state derived from it and the user's choice.
#[derive(Debug, Clone, Default)]
pub(crate) struct DeviceChoiceState {
    pub(crate) inventory: Vec<DeviceChoice>,
    pub(crate) state: MultiDeviceState,
    /// The device the user picked; kept while the other devices are unchanged.
    pub(crate) chosen: Option<ChoiceId>,
    /// Ids of every non-chosen device at the moment of choice.
    pub(crate) others_at_choice: BTreeSet<ChoiceId>,
    /// A choice was made since the last poll that saw no device.
    pub(crate) episode_had_choice: bool,
    /// The choice was dropped because the other devices changed.
    pub(crate) rechoose: bool,
    /// `Some` while the choice dialog is open; the inner value is the staged row.
    pub(crate) dialog: Option<Option<ChoiceId>>,
    /// The chosen device (or its EDL successor) is in the inventory.
    pub(crate) target_present: bool,
}

/// Whether the chosen device is still the one the user picked. `Some(present)`
/// when the other devices are exactly those at the moment of choice; `None`
/// when there is no choice or the other devices changed.
fn held_choice(prev: &DeviceChoiceState, inventory: &[DeviceChoice]) -> Option<bool> {
    let chosen = prev.chosen.as_ref()?;
    let mut others: Vec<&ChoiceId> = inventory.iter().map(|device| &device.id).collect();
    let present = if let Some(position) = others.iter().position(|id| *id == chosen) {
        others.remove(position);
        true
    } else if matches!(chosen, ChoiceId::Serial(_)) {
        // The chosen serial is gone: it may have come back as the one new EDL
        // device. Zero or two new EDL devices prove nothing.
        let mut fresh = inventory.iter().filter(|device| {
            device.mode == DeviceMode::Edl && !prev.others_at_choice.contains(&device.id)
        });
        match (fresh.next(), fresh.next()) {
            (Some(successor), None) => {
                if let Some(position) = others.iter().position(|id| **id == successor.id) {
                    others.remove(position);
                }
                true
            }
            _ => false,
        }
    } else {
        // An EDL device chosen by port has no serial to come back under.
        return None;
    };
    let current: BTreeSet<ChoiceId> = others.iter().map(|id| (*id).clone()).collect();
    // A duplicate id shrinks the set, so it never equals the set at choice.
    (current.len() == others.len() && current == prev.others_at_choice).then_some(present)
}

/// The state with nothing chosen.
fn unchosen_state(inventory: &[DeviceChoice], episode_had_choice: bool) -> MultiDeviceState {
    if inventory.len() < 2 && !episode_had_choice {
        return MultiDeviceState::Single;
    }
    let edl = inventory
        .iter()
        .filter(|device| device.mode == DeviceMode::Edl)
        .count();
    if edl >= 2 {
        return MultiDeviceState::MultipleEdl;
    }
    let mut serials = BTreeSet::new();
    let unselectable = inventory.iter().any(|device| match &device.serial {
        Some(serial) => !serials.insert(serial.as_str()),
        None => device.mode != DeviceMode::Edl,
    });
    if unselectable {
        MultiDeviceState::Unselectable
    } else {
        MultiDeviceState::NeedsChoice
    }
}

/// The next choice state for a fresh inventory. Pure.
///
/// `target_held` means the device layer holds a session target. A target that
/// no choice backs is cleared.
pub(crate) fn next_choice_state(
    prev: &DeviceChoiceState,
    inventory: Vec<DeviceChoice>,
    target_held: bool,
    selection_enabled: bool,
) -> (DeviceChoiceState, TargetAction) {
    let stale_target = if target_held {
        TargetAction::Clear
    } else {
        TargetAction::Keep
    };
    if !selection_enabled {
        let state = MultiDeviceState::classify(&inventory);
        let next = DeviceChoiceState {
            inventory,
            state,
            ..DeviceChoiceState::default()
        };
        return (next, stale_target);
    }
    if inventory.is_empty() {
        return (DeviceChoiceState::default(), TargetAction::Clear);
    }
    let mut next = DeviceChoiceState {
        episode_had_choice: prev.episode_had_choice,
        rechoose: prev.rechoose,
        ..DeviceChoiceState::default()
    };
    let action = if let Some(present) = held_choice(prev, &inventory) {
        next.state = MultiDeviceState::Chosen;
        next.chosen = prev.chosen.clone();
        next.others_at_choice = prev.others_at_choice.clone();
        next.target_present = present;
        TargetAction::Keep
    } else {
        next.state = unchosen_state(&inventory, next.episode_had_choice);
        if prev.chosen.is_some() {
            next.rechoose = true;
            TargetAction::Clear
        } else {
            stale_target
        }
    };
    next.dialog = match (&prev.dialog, next.state) {
        (_, state)
            if !matches!(
                state,
                MultiDeviceState::NeedsChoice | MultiDeviceState::Chosen
            ) =>
        {
            None
        }
        (Some(Some(id)), _) if !inventory.iter().any(|device| device.id == *id) => Some(None),
        (dialog, _) => dialog.clone(),
    };
    next.inventory = inventory;
    (next, action)
}

/// What the poll worker may open for an inventory, decided before any transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PollProbe {
    /// Zero or one device and no target: probe as before selection existed.
    Untargeted,
    /// The chosen device: only the session target, through the target-aware
    /// transports.
    Target,
    /// Several devices, a pending re-prompt, or a target that no choice backs
    /// (or a choice without a target): open nothing.
    Nothing,
}

/// The probe for the state the update loop will compute from the same inputs.
/// Pure. `prev` cannot change while the poll runs: choice input waits for the
/// poll to finish, and the target changes only on the update loop.
pub(crate) fn poll_probe(
    prev: &DeviceChoiceState,
    inventory: &[DeviceChoice],
    target_held: bool,
) -> PollProbe {
    let (next, _) = next_choice_state(prev, inventory.to_vec(), target_held, SELECTION_ENABLED);
    match next.state {
        MultiDeviceState::Single if !target_held => PollProbe::Untargeted,
        MultiDeviceState::Chosen if target_held => PollProbe::Target,
        _ => PollProbe::Nothing,
    }
}

/// Whether a visible target sits in EDL: chosen there by port, or its serial is
/// gone from the inventory while an EDL device is present. Read from the
/// inventory so the poll never calls `edl::check_device`, which could bind.
pub(crate) fn target_in_edl(target: &TargetIdentity, inventory: &[DeviceChoice]) -> bool {
    match target {
        TargetIdentity::EdlOnly { .. } => true,
        TargetIdentity::Serial { serial } => {
            let serial_listed = inventory.iter().any(|device| {
                device.mode != DeviceMode::Edl && device.serial.as_deref() == Some(serial.as_str())
            });
            !serial_listed
                && inventory
                    .iter()
                    .any(|device| device.mode == DeviceMode::Edl)
        }
    }
}

impl DeviceChoiceState {
    /// Fold a poll's inventory in with the production settings.
    pub(crate) fn apply(
        &mut self,
        inventory: Vec<DeviceChoice>,
        target_held: bool,
    ) -> TargetAction {
        self.apply_with(inventory, target_held, SELECTION_ENABLED)
    }

    pub(crate) fn apply_with(
        &mut self,
        inventory: Vec<DeviceChoice>,
        target_held: bool,
        selection_enabled: bool,
    ) -> TargetAction {
        let (next, action) = next_choice_state(self, inventory, target_held, selection_enabled);
        *self = next;
        action
    }

    /// True while the user must act before any device is read, and every Start
    /// is blocked.
    pub(crate) fn is_multiple(&self) -> bool {
        matches!(
            self.state,
            MultiDeviceState::NeedsChoice
                | MultiDeviceState::MultipleEdl
                | MultiDeviceState::Unselectable
        )
    }

    /// The chosen device is not attached, so it shows as disconnected.
    pub(crate) fn chosen_absent(&self) -> bool {
        self.state == MultiDeviceState::Chosen && !self.target_present
    }

    fn lists_once(&self, id: &ChoiceId) -> bool {
        self.inventory
            .iter()
            .filter(|device| device.id == *id)
            .count()
            == 1
    }

    /// Record `id` as the choice and return the target it names: a serial
    /// outside EDL, or the port of an EDL device. Anything else is refused.
    fn choose(&mut self, id: ChoiceId) -> TargetAction {
        if !matches!(
            self.state,
            MultiDeviceState::NeedsChoice | MultiDeviceState::Chosen
        ) || !self.lists_once(&id)
        {
            return TargetAction::Keep;
        }
        let Some(mode) = self
            .inventory
            .iter()
            .find(|device| device.id == id)
            .map(|device| device.mode)
        else {
            return TargetAction::Keep;
        };
        let identity = match (&id, mode) {
            (ChoiceId::Serial(serial), mode) if mode != DeviceMode::Edl => TargetIdentity::Serial {
                serial: serial.clone(),
            },
            (ChoiceId::Location(location), DeviceMode::Edl) => TargetIdentity::EdlOnly {
                location: location.clone(),
            },
            _ => return TargetAction::Keep,
        };
        let others: Vec<ChoiceId> = self
            .inventory
            .iter()
            .map(|device| device.id.clone())
            .filter(|other| *other != id)
            .collect();
        let others_at_choice: BTreeSet<ChoiceId> = others.iter().cloned().collect();
        if others_at_choice.len() != others.len() {
            return TargetAction::Keep;
        }
        self.chosen = Some(id);
        self.others_at_choice = others_at_choice;
        self.episode_had_choice = true;
        self.rechoose = false;
        self.state = MultiDeviceState::Chosen;
        self.target_present = true;
        self.dialog = None;
        TargetAction::Set(identity)
    }

    /// Apply one dialog or list message; `Set` when it made a choice.
    pub(crate) fn handle(&mut self, message: DeviceChoiceMsg) -> TargetAction {
        match message {
            DeviceChoiceMsg::Open => {
                if matches!(
                    self.state,
                    MultiDeviceState::NeedsChoice | MultiDeviceState::Chosen
                ) {
                    self.dialog = Some(None);
                }
            }
            DeviceChoiceMsg::Pick(id) => {
                if self.dialog.is_some() && self.lists_once(&id) {
                    self.dialog = Some(Some(id));
                }
            }
            DeviceChoiceMsg::Confirm => {
                if let Some(Some(id)) = self.dialog.clone() {
                    return self.choose(id);
                }
            }
            DeviceChoiceMsg::Cancel => self.dialog = None,
            DeviceChoiceMsg::PickNow(id) => return self.choose(id),
        }
        TargetAction::Keep
    }
}

/// Test stand-in for the device layer's process-wide session target. Tests run
/// on parallel threads, so each thread records its own calls.
#[cfg(test)]
pub(crate) mod test_target {
    use std::cell::RefCell;

    use ltbox_device::selection::TargetIdentity;

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(crate) enum Call {
        Set(TargetIdentity),
        Clear,
    }

    thread_local! {
        static TARGET: RefCell<Option<TargetIdentity>> = const { RefCell::new(None) };
        static CALLS: RefCell<Vec<Call>> = const { RefCell::new(Vec::new()) };
    }

    pub(crate) fn current() -> Option<TargetIdentity> {
        TARGET.with(|target| target.borrow().clone())
    }

    pub(crate) fn set(identity: TargetIdentity) {
        CALLS.with(|calls| calls.borrow_mut().push(Call::Set(identity.clone())));
        TARGET.with(|target| *target.borrow_mut() = Some(identity));
    }

    pub(crate) fn clear() {
        CALLS.with(|calls| calls.borrow_mut().push(Call::Clear));
        TARGET.with(|target| *target.borrow_mut() = None);
    }

    /// Forget this thread's target and recorded calls.
    pub(crate) fn reset() {
        CALLS.with(|calls| calls.borrow_mut().clear());
        TARGET.with(|target| *target.borrow_mut() = None);
    }

    pub(crate) fn calls() -> Vec<Call> {
        CALLS.with(|calls| calls.borrow().clone())
    }

    pub(crate) fn set_calls() -> usize {
        calls()
            .iter()
            .filter(|call| matches!(call, Call::Set(_)))
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choice(mode: DeviceMode, serial: Option<&str>, port: u8) -> DeviceChoice {
        DeviceChoice::from_candidate(&UsbCandidate {
            mode,
            serial: serial.map(str::to_owned),
            location: UsbLocation {
                bus_id: "1".to_string(),
                port_chain: vec![port],
            },
            vendor_id: 0x18d1,
            product_id: 0x4ee0,
            product: Some("Tablet".to_string()),
            instance_id: None,
        })
    }

    fn adb(serial: &str, port: u8) -> DeviceChoice {
        choice(DeviceMode::Adb, Some(serial), port)
    }

    fn edl(port: u8) -> DeviceChoice {
        choice(DeviceMode::Edl, None, port)
    }

    fn serial(name: &str) -> ChoiceId {
        ChoiceId::Serial(name.to_string())
    }

    /// A state in which `picked` was chosen out of `inventory`.
    fn chosen(inventory: Vec<DeviceChoice>, picked: &str) -> DeviceChoiceState {
        let mut state = DeviceChoiceState::default();
        state.apply_with(inventory, false, true);
        assert_eq!(state.state, MultiDeviceState::NeedsChoice);
        state.handle(DeviceChoiceMsg::PickNow(serial(picked)));
        assert_eq!(state.state, MultiDeviceState::Chosen);
        state
    }

    #[test]
    fn zero_or_one_device_is_single() {
        assert_eq!(MultiDeviceState::classify(&[]), MultiDeviceState::Single);
        let one = [edl(1)];
        assert_eq!(MultiDeviceState::classify(&one), MultiDeviceState::Single);
    }

    #[test]
    fn two_edl_devices_are_multiple_edl() {
        let two = [edl(1), edl(2)];
        assert_eq!(
            MultiDeviceState::classify(&two),
            MultiDeviceState::MultipleEdl
        );
    }

    #[test]
    fn any_other_set_of_two_is_unselectable() {
        let sets = [
            [adb("A", 1), adb("B", 2)],
            [adb("A", 1), edl(2)],
            [
                choice(DeviceMode::Fastboot, Some("A"), 1),
                choice(DeviceMode::Other, None, 2),
            ],
        ];
        for set in sets {
            assert_eq!(
                MultiDeviceState::classify(&set),
                MultiDeviceState::Unselectable
            );
        }
    }

    #[test]
    fn id_is_the_serial_or_else_the_port() {
        let with_serial = choice(DeviceMode::Adb, Some("A1"), 3);
        assert_eq!(with_serial.id, ChoiceId::Serial("A1".to_string()));
        let without = edl(3);
        assert_eq!(
            without.id,
            ChoiceId::Location(UsbLocation {
                bus_id: "1".to_string(),
                port_chain: vec![3],
            })
        );
        assert_eq!(without.product, "Tablet");
    }

    #[test]
    fn apply_tracks_the_latest_inventory() {
        let mut state = DeviceChoiceState::default();
        assert!(!state.is_multiple());
        state.apply(
            vec![adb("A", 1), choice(DeviceMode::Fastboot, Some("B"), 2)],
            false,
        );
        assert!(state.is_multiple());
        state.apply(Vec::new(), false);
        assert!(!state.is_multiple());
        assert!(state.inventory.is_empty());
    }

    #[test]
    fn disabled_selection_classifies_like_before() {
        let mut state = DeviceChoiceState::default();
        let action = state.apply_with(vec![edl(1), edl(2)], false, false);
        assert_eq!(state.state, MultiDeviceState::MultipleEdl);
        assert_eq!(action, TargetAction::Keep);
        state.apply_with(vec![adb("A", 1), adb("B", 2)], false, false);
        assert_eq!(state.state, MultiDeviceState::Unselectable);
        assert!(state.chosen.is_none());
        state.apply_with(vec![adb("A", 1)], false, false);
        assert_eq!(state.state, MultiDeviceState::Single);
        // Production runs with selection on.
        state.apply(vec![adb("A", 1), adb("B", 2)], false);
        assert_eq!(state.state, MultiDeviceState::NeedsChoice);
    }

    #[test]
    fn no_device_ends_the_episode() {
        let mut state = chosen(vec![adb("A", 1), adb("B", 2)], "A");
        let action = state.apply_with(Vec::new(), false, true);
        assert_eq!(action, TargetAction::Clear);
        assert_eq!(state.state, MultiDeviceState::Single);
        assert!(state.chosen.is_none());
        assert!(!state.episode_had_choice);
        // A lone device is now just the single device again.
        state.apply_with(vec![adb("A", 1)], false, true);
        assert_eq!(state.state, MultiDeviceState::Single);
    }

    #[test]
    fn selectable_pair_needs_a_choice_and_blocks() {
        let mut state = DeviceChoiceState::default();
        state.apply_with(vec![adb("A", 1), edl(2)], false, true);
        assert_eq!(state.state, MultiDeviceState::NeedsChoice);
        assert!(state.is_multiple());
        state.apply_with(vec![edl(1), edl(2)], false, true);
        assert_eq!(state.state, MultiDeviceState::MultipleEdl);
        state.apply_with(
            vec![adb("A", 1), choice(DeviceMode::Other, None, 2)],
            false,
            true,
        );
        assert_eq!(state.state, MultiDeviceState::Unselectable);
        state.apply_with(vec![adb("A", 1), adb("A", 2)], false, true);
        assert_eq!(state.state, MultiDeviceState::Unselectable);
    }

    #[test]
    fn chosen_device_rebooting_keeps_the_choice() {
        let mut state = chosen(vec![adb("A", 1), adb("B", 2)], "A");
        assert!(!state.is_multiple());
        // Absent while the others are unchanged.
        let action = state.apply_with(vec![adb("B", 2)], false, true);
        assert_eq!(action, TargetAction::Keep);
        assert_eq!(state.state, MultiDeviceState::Chosen);
        assert_eq!(state.chosen, Some(serial("A")));
        assert!(state.chosen_absent());
        // It returns under the same serial.
        state.apply_with(vec![adb("A", 3), adb("B", 2)], false, true);
        assert_eq!(state.state, MultiDeviceState::Chosen);
        assert!(!state.chosen_absent());
    }

    #[test]
    fn chosen_device_returning_as_the_new_edl_device_keeps_the_choice() {
        let mut state = chosen(vec![adb("A", 1), adb("B", 2)], "A");
        let action = state.apply_with(vec![adb("B", 2), edl(5)], false, true);
        assert_eq!(action, TargetAction::Keep);
        assert_eq!(state.state, MultiDeviceState::Chosen);
        assert!(!state.chosen_absent());
        // Back to ADB: the successor leaves and the serial returns.
        state.apply_with(vec![adb("B", 2), adb("A", 1)], false, true);
        assert_eq!(state.state, MultiDeviceState::Chosen);
        assert!(!state.rechoose);
    }

    #[test]
    fn another_device_added_or_removed_asks_again() {
        let base = chosen(vec![adb("A", 1), adb("B", 2)], "A");

        let mut added = base.clone();
        let action = added.apply_with(vec![adb("A", 1), adb("B", 2), adb("C", 3)], false, true);
        assert_eq!(action, TargetAction::Clear);
        assert_eq!(added.state, MultiDeviceState::NeedsChoice);
        assert!(added.chosen.is_none());
        assert!(added.rechoose);

        // One device left after a choice is still not adopted silently.
        let mut removed = base.clone();
        let action = removed.apply_with(vec![adb("A", 1)], false, true);
        assert_eq!(action, TargetAction::Clear);
        assert_eq!(removed.state, MultiDeviceState::NeedsChoice);
        assert!(removed.rechoose);

        // A new device while the chosen one is absent is also a change.
        let mut absent_added = base;
        absent_added.apply_with(vec![adb("B", 2), adb("C", 3)], false, true);
        assert_eq!(absent_added.state, MultiDeviceState::NeedsChoice);
        assert!(absent_added.rechoose);
    }

    #[test]
    fn two_new_edl_devices_while_chosen_is_absent_ask_again() {
        let mut state = chosen(vec![adb("A", 1), adb("B", 2)], "A");
        let action = state.apply_with(vec![adb("B", 2), edl(5), edl(6)], false, true);
        assert_eq!(action, TargetAction::Clear);
        assert_eq!(state.state, MultiDeviceState::MultipleEdl);
        assert!(state.chosen.is_none());
        assert!(state.rechoose);
    }

    #[test]
    fn an_edl_device_already_present_is_not_a_successor() {
        let mut state = chosen(vec![adb("A", 1), edl(5)], "A");
        state.apply_with(vec![edl(5)], false, true);
        assert_eq!(state.state, MultiDeviceState::Chosen);
        assert!(state.chosen_absent());
    }

    #[test]
    fn chosen_edl_device_that_disappears_asks_again() {
        let mut state = DeviceChoiceState::default();
        state.apply_with(vec![edl(1), adb("B", 2)], false, true);
        let port = edl(1).id;
        state.handle(DeviceChoiceMsg::PickNow(port));
        assert_eq!(state.state, MultiDeviceState::Chosen);
        state.apply_with(vec![adb("B", 2)], false, true);
        assert_eq!(state.state, MultiDeviceState::NeedsChoice);
        assert!(state.rechoose);
    }

    #[test]
    fn a_target_no_choice_backs_is_cleared() {
        let mut state = DeviceChoiceState::default();
        let action = state.apply_with(vec![adb("A", 1), adb("B", 2)], true, true);
        assert_eq!(action, TargetAction::Clear);
        let action = state.apply_with(vec![adb("A", 1), adb("B", 2)], false, true);
        assert_eq!(action, TargetAction::Keep);
    }

    #[test]
    fn dialog_picks_confirm_and_close() {
        let mut state = DeviceChoiceState::default();
        state.apply_with(vec![adb("A", 1), adb("B", 2)], false, true);
        // Nothing opens, picks or confirms without the dialog.
        state.handle(DeviceChoiceMsg::Pick(serial("A")));
        state.handle(DeviceChoiceMsg::Confirm);
        assert_eq!(state.state, MultiDeviceState::NeedsChoice);

        state.handle(DeviceChoiceMsg::Open);
        assert_eq!(state.dialog, Some(None));
        state.handle(DeviceChoiceMsg::Confirm);
        assert_eq!(state.state, MultiDeviceState::NeedsChoice);
        state.handle(DeviceChoiceMsg::Pick(serial("Z")));
        assert_eq!(state.dialog, Some(None));
        state.handle(DeviceChoiceMsg::Pick(serial("B")));
        assert_eq!(state.dialog, Some(Some(serial("B"))));
        state.handle(DeviceChoiceMsg::Cancel);
        assert_eq!(state.dialog, None);

        state.handle(DeviceChoiceMsg::Open);
        state.handle(DeviceChoiceMsg::Pick(serial("B")));
        state.handle(DeviceChoiceMsg::Confirm);
        assert_eq!(state.state, MultiDeviceState::Chosen);
        assert_eq!(state.chosen, Some(serial("B")));
        assert_eq!(
            state.others_at_choice,
            BTreeSet::from([serial("A")]),
            "the others at the moment of choice"
        );
        assert_eq!(state.dialog, None);
        assert!(state.episode_had_choice);
    }

    #[test]
    fn a_dialog_pick_whose_device_left_is_dropped_and_the_dialog_closes() {
        let mut state = DeviceChoiceState::default();
        state.apply_with(vec![adb("A", 1), adb("B", 2), adb("C", 3)], false, true);
        state.handle(DeviceChoiceMsg::Open);
        state.handle(DeviceChoiceMsg::Pick(serial("C")));
        state.apply_with(vec![adb("A", 1), adb("B", 2)], false, true);
        assert_eq!(state.state, MultiDeviceState::NeedsChoice);
        assert_eq!(state.dialog, Some(None));
        state.handle(DeviceChoiceMsg::Confirm);
        assert_eq!(state.state, MultiDeviceState::NeedsChoice);

        state.apply_with(vec![edl(1), edl(2)], false, true);
        assert_eq!(state.state, MultiDeviceState::MultipleEdl);
        assert_eq!(state.dialog, None);
        state.handle(DeviceChoiceMsg::Open);
        assert_eq!(state.dialog, None);
    }

    #[test]
    fn picking_a_duplicate_serial_is_refused() {
        let mut state = DeviceChoiceState::default();
        state.apply_with(vec![adb("A", 1), adb("A", 2)], false, true);
        let action = state.handle(DeviceChoiceMsg::PickNow(serial("A")));
        assert_eq!(action, TargetAction::Keep);
        assert!(state.chosen.is_none());
    }

    #[test]
    fn device_choice_names_a_serial_target_or_an_edl_port() {
        let mut state = DeviceChoiceState::default();
        state.apply(vec![adb("A", 1), edl(2)], false);
        let action = state.handle(DeviceChoiceMsg::PickNow(serial("A")));
        assert_eq!(
            action,
            TargetAction::Set(TargetIdentity::Serial {
                serial: "A".to_string()
            })
        );

        let mut state = DeviceChoiceState::default();
        state.apply(vec![adb("A", 1), edl(2)], false);
        let port = edl(2).id;
        let ChoiceId::Location(location) = port.clone() else {
            panic!("an EDL row is named by its port");
        };
        state.handle(DeviceChoiceMsg::Open);
        assert_eq!(
            state.handle(DeviceChoiceMsg::Pick(port)),
            TargetAction::Keep
        );
        let action = state.handle(DeviceChoiceMsg::Confirm);
        assert_eq!(
            action,
            TargetAction::Set(TargetIdentity::EdlOnly { location })
        );
    }

    #[test]
    fn device_choice_refused_or_ui_only_messages_name_no_target() {
        let mut state = DeviceChoiceState::default();
        state.apply(vec![adb("A", 1)], false);
        // A single device is never chosen.
        for message in [
            DeviceChoiceMsg::Open,
            DeviceChoiceMsg::Pick(serial("A")),
            DeviceChoiceMsg::Confirm,
            DeviceChoiceMsg::PickNow(serial("A")),
            DeviceChoiceMsg::Cancel,
        ] {
            assert_eq!(state.handle(message), TargetAction::Keep);
        }
        assert_eq!(state.state, MultiDeviceState::Single);
        // A port-named device outside EDL is never a target.
        let mut state = DeviceChoiceState {
            inventory: vec![choice(DeviceMode::Other, None, 1), adb("B", 2)],
            state: MultiDeviceState::NeedsChoice,
            ..DeviceChoiceState::default()
        };
        let port = choice(DeviceMode::Other, None, 1).id;
        assert_eq!(
            state.handle(DeviceChoiceMsg::PickNow(port)),
            TargetAction::Keep
        );
        assert!(state.chosen.is_none());
    }

    #[test]
    fn device_choice_poll_probes_only_the_single_or_chosen_device() {
        let empty = DeviceChoiceState::default();
        assert_eq!(poll_probe(&empty, &[], false), PollProbe::Untargeted);
        assert_eq!(
            poll_probe(&empty, &[adb("A", 1)], false),
            PollProbe::Untargeted
        );
        // A target that no choice backs: nothing opens until it is cleared.
        assert_eq!(poll_probe(&empty, &[adb("A", 1)], true), PollProbe::Nothing);
        assert_eq!(poll_probe(&empty, &[], true), PollProbe::Nothing);
        for inventory in [
            vec![adb("A", 1), adb("B", 2)],
            vec![edl(1), edl(2)],
            vec![adb("A", 1), adb("A", 2)],
            vec![adb("A", 1), choice(DeviceMode::Other, None, 2)],
        ] {
            assert_eq!(poll_probe(&empty, &inventory, false), PollProbe::Nothing);
        }

        let chosen = chosen(vec![adb("A", 1), adb("B", 2)], "A");
        assert_eq!(
            poll_probe(&chosen, &[adb("A", 1), adb("B", 2)], true),
            PollProbe::Target
        );
        // Absent while the others are unchanged: still only the target.
        assert_eq!(poll_probe(&chosen, &[adb("B", 2)], true), PollProbe::Target);
        // A choice the device layer does not hold opens nothing.
        assert_eq!(
            poll_probe(&chosen, &[adb("A", 1), adb("B", 2)], false),
            PollProbe::Nothing
        );
        // The others changed: re-prompt, open nothing.
        assert_eq!(
            poll_probe(&chosen, &[adb("A", 1), adb("B", 2), adb("C", 3)], true),
            PollProbe::Nothing
        );
        // One device left after a choice is a re-prompt, not the single device.
        assert_eq!(
            poll_probe(&chosen, &[adb("A", 1)], true),
            PollProbe::Nothing
        );
        assert_eq!(
            poll_probe(&chosen, &[adb("A", 1)], false),
            PollProbe::Nothing
        );
    }

    #[test]
    fn device_choice_target_in_edl_reads_the_inventory() {
        let target = TargetIdentity::Serial {
            serial: "A".to_string(),
        };
        assert!(!target_in_edl(&target, &[adb("A", 1), edl(2)]));
        assert!(target_in_edl(&target, &[adb("B", 1), edl(2)]));
        assert!(!target_in_edl(&target, &[adb("B", 1)]));
        let ChoiceId::Location(location) = edl(2).id else {
            unreachable!()
        };
        assert!(target_in_edl(
            &TargetIdentity::EdlOnly { location },
            &[edl(2)]
        ));
    }
}
