//! Magisk-specific download helpers (Stable + Nightly).
//!
//! Also hosts the shared `fetch_nightly_apk_outer_zip` helper that
//! drives APatch's and KSU's nightly outer-zip flow — kept here
//! because Magisk was the original consumer that defined its shape.

use std::path::{Path, PathBuf};

use fs_err as fs;

use ltbox_core::downloader::download_to_file;
use ltbox_core::github::GitHubClient;
use ltbox_core::{LtboxError, Result, tr_args};

use super::apk::{collect_apks_recursive, pick_preferred_apk_path};
use super::{RootProvider, nightly_artifact_url, provider_repo, resolve_nightly_run};

/// Download latest Magisk APK into `dst_path`; returns the tag name.
pub fn download_latest_magisk_apk(
    provider: RootProvider,
    dst_path: &Path,
    log: &mut Vec<String>,
) -> Result<String> {
    download_magisk_release_apk(provider, None, dst_path, log)
}

pub(super) fn download_magisk_release_apk(
    provider: RootProvider,
    release_tag: Option<&str>,
    dst_path: &Path,
    log: &mut Vec<String>,
) -> Result<String> {
    let repo = provider_repo(provider)
        .ok_or_else(|| LtboxError::Patch("Magisk forks need a local APK for patching".into()))?;
    let client = GitHubClient::new(repo)?;
    let (tag, assets) = client.selected_release_assets(release_tag)?;
    let (name, url) = assets
        .into_iter()
        .find(|(n, _)| {
            let lower = n.to_lowercase();
            lower.ends_with(".apk") && !lower.contains("debug")
        })
        .ok_or_else(|| LtboxError::Download(format!("No release APK on latest {repo}")))?;
    ltbox_core::live!(
        log,
        "[Magisk] {}",
        tr_args!("log_release_latest_asset", tag = tag, name = name)
    );
    download_to_file(&url, dst_path, log)?;
    Ok(tag)
}

/// Download outer nightly ZIP → extract → move inner `.apk` onto `dst_apk`.
/// `rename` falls back to `copy` for cross-volume moves under WSL.
#[allow(clippy::too_many_arguments)]
pub(super) fn fetch_nightly_apk_outer_zip(
    log_tag: &str,
    repo: &str,
    run_id: u64,
    artifact_name: &str,
    staging_name: &str,
    work_dir: &Path,
    dst_apk: &Path,
    log: &mut Vec<String>,
) -> Result<()> {
    let outer_zip_path = work_dir.join(format!("{staging_name}.zip"));
    let url = nightly_artifact_url(repo, run_id, artifact_name);
    download_to_file(&url, &outer_zip_path, log)?;

    let staging = work_dir.join(staging_name);
    if staging.exists() {
        fs::remove_dir_all(&staging).ok();
    }
    fs::create_dir_all(&staging)?;
    {
        let f = fs::File::open(&outer_zip_path)?;
        let mut archive = zip::ZipArchive::new(f)
            .map_err(|e| LtboxError::Patch(format!("{repo}: nightly artifact not a zip: {e}")))?;
        // Stage only the `.apk` entries, each streamed under a hard per-entry
        // size cap. `archive.extract` writes every entry with no bound (a
        // decompression-bomb sink) when all we need is the APK the recursive
        // pick below selects. `enclosed_name` rejects zip-slip paths.
        for i in 0..archive.len() {
            let mut entry = archive
                .by_index(i)
                .map_err(|e| LtboxError::Patch(format!("{repo}: read nightly entry {i}: {e}")))?;
            if !entry.is_file() {
                continue;
            }
            let Some(rel) = entry.enclosed_name() else {
                continue;
            };
            let is_apk = rel
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("apk"));
            if !is_apk {
                continue;
            }
            let dst = staging.join(&rel);
            if let Some(parent) = dst.parent() {
                fs::create_dir_all(parent)?;
            }
            crate::zip_util::copy_capped(
                &mut entry,
                &dst,
                crate::zip_util::MAX_ENTRY_BYTES,
                rel.display(),
            )?;
        }
    }

    // Walk the extracted artifact recursively — some providers nest
    // their APK under `<artifact>/manager/`, `<arch>/`, or
    // `app-release-arm64-v8a/`. Old non-recursive `read_dir` skipped
    // those and reported "no .apk found after extract".
    let mut apk_candidates: Vec<PathBuf> = Vec::new();
    collect_apks_recursive(&staging, &mut apk_candidates);
    let apk_src = pick_preferred_apk_path(&apk_candidates)
        .cloned()
        .ok_or_else(|| {
            LtboxError::Patch(format!(
                "{repo} nightly artifact {artifact_name}: no .apk found after extract"
            ))
        })?;

    if dst_apk.exists() {
        fs::remove_file(dst_apk).ok();
    }
    fs::rename(&apk_src, dst_apk).or_else(|_| fs::copy(&apk_src, dst_apk).map(|_| ()))?;
    ltbox_core::live!(
        log,
        "[{log_tag}] {}",
        tr_args!("log_staged_nightly_apk", path = dst_apk.display())
    );
    Ok(())
}

/// Fetch a nightly Magisk APK via `nightly.link`. Prefers `app-release` /
/// `apk-ng-release` artifacts over debug. `manual_run_id = None` →
/// latest successful `ci.yml` run on `master`.
pub fn download_magisk_apk_nightly(
    provider: RootProvider,
    manual_run_id: Option<u64>,
    work_dir: &Path,
    dst_path: &Path,
    log: &mut Vec<String>,
) -> Result<u64> {
    let (repo, run_id) = resolve_nightly_run(provider, manual_run_id, log)?;
    let client = GitHubClient::new(repo)?;
    let artifact_names = client.workflow_artifacts(run_id)?;
    if artifact_names.is_empty() {
        return Err(LtboxError::Patch(format!(
            "{repo} run {run_id} has no artifacts"
        )));
    }
    // Prefer release variants over debug artifacts.
    let preferred: &[&str] = &["app-release", "apk-ng-release"];
    let artifact_name = preferred
        .iter()
        .find_map(|p| {
            artifact_names
                .iter()
                .find(|n| n.to_lowercase().starts_with(p))
                .cloned()
        })
        .or_else(|| {
            artifact_names
                .iter()
                .find(|n| !n.to_lowercase().contains("debug"))
                .cloned()
        })
        .ok_or_else(|| {
            LtboxError::Patch(format!(
                "{repo} run {run_id}: no release APK artifact (got {artifact_names:?})"
            ))
        })?;
    ltbox_core::live!(
        log,
        "[Magisk] {repo} {}",
        tr_args!("log_nightly_artifact", artifact = artifact_name)
    );
    fetch_nightly_apk_outer_zip(
        "Magisk",
        repo,
        run_id,
        &artifact_name,
        "magisk_nightly",
        work_dir,
        dst_path,
        log,
    )?;
    Ok(run_id)
}
