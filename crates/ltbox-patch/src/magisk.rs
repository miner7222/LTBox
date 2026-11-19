//! Magisk root patching — extracts payload from the Magisk APK and patches the
//! selected root ramdisk via the magiskboot library (no shell-out).
//! APK layout: `lib/arm64-v8a/libmagisk{,64}.so` → `magisk`,
//! `libmagiskinit.so` → `magiskinit`, `libinit-ld.so` → `init-ld`, and,
//! through v31.0 only, `assets/stub.apk` → `stub.apk`. Magisk v31.1 removed
//! stub embedding (upstream c86a8776c), so the stub is optional.

use fs_err as fs;
use std::path::{Path, PathBuf};

use ltbox_core::i18n::tr;
use ltbox_core::{LtboxError, Result, tr_args};

use crate::boot;
use crate::root_pipeline::RootImageTarget;

/// Map the three crypto getprops to the `encrypt_type` that gates `/data`
/// eligibility in [`resolve_preinit_device`]. `"none"` = unencrypted,
/// `"file"` = FBE, `"block"` = full-disk, `"metadata"` = metadata crypto.
pub fn derive_encrypt_type(
    crypto_state: &str,
    crypto_type: &str,
    crypto_metadata_enabled: &str,
) -> &'static str {
    if crypto_state.trim() != "encrypted" {
        "none"
    } else if crypto_type.trim() == "block" {
        "block"
    } else if crypto_metadata_enabled.trim() == "true" {
        "metadata"
    } else {
        "file"
    }
}

/// Resolve Magisk's `PREINITDEVICE` partition name from `/proc/self/mountinfo`.
/// Returns the bare partition name (e.g. `"metadata"`), not a block path.
///
/// Ranking follows Magisk v31.1 `find_preinit_device` (`native/src/core/mount.rs`):
/// ext4 beats f2fs (an f2fs kernel bug can panic), except that `/metadata` is
/// compared by mount point only; then `/data` > `/cache` > `/klogdump` >
/// `/metadata` > `/persist` | `/mnt/vendor/persist`. Read-only, non-`/`-root,
/// pseudo-fs, and device-mapper mounts are filtered out.
///
/// `encrypt_type` gates `/data` eligibility: `"none"` (plain) and `"file"`
/// (FBE) keep `/data` eligible; `"block"` / `"metadata"` devices cannot use
/// userdata for preinit because the partition is unavailable pre-unlock.
///
/// Dynamic-major partitions (majors 240-254) are dropped unless the source path
/// carries `/by-name/` or contains `/vd`, so a super-partition lvm mapping that
/// disappears at next boot is never picked.
pub fn resolve_preinit_device(mountinfo: &str, encrypt_type: &str) -> Option<String> {
    const DYN_MAJOR_MIN: u32 = 240;
    const DYN_MAJOR_MAX: u32 = 254;

    let data_ok = encrypt_type == "none" || encrypt_type == "file";

    // Upstream `PartId` order: smaller is preferred.
    #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    enum PartId {
        Data,
        Cache,
        Klogdump,
        Metadata,
        Persist,
    }
    let part_for_mount = |target: &str| -> Option<PartId> {
        match target {
            "/persist" | "/mnt/vendor/persist" => Some(PartId::Persist),
            "/metadata" => Some(PartId::Metadata),
            "/klogdump" => Some(PartId::Klogdump),
            "/cache" => Some(PartId::Cache),
            "/data" if data_ok => Some(PartId::Data),
            _ => None,
        }
    };

    // (mount point, is_ext4, partition_name) in mountinfo order.
    let mut candidates: Vec<(PartId, bool, String)> = Vec::new();

    for line in mountinfo.lines() {
        // mountinfo: `mount_id parent major:minor root mount_point options - fs_type source super_options`
        let (pre, post) = match line.split_once(" - ") {
            Some(p) => p,
            None => continue,
        };
        let pre_parts: Vec<&str> = pre.split_whitespace().collect();
        let post_parts: Vec<&str> = post.split_whitespace().collect();
        if pre_parts.len() < 6 || post_parts.len() < 3 {
            continue;
        }
        let device_major: u32 = pre_parts[2]
            .split_once(':')
            .and_then(|(maj, _)| maj.parse().ok())
            .unwrap_or(0);
        let root = pre_parts[3];
        let target = pre_parts[4];
        let mount_opts = pre_parts[5];
        let fs_type = post_parts[0];
        let source = post_parts[1];

        if root != "/" {
            continue;
        }
        if !source.starts_with('/') {
            continue;
        }
        if source.contains("/dm-") {
            continue;
        }
        if fs_type != "ext4" && fs_type != "f2fs" {
            continue;
        }
        if !mount_opts.split(',').any(|o| o == "rw") {
            continue;
        }
        // Accept only /dev/block/by-name/* or /dev/block/* sources.
        let parent_ok = {
            let trimmed = source.trim_end_matches(|c| c != '/').trim_end_matches('/');
            trimmed.ends_with("by-name") || trimmed.ends_with("block")
        };
        if !parent_ok {
            continue;
        }

        // Dynamic-major filter: super-partition lvm entries get majors in
        // 240..=254 and vanish on the next boot. Keep them only when the
        // source path signals a stable name (`/by-name/`) or a virtual-disk
        // mapping (`/vd`).
        if (DYN_MAJOR_MIN..=DYN_MAJOR_MAX).contains(&device_major)
            && !source.contains("/vd")
            && !source.contains("/by-name/")
        {
            continue;
        }

        let Some(part) = part_for_mount(target) else {
            continue;
        };
        let name = source.rsplit('/').next().unwrap_or("").to_string();
        if name.is_empty() {
            continue;
        }
        candidates.push((part, fs_type == "ext4", name));
    }

    // Same comparator as upstream; `Less` means preferred.
    use std::cmp::Ordering;
    let compare = |(a, a_ext4, _): &&(PartId, bool, String),
                   (b, b_ext4, _): &&(PartId, bool, String)|
     -> Ordering {
        match (a, b, a_ext4, b_ext4) {
            // metadata is not affected by the f2fs kernel bug
            (PartId::Metadata, _, _, true) | (_, PartId::Metadata, true, _) => a.cmp(b),
            (_, _, true, false) => Ordering::Less,
            (_, _, false, true) => Ordering::Greater,
            _ => a.cmp(b),
        }
    };
    candidates
        .iter()
        .min_by(compare)
        .map(|(_, _, name)| name.clone())
}

