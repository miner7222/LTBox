//! Bounded zip-entry extraction.
//!
//! A zip entry's declared size (`ZipFile::size()`) comes from the untrusted
//! local header, so a hostile archive could OOM a `Vec::with_capacity(size)` /
//! `read_to_end` before any byte is read. [`copy_capped`] streams the entry to
//! disk under a hard upper bound instead.

use fs_err as fs;
use std::io::Read;
use std::path::Path;

use ltbox_core::{LtboxError, Result};

/// Upper bound for any single staged zip entry (Magisk APK payload libs,
/// KernelSU `.ko` / `ksuinit`): well above any real boot payload, yet it stops a
/// runaway stream from a corrupt or hostile archive. Matches the kernel cap in
/// [`crate::gki`].
pub(crate) const MAX_ENTRY_BYTES: u64 = 200 * 1024 * 1024;

/// Stream `src` into `dst`, refusing once more than `max` bytes have been
/// read. `label` names the entry in error messages. Returns the byte count
/// written.
///
/// Use this instead of `Vec::with_capacity(entry.size())` + `read_to_end`;
/// `size()` is untrusted.
pub(crate) fn copy_capped(
    src: impl Read,
    dst: &Path,
    max: u64,
    label: impl std::fmt::Display,
) -> Result<u64> {
    let mut out = fs::File::create(dst)
        .map_err(|e| LtboxError::Patch(format!("create {}: {e}", dst.display())))?;
    // `take(max + 1)` so an exactly-`max`-byte entry passes and only a larger
    // stream trips the cap.
    let copied = std::io::copy(&mut src.take(max + 1), &mut out)
        .map_err(|e| LtboxError::Patch(format!("extract {label}: {e}")))?;
    if copied > max {
        return Err(LtboxError::Patch(format!(
            "{label} exceeds {max} byte cap; refusing to stage"
        )));
    }
    Ok(copied)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn under_cap_streams_all_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        let dst = tmp.path().join("out.bin");
        let data = vec![0xABu8; 1000];
        let n = copy_capped(&data[..], &dst, 4096, "under").unwrap();
        assert_eq!(n, 1000);
        assert_eq!(fs::read(&dst).unwrap(), data);
    }

    #[test]
    fn exact_cap_is_accepted() {
        let tmp = tempfile::tempdir().unwrap();
        let dst = tmp.path().join("out.bin");
        let data = vec![1u8; 1000];
        let n = copy_capped(&data[..], &dst, 1000, "exact").unwrap();
        assert_eq!(n, 1000);
    }

    #[test]
    fn over_cap_rejected_without_buffering() {
        let tmp = tempfile::tempdir().unwrap();
        let dst = tmp.path().join("out.bin");
        let data = vec![2u8; 5000];
        let err = copy_capped(&data[..], &dst, 1000, "over").unwrap_err();
        match err {
            LtboxError::Patch(msg) => assert!(msg.contains("exceeds"), "got: {msg}"),
            other => panic!("expected Patch, got {other:?}"),
        }
    }
}
