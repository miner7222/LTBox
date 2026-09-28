//! Debloat wizard state.

use super::*;
use crate::debloat::{DebloatList, DebloatMethod, DebloatPackage, PackageState};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DebloatAction {
    Remove,
    Restore,
}
impl DebloatAction {
    pub(crate) fn label_key(&self) -> &'static str {
        match self {
            Self::Remove => "debloat_remove",
            Self::Restore => "debloat_restore",
        }
    }
    pub(crate) fn desc_key(&self) -> &'static str {
        match self {
            Self::Remove => "debloat_remove_desc",
            Self::Restore => "debloat_restore_desc",
        }
    }
}

/// Which preset a selection shortcut applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DebloatPreset {
    Recommended,
    All,
    None,
}

/// One selected app and the state it was read in, which decides the
/// restore command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DebloatTarget {
    pub(crate) package: DebloatPackage,
    /// `None` when the device could not be read.
    pub(crate) state: Option<PackageState>,
}

#[derive(Default)]
pub(crate) struct DebloatWizard {
    pub(crate) step: usize,
    pub(crate) action: Option<DebloatAction>,
    /// Package ids picked on the Apps step.
    pub(crate) selected: BTreeSet<String>,
    /// Set once the Apps step has been seeded, so going back and forth
    /// keeps the user's own picks.
    pub(crate) seeded: bool,
    /// Per-package state read from the device on the way into the Apps
    /// step. `None` until read, or when the read failed.
    pub(crate) states: Option<BTreeMap<String, PackageState>>,
    /// Why the state read failed; every app then stays selectable.
    pub(crate) scan_error: Option<String>,
}

pub(crate) const DEBLOAT_STEPS: &[&str] = &[
    "sysupdate_step_action",
    "debloat_step_apps",
    "sysupdate_step_confirm",
    "sysupdate_step_execute",
];

impl DebloatWizard {
    pub(crate) fn state_of(&self, package: &DebloatPackage) -> Option<PackageState> {
        self.states.as_ref()?.get(&package.id).copied()
    }

    /// Whether the chosen action would change `package`. An app already
    /// removed is not offered for removal again, nor an installed one for
    /// restoring. Unknown state keeps the app selectable.
    pub(crate) fn is_actionable(&self, package: &DebloatPackage) -> bool {
        let Some(action) = self.action else {
            return false;
        };
        match (action, self.state_of(package)) {
            (_, None) => true,
            (DebloatAction::Remove, Some(PackageState::Installed)) => true,
            (DebloatAction::Remove, Some(PackageState::Disabled)) => {
                package.method == DebloatMethod::Uninstall
            }
            (DebloatAction::Remove, Some(PackageState::Removed | PackageState::Absent)) => false,
            (DebloatAction::Restore, Some(PackageState::Removed | PackageState::Disabled)) => true,
            (DebloatAction::Restore, Some(PackageState::Installed | PackageState::Absent)) => false,
        }
    }

    pub(crate) fn actionable_count(&self, list: &DebloatList) -> usize {
        list.packages
            .iter()
            .filter(|package| self.is_actionable(package))
            .count()
    }

    /// Seed the Apps step on first entry: the recommended preset for
    /// removal, nothing for restoring.
    pub(crate) fn seed(&mut self, list: &DebloatList) {
        if !self.seeded {
            let preset = match self.action {
                Some(DebloatAction::Remove) => DebloatPreset::Recommended,
                _ => DebloatPreset::None,
            };
            self.apply_preset(list, preset);
            self.seeded = true;
        }
    }

    pub(crate) fn apply_preset(&mut self, list: &DebloatList, preset: DebloatPreset) {
        self.selected = list
            .packages
            .iter()
            .filter(|package| self.is_actionable(package))
            .filter(|package| match preset {
                DebloatPreset::Recommended => package.recommended,
                DebloatPreset::All => true,
                DebloatPreset::None => false,
            })
            .map(|package| package.id.clone())
            .collect();
    }

    pub(crate) fn toggle(&mut self, list: &DebloatList, id: &str) {
        if self.selected.remove(id) {
            return;
        }
        if list
            .packages
            .iter()
            .any(|package| package.id == id && self.is_actionable(package))
        {
            self.selected.insert(id.to_string());
        }
    }

