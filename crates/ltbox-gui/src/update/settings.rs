//! Settings-view handler: language, theme, default-loader path.
//! Extracted from `main.rs`.
use crate::*;
use iced::Task;

impl App {
    /// Settings view — language pick, theme dropdown, default-loader
    /// path management. Each variant either updates `self.settings`
    /// and persists, or spawns the file picker `Task` for the loader
    /// path.
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
                self.use_system_font = use_system_font;
                self.persist_settings();
                Task::none()
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
            SettingsMsg::SettingsPickDefaultLoader => {
                let spec = loader_file_spec();
                pickers::pick_file_for(spec, &self.recent_paths, |__v| {
                    Message::Settings(SettingsMsg::SettingsDefaultLoaderChosen(__v))
                })
            }
            SettingsMsg::SettingsDefaultLoaderChosen(path) => {
                if let Some(p) = path {
                    self.remember_recent(pickers::PickerKind::File, &p);
                    self.default_loader_path = Some(p);
                    self.persist_settings();
                }
                Task::none()
            }
            SettingsMsg::SettingsClearDefaultLoader => {
                self.default_loader_path = None;
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
                // Skip while a flash/root op is live — it owns the very
                // `work_*` dirs we'd be deleting. Also ignore a double-press.
                if self.operation.is_running() || self.cleaning_temp {
                    return Task::none();
                }
                self.cleaning_temp = true;
                // Hold the global `busy` lock for the (brief) sweep so every
                // existing op-start guard blocks a flash/root from racing the
                // cleaner — which enumerates `work_*`/`output_*` up front and
                // could otherwise delete a dir a freshly-started op just
                // recreated. The progress dialog is suppressed for this
                // lightweight op (see `should_show_busy_progress_dialog`); the
                // button's own "Cleaning…" state is the only feedback.
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
                // Rescan so the size readout + enabled state reflect what
                // actually remains (a locked dir could survive the sweep).
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
