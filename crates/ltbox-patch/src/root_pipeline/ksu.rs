//! KernelSU-family (KernelSU / KSU-Next / SukiSU / ReSukiSU) manager APK,
//! kernel `.ko`, and `ksuinit` payload acquisition.
//!
//! Also hosts the manager-APK orchestration entry point
//! [`stage_root_manager_apk`], which dispatches across all root families
//! and lives here because the KSU branches are the bulk of its logic.

use std::path::{Path, PathBuf};

use fs_err as fs;
use sha2::{Digest, Sha256};

use ltbox_core::downloader::download_to_file;
use ltbox_core::github::{GitHubClient, WorkflowArtifact};
use ltbox_core::i18n::tr;
use ltbox_core::{LtboxError, Result, tr_args};

use super::apatch::{download_apatch_payload_nightly, download_apatch_release_payload};
use super::apk::{
    copy_apk_to, extract_first_apk_from_zip, ksu_manager_nightly_preferences,
    ksu_manager_stable_preferences, select_manager_asset, stage_manager_from_downloaded_asset,
};
use super::magisk::{
    download_magisk_apk_nightly, download_magisk_release_apk, fetch_nightly_apk_outer_zip,
};
use super::{
    RootFamily, RootPipelineConfig, RootProvider, RootVersion, nightly_artifact_url, provider_repo,
    resolve_nightly_run,
};

fn download_ksu_manager_apk_release(
    provider: RootProvider,
    release_tag: Option<&str>,
    work_dir: &Path,
    manager_apk: &Path,
    log: &mut Vec<String>,
) -> Result<String> {
    let repo = provider_repo(provider).ok_or_else(|| {
        LtboxError::Patch(format!(
            "download_ksu_manager_apk: unsupported provider {provider:?}"
        ))
    })?;
    let client = GitHubClient::new(repo)?;
    let (tag, assets) = client.selected_release_assets(release_tag)?;
    let (name, url) = select_manager_asset(&assets, ksu_manager_stable_preferences(provider))
        .ok_or_else(|| LtboxError::Download(format!("No manager APK artifact on latest {repo}")))?;
    ltbox_core::live!(
        log,
        "[KSU] {repo} {}",
        tr_args!("log_release_latest_asset", tag = tag, name = name)
    );
    let asset_path = work_dir.join(&name);
    download_to_file(&url, &asset_path, log)?;
    stage_manager_from_downloaded_asset(&asset_path, manager_apk, "KSU", log)?;
    Ok(tag)
}

fn download_ksu_manager_apk_nightly(
    provider: RootProvider,
    manual_run_id: Option<u64>,
    work_dir: &Path,
    manager_apk: &Path,
    log: &mut Vec<String>,
) -> Result<u64> {
    let (repo, run_id) = resolve_nightly_run(provider, manual_run_id, log)?;
    let client = GitHubClient::new(repo)?;
    let artifact_names = client.workflow_artifacts(run_id)?;
    let pairs: Vec<(String, String)> = artifact_names
        .iter()
        .map(|name| (name.clone(), String::new()))
        .collect();
    let (artifact_name, _) =
        select_manager_asset(&pairs, ksu_manager_nightly_preferences(provider)).ok_or_else(
            || {
                LtboxError::Patch(format!(
                    "{repo} run {run_id}: no manager APK artifact (got {artifact_names:?})"
                ))
            },
        )?;
    ltbox_core::live!(
        log,
        "[KSU] {repo} {}",
        tr_args!("log_nightly_artifact", artifact = artifact_name)
    );
    fetch_nightly_apk_outer_zip(
        "KSU",
        repo,
        run_id,
        &artifact_name,
        "ksu_manager_nightly",
        work_dir,
        manager_apk,
        log,
    )?;
    Ok(run_id)
}

fn select_skroot_manager_asset(assets: &[(String, String)]) -> Option<(String, String)> {
    let is_lite_apk = |name: &str| {
        let lower = name.to_ascii_lowercase();
        lower.ends_with(".apk") && lower.contains("lite") && !lower.contains("pro")
    };

    select_manager_asset(assets, &["skroot_lite", "skroot-lite", "lite"])
        .filter(|(name, _)| is_lite_apk(name))
        .or_else(|| assets.iter().find(|(name, _)| is_lite_apk(name)).cloned())
}

fn download_skroot_manager_apk(
    work_dir: &Path,
    manager_apk: &Path,
    log: &mut Vec<String>,
) -> Result<String> {
    let repo = provider_repo(RootProvider::Skroot)
        .ok_or_else(|| LtboxError::Patch("SKRoot provider repo missing".into()))?;
    let client = GitHubClient::new(repo)?;
    let (tag, assets) = client.latest_release_assets()?;
    let (name, url) = select_skroot_manager_asset(&assets)
        .ok_or_else(|| LtboxError::Download(format!("No SKRoot Lite APK on latest {repo}")))?;
    ltbox_core::live!(
        log,
        "[SKRoot] {repo} {}",
        tr_args!("log_release_latest_asset", tag = tag, name = name)
    );
    let asset_path = work_dir.join(&name);
    download_to_file(&url, &asset_path, log)?;
    stage_manager_from_downloaded_asset(&asset_path, manager_apk, "SKRoot", log)?;
    Ok(tag)
}

