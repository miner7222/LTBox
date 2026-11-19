//! Settings view (appearance, device connection, and files).

use crate::focus_button::{self as button, button};
use crate::*;
use iced::widget::{self, Space, column, container, row, text};
use iced::{Element, Length, Theme};
use ltbox_core::tr_args;
use theme::with_alpha;

const SETTINGS_ROW_HEIGHT: f32 = 56.0;
const SETTINGS_CONTROL_HEIGHT: f32 = 40.0;

/// Material switch bound to `on_toggle(!selected)`. The message is a parameter
/// because more than one setting is a plain on/off toggle.
fn settings_switch(
    selected: bool,
    on_toggle: impl Fn(bool) -> Message,
) -> Element<'static, Message> {
    let thumb_size = if selected { 24.0 } else { 16.0 };
    let thumb = container(Space::new())
        .width(thumb_size)
        .height(thumb_size)
        .style(move |t: &Theme| container::Style {
            background: Some(
                if selected {
                    pal_of(t).on_primary
                } else {
                    pal_of(t).outline
                }
                .into(),
            ),
            border: iced::Border {
                radius: theme::shape::FULL.into(),
                ..Default::default()
            },
            ..Default::default()
        });
    let track = container(thumb)
        .width(52.0)
        .height(32.0)
        .padding(if selected { 4.0 } else { 8.0 })
        .align_x(if selected {
            iced::alignment::Horizontal::Right
        } else {
            iced::alignment::Horizontal::Left
        })
        .center_y(32.0)
        .style(move |t: &Theme| {
            let style = m3_settings_switch_style(
                t,
                widget::toggler::Status::Active {
                    is_toggled: selected,
                },
            );
            container::Style {
                background: Some(style.background),
                border: iced::Border {
                    color: style.background_border_color,
                    width: 2.0,
                    radius: theme::shape::FULL.into(),
                },
                ..Default::default()
            }
        });
    button(track)
        .padding(8)
        .height(48.0)
        .width(68.0)
        .on_press(on_toggle(!selected))
        .style(|t: &Theme, status| button::Style {
            background: theme::state_layer_bg(status, pal_of(t).on_surface).map(Into::into),
            border: iced::Border {
                radius: theme::shape::FULL.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

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
    let title = row![
        text(label.clone())
            .size(theme::text_size::BODY_MEDIUM)
            .line_height(20.0 / 14.0),
    ]
    .spacing(6.0)
    .align_y(iced::Alignment::Center);
    let mut copy = column![title].spacing(3.0).width(Length::Shrink);
    if !description.is_empty() {
        copy = copy.push(
            text(description)
                .size(theme::text_size::BODY_SMALL)
                .style(muted_style)
                .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
        );
    }

    // Keep the spacer beside the content: a Stack would cap the text height.
    let copy: Element<'static, Message> = match help {
        Some(help) => row![copy, help_button(label, help)]
            .spacing(6)
            .align_y(iced::Alignment::Center)
            .into(),
        None => copy.into(),
    };
    // The group hugs its copy; only its outer slot absorbs spare width.
    let contents = row![container(copy).width(Length::Fill), control]
        .spacing(20.0)
        .width(Length::Fill)
        .align_y(iced::Alignment::Center);
    container(
        row![contents, Space::new().height(SETTINGS_ROW_HEIGHT - 24.0),]
            .align_y(iced::Alignment::Center),
    )
    .padding([12.0, 18.0])
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
    let inner_radius = height / 2.0 - border_width;
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
        let label = text(label)
            .size(SETTINGS_SEGMENT_TEXT_SIZE)
            .font(theme::emphasis::medium())
            .wrapping(iced::widget::text::Wrapping::None);
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
                radius: theme::shape::FULL.into(),
            },
            ..Default::default()
        })
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
                radius: theme::button_radius(status).into(),
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
            radius: theme::button_radius(status).into(),
        },
        ..Default::default()
    }
}

fn settings_text_action(label: String, message: Option<Message>) -> Element<'static, Message> {
    let enabled = message.is_some();
    let content = container(
        text(label)
            .size(theme::text_size::BODY_MEDIUM)
            .font(theme::emphasis::medium())
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
    crate::focus_button::read_only(
        widget::text_input("", &value)
            .on_input(|_| Message::Noop)
            .size(12)
            .padding([12, 12])
            .width(SETTINGS_VALUE_FIELD_WIDTH)
            .style(m3_text_input_style),
    )
}

impl App {
    pub(crate) fn view_settings(&self) -> Element<'_, Message> {
        let s = &self.settings;
        let field_padding = M3_FIELD_PADDING;
        let grid_max_width = SETTINGS_GRID_MAX_WIDTH.max(SETTINGS_PANEL_MAX_WIDTH);

        let language_picker = widget::pick_list(
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
        .width(Length::Fixed(SETTINGS_PICK_LIST_WIDTH));
        let language_control = focus_button::cycle(
            language_picker,
            "settings-language".into(),
            LANGUAGES,
            &s.language,
            |language| Message::Settings(SettingsMsg::SetLanguage(language)),
        );
        let language_row = settings_row(
            self.t("settings_language").to_string(),
            String::new(),
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

        let font_control = settings_switch(self.use_system_font, |v| {
            Message::Settings(SettingsMsg::SetUseSystemFont(v))
        });
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
                    Message::Settings(SettingsMsg::RequestQcomDriverMode(mode)),
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

        let remember_loader_control = settings_switch(self.remember_edl_loader, |v| {
            Message::Settings(SettingsMsg::SetRememberEdlLoader(v))
        });
        let remember_loader_row = settings_row_with_help(
            self.t("settings_remember_loader").to_string(),
            self.t("settings_remember_loader_desc").to_string(),
            Some(self.t("settings_remember_loader_help").to_string()),
            remember_loader_control,
        );
        let device_card = settings_card(
            self.t("settings_device_connection_title").to_string(),
            vec![driver_row, remember_loader_row],
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
        let cleanup_label = if self.cleaning_temp {
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
        let cleanup_control = settings_text_action(cleanup_label, cleanup_message);
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
