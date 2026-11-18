//! Qualcomm 9008 EDL driver detection + auto-install on Windows.
//!
//! Userspace mode requires `qcserlib.inf` from Qualcomm's WinUSB bundle.
//! Kernel mode requires `qcwdfser.inf` from Qualcomm's kernel-driver bundle.
//! Both modes run signed per-arch installers through Windows UAC. When the
//! installer fails only because this host's `pnputil` rejects the `/force`
//! flag its bundled `qdinstall.exe` passes, a second elevated step finishes
//! the install from the files that installer run left behind.

use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use ltbox_core::github::GitHubClient;
use ltbox_core::i18n::tr;
use ltbox_core::{live, tr_args};

use crate::software_fix::escape_powershell_single_quoted;

use super::{
    DriverError, DriverStatus, DriverUpdate, QcomDriverMode, Result, parse_version,
    qcom_driver_mode, version_from_tag, version_lt,
};

/// `Command::new` with no console window.
fn silent_command(program: impl AsRef<OsStr>) -> Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut cmd = Command::new(program);
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

/// Trusted system Windows PowerShell host. Prefer this over a PATH-resolved
/// `powershell` so signature verification and UAC elevation cannot be
/// redirected through a same-named executable earlier on PATH.
///
/// Resolves System32 via `GetSystemDirectoryW` rather than the mutable
/// `SystemRoot` environment variable, and never falls back to PATH.
fn windows_powershell_exe() -> Result<PathBuf> {
    let system32 = windows_system32_dir()?;
    Ok(system32
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe"))
}

/// Absolute System32 directory from the Win32 API (not environment variables).
fn windows_system32_dir() -> Result<PathBuf> {
    use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;

    // First call with an empty buffer returns the required UTF-16 length,
    // including the trailing NUL (or 0 on failure).
    let needed = unsafe { GetSystemDirectoryW(std::ptr::null_mut(), 0) };
    if needed == 0 {
        return Err(DriverError::Io(std::io::Error::last_os_error()));
    }

    let mut buf = vec![0u16; needed as usize];
    let written = unsafe { GetSystemDirectoryW(buf.as_mut_ptr(), needed) };
    if written == 0 || written >= needed {
        // 0 = failure; written >= needed means the buffer was too small
        // (directory changed between calls — extremely rare, still fatal).
        let err = if written == 0 {
            std::io::Error::last_os_error()
        } else {
            std::io::Error::other("GetSystemDirectoryW buffer too small")
        };
        return Err(DriverError::Io(err));
    }

    let path = OsString::from_wide(&buf[..written as usize]);
    let path = PathBuf::from(path);
    if path.as_os_str().is_empty() {
        return Err(DriverError::Io(std::io::Error::other(
            "GetSystemDirectoryW returned an empty path",
        )));
    }
    Ok(path)
}

/// Qualcomm legal entities whose code-signing certificates may sign the
/// driver installers, compared against the signer's `O=` attribute.
const QUALCOMM_SIGNER_ORGS: &[&str] = &[
    "Qualcomm Technologies, Inc.",
    "Qualcomm Incorporated",
    "Qualcomm Innovation Center, Inc.",
];

/// `true` when an Authenticode signer `Subject`'s organization (`O=`) is a
/// Qualcomm entity. A substring match would also accept any valid
/// certificate that merely mentions Qualcomm, e.g. `CN=NotQualcomm Ltd`.
fn signer_subject_is_qualcomm(subject: &str) -> bool {
    subject_attribute(subject, "O").is_some_and(|org| {
        QUALCOMM_SIGNER_ORGS
            .iter()
            .any(|known| org.eq_ignore_ascii_case(known))
    })
}

/// Value of one attribute in an X.500 subject such as
/// `CN="Qualcomm Technologies, Inc.", O=..., C=US`. Values may be quoted or
/// contain bare commas, so an RDN only ends at `,` followed by `KEY=`.
fn subject_attribute<'a>(subject: &'a str, key: &str) -> Option<&'a str> {
    let mut rest = subject.trim();
    while !rest.is_empty() {
        let (name, value_and_more) = rest.split_once('=')?;
        let end = rdn_value_end(value_and_more);
        let value = value_and_more[..end].trim();
        if name.trim().eq_ignore_ascii_case(key) {
            let value = value
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .unwrap_or(value);
            return Some(value);
        }
        rest = value_and_more[end..].trim_start_matches(',').trim_start();
    }
    None
}

/// Byte offset where an RDN value ends: just past the closing quote of a
/// quoted value, or at the first `,` followed by an attribute key and `=`.
fn rdn_value_end(value: &str) -> usize {
    let trimmed = value.trim_start();
    let lead = value.len() - trimmed.len();
    if let Some(quoted) = trimmed.strip_prefix('"')
        && let Some(close) = quoted.find('"')
    {
        return lead + close + 2;
    }
    for (index, _) in value.match_indices(',') {
        let after = value[index + 1..].trim_start();
        let key_len = after
            .bytes()
            .take_while(|byte| byte.is_ascii_alphanumeric() || *byte == b'.')
            .count();
        if key_len > 0 && after.as_bytes().get(key_len) == Some(&b'=') {
            return index;
        }
    }
    value.len()
}

#[derive(Clone, Copy)]
struct WindowsDriverSpec {
    owner_repo: &'static str,
    required_infs: &'static [&'static str],
    version_inf: &'static str,
    /// Add/Remove Programs `DisplayName` of the installed package. When set,
    /// the update check reads its `DisplayVersion` from the uninstall registry
    /// so it can compare in the GitHub release tag's version namespace.
    /// `None` falls back to the INF `DriverVer`. Presence detection always uses
    /// the required INF files, regardless of this update-version source.
    uninstall_display_name: Option<&'static str>,
    asset_x64: &'static str,
    asset_arm64: &'static str,
    asset_x86: &'static str,
    /// Stem of the zip a release may ship instead of the per-arch installers
    /// (`<stem>_<version>.zip`, holding the same `.exe` files at its root).
    bundle_stem: &'static str,
}

const USERSPACE_SPEC: WindowsDriverSpec = WindowsDriverSpec {
    owner_repo: "qualcomm/qcom-usb-userspace-drivers",
    required_infs: &["qcserlib.inf"],
    version_inf: "qcserlib.inf",
    // The userspace package can advance independently of `qcserlib.inf`
    // (package/release 1.0.2.2 still ships INF DriverVer 1.0.2.1). Read the
    // installer's package version to avoid a perpetual false update banner.
    uninstall_display_name: Some("Qualcomm USB Userspace Drivers"),
    asset_x64: "qcom_usb_userspace_drivers_x64.exe",
    asset_arm64: "qcom_usb_userspace_drivers_arm64.exe",
    asset_x86: "qcom_usb_userspace_drivers_x86.exe",
    // release-win-v1.0.2.3 ships only this bundle, while its README still
    // documents the per-arch installers as the package.
    bundle_stem: "qcom_usb_userspace_drivers",
};

const KERNEL_SPEC: WindowsDriverSpec = WindowsDriverSpec {
    owner_repo: "qualcomm/qcom-usb-kernel-drivers",
    required_infs: &["qcwdfser.inf"],
    version_inf: "qcwdfser.inf",
    // The kernel `qcwdfser.inf` DriverVer (e.g. "1.0.3.6") is a different
    // namespace than the QUD release tag / package version (e.g. "1.00.94.6"),
    // so comparing the two always reports an update. The installer registers
    // the QUD package version under this Add/Remove Programs name; read it for
    // a like-for-like comparison instead.
    uninstall_display_name: Some("Qualcomm USB Kernel Drivers"),
    asset_x64: "qcom_usb_kernel_drivers_x64.exe",
    asset_arm64: "qcom_usb_kernel_drivers_arm64.exe",
    asset_x86: "qcom_usb_kernel_drivers_x86.exe",
    bundle_stem: "qcom_usb_kernel_drivers",
};

