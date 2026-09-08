//! Last-known device values and which ones were observed in the latest poll.

use crate::{ConnectionStatus, DevicePollResult};
use ltbox_patch::rollback::FastbootRollbackFloors;

#[derive(Debug, Clone, Copy)]
pub(crate) enum SnapshotField {
    Connection,
    Model,
    AndroidVersion,
    Slot,
    Firmware,
    FirmwareFull,
    Arb,
    Ram,
    Storage,
    MarketName,
    Serial,
    FastbootUserspace,
    PlatformSupported,
    RollbackFloors,
}

#[derive(Debug, Default)]
pub(crate) struct DeviceSnapshot {
    pub(crate) connection: ConnectionStatus,
    pub(crate) model: String,
    pub(crate) android_version: String,
    pub(crate) slot: String,
    pub(crate) firmware: String,
    pub(crate) firmware_full: String,
    pub(crate) arb: String,
    pub(crate) ram: String,
    pub(crate) storage: String,
    pub(crate) market_name: String,
    pub(crate) serial: String,
    pub(crate) fastboot_userspace: bool,
    pub(crate) platform_supported: Option<bool>,
    pub(crate) rollback_floors: Option<FastbootRollbackFloors>,
    pub(crate) fresh: [bool; 14],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SnapshotChange {
    pub(crate) reset_identity: bool,
    pub(crate) context_changed: bool,
    pub(crate) left_fastboot: bool,
}

impl DeviceSnapshot {
    /// Merge partial connected polls without mistaking retained values for new
    /// observations. A nonempty changed serial starts a new device identity.
    pub(crate) fn apply(&mut self, poll: DevicePollResult) -> SnapshotChange {
        let previous_connection = self.connection;
        let serial_changed = !poll.serial.is_empty() && poll.serial != self.serial;
        let disconnected = poll.status == ConnectionStatus::None;
        let context_changed = previous_connection != poll.status
            || if disconnected {
                !self.serial.is_empty()
            } else {
                serial_changed
            };
        let change = SnapshotChange {
            reset_identity: serial_changed || disconnected,
            context_changed,
            left_fastboot: previous_connection == ConnectionStatus::Fastboot
                && poll.status != ConnectionStatus::Fastboot,
        };
        if change.reset_identity {
            *self = Self::default();
        }
        self.fresh.fill(false);
        self.connection = poll.status;
        self.fresh[SnapshotField::Connection as usize] = true;
        if disconnected {
            return change;
        }

        let fields = [
            (SnapshotField::Model, &mut self.model, poll.model),
            (SnapshotField::Slot, &mut self.slot, poll.slot),
            (SnapshotField::Firmware, &mut self.firmware, poll.firmware),
            (
                SnapshotField::FirmwareFull,
                &mut self.firmware_full,
                poll.firmware_full,
            ),
            (SnapshotField::Arb, &mut self.arb, poll.arb),
            (SnapshotField::Ram, &mut self.ram, poll.ram),
            (SnapshotField::Storage, &mut self.storage, poll.storage),
            (
                SnapshotField::MarketName,
                &mut self.market_name,
                poll.market_name,
            ),
            (SnapshotField::Serial, &mut self.serial, poll.serial),
        ];
        for (field, current, observed) in fields {
            if !observed.is_empty() {
                *current = observed;
                self.fresh[field as usize] = true;
            }
        }
        if matches!(
            self.connection,
            ConnectionStatus::Adb | ConnectionStatus::AdbRecovery
        ) {
            self.android_version = poll.android_version;
            self.fresh[SnapshotField::AndroidVersion as usize] = true;
        } else {
            self.android_version.clear();
        }
        self.fastboot_userspace = poll.fastboot_userspace;
        self.fresh[SnapshotField::FastbootUserspace as usize] = true;
        self.platform_supported = poll.platform_supported;
        self.fresh[SnapshotField::PlatformSupported as usize] = poll.platform_supported.is_some();
        if poll.rollback_floors.is_some() {
            self.rollback_floors = poll.rollback_floors;
            self.fresh[SnapshotField::RollbackFloors as usize] = true;
        } else if self.connection != ConnectionStatus::Fastboot {
            self.rollback_floors = None;
        }
        change
    }

