//! Wizard navigation bars and small view widgets/helpers (nav buttons,
//! color blend/easing, device portrait, layout consts). Extracted from main.rs.

use crate::focus_button::{self as button, button};
use crate::*;
use iced::widget::{Space, canvas, column, container, row, text};
use iced::{Element, Length, Point, Radians, Rectangle, Renderer, Theme, mouse, window};
use ltbox_core::model::TB324ZC_MODEL;

const MATERIAL_PROGRESS_PERIOD: std::time::Duration = std::time::Duration::from_millis(1_400);
const MATERIAL_PROGRESS_FRAME: std::time::Duration = std::time::Duration::from_millis(33);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MaterialProgressSize {
    Standard,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct MaterialProgressMetrics {
    diameter: f32,
    stroke_width: f32,
    track_gap: f32,
}

fn material_progress_metrics(size: MaterialProgressSize) -> MaterialProgressMetrics {
    match size {
        MaterialProgressSize::Standard => MaterialProgressMetrics {
            diameter: 40.0,
            stroke_width: 4.0,
            track_gap: 4.0,
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct MaterialProgressArc {
    start_angle: f32,
    sweep_angle: f32,
}

fn material_progress_arc(phase: f32) -> MaterialProgressArc {
    let phase = phase.rem_euclid(1.0);
    let pulse = 0.5 - 0.5 * (std::f32::consts::TAU * phase).cos();
    MaterialProgressArc {
        start_angle: -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * phase,
        sweep_angle: std::f32::consts::TAU * (0.12 + 0.55 * pulse),
    }
}

fn material_progress_gap_angle(metrics: MaterialProgressMetrics, radius: f32) -> f32 {
    // Round caps extend half a stroke beyond each path endpoint. Add one full
    // stroke width to the token gap so the visible cap-to-cap space stays 4px.
    (metrics.track_gap + metrics.stroke_width) / radius
}

#[derive(Debug, Clone, Copy)]
struct MaterialProgress {
    size: MaterialProgressSize,
}

#[derive(Debug, Default)]
struct MaterialProgressState {
    started_at: Option<iced::time::Instant>,
    phase: f32,
}

impl canvas::Program<Message> for MaterialProgress {
    type State = MaterialProgressState;

    fn update(
        &self,
        state: &mut Self::State,
        event: &canvas::Event,
        _bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let canvas::Event::Window(window::Event::RedrawRequested(now)) = event else {
            return None;
        };
        let started_at = *state.started_at.get_or_insert(*now);
        state.phase =
            now.duration_since(started_at).as_secs_f32() / MATERIAL_PROGRESS_PERIOD.as_secs_f32();
        state.phase = state.phase.rem_euclid(1.0);
        Some(canvas::Action::request_redraw_at(
            *now + MATERIAL_PROGRESS_FRAME,
        ))
    }

    fn draw(
        &self,
        state: &Self::State,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let metrics = material_progress_metrics(self.size);
        let arc = material_progress_arc(state.phase);
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let radius = (bounds.width.min(bounds.height) - metrics.stroke_width) / 2.0;
        let gap_angle = material_progress_gap_angle(metrics, radius);
        let center = Point::new(bounds.width / 2.0, bounds.height / 2.0);

        let active_end = arc.start_angle + arc.sweep_angle;
        let track_start = active_end + gap_angle;
        let track_end = arc.start_angle + std::f32::consts::TAU - gap_angle;
        let track = canvas::Path::new(|builder| {
            builder.arc(canvas::path::Arc {
                center,
                radius,
                start_angle: Radians(track_start),
                end_angle: Radians(track_end),
            });
        });
        let active = canvas::Path::new(|builder| {
            builder.arc(canvas::path::Arc {
                center,
                radius,
                start_angle: Radians(arc.start_angle),
                end_angle: Radians(active_end),
            });
        });
        let palette = pal_of(theme);
        let stroke = canvas::Stroke::default()
            .with_width(metrics.stroke_width)
            .with_line_cap(canvas::LineCap::Round);
        frame.stroke(&track, stroke.with_color(palette.surface_container_highest));
        frame.stroke(&active, stroke.with_color(palette.primary));
        vec![frame.into_geometry()]
    }
}

pub(crate) fn material_circular_progress(size: MaterialProgressSize) -> Element<'static, Message> {
    let metrics = material_progress_metrics(size);
    canvas::Canvas::new(MaterialProgress { size })
        .width(Length::Fixed(metrics.diameter))
        .height(Length::Fixed(metrics.diameter))
        .into()
}

/// One full pass through the shape sequence.
const LOADING_INDICATOR_PERIOD: std::time::Duration = std::time::Duration::from_millis(4_000);
/// Active-indicator diameter. M3 scales this component; 48 is the size
/// that fits the popup loading slot the app already reserves.
pub(crate) const LOADING_INDICATOR_SIZE: f32 = 48.0;
/// Lobe counts the indicator morphs through. M3's loading indicator is a
/// loop over seven Material shapes; expressing them as harmonic lobe
/// counts gives the same "one shape flows into the next" reading without
/// needing a shape library and a polygon-interpolation pass.
const LOADING_INDICATOR_LOBES: [f32; 7] = [3.0, 4.0, 5.0, 4.0, 6.0, 5.0, 3.0];
/// How far the lobes push off the base circle, as a fraction of radius.
const LOADING_INDICATOR_AMPLITUDE: f32 = 0.13;
/// Points sampled around the outline. Enough that the fill reads as a
/// smooth curve rather than a polygon at this diameter.
const LOADING_INDICATOR_SAMPLES: usize = 160;

#[derive(Debug, Clone, Copy)]
struct MaterialLoadingIndicator;

/// Outline radius at angle `theta`, morphing between the lobe count at
/// `phase` and the next one. Crossfading two cosine harmonics keeps the
/// transition continuous — stepping the lobe count directly would jump.
fn loading_indicator_radius(base: f32, theta: f32, phase: f32) -> f32 {
    let span = LOADING_INDICATOR_LOBES.len() as f32;
    let pos = phase.rem_euclid(1.0) * span;
    let index = pos.floor() as usize % LOADING_INDICATOR_LOBES.len();
    let next = (index + 1) % LOADING_INDICATOR_LOBES.len();
    // Smoothstep the blend so each shape holds briefly before flowing on,
    // instead of the whole loop reading as one continuous wobble.
    let raw = pos - pos.floor();
    let blend = raw * raw * (3.0 - 2.0 * raw);

    let from = (LOADING_INDICATOR_LOBES[index] * theta).cos();
    let to = (LOADING_INDICATOR_LOBES[next] * theta).cos();
    let lobe = from * (1.0 - blend) + to * blend;
    base * (1.0 + LOADING_INDICATOR_AMPLITUDE * lobe)
}

impl canvas::Program<Message> for MaterialLoadingIndicator {
    type State = MaterialProgressState;

    fn update(
        &self,
        state: &mut Self::State,
        event: &canvas::Event,
        _bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let canvas::Event::Window(window::Event::RedrawRequested(now)) = event else {
            return None;
        };
        let started_at = *state.started_at.get_or_insert(*now);
        state.phase =
            now.duration_since(started_at).as_secs_f32() / LOADING_INDICATOR_PERIOD.as_secs_f32();
        state.phase = state.phase.rem_euclid(1.0);
        Some(canvas::Action::request_redraw_at(
            *now + MATERIAL_PROGRESS_FRAME,
        ))
    }

    fn draw(
        &self,
        state: &Self::State,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let center = Point::new(bounds.width / 2.0, bounds.height / 2.0);
        // Leave room for the lobes so the shape never clips its bounds.
        let base = (bounds.width.min(bounds.height) / 2.0) / (1.0 + LOADING_INDICATOR_AMPLITUDE);
        // Rotation is what turns a pulsing outline into something that
        // reads as active; one turn per shape cycle.
        let spin = std::f32::consts::TAU * state.phase;

        let path = canvas::Path::new(|builder| {
            for i in 0..=LOADING_INDICATOR_SAMPLES {
                let theta = std::f32::consts::TAU * (i as f32 / LOADING_INDICATOR_SAMPLES as f32);
                let radius = loading_indicator_radius(base, theta, state.phase);
                let angle = theta + spin;
                let point = Point::new(
                    center.x + radius * angle.cos(),
                    center.y + radius * angle.sin(),
                );
                if i == 0 {
                    builder.move_to(point);
                } else {
                    builder.line_to(point);
                }
            }
            builder.close();
        });

        frame.fill(&path, pal_of(theme).primary);
        vec![frame.into_geometry()]
    }
}

/// M3 Expressive loading indicator — the specified replacement for an
/// indeterminate circular progress indicator on short waits (roughly
/// 200 ms to 5 s). Unlike a progress ring it communicates through shape
/// and motion rather than an arc sweep, and it is never decorative.
pub(crate) fn material_loading_indicator() -> Element<'static, Message> {
    canvas::Canvas::new(MaterialLoadingIndicator)
        .width(Length::Fixed(LOADING_INDICATOR_SIZE))
        .height(Length::Fixed(LOADING_INDICATOR_SIZE))
        .into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExecPrimaryAction {
    StartOver,
    OpenFolder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ExecActionLayout {
    pub(crate) primary: Option<ExecPrimaryAction>,
    pub(crate) start_over_utility: bool,
}

impl ExecActionLayout {
    /// Whether this layout puts anything in the footer at all. While an
    /// operation runs it does not, and rendering the bar anyway leaves an
    /// empty band across the bottom of the execution screen.
    pub(crate) const fn has_any(self) -> bool {
        self.primary.is_some() || self.start_over_utility
    }
}

pub(crate) const fn exec_action_layout(
    is_busy: bool,
    is_error: bool,
    has_output: bool,
) -> ExecActionLayout {
    if is_busy {
        ExecActionLayout {
            primary: None,
            start_over_utility: false,
        }
    } else if has_output && !is_error {
        ExecActionLayout {
            primary: Some(ExecPrimaryAction::OpenFolder),
            start_over_utility: true,
        }
    } else {
        ExecActionLayout {
            primary: Some(ExecPrimaryAction::StartOver),
            start_over_utility: false,
        }
    }
}

/// True for localized confirmation labels. These enter an operation from a
/// review screen, so the action bar gives the primary control the error role.
pub(crate) fn is_start_label(label: &str) -> bool {
    label == ltbox_core::i18n::tr("btn_start").as_str()
        || label == ltbox_core::i18n::tr("btn_dump").as_str()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WizardNavLayout {
    pub(crate) destructive_primary: bool,
}

pub(crate) fn wizard_nav_layout(next_label: &str) -> WizardNavLayout {
    WizardNavLayout {
        destructive_primary: is_start_label(next_label),
    }
}

fn action_outlined_style(t: &Theme, status: button::Status) -> button::Style {
    let p = pal_of(t);
    if matches!(status, button::Status::Disabled) {
        return button::Style {
            background: None,
            text_color: with_alpha(p.on_surface, 0.38),
            border: iced::Border {
                color: with_alpha(p.on_surface, 0.12),
                width: 1.0,
                radius: theme::button_radius(status).into(),
            },
            ..Default::default()
        };
    }

    button::Style {
        background: theme::state_layer_bg(status, p.on_surface).map(Into::into),
        text_color: p.on_surface,
        border: iced::Border {
            color: p.outline,
            width: 1.0,
            radius: theme::button_radius(status).into(),
        },
        ..Default::default()
    }
}

fn action_error_filled_style(t: &Theme, status: button::Status) -> button::Style {
    let p = pal_of(t);
    if matches!(status, button::Status::Disabled) {
        return button::Style {
            background: Some(with_alpha(p.on_surface, 0.12).into()),
            text_color: with_alpha(p.on_surface, 0.38),
            border: iced::Border {
                radius: theme::button_radius(status).into(),
                ..Default::default()
            },
            ..Default::default()
        };
    }

    button::Style {
        background: Some(theme::mix_color(p.error, p.on_error, theme::state_alpha(status)).into()),
        text_color: p.on_error,
        border: iced::Border {
            radius: theme::button_radius(status).into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn fab_tooltip<'a>(inner: Element<'a, Message>, label: String) -> Element<'a, Message> {
    iced::widget::tooltip(
        inner,
        container(text(label).size(12))
            .padding([6, 10])
            .style(|t: &Theme| {
                let p = pal_of(t);
                container::Style {
                    background: Some(p.inverse_surface.into()),
                    text_color: Some(p.inverse_on_surface),
                    border: iced::Border {
                        radius: theme::shape::XS.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }
            }),
        iced::widget::tooltip::Position::Top,
    )
    .into()
}

#[derive(Debug, Clone, Copy)]
enum ActionButtonRole {
    Outlined,
    Primary,
    Error,
}

fn action_button<'a>(
    icon: Option<iced::widget::Text<'static, Theme, iced::Renderer>>,
    label: String,
    msg: Option<Message>,
    disabled_hint: Option<String>,
    role: ActionButtonRole,
) -> Element<'a, Message> {
    let style = match role {
        ActionButtonRole::Outlined => action_outlined_style,
        ActionButtonRole::Primary => md_filled_btn_style,
        ActionButtonRole::Error => action_error_filled_style,
    };
    let label = text(label)
        .size(theme::text_size::BODY_MEDIUM)
        .font(theme::emphasis::medium())
        .wrapping(iced::widget::text::Wrapping::None);
    let content: Element<'a, Message> = if let Some(icon) = icon {
        row![icon.size(18), label]
            .spacing(8)
            .align_y(iced::Alignment::Center)
            .into()
    } else {
        label.into()
    };
    let mut action = button(
        container(content)
            .height(Length::Fill)
            .center_y(Length::Fill),
    )
    .height(Length::Fixed(M3_BUTTON_HEIGHT))
    .padding([0, 16])
    .style(style);
    let enabled = msg.is_some();
    if let Some(msg) = msg {
        action = action.on_press(msg);
    }
    let action = action.into();
    if !enabled && let Some(hint) = disabled_hint {
        return fab_tooltip(action, hint);
    }
    action
}

pub(crate) fn wizard_secondary_action<'a>(
    icon: iced::widget::Text<'static, Theme, iced::Renderer>,
    label: String,
    msg: Option<Message>,
) -> Element<'a, Message> {
    action_button(Some(icon), label, msg, None, ActionButtonRole::Outlined)
}

pub(crate) fn wizard_primary_action<'a>(
    icon: iced::widget::Text<'static, Theme, iced::Renderer>,
    label: String,
    msg: Option<Message>,
    disabled_hint: Option<String>,
    destructive: bool,
) -> Element<'a, Message> {
    let role = if destructive {
        ActionButtonRole::Error
    } else {
        ActionButtonRole::Primary
    };
    action_button(Some(icon), label, msg, disabled_hint, role)
}

