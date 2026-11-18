//! Capture native (third-party) stdout/stderr into the GUI log on Windows.
//! LTBox's own log lines come from the typed live sink, not from here.

use std::sync::{Arc, Mutex, OnceLock};
#[cfg(windows)]
use std::time::{Duration, Instant};

type Queue = Arc<Mutex<Vec<String>>>;

/// Min gap between interim `\r` progress emits.
#[cfg(windows)]
const INTERIM_EMIT_INTERVAL: Duration = Duration::from_millis(800);

/// Cap queued lines; keep only the newest when native output runs ahead.
#[cfg(windows)]
const TAP_QUEUE_MAX: usize = 512;

/// Cap the in-flight byte buffer when no newline arrives.
#[cfg(windows)]
const PENDING_BUFFER_CAP: usize = 64 * 1024;

static TAP_QUEUE: OnceLock<Queue> = OnceLock::new();

/// Drain the captured lines. Empty on non-Windows or pipe failure.
pub fn drain() -> Vec<String> {
    if let Some(q) = TAP_QUEUE.get()
        && let Ok(mut g) = q.lock()
    {
        return std::mem::take(&mut *g);
    }
    Vec::new()
}

/// Install the tap. Call once before any `println!` runs. Idempotent.
pub fn install() {
    TAP_QUEUE.get_or_init(install_inner);
}

#[cfg(windows)]
fn install_inner() -> Queue {
    use std::io::Read;
    use std::os::windows::io::IntoRawHandle;

    let queue: Queue = Arc::new(Mutex::new(Vec::new()));

    let (mut reader, writer) = match std::io::pipe() {
        Ok(p) => p,
        Err(_) => return queue,
    };

    // Duplicate the write handle so one pipe feeds both stdout and stderr;
    // magiskboot-rs logs through eprintln!.
    let write_raw = writer.into_raw_handle();
    let write_raw_err: *mut core::ffi::c_void = unsafe {
        use windows_sys::Win32::Foundation::{DUPLICATE_SAME_ACCESS, DuplicateHandle};
        use windows_sys::Win32::System::Threading::GetCurrentProcess;
        let mut dup: *mut core::ffi::c_void = std::ptr::null_mut();
        let proc_h = GetCurrentProcess();
        if DuplicateHandle(
            proc_h,
            write_raw as _,
            proc_h,
            &mut dup,
            0,
            0,
            DUPLICATE_SAME_ACCESS,
        ) == 0
        {
            // Fall back to the original; neither handle is closed before exit.
            write_raw
        } else {
            dup
        }
    };
    // SAFETY: both handles live until process exit; `std::io::PipeWriter`
    // was consumed via `into_raw_handle`, so nothing else closes them.
    unsafe {
        use windows_sys::Win32::System::Console::{
            STD_ERROR_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle,
        };
        if SetStdHandle(STD_OUTPUT_HANDLE, write_raw as _) == 0 {
            return queue;
        }
        // stderr best-effort — failure loses eprintln! capture only.
        SetStdHandle(STD_ERROR_HANDLE, write_raw_err as _);
    }

    let q = Arc::clone(&queue);
    let _ = std::thread::Builder::new()
        .name("ltbox-stdout-tap".into())
        .spawn(move || {
            let mut buf = [0u8; 4096];
            let mut pending: Vec<u8> = Vec::new();
            let mut last_emit = Instant::now();
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        pending.extend_from_slice(&buf[..n]);
                        let emitted = flush_lines(&mut pending, &q);
                        if emitted > 0 {
                            last_emit = Instant::now();
                        } else if last_emit.elapsed() >= INTERIM_EMIT_INTERVAL {
                            // No `\n` yet: emit the latest `\r` segment as live
                            // progress, then drop up to it so a CR-only
                            // progress bar cannot grow `pending` unbounded.
                            if emit_interim_progress(&pending, &q) {
                                last_emit = Instant::now();
                                if let Some(idx) = pending.iter().rposition(|&b| b == b'\r') {
                                    pending.drain(..=idx);
                                }
                            }
                        }
                        // Cap the buffer for streams without `\r`/`\n` (e.g. raw
                        // binary); keep the tail so the next `\n` yields a line.
                        if pending.len() > PENDING_BUFFER_CAP {
                            let drop = pending.len() - PENDING_BUFFER_CAP;
                            pending.drain(..drop);
                        }
                    }
                    Err(_) => break,
                }
            }
            if !pending.is_empty() {
                pending.push(b'\n');
                flush_lines(&mut pending, &q);
            }
        });

    queue
}

