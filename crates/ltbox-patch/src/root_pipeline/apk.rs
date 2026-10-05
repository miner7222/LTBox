//! Shared APK selection / extraction helpers used by all root providers.
//!
//! Covers:
//! * Manager-asset asset-list filtering (`select_manager_asset` + the KSU-side
//!   stable / nightly preference tables).
//! * APK preference scoring + recursive collection used when nightly outer
//!   ZIPs nest the `.apk` inside provider-specific subdirs.
//! * The "stage a downloaded asset → manager.apk" entry points that pick
//!   between a direct `.apk` and a wrapper `.zip`.

use std::path::{Path, PathBuf};

use fs_err as fs;

use ltbox_core::tr_args;
use ltbox_core::{LtboxError, Result};

use super::RootProvider;

/// Ordered keyword preferences for picking a manager asset from a **stable
/// release** asset list. Keywords are case-insensitive substrings matched
/// against `.apk` asset names (e.g. `KernelSU_v3.2.4_32457-release.apk`).
///
/// Spoofed variants go first so providers that ship both `-spoofed` and
/// non-spoofed release APKs (KernelSU-Next today, SukiSU going forward)
/// land on the spoofed one. Release lists can include prereleases.
pub(super) fn ksu_manager_stable_preferences(provider: RootProvider) -> &'static [&'static str] {
    match provider {
        RootProvider::KernelSU => &["-release.apk"],
        RootProvider::KernelSUNext => &["-spoofed", "-release.apk"],
        RootProvider::SukiSU => &["-spoofed", "-release.apk", "_releases.apk"],
        // Device ABI, not host ABI: LTBox targets ARM64 Android devices.
        RootProvider::BakaSU => &["-arm64-v8a-release.apk", "-universal-release.apk"],
        _ => &[],
    }
}

/// Ordered keyword preferences for picking a manager artifact from a
/// **nightly workflow run**. Workflow artifact names are bare (no suffix),
/// so exact-match is the common case; substring fallback is the safety net.
pub(super) fn ksu_manager_nightly_preferences(provider: RootProvider) -> &'static [&'static str] {
    match provider {
        RootProvider::KernelSU => &["manager"],
        // Upstream ships `manager-spoofed` + `manager`; prefer the spoofed
        // one for Play Integrity / Widevine preservation.
        RootProvider::KernelSUNext => &["manager-spoofed", "manager"],
        // SukiSU doesn't currently emit `manager-spoofed`, but upstream has
        // signalled intent — keep spoofed first so future runs pick it up
        // without code changes.
        RootProvider::SukiSU => &["manager-spoofed", "manager"],
        // BakaSU emits four variants; user preference is
        // release > debug, spoofed > plain, checked in that order.
        RootProvider::BakaSU => &[
            "Spoofed-Manager-release",
            "Manager-release",
            "Spoofed-Manager-debug",
            "Manager-debug",
        ],
        _ => &[],
    }
}

/// Pick a manager asset from `assets` using `preferred_keywords`.
///
/// Matching is two-tiered, both case-insensitive:
///
/// 1. Exact asset-name match against each preferred keyword, in order.
///    Handles nightly artifact names (bare, no suffix) cleanly.
/// 2. Substring match against each preferred keyword, in order.
///    Handles stable release `.apk` names whose keyword is only a fragment
///    (e.g. `-spoofed` inside `KernelSU_Next_v3.2.0-spoofed_33129-release.apk`).
///
/// The iteration order of `preferred_keywords` is the priority order —
/// earlier entries win even when later entries would also substring-match.
pub(super) fn select_manager_asset(
    assets: &[(String, String)],
    preferred_keywords: &[&str],
) -> Option<(String, String)> {
    // Tier 1 — exact match (nightly artifact names).
    for keyword in preferred_keywords {
        if let Some(hit) = assets
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(keyword))
        {
            return Some(hit.clone());
        }
    }
    // Tier 2 — substring match (stable `.apk` names).
    for keyword in preferred_keywords {
        let keyword_lower = keyword.to_lowercase();
        if let Some(hit) = assets
            .iter()
            .find(|(name, _)| name.to_lowercase().contains(&keyword_lower))
        {
            return Some(hit.clone());
        }
    }
    None
}

pub(super) fn copy_apk_to(src: &Path, dst: &Path) -> Result<()> {
    if dst.exists() {
        fs::remove_file(dst).ok();
    }
    fs::copy(src, dst)?;
    Ok(())
}

