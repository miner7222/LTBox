//! System-update wizard view + steps + the shared exec-step view. Extracted from `main.rs`.

use crate::focus_button::button;
use crate::*;
use iced::widget::{column, container, row, text};
use iced::{Element, Length, Theme};

fn cumulative_flash_percent(snapshot: &ltbox_device::edl::FlashProgress) -> u8 {
    if snapshot.operation_total_bytes == 0 {
        return 0;
    }
    ((u128::from(snapshot.operation_completed_bytes) * 100
        / u128::from(snapshot.operation_total_bytes))
    .min(100)) as u8
}

fn current_partition_size(snapshot: &ltbox_device::edl::FlashProgress) -> Option<String> {
    (snapshot.total_bytes > 0).then(|| format_bytes_auto(snapshot.total_bytes))
}
use ltbox_core::tr_args;

/// Height of the rule separating two metric cells.
const METRIC_DIVIDER_HEIGHT: f32 = 34.0;

/// Divider between metric cells. A bare `rule::vertical` asks for `Fill`
/// height, which made the whole metrics strip stretch down the card and
/// leave its values stranded at the top.
fn metric_divider() -> Element<'static, Message> {
    container(iced::widget::rule::vertical(1).style(shell_rule_style))
        .height(Length::Fixed(METRIC_DIVIDER_HEIGHT))
        .into()
}

