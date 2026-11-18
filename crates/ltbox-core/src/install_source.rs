//! Detects how the running LTBox executable was installed.
//!
//! LTBox packages place a plain-text marker named `ltbox.install-source` next
//! to the executable, in `Contents/Resources` of a macOS app bundle, or at
//! `/usr/share/ltbox/ltbox.install-source` for Unix system packages. Its single
//! line is one of `scoop`, `winget`, `homebrew`, `deb` or `rpm`; see
//! `parse_source` for the other accepted spellings. Parsing is ASCII
//! case-insensitive and ignores surrounding whitespace. A package-managed
//! location stays package-managed without a valid marker, so a third-party
//! package is never mistaken for a direct download.
//!
//! `LTBOX_INSTALL_SOURCE` accepts the same values and takes precedence over
//! both the marker and the executable location.

use std::path::{Path, PathBuf};

/// Filename installed next to the executable by LTBox-owned packages.
pub const INSTALL_SOURCE_MARKER_FILE: &str = "ltbox.install-source";

/// Marker location for Unix packages whose executable is under `/usr`.
pub const SYSTEM_INSTALL_SOURCE_MARKER_PATH: &str = "/usr/share/ltbox/ltbox.install-source";

/// Environment override understood by [`install_source`].
pub const INSTALL_SOURCE_ENV: &str = "LTBOX_INSTALL_SOURCE";

/// Provenance of an LTBox installation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum InstallSource {
    Scoop,
    WinGet,
    Homebrew,
    Deb,
    Rpm,
    /// A package-managed location whose exact manager is unknown.
    OtherPackageManager,
    Direct,
}

impl InstallSource {
    /// Whether the executable is owned by a package manager and must not
    /// overwrite itself.
    pub const fn is_package_managed(self) -> bool {
        !matches!(self, Self::Direct)
    }
}

/// Return the marker path associated with `executable_path`.
pub fn marker_path_for(executable_path: &Path) -> PathBuf {
    executable_path.with_file_name(INSTALL_SOURCE_MARKER_FILE)
}

/// Marker path inside a macOS `.app` bundle, when `executable_path` is one.
///
/// `Contents/MacOS` is a code directory: `codesign` treats every file there as
/// a code object, so a plain marker beside the executable makes signing the
/// bundle fail outright. The marker ships as a bundle resource instead.
pub fn bundle_marker_path_for(executable_path: &Path) -> Option<PathBuf> {
    let macos_dir = executable_path.parent()?;
    if macos_dir.file_name()? != "MacOS" {
        return None;
    }
    Some(
        macos_dir
            .parent()?
            .join("Resources")
            .join(INSTALL_SOURCE_MARKER_FILE),
    )
}

/// Classify an install without consulting the process or filesystem.
///
/// `env_override` has highest priority, followed by the executable-adjacent
/// marker and then the system marker. The system marker is considered only for
/// a known package-managed executable location, so a system package cannot
/// classify an unrelated binary in a user's downloads directory. A `direct`
/// marker cannot make a known system-owned location self-updatable; only the
/// explicit environment override can force that result.
pub fn classify_install_source(
    executable_path: &Path,
    marker_contents: Option<&str>,
    system_marker_contents: Option<&str>,
    env_override: Option<&str>,
) -> InstallSource {
    if let Some(source) = env_override.and_then(parse_source) {
        return source;
    }

    let package_managed_path = is_package_managed_path(executable_path);
    if let Some(source) = marker_contents.and_then(parse_source)
        && (source != InstallSource::Direct || !package_managed_path)
    {
        return source;
    }

    if package_managed_path
        && let Some(source) = system_marker_contents.and_then(parse_source)
        && source != InstallSource::Direct
    {
        return source;
    }

    if package_managed_path {
        InstallSource::OtherPackageManager
    } else {
        InstallSource::Direct
    }
}