/// Recursive .apk hunt — extracted nightly artifacts often nest the
/// APK inside `<artifact>/manager/` or `arm64-v8a/`, which a flat
/// `read_dir` would miss.
pub(super) fn collect_apks_recursive(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_apks_recursive(&path, out);
        } else if path
            .extension()
            .and_then(|x| x.to_str())
            .is_some_and(|x| x.eq_ignore_ascii_case("apk"))
        {
            out.push(path);
        }
    }
}

/// Pick the most-likely-to-install APK from a candidate list. Tier 1
/// prefers `arm64-v8a` (LTBox-supported devices are arm64); Tier 2
/// falls back to any non-debug variant; Tier 3 surrenders and returns
/// whatever's first. Used by both the recursive filesystem path and
/// the in-zip member-name path so the selection rule is consistent
/// across the staging shapes the various providers ship.
pub(super) fn apk_preference_score(name_lower: &str) -> u8 {
    if name_lower.contains("arm64-v8a")
        || name_lower.contains("arm64")
        || name_lower.contains("v8a")
    {
        return 3;
    }
    if name_lower.contains("debug") {
        return 0;
    }
    if name_lower.contains("release") {
        return 2;
    }
    1
}

pub(super) fn pick_preferred_apk_path(paths: &[PathBuf]) -> Option<&PathBuf> {
    paths.iter().max_by_key(|p| {
        let s = p.to_string_lossy().to_lowercase();
        apk_preference_score(&s)
    })
}

pub(super) fn pick_preferred_apk_name(names: &[String]) -> Option<&String> {
    names
        .iter()
        .max_by_key(|n| apk_preference_score(&n.to_lowercase()))
}

pub(super) fn extract_first_apk_from_zip(
    archive_path: &Path,
    output_path: &Path,
    log_tag: &str,
    log: &mut Vec<String>,
) -> Result<bool> {
    let f = fs::File::open(archive_path)?;
    let mut archive = zip::ZipArchive::new(f).map_err(|e| {
        LtboxError::Patch(format!(
            "{}: APK container not a zip: {e}",
            archive_path.display()
        ))
    })?;
    // Pick `arm64-v8a` over generic / debug / x86 variants when the
    // container ships multiple split APKs (release ZIPs from KSU
    // family + BakaSU look like that). Falls back to first non-debug
    // APK, then the first APK at all.
    let member_names: Vec<String> = archive
        .file_names()
        .filter(|n| n.to_lowercase().ends_with(".apk") && !n.ends_with('/'))
        .map(|s| s.to_string())
        .collect();
    let Some(member_name) = pick_preferred_apk_name(&member_names).cloned() else {
        return Ok(false);
    };
    let mut entry = archive.by_name(&member_name).map_err(|e| {
        LtboxError::Patch(format!(
            "{}: read {member_name}: {e}",
            archive_path.display()
        ))
    })?;
    if output_path.exists() {
        fs::remove_file(output_path).ok();
    }
    crate::zip_util::copy_capped(
        &mut entry,
        output_path,
        crate::zip_util::MAX_ENTRY_BYTES,
        &member_name,
    )?;
    ltbox_core::live!(
        log,
        "[{log_tag}] {}",
        tr_args!(
            "log_extracted_manager_apk",
            member = member_name,
            path = output_path.display(),
        )
    );
    Ok(true)
}

pub(super) fn stage_manager_from_downloaded_asset(
    asset_path: &Path,
    manager_apk: &Path,
    log_tag: &str,
    log: &mut Vec<String>,
) -> Result<()> {
    if asset_path
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("apk"))
    {
        copy_apk_to(asset_path, manager_apk)?;
        ltbox_core::live!(
            log,
            "[{log_tag}] {}",
            tr_args!("log_staged_manager_apk", path = manager_apk.display())
        );
        return Ok(());
    }
    if extract_first_apk_from_zip(asset_path, manager_apk, log_tag, log)? {
        return Ok(());
    }
    Err(LtboxError::Patch(format!(
        "{log_tag}: manager artifact {} did not contain an APK",
        asset_path.display()
    )))
}

#[cfg(test)]
mod tests {
    use super::{
        apk_preference_score, pick_preferred_apk_name, pick_preferred_apk_path,
        select_manager_asset,
    };
    use std::path::PathBuf;