pub(crate) fn wizard_action_footer<'a>(
    leading: impl Into<Element<'a, Message>>,
    trailing: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    let leading = leading.into();
    let trailing = trailing.into();

    column![
        iced::widget::rule::horizontal(1).style(shell_rule_style),
        container(
            row![leading, Space::new().width(Length::Fill), trailing]
                .spacing(ACTION_BUTTON_SPACING)
                .align_y(iced::Alignment::Center)
                .height(Length::Fill)
                .width(Length::Fill),
        )
        .padding([0, 24])
        .width(Length::Fill)
        .height(Length::Fixed(WIZARD_ACTION_BAR_HEIGHT - 1.0))
        .style(|t: &Theme| container::Style {
            background: Some(pal_of(t).surface_container_low.into()),
            ..Default::default()
        }),
    ]
    .width(Length::Fill)
    .height(Length::Fixed(WIZARD_ACTION_BAR_HEIGHT))
    .into()
}

pub(crate) fn empty_wizard_nav<'a>() -> Element<'a, Message> {
    Space::new().height(0).into()
}

/// Shared action target. Iced paints the whole hit box, so the visible
/// container also occupies the 48 dp accessible target.
pub(crate) const M3_BUTTON_HEIGHT: f32 = 48.0;

/// Interior padding for text fields and pick lists. At the default the
/// dropdown options came out around 27 px tall; this puts a 13 px option
/// at ~41 px, in reach of the 48 dp M3 asks of a menu item without
/// making the settings rows tower over their labels.
fn m3_button<'a>(
    label: String,
    style: fn(&Theme, button::Status) -> button::Style,
) -> button::Button<'a, Message> {
    button(
        container(
            text(label)
                .size(theme::text_size::BODY_MEDIUM)
                .font(theme::emphasis::medium())
                .line_height(20.0 / 14.0)
                // A localized label must never shred into a per-glyph
                // column when the parent row is tight; let it overflow.
                .wrapping(iced::widget::text::Wrapping::None),
        )
        .height(Length::Fill)
        .center_y(Length::Fill),
    )
    // Vertical padding stays 0 — the fixed height plus the centering
    // container own the vertical metrics.
    .padding(iced::Padding {
        top: 0.0,
        right: M3_BUTTON_H_PADDING,
        bottom: 0.0,
        left: M3_BUTTON_H_PADDING,
    })
    .height(Length::Fixed(M3_BUTTON_HEIGHT))
    .style(style)
}

