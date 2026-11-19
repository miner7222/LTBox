//! APatch / FolkPatch kernel patching via kptools-rs library.
//!
//! Calls the kptools library directly (unpack, patch, repack) inside `work_dir`.

use fs_err as fs;
use std::path::{Path, PathBuf};

use kptools::bootimg;
use kptools::patch::{self, ExtraConfig, PatchArgs};
use kptools::preset::ExtraType;

use ltbox_core::{LtboxError, Result, tr_args};

const SUPERKEY_MIN: usize = 8;
const SUPERKEY_MAX: usize = 63;

/// Validate superkey: 8–63 chars, ASCII alphanumeric only.
pub(crate) fn validate_superkey(sk: &str) -> Result<()> {
    let n = sk.len();
    if !(SUPERKEY_MIN..=SUPERKEY_MAX).contains(&n) {
        return Err(LtboxError::Patch(format!(
            "superkey must be {SUPERKEY_MIN}..={SUPERKEY_MAX} chars, got {n}"
        )));
    }
    if !sk.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(LtboxError::Patch(
            "superkey must be ASCII alphanumeric only".into(),
        ));
    }
    Ok(())
}

/// Patch `work_dir/boot.img` with `work_dir/kpimg` + optional KPMs.
/// An empty key selects APatch's default signature/UID authorization:
/// upstream `boot_patch.sh` omits `-S` when its manager passes `su`.
/// Returns `work_dir/boot_patched.img`. All intermediates land in `work_dir`.
pub fn patch_boot(
    work_dir: &Path,
    kpm_paths: &[PathBuf],
    superkey: &str,
    log: &mut Vec<String>,
) -> Result<PathBuf> {
    if superkey.is_empty() {
        return patch_boot_impl(work_dir, kpm_paths, "", true, log);
    }
    validate_superkey(superkey)?;
    patch_boot_impl(work_dir, kpm_paths, superkey, false, log)
}

/// Match FolkPatch's manager and `boot_patch.sh`: its default key `su` is only
/// a placeholder for signed-manager/UID authorization, so `-S` is omitted and
/// the kernel keeps a zero key hash. Writing `-S su` (FolkPatch before upstream
/// 5259dd44) would let any app authenticate with the fixed key "su".
pub fn patch_folkpatch_boot(
    work_dir: &Path,
    kpm_paths: &[PathBuf],
    log: &mut Vec<String>,
) -> Result<PathBuf> {
    patch_boot_impl(work_dir, kpm_paths, "", true, log)
}

