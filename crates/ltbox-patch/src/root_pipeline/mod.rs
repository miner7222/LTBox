//! End-to-end root pipeline: download → dump → patch → resign → flash.
//!
//! Orchestrates [`crate::magisk`], [`crate::ksu`], [`crate::avb`], and
//! `ltbox_device::edl`. Outputs land in `cfg.output_dir` (patched root image +
//! rebuilt vbmeta), then flash pushes them to the active slot.

use std::path::PathBuf;

// fs_err: io::Error Display includes the path, so bare `?` gives readable errors.
use fs_err as fs;

use ltbox_core::github::GitHubClient;
use ltbox_core::i18n::tr;
use ltbox_core::{LtboxError, Result, tr_args};

use crate::{avb, gki, key_map};

pub use crate::avb::VbmetaOwner;

pub mod apatch;
pub mod apk;
pub mod ksu;
pub mod magisk;
pub mod skroot;

// Flat public API, e.g. `ltbox_patch::root_pipeline::stage_root_manager_apk`.
pub use apatch::{download_apatch_payload, download_apatch_payload_nightly};
pub use ksu::{
    download_ksu_payload, download_ksu_payload_nightly, ksu_gki_branch,
    normalize_ksu_kernel_version, stage_root_manager_apk,
};
pub use magisk::{download_latest_magisk_apk, download_magisk_apk_nightly};

/// Pick the avbtool-rs key_spec for re-signing.
/// Missing pubkey means unsigned; unknown signed pubkeys abort before writes.
fn resolve_signing_key(
    pubkey_sha1: Option<&str>,
    image_name: &str,
    log: &mut Vec<String>,
) -> Result<Option<String>> {
    match key_map::key_spec_for_signed_pubkey(pubkey_sha1) {
        Ok(Some(spec)) => {
            let sha = pubkey_sha1.unwrap_or("").trim();
            ltbox_core::live!(
                log,
                "[AVB] {}",
                tr_args!(
                    "log_avb_signing_key",
                    image = image_name,
                    sha1 = sha,
                    key = spec
                )
            );
            Ok(Some(spec.to_string()))
        }
        Ok(None) => {
            ltbox_core::live!(
                log,
                "[AVB] {}",
                tr_args!("log_avb_unsigned_skip_key", image = image_name)
            );
            Ok(None)
        }
        Err(sha) => Err(LtboxError::Avb(key_map::unresolved_signing_key_error(
            image_name, &sha,
        ))),
    }
}

/// Provider families carried through the GUI wizard state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootFamily {
    /// Magisk / forks — root ramdisk injection.
    Magisk,
    /// KernelSU-style LKM — root ramdisk with ksuinit + kernelsu.ko.
    KernelSU,
    /// APatch — boot image via kptools + kpimg.
    APatch,
    /// SKRoot Lite — direct kernel binary patch inside boot.img.
    Skroot,
}

/// Provider inside the family to fetch from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootProvider {
    Magisk,
    MagiskFork,
    KernelSULocal,
    KernelSU,
    KernelSUNext,
    SukiSU,
    BakaSU,
    APatch,
    FolkPatch,
    Skroot,
}

/// Release channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootVersion {
    Stable,
    Nightly,
}

/// Root image selected for the entire dump → patch → AVB → flash pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootImageTarget {
    Boot,
    InitBoot,
}

impl RootImageTarget {
    /// GPT partition label without an A/B slot suffix.
    pub const fn partition_base(self) -> &'static str {
        match self {
            Self::Boot => "boot",
            Self::InitBoot => "init_boot",
        }
    }

    /// Stock and output image filename used by the patch workspace.
    pub const fn filename(self) -> &'static str {
        match self {
            Self::Boot => "boot.img",
            Self::InitBoot => "init_boot.img",
        }
    }
}

/// Resolve the root target once from the route and connected device model.
pub fn resolve_root_image_target(
    family: RootFamily,
    gki_mode: bool,
    device_model: &str,
) -> RootImageTarget {
    if gki_mode
        || matches!(family, RootFamily::APatch | RootFamily::Skroot)
        || ltbox_core::model::capabilities(device_model).ramdisk_root_uses_boot
    {
        RootImageTarget::Boot
    } else {
        RootImageTarget::InitBoot
    }
}

/// Whether the root target is hash-verified by a parent that must be rebuilt.
///
/// AVB binds a partition to vbmeta in one of two ways. With a **Hash
/// descriptor** in vbmeta, the patched image's digest must be imported back and
/// vbmeta re-signed, or the bootloader rejects it against the stock digest. With
/// a **chain** descriptor, vbmeta pins only the signing key, which re-signing
/// with the same `KEY_MAP` key preserves, so vbmeta is left alone.
///
/// `boot` is chained except on models with `boot_vbmeta_is_hash` (currently
/// TB320FC), whose stock `boot.img` footer is `NONE`-signed because vbmeta
/// carries the digest. `init_boot` is always hashed by a parent; descriptor
/// inspection decides whether vbmeta or vbmeta_system owns it. The model is
/// already known over ADB or Fastboot, so this resolves before EDL and the dump
/// can skip vbmeta when it plays no part.
pub fn root_run_rebuilds_vbmeta(target: RootImageTarget, device_model: &str) -> bool {
    match target {
        RootImageTarget::InitBoot => true,
        RootImageTarget::Boot => ltbox_core::model::capabilities(device_model).boot_vbmeta_is_hash,
    }
}