/// Interior padding of the Dashboard's hero device card. Sized against
/// its `LG` corner via M3's `outer radius - padding = inner
/// radius` rule.
pub(crate) const DEVICE_CARD_PADDING: f32 = 24.0;

/// Icon-button size. M3 requires extra-small and small icon buttons to
/// carry a pointer target of at least 48x48 even when the painted
/// container is smaller.
///
/// iced paints a button's style across the button's own rect, so a 32 px
/// disc inside a 48 px target would leave the state layer on the
/// invisible box instead of the visible shape. Rather than lose hover
/// feedback, the container *is* the target: 48 px is a size M3 lists for
/// icon buttons, so this stays on the scale instead of inventing one.
pub(crate) const M3_ICON_BUTTON_SIZE: f32 = 48.0;

/// Icon button at the M3 target size, with the glyph centred inside.
/// Hand-built versions of this at two call sites came out at 32 px.
pub(crate) fn m3_icon_button(
    glyph: iced::widget::Text<'static, Theme, iced::Renderer>,
    glyph_size: f32,
    style: fn(&Theme, button::Status) -> button::Style,
) -> button::Button<'static, Message> {
    button(
        container(glyph.size(glyph_size))
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill),
    )
    .width(Length::Fixed(M3_ICON_BUTTON_SIZE))
    .height(Length::Fixed(M3_ICON_BUTTON_SIZE))
    .padding(0)
    .style(style)
}

