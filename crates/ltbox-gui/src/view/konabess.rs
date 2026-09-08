//! KonaBess wizard and DTB target-selection dialog.

use crate::*;
use iced::widget::{self, Space, button, column, container, row, scrollable, text};
use iced::{Element, Length, Theme};
use ltbox_core::tr_args;

impl App {
    pub(crate) fn view_konabess_wizard(&self) -> Element<'_, Message> {
        if self.log_popup_open && (self.konabess.step >= 3 || self.konabess.target_popup_open) {
            return self.log_popup_view();
        }
        let step_labels = KONABESS_STEPS
            .iter()
            .map(|key| self.t(key))
            .collect::<Vec<_>>();
        let is_exec = self.konabess.step >= 3;
        let step_bar = if is_exec {
            empty_wizard_step_bar()
        } else {
            wizard_step_bar(&step_labels, self.konabess.step, self.window_size_class())
        };
        let title_key = match self.konabess.step {
            0 => "edl_loader_title",
            1 => "konabess_table_title",
            2 => "konabess_confirm_title",
            _ => "konabess_apply_title",
        };
        let app_bar_subtitle = (self.konabess.step >= 3)
            .then(|| self.exec_app_bar_subtitle())
            .flatten();
        let body = match self.konabess.step {
            0 => self.konabess_loader_step(),
            1 => self.konabess_table_step(),
            2 => self.konabess_confirm_step(),
            _ => self.konabess_apply_step(),
        };
        let body = if is_exec {
            body
        } else {
            wizard_step_body(self.t(title_key).to_string(), body)
        };