/// Root pipeline input from the GUI wizard.
#[derive(Clone)]
pub struct RootPipelineConfig {
    pub local_ksu: Option<LocalKsuFiles>,
    pub family: RootFamily,
    pub provider: RootProvider,
    pub version: RootVersion,
    /// Resolved once by the caller; all image and partition routing uses this.
    pub root_image_target: RootImageTarget,
    /// Resolved once by the caller via [`root_run_rebuilds_vbmeta`]. `false`
    /// means vbmeta chains the target, so the caller neither dumps nor flashes
    /// it and this pipeline leaves it out of the artifacts.
    pub rebuild_vbmeta: bool,
    /// Descriptor-detected hash owner. Required when rebuilding outside GBL.
    pub vbmeta_owner: Option<VbmetaOwner>,

    /// APK extraction + root image patching workspace. Cleaned on entry.
    pub work_dir: PathBuf,
    /// Where the patched root image + vbmeta land.
    pub output_dir: PathBuf,
    /// EDL loader path (`xbl_s_devprg_ns.melf`).
    pub loader: PathBuf,
    /// Active slot (`_a` / `_b`). Empty is rejected; callers must resolve it.
    pub slot_suffix: String,
    /// Magisk `PREINITDEVICE`. Empty → Magisk resolves at runtime.
    pub preinit_device: String,
    /// GKI-mode only: user-supplied AnyKernel3 zip.
    pub gki_kernel_zip: Option<PathBuf>,
    /// Device kernel version (`major.minor.patch` from `uname -r`) —
    /// used by KSU to pick the matching `.ko` release asset.
    pub kernel_version: Option<String>,
    /// GKI branch from the device's kernel release (`android12`), when ADB
    /// supplied one; `None` matches on kernel version alone, see [`ksu_gki_branch`].
    pub kernel_gki_branch: Option<String>,
    /// GKI mode → patch `boot.img` via `gki::patch_boot` instead of the
    /// Magisk/KSU ramdisk path.
    pub gki_mode: bool,
    /// APatch / FolkPatch: `.kpm` modules to embed.
    pub kpm_paths: Vec<PathBuf>,
    /// APatch: empty for default signature/UID auth, or an optional user key
    /// (8..=63 ASCII alphanumeric). Ignored for FolkPatch.
    pub superkey: String,
    /// Magisk Forks: user-picked variant APK (forks are local-APK-only).
    pub magisk_forks_apk: Option<PathBuf>,
    /// Nightly: manual workflow run ID. `None` → auto-detect latest.
    pub nightly_run_id: Option<u64>,
    /// Stable-channel release tag selected in the GUI; `None` means latest.
    pub release_tag: Option<String>,
}

/// Locally supplied KernelSU-family inputs. They must come from the same build
/// and the module must match the device kernel; no provider download is used.
#[derive(Debug, Clone)]
pub struct LocalKsuFiles {
    pub manager_apk: PathBuf,
    pub ksuinit: PathBuf,
    pub module: PathBuf,
}

impl LocalKsuFiles {
    /// Reject incomplete files and wrong ELF architecture/type before staging.
    pub fn validate(&self) -> Result<()> {
        use std::io::Read;
        let mut apk = zip::ZipArchive::new(fs::File::open(&self.manager_apk)?)
            .map_err(|e| LtboxError::Patch(format!("Manager APK: {e}")))?;
        apk.by_name("AndroidManifest.xml")
            .map_err(|e| LtboxError::Patch(format!("Manager APK manifest: {e}")))?;
        for (path, module) in [(&self.ksuinit, false), (&self.module, true)] {
            let mut file = fs::File::open(path)?;
            let mut header = [0u8; 64];
            file.read_exact(&mut header)?;
            let kind = u16::from_le_bytes([header[16], header[17]]);
            if &header[..4] != b"\x7fELF"
                || header[4] != 2
                || header[5] != 1
                || u16::from_le_bytes([header[18], header[19]]) != 183
                || (module && kind != 1)
                || (!module && !matches!(kind, 2 | 3))
            {
                return Err(LtboxError::Patch(format!(
                    "Expected an arm64 {}: {}",
                    if module {
                        "ELF module"
                    } else {
                        "ELF executable"
                    },
                    path.display()
                )));
            }
        }
        Ok(())
    }
}

/// Per-provider `(workflow_file, default_branch)` for nightly runs.
/// Returns `None` for providers without a nightly channel (e.g. MagiskFork).
pub fn provider_workflow(provider: RootProvider) -> Option<(&'static str, &'static str)> {
    Some(match provider {
        RootProvider::Magisk => ("build.yml", "master"),
        RootProvider::MagiskFork | RootProvider::KernelSULocal => return None,
        RootProvider::KernelSU => ("build-manager.yml", "main"),
        RootProvider::KernelSUNext => ("build-manager-ci.yml", "dev"),
        RootProvider::SukiSU => ("build-manager.yml", "main"),
        RootProvider::BakaSU => ("build-manager.yml", "main"),
        RootProvider::APatch => ("build.yml", "main"),
        RootProvider::FolkPatch => ("build.yml", "main"),
        RootProvider::Skroot => return None,
    })
}

/// Workflows that build a KernelSU-family provider's **release tags**, in
/// preference order; tagged payloads never come from a nightly. `release.yml`
/// is the tag pipeline; the build workflow is a fallback for tags it also
/// builds (BakaSU does, the KernelSU-Next `-ci` workflow does not).
pub fn provider_release_workflows(provider: RootProvider) -> &'static [&'static str] {
    match provider {
        RootProvider::KernelSU | RootProvider::SukiSU | RootProvider::BakaSU => {
            &["release.yml", "build-manager.yml"]
        }
        RootProvider::KernelSUNext => &["release.yml", "build-manager-ci.yml"],
        _ => &[],
    }
}

