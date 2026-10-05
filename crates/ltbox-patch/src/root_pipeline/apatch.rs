//! APatch / FolkPatch payload download (Stable + Nightly) and
//! `assets/kpimg` extraction from the staged APK.

use std::path::Path;

use fs_err as fs;

use ltbox_core::downloader::download_to_file;
use ltbox_core::github::GitHubClient;
use ltbox_core::{LtboxError, Result, tr_args};

use super::magisk::fetch_nightly_apk_outer_zip;
use super::{RootProvider, provider_repo, resolve_nightly_run};

/// Pull `assets/kpimg` out of a staged APatch/FolkPatch APK into `work_dir/kpimg`.
fn extract_kpimg_from_apk(
    repo: &str,
    apk_path: &Path,
    work_dir: &Path,
    log: &mut Vec<String>,
) -> Result<()> {
    let kpimg_dst = work_dir.join("kpimg");
    let f = fs::File::open(apk_path)?;
    let mut archive = zip::ZipArchive::new(f)
        .map_err(|e| LtboxError::Patch(format!("{repo}: APK not a zip: {e}")))?;
    let mut entry = archive
        .by_name("assets/kpimg")
        .map_err(|e| LtboxError::Patch(format!("{repo}: APK missing assets/kpimg: {e}")))?;
    let size = crate::zip_util::copy_capped(
        &mut entry,
        &kpimg_dst,
        crate::zip_util::MAX_ENTRY_BYTES,
        "assets/kpimg",
    )?;
    ltbox_core::live!(
        log,
        "[APatch] {}",
        tr_args!(
            "log_apatch_extracted_kpimg",
            path = kpimg_dst.display(),
            bytes = size,
        )
    );
    Ok(())
}

/// Fetch APatch/FolkPatch Stable APK → stash at `work_dir/apatch.apk`,
/// extract `assets/kpimg` → `work_dir/kpimg`.
pub fn download_apatch_payload(
    provider: RootProvider,
    work_dir: &Path,
    log: &mut Vec<String>,
) -> Result<String> {
    download_apatch_release_payload(provider, None, work_dir, log)
}

pub(super) fn download_apatch_release_payload(
    provider: RootProvider,
    release_tag: Option<&str>,
    work_dir: &Path,
    log: &mut Vec<String>,
) -> Result<String> {
    let repo = provider_repo(provider).ok_or_else(|| {
        LtboxError::Patch(format!(
            "download_apatch_payload: unsupported provider {provider:?}"
        ))
    })?;
    let client = GitHubClient::new(repo)?;
    let (tag, assets) = client.selected_release_assets(release_tag)?;
    let (name, url) = assets
        .into_iter()
        .find(|(n, _)| n.to_lowercase().ends_with(".apk"))
        .ok_or_else(|| LtboxError::Download(format!("No release APK on latest {repo}")))?;
    ltbox_core::live!(
        log,
        "[APatch] {}",
        tr_args!(
            "log_release_latest_asset",
            repo = repo,
            tag = tag,
            name = name
        )
    );

    let apk_path = work_dir.join("apatch.apk");
    download_to_file(&url, &apk_path, log)?;
    extract_kpimg_from_apk(repo, &apk_path, work_dir, log)?;
    Ok(tag)
}

/// Fetch APatch/FolkPatch Nightly APK via `nightly.link` → extract kpimg.
/// `manual_run_id = None` → latest successful run on provider's workflow.
pub fn download_apatch_payload_nightly(
    provider: RootProvider,
    manual_run_id: Option<u64>,
    work_dir: &Path,
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
    let artifact_name =
        select_apatch_nightly_artifact(provider, &artifact_names).ok_or_else(|| {
            LtboxError::Patch(format!(
                "{repo} run {run_id}: no matching manager APK artifact"
            ))
        })?;
    ltbox_core::live!(
        log,
        "[APatch] {}",
        tr_args!(
            "log_nightly_artifact",
            repo = repo,
            artifact = artifact_name
        )
    );
    // Canonical apk path so Stable / Nightly share downstream steps.
    let apk_path = work_dir.join("apatch.apk");
    fetch_nightly_apk_outer_zip(
        "APatch",
        repo,
        super::nightly_artifact_id(&artifacts, repo, run_id, artifact_name)?,
        artifact_name,
        "apatch_nightly",
        work_dir,
        &apk_path,
        log,
    )?;
    extract_kpimg_from_apk(repo, &apk_path, work_dir, log)?;
    Ok(run_id)
}

/// Select an APK artifact, never auxiliary output such as R8 mappings.
pub(super) fn select_apatch_nightly_artifact(
    provider: RootProvider,
    names: &[String],
) -> Option<&str> {
    let prefix = match provider {
        RootProvider::APatch => "apatch",
        RootProvider::FolkPatch => "folkpatch",
        _ => return None,
    };
    names
        .iter()
        .filter(|name| {
            let lower = name.to_ascii_lowercase();
            let name = lower.strip_suffix(".zip").unwrap_or(&lower);
            let name = name.strip_suffix(".apk").unwrap_or(name);
            name == prefix
                || name == format!("{prefix}-release")
                || name == format!("{prefix}-debug")
                || name.starts_with(&format!("{prefix}-release-"))
                || name.starts_with(&format!("{prefix}-debug-"))
        })
        .min_by_key(|name| name.to_ascii_lowercase().contains("debug"))
        .map(String::as_str)
}

#[cfg(test)]
mod artifact_tests {
    use super::*;
    #[test]
    fn nightly_manager_ignores_mappings_and_prefers_release() {
        let mappings = ltbox_core::github::WorkflowArtifact {
            id: 1,
            name: "mappings".into(),
            digest: None,
            expired: false,
            created_at: String::new(),
            expires_at: String::new(),
        };
        assert!(!super::super::provider_has_nightly_manager(
            RootProvider::APatch,
            std::slice::from_ref(&mappings)
        ));
        let manager = ltbox_core::github::WorkflowArtifact {
            id: 2,
            name: "APatch-Release".into(),
            ..mappings.clone()
        };
        assert!(super::super::provider_has_nightly_manager(
            RootProvider::APatch,
            &[mappings, manager]
        ));
        let names = ["mappings", "APatch-Debug", "APatch-Release"].map(String::from);
        assert_eq!(
            select_apatch_nightly_artifact(RootProvider::APatch, &names),
            Some("APatch-Release")
        );
        assert_eq!(
            select_apatch_nightly_artifact(RootProvider::APatch, &names[..1]),
            None
        );
        assert_eq!(
            select_apatch_nightly_artifact(RootProvider::FolkPatch, &names),
            None
        );
        let hashed = ["folkpatch-debug-155eb044", "folkpatch-release-155eb044"].map(String::from);
        assert_eq!(
            select_apatch_nightly_artifact(RootProvider::FolkPatch, &hashed),
            Some("folkpatch-release-155eb044")
        );
        let names = ["mappings", "FolkPatch.apk.zip"].map(String::from);
        assert_eq!(
            select_apatch_nightly_artifact(RootProvider::FolkPatch, &names),
            Some("FolkPatch.apk.zip")
        );
    }
}