fn driver_spec(mode: QcomDriverMode) -> WindowsDriverSpec {
    match mode {
        QcomDriverMode::Userspace => USERSPACE_SPEC,
        QcomDriverMode::Kernel => KERNEL_SPEC,
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
struct GithubRelease {
    #[serde(default)]
    tag_name: String,
    #[serde(default)]
    assets: Vec<GithubAsset>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    /// ISO-8601 publish time, used to pick the newest matching release
    /// deterministically instead of trusting the order GitHub lists them in.
    #[serde(default)]
    published_at: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

/// Windows release tags carry a `win` token (`release-win-v1.0.2.0`); the
/// repo also publishes Linux-only tags that ship no `.exe` installer.
const WIN_TAG_NEEDLE: &str = "win";

/// Signed installer asset name for the host architecture. The release
/// ships one self-extracting `.exe` per arch.
fn arch_installer_asset(spec: WindowsDriverSpec) -> &'static str {
    if cfg!(target_arch = "aarch64") {
        spec.asset_arm64
    } else if cfg!(target_arch = "x86") {
        spec.asset_x86
    } else {
        spec.asset_x64
    }
}

/// Probe whether the Qualcomm USB drivers are installed.
pub fn check_required_drivers() -> DriverStatus {
    let spec = driver_spec(qcom_driver_mode());
    let missing: Vec<&'static str> = spec
        .required_infs
        .iter()
        .copied()
        .filter(|inf| !is_driver_present(inf))
        .collect();

    if missing.is_empty() {
        DriverStatus::Present
    } else {
        DriverStatus::Missing(missing)
    }
}

fn is_driver_present(inf_name: &str) -> bool {
    driver_present_via_pnputil(inf_name) || driver_present_via_driver_store(inf_name)
}

fn driver_present_via_pnputil(inf_name: &str) -> bool {
    let output = match silent_command("pnputil").arg("/enum-drivers").output() {
        Ok(o) => o,
        Err(_) => return false,
    };
    if !output.status.success() {
        return false;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let target = inf_name.to_ascii_lowercase();
    stdout.lines().any(|line| {
        if let Some((_, v)) = line.split_once(':') {
            v.trim().to_ascii_lowercase() == target
        } else {
            false
        }
    })
}

fn driver_present_via_driver_store(inf_name: &str) -> bool {
    let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_string());
    let repo = Path::new(&system_root)
        .join("System32")
        .join("DriverStore")
        .join("FileRepository");
    let Ok(entries) = std::fs::read_dir(&repo) else {
        return false;
    };
    let prefix = inf_name.to_ascii_lowercase();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if name.starts_with(&prefix) {
            return true;
        }
    }
    false
}

fn fetch_win_releases(spec: WindowsDriverSpec) -> Result<Vec<GithubRelease>> {
    GitHubClient::new(spec.owner_repo)
        .and_then(|client| client.releases())
        .map_err(|error| DriverError::Http(error.to_string()))
}

/// Where a selected release keeps the host-arch installer.
#[derive(Debug, Clone, PartialEq, Eq)]
enum InstallerSource {
    /// The signed installer is a release asset of its own.
    Direct { url: String },
    /// The release ships only the `<stem>_<version>.zip` bundle; the signed
    /// installer is the same-named entry at its root.
    Bundle { url: String, zip_name: String },
}

/// Newest stable Windows release that provides the host-arch installer.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SelectedRelease {
    tag: String,
    source: InstallerSource,
}

/// Fetch the latest Windows release that ships the host-arch installer,
/// directly or inside its bundle zip, excluding drafts and prereleases.
/// Shared by the installer and the update check so both resolve to the same release.
fn fetch_latest_win_release() -> Result<SelectedRelease> {
    let spec = driver_spec(qcom_driver_mode());
    let releases = fetch_win_releases(spec)?;
    select_latest_win_release(releases, arch_installer_asset(spec), spec.bundle_stem)
        .ok_or(DriverError::NoAsset)
}

/// Pick the newest published stable Windows release that carries
/// `asset_name`, either as an asset or inside its `<bundle_stem>_<version>.zip`.
/// Drafts and prereleases are rejected before tag/asset matching; among the
/// remaining candidates the highest `published_at` wins.
fn select_latest_win_release(
    releases: Vec<GithubRelease>,
    asset_name: &str,
    bundle_stem: &str,
) -> Option<SelectedRelease> {
    releases
        .into_iter()
        .filter(|r| !r.draft && !r.prerelease)
        .filter(|r| r.tag_name.to_ascii_lowercase().contains(WIN_TAG_NEEDLE))
        .filter_map(|r| {
            let source = installer_source(&r, asset_name, bundle_stem)?;
            Some((r.published_at, r.tag_name, source))
        })
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, tag, source)| SelectedRelease { tag, source })
}

/// The release's own installer asset wins; the bundle zip is the fallback and
/// must be named exactly for this release's version, so an unrelated archive
/// (an older package format, or a drivers-only zip) never matches.
fn installer_source(
    release: &GithubRelease,
    asset_name: &str,
    bundle_stem: &str,
) -> Option<InstallerSource> {
    let find = |name: &str| {
        release
            .assets
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case(name))
    };
    if let Some(asset) = find(asset_name) {
        return Some(InstallerSource::Direct {
            url: asset.browser_download_url.clone(),
        });
    }
    let version = version_from_tag(&release.tag_name)?;
    let asset = find(&format!("{bundle_stem}_{version}.zip"))?;
    Some(InstallerSource::Bundle {
        url: asset.browser_download_url.clone(),
        zip_name: asset.name.clone(),
    })
}

/// Asset line for the log: a bundled installer also names its zip.
fn asset_label(asset_name: &str, source: &InstallerSource) -> String {
    match source {
        InstallerSource::Direct { .. } => asset_name.to_string(),
        InstallerSource::Bundle { zip_name, .. } => format!("{asset_name} ({zip_name})"),
    }
}

/// Largest installer accepted out of a bundle zip (today's are under 1 MB).
const MAX_BUNDLED_INSTALLER_BYTES: u64 = 64 * 1024 * 1024;

/// Copy the root-level `asset_name` entry of the bundle at `zip_path` to
/// `out_path`. Exactly one such entry must exist; the copy is size-capped and
/// still goes through the Authenticode check before it runs.
fn extract_bundled_installer(zip_path: &Path, asset_name: &str, out_path: &Path) -> Result<()> {
    use std::io::{Read, Write};
    let file = std::fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(DriverError::Zip)?;
    let mut found = None;
    for index in 0..archive.len() {
        let member = archive.by_index(index).map_err(DriverError::Zip)?;
        if member.is_file()
            && member
                .name()
                .is_ok_and(|name| name.eq_ignore_ascii_case(asset_name))
        {
            if found.is_some() {
                return Err(DriverError::Parse(format!(
                    "driver bundle holds {asset_name} more than once"
                )));
            }
            found = Some(index);
        }
    }
    let index = found.ok_or(DriverError::NoAsset)?;
    let mut member = archive.by_index(index).map_err(DriverError::Zip)?;
    if member.size() > MAX_BUNDLED_INSTALLER_BYTES {
        return Err(DriverError::Parse(format!(
            "bundled installer too large: {} bytes",
            member.size()
        )));
    }
    let mut out = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(out_path)?;
    let copied = std::io::copy(
        &mut member.by_ref().take(MAX_BUNDLED_INSTALLER_BYTES + 1),
        &mut out,
    )
    .map_err(|e| DriverError::Http(format!("extract: {e}")))?;
    if copied > MAX_BUNDLED_INSTALLER_BYTES {
        return Err(DriverError::Parse(format!(
            "bundled installer too large: {copied} bytes"
        )));
    }
    out.flush()?;
    Ok(())
}

/// Check whether a newer signed driver release exists than the one
/// installed. Returns `Some` only when a driver is present locally AND the
/// latest Windows release is strictly newer. Any failure — no driver
/// installed, version unparseable, offline, GitHub unreachable — collapses
/// to `None` so the caller can fail silently (no banner).
pub fn check_driver_update() -> Option<DriverUpdate> {
    let current = installed_driver_version()?;
    let latest = version_from_tag(&fetch_latest_win_release().ok()?.tag)?;
    if version_lt(&current, &latest) {
        Some(DriverUpdate { current, latest })
    } else {
        None
    }
}

/// Read an `.inf` as text, honouring a UTF-16LE BOM (some signed INFs ship
/// UTF-16) and falling back to lossy UTF-8/ANSI otherwise.
fn read_inf_text(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let u16s: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        Some(String::from_utf16_lossy(&u16s))
    } else {
        Some(String::from_utf8_lossy(&bytes).into_owned())
    }
}

