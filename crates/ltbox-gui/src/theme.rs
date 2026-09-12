//! Material 3 Expressive color system — indigo-seed tonal palettes.
//!
//! Roles per m3.material.io/styles/color/roles. All hand-picked colors
//! go through [`Palette`] so light/dark + re-theming live in one place.

use iced::{Color, color};
use std::sync::RwLock;

/// Semantic color slots per Material 3.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub primary: Color,
    pub on_primary: Color,
    pub primary_container: Color,
    pub on_primary_container: Color,

    pub secondary: Color,
    pub secondary_container: Color,
    pub on_secondary_container: Color,

    pub tertiary: Color,
    pub on_tertiary: Color,

    pub error: Color,
    pub on_error: Color,
    pub error_container: Color,
    pub on_error_container: Color,

    /// Success — M3 doesn't ship this; tonal family of tertiary green.
    pub success: Color,
    pub on_success: Color,
    pub warning: Color,
    pub warning_container: Color,
    pub on_warning_container: Color,

    pub background: Color,

    pub surface: Color,
    pub surface_bright: Color,
    pub inverse_surface: Color,
    pub inverse_on_surface: Color,
    #[allow(dead_code)] // Reserved semantic role for actions on inverse surfaces.
    pub inverse_primary: Color,
    #[allow(dead_code)] // Full surface ramp, even when no current region uses it.
    pub surface_container_lowest: Color,
    pub surface_container_low: Color,
    pub surface_container: Color,
    pub surface_container_high: Color,
    pub surface_container_highest: Color,
    pub on_surface: Color,
    pub on_surface_variant: Color,

    pub outline: Color,
    pub outline_variant: Color,

    pub scrim: Color,
    pub shadow: Color,
}

/// Light palette — indigo primary, neutral surfaces.
pub const LIGHT: Palette = Palette {
    primary: color!(0x465AAA),
    on_primary: color!(0xFFFFFF),
    primary_container: color!(0xDDE1FF),
    on_primary_container: color!(0x001A43),

    secondary: color!(0x5B5D72),
    secondary_container: color!(0xE0E1F9),
    on_secondary_container: color!(0x181A2C),

    tertiary: color!(0x76546F),
    on_tertiary: color!(0xFFFFFF),

    error: color!(0xBA1A1A),
    on_error: color!(0xFFFFFF),
    error_container: color!(0xFFDAD6),
    on_error_container: color!(0x410002),

    success: color!(0x216C2A),
    on_success: color!(0xFFFFFF),
    warning: color!(0x735B00),
    warning_container: color!(0xFFF0C2),
    on_warning_container: color!(0x241A00),

    background: color!(0xFBF8FD),

    surface: color!(0xFBF8FD),
    surface_bright: color!(0xFBF8FD),
    inverse_surface: color!(0x303036),
    inverse_on_surface: color!(0xF2EFF6),
    inverse_primary: color!(0xB5C4FF),
    surface_container_lowest: color!(0xFFFFFF),
    surface_container_low: color!(0xF5F2F7),
    surface_container: color!(0xEFECF1),
    surface_container_high: color!(0xE9E7EB),
    surface_container_highest: color!(0xE3E1E6),
    on_surface: color!(0x1B1B21),
    on_surface_variant: color!(0x47464F),

    outline: color!(0x77767F),
    outline_variant: color!(0xC7C5D0),

    scrim: color!(0x000000),
    shadow: color!(0x000000),
};

/// Dark palette — LIGHT shifted along the M3 tonal scale.
pub const DARK: Palette = Palette {
    primary: color!(0xB5C4FF),
    on_primary: color!(0x152F64),
    primary_container: color!(0x2C4379),
    on_primary_container: color!(0xDDE1FF),

    secondary: color!(0xC4C5DD),
    secondary_container: color!(0x434559),
    on_secondary_container: color!(0xE0E1F9),

    tertiary: color!(0xE5BAD8),
    on_tertiary: color!(0x44263F),

    error: color!(0xFFB4AB),
    on_error: color!(0x690005),
    error_container: color!(0x93000A),
    on_error_container: color!(0xFFDAD6),

    success: color!(0x8ADA95),
    on_success: color!(0x00390C),
    warning: color!(0xF5BE4B),
    warning_container: color!(0x5A4300),
    on_warning_container: color!(0xFFDFA3),

    background: color!(0x131318),

    surface: color!(0x131318),
    surface_bright: color!(0x39383F),
    inverse_surface: color!(0xE4E1E9),
    inverse_on_surface: color!(0x303036),
    inverse_primary: color!(0x465AAA),
    surface_container_lowest: color!(0x0E0E13),
    surface_container_low: color!(0x1B1B21),
    surface_container: color!(0x201F26),
    surface_container_high: color!(0x2A2930),
    surface_container_highest: color!(0x35343B),
    on_surface: color!(0xE4E1E9),
    on_surface_variant: color!(0xC7C5D0),

    outline: color!(0x918F99),
    outline_variant: color!(0x47464F),

    scrim: color!(0x000000),
    shadow: color!(0x000000),
};

