//! Windows Software Fix process advisory; independent of USB polling.
use crate::*;
use iced::Task;
use ltbox_device::software_fix::CloseError;

#[derive(Default)]
pub(crate) struct State {
    pub running: bool,
    pub confirm_open: bool,
    pub checking: bool,
    pub closing: bool,
    pub error_key: Option<&'static str>,
}

impl App {
    pub(crate) fn poll_software_fix(&mut self) -> Task<Message> {
        if !cfg!(windows) || self.software_fix.checking {
            return Task::none();
        }
        #[cfg(feature = "demo")]
        if demo::is_active(self) {
            return Task::none();
        }
        self.software_fix.checking = true;
        Task::perform(
            async {
                tokio::task::spawn_blocking(ltbox_device::software_fix::is_running)
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()))
            },
            Message::SoftwareFixPolled,
        )
    }

    pub(crate) fn software_fix_polled(&mut self, result: Result<bool, String>) {
        self.software_fix.checking = false;
        match result {
            Ok(running) => {
                self.software_fix.running = running;
                if !running {
                    self.software_fix.error_key = None;
                    self.software_fix.confirm_open = false;
                }
            }
            // A failed check is not evidence that Software Fix has exited.
            Err(error) => tracing::warn!(%error, "failed to check Software Fix process"),
        }
    }

    pub(crate) fn can_close_software_fix(&self) -> bool {
        cfg!(windows)
            && self.software_fix.running
            && !self.software_fix.closing
            && !self.operation.is_running()
            && !self.installing_drivers
            && !self.operation.direct_update.is_active()
            && self.konabess.prepared.is_none()
    }

    pub(crate) fn force_close_software_fix(&mut self) -> Task<Message> {
        if self.can_close_software_fix() {
            self.software_fix.confirm_open = true;
        }
        Task::none()
    }

    pub(crate) fn confirm_close_software_fix(&mut self) -> Task<Message> {
        if !self.software_fix.confirm_open {
            return Task::none();
        }
        self.software_fix.confirm_open = false;
        if !self.can_close_software_fix() {
            return Task::none();
        }
        self.software_fix.closing = true;
        self.software_fix.error_key = None;
        Task::perform(
            async {
                tokio::task::spawn_blocking(ltbox_device::software_fix::force_close)
                    .await
                    .unwrap_or_else(|e| Err(CloseError::Failed(e.to_string())))
            },
            Message::SoftwareFixClosed,
        )
    }

    pub(crate) fn software_fix_closed(&mut self, result: Result<(), CloseError>) -> Task<Message> {
        self.software_fix.closing = false;
        match result {
            Ok(()) => {}
            Err(CloseError::Cancelled) => {
                self.software_fix.error_key = Some("software_fix_close_cancelled")
            }
            Err(CloseError::Failed(error)) => {
                tracing::warn!(%error, "failed to close Software Fix");
                self.software_fix.error_key = Some("software_fix_close_failed");
            }
        }
        // Never dismiss optimistically: the process monitor owns visibility.
        self.resume_after_device_poll()
            .chain(Task::done(Message::PollSoftwareFix))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn banner_stays_until_a_successful_absent_process_check() {
        let mut app = App::default();
        app.software_fix_polled(Ok(true));
        assert!(app.software_fix.running);
        app.software_fix_polled(Err("enumeration failed".into()));
        assert!(app.software_fix.running);
        let _ = app.software_fix_closed(Ok(()));
        assert!(app.software_fix.running);
        app.software_fix_polled(Ok(false));
        assert!(!app.software_fix.running);
        assert!(app.software_fix.error_key.is_none());
    }
    #[test]
    fn cancelled_or_failed_close_keeps_the_banner_and_allows_retry() {
        let mut app = App::default();
        app.software_fix.running = true;
        app.software_fix.closing = true;
        let _ = app.software_fix_closed(Err(CloseError::Cancelled));
        assert!(app.software_fix.running);
        assert!(!app.software_fix.closing);
        assert_eq!(
            app.software_fix.error_key,
            Some("software_fix_close_cancelled")
        );
        let _ = app.software_fix_closed(Err(CloseError::Failed("denied".into())));
        assert!(app.software_fix.running);
        assert_eq!(
            app.software_fix.error_key,
            Some("software_fix_close_failed")
        );
    }
    #[test]
    fn force_close_is_disabled_during_device_work_or_another_close() {
        let mut app = App::default();
        app.software_fix.running = true;
        app.begin_silent_op(View::Root);
        assert!(!app.can_close_software_fix());
        assert_eq!(app.force_close_software_fix().units(), 0);
        app.end_silent_op();
        app.software_fix.closing = true;
        assert!(!app.can_close_software_fix());
        app.software_fix.closing = false;
        app.installing_drivers = true;
        assert!(!app.can_close_software_fix());
    }
    #[test]
    fn closing_software_fix_allows_navigation_but_defers_device_work() {
        let mut app = App::default();
        app.software_fix.running = true;
        app.software_fix.closing = true;
        let _ = app.update(Message::Navigate(View::Root));
        let _ = app.update(Message::Root(RootMsg::RootExecStart));
        assert_eq!(app.current_view, View::Root);
        assert_eq!(app.queries.poll_deferred.len(), 1);
        assert_eq!(app.update(Message::PollDevice).units(), 0);
        let _ = app.update(Message::SoftwareFixClosed(Err(CloseError::Cancelled)));
        assert_eq!(app.current_view, View::Root);
        assert!(app.queries.poll_deferred.is_empty());
        assert!(app.software_fix.running);
    }

    #[cfg(windows)]
    #[test]
    fn process_checks_do_not_wait_for_device_operations_and_cannot_overlap() {
        let mut app = App {
            operation: OperationExecution::fixture(true, None, Vec::new(), 0, None),
            ..App::default()
        };
        assert!(app.update(Message::PollSoftwareFix).units() > 0);
        assert!(app.software_fix.checking);
        assert_eq!(app.update(Message::PollSoftwareFix).units(), 0);
        let _ = app.update(Message::SoftwareFixPolled(Ok(true)));
        assert!(!app.software_fix.checking);
        assert!(app.software_fix.running);
        assert!(app.operation.is_running());
    }
    #[cfg(windows)]
    #[test]
    fn close_requires_confirmation_and_cancel_does_not_start_a_worker() {
        let mut app = App::default();
        app.software_fix.running = true;
        assert_eq!(app.confirm_close_software_fix().units(), 0);
        assert_eq!(app.update(Message::ForceCloseSoftwareFix).units(), 0);
        assert!(app.software_fix.confirm_open);
        assert!(!app.software_fix.closing);
        let _ = app.update(Message::CancelCloseSoftwareFix);
        assert!(!app.software_fix.confirm_open);
        assert_eq!(app.confirm_close_software_fix().units(), 0);
        let _ = app.update(Message::ForceCloseSoftwareFix);
        let task = app.update(Message::ConfirmCloseSoftwareFix);
        assert!(task.units() > 0);
        assert!(app.software_fix.closing);
        assert!(!app.software_fix.confirm_open);
        // Drop the task without executing any process or UAC request.
        drop(task);
    }
}
