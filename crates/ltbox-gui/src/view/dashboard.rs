//! Dashboard view: device status and action tiles.

use crate::focus_button::{self as button, button};
use crate::*;
use iced::widget::{Space, column, container, row, text};
use iced::{Element, Length, Theme};

fn dashboard_definition_row<'a>(
    label: Element<'a, Message>,
    value: Element<'a, Message>,
    divider: bool,
) -> Element<'a, Message> {
    let definition = row![
        container(label).width(Length::Fixed(180.0)),
        container(value).width(Length::Fill),
    ]
    .spacing(12.0)
    .width(Length::Fill)
    .align_y(iced::Alignment::Center)
    .padding([9.0, 0.0]);
    let mut row = column![definition].spacing(0).width(Length::Fill);
    if divider {
        row = row.push(iced::widget::rule::horizontal(1).style(shell_rule_style));
    }
    row.into()
}

fn device_identity_detail(model: &str, android_version: &str) -> String {
    if android_version.is_empty() {
        model.to_string()
    } else {
        format!("{model} · Android {android_version}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DashboardActionAvailability {
    ota: bool,
    qfil: bool,
    info: bool,
}

fn dashboard_action_availability(
    serial: &str,
    firmware_full: &str,
    firmware: &str,
) -> DashboardActionAvailability {
    let has_serial = !serial.trim().is_empty();
    let firmware_fingerprint = if firmware_full.is_empty() {
        firmware
    } else {
        firmware_full
    };
    DashboardActionAvailability {
        ota: has_serial && !firmware_fingerprint.trim().is_empty(),
        qfil: has_serial && firmware_fingerprint.to_ascii_uppercase().contains("ZUXOS"),
        info: has_serial,
    }
}

fn dashboard_quick_action_style(t: &Theme, status: button::Status) -> button::Style {
    let p = pal_of(t);
    let mut style = dash_icon_btn_style(t, status);
    style.background = Some(
        theme::mix_color(
            p.surface_container_low,
            p.primary,
            theme::state_alpha(status),
        )
        .into(),
    );
    style.text_color = if matches!(status, button::Status::Disabled) {
        with_alpha(p.on_surface, 0.38)
    } else {
        p.on_surface
    };
    style.border.color = if matches!(status, button::Status::Disabled) {
        with_alpha(p.on_surface, 0.12)
    } else if matches!(status, button::Status::Hovered | button::Status::Pressed) {
        p.primary
    } else {
        p.outline
    };
    style.border.radius = theme::shape::MD.into();
    style
}

fn dashboard_quick_action<'a>(
    label: String,
    enabled: bool,
    message: Message,
) -> Element<'a, Message> {
    button(
        // `Fill` height so the label centres vertically in the button.
        container(
            text(label)
                .size(theme::text_size::BODY_SMALL)
                .wrapping(iced::widget::text::Wrapping::None),
        )
        .height(Length::Fill)
        .align_y(iced::alignment::Vertical::Center),
    )
    .on_press_maybe(enabled.then_some(message))
    .height(Length::Fixed(38.0))
    .padding([0.0, 13.0])
    .style(dashboard_quick_action_style)
    .into()
}