#[cfg(not(windows))]
fn install_inner() -> Queue {
    Arc::new(Mutex::new(Vec::new()))
}

/// Emit the latest `\r` progress segment without draining `pending`.
#[cfg(windows)]
fn emit_interim_progress(pending: &[u8], queue: &Queue) -> bool {
    if pending.is_empty() || !pending.contains(&b'\r') {
        return false;
    }
    // Only bytes after the last `\n` belong to the in-flight line.
    let tail_start = pending
        .iter()
        .rposition(|&b| b == b'\n')
        .map(|i| i + 1)
        .unwrap_or(0);
    let tail = &pending[tail_start..];
    if !tail.contains(&b'\r') {
        return false;
    }
    let segment = last_nonempty_cr_segment(tail);
    if segment.is_empty() {
        return false;
    }
    let line = sanitize_line(segment);
    if line.is_empty() {
        return false;
    }
    if let Ok(mut g) = queue.lock() {
        let is_dup = g.last().map(|l| l == &line).unwrap_or(false);
        if !is_dup {
            g.push(line);
            if g.len() > TAP_QUEUE_MAX {
                let drop = g.len() - TAP_QUEUE_MAX;
                g.drain(..drop);
            }
            return true;
        }
    }
    false
}

#[cfg(windows)]
fn flush_lines(pending: &mut Vec<u8>, queue: &Queue) -> usize {
    // Split on '\n' only, keeping the final `\r` fragment of each line: a
    // `\r` boundary would queue hundreds of entries per second during a pbr
    // progress bar, which crashes the wgpu glyph atlas on some drivers.
    // Returns the push count so the reader can reset its interim throttle.
    let mut emitted = 0usize;
    let mut start = 0usize;
    let mut i = 0usize;
    while i < pending.len() {
        if pending[i] == b'\n' {
            let logical = &pending[start..i];
            let final_segment = last_nonempty_cr_segment(logical);
            if !final_segment.is_empty() {
                let line = sanitize_line(final_segment);
                if !line.is_empty()
                    && let Ok(mut g) = queue.lock()
                {
                    let is_dup = g.last().map(|l| l == &line).unwrap_or(false);
                    if !is_dup {
                        g.push(line);
                        if g.len() > TAP_QUEUE_MAX {
                            let drop = g.len() - TAP_QUEUE_MAX;
                            g.drain(..drop);
                        }
                        emitted += 1;
                    }
                }
            }
            start = i + 1;
        }
        i += 1;
    }
    if start > 0 {
        pending.drain(..start);
    }
    emitted
}

/// Final non-empty `\r`-delimited segment; whole slice if no `\r`.
#[cfg(windows)]
fn last_nonempty_cr_segment(slice: &[u8]) -> &[u8] {
    let mut best: &[u8] = slice;
    let mut cursor = 0usize;
    let mut last_start = 0usize;
    while cursor < slice.len() {
        if slice[cursor] == b'\r' {
            if cursor > last_start {
                best = &slice[last_start..cursor];
            }
            last_start = cursor + 1;
        }
        cursor += 1;
    }
    if slice.len() > last_start {
        best = &slice[last_start..];
    }
    best
}

/// Decode lossily and strip ANSI escapes.
#[cfg(windows)]
fn sanitize_line(bytes: &[u8]) -> String {
    let decoded = String::from_utf8_lossy(bytes);
    let stripped = strip_ansi(&decoded);
    stripped.trim_end().to_string()
}

