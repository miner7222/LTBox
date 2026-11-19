//! Reusable view components (dialogs, cards, step bar, icon tiles, lucide helpers).

use crate::focus_button::{self as button, button};
use crate::*;
use iced::widget::{self, Space, column, container, row, text};
use iced::{Element, Length, Theme};
use theme::with_alpha;

/// Add 8px to the step frame's 16px gap so picker fields sit 24px below the title.
pub(crate) const PICKER_BODY_PADDING: iced::Padding = iced::Padding {
    top: 8.0,
    right: 28.0,
    bottom: 28.0,
    left: 28.0,
};

/// Centered M3 dialog card on a scrim; `inner` owns padding/width. MODAL: the
/// layer is wrapped in `opaque`, so nothing behind it reacts. Use for confirm
/// dialogs and popups that must block the panel behind them.
pub(crate) fn m3_dialog(inner: Element<'_, Message>) -> Element<'_, Message> {
    // The scrim only paints; `opaque` is what stops hover/click falling through
    // the parent `Stack`. The card's own buttons still receive clicks.
    iced::widget::opaque(m3_dialog_layers(inner))
}

/// Like [`m3_dialog`] but MODELESS: no `opaque`, so pointer events fall through
/// the scrim to the panel behind. Used by the busy progress dialog so a
/// long-running flash doesn't trap the user.
pub(crate) fn m3_dialog_modeless(inner: Element<'_, Message>) -> Element<'_, Message> {
    m3_dialog_layers(inner)
}

/// Twelve-pixel label placed directly above an outlined dialog input.
pub(crate) fn dialog_field_label<'a>(label: impl Into<String>) -> Element<'a, Message> {
    text(label.into())
        .size(theme::text_size::BODY_SMALL)
        .style(muted_style)
        .into()
}

/// Inline validation feedback paired with an error-outlined field.
pub(crate) fn dialog_field_error<'a>(message: impl Into<String>) -> Element<'a, Message> {
    row![
        lucide_error(icon::field_error(), 14.0),
        text(message.into())
            .size(theme::text_size::BODY_SMALL)
            .style(|t: &Theme| iced::widget::text::Style {
                color: Some(pal_of(t).error),
            }),
    ]
    .spacing(6)
    .align_y(iced::Alignment::Center)
    .into()
}

/// Standard dialog anatomy. Boundary rules are opt-in and belong only to
/// bodies that actually scroll beneath a fixed header/footer.
pub(crate) fn dialog_sections<'a>(
    header: Element<'a, Message>,
    body: Element<'a, Message>,
    footer: Element<'a, Message>,
    width: f32,
    scrolling: bool,
) -> Element<'a, Message> {
    let mut content = column![
        container(header)
            .padding(iced::Padding {
                top: 24.0,
                right: theme::DIALOG_H_PADDING,
                bottom: 0.0,
                left: theme::DIALOG_H_PADDING,
            })
            .width(Length::Fill)
    ];
    if scrolling {
        content = content.push(widget::rule::horizontal(1).style(shell_rule_style));
    }
    content = content.push(
        container(body)
            .padding(iced::Padding {
                top: 16.0,
                right: theme::DIALOG_H_PADDING,
                bottom: 0.0,
                left: theme::DIALOG_H_PADDING,
            })
            .width(Length::Fill),
    );
    if scrolling {
        content = content.push(widget::rule::horizontal(1).style(shell_rule_style));
    }
    content
        .push(
            container(footer)
                .padding(iced::Padding {
                    top: 24.0,
                    right: theme::DIALOG_H_PADDING,
                    bottom: 24.0,
                    left: theme::DIALOG_H_PADDING,
                })
                .width(Length::Fill),
        )
        .spacing(0)
        .width(Length::Fixed(width))
        .into()
}

/// A dialog selection uses the same radio and state colors as wizard choices.
pub(crate) fn dialog_choice<'a>(
    label: impl Into<String>,
    description: Option<String>,
    selected: bool,
    action: Option<Message>,
) -> Element<'a, Message> {
    let enabled = action.is_some();
    let mut copy = column![
        text(label.into())
            .size(theme::text_size::BODY_MEDIUM)
            .font_maybe(selected.then_some(theme::emphasis::medium()))
    ]
    .spacing(4)
    .width(Length::Fill);
    let height = if description.is_some() { 72.0 } else { 56.0 };
    if let Some(description) = description {
        copy = copy.push(
            text(description)
                .size(theme::text_size::BODY_SMALL)
                .style(muted_style),
        );
    }
    button(container(
        row![
            selection_radio(selected, enabled, false),
            copy,
            Space::new().height(height - 16.0)
        ]
        .spacing(16)
        .align_y(iced::Alignment::Center),
    ))
    .padding([8, 16])
    .width(Length::Fill)
    .on_press_maybe(action)
    .style(move |t, status| dialog_choice_style(t, status, selected))
    .into()
}

pub(crate) fn m3_log_text_field<'a>(
    label: impl Into<String>,
    editor: Element<'a, Message>,
) -> Element<'a, Message> {
    m3_log_text_field_with_action(label, None, editor)
}

pub(crate) fn m3_log_text_field_with_action<'a>(
    label: impl Into<String>,
    action: Option<Element<'a, Message>>,
    editor: Element<'a, Message>,
) -> Element<'a, Message> {
    // Titled like the other dashboard cards rather than as an M2 filled text
    // field: a `primary` active indicator at the bottom of a full-height
    // read-only log would sit hundreds of pixels from its caption, marking
    // "focus" on a surface that is never focused.
    let label = label.into();
    let mut label_content = row![
        text(label)
            .size(theme::text_size::BODY_MEDIUM)
            .font(theme::emphasis::medium())
            .line_height(1.0)
            .style(muted_style),
        Space::new().width(Length::Fill),
    ]
    .spacing(8.0)
    .align_y(iced::Alignment::Center);
    let has_action = action.is_some();
    if let Some(action) = action {
        label_content = label_content.push(action);
    }
    let label_padding = if has_action {
        iced::Padding {
            top: 8.0,
            right: 12.0,
            bottom: 4.0,
            left: 18.0,
        }
    } else {
        iced::Padding {
            top: 12.0,
            right: 18.0,
            bottom: 8.0,
            left: 18.0,
        }
    };
    let label_row = container(label_content)
        .padding(label_padding)
        .width(Length::Fill);

    let field = column![
        label_row,
        container(editor).width(Length::Fill).height(Length::Fill),
    ]
    .spacing(0)
    .height(Length::Fill)
    .width(Length::Fill);

    container(field)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|t: &Theme| {
            theme::surface_card_style(t, theme::SurfaceLevel::Default, theme::shape::MD)
        })
        .into()
}

