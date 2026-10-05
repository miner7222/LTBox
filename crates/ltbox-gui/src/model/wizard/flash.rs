//! Flash wizard state.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeviceRegion {
    Prc,
    Row,
}
impl DeviceRegion {
    pub(crate) fn label_key(&self) -> &'static str {
        match self {
            Self::Prc => "deviceregion_prc",
            Self::Row => "deviceregion_row",
        }
    }

    pub(crate) fn to_region_target(self) -> ltbox_patch::region::RegionTarget {
        match self {
            Self::Prc => ltbox_patch::region::RegionTarget::Prc,
            Self::Row => ltbox_patch::region::RegionTarget::Row,
        }
    }
}

/// Selection state for the Flash wizard's region step. Automatic lookup is a
/// method for resolving a [`DeviceRegion`], never a region passed downstream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FlashRegionSelection {
    Auto,
    Manual(DeviceRegion),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FlashTarget {
    OtherRegion,
    SameRegion,
}
impl FlashTarget {
    pub(crate) fn label_key(&self) -> &'static str {
        match self {
            Self::OtherRegion => "flashtarget_other",
            Self::SameRegion => "flashtarget_same",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DataMode {
    Keep,
    Wipe,
}
impl DataMode {
    pub(crate) fn label_key(&self) -> &'static str {
        match self {
            Self::Keep => "datamode_keep",
            Self::Wipe => "datamode_wipe",
        }
    }
}

/// Which Flash-confirm summary row the "hidden dropdown" editor targets.
/// `Country` is special-cased to reuse the existing country popup; the
/// rest open the shared `flash_confirm_edit_popup`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConfirmField {
    Region,
    Target,
    Data,
    RegionEdit,
    Rollback,
    Country,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FlashStep {
    Region,
    Target,
    Data,
    Folder,
    Bootloader,
    Confirm,
    Flash,
}

impl FlashStep {
    pub(crate) fn label_key(self) -> &'static str {
        match self {
            Self::Region => "flash_step_region",
            Self::Target => "flash_step_target",
            Self::Data => "flash_step_data",
            Self::Folder => "flash_step_folder",
            Self::Bootloader => "flash_step_bootloader",
            Self::Confirm => "flash_step_confirm",
            Self::Flash => "flash_step_flash",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FirmwareIdentity {
    pub(crate) efisp_load: ltbox_patch::efisp_load::EfispLoad,
    pub(crate) key_class: ltbox_patch::key_map::KeyClass,
    pub(crate) fingerprint: Option<String>,
    pub(crate) model_token: Option<String>,
}

impl FirmwareIdentity {
    pub(crate) fn uses_gbl(&self) -> bool {
        self.fingerprint.as_deref().is_some_and(|fp| {
            ltbox_core::model::fingerprint_capabilities(fp).any(|caps| caps.root_uses_gbl)
        }) || self
            .model_token
            .as_deref()
            .is_some_and(|model| ltbox_core::model::capabilities(model).root_uses_gbl)
    }

    pub(crate) fn needs_bootloader_step(&self) -> bool {
        if self.uses_gbl() {
            self.efisp_load != ltbox_patch::efisp_load::EfispLoad::Yes
        } else {
            firmware_needs_bootloader_step(
                self.key_class,
                self.fingerprint.as_deref(),
                self.efisp_load,
            )
        }
    }

    pub(crate) fn from_avb_info(info: &ltbox_patch::avb::AvbImageInfo) -> Self {
        let fingerprint = ltbox_patch::avb::build_fingerprint(info);
        // A known model (or the codename a custom ROM uses for it) wins over
        // the product-name split, so a codename reports its canonical model.
        let model_token = fingerprint.as_deref().and_then(|fp| {
            ltbox_core::model::fingerprint_models(fp)
                .next()
                .map(str::to_owned)
                .or_else(|| fingerprint_model_token(fp))
        });
        Self {
            key_class: ltbox_patch::key_map::classify_pubkey(info.public_key_sha1.as_deref()),
            efisp_load: ltbox_patch::efisp_load::EfispLoad::Undetermined,
            fingerprint,
            model_token,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FirmwareIdentityDialog {
    Ready,
    Failed(String),
}

fn fingerprint_model_token(fingerprint: &str) -> Option<String> {
    fingerprint
        .split('/')
        .nth(1)
        .and_then(|product| product.split('_').next())
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
}

pub(crate) fn firmware_needs_bootloader_step(
    key_class: ltbox_patch::key_map::KeyClass,
    fingerprint: Option<&str>,
    efisp_load: ltbox_patch::efisp_load::EfispLoad,
) -> bool {
    if fingerprint.is_some_and(|fp| {
        ltbox_core::model::fingerprint_capabilities(fp).any(|caps| caps.root_uses_gbl)
    }) {
        return efisp_load != ltbox_patch::efisp_load::EfispLoad::Yes;
    }
    key_class == ltbox_patch::key_map::KeyClass::Lenovo
        && !fingerprint.is_some_and(|fp| {
            ltbox_core::model::fingerprint_capabilities(fp).any(|caps| {
                matches!(
                    caps.rollback,
                    ltbox_core::model::RollbackPolicy::Gbl
                        | ltbox_core::model::RollbackPolicy::ReadOnly
                )
            })
        })
}

pub(crate) const FLASH_STEPS: &[FlashStep] = &[
    FlashStep::Region,
    FlashStep::Target,
    FlashStep::Data,
    FlashStep::Folder,
    FlashStep::Confirm,
    FlashStep::Flash,
];

const FLASH_STEPS_WITH_BOOTLOADER: &[FlashStep] = &[
    FlashStep::Region,
    FlashStep::Target,
    FlashStep::Data,
    FlashStep::Folder,
    FlashStep::Bootloader,
    FlashStep::Confirm,
    FlashStep::Flash,
];

#[derive(Default)]
pub(crate) struct FlashWizard {
    pub(crate) step: usize,
    pub(crate) region_selection: Option<FlashRegionSelection>,
    pub(crate) device_region: Option<DeviceRegion>,
    pub(crate) region_auto_unknown: bool,
    pub(crate) target: Option<FlashTarget>,
    pub(crate) data_mode: Option<DataMode>,
    pub(crate) firmware_folder: Option<String>,
    /// Original `boot` / `vbmeta_system` rollback indices read from the
    /// selected firmware. Missing entries retain their reason for display.
    pub(crate) firmware_rollback_indices: Option<(Result<u64, String>, Result<u64, String>)>,
    /// `true` when the selected firmware folder ships no EDL loader, so the
    /// folder step requires a separately-picked loader before advancing.
    pub(crate) loader_required: bool,
    /// User-picked EDL loader (or the model's remembered one) used when the
    /// firmware folder has none. `None` + `loader_required` blocks Next.
    pub(crate) loader_override: Option<String>,
    /// Reason the last picked loader was rejected (e.g. a standalone `.melf` on
    /// TB323FU), shown in the folder step.
    pub(crate) loader_error: Option<String>,
    pub(crate) firmware_identity: Option<FirmwareIdentity>,
    pub(crate) firmware_identity_pending: bool,
    pub(crate) firmware_identity_dialog: Option<FirmwareIdentityDialog>,
    pub(crate) user_abl_path: Option<String>,
    pub(crate) user_abl_key_class: Option<ltbox_patch::key_map::KeyClass>,
    pub(crate) user_abl_analyzing: bool,
    pub(crate) user_abl_efisp_load: ltbox_patch::efisp_load::EfispLoad,
    pub(crate) no_efisp_load: bool,
}

impl FlashWizard {
    pub(crate) fn set_firmware_rollback_indices(&mut self, folder: &str) {
        let read = |filename: &str| -> Result<u64, String> {
            ltbox_patch::avb::extract_image_avb_info(
                std::path::Path::new(folder).join(filename).as_path(),
            )
            .map(|info| info.rollback_index)
            .map_err(|error| error.to_string())
        };
        self.firmware_rollback_indices = Some((read("boot.img"), read("vbmeta_system.img")));
    }

    pub(crate) fn visible_steps(&self) -> &'static [FlashStep] {
        if self
            .firmware_identity
            .as_ref()
            .is_some_and(|identity| identity.needs_bootloader_step())
        {
            FLASH_STEPS_WITH_BOOTLOADER
        } else {
            FLASH_STEPS
        }
    }

    pub(crate) fn current_step(&self) -> FlashStep {
        self.visible_steps()
            .get(self.step)
            .copied()
            .unwrap_or(FlashStep::Flash)
    }

    pub(crate) fn set_step(&mut self, step: FlashStep) {
        self.step = self
            .visible_steps()
            .iter()
            .position(|candidate| *candidate == step)
            .unwrap_or(0);
    }

    pub(crate) fn reset_firmware_identity(&mut self) {
        self.firmware_identity = None;
        self.firmware_identity_pending = false;
        self.firmware_identity_dialog = None;
        self.clear_bootloader();
    }

    pub(crate) fn clear_bootloader(&mut self) {
        self.user_abl_path = None;
        self.user_abl_key_class = None;
        self.user_abl_analyzing = false;
        self.user_abl_efisp_load = ltbox_patch::efisp_load::EfispLoad::Undetermined;
        self.no_efisp_load = false;
    }

    pub(crate) fn uses_gbl(&self) -> bool {
        self.firmware_identity
            .as_ref()
            .is_some_and(FirmwareIdentity::uses_gbl)
    }

    pub(crate) fn bootloader_can_next(&self) -> bool {
        if self.user_abl_analyzing {
            return false;
        }
        if self.uses_gbl() {
            match self.user_abl_path {
                Some(_) => self.user_abl_efisp_load == ltbox_patch::efisp_load::EfispLoad::Yes,
                None => self.firmware_identity.as_ref().is_some_and(|identity| {
                    identity.efisp_load == ltbox_patch::efisp_load::EfispLoad::Yes
                        || (identity.efisp_load == ltbox_patch::efisp_load::EfispLoad::No
                            && identity.key_class != ltbox_patch::key_map::KeyClass::Testkey)
                }),
            }
        } else {
            self.user_abl_path.is_none()
                || self.user_abl_key_class == Some(ltbox_patch::key_map::KeyClass::Testkey)
        }
    }

    pub(crate) fn record_bootloader_decision(&mut self) {
        self.no_efisp_load = self.uses_gbl()
            && self.user_abl_path.is_none()
            && self.firmware_identity.as_ref().is_some_and(|identity| {
                identity.efisp_load == ltbox_patch::efisp_load::EfispLoad::No
            });
    }

    pub(crate) fn bootloader_execution_allowed(&self) -> bool {
        self.bootloader_can_next()
            && (!self.uses_gbl()
                || self.user_abl_path.is_some()
                || self.firmware_identity.as_ref().is_some_and(|identity| {
                    identity.efisp_load == ltbox_patch::efisp_load::EfispLoad::Yes
                        || (identity.efisp_load == ltbox_patch::efisp_load::EfispLoad::No
                            && self.no_efisp_load)
                }))
    }
}

impl Wizard for FlashWizard {
    fn step(&self) -> usize {
        self.step
    }
    fn step_mut(&mut self) -> &mut usize {
        &mut self.step
    }
    fn step_count(&self) -> usize {
        self.visible_steps().len()
    }
    fn can_next(&self) -> bool {
        match self.current_step() {
            FlashStep::Region => match self.region_selection {
                Some(FlashRegionSelection::Auto) => true,
                Some(FlashRegionSelection::Manual(region)) => self.device_region == Some(region),
                None => false,
            },
            FlashStep::Target => self.target.is_some(),
            FlashStep::Data => self.data_mode.is_some(),
            // Folder picked, and — when it ships no loader — a loader provided.
            FlashStep::Folder => {
                self.firmware_folder.is_some()
                    && (!self.loader_required || self.loader_override.is_some())
                    && !self.firmware_identity_pending
            }
            FlashStep::Bootloader => self.bootloader_can_next(),
            FlashStep::Confirm => {
                self.firmware_identity.is_some()
                    && self.bootloader_execution_allowed()
                    && self.firmware_folder.is_some()
                    && (!self.loader_required || self.loader_override.is_some())
            }
            FlashStep::Flash => false,
        }
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod flash_tests {
    use super::*;
    use ltbox_patch::key_map::KeyClass;

    fn identity(key_class: KeyClass, fingerprint: &str) -> FirmwareIdentity {
        FirmwareIdentity {
            key_class,
            efisp_load: ltbox_patch::efisp_load::EfispLoad::Undetermined,
            fingerprint: Some(fingerprint.to_string()),
            model_token: None,
        }
    }

    #[test]
    fn visible_flash_steps_include_bootloader_only_when_selected_by_gate() {
        let mut without = FlashWizard::default();
        assert_eq!(
            without.visible_steps(),
            &[
                FlashStep::Region,
                FlashStep::Target,
                FlashStep::Data,
                FlashStep::Folder,
                FlashStep::Confirm,
                FlashStep::Flash,
            ]
        );
        without.set_step(FlashStep::Folder);
        without.next();
        assert_eq!(without.current_step(), FlashStep::Confirm);
        without.back();
        assert_eq!(without.current_step(), FlashStep::Folder);

        let mut with = FlashWizard {
            firmware_identity: Some(identity(
                KeyClass::Lenovo,
                "qti/TB320FC/TB320FC:15/build:user/release-keys",
            )),
            ..FlashWizard::default()
        };
        assert_eq!(
            with.visible_steps(),
            &[
                FlashStep::Region,
                FlashStep::Target,
                FlashStep::Data,
                FlashStep::Folder,
                FlashStep::Bootloader,
                FlashStep::Confirm,
                FlashStep::Flash,
            ]
        );
        with.set_step(FlashStep::Folder);
        with.next();
        assert_eq!(with.current_step(), FlashStep::Bootloader);
        with.next();
        assert_eq!(with.current_step(), FlashStep::Confirm);
        with.back();
        assert_eq!(with.current_step(), FlashStep::Bootloader);
        with.back();
        assert_eq!(with.current_step(), FlashStep::Folder);
    }

    #[test]
    fn canoe_efisp_routes_and_candidate_gates() {
        use ltbox_patch::efisp_load::EfispLoad::{No, Undetermined, Yes};
        for model in ["TB323FU", "TB324ZC"] {
            for state in [Yes, No, Undetermined] {
                let mut identity = identity(
                    KeyClass::Unknown,
                    &format!("qti/{model}/{model}:15/build:user/release-keys"),
                );
                identity.efisp_load = state;
                let mut wizard = FlashWizard {
                    firmware_identity: Some(identity),
                    ..Default::default()
                };
                assert_eq!(
                    wizard.visible_steps().contains(&FlashStep::Bootloader),
                    state != Yes
                );
                assert_eq!(wizard.bootloader_can_next(), state != Undetermined);
                assert_eq!(wizard.bootloader_execution_allowed(), state == Yes);
                wizard.record_bootloader_decision();
                assert_eq!(wizard.no_efisp_load, state == No);
                assert_eq!(wizard.bootloader_execution_allowed(), state != Undetermined);
                wizard.user_abl_path = Some("candidate.elf".into());
                for candidate in [Yes, No, Undetermined] {
                    wizard.user_abl_efisp_load = candidate;
                    for key in [KeyClass::Testkey, KeyClass::Lenovo, KeyClass::Unknown] {
                        wizard.user_abl_key_class = Some(key);
                        assert_eq!(wizard.bootloader_can_next(), candidate == Yes);
                    }
                }
                wizard.user_abl_efisp_load = Yes;
                wizard.user_abl_analyzing = true;
                assert!(!wizard.bootloader_can_next());
                wizard.clear_bootloader();
                assert!(!wizard.no_efisp_load);
                assert_eq!(wizard.user_abl_efisp_load, Undetermined);
                wizard.reset_firmware_identity();
                assert!(wizard.firmware_identity.is_none());
            }
        }
    }

    #[test]
    fn canoe_testkey_without_efisp_requires_an_efisp_bootloader() {
        use ltbox_patch::efisp_load::EfispLoad::{No, Undetermined, Yes};
        for model in ["TB323FU", "TB324ZC"] {
            for firmware_state in [No, Undetermined, Yes] {
                for token_only in [false, true] {
                    let mut firmware = identity(
                        KeyClass::Testkey,
                        &format!("qti/{model}/{model}:15/build:user/test-keys"),
                    );
                    firmware.efisp_load = firmware_state;
                    if token_only {
                        firmware.fingerprint = None;
                        firmware.model_token = Some(model.into());
                    }
                    let mut wizard = FlashWizard {
                        firmware_identity: Some(firmware),
                        ..Default::default()
                    };
                    assert_eq!(wizard.bootloader_can_next(), firmware_state == Yes);
                    // A stale opt-out must not bypass the required candidate.
                    wizard.no_efisp_load = true;
                    assert_eq!(wizard.bootloader_execution_allowed(), firmware_state == Yes);
                    wizard.user_abl_path = Some("candidate.elf".into());
                    for candidate in [No, Undetermined, Yes] {
                        wizard.user_abl_efisp_load = candidate;
                        assert_eq!(wizard.bootloader_can_next(), candidate == Yes);
                        assert_eq!(wizard.bootloader_execution_allowed(), candidate == Yes);
                    }
                    wizard.user_abl_analyzing = true;
                    assert!(!wizard.bootloader_can_next());
                    wizard.clear_bootloader();
                    assert_eq!(wizard.bootloader_can_next(), firmware_state == Yes);
                }
            }
        }
    }

    #[test]
    fn other_models_still_require_testkey_candidates() {
        use ltbox_patch::efisp_load::EfispLoad::Yes;
        let mut wizard = FlashWizard {
            firmware_identity: Some(identity(
                KeyClass::Lenovo,
                "qti/TB320FC/TB320FC:15/build:user/release-keys",
            )),
            user_abl_path: Some("candidate.elf".into()),
            user_abl_efisp_load: Yes,
            ..Default::default()
        };
        for key in [KeyClass::Testkey, KeyClass::Lenovo, KeyClass::Unknown] {
            wizard.user_abl_key_class = Some(key);
            assert_eq!(wizard.bootloader_can_next(), key == KeyClass::Testkey);
        }
    }

    #[test]
    fn firmware_bootloader_gate_matches_key_and_model_rules() {
        let other = "qti/TB320FC/TB320FC:15/build:user/release-keys";
        assert!(firmware_needs_bootloader_step(
            KeyClass::Lenovo,
            Some(other),
            ltbox_patch::efisp_load::EfispLoad::Undetermined
        ));

        for model in ["TB323FU", "TB376FC", "TB390FU", "TB391FC"] {
            let fingerprint = format!("qti/{model}/{model}:15/build:user/release-keys");
            assert!(!firmware_needs_bootloader_step(
                KeyClass::Lenovo,
                Some(&fingerprint),
                ltbox_patch::efisp_load::EfispLoad::Yes
            ));
        }

        assert!(!firmware_needs_bootloader_step(
            KeyClass::Testkey,
            Some(other),
            ltbox_patch::efisp_load::EfispLoad::Undetermined
        ));
        assert!(!firmware_needs_bootloader_step(
            KeyClass::Unknown,
            Some(other),
            ltbox_patch::efisp_load::EfispLoad::Undetermined
        ));
    }
}
