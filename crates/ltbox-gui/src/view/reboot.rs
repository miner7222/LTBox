//! Reboot view + confirm popup. Extracted from `main.rs`.

use crate::*;
use iced::widget::{Space, button, column, container, row, text};
use iced::{Element, Length, Theme};
use ltbox_core::tr_args;
use theme::with_alpha;

const REBOOT_CONFIRM_LABEL_WIDTH: f32 = 104.0;
const REBOOT_CHECKLIST_MARKER_SIZE: f32 = 20.0;
const REBOOT_CHECKLIST_ROW_HEIGHT: f32 = 38.0;

fn reboot_disabled_icon(target: RebootTarget, size: f32) -> Element<'static, Message> {
    let glyph = match target {
        RebootTarget::System => icon::reboot_system(),
        RebootTarget::Recovery => icon::reboot_recovery(),
        RebootTarget::Bootloader => icon::reboot_bootloader(),
        RebootTarget::Fastbootd => icon::reboot_fastbootd(),
        RebootTarget::Edl => icon::reboot_edl(),
    };
    lucide_list_disabled(glyph, size)
}

fn reboot_status_pill(copy: String) -> Element<'static, Message> {
    container(
        text(copy)
            .size(theme::text_size::LABEL_SMALL)
            .style(muted_style)
            .wrapping(iced::widget::text::Wrapping::None),
    )
    .height(Length::Fixed(22.0))
    .padding([0, 9])
    .align_y(iced::alignment::Vertical::Center)
    .style(|t: &Theme| container::Style {
        background: Some(pal_of(t).surface_container_high.into()),
        border: iced::Border {
            radius: theme::shape::FULL.into(),
            ..Default::default()
        },
        ..Default::default()
    })
    .into()
}

fn reboot_definition_row(label: String, value: String) -> Element<'static, Message> {
    row![
        container(
            text(label)
                .size(theme::text_size::BODY_SMALL)
                .style(muted_style)
                .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
        )
        .width(Length::Fixed(REBOOT_CONFIRM_LABEL_WIDTH)),
        text(value)
            .size(theme::text_size::BODY_SMALL)
            .font(theme::emphasis::medium())
            .width(Length::Fill)
            .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
    ]
    .spacing(16)
    .align_y(iced::Alignment::Start)
    .width(Length::Fill)
    .padding([9, 0])
    .into()
}

fn reboot_wait_marker(completed: bool) -> Element<'static, Message> {
    let copy = if completed { "\u{2713}" } else { "2" };
    container(
        text(copy)
            .size(theme::text_size::LABEL_SMALL)
            .font(theme::emphasis::medium())
            .style(move |t: &Theme| iced::widget::text::Style {
                color: Some(if completed {
                    pal_of(t).on_primary
                } else {
                    pal_of(t).primary
                }),
            }),
    )
    .width(Length::Fixed(REBOOT_CHECKLIST_MARKER_SIZE))
    .height(Length::Fixed(REBOOT_CHECKLIST_MARKER_SIZE))
    .align_x(iced::alignment::Horizontal::Center)
    .align_y(iced::alignment::Vertical::Center)
    .style(move |t: &Theme| {
        let p = pal_of(t);
        container::Style {
            background: completed.then_some(p.primary.into()),
            border: iced::Border {
                color: p.primary,
                width: 1.5,
                radius: theme::shape::FULL.into(),
            },
            ..Default::default()
        }
    })
    .into()
}

fn reboot_wait_checklist_row(
    label: String,
    status: String,
    completed: bool,
) -> Element<'static, Message> {
    let mut label = text(label)
        .size(theme::text_size::BODY_SMALL)
        .width(Length::Fill)
        .wrapping(iced::widget::text::Wrapping::WordOrGlyph);
    if !completed {
        label = label.font(theme::emphasis::medium());
    }
    row![
        reboot_wait_marker(completed),
        label,
        text(status)
            .size(theme::text_size::BODY_SMALL)
            .style(muted_style)
            .wrapping(iced::widget::text::Wrapping::None),
    ]
    .spacing(12)
    .height(Length::Fixed(REBOOT_CHECKLIST_ROW_HEIGHT))
    .align_y(iced::Alignment::Center)
    .width(Length::Fill)
    .into()
}

