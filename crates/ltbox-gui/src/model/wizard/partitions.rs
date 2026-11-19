//! Flash Partitions and Dump Partitions wizard state (Advanced).

use super::*;
use std::collections::HashSet;

/// The only three actions a partition row can represent.
///
/// `Write` may await a file, while `Skip` and `Erase` never own one.
/// Keeping that invariant in [`FlashPartRow`] prevents stale images from being
/// written after a user changes a row to erase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum FlashRowState {
    #[default]
    Skip,
    Write,
    Erase,
}

/// One GPT entry surfaced in the wizard table.
#[derive(Debug, Clone)]
pub(crate) struct FlashPartRow {
    pub(crate) lun: u8,
    pub(crate) label: String,
    pub(crate) start_sector: u64,
    pub(crate) num_sectors: u64,
    pub(crate) size_bytes: u64,
    pub(crate) file_path: Option<String>,
    pub(crate) state: FlashRowState,
}

impl FlashPartRow {
    /// Assigning an image selects `Write` directly.
    pub(crate) fn assign_file(&mut self, path: String) {
        self.file_path = Some(path);
        self.state = FlashRowState::Write;
    }

    /// Removing an assigned image returns the row to the assignable skip state.
    pub(crate) fn clear_file(&mut self) {
        self.file_path = None;
        self.state = FlashRowState::Skip;
    }

    /// Cycle independently of the picker. Entering erase always drops its file.
    pub(crate) fn advance_action(&mut self) {
        match self.state {
            FlashRowState::Skip => self.state = FlashRowState::Write,
            FlashRowState::Write => {
                self.file_path = None;
                self.state = FlashRowState::Erase;
            }
            FlashRowState::Erase => {
                self.file_path = None;
                self.state = FlashRowState::Skip;
            }
        }
    }
}

#[cfg(test)]
mod flash_part_row_tests {
    use super::*;

    fn row() -> FlashPartRow {
        FlashPartRow {
            lun: 0,
            label: "userdata".to_string(),
            start_sector: 0,
            num_sectors: 1,
            size_bytes: 512,
            file_path: None,
            state: FlashRowState::Skip,
        }
    }

    #[test]
    fn assignment_and_action_transitions_preserve_file_invariants() {
        let mut row = row();
        row.assign_file("userdata.img".to_string());
        assert_eq!(row.state, FlashRowState::Write);
        assert_eq!(row.file_path.as_deref(), Some("userdata.img"));

        row.advance_action();
        assert_eq!(row.state, FlashRowState::Erase);
        assert_eq!(row.file_path, None);

        row.advance_action();
        assert_eq!(row.state, FlashRowState::Skip);
        assert_eq!(row.file_path, None);
    }

    #[test]
    fn clearing_any_selected_file_returns_to_skip() {
        let mut row = row();
        row.assign_file("boot.img".to_string());
        row.clear_file();
        assert_eq!(row.state, FlashRowState::Skip);
        assert_eq!(row.file_path, None);
    }

    #[test]
    fn checkbox_can_select_erase_without_an_image() {
        let mut row = row();
        for state in [
            FlashRowState::Write,
            FlashRowState::Erase,
            FlashRowState::Skip,
        ] {
            row.advance_action();
            assert_eq!(row.state, state);
            assert_eq!(row.file_path, None);
        }
    }
}

/// Live table filter: case-insensitive substring match on the partition
/// label. A blank query matches every row.
pub(crate) fn partition_label_matches(label: &str, query: &str) -> bool {
    let query = query.trim();
    query.is_empty() || label.to_lowercase().contains(&query.to_lowercase())
}

/// Column the partition table is sorted by; a header click fires
/// `*SortBy(col)`, and clicking the active column toggles direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum PartsSortColumn {
    #[default]
    Lun,
    Label,
    Start,
    Size,
    /// File-path column; FlashParts only (DumpParts has none).
    File,
}

