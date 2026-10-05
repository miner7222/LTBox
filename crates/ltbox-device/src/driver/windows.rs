//! Qualcomm 9008 EDL driver detection + auto-install on Windows.
//!
//! Userspace mode requires `qcserlib.inf` from Qualcomm's WinUSB bundle.
//! Kernel mode requires `qcwdfser.inf` from Qualcomm's kernel-driver bundle.
//! Both modes run signed per-arch installers through Windows UAC.

use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::process::Command;

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
    releases_api: &'static str,
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
}

const USERSPACE_SPEC: WindowsDriverSpec = WindowsDriverSpec {
    releases_api: "https://api.github.com/repos/qualcomm/qcom-usb-userspace-drivers/releases?per_page=10",
    required_infs: &["qcserlib.inf"],
    version_inf: "qcserlib.inf",
    // The userspace package can advance independently of `qcserlib.inf`
    // (package/release 1.0.2.2 still ships INF DriverVer 1.0.2.1). Read the
    // installer's package version to avoid a perpetual false update banner.
    uninstall_display_name: Some("Qualcomm USB Userspace Drivers"),
    asset_x64: "qcom_usb_userspace_drivers_x64.exe",
    asset_arm64: "qcom_usb_userspace_drivers_arm64.exe",
    asset_x86: "qcom_usb_userspace_drivers_x86.exe",
};

const KERNEL_SPEC: WindowsDriverSpec = WindowsDriverSpec {
    releases_api: "https://api.github.com/repos/qualcomm/qcom-usb-kernel-drivers/releases?per_page=10",
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
};

fn driver_spec(mode: QcomDriverMode) -> WindowsDriverSpec {
    match mode {
        QcomDriverMode::Userspace => USERSPACE_SPEC,
        QcomDriverMode::Kernel => KERNEL_SPEC,
    }
}

#[derive(Debug, serde::Deserialize)]
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

#[derive(Debug, serde::Deserialize)]
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

/// Fetch the latest Windows release that ships the host-arch installer,
/// excluding drafts and prereleases, returning `(tag_name, asset_download_url)`.
/// Shared by the installer and the update check so both resolve to the same release.
fn fetch_latest_win_release() -> Result<(String, String)> {
    let spec = driver_spec(qcom_driver_mode());
    let meta_agent = ltbox_core::downloader::build_agent();

    let releases: Vec<GithubRelease> = meta_agent
        .get(spec.releases_api)
        .call()?
        .body_mut()
        .read_json()
        .map_err(|e| DriverError::Parse(e.to_string()))?;

    let asset_name = arch_installer_asset(spec);
    let release = select_latest_win_release(releases, asset_name).ok_or(DriverError::NoAsset)?;

    let tag = release.tag_name.clone();
    let asset_url = release
        .assets
        .into_iter()
        .find(|a| a.name.eq_ignore_ascii_case(asset_name))
        .map(|a| a.browser_download_url)
        .ok_or(DriverError::NoAsset)?;
    Ok((tag, asset_url))
}

/// Pick the newest published stable Windows release that carries `asset_name`.
/// Drafts and prereleases are rejected before tag/asset matching; among the
/// remaining candidates the highest `published_at` wins.
fn select_latest_win_release(
    releases: Vec<GithubRelease>,
    asset_name: &str,
) -> Option<GithubRelease> {
    let mut matching: Vec<GithubRelease> = releases
        .into_iter()
        .filter(|r| !r.draft && !r.prerelease)
        .filter(|r| r.tag_name.to_ascii_lowercase().contains(WIN_TAG_NEEDLE))
        .filter(|r| {
            r.assets
                .iter()
                .any(|a| a.name.eq_ignore_ascii_case(asset_name))
        })
        .collect();
    matching.sort_unstable_by(|a, b| b.published_at.cmp(&a.published_at));
    matching.into_iter().next()
}

