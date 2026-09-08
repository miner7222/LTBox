//! Partition + physical-range flash/dump workers. Each runs off the
//! UI thread: route to EDL, open Sahara/Firehose, scan GPTs or
//! transfer images, then reset. Extracted from `main.rs`.

use crate::{
    ConnectionStatus, DumpPartRow, DumpPartsScanResult, FlashPartRow, FlashPartsScanResult,
    FlashRowState, PhaseReporter, ensure_edl,
};
use ltbox_core::tr_args;

/// Shared scan phase for the Flash/Dump Partitions wizards: route to EDL,
/// open Sahara, read GPTs on LUN 0..=5. On success returns the scanned
/// partitions plus the still-open session so the caller can map its rows
/// and then bounce the device back to EDL. On failure it logs (and, for a
/// scan error, resets) and returns the error string. `tag` is the log
/// channel prefix; `open_failed_key` / `scan_failed_key` are the i18n keys
/// for the two failure messages (the only per-wizard text difference).
fn scan_lun_partitions(
    conn: ConnectionStatus,
    loader_path: &str,
    tag: &str,
    open_failed_key: &str,
    scan_failed_key: &str,
    log: &mut Vec<String>,
) -> Result<
    (
        Vec<ltbox_device::edl::GptPartitionInfo>,
        ltbox_device::edl::EdlSession,
    ),
    String,
> {
    if ensure_edl(conn, tag, log).is_err() {
        return Err(ltbox_core::i18n::tr("err_edl_transition_failed"));
    }

    std::thread::sleep(std::time::Duration::from_secs(2));
    let loader = std::path::PathBuf::from(loader_path);
    let mut session = match ltbox_device::edl::EdlSession::open(&loader, log) {
        Ok(s) => s,
        Err(e) => {
            ltbox_core::live!(
                log,
                "[{}] {}",
                tag,
                tr_args!(open_failed_key, error = e.to_string())
            );
            ltbox_core::live!(
                log,
                "{}",
                ltbox_core::i18n::tr("live_edl_open_failed_reboot_notice")
            );
            return Err(tr_args!(
                "err_edl_session_open_failed",
                error = e.to_string()
            ));
        }
    };

    match session.scan_partitions(0..=5, log) {
        Ok(parts) => Ok((parts, session)),
        Err(e) => {
            ltbox_core::live!(
                log,
                "[{}] {}",
                tag,
                tr_args!(scan_failed_key, error = e.to_string())
            );
            let _ = session.reset_to_edl(log);
            Err(tr_args!("err_parts_scan_failed", error = e.to_string()))
        }
    }
}

/// Flash Partitions scan phase. Mirror of `dump_parts_scan`: shares the
/// transition + open + GPT scan via `scan_lun_partitions`, then maps the
/// partitions into checkable flash rows and bounces back to EDL so the
/// exec pass can reopen without a power-cycle.
pub(crate) fn flash_parts_scan(
    conn: ConnectionStatus,
    loader_path: String,
) -> FlashPartsScanResult {
    let mut log = Vec::new();
    let (parts, mut session) = match scan_lun_partitions(
        conn,
        &loader_path,
        "FlashParts",
        "err_edl_session_open_failed",
        "err_parts_scan_failed",
        &mut log,
    ) {
        Ok(v) => v,
        Err(error) => {
            return FlashPartsScanResult {
                logs: log,
                rows: Vec::new(),
                error: Some(error),
            };
        }
    };

    let rows: Vec<FlashPartRow> = parts
        .into_iter()
        .map(|p| FlashPartRow {
            lun: p.lun,
            label: p.name,
            start_sector: p.start_sector,
            num_sectors: p.num_sectors,
            size_bytes: p.size_bytes,
            file_path: None,
            state: FlashRowState::Skip,
        })
        .collect();

    if let Err(e) = session.reset_to_edl(&mut log) {
        ltbox_core::live!(
            log,
            "[FlashParts] {}",
            tr_args!("live_flashparts_reset_failed", error = e)
        );
    }

    ltbox_core::live!(
        log,
        "[FlashParts] {}",
        tr_args!("live_parts_scan_complete", count = rows.len().to_string())
    );
    FlashPartsScanResult {
        logs: log,
        rows,
        error: None,
    }
}

