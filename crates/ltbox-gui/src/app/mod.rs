//! The `App` state, its defaults and the non-update helper methods.

use crate::*;
use iced::widget::column;

mod device;
mod loader;
mod logging;
mod operation;
mod region;

/// Which Advanced sub-wizard (if any) currently owns the screen. Sum
/// type so the dedicated sub-wizards stay mutually exclusive at the type
/// level — adding another wizard turns existing read sites into
/// non-exhaustive `match` errors instead of silent precedence bugs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum AdvancedWizardOpen {
    #[default]
    None,
    FlashParts,
    DumpParts,
    DumpPhys,
    FlashPhys,
    SimpleFlash,
}

impl AdvancedWizardOpen {
    pub(crate) fn is_open(self) -> bool {
        !matches!(self, Self::None)
    }
    pub(crate) fn is_flash_parts(self) -> bool {
        matches!(self, Self::FlashParts)
    }
    pub(crate) fn is_dump_parts(self) -> bool {
        matches!(self, Self::DumpParts)
    }
    pub(crate) fn is_dump_phys(self) -> bool {
        matches!(self, Self::DumpPhys)
    }
    pub(crate) fn is_flash_phys(self) -> bool {
        matches!(self, Self::FlashPhys)
    }
    pub(crate) fn is_simple_flash(self) -> bool {
        matches!(self, Self::SimpleFlash)
    }
}

pub(crate) fn partition_table_leading_action(
    entry_connection: Option<ConnectionStatus>,
) -> WizardLeadingAction {
    match entry_connection {
        Some(ConnectionStatus::Edl) | None => WizardLeadingAction::Back,
        Some(_) => WizardLeadingAction::Cancel,
    }
}

pub(crate) fn edl_entry_action(conn: ConnectionStatus) -> EdlEntryAction {
    match conn {
        ConnectionStatus::Edl => EdlEntryAction::AlreadyEdl,
        ConnectionStatus::Adb | ConnectionStatus::AdbRecovery => EdlEntryAction::AdbReboot,
        ConnectionStatus::Fastboot => EdlEntryAction::FastbootRebootThenAdb,
        ConnectionStatus::AdbUnauthorized
        | ConnectionStatus::AdbSideload
        | ConnectionStatus::AdbServerBlocking
        | ConnectionStatus::None => EdlEntryAction::ManualWait,
    }
}

/// Clamp a requested driver mode to what the host can actually use. Kernel mode
/// is forced back to userspace where it is unsupported (macOS, and non-Debian
/// Linux without `dpkg-query`), so a persisted/stale `kernel` value or a UI race
/// can never leave the app in an unusable kernel state. Mirrors the Settings
/// picker lock in `view::settings`.
pub(crate) fn effective_qcom_driver_mode(
    mode: ltbox_device::driver::QcomDriverMode,
) -> ltbox_device::driver::QcomDriverMode {
    if mode.is_kernel() && !ltbox_device::driver::kernel_mode_supported() {
        ltbox_device::driver::QcomDriverMode::Userspace
    } else {
        mode
    }
}