fn format_exec_duration(duration: std::time::Duration) -> String {
    let seconds = duration.as_secs();
    let hours = seconds / 3_600;
    let minutes = (seconds % 3_600) / 60;
    let seconds = seconds % 60;
    if hours > 0 {
        format!("{hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

impl App {
    pub(crate) fn view_sysupdate_wizard(&self) -> Element<'_, Message> {
        // Exec-step log popup overlay — without this the "Show log" button
        // on the exec card was a no-op for System Update (Flash/Root/Unroot
        // all had it wired; SysUpdate had been missed).
        if self.log_popup_open && self.sysupdate.is_in_exec() {
            return self.log_popup_view();
        }
        let steps = self.sysupdate.steps();
        let step_labels: Vec<&str> = steps.iter().map(|k| self.t(k)).collect();
        let is_exec = self.sysupdate.is_in_exec();
        let step_bar = if is_exec {
            empty_wizard_step_bar()
        } else {
            wizard_step_bar(&step_labels, self.sysupdate.step, self.window_size_class())
        };
        let is_rescue = self.sysupdate.is_rescue();
        let body = if is_rescue {
            match self.sysupdate.step {
                0 => self.sysupdate_action_step(),
                1 => self.sysupdate_rescue_folder_step(),
                2 => self.sysupdate_confirm_step(),
                _ => self.sysupdate_exec_step(),
            }
        } else {
            match self.sysupdate.step {
                0 => self.sysupdate_action_step(),
                1 => self.sysupdate_confirm_step(),
                _ => self.sysupdate_exec_step(),
            }
        };
        let (step_title, app_bar_subtitle) = self.sysupdate_step_copy();
        let body = if is_exec || self.sysupdate.step == 0 {
            body
        } else {
            if self.sysupdate.is_rescue() && self.sysupdate.step == 1 {
                self.wizard_picker_step(step_title, body)
            } else {
                wizard_step_body(step_title, body)
            }
        };
        let last_nav_step = steps.len() - 2; // Exec step has no nav row.
        let nav = if self.sysupdate.step <= last_nav_step {
            let is_start = self.sysupdate.step == last_nav_step;
            let label_owned = if is_start {
                self.t("btn_start").to_string()
            } else {
                self.t("btn_next").to_string()
            };
            let can = self.sysupdate.can_next()
                && !(self.operation.is_running() && is_start)
                && (!is_start || self.device_reachable());
            wizard_nav_generic(
                self.sysupdate.step > 0,
                &label_owned,
                can,
                self.t("btn_back"),
                Message::Sys(SysMsg::SysBack),
                Message::Sys(SysMsg::SysNext),
            )
        } else {
            empty_wizard_nav()
        };
        let core: Element<'_, Message> = column![
            wizard_action_bar(
                self.window_size_class(),
                self.t("nav_sysupdate").to_string(),
                app_bar_subtitle,
            ),
            step_bar,
            body,
            nav,
        ]
        .width(Length::Fill)
        .height(Length::Fill)
        .into();
        if self.sysupdate.rescue_region_popup_open {
            iced::widget::Stack::with_children(vec![core, self.rescue_region_popup_view()]).into()
        } else {
            core
        }
    }

    fn sysupdate_step_copy(&self) -> (String, Option<String>) {
        let rescue = self.sysupdate.is_rescue();
        match (rescue, self.sysupdate.step) {
            (_, 0) => (self.t("sysupdate_action_title").to_string(), None),
            (true, 1) => (
                self.t("edl_loader_title").to_string(),
                Some(self.loader_picker_subtitle()),
            ),
            (true, 2) | (false, 1) => (self.t("sysupdate_confirm_title").to_string(), None),
            _ => {
                let (title, _) = self.exec_status_copy();
                (title, self.exec_app_bar_subtitle())
            }
        }
    }

    pub(crate) fn sysupdate_action_step(&self) -> Element<'_, Message> {
        let size_class = self.window_size_class();
        let content_width = self.window_size.0
            - match size_class {
                WindowSizeClass::Compact => SIDEBAR_RAIL_WIDTH,
                WindowSizeClass::Expanded => SIDEBAR_EXPANDED_WIDTH,
            };
        let icon_size = self.wizard_list_icon(WIZARD_LIST_GLYPH_ICON_SIZE);
        let metrics = self.wizard_list_metrics(WIZARD_LIST_LABEL_SIZE, WIZARD_LIST_DESC_SIZE);
        let off_icon = lucide_list_primary(icon::tile_update_off(), icon_size);
        let on_icon = lucide_list_primary(icon::tile_update_on(), icon_size);
        // TB323FU's vendor_boot/vbmeta sit on a different UFS LUN than the
        // Boot Recovery worker targets, so the flow can't run on it — disable
        // the row (alongside the non-Qualcomm platform gate).
        let rescue_disabled =
            self.device.platform_supported == Some(false) || !self.model_capabilities().rescue;
        // Gray the icon when disabled, matching the other wizards' disabled
        // list rows.
        let rescue_icon = if rescue_disabled {
            lucide_list_disabled(icon::tile_rescue(), icon_size)
        } else {
            lucide_list_primary(icon::tile_rescue(), icon_size)
        };
        let mut cards = column![
            wizard_list_option_card(
                off_icon,
                self.t(SysUpdateAction::Disable.label_key()),
                self.t(SysUpdateAction::Disable.desc_key()),
                self.sysupdate.action == Some(SysUpdateAction::Disable),
                Some(Message::Sys(SysMsg::SysAction(SysUpdateAction::Disable))),
                metrics,
            ),
            wizard_list_option_card(
                on_icon,
                self.t(SysUpdateAction::Enable.label_key()),
                self.t(SysUpdateAction::Enable.desc_key()),
                self.sysupdate.action == Some(SysUpdateAction::Enable),
                Some(Message::Sys(SysMsg::SysAction(SysUpdateAction::Enable))),
                metrics,
            ),
        ]
        .spacing(8.0)
        .width(Length::Fill);
        let rescue_sub = if rescue_disabled {
            if self.requires_sahara_manifest() {
                tr_args!("model_unsupported", model = self.device.model.as_str())
            } else if self.is_xiaoxin_pro13() {
                tr_args!("model_unsupported", model = "TB376FC / TB390FU")
            } else {
                self.t("sysupdate_rescue_req").to_string()
            }
        } else {
            self.t(SysUpdateAction::Rescue.desc_key()).to_string()
        };
        cards = cards.push(wizard_list_option_card(
            rescue_icon,
            self.t(SysUpdateAction::Rescue.label_key()),
            &rescue_sub,
            self.sysupdate.action == Some(SysUpdateAction::Rescue),
            (!rescue_disabled).then_some(Message::Sys(SysMsg::SysAction(SysUpdateAction::Rescue))),
            metrics,
        ));
        wizard_selection_step(
            size_class,
            content_width,
            self.t("sysupdate_action_title").to_string(),
            cards.into(),
            Some((self.t("sysupdate_action_title").to_string(), vec![])),
        )
    }

    pub(crate) fn sysupdate_confirm_step(&self) -> Element<'_, Message> {
        let dash = "—".to_string();
        let action = self
            .sysupdate
            .action
            .map(|a| self.t(a.label_key()).to_string())
            .unwrap_or_else(|| dash.clone());
        let mut grid_rows = vec![confirm_definition_row(
            self.t("sysupdate_step_action"),
            &action,
        )];
        let mut trailing_rows = Vec::new();
        // Rescue: echo the chosen firmware folder + region so the user
        // confirms exactly what's about to flash.
        if self.sysupdate.is_rescue() {
            let folder = self
                .sysupdate
                .rescue_folder
                .clone()
                .unwrap_or_else(|| dash.clone());
            let region = self
                .sysupdate
                .rescue_region
                .map(|r| self.t(r.label_key()).to_string())
                .unwrap_or_else(|| dash.clone());
            trailing_rows.push(confirm_path_row(self.t("edl_loader_label"), &folder));
            grid_rows.push(confirm_definition_row(
                self.t("rescue_region_label"),
                &region,
            ));
        }
        self.confirm_step_frame(vec![], grid_rows, trailing_rows)
    }

    pub(crate) fn sysupdate_rescue_folder_step(&self) -> Element<'_, Message> {
        self.loader_picker_card(
            &self.sysupdate.rescue_folder,
            None,
            Message::Sys(SysMsg::SysRescueSelectFolder),
            |p| Message::Sys(SysMsg::SysRescueFolderChosen(Some(p))),
        )
    }

    pub(crate) fn sysupdate_exec_step(&self) -> Element<'_, Message> {
        self.exec_step_view()
    }

    pub(crate) fn exec_status_copy(&self) -> (String, String) {
        if self.operation.is_running() {
            (
                self.t("exec_executing_title").to_string(),
                self.t("exec_executing_subtitle").to_string(),
            )
        } else if self.operation_error.is_some() {
            (
                self.t("exec_failed_title").to_string(),
                self.t("exec_failed_subtitle").to_string(),
            )
        } else {
            (
                self.t("exec_done_title").to_string(),
                self.t("exec_done_subtitle").to_string(),
            )
        }
    }

    /// The execution app bar retains only the safety guidance from the
    /// shared status copy. Completion and failure details stay in the body.
    pub(crate) fn exec_app_bar_subtitle(&self) -> Option<String> {
        self.operation
            .is_running()
            .then(|| self.t("exec_executing_subtitle").to_string())
    }

    /// Shared execution view: current activity, operation-phase checklist,
    /// and a continuously mounted log.
    pub(crate) fn exec_step_view(&self) -> Element<'_, Message> {
        self.exec_step_view_layout()
    }

    /// Kept as the full-flash call-site name; every flow now uses the same
    /// live-log execution layout.
    pub(crate) fn exec_step_view_with_inline_log(&self) -> Element<'_, Message> {
        self.exec_step_view_layout()
    }

    fn exec_step_view_layout(&self) -> Element<'_, Message> {
        let (_, detail) = self.exec_status_copy();
        let is_error = self.operation_error.is_some();
        let is_busy = self.operation.is_running();
        let phase_kind = self.operation.phase_kind();
        let current_phase = self.operation.current_step();
        let phase_percent = self
            .firmware_write_progress_phase_active()
            .then(|| self.flash_progress.as_ref().map(cumulative_flash_percent))
            .flatten()
            // Full-flash overlays are generated in several batches, so their
            // final byte denominator is not known at the phase boundary. Keep
            // that phase at its weighted start rather than letting a growing
            // denominator move the overall track backwards. The rawprogram
            // and Simple Flash plans are known up front and remain determinate.
            .filter(|_| !(phase_kind == Some(OperationPhaseKind::Flash) && current_phase == 7));
        let progress = phase_kind.map_or_else(
            || {
                operation_progress_fraction(
                    current_phase,
                    self.operation.steps.len(),
                    phase_percent,
                    !is_busy && !is_error,
                )
            },
            |kind| kind.progress_fraction(current_phase, phase_percent, !is_busy && !is_error),
        );
        let progress_pct = (progress * 100.0).round() as u8;
        let current_step = self
            .operation
            .steps
            .get(current_phase.min(self.operation.steps.len().saturating_sub(1)));
        let phase_label = current_step
            .map(|step| step.label.clone())
            .unwrap_or_else(|| detail.clone());
        let show_partition_progress = self.firmware_flash_progress_label().is_some();
        let write_progress = self
            .flash_progress
            .as_ref()
            .filter(|snapshot| show_partition_progress && !snapshot.partition.is_empty());
        let now_label = write_progress
            .map(|snapshot| snapshot.partition.clone())
            .unwrap_or(phase_label);
        let byte_progress = write_progress
            .filter(|snapshot| snapshot.operation_total_bytes > 0)
            .map(|snapshot| {
                format!(
                    "{} / {}",
                    format_bytes_auto(snapshot.operation_completed_bytes),
                    format_bytes_auto(snapshot.operation_total_bytes)
                )
            });
        let transferred = byte_progress.clone().unwrap_or_else(|| "—".to_string());

        let mut now_copy = column![
            text(now_label)
                .size(theme::text_size::TITLE_MEDIUM)
                .font(theme::emphasis::medium())
                .style(on_surface_style)
                .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
        ]
        .spacing(3.0)
        .width(Length::Fill);
        // Only when the checklist is not on screen. Showing both would state
        // the same position twice, which is what the step bar used to do.
        if self.window_size_class() == WindowSizeClass::Compact && !self.operation.steps.is_empty()
        {
            now_copy = now_copy.push(
                text(tr_args!(
                    "exec_step_position",
                    n = self.operation.current_step().saturating_add(1),
                    total = self.operation.steps.len()
                ))
                .size(theme::text_size::BODY_SMALL)
                .style(muted_style),
            );
        }
        if let Some(bytes) = write_progress.and_then(current_partition_size) {
            now_copy = now_copy.push(
                text(bytes)
                    .size(theme::text_size::BODY_SMALL)
                    .style(muted_style),
            );
        }
        if is_error && let Some(error) = self.operation_error.as_deref() {
            let summary = concise_error_summary(error, EXEC_ERROR_SUMMARY_MAX_CHARS);
            if !summary.is_empty() {
                now_copy = now_copy.push(
                    text(summary)
                        .size(theme::text_size::BODY_SMALL)
                        .style(|t: &Theme| iced::widget::text::Style {
                            color: Some(pal_of(t).error),
                        })
                        .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
                );
            }
        }

        let elapsed = self.operation.elapsed();
        let transport_hint = self
            .operation
            .phase_kind()
            .map(|kind| kind.transport_hint(self.operation.current_step()))
            .unwrap_or(OperationTransportHint::Current);
        let transport = match transport_hint {
            OperationTransportHint::Current if self.device.connection == ConnectionStatus::None => {
                "—".to_string()
            }
            OperationTransportHint::Current => self.t(self.connection_label_key()).to_string(),
            OperationTransportHint::Adb => self.t("conn_adb").to_string(),
            OperationTransportHint::Fastboot => self.t("conn_fastboot").to_string(),
            OperationTransportHint::Edl => self.t("conn_edl").to_string(),
            OperationTransportHint::Disconnected => "—".to_string(),
        };
        let metric = |label: String, value: String| {
            container(
                column![
                    text(label).size(11.0).style(muted_style),
                    text(value)
                        .size(12.0)
                        .font(theme::emphasis::medium())
                        .wrapping(iced::widget::text::Wrapping::None),
                ]
                .spacing(2.0),
            )
            .padding([10.0, 14.0])
            .width(Length::FillPortion(1))
        };
        let metrics = container(
            row![
                metric(self.t("exec_metric_transport").to_string(), transport,),
                metric_divider(),
                metric(self.t("exec_metric_transferred").to_string(), transferred),
                metric_divider(),
                metric(
                    self.t("exec_metric_elapsed").to_string(),
                    format_exec_duration(elapsed),
                ),
            ]
            .spacing(0)
            .width(Length::Fill),
        )
        .width(Length::Fill)
        .style(|t: &Theme| container::Style {
            border: iced::Border {
                color: pal_of(t).outline_variant,
                width: 1.0,
                radius: theme::shape::MD.into(),
            },
            ..Default::default()
        });
        let step_card = container(
            column![
                row![
                    now_copy,
                    text(format!("{progress_pct}%"))
                        .size(theme::text_size::TITLE_MEDIUM)
                        .font(theme::emphasis::medium()),
                ]
                .spacing(16.0)
                .align_y(iced::Alignment::Start),
                iced::widget::progress_bar(0.0..=1.0, progress)
                    .girth(6)
                    .style(|t: &Theme| {
                        let p = pal_of(t);
                        iced::widget::progress_bar::Style {
                            background: p.surface_container_highest.into(),
                            bar: p.primary.into(),
                            border: iced::Border {
                                radius: theme::shape::FULL.into(),
                                ..Default::default()
                            },
                        }
                    }),
                metrics,
            ]
            .spacing(14.0)
            .width(Length::Fill),
        )
        .padding([16.0, 18.0])
        .width(Length::Fill)
        .style(|t: &Theme| {
            theme::surface_card_style(t, theme::SurfaceLevel::Default, theme::shape::MD)
        });

        let operation_complete = !is_busy && !is_error;
        let current_index = self
            .operation
            .current_step()
            .min(self.operation.steps.len().saturating_sub(1));
        let mut checklist_rows = column![].spacing(0).width(Length::Fill);
        for (index, step) in self.operation.steps.iter().enumerate() {
            let state = if operation_complete {
                WizardStepState::Completed
            } else {
                wizard_step_state(index, current_index)
            };
            let marker_text = if state == WizardStepState::Completed {
                "\u{2713}".to_string()
            } else {
                (index + 1).to_string()
            };
            let marker = container(
                text(marker_text)
                    .size(11.0)
                    .font(theme::emphasis::medium())
                    .style(move |t: &Theme| {
                        let p = pal_of(t);
                        let color = match state {
                            WizardStepState::Completed => p.on_primary,
                            WizardStepState::Active if is_error => p.error,
                            WizardStepState::Active => p.primary,
                            WizardStepState::Upcoming => p.on_surface_variant,
                        };
                        iced::widget::text::Style { color: Some(color) }
                    }),
            )
            .width(Length::Fixed(20.0))
            .height(Length::Fixed(20.0))
            .align_x(iced::alignment::Horizontal::Center)
            .align_y(iced::alignment::Vertical::Center)
            .style(move |t: &Theme| {
                let p = pal_of(t);
                let (background, border_color) = match state {
                    WizardStepState::Completed => (Some(p.primary.into()), p.primary),
                    WizardStepState::Active if is_error => (None, p.error),
                    WizardStepState::Active => (None, p.primary),
                    WizardStepState::Upcoming => (None, p.outline_variant),
                };
                container::Style {
                    background,
                    border: iced::Border {
                        color: border_color,
                        width: 1.5,
                        radius: theme::shape::FULL.into(),
                    },
                    ..Default::default()
                }
            });
            // 12, not 14: these are phase sentences ("펌웨어 입력 파일 검증"),
            // not the mockup's one-word partition names, and at 14 every row
            // wrapped to two lines.
            let mut phase = text(step.label.clone())
                .size(theme::text_size::BODY_SMALL)
                .width(Length::Fill)
                .wrapping(iced::widget::text::Wrapping::WordOrGlyph)
                .style(move |t: &Theme| {
                    let p = pal_of(t);
                    let color = match state {
                        WizardStepState::Completed | WizardStepState::Active => p.on_surface,
                        WizardStepState::Upcoming => p.on_surface_variant,
                    };
                    iced::widget::text::Style { color: Some(color) }
                });
            if state == WizardStepState::Active {
                phase = phase.font(theme::emphasis::medium());
            }
            // No trailing status word. The marker already says which state a
            // row is in — filled check done, ringed number running, flat
            // number waiting — and repeating it in text was taking the width
            // the label needs.
            checklist_rows = checklist_rows.push(
                row![marker, phase]
                    .spacing(12)
                    .height(Length::Fixed(38.0))
                    .align_y(iced::Alignment::Center),
            );
        }
        // M3's canonical supporting-pane layout fixes the secondary pane at
        // 360dp with a 24dp gutter on expanded windows. It also says compact
        // should show one pane rather than split — kept split here by request,
        // just narrower so the log still has room.
        let checklist_width = match self.window_size_class() {
            WindowSizeClass::Expanded => Length::Fixed(340.0),
            WindowSizeClass::Compact => Length::Fixed(260.0),
        };
        let checklist = container(checklist_rows)
            .padding([8, 16])
            .width(checklist_width)
            .height(Length::Fill)
            .style(|t: &Theme| {
                theme::surface_card_style(t, theme::SurfaceLevel::Default, theme::shape::MD)
            });

        let save_action: Element<'_, Message> = button(
            text(self.t("btn_save").to_string())
                .size(theme::text_size::BODY_SMALL)
                .font(theme::emphasis::medium()),
        )
        .on_press(Message::SaveLog)
        .padding([6.0, 10.0])
        .style(md_text_btn_style)
        .into();
        let editor = iced::widget::text_editor(&self.log_editor)
            .on_action(Message::LogEditorAction)
            .size(11.0)
            .height(Length::Fill)
            .padding(iced::Padding {
                top: 0.0,
                right: 16.0,
                bottom: 10.0,
                left: 16.0,
            })
            .style(m3_log_text_editor_style);
        let log_card = m3_log_text_field_with_action(
            self.t("dash_log").to_string(),
            Some(save_action),
            editor.into(),
        );
        let details: Element<'_, Message> = match self.window_size_class() {
            WindowSizeClass::Expanded => row![checklist, log_card]
                .spacing(24.0)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_y(iced::Alignment::Start)
                .into(),
            // M3's supporting-pane layout shows one pane at compact rather
            // than splitting. Position moves into the card heading below,
            // which is why that counter is compact-only.
            WindowSizeClass::Compact => log_card,
        };

        let has_output = !is_busy
            && self.current_view == View::Advanced
            && self.adv_wizard.output_dir.is_some()
            && self
                .adv_wizard
                .action
                .map(|action| action.produces_output())
                .unwrap_or(false);
        let action_layout = exec_action_layout(is_busy, is_error, has_output);
        let mut actions = row![]
            .spacing(ACTION_BUTTON_SPACING)
            .align_y(iced::Alignment::Center)
            .height(Length::Fill);
        if action_layout.start_over_utility {
            actions = actions.push(wizard_secondary_action(
                icon::fab_start_over(),
                self.t("btn_start_over").to_string(),
                Some(Message::StartOver),
            ));
        }
        if let Some(primary) = action_layout.primary {
            actions = match primary {
                ExecPrimaryAction::StartOver => actions.push(wizard_primary_action(
                    icon::fab_start_over(),
                    self.t("btn_start_over").to_string(),
                    Some(Message::StartOver),
                    None,
                    false,
                )),
                ExecPrimaryAction::OpenFolder => actions.push(wizard_primary_action(
                    icon::fab_open_folder(),
                    self.t("btn_open_folder").to_string(),
                    Some(Message::Adv(AdvMsg::AdvWizOpenOutputFolder)),
                    None,
                    false,
                )),
            };
        }

        let content = column![step_card, details]
            .spacing(16.0)
            .padding(16.0)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(iced::Alignment::Start);
        let body = container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(iced::alignment::Horizontal::Center)
            .align_y(iced::alignment::Vertical::Top);

        // While the operation runs there is nothing to offer — Save moved into
        // the log card's header — so the footer would render as an empty band
        // across the bottom of the screen.
        if !action_layout.has_any() {
            return body.into();
        }
        column![
            body,
            wizard_action_footer(row![].height(Length::Fill), actions),
        ]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::{cumulative_flash_percent, current_partition_size};

    #[test]
    fn overall_flash_percent_uses_operation_bytes_not_current_partition() {
        let snapshot = ltbox_device::edl::FlashProgress {
            partition: "vendor_boot".into(),
            percent: 5,
            completed_bytes: 5,
            total_bytes: 100,
            operation_completed_bytes: 700,
            operation_total_bytes: 1_000,
        };
        assert_eq!(cumulative_flash_percent(&snapshot), 70);
        assert_eq!(
            current_partition_size(&snapshot),
            Some(crate::format_bytes_auto(100))
        );
        assert_eq!(current_partition_size(&Default::default()), None);
    }
}