/// Teal seed palette, generated to the same role structure as the indigo base.
pub const TEAL_LIGHT: Palette = Palette {
    primary: color!(0x006A6A),
    on_primary: color!(0xFFFFFF),
    primary_container: color!(0x9CF1EF),
    on_primary_container: color!(0x002020),

    secondary: color!(0x4A6363),
    secondary_container: color!(0xCCE8E7),
    on_secondary_container: color!(0x051F1F),

    tertiary: color!(0x4B607C),
    on_tertiary: color!(0xFFFFFF),

    error: LIGHT.error,
    on_error: LIGHT.on_error,
    error_container: LIGHT.error_container,
    on_error_container: LIGHT.on_error_container,

    success: LIGHT.success,
    on_success: LIGHT.on_success,
    warning: LIGHT.warning,
    warning_container: LIGHT.warning_container,
    on_warning_container: LIGHT.on_warning_container,

    background: color!(0xF7FAF9),

    surface: color!(0xF7FAF9),
    surface_bright: color!(0xF7FAF9),
    inverse_surface: color!(0x2E3131),
    inverse_on_surface: color!(0xEFF2F1),
    inverse_primary: color!(0x80D5D3),
    surface_container_lowest: color!(0xFFFFFF),
    surface_container_low: color!(0xF0F4F3),
    surface_container: color!(0xEAEEED),
    surface_container_high: color!(0xE4E8E7),
    surface_container_highest: color!(0xDEE2E1),
    on_surface: LIGHT.on_surface,
    on_surface_variant: color!(0x3F4948),

    outline: color!(0x6F7978),
    outline_variant: color!(0xBFC9C8),

    scrim: color!(0x000000),
    shadow: color!(0x000000),
};

pub const TEAL_DARK: Palette = Palette {
    primary: color!(0x80D5D3),
    on_primary: color!(0x003737),
    primary_container: color!(0x004F4F),
    on_primary_container: color!(0x9CF1EF),

    secondary: color!(0xB0CCCB),
    secondary_container: color!(0x324B4A),
    on_secondary_container: color!(0xCCE8E7),

    tertiary: color!(0xB3C8E9),
    on_tertiary: color!(0x1C314C),

    error: DARK.error,
    on_error: DARK.on_error,
    error_container: DARK.error_container,
    on_error_container: DARK.on_error_container,

    success: DARK.success,
    on_success: DARK.on_success,
    warning: DARK.warning,
    warning_container: DARK.warning_container,
    on_warning_container: DARK.on_warning_container,

    background: color!(0x111414),

    surface: color!(0x111414),
    surface_bright: color!(0x363A39),
    inverse_surface: color!(0xDEE2E1),
    inverse_on_surface: color!(0x2E3131),
    inverse_primary: color!(0x006A6A),
    surface_container_lowest: color!(0x0C0F0F),
    surface_container_low: color!(0x191C1C),
    surface_container: color!(0x1D2020),
    surface_container_high: color!(0x272B2A),
    surface_container_highest: color!(0x323535),
    on_surface: DARK.on_surface,
    on_surface_variant: color!(0xBFC9C8),

    outline: color!(0x899392),
    outline_variant: color!(0x3F4948),

    scrim: color!(0x000000),
    shadow: color!(0x000000),
};

