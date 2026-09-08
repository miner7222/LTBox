//! Shared widget style functions (text/button/container/rule styles). Extracted from `main.rs`.

use crate::*;
use iced::Theme;
use iced::widget::{
    button, checkbox, container, pick_list, scrollable, text_editor, text_input, toggler,
};
use theme::{is_dark, mix_color, with_alpha};

/// `on_surface_variant` — secondary labels / descriptions.
pub(crate) fn muted_style(t: &Theme) -> iced::widget::text::Style {
    iced::widget::text::Style {
        color: Some(pal_of(t).on_surface_variant),
    }
}

/// `on_surface` — primary foreground on surface containers.
pub(crate) fn on_surface_style(t: &Theme) -> iced::widget::text::Style {
    iced::widget::text::Style {
        color: Some(pal_of(t).on_surface),
    }
}

/// `primary` — accent emphasis (active labels, live-op markers).
pub(crate) fn accent_style(t: &Theme) -> iced::widget::text::Style {
    iced::widget::text::Style {
        color: Some(pal_of(t).primary),
    }
}

/// `success` — completion markers and "ok" status.
pub(crate) fn success_style(t: &Theme) -> iced::widget::text::Style {
    iced::widget::text::Style {
        color: Some(pal_of(t).success),
    }
}

/// `warning` — destructive-action callouts (e.g. full-flash confirm
/// step). Kept distinct from `error_style` so it reads as "heads up, not
/// a failure".
pub(crate) fn warning_style(t: &Theme) -> iced::widget::text::Style {
    iced::widget::text::Style {
        color: Some(pal_of(t).warning),
    }
}

pub(crate) fn warning_container_text_style(t: &Theme) -> iced::widget::text::Style {
    iced::widget::text::Style {
        color: Some(pal_of(t).on_warning_container),
    }
}

pub(crate) fn error_container_text_style(t: &Theme) -> iced::widget::text::Style {
    iced::widget::text::Style {
        color: Some(pal_of(t).on_error_container),
    }
}

pub(crate) fn m3_text_input_style(t: &Theme, status: text_input::Status) -> text_input::Style {
    let p = pal_of(t);
    let focused = matches!(status, text_input::Status::Focused { .. });
    let hovered = matches!(
        status,
        text_input::Status::Hovered | text_input::Status::Focused { is_hovered: true }
    );
    let disabled = matches!(status, text_input::Status::Disabled);
    // M3 outlined field: resting `outline`, hover `on_surface`, focus `primary`.
    // `outline_variant` (used before) is a divider tone ~1.5:1 on surface — the
    // resting control edge could vanish.
    let border_color = if focused {
        p.primary
    } else if hovered {
        p.on_surface
    } else {
        p.outline
    };
    text_input::Style {
        background: if disabled {
            with_alpha(p.on_surface, 0.04).into()
        } else {
            // M3 outlined fields have a transparent container — the
            // outline alone defines them. `surface_container_lowest` was
            // fine on light but in dark it is `0x0E0E13`, i.e. *darker*
            // than the card underneath, so every field read as a hole
            // punched in the panel instead of a control sitting on it.
            iced::Color::TRANSPARENT.into()
        },
        border: iced::Border {
            color: if disabled {
                with_alpha(p.on_surface, 0.12)
            } else {
                border_color
            },
            width: if focused { 2.0 } else { 1.0 },
            radius: theme::shape::SM.into(),
        },
        icon: if disabled {
            with_alpha(p.on_surface, 0.38)
        } else {
            p.on_surface_variant
        },
        placeholder: with_alpha(p.on_surface, if disabled { 0.38 } else { 0.62 }),
        value: if disabled {
            with_alpha(p.on_surface, 0.38)
        } else {
            p.on_surface
        },
        selection: with_alpha(p.primary, 0.30),
    }
}

/// M3 invalid outlined field: keep the normal input colors and state behavior,
/// but make the error edge the persistent, two-pixel validation signal.
pub(crate) fn m3_text_input_error_style(
    t: &Theme,
    status: text_input::Status,
) -> text_input::Style {
    let mut style = m3_text_input_style(t, status);
    style.border.color = pal_of(t).error;
    style.border.width = 2.0;
    style
}