/// Shared scrim + centered card layers behind [`m3_dialog`] /
/// [`m3_dialog_modeless`]. The scrim only paints its dim background (in iced a
/// plain `container` does not capture pointer events); modality is decided by
/// the caller wrapping this in `opaque` or not.
fn m3_dialog_layers(inner: Element<'_, Message>) -> Element<'_, Message> {
    let card = container(inner).style(move |t: &Theme| {
        let p = pal_of(t);
        container::Style {
            background: Some(p.surface_container_high.into()),
            border: iced::Border {
                color: p.outline_variant,
                width: 1.0,
                // M3 puts every dialog at the extra-large step; the mockup
                // squares its cards down to 12, but dialogs are the one
                // component the spec keeps round.
                radius: theme::shape::XL.into(),
            },
            shadow: iced::Shadow {
                color: with_alpha(p.shadow, 0.3),
                offset: iced::Vector::new(0.0, 8.0),
                blur_radius: 24.0,
            },
            ..Default::default()
        }
    });
    let scrim = container(Space::new().width(Length::Fill).height(Length::Fill))
        .width(Length::Fill)
        .height(Length::Fill)
        // M3 modal scrim: the `scrim` role (black) at 32%.
        .style(|t: &Theme| container::Style {
            background: Some(with_alpha(pal_of(t).scrim, 0.32).into()),
            ..Default::default()
        });
    let centered = container(card)
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill);
    iced::widget::stack![scrim, centered].into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WizardStepState {
    Completed,
    Active,
    Upcoming,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BannerSeverity {
    Warning,
    Error,
}

pub(crate) fn wizard_step_state(index: usize, current: usize) -> WizardStepState {
    use WizardStepState::{Active, Completed, Upcoming};

    match index.cmp(&current) {
        std::cmp::Ordering::Less => Completed,
        std::cmp::Ordering::Equal => Active,
        std::cmp::Ordering::Greater => Upcoming,
    }
}

/// Expanded step bar row height; the trailing rule adds one more pixel.
const WIZARD_STEP_BAR_HEIGHT: f32 = 52.0;
/// Compact's condensed step indicator row height.
const COMPACT_WIZARD_STEP_BAR_HEIGHT: f32 = 48.0;

pub(crate) fn wizard_step_bar(
    steps: &[&str],
    current: usize,
    size_class: WindowSizeClass,
) -> Element<'static, Message> {
    let labels: Vec<String> = steps.iter().map(|label| (*label).to_string()).collect();
    let steps_len = labels.len();

    if size_class == WindowSizeClass::Compact {
        const TRACK_WIDTH: f32 = 110.0;
        let displayed_step = current.saturating_add(1).min(steps_len.max(1));
        let progress = if steps_len == 0 {
            0.0
        } else {
            displayed_step as f32 / steps_len as f32
        };
        let active_label = labels
            .get(current)
            .or_else(|| labels.last())
            .cloned()
            .unwrap_or_default();
        let track = container(
            container(Space::new())
                .width(Length::Fixed(TRACK_WIDTH * progress))
                .height(Length::Fixed(4.0))
                .style(|t: &Theme| container::Style {
                    background: Some(pal_of(t).primary.into()),
                    ..Default::default()
                }),
        )
        .width(Length::Fixed(TRACK_WIDTH))
        .height(Length::Fixed(4.0))
        .style(|t: &Theme| container::Style {
            background: Some(pal_of(t).surface_container_high.into()),
            border: iced::Border {
                radius: theme::shape::FULL.into(),
                ..Default::default()
            },
            ..Default::default()
        });
        let condensed = row![
            track,
            text(format!("{displayed_step} / {steps_len} ·"))
                .size(12)
                .font(theme::emphasis::medium())
                .wrapping(iced::widget::text::Wrapping::None),
            text(active_label)
                .size(12)
                .style(muted_style)
                .wrapping(iced::widget::text::Wrapping::None),
        ]
        .spacing(10)
        .padding([8, 24])
        .height(Length::Fixed(COMPACT_WIZARD_STEP_BAR_HEIGHT))
        .align_y(iced::Alignment::Center);
        return column![
            container(condensed)
                .width(Length::Fill)
                // The step bar reads as part of the content area, not as
                // shell chrome, so it takes the body background rather than
                // the surface the app bar and status bar share.
                .style(|t: &Theme| container::Style {
                    background: Some(pal_of(t).background.into()),
                    ..Default::default()
                }),
            widget::rule::horizontal(1).style(shell_rule_style),
        ]
        .height(Length::Fixed(COMPACT_WIZARD_STEP_BAR_HEIGHT + 1.0))
        .into();
    }

    {
        let mut r = row![]
            .spacing(0)
            .align_y(iced::Alignment::Center)
            .padding([8, 24])
            .height(Length::Fixed(WIZARD_STEP_BAR_HEIGHT));

        for (i, label) in labels.iter().enumerate() {
            if i > 0 {
                let completed = i <= current;
                let connector = container(Space::new().width(Length::Fixed(12.0)))
                    .width(Length::Fill)
                    .height(2)
                    .style(move |t: &Theme| {
                        let p = pal_of(t);
                        let color = if completed {
                            p.primary
                        } else {
                            p.outline_variant
                        };
                        container::Style {
                            background: Some(color.into()),
                            ..Default::default()
                        }
                    });
                r = r.push(container(connector).padding([0, 10]).width(Length::Fill));
            }

            let state = wizard_step_state(i, current);
            let marker_text = if state == WizardStepState::Completed {
                "\u{2713}".to_string()
            } else {
                (i + 1).to_string()
            };

            let marker = container(text(marker_text).size(12).center().style(move |t: &Theme| {
                let p = pal_of(t);
                let color = match state {
                    WizardStepState::Completed => p.on_primary_container,
                    WizardStepState::Active => p.on_primary,
                    WizardStepState::Upcoming => p.on_surface_variant,
                };
                iced::widget::text::Style { color: Some(color) }
            }))
            .width(26)
            .height(26)
            .align_x(iced::alignment::Horizontal::Center)
            .align_y(iced::alignment::Vertical::Center)
            .style(move |t: &Theme| {
                let p = pal_of(t);
                let (background, border_color) = match state {
                    WizardStepState::Completed => (p.primary_container, p.primary_container),
                    WizardStepState::Active => (p.primary, p.primary),
                    WizardStepState::Upcoming => (p.surface_container_high, p.outline_variant),
                };
                container::Style {
                    background: Some(background.into()),
                    border: iced::Border {
                        color: border_color,
                        width: 1.0,
                        radius: theme::shape::FULL.into(),
                    },
                    ..Default::default()
                }
            });

            let mut label_node = text(label.clone())
                .size(12)
                .wrapping(iced::widget::text::Wrapping::None)
                .style(move |t: &Theme| {
                    let p = pal_of(t);
                    let color = match state {
                        WizardStepState::Completed => p.on_primary_container,
                        WizardStepState::Active => p.on_surface,
                        WizardStepState::Upcoming => p.on_surface_variant,
                    };
                    iced::widget::text::Style { color: Some(color) }
                });
            if state == WizardStepState::Active {
                label_node = label_node.font(theme::emphasis::medium());
            }
            let step_node: Element<'static, Message> = row![marker, label_node]
                .spacing(8)
                .align_y(iced::Alignment::Center)
                .into();
            r = r.push(step_node);
        }

        column![
            container(r)
                .width(Length::Fill)
                // The step bar reads as part of the content area, not as
                // shell chrome, so it takes the body background rather than
                // the surface the app bar and status bar share.
                .style(|t: &Theme| container::Style {
                    background: Some(pal_of(t).background.into()),
                    ..Default::default()
                }),
            widget::rule::horizontal(1).style(shell_rule_style),
        ]
        .height(Length::Fixed(WIZARD_STEP_BAR_HEIGHT + 1.0))
        .into()
    }
}

/// Execution screens carry phase position in their checklist, so reserving
/// the normal setup-step bar would repeat the same hierarchy twice.
pub(crate) fn empty_wizard_step_bar() -> Element<'static, Message> {
    Space::new().height(0).into()
}

/// Lines of step guidance the fixed-height app bars show before cutting.
const SUBTITLE_MAX_LINES: usize = 2;