/// M3 filled button at the common height. Returns the `Button` so the
/// caller still owns `on_press` (several sites gate it on state).
pub(crate) fn m3_filled_button<'a>(label: String) -> button::Button<'a, Message> {
    m3_button(label, md_filled_btn_style)
}

/// M3 outlined button at the common height — the dismissive half of a
/// dialog's outline-dismiss / filled-confirm action pair.
pub(crate) fn m3_outlined_button<'a>(label: String) -> button::Button<'a, Message> {
    m3_button(label, action_outlined_style)
}

/// M3 text button at the common height. Keep this for intentionally low-key
/// standalone actions; dialog dismiss/confirm pairs use [`m3_outlined_button`].
pub(crate) fn m3_text_button<'a>(label: String) -> button::Button<'a, Message> {
    m3_button(label, md_text_btn_style)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WizardLeadingAction {
    None,
    Back,
    Cancel,
}

fn wizard_nav_actions<'a>(
    leading_action: WizardLeadingAction,
    next_label: &str,
    can_next: bool,
    disabled_next_hint: Option<String>,
    back_label: &str,
    back_msg: Message,
    next_msg: Message,
) -> Element<'a, Message> {
    let layout = wizard_nav_layout(next_label);
    let mut trailing = row![]
        .spacing(ACTION_BUTTON_SPACING)
        .align_y(iced::Alignment::Center)
        .height(Length::Fill);

    let (cancel_label, cancel_msg) = match leading_action {
        WizardLeadingAction::None => (
            ltbox_core::i18n::tr("btn_cancel").to_string(),
            Message::StartOver,
        ),
        WizardLeadingAction::Cancel => (back_label.to_string(), back_msg),
        WizardLeadingAction::Back => {
            trailing = trailing.push(action_button(
                None,
                back_label.to_string(),
                Some(back_msg),
                None,
                ActionButtonRole::Outlined,
            ));
            (
                ltbox_core::i18n::tr("btn_cancel").to_string(),
                Message::StartOver,
            )
        }
    };

    let leading = row![action_button(
        None,
        cancel_label,
        Some(cancel_msg),
        None,
        ActionButtonRole::Outlined,
    )]
    .align_y(iced::Alignment::Center)
    .height(Length::Fill);

    trailing = trailing.push(action_button(
        None,
        next_label.to_string(),
        can_next.then_some(next_msg),
        disabled_next_hint,
        if layout.destructive_primary {
            ActionButtonRole::Error
        } else {
            ActionButtonRole::Primary
        },
    ));

    wizard_action_footer(leading, trailing)
}

pub(crate) fn wizard_nav<'a>(
    can_back: bool,
    next_label: &str,
    can_next: bool,
    back_label: &str,
) -> Element<'a, Message> {
    wizard_nav_actions(
        if can_back {
            WizardLeadingAction::Back
        } else {
            WizardLeadingAction::None
        },
        next_label,
        can_next,
        None,
        back_label,
        Message::Root(RootMsg::RootBack),
        Message::Root(RootMsg::RootNext),
    )
}

// =========================================================================
// Reusable widgets
// =========================================================================

/// Navigation-drawer item and compact-rail geometry.
pub(crate) const NAV_BTN_HEIGHT: f32 = 56.0;
/// Keep the same item height during drawer expansion to avoid vertical jumps.
pub(crate) const NAV_BTN_COLLAPSED_HEIGHT: f32 = NAV_BTN_HEIGHT;
pub(crate) const NAV_BTN_COLLAPSED_WIDTH: f32 = 56.0;
pub(crate) const NAV_INDICATOR_COLLAPSED_HEIGHT: f32 = 32.0;