pub(crate) fn m3_log_text_editor_style(
    t: &Theme,
    status: text_editor::Status,
) -> text_editor::Style {
    let p = pal_of(t);
    let disabled = matches!(status, text_editor::Status::Disabled);
    text_editor::Style {
        // Transparent so the log reads as the body of its card rather
        // than a second, darker slab inset into one.
        background: iced::Color::TRANSPARENT.into(),
        border: iced::Border {
            color: iced::Color::TRANSPARENT,
            width: 0.0,
            radius: 0.0.into(),
        },
        placeholder: with_alpha(p.on_surface, if disabled { 0.38 } else { 0.62 }),
        value: if disabled {
            with_alpha(p.on_surface, 0.38)
        } else {
            p.on_surface
        },
        selection: with_alpha(p.primary, 0.30),
    }
}

pub(crate) fn m3_pick_list_style(t: &Theme, status: pick_list::Status) -> pick_list::Style {
    let p = pal_of(t);
    let active = pick_list::Style {
        text_color: p.on_surface,
        placeholder_color: with_alpha(p.on_surface, 0.62),
        handle_color: p.on_surface_variant,
        // Transparent container, same reasoning as `m3_text_input_style`:
        // in dark, `surface_container_lowest` sits below the card tone
        // and the control reads as a hole rather than a raised field.
        background: iced::Color::TRANSPARENT.into(),
        // `outline` (not the ~1.5:1 `outline_variant` divider tone) so the
        // resting control edge clears the 3:1 UI-contrast threshold.
        border: iced::Border {
            color: p.outline,
            width: 1.0,
            radius: theme::shape::SM.into(),
        },
    };
    match status {
        pick_list::Status::Active => active,
        pick_list::Status::Hovered | pick_list::Status::Opened { .. } => pick_list::Style {
            border: iced::Border {
                color: p.primary,
                width: 1.0,
                radius: theme::shape::SM.into(),
            },
            ..active
        },
    }
}

pub(crate) fn m3_pick_list_menu_style(t: &Theme) -> iced::widget::overlay::menu::Style {
    let p = pal_of(t);
    iced::widget::overlay::menu::Style {
        // M3 exposed dropdown menus use an on-surface state layer for the
        // selected / hovered simple item instead of swapping surface tones.
        background: p.surface_container.into(),
        border: iced::Border {
            color: iced::Color::TRANSPARENT,
            width: 0.0,
            radius: theme::shape::SM.into(),
        },
        text_color: p.on_surface,
        selected_text_color: p.on_surface,
        selected_background: with_alpha(p.on_surface, theme::state::HOVER).into(),
        shadow: theme::elevation(2, is_dark(t)),
    }
}

