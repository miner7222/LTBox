//! Platform installer entry points.

#[cfg(target_os = "linux")]
use crate::APP_ID;

/// Embedded udev rules — installable from binary without source tree.
#[cfg(target_os = "linux")]
const UDEV_RULES_CONTENT: &str = include_str!("../../../misc/udev/51-ltbox-qcom.rules");

#[cfg(target_os = "linux")]
const UDEV_RULES_PATH: &str = "/etc/udev/rules.d/51-ltbox-qcom.rules";

/// Embedded `.desktop` template — installable via `--install-desktop`.
#[cfg(target_os = "linux")]
const DESKTOP_FILE_TEMPLATE: &str =
    include_str!("../../../misc/desktop/io.github.miner7222.LTBox.desktop");

#[cfg(target_os = "linux")]
const APP_ICON_SVG: &str = include_str!("../assets/icon_source.svg");

/// `ltbox --install-udev` entry point. Writes bundled rules, reloads
/// udev, triggers it, exits. Linux-only — invoke via `sudo` or
/// `pkexec`. Windows / macOS print a one-line refusal.
#[cfg(target_os = "linux")]
pub(super) fn install_udev_rules() -> ! {
    eprintln!("[ltbox] Installing udev rules → {UDEV_RULES_PATH}");
    if let Err(e) = std::fs::write(UDEV_RULES_PATH, UDEV_RULES_CONTENT) {
        eprintln!("[ltbox] write {UDEV_RULES_PATH}: {e}");
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            let exe = std::env::current_exe()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| "ltbox".into());
            eprintln!();
            eprintln!("[ltbox] Permission denied — needs root. Re-run as:");
            eprintln!("  sudo {exe} --install-udev");
            eprintln!("  pkexec {exe} --install-udev");
        }
        std::process::exit(1);
    }
    // Force world-readable: a hardened root umask (0077) would leave 0600,
    // which udev can't apply and the GUI's read-back (as the normal user)
    // would report as `UdevRulesNoPermission`.
    use std::os::unix::fs::PermissionsExt;
    if let Err(e) =
        std::fs::set_permissions(UDEV_RULES_PATH, std::fs::Permissions::from_mode(0o644))
    {
        eprintln!("[ltbox] WARNING: chmod 0644 {UDEV_RULES_PATH}: {e}");
    }
    let reload_ok = std::process::Command::new("udevadm")
        .args(["control", "--reload"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !reload_ok {
        eprintln!(
            "[ltbox] WARNING: `udevadm control --reload` failed (rules still on disk; reboot will pick them up)"
        );
    }
    let trigger_ok = std::process::Command::new("udevadm")
        .arg("trigger")
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !trigger_ok {
        eprintln!(
            "[ltbox] WARNING: `udevadm trigger` failed (replug device manually to apply rules)"
        );
    }
    eprintln!();
    eprintln!(
        "[ltbox] Done. Replug a connected Qualcomm 9008 / Lenovo USB device for the new ACL grants to take effect."
    );
    std::process::exit(0);
}

#[cfg(not(target_os = "linux"))]
pub(super) fn install_udev_rules() -> ! {
    eprintln!("[ltbox] --install-udev is Linux-only — udev does not exist on this host.");
    std::process::exit(1);
}

/// Escape both the desktop string layer and the Exec argument layer.
#[cfg(any(target_os = "linux", test))]
fn desktop_exec(executable: &str) -> Option<String> {
    if executable
        .chars()
        .any(|c| c == '\n' || c == '\r' || c == '\0')
    {
        return None;
    }
    let mut result = String::from("\"");
    for ch in executable.chars() {
        match ch {
            '\\' => result.push_str(r"\\\\"),
            '"' | '`' | '$' => {
                result.push_str(r"\\");
                result.push(ch);
            }
            '%' => result.push_str("%%"),
            _ => result.push(ch),
        }
    }
    result.push('"');
    Some(result)
}

/// `ltbox --install-desktop` entry point. Linux only. Per-user install
/// under `$XDG_DATA_HOME` (default `~/.local/share`); refreshes the
/// desktop and icon caches. The `__LTBOX_EXEC__` placeholder in the bundled
/// `.desktop` is substituted with `current_exe()` at install time so a
/// tarball-extracted binary works without being on PATH.
#[cfg(target_os = "linux")]
pub(super) fn install_desktop_file() -> ! {
    use std::fs;
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share"))
        });
    let Some(data_home) = data_home else {
        eprintln!("[ltbox] $HOME and $XDG_DATA_HOME both unset; cannot resolve install dir.");
        std::process::exit(1);
    };

    let apps_dir = data_home.join("applications");
    let icons_dir = data_home.join("icons/hicolor/scalable/apps");
    let desktop_path = apps_dir.join(format!("{APP_ID}.desktop"));
    let icon_path = icons_dir.join(format!("{APP_ID}.svg"));

    if let Err(e) = fs::create_dir_all(&apps_dir) {
        eprintln!("[ltbox] mkdir {}: {e}", apps_dir.display());
        std::process::exit(1);
    }
    if let Err(e) = fs::create_dir_all(&icons_dir) {
        eprintln!("[ltbox] mkdir {}: {e}", icons_dir.display());
        std::process::exit(1);
    }

    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "ltbox".into());
    let Some(quoted) = desktop_exec(&exe) else {
        eprintln!("[ltbox] Executable path cannot be represented in a desktop entry.");
        std::process::exit(1);
    };
    let desktop = DESKTOP_FILE_TEMPLATE.replace("__LTBOX_EXEC__", &quoted);

    eprintln!("[ltbox] Writing desktop entry → {}", desktop_path.display());
    if let Err(e) = fs::write(&desktop_path, desktop) {
        eprintln!("[ltbox] write {}: {e}", desktop_path.display());
        std::process::exit(1);
    }

    eprintln!("[ltbox] Writing icon            → {}", icon_path.display());
    if let Err(e) = fs::write(&icon_path, APP_ICON_SVG) {
        eprintln!("[ltbox] write {}: {e}", icon_path.display());
        std::process::exit(1);
    }

    // Best-effort cache refresh; a missing tool or cache file must not abort
    // the install.
    let _ = std::process::Command::new("update-desktop-database")
        .arg(&apps_dir)
        .status();
    let _ = std::process::Command::new("gtk-update-icon-cache")
        .arg("-q")
        .arg(data_home.join("icons/hicolor"))
        .status();

    eprintln!();
    eprintln!(
        "[ltbox] Done. The entry should appear in your app menu within a few seconds. \
         Re-run with `--install-desktop` after moving the binary."
    );
    std::process::exit(0);
}

#[cfg(not(target_os = "linux"))]
pub(super) fn install_desktop_file() -> ! {
    eprintln!(
        "[ltbox] --install-desktop is Linux-only — desktop entries follow the freedesktop.org spec."
    );
    std::process::exit(1);
}

#[cfg(test)]
mod desktop_tests {
    use super::desktop_exec;
    #[test]
    fn exec_quotes_spaces_and_escapes_field_codes_and_metacharacters() {
        assert_eq!(
            desktop_exec("/home/a b/ltbox"),
            Some("\"/home/a b/ltbox\"".into())
        );
        assert_eq!(
            desktop_exec("/tmp/100%/ltbox"),
            Some("\"/tmp/100%%/ltbox\"".into())
        );
        assert!(desktop_exec("/tmp/bad\nExec=other").is_none());
        for character in ['$', '`', '"'] {
            assert!(
                desktop_exec(&character.to_string())
                    .unwrap()
                    .contains(&format!(r"\\{character}"))
            );
        }
        assert_eq!(desktop_exec(r"a\b").unwrap(), "\"a\\\\\\\\b\"");
    }
}
