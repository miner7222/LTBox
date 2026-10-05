//! KonaBess GPU-table wizard state.

use super::*;

pub(crate) const KONABESS_STEPS: &[&str] = &[
    "edl_loader_label",
    "konabess_step_table",
    "konabess_step_confirm",
    "konabess_step_apply",
];

/// Device state retained across the inspection worker's UI selection pause.
/// Part 2 can consume these exact stock images and the already-resolved slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KonaBessPrepared {
    pub(crate) work_dir: std::path::PathBuf,
    pub(crate) vendor_boot: std::path::PathBuf,
    pub(crate) vbmeta: std::path::PathBuf,
    pub(crate) backup_dir: std::path::PathBuf,
    pub(crate) slot_suffix: String,
    /// Android's best-effort `ro.boot.dtb_idx` hint, captured before EDL.
    pub(crate) probable_dtb_index: Option<usize>,
    /// Model detected over ADB/Fastboot before EDL, which wipes it.
    pub(crate) device_model: String,
}

/// KonaBess wizard state. The prepared workspace is populated only after the
/// non-destructive device inspection and remains live through target selection.
#[derive(Debug, Clone, Default)]
pub(crate) struct KonaBessWizard {
    pub(crate) step: usize,
    pub(crate) loader_path: Option<String>,
    pub(crate) loader_error: Option<String>,
    pub(crate) import_path: Option<String>,
    pub(crate) import_error: Option<String>,
    pub(crate) import_warnings: Vec<GpuTableIssue>,
    /// Device table retained for comparison and one-click revert.
    pub(crate) stock_table: Option<GpuTable>,
    /// In-memory table that will be passed directly to the AVB build path.
    pub(crate) edited_table: Option<GpuTable>,
    pub(crate) edited_dirty: bool,
    /// User-entered text is retained independently from the last parseable
    /// value committed to `edited_table`, so partial input never snaps back.
    pub(crate) cell_edits: BTreeMap<GpuCellKey, GpuCellEdit>,
    /// DTBs whose GPU table parsed from the dumped vendor_boot image.
    pub(crate) candidates: Vec<VendorBootDtbInfo>,
    /// The one DTB index passed to the existing single-target patch API.
    pub(crate) selected_target_index: Option<usize>,
    /// Upstream KonaBess's probable target, when it is one of the candidates.
    pub(crate) probable_target_index: Option<usize>,
    /// Modal ownership stays with the wizard rather than parallel App flags.
    pub(crate) target_popup_open: bool,
    /// Initial selection is part of the inspection pause; cancelling it abandons
    /// the prepared device flow. Reopening the picker from the table does not.
    pub(crate) target_popup_abandons_on_dismiss: bool,
    pub(crate) prepared: Option<KonaBessPrepared>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum GpuCellLocation {
    Level { level: usize, property: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GpuPropertyLocation {
    GroupHeader,
    Level,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GpuPropertyEditability {
    ReadOnly,
    Editable,
}

pub(crate) fn gpu_property_editability(
    location: GpuPropertyLocation,
    property_name: &str,
) -> GpuPropertyEditability {
    match location {
        GpuPropertyLocation::GroupHeader => GpuPropertyEditability::ReadOnly,
        GpuPropertyLocation::Level if property_name == "reg" => GpuPropertyEditability::ReadOnly,
        GpuPropertyLocation::Level => GpuPropertyEditability::Editable,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct GpuCellKey {
    pub(crate) group: usize,
    pub(crate) location: GpuCellLocation,
    pub(crate) cell: usize,
}

impl GpuCellKey {
    pub(crate) const fn level(group: usize, level: usize, property: usize, cell: usize) -> Self {
        Self {
            group,
            location: GpuCellLocation::Level { level, property },
            cell,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GpuCellEdit {
    pub(crate) text: String,
    pub(crate) has_error: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum KonaBessImportError {
    NoTarget,
    TargetChipUnknown,
    ChipMismatch { expected: String, actual: String },
}

impl KonaBessWizard {
    /// Accept an inspection result and require explicit single-target selection.
    /// Candidates are driven solely by GPU tables parsed from the device image.
    pub(crate) fn apply_inspection_result(
        &mut self,
        inspected: Vec<VendorBootDtbInfo>,
        probable_dtb_index: Option<usize>,
    ) {
        self.candidates = inspected
            .into_iter()
            .filter(|candidate| candidate.table.is_some())
            .collect();
        self.probable_target_index = probable_dtb_index.filter(|index| {
            self.candidates
                .iter()
                .any(|candidate| candidate.index == *index)
        });
        self.clear_table_selection();
        self.target_popup_open = true;
        self.target_popup_abandons_on_dismiss = true;
    }

    pub(crate) fn is_probable_target(&self, target_index: usize) -> bool {
        self.probable_target_index == Some(target_index)
    }

    /// Select one candidate by its stable vendor_boot DTB index.
    pub(crate) fn select_target(&mut self, target_index: usize) -> bool {
        let Some(table) = self
            .candidates
            .iter()
            .find(|candidate| candidate.index == target_index && candidate.chip.is_some())
            .and_then(|candidate| candidate.table.clone())
        else {
            return false;
        };
        self.selected_target_index = Some(target_index);
        self.edited_table = Some(table.clone());
        self.stock_table = Some(table);
        self.cell_edits.clear();
        self.edited_dirty = false;
        self.import_path = None;
        self.import_error = None;
        self.import_warnings.clear();
        true
    }

    /// Confirm target selection. The popup remains open until one DTB is set.
    pub(crate) fn confirm_target(&mut self) -> Option<usize> {
        let selected = self.selected_target_index?;
        self.stock_table.as_ref()?;
        self.edited_table.as_ref()?;
        self.target_popup_open = false;
        self.target_popup_abandons_on_dismiss = false;
        Some(selected)
    }

    /// Dismissal is a non-error state; a later Apply inspection can reopen it.
    pub(crate) fn dismiss_target_popup(&mut self) -> bool {
        let abandons = self.target_popup_abandons_on_dismiss;
        self.target_popup_open = false;
        self.target_popup_abandons_on_dismiss = false;
        abandons
    }

    pub(crate) fn open_target_popup(&mut self) {
        self.target_popup_open = true;
        self.target_popup_abandons_on_dismiss = false;
    }

    pub(crate) fn selected_target(&self) -> Option<&VendorBootDtbInfo> {
        let index = self.selected_target_index?;
        self.candidates
            .iter()
            .find(|candidate| candidate.index == index)
    }

    pub(crate) fn selected_chip(&self) -> Option<&str> {
        self.selected_target()?.chip.as_deref()
    }

    pub(crate) fn overwrite_edited_from_import(
        &mut self,
        export: KonaBessExport,
    ) -> Result<(), KonaBessImportError> {
        let Some(target) = self.selected_target() else {
            return Err(KonaBessImportError::NoTarget);
        };
        let Some(expected) = target.chip.as_deref() else {
            return Err(KonaBessImportError::TargetChipUnknown);
        };
        if !chip_names_match(&export.chip, expected) {
            return Err(KonaBessImportError::ChipMismatch {
                expected: expected.to_string(),
                actual: export.chip,
            });
        }
        let mut table = export.table;
        if let Some(stock) = self.stock_table.as_ref() {
            // KonaBess profiles provide editable level data. The selected
            // device remains authoritative for group identity and every bin
            // header property, including FDT cell metadata and SKU bindings.
            table.groups = stock
                .groups
                .iter()
                .map(|stock_group| {
                    table
                        .groups
                        .iter()
                        .find(|imported_group| imported_group.id == stock_group.id)
                        .map_or_else(
                            || stock_group.clone(),
                            |imported_group| GpuGroup {
                                id: stock_group.id,
                                header_properties: stock_group.header_properties.clone(),
                                levels: imported_group.levels.clone(),
                            },
                        )
                })
                .collect();
        }
        if let Some(stock) = self.stock_table.as_ref()
            && let Ok(normalized) = normalize_edited_gpu_table(stock, &table)
        {
            table = normalized.table;
        }
        self.edited_table = Some(table);
        self.cell_edits.clear();
        self.edited_dirty = self.edited_table != self.stock_table;
        self.import_warnings = export.import_warnings;
        Ok(())
    }

    /// Retain the exact text being typed and commit only values that parse.
    /// Frequencies are presented in MHz but stored as exact integer Hz.
    pub(crate) fn edit_cell(&mut self, key: GpuCellKey, text: String) -> bool {
        let Some(property_name) = self
            .cell_property(key)
            .map(|property| property.name.clone())
        else {
            return false;
        };
        if gpu_property_editability(GpuPropertyLocation::Level, &property_name)
            == GpuPropertyEditability::ReadOnly
        {
            return false;
        }
        let parsed = if property_name == "qcom,gpu-freq" {
            parse_gpu_frequency_mhz(&text)
        } else {
            parse_gpu_cell(&text).map_err(|_| ())
        };
        let has_error = parsed.is_err();
        self.cell_edits.insert(key, GpuCellEdit { text, has_error });
        let Ok(value) = parsed else {
            return false;
        };
        let Some(stock) = self.stock_table.as_ref() else {
            return false;
        };
        let Some(mut edited) = self.edited_table.clone() else {
            return false;
        };
        let Some(cell) = Self::cell_mut(&mut edited, key) else {
            return false;
        };
        *cell = value;
        let Ok(normalized) = normalize_edited_gpu_table(stock, &edited) else {
            return false;
        };
        self.edited_table = Some(normalized.table);
        self.edited_dirty = self.edited_table != self.stock_table;
        true
    }

    pub(crate) fn cell_text(&self, key: GpuCellKey, committed: u32, property: &str) -> String {
        self.cell_edits.get(&key).map_or_else(
            || {
                if property == "qcom,gpu-freq" {
                    format_gpu_frequency_mhz(committed)
                } else {
                    committed.to_string()
                }
            },
            |edit| edit.text.clone(),
        )
    }

    pub(crate) fn cell_has_input_error(&self, key: GpuCellKey) -> bool {
        self.cell_edits.get(&key).is_some_and(|edit| edit.has_error)
    }

    pub(crate) fn issue_matches_cell(&self, issue: &GpuTableIssue, key: GpuCellKey) -> bool {
        self.cell_property_path(key)
            .is_some_and(|path| issue.path == path)
    }

    /// Blocking parser/structural findings plus all non-blocking validation and
    /// retargeting advisories currently implied by the working table.
    pub(crate) fn editor_validation(&self) -> GpuTableValidation {
        let Some(edited) = self.edited_table.as_ref() else {
            return GpuTableValidation::default();
        };
        let mut validation = validate_gpu_table(edited);
        for (key, edit) in &self.cell_edits {
            if edit.has_error {
                validation.hard_errors.push(GpuTableIssue {
                    path: self.cell_property_path(*key).map_or_else(
                        || "table".to_string(),
                        |path| format!("{path}[{}]", key.cell),
                    ),
                    message: "cell input is not a parseable u32 value".to_string(),
                });
            }
        }
        if let Some(stock) = self.stock_table.as_ref()
            && let Ok(normalized) = normalize_edited_gpu_table(stock, edited)
        {
            validation.warnings = normalized.advisories;
        }
        validation
            .warnings
            .extend(self.import_warnings.iter().cloned());
        validation
    }

    /// Append a copy of the last sibling through the core's schema-preserving
    /// constructor, then apply the core's index/initial-target normalization.
    pub(crate) fn add_level(&mut self, group_position: usize) -> bool {
        if self.editor_validation().has_hard_errors() {
            return false;
        }
        let Some(stock) = self.stock_table.as_ref() else {
            return false;
        };
        let Some(mut edited) = self.edited_table.clone() else {
            return false;
        };
        let Some(group) = edited.groups.get(group_position) else {
            return false;
        };
        let Some(template) = group.levels.last() else {
            return false;
        };
        let Ok(new_level_id) = u32::try_from(group.levels.len()) else {
            return false;
        };
        let Ok(new_level) =
            build_gpu_level_from_template(group, template.id, new_level_id, |property| {
                property.cells.clone()
            })
        else {
            return false;
        };
        edited.groups[group_position].levels.push(new_level);
        let Ok(normalized) = normalize_edited_gpu_table(stock, &edited) else {
            return false;
        };
        self.edited_table = Some(normalized.table);
        self.cell_edits.clear();
        self.edited_dirty = self.edited_table != self.stock_table;
        true
    }

    /// Remove one row while refusing to create an invalid empty group.
    pub(crate) fn remove_level(&mut self, group_position: usize, level_position: usize) -> bool {
        if self.editor_validation().has_hard_errors() {
            return false;
        }
        let Some(stock) = self.stock_table.as_ref() else {
            return false;
        };
        let Some(mut edited) = self.edited_table.clone() else {
            return false;
        };
        let Some(group) = edited.groups.get_mut(group_position) else {
            return false;
        };
        if group.levels.len() <= 1 || level_position >= group.levels.len() {
            return false;
        }
        group.levels.remove(level_position);
        let Ok(normalized) = normalize_edited_gpu_table(stock, &edited) else {
            return false;
        };
        self.edited_table = Some(normalized.table);
        self.cell_edits.clear();
        self.edited_dirty = self.edited_table != self.stock_table;
        true
    }

    pub(crate) fn revert_edits(&mut self) -> bool {
        let Some(stock) = self.stock_table.clone() else {
            return false;
        };
        self.edited_table = Some(stock);
        self.cell_edits.clear();
        self.edited_dirty = false;
        self.import_path = None;
        self.import_error = None;
        self.import_warnings.clear();
        true
    }

    fn clear_table_selection(&mut self) {
        self.selected_target_index = None;
        self.stock_table = None;
        self.edited_table = None;
        self.cell_edits.clear();
        self.edited_dirty = false;
        self.import_path = None;
        self.import_error = None;
        self.import_warnings.clear();
    }

    /// Remove inspection scratch when the flow is closed or abandoned. Stock
    /// backups are intentionally retained; only the part-2 working copies go.
    pub(crate) fn cleanup_prepared(&mut self) {
        if let Some(prepared) = self.prepared.take() {
            let _ = std::fs::remove_dir_all(prepared.work_dir);
        }
        self.candidates.clear();
        self.probable_target_index = None;
        self.target_popup_open = false;
        self.target_popup_abandons_on_dismiss = false;
        self.clear_table_selection();
    }
}

impl Wizard for KonaBessWizard {
    fn reset(&mut self) {
        self.cleanup_prepared();
        *self = Self::default();
    }

    fn step(&self) -> usize {
        self.step
    }

    fn step_mut(&mut self) -> &mut usize {
        &mut self.step
    }

    fn step_count(&self) -> usize {
        KONABESS_STEPS.len()
    }

    fn can_next(&self) -> bool {
        match self.step {
            0 => self.loader_path.is_some() && self.loader_error.is_none(),
            1 => {
                self.prepared.is_some()
                    && self.selected_chip().is_some()
                    && self.stock_table.is_some()
                    && self.edited_table.is_some()
                    && !self.editor_validation().has_hard_errors()
            }
            2 => {
                self.loader_path.is_some()
                    && self.prepared.is_some()
                    && self.selected_chip().is_some()
                    && self.edited_table.is_some()
                    && !self.editor_validation().has_hard_errors()
            }
            3 => false,
            _ => false,
        }
    }
}

impl KonaBessWizard {
    fn cell_property(&self, key: GpuCellKey) -> Option<&ltbox_patch::konabess::GpuProperty> {
        let group = self.edited_table.as_ref()?.groups.get(key.group)?;
        let GpuCellLocation::Level { level, property } = key.location;
        group.levels.get(level)?.properties.get(property)
    }

    fn cell_mut(table: &mut GpuTable, key: GpuCellKey) -> Option<&mut u32> {
        let group = table.groups.get_mut(key.group)?;
        let GpuCellLocation::Level { level, property } = key.location;
        let property = group.levels.get_mut(level)?.properties.get_mut(property)?;
        property.cells.get_mut(key.cell)
    }

    fn cell_property_path(&self, key: GpuCellKey) -> Option<String> {
        let table = self.edited_table.as_ref()?;
        let group = table.groups.get(key.group)?;
        let property = self.cell_property(key)?;
        let GpuCellLocation::Level { level, .. } = key.location;
        Some(format!(
            "group {} / level {} / {}",
            group.id,
            group.levels.get(level)?.id,
            property.name
        ))
    }
}

fn parse_gpu_frequency_mhz(input: &str) -> Result<u32, ()> {
    const HZ_PER_MHZ: u32 = 1_000_000;
    let value = input.trim();
    let Some((whole, fraction)) = value.split_once('.') else {
        return parse_gpu_cell(value)
            .map_err(|_| ())?
            .checked_mul(HZ_PER_MHZ)
            .ok_or(());
    };
    if value.matches('.').count() != 1
        || fraction.is_empty()
        || whole.starts_with("0x")
        || whole.starts_with("0X")
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(());
    }
    let significant_fraction = fraction.trim_end_matches('0');
    if significant_fraction.len() > 6 {
        return Err(());
    }
    let whole_hz = parse_gpu_cell(whole)
        .map_err(|_| ())?
        .checked_mul(HZ_PER_MHZ)
        .ok_or(())?;
    let fraction_hz = if significant_fraction.is_empty() {
        0
    } else {
        let parsed = parse_gpu_cell(significant_fraction).map_err(|_| ())?;
        let scale = 10_u32
            .checked_pow(u32::try_from(6 - significant_fraction.len()).map_err(|_| ())?)
            .ok_or(())?;
        parsed.checked_mul(scale).ok_or(())?
    };
    whole_hz.checked_add(fraction_hz).ok_or(())
}

fn format_gpu_frequency_mhz(frequency_hz: u32) -> String {
    const HZ_PER_MHZ: u32 = 1_000_000;
    let whole = frequency_hz / HZ_PER_MHZ;
    let remainder = frequency_hz % HZ_PER_MHZ;
    if remainder == 0 {
        whole.to_string()
    } else {
        format!("{whole}.{remainder:06}")
            .trim_end_matches('0')
            .to_string()
    }
}

#[cfg(test)]
mod konabess_tests {
    use super::*;
    use ltbox_patch::konabess::{GpuGroup, GpuLevel, GpuProperty, GpuTable};

    fn table(frequency: u32) -> GpuTable {
        GpuTable {
            groups: vec![GpuGroup {
                id: 0,
                header_properties: vec![],
                levels: vec![GpuLevel {
                    id: 0,
                    properties: vec![
                        GpuProperty {
                            name: "reg".into(),
                            cells: vec![0],
                        },
                        GpuProperty {
                            name: "qcom,gpu-freq".into(),
                            cells: vec![frequency],
                        },
                        GpuProperty {
                            name: "qcom,level".into(),
                            cells: vec![200],
                        },
                    ],
                }],
            }],
        }
    }

    fn prepared() -> KonaBessPrepared {
        KonaBessPrepared {
            work_dir: "work".into(),
            vendor_boot: "vendor_boot.img".into(),
            vbmeta: "vbmeta.img".into(),
            backup_dir: "backup".into(),
            slot_suffix: "_a".into(),
            probable_dtb_index: None,
            device_model: "TB323FU".into(),
        }
    }

    fn ready_wizard(frequency: u32) -> KonaBessWizard {
        let mut wizard = KonaBessWizard::default();
        wizard.apply_inspection_result(vec![candidate(1, Some("sun"), Some(frequency))], None);
        assert!(wizard.select_target(1));
        wizard.prepared = Some(prepared());
        wizard.step = 1;
        wizard
    }

    fn candidate(index: usize, chip: Option<&str>, frequency: Option<u32>) -> VendorBootDtbInfo {
        VendorBootDtbInfo {
            index,
            model: Some(format!("model-{index}")),
            chip: chip.map(str::to_owned),
            gpu_shape: None,
            table: frequency.map(table),
        }
    }

    #[test]
    fn inspection_without_export_offers_every_parsed_gpu_table() {
        let mut wizard = KonaBessWizard::default();

        wizard.apply_inspection_result(
            vec![
                candidate(1, None, Some(700_000_000)),
                candidate(2, Some("sun"), None),
                candidate(3, Some("sun"), Some(900_000_000)),
            ],
            None,
        );

        assert_eq!(
            wizard
                .candidates
                .iter()
                .map(|candidate| candidate.index)
                .collect::<Vec<_>>(),
            vec![1, 3]
        );
    }

    #[test]
    fn selecting_a_target_populates_stock_and_edited_tables() {
        let mut wizard = KonaBessWizard::default();
        wizard.apply_inspection_result(
            vec![
                candidate(2, Some("sun"), Some(700_000_000)),
                candidate(7, Some("sun"), Some(900_000_000)),
            ],
            None,
        );

        assert!(wizard.select_target(2));
        assert_eq!(wizard.selected_target_index, Some(2));
        assert_eq!(wizard.stock_table, Some(table(700_000_000)));
        assert_eq!(wizard.edited_table, wizard.stock_table);
        assert!(!wizard.edited_dirty);

        wizard
            .overwrite_edited_from_import(KonaBessExport {
                chip: "sun".into(),
                description: "import from first target".into(),
                table: table(800_000_000),
                import_warnings: vec![],
            })
            .unwrap();
        wizard.import_path = Some("first-target.txt".into());
        wizard.import_error = Some("stale error".into());
        assert!(wizard.edit_cell(GpuCellKey::level(0, 0, 1, 0), "801".to_string()));
        assert!(!wizard.cell_edits.is_empty());
        assert!(wizard.edited_dirty);

        assert!(wizard.select_target(7));
        assert_eq!(wizard.selected_target_index, Some(7));
        assert_eq!(wizard.stock_table, Some(table(900_000_000)));
        assert_eq!(wizard.edited_table, Some(table(900_000_000)));
        assert!(!wizard.edited_dirty);
        assert!(wizard.cell_edits.is_empty());
        assert!(wizard.import_path.is_none());
        assert!(wizard.import_error.is_none());
        assert!(!wizard.select_target(99));
        assert_eq!(wizard.selected_target_index, Some(7));
    }

    #[test]
    fn confirming_requires_a_selected_target() {
        let mut wizard = KonaBessWizard::default();
        wizard.apply_inspection_result(vec![candidate(3, Some("sun"), Some(700_000_000))], None);

        assert_eq!(wizard.confirm_target(), None);
        assert!(wizard.target_popup_open);
        assert!(wizard.select_target(3));
        assert_eq!(wizard.confirm_target(), Some(3));
        assert!(!wizard.target_popup_open);
    }

    #[test]
    fn initial_picker_cancel_abandons_but_reopened_picker_cancel_does_not() {
        let mut wizard = KonaBessWizard::default();
        wizard.apply_inspection_result(vec![candidate(3, Some("sun"), Some(700_000_000))], None);
        assert!(wizard.select_target(3));

        assert!(wizard.dismiss_target_popup());
        wizard.open_target_popup();
        assert!(!wizard.dismiss_target_popup());
    }

    #[test]
    fn import_overwrites_only_edited_table_and_tracks_dirty_state() {
        let mut wizard = KonaBessWizard::default();
        wizard.apply_inspection_result(vec![candidate(1, Some("sun"), Some(700_000_000))], None);
        assert!(wizard.select_target(1));
        let stock = wizard.stock_table.clone();

        wizard
            .overwrite_edited_from_import(KonaBessExport {
                chip: "sun".into(),
                description: "import".into(),
                table: table(950_000_000),
                import_warnings: vec![],
            })
            .unwrap();

        assert_eq!(wizard.stock_table, stock);
        assert_eq!(wizard.edited_table, Some(table(950_000_000)));
        assert!(wizard.edited_dirty);
        assert!(wizard.revert_edits());
        assert_eq!(wizard.edited_table, stock);
        assert!(!wizard.edited_dirty);
    }

    #[test]
    fn import_commits_the_core_normalized_table() {
        let mut wizard = ready_wizard(900_000_000);
        let group = &mut wizard.edited_table.as_mut().unwrap().groups[0];
        group.header_properties.push(GpuProperty {
            name: "qcom,initial-pwrlevel".into(),
            cells: vec![0],
        });
        let mut second = group.levels[0].clone();
        second.id = 1;
        second.properties[0].cells[0] = 1;
        second.properties[1].cells[0] = 700_000_000;
        group.levels.push(second);
        wizard.stock_table = wizard.edited_table.clone();
        let mut imported = wizard.edited_table.clone().unwrap();
        imported.groups[0].header_properties[0].cells[0] = 1;
        let expected = normalize_edited_gpu_table(wizard.stock_table.as_ref().unwrap(), &imported)
            .unwrap()
            .table;

        wizard
            .overwrite_edited_from_import(KonaBessExport {
                chip: "sun".into(),
                description: "normalization regression".into(),
                table: imported,
                import_warnings: vec![],
            })
            .unwrap();

        assert_eq!(wizard.edited_table, Some(expected));
        assert_eq!(
            wizard.edited_table.as_ref().unwrap().groups[0].header_properties[0].cells,
            [0]
        );
    }

    #[test]
    fn cell_edit_updates_only_working_copy_and_retains_invalid_text() {
        let mut wizard = ready_wizard(700_000_000);
        let stock = wizard.stock_table.clone();
        let frequency = GpuCellKey::level(0, 0, 1, 0);

        assert!(wizard.edit_cell(frequency, "812.345678".into()));
        assert_eq!(
            wizard.edited_table.as_ref().unwrap().groups[0].levels[0].properties[1].cells,
            [812_345_678]
        );
        assert_eq!(wizard.stock_table, stock);
        assert!(wizard.edited_dirty);

        assert!(!wizard.edit_cell(frequency, "812.".into()));
        assert_eq!(wizard.cell_edits[&frequency].text, "812.");
        assert!(wizard.cell_edits[&frequency].has_error);
        assert!(wizard.editor_validation().has_hard_errors());
        assert!(!wizard.can_next());
        assert_eq!(
            wizard.edited_table.as_ref().unwrap().groups[0].levels[0].properties[1].cells,
            [812_345_678]
        );
    }

    #[test]
    fn every_group_header_property_is_read_only_and_level_reg_stays_read_only() {
        for property_name in [
            "qcom,speed-bin",
            "qcom,sku-codes",
            "#address-cells",
            "#size-cells",
            "qcom,initial-pwrlevel",
            "qcom,initial-min-pwrlevel",
            "vendor,unknown-header",
        ] {
            assert_eq!(
                gpu_property_editability(GpuPropertyLocation::GroupHeader, property_name),
                GpuPropertyEditability::ReadOnly
            );
        }
        assert_eq!(
            gpu_property_editability(GpuPropertyLocation::Level, "reg"),
            GpuPropertyEditability::ReadOnly
        );
        assert_eq!(
            gpu_property_editability(GpuPropertyLocation::Level, "qcom,gpu-freq"),
            GpuPropertyEditability::Editable
        );

        let mut wizard = ready_wizard(700_000_000);
        let before = wizard.edited_table.clone();
        assert!(!wizard.edit_cell(GpuCellKey::level(0, 0, 0, 0), "99".into()));
        assert_eq!(wizard.edited_table, before);
        assert!(wizard.cell_edits.is_empty());
    }

    #[test]
    fn accepted_cell_edit_commits_the_core_normalized_table() {
        let mut wizard = ready_wizard(900_000_000);
        let group = &mut wizard.edited_table.as_mut().unwrap().groups[0];
        group.header_properties.push(GpuProperty {
            name: "qcom,initial-pwrlevel".into(),
            cells: vec![0],
        });
        let mut second = group.levels[0].clone();
        second.id = 1;
        second.properties[0].cells[0] = 1;
        second.properties[1].cells[0] = 700_000_000;
        group.levels.push(second);
        wizard.stock_table = wizard.edited_table.clone();
        wizard.edited_table.as_mut().unwrap().groups[0].header_properties[0].cells[0] = 1;

        let mut candidate = wizard.edited_table.clone().unwrap();
        candidate.groups[0].levels[0].properties[2].cells[0] = 300;
        let expected = normalize_edited_gpu_table(wizard.stock_table.as_ref().unwrap(), &candidate)
            .unwrap()
            .table;

        assert!(wizard.edit_cell(GpuCellKey::level(0, 0, 2, 0), "300".into()));
        assert_eq!(wizard.edited_table, Some(expected));
        assert_eq!(
            wizard.edited_table.as_ref().unwrap().groups[0].header_properties[0].cells,
            [0]
        );
    }

    #[test]
    fn advisory_is_visible_but_does_not_block_advancing() {
        let mut wizard = ready_wizard(700_000_000);
        let frequency = GpuCellKey::level(0, 0, 1, 0);

        assert!(wizard.edit_cell(frequency, "2000".into()));
        let validation = wizard.editor_validation();

        assert!(!validation.has_hard_errors());
        assert!(!validation.warnings.is_empty());
        assert!(wizard.can_next());
    }

    #[test]
    fn deleted_initial_target_advisory_does_not_block_advancing() {
        let mut wizard = ready_wizard(900_000_000);
        let group = &mut wizard.edited_table.as_mut().unwrap().groups[0];
        group.header_properties.push(GpuProperty {
            name: "qcom,initial-pwrlevel".into(),
            cells: vec![0],
        });
        let mut second = group.levels[0].clone();
        second.id = 1;
        second.properties[0].cells[0] = 1;
        second.properties[1].cells[0] = 700_000_000;
        group.levels.push(second);
        wizard.stock_table = wizard.edited_table.clone();
        let mut removed = wizard.stock_table.clone().unwrap();
        removed.groups[0].levels.remove(0);
        let expected = normalize_edited_gpu_table(wizard.stock_table.as_ref().unwrap(), &removed)
            .unwrap()
            .table;

        assert!(wizard.remove_level(0, 0));
        let validation = wizard.editor_validation();

        assert_eq!(wizard.edited_table, Some(expected));
        assert_eq!(
            wizard.edited_table.as_ref().unwrap().groups[0].header_properties[0].cells,
            [0]
        );
        assert!(!validation.has_hard_errors());
        assert!(
            validation
                .warnings
                .iter()
                .any(|warning| warning.message.contains("was deleted"))
        );
        assert!(wizard.can_next());
    }

    #[test]
    fn import_preserves_device_group_headers_and_ignores_foreign_groups() {
        let mut wizard = ready_wizard(700_000_000);
        let sku_codes = GpuProperty {
            name: "qcom,sku-codes".into(),
            cells: vec![1, 2, 3],
        };
        wizard.stock_table.as_mut().unwrap().groups[0]
            .header_properties
            .push(sku_codes.clone());
        wizard.edited_table.as_mut().unwrap().groups[0]
            .header_properties
            .push(sku_codes);
        let mut imported = wizard.edited_table.clone().unwrap();
        imported.groups[0].header_properties[0].cells = vec![9, 8, 7];
        imported.groups[0].levels[0].properties[1].cells[0] = 800_000_000;
        let mut foreign_group = imported.groups[0].clone();
        foreign_group.id = 99;
        imported.groups.push(foreign_group);

        wizard
            .overwrite_edited_from_import(KonaBessExport {
                chip: "sun".into(),
                description: "unsafe header replacement".into(),
                table: imported,
                import_warnings: vec![],
            })
            .unwrap();

        assert_eq!(
            wizard.edited_table.as_ref().unwrap().groups[0].header_properties[0].cells,
            [1, 2, 3]
        );
        assert_eq!(
            wizard.edited_table.as_ref().unwrap().groups[0].levels[0].properties[1].cells,
            [800_000_000]
        );
        assert_eq!(wizard.edited_table.as_ref().unwrap().groups.len(), 1);
    }

    #[test]
    fn added_level_uses_exact_ordered_schema_of_heterogeneous_sibling() {
        let mut wizard = ready_wizard(900_000_000);
        let group = &mut wizard.edited_table.as_mut().unwrap().groups[0];
        group.levels[0].properties.push(GpuProperty {
            name: "qcom,acd-level".into(),
            cells: vec![1],
        });
        group.levels.push(GpuLevel {
            id: 1,
            properties: vec![
                GpuProperty {
                    name: "reg".into(),
                    cells: vec![1],
                },
                GpuProperty {
                    name: "qcom,gpu-freq".into(),
                    cells: vec![700_000_000],
                },
                GpuProperty {
                    name: "qcom,level".into(),
                    cells: vec![150],
                },
                GpuProperty {
                    name: "qcom,bus-freq".into(),
                    cells: vec![4],
                },
            ],
        });
        wizard.stock_table = wizard.edited_table.clone();
        let first_names = wizard.edited_table.as_ref().unwrap().groups[0].levels[0]
            .properties
            .iter()
            .map(|property| property.name.clone())
            .collect::<Vec<_>>();
        let template_names = wizard.edited_table.as_ref().unwrap().groups[0].levels[1]
            .properties
            .iter()
            .map(|property| property.name.clone())
            .collect::<Vec<_>>();

        assert!(wizard.add_level(0));

        let added_names = wizard.edited_table.as_ref().unwrap().groups[0].levels[2]
            .properties
            .iter()
            .map(|property| property.name.clone())
            .collect::<Vec<_>>();
        assert_eq!(added_names, template_names);
        assert_ne!(added_names, first_names);
    }

    #[test]
    fn final_level_cannot_be_removed_and_revert_restores_exact_stock() {
        let mut wizard = ready_wizard(700_000_000);
        let stock = wizard.stock_table.clone();
        let vote = GpuCellKey::level(0, 0, 2, 0);

        assert!(!wizard.remove_level(0, 0));
        assert!(wizard.edit_cell(vote, "300".into()));
        wizard.import_path = Some("import.txt".into());
        assert!(wizard.revert_edits());

        assert_eq!(wizard.edited_table, stock);
        assert!(wizard.cell_edits.is_empty());
        assert!(wizard.import_path.is_none());
        assert!(!wizard.edited_dirty);
    }

    #[test]
    fn mhz_display_and_parser_round_trip_exact_hz() {
        for frequency in [750_000_000, 231_234_567, u32::MAX] {
            let display = format_gpu_frequency_mhz(frequency);
            assert_eq!(parse_gpu_frequency_mhz(&display), Ok(frequency));
        }
        assert_eq!(parse_gpu_frequency_mhz("231.2345670"), Ok(231_234_567));
        assert_eq!(parse_gpu_frequency_mhz("750.5"), Ok(750_500_000));
        assert_eq!(parse_gpu_frequency_mhz("0x2EE"), Ok(750_000_000));
        assert!(parse_gpu_frequency_mhz("0x2EE.5").is_err());
        assert!(parse_gpu_frequency_mhz("750.0x5").is_err());
        assert!(parse_gpu_frequency_mhz("231.2345671").is_err());
    }

    #[test]
    fn matching_import_that_does_not_change_values_stays_clean() {
        let mut wizard = KonaBessWizard::default();
        wizard.apply_inspection_result(vec![candidate(1, Some("sun"), Some(700_000_000))], None);
        assert!(wizard.select_target(1));

        wizard
            .overwrite_edited_from_import(KonaBessExport {
                chip: "sun".into(),
                description: String::new(),
                table: table(700_000_000),
                import_warnings: vec![],
            })
            .unwrap();

        assert!(!wizard.edited_dirty);
    }

    #[test]
    fn import_naming_an_existing_chip_alias_is_accepted() {
        let mut wizard = KonaBessWizard::default();
        wizard.apply_inspection_result(vec![candidate(1, Some("sun"), Some(700_000_000))], None);
        assert!(wizard.select_target(1));

        wizard
            .overwrite_edited_from_import(KonaBessExport {
                chip: "tuna".into(),
                description: String::new(),
                table: table(900_000_000),
                import_warnings: vec![],
            })
            .unwrap();

        assert_eq!(wizard.stock_table, Some(table(700_000_000)));
        assert_eq!(wizard.edited_table, Some(table(900_000_000)));
        assert!(wizard.edited_dirty);
    }

    #[test]
    fn chip_mismatch_does_not_overwrite_the_working_copy() {
        let mut wizard = KonaBessWizard::default();
        wizard.apply_inspection_result(vec![candidate(1, Some("sun"), Some(700_000_000))], None);
        assert!(wizard.select_target(1));
        let edited = wizard.edited_table.clone();

        let error = wizard
            .overwrite_edited_from_import(KonaBessExport {
                chip: "pineapple".into(),
                description: String::new(),
                table: table(900_000_000),
                import_warnings: vec![],
            })
            .unwrap_err();

        assert_eq!(
            error,
            KonaBessImportError::ChipMismatch {
                expected: "sun".into(),
                actual: "pineapple".into(),
            }
        );
        assert_eq!(wizard.edited_table, edited);
        assert!(!wizard.edited_dirty);
    }

    #[test]
    fn probable_dtb_match_is_only_a_hint_until_explicitly_selected() {
        let mut wizard = KonaBessWizard::default();
        wizard.apply_inspection_result(
            vec![
                candidate(2, Some("sun"), Some(700_000_000)),
                candidate(7, Some("sun"), Some(900_000_000)),
            ],
            Some(7),
        );

        assert!(wizard.target_popup_open);
        assert!(wizard.target_popup_abandons_on_dismiss);
        assert!(wizard.is_probable_target(7));
        assert_eq!(wizard.selected_target_index, None);
        assert_eq!(wizard.stock_table, None);
        assert_eq!(wizard.edited_table, None);
        assert_eq!(wizard.confirm_target(), None);
        assert!(wizard.target_popup_open);

        assert!(wizard.select_target(7));
        assert_eq!(wizard.selected_target_index, Some(7));
        assert_eq!(wizard.stock_table, Some(table(900_000_000)));

        let mut unknown_chip_wizard = KonaBessWizard::default();
        unknown_chip_wizard
            .apply_inspection_result(vec![candidate(7, None, Some(900_000_000))], Some(7));

        assert!(unknown_chip_wizard.target_popup_open);
        assert!(unknown_chip_wizard.is_probable_target(7));
        assert_eq!(unknown_chip_wizard.selected_target_index, None);
        assert!(!unknown_chip_wizard.select_target(7));
    }

    #[test]
    fn unknown_probable_dtb_requires_manual_selection() {
        let mut wizard = KonaBessWizard::default();
        wizard
            .apply_inspection_result(vec![candidate(2, Some("sun"), Some(700_000_000))], Some(99));

        assert!(wizard.target_popup_open);
        assert_eq!(wizard.probable_target_index, None);
        assert_eq!(wizard.selected_target_index, None);
    }

    #[test]
    fn reset_removes_workspace_and_table_state() {
        let root = tempfile::tempdir().unwrap();
        let work_dir = root.path().join("work");
        std::fs::create_dir_all(&work_dir).unwrap();
        let mut wizard = KonaBessWizard {
            prepared: Some(KonaBessPrepared {
                vendor_boot: work_dir.join("vendor_boot.img"),
                vbmeta: work_dir.join("vbmeta.img"),
                backup_dir: root.path().join("backup"),
                slot_suffix: "_a".into(),
                probable_dtb_index: None,
                device_model: "TB323FU".into(),
                work_dir: work_dir.clone(),
            }),
            candidates: vec![candidate(2, Some("sun"), Some(700_000_000))],
            ..KonaBessWizard::default()
        };
        assert!(wizard.select_target(2));

        wizard.reset();

        assert!(!work_dir.exists());
        assert!(wizard.candidates.is_empty());
        assert_eq!(wizard.selected_target_index, None);
        assert!(wizard.stock_table.is_none());
        assert!(wizard.edited_table.is_none());
        assert!(!wizard.edited_dirty);
    }
}
