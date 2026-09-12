#![windows_subsystem = "windows"]
//! LTBox GUI — iced desktop shell for the v3.0.0 Rust rewrite.
//!
//! Orchestrates `ltbox-core`, `ltbox-device`, `ltbox-patch` through a
//! sidebar + wizard UX. [`main`] handles startup (single-instance lock,
//! AppUserModelID, window + font bundle); [`App`] owns every wizard
//! state machine, the device poll subscription, persisted settings,
//! and the active palette.
//!
//! Wizards: Flash · SystemUpdate · Root · Unroot · Reboot · Advanced.
//! Sub-modules: [`theme`] M3 tokens · [`settings_store`] `settings.json`
//! in the user config dir · [`stdout_tap`] native-crate log capture.

#[rustfmt::skip]
#[allow(dead_code)]
#[path = "icon.rs"]
mod icon;
mod arb;
mod arb_overlay;
mod backup;
mod country_flags;
#[cfg(feature = "demo")]
mod demo;
mod device_name;
mod device_queries;
mod device_snapshot;
mod file_hash;
mod focus_button;
mod layout_constraints;
mod loader;
#[cfg(test)]
mod manual_rollback_tests;
mod message;
mod model;
mod operation_execution;
mod operation_phase;
mod pickers;
mod platform_installers;
mod root_manager;
mod self_update;
mod settings_store;
mod software_fix;
mod stdout_tap;
mod theme;
mod theme_detect;
mod translations;
mod update;
mod view;
mod widgets;
mod workers;

// Extracted items live in their own modules; re-export so the rest of the
// crate keeps referring to them unqualified.
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
use iced::widget::{Space, column, row, text};
use iced::{Element, Length, Subscription, Task, Theme};

use theme::{Palette, ThemeSeed, palette_for, with_alpha};

/// Palette lookup from `iced` style closures that only have `&Theme`.
fn pal_of(t: &Theme) -> Palette {
    theme::active_palette_for(t)
}

/// Upper bound on `App.log_lines` — keeps memory flat over long sessions.
const LOG_MAX_LINES: usize = 500;
const EXEC_ERROR_SUMMARY_MAX_CHARS: usize = 180;

/// 32×32 RGBA image handle for the custom title-bar brand icon. Built once,
/// cheap to clone (ref-counted). Only used by the custom borderless title bar
/// (Windows / Linux); macOS uses the native system title bar (see
/// [`SYSTEM_WINDOW_CHROME`]).
static TITLE_BAR_ICON_HANDLE: std::sync::LazyLock<iced::widget::image::Handle> =
    std::sync::LazyLock::new(|| {
        let bytes: &'static [u8] = include_bytes!("../assets/icon_32.bin");
        iced::widget::image::Handle::from_rgba(32, 32, bytes.to_vec())
    });

/// Reverse-DNS app id. Becomes Wayland `app_id` / X11 `WM_CLASS` via
/// iced `Settings::id`; matches the shipped `.desktop`'s
/// `StartupWMClass=` so the window binds to the launcher entry.
const APP_ID: &str = "io.github.miner7222.LTBox";

/// Initial window dimensions on first run (logical pixels). Used both
/// by `main`'s `window::Settings::size` fallback and by `App::new` when
/// no persisted size exists yet — they must stay in lockstep.
const DEFAULT_WINDOW_WIDTH: f32 = 820.0;
const DEFAULT_WINDOW_HEIGHT: f32 = 720.0;
/// Floor for cursor-drag resize and for the launch-time geometry
/// (`window::Settings::min_size`). Anything below the width stops laying out
/// cleanly — wizard cards overlap, the sidebar tween jumps.
///
/// The 720px height fits the common Flash-confirm step without scrolling.
/// Taller step bodies and the navigation list scroll within their panels.
const MIN_WINDOW_WIDTH: f32 = 820.0;
const MIN_WINDOW_HEIGHT: f32 = 720.0;
/// macOS uses the native window chrome (system title bar + traffic lights +
/// native resize edges); Windows / Linux keep LTBox's custom borderless title
/// bar and the 8 overlaid resize handles. Gates both the
/// `window::Settings::decorations` flag and the custom-chrome widgets in
/// `view::chrome`, so the two stay in lockstep.
pub(crate) const SYSTEM_WINDOW_CHROME: bool = cfg!(target_os = "macos");
/// Minimum interval between window-size persistence writes. Cursor-drag
/// resize fires `Event::Window(Resized)` continuously; throttling to
/// ~250 ms keeps the JSON file from being rewritten 60 times per second
/// while still capturing the final geometry quickly after the drag ends.
const WINDOW_SIZE_SAVE_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);

/// Upstream repo for the status-bar update affordance.
const UPDATE_REPO: &str = "miner7222/LTBox";

/// Background probe for the status-bar update affordance. Walks
/// `/releases?per_page=100`, returns the latest non-draft /
/// non-prerelease whose semver beats `CARGO_PKG_VERSION`. `None` on
/// network/parse failure or already-current — the status bar stays unchanged.
///
/// Runs synchronously on a `spawn_blocking` worker so the async runtime
/// stays free; the result lands as `Message::UpdateCheckDone`.
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

/// Package-manager command shown by the update dialog.
///
/// Keeping this mapping independent of GUI state makes every install channel
/// explicit and leaves unknown package managers without a guessed command.
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
    // Linux/X11 renderer default. On some X11 + Mesa/driver combos wgpu
    // selects a Vulkan adapter whose X11 surface/device creation fails, so
    // the window never appears and `./ltbox` looks dead (issue #69). OpenGL
    // is robust there and more than enough for this UI, so default the wgpu
    // backend to GL on an X11 session when the user hasn't picked one. Wayland
    // keeps the wgpu default (Vulkan), which the Linux roadmap relies on for
    // recent Nvidia. Override anytime, e.g. `WGPU_BACKEND=vulkan ./ltbox`.
    #[cfg(target_os = "linux")]
    {
        // Treat a var as set only when it is non-empty — winit reads these
        // the same way (an empty value means "unset").
        let non_empty = |key: &str| std::env::var_os(key).is_some_and(|v| !v.is_empty());
        // Only the singular WGPU_BACKEND is read by this iced/wgpu stack, so
        // that alone counts as the user picking a backend. Checking the plural
        // WGPU_BACKENDS would let a value wgpu ignores silently suppress the
        // fallback below.
        let backend_chosen = non_empty("WGPU_BACKEND");
        // winit selects Wayland when WAYLAND_DISPLAY or WAYLAND_SOCKET is set,
        // otherwise X11 via DISPLAY — mirror that to scope the override to
        // pure-X11 sessions only.
        let wayland_session = non_empty("WAYLAND_DISPLAY") || non_empty("WAYLAND_SOCKET");
        let is_x11_session = !wayland_session && non_empty("DISPLAY");
        if !backend_chosen && is_x11_session {
            // SAFETY: first statement in `main`, before the stdout tap, the
            // tracing writer, tokio, or iced spawn any threads — so the
            // process is still single-threaded as `set_var` requires.
            unsafe {
                std::env::set_var("WGPU_BACKEND", "gl");
            }
        }
    }

    // Windows renderer default. Left to itself, wgpu may pick its OpenGL
    // backend, which on hybrid laptops routes through the integrated GPU's
    // OpenGL ICD (e.g. AMD's `atio6axx.dll`) — a fragile path that crashes
    // with an access violation (c0000005) on some driver/GPU combos. DX12 is
    // the native, robust path on Windows 10+ (including AMD/Intel iGPUs) and
    // covers this UI fully, so default the wgpu backend to DX12 when the user
    // hasn't picked one. The software renderer stays reachable via
    // `ICED_BACKEND=tiny-skia` for hosts with broken GPU drivers; override the
    // backend anytime, e.g. `WGPU_BACKEND=vulkan ltbox.exe`.
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

    // Pre-iced CLI subcommands. Each handler exits the process so
    // the iced setup path runs only when no subcommand fires. Kept
    // tiny + dep-free (no `clap`) — there's exactly one flag and it
    // doesn't need argument parsing beyond presence detection.
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

    // Single-instance lock via fs2 advisory lock in the system temp
    // dir. Kernel drops the lock on dirty shutdown. Version-agnostic
    // filename so a running v3.0.0 blocks a v3.0.1 during in-place update.
    let _instance_guard: Option<std::fs::File> = {
        use fs2::FileExt;
        let lock_path = std::env::temp_dir().join("ltbox-gui-singleton.lock");
        match std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)
        {
            Ok(f) => {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
                loop {
                    match f.try_lock_exclusive() {
                        Ok(()) => break Some(f),
                        // Only an updater-spawned process waits for the old
                        // instance to release the lock. An ordinary second
                        // launch keeps the existing quiet, immediate exit.
                        Err(_) if post_update_relaunch && std::time::Instant::now() < deadline => {
                            std::thread::sleep(std::time::Duration::from_millis(100));
                        }
                        Err(_) => return Ok(()),
                    }
                }
            }
            // Can't create the guard (sandboxed FS). A post-update child still
            // pauses long enough for the spawning process to leave its runtime;
            // ordinary launches preserve the prior unguarded behavior.
            Err(_) => {
                if post_update_relaunch {
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
                None
            }
        }
    };
    if _instance_guard.is_some() {
        self_update::cleanup_stale_update_backups();
    }

    // libusb (via `adb_client` → `rusb`) probes for optional backends by
    // calling LoadLibrary on `%SystemRoot%\System32\libusbK.dll`, and it
    // already treats a NULL handle as "backend unavailable". But when that
    // system DLL is present and corrupt, the Windows hard-error handler
    // raises a modal "Bad Image" box (0xc000012f) on every probe, which the
    // user cannot dismiss for good and which LTBox never gets to explain.
    // SEM_FAILCRITICALERRORS turns that into the plain NULL libusb already
    // handles, leaving the WinUSB backend — and every `nusb` EDL path, which
    // never touches libusbK — working. Must run before the first USB probe.
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
    stdout_tap::install();

    // `_log_guard` MUST live for the whole process — dropping it
    // flushes the non-blocking writer; losing it loses the last
    // minute of events on a crash.
    let _log_guard = init_tracing();

    // Package-manager upgrades replace/prune executable directories. Move any
    // v3 executable-adjacent data before the first worker can create the new
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
    // Restore the user's previous window geometry if persisted (clamped
    // to ≥ `MIN_WINDOW_*` so corrupted / pre-min-size config files can
    // never launch a sub-floor window). Falls back to the default size
    // on first run.
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
        // Cursor-drag resize: `MIN_WINDOW_*` is the floor; anything
        // below is unsupported (sidebar + wizard cards stop laying out
        // cleanly). On Windows / Linux the borderless decorations strip
        // native resize edges off the window, so the GUI overlays 8 invisible
        // resize handles on the root Stack which emit
        // `WindowMsg::WindowResize(direction)` and call
        // `iced::window::drag_resize` on the host window. (macOS keeps native
        // decorations + resize, so those handles are not rendered there.) The
        // user's resized geometry is persisted to `PersistedSettings::window_size`
        // and restored above on the next launch on every platform.
        min_size: Some(iced::Size::new(MIN_WINDOW_WIDTH, MIN_WINDOW_HEIGHT)),
        icon: win_icon,
        // macOS → native decorations (system title bar + resize); other
        // platforms → borderless + custom chrome (see SYSTEM_WINDOW_CHROME).
        decorations: SYSTEM_WINDOW_CHROME,
        ..Default::default()
    };
    // Bundle Noto Sans CJK at compile time so cosmic-text can fall
    // back for Hangul / Hanzi glyphs. Noto's Latin + Cyrillic cover
    // English and Russian UI through the same family.
    let mut app = iced::application(App::new, App::update, App::view)
        .title("LTBox")
        // Application id propagates to winit:
        //   * Wayland → `app_id` on the xdg-shell toplevel
        //   * X11     → `WM_CLASS` (instance + class)
        // Matches `StartupWMClass=` in the shipped `.desktop` file
        // so GNOME / KDE / etc bind the running window to the
        // launcher entry. Without this they fall back to the binary
        // name (`ltbox`) which would only match if the desktop file
        // also said `StartupWMClass=ltbox` — using a reverse-DNS id
        // keeps it future-proof against a renamed binary.
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
        // a family the bundle does not carry, so it fell through to whatever
        // the system offered — Courier New on Windows.
        include_bytes!("../fonts/noto/NotoSansMono-Regular.subset.ttf") as &[u8],
    ] {
        app = app.font(bytes);
    }
    // Subset Lucide TTF generated at build time from
    // `fonts/lucide.toml`. Registered under the family `"lucide"` so
    // the text-based icon widgets from `mod icon` resolve against it.
    app = app.font(icon::FONT);
    app.run()
}

/// Global tracing subscriber writing daily-rotated files under
/// `%APPDATA%\ltbox\logs\`. Caller must hold the returned `WorkerGuard`
/// for the process lifetime — dropping it flushes queued entries.
/// Filter: `RUST_LOG` env var, falling back to `info`.
fn init_tracing() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    use camino::Utf8PathBuf;
    use tracing_subscriber::{EnvFilter, fmt};

    // Fall back to `%TEMP%\ltbox-logs` on non-UTF-8 APPDATA paths.
    let log_dir: Utf8PathBuf = dirs::config_dir()
        .and_then(|d| Utf8PathBuf::from_path_buf(d.join("ltbox").join("logs")).ok())
        .unwrap_or_else(|| {
            Utf8PathBuf::from_path_buf(std::env::temp_dir().join("ltbox-logs"))
                .unwrap_or_else(|_| Utf8PathBuf::from("ltbox-logs"))
        });
    if std::fs::create_dir_all(&log_dir).is_err() {
        return None;
    }

    let file_appender = tracing_appender::rolling::daily(log_dir.as_std_path(), "ltbox.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    // `adb_client` logs a line per connect, and the dashboard reconnects
    // every poll, so it is held at `warn` unless RUST_LOG asks for more.
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,adb_client=warn"));

    // `init` rather than `set_global_default`: it also installs the
    // `log` -> `tracing` bridge, so records from dependencies that use
    // the `log` crate reach the file. `adb_client` reports the device's
    // CNXN banner — the string that carries the real connection state,
    // `device::` / `recovery::` / `sideload::` — only through `log`.
    fmt()
        .with_env_filter(filter)
        .with_writer(non_blocking)
        .with_ansi(false)
        .with_target(true)
        .init();

    Some(guard)
}

// =========================================================================
// Navigation
// =========================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum View {
    #[default]
    Dashboard,
    Flash,
    SystemUpdate,
    Root,
    Unroot,
    KonaBess,
    Reboot,
    Advanced,
    Settings,
    About,
}

impl View {
    fn label_key(&self) -> &'static str {
        match self {
            Self::Dashboard => "nav_dashboard",
            Self::Flash => "nav_flash",
            Self::SystemUpdate => "nav_sysupdate",
            Self::Root => "nav_root",
            Self::Unroot => "nav_unroot",
            Self::KonaBess => "nav_konabess",
            Self::Reboot => "nav_reboot",
            Self::Advanced => "nav_advanced",
            Self::Settings => "nav_settings",
            Self::About => "nav_about",
        }
    }

    fn sidebar_label_key(&self) -> &'static str {
        match self {
            Self::Flash => "nav_flash_sidebar",
            Self::KonaBess => "nav_konabess_sidebar",
            _ => self.label_key(),
        }
    }

    fn nav_icon(&self) -> iced::widget::Text<'static, Theme, iced::Renderer> {
        match self {
            Self::Dashboard => icon::nav_dashboard(),
            Self::Flash => icon::nav_flash(),
            Self::SystemUpdate => icon::nav_system_update(),
            Self::Root => icon::nav_root(),
            Self::Unroot => icon::nav_unroot(),
            Self::KonaBess => icon::nav_konabess(),
            Self::Reboot => icon::nav_reboot(),
            Self::Advanced => icon::nav_advanced(),
            Self::Settings => icon::nav_settings(),
            Self::About => icon::nav_about(),
        }
    }
}

const NAV_MAIN: &[View] = &[
    View::Dashboard,
    View::Flash,
    View::SystemUpdate,
    View::Root,
    View::Unroot,
    View::KonaBess,
    View::Reboot,
];
const NAV_TOOLS: &[View] = &[View::Advanced, View::Settings];

/// One-shot reboot target for the Reboot panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RebootTarget {
    System,
    Recovery,
    Bootloader,
    /// Userspace fastboot. Reached with `reboot fastboot` over ADB, and
    /// with `reboot-fastboot` from the bootloader.
    Fastbootd,
    Edl,
}
impl RebootTarget {
    fn label_key(&self) -> &'static str {
        match self {
            Self::System => "reboot_system",
            Self::Recovery => "reboot_recovery",
            Self::Bootloader => "reboot_bootloader",
            Self::Fastbootd => "reboot_fastbootd",
            Self::Edl => "reboot_edl",
        }
    }
    /// Short-name key used inside the confirm popup so "Reboot to
    /// {Reboot to System}?" doesn't double-phrase.
    fn short_name_key(&self) -> &'static str {
        match self {
            Self::System => "reboot_target_system",
            Self::Recovery => "reboot_target_recovery",
            Self::Bootloader => "reboot_target_bootloader",
            Self::Fastbootd => "reboot_target_fastbootd",
            Self::Edl => "reboot_target_edl",
        }
    }
    /// Reachable from `conn`. Impossible combos (Fastboot → Recovery,
    /// EDL → Recovery/Bootloader — Firehose only resets system/edl)
    /// stay disabled.
    fn available_from(&self, conn: ConnectionStatus) -> bool {
        match (conn, self) {
            (ConnectionStatus::None, _) => false,
            (ConnectionStatus::AdbUnauthorized, _) => false,
            // minadbd answers `reboot:` even though it refuses `shell:`,
            // so system/recovery/bootloader work. EDL does not: LTBox
            // reaches it by running `reboot edl` in a shell there is none
            // of, and the resulting error can pass for adbd dropping the
            // connection after a reboot that never fired.
            (ConnectionStatus::AdbSideload, Self::Edl) => false,
            (ConnectionStatus::AdbSideload, _) => true,
            (ConnectionStatus::AdbServerBlocking, _) => false,
            (ConnectionStatus::Adb, _) => true,
            (ConnectionStatus::AdbRecovery, _) => true,
            (ConnectionStatus::Fastboot, Self::Recovery) => false,
            (ConnectionStatus::Fastboot, _) => true,
            (ConnectionStatus::Edl, Self::System | Self::Edl) => true,
            (ConnectionStatus::Edl, _) => false,
        }
    }
    /// Whether this target is the mode represented by the current transport.
    /// A no-op reboot is shown as "current state" rather than as an action.
    fn is_current_from(&self, conn: ConnectionStatus, fastboot_userspace: bool) -> bool {
        matches!(
            (conn, self, fastboot_userspace),
            (ConnectionStatus::Adb, Self::System, _)
                | (ConnectionStatus::AdbRecovery, Self::Recovery, _)
                | (ConnectionStatus::AdbSideload, Self::Recovery, _)
                | (ConnectionStatus::Fastboot, Self::Bootloader, false)
                | (ConnectionStatus::Fastboot, Self::Fastbootd, true)
                | (ConnectionStatus::Edl, Self::Edl, _)
        )
    }
    fn all() -> &'static [RebootTarget] {
        &[
            Self::System,
            Self::Recovery,
            Self::Bootloader,
            Self::Fastbootd,
            Self::Edl,
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AdvAction {
    RegionConvert,
    ImageInfo,
    PatchDevinfo,
    DetectArb,
    PatchArb,
    ConvertXml,
    DumpPartitions,
    DumpPhysical,
    FlashPartitions,
    FlashPhysical,
    RebuildVbmeta,
    SimpleFlash,
}
impl AdvAction {
    /// Whether this action writes to the device rather than reading from
    /// it or transforming a local file. The Advanced grid renders these
    /// on the `error` role: a tile that flashes a partition should not
    /// be visually interchangeable with one that dumps it.
    fn is_destructive(&self) -> bool {
        matches!(
            self,
            Self::FlashPartitions | Self::FlashPhysical | Self::SimpleFlash
        )
    }

    fn label_key(&self) -> &'static str {
        match self {
            Self::RegionConvert => "adv_region_convert",
            Self::ImageInfo => "adv_image_info",
            Self::PatchDevinfo => "adv_patch_devinfo",
            Self::DetectArb => "adv_detect_arb",
            Self::PatchArb => "adv_patch_arb",
            Self::ConvertXml => "adv_convert_xml",
            Self::DumpPartitions => "adv_dump_partitions",
            Self::DumpPhysical => "adv_dump_physical",
            Self::FlashPartitions => "adv_flash_partitions",
            Self::FlashPhysical => "adv_flash_physical",
            Self::RebuildVbmeta => "adv_rebuild_vbmeta",
            Self::SimpleFlash => "adv_simple_flash",
        }
    }
    fn desc_key(&self) -> &'static str {
        match self {
            Self::RegionConvert => "adv_region_convert_desc",
            Self::ImageInfo => "adv_image_info_desc",
            Self::PatchDevinfo => "adv_patch_devinfo_desc",
            Self::DetectArb => "adv_detect_arb_desc",
            Self::PatchArb => "adv_patch_arb_desc",
            Self::ConvertXml => "adv_convert_xml_desc",
            Self::DumpPartitions => "adv_dump_partitions_desc",
            Self::DumpPhysical => "adv_dump_physical_desc",
            Self::FlashPartitions => "adv_flash_partitions_desc",
            Self::FlashPhysical => "adv_flash_physical_desc",
            Self::RebuildVbmeta => "adv_rebuild_vbmeta_desc",
            Self::SimpleFlash => "adv_simple_flash_desc",
        }
    }
    /// Browse-tile sub-description: *what* to pick, not the action's
    /// high-level description.
    fn source_desc_key(&self) -> &'static str {
        match self {
            Self::RegionConvert => "adv_src_region_convert",
            Self::ImageInfo => "adv_src_image_info",
            Self::PatchDevinfo => "adv_src_patch_devinfo",
            Self::DetectArb => "adv_src_detect_arb",
            Self::PatchArb => "adv_src_patch_arb_folder",
            Self::ConvertXml => "adv_src_convert_xml",
            Self::DumpPartitions => "adv_src_dump_partitions",
            Self::DumpPhysical => "adv_src_dump_physical",
            Self::FlashPartitions => "adv_src_flash_partitions",
            Self::FlashPhysical => "adv_src_flash_physical",
            Self::RebuildVbmeta => "adv_src_rebuild_vbmeta",
            // SimpleFlash uses a dedicated wizard (folder picker on Next),
            // not the generic source tile — reuse the flash-folder caption.
            Self::SimpleFlash => "flash_folder_desc",
        }
    }
    /// snake_case slug for `{exe_dir}/output_{slug}/` — Advanced ops
    /// drop artefacts here instead of asking the user for a location.
    fn output_slug(&self) -> &'static str {
        match self {
            Self::RegionConvert => "region_convert",
            Self::ImageInfo => "image_info",
            Self::PatchDevinfo => "patch_devinfo",
            Self::DetectArb => "detect_arb",
            Self::PatchArb => "rb",
            Self::ConvertXml => "convert_xml",
            Self::DumpPartitions => "dump_partitions",
            Self::DumpPhysical => "dump_physical",
            Self::FlashPartitions => "flash_partitions",
            Self::FlashPhysical => "flash_physical",
            Self::RebuildVbmeta => "rebuild_vbmeta",
            Self::SimpleFlash => "simple_flash",
        }
    }
    /// True iff the action writes into the output folder — gates the
    /// "Open Folder" pill on the Done card.
    fn produces_output(&self) -> bool {
        matches!(
            self,
            Self::RegionConvert
                | Self::PatchDevinfo
                | Self::PatchArb
                | Self::ConvertXml
                | Self::RebuildVbmeta
        )
    }
}

/// Auto-output directory for an Advanced wizard action. Caller
/// `create_dir_all`s before writing. Routes through
/// [`ltbox_core::app_paths::auto_output_dir_for`] so AppImage /
/// distro-installed Linux copies don't try to write next to a
/// read-only or root-owned executable. Windows path stays
/// exe-adjacent (`<exe-dir>/output_<slug>`) for v3 continuity.
fn adv_output_dir(action: AdvAction) -> std::path::PathBuf {
    ltbox_core::app_paths::auto_output_dir_for(action.output_slug())
}

/// Launch the platform file manager on `path`.
///
/// Returns `Ok(())` only when a launcher actually accepted the spawn
/// — previously every error path was a `let _ = …` swallow, which on
/// Linux meant a missing `xdg-open` (or a desktop session without a
/// MIME handler for `inode/directory`) silently no-op'd. Caller is
/// expected to surface the returned error string in the GUI log /
/// error popup so users know why the "Open Folder" button did
/// nothing.
fn open_in_file_manager(path: &std::path::Path) -> std::result::Result<(), String> {
    #[cfg(windows)]
    {
        // `CREATE_NO_WINDOW` hides the transient cmd flash.
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std::process::Command::new("explorer")
            .arg(path)
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("explorer {}: {e}", path.display()))
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("open {}: {e}", path.display()))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // Try xdg-open first (every desktop ships one); fall back to
        // GNOME's `gio open` which behaves correctly on
        // xdg-portal-only sessions where `xdg-open` itself errors out
        // mapping `inode/directory`. Capture the xdg error before
        // touching `gio` so the match below is exhaustive (compiler
        // can't see that the early return makes `xdg` provably Err
        // by this point).
        let xdg = std::process::Command::new("xdg-open").arg(path).spawn();
        if xdg.is_ok() {
            return Ok(());
        }
        let xdg_err = xdg.expect_err("checked Ok above");
        let gio = std::process::Command::new("gio")
            .arg("open")
            .arg(path)
            .spawn();
        match gio {
            Ok(_) => Ok(()),
            Err(gio_err) => Err(format!(
                "xdg-open {}: {xdg_err}; gio open {}: {gio_err}",
                path.display(),
                path.display(),
            )),
        }
    }
}
struct AdvSection {
    title_key: &'static str,
    items: &'static [AdvAction],
}