/// Stage the manager APK used for post-root control into `work_dir/manager.apk`.
pub fn stage_root_manager_apk(
    cfg: &RootPipelineConfig,
    log: &mut Vec<String>,
) -> Result<Option<PathBuf>> {
    fs::create_dir_all(&cfg.work_dir)?;
    let manager_apk = cfg.work_dir.join("manager.apk");
    if manager_apk.exists() {
        fs::remove_file(&manager_apk).ok();
    }

    if cfg.gki_mode {
        let Some(kernel_zip) = cfg.gki_kernel_zip.as_ref() else {
            return Ok(None);
        };
        return if extract_first_apk_from_zip(kernel_zip, &manager_apk, "GKI", log)? {
            Ok(Some(manager_apk))
        } else {
            ltbox_core::live!(log, "[GKI] {}", tr("log_gki_no_manager_apk"));
            Ok(None)
        };
    }

    match cfg.family {
        RootFamily::Magisk => match (cfg.provider, cfg.version) {
            (RootProvider::MagiskFork, _) => {
                let src = cfg.magisk_forks_apk.as_ref().ok_or_else(|| {
                    LtboxError::Patch("Magisk forks require a local APK — none supplied".into())
                })?;
                copy_apk_to(src, &manager_apk)?;
                ltbox_core::live!(
                    log,
                    "[Magisk] {}",
                    tr_args!("log_magisk_staged_fork_apk", path = manager_apk.display())
                );
            }
            (_, RootVersion::Stable) => {
                download_magisk_release_apk(
                    cfg.provider,
                    cfg.release_tag.as_deref(),
                    &manager_apk,
                    log,
                )?;
            }
            (_, RootVersion::Nightly) => {
                download_magisk_apk_nightly(
                    cfg.provider,
                    cfg.nightly_run_id,
                    &cfg.work_dir,
                    &manager_apk,
                    log,
                )?;
            }
        },
        RootFamily::KernelSU => match cfg.version {
            RootVersion::Stable => {
                download_ksu_manager_apk_release(
                    cfg.provider,
                    cfg.release_tag.as_deref(),
                    &cfg.work_dir,
                    &manager_apk,
                    log,
                )?;
            }
            RootVersion::Nightly => {
                download_ksu_manager_apk_nightly(
                    cfg.provider,
                    cfg.nightly_run_id,
                    &cfg.work_dir,
                    &manager_apk,
                    log,
                )?;
            }
        },
        RootFamily::APatch => {
            let apk_path = cfg.work_dir.join("apatch.apk");
            match cfg.version {
                RootVersion::Stable => {
                    download_apatch_release_payload(
                        cfg.provider,
                        cfg.release_tag.as_deref(),
                        &cfg.work_dir,
                        log,
                    )?;
                }
                RootVersion::Nightly => {
                    download_apatch_payload_nightly(
                        cfg.provider,
                        cfg.nightly_run_id,
                        &cfg.work_dir,
                        log,
                    )?;
                }
            }
            copy_apk_to(&apk_path, &manager_apk)?;
            ltbox_core::live!(
                log,
                "[APatch] {}",
                tr_args!("log_staged_manager_apk", path = manager_apk.display())
            );
        }
        RootFamily::Skroot => {
            download_skroot_manager_apk(&cfg.work_dir, &manager_apk, log)?;
        }
    }

    Ok(Some(manager_apk))
}

// KSU payload: `.ko` is a release asset (per-kernel), `ksuinit` is a
// workflow artifact fetched via `nightly.link` (GitHub API needs auth).

/// Reduce kernel version to `major.minor` for KSU asset matching
/// (e.g. `6.6.118` → `6.6`). Already-short strings pass through.
pub fn normalize_ksu_kernel_version(kver: &str) -> Option<String> {
    let trimmed = kver.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut parts = trimmed.split('.');
    let major = parts.next()?;
    let minor = parts.next()?;
    if major.is_empty() || !major.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let minor_digits: String = minor.chars().take_while(|c| c.is_ascii_digit()).collect();
    if minor_digits.is_empty() {
        return None;
    }
    Some(format!("{major}.{minor_digits}"))
}

/// True iff `lower_filename` embeds `kver` between `-{kver}_` delimiters.
/// Prevents unanchored `"6.1"` from matching 6.10 / 6.11 / etc.
fn ksu_ko_kver_matches(lower_filename: &str, kver: &str) -> bool {
    let needle = format!("-{kver}_");
    lower_filename.contains(&needle)
}

