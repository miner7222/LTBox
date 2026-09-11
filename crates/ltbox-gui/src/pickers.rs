//! File/folder picker categories and rfd helpers.
//!
//! Folder picker kinds have separate recents; file picks share one bucket
//! and vary through [`FilePickSpec`].

use iced::Task;
use rfd::AsyncFileDialog;
use std::path::PathBuf;

use crate::settings_store::RecentPaths;

/// Picker category for recents and dialog type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PickerKind {
    /// Folder containing the fixed-name EDL loader `xbl_s_devprg_ns.melf`.
    LoaderFolder,
    /// Full QFIL firmware folder (programmer + all XML + partition images).
    QfilFirmwareFolder,
    /// Folder with encrypted `rawprogram*.x` files.
    EncryptedRawprogramFolder,
    /// Output / save destination folder (dumps, log saves).
    OutputFolder,
    /// Unified file pick — customised by [`FilePickSpec`] per call.
    File,
}

impl PickerKind {
    /// Stable string used as the JSON key in the on-disk recents map.
    /// **Must not change** without a migration — renaming a key silently
    /// orphans the user's history.
    pub fn storage_key(self) -> &'static str {
        match self {
            Self::LoaderFolder => "loader_folder",
            Self::QfilFirmwareFolder => "qfil_firmware_folder",
            Self::EncryptedRawprogramFolder => "encrypted_rawprogram_folder",
            Self::OutputFolder => "output_folder",
            Self::File => "file",
        }
    }

    /// `true` iff the picker opens a folder dialog (vs a file dialog).
    pub fn is_folder(self) -> bool {
        !matches!(self, Self::File)
    }
}

/// File-picker parameters; recents stay in the shared `File` bucket.
#[derive(Debug, Clone)]
pub struct FilePickSpec {
    /// Extensions without the leading dot, e.g. `["img", "bin"]`.
    /// Empty = no filter (native "All files").
    pub exts: Vec<String>,
    /// Human-readable filter label shown in the dialog's type dropdown.
    pub filter_label: String,
    /// `true` for multi-select (`pick_files`), `false` for single (`pick_file`).
    pub multi: bool,
}

impl FilePickSpec {
    /// Single-file, no filter.
    pub fn single() -> Self {
        Self {
            exts: Vec::new(),
            filter_label: String::new(),
            multi: false,
        }
    }

    /// Multi-file, no filter.
    pub fn multi() -> Self {
        Self {
            exts: Vec::new(),
            filter_label: String::new(),
            multi: true,
        }
    }

    /// Attach an ext filter (fluent builder). Both `filter_label` and
    /// `exts` must be set for the filter to register with rfd.
    pub fn with_filter(mut self, filter_label: impl Into<String>, exts: &[&str]) -> Self {
        self.filter_label = filter_label.into();
        self.exts = exts.iter().map(|s| (*s).to_string()).collect();
        self
    }
}

/// Match display history using the same extension set as the native dialog.
pub(crate) fn path_matches_extensions(path: &str, extensions: &[&str]) -> bool {
    extensions.is_empty()
        || std::path::Path::new(path)
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                extensions
                    .iter()
                    .any(|candidate| candidate.eq_ignore_ascii_case(extension))
            })
}

/// Open a folder picker seeded from that kind's most-recent path.
pub fn pick_folder_for<M: 'static + Send>(
    kind: PickerKind,
    recents: &RecentPaths,
    on_pick: impl 'static + Send + Fn(Option<String>) -> M,
) -> Task<M> {
    debug_assert!(
        kind.is_folder(),
        "pick_folder_for called with file kind {kind:?}"
    );
    let start_dir: Option<PathBuf> = recents.most_recent(kind.storage_key()).map(PathBuf::from);
    Task::perform(
        async move {
            let mut dialog = AsyncFileDialog::new();
            if let Some(sd) = start_dir.filter(|p| p.is_dir()) {
                dialog = dialog.set_directory(sd);
            }
            dialog
                .pick_folder()
                .await
                .map(|f| f.path().to_string_lossy().to_string())
        },
        on_pick,
    )
}

