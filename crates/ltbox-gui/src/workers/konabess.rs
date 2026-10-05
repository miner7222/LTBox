//! KonaBess workers: inspect stock images, pause for UI target/table review,
//! selection, then rebuild and flash the AVB-matched image pair.

use crate::{
    ConnectionStatus, KonaBessPrepared, LiveLabels, PhaseReporter, open_edl_session,
    prepare_canoe_efisp, provision_canoe_efisp, transition_to_edl,
};
use ltbox_core::{live, tr_args};
use ltbox_patch::konabess::{GpuTable, KonaBessAvbOutput, KonaBessBuildStage, VendorBootDtbInfo};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub(crate) struct KonaBessInspectionResult {
    pub(crate) prepared: KonaBessPrepared,
    pub(crate) candidates: Vec<VendorBootDtbInfo>,
    pub(crate) log: Vec<String>,
}

struct InspectionPaths {
    work_dir: PathBuf,
    backup_root: Option<PathBuf>,
    device_model: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExploitGateKind {
    SignedVbmeta,
    EfispGbl,
}

fn exploit_gate_kind(
    image_capabilities: Option<&ltbox_core::model::ModelCapabilities>,
    fallback_uses_gbl: bool,
) -> ExploitGateKind {
    let uses_gbl = image_capabilities
        .map(|capabilities| capabilities.root_uses_gbl)
        .unwrap_or(fallback_uses_gbl);
    if uses_gbl {
        ExploitGateKind::EfispGbl
    } else {
        ExploitGateKind::SignedVbmeta
    }
}

/// What a vendor_boot fingerprint settles about the image's profile.
struct ImageIdentity {
    /// Profile deciding the exploit route; `None` when no fingerprint is
    /// available or it names no model and the detected model is not known.
    capabilities: Option<&'static ltbox_core::model::ModelCapabilities>,
    /// Set when the fingerprint names no known model and the detected model's
    /// profile stands in for it.
    unidentified_as: Option<&'static str>,
}

/// Resolve the profile for an image dumped from the connected device. A
/// fingerprint naming no known model cannot contradict the device, so a known
/// detected model's profile is used instead; a fingerprint naming a model keeps
/// its own profile, preferring a GBL one as before.
fn image_identity(fingerprint: Option<&str>, device_model: &str) -> ImageIdentity {
    use ltbox_core::model::{
        capabilities, fingerprint_capabilities, fingerprint_names_known_model, known_model,
    };
    match fingerprint {
        None => ImageIdentity {
            capabilities: None,
            unidentified_as: None,
        },
        Some(fp) if !fingerprint_names_known_model(fp) => {
            let model = known_model(device_model);
            ImageIdentity {
                capabilities: model.map(capabilities),
                unidentified_as: model,
            }
        }
        Some(fp) => {
            let mut profiles = fingerprint_capabilities(fp);
            let first = profiles.next();
            ImageIdentity {
                capabilities: profiles.find(|p| p.root_uses_gbl).or(first),
                unidentified_as: None,
            }
        }
    }
}

trait KonaBessInspectionBackend {
    fn resolve_active_slot(&mut self, log: &mut Vec<String>) -> Result<String, String>;
    fn resolve_probable_dtb_index(&mut self, log: &mut Vec<String>) -> Option<usize>;
    fn enter_edl(&mut self, log: &mut Vec<String>) -> Result<(), String>;
    fn dump_partition(
        &mut self,
        partition: &str,
        destination: &Path,
        log: &mut Vec<String>,
    ) -> Result<(), String>;
    fn run_exploit_gate(
        &mut self,
        slot_suffix: &str,
        vendor_boot: &Path,
        vbmeta: &Path,
        work_dir: &Path,
        log: &mut Vec<String>,
    ) -> Result<(), String>;
    fn inspect_gpu_candidates(
        &mut self,
        vendor_boot: &Path,
    ) -> Result<Vec<VendorBootDtbInfo>, String>;
    fn prepare_for_selection(&mut self, log: &mut Vec<String>) -> Result<(), String>;
    fn recover_after_error(&mut self, log: &mut Vec<String>);
}

struct DeviceBackend<'a> {
    conn: ConnectionStatus,
    loader: &'a Path,
    uses_gbl: bool,
    device_model: &'a str,
    phases: &'a PhaseReporter,
    session: Option<ltbox_device::edl::EdlSession>,
    writes_started: bool,
}

impl DeviceBackend<'_> {
    fn session(&mut self) -> Result<&mut ltbox_device::edl::EdlSession, String> {
        self.session.as_mut().ok_or_else(|| {
            tr_args!(
                "err_edl_session_open_failed",
                error = ltbox_core::i18n::tr("err_task_failed")
            )
        })
    }
}

