//! Durable, per-operation snapshots. Manifests are for human identification only.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use chrono::Local;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(crate) const MANIFEST_NAME: &str = "manifest.json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BackupFolderEntry {
    pub(crate) path: PathBuf,
    pub(crate) has_manifest: bool,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub(crate) struct BackupManifestInfo {
    #[serde(default)]
    pub(crate) model: Option<String>,
    #[serde(default)]
    pub(crate) fingerprint: Option<String>,
    #[serde(default)]
    pub(crate) recorded_at: Option<String>,
    #[serde(default)]
    pub(crate) slot: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct BackupManifestDialog {
    pub(crate) folder: PathBuf,
    pub(crate) result: Result<BackupManifestInfo, String>,
}

pub(crate) fn root_backup_folders() -> Result<Vec<BackupFolderEntry>, String> {
    root_backup_folders_in(&ltbox_core::app_paths::backup_root())
}

fn root_backup_folders_in(root: &Path) -> Result<Vec<BackupFolderEntry>, String> {
    let root = root.join("root");
    let entries = match std::fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("{}: {error}", root.display())),
    };
    let mut folders = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("{}: {error}", root.display()))?;
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        if !metadata.file_type().is_dir() {
            continue;
        }
        let manifest = path.join(MANIFEST_NAME);
        let has_manifest = std::fs::symlink_metadata(&manifest)
            .map(|metadata| metadata.file_type().is_file())
            .unwrap_or(false);
        folders.push(BackupFolderEntry { path, has_manifest });
    }
    folders.sort_by(|left, right| right.path.file_name().cmp(&left.path.file_name()));
    Ok(folders)
}

pub(crate) fn read_backup_manifest(folder: &Path) -> Result<BackupManifestInfo, String> {
    let path = folder.join(MANIFEST_NAME);
    let file =
        std::fs::File::open(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    serde_json::from_reader(file).map_err(|error| format!("{}: {error}", path.display()))
}

fn path_component(value: &str, fallback: &str) -> String {
    let value = value.trim();
    let sanitized: String = value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty() || sanitized.chars().all(|c| c == '_') {
        fallback.to_string()
    } else {
        sanitized
    }
}

pub(crate) fn create_backup_dir(operation: &str, model: &str) -> Result<PathBuf, String> {
    create_backup_dir_in(&ltbox_core::app_paths::backup_root(), operation, model)
}

pub(crate) fn create_backup_dir_in(
    root: &Path,
    operation: &str,
    model: &str,
) -> Result<PathBuf, String> {
    reserve_backup_dir(
        root,
        operation,
        model,
        &Local::now().format("%Y-%m-%d_%H%M%S").to_string(),
    )
    .map_err(|error| error.to_string())
}

fn reserve_backup_dir(
    root: &Path,
    operation: &str,
    model: &str,
    timestamp: &str,
) -> std::io::Result<PathBuf> {
    let parent = root.join(path_component(operation, "backup"));
    std::fs::create_dir_all(&parent)?;
    let stem = format!("{}_{timestamp}", path_component(model, "unknown_model"));
    // create_dir is exclusive, including when two workers start in the same second.
    for attempt in 0..u32::MAX {
        let name = if attempt == 0 {
            stem.clone()
        } else {
            format!("{stem}_{}", attempt + 1)
        };
        let path = parent.join(name);
        match std::fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::other("backup directory names exhausted"))
}

#[derive(Serialize)]
struct BackupManifest<'a> {
    version: u8,
    operation: &'a str,
    model: Option<&'a str>,
    fingerprint: Option<String>,
    recorded_at: String,
    slot: Option<&'a str>,
    purpose: &'static str,
    files: Vec<BackupFile>,
}

#[derive(Serialize)]
struct BackupFile {
    filename: String,
    size_bytes: u64,
    sha256: String,
    fingerprint: Option<String>,
    /// Image AVB index, not a claim about the device's stored rollback floor.
    rollback_index: Option<u64>,
}

