//! Safe path joining for untrusted relative references.
//!
//! Firmware metadata (Sahara manifests, rawprogram XML) and device GPT labels
//! carry path strings that LTBox joins onto a working directory. A hostile or
//! corrupt source could supply an absolute path or `..` to read or overwrite
//! an arbitrary file; [`safe_join`] rejects lexical traversal first.

use std::path::{Component, Path, PathBuf};

use crate::error::{LtboxError, Result};

/// Join `rel` onto `base` after rejecting lexical traversal.
///
/// Accepts plain names and forward sub-paths (`Normal` components and `.`),
/// preserving legitimate firmware layouts. Rejects:
/// * absolute paths and Windows drive / UNC prefixes (`RootDir`, `Prefix`),
/// * `..` parent-directory components (the traversal vector),
/// * empty input, or a reference that resolves back to `base` itself.
///
/// Symlinks are not resolved. This does not guarantee filesystem containment:
/// a symlink below `base` can still lead outside it. Callers requiring that
/// guarantee must additionally control directory contents or use handle-based
/// filesystem traversal; canonicalize-then-use alone is vulnerable to races.
pub fn safe_join(base: &Path, rel: &str) -> Result<PathBuf> {
    if rel.trim().is_empty() {
        return Err(LtboxError::Other("empty path reference".into()));
    }
    let mut out = base.to_path_buf();
    for comp in Path::new(rel).components() {
        match comp {
            Component::Normal(c) => out.push(c),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(LtboxError::Other(format!(
                    "path reference `{rel}` contains a `..` component"
                )));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(LtboxError::Other(format!(
                    "path reference `{rel}` is absolute or has a drive/UNC prefix"
                )));
            }
        }
    }
    // Only `.` components leave `out == base`; callers must never open the
    // base directory as a file.
    if out == base {
        return Err(LtboxError::Other(format!(
            "path reference `{rel}` resolves to the base directory"
        )));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_filename_joins() {
        let got = safe_join(Path::new("/fw"), "boot.img").unwrap();
        assert_eq!(got, Path::new("/fw").join("boot.img"));
    }

    #[test]
    fn forward_subpath_allowed() {
        let got = safe_join(Path::new("/fw"), "images/boot.img").unwrap();
        assert_eq!(got, Path::new("/fw").join("images").join("boot.img"));
    }

    #[test]
    fn current_dir_component_is_harmless() {
        let got = safe_join(Path::new("/fw"), "./boot.img").unwrap();
        assert_eq!(got, Path::new("/fw").join("boot.img"));
    }

    #[test]
    fn parent_traversal_rejected() {
        assert!(safe_join(Path::new("/fw"), "../secret").is_err());
        assert!(safe_join(Path::new("/fw"), "a/../../secret").is_err());
    }

    #[test]
    fn absolute_rejected() {
        assert!(safe_join(Path::new("/fw"), "/etc/passwd").is_err());
    }

    #[test]
    fn empty_or_dot_only_rejected() {
        assert!(safe_join(Path::new("/fw"), "").is_err());
        assert!(safe_join(Path::new("/fw"), "   ").is_err());
        assert!(safe_join(Path::new("/fw"), ".").is_err());
    }

    #[cfg(windows)]
    #[test]
    fn windows_prefix_and_backslash_traversal_rejected() {
        // Every Windows path-prefix shape is rejected via the
        // `Component::Prefix` arm, and a backslash `..` is a `ParentDir`.
        // Drive-absolute:
        assert!(safe_join(Path::new("C:\\fw"), "C:\\Windows\\x").is_err());
        // Drive-relative (no root after the colon — resolves against the
        // drive's current directory, not `base`):
        assert!(safe_join(Path::new("C:\\fw"), "C:relative.img").is_err());
        // UNC share:
        assert!(safe_join(Path::new("C:\\fw"), "\\\\server\\share\\x").is_err());
        // Verbatim / extended-length prefix:
        assert!(safe_join(Path::new("C:\\fw"), "\\\\?\\C:\\x").is_err());
        // Backslash parent traversal:
        assert!(safe_join(Path::new("C:\\fw"), "..\\..\\x").is_err());
    }
}