impl KonaBessInspectionBackend for DeviceBackend<'_> {
    fn resolve_active_slot(&mut self, log: &mut Vec<String>) -> Result<String, String> {
        ltbox_device::controller::poll_active_slot(std::time::Duration::from_secs(30), log)
            .map_err(|error| error.to_string())
    }

    fn resolve_probable_dtb_index(&mut self, _log: &mut Vec<String>) -> Option<usize> {
        let mut adb = ltbox_device::adb::AdbManager::new_if_connected()?;
        adb.shell("getprop ro.boot.dtb_idx")
            .ok()?
            .trim()
            .parse()
            .ok()
    }

    fn enter_edl(&mut self, log: &mut Vec<String>) -> Result<(), String> {
        transition_to_edl(self.conn, log)?;
        self.session = Some(open_edl_session(self.loader, log)?);
        Ok(())
    }

    fn dump_partition(
        &mut self,
        partition: &str,
        destination: &Path,
        log: &mut Vec<String>,
    ) -> Result<(), String> {
        let lun = ltbox_core::partition_lun::lun_for_partition(partition).unwrap_or(4);
        self.session()?
            .dump_partition(partition, destination, 0, lun, log)
            .map_err(|error| {
                tr_args!(
                    "err_root_dump_partition_failed",
                    partition = partition,
                    error = error
                )
            })
    }

    fn run_exploit_gate(
        &mut self,
        slot_suffix: &str,
        vendor_boot: &Path,
        vbmeta: &Path,
        work_dir: &Path,
        log: &mut Vec<String>,
    ) -> Result<(), String> {
        let fingerprint = ltbox_patch::avb::extract_image_avb_info(vendor_boot)
            .ok()
            .and_then(|info| ltbox_patch::avb::build_fingerprint(&info));
        let identity = image_identity(fingerprint.as_deref(), self.device_model);
        if let (Some(model), Some(fp)) = (identity.unidentified_as, fingerprint.as_deref()) {
            live!(
                log,
                "[KonaBess] {}",
                tr_args!(
                    "live_image_fingerprint_unidentified",
                    fingerprint = fp,
                    model = model
                )
            );
        }
        let image_capabilities = identity.capabilities;
        let unsupported = (!ltbox_core::model::capabilities(self.device_model).konabess)
            .then_some(self.device_model)
            .or_else(|| {
                fingerprint
                    .as_deref()
                    .and_then(|fp| ltbox_core::model::fingerprint_model_lacking(fp, |p| p.konabess))
            });
        if let Some(model) = unsupported {
            return Err(tr_args!("model_unsupported", model = model));
        }
        match exploit_gate_kind(image_capabilities, self.uses_gbl) {
            ExploitGateKind::EfispGbl => {
                let efi_dir = work_dir.join("efisp_gbl");
                let staged = prepare_canoe_efisp(
                    self.session()?,
                    slot_suffix,
                    Some(vendor_boot),
                    work_dir,
                    &efi_dir,
                    log,
                )?;
                self.writes_started = staged.is_some();
                if self.writes_started {
                    self.phases.mark_writes_started();
                }
                provision_canoe_efisp(self.session()?, staged.as_deref(), log)?;
                Ok(())
            }
            ExploitGateKind::SignedVbmeta => {
                let info = ltbox_patch::avb::extract_image_avb_info(vbmeta)
                    .map_err(|error| error.to_string())?;
                validate_signing_key(info.public_key_sha1.as_deref())
            }
        }
    }

    fn inspect_gpu_candidates(
        &mut self,
        vendor_boot: &Path,
    ) -> Result<Vec<VendorBootDtbInfo>, String> {
        let image = std::fs::read(vendor_boot).map_err(|error| error.to_string())?;
        ltbox_patch::konabess::inspect_vendor_boot_gpu_candidates(&image)
            .map_err(|error| error.to_string())
    }

    fn prepare_for_selection(&mut self, log: &mut Vec<String>) -> Result<(), String> {
        self.session()?
            .reset_to_edl(log)
            .map_err(|error| error.to_string())?;
        self.session = None;
        Ok(())
    }

    fn recover_after_error(&mut self, log: &mut Vec<String>) {
        if let Some(session) = self.session.as_mut() {
            session.reset_tolerant(log);
            self.session = None;
            return;
        }
        if let Ok(mut session) = ltbox_device::edl::EdlSession::open(self.loader, log) {
            session.reset_tolerant(log);
        }
    }
}

fn validate_signing_key(pubkey_sha1: Option<&str>) -> Result<(), String> {
    ltbox_patch::key_map::key_spec_for_signed_pubkey(pubkey_sha1)
        .map(|_| ())
        .map_err(|key| ltbox_patch::key_map::unresolved_signing_key_error("vbmeta.img", &key))
}

fn persist_backup(vendor_boot: &Path, vbmeta: &Path, backup_dir: &Path) -> Result<(), String> {
    std::fs::copy(vendor_boot, backup_dir.join("vendor_boot.img"))
        .map_err(|error| error.to_string())?;
    std::fs::copy(vbmeta, backup_dir.join("vbmeta.img")).map_err(|error| error.to_string())?;
    Ok(())
}

fn backup_fingerprint(vendor_boot: &Path) -> Option<String> {
    ltbox_patch::avb::extract_image_avb_info(vendor_boot)
        .ok()
        .and_then(|info| ltbox_patch::avb::build_fingerprint(&info))
}