const ADV_SECTIONS: &[AdvSection] = &[
    AdvSection {
        title_key: "adv_section_region_patch",
        items: &[AdvAction::RegionConvert, AdvAction::PatchDevinfo],
    },
    AdvSection {
        title_key: "adv_section_rollback",
        items: &[
            AdvAction::ImageInfo,
            AdvAction::DetectArb,
            AdvAction::PatchArb,
            AdvAction::RebuildVbmeta,
        ],
    },
    AdvSection {
        title_key: "adv_section_edl_ops",
        items: &[
            AdvAction::ConvertXml,
            // Per-partition Read / Write paired together (read above
            // write so users can dump first, then re-flash if needed).
            AdvAction::DumpPartitions,
            AdvAction::FlashPartitions,
            // Whole-LUN dump / flash paired the same way.
            AdvAction::DumpPhysical,
            AdvAction::FlashPhysical,
            // Stock-equivalent flash: no checks, no edits — just flashing.
            AdvAction::SimpleFlash,
        ],
    },
];

// =========================================================================
// Root wizard types
// =========================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    Magisk,
    KernelSU,
    APatch,
    Skroot,
}
impl Family {
    fn label_key(&self) -> &'static str {
        match self {
            Self::Magisk => "family_magisk",
            Self::KernelSU => "family_ksu",
            Self::APatch => "family_apatch",
            Self::Skroot => "family_skroot",
        }
    }
    fn desc_key(&self) -> &'static str {
        match self {
            Self::Magisk => "family_magisk_desc",
            Self::KernelSU => "family_ksu_desc",
            Self::APatch => "family_apatch_desc",
            Self::Skroot => "family_skroot_desc",
        }
    }
    fn has_modes(&self) -> bool {
        matches!(self, Self::KernelSU | Self::Skroot)
    }
    fn providers(&self) -> &'static [Provider] {
        match self {
            Self::Magisk => &[Provider::Magisk, Provider::MagiskForks],
            Self::KernelSU => &[
                Provider::KernelSU,
                Provider::KernelSUNext,
                Provider::SukiSU,
                Provider::ReSukiSU,
            ],
            Self::APatch => &[Provider::APatch, Provider::FolkPatch],
            Self::Skroot => &[],
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Provider {
    Magisk,
    MagiskForks,
    KernelSU,
    KernelSUNext,
    SukiSU,
    ReSukiSU,
    APatch,
    FolkPatch,
}
impl Provider {
    fn label_key(&self) -> &'static str {
        match self {
            Self::Magisk => "provider_magisk",
            Self::MagiskForks => "provider_magisk_forks",
            Self::KernelSU => "provider_ksu",
            Self::KernelSUNext => "provider_ksu_next",
            Self::SukiSU => "provider_sukisu",
            Self::ReSukiSU => "provider_resukisu",
            Self::APatch => "provider_apatch",
            Self::FolkPatch => "provider_folkpatch",
        }
    }
    fn desc_key(&self) -> Option<&'static str> {
        match self {
            Self::Magisk => Some("provider_magisk_desc"),
            Self::MagiskForks => Some("provider_magisk_forks_desc"),
            Self::KernelSU => Some("provider_ksu_desc"),
            Self::KernelSUNext => Some("provider_ksu_next_desc"),
            Self::SukiSU => Some("provider_sukisu_desc"),
            Self::ReSukiSU => Some("provider_resukisu_desc"),
            Self::APatch => Some("provider_apatch_desc"),
            Self::FolkPatch => Some("provider_folkpatch_desc"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RootMode {
    Lkm,
    Gki,
}
impl RootMode {
    fn label_key(&self) -> &'static str {
        match self {
            Self::Lkm => "rootmode_lkm",
            Self::Gki => "rootmode_gki",
        }
    }
    fn desc_key(&self) -> &'static str {
        match self {
            Self::Lkm => "rootmode_lkm_desc",
            Self::Gki => "rootmode_gki_desc",
        }
    }
    fn icon_disabled(self, size: f32) -> Element<'static, Message> {
        let glyph = match self {
            Self::Lkm => icon::root_lkm(),
            Self::Gki => icon::root_gki(),
        };
        lucide_disabled(glyph, size)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SkrootFlavor {
    Lite,
    Pro,
}
impl SkrootFlavor {
    fn label_key(&self) -> &'static str {
        match self {
            Self::Lite => "skroot_flavor_lite",
            Self::Pro => "skroot_flavor_pro",
        }
    }
    fn desc_key(&self) -> &'static str {
        match self {
            Self::Lite => "skroot_flavor_lite_desc",
            Self::Pro => "skroot_flavor_pro_desc",
        }
    }
    fn icon(self, size: f32) -> Element<'static, Message> {
        let glyph = match self {
            Self::Lite => icon::skroot_lite(),
            Self::Pro => icon::root_lkm(),
        };
        lucide_primary(glyph, size)
    }
    fn icon_disabled(self, size: f32) -> Element<'static, Message> {
        let glyph = match self {
            Self::Lite => icon::skroot_lite(),
            Self::Pro => icon::root_lkm(),
        };
        lucide_disabled(glyph, size)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VerChoice {
    Stable,
    Nightly,
}
impl VerChoice {
    fn label_key(&self) -> &'static str {
        match self {
            Self::Stable => "verchoice_stable",
            Self::Nightly => "verchoice_nightly",
        }
    }
    fn desc_key(&self) -> &'static str {
        match self {
            Self::Stable => "verchoice_stable_desc",
            Self::Nightly => "verchoice_nightly_desc",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NightlySource {
    AutoDetect,
    ManualInput,
}
impl NightlySource {
    fn label_key(&self) -> &'static str {
        match self {
            Self::AutoDetect => "nightly_auto",
            Self::ManualInput => "nightly_manual",
        }
    }
    fn desc_key(&self) -> &'static str {
        match self {
            Self::AutoDetect => "nightly_auto_desc",
            Self::ManualInput => "nightly_manual_desc",
        }
    }
}

// =========================================================================
// Settings state
// =========================================================================

/// Theme preference. `System` reads the OS setting via
/// `theme_detect::system_prefers_dark`; Light/Dark override.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
}
impl ThemeChoice {
    fn label_key(&self) -> &'static str {
        match self {
            Self::System => "theme_system",
            Self::Light => "theme_light",
            Self::Dark => "theme_dark",
        }
    }
    fn code(&self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
    fn from_code(c: &str) -> Option<Self> {
        match c {
            "system" => Some(Self::System),
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            _ => None,
        }
    }
}

/// Match a SKU token inside an arbitrary string using the shared model-identity
/// rules. Alphanumeric word boundaries prevent a future suffixed model from
/// colliding with the bare match.
pub(crate) fn fingerprint_token_match(haystack: &str, model: &str) -> bool {
    ltbox_core::model::fingerprint_model_match(haystack, model)
}

fn concise_error_summary(error: &str, max_chars: usize) -> String {
    let summary = error
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if summary.chars().count() <= max_chars {
        return summary;
    }
    if max_chars == 0 {
        return String::new();
    }

    let mut truncated: String = summary.chars().take(max_chars - 1).collect();
    truncated.push('…');
    truncated
}

fn busy_navigation_target(busy: bool, busy_view: Option<View>) -> Option<View> {
    if busy { busy_view } else { None }
}

// Icon glyphs for the current-step card (running / done / failed).
// Colour is applied at the call site so running/done/failed each paint
// with the palette role appropriate to the outcome (primary / success
// / error).

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum RollbackSetting {
    On,
    Auto,
    Manual,
    #[default]
    Off,
}
impl RollbackSetting {
    fn label_key(&self) -> &'static str {
        match self {
            Self::On => "rollback_on",
            Self::Auto => "rollback_auto",
            Self::Manual => "rollback_manual",
            Self::Off => "rollback_off",
        }
    }
    /// Map the wizard setting to the worker's rollback mode.
    fn to_mode(self) -> ltbox_patch::rollback::RollbackMode {
        match self {
            Self::On => ltbox_patch::rollback::RollbackMode::On,
            Self::Auto => ltbox_patch::rollback::RollbackMode::Auto,
            Self::Manual => ltbox_patch::rollback::RollbackMode::Manual,
            Self::Off => ltbox_patch::rollback::RollbackMode::Off,
        }
    }
}

/// Explicit per-partition rollback targets for `RollbackMode::Manual`.
pub(crate) type ManualRollbackIndices = ltbox_patch::rollback::RollbackIndices;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ManualRollbackEditor {
    Boot,
    VbmetaSystem,
}

#[derive(Debug, Clone)]
struct SettingsState {
    language: Language,
}

impl Default for SettingsState {
    fn default() -> Self {
        Self {
            language: Language::En,
        }
    }
}

/// Derived from wizard selections; reset after the op finishes.
#[derive(Debug, Clone, Default)]
pub(crate) struct WorkflowConfig {
    pub(crate) modify_region: bool,
    pub(crate) device_region: Option<DeviceRegion>,
    pub(crate) modify_rollback: RollbackSetting,
    /// Explicit rollback targets used only by `RollbackMode::Manual`.
    pub(crate) manual_rollback_indices: Option<ManualRollbackIndices>,
    pub(crate) wipe: bool,
    pub(crate) country_action: CountryAction,
}

/// Sortable header cell for the FlashParts / DumpParts partition table.
/// Renders `label` followed by either ▲/▼ (active sort, direction
/// reflects `desc`) or ⇅ (sortable but inactive). Click fires `msg`.
/// Transparent button so the cell reads as text first.
/// Sort marker for a table column head.
///
/// The active column shows the direction it is sorted in; the others show that
/// they *can* be sorted. Both are arrows so the pair reads as one control in
/// two states rather than two different marks, and the idle arrow drops to the
/// disabled tone so the sorted column is the one that stands out.
fn parts_sort_marker(is_active: bool, desc: bool) -> Element<'static, Message> {
    let glyph = if is_active {
        if desc { "\u{2193}" } else { "\u{2191}" }
    } else {
        "\u{21c5}"
    };
    text(glyph)
        .size(11)
        .style(move |t: &Theme| iced::widget::text::Style {
            color: Some(if is_active {
                pal_of(t).on_surface
            } else {
                with_alpha(pal_of(t).on_surface, 0.38)
            }),
        })
        .into()
}

fn parts_sort_header(
    label: String,
    is_active: bool,
    desc: bool,
    width: Length,
    msg: Message,
) -> Element<'static, Message> {
    button(
        row![
            text(label).size(11).style(muted_style),
            parts_sort_marker(is_active, desc),
        ]
        .spacing(4)
        .align_y(iced::Alignment::Center),
    )
    .padding(0)
    .width(width)
    .style(|_t: &Theme, _s| button::Style {
        background: None,
        ..Default::default()
    })
    .on_press(msg)
    .into()
}

/// Human-readable auto-unit byte formatter (B/KB/MB/GB).
fn format_bytes_auto(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.2} GB", b / GB)
    } else if b >= MB {
        format!("{:.2} MB", b / MB)
    } else if b >= KB {
        format!("{:.2} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

// =========================================================================
// Async poll results + popup UI state
// =========================================================================

#[derive(Debug, Clone, Default)]
struct DevicePollResult {
    status: ConnectionStatus,
    /// `status == Fastboot` and the endpoint answered `is-userspace: yes`,
    /// i.e. fastbootd rather than the bootloader. Display only — every
    /// behavioural branch treats the two the same.
    fastboot_userspace: bool,
    model: String,
    /// Trimmed `ro.build.version.release`. Only populated by ADB polls;
    /// fastboot and EDL do not expose Android system properties.
    android_version: String,
    slot: String,
    /// Trimmed `ro.build.display.id` — leading device-model prefix
    /// stripped so the dashboard cell stays readable.
    firmware: String,
    /// Untrimmed `ro.build.display.id` exactly as the device reports
    /// it. Required by Lenovo's OTA `querynewfirmware` endpoint —
    /// passing the trimmed form returns an empty `<firmwareupdate/>`
    /// because the upstream key matches the full string.
    firmware_full: String,
    arb: String,
    /// `boot` / `vbmeta_system` rollback floors classified from the
    /// fastboot `stored_rollback_index:N` vars. Only ever `Some` on a
    /// bootloader-mode poll — no other transport reports them.
    rollback_floors: Option<ltbox_patch::rollback::FastbootRollbackFloors>,
    ram: String,
    storage: String,
    market_name: String,
    /// Device serial captured from ADB or fastboot. Empty when no
    /// connected device produced a serial (EDL/Sahara never reports
    /// one). Used by the device-info popup to query the Lenovo PTSTPD
    /// API. Reset to empty whenever the device disconnects so a stale
    /// serial does not bleed across hardware swaps mid-session.
    serial: String,
    platform_supported: Option<bool>, // None = unknown, Some(true) = qcom, Some(false) = unsupported
}

/// Loading state for the device-info popup. The popup view branches on
/// this to render a progress indicator / table / error banner while keeping the
/// modal open so the user has a clear target to dismiss.
#[derive(Debug, Clone)]
enum DeviceInfoState {
    /// Fetch is in flight; render a progress indicator and disable retry.
    Loading,
    /// `device_info_cache[serial]` is populated; render the table.
    Ready,
    /// Fetch failed; render the message + a retry pill.
    Error(String),
}

/// Loading state for the firmware-OTA popup. Mirrors `DeviceInfoState`
/// but adds a `NoUpdate` arm — the upstream `<firmwareupdate/>` empty
/// payload means "no OTA staged for this firmware id" and renders as a
/// single placeholder line, not as an error banner.
#[derive(Debug, Clone)]
enum OtaPopupState {
    Loading,
    NoUpdate,
    Ready(ltbox_core::lenovo_ota::OtaUpdate),
    Error(String),
}

/// Worker result for the QFIL-firmware lookup: a global (non-CN) device, a CN
/// device whose MTM has no published package, or the resolved package.
#[derive(Debug, Clone)]
pub(crate) enum QfilOutcome {
    /// `SaleArea != CN` — point the user at Lenovo Software Fix instead.
    Global,
    /// CN device, but the MTM resolved to no flashing-machine package.
    NoPackage,
    /// Resolved official QFIL package.
    Package(ltbox_core::lenovo_qfil::QfilPackage),
}

/// Loading state for the QFIL-firmware popup. Mirrors [`OtaPopupState`] with a
/// `Global` arm (non-CN device) and a `NoPackage` arm (CN, MTM unmatched).
#[derive(Debug, Clone)]
enum QfilPopupState {
    Loading,
    Global,
    NoPackage,
    Ready(ltbox_core::lenovo_qfil::QfilPackage),
    Error(String),
}

/// Parse hwboardid: `"SM8750P_16+512_13"` → `("16 GB", "512 GB")`.
fn parse_hwboardid_ram_storage(hwboardid: &str) -> (String, String) {
    let parts: Vec<&str> = hwboardid.split('_').collect();
    for part in &parts {
        if let Some((ram, storage)) = part.split_once('+')
            && ram.chars().all(|c| c.is_ascii_digit())
            && storage.chars().all(|c| c.is_ascii_digit())
        {
            return (format!("{ram} GB"), format!("{storage} GB"));
        }
    }
    (String::new(), String::new())
}

/// Pre-translated live-log strings for spawn_blocking closures that
/// can't carry `self` across thread boundaries.
#[derive(Debug, Clone)]
pub(crate) struct LiveLabels {
    pub(crate) closing_dump: String,
    pub(crate) flash_completed: String,
    pub(crate) adb_no_kver: String,
    pub(crate) backup_saved_prefix: String,
    pub(crate) root_resolved_prefix: String,
    pub(crate) root_backup_copy_prefix: String,
}

/// Classify a model → rollback-protection i18n key. Every supported model
/// enforces AVB rollback protection except the PRC-only TB322FC, and an
/// unknown model is assumed protected, so this is a TB322FC check.
/// How the rollback-index popup renders a stored floor. Clicking a value
/// steps to the next form and wraps back around.
///
/// A rollback index is a unix timestamp, but fastboot reports it base-16
/// (`stored_rollback_index:N = 41B7A200`), so the raw form a user sees in
/// `fastboot getvar all` is hex. The cycle walks outward from that raw
/// value to progressively more readable renderings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum RollbackValueFormat {
    /// As `fastboot getvar all` prints it — base-16.
    #[default]
    Raw,
    /// The same number in decimal, i.e. a plain unix timestamp.
    Unix,
    /// `YYYY-MM-DD`, UTC.
    Date,
}

impl RollbackValueFormat {
    pub(crate) fn next(self) -> Self {
        match self {
            Self::Raw => Self::Unix,
            Self::Unix => Self::Date,
            Self::Date => Self::Raw,
        }
    }

    /// Render `index` in this form. The returned string is exactly what
    /// the copy button puts on the clipboard.
    pub(crate) fn render(self, index: u64) -> String {
        match self {
            Self::Raw => format!("0x{index:X}"),
            Self::Unix => index.to_string(),
            Self::Date => format_unix_date_utc(index),
        }
    }

    /// i18n key naming the current form, shown beside the value so the
    /// cycle is self-explanatory rather than a guessing game.
    pub(crate) const fn label_key(self) -> &'static str {
        match self {
            Self::Raw => "rollback_format_raw",
            Self::Unix => "rollback_format_unix",
            Self::Date => "rollback_format_date",
        }
    }

    /// Parse user input using the same convention used for rendering:
    /// raw is `0x…` hexadecimal, Unix is decimal, and Date is a UTC
    /// calendar day represented at midnight.
    pub(crate) fn parse(self, input: &str) -> Result<u64, String> {
        let trimmed = input.trim();
        match self {
            Self::Raw => {
                let digits = trimmed
                    .strip_prefix("0x")
                    .or_else(|| trimmed.strip_prefix("0X"))
                    .ok_or_else(|| "rollback_manual_error_prefix".to_string())?;
                u64::from_str_radix(digits, 16).map_err(|_| "rollback_manual_error_hex".to_string())
            }
            Self::Unix => trimmed
                .parse::<u64>()
                .map_err(|_| "rollback_manual_error_decimal".to_string()),
            Self::Date => {
                let bytes = trimmed.as_bytes();
                if trimmed.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
                    return Err("rollback_manual_error_date_shape".to_string());
                }
                let (year, rest) = trimmed.split_at(4);
                if rest.len() != 6 || !rest.starts_with('-') {
                    return Err("rollback_manual_error_date_shape".to_string());
                }
                let month = &rest[1..3];
                let day = &rest[4..6];
                let year: i32 = year
                    .parse()
                    .map_err(|_| "rollback_manual_error_date_value".to_string())?;
                let month: u32 = month
                    .parse()
                    .ok()
                    .filter(|month| (1..=12).contains(month))
                    .ok_or_else(|| "rollback_manual_error_date_value".to_string())?;
                let day: u32 = day
                    .parse()
                    .ok()
                    .filter(|day| (1..=31).contains(day))
                    .ok_or_else(|| "rollback_manual_error_date_value".to_string())?;

                let days_since_epoch = civil_from_days_ordinal(year, month, day)
                    .ok_or_else(|| "rollback_manual_error_date_value".to_string())?;
                let timestamp = days_since_epoch * 86_400;
                u64::try_from(timestamp).map_err(|_| "rollback_manual_error_date_value".to_string())
            }
        }
    }
}

/// Convert a proleptic Gregorian UTC date to days since the Unix epoch
/// using the inverse of `civil_from_days`. Returns `None` for impossible
/// dates such as February 30.
fn civil_from_days_ordinal(year: i32, month: u32, day: u32) -> Option<i64> {
    const DAYS_IN_MONTH: [u32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let leap_year = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let mut days_in_month = DAYS_IN_MONTH;
    if leap_year {
        days_in_month[1] = 29;
    }
    let month_index = usize::try_from(month.checked_sub(1)?).ok()?;
    if day > days_in_month[month_index] {
        return None;
    }

    let adjusted_year = if month <= 2 { year - 1 } else { year };
    let era = if adjusted_year >= 0 {
        adjusted_year
    } else {
        adjusted_year - 399
    } / 400;
    let era = i64::from(era);
    let year_of_era = i64::from(adjusted_year) - era * 400;
    let month_shifted = i64::from(if month > 2 { month - 3 } else { month + 9 });
    let day_of_year = (153 * month_shifted + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    Some(era * 146_097 + day_of_era - 719_468)
}

/// Current Unix timestamp in whole seconds.
pub(crate) fn current_unix_timestamp() -> Option<u64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs())
}

/// Rollback-protection answer for a model, or `""` when the model is
/// unknown.
///
/// `is_rollback_protected_model` is a deny-list (only TB322FC is exempt),
/// so an empty model used to come back protected — the Dashboard then
/// asserted "Yes" for a device it could not identify, and it was the one
/// field that never degraded to the em dash the others show. A blank
/// answer lets the Dashboard's existing empty-string path render `—`.
fn arb_from_model(model: &str) -> &'static str {
    if model.trim().is_empty() {
        ""
    } else if is_rollback_protected_model(model) {
        "arb_yes"
    } else {
        "arb_no"
    }
}

/// Normalize an optional fastboot `current-slot` to a partition suffix
/// (`_a`/`_b`), defaulting to `_a` when unknown (e.g. EDL-start with no
/// fastboot probe).
pub(crate) fn active_slot_suffix(slot: Option<&str>) -> &'static str {
    match slot {
        Some(s) if s.eq_ignore_ascii_case("_b") || s.eq_ignore_ascii_case("b") => "_b",
        _ => "_a",
    }
}

/// Read the device's committed AVB rollback index by dumping the active-slot
/// `boot` + `vbmeta_system` over EDL and taking the higher index. Used when
/// fastboot can't report `stored_rollback_index` (every model but the no-ARB
/// TB322FC). `slot` is the active-slot suffix; falls back to `_a` when
/// unknown. The max is the device's rollback floor — bumping a partition
/// above its own claim is safe, so the generic key-map overlay path needs
/// only this single value (vs the per-partition split the TB323FU testkey
/// path keeps for its re-sign targets).
fn read_device_rollback_index_via_edl(
    session: &mut ltbox_device::edl::EdlSession,
    slot: Option<&str>,
    work_dir: &std::path::Path,
    log: &mut Vec<String>,
) -> std::result::Result<u64, String> {
    let s = active_slot_suffix(slot);
    let boot = format!("boot{s}");
    let vbs = format!("vbmeta_system{s}");
    let boot_lun = ltbox_core::partition_lun::lun_for_partition(&boot)
        .ok_or_else(|| format!("no LUN for {boot}"))?;
    let vbs_lun = ltbox_core::partition_lun::lun_for_partition(&vbs)
        .ok_or_else(|| format!("no LUN for {vbs}"))?;
    let boot_img = work_dir.join(format!("dev_{boot}.img"));
    let vbs_img = work_dir.join(format!("dev_{vbs}.img"));
    session
        .dump_partition(&boot, &boot_img, 0, boot_lun, log)
        .map_err(|e| format!("dump device {boot}: {e}"))?;
    session
        .dump_partition(&vbs, &vbs_img, 0, vbs_lun, log)
        .map_err(|e| format!("dump device {vbs}: {e}"))?;
    let boot_idx = ltbox_patch::avb::extract_image_avb_info(&boot_img)
        .map_err(|e| format!("AVB {boot}: {e}"))?
        .rollback_index;
    let vbs_idx = ltbox_patch::avb::extract_image_avb_info(&vbs_img)
        .map_err(|e| format!("AVB {vbs}: {e}"))?
        .rollback_index;
    let _ = std::fs::remove_file(&boot_img);
    let _ = std::fs::remove_file(&vbs_img);
    Ok(boot_idx.max(vbs_idx))
}

/// Route device into EDL (Qualcomm 9008). Shared by Root/Unroot/Flash.
///
/// Already-EDL: no-op. Fastboot live: continue system boot, wait for ADB,
/// then `adb reboot edl`. ADB live: `adb reboot edl`. If ADB is not
/// usable, ask the user to reboot manually and wait for 9008.
///
/// `conn` is the caller's captured `App.connection`, used only as a
/// fallback. The body re-probes EDL → Fastboot → ADB live because flows
/// (e.g. Flash) may reboot the device themselves between worker spawn
/// and the EDL transition (ADB → bootloader for variable query), making
/// the captured `conn` stale.
pub(crate) fn transition_to_edl(
    conn: ConnectionStatus,
    log: &mut Vec<String>,
) -> std::result::Result<(), String> {
    ltbox_device::selection::ensure_single_usb_target().map_err(|e| e.to_string())?;
    let live = probe_connection_for_edl().unwrap_or(conn);
    ensure_edl(live, "EDL", log).map_err(|()| ltbox_core::i18n::tr("err_edl_transition_failed"))
}

/// Quick EDL/Fastboot/ADB probe in that order. Returns `None` only when
/// every transport is silent (caller falls back to its captured conn).
fn probe_connection_for_edl() -> Option<ConnectionStatus> {
    if ltbox_device::edl::check_device() {
        return Some(ConnectionStatus::Edl);
    }
    if ltbox_device::fastboot::FastbootDevice::check_device() {
        return Some(ConnectionStatus::Fastboot);
    }
    let mut adb = ltbox_device::adb::AdbManager::new();
    match adb.check_device_state().ok().flatten() {
        Some("device" | "recovery") => Some(ConnectionStatus::Adb),
        Some("adb_server_blocking") => Some(ConnectionStatus::AdbServerBlocking),
        Some("unauthorized" | "authorizing") => Some(ConnectionStatus::AdbUnauthorized),
        Some("sideload") => Some(ConnectionStatus::AdbSideload),
        _ => None,
    }
}

/// Wrap a heavy blocking flow as a `Task<Message>`. Runs `f` on the
/// 64 MiB heavy-task pool via `spawn_blocking + run_heavy`, then sends
/// the result through `done`. Both `run_heavy` panics and the
/// `spawn_blocking` JoinError collapse to a single error string passed
/// to `fallback`, so callers no longer hand-write the two-level
/// `unwrap_or_else` chain.
fn task_heavy<T, F, G, D>(f: F, done: D, fallback: G) -> Task<Message>
where
    F: FnOnce() -> T + Send + 'static,
    G: FnOnce(String) -> T + Send + 'static,
    D: FnOnce(T) -> Message + Send + 'static,
    T: Send + 'static,
{
    Task::perform(
        async move {
            match tokio::task::spawn_blocking(move || ltbox_core::runtime::run_heavy(f)).await {
                Ok(Ok(v)) => v,
                Ok(Err(e)) => fallback(e),
                Err(_) => fallback("task panicked".to_string()),
            }
        },
        done,
    )
}

/// Map a PTSTPD `MachineInfo`'s `SaleArea` to a flash region: `"CN"` → PRC,
/// JSON `null` → ROW, anything else (or missing) → `None` (can't infer).
fn region_from_salearea(info: &ltbox_core::lenovo_info::MachineInfo) -> Option<DeviceRegion> {
    match info.field("SaleArea") {
        ltbox_core::lenovo_info::FieldValue::Value(s) if s.eq_ignore_ascii_case("CN") => {
            Some(DeviceRegion::Prc)
        }
        ltbox_core::lenovo_info::FieldValue::Null => Some(DeviceRegion::Row),
        _ => None,
    }
}