#[derive(Default)]
pub(crate) struct FlashPartsWizard {
    pub(crate) step: usize, // 0=Loader, 1=Select, 2=Confirm, 3=Exec
    pub(crate) loader_path: Option<String>,
    /// Loader-resolution failure, kept apart from any scan error so a
    /// refused pick does not overwrite why the last scan failed.
    pub(crate) loader_error: Option<String>,
    pub(crate) rows: Vec<FlashPartRow>,
    pub(crate) scanning: bool,
    pub(crate) scan_error: Option<String>,
    /// Connection state captured when the GPT scan started. The table-step
    /// leading action must not infer this from the live post-scan EDL state.
    pub(crate) entry_connection: Option<ConnectionStatus>,
    pub(crate) sort_col: PartsSortColumn,
    /// `true` → descending. Defaults to ascending so the first layout roughly
    /// follows GPT order (LUN, then label).
    pub(crate) sort_desc: bool,
    /// Select-step table filter; hides rows without changing their state.
    pub(crate) search: String,
    /// Select-step view filter: show only rows that are not `Skip`. Like
    /// `search`, it hides rows without changing what is written or erased.
    pub(crate) selected_only: bool,
    /// Rows that were non-`Skip` when `selected_only` was switched on. They
    /// stay visible even if cycled back to `Skip`, so the table does not
    /// shift under the cursor mid-click. Cleared with the filter.
    pub(crate) selected_only_pinned: HashSet<FlashRowKey>,
}

/// Stable row identity for the visible-set pin; survives re-sorting.
pub(crate) type FlashRowKey = (u8, String, u64);

fn flash_row_key(row: &FlashPartRow) -> FlashRowKey {
    (row.lun, row.label.clone(), row.start_sector)
}

pub(crate) const FLASH_PARTS_STEPS: &[&str] = &[
    "edl_loader_label",
    "flash_parts_step_select",
    "flash_step_confirm",
    "flash_step_flash",
];

impl FlashPartsWizard {
    /// Rows the table shows under the current search and the Selected-only
    /// filter, with their index in `rows` so row messages keep addressing the
    /// right entry. A `Write` row still waiting for its image counts as
    /// selected, so the row that blocks Next never hides. A row that was
    /// selected when the filter went on stays until the filter is turned off.
    pub(crate) fn visible_rows(&self) -> impl Iterator<Item = (usize, &FlashPartRow)> {
        self.rows.iter().enumerate().filter(|(_, row)| {
            partition_label_matches(&row.label, &self.search)
                && (!self.selected_only
                    || row.state != FlashRowState::Skip
                    || self.selected_only_pinned.contains(&flash_row_key(row)))
        })
    }

    /// Switch the Selected-only filter. Turning it on pins the rows that are
    /// selected right now; turning it off (or re-applying) drops the pins.
    pub(crate) fn set_selected_only(&mut self, on: bool) {
        self.selected_only_pinned.clear();
        if on {
            self.selected_only_pinned = self
                .rows
                .iter()
                .filter(|row| row.state != FlashRowState::Skip)
                .map(flash_row_key)
                .collect();
        }
        self.selected_only = on;
    }

    pub(crate) fn active_rows(&self) -> Vec<FlashPartRow> {
        self.rows
            .iter()
            .filter(|r| match r.state {
                FlashRowState::Write => r.file_path.is_some(),
                FlashRowState::Erase => true,
                FlashRowState::Skip => false,
            })
            .cloned()
            .collect()
    }

    /// Stable-sort `rows` by `sort_col` / `sort_desc`, tie-breaking on
    /// (lun, label) for a deterministic order.
    pub(crate) fn apply_sort(&mut self) {
        let col = self.sort_col;
        let desc = self.sort_desc;
        self.rows.sort_by(|a, b| {
            let ord = match col {
                PartsSortColumn::Lun => a.lun.cmp(&b.lun),
                // ASCII byte order: uppercase sorts before lowercase.
                PartsSortColumn::Label => a.label.cmp(&b.label),
                PartsSortColumn::Start => a.start_sector.cmp(&b.start_sector),
                PartsSortColumn::Size => a.size_bytes.cmp(&b.size_bytes),
                PartsSortColumn::File => a
                    .file_path
                    .as_deref()
                    .unwrap_or("")
                    .cmp(b.file_path.as_deref().unwrap_or("")),
            };
            let ord = ord
                .then_with(|| a.lun.cmp(&b.lun))
                .then_with(|| a.label.cmp(&b.label));
            if desc { ord.reverse() } else { ord }
        });
    }

