//! Modal popup views (device info, OTA, ARB index, country, region, rescue region, log). Extracted from `main.rs`.

use crate::focus_button::{self as button, button};
use crate::*;
use iced::widget::{self, Space, column, container, row, scrollable, text, text_input};
use iced::{Element, Length, Theme};
use theme::with_alpha;

const COUNTRY_LIST_HEIGHT: f32 = 300.0;
const COUNTRY_ROW_HEIGHT: f32 = 44.0;
const COUNTRY_FLAG_WIDTH: f32 = 20.0;
const COUNTRY_FLAG_HEIGHT: f32 = 14.0;
const COUNTRY_FLAG_RADIUS: f32 = 4.0;

fn popup_sections<'a>(
    header: impl Into<Element<'a, Message>>,
    body: impl Into<Element<'a, Message>>,
    footer: impl Into<Element<'a, Message>>,
    width: f32,
    scrolling: bool,
) -> Element<'a, Message> {
    dialog_sections(header.into(), body.into(), footer.into(), width, scrolling)
}

fn country_matches_search(entry: &CountryEntry, query: &str) -> bool {
    let query = query.trim().to_ascii_lowercase();
    query.is_empty()
        || entry.code.to_ascii_lowercase().contains(&query)
        || entry.name.to_ascii_lowercase().contains(&query)
}