fn execute_inspection<B: KonaBessInspectionBackend>(
    backend: &mut B,
    paths: &InspectionPaths,
    phases: &PhaseReporter,
    log: &mut Vec<String>,
) -> Result<(KonaBessPrepared, Vec<VendorBootDtbInfo>), String> {
    live!(log, "[KonaBess] {}", phases.marker(1));

    // This probe is intentionally the first device operation. EDL cannot
    // report an active Android slot, and failure must not fall back to `_a`.
    let slot_suffix = backend.resolve_active_slot(log)?;
    // Upstream KonaBess uses this Android-only property as a selection hint.
    // Missing or malformed values are normal and must not block inspection.
    let probable_dtb_index = backend.resolve_probable_dtb_index(log);
    backend.enter_edl(log)?;

    live!(log, "[KonaBess] {}", phases.marker(2));
    let vendor_boot_partition = format!("vendor_boot{slot_suffix}");
    let vbmeta_partition = format!("vbmeta{slot_suffix}");
    let vendor_boot = paths.work_dir.join("vendor_boot.img");
    let vbmeta = paths.work_dir.join("vbmeta.img");
    backend.dump_partition(&vendor_boot_partition, &vendor_boot, log)?;
    backend.dump_partition(&vbmeta_partition, &vbmeta, log)?;

    // Gate only after both source images exist. TB323FU takes the shared efisp
    // path; every other model resolves vbmeta through KEY_MAP and permits an
    // absent key as unsigned.
    backend.run_exploit_gate(&slot_suffix, &vendor_boot, &vbmeta, &paths.work_dir, log)?;

    live!(log, "[KonaBess] {}", phases.marker(3));
    let candidates = backend.inspect_gpu_candidates(&vendor_boot)?;

    // Return Firehose to Sahara so part 2 can open a fresh session after the
    // UI selection pause. Backup creation is last, making it success-only.
    backend.prepare_for_selection(log)?;
    let backup_dir = paths.backup_root.as_deref().map_or_else(
        || crate::backup::create_backup_dir("konabess", &paths.device_model),
        |root| crate::backup::create_backup_dir_in(root, "konabess", &paths.device_model),
    )?;
    persist_backup(&vendor_boot, &vbmeta, &backup_dir)?;
    let fingerprint = backup_fingerprint(&backup_dir.join("vendor_boot.img"));
    if let Err(error) = crate::backup::write_backup_manifest(
        &backup_dir,
        "konabess",
        &paths.device_model,
        fingerprint.as_deref(),
        Some(&slot_suffix),
    ) {
        live!(
            log,
            "[Backup] {}",
            ltbox_core::tr_args!("log_backup_manifest_failed", error = error)
        );
    }

    Ok((
        KonaBessPrepared {
            work_dir: paths.work_dir.clone(),
            vendor_boot,
            vbmeta,
            backup_dir,
            slot_suffix,
            probable_dtb_index,
            device_model: paths.device_model.clone(),
        },
        candidates,
    ))
}

pub(crate) fn konabess_inspection_worker(
    conn: ConnectionStatus,
    loader: PathBuf,
    uses_gbl: bool,
    device_model: String,
    ll: LiveLabels,
    phases: PhaseReporter,
) -> Result<KonaBessInspectionResult, String> {
    let mut log = Vec::new();
    if !ltbox_core::model::capabilities(&device_model).konabess {
        return Err(tr_args!("model_unsupported", model = device_model.as_str()));
    }
    let work_dir = ltbox_core::app_paths::work_dir_for("konabess");
    let _ = std::fs::remove_dir_all(&work_dir);
    std::fs::create_dir_all(&work_dir).map_err(|error| error.to_string())?;
    let paths = InspectionPaths {
        work_dir: work_dir.clone(),
        backup_root: None,
        device_model: device_model.clone(),
    };
    let mut backend = DeviceBackend {
        conn,
        loader: &loader,
        uses_gbl,
        device_model: &device_model,
        phases: &phases,
        session: None,
        writes_started: false,
    };

    match execute_inspection(&mut backend, &paths, &phases, &mut log) {
        Ok((prepared, candidates)) => {
            live!(log, "[KonaBess] {}", ll.backup_saved(&prepared.backup_dir));
            Ok(KonaBessInspectionResult {
                prepared,
                candidates,
                log,
            })
        }
        Err(error) => {
            let error = if backend.writes_started {
                tr_args!("err_root_partial_write_recovery", error = error)
            } else {
                backend.recover_after_error(&mut log);
                error
            };
            let _ = std::fs::remove_dir_all(&work_dir);
            Err(error)
        }
    }
}

trait KonaBessFlashBackend {
    fn build_pair(
        &mut self,
        firmware_dir: &Path,
        output_dir: &Path,
        edit: KonaBessTableEdit<'_>,
        on_stage: &mut dyn FnMut(KonaBessBuildStage),
    ) -> Result<KonaBessAvbOutput, String>;
    fn open_session(&mut self, log: &mut Vec<String>) -> Result<(), String>;
    fn verify_abl(
        &mut self,
        prepared: &KonaBessPrepared,
        fresh: bool,
        log: &mut Vec<String>,
    ) -> Result<(), String>;
    fn flash_batch(
        &mut self,
        requests: &[ltbox_device::edl::PartitionFlash<'_>],
        phases: &PhaseReporter,
        log: &mut Vec<String>,
    ) -> Result<(), String>;
    fn reboot(&mut self, log: &mut Vec<String>);
    fn writes_started(&self) -> bool;
    fn recover_before_write(&mut self, log: &mut Vec<String>);
}

struct FlashDeviceBackend<'a> {
    loader: &'a Path,
    session: Option<ltbox_device::edl::EdlSession>,
    writes_started: bool,
}

impl FlashDeviceBackend<'_> {
    fn session(&mut self) -> Result<&mut ltbox_device::edl::EdlSession, String> {
        self.session.as_mut().ok_or_else(|| {
            tr_args!(
                "err_edl_session_open_failed",
                error = ltbox_core::i18n::tr("err_task_failed")
            )
        })
    }
}

