//! Boot image patching — wraps magiskboot for root operations.

use fs_err as fs;
use std::path::Path;

use ltbox_core::{LtboxError, Result};

/// Return the top-level ramdisk produced by unpacking boot or init_boot.
pub(crate) fn root_ramdisk_name(work_dir: &Path) -> Result<&'static str> {
    const RAMDISK: &str = "ramdisk.cpio";
    if work_dir.join(RAMDISK).is_file() {
        Ok(RAMDISK)
    } else {
        Err(LtboxError::Patch(
            "ramdisk.cpio not found after unpack".into(),
        ))
    }
}

/// Unpack a boot image into components. Non-zero magiskboot exit becomes `Err`.
pub fn unpack(image: &Path, work_dir: &Path) -> Result<()> {
    let name = image
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("boot.img");
    let dst = work_dir.join(name);
    if image != dst {
        fs::copy(image, &dst).map_err(|e| LtboxError::BootImage(e.to_string()))?;
    }
    let code = run_magiskboot(work_dir, &["unpack", name])?;
    if code == 0 {
        Ok(())
    } else {
        check_magiskboot("unpack", code)
    }
}

/// Repack boot image from components. Non-zero magiskboot exit becomes `Err`.
pub fn repack(orig_image: &str, work_dir: &Path) -> Result<()> {
    check_magiskboot("repack", run_magiskboot(work_dir, &["repack", orig_image])?)
}

/// CPIO operations on ramdisk. Raw exit code — caller decides what's an error.
/// Use [`cpio_checked`] for mutating commands where non-zero means failure.
/// Leave this untouched for `test` / `exists` whose rc is a status flag.
pub fn cpio(work_dir: &Path, cpio_file: &str, commands: &[&str]) -> Result<i32> {
    let mut args = vec!["cpio", cpio_file];
    args.extend_from_slice(commands);
    run_magiskboot(work_dir, &args)
}

/// CPIO operations that must succeed — non-zero magiskboot exit becomes `Err`.
/// Use for `add`, `mv`, `mkdir`, `backup`, `patch`, etc. where any failure
/// leaves the ramdisk half-patched and the repack unsafe to ship.
pub fn cpio_checked(work_dir: &Path, cpio_file: &str, commands: &[&str]) -> Result<()> {
    check_magiskboot(
        &format!("cpio {}", commands.join(" ")),
        cpio(work_dir, cpio_file, commands)?,
    )
}

/// CPIO operations with extra env vars set for the duration of the call.
///
/// Required for `cpio … patch` on ramdisk-root flows: magiskboot reads
/// `KEEPVERITY` / `KEEPFORCEENCRYPT` from the process env, and without them
/// *strips* dm-verity and forceencrypt fstab flags. The overrides apply only to
/// the isolated child process.
///
/// Always checked: a non-zero rc from `patch` means there is nothing to repack.
pub fn cpio_with_env(
    work_dir: &Path,
    cpio_file: &str,
    commands: &[&str],
    envs: &[(&str, &str)],
) -> Result<()> {
    let mut args = vec!["cpio", cpio_file];
    args.extend_from_slice(commands);
    check_magiskboot(
        &format!("cpio {}", commands.join(" ")),
        run_magiskboot_with_env(work_dir, &args, envs)?,
    )
}

/// Map magiskboot exit code to a `Result`. Exit 0 = success; anything else
/// surfaces as `LtboxError::BootImage` with the operation label.
fn check_magiskboot(op: &str, code: i32) -> Result<()> {
    if code == 0 {
        Ok(())
    } else {
        Err(LtboxError::BootImage(format!(
            "magiskboot {op} failed (exit={code})"
        )))
    }
}

/// SHA1 hash of a file (computed in Rust, no magiskboot needed).
pub fn sha1(file_path: &Path) -> Result<String> {
    let data = fs::read(file_path).map_err(|e| LtboxError::BootImage(e.to_string()))?;
    Ok(sha1_hash(&data))
}