/// Collapsed sidebar rail width (icon-only). This is also the fixed baseline
/// used for window-size classification: using the adaptive rendered width
/// here would make the class oscillate in the rail-width feedback band.
pub(crate) const SIDEBAR_RAIL_WIDTH: f32 = 80.0;
pub(crate) const SIDEBAR_EXPANDED_WIDTH: f32 = 232.0;

pub(crate) fn nav_btn<'a>(
    view: View,
    label: &str,
    active: bool,
    enabled: bool,
    label_alpha: f32,
    collapsed: bool,
) -> Element<'a, Message> {
    // Selection changes the indicator rather than resizing the glyph.
    let icon = lucide_icon(view.nav_icon(), 24.0, move |t: &Theme| {
        let p = pal_of(t);
        if !enabled {
            with_alpha(p.on_surface, 0.38)
        } else if active {
            p.on_surface
        } else {
            p.on_surface_variant
        }
    });
    let icon_slot: Element<'a, Message> = container(icon)
        .width(Length::Fixed(24.0))
        .height(Length::Fixed(24.0))
        .align_x(iced::alignment::Horizontal::Center)
        .align_y(iced::alignment::Vertical::Center)
        .into();

    // Both forms keep the icon center 40px from the rail's left edge.
    let mut inner = iced::widget::row![icon_slot]
        .spacing(12)
        .align_y(iced::Alignment::Center);
    if !collapsed && label_alpha > 0.0 {
        // Resolve the base text color (hover / disabled apply via the
        // button style below; here we just fade the label in along
        // the spring), then re-apply alpha so the glyph fades in step
        // with the sidebar width tween. The active row sits on a
        // `secondary_container` pill, so its label takes the matching
        // on-color; inactive rows sit on the bare panel.
        let alpha = label_alpha;
        let base_label_color = move |t: &Theme| -> iced::Color {
            let p = pal_of(t);
            if !enabled {
                with_alpha(p.on_surface, 0.38)
            } else {
                p.on_surface
            }
        };
        let mut label_text = text(label.to_string())
            .size(theme::text_size::BODY_MEDIUM)
            .height(Length::Fill)
            .align_y(iced::alignment::Vertical::Center);
        if active && enabled {
            label_text = label_text.font(theme::emphasis::medium());
        }
        inner = inner.push(
            label_text
                // Forbid wrapping: during the sidebar spring there is
                // a brief window where the panel is wide enough to
                // mount the label but too narrow for long glyphs to
                // fit on one line. Wrapping into 2 rows mid-tween then
                // collapsing back to 1 row reads as a jank flicker.
                // No-wrap lets the text overflow under the panel's
                // clip rect instead — invisible until width settles.
                .wrapping(iced::widget::text::Wrapping::None)
                .style(move |t: &Theme| iced::widget::text::Style {
                    color: Some(with_alpha(base_label_color(t), alpha)),
                }),
        );
    }
    let content: Element<'a, Message> = container(inner)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced::alignment::Horizontal::Left)
        .align_y(iced::Alignment::Center)
        .into();

    let item_width = if collapsed {
        Length::Fixed(NAV_BTN_COLLAPSED_WIDTH)
    } else {
        Length::Fill
    };
    let item_height = if collapsed {
        NAV_BTN_COLLAPSED_HEIGHT
    } else {
        NAV_BTN_HEIGHT
    };

    // Selection is its own layer behind the interactive item. The button
    // paints only pointer state, so active fill geometry stays independent
    // from the press target and collapses to the rail's 56x32 indicator.
    let indicator_height = if collapsed {
        NAV_INDICATOR_COLLAPSED_HEIGHT
    } else {
        NAV_BTN_HEIGHT
    };
    let indicator: Element<'a, Message> = if active {
        container(Space::new().width(Length::Fill).height(Length::Fill))
            .width(item_width)
            .height(Length::Fixed(indicator_height))
            .style(|t: &Theme| container::Style {
                background: Some(pal_of(t).secondary_container.into()),
                border: iced::Border {
                    radius: theme::shape::FULL.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    } else {
        Space::new()
            .width(item_width)
            .height(Length::Fixed(indicator_height))
            .into()
    };
    let indicator_layer: Element<'a, Message> = container(indicator)
        .width(item_width)
        .height(Length::Fixed(item_height))
        .align_x(iced::alignment::Horizontal::Center)
        .align_y(iced::alignment::Vertical::Center)
        .into();

    let btn = button(content)
        .padding([0, if collapsed { 16 } else { 20 }])
        .width(item_width)
        .height(Length::Fixed(item_height))
        .style(move |t: &Theme, status| {
            let p = pal_of(t);
            let pill = iced::Border {
                radius: theme::shape::FULL.into(),
                ..Default::default()
            };
            if !enabled {
                return button::Style {
                    background: None,
                    text_color: with_alpha(p.on_surface, 0.38),
                    border: pill,
                    ..Default::default()
                };
            }
            button::Style {
                background: theme::state_layer_bg(status, p.on_surface).map(Into::into),
                text_color: if active {
                    p.on_surface
                } else {
                    p.on_surface_variant
                },
                border: pill,
                ..Default::default()
            }
        });
    let btn: Element<'a, Message> = if enabled {
        btn.on_press(Message::Navigate(view)).into()
    } else {
        btn.into()
    };
    iced::widget::Stack::with_children(vec![indicator_layer, btn])
        .width(item_width)
        .height(Length::Fixed(item_height))
        .into()
}