        let nav: Element<'_, Message> = if konabess_nav_visible(self.konabess.step) {
            let is_confirm = self.konabess.step == 2;
            let unsupported = (!ltbox_core::model::capabilities(&self.device.model).konabess)
                .then(|| tr_args!("model_unsupported", model = "TB376FC / TB390FU"));
            let label = if is_confirm {
                self.t("btn_start")
            } else {
                self.t("btn_next")
            };
            if self.konabess.step == 1 {
                wizard_nav_cancel_generic_with_disabled_next_tooltip(
                    label,
                    self.konabess.can_next()
                        && !self.operation.is_running()
                        && ltbox_core::model::capabilities(&self.device.model).konabess,
                    unsupported,
                    self.t("btn_cancel"),
                    Message::KonaBess(KonaBessMsg::KonaBessBack),
                    Message::KonaBess(KonaBessMsg::KonaBessNext),
                )
            } else {
                wizard_nav_generic_with_disabled_next_tooltip(
                    self.konabess.step > 0,
                    label,
                    self.konabess.can_next()
                        && !self.operation.is_running()
                        && ltbox_core::model::capabilities(&self.device.model).konabess,
                    unsupported,
                    self.t("btn_back"),
                    Message::KonaBess(KonaBessMsg::KonaBessBack),
                    Message::KonaBess(KonaBessMsg::KonaBessNext),
                )
            }
        } else {
            empty_wizard_nav()
        };

        column![
            wizard_action_bar(
                self.window_size_class(),
                self.t("nav_konabess").to_string(),
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

    fn konabess_loader_step(&self) -> Element<'_, Message> {
        self.loader_picker_card(
            &self.konabess.loader_path,
            self.konabess.loader_error.as_ref(),
            Message::KonaBess(KonaBessMsg::KonaBessSelectLoader),
            |path| Message::KonaBess(KonaBessMsg::KonaBessLoaderChosen(Some(path))),
        )
    }

    fn konabess_table_step(&self) -> Element<'_, Message> {
        let target = self
            .konabess
            .selected_target()
            .map(target_label)
            .unwrap_or_else(|| self.t("konabess_target_none").to_string());
        let target_button =
            m3_text_button(format!("{}: {target}", self.t("konabess_table_target")))
                .on_press(Message::KonaBess(KonaBessMsg::KonaBessOpenTarget));
        let import_button = m3_text_button(self.t("konabess_import_button").to_string())
            .on_press(Message::KonaBess(KonaBessMsg::KonaBessSelectImport));
        let mut revert_button = m3_text_button(self.t("konabess_revert_button").to_string());
        if self.konabess.edited_dirty {
            revert_button =
                revert_button.on_press(Message::KonaBess(KonaBessMsg::KonaBessRevertEdits));
        }
        let mut toolbar = row![target_button, Space::new().width(Length::Fill)]
            .spacing(4.0)
            .align_y(iced::Alignment::Center)
            .width(Length::Fill);
        if self.konabess.edited_dirty {
            toolbar = toolbar.push(
                text(self.t("konabess_table_modified").to_string())
                    .size(11.0)
                    .style(muted_style),
            );
        }
        toolbar = toolbar.push(revert_button).push(import_button);

        let mut content = column![
            toolbar,
            text(self.t("konabess_table_value_note").to_string())
                .size(11.0)
                .style(muted_style),
        ]
        .spacing(8.0)
        .width(Length::Fill);
        if let Some(error) = self.konabess.import_error.as_deref() {
            content = content.push(
                text(format!("⚠ {error}"))
                    .size(11.0)
                    .style(|theme: &Theme| iced::widget::text::Style {
                        color: Some(pal_of(theme).error),
                    }),
            );
        } else if let Some(path) = self.konabess.import_path.as_deref() {
            content = content.push(
                text(tr_args!("konabess_import_loaded", path = path))
                    .size(11.0)
                    .style(muted_style),
            );
        }
        let validation = self.konabess.editor_validation();
        if !validation.hard_errors.is_empty() {
            content = content.push(finding_panel(&validation.hard_errors, false, self));
        }
        if !validation.warnings.is_empty() {
            content = content.push(finding_panel(&validation.warnings, true, self));
        }
        content = content.push(widget::rule::horizontal(1));
        content = content.push(match self.konabess.edited_table.as_ref() {
            Some(table) => gpu_table_view(table, self, &validation),
            None => text(self.t("konabess_target_no_table").to_string())
                .size(12.0)
                .style(muted_style)
                .center()
                .width(Length::Fill)
                .into(),
        });
        content = content.push(
            text(self.t("konabess_attribution").to_string())
                .size(theme::text_size::LABEL_SMALL)
                .wrapping(iced::widget::text::Wrapping::None)
                .style(muted_style),
        );

        container(content.padding(20.0))
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn konabess_confirm_step(&self) -> Element<'_, Message> {
        let dash = "—";
        let chip = self.konabess.selected_chip().unwrap_or(dash);
        let target = self
            .konabess
            .selected_target()
            .map(target_label)
            .unwrap_or_else(|| dash.to_string());
        let stock_shape = self
            .konabess
            .stock_table
            .as_ref()
            .map(table_shape)
            .unwrap_or_else(|| dash.to_string());
        let edited_shape = self
            .konabess
            .edited_table
            .as_ref()
            .map(table_shape)
            .unwrap_or_else(|| dash.to_string());
        let change_state = if self.konabess.edited_dirty {
            self.t("konabess_confirm_modified")
        } else {
            self.t("konabess_confirm_unchanged")
        };
        let loader = self.konabess.loader_path.as_deref().unwrap_or(dash);
        let import_path = self.konabess.import_path.as_deref().unwrap_or(dash);

        self.confirm_step_frame(
            vec![],
            vec![
                info_kv_center(self.t("konabess_confirm_chip"), chip),
                info_kv_center(self.t("konabess_table_target"), &target),
                info_kv_center(self.t("konabess_confirm_device_values"), &stock_shape),
                info_kv_center(self.t("konabess_confirm_edited_values"), &edited_shape),
                info_kv_center(self.t("konabess_confirm_changes"), change_state),
            ],
            vec![
                info_kv_center(self.t("edl_loader_label"), loader),
                info_kv_center(self.t("konabess_confirm_import"), import_path),
            ],
        )
    }

    fn konabess_apply_step(&self) -> Element<'_, Message> {
        self.exec_step_view()
    }

    pub(crate) fn konabess_target_popup_view(&self) -> Element<'_, Message> {
        let selected = self.konabess.selected_target_index;
        let mut candidates = column![].spacing(4.0).width(Length::Fill);
        for candidate in &self.konabess.candidates {
            let index = candidate.index;
            let is_selected = selected == Some(index);
            let can_select = candidate.chip.is_some();
            let model = candidate
                .model
                .as_deref()
                .unwrap_or_else(|| self.t("common_unknown"));
            let chip = candidate
                .chip
                .as_deref()
                .unwrap_or_else(|| self.t("common_unknown"));
            let shape = compact_gpu_shape(candidate.gpu_shape.as_ref(), self);
            let is_likely = self.konabess.is_probable_target(index);
            let likely_note = is_likely.then(|| self.t("konabess_target_likely").to_string());
            let details = row![
                text(format!("#{index} · {model} · {chip}"))
                    .size(theme::text_size::BODY_MEDIUM)
                    .width(Length::Fill),
            ]
            .align_y(iced::Alignment::Center);
            let mut candidate_body = column![details].spacing(3.0);
            if let Some(note) = likely_note {
                candidate_body = candidate_body.push(
                    text(note)
                        .size(11.0)
                        .style(move |theme| target_note_style(theme, is_selected, is_likely)),
                );
            }
            candidate_body = candidate_body.push(
                text(shape)
                    .size(11.0)
                    .style(move |theme| target_shape_style(theme, is_selected, is_likely)),
            );
            if !can_select {
                candidate_body = candidate_body.push(
                    text(self.t("konabess_target_unknown_chip_unusable").to_string())
                        .size(11.0)
                        .style(|theme: &Theme| iced::widget::text::Style {
                            color: Some(pal_of(theme).error),
                        }),
                );
            }
            let mut candidate_button = button(candidate_body);
            if can_select {
                candidate_button = candidate_button.on_press(Message::KonaBess(
                    KonaBessMsg::KonaBessTargetSelected(index),
                ));
            }
            candidates = candidates.push(
                candidate_button
                    .padding([9.0, 12.0])
                    .width(Length::Fill)
                    .style(move |theme: &Theme, status| {
                        let palette = pal_of(theme);
                        let background = if is_selected {
                            Some(
                                theme::mix_color(
                                    palette.primary,
                                    palette.on_primary,
                                    theme::state_alpha(status),
                                )
                                .into(),
                            )
                        } else if is_likely {
                            Some(
                                theme::mix_color(
                                    palette.secondary_container,
                                    palette.on_secondary_container,
                                    theme::state_alpha(status),
                                )
                                .into(),
                            )
                        } else {
                            theme::state_layer_bg(status, palette.on_surface).map(Into::into)
                        };
                        button::Style {
                            background,
                            text_color: if is_selected {
                                palette.on_primary
                            } else if is_likely {
                                palette.on_secondary_container
                            } else {
                                palette.on_surface
                            },
                            border: iced::Border {
                                color: if is_selected {
                                    palette.primary
                                } else if is_likely {
                                    palette.secondary
                                } else {
                                    palette.outline
                                },
                                width: 1.0,
                                radius: theme::shape::SM.into(),
                            },
                            ..Default::default()
                        }
                    }),
            );
        }
        if self.konabess.candidates.is_empty() {
            candidates = candidates.push(
                text(self.t("konabess_target_no_candidates").to_string())
                    .size(12.0)
                    .style(muted_style)
                    .center()
                    .width(Length::Fill),
            );
        }

        let summary = tr_args!(
            "konabess_target_summary",
            count = self.konabess.candidates.len().to_string()
        );
        let mut confirm = m3_filled_button(self.t("btn_ok").to_string());
        if selected.is_some() {
            confirm = confirm.on_press(Message::KonaBess(KonaBessMsg::KonaBessTargetConfirm));
        }
        let header: Element<'_, Message> = column![
            text(self.t("konabess_target_title").to_string()).size(16.0),
            text(self.t("konabess_target_subtitle").to_string())
                .size(12.0)
                .style(muted_style),
            text(summary).size(11.0).style(muted_style),
        ]
        .spacing(6)
        .into();
        let body: Element<'_, Message> = scrollable(candidates)
            .style(m3_scrollable_style)
            .height(Length::Fixed(300.0))
            .into();
        let footer: Element<'_, Message> = row![
            Space::new().width(Length::Fill),
            m3_outlined_button(self.t("btn_cancel").to_string())
                .on_press(Message::KonaBess(KonaBessMsg::KonaBessTargetDismiss)),
            confirm,
        ]
        .spacing(10)
        .align_y(iced::Alignment::Center)
        .into();
        m3_dialog(dialog_sections(
            header,
            body,
            footer,
            theme::DIALOG_WIDTH_LG,
            true,
        ))
    }
}

