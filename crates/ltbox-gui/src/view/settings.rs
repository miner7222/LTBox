//! Settings view (appearance, device connection, and files).

use crate::*;
use iced::widget::{self, Space, button, column, container, row, text};
use iced::{Element, Length, Theme};
use ltbox_core::tr_args;
use theme::with_alpha;

const SETTINGS_ROW_HEIGHT: f32 = 56.0;
const SETTINGS_CONTROL_HEIGHT: f32 = 36.0;

fn settings_row(
    label: String,
    description: String,
    control: Element<'static, Message>,
) -> Element<'static, Message> {
    settings_row_with_help(label, description, None, control)
}

fn settings_row_with_help(
    label: String,
    description: String,
    help: Option<String>,
    control: Element<'static, Message>,
) -> Element<'static, Message> {
    let mut title = row![
        text(label)
            .size(theme::text_size::BODY_MEDIUM)
            .line_height(1.0),
    ]
    .spacing(6.0)
    .align_y(iced::Alignment::Center);
    if let Some(help) = help {
        title = title.push(settings_help(help));
    }
    let mut copy = column![title].spacing(3.0).width(Length::Fill);
    if !description.is_empty() {
        copy = copy.push(
            text(description)
                .size(theme::text_size::BODY_SMALL)
                .style(muted_style)
                .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
        );
    }

    // `Fill` height so the row centres inside the minimum-height spacer the
    // stack below establishes. Left at Shrink it sat at the spacer's top edge,
    // which showed up as a bigger gap under the last row of every card.
    let contents = row![copy, control]
        .spacing(20.0)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_y(iced::Alignment::Center);
    container(iced::widget::stack![
        Space::new()
            .width(Length::Fill)
            .height(Length::Fixed(SETTINGS_ROW_HEIGHT - 16.0)),
        contents,
    ])
    .padding([8.0, 18.0])
    .width(Length::Fill)
    .align_y(iced::alignment::Vertical::Center)
    .into()
}

fn settings_card(title: String, rows: Vec<Element<'static, Message>>) -> Element<'static, Message> {
    let header = container(
        text(title)
            .size(theme::text_size::TITLE_MEDIUM)
            .font(theme::emphasis::medium()),
    )
    .padding([14.0, 18.0])
    .width(Length::Fill);
    let mut contents = column![header, widget::rule::horizontal(1).style(shell_rule_style)];
    let last = rows.len().saturating_sub(1);
    for (index, settings_row) in rows.into_iter().enumerate() {
        contents = contents.push(settings_row);
        if index != last {
            contents = contents.push(widget::rule::horizontal(1).style(shell_rule_style));
        }
    }

    container(contents.width(Length::Fill))
        .width(Length::Fill)
        .style(|t: &Theme| {
            let p = pal_of(t);
            container::Style {
                background: Some(p.surface_container.into()),
                border: iced::Border {
                    color: p.outline_variant,
                    width: 1.0,
                    radius: theme::shape::MD.into(),
                },
                ..Default::default()
            }
        })
        .into()
}

fn segment_divider_style(t: &Theme, enabled: bool) -> widget::rule::Style {
    let p = pal_of(t);
    widget::rule::Style {
        color: if enabled {
            p.outline
        } else {
            with_alpha(p.on_surface, 0.12)
        },
        radius: 0.0.into(),
        fill_mode: widget::rule::FillMode::Full,
        snap: true,
    }
}

fn settings_segmented_control(
    options: Vec<(String, bool, Message)>,
    enabled: bool,
) -> Element<'static, Message> {
    let height = SETTINGS_CONTROL_HEIGHT;
    let border_width = 1.0;
    let segment_height = Length::Fixed(height - 2.0 * border_width);
    let inner_radius = theme::shape::SM - border_width;
    let segment_count = options.len();
    let mut segments = row![].height(segment_height);
    for (index, (label, selected, message)) in options.into_iter().enumerate() {
        let radius = iced::border::Radius {
            top_left: if index == 0 { inner_radius } else { 0.0 },
            bottom_left: if index == 0 { inner_radius } else { 0.0 },
            top_right: if index + 1 == segment_count {
                inner_radius
            } else {
                0.0
            },
            bottom_right: if index + 1 == segment_count {
                inner_radius
            } else {
                0.0
            },
        };
        if index > 0 {
            segments = segments.push(
                widget::rule::vertical(1)
                    .style(move |theme: &Theme| segment_divider_style(theme, enabled)),
            );
        }
        let mut label = text(label)
            .size(SETTINGS_SEGMENT_TEXT_SIZE)
            .wrapping(iced::widget::text::Wrapping::None);
        if selected {
            label = label.font(theme::emphasis::medium());
        }
        let cell = container(label)
            .height(segment_height)
            .padding([0.0, SETTINGS_SEGMENT_HORIZONTAL_PADDING])
            .align_x(iced::alignment::Horizontal::Center)
            .align_y(iced::alignment::Vertical::Center);
        segments = segments.push(
            button(cell)
                .on_press_maybe(enabled.then_some(message))
                .padding(0)
                .height(segment_height)
                .style(move |t: &Theme, status| {
                    m3_segment_button_style(t, status, selected, radius)
                }),
        );
    }

    container(segments)
        .height(Length::Fixed(height))
        .padding(border_width)
        .style(move |t: &Theme| container::Style {
            border: iced::Border {
                color: if enabled {
                    pal_of(t).outline
                } else {
                    with_alpha(pal_of(t).on_surface, 0.12)
                },
                width: border_width,
                radius: theme::shape::SM.into(),
            },
            ..Default::default()
        })
        .into()
}

