//! Persisted user settings (language, theme, recents).
//!
//! Lives in the user's config dir (outside the install tree so
//! replacing `ltbox.exe` keeps preferences).

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const APP_DIR: &str = "ltbox";
const FILE_NAME: &str = "settings.json";

/// Maximum number of recent paths to remember per category.
pub const RECENT_MAX: usize = 3;

/// Legacy global-files bucket key for migration from pre-category config.
/// v2 / early-v3 settings only had `files: Vec<String>` + `folders: Vec<String>`;
/// rather than throw the history away, we bin it into these stable keys so
/// the user still sees their last-used paths somewhere — they can pick the
/// actual category bucket next time they Browse.
pub const LEGACY_FILES_KEY: &str = "legacy.files";
pub const LEGACY_FOLDERS_KEY: &str = "legacy.folders";

/// Per-category MRU path lists. Category keys are stable strings (see
/// `PickerKind::storage_key` in pickers.rs) so the JSON roundtrips without
/// coupling persistence to enum Variant ordering.
///
/// `BTreeMap` (not `HashMap`) for deterministic JSON output — diffing the
/// settings file for troubleshooting is far easier when key order doesn't
/// jitter between saves.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecentPaths {
    #[serde(default)]
    pub by_kind: BTreeMap<String, Vec<String>>,

    // ---- Legacy fields kept ONLY for load-migration ----------------------
    // Old config had these as top-level arrays. `#[serde(default)]` lets
    // newer builds still read them; `migrate_legacy()` folds them into
    // `by_kind` after load. On the next save the legacy arrays emit as
    // empty (skip on empty below) so the file gradually self-cleans.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub folders: Vec<String>,
}

impl RecentPaths {
    /// Push a path onto the MRU list for `kind`. Returns `true` iff the
    /// list changed (useful to skip redundant settings writes).
    pub fn push(&mut self, kind: &str, path: &str) -> bool {
        if kind.is_empty() {
            return false;
        }
        let list = self.by_kind.entry(kind.to_string()).or_default();
        push_front_dedup(list, path)
    }

