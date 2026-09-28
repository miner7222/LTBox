//! Debloat catalogue: per-model lists of preinstalled apps LTBox can remove
//! for the current user, bundled from `debloat/*.json`.
//!
//! Each list is reviewed in the repository and ships with the binary, so a
//! list only changes through a release. `recommended` marks the preset the
//! wizard preselects; everything else stays opt-in.

use serde::Deserialize;

/// How an app is taken away, and therefore how it comes back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DebloatMethod {
    /// `pm uninstall -k --user 0`; restored with `cmd package install-existing`.
    Uninstall,
    /// `pm disable-user --user 0`; restored with `pm enable`.
    Disable,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DebloatPackage {
    pub(crate) id: String,
    /// App name as the device shows it. Product names, so not localized.
    pub(crate) label: String,
    pub(crate) method: DebloatMethod,
    pub(crate) recommended: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DebloatList {
    pub(crate) models: Vec<String>,
    pub(crate) packages: Vec<DebloatPackage>,
}

const LIST_SOURCES: &[(&str, &str)] = &[("TB321FU.json", include_str!("../debloat/TB321FU.json"))];

static LISTS: std::sync::LazyLock<Vec<DebloatList>> = std::sync::LazyLock::new(|| {
    LIST_SOURCES
        .iter()
        .map(|(name, source)| {
            serde_json::from_str(source)
                .unwrap_or_else(|error| panic!("debloat/{name} must parse: {error}"))
        })
        .collect()
});

/// The bundled list for `model`, if LTBox ships one.
pub(crate) fn list_for_model(model: &str) -> Option<&'static DebloatList> {
    LISTS.iter().find(|list| {
        list.models
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(model))
    })
}

/// Where a catalogued app stands for user 0 on the connected device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PackageState {
    Installed,
    /// Installed but disabled (`disable-user` or equivalent).
    Disabled,
    /// On the system image but uninstalled for user 0.
    Removed,
    /// Not on this firmware at all.
    Absent,
}

/// Package ids from `pm list packages` output (`package:<id>` lines).
fn listed_packages(output: &str) -> std::collections::BTreeSet<&str> {
    output
        .lines()
        .filter_map(|line| line.trim().strip_prefix("package:"))
        .collect()
}

/// Classify `id` from the three listings `pm list packages` gives for user 0:
/// everything including uninstalled (`-u`), installed, and disabled (`-d`).
pub(crate) fn package_states<'a>(
    ids: impl IntoIterator<Item = &'a str>,
    all_output: &str,
    installed_output: &str,
    disabled_output: &str,
) -> std::collections::BTreeMap<String, PackageState> {
    let all = listed_packages(all_output);
    let installed = listed_packages(installed_output);
    let disabled = listed_packages(disabled_output);
    ids.into_iter()
        .map(|id| {
            let state = if disabled.contains(id) {
                PackageState::Disabled
            } else if installed.contains(id) {
                PackageState::Installed
            } else if all.contains(id) {
                PackageState::Removed
            } else {
                PackageState::Absent
            };
            (id.to_string(), state)
        })
        .collect()
}

/// Android package names are the only thing interpolated into a shell
/// command, so the catalogue must never hold anything else.
pub(crate) fn is_valid_package_id(id: &str) -> bool {
    let mut segments = id.split('.');
    let valid_segment = |segment: &str| {
        segment
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_alphabetic())
            && segment
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    };
    id.contains('.') && segments.all(valid_segment)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn every_bundled_list_parses_with_safe_unique_packages() {
        assert_eq!(LISTS.len(), LIST_SOURCES.len());
        let mut models = BTreeSet::new();
        for (list, (name, _)) in LISTS.iter().zip(LIST_SOURCES) {
            assert!(!list.models.is_empty(), "{name}: no model");
            assert!(!list.packages.is_empty(), "{name}: no package");
            assert!(
                list.packages.iter().any(|package| package.recommended),
                "{name}: the recommended preset is empty"
            );
            for model in &list.models {
                assert!(
                    ltbox_core::model::SUPPORTED_MODELS.contains(&model.as_str()),
                    "{name}: {model} is not a supported model"
                );
                assert!(models.insert(model.clone()), "{model} has two lists");
            }
            let mut ids = BTreeSet::new();
            for package in &list.packages {
                assert!(
                    is_valid_package_id(&package.id),
                    "{name}: {} is not a package name",
                    package.id
                );
                assert!(!package.label.trim().is_empty(), "{name}: {}", package.id);
                assert!(ids.insert(&package.id), "{name}: {} twice", package.id);
            }
        }
    }

    #[test]
    fn lists_resolve_by_model_regardless_of_case() {
        assert!(list_for_model("TB321FU").is_some());
        assert!(list_for_model("tb321fu").is_some());
        assert!(list_for_model("TB320FC").is_none());
        assert!(list_for_model("").is_none());
    }

    #[test]
    fn package_states_follow_the_user_0_listings() {
        let states = package_states(
            [
                "com.zui.notes",
                "com.zui.browser",
                "com.zui.gallery",
                "com.zui.weather",
            ],
            "package:com.zui.notes\npackage:com.zui.browser\npackage:com.zui.gallery\r\n",
            "package:com.zui.notes\npackage:com.zui.browser\n",
            "package:com.zui.browser\n",
        );
        assert_eq!(states["com.zui.notes"], PackageState::Installed);
        assert_eq!(states["com.zui.browser"], PackageState::Disabled);
        assert_eq!(states["com.zui.gallery"], PackageState::Removed);
        assert_eq!(states["com.zui.weather"], PackageState::Absent);
    }

    #[test]
    fn package_ids_reject_shell_syntax() {
        for id in ["com.zui.notes", "cn.wps.moffice_eng", "io.moreless.tide"] {
            assert!(is_valid_package_id(id), "{id}");
        }
        for id in [
            "",
            "notes",
            "com.zui.notes; reboot",
            "com.zui.notes && rm",
            "com..notes",
            "com.1zui",
            "com.zui.notes\n",
            "$(id).x",
        ] {
            assert!(!is_valid_package_id(id), "{id:?}");
        }
    }
}