/// Rose seed palette for users who want a warmer accent family.
pub const ROSE_LIGHT: Palette = Palette {
    primary: color!(0x984061),
    on_primary: color!(0xFFFFFF),
    primary_container: color!(0xFFD9E3),
    on_primary_container: color!(0x3E001D),

    secondary: color!(0x74565F),
    secondary_container: color!(0xFFD9E3),
    on_secondary_container: color!(0x2B151D),

    tertiary: color!(0x7D5635),
    on_tertiary: color!(0xFFFFFF),

    error: LIGHT.error,
    on_error: LIGHT.on_error,
    error_container: LIGHT.error_container,
    on_error_container: LIGHT.on_error_container,

    success: LIGHT.success,
    on_success: LIGHT.on_success,
    warning: LIGHT.warning,
    warning_container: LIGHT.warning_container,
    on_warning_container: LIGHT.on_warning_container,

    background: color!(0xFFFBFF),

    surface: color!(0xFFFBFF),
    surface_bright: color!(0xFFFBFF),
    inverse_surface: color!(0x352F33),
    inverse_on_surface: color!(0xFCEFF4),
    inverse_primary: color!(0xFFB1C8),
    surface_container_lowest: color!(0xFFFFFF),
    surface_container_low: color!(0xFCF0F4),
    surface_container: color!(0xF6EAEE),
    surface_container_high: color!(0xF0E4E8),
    surface_container_highest: color!(0xEADFE3),
    on_surface: LIGHT.on_surface,
    on_surface_variant: color!(0x514349),

    outline: color!(0x82737A),
    outline_variant: color!(0xD4C2C8),

    scrim: color!(0x000000),
    shadow: color!(0x000000),
};