    /// Header click: toggle direction on the active column, otherwise
    /// switch to the new column ascending.
    pub(crate) fn toggle_sort(&mut self, col: PartsSortColumn) {
        if self.sort_col == col {
            self.sort_desc = !self.sort_desc;
        } else {
            self.sort_col = col;
            self.sort_desc = false;
        }
        self.apply_sort();
    }
}

impl Wizard for FlashPartsWizard {
    fn step(&self) -> usize {
        self.step
    }
    fn step_mut(&mut self) -> &mut usize {
        &mut self.step
    }
    fn step_count(&self) -> usize {
        FLASH_PARTS_STEPS.len()
    }
    fn can_next(&self) -> bool {
        match self.step {
            0 => self.loader_path.is_some() && self.loader_error.is_none() && !self.scanning,
            1 | 2 => {
                self.rows
                    .iter()
                    .all(|r| r.state != FlashRowState::Write || r.file_path.is_some())
                    && self.rows.iter().any(|r| match r.state {
                        FlashRowState::Write => r.file_path.is_some(),
                        FlashRowState::Erase => true,
                        FlashRowState::Skip => false,
                    })
            }
            _ => false,
        }
    }
}

/// Scan-phase result carried in a single message. Same shape as the
/// DumpParts variant but with the Flash row type.
#[derive(Debug, Clone, Default)]
pub(crate) struct FlashPartsScanResult {
    pub(crate) logs: Vec<String>,
    pub(crate) rows: Vec<FlashPartRow>,
    pub(crate) error: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct DumpPartRow {
    pub(crate) lun: u8,
    pub(crate) label: String,
    pub(crate) start_sector: u64,
    pub(crate) num_sectors: u64,
    pub(crate) size_bytes: u64,
    pub(crate) selected: bool,
}

/// Scan-phase result carried in a single message.
#[derive(Debug, Clone, Default)]
pub(crate) struct DumpPartsScanResult {
    pub(crate) logs: Vec<String>,
    pub(crate) rows: Vec<DumpPartRow>,
    pub(crate) error: Option<String>,
}

#[derive(Default)]
pub(crate) struct DumpPartsWizard {
    pub(crate) step: usize, // 0=Loader, 1=Select, 2=Exec
    pub(crate) loader_path: Option<String>,
    /// Loader-resolution failure, kept apart from any scan error so a
    /// refused pick does not overwrite why the last scan failed.
    pub(crate) loader_error: Option<String>,
    pub(crate) rows: Vec<DumpPartRow>,
    pub(crate) output_dir: Option<String>,
    pub(crate) scanning: bool,
    pub(crate) scan_error: Option<String>,
    /// Connection state captured when the GPT scan started. The table-step
    /// leading action must not infer this from the live post-scan EDL state.
    pub(crate) entry_connection: Option<ConnectionStatus>,
    pub(crate) sort_col: PartsSortColumn,
    pub(crate) sort_desc: bool,
    /// Select-step table filter; hides rows without changing their selection.
    pub(crate) search: String,
}

pub(crate) const DUMP_PARTS_STEPS: &[&str] = &[
    "edl_loader_label",
    "dump_parts_step_select",
    "dump_parts_step_dump",
];

impl DumpPartsWizard {
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }
    pub(crate) fn back(&mut self) {
        if self.step > 0 {
            self.step -= 1;
        }
    }
    pub(crate) fn can_next(&self) -> bool {
        match self.step {
            0 => self.loader_path.is_some() && self.loader_error.is_none() && !self.scanning,
            1 => self.rows.iter().any(|r| r.selected),
            _ => false,
        }
    }
    pub(crate) fn selected_rows(&self) -> Vec<DumpPartRow> {
        self.rows.iter().filter(|r| r.selected).cloned().collect()
    }

