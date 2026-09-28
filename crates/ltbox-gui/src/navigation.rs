//! Sidebar views, reboot targets and the Advanced action catalogue.

use crate::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum View {
    #[default]
    Dashboard,
    Flash,
    SystemUpdate,
    Debloat,
    Root,
    Unroot,
    KonaBess,
    Reboot,
    Advanced,
    Settings,
    About,
}

impl View {
    pub(crate) fn label_key(&self) -> &'static str {
        match self {
            Self::Dashboard => "nav_dashboard",
            Self::Flash => "nav_flash",
            Self::SystemUpdate => "nav_sysupdate",
            Self::Debloat => "nav_debloat",
            Self::Root => "nav_root",
            Self::Unroot => "nav_unroot",
            Self::KonaBess => "nav_konabess",
            Self::Reboot => "nav_reboot",
            Self::Advanced => "nav_advanced",
            Self::Settings => "nav_settings",
            Self::About => "nav_about",
        }
    }

    pub(crate) fn sidebar_label_key(&self) -> &'static str {
        match self {
            Self::Flash => "nav_flash_sidebar",
            Self::KonaBess => "nav_konabess_sidebar",
            _ => self.label_key(),
        }
    }

    pub(crate) fn nav_icon(&self) -> iced::widget::Text<'static, Theme, iced::Renderer> {
        match self {
            Self::Dashboard => icon::nav_dashboard(),
            Self::Flash => icon::nav_flash(),
            Self::SystemUpdate => icon::nav_system_update(),
            Self::Debloat => icon::nav_debloat(),
            Self::Root => icon::nav_root(),
            Self::Unroot => icon::nav_unroot(),
            Self::KonaBess => icon::nav_konabess(),
            Self::Reboot => icon::nav_reboot(),
            Self::Advanced => icon::nav_advanced(),
            Self::Settings => icon::nav_settings(),
            Self::About => icon::nav_about(),
        }
    }
}

pub(crate) const NAV_MAIN: &[View] = &[
    View::Dashboard,
    View::Flash,
    View::SystemUpdate,
    View::Debloat,
    View::Root,
    View::Unroot,
    View::KonaBess,
    View::Reboot,
];
pub(crate) const NAV_TOOLS: &[View] = &[View::Advanced, View::Settings];

