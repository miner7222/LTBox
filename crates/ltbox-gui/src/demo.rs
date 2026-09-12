//! Launch-time screenshot scenes for the opt-in `demo` feature.
//!
//! `LTBOX_DEMO` accepts dashboard scenes `dashboard`, `drivers-missing`,
//! `adb-conflict`, `software-fix`, and `dual-usb-advisory`, plus wizard scenes `<flow>:<step>`
//! where `flow` is `same` or `other` and `step` is `region`, `target`, `data`,
//! `country`, `folder`, `confirm`, or `flash`. It also accepts first-screen view scenes
//! `view:root`, `view:unroot`, `view:sysupdate`, `view:konabess`, `view:konabess-table`,
//! `view:reboot`, `view:reboot-wait`, `view:advanced`, `view:settings`, and
//! `view:about`, plus the
//! static inspection scenes enumerated in [`VALID_SCENES`], including the
//! Advanced partition-table scenes `view:flash-parts` and `view:dump-parts`.

use crate::*;

const FIRMWARE_FOLDER: &str =
    "/Users/ltbox/Firmware/TB520FU_ROW_OPEN_USER_Q00002.0_W_ZUI_17.5.10.096_ST_251127";

const LOADER_PATH: &str = "/Users/ltbox/Firmware/prog_firehose_ddr.elf";
const DUMP_OUTPUT_DIR: &str = "/Users/ltbox/Dumps/TB520FU_2026-09-08";

