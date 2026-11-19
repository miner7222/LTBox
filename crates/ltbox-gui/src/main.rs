#![windows_subsystem = "windows"]
//! LTBox GUI — iced desktop shell.
//!
//! Orchestrates `ltbox-core`, `ltbox-device` and `ltbox-patch` through a
//! sidebar and wizard UX. [`main`] handles startup (single-instance lock, AppUserModelID,
//! window + font bundle); [`App`] owns every wizard state machine, the device
//! poll subscription, persisted settings and the active palette.
//!
//! Wizards: Flash · SystemUpdate · Root · Unroot · KonaBess · Reboot · Advanced.
//! Sub-modules: [`theme`] M3 tokens · [`settings_store`] `settings.json`
//! in the user config dir · [`stdout_tap`] native-crate log capture.

#[allow(dead_code)]
mod icon {
    include!(concat!(env!("OUT_DIR"), "/icon.rs"));
}
mod arb;
mod arb_overlay;
mod backup;
mod country_flags;
#[cfg(feature = "demo")]
mod demo;
#[cfg(feature = "demo")]
mod demo_logs;
mod device_choice;
mod device_name;
mod device_queries;
mod device_snapshot;
mod focus_button;
mod layout_constraints;
mod loader;
mod log_history;
mod log_messages;
use log_messages::LiveLabels;
mod app;
#[cfg(test)]
mod manual_rollback_tests;
mod message;
mod model;
mod navigation;
mod operation_execution;
mod operation_phase;
mod pickers;
mod platform_installers;
mod poll_state;
mod root_choice;
mod root_manager;
mod self_update;
mod settings_state;
mod settings_store;
mod single_instance;
mod software_fix;
mod stdout_tap;
mod theme;
mod theme_detect;
mod translations;
mod update;
mod view;
mod widgets;
mod workers;
pub(crate) use app::*;
pub(crate) use navigation::*;
pub(crate) use poll_state::*;
pub(crate) use root_choice::*;
pub(crate) use settings_state::*;

// Re-exported so the rest of the crate refers to these unqualified.
pub(crate) use arb::{detect_arb_run, format_unix_date_utc, format_unix_timestamp_utc};
pub(crate) use arb_overlay::*;
pub(crate) use device_name::*;
use device_queries::{DeviceQueries, LookupKind};
use device_snapshot::DeviceSnapshot;
pub(crate) use layout_constraints::*;
pub(crate) use loader::*;
pub(crate) use message::*;
pub(crate) use model::country::*;
pub(crate) use model::device::*;
pub(crate) use model::wizard::*;
use operation_execution::{OperationExecution, OperationKind};
pub(crate) use operation_phase::*;
use platform_installers::{install_desktop_file, install_udev_rules};
pub(crate) use root_manager::{
    install_root_manager_apk, stage_manager_apk_for_manual_install,
    wait_and_install_root_manager_apk,
};
pub(crate) use self_update::{DirectUpdateState, SelfUpdateFailure, SelfUpdateFailureKind};
pub(crate) use translations::*;
pub(crate) use view::components::*;
pub(crate) use view::styles::*;
pub(crate) use widgets::*;
pub(crate) use workers::advanced::*;
pub(crate) use workers::edl_transition::*;
pub(crate) use workers::flash::*;
pub(crate) use workers::konabess::*;
pub(crate) use workers::reboot::*;
pub(crate) use workers::root::*;
pub(crate) use workers::sysupdate::*;
pub(crate) use workers::transfer::*;
pub(crate) use workers::unroot::*;

use ltbox_core::{live, tr_args};

use crate::focus_button::{self as button, button};
use iced::widget::{Space, row, text};
use iced::{Element, Length, Subscription, Task, Theme};

use theme::{Palette, ThemeSeed, palette_for, with_alpha};

/// Palette lookup from `iced` style closures that only have `&Theme`.
fn pal_of(t: &Theme) -> Palette {
    theme::active_palette_for(t)
}

/// Upper bound on `App.log_lines` — keeps memory flat over long sessions.
const LOG_MAX_LINES: usize = 500;
const EXEC_ERROR_SUMMARY_MAX_CHARS: usize = 180;

/// 32×32 RGBA handle for the custom title-bar brand icon (Windows / Linux;
/// macOS uses the native title bar, see [`SYSTEM_WINDOW_CHROME`]).
static TITLE_BAR_ICON_HANDLE: std::sync::LazyLock<iced::widget::image::Handle> =
    std::sync::LazyLock::new(|| {
        let bytes: &'static [u8] = include_bytes!("../assets/icon_32.bin");
        iced::widget::image::Handle::from_rgba(32, 32, bytes.to_vec())
    });