/// Compact desktop top app bar for a screen or wizard title/description.
pub(crate) fn large_top_app_bar<'a>(
    title: String,
    subtitle: Option<String>,
) -> Element<'a, Message> {
    let padding = iced::Padding {
        top: 5.0,
        right: 24.0,
        bottom: 5.0,
        left: 24.0,
    };
    let content_min_height = WIZARD_TOP_APP_BAR_HEIGHT - padding.top - padding.bottom;
    // The subtitle sits beside the title and may wrap to two lines. Its box is
    // fixed at two line heights, so longer guidance is cut rather than growing
    // the bar, and a single line stays vertically centred beside the title.
    let mut content = row![
        text(title)
            .size(theme::text_size::TITLE_LARGE)
            .font(theme::emphasis::medium())
            .line_height(28.0 / 22.0)
            .style(on_surface_style)
            .wrapping(iced::widget::text::Wrapping::None)
    ]
    .spacing(14)
    .width(Length::Fill)
    .align_y(iced::Alignment::Center);

    if let Some(subtitle) = subtitle.filter(|s| !s.trim().is_empty()) {
        content = content.push(
            text(subtitle)
                .size(theme::text_size::BODY_SMALL)
                .line_height(16.0 / 12.0)
                .style(muted_style)
                .width(Length::Fill)
                .height(Length::Fixed(SUBTITLE_MAX_LINES as f32 * 16.0))
                .align_y(iced::alignment::Vertical::Center)
                .wrapping(iced::widget::text::Wrapping::Word),
        );
    }

    column![
        container(
            row![
                Space::new().height(Length::Fixed(content_min_height)),
                container(content)
                    .width(Length::Fill)
                    .max_width(WIZARD_TOP_APP_BAR_MAX_WIDTH),
            ]
            // At 64px this is M3's small top app bar, not the large variant,
            // so the title is centred in the bar rather than baseline-aligned.
            .align_y(iced::Alignment::Center)
        )
        .width(Length::Fill)
        .height(Length::Fixed(WIZARD_TOP_APP_BAR_HEIGHT))
        .padding(padding)
        .align_x(iced::Alignment::Start)
        .style(|t: &Theme| panel_bg(t)),
        widget::rule::horizontal(1).style(shell_rule_style),
    ]
    .into()
}

/// Wizard app bar for the flow title and optional step guidance. Expanded
/// layouts put the title and up to two lines of guidance side by side;
/// Compact retains the stacked 64 px bar.
pub(crate) fn wizard_action_bar<'a>(
    size_class: WindowSizeClass,
    title: String,
    subtitle: Option<String>,
) -> Element<'a, Message> {
    if size_class == WindowSizeClass::Compact {
        return large_top_app_bar(title, subtitle);
    }

    let title = text(title)
        .size(theme::text_size::TITLE_LARGE)
        .font(theme::emphasis::medium())
        .line_height(1.0)
        .style(on_surface_style)
        .wrapping(iced::widget::text::Wrapping::None);
    let mut content = row![title]
        .spacing(14)
        .width(Length::Fill)
        .align_y(iced::Alignment::Center);
    if let Some(subtitle) = subtitle.filter(|s| !s.trim().is_empty()) {
        content = content.push(
            text(subtitle)
                .size(theme::text_size::BODY_SMALL)
                .line_height(16.0 / 12.0)
                .style(muted_style)
                .width(Length::Fill)
                .height(Length::Fixed(SUBTITLE_MAX_LINES as f32 * 16.0))
                .align_y(iced::alignment::Vertical::Center)
                .wrapping(iced::widget::text::Wrapping::Word),
        );
    }

    column![
        container(content)
            .width(Length::Fill)
            .height(Length::Fixed(WIZARD_TOP_APP_BAR_HEIGHT))
            .padding([0, 24])
            .align_y(iced::alignment::Vertical::Center)
            .style(|t: &Theme| panel_bg(t)),
        widget::rule::horizontal(1).style(shell_rule_style),
    ]
    .into()
}

/// Put the current step name at the top of the wizard content area. The
/// app bar names the flow; this heading names the step within that flow.
pub(crate) fn wizard_step_body<'a>(
    title: String,
    body: Element<'a, Message>,
) -> Element<'a, Message> {
    column![
        container(
            text(title)
                .size(theme::text_size::TITLE_MEDIUM)
                .font(theme::emphasis::medium())
                .style(on_surface_style)
                .width(Length::Fill)
                .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
        )
        .padding(iced::Padding {
            top: 18.0,
            right: 24.0,
            bottom: 0.0,
            left: 24.0,
        })
        .width(Length::Fill),
        body,
    ]
    .spacing(0)
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

pub(crate) fn sec_hdr<'a>(label: &str, label_alpha: f32, collapsed: bool) -> Element<'a, Message> {
    // Reserve the same vertical slot for the divider and label so the Tools
    // destinations stay in place throughout the hover transition.
    const HEADER_HEIGHT: f32 = 40.0;
    if collapsed {
        return container(widget::rule::horizontal(1).style(shell_rule_style))
            .padding([0, 12])
            .width(Length::Fixed(NAV_BTN_COLLAPSED_WIDTH))
            .height(Length::Fixed(HEADER_HEIGHT))
            .align_y(iced::Alignment::Center)
            .into();
    }
    let owned = label.to_string();
    let alpha = label_alpha;
    container(
        // Uppercase and letter-spacing carry "section label" here; the
        // mockup's mono does not, because it has no CJK and this string is
        // localized — asking for a Latin-only face would break ko/ja/zh/ru.
        text(owned.to_uppercase())
            .size(theme::text_size::LABEL_SMALL)
            // Same no-wrap rationale as nav_btn — section header text
            // ("Tools" / "도구") must not flow into two lines mid-tween.
            .wrapping(iced::widget::text::Wrapping::None)
            .style(move |t: &Theme| iced::widget::text::Style {
                color: Some(with_alpha(pal_of(t).on_surface_variant, alpha)),
            }),
    )
    .padding([0, 20])
    .height(Length::Fixed(HEADER_HEIGHT))
    .align_y(iced::Alignment::Center)
    .into()
}

pub(crate) fn info_kv<'a>(label: &str, value: &str) -> Element<'a, Message> {
    column![
        text(label.to_string())
            .size(theme::text_size::LABEL_SMALL)
            .style(muted_style),
        // Value outranks its caption on weight and color rather than size; at 16 px
        // the kv grid competed with the device name above it.
        text(value.to_string())
            .size(theme::text_size::BODY_MEDIUM)
            .font(theme::emphasis::medium()),
    ]
    .spacing(3.0)
    .into()
}

pub(crate) fn info_kv_center<'a>(label: &str, value: &str) -> Element<'a, Message> {
    column![
        text(label.to_string())
            .size(11)
            .style(muted_style)
            .width(Length::Fill)
            .center(),
        // `WordOrGlyph` so a long, space-less file path wraps at glyph
        // boundaries within the panel instead of overflowing + clipping.
        text(value.to_string())
            .size(14)
            .width(Length::Fill)
            .center()
            .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
    ]
    .spacing(3)
    .width(Length::Fill)
    .align_x(iced::Alignment::Center)
    .into()
}

/// Read-only row used by wizard confirmation screens. It shares the
/// definition-list hierarchy of the editable full-flash review.
pub(crate) fn confirm_definition_row<'a>(label: &str, value: &str) -> Element<'a, Message> {
    confirm_definition_content(
        label,
        text(value.to_string())
            .size(theme::text_size::BODY_MEDIUM)
            .font(theme::emphasis::medium())
            .width(Length::Fill)
            .wrapping(iced::widget::text::Wrapping::WordOrGlyph)
            .into(),
    )
}

/// One wrapping body sentence stating what a device-writing confirm step will
/// change. Callers pass it as the first leading element of
/// [`App::confirm_step_frame`] and only when the sentence holds for the route.
pub(crate) fn confirm_effect_line<'a>(sentence: String) -> Element<'a, Message> {
    text(sentence)
        .size(theme::text_size::BODY_MEDIUM)
        .width(Length::Fill)
        .wrapping(iced::widget::text::Wrapping::WordOrGlyph)
        .into()
}

/// Measure against the value column, not the window, and retain the full path in a tooltip.
pub(crate) fn confirm_path_row<'a>(label: &str, value: &str) -> Element<'a, Message> {
    confirm_definition_content(label, compact_review_path(value))
}

pub(crate) fn compact_review_path<'a>(value: &str) -> Element<'a, Message> {
    let path = value.to_string();
    let full_path = path.clone();
    let value = widget::responsive(move |size| {
        container(
            text(elide_path_middle(&path, size.width))
                .font(theme::mono_font())
                .size(theme::text_size::BODY_SMALL)
                .wrapping(iced::widget::text::Wrapping::None),
        )
        .width(Length::Fill)
        .height(20)
        .clip(true)
        .into()
    });
    widget::tooltip(
        container(value).width(Length::Fill).height(20),
        container(
            text(full_path)
                .size(theme::text_size::BODY_SMALL)
                .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
        )
        .padding([8, 12])
        .max_width(480)
        .style(|t| theme::tooltip_style(t, theme::shape::SM)),
        widget::tooltip::Position::Top,
    )
    .into()
}

