//! KonaBess wizard and DTB target-selection dialog.

use crate::focus_button::{self as button, button};
use crate::*;
use iced::widget::{self, Space, column, container, row, scrollable, text};
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
        let app_bar_subtitle = if self.konabess.step == 0 {
            Some(self.loader_picker_subtitle())
        } else {
            (self.konabess.step >= 3)
                .then(|| self.exec_app_bar_subtitle())
                .flatten()
        };
        let body = match self.konabess.step {
            0 => self.konabess_loader_step(),
            1 => self.konabess_table_step(),
            2 => self.konabess_confirm_step(),
            _ => self.konabess_apply_step(),
        };
        let body = if is_exec {
            body
        } else if self.konabess.step == 0 {
            self.wizard_picker_step(self.t(title_key).to_string(), body)
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

        let mut content = column![toolbar].spacing(8.0).width(Length::Fill);
        if let (Some(table), Some(stock), Some(chip)) = (
            self.konabess.edited_table.as_ref(),
            self.konabess.stock_table.as_ref(),
            self.konabess.selected_chip(),
        ) {
            content = content.push(gpu_summary_view(table, stock, chip, self));
        }
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
        let fill_height = self.window_size.1 >= GPU_TABLE_FILL_MIN_WINDOW_HEIGHT;
        content = content.push(match self.konabess.edited_table.as_ref() {
            Some(table) => gpu_table_view(table, self, &validation, fill_height),
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
                .width(Length::Fill)
                .align_x(iced::alignment::Horizontal::Right)
                .wrapping(iced::widget::text::Wrapping::WordOrGlyph)
                .style(muted_style),
        );

        let padded = content.padding(20.0);
        if fill_height {
            return container(padded)
                .width(Length::Fill)
                .height(Length::Fill)
                .into();
        }
        scrollable(padded)
            .style(m3_scrollable_style)
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
                confirm_definition_row(self.t("konabess_confirm_chip"), chip),
                confirm_definition_row(self.t("konabess_table_target"), &target),
                confirm_definition_row(self.t("konabess_confirm_device_values"), &stock_shape),
                confirm_definition_row(self.t("konabess_confirm_edited_values"), &edited_shape),
                confirm_definition_row(self.t("konabess_confirm_changes"), change_state),
            ],
            vec![
                confirm_path_row(self.t("edl_loader_label"), loader),
                confirm_path_row(self.t("konabess_confirm_import"), import_path),
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

const GPU_TABLE_HEADER_HEIGHT: f32 = 42.0;
const GPU_TABLE_ROW_HEIGHT: f32 = 58.0;
const GPU_LEVEL_COLUMN_WIDTH: f32 = 136.0;
const GPU_FREQUENCY_COLUMN_WIDTH: f32 = 142.0;
const GPU_VOLTAGE_COLUMN_WIDTH: f32 = 230.0;
const GPU_DELTA_COLUMN_WIDTH: f32 = 106.0;
const GPU_ACTION_COLUMN_WIDTH: f32 = 76.0;
const GPU_BUS_INPUT_WIDTH: f32 = 68.0;

#[derive(Debug, Clone, PartialEq, Eq)]
enum GpuTableColumn {
    Frequency,
    Voltage,
    Delta,
    Bus(Vec<String>),
    Other(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VoltageDelta {
    Stock,
    Down(usize),
    Up(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GpuComparisonSummary {
    stock_max_vote: Option<u32>,
    undervolted_levels: usize,
    frequency_changes_by_bin: Vec<(u32, usize)>,
    has_non_comparable_bin: bool,
}

fn table_property_names(table: &ltbox_patch::konabess::GpuTable) -> Vec<&str> {
    let mut names = Vec::new();
    for group in &table.groups {
        for name in ordered_property_names(group) {
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names
}

fn gpu_table_columns(table: &ltbox_patch::konabess::GpuTable) -> Vec<GpuTableColumn> {
    let property_names = table_property_names(table);
    let bus_names = property_names
        .iter()
        .filter(|name| name.starts_with("qcom,bus-"))
        .map(|name| (*name).to_string())
        .collect::<Vec<_>>();
    let mut columns = vec![
        GpuTableColumn::Frequency,
        GpuTableColumn::Voltage,
        GpuTableColumn::Delta,
    ];
    if !bus_names.is_empty() {
        columns.push(GpuTableColumn::Bus(bus_names));
    }
    columns.extend(
        property_names
            .into_iter()
            .filter(|name| {
                !matches!(*name, "reg" | "qcom,gpu-freq" | "qcom,level")
                    && !name.starts_with("qcom,bus-")
            })
            .map(|name| GpuTableColumn::Other(name.to_string())),
    );
    columns
}

fn scalar_property(properties: &[ltbox_patch::konabess::GpuProperty], name: &str) -> Option<u32> {
    let cells = &properties
        .iter()
        .find(|property| property.name == name)?
        .cells;
    (cells.len() == 1).then(|| cells[0])
}

fn comparable_stock_group<'a>(
    group: &ltbox_patch::konabess::GpuGroup,
    stock: &'a ltbox_patch::konabess::GpuTable,
) -> Option<&'a ltbox_patch::konabess::GpuGroup> {
    stock
        .groups
        .iter()
        .find(|candidate| candidate.id == group.id)
        .filter(|candidate| candidate.levels.len() == group.levels.len())
}

fn voltage_delta(chip: &str, edited_vote: u32, stock_vote: u32) -> Option<VoltageDelta> {
    if edited_vote == stock_vote {
        return Some(VoltageDelta::Stock);
    }
    let votes = ltbox_patch::konabess::regulator_level_votes(chip)?;
    let edited = votes.iter().position(|vote| *vote == edited_vote)?;
    let stock = votes.iter().position(|vote| *vote == stock_vote)?;
    Some(if edited < stock {
        VoltageDelta::Down(stock - edited)
    } else {
        VoltageDelta::Up(edited - stock)
    })
}

fn gpu_comparison_summary(
    table: &ltbox_patch::konabess::GpuTable,
    stock: &ltbox_patch::konabess::GpuTable,
    chip: &str,
) -> GpuComparisonSummary {
    let stock_max_vote = stock
        .groups
        .iter()
        .flat_map(|group| &group.levels)
        .filter_map(|level| scalar_property(&level.properties, "qcom,level"))
        .max();
    let mut undervolted_levels = 0;
    let mut frequency_changes_by_bin = Vec::new();
    let mut has_non_comparable_bin = table.groups.len() != stock.groups.len();

    for group in &table.groups {
        let Some(stock_group) = comparable_stock_group(group, stock) else {
            has_non_comparable_bin = true;
            continue;
        };
        let mut frequency_changes = 0;
        for (level, stock_level) in group.levels.iter().zip(&stock_group.levels) {
            if let (Some(edited_vote), Some(stock_vote)) = (
                scalar_property(&level.properties, "qcom,level"),
                scalar_property(&stock_level.properties, "qcom,level"),
            ) && matches!(
                voltage_delta(chip, edited_vote, stock_vote),
                Some(VoltageDelta::Down(_))
            ) {
                undervolted_levels += 1;
            }
            if scalar_property(&level.properties, "qcom,gpu-freq")
                != scalar_property(&stock_level.properties, "qcom,gpu-freq")
            {
                frequency_changes += 1;
            }
        }
        if frequency_changes > 0 {
            frequency_changes_by_bin.push((group.id, frequency_changes));
        }
    }

    GpuComparisonSummary {
        stock_max_vote,
        undervolted_levels,
        frequency_changes_by_bin,
        has_non_comparable_bin,
    }
}

fn voltage_label(chip: &str, vote: u32) -> String {
    ltbox_patch::konabess::regulator_level_name(chip, vote)
        .map_or_else(|| vote.to_string(), |name| format!("{name} ({vote})"))
}

fn gpu_summary_view<'a>(
    table: &ltbox_patch::konabess::GpuTable,
    stock: &ltbox_patch::konabess::GpuTable,
    chip: &str,
    app: &'a App,
) -> Element<'a, Message> {
    let summary = gpu_comparison_summary(table, stock, chip);
    let frequency_changes = if summary.frequency_changes_by_bin.is_empty() {
        app.t("konabess_summary_none").to_string()
    } else {
        summary
            .frequency_changes_by_bin
            .iter()
            .map(|(bin, count)| {
                tr_args!(
                    "konabess_summary_frequency_bin",
                    bin = bin.to_string(),
                    count = count.to_string()
                )
            })
            .collect::<Vec<_>>()
            .join(" · ")
    };
    let stock_max = summary.stock_max_vote.map_or_else(
        || app.t("common_unknown").to_string(),
        |vote| voltage_label(chip, vote),
    );
    let mut metrics = row![
        summary_metric(
            app.t("konabess_summary_chip"),
            format!("{chip} · {}", app.device.model),
        ),
        summary_metric(app.t("konabess_summary_stock_max"), stock_max),
        summary_metric(
            app.t("konabess_summary_undervolted"),
            summary.undervolted_levels.to_string(),
        ),
        summary_metric(
            app.t("konabess_summary_frequency_changes"),
            frequency_changes,
        ),
    ]
    .spacing(8.0)
    .width(Length::Fill)
    .align_y(iced::Alignment::Center);
    if summary.has_non_comparable_bin {
        metrics = metrics.push(summary_partial_note(
            app.t("konabess_summary_comparable_only"),
        ));
    }
    metrics.wrap().vertical_spacing(6.0).into()
}

fn summary_metric(label: &str, value: String) -> Element<'static, Message> {
    container(
        row![
            text(label.to_string()).size(11.0).style(muted_style),
            text(value).size(11.0).font(theme::emphasis::medium()),
        ]
        .spacing(5.0)
        .align_y(iced::Alignment::Center),
    )
    .height(Length::Fixed(26.0))
    .padding([0.0, 10.0])
    .align_y(iced::alignment::Vertical::Center)
    .style(summary_metric_style)
    .into()
}

fn summary_partial_note(value: &str) -> Element<'static, Message> {
    container(
        text(value.to_string())
            .size(11.0)
            .style(warning_container_text_style),
    )
    .height(Length::Fixed(26.0))
    .padding([0.0, 10.0])
    .align_y(iced::alignment::Vertical::Center)
    .style(comparison_note_style)
    .into()
}

fn summary_metric_style(theme: &Theme) -> container::Style {
    let palette = pal_of(theme);
    container::Style {
        background: Some(palette.surface_container_high.into()),
        border: iced::Border {
            radius: theme::shape::FULL.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Shortest window that still leaves the table room to fill and scroll behind
/// its own header. Below this the step scrolls as one page instead.
const GPU_TABLE_FILL_MIN_WINDOW_HEIGHT: f32 = 700.0;

fn gpu_table_view<'a>(
    table: &'a ltbox_patch::konabess::GpuTable,
    app: &'a App,
    validation: &ltbox_patch::konabess::GpuTableValidation,
    fill_height: bool,
) -> Element<'a, Message> {
    let columns = gpu_table_columns(table);
    let table_width = GPU_LEVEL_COLUMN_WIDTH
        + columns
            .iter()
            .map(|column| gpu_table_column_width(table, column))
            .sum::<f32>()
        + GPU_ACTION_COLUMN_WIDTH;
    let header = gpu_table_header(table, &columns, app);
    let mut groups = column![].spacing(18.0).width(Length::Fixed(table_width));
    let has_hard_errors = validation.has_hard_errors();
    for (group_position, group) in table.groups.iter().enumerate() {
        let stock_group = app
            .konabess
            .stock_table
            .as_ref()
            .and_then(|stock| comparable_stock_group(group, stock));
        let comparable = stock_group.is_some();
        let has_warning = validation
            .warnings
            .iter()
            .any(|issue| issue_belongs_to_group(issue, group.id));
        let mut add_button = m3_text_button(app.t("konabess_add_level").to_string());
        if !has_hard_errors {
            add_button = add_button.on_press(Message::KonaBess(KonaBessMsg::KonaBessAddLevel(
                group_position,
            )));
        }
        let mut group_label = row![].spacing(5.0).align_y(iced::Alignment::Center);
        if has_warning {
            group_label = group_label.push(lucide_icon(
                icon::banner_warning(),
                13.0,
                |theme: &Theme| pal_of(theme).on_warning_container,
            ));
        }
        group_label = group_label.push(
            text(format!("Bin {}", group.id))
                .size(14.0)
                .font(theme::emphasis::medium())
                .style(move |theme| group_heading_text_style(theme, has_warning)),
        );
        let group_label = container(group_label)
            .padding([4.0, 8.0])
            .style(move |theme| group_heading_style(theme, has_warning));
        let mut group_heading = row![group_label]
            .spacing(8.0)
            .align_y(iced::Alignment::Center)
            .width(Length::Fixed(table_width));
        for property in &group.header_properties {
            group_heading = group_heading.push(group_property_chip(property));
        }
        if !comparable {
            group_heading = group_heading.push(summary_partial_note(
                app.t("konabess_comparison_unavailable"),
            ));
        }
        group_heading = group_heading.push(add_button);
        let group_heading = group_heading.wrap().vertical_spacing(6.0);

        let mut table_rows = column![].spacing(0).width(Length::Fixed(table_width));
        for (level_position, level) in group.levels.iter().enumerate() {
            let mut remove_button = m3_text_button(app.t("konabess_remove_level").to_string());
            if group.levels.len() > 1 && !has_hard_errors {
                remove_button = remove_button.on_press(Message::KonaBess(
                    KonaBessMsg::KonaBessRemoveLevel(group_position, level_position),
                ));
            }
            let mut cells = row![level_cell(group, level_position, level.id, app)].spacing(0);
            for column in &columns {
                cells = cells.push(gpu_table_body_cell(
                    column,
                    table,
                    group_position,
                    level_position,
                    level,
                    stock_group,
                    app,
                    validation,
                ));
            }
            cells = cells.push(
                container(remove_button)
                    .padding([4.0, 5.0])
                    .width(Length::Fixed(GPU_ACTION_COLUMN_WIDTH))
                    .height(Length::Fixed(GPU_TABLE_ROW_HEIGHT))
                    .align_y(iced::alignment::Vertical::Center)
                    .style(table_border_style(false)),
            );
            table_rows = table_rows.push(cells);
        }
        groups = groups.push(column![group_heading, table_rows].spacing(6.0));
    }

    // Everything above the table is fixed height, so a `Fill` table is the
    // first thing squeezed out when the window is short — at the minimum it
    // collapsed to nothing at all. Only claim the remaining height when there
    // is enough of it; otherwise take the natural height and let the step
    // scroll as one page, which the caller arranges.
    let vertical = if fill_height {
        Length::Fill
    } else {
        Length::Shrink
    };
    let body: Element<'_, Message> = if fill_height {
        scrollable(groups)
            .direction(widget::scrollable::Direction::Vertical(
                widget::scrollable::Scrollbar::default(),
            ))
            .style(m3_scrollable_style)
            .width(Length::Fixed(table_width))
            .height(Length::Fill)
            .into()
    } else {
        groups.into()
    };
    let fixed_header_table = column![header, body]
        .spacing(0)
        .width(Length::Fixed(table_width))
        .height(vertical);
    container(
        scrollable(fixed_header_table)
            .direction(widget::scrollable::Direction::Horizontal(
                widget::scrollable::Scrollbar::default(),
            ))
            .style(m3_scrollable_style)
            .width(Length::Fixed(table_width))
            .height(vertical),
    )
    .center_x(Length::Fill)
    .height(vertical)
    .into()
}

fn gpu_table_header<'a>(
    table: &ltbox_patch::konabess::GpuTable,
    columns: &[GpuTableColumn],
    app: &'a App,
) -> Element<'a, Message> {
    let mut header = row![table_header_cell(
        text(app.t("konabess_column_level").to_string()).size(11.0),
        GPU_LEVEL_COLUMN_WIDTH,
    )]
    .spacing(0);
    for column in columns {
        let width = gpu_table_column_width(table, column);
        let content: Element<'a, Message> = match column {
            GpuTableColumn::Frequency => text(app.t("konabess_column_frequency").to_string())
                .size(11.0)
                .into(),
            GpuTableColumn::Voltage => text(app.t("konabess_column_voltage").to_string())
                .size(11.0)
                .into(),
            GpuTableColumn::Delta => text(app.t("konabess_column_delta").to_string())
                .size(11.0)
                .into(),
            GpuTableColumn::Bus(names) => column![
                text(app.t("konabess_column_bus").to_string()).size(11.0),
                text(
                    names
                        .iter()
                        .map(|name| { name.strip_prefix("qcom,bus-").unwrap_or(name).to_string() })
                        .collect::<Vec<_>>()
                        .join(" / "),
                )
                .size(theme::text_size::LABEL_SMALL)
                .font(theme::mono_font())
                .style(muted_style),
            ]
            .spacing(1.0)
            .into(),
            GpuTableColumn::Other(name) => text(property_label(name)).size(11.0).into(),
        };
        header = header.push(table_header_cell(content, width));
    }
    header = header.push(table_header_cell(
        Space::new().width(Length::Shrink),
        GPU_ACTION_COLUMN_WIDTH,
    ));
    header.into()
}