// Device portrait handles — built once, cloned each render.
// Unknown models fall through to `GENERIC_TABLET_SVG_HANDLE`.
static LAVIETAB9QHD1_HANDLE: std::sync::LazyLock<iced::widget::image::Handle> =
    std::sync::LazyLock::new(|| {
        iced::widget::image::Handle::from_bytes(
            include_bytes!("../assets/devices/9qhd1.png").as_slice(),
        )
    });
static TB320FC_HANDLE: std::sync::LazyLock<iced::widget::image::Handle> =
    std::sync::LazyLock::new(|| {
        iced::widget::image::Handle::from_bytes(
            include_bytes!("../assets/devices/tb320fc.png").as_slice(),
        )
    });
static TB321FU_HANDLE: std::sync::LazyLock<iced::widget::image::Handle> =
    std::sync::LazyLock::new(|| {
        iced::widget::image::Handle::from_bytes(
            include_bytes!("../assets/devices/tb321fu.png").as_slice(),
        )
    });
static TB322FC_HANDLE: std::sync::LazyLock<iced::widget::image::Handle> =
    std::sync::LazyLock::new(|| {
        iced::widget::image::Handle::from_bytes(
            include_bytes!("../assets/devices/tb322fc.png").as_slice(),
        )
    });
static TB323FU_HANDLE: std::sync::LazyLock<iced::widget::image::Handle> =
    std::sync::LazyLock::new(|| {
        iced::widget::image::Handle::from_bytes(
            include_bytes!("../assets/devices/tb323fu.png").as_slice(),
        )
    });
static TB324ZC_HANDLE: std::sync::LazyLock<iced::widget::image::Handle> =
    std::sync::LazyLock::new(|| {
        iced::widget::image::Handle::from_bytes(
            include_bytes!("../assets/devices/tb324zc.png").as_slice(),
        )
    });
static TB376FC_HANDLE: std::sync::LazyLock<iced::widget::image::Handle> =
    std::sync::LazyLock::new(|| {
        iced::widget::image::Handle::from_bytes(
            include_bytes!("../assets/devices/tb376fc.png").as_slice(),
        )
    });
static TB520FU_HANDLE: std::sync::LazyLock<iced::widget::image::Handle> =
    std::sync::LazyLock::new(|| {
        iced::widget::image::Handle::from_bytes(
            include_bytes!("../assets/devices/tb520fu.png").as_slice(),
        )
    });
static TB710FU_HANDLE: std::sync::LazyLock<iced::widget::image::Handle> =
    std::sync::LazyLock::new(|| {
        iced::widget::image::Handle::from_bytes(
            include_bytes!("../assets/devices/tb710fu.png").as_slice(),
        )
    });
static GENERIC_TABLET_SVG_HANDLE: std::sync::LazyLock<iced::widget::svg::Handle> =
    std::sync::LazyLock::new(|| {
        iced::widget::svg::Handle::from_memory(
            include_bytes!("../assets/devices/generic_tablet.svg").as_slice(),
        )
    });

/// Asset for the Dashboard portrait slot.
pub(crate) enum DevicePortrait {
    Png(iced::widget::image::Handle),
    Svg(iced::widget::svg::Handle),
}

pub(crate) fn device_portrait(model: &str) -> DevicePortrait {
    match model.to_uppercase().as_str() {
        "LAVIETAB9QHD1" => DevicePortrait::Png(LAVIETAB9QHD1_HANDLE.clone()),
        "TB320FC" => DevicePortrait::Png(TB320FC_HANDLE.clone()),
        "TB321FU" => DevicePortrait::Png(TB321FU_HANDLE.clone()),
        "TB322FC" => DevicePortrait::Png(TB322FC_HANDLE.clone()),
        "TB323FU" => DevicePortrait::Png(TB323FU_HANDLE.clone()),
        TB324ZC_MODEL => DevicePortrait::Png(TB324ZC_HANDLE.clone()),
        "TB376FC" | "TB390FU" => DevicePortrait::Png(TB376FC_HANDLE.clone()),
        "TB520FU" => DevicePortrait::Png(TB520FU_HANDLE.clone()),
        "TB710FU" => DevicePortrait::Png(TB710FU_HANDLE.clone()),
        _ => DevicePortrait::Svg(GENERIC_TABLET_SVG_HANDLE.clone()),
    }
}

/// Product-specific content-width threshold for the two-pane wizard.
/// This is measured after navigation, not an M3 window-size breakpoint.
const EXPANDED_CONTENT_WIDTH: f32 = 1000.0;

#[derive(Debug, Clone, Copy)]
pub(crate) struct ListRowMetrics {
    pub(crate) height: f32,
    pub(crate) label_size: f32,
    pub(crate) desc_size: f32,
    pub(crate) padding: iced::Padding,
    /// Gap between the label and its description.
    pub(crate) text_gap: f32,
    /// Gap between the icon and the text stack.
    pub(crate) icon_gap: f32,
}

pub(crate) const WIZARD_CONFIRM_MAX_WIDTH: f32 = 660.0;
pub(crate) const WIZARD_TOP_APP_BAR_HEIGHT: f32 = 64.0;
pub(crate) const WIZARD_TOP_APP_BAR_MAX_WIDTH: f32 = 1040.0;
pub(crate) const WIZARD_ACTION_BAR_HEIGHT: f32 = 60.0;
pub(crate) const ACTION_BUTTON_SPACING: f32 = 10.0;
pub(crate) const SETTINGS_PANEL_MAX_WIDTH: f32 = 620.0;