/// The GKI branch embedded in a kernel release string — `android12` out of
/// `5.10.198-android12-9-gabc1234`.
///
/// A kernel version alone does not identify a GKI: 5.10 ships as both
/// `android12-5.10` and `android13-5.10`, and an LKM built for the wrong one
/// loads and then leaves no root after a reboot, with no error to show for it
/// (issue #93). `/proc/version` carries the branch, so read it rather than
/// guessing from the version.
pub fn ksu_gki_branch(kernel_release: &str) -> Option<String> {
    let lower = kernel_release.to_ascii_lowercase();
    let start = lower.find("android")?;
    let digits: String = lower[start + "android".len()..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    (!digits.is_empty()).then(|| format!("android{digits}"))
}

/// Whether an asset's GKI branch disqualifies it for this device.
///
/// With the device branch known, the asset has to name that exact branch —
/// nothing else can be right. Without it (manual kernel-version entry, or a
/// `/proc/version` with no branch at all) fall back to the one pairing that is
/// known-wrong on every tablet LTBox supports: their 5.10 kernels are all the
/// Android 12 GKI, so an android13 module never belongs on one.
fn branch_rejected(lower_filename: &str, kver: &str, device_branch: Option<&str>) -> bool {
    match device_branch {
        Some(branch) => !lower_filename.contains(branch),
        None => kver == "5.10" && lower_filename.contains("android13"),
    }
}

fn select_ksu_release_ko_asset(
    assets: &[(String, String)],
    kver: &str,
    device_branch: Option<&str>,
) -> Option<(String, String)> {
    let want = kver.to_lowercase();
    assets
        .iter()
        .find(|(n, _)| {
            let lower = n.to_lowercase();
            lower.ends_with("_kernelsu.ko")
                && ksu_ko_kver_matches(&lower, &want)
                && !branch_rejected(&lower, &want, device_branch)
        })
        .cloned()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ArtifactArchitecture {
    Aarch64,
    Unmarked,
    Rejected,
}

fn artifact_architecture(name: &str) -> ArtifactArchitecture {
    let lower = name.to_ascii_lowercase();
    if lower.contains("x86_64") || lower.contains("x86") || lower.contains("amd64") {
        ArtifactArchitecture::Rejected
    } else if lower.contains("aarch64") || lower.contains("arm64") {
        ArtifactArchitecture::Aarch64
    } else {
        ArtifactArchitecture::Unmarked
    }
}

/// Pick an arm64-safe artifact independently of the order returned by GitHub.
/// Current aarch64 names win over legacy names with no architecture marker;
/// explicitly x86-family names are never eligible.
fn select_arch_safe_artifact(
    artifact_names: &[String],
    matches_payload: impl Fn(&str) -> bool,
) -> Option<String> {
    artifact_names
        .iter()
        .find(|name| {
            matches_payload(name) && artifact_architecture(name) == ArtifactArchitecture::Aarch64
        })
        .or_else(|| {
            artifact_names.iter().find(|name| {
                matches_payload(name)
                    && artifact_architecture(name) == ArtifactArchitecture::Unmarked
            })
        })
        .cloned()
}

fn select_ksu_nightly_ko_artifact(
    artifact_names: &[String],
    kver: &str,
    device_branch: Option<&str>,
) -> Option<String> {
    // Accept legacy `_kernelsu.ko` and current `-{kver}-lkm` naming.
    // Trailing `-`/EOS sentinel prevents `6.1` matching `6.10/6.11/6.12`.
    let want = kver.to_lowercase();
    let lkm_marker = format!("-{want}-lkm");
    select_arch_safe_artifact(artifact_names, |n| {
        let lower = n.to_lowercase();
        if branch_rejected(&lower, &want, device_branch) {
            return false;
        }
        // Legacy: "*-{kver}_kernelsu.ko"
        if lower.contains("_kernelsu.ko") && ksu_ko_kver_matches(&lower, &want) {
            return true;
        }
        // Current: "android<api>-{kver}-lkm" (zip wrapper, real
        // .ko inside).
        lower.contains(&lkm_marker)
    })
}

fn select_ksuinit_artifact(artifact_names: &[String]) -> Option<String> {
    select_arch_safe_artifact(artifact_names, |name| {
        name.to_ascii_lowercase().starts_with("ksuinit")
    })
}

#[derive(Debug, PartialEq, Eq)]
enum ArtifactDigestStatus {
    Verified,
    Skipped,
}

#[derive(Debug, PartialEq, Eq)]
struct ArtifactDigestMismatch {
    expected: String,
    actual: String,
}

fn compare_artifact_digest(
    reported_digest: Option<&str>,
    actual_sha256: &str,
) -> std::result::Result<ArtifactDigestStatus, ArtifactDigestMismatch> {
    let Some(reported) = reported_digest.filter(|digest| !digest.is_empty()) else {
        return Ok(ArtifactDigestStatus::Skipped);
    };
    let Some(expected) = reported.strip_prefix("sha256:") else {
        return Ok(ArtifactDigestStatus::Skipped);
    };
    if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Ok(ArtifactDigestStatus::Skipped);
    }
    if expected.eq_ignore_ascii_case(actual_sha256) {
        Ok(ArtifactDigestStatus::Verified)
    } else {
        Err(ArtifactDigestMismatch {
            expected: expected.to_string(),
            actual: actual_sha256.to_string(),
        })
    }
}

fn sha256_hex_file(path: &Path) -> Result<String> {
    use std::io::Read;

    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn verify_nightly_artifact_zip(
    path: &Path,
    artifact_name: &str,
    reported_digest: Option<&str>,
    log: &mut Vec<String>,
) -> Result<()> {
    let actual = sha256_hex_file(path)?;
    match compare_artifact_digest(reported_digest, &actual) {
        Ok(ArtifactDigestStatus::Verified) => Ok(()),
        Ok(ArtifactDigestStatus::Skipped) => {
            ltbox_core::live!(
                log,
                "[KSU] {}",
                tr_args!("log_ksu_artifact_hash_skipped", artifact = artifact_name)
            );
            Ok(())
        }
        Err(mismatch) => {
            let _ = fs::remove_file(path);
            Err(LtboxError::Download(tr_args!(
                "err_ksu_artifact_hash_mismatch",
                artifact = artifact_name,
                expected = mismatch.expected,
                actual = mismatch.actual,
            )))
        }
    }
}

fn artifact_digest<'a>(artifacts: &'a [WorkflowArtifact], name: &str) -> Option<&'a str> {
    artifacts
        .iter()
        .find(|artifact| artifact.name == name)
        .and_then(|artifact| artifact.digest.as_deref())
}

fn download_ksu_ko_artifact(
    repo: &str,
    run_id: u64,
    ko_artifact: &str,
    reported_digest: Option<&str>,
    staging_dir: &Path,
    log: &mut Vec<String>,
) -> Result<()> {
    let ko_zip_path = staging_dir.join("ksu_lkm_artifact.zip");
    let ko_url = nightly_artifact_url(repo, run_id, ko_artifact);
    download_to_file(&ko_url, &ko_zip_path, log)?;
    verify_nightly_artifact_zip(&ko_zip_path, ko_artifact, reported_digest, log)?;
    {
        let f = fs::File::open(&ko_zip_path)?;
        let mut archive = zip::ZipArchive::new(f)
            .map_err(|e| LtboxError::Patch(format!("{repo}: LKM artifact not a zip: {e}")))?;
        let member_name: String = archive
            .file_names()
            .find(|n| n.to_lowercase().ends_with(".ko"))
            .map(|s| s.to_string())
            .ok_or_else(|| {
                LtboxError::Patch(format!("{repo} {ko_artifact}: no .ko entry in zip"))
            })?;
        let mut entry = archive
            .by_name(&member_name)
            .map_err(|e| LtboxError::Patch(format!("{repo} {ko_artifact}: {e}")))?;
        let ko_path = staging_dir.join("kernelsu.ko");
        crate::zip_util::copy_capped(
            &mut entry,
            &ko_path,
            crate::zip_util::MAX_ENTRY_BYTES,
            &member_name,
        )?;
    }
    let _ = fs::remove_file(&ko_zip_path);
    Ok(())
}

fn stable_lkm_sources_exhausted(
    tag: &str,
    kver: &str,
    details: impl std::fmt::Display,
) -> LtboxError {
    let details = details.to_string();
    let translated = tr_args!(
        "err_ksu_stable_lkm_sources_exhausted",
        tag = tag,
        kver = kver,
        details = details,
    );
    let message = if translated == "err_ksu_stable_lkm_sources_exhausted" {
        format!(
            "No KernelSU LKM was found after trying both release assets and workflow artifacts for release {tag} and kernel {kver}. Workflow artifacts may have expired (typically after about 90 days). Details: {details}"
        )
    } else {
        translated
    };
    LtboxError::Download(message)
}

pub fn download_ksu_payload(
    provider: RootProvider,
    kernel_version: Option<&str>,
    device_branch: Option<&str>,
    staging_dir: &Path,
    log: &mut Vec<String>,
) -> Result<()> {
    download_ksu_release_payload(
        provider,
        None,
        kernel_version,
        device_branch,
        staging_dir,
        log,
    )
}

