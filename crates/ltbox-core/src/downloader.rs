//! HTTP download helpers for root-pipeline asset fetches.
//!
//! Blocking `ureq` wrapper that streams a URL to disk, logs start/results, and
//! publishes transient progress to the live sink. Pairs with [`crate::github::GitHubClient`]
//! for release-asset URL resolution.

use std::path::Path;

use crate::error::{LtboxError, Result};

/// `ltbox/<version>` user agent for every outbound request, including the
/// separate short-timeout agent in `probe_connectivity`.
pub const USER_AGENT: &str = concat!("ltbox/", env!("CARGO_PKG_VERSION"));

/// Process-wide `ureq::Agent`, so every outbound request shares one TLS config
/// and connection pool.
///
/// Phase budgets: 15 s connect, 30 s response headers, 600 s total body receive.
/// The body budget starts after headers and is not reset by reads or progress,
/// so a slow but steady download can still time out.
fn shared_agent() -> &'static ureq::Agent {
    use std::sync::OnceLock;
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .user_agent(USER_AGENT)
            .timeout_connect(Some(std::time::Duration::from_secs(15)))
            .timeout_recv_response(Some(std::time::Duration::from_secs(30)))
            .timeout_recv_body(Some(std::time::Duration::from_secs(600)))
            .build()
            .new_agent()
    })
}

/// Clone the shared `ureq::Agent` handle (cheap, `Arc`-backed). Use it for
/// every outbound HTTP request, in any crate.
pub fn build_agent() -> ureq::Agent {
    shared_agent().clone()
}

/// Event emitted by [`stream_with_progress`]; callers own all formatting and
/// i18n.
pub enum DownloadEvent {
    /// Stream opened, before any bytes have been read.
    Start,
    /// Known `Content-Length`: a 750 ms progress tick fired.
    ProgressPct {
        downloaded_mb: f64,
        total_mb: f64,
        pct: i32,
        speed_mbps: f64,
    },
    /// Unknown length (chunked or no header): 750 ms tick fired.
    ProgressChunked { downloaded_mb: f64, speed_mbps: f64 },
    /// Body fully read + flushed to disk.
    Done { downloaded_mb: f64, elapsed_s: f64 },
}

/// Stream `url` to `out_path` in 64 KiB chunks, calling `on_event` for start,
/// progress (throttled to one per 750 ms) and completion.
///
/// Creates missing parent dirs. Bytes land in a sibling temporary file that
/// replaces `out_path` only after a complete download; partials are removed on
/// failure, so a reader never sees a truncated destination.
pub fn stream_with_progress<F>(
    agent: &ureq::Agent,
    url: &str,
    out_path: &Path,
    log: &mut Vec<String>,
    mut on_event: F,
) -> Result<()>
where
    F: FnMut(&mut Vec<String>, DownloadEvent),
{
    use std::io::{Read, Write};

    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut resp = agent
        .get(url)
        .call()
        .map_err(|e| LtboxError::Download(format!("GET {url}: {e}")))?;
    let total: Option<u64> = resp
        .headers()
        .get(ureq::http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok());
    let mut reader = resp.body_mut().as_reader();

    let write_result = (|| -> Result<(u64, std::time::Instant, tempfile::NamedTempFile)> {
        let mut file = create_partial_file(out_path).map_err(|e| {
            LtboxError::Download(format!("create partial for {}: {e}", out_path.display()))
        })?;
        let mut buf = [0u8; 64 * 1024];
        let mut downloaded: u64 = 0;

        let started_at = std::time::Instant::now();
        let mut last_emit_at = started_at;

        on_event(log, DownloadEvent::Start);

        loop {
            let n = reader
                .read(&mut buf)
                .map_err(|e| LtboxError::Download(format!("read: {e}")))?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n])?;
            downloaded += n as u64;

            let now = std::time::Instant::now();
            let dl_mb = downloaded as f64 / 1_000_000.0;
            let elapsed = now.duration_since(started_at).as_secs_f64().max(0.001);
            let speed_mbps = dl_mb / elapsed;
            if let Some(total) = total
                && total > 0
            {
                let pct = (downloaded * 100 / total) as i32;
                if now.duration_since(last_emit_at) >= std::time::Duration::from_millis(750) {
                    last_emit_at = now;
                    let total_mb = total as f64 / 1_000_000.0;
                    on_event(
                        log,
                        DownloadEvent::ProgressPct {
                            downloaded_mb: dl_mb,
                            total_mb,
                            pct,
                            speed_mbps,
                        },
                    );
                }
            } else if now.duration_since(last_emit_at) >= std::time::Duration::from_millis(750) {
                last_emit_at = now;
                on_event(
                    log,
                    DownloadEvent::ProgressChunked {
                        downloaded_mb: dl_mb,
                        speed_mbps,
                    },
                );
            }
        }

        file.flush()?;
        file.as_file().sync_all().map_err(|e| {
            LtboxError::Download(format!("sync partial {}: {e}", file.path().display()))
        })?;
        Ok((downloaded, started_at, file))
    })();

    match write_result {
        Ok((downloaded, started_at, partial)) => {
            // Same filesystem, so persist replaces the destination atomically;
            // on failure the destination is untouched and the partial dropped.
            let partial_path = partial.path().to_path_buf();
            if let Err(e) = persist_replacing(partial, out_path) {
                return Err(LtboxError::Download(format!(
                    "finalize {} -> {}: {e}",
                    partial_path.display(),
                    out_path.display(),
                )));
            }
            let elapsed_s = started_at.elapsed().as_secs_f64().max(0.001);
            let dl_mb = downloaded as f64 / 1_000_000.0;
            on_event(
                log,
                DownloadEvent::Done {
                    downloaded_mb: dl_mb,
                    elapsed_s,
                },
            );
            Ok(())
        }
        // The partial was dropped (and removed) when the write closure failed.
        Err(e) => Err(e),
    }
}

