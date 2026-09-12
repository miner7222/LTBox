//! OS theme detection — `true` when the host OS is in dark mode.
//!
//! Backed by the `dark-light` crate so the same probe works on
//! Windows (registry: `AppsUseLightTheme`), macOS (NSAppearance via
//! the Cocoa runtime), and the major Linux desktops (GNOME's
//! `org.gnome.desktop.interface color-scheme`, KDE's
//! `kdeglobals` ColorScheme key, plus the freedesktop XDG portal).
//!
//! Earlier this module hand-rolled a `RegGetValueW` call for
//! Windows + a `false` stub for everything else. Routing through
//! `dark-light` removes the platform-specific FFI from this crate
//! and lets the "Follow system" theme toggle in Settings actually
//! work on Linux + macOS without any further per-platform wiring.

pub fn system_prefers_dark() -> bool {
    // `dark-light::detect()` returns `Mode::Dark | Light | Unspecified`.
    // Treat `Unspecified` as light — same fallback the legacy Windows
    // probe used when the registry key was missing, and the safer
    // default for accessibility (light text on a dark surface is the
    // higher-contrast failure mode).
    matches!(dark_light::detect(), Ok(dark_light::Mode::Dark))
}

/// Probe once at startup, avoiding platform calls on animation frames.
pub fn reduced_motion() -> bool {
    static REDUCED: std::sync::LazyLock<bool> = std::sync::LazyLock::new(detect_reduced_motion);
    *REDUCED
}

#[cfg(windows)]
fn detect_reduced_motion() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW,
    };
    let mut enabled: i32 = 1;
    // SAFETY: this query writes one BOOL into a valid, writable i32.
    let success = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            (&mut enabled as *mut i32).cast(),
            0,
        )
    };
    success != 0 && enabled == 0
}

#[cfg(not(windows))]
fn detect_reduced_motion() -> bool {
    #[cfg(target_os = "macos")]
    let output = std::process::Command::new("defaults")
        .args(["read", "com.apple.universalaccess", "reduceMotion"])
        .output();
    #[cfg(not(target_os = "macos"))]
    let output = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "enable-animations"])
        .output();
    output.ok().filter(|o| o.status.success()).is_some_and(|o| {
        let value = String::from_utf8_lossy(&o.stdout);
        #[cfg(target_os = "macos")]
        {
            value.trim() == "1"
        }
        #[cfg(not(target_os = "macos"))]
        {
            value.trim() == "false"
        }
    })
}
