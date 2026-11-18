//! Device-class classifiers and the live connection state.

use crate::theme::Palette;
/// Whether the model needs the long-edge USB port advisory.
pub(crate) fn is_dual_usbc_model(model: &str) -> bool {
    ltbox_core::model::capabilities(model).dual_usb
}

/// Whether a root run on `model` skips AVB post-processing entirely
/// (`root_uses_gbl`). Such models sign their chain through the testkey
/// efisp/GBL route, so the backed-up images carry no AVB metadata.
///
/// Unroot therefore cannot compare the backup against the device and restores
/// it verbatim. Keying on the capability keeps Root and Unroot on one route.
pub(crate) fn root_skips_avb_postprocess(model: &str) -> bool {
    ltbox_core::model::capabilities(model).root_uses_gbl
}

/// Every supported model enforces AVB rollback protection except the PRC-only
/// TB322FC and TB324ZC. Decides whether a missing fastboot
/// `stored_rollback_index` means "no ARB, skip" (those two) or "ARB present but
/// fastboot can't report it, read it over EDL" (everything else). An unknown
/// model counts as protected: reading the index beats risking a
/// rollback-rejected downgrade.
pub(crate) fn is_rollback_protected_model(model: &str) -> bool {
    ltbox_core::model::capabilities(model)
        .rollback
        .is_protected()
}

/// Apply the rollback policy's restrictions to a requested mode. Shared by the
/// confirm editor and the worker.
pub(crate) fn effective_rollback_mode(
    policy: ltbox_core::model::RollbackPolicy,
    mode: ltbox_patch::rollback::RollbackMode,
) -> ltbox_patch::rollback::RollbackMode {
    use ltbox_core::model::RollbackPolicy;
    use ltbox_patch::rollback::RollbackMode;
    match (policy, mode) {
        (RollbackPolicy::ReadOnly, _) => RollbackMode::Auto,
        // A blind On would bump even matching indices and force the testkey
        // chain with no downgrade in play, so it becomes Auto. Manual carries
        // explicit targets and stays: the worker signs the chain and provisions
        // the `_arb` GBL for it.
        (RollbackPolicy::Gbl, RollbackMode::On) => RollbackMode::Auto,
        _ => mode,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ltbox_core::model::LAVIE_TAB_9QHD1_MODEL;

    #[test]
    fn lavie_tab_9qhd1_inherits_tb320fc_device_behavior() {
        assert_eq!(
            ltbox_core::model::capabilities(LAVIE_TAB_9QHD1_MODEL),
            ltbox_core::model::capabilities(ltbox_core::model::TB320FC_MODEL)
        );
        assert!(is_dual_usbc_model(LAVIE_TAB_9QHD1_MODEL));
        assert!(ltbox_core::model::capabilities("TB323FU").root_uses_gbl);
        assert!(!is_dual_usbc_model("TB330FU"));
    }

    #[test]
    fn prc_only_tb322fc_and_tb324zc_are_exempt_but_tb323fu_is_protected() {
        assert!(!is_rollback_protected_model("TB322FC"));
        assert!(!is_rollback_protected_model("TB324ZC"));
        assert!(is_rollback_protected_model("TB323FU"));
    }

    #[test]
    fn xiaoxin_pro13_models_are_rollback_protected() {
        assert!(is_rollback_protected_model(
            ltbox_core::model::TB376FC_MODEL
        ));
        assert!(is_rollback_protected_model(
            ltbox_core::model::TB390FU_MODEL
        ));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ConnectionStatus {
    #[default]
    None,
    Adb,
    /// ADB inside TWRP (product name starts with `twrp_`). Same transitions as
    /// `Adb`; different label.
    AdbRecovery,
    /// ADB sees the device but USB-debug auth is unaccepted
    /// (`unauthorized` / `authorizing`); shell probes fail.
    AdbUnauthorized,
    /// Recovery sideload: adbd connects without an auth challenge but serves
    /// no `shell:`, so no property can be read. Kept apart from
    /// `AdbUnauthorized` so the user is never told to re-tap "Allow USB
    /// debugging" at a screen with no such prompt.
    AdbSideload,
    /// An external `adb.exe` server (or anything else on `127.0.0.1:5037`)
    /// holds the Android USB interface, so LTBox's libusb claim returns
    /// `LIBUSB_ERROR_BUSY` although the device is authorized. Distinct from
    /// `AdbUnauthorized` so the dashboard can offer "kill server".
    AdbServerBlocking,
    Fastboot,
    Edl,
}
impl ConnectionStatus {
    pub(crate) fn label_key(&self) -> &'static str {
        match self {
            Self::None => "conn_disconnected",
            Self::Adb => "conn_adb",
            Self::AdbRecovery => "conn_adb_recovery",
            Self::AdbUnauthorized => "conn_adb_unauthorized",
            Self::AdbSideload => "conn_adb_sideload",
            Self::AdbServerBlocking => "conn_adb_server_blocking",
            Self::Fastboot => "conn_fastboot",
            Self::Edl => "conn_edl",
        }
    }
    pub(crate) fn color(&self, pal: &Palette) -> iced::Color {
        match self {
            Self::None => pal.on_surface_variant,
            Self::Adb | Self::AdbRecovery => pal.success,
            Self::AdbUnauthorized | Self::AdbServerBlocking | Self::AdbSideload => pal.warning,
            Self::Fastboot => pal.warning,
            Self::Edl => pal.tertiary,
        }
    }
    /// True when exec paths should skip the ADB probe. AdbUnauthorized,
    /// AdbSideload and AdbServerBlocking all count as "no usable ADB" —
    /// shell would fail.
    pub(crate) fn skip_adb(self) -> bool {
        matches!(
            self,
            Self::Fastboot
                | Self::Edl
                | Self::AdbUnauthorized
                | Self::AdbSideload
                | Self::AdbServerBlocking
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EdlEntryAction {
    AlreadyEdl,
    AdbReboot,
    FastbootRebootThenAdb,
    ManualWait,
}