/// Compress a file. Non-zero magiskboot exit becomes `Err`.
pub fn compress(work_dir: &Path, format: &str, input: &str, output: &str) -> Result<()> {
    check_magiskboot(
        "compress",
        run_magiskboot(work_dir, &[&format!("compress={format}"), input, output])?,
    )
}

/// Detect and decompress a file. Non-zero magiskboot exit becomes `Err`.
pub fn decompress(work_dir: &Path, input: &str, output: &str) -> Result<()> {
    check_magiskboot(
        "decompress",
        run_magiskboot(work_dir, &["decompress", input, output])?,
    )
}

/// Hosts can reuse their executable as the isolated magiskboot child. Call
/// `dispatch_magiskboot_helper` at the very start of main, before GUI/runtime
/// initialization, and register that executable for subsequent patch calls.
static MAGISKBOOT_HOST: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
const HELPER_FLAG: &str = "--ltbox-internal-magiskboot";

pub fn register_magiskboot_host(path: std::path::PathBuf) -> Result<()> {
    MAGISKBOOT_HOST
        .set(path)
        .map_err(|_| LtboxError::BootImage("magiskboot host already registered".into()))
}

/// Returns `None` for ordinary application invocations. The child inherits its
/// working directory and patch flags from `Command`, never changing the host.
pub fn dispatch_magiskboot_helper() -> Option<i32> {
    // `args_os`: this runs on every launch, and `args` panics on a
    // non-Unicode argument before the app has started.
    let mut args = std::env::args_os();
    args.next();
    if args.next().as_deref() != Some(std::ffi::OsStr::new(HELPER_FLAG)) {
        return None;
    }
    let Ok(helper_args) = args
        .map(std::ffi::OsString::into_string)
        .collect::<std::result::Result<Vec<_>, _>>()
    else {
        eprintln!("magiskboot helper: arguments must be valid Unicode");
        return Some(1);
    };
    let full_args = std::iter::once("magiskboot".to_owned())
        .chain(helper_args)
        .collect();
    let cmds = magiskboot::base::CmdArgs::from_env_args(full_args);
    Some(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            magiskboot::cli::boot_main(cmds).unwrap_or(1)
        }))
        .unwrap_or(1),
    )
}

fn run_magiskboot(work_dir: &Path, args: &[&str]) -> Result<i32> {
    run_magiskboot_with_env(work_dir, args, &[])
}

fn run_magiskboot_with_env(work_dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> Result<i32> {
    let executable = match MAGISKBOOT_HOST.get() {
        Some(path) => path.clone(),
        None => {
            // Standalone library tests/consumers may install the companion
            // binary next to their executable (Cargo tests live under deps/).
            let current = std::env::current_exe()?;
            let mut directory = current
                .parent()
                .ok_or_else(|| LtboxError::BootImage("No executable directory".into()))?;
            if directory.file_name().is_some_and(|name| name == "deps") {
                directory = directory.parent().unwrap_or(directory);
            }
            directory.join(format!("ltbox-magiskboot{}", std::env::consts::EXE_SUFFIX))
        }
    };
    let mut command = std::process::Command::new(executable);
    command
        .arg(HELPER_FLAG)
        .args(args)
        .current_dir(work_dir)
        .envs(envs.iter().copied())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let status = command.status().map_err(|e| {
        LtboxError::BootImage(format!("Cannot run isolated magiskboot helper: {e}"))
    })?;
    status.code().ok_or_else(|| {
        LtboxError::BootImage("magiskboot helper terminated without an exit code".into())
    })
}

fn sha1_hash(data: &[u8]) -> String {
    use digest::Digest;
    let mut h = sha1::Sha1::new();
    h.update(data);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_ramdisk_name_uses_top_level_cpio() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("ramdisk.cpio"), b"top-level").unwrap();
        assert_eq!(root_ramdisk_name(temp.path()).unwrap(), "ramdisk.cpio");
    }

    #[test]
    fn root_ramdisk_name_rejects_missing_ramdisk() {
        let temp = tempfile::tempdir().unwrap();
        assert!(root_ramdisk_name(temp.path()).is_err());
    }
}
