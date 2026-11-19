//! Queries over the connected device and its model capabilities.

use crate::*;

/// UI-side model check for the selected firmware, in front of the flash
/// worker's own pre-EDL check (which stays authoritative).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FlashModelPrecheck {
    Match,
    Mismatch,
    /// No fingerprint, no device model, or an EDL start (the worker reads the
    /// model over EDL instead). Never blocks and never reads as a match.
    NotChecked,
}

// `app::device` is private and not re-exported, so callers outside `app` use
// these predicates instead of naming the variants.
impl FlashModelPrecheck {
    pub(crate) fn is_match(self) -> bool {
        self == Self::Match
    }

    pub(crate) fn is_mismatch(self) -> bool {
        self == Self::Mismatch
    }

    #[cfg(test)]
    pub(crate) fn is_not_checked(self) -> bool {
        self == Self::NotChecked
    }
}

impl App {
    /// The flash worker's non-EDL model check, evaluated on current state.
    /// Same inputs and predicate: the fingerprint from the folder inspection,
    /// the polled model with spaces removed, `fingerprint_model_match`.
    pub(crate) fn flash_model_precheck(&self) -> FlashModelPrecheck {
        // The worker skips this check when it starts in EDL. With no device
        // connected, `device.model` may be left over from an earlier tablet,
        // so never report a verdict from it (Start is blocked anyway).
        if matches!(
            self.device.connection,
            ConnectionStatus::Edl | ConnectionStatus::None
        ) {
            return FlashModelPrecheck::NotChecked;
        }
        let Some(fingerprint) = self
            .flash
            .firmware_identity
            .as_ref()
            .and_then(|identity| identity.fingerprint.as_deref())
        else {
            return FlashModelPrecheck::NotChecked;
        };
        let normalized_model = self.device.model.replace(' ', "");
        if normalized_model.is_empty() {
            FlashModelPrecheck::NotChecked
        } else if ltbox_core::model::fingerprint_model_match(fingerprint, &normalized_model) {
            FlashModelPrecheck::Match
        } else {
            FlashModelPrecheck::Mismatch
        }
    }

    /// The blocking message for a model mismatch, or `None` when the
    /// pre-check passes or cannot be evaluated.
    pub(crate) fn flash_model_precheck_error(&self) -> Option<String> {
        if self.flash_model_precheck() != FlashModelPrecheck::Mismatch {
            return None;
        }
        let firmware = self
            .flash
            .firmware_identity
            .as_ref()
            .and_then(|identity| identity.model_token.clone())
            .unwrap_or_else(|| self.t("flash_firmware_model_unknown").to_string());
        Some(tr_args!(
            "err_flash_model_precheck",
            firmware = firmware,
            device = self.device.model.clone()
        ))
    }