    /// MRU list for `kind`, or empty slice if none.
    pub fn recent(&self, kind: &str) -> &[String] {
        self.by_kind.get(kind).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Most-recent entry for `kind` — handy as a `rfd` starting dir.
    pub fn most_recent(&self, kind: &str) -> Option<&str> {
        self.by_kind
            .get(kind)
            .and_then(|v| v.first())
            .map(String::as_str)
    }

    /// Fold legacy `files` / `folders` arrays into the kind map. Idempotent.
    pub fn migrate_legacy(&mut self) {
        for p in std::mem::take(&mut self.files) {
            let _ = self.push(LEGACY_FILES_KEY, &p);
        }
        for p in std::mem::take(&mut self.folders) {
            let _ = self.push(LEGACY_FOLDERS_KEY, &p);
        }
    }
}

fn push_front_dedup(list: &mut Vec<String>, path: &str) -> bool {
    if path.is_empty() {
        return false;
    }
    let before = list.clone();
    list.retain(|p| p != path);
    list.insert(0, path.to_string());
    if list.len() > RECENT_MAX {
        list.truncate(RECENT_MAX);
    }
    list != &before
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedSettings {
    #[serde(default = "default_language")]
    pub language: String,
    /// "system" | "light" | "dark". Blank in old configs — loader
    /// upgrades via the legacy `dark_mode` field below.
    #[serde(default = "default_theme")]
    pub theme: String,
    /// Material color seed. Defaults to the original indigo palette.
    #[serde(default = "default_theme_seed")]
    pub theme_seed: String,
    /// Use iced's operating-system default font instead of the bundled Noto
    /// family. The renderer binds its default font at startup, so changes take
    /// effect on the next application launch.
    #[serde(default)]
    pub use_system_font: bool,
    /// Legacy flag kept for upgrade compatibility. `theme` is the
    /// source of truth for new saves.
    #[serde(default)]
    pub dark_mode: bool,
    #[serde(default)]
    pub recent_paths: RecentPaths,
    /// Optional default EDL loader (`xbl_s_devprg_ns.melf`) path. When
    /// set, every wizard / Reboot-to-EDL flow auto-fills this path
    /// instead of opening the file picker. Single-device users skip the
    /// picker on every loader prompt; the file is still re-validated at
    /// exec start so a deleted/moved loader surfaces as an error before
    /// the wizard kicks off the device side. `None` = picker shows as
    /// before.
    #[serde(default)]
    pub default_loader_path: Option<String>,
    /// Qualcomm USB driver family: "userspace" or "kernel". Defaults to
    /// "kernel" on Windows and Linux, "userspace" elsewhere (see
    /// [`default_qcom_driver_mode`]). Unknown / missing values are normalized
    /// by the GUI when loaded.
    #[serde(default = "default_qcom_driver_mode")]
    pub qcom_driver_mode: String,
    /// Last window size (logical pixels) recorded on resize. Restored on
    /// next launch so the user's preferred geometry survives restarts.
    /// `None` on first run → the default 820×720 in `main` applies.
    #[serde(default)]
    pub window_size: Option<(f32, f32)>,
    /// User dismissed the optional Qualcomm USB driver *update* prompt via
    /// "don't show again". Suppresses the startup version check + update
    /// banner from here on. Does NOT affect the missing-driver install
    /// banner, which always shows when the driver is absent.
    #[serde(default)]
    pub qcom_driver_update_dismissed: bool,
    /// Models for which the user chose "don't show again" on the dual-USB-C
    /// port advisory. Per-model, so a different dual-port model still shows
    /// the advisory once.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dual_usb_advisory_dismissed_models: Vec<String>,
}

fn default_language() -> String {
    "en".to_string()
}

/// Map an OS locale tag (e.g. `ko-KR`, `zh-Hant-TW`, `en-US`) to a UI
/// language code LTBox ships. Every Chinese variant maps to `zh`; Korean,
/// Russian, and Japanese map to their matching UI language. Anything else
/// falls back to English.
fn ui_lang_for_locale(locale: &str) -> &'static str {
    // Compare only the BCP-47 / POSIX primary language subtag (`ko-KR`,
    // `ko_KR.UTF-8`, `zh-Hant-TW`), so neighbours like Konkani (`kok`) or
    // Zhuang (`zha`) aren't mistaken for Korean / Chinese.
    let primary = locale
        .split(['-', '_', '.', '@'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    match primary.as_str() {
        "ko" => "ko",
        "zh" => "zh",
        "ru" => "ru",
        "ja" => "ja",
        _ => "en",
    }
}

/// The host OS UI locale mapped to a shipped language, or `en` when the locale
/// is unavailable / unsupported. Used only on first run (no saved settings).
fn detect_os_language() -> String {
    sys_locale::get_locale()
        .map(|l| ui_lang_for_locale(&l))
        .unwrap_or("en")
        .to_string()
}

fn default_theme() -> String {
    String::new()
}

fn default_theme_seed() -> String {
    "indigo".to_string()
}

/// Default Qualcomm USB driver family.
///
/// Defaults to `"kernel"` where the kernel driver is a viable default — Windows
/// (signed kernel driver) and Debian-style Linux (`dpkg-query` present, so the
/// `qud` package install path works). Other Linux distros and macOS have no
/// usable kernel-driver path, so they default to `"userspace"`/udev. See
/// [`ltbox_device::driver::kernel_mode_supported`].
fn default_qcom_driver_mode() -> String {
    if ltbox_device::driver::kernel_mode_supported() {
        "kernel".to_string()
    } else {
        "userspace".to_string()
    }
}

impl Default for PersistedSettings {
    fn default() -> Self {
        Self {
            language: default_language(),
            theme: "system".to_string(),
            theme_seed: default_theme_seed(),
            use_system_font: false,
            dark_mode: false,
            recent_paths: RecentPaths::default(),
            default_loader_path: None,
            qcom_driver_mode: default_qcom_driver_mode(),
            window_size: None,
            qcom_driver_update_dismissed: false,
            dual_usb_advisory_dismissed_models: Vec::new(),
        }
    }
}

fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(APP_DIR).join(FILE_NAME))
}

/// Load settings. On missing / malformed / no config dir, returns
/// [`first_run_default`] — the default with the UI language seeded from the
/// OS locale (Korean / Chinese, else English).
///
/// Legacy `recent_paths.files` / `recent_paths.folders` arrays from
/// pre-per-category builds are folded into `by_kind` under
/// [`LEGACY_FILES_KEY`] / [`LEGACY_FOLDERS_KEY`] — no history loss when
/// upgrading.
pub fn load() -> PersistedSettings {
    let Some(path) = config_path() else {
        return first_run_default();
    };
    let Ok(data) = std::fs::read_to_string(&path) else {
        return first_run_default();
    };
    let mut settings: PersistedSettings =
        serde_json::from_str(&data).unwrap_or_else(|_| first_run_default());
    settings.recent_paths.migrate_legacy();
    settings
}

/// First-run default — like [`PersistedSettings::default`] but with the UI
/// language seeded from the OS locale so a fresh install opens in the user's
/// language without a manual pick. A saved config (even one missing the
/// `language` key) is respected as-is; only the no-saved-settings paths hit
/// this.
fn first_run_default() -> PersistedSettings {
    PersistedSettings {
        language: detect_os_language(),
        ..PersistedSettings::default()
    }
}

/// Persist settings without exposing a partial JSON file. Report failures in
/// the log while allowing the GUI to continue with its in-memory settings.
pub fn save(settings: &PersistedSettings) {
    let Some(path) = config_path() else {
        tracing::warn!("cannot save settings: no configuration directory is available");
        return;
    };
    if let Err(error) = save_to_path(&path, settings) {
        tracing::warn!(path = %path.display(), %error, "failed to save settings");
    }
}

fn save_to_path(path: &Path, settings: &PersistedSettings) -> io::Result<()> {
    // Serialize first so a serialization failure cannot touch the saved file.
    let json = serde_json::to_vec_pretty(settings).map_err(io::Error::other)?;
    atomic_write_with(path, |file| file.write_all(&json))
}

fn atomic_write_with(
    path: &Path,
    write: impl FnOnce(&mut std::fs::File) -> io::Result<()>,
) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    // A unique sibling keeps replacement on the same filesystem. RAII removes
    // only the temporary file on write/sync/replace failure, never the original.
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    write(temporary.as_file_mut())?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
#[path = "settings_store_tests.rs"]
mod persistence_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_follow_system() {
        let s = PersistedSettings::default();
        assert_eq!(s.language, "en");
        assert_eq!(s.theme, "system");
        assert_eq!(s.theme_seed, "indigo");
        assert!(!s.use_system_font);
        assert!(!s.dark_mode);
    }