impl App {
    /// Not `cfg`-gated: gating only the banner would leave
    /// `ForceCloseSoftwareFix` unconstructed off Windows, which `-D dead-code`
    /// rejects. `poll_software_fix` and `can_close_software_fix` hold the
    /// runtime `cfg!(windows)` guards, so off Windows `running` is only set by
    /// the demo scene.
    fn software_fix_banner(&self) -> Element<'_, Message> {
        let label = if self.software_fix.closing {
            "software_fix_closing"
        } else {
            "btn_software_fix_force_close"
        };
        let action = button(text(self.t(label).to_string()).size(theme::text_size::BODY_MEDIUM))
            .on_press_maybe(
                self.can_close_software_fix()
                    .then_some(Message::ForceCloseSoftwareFix),
            )
            .padding([10.0, 18.0])
            .style(banner_filled_btn_style);
        let mut body_copy = column![
            text(self.t("dash_software_fix_desc").to_string())
                .size(theme::text_size::BODY_MEDIUM)
                .line_height(iced::widget::text::LineHeight::Absolute(20.0.into()))
                .style(warning_container_text_style),
        ]
        .spacing(8.0)
        .width(Length::Fill);
        if let Some(key) = self.software_fix.error_key {
            body_copy = body_copy.push(
                text(self.t(key).to_string())
                    .size(theme::text_size::BODY_MEDIUM)
                    .line_height(iced::widget::text::LineHeight::Absolute(20.0.into()))
                    .style(warning_container_text_style),
            );
        }
        self.message_banner_with_trailing(
            BannerSeverity::Warning,
            icon::banner_warning(),
            self.t("dash_software_fix_title").to_string(),
            body_copy,
            Some(action.into()),
        )
    }

    /// The banner for the multi-device states, with the button that opens the
    /// choice dialog when a choice is possible.
    fn multi_device_banner(&self) -> Option<Element<'_, Message>> {
        let (body_key, can_choose) = match self.device_choice.state {
            device_choice::MultiDeviceState::Single | device_choice::MultiDeviceState::Chosen => {
                return None;
            }
            device_choice::MultiDeviceState::NeedsChoice => (
                if self.device_choice.rechoose {
                    "dash_multi_device_rechoose"
                } else {
                    "dash_multi_device_choose"
                },
                true,
            ),
            device_choice::MultiDeviceState::MultipleEdl => ("dash_multi_edl", false),
            device_choice::MultiDeviceState::Unselectable => {
                ("dash_multi_device_disconnect", false)
            }
        };
        let body = text(self.t(body_key).to_string())
            .size(theme::text_size::BODY_MEDIUM)
            .line_height(iced::widget::text::LineHeight::Absolute(20.0.into()))
            .style(warning_container_text_style)
            .width(Length::Fill);
        let choose: Option<Element<'_, Message>> = can_choose.then(|| {
            button(
                text(self.t("btn_choose_device").to_string())
                    .size(theme::text_size::BODY_MEDIUM)
                    .wrapping(iced::widget::text::Wrapping::None),
            )
            .on_press_maybe(
                (!self.operation.is_running())
                    .then_some(Message::DeviceChoice(DeviceChoiceMsg::Open)),
            )
            .padding([10.0, 18.0])
            .height(Length::Fixed(40.0))
            .style(banner_filled_btn_style)
            .into()
        });
        Some(self.message_banner_with_trailing(
            BannerSeverity::Warning,
            icon::banner_warning(),
            self.t("dash_multi_device_title").to_string(),
            body,
            choose,
        ))
    }

    /// The card body while the user must choose: one connected choice row per
    /// device, scrolling inside the card past three rows. A row chooses at once.
    fn device_choice_card_list(&self) -> Element<'_, Message> {
        let mut rows = column![].spacing(2.0).width(Length::Fill);
        for (index, device) in self.device_choice.inventory.iter().enumerate() {
            let (label, detail) = self.device_choice_row_copy(index, device);
            rows = rows.push(dialog_choice(
                label,
                Some(detail),
                false,
                Some(Message::DeviceChoice(DeviceChoiceMsg::PickNow(
                    device.id.clone(),
                ))),
            ));
        }
        container(iced::widget::scrollable(rows).style(m3_scrollable_style))
            .max_height(super::popups::DEVICE_CHOICE_LIST_MAX_HEIGHT)
            .width(Length::Fill)
            .into()
    }

    /// The device card body while nothing is connected: a title and the three
    /// steps that get a tablet detected.
    fn dashboard_no_device_steps(&self) -> Element<'_, Message> {
        let step = |number: &'static str, key: &'static str| -> Element<'_, Message> {
            row![
                container(
                    text(number)
                        .size(theme::text_size::BODY_MEDIUM)
                        .font(theme::emphasis::medium())
                        .style(muted_style),
                )
                .width(Length::Fixed(20.0)),
                text(self.t(key).to_string())
                    .size(theme::text_size::BODY_MEDIUM)
                    .line_height(iced::widget::text::LineHeight::Absolute(20.0.into()))
                    .width(Length::Fill),
            ]
            .spacing(8.0)
            .width(Length::Fill)
            .into()
        };
        column![
            text(self.t("dash_no_device_title").to_string())
                .size(theme::text_size::BODY_MEDIUM)
                .font(theme::emphasis::medium()),
            step("1", "dash_no_device_step_debugging"),
            step("2", "dash_no_device_step_cable"),
            step("3", "dash_no_device_step_prompt"),
            Space::new().height(2.0),
        ]
        .spacing(8.0)
        .padding([6.0, 0.0])
        .width(Length::Fill)
        .into()
    }

    pub(crate) fn view_dashboard(&self) -> Element<'_, Message> {
        iced::widget::responsive(move |size| self.dashboard_at_height(size.height)).into()
    }

    fn dashboard_at_height(&self, viewport_height: f32) -> Element<'_, Message> {
        let model = if self.device.model.is_empty() {
            "—"
        } else {
            &self.device.model
        };
        let slot = if self.device.slot.is_empty() {
            "—"
        } else {
            &self.device.slot
        };
        let firmware = if self.device.firmware.is_empty() {
            "—"
        } else {
            &self.device.firmware
        };
        // Shows only the rollback policy; committed floors are deliberately not
        // rendered here, even when fastboot supplied them.
        let arb_key = arb_from_model(&self.device.model);
        // Dedicated on/off keys: the shared yes/no wording reads as an answer
        // to a question the row never asks.
        let arb_display = match arb_key {
            "" => "—".to_string(),
            "arb_yes" => self.t("dash_rollback_on").to_string(),
            _ => self.t("dash_rollback_off").to_string(),
        };
        let arb = arb_display.as_str();
        let ram = if self.device.ram.is_empty() {
            "—"
        } else {
            &self.device.ram
        };
        let storage = if self.device.storage.is_empty() {
            "—"
        } else {
            &self.device.storage
        };
        let mut banners: Vec<Element<'_, Message>> = Vec::new();

        if self.software_fix.running {
            banners.push(self.software_fix_banner());
        }

        // With several devices nothing was read, so an ADB, sideload or platform
        // banner would describe a device LTBox never looked at.
        if let Some(banner) = self.multi_device_banner() {
            banners.push(banner);
        // ADB access problems win over the platform warning: an unreadable
        // `ro.boot.hardware` would otherwise read as "unsupported platform".
        } else if self.device.connection == ConnectionStatus::AdbServerBlocking {
            let msg = text(self.t("dash_adb_server_blocking").to_string())
                .size(theme::text_size::BODY_MEDIUM)
                .line_height(iced::widget::text::LineHeight::Absolute(20.0.into()))
                .style(warning_container_text_style)
                .width(Length::Fill);
            let kill_btn = button(
                text(self.t("btn_kill_adb_server").to_string())
                    .size(theme::text_size::BODY_MEDIUM)
                    .wrapping(iced::widget::text::Wrapping::None),
            )
            .on_press(Message::KillAdbServer)
            .padding([10.0, 18.0])
            .height(Length::Fixed(40.0))
            .style(banner_filled_btn_style);
            banners.push(self.message_banner_with_trailing(
                BannerSeverity::Warning,
                icon::banner_warning(),
                self.t("dash_adb_server_title").to_string(),
                msg,
                Some(kill_btn.into()),
            ));
        } else if self.device.connection == ConnectionStatus::AdbUnauthorized {
            banners.push(
                self.message_banner(
                    BannerSeverity::Warning,
                    icon::banner_warning(),
                    self.t("banner_warning_title").to_string(),
                    text(self.t("dash_adb_unauthorized").to_string())
                        .size(theme::text_size::BODY_MEDIUM)
                        .line_height(iced::widget::text::LineHeight::Absolute(20.0.into()))
                        .style(warning_container_text_style)
                        .width(Length::Fill),
                ),
            );
        } else if self.device.connection == ConnectionStatus::AdbSideload {
            banners.push(
                self.message_banner(
                    BannerSeverity::Warning,
                    icon::banner_warning(),
                    self.t("banner_warning_title").to_string(),
                    text(self.t("dash_adb_sideload").to_string())
                        .size(theme::text_size::BODY_MEDIUM)
                        .line_height(iced::widget::text::LineHeight::Absolute(20.0.into()))
                        .style(warning_container_text_style)
                        .width(Length::Fill),
                ),
            );
        } else if self.device.platform_supported == Some(false) {
            banners.push(
                self.message_banner(
                    BannerSeverity::Warning,
                    icon::banner_warning(),
                    self.t("banner_warning_title").to_string(),
                    text(self.t("dash_unsupported_platform").to_string())
                        .size(theme::text_size::BODY_MEDIUM)
                        .line_height(iced::widget::text::LineHeight::Absolute(20.0.into()))
                        .style(warning_container_text_style)
                        .width(Length::Fill),
                ),
            );
        }

        if let Some(banner) = self.driver_install_banner() {
            banners.push(banner);
        }

        let mut content = column![]
            .spacing(14.0)
            .width(Length::Fill)
            .height(Length::Fill);

        let needs_choice = self.device_choice.state == device_choice::MultiDeviceState::NeedsChoice;
        // Reserve the portrait's height even before a model is known.
        let mut identity = row![]
            .height(64.0)
            .spacing(16.0)
            .align_y(iced::Alignment::Center);
        if !needs_choice && !self.device.model.is_empty() {
            let portrait: Element<'_, Message> = match device_portrait(&self.device.model) {
                DevicePortrait::Png(handle) => iced::widget::image(handle)
                    .content_fit(iced::ContentFit::ScaleDown)
                    .into(),
                DevicePortrait::Svg(handle) => iced::widget::svg(handle)
                    .content_fit(iced::ContentFit::ScaleDown)
                    .into(),
            };
            identity = identity.push(
                // Not `center_x(Fill)`: it would override the fixed width.
                container(portrait)
                    .width(Length::Fixed(92.0))
                    .height(Length::Fixed(64.0))
                    .align_x(iced::alignment::Horizontal::Left)
                    .align_y(iced::alignment::Vertical::Center),
            );
        }
        // With a choice pending no device was read, so the card names the choice.
        let (device_name, identity_detail) = if needs_choice {
            (
                self.t("device_choice_title").to_string(),
                self.t("device_choice_body").to_string(),
            )
        } else if self.device.market_name.is_empty() {
            (
                if self.device.model.is_empty() {
                    self.t("dash_device").to_string()
                } else {
                    model.to_string()
                },
                device_identity_detail(model, &self.device.android_version),
            )
        } else {
            (
                self.device.market_name.clone(),
                device_identity_detail(model, &self.device.android_version),
            )
        };
        identity = identity.push(
            column![
                text(device_name)
                    .size(theme::text_size::TITLE_MEDIUM)
                    .font(theme::emphasis::medium()),
                text(identity_detail).style(muted_style),
            ]
            .spacing(4.0)
            .width(Length::Fill),
        );
        // Only with a choice behind it: with one device there is nothing to change.
        if self.device_choice.state == device_choice::MultiDeviceState::Chosen
            && self.device_choice.inventory.len() >= 2
        {
            identity = identity.push(
                m3_text_button(self.t("btn_change_device").to_string()).on_press_maybe(
                    (!self.operation.is_running())
                        .then_some(Message::DeviceChoice(DeviceChoiceMsg::Open)),
                ),
            );
        }
        let label = |key: &'static str| -> Element<'_, Message> {
            text(self.t(key).to_string())
                .size(theme::text_size::LABEL_SMALL)
                .style(muted_style)
                .into()
        };
        let value = |value: String| -> Element<'_, Message> {
            text(value)
                .size(theme::text_size::BODY_MEDIUM)
                .line_height(iced::widget::text::LineHeight::Absolute(20.0.into()))
                .font(theme::emphasis::medium())
                .wrapping(iced::widget::text::Wrapping::WordOrGlyph)
                .into()
        };

        let rollback_label = label("device_arb");

        let rollback_value: Element<'_, Message> = if self.device.rollback_floors.is_some() {
            iced::widget::tooltip(
                button(value(arb.to_string()))
                    .on_press(Message::RollbackDetailOpen)
                    .padding(0)
                    .width(Length::Fill)
                    .style(dash_clickable_btn_style),
                container(text(self.t("rollback_open_tip").to_string()).size(11.0))
                    .padding([6, 10])
                    .style(|t: &Theme| theme::tooltip_style(t, theme::shape::SM)),
                iced::widget::tooltip::Position::Top,
            )
            .into()
        } else {
            value(arb.to_string())
        };
        let actions = if needs_choice {
            DashboardActionAvailability {
                ota: false,
                qfil: false,
                info: false,
            }
        } else {
            dashboard_action_availability(
                &self.device.serial,
                &self.device.firmware_full,
                &self.device.firmware,
            )
        };
        let quick_actions = row![
            dashboard_quick_action(
                self.t("dash_action_ota_lookup").to_string(),
                actions.ota,
                Message::OtaOpen,
            ),
            dashboard_quick_action(
                self.t("dash_action_firmware_lookup").to_string(),
                actions.qfil,
                Message::QfilOpen,
            ),
            dashboard_quick_action(
                self.t("device_info_popup_title").to_string(),
                actions.info,
                Message::DeviceInfoOpen,
            ),
        ]
        .spacing(8.0)
        .align_y(iced::Alignment::Center);
        // No tablet: the table would be four rows of dashes, so say what to do.
        let details: Element<'_, Message> = if needs_choice {
            self.device_choice_card_list()
        } else if self.device.connection == ConnectionStatus::None {
            self.dashboard_no_device_steps()
        } else {
            column![
                dashboard_definition_row(
                    text(format!(
                        "{} · {}",
                        self.t("device_ram"),
                        self.t("device_storage")
                    ))
                    .size(theme::text_size::LABEL_SMALL)
                    .style(muted_style)
                    .into(),
                    value(format!("{ram} · {storage}")),
                    true,
                ),
                dashboard_definition_row(label("device_slot"), value(slot.to_string()), true,),
                dashboard_definition_row(rollback_label, rollback_value, true),
                dashboard_definition_row(
                    label("device_firmware"),
                    value(firmware.to_string()),
                    false,
                ),
            ]
            .spacing(0)
            .width(Length::Fill)
            .into()
        };
        let device_card_inner = column![
            identity,
            Space::new().height(12.0),
            details,
            iced::widget::rule::horizontal(1).style(shell_rule_style),
            Space::new().height(14.0),
            quick_actions,
        ]
        .spacing(0)
        .width(Length::Fill);
        content = content.push(
            container(
                // Padding scales with the corner radius (M3: outer radius -
                // padding = inner radius); a small inset would crowd the curve.
                container(device_card_inner)
                    .padding(DEVICE_CARD_PADDING)
                    .width(Length::Fill),
            )
            .width(Length::Fill)
            .style(|t: &Theme| {
                // Hero card: larger shape and the brightest surface, same
                // zero-elevation outline as the other cards. No type escalation;
                // the content is mostly reference data.
                theme::surface_card_style(t, theme::SurfaceLevel::Brightest, theme::shape::LG)
            }),
        );
        // Plain text, not the shared log editor: the editor tracks the document
        // end, which leaves a blank band above short logs.
        let dash_log = iced::widget::scrollable(
            text(self.log_lines.join("\n"))
                .size(theme::text_size::BODY_SMALL)
                .line_height(16.0 / 12.0)
                .width(Length::Fill)
                .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
        )
        .height(Length::Fill)
        .width(Length::Fill)
        .style(m3_scrollable_style);
        let clear_log_action =
            m3_text_button(self.t("btn_clear").to_string()).on_press(Message::ClearLog);
        let save_log_action =
            m3_text_button(self.t("btn_save").to_string()).on_press(Message::SaveLog);
        let log_header = container(
            row![
                text(self.t("dash_log").to_string())
                    .size(theme::text_size::BODY_MEDIUM)
                    .font(theme::emphasis::medium())
                    .style(muted_style),
                Space::new().width(Length::Fill),
                clear_log_action,
                save_log_action,
            ]
            .spacing(8.0)
            .align_y(iced::Alignment::Center),
        )
        .padding(iced::Padding {
            top: 6.0,
            right: 16.0,
            bottom: 2.0,
            left: 18.0,
        })
        .width(Length::Fill);
        let log_card = container(
            column![
                log_header,
                container(dash_log)
                    .padding(iced::Padding {
                        top: 0.0,
                        right: 16.0,
                        bottom: 10.0,
                        left: 16.0,
                    })
                    .width(Length::Fill)
                    .height(Length::Fill),
            ]
            .spacing(0)
            .width(Length::Fill)
            .height(Length::Fill),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|t: &Theme| {
            theme::surface_card_style(t, theme::SurfaceLevel::Default, theme::shape::MD)
        });
        // Shown only while an operation runs: the way back to a flow the user
        // navigated away from.
        if busy_navigation_target(self.operation.is_running(), self.operation.view()).is_some() {
            let resume = button(
                row![
                    text(format!(
                        "{} — {}",
                        self.busy_operation_label(),
                        self.t("dash_operation_in_progress")
                    ))
                    .size(theme::text_size::BODY_MEDIUM)
                    .style(accent_style),
                    Space::new().width(Length::Fill),
                    text(self.t("dash_open_operation").to_string())
                        .size(theme::text_size::BODY_SMALL)
                        .style(accent_style),
                    icon::fab_next().size(18.0).style(accent_style),
                ]
                .spacing(6.0)
                .align_y(iced::Alignment::Center),
            )
            .on_press(Message::ResumeBusyOperation)
            .padding([12.0, 16.0])
            .width(Length::Fill)
            .style(|t: &Theme, status| {
                let p = pal_of(t);
                button::Style {
                    background: Some(
                        theme::mix_color(
                            p.surface_container,
                            p.on_surface,
                            theme::state_alpha(status),
                        )
                        .into(),
                    ),
                    text_color: p.on_surface,
                    border: iced::Border {
                        color: p.outline_variant,
                        width: 1.0,
                        radius: theme::shape::MD.into(),
                    },
                    ..Default::default()
                }
            });
            content = content.push(resume);
        }
        content = content.push(log_card);
        if banners.is_empty() {
            content.into()
        } else {
            // Banners extend the page instead of squeezing the cards.
            iced::widget::scrollable(
                column![
                    iced::widget::Column::with_children(banners).spacing(14.0),
                    content.height(viewport_height),
                ]
                .spacing(14.0)
                .width(Length::Fill),
            )
            .height(Length::Fill)
            .width(Length::Fill)
            .style(m3_scrollable_style)
            .into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_detail_omits_unknown_android_version_without_a_separator() {
        assert_eq!(device_identity_detail("TB520FU", ""), "TB520FU");
        assert_eq!(
            device_identity_detail("TB520FU", "15"),
            "TB520FU · Android 15"
        );
    }

    #[test]
    fn dashboard_actions_follow_serial_and_firmware_requirements() {
        assert_eq!(
            dashboard_action_availability("", "TB320FC_ZUXOS", ""),
            DashboardActionAvailability {
                ota: false,
                qfil: false,
                info: false,
            }
        );
        assert_eq!(
            dashboard_action_availability("serial", "", ""),
            DashboardActionAvailability {
                ota: false,
                qfil: false,
                info: true,
            }
        );
        assert_eq!(
            dashboard_action_availability("serial", "TB320FC_ROW", "fallback_ZUXOS"),
            DashboardActionAvailability {
                ota: true,
                qfil: false,
                info: true,
            }
        );
        assert_eq!(
            dashboard_action_availability("serial", "", "fallback_zuxos"),
            DashboardActionAvailability {
                ota: true,
                qfil: true,
                info: true,
            }
        );
    }
}