fn settings_help(tip: String) -> Element<'static, Message> {
    widget::tooltip(
        button(text("?").size(11.0))
            .padding([2, 6])
            .on_press(Message::ToastShow(tip.clone()))
            .style(|t: &Theme, status| {
                let p = pal_of(t);
                button::Style {
                    background: theme::state_layer_bg(status, p.on_surface_variant).map(Into::into),
                    text_color: p.on_surface_variant,
                    border: iced::Border {
                        radius: theme::shape::SM.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }
            }),
        container(text(tip).size(11.0))
            .padding([6, 10])
            .max_width(280.0)
            .style(|t: &Theme| theme::tooltip_style(t, theme::shape::SM)),
        widget::tooltip::Position::Top,
    )
    .gap(6.0)
    .into()
}

fn settings_action_style(
    t: &Theme,
    status: button::Status,
    enabled: bool,
    error_role: bool,
) -> button::Style {
    let p = pal_of(t);
    if !enabled || matches!(status, button::Status::Disabled) {
        return button::Style {
            background: None,
            text_color: with_alpha(p.on_surface, 0.38),
            border: iced::Border {
                color: with_alpha(p.on_surface, 0.12),
                width: 1.0,
                radius: theme::shape::SM.into(),
            },
            ..Default::default()
        };
    }
    let foreground = if error_role {
        p.error
    } else {
        p.on_surface_variant
    };
    button::Style {
        background: theme::state_layer_bg(status, foreground).map(Into::into),
        text_color: foreground,
        border: iced::Border {
            color: if matches!(status, button::Status::Hovered | button::Status::Pressed) {
                foreground
            } else {
                p.outline
            },
            width: 1.0,
            radius: theme::shape::SM.into(),
        },
        ..Default::default()
    }
}

fn settings_icon_action(
    glyph: iced::widget::Text<'static, Theme, iced::Renderer>,
    tip: String,
    message: Option<Message>,
    error_role: bool,
) -> Element<'static, Message> {
    let enabled = message.is_some();
    let side = Length::Fixed(SETTINGS_CONTROL_HEIGHT);
    let icon = lucide_icon(glyph, 18.0, move |t: &Theme| {
        let p = pal_of(t);
        if !enabled {
            with_alpha(p.on_surface, 0.38)
        } else if error_role {
            p.error
        } else {
            p.on_surface_variant
        }
    });
    let mut action = button(
        container(icon)
            .width(side)
            .height(side)
            .align_x(iced::alignment::Horizontal::Center)
            .align_y(iced::alignment::Vertical::Center),
    )
    .padding(0)
    .width(side)
    .height(side)
    .style(move |t: &Theme, status| settings_action_style(t, status, enabled, error_role));
    if let Some(message) = message {
        action = action.on_press(message);
    }
    widget::tooltip(
        action,
        container(text(tip).size(11.0))
            .padding([6, 10])
            .style(|t: &Theme| theme::tooltip_style(t, theme::shape::XS)),
        widget::tooltip::Position::Top,
    )
    .gap(6.0)
    .into()
}

fn settings_text_action(label: String, message: Option<Message>) -> Element<'static, Message> {
    let enabled = message.is_some();
    let content = container(
        text(label)
            .size(theme::text_size::BODY_MEDIUM)
            .wrapping(iced::widget::text::Wrapping::None),
    )
    .height(Length::Fixed(SETTINGS_CONTROL_HEIGHT))
    .padding([0.0, 16.0])
    .align_y(iced::alignment::Vertical::Center);
    let mut action = button(content)
        .padding(0)
        .height(Length::Fixed(SETTINGS_CONTROL_HEIGHT))
        .style(move |t: &Theme, status| settings_action_style(t, status, enabled, false));
    if let Some(message) = message {
        action = action.on_press(message);
    }
    action.into()
}