#[cfg(windows)]
fn strip_ansi(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1B {
            i += 1;
            if i < bytes.len() && bytes[i] == b'[' {
                i += 1;
                while i < bytes.len() && !bytes[i].is_ascii_alphabetic() {
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1;
                }
            } else {
                i += 1;
            }
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn strip_ansi_removes_csi_sequences() {
        let s = "\x1b[91mfailed\x1b[39m";
        assert_eq!(strip_ansi(s), "failed");
    }

    #[test]
    fn flush_lines_emits_one_entry_per_newline() {
        let queue: Queue = Arc::new(Mutex::new(Vec::new()));
        let mut buf = b"hello\nworld\n".to_vec();
        flush_lines(&mut buf, &queue);
        let lines = queue.lock().unwrap().clone();
        assert_eq!(lines, vec!["hello".to_string(), "world".to_string()]);
        assert!(buf.is_empty());
    }

    #[test]
    fn flush_lines_collapses_pbr_cr_updates_into_final_state() {
        // pbr rewrites one line via \r…\n; only the final segment is queued.
        let queue: Queue = Arc::new(Mutex::new(Vec::new()));
        let mut buf = b"progress 10%\rprogress 50%\rprogress 100%\n".to_vec();
        flush_lines(&mut buf, &queue);
        let lines = queue.lock().unwrap().clone();
        assert_eq!(lines, vec!["progress 100%".to_string()]);
    }

    #[test]
    fn flush_lines_waits_for_newline_before_flushing_pbr_line() {
        // Partial \r-only line must not leak — flush only at \n.
        let queue: Queue = Arc::new(Mutex::new(Vec::new()));
        let mut buf = b"progress 10%\rprogress 20%\r".to_vec();
        flush_lines(&mut buf, &queue);
        assert!(queue.lock().unwrap().is_empty());
        assert_eq!(buf, b"progress 10%\rprogress 20%\r".to_vec());
    }

    #[test]
    fn flush_lines_dedupes_consecutive_duplicates() {
        let queue: Queue = Arc::new(Mutex::new(Vec::new()));
        let mut buf = b"same line\nsame line\n".to_vec();
        flush_lines(&mut buf, &queue);
        let lines = queue.lock().unwrap().clone();
        assert_eq!(lines, vec!["same line".to_string()]);
    }

    #[test]
    fn flush_lines_returns_emit_count() {
        let queue: Queue = Arc::new(Mutex::new(Vec::new()));
        let mut buf = b"a\nb\nc\n".to_vec();
        let n = flush_lines(&mut buf, &queue);
        assert_eq!(n, 3);
    }

    #[test]
    fn emit_interim_progress_surfaces_latest_cr_segment() {
        let queue: Queue = Arc::new(Mutex::new(Vec::new()));
        // No \n → flush_lines is a no-op, interim push should fire.
        let pending = b"Sending partition: 10%\rSending partition: 40%\r".to_vec();
        assert!(emit_interim_progress(&pending, &queue));
        assert_eq!(
            queue.lock().unwrap().clone(),
            vec!["Sending partition: 40%".to_string()]
        );
    }

    #[test]
    fn emit_interim_progress_ignores_dupes() {
        let queue: Queue = Arc::new(Mutex::new(Vec::new()));
        queue.lock().unwrap().push("same".to_string());
        let pending = b"same\r".to_vec();
        assert!(!emit_interim_progress(&pending, &queue));
        assert_eq!(queue.lock().unwrap().len(), 1);
    }

    #[test]
    fn emit_interim_progress_noop_without_cr() {
        let queue: Queue = Arc::new(Mutex::new(Vec::new()));
        let pending = b"partial line without cr".to_vec();
        assert!(!emit_interim_progress(&pending, &queue));
        assert!(queue.lock().unwrap().is_empty());
    }

    #[test]
    fn last_nonempty_cr_segment_returns_tail_segment() {
        assert_eq!(last_nonempty_cr_segment(b"a\rb\rc"), b"c");
        assert_eq!(last_nonempty_cr_segment(b"a\rb\r"), b"b");
        assert_eq!(last_nonempty_cr_segment(b"hello"), b"hello");
        assert_eq!(last_nonempty_cr_segment(b""), b"");
    }
}