fn target_label(target: &ltbox_patch::konabess::VendorBootDtbInfo) -> String {
    format!("#{}", target.index)
}

fn table_shape(table: &ltbox_patch::konabess::GpuTable) -> String {
    table
        .groups
        .iter()
        .map(|group| format!("{}×{}", group.id, group.levels.len()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn ordered_property_names(group: &ltbox_patch::konabess::GpuGroup) -> Vec<&str> {
    let mut names = Vec::new();
    for level in &group.levels {
        for property in &level.properties {
            if !names.contains(&property.name.as_str()) {
                names.push(property.name.as_str());
            }
        }
    }
    names
}

fn gpu_table_view<'a>(
    table: &'a ltbox_patch::konabess::GpuTable,
    app: &'a App,
    validation: &ltbox_patch::konabess::GpuTableValidation,
) -> Element<'a, Message> {
    let mut groups = column![].spacing(18.0).width(Length::Shrink);
    let has_hard_errors = validation.has_hard_errors();
    for (group_position, group) in table.groups.iter().enumerate() {
        let has_warning = validation
            .warnings
            .iter()
            .any(|issue| issue_belongs_to_group(issue, group.id));
        let property_names = ordered_property_names(group);
        let mut add_button = m3_text_button(app.t("konabess_add_level").to_string());
        if !has_hard_errors {
            add_button = add_button.on_press(Message::KonaBess(KonaBessMsg::KonaBessAddLevel(
                group_position,
            )));
        }
        // `groups` must stay intrinsic-width so the two-axis scrollable can
        // expose wide device tables. A Fill row (or Fill spacer) under that
        // Shrink parent creates contradictory horizontal constraints.
        let mut group_label = row![].spacing(5.0).align_y(iced::Alignment::Center);
        if has_warning {
            group_label = group_label.push(
                text("⚠")
                    .size(theme::text_size::BODY_MEDIUM)
                    .style(warning_container_text_style),
            );
        }
        group_label = group_label.push(
            text(format!("Bin {}", group.id))
                .size(14.0)
                .style(move |theme| group_heading_text_style(theme, has_warning)),
        );
        let group_label = container(group_label)
            .padding([4.0, 8.0])
            .style(move |theme| group_heading_style(theme, has_warning));
        let group_heading = row![group_label, add_button]
            .spacing(8.0)
            .align_y(iced::Alignment::Center)
            .width(Length::Shrink);

        let mut header_properties = column![].spacing(0).width(Length::Shrink);
        for property in &group.header_properties {
            let property_width = property_cells_width(property.cells.len());
            let mut property_row =
                row![table_cell(property_label(&property.name), true, 250.0,)].spacing(0);
            let value_cell =
                match gpu_property_editability(GpuPropertyLocation::GroupHeader, &property.name) {
                    GpuPropertyEditability::ReadOnly => {
                        read_only_property_cell(property, property_width)
                    }
                    GpuPropertyEditability::Editable => {
                        unreachable!("group header properties are always read-only")
                    }
                };
            property_row = property_row.push(value_cell);
            header_properties = header_properties.push(property_row);
        }

        let mut table_rows = column![].spacing(0).width(Length::Shrink);
        let mut header = row![table_cell("Level".to_string(), true, 150.0,)].spacing(0);
        for name in &property_names {
            header = header.push(table_cell(
                property_label(name),
                true,
                property_column_width(group, name),
            ));
        }
        table_rows = table_rows.push(header);
        for (level_position, level) in group.levels.iter().enumerate() {
            let mut remove_button = m3_text_button(app.t("konabess_remove_level").to_string());
            if group.levels.len() > 1 && !has_hard_errors {
                remove_button = remove_button.on_press(Message::KonaBess(
                    KonaBessMsg::KonaBessRemoveLevel(group_position, level_position),
                ));
            }
            let level_control = container(
                row![text(level.id.to_string()).size(12.0), remove_button]
                    .spacing(6.0)
                    .align_y(iced::Alignment::Center),
            )
            .padding([4.0, 7.0])
            .width(Length::Fixed(150.0))
            .height(Length::Fixed(58.0))
            .align_y(iced::alignment::Vertical::Center)
            .style(derived_table_cell_style);
            let mut cells = row![level_control].spacing(0);
            for name in &property_names {
                let property = level
                    .properties
                    .iter()
                    .enumerate()
                    .find(|(_, property)| property.name == *name);
                let width = property_column_width(group, name);
                cells = cells.push(match property {
                    Some((property_position, property)) => editable_property_cell(
                        property,
                        |cell| {
                            GpuCellKey::level(
                                group_position,
                                level_position,
                                property_position,
                                cell,
                            )
                        },
                        width,
                        app,
                        validation,
                    ),
                    None => table_cell("—".to_string(), false, width),
                });
            }
            table_rows = table_rows.push(cells);
        }
        groups = groups.push(column![group_heading, header_properties, table_rows,].spacing(6.0));
    }

    scrollable(groups)
        .direction(widget::scrollable::Direction::Both {
            vertical: widget::scrollable::Scrollbar::default(),
            horizontal: widget::scrollable::Scrollbar::default(),
        })
        .style(m3_scrollable_style)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn table_cell(value: String, header: bool, width: f32) -> Element<'static, Message> {
    container(
        text(value)
            .size(if header { 11.0 } else { 12.0 })
            .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
    )
    .padding([7.0, 9.0])
    .width(Length::Fixed(width))
    .height(Length::Fixed(if header { 52.0 } else { 58.0 }))
    .align_y(iced::alignment::Vertical::Center)
    .style(table_border_style(header))
    .into()
}

fn table_border_style(header: bool) -> impl Fn(&Theme) -> container::Style {
    move |theme: &Theme| {
        let palette = pal_of(theme);
        container::Style {
            background: header.then(|| palette.surface_container_high.into()),
            border: iced::Border {
                color: palette.outline_variant,
                width: 1.0,
                radius: 0.0.into(),
            },
            ..Default::default()
        }
    }
}

fn editable_property_cell<'a>(
    property: &ltbox_patch::konabess::GpuProperty,
    key_for_cell: impl Fn(usize) -> GpuCellKey,
    width: f32,
    app: &'a App,
    validation: &ltbox_patch::konabess::GpuTableValidation,
) -> Element<'a, Message> {
    let mut inputs = row![].spacing(6.0);
    for (cell_position, committed) in property.cells.iter().copied().enumerate() {
        let key = key_for_cell(cell_position);
        let value = app.konabess.cell_text(key, committed, &property.name);
        if gpu_property_editability(GpuPropertyLocation::Level, &property.name)
            == GpuPropertyEditability::ReadOnly
        {
            inputs = inputs.push(derived_value_cell(value));
            continue;
        }
        let parser_error = app.konabess.cell_has_input_error(key);
        let hard_error = parser_error
            || validation
                .hard_errors
                .iter()
                .any(|issue| app.konabess.issue_matches_cell(issue, key));
        let warning = !hard_error
            && validation
                .warnings
                .iter()
                .any(|issue| app.konabess.issue_matches_cell(issue, key));
        if matches!(property.name.as_str(), "qcom,level" | "qcom,cx-level")
            && let Some(chip) = app.konabess.selected_chip()
            && let Some(options) = regulator_vote_choices(chip, committed)
        {
            let selected = RegulatorVoteChoice::new(chip, committed);
            let picker = widget::pick_list(options, Some(selected), move |choice| {
                Message::KonaBess(KonaBessMsg::KonaBessCellChanged(
                    key,
                    choice.vote.to_string(),
                ))
            })
            .text_size(12.0)
            .padding([7.0, 8.0])
            .style(move |theme: &Theme, status| {
                let mut style = m3_pick_list_style(theme, status);
                if hard_error {
                    style.border.color = pal_of(theme).error;
                    style.border.width = 2.0;
                } else if warning {
                    style.border.color = pal_of(theme).warning;
                    style.border.width = 2.0;
                }
                style
            })
            .menu_style(m3_pick_list_menu_style)
            .width(Length::Fixed((width - 16.0).max(104.0)));
            inputs = inputs.push(picker);
            continue;
        }
        let input = widget::text_input("", &value)
            .on_input(move |text| Message::KonaBess(KonaBessMsg::KonaBessCellChanged(key, text)))
            .padding([7.0, 8.0])
            .size(12.0)
            .width(Length::Fixed(104.0))
            .style(move |theme: &Theme, status| {
                let mut style = m3_text_input_style(theme, status);
                if hard_error {
                    style.border.color = pal_of(theme).error;
                    style.border.width = 2.0;
                } else if warning {
                    let palette = pal_of(theme);
                    style.background = palette.warning_container.into();
                    style.value = palette.on_warning_container;
                    style.placeholder = theme::with_alpha(palette.on_warning_container, 0.62);
                    style.selection = theme::with_alpha(palette.warning, 0.30);
                    style.border.color = palette.warning;
                    style.border.width = 2.0;
                }
                style
            });
        inputs = inputs.push(input);
    }
    container(inputs)
        .padding([7.0, 8.0])
        .width(Length::Fixed(width))
        .height(Length::Fixed(58.0))
        .align_y(iced::alignment::Vertical::Center)
        .style(table_border_style(false))
        .into()
}