fn table_header_cell<'a>(
    content: impl Into<Element<'a, Message>>,
    width: f32,
) -> Element<'a, Message> {
    container(content)
        .padding([5.0, 9.0])
        .width(Length::Fixed(width))
        .height(Length::Fixed(GPU_TABLE_HEADER_HEIGHT))
        .align_y(iced::alignment::Vertical::Center)
        .style(table_border_style(true))
        .into()
}

fn group_property_chip(property: &ltbox_patch::konabess::GpuProperty) -> Element<'static, Message> {
    debug_assert_eq!(
        gpu_property_editability(GpuPropertyLocation::GroupHeader, &property.name),
        GpuPropertyEditability::ReadOnly,
    );
    container(
        row![
            text(property_label(&property.name))
                .size(theme::text_size::LABEL_SMALL)
                .style(muted_style),
            text(
                property
                    .cells
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(" "),
            )
            .size(theme::text_size::LABEL_SMALL)
            .font(theme::mono_font()),
        ]
        .spacing(5.0)
        .align_y(iced::Alignment::Center),
    )
    .height(Length::Fixed(24.0))
    .padding([0.0, 8.0])
    .align_y(iced::alignment::Vertical::Center)
    .style(summary_metric_style)
    .into()
}

fn gpu_table_column_width(table: &ltbox_patch::konabess::GpuTable, column: &GpuTableColumn) -> f32 {
    match column {
        GpuTableColumn::Frequency => GPU_FREQUENCY_COLUMN_WIDTH,
        GpuTableColumn::Voltage => GPU_VOLTAGE_COLUMN_WIDTH,
        GpuTableColumn::Delta => GPU_DELTA_COLUMN_WIDTH,
        GpuTableColumn::Bus(names) => {
            16.0 + names.len().max(1) as f32 * (GPU_BUS_INPUT_WIDTH + 4.0) - 4.0
        }
        GpuTableColumn::Other(name) => table
            .groups
            .iter()
            .map(|group| property_column_width(group, name))
            .fold(190.0, f32::max),
    }
}