    /// Whether the latest poll supplied this value, rather than retaining it
    /// or clearing it because its device/transport disappeared.
    pub(crate) fn is_fresh(&self, field: SnapshotField) -> bool {
        self.fresh[field as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_poll(serial: &str) -> DevicePollResult {
        DevicePollResult {
            status: ConnectionStatus::Fastboot,
            serial: serial.into(),
            model: "model".into(),
            slot: "_a".into(),
            firmware: "firmware".into(),
            firmware_full: "full-firmware".into(),
            arb: "arb".into(),
            ram: "ram".into(),
            storage: "storage".into(),
            market_name: "market".into(),
            platform_supported: Some(true),
            rollback_floors: Some(FastbootRollbackFloors {
                vbmeta_system_location: 2,
                vbmeta_system_index: 0x100,
                boot_location: 3,
                boot_index: 0x200,
            }),
            ..DevicePollResult::default()
        }
    }

    fn assert_details_empty(snapshot: &DeviceSnapshot) {
        assert!(snapshot.model.is_empty());
        assert!(snapshot.android_version.is_empty());
        assert!(snapshot.slot.is_empty());
        assert!(snapshot.firmware.is_empty());
        assert!(snapshot.firmware_full.is_empty());
        assert!(snapshot.arb.is_empty());
        assert!(snapshot.ram.is_empty());
        assert!(snapshot.storage.is_empty());
        assert!(snapshot.market_name.is_empty());
        assert!(snapshot.rollback_floors.is_none());
        assert!(snapshot.platform_supported.is_none());
    }

    #[test]
    fn changed_serial_clears_previous_details_even_from_unknown_identity() {
        for previous_serial in ["A", ""] {
            let mut snapshot = DeviceSnapshot::default();
            snapshot.apply(full_poll(previous_serial));
            let change = snapshot.apply(DevicePollResult {
                status: ConnectionStatus::Fastboot,
                serial: "B".into(),
                ..DevicePollResult::default()
            });
            assert!(change.reset_identity);
            assert!(change.context_changed);
            assert!(!change.left_fastboot);
            assert_eq!(snapshot.serial, "B");
            assert_details_empty(&snapshot);
            assert!(snapshot.is_fresh(SnapshotField::Serial));
            assert!(!snapshot.is_fresh(SnapshotField::Model));
        }
    }

    #[test]
    fn blank_fastboot_poll_retains_values_but_marks_them_stale() {
        let mut snapshot = DeviceSnapshot::default();
        snapshot.apply(full_poll("A"));
        assert!(snapshot.is_fresh(SnapshotField::RollbackFloors));
        let change = snapshot.apply(DevicePollResult {
            status: ConnectionStatus::Fastboot,
            ..DevicePollResult::default()
        });
        assert!(!change.reset_identity);
        assert!(!change.context_changed);
        assert_eq!(snapshot.serial, "A");
        assert_eq!(snapshot.model, "model");
        assert!(snapshot.rollback_floors.is_some());
        assert!(snapshot.platform_supported.is_none());
        assert!(!snapshot.is_fresh(SnapshotField::Model));
        assert!(!snapshot.is_fresh(SnapshotField::Serial));
        assert!(!snapshot.is_fresh(SnapshotField::RollbackFloors));
        assert!(snapshot.is_fresh(SnapshotField::Connection));
    }

    #[test]
    fn edl_transition_retains_identity_but_clears_floors() {
        let mut snapshot = DeviceSnapshot::default();
        snapshot.apply(full_poll("A"));
        let change = snapshot.apply(DevicePollResult {
            status: ConnectionStatus::Edl,
            ..DevicePollResult::default()
        });
        assert!(!change.reset_identity);
        assert!(change.context_changed);
        assert!(change.left_fastboot);
        assert_eq!(snapshot.serial, "A");
        assert_eq!(snapshot.model, "model");
        assert!(snapshot.rollback_floors.is_none());
        assert!(!snapshot.is_fresh(SnapshotField::Serial));
        assert!(!snapshot.is_fresh(SnapshotField::RollbackFloors));
    }

    #[test]
    fn android_version_is_adb_only_and_clears_when_transport_changes() {
        let mut snapshot = DeviceSnapshot::default();
        snapshot.apply(DevicePollResult {
            status: ConnectionStatus::Adb,
            model: "TB520FU".into(),
            android_version: "15".into(),
            ..DevicePollResult::default()
        });
        assert_eq!(snapshot.android_version, "15");
        assert!(snapshot.is_fresh(SnapshotField::AndroidVersion));

        snapshot.apply(DevicePollResult {
            status: ConnectionStatus::Fastboot,
            model: "TB520FU".into(),
            ..DevicePollResult::default()
        });
        assert!(snapshot.android_version.is_empty());
        assert!(!snapshot.is_fresh(SnapshotField::AndroidVersion));
    }

    #[test]
    fn disconnect_clears_snapshot_and_observations() {
        let mut snapshot = DeviceSnapshot::default();
        snapshot.apply(full_poll("A"));
        let change = snapshot.apply(DevicePollResult::default());
        assert!(change.reset_identity);
        assert!(change.context_changed);
        assert!(change.left_fastboot);
        assert_eq!(snapshot.connection, ConnectionStatus::None);
        assert!(snapshot.serial.is_empty());
        assert!(!snapshot.fastboot_userspace);
        assert_details_empty(&snapshot);
        assert!(snapshot.is_fresh(SnapshotField::Connection));
        assert!(!snapshot.is_fresh(SnapshotField::Serial));
        assert!(!snapshot.is_fresh(SnapshotField::RollbackFloors));
    }
}
