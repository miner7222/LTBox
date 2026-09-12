//! Window-chrome handlers: titlebar buttons, cursor-drag resize, and
//! debounced geometry persistence. Extracted from `main.rs`.
use crate::*;
use iced::Task;

impl App {
    /// Dispatch a `WindowMsg` — titlebar drag / minimize / maximize /
    /// close / cursor-drag resize. All variants except
    /// `WindowIdReceived` are gated on `self.window_id` being set;
    /// before iced delivers the window id those calls would be
    /// no-ops anyway.
    pub(crate) fn update_window(&mut self, msg: WindowMsg) -> Task<Message> {
        match msg {
            WindowMsg::WindowIdReceived(id) => {
                // A startup query can finish before the native window opens.
                // Do not let that empty reply erase the Opened event's ID.
                self.window_id = id.or(self.window_id);
                self.window_id
                    .map(|id| iced::window::is_maximized(id).map(Message::WindowMaximized))
                    .unwrap_or_else(Task::none)
            }
            WindowMsg::WindowDrag => self
                .window_id
                .map(iced::window::drag)
                .unwrap_or_else(Task::none),
            WindowMsg::WindowMinimize => self
                .window_id
                .map(|id| iced::window::minimize(id, true))
                .unwrap_or_else(Task::none),
            WindowMsg::WindowToggleMaximize => self
                .window_id
                .map(|id| {
                    let maximized = !self.window_maximized;
                    self.window_maximized = maximized;
                    iced::window::maximize(id, maximized)
                })
                .unwrap_or_else(Task::none),
            WindowMsg::WindowClose => {
                // Exiting a process while its updater may be between filesystem
                // operations would defeat the rollback guarantee. Native close
                // requests and the custom titlebar both route through here.
                if self.operation.direct_update.is_active() {
                    return Task::none();
                }
                // Closing while a flash/root/probe (or any other busy op)
                // is live would tear down the process mid-work. Refuse and
                // surface the same "X is in progress" wording the progress
                // dialog uses — no new locale keys required.
                if self.operation.is_running() {
                    let op_name = self.busy_operation_label();
                    self.error_msg = Some(ltbox_core::tr_args!(
                        "progress_dialog_body",
                        operation = op_name
                    ));
                    return Task::none();
                }
                self.konabess.cleanup_prepared();
                self.window_id
                    .map(iced::window::close)
                    .unwrap_or_else(Task::none)
            }
            WindowMsg::WindowResize(direction) => self
                .window_id
                .map(|id| iced::window::drag_resize(id, direction))
                .unwrap_or_else(Task::none),
        }
    }

    /// Cursor-drag resize / maximize / restore funnel through here.
    /// Snap the persisted size to the `MIN_WINDOW_*` floor so a
    /// maximize → store → relaunch sequence still launches at a usable
    /// geometry rather than below the layout floor.
    pub(crate) fn update_window_resized(&mut self, w: f32, h: f32) -> Task<Message> {
        let w = w.max(MIN_WINDOW_WIDTH);
        let h = h.max(MIN_WINDOW_HEIGHT);
        if (w, h) != self.window_size {
            self.window_size = (w, h);
            self.window_size_dirty = true;
        }
        self.window_id
            .map(|id| iced::window::is_maximized(id).map(Message::WindowMaximized))
            .unwrap_or_else(Task::none)
    }

    /// Debounced persistence tick — only flushes when the resize
    /// stream has been quiet for `WINDOW_SIZE_SAVE_INTERVAL`.
    pub(crate) fn update_persist_window_size(&mut self) -> Task<Message> {
        if self.window_size_dirty
            && self.window_size_last_save.elapsed() >= WINDOW_SIZE_SAVE_INTERVAL
        {
            self.persist_settings();
            self.window_size_dirty = false;
            self.window_size_last_save = std::time::Instant::now();
        }
        Task::none()
    }
}

#[cfg(test)]
mod tests {
    use crate::*;

    #[test]
    fn late_empty_startup_query_preserves_window_controls_during_work() {
        let id = iced::window::Id::unique();
        let mut app = App {
            operation: OperationExecution::fixture(true, Some(View::Root), Vec::new(), 0, None),
            ..App::default()
        };
        let _ = app.update(Message::Window(WindowMsg::WindowIdReceived(Some(id))));
        let _ = app.update(Message::Window(WindowMsg::WindowIdReceived(None)));
        assert_eq!(app.window_id, Some(id));
        assert_eq!(
            app.update(Message::Window(WindowMsg::WindowDrag)).units(),
            1
        );
        assert!(app.operation.is_running());
    }

    #[test]
    fn window_close_refuses_while_busy() {
        let mut app = App {
            operation: OperationExecution::fixture(true, Some(View::Root), Vec::new(), 0, None),
            ..App::default()
        };
        let _task = app.update_window(WindowMsg::WindowClose);
        assert!(app.operation.is_running(), "busy op must remain active");
        assert_eq!(app.operation.view(), Some(View::Root));
        // Reuses progress_dialog_body with the busy op label. Locale may be
        // non-English, so compare against the same formatter rather than
        // hard-coded English fragments.
        let expected = ltbox_core::tr_args!(
            "progress_dialog_body",
            operation = app.busy_operation_label()
        );
        assert_eq!(app.error_msg.as_deref(), Some(expected.as_str()));
    }

    #[test]
    fn window_close_allows_when_idle() {
        let mut app = App {
            operation: OperationExecution::fixture(false, None, Vec::new(), 0, None),
            error_msg: None,
            window_id: None,
            ..App::default()
        };
        let _task = app.update_window(WindowMsg::WindowClose);
        assert!(app.error_msg.is_none());
        assert!(!app.operation.is_running());
    }
}
