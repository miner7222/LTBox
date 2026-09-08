//! Partition + physical-storage dump/flash wizard views + steps. Extracted from `main.rs`.

use crate::*;
use iced::widget::{self, Space, button, column, container, row, scrollable, text};
use iced::{Element, Length, Theme};
use ltbox_core::tr_args;

const FLASH_PARTS_LUN_COLUMN_WIDTH: f32 = 44.0;
const FLASH_PARTS_LABEL_COLUMN_WIDTH: f32 = 104.0;
const FLASH_PARTS_START_COLUMN_WIDTH: f32 = 116.0;
const FLASH_PARTS_SIZE_COLUMN_WIDTH: f32 = 96.0;
const FLASH_PARTS_STATE_COLUMN_WIDTH: f32 = 72.0;
const FLASH_PARTS_FILE_ACTION_SIZE: f32 = 28.0;
const FLASH_PARTS_ROW_HEIGHT: f32 = 40.0;

/// Numeric partition header with the same sort affordance as
/// `parts_sort_header`, but aligned to the trailing edge of its cell.
fn parts_numeric_sort_header(
    label: String,
    is_active: bool,
    desc: bool,
    width: Length,
    msg: Message,
) -> Element<'static, Message> {
    button(
        container(
            row![
                text(label).size(11).style(muted_style),
                parts_sort_marker(is_active, desc),
            ]
            .spacing(4)
            .align_y(iced::Alignment::Center),
        )
        .width(Length::Fill)
        .align_x(iced::alignment::Horizontal::Right),
    )
    .padding(0)
    .width(width)
    .style(|_t: &Theme, _s| button::Style {
        background: None,
        ..Default::default()
    })
    .on_press(msg)
    .into()
}

fn m3_erase_marker() -> Element<'static, Message> {
    // Square badge, so both axes ride the one factor its side does.
    let dash = container(Space::new())
        .width(Length::Fixed(FLASH_PARTS_ERASE_DASH_WIDTH))
        .height(Length::Fixed(FLASH_PARTS_ERASE_DASH_HEIGHT))
        .style(|t: &Theme| {
            let p = pal_of(t);
            container::Style {
                background: Some(p.on_error.into()),
                border: iced::Border {
                    radius: theme::shape::FULL.into(),
                    ..Default::default()
                },
                ..Default::default()
            }
        });

    container(dash)
        .width(Length::Fixed(FLASH_PARTS_MARKER_SIZE))
        .height(Length::Fixed(FLASH_PARTS_MARKER_SIZE))
        .align_x(iced::alignment::Horizontal::Center)
        .align_y(iced::alignment::Vertical::Center)
        .style(|t: &Theme| {
            let p = pal_of(t);
            container::Style {
                background: Some(p.error.into()),
                border: iced::Border {
                    color: p.error,
                    width: 2.0,
                    radius: theme::shape::XS.into(),
                },
                ..Default::default()
            }
        })
        .into()
}

fn partition_file_button(
    glyph: iced::widget::Text<'static, Theme, iced::Renderer>,
    on_press: Option<Message>,
    clear: bool,
) -> iced::widget::Button<'static, Message> {
    let enabled = on_press.is_some();
    let mut action = button(
        container(glyph.size(15))
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill),
    )
    .padding(0)
    .width(Length::Fixed(FLASH_PARTS_FILE_ACTION_SIZE))
    .height(Length::Fixed(FLASH_PARTS_FILE_ACTION_SIZE))
    .style(move |t: &Theme, status| {
        let p = pal_of(t);
        let destructive_hover =
            clear && matches!(status, button::Status::Hovered | button::Status::Pressed);
        let foreground = if !enabled {
            with_alpha(p.on_surface, 0.34)
        } else if destructive_hover {
            p.error
        } else {
            p.on_surface_variant
        };
        let background = if !enabled {
            None
        } else if destructive_hover {
            Some(with_alpha(p.error, 0.10).into())
        } else {
            theme::state_layer_bg(status, p.on_surface).map(Into::into)
        };
        button::Style {
            background,
            text_color: foreground,
            border: iced::Border {
                color: if enabled {
                    p.outline
                } else {
                    with_alpha(p.outline, 0.34)
                },
                width: 1.0,
                radius: theme::shape::SM.into(),
            },
            ..Default::default()
        }
    });
    if let Some(message) = on_press {
        action = action.on_press(message);
    }
    action
}

impl App {
    pub(crate) fn view_flash_parts_wizard(&self) -> Element<'_, Message> {
        if self.log_popup_open && self.flash_parts.step >= 3 {
            return self.log_popup_view();
        }

        let step_labels: Vec<&str> = FLASH_PARTS_STEPS.iter().map(|k| self.t(k)).collect();
        let is_exec = self.flash_parts.step >= 3;
        let step_bar = if is_exec {
            empty_wizard_step_bar()
        } else {
            wizard_step_bar(
                &step_labels,
                self.flash_parts.step,
                self.window_size_class(),
            )
        };

