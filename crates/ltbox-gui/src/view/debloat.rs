//! Debloat wizard view: action, app checklist, confirm, execution.

use crate::debloat::{DebloatList, DebloatMethod, PackageState};
use crate::focus_button::{self, button};
use crate::*;
use iced::widget::{self, column, container, row, scrollable, text};
use iced::{Element, Length, Theme};
use ltbox_core::tr_args;

impl App {
    pub(crate) fn view_debloat_wizard(&self) -> Element<'_, Message> {
        if self.log_popup_open && self.debloat.is_in_exec() {
            return self.log_popup_view();
        }
        let step_labels: Vec<&str> = DEBLOAT_STEPS.iter().map(|k| self.t(k)).collect();
        let is_exec = self.debloat.is_in_exec();
        let step_bar = if is_exec {
            empty_wizard_step_bar()
        } else {
            wizard_step_bar(&step_labels, self.debloat.step, self.window_size_class())
        };
        let list = self.debloat_list();
        let body = match (self.debloat.step, list) {
            (0, _) | (_, None) => self.debloat_action_step(),
            (1, Some(list)) => wizard_step_body(
                self.t("debloat_apps_title").to_string(),
                self.debloat_apps_step(list),
            ),
            (2, Some(list)) => wizard_step_body(
                self.t("sysupdate_confirm_title").to_string(),
                self.debloat_confirm_step(list),
            ),
            _ => self.exec_step_view(),
        };
        let reading = self.debloat.step == 0 && self.operation.is_running();
        let app_bar_subtitle = if reading {
            Some(self.t("live_debloat_reading_states").to_string())
        } else {
            is_exec.then(|| self.exec_app_bar_subtitle()).flatten()
        };
        let last_nav_step = DEBLOAT_STEPS.len() - 2;
        let nav = if self.debloat.step <= last_nav_step {
            let is_start = self.debloat.step == last_nav_step;
            let label = if is_start {
                self.t("btn_start")
            } else {
                self.t("btn_next")
            };
            let can = list.is_some()
                && !reading
                && self.debloat.can_next()
                && !(self.operation.is_running() && is_start)
                && (!is_start || self.device_reachable());
            wizard_nav_generic(
                self.debloat.step > 0,
                label,
                can,
                self.t("btn_back"),
                Message::Debloat(DebloatMsg::Back),
                Message::Debloat(DebloatMsg::Next),
            )
        } else {
            empty_wizard_nav()
        };
        column![
            wizard_action_bar(
                self.window_size_class(),
                self.t("nav_debloat").to_string(),
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

    fn debloat_action_step(&self) -> Element<'_, Message> {
        let content_width = self.window_size.0
            - match self.window_size_class() {
                WindowSizeClass::Compact => SIDEBAR_RAIL_WIDTH,
                WindowSizeClass::Expanded => SIDEBAR_EXPANDED_WIDTH,
            };
        let icon_size = self.wizard_list_icon(WIZARD_LIST_GLYPH_ICON_SIZE);
        let metrics = self.wizard_list_metrics(WIZARD_LIST_LABEL_SIZE, WIZARD_LIST_DESC_SIZE);
        let available = self.debloat_list().is_some();
        // Without a list for this model both actions stay visible but inert,
        // and their description says why.
        let unavailable = if self.device.model.is_empty() {
            self.t("debloat_no_device").to_string()
        } else {
            tr_args!("debloat_no_list", model = self.device.model.as_str())
        };
        let mut cards = column![].spacing(8.0).width(Length::Fill);
        for (action, glyph) in [
            (DebloatAction::Remove, icon::tile_debloat_remove()),
            (DebloatAction::Restore, icon::tile_debloat_restore()),
        ] {
            let icon = if available {
                lucide_list_primary(glyph, icon_size)
            } else {
                lucide_list_disabled(glyph, icon_size)
            };
            let sub = if available {
                self.t(action.desc_key()).to_string()
            } else {
                unavailable.clone()
            };
            cards = cards.push(wizard_list_option_card(
                icon,
                self.t(action.label_key()),
                &sub,
                self.debloat.action == Some(action),
                available.then_some(Message::Debloat(DebloatMsg::Action(action))),
                metrics,
            ));
        }
        wizard_selection_step(
            content_width,
            self.t("sysupdate_action_title").to_string(),
            cards.into(),
        )
    }

    fn debloat_apps_step(&self, list: &'static DebloatList) -> Element<'_, Message> {
        let preset_button = |key: &str, preset: DebloatPreset| -> Element<'_, Message> {
            button(
                text(self.t(key).to_string())
                    .size(theme::text_size::BODY_SMALL)
                    .font(theme::emphasis::medium()),
            )
            .on_press(Message::Debloat(DebloatMsg::Preset(preset)))
            .padding([6.0, 10.0])
            .style(md_text_btn_style)
            .into()
        };
        let toolbar = row![
            text(tr_args!(
                "debloat_selected_count",
                count = self.debloat.selected.len(),
                total = self.debloat.actionable_count(list)
            ))
            .size(theme::text_size::BODY_SMALL)
            .style(muted_style)
            .width(Length::Fill),
            preset_button("debloat_select_recommended", DebloatPreset::Recommended),
            preset_button("debloat_select_all", DebloatPreset::All),
            preset_button("debloat_select_none", DebloatPreset::None),
        ]
        .spacing(4.0)
        .align_y(iced::Alignment::Center);

        // Apps the action would change come first; the rest stay listed,
        // inert, with the state that rules them out.
        let (actionable, settled): (Vec<_>, Vec<_>) = list
            .packages
            .iter()
            .partition(|package| self.debloat.is_actionable(package));
        let mut rows = column![].spacing(0).width(Length::Fill);
        for package in actionable.iter().chain(&settled) {
            let enabled = self.debloat.is_actionable(package);
            let selected = self.debloat.selected.contains(&package.id);
            let checkbox: Element<'_, Message> = if enabled {
                let id = package.id.clone();
                focus_button::actionable(
                    widget::checkbox(selected)
                        .style(m3_checkbox_style)
                        .on_toggle(move |_| Message::Debloat(DebloatMsg::Toggle(id.clone()))),
                    Some(Message::Debloat(DebloatMsg::Toggle(package.id.clone()))),
                )
            } else {
                widget::checkbox(false).style(m3_checkbox_style).into()
            };
            let method = match package.method {
                DebloatMethod::Uninstall => self.t("debloat_method_uninstall"),
                DebloatMethod::Disable => self.t("debloat_method_disable"),
            };
            let mut detail = vec![package.id.clone(), method.to_string()];
            if let Some(state) = self.debloat.state_of(package) {
                detail.push(
                    self.t(match state {
                        PackageState::Installed => "debloat_state_installed",
                        PackageState::Disabled => "debloat_state_disabled",
                        PackageState::Removed => "debloat_state_removed",
                        PackageState::Absent => "debloat_state_absent",
                    })
                    .to_string(),
                );
            }
            if !package.recommended {
                detail.push(self.t("debloat_optional").to_string());
            }
            let label = text(package.label.clone())
                .size(theme::text_size::BODY_MEDIUM)
                .font(theme::emphasis::medium())
                .wrapping(widget::text::Wrapping::WordOrGlyph);
            let entry = row![
                container(checkbox).width(Length::Fixed(32.0)),
                column![
                    if enabled {
                        label
                    } else {
                        label.style(muted_style)
                    },
                    text(detail.join(" · "))
                        .size(theme::text_size::BODY_SMALL)
                        .style(muted_style)
                        .wrapping(widget::text::Wrapping::WordOrGlyph),
                ]
                .spacing(2.0)
                .width(Length::Fill),
            ]
            .spacing(8.0)
            .padding([6.0, 10.0])
            .align_y(iced::Alignment::Center);
            // Tint selected rows so the change set is visible at a glance.
            rows = rows.push(container(entry).width(Length::Fill).style(
                move |t: &Theme| -> container::Style {
                    let p = pal_of(t);
                    container::Style {
                        background: selected
                            .then_some(iced::Background::Color(p.secondary_container)),
                        text_color: selected.then_some(p.on_secondary_container),
                        ..Default::default()
                    }
                },
            ));
        }

        let notice = if let Some(error) = self.debloat.scan_error.as_deref() {
            Some(tr_args!("debloat_states_unknown", error = error))
        } else if actionable.is_empty() {
            Some(
                self.t(match self.debloat.action {
                    Some(DebloatAction::Restore) => "debloat_nothing_to_restore",
                    _ => "debloat_nothing_to_remove",
                })
                .to_string(),
            )
        } else {
            None
        };
        let mut content = column![]
            .spacing(8.0)
            .width(Length::Fill)
            .height(Length::Fill);
        if let Some(notice) = notice {
            content = content.push(
                text(notice)
                    .size(theme::text_size::BODY_SMALL)
                    .style(on_surface_style)
                    .wrapping(widget::text::Wrapping::WordOrGlyph),
            );
        }
        content
            .push(toolbar)
            .push(widget::rule::horizontal(1).style(shell_rule_style))
            .push(
                scrollable(rows)
                    .style(m3_scrollable_style)
                    .height(Length::Fill)
                    .width(Length::Fill),
            )
            .into()
    }
    fn debloat_confirm_step(&self, list: &'static DebloatList) -> Element<'_, Message> {
        let action = self
            .debloat
            .action
            .map(|a| self.t(a.label_key()).to_string())
            .unwrap_or_else(|| "—".to_string());
        let targets = self.debloat.selected_targets(list);
        let apps = targets
            .iter()
            .map(|target| target.package.label.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let grid_rows = vec![
            confirm_definition_row(self.t("sysupdate_step_action"), &action),
            confirm_definition_row(
                self.t("debloat_step_apps"),
                &tr_args!(
                    "debloat_selected_count",
                    count = targets.len(),
                    total = self.debloat.actionable_count(list)
                ),
            ),
        ];
        let trailing = vec![
            text(apps)
                .size(theme::text_size::BODY_SMALL)
                .style(muted_style)
                .wrapping(widget::text::Wrapping::WordOrGlyph)
                .into(),
        ];
        self.confirm_step_frame(vec![], grid_rows, trailing)
    }
}