/// Check whether a newer signed driver release exists than the one
/// installed. Returns `Some` only when a driver is present locally AND the
/// latest Windows release is strictly newer. Any failure — no driver
/// installed, version unparseable, offline, GitHub unreachable — collapses
/// to `None` so the caller can fail silently (no banner).
pub fn check_driver_update() -> Option<DriverUpdate> {
    let current = installed_driver_version()?;
    let (tag, _url) = fetch_latest_win_release().ok()?;
    let latest = version_from_tag(&tag)?;
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
    let asset_name = arch_installer_asset(driver_spec(qcom_driver_mode()));
    let (_tag, asset_url) = fetch_latest_win_release()?;

    live!(
        log,
        "[Driver] {}",
        tr_args!("live_driver_asset", name = asset_name)
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
        download_with_progress(&dl_agent, &asset_url, asset_name, &exe_path, log)?;
        verify_qualcomm_authenticode(&exe_path, log)?;
        live!(log, "[Driver] {}", tr("live_driver_running_installer"));
        run_installer_elevated(&exe_path, log)?;
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

/// Run the signed installer through UAC and map cancel vs failure.
fn run_installer_elevated(exe: &Path, log: &mut Vec<String>) -> Result<()> {
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
            live!(
                log,
                "[Driver] {}",
                tr_args!("live_driver_installer_failed", exit = other)
            );
            Err(DriverError::InstallerFailed { exit_code: other })
        }
    }
}

/// Launch `exe` through the UAC prompt and wait for it. Exit 1223 is a
/// cancelled prompt; exit 1 with a stderr message is any other launch failure.
fn run_elevated(exe: &Path) -> Result<std::process::Output> {
    // Escape for a PowerShell single-quoted string literal (`'` → `''`).
    // The temp path is process-id-derived so quotes are not expected, but
    // escape defensively rather than trust the environment.
    let exe_str = escape_powershell_single_quoted(&exe.to_string_lossy());
    // `Process.Start` rather than `Start-Process`: the cmdlet rethrows launch
    // errors as InvalidOperationException and drops the Win32 code, so a UAC
    // cancel could not be told apart from any other failure. Only
    // ERROR_CANCELLED (1223) is a cancel; other launch failures exit 1 with
    // their message on stderr (UTF-8, so localized text survives any code
    // page). A `$null` process (no handle to wait on) is a failure, never a
    // silent success over a driver that did not install.
    let script = format!(
        "[Console]::OutputEncoding=[System.Text.Encoding]::UTF8; \
         $i = New-Object System.Diagnostics.ProcessStartInfo; \
         $i.FileName = '{exe_str}'; $i.Verb = 'runas'; $i.UseShellExecute = $true; \
         try {{ $p = [System.Diagnostics.Process]::Start($i) }} \
         catch {{ $e = $_.Exception; while ($null -ne $e) {{ \
         if ($e.NativeErrorCode -eq 1223) {{ exit 1223 }}; $e = $e.InnerException }}; \
         [Console]::Error.Write($_.Exception.Message); exit 1 }}; \
         if ($null -eq $p) {{ exit 1 }}; $p.WaitForExit(); exit $p.ExitCode"
    );

    silent_command(windows_powershell_exe()?)
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-Command")
        .arg(&script)
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
            for asset_name in [spec.asset_x64, spec.asset_arm64, spec.asset_x86] {
                let result = (|| -> std::result::Result<(), String> {
                    let releases: Vec<GithubRelease> = ltbox_core::downloader::build_agent()
                        .get(spec.releases_api)
                        .call()
                        .map_err(|error| error.to_string())?
                        .body_mut()
                        .read_json()
                        .map_err(|error| error.to_string())?;
                    let release =
                        select_latest_win_release(releases, asset_name).ok_or_else(|| {
                            "production release selector found no matching asset".to_string()
                        })?;
                    let url = release
                        .assets
                        .iter()
                        .find(|asset| asset.name.eq_ignore_ascii_case(asset_name))
                        .map(|asset| asset.browser_download_url.as_str())
                        .ok_or_else(|| {
                            "selected release omitted its matching asset URL".to_string()
                        })?;
                    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
                    let path = temp.path().join(asset_name);
                    ltbox_core::downloader::download_to_file(url, &path, &mut Vec::new())
                        .map_err(|error| error.to_string())?;
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
            let selected = select_latest_win_release(releases, asset)
                .expect("stable Windows release with matching asset");
            assert_eq!(selected.tag_name, "release-win-v1.0.2.0");
            assert_eq!(
                selected.assets[0].browser_download_url,
                "https://example.test/latest-stable"
            );
        }
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