/// Staged name of the stub APK. Present only for Magisk releases that still
/// embed it in the ramdisk (v31.0 and older).
const STUB_APK: &str = "stub.apk";

/// Staged copy of the APK's `assets/util_functions.sh`, whose build-time
/// `MAGISK_VER_CODE` line tells which magiskboot the app itself would run.
const UTIL_FUNCTIONS: &str = "util_functions.sh";

/// First Magisk release whose magiskboot always XZ-compresses code with a BCJ
/// pre-filter and whose magiskinit decodes it (upstream `da320823a`).
const FIRST_BCJ_VERSION_CODE: u32 = 31100;

/// `MAGISK_VER_CODE` from a staged `util_functions.sh`, or `None` when the
/// line is missing or not a number.
fn magisk_version_code(util_functions: &str) -> Option<u32> {
    util_functions.lines().find_map(|line| {
        let value = line.trim().strip_prefix("MAGISK_VER_CODE=")?;
        // `\u{22}` is `"`: a bare one in a char literal derails the locale
        // guard's string scanner (crates/ltbox-gui/tests/locale_guards.rs).
        value.trim().trim_matches(['\'', '\u{22}']).parse().ok()
    })
}

/// Extract Magisk payload from `apk_path` into `staging_dir`, overwriting
/// existing files.
/// Accepts both `libmagisk.so` (newer) and `libmagisk64.so` (older) for `magisk`.
/// `assets/stub.apk` is optional: when the APK has none, any stale staged
/// copy is removed so [`patch_root_image`] does not embed a stub that this
/// release's `magiskinit` no longer handles.
pub fn extract_apk_payload(apk_path: &Path, staging_dir: &Path) -> Result<()> {
    fs::create_dir_all(staging_dir)?;

    let file = fs::File::open(apk_path)
        .map_err(|e| LtboxError::Patch(format!("open APK {}: {e}", apk_path.display())))?;
    let mut zip =
        zip::ZipArchive::new(file).map_err(|e| LtboxError::Patch(format!("APK read: {e}")))?;

    // (candidate entries, staged name, required)
    let mapping: &[(&[&str], &str, bool)] = &[
        (
            &["lib/arm64-v8a/libmagisk.so", "lib/arm64-v8a/libmagisk64.so"],
            "magisk",
            true,
        ),
        (&["lib/arm64-v8a/libmagiskinit.so"], "magiskinit", true),
        (&["lib/arm64-v8a/libinit-ld.so"], "init-ld", true),
        (&["assets/stub.apk"], STUB_APK, false),
        (&["assets/util_functions.sh"], UTIL_FUNCTIONS, false),
    ];

    for (candidates, dst_name, required) in mapping {
        let dst_path = staging_dir.join(dst_name);
        let mut staged = false;
        for entry_name in candidates.iter() {
            let mut entry = match zip.by_name(entry_name) {
                Ok(e) => e,
                Err(_) => continue,
            };
            crate::zip_util::copy_capped(
                &mut entry,
                &dst_path,
                crate::zip_util::MAX_ENTRY_BYTES,
                entry_name,
            )?;
            staged = true;
            break;
        }
        if staged {
            continue;
        }
        if *required {
            return Err(LtboxError::Patch(format!(
                "APK missing entry for {dst_name} (checked {candidates:?})"
            )));
        }
        match fs::remove_file(&dst_path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

/// Patch the selected root image with Magisk. `work_dir` must contain the image
/// plus the payload files from [`extract_apk_payload`]. Writes
/// `work_dir/new-boot.img`; caller handles resign + flash.
///
/// `preinit_device` → Magisk `PREINITDEVICE` config. Empty string omits
/// the entry and lets Magisk resolve at boot (can fail on some devices).
pub fn patch_root_image(
    work_dir: &Path,
    target: RootImageTarget,
    preinit_device: &str,
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

    ltbox_core::live!(
        log,
        "[Magisk] {}",
        tr_args!("log_root_unpack_image", image = img_name)
    );
    boot::unpack(&img_path, work_dir)?;

    let ramdisk_name = boot::root_ramdisk_name(work_dir)?;
    let ramdisk = work_dir.join(ramdisk_name);

    // magiskboot cpio test: 0=stock, 1=already-patched, 2=unsupported.
    ltbox_core::live!(log, "[Magisk] {}", tr("log_magisk_cpio_test"));
    let status = boot::cpio(work_dir, ramdisk_name, &["test"])?;
    match status {
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

    // SHA-1 of stock boot for Magisk config.
    let sha1 = boot::sha1(&img_path)?;
    ltbox_core::live!(
        log,
        "[Magisk] {}",
        tr_args!("log_magisk_stock_sha1", sha1 = sha1)
    );

    // Back up stock ramdisk so Magisk can restore on unroot.
    fs::copy(&ramdisk, work_dir.join("ramdisk.cpio.orig"))?;

    ltbox_core::live!(log, "[Magisk] {}", tr("log_magisk_compressing_payload"));
    // Mirror the APK's own boot_patch.sh: v31.0 and older embed the stub,
    // v31.1+ ship no stub and their magiskinit ignores one.
    let embed_stub = work_dir.join(STUB_APK).is_file();
    // The app runs its own release's magiskboot: v31.1+ adds a BCJ pre-filter
    // that only its matching magiskinit decodes. An unreadable version keeps
    // plain XZ, which every magiskinit decodes.
    let version_code = match fs::read_to_string(work_dir.join(UTIL_FUNCTIONS)) {
        Ok(text) => magisk_version_code(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    let xz_bcj = version_code.is_some_and(|code| code >= FIRST_BCJ_VERSION_CODE);
    tracing::info!("Magisk version code {version_code:?}; XZ BCJ pre-filter: {xz_bcj}");
    let compress_xz = |input: &str, output: &str| {
        if xz_bcj {
            boot::compress_xz_bcj(work_dir, input, output)
        } else {
            boot::compress(work_dir, "xz", input, output)
        }
    };
    compress_xz("magisk", "magisk.xz")?;
    if embed_stub {
        compress_xz(STUB_APK, "stub.xz")?;
    }
    compress_xz("init-ld", "init-ld.xz")?;

    // Config file baked into .backup/.magisk.
    let mut config = String::new();
    config.push_str("KEEPVERITY=true\n");
    config.push_str("KEEPFORCEENCRYPT=true\n");
    config.push_str("RECOVERYMODE=false\n");
    config.push_str("VENDORBOOT=false\n");
    if !preinit_device.is_empty() {
        config.push_str(&format!("PREINITDEVICE={preinit_device}\n"));
    }
    config.push_str(&format!("SHA1={sha1}\n"));
    fs::write(work_dir.join("config"), &config)?;

    // Patch ramdisk in one cpio pass. `KEEPVERITY` / `KEEPFORCEENCRYPT` must be
    // set or magiskboot strips dm-verity and forceencrypt from fstab, which
    // bricks devices that expect AVB + metadata encryption.
    ltbox_core::live!(log, "[Magisk] {}", tr("log_magisk_cpio_patch"));
    let mut cpio_cmds: Vec<&str> = vec![
        "add 0750 init magiskinit",
        "mkdir 0750 overlay.d",
        "mkdir 0750 overlay.d/sbin",
        "add 0644 overlay.d/sbin/magisk.xz magisk.xz",
    ];
    if embed_stub {
        cpio_cmds.push("add 0644 overlay.d/sbin/stub.xz stub.xz");
    }
    cpio_cmds.extend([
        "add 0644 overlay.d/sbin/init-ld.xz init-ld.xz",
        "patch",
        "backup ramdisk.cpio.orig",
        "mkdir 000 .backup",
        "add 000 .backup/.magisk config",
    ]);
    boot::cpio_with_env(
        work_dir,
        ramdisk_name,
        &cpio_cmds,
        &[
            ("KEEPVERITY", "true"),
            ("KEEPFORCEENCRYPT", "true"),
            ("PATCHVBMETAFLAG", "false"),
            // `backup` XZ-compresses the stock files it stores under .backup/,
            // and v31.1 magiskboot applies BCJ there too.
            ("XZBCJ", if xz_bcj { "true" } else { "false" }),
        ],
    )?;

    // Clean up staging — don't leave plaintext payload next to the repacked image.
    for name in [
        "ramdisk.cpio.orig",
        "config",
        "magisk.xz",
        "stub.xz",
        "init-ld.xz",
    ] {
        let _ = fs::remove_file(work_dir.join(name));
    }

    ltbox_core::live!(
        log,
        "[Magisk] {}",
        tr_args!("log_root_repack_image", image = img_name)
    );
    boot::repack(img_name, work_dir)?;

    let new_boot = work_dir.join("new-boot.img");
    if !new_boot.exists() {
        return Err(LtboxError::Patch(
            "magiskboot repack produced no new-boot.img".into(),
        ));
    }
    ltbox_core::live!(log, "[Magisk] {}", tr("log_patch_complete"));
    Ok(new_boot)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Minimal mountinfo line template; only parser-relevant fields matter.
    // `major` matters for the dynamic-major filter; default 259 is ufs/sda.
    fn line(target: &str, opts: &str, fs: &str, source: &str) -> String {
        format!("1 1 259:1 / {target} {opts} shared:1 - {fs} {source} rw",)
    }

    fn line_major(major: u32, target: &str, opts: &str, fs: &str, source: &str) -> String {
        format!("1 1 {major}:1 / {target} {opts} shared:1 - {fs} {source} rw",)
    }

    #[test]
    fn picks_metadata_over_persist() {
        let info = format!(
            "{}\n{}",
            line(
                "/persist",
                "rw,seclabel",
                "ext4",
                "/dev/block/by-name/persist"
            ),
            line(
                "/metadata",
                "rw,seclabel",
                "ext4",
                "/dev/block/by-name/metadata"
            ),
        );
        assert_eq!(
            resolve_preinit_device(&info, "file").as_deref(),
            Some("metadata")
        );
    }

    #[test]
    fn prefers_ext4_over_f2fs_within_same_tier() {
        let info = format!(
            "{}\n{}",
            line(
                "/persist",
                "rw,seclabel",
                "f2fs",
                "/dev/block/by-name/persist"
            ),
            line(
                "/mnt/vendor/persist",
                "rw,seclabel",
                "ext4",
                "/dev/block/by-name/vendor_persist"
            ),
        );
        assert_eq!(
            resolve_preinit_device(&info, "file").as_deref(),
            Some("vendor_persist")
        );
    }

    /// Magisk v31.1 `find_preinit_device` order: data > cache > klogdump >
    /// metadata > persist, ext4 over f2fs except against metadata.
    #[test]
    fn ranking_matches_magisk_v31_1() {
        let pick = |mounts: &[(&str, &str, &str)], crypto: &str| {
            let info = mounts
                .iter()
                .map(|(target, fs, name)| {
                    line(
                        target,
                        "rw,seclabel",
                        fs,
                        &format!("/dev/block/by-name/{name}"),
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            resolve_preinit_device(&info, crypto)
        };
        let metadata = ("/metadata", "ext4", "metadata");
        // Usable /data wins over metadata on the same fs type.
        assert_eq!(
            pick(&[metadata, ("/data", "ext4", "userdata")], "file").as_deref(),
            Some("userdata")
        );
        // Metadata-encrypted /data is never eligible.
        assert_eq!(
            pick(&[metadata, ("/data", "ext4", "userdata")], "metadata").as_deref(),
            Some("metadata")
        );
        // An ext4 metadata beats f2fs data; an f2fs metadata still loses to ext4 data.
        assert_eq!(
            pick(&[metadata, ("/data", "f2fs", "userdata")], "file").as_deref(),
            Some("metadata")
        );
        assert_eq!(
            pick(
                &[
                    ("/metadata", "f2fs", "metadata"),
                    ("/data", "ext4", "userdata")
                ],
                "file"
            )
            .as_deref(),
            Some("userdata")
        );
        // Cache beats metadata; metadata beats persist even as f2fs vs ext4.
        assert_eq!(
            pick(&[metadata, ("/cache", "ext4", "cache")], "metadata").as_deref(),
            Some("cache")
        );
        assert_eq!(
            pick(
                &[
                    ("/persist", "ext4", "persist"),
                    ("/metadata", "f2fs", "metadata")
                ],
                "metadata"
            )
            .as_deref(),
            Some("metadata")
        );
        // Outside metadata, ext4 wins across mount points.
        assert_eq!(
            pick(
                &[("/cache", "f2fs", "cache"), ("/persist", "ext4", "persist")],
                "metadata"
            )
            .as_deref(),
            Some("persist")
        );
    }

    #[test]
    fn drops_readonly_mounts() {
        let info = line(
            "/metadata",
            "ro,seclabel",
            "ext4",
            "/dev/block/by-name/metadata",
        );
        assert_eq!(resolve_preinit_device(&info, "file"), None);
    }

    #[test]
    fn drops_device_mapper_sources() {
        let info = line("/metadata", "rw", "ext4", "/dev/block/dm-5");
        assert_eq!(resolve_preinit_device(&info, "file"), None);
    }

    #[test]
    fn drops_tmpfs_mounts() {
        let info = "1 1 0:1 / /metadata rw shared:1 - tmpfs tmpfs rw".to_string();
        assert_eq!(resolve_preinit_device(&info, "file"), None);
    }

    #[test]
    fn empty_returns_none() {
        assert_eq!(resolve_preinit_device("", "file"), None);
    }

    #[test]
    fn data_eligible_when_unencrypted_or_fbe() {
        let info = line(
            "/data",
            "rw,seclabel",
            "f2fs",
            "/dev/block/by-name/userdata",
        );
        assert_eq!(
            resolve_preinit_device(&info, "none").as_deref(),
            Some("userdata")
        );
        assert_eq!(
            resolve_preinit_device(&info, "file").as_deref(),
            Some("userdata")
        );
    }

    #[test]
    fn data_dropped_on_block_or_metadata_crypto() {
        let info = line(
            "/data",
            "rw,seclabel",
            "f2fs",
            "/dev/block/by-name/userdata",
        );
        assert_eq!(resolve_preinit_device(&info, "block"), None);
        assert_eq!(resolve_preinit_device(&info, "metadata"), None);
    }

    #[test]
    fn drops_dynamic_major_without_stable_hint() {
        // major 253 (dm-crypt / super lvm) + no `/vd` or `/by-name/` in path → drop.
        let info = line_major(253, "/metadata", "rw,seclabel", "ext4", "/dev/block/dm-0");
        assert_eq!(resolve_preinit_device(&info, "file"), None);
    }

    #[test]
    fn keeps_dynamic_major_with_by_name() {
        let info = line_major(
            253,
            "/metadata",
            "rw,seclabel",
            "ext4",
            "/dev/block/by-name/metadata",
        );
        assert_eq!(
            resolve_preinit_device(&info, "file").as_deref(),
            Some("metadata")
        );
    }

    #[test]
    fn encrypt_type_cascade() {
        assert_eq!(derive_encrypt_type("unencrypted", "", ""), "none");
        assert_eq!(derive_encrypt_type("encrypted", "block", ""), "block");
        assert_eq!(derive_encrypt_type("encrypted", "file", "true"), "metadata");
        assert_eq!(derive_encrypt_type("encrypted", "file", "false"), "file");
        assert_eq!(derive_encrypt_type("encrypted", "file", ""), "file");
    }

    fn write_apk(path: &Path, entries: &[&str]) {
        use std::io::Write;
        let mut archive = zip::ZipWriter::new(fs::File::create(path).unwrap());
        for name in entries {
            archive
                .start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            archive.write_all(name.as_bytes()).unwrap();
        }
        archive.finish().unwrap();
    }

    const CORE_ENTRIES: [&str; 3] = [
        "lib/arm64-v8a/libmagisk.so",
        "lib/arm64-v8a/libmagiskinit.so",
        "lib/arm64-v8a/libinit-ld.so",
    ];

    #[test]
    fn stages_stub_when_the_apk_still_embeds_one() {
        let temp = tempfile::tempdir().unwrap();
        let apk = temp.path().join("magisk.apk");
        let mut entries = CORE_ENTRIES.to_vec();
        entries.push("assets/stub.apk");
        write_apk(&apk, &entries);
        let staging = temp.path().join("work");
        extract_apk_payload(&apk, &staging).unwrap();
        for name in ["magisk", "magiskinit", "init-ld", STUB_APK] {
            assert!(staging.join(name).is_file(), "{name} not staged");
        }
    }

    #[test]
    fn stubless_apk_stages_core_payload_and_drops_a_stale_stub() {
        // Magisk v31.1+ ships no assets/stub.apk.
        let temp = tempfile::tempdir().unwrap();
        let apk = temp.path().join("magisk.apk");
        write_apk(&apk, &CORE_ENTRIES);
        let staging = temp.path().join("work");
        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join(STUB_APK), b"stale").unwrap();
        extract_apk_payload(&apk, &staging).unwrap();
        for name in ["magisk", "magiskinit", "init-ld"] {
            assert!(staging.join(name).is_file(), "{name} not staged");
        }
        assert!(!staging.join(STUB_APK).exists());
    }

    #[test]
    fn stages_util_functions_and_drops_a_stale_copy() {
        let temp = tempfile::tempdir().unwrap();
        let staging = temp.path().join("work");
        let with = temp.path().join("with.apk");
        let mut entries = CORE_ENTRIES.to_vec();
        entries.push("assets/util_functions.sh");
        write_apk(&with, &entries);
        extract_apk_payload(&with, &staging).unwrap();
        assert!(staging.join(UTIL_FUNCTIONS).is_file());

        // A stale copy from an earlier APK must not decide the BCJ mode.
        let without = temp.path().join("without.apk");
        write_apk(&without, &CORE_ENTRIES);
        extract_apk_payload(&without, &staging).unwrap();
        assert!(!staging.join(UTIL_FUNCTIONS).exists());
    }

    #[test]
    fn reads_the_build_time_version_code() {
        // Layout Magisk's Setup.kt writes over `#MAGISK_VERSION_STUB`.
        let script =
            "#!/system/bin/sh\nMAGISK_VER='31.1'\nMAGISK_VER_CODE=31100\n\nui_print() {}\n";
        assert_eq!(magisk_version_code(script), Some(31100));
        assert_eq!(
            magisk_version_code("MAGISK_VER_CODE='30700'\n"),
            Some(30700)
        );
        assert_eq!(
            magisk_version_code("  MAGISK_VER_CODE=\"31000\"  \r\n"),
            Some(31000)
        );
        // The unbuilt source keeps only the stub marker.
        assert_eq!(magisk_version_code("#MAGISK_VERSION_STUB\n"), None);
        assert_eq!(magisk_version_code("MAGISK_VER_CODE=\n"), None);
        assert_eq!(magisk_version_code("MAGISK_VER_CODE=31.1\n"), None);
        assert_eq!(magisk_version_code("# MAGISK_VER_CODE=31100\n"), None);
    }

    #[test]
    fn missing_core_payload_still_fails() {
        let temp = tempfile::tempdir().unwrap();
        let apk = temp.path().join("magisk.apk");
        write_apk(&apk, &CORE_ENTRIES[..2]);
        let error = extract_apk_payload(&apk, &temp.path().join("work")).unwrap_err();
        assert!(error.to_string().contains("init-ld"), "{error}");
    }
}
