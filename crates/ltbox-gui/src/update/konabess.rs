//! KonaBess wizard handlers for stock-image inspection, target selection, and
//! the irreversible rebuild/flash continuation.

use crate::*;
use iced::Task;
use ltbox_core::tr_args;

impl App {
    pub(crate) fn update_konabess(&mut self, msg: KonaBessMsg) -> Task<Message> {
        if !ltbox_core::model::capabilities(&self.device.model).konabess
            && matches!(
                &msg,
                KonaBessMsg::KonaBessSelectLoader | KonaBessMsg::KonaBessNext
            )
        {
            self.error_msg = Some(tr_args!("model_unsupported", model = "TB376FC / TB390FU"));
            return Task::none();
        }
        match msg {
            KonaBessMsg::KonaBessSelectLoader => self.pick_loader_with_default(|path| {
                Message::KonaBess(KonaBessMsg::KonaBessLoaderChosen(path))
            }),
            KonaBessMsg::KonaBessLoaderChosen(path) => {
                self.apply_loader_pick(path, |app, loader, err| {
                    // KonaBess additionally refuses a loader whose kind does not
                    // match the connected model.
                    match loader {
                        Some(l) if !app.loader_fits_model(std::path::Path::new(&l)) => {
                            app.konabess.loader_path = None;
                            app.konabess.loader_error =
                                Some(app.t("loader_model_mismatch_tooltip").to_string());
                        }
                        other => {
                            app.konabess.loader_path = other;
                            app.konabess.loader_error = err;
                        }
                    }
                });
                Task::none()
            }
            KonaBessMsg::KonaBessSelectImport => pickers::pick_file_for(
                pickers::FilePickSpec::single().with_filter(
                    self.t("picker_target_konabess_export").to_string(),
                    &["txt"],
                ),
                &self.recent_paths,
                |path| Message::KonaBess(KonaBessMsg::KonaBessImportChosen(path)),
            ),
            KonaBessMsg::KonaBessImportChosen(path) => {
                if let Some(path) = path {
                    if std::path::Path::new(&path).is_file() {
                        self.remember_recent(pickers::PickerKind::File, &path);
                    }
                    match ltbox_patch::konabess::read_export(std::path::Path::new(&path)) {
                        Ok(export) => match self.konabess.overwrite_edited_from_import(export) {
                            Ok(()) => {
                                self.konabess.import_path = Some(path);
                                self.konabess.import_error = None;
                            }
                            Err(KonaBessImportError::ChipMismatch { expected, actual }) => {
                                self.konabess.import_error = Some(tr_args!(
                                    "konabess_import_chip_mismatch",
                                    expected = expected,
                                    actual = actual
                                ));
                            }
                            Err(KonaBessImportError::NoTarget) => {
                                self.konabess.import_error =
                                    Some(self.t("konabess_import_no_target").to_string());
                            }
                            Err(KonaBessImportError::TargetChipUnknown) => {
                                self.konabess.import_error =
                                    Some(self.t("konabess_import_unknown_chip").to_string());
                            }
                        },
                        Err(error) => {
                            self.konabess.import_error = Some(tr_args!(
                                "konabess_export_invalid",
                                error = error.to_string()
                            ));
                        }
                    }
                }
                Task::none()
            }
            KonaBessMsg::KonaBessOpenTarget => {
                self.konabess.open_target_popup();
                Task::none()
            }
            KonaBessMsg::KonaBessRevertEdits => {
                self.konabess.revert_edits();
                Task::none()
            }
            KonaBessMsg::KonaBessCellChanged(key, value) => {
                self.konabess.edit_cell(key, value);
                Task::none()
            }
            KonaBessMsg::KonaBessAddLevel(group) => {
                self.konabess.add_level(group);
                Task::none()
            }
            KonaBessMsg::KonaBessRemoveLevel(group, level) => {
                self.konabess.remove_level(group, level);
                Task::none()
            }
            KonaBessMsg::KonaBessNext => {
                match self.konabess.step {
                    0 => {
                        let selected = self.konabess.loader_path.clone();
                        match self.validate_loader_path(&selected) {
                            Ok(loader) if self.loader_fits_model(std::path::Path::new(&loader)) => {
                                self.konabess.loader_error = None;
                                if self.operation.is_running() {
                                    return Task::none();
                                }
                                self.konabess.cleanup_prepared();
                                let phases = self
                                    .begin_phased_op(View::KonaBess, OperationPhaseKind::KonaBess);
                                let conn = self.device.connection;
                                let uses_gbl = ltbox_core::model::capabilities(&self.device.model)
                                    .root_uses_gbl;
                                let device_model = self.device.model.clone();
                                let ll = self.live_labels();
                                let loader = std::path::PathBuf::from(loader);
                                return Task::perform(
                                    async move {
                                        tokio::task::spawn_blocking(move || {
                                            ltbox_core::runtime::run_heavy(move || {
                                                konabess_inspection_worker(
                                                    conn,
                                                    loader,
                                                    uses_gbl,
                                                    device_model,
                                                    ll,
                                                    phases,
                                                )
                                            })
                                            .and_then(|result| result)
                                        })
                                        .await
                                        .unwrap_or_else(
                                            |_| Err(ltbox_core::i18n::tr("err_task_failed")),
                                        )
                                    },
                                    |result| match result {
                                        Ok(result) => Message::KonaBess(
                                            KonaBessMsg::KonaBessInspectionReady(result),
                                        ),
                                        Err(error) => Message::KonaBess(
                                            KonaBessMsg::KonaBessInspectionFailed(error),
                                        ),
                                    },
                                );
                            }
                            Ok(_) => {
                                self.error_msg = None;
                                self.konabess.loader_error =
                                    Some(self.t("loader_model_mismatch_tooltip").to_string());
                            }
                            Err(()) => {
                                self.konabess.loader_error = self.error_msg.take();
                            }
                        }
                    }
                    1 if self.konabess.can_next() => self.konabess.next(),
                    2 if self.konabess.can_next() => {
                        if self.operation.is_running() {
                            return Task::none();
                        }
                        let Some(loader) = self.konabess.loader_path.clone() else {
                            return Task::none();
                        };
                        if self.validate_loader_path(&Some(loader.clone())).is_err() {
                            return Task::none();
                        }
                        let Some(prepared) = self.konabess.prepared.clone() else {
                            return Task::none();
                        };
                        let Some(target_index) = self.konabess.selected_target_index else {
                            return Task::none();
                        };
                        let Some(chip) = self.konabess.selected_chip().map(str::to_owned) else {
                            return Task::none();
                        };
                        let Some(table) = self.konabess.edited_table.clone() else {
                            return Task::none();
                        };
                        self.konabess.next();
                        let phases =
                            self.begin_phased_op(View::KonaBess, OperationPhaseKind::KonaBess);
                        return Task::perform(
                            async move {
                                tokio::task::spawn_blocking(move || {
                                    ltbox_core::runtime::run_heavy(move || {
                                        konabess_flash_worker(
                                            std::path::PathBuf::from(loader),
                                            prepared,
                                            target_index,
                                            chip,
                                            table,
                                            phases,
                                        )
                                    })
                                    .and_then(|result| result)
                                })
                                .await
                                .unwrap_or_else(|_| Err(ltbox_core::i18n::tr("err_task_failed")))
                            },
                            |result| match result {
                                Ok(log) => Message::KonaBess(KonaBessMsg::KonaBessFlashDone(log)),
                                Err(error) => Message::OperationError(error),
                            },
                        );
                    }
                    2 | 3 => {}
                    _ => {}
                }
                Task::none()
            }
            KonaBessMsg::KonaBessBack => {
                if self.konabess.step == 0 {
                    self.konabess.reset();
                } else if self.konabess.step == 1 && self.konabess.prepared.is_some() {
                    let _ = self.konabess.dismiss_target_popup();
                    return self.cancel_konabess_inspection();
                } else {
                    self.konabess.back();
                }
                Task::none()
            }
            KonaBessMsg::KonaBessInspectionReady(result) => {
                self.flush_exec_done_log(result.log);
                self.end_op();
                self.operation.set_completed_step(2);
                let probable_dtb_index = result.prepared.probable_dtb_index;
                self.konabess.prepared = Some(result.prepared);
                if self.operation.direct_update.is_active() {
                    // Defensive overlap: finish the existing inspection's EDL
                    // cleanup instead of retaining a table the update modal
                    // cannot let the user apply or cancel. CancelDone releases
                    // the busy reservation before the updater may exit.
                    return self.cancel_konabess_inspection();
                }
                self.konabess
                    .apply_inspection_result(result.candidates, probable_dtb_index);
                self.konabess.step = 1;
                Task::none()
            }
            KonaBessMsg::KonaBessInspectionFailed(error) => {
                self.konabess.cleanup_prepared();
                self.konabess.step = 0;
                self.update(Message::OperationError(error))
            }
            KonaBessMsg::KonaBessTargetSelected(index) => {
                self.konabess.select_target(index);
                Task::none()
            }
            KonaBessMsg::KonaBessTargetConfirm => {
                self.konabess.confirm_target();
                Task::none()
            }
            KonaBessMsg::KonaBessTargetDismiss => {
                if self.konabess.dismiss_target_popup() {
                    self.cancel_konabess_inspection()
                } else {
                    Task::none()
                }
            }
            KonaBessMsg::KonaBessCancelDone(log) => {
                self.flush_exec_done_log(log);
                self.end_silent_op();
                self.konabess.cleanup_prepared();
                self.konabess.step = 0;
                Task::none()
            }
            KonaBessMsg::KonaBessFlashDone(log) => {
                self.flush_exec_done_log(log);
                self.end_op();
                self.konabess.cleanup_prepared();
                Task::none()
            }
        }
    }

