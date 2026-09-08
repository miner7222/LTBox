//! Shared dimensions for localized copy rendered in constrained slots.
//!
//! `tests/locale_guards.rs` imports this module directly so the regression
//! guard measures against the same budgets as the production widgets.

pub(crate) const WIZARD_LIST_CARD_HEIGHT: f32 = 56.0;
pub(crate) const WIZARD_LIST_MAX_WIDTH: f32 = 720.0;
pub(crate) const WIZARD_LIST_ICON_SIZE: f32 = 32.0;
/// List-row icon size for Lucide glyphs. A stroke glyph fills its em box, so it
/// reads much heavier than a brand logo at the same size; glyph rows use this
/// while logo rows use `WIZARD_LIST_ICON_SIZE`.
pub(crate) const WIZARD_LIST_GLYPH_ICON_SIZE: f32 = 24.0;
pub(crate) const WIZARD_LIST_LABEL_SIZE: f32 = 14.0;
pub(crate) const WIZARD_LIST_DESC_SIZE: f32 = 12.0;
pub(crate) const WIZARD_LIST_VERTICAL_PADDING: f32 = 6.0;
pub(crate) const WIZARD_LIST_HORIZONTAL_PADDING: f32 = 16.0;
pub(crate) const WIZARD_LIST_TEXT_GAP: f32 = 2.0;
pub(crate) const WIZARD_LIST_ICON_GAP: f32 = 12.0;
pub(crate) const WIZARD_STEP_HORIZONTAL_PADDING: f32 = 28.0;
pub(crate) const WIZARD_HELP_PANEL_WIDTH: f32 = 280.0;
pub(crate) const WIZARD_HELP_PANEL_MIN_WIDTH: f32 = 200.0;
pub(crate) const WIZARD_HELP_PANEL_GAP: f32 = 22.0;

pub(crate) const SETTINGS_PICK_LIST_WIDTH: f32 = 176.0;
pub(crate) const SETTINGS_PICK_LIST_TEXT_SIZE: f32 = 14.0;
pub(crate) const SETTINGS_GRID_MAX_WIDTH: f32 = 840.0;
pub(crate) const SETTINGS_VALUE_FIELD_WIDTH: f32 = 280.0;
pub(crate) const SETTINGS_SEGMENT_TEXT_SIZE: f32 = 12.0;
pub(crate) const SETTINGS_SEGMENT_HORIZONTAL_PADDING: f32 = 13.0;

pub(crate) const M3_FIELD_PADDING: iced::Padding = iced::Padding {
    top: 12.0,
    right: 16.0,
    bottom: 12.0,
    left: 16.0,
};

pub(crate) const M3_BUTTON_H_PADDING: f32 = 16.0;

/// Dialog width scale. Callers keep ownership of their content layout while
/// choosing the width by kind: short confirmation/input, choice/detail, editor.
/// These live here rather than in `theme` because the locale guards include
/// this module standalone and must measure the same budgets the widgets use.
pub(crate) const DIALOG_WIDTH_SM: f32 = 400.0;
pub(crate) const DIALOG_WIDTH_MD: f32 = 520.0;
pub(crate) const DIALOG_WIDTH_LG: f32 = 720.0;

/// Side inset shared by a dialog's header, body and footer.
pub(crate) const DIALOG_H_PADDING: f32 = 20.0;

pub(crate) const DIRECT_UPDATE_DIALOG_WIDTH: f32 = DIALOG_WIDTH_MD;
pub(crate) const DIRECT_UPDATE_DIALOG_ACTION_SPACING: f32 = 8.0;
pub(crate) const DIRECT_UPDATE_DIALOG_ACTION_SIZE: f32 = 14.0;

pub(crate) const REGION_TARGET_POPUP_WIDTH: f32 = DIALOG_WIDTH_MD;
pub(crate) const REGION_TARGET_POPUP_TITLE_SIZE: f32 = 16.0;
pub(crate) const REGION_TARGET_POPUP_ACTION_SIZE: f32 = 14.0;
