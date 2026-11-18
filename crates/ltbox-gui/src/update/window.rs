//! Window-chrome handlers: titlebar buttons, cursor-drag resize, and
//! debounced geometry persistence.
use crate::*;
use iced::Task;

impl App {
    /// Dispatch a `WindowMsg`. Window actions are no-ops until `window_id` is
    /// set by `WindowIdReceived`.
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
                // Exiting mid-update would defeat the updater's rollback
                // guarantee. Native close requests and the titlebar both land
                // here.
                if self.operation.direct_update.is_active() {
                    return Task::none();
                }
                // Closing during any busy operation would tear it down
                // mid-work: refuse with the progress dialog's "in progress"
                // wording.
                if self.operation.is_running() {
                    let op_name = self.busy_operation_label();
                    self.error_msg = Some(ltbox_core::tr_args!(
                        "progress_dialog_body",
                        operation = op_name
                    ));
                    return Task::none();
                }
                self.konabess.cleanup_prepared();
                self.persist_settings();
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

    /// Keep layout current, then confirm native maximize state before saving
    /// normal geometry. Late replies from earlier resize events are ignored.
    pub(crate) fn update_window_resized(&mut self, w: f32, h: f32) -> Task<Message> {
        if !w.is_finite() || !h.is_finite() || w <= 0.0 || h <= 0.0 {
            return Task::none();
        }
        let w = w.max(MIN_WINDOW_WIDTH);
        let h = h.max(MIN_WINDOW_HEIGHT);
        self.window_size = (w, h);
        self.window_size_last_change = std::time::Instant::now();
        let request = self.window_size_last_change;
        self.window_id
            .map(|id| {
                iced::window::is_maximized(id)
                    .map(move |maximized| Message::WindowGeometryMeasured(request, w, h, maximized))
            })
            .unwrap_or_else(Task::none)
    }

    /// Debounced persistence tick: flushes only once resizing has been quiet
    /// for `WINDOW_SIZE_SAVE_INTERVAL`.
    pub(crate) fn update_persist_window_size(&mut self) -> Task<Message> {
        if self.window_size_dirty
            && self.window_size_last_change.elapsed() >= WINDOW_SIZE_SAVE_INTERVAL
        {
            self.persist_settings();
            self.window_size_dirty = false;
        }
        Task::none()
    }
}

#[cfg(test)]
mod tests {
    use crate::*;

    #[test]
    fn maximized_and_stale_measurements_do_not_overwrite_restore_geometry() {
        let mut app = App::default();
        let normal = app.window_restore_size;
        drop(app.update_window_resized(1920.0, 1080.0));
        let old = app.window_size_last_change;
        drop(app.update(Message::WindowGeometryMeasured(old, 1920.0, 1080.0, true)));
        assert_eq!(app.window_restore_size, normal);
        drop(app.update_window_resized(1000.0, 800.0));
        let current = app.window_size_last_change;
        drop(app.update(Message::WindowGeometryMeasured(old, 1920.0, 1080.0, false)));
        assert_eq!(app.window_restore_size, normal);
        drop(app.update(Message::WindowGeometryMeasured(
            current, 1000.0, 800.0, false,
        )));
        assert_eq!(app.window_restore_size, (1000.0, 800.0));
        drop(app.update_persist_window_size());
        assert!(app.window_size_dirty, "recent resize must remain debounced");
    }

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
        // Compare against the same formatter; the locale may not be English.
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
