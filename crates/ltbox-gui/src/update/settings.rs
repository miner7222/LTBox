//! Settings-view handler.
use crate::*;
use iced::Task;

impl App {
    /// Settings view: language, theme, font, driver mode, loader memory, backup
    /// folder and temp-file cleanup.
    pub(crate) fn update_settings(&mut self, msg: SettingsMsg) -> Task<Message> {
        match msg {
            SettingsMsg::SetLanguage(l) => {
                self.settings.language = l;
                self.translations = Translations::load(l);
                install_core_translator(l);
                self.persist_settings();
                Task::none()
            }
            SettingsMsg::SetThemeSeed(seed) => {
                self.theme_seed = seed;
                self.sync_runtime_theme();
                self.persist_settings();
                Task::none()
            }
            SettingsMsg::SetUseSystemFont(use_system_font) => {
                if self.use_system_font == use_system_font {
                    return Task::none();
                }
                self.use_system_font = use_system_font;
                self.persist_settings();
                Task::done(Message::ToastShow(
                    self.t("settings_font_restart_required").to_string(),
                ))
            }
            SettingsMsg::SetQcomDriverMode(mode) => {
                if self.operation.is_running() {
                    return Task::none();
                }
                let mode = effective_qcom_driver_mode(mode);
                if self.qcom_driver_mode == mode {
                    return Task::none();
                }
                self.qcom_driver_mode = mode;
                ltbox_device::driver::set_qcom_driver_mode(mode);
                self.driver_status = None;
                self.driver_update = None;
                self.qcom_driver_update_dismissed = false;
                self.persist_settings();
                Task::batch([
                    Task::perform(
                        async {
                            tokio::task::spawn_blocking(
                                ltbox_device::driver::check_required_drivers,
                            )
                            .await
                            .unwrap_or(ltbox_device::driver::DriverStatus::NotWindows)
                        },
                        Message::DriverCheckDone,
                    ),
                    Task::perform(
                        async {
                            tokio::task::spawn_blocking(ltbox_device::driver::check_driver_update)
                                .await
                                .unwrap_or(None)
                        },
                        Message::DriverUpdateCheckDone,
                    ),
                    Task::perform(
                        async {
                            tokio::task::spawn_blocking(ltbox_device::driver::probe_connectivity)
                                .await
                                .unwrap_or(false)
                        },
                        Message::ConnectivityChecked,
                    ),
                ])
            }
            SettingsMsg::SetRememberEdlLoader(remember) => {
                if self.remember_edl_loader == remember {
                    return Task::none();
                }
                self.remember_edl_loader = remember;
                // Learned loaders are kept either way, so switching back on
                // restores them.
                self.persist_settings();
                Task::none()
            }
            SettingsMsg::OpenBackupFolder => {
                let path = ltbox_core::app_paths::backup_root();
                if let Err(error) = std::fs::create_dir_all(&path) {
                    tracing::warn!("failed to create backup folder {}: {error}", path.display());
                } else if let Err(error) = open::that_detached(&path) {
                    tracing::warn!("failed to open backup folder {}: {error}", path.display());
                }
                Task::none()
            }
            SettingsMsg::CleanupTempFiles => {
                // Skip while an operation runs (it owns the `work_*` dirs) and
                // on a double-press.
                if self.operation.is_running() || self.cleaning_temp {
                    return Task::none();
                }
                self.cleaning_temp = true;
                // Mark the app busy for the sweep so no operation can start and
                // race the cleaner, which lists `work_*`/`output_*` up front and
                // could delete a freshly recreated dir. The progress dialog is
                // suppressed for it (see `should_show_busy_progress_dialog`).
                self.operation.start(None, OperationKind::Cleanup, None);
                Task::perform(
                    async {
                        tokio::task::spawn_blocking(
                            ltbox_core::app_paths::clean_temp_files_reporting,
                        )
                        .await
                        .ok();
                    },
                    |()| Message::Settings(SettingsMsg::CleanupDone),
                )
            }
            SettingsMsg::CleanupDone => {
                self.cleaning_temp = false;
                self.operation.finish(false);
                // Rescan: a locked dir can survive the sweep.
                self.scan_temp_files_task()
            }
            SettingsMsg::TempScanDone(bytes) => {
                self.temp_files_bytes = Some(bytes);
                Task::none()
            }
        }
    }

    /// Off-thread scan of removable temp files; result lands as
    /// [`SettingsMsg::TempScanDone`]. Dispatched on Settings entry and after
    /// every sweep.
    pub(crate) fn scan_temp_files_task(&self) -> Task<Message> {
        Task::perform(
            async {
                tokio::task::spawn_blocking(ltbox_core::app_paths::temp_files_size)
                    .await
                    .unwrap_or(0)
            },
            |bytes| Message::Settings(SettingsMsg::TempScanDone(bytes)),
        )
    }
}