/// Require manager artifacts for APatch-family nightly build choices.
pub fn provider_has_nightly_manager(
    provider: RootProvider,
    artifacts: &[ltbox_core::github::WorkflowArtifact],
) -> bool {
    if matches!(provider, RootProvider::APatch | RootProvider::FolkPatch) {
        let names: Vec<_> = artifacts
            .iter()
            .map(|artifact| artifact.name.clone())
            .collect();
        apatch::select_apatch_nightly_artifact(provider, &names).is_some()
    } else {
        !artifacts.is_empty()
    }
}

/// Resolve `(repo, run_id)` for a nightly fetch. Manual IDs are validated
/// against the provider's workflow so bad IDs fail fast, not at nightly.link.
pub(super) fn resolve_nightly_run(
    provider: RootProvider,
    manual_run_id: Option<u64>,
    log: &mut Vec<String>,
) -> Result<(&'static str, u64)> {
    let repo = provider_repo(provider).ok_or_else(|| {
        LtboxError::Patch(format!(
            "resolve_nightly_run: unsupported provider {provider:?}"
        ))
    })?;
    let (workflow_file, branch) = provider_workflow(provider).ok_or_else(|| {
        LtboxError::Patch(format!(
            "resolve_nightly_run: no workflow metadata for {provider:?}"
        ))
    })?;
    let client = GitHubClient::new(repo)?;

    let run_id = match manual_run_id {
        Some(id) => {
            ltbox_core::live!(
                log,
                "[Nightly] {}",
                tr_args!(
                    "log_nightly_validating_manual",
                    repo = repo,
                    id = id,
                    workflow = workflow_file,
                    branch = branch,
                )
            );
            if !client.workflow_run_matches(id, workflow_file, Some(branch))? {
                return Err(LtboxError::Patch(format!(
                    "Manual run id {id} does not match workflow {workflow_file} on branch {branch} of {repo}"
                )));
            }
            id
        }
        None => {
            ltbox_core::live!(
                log,
                "[Nightly] {}",
                tr_args!(
                    "log_nightly_auto_detect",
                    repo = repo,
                    workflow = workflow_file,
                    branch = branch,
                )
            );
            client
                .latest_successful_run(workflow_file, Some(branch))?
                .ok_or_else(|| {
                    LtboxError::Patch(format!(
                        "No successful {workflow_file} run found on {repo}:{branch}"
                    ))
                })?
        }
    };
    ltbox_core::live!(
        log,
        "[Nightly] {}",
        tr_args!("log_nightly_using_run_id", repo = repo, id = run_id)
    );
    Ok((repo, run_id))
}

/// Resolve and cache one nightly run ID so all artifacts match.
pub fn ensure_nightly_run_id(cfg: &mut RootPipelineConfig, log: &mut Vec<String>) -> Result<()> {
    if !matches!(cfg.version, RootVersion::Nightly) {
        return Ok(());
    }
    if cfg.nightly_run_id.is_some() {
        return Ok(());
    }
    if matches!(
        cfg.provider,
        RootProvider::MagiskFork | RootProvider::KernelSULocal
    ) {
        return Ok(());
    }
    let (_repo, run_id) = resolve_nightly_run(cfg.provider, None, log)?;
    cfg.nightly_run_id = Some(run_id);
    Ok(())
}

/// Build the `nightly.link` public-mirror URL for a workflow artifact ID. The
/// response is always ZIP-wrapped.
///
/// The run + name form (`/actions/runs/{run}/{name}.zip`) 404s for existing
/// artifacts, so downloads go through the artifact ID.
pub(super) fn nightly_artifact_url(repo: &str, artifact_id: u64) -> String {
    format!("https://nightly.link/{repo}/actions/artifacts/{artifact_id}.zip")
}

/// ID of the artifact called `name` among `run_id`'s `artifacts`. A name that
/// cannot be resolved is a hard error: there is no run + name URL fallback.
pub(super) fn nightly_artifact_id(
    artifacts: &[ltbox_core::github::WorkflowArtifact],
    repo: &str,
    run_id: u64,
    name: &str,
) -> Result<u64> {
    artifacts
        .iter()
        .find(|artifact| artifact.name == name)
        .map(|artifact| artifact.id)
        .ok_or_else(|| {
            LtboxError::Download(format!(
                "{repo} run {run_id}: cannot resolve an artifact ID for `{name}`"
            ))
        })
}

/// Resolve the GitHub repo slug for a given provider.
pub fn provider_repo(provider: RootProvider) -> Option<&'static str> {
    Some(match provider {
        RootProvider::Magisk => "topjohnwu/Magisk",
        RootProvider::MagiskFork | RootProvider::KernelSULocal => return None,
        RootProvider::KernelSU => "tiann/KernelSU",
        // The old `rifsxd/KernelSU-Next` slug redirects, but its release assets
        // aren't mirrored there.
        RootProvider::KernelSUNext => "KernelSU-Next/KernelSU-Next",
        RootProvider::SukiSU => "SukiSU-Ultra/SukiSU-Ultra",
        // The old `ReSukiSU/ReSukiSU` slug redirects on GitHub but not on
        // nightly.link, so artifact downloads need this one.
        RootProvider::BakaSU => "Baka-SU/BakaSU",
        RootProvider::APatch => "bmax121/APatch",
        RootProvider::FolkPatch => "LyraVoid/FolkPatch",
        RootProvider::Skroot => "abcz316/SKRoot-linuxKernelRoot",
    })
}

