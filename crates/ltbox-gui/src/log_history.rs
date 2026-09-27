//! Full session transcript on disk, independent of the 500-line GUI tail.
//! Progress is a replaceable row on screen and a five-second sample in exports.
//!
//! The app also mirrors the transcript into `logs/sessions/`, so an operation
//! that failed can still be read after LTBox is closed.

use ltbox_core::live_sink::{Entry, Kind};
use std::{
    cell::RefCell,
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const SAMPLE_INTERVAL: Duration = Duration::from_secs(5);

/// Session transcripts kept on disk, newest first; older ones are removed
/// when a new session starts.
const SESSION_LOGS_KEPT: usize = 30;

struct Archive {
    file: Option<File>,
    fallback: String,
    error: Option<String>,
    /// Persistent copy of every archived line. Dropped on the first write
    /// error: the in-memory transcript and export stay authoritative.
    mirror: Option<File>,
}

impl Default for Archive {
    fn default() -> Self {
        match tempfile::tempfile() {
            Ok(file) => Self {
                file: Some(file),
                fallback: String::new(),
                error: None,
                mirror: None,
            },
            Err(e) => Self {
                file: None,
                fallback: String::new(),
                error: Some(e.to_string()),
                mirror: None,
            },
        }
    }
}

impl Archive {
    fn append(&mut self, line: &str) {
        if let Some(mirror) = &mut self.mirror
            && writeln!(mirror, "{line}").is_err()
        {
            self.mirror = None;
        }
        if self.error.is_none()
            && let Some(file) = &mut self.file
        {
            match file
                .seek(SeekFrom::End(0))
                .and_then(|_| writeln!(file, "{line}"))
            {
                Ok(_) => return,
                Err(e) => self.error = Some(e.to_string()),
            }
        }
        // Never silently discard a line on a full/unavailable temporary disk.
        self.fallback.push_str(line);
        self.fallback.push('\n');
    }

    fn text(&mut self) -> String {
        let mut text = String::new();
        if let Some(file) = &mut self.file
            && let Err(e) = file.rewind().and_then(|_| file.read_to_string(&mut text))
        {
            self.error = Some(e.to_string());
        }
        if let Some(error) = &self.error {
            text.push_str(&format!("[Log] Transcript storage error: {error}\n"));
        }
        text.push_str(&self.fallback);
        text
    }
}

struct Progress {
    key: String,
    line: String,
    last_saved: Option<String>,
    saved_at: Instant,
}

#[derive(Default)]
pub(crate) struct LogHistory {
    archive: RefCell<Archive>,
    progress: Option<Progress>,
}

impl LogHistory {
    pub(crate) fn with_initial(line: &str) -> Self {
        let history = Self::default();
        history.archive.borrow_mut().append(line);
        history
    }

    /// Return an ordinary visible line; progress remains a single separate row.
    pub(crate) fn record(&mut self, entry: Entry, now: Instant) -> Option<String> {
        match entry.kind {
            Kind::Progress { key } => {
                if self.progress.as_ref().is_none_or(|p| p.key != key) {
                    self.finish_progress();
                    self.progress = Some(Progress {
                        key,
                        line: entry.line,
                        last_saved: None,
                        saved_at: now,
                    });
                } else if let Some(progress) = &mut self.progress {
                    progress.line = entry.line;
                    if now.saturating_duration_since(progress.saved_at) >= SAMPLE_INTERVAL {
                        self.archive.borrow_mut().append(&progress.line);
                        progress.last_saved = Some(progress.line.clone());
                        progress.saved_at = now;
                    }
                }
                None
            }
            Kind::Debug => {
                self.archive
                    .borrow_mut()
                    .append(&format!("[Debug] {}", entry.line));
                None
            }
            Kind::Info => {
                self.finish_progress();
                self.archive.borrow_mut().append(&entry.line);
                Some(entry.line)
            }
        }
    }

    pub(crate) fn finish_progress(&mut self) {
        if let Some(progress) = self.progress.take()
            && progress.last_saved.as_deref() != Some(&progress.line)
        {
            self.archive.borrow_mut().append(&progress.line);
        }
    }

    pub(crate) fn progress_line(&self) -> Option<&str> {
        self.progress.as_ref().map(|p| p.line.as_str())
    }

    /// Start mirroring this session into a new timestamped file under `dir`,
    /// seeded with what is already archived, and prune older sessions.
    pub(crate) fn persist_to(&mut self, dir: &Path, stamp: &str) -> std::io::Result<PathBuf> {
        std::fs::create_dir_all(dir)?;
        prune_session_logs(dir, SESSION_LOGS_KEPT.saturating_sub(1));
        let path = dir.join(format!("session-{stamp}.log"));
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        let archived = self.archive.borrow_mut().text();
        if !archived.is_empty() {
            writeln!(file, "{}", archived.trim_end_matches('\n'))?;
        }
        self.archive.borrow_mut().mirror = Some(file);
        Ok(path)
    }

    /// Hand the session file over to a fresh history, so clearing the
    /// on-screen log does not end the on-disk transcript.
    pub(crate) fn cleared(&mut self) -> Self {
        // Preserve the latest progress even before its next five-second sample.
        self.finish_progress();
        let next = Self::default();
        next.archive.borrow_mut().mirror = self.archive.borrow_mut().mirror.take();
        next
    }

    pub(crate) fn text(&self) -> String {
        let mut text = self.archive.borrow_mut().text();
        if let Some(progress) = &self.progress
            && progress.last_saved.as_deref() != Some(&progress.line)
        {
            text.push_str(&progress.line);
            text.push('\n');
        }
        text.trim_end_matches('\n').to_owned()
    }
}

/// Remove all but the newest `keep` session transcripts in `dir`. Names sort
/// by their timestamp, so the lexical order is the chronological one.
fn prune_session_logs(dir: &Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut sessions: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("session-") && name.ends_with(".log"))
        })
        .collect();
    sessions.sort();
    let excess = sessions.len().saturating_sub(keep);
    for path in sessions.into_iter().take(excess) {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn progress(key: &str, line: &str) -> Entry {
        Entry {
            line: line.into(),
            kind: Kind::Progress { key: key.into() },
        }
    }

    #[test]
    fn progress_is_replaced_and_sampled_but_last_state_survives_an_error() {
        let mut log = LogHistory::default();
        let now = Instant::now();
        for second in 0..10 {
            assert!(
                log.record(
                    progress("a", &format!("{second}%")),
                    now + Duration::from_secs(second)
                )
                .is_none()
            );
        }
        assert_eq!(log.progress_line(), Some("9%"));
        assert_eq!(log.text(), "5%\n9%");
        log.record(
            Entry::info("write failed: timeout".into()),
            now + Duration::from_secs(10),
        );
        assert_eq!(log.text(), "5%\n9%\nwrite failed: timeout");
        assert!(log.progress_line().is_none());
    }

    #[test]
    fn export_keeps_early_lines_diagnostics_and_repeated_sessions() {
        let mut log = LogHistory::default();
        let now = Instant::now();
        for i in 0..600 {
            log.record(Entry::info(format!("line {i}")), now);
        }
        assert!(log.record(Entry::debug("GPT patch".into()), now).is_none());
        for _ in 0..2 {
            assert!(
                log.record(Entry::info("Sahara connected".into()), now)
                    .is_some()
            );
        }
        let text = log.text();
        assert!(text.starts_with("line 0\n"));
        assert!(text.contains("[Debug] GPT patch"));
        assert_eq!(text.matches("Sahara connected").count(), 2);
    }

    #[test]
    fn save_does_not_duplicate_pending_progress_or_move_append_position() {
        let mut log = LogHistory::with_initial("start");
        let now = Instant::now();
        log.record(progress("a", "10%"), now);
        assert_eq!(log.text(), log.text());
        log.record(progress("b", "20%"), now);
        log.record(Entry::info("done".into()), now);
        assert_eq!(log.text(), "start\n10%\n20%\ndone");
    }

    #[test]
    fn unavailable_temp_storage_preserves_lines_in_memory() {
        let mut archive = Archive {
            file: None,
            fallback: String::new(),
            error: Some("disk unavailable".into()),
            mirror: None,
        };
        archive.append("first");
        archive.append("error detail");
        let text = archive.text();
        assert!(text.contains("disk unavailable"));
        assert!(text.ends_with("first\nerror detail\n"));
    }

    #[test]
    fn session_file_holds_the_whole_transcript_and_survives_a_clear() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = LogHistory::with_initial("ready");
        let path = log.persist_to(dir.path(), "2026-09-27_14-00-00").unwrap();
        let now = Instant::now();
        log.record(Entry::info("[Country] Phase 1/5".into()), now);
        log.record(Entry::debug("GPT patch".into()), now);
        let mut log = log.cleared();
        log.record(Entry::info("after clear".into()), now);
        drop(log);
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            "ready\n[Country] Phase 1/5\n[Debug] GPT patch\nafter clear\n"
        );
    }

    #[test]
    fn clearing_preserves_latest_progress_without_duplicate_samples() {
        for sampled in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let mut log = LogHistory::with_initial("ready");
            let path = log.persist_to(dir.path(), "2026-09-28_00-00-00").unwrap();
            let now = Instant::now();
            log.record(progress("flash", "10%"), now);
            if sampled {
                log.record(progress("flash", "99%"), now + SAMPLE_INTERVAL);
            } else {
                log.record(progress("flash", "99%"), now + Duration::from_secs(1));
            }
            let before_clear = log.text();
            let mut log = log.cleared();
            assert_eq!(log.text(), "");
            assert!(log.progress_line().is_none());
            assert_eq!(
                std::fs::read_to_string(&path).unwrap(),
                format!("{before_clear}\n")
            );
            // A second clear must keep the same file without re-seeding it.
            log = log.cleared();
            log.record(
                Entry::info("write failed".into()),
                now + Duration::from_secs(6),
            );
            drop(log);
            assert_eq!(
                std::fs::read_to_string(path).unwrap(),
                "ready\n99%\nwrite failed\n"
            );
        }
    }

    #[test]
    fn old_session_files_are_pruned_and_other_files_kept() {
        let dir = tempfile::tempdir().unwrap();
        for day in 1..=SESSION_LOGS_KEPT + 5 {
            std::fs::write(dir.path().join(format!("session-2026-08-{day:02}.log")), "").unwrap();
        }
        std::fs::write(dir.path().join("notes.txt"), "").unwrap();
        let mut log = LogHistory::default();
        let newest = log.persist_to(dir.path(), "2026-09-27_14-00-00").unwrap();

        let mut names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert!(names.contains(&"notes.txt".to_string()));
        let sessions: Vec<_> = names.iter().filter(|n| n.starts_with("session-")).collect();
        assert_eq!(sessions.len(), SESSION_LOGS_KEPT);
        assert!(!names.contains(&"session-2026-08-01.log".to_string()));
        assert!(newest.exists());
    }
}