fn patch_boot_impl(
    work_dir: &Path,
    kpm_paths: &[PathBuf],
    superkey: &str,
    root_key: bool,
    log: &mut Vec<String>,
) -> Result<PathBuf> {
    // kptools-base defaults LOG_ENABLE off. Enable it: the GUI's stdout_tap
    // feeds stderr to the live log panel, the only view into kptools mid-patch.
    // Idempotent.
    kptools::log::set_log_enable(true);

    let boot_in = work_dir.join("boot.img");
    let kpimg = work_dir.join("kpimg");
    let kernel_ori = work_dir.join("kernel.ori");
    let kernel_out = work_dir.join("kernel.out");
    let boot_out = work_dir.join("boot_patched.img");

    if !boot_in.exists() {
        return Err(LtboxError::Patch(format!(
            "apatch: work_dir missing boot.img at {}",
            boot_in.display()
        )));
    }
    if !kpimg.exists() {
        return Err(LtboxError::Patch(format!(
            "apatch: work_dir missing kpimg at {}",
            kpimg.display()
        )));
    }

    ltbox_core::live!(
        log,
        "[APatch] {}",
        tr_args!(
            "log_apatch_extract_kernel",
            input = boot_in.display(),
            output = kernel_ori.display()
        )
    );
    bootimg::extract_kernel(&boot_in, &kernel_ori)
        .map_err(|e| LtboxError::Patch(format!("kptools extract_kernel failed: {e}")))?;

    // Bail if already patched. `parse_image_patch_info` is Ok for any kernel with
    // a Linux banner, so only `preset_offset.is_some()` tells patched from pristine.
    {
        let kimg_bytes = fs::read(&kernel_ori)?;
        if let Ok(info) = patch::parse_image_patch_info(&kimg_bytes)
            && info.preset_offset.is_some()
        {
            return Err(LtboxError::Patch(
                "boot kernel is already APatch-patched — unroot first before re-patching".into(),
            ));
        }

        // CONFIG_KALLSYMS gate: kptools needs symbol resolution, and patching
        // without it bricks. Detected by byte search in the IKCONFIG payload;
        // absent IKCONFIG is inconclusive (warn, don't block).
        let has_kallsyms = kimg_bytes
            .windows(b"CONFIG_KALLSYMS=y".len())
            .any(|w| w == b"CONFIG_KALLSYMS=y");
        let has_kallsyms_all = kimg_bytes
            .windows(b"CONFIG_KALLSYMS_ALL=y".len())
            .any(|w| w == b"CONFIG_KALLSYMS_ALL=y");
        let has_ikconfig_marker = kimg_bytes
            .windows(b"CONFIG_IKCONFIG".len())
            .any(|w| w == b"CONFIG_IKCONFIG");
        if !has_kallsyms && has_ikconfig_marker {
            return Err(LtboxError::Patch(
                "kernel missing CONFIG_KALLSYMS=y — kptools cannot resolve patch points. Flashing would brick the device. Aborting.".into(),
            ));
        } else if !has_kallsyms {
            ltbox_core::live!(
                log,
                "[APatch] {}",
                ltbox_core::i18n::tr("log_apatch_kallsyms_inconclusive")
            );
        } else {
            ltbox_core::live!(
                log,
                "[APatch] {}",
                ltbox_core::i18n::tr("log_apatch_kallsyms_ok")
            );
            if !has_kallsyms_all {
                ltbox_core::live!(
                    log,
                    "[APatch] {}",
                    ltbox_core::i18n::tr("log_apatch_kallsyms_all_missing")
                );
            }
        }
    }

    let extras: Vec<ExtraConfig> = kpm_paths
        .iter()
        .map(|p| {
            ExtraConfig::from_path(p, ExtraType::Kpm).map_err(|e| {
                LtboxError::Patch(format!(
                    "kptools ExtraConfig::from_path({}) failed: {e}",
                    p.display()
                ))
            })
        })
        .collect::<Result<_>>()?;
    ltbox_core::live!(
        log,
        "[APatch] {}",
        tr_args!(
            "log_apatch_patching_kernel",
            kpm_count = extras.len(),
            superkey_len = superkey.len()
        )
    );

    patch::patch_update_img(PatchArgs {
        kimg_path: &kernel_ori,
        kpimg_path: &kpimg,
        out_path: &kernel_out,
        superkey,
        root_key,
        additional: Vec::new(),
        extras,
    })
    .map_err(|e| LtboxError::Patch(format!("kptools patch_update_img failed: {e}")))?;

    ltbox_core::live!(
        log,
        "[APatch] {}",
        tr_args!(
            "log_apatch_repack_bootimg",
            boot = boot_in.display(),
            kernel = kernel_out.display(),
            output = boot_out.display()
        )
    );
    bootimg::repack_bootimg(&boot_in, &kernel_out, &boot_out)
        .map_err(|e| LtboxError::Patch(format!("kptools repack_bootimg failed: {e}")))?;

    Ok(boot_out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn superkey_too_short() {
        assert!(validate_superkey("abc").is_err());
        assert!(validate_superkey("su").is_err());
    }

    #[test]
    fn superkey_too_long() {
        let s = "a".repeat(64);
        assert!(validate_superkey(&s).is_err());
    }

    #[test]
    fn superkey_non_alnum() {
        assert!(validate_superkey("abc!def@").is_err());
    }

    #[test]
    fn superkey_boundary_min_ok() {
        assert!(validate_superkey("abcdefgh").is_ok());
    }

    #[test]
    fn superkey_boundary_max_ok() {
        let s = "a".repeat(63);
        assert!(validate_superkey(&s).is_ok());
    }

    #[test]
    fn patch_boot_missing_boot_img_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let mut log = Vec::new();
        let err = patch_boot(tmp.path(), &[], "abcdefgh", &mut log).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("missing boot.img"), "unexpected: {msg}");
    }

    #[test]
    fn folkpatch_default_auth_reaches_boot_processing() {
        let tmp = tempfile::tempdir().unwrap();
        let err = patch_folkpatch_boot(tmp.path(), &[], &mut Vec::new()).unwrap_err();
        assert!(err.to_string().contains("missing boot.img"));
    }

    #[test]
    fn apatch_default_auth_reaches_boot_processing() {
        let tmp = tempfile::tempdir().unwrap();
        let err = patch_boot(tmp.path(), &[], "", &mut Vec::new()).unwrap_err();
        assert!(err.to_string().contains("missing boot.img"));
    }

    #[test]
    #[ignore = "needs LTBOX_TEST_PRISTINE_BOOT_IMG, LTBOX_TEST_APATCH_KPIMG, LTBOX_TEST_APATCH_EXPECTED_BOOT"]
    fn apatch_default_boot_matches_upstream() {
        let fixture = |name: &str| PathBuf::from(std::env::var_os(name).expect(name));
        let tmp = tempfile::tempdir().unwrap();
        fs::copy(
            fixture("LTBOX_TEST_PRISTINE_BOOT_IMG"),
            tmp.path().join("boot.img"),
        )
        .unwrap();
        fs::copy(fixture("LTBOX_TEST_APATCH_KPIMG"), tmp.path().join("kpimg")).unwrap();
        let output = patch_boot(tmp.path(), &[], "", &mut Vec::new()).unwrap();
        let expected = fs::read(fixture("LTBOX_TEST_APATCH_EXPECTED_BOOT")).unwrap();
        let actual = fs::read(output).unwrap();
        assert_eq!(actual.len(), expected.len(), "boot image sizes differ");
        assert!(
            actual == expected,
            "APatch boot differs from upstream without -S"
        );
    }

    /// Compare the actual library path to upstream FolkPatch 115404+ `boot_patch.sh`
    /// with its default key, which omits `-S`. Fixtures are external: pristine boot,
    /// FolkPatch kpimg, and upstream boot output; an output made with `-S su`
    /// (FolkPatch before 5259dd44) is stale and must fail.
    #[test]
    #[ignore = "needs LTBOX_TEST_PRISTINE_BOOT_IMG, LTBOX_TEST_FOLKPATCH_KPIMG, LTBOX_TEST_FOLKPATCH_EXPECTED_BOOT"]
    fn folkpatch_boot_matches_upstream() {
        let fixture = |name: &str| PathBuf::from(std::env::var_os(name).expect(name));
        let tmp = tempfile::tempdir().unwrap();
        fs::copy(
            fixture("LTBOX_TEST_PRISTINE_BOOT_IMG"),
            tmp.path().join("boot.img"),
        )
        .unwrap();
        fs::copy(
            fixture("LTBOX_TEST_FOLKPATCH_KPIMG"),
            tmp.path().join("kpimg"),
        )
        .unwrap();
        let output = patch_folkpatch_boot(tmp.path(), &[], &mut Vec::new()).unwrap();
        let expected = fs::read(fixture("LTBOX_TEST_FOLKPATCH_EXPECTED_BOOT")).unwrap();
        let actual = fs::read(output).unwrap();
        assert_eq!(actual.len(), expected.len(), "boot image sizes differ");
        assert!(
            actual == expected,
            "FolkPatch boot differs from upstream without -S"
        );
    }

    #[test]
    fn patch_boot_missing_kpimg_errors() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("boot.img"), b"fake").unwrap();
        let mut log = Vec::new();
        let err = patch_boot(tmp.path(), &[], "abcdefgh", &mut log).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("missing kpimg"), "unexpected: {msg}");
    }

    /// Regression guard: `parse_image_patch_info(..).is_ok()` is true on every
    /// kernel with a Linux banner, pristine ones included, so the already-patched
    /// check must key on `preset_offset.is_some()`. Asserts `preset_offset = None`
    /// on the pristine boot image named by `LTBOX_TEST_PRISTINE_BOOT_IMG`;
    /// `#[ignore]`d because CI ships no firmware. Run locally with
    ///
    ///     LTBOX_TEST_PRISTINE_BOOT_IMG=/path/to/boot.img \
    ///         cargo test -p ltbox-patch -- --ignored pristine_boot_not_flagged_as_patched
    ///
    /// (`set VAR=...` on Windows cmd, `$env:VAR="..."` in PowerShell.) See
    /// `docs/vault/70_References.md` for the full env-var list.
    #[test]
    #[ignore = "needs LTBOX_TEST_PRISTINE_BOOT_IMG=/path/to/boot.img"]
    fn pristine_boot_not_flagged_as_patched() {
        let env_key = "LTBOX_TEST_PRISTINE_BOOT_IMG";
        let Some(boot_img) = std::env::var_os(env_key).map(std::path::PathBuf::from) else {
            panic!(
                "{env_key} not set — point it at a known-pristine boot.img and re-run; \
                 see docs/vault/70_References.md"
            );
        };
        if !boot_img.exists() {
            panic!("{env_key} = {} does not exist on disk", boot_img.display());
        }
        let tmp = tempfile::tempdir().unwrap();
        let kernel = tmp.path().join("kernel.ori");
        kptools::bootimg::extract_kernel(&boot_img, &kernel).unwrap();
        let bytes = fs_err::read(&kernel).unwrap();
        let info =
            kptools::patch::parse_image_patch_info(&bytes).expect("pristine kernel parses cleanly");
        assert!(
            info.preset_offset.is_none(),
            "pristine kernel should report preset_offset=None, got {:?}",
            info.preset_offset
        );
    }
}
