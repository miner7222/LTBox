//! Reusable view components (dialogs, cards, step bar, icon tiles, lucide helpers). Extracted from `main.rs`.

use crate::focus_button::{self as button, button};
use crate::*;
use iced::widget::{self, Space, column, container, row, text};
use iced::{Element, Length, Theme};
use theme::with_alpha;

/// Centered M3 dialog card on a scrim. Inner owns padding/width. MODAL: the
/// whole layer is wrapped in `opaque`, so it captures every pointer event and
/// nothing behind it reacts. Use for confirm dialogs (reboot, country,
/// region-target, rescue, root prompts) that must block the panel behind them.
pub(crate) fn m3_dialog(inner: Element<'_, Message>) -> Element<'_, Message> {
    // `opaque` makes the whole dialog layer capture every pointer event, so
    // hover/click can't fall through the parent `Stack` to the panel behind it
    // (the scrim alone only paints — it doesn't block). The card's own buttons
    // still receive their clicks.
    iced::widget::opaque(m3_dialog_layers(inner))
}

/// Like [`m3_dialog`] but MODELESS: no `opaque` wrapper, so pointer events fall
/// through the scrim to the panel behind. Use for the busy progress dialog — a
/// long-running flash must not trap the user; the sidebar (and current view)
/// stay clickable so they can navigate back to the op's progress screen.
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
                top: 18.0,
                right: theme::DIALOG_H_PADDING,
                bottom: 10.0,
                left: theme::DIALOG_H_PADDING,
            })
            .width(Length::Fill)
    ];
    if scrolling {
        content = content.push(widget::rule::horizontal(1).style(shell_rule_style));
    }
    content = content.push(
        container(body)
            .padding([16.0, theme::DIALOG_H_PADDING])
            .width(Length::Fill),
    );
    if scrolling {
        content = content.push(widget::rule::horizontal(1).style(shell_rule_style));
    }
    content
        .push(
            container(footer)
                .padding([14.0, theme::DIALOG_H_PADDING])
                .width(Length::Fill),
        )
        .spacing(0)
        .width(Length::Fixed(width))
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
    // Titled like the other dashboard cards rather than like an M2 filled
    // text field. The old form put a `primary` caption at the top and a
    // 2 px `primary` active indicator at the very bottom — on a
    // full-height read-only log that indicator ended up as a stray blue
    // rule hundreds of pixels away from its label, marking "focus" on a
    // surface that is never focused.
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
        // M3 modal scrim: the `scrim` role (black) at 32%, not a hardcoded 45%.
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
/// Compact's settled condensed indicator keeps its original footprint.
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
    // Beside the title, not stacked under it. The mockup wraps to a second
    // line in compact, but that guard was for arbitrary step descriptions —
    // only two subtitles survive in the whole app and both are short, so
    // they ride the baseline and ellipsize rather than growing the bar.
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
                .wrapping(iced::widget::text::Wrapping::None),
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
            // The 132px large app bar sat its title on the baseline, M3's
            // large-top-app-bar behaviour. At 64px this is a small top app
            // bar, whose title is centred in the bar.
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
/// layouts keep both on one line; Compact retains the stacked 64 px bar.
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
                .line_height(1.0)
                .style(muted_style)
                .width(Length::Fill)
                .wrapping(iced::widget::text::Wrapping::None),
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
        // Value outranks its caption on weight and color rather than
        // size — at 16 px the kv grid competed with the device
        // name above it, which is what pushed that name oversized in the
        // first place.
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