pub(crate) const VALID_SCENES: &[&str] = &[
    "dashboard",
    "drivers-missing",
    "adb-conflict",
    "software-fix",
    "dual-usb-advisory",
    "same:region",
    "same:target",
    "same:data",
    "same:country",
    "same:folder",
    "same:confirm",
    "same:flash",
    "other:region",
    "other:target",
    "other:data",
    "other:country",
    "other:folder",
    "other:confirm",
    "other:flash",
    "view:root",
    "view:unroot",
    "view:sysupdate",
    "view:konabess",
    "view:konabess-table",
    "view:reboot",
    "view:reboot-wait",
    "view:advanced",
    "view:settings",
    "view:about",
    "view:root-mode",
    "view:root-skroot-flavor",
    "view:root-provider",
    "view:root-version",
    "view:root-nightly-source",
    "view:root-run-id",
    "view:root-kernel-version",
    "view:root-superkey",
    "view:advanced-region-target",
    "view:sysupdate-rescue-loader",
    "view:sysupdate-rescue-region",
    "view:sysupdate-rescue-confirm",
    "view:flash-parts",
    "view:dump-parts",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scene {
    Dashboard,
    DriversMissing,
    AdbConflict,
    SoftwareFix,
    DualUsbAdvisory,
    Wizard { flow: Flow, step: WizardStep },
    View(View),
    KonaBessTable,
    RebootWait,
    Root(RootScene),
    AdvancedRegionTarget,
    SysUpdateRescue(SysUpdateRescueScene),
    PartitionTable(PartitionTableScene),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PartitionTableScene {
    Flash,
    Dump,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RootScene {
    Mode,
    SkrootFlavor,
    Provider,
    Version,
    NightlySource,
    RunIdPopup,
    KernelVersionPopup,
    SuperkeyPopup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SysUpdateRescueScene {
    Loader,
    RegionPopup,
    Confirm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Flow {
    Same,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WizardStep {
    Region,
    Target,
    Data,
    Country,
    Folder,
    Confirm,
    Flash,
}

impl Scene {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "dashboard" => Some(Self::Dashboard),
            "drivers-missing" => Some(Self::DriversMissing),
            "adb-conflict" => Some(Self::AdbConflict),
            "software-fix" => Some(Self::SoftwareFix),
            "dual-usb-advisory" => Some(Self::DualUsbAdvisory),
            "view:root" => Some(Self::View(View::Root)),
            "view:unroot" => Some(Self::View(View::Unroot)),
            "view:sysupdate" => Some(Self::View(View::SystemUpdate)),
            "view:konabess" => Some(Self::View(View::KonaBess)),
            "view:konabess-table" => Some(Self::KonaBessTable),
            "view:reboot" => Some(Self::View(View::Reboot)),
            "view:reboot-wait" => Some(Self::RebootWait),
            "view:advanced" => Some(Self::View(View::Advanced)),
            "view:settings" => Some(Self::View(View::Settings)),
            "view:about" => Some(Self::View(View::About)),
            "view:root-mode" => Some(Self::Root(RootScene::Mode)),
            "view:root-skroot-flavor" => Some(Self::Root(RootScene::SkrootFlavor)),
            "view:root-provider" => Some(Self::Root(RootScene::Provider)),
            "view:root-version" => Some(Self::Root(RootScene::Version)),
            "view:root-nightly-source" => Some(Self::Root(RootScene::NightlySource)),
            "view:root-run-id" => Some(Self::Root(RootScene::RunIdPopup)),
            "view:root-kernel-version" => Some(Self::Root(RootScene::KernelVersionPopup)),
            "view:root-superkey" => Some(Self::Root(RootScene::SuperkeyPopup)),
            "view:advanced-region-target" => Some(Self::AdvancedRegionTarget),
            "view:sysupdate-rescue-loader" => {
                Some(Self::SysUpdateRescue(SysUpdateRescueScene::Loader))
            }
            "view:sysupdate-rescue-region" => {
                Some(Self::SysUpdateRescue(SysUpdateRescueScene::RegionPopup))
            }
            "view:sysupdate-rescue-confirm" => {
                Some(Self::SysUpdateRescue(SysUpdateRescueScene::Confirm))
            }
            "view:flash-parts" => Some(Self::PartitionTable(PartitionTableScene::Flash)),
            "view:dump-parts" => Some(Self::PartitionTable(PartitionTableScene::Dump)),
            _ => {
                let (flow, step) = value.split_once(':')?;
                let flow = match flow {
                    "same" => Flow::Same,
                    "other" => Flow::Other,
                    _ => return None,
                };
                let step = match step {
                    "region" => WizardStep::Region,
                    "target" => WizardStep::Target,
                    "data" => WizardStep::Data,
                    "country" => WizardStep::Country,
                    "folder" => WizardStep::Folder,
                    "confirm" => WizardStep::Confirm,
                    "flash" => WizardStep::Flash,
                    _ => return None,
                };
                Some(Self::Wizard { flow, step })
            }
        }
    }

    fn poll_result(self) -> DevicePollResult {
        match self {
            Self::SoftwareFix | Self::DriversMissing => return DevicePollResult::default(),
            Self::AdbConflict => {
                return DevicePollResult {
                    status: ConnectionStatus::AdbServerBlocking,
                    ..DevicePollResult::default()
                };
            }
            Self::DualUsbAdvisory => {
                return DevicePollResult {
                    status: ConnectionStatus::Adb,
                    model: "TB323FU".to_string(),
                    android_version: "15".to_string(),
                    slot: "_a".to_string(),
                    firmware: "ZUXOS_1.0.0".to_string(),
                    firmware_full: "ZUXOS_1.0.0".to_string(),
                    arb: "arb_yes".to_string(),
                    ram: "12 GB".to_string(),
                    storage: "256 GB".to_string(),
                    market_name: "Legion Y700".to_string(),
                    platform_supported: Some(true),
                    ..DevicePollResult::default()
                };
            }
            Self::Dashboard
            | Self::Wizard { .. }
            | Self::View(_)
            | Self::KonaBessTable
            | Self::RebootWait
            | Self::Root(_)
            | Self::AdvancedRegionTarget
            | Self::SysUpdateRescue(_)
            | Self::PartitionTable(_) => {}
        }

        DevicePollResult {
            status: ConnectionStatus::Adb,
            model: "TB520FU".to_string(),
            android_version: "15".to_string(),
            slot: "_a".to_string(),
            firmware: "ZUXOS_1.5.10.186_ST_260408".to_string(),
            firmware_full: "ZUXOS_1.5.10.186_ST_260408".to_string(),
            arb: "arb_yes".to_string(),
            ram: "12 GB".to_string(),
            storage: "256 GB".to_string(),
            market_name: "YOGA Pad Pro".to_string(),
            platform_supported: Some(true),
            ..DevicePollResult::default()
        }
    }
}

pub(crate) fn initialize(app: &mut App) {
    let Some(value) = std::env::var_os("LTBOX_DEMO") else {
        return;
    };
    let value = value.to_string_lossy();
    let Some(scene) = Scene::parse(&value) else {
        tracing::warn!(
            "unrecognized LTBOX_DEMO={value:?}; valid scenes: {}",
            VALID_SCENES.join(", ")
        );
        return;
    };

    app.demo_scene = Some(scene);
    app.driver_status = Some(match scene {
        Scene::DriversMissing => ltbox_device::driver::DriverStatus::Missing(vec!["qcserlib.inf"]),
        _ => ltbox_device::driver::DriverStatus::Present,
    });
    app.online = Some(true);
    app.software_fix.running = scene == Scene::SoftwareFix;
    // Screenshot scenes should open on the screen they name, not behind the
    // launch disclaimer.
    app.startup_disclaimer_open = false;

    if scene == Scene::DualUsbAdvisory {
        apply_dual_usb_advisory_scene(app);
        return;
    }

    drop(app.update(Message::DevicePolled(scene.poll_result())));

    match scene {
        Scene::Wizard { flow, step } => apply_wizard_scene(app, flow, step),
        Scene::View(view) => app.current_view = view,
        Scene::KonaBessTable => apply_konabess_table_scene(app),
        Scene::RebootWait => apply_reboot_wait_scene(app),
        Scene::Root(root_scene) => apply_root_scene(app, root_scene),
        Scene::AdvancedRegionTarget => apply_advanced_region_target_scene(app),
        Scene::SysUpdateRescue(rescue_scene) => apply_sysupdate_rescue_scene(app, rescue_scene),
        Scene::PartitionTable(table) => apply_partition_table_scene(app, table),
        Scene::SoftwareFix
        | Scene::DualUsbAdvisory
        | Scene::Dashboard
        | Scene::DriversMissing
        | Scene::AdbConflict => {}
    }
}

fn apply_reboot_wait_scene(app: &mut App) {
    app.current_view = View::Reboot;
    app.operation
        .start(Some(View::Reboot), OperationKind::Unphased, None);
    app.reboot_wait_transition = true;
    app.reboot_wait_dialog_open = true;
}

fn apply_dual_usb_advisory_scene(app: &mut App) {
    app.current_view = View::Dashboard;
    app.startup_disclaimer_open = false;
    app.dual_usb_advisory_dismissed.clear();
    app.dual_usb_advisory_closed.clear();
    drop(app.update(Message::DevicePolled(Scene::DualUsbAdvisory.poll_result())));
}

fn apply_konabess_table_scene(app: &mut App) {
    use ltbox_patch::konabess::{GpuGroup, GpuLevel, GpuProperty, GpuTable, VendorBootDtbInfo};

    fn property(name: &str, value: u32) -> GpuProperty {
        GpuProperty {
            name: name.to_string(),
            cells: vec![value],
        }
    }

    fn level(id: u32, frequency_mhz: u32, voltage: u32, bus: [u32; 3]) -> GpuLevel {
        GpuLevel {
            id,
            properties: vec![
                property("reg", id),
                property("qcom,gpu-freq", frequency_mhz * 1_000_000),
                property("qcom,level", voltage),
                property("qcom,bus-freq", bus[0]),
                property("qcom,bus-min", bus[1]),
                property("qcom,bus-max", bus[2]),
            ],
        }
    }

    fn group(id: u32, initial: u32, floor: u32, levels: Vec<GpuLevel>) -> GpuGroup {
        GpuGroup {
            id,
            header_properties: vec![
                property("qcom,speed-bin", id),
                property("qcom,initial-pwrlevel", initial),
                property("qcom,initial-min-pwrlevel", floor),
            ],
            levels,
        }
    }

    let stock = GpuTable {
        groups: vec![
            group(
                0,
                2,
                5,
                vec![
                    level(0, 1200, 452, [12, 10, 12]),
                    level(1, 1100, 448, [12, 10, 12]),
                    level(2, 1000, 432, [11, 9, 12]),
                    level(3, 900, 416, [10, 8, 11]),
                    level(4, 800, 384, [8, 7, 10]),
                    level(5, 700, 320, [6, 5, 8]),
                ],
            ),
            group(
                1,
                1,
                3,
                vec![
                    level(0, 1150, 448, [12, 10, 12]),
                    level(1, 1050, 432, [11, 9, 12]),
                    level(2, 950, 416, [10, 8, 11]),
                    level(3, 850, 384, [8, 7, 10]),
                ],
            ),
        ],
    };
    let edited = GpuTable {
        groups: vec![
            group(
                0,
                2,
                5,
                vec![
                    level(0, 1200, 448, [12, 10, 12]),
                    level(1, 1100, 448, [12, 10, 12]),
                    level(2, 1050, 452, [11, 9, 12]),
                    level(3, 900, 416, [10, 8, 11]),
                    level(4, 800, 384, [8, 7, 10]),
                    level(5, 700, 320, [6, 5, 8]),
                ],
            ),
            group(
                1,
                1,
                2,
                vec![
                    level(0, 1150, 448, [12, 10, 12]),
                    level(1, 1050, 432, [11, 9, 12]),
                    level(2, 950, 416, [10, 8, 11]),
                ],
            ),
        ],
    };
    let target = VendorBootDtbInfo {
        index: 3,
        model: Some("Qualcomm Technologies, Inc. Sun GPU".to_string()),
        chip: Some("sun".to_string()),
        gpu_shape: None,
        table: Some(stock.clone()),
    };

    app.current_view = View::KonaBess;
    app.konabess = KonaBessWizard {
        step: 1,
        loader_path: Some(LOADER_PATH.to_string()),
        stock_table: Some(stock),
        edited_table: Some(edited),
        edited_dirty: true,
        candidates: vec![target],
        selected_target_index: Some(3),
        probable_target_index: Some(3),
        prepared: Some(KonaBessPrepared {
            work_dir: "demo/konabess".into(),
            vendor_boot: "demo/vendor_boot.img".into(),
            vbmeta: "demo/vbmeta.img".into(),
            backup_dir: "demo/backup".into(),
            slot_suffix: "_a".to_string(),
            probable_dtb_index: Some(3),
        }),
        ..KonaBessWizard::default()
    };
}

fn apply_root_scene(app: &mut App, scene: RootScene) {
    app.current_view = View::Root;
    app.root = match scene {
        RootScene::Mode => RootWizard {
            step: 1,
            family: Some(Family::KernelSU),
            mode: Some(RootMode::Lkm),
            ..RootWizard::default()
        },
        RootScene::SkrootFlavor => RootWizard {
            step: 1,
            family: Some(Family::Skroot),
            skroot_flavor: Some(SkrootFlavor::Lite),
            ..RootWizard::default()
        },
        RootScene::Provider => RootWizard {
            step: 2,
            family: Some(Family::Magisk),
            provider: Some(Provider::Magisk),
            ..RootWizard::default()
        },
        RootScene::Version => RootWizard {
            step: 3,
            family: Some(Family::Magisk),
            provider: Some(Provider::Magisk),
            version: Some(VerChoice::Stable),
            ..RootWizard::default()
        },
        RootScene::NightlySource | RootScene::RunIdPopup => RootWizard {
            step: 4,
            family: Some(Family::Magisk),
            provider: Some(Provider::Magisk),
            version: Some(VerChoice::Nightly),
            nightly_source: Some(match scene {
                RootScene::RunIdPopup => NightlySource::ManualInput,
                _ => NightlySource::AutoDetect,
            }),
            run_id_popup_open: scene == RootScene::RunIdPopup,
            ..RootWizard::default()
        },
        RootScene::KernelVersionPopup => RootWizard {
            step: 6,
            family: Some(Family::KernelSU),
            mode: Some(RootMode::Lkm),
            provider: Some(Provider::KernelSU),
            version: Some(VerChoice::Stable),
            folder_path: Some(FIRMWARE_FOLDER.to_string()),
            kernel_version_popup_open: true,
            ..RootWizard::default()
        },
        RootScene::SuperkeyPopup => RootWizard {
            step: 8,
            family: Some(Family::APatch),
            provider: Some(Provider::APatch),
            version: Some(VerChoice::Stable),
            superkey_popup_open: true,
            ..RootWizard::default()
        },
    };
}

fn apply_advanced_region_target_scene(app: &mut App) {
    app.current_view = View::Advanced;
    app.adv_wizard = AdvWizard {
        action: Some(AdvAction::RegionConvert),
        step: 1,
        file_path: Some(FIRMWARE_FOLDER.to_string()),
        region_target: Some(DeviceRegion::Row),
        ..AdvWizard::default()
    };
    app.region_target_popup_open = true;
}

fn apply_sysupdate_rescue_scene(app: &mut App, scene: SysUpdateRescueScene) {
    app.current_view = View::SystemUpdate;
    app.sysupdate = SysUpdateWizard {
        step: match scene {
            SysUpdateRescueScene::Loader | SysUpdateRescueScene::RegionPopup => 1,
            SysUpdateRescueScene::Confirm => 2,
        },
        action: Some(SysUpdateAction::Rescue),
        rescue_folder: (scene != SysUpdateRescueScene::Loader).then(|| FIRMWARE_FOLDER.to_string()),
        rescue_region: (scene != SysUpdateRescueScene::Loader).then_some(RescueRegion::Prc),
        rescue_region_popup_open: scene == SysUpdateRescueScene::RegionPopup,
        rescue_region_confirmed: scene == SysUpdateRescueScene::Confirm,
    };
}

/// A representative UFS partition layout, so the table steps render with the
/// row count and label lengths a real device produces.
const DEMO_PARTITIONS: &[(u8, &str, u64, u64)] = &[
    (0, "xbl_a", 6144, 8192),
    (0, "xbl_config_a", 14336, 256),
    (0, "aop_a", 14592, 4096),
    (0, "tz_a", 18688, 8192),
    (0, "hyp_a", 26880, 4096),
    (0, "abl_a", 30976, 1024),
    (0, "keymaster_a", 32000, 512),
    (0, "cmnlib_a", 32512, 1024),
    (0, "devcfg_a", 33536, 256),
    (0, "featenabler_a", 33792, 256),
    (1, "boot_a", 0, 262144),
    (1, "init_boot_a", 262144, 16384),
    (1, "vendor_boot_a", 278528, 131072),
    (1, "dtbo_a", 409600, 49152),
    (1, "vbmeta_a", 458752, 128),
    (1, "vbmeta_system_a", 458880, 128),
    (2, "super", 0, 16777216),
    (2, "userdata", 16777216, 471859200),
    (3, "persist", 0, 65536),
    (3, "modemst1", 65536, 4096),
    (3, "modemst2", 69632, 4096),
    (3, "fsg", 73728, 4096),
    (4, "metadata", 0, 32768),
    (4, "misc", 32768, 2048),
];

const SECTOR: u64 = 4096;

fn apply_partition_table_scene(app: &mut App, table: PartitionTableScene) {
    app.current_view = View::Advanced;
    match table {
        PartitionTableScene::Flash => {
            app.advanced_wizard_open = AdvancedWizardOpen::FlashParts;
            app.flash_parts = FlashPartsWizard {
                step: 1,
                loader_path: Some(LOADER_PATH.to_string()),
                entry_connection: Some(ConnectionStatus::Edl),
                rows: DEMO_PARTITIONS
                    .iter()
                    .enumerate()
                    .map(|(i, &(lun, label, start, sectors))| FlashPartRow {
                        lun,
                        label: label.to_string(),
                        start_sector: start,
                        num_sectors: sectors,
                        size_bytes: sectors * SECTOR,
                        // Two picked images and one erase, so the table shows
                        // every row state at once.
                        file_path: match i {
                            10 => Some(format!("{FIRMWARE_FOLDER}/boot.img")),
                            12 => Some(format!("{FIRMWARE_FOLDER}/vendor_boot.img")),
                            _ => None,
                        },
                        state: match i {
                            10 | 12 => FlashRowState::Write,
                            23 => FlashRowState::Erase,
                            _ => FlashRowState::Skip,
                        },
                    })
                    .collect(),
                ..FlashPartsWizard::default()
            };
        }
        PartitionTableScene::Dump => {
            app.advanced_wizard_open = AdvancedWizardOpen::DumpParts;
            app.dump_parts = DumpPartsWizard {
                step: 1,
                loader_path: Some(LOADER_PATH.to_string()),
                entry_connection: Some(ConnectionStatus::Edl),
                output_dir: Some(DUMP_OUTPUT_DIR.to_string()),
                rows: DEMO_PARTITIONS
                    .iter()
                    .enumerate()
                    .map(|(i, &(lun, label, start, sectors))| DumpPartRow {
                        lun,
                        label: label.to_string(),
                        start_sector: start,
                        num_sectors: sectors,
                        size_bytes: sectors * SECTOR,
                        selected: matches!(i, 10 | 11 | 18),
                    })
                    .collect(),
                ..DumpPartsWizard::default()
            };
        }
    }
}

fn apply_wizard_scene(app: &mut App, flow: Flow, step: WizardStep) {
    let (target, data_mode, modify_region, modify_rollback, wipe) = match flow {
        Flow::Same => (
            FlashTarget::SameRegion,
            DataMode::Keep,
            false,
            RollbackSetting::Auto,
            false,
        ),
        Flow::Other => (
            FlashTarget::OtherRegion,
            DataMode::Wipe,
            true,
            RollbackSetting::On,
            true,
        ),
    };
    let config = WorkflowConfig {
        modify_region,
        device_region: Some(DeviceRegion::Prc),
        modify_rollback,
        manual_rollback_indices: None,
        wipe,
        country_action: CountryAction::Set("US".to_string()),
    };

    app.current_view = View::Flash;
    app.flash = FlashWizard {
        step: match step {
            WizardStep::Region => 0,
            WizardStep::Target => 1,
            WizardStep::Data => 2,
            WizardStep::Country | WizardStep::Folder => 3,
            WizardStep::Confirm => 4,
            WizardStep::Flash => 5,
        },
        region_selection: Some(FlashRegionSelection::Manual(DeviceRegion::Prc)),
        device_region: Some(DeviceRegion::Prc),
        target: Some(target),
        data_mode: Some(data_mode),
        firmware_folder: Some(FIRMWARE_FOLDER.to_string()),
        ..FlashWizard::default()
    };
    app.wf_config = config.clone();
    app.confirm_baseline = Some(config);
    app.country_popup_open = step == WizardStep::Country;

    if step == WizardStep::Flash {
        let reporter = app.begin_phased_op(View::Flash, OperationPhaseKind::Flash);
        let _ = reporter.marker(7);
    }
}

pub(crate) fn is_active(app: &App) -> bool {
    app.demo_scene.is_some()
}

pub(crate) fn prepare_flash_region_on_entry(app: &mut App) -> bool {
    if app.demo_scene.is_none() {
        return false;
    }
    if app.flash.device_region.is_none() {
        app.flash.region_selection = Some(FlashRegionSelection::Manual(DeviceRegion::Row));
        app.flash.device_region = Some(DeviceRegion::Row);
        app.flash.step = 1;
    }
    true
}

pub(crate) fn poll_result(app: &App) -> Option<DevicePollResult> {
    app.demo_scene.map(Scene::poll_result)
}

pub(crate) fn blocks_flash_execution(app: &App) -> bool {
    app.demo_scene.is_some()
}

pub(crate) fn blocks_device_action(app: &App, message: &Message) -> bool {
    app.demo_scene.is_some()
        && matches!(
            message,
            Message::Flash(_)
                | Message::Root(_)
                | Message::Unroot(_)
                | Message::Sys(_)
                | Message::Adv(_)
                | Message::KonaBess(_)
                | Message::FlashParts(_)
                | Message::DumpParts(_)
                | Message::DumpPhys(_)
                | Message::FlashPhys(_)
                | Message::SimpleFlash(_)
                | Message::Reboot(_)
                | Message::ConfirmCloseSoftwareFix
                | Message::KillAdbServer
                | Message::InstallDrivers
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_documented_scene_parses() {
        for scene in VALID_SCENES {
            assert!(Scene::parse(scene).is_some(), "failed to parse {scene}");
        }
        assert!(Scene::parse("same:unknown").is_none());
        assert!(Scene::parse("unknown").is_none());
    }

    #[test]
    fn fabricated_model_uses_the_tb520fu_portrait() {
        let result = Scene::Dashboard.poll_result();
        assert_eq!(result.model, "TB520FU");
        assert!(matches!(
            device_portrait(&result.model),
            DevicePortrait::Png(_)
        ));
    }

    #[test]
    fn dual_usb_advisory_scene_opens_the_tb323fu_guide_from_its_device_poll() {
        let mut app = App {
            dual_usb_advisory_dismissed: vec!["TB323FU".to_string()],
            dual_usb_advisory_closed: vec!["TB323FU".to_string()],
            ..App::default()
        };

        apply_dual_usb_advisory_scene(&mut app);

        assert_eq!(app.device.model, "TB323FU");
        assert_eq!(app.dual_usb_advisory_model(), Some("TB323FU"));
        assert!(app.dual_usb_help_open);
        assert!(!app.startup_disclaimer_open);
    }

    #[test]
    fn active_demo_scene_enters_flash_without_prompt_or_region_probe() {
        let scene = Scene::Dashboard;
        let mut app = App {
            demo_scene: Some(scene),
            ..App::default()
        };
        drop(app.update(Message::DevicePolled(scene.poll_result())));

        assert!(app.device.serial.is_empty());
        drop(app.prepare_flash_region_on_entry());

        assert_eq!(app.flash.device_region, Some(DeviceRegion::Row));
        assert_eq!(
            app.flash.region_selection,
            Some(FlashRegionSelection::Manual(DeviceRegion::Row))
        );
        assert_eq!(app.flash.step, 1);
        assert!(app.flash_serial_prompt.is_none());
        assert!(app.queries.region_pending.is_none());

        app.flash.region_selection = Some(FlashRegionSelection::Manual(DeviceRegion::Prc));
        app.flash.device_region = Some(DeviceRegion::Prc);
        app.flash.step = 4;
        drop(app.prepare_flash_region_on_entry());
        assert_eq!(app.flash.device_region, Some(DeviceRegion::Prc));
        assert_eq!(
            app.flash.region_selection,
            Some(FlashRegionSelection::Manual(DeviceRegion::Prc))
        );
        assert_eq!(app.flash.step, 4);
    }

    #[test]
    fn view_scenes_open_their_first_screen_with_populated_identity() {
        let expected_views = [
            (Scene::parse("view:root").unwrap(), View::Root),
            (Scene::parse("view:unroot").unwrap(), View::Unroot),
            (Scene::parse("view:sysupdate").unwrap(), View::SystemUpdate),
            (Scene::parse("view:konabess").unwrap(), View::KonaBess),
            (Scene::parse("view:reboot").unwrap(), View::Reboot),
            (Scene::parse("view:advanced").unwrap(), View::Advanced),
            (Scene::parse("view:settings").unwrap(), View::Settings),
            (Scene::parse("view:about").unwrap(), View::About),
        ];

        for (scene, expected_view) in expected_views {
            let mut app = App {
                demo_scene: Some(scene),
                ..App::default()
            };
            drop(app.update(Message::DevicePolled(scene.poll_result())));
            if let Scene::View(view) = scene {
                app.current_view = view;
            }

            assert_eq!(app.current_view, expected_view);
            assert_eq!(app.device.connection, ConnectionStatus::Adb);
            assert_eq!(app.device.model, "TB520FU");
            assert_eq!(app.device.android_version, "15");
            assert_eq!(app.device.market_name, "YOGA Pad Pro");
            assert_eq!(app.device.ram, "12 GB");
            assert_eq!(app.device.storage, "256 GB");
            assert_eq!(app.device.slot, "_a");
            assert_eq!(app.device.arb, "arb_yes");
            assert_eq!(app.device.firmware, "ZUXOS_1.5.10.186_ST_260408");
            assert_eq!(app.device.firmware_full, "ZUXOS_1.5.10.186_ST_260408");

            match expected_view {
                View::Root => assert_eq!(app.root.step, 0),
                View::Unroot => assert_eq!(app.unroot.step, 0),
                View::SystemUpdate => assert_eq!(app.sysupdate.step, 0),
                View::KonaBess => {
                    assert_eq!(app.konabess.step, 0);
                    assert!(app.konabess.loader_path.is_none());
                    assert!(app.konabess.prepared.is_none());
                }
                View::Advanced => {
                    assert_eq!(app.advanced_wizard_open, AdvancedWizardOpen::None);
                    assert!(app.adv_wizard.action.is_none());
                }
                View::Reboot => assert!(app.reboot_confirm_target.is_none()),
                View::Settings | View::About => {}
                View::Dashboard | View::Flash => unreachable!(),
            }
        }
    }

    #[test]
    fn konabess_table_scene_opens_a_populated_comparison() {
        let scene = Scene::parse("view:konabess-table").unwrap();
        let mut app = App {
            demo_scene: Some(scene),
            ..App::default()
        };
        drop(app.update(Message::DevicePolled(scene.poll_result())));
        apply_konabess_table_scene(&mut app);

        assert_eq!(app.current_view, View::KonaBess);
        assert_eq!(app.konabess.step, 1);
        assert_eq!(app.konabess.selected_chip(), Some("sun"));
        assert_eq!(app.konabess.stock_table.as_ref().unwrap().groups.len(), 2);
        assert_eq!(app.konabess.edited_table.as_ref().unwrap().groups.len(), 2);
        assert_eq!(
            app.konabess.stock_table.as_ref().unwrap().groups[1]
                .levels
                .len(),
            4
        );
        assert_eq!(
            app.konabess.edited_table.as_ref().unwrap().groups[1]
                .levels
                .len(),
            3
        );
        assert!(app.konabess.edited_dirty);
        assert!(app.konabess.can_next());
    }

    #[test]
    fn static_inspection_scenes_assign_the_required_view_model_state() {
        let cases = [
            ("view:root-mode", View::Root),
            ("view:root-skroot-flavor", View::Root),
            ("view:root-provider", View::Root),
            ("view:root-version", View::Root),
            ("view:root-nightly-source", View::Root),
            ("view:root-run-id", View::Root),
            ("view:root-kernel-version", View::Root),
            ("view:root-superkey", View::Root),
            ("view:advanced-region-target", View::Advanced),
            ("view:sysupdate-rescue-loader", View::SystemUpdate),
            ("view:sysupdate-rescue-region", View::SystemUpdate),
            ("view:sysupdate-rescue-confirm", View::SystemUpdate),
        ];

        for (value, expected_view) in cases {
            let scene = Scene::parse(value).unwrap();
            let mut app = App::default();
            match scene {
                Scene::Root(root_scene) => apply_root_scene(&mut app, root_scene),
                Scene::AdvancedRegionTarget => apply_advanced_region_target_scene(&mut app),
                Scene::SysUpdateRescue(rescue_scene) => {
                    apply_sysupdate_rescue_scene(&mut app, rescue_scene)
                }
                _ => unreachable!("unexpected scene {value}"),
            }
            assert_eq!(app.current_view, expected_view, "scene {value}");

            match scene {
                Scene::Root(RootScene::Mode) => {
                    assert_eq!(app.root.step, 1);
                    assert_eq!(app.root.family, Some(Family::KernelSU));
                    assert_eq!(app.root.mode, Some(RootMode::Lkm));
                }
                Scene::Root(RootScene::SkrootFlavor) => {
                    assert_eq!(app.root.step, 1);
                    assert_eq!(app.root.family, Some(Family::Skroot));
                    assert_eq!(app.root.skroot_flavor, Some(SkrootFlavor::Lite));
                }
                Scene::Root(RootScene::Provider) => {
                    assert_eq!(app.root.step, 2);
                    assert_eq!(app.root.family, Some(Family::Magisk));
                    assert_eq!(app.root.provider, Some(Provider::Magisk));
                }
                Scene::Root(RootScene::Version) => {
                    assert_eq!(app.root.step, 3);
                    assert_eq!(app.root.version, Some(VerChoice::Stable));
                }
                Scene::Root(RootScene::NightlySource) => {
                    assert_eq!(app.root.step, 4);
                    assert_eq!(app.root.version, Some(VerChoice::Nightly));
                    assert_eq!(app.root.nightly_source, Some(NightlySource::AutoDetect));
                }
                Scene::Root(RootScene::RunIdPopup) => {
                    assert!(app.root.run_id_popup_open);
                    assert_eq!(app.root.nightly_source, Some(NightlySource::ManualInput));
                }
                Scene::Root(RootScene::KernelVersionPopup) => {
                    assert_eq!(app.root.step, 6);
                    assert!(app.root.kernel_version_popup_open);
                    assert_eq!(app.root.folder_path.as_deref(), Some(FIRMWARE_FOLDER));
                }
                Scene::Root(RootScene::SuperkeyPopup) => {
                    assert_eq!(app.root.step, 8);
                    assert_eq!(app.root.family, Some(Family::APatch));
                    assert!(app.root.superkey_popup_open);
                }
                Scene::AdvancedRegionTarget => {
                    assert_eq!(app.adv_wizard.action, Some(AdvAction::RegionConvert));
                    assert_eq!(app.adv_wizard.step, 1);
                    assert_eq!(app.adv_wizard.file_path.as_deref(), Some(FIRMWARE_FOLDER));
                    assert_eq!(app.adv_wizard.region_target, Some(DeviceRegion::Row));
                    assert!(app.region_target_popup_open);
                }
                Scene::SysUpdateRescue(rescue_scene) => {
                    assert_eq!(app.sysupdate.action, Some(SysUpdateAction::Rescue));
                    match rescue_scene {
                        SysUpdateRescueScene::Loader => {
                            assert_eq!(app.sysupdate.step, 1);
                            assert!(app.sysupdate.rescue_folder.is_none());
                            assert!(app.sysupdate.rescue_region.is_none());
                            assert!(!app.sysupdate.rescue_region_popup_open);
                        }
                        SysUpdateRescueScene::RegionPopup => {
                            assert_eq!(app.sysupdate.step, 1);
                            assert_eq!(
                                app.sysupdate.rescue_folder.as_deref(),
                                Some(FIRMWARE_FOLDER)
                            );
                            assert_eq!(app.sysupdate.rescue_region, Some(RescueRegion::Prc));
                            assert!(app.sysupdate.rescue_region_popup_open);
                            assert!(!app.sysupdate.rescue_region_confirmed);
                        }
                        SysUpdateRescueScene::Confirm => {
                            assert_eq!(app.sysupdate.step, 2);
                            assert_eq!(
                                app.sysupdate.rescue_folder.as_deref(),
                                Some(FIRMWARE_FOLDER)
                            );
                            assert_eq!(app.sysupdate.rescue_region, Some(RescueRegion::Prc));
                            assert!(app.sysupdate.rescue_region_confirmed);
                        }
                    }
                }
                _ => unreachable!("unexpected scene {value}"),
            }
        }
    }

    #[test]
    fn reboot_wait_scene_uses_the_tracked_reboot_operation() {
        let mut app = App::default();
        apply_reboot_wait_scene(&mut app);

        assert_eq!(app.current_view, View::Reboot);
        assert!(app.operation.is_running());
        assert_eq!(app.operation.view(), Some(View::Reboot));
        assert!(app.reboot_wait_transition);
        assert!(app.reboot_wait_dialog_open);
        assert!(!app.should_show_busy_progress_dialog());
    }

    #[test]
    fn dashboard_identity_is_populated_but_unreadable_scenes_stay_empty() {
        let mut dashboard = App::default();
        drop(dashboard.update(Message::DevicePolled(Scene::Dashboard.poll_result())));
        assert_eq!(dashboard.device.connection, ConnectionStatus::Adb);
        assert_eq!(dashboard.device.model, "TB520FU");
        assert_eq!(dashboard.device.android_version, "15");
        assert_eq!(dashboard.device.market_name, "YOGA Pad Pro");
        assert_eq!(dashboard.device.ram, "12 GB");
        assert_eq!(dashboard.device.storage, "256 GB");
        assert_eq!(dashboard.device.slot, "_a");
        assert_eq!(dashboard.device.arb, "arb_yes");
        assert_eq!(dashboard.device.firmware, "ZUXOS_1.5.10.186_ST_260408");
        assert_eq!(dashboard.device.firmware_full, "ZUXOS_1.5.10.186_ST_260408");

        for (scene, status) in [
            (Scene::DriversMissing, ConnectionStatus::None),
            (Scene::AdbConflict, ConnectionStatus::AdbServerBlocking),
        ] {
            let mut app = App::default();
            for _ in 0..2 {
                drop(app.update(Message::DevicePolled(scene.poll_result())));
                assert_eq!(app.device.connection, status);
                assert!(app.device.model.is_empty());
                assert!(app.device.android_version.is_empty());
                assert!(app.device.market_name.is_empty());
                assert!(app.device.ram.is_empty());
                assert!(app.device.storage.is_empty());
                assert!(app.device.slot.is_empty());
                assert!(app.device.arb.is_empty());
                assert!(app.device.firmware.is_empty());
                assert!(app.device.firmware_full.is_empty());
            }
        }
    }

    #[test]
    fn flash_scene_is_busy_on_the_firmware_write_phase() {
        let mut app = App {
            demo_scene: Some(Scene::Wizard {
                flow: Flow::Other,
                step: WizardStep::Flash,
            }),
            ..App::default()
        };
        apply_wizard_scene(&mut app, Flow::Other, WizardStep::Flash);

        assert_eq!(app.current_view, View::Flash);
        assert_eq!(app.flash.step, 5);
        assert!(app.operation.is_running());
        assert_eq!(app.operation.view(), Some(View::Flash));
        assert_eq!(app.operation.phase_kind(), Some(OperationPhaseKind::Flash));
        assert_eq!(app.operation.current_step(), 6);
        assert_eq!(app.operation.steps.len(), 9);
        assert!(blocks_flash_execution(&app));
    }

    #[test]
    fn active_demo_scene_short_circuits_flash_execution() {
        let mut app = App {
            demo_scene: Some(Scene::Dashboard),
            ..App::default()
        };

        drop(app.update_flash(FlashMsg::FlashExecStart));

        assert!(!app.operation.is_running());
        assert!(app.operation.steps.is_empty());
        assert_eq!(app.operation.phase_kind(), None);
    }

    #[test]
    fn active_demo_scene_blocks_every_device_workflow_family() {
        let app = App {
            demo_scene: Some(Scene::Dashboard),
            ..App::default()
        };
        let blocked = [
            Message::Flash(FlashMsg::FlashBack),
            Message::Root(RootMsg::RootBack),
            Message::Unroot(UnrootMsg::UnrootBack),
            Message::Sys(SysMsg::SysBack),
            Message::Adv(AdvMsg::AdvWizBack),
            Message::KonaBess(KonaBessMsg::KonaBessBack),
            Message::FlashParts(FlashPartsMsg::FlashPartsBack),
            Message::DumpParts(DumpPartsMsg::DumpPartsBack),
            Message::DumpPhys(DumpPhysMsg::DumpPhysBack),
            Message::FlashPhys(FlashPhysMsg::FlashPhysBack),
            Message::SimpleFlash(SimpleFlashMsg::SimpleFlashBack),
            Message::Reboot(RebootMsg::RebootDismiss),
            Message::KillAdbServer,
            Message::InstallDrivers,
        ];

        for message in blocked {
            assert!(blocks_device_action(&app, &message));
        }
        assert!(!blocks_device_action(&app, &Message::PollDevice));
        assert!(!blocks_device_action(
            &app,
            &Message::Navigate(View::Dashboard)
        ));
    }
}