impl KonaBessFlashBackend for FlashDeviceBackend<'_> {
    fn build_pair(
        &mut self,
        firmware_dir: &Path,
        output_dir: &Path,
        edit: KonaBessTableEdit<'_>,
        on_stage: &mut dyn FnMut(KonaBessBuildStage),
    ) -> Result<KonaBessAvbOutput, String> {
        ltbox_patch::konabess::build_konabess_avb_images_from_table_with_progress(
            firmware_dir,
            output_dir,
            edit.target_index,
            edit.chip,
            edit.table,
            edit.gbl_verified,
            on_stage,
        )
        .map_err(|error| error.to_string())
    }

    fn verify_abl(
        &mut self,
        prepared: &KonaBessPrepared,
        fresh: bool,
        log: &mut Vec<String>,
    ) -> Result<(), String> {
        if fresh {
            crate::dump_verified_active_abl(
                self.session()?,
                &prepared.slot_suffix,
                &prepared.work_dir.join("abl-current.img"),
                log,
            )
        } else {
            crate::verify_abl_efisp(&prepared.work_dir.join("abl.img"))
        }
    }

    fn open_session(&mut self, log: &mut Vec<String>) -> Result<(), String> {
        self.session = Some(open_edl_session(self.loader, log)?);
        Ok(())
    }

    fn flash_batch(
        &mut self,
        requests: &[ltbox_device::edl::PartitionFlash<'_>],
        phases: &PhaseReporter,
        log: &mut Vec<String>,
    ) -> Result<(), String> {
        let session = self
            .session
            .as_mut()
            .ok_or_else(|| ltbox_core::i18n::tr("err_task_failed"))?;
        session
            .flash_partition_batch(
                requests,
                log,
                |_, _, _| {},
                || {
                    self.writes_started = true;
                    phases.mark_writes_started();
                },
            )
            .map_err(|error| {
                tr_args!(
                    "err_root_flash_partition_failed",
                    partition = error.partition,
                    error = error.source
                )
            })
    }

    fn reboot(&mut self, log: &mut Vec<String>) {
        if let Some(session) = self.session.as_mut() {
            session.reset_tolerant(log);
        }
        self.session = None;
    }

    fn writes_started(&self) -> bool {
        self.writes_started
    }

    fn recover_before_write(&mut self, log: &mut Vec<String>) {
        if let Some(session) = self.session.as_mut() {
            session.reset_tolerant(log);
            self.session = None;
            return;
        }
        if let Ok(mut session) = ltbox_device::edl::EdlSession::open(self.loader, log) {
            session.reset_tolerant(log);
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct KonaBessTableEdit<'a> {
    target_index: usize,
    chip: &'a str,
    table: &'a GpuTable,
    /// GBL route requested by the inspected firmware. Preserved and fresh
    /// active-slot ABL evidence is required before skipping AVB and writing.
    gbl_verified: bool,
}

fn execute_flash<B: KonaBessFlashBackend>(
    backend: &mut B,
    prepared: &KonaBessPrepared,
    edit: KonaBessTableEdit<'_>,
    phases: &PhaseReporter,
    log: &mut Vec<String>,
) -> Result<(), String> {
    if edit.gbl_verified {
        backend.verify_abl(prepared, false, log)?;
    }
    let output_dir = prepared.work_dir.join("rebuilt");
    let output = backend.build_pair(&prepared.work_dir, &output_dir, edit, &mut |stage| {
        live!(
            log,
            "[KonaBess] {}",
            phases.marker(crate::konabess_build_phase(stage))
        );
    })?;

    let vendor_boot_partition = format!("vendor_boot{}", prepared.slot_suffix);
    let vbmeta_partition = format!("vbmeta{}", prepared.slot_suffix);
    let vendor_boot_lun = ltbox_core::partition_lun::lun_for_partition(&vendor_boot_partition)
        .ok_or_else(|| {
            tr_args!(
                "err_no_hardcoded_lun",
                partition = vendor_boot_partition.as_str()
            )
        })?;
    let vbmeta_lun =
        ltbox_core::partition_lun::lun_for_partition(&vbmeta_partition).ok_or_else(|| {
            tr_args!(
                "err_no_hardcoded_lun",
                partition = vbmeta_partition.as_str()
            )
        })?;

    live!(log, "[KonaBess] {}", phases.marker(6));
    backend.open_session(log)?;
    if edit.gbl_verified {
        backend.verify_abl(prepared, true, log)?;
    }
    let mut requests = vec![ltbox_device::edl::PartitionFlash {
        label: &vendor_boot_partition,
        image: &output.vendor_boot,
        slot: 0,
        lun: vendor_boot_lun,
    }];
    // The GBL route does not rebuild vbmeta.
    if let Some(vbmeta) = output.vbmeta.as_deref() {
        requests.push(ltbox_device::edl::PartitionFlash {
            label: &vbmeta_partition,
            image: vbmeta,
            slot: 0,
            lun: vbmeta_lun,
        });
    }
    backend.flash_batch(&requests, phases, log)?;

    // The reset is deliberately unreachable until both members of the
    // AVB-matched pair have completed in this same backend session.
    live!(log, "[KonaBess] {}", phases.marker(7));
    backend.reboot(log);
    // Not the firmware label: this run wrote vendor_boot and vbmeta.
    live!(
        log,
        "[KonaBess] {}",
        ltbox_core::i18n::tr("live_image_flash_completed")
    );
    Ok(())
}

fn run_flash<B: KonaBessFlashBackend>(
    backend: &mut B,
    prepared: &KonaBessPrepared,
    edit: KonaBessTableEdit<'_>,
    phases: &PhaseReporter,
    log: &mut Vec<String>,
) -> Result<(), String> {
    match execute_flash(backend, prepared, edit, phases, log) {
        Ok(()) => Ok(()),
        Err(error) if backend.writes_started() => {
            // Never reset a device that may contain only one member of the
            // rebuilt AVB pair. The retained backup is the recovery source.
            Err(tr_args!(
                "err_konabess_partial_write_recovery",
                error = error
            ))
        }
        Err(error) => {
            backend.recover_before_write(log);
            Err(error)
        }
    }
}

pub(crate) fn konabess_flash_worker(
    loader: PathBuf,
    prepared: KonaBessPrepared,
    target_index: usize,
    chip: String,
    table: GpuTable,
    phases: PhaseReporter,
) -> Result<Vec<String>, String> {
    let mut log = Vec::new();
    let prepared_fingerprint = ltbox_patch::avb::extract_image_avb_info(&prepared.vendor_boot)
        .ok()
        .and_then(|info| ltbox_patch::avb::build_fingerprint(&info));
    if let Some(model) = prepared_fingerprint
        .as_deref()
        .and_then(|fp| ltbox_core::model::fingerprint_model_lacking(fp, |p| p.konabess))
    {
        return Err(tr_args!("model_unsupported", model = model));
    }
    // The fingerprint selects the route (the detected model's profile when it
    // names none); execute_flash separately verifies the inspection ABL and
    // the current session ABL before trusting it.
    let identity = image_identity(prepared_fingerprint.as_deref(), &prepared.device_model);
    if let (Some(model), Some(fp)) = (identity.unidentified_as, prepared_fingerprint.as_deref()) {
        if !ltbox_core::model::capabilities(model).konabess {
            return Err(tr_args!("model_unsupported", model = model));
        }
        live!(
            log,
            "[KonaBess] {}",
            tr_args!(
                "live_image_fingerprint_unidentified",
                fingerprint = fp,
                model = model
            )
        );
    }
    let gbl_verified = identity
        .capabilities
        .is_some_and(|profile| profile.root_uses_gbl);
    let mut backend = FlashDeviceBackend {
        loader: &loader,
        session: None,
        writes_started: false,
    };
    run_flash(
        &mut backend,
        &prepared,
        KonaBessTableEdit {
            target_index,
            chip: &chip,
            table: &table,
            gbl_verified,
        },
        &phases,
        &mut log,
    )?;
    Ok(log)
}

trait KonaBessCancelBackend {
    fn reboot_to_system(&mut self, log: &mut Vec<String>);
}

struct CancelDeviceBackend<'a> {
    loader: &'a Path,
}

impl KonaBessCancelBackend for CancelDeviceBackend<'_> {
    fn reboot_to_system(&mut self, log: &mut Vec<String>) {
        // Inspection returned Firehose to Sahara, so cancellation opens a
        // fresh session exactly like the pre-write recovery path.
        if let Ok(mut session) = ltbox_device::edl::EdlSession::open(self.loader, log) {
            session.reset_tolerant(log);
        }
    }
}