/// Move a finished temp file over `dest`, replacing it.
///
/// `NamedTempFile::persist` makes a single `MoveFileExW` attempt on Windows;
/// `std::fs::rename` also retries an access-denied replace with POSIX rename
/// semantics, which succeeds while antivirus or the indexer holds the old
/// destination open with delete sharing. On failure the temp file is removed
/// and `dest` is untouched.
pub fn persist_replacing(file: tempfile::NamedTempFile, dest: &Path) -> std::io::Result<()> {
    match file.persist(dest) {
        Ok(_) => Ok(()),
        Err(e) if cfg!(windows) && e.error.kind() == std::io::ErrorKind::PermissionDenied => {
            match std::fs::rename(e.file.path(), dest) {
                Ok(()) => {
                    // The temp path is gone; keep the guard from deleting it.
                    let _ = e.file.keep();
                    Ok(())
                }
                Err(err) => Err(err),
            }
        }
        Err(e) => Err(e.error),
    }
}

/// Exclusively create a hidden, randomly named partial file next to
/// `out_path` (same filesystem, so the final rename is atomic). Dropping it
/// removes it, so every early return cleans up the partial.
fn create_partial_file(out_path: &Path) -> std::io::Result<tempfile::NamedTempFile> {
    let file_name = out_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("download");
    let parent = out_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    tempfile::Builder::new()
        .prefix(&format!(".{file_name}.ltbox-partial-"))
        .tempfile_in(parent)
}