    /// Label key for the live connection: [`ConnectionStatus::label_key`],
    /// except a Fastboot endpoint reporting `is-userspace` reads as fastbootd.
    pub(crate) fn connection_label_key(&self) -> &'static str {
        if self.device.connection == ConnectionStatus::Fastboot && self.device.fastboot_userspace {
            "conn_fastbootd"
        } else {
            self.device.connection.label_key()
        }
    }

    /// The connected dual-USB-C model whose port guide may open, or `None`.
    /// Requires the dual-USB capability and that the user has neither dismissed
    /// ("don't show again") nor closed it for this session.
    pub(crate) fn dual_usb_advisory_model(&self) -> Option<&str> {
        let m = self.device.model.as_str();
        let hidden = |list: &[String]| list.iter().any(|x| x.eq_ignore_ascii_case(m));
        if !m.is_empty()
            && is_dual_usbc_model(m)
            && !hidden(&self.dual_usb_advisory_dismissed)
            && !hidden(&self.dual_usb_advisory_closed)
        {
            Some(m)
        } else {
            None
        }
    }

    /// Whether the Dashboard's rollback cell opens the floor breakdown.
    ///
    /// Floors are only populated by a bootloader-mode poll, so their presence
    /// is the "in bootloader" test. The model check keeps rollback-unprotected
    /// models (TB322FC, TB324ZC) from offering a breakdown behind a cell that
    /// reads "No".
    #[cfg(test)]
    pub(crate) fn rollback_detail_available(&self) -> bool {
        self.device.rollback_floors.is_some() && is_rollback_protected_model(&self.device.model)
    }

    pub(crate) fn is_nav_enabled(&self, view: View) -> bool {
        // About is device/platform-independent.
        if matches!(view, View::About) {
            return true;
        }
        if self.device.platform_supported == Some(false) {
            return matches!(view, View::Dashboard | View::SystemUpdate | View::Settings);
        }
        true
    }

    /// The same model policy used by the workers and patch pipeline.
    pub(crate) fn model_capabilities(&self) -> &'static ltbox_core::model::ModelCapabilities {
        ltbox_core::model::capabilities(&self.device.model)
    }

    /// GBL policy follows the target firmware; read-only applies if either the
    /// device or the target requires it. Without a folder, uses the device's.
    pub(crate) fn flash_rollback_policy(&self) -> ltbox_core::model::RollbackPolicy {
        use ltbox_core::model::{RollbackPolicy, fingerprint_capabilities};
        let device = self.model_capabilities().rollback;
        let target = self
            .flash
            .firmware_identity
            .as_ref()
            .and_then(|identity| identity.fingerprint.as_deref())
            .and_then(|fp| {
                fingerprint_capabilities(fp)
                    .map(|profile| profile.rollback)
                    .reduce(|a, b| {
                        if a == RollbackPolicy::ReadOnly || b == RollbackPolicy::ReadOnly {
                            RollbackPolicy::ReadOnly
                        } else if a == RollbackPolicy::Gbl || b == RollbackPolicy::Gbl {
                            RollbackPolicy::Gbl
                        } else {
                            a
                        }
                    })
            });
        if device == RollbackPolicy::ReadOnly || target == Some(RollbackPolicy::ReadOnly) {
            RollbackPolicy::ReadOnly
        } else {
            target.unwrap_or(device)
        }
    }

    pub(crate) fn is_xiaoxin_pro13(&self) -> bool {
        ltbox_core::model::is_xiaoxin_pro13_model(&self.device.model)
    }

    /// Which images the unroot folder picker should name.
    ///
    /// What a root backup holds is decided by the root run: the image target
    /// follows the route and model, and a vbmeta image is present only when
    /// rebuilt. Asking the same two functions keeps the picker from naming a
    /// file that run never wrote.
    pub(crate) fn unroot_folder_desc(&self, unroot_type: UnrootType) -> &str {
        let target = ltbox_patch::root_pipeline::resolve_root_image_target(
            ltbox_patch::root_pipeline::RootFamily::Magisk,
            matches!(unroot_type, UnrootType::APatchGki),
            &self.device.model,
        );
        // The testkey efisp/GBL route (`root_uses_gbl`) leaves vbmeta out of the
        // backup, overriding the rebuild rule.
        let with_vbmeta = !root_skips_avb_postprocess(&self.device.model)
            && ltbox_patch::root_pipeline::root_run_rebuilds_vbmeta(target, &self.device.model);
        match (target, with_vbmeta) {
            (ltbox_patch::root_pipeline::RootImageTarget::Boot, true) => {
                self.t("unroot_folderdesc_boot_vbmeta")
            }
            (ltbox_patch::root_pipeline::RootImageTarget::Boot, false) => {
                self.t("unroot_folderdesc_boot")
            }
            (ltbox_patch::root_pipeline::RootImageTarget::InitBoot, true) => {
                self.t("unroot_folderdesc_init_boot_vbmeta")
            }
            (ltbox_patch::root_pipeline::RootImageTarget::InitBoot, false) => {
                self.t("unroot_folderdesc_init_boot")
            }
        }
    }

    pub(crate) fn advanced_picker_exts(&self) -> (&'static str, &'static [&'static str]) {
        if matches!(
            self.adv_wizard.action,
            Some(AdvAction::DetectArb | AdvAction::PatchDevinfo)
        ) {
            ("EDL loader", self.loader_picker_exts())
        } else {
            self.adv_wizard.accepted_exts()
        }
    }

    /// Whether the polled device is a PRC-only model (`prc_only` capability).
    /// The Flash wizard disables the ROW and OtherRegion cards for it.
    pub(crate) fn is_prc_only(&self) -> bool {
        self.model_capabilities().prc_only
    }

    /// True when the dashboard poll has the device in a mode the wizards can
    /// transition out of. Gates each wizard's "Start" button: no connection or
    /// `AdbUnauthorized` would only make the worker bail with "no device".
    pub(crate) fn device_reachable(&self) -> bool {
        matches!(
            self.device.connection,
            ConnectionStatus::Adb
                | ConnectionStatus::AdbRecovery
                | ConnectionStatus::Fastboot
                | ConnectionStatus::Edl
        )
    }

    /// Why [`Self::device_reachable`] fails, as a sentence for a disabled
    /// Start; `None` exactly when it holds. Exhaustive so a new connection
    /// state must be given a reason.
    pub(crate) fn device_block_reason(&self) -> Option<String> {
        let key = match self.device.connection {
            ConnectionStatus::Adb
            | ConnectionStatus::AdbRecovery
            | ConnectionStatus::Fastboot
            | ConnectionStatus::Edl => return None,
            ConnectionStatus::None => "err_no_device_connected",
            ConnectionStatus::AdbServerBlocking => "start_blocked_adb_server",
            ConnectionStatus::AdbUnauthorized => "start_blocked_adb_unauthorized",
            ConnectionStatus::AdbSideload => "start_blocked_adb_sideload",
        };
        Some(self.t(key).to_string())
    }

    /// The muted line under a wizard's first step while no device is
    /// reachable. "No tablet is connected" only when none is; a connected but
    /// unusable tablet gets its own cause instead. Never blocks Next.
    pub(crate) fn wizard_device_note(&self) -> Option<String> {
        if self.device.connection == ConnectionStatus::None {
            Some(self.t("wizard_device_note").to_string())
        } else {
            self.device_block_reason()
        }
    }

    /// `model_unsupported` for the polled model.
    pub(crate) fn model_unsupported_reason(&self) -> String {
        tr_args!("model_unsupported", model = self.device.model.as_str())
    }
}

