//! Persisted user settings (language, theme, recents, loader memory).
//!
//! Lives in the user's config dir, outside the install tree, so replacing
//! `ltbox.exe` keeps preferences.

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[cfg(not(test))]
const APP_DIR: &str = "ltbox";
#[cfg(not(test))]
const FILE_NAME: &str = "settings.json";

/// Maximum visible recents; stored per folder category or file extension.
pub const RECENT_MAX: usize = 5;

/// Legacy bucket keys for the pre-category `files` / `folders` arrays: `migrate_legacy`
/// bins that history under these keys so the last-used paths survive the upgrade.
pub const LEGACY_FILES_KEY: &str = "legacy.files";
pub const LEGACY_FOLDERS_KEY: &str = "legacy.folders";

/// Per-category MRU path lists. Category keys are stable strings (see
/// `PickerKind::storage_key` in pickers.rs) so the JSON does not depend on enum
/// variant order.
///
/// `BTreeMap` (not `HashMap`) keeps the JSON key order deterministic across saves.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecentPaths {
    #[serde(default)]
    pub by_kind: BTreeMap<String, Vec<String>>,
    /// Last user selection, in milliseconds since the Unix epoch. Missing
    /// timestamps are legacy entries; their stored relative order is retained.
    #[serde(default)]
    pub selected_at_ms: BTreeMap<String, u64>,

    // ---- Legacy fields kept ONLY for load-migration ----------------------
    // Old configs had these as top-level arrays. `migrate_legacy()` folds them into
    // `by_kind` after load, and they are skipped on save once empty, so the file
    // gradually self-cleans.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub folders: Vec<String>,
}

impl RecentPaths {
    /// Push a path onto the MRU list for `kind`. Returns `true` iff the
    /// list changed (useful to skip redundant settings writes).
    pub fn push(&mut self, kind: &str, path: &str) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(u64::MAX as u128) as u64;
        self.push_at(kind, path, now)
    }

    fn push_at(&mut self, kind: &str, path: &str, timestamp: u64) -> bool {
        if kind.is_empty() || path.is_empty() {
            return false;
        }
        let timestamp_changed =
            self.selected_at_ms.insert(path.to_owned(), timestamp) != Some(timestamp);
        let list = self.by_kind.entry(kind.to_string()).or_default();
        let changed = if kind == "file" {
            push_file_recent(list, path)
        } else {
            push_front_dedup(list, path)
        };
        self.selected_at_ms
            .retain(|path, _| self.by_kind.values().any(|paths| paths.contains(path)));
        changed || timestamp_changed
    }

    /// Merge the caller's eligible paths by selection time, then limit the
    /// combined list. Stable sorting preserves legacy order and timestamp ties.
    pub fn visible<'a>(&self, items: &'a [String]) -> Vec<&'a String> {
        let mut paths: Vec<_> = items.iter().collect();
        paths.sort_by_key(|path| {
            std::cmp::Reverse(self.selected_at_ms.get(*path).copied().unwrap_or(0))
        });
        paths.truncate(RECENT_MAX);
        paths
    }

    /// MRU list for `kind`, or empty slice if none.
    pub fn recent(&self, kind: &str) -> &[String] {
        self.by_kind.get(kind).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Fold legacy `files` / `folders` arrays into the kind map. Idempotent.
    pub fn migrate_legacy(&mut self) {
        for p in std::mem::take(&mut self.files) {
            push_front_dedup(self.by_kind.entry(LEGACY_FILES_KEY.into()).or_default(), &p);
        }
        for p in std::mem::take(&mut self.folders) {
            push_front_dedup(
                self.by_kind.entry(LEGACY_FOLDERS_KEY.into()).or_default(),
                &p,
            );
        }
    }
}

// File pickers filter by extension after loading. Keep up to `RECENT_MAX`
// entries per extension so unrelated APK/module picks cannot evict EDL loaders
// or images.
fn push_file_recent(list: &mut Vec<String>, path: &str) -> bool {
    if path.is_empty() {
        return false;
    }
    let before = list.clone();
    list.retain(|p| p != path);
    list.insert(0, path.to_owned());
    let mut counts = BTreeMap::new();
    list.retain(|p| {
        let extension = Path::new(p)
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
        let count = counts.entry(extension).or_insert(0);
        *count += 1;
        *count <= RECENT_MAX
    });
    *list != before
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
    /// "system" | "light" | "dark". Blank in old configs; the GUI then falls back to
    /// the legacy `dark_mode` field below.
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
    /// Remember the EDL loader each model was last flashed with, and reuse it
    /// automatically. On by default.
    #[serde(default = "default_remember_edl_loader")]
    pub remember_edl_loader: bool,
    /// Model name (upper-case) → the loader path that model last completed a
    /// Sahara upload with. Only written after an upload actually succeeded, so
    /// an entry is evidence the loader works on that model, not a guess.
    ///
    /// `BTreeMap` for deterministic JSON, matching [`RecentPaths::by_kind`].
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub remembered_edl_loaders: BTreeMap<String, String>,
    /// Qualcomm USB driver family: "userspace" or "kernel". The default follows
    /// [`default_qcom_driver_mode`]. Unknown / missing values are normalized by the GUI
    /// when loaded.
    #[serde(default = "default_qcom_driver_mode")]
    pub qcom_driver_mode: String,
    /// Last window size (logical pixels), recorded on resize and restored on the next
    /// launch. `None` on first run: the default 820×720 in `main` applies.
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
/// Russian, Japanese, and French map to their matching UI language. Anything else
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
        "fr" => "fr",
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

/// EDL-loader memory is on by default: most users have one or two devices and would
/// otherwise re-pick the same file on every operation.
fn default_remember_edl_loader() -> bool {
    true
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
            remember_edl_loader: default_remember_edl_loader(),
            remembered_edl_loaders: BTreeMap::new(),
            qcom_driver_mode: default_qcom_driver_mode(),
            window_size: None,
            qcom_driver_update_dismissed: false,
            dual_usb_advisory_dismissed_models: Vec::new(),
        }
    }
}