pub(super) fn download_ksu_release_payload(
    provider: RootProvider,
    release_tag: Option<&str>,
    kernel_version: Option<&str>,
    device_branch: Option<&str>,
    staging_dir: &Path,
    log: &mut Vec<String>,
) -> Result<()> {
    let repo = provider_repo(provider)
        .ok_or_else(|| LtboxError::Patch(format!("Unknown KSU provider: {provider:?}")))?;
    let client = GitHubClient::new(repo)?;
    let (tag, assets) = client.selected_release_assets(release_tag)?;
    ltbox_core::live!(
        log,
        "[KSU] {}",
        tr_args!("log_ksu_latest_release", tag = tag)
    );

    // -------- 1. Per-kernel `.ko` from release assets --------
    // KSU tags assets by kernel branch (`android15-6.6_kernelsu.ko`);
    // strip patch suffix from device kver before matching.
    let kver = kernel_version
        .and_then(normalize_ksu_kernel_version)
        .ok_or_else(|| {
            LtboxError::Download(
                "KernelSU LKM requires a kernel version such as `6.1`; no safe module fallback is allowed."
                    .into(),
            )
        })?;
    fs::create_dir_all(staging_dir)?;
    let release_ko = select_ksu_release_ko_asset(&assets, &kver, device_branch);
    if let Some((ko_name, ko_url)) = release_ko.as_ref() {
        ltbox_core::live!(
            log,
            "[KSU] {}",
            tr_args!("log_ksu_downloading_lkm_release_asset", name = ko_name)
        );
        let ko_path = staging_dir.join("kernelsu.ko");
        download_to_file(ko_url, &ko_path, log)?;
    }

    // Resolve the release-tag run once. It always supplies ksuinit and, when
    // the release no longer publishes a raw .ko, supplies the LKM fallback.
    let run_id = client.workflow_run_for_tag(&tag).map_err(|e| {
        if release_ko.is_none() {
            stable_lkm_sources_exhausted(&tag, &kver, e)
        } else {
            LtboxError::Download(format!(
                "No workflow run found for tag {tag} on {repo}: {e}"
            ))
        }
    })?;
    let artifacts = client.workflow_artifact_details(run_id).map_err(|e| {
        if release_ko.is_none() {
            stable_lkm_sources_exhausted(&tag, &kver, e)
        } else {
            LtboxError::Download(format!("Cannot list artifacts for run {run_id}: {e}"))
        }
    })?;
    let artifact_names: Vec<String> = artifacts
        .iter()
        .map(|artifact| artifact.name.clone())
        .collect();

    if release_ko.is_none() {
        let ko_artifact = select_ksu_nightly_ko_artifact(&artifact_names, &kver, device_branch)
            .ok_or_else(|| {
                stable_lkm_sources_exhausted(
                    &tag,
                    &kver,
                    format!("run {run_id} artifacts: {artifact_names:?}"),
                )
            })?;
        ltbox_core::live!(
            log,
            "[KSU] {}",
            tr_args!(
                "log_ksu_downloading_lkm_workflow_artifact",
                name = ko_artifact
            )
        );
        download_ksu_ko_artifact(
            repo,
            run_id,
            &ko_artifact,
            artifact_digest(&artifacts, &ko_artifact),
            staging_dir,
            log,
        )
        .map_err(|e| stable_lkm_sources_exhausted(&tag, &kver, e))?;
    }

    // -------- 2. `ksuinit` binary via nightly.link --------
    let ksuinit_artifact = select_ksuinit_artifact(&artifact_names).ok_or_else(|| {
        LtboxError::Download(format!(
            "No arm64-safe `ksuinit*` workflow artifact on run {run_id} of {repo}"
        ))
    })?;
    let nightly_url = format!(
        "https://nightly.link/{repo}/actions/runs/{run_id}/{ksuinit_artifact}.zip",
        repo = repo,
        run_id = run_id,
        ksuinit_artifact = ksuinit_artifact,
    );
    ltbox_core::live!(
        log,
        "[KSU] {}",
        tr_args!("log_ksu_downloading_ksuinit", name = ksuinit_artifact)
    );
    let tmp_zip = staging_dir.join(format!("{ksuinit_artifact}.zip"));
    download_to_file(&nightly_url, &tmp_zip, log)?;
    verify_nightly_artifact_zip(
        &tmp_zip,
        &ksuinit_artifact,
        artifact_digest(&artifacts, &ksuinit_artifact),
        log,
    )?;

    let file = fs::File::open(&tmp_zip)
        .map_err(|e| LtboxError::Patch(format!("open ksuinit zip: {e}")))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| LtboxError::Patch(format!("ksuinit zip read: {e}")))?;
    let member_name: Option<String> = archive
        .file_names()
        .find(|n| n.ends_with("ksuinit") && !n.ends_with('/'))
        .map(|s| s.to_string());
    let member_name = member_name.ok_or_else(|| {
        LtboxError::Patch(format!(
            "`ksuinit` entry missing from {ksuinit_artifact}.zip"
        ))
    })?;
    let mut entry = archive
        .by_name(&member_name)
        .map_err(|e| LtboxError::Patch(format!("ksuinit zip entry: {e}")))?;
    // magiskboot expects `init`, not `ksuinit`.
    let init_path = staging_dir.join("init");
    let copied = crate::zip_util::copy_capped(
        &mut entry,
        &init_path,
        crate::zip_util::MAX_ENTRY_BYTES,
        &member_name,
    )?;
    drop(entry);
    let _ = fs::remove_file(&tmp_zip);
    ltbox_core::live!(
        log,
        "[KSU] {}",
        tr_args!("log_ksu_staged_init_lkm", bytes = copied)
    );
    Ok(())
}

