//! Unroot-wizard handler. Extracted from `main.rs`.

use crate::*;
use iced::Task;

impl App {
    fn refresh_unroot_backups(&mut self) -> Task<Message> {
        let request = std::time::Instant::now();
        self.unroot.backup_scan_request = Some(request);
        task_heavy(
            backup::root_backup_folders,
            move |result| Message::Unroot(UnrootMsg::BackupsLoaded(request, result)),
            Err,
        )
    }

    pub(crate) fn update_unroot(&mut self, msg: UnrootMsg) -> Task<Message> {
        match msg {
            UnrootMsg::SetUnrootType(t) => {
                if !ltbox_core::model::capabilities(&self.device.model).unroot {
                    return Task::none();
                }
                self.unroot.unroot_type = Some(t);
                Task::none()
            }
            UnrootMsg::UnrootSelectFolder => {
                self.picker_target = PickerTarget::UnrootFolder;
                pickers::pick_folder_for(
                    pickers::PickerKind::QfilFirmwareFolder,
                    &self.recent_paths,
                    Message::FolderSelected,
                )
            }
            UnrootMsg::UnrootBackupPicked(path) => {
                if std::path::Path::new(&path).is_dir() {
                    self.unroot.folder_path = Some(path);
                    self.unroot.selected_manifest = None;
                }
                Task::none()
            }
            UnrootMsg::UnrootBackupManifestOpen(path) => {
                let folder = std::path::PathBuf::from(path);
                let request = std::time::Instant::now();
                self.unroot.backup_manifest_request = Some(request);
                self.unroot.backup_manifest_dialog = None;
                let failed_folder = folder.clone();
                task_heavy(
                    move || backup::BackupManifestDialog {
                        result: backup::read_backup_manifest(&folder),
                        folder,
                    },
                    move |dialog| Message::Unroot(UnrootMsg::ManifestLoaded(request, dialog)),
                    move |error| backup::BackupManifestDialog {
                        folder: failed_folder,
                        result: Err(error),
                    },
                )
            }
            UnrootMsg::UnrootBackupManifestClose => {
                self.unroot.backup_manifest_request = None;
                self.unroot.backup_manifest_dialog = None;
                Task::none()
            }
            UnrootMsg::BackupsLoaded(request, result) => {
                if self.unroot.backup_scan_request != Some(request) {
                    return Task::none();
                }
                self.unroot.backup_scan_request = None;
                match result {
                    Ok(folders) => {
                        self.unroot.backup_folders = folders;
                        self.unroot.backup_scan_error = None;
                    }
                    Err(error) => {
                        self.unroot.backup_folders.clear();
                        self.unroot.backup_scan_error = Some(error);
                    }
                }
                Task::none()
            }
            UnrootMsg::ManifestLoaded(request, dialog) => {
                if self.unroot.backup_manifest_request == Some(request) {
                    self.unroot.backup_manifest_request = None;
                    self.unroot.backup_manifest_dialog = Some(dialog);
                }
                Task::none()
            }
            UnrootMsg::UnrootSelectLoader => {
                self.pick_loader(|__v| Message::Unroot(UnrootMsg::UnrootLoaderChosen(__v)))
            }
            UnrootMsg::UnrootLoaderChosen(path) => {
                self.apply_loader_pick(path, |app, loader, err| {
                    app.unroot.loader_path = loader;
                    app.unroot.loader_error = err;
                });
                Task::none()
            }
            UnrootMsg::UnrootNext => {
                if self.unroot.step == 3 {
                    self.unroot.next();
                    return self.update(Message::Unroot(UnrootMsg::UnrootExecStart));
                }
                self.unroot.next();
                // Every route that picks a folder (list, browse, recent) passes
                // through this Next, so the manifest is read here, once, for
                // the folder the confirm step will show.
                self.unroot.selected_manifest = (self.unroot.step == 3)
                    .then_some(self.unroot.folder_path.as_deref())
                    .flatten()
                    .map(|folder| backup::read_backup_manifest(std::path::Path::new(folder)));
                // On entering the loader step with a usable remembered loader,
                // pre-fill it and skip to the folder step (as `RootNext` does at
                // step 5). Back returns to the skipped step; `is_none()` keeps a
                // loader picked there from being overwritten.
                if self.unroot.step == 1
                    && self.unroot.loader_path.is_none()
                    && let Some(path) = self.remembered_loader_for_model()
                {
                    self.unroot.loader_path = Some(path);
                    self.unroot.next();
                }
                if self.unroot.step == 2 {
                    return self.refresh_unroot_backups();
                }
                Task::none()
            }
            UnrootMsg::UnrootBack => {
                self.unroot.back();
                self.unroot.selected_manifest = None;
                Task::none()
            }
            UnrootMsg::UnrootExecStart => {
                if !unroot_connection_ready(self.device.connection) {
                    self.error_msg = Some(self.t("err_unroot_connection_required").to_string());
                    return Task::none();
                }
                if !ltbox_core::model::capabilities(&self.device.model).unroot {
                    self.error_msg = Some(tr_args!(
                        "model_unsupported",
                        model = self.device.model.as_str()
                    ));
                    return Task::none();
                }
                let Some(unroot_type) = self.unroot.unroot_type else {
                    return Task::none();
                };
                let Some(folder) = self.unroot.folder_path.clone() else {
                    return Task::none();
                };
                let conn = self.device.connection;
                let device_model = self.device.model.clone();
                // The loader is independent of the backup folder (`folder` holds
                // boot.img + vbmeta.img). `validate_loader_path` reports a missing
                // file before any device work starts.
                let loader_override =
                    match self.validate_loader_path(&self.unroot.loader_path.clone()) {
                        Ok(p) => Some(p),
                        Err(()) => return Task::none(),
                    };
                let phases = self.begin_phased_op(View::Unroot, OperationPhaseKind::Unroot);
                self.error_msg = None;
                self.log_push(format!(
                    "[Unroot] {}",
                    tr_args!("log_op_starting", what = self.t(unroot_type.label_key()))
                ));
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            ltbox_core::runtime::run_heavy(move || {
                                unroot_worker(
                                    folder,
                                    unroot_type,
                                    loader_override,
                                    device_model,
                                    conn,
                                    phases,
                                )
                            })
                            .and_then(|r| r)
                        })
                        .await
                        .unwrap_or_else(|_| Err(ltbox_core::i18n::tr("err_task_failed")))
                    },
                    |result| match result {
                        Ok(lines) => Message::Unroot(UnrootMsg::UnrootExecDone(lines)),
                        Err(e) => Message::OperationError(e),
                    },
                )
            }
            UnrootMsg::UnrootExecDone(lines) => {
                self.flush_exec_done_log(lines);
                self.end_op();
                Task::none()
            }
        }
    }
}

#[cfg(test)]
mod async_backup_tests {
    use super::*;
    #[test]
    fn stale_manifest_completion_does_not_reopen_closed_dialog() {
        let mut app = App::default();
        let request = std::time::Instant::now();
        app.unroot.backup_manifest_request = Some(request);
        drop(app.update_unroot(UnrootMsg::UnrootBackupManifestClose));
        drop(app.update_unroot(UnrootMsg::ManifestLoaded(
            request,
            backup::BackupManifestDialog {
                folder: "old".into(),
                result: Err("old error".into()),
            },
        )));
        assert!(app.unroot.backup_manifest_dialog.is_none());
    }
    #[test]
    fn stale_scan_completion_cannot_replace_a_newer_request() {
        let mut app = App::default();
        let old = std::time::Instant::now();
        app.unroot.backup_scan_request = Some(old + std::time::Duration::from_secs(1));
        drop(app.update_unroot(UnrootMsg::BackupsLoaded(old, Err("stale".into()))));
        assert!(app.unroot.backup_scan_error.is_none());
        assert!(app.unroot.backup_scan_request.is_some());
    }
}
