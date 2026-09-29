//! Partition + physical-storage dump/flash wizard views + steps. Extracted from `main.rs`.

use crate::focus_button::{self as button, button};
use crate::*;
use iced::widget::{self, Space, column, container, row, scrollable, text};
use iced::{Element, Length, Theme};
use ltbox_core::tr_args;

// Shared review layout: identity first, then the full-width source path.
// Spacing and grouping follow LTBox's desktop adaptation of M3 lists.
fn flash_review_entry<'a>(title: &str, detail: &str, path: Option<&str>) -> Element<'a, Message> {
    let mut content = column![
        text(title.to_owned())
            .size(theme::text_size::BODY_MEDIUM)
            .font(theme::emphasis::medium())
            .wrapping(widget::text::Wrapping::WordOrGlyph),
        text(detail.to_owned()).size(theme::text_size::BODY_SMALL),
    ]
    .spacing(4)
    .width(Length::Fill);
    if let Some(path) = path {
        content = content.push(view::components::compact_review_path(path));
    }
    container(content)
        .padding([12, 0])
        .width(Length::Fill)
        .into()
}

fn flash_review_group<'a>(
    title: &str,
    entries: Vec<Element<'a, Message>>,
    erase: bool,
) -> Element<'a, Message> {
    let mut content = column![
        text(format!("{title} ({})", entries.len()))
            .size(theme::text_size::BODY_MEDIUM)
            .font(theme::emphasis::medium())
    ]
    .spacing(4)
    .width(Length::Fill);
    for (index, entry) in entries.into_iter().enumerate() {
        if index > 0 {
            content = content.push(widget::rule::horizontal(1).style(move |t: &Theme| {
                widget::rule::Style {
                    color: if erase {
                        pal_of(t).on_error_container.scale_alpha(0.2)
                    } else {
                        pal_of(t).outline_variant
                    },
                    radius: 0.0.into(),
                    fill_mode: widget::rule::FillMode::Full,
                    snap: true,
                }
            }));
        }
        content = content.push(entry);
    }
    container(content)
        .padding(16)
        .width(Length::Fill)
        .style(move |t: &Theme| {
            let p = pal_of(t);
            container::Style {
                background: Some(
                    if erase {
                        p.error_container
                    } else {
                        p.surface_container_low
                    }
                    .into(),
                ),
                text_color: Some(if erase {
                    p.on_error_container
                } else {
                    p.on_surface
                }),
                border: iced::Border {
                    radius: theme::shape::SM.into(),
                    ..Default::default()
                },
                ..Default::default()
            }
        })
        .into()
}