/// Resolve the QFIL-firmware outcome for a serial (blocking; runs in the
/// worker). `cached` supplies MTM + SaleArea when already known; otherwise
/// machine info is fetched here. Non-CN `SaleArea` short-circuits to
/// [`QfilOutcome::Global`]; a CN device queries the official package.
fn resolve_qfil(serial: &str, cached: Option<(String, String)>) -> Result<QfilOutcome, String> {
    use ltbox_core::lenovo_info::FieldValue;
    let (mtm, area) = match cached {
        Some(t) => t,
        None => {
            let info =
                ltbox_core::lenovo_info::fetch_machine_info(serial).map_err(|e| e.to_string())?;
            let field = |k: &str| match info.field(k) {
                FieldValue::Value(s) => s,
                _ => String::new(),
            };
            (field("MTM"), field("SaleArea"))
        }
    };
    // Global (non-CN) devices have no PTSTPD flashing-machine entry.
    if !area.eq_ignore_ascii_case("CN") {
        return Ok(QfilOutcome::Global);
    }
    if mtm.trim().is_empty() {
        return Ok(QfilOutcome::NoPackage);
    }
    match ltbox_core::lenovo_qfil::fetch_qfil_package(&mtm).map_err(|e| e.to_string())? {
        Some(pkg) => Ok(QfilOutcome::Package(pkg)),
        None => Ok(QfilOutcome::NoPackage),
    }
}

// =========================================================================
// App
// =========================================================================

/// Which Advanced sub-wizard (if any) currently owns the screen. Sum
/// type so the dedicated sub-wizards stay mutually exclusive at the type
/// level — adding another wizard turns existing read sites into
/// non-exhaustive `match` errors instead of silent precedence bugs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum AdvancedWizardOpen {
    #[default]
    None,
    FlashParts,
    DumpParts,
    DumpPhys,
    FlashPhys,
    SimpleFlash,
}

impl AdvancedWizardOpen {
    fn is_open(self) -> bool {
        !matches!(self, Self::None)
    }
    fn is_flash_parts(self) -> bool {
        matches!(self, Self::FlashParts)
    }
    fn is_dump_parts(self) -> bool {
        matches!(self, Self::DumpParts)
    }
    fn is_dump_phys(self) -> bool {
        matches!(self, Self::DumpPhys)
    }
    fn is_flash_phys(self) -> bool {
        matches!(self, Self::FlashPhys)
    }
    fn is_simple_flash(self) -> bool {
        matches!(self, Self::SimpleFlash)
    }
}

fn partition_table_leading_action(
    entry_connection: Option<ConnectionStatus>,
) -> WizardLeadingAction {
    match entry_connection {
        Some(ConnectionStatus::Edl) | None => WizardLeadingAction::Back,
        Some(_) => WizardLeadingAction::Cancel,
    }
}

fn edl_entry_action(conn: ConnectionStatus) -> EdlEntryAction {
    match conn {
        ConnectionStatus::Edl => EdlEntryAction::AlreadyEdl,
        ConnectionStatus::Adb | ConnectionStatus::AdbRecovery => EdlEntryAction::AdbReboot,
        ConnectionStatus::Fastboot => EdlEntryAction::FastbootRebootThenAdb,
        ConnectionStatus::AdbUnauthorized
        | ConnectionStatus::AdbSideload
        | ConnectionStatus::AdbServerBlocking
        | ConnectionStatus::None => EdlEntryAction::ManualWait,
    }
}

/// Clamp a requested driver mode to what the host can actually use. Kernel mode
/// is forced back to userspace where it is unsupported (macOS, and non-Debian
/// Linux without `dpkg-query`), so a persisted/stale `kernel` value or a UI race
/// can never leave the app in an unusable kernel state. Mirrors the Settings
/// picker lock in `view::settings`.
fn effective_qcom_driver_mode(
    mode: ltbox_device::driver::QcomDriverMode,
) -> ltbox_device::driver::QcomDriverMode {
    if mode.is_kernel() && !ltbox_device::driver::kernel_mode_supported() {
        ltbox_device::driver::QcomDriverMode::Userspace
    } else {
        mode
    }
}