pub(crate) struct App {
    pub(crate) window_id: Option<iced::window::Id>,
    /// Host maximized state for the custom titlebar restore/maximize glyph.
    /// Iced exposes this as a query, not a window event, so update/window.rs
    /// refreshes it when the id arrives and after resize/toggle traffic.
    pub(crate) window_maximized: bool,
    pub(crate) current_view: View,
    /// Effective dark-mode flag — cached to keep repaint off the OS
    /// registry. Recomputed on theme-choice change.
    pub(crate) dark_mode: bool,
    pub(crate) theme_choice: ThemeChoice,
    pub(crate) theme_seed: ThemeSeed,
    /// Persisted font-source preference. The runtime font is bound before iced
    /// starts, so editing this field only affects the next launch.
    pub(crate) use_system_font: bool,
    pub(crate) settings: SettingsState,
    pub(crate) translations: Translations,
    /// Per-launch disclaimer gate. It is deliberately absent from persisted settings.
    pub(crate) startup_disclaimer_open: bool,
    pub(crate) startup_disclaimer_checked: bool,
    /// Session-only open state for the About screen's license inventory.
    pub(crate) about_licenses_open: bool,
    pub(crate) help_dialog: Option<(String, String)>,
    pub(crate) root: RootWizard,
    pub(crate) flash: FlashWizard,
    pub(crate) sysupdate: SysUpdateWizard,
    pub(crate) debloat: DebloatWizard,
    pub(crate) unroot: UnrootWizard,
    /// Staged path for the pending advanced action — replayed into the
    /// exec path on Start so no second dialog fires.
    pub(crate) adv_confirm_path: Option<String>,
    pub(crate) adv_wizard: AdvWizard,
    /// Dedicated EDL-based KonaBess flow; target-popup state is owned here.
    pub(crate) konabess: KonaBessWizard,
    pub(crate) wf_config: WorkflowConfig,
    /// Flash-confirm "hidden dropdown" editor: which row's option picker is
    /// open (`None` = closed). `Country` reuses `country_popup_open` instead.
    pub(crate) confirm_edit_field: Option<ConfirmField>,
    /// Snapshot of `wf_config` taken when the confirm step is first entered.
    /// A confirm row is rendered as "changed" (accent background + hover
    /// caution) when its field diverges from this baseline.
    pub(crate) confirm_baseline: Option<WorkflowConfig>,
    /// Confirm-step manual rollback editor. `Some` only while open; its
    /// values become `wf_config` targets only after a valid confirm.
    pub(crate) manual_rollback_editor: Option<ManualRollbackEditor>,
    /// In-flight text for the two manual rollback fields. Kept outside the
    /// editor so a cancelled popup cannot mutate the confirmed targets.
    pub(crate) manual_rollback_buffers: Option<(String, String)>,
    /// Last value each buffer parsed to. Switching display format renders
    /// from this rather than re-parsing the text, because the date form has
    /// day granularity and a text round-trip would silently move a value
    /// back to midnight.
    pub(crate) manual_rollback_values: (Option<u64>, Option<u64>),
    pub(crate) country_popup_open: bool,
    /// Live search text and the row staged inside the country popup. The
    /// committed workflow value is only changed by the popup footer action.
    pub(crate) country_popup_search: String,
    pub(crate) country_popup_draft: CountryAction,
    /// Routes `SelectCountry` back to the Advanced wizard instead of
    /// the Flash flow when PatchDevinfo opened the popup.
    pub(crate) adv_needs_country: bool,
    /// Region-convert target picker overlay. Shown when the
    /// `RegionConvert` wizard reaches step 1 so the user can pick
    /// PRC or ROW as the destination explicitly instead of relying
    /// on the prior auto-flip behaviour.
    pub(crate) region_target_popup_open: bool,
    /// Staging slot for the Reboot confirm popup.
    pub(crate) reboot_confirm_target: Option<RebootTarget>,
    /// True while the current tracked Reboot operation is waiting for EDL.
    /// Kept separate from visibility so Close can hide only the dialog.
    pub(crate) reboot_wait_transition: bool,
    pub(crate) reboot_wait_dialog_open: bool,
    // Device & operation state
    pub(crate) software_fix: software_fix::State,
    pub(crate) device: DeviceSnapshot,
    pub(crate) queries: DeviceQueries,
    pub(crate) adb_server_kill_in_flight: bool,
    /// Device-info popup state. `Some((serial, state))` while open.
    pub(crate) device_info_popup: Option<(String, DeviceInfoState)>,
    /// Firmware-OTA popup state. `Some((serial, firmware_id, state))` while open.
    pub(crate) ota_popup: Option<(String, String, OtaPopupState)>,
    /// Selectable mirror of OTA changelog — `text` widget can't be selected.
    pub(crate) ota_changelog_editor: iced::widget::text_editor::Content,
    /// QFIL-firmware popup state. `Some((serial, state))` while open.
    pub(crate) qfil_popup: Option<(String, QfilPopupState)>,
    /// Manual-serial prompt for region detection. `Some(buffer)` = open;
    /// buffer holds the in-progress input. Opened by the automatic-selection
    /// option when no
    /// usable polled serial is available.
    pub(crate) flash_serial_prompt: Option<String>,
    pub(crate) rollback_popup_open: bool,
    /// Shared across both rows so `boot` and `vbmeta_system` stay
    /// directly comparable while cycling.
    pub(crate) rollback_value_format: RollbackValueFormat,
    /// Separate from `rollback_value_format`: the dashboard parses fastboot
    /// `getvar` output and so defaults to hex, while this editor mirrors
    /// `avbtool info_image`, which prints a plain unix timestamp.
    pub(crate) manual_rollback_format: RollbackValueFormat,
    /// Transient toast message; auto-cleared by a delayed task.
    pub(crate) toast_msg: Option<String>,
    pub(crate) toast_generation: u64,
    /// Compact-sidebar hover state — true when the mouse is over the rail.
    pub(crate) sidebar_expanded: bool,
    /// Compact overlay tween progress in [0.0, 1.0].
    /// Width = lerp(64, 232, anim); Expanded layout ignores this value.
    /// Driven by an M3 Expressive Spatial spring (see `SidebarAnimTick`).
    pub(crate) sidebar_anim: f32,
    /// Spring velocity for `sidebar_anim`. Settle requires both the
    /// displacement to target AND the velocity to be near zero so we
    /// don't stop the subscription mid-overshoot.
    pub(crate) sidebar_velocity: f32,
    pub(crate) sidebar_label_alpha: f32,
    pub(crate) sidebar_label_velocity: f32,
    /// Current layout dimensions, including maximized windows.
    pub(crate) window_size: (f32, f32),
    /// Last confirmed normal-window size used on the next launch.
    pub(crate) window_restore_size: (f32, f32),
    /// Last resize event, used for trailing debounce rather than throttling.
    pub(crate) window_size_last_change: std::time::Instant,
    /// `true` while a pending window-size update hasn't been flushed
    /// to disk. Cleared by `persist_window_size_if_due`.
    pub(crate) window_size_dirty: bool,
    // Device portrait derived at view time via `device_portrait()`.
    pub(crate) operation: OperationExecution,
    /// Persisted recent picks. Rendered as chips under every picker.
    pub(crate) recent_paths: settings_store::RecentPaths,
    /// Reuse the loader a model last uploaded successfully, skipping its loader
    /// step. Off means every loader prompt shows its picker.
    pub(crate) remember_edl_loader: bool,
    /// Model name (upper-case) → loader path, populated only by uploads that
    /// actually completed. Mirrors `settings_store::remembered_edl_loaders`.
    pub(crate) remembered_edl_loaders: std::collections::BTreeMap<String, String>,
    /// Model the running operation belongs to, captured before the device
    /// re-enumerates into EDL (which can blank `device.model`). The loader an
    /// operation uploads is credited to this, not to whatever is connected when
    /// the upload lands. Not persisted.
    pub(crate) loader_memory_model: Option<String>,
    pub(crate) qcom_driver_mode: ltbox_device::driver::QcomDriverMode,
    /// `true` while a Settings "Clean temporary files" sweep is running.
    pub(crate) cleaning_temp: bool,
    /// Cached on-disk size of removable temp files (`work_*` + `output_*`),
    /// rescanned on Settings entry and after a sweep. `None` until first
    /// scanned; drives the cleanup button's enabled state + size readout.
    pub(crate) temp_files_bytes: Option<u64>,
    pub(crate) log_lines: Vec<String>,
    pub(crate) log_history: log_history::LogHistory,
    /// Selectable mirror of `log_lines`. Rebuilt on drain tick when `log_dirty`
    /// — batched to keep a long pbr flash from crashing wgpu.
    pub(crate) log_editor: iced::widget::text_editor::Content,
    pub(crate) log_dirty: bool,
    pub(crate) image_info_log: String,
    pub(crate) image_info_log_editor: iced::widget::text_editor::Content,
    pub(crate) pending_log_save_source: LogSaveSource,
    pub(crate) error_msg: Option<String>,
    pub(crate) operation_error: Option<String>,
    pub(crate) picker_target: PickerTarget,
    pub(crate) driver_status: Option<ltbox_device::driver::DriverStatus>,
    pub(crate) installing_drivers: bool,
    /// Session-only post-install reminder. Set after a successful Qualcomm
    /// driver install/update and cleared when the user closes its banner.
    pub(crate) driver_restart_recommended: bool,
    /// `Some` when the installed Qualcomm driver is older than the latest
    /// release — drives the optional amber "update available" banner. Held
    /// `None` when up to date, not installed, offline, or the user chose
    /// "don't show again".
    pub(crate) driver_update: Option<ltbox_device::driver::DriverUpdate>,
    /// Result of the startup GitHub-reachability probe. `None` until the
    /// probe lands; `Some(false)` disables the driver install/update
    /// buttons with an "internet required" tooltip.
    pub(crate) online: Option<bool>,
    /// Persisted "don't show again" for the driver-update prompt. Skips the
    /// update check + banner; never affects the missing-driver banner.
    pub(crate) qcom_driver_update_dismissed: bool,
    /// Models whose dual-USB-C port guide the user permanently dismissed
    /// ("don't show again"); loaded from + saved to settings.
    pub(crate) dual_usb_advisory_dismissed: Vec<String>,
    /// Models whose guide was closed this session only ("close"). Not
    /// persisted, so the guide returns on the next launch.
    pub(crate) dual_usb_advisory_closed: Vec<String>,
    /// Whether the illustrated dual-USB-C port guide is open.
    pub(crate) dual_usb_help_open: bool,
    /// Model the open dual-USB-C port guide describes. Session-only so the
    /// guide keeps its subject while the live device disconnects or changes.
    pub(crate) dual_usb_help_model: String,
    /// Friendly name captured with the model so the guide subtitle survives
    /// a disconnect while the dialog remains open.
    pub(crate) dual_usb_help_name: String,
    /// Newest stable (`prerelease == false && draft == false`) release on
    /// `miner7222/LTBox` whose semver is strictly greater than the
    /// running build's. `None` either before the background probe lands
    /// or when the running build is already at-or-ahead of the latest
    /// stable. Populates the green sidebar "Update available" pill.
    pub(crate) update_available: Option<ltbox_core::github::StableRelease>,
    /// Package-managed install source while the update instructions dialog is
    /// open, or `Direct` while the verified self-update dialog is open.
    pub(crate) update_dialog_source: Option<ltbox_core::install_source::InstallSource>,
    pub(crate) flash_parts: FlashPartsWizard,
    pub(crate) dump_parts: DumpPartsWizard,
    pub(crate) dump_phys: DumpPhysWizard,
    pub(crate) flash_phys: FlashPhysWizard,
    pub(crate) simple_flash: SimpleFlashWizard,
    /// Single sum-typed flag for the mutually-exclusive Advanced
    /// sub-wizards. Replaces 4 parallel booleans whose `if/else if`
    /// read sites would silently pick a precedence if two ever got
    /// set. `match`-driven dispatch makes that bug class unreachable.
    pub(crate) advanced_wizard_open: AdvancedWizardOpen,
    /// Latest live firmware flash progress snapshot for the shared exec card.
    pub(crate) flash_progress: Option<ltbox_device::edl::FlashProgress>,
    pub(crate) log_popup_open: bool,
    #[cfg(feature = "demo")]
    pub(crate) demo_scene: Option<demo::Scene>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum PickerTarget {
    #[default]
    None,
    RootFile,
    UnrootFolder,
    FlashFolder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum LogSaveSource {
    #[default]
    Main,
    ImageInfo,
}

impl PickerTarget {
    /// Map this routing target to the recents bucket it should store into.
    /// `None` returns `File` defensively so callers get a valid bucket even
    /// if they forgot to set the target — the recents entry is harmless;
    /// the field-routing `match` in `FolderSelected` / `FileSelected` is
    /// what actually prevents wrong writes.
    pub(crate) fn kind(self) -> pickers::PickerKind {
        use pickers::PickerKind;
        match self {
            // Root OTA file is a unified file pick (zip or apk).
            Self::None | Self::RootFile => PickerKind::File,
            // Firmware folders all share the "full QFIL" bucket — Unroot
            // and Flash typically point the user at the same dump/archive
            // they extracted from `ltbox dump full`.
            Self::UnrootFolder | Self::FlashFolder => PickerKind::QfilFirmwareFolder,
        }
    }
}

impl Default for App {
    fn default() -> Self {
        let persisted = settings_store::load();
        let lang = Language::from_code(&persisted.language).unwrap_or(Language::En);
        // Upgrade path: prefer `theme`, fall back to legacy `dark_mode`.
        let theme_choice = ThemeChoice::from_code(&persisted.theme).unwrap_or({
            if persisted.theme.is_empty() && persisted.dark_mode {
                ThemeChoice::Dark
            } else {
                ThemeChoice::System
            }
        });
        let theme_seed = ThemeSeed::from_code(&persisted.theme_seed).unwrap_or_default();
        let qcom_driver_mode = effective_qcom_driver_mode(
            ltbox_device::driver::QcomDriverMode::from_code(&persisted.qcom_driver_mode),
        );
        ltbox_device::driver::set_qcom_driver_mode(qcom_driver_mode);
        let dark_mode = match theme_choice {
            ThemeChoice::Light => false,
            ThemeChoice::Dark => true,
            ThemeChoice::System => theme_detect::system_prefers_dark(),
        };
        theme::set_runtime_theme(theme_seed, dark_mode);
        install_core_translator(lang);
        let translations = Translations::load(lang);
        let ready_log = translations.t("log_ready").to_string();
        Self {
            window_id: None,
            window_maximized: false,
            current_view: View::default(),
            dark_mode,
            theme_choice,
            theme_seed,
            use_system_font: persisted.use_system_font,
            settings: SettingsState { language: lang },
            translations,
            startup_disclaimer_open: true,
            startup_disclaimer_checked: false,
            about_licenses_open: false,
            help_dialog: None,
            root: RootWizard::default(),
            flash: FlashWizard::default(),
            sysupdate: SysUpdateWizard::default(),
            debloat: DebloatWizard::default(),
            unroot: UnrootWizard::default(),
            adv_confirm_path: None,
            adv_wizard: AdvWizard::default(),
            konabess: KonaBessWizard::default(),
            wf_config: WorkflowConfig::default(),
            confirm_edit_field: None,
            confirm_baseline: None,
            manual_rollback_editor: None,
            manual_rollback_buffers: None,
            manual_rollback_values: (None, None),
            country_popup_open: false,
            country_popup_search: String::new(),
            country_popup_draft: CountryAction::Unset,
            adv_needs_country: false,
            region_target_popup_open: false,
            reboot_confirm_target: None,
            reboot_wait_transition: false,
            reboot_wait_dialog_open: false,
            software_fix: software_fix::State::default(),
            device: DeviceSnapshot::default(),
            queries: DeviceQueries::default(),
            adb_server_kill_in_flight: false,
            device_info_popup: None,
            ota_popup: None,
            ota_changelog_editor: iced::widget::text_editor::Content::with_text(""),
            qfil_popup: None,
            flash_serial_prompt: None,
            rollback_popup_open: false,
            rollback_value_format: RollbackValueFormat::default(),
            manual_rollback_format: RollbackValueFormat::Unix,
            toast_msg: None,
            toast_generation: 0,
            sidebar_expanded: false,
            sidebar_anim: 0.0,
            sidebar_velocity: 0.0,
            sidebar_label_alpha: 0.0,
            sidebar_label_velocity: 0.0,
            // Use the persisted size if present, otherwise the default
            // initial window dimensions (kept in lockstep with the
            // values passed to `iced::window::Settings::size` in `main`).
            window_size: persisted
                .window_size
                .unwrap_or((DEFAULT_WINDOW_WIDTH, DEFAULT_WINDOW_HEIGHT)),
            window_restore_size: persisted
                .window_size
                .unwrap_or((DEFAULT_WINDOW_WIDTH, DEFAULT_WINDOW_HEIGHT)),
            window_size_last_change: std::time::Instant::now(),
            window_size_dirty: false,
            operation: OperationExecution::default(),
            recent_paths: persisted.recent_paths.clone(),
            remember_edl_loader: persisted.remember_edl_loader,
            remembered_edl_loaders: persisted.remembered_edl_loaders.clone(),
            loader_memory_model: None,
            qcom_driver_mode,
            cleaning_temp: false,
            temp_files_bytes: None,
            log_lines: vec![ready_log.clone()],
            log_history: log_history::LogHistory::with_initial(&ready_log),
            log_editor: iced::widget::text_editor::Content::with_text(&ready_log),
            log_dirty: false,
            image_info_log: String::new(),
            image_info_log_editor: iced::widget::text_editor::Content::with_text(""),
            pending_log_save_source: LogSaveSource::Main,
            error_msg: None,
            operation_error: None,
            picker_target: PickerTarget::None,
            driver_status: None,
            installing_drivers: false,
            driver_restart_recommended: false,
            driver_update: None,
            online: None,
            qcom_driver_update_dismissed: persisted.qcom_driver_update_dismissed,
            dual_usb_advisory_dismissed: persisted.dual_usb_advisory_dismissed_models.clone(),
            dual_usb_advisory_closed: Vec::new(),
            dual_usb_help_open: false,
            dual_usb_help_model: String::new(),
            dual_usb_help_name: String::new(),
            update_available: None,
            update_dialog_source: None,
            flash_parts: FlashPartsWizard::default(),
            dump_parts: DumpPartsWizard::default(),
            dump_phys: DumpPhysWizard::default(),
            flash_phys: FlashPhysWizard::default(),
            simple_flash: SimpleFlashWizard::default(),
            advanced_wizard_open: AdvancedWizardOpen::default(),
            flash_progress: None,
            log_popup_open: false,
            #[cfg(feature = "demo")]
            demo_scene: None,
        }
    }
}

impl App {
    pub(crate) fn new() -> (Self, Task<Message>) {
        // Window-id + driver check + update check all fire in parallel.
        let app = Self::default();
        #[cfg(feature = "demo")]
        let mut app = app;
        #[cfg(feature = "demo")]
        demo::initialize(&mut app);
        let win =
            iced::window::latest().map(|__v| Message::Window(WindowMsg::WindowIdReceived(__v)));
        #[cfg(feature = "demo")]
        if demo::is_active(&app) {
            return (app, win);
        }
        let mut app = app;
        let stamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
        if let Err(error) = app
            .log_history
            .persist_to(&log_dir().join("sessions"), &stamp)
        {
            tracing::warn!("session transcript not persisted: {error}");
        }
        let driver_check = Task::perform(
            async {
                tokio::task::spawn_blocking(ltbox_device::driver::check_required_drivers)
                    .await
                    .unwrap_or(ltbox_device::driver::DriverStatus::NotWindows)
            },
            Message::DriverCheckDone,
        );
        // GitHub releases probe — runs once at startup. `latest_stable_release`
        // walks `/releases?per_page=100` (not `/releases/latest`) so the
        // result is well-defined even when the repo has only prereleases
        // published. Network failure / parse failure → `None`, no banner.
        let update_check = Task::perform(
            async {
                tokio::task::spawn_blocking(check_for_update)
                    .await
                    .unwrap_or(None)
            },
            Message::UpdateCheckDone,
        );
        // GitHub-reachability probe — gates the driver install/update
        // buttons so the user can't click into a guaranteed-to-fail
        // download while offline.
        let connectivity = Task::perform(
            async {
                tokio::task::spawn_blocking(ltbox_device::driver::probe_connectivity)
                    .await
                    .unwrap_or(false)
            },
            Message::ConnectivityChecked,
        );
        // Advisory startup probe, separate from the driver-button gate
        // above: it splits "no link at all" from "link up, GitHub
        // blocked" so the log can name which one the user is hitting.
        // Nothing waits on it and nothing is gated by it.
        let connectivity_notice = Task::perform(
            async {
                tokio::task::spawn_blocking(ltbox_core::connectivity::probe)
                    .await
                    .unwrap_or(ltbox_core::connectivity::ConnectivityReport {
                        internet: true,
                        github: true,
                    })
            },
            Message::StartupConnectivityProbed,
        );
        // Qualcomm driver version check. Skipped entirely (no network call)
        // when the user chose "don't show again" for driver updates. A
        // silent failure (offline / GitHub down / parse) yields `None`, so
        // no banner — distinct from the missing-driver banner, which the
        // separate `driver_check` above always drives.
        let driver_update_check = if app.qcom_driver_update_dismissed {
            Task::none()
        } else {
            Task::perform(
                async {
                    tokio::task::spawn_blocking(ltbox_device::driver::check_driver_update)
                        .await
                        .unwrap_or(None)
                },
                Message::DriverUpdateCheckDone,
            )
        };
        (
            app,
            Task::batch([
                win,
                driver_check,
                update_check,
                connectivity,
                connectivity_notice,
                driver_update_check,
                Task::done(Message::PollSoftwareFix),
            ]),
        )
    }

    pub(crate) fn theme(&self) -> Theme {
        self.sync_runtime_theme();
        Theme::custom(
            format!(
                "LTBox {} {}",
                self.theme_seed.code(),
                if self.dark_mode { "dark" } else { "light" }
            ),
            theme::iced_palette(self.theme_seed, self.dark_mode),
        )
    }

    pub(crate) fn sync_runtime_theme(&self) {
        theme::set_runtime_theme(self.theme_seed, self.dark_mode);
    }

    /// Localized string. Falls back to English, then the key itself.
    pub(crate) fn t<'a>(&'a self, key: &'a str) -> &'a str {
        self.translations.t(key)
    }

    pub(crate) fn pal(&self) -> Palette {
        palette_for(self.theme_seed, self.dark_mode)
    }

    pub(crate) fn parse_manual_rollback(&self, input: &str) -> Result<u64, String> {
        let index = self.manual_rollback_format.parse(input)?;
        let now =
            current_unix_timestamp().ok_or_else(|| "rollback_manual_error_clock".to_string())?;
        if index < now {
            Ok(index)
        } else {
            Err("rollback_manual_error_future".to_string())
        }
    }

    pub(crate) fn open_manual_rollback_editor(&mut self) -> Task<Message> {
        let advanced = self.current_view == View::Advanced
            && self.adv_wizard.action == Some(AdvAction::PatchArb);
        let defaults = if advanced {
            self.adv_wizard.arb_inspect.map(|(a, b)| (Ok(a), Ok(b)))
        } else {
            self.flash.firmware_rollback_indices.clone()
        };
        let Some(defaults) = defaults else {
            return Task::none();
        };

        let seed = |result: &Result<u64, String>| -> String {
            result.as_ref().ok().map_or_else(String::new, |index| {
                self.manual_rollback_format.render(*index)
            })
        };
        // Values the user already confirmed win over the image defaults —
        // reopening the editor to check a number must not silently discard it.
        // The image index stays visible under each field either way.
        let buffers = match if advanced {
            self.adv_wizard.arb_targets
        } else {
            self.wf_config.manual_rollback_indices
        } {
            Some(entered) => (
                self.manual_rollback_format.render(entered.boot),
                self.manual_rollback_format.render(entered.vbmeta_system),
            ),
            None => (seed(&defaults.0), seed(&defaults.1)),
        };
        self.confirm_edit_field = if advanced {
            None
        } else {
            Some(ConfirmField::Rollback)
        };
        self.manual_rollback_editor = Some(ManualRollbackEditor::Boot);
        self.manual_rollback_values = (
            self.manual_rollback_format.parse(&buffers.0).ok(),
            self.manual_rollback_format.parse(&buffers.1).ok(),
        );
        self.manual_rollback_buffers = Some(buffers);
        Task::none()
    }

    /// Compare the confirmed Manual targets against the selected firmware's
    /// own image indices. Device floors are intentionally not consulted.
    pub(crate) fn manual_rollback_downgrade_warning(&self) -> Option<()> {
        let targets = self.wf_config.manual_rollback_indices?;
        let originals = self.flash.firmware_rollback_indices.as_ref()?;
        let boot_lower = matches!(&originals.0, Ok(original) if targets.boot < *original);
        let vbmeta_lower =
            matches!(&originals.1, Ok(original) if targets.vbmeta_system < *original);
        (boot_lower || vbmeta_lower).then_some(())
    }

    pub(crate) fn country_popup_selected_code(&self) -> Option<&str> {
        if self.adv_needs_country {
            self.adv_wizard.country.as_deref()
        } else {
            self.wf_config.country_action.target()
        }
    }

    pub(crate) fn persist_settings(&self) {
        #[cfg(feature = "demo")]
        if demo::is_active(self) {
            return;
        }
        settings_store::save(&settings_store::PersistedSettings {
            language: self.settings.language.code().to_string(),
            theme: self.theme_choice.code().to_string(),
            theme_seed: self.theme_seed.code().to_string(),
            use_system_font: self.use_system_font,
            // Legacy field kept readable by older builds.
            dark_mode: self.dark_mode,
            recent_paths: self.recent_paths.clone(),
            remember_edl_loader: self.remember_edl_loader,
            remembered_edl_loaders: self.remembered_edl_loaders.clone(),
            qcom_driver_mode: self.qcom_driver_mode.code().to_string(),
            window_size: Some(self.window_restore_size),
            qcom_driver_update_dismissed: self.qcom_driver_update_dismissed,
            dual_usb_advisory_dismissed_models: self.dual_usb_advisory_dismissed.clone(),
        });
    }

    /// Record `path` in the MRU list for `kind`. Persists on change so
    /// the list survives restarts (write is cheap — small JSON, and only
    /// triggers when the list actually moves).
    pub(crate) fn remember_recent(&mut self, kind: pickers::PickerKind, path: &str) {
        if self.recent_paths.push(kind.storage_key(), path) {
            self.persist_settings();
        }
    }

    /// Error to surface for a finished Firehose GPT scan: the worker's own
    /// failure, or a scan that came back with no partitions at all. `None`
    /// means the table is usable and the wizard may advance.
    pub(crate) fn parts_scan_outcome(
        &self,
        error: Option<String>,
        rows_empty: bool,
    ) -> Option<String> {
        error.or_else(|| rows_empty.then(|| self.t("err_parts_scan_empty").to_string()))
    }

    pub(crate) fn subscription(&self) -> Subscription<Message> {
        let mut subs = vec![
            iced::time::every(std::time::Duration::from_secs(3)).map(|_| Message::PollDevice),
            // 500 ms drain — 4 Hz drove some GPU drivers into TDR
            // during long qdl flashes.
            iced::time::every(std::time::Duration::from_millis(500))
                .map(|_| Message::DrainStdoutTap),
        ];
        #[cfg(windows)]
        subs.push(
            iced::time::every(std::time::Duration::from_secs(2)).map(|_| Message::PollSoftwareFix),
        );
        // Compact hover drives the sidebar width spring. Expanded uses a
        // fixed drawer, but a single tick still clears compact hover state
        // inherited across a resize before the hidden spring settles closed.
        // Once both state and spring are settled, no 16 ms subscription runs.
        let sidebar_target = self.sidebar_anim_target();
        let sidebar_hover_settled =
            self.window_size_class() == WindowSizeClass::Compact || !self.sidebar_expanded;
        let sidebar_settled = (self.sidebar_anim - sidebar_target).abs() < 0.001
            && self.sidebar_velocity.abs() < 0.05
            && (self.sidebar_label_alpha - f32::from(self.sidebar_expanded)).abs() < 0.001
            && self.sidebar_label_velocity.abs() < 0.05
            && sidebar_hover_settled;
        if !sidebar_settled {
            subs.push(
                iced::time::every(std::time::Duration::from_millis(16))
                    .map(|_| Message::SidebarAnimTick),
            );
        }
        // Listen for window resize events so the user's preferred
        // geometry survives a restart. `event::listen_with` filters at
        // the source so non-window events don't bubble back as
        // `Message::Noop`.
        subs.push(iced::event::listen_with(
            |event, status, window_id| match event {
                iced::Event::Window(iced::window::Event::Opened { .. }) => Some(Message::Window(
                    WindowMsg::WindowIdReceived(Some(window_id)),
                )),
                iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                    key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Tab),
                    modifiers,
                    ..
                }) if status == iced::event::Status::Ignored => {
                    Some(Message::FocusMove(modifiers.shift()))
                }
                iced::Event::Window(iced::window::Event::Resized(size)) => {
                    Some(Message::WindowResized(size.width, size.height))
                }
                iced::Event::Window(iced::window::Event::CloseRequested) => {
                    Some(Message::Window(WindowMsg::WindowClose))
                }
                _ => None,
            },
        ));
        // Debounced window-size persistence tick: only fires while a
        // pending size update hasn't been flushed yet.
        if self.window_size_dirty {
            subs.push(
                iced::time::every(WINDOW_SIZE_SAVE_INTERVAL).map(|_| Message::PersistWindowSize),
            );
        }
        if self.theme_choice == ThemeChoice::System {
            subs.push(
                iced::time::every(std::time::Duration::from_secs(2))
                    .map(|_| Message::RefreshSystemTheme),
            );
        }
        Subscription::batch(subs)
    }

    /// Shared error-state body. Renders the localized header
    /// (`error_key`), the raw upstream error text, and a Retry pill
    /// that fires `retry_msg`. Same shape as the loading view —
    /// pulled out of the device-info / OTA popups which had two
    /// near-identical copies.
    pub(crate) fn popup_error_view(
        &self,
        error_key: &str,
        e: &str,
        retry_msg: Message,
    ) -> Element<'_, Message> {
        column![
            text(self.t(error_key).to_string())
                .size(theme::text_size::BODY_MEDIUM)
                .style(|t: &Theme| iced::widget::text::Style {
                    color: Some(pal_of(t).error),
                }),
            text(e.to_string()).size(11).style(muted_style),
            Space::new().height(8),
            m3_filled_button(self.t("btn_retry").to_string()).on_press(retry_msg),
        ]
        .spacing(8)
        .into()
    }

    /// Sidebar tween target for the compact hover overlay.
    ///
    /// Expanded has a separate fixed-width rendering path, so its compact
    /// animation state settles closed and cannot react to pointer proximity.
    pub(crate) fn sidebar_anim_target(&self) -> f32 {
        if self.window_size_class() == WindowSizeClass::Compact && self.sidebar_expanded {
            1.0
        } else {
            0.0
        }
    }

    /// Visual openness used by drawer labels.
    /// Expanded labels are always fully visible; Compact follows the spring.
    pub(crate) fn sidebar_visual_progress(&self) -> f32 {
        match self.window_size_class() {
            WindowSizeClass::Compact => self.sidebar_label_alpha,
            WindowSizeClass::Expanded => 1.0,
        }
    }

    /// Compact update action attached to the version at the status bar's
    /// trailing edge. It deliberately has no sidebar-animation dependency.
    pub(crate) fn status_update_available_button(&self) -> Element<'_, Message> {
        let content = row![
            icon::tile_update_on()
                .size(12)
                .line_height(1.0)
                .style(|t: &Theme| iced::widget::text::Style {
                    color: Some(pal_of(t).on_tertiary),
                }),
            text(self.t("status_update_available").to_string())
                .size(theme::text_size::BODY_SMALL)
                .line_height(1.0)
                .wrapping(iced::widget::text::Wrapping::None),
        ]
        .spacing(4)
        .align_y(iced::Alignment::Center);
        button(content)
            .on_press(Message::OpenUpdate)
            .padding([6, 8])
            .style(|t: &Theme, status| {
                let p = pal_of(t);
                button::Style {
                    background: Some(
                        theme::mix_color(p.tertiary, p.on_tertiary, theme::state_alpha(status))
                            .into(),
                    ),
                    text_color: p.on_tertiary,
                    border: iced::Border {
                        radius: theme::shape::FULL.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }
            })
            .into()
    }

    /// Per-extension recents strip for file pickers.
    pub(crate) fn recent_file_chips<F>(
        &self,
        accepted_exts: &[&str],
        on_pick: F,
        label_key: &str,
    ) -> Element<'_, Message>
    where
        F: Fn(String) -> Message,
    {
        let all = self
            .recent_paths
            .recent(pickers::PickerKind::File.storage_key());
        let filtered: Vec<String> = if accepted_exts.is_empty() {
            all.to_vec()
        } else {
            all.iter()
                .filter(|p| pickers::path_matches_extensions(p, accepted_exts))
                .cloned()
                .collect()
        };
        self.recent_chips(&filtered, on_pick, label_key, true)
    }

    /// Empty column when the list is empty so call sites can splice
    /// it in unconditionally.
    pub(crate) fn recent_chips<F>(
        &self,
        items: &[String],
        on_pick: F,
        label_key: &str,
        is_file_picker: bool,
    ) -> Element<'_, Message>
    where
        F: Fn(String) -> Message,
    {
        self.picker_recent_list(items, on_pick, label_key, is_file_picker)
    }
}