#[allow(clippy::too_many_arguments)]
fn gpu_table_body_cell<'a>(
    column: &GpuTableColumn,
    table: &ltbox_patch::konabess::GpuTable,
    group_position: usize,
    level_position: usize,
    level: &'a ltbox_patch::konabess::GpuLevel,
    stock_group: Option<&ltbox_patch::konabess::GpuGroup>,
    app: &'a App,
    validation: &ltbox_patch::konabess::GpuTableValidation,
) -> Element<'a, Message> {
    let width = gpu_table_column_width(table, column);
    match column {
        GpuTableColumn::Frequency => {
            let property = level
                .properties
                .iter()
                .enumerate()
                .find(|(_, property)| property.name == "qcom,gpu-freq");
            let stock_frequency = stock_group
                .and_then(|group| group.levels.get(level_position))
                .and_then(|level| scalar_property(&level.properties, "qcom,gpu-freq"));
            match property {
                Some((property_position, property)) => frequency_property_cell(
                    property,
                    |cell| {
                        GpuCellKey::level(group_position, level_position, property_position, cell)
                    },
                    stock_frequency,
                    width,
                    app,
                    validation,
                ),
                None => table_cell("—".to_string(), false, width),
            }
        }
        GpuTableColumn::Voltage => {
            let property = level
                .properties
                .iter()
                .enumerate()
                .find(|(_, property)| property.name == "qcom,level");
            match property {
                Some((property_position, property)) => voltage_property_cell(
                    property,
                    |cell| {
                        GpuCellKey::level(group_position, level_position, property_position, cell)
                    },
                    width,
                    app,
                    validation,
                ),
                None => table_cell("—".to_string(), false, width),
            }
        }
        GpuTableColumn::Delta => {
            let delta = stock_group
                .and_then(|group| group.levels.get(level_position))
                .and_then(|stock_level| {
                    Some((
                        scalar_property(&level.properties, "qcom,level")?,
                        scalar_property(&stock_level.properties, "qcom,level")?,
                    ))
                })
                .and_then(|(edited, stock)| {
                    voltage_delta(app.konabess.selected_chip()?, edited, stock)
                });
            delta_cell(delta, width, app)
        }
        GpuTableColumn::Bus(names) => bus_property_cell(
            names,
            group_position,
            level_position,
            level,
            width,
            app,
            validation,
        ),
        GpuTableColumn::Other(name) => {
            let property = level
                .properties
                .iter()
                .enumerate()
                .find(|(_, property)| property.name == *name);
            match property {
                Some((property_position, property)) => editable_property_cell(
                    property,
                    |cell| {
                        GpuCellKey::level(group_position, level_position, property_position, cell)
                    },
                    width,
                    app,
                    validation,
                ),
                None => table_cell("—".to_string(), false, width),
            }
        }
    }
}

