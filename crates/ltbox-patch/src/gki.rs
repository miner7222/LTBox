//! GKI custom-kernel root — replaces the kernel blob inside `boot.img`
//! with one supplied by the user. Two input shapes accepted:
//!
//! * **AnyKernel3 zip**: `Image` / `kernel` / etc. pulled out via
//!   `extract_kernel_from_zip`.
//! * **Raw `boot.img`**: unpacked with magiskboot in a scratch subdir under
//!   `work_dir`, and its `kernel` is copied over the stock kernel
//!   (`extract_kernel_from_boot_img`), so any donor boot image works without
//!   repackaging as AnyKernel3.
//!
//! The patcher entry selects by extension (`.zip` or `.img`); anything else
//! errors before the stock unpack runs.
//!
//! Writes `work_dir/new-boot.img` for the subsequent AVB re-sign step.

use fs_err as fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use ltbox_core::i18n::tr;
use ltbox_core::{LtboxError, Result};

use crate::boot;

/// Candidate kernel entry names inside AnyKernel3 zips; shortest path wins.
const CANDIDATES: &[&str] = &[
    "Image",
    "kernel",
    "Image.gz-dtb",
    "Image-dtb",
    "Image.gz",
    "zImage",
    "zImage-dtb",
];

/// Subdirectory under `work_dir` for unpacking a donor `boot.img`, kept apart
/// from the stock unpack so the two `kernel` files don't collide and the stock
/// dtb / ramdisk stay intact for the repack.
const CUSTOM_BOOT_SUBDIR: &str = "custom_kernel_extract";

/// Patch `boot.img` inside `work_dir` with a kernel sourced from
/// `kernel_src`. Branches on the source extension:
///
/// * `.zip` → AnyKernel3 layout, kernel pulled from the zip entry.
/// * `.img` → boot.img donor, unpacked in a scratch subdir to extract
///   its `kernel` blob.
///
/// Any other extension errors up front, before the stock unpack.
///
/// On success, writes `work_dir/new-boot.img`.
pub fn patch_boot(work_dir: &Path, kernel_src: &Path, log: &mut Vec<String>) -> Result<PathBuf> {
    // Validate the extension before the stock unpack: the GUI picker filters to
    // `.zip` / `.img`, but a typed path or stale recent-paths entry may not.
    let ext = kernel_src
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("zip") | Some("img") => {}
        other => {
            return Err(LtboxError::Patch(format!(
                "unsupported kernel source extension {:?} for {}; expected .zip or .img",
                other,
                kernel_src.display()
            )));
        }
    }

    let img_name = "boot.img";
    let img_path = work_dir.join(img_name);
    if !img_path.exists() {
        return Err(LtboxError::Patch(format!(
            "boot.img not found in {}",
            work_dir.display()
        )));
    }

    ltbox_core::live!(log, "[GKI] {}", tr("log_gki_unpack_boot"));
    boot::unpack(&img_path, work_dir)?;
    let kernel_dst = work_dir.join("kernel");
    if !kernel_dst.exists() {
        return Err(LtboxError::Patch(
            "magiskboot unpack did not produce a `kernel` file — boot image has no kernel".into(),
        ));
    }

    // Kernel-version sanity check — diagnostic only, catches wrong-kernel-family zips.
    if let Some(ver) = extract_linux_version(&kernel_dst) {
        ltbox_core::live!(
            log,
            "[GKI] {}",
            ltbox_core::tr_args!("log_gki_stock_kver", version = ver)
        );
    } else {
        ltbox_core::live!(log, "[GKI] {}", tr("log_gki_stock_kver_missing"));
    }

    ltbox_core::live!(
        log,
        "[GKI] {}",
        ltbox_core::tr_args!("log_gki_extracting_kernel", path = kernel_src.display())
    );
    match ext.as_deref() {
        Some("zip") => extract_kernel_from_zip(kernel_src, &kernel_dst, log)?,
        // Whitelist-validated above; the remaining variant is `.img`.
        _ => extract_kernel_from_boot_img(kernel_src, &kernel_dst, work_dir, log)?,
    }

    if let Some(ver) = extract_linux_version(&kernel_dst) {
        ltbox_core::live!(
            log,
            "[GKI] {}",
            ltbox_core::tr_args!("log_gki_replacement_kver", version = ver)
        );
    } else {
        ltbox_core::live!(log, "[GKI] {}", tr("log_gki_replacement_kver_missing"));
    }

    ltbox_core::live!(log, "[GKI] {}", tr("log_gki_repack_boot"));
    boot::repack(img_name, work_dir)?;
    let new_boot = work_dir.join("new-boot.img");
    if !new_boot.exists() {
        return Err(LtboxError::Patch(
            "magiskboot repack produced no new-boot.img".into(),
        ));
    }
    ltbox_core::live!(log, "[GKI] {}", tr("log_patch_complete"));
    Ok(new_boot)
}

/// Scan kernel binary for `"Linux version X.Y.Z …"` banner; returns `X.Y.Z`
/// or `None` if absent. Caller treats `None` as inconclusive.
fn extract_linux_version(kernel_path: &Path) -> Option<String> {
    let data = fs::read(kernel_path).ok()?;
    const MARKER: &[u8] = b"Linux version ";
    let idx = data.windows(MARKER.len()).position(|w| w == MARKER)?;
    let rest = &data[idx + MARKER.len()..];
    // Stop at first non-digit/dot to avoid trailing `(builder@host) #...` cruft.
    let mut out = String::new();
    for b in rest.iter().take(64) {
        if b.is_ascii_digit() || *b == b'.' {
            out.push(*b as char);
        } else {
            break;
        }
    }
    // Require two dots so partial matches don't slip through.
    if out.chars().filter(|c| *c == '.').count() >= 2 {
        Some(out)
    } else {
        None
    }
}