/// One-shot reboot target for the Reboot panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RebootTarget {
    System,
    Recovery,
    Bootloader,
    /// Userspace fastboot. Reached with `reboot fastboot` over ADB, and
    /// with `reboot-fastboot` from the bootloader.
    Fastbootd,
    Edl,
}
impl RebootTarget {
    pub(crate) fn label_key(&self) -> &'static str {
        match self {
            Self::System => "reboot_system",
            Self::Recovery => "reboot_recovery",
            Self::Bootloader => "reboot_bootloader",
            Self::Fastbootd => "reboot_fastbootd",
            Self::Edl => "reboot_edl",
        }
    }
    /// Short-name key used inside the confirm popup so "Reboot to
    /// {Reboot to System}?" doesn't double-phrase.
    pub(crate) fn short_name_key(&self) -> &'static str {
        match self {
            Self::System => "reboot_target_system",
            Self::Recovery => "reboot_target_recovery",
            Self::Bootloader => "reboot_target_bootloader",
            Self::Fastbootd => "reboot_target_fastbootd",
            Self::Edl => "reboot_target_edl",
        }
    }
    /// Reachable from `conn`. Impossible combos (Fastboot → Recovery,
    /// EDL → Recovery/Bootloader — Firehose only resets system/edl)
    /// stay disabled.
    pub(crate) fn available_from(&self, conn: ConnectionStatus) -> bool {
        match (conn, self) {
            (ConnectionStatus::None, _) => false,
            (ConnectionStatus::AdbUnauthorized, _) => false,
            // minadbd answers `reboot:` even though it refuses `shell:`,
            // so system/recovery/bootloader work. EDL does not: LTBox
            // reaches it by running `reboot edl` in a shell there is none
            // of, and the resulting error can pass for adbd dropping the
            // connection after a reboot that never fired.
            (ConnectionStatus::AdbSideload, Self::Edl) => false,
            (ConnectionStatus::AdbSideload, _) => true,
            (ConnectionStatus::AdbServerBlocking, _) => false,
            (ConnectionStatus::Adb, _) => true,
            (ConnectionStatus::AdbRecovery, _) => true,
            (ConnectionStatus::Fastboot, Self::Recovery) => false,
            (ConnectionStatus::Fastboot, _) => true,
            (ConnectionStatus::Edl, Self::System | Self::Edl) => true,
            (ConnectionStatus::Edl, _) => false,
        }
    }
    /// Whether this target is the mode represented by the current transport.
    /// A no-op reboot is shown as "current state" rather than as an action.
    pub(crate) fn is_current_from(&self, conn: ConnectionStatus, fastboot_userspace: bool) -> bool {
        matches!(
            (conn, self, fastboot_userspace),
            (ConnectionStatus::Adb, Self::System, _)
                | (ConnectionStatus::AdbRecovery, Self::Recovery, _)
                | (ConnectionStatus::AdbSideload, Self::Recovery, _)
                | (ConnectionStatus::Fastboot, Self::Bootloader, false)
                | (ConnectionStatus::Fastboot, Self::Fastbootd, true)
                | (ConnectionStatus::Edl, Self::Edl, _)
        )
    }
    pub(crate) fn all() -> &'static [RebootTarget] {
        &[
            Self::System,
            Self::Recovery,
            Self::Bootloader,
            Self::Fastbootd,
            Self::Edl,
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AdvAction {
    RegionConvert,
    ImageInfo,
    PatchDevinfo,
    DetectArb,
    PatchArb,
    ConvertXml,
    DumpPartitions,
    DumpPhysical,
    FlashPartitions,
    FlashPhysical,
    RebuildVbmeta,
    SimpleFlash,
}
impl AdvAction {
    /// Whether this action writes to the device rather than reading from
    /// it or transforming a local file. The Advanced grid renders these
    /// on the `error` role: a tile that flashes a partition should not
    /// be visually interchangeable with one that dumps it.
    pub(crate) fn is_destructive(&self) -> bool {
        matches!(
            self,
            Self::FlashPartitions | Self::FlashPhysical | Self::SimpleFlash
        )
    }

    pub(crate) fn label_key(&self) -> &'static str {
        match self {
            Self::RegionConvert => "adv_region_convert",
            Self::ImageInfo => "adv_image_info",
            Self::PatchDevinfo => "adv_patch_devinfo",
            Self::DetectArb => "adv_detect_arb",
            Self::PatchArb => "adv_patch_arb",
            Self::ConvertXml => "adv_convert_xml",
            Self::DumpPartitions => "adv_dump_partitions",
            Self::DumpPhysical => "adv_dump_physical",
            Self::FlashPartitions => "adv_flash_partitions",
            Self::FlashPhysical => "adv_flash_physical",
            Self::RebuildVbmeta => "adv_rebuild_vbmeta",
            Self::SimpleFlash => "adv_simple_flash",
        }
    }
    pub(crate) fn desc_key(&self) -> &'static str {
        match self {
            Self::RegionConvert => "adv_region_convert_desc",
            Self::ImageInfo => "adv_image_info_desc",
            Self::PatchDevinfo => "adv_patch_devinfo_desc",
            Self::DetectArb => "adv_detect_arb_desc",
            Self::PatchArb => "adv_patch_arb_desc",
            Self::ConvertXml => "adv_convert_xml_desc",
            Self::DumpPartitions => "adv_dump_partitions_desc",
            Self::DumpPhysical => "adv_dump_physical_desc",
            Self::FlashPartitions => "adv_flash_partitions_desc",
            Self::FlashPhysical => "adv_flash_physical_desc",
            Self::RebuildVbmeta => "adv_rebuild_vbmeta_desc",
            Self::SimpleFlash => "adv_simple_flash_desc",
        }
    }
    /// Browse-tile sub-description: *what* to pick, not the action's
    /// high-level description.
    pub(crate) fn source_desc_key(&self) -> &'static str {
        match self {
            Self::RegionConvert => "adv_src_region_convert",
            Self::ImageInfo => "adv_src_image_info",
            Self::PatchDevinfo => "adv_src_patch_devinfo",
            Self::DetectArb => "adv_src_detect_arb",
            Self::PatchArb => "adv_src_patch_arb_folder",
            Self::ConvertXml => "adv_src_convert_xml",
            Self::DumpPartitions => "adv_src_dump_partitions",
            Self::DumpPhysical => "adv_src_dump_physical",
            Self::FlashPartitions => "adv_src_flash_partitions",
            Self::FlashPhysical => "adv_src_flash_physical",
            Self::RebuildVbmeta => "adv_src_rebuild_vbmeta",
            // SimpleFlash uses a dedicated wizard (folder picker on Next),
            // not the generic source tile — reuse the flash-folder caption.
            Self::SimpleFlash => "flash_folder_desc",
        }
    }
    /// snake_case slug for `{exe_dir}/output_{slug}/` — Advanced ops
    /// drop artefacts here instead of asking the user for a location.
    pub(crate) fn output_slug(&self) -> &'static str {
        match self {
            Self::RegionConvert => "region_convert",
            Self::ImageInfo => "image_info",
            Self::PatchDevinfo => "patch_devinfo",
            Self::DetectArb => "detect_arb",
            Self::PatchArb => "rb",
            Self::ConvertXml => "convert_xml",
            Self::DumpPartitions => "dump_partitions",
            Self::DumpPhysical => "dump_physical",
            Self::FlashPartitions => "flash_partitions",
            Self::FlashPhysical => "flash_physical",
            Self::RebuildVbmeta => "rebuild_vbmeta",
            Self::SimpleFlash => "simple_flash",
        }
    }
    /// True iff the action writes into the output folder — gates the
    /// "Open Folder" pill on the Done card.
    pub(crate) fn produces_output(&self) -> bool {
        matches!(
            self,
            Self::RegionConvert
                | Self::PatchDevinfo
                | Self::PatchArb
                | Self::ConvertXml
                | Self::RebuildVbmeta
        )
    }
}