/// Download `.ko` + `init` from a KSU nightly run into `staging_dir`.
/// LKM selection requires an exact kernel major.minor match.
/// `manual_run_id = None` → latest successful run on provider's workflow.
pub fn download_ksu_payload_nightly(
    provider: RootProvider,
    kernel_version: Option<&str>,
    device_branch: Option<&str>,
    manual_run_id: Option<u64>,
    staging_dir: &Path,
    log: &mut Vec<String>,
) -> Result<u64> {
    let (repo, run_id) = resolve_nightly_run(provider, manual_run_id, log)?;
    let client = GitHubClient::new(repo)?;
    let artifacts = client.workflow_artifact_details(run_id)?;
    let artifact_names: Vec<String> = artifacts
        .iter()
        .map(|artifact| artifact.name.clone())
        .collect();
    if artifact_names.is_empty() {
        return Err(LtboxError::Patch(format!(
            "{repo} run {run_id} has no artifacts"
        )));
    }

    fs::create_dir_all(staging_dir)?;
    let kver = kernel_version
        .and_then(normalize_ksu_kernel_version)
        .ok_or_else(|| {
            LtboxError::Patch(
                "KernelSU Nightly LKM requires a kernel version such as `6.1`; no safe module fallback is allowed."
                    .into(),
            )
        })?;

    // -------- 1. Kernel `.ko` --------
    let ko_artifact = select_ksu_nightly_ko_artifact(&artifact_names, &kver, device_branch)
        .ok_or_else(|| {
            LtboxError::Patch(format!(
            "{repo} run {run_id}: no *_kernelsu.ko artifact matching kernel {kver} (artifacts={artifact_names:?})"
        ))
    })?;
    ltbox_core::live!(
        log,
        "[KSU] {}",
        tr_args!("log_ksu_nightly_lkm_artifact", artifact = ko_artifact)
    );
    download_ksu_ko_artifact(
        repo,
        run_id,
        &ko_artifact,
        artifact_digest(&artifacts, &ko_artifact),
        staging_dir,
        log,
    )?;

    // -------- 2. ksuinit → `init` --------
    let init_artifact = select_ksuinit_artifact(&artifact_names).ok_or_else(|| {
        LtboxError::Patch(format!(
            "{repo} run {run_id}: no arm64-safe ksuinit artifact (got {artifact_names:?})"
        ))
    })?;
    ltbox_core::live!(
        log,
        "[KSU] {}",
        tr_args!("log_ksu_nightly_ksuinit_artifact", artifact = init_artifact)
    );
    let init_zip_path = staging_dir.join("ksu_nightly_init.zip");
    let init_url = nightly_artifact_url(repo, run_id, &init_artifact);
    download_to_file(&init_url, &init_zip_path, log)?;
    verify_nightly_artifact_zip(
        &init_zip_path,
        &init_artifact,
        artifact_digest(&artifacts, &init_artifact),
        log,
    )?;
    {
        let f = fs::File::open(&init_zip_path)?;
        let mut archive = zip::ZipArchive::new(f)
            .map_err(|e| LtboxError::Patch(format!("{repo}: ksuinit artifact not a zip: {e}")))?;
        let member_name: String = archive
            .file_names()
            .find(|n| n.ends_with("ksuinit") && !n.ends_with('/'))
            .map(|s| s.to_string())
            .ok_or_else(|| {
                LtboxError::Patch(format!("{repo} {init_artifact}: no ksuinit entry in zip"))
            })?;
        let mut entry = archive
            .by_name(&member_name)
            .map_err(|e| LtboxError::Patch(format!("{repo} {init_artifact}: {e}")))?;
        let init_path = staging_dir.join("init");
        crate::zip_util::copy_capped(
            &mut entry,
            &init_path,
            crate::zip_util::MAX_ENTRY_BYTES,
            &member_name,
        )?;
    }
    let _ = fs::remove_file(&init_zip_path);
    ltbox_core::live!(log, "[KSU] {}", tr("log_ksu_staged_nightly_init"));
    Ok(run_id)
}

#[cfg(test)]
mod tests {
    use super::{
        ArtifactDigestMismatch, ArtifactDigestStatus, RootProvider, compare_artifact_digest,
        download_ksu_manager_apk_nightly, download_ksu_manager_apk_release, download_ksu_payload,
        download_ksu_payload_nightly, ksu_gki_branch, ksu_ko_kver_matches,
        normalize_ksu_kernel_version, select_ksu_nightly_ko_artifact, select_ksu_release_ko_asset,
        select_ksuinit_artifact, select_skroot_manager_asset, stable_lkm_sources_exhausted,
    };

    const SHA256_LOWER: &str = "df471282e461086739bebb088aa07c7226158ffc7a8f5495c86d2e10dba37e83";

    #[test]
    fn artifact_digest_match_passes() {
        let reported = format!("sha256:{SHA256_LOWER}");
        assert_eq!(
            compare_artifact_digest(Some(&reported), SHA256_LOWER),
            Ok(ArtifactDigestStatus::Verified)
        );
    }

    #[test]
    fn artifact_digest_mismatch_fails() {
        let reported = format!("sha256:{SHA256_LOWER}");
        let actual = "0000000000000000000000000000000000000000000000000000000000000000";
        assert_eq!(
            compare_artifact_digest(Some(&reported), actual),
            Err(ArtifactDigestMismatch {
                expected: SHA256_LOWER.to_string(),
                actual: actual.to_string(),
            })
        );
    }

    #[test]
    fn artifact_digest_absent_or_unusable_skips() {
        for reported in [
            None,
            Some(""),
            Some(SHA256_LOWER),
            Some("sha256:not-a-hex-digest"),
        ] {
            assert_eq!(
                compare_artifact_digest(reported, SHA256_LOWER),
                Ok(ArtifactDigestStatus::Skipped)
            );
        }
    }

    #[test]
    fn artifact_digest_hex_comparison_is_case_insensitive() {
        let reported = format!("sha256:{}", SHA256_LOWER.to_ascii_uppercase());
        assert_eq!(
            compare_artifact_digest(Some(&reported), SHA256_LOWER),
            Ok(ArtifactDigestStatus::Verified)
        );
    }

    #[test]
    fn exact_major_minor_matches() {
        assert!(ksu_ko_kver_matches("android15-6.1_kernelsu.ko", "6.1"));
        assert!(ksu_ko_kver_matches("android14-5.15_kernelsu.ko", "5.15"));
    }

    #[test]
    fn longer_minor_does_not_match_shorter_prefix() {
        // Regression: unanchored `contains("6.1")` used to match 6.10/6.11/etc.
        assert!(!ksu_ko_kver_matches("android15-6.10_kernelsu.ko", "6.1"));
        assert!(!ksu_ko_kver_matches("android15-6.11_kernelsu.ko", "6.1"));
        assert!(!ksu_ko_kver_matches("android15-6.12_kernelsu.ko", "6.1"));
        assert!(!ksu_ko_kver_matches("android15-6.13_kernelsu.ko", "6.1"));
    }