fn level_cell<'a>(
    group: &ltbox_patch::konabess::GpuGroup,
    level_position: usize,
    level_id: u32,
    app: &'a App,
) -> Element<'a, Message> {
    let initial = scalar_property(&group.header_properties, "qcom,initial-pwrlevel")
        == u32::try_from(level_position).ok();
    let floor = scalar_property(&group.header_properties, "qcom,initial-min-pwrlevel")
        == u32::try_from(level_position).ok();
    let mut content = row![
        text(level_id.to_string())
            .size(12.0)
            .font(theme::mono_font())
            .style(muted_style),
    ]
    .spacing(5.0)
    .align_y(iced::Alignment::Center);
    if initial {
        content = content.push(level_badge(app.t("konabess_badge_default"), true));
    }
    if floor {
        content = content.push(level_badge(app.t("konabess_badge_floor"), false));
    }
    container(content)
        .padding([4.0, 8.0])
        .width(Length::Fixed(GPU_LEVEL_COLUMN_WIDTH))
        .height(Length::Fixed(GPU_TABLE_ROW_HEIGHT))
        .align_y(iced::alignment::Vertical::Center)
        .style(derived_table_cell_style)
        .into()
}

fn level_badge(value: &str, initial: bool) -> Element<'static, Message> {
    container(
        text(value.to_string())
            .size(theme::text_size::LABEL_SMALL)
            .font(theme::emphasis::medium()),
    )
    .height(Length::Fixed(18.0))
    .padding([0.0, 6.0])
    .align_y(iced::alignment::Vertical::Center)
    .style(move |theme: &Theme| {
        let palette = pal_of(theme);
        let (background, foreground) = if initial {
            (palette.primary_container, palette.on_primary_container)
        } else {
            (palette.surface_container_high, palette.on_surface_variant)
        };
        container::Style {
            background: Some(background.into()),
            text_color: Some(foreground),
            border: iced::Border {
                radius: theme::shape::FULL.into(),
                ..Default::default()
            },
            ..Default::default()
        }
    })
    .into()
}

