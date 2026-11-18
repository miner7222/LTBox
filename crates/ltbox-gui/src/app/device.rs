//! Queries over the connected device and its model capabilities.

use crate::*;

impl App {
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
}