/// Strip exactly one balanced surrounding `"` pair, leaving an unbalanced
/// quote untouched so it fails downstream validation.
fn strip_balanced_quotes(s: &str) -> &str {
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

/// Parse the version half of an INF `DriverVer = MM/DD/YYYY,V.V.V.V` line
/// (case-insensitive key, optional spaces, comma-separated date+version).
fn parse_driver_ver(inf_text: &str) -> Option<String> {
    for line in inf_text.lines() {
        let line = line.trim();
        let Some((key, val)) = line.split_once('=') else {
            continue;
        };
        if !key.trim().eq_ignore_ascii_case("DriverVer") {
            continue;
        }
        // INF allows the whole value to be wrapped in one quote pair
        // (`DriverVer = "date,version"`). Strip a *balanced* pair only — an
        // unbalanced quote is left in place so it fails the strict parse.
        let val = strip_balanced_quotes(val.trim());
        // `DriverVer` is `date,version` (or just `version`). Take everything
        // after the FIRST comma — a stray extra comma then stays inside a
        // component (`1.+2,1.2` → component `+2,1`) and fails the parse,
        // rather than `rsplit` silently grabbing a clean trailing fragment.
        let ver = val.split_once(',').map(|(_, v)| v).unwrap_or(val).trim();
        // Strict parse: a malformed `DriverVer` version yields `None`
        // rather than a truncated comparison value.
        if parse_version(ver).is_some() {
            return Some(ver.to_string());
        }
    }
    None
}

/// Installed driver version for the active mode, in the same version namespace
/// as [`version_from_tag`], or `None` when the driver is not installed / the
/// version is unparseable (which collapses the update check to no banner).
///
/// Both Windows packages read their installer `DisplayVersion` from the
/// uninstall registry because their INF `DriverVer` values can differ from the
/// corresponding release tags (see [`USERSPACE_SPEC`] and [`KERNEL_SPEC`]).
pub fn installed_driver_version() -> Option<String> {
    let spec = driver_spec(qcom_driver_mode());
    match spec.uninstall_display_name {
        Some(name) => installed_version_from_registry(name),
        None => installed_inf_driver_version(spec),
    }
}

/// Read the installed package `DisplayVersion` from the Windows "Add/Remove
/// Programs" (uninstall) registry by matching `display_name` across the 64-bit,
/// 32-bit (`WOW6432Node`), and per-user hives. Returns `None` when the package
/// is not registered or its version is unparseable. PowerShell is reused here
/// (as for the elevated installer) to avoid a registry-crate dependency.
fn installed_version_from_registry(display_name: &str) -> Option<String> {
    // Escape for a PowerShell single-quoted literal (`'` → `''`). The names are
    // static constants without quotes, but escape defensively.
    let needle = escape_powershell_single_quoted(display_name);
    let script = format!(
        "$ErrorActionPreference='SilentlyContinue'; \
         $p=@('HKLM:\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\*',\
         'HKLM:\\SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\*',\
         'HKCU:\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\*'); \
         $v=Get-ItemProperty $p | Where-Object {{ $_.DisplayName -eq '{needle}' }} | \
         Select-Object -First 1 -ExpandProperty DisplayVersion; \
         if ($v) {{ [Console]::Out.Write($v) }}"
    );
    let out = silent_command(windows_powershell_exe().ok()?)
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-Command")
        .arg(&script)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let version = String::from_utf8_lossy(&out.stdout).trim().to_string();
    parse_version(&version).is_some().then_some(version)
}

/// Highest `DriverVer` among installed `version_inf` DriverStore copies,
/// or `None` when the driver is not installed. Windows may stage several
/// `*.inf_*` folders (multiple versions); the max is the effective
/// one for an update comparison.
fn installed_inf_driver_version(spec: WindowsDriverSpec) -> Option<String> {
    let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_string());
    let repo = Path::new(&system_root)
        .join("System32")
        .join("DriverStore")
        .join("FileRepository");
    let entries = std::fs::read_dir(&repo).ok()?;
    let mut best: Option<String> = None;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if !name.starts_with(spec.version_inf) {
            continue;
        }
        let inf = entry.path().join(spec.version_inf);
        let Some(text) = read_inf_text(&inf) else {
            continue;
        };
        if let Some(v) = parse_driver_ver(&text) {
            best = match best {
                Some(b) if !version_lt(&b, &v) => Some(b),
                _ => Some(v),
            };
        }
    }
    best
}

/// Download the host-arch userspace-driver installer and run it elevated.
pub fn download_and_install(log: &mut Vec<String>) -> Result<()> {
    live!(log, "[Driver] {}", tr("live_driver_fetch_meta"));
    let spec = driver_spec(qcom_driver_mode());
    let asset_name = arch_installer_asset(spec);
    let SelectedRelease { tag, source } = fetch_latest_win_release()?;
    // The fallback registers the package under the release-tag version, the
    // namespace the update check compares against.
    let fallback =
        spec.uninstall_display_name
            .zip(version_from_tag(&tag))
            .map(|(display_name, version)| PnputilFallback {
                display_name,
                version,
            });

    live!(
        log,
        "[Driver] {}",
        tr_args!(
            "live_driver_asset",
            name = &asset_label(asset_name, &source)
        )
    );

    // Private, exclusive temp dir under the process temp root. A predictable
    // `temp_dir()/ltbox_qcom_drv_{pid}` path is a classic local symlink /
    // content-swap race before the elevated installer runs; exclusive
    // create_dir with a unique name rejects pre-created paths and keeps the
    // downloaded package private until install completes.
    let tmp_dir = PrivateTempDir::create("ltbox_qcom_drv")?;
    let exe_path = tmp_dir.path().join(asset_name);

    let dl_agent = ltbox_core::downloader::build_agent();

    let result = (|| {
        match &source {
            InstallerSource::Direct { url } => {
                download_with_progress(&dl_agent, url, asset_name, &exe_path, log)?;
            }
            InstallerSource::Bundle { url, zip_name } => {
                let zip_path = tmp_dir.path().join(zip_name);
                download_with_progress(&dl_agent, url, zip_name, &zip_path, log)?;
                extract_bundled_installer(&zip_path, asset_name, &exe_path)?;
            }
        }
        verify_qualcomm_authenticode(&exe_path, log)?;
        live!(log, "[Driver] {}", tr("live_driver_running_installer"));
        run_installer_elevated(&exe_path, fallback.as_ref(), log)?;
        live!(log, "[Driver] {}", tr("live_driver_install_finished"));
        Ok(())
    })();
    drop(tmp_dir);
    result
}

/// Require a Valid Authenticode signature whose signer Subject names Qualcomm
/// before launching the elevated installer. Failures map to actionable
/// `DriverError::Parse` messages (no new error variants).
fn verify_qualcomm_authenticode(exe: &Path, log: &mut Vec<String>) -> Result<()> {
    // Escape for a PowerShell single-quoted string literal (`'` -> `''`).
    let exe_str = escape_powershell_single_quoted(&exe.to_string_lossy());
    // Emit a single parseable line: STATUS|SUBJECT. STATUS is the
    // `SignatureStatus` enum name (Valid, NotSigned, HashMismatch, ...).
    // Subject is empty when unsigned / unavailable.
    let script = format!(
        "$ErrorActionPreference='Stop'; \
         try {{ \
           $s = Get-AuthenticodeSignature -LiteralPath '{exe_str}'; \
           $status = [string]$s.Status; \
           $subject = if ($null -ne $s.SignerCertificate) {{ [string]$s.SignerCertificate.Subject }} else {{ '' }}; \
           [Console]::Out.Write(($status + '|' + $subject)); \
           exit 0 \
         }} catch {{ \
           [Console]::Error.Write($_.Exception.Message); \
           exit 2 \
         }}"
    );

    let out = silent_command(windows_powershell_exe()?)
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-Command")
        .arg(&script)
        .output()
        .map_err(|e| {
            DriverError::Io(std::io::Error::new(
                e.kind(),
                format!("authenticode verification failed to start: {e}"),
            ))
        })?;

    if !out.status.success() {
        let detail = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let msg = if detail.is_empty() {
            format!(
                "authenticode verification failed (exit {})",
                out.status.code().unwrap_or(-1)
            )
        } else {
            format!("authenticode verification failed: {detail}")
        };
        live!(log, "[Driver] {msg}");
        return Err(DriverError::Parse(msg));
    }

    let line = String::from_utf8_lossy(&out.stdout);
    let line = line.trim();
    let (status, subject) = line.split_once('|').unwrap_or((line, ""));
    let status = status.trim();
    let subject = subject.trim();

    if !status.eq_ignore_ascii_case("Valid") {
        let msg = if status.is_empty() {
            "installer Authenticode signature is not Valid".to_string()
        } else {
            format!("installer Authenticode signature status is {status}, expected Valid")
        };
        live!(log, "[Driver] {msg}");
        return Err(DriverError::Parse(msg));
    }

    if !signer_subject_is_qualcomm(subject) {
        let msg = if subject.is_empty() {
            "installer Authenticode signer is missing; expected Qualcomm".to_string()
        } else {
            format!("installer Authenticode signer is not Qualcomm (subject: {subject})")
        };
        live!(log, "[Driver] {msg}");
        return Err(DriverError::Parse(msg));
    }

    Ok(())
}