#[cfg(test)]
mod start_reason_tests {
    use super::*;

    const ALL_CONNECTIONS: [ConnectionStatus; 8] = [
        ConnectionStatus::None,
        ConnectionStatus::Adb,
        ConnectionStatus::AdbRecovery,
        ConnectionStatus::AdbUnauthorized,
        ConnectionStatus::AdbSideload,
        ConnectionStatus::AdbServerBlocking,
        ConnectionStatus::Fastboot,
        ConnectionStatus::Edl,
    ];

    /// Exhaustive: a new variant fails to compile here until it is listed.
    fn expected_reason_key(conn: ConnectionStatus) -> Option<&'static str> {
        match conn {
            ConnectionStatus::Adb
            | ConnectionStatus::AdbRecovery
            | ConnectionStatus::Fastboot
            | ConnectionStatus::Edl => None,
            ConnectionStatus::None => Some("err_no_device_connected"),
            ConnectionStatus::AdbServerBlocking => Some("start_blocked_adb_server"),
            ConnectionStatus::AdbUnauthorized => Some("start_blocked_adb_unauthorized"),
            ConnectionStatus::AdbSideload => Some("start_blocked_adb_sideload"),
        }
    }

    fn app_on(conn: ConnectionStatus) -> App {
        let mut app = App::default();
        app.device.connection = conn;
        app
    }

    #[test]
    fn device_block_reason_is_none_for_the_four_reachable_states() {
        for conn in [
            ConnectionStatus::Adb,
            ConnectionStatus::AdbRecovery,
            ConnectionStatus::Fastboot,
            ConnectionStatus::Edl,
        ] {
            let app = app_on(conn);
            assert!(app.device_reachable(), "{conn:?}");
            assert_eq!(app.device_block_reason(), None, "{conn:?}");
        }
    }

    #[test]
    fn device_block_reason_names_every_unreachable_state() {
        for conn in ALL_CONNECTIONS {
            let app = app_on(conn);
            assert_eq!(
                app.device_block_reason().is_none(),
                app.device_reachable(),
                "{conn:?}"
            );
            assert_eq!(
                app.device_block_reason(),
                expected_reason_key(conn).map(|key| app.t(key).to_string()),
                "{conn:?}"
            );
        }
    }

    #[test]
    fn device_block_reason_step_one_note_claims_no_tablet_only_when_none() {
        for conn in ALL_CONNECTIONS {
            let app = app_on(conn);
            let note = app.wizard_device_note();
            assert_eq!(note.is_some(), !app.device_reachable(), "{conn:?}");
            if conn == ConnectionStatus::None {
                assert_eq!(note.as_deref(), Some(app.t("wizard_device_note")));
            } else {
                assert_eq!(note, app.device_block_reason(), "{conn:?}");
            }
        }
    }

    fn running(app: &mut App) {
        app.operation.start(None, OperationKind::Unphased, None);
    }

    fn unsupported_model(
        pick: impl Fn(&ltbox_core::model::ModelCapabilities) -> bool,
    ) -> &'static str {
        ltbox_core::model::SUPPORTED_MODELS
            .iter()
            .copied()
            .find(|model| !pick(ltbox_core::model::capabilities(model)))
            .expect("a model without the capability")
    }

    /// Shared assertions for one footer state: a reason only while disabled,
    /// and every disabled Start that is not busy says why.
    fn assert_reason_consistent(can: bool, reason: Option<String>, is_start: bool, app: &App) {
        if can {
            assert!(reason.is_none(), "enabled Start shows {reason:?}");
        } else if is_start && !app.operation.is_running() {
            assert!(reason.is_some(), "disabled Start without a reason");
        }
    }

    #[test]
    fn device_block_reason_root_start_gate_is_pinned() {
        for conn in ALL_CONNECTIONS {
            for busy in [false, true] {
                let mut app = app_on(conn);
                app.root.step = 6;
                if busy {
                    running(&mut app);
                }
                let can = app.root_nav_can();
                assert_eq!(can, !busy && app.device_reachable(), "{conn:?} busy={busy}");
                assert_reason_consistent(can, app.root_nav_block_reason(), true, &app);
                if !busy {
                    assert_eq!(app.root_nav_block_reason(), app.device_block_reason());
                }
            }
        }
        // The capability gates every step and wins over the device cause.
        let mut app = app_on(ConnectionStatus::None);
        app.device.model = unsupported_model(|caps| caps.root).to_string();
        app.root.family = Some(Family::Magisk);
        assert!(!app.root_nav_can());
        assert_eq!(
            app.root_nav_block_reason(),
            Some(app.model_unsupported_reason())
        );
        // A missing selection on a non-Start step has no footer reason.
        let app = app_on(ConnectionStatus::None);
        assert!(!app.root_nav_can());
        assert_eq!(app.root_nav_block_reason(), None);
        // The device is not needed before Start.
        let mut app = app_on(ConnectionStatus::None);
        app.root.family = Some(Family::Magisk);
        assert!(app.root_nav_can());
        assert_eq!(app.root_nav_block_reason(), None);
    }

    #[test]
    fn device_block_reason_unroot_keeps_its_stricter_rule() {
        for conn in ALL_CONNECTIONS {
            for busy in [false, true] {
                let mut app = app_on(conn);
                app.unroot.step = 3;
                if busy {
                    running(&mut app);
                }
                let ready = crate::workers::unroot::unroot_connection_ready(conn);
                let can = app.unroot_nav_can();
                assert_eq!(can, !busy && ready, "{conn:?} busy={busy}");
                let reason = app.unroot_nav_block_reason();
                assert_reason_consistent(can, reason.clone(), true, &app);
                if !busy && !ready {
                    assert_eq!(
                        reason.as_deref(),
                        Some(app.t("err_unroot_connection_required"))
                    );
                }
            }
        }
        let mut app = app_on(ConnectionStatus::Adb);
        app.device.model = unsupported_model(|caps| caps.unroot).to_string();
        app.unroot.step = 3;
        assert!(!app.unroot_nav_can());
        assert_eq!(
            app.unroot_nav_block_reason(),
            Some(app.model_unsupported_reason())
        );
    }

    #[test]
    fn device_block_reason_sysupdate_and_simple_flash_start_gates_are_pinned() {
        for conn in ALL_CONNECTIONS {
            for busy in [false, true] {
                let mut app = app_on(conn);
                app.sysupdate.action = Some(SysUpdateAction::Disable);
                app.sysupdate.step = 1;
                app.simple_flash.step = 1;
                if busy {
                    running(&mut app);
                }
                let expected = !busy && app.device_reachable();
                let can = app.sysupdate_nav_can();
                assert_eq!(can, expected, "{conn:?} busy={busy}");
                assert_reason_consistent(can, app.sysupdate_nav_block_reason(), true, &app);
                let can = app.simple_flash_nav_can();
                assert_eq!(can, expected, "{conn:?} busy={busy}");
                assert_reason_consistent(can, app.simple_flash_nav_block_reason(), true, &app);
            }
        }
    }

    #[test]
    fn device_block_reason_partition_tools_gates_are_pinned() {
        for conn in ALL_CONNECTIONS {
            for busy in [false, true] {
                let mut app = app_on(conn);
                app.flash_phys.step = 2;
                app.flash_phys.selected[0] = true;
                app.flash_phys.file_paths[0] = Some("/tmp/lun0.bin".to_string());
                app.dump_phys.step = 1;
                app.dump_phys.selected[0] = true;
                if busy {
                    running(&mut app);
                }
                let expected = !busy && app.device_reachable();
                let can = app.flash_phys_nav_can();
                assert_eq!(can, expected, "{conn:?} busy={busy}");
                assert_reason_consistent(can, app.flash_phys_nav_block_reason(), true, &app);
                let can = app.dump_phys_nav_can();
                assert_eq!(can, expected, "{conn:?} busy={busy}");
                assert_reason_consistent(can, app.dump_phys_nav_block_reason(), true, &app);
            }
        }
        // Flash Partitions' Scan (step 0) and dump steps need a loader first;
        // without one the footer explains nothing and stays disabled.
        let app = app_on(ConnectionStatus::Adb);
        assert!(!app.flash_parts_nav_can());
        assert_eq!(app.flash_parts_nav_block_reason(), None);
        assert!(!app.dump_parts_nav_can());
        assert_eq!(app.dump_parts_nav_block_reason(), None);
        let mut app = app_on(ConnectionStatus::None);
        app.flash_parts.loader_path = Some("/tmp/prog.melf".to_string());
        app.dump_parts.loader_path = Some("/tmp/prog.melf".to_string());
        assert!(!app.flash_parts_nav_can());
        assert_eq!(
            app.flash_parts_nav_block_reason(),
            app.device_block_reason()
        );
        assert!(!app.dump_parts_nav_can());
        assert_eq!(app.dump_parts_nav_block_reason(), app.device_block_reason());
        app.device.connection = ConnectionStatus::Edl;
        assert!(app.flash_parts_nav_can());
        assert!(app.dump_parts_nav_can());
    }

    fn flash_confirm_app(conn: ConnectionStatus, model: &str) -> App {
        let mut app = app_on(conn);
        app.device.model = model.to_string();
        app.flash.firmware_folder = Some("/tmp/fw".to_string());
        app.flash.firmware_identity = Some(FirmwareIdentity {
            efisp_load: ltbox_patch::efisp_load::EfispLoad::Undetermined,
            key_class: ltbox_patch::key_map::KeyClass::Testkey,
            fingerprint: Some("Lenovo/TB520FU/TB520FU:17/CP2A/test:user/test-keys".to_string()),
            model_token: Some("TB520FU".to_string()),
        });
        app.flash.set_step(FlashStep::Confirm);
        app
    }

    #[test]
    fn device_block_reason_flash_confirm_gate_is_pinned() {
        for conn in ALL_CONNECTIONS {
            for busy in [false, true] {
                // Empty model: the pre-check is not run and never blocks.
                let mut app = flash_confirm_app(conn, "");
                if busy {
                    running(&mut app);
                }
                let can = app.flash_nav_can();
                assert_eq!(can, !busy && app.device_reachable(), "{conn:?} busy={busy}");
                assert_reason_consistent(can, app.flash_nav_block_reason(), true, &app);
            }
        }
        // The model mismatch comes first, even without another cause.
        let app = flash_confirm_app(ConnectionStatus::Adb, "TB323FU");
        assert!(!app.flash_nav_can());
        assert_eq!(
            app.flash_nav_block_reason(),
            app.flash_model_precheck_error()
        );
        // An uninspected firmware is named before the device.
        let mut app = flash_confirm_app(ConnectionStatus::None, "");
        app.flash.firmware_identity = None;
        assert!(!app.flash_nav_can());
        assert_eq!(
            app.flash_nav_block_reason().as_deref(),
            Some(app.t("start_blocked_firmware_unchecked"))
        );
        let app = flash_confirm_app(ConnectionStatus::Adb, "TB520FU");
        assert!(app.flash_nav_can());
        assert_eq!(app.flash_nav_block_reason(), None);
    }

    #[test]
    fn device_block_reason_detect_arb_names_the_device() {
        let protected = ltbox_core::model::SUPPORTED_MODELS
            .iter()
            .copied()
            .find(|model| crate::model::device::is_rollback_protected_model(model))
            .expect("a rollback-protected model");
        for conn in ALL_CONNECTIONS {
            let mut app = app_on(conn);
            app.device.model = protected.to_string();
            app.adv_wizard.action = Some(AdvAction::DetectArb);
            let can = app.adv_nav_can();
            let reason = app.adv_nav_block_reason();
            if can {
                assert!(reason.is_none());
            }
            if !app.device_reachable() {
                assert!(!can);
                assert_eq!(reason, app.device_block_reason(), "{conn:?}");
            }
        }
    }
}