pub(crate) fn confirm_definition_content<'a>(
    label: &str,
    value: Element<'a, Message>,
) -> Element<'a, Message> {
    column![
        row![
            container(
                text(label.to_string())
                    .size(theme::text_size::BODY_SMALL)
                    .style(muted_style)
                    .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
            )
            .width(Length::Fixed(180.0)),
            value,
        ]
        .spacing(16)
        .align_y(iced::Alignment::Start)
        .width(Length::Fill)
        .padding([9, 0]),
        widget::rule::horizontal(1).style(shell_rule_style),
    ]
    .spacing(0)
    .width(Length::Fill)
    .into()
}

/// Compact two-column information table shared by read-only detail dialogs.
pub(crate) fn info_key_value_table(fields: Vec<(String, String)>) -> Element<'static, Message> {
    let mut table = column![].spacing(0);
    for (index, (key, value)) in fields.into_iter().enumerate() {
        let key_cell = text(key).size(12).style(muted_style).width(180);
        let value_cell = text(value)
            .size(12)
            .width(Length::Fill)
            .wrapping(iced::widget::text::Wrapping::WordOrGlyph);
        let row_inner = row![key_cell, value_cell]
            .spacing(12)
            .padding([4, 10])
            .align_y(iced::Alignment::Center);
        let zebra = index % 2 == 1;
        table = table.push(container(row_inner).width(Length::Fill).style(
            move |theme: &Theme| -> container::Style {
                let palette = pal_of(theme);
                container::Style {
                    background: zebra.then_some(palette.surface_container_low.into()),
                    ..Default::default()
                }
            },
        ));
    }
    table.into()
}

pub(crate) fn adv_grid_btn<'a>(item: AdvAction, label: &str) -> Element<'a, Message> {
    // Inner container: border-only via `sel_card_style`. An opaque
    // background, as `theme::surface_card_style` paints, would sit on top
    // of the button's hover fill and swallow the highlight, making the
    // grid feel dead on hover.
    let destructive = item.is_destructive();
    let foreground = move |t: &Theme| {
        let p = pal_of(t);
        iced::widget::text::Style {
            color: Some(if destructive { p.error } else { p.on_surface }),
        }
    };
    let content = container(
        text(label.to_string())
            .size(12.0)
            .width(Length::Fill)
            .style(foreground),
    )
    .padding([18.0, 14.0])
    .width(Length::Fill)
    .align_x(iced::alignment::Horizontal::Left)
    .style(move |t: &Theme| sel_card_style_for(t, false, destructive));

    button(content)
        .on_press(Message::Adv(AdvMsg::AdvConfirm(item)))
        .padding(0)
        .width(Length::Fill)
        .style(move |t: &Theme, status| sel_card_btn_style_for(t, status, false, destructive))
        .into()
}

pub(crate) fn svg_icon(bytes: &'static [u8], size: f32) -> Element<'static, Message> {
    iced::widget::svg(iced::widget::svg::Handle::from_memory(bytes))
        .width(size)
        .height(size)
        .into()
}

static SKROOT_ICON_HANDLE: std::sync::LazyLock<iced::widget::image::Handle> =
    std::sync::LazyLock::new(|| {
        iced::widget::image::Handle::from_bytes(
            include_bytes!("../../assets/icons/skroot.png").as_slice(),
        )
    });

pub(crate) fn skroot_icon(size: f32) -> Element<'static, Message> {
    widget::image(SKROOT_ICON_HANDLE.clone())
        .width(size)
        .height(size)
        .content_fit(iced::ContentFit::ScaleDown)
        .into()
}

/// Primary-coloured Lucide icon sized to `size`, for wizard tiles, status
/// markers and confirm-step eyebrows.
pub(crate) fn lucide_primary(
    icon: iced::widget::Text<'static, Theme, iced::Renderer>,
    size: f32,
) -> Element<'static, Message> {
    icon.size(size)
        .style(|t: &Theme| iced::widget::text::Style {
            color: Some(pal_of(t).primary),
        })
        .into()
}

/// Primary-coloured Lucide icon for a single-column wizard row. The row's
/// cross-axis limit can be shorter than Iced's default 1.3x text line box at
/// large icon sizes; use a one-em line box here so the glyph remains centred
/// without changing the shared Lucide helpers used by other surfaces.
pub(crate) fn lucide_list_primary(
    icon: iced::widget::Text<'static, Theme, iced::Renderer>,
    size: f32,
) -> Element<'static, Message> {
    icon.size(size)
        .line_height(1.0)
        .style(|t: &Theme| iced::widget::text::Style {
            color: Some(pal_of(t).primary),
        })
        .into()
}

/// Error-coloured Lucide icon. Pairs with
/// [`wizard_list_option_card_destructive`] so a data-erasing
/// option reads as destructive at the glyph, not just at the border.
pub(crate) fn lucide_error(
    icon: iced::widget::Text<'static, Theme, iced::Renderer>,
    size: f32,
) -> Element<'static, Message> {
    icon.size(size)
        .style(|t: &Theme| iced::widget::text::Style {
            color: Some(pal_of(t).error),
        })
        .into()
}

/// Disabled-state Lucide icon — `on_surface` at 0.38 alpha (M3 disabled
/// content tone). Pass `None` to [`wizard_list_option_card`] so the whole row
/// reads as "not pickable on this device".
pub(crate) fn lucide_disabled(
    icon: iced::widget::Text<'static, Theme, iced::Renderer>,
    size: f32,
) -> Element<'static, Message> {
    icon.size(size)
        .style(|t: &Theme| iced::widget::text::Style {
            color: Some(with_alpha(pal_of(t).on_surface, 0.38)),
        })
        .into()
}

/// Disabled-state counterpart to [`lucide_list_primary`].
pub(crate) fn lucide_list_disabled(
    icon: iced::widget::Text<'static, Theme, iced::Renderer>,
    size: f32,
) -> Element<'static, Message> {
    icon.size(size)
        .line_height(1.0)
        .style(|t: &Theme| iced::widget::text::Style {
            color: Some(with_alpha(pal_of(t).on_surface, 0.38)),
        })
        .into()
}

/// Lucide icon coloured by an arbitrary theme-driven closure. Used
/// where colour depends on widget state (nav active / disabled,
/// op success / failure, title-bar hover).
pub(crate) fn lucide_icon(
    icon: iced::widget::Text<'static, Theme, iced::Renderer>,
    size: f32,
    color: impl Fn(&Theme) -> iced::Color + 'static,
) -> Element<'static, Message> {
    icon.size(size)
        // Without this the glyph carries iced's default 1.3 relative line
        // height, so a 20px icon is laid out in a 26px box and its ink sits
        // below the centre of whatever slot it is aligned in. Pinning the line
        // box to the glyph size makes centring exact.
        .line_height(iced::widget::text::LineHeight::Absolute(size.into()))
        .style(move |t: &Theme| iced::widget::text::Style {
            color: Some(color(t)),
        })
        .into()
}

/// Keep wizard choices in one centered column at every window width.
/// The existing desktop width cap leaves extra space as margins, not empty panes.
pub(crate) fn wizard_selection_step<'a>(
    content_width: f32,
    step_title: String,
    options: Element<'a, Message>,
) -> Element<'a, Message> {
    let heading = text(step_title)
        .size(theme::text_size::TITLE_MEDIUM)
        .font(theme::emphasis::medium())
        .style(on_surface_style)
        .width(Length::Fill)
        .wrapping(iced::widget::text::Wrapping::WordOrGlyph);

    container(
        container(column![heading, options].spacing(16).width(Length::Fill))
            .padding([20.0, WIZARD_STEP_HORIZONTAL_PADDING])
            .width(Length::Fixed(
                content_width.clamp(1.0, WIZARD_LIST_MAX_WIDTH),
            )),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .align_x(iced::alignment::Horizontal::Center)
    .align_y(iced::alignment::Vertical::Top)
    .into()
}