pub const ROSE_DARK: Palette = Palette {
    primary: color!(0xFFB1C8),
    on_primary: color!(0x5E1134),
    primary_container: color!(0x7B2949),
    on_primary_container: color!(0xFFD9E3),

    secondary: color!(0xE3BDC8),
    secondary_container: color!(0x5A3F49),
    on_secondary_container: color!(0xFFD9E3),

    tertiary: color!(0xF2BD91),
    on_tertiary: color!(0x49290D),

    error: DARK.error,
    on_error: DARK.on_error,
    error_container: DARK.error_container,
    on_error_container: DARK.on_error_container,

    success: DARK.success,
    on_success: DARK.on_success,
    warning: DARK.warning,
    warning_container: DARK.warning_container,
    on_warning_container: DARK.on_warning_container,

    background: color!(0x171216),

    surface: color!(0x171216),
    surface_bright: color!(0x3F373C),
    inverse_surface: color!(0xEADFE3),
    inverse_on_surface: color!(0x352F33),
    inverse_primary: color!(0x984061),
    surface_container_lowest: color!(0x120D10),
    surface_container_low: color!(0x211A1E),
    surface_container: color!(0x261E23),
    surface_container_high: color!(0x30282D),
    surface_container_highest: color!(0x3B3337),
    on_surface: DARK.on_surface,
    on_surface_variant: color!(0xD4C2C8),

    outline: color!(0x9C8D93),
    outline_variant: color!(0x514349),

    scrim: color!(0x000000),
    shadow: color!(0x000000),
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeSeed {
    #[default]
    Indigo,
    Teal,
    Rose,
}

impl ThemeSeed {
    pub const fn label_key(self) -> &'static str {
        match self {
            Self::Indigo => "theme_seed_indigo",
            Self::Teal => "theme_seed_teal",
            Self::Rose => "theme_seed_rose",
        }
    }

    pub const fn code(self) -> &'static str {
        match self {
            Self::Indigo => "indigo",
            Self::Teal => "teal",
            Self::Rose => "rose",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "indigo" => Some(Self::Indigo),
            "teal" => Some(Self::Teal),
            "rose" => Some(Self::Rose),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct RuntimeTheme {
    seed: ThemeSeed,
    /// Read only via `runtime_dark`, whose sole consumer is macOS-gated.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    dark_mode: bool,
}

static RUNTIME_THEME: RwLock<RuntimeTheme> = RwLock::new(RuntimeTheme {
    seed: ThemeSeed::Indigo,
    dark_mode: false,
});

pub fn set_runtime_theme(seed: ThemeSeed, dark_mode: bool) {
    if let Ok(mut runtime) = RUNTIME_THEME.write() {
        *runtime = RuntimeTheme { seed, dark_mode };
    }
}

fn runtime_theme() -> RuntimeTheme {
    RUNTIME_THEME.read().map_or(
        RuntimeTheme {
            seed: ThemeSeed::Indigo,
            dark_mode: false,
        },
        |runtime| *runtime,
    )
}

/// Current runtime dark-mode flag (kept in sync by `sync_runtime_theme`). Lets
/// view code pick light/dark assets without an `&iced::Theme` in hand.
///
/// Only consumed by the macOS About-view app-icon selection, so it is dead on
/// every other target.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn runtime_dark() -> bool {
    runtime_theme().dark_mode
}

pub fn palette_for(seed: ThemeSeed, dark_mode: bool) -> Palette {
    match (seed, dark_mode) {
        (ThemeSeed::Indigo, false) => LIGHT,
        (ThemeSeed::Indigo, true) => DARK,
        (ThemeSeed::Teal, false) => TEAL_LIGHT,
        (ThemeSeed::Teal, true) => TEAL_DARK,
        (ThemeSeed::Rose, false) => ROSE_LIGHT,
        (ThemeSeed::Rose, true) => ROSE_DARK,
    }
}

pub fn active_palette_for(t: &iced::Theme) -> Palette {
    let runtime = runtime_theme();
    palette_for(runtime.seed, is_dark(t))
}

pub fn iced_palette(seed: ThemeSeed, dark_mode: bool) -> iced::theme::Palette {
    let p = palette_for(seed, dark_mode);
    iced::theme::Palette {
        background: p.background,
        text: p.on_surface,
        primary: p.primary,
        success: p.success,
        warning: p.warning,
        danger: p.error,
    }
}

/// Probe `iced::Theme` for the active mode. We don't store a flag on
/// the theme directly, so the heuristic looks at `palette().background`
/// — light backgrounds have a high red channel (M3 surface tones land
/// at `0xFB+` on light), dark ones at `0x13+`. Centralised so both
/// `theme::tooltip_style` (and the rest of this module) and the GUI
/// call sites agree on a single source of truth.
pub fn is_dark(t: &iced::Theme) -> bool {
    t.palette().background.r < 0.5
}

/// Overlay a color with alpha — used for M3 state layers.
pub const fn with_alpha(c: Color, a: f32) -> Color {
    Color { a, ..c }
}

/// Blend `overlay` over `base` with the given alpha. Used to flatten
/// an M3 state-layer (translucent on_X color) into a single opaque
/// background tint, since `iced::widget::button::Style::background`
/// only accepts one color/gradient at a time and can't stack a
/// semi-transparent layer over the tonal fill.
pub fn mix_color(base: Color, overlay: Color, alpha: f32) -> Color {
    let inv = 1.0 - alpha;
    Color {
        r: base.r * inv + overlay.r * alpha,
        g: base.g * inv + overlay.g * alpha,
        b: base.b * inv + overlay.b * alpha,
        a: 1.0,
    }
}

/// M3 state-layer alphas.
pub mod state {
    pub const HOVER: f32 = 0.08;
    pub const PRESSED: f32 = 0.12;
}

/// M3 state-layer alpha for an `iced::widget::button::Status` — `0.0`
/// when idle, `HOVER` on hover, `PRESSED` while pressed. Centralises
/// the inline `match status { Hovered => HOVER, Pressed => PRESSED,
/// _ => 0.0 }` pattern that was scattered across the GUI's button
/// style closures.
pub fn state_alpha(status: iced::widget::button::Status) -> f32 {
    use iced::widget::button::Status;
    match status {
        Status::Hovered => state::HOVER,
        Status::Pressed => state::PRESSED,
        _ => 0.0,
    }
}

/// Combine [`state_alpha`] with [`with_alpha`] to produce the M3
/// state-layer tint for a button's background overlay. Returns
/// `None` when the button is idle so callers can use
/// `Option<Background>` directly.
///
/// `layer_color` is the M3 "on-X" color of the surface the button
/// sits on (usually `palette.on_surface`).
pub fn state_layer_bg(status: iced::widget::button::Status, layer_color: Color) -> Option<Color> {
    let alpha = state_alpha(status);
    if alpha == 0.0 {
        None
    } else {
        Some(with_alpha(layer_color, alpha))
    }
}

/// Plain tooltip: inverse surface, extra-small shape, no elevation.
pub fn tooltip_style(t: &iced::Theme, _radius: f32) -> iced::widget::container::Style {
    let p = active_palette_for(t);
    iced::widget::container::Style {
        background: Some(p.inverse_surface.into()),
        text_color: Some(p.inverse_on_surface),
        border: iced::Border {
            radius: shape::XS.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Small expressive square buttons morph to the small shape while pressed.
pub fn button_radius(status: iced::widget::button::Status) -> f32 {
    if matches!(status, iced::widget::button::Status::Pressed) {
        shape::SM
    } else {
        shape::MD
    }
}

/// M3 shape scale (corner radius in px).
///
/// Keep this module as the plain M3 corner scale; components choose the
/// appropriate rung at their call sites.
pub mod shape {
    pub const XS: f32 = 4.0;
    pub const SM: f32 = 8.0;
    pub const MD: f32 = 12.0;
    pub const LG: f32 = 16.0;
    pub const XL: f32 = 28.0;
    pub const FULL: f32 = 9999.0;
}

/// Broadest of the three bundled families: the only one carrying Hangul, and it
/// covers Latin and Cyrillic as well. Used for every language whose script it
/// renders idiomatically — the Japanese and Chinese families exist for their
/// regional CJK forms, not for Latin or Cyrillic coverage.
const DEFAULT_FONT_FAMILY: &str = "Noto Sans KR";

/// UI font family for this run, bound by [`set_font_family`].
static FONT_FAMILY: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();
/// Whether this run should use iced's OS-default font instead of bundled Noto.
static USE_SYSTEM_FONT: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// The active UI font family — always one the app itself registers.
///
/// This has to name a family `fn main` actually registers
/// (`Noto Sans JP` / `Noto Sans KR` / `Noto Sans SC`). Naming a family it does
/// not — `Noto Sans CJK KR`, the separate Source Han release — resolved only on
/// machines that happened to have that family installed. Everywhere else
/// cosmic-text found no match and fell through to fallback, where a *weighted*
/// request ([`emphasis`]) is resolved by weight proximity across the entire
/// font database: it lands on whatever unrelated face on that machine happens
/// to own a 500-weight, which for some users was an icon font, so every
/// medium-weight Latin run rendered as tofu and arrows while unweighted text
/// beside it looked fine.
pub fn font_family() -> &'static str {
    FONT_FAMILY.get().copied().unwrap_or(DEFAULT_FONT_FAMILY)
}

/// Bind the UI font family for this run.
///
/// Called once from `fn main` before the iced settings are built. A language
/// change afterwards takes effect on the next launch: iced fixes `default_font`
/// at startup, and letting [`emphasis`] move while it stayed put would split
/// the UI across two faces mid-session.
pub fn set_font_family(family: &'static str) {
    let _ = FONT_FAMILY.set(family);
}

/// Bind the font source for this run. Repeated calls are intentionally ignored:
/// iced fixes its default font at startup, and tests may initialize `App` more
/// than once in the same process.
pub fn set_use_system_font(use_system_font: bool) {
    let _ = USE_SYSTEM_FONT.set(use_system_font);
}

/// Font used by iced settings and by weighted text variants.
pub fn default_font() -> iced::Font {
    if USE_SYSTEM_FONT.get().copied().unwrap_or(false) {
        iced::Font::DEFAULT
    } else {
        iced::Font::with_name(font_family())
    }
}

/// Bundled fixed-width face, for content that reads as data rather than prose:
/// file paths and country codes. Latin only — a CJK monospace would cost
/// megabytes, and the mockup's JetBrains Mono has no CJK either, so localized
/// copy must not ask for this.
pub fn mono_font() -> iced::Font {
    iced::Font::with_name("Noto Sans Mono")
}

/// The bundled face that renders `language_code` idiomatically.
///
/// Han characters shared between the three locales have different regional
/// forms, and a face only carries one set — so a Japanese UI rendered through
/// the Korean face shows Korean kanji shapes next to kana pulled from the
/// Japanese face by script fallback. Pick the matching face up front and let
/// fallback handle only what it genuinely lacks.
pub fn font_family_for_language(language_code: &str) -> &'static str {
    match language_code {
        "ja" => "Noto Sans JP",
        "zh" => "Noto Sans SC",
        _ => DEFAULT_FONT_FAMILY,
    }
}

/// M3 Expressive type emphasis.
///
/// Expressive carries hierarchy on weight contrast as much as on size. Each
/// bundled family provides regular, medium and bold faces so these requests
/// resolve within the active family.
pub mod emphasis {
    use iced::Font;
    use iced::font::Weight;

    fn weighted(weight: Weight) -> Font {
        Font {
            weight,
            ..super::default_font()
        }
    }

    /// Titles, active nav labels, key data values.
    pub fn medium() -> Font {
        weighted(Weight::Medium)
    }

    /// Headlines and the label on a screen's single primary action.
    pub fn bold() -> Font {
        weighted(Weight::Bold)
    }
}

/// M3 type scale (font size in px).
pub mod text_size {
    pub const TITLE_LARGE: f32 = 22.0;
    pub const TITLE_MEDIUM: f32 = 16.0;
    pub const BODY_MEDIUM: f32 = 14.0;
    pub const BODY_SMALL: f32 = 12.0;
    pub const LABEL_SMALL: f32 = 11.0;
}

/// Dialog width scale and shared inset. Defined in `layout_constraints` so the
/// locale guards measure the same budgets; re-exported here because dialogs
/// reach for them alongside the rest of the shape and type scales.
pub(crate) use crate::layout_constraints::{
    DIALOG_H_PADDING, DIALOG_WIDTH_LG, DIALOG_WIDTH_MD, DIALOG_WIDTH_SM,
};

/// Which palette surface container the card fills with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceLevel {
    /// `surface_container` — default card surface.
    Default,
    /// `surface_bright` — a consistently bright region in either theme.
    Brightest,
}

impl SurfaceLevel {
    fn bg(self, p: &Palette) -> iced::Color {
        match self {
            Self::Default => p.surface_container,
            Self::Brightest => p.surface_bright,
        }
    }
}

/// Shared M3 outlined card/panel style at elevation zero.
/// Floating surfaces use their own styles and shadows.
pub fn surface_card_style(
    t: &iced::Theme,
    level: SurfaceLevel,
    radius: f32,
) -> iced::widget::container::Style {
    use iced::widget::container;
    let p = active_palette_for(t);
    container::Style {
        background: Some(level.bg(&p).into()),
        border: iced::Border {
            color: p.outline_variant,
            width: 1.0,
            radius: radius.into(),
        },
        ..Default::default()
    }
}

/// Shadow for a full-height drawer that overlaps content sideways.
///
/// [`elevation`] models M3's downward key light, which suits a card or a
/// dialog floating above what is under it. A rail that spans the window
/// overlaps its neighbour horizontally, so a downward offset casts onto the
/// status bar it sits against and casts nothing onto the content it actually
/// covers. This offsets along the leading edge instead, with no vertical
/// component.
/// `openness` is how far the drawer is open, 0.0 closed to 1.0 open. The
/// shadow rides it so it fades out with the width instead of holding full
/// strength through the collapse and then vanishing in one frame.
/// Width of the drawer's cast shadow when fully open.
pub const DRAWER_EDGE_SHADOW_WIDTH: f32 = 14.0;

/// The hover drawer casts along its trailing edge only.
///
/// A container `Shadow` cannot do this: its blur spreads in every direction and
/// only the horizontal offset trims one side, so a full-height panel gets a
/// halo above and below where there is no offset to cancel it. Those edges meet
/// the title bar and the status bar, and the halo reads as a smudge on both.
/// A gradient strip laid beside the panel casts to the right and nowhere else.
pub fn drawer_edge_gradient(dark_mode: bool, openness: f32) -> iced::Background {
    use iced::{Color, gradient};
    let t = openness.clamp(0.0, 1.0);
    let alpha = if dark_mode { 0.24 } else { 0.15 } * t;
    // Angle 0 runs bottom-to-top, so a quarter turn casts left-to-right.
    iced::Background::Gradient(iced::Gradient::Linear(
        gradient::Linear::new(std::f32::consts::FRAC_PI_2)
            .add_stop(0.0, Color::from_rgba(0.0, 0.0, 0.0, alpha))
            .add_stop(1.0, Color::TRANSPARENT),
    ))
}

/// M3 elevation → `iced::Shadow`. `0` = none, `5` = modal-dialog.
pub fn elevation(level: u8, dark_mode: bool) -> iced::Shadow {
    use iced::{Color, Shadow, Vector};
    // Dark M3 conveys elevation mainly through tonal surface containers, so
    // shadows stay subtle — a gentle ramp by level (0.20..0.36) rather than a
    // flat, heavy 0.6 black. Light theme keeps one soft key shadow.
    let shadow_color = if dark_mode {
        let alpha = 0.20 + 0.04 * f32::from(level.min(5).saturating_sub(1));
        Color::from_rgba(0.0, 0.0, 0.0, alpha)
    } else {
        Color::from_rgba(0.0, 0.0, 0.0, 0.15)
    };
    match level {
        0 => Shadow {
            color: Color::TRANSPARENT,
            offset: Vector::ZERO,
            blur_radius: 0.0,
        },
        1 => Shadow {
            color: shadow_color,
            offset: Vector::new(0.0, 1.0),
            blur_radius: 3.0,
        },
        2 => Shadow {
            color: shadow_color,
            offset: Vector::new(0.0, 2.0),
            blur_radius: 6.0,
        },
        3 => Shadow {
            color: shadow_color,
            offset: Vector::new(0.0, 4.0),
            blur_radius: 8.0,
        },
        4 => Shadow {
            color: shadow_color,
            offset: Vector::new(0.0, 6.0),
            blur_radius: 10.0,
        },
        _ => Shadow {
            color: shadow_color,
            offset: Vector::new(0.0, 8.0),
            blur_radius: 12.0,
        },
    }
}

#[cfg(test)]
mod font_family_tests {
    use iced::advanced::graphics::text::cosmic_text::fontdb;
    use iced::font::Weight;

    /// Every family the UI can ask for must be one the app itself registers.
    ///
    /// The database here holds *only* the bundled faces — no system fonts — so
    /// this fails when a family name or required weight drifts from the faces
    /// the application registers.
    #[test]
    fn every_language_and_weight_resolves_against_the_bundled_faces_alone() {
        let mut db = fontdb::Database::new();
        for bytes in [
            include_bytes!("../fonts/noto/NotoSansKR-Regular.subset.otf") as &[u8],
            include_bytes!("../fonts/noto/NotoSansKR-Medium.subset.otf") as &[u8],
            include_bytes!("../fonts/noto/NotoSansKR-Bold.subset.otf") as &[u8],
            include_bytes!("../fonts/noto/NotoSansJP-Regular.subset.otf") as &[u8],
            include_bytes!("../fonts/noto/NotoSansJP-Medium.subset.otf") as &[u8],
            include_bytes!("../fonts/noto/NotoSansJP-Bold.subset.otf") as &[u8],
            include_bytes!("../fonts/noto/NotoSansSC-Regular.subset.otf") as &[u8],
            include_bytes!("../fonts/noto/NotoSansSC-Medium.subset.otf") as &[u8],
            include_bytes!("../fonts/noto/NotoSansSC-Bold.subset.otf") as &[u8],
        ] {
            db.load_font_data(bytes.to_vec());
        }
        assert!(!db.is_empty(), "the bundle must carry at least one face");

        for code in ["en", "ko", "zh", "ru", "ja"] {
            let family = super::font_family_for_language(code);
            for (requested, requested_db_weight) in [
                (Weight::Normal, fontdb::Weight::NORMAL),
                (Weight::Medium, fontdb::Weight::MEDIUM),
                (Weight::Bold, fontdb::Weight::BOLD),
            ] {
                let query = fontdb::Query {
                    families: &[fontdb::Family::Name(family)],
                    weight: requested_db_weight,
                    ..Default::default()
                };
                let face_id = db.query(&query).unwrap_or_else(|| {
                    panic!(
                        "{code} asks for {family:?} at {requested:?}, which the bundled fonts do not declare; declared families: {:?}",
                        db.faces()
                            .flat_map(|face| face.families.iter().map(|(name, _)| name.clone()))
                            .collect::<Vec<_>>()
                    )
                });
                let matched_weight = db.face(face_id).expect("matched face must exist").weight;
                assert_eq!(
                    matched_weight, requested_db_weight,
                    "{code} asks for {family:?} at {requested:?}, but the bundled family matched {matched_weight:?}"
                );
            }
        }
    }

    /// The unbound default has to resolve too — it is what every text widget
    /// gets before `fn main` binds a language-specific face.
    #[test]
    fn the_default_family_is_bundled() {
        assert_eq!(super::font_family(), super::DEFAULT_FONT_FAMILY);
        assert_eq!(
            super::font_family_for_language("ko"),
            super::DEFAULT_FONT_FAMILY
        );
    }

    /// Korean is the only script just one bundled face covers, so it must keep
    /// the face that has it.
    #[test]
    fn hangul_locales_keep_the_korean_face() {
        assert_eq!(super::font_family_for_language("ko"), "Noto Sans KR");
        assert_eq!(super::font_family_for_language("ja"), "Noto Sans JP");
        assert_eq!(super::font_family_for_language("zh"), "Noto Sans SC");
    }
}