fn settings_value_field(value: String) -> Element<'static, Message> {
    let shown = value.clone();
    let field = container(
        text(shown)
            .size(12.0)
            .width(Length::Fill)
            .wrapping(iced::widget::text::Wrapping::None),
    )
    .padding([0.0, 12.0])
    .width(Length::Fixed(SETTINGS_VALUE_FIELD_WIDTH))
    .height(Length::Fixed(SETTINGS_CONTROL_HEIGHT))
    .align_y(iced::alignment::Vertical::Center)
    .clip(true)
    .style(|t: &Theme| container::Style {
        border: iced::Border {
            color: pal_of(t).outline,
            width: 1.0,
            radius: theme::shape::SM.into(),
        },
        ..Default::default()
    });
    widget::tooltip(
        field,
        container(
            text(value)
                .size(11.0)
                .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
        )
        .padding([6, 10])
        .max_width(360.0)
        .style(|t: &Theme| theme::tooltip_style(t, theme::shape::XS)),
        widget::tooltip::Position::Top,
    )
    .gap(6.0)
    .into()
}

impl App {
    pub(crate) fn view_settings(&self) -> Element<'_, Message> {
        let s = &self.settings;
        let field_padding = M3_FIELD_PADDING;
        let grid_max_width = SETTINGS_GRID_MAX_WIDTH.max(SETTINGS_PANEL_MAX_WIDTH);

