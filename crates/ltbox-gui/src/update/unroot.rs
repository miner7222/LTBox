//! Unroot-wizard handler. Extracted from `main.rs`.

use crate::*;
use iced::Task;

impl App {
    fn refresh_unroot_backups(&mut self) {
        match backup::root_backup_folders() {
            Ok(folders) => {
                self.unroot.backup_folders = folders;
                self.unroot.backup_scan_error = None;
            }
            Err(error) => {
                self.unroot.backup_folders.clear();
                self.unroot.backup_scan_error = Some(error);
            }
        }
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
                }
                Task::none()
            }
            UnrootMsg::UnrootBackupManifestOpen(path) => {
                let folder = std::path::PathBuf::from(path);
                let result = backup::read_backup_manifest(&folder);
                self.unroot.backup_manifest_dialog =
                    Some(backup::BackupManifestDialog { folder, result });
                Task::none()
            }
            UnrootMsg::UnrootBackupManifestClose => {
                self.unroot.backup_manifest_dialog = None;
                Task::none()
            }
            UnrootMsg::UnrootSelectLoader => self.pick_loader_with_default(|__v| {
                Message::Unroot(UnrootMsg::UnrootLoaderChosen(__v))
            }),
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
                // If we just advanced onto the loader step and a
                // Settings-level default loader is configured + still
                // on disk, pre-fill it + skip straight to the folder
                // step — matches the Root wizard's loader-skip pattern
                // (see `RootNext` step-5 fill + advance).
                if self.unroot.step == 1
                    && self.unroot.loader_path.is_none()
                    && let Some(path) = self.resolved_default_loader()
                {
                    self.unroot.loader_path = Some(path);
                    self.unroot.next();
                }
                if self.unroot.step == 2 {
                    self.refresh_unroot_backups();
                }
                Task::none()
            }
            UnrootMsg::UnrootBack => {
                self.unroot.back();
                Task::none()
            }
            UnrootMsg::UnrootExecStart => {
                if !ltbox_core::model::capabilities(&self.device.model).unroot {
                    self.error_msg =
                        Some(tr_args!("model_unsupported", model = "TB376FC / TB390FU"));
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
                // Loader is decoupled from the backup folder — `folder`
                // holds boot.img + vbmeta.img, the loader can live
                // anywhere (Settings default, or whatever the user
                // pointed the loader picker at). `validate_loader_path`
                // surfaces a missing-file error before the device-side
                // work starts, matching the other wizards' behaviour.
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
