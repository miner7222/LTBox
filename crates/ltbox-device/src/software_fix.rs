//! Native Windows process detection and explicit close support for Software Fix.
//!
//! Polling uses the Tool Help process snapshot API, so it does not launch a
//! shell or inspect child processes. Closing is an explicit elevated action:
//! the embedded PowerShell script targets only the `Software Fix` process
//! name, then this module verifies that no matching executable remains.

#[cfg(any(windows, test))]
const TARGET_EXE_BASENAME: &str = "Software Fix.exe";

/// The only process operation allowed by [`force_close`]. Keep this embedded
/// so no user-controlled or temporary script can be substituted.
#[cfg(any(windows, test))]
const FORCE_CLOSE_INNER_SCRIPT: &str = r#"$ErrorActionPreference = 'Stop'
$processes = Get-Process -Name 'Software Fix' -ErrorAction SilentlyContinue
if ($null -ne $processes) {
    $processes | Stop-Process -Force -ErrorAction Stop
}
"#;

/// Escape a value for embedding inside a PowerShell single-quoted string
/// literal (`'` -> `''`). Callers must still wrap the result in `'...'`.
#[cfg(windows)]
pub(crate) fn escape_powershell_single_quoted(value: &str) -> String {
    value.replace('\'', "''")
}

/// Failure while explicitly closing Software Fix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloseError {
    /// The user cancelled the Windows elevation prompt.
    Cancelled,
    /// The elevated close operation failed for another reason.
    Failed(String),
}

/// Match the exact executable basename reported by Windows.
///
/// `PROCESSENTRY32W::szExeFile` contains the executable basename, including
/// `.exe`; matching the whole string prevents similarly named processes such
/// as `RSA Software Fix.exe` from being selected.
#[cfg(any(windows, test))]
fn matches_exe_basename(exe_name: &str) -> bool {
    exe_name.eq_ignore_ascii_case(TARGET_EXE_BASENAME)
}

/// Check whether an exact `Software Fix.exe` process is running.
#[cfg(windows)]
pub fn is_running() -> Result<bool, String> {
    windows::is_running()
}

/// Non-Windows hosts do not run the Windows Software Fix process.
#[cfg(not(windows))]
pub fn is_running() -> Result<bool, String> {
    Ok(false)
}

/// Ask Windows UAC for permission to stop every exact `Software Fix.exe`
/// process, then verify that no matching process remains.
#[cfg(windows)]
pub fn force_close() -> Result<(), CloseError> {
    windows::force_close()
}

/// Software Fix is only available on Windows.
#[cfg(not(windows))]
pub fn force_close() -> Result<(), CloseError> {
    Err(CloseError::Failed(
        "closing Software Fix is unsupported on this platform".to_string(),
    ))
}

