//! About panel: app icon, version, project links, and license.

use crate::*;
use iced::widget::{self, button, column, container, row, text};
use iced::{Element, Length, Theme};
use theme::mix_color;

const GITHUB_URL: &str = "https://github.com/miner7222/LTBox";
const WIKI_URL: &str = "https://miner7222.github.io/ltbox";
const ISSUES_URL: &str = "https://github.com/miner7222/LTBox/issues";

impl App {
    pub(crate) fn view_about(&self) -> Element<'_, Message> {
        let app_icon = about_app_icon(88.0);
        let title = text("LTBox").size(theme::text_size::TITLE_LARGE);
        // No width cap: the text sizes to its content so it stays on one line
        // when the content area has room (the column centers it). A fixed
        // max_width forced a needless second line even on a wide window.
        let description = text(self.t("about_description").to_string())
            .size(12.0)
            .style(muted_style)
            .center();
        // Append the build commit (set by build.rs) so bug reports can pin the
        // exact build; omitted for a tarball build with no git checkout.
        let version_label = match option_env!("LTBOX_GIT_HASH") {
            Some(hash) if !hash.is_empty() => {
                format!("v{} · {hash}", env!("CARGO_PKG_VERSION"))
            }
            _ => format!("v{}", env!("CARGO_PKG_VERSION")),
        };
        let version = text(version_label)
            .size(theme::text_size::BODY_SMALL)
            .style(muted_style);

        let links = row![
            about_link_button(
                icon::about_github(),
                GITHUB_URL,
                self.t("about_github").to_string(),
            ),
            about_link_button(
                icon::about_issue(),
                ISSUES_URL,
                self.t("about_issue").to_string(),
            ),
            about_link_button(
                icon::about_wiki(),
                WIKI_URL,
                self.t("about_wiki").to_string(),
            ),
        ]
        .spacing(12.0)
        .align_y(iced::Alignment::Center);

        // Fine-print footer. The dialog behind the link opens with LTBox's own
        // license, so naming it out here as well would only repeat it.
        let licenses_link = iced::widget::rich_text([iced::widget::span(
            self.t("about_licenses_link").to_string(),
        )
        .size(12.0)
        .color(self.pal().primary)
        .underline(true)
        .link(())])
        .on_link_click(|()| Message::AboutLicensesOpen);

        let col = column![app_icon, title, description, version, links, licenses_link]
            .spacing(14.0)
            .align_x(iced::Alignment::Center);

        container(col)
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into()
    }
}

// macOS About-panel icon handles. Built once (stable image `Id`): iced's
// `Handle::from_bytes` mints a fresh unique id on every call, so rebuilding the
// handle each render re-uploads the texture and makes the icon flicker —
// caching matches the device-portrait handles in `widgets.rs`.
#[cfg(target_os = "macos")]
static ABOUT_ICON_LIGHT: std::sync::LazyLock<iced::widget::image::Handle> =
    std::sync::LazyLock::new(|| {
        iced::widget::image::Handle::from_bytes(
            include_bytes!("../../assets/icon_macos.png").as_slice(),
        )
    });
#[cfg(target_os = "macos")]
static ABOUT_ICON_DARK: std::sync::LazyLock<iced::widget::image::Handle> =
    std::sync::LazyLock::new(|| {
        iced::widget::image::Handle::from_bytes(
            include_bytes!("../../assets/icon_macos_dark.png").as_slice(),
        )
    });

/// App icon for the About panel. macOS uses the rounded squircle PNG —
/// light or dark variant to match LTBox's current theme; other platforms use
/// the scalable flat SVG logo so it stays crisp at any size.
fn about_app_icon(size: f32) -> Element<'static, Message> {
    #[cfg(target_os = "macos")]
    {
        // Match the macOS Liquid Glass icon's appearance to the active theme.
        // Clone a cached handle (cheap, ref-counted) so the id stays stable.
        let handle = if theme::runtime_dark() {
            ABOUT_ICON_DARK.clone()
        } else {
            ABOUT_ICON_LIGHT.clone()
        };
        iced::widget::image(handle).width(size).height(size).into()
    }
    #[cfg(not(target_os = "macos"))]
    {
        svg_icon(include_bytes!("../../assets/icon_source.svg"), size)
    }
}

/// Round icon-only button that opens `url` in the host browser, with a tooltip.
/// Styled like the Settings inline icon buttons (tonal `secondary_container`
/// base + pre-composited M3 state layer).
fn about_link_button(
    glyph: iced::widget::Text<'static, Theme, iced::Renderer>,
    url: &'static str,
    tip: String,
) -> Element<'static, Message> {
    let side = Length::Fixed(40.0);
    let btn = button(
        container(lucide_icon(glyph, 20.0, |t: &Theme| {
            pal_of(t).on_secondary_container
        }))
        .width(side)
        .height(side)
        .center_x(side)
        .center_y(side),
    )
    .on_press(Message::OpenUrl(url))
    .padding(0)
    .style(|t: &Theme, status| {
        let p = pal_of(t);
        let base = p.secondary_container;
        let bg = match status {
            button::Status::Hovered => {
                mix_color(base, p.on_secondary_container, theme::state::HOVER)
            }
            button::Status::Pressed => {
                mix_color(base, p.on_secondary_container, theme::state::PRESSED)
            }
            _ => base,
        };
        button::Style {
            background: Some(bg.into()),
            border: iced::Border {
                radius: theme::shape::SM.into(),
                ..Default::default()
            },
            ..Default::default()
        }
    });
    widget::tooltip(
        btn,
        container(text(tip).size(11))
            .padding([6, 10])
            .style(|t: &Theme| theme::tooltip_style(t, theme::shape::XS)),
        widget::tooltip::Position::Top,
    )
    .into()
}