/// Pre-fetch root payloads before EDL; `build_patched_artifacts` runs offline.
pub fn stage_root_payload(cfg: &RootPipelineConfig, log: &mut Vec<String>) -> Result<()> {
    fs::create_dir_all(&cfg.work_dir)?;
    if cfg.gki_mode {
        return Ok(());
    }
    match cfg.family {
        RootFamily::Magisk => {
            // Skip if already extracted from a prior call.
            if cfg.work_dir.join("magiskinit").exists() {
                return Ok(());
            }
            let apk_path = cfg.work_dir.join("magisk.apk");
            let manager_apk = cfg.work_dir.join("manager.apk");
            // Reuse the APK from stage_root_manager_apk to avoid a second download.
            if !apk_path.exists() {
                if matches!(cfg.provider, RootProvider::MagiskFork) {
                    let src = cfg.magisk_forks_apk.as_ref().ok_or_else(|| {
                        LtboxError::Patch("Magisk forks require a local APK — none supplied".into())
                    })?;
                    if !src.exists() {
                        return Err(LtboxError::Patch(format!(
                            "Magisk forks APK does not exist: {}",
                            src.display()
                        )));
                    }
                    fs::copy(src, &apk_path)
                        .map_err(|e| LtboxError::Patch(format!("stage forks APK: {e}")))?;
                } else if manager_apk.exists() {
                    fs::copy(&manager_apk, &apk_path).map_err(|e| {
                        LtboxError::Patch(format!("magisk.apk copy from manager.apk: {e}"))
                    })?;
                } else {
                    match cfg.version {
                        RootVersion::Stable => {
                            magisk::download_magisk_release_apk(
                                cfg.provider,
                                cfg.release_tag.as_deref(),
                                &apk_path,
                                log,
                            )?;
                        }
                        RootVersion::Nightly => {
                            download_magisk_apk_nightly(
                                cfg.provider,
                                cfg.nightly_run_id,
                                &cfg.work_dir,
                                &apk_path,
                                log,
                            )?;
                        }
                    }
                }
            }
            ltbox_core::live!(log, "[Magisk] {}", tr("log_magisk_extracting_payload"));
            crate::magisk::extract_apk_payload(&apk_path, &cfg.work_dir)?;
        }
        RootFamily::KernelSU => {
            if cfg.provider == RootProvider::KernelSULocal {
                let staged = LocalKsuFiles {
                    manager_apk: cfg.work_dir.join("manager.apk"),
                    ksuinit: cfg.work_dir.join("init"),
                    module: cfg.work_dir.join("kernelsu.ko"),
                };
                if staged.ksuinit.is_file() && staged.module.is_file() {
                    return staged.validate();
                }
                let local = cfg
                    .local_ksu
                    .as_ref()
                    .ok_or_else(|| LtboxError::Patch("Missing local KernelSU files".into()))?;
                local.validate()?;
                fs::copy(&local.ksuinit, cfg.work_dir.join("init"))?;
                fs::copy(&local.module, cfg.work_dir.join("kernelsu.ko"))?;
                return Ok(());
            }
            // Skip if both files already on disk from a prior call.
            let ko = cfg.work_dir.join("kernelsu.ko");
            let init = cfg.work_dir.join("init");
            if ko.exists() && init.exists() {
                return Ok(());
            }
            match cfg.version {
                RootVersion::Stable => {
                    ltbox_core::live!(log, "[KSU] {}", tr("log_ksu_fetching_stable"));
                    ksu::download_ksu_release_payload(
                        cfg.provider,
                        cfg.release_tag.as_deref(),
                        cfg.kernel_version.as_deref(),
                        cfg.kernel_gki_branch.as_deref(),
                        &cfg.work_dir,
                        log,
                    )?;
                }
                RootVersion::Nightly => {
                    ltbox_core::live!(
                        log,
                        "[KSU] {}",
                        tr_args!(
                            "log_ksu_fetching_nightly",
                            run_id = cfg
                                .nightly_run_id
                                .map_or_else(|| tr("log_value_auto"), |id| id.to_string()),
                        )
                    );
                    download_ksu_payload_nightly(
                        cfg.provider,
                        cfg.kernel_version.as_deref(),
                        cfg.kernel_gki_branch.as_deref(),
                        cfg.nightly_run_id,
                        &cfg.work_dir,
                        log,
                    )?;
                }
            }
        }
        RootFamily::APatch => {
            // stage_root_manager_apk already fetched the APK and kpimg
            // (via download_apatch_payload).
        }
        RootFamily::Skroot => {
            // SKRoot Lite patches the dumped kernel directly; only the manager
            // APK (stage_root_manager_apk) is needed.
        }
    }
    Ok(())
}

/// Offline pipeline outcome — everything before the EDL flash step.
pub struct PatchedArtifacts {
    pub patched_root_image: PathBuf,
    /// `None` when vbmeta is left untouched (GBL root, or vbmeta chains the target).
    pub patched_vbmeta: Option<PathBuf>,
    pub manager_apk: Option<PathBuf>,
    /// Target partition name (`init_boot_a`, `boot_a`, …).
    pub root_partition: String,
    pub vbmeta_partition: Option<String>,
    pub vbmeta_owner: Option<VbmetaOwner>,
    /// SKRoot's generated key. It intentionally never enters the operation log.
    pub skroot_root_key: Option<String>,
}