fn frequency_property_cell<'a>(
    property: &ltbox_patch::konabess::GpuProperty,
    key_for_cell: impl Fn(usize) -> GpuCellKey,
    stock_frequency: Option<u32>,
    width: f32,
    app: &'a App,
    validation: &ltbox_patch::konabess::GpuTableValidation,
) -> Element<'a, Message> {
    let current_frequency = (property.cells.len() == 1).then(|| property.cells[0]);
    let inputs = property_inputs(property, key_for_cell, width - 16.0, app, validation);
    let mut content = column![inputs].spacing(1.0);
    if let (Some(current), Some(stock)) = (current_frequency, stock_frequency)
        && current != stock
    {
        content = content.push(
            text(tr_args!(
                "konabess_stock_hint",
                value = format_frequency_mhz(stock)
            ))
            .size(theme::text_size::LABEL_SMALL)
            .font(theme::mono_font())
            .style(muted_style),
        );
    }
    container(content)
        .padding([4.0, 8.0])
        .width(Length::Fixed(width))
        .height(Length::Fixed(GPU_TABLE_ROW_HEIGHT))
        .align_y(iced::alignment::Vertical::Center)
        .style(table_border_style(false))
        .into()
}

fn voltage_property_cell<'a>(
    property: &ltbox_patch::konabess::GpuProperty,
    key_for_cell: impl Fn(usize) -> GpuCellKey,
    width: f32,
    app: &'a App,
    validation: &ltbox_patch::konabess::GpuTableValidation,
) -> Element<'a, Message> {
    let mut controls = row![].spacing(7.0).align_y(iced::Alignment::Center);
    for (cell_position, committed) in property.cells.iter().copied().enumerate() {
        let key = key_for_cell(cell_position);
        let (hard_error, warning) = cell_validation_state(app, validation, key);
        if let Some(chip) = app.konabess.selected_chip()
            && let Some(options) = regulator_vote_choices(chip, committed)
        {
            let selected = RegulatorVoteChoice::new(chip, committed);
            let keyboard_options = options.clone();
            let picker = widget::pick_list(options, Some(selected.clone()), move |choice| {
                Message::KonaBess(KonaBessMsg::KonaBessCellChanged(
                    key,
                    choice.vote.to_string(),
                ))
            })
            .text_size(12.0)
            .font(theme::emphasis::medium())
            .padding([7.0, 8.0])
            .style(move |theme: &Theme, status| {
                gpu_picker_style(theme, status, hard_error, warning)
            })
            .menu_style(m3_pick_list_menu_style)
            .width(Length::Fixed(158.0));
            controls = controls.push(focus_button::cycle(
                picker,
                format!("gpu-vote-{key:?}"),
                &keyboard_options,
                &selected,
                |choice| {
                    Message::KonaBess(KonaBessMsg::KonaBessCellChanged(
                        key,
                        choice.vote.to_string(),
                    ))
                },
            ));
            if selected.name.is_some() {
                controls = controls.push(
                    text(committed.to_string())
                        .size(11.0)
                        .font(theme::mono_font())
                        .style(muted_style),
                );
            }
            continue;
        }
        controls = controls.push(gpu_text_input(
            app.konabess.cell_text(key, committed, &property.name),
            key,
            158.0,
            hard_error,
            warning,
        ));
    }
    container(controls)
        .padding([4.0, 8.0])
        .width(Length::Fixed(width))
        .height(Length::Fixed(GPU_TABLE_ROW_HEIGHT))
        .align_y(iced::alignment::Vertical::Center)
        .style(table_border_style(false))
        .into()
}