    #[test]
    fn os_locale_maps_to_ui_language() {
        // Korean (BCP-47 + POSIX forms).
        for l in ["ko-KR", "ko", "ko_KR.UTF-8", "KO"] {
            assert_eq!(ui_lang_for_locale(l), "ko", "{l}");
        }
        // Chinese variants all collapse to zh.
        for l in [
            "zh-CN",
            "zh-TW",
            "zh-HK",
            "zh-Hans",
            "zh-Hant-TW",
            "zh_CN.UTF-8",
            "ZH",
        ] {
            assert_eq!(ui_lang_for_locale(l), "zh", "{l}");
        }
        // Russian and Japanese.
        for l in ["ru-RU", "ru", "ru_RU.UTF-8", "RU"] {
            assert_eq!(ui_lang_for_locale(l), "ru", "{l}");
        }
        for l in ["ja-JP", "ja", "ja_JP.UTF-8", "JA"] {
            assert_eq!(ui_lang_for_locale(l), "ja", "{l}");
        }
        // Everything else → English — including neighbours that merely share a
        // prefix (Konkani `kok`, Zhuang `zha`).
        for l in ["en-US", "de", "kok-IN", "zha-CN", ""] {
            assert_eq!(ui_lang_for_locale(l), "en", "{l}");
        }
    }