    #[test]
    fn different_major_does_not_match() {
        assert!(!ksu_ko_kver_matches("android15-5.15_kernelsu.ko", "6.1"));
        assert!(!ksu_ko_kver_matches("android14-6.1_kernelsu.ko", "5.15"));
    }

    #[test]
    fn missing_leading_dash_does_not_match() {
        // `-{kver}_` boundary is required; bare `6.1_kernelsu.ko` is not a stock layout.
        assert!(!ksu_ko_kver_matches("6.1_kernelsu.ko", "6.1"));
    }

    #[test]
    fn ksu_kernel_version_normalizes_to_major_minor() {
        assert_eq!(normalize_ksu_kernel_version("6.1"), Some("6.1".to_string()));
        assert_eq!(
            normalize_ksu_kernel_version("6.1.75"),
            Some("6.1".to_string())
        );
        assert_eq!(
            normalize_ksu_kernel_version("  5.15.149-android14  "),
            Some("5.15".to_string())
        );
    }

    #[test]
    fn ksu_kernel_version_rejects_missing_or_malformed_input() {
        assert_eq!(normalize_ksu_kernel_version(""), None);
        assert_eq!(normalize_ksu_kernel_version("6"), None);
        assert_eq!(normalize_ksu_kernel_version("six.one"), None);
    }

    #[test]
    fn ksu_release_asset_selection_requires_matching_kernel() {
        let assets = vec![
            (
                "android14-5.15_kernelsu.ko".to_string(),
                "https://example.invalid/5.15.ko".to_string(),
            ),
            (
                "android15-6.6_kernelsu.ko".to_string(),
                "https://example.invalid/6.6.ko".to_string(),
            ),
        ];

        let picked = select_ksu_release_ko_asset(&assets, "6.6", None).expect("6.6 asset");
        assert_eq!(picked.0, "android15-6.6_kernelsu.ko");
        assert!(select_ksu_release_ko_asset(&assets, "6.1", None).is_none());
    }

    #[test]
    fn ksu_nightly_artifact_selection_does_not_fallback_to_any_module() {
        let artifacts = vec![
            "android14-5.15_kernelsu.ko".to_string(),
            "ksuinit-arm64.zip".to_string(),
        ];

        assert_eq!(
            select_ksu_nightly_ko_artifact(&artifacts, "5.15", None),
            Some("android14-5.15_kernelsu.ko".to_string())
        );
        assert_eq!(
            select_ksu_nightly_ko_artifact(&artifacts, "6.1", None),
            None
        );
    }

    #[test]
    fn ksu_nightly_artifact_selection_picks_new_lkm_naming() {
        // Real artifact list emitted by 2026 KernelSU / KSU-Next /
        // SukiSU / ReSukiSU nightlies — bare `<branch>-<kver>-lkm`
        // wrapper instead of the old `*_kernelsu.ko` filename.
        let artifacts = vec![
            "manager".to_string(),
            "ksud-aarch64-linux-android".to_string(),
            "android16-6.12-lkm".to_string(),
            "android15-6.6-lkm".to_string(),
            "android14-5.15-lkm".to_string(),
            "android14-6.1-lkm".to_string(),
            "android13-5.10-lkm".to_string(),
            "ksuinit".to_string(),
        ];

        assert_eq!(
            select_ksu_nightly_ko_artifact(&artifacts, "6.6", None),
            Some("android15-6.6-lkm".to_string())
        );
        assert_eq!(
            select_ksu_nightly_ko_artifact(&artifacts, "5.15", None),
            Some("android14-5.15-lkm".to_string())
        );
        // 6.1 must not steal 6.10 / 6.11 / 6.12 — kver match anchors
        // both sides via the surrounding `-` markers.
        assert_eq!(
            select_ksu_nightly_ko_artifact(&artifacts, "6.1", None),
            Some("android14-6.1-lkm".to_string())
        );
        // No 4.x in this artifact set.
        assert_eq!(
            select_ksu_nightly_ko_artifact(&artifacts, "4.14", None),
            None
        );
    }

    #[test]
    fn the_gki_branch_is_read_out_of_the_kernel_release() {
        assert_eq!(
            ksu_gki_branch("5.10.198-android12-9-gabc1234"),
            Some("android12".to_string())
        );
        assert_eq!(
            ksu_gki_branch("6.6.30-android15-8-g0123456-ab12345678"),
            Some("android15".to_string())
        );
        // Old kernels predate the GKI branch entirely.
        assert_eq!(ksu_gki_branch("4.14.180"), None);
        // A manually typed version carries nothing to read.
        assert_eq!(ksu_gki_branch("5.10"), None);
    }

    #[test]
    fn a_known_branch_picks_its_own_module_at_the_same_kernel_version() {
        // The issue-93 shape: one kernel version, two branches published.
        let artifacts = vec![
            "android13-5.10-lkm".to_string(),
            "android12-5.10-lkm".to_string(),
        ];
        assert_eq!(
            select_ksu_nightly_ko_artifact(&artifacts, "5.10", Some("android12")),
            Some("android12-5.10-lkm".to_string())
        );
        // And the other way round, so this is a match rather than a blocklist.
        assert_eq!(
            select_ksu_nightly_ko_artifact(&artifacts, "5.10", Some("android13")),
            Some("android13-5.10-lkm".to_string())
        );

        let assets = vec![
            (
                "android13-5.10_kernelsu.ko".to_string(),
                "https://example.invalid/13".to_string(),
            ),
            (
                "android12-5.10_kernelsu.ko".to_string(),
                "https://example.invalid/12".to_string(),
            ),
        ];
        assert_eq!(
            select_ksu_release_ko_asset(&assets, "5.10", Some("android12")).map(|(name, _)| name),
            Some("android12-5.10_kernelsu.ko".to_string())
        );
    }

    #[test]
    fn a_repo_without_the_devices_branch_finds_nothing() {
        // Failing loudly beats a module that loads and then drops root on the
        // next boot, which is what the silent mismatch did.
        let artifacts = vec!["android13-5.10-lkm".to_string()];
        assert_eq!(
            select_ksu_nightly_ko_artifact(&artifacts, "5.10", Some("android12")),
            None
        );
    }