fn execute_cancel<B: KonaBessCancelBackend>(backend: &mut B, log: &mut Vec<String>) {
    backend.reboot_to_system(log);
}

pub(crate) fn konabess_cancel_worker(loader: PathBuf) -> Vec<String> {
    let mut log = Vec::new();
    let mut backend = CancelDeviceBackend { loader: &loader };
    execute_cancel(&mut backend, &mut log);
    log
}

#[cfg(test)]
mod tests {
    use super::*;
    use ltbox_patch::konabess::VendorBootDtbInfo;

    #[derive(Default)]
    struct FakeBackend {
        events: Vec<String>,
        gate_error: Option<String>,
        candidate: Option<VendorBootDtbInfo>,
        probable_dtb_index: Option<usize>,
    }

    impl KonaBessInspectionBackend for FakeBackend {
        fn resolve_active_slot(&mut self, _log: &mut Vec<String>) -> Result<String, String> {
            self.events.push("slot".into());
            Ok("_b".into())
        }

        fn resolve_probable_dtb_index(&mut self, _log: &mut Vec<String>) -> Option<usize> {
            self.events.push("dtb-index".into());
            self.probable_dtb_index
        }

        fn enter_edl(&mut self, _log: &mut Vec<String>) -> Result<(), String> {
            self.events.push("edl".into());
            Ok(())
        }

        fn dump_partition(
            &mut self,
            partition: &str,
            destination: &Path,
            _log: &mut Vec<String>,
        ) -> Result<(), String> {
            self.events.push(format!("dump:{partition}"));
            std::fs::write(destination, partition).map_err(|error| error.to_string())
        }

        fn run_exploit_gate(
            &mut self,
            _slot_suffix: &str,
            _vendor_boot: &Path,
            _vbmeta: &Path,
            _work_dir: &Path,
            _log: &mut Vec<String>,
        ) -> Result<(), String> {
            self.events.push("gate".into());
            self.gate_error.take().map_or(Ok(()), Err)
        }

        fn inspect_gpu_candidates(
            &mut self,
            vendor_boot: &Path,
        ) -> Result<Vec<VendorBootDtbInfo>, String> {
            assert_eq!(
                std::fs::read_to_string(vendor_boot).unwrap(),
                "vendor_boot_b"
            );
            self.events.push("classify".into());
            Ok(self.candidate.take().into_iter().collect())
        }

        fn prepare_for_selection(&mut self, _log: &mut Vec<String>) -> Result<(), String> {
            self.events.push("pause".into());
            Ok(())
        }

        fn recover_after_error(&mut self, _log: &mut Vec<String>) {
            self.events.push("recover".into());
        }
    }

    #[derive(Default)]
    struct FakeFlashBackend {
        events: Vec<String>,
        build_error: Option<String>,
        abl_error: Option<bool>,
        fail_second_write: bool,
        session_open: bool,
        writes_started: bool,
    }