/// Measure against the value column, not the window, and retain the full path in a tooltip.
pub(crate) fn confirm_path_row<'a>(label: &str, value: &str) -> Element<'a, Message> {
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
    confirm_definition_content(
        label,
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
        .into(),
    )
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
    // Inner container: border-only via `sel_card_style`. Earlier
    // version used `theme::surface_card_style` which paints an opaque
    // bg — that bg sat on top of the button's hover fill, swallowing
    // the highlight and making the grid feel dead on hover.
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

/// Primary-coloured Lucide icon sized to `size`. Matches the colour
/// role the old per-asset SVG glyphs used for wizard tiles, status
/// markers, and confirm-step eyebrows.
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

/// Lay out wizard choices as one compact column or as an Expanded two-pane
/// surface. Help copy is always existing localized copy supplied by the step.
pub(crate) fn wizard_selection_step<'a>(
    size_class: WindowSizeClass,
    content_width: f32,
    step_title: String,
    options: Element<'a, Message>,
    help: Option<(String, Vec<String>)>,
) -> Element<'a, Message> {
    let heading = text(step_title)
        .size(theme::text_size::TITLE_MEDIUM)
        .font(theme::emphasis::medium())
        .style(on_surface_style)
        .width(Length::Fill)
        .wrapping(iced::widget::text::Wrapping::WordOrGlyph);

    if size_class == WindowSizeClass::Expanded
        && let Some((help_title, help_paragraphs)) = help
    {
        let mut copy = column![
            text(help_title)
                .size(theme::text_size::BODY_MEDIUM)
                .font(theme::emphasis::medium())
                .style(on_surface_style)
                .width(Length::Fill)
                .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
        ]
        .spacing(10)
        .width(Length::Fill);
        for paragraph in help_paragraphs {
            if !paragraph.trim().is_empty() {
                copy = copy.push(
                    text(paragraph)
                        .size(theme::text_size::BODY_SMALL)
                        .style(muted_style)
                        .width(Length::Fill)
                        .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
                );
            }
        }

        // Fill, not centred-inside-Fill. Centring the 720 column in its own
        // half left the options stranded mid-pane while the help panel clung
        // to the far edge; the whole assembly is centred below instead.
        let main = column![heading, options]
            .spacing(16)
            .width(Length::Fill)
            .height(Length::Fill);
        let help_width =
            (content_width * 0.25).clamp(WIZARD_HELP_PANEL_MIN_WIDTH, WIZARD_HELP_PANEL_WIDTH);
        let help_panel = container(copy)
            .width(Length::Fixed(help_width))
            .height(Length::Fill)
            .padding(iced::Padding {
                top: 4.0,
                right: 0.0,
                bottom: 0.0,
                left: 0.0,
            });
        // M3 caps content and centres it rather than stretching: past the cap
        // a wider window buys margin, not layout. Capping the pair together
        // keeps the help panel next to the options it explains instead of
        // pinning it to the window edge.
        let assembly_width = WIZARD_LIST_MAX_WIDTH + WIZARD_HELP_PANEL_GAP + 1.0 + help_width;
        // Fixed, not `max_width`: the cap did not survive this nesting, and
        // the block silently stretched to the window. `content_width` is
        // already known here, so the width is computed rather than negotiated.
        let available = (content_width - 2.0 * WIZARD_STEP_HORIZONTAL_PADDING).max(1.0);
        let block_width = assembly_width.min(available);
        let block = container(
            row![
                main,
                widget::rule::vertical(1).style(shell_rule_style),
                help_panel,
            ]
            .spacing(WIZARD_HELP_PANEL_GAP)
            .width(Length::Fill)
            .height(Length::Fill),
        )
        .width(Length::Fixed(block_width))
        .height(Length::Fill);
        return container(block)
            .padding([20.0, WIZARD_STEP_HORIZONTAL_PADDING])
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(iced::alignment::Horizontal::Center)
            .into();
    }

    container(
        container(column![heading, options].spacing(16).width(Length::Fill))
            .padding([20.0, WIZARD_STEP_HORIZONTAL_PADDING])
            .width(Length::Fill)
            .max_width(WIZARD_LIST_MAX_WIDTH),
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
                p.on_surface_variant
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
                    if selected {
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
    let inner = container(iced::widget::stack![
        Space::new()
            .width(Length::Fill)
            .height(min_height - metrics.padding.top - metrics.padding.bottom),
        container(body).center_y(Length::Fill).width(Length::Fill),
    ])
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
            Self::MagiskForks => include_bytes!("../../assets/icons/magisk_forks.svg"),
            Self::KernelSU => include_bytes!("../../assets/icons/kernelsu.svg"),
            Self::KernelSUNext => include_bytes!("../../assets/icons/kernelsu_next.svg"),
            Self::SukiSU => include_bytes!("../../assets/icons/sukisu.svg"),
            Self::ReSukiSU => include_bytes!("../../assets/icons/sukisu.svg"),
            Self::APatch => include_bytes!("../../assets/icons/apatch.svg"),
            Self::FolkPatch => include_bytes!("../../assets/icons/folkpatch.svg"),
        };
        svg_icon(bytes, size)
    }
}

