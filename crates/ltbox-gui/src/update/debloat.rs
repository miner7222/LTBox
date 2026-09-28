//! Debloat wizard handler: pick remove/restore, pick apps, run over ADB.

use crate::*;
use iced::Task;
use ltbox_core::tr_args;

impl App {
    /// The bundled list for the connected model, if LTBox ships one.
    pub(crate) fn debloat_list(&self) -> Option<&'static crate::debloat::DebloatList> {
        crate::debloat::list_for_model(&self.device.model)
    }

    pub(crate) fn update_debloat(&mut self, msg: DebloatMsg) -> Task<Message> {
        match msg {
            DebloatMsg::Action(action) => {
                if self.debloat_list().is_some() && self.debloat.action != Some(action) {
                    self.debloat.action = Some(action);
                    // The preset depends on the action (and on the states
                    // read for it), so a new action starts a fresh pick.
                    self.debloat.seeded = false;
                }
                Task::none()
            }
            DebloatMsg::Next => {
                let Some(list) = self.debloat_list() else {
                    return Task::none();
                };
                if !self.debloat.can_next() {
                    return Task::none();
                }
                // Action(0) → Apps(1) → Confirm(2) → Exec(3).
                if self.debloat.step == 2 {
                    self.debloat.next();
                    return self.update(Message::Debloat(DebloatMsg::ExecStart));
                }
                if self.debloat.step == 0 {
                    // Read the device first, so the Apps step only offers
                    // apps the chosen action would change.
                    if self.operation.is_running() {
                        return Task::none();
                    }
                    let conn = self.device.connection;
                    let ids = list.packages.iter().map(|p| p.id.clone()).collect();
                    self.begin_op(View::Debloat);
                    return task_heavy(
                        move || debloat_scan(conn, ids),
                        |result| Message::Debloat(DebloatMsg::ScanDone(result)),
                        |error| DebloatScanResult {
                            logs: Vec::new(),
                            states: None,
                            error: Some(error),
                        },
                    );
                }
                self.debloat.next();
                Task::none()
            }
            DebloatMsg::ScanDone(result) => {
                self.flush_exec_done_log(result.logs);
                self.end_op();
                let Some(list) = self.debloat_list() else {
                    return Task::none();
                };
                if self.debloat.step != 0 {
                    return Task::none();
                }
                self.debloat.states = result.states;
                self.debloat.scan_error = result.error;
                self.debloat.seeded = false;
                self.debloat.seed(list);
                self.debloat.next();
                Task::none()
            }
            DebloatMsg::Back => {
                self.debloat.back();
                Task::none()
            }
            DebloatMsg::Toggle(id) => {
                if let Some(list) = self.debloat_list() {
                    self.debloat.toggle(list, &id);
                }
                Task::none()
            }
            DebloatMsg::Preset(preset) => {
                if let Some(list) = self.debloat_list() {
                    self.debloat.apply_preset(list, preset);
                }
                Task::none()
            }
            DebloatMsg::ExecStart => {
                let (Some(action), Some(list)) = (self.debloat.action, self.debloat_list()) else {
                    return Task::none();
                };
                let targets = self.debloat.selected_targets(list);
                if targets.is_empty() {
                    return Task::none();
                }
                let conn = self.device.connection;
                let phase_kind = match action {
                    DebloatAction::Remove => OperationPhaseKind::DebloatRemove,
                    DebloatAction::Restore => OperationPhaseKind::DebloatRestore,
                };
                let phases = self.begin_phased_op(View::Debloat, phase_kind);
                self.error_msg = None;
                self.log_push(format!(
                    "[Debloat] {}",
                    tr_args!(
                        "log_debloat_starting",
                        action = self.t(action.label_key()),
                        count = targets.len()
                    )
                ));
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            debloat_worker(action, targets, conn, phases)
                        })
                        .await
                        .unwrap_or_else(|_| Err(ltbox_core::i18n::tr("err_task_failed")))
                    },
                    |result| match result {
                        Ok(lines) => Message::Debloat(DebloatMsg::ExecDone(lines)),
                        Err(e) => Message::OperationError(e),
                    },
                )
            }
            DebloatMsg::ExecDone(lines) => {
                self.flush_exec_done_log(lines);
                self.end_op();
                Task::none()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debloat_needs_a_bundled_list_for_the_connected_model() {
        let mut app = App::default();
        app.device.model = "TB320FC".into();
        let _ = app.update_debloat(DebloatMsg::Action(DebloatAction::Remove));
        assert_eq!(app.debloat.action, None);
    }

    fn scanned(app: &App, state: crate::debloat::PackageState) -> DebloatScanResult {
        DebloatScanResult {
            logs: Vec::new(),
            states: Some(
                app.debloat_list()
                    .unwrap()
                    .packages
                    .iter()
                    .map(|p| (p.id.clone(), state))
                    .collect(),
            ),
            error: None,
        }
    }

    #[test]
    fn apps_step_opens_after_reading_the_device() {
        use crate::debloat::PackageState;
        let mut app = App::default();
        app.device.model = "TB321FU".into();
        let _ = app.update_debloat(DebloatMsg::Action(DebloatAction::Remove));
        let _ = app.update_debloat(DebloatMsg::Next);
        assert_eq!(
            app.debloat.step, 0,
            "the Apps step waits for the device read"
        );
        assert!(app.operation.is_running());

        let result = scanned(&app, PackageState::Installed);
        let _ = app.update_debloat(DebloatMsg::ScanDone(result));
        assert_eq!(app.debloat.step, 1);
        assert!(!app.operation.is_running());
        let recommended = app
            .debloat_list()
            .unwrap()
            .packages
            .iter()
            .filter(|p| p.recommended)
            .count();
        assert_eq!(app.debloat.selected.len(), recommended);

        let _ = app.update_debloat(DebloatMsg::Preset(DebloatPreset::None));
        let _ = app.update_debloat(DebloatMsg::Next);
        assert_eq!(app.debloat.step, 1, "an empty selection cannot advance");
    }

    #[test]
    fn nothing_is_preselected_when_every_app_is_already_removed() {
        use crate::debloat::PackageState;
        let mut app = App::default();
        app.device.model = "TB321FU".into();
        let _ = app.update_debloat(DebloatMsg::Action(DebloatAction::Remove));
        let _ = app.update_debloat(DebloatMsg::Next);
        let result = scanned(&app, PackageState::Removed);
        let _ = app.update_debloat(DebloatMsg::ScanDone(result));
        assert_eq!(app.debloat.step, 1);
        assert!(app.debloat.selected.is_empty());
        assert!(!app.debloat.can_next());
    }
}