        let language_control: Element<'static, Message> = widget::pick_list(
            LANGUAGES
                .iter()
                .map(|language| language.label())
                .collect::<Vec<_>>(),
            Some(s.language.label()),
            |selected| {
                let language = LANGUAGES
                    .iter()
                    .find(|language| language.label() == selected)
                    .copied()
                    .unwrap_or(Language::En);
                Message::Settings(SettingsMsg::SetLanguage(language))
            },
        )
        .text_size(SETTINGS_PICK_LIST_TEXT_SIZE)
        .padding(field_padding)
        .style(m3_pick_list_style)
        .menu_style(m3_pick_list_menu_style)
        .width(Length::Fixed(SETTINGS_PICK_LIST_WIDTH))
        .into();
        let language_row = settings_row(
            self.t("settings_language").to_string(),
            self.t("settings_language_desc").to_string(),
            language_control,
        );

        let theme_control = settings_segmented_control(
            [ThemeChoice::System, ThemeChoice::Light, ThemeChoice::Dark]
                .into_iter()
                .map(|choice| {
                    (
                        self.t(choice.label_key()).to_string(),
                        self.theme_choice == choice,
                        Message::SetTheme(choice),
                    )
                })
                .collect(),
            true,
        );
        let theme_row = settings_row(
            self.t("settings_theme").to_string(),
            String::new(),
            theme_control,
        );

        let seed_control = settings_segmented_control(
            [ThemeSeed::Indigo, ThemeSeed::Teal, ThemeSeed::Rose]
                .into_iter()
                .map(|seed| {
                    (
                        self.t(seed.label_key()).to_string(),
                        self.theme_seed == seed,
                        Message::Settings(SettingsMsg::SetThemeSeed(seed)),
                    )
                })
                .collect(),
            true,
        );
        let seed_row = settings_row(
            self.t("settings_theme_seed").to_string(),
            self.t("settings_theme_seed_desc").to_string(),
            seed_control,
        );

        let font_control: Element<'static, Message> = widget::toggler(self.use_system_font)
            .on_toggle(|enabled| Message::Settings(SettingsMsg::SetUseSystemFont(enabled)))
            .size(24.0)
            .style(m3_settings_switch_style)
            .into();
        let font_row = settings_row(
            self.t("settings_system_font").to_string(),
            self.t("settings_system_font_desc").to_string(),
            font_control,
        );
        let appearance_card = settings_card(
            self.t("settings_appearance_title").to_string(),
            vec![language_row, theme_row, seed_row, font_row],
        );

        let driver_userspace = self.t("settings_qcom_driver_mode_userspace").to_string();
        let driver_kernel = self.t("settings_qcom_driver_mode_kernel").to_string();
        let kernel_mode_supported = ltbox_device::driver::kernel_mode_supported();
        let driver_help_key = if cfg!(target_os = "macos") {
            "settings_qcom_driver_mode_macos"
        } else if !kernel_mode_supported {
            "settings_qcom_driver_mode_linux_unsupported"
        } else {
            "settings_qcom_driver_mode_help"
        };
        let driver_control = settings_segmented_control(
            [
                (driver_kernel, ltbox_device::driver::QcomDriverMode::Kernel),
                (
                    driver_userspace,
                    ltbox_device::driver::QcomDriverMode::Userspace,
                ),
            ]
            .into_iter()
            .map(|(label, mode)| {
                (
                    label,
                    self.qcom_driver_mode == mode,
                    Message::Settings(SettingsMsg::SetQcomDriverMode(mode)),
                )
            })
            .collect(),
            !self.operation.is_running() && kernel_mode_supported,
        );
        let driver_row = settings_row_with_help(
            self.t("settings_qcom_driver_mode").to_string(),
            self.t("settings_qcom_driver_mode_desc").to_string(),
            Some(self.t(driver_help_key).to_string()),
            driver_control,
        );

        let default_loader_value = self
            .default_loader_path
            .clone()
            .unwrap_or_else(|| self.t("settings_default_loader_unset").to_string());
        let default_loader_control = row![
            settings_value_field(default_loader_value),
            settings_icon_action(
                icon::fab_open_folder(),
                self.t("settings_default_loader_browse").to_string(),
                Some(Message::Settings(SettingsMsg::SettingsPickDefaultLoader)),
                false,
            ),
            settings_icon_action(
                icon::settings_clear(),
                self.t("settings_default_loader_clear").to_string(),
                self.default_loader_path
                    .is_some()
                    .then_some(Message::Settings(SettingsMsg::SettingsClearDefaultLoader)),
                true,
            ),
        ]
        .spacing(8.0)
        .align_y(iced::Alignment::Center);
        let default_loader_row = settings_row_with_help(
            self.t("settings_default_loader").to_string(),
            self.t("settings_default_loader_desc").to_string(),
            Some(self.t("settings_default_loader_help").to_string()),
            default_loader_control.into(),
        );
        let device_card = settings_card(
            self.t("settings_device_connection_title").to_string(),
            vec![driver_row, default_loader_row],
        );

        let backup_path = ltbox_core::app_paths::backup_root().display().to_string();
        let backup_control: Element<'static, Message> = row![
            settings_value_field(backup_path),
            settings_text_action(
                self.t("settings_backup_folder_open").to_string(),
                Some(Message::Settings(SettingsMsg::OpenBackupFolder)),
            ),
        ]
        .spacing(8.0)
        .align_y(iced::Alignment::Center)
        .into();
        let backup_row = settings_row(
            self.t("settings_backup_folder").to_string(),
            self.t("settings_backup_folder_desc").to_string(),
            backup_control,
        );

        let cleanup_enabled = !self.operation.is_running()
            && !self.cleaning_temp
            && matches!(self.temp_files_bytes, Some(bytes) if bytes > 0);
        let cleanup_tip = if self.cleaning_temp {
            self.t("settings_cleanup_busy").to_string()
        } else {
            self.t("settings_cleanup_button").to_string()
        };
        let cleanup_size = self
            .temp_files_bytes
            .map(format_bytes_auto)
            .unwrap_or_else(|| "…".to_string());
        let cleanup_description = tr_args!("settings_cleanup_desc", size = cleanup_size);
        let cleanup_message =
            cleanup_enabled.then_some(Message::Settings(SettingsMsg::CleanupTempFiles));
        let cleanup_control =
            settings_icon_action(icon::settings_clear(), cleanup_tip, cleanup_message, true);
        let cleanup_row = settings_row_with_help(
            self.t("settings_cleanup").to_string(),
            cleanup_description,
            Some(self.t("settings_cleanup_help").to_string()),
            cleanup_control,
        );
        let files_card = settings_card(
            self.t("settings_files_title").to_string(),
            vec![backup_row, cleanup_row],
        );

        let mut cards = column![].spacing(18.0).width(Length::Fill);
        if let Some(banner) = self.driver_install_banner() {
            cards = cards.push(centered_max_width(banner, grid_max_width));
        }
        cards = cards.push(centered_max_width(appearance_card, grid_max_width));
        cards = cards.push(centered_max_width(device_card, grid_max_width));
        cards = cards.push(centered_max_width(files_card, grid_max_width));

        let body = widget::scrollable(container(cards).padding(24.0).width(Length::Fill))
            .style(m3_scrollable_style)
            .width(Length::Fill)
            .height(Length::Fill);

        column![
            large_top_app_bar(self.t("settings_title").to_string(), None),
            body,
        ]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }
}