/// Where the Qualcomm installers extract their payload. Both packages share
/// it, and both leave it in place when their `qdinstall.exe -i` step fails.
const QUD_INSTALL_DIR: &str = r"Qualcomm\Qualcomm USB Drivers";

/// Uninstall key `qdinstall.exe` writes on success; both packages share it.
const QUD_UNINSTALL_KEY: &str =
    r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Qualcomm USB Drivers";

/// Publisher `qdinstall.exe` registers (`QCOM_USB_DRIVERS_COMPANY_NAME`).
const QUD_PUBLISHER: &str = "Qualcomm Technologies, Inc.";

/// Installer exit code when `qdinstall.exe -i` returns `pnputil`'s usage
/// error. Any other failure never reaches the fallback.
const INSTALLER_PNPUTIL_USAGE_EXIT: i32 = 1;

/// Fallback-script exit codes, outside the Win32 error range `pnputil` uses.
const FALLBACK_EXIT_NO_FRESH_PAYLOAD: i32 = 0x2000_0001;
const FALLBACK_EXIT_SCRIPT_ERROR: i32 = 0x2000_0002;

/// .NET `DateTime` ticks (100 ns) at the Unix epoch.
const DOTNET_TICKS_AT_UNIX_EPOCH: i64 = 621_355_968_000_000_000;

/// Slack for filesystem timestamp granularity when checking that the payload
/// was extracted by this installer run (2 s, the coarsest common granularity).
const FRESH_PAYLOAD_SLACK_TICKS: i64 = 20_000_000;

/// What the fallback registers in place of `qdinstall.exe`.
struct PnputilFallback {
    /// Add/Remove Programs name of the package being installed.
    display_name: &'static str,
    /// Package version, already validated by [`version_from_tag`].
    version: String,
}

/// Run the signed installer through UAC and map cancel vs failure.
///
/// The installer itself is elevated directly, so the prompt still names the
/// Qualcomm-signed binary. Only when it exits with `pnputil`'s usage error
/// and this host's `pnputil` provably rejects `/force` does a second, separate
/// prompt run [`build_pnputil_fallback_script`].
fn run_installer_elevated(
    exe: &Path,
    fallback: Option<&PnputilFallback>,
    log: &mut Vec<String>,
) -> Result<()> {
    let launched_at = std::time::SystemTime::now();
    let out = run_elevated(exe)?;
    let code = out.status.code().unwrap_or(-1);
    match code {
        0 => Ok(()),
        // ERROR_CANCELLED — the user dismissed the UAC elevation prompt.
        1223 => {
            live!(log, "[Driver] {}", tr("live_driver_install_cancelled"));
            Err(DriverError::InstallCancelled)
        }
        other => {
            let detail = String::from_utf8_lossy(&out.stderr).trim().to_string();
            if !detail.is_empty() {
                live!(log, "[Driver] {detail}");
            }
            // The launcher also exits 1 when the installer never started, but
            // then it writes the launch error to stderr.
            if other == INSTALLER_PNPUTIL_USAGE_EXIT
                && detail.is_empty()
                && let Some(fallback) = fallback
                && pnputil_rejects_add_driver_force()
                && qud_payload_is_fresh(launched_at)
            {
                return run_pnputil_fallback(fallback, dotnet_utc_ticks(launched_at), other, log);
            }
            live!(
                log,
                "[Driver] {}",
                tr_args!("live_driver_installer_failed", exit = other)
            );
            Err(DriverError::InstallerFailed { exit_code: other })
        }
    }
}

/// Finish the install the failed installer started, elevated, and map the
/// outcome. A failure still reports the installer's own exit code.
fn run_pnputil_fallback(
    fallback: &PnputilFallback,
    launched_at: i64,
    installer_exit: i32,
    log: &mut Vec<String>,
) -> Result<()> {
    live!(
        log,
        "[Driver] {}",
        tr("live_driver_installer_pnputil_fallback")
    );
    let script = build_pnputil_fallback_script(fallback, launched_at);
    let out = run_elevated_powershell(&script)?;
    match out.status.code().unwrap_or(-1) {
        0 => Ok(()),
        1223 => {
            live!(log, "[Driver] {}", tr("live_driver_install_cancelled"));
            Err(DriverError::InstallCancelled)
        }
        code => {
            let detail = String::from_utf8_lossy(&out.stderr).trim().to_string();
            if !detail.is_empty() {
                live!(log, "[Driver] {detail}");
            }
            let reason = match code {
                FALLBACK_EXIT_NO_FRESH_PAYLOAD => {
                    "pnputil fallback skipped: the installer left no payload from this run"
                        .to_string()
                }
                FALLBACK_EXIT_SCRIPT_ERROR => "pnputil fallback script failed".to_string(),
                code => format!("pnputil fallback: add-driver exited with code {code}"),
            };
            live!(log, "[Driver] {reason}");
            live!(
                log,
                "[Driver] {}",
                tr_args!("live_driver_installer_failed", exit = installer_exit)
            );
            Err(DriverError::InstallerFailed {
                exit_code: installer_exit,
            })
        }
    }
}

/// `true` when this host's `pnputil` rejects `/force` on `/add-driver`.
///
/// Both probes target a directory that does not exist under System32 (which a
/// standard user cannot create), so neither can add a driver. `/force` fails
/// argument parsing with exit 1 and the usage text; without it the same call
/// gets past parsing and fails on the missing path with another code. Runs
/// unelevated: parsing happens before any privilege check.
fn pnputil_rejects_add_driver_force() -> bool {
    let Ok(system32) = windows_system32_dir() else {
        return false;
    };
    let pnputil = system32.join("pnputil.exe");
    let absent = system32
        .join(format!("ltbox-pnputil-probe-{}", std::process::id()))
        .join("*.inf");
    if absent.parent().is_some_and(Path::exists) {
        return false;
    }
    let probe = |force: bool| {
        let mut cmd = silent_command(&pnputil);
        cmd.arg("/add-driver").arg(&absent).arg("/install");
        if force {
            cmd.arg("/force");
        }
        cmd.output().ok().and_then(|out| out.status.code())
    };
    probe_shows_force_rejected(probe(true), probe(false))
}

/// Only the flag can explain the failure when the `/force` call exits 1 and
/// the identical call without it does not.
fn probe_shows_force_rejected(with_force: Option<i32>, without_force: Option<i32>) -> bool {
    with_force == Some(1) && without_force.is_some_and(|code| code != 1)
}

/// Unelevated pre-check of the payload gate, so a run that the fallback script
/// would refuse never shows a second prompt. The elevated script repeats the
/// check against the real Program Files folder; this one may read the
/// environment because it can only skip the fallback, never enable it alone.
fn qud_payload_is_fresh(launched_at: std::time::SystemTime) -> bool {
    std::env::var_os("ProgramFiles").is_some_and(|program_files| {
        payload_is_fresh(
            &Path::new(&program_files).join(QUD_INSTALL_DIR),
            launched_at,
        )
    })
}

/// `true` when `dir` holds `qdinstall.exe` and at least one `.inf` extracted
/// after `launched_at` (less [`FRESH_PAYLOAD_SLACK_TICKS`]).
///
/// The installer deletes and re-creates `dir` before extracting, so the
/// directory's last write time moves past the launch. The extractor keeps the
/// archive's write times on files but stamps their creation time, so a fresh
/// `qdinstall.exe` creation time is accepted as well. Mirrors the checks in
/// [`build_pnputil_fallback_script`].
fn payload_is_fresh(dir: &Path, launched_at: std::time::SystemTime) -> bool {
    let slack = std::time::Duration::from_nanos(FRESH_PAYLOAD_SLACK_TICKS as u64 * 100);
    let cutoff = launched_at.checked_sub(slack).unwrap_or(launched_at);
    let after_cutoff =
        |time: std::io::Result<std::time::SystemTime>| time.is_ok_and(|time| time >= cutoff);
    let Ok(dir_meta) = std::fs::symlink_metadata(dir) else {
        return false;
    };
    let Ok(exe_meta) = std::fs::symlink_metadata(dir.join("qdinstall.exe")) else {
        return false;
    };
    dir_meta.is_dir()
        && exe_meta.is_file()
        && (after_cutoff(dir_meta.modified()) || after_cutoff(exe_meta.created()))
        && contains_inf(dir, 4)
}

/// Whether `dir` or a subdirectory up to `depth` levels down holds an `.inf`.
fn contains_inf(dir: &Path, depth: u32) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let path = entry.path();
        match entry.file_type() {
            Ok(kind) if kind.is_file() => path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("inf")),
            Ok(kind) if kind.is_dir() && depth > 0 => contains_inf(&path, depth - 1),
            _ => false,
        }
    })
}