fn format_reboot_wait_elapsed(duration: std::time::Duration) -> String {
    let seconds = duration.as_secs();
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

impl App {
    pub(crate) fn view_reboot(&self) -> Element<'_, Message> {
        let conn = self.device.connection;
        let icon_size = self.wizard_list_icon(WIZARD_LIST_GLYPH_ICON_SIZE);
        let (label_size, _) =
            self.wizard_list_text(theme::text_size::BODY_MEDIUM, theme::text_size::BODY_SMALL);
        let row_height = self.wizard_list_row_height();
        let current_transport = self.t(self.connection_label_key()).to_string();
        // With nothing attached every row would carry the same reason, which
        // tells the rows apart from nothing. The context line above already
        // says it once.
        let show_reasons = conn != ConnectionStatus::None;
        let context = row![
            text(self.t("reboot_current_connection").to_string())
                .size(theme::text_size::BODY_SMALL)
                .style(muted_style),
            reboot_status_pill(current_transport.clone()),
        ]
        .spacing(8.0)
        .align_y(iced::Alignment::Center);

        let mut cards = column![].spacing(8.0).width(Length::Fill);
        for &target in RebootTarget::all().iter() {
            let available = target.available_from(conn);
            let current = target.is_current_from(conn, self.device.fastboot_userspace);
            let enabled = available && !current;
            let label = self.t(target.label_key()).to_string();
            let label_style = if enabled {
                on_surface_style
            } else {
                |t: &Theme| iced::widget::text::Style {
                    color: Some(with_alpha(pal_of(t).on_surface, 0.38)),
                }
            };
            let icon = if enabled {
                target.icon(icon_size)
            } else {
                reboot_disabled_icon(target, icon_size)
            };
            let label_text = text(label)
                .size(label_size)
                .style(label_style)
                .width(Length::Fill);
            let mut card_content = row![icon_tile(icon), label_text,]
                .spacing(12.0)
                .align_y(iced::Alignment::Center);
            if !enabled && show_reasons {
                let reason = if current {
                    self.t("reboot_reason_current_state").to_string()
                } else {
                    tr_args!(
                        "reboot_reason_unavailable_in",
                        transport = current_transport.as_str()
                    )
                };
                card_content = card_content.push(reboot_status_pill(reason));
            }
            let card_inner = container(card_content)
                .padding([10.0, 14.0])
                .width(Length::Fill)
                .height(Length::Fixed(row_height))
                .center_y(Length::Fixed(row_height))
                .style(move |t: &Theme| {
                    let p = pal_of(t);
                    if enabled {
                        sel_card_style(t, false)
                    } else {
                        container::Style {
                            background: Some(with_alpha(p.on_surface, 0.04).into()),
                            border: iced::Border {
                                color: p.outline_variant,
                                width: 1.0,
                                radius: theme::shape::MD.into(),
                            },
                            ..Default::default()
                        }
                    }
                });
            let btn: Element<'_, Message> = if enabled {
                button(card_inner)
                    .on_press(Message::Reboot(RebootMsg::RebootRequest(target)))
                    .padding(0)
                    .width(Length::Fill)
                    .style(|t: &Theme, status| sel_card_btn_style(t, status, false))
                    .into()
            } else {
                card_inner.into()
            };
            cards = cards.push(btn);
        }

        // Same inset as the root list so both single-column lists sit on the
        // same margins. The context line stays adjacent to the availability it
        // explains instead of relying on the distant status bar.
        let list = column![context, cards]
            .spacing(14.0)
            .padding([20.0, 28.0])
            .width(Length::Fill);

        let body: Element<'_, Message> = match self.window_size_class() {
            WindowSizeClass::Expanded => self.wizard_picker_step(String::new(), list.into()),
            WindowSizeClass::Compact => container(centered_step(
                list,
                self.wizard_list_max_width(WIZARD_LIST_MAX_WIDTH),
            ))
            .padding(24.0)
            .width(Length::Fill)
            .height(Length::Fill)
            .into(),
        };

        column![
            large_top_app_bar(self.t("reboot_title").to_string(), None),
            body,
        ]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    /// M3 confirm dialog for the Reboot panel.
    pub(crate) fn reboot_confirm_popup(&self, target: RebootTarget) -> Element<'_, Message> {
        let short = self.t(target.short_name_key()).to_string();
        let title = if target == RebootTarget::Edl {
            self.t("reboot_confirm_edl_title").to_string()
        } else {
            tr_args!("reboot_confirm_title", target = short.as_str())
        };
        let header: Element<'_, Message> = text(title).size(theme::text_size::TITLE_LARGE).into();
        let source = self.t(self.connection_label_key());
        let transition = format!("{source} \u{2192} {short}");
        let dash = "\u{2014}";
        let device_name = if self.device.market_name.is_empty() {
            dash
        } else {
            self.device.market_name.as_str()
        };
        let device_model = if self.device.model.is_empty() {
            dash
        } else {
            self.device.model.as_str()
        };
        let definition = column![
            reboot_definition_row(self.t("reboot_confirm_transition").to_string(), transition,),
            iced::widget::rule::horizontal(1).style(shell_rule_style),
            reboot_definition_row(
                self.t("dash_device").to_string(),
                format!("{device_name} \u{b7} {device_model}"),
            ),
        ]
        .spacing(0)
        .width(Length::Fill);
        let body: Element<'_, Message> = if target == RebootTarget::Edl {
            let warning = self.message_banner(
                BannerSeverity::Warning,
                icon::banner_warning(),
                self.t("reboot_confirm_warning_title").to_string(),
                text(self.t("reboot_confirm_warning_body").to_string())
                    .size(theme::text_size::BODY_SMALL)
                    .style(warning_container_text_style)
                    .width(Length::Fill)
                    .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
            );
            column![definition, warning]
                .spacing(14.0)
                .width(Length::Fill)
                .into()
        } else {
            definition.into()
        };
        let footer: Element<'_, Message> = row![
            Space::new().width(Length::Fill),
            m3_outlined_button(self.t("btn_cancel").to_string())
                .on_press(Message::Reboot(RebootMsg::RebootDismiss)),
            {
                // Mid-popup disconnect → drop the on_press so the confirm
                // button cannot fire a worker on a vanished transport.
                let mut b = m3_filled_button(self.t("btn_reboot_confirm").to_string());
                if target.available_from(self.device.connection)
                    && !target
                        .is_current_from(self.device.connection, self.device.fastboot_userspace)
                {
                    b = b.on_press(Message::Reboot(RebootMsg::RebootConfirm));
                }
                b
            },
        ]
        .spacing(10)
        .align_y(iced::Alignment::Center)
        .into();
        m3_dialog(dialog_sections(
            header,
            body,
            footer,
            theme::DIALOG_WIDTH_SM,
            false,
        ))
    }

    /// Modeless ADB/Fastboot-to-EDL wait dialog. Closing it only hides this
    /// surface; the tracked blocking operation continues until port detection
    /// completes or times out.
    pub(crate) fn reboot_wait_popup(&self) -> Element<'_, Message> {
        let header: Element<'_, Message> = column![
            text(self.t("reboot_wait_title").to_string()).size(theme::text_size::TITLE_LARGE),
            text(self.t("reboot_wait_body").to_string())
                .size(theme::text_size::BODY_SMALL)
                .style(muted_style)
                .width(Length::Fill)
                .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
        ]
        .spacing(3)
        .width(Length::Fill)
        .into();
        let elapsed = format_reboot_wait_elapsed(self.operation.elapsed());
        let body: Element<'_, Message> = column![
            reboot_wait_checklist_row(
                self.t("reboot_wait_command_sent").to_string(),
                self.t("reboot_wait_complete").to_string(),
                true,
            ),
            reboot_wait_checklist_row(
                self.t("reboot_wait_detect_port").to_string(),
                tr_args!("reboot_wait_status", elapsed = elapsed),
                false,
            ),
        ]
        .spacing(0)
        .width(Length::Fill)
        .into();
        let footer: Element<'_, Message> = row![
            text(self.t("reboot_wait_hint").to_string())
                .size(theme::text_size::BODY_SMALL)
                .style(muted_style)
                .width(Length::Fill)
                .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
            m3_outlined_button(self.t("btn_close").to_string())
                .on_press(Message::RebootWaitDismiss),
        ]
        .spacing(10)
        .align_y(iced::Alignment::Center)
        .width(Length::Fill)
        .into();

        m3_dialog_modeless(dialog_sections(
            header,
            body,
            footer,
            theme::DIALOG_WIDTH_SM,
            false,
        ))
    }
}
