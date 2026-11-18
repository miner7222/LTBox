//! Operation log buffer, history streams and log export.

use crate::*;

impl App {
    /// Push one line, trim to `LOG_MAX_LINES`. The editor is rebuilt on the drain
    /// tick, not per push: per-push reshaping drove wgpu into TDR in long flashes.
    pub(crate) fn log_push<S: Into<String>>(&mut self, line: S) {
        self.record_log_entry(ltbox_core::live_sink::Entry::info(line.into()));
    }

    pub(crate) fn record_log_entry(&mut self, entry: ltbox_core::live_sink::Entry) {
        if let Some(line) = self.log_history.record(entry, std::time::Instant::now()) {
            self.log_lines.push(line);
        }
        self.trim_log();
        self.log_dirty = true;
    }

    /// Our typed sink is the sole producer of LTBox messages. The native tap
    /// contains only third-party output, so repeated sessions need no text dedup.
    pub(crate) fn drain_pending_log_streams(&mut self) -> usize {
        let sink_lines = ltbox_core::live_sink::drain();
        let tap_lines = stdout_tap::drain();
        let total = sink_lines.len() + tap_lines.len();
        for entry in sink_lines {
            self.record_log_entry(entry);
        }
        for line in tap_lines {
            self.log_push(format!("[Native] {line}"));
        }
        total
    }

    /// Final flush at `*ExecDone`. The closure's Vec is ignored: `live!` already
    /// streamed every line through the sink, so re-appending would duplicate the
    /// transcript.
    pub(crate) fn flush_exec_done_log(&mut self, _vec_from_closure: Vec<String>) {
        // Drain what the 500 ms tick missed so the closing lines show immediately.
        self.drain_pending_log_streams();
    }

    pub(crate) fn set_image_info_log(&mut self, text: String) {
        self.image_info_log = text;
        self.image_info_log_editor =
            iced::widget::text_editor::Content::with_text(&self.image_info_log);
        use iced::widget::text_editor::{Action, Motion};
        self.image_info_log_editor
            .perform(Action::Move(Motion::DocumentEnd));
    }

    pub(crate) fn image_info_exec_active(&self) -> bool {
        self.current_view == View::Advanced
            && self.adv_wizard.is_image_info()
            && self.adv_wizard.step == self.adv_wizard.exec_step()
    }

    pub(crate) fn active_log_save_source(&self) -> LogSaveSource {
        if self.image_info_exec_active() {
            LogSaveSource::ImageInfo
        } else {
            LogSaveSource::Main
        }
    }

    pub(crate) fn log_text_for_save(&self, source: LogSaveSource) -> String {
        match source {
            LogSaveSource::Main => self.log_history.text(),
            LogSaveSource::ImageInfo => self.image_info_log.clone(),
        }
    }

    pub(crate) fn note_log_save_result(&mut self, source: LogSaveSource, line: String) {
        match source {
            LogSaveSource::Main => self.log_push(line),
            LogSaveSource::ImageInfo => {
                let mut text = self.image_info_log.trim_end().to_string();
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(&line);
                self.set_image_info_log(text);
            }
        }
    }

    /// 80-wide `=` separator with an optional centred label.
    pub(crate) fn log_separator(&mut self, label: Option<&str>) {
        const BAR: &str =
            "================================================================================";
        let line = match label {
            Some(s) if !s.is_empty() => {
                let inner = format!(" {s} ");
                let bar_len = BAR.len();
                let inner_len = inner.chars().count();
                if inner_len >= bar_len {
                    inner
                } else {
                    let side = (bar_len - inner_len) / 2;
                    let left = &BAR[..side];
                    let right = &BAR[..bar_len - side - inner_len];
                    format!("{left}{inner}{right}")
                }
            }
            _ => BAR.to_string(),
        };
        self.log_push(line);
    }

    pub(crate) fn trim_log(&mut self) {
        if self.log_lines.len() > LOG_MAX_LINES {
            let drop = self.log_lines.len() - LOG_MAX_LINES;
            self.log_lines.drain(..drop);
        }
    }

    /// Rebuild the editor from `log_lines` and scroll to the end. Selection resets.
    pub(crate) fn rebuild_log_editor(&mut self) {
        let mut joined = self.log_lines.join("\n");
        if let Some(progress) = self.log_history.progress_line() {
            if !joined.is_empty() {
                joined.push('\n');
            }
            joined.push_str(progress);
        }
        self.log_editor = iced::widget::text_editor::Content::with_text(&joined);
        use iced::widget::text_editor::{Action, Motion};
        self.log_editor.perform(Action::Move(Motion::DocumentEnd));
        self.log_dirty = false;
    }
}