/// UTC .NET ticks for `time`, comparable to `FileSystemInfo.LastWriteTimeUtc.Ticks`.
fn dotnet_utc_ticks(time: std::time::SystemTime) -> i64 {
    let since_epoch = time
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let ticks = since_epoch.as_nanos() / 100;
    DOTNET_TICKS_AT_UNIX_EPOCH.saturating_add(i64::try_from(ticks).unwrap_or(i64::MAX))
}

/// Elevated script that finishes what the installer's `qdinstall.exe -i`
/// would have done, minus `/force`: add and install every INF, rescan
/// devices, and write the same uninstall entry, so the update check and
/// Qualcomm's own uninstaller keep working. The optional WWAN service
/// registration is skipped; EDL does not use it.
///
/// It refuses to act unless the install directory holds a payload extracted
/// by this installer run (same rule as [`payload_is_fresh`], with
/// `launched_at` in .NET UTC ticks). A stale payload from an older run is
/// never installed or registered under the new version.
///
/// Every interpolated value is a constant or digits-and-dots, never a path
/// the user can influence.
fn build_pnputil_fallback_script(fallback: &PnputilFallback, launched_at: i64) -> String {
    let lit = |s: &str| escape_powershell_single_quoted(s);
    let display_name = lit(fallback.display_name);
    let version = lit(&fallback.version);
    let install_dir = lit(QUD_INSTALL_DIR);
    let uninstall_key = lit(QUD_UNINSTALL_KEY);
    let publisher = lit(QUD_PUBLISHER);
    let fresh_after = launched_at.saturating_sub(FRESH_PAYLOAD_SLACK_TICKS);
    format!(
        r#"$ErrorActionPreference = 'Stop'
try {{
    $installPath = Join-Path ([Environment]::GetFolderPath('ProgramFiles')) '{install_dir}'
    $qdinstall = Join-Path $installPath 'qdinstall.exe'
    $dir = Get-Item -LiteralPath $installPath -ErrorAction SilentlyContinue
    if ($null -eq $dir -or -not $dir.PSIsContainer) {{ exit {FALLBACK_EXIT_NO_FRESH_PAYLOAD} }}
    if (-not (Test-Path -LiteralPath $qdinstall -PathType Leaf)) {{ exit {FALLBACK_EXIT_NO_FRESH_PAYLOAD} }}
    $exe = Get-Item -LiteralPath $qdinstall
    if ($dir.LastWriteTimeUtc.Ticks -lt {fresh_after} -and $exe.CreationTimeUtc.Ticks -lt {fresh_after}) {{ exit {FALLBACK_EXIT_NO_FRESH_PAYLOAD} }}
    $infs = @(Get-ChildItem -LiteralPath $installPath -Filter '*.inf' -Recurse -File -ErrorAction SilentlyContinue)
    if ($infs.Count -eq 0) {{ exit {FALLBACK_EXIT_NO_FRESH_PAYLOAD} }}

    $pnputil = Join-Path ([Environment]::SystemDirectory) 'pnputil.exe'
    & $pnputil /add-driver (Join-Path $installPath '*.inf') /subdirs /install | Out-Null
    $pnputilExit = $LASTEXITCODE
    # 3010 = reboot required, 259 = no device needed an update; qdinstall
    # treats both as success.
    if ($pnputilExit -ne 0 -and $pnputilExit -ne 3010 -and $pnputilExit -ne 259) {{ exit $pnputilExit }}
    & $pnputil /scan-devices | Out-Null

    $bytes = (Get-ChildItem -LiteralPath $installPath -Recurse -File | Measure-Object -Property Length -Sum).Sum
    $root = [Microsoft.Win32.RegistryKey]::OpenBaseKey([Microsoft.Win32.RegistryHive]::LocalMachine, [Microsoft.Win32.RegistryView]::Registry64)
    $key = $root.CreateSubKey('{uninstall_key}')
    $key.SetValue('Publisher', '{publisher}', [Microsoft.Win32.RegistryValueKind]::String)
    $key.SetValue('DisplayName', '{display_name}', [Microsoft.Win32.RegistryValueKind]::String)
    $key.SetValue('DisplayVersion', '{version}', [Microsoft.Win32.RegistryValueKind]::String)
    $key.SetValue('InstallLocation', $installPath, [Microsoft.Win32.RegistryValueKind]::String)
    $key.SetValue('UninstallString', ('"' + $qdinstall + '" -x'), [Microsoft.Win32.RegistryValueKind]::String)
    $key.SetValue('NoModify', 1, [Microsoft.Win32.RegistryValueKind]::DWord)
    $key.SetValue('NoRepair', 1, [Microsoft.Win32.RegistryValueKind]::DWord)
    $key.SetValue('EstimatedSize', [int]([int64]$bytes / 1024), [Microsoft.Win32.RegistryValueKind]::DWord)
    $key.Close()
    exit 0
}} catch {{
    exit {FALLBACK_EXIT_SCRIPT_ERROR}
}}
"#
    )
}

/// Run `inner` in an elevated, hidden Windows PowerShell and wait for it.
/// Exit codes follow [`elevated_launcher_script`].
fn run_elevated_powershell(inner: &str) -> Result<std::process::Output> {
    let powershell = windows_powershell_exe()?;
    run_elevated_launcher(&elevated_launcher_script(&powershell, Some(inner))?)
}

/// Launch `exe` through the UAC prompt and wait for it. Exit 1223 is a
/// cancelled prompt; exit 1 with a stderr message is any other launch failure.
fn run_elevated(exe: &Path) -> Result<std::process::Output> {
    run_elevated_launcher(&elevated_launcher_script(exe, None)?)
}

/// Non-elevated PowerShell that starts `file_name` through the UAC prompt and
/// exits with its exit code. With `inner`, `file_name` is PowerShell and runs
/// that script hidden; the script travels as `-EncodedCommand` (encoded by
/// the launcher from a literal here-string), so no quoting crosses the UAC
/// boundary.
///
/// `Process.Start` rather than `Start-Process`: the cmdlet rethrows launch
/// errors as InvalidOperationException and drops the Win32 code, so a UAC
/// cancel could not be told apart from any other failure. Only
/// ERROR_CANCELLED (1223) is a cancel; other launch failures exit 1 with
/// their message on stderr (UTF-8, so localized text survives any code
/// page). A `$null` process (no handle to wait on) is a failure, never a
/// silent success over a driver that did not install.
fn elevated_launcher_script(file_name: &Path, inner: Option<&str>) -> Result<String> {
    // Escape for a PowerShell single-quoted string literal (`'` → `''`).
    let file_name = escape_powershell_single_quoted(&file_name.to_string_lossy());
    let arguments = match inner {
        None => String::new(),
        Some(inner) => {
            // A line starting with `'@` would end the here-string early.
            if inner.lines().any(|line| line.starts_with("'@")) {
                return Err(DriverError::Parse(
                    "elevated script cannot be embedded in a here-string".into(),
                ));
            }
            format!(
                "$inner = @'\n{inner}\n'@\n\
                 $i.Arguments = '-NoProfile -NonInteractive -WindowStyle Hidden -EncodedCommand ' + \
                 [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($inner))\n"
            )
        }
    };
    Ok(format!(
        "[Console]::OutputEncoding=[System.Text.Encoding]::UTF8\n\
         $i = New-Object System.Diagnostics.ProcessStartInfo\n\
         $i.FileName = '{file_name}'\n\
         {arguments}\
         $i.Verb = 'runas'; $i.UseShellExecute = $true\n\
         try {{ $p = [System.Diagnostics.Process]::Start($i) }} \
         catch {{ $e = $_.Exception; while ($null -ne $e) {{ \
         if ($e.NativeErrorCode -eq 1223) {{ exit 1223 }}; $e = $e.InnerException }}; \
         [Console]::Error.Write($_.Exception.Message); exit 1 }}\n\
         if ($null -eq $p) {{ exit 1 }}; $p.WaitForExit(); exit $p.ExitCode\n"
    ))
}

fn run_elevated_launcher(script: &str) -> Result<std::process::Output> {
    silent_command(windows_powershell_exe()?)
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-Command")
        .arg(script)
        .output()
        .map_err(DriverError::Io)
}