/// Record the saved bytes and any readable AVB metadata. This file is never
/// read to authorize restoration or to check a user-selected firmware image.
pub(crate) fn write_backup_manifest(
    dir: &Path,
    operation: &str,
    model: &str,
    fingerprint: Option<&str>,
    slot: Option<&str>,
) -> Result<(), String> {
    let mut paths = std::fs::read_dir(dir)
        .map_err(|error| error.to_string())?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    paths.sort();
    let mut files = Vec::new();
    for path in paths {
        if !std::fs::symlink_metadata(&path)
            .map_err(|error| error.to_string())?
            .is_file()
            || path.file_name().is_some_and(|name| name == MANIFEST_NAME)
        {
            continue;
        }
        let mut file = std::fs::File::open(&path).map_err(|error| error.to_string())?;
        let mut hash = Sha256::new();
        let mut size_bytes = 0;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let len = file.read(&mut buffer).map_err(|error| error.to_string())?;
            if len == 0 {
                break;
            }
            hash.update(&buffer[..len]);
            size_bytes += len as u64;
        }
        let avb = if path.extension().is_some_and(|extension| extension == "img") {
            ltbox_patch::avb::extract_image_avb_info(&path).ok()
        } else {
            None
        };
        files.push(BackupFile {
            filename: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            size_bytes,
            sha256: hash
                .finalize()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
            fingerprint: avb.as_ref().and_then(ltbox_patch::avb::build_fingerprint),
            rollback_index: avb.as_ref().map(|info| info.rollback_index),
        });
    }
    let manifest = BackupManifest {
        version: 1,
        operation,
        model: (!model.trim().is_empty()).then_some(model),
        fingerprint: fingerprint
            .filter(|fp| !fp.trim().is_empty())
            .map(str::to_owned)
            .or_else(|| files.iter().find_map(|file| file.fingerprint.clone())),
        recorded_at: Local::now().to_rfc3339(),
        slot,
        purpose: "Manual identification only; not used for restore validation. Images may already be modified.",
        files,
    };
    let mut bytes = serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    // Each caller owns a freshly reserved directory. Never replace a pre-existing manifest.
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dir.join(MANIFEST_NAME))
        .map_err(|error| error.to_string())?;
    output.write_all(&bytes).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_second_runs_preserve_previous_bytes_and_isolate_models() {
        let temp = tempfile::tempdir().unwrap();
        let first =
            reserve_backup_dir(temp.path(), "root", "TB322FC", "2026-09-06_170817").unwrap();
        std::fs::write(first.join("boot.img"), b"first").unwrap();
        let second =
            reserve_backup_dir(temp.path(), "root", "TB322FC", "2026-09-06_170817").unwrap();
        let other =
            reserve_backup_dir(temp.path(), "root", "TB323FU", "2026-09-06_170817").unwrap();
        assert_eq!(first, temp.path().join("root/TB322FC_2026-09-06_170817"));
        assert_ne!(first, second);
        assert_ne!(first, other);
        assert_eq!(std::fs::read(first.join("boot.img")).unwrap(), b"first");
        assert!(second.read_dir().unwrap().next().is_none());
    }

    #[test]
    fn model_names_cannot_escape_backup_operation() {
        let temp = tempfile::tempdir().unwrap();
        let dir =
            reserve_backup_dir(temp.path(), "root", "../../TB:322FC", "2026-09-06_170817").unwrap();
        assert_eq!(dir.parent(), Some(temp.path().join("root").as_path()));
        assert_eq!(path_component("", "unknown_model"), "unknown_model");
    }

    #[test]
    fn concurrent_backups_reserve_distinct_directories() {
        let temp = tempfile::tempdir().unwrap();
        let paths = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        reserve_backup_dir(temp.path(), "root", "TB322FC", "2026-09-06_170817")
                            .unwrap()
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect::<std::collections::BTreeSet<_>>()
        });
        assert_eq!(paths.len(), 8);
    }

    #[test]
    fn manifest_reads_fingerprint_and_rollback_from_saved_image() {
        use avbtool_rs::builder::{PropertySpec, VbmetaImageArgs, make_vbmeta_image};
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("boot.img");
        let fingerprint = "Lenovo/TB322FC/TB322FC:15/example:user/release-keys";
        make_vbmeta_image(
            &path,
            &VbmetaImageArgs {
                algorithm_name: "NONE".into(),
                key_spec: None,
                public_key_metadata: None,
                rollback_index: 123456,
                flags: 0,
                rollback_index_location: 3,
                properties: vec![PropertySpec {
                    key: "com.android.build.boot.fingerprint".into(),
                    value: fingerprint.as_bytes().to_vec(),
                }],
                kernel_cmdlines: vec![],
                extra_descriptors: vec![],
                include_descriptors_from_images: vec![],
                chain_partitions: vec![],
                release_string: None,
                append_to_release_string: None,
                padding_size: 4096,
            },
        )
        .unwrap();
        let before = std::fs::read(&path).unwrap();
        write_backup_manifest(temp.path(), "root", "TB322FC", None, Some("_b")).unwrap();
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(temp.path().join(MANIFEST_NAME)).unwrap())
                .unwrap();
        assert_eq!(manifest["fingerprint"], fingerprint);
        assert_eq!(manifest["files"][0]["fingerprint"], fingerprint);
        assert_eq!(manifest["files"][0]["rollback_index"], 123456);
        assert_eq!(manifest["slot"], "_b");
        assert_eq!(std::fs::read(path).unwrap(), before);
    }

    #[test]
    fn manifest_records_hashes_and_unknown_metadata_without_rejecting_images() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("boot.img"), b"abc").unwrap();
        write_backup_manifest(temp.path(), "root", "TB322FC", None, Some("_a")).unwrap();
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(temp.path().join(MANIFEST_NAME)).unwrap())
                .unwrap();
        assert_eq!(manifest["model"], "TB322FC");
        assert!(manifest["fingerprint"].is_null());
        assert_eq!(
            manifest["files"][0]["sha256"],
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(manifest["files"][0]["rollback_index"].is_null());
        assert_eq!(std::fs::read(temp.path().join("boot.img")).unwrap(), b"abc");
    }

    #[test]
    fn root_backup_list_is_newest_first_and_marks_real_manifests() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        std::fs::create_dir_all(root.join("TB322FC_2026-09-10_080000")).unwrap();
        std::fs::create_dir_all(root.join("TB322FC_2026-09-12_090000")).unwrap();
        std::fs::write(root.join("TB322FC_2026-09-12_090000/manifest.json"), b"{}").unwrap();
        std::fs::write(root.join("not-a-folder"), b"ignored").unwrap();

        let entries = root_backup_folders_in(temp.path()).unwrap();

        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries[0].path.file_name().unwrap(),
            "TB322FC_2026-09-12_090000"
        );
        assert!(entries[0].has_manifest);
        assert!(!entries[1].has_manifest);
    }

    #[test]
    fn missing_root_backup_directory_is_an_empty_list() {
        let temp = tempfile::tempdir().unwrap();
        assert!(root_backup_folders_in(temp.path()).unwrap().is_empty());
    }

    #[test]
    fn manifest_reader_accepts_the_identification_fields_without_file_details() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join(MANIFEST_NAME),
            br#"{
                "version": 1,
                "operation": "root",
                "model": "TB322FC",
                "fingerprint": "Lenovo/TB322FC/example",
                "recorded_at": "2026-09-12T09:30:00+09:00",
                "slot": "_b",
                "files": []
            }"#,
        )
        .unwrap();

        let info = read_backup_manifest(temp.path()).unwrap();

        assert_eq!(info.model.as_deref(), Some("TB322FC"));
        assert_eq!(info.fingerprint.as_deref(), Some("Lenovo/TB322FC/example"));
        assert_eq!(
            info.recorded_at.as_deref(),
            Some("2026-09-12T09:30:00+09:00")
        );
        assert_eq!(info.slot.as_deref(), Some("_b"));
    }
}