/// Country-list row with a bundled, font-independent flag.
fn country_popup_row<'a>(
    leading: Option<Element<'a, Message>>,
    code: &'static str,
    name: &'static str,
    selected: bool,
    disabled: bool,
) -> Element<'a, Message> {
    let foreground =
        move |t: &Theme| with_alpha(pal_of(t).on_surface, if disabled { 0.38 } else { 1.0 });
    let code_foreground = move |t: &Theme| {
        with_alpha(
            if selected {
                pal_of(t).on_surface
            } else {
                pal_of(t).on_surface_variant
            },
            if disabled { 0.38 } else { 1.0 },
        )
    };
    let mut contents = row![].spacing(11).align_y(iced::Alignment::Center);
    if let Some(leading) = leading {
        contents = contents.push(
            container(leading)
                .width(Length::Fixed(COUNTRY_FLAG_WIDTH))
                .height(Length::Fixed(COUNTRY_FLAG_HEIGHT))
                .clip(true)
                .style(|_: &Theme| container::Style {
                    border: iced::Border {
                        radius: COUNTRY_FLAG_RADIUS.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
        );
    }
    contents = contents
        .push(
            text(code)
                .font(theme::mono_font())
                .size(theme::text_size::BODY_SMALL)
                .width(Length::Fixed(26.0))
                .style(move |t: &Theme| iced::widget::text::Style {
                    color: Some(code_foreground(t)),
                }),
        )
        .push(
            text(name)
                .size(theme::text_size::BODY_MEDIUM)
                .width(Length::Fill)
                .wrapping(iced::widget::text::Wrapping::None)
                .style(move |t: &Theme| iced::widget::text::Style {
                    color: Some(foreground(t)),
                }),
        );
    if selected {
        contents = contents.push(lucide_icon(icon::mark_check(), 16.0, |t: &Theme| {
            pal_of(t).primary
        }));
    }

    // A button lays its content out at its natural height and pins it to the
    // top, so the row has to claim the button's fixed height before centring
    // inside it.
    let mut row_button = button(contents.width(Length::Fill).height(Length::Fill))
        .height(Length::Fixed(COUNTRY_ROW_HEIGHT))
        .width(Length::Fill)
        .padding([0, 20])
        .style(move |t: &Theme, status| {
            let p = pal_of(t);
            let state_alpha = if disabled {
                0.0
            } else {
                theme::state_alpha(status)
            };
            button::Style {
                background: if selected {
                    Some(p.secondary_container.into())
                } else if state_alpha > 0.0 {
                    Some(with_alpha(p.on_surface, state_alpha).into())
                } else {
                    None
                },
                text_color: foreground(t),
                ..Default::default()
            }
        });
    if !disabled {
        row_button = row_button.on_press(Message::SelectCountry(code.to_string()));
    }
    row_button.into()
}

impl App {
    pub(crate) fn software_fix_confirm_dialog(&self) -> Element<'_, Message> {
        m3_dialog(popup_sections(
            text(self.t("software_fix_confirm_title").to_string())
                .size(theme::text_size::TITLE_LARGE),
            text(self.t("software_fix_elevation_hint").to_string())
                .size(theme::text_size::BODY_MEDIUM)
                .style(muted_style),
            row![
                Space::new().width(Length::Fill),
                m3_outlined_button(self.t("btn_cancel").to_string())
                    .on_press(Message::CancelCloseSoftwareFix),
                m3_filled_button(self.t("btn_ok").to_string()).on_press_maybe(
                    self.can_close_software_fix()
                        .then_some(Message::ConfirmCloseSoftwareFix)
                ),
            ]
            .spacing(10.0)
            .align_y(iced::Alignment::Center),
            theme::DIALOG_WIDTH_SM,
            false,
        ))
    }

    /// Illustrated guide for the data-capable port on dual-USB-C tablets.
    pub(crate) fn dual_usb_help_dialog(&self) -> Element<'_, Message> {
        let model = self.dual_usb_help_model.clone();
        let marker_dot = |success: bool| {
            let icon = if success {
                icon::mark_check()
            } else {
                icon::win_close()
            };
            container(icon.size(16))
                .width(26)
                .height(26)
                .align_x(iced::Alignment::Center)
                .align_y(iced::Alignment::Center)
                .style(move |t: &Theme| {
                    let p = pal_of(t);
                    container::Style {
                        background: Some((if success { p.success } else { p.error }).into()),
                        text_color: Some(if success { p.on_success } else { p.on_error }),
                        border: iced::Border {
                            radius: theme::shape::FULL.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }
                })
        };
        let marker_caption = |label: String, muted: bool| {
            container(
                text(label)
                    .size(theme::text_size::LABEL_SMALL)
                    .wrapping(iced::widget::text::Wrapping::None),
            )
            .padding([2, 9])
            .style(move |t: &Theme| {
                let p = pal_of(t);
                container::Style {
                    background: Some(p.surface_container_low.into()),
                    text_color: Some(if muted {
                        p.on_surface_variant
                    } else {
                        p.on_surface
                    }),
                    border: iced::Border {
                        color: p.outline_variant,
                        width: 1.0,
                        radius: theme::shape::FULL.into(),
                    },
                    ..Default::default()
                }
            })
        };

        // Every dual-USB model ships the same 500x300 front-on portrait, so a
        // 300x180 box holds it edge to edge with no letterboxing and a marker
        // pinned to one side names the same physical port on all of them.
        const PORTRAIT_W: f32 = 300.0;
        const PORTRAIT_H: f32 = 180.0;
        const MARKER_H: f32 = 48.0;
        let figure_h = PORTRAIT_H + MARKER_H;

        let portrait: Element<'_, Message> = match device_portrait(&self.dual_usb_help_model) {
            DevicePortrait::Png(handle) => widget::image(handle)
                .width(Length::Fill)
                .height(Length::Fill)
                .content_fit(iced::ContentFit::Contain)
                .into(),
            DevicePortrait::Svg(handle) => widget::svg(handle)
                .width(Length::Fill)
                .height(Length::Fill)
                .content_fit(iced::ContentFit::Contain)
                .into(),
        };
        let portrait_layer = container(
            container(portrait)
                .width(Length::Fixed(PORTRAIT_W))
                .height(Length::Fixed(PORTRAIT_H)),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced::Alignment::Center)
        .align_y(iced::Alignment::Start);

        let marker = |success: bool, label: String| {
            column![marker_dot(success), marker_caption(label, !success)]
                .spacing(4)
                .align_x(iced::Alignment::Center)
        };
        // Hangs just past the portrait's lower edge, as the mockup drops its
        // bottom-port marker below the tablet outline.
        let bottom_layer = container(marker(
            true,
            self.t("dual_usb_help_bottom_label").to_string(),
        ))
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced::Alignment::Center)
        .align_y(iced::Alignment::End);
        // Centred on the portrait rather than on the taller stack, so it meets
        // the side port instead of drifting down with the caption row.
        let side_layer = container(
            container(marker(
                false,
                self.t("dual_usb_help_side_label").to_string(),
            ))
            .width(Length::Fill)
            .height(Length::Fixed(PORTRAIT_H))
            .align_x(iced::Alignment::End)
            .align_y(iced::Alignment::Center),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .align_y(iced::Alignment::Start);

        let portrait_guide = container(
            widget::stack![portrait_layer, bottom_layer, side_layer]
                // Only a marker's width of overhang past the portrait, so the
                // side dot lands on the edge it names instead of floating off
                // in the card's margin.
                .width(Length::Fixed(PORTRAIT_W + 40.0))
                .height(Length::Fixed(figure_h)),
        )
        .width(Length::Fill)
        .height(Length::Fixed(figure_h + 24.0))
        .align_x(iced::Alignment::Center)
        .align_y(iced::Alignment::Center)
        .style(|t: &Theme| {
            let p = pal_of(t);
            container::Style {
                background: Some(p.surface.into()),
                border: iced::Border {
                    color: p.outline_variant,
                    width: 1.0,
                    radius: theme::shape::LG.into(),
                },
                ..Default::default()
            }
        });

        let dont_show = m3_outlined_button(self.t("driver_dont_show_again").to_string())
            .on_press(Message::DismissDualUsbAdvisory(model.clone()));
        let close = m3_filled_button(self.t("btn_close").to_string())
            .on_press(Message::CloseDualUsbAdvisory(model));
        let actions = row![dont_show, Space::new().width(Length::Fill), close];
        let subtitle = if self.dual_usb_help_name.is_empty()
            || self
                .dual_usb_help_name
                .eq_ignore_ascii_case(&self.dual_usb_help_model)
        {
            self.dual_usb_help_model.clone()
        } else {
            format!("{} · {}", self.dual_usb_help_name, self.dual_usb_help_model)
        };
        let content = popup_sections(
            column![
                text(self.t("dual_usb_help_title").to_string()).size(theme::text_size::TITLE_LARGE),
                text(subtitle)
                    .size(theme::text_size::BODY_SMALL)
                    .style(muted_style),
            ]
            .spacing(4),
            column![
                portrait_guide,
                text(self.t("dual_usb_help_body").to_string())
                    .size(theme::text_size::BODY_MEDIUM)
                    .style(muted_style)
                    .wrapping(iced::widget::text::Wrapping::WordOrGlyph)
                    .width(Length::Fill),
            ]
            .spacing(16),
            actions,
            theme::DIALOG_WIDTH_MD,
            false,
        );

        m3_dialog(content)
    }

    /// Scrollable inventory of components bundled or linked into LTBox, plus
    /// credits for independently reimplemented interoperable formats.
    pub(crate) fn about_licenses_dialog(&self) -> Element<'_, Message> {
        let license_entry = |name: &'static str, license: &'static str| {
            row![
                text(name)
                    .size(theme::text_size::BODY_MEDIUM)
                    .font(theme::emphasis::medium())
                    .width(Length::FillPortion(2)),
                text(license)
                    .size(theme::text_size::BODY_MEDIUM)
                    .style(muted_style)
                    .wrapping(iced::widget::text::Wrapping::WordOrGlyph)
                    .width(Length::FillPortion(3)),
            ]
            .spacing(16)
            .align_y(iced::Alignment::Start)
        };

        let licenses = column![
            license_entry("LTBox", "GPL-3.0-or-later"),
            license_entry(
                "Noto Sans CJK",
                "SIL Open Font License 1.1 — © 2014-2021 Adobe",
            ),
            license_entry("Lucide", "ISC"),
            license_entry("flag-icons 7.3.2", "MIT — © Panayiotis Lipiridis"),
            text(include_str!("../../assets/flags/LICENSE")).size(theme::text_size::BODY_SMALL),
            license_entry("qdl", "BSD-3-Clause — Qualcomm"),
            license_entry("magiskboot", "GPL-3.0-or-later"),
            license_entry("kptools", "GPL-2.0-or-later"),
            license_entry("avbtool-rs", "Apache-2.0"),
            text(self.t("about_licenses_other").to_string())
                .size(theme::text_size::BODY_SMALL)
                .style(muted_style),
        ]
        .spacing(10)
        .width(Length::Fill);

        let credits = column![text("KonaBess by libxzr"), text("SKRoot by abcz316")]
            .spacing(10)
            .width(Length::Fill);

        let body = column![
            text(self.t("about_licenses_section_licenses").to_string())
                .size(theme::text_size::TITLE_MEDIUM)
                .font(theme::emphasis::bold()),
            licenses,
            widget::rule::horizontal(1),
            text(self.t("about_licenses_section_credits").to_string())
                .size(theme::text_size::TITLE_MEDIUM)
                .font(theme::emphasis::bold()),
            credits,
        ]
        .spacing(14)
        .width(Length::Fill);

        let close = m3_outlined_button(self.t("btn_close").to_string())
            .on_press(Message::AboutLicensesClose);
        let actions =
            row![Space::new().width(Length::Fill), close].align_y(iced::Alignment::Center);

        let content = popup_sections(
            text(self.t("about_licenses_title").to_string()).size(theme::text_size::TITLE_LARGE),
            scrollable(body)
                .style(m3_scrollable_style)
                .height(Length::Fixed(420.0))
                .width(Length::Fill),
            actions,
            theme::DIALOG_WIDTH_MD,
            true,
        );

        m3_dialog(content)
    }

    /// Update flow opened from the status-bar version affordance. Direct downloads get the
    /// verified self-updater; package-managed installs keep their command.
    pub(crate) fn update_dialog_view(&self) -> Element<'_, Message> {
        let (Some(source), Some(release)) =
            (self.update_dialog_source, self.update_available.as_ref())
        else {
            return container(text("")).into();
        };
        if source == ltbox_core::install_source::InstallSource::Direct {
            return self.direct_update_dialog_view(release);
        }

        let upgrade = package_upgrade_command(source);
        let title =
            text(self.t("update_dialog_title").to_string()).size(theme::text_size::TITLE_LARGE);
        let version = text(
            // Tags carry a leading `v`; the string already says "version",
            // so trim it rather than rendering "Version v3.3.0".
            self.t("update_dialog_version")
                .replace("{version}", release.tag.trim_start_matches('v')),
        )
        .size(theme::text_size::TITLE_MEDIUM);
        let body_key = if upgrade.available {
            "update_dialog_package_body"
        } else {
            "update_dialog_other_body"
        };
        let body = text(self.t(body_key).to_string())
            .size(theme::text_size::BODY_MEDIUM)
            .style(muted_style)
            .wrapping(iced::widget::text::Wrapping::WordOrGlyph)
            .width(Length::Fill);

        let command_area: Element<'_, Message> = if upgrade.available {
            let command = iced::widget::text_input("", upgrade.command)
                // Keep the field selectable without allowing the displayed
                // package-manager command to be changed.
                .on_input(|_| Message::Noop)
                .padding([8, 12])
                .line_height(iced::widget::text::LineHeight::Absolute(24.0.into()))
                .size(theme::text_size::BODY_MEDIUM)
                .style(m3_text_input_style);
            let copy = m3_text_button(self.t("update_dialog_copy").to_string())
                .on_press(Message::CopyToClipboard(upgrade.command.to_string()));
            column![
                dialog_field_label(self.t("update_dialog_command_label").to_string()),
                row![command, copy]
                    .spacing(8)
                    .align_y(iced::Alignment::Center),
            ]
            .spacing(6)
            .into()
        } else {
            column![].into()
        };

        let close = m3_outlined_button(self.t("btn_close").to_string())
            .on_press(Message::UpdateDialogClose);
        let release_page = m3_filled_button(self.t("update_dialog_release_page").to_string())
            .on_press(Message::OpenUpdateReleasePage);
        let actions = row![Space::new().width(Length::Fill), close, release_page]
            .spacing(8)
            .align_y(iced::Alignment::Center);

        let content = popup_sections(
            title,
            column![version, body, command_area].spacing(14),
            actions,
            theme::DIALOG_WIDTH_MD,
            false,
        );

        m3_dialog(content)
    }

    fn direct_update_dialog_view(
        &self,
        release: &ltbox_core::github::StableRelease,
    ) -> Element<'_, Message> {
        let title =
            text(self.t("update_dialog_title").to_string()).size(theme::text_size::TITLE_LARGE);
        let version = text(
            self.t("update_dialog_version")
                .replace("{version}", release.tag.trim_start_matches('v')),
        )
        .size(theme::text_size::TITLE_MEDIUM);

        let state_body: Element<'_, Message> = match &self.operation.direct_update {
            DirectUpdateState::Ready => Space::new().into(),
            DirectUpdateState::Updating => row![
                material_circular_progress(MaterialProgressSize::Standard),
                text(self.t("update_dialog_downloading").to_string())
                    .size(theme::text_size::BODY_MEDIUM)
                    .style(muted_style),
            ]
            .spacing(14)
            .align_y(iced::Alignment::Center)
            .into(),
            DirectUpdateState::Failed(failure) => {
                let reason_key = match failure.kind {
                    SelfUpdateFailureKind::NoMatchingBuild => "update_dialog_error_no_build",
                    SelfUpdateFailureKind::InstallLocation => {
                        "update_dialog_error_install_location"
                    }
                    SelfUpdateFailureKind::NotWritable => "update_dialog_error_not_writable",
                    SelfUpdateFailureKind::Download => "update_dialog_error_download",
                    SelfUpdateFailureKind::HashMismatch => "update_dialog_error_hash",
                    SelfUpdateFailureKind::Extract => "update_dialog_error_extract",
                    SelfUpdateFailureKind::ArchiveLayout => "update_dialog_error_layout",
                    SelfUpdateFailureKind::Swap => "update_dialog_error_swap",
                    SelfUpdateFailureKind::Restart => "update_dialog_error_restart",
                };
                let detail = self
                    .t("update_dialog_error_detail")
                    .replace("{error}", &failure.detail);
                column![
                    text(self.t("update_dialog_failed").to_string())
                        .size(theme::text_size::BODY_MEDIUM)
                        .style(|theme: &Theme| iced::widget::text::Style {
                            color: Some(pal_of(theme).error),
                        }),
                    text(self.t(reason_key).to_string())
                        .size(theme::text_size::BODY_MEDIUM)
                        .wrapping(iced::widget::text::Wrapping::WordOrGlyph)
                        .width(Length::Fill),
                    text(detail)
                        .size(theme::text_size::BODY_SMALL)
                        .style(muted_style)
                        .wrapping(iced::widget::text::Wrapping::WordOrGlyph)
                        .width(Length::Fill),
                ]
                .spacing(6)
                .into()
            }
            DirectUpdateState::Restarting => text(self.t("update_dialog_restarting").to_string())
                .size(theme::text_size::BODY_MEDIUM)
                .style(success_style)
                .into(),
        };

        let mut actions = row![Space::new().width(Length::Fill)]
            .spacing(DIRECT_UPDATE_DIALOG_ACTION_SPACING)
            .align_y(iced::Alignment::Center);
        if matches!(
            &self.operation.direct_update,
            DirectUpdateState::Ready | DirectUpdateState::Failed(_)
        ) {
            actions = actions.push(
                m3_outlined_button(self.t("btn_close").to_string())
                    .on_press(Message::UpdateDialogClose),
            );
            let install_label = if matches!(&self.operation.direct_update, DirectUpdateState::Ready)
            {
                self.t("update_dialog_install")
            } else {
                self.t("btn_retry")
            };
            actions = actions.push(
                m3_filled_button(install_label.to_string()).on_press_maybe(
                    self.can_install_self_update()
                        .then_some(Message::InstallSelfUpdate),
                ),
            );
        }

        let mut content = column![version].spacing(14);
        if !matches!(self.operation.direct_update, DirectUpdateState::Ready) {
            content = content.push(state_body);
        }
        if let Some(reason) = self.self_update_blocked_reason() {
            content = content.push(
                text(reason)
                    .size(theme::text_size::BODY_MEDIUM)
                    .style(muted_style)
                    .wrapping(iced::widget::text::Wrapping::WordOrGlyph)
                    .width(Length::Fill),
            );
        }
        // Browsing release notes is supplementary, not a third dialog decision.
        // Keep the dismiss/confirm pair adjacent in both ready and failed states.
        if matches!(
            &self.operation.direct_update,
            DirectUpdateState::Ready | DirectUpdateState::Failed(_)
        ) {
            content = content.push(
                m3_text_button(self.t("update_dialog_release_page").to_string())
                    .on_press(Message::OpenUpdateReleasePage),
            );
        }
        let content = popup_sections(title, content, actions, DIRECT_UPDATE_DIALOG_WIDTH, false);
        m3_dialog(content)
    }

    /// Device-info popup: render the Lenovo PTSTPD `data` block as a
    /// 2-column key/value table. Branches on `DeviceInfoState` so the
    /// modal stays open through Loading / Error / Ready transitions
    /// without flashing in/out of existence.
    pub(crate) fn device_info_popup_view(&self) -> Element<'_, Message> {
        let Some((serial, state)) = self.device_info_popup.clone() else {
            return container(text("")).into();
        };
        let title =
            text(self.t("device_info_popup_title").to_string()).size(theme::text_size::TITLE_LARGE);
        // Copy-icon button — only enabled once the upstream payload is
        // cached; clicking copies the unmodified `data` JSON to the
        // clipboard and surfaces a toast.
        let copy_payload: Option<String> = self
            .queries
            .info_cache
            .get(&serial)
            .map(|i| i.data_pretty.clone());
        let copy_glyph = text("⧉").size(16);
        let copy_btn = if let Some(payload) = copy_payload {
            button(container(copy_glyph).padding([2, 6]))
                .on_press(Message::CopyToClipboard(payload))
                .padding(0)
                .style(|t: &Theme, status| {
                    let p = pal_of(t);
                    // `surface_container` base + M3 state layer on hover / press.
                    let bg = theme::mix_color(
                        p.surface_container,
                        p.on_surface,
                        theme::state_alpha(status),
                    );
                    button::Style {
                        background: Some(bg.into()),
                        text_color: p.on_surface,
                        border: iced::Border {
                            radius: 6.0.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }
                })
        } else {
            // Same shape, no on_press — keeps the header layout stable
            // during the loading / error states without leaving an
            // active click target.
            button(container(copy_glyph).padding([2, 6]))
                .padding(0)
                .style(|t: &Theme, _s| {
                    let p = pal_of(t);
                    button::Style {
                        background: Some(p.surface_container.into()),
                        text_color: p.on_surface_variant,
                        border: iced::Border {
                            radius: 6.0.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }
                })
        };
        let header = iced::widget::row![title, Space::new().width(Length::Fill), copy_btn]
            .align_y(iced::Alignment::Center);
        let serial_line = text(format!("{}: {serial}", self.t("device_info_popup_serial")))
            .size(12)
            .style(muted_style);

        let body: Element<'_, Message> = match &state {
            DeviceInfoState::Loading => self.popup_loading_view(),
            DeviceInfoState::Error(e) => {
                self.popup_error_view("device_info_popup_error", e, Message::DeviceInfoRetry)
            }
            DeviceInfoState::Ready => {
                let info = match self.queries.info_cache.get(&serial) {
                    Some(i) => i,
                    None => {
                        return container(text("")).into();
                    }
                };
                let fields = info
                    .fields
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone().unwrap_or_default()))
                    .collect();
                scrollable(info_key_value_table(fields))
                    .style(m3_scrollable_style)
                    .height(Length::Fixed(420.0))
                    .width(Length::Fill)
                    .into()
            }
        };

        let close_btn =
            m3_outlined_button(self.t("btn_close").to_string()).on_press(Message::DeviceInfoClose);

        let content = popup_sections(
            column![header, serial_line].spacing(12),
            body,
            iced::widget::row![Space::new().width(Length::Fill), close_btn]
                .align_y(iced::Alignment::Center),
            theme::DIALOG_WIDTH_MD,
            matches!(state, DeviceInfoState::Ready),
        );

        m3_dialog(content)
    }

    /// Lenovo OTA "querynewfirmware" popup. Opens when the user clicks
    /// the dashboard firmware version. Mirrors `device_info_popup_view`
    /// for header / progress / error / close-button shape, but renders
    /// the OTA payload as a stacked card (From / To / Size / MD5 /
    /// Changelog / Download) instead of a flat key-value table.
    pub(crate) fn ota_popup_view(&self) -> Element<'_, Message> {
        let Some((_serial, _firmware_id, state)) = self.ota_popup.clone() else {
            return container(text("")).into();
        };
        let title = text(self.t("ota_popup_title").to_string()).size(theme::text_size::TITLE_LARGE);
        let header = iced::widget::row![title, Space::new().width(Length::Fill)]
            .align_y(iced::Alignment::Center);

        let body: Element<'_, Message> = match &state {
            OtaPopupState::Loading => self.popup_loading_view(),
            OtaPopupState::Error(e) => {
                self.popup_error_view("ota_popup_error", e, Message::OtaRetry)
            }
            OtaPopupState::NoUpdate => container(
                text(self.t("ota_popup_unavailable").to_string())
                    .size(14)
                    .style(muted_style)
                    .width(Length::Fill)
                    .center(),
            )
            .width(Length::Fill)
            .height(48)
            .center_x(Length::Fill)
            .center_y(48)
            .into(),
            OtaPopupState::Ready(update) => {
                // Changelog text lives in `self.ota_changelog_editor`,
                // seeded by the `OtaFetched` handler from `desc_cn`
                // (Chinese GUI locale, when populated) or `desc_en`.
                // Rendered here through `text_editor` so drag-select +
                // Ctrl+C work — a plain `text` widget is a static label
                // and won't surface a selection.
                let size_str = ltbox_core::lenovo_ota::format_size(update.size_bytes);

                let from_to_row = column![
                    text(format!("{}: {}", self.t("ota_popup_from"), update.from))
                        .size(12)
                        .style(muted_style),
                    text(format!("{}: {}", self.t("ota_popup_to"), update.to))
                        .size(theme::text_size::BODY_MEDIUM),
                ]
                .spacing(4);

                let meta_row = iced::widget::row![
                    info_kv(self.t("ota_popup_size"), &size_str),
                    info_kv(self.t("ota_popup_md5"), &update.md5),
                ]
                .spacing(40);

                let changelog_editor: Element<'_, Message> =
                    iced::widget::text_editor(&self.ota_changelog_editor)
                        .on_action(Message::OtaChangelogAction)
                        .size(12)
                        .into();
                let changelog_block = column![
                    text(self.t("ota_popup_changelog").to_string())
                        .size(11)
                        .style(muted_style),
                    container(changelog_editor)
                        .padding([8, 10])
                        .width(Length::Fill)
                        .style(|t: &Theme| {
                            let p = pal_of(t);
                            container::Style {
                                background: Some(p.surface_container_low.into()),
                                border: iced::Border {
                                    color: p.outline_variant,
                                    width: 1.0,
                                    radius: theme::shape::SM.into(),
                                },
                                ..Default::default()
                            }
                        }),
                ]
                .spacing(4);

                scrollable(
                    column![
                        from_to_row,
                        widget::rule::horizontal(1),
                        meta_row,
                        widget::rule::horizontal(1),
                        changelog_block,
                    ]
                    .spacing(12)
                    .width(Length::Fill),
                )
                .style(m3_scrollable_style)
                .height(Length::Fixed(420.0))
                .width(Length::Fill)
                .into()
            }
        };

        // Both buttons live on the fixed dialog footer below the scrollable.
        // Dismiss stays outlined to the left of the filled download action.
        let download_url: Option<String> = match &state {
            OtaPopupState::Ready(u) if !u.download_url.is_empty() => Some(u.download_url.clone()),
            _ => None,
        };
        let close_btn =
            m3_outlined_button(self.t("btn_close").to_string()).on_press(Message::OtaClose);
        let mut action_row = iced::widget::row![Space::new().width(Length::Fill), close_btn]
            .spacing(8)
            .align_y(iced::Alignment::Center);
        if let Some(url) = download_url {
            let download_btn = m3_filled_button(self.t("ota_popup_download").to_string())
                .on_press(Message::OtaOpenDownload(url));
            action_row = action_row.push(download_btn);
        }

        let content = popup_sections(
            header,
            body,
            action_row,
            theme::DIALOG_WIDTH_MD,
            matches!(state, OtaPopupState::Ready(_)),
        );

        m3_dialog(content)
    }

    /// QFIL-firmware popup: the official flash-tool package for a CN device
    /// (resolved via MTM → `getPadFlashingMachine`), or a Software Fix pointer
    /// for a global device. Branches on `QfilPopupState` like the OTA popup.
    pub(crate) fn qfil_popup_view(&self) -> Element<'_, Message> {
        let Some((_serial, state)) = self.qfil_popup.clone() else {
            return container(text("")).into();
        };
        let title =
            text(self.t("qfil_popup_title").to_string()).size(theme::text_size::TITLE_LARGE);
        let header = row![title, Space::new().width(Length::Fill)].align_y(iced::Alignment::Center);

        let placeholder = |key: &str| -> Element<'_, Message> {
            container(
                text(self.t(key).to_string())
                    .size(14)
                    .style(muted_style)
                    .width(Length::Fill)
                    .center(),
            )
            .width(Length::Fill)
            .height(48)
            .center_x(Length::Fill)
            .center_y(48)
            .into()
        };

        let body: Element<'_, Message> = match &state {
            QfilPopupState::Loading => self.popup_loading_view(),
            QfilPopupState::Error(e) => {
                self.popup_error_view("qfil_popup_error", e, Message::QfilRetry)
            }
            QfilPopupState::NoPackage => placeholder("qfil_popup_no_package"),
            QfilPopupState::Global => self.qfil_global_message(),
            QfilPopupState::Ready(pkg) => {
                let updated = pkg
                    .upd_time
                    .map(|t| crate::format_unix_timestamp_utc(t as u64))
                    .unwrap_or_default();
                let mut rows = column![].spacing(10).width(Length::Fill);
                if !pkg.version.is_empty() {
                    rows = rows.push(info_kv(self.t("qfil_popup_version"), &pkg.version));
                }
                if !pkg.file_name.is_empty() {
                    rows = rows.push(info_kv(self.t("qfil_popup_file"), &pkg.file_name));
                }
                if !pkg.platform.is_empty() {
                    rows = rows.push(info_kv(self.t("qfil_popup_platform"), &pkg.platform));
                }
                if !updated.is_empty() {
                    rows = rows.push(info_kv(self.t("qfil_popup_updated"), &updated));
                }
                // Archive password (fixed constant) with a copy affordance —
                // it isn't discoverable and the user needs it to extract.
                let pw = ltbox_core::lenovo_qfil::package_password();
                let pw_row = row![
                    info_kv(self.t("qfil_popup_password"), &pw),
                    Space::new().width(Length::Fill),
                    m3_text_button(self.t("qfil_popup_copy").to_string())
                        .on_press(Message::CopyToClipboard(pw.clone())),
                ]
                .align_y(iced::Alignment::Center);
                rows = rows.push(widget::rule::horizontal(1));
                rows = rows.push(pw_row);
                rows.into()
            }
        };

        let download_url: Option<String> = match &state {
            QfilPopupState::Ready(p) if !p.download_url.is_empty() => Some(p.download_url.clone()),
            _ => None,
        };
        let close_btn =
            m3_outlined_button(self.t("btn_close").to_string()).on_press(Message::QfilClose);
        let mut action_row = row![Space::new().width(Length::Fill), close_btn]
            .spacing(8)
            .align_y(iced::Alignment::Center);
        if let Some(url) = download_url {
            action_row = action_row.push(
                m3_filled_button(self.t("qfil_popup_download").to_string())
                    .on_press(Message::OpenExternalUrl(url)),
            );
        }

        let content = popup_sections(header, body, action_row, theme::DIALOG_WIDTH_MD, false);

        m3_dialog(content)
    }

    /// The global-device message with an inline "Software Fix" hyperlink. The
    /// term is untranslated (product name) in every locale, so we split the
    /// localized sentence on it and link that span.
    fn qfil_global_message(&self) -> Element<'_, Message> {
        const SOFTWARE_FIX_URL: &str = "https://pcsupport.lenovo.com/rescue-and-smart-assistant";
        const LINK_TERM: &str = "Software Fix";
        let msg = self.t("qfil_popup_global").to_string();
        let primary = self.pal().primary;
        let content: Element<'_, Message> = if let Some(idx) = msg.find(LINK_TERM) {
            let before = msg[..idx].to_string();
            let after = msg[idx + LINK_TERM.len()..].to_string();
            iced::widget::rich_text([
                iced::widget::span(before).size(theme::text_size::BODY_MEDIUM),
                iced::widget::span(LINK_TERM)
                    .size(theme::text_size::BODY_MEDIUM)
                    .color(primary)
                    .underline(true)
                    .link(SOFTWARE_FIX_URL.to_string()),
                iced::widget::span(after).size(theme::text_size::BODY_MEDIUM),
            ])
            .on_link_click(Message::OpenExternalUrl)
            .into()
        } else {
            text(msg).size(theme::text_size::BODY_MEDIUM).into()
        };
        container(content)
            .padding([12, 4])
            .width(Length::Fill)
            .into()
    }

    /// One `<partition> = <value>` row of the rollback-index popup.
    ///
    /// The value is a button rather than static text: pressing it steps
    /// the shared format cycle, so the same click target answers "what
    /// number is this really" and "what date is that". The copy button
    /// beside it copies exactly the string currently on screen.
    fn rollback_floor_row<'a>(&'a self, partition: &str, index: u64) -> Element<'a, Message> {
        let rendered = self.rollback_value_format.render(index);
        let value_btn = button(
            text(rendered.clone())
                .size(theme::text_size::TITLE_MEDIUM)
                .font(theme::emphasis::medium())
                .wrapping(iced::widget::text::Wrapping::None),
        )
        .on_press(Message::RollbackDetailCycleFormat)
        .padding([6, 10])
        .style(|t: &Theme, status| {
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
        });

        let copy_btn = m3_icon_button(icon::action_copy(), 16.0, |t: &Theme, status| {
            let p = pal_of(t);
            button::Style {
                background: theme::state_layer_bg(status, p.on_surface).map(Into::into),
                text_color: p.on_surface_variant,
                border: iced::Border {
                    radius: theme::shape::SM.into(),
                    ..Default::default()
                },
                ..Default::default()
            }
        })
        .on_press(Message::CopyToClipboard(rendered));

        let copy_btn = widget::tooltip(
            copy_btn,
            container(text(self.t("rollback_copy_tip").to_string()).size(11))
                .padding([6, 10])
                .style(|t: &Theme| theme::tooltip_style(t, theme::shape::SM)),
            widget::tooltip::Position::Top,
        )
        .gap(6);

        row![
            text(partition.to_string())
                .size(theme::text_size::BODY_MEDIUM)
                .style(muted_style)
                .width(Length::Fixed(150.0)),
            value_btn,
            Space::new().width(Length::Fill),
            copy_btn,
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center)
        .into()
    }

    /// Rollback-index breakdown for a device in bootloader mode.
    ///
    /// The Dashboard cell only answers "is rollback protection on"; this
    /// is where the two committed floors live, since a raw index is
    /// meaningless without knowing which partition it guards and what
    /// the number represents.
    pub(crate) fn rollback_detail_popup_view(&self) -> Element<'_, Message> {
        let Some(floors) = self.device.rollback_floors else {
            return container(text("")).into();
        };
        let slot = active_slot_suffix(Some(&self.device.slot));

        let title =
            text(self.t("rollback_popup_title").to_string()).size(theme::text_size::TITLE_LARGE);
        let desc = text(self.t("rollback_popup_desc").to_string())
            .size(theme::text_size::BODY_MEDIUM)
            .style(muted_style)
            .wrapping(iced::widget::text::Wrapping::WordOrGlyph)
            .width(Length::Fill);

        // Naming the active form turns the cycle from a hidden trick into
        // a legible control.
        let format_hint = row![
            text(self.t("rollback_format_label").to_string())
                .size(theme::text_size::LABEL_SMALL)
                .style(muted_style),
            text(self.t(self.rollback_value_format.label_key()).to_string())
                .size(theme::text_size::LABEL_SMALL)
                .font(theme::emphasis::medium())
                .style(accent_style),
            Space::new().width(Length::Fill),
            text(self.t("rollback_cycle_tip").to_string())
                .size(theme::text_size::LABEL_SMALL)
                .style(muted_style),
        ]
        .spacing(6)
        .align_y(iced::Alignment::Center);

        let rows = column![
            self.rollback_floor_row(&format!("boot{slot}"), floors.boot_index),
            self.rollback_floor_row(&format!("vbmeta_system{slot}"), floors.vbmeta_system_index),
        ]
        .spacing(4);

        let close_btn = m3_outlined_button(self.t("btn_close").to_string())
            .on_press(Message::RollbackDetailClose);

        let content = popup_sections(
            title,
            column![desc, format_hint, rows].spacing(14),
            row![Space::new().width(Length::Fill), close_btn].align_y(iced::Alignment::Center),
            theme::DIALOG_WIDTH_MD,
            false,
        );

        m3_dialog(content)
    }

    /// Manual rollback-index editor opened from the Flash-confirm rollback
    /// picker. Reuses the dashboard breakdown's format cycle and row shape;
    /// confirm stays disabled until both explicit targets are valid.
    pub(crate) fn manual_rollback_popup_view(&self) -> Element<'_, Message> {
        let Some((boot_buffer, vbmeta_buffer)) = self.manual_rollback_buffers.as_ref() else {
            return container(text("")).into();
        };

        let title =
            text(self.t("rollback_popup_title").to_string()).size(theme::text_size::TITLE_LARGE);
        let desc = text(self.t("rollback_manual_desc").to_string())
            .size(theme::text_size::BODY_MEDIUM)
            .style(muted_style)
            .wrapping(iced::widget::text::Wrapping::WordOrGlyph)
            .width(Length::Fill);

        let format_hint = row![
            text(self.t("rollback_format_label").to_string())
                .size(theme::text_size::LABEL_SMALL)
                .style(muted_style),
            text(self.t(self.manual_rollback_format.label_key()).to_string())
                .size(theme::text_size::LABEL_SMALL)
                .font(theme::emphasis::medium())
                .style(accent_style),
            Space::new().width(Length::Fill),
            text(self.t("rollback_cycle_tip").to_string())
                .size(theme::text_size::LABEL_SMALL)
                .style(muted_style),
        ]
        .spacing(6)
        .align_y(iced::Alignment::Center);

        let boot_result = self.parse_manual_rollback(boot_buffer);
        let vbmeta_result = self.parse_manual_rollback(vbmeta_buffer);
        let both_valid = boot_result.is_ok() && vbmeta_result.is_ok();
        // The hint under each field reports what the *image* carries, so it has
        // to be read from the firmware every time. Deriving it from the field
        // made it echo whatever the user had just typed.
        let originals = self.flash.firmware_rollback_indices.as_ref();
        let boot_field = self.manual_rollback_input(
            "boot",
            boot_buffer,
            boot_result,
            originals.map(|o| &o.0),
            ManualRollbackEditor::Boot,
        );
        let vbmeta_field = self.manual_rollback_input(
            "vbmeta_system",
            vbmeta_buffer,
            vbmeta_result,
            originals.map(|o| &o.1),
            ManualRollbackEditor::VbmetaSystem,
        );

        let cancel_btn = m3_outlined_button(self.t("btn_cancel").to_string())
            .on_press(Message::Flash(FlashMsg::FlashManualRollbackCancel));
        let ok_btn = {
            let btn = m3_filled_button(self.t("btn_ok").to_string());
            if both_valid {
                btn.on_press(Message::Flash(FlashMsg::FlashManualRollbackConfirm))
            } else {
                btn
            }
        };

        let content = popup_sections(
            title,
            column![desc, format_hint, boot_field, vbmeta_field].spacing(14),
            row![Space::new().width(Length::Fill), cancel_btn, ok_btn]
                .spacing(8)
                .align_y(iced::Alignment::Center),
            theme::DIALOG_WIDTH_LG,
            false,
        );

        m3_dialog(content)
    }

    fn manual_rollback_input<'a>(
        &'a self,
        partition: &'static str,
        buffer: &str,
        result: Result<u64, String>,
        original: Option<&Result<u64, String>>,
        field: ManualRollbackEditor,
    ) -> Element<'a, Message> {
        let format_button =
            button(text(self.t(self.manual_rollback_format.label_key()).to_string()).size(12))
                .on_press(Message::Flash(FlashMsg::FlashManualRollbackCycleFormat))
                .padding([4, 8])
                .style(|t: &Theme, status| {
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
                });
        let invalid = result.is_err();
        let input = iced::widget::text_input(self.t("rollback_manual_placeholder"), buffer)
            .on_input(move |value| Message::Flash(FlashMsg::FlashManualRollbackInput(field, value)))
            .padding([8, 12])
            .line_height(iced::widget::text::LineHeight::Absolute(24.0.into()))
            .size(theme::text_size::TITLE_MEDIUM)
            .width(Length::Fill)
            .style(move |t, status| {
                if invalid {
                    m3_text_input_error_style(t, status)
                } else {
                    m3_text_input_style(t, status)
                }
            });
        let input = container(input).height(Length::Fixed(40.0)).center_y(40);

        let status: Element<'_, Message> = match (&result, original) {
            // What the user typed is what they can act on, so its error wins
            // the one line this row has.
            (Err(reason), _) => dialog_field_error(self.t(reason).to_string()),
            (Ok(_), Some(Ok(index))) => text(tr_args!(
                "rollback_manual_original",
                index = self.manual_rollback_format.render(*index)
            ))
            .size(12)
            .style(success_style)
            .into(),
            // Unreadable image: say why rather than inventing an index.
            (Ok(_), Some(Err(reason))) => text(self.t(reason).to_string())
                .size(12)
                .style(muted_style)
                .into(),
            (Ok(_), None) => Space::new().into(),
        };

        column![
            dialog_field_label(partition),
            row![input, format_button]
                .spacing(8)
                .align_y(iced::Alignment::Center),
            status,
        ]
        .spacing(6)
        .into()
    }

    /// PatchArb timestamp popup. Reads `adv_wizard.arb_index_buffer`
    /// for the in-flight typing and renders the UTC representation in
    /// real time once the buffer hits exactly 10 digits. OK is enabled
    /// only on a 10-digit buffer that parses to a `u64`.
    pub(crate) fn arb_index_popup_view(&self) -> Element<'_, Message> {
        let buf = self.adv_wizard.arb_index_buffer.clone();
        let valid = buf.len() == 10 && buf.parse::<u64>().is_ok();

        // UTC preview only when the buffer is exactly 10 digits, so
        // shrinking the value (e.g. backspacing while editing) makes
        // the preview disappear instead of jumping to a stale time.
        let utc_preview: Element<'_, Message> = if valid {
            let ts: u64 = buf.parse().unwrap_or(0);
            let formatted = format_unix_timestamp_utc(ts);
            text(formatted)
                .size(theme::text_size::BODY_MEDIUM)
                .style(success_style)
                .into()
        } else {
            // Keep a fixed-height placeholder so the layout doesn't
            // jump when the preview appears / disappears.
            container(text("").size(theme::text_size::BODY_MEDIUM))
                .height(20)
                .into()
        };

        let header = column![
            text(self.t("arb_index_popup_title").to_string()).size(theme::text_size::TITLE_LARGE),
            text(self.t("arb_index_popup_subtitle").to_string())
                .size(theme::text_size::BODY_SMALL)
                .style(muted_style),
        ]
        .spacing(3);

        let input = iced::widget::text_input(
            self.t("arb_index_popup_placeholder"),
            &self.adv_wizard.arb_index_buffer,
        )
        .on_input(|s| Message::Adv(AdvMsg::AdvWizArbIndexInput(s)))
        .padding([8, 12])
        .line_height(iced::widget::text::LineHeight::Absolute(24.0.into()))
        .size(14)
        .width(Length::Fill)
        .style(m3_text_input_style);
        let input = if valid {
            input.on_submit(Message::Adv(AdvMsg::AdvWizArbIndexConfirm))
        } else {
            input
        };

        let input = container(input).height(Length::Fixed(40.0)).center_y(40);
        let cancel_btn = m3_outlined_button(self.t("btn_cancel").to_string())
            .on_press(Message::Adv(AdvMsg::AdvWizArbIndexCancel));
        let ok_btn = {
            let btn = m3_filled_button(self.t("btn_ok").to_string());
            if valid {
                btn.on_press(Message::Adv(AdvMsg::AdvWizArbIndexConfirm))
            } else {
                btn
            }
        };

        let content = popup_sections(
            header,
            column![input, utc_preview].spacing(6),
            iced::widget::row![Space::new().width(Length::Fill), cancel_btn, ok_btn]
                .spacing(8)
                .align_y(iced::Alignment::Center),
            theme::DIALOG_WIDTH_SM,
            false,
        );

        m3_dialog(content)
    }

    /// Manual serial-number prompt for auto region detection. Shown by the
    /// Flash region-step Auto FAB when no usable polled serial is available
    /// (device not in ADB/fastboot, or a garbled read).
    pub(crate) fn flash_serial_prompt_view(&self) -> Element<'_, Message> {
        let Some(buf) = self.flash_serial_prompt.clone() else {
            return container(text("")).into();
        };
        let valid = !buf.trim().is_empty();
        let header = column![
            text(self.t("flash_serial_prompt_title").to_string())
                .size(theme::text_size::TITLE_LARGE),
            text(self.t("flash_serial_prompt_subtitle").to_string())
                .size(theme::text_size::BODY_SMALL)
                .style(muted_style),
        ]
        .spacing(3);
        let input = iced::widget::text_input(self.t("flash_serial_prompt_placeholder"), &buf)
            .on_input(|s| Message::Flash(FlashMsg::FlashSerialPromptInput(s)))
            .padding([8, 12])
            .line_height(iced::widget::text::LineHeight::Absolute(24.0.into()))
            .size(14)
            .width(Length::Fill)
            .style(m3_text_input_style);
        let input = if valid {
            input.on_submit(Message::Flash(FlashMsg::FlashSerialPromptSubmit))
        } else {
            input
        };
        let input = container(input).height(Length::Fixed(40.0)).center_y(40);
        let skip_btn = m3_outlined_button(self.t("flash_serial_prompt_skip").to_string())
            .on_press(Message::Flash(FlashMsg::FlashSerialPromptSkip));
        let ok_btn = {
            let btn = m3_filled_button(self.t("btn_ok").to_string());
            if valid {
                btn.on_press(Message::Flash(FlashMsg::FlashSerialPromptSubmit))
            } else {
                btn
            }
        };
        let content = popup_sections(
            header,
            input,
            row![Space::new().width(Length::Fill), skip_btn, ok_btn]
                .spacing(8)
                .align_y(iced::Alignment::Center),
            theme::DIALOG_WIDTH_SM,
            false,
        );
        m3_dialog(content)
    }

    pub(crate) fn country_popup_view(&self) -> Element<'_, Message> {
        let query = self.country_popup_search.trim().to_ascii_lowercase();
        let filtered: Vec<&CountryEntry> = COUNTRY_CODES
            .iter()
            .filter(|entry| country_matches_search(entry, &query))
            .collect();
        let selected_code = self.country_popup_draft.target();
        let mut list = column![].spacing(0).width(Length::Fill);
        let mut has_row = false;

        // Flash wizard only — hide "Do not change" from the Advanced
        // PatchDevinfo flow because that action requires a concrete target.
        // Once search starts, only matching countries remain in the results.
        if !self.adv_needs_country && query.is_empty() {
            let no_change_selected = self.country_popup_draft.is_skipped()
                || (matches!(self.country_popup_draft, CountryAction::Unset)
                    && !self.wf_config.wipe);
            let mut contents = row![
                text(self.t("popup_country_do_not_change").to_string())
                    .size(theme::text_size::BODY_MEDIUM)
                    .width(Length::Fill),
            ]
            .align_y(iced::Alignment::Center)
            .width(Length::Fill);
            if no_change_selected {
                contents = contents.push(lucide_icon(icon::mark_check(), 16.0, |t: &Theme| {
                    pal_of(t).primary
                }));
            }
            list = list.push(
                button(contents.height(Length::Fill))
                    .on_press(Message::SkipCountryPatch)
                    .height(Length::Fixed(COUNTRY_ROW_HEIGHT))
                    .width(Length::Fill)
                    .padding([0, 20])
                    .style(move |t: &Theme, status| {
                        let p = pal_of(t);
                        let alpha = theme::state_alpha(status);
                        button::Style {
                            background: if no_change_selected {
                                Some(p.secondary_container.into())
                            } else if alpha > 0.0 {
                                Some(with_alpha(p.on_surface, alpha).into())
                            } else {
                                None
                            },
                            text_color: p.on_surface,
                            ..Default::default()
                        }
                    }),
            );
            has_row = true;
        }

        // TB322FC PRC-only: only CN is selectable in the Flash wizard. The
        // Advanced operation permits every country, so the gate is lifted there.
        let tb322fc = self.model_capabilities().prc_only && !self.adv_needs_country;
        for entry in &filtered {
            if has_row {
                list = list.push(widget::rule::horizontal(1).style(shell_rule_style));
            }
            let selected = selected_code == Some(entry.code);
            let disabled = tb322fc && !entry.code.eq_ignore_ascii_case("CN");
            list = list.push(country_popup_row(
                country_flags::svg(entry.code).map(|bytes| {
                    widget::svg(widget::svg::Handle::from_memory(bytes))
                        .width(COUNTRY_FLAG_WIDTH)
                        .height(COUNTRY_FLAG_HEIGHT)
                        .into()
                }),
                entry.code,
                entry.name,
                selected,
                disabled,
            ));
            has_row = true;
        }

        let search = text_input(self.t("popup_country_search"), &self.country_popup_search)
            .on_input(Message::CountrySearchInput)
            .width(Length::Fill)
            .padding([8, 12])
            .line_height(iced::widget::text::LineHeight::Absolute(24.0.into()))
            .size(theme::text_size::BODY_MEDIUM)
            .style(m3_text_input_style);
        let header = container(
            column![
                text(self.t("adv_country_title").to_string())
                    .size(theme::text_size::TITLE_MEDIUM)
                    .font(theme::emphasis::medium()),
                text(self.t("adv_country_subtitle").to_string())
                    .size(theme::text_size::BODY_SMALL)
                    .style(muted_style)
                    .width(Length::Fill)
                    .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
            ]
            .spacing(3)
            .width(Length::Fill),
        )
        .padding(iced::Padding {
            top: 18.0,
            right: 20.0,
            bottom: 10.0,
            left: 20.0,
        })
        .width(Length::Fill);
        let search_area = container(container(search).height(Length::Fixed(40.0)).center_y(40))
            .padding([16, 20])
            .width(Length::Fill);
        let list_area = column![
            widget::rule::horizontal(1).style(shell_rule_style),
            scrollable(list)
                .style(m3_scrollable_style)
                .height(Length::Fixed(COUNTRY_LIST_HEIGHT))
                .width(Length::Fill),
            widget::rule::horizontal(1).style(shell_rule_style),
        ]
        .spacing(0)
        .width(Length::Fill);

        let count = tr_args!("adv_country_pick_count", count = filtered.len().to_string());
        let can_confirm = if self.adv_needs_country {
            self.country_popup_draft.target().is_some()
        } else {
            !self.wf_config.wipe || !matches!(self.country_popup_draft, CountryAction::Unset)
        };
        let footer = container(
            row![
                text(count)
                    .size(theme::text_size::BODY_SMALL)
                    .style(muted_style),
                Space::new().width(Length::Fill),
                m3_outlined_button(self.t("btn_cancel").to_string())
                    .on_press(Message::DismissCountryPopup),
                m3_filled_button(self.t("btn_select").to_string())
                    .on_press_maybe(can_confirm.then_some(Message::CountryPopupConfirm),),
            ]
            .spacing(10)
            .align_y(iced::Alignment::Center)
            .width(Length::Fill),
        )
        .padding([14, 20])
        .width(Length::Fill);

        let popup_content: Element<'_, Message> = column![header, search_area, list_area, footer]
            .spacing(0)
            .width(Length::Fixed(theme::DIALOG_WIDTH_MD))
            .into();
        m3_dialog(popup_content)
    }

    /// PRC / ROW radio popup for the Advanced RegionConvert wizard.
    /// Smaller than the country popup (only two choices) so the
    /// content uses M3 radio rows in a fixed-width card.
    pub(crate) fn region_target_popup_view(&self) -> Element<'_, Message> {
        let selected = self.adv_wizard.region_target;
        let mut list = column![].spacing(2);
        for target in [DeviceRegion::Prc, DeviceRegion::Row] {
            let is_selected = selected == Some(target);
            let label = self.t(target.label_key()).to_string();
            list = list.push(
                button(text(label).size(theme::text_size::BODY_MEDIUM))
                    .on_press(Message::SelectRegionTarget(target))
                    .padding([6, 14])
                    .width(Length::Fill)
                    .style(move |t: &Theme, status| {
                        let p = pal_of(t);
                        button::Style {
                            background: if is_selected {
                                Some(
                                    theme::mix_color(
                                        p.primary,
                                        p.on_primary,
                                        theme::state_alpha(status),
                                    )
                                    .into(),
                                )
                            } else {
                                theme::state_layer_bg(status, p.on_surface).map(Into::into)
                            },
                            text_color: if is_selected {
                                p.on_primary
                            } else {
                                p.on_surface
                            },
                            ..Default::default()
                        }
                    }),
            );
        }

        let popup_content = popup_sections(
            text(self.t("popup_select_region_target").to_string())
                .size(REGION_TARGET_POPUP_TITLE_SIZE),
            list,
            row![
                Space::new().width(Length::Fill),
                m3_outlined_button(self.t("btn_cancel").to_string())
                    .on_press(Message::DismissRegionTargetPopup),
            ]
            .align_y(iced::Alignment::Center),
            REGION_TARGET_POPUP_WIDTH,
            false,
        );
        m3_dialog(popup_content)
    }

    /// Flash-confirm "hidden dropdown" editor. A small radio popup (same
    /// shape as `region_target_popup_view`) listing the alternatives for
    /// whichever confirm row was clicked. Each pick writes straight to
    /// `wf_config`. `Country` is handled by the country popup, so it never
    /// reaches here.
    pub(crate) fn flash_confirm_edit_popup(&self, field: ConfirmField) -> Element<'_, Message> {
        // (label, selected, on_press, disabled)
        let cfg = &self.wf_config;
        let tb322 = self.model_capabilities().prc_only;
        let opts: Vec<(String, bool, Message, bool)> = match field {
            ConfirmField::Region => [DeviceRegion::Prc, DeviceRegion::Row]
                .into_iter()
                .map(|r| {
                    (
                        self.t(r.label_key()).to_string(),
                        cfg.device_region == Some(r),
                        Message::Flash(FlashMsg::FlashConfirmSetRegion(r)),
                        tb322 && r == DeviceRegion::Row,
                    )
                })
                .collect(),
            ConfirmField::Target => [FlashTarget::OtherRegion, FlashTarget::SameRegion]
                .into_iter()
                .map(|t| {
                    (
                        self.t(t.label_key()).to_string(),
                        cfg.modify_region == (t == FlashTarget::OtherRegion),
                        Message::Flash(FlashMsg::FlashConfirmSetTarget(t)),
                        tb322 && t == FlashTarget::OtherRegion,
                    )
                })
                .collect(),
            ConfirmField::Data => [DataMode::Keep, DataMode::Wipe]
                .into_iter()
                .map(|d| {
                    (
                        self.t(if d == DataMode::Wipe {
                            "flash_confirm_data_wipe"
                        } else {
                            "flash_confirm_data_keep"
                        })
                        .to_string(),
                        cfg.wipe == (d == DataMode::Wipe),
                        Message::Flash(FlashMsg::FlashConfirmSetData(d)),
                        false,
                    )
                })
                .collect(),
            ConfirmField::RegionEdit => [true, false]
                .into_iter()
                .map(|on| {
                    (
                        self.t(if on {
                            "flash_confirm_rb_on"
                        } else {
                            "flash_confirm_rb_off"
                        })
                        .to_string(),
                        cfg.modify_region == on,
                        Message::Flash(FlashMsg::FlashConfirmSetRegionEdit(on)),
                        // PRC-only TB322FC can't cross regions — disable "On"
                        // to match the Target editor's OtherRegion gate.
                        tb322 && on,
                    )
                })
                .collect(),
            ConfirmField::Rollback => [
                RollbackSetting::Manual,
                RollbackSetting::On,
                RollbackSetting::Auto,
                RollbackSetting::Off,
            ]
            .into_iter()
            .map(|s| {
                (
                    self.t(match s {
                        RollbackSetting::Manual => "flash_confirm_rb_manual",
                        RollbackSetting::On => "flash_confirm_rb_on",
                        RollbackSetting::Auto => "flash_confirm_rb_auto",
                        RollbackSetting::Off => "flash_confirm_rb_off",
                    })
                    .to_string(),
                    cfg.modify_rollback == s,
                    Message::Flash(FlashMsg::FlashConfirmSetRollback(s)),
                    effective_rollback_mode(self.flash_rollback_policy(), s.to_mode())
                        != s.to_mode(),
                )
            })
            .collect(),
            // Country is routed to the dedicated country popup, never here.
            ConfirmField::Country => Vec::new(),
        };

        let mut list = column![].spacing(2);
        for (label, is_selected, on_press, disabled) in opts {
            let mut btn = button(
                row![
                    selection_radio(is_selected, !disabled, false),
                    text(label).size(theme::text_size::BODY_MEDIUM),
                ]
                .spacing(16)
                .align_y(iced::Alignment::Center),
            )
            .padding([12, 16])
            .width(Length::Fill)
            .style(move |t: &Theme, status| {
                let mut style = expressive_choice_style(t, status, is_selected, false);
                if disabled {
                    style.text_color = with_alpha(pal_of(t).on_surface, 0.38);
                }
                style
            });
            if !disabled {
                btn = btn.on_press(on_press);
            }
            list = list.push(btn);
        }

        let popup_content = popup_sections(
            text(self.t("flash_confirm_edit_title").to_string())
                .size(theme::text_size::TITLE_LARGE),
            list,
            row![
                Space::new().width(Length::Fill),
                m3_outlined_button(self.t("btn_cancel").to_string())
                    .on_press(Message::Flash(FlashMsg::FlashConfirmClose)),
            ]
            .align_y(iced::Alignment::Center),
            theme::DIALOG_WIDTH_MD,
            false,
        );
        m3_dialog(popup_content)
    }

    pub(crate) fn flash_firmware_identity_popup(&self) -> Element<'_, Message> {
        let dialog = self
            .flash
            .firmware_identity_dialog
            .as_ref()
            .expect("firmware identity dialog must be open");
        let ready = matches!(dialog, FirmwareIdentityDialog::Ready);
        let title_key = if ready {
            "flash_firmware_identity_title"
        } else {
            "flash_firmware_identity_error_title"
        };

        let details: Element<'_, Message> = match dialog {
            FirmwareIdentityDialog::Ready => {
                let identity = self.flash.firmware_identity.as_ref();
                let key_class = identity
                    .map(|value| value.key_class)
                    .unwrap_or(ltbox_patch::key_map::KeyClass::Unknown);
                let verdict_key = match key_class {
                    ltbox_patch::key_map::KeyClass::Testkey => "flash_key_testkey",
                    ltbox_patch::key_map::KeyClass::Lenovo => "flash_key_lenovo",
                    ltbox_patch::key_map::KeyClass::Unknown => "flash_key_unknown",
                };
                let model = identity
                    .and_then(|value| value.model_token.as_deref())
                    .unwrap_or_else(|| self.t("flash_firmware_model_unknown"));
                let mut details = column![].spacing(8);
                if identity.is_some_and(FirmwareIdentity::uses_gbl) {
                    let efisp_key = match identity.map(|value| value.efisp_load) {
                        Some(ltbox_patch::efisp_load::EfispLoad::Yes) => "common_yes",
                        Some(ltbox_patch::efisp_load::EfispLoad::No) => "common_no",
                        _ => "efisp_load_unknown",
                    };
                    details = details.push(info_kv_center(
                        self.t("efisp_load_label"),
                        self.t(efisp_key),
                    ));
                }
                details
                    .push(info_kv_center(
                        self.t("flash_firmware_identity_key"),
                        self.t(verdict_key),
                    ))
                    .push(info_kv_center(
                        self.t("flash_firmware_identity_model"),
                        model,
                    ))
                    .into()
            }
            FirmwareIdentityDialog::Failed(error) => text(error.clone())
                .size(theme::text_size::BODY_MEDIUM)
                .wrapping(iced::widget::text::Wrapping::WordOrGlyph)
                .into(),
        };

        let action_label = if ready { "btn_next" } else { "btn_close" };
        let action = if ready {
            m3_filled_button(self.t(action_label).to_string())
        } else {
            m3_outlined_button(self.t(action_label).to_string())
        };
        let popup_content = popup_sections(
            text(self.t(title_key).to_string()).size(16),
            details,
            row![
                Space::new().width(Length::Fill),
                action.on_press(Message::Flash(FlashMsg::FlashFirmwareIdentityDialogAction,)),
            ],
            theme::DIALOG_WIDTH_MD,
            false,
        );
        m3_dialog(popup_content)
    }

    pub(crate) fn rescue_region_popup_view(&self) -> Element<'_, Message> {
        let mk_option = |region: RescueRegion, desc_key: &'static str| {
            let label = self.t(region.label_key()).to_string();
            let desc = self.t(desc_key).to_string();
            let selected = self.sysupdate.rescue_region == Some(region);
            button(
                column![
                    text(label)
                        .size(theme::text_size::BODY_MEDIUM)
                        .style(on_surface_style),
                    text(desc).size(12).style(muted_style),
                ]
                .spacing(4),
            )
            .on_press(Message::Sys(SysMsg::SysRescueRegion(region)))
            .padding([10, 16])
            .width(Length::Fill)
            .style(move |t: &Theme, status| {
                let p = pal_of(t);
                let background = if selected {
                    Some(
                        theme::mix_color(
                            p.primary_container,
                            p.on_primary_container,
                            theme::state_alpha(status),
                        )
                        .into(),
                    )
                } else {
                    theme::state_layer_bg(status, p.on_surface).map(Into::into)
                };
                button::Style {
                    background,
                    text_color: p.on_surface,
                    border: iced::Border {
                        color: if selected { p.primary } else { p.outline },
                        width: 1.0,
                        radius: theme::shape::SM.into(),
                    },
                    ..Default::default()
                }
            })
        };
        let popup_content = popup_sections(
            text(self.t("rescue_region_popup_title").to_string()).size(16),
            column![
                text(self.t("rescue_region_popup_subtitle").to_string())
                    .size(12)
                    .style(muted_style),
                mk_option(RescueRegion::Prc, "rescue_region_prc_desc"),
                mk_option(RescueRegion::Row, "rescue_region_row_desc"),
            ]
            .spacing(10),
            row![
                Space::new().width(Length::Fill),
                m3_outlined_button(self.t("btn_cancel").to_string())
                    .on_press(Message::Sys(SysMsg::SysRescueRegionPopupDismiss)),
            ]
            .align_y(iced::Alignment::Center),
            theme::DIALOG_WIDTH_MD,
            false,
        );
        m3_dialog(popup_content)
    }

    /// Full-viewport log popup. Replaces the wizard body while open;
    /// dismissed via Close.
    pub(crate) fn log_popup_view(&self) -> Element<'_, Message> {
        let editor = iced::widget::text_editor(&self.log_editor)
            .on_action(Message::LogEditorAction)
            .size(11)
            .height(Length::Fill)
            .padding(iced::Padding {
                top: 0.0,
                right: 16.0,
                bottom: 10.0,
                left: 16.0,
            })
            .style(m3_log_text_editor_style);
        let body = column![
            row![
                text(self.t("log_popup_title").to_string()).size(theme::text_size::TITLE_LARGE),
                Space::new().width(Length::Fill),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
            widget::rule::horizontal(1),
            m3_log_text_field(self.t("dash_log").to_string(), editor.into()),
        ]
        .spacing(12)
        .padding(20)
        .width(Length::Fill)
        .height(Length::Fill);
        let utility_actions = row![
            wizard_secondary_action(
                icon::fab_save_log(),
                self.t("btn_save_log").to_string(),
                Some(Message::SaveLog),
            ),
            wizard_secondary_action(
                icon::fab_cancel(),
                self.t("btn_close").to_string(),
                Some(Message::ToggleLogPopup(false)),
            ),
        ]
        .spacing(ACTION_BUTTON_SPACING)
        .align_y(iced::Alignment::Center)
        .height(Length::Fill);
        let actions = utility_actions;

        column![
            container(body).width(Length::Fill).height(Length::Fill),
            wizard_action_footer(row![].height(Length::Fill), actions),
        ]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }
}

// The locale guard measures these popup buttons through shared layout
// constants, but the buttons take their size from their type role. Pin them
// together so the guard cannot measure a size the buttons stopped using.
const _: () = {
    assert!(DIRECT_UPDATE_DIALOG_ACTION_SIZE.to_bits() == theme::text_size::BODY_MEDIUM.to_bits());
    assert!(REGION_TARGET_POPUP_ACTION_SIZE.to_bits() == theme::text_size::BODY_MEDIUM.to_bits());
};

#[cfg(test)]
mod country_popup_tests {
    use super::*;

    #[test]
    fn country_search_matches_name_and_code_case_insensitively() {
        let korea = COUNTRY_CODES
            .iter()
            .find(|entry| entry.code == "KR")
            .expect("KR country entry");

        assert!(country_matches_search(korea, "kr"));
        assert!(country_matches_search(korea, "KORE"));
        assert!(country_matches_search(korea, "  korea  "));
        assert!(!country_matches_search(korea, "japan"));
    }
}