/// Stream the installer download with driver-flow log formatting.
fn download_with_progress(
    agent: &ureq::Agent,
    url: &str,
    display_name: &str,
    out_path: &Path,
    log: &mut Vec<String>,
) -> Result<()> {
    use ltbox_core::downloader::{DownloadEvent, stream_with_progress};
    let display_name = display_name.to_string();
    let progress_key = ltbox_core::live_sink::progress_key("driver-download");
    stream_with_progress(agent, url, out_path, log, move |log, event| match event {
        DownloadEvent::Start => {
            live!(
                log,
                "[Driver] {}",
                tr_args!("live_driver_downloading", name = &display_name)
            );
        }
        DownloadEvent::ProgressPct {
            downloaded_mb,
            total_mb,
            pct,
            speed_mbps,
        } => {
            ltbox_core::live_sink::progress(
                &progress_key,
                format!(
                    "[Driver] {}",
                    tr_args!(
                        "live_driver_progress_pct",
                        name = &display_name,
                        pct = pct,
                        downloaded =
                            ltbox_core::log_format::decimal_bytes(downloaded_mb * 1_000_000.0),
                        total = ltbox_core::log_format::decimal_bytes(total_mb * 1_000_000.0),
                        speed = ltbox_core::log_format::decimal_bytes(speed_mbps * 1_000_000.0),
                    )
                ),
            );
        }
        DownloadEvent::ProgressChunked {
            downloaded_mb,
            speed_mbps,
        } => {
            ltbox_core::live_sink::progress(
                &progress_key,
                format!(
                    "[Driver] {}",
                    tr_args!(
                        "live_driver_progress_chunked",
                        name = &display_name,
                        downloaded =
                            ltbox_core::log_format::decimal_bytes(downloaded_mb * 1_000_000.0),
                        speed = ltbox_core::log_format::decimal_bytes(speed_mbps * 1_000_000.0),
                    )
                ),
            );
        }
        DownloadEvent::Done {
            downloaded_mb,
            elapsed_s,
        } => {
            live!(
                log,
                "[Driver] {}",
                tr_args!(
                    "live_driver_dl_done",
                    name = &display_name,
                    size = ltbox_core::log_format::decimal_bytes(downloaded_mb * 1_000_000.0),
                    elapsed = ltbox_core::log_format::elapsed(elapsed_s),
                )
            );
        }
    })
    .map_err(|e| DriverError::Http(format!("download: {e}")))
}

/// Owner-only scratch directory that is removed on drop.
struct PrivateTempDir {
    dir: tempfile::TempDir,
}

impl PrivateTempDir {
    /// Create an exclusive, non-symlink directory under `std::env::temp_dir()`
    /// with a random name. Mirrors the Linux driver installer's private
    /// staging helper so the elevated installer cannot race a pre-planted path.
    fn create(prefix: &str) -> Result<Self> {
        let dir = tempfile::Builder::new()
            .prefix(&format!("{prefix}_"))
            .tempdir()
            .map_err(|e| {
                DriverError::Io(std::io::Error::new(
                    e.kind(),
                    format!("create private temp dir: {e}"),
                ))
            })?;
        verify_private_dir(dir.path())?;
        Ok(Self { dir })
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }
}