    /// Rows the table shows under the current search, with their index in
    /// `rows` so row messages keep addressing the right entry.
    pub(crate) fn visible_rows(&self) -> impl Iterator<Item = (usize, &DumpPartRow)> {
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, row)| partition_label_matches(&row.label, &self.search))
    }

    /// Header checkbox state: every visible row is selected.
    pub(crate) fn all_visible_selected(&self) -> bool {
        let mut visible = self.visible_rows().peekable();
        visible.peek().is_some() && visible.all(|(_, row)| row.selected)
    }

    /// Header checkbox: select every visible row, or clear them when all are
    /// already selected. Rows hidden by the search keep their selection.
    pub(crate) fn toggle_visible(&mut self) {
        let target = !self.all_visible_selected();
        for row in self.rows.iter_mut() {
            if partition_label_matches(&row.label, &self.search) {
                row.selected = target;
            }
        }
    }

    pub(crate) fn apply_sort(&mut self) {
        let col = self.sort_col;
        let desc = self.sort_desc;
        self.rows.sort_by(|a, b| {
            let ord = match col {
                PartsSortColumn::Lun => a.lun.cmp(&b.lun),
                // ASCII byte order: uppercase sorts before lowercase.
                PartsSortColumn::Label => a.label.cmp(&b.label),
                PartsSortColumn::Start => a.start_sector.cmp(&b.start_sector),
                PartsSortColumn::Size => a.size_bytes.cmp(&b.size_bytes),
                // DumpParts has no file column; behave as Lun fallback.
                PartsSortColumn::File => a.lun.cmp(&b.lun),
            };
            let ord = ord
                .then_with(|| a.lun.cmp(&b.lun))
                .then_with(|| a.label.cmp(&b.label));
            if desc { ord.reverse() } else { ord }
        });
    }

    pub(crate) fn toggle_sort(&mut self, col: PartsSortColumn) {
        if self.sort_col == col {
            self.sort_desc = !self.sort_desc;
        } else {
            self.sort_col = col;
            self.sort_desc = false;
        }
        self.apply_sort();
    }
}

#[cfg(test)]
mod search_tests {
    use super::*;

    fn dump_row(label: &str, selected: bool) -> DumpPartRow {
        DumpPartRow {
            lun: 0,
            label: label.to_string(),
            start_sector: 0,
            num_sectors: 1,
            size_bytes: 512,
            selected,
        }
    }

    #[test]
    fn label_search_is_trimmed_case_insensitive_substring() {
        assert!(partition_label_matches("boot_a", ""));
        assert!(partition_label_matches("boot_a", "   "));
        assert!(partition_label_matches("vendor_boot_a", "BOOT"));
        assert!(partition_label_matches("vendor_boot_a", " boot_a "));
        assert!(!partition_label_matches("userdata", "boot"));
    }

    #[test]
    fn visible_rows_keep_their_original_indices() {
        let wizard = DumpPartsWizard {
            rows: vec![
                dump_row("abl_a", false),
                dump_row("boot_a", false),
                dump_row("boot_b", false),
            ],
            search: "boot".to_string(),
            ..Default::default()
        };
        let indices: Vec<_> = wizard.visible_rows().map(|(idx, _)| idx).collect();
        assert_eq!(indices, [1, 2]);
    }

    #[test]
    fn select_all_only_touches_visible_rows() {
        let mut wizard = DumpPartsWizard {
            rows: vec![
                dump_row("abl_a", true),
                dump_row("boot_a", false),
                dump_row("boot_b", false),
            ],
            search: "boot".to_string(),
            ..Default::default()
        };
        assert!(!wizard.all_visible_selected());

        wizard.toggle_visible();
        assert!(wizard.rows.iter().all(|row| row.selected));
        assert!(wizard.all_visible_selected());

        wizard.toggle_visible();
        let selected: Vec<_> = wizard.rows.iter().map(|row| row.selected).collect();
        assert_eq!(selected, [true, false, false]);
    }

    fn flash_row(label: &str, state: FlashRowState, file: Option<&str>) -> FlashPartRow {
        FlashPartRow {
            lun: 0,
            label: label.to_string(),
            start_sector: 0,
            num_sectors: 1,
            size_bytes: 512,
            file_path: file.map(str::to_string),
            state,
        }
    }