/// Build patched artifacts: fetch payload, patch, resign, rebuild vbmeta,
/// move finals into `output_dir`. Caller must have already dumped stock
/// images into `cfg.work_dir` (GUI reuses the EDL session for flash).
pub fn build_patched_artifacts(
    cfg: &RootPipelineConfig,
    uses_gbl: bool,
    log: &mut Vec<String>,
) -> Result<PatchedArtifacts> {
    fs::create_dir_all(&cfg.work_dir)?;
    fs::create_dir_all(&cfg.output_dir)?;

    if uses_gbl {
        use crate::efisp_load::{EfispLoad, detect};
        let abl = fs::read(cfg.work_dir.join("abl.img"))
            .map_err(|_| LtboxError::Patch(tr("err_abl_efisp_undetermined")))?;
        match detect(&abl) {
            EfispLoad::Yes => {}
            EfispLoad::No => return Err(LtboxError::Patch(tr("err_abl_efisp_not_loaded"))),
            EfispLoad::Undetermined => {
                return Err(LtboxError::Patch(tr("err_abl_efisp_undetermined")));
            }
        }
    }

    let stock_filename = cfg.root_image_target.filename();
    let stock_root_image_src = cfg.work_dir.join(stock_filename);
    let rebuild_vbmeta = !uses_gbl && cfg.rebuild_vbmeta;
    let owner = if rebuild_vbmeta {
        Some(
            cfg.vbmeta_owner
                .ok_or_else(|| LtboxError::Avb("Root hash owner has not been detected".into()))?,
        )
    } else {
        None
    };
    let owner_name = owner.map_or("vbmeta.img", VbmetaOwner::filename);
    let vbmeta_src = cfg.work_dir.join(owner_name);
    if !stock_root_image_src.exists() {
        return Err(LtboxError::Patch(format!(
            "work_dir is missing the stock {stock_filename} dump"
        )));
    }
    // vbmeta is dumped only when it takes part: GBL root leaves vbmeta alone and
    // a chained target is verified by its own footer.
    if rebuild_vbmeta && !vbmeta_src.exists() {
        return Err(LtboxError::Patch(format!(
            "work_dir is missing the stock {owner_name} dump"
        )));
    }
    // The GUI prefetches these before EDL; fetch here too for headless callers.
    // Both helpers skip already-staged files.
    let staged_manager_apk = cfg.work_dir.join("manager.apk");
    if !cfg.gki_mode && !staged_manager_apk.exists() {
        stage_root_manager_apk(cfg, log)?;
    }
    if !cfg.gki_mode {
        stage_root_payload(cfg, log)?;
    }

    let (patched_root_image, skroot_root_key) = if cfg.gki_mode {
        // GKI: swap kernel blob from user's AnyKernel3 zip — no GitHub fetch.
        let kernel_zip = cfg.gki_kernel_zip.as_ref().ok_or_else(|| {
            LtboxError::Patch("GKI mode requires a custom kernel zip — none supplied".into())
        })?;
        ltbox_core::live!(
            log,
            "[GKI] {}",
            tr_args!("log_gki_kernel_zip", path = kernel_zip.display())
        );
        (gki::patch_boot(&cfg.work_dir, kernel_zip, log)?, None)
    } else {
        match cfg.family {
            RootFamily::Magisk => {
                ltbox_core::live!(
                    log,
                    "[Magisk] {}",
                    tr_args!("log_magisk_patching_image", image = stock_filename)
                );
                (
                    crate::magisk::patch_root_image(
                        &cfg.work_dir,
                        cfg.root_image_target,
                        &cfg.preinit_device,
                        log,
                    )?,
                    None,
                )
            }
            RootFamily::KernelSU => {
                ltbox_core::live!(
                    log,
                    "[KSU] {}",
                    tr_args!("log_ksu_patching_image", image = stock_filename)
                );
                (
                    crate::ksu::patch_root_image(&cfg.work_dir, cfg.root_image_target, log)?,
                    None,
                )
            }
            RootFamily::APatch => {
                let folkpatch = cfg.provider == RootProvider::FolkPatch;
                ltbox_core::live!(
                    log,
                    "[APatch] {}",
                    tr_args!(
                        "log_apatch_patching_boot",
                        kpm_count = cfg.kpm_paths.len(),
                        superkey_len = if folkpatch {
                            crate::apatch::FOLKPATCH_SUPERKEY.len()
                        } else {
                            cfg.superkey.len()
                        },
                    )
                );
                (
                    if folkpatch {
                        crate::apatch::patch_folkpatch_boot(&cfg.work_dir, &cfg.kpm_paths, log)?
                    } else {
                        crate::apatch::patch_boot(
                            &cfg.work_dir,
                            &cfg.kpm_paths,
                            &cfg.superkey,
                            log,
                        )?
                    },
                    None,
                )
            }
            RootFamily::Skroot => {
                let patched = skroot::patch_boot(&cfg.work_dir, log)?;
                (patched.image, Some(patched.root_key))
            }
        }
    };

    let final_root_image = cfg.output_dir.join(stock_filename);
    if final_root_image.exists() {
        fs::remove_file(&final_root_image).ok();
    }
    fs::rename(&patched_root_image, &final_root_image)?;
    ltbox_core::live!(
        log,
        "[Root] {}",
        ltbox_core::tr_args!(
            "log_root_patched",
            image = stock_filename,
            path = final_root_image.display()
        )
    );

    // The caller must resolve the active slot (`controller::poll_active_slot`).
    // Guessing `_a` would patch the wrong slot on a device running `_b` and
    // report success while the active slot stays stock.
    if cfg.slot_suffix.is_empty() {
        return Err(LtboxError::Patch(
            "slot_suffix is empty; caller must resolve the active slot via \
             controller::poll_active_slot before invoking the root pipeline"
                .to_string(),
        ));
    }
    let suffix = cfg.slot_suffix.clone();

    // GBL with a signed stock image keeps its metadata, including the key pinned
    // by vbmeta; otherwise a new hash footer is added.
    let stock_info = avb::extract_image_avb_info(&stock_root_image_src)?;
    if stock_info.partition_name.as_deref() != Some(cfg.root_image_target.partition_base()) {
        return Err(LtboxError::Avb(format!(
            "stock {} AVB descriptor targets {:?}, expected {}",
            stock_filename,
            stock_info.partition_name,
            cfg.root_image_target.partition_base(),
        )));
    }
    let preserve_stock = uses_gbl
        && stock_info
            .public_key_sha1
            .as_deref()
            .is_some_and(|key| !key.trim().is_empty());
    let root_image_key = if preserve_stock {
        avb::preserve_stock_vbmeta(&stock_root_image_src, &final_root_image)?;
        ltbox_core::live!(
            log,
            "[AVB] {}",
            tr_args!("log_avb_preserved_stock_vbmeta", image = stock_filename)
        );
        None
    } else {
        let key = resolve_signing_key(stock_info.public_key_sha1.as_deref(), stock_filename, log)?;
        avb::add_hash_footer(
            &final_root_image,
            &stock_info,
            key.as_deref(),
            Some(stock_info.rollback_index),
        )?;
        ltbox_core::live!(
            log,
            "[AVB] {}",
            tr_args!(
                "log_avb_hash_footer_added",
                image = stock_filename,
                algorithm = stock_info.algorithm,
                index = stock_info.rollback_index,
                key = key.as_deref().unwrap_or(&tr("log_value_unsigned"))
            )
        );
        key
    };

    let (patched_vbmeta, vbmeta_partition) = if uses_gbl {
        // GBL needs a consistently signed root image; its vbmeta partition stays
        // untouched.
        (None, None)
    } else {
        // A chained target carries its own signature; that signature is the
        // only thing vbmeta checks. An unsigned stock footer therefore means
        // the chain assumption is wrong for this device, and re-signing would
        // produce an image nothing verifies — abort before any write.
        if !rebuild_vbmeta && root_image_key.is_none() {
            return Err(LtboxError::Avb(format!(
                "stock {stock_filename} is unsigned, so vbmeta cannot be chaining {}",
                cfg.root_image_target.partition_base(),
            )));
        }

        // A chained target (every model without `boot_vbmeta_is_hash`) is pinned
        // by key, not digest, so same-key re-signing keeps it valid. There is
        // nothing to rebuild and no vbmeta dump.
        if !rebuild_vbmeta {
            ltbox_core::live!(
                log,
                "[AVB] {}",
                tr_args!(
                    "log_avb_vbmeta_chained_untouched",
                    partition = cfg.root_image_target.partition_base()
                )
            );
            (None, None)
        } else {
            // Refresh vbmeta from the descriptor in final_root_image. Its pubkey
            // may differ from the root image's, so check it against KEY_MAP.
            let stock_vbmeta_info = avb::extract_image_avb_info(&vbmeta_src)?;
            let vbmeta_key = resolve_signing_key(
                stock_vbmeta_info.public_key_sha1.as_deref(),
                owner_name,
                log,
            )?;
            let final_vbmeta = cfg.output_dir.join(owner_name);
            match vbmeta_key.as_deref() {
                Some(key) => {
                    avb::rebuild_root_vbmeta_owner(
                        &final_vbmeta,
                        &vbmeta_src,
                        &final_root_image,
                        cfg.root_image_target.partition_base(),
                        key,
                    )?;
                    ltbox_core::live!(
                        log,
                        "[AVB] {}",
                        tr_args!(
                            "log_avb_rebuilt_vbmeta_from_partition_image",
                            image = owner_name,
                            root_image = stock_filename,
                            path = final_vbmeta.display(),
                            key = key
                        ),
                    );
                }
                None => {
                    // Unsigned vbmeta is copied as is; a stale Hash/Hashtree
                    // descriptor is fine because NONE-algorithm vbmeta is not verified.
                    fs::copy(&vbmeta_src, &final_vbmeta)?;
                    ltbox_core::live!(
                        log,
                        "[AVB] {}",
                        tr_args!(
                            "log_avb_vbmeta_unsigned_copied",
                            image = owner_name,
                            path = final_vbmeta.display()
                        ),
                    );
                }
            }
            (
                Some(final_vbmeta),
                Some(format!(
                    "{}{suffix}",
                    owner
                        .ok_or_else(|| LtboxError::Avb("Root hash owner missing".into()))?
                        .partition_base()
                )),
            )
        }
    };

    Ok(PatchedArtifacts {
        patched_root_image: final_root_image,
        patched_vbmeta,
        manager_apk: staged_manager_apk.exists().then_some(staged_manager_apk),
        root_partition: format!("{}{suffix}", cfg.root_image_target.partition_base()),
        vbmeta_partition,
        vbmeta_owner: owner,
        skroot_root_key,
    })
}

