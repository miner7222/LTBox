//! Dashboard view (device status, action tiles). Extracted from `main.rs`.

use crate::*;
use iced::widget::{Space, button, column, container, row, text};
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
        // `Fill` height so the label centres in the 38px button instead of
        // resting against its top edge.
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
    /// Not gated on Windows even though the process it reports is. The confirm
    /// dialog behind it never was, so gating only the banner left
    /// `ForceCloseSoftwareFix` without a constructor off Windows, which
    /// `-D dead-code` rejects. Nothing here acts on its own: `poll_software_fix`
    /// and `can_close_software_fix` hold the runtime `cfg!(windows)` guards, so
    /// elsewhere `running` is only ever set by the demo scene.
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
                .size(theme::text_size::BODY_SMALL)
                .style(warning_container_text_style),
        ]
        .spacing(8.0)
        .width(Length::Fill);
        if let Some(key) = self.software_fix.error_key {
            body_copy = body_copy.push(
                text(self.t(key).to_string())
                    .size(theme::text_size::BODY_SMALL)
                    .style(warning_container_text_style),
            );
        }
        self.message_banner(
            BannerSeverity::Warning,
            icon::banner_warning(),
            self.t("dash_software_fix_title").to_string(),
            row![body_copy, action]
                .spacing(16.0)
                .align_y(iced::Alignment::Center)
                .width(Length::Fill),
        )
    }

    pub(crate) fn view_dashboard(&self) -> Element<'_, Message> {
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
        // The dashboard answers the policy question only. Committed rollback
        // floors are deliberately not rendered here, even when fastboot has
        // supplied them; the tooltip explains how that answer is established.
        let arb_key = arb_from_model(&self.device.model);
        let arb_display = if arb_key.is_empty() {
            "—".to_string()
        } else {
            self.t(arb_key).to_string()
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
        // Title + divider dropped — sidebar already labels the active view,
        // so the duplicate header was eating vertical space without telling
        // the user anything new. `height(Fill)` so the log card (the last
        // child) can claim the remaining vertical space — keeps the top +
        // bottom dashboard margins symmetric.
        let mut content = column![]
            .spacing(14.0)
            .width(Length::Fill)
            .height(Length::Fill);

        if self.software_fix.running {
            content = content.push(self.software_fix_banner());
        }

        // Unauthorized ADB wins over the platform warning — empty
        // `ro.boot.hardware` otherwise reads as "unsupported platform".
        if self.device.connection == ConnectionStatus::AdbServerBlocking {
            let msg = text(self.t("dash_adb_server_blocking").to_string())
                .size(theme::text_size::BODY_SMALL)
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
            content = content.push(
                self.message_banner(
                    BannerSeverity::Warning,
                    icon::banner_warning(),
                    self.t("banner_warning_title").to_string(),
                    row![msg, kill_btn]
                        .spacing(12.0)
                        .width(Length::Fill)
                        .align_y(iced::Alignment::Center),
                ),
            );
        } else if self.device.connection == ConnectionStatus::AdbUnauthorized {
            content = content.push(
                self.message_banner(
                    BannerSeverity::Warning,
                    icon::banner_warning(),
                    self.t("banner_warning_title").to_string(),
                    text(self.t("dash_adb_unauthorized").to_string())
                        .size(theme::text_size::BODY_SMALL)
                        .style(warning_container_text_style)
                        .width(Length::Fill),
                ),
            );
        } else if self.device.connection == ConnectionStatus::AdbSideload {
            content = content.push(
                self.message_banner(
                    BannerSeverity::Warning,
                    icon::banner_warning(),
                    self.t("banner_warning_title").to_string(),
                    text(self.t("dash_adb_sideload").to_string())
                        .size(theme::text_size::BODY_SMALL)
                        .style(warning_container_text_style)
                        .width(Length::Fill),
                ),
            );
        } else if self.device.platform_supported == Some(false) {
            content = content.push(
                self.message_banner(
                    BannerSeverity::Warning,
                    icon::banner_warning(),
                    self.t("banner_warning_title").to_string(),
                    text(self.t("dash_unsupported_platform").to_string())
                        .size(theme::text_size::BODY_SMALL)
                        .style(warning_container_text_style)
                        .width(Length::Fill),
                ),
            );
        }

        if let Some(banner) = self.driver_install_banner() {
            content = content.push(banner);
        }

        let mut identity = row![].spacing(16.0).align_y(iced::Alignment::Center);
        if !self.device.model.is_empty() {
            let portrait: Element<'_, Message> = match device_portrait(&self.device.model) {
                DevicePortrait::Png(handle) => iced::widget::image(handle)
                    .content_fit(iced::ContentFit::ScaleDown)
                    .into(),
                DevicePortrait::Svg(handle) => iced::widget::svg(handle)
                    .content_fit(iced::ContentFit::ScaleDown)
                    .into(),
            };
            identity = identity.push(
                // `center_x(Fill)` would overwrite the fixed width and let the
                // thumbnail slot absorb slack, indenting the image away from
                // the definition-label column below it.
                container(portrait)
                    .width(Length::Fixed(92.0))
                    .height(Length::Fixed(64.0))
                    .align_x(iced::alignment::Horizontal::Left)
                    .align_y(iced::alignment::Vertical::Center),
            );
        }
        let device_name = if self.device.market_name.is_empty() {
            if self.device.model.is_empty() {
                self.t("dash_device").to_string()
            } else {
                model.to_string()
            }
        } else {
            self.device.market_name.clone()
        };
        let identity_detail = device_identity_detail(model, &self.device.android_version);
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
        let label = |key: &'static str| -> Element<'_, Message> {
            text(self.t(key).to_string())
                .size(theme::text_size::LABEL_SMALL)
                .style(muted_style)
                .into()
        };
        let value = |value: String| -> Element<'_, Message> {
            text(value)
                .size(theme::text_size::BODY_MEDIUM)
                .font(theme::emphasis::medium())
                .wrapping(iced::widget::text::Wrapping::WordOrGlyph)
                .into()
        };

        let rollback_help_key = if matches!(
            self.device.model.to_ascii_uppercase().as_str(),
            "TB321FU" | "TB520FU"
        ) {
            "dash_rollback_help_fastboot"
        } else {
            "dash_rollback_help_edl"
        };
        // With no device attached the model is unknown, so the branch above
        // would present the EDL wording as if it had been determined. Show the
        // badge without a tooltip rather than answering for a device that is
        // not there.
        let rollback_help_text: Option<String> = (self.device.connection != ConnectionStatus::None)
            .then(|| self.t(rollback_help_key).to_string());
        let rollback_help = iced::widget::tooltip(
            container(text("?").size(11.0).style(muted_style))
                .padding([2, 6])
                .style(|t: &Theme| {
                    let p = pal_of(t);
                    container::Style {
                        background: Some(with_alpha(p.on_surface_variant, 0.10).into()),
                        border: iced::Border {
                            radius: theme::shape::SM.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }
                }),
            container(text(rollback_help_text.clone().unwrap_or_default()).size(11.0))
                .padding(if rollback_help_text.is_some() {
                    iced::Padding::from([6, 10])
                } else {
                    iced::Padding::ZERO
                })
                .max_width(320.0)
                .style(move |t: &Theme| {
                    if rollback_help_text.is_some() {
                        theme::tooltip_style(t, theme::shape::SM)
                    } else {
                        iced::widget::container::Style::default()
                    }
                }),
            iced::widget::tooltip::Position::Right,
        );
        let rollback_label: Element<'_, Message> = row![
            text(self.t("device_arb").to_string())
                .size(theme::text_size::LABEL_SMALL)
                .style(muted_style),
            rollback_help,
        ]
        .spacing(6.0)
        .align_y(iced::Alignment::Center)
        .into();

        let rollback_value: Element<'_, Message> = if self.device.rollback_floors.is_some() {
            iced::widget::tooltip(
                button(value(arb.to_string()))
                    .on_press(Message::RollbackDetailOpen)
                    .padding([4, 0])
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
        let actions = dashboard_action_availability(
            &self.device.serial,
            &self.device.firmware_full,
            &self.device.firmware,
        );
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
        let device_card_inner = column![
            identity,
            Space::new().height(12.0),
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
            dashboard_definition_row(label("device_firmware"), value(firmware.to_string()), false,),
            iced::widget::rule::horizontal(1).style(shell_rule_style),
            Space::new().height(14.0),
            quick_actions,
        ]
        .spacing(0)
        .width(Length::Fill);
        content = content.push(
            container(
                // Padding has to scale with the corner radius: M3 states
                // the relationship as `outer radius - padding = inner
                // radius`, so the 10/18 inset that suited a 12 px corner
                // leaves text crowding the curve at 32 px. A uniform 24
                // keeps a comfortable 8 px inner radius and reads evenly
                // on all four sides, which the old asymmetric values did
                // not.
                container(device_card_inner)
                    .padding(DEVICE_CARD_PADDING)
                    .width(Length::Fill),
            )
            .width(Length::Fill)
            .style(|t: &Theme| {
                // The hero uses a larger shape and the brightest surface
                // the mode offers, while keeping the same elevation-zero
                // outline treatment as the surrounding cards.
                //
                // Deliberately no type escalation — the connected device
                // is the subject of the whole app, but the card is mostly
                // reference data and shouting it was already tried and
                // rejected.
                theme::surface_card_style(t, theme::SurfaceLevel::Brightest, theme::shape::LG)
            }),
        );
        // Render from the source lines so the first entry stays at the top of
        // the history card. The shared editor tracks the document end for
        // execution views, which left a large blank band above short logs on
        // the dashboard.
        let dash_log = iced::widget::scrollable(
            text(self.log_lines.join("\n"))
                .size(11.0)
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
        // Only while an operation runs. Idle, this was a whole card spending
        // itself on "nothing in progress"; running, it is the only way back to
        // a flow the user navigated away from, so it cannot just be deleted.
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
        content.into()
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
