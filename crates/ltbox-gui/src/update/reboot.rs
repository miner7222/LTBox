//! Reboot-view handler (target pick, confirm, EDL-loader dispatch). Extracted from `main.rs`.

use crate::*;
use iced::Task;

impl App {
    pub(crate) fn update_reboot(&mut self, msg: RebootMsg) -> Task<Message> {
        match msg {
            RebootMsg::RebootRequest(target) => {
                if self.operation.is_running() {
                    return Task::none();
                }
                if !target.available_from(self.device.connection) {
                    self.error_msg = Some(format!(
                        "{:?} not reachable from {:?}",
                        target, self.device.connection
                    ));
                    return Task::none();
                }
                self.reboot_confirm_target = Some(target);
                Task::none()
            }
            RebootMsg::RebootDismiss => {
                self.reboot_confirm_target = None;
                Task::none()
            }
            RebootMsg::RebootConfirm => {
                if let Some(t) = self.reboot_confirm_target.take() {
                    return self.update(Message::Reboot(RebootMsg::RebootTo(t)));
                }
                Task::none()
            }
            RebootMsg::RebootTo(target) => {
                if self.operation.is_running() {
                    return Task::none();
                }
                let conn = self.device.connection;
                if !target.available_from(conn) {
                    self.error_msg = Some(format!("{:?} not reachable from {:?}", target, conn));
                    return Task::none();
                }
                // EDL needs a Firehose loader before Power(reset).
                if matches!(conn, ConnectionStatus::Edl) {
                    return self.pick_loader(move |path| {
                        Message::Reboot(RebootMsg::RebootEdlWithLoader(target, path))
                    });
                }
                self.begin_op(View::Reboot);
                self.error_msg = None;
                self.log_push(format!(
                    "[Reboot] {}",
                    tr_args!(
                        "log_reboot_target_from",
                        target = self.t(target.label_key()),
                        source = self.t(self.connection_label_key())
                    ),
                ));
                let reboot_cmd_sent = self.t("log_reboot_command_sent").to_string();
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            reboot_worker(conn, target, reboot_cmd_sent)
                        })
                        .await
                        .unwrap_or_else(|_| Err(ltbox_core::i18n::tr("err_task_failed")))
                    },
                    |r| match r {
                        Ok(lines) => Message::Reboot(RebootMsg::RebootDone(lines)),
                        Err(e) => Message::OperationError(e),
                    },
                )
            }
            RebootMsg::RebootEdlWithLoader(target, path) => {
                let Some(loader_input) = path else {
                    self.log_push(format!(
                        "[Reboot] {}",
                        self.t("log_reboot_cancelled_no_loader")
                    ));
                    return Task::none();
                };
                // `resolve_loader_input` accepts loader files and, for older
                // recents, folders.
                let loader = match self.resolve_loader_input(&loader_input) {
                    Ok(p) => std::path::PathBuf::from(p),
                    Err(msg) => {
                        self.error_msg = Some(msg);
                        return Task::none();
                    }
                };
                if !loader.exists() {
                    self.error_msg = Some(tr_args!("err_loader_missing", path = loader.display()));
                    return Task::none();
                }
                self.start_edl_reboot_with_loader(target, loader)
            }
            RebootMsg::RebootDone(lines) => {
                self.end_op();
                self.flush_exec_done_log(lines);
                Task::none()
            }
        }
    }

    pub(crate) fn start_edl_reboot_with_loader(
        &mut self,
        target: RebootTarget,
        loader: std::path::PathBuf,
    ) -> Task<Message> {
        if !loader.exists() {
            self.error_msg = Some(tr_args!("err_loader_missing", path = loader.display()));
            return Task::none();
        }
        self.begin_op(View::Reboot);
        self.error_msg = None;
        self.log_push(format!(
            "[Reboot] {}",
            tr_args!(
                "log_reboot_target_from_edl",
                target = self.t(target.label_key()),
                loader = loader.display().to_string()
            ),
        ));
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || reboot_edl_with_loader_worker(loader, target))
                    .await
                    .unwrap_or_else(|_| Err(ltbox_core::i18n::tr("err_task_failed")))
            },
            |r| match r {
                Ok(lines) => Message::Reboot(RebootMsg::RebootDone(lines)),
                Err(e) => Message::OperationError(e),
            },
        )
    }
}