/// One cell inside the Settings segmented controls. The shared outer
/// container draws the interactive `outline`; cells only paint selection and
/// state layers so adjacent borders never double up.
pub(crate) fn m3_segment_button_style(
    t: &Theme,
    status: button::Status,
    selected: bool,
    radius: iced::border::Radius,
) -> button::Style {
    let p = pal_of(t);
    let foreground = if selected {
        p.on_surface
    } else {
        p.on_surface_variant
    };
    let background = if selected {
        let alpha = theme::state_alpha(status);
        Some(mix_color(p.secondary_container, foreground, alpha).into())
    } else {
        theme::state_layer_bg(status, foreground).map(Into::into)
    };
    button::Style {
        background,
        text_color: foreground,
        border: iced::Border {
            radius,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// M3 switch treatment for the Settings system-font preference.
pub(crate) fn m3_settings_switch_style(t: &Theme, status: toggler::Status) -> toggler::Style {
    let p = pal_of(t);
    let (is_toggled, hovered, disabled) = match status {
        toggler::Status::Active { is_toggled } => (is_toggled, false, false),
        toggler::Status::Hovered { is_toggled } => (is_toggled, true, false),
        toggler::Status::Disabled { is_toggled } => (is_toggled, false, true),
    };
    let alpha = if disabled { 0.38 } else { 1.0 };
    let track = if is_toggled {
        if hovered {
            mix_color(p.primary, p.on_primary, theme::state::HOVER)
        } else {
            p.primary
        }
    } else if hovered {
        mix_color(p.surface_container_high, p.on_surface, theme::state::HOVER)
    } else {
        p.surface_container_high
    };
    let outline = if is_toggled { p.primary } else { p.outline };
    let thumb = if is_toggled { p.on_primary } else { p.outline };
    toggler::Style {
        background: with_alpha(track, alpha).into(),
        background_border_width: 2.0,
        background_border_color: with_alpha(outline, alpha),
        foreground: with_alpha(thumb, alpha).into(),
        foreground_border_width: 0.0,
        foreground_border_color: iced::Color::TRANSPARENT,
        text_color: None,
        border_radius: None,
        padding_ratio: 0.16,
    }
}

pub(crate) fn m3_checkbox_style(t: &Theme, status: checkbox::Status) -> checkbox::Style {
    let p = pal_of(t);
    let is_checked = match status {
        checkbox::Status::Active { is_checked }
        | checkbox::Status::Hovered { is_checked }
        | checkbox::Status::Disabled { is_checked } => is_checked,
    };
    let disabled = matches!(status, checkbox::Status::Disabled { .. });
    let hovered = matches!(status, checkbox::Status::Hovered { .. });
    if disabled {
        return checkbox::Style {
            background: with_alpha(p.on_surface, 0.12).into(),
            icon_color: with_alpha(p.on_surface, 0.38),
            border: iced::Border {
                color: with_alpha(p.on_surface, 0.38),
                width: 1.0,
                radius: theme::shape::XS.into(),
            },
            text_color: Some(with_alpha(p.on_surface, 0.38)),
        };
    }
    checkbox::Style {
        background: if is_checked {
            let bg = if hovered {
                mix_color(p.primary, p.on_primary, theme::state::HOVER)
            } else {
                p.primary
            };
            bg.into()
        } else {
            iced::Color::TRANSPARENT.into()
        },
        icon_color: if is_checked {
            p.on_primary
        } else {
            iced::Color::TRANSPARENT
        },
        border: iced::Border {
            color: if is_checked {
                p.primary
            } else if hovered {
                p.on_surface
            } else {
                p.outline
            },
            width: 2.0,
            radius: theme::shape::XS.into(),
        },
        text_color: Some(p.on_surface),
    }
}

pub(crate) fn m3_scrollable_style(t: &Theme, status: scrollable::Status) -> scrollable::Style {
    let p = pal_of(t);
    let hovered = matches!(status, scrollable::Status::Hovered { .. });
    let dragged = matches!(status, scrollable::Status::Dragged { .. });
    let rail = scrollable::Rail {
        background: Some(
            with_alpha(p.on_surface, if hovered || dragged { 0.08 } else { 0.04 }).into(),
        ),
        border: iced::Border {
            radius: theme::shape::FULL.into(),
            ..Default::default()
        },
        scroller: scrollable::Scroller {
            background: with_alpha(
                p.on_surface_variant,
                if dragged {
                    0.62
                } else if hovered {
                    0.48
                } else {
                    0.34
                },
            )
            .into(),
            border: iced::Border {
                radius: theme::shape::FULL.into(),
                ..Default::default()
            },
        },
    };
    scrollable::Style {
        container: container::Style::default(),
        vertical_rail: rail,
        horizontal_rail: rail,
        gap: None,
        auto_scroll: scrollable::AutoScroll {
            background: p.surface_container_high.into(),
            border: iced::Border {
                color: p.outline,
                width: 1.0,
                radius: theme::shape::FULL.into(),
            },
            shadow: theme::elevation(2, is_dark(t)),
            icon: p.on_surface,
        },
    }
}

/// Transparent button; tinted on hover. Used on dashboard cells.
pub(crate) fn dash_clickable_btn_style(t: &Theme, status: button::Status) -> button::Style {
    let p = pal_of(t);
    button::Style {
        background: theme::state_layer_bg(status, p.on_surface).map(Into::into),
        text_color: p.on_surface,
        border: iced::Border {
            radius: theme::shape::SM.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Dashboard device-info icon button: outlined at rest, tonal on hover.
pub(crate) fn dash_icon_btn_style(t: &Theme, status: button::Status) -> button::Style {
    let p = pal_of(t);
    let active = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: active.then_some(p.surface_container_low.into()),
        text_color: if active {
            p.primary
        } else {
            p.on_surface_variant
        },
        border: iced::Border {
            color: if active { p.primary } else { p.outline },
            width: 1.0,
            radius: theme::shape::SM.into(),
        },
        ..Default::default()
    }
}

/// M3 filled button — primary bg + state-layer overlay on hover/press.
pub(crate) fn md_filled_btn_style(t: &Theme, status: button::Status) -> button::Style {
    let p = pal_of(t);
    // M3 spec: disabled filled button = `on_surface @ 12%` background +
    // `on_surface @ 38%` label. Without this branch, dropping `on_press`
    // left the button looking identical to the active primary fill —
    // the only cue was the cursor not flipping to a pointer, which
    // users on touch / stable-pointer setups never noticed.
    if matches!(status, button::Status::Disabled) {
        return button::Style {
            background: Some(with_alpha(p.on_surface, 0.12).into()),
            text_color: with_alpha(p.on_surface, 0.38),
            border: iced::Border {
                radius: theme::shape::SM.into(),
                ..Default::default()
            },
            ..Default::default()
        };
    }
    let bg = mix_color(p.primary, p.on_primary, theme::state_alpha(status));
    button::Style {
        background: Some(bg.into()),
        text_color: p.on_primary,
        border: iced::Border {
            radius: theme::shape::SM.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

/// M3 text button — no fill, state layer on hover/press.
pub(crate) fn md_text_btn_style(t: &Theme, status: button::Status) -> button::Style {
    let p = pal_of(t);
    let bg_alpha = theme::state_alpha(status);
    button::Style {
        background: if bg_alpha > 0.0 {
            Some(with_alpha(p.primary, bg_alpha).into())
        } else {
            None
        },
        text_color: p.primary,
        border: iced::Border {
            radius: theme::shape::SM.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Filled button for primary actions inside warning banners. Uses the
/// warning role pair (`on_warning_container` fill / `warning_container`
/// label) with M3 state layers and the standard small control shape so the action
/// reads as the banner's primary control without borrowing the app-wide
/// primary fill.
pub(crate) fn banner_filled_btn_style(t: &Theme, status: button::Status) -> button::Style {
    let p = pal_of(t);
    if matches!(status, button::Status::Disabled) {
        return button::Style {
            background: Some(with_alpha(p.on_warning_container, 0.12).into()),
            text_color: with_alpha(p.on_warning_container, 0.38),
            border: iced::Border {
                radius: theme::shape::SM.into(),
                ..Default::default()
            },
            ..Default::default()
        };
    }
    // Filled on a warning-container surface: dark/light role inversion of
    // the container pair, with the on-color state layer mixed in.
    let bg = mix_color(
        p.on_warning_container,
        p.warning_container,
        theme::state_alpha(status),
    );
    button::Style {
        background: Some(bg.into()),
        text_color: p.warning_container,
        border: iced::Border {
            radius: theme::shape::SM.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Text button for warning banners ("Don't show again"). Uses the matching
/// warning on-container role and state layer. The default
/// `md_text_btn_style` uses the theme `primary`, which can be low-contrast
/// lavender on amber in dark mode — the visibility bug this fixes. The banner
/// background is theme-independent, so the on-color is too.
pub(crate) fn banner_text_btn_style(t: &Theme, status: button::Status) -> button::Style {
    let on_banner = pal_of(t).on_warning_container;
    let bg_alpha = theme::state_alpha(status);
    button::Style {
        background: if bg_alpha > 0.0 {
            Some(with_alpha(on_banner, bg_alpha).into())
        } else {
            None
        },
        text_color: on_banner,
        border: iced::Border {
            radius: theme::shape::SM.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Shared `Rule` styling so every shell-level divider (window
/// outline, title-bar bottom, sidebar-content split, status-bar
/// top) reads as the same hairline. Default rule color is
/// `background.strong` from iced's extended palette which is
/// noticeably darker than the M3 `outline_variant` used elsewhere.
pub(crate) fn shell_rule_style(t: &Theme) -> iced::widget::rule::Style {
    iced::widget::rule::Style {
        color: pal_of(t).outline_variant,
        radius: 0.0.into(),
        fill_mode: iced::widget::rule::FillMode::Full,
        snap: true,
    }
}

/// Border-less surface fill for the sidebar / status-bar shell
/// panels. Adjacent shells double up `iced::Border` lines when
/// each side has its own 1-px outline; per M3 nav-rail / bottom-app-
/// bar guidance each shared edge should carry exactly one divider,
/// drawn as an explicit `Rule` widget so the corners read as
/// clean T-junctions instead of a 2-px overlap.
pub(crate) fn panel_bg(t: &Theme) -> container::Style {
    let p = pal_of(t);
    container::Style {
        background: Some(p.surface_container_low.into()),
        ..Default::default()
    }
}

/// Inner container style for option / Browse cards. Transparent
/// background — the outer button paints a hover-aware fill via
/// [`sel_card_btn_style`]; this style only renders the rounded border
/// so the visual outline survives without blocking the button's
/// interactive bg.
pub(crate) fn sel_card_style(t: &Theme, selected: bool) -> container::Style {
    sel_card_style_for(t, selected, false)
}

/// [`sel_card_style`] with an accent switch. `destructive` swaps the
/// primary accent for the `error` role so an option that erases user
/// data or overwrites a partition is distinguishable from a safe one
/// *before* it is picked, not only in the confirm copy.
pub(crate) fn sel_card_style_for(t: &Theme, selected: bool, destructive: bool) -> container::Style {
    let p = pal_of(t);
    let accent = if destructive { p.error } else { p.primary };
    container::Style {
        background: None,
        border: iced::Border {
            // Interactive target edges use `outline`; selected and destructive
            // targets retain their semantic accent.
            color: if selected || destructive {
                accent
            } else {
                p.outline
            },
            width: if selected { 2.0 } else { 1.0 },
            radius: theme::shape::MD.into(),
        },
        ..Default::default()
    }
}

/// Outer button style for option / Browse cards. Border carries the same
/// MD radius as [`sel_card_style`] so the bg fill clips to the rounded
/// shape instead of bleeding out as a square.
///
/// The three states have to be rankable at a glance. They used to be
/// `primary @ 12%` (selected) vs `primary @ 8%` (hover) — a 4% delta that
/// left the 1 px → 2 px border as the only reliable cue, so a hovered
/// card read as selected. Selection now moves to its own opaque tonal
/// role (`secondary_container`) while hover stays a translucent state
/// layer over the resting fill, which keeps "pointer is here" and "this
/// is your choice" on separate visual channels.
pub(crate) fn sel_card_btn_style(
    t: &Theme,
    status: button::Status,
    selected: bool,
) -> button::Style {
    sel_card_btn_style_for(t, status, selected, false)
}

/// [`sel_card_btn_style`] with the same accent switch as
/// [`sel_card_style_for`]. A selected destructive option fills with
/// `error_container` rather than `secondary_container`.
pub(crate) fn sel_card_btn_style_for(
    t: &Theme,
    status: button::Status,
    selected: bool,
    destructive: bool,
) -> button::Style {
    let p = pal_of(t);
    let base = match (selected, destructive) {
        (true, true) => p.error_container,
        (true, false) => p.secondary_container,
        // Unselected rows sit one step below the card tone so the selected
        // row's container fill is what carries the contrast.
        (false, _) => p.surface_container_low,
    };
    let bg = theme::mix_color(base, p.on_surface, theme::state_alpha(status));
    button::Style {
        background: Some(bg.into()),
        text_color: if selected && destructive {
            p.on_error_container
        } else {
            p.on_surface
        },
        border: iced::Border {
            radius: theme::shape::MD.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}