/// Detect the provenance of the running LTBox executable.
pub fn install_source() -> InstallSource {
    let env_override = std::env::var(INSTALL_SOURCE_ENV).ok();
    let Ok(executable_path) = std::env::current_exe() else {
        return classify_install_source(Path::new(""), None, None, env_override.as_deref());
    };
    let marker_contents = std::fs::read_to_string(marker_path_for(&executable_path))
        .ok()
        .or_else(|| {
            bundle_marker_path_for(&executable_path)
                .and_then(|path| std::fs::read_to_string(path).ok())
        });
    #[cfg(unix)]
    let system_marker_contents = std::fs::read_to_string(SYSTEM_INSTALL_SOURCE_MARKER_PATH).ok();
    #[cfg(not(unix))]
    let system_marker_contents: Option<String> = None;
    classify_install_source(
        &executable_path,
        marker_contents.as_deref(),
        system_marker_contents.as_deref(),
        env_override.as_deref(),
    )
}

fn parse_source(value: &str) -> Option<InstallSource> {
    match value.trim().to_ascii_lowercase().as_str() {
        "scoop" => Some(InstallSource::Scoop),
        "winget" => Some(InstallSource::WinGet),
        "homebrew" | "brew" => Some(InstallSource::Homebrew),
        "deb" => Some(InstallSource::Deb),
        "rpm" => Some(InstallSource::Rpm),
        "other" | "unknown" | "package-manager" => Some(InstallSource::OtherPackageManager),
        "direct" => Some(InstallSource::Direct),
        _ => None,
    }
}