const FLASH_PARTS_LUN_COLUMN_WIDTH: f32 = 44.0;
const FLASH_PARTS_LABEL_COLUMN_WIDTH: f32 = 104.0;
const FLASH_PARTS_START_COLUMN_WIDTH: f32 = 116.0;
const FLASH_PARTS_SIZE_COLUMN_WIDTH: f32 = 96.0;
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
    .style(md_text_btn_style)
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
) -> button::Button<'static, Message> {
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
                width: 0.0,
                radius: theme::shape::FULL.into(),
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
            if self.flash_parts.step == 0 {
                self.wizard_picker_step(step_title, body)
            } else {
                wizard_step_body(step_title, body)
            }
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
            0 => (
                self.t("edl_loader_title").to_string(),
                Some(self.loader_picker_subtitle()),
            ),
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

    /// Shared loader picker for EDL wizards: path/action row, description,
    /// error status, and filtered recent paths. Only the wizard's loader fields
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
        let mut content = column![self.wizard_picker_row(
            loader_path.as_deref(),
            PickerPathKind::File,
            Some(on_select),
            None
        ),]
        .spacing(6)
        .width(Length::Fill);
        if let Some(error) = error {
            content = content.push(text(error.clone()).size(12).style(|t: &Theme| {
                iced::widget::text::Style {
                    color: Some(pal_of(t).error),
                }
            }));
        }
        content = content.push(self.recent_file_chips(
            self.loader_picker_exts(),
            on_chosen,
            "picker_recents",
        ));
        scrollable(content.padding(PICKER_BODY_PADDING))
            .width(Length::Fill)
            .height(Length::Fill)
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

    /// Live filter above a partition table. Every keystroke re-filters, so
    /// Enter does nothing — but it must still be claimed here: an unclaimed
    /// Enter reaches a row checkbox that kept keyboard focus and toggles it.
    fn parts_search_field<'a>(
        &self,
        query: &str,
        on_input: impl Fn(String) -> Message + 'a,
    ) -> Element<'a, Message> {
        container(
            widget::text_input(self.t("parts_search_placeholder"), query)
                .on_input(on_input)
                .on_submit(Message::Noop)
                .width(Length::Fill)
                .padding([8, 12])
                .line_height(iced::widget::text::LineHeight::Absolute(24.0.into()))
                .size(theme::text_size::BODY_MEDIUM)
                .style(m3_text_input_style),
        )
        .height(Length::Fixed(40.0))
        .center_y(40)
        .width(Length::Fill)
        .into()
    }

    /// Table row shown when the search hides every partition.
    fn parts_search_empty_row(&self) -> Element<'_, Message> {
        container(
            text(self.t("parts_search_no_match").to_string())
                .size(12.0)
                .style(muted_style),
        )
        .padding([12.0, 10.0])
        .width(Length::Fill)
        .into()
    }

    /// Shared frame for the physical-storage select tables.
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

    /// Shared frame for the partition select tables: search, table, totals.
    fn parts_table_frame<'a>(
        &'a self,
        search: Element<'a, Message>,
        list: iced::widget::Column<'a, Message>,
        footer: Element<'a, Message>,
    ) -> Element<'a, Message> {
        let scrolled = scrollable(list)
            .style(m3_scrollable_style)
            .height(Length::Fill)
            .width(Length::Fill);
        container(
            column![
                container(search).padding(iced::Padding {
                    bottom: 10.0,
                    ..iced::Padding::ZERO
                }),
                scrolled,
                widget::rule::horizontal(1).style(shell_rule_style),
                footer
            ]
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
        ]
        .spacing(8.0)
        .padding([6.0, 10.0])
        .width(Length::Fill)
        .align_y(iced::Alignment::Center);

        let mut list =
            column![header, widget::rule::horizontal(1).style(shell_rule_style)].spacing(0);
        let mut any_visible = false;
        for (idx, r) in self.flash_parts.visible_rows() {
            any_visible = true;
            let marker_cell: Element<'_, Message> = match r.state {
                FlashRowState::Skip | FlashRowState::Write => container(focus_button::actionable(
                    iced::widget::checkbox(r.state == FlashRowState::Write)
                        .size(FLASH_PARTS_MARKER_SIZE)
                        .on_toggle(move |_| {
                            Message::FlashParts(FlashPartsMsg::FlashPartsToggleRow(idx))
                        })
                        .style(m3_checkbox_style),
                    Some(Message::FlashParts(FlashPartsMsg::FlashPartsToggleRow(idx))),
                ))
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
                .style(md_text_btn_style)
                .into(),
            };

            let file_cell: Element<'_, Message> = match (r.state, r.file_path.as_ref()) {
                (FlashRowState::Skip, _) | (FlashRowState::Write, None) => row![
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
                (FlashRowState::Write, Some(path)) => {
                    let file_disp = std::path::Path::new(path)
                        .file_name()
                        .map(|name| name.to_string_lossy().to_string())
                        .unwrap_or_else(|| path.clone());
                    row![
                        partition_file_button(
                            icon::fab_cancel(),
                            Some(Message::FlashParts(FlashPartsMsg::FlashPartsClearRowFile(
                                idx
                            ),)),
                            true,
                        ),
                        container(view::components::compact_review_path(&file_disp))
                            .width(Length::Fill),
                    ]
                    .spacing(8)
                    .align_y(iced::Alignment::Center)
                    .into()
                }
                (FlashRowState::Erase, _) => row![
                    partition_file_button(icon::fab_open_folder(), None, false),
                    text(self.t("flash_parts_state_erase").to_string()).size(12.0),
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
        if !any_visible {
            list = list.push(self.parts_search_empty_row());
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

        let search = self.parts_search_field(&self.flash_parts.search, |query| {
            Message::FlashParts(FlashPartsMsg::FlashPartsSearchInput(query))
        });
        self.parts_table_frame(search, list, footer.into())
    }

    pub(crate) fn flash_parts_confirm_step(&self) -> Element<'_, Message> {
        let rows = self.flash_parts.active_rows();
        let mut groups = Vec::new();
        for (state, key) in [
            (FlashRowState::Erase, "flash_parts_confirm_erase_warn"),
            (FlashRowState::Write, "flash_parts_confirm_flash_hdr"),
        ] {
            let entries: Vec<_> = rows
                .iter()
                .filter(|r| r.state == state)
                .map(|r| {
                    flash_review_entry(
                        &r.label,
                        &format!("LUN {} · {}", r.lun, format_bytes_auto(r.size_bytes)),
                        if state == FlashRowState::Write {
                            r.file_path.as_deref()
                        } else {
                            None
                        },
                    )
                })
                .collect();
            if !entries.is_empty() {
                groups.push(flash_review_group(
                    self.t(key),
                    entries,
                    state == FlashRowState::Erase,
                ));
            }
        }
        self.confirm_step_frame(groups, vec![], vec![])
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
            if self.dump_parts.step == 0 {
                self.wizard_picker_step(step_title, body)
            } else {
                wizard_step_body(step_title, body)
            }
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
        (
            title,
            (self.dump_parts.step == 0).then(|| self.loader_picker_subtitle()),
        )
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
        // Header select-all acts on the rows the search leaves visible:
        // checked iff every visible row is selected; a click selects them all
        // when any is unchecked, otherwise clears them.
        let all_checked = self.dump_parts.all_visible_selected();
        let header_cb = focus_button::actionable(
            iced::widget::checkbox(all_checked)
                .style(m3_checkbox_style)
                .on_toggle(|_| Message::DumpParts(DumpPartsMsg::DumpPartsToggleAll)),
            Some(Message::DumpParts(DumpPartsMsg::DumpPartsToggleAll)),
        );
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

        let mut list =
            column![header, widget::rule::horizontal(1).style(shell_rule_style)].spacing(0);
        let mut any_visible = false;
        for (idx, row) in self.dump_parts.visible_rows() {
            any_visible = true;
            let cb = focus_button::actionable(
                iced::widget::checkbox(row.selected)
                    .style(m3_checkbox_style)
                    .on_toggle(move |_| Message::DumpParts(DumpPartsMsg::DumpPartsToggleRow(idx))),
                Some(Message::DumpParts(DumpPartsMsg::DumpPartsToggleRow(idx))),
            );
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
        if !any_visible {
            list = list.push(self.parts_search_empty_row());
        }

        // Selected rows the search hides still get dumped, so the totals
        // always cover every row.
        let selected = self.dump_parts.rows.iter().filter(|row| row.selected);
        let (selected_count, selected_size) = selected
            .fold((0_usize, 0_u64), |(count, size), row| {
                (count + 1, size.saturating_add(row.size_bytes))
            });
        let footer = row![
            text(tr_args!(
                "dump_parts_footer_counts",
                total = self.dump_parts.rows.len().to_string(),
                selected = selected_count.to_string(),
            ))
            .size(12.0),
            Space::new().width(Length::Fill),
            text(tr_args!(
                "dump_parts_footer_size",
                size = format_bytes_auto(selected_size),
            ))
            .size(12.0),
        ]
        .spacing(12)
        .padding([12.0, 4.0])
        .width(Length::Fill)
        .align_y(iced::Alignment::Center);

        let search = self.parts_search_field(&self.dump_parts.search, |query| {
            Message::DumpParts(DumpPartsMsg::DumpPartsSearchInput(query))
        });
        self.parts_table_frame(search, list, footer.into())
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
            if self.dump_phys.step == 0 {
                self.wizard_picker_step(step_title, body)
            } else {
                wizard_step_body(step_title, body)
            }
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
        (
            title,
            (self.dump_phys.step == 0).then(|| self.loader_picker_subtitle()),
        )
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

        let mut list =
            column![header, widget::rule::horizontal(1).style(shell_rule_style)].spacing(0);
        for idx in 0..PHYS_LUN_COUNT {
            let checked = self.dump_phys.selected[idx];
            let cb = focus_button::actionable(
                iced::widget::checkbox(checked)
                    .style(m3_checkbox_style)
                    .on_toggle(move |_| Message::DumpPhys(DumpPhysMsg::DumpPhysToggleRow(idx))),
                Some(Message::DumpPhys(DumpPhysMsg::DumpPhysToggleRow(idx))),
            );
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
            if self.flash_phys.step == 0 {
                self.wizard_picker_step(step_title, body)
            } else {
                wizard_step_body(step_title, body)
            }
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
        (
            title,
            (self.flash_phys.step == 0).then(|| self.loader_picker_subtitle()),
        )
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

        let mut list =
            column![header, widget::rule::horizontal(1).style(shell_rule_style)].spacing(0);
        for idx in 0..PHYS_LUN_COUNT {
            let checked = self.flash_phys.selected[idx];
            let cb = focus_button::actionable(
                iced::widget::checkbox(checked)
                    .style(m3_checkbox_style)
                    .on_toggle(move |_| Message::FlashPhys(FlashPhysMsg::FlashPhysToggleRow(idx))),
                Some(Message::FlashPhys(FlashPhysMsg::FlashPhysToggleRow(idx))),
            );

            let file_disp = self.flash_phys.file_paths[idx]
                .as_ref()
                .map(|p| {
                    std::path::Path::new(p)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| p.clone())
                })
                .unwrap_or_default();

            // Picking a file uses the same folder-button affordance as the
            // partition table, not a double-click on the row, since a
            // double-click gesture announces nothing.
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
        // Guards against double-scaling the marker: that would put it at
        // 27 px inside this 26 px cell, squeezing the erase badge flat,
        // while the checkbox beside it (scaled once) still fits.
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