#[cfg(not(test))]
fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(APP_DIR).join(FILE_NAME))
}

// GUI handler tests instantiate App and trigger saves. They must never read
// or overwrite the user's settings shared by installed and debug builds.
// Persistence tests exercise explicit temporary paths through save_to_path.
#[cfg(test)]
fn config_path() -> Option<PathBuf> {
    None
}

/// Load settings. On missing / malformed / no config dir, returns
/// [`first_run_default`]; a malformed file is first renamed to `<name>.bad-<unix secs>`.
///
/// Legacy `recent_paths.files` / `recent_paths.folders` arrays are folded into `by_kind`
/// under [`LEGACY_FILES_KEY`] / [`LEGACY_FOLDERS_KEY`].
pub fn load() -> PersistedSettings {
    let Some(path) = config_path() else {
        return first_run_default();
    };
    load_from(&path)
}

fn load_from(path: &Path) -> PersistedSettings {
    let Ok(data) = std::fs::read_to_string(path) else {
        return first_run_default();
    };
    let mut settings: PersistedSettings = match serde_json::from_str(&data) {
        Ok(settings) => settings,
        Err(error) => {
            let seconds = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            if let Some(name) = path.file_name() {
                let mut backup_name = name.to_os_string();
                backup_name.push(format!(".bad-{seconds}"));
                let _ = std::fs::rename(path, path.with_file_name(backup_name));
            }
            tracing::warn!(%error, "failed to parse settings");
            return first_run_default();
        }
    };
    settings.recent_paths.migrate_legacy();
    settings
}

/// First-run default: [`PersistedSettings::default`] with the UI language seeded from
/// the OS locale. A saved config (even one missing the `language` key) is respected
/// as-is; only the no-saved-settings paths use this.
fn first_run_default() -> PersistedSettings {
    PersistedSettings {
        language: detect_os_language(),
        ..PersistedSettings::default()
    }
}

enum SaveRequest {
    Save(Box<PersistedSettings>),
    Flush(std::sync::mpsc::Sender<()>),
}
static WRITER: std::sync::OnceLock<Option<std::sync::mpsc::Sender<SaveRequest>>> =
    std::sync::OnceLock::new();

/// Queue snapshots in order; disk synchronization never blocks UI updates.
pub fn save(settings: &PersistedSettings) {
    let writer = WRITER.get_or_init(|| {
        let (sender, receiver) = std::sync::mpsc::channel();
        match std::thread::Builder::new()
            .name("settings-writer".into())
            .spawn(move || {
                while let Ok(request) = receiver.recv() {
                    match request {
                        SaveRequest::Save(settings) => {
                            if let Some(path) = config_path() {
                                if let Err(error) = save_to_path(&path, &settings) {
                                    tracing::warn!(%error, "failed to save settings");
                                }
                            } else {
                                tracing::warn!("no configuration directory available");
                            }
                        }
                        SaveRequest::Flush(done) => {
                            let _ = done.send(());
                        }
                    }
                }
            }) {
            Ok(_) => Some(sender),
            Err(error) => {
                tracing::warn!(%error, "cannot start settings writer");
                None
            }
        }
    });
    if let Some(writer) = writer {
        let _ = writer.send(SaveRequest::Save(Box::new(settings.clone())));
    }
}