    fn cancel_konabess_inspection(&mut self) -> Task<Message> {
        let Some(loader) = self.konabess.loader_path.clone() else {
            self.konabess.cleanup_prepared();
            self.konabess.step = 0;
            return Task::none();
        };
        self.begin_silent_op(View::KonaBess);
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    ltbox_core::runtime::run_heavy(move || {
                        konabess_cancel_worker(std::path::PathBuf::from(loader))
                    })
                })
                .await
                .ok()
                .and_then(Result::ok)
                .unwrap_or_default()
            },
            |log| Message::KonaBess(KonaBessMsg::KonaBessCancelDone(log)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ltbox_patch::konabess::{
        GpuGroup, GpuLevel, GpuProperty, GpuTable, KonaBessExport, VendorBootDtbInfo,
    };

    const SUN_EXPORT: &str = "konabess://H4sIAAAAAAAACmWOywrCMBBFf6WM2wTiYxUf+CFu2mSsAzGmmUbF0n+XpEUUF7O551zmDmAuFEADJw8CLLIBDclTDwLOETvQ0JnbVbQhyfCIDu/oWKpqOPmSc0C0siFf7audejZ42M6EPPVUu09rEpaZL5heKA06x1OqSlpbG5H5GxT9b8Cx/I/YFulHyZvn6mq9yWgsB+MbZCgNFesAAAA=";
    const UNKNOWN_FIELD_EXPORT: &str = "konabess://H4sIAAAAAAAACmWPywrCMBBFf6WM2wbqYxUf+CGCtMm1BtKY5qFi6b9L0iKKi9nccy4zM5C4KkucfDRUkoQXxCkaFaiki0NPnHpx68rWRmYfTuMO7VlVDCeTc28ByRplin2xq54NDtuZKKOCqvWnNQnLxBdevcAEtPZTWuW0ltLB+2+Q9b8Djnm/Q5ulHyXdPFdX601CY570TgzR4dzdJIgPBFM3GpJ4cBHj+AbhUQm8CgEAAA==";

    fn table(frequency: u32) -> GpuTable {
        GpuTable {
            groups: vec![GpuGroup {
                id: 0,
                header_properties: vec![GpuProperty {
                    name: "qcom,speed-bin".into(),
                    cells: vec![0],
                }],
                levels: vec![GpuLevel {
                    id: 0,
                    properties: vec![
                        GpuProperty {
                            name: "reg".into(),
                            cells: vec![0],
                        },
                        GpuProperty {
                            name: "qcom,gpu-freq".into(),
                            cells: vec![frequency],
                        },
                        GpuProperty {
                            name: "qcom,level".into(),
                            cells: vec![200],
                        },
                    ],
                }],
            }],
        }
    }

    fn candidate(index: usize, chip: &str, frequency: u32) -> VendorBootDtbInfo {
        VendorBootDtbInfo {
            index,
            model: Some("test".into()),
            chip: Some(chip.into()),
            gpu_shape: None,
            table: Some(table(frequency)),
        }
    }

    fn app_ready_for_inspection_result() -> App {
        App {
            konabess: KonaBessWizard {
                step: 0,
                loader_path: Some("loader.melf".into()),
                ..KonaBessWizard::default()
            },
            ..App::default()
        }
    }

    fn prepared(root: &std::path::Path, probable_dtb_index: Option<usize>) -> KonaBessPrepared {
        let work_dir = root.join("work");
        std::fs::create_dir_all(&work_dir).unwrap();
        KonaBessPrepared {
            vendor_boot: work_dir.join("vendor_boot.img"),
            vbmeta: work_dir.join("vbmeta.img"),
            backup_dir: root.join("backup_konabess"),
            slot_suffix: "_b".into(),
            probable_dtb_index,
            work_dir,
        }
    }

    #[test]
    fn self_update_waits_for_late_inspection_cleanup_before_exiting() {
        let root = tempfile::tempdir().unwrap();
        let mut app = app_ready_for_inspection_result();
        let prepared = prepared(root.path(), None);
        let work_dir = prepared.work_dir.clone();
        app.begin_silent_op(View::KonaBess);
        app.operation.direct_update = DirectUpdateState::Updating;

        // Drop the cancellation task without contacting hardware. Its completion
        // is injected below, after the updater reports success.
        assert_eq!(
            app.update(Message::KonaBess(KonaBessMsg::KonaBessInspectionReady(
                KonaBessInspectionResult {
                    prepared,
                    candidates: vec![],
                    log: vec![]
                }
            )))
            .units(),
            1
        );
        assert!(app.operation.is_running());
        assert!(!app.konabess.target_popup_open);
        drop(app.update(Message::SelfUpdateFinished(Ok(()))));
        assert!(!app.can_exit_after_self_update());
        drop(app.update(Message::KonaBess(KonaBessMsg::KonaBessCancelDone(vec![]))));
        assert!(!app.operation.is_running());
        assert!(app.konabess.prepared.is_none());
        assert!(!work_dir.exists());
        assert!(app.can_exit_after_self_update());
    }

    #[test]
    fn inspection_result_enters_target_picker_without_preselection() {
        let root = tempfile::tempdir().unwrap();
        let mut app = app_ready_for_inspection_result();
        let prepared = prepared(root.path(), Some(4));

        let task = app.update_konabess(KonaBessMsg::KonaBessInspectionReady(
            KonaBessInspectionResult {
                prepared: prepared.clone(),
                candidates: vec![candidate(4, "sun", 700_000_000)],
                log: vec![],
            },
        ));

        assert!(app.konabess.target_popup_open);
        assert_eq!(app.konabess.candidates.len(), 1);
        assert!(app.konabess.is_probable_target(4));
        assert_eq!(app.konabess.selected_target_index, None);
        assert_eq!(app.konabess.prepared, Some(prepared));
        assert_eq!(app.konabess.stock_table, None);
        assert_eq!(app.konabess.edited_table, None);
        assert_eq!(app.konabess.step, 1);
        assert!(!app.operation.is_running());
        assert_eq!(task.units(), 0);
    }

    #[test]
    fn sidebar_round_trip_preserves_prepared_konabess_table_state() {
        let root = tempfile::tempdir().unwrap();
        let loader = root.path().join("loader.melf");
        std::fs::write(&loader, []).unwrap();
        let prepared = prepared(root.path(), Some(4));
        let work_dir = prepared.work_dir.clone();
        std::fs::write(&prepared.vendor_boot, [1]).unwrap();
        std::fs::write(&prepared.vbmeta, [2]).unwrap();
        let mut app = App {
            device: DeviceSnapshot {
                connection: ConnectionStatus::Edl,
                ..Default::default()
            },
            current_view: View::KonaBess,
            konabess: KonaBessWizard {
                step: 1,
                loader_path: Some(loader.display().to_string()),
                prepared: Some(prepared.clone()),
                ..KonaBessWizard::default()
            },
            ..App::default()
        };
        app.konabess.apply_inspection_result(
            vec![
                candidate(4, "sun", 700_000_000),
                candidate(7, "sun", 900_000_000),
            ],
            Some(4),
        );
        assert!(app.konabess.select_target(4));
        assert_eq!(app.konabess.confirm_target(), Some(4));
        app.konabess
            .overwrite_edited_from_import(KonaBessExport {
                chip: "sun".into(),
                description: "sidebar state".into(),
                table: table(800_000_000),
                import_warnings: vec![],
            })
            .unwrap();
        app.konabess.import_path = Some("imported-settings.txt".into());

        let expected_candidates = app.konabess.candidates.clone();
        let expected_stock = app.konabess.stock_table.clone();
        let expected_edited = app.konabess.edited_table.clone();

        assert_eq!(app.update(Message::Navigate(View::Dashboard)).units(), 0);
        assert_eq!(app.current_view, View::Dashboard);
        assert!(work_dir.exists());
        assert_eq!(app.update(Message::Navigate(View::KonaBess)).units(), 0);

        assert_eq!(app.current_view, View::KonaBess);
        assert_eq!(app.konabess.step, 1);
        assert_eq!(app.konabess.prepared, Some(prepared));
        assert_eq!(app.konabess.candidates, expected_candidates);
        assert_eq!(app.konabess.selected_target_index, Some(4));
        assert_eq!(app.konabess.stock_table, expected_stock);
        assert_eq!(app.konabess.edited_table, expected_edited);
        assert!(app.konabess.edited_dirty);
        assert_eq!(
            app.konabess.import_path.as_deref(),
            Some("imported-settings.txt")
        );
        assert!(work_dir.exists());
        assert!(
            app.konabess
                .prepared
                .as_ref()
                .is_some_and(|state| state.vendor_boot.exists() && state.vbmeta.exists())
        );
    }

    #[test]
    fn every_inspection_result_opens_existing_target_popup_without_selection() {
        let cases = [
            (None, vec![candidate(4, "sun", 700_000_000)], None),
            (Some(99), vec![candidate(4, "sun", 700_000_000)], None),
            (
                Some(4),
                vec![
                    candidate(4, "sun", 700_000_000),
                    candidate(4, "sun", 800_000_000),
                ],
                None,
            ),
        ];

        for (probable_dtb_index, candidates, expected_selection) in cases {
            let root = tempfile::tempdir().unwrap();
            let mut app = app_ready_for_inspection_result();
            let task = app.update_konabess(KonaBessMsg::KonaBessInspectionReady(
                KonaBessInspectionResult {
                    prepared: prepared(root.path(), probable_dtb_index),
                    candidates,
                    log: vec![],
                },
            ));

            assert!(app.konabess.target_popup_open);
            assert_eq!(app.konabess.selected_target_index, expected_selection);
            assert_eq!(app.konabess.step, 1);
            assert!(!app.operation.is_running());
            assert_eq!(task.units(), 0);
        }
    }

    #[test]
    fn imported_chip_mismatch_surfaces_error_without_changing_table() {
        let root = tempfile::tempdir().unwrap();
        let export_path = root.path().join("settings.txt");
        std::fs::write(&export_path, SUN_EXPORT).unwrap();
        let mut app = app_ready_for_inspection_result();
        app.konabess
            .apply_inspection_result(vec![candidate(4, "pineapple", 700_000_000)], Some(4));
        assert!(app.konabess.select_target(4));
        assert_eq!(app.konabess.confirm_target(), Some(4));
        let before = app.konabess.edited_table.clone();

        let task = app.update_konabess(KonaBessMsg::KonaBessImportChosen(Some(
            export_path.display().to_string(),
        )));

        let error = app.konabess.import_error.as_deref().unwrap();
        assert!(error.contains("pineapple"));
        assert!(error.contains("sun"));
        assert_eq!(app.konabess.edited_table, before);
        assert!(!app.konabess.edited_dirty);
        assert_eq!(task.units(), 0);
    }

    #[test]
    fn imported_unknown_field_is_accepted_and_surfaces_a_warning() {
        let root = tempfile::tempdir().unwrap();
        let export_path = root.path().join("settings.txt");
        std::fs::write(&export_path, UNKNOWN_FIELD_EXPORT).unwrap();
        let mut app = app_ready_for_inspection_result();
        app.konabess
            .apply_inspection_result(vec![candidate(4, "sun", 700_000_000)], Some(4));
        assert!(app.konabess.select_target(4));
        assert_eq!(app.konabess.confirm_target(), Some(4));

        let task = app.update_konabess(KonaBessMsg::KonaBessImportChosen(Some(
            export_path.display().to_string(),
        )));

        assert!(app.konabess.import_error.is_none());
        assert_eq!(
            app.konabess.import_path.as_deref(),
            Some(export_path.to_str().unwrap())
        );
        assert!(
            app.konabess
                .editor_validation()
                .warnings
                .iter()
                .any(|warning| {
                    warning.path == "export / future_mode"
                        && warning.message.contains("unknown export field")
                })
        );
        assert_eq!(task.units(), 0);
    }

    #[test]
    fn loader_next_starts_export_free_inspection_before_table_step() {
        let root = tempfile::tempdir().unwrap();
        let loader = root.path().join("loader.melf");
        std::fs::write(&loader, []).unwrap();
        let mut app = App {
            konabess: KonaBessWizard {
                loader_path: Some(loader.display().to_string()),
                ..KonaBessWizard::default()
            },
            ..App::default()
        };

        let task = app.update_konabess(KonaBessMsg::KonaBessNext);

        assert_eq!(task.units(), 1);
        assert!(app.operation.is_running());
        assert_eq!(app.konabess.step, 0);
        assert!(app.konabess.import_path.is_none());
        assert!(app.konabess.edited_table.is_none());
    }

    #[test]
    fn confirm_next_starts_in_memory_apply_step() {
        let root = tempfile::tempdir().unwrap();
        let loader = root.path().join("loader.melf");
        std::fs::write(&loader, []).unwrap();
        let prepared = prepared(root.path(), Some(4));
        let mut app = App {
            konabess: KonaBessWizard {
                loader_path: Some(loader.display().to_string()),
                prepared: Some(prepared),
                ..KonaBessWizard::default()
            },
            ..App::default()
        };
        app.konabess
            .apply_inspection_result(vec![candidate(4, "sun", 700_000_000)], Some(4));
        assert!(app.konabess.select_target(4));
        assert_eq!(app.konabess.confirm_target(), Some(4));
        app.konabess.step = 2;

        let task = app.update_konabess(KonaBessMsg::KonaBessNext);

        assert_eq!(task.units(), 1);
        assert_eq!(app.konabess.step, 3);
        assert!(app.operation.is_running());
    }

    #[test]
    fn wizard_transitions_table_confirm_apply_and_cleans_up_on_abandon() {
        let root = tempfile::tempdir().unwrap();
        let loader = root.path().join("loader.melf");
        std::fs::write(&loader, []).unwrap();
        let prepared = prepared(root.path(), Some(4));
        let work_dir = prepared.work_dir.clone();
        let mut app = App {
            konabess: KonaBessWizard {
                loader_path: Some(loader.display().to_string()),
                prepared: Some(prepared),
                ..KonaBessWizard::default()
            },
            ..App::default()
        };
        app.konabess
            .apply_inspection_result(vec![candidate(4, "sun", 700_000_000)], Some(4));
        app.konabess.step = 1;

        assert_eq!(app.update_konabess(KonaBessMsg::KonaBessNext).units(), 0);
        assert_eq!(app.konabess.step, 1);
        assert!(app.konabess.target_popup_open);

        assert_eq!(
            app.update_konabess(KonaBessMsg::KonaBessTargetSelected(4))
                .units(),
            0
        );
        assert_eq!(
            app.update_konabess(KonaBessMsg::KonaBessTargetConfirm)
                .units(),
            0
        );
        assert!(!app.konabess.target_popup_open);
        assert_eq!(app.update_konabess(KonaBessMsg::KonaBessNext).units(), 0);
        assert_eq!(app.konabess.step, 2);
        assert_eq!(app.update_konabess(KonaBessMsg::KonaBessBack).units(), 0);
        assert_eq!(app.konabess.step, 1);

        let cancel = app.update_konabess(KonaBessMsg::KonaBessBack);
        assert_eq!(cancel.units(), 1);
        assert!(app.operation.is_running());
        let _ = app.update_konabess(KonaBessMsg::KonaBessCancelDone(vec![]));
        assert_eq!(app.konabess.step, 0);
        assert!(app.konabess.prepared.is_none());
        assert!(!work_dir.exists());
    }

    #[test]
    fn flash_completion_cleans_prepared_workspace() {
        let root = tempfile::tempdir().unwrap();
        let prepared = prepared(root.path(), Some(4));
        let work_dir = prepared.work_dir.clone();
        let mut app = App {
            konabess: KonaBessWizard {
                step: 3,
                prepared: Some(prepared),
                candidates: vec![candidate(4, "sun", 700_000_000)],
                ..KonaBessWizard::default()
            },
            ..App::default()
        };
        assert!(app.konabess.select_target(4));

        let task = app.update_konabess(KonaBessMsg::KonaBessFlashDone(vec![]));

        assert_eq!(task.units(), 0);
        assert!(app.konabess.prepared.is_none());
        assert!(app.konabess.candidates.is_empty());
        assert!(app.konabess.stock_table.is_none());
        assert!(app.konabess.edited_table.is_none());
        assert!(!work_dir.exists());
    }
}