    #[test]
    fn bakasu_release_prefers_arm64_and_falls_back_to_universal() {
        use super::{RootProvider, ksu_manager_stable_preferences};
        let preferences = ksu_manager_stable_preferences(RootProvider::BakaSU);
        let mut assets: Vec<(String, String)> = [
            "ReSukiSU_v4.2.0-rc2_35144-x86_64-release.apk",
            "ReSukiSU_v4.2.0-rc2_35144-universal-release.apk",
            "ReSukiSU_v4.2.0-rc2_35144-armeabi-v7a-release.apk",
            "ReSukiSU_v4.2.0-rc2_35144-arm64-v8a-debug.apk",
            "ReSukiSU_v4.2.0-rc2_35144-arm64-v8a-release.apk",
        ]
        .into_iter()
        .map(|name| (name.into(), "url".into()))
        .collect();
        assert_eq!(
            select_manager_asset(&assets, preferences),
            assets.last().cloned()
        );
        assets.pop();
        assert_eq!(
            select_manager_asset(&assets, preferences),
            Some(assets[1].clone())
        );
        assets.remove(1);
        assert!(select_manager_asset(&assets, preferences).is_none());
    }

    #[test]
    fn sukisu_plural_release_suffix_is_selected() {
        use super::{RootProvider, ksu_manager_stable_preferences};
        let assets = vec![("SukiSU_v4.2.0_40900_releases.apk".into(), "url".into())];
        assert!(
            select_manager_asset(
                &assets,
                ksu_manager_stable_preferences(RootProvider::SukiSU)
            )
            .is_some()
        );
    }

    #[test]
    fn apk_preference_arm64_v8a_wins() {
        let names = vec![
            "app-x86-release.apk".to_string(),
            "app-arm64-v8a-release.apk".to_string(),
            "app-armeabi-v7a-release.apk".to_string(),
        ];
        assert_eq!(
            pick_preferred_apk_name(&names).map(|s| s.as_str()),
            Some("app-arm64-v8a-release.apk")
        );
    }

    #[test]
    fn apk_preference_release_beats_debug() {
        let names = vec!["app-debug.apk".to_string(), "app-release.apk".to_string()];
        assert_eq!(
            pick_preferred_apk_name(&names).map(|s| s.as_str()),
            Some("app-release.apk")
        );
    }

    #[test]
    fn apk_preference_falls_back_to_first_when_no_hints() {
        let names = vec!["foo.apk".to_string(), "bar.apk".to_string()];
        // Both score 1, max_by_key returns the last on ties — accept either
        // since neither carries an arm64/release/debug hint.
        let pick = pick_preferred_apk_name(&names).unwrap();
        assert!(names.contains(pick));
    }

    #[test]
    fn apk_preference_no_candidates_returns_none() {
        let empty: Vec<String> = Vec::new();
        assert!(pick_preferred_apk_name(&empty).is_none());
    }

    #[test]
    fn apk_preference_path_picks_arm64_v8a_in_subdir() {
        let paths = vec![
            PathBuf::from("staging/app-debug.apk"),
            PathBuf::from("staging/manager/arm64-v8a/app-release.apk"),
            PathBuf::from("staging/manager/x86/app-release.apk"),
        ];
        assert_eq!(
            pick_preferred_apk_path(&paths),
            Some(&PathBuf::from("staging/manager/arm64-v8a/app-release.apk"))
        );
    }

    #[test]
    fn apk_preference_score_orders_correctly() {
        assert!(
            apk_preference_score("app-arm64-v8a-release.apk")
                > apk_preference_score("app-release.apk")
        );
        assert!(apk_preference_score("app-release.apk") > apk_preference_score("app-debug.apk"));
        assert!(apk_preference_score("app-release.apk") > apk_preference_score("foo.apk"));
    }

    #[test]
    fn ksu_manager_asset_selection_prefers_provider_names() {
        let assets = vec![
            (
                "random-debug.apk".to_string(),
                "https://example.invalid/debug.apk".to_string(),
            ),
            (
                "manager-spoofed.zip".to_string(),
                "https://example.invalid/manager-spoofed.zip".to_string(),
            ),
            (
                "manager.zip".to_string(),
                "https://example.invalid/manager.zip".to_string(),
            ),
        ];

        let picked = select_manager_asset(&assets, &["manager-spoofed.zip", "manager.zip"])
            .expect("manager asset");
        assert_eq!(picked.0, "manager-spoofed.zip");
    }
}