/// Reverse-DNS app id: Wayland `app_id` / X11 `WM_CLASS` via iced
/// `Settings::id`; matches the shipped `.desktop`'s `StartupWMClass=` so the
/// window binds to the launcher entry.
const APP_ID: &str = "io.github.miner7222.LTBox";

/// Initial window size on first run (logical pixels), used by `main`'s
/// `window::Settings::size` fallback and by `App::default`; keep them equal.
const DEFAULT_WINDOW_WIDTH: f32 = 820.0;
const DEFAULT_WINDOW_HEIGHT: f32 = 720.0;
/// Floor for cursor-drag resize and the launch-time geometry
/// (`window::Settings::min_size`). Below the width, wizard cards overlap and
/// the sidebar tween jumps.
///
/// The 720px height fits the Flash-confirm step without scrolling; taller
/// step bodies and the navigation list scroll within their panels.
const MIN_WINDOW_WIDTH: f32 = 820.0;
const MIN_WINDOW_HEIGHT: f32 = 720.0;
/// macOS uses native window chrome (title bar, traffic lights, resize edges);
/// Windows / Linux use LTBox's borderless title bar plus 8 overlaid resize
/// handles. Gates both `window::Settings::decorations` and the custom-chrome
/// widgets in `view::chrome`.
pub(crate) const SYSTEM_WINDOW_CHROME: bool = cfg!(target_os = "macos");
/// Quiet period after the last resize before the window size is written.
/// Cursor-drag resize emits `Event::Window(Resized)` continuously.
const WINDOW_SIZE_SAVE_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);

/// Upstream repo for the status-bar update affordance.
const UPDATE_REPO: &str = "miner7222/LTBox";

/// Background probe for the status-bar update affordance: the latest
/// non-draft, non-prerelease release (via `/releases?per_page=100`) whose
/// semver beats `CARGO_PKG_VERSION`. `None` on network/parse failure or when
/// already current. Blocking; runs on a `spawn_blocking` worker and lands as
/// `Message::UpdateCheckDone`.
fn check_for_update() -> Option<ltbox_core::github::StableRelease> {
    let current = semver::Version::parse(env!("CARGO_PKG_VERSION")).ok()?;
    let client = ltbox_core::github::GitHubClient::new(UPDATE_REPO).ok()?;
    let stable = client.latest_stable_release().ok().flatten()?;
    let stable_ver = semver::Version::parse(stable.tag.trim_start_matches('v')).ok()?;
    if stable_ver > current {
        Some(stable)
    } else {
        None
    }
}

/// Package-manager command shown by the update dialog. Independent of GUI
/// state; unknown package managers get no guessed command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PackageUpgradeCommand {
    command: &'static str,
    available: bool,
}

const fn package_upgrade_command(
    source: ltbox_core::install_source::InstallSource,
) -> PackageUpgradeCommand {
    use ltbox_core::install_source::InstallSource;

    let command = match source {
        InstallSource::Scoop => "scoop update ltbox",
        InstallSource::WinGet => "winget upgrade miner7222.LTBox",
        InstallSource::Homebrew => "brew upgrade --cask ltbox",
        InstallSource::Deb => "sudo apt update && sudo apt upgrade ltbox",
        InstallSource::Rpm => "sudo dnf upgrade ltbox",
        InstallSource::OtherPackageManager | InstallSource::Direct => "",
        _ => "",
    };
    PackageUpgradeCommand {
        command,
        available: !command.is_empty(),
    }
}