#[cfg(test)]
mod root_target_tests {
    use super::*;
    fn input_config(work: &std::path::Path) -> RootPipelineConfig {
        RootPipelineConfig {
            local_ksu: None,
            family: RootFamily::KernelSU,
            provider: RootProvider::KernelSULocal,
            version: RootVersion::Stable,
            root_image_target: RootImageTarget::Boot,
            rebuild_vbmeta: false,
            vbmeta_owner: None,
            work_dir: work.into(),
            output_dir: work.join("out"),
            loader: PathBuf::new(),
            slot_suffix: "_a".into(),
            preinit_device: String::new(),
            gki_kernel_zip: None,
            kernel_version: None,
            kernel_gki_branch: None,
            gki_mode: false,
            kpm_paths: Vec::new(),
            superkey: String::new(),
            magisk_forks_apk: None,
            nightly_run_id: None,
            release_tag: None,
        }
    }

    #[test]
    fn nightly_url_uses_the_artifact_id() {
        assert_eq!(
            nightly_artifact_url("LyraVoid/FolkPatch", 11302121949),
            "https://nightly.link/LyraVoid/FolkPatch/actions/artifacts/11302121949.zip"
        );
        let artifact = ltbox_core::github::WorkflowArtifact {
            id: 7,
            name: "manager".into(),
            digest: None,
            expired: false,
            created_at: String::new(),
            expires_at: String::new(),
        };
        assert_eq!(
            nightly_artifact_id(std::slice::from_ref(&artifact), "o/r", 9, "manager").unwrap(),
            7
        );
        let err = nightly_artifact_id(&[artifact], "o/r", 9, "other").unwrap_err();
        assert!(err.to_string().contains("run 9") && err.to_string().contains("other"));
    }