/// Build an rfd file dialog from `spec`.
fn build_file_dialog(spec: &FilePickSpec, recents: &RecentPaths) -> AsyncFileDialog {
    let mut dialog = AsyncFileDialog::new();
    if !spec.exts.is_empty() && !spec.filter_label.is_empty() {
        let exts: Vec<&str> = spec.exts.iter().map(String::as_str).collect();
        dialog = dialog.add_filter(&spec.filter_label, &exts);
    }
    if let Some(sd) = recents
        .most_recent(PickerKind::File.storage_key())
        .map(PathBuf::from)
        .filter(|p| p.exists())
    {
        // Recent may be a file path — rfd wants a directory, so normalise
        // to parent when needed. Missing/non-dir parent falls through to
        // the OS default (no set_directory call).
        let dir = if sd.is_dir() {
            sd
        } else {
            sd.parent().map(PathBuf::from).unwrap_or(sd)
        };
        if dir.is_dir() {
            dialog = dialog.set_directory(dir);
        }
    }
    dialog
}

/// Single-file pick, `None` on cancel.
pub fn pick_file_for<M: 'static + Send>(
    spec: FilePickSpec,
    recents: &RecentPaths,
    on_pick: impl 'static + Send + Fn(Option<String>) -> M,
) -> Task<M> {
    debug_assert!(
        !spec.multi,
        "pick_file_for called with multi=true spec; use pick_files_for"
    );
    let dialog = build_file_dialog(&spec, recents);
    Task::perform(
        async move {
            dialog
                .pick_file()
                .await
                .map(|h| h.path().to_string_lossy().to_string())
        },
        on_pick,
    )
}

/// Multi-file pick, `None` on cancel.
pub fn pick_files_for<M: 'static + Send>(
    spec: FilePickSpec,
    recents: &RecentPaths,
    on_pick: impl 'static + Send + Fn(Option<Vec<String>>) -> M,
) -> Task<M> {
    debug_assert!(
        spec.multi,
        "pick_files_for called with multi=false spec; use pick_file_for"
    );
    let dialog = build_file_dialog(&spec, recents);
    Task::perform(
        async move {
            dialog.pick_files().await.map(|handles| {
                handles
                    .into_iter()
                    .map(|h| h.path().to_string_lossy().to_string())
                    .collect::<Vec<_>>()
            })
        },
        on_pick,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_keys_are_unique_and_stable() {
        let all = [
            PickerKind::LoaderFolder,
            PickerKind::QfilFirmwareFolder,
            PickerKind::EncryptedRawprogramFolder,
            PickerKind::OutputFolder,
            PickerKind::File,
        ];
        let mut keys: Vec<&str> = all.iter().map(|k| k.storage_key()).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), all.len(), "storage_key collision");

        // Spot-check stable literals — renaming any of these breaks user
        // recents on upgrade. This test fails loudly if someone edits one.
        assert_eq!(PickerKind::LoaderFolder.storage_key(), "loader_folder");
        assert_eq!(PickerKind::File.storage_key(), "file");
    }

    #[test]
    fn is_folder_only_false_for_file() {
        for k in [
            PickerKind::LoaderFolder,
            PickerKind::QfilFirmwareFolder,
            PickerKind::EncryptedRawprogramFolder,
            PickerKind::OutputFolder,
        ] {
            assert!(k.is_folder());
        }
        assert!(!PickerKind::File.is_folder());
    }

    #[test]
    fn spec_builder_sets_filter() {
        let s = FilePickSpec::single().with_filter("Partition image", &["img", "bin"]);
        assert!(!s.multi);
        assert_eq!(s.exts, vec!["img".to_string(), "bin".to_string()]);
        assert_eq!(s.filter_label, "Partition image");
    }

    #[test]
    fn spec_multi_builder() {
        let s = FilePickSpec::multi().with_filter("KPM modules", &["kpm"]);
        assert!(s.multi);
        assert_eq!(s.exts, vec!["kpm".to_string()]);
    }

    #[test]
    fn spec_without_filter_leaves_exts_empty() {
        let s = FilePickSpec::single();
        assert!(s.exts.is_empty());
        assert!(s.filter_label.is_empty());
    }
}