fn main() -> iced::Result {
    #[cfg(feature = "demo")]
    {
        let mut args = std::env::args_os().skip(1);
        if args.next().as_deref() == Some(std::ffi::OsStr::new("--demo-log-catalog")) {
            let directory = args
                .next()
                .expect("--demo-log-catalog requires an output directory");
            demo_logs::export(std::path::Path::new(&directory))
                .expect("failed to write the synthetic log catalogue");
            return Ok(());
        }
    }
    if let Some(code) = ltbox_patch::boot::dispatch_magiskboot_helper() {
        std::process::exit(code);
    }
    if let Ok(executable) = std::env::current_exe() {
        // Register before any worker starts. GUI releases need no companion binary.
        let _ = ltbox_patch::boot::register_magiskboot_host(executable);
    }
    // Linux/X11 renderer default: on some X11 + Mesa/driver combos wgpu picks a
    // Vulkan adapter whose surface creation fails and the window never appears.
    // Default to GL on an X11 session when the user hasn't picked a backend;
    // Wayland keeps the wgpu default (Vulkan), relied on for recent Nvidia.
    // Override with e.g. `WGPU_BACKEND=vulkan ./ltbox`.
    #[cfg(target_os = "linux")]
    {
        // Treat a var as set only when it is non-empty — winit reads these
        // the same way (an empty value means "unset").
        let non_empty = |key: &str| std::env::var_os(key).is_some_and(|v| !v.is_empty());
        // Only the singular WGPU_BACKEND is read by this iced/wgpu stack; honoring
        // the plural WGPU_BACKENDS would let an ignored value suppress the fallback.
        let backend_chosen = non_empty("WGPU_BACKEND");
        // winit selects Wayland when WAYLAND_DISPLAY or WAYLAND_SOCKET is set,
        // otherwise X11 via DISPLAY — mirror that to scope the override to
        // pure-X11 sessions only.
        let wayland_session = non_empty("WAYLAND_DISPLAY") || non_empty("WAYLAND_SOCKET");
        let is_x11_session = !wayland_session && non_empty("DISPLAY");
        if !backend_chosen && is_x11_session {
            // SAFETY: still among the first statements of `main`, before the stdout
            // tap, tracing writer, tokio or iced spawn any threads, so the process is
            // single-threaded as `set_var` requires.
            unsafe {
                std::env::set_var("WGPU_BACKEND", "gl");
            }
        }
    }

    // Windows renderer default: left alone wgpu may pick OpenGL, which on hybrid
    // laptops goes through the integrated GPU's OpenGL ICD (e.g. AMD's
    // `atio6axx.dll`) and crashes with an access violation (c0000005) on some
    // driver/GPU combos. Default to DX12 when the user hasn't picked a backend.
    // `ICED_BACKEND=tiny-skia` selects the software renderer; override the
    // backend with e.g. `WGPU_BACKEND=vulkan ltbox.exe`.
    #[cfg(target_os = "windows")]
    {
        // Treat a var as set only when non-empty (an empty value reads as
        // unset), matching the Linux branch above.
        let backend_chosen = std::env::var_os("WGPU_BACKEND").is_some_and(|v| !v.is_empty());
        if !backend_chosen {
            // SAFETY: still in the first statements of `main`, before the
            // stdout tap, tracing, tokio, or iced spawn any threads — the
            // process is single-threaded as `set_var` requires.
            unsafe {
                std::env::set_var("WGPU_BACKEND", "dx12");
            }
        }
    }

    // Pre-iced CLI flags (`--install-udev`, `--install-desktop`); each handler
    // exits the process, so iced setup runs only when neither is given. Plain
    // presence checks, no `clap`.
    let args: Vec<String> = std::env::args().collect();
    let post_update_relaunch = args
        .iter()
        .any(|argument| argument == self_update::POST_UPDATE_RELAUNCH_ARG);
    if args.iter().any(|a| a == "--install-udev") {
        install_udev_rules();
    }
    if args.iter().any(|a| a == "--install-desktop") {
        install_desktop_file();
    }

    // Preserve the version-agnostic path for compatibility with running older
    // releases. Failure to create or acquire a usable guard must fail closed.
    let lock_path = std::env::temp_dir().join("ltbox-gui-singleton.lock");
    let _instance_guard = match single_instance::acquire(&lock_path, post_update_relaunch) {
        Ok(Some(file)) => file,
        Ok(None) => return Ok(()),
        Err(error) => {
            let description = format!(
                "Cannot acquire the LTBox instance lock at {}: {error}",
                lock_path.display()
            );
            eprintln!("{description}");
            rfd::MessageDialog::new()
                .set_title("LTBox")
                .set_description(description)
                .set_level(rfd::MessageLevel::Error)
                .show();
            return Ok(());
        }
    };
    self_update::cleanup_stale_update_backups();

    // libusb (via `adb_client` → `rusb`) calls LoadLibrary on
    // `%SystemRoot%\System32\libusbK.dll` and treats a NULL handle as "backend
    // unavailable". If that DLL is present but corrupt, the Windows hard-error
    // handler raises an undismissable "Bad Image" box (0xc000012f) on every
    // probe. SEM_FAILCRITICALERRORS turns it into the NULL libusb handles, leaving
    // WinUSB and every `nusb` EDL path (which never touches libusbK) working.
    // Must run before the first USB probe.
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Diagnostics::Debug::{
            SEM_FAILCRITICALERRORS, SEM_NOOPENFILEERRORBOX, SetErrorMode,
        };
        unsafe {
            SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOOPENFILEERRORBOX);
        }
    }

    // Override AppUserModelID so taskbar / jump-list show "LTBox"
    // instead of the Cargo crate name. Must run before window creation.
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
        let id: Vec<u16> = "LTBox.App\0".encode_utf16().collect();
        unsafe {
            SetCurrentProcessExplicitAppUserModelID(id.as_ptr());
        }
    }

    // Must run before any stdout write — the pipe has to be live
    // before the first `println!` resolves.
    ltbox_core::live_sink::attach_gui();
    stdout_tap::install();

    // `_log_guard` must live for the whole process: dropping it flushes the
    // non-blocking writer, and losing it loses the last events on a crash.
    let _log_guard = init_tracing();

    // Package-manager upgrades replace/prune executable directories. Move any
    // v3 executable-adjacent data before the first worker creates the new
    // `%LOCALAPPDATA%\ltbox` destinations. Individual failures are non-fatal.
    #[cfg(windows)]
    for error in ltbox_core::app_paths::migrate_legacy_windows_data() {
        tracing::warn!("{error}");
    }

    // The error-mode guard above turns a corrupt system libusbK.dll into a
    // silent NULL, so name it here instead: this is the whole diagnosis for a
    // report that otherwise arrives as a screenshot of a Windows dialog.
    #[cfg(windows)]
    if let Some(system_root) = std::env::var_os("SystemRoot") {
        let dll = std::path::Path::new(&system_root)
            .join("System32")
            .join("libusbK.dll");
        // Only a present-but-not-a-PE file is worth reporting. Absent is
        // normal (libusb falls back to WinUSB), and an unreadable file is
        // someone else's permissions problem.
        let head = std::fs::File::open(&dll).and_then(|mut f| {
            use std::io::Read;
            let mut magic = [0u8; 2];
            f.read_exact(&mut magic).map(|()| magic)
        });
        if let Ok(magic) = head
            && &magic != b"MZ"
        {
            tracing::warn!(
                path = %dll.display(),
                "system libusbK.dll is not a PE image; libusb's libusbK backend will be unavailable"
            );
        }
    }

    let win_icon =
        iced::window::icon::from_rgba(include_bytes!("../assets/icon_32.bin").to_vec(), 32, 32)
            .ok();
    // Restore the persisted window geometry, clamped to `MIN_WINDOW_*` so a
    // corrupt or pre-min-size config can't launch a sub-floor window; default
    // size on first run.
    let persisted = settings_store::load();
    let persisted_size = persisted
        .window_size
        .map(|(w, h)| iced::Size::new(w.max(MIN_WINDOW_WIDTH), h.max(MIN_WINDOW_HEIGHT)))
        .unwrap_or_else(|| iced::Size::new(DEFAULT_WINDOW_WIDTH, DEFAULT_WINDOW_HEIGHT));
    // Bind the UI face before the iced settings below read it. The saved
    // language decides which of the three bundled Noto faces renders Han
    // idiomatically; `App::default` reads the same value for its translations.
    theme::set_use_system_font(persisted.use_system_font);
    theme::set_font_family(theme::font_family_for_language(&persisted.language));
    let window_settings = iced::window::Settings {
        size: persisted_size,
        // `MIN_WINDOW_*` is the resize floor. With borderless decorations (Windows /
        // Linux) the GUI overlays 8 invisible resize handles that emit
        // `WindowMsg::WindowResize` and call `iced::window::drag_resize`; macOS keeps
        // native resize. The geometry persists to `PersistedSettings::window_size`
        // and is restored above on the next launch.
        min_size: Some(iced::Size::new(MIN_WINDOW_WIDTH, MIN_WINDOW_HEIGHT)),
        icon: win_icon,
        // macOS → native decorations (system title bar + resize); other
        // platforms → borderless + custom chrome (see SYSTEM_WINDOW_CHROME).
        decorations: SYSTEM_WINDOW_CHROME,
        ..Default::default()
    };
    // Noto subsets are bundled at compile time so cosmic-text can fall back for
    // Hangul / Hanzi glyphs; the same family covers Latin + Cyrillic.
    let mut app = iced::application(App::new, App::update, App::view)
        .title("LTBox")
        // Application id propagates to winit (Wayland `app_id`, X11 `WM_CLASS`) and
        // matches `StartupWMClass=` in the shipped `.desktop` file so GNOME / KDE bind
        // the window to the launcher entry instead of falling back to the binary name.
        .settings(iced::Settings {
            id: Some(APP_ID.to_string()),
            default_font: theme::default_font(),
            ..iced::Settings::default()
        })
        .theme(App::theme)
        .subscription(App::subscription)
        .exit_on_close_request(false)
        .window(window_settings);
    for bytes in [
        include_bytes!("../fonts/noto/NotoSansKR-Regular.subset.otf") as &[u8],
        include_bytes!("../fonts/noto/NotoSansKR-Medium.subset.otf") as &[u8],
        include_bytes!("../fonts/noto/NotoSansKR-Bold.subset.otf") as &[u8],
        include_bytes!("../fonts/noto/NotoSansJP-Regular.subset.otf") as &[u8],
        include_bytes!("../fonts/noto/NotoSansJP-Medium.subset.otf") as &[u8],
        include_bytes!("../fonts/noto/NotoSansJP-Bold.subset.otf") as &[u8],
        include_bytes!("../fonts/noto/NotoSansSC-Regular.subset.otf") as &[u8],
        include_bytes!("../fonts/noto/NotoSansSC-Medium.subset.otf") as &[u8],
        include_bytes!("../fonts/noto/NotoSansSC-Bold.subset.otf") as &[u8],
        // Latin-only, for the few slots that want fixed-width digits and
        // letters: file paths and country codes. `iced::Font::MONOSPACE` names
        // a family the bundle does not carry, so it would fall through to
        // whatever the system offers — Courier New on Windows.
        include_bytes!("../fonts/noto/NotoSansMono-Regular.subset.ttf") as &[u8],
    ] {
        app = app.font(bytes);
    }
    // Subset Lucide TTF generated at build time from `fonts/lucide.toml`,
    // registered as family "lucide" for the text-based icon widgets (`mod icon`).
    app = app.font(icon::FONT);
    let result = app.run();
    settings_store::flush();
    result
}

