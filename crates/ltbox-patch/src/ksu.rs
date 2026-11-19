//! KernelSU LKM patching — replaces `init` in the selected root ramdisk with
//! KernelSU's bootstrap binary and stages `kernelsu.ko` so the stock kernel
//! loads the module at boot. Works for KernelSU, KSU-Next, and forks.

use std::path::{Path, PathBuf};

use ltbox_core::i18n::tr;
use ltbox_core::{LtboxError, Result, tr_args};

use crate::boot;
use crate::root_pipeline::RootImageTarget;

/// Map the pre-patch ramdisk probes to a refusal. Each argument is a
/// `magiskboot cpio` exit status: `test_status` 0=stock, 1=Magisk-patched,
/// anything else unsupported; `real_exists`/`ko_exists` 0 means the entry is
/// present (`init.real` / `kernelsu.ko` exist only after a prior KSU run).
fn check_ksu_input(
    img_name: &str,
    test_status: i32,
    real_exists: i32,
    ko_exists: i32,
) -> Result<()> {
    match test_status {
        0 => {}
        1 => {
            return Err(LtboxError::Patch(format!(
                "{img_name} is already Magisk-patched — flash stock first"
            )));
        }
        other => {
            return Err(LtboxError::Patch(format!(
                "Unsupported boot image layout (cpio test = {other})"
            )));
        }
    }
    if real_exists == 0 || ko_exists == 0 {
        return Err(LtboxError::Patch(format!(
            "{img_name} is already KernelSU-patched — flash stock first"
        )));
    }
    Ok(())
}

/// Patch the selected root image with KernelSU. `work_dir` must contain the
/// image, `init` (ksuinit), and `kernelsu.ko`.
/// Writes `work_dir/new-boot.img`; caller handles AVB resign + flash.
pub fn patch_root_image(
    work_dir: &Path,
    target: RootImageTarget,
    log: &mut Vec<String>,
) -> Result<PathBuf> {
    let img_name = target.filename();
    let img_path = work_dir.join(img_name);
    if !img_path.exists() {
        return Err(LtboxError::Patch(format!(
            "{img_name} not found in {}",
            work_dir.display()
        )));
    }
    for needed in ["init", "kernelsu.ko"] {
        if !work_dir.join(needed).exists() {
            return Err(LtboxError::Patch(format!(
                "KSU payload '{needed}' missing from {}",
                work_dir.display()
            )));
        }
    }

    ltbox_core::live_debug!(
        log,
        "[KSU] {}",
        tr_args!("log_root_unpack_image", image = img_name)
    );
    boot::unpack(&img_path, work_dir)?;

    let ramdisk_name = boot::root_ramdisk_name(work_dir)?;

    // Same guards as upstream ksud: refuse Magisk-patched or unsupported
    // ramdisks, and refuse to double-patch.
    let test_status = boot::cpio(work_dir, ramdisk_name, &["test"])?;
    let real_exists = boot::cpio(work_dir, ramdisk_name, &["exists init.real"])?;
    let ko_exists = boot::cpio(work_dir, ramdisk_name, &["exists kernelsu.ko"])?;
    check_ksu_input(img_name, test_status, real_exists, ko_exists)?;

    // Move stock init → init.real so ksuinit can chain to it. Loose-ramdisk
    // images have no top-level init, so skip the rename there.
    let has_init = boot::cpio(work_dir, ramdisk_name, &["exists init"])?;
    if has_init == 0 {
        ltbox_core::live_debug!(log, "[KSU] {}", tr("log_ksu_cpio_mv_init"));
        boot::cpio_checked(work_dir, ramdisk_name, &["mv init init.real"])?;
    } else {
        ltbox_core::live!(log, "[KSU] {}", tr("log_ksu_no_stock_init"));
    }

    ltbox_core::live_debug!(log, "[KSU] {}", tr("log_ksu_cpio_add"));
    boot::cpio_checked(work_dir, ramdisk_name, &["add 0755 init init"])?;
    boot::cpio_checked(
        work_dir,
        ramdisk_name,
        &["add 0755 kernelsu.ko kernelsu.ko"],
    )?;

    ltbox_core::live_debug!(
        log,
        "[KSU] {}",
        tr_args!("log_root_repack_image", image = img_name)
    );
    boot::repack(img_name, work_dir)?;

    let new_boot = work_dir.join("new-boot.img");
    if !new_boot.exists() {
        return Err(LtboxError::Patch(
            "magiskboot repack produced no new-boot.img".into(),
        ));
    }
    ltbox_core::live!(log, "[KSU] {}", tr("log_patch_complete"));
    Ok(new_boot)
}

#[cfg(test)]
mod tests {
    use super::check_ksu_input;

    fn msg(r: ltbox_core::Result<()>) -> String {
        r.unwrap_err().to_string()
    }

    #[test]
    fn stock_ramdisk_is_accepted() {
        assert!(check_ksu_input("boot.img", 0, 1, 1).is_ok());
    }

    #[test]
    fn magisk_patched_ramdisk_is_refused() {
        let m = msg(check_ksu_input("boot.img", 1, 1, 1));
        assert!(m.contains("already Magisk-patched"), "{m}");
    }

    #[test]
    fn unsupported_layout_is_refused() {
        let m = msg(check_ksu_input("boot.img", 2, 1, 1));
        assert!(
            m.contains("Unsupported boot image layout (cpio test = 2)"),
            "{m}"
        );
    }

    #[test]
    fn prior_ksu_patch_is_refused_by_either_marker() {
        for (real, ko) in [(0, 1), (1, 0), (0, 0)] {
            let m = msg(check_ksu_input("boot.img", 0, real, ko));
            assert!(m.contains("already KernelSU-patched"), "{m}");
        }
    }
}