fn verify_private_dir(path: &Path) -> Result<()> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| {
        DriverError::Io(std::io::Error::new(
            e.kind(),
            format!("stat private temp dir {}: {e}", path.display()),
        ))
    })?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(DriverError::Io(std::io::Error::other(format!(
            "private temp path {} is not a plain directory",
            path.display()
        ))));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Weekly network smoke test for every installer LTBox may select. It is
    /// deliberately ignored: CI enables it with `weekly_fetch`. Downloaded
    /// installers stay in `tempfile` and only have their Authenticode signer
    /// read — never started — so this test cannot touch the driver store or
    /// hardware. It pins the production signer check to the real certificates.
    #[test]
    #[ignore = "weekly_fetch: downloads published Qualcomm Windows installers"]
    fn weekly_fetch_windows_driver_installers_all_arches() {
        let mut failures = Vec::new();
        for spec in [USERSPACE_SPEC, KERNEL_SPEC] {
            let releases = fetch_win_releases(spec);
            for asset_name in [spec.asset_x64, spec.asset_arm64, spec.asset_x86] {
                let result = (|| -> std::result::Result<(), String> {
                    let releases = releases.as_ref().map_err(|error| error.to_string())?;
                    let release =
                        select_latest_win_release(releases.clone(), asset_name, spec.bundle_stem)
                            .ok_or_else(|| {
                            "production release selector found no matching asset".to_string()
                        })?;
                    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
                    let path = temp.path().join(asset_name);
                    match &release.source {
                        InstallerSource::Direct { url } => {
                            ltbox_core::downloader::download_to_file(url, &path, &mut Vec::new())
                                .map_err(|error| error.to_string())?;
                        }
                        InstallerSource::Bundle { url, zip_name } => {
                            let zip_path = temp.path().join(zip_name);
                            ltbox_core::downloader::download_to_file(
                                url,
                                &zip_path,
                                &mut Vec::new(),
                            )
                            .map_err(|error| error.to_string())?;
                            extract_bundled_installer(&zip_path, asset_name, &path)
                                .map_err(|error| error.to_string())?;
                        }
                    }
                    if std::fs::metadata(&path)
                        .map_err(|error| error.to_string())?
                        .len()
                        == 0
                    {
                        return Err("downloaded installer is empty".to_string());
                    }
                    verify_qualcomm_authenticode(&path, &mut Vec::new())
                        .map_err(|error| error.to_string())?;
                    Ok(())
                })();
                if let Err(error) = result {
                    failures.push(format!("{asset_name}: {error}"));
                }
            }
        }
        assert!(
            failures.is_empty(),
            "weekly Windows driver fetch failures:\n{}",
            failures.join("\n")
        );
    }

    /// `Missing(...)` must never carry an empty vec — empty list
    /// would make the GUI banner say "missing nothing" which is
    /// confusing.
    #[test]
    fn missing_list_is_empty_when_all_present() {
        if let DriverStatus::Missing(list) = check_required_drivers() {
            assert!(!list.is_empty());
        }
    }

    /// Host-arch installer asset name resolves to one of the three
    /// shipped variants.
    #[test]
    fn arch_asset_is_known() {
        for spec in [USERSPACE_SPEC, KERNEL_SPEC] {
            let name = arch_installer_asset(spec);
            assert!(
                matches!(
                    name,
                    "qcom_usb_userspace_drivers_x64.exe"
                        | "qcom_usb_userspace_drivers_arm64.exe"
                        | "qcom_usb_userspace_drivers_x86.exe"
                        | "qcom_usb_kernel_drivers_x64.exe"
                        | "qcom_usb_kernel_drivers_arm64.exe"
                        | "qcom_usb_kernel_drivers_x86.exe"
                ),
                "unexpected asset name: {name}"
            );
        }
    }

    #[test]
    fn select_latest_win_release_ignores_drafts_and_prereleases() {
        // Shared fixture shape for both USERSPACE_SPEC and KERNEL_SPEC asset names.
        for spec in [USERSPACE_SPEC, KERNEL_SPEC] {
            let asset = arch_installer_asset(spec);
            let releases = vec![
                GithubRelease {
                    tag_name: "release-win-v9.9.9.9".into(),
                    assets: vec![GithubAsset {
                        name: asset.to_string(),
                        browser_download_url: "https://example.test/prerelease".into(),
                    }],
                    draft: false,
                    prerelease: true,
                    published_at: "2026-07-20T00:00:00Z".into(),
                },
                GithubRelease {
                    tag_name: "release-win-v8.8.8.8".into(),
                    assets: vec![GithubAsset {
                        name: asset.to_string(),
                        browser_download_url: "https://example.test/draft".into(),
                    }],
                    draft: true,
                    prerelease: false,
                    published_at: "2026-07-19T00:00:00Z".into(),
                },
                GithubRelease {
                    tag_name: "release-win-v1.0.1.0".into(),
                    assets: vec![GithubAsset {
                        name: asset.to_string(),
                        browser_download_url: "https://example.test/older-stable".into(),
                    }],
                    draft: false,
                    prerelease: false,
                    published_at: "2026-07-10T00:00:00Z".into(),
                },
                GithubRelease {
                    tag_name: "release-win-v1.0.2.0".into(),
                    assets: vec![GithubAsset {
                        name: asset.to_string(),
                        browser_download_url: "https://example.test/latest-stable".into(),
                    }],
                    draft: false,
                    prerelease: false,
                    published_at: "2026-07-15T00:00:00Z".into(),
                },
                GithubRelease {
                    tag_name: "release-lnx-v2.0.0.0".into(),
                    assets: vec![GithubAsset {
                        name: asset.to_string(),
                        browser_download_url: "https://example.test/linux-only".into(),
                    }],
                    draft: false,
                    prerelease: false,
                    published_at: "2026-07-18T00:00:00Z".into(),
                },
            ];
            let selected = select_latest_win_release(releases, asset, spec.bundle_stem)
                .expect("stable Windows release with matching asset");
            assert_eq!(selected.tag, "release-win-v1.0.2.0");
            assert_eq!(
                selected.source,
                InstallerSource::Direct {
                    url: "https://example.test/latest-stable".into()
                }
            );
        }
    }

    fn release(tag: &str, published_at: &str, assets: &[&str]) -> GithubRelease {
        GithubRelease {
            tag_name: tag.into(),
            assets: assets
                .iter()
                .map(|name| GithubAsset {
                    name: (*name).into(),
                    browser_download_url: format!("https://example.test/{name}"),
                })
                .collect(),
            draft: false,
            prerelease: false,
            published_at: published_at.into(),
        }
    }

    /// Mirrors the userspace repo as of 2026-11: 1.0.2.3 ships only its
    /// bundle zip, 1.0.2.2 the per-arch installers, and a legacy 1.00.1.6
    /// release an unrelated AnyCPU zip.
    #[test]
    fn select_latest_win_release_falls_back_to_the_bundle_zip() {
        let spec = USERSPACE_SPEC;
        let asset = arch_installer_asset(spec);
        let releases = vec![
            release(
                "release-lnx-v1.00.1.9",
                "2026-09-16T14:36:13Z",
                &["qualcomm-userspace-driver_1.00.1.9_linux-anycpu.zip"],
            ),
            release(
                "release-win-v1.0.2.3",
                "2026-08-04T12:27:07Z",
                &["qcom_usb_userspace_drivers_1.0.2.3.zip"],
            ),
            release(
                "release-win-v1.0.2.2",
                "2026-06-22T19:35:59Z",
                &[
                    "qcom_usb_userspace_drivers_arm64.exe",
                    "qcom_usb_userspace_drivers_x64.exe",
                    "qcom_usb_userspace_drivers_x86.exe",
                    "qud-win-userspace-v1.0.2.2_x86_64_arm64_signed.zip",
                ],
            ),
        ];
        let selected = select_latest_win_release(releases.clone(), asset, spec.bundle_stem)
            .expect("bundle release");
        assert_eq!(selected.tag, "release-win-v1.0.2.3");
        assert_eq!(
            selected.source,
            InstallerSource::Bundle {
                url: "https://example.test/qcom_usb_userspace_drivers_1.0.2.3.zip".into(),
                zip_name: "qcom_usb_userspace_drivers_1.0.2.3.zip".into(),
            }
        );

        // A zip named for another version, or a legacy-format zip, never
        // counts as the bundle; the older direct installer is used instead.
        let mismatched = vec![
            release(
                "release-win-v1.0.2.3",
                "2026-08-04T12:27:07Z",
                &["qcom_usb_userspace_drivers_1.0.2.2.zip"],
            ),
            release(
                "release-win-v1.00.1.6",
                "2026-07-01T00:00:00Z",
                &["Qualcomm_Userspace_Driver.1.00.1.6.Windows-AnyCPU.zip"],
            ),
            releases[2].clone(),
        ];
        let selected =
            select_latest_win_release(mismatched, asset, spec.bundle_stem).expect("direct");
        assert_eq!(selected.tag, "release-win-v1.0.2.2");
        assert!(matches!(selected.source, InstallerSource::Direct { .. }));

        // A release with both keeps using its direct installer.
        let both = vec![release(
            "release-win-v1.0.2.4",
            "2026-12-01T00:00:00Z",
            &[asset, "qcom_usb_userspace_drivers_1.0.2.4.zip"],
        )];
        let selected = select_latest_win_release(both, asset, spec.bundle_stem).expect("both");
        assert!(matches!(selected.source, InstallerSource::Direct { .. }));
    }

    fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, data) in entries {
            zip.start_file(*name, options).unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn extract_bundled_installer_takes_only_the_root_entry() {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("bundle.zip");
        write_zip(
            &zip_path,
            &[
                ("ReleaseNotes.md", b"notes"),
                ("nested/qcom_usb_userspace_drivers_x64.exe", b"nested"),
                ("QCOM_USB_USERSPACE_DRIVERS_X64.EXE", b"root"),
                ("qcom_usb_userspace_drivers_arm64.exe", b"arm"),
            ],
        );
        let out = dir.path().join("x64.exe");
        extract_bundled_installer(&zip_path, "qcom_usb_userspace_drivers_x64.exe", &out).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), b"root");

        // The output is created fresh, never overwritten.
        assert!(
            extract_bundled_installer(&zip_path, "qcom_usb_userspace_drivers_x64.exe", &out)
                .is_err()
        );
        // Missing entry.
        let missing = dir.path().join("x86.exe");
        assert!(matches!(
            extract_bundled_installer(&zip_path, "qcom_usb_userspace_drivers_x86.exe", &missing),
            Err(DriverError::NoAsset)
        ));

        // Two root entries with the same name are ambiguous.
        let dup_path = dir.path().join("dup.zip");
        write_zip(&dup_path, &[("a.exe", b"one"), ("A.EXE", b"two")]);
        assert!(matches!(
            extract_bundled_installer(&dup_path, "a.exe", &dir.path().join("dup.exe")),
            Err(DriverError::Parse(_))
        ));
    }

    #[test]
    fn parse_driver_ver_handles_spacing_and_date() {
        assert_eq!(
            parse_driver_ver("[Version]\nDriverVer = 09/27/2023,1.0.2.0\n").as_deref(),
            Some("1.0.2.0")
        );
        assert_eq!(
            parse_driver_ver("driverver=01/01/2020, 2.0.0.1").as_deref(),
            Some("2.0.0.1")
        );
        // No DriverVer line → None.
        assert_eq!(parse_driver_ver("[Version]\nClass=USB\n"), None);
        // Malformed version components → None (not a truncated value).
        assert_eq!(parse_driver_ver("DriverVer=09/27/2023,1..2"), None);
        assert_eq!(parse_driver_ver("DriverVer=09/27/2023,1."), None);
        assert_eq!(parse_driver_ver("DriverVer=09/27/2023,."), None);
        // Sign-prefixed components are rejected by the digit gate.
        assert_eq!(parse_driver_ver("DriverVer=09/27/2023,+1"), None);
        assert_eq!(parse_driver_ver("DriverVer=09/27/2023,1.+2"), None);
        // Take the version after the FIRST comma — a stray extra comma keeps
        // the bad fragment in a component instead of grabbing a clean tail.
        assert_eq!(parse_driver_ver("DriverVer=09/27/2023,1.+2,1.2"), None);
        // Unbalanced quotes are not trimmed → fail the strict parse.
        assert_eq!(parse_driver_ver("DriverVer=09/27/2023,\"1.2"), None);
        assert_eq!(parse_driver_ver("DriverVer=09/27/2023,1.2\""), None);
        // A balanced quote pair wrapping the whole value is stripped.
        assert_eq!(
            parse_driver_ver("DriverVer=\"09/27/2023,1.0.2.0\"").as_deref(),
            Some("1.0.2.0")
        );
    }

    /// Both Windows driver modes compare installer package versions from the
    /// uninstall registry while required INFs remain the presence signal.
    #[test]
    fn both_specs_use_uninstall_registry() {
        assert_eq!(
            USERSPACE_SPEC.uninstall_display_name,
            Some("Qualcomm USB Userspace Drivers")
        );
        assert_eq!(
            KERNEL_SPEC.uninstall_display_name,
            Some("Qualcomm USB Kernel Drivers")
        );
    }

    #[test]
    fn powershell_single_quote_escape_doubles_quotes() {
        assert_eq!(escape_powershell_single_quoted("plain"), "plain");
        assert_eq!(
            escape_powershell_single_quoted(r"C:\tmp\O'Brien\a.exe"),
            r"C:\tmp\O''Brien\a.exe"
        );
        assert_eq!(escape_powershell_single_quoted("''"), "''''");
    }

    #[test]
    fn signer_subject_requires_a_qualcomm_organization() {
        for subject in [
            "CN=Qualcomm Technologies, Inc., O=Qualcomm Technologies, Inc., L=San Diego, S=California, C=US",
            r#"CN="Qualcomm Technologies, Inc.", O="Qualcomm Technologies, Inc.", L=San Diego, S=California, C=US"#,
            "CN=QUALCOMM INCORPORATED, O=QUALCOMM INCORPORATED, C=US",
            "O=Qualcomm Innovation Center, Inc., C=US",
        ] {
            assert!(signer_subject_is_qualcomm(subject), "{subject}");
        }
        for subject in [
            "",
            "CN=Acme Drivers",
            "CN=Microsoft Windows, O=Microsoft Corporation",
            "CN=QUALCOMM INCORPORATED",
            "CN=Qualcomm Technologies, Inc., O=NotQualcomm Ltd, C=US",
            "CN=x, O=Qualcomm Technologies, Inc. Fan Club, C=US",
            "cn=qualcomm, o=qualcomm",
        ] {
            assert!(!signer_subject_is_qualcomm(subject), "{subject}");
        }
    }

    /// A launch failure that is not a UAC cancel must not read as one. A
    /// missing file fails inside `Process.Start` before any prompt appears.
    #[cfg(windows)]
    #[test]
    fn elevated_launch_failure_is_not_reported_as_cancel() {
        let dir = tempfile::tempdir().unwrap();
        let out = run_elevated(&dir.path().join("missing-installer.exe")).unwrap();
        assert_eq!(out.status.code(), Some(1));
        assert!(!String::from_utf8_lossy(&out.stderr).trim().is_empty());
    }

    fn sample_fallback() -> PnputilFallback {
        PnputilFallback {
            display_name: "Qualcomm USB Kernel Drivers",
            version: "1.00.95.6".into(),
        }
    }

    #[test]
    fn force_probe_needs_the_flag_to_be_the_only_difference() {
        // Current pnputil: usage error with /force, missing path without it.
        assert!(probe_shows_force_rejected(Some(1), Some(2)));
        // Older pnputil that tolerated /force: both reach the missing path.
        assert!(!probe_shows_force_rejected(Some(2), Some(2)));
        // Both exit 1: something other than the flag fails, so no fallback.
        assert!(!probe_shows_force_rejected(Some(1), Some(1)));
        assert!(!probe_shows_force_rejected(Some(1), None));
        assert!(!probe_shows_force_rejected(None, Some(2)));
    }

    #[test]
    fn dotnet_ticks_match_the_dotnet_epoch() {
        assert_eq!(
            dotnet_utc_ticks(std::time::UNIX_EPOCH),
            DOTNET_TICKS_AT_UNIX_EPOCH
        );
        // 2026-11-17T14:22:50Z.
        let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_794_925_370);
        assert_eq!(dotnet_utc_ticks(t), 639_305_221_700_000_000);
    }

    /// The fallback must install without `/force`, refuse a payload older than
    /// this installer run, and register the qdinstall uninstall key.
    #[test]
    fn fallback_script_installs_fresh_payload_without_force() {
        let launched_at = 639_305_221_700_000_000;
        let script = build_pnputil_fallback_script(&sample_fallback(), launched_at);
        let pnputil_lines: Vec<&str> = script
            .lines()
            .filter(|l| l.contains("& $pnputil"))
            .collect();
        assert_eq!(pnputil_lines.len(), 2, "{pnputil_lines:?}");
        assert!(
            pnputil_lines[0].contains("/add-driver")
                && pnputil_lines[0].contains("/subdirs /install")
        );
        assert!(!script.contains("/force"));
        let cutoff = launched_at - FRESH_PAYLOAD_SLACK_TICKS;
        let freshness = format!(
            "$dir.LastWriteTimeUtc.Ticks -lt {cutoff} -and $exe.CreationTimeUtc.Ticks -lt {cutoff}) {{ exit {FALLBACK_EXIT_NO_FRESH_PAYLOAD} }}"
        );
        assert!(script.contains(&freshness), "{script}");
        assert!(script.contains(&format!("'{QUD_UNINSTALL_KEY}'")));
        assert!(script.contains("'DisplayVersion', '1.00.95.6'"));
        assert!(script.contains("'DisplayName', 'Qualcomm USB Kernel Drivers'"));
        assert!(script.contains("('\"' + $qdinstall + '\" -x')"));
        assert!(script.contains("[Microsoft.Win32.RegistryView]::Registry64"));
    }

    #[test]
    fn payload_gate_needs_a_fresh_qdinstall_and_an_inf() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("payload");
        std::fs::create_dir_all(dir.join("drivers")).unwrap();
        let before = std::time::SystemTime::now() - std::time::Duration::from_secs(60);
        let after = std::time::SystemTime::now() + std::time::Duration::from_secs(60);

        // No qdinstall.exe yet.
        std::fs::write(dir.join("drivers").join("qcwdfser.inf"), "x").unwrap();
        assert!(!payload_is_fresh(&dir, before));

        std::fs::write(dir.join("qdinstall.exe"), "x").unwrap();
        assert!(payload_is_fresh(&dir, before));
        // Extracted before this launch: an older run's payload.
        assert!(!payload_is_fresh(&dir, after));

        // No INF anywhere.
        std::fs::remove_file(dir.join("drivers").join("qcwdfser.inf")).unwrap();
        assert!(!payload_is_fresh(&dir, before));
        assert!(!payload_is_fresh(&root.path().join("missing"), before));
    }

    /// Both generated scripts must parse in the PowerShell that runs them.
    #[cfg(windows)]
    #[test]
    fn generated_powershell_scripts_parse() {
        let inner = build_pnputil_fallback_script(&sample_fallback(), 639_305_221_700_000_000);
        let powershell = windows_powershell_exe().unwrap();
        let launcher = elevated_launcher_script(&powershell, Some(&inner)).unwrap();
        let bare = elevated_launcher_script(Path::new(r"C:\tmp\O'Brien\a.exe"), None).unwrap();
        let dir = tempfile::tempdir().unwrap();
        for (name, script) in [("inner", &inner), ("launcher", &launcher), ("bare", &bare)] {
            let path = dir.path().join(format!("{name}.ps1"));
            // BOM so Windows PowerShell reads the file as UTF-8.
            std::fs::write(&path, format!("\u{feff}{script}")).unwrap();
            let check = format!(
                "$e = $null; [void][System.Management.Automation.Language.Parser]::ParseFile('{}', [ref]$null, [ref]$e); if ($e.Count) {{ $e | ForEach-Object {{ [Console]::Error.WriteLine($_.Message) }}; exit 3 }}",
                escape_powershell_single_quoted(&path.to_string_lossy()),
            );
            let out = silent_command(&powershell)
                .args(["-NoProfile", "-NonInteractive", "-Command", &check])
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{name} script failed to parse: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }

    #[test]
    fn launcher_rejects_scripts_that_close_the_here_string() {
        assert!(elevated_launcher_script(Path::new("p.exe"), Some("ok\n'@\nboom")).is_err());
        assert!(elevated_launcher_script(Path::new("p.exe"), Some("x = '@'")).is_ok());
    }

    #[test]
    fn subject_attribute_reads_quoted_and_comma_values() {
        let subject = r#"CN="A, B", O=Org, Inc., OU=Unit, C=US"#;
        assert_eq!(subject_attribute(subject, "CN"), Some("A, B"));
        assert_eq!(subject_attribute(subject, "o"), Some("Org, Inc."));
        assert_eq!(subject_attribute(subject, "OU"), Some("Unit"));
        assert_eq!(subject_attribute(subject, "C"), Some("US"));
        assert_eq!(subject_attribute(subject, "L"), None);
    }

    #[test]
    fn windows_powershell_path_is_system32_host() {
        let path = windows_powershell_exe().expect("resolve system powershell");
        let s = path
            .to_string_lossy()
            .replace('/', "\\")
            .to_ascii_lowercase();
        assert!(
            s.ends_with(r"\system32\windowspowershell\v1.0\powershell.exe"),
            "unexpected powershell path: {s}"
        );
        // Must not be a bare PATH-relative name.
        assert_ne!(path.as_os_str(), OsStr::new("powershell"));
        assert_ne!(path.as_os_str(), OsStr::new("powershell.exe"));
        // Must not depend on a mutable SystemRoot environment value.
        assert!(
            !s.contains("systemroot"),
            "powershell path must not embed SystemRoot env text: {s}"
        );
    }

    #[test]
    fn private_temp_dir_is_exclusive_and_cleaned_on_drop() {
        let dir = PrivateTempDir::create("ltbox_qcom_drv_test").expect("create temp dir");
        let path = dir.path().to_path_buf();
        assert!(path.is_dir());
        assert!(
            path.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("ltbox_qcom_drv_test_"))
        );

        // Two directories created with one prefix never share a path.
        let other = PrivateTempDir::create("ltbox_qcom_drv_test").expect("second dir");
        assert_ne!(other.path(), path);

        drop(dir);
        assert!(
            !path.exists(),
            "PrivateTempDir drop must remove the scratch tree"
        );
    }

    #[test]
    fn private_temp_dir_rejects_non_directory_path() {
        let decoy = tempfile::NamedTempFile::new().expect("create decoy file");
        let err =
            verify_private_dir(decoy.path()).expect_err("file must not pass dir verification");
        assert!(err.to_string().contains("not a plain directory"));
    }
}