fn delta_cell<'a>(delta: Option<VoltageDelta>, width: f32, app: &'a App) -> Element<'a, Message> {
    let badge = delta.map(|delta| {
        let (label, tone) = match delta {
            VoltageDelta::Stock => (app.t("konabess_delta_stock").to_string(), DeltaTone::Stock),
            VoltageDelta::Down(steps) => (
                tr_args!("konabess_delta_down", count = steps.to_string()),
                DeltaTone::Down,
            ),
            VoltageDelta::Up(steps) => (
                tr_args!("konabess_delta_up", count = steps.to_string()),
                DeltaTone::Up,
            ),
        };
        container(
            text(label)
                .size(theme::text_size::LABEL_SMALL)
                .font(theme::emphasis::medium())
                .wrapping(iced::widget::text::Wrapping::None),
        )
        .height(Length::Fixed(22.0))
        .padding([0.0, 8.0])
        .align_y(iced::alignment::Vertical::Center)
        .style(move |theme: &Theme| delta_badge_style(theme, tone))
    });
    let content: Element<'a, Message> =
        badge.map_or_else(|| Space::new().width(Length::Shrink).into(), Into::into);
    container(content)
        .padding([4.0, 8.0])
        .width(Length::Fixed(width))
        .height(Length::Fixed(GPU_TABLE_ROW_HEIGHT))
        .align_y(iced::alignment::Vertical::Center)
        .style(table_border_style(false))
        .into()
}

#[derive(Debug, Clone, Copy)]
enum DeltaTone {
    Stock,
    Down,
    Up,
}