    #[test]
    fn an_unknown_branch_still_refuses_android13_on_5_10() {
        // Manual kernel-version entry has no branch to match on. Every 5.10
        // tablet LTBox supports is the Android 12 GKI, so this pairing stays
        // barred even then.
        let artifacts = vec![
            "android13-5.10-lkm".to_string(),
            "android12-5.10-lkm".to_string(),
        ];
        assert_eq!(
            select_ksu_nightly_ko_artifact(&artifacts, "5.10", None),
            Some("android12-5.10-lkm".to_string())
        );
        // The fallback is scoped to 5.10; android13 ships a 5.15 GKI too.
        let artifacts = vec!["android13-5.15-lkm".to_string()];
        assert_eq!(
            select_ksu_nightly_ko_artifact(&artifacts, "5.15", None),
            Some("android13-5.15-lkm".to_string())
        );
    }

    #[test]
    fn ksu_artifact_selection_prefers_aarch64_in_live_upstream_order() {
        // Regression fixture from tiann/KernelSU run 33170552380: GitHub listed
        // the matching x86_64 LKM before aarch64 and ksuinit-x86_64 before
        // ksuinit-aarch64.
        let artifacts = vec![
            "x86_64-android14-6.1-lkm".to_string(),
            "aarch64-android14-6.1-lkm".to_string(),
            "ksuinit-x86_64".to_string(),
            "ksuinit-aarch64".to_string(),
        ];

        assert_eq!(
            select_ksu_nightly_ko_artifact(&artifacts, "6.1", None),
            Some("aarch64-android14-6.1-lkm".to_string())
        );
        assert_eq!(
            select_ksuinit_artifact(&artifacts),
            Some("ksuinit-aarch64".to_string())
        );
    }

    #[test]
    fn ksu_artifact_selection_rejects_x86_only_matches() {
        let artifacts = vec![
            "x86_64-android14-6.1-lkm".to_string(),
            "ksuinit-amd64".to_string(),
        ];

        assert_eq!(
            select_ksu_nightly_ko_artifact(&artifacts, "6.1", None),
            None
        );
        assert_eq!(select_ksuinit_artifact(&artifacts), None);
    }

    #[test]
    fn stable_lkm_exhausted_error_names_both_sources_and_match() {
        let error =
            stable_lkm_sources_exhausted("v3.3.0", "6.1", "no matching artifact").to_string();

        assert!(error.contains("both release assets and workflow artifacts"));
        assert!(error.contains("v3.3.0"));
        assert!(error.contains("6.1"));
        assert!(error.contains("90 days"));
    }

    #[test]
    fn skroot_manager_asset_selection_picks_lite_not_pro() {
        let assets = vec![
            (
                "SKRoot_Pro.2026-6-1.apk".to_string(),
                "https://example.invalid/pro.apk".to_string(),
            ),
            (
                "notes.txt".to_string(),
                "https://example.invalid/notes.txt".to_string(),
            ),
            (
                "SKRoot_Lite.2026-6-1.apk".to_string(),
                "https://example.invalid/lite.apk".to_string(),
            ),
        ];

        let picked = select_skroot_manager_asset(&assets).expect("lite asset");
        assert_eq!(picked.0, "SKRoot_Lite.2026-6-1.apk");

        let pro_only = vec![(
            "SKRoot_Pro.2026-6-1.apk".to_string(),
            "https://example.invalid/pro.apk".to_string(),
        )];
        assert!(select_skroot_manager_asset(&pro_only).is_none());
    }

    /// Network-dependent end-to-end probe of every LKM provider's
    /// manager-APK fetch path (Stable + Nightly auto). Each iteration
    /// uses an isolated tempdir so failures don't poison subsequent
    /// runs. Marked `#[ignore]` so CI / `cargo test` skip it; run
    /// locally with:
    ///
    ///     cargo test -p ltbox-patch --lib -- --ignored --nocapture lkm_manager_download_smoke
    ///
    /// Pass criteria per provider/channel:
    /// 1. Function returns `Ok(_)`.
    /// 2. `manager.apk` exists at the expected path.
    /// 3. The file is non-empty (full APK download / extraction).
    #[test]
    #[ignore = "hits GitHub releases + nightly.link; run manually"]
    fn lkm_manager_download_smoke() {
        let providers: &[(RootProvider, &str)] = &[
            (RootProvider::KernelSU, "tiann/KernelSU"),
            (RootProvider::KernelSUNext, "KernelSU-Next/KernelSU-Next"),
            (RootProvider::SukiSU, "SukiSU-Ultra/SukiSU-Ultra"),
            (RootProvider::ReSukiSU, "ReSukiSU/ReSukiSU"),
        ];

        let mut report: Vec<(String, String)> = Vec::new();

        for (provider, repo) in providers.iter().copied() {
            // ----- Stable -----
            let stable_label = format!("{repo} stable");
            // ReSukiSU has no Stable releases — expect Err.
            if matches!(provider, RootProvider::ReSukiSU) {
                report.push((
                    stable_label.clone(),
                    "skipped (no Stable channel)".to_string(),
                ));
            } else {
                let tmp = tempfile::tempdir().expect("tempdir");
                let manager_apk = tmp.path().join("manager.apk");
                let mut log = Vec::new();
                let result = download_ksu_manager_apk_release(
                    provider,
                    None,
                    tmp.path(),
                    &manager_apk,
                    &mut log,
                );
                let outcome = match result {
                    Ok(tag) => match (
                        manager_apk.exists(),
                        std::fs::metadata(&manager_apk)
                            .map(|m| m.len())
                            .unwrap_or(0),
                    ) {
                        (true, n) if n > 0 => format!("OK tag={tag} size={n}"),
                        (true, _) => "FAIL: manager.apk empty".to_string(),
                        (false, _) => "FAIL: manager.apk missing".to_string(),
                    },
                    Err(e) => format!("FAIL: {e}"),
                };
                eprintln!("[{stable_label}] {outcome}");
                report.push((stable_label, outcome));
            }

            // ----- Nightly auto-detect -----
            let nightly_label = format!("{repo} nightly");
            let tmp = tempfile::tempdir().expect("tempdir");
            let manager_apk = tmp.path().join("manager.apk");
            let mut log = Vec::new();
            let result = download_ksu_manager_apk_nightly(
                provider,
                None,
                tmp.path(),
                &manager_apk,
                &mut log,
            );
            let outcome = match result {
                Ok(run_id) => match (
                    manager_apk.exists(),
                    std::fs::metadata(&manager_apk)
                        .map(|m| m.len())
                        .unwrap_or(0),
                ) {
                    (true, n) if n > 0 => format!("OK run={run_id} size={n}"),
                    (true, _) => "FAIL: manager.apk empty".to_string(),
                    (false, _) => "FAIL: manager.apk missing".to_string(),
                },
                Err(e) => format!("FAIL: {e}"),
            };
            eprintln!("[{nightly_label}] {outcome}");
            report.push((nightly_label, outcome));
        }

        eprintln!("\n=== LKM manager-APK download report ===");
        for (label, outcome) in &report {
            eprintln!("  {label}: {outcome}");
        }
        eprintln!();

        let failures: Vec<&(String, String)> = report
            .iter()
            .filter(|(_, o)| o.starts_with("FAIL"))
            .collect();
        assert!(
            failures.is_empty(),
            "{} provider/channel combinations failed: {:#?}",
            failures.len(),
            failures
        );
    }