/// Drain queued saves after the GUI event loop exits.
pub fn flush() {
    if let Some(Some(writer)) = WRITER.get() {
        let (sender, receiver) = std::sync::mpsc::channel();
        if writer.send(SaveRequest::Flush(sender)).is_ok() {
            let _ = receiver.recv();
        }
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
    fn malformed_settings_are_preserved_and_defaults_returned() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let invalid = "{invalid json";
        std::fs::write(&path, invalid).unwrap();
        let loaded = load_from(&path);
        assert_eq!(
            serde_json::to_value(loaded).unwrap(),
            serde_json::to_value(first_run_default()).unwrap()
        );
        assert!(!path.exists());
        let backups: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(backups.len(), 1);
        let suffix = backups[0]
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .strip_prefix("settings.json.bad-")
            .unwrap();
        assert!(suffix.parse::<u64>().is_ok());
        assert_eq!(std::fs::read_to_string(&backups[0]).unwrap(), invalid);
    }

    #[test]
    fn valid_settings_are_left_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let settings = PersistedSettings {
            language: "fr".into(),
            ..PersistedSettings::default()
        };
        let data = serde_json::to_string(&settings).unwrap();
        std::fs::write(&path, &data).unwrap();
        assert_eq!(load_from(&path).language, "fr");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), data);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn eligible_recents_merge_by_selection_time_after_reload() {
        let mut recents = RecentPaths::default();
        recents.push_at("file", "kernel.zip", 100);
        recents.push_at("file", "boot.img", 300);
        recents.push_at("file", "other.ZIP", 200);
        recents.push_at("file", "init_boot.img", 400);
        recents.push_at("file", "manager.apk", 500);
        let mut loaded: RecentPaths =
            serde_json::from_str(&serde_json::to_string(&recents).unwrap()).unwrap();
        let eligible = |r: &RecentPaths| {
            r.recent("file")
                .iter()
                .filter(|p| crate::pickers::path_matches_extensions(p, &["img", "zip"]))
                .cloned()
                .collect::<Vec<_>>()
        };
        assert_eq!(
            loaded.visible(&eligible(&loaded)),
            vec!["init_boot.img", "boot.img", "other.ZIP", "kernel.zip"]
        );
        loaded.push_at("file", "kernel.zip", 600);
        assert_eq!(
            loaded.visible(&eligible(&loaded)),
            vec!["kernel.zip", "init_boot.img", "boot.img", "other.ZIP"]
        );
        // Re-selecting the first entry must still update its persisted time.
        assert!(loaded.push_at("file", "kernel.zip", 700));
        assert_eq!(loaded.selected_at_ms["kernel.zip"], 700);
    }

    #[test]
    fn timestamp_free_recents_keep_legacy_order_and_folder_limit() {
        let mut r: RecentPaths = serde_json::from_str(r#"{"by_kind":{"file":["a.zip","b.img","c.zip","d.img"],"output_folder":["new","old"]}}"#).unwrap();
        r.migrate_legacy();
        assert!(r.selected_at_ms.is_empty());
        assert_eq!(
            r.visible(r.recent("file")),
            vec!["a.zip", "b.img", "c.zip", "d.img"]
        );
        r.push_at("output_folder", "old", 10);
        assert_eq!(r.visible(r.recent("output_folder")), vec!["old", "new"]);
    }

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
        // French, including regional variants.
        for l in ["fr-FR", "fr-CA", "fr-BE", "fr", "fr_FR.UTF-8", "FR"] {
            assert_eq!(ui_lang_for_locale(l), "fr", "{l}");
        }
        // Everything else → English — including neighbours that merely share a
        // prefix (Konkani `kok`, Zhuang `zha`).
        for l in ["en-US", "de", "fur-IT", "kok-IN", "zha-CN", ""] {
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
    fn loader_memory_defaults_on_and_starts_empty() {
        let s: PersistedSettings = serde_json::from_str(r#"{}"#).unwrap();
        assert!(s.remember_edl_loader);
        assert!(s.remembered_edl_loaders.is_empty());
        assert!(PersistedSettings::default().remember_edl_loader);
    }

    #[test]
    fn a_settings_file_from_the_default_loader_era_still_loads() {
        // `default_loader_path` had no model attached, so there is nothing to
        // carry over: the field is simply ignored and the memory starts empty
        // with the feature on. Everything else in the old file survives.
        let legacy = r#"{
            "language": "ko",
            "default_loader_path": "D:\\fw\\xbl_s_devprg_ns.melf"
        }"#;
        let s: PersistedSettings = serde_json::from_str(legacy).unwrap();
        assert_eq!(s.language, "ko");
        assert!(s.remember_edl_loader);
        assert!(s.remembered_edl_loaders.is_empty());
    }

    #[test]
    fn loader_memory_roundtrips_and_is_omitted_when_empty() {
        let mut s = PersistedSettings::default();
        assert!(
            !serde_json::to_string(&s)
                .unwrap()
                .contains("remembered_edl_loaders")
        );

        s.remember_edl_loader = false;
        s.remembered_edl_loaders
            .insert("TB320FC".into(), "D:/fw/loader.melf".into());
        let stored: PersistedSettings =
            serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert!(!stored.remember_edl_loader);
        assert_eq!(
            stored
                .remembered_edl_loaders
                .get("TB320FC")
                .map(String::as_str),
            Some("D:/fw/loader.melf")
        );
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
        for p in ["/a", "/b", "/c", "/d", "/e", "/f", "/a"] {
            r.push("k", p);
        }
        // Trace: [/a] → [/b,/a] → ... → [/f,/e,/d,/c,/b] (cap=5 drops
        // /a) → [/a,/f,/e,/d,/c] (/a re-enters at front as a fresh item).
        // Dedup only kicks in while the entry is still inside the cap.
        assert_eq!(r.recent("k"), &["/a", "/f", "/e", "/d", "/c"]);
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