/// Preflight every Flash-state partition row before any device write.
/// Returns `Err` listing all missing sources so the whole batch aborts
/// instead of silently skipping and reporting success.
pub(crate) fn preflight_flash_part_sources(rows: &[FlashPartRow]) -> Result<(), String> {
    let mut missing = Vec::new();
    for row in rows {
        if row.state != FlashRowState::Write {
            continue;
        }
        match row.file_path.as_ref() {
            None => missing.push(format!("{}: no file selected", row.label)),
            Some(path) if !std::path::Path::new(path).is_file() => {
                missing.push(format!(
                    "{}: {}",
                    row.label,
                    tr_args!("err_path_missing", path = path)
                ));
            }
            _ => {}
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(missing.join("; "))
    }
}

/// Preflight every selected physical-flash `(lun, path)` pair before any
/// device write. Missing images fail the whole operation.
pub(crate) fn preflight_flash_phys_sources(pairs: &[(u8, String)]) -> Result<(), String> {
    let mut missing = Vec::new();
    for (lun, path) in pairs {
        if !std::path::Path::new(path).is_file() {
            missing.push(format!(
                "LUN {lun}: {}",
                tr_args!("err_path_missing", path = path)
            ));
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(missing.join("; "))
    }
}

/// Surface dump failures to the UI. Critical partition failures take
/// priority; any remaining partial failures still become an error so a
/// partial dump is never reported as unconditional success.
pub(crate) fn dump_parts_outcome_error(
    critical_failures: &[String],
    all_failures: &[String],
) -> Option<String> {
    if !critical_failures.is_empty() {
        let labels = critical_failures.join(", ");
        let translated = tr_args!("live_dumpparts_critical_failure", labels = labels.clone());
        // Keep labels visible even before a GUI translator is installed
        // (tr falls back to the bare key, which has no `{labels}` slot).
        Some(if translated.contains(&labels) {
            translated
        } else {
            format!("{translated}: {labels}")
        })
    } else if !all_failures.is_empty() {
        Some(format!("Dump incomplete: {}", all_failures.join(", ")))
    } else {
        None
    }
}

/// Exec phase. Reopens the EDL session, walks the active rows, flashing
/// or erasing each, then reboots to system.
pub(crate) fn flash_parts_execute(
    loader_path: String,
    rows: Vec<FlashPartRow>,
    phases: PhaseReporter,
) -> Result<Vec<String>, String> {
    let mut log = Vec::new();
    // Fail closed before opening Firehose / writing anything when a
    // selected flash image is missing on disk.
    if let Err(e) = preflight_flash_part_sources(&rows) {
        ltbox_core::live!(log, "[FlashParts] {e}");
        return Err(e);
    }
    ltbox_core::live!(log, "[FlashParts] {}", phases.marker(1));
    std::thread::sleep(std::time::Duration::from_secs(2));
    let loader = std::path::PathBuf::from(&loader_path);
    let mut session = match ltbox_device::edl::EdlSession::open(&loader, &mut log) {
        Ok(s) => s,
        Err(e) => {
            ltbox_core::live!(
                log,
                "[FlashParts] {}",
                tr_args!("err_edl_session_open_failed", error = e.to_string())
            );
            ltbox_core::live!(
                log,
                "{}",
                ltbox_core::i18n::tr("live_edl_open_failed_reboot_notice")
            );
            return Err(tr_args!(
                "err_edl_session_open_failed",
                error = e.to_string()
            ));
        }
    };

    ltbox_core::live!(log, "[FlashParts] {}", phases.marker(2));
    for row in &rows {
        match row.state {
            FlashRowState::Write => {
                // Sources were preflighted; treat absence as a hard error if
                // state races somehow remove the path between checks.
                let Some(path) = row.file_path.as_ref() else {
                    return Err(format!("{}: no file selected", row.label));
                };
                let img = std::path::Path::new(path);
                if !img.is_file() {
                    return Err(format!(
                        "{}: {}",
                        row.label,
                        tr_args!("err_path_missing", path = path)
                    ));
                }
                let file_name = img
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.clone());
                ltbox_core::live!(
                    log,
                    "[FlashParts] {}",
                    tr_args!(
                        "live_flashparts_flashing",
                        label = row.label,
                        file = file_name,
                        lun = row.lun.to_string()
                    )
                );
                phases.mark_writes_started();
                if let Err(e) = session.flash_partition_at(
                    &row.label,
                    img,
                    row.lun,
                    &row.start_sector.to_string(),
                    row.num_sectors,
                    &mut log,
                ) {
                    ltbox_core::live!(
                        log,
                        "[FlashParts] {}",
                        tr_args!(
                            "live_flashparts_part_failed",
                            label = row.label,
                            error = e.to_string()
                        )
                    );
                    // Abort the remaining writes — a failed write can mean a
                    // dropped link, and the device is left in EDL for retry.
                    return Err(tr_args!(
                        "err_flash_parts_part_failed",
                        label = row.label,
                        error = e.to_string()
                    ));
                }
            }
            FlashRowState::Erase => {
                ltbox_core::live!(
                    log,
                    "[FlashParts] {}",
                    tr_args!(
                        "live_flashparts_erasing",
                        label = row.label,
                        lun = row.lun.to_string(),
                        sectors = row.num_sectors.to_string()
                    )
                );
                // GPT sector counts are u64; reject counts the erase API
                // cannot represent instead of truncating them.
                let erase_outcome = match usize::try_from(row.num_sectors) {
                    Ok(count) => {
                        phases.mark_writes_started();
                        session
                            .erase_partition_at(
                                &row.label,
                                row.lun,
                                &row.start_sector.to_string(),
                                count,
                                &mut log,
                            )
                            .map_err(|e| e.to_string())
                    }
                    Err(_) => Err(format!(
                        "partition geometry out of range (start_sector={}, num_sectors={})",
                        row.start_sector, row.num_sectors
                    )),
                };
                if let Err(e) = erase_outcome {
                    ltbox_core::live!(
                        log,
                        "[FlashParts] {}",
                        tr_args!(
                            "live_flashparts_erase_failed",
                            label = row.label,
                            error = e.to_string()
                        )
                    );
                    return Err(tr_args!(
                        "err_flash_parts_erase_failed",
                        label = row.label,
                        error = e.to_string()
                    ));
                }
            }
            FlashRowState::Skip => {}
        }
    }

    ltbox_core::live!(log, "[FlashParts] {}", phases.marker(3));
    ltbox_core::live!(
        log,
        "[FlashParts] {}",
        ltbox_core::i18n::tr("live_resetting_system")
    );
    session.reset_tolerant(&mut log);
    ltbox_core::live!(log, "[FlashParts] {}", ltbox_core::i18n::tr("live_op_done"));
    Ok(log)
}

/// Scan GPTs on LUNs 0..=5 using the picked loader. Leaves the device
/// in EDL (bounces through `reset_to_edl`) so the dump pass can re-open
/// Sahara without a power-cycle.
pub(crate) fn dump_parts_scan(conn: ConnectionStatus, loader_path: String) -> DumpPartsScanResult {
    let mut log = Vec::new();
    let (parts, mut session) = match scan_lun_partitions(
        conn,
        &loader_path,
        "DumpParts",
        "err_edl_session_open_failed",
        "err_parts_scan_failed",
        &mut log,
    ) {
        Ok(v) => v,
        Err(error) => {
            return DumpPartsScanResult {
                logs: log,
                rows: Vec::new(),
                error: Some(error),
            };
        }
    };

    let rows: Vec<DumpPartRow> = parts
        .into_iter()
        .map(|p| DumpPartRow {
            lun: p.lun,
            label: p.name,
            start_sector: p.start_sector,
            num_sectors: p.num_sectors,
            size_bytes: p.size_bytes,
            selected: false,
        })
        .collect();

    // Bounce back to Sahara so the next `open()` on the dump pass gets
    // a fresh Hello. Without this Sahara times out.
    if let Err(e) = session.reset_to_edl(&mut log) {
        ltbox_core::live!(
            log,
            "[DumpParts] {}",
            tr_args!("live_dumpparts_reset_failed", error = e)
        );
    }

    ltbox_core::live!(
        log,
        "[DumpParts] {}",
        tr_args!("live_parts_scan_complete", count = rows.len().to_string())
    );
    DumpPartsScanResult {
        logs: log,
        rows,
        error: None,
    }
}

/// Post-dump stability window before the next EDL op. Large partition
/// reads (e.g. boot_a ~96 MB) leave the USB endpoint in a lingering state;
/// a subsequent reset/open can race a still-draining read and surface as
/// "stale COM port" or Sahara timeout. Mirrors v2 `post_sleep=15` in
/// `bin/ltbox/actions/edl.py::dump_partitions`.
const EDL_POST_DUMP_STABILIZE: std::time::Duration = std::time::Duration::from_secs(15);

/// Partition bases whose dump failure must be surfaced as a critical
/// error, not a per-row log line. These carry region/board state that a
/// subsequent rescue flow cannot reconstruct from scratch. Mirrors v2
/// `critical_targets` set in `bin/ltbox/actions/edl.py::dump_partitions`.
const CRITICAL_DUMP_BASES: &[&str] = &["devinfo", "persist", "oemowninfo"];

/// Match a partition label (possibly slot-suffixed) against the critical
/// base set. `devinfo`, `devinfo_a`, `DEVINFO_B` all match.
pub(crate) fn is_critical_dump_label(label: &str) -> bool {
    let l = label.to_ascii_lowercase();
    CRITICAL_DUMP_BASES
        .iter()
        .any(|base| l == *base || l.starts_with(&format!("{base}_")))
}

#[derive(Debug, Default)]
pub(crate) struct CountryPatchProgress {
    /// Labels that must be patched for the run to count as complete. Set
    /// per-run because the country-code partition differs by model
    /// (`devinfo` on most SKUs, `oemowninfo` on TB320FC / TB323FU).
    expected: Vec<String>,
    flashed_or_confirmed: Vec<String>,
    failures: Vec<String>,
}

impl CountryPatchProgress {
    pub(crate) fn new(expected: &[&str]) -> Self {
        Self {
            expected: expected.iter().map(|s| s.to_string()).collect(),
            ..Self::default()
        }
    }

    pub(crate) fn mark_flashed(&mut self, label: &str) {
        if !self.flashed_or_confirmed.iter().any(|seen| seen == label) {
            self.flashed_or_confirmed.push(label.to_string());
        }
    }

    pub(crate) fn mark_failed(&mut self, label: &str, reason: impl Into<String>) {
        self.failures.push(format!("{label}: {}", reason.into()));
    }

    pub(crate) fn finish(&self) -> std::result::Result<(), String> {
        let missing = self
            .expected
            .iter()
            .filter(|label| !self.flashed_or_confirmed.iter().any(|seen| seen == *label))
            .cloned()
            .collect::<Vec<_>>();

        if self.failures.is_empty() && missing.is_empty() {
            return Ok(());
        }

        let mut parts = Vec::new();
        if !self.failures.is_empty() {
            parts.push(self.failures.join("; "));
        }
        if !missing.is_empty() {
            parts.push(tr_args!(
                "country_reason_missing",
                items = missing.join(", ")
            ));
        }
        Err(tr_args!(
            "err_country_patch_incomplete",
            details = parts.join("; ")
        ))
    }
}

/// Forward buffered worker logs to the stdout tap queue immediately.
///
/// Long-running advanced actions often collect lines in a local `Vec<String>`
/// and only hand that vec back on completion, which makes the exec card look
/// stalled. Emitting lines here lets the UI drain them every 500 ms via
/// `DrainStdoutTap`.
pub(crate) fn flush_worker_logs(log: &mut Vec<String>) {
    for line in log.drain(..) {
        println!("{line}");
    }
}

/// Dump selected partitions to `output_folder` as `<label>.img`. Reopens
/// the EDL session (previous scan left device waiting at Sahara), runs
/// the reads back-to-back, then reboots to system.
///
/// Returns `Err` when any selected partition fails (critical failures use
/// the dedicated critical message). The UI must not treat a partial dump
/// as unconditional success.
pub(crate) fn dump_parts_execute(
    loader_path: String,
    output_folder: String,
    rows: Vec<DumpPartRow>,
    phases: PhaseReporter,
) -> Result<Vec<String>, String> {
    let mut log = Vec::new();
    let out_dir = std::path::PathBuf::from(&output_folder);
    if let Err(e) = std::fs::create_dir_all(&out_dir) {
        let msg = tr_args!("live_dumpparts_create_output_failed", error = e.to_string());
        ltbox_core::live!(log, "[DumpParts] {msg}");
        flush_worker_logs(&mut log);
        return Err(msg);
    }

    ltbox_core::live!(log, "[DumpParts] {}", phases.marker(1));
    std::thread::sleep(std::time::Duration::from_secs(2));
    let loader = std::path::PathBuf::from(&loader_path);
    let mut session = match ltbox_device::edl::EdlSession::open(&loader, &mut log) {
        Ok(s) => s,
        Err(e) => {
            let msg = tr_args!("err_edl_session_open_failed", error = e.to_string());
            ltbox_core::live!(log, "[DumpParts] {msg}");
            flush_worker_logs(&mut log);
            return Err(msg);
        }
    };

    ltbox_core::live!(log, "[DumpParts] {}", phases.marker(2));
    let mut critical_failures: Vec<String> = Vec::new();
    let mut all_failures: Vec<String> = Vec::new();
    for row in &rows {
        let out_path =
            match ltbox_core::safe_path::safe_join(&out_dir, &format!("{}.img", row.label)) {
                Ok(p) => p,
                Err(e) => {
                    // A device-reported GPT label is untrusted; refuse one that
                    // would escape the chosen output directory rather than
                    // writing through the traversal.
                    ltbox_core::live!(
                        log,
                        "[DumpParts] {}",
                        tr_args!(
                            "live_dumpparts_part_failed",
                            label = row.label,
                            error = e.to_string()
                        )
                    );
                    all_failures.push(row.label.clone());
                    if is_critical_dump_label(&row.label) {
                        critical_failures.push(row.label.clone());
                    }
                    continue;
                }
            };
        ltbox_core::live!(
            log,
            "[DumpParts] {}",
            tr_args!(
                "live_dumpparts_dumping",
                label = row.label,
                path = out_path.display().to_string(),
                lun = row.lun.to_string(),
                bytes = row.size_bytes.to_string()
            )
        );
        // GPT sector counts are u64; the dump path still needs a usize
        // length. Reject out-of-range counts rather than truncating.
        let dump_outcome = match usize::try_from(row.num_sectors) {
            Ok(count) => session
                .dump_partition_at(
                    &row.label,
                    &out_path,
                    row.lun,
                    row.start_sector,
                    count,
                    &mut log,
                )
                .map_err(|e| e.to_string()),
            Err(_) => Err(format!(
                "partition geometry out of range (start_sector={}, num_sectors={})",
                row.start_sector, row.num_sectors
            )),
        };
        if let Err(e) = dump_outcome {
            ltbox_core::live!(
                log,
                "[DumpParts] {}",
                tr_args!("live_dumpparts_part_failed", label = row.label, error = e)
            );
            all_failures.push(row.label.clone());
            if is_critical_dump_label(&row.label) {
                critical_failures.push(row.label.clone());
            }
        }
    }

    ltbox_core::live!(log, "[DumpParts] {}", phases.marker(3));
    ltbox_core::live!(
        log,
        "[DumpParts] {}",
        tr_args!(
            "live_dumpparts_stabilizing",
            seconds = EDL_POST_DUMP_STABILIZE.as_secs().to_string()
        )
    );
    std::thread::sleep(EDL_POST_DUMP_STABILIZE);
    ltbox_core::live!(log, "[DumpParts] {}", phases.marker(4));
    ltbox_core::live!(
        log,
        "[DumpParts] {}",
        ltbox_core::i18n::tr("live_resetting_system")
    );
    session.reset_tolerant(&mut log);
    // Surface critical/partial failures after reset so the UI can fail the
    // op; a silent "Done." would hide incomplete rescue material.
    if let Some(err) = dump_parts_outcome_error(&critical_failures, &all_failures) {
        ltbox_core::live!(log, "[DumpParts] {err}");
        flush_worker_logs(&mut log);
        return Err(err);
    }
    ltbox_core::live!(log, "[DumpParts] {}", ltbox_core::i18n::tr("live_op_done"));
    Ok(log)
}

/// Whole-LUN dump. Walks each selected LUN and writes it as
/// `lun_N.img` into `output_folder`. Unlike `dump_parts_execute` there
/// is no prior scan phase — the LUN set comes straight from the user's
/// checkboxes.
///
/// Returns `Err` on setup failure or when any selected LUN dump fails so
/// the UI does not report unconditional success for a partial dump.
pub(crate) fn dump_physical_execute(
    conn: ConnectionStatus,
    loader_path: String,
    output_folder: String,
    luns: Vec<u8>,
    phases: PhaseReporter,
) -> Result<Vec<String>, String> {
    let mut log = Vec::new();
    ltbox_core::live!(log, "[DumpPhys] {}", phases.marker(1));
    if ensure_edl(conn, "DumpPhys", &mut log).is_err() {
        flush_worker_logs(&mut log);
        return Err(ltbox_core::i18n::tr("err_edl_transition_failed"));
    }
    flush_worker_logs(&mut log);
    let out_dir = std::path::PathBuf::from(&output_folder);
    if let Err(e) = std::fs::create_dir_all(&out_dir) {
        let msg = tr_args!("live_dump_phys_create_output_failed", error = e.to_string());
        ltbox_core::live!(log, "[DumpPhys] {msg}");
        flush_worker_logs(&mut log);
        return Err(msg);
    }

    ltbox_core::live!(log, "[DumpPhys] {}", phases.marker(2));
    std::thread::sleep(std::time::Duration::from_secs(2));
    let loader = std::path::PathBuf::from(&loader_path);
    let mut session = match ltbox_device::edl::EdlSession::open(&loader, &mut log) {
        Ok(s) => s,
        Err(e) => {
            let msg = tr_args!("err_edl_session_open_failed", error = e.to_string());
            ltbox_core::live!(log, "[DumpPhys] {msg}");
            flush_worker_logs(&mut log);
            return Err(msg);
        }
    };
    flush_worker_logs(&mut log);

    ltbox_core::live!(log, "[DumpPhys] {}", phases.marker(3));
    let mut failure_msgs: Vec<String> = Vec::new();
    for lun in &luns {
        let out_path = out_dir.join(format!("lun_{lun}.img"));
        ltbox_core::live!(
            log,
            "[DumpPhys] {}",
            tr_args!(
                "live_dump_phys_dumping_lun",
                lun = lun.to_string(),
                path = out_path.display().to_string()
            )
        );
        flush_worker_logs(&mut log);
        if let Err(e) = session.dump_physical_storage(*lun, &out_path, &mut log) {
            let msg = tr_args!(
                "live_dump_phys_lun_failed",
                lun = lun.to_string(),
                error = e.to_string()
            );
            ltbox_core::live!(log, "[DumpPhys] {msg}");
            failure_msgs.push(msg);
        }
        flush_worker_logs(&mut log);
    }

    ltbox_core::live!(log, "[DumpPhys] {}", phases.marker(4));
    ltbox_core::live!(
        log,
        "[DumpPhys] {}",
        tr_args!(
            "live_dump_phys_stabilizing_usb",
            seconds = EDL_POST_DUMP_STABILIZE.as_secs().to_string()
        )
    );
    flush_worker_logs(&mut log);
    std::thread::sleep(EDL_POST_DUMP_STABILIZE);
    ltbox_core::live!(log, "[DumpPhys] {}", phases.marker(5));
    ltbox_core::live!(
        log,
        "[DumpPhys] {}",
        ltbox_core::i18n::tr("live_resetting_system")
    );
    session.reset_tolerant(&mut log);
    if !failure_msgs.is_empty() {
        let err = failure_msgs.join("; ");
        ltbox_core::live!(log, "[DumpPhys] {err}");
        flush_worker_logs(&mut log);
        return Err(err);
    }
    ltbox_core::live!(log, "[DumpPhys] {}", ltbox_core::i18n::tr("live_op_done"));
    flush_worker_logs(&mut log);
    Ok(log)
}

/// Whole-LUN raw flash. Each `(lun, path)` pair is written verbatim
/// from sector 0. Mirrors qdlrs `OverwriteStorage`.
pub(crate) fn flash_physical_execute(
    conn: ConnectionStatus,
    loader_path: String,
    pairs: Vec<(u8, String)>,
    phases: PhaseReporter,
) -> Result<Vec<String>, String> {
    let mut log = Vec::new();
    // Fail closed before EDL transition / any write when a selected image
    // is missing on disk.
    if let Err(e) = preflight_flash_phys_sources(&pairs) {
        ltbox_core::live!(log, "[FlashPhys] {e}");
        return Err(e);
    }
    ltbox_core::live!(log, "[FlashPhys] {}", phases.marker(1));
    if ensure_edl(conn, "FlashPhys", &mut log).is_err() {
        return Err(ltbox_core::i18n::tr("err_edl_transition_failed"));
    }

    ltbox_core::live!(log, "[FlashPhys] {}", phases.marker(2));
    std::thread::sleep(std::time::Duration::from_secs(2));
    let loader = std::path::PathBuf::from(&loader_path);
    let mut session = match ltbox_device::edl::EdlSession::open(&loader, &mut log) {
        Ok(s) => s,
        Err(e) => {
            ltbox_core::live!(
                log,
                "[FlashPhys] {}",
                tr_args!("err_edl_session_open_failed", error = e.to_string())
            );
            ltbox_core::live!(
                log,
                "{}",
                ltbox_core::i18n::tr("live_edl_open_failed_reboot_notice")
            );
            return Err(tr_args!(
                "err_edl_session_open_failed",
                error = e.to_string()
            ));
        }
    };

    ltbox_core::live!(log, "[FlashPhys] {}", phases.marker(3));
    for (lun, path) in &pairs {
        let img = std::path::Path::new(path);
        if !img.is_file() {
            // Preflight already checked; re-check is a hard failure.
            return Err(format!(
                "LUN {lun}: {}",
                tr_args!("err_path_missing", path = path)
            ));
        }
        let file_name = img
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.clone());
        ltbox_core::live!(
            log,
            "[FlashPhys] {}",
            tr_args!(
                "live_flashphys_flashing",
                lun = lun.to_string(),
                file = file_name
            )
        );
        phases.mark_writes_started();
        if let Err(e) = session.flash_physical_storage(*lun, img, &mut log) {
            ltbox_core::live!(
                log,
                "[FlashPhys] {}",
                tr_args!(
                    "live_flashphys_lun_failed",
                    lun = lun.to_string(),
                    error = e.to_string()
                )
            );
            // Abort remaining LUN writes; device stays in EDL for retry.
            return Err(tr_args!(
                "err_flash_phys_write_failed",
                lun = lun.to_string(),
                error = e.to_string()
            ));
        }
    }

    ltbox_core::live!(log, "[FlashPhys] {}", phases.marker(4));
    ltbox_core::live!(
        log,
        "[FlashPhys] {}",
        ltbox_core::i18n::tr("live_resetting_system")
    );
    session.reset_tolerant(&mut log);
    ltbox_core::live!(log, "[FlashPhys] {}", ltbox_core::i18n::tr("live_op_done"));
    Ok(log)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flash_row(label: &str, path: Option<&str>, state: FlashRowState) -> FlashPartRow {
        FlashPartRow {
            lun: 0,
            label: label.to_string(),
            start_sector: 0,
            num_sectors: 1,
            size_bytes: 4096,
            file_path: path.map(|p| p.to_string()),
            state,
        }
    }

    #[test]
    fn preflight_flash_part_sources_fails_when_missing() {
        let rows = vec![flash_row(
            "boot_a",
            Some("Z:/ltbox-definitely-missing-boot_a.img"),
            FlashRowState::Write,
        )];
        let err = preflight_flash_part_sources(&rows).expect_err("missing image must fail");
        assert!(err.contains("boot_a"), "err={err}");
        assert!(
            err.contains("ltbox-definitely-missing-boot_a.img") || err.contains("err_path_missing"),
            "err={err}"
        );
    }

    #[test]
    fn preflight_flash_part_sources_ok_when_file_exists() {
        let dir = tempfile::tempdir().unwrap();
        let img = dir.path().join("boot_a.img");
        std::fs::write(&img, b"x").unwrap();
        let rows = vec![flash_row(
            "boot_a",
            Some(img.to_str().unwrap()),
            FlashRowState::Write,
        )];
        preflight_flash_part_sources(&rows).expect("existing image must pass");
    }

    #[test]
    fn preflight_flash_part_sources_ignores_erase_rows() {
        let rows = vec![flash_row("userdata", None, FlashRowState::Erase)];
        preflight_flash_part_sources(&rows).expect("erase rows need no source path");
    }

    #[test]
    fn preflight_flash_part_sources_rejects_directory() {
        let dir = tempfile::tempdir().unwrap();
        let rows = vec![flash_row(
            "boot_a",
            Some(dir.path().to_str().unwrap()),
            FlashRowState::Write,
        )];

        let err = preflight_flash_part_sources(&rows)
            .expect_err("directory must not pass as a partition image");
        assert!(err.contains("boot_a"), "err={err}");
    }

    #[test]
    fn preflight_flash_phys_sources_fails_when_missing() {
        let pairs = vec![(0u8, "Z:/ltbox-definitely-missing-lun0.img".to_string())];
        let err = preflight_flash_phys_sources(&pairs).expect_err("missing LUN image must fail");
        assert!(err.contains("LUN 0"), "err={err}");
        assert!(
            err.contains("ltbox-definitely-missing-lun0.img") || err.contains("err_path_missing"),
            "err={err}"
        );
    }

    #[test]
    fn preflight_flash_phys_sources_rejects_directory() {
        let dir = tempfile::tempdir().unwrap();
        let pairs = vec![(0u8, dir.path().to_string_lossy().into_owned())];

        let err = preflight_flash_phys_sources(&pairs)
            .expect_err("directory must not pass as a physical image");
        assert!(err.contains("LUN 0"), "err={err}");
    }

    #[test]
    fn dump_parts_outcome_surfaces_critical_and_partial() {
        assert!(dump_parts_outcome_error(&[], &[]).is_none());

        let critical = dump_parts_outcome_error(
            &["persist".to_string()],
            &["persist".to_string(), "modem".to_string()],
        )
        .expect("critical must surface");
        assert!(critical.contains("persist"), "err={critical}");

        let partial =
            dump_parts_outcome_error(&[], &["modem".to_string()]).expect("partial must surface");
        assert!(partial.contains("modem"), "err={partial}");
    }

    #[test]
    fn is_critical_dump_label_matches_bases_and_slots() {
        assert!(is_critical_dump_label("persist"));
        assert!(is_critical_dump_label("devinfo_a"));
        assert!(is_critical_dump_label("OEMOWNINFO_B"));
        assert!(!is_critical_dump_label("boot_a"));
    }
}