    /// Selected, actionable packages in list order. Ids outside `list` never
    /// reach the worker, whatever the selection holds.
    pub(crate) fn selected_targets(&self, list: &DebloatList) -> Vec<DebloatTarget> {
        list.packages
            .iter()
            .filter(|package| self.selected.contains(&package.id) && self.is_actionable(package))
            .map(|package| DebloatTarget {
                package: package.clone(),
                state: self.state_of(package),
            })
            .collect()
    }
}

impl Wizard for DebloatWizard {
    fn step(&self) -> usize {
        self.step
    }
    fn step_mut(&mut self) -> &mut usize {
        &mut self.step
    }
    fn step_count(&self) -> usize {
        DEBLOAT_STEPS.len()
    }
    fn can_next(&self) -> bool {
        match self.step {
            0 => self.action.is_some(),
            1 => !self.selected.is_empty(),
            2 => true,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list() -> &'static DebloatList {
        crate::debloat::list_for_model("TB321FU").expect("TB321FU ships a list")
    }

    fn remover() -> DebloatWizard {
        DebloatWizard {
            action: Some(DebloatAction::Remove),
            ..DebloatWizard::default()
        }
    }

    fn states(state: impl Fn(&DebloatPackage) -> PackageState) -> BTreeMap<String, PackageState> {
        list()
            .packages
            .iter()
            .map(|package| (package.id.clone(), state(package)))
            .collect()
    }

    #[test]
    fn apps_step_starts_on_the_recommended_preset_and_keeps_user_picks() {
        let mut wizard = remover();
        wizard.seed(list());
        let recommended = list().packages.iter().filter(|p| p.recommended).count();
        assert_eq!(wizard.selected.len(), recommended);

        let optional = list()
            .packages
            .iter()
            .find(|p| !p.recommended)
            .expect("the list keeps at least one opt-in app");
        wizard.toggle(list(), &optional.id);
        wizard.seed(list());
        assert!(wizard.selected.contains(&optional.id));
    }

    #[test]
    fn presets_select_everything_or_nothing() {
        let mut wizard = remover();
        wizard.apply_preset(list(), DebloatPreset::All);
        assert_eq!(wizard.selected.len(), list().packages.len());
        wizard.apply_preset(list(), DebloatPreset::None);
        assert!(wizard.selected.is_empty());
        wizard.step = 1;
        assert!(!wizard.can_next());
    }

    #[test]
    fn apps_already_gone_are_not_offered_for_removal() {
        let mut wizard = remover();
        wizard.states = Some(states(|_| PackageState::Removed));
        wizard.seed(list());
        assert!(wizard.selected.is_empty());
        assert_eq!(wizard.actionable_count(list()), 0);
        wizard.apply_preset(list(), DebloatPreset::All);
        assert!(wizard.selected.is_empty());
        wizard.toggle(list(), &list().packages[0].id);
        assert!(wizard.selected.is_empty());
    }

    #[test]
    fn restore_offers_only_removed_or_disabled_apps() {
        let mut wizard = DebloatWizard {
            action: Some(DebloatAction::Restore),
            ..DebloatWizard::default()
        };
        let removed = &list().packages[0];
        let disabled = &list().packages[1];
        wizard.states = Some(states(|package| {
            if package.id == removed.id {
                PackageState::Removed
            } else if package.id == disabled.id {
                PackageState::Disabled
            } else {
                PackageState::Installed
            }
        }));
        wizard.seed(list());
        assert!(
            wizard.selected.is_empty(),
            "restore starts with nothing picked"
        );
        wizard.apply_preset(list(), DebloatPreset::All);
        let targets = wizard.selected_targets(list());
        assert_eq!(
            targets
                .iter()
                .map(|target| (target.package.id.as_str(), target.state))
                .collect::<Vec<_>>(),
            vec![
                (removed.id.as_str(), Some(PackageState::Removed)),
                (disabled.id.as_str(), Some(PackageState::Disabled)),
            ]
        );
    }

    #[test]
    fn only_catalogued_packages_reach_the_worker() {
        let mut wizard = remover();
        wizard
            .selected
            .insert("com.example.injected; reboot".into());
        wizard.selected.insert(list().packages[0].id.clone());
        let targets = wizard.selected_targets(list());
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].package, list().packages[0]);
    }
}