fn delta_badge_style(theme: &Theme, tone: DeltaTone) -> container::Style {
    let palette = pal_of(theme);
    let (background, foreground) = match tone {
        DeltaTone::Stock => (palette.surface_container_high, palette.on_surface_variant),
        DeltaTone::Down => (palette.secondary_container, palette.on_secondary_container),
        DeltaTone::Up => (palette.warning_container, palette.on_warning_container),
    };
    container::Style {
        background: Some(background.into()),
        text_color: Some(foreground),
        border: iced::Border {
            radius: theme::shape::FULL.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn bus_property_cell<'a>(
    names: &[String],
    group_position: usize,
    level_position: usize,
    level: &'a ltbox_patch::konabess::GpuLevel,
    width: f32,
    app: &'a App,
    validation: &ltbox_patch::konabess::GpuTableValidation,
) -> Element<'a, Message> {
    let mut controls = row![].spacing(4.0).align_y(iced::Alignment::Center);
    for name in names {
        let property = level
            .properties
            .iter()
            .enumerate()
            .find(|(_, property)| property.name == *name);
        controls = controls.push(match property {
            Some((property_position, property)) => property_inputs(
                property,
                |cell| GpuCellKey::level(group_position, level_position, property_position, cell),
                GPU_BUS_INPUT_WIDTH,
                app,
                validation,
            )
            .into(),
            None => derived_value_cell_with_width("—".to_string(), GPU_BUS_INPUT_WIDTH),
        });
    }
    container(controls)
        .padding([4.0, 8.0])
        .width(Length::Fixed(width))
        .height(Length::Fixed(GPU_TABLE_ROW_HEIGHT))
        .align_y(iced::alignment::Vertical::Center)
        .style(table_border_style(false))
        .into()
}

fn format_frequency_mhz(frequency_hz: u32) -> String {
    const HZ_PER_MHZ: u32 = 1_000_000;
    let whole = frequency_hz / HZ_PER_MHZ;
    let remainder = frequency_hz % HZ_PER_MHZ;
    if remainder == 0 {
        whole.to_string()
    } else {
        format!("{whole}.{remainder:06}")
            .trim_end_matches('0')
            .to_string()
    }
}

fn table_cell(value: String, header: bool, width: f32) -> Element<'static, Message> {
    container(
        text(value)
            .size(if header { 11.0 } else { 12.0 })
            .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
    )
    .padding([7.0, 9.0])
    .width(Length::Fixed(width))
    .height(Length::Fixed(if header {
        GPU_TABLE_HEADER_HEIGHT
    } else {
        GPU_TABLE_ROW_HEIGHT
    }))
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
    let inputs = property_inputs(property, key_for_cell, width - 16.0, app, validation);
    container(inputs)
        .padding([7.0, 8.0])
        .width(Length::Fixed(width))
        .height(Length::Fixed(GPU_TABLE_ROW_HEIGHT))
        .align_y(iced::alignment::Vertical::Center)
        .style(table_border_style(false))
        .into()
}

fn property_inputs<'a>(
    property: &ltbox_patch::konabess::GpuProperty,
    key_for_cell: impl Fn(usize) -> GpuCellKey,
    available_width: f32,
    app: &'a App,
    validation: &ltbox_patch::konabess::GpuTableValidation,
) -> iced::widget::Row<'a, Message> {
    let mut inputs = row![].spacing(6.0);
    let gaps = property.cells.len().saturating_sub(1) as f32 * 6.0;
    let field_width = ((available_width - gaps) / property.cells.len().max(1) as f32).max(52.0);
    for (cell_position, committed) in property.cells.iter().copied().enumerate() {
        let key = key_for_cell(cell_position);
        let value = app.konabess.cell_text(key, committed, &property.name);
        if gpu_property_editability(GpuPropertyLocation::Level, &property.name)
            == GpuPropertyEditability::ReadOnly
        {
            inputs = inputs.push(derived_value_cell_with_width(value, field_width));
            continue;
        }
        let (hard_error, warning) = cell_validation_state(app, validation, key);
        if matches!(property.name.as_str(), "qcom,level" | "qcom,cx-level")
            && let Some(chip) = app.konabess.selected_chip()
            && let Some(options) = regulator_vote_choices(chip, committed)
        {
            let selected = RegulatorVoteChoice::new(chip, committed);
            let keyboard_options = options.clone();
            let picker = widget::pick_list(options, Some(selected.clone()), move |choice| {
                Message::KonaBess(KonaBessMsg::KonaBessCellChanged(
                    key,
                    choice.vote.to_string(),
                ))
            })
            .text_size(12.0)
            .padding([7.0, 8.0])
            .style(move |theme: &Theme, status| {
                gpu_picker_style(theme, status, hard_error, warning)
            })
            .menu_style(m3_pick_list_menu_style)
            .width(Length::Fixed(field_width));
            inputs = inputs.push(focus_button::cycle(
                picker,
                format!("gpu-vote-{key:?}"),
                &keyboard_options,
                &selected,
                |choice| {
                    Message::KonaBess(KonaBessMsg::KonaBessCellChanged(
                        key,
                        choice.vote.to_string(),
                    ))
                },
            ));
            continue;
        }
        inputs = inputs.push(gpu_text_input(value, key, field_width, hard_error, warning));
    }
    inputs
}

fn cell_validation_state(
    app: &App,
    validation: &ltbox_patch::konabess::GpuTableValidation,
    key: GpuCellKey,
) -> (bool, bool) {
    let hard_error = app.konabess.cell_has_input_error(key)
        || validation
            .hard_errors
            .iter()
            .any(|issue| app.konabess.issue_matches_cell(issue, key));
    let warning = !hard_error
        && validation
            .warnings
            .iter()
            .any(|issue| app.konabess.issue_matches_cell(issue, key));
    (hard_error, warning)
}

fn gpu_picker_style(
    theme: &Theme,
    status: widget::pick_list::Status,
    hard_error: bool,
    warning: bool,
) -> widget::pick_list::Style {
    let mut style = m3_pick_list_style(theme, status);
    if hard_error {
        style.border.color = pal_of(theme).error;
        style.border.width = 2.0;
    } else if warning {
        style.border.color = pal_of(theme).warning;
        style.border.width = 2.0;
    }
    style
}