    #[test]
    fn partial_json_fills_defaults() {
        let s: PersistedSettings = serde_json::from_str(r#"{"dark_mode": true}"#).unwrap();
        assert_eq!(s.language, "en");
        assert_eq!(s.theme, "");
        assert_eq!(s.theme_seed, "indigo");
        assert_eq!(s.qcom_driver_mode, default_qcom_driver_mode());
        assert!(!s.use_system_font);
        assert!(s.dark_mode);
    }

    #[test]
    fn theme_field_roundtrips() {
        let s: PersistedSettings =
            serde_json::from_str(r#"{"theme": "dark", "theme_seed": "teal"}"#).unwrap();
        assert_eq!(s.theme, "dark");
        assert_eq!(s.theme_seed, "teal");
    }

    #[test]
    fn system_font_field_roundtrips() {
        let s: PersistedSettings = serde_json::from_str(r#"{"use_system_font": true}"#).unwrap();
        assert!(s.use_system_font);
        let json = serde_json::to_string(&s).unwrap();
        let stored: PersistedSettings = serde_json::from_str(&json).unwrap();
        assert!(stored.use_system_font);
    }

    #[test]
    fn push_per_kind_independent() {
        let mut r = RecentPaths::default();
        assert!(r.push("loader_folder", "/a"));
        assert!(r.push("qfil_firmware", "/b"));
        assert_eq!(r.recent("loader_folder"), &["/a".to_string()]);
        assert_eq!(r.recent("qfil_firmware"), &["/b".to_string()]);
        assert!(r.recent("other").is_empty());
    }

    #[test]
    fn push_dedups_and_truncates() {
        let mut r = RecentPaths::default();
        for p in ["/a", "/b", "/c", "/d", "/a"] {
            r.push("k", p);
        }
        // Trace: [/a] → [/b,/a] → [/c,/b,/a] → [/d,/c,/b] (cap=3 drops
        // /a) → [/a,/d,/c] (/a re-enters at front as a fresh item).
        // Dedup only kicks in while the entry is still inside the cap.
        assert_eq!(r.recent("k"), &["/a", "/d", "/c"]);
    }

    #[test]
    fn push_dedups_within_cap() {
        let mut r = RecentPaths::default();
        for p in ["/a", "/b", "/c", "/a"] {
            r.push("k", p);
        }
        // Same as above but /a still present when re-pushed — moves to
        // front, /b /c shift, no new slot consumed.
        assert_eq!(r.recent("k"), &["/a", "/c", "/b"]);
    }

    #[test]
    fn most_recent_returns_top() {
        let mut r = RecentPaths::default();
        r.push("k", "/x");
        r.push("k", "/y");
        assert_eq!(r.most_recent("k"), Some("/y"));
        assert_eq!(r.most_recent("empty"), None);
    }

    #[test]
    fn migrate_legacy_moves_flat_arrays_into_buckets() {
        // Old config schema as on-disk JSON.
        let json = r#"{
            "language": "en",
            "theme": "system",
            "recent_paths": {
                "files": ["/f1", "/f2"],
                "folders": ["/d1"]
            }
        }"#;
        let mut s: PersistedSettings = serde_json::from_str(json).unwrap();
        s.recent_paths.migrate_legacy();

        // Legacy arrays drained.
        assert!(s.recent_paths.files.is_empty());
        assert!(s.recent_paths.folders.is_empty());
        // History preserved under the legacy bucket keys.
        assert_eq!(s.recent_paths.recent(LEGACY_FILES_KEY), &["/f2", "/f1"]);
        assert_eq!(s.recent_paths.recent(LEGACY_FOLDERS_KEY), &["/d1"]);
    }

    #[test]
    fn empty_kind_is_rejected() {
        let mut r = RecentPaths::default();
        assert!(!r.push("", "/x"));
        assert!(!r.push("k", ""));
    }
}