pub(crate) fn wizard_list_option_card(
    icon: Element<'static, Message>,
    label: &str,
    sub: &str,
    selected: bool,
    msg: Option<Message>,
    metrics: ListRowMetrics,
) -> Element<'static, Message> {
    wizard_list_option_card_with_role(
        icon,
        label,
        sub,
        selected,
        msg,
        metrics,
        WizardListOptionRole::Standard,
    )
}

/// Selection-row variant with an inline recommendation pill. The pill stays
/// inside the row's trailing edge so it participates in layout instead of
/// covering the label or card border.
pub(crate) fn wizard_list_option_card_recommended(
    icon: Element<'static, Message>,
    label: &str,
    sub: &str,
    selected: bool,
    msg: Option<Message>,
    metrics: ListRowMetrics,
    recommendation: (&str, &str),
) -> Element<'static, Message> {
    wizard_list_option_card_with_role(
        icon,
        label,
        sub,
        selected,
        msg,
        metrics,
        WizardListOptionRole::Recommended {
            label: recommendation.0.to_string(),
            tip: recommendation.1.to_string(),
        },
    )
}

/// Error-role variant for an irreversible selection such as wiping user data.
pub(crate) fn wizard_list_option_card_destructive(
    icon: Element<'static, Message>,
    label: &str,
    sub: &str,
    selected: bool,
    msg: Option<Message>,
    metrics: ListRowMetrics,
) -> Element<'static, Message> {
    wizard_list_option_card_with_role(
        icon,
        label,
        sub,
        selected,
        msg,
        metrics,
        WizardListOptionRole::Destructive,
    )
}

enum WizardListOptionRole {
    Standard,
    Destructive,
    Recommended { label: String, tip: String },
}

/// Radio indicator for a selection row — these rows are a single-choice
/// group, and without it nothing on the row says so before you click one.
/// The ring takes the interactive `outline` tone at rest and the row's own
/// accent once chosen, so a destructive choice reads red rather than primary.
pub(crate) fn selection_radio(
    selected: bool,
    enabled: bool,
    destructive: bool,
) -> Element<'static, Message> {
    const RING: f32 = 20.0;
    const DOT: f32 = 10.0;
    let dot: Element<'static, Message> = if selected {
        container(Space::new())
            .width(Length::Fixed(DOT))
            .height(Length::Fixed(DOT))
            .style(move |t: &Theme| {
                let p = pal_of(t);
                let accent = if destructive { p.error } else { p.primary };
                container::Style {
                    background: Some(
                        if enabled {
                            accent
                        } else {
                            with_alpha(accent, 0.38)
                        }
                        .into(),
                    ),
                    border: iced::Border {
                        radius: theme::shape::FULL.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }
            })
            .into()
    } else {
        Space::new().into()
    };
    container(dot)
        .width(Length::Fixed(RING))
        .height(Length::Fixed(RING))
        .align_x(iced::alignment::Horizontal::Center)
        .align_y(iced::alignment::Vertical::Center)
        .style(move |t: &Theme| {
            let p = pal_of(t);
            let accent = if destructive { p.error } else { p.primary };
            let ring = if selected { accent } else { p.outline };
            container::Style {
                border: iced::Border {
                    color: if enabled {
                        ring
                    } else {
                        with_alpha(ring, 0.38)
                    },
                    width: 2.0,
                    radius: theme::shape::FULL.into(),
                },
                ..Default::default()
            }
        })
        .into()
}

fn wizard_list_option_card_with_role(
    icon: Element<'static, Message>,
    label: &str,
    sub: &str,
    selected: bool,
    msg: Option<Message>,
    metrics: ListRowMetrics,
    role: WizardListOptionRole,
) -> Element<'static, Message> {
    let (destructive, recommendation) = match role {
        WizardListOptionRole::Standard => (false, None),
        WizardListOptionRole::Destructive => (true, None),
        WizardListOptionRole::Recommended { label, tip } => (false, Some((label, tip))),
    };
    let enabled = msg.is_some();
    let selected = selected && enabled;
    let foreground = move |t: &Theme| {
        let p = pal_of(t);
        iced::widget::text::Style {
            color: Some(if selected && destructive {
                p.on_error_container
            } else if selected {
                p.on_primary_container
            } else if enabled {
                p.on_surface
            } else {
                with_alpha(p.on_surface, 0.38)
            }),
        }
    };
    let mut label_text = text(label.to_string())
        .size(metrics.label_size)
        .line_height(20.0 / 14.0)
        .style(foreground)
        .width(Length::Fill);
    if selected && enabled {
        label_text = label_text.font(theme::emphasis::medium());
    }
    let mut copy = column![label_text]
        .spacing(metrics.text_gap)
        .width(Length::Fill);
    if !sub.is_empty() {
        copy = copy.push(
            text(sub.to_string())
                .size(metrics.desc_size)
                .line_height(16.0 / 12.0)
                .style(move |t: &Theme| {
                    if selected || !enabled {
                        foreground(t)
                    } else {
                        muted_style(t)
                    }
                })
                .width(Length::Fill)
                .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
        );
    }
    let text_block = container(copy).width(Length::Fill);
    let mut body = row![
        selection_radio(selected && enabled, enabled, destructive && enabled),
        icon_tile(icon),
        text_block
    ]
    .spacing(metrics.icon_gap)
    .align_y(iced::Alignment::Center);
    if let Some((label, tip)) = recommendation {
        let pill = container(
            row![
                lucide_icon(icon::rec_badge(), 11.0, |t: &Theme| pal_of(t)
                    .on_primary_container),
                text(label)
                    .size(11.0)
                    .font(theme::emphasis::medium())
                    .wrapping(iced::widget::text::Wrapping::None)
                    .style(|t: &Theme| iced::widget::text::Style {
                        color: Some(pal_of(t).on_primary_container),
                    }),
            ]
            .spacing(4.0)
            .align_y(iced::Alignment::Center),
        )
        .height(Length::Fixed(20.0))
        .padding([0, 8])
        .align_y(iced::alignment::Vertical::Center)
        .style(|t: &Theme| {
            let p = pal_of(t);
            container::Style {
                background: Some(p.primary_container.into()),
                border: iced::Border {
                    radius: theme::shape::FULL.into(),
                    ..Default::default()
                },
                ..Default::default()
            }
        });
        let pill_with_tip = widget::tooltip(
            pill,
            container(
                text(tip)
                    .size(12.0)
                    .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
            )
            .padding([8, 12])
            .max_width(240.0)
            .style(|t: &Theme| theme::tooltip_style(t, theme::shape::SM)),
            widget::tooltip::Position::Top,
        )
        .gap(6.0);
        body = body.push(pill_with_tip);
    }

    let min_height = if sub.is_empty() { metrics.height } else { 72.0 };
    let inner = container(
        row![
            body.width(Length::Fill),
            Space::new().height(min_height - metrics.padding.top - metrics.padding.bottom),
        ]
        .align_y(iced::Alignment::Center),
    )
    .padding(metrics.padding)
    .width(Length::Fill)
    .align_y(iced::Alignment::Center);
    let btn = button(inner).padding(0).width(Length::Fill);
    match msg {
        Some(m) => btn
            .on_press(m)
            .style(move |t: &Theme, status| {
                expressive_choice_style(t, status, selected, destructive)
            })
            .into(),
        None => btn
            .style(|t: &Theme, _status| {
                let p = pal_of(t);
                button::Style {
                    background: Some(with_alpha(p.surface_container_low, 0.5).into()),
                    text_color: with_alpha(p.on_surface, 0.38),
                    border: iced::Border {
                        radius: 4.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }
            })
            .into(),
    }
}

