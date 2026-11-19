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
            SettingsMsg::RequestQcomDriverMode(mode) => {
                if self.operation.is_running() {
                    return Task::none();
                }
                // Clamp first so a mode the host cannot use never opens a
                // dialog; choosing the current mode is a no-op.
                let mode = effective_qcom_driver_mode(mode);
                self.pending_qcom_driver_mode = (mode != self.qcom_driver_mode).then_some(mode);
                Task::none()
            }
            SettingsMsg::CancelQcomDriverMode => {
                self.pending_qcom_driver_mode = None;
                Task::none()
            }
            SettingsMsg::SetQcomDriverMode(mode) => {
                // The confirm dialog is answered either way, even when the
                // switch below is refused.
                self.pending_qcom_driver_mode = None;
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

#[cfg(test)]
mod tests {
    use super::*;
    use ltbox_device::driver::QcomDriverMode;

    /// Kernel is the stored mode; Userspace is the target. Userspace is
    /// supported on every host, so the tests need no platform branch.
    fn app_in_kernel_mode() -> App {
        App {
            qcom_driver_mode: QcomDriverMode::Kernel,
            ..App::default()
        }
    }

    #[test]
    fn requesting_a_different_driver_mode_only_opens_the_dialog() {
        let mut app = app_in_kernel_mode();
        let _ = app.update_settings(SettingsMsg::RequestQcomDriverMode(
            QcomDriverMode::Userspace,
        ));
        assert_eq!(
            app.pending_qcom_driver_mode,
            Some(QcomDriverMode::Userspace)
        );
        assert_eq!(app.qcom_driver_mode, QcomDriverMode::Kernel);
    }

    #[test]
    fn requesting_the_current_driver_mode_does_nothing() {
        let mut app = app_in_kernel_mode();
        let _ = app.update_settings(SettingsMsg::RequestQcomDriverMode(QcomDriverMode::Kernel));
        // Kernel clamps to Userspace on a host without kernel support, so
        // only assert the no-op where Kernel is the effective request.
        if ltbox_device::driver::kernel_mode_supported() {
            assert_eq!(app.pending_qcom_driver_mode, None);
        }
        assert_eq!(app.qcom_driver_mode, QcomDriverMode::Kernel);
    }

    #[test]
    fn cancelling_leaves_the_stored_driver_mode_unchanged() {
        let mut app = app_in_kernel_mode();
        let _ = app.update_settings(SettingsMsg::RequestQcomDriverMode(
            QcomDriverMode::Userspace,
        ));
        let _ = app.update_settings(SettingsMsg::CancelQcomDriverMode);
        assert_eq!(app.pending_qcom_driver_mode, None);
        assert_eq!(app.qcom_driver_mode, QcomDriverMode::Kernel);
    }

    #[test]
    fn confirming_runs_the_switch_and_closes_the_dialog() {
        let previous = ltbox_device::driver::qcom_driver_mode();
        let mut app = app_in_kernel_mode();
        let _ = app.update_settings(SettingsMsg::RequestQcomDriverMode(
            QcomDriverMode::Userspace,
        ));
        let _ = app.update_settings(SettingsMsg::SetQcomDriverMode(QcomDriverMode::Userspace));
        // The switch sets a process-wide mode; put it back for other tests.
        ltbox_device::driver::set_qcom_driver_mode(previous);
        assert_eq!(app.pending_qcom_driver_mode, None);
        assert_eq!(app.qcom_driver_mode, QcomDriverMode::Userspace);
    }

    #[test]
    fn a_refused_switch_still_closes_the_dialog() {
        let mut app = app_in_kernel_mode();
        let _ = app.update_settings(SettingsMsg::RequestQcomDriverMode(
            QcomDriverMode::Userspace,
        ));
        app.operation.start(None, OperationKind::Cleanup, None);
        let _ = app.update_settings(SettingsMsg::SetQcomDriverMode(QcomDriverMode::Userspace));
        assert_eq!(app.pending_qcom_driver_mode, None);
        assert_eq!(app.qcom_driver_mode, QcomDriverMode::Kernel);
    }

    #[test]
    fn no_dialog_opens_while_an_operation_runs() {
        let mut app = app_in_kernel_mode();
        app.operation.start(None, OperationKind::Cleanup, None);
        let _ = app.update_settings(SettingsMsg::RequestQcomDriverMode(
            QcomDriverMode::Userspace,
        ));
        assert_eq!(app.pending_qcom_driver_mode, None);
    }
}