        let body: Element<'_, Message> = match self.flash_parts.step {
            0 => self.flash_parts_loader_step(),
            1 => self.flash_parts_select_step(),
            2 => self.flash_parts_confirm_step(),
            _ => self.exec_step_view(),
        };
        let (step_title, app_bar_subtitle) = self.flash_parts_step_copy();
        let body = if is_exec {
            body
        } else {
            wizard_step_body(step_title, body)
        };

        let nav = if self.flash_parts.step < 3 {
            let label = match self.flash_parts.step {
                0 => self.t("btn_scan").to_string(),
                1 => self.t("btn_next").to_string(),
                2 => self.t("btn_start").to_string(),
                _ => self.t("btn_next").to_string(),
            };
            let is_start = self.flash_parts.step == 2 || self.flash_parts.step == 0;
            // No loader-fit gate here: by the Confirm step the device is already
            // in EDL (the GPT scan transitioned it) where the model can't be
            // polled, and the loader was already validated by that scan.
            let can = self.flash_parts.can_next()
                && !(self.operation.is_running() && is_start)
                && (!is_start || self.device_reachable());
            let leading_action = if self.flash_parts.step == 1 {
                partition_table_leading_action(self.flash_parts.entry_connection)
            } else {
                WizardLeadingAction::Back
            };
            let leading_label = if leading_action == WizardLeadingAction::Cancel {
                self.t("btn_cancel")
            } else {
                self.t("btn_back")
            };
            wizard_nav_generic_with_leading_action(
                leading_action,
                &label,
                can,
                leading_label,
                if self.flash_parts.step == 0 {
                    Message::FlashParts(FlashPartsMsg::FlashPartsClose)
                } else {
                    Message::FlashParts(FlashPartsMsg::FlashPartsBack)
                },
                Message::FlashParts(FlashPartsMsg::FlashPartsNext),
            )
        } else {
            empty_wizard_nav()
        };

        column![
            wizard_action_bar(
                self.window_size_class(),
                self.t(AdvAction::FlashPartitions.label_key()).to_string(),
                app_bar_subtitle,
            ),
            step_bar,
            body,
            nav,
        ]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    fn flash_parts_step_copy(&self) -> (String, Option<String>) {
        let (title, app_bar_subtitle) = match self.flash_parts.step {
            0 => (self.t("edl_loader_title").to_string(), None),
            1 => (
                self.t("flash_parts_select_title").to_string(),
                Some(self.t("flash_parts_select_subtitle").to_string()),
            ),
            2 => (self.t("flash_parts_confirm_title").to_string(), None),
            _ => {
                let (title, _) = self.exec_status_copy();
                return (title, self.exec_app_bar_subtitle());
            }
        };
        (title, app_bar_subtitle)
    }

    /// Shared loader-picker card for the EDL parts / physical-storage
    /// wizards: a Browse-loader button, the resolved-path / error status
    /// line, and the recent-loader chips. Only the wizard's loader fields
    /// and the two Message variants differ between callers, so they are
    /// threaded in as params; the title / placeholder / accepted
    /// extensions / colors are identical across all four wizards.
    ///
    /// `error` is whatever the caller wants surfaced on this step. The two
    /// partition wizards scan from here, so they pass the loader failure when
    /// there is one and the scan failure otherwise.
    pub(crate) fn loader_picker_card<'a>(
        &'a self,
        loader_path: &'a Option<String>,
        error: Option<&'a String>,
        on_select: Message,
        on_chosen: impl Fn(String) -> Message,
    ) -> Element<'a, Message> {
        let selected = loader_path.is_some();
        let loader_error = error;
        let status = match (loader_path, loader_error) {
            (_, Some(e)) => format!("⚠ {e}"),
            (Some(p), None) => p.clone(),
            _ => self.t("edl_loader_placeholder").to_string(),
        };
        let btn = button(
            container(
                column![
                    text(self.t("btn_browse_loader").to_string())
                        .size(14.0)
                        .center(),
                    text(self.loader_picker_desc())
                        .size(11.0)
                        .style(muted_style)
                        .center(),
                ]
                .spacing(6.0)
                .width(Length::Fill)
                .align_x(iced::Alignment::Center),
            )
            .padding([20.0, 24.0])
            .width(Length::Fixed(280.0))
            .style(move |t: &Theme| sel_card_style(t, selected)),
        )
        .on_press(on_select)
        .padding(0)
        .style(move |t: &Theme, status| sel_card_btn_style(t, status, selected));
        let has_error = loader_error.is_some();
        let status_style = move |t: &Theme| {
            let p = pal_of(t);
            iced::widget::text::Style {
                color: Some(if has_error {
                    p.error
                } else if selected {
                    p.success
                } else {
                    p.outline
                }),
            }
        };
        let chips = self.recent_file_chips(LOADER_PICKER_EXTS, on_chosen, "picker_recents");
        let col = column![
            btn,
            text(status)
                .size(12.0)
                .width(Length::Fill)
                .style(status_style)
                .center()
                .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
            chips,
        ]
        .spacing(14.0)
        .padding(28.0)
        .width(Length::Fill)
        .align_x(iced::Alignment::Center);
        container(col)
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .align_y(iced::alignment::Vertical::Top)
            .into()
    }

    pub(crate) fn flash_parts_loader_step(&self) -> Element<'_, Message> {
        self.loader_picker_card(
            &self.flash_parts.loader_path,
            self.flash_parts
                .loader_error
                .as_ref()
                .or(self.flash_parts.scan_error.as_ref()),
            Message::FlashParts(FlashPartsMsg::FlashPartsSelectLoader),
            |p| Message::FlashParts(FlashPartsMsg::FlashPartsLoaderChosen(Some(p))),
        )
    }

    /// Shared frame for the partition / physical-storage select tables.
    fn select_step_frame<'a>(
        &'a self,
        list: iced::widget::Column<'a, Message>,
    ) -> Element<'a, Message> {
        let scrolled = scrollable(list)
            .style(m3_scrollable_style)
            .height(Length::Fill)
            .width(Length::Fill);
        let col = column![scrolled,]
            .spacing(10.0)
            .padding(20.0)
            .width(Length::Fill)
            .align_x(iced::Alignment::Center);
        container(col)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn flash_parts_select_frame<'a>(
        &'a self,
        list: iced::widget::Column<'a, Message>,
        footer: Element<'a, Message>,
    ) -> Element<'a, Message> {
        let scrolled = scrollable(list)
            .style(m3_scrollable_style)
            .height(Length::Fill)
            .width(Length::Fill);
        container(
            column![scrolled, widget::rule::horizontal(1), footer]
                .spacing(0)
                .width(Length::Fill)
                .height(Length::Fill),
        )
        .padding(20.0)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    pub(crate) fn flash_parts_select_step(&self) -> Element<'_, Message> {
        let active = self.flash_parts.sort_col;
        let desc = self.flash_parts.sort_desc;
        let mk_msg = |c: PartsSortColumn| Message::FlashParts(FlashPartsMsg::FlashPartsSortBy(c));
        let header = row![
            Space::new().width(Length::Fixed(FLASH_PARTS_MARKER_CELL_WIDTH)), // checkbox col
            parts_sort_header(
                self.t("flash_parts_col_lun").to_string(),
                active == PartsSortColumn::Lun,
                desc,
                Length::Fixed(FLASH_PARTS_LUN_COLUMN_WIDTH),
                mk_msg(PartsSortColumn::Lun),
            ),
            parts_sort_header(
                self.t("flash_parts_col_label").to_string(),
                active == PartsSortColumn::Label,
                desc,
                Length::Fixed(FLASH_PARTS_LABEL_COLUMN_WIDTH),
                mk_msg(PartsSortColumn::Label),
            ),
            parts_numeric_sort_header(
                self.t("flash_parts_col_start").to_string(),
                active == PartsSortColumn::Start,
                desc,
                Length::Fixed(FLASH_PARTS_START_COLUMN_WIDTH),
                mk_msg(PartsSortColumn::Start),
            ),
            parts_numeric_sort_header(
                self.t("dump_parts_col_size").to_string(),
                active == PartsSortColumn::Size,
                desc,
                Length::Fixed(FLASH_PARTS_SIZE_COLUMN_WIDTH),
                mk_msg(PartsSortColumn::Size),
            ),
            parts_sort_header(
                self.t("flash_parts_col_file").to_string(),
                active == PartsSortColumn::File,
                desc,
                Length::Fill,
                mk_msg(PartsSortColumn::File),
            ),
            text(self.t("flash_parts_col_state").to_string())
                .size(11.0)
                .width(Length::Fixed(FLASH_PARTS_STATE_COLUMN_WIDTH))
                .style(muted_style),
        ]
        .spacing(8.0)
        .padding([6.0, 10.0])
        .width(Length::Fill)
        .align_y(iced::Alignment::Center);

        let mut list = column![header, widget::rule::horizontal(1)].spacing(0);
        for (idx, r) in self.flash_parts.rows.iter().enumerate() {
            let marker_cell: Element<'_, Message> = match r.state {
                FlashRowState::Skip | FlashRowState::Write => container(
                    iced::widget::checkbox(r.state == FlashRowState::Write)
                        .size(FLASH_PARTS_MARKER_SIZE)
                        .on_toggle(move |_| {
                            Message::FlashParts(FlashPartsMsg::FlashPartsToggleRow(idx))
                        })
                        .style(m3_checkbox_style),
                )
                // `center_x(Fill)` would overwrite the fixed width set above
                // and let this cell take slack, pushing every data column out
                // of line with its header. Align inside the width instead.
                .width(Length::Fixed(FLASH_PARTS_MARKER_CELL_WIDTH))
                .height(Length::Fixed(FLASH_PARTS_ROW_HEIGHT))
                .align_x(iced::alignment::Horizontal::Center)
                .align_y(iced::alignment::Vertical::Center)
                .into(),
                FlashRowState::Erase => button(
                    container(m3_erase_marker())
                        .width(Length::Fill)
                        .height(Length::Fill)
                        .center_x(Length::Fill)
                        .center_y(Length::Fill),
                )
                .padding(0)
                .width(Length::Fixed(FLASH_PARTS_MARKER_CELL_WIDTH))
                .height(Length::Fixed(FLASH_PARTS_ROW_HEIGHT))
                .on_press(Message::FlashParts(FlashPartsMsg::FlashPartsToggleRow(idx)))
                .style(|_t: &Theme, _s| button::Style {
                    background: None,
                    ..Default::default()
                })
                .into(),
            };

            let file_cell: Element<'_, Message> = match r.state {
                FlashRowState::Skip => row![
                    partition_file_button(
                        icon::fab_open_folder(),
                        Some(Message::FlashParts(FlashPartsMsg::FlashPartsPickRowFile(
                            idx
                        ))),
                        false,
                    ),
                    text("—").size(12.0).style(muted_style),
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center)
                .into(),
                FlashRowState::Write => {
                    let file_disp = r
                        .file_path
                        .as_ref()
                        .map(|p| {
                            std::path::Path::new(p)
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_else(|| p.clone())
                        })
                        .unwrap_or_default();
                    row![
                        partition_file_button(
                            icon::fab_cancel(),
                            Some(Message::FlashParts(FlashPartsMsg::FlashPartsClearRowFile(
                                idx
                            ),)),
                            true,
                        ),
                        text(file_disp)
                            .size(12.0)
                            .width(Length::Fill)
                            .wrapping(iced::widget::text::Wrapping::None),
                    ]
                    .spacing(8)
                    .align_y(iced::Alignment::Center)
                    .into()
                }
                FlashRowState::Erase => row![
                    partition_file_button(icon::fab_open_folder(), None, false),
                    text("—").size(12.0),
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center)
                .into(),
            };

            let data_row = iced::widget::row![
                marker_cell,
                text(r.lun.to_string())
                    .size(12.0)
                    .width(Length::Fixed(FLASH_PARTS_LUN_COLUMN_WIDTH)),
                text(r.label.clone())
                    .size(12.0)
                    .width(Length::Fixed(FLASH_PARTS_LABEL_COLUMN_WIDTH))
                    .wrapping(iced::widget::text::Wrapping::None),
                text(r.start_sector.to_string())
                    .size(12.0)
                    .width(Length::Fixed(FLASH_PARTS_START_COLUMN_WIDTH))
                    .align_x(iced::alignment::Horizontal::Right),
                text(format_bytes_auto(r.size_bytes))
                    .size(12.0)
                    .width(Length::Fixed(FLASH_PARTS_SIZE_COLUMN_WIDTH))
                    .align_x(iced::alignment::Horizontal::Right),
                container(file_cell).width(Length::Fill),
                text(match r.state {
                    FlashRowState::Skip => "—",
                    FlashRowState::Write => self.t("flash_parts_state_write"),
                    FlashRowState::Erase => self.t("flash_parts_state_erase"),
                })
                .size(12.0)
                .width(Length::Fixed(FLASH_PARTS_STATE_COLUMN_WIDTH)),
            ]
            .spacing(8.0)
            .padding([0.0, 10.0])
            .width(Length::Fill)
            .height(Length::Fixed(FLASH_PARTS_ROW_HEIGHT))
            .align_y(iced::Alignment::Center);

            // Tint the whole row by its tri-state so flash/erase pop
            // visually; light/dark both pull from the M3 container roles.
            let row_state = r.state;
            let tinted = container(data_row).width(Length::Fill).style(
                move |t: &Theme| -> container::Style {
                    let p = pal_of(t);
                    let (bg, text_color) = match row_state {
                        FlashRowState::Write => {
                            (Some(p.secondary_container), Some(p.on_secondary_container))
                        }
                        FlashRowState::Erase => {
                            (Some(p.error_container), Some(p.on_error_container))
                        }
                        FlashRowState::Skip => (None, None),
                    };
                    container::Style {
                        background: bg.map(iced::Background::Color),
                        text_color,
                        ..Default::default()
                    }
                },
            );

            list = list.push(tinted);
        }

        let write_count = self
            .flash_parts
            .rows
            .iter()
            .filter(|row| row.state == FlashRowState::Write)
            .count();
        let erase_count = self
            .flash_parts
            .rows
            .iter()
            .filter(|row| row.state == FlashRowState::Erase)
            .count();
        let touched_size = self
            .flash_parts
            .rows
            .iter()
            .filter(|row| matches!(row.state, FlashRowState::Write | FlashRowState::Erase))
            .fold(0_u64, |total, row| total.saturating_add(row.size_bytes));
        let footer = row![
            text(tr_args!(
                "flash_parts_footer_counts",
                total = self.flash_parts.rows.len().to_string(),
                write = write_count.to_string(),
                erase = erase_count.to_string(),
            ))
            .size(12.0),
            Space::new().width(Length::Fill),
            text(tr_args!(
                "flash_parts_footer_size",
                size = format_bytes_auto(touched_size),
            ))
            .size(12.0),
        ]
        .spacing(12)
        .padding([12.0, 4.0])
        .width(Length::Fill)
        .align_y(iced::Alignment::Center);

        self.flash_parts_select_frame(list, footer.into())
    }

    pub(crate) fn flash_parts_confirm_step(&self) -> Element<'_, Message> {
        let rows = self.flash_parts.active_rows();
        let erase_rows: Vec<&FlashPartRow> = rows
            .iter()
            .filter(|r| r.state == FlashRowState::Erase)
            .collect();
        let flash_rows: Vec<&FlashPartRow> = rows
            .iter()
            .filter(|r| r.state == FlashRowState::Write)
            .collect();

        let mut leading: Vec<Element<'_, Message>> = Vec::new();

        // ERASE block first, error-toned and loud.
        if !erase_rows.is_empty() {
            let mut erase_col = column![
                text(self.t("flash_parts_confirm_erase_warn").to_string())
                    .size(14.0)
                    .style(|t: &Theme| iced::widget::text::Style {
                        color: Some(pal_of(t).error),
                    })
            ]
            .spacing(4.0);
            for r in &erase_rows {
                erase_col = erase_col.push(
                    text(format!(
                        "⛔ {} (LUN {}, {})",
                        r.label,
                        r.lun,
                        format_bytes_auto(r.size_bytes)
                    ))
                    .size(theme::text_size::BODY_MEDIUM)
                    .style(|t: &Theme| iced::widget::text::Style {
                        color: Some(pal_of(t).error),
                    }),
                );
            }
            leading.push(
                container(erase_col)
                    .padding(14.0)
                    .width(Length::Fill)
                    .style(move |t: &Theme| container::Style {
                        background: Some(iced::Background::Color(pal_of(t).error_container)),
                        border: iced::Border {
                            color: pal_of(t).error,
                            width: 1.0,
                            radius: theme::shape::SM.into(),
                        },
                        text_color: Some(pal_of(t).on_error_container),
                        ..Default::default()
                    })
                    .into(),
            );
        }

        // FLASH block.
        if !flash_rows.is_empty() {
            let mut flash_col = column![
                text(self.t("flash_parts_confirm_flash_hdr").to_string())
                    .size(14.0)
                    .style(on_surface_style)
            ]
            .spacing(4.0);
            for r in &flash_rows {
                let fname = r
                    .file_path
                    .as_ref()
                    .map(|p| {
                        std::path::Path::new(p)
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| p.clone())
                    })
                    .unwrap_or_default();
                flash_col = flash_col.push(
                    text(format!("• {} (LUN {}) ← {}", r.label, r.lun, fname))
                        .size(12.0)
                        .style(muted_style),
                );
            }
            leading.push(
                container(flash_col)
                    .padding(14.0)
                    .width(Length::Fill)
                    .into(),
            );
        }

        self.confirm_step_frame(leading, vec![], vec![])
    }

    pub(crate) fn view_dump_parts_wizard(&self) -> Element<'_, Message> {
        if self.log_popup_open && self.dump_parts.step >= 2 {
            return self.log_popup_view();
        }

        let step_labels: Vec<&str> = DUMP_PARTS_STEPS.iter().map(|k| self.t(k)).collect();
        let is_exec = self.dump_parts.step >= 2;
        let step_bar = if is_exec {
            empty_wizard_step_bar()
        } else {
            wizard_step_bar(&step_labels, self.dump_parts.step, self.window_size_class())
        };

        let body: Element<'_, Message> = match self.dump_parts.step {
            0 => self.dump_parts_loader_step(),
            1 => self.dump_parts_select_step(),
            _ => self.exec_step_view(),
        };
        let (step_title, app_bar_subtitle) = self.dump_parts_step_copy();
        let body = if is_exec {
            body
        } else {
            wizard_step_body(step_title, body)
        };

        let nav = if self.dump_parts.step < 2 {
            let is_dump_step = self.dump_parts.step == 1;
            let label = if is_dump_step {
                self.t("btn_dump").to_string()
            } else {
                self.t("btn_scan").to_string()
            };
            // DumpParts touches EDL on both Scan (step 0) and Dump
            // (step 1) — both spawn workers that talk to the device.
            // Gate both buttons on reachability.
            let can = self.dump_parts.can_next()
                && !self.operation.is_running()
                && self.device_reachable();
            let leading_action = if self.dump_parts.step == 1 {
                partition_table_leading_action(self.dump_parts.entry_connection)
            } else {
                WizardLeadingAction::Back
            };
            let leading_label = if leading_action == WizardLeadingAction::Cancel {
                self.t("btn_cancel")
            } else {
                self.t("btn_back")
            };
            wizard_nav_generic_with_leading_action(
                leading_action,
                &label,
                can,
                leading_label,
                if self.dump_parts.step == 0 {
                    Message::DumpParts(DumpPartsMsg::DumpPartsClose)
                } else {
                    Message::DumpParts(DumpPartsMsg::DumpPartsBack)
                },
                Message::DumpParts(DumpPartsMsg::DumpPartsNext),
            )
        } else {
            empty_wizard_nav()
        };

        column![
            wizard_action_bar(
                self.window_size_class(),
                self.t(AdvAction::DumpPartitions.label_key()).to_string(),
                app_bar_subtitle,
            ),
            step_bar,
            body,
            nav,
        ]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    fn dump_parts_step_copy(&self) -> (String, Option<String>) {
        let title = match self.dump_parts.step {
            0 => self.t("edl_loader_title").to_string(),
            1 => self.t("dump_parts_select_title").to_string(),
            _ => {
                let (title, _) = self.exec_status_copy();
                return (title, self.exec_app_bar_subtitle());
            }
        };
        (title, None)
    }

    pub(crate) fn dump_parts_loader_step(&self) -> Element<'_, Message> {
        self.loader_picker_card(
            &self.dump_parts.loader_path,
            self.dump_parts
                .loader_error
                .as_ref()
                .or(self.dump_parts.scan_error.as_ref()),
            Message::DumpParts(DumpPartsMsg::DumpPartsSelectLoader),
            |p| Message::DumpParts(DumpPartsMsg::DumpPartsLoaderChosen(Some(p))),
        )
    }

    pub(crate) fn dump_parts_select_step(&self) -> Element<'_, Message> {
        let active = self.dump_parts.sort_col;
        let desc = self.dump_parts.sort_desc;
        let mk_msg = |c: PartsSortColumn| Message::DumpParts(DumpPartsMsg::DumpPartsSortBy(c));
        // Header select-all: checked iff every row is selected (and there
        // is at least one row). Click flips toward whichever direction
        // would change state for the majority — full-select if any are
        // unchecked, else clear.
        let all_checked =
            !self.dump_parts.rows.is_empty() && self.dump_parts.rows.iter().all(|r| r.selected);
        let header_cb = iced::widget::checkbox(all_checked)
            .style(m3_checkbox_style)
            .on_toggle(|_| Message::DumpParts(DumpPartsMsg::DumpPartsToggleAll));
        let header = row![
            container(header_cb).width(Length::Fixed(32.0)),
            parts_sort_header(
                self.t("flash_parts_col_lun").to_string(),
                active == PartsSortColumn::Lun,
                desc,
                Length::Fixed(50.0),
                mk_msg(PartsSortColumn::Lun),
            ),
            parts_sort_header(
                self.t("flash_parts_col_label").to_string(),
                active == PartsSortColumn::Label,
                desc,
                Length::FillPortion(3),
                mk_msg(PartsSortColumn::Label),
            ),
            parts_numeric_sort_header(
                self.t("flash_parts_col_start").to_string(),
                active == PartsSortColumn::Start,
                desc,
                Length::FillPortion(2),
                mk_msg(PartsSortColumn::Start),
            ),
            parts_numeric_sort_header(
                self.t("dump_parts_col_size").to_string(),
                active == PartsSortColumn::Size,
                desc,
                Length::FillPortion(2),
                mk_msg(PartsSortColumn::Size),
            ),
        ]
        .spacing(8.0)
        .padding([6.0, 10.0])
        .align_y(iced::Alignment::Center);

        let mut list = column![header, widget::rule::horizontal(1)].spacing(0);
        for (idx, row) in self.dump_parts.rows.iter().enumerate() {
            let cb = iced::widget::checkbox(row.selected)
                .style(m3_checkbox_style)
                .on_toggle(move |_| Message::DumpParts(DumpPartsMsg::DumpPartsToggleRow(idx)));
            let data_row = iced::widget::row![
                container(cb).width(Length::Fixed(32.0)),
                text(row.lun.to_string())
                    .size(12.0)
                    .width(Length::Fixed(50.0)),
                text(row.label.clone())
                    .size(12.0)
                    .width(Length::FillPortion(3)),
                text(row.start_sector.to_string())
                    .size(12.0)
                    .width(Length::FillPortion(2))
                    .align_x(iced::alignment::Horizontal::Right),
                text(format_bytes_auto(row.size_bytes))
                    .size(12.0)
                    .width(Length::FillPortion(2))
                    .align_x(iced::alignment::Horizontal::Right),
            ]
            .spacing(8.0)
            .padding([4.0, 10.0])
            .align_y(iced::Alignment::Center);
            // Tint selected rows so the dump set is visible at a glance.
            let selected = row.selected;
            let tinted = container(data_row).width(Length::Fill).style(
                move |t: &Theme| -> container::Style {
                    let p = pal_of(t);
                    container::Style {
                        background: if selected {
                            Some(iced::Background::Color(p.secondary_container))
                        } else {
                            None
                        },
                        text_color: selected.then_some(p.on_secondary_container),
                        ..Default::default()
                    }
                },
            );
            list = list.push(tinted);
        }

        self.select_step_frame(list)
    }

    pub(crate) fn view_dump_phys_wizard(&self) -> Element<'_, Message> {
        if self.log_popup_open && self.dump_phys.step >= 2 {
            return self.log_popup_view();
        }

        let step_labels: Vec<&str> = DUMP_PHYS_STEPS.iter().map(|k| self.t(k)).collect();
        let is_exec = self.dump_phys.step >= 2;
        let step_bar = if is_exec {
            empty_wizard_step_bar()
        } else {
            wizard_step_bar(&step_labels, self.dump_phys.step, self.window_size_class())
        };

        let body: Element<'_, Message> = match self.dump_phys.step {
            0 => self.dump_phys_loader_step(),
            1 => self.dump_phys_select_step(),
            _ => self.exec_step_view(),
        };
        let (step_title, app_bar_subtitle) = self.dump_phys_step_copy();
        let body = if is_exec {
            body
        } else {
            wizard_step_body(step_title, body)
        };

        let nav = if self.dump_phys.step < 2 {
            let is_dump_step = self.dump_phys.step == 1;
            let label = if is_dump_step {
                self.t("btn_dump").to_string()
            } else {
                self.t("btn_next").to_string()
            };
            // DumpPhys talks to EDL — gate both Scan + Dump on a
            // reachable device.
            let can = self.dump_phys.can_next()
                && !self.operation.is_running()
                && self.device_reachable();
            wizard_nav_generic(
                true,
                &label,
                can,
                self.t("btn_back"),
                if self.dump_phys.step == 0 {
                    Message::DumpPhys(DumpPhysMsg::DumpPhysClose)
                } else {
                    Message::DumpPhys(DumpPhysMsg::DumpPhysBack)
                },
                Message::DumpPhys(DumpPhysMsg::DumpPhysNext),
            )
        } else {
            empty_wizard_nav()
        };

        column![
            wizard_action_bar(
                self.window_size_class(),
                self.t(AdvAction::DumpPhysical.label_key()).to_string(),
                app_bar_subtitle,
            ),
            step_bar,
            body,
            nav,
        ]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    fn dump_phys_step_copy(&self) -> (String, Option<String>) {
        let title = match self.dump_phys.step {
            0 => self.t("edl_loader_title").to_string(),
            1 => self.t("phys_select_title").to_string(),
            _ => {
                let (title, _) = self.exec_status_copy();
                return (title, self.exec_app_bar_subtitle());
            }
        };
        (title, None)
    }

    pub(crate) fn dump_phys_loader_step(&self) -> Element<'_, Message> {
        self.loader_picker_card(
            &self.dump_phys.loader_path,
            self.dump_phys.loader_error.as_ref(),
            Message::DumpPhys(DumpPhysMsg::DumpPhysSelectLoader),
            |p| Message::DumpPhys(DumpPhysMsg::DumpPhysLoaderChosen(Some(p))),
        )
    }

    pub(crate) fn dump_phys_select_step(&self) -> Element<'_, Message> {
        let header = row![
            text(" ").size(11.0).width(Length::Fixed(32.0)),
            text(self.t("phys_col_storage").to_string())
                .size(11.0)
                .width(Length::Fill)
                .style(muted_style),
        ]
        .spacing(8.0)
        .padding([6.0, 10.0])
        .align_y(iced::Alignment::Center);

        let mut list = column![header, widget::rule::horizontal(1)].spacing(0);
        for idx in 0..PHYS_LUN_COUNT {
            let checked = self.dump_phys.selected[idx];
            let cb = iced::widget::checkbox(checked)
                .style(m3_checkbox_style)
                .on_toggle(move |_| Message::DumpPhys(DumpPhysMsg::DumpPhysToggleRow(idx)));
            let data_row = iced::widget::row![
                container(cb).width(Length::Fixed(32.0)),
                text(format!("LUN {idx}")).size(12.0).width(Length::Fill),
            ]
            .spacing(8.0)
            .padding([4.0, 10.0])
            .align_y(iced::Alignment::Center);
            list = list.push(data_row);
        }

        self.select_step_frame(list)
    }

    pub(crate) fn view_flash_phys_wizard(&self) -> Element<'_, Message> {
        if self.log_popup_open && self.flash_phys.step >= 3 {
            return self.log_popup_view();
        }

        let step_labels: Vec<&str> = FLASH_PHYS_STEPS.iter().map(|k| self.t(k)).collect();
        let is_exec = self.flash_phys.step >= 3;
        let step_bar = if is_exec {
            empty_wizard_step_bar()
        } else {
            wizard_step_bar(&step_labels, self.flash_phys.step, self.window_size_class())
        };

        let body: Element<'_, Message> = match self.flash_phys.step {
            0 => self.flash_phys_loader_step(),
            1 => self.flash_phys_select_step(),
            2 => self.flash_phys_confirm_step(),
            _ => self.exec_step_view(),
        };
        let (step_title, app_bar_subtitle) = self.flash_phys_step_copy();
        let body = if is_exec {
            body
        } else {
            wizard_step_body(step_title, body)
        };

        let nav = if self.flash_phys.step < 3 {
            let label = match self.flash_phys.step {
                0 => self.t("btn_next").to_string(),
                1 => self.t("btn_next").to_string(),
                2 => self.t("btn_start").to_string(),
                _ => self.t("btn_next").to_string(),
            };
            let is_start = self.flash_phys.step == 2;
            // No loader-fit gate here: by the Confirm step the device is already
            // in EDL where the model can't be polled, and the loader was already
            // used to open the session.
            let can = self.flash_phys.can_next()
                && !(self.operation.is_running() && is_start)
                && (!is_start || self.device_reachable());
            wizard_nav_generic(
                true,
                &label,
                can,
                self.t("btn_back"),
                if self.flash_phys.step == 0 {
                    Message::FlashPhys(FlashPhysMsg::FlashPhysClose)
                } else {
                    Message::FlashPhys(FlashPhysMsg::FlashPhysBack)
                },
                Message::FlashPhys(FlashPhysMsg::FlashPhysNext),
            )
        } else {
            empty_wizard_nav()
        };

        column![
            wizard_action_bar(
                self.window_size_class(),
                self.t(AdvAction::FlashPhysical.label_key()).to_string(),
                app_bar_subtitle,
            ),
            step_bar,
            body,
            nav,
        ]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    fn flash_phys_step_copy(&self) -> (String, Option<String>) {
        let title = match self.flash_phys.step {
            0 => self.t("edl_loader_title").to_string(),
            1 => self.t("phys_select_title").to_string(),
            2 => self.t("flash_parts_confirm_title").to_string(),
            _ => {
                let (title, _) = self.exec_status_copy();
                return (title, self.exec_app_bar_subtitle());
            }
        };
        (title, None)
    }

    pub(crate) fn flash_phys_loader_step(&self) -> Element<'_, Message> {
        self.loader_picker_card(
            &self.flash_phys.loader_path,
            self.flash_phys.loader_error.as_ref(),
            Message::FlashPhys(FlashPhysMsg::FlashPhysSelectLoader),
            |p| Message::FlashPhys(FlashPhysMsg::FlashPhysLoaderChosen(Some(p))),
        )
    }

    pub(crate) fn flash_phys_select_step(&self) -> Element<'_, Message> {
        let header = row![
            text(" ").size(11.0).width(Length::Fixed(32.0)),
            text(self.t("phys_col_storage").to_string())
                .size(11.0)
                .width(Length::FillPortion(2))
                .style(muted_style),
            text(self.t("flash_parts_col_file").to_string())
                .size(11.0)
                .width(Length::FillPortion(3))
                .style(muted_style),
        ]
        .spacing(8.0)
        .padding([6.0, 10.0])
        .align_y(iced::Alignment::Center);

        let mut list = column![header, widget::rule::horizontal(1)].spacing(0);
        for idx in 0..PHYS_LUN_COUNT {
            let checked = self.flash_phys.selected[idx];
            let cb = iced::widget::checkbox(checked)
                .style(m3_checkbox_style)
                .on_toggle(move |_| Message::FlashPhys(FlashPhysMsg::FlashPhysToggleRow(idx)));

            let file_disp = self.flash_phys.file_paths[idx]
                .as_ref()
                .map(|p| {
                    std::path::Path::new(p)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| p.clone())
                })
                .unwrap_or_default();

            // Picking a file used to need a double-click on the row, which
            // nothing announced. The partition table already spells the same
            // action as a folder button, so this borrows it.
            let data_row = iced::widget::row![
                container(cb).width(Length::Fixed(32.0)),
                text(format!("LUN {idx}"))
                    .size(12.0)
                    .width(Length::FillPortion(2)),
                row![
                    text(file_disp).size(12.0).width(Length::Fill),
                    partition_file_button(
                        icon::fab_open_folder(),
                        Some(Message::FlashPhys(FlashPhysMsg::FlashPhysPickRowFile(idx))),
                        false,
                    ),
                ]
                .spacing(8.0)
                .align_y(iced::Alignment::Center)
                .width(Length::FillPortion(3)),
            ]
            .spacing(8.0)
            .padding([4.0, 10.0])
            .align_y(iced::Alignment::Center);

            list = list.push(data_row);
        }

        self.select_step_frame(list)
    }

    pub(crate) fn flash_phys_confirm_step(&self) -> Element<'_, Message> {
        let pairs = self.flash_phys.active_pairs();

        let mut leading: Vec<Element<'_, Message>> = Vec::new();

        if !pairs.is_empty() {
            let mut list = column![
                text(self.t("flash_parts_confirm_flash_hdr").to_string())
                    .size(14.0)
                    .style(on_surface_style)
            ]
            .spacing(4.0);
            for (lun, path) in &pairs {
                let fname = std::path::Path::new(path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.clone());
                list = list.push(
                    text(format!("• LUN {lun} ← {fname}"))
                        .size(12.0)
                        .style(muted_style),
                );
            }
            leading.push(container(list).padding(14.0).width(Length::Fill).into());
        }

        self.confirm_step_frame(leading, vec![], vec![])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tri_state_marker_always_fits_its_cell() {
        // Scaling the marker twice put it at 27 px inside a 26 px cell, which
        // squeezed the erase badge flat while the checkbox beside it — scaled
        // once — still looked right.
        // Exercise compact, both sides of the class boundary, and expanded.
        for content_width in [756.0, 999.999, 1000.0, 1256.0, 4000.0] {
            let side = FLASH_PARTS_MARKER_SIZE;
            let cell = FLASH_PARTS_ROW_HEIGHT;
            assert!(
                side <= cell,
                "marker {side} does not fit the {cell} cell at content width {content_width}"
            );
        }
    }
}