    #[test]
    fn local_ksu_stages_all_three_inputs_without_provider_or_kernel_lookup() {
        use std::io::Write;
        let temp = tempfile::tempdir().unwrap();
        let apk = temp.path().join("local.apk");
        let mut archive = zip::ZipWriter::new(fs::File::create(&apk).unwrap());
        archive
            .start_file(
                "AndroidManifest.xml",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        archive.write_all(b"manifest").unwrap();
        archive.finish().unwrap();
        let elf = |kind: u16| {
            let mut bytes = vec![0; 64];
            bytes[..4].copy_from_slice(b"\x7fELF");
            bytes[4] = 2;
            bytes[5] = 1;
            bytes[16..18].copy_from_slice(&kind.to_le_bytes());
            bytes[18] = 183;
            bytes
        };
        let init = temp.path().join("ksuinit");
        let module = temp.path().join("kernelsu.ko");
        fs::write(&init, elf(3)).unwrap();
        fs::write(&module, elf(1)).unwrap();
        let mut cfg = input_config(&temp.path().join("stage"));
        cfg.local_ksu = Some(LocalKsuFiles {
            manager_apk: apk,
            ksuinit: init,
            module: module.clone(),
        });
        stage_root_manager_apk(&cfg, &mut Vec::new()).unwrap();
        stage_root_payload(&cfg, &mut Vec::new()).unwrap();
        assert_eq!(fs::read(cfg.work_dir.join("init")).unwrap(), elf(3));
        assert_eq!(fs::read(cfg.work_dir.join("kernelsu.ko")).unwrap(), elf(1));
        fs::write(&module, elf(3)).unwrap();
        assert!(cfg.local_ksu.as_ref().unwrap().validate().is_err());
        fs::remove_file(&module).unwrap();
        // A second offline patch pass uses the validated staged snapshot.
        stage_root_payload(&cfg, &mut Vec::new()).unwrap();
        cfg.work_dir = temp.path().join("fresh");
        assert!(stage_root_payload(&cfg, &mut Vec::new()).is_err());
    }

    #[test]
    #[ignore = "weekly live download contract; no devices or downloaded code executed"]
    fn weekly_fetch_root_providers() {
        let filter = std::env::var("LTBOX_CHECK_PROVIDER").unwrap_or_default();
        let providers = [
            (RootProvider::Magisk, RootFamily::Magisk),
            (RootProvider::KernelSU, RootFamily::KernelSU),
            (RootProvider::KernelSUNext, RootFamily::KernelSU),
            (RootProvider::SukiSU, RootFamily::KernelSU),
            (RootProvider::BakaSU, RootFamily::KernelSU),
            (RootProvider::APatch, RootFamily::APatch),
            (RootProvider::FolkPatch, RootFamily::APatch),
            (RootProvider::Skroot, RootFamily::Skroot),
        ];
        let mut failures = Vec::new();
        let mut checked = 0;
        for (provider, family) in providers {
            if !filter.is_empty() && filter != format!("{provider:?}") {
                continue;
            }
            for version in [RootVersion::Stable, RootVersion::Nightly] {
                if provider == RootProvider::Skroot && version == RootVersion::Nightly {
                    continue;
                }
                checked += 1;
                let label = format!("{provider:?}/{version:?}");
                let temp = tempfile::tempdir().unwrap();
                let mut cfg = input_config(temp.path());
                cfg.provider = provider;
                cfg.family = family;
                cfg.version = version;
                let result = (|| -> Result<()> {
                    if version == RootVersion::Nightly {
                        let (workflow, branch) = provider_workflow(provider).unwrap();
                        cfg.nightly_run_id = GitHubClient::new(provider_repo(provider).unwrap())?
                            .recent_available_runs_matching(workflow, branch, |artifacts| {
                                provider_has_nightly_manager(provider, artifacts)
                            })?
                            .first()
                            .and_then(|r| r.run_id);
                        if cfg.nightly_run_id.is_none() {
                            return Err(LtboxError::Download(
                                "No available nightly builds under 90 days".into(),
                            ));
                        }
                    }
                    if version == RootVersion::Stable {
                        // Match the GUI picker, including published prereleases.
                        cfg.release_tag = Some(
                            GitHubClient::new(provider_repo(provider).unwrap())?
                                .recent_published_releases()?
                                .first()
                                .ok_or_else(|| {
                                    LtboxError::Download("No published releases".into())
                                })?
                                .tag
                                .clone(),
                        );
                    }
                    let manager = stage_root_manager_apk(&cfg, &mut Vec::new())?.unwrap();
                    let mut apk = zip::ZipArchive::new(fs::File::open(&manager)?)
                        .map_err(|e| LtboxError::Patch(e.to_string()))?;
                    apk.by_name("AndroidManifest.xml")
                        .map_err(|e| LtboxError::Patch(e.to_string()))?;
                    if family == RootFamily::KernelSU {
                        for (kernel, branch) in [
                            ("5.10", "android12"),
                            ("5.15", "android13"),
                            ("6.1", "android14"),
                            ("6.6", "android15"),
                        ] {
                            let mut kernel_cfg = cfg.clone();
                            kernel_cfg.work_dir = temp.path().join(kernel);
                            kernel_cfg.kernel_version = Some(kernel.into());
                            kernel_cfg.kernel_gki_branch = Some(branch.into());
                            if let Err(error) = stage_root_payload(&kernel_cfg, &mut Vec::new()) {
                                failures.push(format!("{label}/{branch}-{kernel}: {error}"));
                            } else {
                                let inputs = LocalKsuFiles {
                                    manager_apk: manager.clone(),
                                    ksuinit: kernel_cfg.work_dir.join("init"),
                                    module: kernel_cfg.work_dir.join("kernelsu.ko"),
                                };
                                match inputs.validate() {
                                    Ok(()) => eprintln!("PASS {label}/{branch}-{kernel}"),
                                    Err(error) => failures.push(format!(
                                        "{label}/{branch}-{kernel} payload validation: {error}"
                                    )),
                                }
                            }
                        }
                    } else {
                        stage_root_payload(&cfg, &mut Vec::new())?;
                    }
                    Ok(())
                })();
                match result {
                    Ok(()) => eprintln!("PASS {label} manager/payload"),
                    Err(error) => failures.push(format!("{label}: {error}")),
                }
            }
        }
        assert!(checked > 0, "Unknown provider filter: {filter}");
        assert!(
            failures.is_empty(),
            "Download contracts failed:\n{}",
            failures.join("\n")
        );
    }

    #[test]
    fn root_target_matrix_routes_tb320fc_families_to_boot() {
        for model in ["TB320FC", "LAVIETab9QHD1"] {
            assert_eq!(
                resolve_root_image_target(RootFamily::Magisk, false, model),
                RootImageTarget::Boot
            );
            assert_eq!(
                resolve_root_image_target(RootFamily::KernelSU, false, model),
                RootImageTarget::Boot
            );
            assert_eq!(
                resolve_root_image_target(RootFamily::KernelSU, true, model),
                RootImageTarget::Boot
            );
            assert_eq!(
                resolve_root_image_target(RootFamily::APatch, false, model),
                RootImageTarget::Boot
            );
            assert_eq!(
                resolve_root_image_target(RootFamily::Skroot, false, model),
                RootImageTarget::Boot
            );
        }
    }

    #[test]
    fn root_target_matrix_keeps_other_model_rules() {
        for family in [RootFamily::Magisk, RootFamily::KernelSU] {
            assert_eq!(
                resolve_root_image_target(family, false, "TB321FU"),
                RootImageTarget::InitBoot
            );
        }
        assert_eq!(
            resolve_root_image_target(RootFamily::KernelSU, true, "TB321FU"),
            RootImageTarget::Boot
        );
        assert_eq!(
            resolve_root_image_target(RootFamily::APatch, false, "TB321FU"),
            RootImageTarget::Boot
        );
        assert_eq!(
            resolve_root_image_target(RootFamily::Skroot, false, "TB321FU"),
            RootImageTarget::Boot
        );
    }

    #[test]
    fn only_tb320fc_family_rebuilds_vbmeta_for_a_boot_target() {
        for model in ["TB320FC", "LAVIETab9QHD1"] {
            assert!(root_run_rebuilds_vbmeta(RootImageTarget::Boot, model));
        }
        for model in ["TB321FU", "TB322FC", "TB323FU", "TB520FU", "TB710FU"] {
            assert!(!root_run_rebuilds_vbmeta(RootImageTarget::Boot, model));
        }
    }

    #[test]
    fn an_init_boot_target_always_rebuilds_its_hash_owner() {
        for model in ["TB320FC", "TB321FU", "TB322FC", "TB520FU", "TB710FU"] {
            assert!(root_run_rebuilds_vbmeta(RootImageTarget::InitBoot, model));
        }
    }

    #[test]
    fn root_target_names_are_consistent() {
        assert_eq!(RootImageTarget::Boot.partition_base(), "boot");
        assert_eq!(RootImageTarget::Boot.filename(), "boot.img");
        assert_eq!(RootImageTarget::InitBoot.partition_base(), "init_boot");
        assert_eq!(RootImageTarget::InitBoot.filename(), "init_boot.img");
    }
}