/// Auto-output directory for an Advanced wizard action. Caller
/// `create_dir_all`s before writing. Routes through
/// [`ltbox_core::app_paths::auto_output_dir_for`] so AppImage /
/// distro-installed Linux copies don't try to write next to a
/// read-only or root-owned executable. Windows path stays
/// exe-adjacent (`<exe-dir>/output_<slug>`) for v3 continuity.
pub(crate) fn adv_output_dir(action: AdvAction) -> std::path::PathBuf {
    ltbox_core::app_paths::auto_output_dir_for(action.output_slug())
}

/// Launch the platform file manager on `path`.
///
/// Returns `Ok(())` only when a launcher actually accepted the spawn, so a
/// missing `xdg-open` (or a Linux desktop session without a MIME handler for
/// `inode/directory`) surfaces instead of silently no-op'ing. Callers must
/// show the returned error in the GUI log / error popup so users know why
/// "Open Folder" did nothing.
pub(crate) fn open_in_file_manager(path: &std::path::Path) -> std::result::Result<(), String> {
    #[cfg(windows)]
    {
        // `CREATE_NO_WINDOW` hides the transient cmd flash.
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std::process::Command::new("explorer")
            .arg(path)
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("explorer {}: {e}", path.display()))
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("open {}: {e}", path.display()))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // Try xdg-open first (every desktop ships one); fall back to
        // GNOME's `gio open` which behaves correctly on
        // xdg-portal-only sessions where `xdg-open` itself errors out
        // mapping `inode/directory`. Capture the xdg error before
        // touching `gio` so the match below is exhaustive (compiler
        // can't see that the early return makes `xdg` provably Err
        // by this point).
        let xdg = std::process::Command::new("xdg-open").arg(path).spawn();
        if xdg.is_ok() {
            return Ok(());
        }
        let xdg_err = xdg.expect_err("checked Ok above");
        let gio = std::process::Command::new("gio")
            .arg("open")
            .arg(path)
            .spawn();
        match gio {
            Ok(_) => Ok(()),
            Err(gio_err) => Err(format!(
                "xdg-open {}: {xdg_err}; gio open {}: {gio_err}",
                path.display(),
                path.display(),
            )),
        }
    }
}
pub(crate) struct AdvSection {
    pub(crate) title_key: &'static str,
    pub(crate) items: &'static [AdvAction],
}

pub(crate) const ADV_SECTIONS: &[AdvSection] = &[
    AdvSection {
        title_key: "adv_section_region_patch",
        items: &[AdvAction::RegionConvert, AdvAction::PatchDevinfo],
    },
    AdvSection {
        title_key: "adv_section_rollback",
        items: &[
            AdvAction::ImageInfo,
            AdvAction::DetectArb,
            AdvAction::PatchArb,
            AdvAction::RebuildVbmeta,
        ],
    },
    AdvSection {
        title_key: "adv_section_edl_ops",
        items: &[
            AdvAction::ConvertXml,
            // Per-partition Read / Write paired together (read above
            // write so users can dump first, then re-flash if needed).
            AdvAction::DumpPartitions,
            AdvAction::FlashPartitions,
            // Whole-LUN dump / flash paired the same way.
            AdvAction::DumpPhysical,
            AdvAction::FlashPhysical,
            // Stock-equivalent flash: no checks, no edits — just flashing.
            AdvAction::SimpleFlash,
        ],
    },
];