fn read_only_property_cell(
    property: &ltbox_patch::konabess::GpuProperty,
    width: f32,
) -> Element<'static, Message> {
    let mut values = row![].spacing(6.0);
    for cell in &property.cells {
        values = values.push(
            container(text(cell.to_string()).size(12.0))
                .padding([7.0, 8.0])
                .width(Length::Fixed(104.0))
                .style(derived_value_style),
        );
    }
    container(values)
        .padding([7.0, 8.0])
        .width(Length::Fixed(width))
        .height(Length::Fixed(58.0))
        .align_y(iced::alignment::Vertical::Center)
        .style(table_border_style(false))
        .into()
}

fn derived_value_cell(value: String) -> Element<'static, Message> {
    container(text(value).size(12.0))
        .padding([7.0, 8.0])
        .width(Length::Fixed(104.0))
        .style(derived_value_style)
        .into()
}

fn derived_value_style(theme: &Theme) -> container::Style {
    let palette = pal_of(theme);
    container::Style {
        background: Some(palette.surface_container_high.into()),
        text_color: Some(palette.on_surface_variant),
        border: iced::Border {
            color: palette.outline_variant,
            width: 1.0,
            radius: theme::shape::XS.into(),
        },
        ..Default::default()
    }
}

fn derived_table_cell_style(theme: &Theme) -> container::Style {
    let palette = pal_of(theme);
    container::Style {
        background: Some(palette.surface_container_high.into()),
        text_color: Some(palette.on_surface_variant),
        border: iced::Border {
            color: palette.outline_variant,
            width: 1.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}

fn group_heading_text_style(theme: &Theme, warning: bool) -> iced::widget::text::Style {
    if warning {
        warning_container_text_style(theme)
    } else {
        muted_style(theme)
    }
}

fn group_heading_style(theme: &Theme, warning: bool) -> container::Style {
    if !warning {
        return container::Style::default();
    }
    let palette = pal_of(theme);
    container::Style {
        background: Some(palette.warning_container.into()),
        text_color: Some(palette.on_warning_container),
        border: iced::Border {
            color: palette.warning,
            width: 1.0,
            radius: theme::shape::SM.into(),
        },
        ..Default::default()
    }
}

fn property_cells_width(cell_count: usize) -> f32 {
    ((cell_count.max(1) as f32) * 110.0 + 16.0).max(190.0)
}

fn property_column_width(group: &ltbox_patch::konabess::GpuGroup, name: &str) -> f32 {
    group
        .levels
        .iter()
        .filter_map(|level| {
            level
                .properties
                .iter()
                .find(|property| property.name == name)
        })
        .map(|property| property_cells_width(property.cells.len()))
        .fold(
            if matches!(name, "qcom,level" | "qcom,cx-level") {
                260.0
            } else {
                190.0
            },
            f32::max,
        )
}

fn property_label(name: &str) -> String {
    name.strip_prefix("qcom,").unwrap_or(name).to_string()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RegulatorVoteChoice {
    vote: u32,
    name: Option<&'static str>,
}

impl RegulatorVoteChoice {
    fn new(chip: &str, vote: u32) -> Self {
        Self {
            vote,
            name: ltbox_patch::konabess::regulator_level_name(chip, vote),
        }
    }
}

impl std::fmt::Display for RegulatorVoteChoice {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.name {
            Some(name) => write!(formatter, "{name} ({})", self.vote),
            None => self.vote.fmt(formatter),
        }
    }
}

fn regulator_vote_choices(chip: &str, current: u32) -> Option<Vec<RegulatorVoteChoice>> {
    let votes = ltbox_patch::konabess::regulator_level_votes(chip)?;
    let mut choices = votes
        .iter()
        .copied()
        .map(|vote| RegulatorVoteChoice::new(chip, vote))
        .collect::<Vec<_>>();
    if !votes.contains(&current) {
        choices.push(RegulatorVoteChoice::new(chip, current));
    }
    Some(choices)
}

fn finding_panel(
    issues: &[ltbox_patch::konabess::GpuTableIssue],
    warning: bool,
    app: &App,
) -> Element<'static, Message> {
    let count = finding_count(issues);
    let mut content = if warning {
        column![
            text(tr_args!(
                "konabess_warning_summary",
                count = count.to_string()
            ))
            .size(11.0)
            .wrapping(iced::widget::text::Wrapping::None)
        ]
    } else {
        column![
            text(tr_args!(
                "konabess_error_summary",
                count = count.to_string()
            ))
            .size(12.0)
        ]
    }
    .spacing(3.0);
    if !warning {
        for issue in issues {
            content = content.push(text(localized_issue(issue, false, app)).size(11.0));
        }
    }
    container(content)
        .padding([9.0, 12.0])
        .width(Length::Fill)
        .style(move |theme: &Theme| {
            let palette = pal_of(theme);
            let (background, foreground, border) = if warning {
                (
                    palette.warning_container,
                    palette.on_warning_container,
                    palette.warning,
                )
            } else {
                (
                    palette.error_container,
                    palette.on_error_container,
                    palette.error,
                )
            };
            container::Style {
                background: Some(background.into()),
                text_color: Some(foreground),
                border: iced::Border {
                    color: border,
                    width: 1.0,
                    radius: theme::shape::SM.into(),
                },
                ..Default::default()
            }
        })
        .into()
}

const fn finding_count(issues: &[ltbox_patch::konabess::GpuTableIssue]) -> usize {
    issues.len()
}

fn issue_belongs_to_group(issue: &ltbox_patch::konabess::GpuTableIssue, group_id: u32) -> bool {
    let group_path = format!("group {group_id}");
    issue.path == group_path
        || issue
            .path
            .strip_prefix(&group_path)
            .is_some_and(|suffix| suffix.starts_with(" / "))
}

fn localized_issue(
    issue: &ltbox_patch::konabess::GpuTableIssue,
    warning: bool,
    app: &App,
) -> String {
    let detail_key = if !warning {
        "konabess_error_invalid_cell"
    } else if issue.message.contains("outside the observed stock range") {
        "konabess_warning_outside_stock"
    } else if issue.message.contains("not strictly descending") {
        "konabess_warning_frequency_order"
    } else if issue.message.contains("was deleted") {
        "konabess_warning_retargeted"
    } else if issue.message.contains("first match wins") {
        "konabess_warning_duplicate_frequency"
    } else {
        "konabess_warning_other"
    };
    format!("{}: {}", issue.path, app.t(detail_key))
}

const fn konabess_nav_visible(step: usize) -> bool {
    step < 3
}

fn target_note_style(
    theme: &Theme,
    is_selected: bool,
    is_likely: bool,
) -> iced::widget::text::Style {
    if is_selected {
        iced::widget::text::Style {
            color: Some(pal_of(theme).on_primary),
        }
    } else if is_likely {
        iced::widget::text::Style {
            color: Some(pal_of(theme).on_secondary_container),
        }
    } else {
        muted_style(theme)
    }
}

fn target_shape_style(
    theme: &Theme,
    is_selected: bool,
    is_likely: bool,
) -> iced::widget::text::Style {
    if is_selected {
        iced::widget::text::Style {
            color: Some(theme::with_alpha(pal_of(theme).on_primary, 0.72)),
        }
    } else if is_likely {
        iced::widget::text::Style {
            color: Some(theme::with_alpha(
                pal_of(theme).on_secondary_container,
                0.78,
            )),
        }
    } else {
        muted_style(theme)
    }
}

fn compact_gpu_shape(shape: Option<&ltbox_patch::konabess::GpuTableShape>, app: &App) -> String {
    let Some(shape) = shape else {
        return app.t("konabess_target_no_table").to_string();
    };
    if shape.groups.is_empty() {
        return app.t("konabess_target_no_table").to_string();
    }
    shape
        .groups
        .iter()
        .map(|group| format!("G{}×{}", group.id, group.level_count))
        .collect::<Vec<_>>()
        .join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ltbox_patch::konabess::{
        GpuGroup, GpuLevel, GpuProperty, GpuTableIssue, VendorBootDtbInfo,
    };

    #[test]
    fn wizard_nav_is_present_before_exec_and_hidden_during_exec() {
        for step in 0..3 {
            assert!(konabess_nav_visible(step));
        }
        for step in [3, 4, usize::MAX] {
            assert!(!konabess_nav_visible(step));
        }
    }

    #[test]
    fn selected_target_sub_lines_use_on_primary_colors() {
        let theme = Theme::Light;
        let palette = pal_of(&theme);

        assert_eq!(
            target_note_style(&theme, true, false).color,
            Some(palette.on_primary)
        );
        assert_eq!(
            target_shape_style(&theme, true, false).color,
            Some(theme::with_alpha(palette.on_primary, 0.72))
        );
        assert_eq!(
            target_note_style(&theme, false, false).color,
            muted_style(&theme).color
        );
        assert_eq!(
            target_shape_style(&theme, false, false).color,
            muted_style(&theme).color
        );
        assert_eq!(
            target_note_style(&theme, false, true).color,
            Some(palette.on_secondary_container)
        );
        assert_eq!(
            target_shape_style(&theme, false, true).color,
            Some(theme::with_alpha(palette.on_secondary_container, 0.78))
        );
    }

    #[test]
    fn advisory_count_includes_cell_and_group_only_findings() {
        let findings = vec![
            GpuTableIssue {
                path: "group 0 / level 0 / qcom,gpu-freq".into(),
                message: "outside the observed stock range".into(),
            },
            GpuTableIssue {
                path: "group 0".into(),
                message: "frequencies are not strictly descending".into(),
            },
            GpuTableIssue {
                path: "group 0 / qcom,initial-pwrlevel".into(),
                message: "target frequency was deleted".into(),
            },
        ];

        assert_eq!(finding_count(&findings), 3);
        assert!(
            findings
                .iter()
                .all(|finding| issue_belongs_to_group(finding, 0))
        );
        assert!(!issue_belongs_to_group(&findings[0], 1));
    }

    #[test]
    fn target_label_contains_only_the_dtb_index() {
        let target = VendorBootDtbInfo {
            index: 6,
            model: Some("Qualcomm Technologies, Inc. SunP v2 Alt. Thermal Profile SoC".into()),
            chip: Some("sun".into()),
            gpu_shape: None,
            table: None,
        };

        assert_eq!(target_label(&target), "#6");
    }

    #[test]
    fn structural_labels_strip_only_the_qcom_prefix() {
        assert_eq!(property_label("qcom,speed-bin"), "speed-bin");
        assert_eq!(property_label("qcom,initial-pwrlevel"), "initial-pwrlevel");
        assert_eq!(property_label("reg"), "reg");
        assert_eq!(property_label("#size-cells"), "#size-cells");
    }

    #[test]
    fn regulator_picker_labels_keep_exact_votes_and_unknown_values() {
        let choices = regulator_vote_choices("sun", 51).expect("sun has an upstream mapping");
        assert!(
            choices
                .iter()
                .any(|choice| choice.to_string() == "NOM (256)")
        );
        assert!(choices.iter().any(|choice| choice.to_string() == "51"));
    }

    #[test]
    fn table_columns_follow_first_source_occurrence_across_heterogeneous_rows() {
        let group = GpuGroup {
            id: 0,
            header_properties: vec![],
            levels: vec![
                GpuLevel {
                    id: 0,
                    properties: vec![
                        GpuProperty {
                            name: "reg".into(),
                            cells: vec![0],
                        },
                        GpuProperty {
                            name: "qcom,gpu-freq".into(),
                            cells: vec![900_000_000],
                        },
                    ],
                },
                GpuLevel {
                    id: 1,
                    properties: vec![
                        GpuProperty {
                            name: "reg".into(),
                            cells: vec![1],
                        },
                        GpuProperty {
                            name: "qcom,acd-level".into(),
                            cells: vec![2],
                        },
                    ],
                },
            ],
        };

        assert_eq!(
            ordered_property_names(&group),
            ["reg", "qcom,gpu-freq", "qcom,acd-level"]
        );
    }
}