/// `<config>/ltbox/logs`, or `<temp>/ltbox-logs` when the config directory
/// is unknown or not UTF-8. Holds `ltbox.log` and the `sessions/` transcripts.
pub(crate) fn log_dir() -> std::path::PathBuf {
    dirs::config_dir()
        .map(|d| d.join("ltbox").join("logs"))
        .filter(|d| d.to_str().is_some())
        .unwrap_or_else(|| std::env::temp_dir().join("ltbox-logs"))
}

/// Default `RUST_LOG` directives for the log file.
///
/// * `adb_client` logs a line per connect, and the dashboard reconnects every
///   poll.
/// * `iced_winit` and `iced_wgpu` dump their window and compositor settings at
///   `info` on every launch. Linux prints the icon's RGBA buffer (about 4,000
///   lines for our 32x32 icon); Windows prints an icon handle and macOS NoIcon.
/// * `iced_futures::subscription::tracker` warns once per event it drops while a subscription's
///   channel is full, in bursts of hundreds.
///
/// Each is held back unless RUST_LOG asks for more.
const DEFAULT_LOG_FILTER: &str =
    "info,adb_client=warn,iced_winit=warn,iced_wgpu=warn,iced_futures::subscription::tracker=error";

/// Global tracing subscriber writing daily-rotated files under [`log_dir`].
/// Caller must hold the returned `WorkerGuard` for the process lifetime;
/// dropping it flushes queued entries. Filter: `RUST_LOG`, else
/// [`DEFAULT_LOG_FILTER`].
fn init_tracing() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    use camino::Utf8PathBuf;
    use tracing_subscriber::{EnvFilter, fmt};

    // `log_dir()` already falls back to the temp dir; this relative `ltbox-logs`
    // applies only if that path is not UTF-8 either.
    let log_dir: Utf8PathBuf =
        Utf8PathBuf::from_path_buf(log_dir()).unwrap_or_else(|_| Utf8PathBuf::from("ltbox-logs"));
    if std::fs::create_dir_all(&log_dir).is_err() {
        return None;
    }

    let file_appender = tracing_appender::rolling::daily(log_dir.as_std_path(), "ltbox.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_LOG_FILTER));

    // `init` rather than `set_global_default`: it also installs the `log` ->
    // `tracing` bridge. `adb_client` reports the device's CNXN banner (the real
    // connection state: `device::` / `recovery::` / `sideload::`) only through `log`.
    fmt()
        .with_env_filter(filter)
        .with_writer(non_blocking)
        .with_ansi(false)
        .with_target(true)
        .init();

    Some(guard)
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