/// Wrap a wizard icon. Icons already carry their own rounded-rect bg,
/// so no outer border.
pub(crate) fn icon_tile(icon: Element<'static, Message>) -> Element<'static, Message> {
    container(icon).padding(0).into()
}

impl RebootTarget {
    pub(crate) fn icon(self, size: f32) -> Element<'static, Message> {
        let glyph = match self {
            Self::System => icon::reboot_system(),
            Self::Recovery => icon::reboot_recovery(),
            Self::Bootloader => icon::reboot_bootloader(),
            Self::Fastbootd => icon::reboot_fastbootd(),
            Self::Edl => icon::reboot_edl(),
        };
        lucide_list_primary(glyph, size)
    }
}

impl Family {
    pub(crate) fn icon_sized(self, size: f32) -> Element<'static, Message> {
        // Kept as bundled SVG assets — these are per-brand logos, not
        // monochrome glyphs, so Lucide's icon set doesn't cover them.
        let bytes: &'static [u8] = match self {
            Self::Magisk => include_bytes!("../../assets/icons/magisk.svg"),
            Self::KernelSU => include_bytes!("../../assets/icons/kernelsu.svg"),
            Self::APatch => include_bytes!("../../assets/icons/apatch.svg"),
            Self::Skroot => return skroot_icon(size),
        };
        svg_icon(bytes, size)
    }
}

impl Provider {
    /// Provider brand logo at the explicit size used by a compact list row.
    pub(crate) fn icon_sized(self, size: f32) -> Element<'static, Message> {
        // Provider brand logos — kept as bespoke SVG, not Lucide.
        let bytes: &'static [u8] = match self {
            Self::Magisk => include_bytes!("../../assets/icons/magisk.svg"),
            Self::MagiskForks => include_bytes!("../../assets/icons/local_magisk.svg"),
            Self::KernelSULocal => include_bytes!("../../assets/icons/local_ksu.svg"),
            Self::KernelSU => include_bytes!("../../assets/icons/kernelsu.svg"),
            Self::KernelSUNext => include_bytes!("../../assets/icons/kernelsu_next.svg"),
            Self::SukiSU => include_bytes!("../../assets/icons/sukisu.svg"),
            Self::BakaSU => include_bytes!("../../assets/icons/bakasu.svg"),
            Self::APatch => include_bytes!("../../assets/icons/apatch.svg"),
            Self::FolkPatch => include_bytes!("../../assets/icons/folkpatch.svg"),
        };
        svg_icon(bytes, size)
    }
}

impl RootMode {
    pub(crate) fn icon(self, size: f32) -> Element<'static, Message> {
        // Lucide chip/layers glyphs.
        let glyph = match self {
            Self::Lkm => icon::root_lkm(),
            Self::Gki => icon::root_gki(),
        };
        lucide_primary(glyph, size)
    }
}

impl VerChoice {
    pub(crate) fn icon(self, size: f32) -> Element<'static, Message> {
        let glyph = match self {
            Self::Stable => icon::ver_stable(),
            Self::Nightly => icon::ver_nightly(),
        };
        lucide_primary(glyph, size)
    }
}

impl NightlySource {
    pub(crate) fn icon(self, size: f32) -> Element<'static, Message> {
        let glyph = match self {
            Self::AutoDetect => icon::nightly_auto(),
            Self::ManualInput => icon::nightly_manual(),
        };
        lucide_primary(glyph, size)
    }
}

impl App {
    /// Device row shared by every device-writing confirm step:
    /// `{market_name} · {model} · {connection} · {slot}`. An empty name or
    /// model reads as a dash; the slot part is left out while it is unknown.
    pub(crate) fn confirm_device_row<'a>(&self) -> Element<'a, Message> {
        let or_dash = |value: &str| {
            if value.is_empty() {
                "—".to_string()
            } else {
                value.to_string()
            }
        };
        let mut parts = vec![
            or_dash(&self.device.market_name),
            or_dash(&self.device.model),
            or_dash(self.t(self.connection_label_key())),
        ];
        if !self.device.slot.is_empty() {
            parts.push(self.device.slot.clone());
        }
        confirm_definition_row(self.t("dash_device"), &parts.join(" · "))
    }

    /// Shared confirm-screen frame below the in-content step heading. Leading
    /// rows are full-width callouts or lists; review values use the same
    /// one-column definition list as the editable full-flash confirmation.
    pub(crate) fn confirm_step_frame<'a>(
        &self,
        leading: Vec<Element<'a, Message>>,
        grid: Vec<Element<'a, Message>>,
        trailing: Vec<Element<'a, Message>>,
    ) -> Element<'a, Message> {
        let mut content = column![]
            .spacing(8)
            .padding([18, 28])
            .width(Length::Fill)
            .align_x(iced::Alignment::Center);

        if !leading.is_empty() {
            content = content.push(column(leading).spacing(8).width(Length::Fill));
        }

        // Definition rows own their bottom dividers. Paths are definition rows
        // too: a second section divider would duplicate the preceding row's
        // rule. Keep every summary row in one uninterrupted list regardless of
        // which optional groups the caller supplies.
        let rows: Vec<_> = grid.into_iter().chain(trailing).collect();
        if !rows.is_empty() {
            content = content.push(column(rows).spacing(0).width(Length::Fill));
        }

        // The scroller itself shrinks when content is short. Its child
        // deliberately has no fill height: a scrollable measures content
        // with an unbounded vertical limit, where Fill cannot resolve to the
        // viewport.
        let summary = iced::widget::scrollable(content)
            .style(m3_scrollable_style)
            .height(Length::Shrink)
            .width(Length::Fill);

        container(
            container(summary)
                .width(Length::Fill)
                .max_width(WIZARD_CONFIRM_MAX_WIDTH),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .align_y(iced::alignment::Vertical::Top)
        .into()
    }
}
impl App {
    /// Width available to picker text after the navigation rail and step padding.
    pub(crate) fn picker_text_width(&self, actions: usize) -> f32 {
        let sidebar = if self.window_size_class() == WindowSizeClass::Expanded {
            SIDEBAR_EXPANDED_WIDTH
        } else {
            SIDEBAR_RAIL_WIDTH
        };
        let content_width = self.window_size.0 - sidebar;
        let main_width =
            content_width.min(WIZARD_LIST_MAX_WIDTH) - 2.0 * WIZARD_STEP_HORIZONTAL_PADDING;
        (main_width - 80.0 - actions as f32 * 90.0).max(60.0)
    }