pub(crate) const FLASH_PARTS_MARKER_CELL_WIDTH: f32 = 32.0;
pub(crate) const FLASH_PARTS_MARKER_SIZE: f32 = 16.0;
pub(crate) const FLASH_PARTS_ERASE_DASH_WIDTH: f32 = 9.0;
pub(crate) const FLASH_PARTS_ERASE_DASH_HEIGHT: f32 = 2.0;

pub(crate) fn centered_max_width<'a>(
    content: impl Into<Element<'a, Message>>,
    max_width: f32,
) -> Element<'a, Message> {
    container(
        container(content.into())
            .width(Length::Fill)
            .max_width(max_width),
    )
    .width(Length::Fill)
    .center_x(Length::Fill)
    .into()
}

/// Width-capped column centred horizontally and anchored to the top of its
/// pane. M3 aligns pane content from the top — its ruler set runs Title then
/// Content, fixing where content *begins* — and reserves vertical centring for
/// dialogs. Centring here left a short list floating in the middle of the
/// window with a wide band of dead space above and below it.
pub(crate) fn centered_step<'a>(
    content: impl Into<Element<'a, Message>>,
    max_width: f32,
) -> Element<'a, Message> {
    container(
        container(content.into())
            .width(Length::Fill)
            .max_width(max_width),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .center_x(Length::Fill)
    .align_y(iced::Alignment::Start)
    .into()
}

/// Shared window class for layout decisions; callers need no width arithmetic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WindowSizeClass {
    Compact,
    Expanded,
}

impl WindowSizeClass {
    /// Classify the available content width after the sidebar rail.
    pub(crate) fn for_content_width(content_width: f32) -> Self {
        if content_width >= EXPANDED_CONTENT_WIDTH {
            Self::Expanded
        } else {
            Self::Compact
        }
    }
}

impl App {
    /// Shared layout class based on window width minus the compact rail.
    ///
    /// Keep this baseline independent of the rendered rail width. Expanded
    /// layout uses a wider fixed rail, and feeding that width back into this
    /// decision would oscillate between the two classes.
    pub(crate) fn window_size_class(&self) -> WindowSizeClass {
        WindowSizeClass::for_content_width(self.window_size.0 - SIDEBAR_RAIL_WIDTH)
    }

    /// Width cap for a single-column wizard list. Selection rows keep the same
    /// desktop density in both window classes; only their surrounding layout
    /// changes between one and two panes.
    pub(crate) fn wizard_list_max_width(&self, base: f32) -> f32 {
        base
    }

    /// The adaptive dimensions of one single-column list row, resolved together
    /// so a caller uses one class for height and text.
    pub(crate) fn wizard_list_metrics(&self, label_base: f32, desc_base: f32) -> ListRowMetrics {
        let (label_size, desc_size) = self.wizard_list_text(label_base, desc_base);
        ListRowMetrics {
            height: self.wizard_list_row_height(),
            label_size,
            desc_size,
            padding: iced::Padding::from([
                WIZARD_LIST_VERTICAL_PADDING,
                WIZARD_LIST_HORIZONTAL_PADDING,
            ]),
            text_gap: WIZARD_LIST_TEXT_GAP,
            icon_gap: WIZARD_LIST_ICON_GAP,
        }
    }

    pub(crate) fn wizard_list_row_height(&self) -> f32 {
        WIZARD_LIST_CARD_HEIGHT
    }

    /// Brand marks may use 32 px while stroke glyphs use 24 px; neither grows
    /// with the window.
    pub(crate) fn wizard_list_icon(&self, base: f32) -> f32 {
        base.min(WIZARD_LIST_ICON_SIZE)
    }

    /// Label and description sizes for a list row, from that row's own bases.
    pub(crate) fn wizard_list_text(&self, label_base: f32, desc_base: f32) -> (f32, f32) {
        (label_base, desc_base)
    }
}

pub(crate) fn wizard_nav_generic<'a>(
    can_back: bool,
    next_label: &str,
    can_next: bool,
    back_label: &str,
    back_msg: Message,
    next_msg: Message,
) -> Element<'a, Message> {
    wizard_nav_generic_with_disabled_next_tooltip(
        can_back, next_label, can_next, None, back_label, back_msg, next_msg,
    )
}

pub(crate) fn wizard_nav_generic_with_leading_action<'a>(
    leading_action: WizardLeadingAction,
    next_label: &str,
    can_next: bool,
    leading_label: &str,
    leading_msg: Message,
    next_msg: Message,
) -> Element<'a, Message> {
    wizard_nav_actions(
        leading_action,
        next_label,
        can_next,
        None,
        leading_label,
        leading_msg,
        next_msg,
    )
}

pub(crate) fn wizard_nav_generic_with_disabled_next_tooltip<'a>(
    can_back: bool,
    next_label: &str,
    can_next: bool,
    disabled_next_hint: Option<String>,
    back_label: &str,
    back_msg: Message,
    next_msg: Message,
) -> Element<'a, Message> {
    wizard_nav_actions(
        if can_back {
            WizardLeadingAction::Back
        } else {
            WizardLeadingAction::None
        },
        next_label,
        can_next,
        disabled_next_hint,
        back_label,
        back_msg,
        next_msg,
    )
}

pub(crate) fn wizard_nav_cancel_generic_with_disabled_next_tooltip<'a>(
    next_label: &str,
    can_next: bool,
    disabled_next_hint: Option<String>,
    cancel_label: &str,
    cancel_msg: Message,
    next_msg: Message,
) -> Element<'a, Message> {
    wizard_nav_actions(
        WizardLeadingAction::Cancel,
        next_label,
        can_next,
        disabled_next_hint,
        cancel_label,
        cancel_msg,
        next_msg,
    )
}