impl RootMode {
    pub(crate) fn icon(self, size: f32) -> Element<'static, Message> {
        // Lucide chip/layers glyphs in place of the old bespoke SVGs.
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
    /// Shared confirm-screen frame below the in-content step heading. Leading
    /// rows are full-width callouts or lists; review values use the same
    /// one-column definition list as the editable full-flash confirmation.
    pub(crate) fn confirm_step_frame<'a>(
        &self,
        leading: Vec<Element<'a, Message>>,
        grid: Vec<Element<'a, Message>>,
        trailing: Vec<Element<'a, Message>>,
    ) -> Element<'a, Message> {
        let mut groups: Vec<Element<'a, Message>> = Vec::new();

        if !leading.is_empty() {
            groups.push(
                column(leading)
                    .spacing(8)
                    .width(Length::Fill)
                    .align_x(iced::Alignment::Center)
                    .into(),
            );
        }

        if !grid.is_empty() {
            groups.push(column(grid).spacing(0).width(Length::Fill).into());
        }

        if !trailing.is_empty() {
            groups.push(
                column(trailing)
                    .spacing(0)
                    .width(Length::Fill)
                    .align_x(iced::Alignment::Center)
                    .into(),
            );
        }

        let mut content = column![]
            .spacing(8)
            .padding([18, 28])
            .width(Length::Fill)
            .align_x(iced::Alignment::Center);
        for (index, group) in groups.into_iter().enumerate() {
            if index > 0 {
                content = content.push(widget::rule::horizontal(1));
            }
            content = content.push(group);
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
        let main_width = if self.window_size_class() == WindowSizeClass::Expanded {
            let help_width =
                (content_width * 0.25).clamp(WIZARD_HELP_PANEL_MIN_WIDTH, WIZARD_HELP_PANEL_WIDTH);
            let block_width = (WIZARD_LIST_MAX_WIDTH + WIZARD_HELP_PANEL_GAP + 1.0 + help_width)
                .min(content_width - 2.0 * WIZARD_STEP_HORIZONTAL_PADDING);
            block_width - help_width - 1.0 - 2.0 * WIZARD_HELP_PANEL_GAP
        } else {
            content_width.min(WIZARD_LIST_MAX_WIDTH) - 2.0 * WIZARD_STEP_HORIZONTAL_PADDING
        };
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
        for (index, path) in items.iter().take(settings_store::RECENT_MAX).enumerate() {
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

/// Preserve the drive/root and filename while shortening the intervening path.
pub(crate) fn elide_path_middle(path: &str, width: f32) -> String {
    // Monospace Latin glyphs occupy roughly 0.6em; CJK glyphs occupy one em.
    let budget = (width / (theme::text_size::BODY_SMALL * 0.62))
        .floor()
        .max(1.0) as usize;
    let chars: Vec<char> = path.chars().collect();
    let units = |c: char| if c as u32 >= 0x1100 { 2usize } else { 1usize };
    if chars.iter().map(|c| units(*c)).sum::<usize>() <= budget {
        return path.to_string();
    }
    let prefix_len = if chars.get(1) == Some(&':') {
        3.min(chars.len())
    } else if chars.first().is_some_and(|c| *c == '/' || *c == '\\') {
        if chars.get(1) == chars.first() {
            chars
                .iter()
                .enumerate()
                .skip(2)
                .filter(|(_, c)| **c == '/' || **c == '\\')
                .nth(1)
                .map(|(i, _)| i + 1)
                .unwrap_or(2)
        } else {
            1
        }
    } else {
        0
    };
    // UNC server/share names can themselves exceed the available width.
    // Preserve a root only when it leaves room for the ellipsis and path tail.
    let prefix_len = if chars[..prefix_len].iter().map(|c| units(*c)).sum::<usize>() + 1 < budget {
        prefix_len
    } else {
        0
    };
    let prefix: String = chars[..prefix_len].iter().collect();
    let mut remaining = budget.saturating_sub(prefix.chars().map(units).sum::<usize>() + 1);
    let mut cut = chars.len();
    while cut > prefix_len && remaining >= units(chars[cut - 1]) {
        remaining -= units(chars[cut - 1]);
        cut -= 1;
    }
    format!("{}…{}", prefix, chars[cut..].iter().collect::<String>())
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

#[cfg(test)]
mod picker_path_tests {
    use super::elide_path_middle;

    #[test]
    fn resizing_reveals_the_full_path_and_keeps_drive_and_tail_when_narrow() {
        let path = r"D:\Firmware\SomeLongDeviceName\SomeLongBuildName\image";
        let narrow = elide_path_middle(path, 180.0);
        assert!(narrow.starts_with(r"D:\"));
        assert!(narrow.contains('…'));
        assert!(narrow.ends_with("image"));
        assert_eq!(elide_path_middle(path, 2000.0), path);
    }

    #[test]
    fn unix_unc_and_unicode_roots_survive_elision() {
        assert!(
            elide_path_middle("/home/user/firmware/verylongbuild/image", 100.0).starts_with("/…")
        );
        assert!(
            elide_path_middle(r"\\server\share\firmware\longbuild\image", 230.0)
                .starts_with(r"\\server\share\")
        );
        let result = elide_path_middle("D:/펌웨어/아주긴폴더이름/이미지.img", 100.0);
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
        for width in [0.0_f32, 1.0, 20.0, 100.0, 240.0] {
            let shown = elide_path_middle(&path, width);
            let budget = (width / (crate::theme::text_size::BODY_SMALL * 0.62))
                .floor()
                .max(1.0) as usize;
            let units: usize = shown
                .chars()
                .map(|c| {
                    if c == '…' || (c as u32) < 0x1100 {
                        1
                    } else {
                        2
                    }
                })
                .sum();
            assert!(units <= budget, "{width}: {shown}");
        }
    }
}
impl App {
    /// Name the connected model's required loader; without a device, show
    /// the standard MELF hint while the picker still accepts XML too.
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

    /// Keep the same supporting pane beside picker and selection steps.
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
        wizard_selection_step(size, width, title, body, Some((String::new(), vec![])))
    }
}