    pub(crate) fn wizard_picker_row(
        &self,
        path: Option<&str>,
        kind: PickerPathKind,
        select: Option<Message>,
        clear: Option<Message>,
    ) -> Element<'static, Message> {
        let mut controls = row![
            picker_path_field(
                path,
                self.t(kind.placeholder_key()).to_string(),
                false,
                self.picker_text_width(if clear.is_some() { 2 } else { 1 })
            ),
            picker_action_button(self.t("btn_pick").to_string(), select, true),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center)
        .width(Length::Fill);
        if let Some(clear) = clear {
            controls = controls.push(picker_action_button(
                self.t("btn_clear").to_string(),
                Some(clear),
                false,
            ));
        }
        controls.into()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PickerPathKind {
    File,
    Folder,
}

impl PickerPathKind {
    fn placeholder_key(self) -> &'static str {
        match self {
            Self::File => "picker_no_file_selected",
            Self::Folder => "picker_no_folder_selected",
        }
    }
}
impl App {
    pub(crate) fn picker_recent_list<F>(
        &self,
        items: &[String],
        on_pick: F,
        label_key: &str,
        is_file_picker: bool,
    ) -> Element<'_, Message>
    where
        F: Fn(String) -> Message,
    {
        if items.is_empty() {
            return column![].into();
        }
        let mut rows = column![].spacing(0).width(Length::Fill);
        for (index, path) in self.recent_paths.visible(items).into_iter().enumerate() {
            if index > 0 {
                rows = rows.push(iced::widget::rule::horizontal(1).style(|t: &Theme| {
                    iced::widget::rule::Style {
                        color: pal_of(t).outline_variant,
                        radius: 0.0.into(),
                        fill_mode: iced::widget::rule::FillMode::Full,
                        snap: true,
                    }
                }));
            }
            let exists = if is_file_picker {
                std::path::Path::new(path).is_file()
            } else {
                std::path::Path::new(path).is_dir()
            };
            let foreground = |t: &Theme| pal_of(t).on_surface_variant;
            let message = exists.then(|| on_pick(path.clone()));
            let recent_path = path.clone();
            let path_text = iced::widget::responsive(move |size| {
                container(
                    text(elide_path_middle(&recent_path, (size.width - 8.0).max(0.0)))
                        .font(theme::mono_font())
                        .size(theme::text_size::BODY_SMALL)
                        .style(move |t: &Theme| iced::widget::text::Style {
                            color: Some(foreground(t)),
                        })
                        .width(Length::Fill)
                        .wrapping(iced::widget::text::Wrapping::None),
                )
                .width(Length::Fill)
                .height(Length::Fill)
                .padding(iced::Padding {
                    right: 8.0,
                    ..Default::default()
                })
                .align_y(iced::Alignment::Center)
                .clip(true)
                .into()
            });
            let mut content = row![
                lucide_icon(icon::fab_open_folder(), 17.0, foreground),
                path_text,
            ]
            .spacing(11)
            .align_y(iced::Alignment::Center)
            .width(Length::Fill);
            if !exists {
                content = content.push(
                    text(
                        self.t(if is_file_picker {
                            "recent_missing_file"
                        } else {
                            "recent_missing_folder"
                        })
                        .to_string(),
                    )
                    .size(theme::text_size::LABEL_SMALL)
                    .wrapping(iced::widget::text::Wrapping::None)
                    .style(|t: &Theme| iced::widget::text::Style {
                        color: Some(pal_of(t).error),
                    }),
                );
            }
            rows = rows.push(
                button(
                    container(content)
                        .height(Length::Fill)
                        .align_y(iced::Alignment::Center)
                        .width(Length::Fill),
                )
                .on_press_maybe(message)
                .height(44)
                .width(Length::Fill)
                .padding([0, 14])
                .style(move |t: &Theme, status| button::Style {
                    background: (exists && theme::state_alpha(status) > 0.0).then_some(
                        theme::with_alpha(pal_of(t).on_surface, theme::state_alpha(status)).into(),
                    ),
                    text_color: pal_of(t).on_surface,
                    ..Default::default()
                }),
            );
        }
        column![
            row![
                lucide_icon(icon::history(), 12.0, |t: &Theme| pal_of(t)
                    .on_surface_variant),
                text(self.t(label_key).to_string())
                    .size(theme::text_size::LABEL_SMALL)
                    .font(theme::emphasis::medium())
                    .style(muted_style),
            ]
            .spacing(6)
            .align_y(iced::Alignment::Center),
            container(rows)
                .width(Length::Fill)
                .clip(true)
                .style(|t: &Theme| container::Style {
                    border: iced::Border {
                        color: pal_of(t).outline_variant,
                        width: 1.0,
                        radius: theme::shape::MD.into()
                    },
                    ..Default::default()
                }),
        ]
        .spacing(8)
        .padding(iced::Padding {
            top: 16.0,
            ..Default::default()
        })
        .width(Length::Fill)
        .into()
    }
}
const PICKER_PATH_HEIGHT: f32 = 36.0;

/// Fewest units the last component keeps beside a root before the root is
/// dropped in favour of a longer piece of the name itself.
const ELIDE_MIN_COMPONENT_UNITS: usize = 12;

/// Shorten a path to `width` by whole components. The root and the last two
/// components survive when they fit, then the root and the last component;
/// parents collapse into a single ellipsis. Only a last component that cannot
/// fit alone is cut, in its middle, so its head and tail both remain.
pub(crate) fn elide_path_middle(path: &str, width: f32) -> String {
    // Monospace Latin glyphs occupy roughly 0.6em; CJK glyphs occupy one em.
    let budget = (width / (theme::text_size::BODY_SMALL * 0.62))
        .floor()
        .max(1.0) as usize;
    let chars: Vec<char> = path.chars().collect();
    let units = |c: char| if c as u32 >= 0x1100 { 2usize } else { 1usize };
    let measure = |part: &[char]| part.iter().map(|c| units(*c)).sum::<usize>();
    if measure(&chars) <= budget {
        return path.to_string();
    }
    let is_sep = |c: char| c == '/' || c == '\\';
    let prefix_len = if chars.get(1) == Some(&':') {
        3.min(chars.len())
    } else if chars.first().is_some_and(|c| is_sep(*c)) {
        if chars.get(1) == chars.first() {
            chars
                .iter()
                .enumerate()
                .skip(2)
                .filter(|(_, c)| is_sep(**c))
                .nth(1)
                .map(|(i, _)| i + 1)
                .unwrap_or(2)
        } else {
            1
        }
    } else {
        0
    };
    let prefix = &chars[..prefix_len];
    let rest = &chars[prefix_len..];
    // A trailing separator belongs to the last component.
    let body_end = rest
        .iter()
        .rposition(|c| !is_sep(*c))
        .map_or(0, |index| index + 1);
    let last_start = rest[..body_end]
        .iter()
        .rposition(|c| is_sep(*c))
        .map_or(0, |index| index + 1);
    let prev_start = rest[..last_start.saturating_sub(1)]
        .iter()
        .rposition(|c| is_sep(*c))
        .map_or(0, |index| index + 1);
    let last = &rest[last_start..];

    // `parents_sep` is the separator that follows the ellipsis standing in for
    // the dropped parents; the ellipsis is budgeted as one unit.
    let join = |prefix: &[char], parents_sep: Option<char>, tail: &[char]| -> Option<String> {
        let used = measure(prefix) + parents_sep.map_or(0, |_| 2) + measure(tail);
        if used > budget {
            return None;
        }
        let mut shown: String = prefix.iter().collect();
        if let Some(sep) = parents_sep {
            shown.push('…');
            shown.push(sep);
        }
        shown.extend(tail.iter());
        Some(shown)
    };
    if last_start > 0 {
        if prev_start > 0
            && let Some(shown) = join(prefix, Some(rest[prev_start - 1]), &rest[prev_start..])
        {
            return shown;
        }
        let sep = rest[last_start - 1];
        if let Some(shown) = join(prefix, Some(sep), last) {
            return shown;
        }
        // A root wider than the budget (long UNC names) is the first thing to go.
        if !prefix.is_empty()
            && let Some(shown) = join(&[], Some(sep), last)
        {
            return shown;
        }
    }

    // The last component alone is too wide: keep its head and its tail.
    let prefix = if measure(prefix) + 1 + ELIDE_MIN_COMPONENT_UNITS <= budget {
        prefix
    } else {
        &[]
    };
    let name = if last.is_empty() { rest } else { last };
    let avail = budget.saturating_sub(measure(prefix) + 1);
    let head_budget = avail.div_ceil(2);
    let mut head_end = 0;
    let mut spent = 0;
    while head_end < name.len() && spent + units(name[head_end]) <= head_budget {
        spent += units(name[head_end]);
        head_end += 1;
    }
    let mut tail_budget = avail - spent;
    let mut tail_start = name.len();
    while tail_start > head_end && units(name[tail_start - 1]) <= tail_budget {
        tail_budget -= units(name[tail_start - 1]);
        tail_start -= 1;
    }
    let mut shown: String = prefix.iter().collect();
    shown.extend(name[..head_end].iter());
    shown.push('…');
    shown.extend(name[tail_start..].iter());
    shown
}