    /// Network-dependent probe for the full `download_ksu_payload`
    /// path — `.ko` (kernel module) + `ksuinit` artifact extraction —
    /// against kernel `6.6` for every KSU-family provider that ships
    /// release artifacts.
    ///
    ///     cargo test -p ltbox-patch --lib -- --ignored --nocapture lkm_payload_download_smoke
    #[test]
    #[ignore = "hits GitHub releases + nightly.link; run manually"]
    fn lkm_payload_download_smoke() {
        const KVER: &str = "6.6";
        let providers: &[(RootProvider, &str)] = &[
            (RootProvider::KernelSU, "tiann/KernelSU"),
            (RootProvider::KernelSUNext, "KernelSU-Next/KernelSU-Next"),
            (RootProvider::SukiSU, "SukiSU-Ultra/SukiSU-Ultra"),
        ];

        let mut report: Vec<(String, String)> = Vec::new();

        for (provider, repo) in providers.iter().copied() {
            let label = format!("{repo} payload k{KVER}");
            let tmp = tempfile::tempdir().expect("tempdir");
            let mut log = Vec::new();
            let result = download_ksu_payload(provider, Some(KVER), None, tmp.path(), &mut log);
            let outcome = match result {
                Ok(()) => {
                    let ko = tmp.path().join("kernelsu.ko");
                    let init = tmp.path().join("init");
                    let ko_n = std::fs::metadata(&ko).map(|m| m.len()).unwrap_or(0);
                    let init_n = std::fs::metadata(&init).map(|m| m.len()).unwrap_or(0);
                    if ko.exists() && ko_n > 0 && init.exists() && init_n > 0 {
                        format!("OK ko={ko_n} init={init_n}")
                    } else {
                        format!(
                            "FAIL: ko_exists={} ko_size={} init_exists={} init_size={}",
                            ko.exists(),
                            ko_n,
                            init.exists(),
                            init_n
                        )
                    }
                }
                Err(e) => format!("FAIL: {e}"),
            };
            eprintln!("[{label}] {outcome}");
            report.push((label, outcome));
        }

        eprintln!("\n=== LKM payload download report ===");
        for (label, outcome) in &report {
            eprintln!("  {label}: {outcome}");
        }
        eprintln!();

        let failures: Vec<&(String, String)> = report
            .iter()
            .filter(|(_, o)| o.starts_with("FAIL"))
            .collect();
        assert!(
            failures.is_empty(),
            "{} provider payloads failed: {:#?}",
            failures.len(),
            failures
        );
    }

    /// Nightly counterpart to `lkm_payload_download_smoke` — exercises
    /// `download_ksu_payload_nightly` so the per-kernel `.ko` artifact
    /// selection + ksuinit extraction get checked against every
    /// provider's actual nightly run, including ReSukiSU which has no
    /// Stable channel and is the only path that's actually used in
    /// production for that fork.
    ///
    ///     cargo test -p ltbox-patch --lib -- --ignored --nocapture lkm_payload_nightly_download_smoke
    #[test]
    #[ignore = "hits GitHub releases + nightly.link; run manually"]
    fn lkm_payload_nightly_download_smoke() {
        const KVER: &str = "6.6";
        let providers: &[(RootProvider, &str)] = &[
            (RootProvider::KernelSU, "tiann/KernelSU"),
            (RootProvider::KernelSUNext, "KernelSU-Next/KernelSU-Next"),
            (RootProvider::SukiSU, "SukiSU-Ultra/SukiSU-Ultra"),
            (RootProvider::ReSukiSU, "ReSukiSU/ReSukiSU"),
        ];

        let mut report: Vec<(String, String)> = Vec::new();

        for (provider, repo) in providers.iter().copied() {
            let label = format!("{repo} nightly payload k{KVER}");
            let tmp = tempfile::tempdir().expect("tempdir");
            let mut log = Vec::new();
            let result = download_ksu_payload_nightly(
                provider,
                Some(KVER),
                None,
                None,
                tmp.path(),
                &mut log,
            );
            let outcome = match result {
                Ok(run_id) => {
                    let ko = tmp.path().join("kernelsu.ko");
                    let init = tmp.path().join("init");
                    let ko_n = std::fs::metadata(&ko).map(|m| m.len()).unwrap_or(0);
                    let init_n = std::fs::metadata(&init).map(|m| m.len()).unwrap_or(0);
                    if ko.exists() && ko_n > 0 && init.exists() && init_n > 0 {
                        format!("OK run={run_id} ko={ko_n} init={init_n}")
                    } else {
                        format!(
                            "FAIL: ko_exists={} ko_size={} init_exists={} init_size={}",
                            ko.exists(),
                            ko_n,
                            init.exists(),
                            init_n
                        )
                    }
                }
                Err(e) => format!("FAIL: {e}"),
            };
            eprintln!("[{label}] {outcome}");
            report.push((label, outcome));
        }

        eprintln!("\n=== LKM nightly payload download report ===");
        for (label, outcome) in &report {
            eprintln!("  {label}: {outcome}");
        }
        eprintln!();

        let failures: Vec<&(String, String)> = report
            .iter()
            .filter(|(_, o)| o.starts_with("FAIL"))
            .collect();
        assert!(
            failures.is_empty(),
            "{} nightly payloads failed: {:#?}",
            failures.len(),
            failures
        );
    }
}