struct App {
    window_id: Option<iced::window::Id>,
    /// Host maximized state for the custom titlebar restore/maximize glyph.
    /// Iced exposes this as a query, not a window event, so update/window.rs
    /// refreshes it when the id arrives and after resize/toggle traffic.
    window_maximized: bool,
    current_view: View,
    /// Effective dark-mode flag — cached to keep repaint off the OS
    /// registry. Recomputed on theme-choice change.
    dark_mode: bool,
    theme_choice: ThemeChoice,
    theme_seed: ThemeSeed,
    /// Persisted font-source preference. The runtime font is bound before iced
    /// starts, so editing this field only affects the next launch.
    use_system_font: bool,
    settings: SettingsState,
    translations: Translations,
    /// Per-launch disclaimer gate. It is deliberately absent from persisted settings.
    startup_disclaimer_open: bool,
    startup_disclaimer_checked: bool,
    /// Session-only open state for the About screen's license inventory.
    about_licenses_open: bool,
    root: RootWizard,
    flash: FlashWizard,
    sysupdate: SysUpdateWizard,
    unroot: UnrootWizard,
    /// Staged path for the pending advanced action — replayed into the
    /// exec path on Start so no second dialog fires.
    adv_confirm_path: Option<String>,
    adv_wizard: AdvWizard,
    /// Dedicated EDL-based KonaBess flow; target-popup state is owned here.
    konabess: KonaBessWizard,
    wf_config: WorkflowConfig,
    /// Flash-confirm "hidden dropdown" editor: which row's option picker is
    /// open (`None` = closed). `Country` reuses `country_popup_open` instead.
    confirm_edit_field: Option<ConfirmField>,
    /// Snapshot of `wf_config` taken when the confirm step is first entered.
    /// A confirm row is rendered as "changed" (accent background + hover
    /// caution) when its field diverges from this baseline.
    confirm_baseline: Option<WorkflowConfig>,
    /// Confirm-step manual rollback editor. `Some` only while open; its
    /// values become `wf_config` targets only after a valid confirm.
    manual_rollback_editor: Option<ManualRollbackEditor>,
    /// In-flight text for the two manual rollback fields. Kept outside the
    /// editor so a cancelled popup cannot mutate the confirmed targets.
    manual_rollback_buffers: Option<(String, String)>,
    /// Last value each buffer parsed to. Switching display format renders
    /// from this rather than re-parsing the text, because the date form has
    /// day granularity and a text round-trip would silently move a value
    /// back to midnight.
    manual_rollback_values: (Option<u64>, Option<u64>),
    country_popup_open: bool,
    /// Live search text and the row staged inside the country popup. The
    /// committed workflow value is only changed by the popup footer action.
    country_popup_search: String,
    country_popup_draft: CountryAction,
    /// Routes `SelectCountry` back to the Advanced wizard instead of
    /// the Flash flow when PatchDevinfo opened the popup.
    adv_needs_country: bool,
    /// Region-convert target picker overlay. Shown when the
    /// `RegionConvert` wizard reaches step 1 so the user can pick
    /// PRC or ROW as the destination explicitly instead of relying
    /// on the prior auto-flip behaviour.
    region_target_popup_open: bool,
    /// Staging slot for the Reboot confirm popup.
    reboot_confirm_target: Option<RebootTarget>,
    /// True while the current tracked Reboot operation is waiting for EDL.
    /// Kept separate from visibility so Close can hide only the dialog.
    reboot_wait_transition: bool,
    reboot_wait_dialog_open: bool,
    // Device & operation state
    software_fix: software_fix::State,
    device: DeviceSnapshot,
    queries: DeviceQueries,
    adb_server_kill_in_flight: bool,
    /// Device-info popup state. `Some((serial, state))` while open.
    device_info_popup: Option<(String, DeviceInfoState)>,
    /// Firmware-OTA popup state. `Some((serial, firmware_id, state))` while open.
    ota_popup: Option<(String, String, OtaPopupState)>,
    /// Selectable mirror of OTA changelog — `text` widget can't be selected.
    ota_changelog_editor: iced::widget::text_editor::Content,
    /// QFIL-firmware popup state. `Some((serial, state))` while open.
    qfil_popup: Option<(String, QfilPopupState)>,
    /// Manual-serial prompt for region detection. `Some(buffer)` = open;
    /// buffer holds the in-progress input. Opened by the automatic-selection
    /// option when no
    /// usable polled serial is available.
    flash_serial_prompt: Option<String>,
    rollback_popup_open: bool,
    /// Shared across both rows so `boot` and `vbmeta_system` stay
    /// directly comparable while cycling.
    rollback_value_format: RollbackValueFormat,
    /// Separate from `rollback_value_format`: the dashboard parses fastboot
    /// `getvar` output and so defaults to hex, while this editor mirrors
    /// `avbtool info_image`, which prints a plain unix timestamp.
    manual_rollback_format: RollbackValueFormat,
    /// Transient toast message; auto-cleared by a delayed task.
    toast_msg: Option<String>,
    /// Compact-sidebar hover state — true when the mouse is over the rail.
    sidebar_expanded: bool,
    /// Compact overlay tween progress in [0.0, 1.0].
    /// Width = lerp(64, 232, anim); Expanded layout ignores this value.
    /// Driven by an M3 Expressive Spatial spring (see `SidebarAnimTick`).
    sidebar_anim: f32,
    /// Spring velocity for `sidebar_anim`. Settle requires both the
    /// displacement to target AND the velocity to be near zero so we
    /// don't stop the subscription mid-overshoot.
    sidebar_velocity: f32,
    sidebar_label_alpha: f32,
    sidebar_label_velocity: f32,
    /// Current logical window size. Tracks `Event::Window(Resized)`
    /// so the user's preferred geometry survives restarts via
    /// `PersistedSettings::window_size`. A simple `Instant` debounce
    /// throttles persistence writes during cursor-drag resize since
    /// resize events fire on every frame.
    window_size: (f32, f32),
    /// Last instant a window-size save hit disk. Cursor-drag resize
    /// fires `Resized` continuously; persistence is throttled to once
    /// per `WINDOW_SIZE_SAVE_INTERVAL`.
    window_size_last_save: std::time::Instant,
    /// `true` while a pending window-size update hasn't been flushed
    /// to disk. Cleared by `persist_window_size_if_due`.
    window_size_dirty: bool,
    // Device portrait derived at view time via `device_portrait()`.
    operation: OperationExecution,
    /// Persisted recent picks. Rendered as chips under every picker.
    recent_paths: settings_store::RecentPaths,
    /// When set, every loader picker bypasses to this path. Re-validated at exec.
    default_loader_path: Option<String>,
    qcom_driver_mode: ltbox_device::driver::QcomDriverMode,
    /// `true` while a Settings "Clean temporary files" sweep is running.
    cleaning_temp: bool,
    /// Cached on-disk size of removable temp files (`work_*` + `output_*`),
    /// rescanned on Settings entry and after a sweep. `None` until first
    /// scanned; drives the cleanup button's enabled state + size readout.
    temp_files_bytes: Option<u64>,
    log_lines: Vec<String>,
    /// Selectable mirror of `log_lines`. Rebuilt on drain tick when `log_dirty`
    /// — batched to keep a long pbr flash from crashing wgpu.
    log_editor: iced::widget::text_editor::Content,
    log_dirty: bool,
    image_info_log: String,
    image_info_log_editor: iced::widget::text_editor::Content,
    pending_log_save_source: LogSaveSource,
    error_msg: Option<String>,
    operation_error: Option<String>,
    picker_target: PickerTarget,
    driver_status: Option<ltbox_device::driver::DriverStatus>,
    installing_drivers: bool,
    /// Session-only post-install reminder. Set after a successful Qualcomm
    /// driver install/update and cleared when the user closes its banner.
    driver_restart_recommended: bool,
    /// `Some` when the installed Qualcomm driver is older than the latest
    /// release — drives the optional amber "update available" banner. Held
    /// `None` when up to date, not installed, offline, or the user chose
    /// "don't show again".
    driver_update: Option<ltbox_device::driver::DriverUpdate>,
    /// Result of the startup GitHub-reachability probe. `None` until the
    /// probe lands; `Some(false)` disables the driver install/update
    /// buttons with an "internet required" tooltip.
    online: Option<bool>,
    /// Persisted "don't show again" for the driver-update prompt. Skips the
    /// update check + banner; never affects the missing-driver banner.
    qcom_driver_update_dismissed: bool,
    /// Models whose dual-USB-C port guide the user permanently dismissed
    /// ("don't show again"); loaded from + saved to settings.
    dual_usb_advisory_dismissed: Vec<String>,
    /// Models whose guide was closed this session only ("close"). Not
    /// persisted, so the guide returns on the next launch.
    dual_usb_advisory_closed: Vec<String>,
    /// Whether the illustrated dual-USB-C port guide is open.
    dual_usb_help_open: bool,
    /// Model the open dual-USB-C port guide describes. Session-only so the
    /// guide keeps its subject while the live device disconnects or changes.
    dual_usb_help_model: String,
    /// Friendly name captured with the model so the guide subtitle survives
    /// a disconnect while the dialog remains open.
    dual_usb_help_name: String,
    /// Newest stable (`prerelease == false && draft == false`) release on
    /// `miner7222/LTBox` whose semver is strictly greater than the
    /// running build's. `None` either before the background probe lands
    /// or when the running build is already at-or-ahead of the latest
    /// stable. Populates the green sidebar "Update available" pill.
    update_available: Option<ltbox_core::github::StableRelease>,
    /// Package-managed install source while the update instructions dialog is
    /// open, or `Direct` while the verified self-update dialog is open.
    update_dialog_source: Option<ltbox_core::install_source::InstallSource>,
    flash_parts: FlashPartsWizard,
    dump_parts: DumpPartsWizard,
    dump_phys: DumpPhysWizard,
    flash_phys: FlashPhysWizard,
    simple_flash: SimpleFlashWizard,
    /// Single sum-typed flag for the mutually-exclusive Advanced
    /// sub-wizards. Replaces 4 parallel booleans whose `if/else if`
    /// read sites would silently pick a precedence if two ever got
    /// set. `match`-driven dispatch makes that bug class unreachable.
    advanced_wizard_open: AdvancedWizardOpen,
    /// Latest live firmware flash progress snapshot for the shared exec card.
    flash_progress: Option<ltbox_device::edl::FlashProgress>,
    log_popup_open: bool,
    #[cfg(feature = "demo")]
    demo_scene: Option<demo::Scene>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum PickerTarget {
    #[default]
    None,
    RootFile,
    UnrootFolder,
    FlashFolder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum LogSaveSource {
    #[default]
    Main,
    ImageInfo,
}

impl PickerTarget {
    /// Map this routing target to the recents bucket it should store into.
    /// `None` returns `File` defensively so callers get a valid bucket even
    /// if they forgot to set the target — the recents entry is harmless;
    /// the field-routing `match` in `FolderSelected` / `FileSelected` is
    /// what actually prevents wrong writes.
    fn kind(self) -> pickers::PickerKind {
        use pickers::PickerKind;
        match self {
            // Root OTA file is a unified file pick (zip or apk).
            Self::None | Self::RootFile => PickerKind::File,
            // Firmware folders all share the "full QFIL" bucket — Unroot
            // and Flash typically point the user at the same dump/archive
            // they extracted from `ltbox dump full`.
            Self::UnrootFolder | Self::FlashFolder => PickerKind::QfilFirmwareFolder,
        }
    }
}

impl Default for App {
    fn default() -> Self {
        let persisted = settings_store::load();
        let lang = Language::from_code(&persisted.language).unwrap_or(Language::En);
        // Upgrade path: prefer `theme`, fall back to legacy `dark_mode`.
        let theme_choice = ThemeChoice::from_code(&persisted.theme).unwrap_or({
            if persisted.theme.is_empty() && persisted.dark_mode {
                ThemeChoice::Dark
            } else {
                ThemeChoice::System
            }
        });
        let theme_seed = ThemeSeed::from_code(&persisted.theme_seed).unwrap_or_default();
        let qcom_driver_mode = effective_qcom_driver_mode(
            ltbox_device::driver::QcomDriverMode::from_code(&persisted.qcom_driver_mode),
        );
        ltbox_device::driver::set_qcom_driver_mode(qcom_driver_mode);
        let dark_mode = match theme_choice {
            ThemeChoice::Light => false,
            ThemeChoice::Dark => true,
            ThemeChoice::System => theme_detect::system_prefers_dark(),
        };
        theme::set_runtime_theme(theme_seed, dark_mode);
        install_core_translator(lang);
        let translations = Translations::load(lang);
        let ready_log = translations.t("log_ready").to_string();
        Self {
            window_id: None,
            window_maximized: false,
            current_view: View::default(),
            dark_mode,
            theme_choice,
            theme_seed,
            use_system_font: persisted.use_system_font,
            settings: SettingsState { language: lang },
            translations,
            startup_disclaimer_open: true,
            startup_disclaimer_checked: false,
            about_licenses_open: false,
            root: RootWizard::default(),
            flash: FlashWizard::default(),
            sysupdate: SysUpdateWizard::default(),
            unroot: UnrootWizard::default(),
            adv_confirm_path: None,
            adv_wizard: AdvWizard::default(),
            konabess: KonaBessWizard::default(),
            wf_config: WorkflowConfig::default(),
            confirm_edit_field: None,
            confirm_baseline: None,
            manual_rollback_editor: None,
            manual_rollback_buffers: None,
            manual_rollback_values: (None, None),
            country_popup_open: false,
            country_popup_search: String::new(),
            country_popup_draft: CountryAction::Unset,
            adv_needs_country: false,
            region_target_popup_open: false,
            reboot_confirm_target: None,
            reboot_wait_transition: false,
            reboot_wait_dialog_open: false,
            software_fix: software_fix::State::default(),
            device: DeviceSnapshot::default(),
            queries: DeviceQueries::default(),
            adb_server_kill_in_flight: false,
            device_info_popup: None,
            ota_popup: None,
            ota_changelog_editor: iced::widget::text_editor::Content::with_text(""),
            qfil_popup: None,
            flash_serial_prompt: None,
            rollback_popup_open: false,
            rollback_value_format: RollbackValueFormat::default(),
            manual_rollback_format: RollbackValueFormat::Unix,
            toast_msg: None,
            sidebar_expanded: false,
            sidebar_anim: 0.0,
            sidebar_velocity: 0.0,
            sidebar_label_alpha: 0.0,
            sidebar_label_velocity: 0.0,
            // Use the persisted size if present, otherwise the default
            // initial window dimensions (kept in lockstep with the
            // values passed to `iced::window::Settings::size` in `main`).
            window_size: persisted
                .window_size
                .unwrap_or((DEFAULT_WINDOW_WIDTH, DEFAULT_WINDOW_HEIGHT)),
            window_size_last_save: std::time::Instant::now(),
            window_size_dirty: false,
            operation: OperationExecution::default(),
            recent_paths: persisted.recent_paths.clone(),
            default_loader_path: persisted.default_loader_path.clone(),
            qcom_driver_mode,
            cleaning_temp: false,
            temp_files_bytes: None,
            log_lines: vec![ready_log.clone()],
            log_editor: iced::widget::text_editor::Content::with_text(&ready_log),
            log_dirty: false,
            image_info_log: String::new(),
            image_info_log_editor: iced::widget::text_editor::Content::with_text(""),
            pending_log_save_source: LogSaveSource::Main,
            error_msg: None,
            operation_error: None,
            picker_target: PickerTarget::None,
            driver_status: None,
            installing_drivers: false,
            driver_restart_recommended: false,
            driver_update: None,
            online: None,
            qcom_driver_update_dismissed: persisted.qcom_driver_update_dismissed,
            dual_usb_advisory_dismissed: persisted.dual_usb_advisory_dismissed_models.clone(),
            dual_usb_advisory_closed: Vec::new(),
            dual_usb_help_open: false,
            dual_usb_help_model: String::new(),
            dual_usb_help_name: String::new(),
            update_available: None,
            update_dialog_source: None,
            flash_parts: FlashPartsWizard::default(),
            dump_parts: DumpPartsWizard::default(),
            dump_phys: DumpPhysWizard::default(),
            flash_phys: FlashPhysWizard::default(),
            simple_flash: SimpleFlashWizard::default(),
            advanced_wizard_open: AdvancedWizardOpen::default(),
            flash_progress: None,
            log_popup_open: false,
            #[cfg(feature = "demo")]
            demo_scene: None,
        }
    }
}

impl App {
    fn new() -> (Self, Task<Message>) {
        // Window-id + driver check + update check all fire in parallel.
        let app = Self::default();
        #[cfg(feature = "demo")]
        let mut app = app;
        #[cfg(feature = "demo")]
        demo::initialize(&mut app);
        let win =
            iced::window::latest().map(|__v| Message::Window(WindowMsg::WindowIdReceived(__v)));
        #[cfg(feature = "demo")]
        if demo::is_active(&app) {
            return (app, win);
        }
        let driver_check = Task::perform(
            async {
                tokio::task::spawn_blocking(ltbox_device::driver::check_required_drivers)
                    .await
                    .unwrap_or(ltbox_device::driver::DriverStatus::NotWindows)
            },
            Message::DriverCheckDone,
        );
        // GitHub releases probe — runs once at startup. `latest_stable_release`
        // walks `/releases?per_page=100` (not `/releases/latest`) so the
        // result is well-defined even when the repo has only prereleases
        // published. Network failure / parse failure → `None`, no banner.
        let update_check = Task::perform(
            async {
                tokio::task::spawn_blocking(check_for_update)
                    .await
                    .unwrap_or(None)
            },
            Message::UpdateCheckDone,
        );
        // GitHub-reachability probe — gates the driver install/update
        // buttons so the user can't click into a guaranteed-to-fail
        // download while offline.
        let connectivity = Task::perform(
            async {
                tokio::task::spawn_blocking(ltbox_device::driver::probe_connectivity)
                    .await
                    .unwrap_or(false)
            },
            Message::ConnectivityChecked,
        );
        // Advisory startup probe, separate from the driver-button gate
        // above: it splits "no link at all" from "link up, GitHub
        // blocked" so the log can name which one the user is hitting.
        // Nothing waits on it and nothing is gated by it.
        let connectivity_notice = Task::perform(
            async {
                tokio::task::spawn_blocking(ltbox_core::connectivity::probe)
                    .await
                    .unwrap_or(ltbox_core::connectivity::ConnectivityReport {
                        internet: true,
                        github: true,
                    })
            },
            Message::StartupConnectivityProbed,
        );
        // Qualcomm driver version check. Skipped entirely (no network call)
        // when the user chose "don't show again" for driver updates. A
        // silent failure (offline / GitHub down / parse) yields `None`, so
        // no banner — distinct from the missing-driver banner, which the
        // separate `driver_check` above always drives.
        let driver_update_check = if app.qcom_driver_update_dismissed {
            Task::none()
        } else {
            Task::perform(
                async {
                    tokio::task::spawn_blocking(ltbox_device::driver::check_driver_update)
                        .await
                        .unwrap_or(None)
                },
                Message::DriverUpdateCheckDone,
            )
        };
        (
            app,
            Task::batch([
                win,
                driver_check,
                update_check,
                connectivity,
                connectivity_notice,
                driver_update_check,
                Task::done(Message::PollSoftwareFix),
            ]),
        )
    }
    fn theme(&self) -> Theme {
        self.sync_runtime_theme();
        Theme::custom(
            format!(
                "LTBox {} {}",
                self.theme_seed.code(),
                if self.dark_mode { "dark" } else { "light" }
            ),
            theme::iced_palette(self.theme_seed, self.dark_mode),
        )
    }

    fn sync_runtime_theme(&self) {
        theme::set_runtime_theme(self.theme_seed, self.dark_mode);
    }

    /// Localized string. Falls back to English, then the key itself.
    fn t<'a>(&'a self, key: &'a str) -> &'a str {
        self.translations.t(key)
    }

    fn pal(&self) -> Palette {
        palette_for(self.theme_seed, self.dark_mode)
    }

    /// Push one line, trim to `LOG_MAX_LINES`. Editor rebuild is
    /// deferred to the drain tick — per-push reshape was driving
    /// wgpu into TDR during long pbr flashes.
    fn log_push<S: Into<String>>(&mut self, line: S) {
        let s = line.into();
        self.log_lines.push(s);
        self.trim_log();
        self.log_dirty = true;
    }

    fn parse_manual_rollback(&self, input: &str) -> Result<u64, String> {
        let index = self.manual_rollback_format.parse(input)?;
        let now =
            current_unix_timestamp().ok_or_else(|| "rollback_manual_error_clock".to_string())?;
        if index < now {
            Ok(index)
        } else {
            Err("rollback_manual_error_future".to_string())
        }
    }

    pub(crate) fn open_manual_rollback_editor(&mut self) -> Task<Message> {
        let advanced = self.current_view == View::Advanced
            && self.adv_wizard.action == Some(AdvAction::PatchArb);
        let defaults = if advanced {
            self.adv_wizard.arb_inspect.map(|(a, b)| (Ok(a), Ok(b)))
        } else {
            self.flash.firmware_rollback_indices.clone()
        };
        let Some(defaults) = defaults else {
            return Task::none();
        };

        let seed = |result: &Result<u64, String>| -> String {
            result.as_ref().ok().map_or_else(String::new, |index| {
                self.manual_rollback_format.render(*index)
            })
        };
        // Values the user already confirmed win over the image defaults —
        // reopening the editor to check a number must not silently discard it.
        // The image index stays visible under each field either way.
        let buffers = match if advanced {
            self.adv_wizard.arb_targets
        } else {
            self.wf_config.manual_rollback_indices
        } {
            Some(entered) => (
                self.manual_rollback_format.render(entered.boot),
                self.manual_rollback_format.render(entered.vbmeta_system),
            ),
            None => (seed(&defaults.0), seed(&defaults.1)),
        };
        self.confirm_edit_field = if advanced {
            None
        } else {
            Some(ConfirmField::Rollback)
        };
        self.manual_rollback_editor = Some(ManualRollbackEditor::Boot);
        self.manual_rollback_values = (
            self.manual_rollback_format.parse(&buffers.0).ok(),
            self.manual_rollback_format.parse(&buffers.1).ok(),
        );
        self.manual_rollback_buffers = Some(buffers);
        Task::none()
    }

    /// Compare the confirmed Manual targets against the selected firmware's
    /// own image indices. Device floors are intentionally not consulted.
    pub(crate) fn manual_rollback_downgrade_warning(&self) -> Option<()> {
        let targets = self.wf_config.manual_rollback_indices?;
        let originals = self.flash.firmware_rollback_indices.as_ref()?;
        let boot_lower = matches!(&originals.0, Ok(original) if targets.boot < *original);
        let vbmeta_lower =
            matches!(&originals.1, Ok(original) if targets.vbmeta_system < *original);
        (boot_lower || vbmeta_lower).then_some(())
    }

    /// Tap + sink drain shared by `Message::DrainStdoutTap` and
    /// every `*ExecDone` handler. Pulls third-party `println!` from
    /// the Windows stdout pipe AND our own `live!` lines from the
    /// in-process sink, dedupes against the recent log tail (catches
    /// the tap-late race where a `live!` line lands in the sink at
    /// tick T and only surfaces in the tap at tick T+1) and
    /// in-batch (catches the same line landing in BOTH streams at
    /// the same tick). Returns count of new lines added so callers
    /// can decide whether to rebuild the editor.
    fn drain_pending_log_streams(&mut self) -> usize {
        let tap_lines = stdout_tap::drain();
        let sink_lines = ltbox_core::live_sink::drain();
        let total = tap_lines.len() + sink_lines.len();
        if total == 0 {
            return 0;
        }
        let mut seen: std::collections::HashSet<String> =
            std::collections::HashSet::with_capacity(total + 32);
        let tail_window = self.log_lines.len().saturating_sub(32);
        seen.extend(self.log_lines[tail_window..].iter().cloned());
        let mut combined: Vec<String> = Vec::with_capacity(total);
        for line in tap_lines.into_iter().chain(sink_lines) {
            if seen.insert(line.clone()) {
                combined.push(line);
            }
        }
        let added = combined.len();
        if added > 0 {
            self.log_extend(combined);
        }
        added
    }

    /// Final flush at `*ExecDone` time. The closure's local Vec is
    /// dropped — `live!` already pushed every line through the sink
    /// path (bulk-streamed across the run) and the macro's Vec copy
    /// is pure dead weight at completion time. Re-appending it via
    /// `log_extend` doubled the entire transcript on screen; the
    /// adjacent-tail dedup only collapses the boundary line, not the
    /// 100+ interior lines.
    fn flush_exec_done_log(&mut self, _vec_from_closure: Vec<String>) {
        // `_vec_from_closure` intentionally ignored — see above.
        // Drain whatever the 500 ms tick missed between the last
        // `Message::DrainStdoutTap` and the closure's return so the
        // user sees the closing lines without a tick of latency.
        self.drain_pending_log_streams();
    }

    /// Bulk append; one truncation pass.
    fn log_extend<I: IntoIterator<Item = String>>(&mut self, lines: I) {
        // Adjacent dedup against the existing tail collapses repeated
        // streamed lines without duplicating the visible transcript.
        let mut prev_tail = self.log_lines.last().cloned();
        let mut accepted: Vec<String> = Vec::new();
        for line in lines {
            if prev_tail.as_deref() == Some(line.as_str()) {
                continue;
            }
            prev_tail = Some(line.clone());
            accepted.push(line);
        }
        if !accepted.is_empty() {
            self.log_lines.extend(accepted);
            self.trim_log();
            self.log_dirty = true;
        }
    }

    fn begin_op(&mut self, view: View) {
        self.begin_silent_op(view);
        let label = self.t("log_separator_start").to_string();
        self.log_separator(Some(&label));
    }

    fn begin_phased_op(&mut self, view: View, kind: OperationPhaseKind) -> PhaseReporter {
        debug_assert!(OperationPhaseKind::all().contains(&kind));
        let reporter = PhaseReporter::from_labels(
            kind.keys()
                .iter()
                .map(|key| self.t(key).to_string())
                .collect(),
        );
        self.reset_operation_feedback();
        self.operation.start(
            Some(view),
            OperationKind::Phased(kind),
            Some(reporter.clone()),
        );
        let label = self.t("log_separator_start").to_string();
        self.log_separator(Some(&label));
        reporter
    }

    /// Snapshot localized log strings for use across thread boundaries.
    fn live_labels(&self) -> LiveLabels {
        let t = |k: &str| self.t(k).to_string();
        LiveLabels {
            closing_dump: t("live_closing_dump_session"),
            flash_completed: t("live_flash_completed"),
            adb_no_kver: t("live_adb_no_kver"),
            backup_saved_prefix: t("live_backup_saved_prefix"),
            root_resolved_prefix: t("live_root_resolved_prefix"),
            root_backup_copy_prefix: t("live_root_backup_copy_prefix"),
        }
    }

    fn end_op(&mut self) {
        self.operation.finish(true);
        self.clear_flash_progress();
    }

    fn fail_op(&mut self) {
        self.operation.finish(false);
        self.clear_flash_progress();
    }

    fn reset_operation_feedback(&mut self) {
        self.error_msg = None;
        self.operation_error = None;
        self.clear_flash_progress();
    }

    fn begin_silent_op(&mut self, view: View) {
        self.reset_operation_feedback();
        self.operation
            .start(Some(view), OperationKind::Unphased, None);
    }

    fn end_silent_op(&mut self) {
        self.fail_op();
    }

    fn clear_flash_progress(&mut self) {
        self.flash_progress = None;
        ltbox_device::edl::clear_flash_progress();
    }

    /// True only while a busy op is on the exact firmware-write progress phase.
    fn firmware_write_progress_phase_active(&self) -> bool {
        if !self.operation.is_running() {
            return false;
        }
        let Some(kind) = self.operation.phase_kind() else {
            return false;
        };
        // Overflow-safe: current operation step is zero-based; policies use
        // the one-based phase numbers printed in the live log.
        self.operation
            .current_step()
            .checked_add(1)
            .is_some_and(|step| kind.is_firmware_progress_step(step))
    }

    fn refresh_flash_progress_snapshot(&mut self) {
        self.flash_progress = if self.firmware_write_progress_phase_active() {
            ltbox_device::edl::flash_progress()
        } else {
            None
        };
    }

    /// Secondary line under the firmware-write phase label, e.g. `super (42%)`.
    pub(crate) fn firmware_flash_progress_label(&self) -> Option<String> {
        if !self.firmware_write_progress_phase_active() || self.operation_error.is_some() {
            return None;
        }
        let progress = self.flash_progress.as_ref()?;
        if progress.partition.is_empty() {
            return None;
        }
        Some(format!("{} ({}%)", progress.partition, progress.percent))
    }

    fn set_image_info_log(&mut self, text: String) {
        self.image_info_log = text;
        self.image_info_log_editor =
            iced::widget::text_editor::Content::with_text(&self.image_info_log);
        use iced::widget::text_editor::{Action, Motion};
        self.image_info_log_editor
            .perform(Action::Move(Motion::DocumentEnd));
    }

    fn image_info_exec_active(&self) -> bool {
        self.current_view == View::Advanced
            && self.adv_wizard.is_image_info()
            && self.adv_wizard.step == self.adv_wizard.exec_step()
    }

    fn active_log_save_source(&self) -> LogSaveSource {
        if self.image_info_exec_active() {
            LogSaveSource::ImageInfo
        } else {
            LogSaveSource::Main
        }
    }

    fn country_popup_selected_code(&self) -> Option<&str> {
        if self.adv_needs_country {
            self.adv_wizard.country.as_deref()
        } else {
            self.wf_config.country_action.target()
        }
    }

    fn log_text_for_save(&self, source: LogSaveSource) -> String {
        match source {
            LogSaveSource::Main => self.log_lines.join("\n"),
            LogSaveSource::ImageInfo => self.image_info_log.clone(),
        }
    }

    fn note_log_save_result(&mut self, source: LogSaveSource, line: String) {
        match source {
            LogSaveSource::Main => self.log_push(line),
            LogSaveSource::ImageInfo => {
                let mut text = self.image_info_log.trim_end().to_string();
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(&line);
                self.set_image_info_log(text);
            }
        }
    }

    /// 80-wide `=` separator with an optional centred label.
    fn log_separator(&mut self, label: Option<&str>) {
        const BAR: &str =
            "================================================================================";
        let line = match label {
            Some(s) if !s.is_empty() => {
                let inner = format!(" {s} ");
                let bar_len = BAR.len();
                let inner_len = inner.chars().count();
                if inner_len >= bar_len {
                    inner
                } else {
                    let side = (bar_len - inner_len) / 2;
                    let left = &BAR[..side];
                    let right = &BAR[..bar_len - side - inner_len];
                    format!("{left}{inner}{right}")
                }
            }
            _ => BAR.to_string(),
        };
        self.log_push(line);
    }

    fn trim_log(&mut self) {
        if self.log_lines.len() > LOG_MAX_LINES {
            let drop = self.log_lines.len() - LOG_MAX_LINES;
            self.log_lines.drain(..drop);
        }
    }

    fn advanced_inline_exec_surface_active(&self) -> bool {
        if self.advanced_wizard_open.is_flash_parts() {
            return self.flash_parts.step >= 3;
        }
        if self.advanced_wizard_open.is_dump_parts() {
            return self.dump_parts.step >= 2;
        }
        if self.advanced_wizard_open.is_dump_phys() {
            return self.dump_phys.step >= 2;
        }
        if self.advanced_wizard_open.is_flash_phys() {
            return self.flash_phys.step >= 3;
        }
        if self.advanced_wizard_open.is_simple_flash() {
            return self.simple_flash.step >= 2;
        }
        self.adv_wizard.action.is_some() && self.adv_wizard.step == self.adv_wizard.exec_step()
    }

    fn current_view_has_inline_exec_surface(&self) -> bool {
        match self.current_view {
            View::Flash => self.flash.is_in_exec(),
            View::SystemUpdate => self.sysupdate.is_in_exec(),
            View::Root => self.root.is_in_exec(),
            View::Unroot => self.unroot.is_in_exec(),
            View::KonaBess => self.konabess.step >= 3,
            View::Advanced => self.advanced_inline_exec_surface_active(),
            View::Dashboard | View::Reboot | View::Settings | View::About => false,
        }
    }

    fn current_view_shows_shared_exec_surface(&self) -> bool {
        self.current_view_has_inline_exec_surface() && !self.image_info_exec_active()
    }

    fn should_show_error_banner(&self) -> bool {
        let shared_surface_owns_error = self.current_view_shows_shared_exec_surface()
            && self.operation_error.is_some()
            && self.operation_error.as_deref() == self.error_msg.as_deref();
        self.error_msg.is_some() && !shared_surface_owns_error
    }

    fn blocking_popup_open(&self) -> bool {
        self.country_popup_open
            || self.konabess.target_popup_open
            || self.reboot_confirm_target.is_some()
            || self.sysupdate.rescue_region_popup_open
            || self.root.superkey_popup_open
            || self.root.run_id_popup_open
            || self.root.kernel_version_popup_open
    }

    /// True when the Advanced view holds wizard state the user would lose to a
    /// sidebar bounce: an op's exec/result surface (running, or its Done screen
    /// still up), a generic op sitting on its confirm step, or a partition
    /// read/write whose GPT table is still valid (device still in EDL). The
    /// `Navigate` handler consults this to skip the entry-time reset so
    /// navigating away and back keeps the user's place.
    fn advanced_in_progress(&self) -> bool {
        // The exec/result surface must survive a sidebar bounce until the user
        // hits 'start over' — mirrors the `is_in_exec()` gate the Root / Flash /
        // etc. views use in `Navigate`. `busy` already covers a running op; this
        // also keeps the result on screen after the op finishes.
        if self.advanced_inline_exec_surface_active() {
            return true;
        }
        use AdvancedWizardOpen as W;
        match self.advanced_wizard_open {
            // Generic advanced op (PatchArb / PatchDevinfo / DetectArb / ...):
            // preserve only on the confirm step (waiting to start).
            W::None => self.adv_wizard.action.is_some() && self.adv_wizard.is_confirm_step(),
            // Read/Write Partitions: a rendered GPT table — or the confirm
            // screen after it — survives as long as the device stays in EDL,
            // since the table reflects the live partition layout.
            W::FlashParts => {
                self.device.connection == ConnectionStatus::Edl && !self.flash_parts.rows.is_empty()
            }
            W::DumpParts => {
                self.device.connection == ConnectionStatus::Edl && !self.dump_parts.rows.is_empty()
            }
            // Physical storage: preserve the confirm screen (FlashPhys);
            // DumpPhys runs Select → Exec with no confirm screen to preserve.
            W::FlashPhys => self.flash_phys.step + 2 == FLASH_PHYS_STEPS.len(),
            W::DumpPhys => false,
            // Simple Flash: preserve the confirm screen (folder already
            // picked) so a sidebar bounce returns the user to it.
            W::SimpleFlash => self.simple_flash.step == 1,
        }
    }

    /// True while KonaBess owns a prepared EDL workspace whose table or
    /// confirm screen must survive a sidebar bounce. This mirrors the
    /// partition-table branch of `advanced_in_progress`.
    fn konabess_in_progress(&self) -> bool {
        self.device.connection == ConnectionStatus::Edl
            && self.konabess.prepared.is_some()
            && matches!(self.konabess.step, 1 | 2)
    }

    fn should_show_busy_progress_dialog(&self) -> bool {
        let dedicated_reboot_wait =
            self.reboot_wait_transition && self.operation.view() == Some(View::Reboot);
        self.operation.is_running()
            // The temp-file cleanup borrows `busy` only to lock out racing
            // device ops; it's a sub-second maintenance action with its own
            // in-button "Cleaning…" state, so it gets no full-screen dialog.
            && !self.cleaning_temp
            && self.current_view != View::Dashboard
            && !self.blocking_popup_open()
            && !self.current_view_has_inline_exec_surface()
            // The EDL transition has its own truthful checklist. Keep the
            // generic spinner suppressed even after that modeless dialog is
            // closed; the tracked operation itself continues unchanged.
            && !dedicated_reboot_wait
    }

    fn advanced_operation_label(&self) -> Option<String> {
        if self.advanced_wizard_open.is_flash_parts() {
            return Some(self.t(AdvAction::FlashPartitions.label_key()).to_string());
        }
        if self.advanced_wizard_open.is_dump_parts() {
            return Some(self.t(AdvAction::DumpPartitions.label_key()).to_string());
        }
        if self.advanced_wizard_open.is_dump_phys() {
            return Some(self.t(AdvAction::DumpPhysical.label_key()).to_string());
        }
        if self.advanced_wizard_open.is_flash_phys() {
            return Some(self.t(AdvAction::FlashPhysical.label_key()).to_string());
        }
        if self.advanced_wizard_open.is_simple_flash() {
            return Some(self.t(AdvAction::SimpleFlash.label_key()).to_string());
        }
        self.adv_wizard
            .action
            .map(|action| self.t(action.label_key()).to_string())
    }

    fn busy_operation_label(&self) -> String {
        if self.operation.view() == Some(View::Advanced)
            && let Some(label) = self.advanced_operation_label()
        {
            return label;
        }
        self.operation
            .view()
            .map(|view| self.t(view.label_key()).to_string())
            .unwrap_or_else(|| self.t("status_working").to_string())
    }

    /// Override busy-dialog body for the four Advanced partition/physical
    /// flows during their reboot → loader → GPT-scan preamble. Gated on
    /// `busy_view == Advanced` so a stale wizard doesn't hijack unrelated ops.
    ///
    /// When the Advanced view is busy but no specific sub-action labels
    /// itself, the default template "{operation} 중입니다." substitutes
    /// in `nav_advanced` ("고급") and reads awkwardly across all four
    /// locales — "고급 중입니다." / "Advanced is in progress." /
    /// "高级 正在进行中。" / "Дополнительно выполняется." — because
    /// the operation token is a section noun, not a verb phrase. The
    /// `busy_advanced_generic` key carries a per-locale full sentence
    /// for this fallback.
    fn busy_body_override(&self) -> Option<String> {
        if self.operation.view() == Some(View::KonaBess) {
            let key = if self.konabess.prepared.is_some() {
                "busy_konabess_cancel"
            } else {
                "busy_konabess_inspection"
            };
            return Some(self.t(key).to_string());
        }
        if self.operation.view() != Some(View::Advanced) {
            return None;
        }
        // Simple Flash is a full firmware flash, not a partition scan/write —
        // let it fall through to the default "{operation} in progress" template
        // (operation = its own label) instead of the partition-scan body.
        if self.advanced_wizard_open.is_open() && !self.advanced_wizard_open.is_simple_flash() {
            // Write Partitions' exec phase is a partition *write*; the loader-
            // upload + GPT scan preamble (and the other advanced flows) keep
            // the scan label.
            let key = if self.advanced_wizard_open.is_flash_parts() && self.flash_parts.is_in_exec()
            {
                "busy_partition_write"
            } else {
                "busy_partition_scan"
            };
            return Some(self.t(key).to_string());
        }
        if self.advanced_operation_label().is_none() {
            return Some(self.t("busy_advanced_generic").to_string());
        }
        None
    }

    /// Rebuild the editor from `log_lines` and auto-scroll to the
    /// bottom via `Motion::DocumentEnd`. Selection state resets.
    fn rebuild_log_editor(&mut self) {
        let joined = self.log_lines.join("\n");
        self.log_editor = iced::widget::text_editor::Content::with_text(&joined);
        use iced::widget::text_editor::{Action, Motion};
        self.log_editor.perform(Action::Move(Motion::DocumentEnd));
        self.log_dirty = false;
    }

    /// Picker shortcut: routes through the resolved Settings default loader when
    /// it exists and fits the connected model, else opens `loader_file_spec`.
    /// Dedupe across `*SelectLoader` handlers.
    fn pick_loader_with_default<F>(&mut self, on_chosen: F) -> Task<Message>
    where
        F: 'static + Send + Fn(Option<String>) -> Message,
    {
        if let Some(path) = self.resolved_default_loader() {
            return self.update(on_chosen(Some(path)));
        }
        pickers::pick_file_for(self.model_loader_file_spec(), &self.recent_paths, on_chosen)
    }

    /// Record the picked flash firmware folder and flag whether it ships an EDL
    /// loader (mirroring the worker's dir-then-parent lookup). When it does not,
    /// pre-fill a configured Settings default loader if it fits the model; the
    /// folder step otherwise requires the user to pick one before advancing.
    fn set_flash_firmware_folder(&mut self, path: String) {
        let was_after_folder = matches!(
            self.flash.current_step(),
            FlashStep::Bootloader | FlashStep::Confirm
        );
        self.flash.reset_firmware_identity();
        // Users frequently pick the extracted firmware ROOT instead of the
        // `image` folder LTBox flashes; retarget to a direct `image/` child
        // when one exists so the common mis-selection just works.
        let path = redirect_str(path);
        let dir = std::path::Path::new(&path);
        let has_loader = find_firmware_loader(dir).is_some();
        self.flash.loader_required = !has_loader;
        self.flash.loader_override = if self.flash.loader_required {
            self.resolved_default_loader()
        } else {
            None
        };
        self.flash.loader_error = None;
        self.flash.firmware_folder = Some(path.clone());
        self.flash.set_firmware_rollback_indices(&path);
        if was_after_folder || (self.flash.loader_required && self.flash.loader_override.is_none())
        {
            self.flash.set_step(FlashStep::Folder);
        }
    }

    /// Map cached PTSTPD `SaleArea` for the connected device → `DeviceRegion`.
    /// `"CN"` → PRC, JSON null → ROW, anything else → `None`. Cache-only.
    fn inferred_flash_region(&self) -> Option<DeviceRegion> {
        if self.device.serial.is_empty() {
            return None;
        }
        let info = self.queries.info_cache.get(&self.device.serial)?;
        region_from_salearea(info)
    }

    /// Prepare the Flash region step after `flash.reset()` without initiating
    /// a lookup. Demo scenes keep their deterministic state, while PRC-only
    /// models still skip the single-choice step.
    fn prepare_flash_region_on_entry(&mut self) -> Task<Message> {
        #[cfg(feature = "demo")]
        if demo::prepare_flash_region_on_entry(self) {
            return Task::none();
        }
        if self.is_prc_only() {
            self.flash.region_selection = Some(FlashRegionSelection::Manual(DeviceRegion::Prc));
            self.flash.device_region = Some(DeviceRegion::Prc);
            self.flash.step = 1;
        }
        Task::none()
    }

    /// Kick off region auto-detection after the automatic-selection option is
    /// chosen and Next is pressed:
    /// * PRC-only model → preselect PRC and jump to the target step.
    /// * usable polled serial → probe PTSTPD (region step shows an indicator).
    /// * no usable serial → open the manual-serial prompt.
    ///
    /// On a fetch failure / inconclusive SaleArea the handler falls back to the
    /// manual PRC/ROW cards, so this never blocks the wizard.
    fn begin_flash_region_auto(&mut self) -> Task<Message> {
        if self.is_prc_only() {
            // PRC-only SKU: no lookup needed; skip straight to the target step.
            self.flash.device_region = Some(DeviceRegion::Prc);
            self.flash.step = 1;
            return Task::none();
        }
        if self.has_pollable_serial() {
            let serial = self.device.serial.trim().to_string();
            return self.start_region_probe(serial);
        }
        // Serial comes only from an ADB/fastboot poll; ask for it manually.
        self.flash_serial_prompt = Some(String::new());
        Task::none()
    }

    /// Mint a fresh probe id, mark it pending (shows the progress indicator), and spawn
    /// the lookup. The id is the staleness token the result handler checks.
    fn start_region_probe(&mut self, serial: String) -> Task<Message> {
        let id = self.queries.start_region();
        self.spawn_auto_region_fetch(id, serial)
    }

    /// Whether the polled serial is usable for an automatic region lookup: the
    /// device is in an ADB/fastboot state (the only ones that yield a serial)
    /// and the serial has the expected `HA…` prefix (guards against a garbled
    /// read). When false, region detection falls back to the manual prompt.
    fn has_pollable_serial(&self) -> bool {
        matches!(
            self.device.connection,
            ConnectionStatus::Adb | ConnectionStatus::AdbRecovery | ConnectionStatus::Fastboot
        ) && self.device.serial.trim().starts_with("HA")
    }

    /// Off-thread PTSTPD fetch for auto region detection. Reuses the device
    /// -info cache when the serial is already known this session; otherwise
    /// fetches. Result arrives as `FlashMsg::FlashAutoRegionFetched`.
    fn spawn_auto_region_fetch(&self, id: u64, serial: String) -> Task<Message> {
        if let Some(info) = self.queries.info_cache.get(&serial).cloned() {
            return Task::done(Message::Flash(FlashMsg::FlashAutoRegionFetched(
                id,
                serial,
                Ok(info),
            )));
        }
        // The fallback (heavy-thread spawn failure / panic) carries the same id
        // + serial so the handler clears the progress indicator and falls back to manual
        // instead of treating it as stale.
        let serial_fb = serial.clone();
        task_heavy(
            move || {
                let r =
                    ltbox_core::lenovo_info::fetch_machine_info(&serial).map_err(|e| e.to_string());
                (id, serial, r)
            },
            |(id, s, r)| Message::Flash(FlashMsg::FlashAutoRegionFetched(id, s, r)),
            move |e| (id, serial_fb, Err(e)),
        )
    }

    /// Returns the Settings-level default EDL loader path when it is set
    /// **and** the file currently exists on disk. Used by every wizard
    /// open / reset path to decide whether to pre-fill its loader slot
    /// and skip past the loader step. Returns `None` when the default is
    /// unset or the file has been moved/deleted since it was saved (in
    /// which case the wizard falls back to the picker step as if no
    /// default had been configured — better than auto-advancing past a
    /// step with a missing file and surfacing the error later).
    fn resolved_default_loader(&self) -> Option<String> {
        let p = self.default_loader_path.as_deref()?;
        // Bypass the default when its extension doesn't fit the connected model
        // (TB323FU needs the .xml/.x manifest, others a .melf) so the wizard shows
        // its loader picker instead of auto-advancing with the wrong loader.
        if std::path::Path::new(p).is_file() && self.loader_fits_model(std::path::Path::new(p)) {
            Some(p.to_string())
        } else {
            None
        }
    }

    /// Apply the resolved default loader to whichever advanced-wizard
    /// loader-step is currently open. Pre-fills the wizard's `loader_path`
    /// and either advances directly to the Select step (DumpPhys /
    /// FlashPhys — no scan needed) or fires the GPT scan (FlashParts /
    /// DumpParts — Select step requires populated rows). Called from
    /// `AdvConfirm` after a wizard's `_open` flag flips.
    ///
    /// Returns `Task::none()` when the default loader is unset or the
    /// file is missing — the caller's existing flow then surfaces the
    /// loader step as before.
    fn apply_default_loader_to_advanced_wizard(&mut self) -> Task<Message> {
        let Some(path) = self.resolved_default_loader() else {
            // Default set but its extension doesn't fit the connected model
            // (resolved to None): surface the open wizard's loader picker with a
            // notice so it doesn't look like a bug. `loader_step_card` renders it.
            if self.default_loader_path.is_some() && !self.default_loader_fits_model() {
                let notice = ltbox_core::i18n::tr("loader_default_ext_unsupported");
                if self.advanced_wizard_open.is_flash_parts() {
                    self.flash_parts.scan_error = Some(notice);
                } else if self.advanced_wizard_open.is_dump_parts() {
                    self.dump_parts.scan_error = Some(notice);
                } else if self.advanced_wizard_open.is_dump_phys() {
                    self.dump_phys.loader_error = Some(notice);
                } else if self.advanced_wizard_open.is_flash_phys() {
                    self.flash_phys.loader_error = Some(notice);
                }
            }
            return Task::none();
        };
        if self.advanced_wizard_open.is_flash_parts() {
            // Leave step at 0 (Loader); FlashPartsScanDone advances to
            // Select on success, so jumping past step 0 here would
            // double-advance past Select.
            self.flash_parts.loader_path = Some(path);
            return self.update(Message::FlashParts(FlashPartsMsg::FlashPartsScanStart));
        } else if self.advanced_wizard_open.is_dump_parts() {
            self.dump_parts.loader_path = Some(path);
            return self.update(Message::DumpParts(DumpPartsMsg::DumpPartsScanStart));
        } else if self.advanced_wizard_open.is_dump_phys() {
            // Whole-LUN — no scan. Skip to Select directly.
            self.dump_phys.loader_path = Some(path);
            self.dump_phys.step = 1;
        } else if self.advanced_wizard_open.is_flash_phys() {
            self.flash_phys.loader_path = Some(path);
            self.flash_phys.step = 1;
        }
        Task::none()
    }

    /// Pre-fill the top-level KonaBess wizard from the configured default EDL
    /// loader, preserving the former Advanced-tile entry behavior.
    fn apply_default_loader_to_konabess(&mut self) {
        let Some(path) = self.resolved_default_loader() else {
            if self.default_loader_path.is_some() && !self.default_loader_fits_model() {
                self.konabess.loader_error =
                    Some(ltbox_core::i18n::tr("loader_default_ext_unsupported"));
            }
            return;
        };
        match self.resolve_loader_input(&path) {
            Ok(loader) if self.loader_fits_model(std::path::Path::new(&loader)) => {
                self.konabess.loader_path = Some(loader);
                self.konabess.loader_error = None;
                self.konabess.step = 0;
            }
            Ok(_) => {
                self.konabess.loader_error =
                    Some(self.t("loader_model_mismatch_tooltip").to_string());
            }
            Err(message) => self.konabess.loader_error = Some(message),
        }
    }

    /// Validate a picked/default EDL loader before device work starts.
    fn validate_loader_path(&mut self, path: &Option<String>) -> Result<String, ()> {
        let Some(p) = path.as_deref() else {
            self.error_msg = Some(self.t("err_loader_not_selected").to_string());
            return Err(());
        };
        let pb = std::path::Path::new(p);
        if !pb.is_file() {
            let msg = tr_args!("err_loader_missing", path = p);
            self.error_msg = Some(msg);
            return Err(());
        }
        // A `.x` loader (encrypted manifest) passes through as-is here;
        // `EdlSession::open` decrypts it to the sibling `.xml` at load time.
        Ok(p.to_string())
    }

    fn persist_settings(&self) {
        #[cfg(feature = "demo")]
        if demo::is_active(self) {
            return;
        }
        settings_store::save(&settings_store::PersistedSettings {
            language: self.settings.language.code().to_string(),
            theme: self.theme_choice.code().to_string(),
            theme_seed: self.theme_seed.code().to_string(),
            use_system_font: self.use_system_font,
            // Legacy field kept readable by older builds.
            dark_mode: self.dark_mode,
            recent_paths: self.recent_paths.clone(),
            default_loader_path: self.default_loader_path.clone(),
            qcom_driver_mode: self.qcom_driver_mode.code().to_string(),
            window_size: Some(self.window_size),
            qcom_driver_update_dismissed: self.qcom_driver_update_dismissed,
            dual_usb_advisory_dismissed_models: self.dual_usb_advisory_dismissed.clone(),
        });
    }

    /// Label key for the live connection. Identical to
    /// [`ConnectionStatus::label_key`] except that a Fastboot endpoint
    /// reporting `is-userspace` is named fastbootd — the two are the same
    /// connection everywhere else, so only the label distinguishes them.
    fn connection_label_key(&self) -> &'static str {
        if self.device.connection == ConnectionStatus::Fastboot && self.device.fastboot_userspace {
            "conn_fastbootd"
        } else {
            self.device.connection.label_key()
        }
    }

    /// The connected dual-USB-C model whose port guide is currently eligible
    /// to open, or `None`. Eligible when the model has the dual-USB capability
    /// and the user has neither permanently dismissed ("don't show again")
    /// nor session-closed ("close") it.
    fn dual_usb_advisory_model(&self) -> Option<&str> {
        let m = self.device.model.as_str();
        let hidden = |list: &[String]| list.iter().any(|x| x.eq_ignore_ascii_case(m));
        if !m.is_empty()
            && is_dual_usbc_model(m)
            && !hidden(&self.dual_usb_advisory_dismissed)
            && !hidden(&self.dual_usb_advisory_closed)
        {
            Some(m)
        } else {
            None
        }
    }

    /// Record `path` in the MRU list for `kind`. Persists on change so
    /// the list survives restarts (write is cheap — small JSON, and only
    /// triggers when the list actually moves).
    fn remember_recent(&mut self, kind: pickers::PickerKind, path: &str) {
        if self.recent_paths.push(kind.storage_key(), path) {
            self.persist_settings();
        }
    }

    /// Resolve loader input from the unified picker path.
    ///
    /// Preferred path is a file (`*.melf`, `*.mbn`, `*.elf`) and accepts
    /// any filename with one of those extensions. A directory is still
    /// accepted for backwards compatibility with older recents entries
    /// and is resolved via [`find_edl_loader`].
    /// Error to surface for a finished Firehose GPT scan: the worker's own
    /// failure, or a scan that came back with no partitions at all. `None`
    /// means the table is usable and the wizard may advance. Both partition
    /// wizards run the same scan, but only one of them used to notice the
    /// empty case.
    fn parts_scan_outcome(&self, error: Option<String>, rows_empty: bool) -> Option<String> {
        error.or_else(|| rows_empty.then(|| self.t("err_parts_scan_empty").to_string()))
    }

    /// Route a picked loader into one wizard's `(loader_path, loader_error)`
    /// pair. Cancelling changes nothing; a resolved pick clears the error; a
    /// rejected pick clears the path, so a wizard can never advance on the
    /// loader it just refused.
    fn apply_loader_pick<F>(&mut self, picked: Option<String>, set: F)
    where
        F: FnOnce(&mut Self, Option<String>, Option<String>),
    {
        let Some(p) = picked else { return };
        match self.resolve_loader_input(&p) {
            Ok(loader) => set(self, Some(loader), None),
            Err(msg) => set(self, None, Some(msg)),
        }
    }

    fn resolve_loader_input(&mut self, selected_path: &str) -> std::result::Result<String, String> {
        let path = std::path::Path::new(selected_path);
        if path.is_file() {
            self.remember_recent(pickers::PickerKind::File, selected_path);
            // TB323FU model gate: if the user picked a `.melf` but the
            // device is a TB323FU, upgrade to the
            // `qsahara_device_programmer.xml` manifest sitting in the
            // same folder. If the manifest is missing the .melf
            // alone is wrong and would fail mid-Sahara — abort up
            // front. Performed during resolve so the wizard's Confirm
            // step shows the correct path.
            if self.requires_sahara_manifest()
                && is_melf_loader(path)
                && let Some(parent) = path.parent()
            {
                if let Some(manifest) = resolve_sahara_manifest(parent) {
                    return Ok(manifest.to_string_lossy().to_string());
                }
                return Err(tr_args!(
                    "err_efisp_loader_manifest_required",
                    model = self.device.model.as_str(),
                    path = path.display()
                ));
            }
            // Encrypted multi-image manifest picked directly
            // (`qsahara_device_programmer.x`) passes through as-is;
            // `EdlSession::open` decrypts it to the sibling `.xml`.
            if ltbox_core::sahara_xml::is_encrypted_manifest_filename(path) {
                return Ok(selected_path.to_string());
            }
            if is_loader_file(path) {
                return Ok(selected_path.to_string());
            }
            return Err(tr_args!(
                "err_unsupported_loader_file",
                path = selected_path
            ));
        }

        if path.is_dir() {
            self.remember_recent(pickers::PickerKind::LoaderFolder, selected_path);
            return find_edl_loader(path)
                .map(|p| p.to_string_lossy().to_string())
                .ok_or_else(|| tr_args!("err_loader_not_found_in_path", path = selected_path));
        }

        Err(tr_args!("err_path_missing", path = selected_path))
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subs = vec![
            iced::time::every(std::time::Duration::from_secs(3)).map(|_| Message::PollDevice),
            // 500 ms drain — 4 Hz drove some GPU drivers into TDR
            // during long qdl flashes.
            iced::time::every(std::time::Duration::from_millis(500))
                .map(|_| Message::DrainStdoutTap),
        ];
        #[cfg(windows)]
        subs.push(
            iced::time::every(std::time::Duration::from_secs(2)).map(|_| Message::PollSoftwareFix),
        );
        // Compact hover drives the sidebar width spring. Expanded uses a
        // fixed drawer, but a single tick still clears compact hover state
        // inherited across a resize before the hidden spring settles closed.
        // Once both state and spring are settled, no 16 ms subscription runs.
        let sidebar_target = self.sidebar_anim_target();
        let sidebar_hover_settled =
            self.window_size_class() == WindowSizeClass::Compact || !self.sidebar_expanded;
        let sidebar_settled = (self.sidebar_anim - sidebar_target).abs() < 0.001
            && self.sidebar_velocity.abs() < 0.05
            && (self.sidebar_label_alpha - f32::from(self.sidebar_expanded)).abs() < 0.001
            && self.sidebar_label_velocity.abs() < 0.05
            && sidebar_hover_settled;
        if !sidebar_settled {
            subs.push(
                iced::time::every(std::time::Duration::from_millis(16))
                    .map(|_| Message::SidebarAnimTick),
            );
        }
        // Listen for window resize events so the user's preferred
        // geometry survives a restart. `event::listen_with` filters at
        // the source so non-window events don't bubble back as
        // `Message::Noop`.
        subs.push(iced::event::listen_with(
            |event, status, window_id| match event {
                iced::Event::Window(iced::window::Event::Opened { .. }) => Some(Message::Window(
                    WindowMsg::WindowIdReceived(Some(window_id)),
                )),
                iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                    key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Tab),
                    modifiers,
                    ..
                }) if status == iced::event::Status::Ignored => {
                    Some(Message::FocusMove(modifiers.shift()))
                }
                iced::Event::Window(iced::window::Event::Resized(size)) => {
                    Some(Message::WindowResized(size.width, size.height))
                }
                iced::Event::Window(iced::window::Event::CloseRequested) => {
                    Some(Message::Window(WindowMsg::WindowClose))
                }
                _ => None,
            },
        ));
        // Debounced window-size persistence tick: only fires while a
        // pending size update hasn't been flushed yet.
        if self.window_size_dirty {
            subs.push(
                iced::time::every(WINDOW_SIZE_SAVE_INTERVAL).map(|_| Message::PersistWindowSize),
            );
        }
        if self.theme_choice == ThemeChoice::System {
            subs.push(
                iced::time::every(std::time::Duration::from_secs(2))
                    .map(|_| Message::RefreshSystemTheme),
            );
        }
        Subscription::batch(subs)
    }

    /// Shared error-state body. Renders the localized header
    /// (`error_key`), the raw upstream error text, and a Retry pill
    /// that fires `retry_msg`. Same shape as the loading view —
    /// pulled out of the device-info / OTA popups which had two
    /// near-identical copies.
    fn popup_error_view(
        &self,
        error_key: &str,
        e: &str,
        retry_msg: Message,
    ) -> Element<'_, Message> {
        column![
            text(self.t(error_key).to_string())
                .size(theme::text_size::BODY_MEDIUM)
                .style(|t: &Theme| iced::widget::text::Style {
                    color: Some(pal_of(t).error),
                }),
            text(e.to_string()).size(11).style(muted_style),
            Space::new().height(8),
            m3_filled_button(self.t("btn_retry").to_string()).on_press(retry_msg),
        ]
        .spacing(8)
        .into()
    }

    /// Sidebar tween target for the compact hover overlay.
    ///
    /// Expanded has a separate fixed-width rendering path, so its compact
    /// animation state settles closed and cannot react to pointer proximity.
    fn sidebar_anim_target(&self) -> f32 {
        if self.window_size_class() == WindowSizeClass::Compact && self.sidebar_expanded {
            1.0
        } else {
            0.0
        }
    }

    /// Visual openness used by drawer labels.
    /// Expanded labels are always fully visible; Compact follows the spring.
    fn sidebar_visual_progress(&self) -> f32 {
        match self.window_size_class() {
            WindowSizeClass::Compact => self.sidebar_label_alpha,
            WindowSizeClass::Expanded => 1.0,
        }
    }

    /// Whether the Dashboard's rollback cell opens the floor breakdown.
    ///
    /// Both halves matter. Floors are only ever populated by a
    /// bootloader-mode poll, so their presence *is* the "in bootloader"
    /// test. The model check keeps an exempt SKU (TB322FC) from offering
    /// a breakdown behind a cell that reads "No" — the two would
    /// contradict each other even if its bootloader did report two
    /// populated locations.
    #[cfg(test)]
    pub(crate) fn rollback_detail_available(&self) -> bool {
        self.device.rollback_floors.is_some() && is_rollback_protected_model(&self.device.model)
    }

    fn is_nav_enabled(&self, view: View) -> bool {
        // About is informational and device/platform-independent — always on.
        if matches!(view, View::About) {
            return true;
        }
        if self.device.platform_supported == Some(false) {
            return matches!(view, View::Dashboard | View::SystemUpdate | View::Settings);
        }
        true
    }

    /// The same model policy used by the workers and patch pipeline.
    fn model_capabilities(&self) -> &'static ltbox_core::model::ModelCapabilities {
        ltbox_core::model::capabilities(&self.device.model)
    }

    /// Firmware GBL policy follows the target; read-only rollback applies
    /// whenever either the device or target requires it. Before the folder is
    /// known, show the connected device's supported choices.
    fn flash_rollback_policy(&self) -> ltbox_core::model::RollbackPolicy {
        use ltbox_core::model::{RollbackPolicy, fingerprint_capabilities};
        let device = self.model_capabilities().rollback;
        let target = self
            .flash
            .firmware_identity
            .as_ref()
            .and_then(|identity| identity.fingerprint.as_deref())
            .and_then(|fp| {
                fingerprint_capabilities(fp)
                    .map(|profile| profile.rollback)
                    .reduce(|a, b| {
                        if a == RollbackPolicy::ReadOnly || b == RollbackPolicy::ReadOnly {
                            RollbackPolicy::ReadOnly
                        } else if a == RollbackPolicy::Gbl || b == RollbackPolicy::Gbl {
                            RollbackPolicy::Gbl
                        } else {
                            a
                        }
                    })
            });
        if device == RollbackPolicy::ReadOnly || target == Some(RollbackPolicy::ReadOnly) {
            RollbackPolicy::ReadOnly
        } else {
            target.unwrap_or(device)
        }
    }

    /// Whether the polled device is a TB323FU. Drives the multi-image
    /// EDL loader path: TB323FU doesn't accept a single
    /// `xbl_s_devprg_ns.melf`; it needs the full
    /// `qsahara_device_programmer.xml` manifest + the per-id ELF /
    /// MBN payloads it references. The loader resolver upgrades a
    /// stray `.melf` selection to the manifest when one exists in
    /// the same folder; if not, it aborts up front rather than
    /// failing mid-Sahara.
    fn requires_sahara_manifest(&self) -> bool {
        self.model_capabilities().requires_sahara_manifest
    }

    fn is_xiaoxin_pro13(&self) -> bool {
        ltbox_core::model::is_xiaoxin_pro13_model(&self.device.model)
    }

    /// True when `path`'s extension is the EDL loader form the connected device
    /// needs: manifest-route models load `.xml` / `.x`; other connected models
    /// load `.melf`. With no connected device, either loader form is valid.
    /// Inspects only the picked file's own extension, never the `.mbn` / `.elf`
    /// images a manifest references internally.
    fn loader_fits_model(&self, path: &std::path::Path) -> bool {
        if self.device.connection == ConnectionStatus::None {
            return loader_ext_fits_model(false, path) || loader_ext_fits_model(true, path);
        }
        loader_ext_fits_model(self.model_capabilities().requires_sahara_manifest, path)
    }

    /// True when the Settings default EDL loader is unset, or its extension fits
    /// the connected model (see [`Self::loader_fits_model`]). When false the
    /// default is bypassed so the wizard's loader picker is shown instead.
    fn default_loader_fits_model(&self) -> bool {
        match self.default_loader_path.as_deref() {
            None => true,
            Some(p) => self.loader_fits_model(std::path::Path::new(p)),
        }
    }

    /// Which images the unroot folder picker should name.
    ///
    /// What a root backup holds is decided by the root run, not by the wizard
    /// card the user picks here: the target follows the route and the model,
    /// and vbmeta is only in the folder when the run had to rebuild it. Asking
    /// the same two functions the root run asked keeps the picker from naming a
    /// file that run never wrote — a chained `boot` target leaves no vbmeta.img
    /// at all, and only the TB320FC family roots to `boot` outside the GKI
    /// route.
    fn unroot_folder_desc(&self, unroot_type: UnrootType) -> &str {
        let target = ltbox_patch::root_pipeline::resolve_root_image_target(
            ltbox_patch::root_pipeline::RootFamily::Magisk,
            matches!(unroot_type, UnrootType::APatchGki),
            &self.device.model,
        );
        // The testkey efisp/GBL route leaves vbmeta out of the backup even for
        // a target vbmeta would normally hash, so it overrides the rebuild rule.
        let with_vbmeta = !root_skips_avb_postprocess(&self.device.model)
            && ltbox_patch::root_pipeline::root_run_rebuilds_vbmeta(target, &self.device.model);
        match (target, with_vbmeta) {
            (ltbox_patch::root_pipeline::RootImageTarget::Boot, true) => {
                self.t("unroot_folderdesc_boot_vbmeta")
            }
            (ltbox_patch::root_pipeline::RootImageTarget::Boot, false) => {
                self.t("unroot_folderdesc_boot")
            }
            (ltbox_patch::root_pipeline::RootImageTarget::InitBoot, true) => {
                self.t("unroot_folderdesc_init_boot_vbmeta")
            }
            (ltbox_patch::root_pipeline::RootImageTarget::InitBoot, false) => {
                self.t("unroot_folderdesc_init_boot")
            }
        }
    }

    fn loader_picker_exts(&self) -> &'static [&'static str] {
        loader::loader_picker_extensions(
            self.device.connection != ConnectionStatus::None && !self.device.model.is_empty(),
            self.device.connection != ConnectionStatus::None && self.requires_sahara_manifest(),
        )
    }

    fn model_loader_file_spec(&self) -> pickers::FilePickSpec {
        let exts = self.loader_picker_exts();
        pickers::FilePickSpec::single()
            .with_filter(format!("EDL loader ({})", exts.join(" / ")), exts)
    }

    fn advanced_picker_exts(&self) -> (&'static str, &'static [&'static str]) {
        if matches!(
            self.adv_wizard.action,
            Some(AdvAction::DetectArb | AdvAction::PatchDevinfo)
        ) {
            ("EDL loader", self.loader_picker_exts())
        } else {
            self.adv_wizard.accepted_exts()
        }
    }

    /// Whether the polled device is a TB322FC. PRC-only SKU — the Flash
    /// wizard hides ROW + OtherRegion as disabled cards so the user
    /// cannot pick a region or cross-region flash target that the
    /// hardware doesn't ship with.
    fn is_prc_only(&self) -> bool {
        self.model_capabilities().prc_only
    }

    /// True when the dashboard poll has placed the device in a mode
    /// any wizard can transition out of (`ensure_*` helpers + the
    /// flash/sysupdate bridges). Used to gate every wizard's final
    /// "Start" button — `None` and `AdbUnauthorized` mean we can't
    /// even start the operation, so spawning a worker that would
    /// immediately bail with "no device" is just noise.
    fn device_reachable(&self) -> bool {
        matches!(
            self.device.connection,
            ConnectionStatus::Adb
                | ConnectionStatus::AdbRecovery
                | ConnectionStatus::Fastboot
                | ConnectionStatus::Edl
        )
    }

    /// Re-populate `ota_changelog_editor` from the current popup
    /// state. Picks `desc_cn` for the Chinese GUI locale (with
    /// `desc_en` fallback when `desc_cn` is empty), `desc_en`
    /// otherwise. Called from both `OtaOpen` (cache restore) and
    /// `OtaFetched` (fresh fetch) so the editor's contents stay in
    /// lockstep with whatever the popup is about to render.
    fn seed_ota_changelog_editor(&mut self, state: &OtaPopupState) {
        let editor_text = if let OtaPopupState::Ready(u) = state {
            let prefer_cn = matches!(self.settings.language, Language::Zh);
            let raw = if prefer_cn && !u.desc_cn.trim().is_empty() {
                &u.desc_cn
            } else if !u.desc_en.trim().is_empty() {
                &u.desc_en
            } else {
                &u.desc_cn
            };
            ltbox_core::lenovo_ota::format_changelog(raw)
        } else {
            String::new()
        };
        self.ota_changelog_editor = iced::widget::text_editor::Content::with_text(&editor_text);
    }

    /// Build the off-thread QFIL-fetch task for `serial`. Reuses the
    /// device-info cache for MTM + SaleArea when the device-info popup already
    /// fetched them this session; otherwise the worker fetches machine info
    /// itself. Result arrives as `Message::QfilFetched`.
    fn spawn_qfil_fetch(&mut self, serial: String) -> Task<Message> {
        use ltbox_core::lenovo_info::FieldValue;
        let cached: Option<(String, String)> = self.queries.info_cache.get(&serial).map(|info| {
            let field = |k: &str| match info.field(k) {
                FieldValue::Value(s) => s,
                _ => String::new(),
            };
            (field("MTM"), field("SaleArea"))
        });
        let serial_for_task = serial;
        let task = task_heavy(
            move || {
                let outcome = resolve_qfil(&serial_for_task, cached);
                (serial_for_task, outcome)
            },
            |(s, r)| Message::QfilFetched(s, r),
            |e| (String::new(), Err(e)),
        );
        self.queries.track_lookup(LookupKind::Qfil, task)
    }

    /// Compact update action attached to the version at the status bar's
    /// trailing edge. It deliberately has no sidebar-animation dependency.
    fn status_update_available_button(&self) -> Element<'_, Message> {
        let content = row![
            icon::tile_update_on()
                .size(12)
                .line_height(1.0)
                .style(|t: &Theme| iced::widget::text::Style {
                    color: Some(pal_of(t).on_tertiary),
                }),
            text(self.t("status_update_available").to_string())
                .size(theme::text_size::BODY_SMALL)
                .line_height(1.0)
                .wrapping(iced::widget::text::Wrapping::None),
        ]
        .spacing(4)
        .align_y(iced::Alignment::Center);
        button(content)
            .on_press(Message::OpenUpdate)
            .padding([6, 8])
            .style(|t: &Theme, status| {
                let p = pal_of(t);
                button::Style {
                    background: Some(
                        theme::mix_color(p.tertiary, p.on_tertiary, theme::state_alpha(status))
                            .into(),
                    ),
                    text_color: p.on_tertiary,
                    border: iced::Border {
                        radius: theme::shape::FULL.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }
            })
            .into()
    }

    /// Per-extension recents strip for file pickers.
    fn recent_file_chips<F>(
        &self,
        accepted_exts: &[&str],
        on_pick: F,
        label_key: &str,
    ) -> Element<'_, Message>
    where
        F: Fn(String) -> Message,
    {
        let all = self
            .recent_paths
            .recent(pickers::PickerKind::File.storage_key());
        let filtered: Vec<String> = if accepted_exts.is_empty() {
            all.to_vec()
        } else {
            all.iter()
                .filter(|p| pickers::path_matches_extensions(p, accepted_exts))
                .cloned()
                .collect()
        };
        self.recent_chips(&filtered, on_pick, label_key, true)
    }

    /// Empty column when the list is empty so call sites can splice
    /// it in unconditionally.
    fn recent_chips<F>(
        &self,
        items: &[String],
        on_pick: F,
        label_key: &str,
        is_file_picker: bool,
    ) -> Element<'_, Message>
    where
        F: Fn(String) -> Message,
    {
        self.picker_recent_list(items, on_pick, label_key, is_file_picker)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device_poll(model: &str) -> DevicePollResult {
        DevicePollResult {
            status: ConnectionStatus::Adb,
            model: model.to_string(),
            market_name: "Legion Y700".to_string(),
            ..DevicePollResult::default()
        }
    }

    #[test]
    fn package_upgrade_commands_cover_every_install_source() {
        use ltbox_core::install_source::InstallSource;

        for (source, expected) in [
            (InstallSource::Scoop, Some("scoop update ltbox")),
            (
                InstallSource::WinGet,
                Some("winget upgrade miner7222.LTBox"),
            ),
            (InstallSource::Homebrew, Some("brew upgrade --cask ltbox")),
            (
                InstallSource::Deb,
                Some("sudo apt update && sudo apt upgrade ltbox"),
            ),
            (InstallSource::Rpm, Some("sudo dnf upgrade ltbox")),
            (InstallSource::OtherPackageManager, None),
            (InstallSource::Direct, None),
        ] {
            let actual = package_upgrade_command(source);
            assert_eq!(actual.command, expected.unwrap_or_default());
            assert_eq!(actual.available, expected.is_some());
        }
    }

    #[test]
    fn primary_phase_plans_use_refined_counts() {
        assert_eq!(OperationPhaseKind::Flash.keys().len(), 9);
        assert_eq!(OperationPhaseKind::Root.keys().len(), 8);
        assert_eq!(OperationPhaseKind::Unroot.keys().len(), 6);
    }

    #[test]
    fn system_update_phase_plans_match_each_action() {
        assert_eq!(OperationPhaseKind::SysUpdateDisable.keys().len(), 3);
        assert_eq!(OperationPhaseKind::SysUpdateEnable.keys().len(), 3);
        assert_eq!(OperationPhaseKind::BootRecovery.keys().len(), 7);
    }

    #[test]
    fn advanced_edl_phase_plans_match_each_action() {
        assert_eq!(OperationPhaseKind::ChangeCountry.keys().len(), 5);
        assert_eq!(OperationPhaseKind::DetectArb.keys().len(), 5);
        assert_eq!(OperationPhaseKind::SimpleFlash.keys().len(), 5);
        assert_eq!(OperationPhaseKind::FlashPartitions.keys().len(), 3);
        assert_eq!(OperationPhaseKind::DumpPartitions.keys().len(), 4);
        assert_eq!(OperationPhaseKind::FlashPhysical.keys().len(), 4);
        assert_eq!(OperationPhaseKind::DumpPhysical.keys().len(), 5);
        assert_eq!(OperationPhaseKind::KonaBess.keys().len(), 7);
    }

    #[test]
    fn offline_advanced_phase_plans_match_each_action() {
        assert_eq!(OperationPhaseKind::OfflineConvertXml.keys().len(), 3);
        assert_eq!(OperationPhaseKind::RegionConversion.keys().len(), 4);
        assert_eq!(OperationPhaseKind::PatchArb.keys().len(), 4);
        assert_eq!(OperationPhaseKind::RebuildVbmeta.keys().len(), 3);
        assert_eq!(
            OperationPhaseKind::for_advanced_file(AdvAction::ConvertXml),
            Some(OperationPhaseKind::OfflineConvertXml)
        );
        assert_eq!(
            OperationPhaseKind::for_advanced_file(AdvAction::RegionConvert),
            Some(OperationPhaseKind::RegionConversion)
        );
        assert_eq!(
            OperationPhaseKind::for_advanced_file(AdvAction::PatchArb),
            Some(OperationPhaseKind::PatchArb)
        );
        assert_eq!(
            OperationPhaseKind::for_advanced_file(AdvAction::RebuildVbmeta),
            Some(OperationPhaseKind::RebuildVbmeta)
        );
        assert_eq!(
            OperationPhaseKind::for_advanced_file(AdvAction::ImageInfo),
            None
        );
    }

    #[test]
    fn operation_phase_reporter_marker_uses_the_same_snapshot_total_and_label() {
        install_core_translator(Language::En);
        let reporter =
            PhaseReporter::from_labels(vec!["Prepare".into(), "Write".into(), "Reboot".into()]);
        let marker = reporter.marker(2);
        assert!(marker.contains("2/3"));
        assert!(marker.contains("Write"));
        assert_eq!(reporter.steps()[1].label, "Write");
    }

    #[test]
    fn operation_phase_every_plan_has_unique_nonempty_keys() {
        for kind in OperationPhaseKind::all() {
            let keys = kind.keys();
            assert!(!keys.is_empty());
            let unique = keys
                .iter()
                .copied()
                .collect::<std::collections::HashSet<_>>();
            assert_eq!(unique.len(), keys.len(), "duplicate key in {kind:?}");
        }
    }

    #[test]
    fn all_locale_tables_load_through_translations() {
        for language in [
            Language::En,
            Language::Ko,
            Language::Zh,
            Language::Ru,
            Language::Ja,
        ] {
            assert!(!Translations::load(language).primary.is_empty());
        }
    }

    #[test]
    fn about_license_messages_open_and_close_the_dialog() {
        let mut app = App::default();
        assert!(!app.about_licenses_open);

        let _ = app.update(Message::AboutLicensesOpen);
        assert!(app.about_licenses_open);

        let _ = app.update(Message::AboutLicensesClose);
        assert!(!app.about_licenses_open);
    }

    #[test]
    fn an_empty_partition_scan_is_an_error_in_both_wizards() {
        // Read Partitions reported it; Flash Partitions silently sat on the
        // loader step with an empty table and no explanation.
        let app = App::default();
        assert!(app.parts_scan_outcome(None, true).is_some());
        assert!(app.parts_scan_outcome(None, false).is_none());
        // A worker failure outranks the empty check.
        assert_eq!(
            app.parts_scan_outcome(Some("boom".to_string()), true)
                .as_deref(),
            Some("boom")
        );
    }

    #[test]
    fn a_refused_loader_pick_clears_the_path_and_blocks_next() {
        // Previously each wizard hand-rolled this: most left the old
        // loader_path in place on a bad pick, and only KonaBess gated Next on
        // the error, so the rest advanced on a loader they had just refused.
        let dir = tempfile::tempdir().unwrap();
        let good = dir.path().join("xbl_s_devprg_ns.melf");
        std::fs::write(&good, b"loader").unwrap();
        let bad = dir.path().join("not-a-loader.txt");
        std::fs::write(&bad, b"nope").unwrap();

        let mut app = App::default();
        let _ = app.update_dump_phys(DumpPhysMsg::DumpPhysLoaderChosen(Some(
            good.to_string_lossy().to_string(),
        )));
        assert!(app.dump_phys.loader_path.is_some());
        assert!(app.dump_phys.loader_error.is_none());
        assert!(app.dump_phys.can_next());

        let _ = app.update_dump_phys(DumpPhysMsg::DumpPhysLoaderChosen(Some(
            bad.to_string_lossy().to_string(),
        )));
        assert!(app.dump_phys.loader_path.is_none());
        assert!(app.dump_phys.loader_error.is_some());
        assert!(!app.dump_phys.can_next());

        // Cancelling the picker leaves the step exactly as it was.
        let before = app.dump_phys.loader_error.clone();
        let _ = app.update_dump_phys(DumpPhysMsg::DumpPhysLoaderChosen(None));
        assert_eq!(app.dump_phys.loader_error, before);
    }

    #[test]
    fn a_loader_pick_no_longer_overwrites_why_the_scan_failed() {
        // Both partition wizards used to funnel loader errors into
        // `scan_error`, so re-picking a loader erased the scan failure.
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("not-a-loader.txt");
        std::fs::write(&bad, b"nope").unwrap();

        let mut app = App::default();
        app.dump_parts.scan_error = Some("scan blew up".to_string());
        let _ = app.update_dump_parts(DumpPartsMsg::DumpPartsLoaderChosen(Some(
            bad.to_string_lossy().to_string(),
        )));
        assert_eq!(app.dump_parts.scan_error.as_deref(), Some("scan blew up"));
        assert!(app.dump_parts.loader_error.is_some());
    }

    #[test]
    fn root_and_unroot_loader_steps_upgrade_a_tb323fu_melf_to_the_manifest() {
        // Every other wizard resolved the pick through `resolve_loader_input`;
        // these two assigned the raw path, so a TB323FU `.melf` reached the
        // Sahara handshake without the manifest it needs.
        let dir = tempfile::tempdir().unwrap();
        let melf = dir.path().join("xbl_s_devprg_ns.melf");
        std::fs::write(&melf, b"loader").unwrap();
        let manifest = dir.path().join(ltbox_core::sahara_xml::MANIFEST_FILENAME);
        std::fs::write(&manifest, b"<data/>").unwrap();
        let picked = melf.to_string_lossy().to_string();
        let want = manifest.to_string_lossy().to_string();

        let mut root_app = App {
            device: DeviceSnapshot {
                model: "TB323FU".to_string(),
                ..Default::default()
            },
            ..App::default()
        };
        let _ = root_app.update(Message::Root(RootMsg::RootLoaderChosen(Some(
            picked.clone(),
        ))));
        assert_eq!(root_app.root.folder_path.as_deref(), Some(want.as_str()));
        assert!(root_app.error_msg.is_none());

        let mut unroot_app = App {
            device: DeviceSnapshot {
                model: "TB323FU".to_string(),
                ..Default::default()
            },
            ..App::default()
        };
        let _ = unroot_app.update(Message::Unroot(UnrootMsg::UnrootLoaderChosen(Some(
            picked.clone(),
        ))));
        assert_eq!(
            unroot_app.unroot.loader_path.as_deref(),
            Some(want.as_str())
        );

        // A non-TB323FU keeps the .melf it was given.
        let mut other = App {
            device: DeviceSnapshot {
                model: "TB320FC".to_string(),
                ..Default::default()
            },
            ..App::default()
        };
        let _ = other.update(Message::Root(RootMsg::RootLoaderChosen(Some(
            picked.clone(),
        ))));
        assert_eq!(other.root.folder_path.as_deref(), Some(picked.as_str()));

        // Both steps still record the pick in the shared file recents.
        assert!(
            root_app
                .recent_paths
                .recent(pickers::PickerKind::File.storage_key())
                .contains(&picked)
        );
    }

    #[test]
    fn fastbootd_is_labelled_apart_from_the_bootloader() {
        let mut app = App {
            device: DeviceSnapshot {
                connection: ConnectionStatus::Fastboot,
                ..Default::default()
            },
            ..App::default()
        };
        assert_eq!(app.connection_label_key(), "conn_fastboot");

        app.device.fastboot_userspace = true;
        assert_eq!(app.connection_label_key(), "conn_fastbootd");

        // The flag only ever qualifies a Fastboot connection.
        app.device.connection = ConnectionStatus::Adb;
        assert_eq!(app.connection_label_key(), "conn_adb");
    }

    #[test]
    fn fastbootd_is_reachable_from_every_transport_that_can_ask_for_it() {
        // ADB routes it through the `reboot:` service, so it works even in
        // sideload, where there is no shell. The bootloader sends
        // `reboot-fastboot`. EDL resets only to system or back to EDL.
        assert!(RebootTarget::Fastbootd.available_from(ConnectionStatus::Adb));
        assert!(RebootTarget::Fastbootd.available_from(ConnectionStatus::Fastboot));
        assert!(RebootTarget::Fastbootd.available_from(ConnectionStatus::AdbSideload));
        assert!(!RebootTarget::Fastbootd.available_from(ConnectionStatus::Edl));
        assert!(!RebootTarget::Fastbootd.available_from(ConnectionStatus::AdbUnauthorized));
    }

    #[test]
    fn reboot_targets_recognize_the_current_transport_state() {
        assert!(RebootTarget::System.is_current_from(ConnectionStatus::Adb, false));
        assert!(RebootTarget::Recovery.is_current_from(ConnectionStatus::AdbRecovery, false));
        assert!(RebootTarget::Edl.is_current_from(ConnectionStatus::Edl, false));
        assert!(RebootTarget::Bootloader.is_current_from(ConnectionStatus::Fastboot, false));
        assert!(RebootTarget::Fastbootd.is_current_from(ConnectionStatus::Fastboot, true));
        assert!(!RebootTarget::Fastbootd.is_current_from(ConnectionStatus::Fastboot, false));
    }

    #[test]
    fn closing_the_edl_wait_dialog_keeps_the_reboot_operation_running() {
        let mut app = App::default();
        app.device.connection = ConnectionStatus::Adb;

        drop(app.update(Message::Reboot(RebootMsg::RebootTo(RebootTarget::Edl))));
        let operation_id = app.operation.id();
        assert!(app.operation.is_running());
        assert_eq!(app.operation.view(), Some(View::Reboot));
        assert!(app.reboot_wait_transition);
        assert!(app.reboot_wait_dialog_open);
        assert!(!app.should_show_busy_progress_dialog());

        drop(app.update(Message::RebootWaitDismiss));

        assert!(app.operation.is_running());
        assert_eq!(app.operation.id(), operation_id);
        assert!(app.reboot_wait_transition);
        assert!(!app.reboot_wait_dialog_open);
        assert!(!app.should_show_busy_progress_dialog());
    }

    #[test]
    fn edl_wait_state_clears_when_the_reboot_operation_completes() {
        let mut app = App::default();
        app.device.connection = ConnectionStatus::Adb;
        drop(app.update(Message::Reboot(RebootMsg::RebootTo(RebootTarget::Edl))));
        let operation_id = app.operation.id().expect("reboot operation id");

        drop(app.update(Message::OperationEvent(
            operation_id,
            Box::new(Message::Reboot(RebootMsg::RebootDone(Vec::new()))),
        )));

        assert!(!app.operation.is_running());
        assert!(!app.reboot_wait_transition);
        assert!(!app.reboot_wait_dialog_open);
    }

    #[test]
    fn dual_usb_guide_auto_opens_on_first_eligible_poll() {
        let mut app = App {
            startup_disclaimer_open: false,
            dual_usb_advisory_dismissed: Vec::new(),
            dual_usb_advisory_closed: Vec::new(),
            ..App::default()
        };
        assert!(!app.dual_usb_help_open);
        assert!(app.dual_usb_help_model.is_empty());

        let _ = app.update(Message::DevicePolled(device_poll("TB323FU")));
        assert!(app.dual_usb_help_open);
        assert_eq!(app.dual_usb_help_model, "TB323FU");
        assert_eq!(app.dual_usb_help_name, "Legion Y700");
    }

    #[test]
    fn dual_usb_guide_waits_for_startup_disclaimer_to_close() {
        let mut app = App {
            startup_disclaimer_open: true,
            startup_disclaimer_checked: false,
            dual_usb_advisory_dismissed: Vec::new(),
            dual_usb_advisory_closed: Vec::new(),
            ..App::default()
        };

        let _ = app.update(Message::DevicePolled(device_poll("TB323FU")));
        assert!(!app.dual_usb_help_open);
        assert!(app.dual_usb_help_model.is_empty());

        let _ = app.update(Message::StartupDisclaimerToggled(true));
        let _ = app.update(Message::StartupDisclaimerConfirm);
        assert!(!app.startup_disclaimer_open);
        assert!(app.dual_usb_help_open);
        assert_eq!(app.dual_usb_help_model, "TB323FU");
    }

    #[test]
    fn dual_usb_guide_stays_open_across_unplug_and_same_model_replug() {
        let mut app = App {
            startup_disclaimer_open: false,
            dual_usb_advisory_dismissed: Vec::new(),
            dual_usb_advisory_closed: Vec::new(),
            ..App::default()
        };

        let _ = app.update(Message::DevicePolled(device_poll("TB323FU")));
        assert!(app.dual_usb_help_open);
        assert_eq!(app.dual_usb_help_model, "TB323FU");

        let _ = app.update(Message::DevicePolled(DevicePollResult::default()));
        assert!(app.dual_usb_help_open);
        assert_eq!(app.dual_usb_help_model, "TB323FU");
        assert_eq!(app.dual_usb_help_name, "Legion Y700");

        let _ = app.update(Message::DevicePolled(device_poll("TB323FU")));
        assert!(app.dual_usb_help_open);
        assert_eq!(app.dual_usb_help_model, "TB323FU");
    }

    #[test]
    fn dual_usb_guide_does_not_reopen_after_session_close_and_replug() {
        let mut app = App {
            startup_disclaimer_open: false,
            dual_usb_advisory_dismissed: Vec::new(),
            dual_usb_advisory_closed: Vec::new(),
            ..App::default()
        };

        let _ = app.update(Message::DevicePolled(device_poll("TB323FU")));
        let _ = app.update(Message::CloseDualUsbAdvisory("TB323FU".to_string()));

        assert!(!app.dual_usb_help_open);
        assert_eq!(app.dual_usb_help_model, "TB323FU");
        assert_eq!(app.dual_usb_advisory_closed, ["TB323FU"]);

        let _ = app.update(Message::DevicePolled(DevicePollResult::default()));
        let _ = app.update(Message::DevicePolled(device_poll("TB323FU")));
        assert!(!app.dual_usb_help_open);
        assert_eq!(app.dual_usb_help_model, "TB323FU");
    }

    #[test]
    fn dual_usb_dont_show_again_roundtrips_and_suppresses_a_fresh_app() {
        let mut app = App {
            startup_disclaimer_open: false,
            dual_usb_advisory_dismissed: Vec::new(),
            dual_usb_advisory_closed: Vec::new(),
            ..App::default()
        };
        let _ = app.update(Message::DevicePolled(device_poll("TB323FU")));
        let _ = app.update(Message::DismissDualUsbAdvisory("TB323FU".to_string()));
        assert!(!app.dual_usb_help_open);
        assert_eq!(app.dual_usb_help_model, "TB323FU");

        let saved = settings_store::PersistedSettings {
            dual_usb_advisory_dismissed_models: app.dual_usb_advisory_dismissed.clone(),
            ..settings_store::PersistedSettings::default()
        };
        let json = serde_json::to_string(&saved).unwrap();
        let restored: settings_store::PersistedSettings = serde_json::from_str(&json).unwrap();
        let mut fresh_app = App {
            startup_disclaimer_open: false,
            dual_usb_advisory_dismissed: restored.dual_usb_advisory_dismissed_models,
            dual_usb_advisory_closed: Vec::new(),
            ..App::default()
        };

        let _ = fresh_app.update(Message::DevicePolled(device_poll("TB323FU")));
        assert!(!fresh_app.dual_usb_help_open);
        assert!(fresh_app.dual_usb_help_model.is_empty());
        assert_eq!(fresh_app.dual_usb_advisory_model(), None);
    }

    #[test]
    fn second_dual_usb_model_still_auto_opens_after_first_model_is_closed() {
        let mut app = App {
            startup_disclaimer_open: false,
            dual_usb_advisory_dismissed: Vec::new(),
            dual_usb_advisory_closed: Vec::new(),
            ..App::default()
        };

        let _ = app.update(Message::DevicePolled(device_poll("TB323FU")));
        let _ = app.update(Message::CloseDualUsbAdvisory("TB323FU".to_string()));
        let _ = app.update(Message::DevicePolled(device_poll("TB322FC")));

        assert!(app.dual_usb_help_open);
        assert_eq!(app.dual_usb_help_model, "TB322FC");
        assert_eq!(app.dual_usb_advisory_model(), Some("TB322FC"));
    }

    #[test]
    fn driver_restart_recommendation_tracks_success_and_close() {
        let mut app = App::default();
        assert!(!app.driver_restart_recommended);

        let _ = app.update(Message::InstallDriversDone(Ok(Vec::new())));
        assert!(app.driver_restart_recommended);

        let _ = app.update(Message::CloseDriverRestartRecommended);
        assert!(!app.driver_restart_recommended);
    }

    #[test]
    fn sidebar_specific_label_keys_use_trimmed_variants() {
        assert_eq!(View::Flash.sidebar_label_key(), "nav_flash_sidebar");
        assert_eq!(View::Flash.label_key(), "nav_flash");
        assert_eq!(View::KonaBess.sidebar_label_key(), "nav_konabess_sidebar");
        assert_eq!(View::KonaBess.label_key(), "nav_konabess");
        assert_eq!(
            View::Dashboard.sidebar_label_key(),
            View::Dashboard.label_key()
        );
    }

    #[test]
    fn sidebar_animation_is_hover_driven_only_in_compact_layout() {
        let mut app = App::default();
        app.window_size.0 = MIN_WINDOW_WIDTH;
        assert_eq!(app.window_size_class(), WindowSizeClass::Compact);
        assert_eq!(app.sidebar_anim_target(), 0.0);

        let _ = app.update(Message::SidebarHoverEnter);
        assert!(app.sidebar_expanded);
        assert_eq!(app.sidebar_anim_target(), 1.0);

        app.window_size.0 = 1320.0;
        app.sidebar_expanded = false;
        let _ = app.update(Message::SidebarHoverEnter);
        assert_eq!(app.window_size_class(), WindowSizeClass::Expanded);
        assert!(!app.sidebar_expanded);
        assert_eq!(app.sidebar_anim_target(), 0.0);
        assert_eq!(app.sidebar_visual_progress(), 1.0);
    }

    #[test]
    fn konabess_is_in_main_navigation_directly_after_unroot() {
        let unroot = NAV_MAIN
            .iter()
            .position(|view| *view == View::Unroot)
            .expect("Unroot is in main navigation");
        assert_eq!(NAV_MAIN.get(unroot + 1), Some(&View::KonaBess));
        assert_eq!(NAV_MAIN.get(unroot + 2), Some(&View::Reboot));
        assert!(!NAV_TOOLS.contains(&View::KonaBess));
    }

    #[test]
    fn sidebar_motion_settles_after_reversing_direction() {
        let mut app = App::default();
        app.window_size.0 = 900.0;
        let _ = app.update(Message::SidebarHoverEnter);
        for _ in 0..4 {
            let _ = app.update(Message::SidebarAnimTick);
        }
        let _ = app.update(Message::SidebarHoverExit);
        for _ in 0..300 {
            let _ = app.update(Message::SidebarAnimTick);
            assert!((0.0..=1.0).contains(&app.sidebar_label_alpha));
        }
        assert_eq!(app.sidebar_anim, 0.0);
        assert_eq!(app.sidebar_velocity, 0.0);
        assert_eq!(app.sidebar_label_alpha, 0.0);
        assert_eq!(app.sidebar_label_velocity, 0.0);
    }

    #[test]
    fn unknown_key_falls_back_to_itself() {
        let t = Translations::load(Language::En);
        assert_eq!(t.t("__no_such_key__"), "__no_such_key__");
    }

    #[test]
    fn non_empty_prop_treats_blank_as_absent() {
        assert_eq!(non_empty_prop(""), None);
        assert_eq!(non_empty_prop("   \n\t"), None);
        assert_eq!(
            non_empty_prop("  Tab Plus 14  \n"),
            Some("Tab Plus 14".to_string())
        );
    }

    #[test]
    fn select_device_name_falls_back_through_lgsi_props() {
        use std::collections::HashMap;
        let pick = |map: HashMap<&'static str, &'static str>| {
            select_device_name(|p| map.get(p).copied().unwrap_or("").to_string())
        };

        // Primary populated wins.
        assert_eq!(
            pick(HashMap::from([(
                "ro.vendor.config.lgsi.en.market_name",
                "Tab Plus"
            )])),
            "Tab Plus"
        );
        // Primary whitespace-only -> vendor LGSI market name.
        assert_eq!(
            pick(HashMap::from([
                ("ro.vendor.config.lgsi.en.market_name", "   "),
                ("ro.vendor.config.lgsi.market_name", "Tab Vendor"),
            ])),
            "Tab Vendor"
        );
        // -> system LGSI market name.
        assert_eq!(
            pick(HashMap::from([(
                "ro.config.lgsi.market_name",
                "Tab System"
            )])),
            "Tab System"
        );
        // -> legacy kirby_en final fallback (preserved).
        assert_eq!(
            pick(HashMap::from([("ro.vendor.config.lgsi.kirby_en", "Kirby")])),
            "Kirby"
        );
        // Nothing populated -> empty string.
        assert_eq!(pick(HashMap::new()), "");
    }

    #[test]
    fn efisp_asset_suffix_picks_prc_or_row() {
        assert_eq!(efisp_asset_suffix(true, false), "_prc.efi");
        assert_eq!(efisp_asset_suffix(false, false), "_row.efi");
        // Anti-rollback downgrade requests the `_arb` GBL (testkey root).
        assert_eq!(efisp_asset_suffix(true, true), "_prc_arb.efi");
        assert_eq!(efisp_asset_suffix(false, true), "_row_arb.efi");
    }

    #[test]
    fn efisp_is_empty_only_for_all_zero() {
        assert!(!efisp_is_empty(&[]));
        assert!(efisp_is_empty(&[0u8; 4096]));
        assert!(!efisp_is_empty(&[0, 0, 1, 0]));
        let mut buf = vec![0u8; 1024];
        buf[1000] = 0xEF;
        assert!(!efisp_is_empty(&buf));
    }

    #[test]
    fn advanced_in_progress_gates_partition_table_on_edl() {
        let row = || FlashPartRow {
            lun: 4,
            label: "boot_a".into(),
            start_sector: 0,
            num_sectors: 0,
            size_bytes: 0,
            file_path: None,
            state: FlashRowState::Skip,
        };
        let mut app = App {
            device: DeviceSnapshot {
                connection: ConnectionStatus::Edl,
                ..Default::default()
            },
            advanced_wizard_open: AdvancedWizardOpen::FlashParts,
            ..App::default()
        };
        // No scanned rows yet → not preserve-worthy.
        assert!(!app.advanced_in_progress());
        // GPT table loaded + still in EDL → preserve.
        app.flash_parts.rows = vec![row()];
        assert!(app.advanced_in_progress());
        // Device left EDL → table is stale → reset.
        app.device.connection = ConnectionStatus::None;
        assert!(!app.advanced_in_progress());

        // Physical confirm screen preserves; DumpPhys (no confirm) + the grid
        // do not.
        let mut app = App {
            advanced_wizard_open: AdvancedWizardOpen::FlashPhys,
            ..App::default()
        };
        app.flash_phys.step = FLASH_PHYS_STEPS.len() - 2; // Confirm
        assert!(app.advanced_in_progress());
        app.flash_phys.step = 1; // Select
        assert!(!app.advanced_in_progress());
        app.advanced_wizard_open = AdvancedWizardOpen::DumpPhys;
        assert!(!app.advanced_in_progress());
        app.advanced_wizard_open = AdvancedWizardOpen::None;
        assert!(!app.advanced_in_progress());

        // Exec / result surface preserves until 'start over': a Simple Flash on
        // its confirm step (folder picked) AND on its exec/result step (>=2)
        // both survive a sidebar bounce; the intro step (0, after 'start over')
        // resets.
        let mut app = App {
            advanced_wizard_open: AdvancedWizardOpen::SimpleFlash,
            ..App::default()
        };
        app.simple_flash.step = 1; // Confirm
        assert!(app.advanced_in_progress());
        app.simple_flash.step = 2; // Exec / result
        assert!(app.advanced_in_progress());
        app.simple_flash.step = 0; // Intro
        assert!(!app.advanced_in_progress());
    }

    #[test]
    fn logs_cannot_change_operation_progress() {
        let mut app = App::default();
        let reporter = app.begin_phased_op(View::Root, OperationPhaseKind::Root);
        let _ = reporter.marker(3);
        app.log_push("[dl] file 45% (12.3/45.6 MB)");
        app.log_push("[old worker] Phase 7/8");
        assert_eq!(app.operation.current_step(), 2);
        app.fail_op();
        let next = app.begin_phased_op(View::Root, OperationPhaseKind::Root);
        let _ = reporter.marker(8);
        assert_eq!(app.operation.current_step(), 0);
        let _ = next.marker(2);
        assert_eq!(app.operation.current_step(), 1);
    }

    #[test]
    fn clear_log_empties_history_and_rebuilds_the_editor() {
        let mut app = App {
            log_lines: vec!["first".into(), "second".into()],
            ..Default::default()
        };
        app.rebuild_log_editor();

        drop(app.update(Message::ClearLog));

        assert!(app.log_lines.is_empty());
        assert_eq!(app.log_editor.text(), "");
        assert!(!app.log_dirty);
    }

    #[test]
    fn primary_workers_emit_every_phase_in_order() {
        let compact = |source: &str| {
            source
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>()
        };
        let assert_marker_order = |source: &str, total: usize| {
            let mut previous = None;
            for phase in 1..=total {
                let marker = format!("phases.marker({phase})");
                let positions = source
                    .match_indices(&marker)
                    .map(|(position, _)| position)
                    .collect::<Vec<_>>();
                assert_eq!(positions.len(), 1, "expected one {marker}");
                if let Some(previous) = previous {
                    assert!(previous < positions[0], "{marker} is out of order");
                }
                previous = positions.first().copied();
            }
        };
        let flash = compact(include_str!("workers/flash/full.rs"));
        let root = compact(include_str!("workers/root.rs"));
        let unroot = compact(include_str!("workers/unroot.rs"));
        assert_marker_order(&flash, 9);
        assert_marker_order(&root, 8);
        assert_marker_order(&unroot, 5);
    }

    #[test]
    fn system_update_worker_reports_each_action_phase_in_order() {
        let compact = include_str!("workers/sysupdate.rs")
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>();
        for phase in 1..=7 {
            assert!(
                compact.contains(&format!("phases.marker({phase})")),
                "missing System Update phase {phase}"
            );
        }
        assert!(!compact.contains("phase_marker("));
    }

    #[test]
    fn advanced_edl_workers_report_their_phase_boundaries() {
        let compact = |source: &str| {
            source
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>()
        };
        let function = |source: &str, start: &str, end: Option<&str>| {
            let (_, tail) = source.split_once(start).expect("worker function exists");
            end.and_then(|end| tail.split_once(end).map(|(body, _)| body))
                .unwrap_or(tail)
                .to_string()
        };
        let assert_once_in_order = |source: &str, total: usize| {
            let mut previous = None;
            for phase in 1..=total {
                let marker = format!("phases.marker({phase})");
                let positions = source
                    .match_indices(&marker)
                    .map(|(position, _)| position)
                    .collect::<Vec<_>>();
                assert_eq!(positions.len(), 1, "expected one {marker}");
                if let Some(previous) = previous {
                    assert!(previous < positions[0], "{marker} is out of order");
                }
                previous = positions.first().copied();
            }
        };

        let transfer = compact(include_str!("workers/transfer.rs"));
        let flash_parts = function(
            &transfer,
            "pub(crate)fnflash_parts_execute(",
            Some("pub(crate)fndump_parts_scan("),
        );
        let dump_parts = function(
            &transfer,
            "pub(crate)fndump_parts_execute(",
            Some("pub(crate)fndump_physical_execute("),
        );
        let dump_physical = function(
            &transfer,
            "pub(crate)fndump_physical_execute(",
            Some("pub(crate)fnflash_physical_execute("),
        );
        let flash_physical = function(&transfer, "pub(crate)fnflash_physical_execute(", None);
        assert_once_in_order(&flash_parts, 3);
        assert_once_in_order(&dump_parts, 4);
        assert_once_in_order(&dump_physical, 5);
        assert_once_in_order(&flash_physical, 4);

        let simple = compact(include_str!("workers/flash/simple.rs"));
        assert_once_in_order(&simple, 5);

        let country = compact(include_str!("workers/flash/country.rs"));
        for phase in [1, 2, 5] {
            assert_eq!(
                country.matches(&format!("phases.marker({phase})")).count(),
                1
            );
        }
        let country_shared = compact(include_str!("workers/flash/mod.rs"));
        for phase in [3, 4] {
            assert_eq!(
                country_shared
                    .matches(&format!("phases.marker({phase})"))
                    .count(),
                1
            );
        }

        let arb = compact(include_str!("arb.rs"));
        assert_eq!(arb.matches("phases.marker(1)").count(), 1);
        assert_eq!(arb.matches("phases.marker(2)").count(), 1);
        assert_eq!(arb.matches("phases.marker(3)").count(), 1);
        assert_eq!(arb.matches("phases.marker(4)").count(), 2);
        assert_eq!(arb.matches("phases.marker(5)").count(), 2);
    }

    #[test]
    fn offline_advanced_worker_reports_each_phase_boundary() {
        let source = include_str!("workers/advanced.rs")
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>();
        let action = |start: &str, end: Option<&str>| {
            let (_, tail) = source.split_once(start).expect("action arm exists");
            end.and_then(|end| tail.split_once(end).map(|(body, _)| body))
                .unwrap_or(tail)
                .to_string()
        };
        let assert_once_in_order = |body: &str, total: usize| {
            let mut previous = None;
            for phase in 1..=total {
                let marker = format!("phases.marker({phase})");
                let positions = body
                    .match_indices(&marker)
                    .map(|(position, _)| position)
                    .collect::<Vec<_>>();
                assert_eq!(positions.len(), 1, "expected one {marker}");
                if let Some(previous) = previous {
                    assert!(previous < positions[0], "{marker} is out of order");
                }
                previous = positions.first().copied();
            }
        };

        let xml = action("AdvAction::ConvertXml=>{", Some("AdvAction::DetectArb=>{"));
        let region = action(
            "AdvAction::RegionConvert=>{",
            Some("AdvAction::PatchDevinfo=>{"),
        );
        let patch_arb = action(
            "pub(crate)fnpatch_firmware_rollback(",
            Some("pub(crate)fnadvanced_file_worker("),
        );
        let rebuild = action("AdvAction::RebuildVbmeta=>{", None);
        assert_once_in_order(&xml, 3);
        assert_once_in_order(&patch_arb, 4);
        assert_once_in_order(&rebuild, 3);
        assert_eq!(region.matches("phases.marker(1)").count(), 1);
        assert!(region.contains("RegionBuildStage::Inspect=>2"));
        assert!(region.contains("RegionBuildStage::PatchVendorBoot=>3"));
        assert!(region.contains("RegionBuildStage::RebuildVbmeta=>4"));
        assert_eq!(region.matches("phases.marker(4)").count(), 1);
        assert!(!source.contains("phase_marker("));
    }

    #[test]
    fn refined_phase_labels_exist_in_every_locale() {
        let keys = [
            "op_flash_phase_5",
            "op_flash_phase_6",
            "op_flash_phase_7",
            "op_unroot_phase_4",
            "op_unroot_phase_5",
            "op_unroot_phase_6",
        ];
        for lang in [
            Language::En,
            Language::Ko,
            Language::Zh,
            Language::Ru,
            Language::Ja,
        ] {
            let translations = Translations::load(lang);
            for key in keys {
                assert_ne!(translations.t(key), key, "{lang:?} missing {key}");
            }
        }
    }

    #[test]
    fn every_operation_phase_label_exists_in_every_locale() {
        for lang in [
            Language::En,
            Language::Ko,
            Language::Zh,
            Language::Ru,
            Language::Ja,
        ] {
            let translations = Translations::load(lang);
            for kind in OperationPhaseKind::all() {
                for key in kind.keys() {
                    assert_ne!(translations.t(key), *key, "{lang:?} missing {key}");
                }
            }
        }
    }

    // Wizard state-machine tests ------------------------------------------

    #[test]
    fn flash_wizard_next_back_round_trip() {
        let mut w = FlashWizard::default();
        assert_eq!(w.step, 0);
        // Can't advance without a region selected.
        assert!(!w.can_next());
        w.region_selection = Some(FlashRegionSelection::Manual(DeviceRegion::Prc));
        w.device_region = Some(DeviceRegion::Prc);
        assert!(w.can_next());
        w.next();
        assert_eq!(w.step, 1);
        w.back();
        assert_eq!(w.step, 0);
        // Reset wipes every field.
        w.next();
        w.reset();
        assert_eq!(w.step, 0);
        assert!(w.region_selection.is_none());
        assert!(w.device_region.is_none());
    }

    #[test]
    fn flash_region_auto_lookup_is_user_initiated() {
        let mut app = App::default();

        drop(app.prepare_flash_region_on_entry());
        assert!(app.flash.region_selection.is_none());
        assert!(app.flash_serial_prompt.is_none());
        assert!(app.queries.region_pending.is_none());

        drop(app.update_flash(FlashMsg::FlashRegionAuto));
        assert_eq!(app.flash.region_selection, Some(FlashRegionSelection::Auto));
        assert!(app.flash_serial_prompt.is_none());

        drop(app.update_flash(FlashMsg::FlashNext));
        assert_eq!(app.flash.step, 0);
        assert!(app.flash_serial_prompt.is_some());

        drop(app.update_flash(FlashMsg::FlashSerialPromptSkip));
        assert!(app.flash_serial_prompt.is_none());
        assert!(app.flash.region_auto_unknown);
    }

    #[test]
    fn flash_confirm_requires_loader_when_folder_has_none() {
        let mut w = FlashWizard {
            step: 4,
            firmware_folder: Some("firmware".to_string()),
            firmware_identity: Some(FirmwareIdentity {
                efisp_load: ltbox_patch::efisp_load::EfispLoad::Undetermined,
                key_class: ltbox_patch::key_map::KeyClass::Testkey,
                fingerprint: None,
                model_token: None,
            }),
            loader_required: true,
            ..Default::default()
        };
        assert!(!w.can_next());

        w.loader_override = Some("prog_firehose.elf".to_string());
        assert!(w.can_next());
    }

    #[test]
    fn confirm_step_is_the_step_before_exec() {
        // Linear (trait default): confirm = step_count - 2, exec = -1.
        let mut f = FlashWizard::default();
        let confirm = f.step_count() - 2;
        f.step = 0;
        assert!(!f.is_on_confirm_step());
        f.step = confirm;
        assert!(f.is_on_confirm_step());
        assert!(!f.is_in_exec());
        f.step = f.step_count() - 1;
        assert!(!f.is_on_confirm_step());
        assert!(f.is_in_exec());

        // SysUpdate step count flexes with rescue mode; confirm still tracks
        // step_count - 2 on both the compact and the longer rescue flow.
        let mut s = SysUpdateWizard::default();
        s.step = s.step_count() - 2; // compact: confirm = step 1
        assert!(s.is_on_confirm_step());
        s.action = Some(SysUpdateAction::Rescue);
        s.step = s.step_count() - 2; // rescue: confirm = step 2
        assert!(s.is_on_confirm_step());
        s.step = 1; // rescue folder step — not confirm
        assert!(!s.is_on_confirm_step());

        // Root is non-linear: confirm = step 6, exec = step 7.
        let mut r = RootWizard {
            step: 6,
            ..Default::default()
        };
        assert!(r.is_on_confirm_step());
        r.step = 7;
        assert!(!r.is_on_confirm_step());
        assert!(r.is_in_exec());
        r.step = 0;
        assert!(!r.is_on_confirm_step());
    }

    #[test]
    fn root_wizard_kernelsu_lkm_path() {
        let mut w = RootWizard {
            family: Some(Family::KernelSU),
            ..RootWizard::default()
        };
        w.next(); // 0 → 1 (Mode)
        assert_eq!(w.step, 1);
        w.mode = Some(RootMode::Lkm);
        w.next(); // 1 → 2 (Provider)
        assert_eq!(w.step, 2);
        w.provider = Some(Provider::KernelSU);
        w.next(); // 2 → 3 (Version)
        assert_eq!(w.step, 3);
        w.version = Some(VerChoice::Stable);
        w.next(); // Stable skips NightlySource, jumps to Confirm (5)
        assert_eq!(w.step, 5);
    }

    #[test]
    fn root_wizard_kernelsu_lkm_requires_kernel_version_before_exec() {
        let mut w = RootWizard {
            family: Some(Family::KernelSU),
            mode: Some(RootMode::Lkm),
            provider: Some(Provider::KernelSU),
            version: Some(VerChoice::Stable),
            folder_path: Some("firmware".to_string()),
            step: 6,
            ..RootWizard::default()
        };

        assert!(w.needs_ksu_lkm_kernel_version());
        w.kernel_version = Some("6.1".to_string());
        assert!(!w.needs_ksu_lkm_kernel_version());
    }

    #[test]
    fn root_wizard_magisk_skips_mode() {
        let mut w = RootWizard {
            family: Some(Family::Magisk),
            ..RootWizard::default()
        };
        w.next(); // 0 → 2 directly (Magisk has no modes)
        assert_eq!(w.step, 2);
    }

    #[test]
    fn root_wizard_skroot_lite_skips_provider_version() {
        let mut w = RootWizard {
            family: Some(Family::Skroot),
            ..RootWizard::default()
        };
        w.next(); // 0 → 1 (Lite / Pro)
        assert_eq!(w.step, 1);
        assert!(!w.can_next());
        w.skroot_flavor = Some(SkrootFlavor::Pro);
        assert!(!w.can_next());
        w.skroot_flavor = Some(SkrootFlavor::Lite);
        assert!(w.can_next());
        w.next(); // 1 → 5 (Loader)
        assert_eq!(w.step, 5);
        assert_eq!(w.display_step(), 2);
        w.back(); // 5 → 1
        assert_eq!(w.step, 1);
    }

    #[test]
    fn image_info_wizard_runs_after_multi_image_selection() {
        let mut w = AdvWizard::default();
        w.open(AdvAction::ImageInfo);

        assert_eq!(w.steps(), &["adv_step_source", "adv_step_info"]);
        assert!(!w.is_confirm_step());
        assert!(!w.can_next());

        w.file_paths = vec!["boot.img".into(), "vbmeta.img".into()];
        assert!(w.can_next());
        w.next();
        assert_eq!(w.step, w.exec_step());
    }

    #[test]
    fn advanced_menu_taxonomy_matches_avb_image_reclass() {
        let section = |key: &str| {
            ADV_SECTIONS
                .iter()
                .find(|section| section.title_key == key)
                .expect("section exists")
                .items
        };

        assert_eq!(
            section("adv_section_region_patch"),
            &[AdvAction::RegionConvert, AdvAction::PatchDevinfo]
        );
        assert!(
            ADV_SECTIONS
                .iter()
                .all(|section| section.title_key != "adv_section_country_code")
        );
        assert_eq!(
            section("adv_section_rollback"),
            &[
                AdvAction::ImageInfo,
                AdvAction::DetectArb,
                AdvAction::PatchArb,
                AdvAction::RebuildVbmeta,
            ]
        );
        assert_eq!(
            section("adv_section_edl_ops"),
            &[
                AdvAction::ConvertXml,
                AdvAction::DumpPartitions,
                AdvAction::FlashPartitions,
                AdvAction::DumpPhysical,
                AdvAction::FlashPhysical,
                AdvAction::SimpleFlash,
            ]
        );
    }

    fn assert_template_call_replaces(source: &str, key: &str, placeholders: &[&str]) {
        // Whitespace-strip the whole source so rustfmt line-wrapping (which can
        // split a tr_args! call across lines) doesn't hide it. Accept either
        // substitution form: the manual tr(key) followed by a replace chain, or
        // the tr_args! macro (which expands to the same chain). Both guarantee
        // the placeholder is filled rather than shipped literally.
        let compact: String = source.chars().filter(|c| !c.is_whitespace()).collect();
        let tr_args_needle = format!("tr_args!(\"{key}\"");
        if let Some(pos) = compact.find(&tr_args_needle) {
            let window = &compact[pos..(pos + 2_000).min(compact.len())];
            for placeholder in placeholders {
                assert!(
                    window.contains(&format!("{placeholder}=")),
                    "{key} (tr_args!) must pass {placeholder}"
                );
            }
            return;
        }
        let needle = format!("tr(\"{key}\")");
        let pos = compact.find(&needle).expect("template key must be used");
        let window = &compact[pos..(pos + 2_000).min(compact.len())];
        for placeholder in placeholders {
            assert!(
                window.contains(&format!(".replace(\"{{{placeholder}}}\"")),
                "{key} must replace {{{placeholder}}} near its log call"
            );
        }
    }

    #[test]
    fn high_risk_log_templates_replace_visible_placeholders() {
        // Concatenate the GUI sources that carry high-risk log templates;
        // some live in main.rs, others in the extracted worker modules.
        let gui_src = concat!(
            include_str!("main.rs"),
            include_str!("arb.rs"),
            include_str!("root_manager.rs"),
            include_str!("arb_overlay.rs"),
            include_str!("workers/transfer.rs"),
            include_str!("workers/flash/mod.rs"),
            include_str!("workers/flash/full.rs"),
            include_str!("workers/flash/country.rs"),
            include_str!("workers/flash/simple.rs"),
        );
        let edl_rs = include_str!("../../ltbox-device/src/edl.rs");

        assert_template_call_replaces(
            edl_rs,
            "log_edl_flash_program_cmd",
            &["label", "image", "lun", "start", "sectors"],
        );
        assert_template_call_replaces(
            gui_src,
            "live_country_dump_partition",
            &["label", "lun", "start", "sectors"],
        );
        assert_template_call_replaces(gui_src, "live_dump_phys_dumping_lun", &["lun", "path"]);
        assert_template_call_replaces(gui_src, "live_dump_phys_lun_failed", &["lun", "error"]);
    }

    #[test]
    fn country_popup_selection_uses_opening_flow_context() {
        let app = App {
            adv_needs_country: true,
            adv_wizard: AdvWizard {
                country: Some("KR".to_string()),
                ..AdvWizard::default()
            },
            wf_config: WorkflowConfig {
                country_action: CountryAction::Set("CN".to_string()),
                ..WorkflowConfig::default()
            },
            ..App::default()
        };
        assert_eq!(app.country_popup_selected_code(), Some("KR"));

        let app = App {
            adv_needs_country: false,
            adv_wizard: AdvWizard {
                country: Some("KR".to_string()),
                ..AdvWizard::default()
            },
            wf_config: WorkflowConfig {
                country_action: CountryAction::Set("CN".to_string()),
                ..WorkflowConfig::default()
            },
            ..App::default()
        };
        assert_eq!(app.country_popup_selected_code(), Some("CN"));
    }

    #[test]
    fn country_popup_stages_a_row_until_footer_confirmation() {
        let mut app = App {
            country_popup_open: true,
            country_popup_draft: CountryAction::Set("CN".to_string()),
            wf_config: WorkflowConfig {
                country_action: CountryAction::Set("US".to_string()),
                ..WorkflowConfig::default()
            },
            ..App::default()
        };

        let _ = app.update(Message::SelectCountry("KR".to_string()));
        assert!(app.country_popup_open);
        assert_eq!(app.country_popup_draft.target(), Some("KR"));
        assert_eq!(app.wf_config.country_action.target(), Some("US"));

        let _ = app.update(Message::CountryPopupConfirm);
        assert!(!app.country_popup_open);
        assert_eq!(app.wf_config.country_action.target(), Some("KR"));
    }

    #[test]
    fn flash_keep_data_preserves_confirm_country_override() {
        let mut app = App {
            wf_config: WorkflowConfig {
                wipe: true,
                country_action: CountryAction::Set("KR".to_string()),
                ..WorkflowConfig::default()
            },
            ..App::default()
        };

        let _ = app.update_flash(FlashMsg::FlashConfirmSetData(DataMode::Keep));

        assert!(!app.wf_config.wipe);
        assert_eq!(app.wf_config.country_action.target(), Some("KR"));
    }

    #[test]
    fn flash_confirm_country_popup_dismiss_stays_on_confirm() {
        let mut app = App {
            flash: FlashWizard {
                step: 4,
                ..FlashWizard::default()
            },
            country_popup_open: true,
            wf_config: WorkflowConfig {
                country_action: CountryAction::Unset,
                ..WorkflowConfig::default()
            },
            ..App::default()
        };

        let _ = app.update(Message::DismissCountryPopup);

        assert!(!app.country_popup_open);
        assert_eq!(app.flash.step, 4);
    }

    #[test]
    fn sysupdate_wizard_gate_requires_action() {
        let mut w = SysUpdateWizard::default();
        assert!(!w.can_next());
        w.action = Some(SysUpdateAction::Disable);
        assert!(w.can_next());
        w.next();
        assert_eq!(w.step, 1);
        w.next();
        w.next();
        // Caps at len - 1.
        assert_eq!(w.step, SYSUPDATE_STEPS_COMPACT.len() - 1);
    }

    #[test]
    fn flash_parts_wizard_requires_selection() {
        let mut w = FlashPartsWizard::default();
        assert!(!w.can_next());
        w.loader_path = Some("/tmp/xbl.melf".to_string());
        // Step 0 only needs a loader picked.
        assert!(w.can_next());
        w.next();
        assert_eq!(w.step, 1);
        // Step 1: need at least one row with a resolvable action.
        w.rows.push(FlashPartRow {
            lun: 0,
            label: "boot_a".into(),
            start_sector: 0,
            num_sectors: 8192,
            size_bytes: 4 * 1024 * 1024,
            file_path: None,
            state: FlashRowState::Skip,
        });
        assert!(!w.can_next()); // Unchecked doesn't count
        w.rows[0].state = FlashRowState::Write;
        assert!(!w.can_next()); // Flash w/o file still invalid
        w.rows[0].file_path = Some("/tmp/boot.img".into());
        assert!(w.can_next());
        // Erase alone is enough — no file required.
        w.rows[0].state = FlashRowState::Erase;
        w.rows[0].file_path = None;
        assert!(w.can_next());
    }

    #[test]
    fn partition_checkbox_cycles_without_picker_and_blocks_incomplete_writes() {
        let mut app = App::default();
        app.flash_parts.step = 1;
        app.flash_parts.rows.push(FlashPartRow {
            lun: 0,
            label: "userdata".into(),
            start_sector: 0,
            num_sectors: 1,
            size_bytes: 512,
            file_path: None,
            state: FlashRowState::Skip,
        });
        let task = app.update(Message::FlashParts(FlashPartsMsg::FlashPartsToggleRow(0)));
        assert_eq!(task.units(), 0);
        assert_eq!(app.flash_parts.rows[0].state, FlashRowState::Write);
        assert!(!app.flash_parts.can_next());
        let task = app.update(Message::FlashParts(FlashPartsMsg::FlashPartsToggleRow(0)));
        assert_eq!(task.units(), 0);
        assert_eq!(app.flash_parts.rows[0].state, FlashRowState::Erase);
        assert!(app.flash_parts.can_next());
        let mut incomplete = app.flash_parts.rows[0].clone();
        incomplete.state = FlashRowState::Write;
        app.flash_parts.rows.push(incomplete);
        for step in [1, 2] {
            app.flash_parts.step = step;
            assert!(!app.flash_parts.can_next());
        }
        assert_eq!(
            app.update(Message::FlashParts(FlashPartsMsg::FlashPartsExecStart))
                .units(),
            0
        );
        assert!(!app.operation.is_running());
    }

    #[test]
    fn partition_confirmation_cancel_restores_only_transitional_entry_modes() {
        let loader = tempfile::Builder::new().suffix(".melf").tempfile().unwrap();
        for entry in [
            ConnectionStatus::Adb,
            ConnectionStatus::Fastboot,
            ConnectionStatus::Edl,
        ] {
            let mut app = App {
                current_view: View::Advanced,
                advanced_wizard_open: AdvancedWizardOpen::FlashParts,
                flash_parts: FlashPartsWizard {
                    step: 2,
                    loader_path: Some(loader.path().to_string_lossy().into_owned()),
                    entry_connection: Some(entry),
                    ..Default::default()
                },
                ..App::default()
            };
            // Dropping the task proves scheduling without touching any device.
            let _task = app.update(Message::StartOver);
            assert_eq!(app.advanced_wizard_open, AdvancedWizardOpen::None);
            assert_eq!(app.operation.is_running(), entry != ConnectionStatus::Edl);
            assert_eq!(app.flash_parts.entry_connection, None);
        }
    }

    #[test]
    fn advanced_partition_tables_started_in_edl_keep_back_without_rebooting() {
        assert_eq!(
            partition_table_leading_action(Some(ConnectionStatus::Edl)),
            WizardLeadingAction::Back
        );

        let mut flash_app = App {
            device: DeviceSnapshot {
                connection: ConnectionStatus::Edl,
                ..Default::default()
            },
            advanced_wizard_open: AdvancedWizardOpen::FlashParts,
            flash_parts: FlashPartsWizard {
                step: 1,
                entry_connection: Some(ConnectionStatus::Edl),
                ..FlashPartsWizard::default()
            },
            ..App::default()
        };
        let _task = flash_app.update_flash_parts(FlashPartsMsg::FlashPartsBack);
        assert_eq!(flash_app.flash_parts.step, 0);
        assert!(!flash_app.operation.is_running());
        assert_eq!(
            flash_app.advanced_wizard_open,
            AdvancedWizardOpen::FlashParts
        );

        let mut dump_app = App {
            device: DeviceSnapshot {
                connection: ConnectionStatus::Edl,
                ..Default::default()
            },
            advanced_wizard_open: AdvancedWizardOpen::DumpParts,
            dump_parts: DumpPartsWizard {
                step: 1,
                entry_connection: Some(ConnectionStatus::Edl),
                ..DumpPartsWizard::default()
            },
            ..App::default()
        };
        let _task = dump_app.update_dump_parts(DumpPartsMsg::DumpPartsBack);
        assert_eq!(dump_app.dump_parts.step, 0);
        assert!(!dump_app.operation.is_running());
        assert_eq!(dump_app.advanced_wizard_open, AdvancedWizardOpen::DumpParts);
    }

    #[test]
    fn advanced_partition_tables_started_elsewhere_cancel_and_schedule_system_reboot() {
        assert_eq!(
            partition_table_leading_action(Some(ConnectionStatus::Adb)),
            WizardLeadingAction::Cancel
        );
        assert_eq!(
            partition_table_leading_action(Some(ConnectionStatus::Fastboot)),
            WizardLeadingAction::Cancel
        );
        assert_eq!(
            partition_table_leading_action(Some(ConnectionStatus::None)),
            WizardLeadingAction::Cancel
        );
        assert_eq!(
            partition_table_leading_action(None),
            WizardLeadingAction::Back
        );

        let loader = tempfile::Builder::new()
            .suffix(".melf")
            .tempfile()
            .expect("temporary loader");
        let loader_path = loader.path().to_string_lossy().to_string();

        let mut flash_app = App {
            advanced_wizard_open: AdvancedWizardOpen::FlashParts,
            // The live state is EDL after the scan; only the captured entry
            // state can prove that LTBox changed it.
            device: DeviceSnapshot {
                connection: ConnectionStatus::Edl,
                ..Default::default()
            },
            flash_parts: FlashPartsWizard {
                step: 1,
                loader_path: Some(loader_path.clone()),
                entry_connection: Some(ConnectionStatus::Adb),
                ..FlashPartsWizard::default()
            },
            ..App::default()
        };
        let _task = flash_app.update_flash_parts(FlashPartsMsg::FlashPartsBack);
        assert!(flash_app.operation.is_running());
        assert_eq!(flash_app.operation.view(), Some(View::Reboot));
        assert_eq!(flash_app.advanced_wizard_open, AdvancedWizardOpen::None);
        assert_eq!(flash_app.flash_parts.entry_connection, None);

        let mut dump_app = App {
            device: DeviceSnapshot {
                connection: ConnectionStatus::Edl,
                ..Default::default()
            },
            advanced_wizard_open: AdvancedWizardOpen::DumpParts,
            dump_parts: DumpPartsWizard {
                step: 1,
                loader_path: Some(loader_path),
                entry_connection: Some(ConnectionStatus::Fastboot),
                ..DumpPartsWizard::default()
            },
            ..App::default()
        };
        let _task = dump_app.update_dump_parts(DumpPartsMsg::DumpPartsBack);
        assert!(dump_app.operation.is_running());
        assert_eq!(dump_app.operation.view(), Some(View::Reboot));
        assert_eq!(dump_app.advanced_wizard_open, AdvancedWizardOpen::None);
        assert_eq!(dump_app.dump_parts.entry_connection, None);
    }

    #[test]
    fn flash_parts_erase_marker_keeps_checkbox_square_footprint() {
        assert_eq!(FLASH_PARTS_MARKER_CELL_WIDTH, 32.0);
        assert_eq!(FLASH_PARTS_MARKER_SIZE, 16.0);
        let dash_width = std::hint::black_box(FLASH_PARTS_ERASE_DASH_WIDTH);
        let marker_size = std::hint::black_box(FLASH_PARTS_MARKER_SIZE);
        assert!(dash_width < marker_size);
        assert!(marker_size < FLASH_PARTS_MARKER_CELL_WIDTH);
    }

    #[test]
    fn busy_progress_dialog_shows_only_without_inline_log_surface() {
        let mut app = App {
            operation: OperationExecution::fixture(true, Some(View::Reboot), Vec::new(), 0, None),
            current_view: View::Reboot,
            ..App::default()
        };

        assert!(app.should_show_busy_progress_dialog());

        app.current_view = View::Dashboard;
        assert!(!app.should_show_busy_progress_dialog());

        app.current_view = View::Advanced;
        app.advanced_wizard_open = AdvancedWizardOpen::FlashParts;
        app.flash_parts.step = 0;
        assert!(app.should_show_busy_progress_dialog());

        app.flash_parts.step = 3;
        assert!(!app.should_show_busy_progress_dialog());

        app.advanced_wizard_open = AdvancedWizardOpen::DumpParts;
        app.dump_parts.step = 0;
        assert!(app.should_show_busy_progress_dialog());

        app.dump_parts.step = 2;
        assert!(!app.should_show_busy_progress_dialog());

        app.advanced_wizard_open = AdvancedWizardOpen::None;
        app.current_view = View::Flash;
        app.flash.step = FLASH_STEPS.len() - 1;
        assert!(!app.should_show_busy_progress_dialog());
    }

    #[test]
    fn konabess_inspection_uses_busy_dialog_and_flash_uses_inline_exec_surface() {
        let mut app = App {
            operation: OperationExecution::fixture(true, Some(View::KonaBess), Vec::new(), 0, None),
            current_view: View::KonaBess,
            ..App::default()
        };

        app.konabess.step = 0;
        assert!(!app.current_view_has_inline_exec_surface());
        assert!(app.should_show_busy_progress_dialog());
        assert_eq!(
            app.busy_body_override().as_deref(),
            Some(app.t("busy_konabess_inspection"))
        );

        app.konabess.step = 3;
        assert!(app.current_view_has_inline_exec_surface());
        assert!(!app.should_show_busy_progress_dialog());
    }

    #[test]
    fn material_progress_replaces_iced_aw_spinner() {
        let loading_views = concat!(
            include_str!("view/chrome.rs"),
            include_str!("view/flash.rs"),
            include_str!("view/sysupdate.rs"),
        );
        assert!(
            !loading_views.contains("Spinner::new"),
            "all loading surfaces must use the shared Material progress ring"
        );
    }

    #[test]
    fn log_popup_uses_labeled_action_bar_buttons() {
        let source = include_str!("view/popups.rs");
        let popup = source
            .split_once("pub(crate) fn log_popup_view")
            .expect("log popup view must exist")
            .1;
        assert!(
            popup.contains("wizard_secondary_action("),
            "save and close must use visible-label action buttons"
        );
        assert!(
            popup.contains("wizard_action_footer("),
            "the log popup must use the compact action bar"
        );
        assert!(
            !popup.contains("floating_surface_action("),
            "the log popup must not render navigation as a FAB"
        );
    }

    #[test]
    fn wizard_step_state_tracks_completed_active_and_upcoming() {
        assert_eq!(wizard_step_state(0, 2), WizardStepState::Completed);
        assert_eq!(wizard_step_state(2, 2), WizardStepState::Active);
        assert_eq!(wizard_step_state(3, 2), WizardStepState::Upcoming);
    }

    #[test]
    fn image_info_result_uses_shared_action_hierarchy() {
        let source = include_str!("view/advanced.rs");
        let result = source
            .split_once("pub(crate) fn adv_image_info_exec_step")
            .expect("image info execution view must exist")
            .1
            .split_once("pub(crate) fn view_simple_flash_wizard")
            .expect("simple flash view must follow image info")
            .0;
        assert!(result.contains("wizard_secondary_action"));
        assert!(result.contains("wizard_primary_action"));
        assert!(result.contains("wizard_action_footer"));
        assert!(!result.contains("floating_surface_action"));
    }

    /// An unidentified device must not be reported as rollback-protected:
    /// the check is a deny-list, so an empty model would otherwise assert
    /// "Yes" for hardware we never read.
    #[test]
    fn arb_answer_is_blank_until_the_model_is_known() {
        assert_eq!(arb_from_model(""), "");
        assert_eq!(arb_from_model("   "), "");
        assert_eq!(arb_from_model("TB322FC"), "arb_no");
        assert_eq!(arb_from_model("TB520FU"), "arb_yes");
    }

    /// Floors come only from a bootloader poll, so their presence is the
    /// transport test; the model check keeps an exempt SKU from offering
    /// a breakdown behind a cell that reads "No".
    #[test]
    fn rollback_detail_needs_both_floors_and_a_protected_model() {
        let floors = ltbox_patch::rollback::FastbootRollbackFloors {
            vbmeta_system_location: 2,
            vbmeta_system_index: 0x69D1_A600,
            boot_location: 3,
            boot_index: 0x69D1_A600,
        };

        let mut app = App {
            device: DeviceSnapshot {
                model: "TB520FU".into(),
                rollback_floors: Some(floors),
                ..Default::default()
            },
            ..App::default()
        };
        assert!(app.rollback_detail_available());

        // Exempt SKU — the cell reads "No", so it must not be clickable.
        app.device.model = "TB322FC".into();
        assert!(!app.rollback_detail_available());

        // Any non-bootloader transport leaves the floors unset.
        app.device.model = "TB520FU".into();
        app.device.rollback_floors = None;
        assert!(!app.rollback_detail_available());
    }

    /// The popup's cycle must return to where it started, and each form
    /// must render the value the copy button will put on the clipboard.
    #[test]
    fn rollback_value_format_cycles_and_renders() {
        // Real TB520FU floor: `stored_rollback_index:3 = 69D1A600`.
        const IDX: u64 = 0x69D1_A600;

        let raw = RollbackValueFormat::Raw;
        assert_eq!(raw.render(IDX), "0x69D1A600");

        let unix = raw.next();
        assert_eq!(unix, RollbackValueFormat::Unix);
        assert_eq!(unix.render(IDX), "1775347200");

        let date = unix.next();
        assert_eq!(date, RollbackValueFormat::Date);
        assert_eq!(date.render(IDX), "2026-04-05");

        assert_eq!(date.next(), RollbackValueFormat::Raw);
    }

    #[test]
    fn manual_rollback_format_round_trips_in_every_mode() {
        const IDX: u64 = 0x69D1_A600;
        let cases = [
            (RollbackValueFormat::Raw, format!("0x{IDX:X}")),
            (RollbackValueFormat::Unix, IDX.to_string()),
            (RollbackValueFormat::Date, "2026-04-05".to_string()),
        ];

        for (format, rendered) in cases {
            assert_eq!(format.render(IDX), rendered);
            assert_eq!(format.parse(&rendered), Ok(IDX));
        }
    }

    #[test]
    fn manual_rollback_rejects_future_timestamps() {
        let app = App {
            rollback_value_format: RollbackValueFormat::Unix,
            ..App::default()
        };
        let now = current_unix_timestamp().expect("clock is after the epoch");
        let rejects_same_timestamp = (0..10).any(|_| {
            app.parse_manual_rollback(&now.to_string())
                == Err("rollback_manual_error_future".to_string())
        });
        assert!(
            rejects_same_timestamp,
            "same-second target must be rejected"
        );
        assert!(app.parse_manual_rollback("1775347199").is_ok());
    }

    #[test]
    fn reopening_the_manual_editor_keeps_what_the_user_confirmed() {
        let mut app = App {
            manual_rollback_format: RollbackValueFormat::Unix,
            ..Default::default()
        };
        // Image defaults differ from what the user settled on.
        app.flash.firmware_rollback_indices = Some((Ok(1_500_000_000), Ok(1_500_000_000)));
        app.wf_config.manual_rollback_indices = Some(ManualRollbackIndices {
            boot: 1_700_000_000,
            vbmeta_system: 1_600_000_000,
        });

        let _ = app.open_manual_rollback_editor();
        let (boot, vbmeta) = app.manual_rollback_buffers.clone().expect("buffers");
        assert_eq!(
            boot, "1700000000",
            "reopening must not revert to the image value"
        );
        assert_eq!(vbmeta, "1600000000");
        // The hint under each field reads these, so reopening must leave them
        // as the image reported them rather than adopting what the user typed.
        assert_eq!(
            app.flash.firmware_rollback_indices,
            Some((Ok(1_500_000_000), Ok(1_500_000_000))),
            "image indices are the hint's source and are not the user's values"
        );
    }

    #[test]
    fn manual_rollback_editor_defaults_to_unix_and_dashboard_to_raw() {
        let app = App::default();
        assert_eq!(app.manual_rollback_format, RollbackValueFormat::Unix);
        assert_eq!(app.rollback_value_format, RollbackValueFormat::Raw);
    }

    #[test]
    fn manual_rollback_cycle_reexpresses_the_typed_values() {
        let mut app = App {
            manual_rollback_format: RollbackValueFormat::Unix,
            manual_rollback_buffers: Some(("1700000000".into(), "1600000000".into())),
            manual_rollback_values: (Some(1_700_000_000), Some(1_600_000_000)),
            ..Default::default()
        };

        let _ = app.update(Message::Flash(FlashMsg::FlashManualRollbackCycleFormat));
        let (boot, vbmeta) = app.manual_rollback_buffers.clone().expect("buffers");
        assert_eq!(app.manual_rollback_format, RollbackValueFormat::Date);
        assert_eq!(boot, RollbackValueFormat::Date.render(1_700_000_000));
        assert_eq!(vbmeta, RollbackValueFormat::Date.render(1_600_000_000));

        let _ = app.update(Message::Flash(FlashMsg::FlashManualRollbackCycleFormat));
        let (boot, _) = app.manual_rollback_buffers.clone().expect("buffers");
        assert_eq!(app.manual_rollback_format, RollbackValueFormat::Raw);
        assert_eq!(boot, RollbackValueFormat::Raw.render(1_700_000_000));
    }

    #[test]
    fn manual_rollback_cycle_leaves_unparsable_text_alone() {
        let mut app = App {
            manual_rollback_format: RollbackValueFormat::Unix,
            manual_rollback_buffers: Some(("not-a-number".into(), "1600000000".into())),
            manual_rollback_values: (None, Some(1_600_000_000)),
            ..Default::default()
        };

        let _ = app.update(Message::Flash(FlashMsg::FlashManualRollbackCycleFormat));
        let (boot, vbmeta) = app.manual_rollback_buffers.clone().expect("buffers");
        assert_eq!(boot, "not-a-number");
        assert_eq!(vbmeta, RollbackValueFormat::Date.render(1_600_000_000));
    }

    #[test]
    fn manual_rollback_requires_valid_confirm() {
        install_core_translator(Language::En);
        let mut app = App {
            flash: FlashWizard {
                step: 4,
                firmware_folder: Some("firmware".into()),
                firmware_rollback_indices: Some((Ok(100), Ok(200))),
                ..FlashWizard::default()
            },
            wf_config: WorkflowConfig {
                modify_rollback: RollbackSetting::Off,
                ..WorkflowConfig::default()
            },
            confirm_edit_field: Some(ConfirmField::Rollback),
            ..App::default()
        };
        app.manual_rollback_editor = Some(ManualRollbackEditor::Boot);
        app.manual_rollback_buffers = Some(("99".to_string(), "199".to_string()));

        // A future target is valid decimal but fails the time gate.
        let future = current_unix_timestamp().map(|now| now + 1).unwrap_or(0);
        app.manual_rollback_buffers = Some((future.to_string(), "199".to_string()));
        let _ = app.update_flash(FlashMsg::FlashManualRollbackConfirm);
        assert_eq!(app.wf_config.modify_rollback, RollbackSetting::Off);
        assert_eq!(app.wf_config.manual_rollback_indices, None);

        let _ = app.update(Message::Flash(FlashMsg::FlashConfirmSetRollback(
            RollbackSetting::Manual,
        )));
        assert_eq!(app.wf_config.modify_rollback, RollbackSetting::Off);
        assert!(app.manual_rollback_editor.is_some());
        app.manual_rollback_buffers = Some(("0x63".to_string(), "0x199".to_string()));
        app.rollback_value_format = RollbackValueFormat::Unix;
        let _ = app.update_flash(FlashMsg::FlashManualRollbackConfirm);
        assert_ne!(app.wf_config.modify_rollback, RollbackSetting::Manual);
        assert_eq!(app.wf_config.manual_rollback_indices, None);

        app.rollback_value_format = RollbackValueFormat::Unix;
        app.manual_rollback_buffers = Some(("99".to_string(), "199".to_string()));
        let _ = app.update_flash(FlashMsg::FlashManualRollbackConfirm);
        assert_eq!(app.wf_config.modify_rollback, RollbackSetting::Manual);
        assert_eq!(
            app.wf_config.manual_rollback_indices,
            Some(ManualRollbackIndices {
                boot: 99,
                vbmeta_system: 199
            })
        );
        assert_eq!(app.confirm_edit_field, None);
        assert_eq!(app.manual_rollback_editor, None);
    }

    #[test]
    fn concise_error_summary_collapses_lines_and_truncates_unicode() {
        assert_eq!(
            concise_error_summary("\n  loader   handshake failed  \nfull detail", 80),
            "loader handshake failed"
        );
        assert_eq!(concise_error_summary("가나다라마바사", 5), "가나다라…");
    }

    #[test]
    fn shared_execution_failure_owns_error_presentation() {
        let mut app = App {
            error_msg: Some("failed".into()),
            operation_error: Some("failed".into()),
            ..App::default()
        };
        app.current_view = View::Flash;
        app.flash.step = FLASH_STEPS.len() - 1;
        assert!(!app.should_show_error_banner());

        app.current_view = View::Dashboard;
        assert!(app.should_show_error_banner());

        app.current_view = View::Advanced;
        app.adv_wizard.open(AdvAction::ImageInfo);
        app.adv_wizard.step = app.adv_wizard.exec_step();
        assert!(app.should_show_error_banner());
    }

    #[test]
    fn non_operation_error_on_execution_surface_stays_global() {
        let mut app = App {
            current_view: View::Flash,
            error_msg: Some("log save failed".into()),
            operation_error: None,
            ..App::default()
        };
        app.flash.step = FLASH_STEPS.len() - 1;

        assert!(app.should_show_error_banner());
    }

    #[test]
    fn operation_error_drives_shared_failure_status() {
        let app = App {
            operation_error: Some("firehose failed".into()),
            ..App::default()
        };

        assert_eq!(app.exec_status_copy().0, app.t("exec_failed_title"));
    }

    #[test]
    fn failed_operation_preserves_the_phase_that_failed() {
        let mut app = App {
            operation: OperationExecution::fixture(
                true,
                Some(View::Flash),
                vec![
                    OpStep {
                        label: "one".into(),
                    },
                    OpStep {
                        label: "two".into(),
                    },
                ],
                0,
                None,
            ),
            ..App::default()
        };

        app.fail_op();

        assert_eq!(app.operation.current_step(), 0);
        assert!(!app.operation.is_running());
        assert_eq!(app.operation.view(), None);
    }

    #[test]
    fn firmware_progress_steps_map_only_full_and_simple_flash() {
        assert!(OperationPhaseKind::Flash.is_firmware_progress_step(7));
        assert!(OperationPhaseKind::Flash.is_firmware_progress_step(8));
        assert!(OperationPhaseKind::SimpleFlash.is_firmware_progress_step(3));
        for kind in OperationPhaseKind::all() {
            if !matches!(
                kind,
                OperationPhaseKind::Flash | OperationPhaseKind::SimpleFlash
            ) {
                for step in 1..=kind.keys().len() {
                    assert!(!kind.is_firmware_progress_step(step), "{kind:?}, {step}");
                }
            }
        }
    }

    #[test]
    fn firmware_flash_progress_label_visibility_and_format() {
        let app = |kind: OperationPhaseKind, step: usize, busy: bool, err: Option<&str>| App {
            operation: OperationExecution::fixture(busy, None, Vec::new(), step, Some(kind)),
            flash_progress: Some(ltbox_device::edl::FlashProgress {
                partition: "super".into(),
                percent: 42,
                completed_bytes: 42,
                total_bytes: 100,
                operation_completed_bytes: 42,
                operation_total_bytes: 100,
            }),
            operation_error: err.map(str::to_string),
            ..App::default()
        };
        assert_eq!(
            app(OperationPhaseKind::Flash, 6, true, None)
                .firmware_flash_progress_label()
                .as_deref(),
            Some("super (42%)")
        );
        let mut simple = app(OperationPhaseKind::SimpleFlash, 2, true, None);
        simple.flash_progress = Some(ltbox_device::edl::FlashProgress {
            partition: "boot_a".into(),
            percent: 7,
            completed_bytes: 7,
            total_bytes: 100,
            operation_completed_bytes: 7,
            operation_total_bytes: 100,
        });
        assert_eq!(
            simple.firmware_flash_progress_label().as_deref(),
            Some("boot_a (7%)")
        );
        assert!(
            app(OperationPhaseKind::Flash, 5, true, None)
                .firmware_flash_progress_label()
                .is_none()
        );
        assert!(
            app(OperationPhaseKind::FlashPartitions, 1, true, None)
                .firmware_flash_progress_label()
                .is_none()
        );
        assert!(
            app(OperationPhaseKind::FlashPhysical, 2, true, None)
                .firmware_flash_progress_label()
                .is_none()
        );
        assert!(
            app(OperationPhaseKind::Root, 5, true, None)
                .firmware_flash_progress_label()
                .is_none()
        );
        assert!(
            app(OperationPhaseKind::Flash, 6, false, None)
                .firmware_flash_progress_label()
                .is_none()
        );
        assert!(
            app(OperationPhaseKind::Flash, 6, true, Some("boom"))
                .firmware_flash_progress_label()
                .is_none()
        );
    }

    #[test]
    fn flash_progress_clears_across_op_lifecycle() {
        type Transition = (bool, fn(&mut App));
        let transitions: [Transition; 5] = [
            (false, |a| a.begin_op(View::Flash)),
            (true, |a| a.end_op()),
            (true, |a| a.fail_op()),
            (false, |a| a.begin_silent_op(View::Root)),
            (true, |a| a.end_silent_op()),
        ];
        for (running, clear) in transitions {
            let mut app = App::default();
            if running {
                let _ = app.begin_phased_op(View::Flash, OperationPhaseKind::Flash);
            }
            app.flash_progress = Some(ltbox_device::edl::FlashProgress {
                partition: "super".into(),
                percent: 10,
                completed_bytes: 10,
                total_bytes: 100,
                operation_completed_bytes: 10,
                operation_total_bytes: 100,
            });
            clear(&mut app);
            assert!(app.flash_progress.is_none());
            assert_eq!(app.operation.phase_kind(), None);
        }
    }

    #[test]
    fn firmware_write_phase_labels_use_progress_wording() {
        let expected = [
            (Language::En, "Flashing firmware"),
            (Language::Ko, "펌웨어 플래싱 진행"),
            (Language::Zh, "正在刷写固件"),
            (Language::Ru, "Прошивка устройства"),
            (Language::Ja, "ファームウェアをフラッシュ中"),
        ];
        for (lang, label) in expected {
            let translations = Translations::load(lang);
            assert_eq!(translations.t("op_flash_phase_7"), label);
            assert_eq!(translations.t("op_simple_phase_write"), label);
        }
    }

    #[test]
    fn shared_execution_error_is_inline_instead_of_floating() {
        let exec = include_str!("view/sysupdate.rs");
        assert!(exec.contains("concise_error_summary"));
        assert!(exec.contains("m3_log_text_field_with_action"));

        let chrome = include_str!("view/chrome.rs");
        assert!(chrome.contains("should_show_error_banner"));
    }

    #[test]
    fn action_bar_buttons_have_visible_centered_labels() {
        let source = include_str!("widgets.rs");
        let implementation = source
            .split_once("fn action_button")
            .expect("labeled action-button helper must exist")
            .1
            .split_once("pub(crate) fn wizard_secondary_action")
            .expect("secondary action helper must follow the shared button")
            .0;
        assert!(
            implementation.contains(".center_y(Length::Fill)"),
            "action-bar button content must be centered vertically"
        );
        assert!(
            implementation.contains("text(label)"),
            "every action-bar button must render its label"
        );
    }

    #[test]
    fn exec_action_layout_keeps_one_primary_action() {
        assert_eq!(
            exec_action_layout(true, false, false),
            ExecActionLayout {
                primary: None,
                start_over_utility: false,
            }
        );
        assert_eq!(
            exec_action_layout(false, false, false),
            ExecActionLayout {
                primary: Some(ExecPrimaryAction::StartOver),
                start_over_utility: false,
            }
        );
        assert_eq!(
            exec_action_layout(false, false, true),
            ExecActionLayout {
                primary: Some(ExecPrimaryAction::OpenFolder),
                start_over_utility: true,
            }
        );
        assert_eq!(
            exec_action_layout(false, true, true),
            ExecActionLayout {
                primary: Some(ExecPrimaryAction::StartOver),
                start_over_utility: false,
            }
        );
    }

    #[test]
    fn busy_operation_label_names_advanced_subtask() {
        let mut app = App {
            operation: OperationExecution::fixture(true, Some(View::Advanced), Vec::new(), 0, None),
            current_view: View::Advanced,
            ..App::default()
        };

        app.adv_wizard.action = Some(AdvAction::PatchDevinfo);
        assert_eq!(
            app.busy_operation_label(),
            app.t(AdvAction::PatchDevinfo.label_key()).to_string()
        );

        app.advanced_wizard_open = AdvancedWizardOpen::FlashParts;
        assert_eq!(
            app.busy_operation_label(),
            app.t(AdvAction::FlashPartitions.label_key()).to_string()
        );

        app.end_silent_op();
        app.begin_silent_op(View::Reboot);
        assert_eq!(app.busy_operation_label(), app.t("nav_reboot").to_string());
    }

    #[test]
    fn busy_navigation_target_requires_a_live_operation() {
        assert_eq!(
            busy_navigation_target(true, Some(View::Flash)),
            Some(View::Flash)
        );
        assert_eq!(busy_navigation_target(false, Some(View::Flash)), None);
        assert_eq!(busy_navigation_target(true, None), None);
    }

    #[test]
    fn dashboard_offers_resume_only_while_an_operation_runs() {
        // The idle "no operation" card is gone, but the running state still
        // has to offer a way back into the flow the user navigated away from,
        // behind the same guard.
        let source = include_str!("view/dashboard.rs");
        assert!(source.contains("Message::ResumeBusyOperation"));
        assert!(source.contains(
            "busy_navigation_target(self.operation.is_running(), self.operation.view()).is_some()"
        ));
        assert!(!source.contains("dash_no_operation"));
    }

    #[test]
    fn dashboard_open_operation_label_exists_in_every_locale() {
        let en = Translations::load(Language::En);
        assert!(en.fallback.contains_key("dash_open_operation"));
        for lang in [Language::Ko, Language::Zh, Language::Ru, Language::Ja] {
            let translations = Translations::load(lang);
            assert!(translations.primary.contains_key("dash_open_operation"));
        }
    }

    #[test]
    fn loader_file_check_is_extension_based() {
        assert!(is_loader_file(std::path::Path::new("xbl_anything.melf")));
        assert!(is_loader_file(std::path::Path::new("firehose_loader.MBN")));
        assert!(is_loader_file(std::path::Path::new("prog.elf")));
        assert!(!is_loader_file(std::path::Path::new("xbl_s_devprg_ns.bin")));
    }

    #[test]
    fn disconnected_loader_picker_accepts_melf_and_xml() {
        let mut app = App::default();
        let melf = std::path::Path::new("xbl_s_devprg_ns.melf");
        let xml = std::path::Path::new("qsahara_device_programmer.xml");

        assert_eq!(app.loader_picker_exts(), &["melf", "xml"]);
        assert!(app.loader_fits_model(melf));
        assert!(app.loader_fits_model(xml));
        assert_eq!(
            app.loader_picker_subtitle(),
            app.t("loader_picker_subtitle_unknown")
        );

        app.device.connection = ConnectionStatus::Adb;
        app.device.model = "TB520FU".into();
        assert_eq!(app.loader_picker_exts(), &["melf"]);
        assert!(app.loader_fits_model(melf));
        assert!(!app.loader_fits_model(xml));
        assert_eq!(
            app.loader_picker_subtitle(),
            app.t("loader_picker_subtitle_standard")
        );

        app.device.model = "TB323FU".into();
        assert_eq!(app.loader_picker_exts(), &["xml"]);
        assert!(!app.loader_fits_model(melf));
        assert!(app.loader_fits_model(xml));
        assert_eq!(
            app.loader_picker_subtitle(),
            app.t("loader_picker_subtitle_manifest")
        );
    }

    #[test]
    fn edl_entry_action_uses_adb_from_fastboot() {
        assert_eq!(
            edl_entry_action(ConnectionStatus::Fastboot),
            EdlEntryAction::FastbootRebootThenAdb
        );
    }

    #[test]
    fn edl_entry_action_waits_manual_without_usable_adb() {
        assert_eq!(
            edl_entry_action(ConnectionStatus::AdbUnauthorized),
            EdlEntryAction::ManualWait
        );
    }

    #[test]
    fn country_patch_progress_requires_all_expected() {
        install_core_translator(Language::En);
        let mut progress = CountryPatchProgress::new(&["devinfo", "persist"]);
        progress.mark_flashed("devinfo");

        let err = progress.finish().expect_err("persist must be required");
        assert!(err.contains("persist"));
    }

    #[test]
    fn country_patch_progress_oemowninfo_expected() {
        // TB320FC / TB323FU patch oemowninfo instead of devinfo.
        let mut progress = CountryPatchProgress::new(&["oemowninfo", "persist"]);
        progress.mark_flashed("oemowninfo");
        progress.mark_flashed("persist");
        assert!(progress.finish().is_ok());
    }

    #[test]
    fn country_patch_progress_surfaces_partition_failures() {
        install_core_translator(Language::En);
        let mut progress = CountryPatchProgress::new(&["devinfo", "persist"]);
        progress.mark_flashed("devinfo");
        progress.mark_failed("persist", "no known country code");

        let err = progress
            .finish()
            .expect_err("recorded persist failure must fail workflow");
        assert!(err.contains("persist: no known country code"));
    }
}
