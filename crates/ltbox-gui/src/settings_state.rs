//! Settings, rollback-editor and workflow configuration state.

use crate::*;

/// Theme preference. `System` reads the OS setting via
/// `theme_detect::system_prefers_dark`; Light/Dark override.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
}
impl ThemeChoice {
    pub(crate) fn label_key(&self) -> &'static str {
        match self {
            Self::System => "theme_system",
            Self::Light => "theme_light",
            Self::Dark => "theme_dark",
        }
    }
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
    pub(crate) fn from_code(c: &str) -> Option<Self> {
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

pub(crate) fn concise_error_summary(error: &str, max_chars: usize) -> String {
    let summary = error
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    // Keep the first diagnostic sentence, excluding follow-up instructions and
    // nested details. A period inside a filename, version or URL is not a stop.
    let end = summary
        .char_indices()
        .find_map(|(index, ch)| {
            let rest = &summary[index + ch.len_utf8()..];
            let sentence_end = matches!(ch, '。' | '！' | '？')
                || (matches!(ch, '.' | '!' | '?') && rest.starts_with(char::is_whitespace));
            let detail_start = (ch == ':' && rest.starts_with(char::is_whitespace)) || ch == '：';
            if sentence_end {
                Some(index + ch.len_utf8())
            } else if detail_start && index > 0 {
                Some(index)
            } else {
                None
            }
        })
        .unwrap_or(summary.len());
    let summary = summary[..end].trim();
    if summary.chars().count() <= max_chars {
        return summary.to_owned();
    }
    if max_chars == 0 {
        return String::new();
    }

    let mut truncated: String = summary.chars().take(max_chars - 1).collect();
    truncated.push('…');
    truncated
}

pub(crate) fn busy_navigation_target(busy: bool, busy_view: Option<View>) -> Option<View> {
    if busy { busy_view } else { None }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum RollbackSetting {
    On,
    Auto,
    Manual,
    #[default]
    Off,
}
impl RollbackSetting {
    pub(crate) fn label_key(&self) -> &'static str {
        match self {
            Self::On => "rollback_on",
            Self::Auto => "rollback_auto",
            Self::Manual => "rollback_manual",
            Self::Off => "rollback_off",
        }
    }
    /// Map the wizard setting to the worker's rollback mode.
    pub(crate) fn to_mode(self) -> ltbox_patch::rollback::RollbackMode {
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
pub(crate) struct SettingsState {
    pub(crate) language: Language,
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

/// Sort marker for a table column head.
///
/// The active column shows its sort direction; the others show an idle,
/// dimmed "sortable" arrow.
pub(crate) fn parts_sort_marker(is_active: bool, desc: bool) -> Element<'static, Message> {
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

/// Sortable header cell for the FlashParts / DumpParts partition table:
/// `label` plus the sort marker; click fires `msg`.
pub(crate) fn parts_sort_header(
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
pub(crate) fn format_bytes_auto(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        ltbox_core::tr_args!("unit_gigabytes", value = format!("{:.2}", b / GB))
    } else if b >= MB {
        ltbox_core::tr_args!("unit_megabytes", value = format!("{:.2}", b / MB))
    } else if b >= KB {
        ltbox_core::tr_args!("unit_kilobytes", value = format!("{:.2}", b / KB))
    } else {
        ltbox_core::tr_args!("unit_bytes", value = bytes.to_string())
    }
}