fn gpu_text_input<'a>(
    value: String,
    key: GpuCellKey,
    width: f32,
    hard_error: bool,
    warning: bool,
) -> Element<'a, Message> {
    widget::text_input("", &value)
        .on_input(move |text| Message::KonaBess(KonaBessMsg::KonaBessCellChanged(key, text)))
        .padding([7.0, 8.0])
        .size(12.0)
        .font(theme::mono_font())
        .width(Length::Fixed(width))
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
        })
        .into()
}

fn derived_value_cell_with_width(value: String, width: f32) -> Element<'static, Message> {
    container(text(value).size(12.0))
        .padding([7.0, 8.0])
        .width(Length::Fixed(width))
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
        iced::widget::text::Style {
            color: Some(pal_of(theme).on_secondary_container),
        }
    }
}

fn group_heading_style(theme: &Theme, warning: bool) -> container::Style {
    let palette = pal_of(theme);
    let (background, foreground, border) = if warning {
        (
            palette.warning_container,
            palette.on_warning_container,
            palette.warning,
        )
    } else {
        (
            palette.secondary_container,
            palette.on_secondary_container,
            palette.secondary_container,
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
}

fn comparison_note_style(theme: &Theme) -> container::Style {
    let palette = pal_of(theme);
    container::Style {
        background: Some(palette.warning_container.into()),
        text_color: Some(palette.on_warning_container),
        border: iced::Border {
            color: palette.warning,
            width: 1.0,
            radius: theme::shape::FULL.into(),
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
            Some(name) => name.fmt(formatter),
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
    let (severity, icon_glyph, title) = if warning {
        (
            BannerSeverity::Warning,
            icon::banner_warning(),
            tr_args!("konabess_warning_summary", count = count.to_string()),
        )
    } else {
        (
            BannerSeverity::Error,
            icon::banner_error(),
            tr_args!("konabess_error_summary", count = count.to_string()),
        )
    };
    let mut details = column![].spacing(3.0);
    if !warning {
        for issue in issues {
            details = details.push(
                text(localized_issue(issue, false, app))
                    .size(11.0)
                    .style(error_container_text_style),
            );
        }
    }
    app.message_banner(severity, icon_glyph, title, details)
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
    } else if issue.message.contains("unknown export field") {
        "konabess_warning_unknown_export_field"
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
        GpuGroup, GpuLevel, GpuProperty, GpuTable, GpuTableIssue, VendorBootDtbInfo,
    };

    fn comparison_level(id: u32, frequency: u32, vote: u32) -> GpuLevel {
        GpuLevel {
            id,
            properties: vec![
                GpuProperty {
                    name: "reg".into(),
                    cells: vec![id],
                },
                GpuProperty {
                    name: "qcom,gpu-freq".into(),
                    cells: vec![frequency],
                },
                GpuProperty {
                    name: "qcom,level".into(),
                    cells: vec![vote],
                },
            ],
        }
    }

    fn comparison_group(id: u32, levels: Vec<GpuLevel>) -> GpuGroup {
        GpuGroup {
            id,
            header_properties: vec![],
            levels,
        }
    }

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
    fn regulator_picker_uses_names_while_unknown_values_remain_editable() {
        let choices = regulator_vote_choices("sun", 51).expect("sun has an upstream mapping");
        assert!(choices.iter().any(|choice| choice.to_string() == "NOM"));
        assert!(choices.iter().any(|choice| choice.to_string() == "51"));
    }

    #[test]
    fn stock_comparison_uses_row_index_only_when_bin_level_counts_match() {
        let stock = GpuTable {
            groups: vec![
                comparison_group(
                    0,
                    vec![
                        comparison_level(0, 900_000_000, 448),
                        comparison_level(1, 800_000_000, 452),
                    ],
                ),
                comparison_group(
                    1,
                    vec![
                        comparison_level(0, 700_000_000, 432),
                        comparison_level(1, 600_000_000, 416),
                    ],
                ),
            ],
        };
        let edited = GpuTable {
            groups: vec![
                comparison_group(
                    0,
                    vec![
                        comparison_level(0, 900_000_000, 432),
                        comparison_level(1, 825_000_000, 452),
                    ],
                ),
                comparison_group(1, vec![comparison_level(0, 750_000_000, 384)]),
            ],
        };

        assert!(comparable_stock_group(&edited.groups[0], &stock).is_some());
        assert!(comparable_stock_group(&edited.groups[1], &stock).is_none());
        assert_eq!(voltage_delta("sun", 432, 448), Some(VoltageDelta::Down(1)));
        assert_eq!(voltage_delta("sun", 452, 432), Some(VoltageDelta::Up(2)));
        assert_eq!(voltage_delta("sun", 452, 452), Some(VoltageDelta::Stock));
        assert_eq!(
            gpu_comparison_summary(&edited, &stock, "sun"),
            GpuComparisonSummary {
                stock_max_vote: Some(452),
                undervolted_levels: 1,
                frequency_changes_by_bin: vec![(0, 1)],
                has_non_comparable_bin: true,
            }
        );
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