/// Pull the kernel out of a user-supplied donor `boot.img` and copy
/// it over `dst`. The donor is unpacked in an isolated subdir so the stock
/// unpack in `work_dir` (`kernel`, `ramdisk.cpio`, dtb, ...) isn't clobbered;
/// only `kernel` is transplanted.
///
/// The subdir is wiped on entry, and removed on success; a failure leaves it
/// for post-mortem.
fn extract_kernel_from_boot_img(
    boot_img: &Path,
    dst: &Path,
    work_dir: &Path,
    log: &mut Vec<String>,
) -> Result<()> {
    let scratch = work_dir.join(CUSTOM_BOOT_SUBDIR);
    if scratch.exists() {
        fs::remove_dir_all(&scratch)
            .map_err(|e| LtboxError::Patch(format!("clear scratch {}: {e}", scratch.display())))?;
    }
    fs::create_dir_all(&scratch)
        .map_err(|e| LtboxError::Patch(format!("create scratch {}: {e}", scratch.display())))?;
    boot::unpack(boot_img, &scratch)?;
    let donor_kernel = scratch.join("kernel");
    if !donor_kernel.exists() {
        return Err(LtboxError::Patch(format!(
            "donor boot image {} produced no `kernel` after magiskboot unpack — \
             not a recognised boot image format",
            boot_img.display()
        )));
    }
    let copied = fs::copy(&donor_kernel, dst)
        .map_err(|e| LtboxError::Patch(format!("copy donor kernel: {e}")))?;
    // Best-effort: a leftover subdir is wiped by the next run.
    let _ = fs::remove_dir_all(&scratch);
    ltbox_core::live!(
        log,
        "[GKI] {}",
        ltbox_core::tr_args!(
            "log_gki_staged",
            source = boot_img.display(),
            size = ltbox_core::log_format::bytes(copied)
        )
    );
    Ok(())
}

fn extract_kernel_from_zip(zip_path: &Path, dst: &Path, log: &mut Vec<String>) -> Result<()> {
    let file =
        fs::File::open(zip_path).map_err(|e| LtboxError::Patch(format!("open kernel zip: {e}")))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| LtboxError::Patch(format!("kernel zip read: {e}")))?;

    // Per candidate: try exact archive-root match first, then basename match
    // preferring the shortest path so root `Image` beats `subdir/Image`.
    let names: Vec<String> = archive
        .file_names()
        .collect::<zip::result::ZipResult<Vec<_>>>()
        .map_err(|e| LtboxError::Patch(format!("kernel zip read: {e}")))?
        .into_iter()
        .filter(|n| !n.ends_with('/'))
        .map(|s| s.into_owned())
        .collect();

    let mut picked: Option<String> = None;
    'outer: for candidate in CANDIDATES {
        if let Some(n) = names.iter().find(|n| n.as_str() == *candidate) {
            picked = Some(n.clone());
            break;
        }
        let mut matches: Vec<&String> =
            names.iter().filter(|n| basename(n) == *candidate).collect();
        if !matches.is_empty() {
            matches.sort_by_key(|n| (n.matches('/').count(), n.to_lowercase()));
            picked = Some(matches[0].clone());
            break 'outer;
        }
    }

    let name = picked.ok_or_else(|| {
        LtboxError::Patch(format!(
            "No kernel entry in {} (looked for {:?})",
            zip_path.display(),
            CANDIDATES
        ))
    })?;

    let mut entry = archive
        .by_name(&name)
        .map_err(|e| LtboxError::Patch(format!("kernel zip {name}: {e}")))?;
    // Stream to disk rather than pre-size a `Vec` from `size()`, which comes
    // from the untrusted zip header (OOM on a hostile zip). The 200 MiB cap,
    // well above any real boot kernel, also bounds a malformed stream.
    const MAX_KERNEL_BYTES: u64 = 200 * 1024 * 1024;
    let mut out = fs::File::create(dst)?;
    let copied = {
        // `take(MAX + 1)` so an exactly-MAX-byte kernel passes and only a
        // longer stream trips the cap.
        let mut limited = (&mut entry).take(MAX_KERNEL_BYTES + 1);
        std::io::copy(&mut limited, &mut out)?
    };
    if copied > MAX_KERNEL_BYTES {
        return Err(LtboxError::Patch(format!(
            "kernel zip entry {name} exceeds {MAX_KERNEL_BYTES} byte cap; \
             refusing to stage"
        )));
    }
    drop(entry);
    ltbox_core::live!(
        log,
        "[GKI] {}",
        ltbox_core::tr_args!(
            "log_gki_staged",
            source = name,
            size = ltbox_core::log_format::bytes(copied)
        )
    );
    Ok(())
}

fn basename(path: &str) -> &str {
    match path.rfind('/') {
        Some(idx) => &path[idx + 1..],
        None => path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_standard_banner() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        fs::write(
            tmp.path(),
            b"\x00\x00padding\x00Linux version 6.6.118-android15-8-gabc (builder@host) ...",
        )
        .unwrap();
        assert_eq!(
            extract_linux_version(tmp.path()).as_deref(),
            Some("6.6.118")
        );
    }

    #[test]
    fn rejects_two_segment_version() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        fs::write(tmp.path(), b"junk Linux version 6.6 trailing").unwrap();
        assert_eq!(extract_linux_version(tmp.path()), None);
    }

    #[test]
    fn returns_none_without_banner() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        fs::write(tmp.path(), b"ELF\x7f random bytes no banner").unwrap();
        assert_eq!(extract_linux_version(tmp.path()), None);
    }
}
