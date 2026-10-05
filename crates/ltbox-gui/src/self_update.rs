//! Verified, rollback-capable updates for directly downloaded LTBox builds.
//!
//! The downloaded archive is kept entirely in a temporary staging directory
//! until both its published SHA-256 and its complete archive layout have been
//! validated. Only then is a same-filesystem replacement prepared beside the
//! installed program and passed to the small, injected swap state machine.

use ltbox_core::crypto::sha256_file_hex;
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) const POST_UPDATE_RELAUNCH_ARG: &str = "--post-update-relaunch";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DirectUpdateState {
    Ready,
    Updating,
    Failed(SelfUpdateFailure),
    Restarting,
}

impl DirectUpdateState {
    pub(crate) const fn is_active(&self) -> bool {
        matches!(self, Self::Updating | Self::Restarting)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SelfUpdateFailureKind {
    NoMatchingBuild,
    InstallLocation,
    NotWritable,
    Download,
    HashMismatch,
    Extract,
    ArchiveLayout,
    Swap,
    Restart,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SelfUpdateFailure {
    pub(crate) kind: SelfUpdateFailureKind,
    pub(crate) detail: String,
}

impl SelfUpdateFailure {
    fn new(kind: SelfUpdateFailureKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArchiveKind {
    Zip,
    TarGz,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReleaseAsset {
    archive_name: String,
    checksum_name: String,
    archive_kind: ArchiveKind,
    payload_kind: PayloadKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PayloadKind {
    File(&'static str),
    MacBundle,
}

/// Map compile-time target facts to the one release asset LTBox publishes.
fn release_asset_for_target(tag: &str, os: &str, arch: &str) -> Option<ReleaseAsset> {
    let (name, archive_kind, payload_kind) = match (os, arch) {
        ("windows", "x86_64") => (
            format!("LTBox-win_x86_64-{tag}.zip"),
            ArchiveKind::Zip,
            PayloadKind::File("ltbox.exe"),
        ),
        ("windows", "aarch64") => (
            format!("LTBox-win_arm64-{tag}.zip"),
            ArchiveKind::Zip,
            PayloadKind::File("ltbox.exe"),
        ),
        ("linux", "x86_64") => (
            format!("LTBox-linux_x86_64-{tag}.tar.gz"),
            ArchiveKind::TarGz,
            PayloadKind::File("ltbox"),
        ),
        ("linux", "aarch64") => (
            format!("LTBox-linux_arm64-{tag}.tar.gz"),
            ArchiveKind::TarGz,
            PayloadKind::File("ltbox"),
        ),
        ("macos", "x86_64" | "aarch64") => (
            format!("LTBox-macos_universal-{tag}.tar.gz"),
            ArchiveKind::TarGz,
            PayloadKind::MacBundle,
        ),
        _ => return None,
    };
    Some(ReleaseAsset {
        checksum_name: format!("{name}.sha256"),
        archive_name: name,
        archive_kind,
        payload_kind,
    })
}

/// Parse the first SHA-256 token used by GNU `sha256sum`/macOS `shasum`.
fn parse_sha256_sidecar(contents: &str) -> Result<String, &'static str> {
    let hash = contents
        .split_whitespace()
        .next()
        .ok_or("checksum file is empty")?;
    if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("checksum file does not start with a SHA-256 digest");
    }
    Ok(hash.to_ascii_lowercase())
}

fn safe_archive_path(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn extract_zip(archive_path: &Path, output: &Path) -> Result<Vec<PathBuf>, String> {
    let file = File::open(archive_path).map_err(|error| error.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|error| error.to_string())?;
    let mut entries = Vec::with_capacity(archive.len());

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("archive entry {index}: {error}"))?;
        let relative = entry
            .enclosed_name()
            .filter(|path| safe_archive_path(path))
            .ok_or_else(|| format!("archive entry {index} has an unsafe path"))?;
        if entry.is_symlink() {
            return Err(format!(
                "archive entry {} is a symbolic link",
                relative.display()
            ));
        }
        let destination = output.join(&relative);
        entries.push(relative);
        if entry.is_dir() {
            fs::create_dir_all(&destination).map_err(|error| error.to_string())?;
            continue;
        }
        let parent = destination
            .parent()
            .ok_or_else(|| "archive entry has no parent".to_string())?;
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        let mut out = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)
            .map_err(|error| error.to_string())?;
        io::copy(&mut entry, &mut out).map_err(|error| error.to_string())?;
        out.flush().map_err(|error| error.to_string())?;
        out.sync_all().map_err(|error| error.to_string())?;

        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&destination, fs::Permissions::from_mode(mode & 0o7777))
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(entries)
}

fn extract_tar_gz(archive_path: &Path, output: &Path) -> Result<Vec<PathBuf>, String> {
    let file = File::open(archive_path).map_err(|error| error.to_string())?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    let archive_entries = archive.entries().map_err(|error| error.to_string())?;
    let mut paths = Vec::new();

    for (index, entry) in archive_entries.enumerate() {
        let mut entry = entry.map_err(|error| format!("archive entry {index}: {error}"))?;
        let entry_type = entry.header().entry_type();
        if !entry_type.is_dir() && !entry_type.is_file() {
            return Err(format!("archive entry {index} is not a file or directory"));
        }
        let relative = entry
            .path()
            .map_err(|error| format!("archive entry {index}: {error}"))?
            .into_owned();
        if !safe_archive_path(&relative) {
            return Err(format!("archive entry {index} has an unsafe path"));
        }
        let unpacked = entry
            .unpack_in(output)
            .map_err(|error| format!("archive entry {index}: {error}"))?;
        if !unpacked {
            return Err(format!(
                "archive entry {index} escaped the staging directory"
            ));
        }
        paths.push(relative);
    }
    Ok(paths)
}

/// Resolve only the release layouts we publish; no recursive "best match" is
/// allowed because selecting an unexpected executable is worse than aborting.
fn resolve_archive_payload(entries: &[PathBuf], kind: PayloadKind) -> Result<PathBuf, ()> {
    let mut matches = BTreeSet::new();
    for path in entries {
        let components: Vec<&OsStr> = path
            .components()
            .filter_map(|component| match component {
                Component::Normal(value) => Some(value),
                _ => None,
            })
            .collect();
        match kind {
            PayloadKind::File(file_name)
                if components.len() == 2
                    && components[0]
                        .to_str()
                        .is_some_and(|name| name.starts_with("LTBox-"))
                    && components[1] == OsStr::new(file_name) =>
            {
                matches.insert(path.clone());
            }
            PayloadKind::MacBundle => {
                let direct = components.len() == 4
                    && components[0] == OsStr::new("LTBox.app")
                    && components[1] == OsStr::new("Contents")
                    && components[2] == OsStr::new("MacOS")
                    && components[3] == OsStr::new("ltbox");
                let nested = components.len() == 5
                    && components[0]
                        .to_str()
                        .is_some_and(|name| name.starts_with("LTBox-"))
                    && components[1] == OsStr::new("LTBox.app")
                    && components[2] == OsStr::new("Contents")
                    && components[3] == OsStr::new("MacOS")
                    && components[4] == OsStr::new("ltbox");
                if direct {
                    matches.insert(PathBuf::from("LTBox.app"));
                } else if nested {
                    matches.insert(PathBuf::from(components[0]).join("LTBox.app"));
                }
            }
            _ => {}
        }
    }
    if matches.len() == 1 {
        Ok(matches.pop_first().expect("one payload match"))
    } else {
        Err(())
    }
}

#[derive(Debug)]
struct WorkDir(tempfile::TempDir);

impl WorkDir {
    /// Exclusive, randomly named scratch directory (0700 on Unix) removed on
    /// drop.
    fn create() -> io::Result<Self> {
        let mut builder = tempfile::Builder::new();
        builder.prefix("ltbox-update-");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            builder.permissions(fs::Permissions::from_mode(0o700));
        }
        builder.tempdir().map(Self)
    }

    fn path(&self) -> &Path {
        self.0.path()
    }
}

/// Unique name for a sibling path that is only reserved here and created
/// later with `create_new`/`rename`, so it cannot be a held `tempfile` guard.
fn unique_token() -> String {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("{}-{nanos}-{sequence}", std::process::id())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SwapKind {
    AtomicFile,
    MoveAside,
}

#[derive(Debug)]
struct InstallTarget {
    current: PathBuf,
    install_dir: PathBuf,
    relaunch: PathBuf,
    swap_kind: SwapKind,
}

impl InstallTarget {
    fn resolve(current_exe: PathBuf) -> Result<Self, SelfUpdateFailure> {
        #[cfg(target_os = "macos")]
        {
            let bundle = current_exe
                .ancestors()
                .find(|path| path.extension() == Some(OsStr::new("app")))
                .map(Path::to_path_buf)
                .ok_or_else(|| {
                    SelfUpdateFailure::new(
                        SelfUpdateFailureKind::InstallLocation,
                        "the running executable is not inside an app bundle",
                    )
                })?;
            let install_dir = bundle.parent().map(Path::to_path_buf).ok_or_else(|| {
                SelfUpdateFailure::new(
                    SelfUpdateFailureKind::InstallLocation,
                    "the app bundle has no parent directory",
                )
            })?;
            Ok(Self {
                current: bundle,
                install_dir,
                relaunch: current_exe,
                swap_kind: SwapKind::MoveAside,
            })
        }

        #[cfg(not(target_os = "macos"))]
        {
            let install_dir = current_exe.parent().map(Path::to_path_buf).ok_or_else(|| {
                SelfUpdateFailure::new(
                    SelfUpdateFailureKind::InstallLocation,
                    "the running executable has no parent directory",
                )
            })?;
            Ok(Self {
                current: current_exe.clone(),
                install_dir,
                relaunch: current_exe,
                swap_kind: if cfg!(windows) {
                    SwapKind::MoveAside
                } else {
                    SwapKind::AtomicFile
                },
            })
        }
    }
}

fn ensure_install_dir_writable(install_dir: &Path) -> io::Result<()> {
    let mut probe = tempfile::Builder::new()
        .prefix(".ltbox-write-test-")
        .tempfile_in(install_dir)?;
    probe.write_all(b"LTBox update write test")?;
    probe.as_file().sync_all()?;
    // `close` reports a failed removal, which proves delete permission too.
    probe.close()
}

fn sibling_artifact_path(current: &Path, marker: &str) -> io::Result<PathBuf> {
    let parent = current
        .parent()
        .ok_or_else(|| io::Error::other("installed program has no parent directory"))?;
    let file_name = current
        .file_name()
        .ok_or_else(|| io::Error::other("installed program has no file name"))?
        .to_string_lossy();
    for _ in 0..32 {
        let path = parent.join(format!(".{file_name}.{marker}-{}", unique_token()));
        if !path.exists() {
            return Ok(path);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not create a unique update sibling",
    ))
}

fn copy_file_new(source: &Path, destination: &Path) -> io::Result<()> {
    let result = (|| {
        let mut input = File::open(source)?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)?;
        io::copy(&mut input, &mut output)?;
        output.flush()?;
        output.sync_all()?;
        fs::set_permissions(destination, fs::metadata(source)?.permissions())?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(destination);
    }
    result
}

fn copy_tree_new(source: &Path, destination: &Path) -> io::Result<()> {
    fn copy_contents(source: &Path, destination: &Path) -> io::Result<()> {
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            let source_path = entry.path();
            let destination_path = destination.join(entry.file_name());
            let metadata = fs::symlink_metadata(&source_path)?;
            if metadata.file_type().is_symlink() {
                return Err(io::Error::other(
                    "symbolic links are not allowed in an update",
                ));
            }
            if metadata.is_dir() {
                fs::create_dir(&destination_path)?;
                copy_contents(&source_path, &destination_path)?;
                fs::set_permissions(&destination_path, metadata.permissions())?;
            } else if metadata.is_file() {
                copy_file_new(&source_path, &destination_path)?;
            } else {
                return Err(io::Error::other("unsupported file in update bundle"));
            }
        }
        Ok(())
    }

    fs::create_dir(destination)?;
    let result = copy_contents(source, destination)
        .and_then(|()| fs::set_permissions(destination, fs::metadata(source)?.permissions()));
    if result.is_err() {
        let _ = fs::remove_dir_all(destination);
    }
    result
}

fn copy_unit_new(source: &Path, destination: &Path) -> io::Result<()> {
    if source.is_dir() {
        copy_tree_new(source, destination)
    } else {
        copy_file_new(source, destination)
    }
}

fn remove_unit(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

trait SwapOps {
    fn rename(&mut self, from: &Path, to: &Path) -> io::Result<()>;
    fn copy_unit(&mut self, from: &Path, to: &Path) -> io::Result<()>;
    fn remove_unit(&mut self, path: &Path) -> io::Result<()>;
}

struct RealSwapOps;

impl SwapOps for RealSwapOps {
    fn rename(&mut self, from: &Path, to: &Path) -> io::Result<()> {
        fs::rename(from, to)
    }

    fn copy_unit(&mut self, from: &Path, to: &Path) -> io::Result<()> {
        copy_unit_new(from, to)
    }

    fn remove_unit(&mut self, path: &Path) -> io::Result<()> {
        remove_unit(path)
    }
}

#[derive(Debug)]
struct SwapReceipt {
    kind: SwapKind,
    current: PathBuf,
    backup: PathBuf,
    failed_replacement: PathBuf,
}

fn swap_with_ops(
    ops: &mut impl SwapOps,
    kind: SwapKind,
    current: &Path,
    candidate: &Path,
    backup: &Path,
    failed_replacement: &Path,
) -> Result<SwapReceipt, String> {
    if kind == SwapKind::AtomicFile {
        if let Err(error) = ops.rename(candidate, current) {
            let _ = ops.remove_unit(backup);
            return Err(format!("could not replace the installed program: {error}"));
        }
        return Ok(SwapReceipt {
            kind,
            current: current.to_path_buf(),
            backup: backup.to_path_buf(),
            failed_replacement: failed_replacement.to_path_buf(),
        });
    }

    ops.rename(current, backup)
        .map_err(|error| format!("could not preserve the installed program: {error}"))?;
    if let Err(place_error) = ops.rename(candidate, current) {
        match ops.rename(backup, current) {
            Ok(()) => {
                return Err(format!(
                    "could not place the update; the installed version was restored: {place_error}"
                ));
            }
            Err(restore_error) => {
                // A rename can fail for a transient filesystem reason. Copying
                // the preserved original gives restoration a second primitive
                // before the verified candidate is used as the final fallback.
                if ops.copy_unit(backup, current).is_ok() {
                    return Err(format!(
                        "could not place the update; the installed version was copied back after restore failed: {place_error}; {restore_error}"
                    ));
                }
                if ops.rename(candidate, current).is_ok() {
                    return Err(format!(
                        "could not restore the installed version, so the verified replacement was retained: {place_error}; {restore_error}"
                    ));
                }
                return Err(format!(
                    "replacement and restoration both failed; the installed version remains at {}: {place_error}; {restore_error}",
                    backup.display()
                ));
            }
        }
    }

    Ok(SwapReceipt {
        kind,
        current: current.to_path_buf(),
        backup: backup.to_path_buf(),
        failed_replacement: failed_replacement.to_path_buf(),
    })
}

fn rollback_with_ops(ops: &mut impl SwapOps, receipt: &SwapReceipt) -> Result<(), String> {
    if receipt.kind == SwapKind::AtomicFile {
        return ops
            .rename(&receipt.backup, &receipt.current)
            .map_err(|error| {
                format!("could not restore the installed program; the replacement remains: {error}")
            });
    }

    if let Err(error) = ops.rename(&receipt.current, &receipt.failed_replacement) {
        return Err(format!(
            "could not move the replacement aside; the replacement remains installed: {error}"
        ));
    }
    match ops.rename(&receipt.backup, &receipt.current) {
        Ok(()) => {
            let _ = ops.remove_unit(&receipt.failed_replacement);
            Ok(())
        }
        Err(restore_error) => {
            if ops.copy_unit(&receipt.backup, &receipt.current).is_ok() {
                let _ = ops.remove_unit(&receipt.failed_replacement);
                return Ok(());
            }
            if ops
                .rename(&receipt.failed_replacement, &receipt.current)
                .is_ok()
            {
                return Err(format!(
                    "could not restore the installed program; the replacement remains: {restore_error}"
                ));
            }
            Err(format!(
                "could not restore either runnable program; the installed version remains at {}: {restore_error}",
                receipt.backup.display()
            ))
        }
    }
}

fn prepare_candidate_and_backup(
    payload: &Path,
    target: &InstallTarget,
) -> io::Result<(PathBuf, PathBuf, PathBuf)> {
    let candidate = sibling_artifact_path(&target.current, "ltbox-new")?;
    let backup = sibling_artifact_path(&target.current, "ltbox-old")?;
    let failed_replacement = sibling_artifact_path(&target.current, "ltbox-failed")?;
    copy_unit_new(payload, &candidate)?;
    if target.swap_kind == SwapKind::AtomicFile
        && let Err(error) = copy_unit_new(&target.current, &backup)
    {
        let _ = remove_unit(&candidate);
        return Err(error);
    }
    Ok((candidate, backup, failed_replacement))
}

fn cleanup_downloads(archive: &Path, checksum: &Path) {
    let _ = fs::remove_file(archive);
    let _ = fs::remove_file(checksum);
}

pub(crate) fn install_release_and_restart(tag: String) -> Result<(), SelfUpdateFailure> {
    let asset = release_asset_for_target(&tag, std::env::consts::OS, std::env::consts::ARCH)
        .ok_or_else(|| {
            SelfUpdateFailure::new(
                SelfUpdateFailureKind::NoMatchingBuild,
                format!(
                    "{} / {} is not a published target",
                    std::env::consts::OS,
                    std::env::consts::ARCH
                ),
            )
        })?;
    let current_exe = std::env::current_exe().map_err(|error| {
        SelfUpdateFailure::new(SelfUpdateFailureKind::InstallLocation, error.to_string())
    })?;
    let target = InstallTarget::resolve(current_exe)?;
    ensure_install_dir_writable(&target.install_dir).map_err(|error| {
        SelfUpdateFailure::new(SelfUpdateFailureKind::NotWritable, error.to_string())
    })?;

    // Asset discovery happens only after the local preflight above; no
    // archive bytes are requested for an install directory we cannot update.
    let client = ltbox_core::github::GitHubClient::new(crate::UPDATE_REPO).map_err(|error| {
        SelfUpdateFailure::new(SelfUpdateFailureKind::Download, error.to_string())
    })?;
    let assets = client.release_by_tag(&tag).map_err(|error| {
        SelfUpdateFailure::new(SelfUpdateFailureKind::Download, error.to_string())
    })?;
    let archive_url = assets
        .iter()
        .find(|(name, _)| name == &asset.archive_name)
        .map(|(_, url)| url.clone())
        .ok_or_else(|| {
            SelfUpdateFailure::new(
                SelfUpdateFailureKind::NoMatchingBuild,
                format!("release {tag} has no {} asset", asset.archive_name),
            )
        })?;
    let checksum_url = assets
        .iter()
        .find(|(name, _)| name == &asset.checksum_name)
        .map(|(_, url)| url.clone())
        .ok_or_else(|| {
            SelfUpdateFailure::new(
                SelfUpdateFailureKind::NoMatchingBuild,
                format!("release {tag} has no {} asset", asset.checksum_name),
            )
        })?;

    let work = WorkDir::create().map_err(|error| {
        SelfUpdateFailure::new(SelfUpdateFailureKind::Download, error.to_string())
    })?;
    let archive_path = work.path().join("release-archive");
    let checksum_path = work.path().join("release-archive.sha256");
    let mut log = Vec::new();
    ltbox_core::downloader::download_to_file(&archive_url, &archive_path, &mut log).map_err(
        |error| SelfUpdateFailure::new(SelfUpdateFailureKind::Download, error.to_string()),
    )?;
    if let Err(error) =
        ltbox_core::downloader::download_to_file(&checksum_url, &checksum_path, &mut log)
    {
        cleanup_downloads(&archive_path, &checksum_path);
        return Err(SelfUpdateFailure::new(
            SelfUpdateFailureKind::Download,
            error.to_string(),
        ));
    }

    let expected = fs::read_to_string(&checksum_path)
        .map_err(|error| {
            cleanup_downloads(&archive_path, &checksum_path);
            SelfUpdateFailure::new(SelfUpdateFailureKind::HashMismatch, error.to_string())
        })
        .and_then(|contents| {
            parse_sha256_sidecar(&contents).map_err(|error| {
                cleanup_downloads(&archive_path, &checksum_path);
                SelfUpdateFailure::new(SelfUpdateFailureKind::HashMismatch, error)
            })
        })?;
    let actual = sha256_file_hex(&archive_path).map_err(|error| {
        cleanup_downloads(&archive_path, &checksum_path);
        SelfUpdateFailure::new(SelfUpdateFailureKind::HashMismatch, error.to_string())
    })?;
    if actual != expected {
        cleanup_downloads(&archive_path, &checksum_path);
        return Err(SelfUpdateFailure::new(
            SelfUpdateFailureKind::HashMismatch,
            format!("expected {expected}, downloaded {actual}"),
        ));
    }

    let extracted = work.path().join("extracted");
    fs::create_dir(&extracted).map_err(|error| {
        SelfUpdateFailure::new(SelfUpdateFailureKind::Extract, error.to_string())
    })?;
    let entries = match asset.archive_kind {
        ArchiveKind::Zip => extract_zip(&archive_path, &extracted),
        ArchiveKind::TarGz => extract_tar_gz(&archive_path, &extracted),
    }
    .map_err(|error| SelfUpdateFailure::new(SelfUpdateFailureKind::Extract, error))?;
    let payload_relative = resolve_archive_payload(&entries, asset.payload_kind).map_err(|()| {
        SelfUpdateFailure::new(
            SelfUpdateFailureKind::ArchiveLayout,
            "the archive does not have exactly one expected payload",
        )
    })?;
    let payload = extracted.join(payload_relative);
    let payload_valid = match asset.payload_kind {
        PayloadKind::File(_) => payload.is_file(),
        PayloadKind::MacBundle => {
            payload.is_dir() && payload.join("Contents/MacOS/ltbox").is_file()
        }
    };
    if !payload_valid {
        return Err(SelfUpdateFailure::new(
            SelfUpdateFailureKind::ArchiveLayout,
            "the resolved update payload is missing",
        ));
    }

    // This is the first point where the install directory is touched after
    // the write probe: the archive has been downloaded, verified, completely
    // extracted, and structurally resolved.
    let (candidate, backup, failed_replacement) = prepare_candidate_and_backup(&payload, &target)
        .map_err(|error| {
        SelfUpdateFailure::new(SelfUpdateFailureKind::Swap, error.to_string())
    })?;
    let mut ops = RealSwapOps;
    let receipt = match swap_with_ops(
        &mut ops,
        target.swap_kind,
        &target.current,
        &candidate,
        &backup,
        &failed_replacement,
    ) {
        Ok(receipt) => receipt,
        Err(error) => {
            let _ = remove_unit(&candidate);
            let _ = remove_unit(&failed_replacement);
            return Err(SelfUpdateFailure::new(SelfUpdateFailureKind::Swap, error));
        }
    };

    match Command::new(&target.relaunch)
        .arg(POST_UPDATE_RELAUNCH_ARG)
        .spawn()
    {
        Ok(_) => Ok(()),
        Err(spawn_error) => {
            let rollback = rollback_with_ops(&mut ops, &receipt);
            let detail = match rollback {
                Ok(()) => {
                    format!("restart failed and the installed version was restored: {spawn_error}")
                }
                Err(rollback_error) => format!("restart failed: {spawn_error}; {rollback_error}"),
            };
            Err(SelfUpdateFailure::new(
                SelfUpdateFailureKind::Restart,
                detail,
            ))
        }
    }
}

/// Delete backups only after a process has acquired the singleton lock. The
/// new executable therefore proves it can start before the old copy goes away.
pub(crate) fn cleanup_stale_update_backups() {
    let Ok(current_exe) = std::env::current_exe() else {
        return;
    };
    let Ok(target) = InstallTarget::resolve(current_exe) else {
        return;
    };
    let Some(name) = target.current.file_name().and_then(OsStr::to_str) else {
        return;
    };
    let prefix = format!(".{name}.ltbox-old-");
    let Ok(entries) = fs::read_dir(&target.install_dir) else {
        return;
    };
    for entry in entries.flatten() {
        if entry
            .file_name()
            .to_str()
            .is_some_and(|entry_name| entry_name.starts_with(&prefix))
        {
            let _ = remove_unit(&entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashSet, VecDeque};

    /// Weekly download-only verification of every published direct-update
    /// target. This intentionally bypasses the updater entrypoint: files
    /// remain under `tempfile` and no installed program is replaced or run.
    #[test]
    #[ignore = "weekly_fetch: downloads and hashes every published self-update target"]
    fn weekly_fetch_self_update_archives_and_checksums() {
        let mut failures = Vec::new();
        let client = match ltbox_core::github::GitHubClient::new(crate::UPDATE_REPO) {
            Ok(client) => client,
            Err(error) => {
                failures.push(format!("create GitHub client: {error}"));
                assert!(
                    failures.is_empty(),
                    "weekly self-update fetch failures:\n{}",
                    failures.join("\n")
                );
                return;
            }
        };
        let release = match client.latest_stable_release() {
            Ok(Some(release)) => release,
            Ok(None) => {
                failures.push("no stable LTBox release is published".to_string());
                assert!(
                    failures.is_empty(),
                    "weekly self-update fetch failures:\n{}",
                    failures.join("\n")
                );
                return;
            }
            Err(error) => {
                failures.push(format!("select stable release: {error}"));
                assert!(
                    failures.is_empty(),
                    "weekly self-update fetch failures:\n{}",
                    failures.join("\n")
                );
                return;
            }
        };
        let assets = match client.release_by_tag(&release.tag) {
            Ok(assets) => assets,
            Err(error) => {
                failures.push(format!("load {} assets: {error}", release.tag));
                assert!(
                    failures.is_empty(),
                    "weekly self-update fetch failures:\n{}",
                    failures.join("\n")
                );
                return;
            }
        };

        for (os, arch) in [
            ("windows", "x86_64"),
            ("windows", "aarch64"),
            ("linux", "x86_64"),
            ("linux", "aarch64"),
            ("macos", "x86_64"),
            ("macos", "aarch64"),
        ] {
            let result = (|| -> std::result::Result<(), String> {
                let asset = release_asset_for_target(&release.tag, os, arch)
                    .ok_or_else(|| "production target selector returned no asset".to_string())?;
                let archive_url = assets
                    .iter()
                    .find(|(name, _)| name == &asset.archive_name)
                    .map(|(_, url)| url)
                    .ok_or_else(|| format!("release lacks {}", asset.archive_name))?;
                let checksum_url = assets
                    .iter()
                    .find(|(name, _)| name == &asset.checksum_name)
                    .map(|(_, url)| url)
                    .ok_or_else(|| format!("release lacks {}", asset.checksum_name))?;
                let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
                let archive_path = temp.path().join("archive");
                let checksum_path = temp.path().join("archive.sha256");
                let mut log = Vec::new();
                ltbox_core::downloader::download_to_file(archive_url, &archive_path, &mut log)
                    .map_err(|error| error.to_string())?;
                ltbox_core::downloader::download_to_file(checksum_url, &checksum_path, &mut log)
                    .map_err(|error| error.to_string())?;
                let expected = parse_sha256_sidecar(
                    &fs::read_to_string(&checksum_path).map_err(|error| error.to_string())?,
                )?;
                let actual = sha256_file_hex(&archive_path).map_err(|error| error.to_string())?;
                if actual != expected {
                    return Err(format!(
                        "SHA-256 mismatch: expected {expected}, downloaded {actual}"
                    ));
                }
                Ok(())
            })();
            if let Err(error) = result {
                failures.push(format!("{os}/{arch}: {error}"));
            }
        }
        assert!(
            failures.is_empty(),
            "weekly self-update fetch failures:\n{}",
            failures.join("\n")
        );
    }

    #[test]
    fn asset_selection_covers_every_published_target() {
        for (os, arch, expected) in [
            ("windows", "x86_64", "LTBox-win_x86_64-v3.2.8.zip"),
            ("windows", "aarch64", "LTBox-win_arm64-v3.2.8.zip"),
            ("linux", "x86_64", "LTBox-linux_x86_64-v3.2.8.tar.gz"),
            ("linux", "aarch64", "LTBox-linux_arm64-v3.2.8.tar.gz"),
            ("macos", "x86_64", "LTBox-macos_universal-v3.2.8.tar.gz"),
            ("macos", "aarch64", "LTBox-macos_universal-v3.2.8.tar.gz"),
        ] {
            let asset = release_asset_for_target("v3.2.8", os, arch).unwrap();
            assert_eq!(asset.archive_name, expected);
            assert_eq!(asset.checksum_name, format!("{expected}.sha256"));
        }
        assert!(release_asset_for_target("v3.2.8", "linux", "x86").is_none());
        assert!(release_asset_for_target("v3.2.8", "freebsd", "x86_64").is_none());
    }

    #[test]
    fn sha256_sidecar_accepts_published_format_and_normalizes_case() {
        let upper = "A".repeat(64);
        assert_eq!(
            parse_sha256_sidecar(&format!("{upper}  LTBox.zip\n")).unwrap(),
            "a".repeat(64)
        );
        assert_eq!(
            parse_sha256_sidecar(&"0".repeat(64)).unwrap(),
            "0".repeat(64)
        );
    }

    #[test]
    fn sha256_sidecar_rejects_empty_short_and_non_hex_values() {
        let non_hex = format!("{}z file", "0".repeat(63));
        for invalid in ["", "abcd LTBox.zip", non_hex.as_str()] {
            assert!(parse_sha256_sidecar(invalid).is_err(), "{invalid:?}");
        }
    }

    #[test]
    fn archive_layout_resolves_file_payloads_only_at_published_depth() {
        let entries = vec![
            PathBuf::from("LTBox-win_x86_64-v3.2.8/README.md"),
            PathBuf::from("LTBox-win_x86_64-v3.2.8/ltbox.exe"),
        ];
        assert_eq!(
            resolve_archive_payload(&entries, PayloadKind::File("ltbox.exe")),
            Ok(PathBuf::from("LTBox-win_x86_64-v3.2.8/ltbox.exe"))
        );
        let nested = vec![PathBuf::from("unexpected/bin/ltbox.exe")];
        assert!(resolve_archive_payload(&nested, PayloadKind::File("ltbox.exe")).is_err());
    }

    #[test]
    fn archive_layout_resolves_direct_and_wrapped_mac_bundles() {
        let direct = vec![PathBuf::from("LTBox.app/Contents/MacOS/ltbox")];
        assert_eq!(
            resolve_archive_payload(&direct, PayloadKind::MacBundle),
            Ok(PathBuf::from("LTBox.app"))
        );
        let wrapped = vec![PathBuf::from(
            "LTBox-macos_universal-v3.2.8/LTBox.app/Contents/MacOS/ltbox",
        )];
        assert_eq!(
            resolve_archive_payload(&wrapped, PayloadKind::MacBundle),
            Ok(PathBuf::from("LTBox-macos_universal-v3.2.8/LTBox.app"))
        );
    }

    #[test]
    fn archive_layout_rejects_ambiguous_payloads() {
        let entries = vec![
            PathBuf::from("LTBox-one/ltbox"),
            PathBuf::from("LTBox-two/ltbox"),
        ];
        assert!(resolve_archive_payload(&entries, PayloadKind::File("ltbox")).is_err());
    }

    #[test]
    fn tar_gz_extraction_stages_the_published_linux_layout() {
        let temp = tempfile::tempdir().unwrap();
        let archive_path = temp.path().join("release.tar.gz");
        let encoder = flate2::write::GzEncoder::new(
            File::create(&archive_path).unwrap(),
            flate2::Compression::default(),
        );
        let mut archive = tar::Builder::new(encoder);
        let payload = b"verified LTBox test payload";
        let mut header = tar::Header::new_gnu();
        header.set_path("LTBox-linux_x86_64-v3.2.8/ltbox").unwrap();
        header.set_size(payload.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        archive.append(&header, &payload[..]).unwrap();
        archive.into_inner().unwrap().finish().unwrap();

        let extracted = temp.path().join("extracted");
        fs::create_dir(&extracted).unwrap();
        let entries = extract_tar_gz(&archive_path, &extracted).unwrap();
        let relative = resolve_archive_payload(&entries, PayloadKind::File("ltbox")).unwrap();
        assert_eq!(
            fs::read(extracted.join(relative)).unwrap(),
            payload.as_slice()
        );
    }

    #[test]
    fn zip_extraction_stages_the_published_windows_layout() {
        let temp = tempfile::tempdir().unwrap();
        let archive_path = temp.path().join("release.zip");
        let mut archive = zip::ZipWriter::new(File::create(&archive_path).unwrap());
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        archive
            .start_file("LTBox-win_x86_64-v3.2.8/ltbox.exe", options)
            .unwrap();
        archive.write_all(b"verified LTBox test payload").unwrap();
        archive.finish().unwrap();

        let extracted = temp.path().join("extracted");
        fs::create_dir(&extracted).unwrap();
        let entries = extract_zip(&archive_path, &extracted).unwrap();
        let relative = resolve_archive_payload(&entries, PayloadKind::File("ltbox.exe")).unwrap();
        assert_eq!(
            fs::read(extracted.join(relative)).unwrap(),
            b"verified LTBox test payload"
        );
    }

    #[derive(Default)]
    struct FakeOps {
        present: HashSet<PathBuf>,
        failures: VecDeque<bool>,
        calls: Vec<String>,
    }

    impl FakeOps {
        fn with_paths(paths: &[&str], failures: &[bool]) -> Self {
            Self {
                present: paths.iter().map(PathBuf::from).collect(),
                failures: failures.iter().copied().collect(),
                calls: Vec::new(),
            }
        }

        fn maybe_fail(&mut self) -> io::Result<()> {
            if self.failures.pop_front().unwrap_or(false) {
                Err(io::Error::other("injected failure"))
            } else {
                Ok(())
            }
        }
    }

    impl SwapOps for FakeOps {
        fn rename(&mut self, from: &Path, to: &Path) -> io::Result<()> {
            self.calls
                .push(format!("rename:{}:{}", from.display(), to.display()));
            self.maybe_fail()?;
            if !self.present.remove(from) {
                return Err(io::Error::new(io::ErrorKind::NotFound, "missing source"));
            }
            self.present.remove(to);
            self.present.insert(to.to_path_buf());
            Ok(())
        }

        fn copy_unit(&mut self, from: &Path, to: &Path) -> io::Result<()> {
            self.calls
                .push(format!("copy:{}:{}", from.display(), to.display()));
            self.maybe_fail()?;
            if !self.present.contains(from) {
                return Err(io::Error::new(io::ErrorKind::NotFound, "missing source"));
            }
            if !self.present.insert(to.to_path_buf()) {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "destination exists",
                ));
            }
            Ok(())
        }

        fn remove_unit(&mut self, path: &Path) -> io::Result<()> {
            self.calls.push(format!("remove:{}", path.display()));
            self.maybe_fail()?;
            self.present.remove(path);
            Ok(())
        }
    }

    fn swap_paths() -> (&'static Path, &'static Path, &'static Path, &'static Path) {
        (
            Path::new("current"),
            Path::new("candidate"),
            Path::new("backup"),
            Path::new("failed"),
        )
    }

    #[test]
    fn move_aside_swap_failure_before_move_keeps_original() {
        let (current, candidate, backup, failed) = swap_paths();
        let mut ops = FakeOps::with_paths(&["current", "candidate"], &[true]);
        assert!(
            swap_with_ops(
                &mut ops,
                SwapKind::MoveAside,
                current,
                candidate,
                backup,
                failed
            )
            .is_err()
        );
        assert!(ops.present.contains(current));
        assert!(!ops.present.contains(backup));
    }

    #[test]
    fn move_aside_swap_failure_placing_candidate_restores_original() {
        let (current, candidate, backup, failed) = swap_paths();
        let mut ops = FakeOps::with_paths(&["current", "candidate"], &[false, true, false]);
        assert!(
            swap_with_ops(
                &mut ops,
                SwapKind::MoveAside,
                current,
                candidate,
                backup,
                failed
            )
            .is_err()
        );
        assert!(ops.present.contains(current));
        assert!(!ops.present.contains(backup));
    }

    #[test]
    fn move_aside_swap_uses_copy_restore_when_restore_rename_fails() {
        let (current, candidate, backup, failed) = swap_paths();
        let mut ops = FakeOps::with_paths(&["current", "candidate"], &[false, true, true, false]);
        assert!(
            swap_with_ops(
                &mut ops,
                SwapKind::MoveAside,
                current,
                candidate,
                backup,
                failed
            )
            .is_err()
        );
        assert!(ops.present.contains(current));
        assert!(ops.present.contains(backup));
    }

    #[test]
    fn move_aside_swap_retains_verified_candidate_when_both_restores_fail() {
        let (current, candidate, backup, failed) = swap_paths();
        let mut ops =
            FakeOps::with_paths(&["current", "candidate"], &[false, true, true, true, false]);
        assert!(
            swap_with_ops(
                &mut ops,
                SwapKind::MoveAside,
                current,
                candidate,
                backup,
                failed
            )
            .is_err()
        );
        assert!(ops.present.contains(current));
        assert!(ops.present.contains(backup));
        assert!(!ops.present.contains(candidate));
    }

    #[test]
    fn atomic_swap_failure_leaves_original_and_removes_backup() {
        let (current, candidate, backup, failed) = swap_paths();
        let mut ops = FakeOps::with_paths(&["current", "candidate", "backup"], &[true]);
        assert!(
            swap_with_ops(
                &mut ops,
                SwapKind::AtomicFile,
                current,
                candidate,
                backup,
                failed
            )
            .is_err()
        );
        assert!(ops.present.contains(current));
        assert!(!ops.present.contains(backup));
    }

    #[test]
    fn move_aside_rollback_restores_original_and_removes_replacement() {
        let (current, _candidate, backup, failed) = swap_paths();
        let receipt = SwapReceipt {
            kind: SwapKind::MoveAside,
            current: current.into(),
            backup: backup.into(),
            failed_replacement: failed.into(),
        };
        let mut ops = FakeOps::with_paths(&["current", "backup"], &[]);
        rollback_with_ops(&mut ops, &receipt).unwrap();
        assert!(ops.present.contains(current));
        assert!(!ops.present.contains(backup));
        assert!(!ops.present.contains(failed));
    }

    #[test]
    fn move_aside_rollback_failure_moving_replacement_keeps_it_runnable() {
        let (current, _candidate, backup, failed) = swap_paths();
        let receipt = SwapReceipt {
            kind: SwapKind::MoveAside,
            current: current.into(),
            backup: backup.into(),
            failed_replacement: failed.into(),
        };
        let mut ops = FakeOps::with_paths(&["current", "backup"], &[true]);
        assert!(rollback_with_ops(&mut ops, &receipt).is_err());
        assert!(ops.present.contains(current));
    }

    #[test]
    fn move_aside_rollback_uses_copy_when_restore_rename_fails() {
        let (current, _candidate, backup, failed) = swap_paths();
        let receipt = SwapReceipt {
            kind: SwapKind::MoveAside,
            current: current.into(),
            backup: backup.into(),
            failed_replacement: failed.into(),
        };
        let mut ops = FakeOps::with_paths(&["current", "backup"], &[false, true, false]);
        rollback_with_ops(&mut ops, &receipt).unwrap();
        assert!(ops.present.contains(current));
        assert!(ops.present.contains(backup));
        assert!(!ops.present.contains(failed));
    }

    #[test]
    fn move_aside_rollback_retains_replacement_when_both_restores_fail() {
        let (current, _candidate, backup, failed) = swap_paths();
        let receipt = SwapReceipt {
            kind: SwapKind::MoveAside,
            current: current.into(),
            backup: backup.into(),
            failed_replacement: failed.into(),
        };
        let mut ops = FakeOps::with_paths(&["current", "backup"], &[false, true, true, false]);
        assert!(rollback_with_ops(&mut ops, &receipt).is_err());
        assert!(ops.present.contains(current));
        assert!(ops.present.contains(backup));
        assert!(!ops.present.contains(failed));
    }

    #[test]
    fn atomic_rollback_restores_original() {
        let (current, _candidate, backup, failed) = swap_paths();
        let receipt = SwapReceipt {
            kind: SwapKind::AtomicFile,
            current: current.into(),
            backup: backup.into(),
            failed_replacement: failed.into(),
        };
        let mut ops = FakeOps::with_paths(&["current", "backup"], &[]);
        rollback_with_ops(&mut ops, &receipt).unwrap();
        assert!(ops.present.contains(current));
        assert!(!ops.present.contains(backup));
    }

    #[test]
    fn atomic_rollback_failure_keeps_replacement_runnable() {
        let (current, _candidate, backup, failed) = swap_paths();
        let receipt = SwapReceipt {
            kind: SwapKind::AtomicFile,
            current: current.into(),
            backup: backup.into(),
            failed_replacement: failed.into(),
        };
        let mut ops = FakeOps::with_paths(&["current", "backup"], &[true]);
        assert!(rollback_with_ops(&mut ops, &receipt).is_err());
        assert!(ops.present.contains(current));
        assert!(ops.present.contains(backup));
    }
}