/// Download `url` to `out_path` via [`stream_with_progress`], logging start and
/// result lines and live progress (one update per 750 ms; the GUI log history
/// samples every 5 s). The destination is replaced only after a complete
/// download.
pub fn download_to_file(url: &str, out_path: &Path, log: &mut Vec<String>) -> Result<()> {
    let display_name = out_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("download")
        .to_string();
    let url_for_start = url.to_string();
    let agent = build_agent();
    let progress_key = crate::live_sink::progress_key("download");
    stream_with_progress(&agent, url, out_path, log, move |log, event| {
        // Start/result lines are ordinary log lines; progress carries a key so
        // the sink can coalesce it.
        match event {
            DownloadEvent::Start => {
                crate::live!(
                    log,
                    "[Download] {}",
                    crate::tr_args!(
                        "live_download_start",
                        name = &display_name,
                        url = &url_for_start
                    )
                );
            }
            DownloadEvent::ProgressPct {
                downloaded_mb,
                total_mb,
                pct,
                speed_mbps,
            } => {
                crate::live_sink::progress(
                    &progress_key,
                    format!(
                        "[Download] {}",
                        crate::tr_args!(
                            "live_download_progress_pct",
                            name = &display_name,
                            pct = pct,
                            downloaded =
                                crate::log_format::decimal_bytes(downloaded_mb * 1_000_000.0),
                            total = crate::log_format::decimal_bytes(total_mb * 1_000_000.0),
                            speed = crate::log_format::decimal_bytes(speed_mbps * 1_000_000.0)
                        )
                    ),
                );
            }
            DownloadEvent::ProgressChunked {
                downloaded_mb,
                speed_mbps,
            } => {
                crate::live_sink::progress(
                    &progress_key,
                    format!(
                        "[Download] {}",
                        crate::tr_args!(
                            "live_download_progress_chunked",
                            name = &display_name,
                            downloaded =
                                crate::log_format::decimal_bytes(downloaded_mb * 1_000_000.0),
                            speed = crate::log_format::decimal_bytes(speed_mbps * 1_000_000.0)
                        )
                    ),
                );
            }
            DownloadEvent::Done {
                downloaded_mb,
                elapsed_s,
            } => {
                let avg = downloaded_mb / elapsed_s.max(0.001);
                crate::live!(
                    log,
                    "[Download] {}",
                    crate::tr_args!(
                        "live_download_done",
                        name = &display_name,
                        size = crate::log_format::decimal_bytes(downloaded_mb * 1_000_000.0),
                        elapsed = crate::log_format::elapsed(elapsed_s),
                        avg = crate::log_format::decimal_bytes(avg * 1_000_000.0)
                    )
                );
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    /// Serve one response declaring `content_length` but sending `body`.
    ///
    /// The request head is read first and the socket drained before closing:
    /// closing with unread bytes makes Windows send RST, which the client
    /// reports as os error 10054.
    fn serve_response(body: &'static [u8], content_length: usize) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut request = Vec::new();
            let mut buf = [0u8; 1024];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                match stream.read(&mut buf) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => request.extend_from_slice(&buf[..n]),
                }
            }
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {content_length}\r\nConnection: close\r\n\r\n"
            );
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(body);
            let _ = stream.flush();
            let _ = stream.shutdown(std::net::Shutdown::Write);
            while matches!(stream.read(&mut buf), Ok(n) if n > 0) {}
        });
        format!("http://{addr}/file.bin")
    }

    fn serve_bytes(body: &'static [u8]) -> String {
        serve_response(body, body.len())
    }

    #[test]
    fn partial_file_is_hidden_sibling_removed_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("firmware.zip");
        let partial = create_partial_file(&out).unwrap();
        let path = partial.path().to_path_buf();
        assert_eq!(path.parent(), out.parent());
        let name = path.file_name().unwrap().to_string_lossy();
        assert!(name.starts_with(".firmware.zip.ltbox-partial-"));
        assert_ne!(path, out);
        drop(partial);
        assert!(!path.exists());
    }

    #[test]
    fn download_replaces_destination_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("payload.bin");
        std::fs::write(&out, b"stale-content").unwrap();

        let url = serve_bytes(b"fresh-payload-bytes");
        let mut log = Vec::new();
        download_to_file(&url, &out, &mut log).expect("download");
        assert_eq!(std::fs::read(&out).unwrap(), b"fresh-payload-bytes");

        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains("ltbox-partial"))
            .collect();
        assert!(leftovers.is_empty(), "partials left behind: {leftovers:?}");
        assert!(
            log.iter().any(|l| l.contains("[Download]")),
            "progress / done logging should still fire"
        );
    }

    #[test]
    fn short_download_emits_start_and_done_without_progress_spam() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("small.bin");
        let url = serve_bytes(b"small download");
        let mut events = Vec::new();
        stream_with_progress(&build_agent(), &url, &out, &mut Vec::new(), |_, event| {
            events.push(match event {
                DownloadEvent::Start => "start",
                DownloadEvent::Done { .. } => {
                    assert_eq!(std::fs::read(&out).unwrap(), b"small download");
                    "done"
                }
                _ => "progress",
            });
        })
        .unwrap();
        assert_eq!(events, ["start", "done"]);
    }

    #[test]
    fn failed_download_keeps_existing_destination_and_cleans_partial() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("payload.bin");
        std::fs::write(&out, b"keep-me").unwrap();

        // The body ends short of the declared length, so the read fails after
        // the partial exists. The server keeps its port for the whole test; a
        // dropped port could be reused by a parallel test's server.
        let url = serve_response(b"truncated", 1 << 20);

        let mut log = Vec::new();
        let err = download_to_file(&url, &out, &mut log).expect_err("must fail");
        assert!(matches!(err, LtboxError::Download(_)));
        assert_eq!(std::fs::read(&out).unwrap(), b"keep-me");
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains("ltbox-partial"))
            .collect();
        assert!(leftovers.is_empty(), "partials left behind: {leftovers:?}");
    }
}