#[cfg(windows)]
mod windows {
    use super::{FORCE_CLOSE_INNER_SCRIPT, escape_powershell_single_quoted, matches_exe_basename};
    use std::ffi::{OsStr, OsString};
    use std::io;
    use std::os::windows::ffi::OsStringExt;
    use std::os::windows::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};

    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_NO_MORE_FILES, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const UAC_CANCEL_EXIT_CODE: i32 = 1223;

    /// A native process snapshot handle which is closed on every return path.
    struct Snapshot(HANDLE);

    impl Drop for Snapshot {
        fn drop(&mut self) {
            // SAFETY: `self.0` was returned by CreateToolhelp32Snapshot and is
            // owned by this guard until Drop.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    /// `Command::new` with no console window.
    fn silent_command(program: impl AsRef<OsStr>) -> Command {
        let mut command = Command::new(program);
        command.creation_flags(CREATE_NO_WINDOW);
        command
    }

    /// Check the process executable basename with the native Tool Help API.
    pub(super) fn is_running() -> Result<bool, String> {
        let handle = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if handle == INVALID_HANDLE_VALUE {
            return Err(format!(
                "CreateToolhelp32Snapshot failed: {}",
                io::Error::last_os_error()
            ));
        }
        let snapshot = Snapshot(handle);

        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let first = unsafe { Process32FirstW(snapshot.0, &mut entry) };
        if first == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_NO_MORE_FILES as i32) {
                return Ok(false);
            }
            return Err(format!("Process32FirstW failed: {error}"));
        }

        loop {
            if entry_matches_target(&entry) {
                return Ok(true);
            }

            let next = unsafe { Process32NextW(snapshot.0, &mut entry) };
            if next != 0 {
                continue;
            }

            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_NO_MORE_FILES as i32) {
                return Ok(false);
            }
            return Err(format!("Process32NextW failed: {error}"));
        }
    }

    fn entry_matches_target(entry: &PROCESSENTRY32W) -> bool {
        let end = entry
            .szExeFile
            .iter()
            .position(|&unit| unit == 0)
            .unwrap_or(entry.szExeFile.len());
        let exe_name = String::from_utf16_lossy(&entry.szExeFile[..end]);
        matches_exe_basename(&exe_name)
    }

    /// Resolve the trusted system PowerShell executable without consulting PATH.
    fn windows_powershell_exe() -> Result<PathBuf, String> {
        let system32 = windows_system32_dir()?;
        Ok(system32
            .join("WindowsPowerShell")
            .join("v1.0")
            .join("powershell.exe"))
    }

    /// Resolve System32 through Win32 rather than the mutable SystemRoot env var.
    fn windows_system32_dir() -> Result<PathBuf, String> {
        let needed = unsafe { GetSystemDirectoryW(std::ptr::null_mut(), 0) };
        if needed == 0 {
            return Err(format!(
                "GetSystemDirectoryW failed: {}",
                io::Error::last_os_error()
            ));
        }

        let mut buffer = vec![0u16; needed as usize];
        let written = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), needed) };
        if written == 0 || written >= needed {
            let error = if written == 0 {
                io::Error::last_os_error().to_string()
            } else {
                "GetSystemDirectoryW returned a path larger than its buffer".to_string()
            };
            return Err(format!("GetSystemDirectoryW failed: {error}"));
        }

        let path = OsString::from_wide(&buffer[..written as usize]);
        if path.is_empty() {
            return Err("GetSystemDirectoryW returned an empty path".to_string());
        }
        Ok(PathBuf::from(path))
    }

    /// Build the fixed outer script that elevates and runs the embedded script.
    fn build_elevation_script(power_shell_path: &Path) -> String {
        let power_shell_path = escape_powershell_single_quoted(&power_shell_path.to_string_lossy());
        format!(
            r#"$ErrorActionPreference = 'Stop'
$inner = @'
{FORCE_CLOSE_INNER_SCRIPT}'@
$encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($inner))
try {{
    $child = Start-Process -FilePath '{power_shell_path}' -Verb RunAs -WindowStyle Hidden -Wait -PassThru -ArgumentList @(
        '-NoProfile',
        '-NonInteractive',
        '-WindowStyle',
        'Hidden',
        '-EncodedCommand',
        $encoded
    )
    if ($null -eq $child -or $null -eq $child.ExitCode) {{
        exit 1
    }}
    exit $child.ExitCode
}} catch {{
    $hresult = $_.Exception.HResult
    $native_error_code = $_.Exception.NativeErrorCode
    $inner_hresult = $_.Exception.InnerException.HResult
    $inner_native_error_code = $_.Exception.InnerException.NativeErrorCode
    if (($hresult -band 0xFFFF) -eq 1223 -or $native_error_code -eq 1223 -or
        ($inner_hresult -band 0xFFFF) -eq 1223 -or $inner_native_error_code -eq 1223) {{
        exit 1223
    }}
    [Console]::Error.Write($_.Exception.Message)
    exit 1
}}"#
        )
    }

    /// Classify the outer PowerShell process exit code.
    fn classify_exit_code(code: Option<i32>) -> Result<(), super::CloseError> {
        match code {
            Some(0) => Ok(()),
            Some(UAC_CANCEL_EXIT_CODE) => Err(super::CloseError::Cancelled),
            Some(code) => Err(super::CloseError::Failed(format!(
                "elevated PowerShell exited with code {code}"
            ))),
            None => Err(super::CloseError::Failed(
                "elevated PowerShell terminated without an exit code".to_string(),
            )),
        }
    }

    fn append_stderr(error: super::CloseError, output: &Output) -> super::CloseError {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            return error;
        }
        match error {
            super::CloseError::Failed(message) => {
                super::CloseError::Failed(format!("{message}: {stderr}"))
            }
            cancelled => cancelled,
        }
    }

    pub(super) fn force_close() -> Result<(), super::CloseError> {
        // Avoid prompting for elevation when the UI observed a stale running
        // state. The elevated script still performs the authoritative close
        // if the process starts after this read.
        match is_running() {
            Ok(false) => return Ok(()),
            Ok(true) => {}
            Err(error) => {
                return Err(super::CloseError::Failed(format!(
                    "could not check whether Software Fix is running: {error}"
                )));
            }
        }

        let power_shell_path = windows_powershell_exe().map_err(super::CloseError::Failed)?;
        let script = build_elevation_script(&power_shell_path);
        let output = silent_command(&power_shell_path)
            .arg("-NoProfile")
            .arg("-NonInteractive")
            .arg("-WindowStyle")
            .arg("Hidden")
            .arg("-Command")
            .arg(script)
            .output()
            .map_err(|error| {
                super::CloseError::Failed(format!("failed to start elevated PowerShell: {error}"))
            })?;

        if let Err(error) = classify_exit_code(output.status.code()) {
            return Err(append_stderr(error, &output));
        }

        match is_running() {
            Ok(false) => Ok(()),
            Ok(true) => Err(super::CloseError::Failed(
                "Software Fix is still running after the elevated close operation".to_string(),
            )),
            Err(error) => Err(super::CloseError::Failed(format!(
                "could not verify Software Fix was closed: {error}"
            ))),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::{FORCE_CLOSE_INNER_SCRIPT, classify_exit_code, matches_exe_basename};
        use crate::software_fix::CloseError;

        #[test]
        fn exact_executable_matching_is_case_insensitive() {
            assert!(matches_exe_basename("Software Fix.exe"));
            assert!(matches_exe_basename("software fix.EXE"));
            assert!(!matches_exe_basename("Software Fix"));
            assert!(!matches_exe_basename("RSA Software Fix.exe"));
            assert!(!matches_exe_basename("Software Fix.exe.bak"));
        }

        #[test]
        fn inner_script_targets_only_software_fix() {
            assert!(FORCE_CLOSE_INNER_SCRIPT.contains("Get-Process -Name 'Software Fix'"));
            assert!(FORCE_CLOSE_INNER_SCRIPT.contains("Stop-Process -Force"));
            assert!(
                !FORCE_CLOSE_INNER_SCRIPT
                    .to_ascii_lowercase()
                    .contains("adb")
            );
            assert!(
                !FORCE_CLOSE_INNER_SCRIPT
                    .to_ascii_lowercase()
                    .contains("service")
            );
            assert!(
                !FORCE_CLOSE_INNER_SCRIPT
                    .to_ascii_lowercase()
                    .contains("tree")
            );
        }

        #[test]
        fn exit_code_1223_is_the_only_uac_cancel_classification() {
            assert_eq!(classify_exit_code(Some(1223)), Err(CloseError::Cancelled));
            assert_eq!(
                classify_exit_code(Some(1)),
                Err(CloseError::Failed(
                    "elevated PowerShell exited with code 1".to_string(),
                ))
            );
            assert_eq!(
                classify_exit_code(None),
                Err(CloseError::Failed(
                    "elevated PowerShell terminated without an exit code".to_string(),
                ))
            );
            assert_eq!(classify_exit_code(Some(0)), Ok(()));
        }

        #[test]
        fn native_process_probe_is_read_only() {
            assert!(super::is_running().is_ok());
        }

        #[test]
        fn generated_elevation_script_parses_without_execution() {
            use std::io::Write;
            use std::process::Stdio;
            let power_shell_path = super::windows_powershell_exe().expect("resolve PowerShell");
            let script = super::build_elevation_script(&power_shell_path);
            let parser = r#"$tokens = $null
$errors = $null
[System.Management.Automation.Language.Parser]::ParseInput([Console]::In.ReadToEnd(), [ref]$tokens, [ref]$errors) | Out-Null
if ($errors.Count -ne 0) {
    $errors | ForEach-Object { [Console]::Error.WriteLine($_.ToString()) }
    exit 1
}
exit 0"#;
            let mut child = super::silent_command(&power_shell_path)
                .args(["-NoProfile", "-NonInteractive", "-Command", parser])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("start parser-only PowerShell");
            child
                .stdin
                .take()
                .expect("parser stdin")
                .write_all(script.as_bytes())
                .expect("write script to parser");
            let output = child.wait_with_output().expect("wait for parser");
            assert!(
                output.status.success(),
                "generated elevation script did not parse: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}

#[cfg(all(test, not(windows)))]
mod tests {
    use super::{CloseError, FORCE_CLOSE_INNER_SCRIPT, force_close, matches_exe_basename};

    #[test]
    fn exact_executable_matching_is_case_insensitive() {
        assert!(matches_exe_basename("Software Fix.exe"));
        assert!(matches_exe_basename("software fix.EXE"));
        assert!(!matches_exe_basename("Software Fix"));
        assert!(!matches_exe_basename("RSA Software Fix.exe"));
        assert!(!matches_exe_basename("Software Fix.exe.bak"));
    }

    #[test]
    fn inner_script_targets_only_software_fix() {
        assert!(FORCE_CLOSE_INNER_SCRIPT.contains("Get-Process -Name 'Software Fix'"));
        assert!(FORCE_CLOSE_INNER_SCRIPT.contains("Stop-Process -Force"));
        assert!(
            !FORCE_CLOSE_INNER_SCRIPT
                .to_ascii_lowercase()
                .contains("adb")
        );
        assert!(
            !FORCE_CLOSE_INNER_SCRIPT
                .to_ascii_lowercase()
                .contains("service")
        );
        assert!(
            !FORCE_CLOSE_INNER_SCRIPT
                .to_ascii_lowercase()
                .contains("tree")
        );
    }

    #[test]
    fn force_close_reports_unsupported_platform_without_spawning() {
        assert_eq!(
            force_close(),
            Err(CloseError::Failed(
                "closing Software Fix is unsupported on this platform".to_string(),
            ))
        );
    }
}