#[cfg(test)]
mod tests {
    use super::{
        App, DevicePortrait, MaterialProgressSize, WIZARD_ACTION_BAR_HEIGHT,
        WIZARD_LIST_CARD_HEIGHT, WIZARD_LIST_GLYPH_ICON_SIZE, WIZARD_LIST_ICON_SIZE,
        WindowSizeClass, device_portrait, material_progress_arc, material_progress_gap_angle,
        material_progress_metrics, wizard_nav_layout,
    };

    #[test]
    fn tb324zc_has_its_own_portrait_handle() {
        match (device_portrait("TB323FU"), device_portrait("TB324ZC")) {
            (DevicePortrait::Png(tb323fu), DevicePortrait::Png(tb324zc)) => {
                assert!(tb323fu != tb324zc);
            }
            _ => panic!("both models must dispatch to a PNG portrait"),
        }
    }

    #[test]
    fn xiaoxin_pro13_models_use_the_shared_png_portrait() {
        assert!(matches!(device_portrait("TB376FC"), DevicePortrait::Png(_)));
        assert!(matches!(device_portrait("TB390FU"), DevicePortrait::Png(_)));
    }

    #[test]
    fn wizard_selection_rows_keep_desktop_density_across_window_classes() {
        // Override persisted dimensions so this coverage is machine-independent.
        let mut app = App::default();
        for content_width in [756.0, 999.0, 1000.0, 1256.0, 4000.0] {
            app.window_size.0 = content_width + crate::SIDEBAR_RAIL_WIDTH;
            assert_eq!(app.wizard_list_row_height(), WIZARD_LIST_CARD_HEIGHT);
            assert_eq!(
                app.wizard_list_icon(WIZARD_LIST_ICON_SIZE),
                WIZARD_LIST_ICON_SIZE
            );
            assert_eq!(
                app.wizard_list_icon(WIZARD_LIST_GLYPH_ICON_SIZE),
                WIZARD_LIST_GLYPH_ICON_SIZE
            );
        }
    }

    #[test]
    fn material_progress_metrics_match_m3_tokens() {
        let standard = material_progress_metrics(MaterialProgressSize::Standard);
        assert_eq!(standard.diameter, 40.0);
        assert_eq!(standard.stroke_width, 4.0);
        assert_eq!(standard.track_gap, 4.0);
    }

    #[test]
    fn material_progress_arc_wraps_phase_and_bounds_sweep() {
        let start = material_progress_arc(0.0);
        let wrapped = material_progress_arc(1.0);
        assert!((start.start_angle - wrapped.start_angle).abs() < f32::EPSILON);
        assert!((start.sweep_angle - wrapped.sweep_angle).abs() < f32::EPSILON);

        for phase in [0.0, 0.125, 0.25, 0.5, 0.75, 0.999] {
            let arc = material_progress_arc(phase);
            assert!(arc.sweep_angle >= std::f32::consts::TAU * 0.12);
            assert!(arc.sweep_angle <= std::f32::consts::TAU * 0.67);
        }
    }

    #[test]
    fn material_progress_gap_accounts_for_round_caps() {
        let metrics = material_progress_metrics(MaterialProgressSize::Standard);
        let radius = (metrics.diameter - metrics.stroke_width) / 2.0;
        let centerline_gap = material_progress_gap_angle(metrics, radius) * radius;
        assert_eq!(centerline_gap, metrics.track_gap + metrics.stroke_width);
    }

    #[test]
    fn wizard_nav_layout_marks_confirmation_actions_destructive() {
        assert_eq!(WIZARD_ACTION_BAR_HEIGHT, 60.0);
        for key in ["btn_start", "btn_dump"] {
            let label = ltbox_core::i18n::tr(key);
            let layout = wizard_nav_layout(label.as_str());
            assert!(layout.destructive_primary);
        }

        let next_label = ltbox_core::i18n::tr("btn_next");
        let next = wizard_nav_layout(next_label.as_str());
        assert!(!next.destructive_primary);
    }

    #[test]
    fn window_size_class_uses_the_shared_breakpoint() {
        for (width, class) in [
            (756.0, WindowSizeClass::Compact),
            (999.999, WindowSizeClass::Compact),
            (1000.0, WindowSizeClass::Expanded),
            (1256.0, WindowSizeClass::Expanded),
            (4000.0, WindowSizeClass::Expanded),
        ] {
            assert_eq!(WindowSizeClass::for_content_width(width), class);
        }
    }

    #[test]
    fn window_size_class_accounts_for_the_sidebar_rail() {
        let mut app = App::default();
        for (window_width, content_width, class) in [
            (820.0, 740.0, WindowSizeClass::Compact),
            (1320.0, 1240.0, WindowSizeClass::Expanded),
        ] {
            app.window_size.0 = window_width;
            assert_eq!(window_width - crate::SIDEBAR_RAIL_WIDTH, content_width);
            assert_eq!(app.window_size_class(), class);
        }
    }

    #[test]
    fn window_size_class_stays_expanded_in_the_rail_width_feedback_band() {
        let mut app = App::default();
        for window_width in [1080.0, 1100.0, 1200.0, 1231.0] {
            app.window_size.0 = window_width;

            assert_eq!(
                WindowSizeClass::for_content_width(window_width - crate::SIDEBAR_RAIL_WIDTH),
                WindowSizeClass::Expanded
            );
            assert_eq!(
                WindowSizeClass::for_content_width(window_width - crate::SIDEBAR_EXPANDED_WIDTH),
                WindowSizeClass::Compact
            );
            assert_eq!(app.window_size_class(), WindowSizeClass::Expanded);
        }
    }
}