pub(crate) fn picker_path_field(
    value: Option<&str>,
    placeholder: String,
    filled_background: bool,
    width: f32,
) -> Element<'static, Message> {
    let selected = value.is_some();
    let shown = value
        .map(|path| elide_path_middle(path, width))
        .unwrap_or(placeholder);
    let path = text(shown)
        .size(if selected {
            theme::text_size::BODY_SMALL
        } else {
            theme::text_size::BODY_MEDIUM
        })
        .font_maybe(selected.then_some(theme::mono_font()))
        .width(Length::Fill)
        .align_x(iced::alignment::Horizontal::Left)
        .wrapping(iced::widget::text::Wrapping::None)
        .style(move |t: &Theme| iced::widget::text::Style {
            color: Some(if selected {
                pal_of(t).on_surface
            } else {
                pal_of(t).on_surface_variant
            }),
        });
    container(path)
        .width(Length::Fill)
        .height(Length::Fixed(PICKER_PATH_HEIGHT))
        .padding([0, 12])
        .align_y(iced::alignment::Vertical::Center)
        .clip(true)
        .style(move |t: &Theme| container::Style {
            background: filled_background.then_some(pal_of(t).background.into()),
            border: iced::Border {
                color: pal_of(t).outline,
                width: 1.0,
                radius: theme::shape::SM.into(),
            },
            ..Default::default()
        })
        .into()
}

pub(crate) fn picker_action_button(
    label: String,
    message: Option<Message>,
    outlined: bool,
) -> Element<'static, Message> {
    button(
        container(
            text(label)
                .size(theme::text_size::BODY_SMALL)
                .wrapping(iced::widget::text::Wrapping::None),
        )
        .height(Length::Fill)
        .align_y(iced::alignment::Vertical::Center),
    )
    .on_press_maybe(message)
    .height(Length::Fixed(PICKER_PATH_HEIGHT))
    .padding([0, 12])
    .style(move |t: &Theme, status| {
        let p = pal_of(t);
        let alpha = theme::state_alpha(status);
        button::Style {
            background: (alpha > 0.0).then_some(with_alpha(p.on_surface, alpha).into()),
            text_color: if status == button::Status::Disabled {
                with_alpha(p.on_surface, 0.38)
            } else {
                p.on_surface
            },
            border: iced::Border {
                color: if outlined {
                    p.outline
                } else {
                    iced::Color::TRANSPARENT
                },
                width: if outlined { 1.0 } else { 0.0 },
                radius: theme::button_radius(status).into(),
            },
            ..Default::default()
        }
    })
    .into()
}
impl App {
    /// Hint for the loader picker: the manifest (.xml / .x) hint when the model
    /// needs a Sahara manifest, the standard .melf hint for a known model, else the
    /// generic .melf / .xml / .x hint (no device or unknown model).
    pub(crate) fn loader_picker_subtitle(&self) -> String {
        let key = if self.device.connection == ConnectionStatus::None {
            "loader_picker_subtitle_unknown"
        } else if self.requires_sahara_manifest() {
            "loader_picker_subtitle_manifest"
        } else if self.device.model.is_empty() {
            "loader_picker_subtitle_unknown"
        } else {
            "loader_picker_subtitle_standard"
        };
        self.t(key).to_string()
    }

    /// Center single-pane pickers within the shared wizard width limit.
    pub(crate) fn wizard_picker_step<'a>(
        &self,
        title: String,
        body: Element<'a, Message>,
    ) -> Element<'a, Message> {
        let size = self.window_size_class();
        let width = self.window_size.0
            - if size == WindowSizeClass::Expanded {
                SIDEBAR_EXPANDED_WIDTH
            } else {
                SIDEBAR_RAIL_WIDTH
            };
        container(
            container(wizard_step_body(title, body))
                .width(Length::Fixed(width.clamp(1.0, WIZARD_LIST_MAX_WIDTH)))
                .height(Length::Fill),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced::alignment::Horizontal::Center)
        .into()
    }
}

/// Shared pointer and keyboard entry point for persistent explanatory copy.
pub(crate) fn help_button(title: String, body: String) -> Element<'static, Message> {
    button(
        container(lucide_icon(icon::help_context(), 24.0, |t: &Theme| {
            pal_of(t).on_surface_variant
        }))
        .center_x(48)
        .center_y(48),
    )
    .padding(0)
    .on_press(Message::HelpShow(title, body))
    .style(m3_standard_icon_button_style)
    .into()
}

#[cfg(test)]
mod picker_path_tests {
    use super::elide_path_middle;

    const DEMO_FIRMWARE: &str =
        "/Users/ltbox/Firmware/TB520FU_ROW_OPEN_USER_Q00002.0_W_ZUI_17.5.10.096_ST_251127";
    const DEMO_LAST: &str = "TB520FU_ROW_OPEN_USER_Q00002.0_W_ZUI_17.5.10.096_ST_251127";

    fn budget(width: f32) -> usize {
        (width / (crate::theme::text_size::BODY_SMALL * 0.62))
            .floor()
            .max(1.0) as usize
    }

    fn units(shown: &str) -> usize {
        shown
            .chars()
            .map(|c| {
                if c == '…' || (c as u32) < 0x1100 {
                    1
                } else {
                    2
                }
            })
            .sum()
    }

    #[test]
    fn whole_components_survive_and_parents_collapse_into_one_ellipsis() {
        let path = r"D:\Firmware\SomeLongDeviceName\SomeLongBuildName\image";
        assert_eq!(elide_path_middle(path, 2000.0), path);
        // Root plus the last two components when they fit.
        assert_eq!(
            elide_path_middle(path, 260.0),
            r"D:\…\SomeLongBuildName\image"
        );
        // Then root plus the last component.
        assert_eq!(elide_path_middle(path, 180.0), r"D:\…\image");
        assert_eq!(
            elide_path_middle("/Users/ltbox/Firmware/build/image", 200.0),
            "/…/build/image"
        );
    }

    #[test]
    fn firmware_folder_keeps_its_model_token_at_every_width() {
        let last_start = DEMO_FIRMWARE.rfind('/').unwrap() + 1;
        assert_eq!(&DEMO_FIRMWARE[last_start..], DEMO_LAST);
        for width in [700.0_f32, 520.0, 400.0, 300.0, 200.0] {
            let shown = elide_path_middle(DEMO_FIRMWARE, width);
            assert!(units(&shown) <= budget(width), "{width}: {shown}");
            let tail = shown.rsplit('/').next().unwrap();
            assert!(tail.starts_with("TB520FU"), "{width}: {shown}");
        }
        // Wide enough for the folder but not for its parents: whole folder kept.
        assert_eq!(
            elide_path_middle(DEMO_FIRMWARE, 520.0),
            format!("/…/{DEMO_LAST}")
        );
        // Too narrow for the folder: head and tail of the name both remain.
        let shown = elide_path_middle(DEMO_FIRMWARE, 300.0);
        assert!(shown.contains('…') && shown.ends_with("251127"), "{shown}");
    }

    #[test]
    fn unix_unc_and_unicode_roots_survive_elision() {
        assert_eq!(
            elide_path_middle("/home/user/firmware/verylongbuild/image", 100.0),
            "/…/image"
        );
        assert!(
            elide_path_middle(r"\\server\share\firmware\longbuild\image", 230.0)
                .starts_with(r"\\server\share\")
        );
        let result = elide_path_middle("D:/펌웨어/아주긴폴더이름/이미지.img", 120.0);
        assert!(result.starts_with("D:/"));
        assert!(result.ends_with(".img"));
    }

    #[test]
    fn oversized_unc_roots_and_tiny_widths_stay_within_the_path_budget() {
        let path = format!(
            r"\\{}\{}\image.img",
            "server".repeat(200),
            "공유".repeat(200)
        );
        let paths = [
            path.as_str(),
            DEMO_FIRMWARE,
            r"D:\a\b\c\d",
            "relative/dir/name",
            "/trailing/dir/slash/",
            "/",
        ];
        for path in paths {
            for width in [0.0_f32, 1.0, 20.0, 100.0, 240.0] {
                let shown = elide_path_middle(path, width);
                assert!(units(&shown) <= budget(width), "{width}: {shown}");
            }
        }
    }
}