    impl KonaBessFlashBackend for FakeFlashBackend {
        fn build_pair(
            &mut self,
            firmware_dir: &Path,
            output_dir: &Path,
            edit: KonaBessTableEdit<'_>,
            on_stage: &mut dyn FnMut(KonaBessBuildStage),
        ) -> Result<KonaBessAvbOutput, String> {
            let KonaBessTableEdit {
                target_index,
                gbl_verified,
                ..
            } = edit;
            self.events.push(format!("build:{target_index}"));
            if let Some(error) = self.build_error.take() {
                return Err(error);
            }
            assert_eq!(output_dir, firmware_dir.join("rebuilt"));
            on_stage(KonaBessBuildStage::Inspect);
            on_stage(KonaBessBuildStage::PatchVendorBoot);
            on_stage(KonaBessBuildStage::RebuildVbmeta);
            Ok(KonaBessAvbOutput {
                vendor_boot: output_dir.join("vendor_boot.img"),
                vbmeta: (!gbl_verified).then(|| output_dir.join("vbmeta.img")),
                target_index,
            })
        }

        fn verify_abl(
            &mut self,
            _prepared: &KonaBessPrepared,
            fresh: bool,
            _log: &mut Vec<String>,
        ) -> Result<(), String> {
            assert_eq!(self.session_open, fresh);
            self.events.push(format!("abl:{fresh}"));
            if self.abl_error == Some(fresh) {
                Err("ABL rejected".into())
            } else {
                Ok(())
            }
        }

        fn open_session(&mut self, _log: &mut Vec<String>) -> Result<(), String> {
            assert!(!self.session_open);
            self.session_open = true;
            self.events.push("open".into());
            Ok(())
        }

        fn flash_batch(
            &mut self,
            requests: &[ltbox_device::edl::PartitionFlash<'_>],
            phases: &PhaseReporter,
            _log: &mut Vec<String>,
        ) -> Result<(), String> {
            assert!(self.session_open);
            for request in requests {
                self.writes_started = true;
                phases.mark_writes_started();
                self.events
                    .push(format!("flash:{}:{}", request.label, request.lun));
                if self.fail_second_write && request.label.starts_with("vbmeta") {
                    return Err("second write failed".into());
                }
            }
            Ok(())
        }

        fn reboot(&mut self, _log: &mut Vec<String>) {
            assert!(self.session_open);
            self.events.push("reboot".into());
            self.session_open = false;
        }

        fn writes_started(&self) -> bool {
            self.writes_started
        }

        fn recover_before_write(&mut self, _log: &mut Vec<String>) {
            assert!(!self.writes_started);
            self.events.push("recover".into());
            self.session_open = false;
        }
    }

    fn table() -> GpuTable {
        GpuTable { groups: vec![] }
    }

    fn candidate() -> VendorBootDtbInfo {
        VendorBootDtbInfo {
            index: 2,
            model: Some("test".into()),
            chip: Some("waipio".into()),
            gpu_shape: None,
            table: Some(table()),
        }
    }

    fn test_paths(root: &Path) -> InspectionPaths {
        let work_dir = root.join("work");
        std::fs::create_dir_all(&work_dir).unwrap();
        InspectionPaths {
            work_dir,
            backup_root: Some(root.join("backups")),
            device_model: "test-model".into(),
        }
    }

    fn phases() -> PhaseReporter {
        PhaseReporter::from_labels(vec!["prepare".into(), "dump".into(), "inspect".into()])
    }