    fn flash_wizard() -> FlashPartsWizard {
        FlashPartsWizard {
            step: 1,
            rows: vec![
                flash_row("abl_a", FlashRowState::Skip, None),
                flash_row("boot_a", FlashRowState::Write, Some("boot.img")),
                flash_row("boot_b", FlashRowState::Skip, None),
                flash_row("vendor_boot_a", FlashRowState::Write, None),
                flash_row("userdata", FlashRowState::Erase, None),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn selected_only_shows_every_non_skip_row_with_its_original_index() {
        let mut wizard = flash_wizard();
        wizard.selected_only = true;
        let indices: Vec<_> = wizard.visible_rows().map(|(idx, _)| idx).collect();
        // The pending Write (3) stays visible: it is the row blocking Next.
        assert_eq!(indices, [1, 3, 4]);
    }

    #[test]
    fn selected_only_combines_with_the_search() {
        let mut wizard = flash_wizard();
        wizard.selected_only = true;
        wizard.search = "boot".to_string();
        let indices: Vec<_> = wizard.visible_rows().map(|(idx, _)| idx).collect();
        assert_eq!(indices, [1, 3]);

        wizard.selected_only = false;
        let indices: Vec<_> = wizard.visible_rows().map(|(idx, _)| idx).collect();
        assert_eq!(indices, [1, 2, 3]);
    }

    #[test]
    fn selected_only_never_changes_what_is_written_or_erased() {
        let snapshot = |wizard: &FlashPartsWizard| {
            let states: Vec<_> = wizard
                .rows
                .iter()
                .map(|row| (row.label.clone(), row.state, row.file_path.clone()))
                .collect();
            let active: Vec<_> = wizard
                .active_rows()
                .into_iter()
                .map(|row| (row.label, row.state, row.file_path))
                .collect();
            (states, active, wizard.can_next())
        };
        let mut wizard = flash_wizard();
        let before = snapshot(&wizard);
        wizard.selected_only = true;
        wizard.search = "zzz".to_string();
        assert_eq!(wizard.visible_rows().count(), 0);
        assert_eq!(snapshot(&wizard), before);

        // Clearing the pending Write makes the run startable either way.
        wizard.rows[3].clear_file();
        let with_filter = snapshot(&wizard);
        wizard.selected_only = false;
        wizard.search.clear();
        assert_eq!(snapshot(&wizard), with_filter);
        assert!(with_filter.2);
    }

    #[test]
    fn selected_only_starts_off_and_reset_clears_it() {
        let mut wizard = flash_wizard();
        assert!(!wizard.selected_only);
        wizard.set_selected_only(true);
        assert!(!wizard.selected_only_pinned.is_empty());
        wizard.reset();
        assert!(!wizard.selected_only);
        assert!(wizard.selected_only_pinned.is_empty());
    }

    #[test]
    fn selected_only_keeps_a_row_visible_after_it_cycles_back_to_skip() {
        let mut wizard = flash_wizard();
        wizard.set_selected_only(true);
        let labels = |wizard: &FlashPartsWizard| -> Vec<String> {
            wizard
                .visible_rows()
                .map(|(_, r)| r.label.clone())
                .collect()
        };
        assert_eq!(labels(&wizard), ["boot_a", "vendor_boot_a", "userdata"]);

        // Erase -> Skip on the last row: it must not vanish or shift others.
        wizard.rows[4].advance_action();
        assert_eq!(wizard.rows[4].state, FlashRowState::Skip);
        assert_eq!(labels(&wizard), ["boot_a", "vendor_boot_a", "userdata"]);

        // Turning the filter off and on again re-pins from the current state.
        wizard.set_selected_only(false);
        assert!(wizard.selected_only_pinned.is_empty());
        wizard.set_selected_only(true);
        assert_eq!(labels(&wizard), ["boot_a", "vendor_boot_a"]);

        // The visible set never changes what the worker receives.
        assert_eq!(wizard.active_rows().len(), 1);
    }

    #[test]
    fn select_all_is_unchecked_when_nothing_is_visible() {
        let mut wizard = DumpPartsWizard {
            rows: vec![dump_row("boot_a", true)],
            search: "modem".to_string(),
            ..Default::default()
        };
        assert!(!wizard.all_visible_selected());
        wizard.toggle_visible();
        assert!(wizard.rows[0].selected);
    }
}