fn is_package_managed_path(executable_path: &Path) -> bool {
    let path = executable_path
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();

    // Scoop can be relocated, so match its directory shape, not a home prefix.
    if path.contains("/apps/ltbox/") {
        return true;
    }

    // Match WinGet's stable path suffix to stay pure (no LocalAppData lookup).
    if path.contains("/microsoft/winget/packages/") {
        return true;
    }

    let path_without_drive = path
        .strip_prefix(|character: char| character.is_ascii_alphabetic())
        .and_then(|rest| rest.strip_prefix(':'))
        .unwrap_or(&path);

    [
        "/program files/",
        "/program files (x86)/",
        "/applications/",
        "/opt/homebrew/",
        "/usr/",
        "/opt/",
        "/snap/",
        "/nix/store/",
    ]
    .iter()
    .any(|prefix| path_without_drive.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognised_package_paths_are_managed_without_markers() {
        for path in [
            r"C:\Users\user\scoop\apps\ltbox\1.2.3\ltbox.exe",
            r"D:\PortableScoop\apps\ltbox\current\ltbox.exe",
            r"C:\Users\user\AppData\Local\Microsoft\WinGet\Packages\LTBox\ltbox.exe",
            r"C:\Program Files\LTBox\ltbox.exe",
            r"C:\Program Files (x86)\LTBox\ltbox.exe",
            "/Applications/LTBox.app/Contents/MacOS/ltbox",
            "/opt/homebrew/Caskroom/ltbox/1.2.3/LTBox.app/Contents/MacOS/ltbox",
            "/usr/local/Caskroom/ltbox/1.2.3/LTBox.app/Contents/MacOS/ltbox",
            "/usr/bin/ltbox",
            "/usr/local/bin/ltbox",
            "/opt/ltbox/ltbox",
            "/snap/ltbox/current/ltbox",
            "/nix/store/hash-ltbox/bin/ltbox",
        ] {
            let source = classify_install_source(Path::new(path), None, None, None);
            assert_eq!(
                source,
                InstallSource::OtherPackageManager,
                "{path} should be package-managed"
            );
            assert!(source.is_package_managed());
        }
    }

    #[test]
    fn markers_name_each_exact_package_channel() {
        for (marker, expected) in [
            ("scoop", InstallSource::Scoop),
            (" WinGet\r\n", InstallSource::WinGet),
            ("HOMEBREW\n", InstallSource::Homebrew),
            ("deb ", InstallSource::Deb),
            ("RpM", InstallSource::Rpm),
            ("other", InstallSource::OtherPackageManager),
        ] {
            assert_eq!(
                classify_install_source(Path::new("/usr/bin/ltbox"), Some(marker), None, None),
                expected
            );
        }
    }

    #[test]
    fn marker_in_non_system_path_still_identifies_package() {
        assert_eq!(
            classify_install_source(
                Path::new("/home/user/tools/ltbox"),
                Some("homebrew"),
                None,
                None,
            ),
            InstallSource::Homebrew
        );
        assert_eq!(
            classify_install_source(
                Path::new("/home/user/Downloads/ltbox"),
                Some("direct"),
                None,
                None,
            ),
            InstallSource::Direct
        );
    }

    #[test]
    fn unrecognised_marker_is_ignored() {
        assert_eq!(
            classify_install_source(
                Path::new("/home/user/Downloads/ltbox"),
                Some("future-manager"),
                None,
                None,
            ),
            InstallSource::Direct
        );
        assert_eq!(
            classify_install_source(
                Path::new("/usr/bin/ltbox"),
                Some("future-manager"),
                None,
                None,
            ),
            InstallSource::OtherPackageManager
        );
    }

    #[test]
    fn environment_override_can_force_channel_or_direct() {
        assert_eq!(
            classify_install_source(
                Path::new("/home/user/Downloads/ltbox"),
                None,
                None,
                Some(" RPM "),
            ),
            InstallSource::Rpm
        );
        assert_eq!(
            classify_install_source(
                Path::new("/usr/bin/ltbox"),
                Some("deb"),
                None,
                Some("direct"),
            ),
            InstallSource::Direct
        );
    }

    #[test]
    fn direct_marker_cannot_make_system_path_self_updatable() {
        assert_eq!(
            classify_install_source(Path::new("/usr/bin/ltbox"), Some("direct"), None, None,),
            InstallSource::OtherPackageManager
        );
    }

    #[test]
    fn ordinary_user_path_is_direct() {
        assert_eq!(
            classify_install_source(Path::new("/home/user/Downloads/ltbox"), None, None, None),
            InstallSource::Direct
        );
        assert_eq!(
            classify_install_source(
                Path::new(r"C:\Users\user\Downloads\ltbox.exe"),
                None,
                None,
                None,
            ),
            InstallSource::Direct
        );
    }

    #[test]
    fn system_marker_names_linux_package_channel() {
        assert_eq!(
            classify_install_source(Path::new("/usr/bin/ltbox"), None, Some("deb\n"), None),
            InstallSource::Deb
        );
        assert_eq!(
            classify_install_source(Path::new("/usr/bin/ltbox"), None, Some(" RPM "), None),
            InstallSource::Rpm
        );
    }

    #[test]
    fn adjacent_marker_precedes_system_marker() {
        assert_eq!(
            classify_install_source(
                Path::new("/usr/bin/ltbox"),
                Some("homebrew"),
                Some("deb"),
                None,
            ),
            InstallSource::Homebrew
        );
    }

    #[test]
    fn system_marker_cannot_classify_an_unmanaged_path() {
        assert_eq!(
            classify_install_source(
                Path::new("/home/user/Downloads/ltbox"),
                None,
                Some("deb"),
                None,
            ),
            InstallSource::Direct
        );
    }

    #[test]
    fn direct_system_marker_cannot_downgrade_a_system_path() {
        assert_eq!(
            classify_install_source(Path::new("/usr/bin/ltbox"), None, Some("direct"), None,),
            InstallSource::OtherPackageManager
        );
    }

    #[test]
    fn marker_path_is_next_to_executable() {
        assert_eq!(
            marker_path_for(Path::new("/usr/bin/ltbox")),
            PathBuf::from("/usr/bin/ltbox.install-source")
        );
        assert_eq!(
            SYSTEM_INSTALL_SOURCE_MARKER_PATH,
            "/usr/share/ltbox/ltbox.install-source"
        );
    }

    #[test]
    fn bundle_marker_lives_in_resources_not_the_code_directory() {
        // `codesign` rejects a data file under `Contents/MacOS`, so the macOS
        // marker is a bundle resource.
        assert_eq!(
            bundle_marker_path_for(Path::new("/Applications/LTBox.app/Contents/MacOS/ltbox")),
            Some(PathBuf::from(
                "/Applications/LTBox.app/Contents/Resources/ltbox.install-source"
            ))
        );
        assert_eq!(bundle_marker_path_for(Path::new("/usr/bin/ltbox")), None);
        assert_eq!(bundle_marker_path_for(Path::new("ltbox")), None);
    }
}