    fn flash_phases() -> PhaseReporter {
        PhaseReporter::from_labels(
            [
                "prepare", "dump", "inspect", "patch", "rebuild", "flash", "reboot",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        )
    }

    fn prepared(root: &Path) -> KonaBessPrepared {
        let work_dir = root.join("work");
        KonaBessPrepared {
            vendor_boot: work_dir.join("vendor_boot.img"),
            vbmeta: work_dir.join("vbmeta.img"),
            backup_dir: root.join("backup_konabess"),
            slot_suffix: "_b".into(),
            probable_dtb_index: Some(2),
            device_model: "test-model".into(),
            work_dir,
        }
    }

    #[test]
    fn accepts_key_map_keys_and_unsigned_but_rejects_present_unknown_keys() {
        assert!(validate_signing_key(Some("2597c218aae470a130f61162feaae70afd97f011")).is_ok());
        assert!(validate_signing_key(None).is_ok());
        assert!(validate_signing_key(Some("")).is_ok());
        assert!(validate_signing_key(Some("8fcb864f11f53ed11284615fb67685522085d3a2")).is_err());
        assert!(validate_signing_key(Some("deadbeef")).is_err());
    }

    #[test]
    fn canoe_empty_efisp_requires_provision_and_bypasses_avb_gate() {
        assert_eq!(exploit_gate_kind(None, true), ExploitGateKind::EfispGbl);
        assert_eq!(
            exploit_gate_kind(None, false),
            ExploitGateKind::SignedVbmeta
        );
        assert!(crate::efisp_is_empty(&[0; 32]));
        assert!(!crate::efisp_is_empty(&[0, 0, 1, 0]));
        assert!(validate_signing_key(Some("fixed-or-unknown")).is_err());
    }

    #[test]
    fn image_identity_falls_back_to_a_known_detected_model_only() {
        let baldur = "Lenovo/lineage_baldur/baldur:17/CP2A/eng:userdebug/test-keys";
        let unknown = "Lenovo/lineage_foo/foo:17/CP2A/eng:userdebug/test-keys";
        let tb323fu = ltbox_core::model::capabilities("TB323FU");

        // A codename identifies the image itself; no fallback is involved.
        let identity = image_identity(Some(baldur), "");
        assert_eq!(identity.capabilities, Some(tb323fu));
        assert_eq!(identity.unidentified_as, None);

        // No known model in the fingerprint: the detected TB323FU decides,
        // which keeps the efisp/GBL exploit route.
        let identity = image_identity(Some(unknown), "TB323FU");
        assert!(identity.capabilities.is_some_and(|p| p.root_uses_gbl));
        assert_eq!(identity.unidentified_as, Some("TB323FU"));

        // Without a known detected model there is no profile to stand in.
        for device in ["", "Legion Y700"] {
            let identity = image_identity(Some(unknown), device);
            assert_eq!(identity.capabilities, None, "{device:?}");
            assert_eq!(identity.unidentified_as, None, "{device:?}");
        }
        assert_eq!(image_identity(None, "TB323FU").capabilities, None);
    }

    #[test]
    fn inspected_image_overrides_stale_exploit_route() {
        let tb323fu = ltbox_core::model::capabilities_from_fingerprint(
            "qti/TB323FU/TB323FU:15/build:user/release-keys",
        );
        assert_eq!(exploit_gate_kind(tb323fu, false), ExploitGateKind::EfispGbl);
        let tb320fc = ltbox_core::model::capabilities_from_fingerprint(
            "qti/LAVIETab9QHD1/LAVIETab9QHD1:15/build:user/release-keys",
        );
        assert_eq!(
            exploit_gate_kind(tb320fc, true),
            ExploitGateKind::SignedVbmeta
        );
        let unknown = ltbox_core::model::capabilities_from_fingerprint("qti/unknown/build");
        assert_eq!(exploit_gate_kind(unknown, true), ExploitGateKind::EfispGbl);
    }

    #[test]
    fn resolves_slot_before_edl_and_hands_dump_to_classifier() {
        let root = tempfile::tempdir().unwrap();
        let paths = test_paths(root.path());
        let mut backend = FakeBackend {
            candidate: Some(candidate()),
            ..FakeBackend::default()
        };
        let (prepared, candidates) =
            execute_inspection(&mut backend, &paths, &phases(), &mut Vec::new()).unwrap();

        assert_eq!(candidates, vec![candidate()]);
        assert_eq!(prepared.probable_dtb_index, None);
        assert_eq!(
            backend.events,
            [
                "slot",
                "dtb-index",
                "edl",
                "dump:vendor_boot_b",
                "dump:vbmeta_b",
                "gate",
                "classify",
                "pause"
            ]
        );
        let backup_root = paths.backup_root.as_ref().unwrap();
        let backup_dir = std::fs::read_dir(backup_root.join("konabess"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .next()
            .unwrap();
        assert!(backup_dir.join("vendor_boot.img").is_file());
        assert!(backup_dir.join("vbmeta.img").is_file());
    }

    #[test]
    fn a_gbl_verified_device_flashes_vendor_boot_without_a_vbmeta_pair() {
        // TB323FU verifies through the GBL on `efisp`, so nothing rebuilds the
        // AVB chain — the vbmeta whose Lenovo key cannot be re-signed is never
        // produced, and never flashed.
        let root = tempfile::tempdir().unwrap();
        let mut backend = FakeFlashBackend::default();
        let result = execute_flash(
            &mut backend,
            &prepared(root.path()),
            KonaBessTableEdit {
                gbl_verified: true,
                target_index: 3,
                chip: "waipio",
                table: &table(),
            },
            &flash_phases(),
            &mut Vec::new(),
        );
        assert!(result.is_ok(), "{result:?}");
        assert!(
            backend
                .events
                .iter()
                .any(|e| e.starts_with("flash:vendor_boot")),
            "{:?}",
            backend.events
        );
        assert!(
            !backend.events.iter().any(|e| e.starts_with("flash:vbmeta")),
            "{:?}",
            backend.events
        );
    }

    #[test]
    fn rejected_preserved_or_fresh_abl_prevents_gbl_writes() {
        for fresh in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let mut backend = FakeFlashBackend {
                abl_error: Some(fresh),
                ..Default::default()
            };
            let error = run_flash(
                &mut backend,
                &prepared(root.path()),
                KonaBessTableEdit {
                    gbl_verified: true,
                    target_index: 3,
                    chip: "waipio",
                    table: &table(),
                },
                &flash_phases(),
                &mut Vec::new(),
            )
            .unwrap_err();
            assert_eq!(error, "ABL rejected");
            let expected = if fresh {
                vec!["abl:false", "build:3", "open", "abl:true", "recover"]
            } else {
                vec!["abl:false", "recover"]
            };
            assert_eq!(backend.events, expected);
            assert!(!backend.writes_started);
        }
    }

    #[test]
    fn gate_abort_never_creates_backup_or_classifies() {
        let root = tempfile::tempdir().unwrap();
        let paths = test_paths(root.path());
        let mut backend = FakeBackend {
            gate_error: Some("blocked".into()),
            ..FakeBackend::default()
        };

        let result = execute_inspection(&mut backend, &paths, &phases(), &mut Vec::new());

        assert_eq!(result.unwrap_err(), "blocked");
        assert!(!paths.backup_root.as_ref().unwrap().exists());
        assert!(!backend.events.iter().any(|event| event == "classify"));
    }

    #[test]
    fn repeated_inspection_runs_create_distinct_backups() {
        let root = tempfile::tempdir().unwrap();
        let paths = test_paths(root.path());

        let mut first_backend = FakeBackend {
            candidate: Some(candidate()),
            ..FakeBackend::default()
        };
        let (first, _) =
            execute_inspection(&mut first_backend, &paths, &phases(), &mut Vec::new()).unwrap();
        let first_vendor = std::fs::read(first.backup_dir.join("vendor_boot.img")).unwrap();

        let mut second_backend = FakeBackend::default();
        let (second, _) =
            execute_inspection(&mut second_backend, &paths, &phases(), &mut Vec::new()).unwrap();

        assert_ne!(first.backup_dir, second.backup_dir);
        assert_eq!(
            std::fs::read(first.backup_dir.join("vendor_boot.img")).unwrap(),
            first_vendor
        );
        assert!(second.backup_dir.join("vendor_boot.img").is_file());
    }

    #[test]
    fn failed_backup_copy_preserves_previous_run_and_partial_dump() {
        let root = tempfile::tempdir().unwrap();
        let vendor = root.path().join("vendor_boot.img");
        let vbmeta = root.path().join("vbmeta.img");
        std::fs::write(&vendor, b"original vendor").unwrap();
        std::fs::write(&vbmeta, b"original vbmeta").unwrap();
        let first =
            crate::backup::create_backup_dir_in(root.path(), "konabess", "TB322FC").unwrap();
        persist_backup(&vendor, &vbmeta, &first).unwrap();
        let second =
            crate::backup::create_backup_dir_in(root.path(), "konabess", "TB322FC").unwrap();
        std::fs::write(&vendor, b"modified vendor").unwrap();
        assert!(persist_backup(&vendor, &root.path().join("missing.img"), &second).is_err());
        assert_eq!(
            std::fs::read(first.join("vendor_boot.img")).unwrap(),
            b"original vendor"
        );
        assert_eq!(
            std::fs::read(first.join("vbmeta.img")).unwrap(),
            b"original vbmeta"
        );
        assert_eq!(
            std::fs::read(second.join("vendor_boot.img")).unwrap(),
            b"modified vendor"
        );
    }

    #[test]
    fn flash_writes_both_images_on_retained_slot_in_one_session() {
        let root = tempfile::tempdir().unwrap();
        let mut backend = FakeFlashBackend::default();

        run_flash(
            &mut backend,
            &prepared(root.path()),
            KonaBessTableEdit {
                gbl_verified: false,
                target_index: 9,
                chip: "waipio",
                table: &table(),
            },
            &flash_phases(),
            &mut Vec::new(),
        )
        .unwrap();

        assert_eq!(
            backend.events,
            [
                "build:9",
                "open",
                "flash:vendor_boot_b:4",
                "flash:vbmeta_b:4",
                "reboot"
            ]
        );
        assert_eq!(
            backend
                .events
                .iter()
                .filter(|event| *event == "open")
                .count(),
            1
        );
    }

    #[test]
    fn build_failure_never_opens_a_flash_session_or_writes() {
        let root = tempfile::tempdir().unwrap();
        let mut backend = FakeFlashBackend {
            build_error: Some("signing gate failed".into()),
            ..FakeFlashBackend::default()
        };

        let error = run_flash(
            &mut backend,
            &prepared(root.path()),
            KonaBessTableEdit {
                gbl_verified: false,
                target_index: 3,
                chip: "waipio",
                table: &table(),
            },
            &flash_phases(),
            &mut Vec::new(),
        )
        .unwrap_err();

        assert_eq!(error, "signing gate failed");
        assert_eq!(backend.events, ["build:3", "recover"]);
        assert!(!backend.writes_started);
    }

    #[test]
    fn second_write_failure_is_error_and_never_reboots() {
        let root = tempfile::tempdir().unwrap();
        let mut backend = FakeFlashBackend {
            fail_second_write: true,
            ..FakeFlashBackend::default()
        };

        let error = run_flash(
            &mut backend,
            &prepared(root.path()),
            KonaBessTableEdit {
                gbl_verified: false,
                target_index: 5,
                chip: "waipio",
                table: &table(),
            },
            &flash_phases(),
            &mut Vec::new(),
        )
        .unwrap_err();

        assert!(
            error.contains("second write failed")
                || error.contains("err_konabess_partial_write_recovery")
        );
        assert_eq!(
            backend.events,
            [
                "build:5",
                "open",
                "flash:vendor_boot_b:4",
                "flash:vbmeta_b:4"
            ]
        );
        assert!(backend.writes_started);
        assert!(backend.session_open);
        assert!(!backend.events.iter().any(|event| event == "reboot"));
    }

    #[test]
    fn inspection_carries_probable_dtb_index_from_pre_edl_probe() {
        let root = tempfile::tempdir().unwrap();
        let paths = test_paths(root.path());
        let mut backend = FakeBackend {
            candidate: Some(candidate()),
            probable_dtb_index: Some(2),
            ..FakeBackend::default()
        };

        let (prepared, _) =
            execute_inspection(&mut backend, &paths, &phases(), &mut Vec::new()).unwrap();

        assert_eq!(prepared.probable_dtb_index, Some(2));
        assert!(
            backend.events.iter().position(|event| event == "dtb-index")
                < backend.events.iter().position(|event| event == "edl")
        );
    }

    #[test]
    fn cancelling_target_selection_triggers_system_reboot() {
        #[derive(Default)]
        struct FakeCancelBackend {
            rebooted: bool,
        }

        impl KonaBessCancelBackend for FakeCancelBackend {
            fn reboot_to_system(&mut self, _log: &mut Vec<String>) {
                self.rebooted = true;
            }
        }

        let mut backend = FakeCancelBackend::default();
        execute_cancel(&mut backend, &mut Vec::new());

        assert!(backend.rebooted);
    }
}
