//! Advanced menu + generic adv wizard views + steps + image-info exec. Extracted from `main.rs`.

use crate::focus_button::button;
use crate::*;
use iced::widget::{Space, column, container, row, scrollable, text};
use iced::{Element, Length, Theme};

impl App {
    pub(crate) fn view_advanced(&self) -> Element<'_, Message> {
        // Dedicated wizards preempt the grid.
        if self.advanced_wizard_open.is_flash_parts() {
            return self.view_flash_parts_wizard();
        }
        if self.advanced_wizard_open.is_dump_parts() {
            return self.view_dump_parts_wizard();
        }
        if self.advanced_wizard_open.is_dump_phys() {
            return self.view_dump_phys_wizard();
        }
        if self.advanced_wizard_open.is_flash_phys() {
            return self.view_flash_phys_wizard();
        }
        if self.advanced_wizard_open.is_simple_flash() {
            return self.view_simple_flash_wizard();
        }
        if self.adv_wizard.action.is_some() {
            return self.view_adv_wizard();
        }

        let nav_width = match self.window_size_class() {
            WindowSizeClass::Compact => SIDEBAR_RAIL_WIDTH,
            WindowSizeClass::Expanded => SIDEBAR_EXPANDED_WIDTH,
        };
        let available_width = self.window_size.0 - nav_width - 48.0;
        let columns = ((available_width / 240.0).floor() as usize).max(1);
        let mut content = column![].spacing(14.0).width(Length::Fill);

        for section in ADV_SECTIONS {
            content = content.push(
                text(self.t(section.title_key).to_string())
                    .size(11.0)
                    .style(muted_style),
            );
            let mut rows = column![].spacing(8.0);
            for chunk in section.items.chunks(columns) {
                let mut r = row![].spacing(8.0).width(Length::Fill);
                for &item in chunk {
                    r = r.push(adv_grid_btn(item, self.t(item.label_key())));
                }
                for _ in chunk.len()..columns {
                    r = r.push(Space::new().width(Length::Fill));
                }
                rows = rows.push(r);
            }
            content = content.push(rows);
        }

        let body = scrollable(container(content).padding(24.0).width(Length::Fill))
            .style(m3_scrollable_style)
            .width(Length::Fill)
            .height(Length::Fill);

        column![
            large_top_app_bar(self.t("nav_advanced").to_string(), None),
            body,
        ]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    /// Advanced wizard. PatchDevinfo: source/country/confirm/exec.
    /// Others: source/confirm/exec.
    pub(crate) fn view_adv_wizard(&self) -> Element<'_, Message> {
        let is_exec = self.adv_wizard.step == self.adv_wizard.exec_step();
        let shared_exec = is_exec && !self.adv_wizard.is_image_info();
        if self.log_popup_open && is_exec && !self.adv_wizard.is_image_info() {
            return self.log_popup_view();
        }

        let step_labels: Vec<&str> = self.adv_wizard.steps().iter().map(|k| self.t(k)).collect();
        let step_bar = if shared_exec {
            empty_wizard_step_bar()
        } else {
            wizard_step_bar(&step_labels, self.adv_wizard.step, self.window_size_class())
        };

        let needs_country = self.adv_wizard.needs_country();
        let needs_region_target = self.adv_wizard.needs_region_target();
        let is_confirm = self.adv_wizard.is_confirm_step();

        let detect_arb_step0 = matches!(self.adv_wizard.action, Some(AdvAction::DetectArb))
            && self.adv_wizard.step == 0;
        let body: Element<'_, Message> = if is_exec && self.adv_wizard.is_image_info() {
            self.adv_image_info_exec_step()
        } else if is_exec {
            self.exec_step_view()
        } else if detect_arb_step0 {
            self.adv_wiz_detect_arb_step()
        } else if is_confirm {
            self.adv_wiz_confirm_step()
        } else if needs_country && self.adv_wizard.step == 0 {
            self.adv_wiz_country_step()
        } else if needs_country && self.adv_wizard.step == 1 {
            self.adv_wiz_loader_step()
        } else if needs_region_target && self.adv_wizard.step == 1 {
            self.adv_wiz_region_target_step()
        } else if matches!(self.adv_wizard.action, Some(AdvAction::PatchArb))
            && self.adv_wizard.step == 1
        {
            self.adv_wiz_arb_inspect_step()
        } else {
            self.adv_wiz_source_step()
        };
        let flow_title = self
            .adv_wizard
            .action
            .map(|action| self.t(action.label_key()).to_string())
            .unwrap_or_else(|| self.t("nav_advanced").to_string());
        let (step_title, step_subtitle) = self
            .adv_step_copy()
            .unwrap_or_else(|| (flow_title.clone(), None));
        let body = if shared_exec {
            body
        } else {
            if !is_confirm
                && ((!needs_country && self.adv_wizard.step == 0)
                    || (needs_country && self.adv_wizard.step == 1))
            {
                self.wizard_picker_step(step_title, body)
            } else {
                wizard_step_body(step_title, body)
            }
        };
        let app_bar_subtitle = if is_exec {
            self.exec_app_bar_subtitle()
        } else {
            step_subtitle
        };

        let nav: Element<'_, Message> = if is_exec {
            empty_wizard_nav()
        } else {
            let label = if is_confirm || detect_arb_step0 {
                self.t("btn_start").to_string()
            } else {
                self.t("btn_next").to_string()
            };
            let is_start = is_confirm || detect_arb_step0;
            // DetectArb's Start gets a generic reachability gate. Change Country
            // Code (PatchDevinfo) runs over EDL but is launched from system/ADB
            // (the model is read there), so its Start specifically requires an
            // ADB connection. Other advanced ops are folder-only (no device).
            let needs_device = matches!(self.adv_wizard.action, Some(AdvAction::DetectArb));
            let needs_system =
                is_start && matches!(self.adv_wizard.action, Some(AdvAction::PatchDevinfo));
            let system_ok = self.device.connection == ConnectionStatus::Adb;
            // Change Country's picked loader must still fit the connected model at
            // Start (the device may have been swapped after the loader was picked).
            let loader_ok = !needs_system
                || self
                    .adv_wizard
                    .file_path
                    .as_deref()
                    .map(|p| self.loader_fits_model(std::path::Path::new(p)))
                    .unwrap_or(true);
            let can = (if detect_arb_step0 {
                if self.is_tb320fc() {
                    self.adv_wizard.file_path.is_some()
                } else {
                    true
                }
            } else {
                self.adv_wizard.can_next()
            }) && !self.operation.is_running()
                && (!is_start || !needs_device || self.device_reachable())
                && (!needs_system || system_ok)
                && loader_ok;
            // Explain a Start disabled by the ADB / loader-fit gate on hover.
            let hint: Option<String> = if needs_system && !system_ok {
                Some(self.t("adv_country_needs_system").to_string())
            } else if needs_system && !loader_ok {
                Some(self.t("loader_model_mismatch_tooltip").to_string())
            } else {
                None
            };
            wizard_nav_generic_with_disabled_next_tooltip(
                true,
                &label,
                can,
                hint,
                self.t("btn_back"),
                Message::Adv(AdvMsg::AdvWizBack),
                Message::Adv(AdvMsg::AdvWizNext),
            )
        };

        column![
            wizard_action_bar(self.window_size_class(), flow_title, app_bar_subtitle),
            step_bar,
            body,
            nav,
        ]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    fn adv_step_copy(&self) -> Option<(String, Option<String>)> {
        let action = self.adv_wizard.action?;
        let is_exec = self.adv_wizard.step == self.adv_wizard.exec_step();
        if is_exec {
            let (title, subtitle) = self.exec_status_copy();
            return Some((title, Some(subtitle)));
        }

        let (title, subtitle) = if self.adv_wizard.is_confirm_step() {
            (
                self.t(action.label_key()).to_string(),
                self.t(action.desc_key()).to_string(),
            )
        } else if self.adv_wizard.needs_country() && self.adv_wizard.step == 0 {
            (
                self.t("adv_country_title").to_string(),
                self.t("adv_country_subtitle").to_string(),
            )
        } else if self.adv_wizard.needs_country() && self.adv_wizard.step == 1 {
            (
                self.t("edl_loader_title").to_string(),
                self.loader_picker_subtitle(),
            )
        } else if self.adv_wizard.needs_region_target() && self.adv_wizard.step == 1 {
            (
                self.t("adv_region_target_title").to_string(),
                self.t("adv_region_target_subtitle").to_string(),
            )
        } else if matches!(action, AdvAction::PatchArb) && self.adv_wizard.step == 1 {
            (
                self.t("adv_arb_inspect_title").to_string(),
                self.t("adv_arb_inspect_subtitle").to_string(),
            )
        } else {
            let subtitle = if matches!(action, AdvAction::DetectArb | AdvAction::PatchDevinfo) {
                self.loader_picker_subtitle()
            } else {
                self.t(action.source_desc_key()).to_string()
            };
            (self.t(action.label_key()).to_string(), subtitle)
        };

        Some((title, Some(subtitle)))
    }

    /// Step 0 — shared file/folder source picker.
    pub(crate) fn adv_wiz_source_step(&self) -> Element<'_, Message> {
        let Some(_) = self.adv_wizard.action else {
            return container(text("")).into();
        };
        let status = if self.adv_wizard.is_image_info() && !self.adv_wizard.file_paths.is_empty() {
            Some(tr_args!(
                "adv_image_info_selected_count",
                count = self.adv_wizard.file_paths.len().to_string()
            ))
        } else {
            self.adv_wizard.file_path.clone()
        };
        let recents = if self.adv_wizard.is_image_info() {
            self.recent_file_chips(
                &["img"],
                |p| Message::Adv(AdvMsg::AdvWizBrowseManyDone(Some(vec![p]))),
                "picker_recents",
            )
        } else if self.adv_wizard.is_folder_op() {
            self.recent_chips(
                self.recent_paths
                    .recent(self.adv_wizard.picker_kind().storage_key()),
                |p| Message::Adv(AdvMsg::AdvWizBrowseDone(Some(p))),
                "picker_recents",
                false,
            )
        } else {
            self.recent_file_chips(
                self.advanced_picker_exts().1,
                |p| Message::Adv(AdvMsg::AdvWizBrowseDone(Some(p))),
                "picker_recents",
            )
        };
        scrollable(
            column![
                self.wizard_picker_row(
                    status.as_deref(),
                    if self.adv_wizard.is_folder_op() {
                        PickerPathKind::Folder
                    } else {
                        PickerPathKind::File
                    },
                    Some(Message::Adv(AdvMsg::AdvWizBrowse)),
                    None
                ),
                recents,
            ]
            .spacing(6)
            .padding(28)
            .width(Length::Fill),
        )
        .height(Length::Fill)
        .into()
    }

    /// Step 1 (PatchDevinfo only) — country picker tile; opens the
    /// shared country popup.
    pub(crate) fn adv_wiz_country_step(&self) -> Element<'_, Message> {
        let selected = self.adv_wizard.country.is_some();
        let status = self
            .adv_wizard
            .country
            .clone()
            .unwrap_or_else(|| self.t("adv_country_placeholder").to_string());
        let btn = button(
            container(
                column![
                    text(self.t("btn_pick_country").to_string())
                        .size(14.0)
                        .center(),
                    text(tr_args!(
                        "adv_country_pick_count",
                        count = COUNTRY_CODES.len().to_string()
                    ))
                    .size(11.0)
                    .style(muted_style)
                    .center(),
                ]
                .spacing(6.0)
                .width(Length::Fixed(280.0))
                .align_x(iced::Alignment::Center),
            )
            .padding([20.0, 24.0])
            .width(Length::Fixed(280.0))
            .style(move |t: &Theme| sel_card_style(t, selected)),
        )
        .width(Length::Shrink)
        .on_press(Message::Adv(AdvMsg::AdvWizOpenCountry))
        .padding(0)
        .style(move |t: &Theme, status| sel_card_btn_style(t, status, selected));
        let btn_row = row![
            Space::new().width(Length::Fill),
            btn,
            Space::new().width(Length::Fill),
        ];
        let status_style = move |t: &Theme| {
            let p = pal_of(t);
            iced::widget::text::Style {
                color: Some(if selected { p.success } else { p.outline }),
            }
        };
        let col = column![
            btn_row,
            text(status)
                .size(12.0)
                .width(Length::Fill)
                .style(status_style)
                .center()
                .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
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

    /// Step 1 for Change Country Code: pick the EDL loader (the device is
    /// transitioned to EDL with it). Same loader-browse the DetectArb step uses.
    pub(crate) fn adv_wiz_loader_step(&self) -> Element<'_, Message> {
        let error = (self.default_loader_path.is_some() && !self.default_loader_fits_model())
            .then(|| self.t("loader_default_ext_unsupported").to_string());
        let mut content = column![self.wizard_picker_row(
            self.adv_wizard.file_path.as_deref(),
            PickerPathKind::File,
            Some(Message::Adv(AdvMsg::AdvWizBrowse)),
            None
        ),]
        .spacing(6)
        .width(Length::Fill);
        if let Some(error) = error {
            content =
                content.push(
                    text(error)
                        .size(12)
                        .style(|t: &Theme| iced::widget::text::Style {
                            color: Some(pal_of(t).error),
                        }),
                );
        }
        content = content.push(self.recent_file_chips(
            self.loader_picker_exts(),
            |p| Message::Adv(AdvMsg::AdvWizBrowseDone(Some(p))),
            "picker_recents",
        ));
        scrollable(content.padding(28)).height(Length::Fill).into()
    }

    /// Step 1 for `RegionConvert`: card that opens the target picker
    /// popup. Mirrors `adv_wiz_country_step` shape so the wizard
    /// rendering stays consistent with the other "needs option"
    /// flow (PatchDevinfo).
    pub(crate) fn adv_wiz_region_target_step(&self) -> Element<'_, Message> {
        let selected = self.adv_wizard.region_target.is_some();
        let status = match self.adv_wizard.region_target {
            Some(target) => self.t(target.label_key()).to_string(),
            None => self.t("adv_region_target_placeholder").to_string(),
        };
        let btn = button(
            container(
                column![
                    text(self.t("btn_pick_region_target").to_string())
                        .size(14.0)
                        .center(),
                    text(self.t("adv_region_target_desc").to_string())
                        .size(11.0)
                        .style(muted_style)
                        .center(),
                ]
                .spacing(6.0)
                .width(Length::Fixed(280.0))
                .align_x(iced::Alignment::Center),
            )
            .padding([20.0, 24.0])
            .width(Length::Fixed(280.0))
            .style(move |t: &Theme| sel_card_style(t, selected)),
        )
        .width(Length::Shrink)
        .on_press(Message::Adv(AdvMsg::AdvWizOpenRegionTarget))
        .padding(0)
        .style(move |t: &Theme, status| sel_card_btn_style(t, status, selected));
        let btn_row = row![
            Space::new().width(Length::Fill),
            btn,
            Space::new().width(Length::Fill),
        ];
        let status_style = move |t: &Theme| {
            let p = pal_of(t);
            iced::widget::text::Style {
                color: Some(if selected { p.success } else { p.outline }),
            }
        };
        let col = column![
            btn_row,
            text(status)
                .size(12.0)
                .width(Length::Fill)
                .style(status_style)
                .center()
                .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
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

    /// PatchArb inspect step — render boot.img + vbmeta_system.img
    /// rollback indices (decimal + UTC) read from the picked folder so
    /// the user can sanity-check the source before opening the
    /// timestamp popup. Next on this step opens the popup.
    pub(crate) fn adv_wiz_arb_inspect_step(&self) -> Element<'_, Message> {
        let (boot_idx, vbmeta_idx) = self.adv_wizard.arb_inspect.unwrap_or((0, 0));
        let mk_row = |label_key: &'static str, idx: u64| -> Element<'_, Message> {
            let utc = format_unix_timestamp_utc(idx);
            iced::widget::row![
                text(self.t(label_key).to_string())
                    .size(theme::text_size::BODY_MEDIUM)
                    .style(muted_style)
                    .width(Length::Fixed(220.0)),
                text(idx.to_string())
                    .size(theme::text_size::BODY_MEDIUM)
                    .width(Length::Fixed(140.0)),
                text(utc).size(12.0).style(muted_style),
            ]
            .spacing(12.0)
            .align_y(iced::Alignment::Center)
            .into()
        };
        let col = column![
            mk_row("adv_arb_inspect_boot", boot_idx),
            mk_row("adv_arb_inspect_vbmeta", vbmeta_idx),
        ]
        .spacing(8.0)
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

    /// DetectArb step 0. The TB320FC hardware path needs an EDL loader (the deeper
    /// path falls back to dumping `boot_a` + `vbmeta_system_a` when
    /// stored_rollback_index is missing, so a Firehose loader is
    /// required); other models just see a Start prompt because the
    /// detection runs entirely over fastboot vars.
    pub(crate) fn adv_wiz_detect_arb_step(&self) -> Element<'_, Message> {
        if self.is_tb320fc() {
            self.adv_wiz_loader_step()
        } else {
            container(column![])
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        }
    }

    /// Confirm step — Next becomes Start.
    pub(crate) fn adv_wiz_confirm_step(&self) -> Element<'_, Message> {
        if self.adv_wizard.action.is_none() {
            return container(text("")).into();
        }
        let dash = "—".to_string();
        let path = self
            .adv_wizard
            .file_path
            .clone()
            .unwrap_or_else(|| dash.clone());
        // Change Country Code shows the EDL loader (its `file_path`); other ops
        // show the picked source file/folder.
        let source_label = if self.adv_wizard.needs_country() {
            self.t("edl_loader_label")
        } else {
            self.t("adv_confirm_source")
        };
        let mut grid_rows = Vec::new();
        if self.adv_wizard.needs_country() {
            let code = self.adv_wizard.country.clone().unwrap_or(dash.clone());
            grid_rows.push(confirm_definition_row(self.t("adv_confirm_country"), &code));
        }
        if self.adv_wizard.needs_region_target() {
            let label = self
                .adv_wizard
                .region_target
                .map(|r| self.t(r.label_key()).to_string())
                .unwrap_or(dash);
            grid_rows.push(confirm_definition_row(
                self.t("adv_confirm_region_target"),
                &label,
            ));
        }
        if matches!(self.adv_wizard.action, Some(AdvAction::PatchArb))
            && let Some(idx) = self.adv_wizard.arb_index_committed
        {
            let utc = format_unix_timestamp_utc(idx);
            grid_rows.push(confirm_definition_row(
                self.t("adv_confirm_arb_index"),
                &format!("{idx}  ({utc})"),
            ));
            if let Some((boot_idx, vbmeta_idx)) = self.adv_wizard.arb_inspect {
                grid_rows.push(confirm_definition_row(
                    self.t("adv_arb_inspect_boot"),
                    &format!("{boot_idx} → {idx}"),
                ));
                grid_rows.push(confirm_definition_row(
                    self.t("adv_arb_inspect_vbmeta"),
                    &format!("{vbmeta_idx} → {idx}"),
                ));
            }
        }
        self.confirm_step_frame(
            vec![],
            grid_rows,
            vec![confirm_path_row(source_label, &path)],
        )
    }

    pub(crate) fn adv_image_info_exec_step(&self) -> Element<'_, Message> {
        let editor = iced::widget::text_editor(&self.image_info_log_editor)
            .on_action(Message::ImageInfoLogEditorAction)
            .size(11.0)
            .height(Length::Fill)
            .padding(iced::Padding {
                top: 0.0,
                right: 16.0,
                bottom: 10.0,
                left: 16.0,
            })
            .style(m3_log_text_editor_style);

        let utility_actions = row![wizard_secondary_action(
            icon::fab_save_log(),
            self.t("btn_save_log").to_string(),
            Some(Message::SaveLog),
        )]
        .spacing(ACTION_BUTTON_SPACING)
        .align_y(iced::Alignment::Center);
        let mut actions = utility_actions
            .spacing(ACTION_BUTTON_SPACING)
            .align_y(iced::Alignment::Center)
            .height(Length::Fill);

        if !self.operation.is_running() {
            actions = actions.push(wizard_primary_action(
                icon::fab_start_over(),
                self.t("btn_start_over").to_string(),
                Some(Message::StartOver),
                None,
                false,
            ));
        }

        let body = column![m3_log_text_field(
            self.t("adv_image_info").to_string(),
            editor.into()
        )]
        .spacing(12.0)
        .padding(20.0)
        .width(Length::Fill)
        .height(Length::Fill);

        column![
            container(body).width(Length::Fill).height(Length::Fill),
            wizard_action_footer(row![].height(Length::Fill), actions),
        ]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    /// Simple Firmware Flash wizard: source folder → confirm → exec.
    pub(crate) fn view_simple_flash_wizard(&self) -> Element<'_, Message> {
        if self.log_popup_open && self.simple_flash.step >= 2 {
            return self.log_popup_view();
        }
        let step_labels: Vec<&str> = SIMPLE_FLASH_STEPS.iter().map(|k| self.t(k)).collect();
        let is_exec = self.simple_flash.step >= 2;
        let step_bar = if is_exec {
            empty_wizard_step_bar()
        } else {
            wizard_step_bar(
                &step_labels,
                self.simple_flash.step,
                self.window_size_class(),
            )
        };
        let body: Element<'_, Message> = match self.simple_flash.step {
            0 => self.simple_flash_intro_step(),
            1 => self.simple_flash_confirm_step(),
            _ => self.exec_step_view(),
        };
        let (step_title, app_bar_subtitle) = self.simple_flash_step_copy();
        let body = if is_exec {
            body
        } else {
            wizard_step_body(step_title, body)
        };
        let nav = if self.simple_flash.step < 2 {
            let is_start = self.simple_flash.step == 1;
            let label = if is_start {
                self.t("btn_start").to_string()
            } else {
                self.t("btn_next").to_string()
            };
            let can = self.simple_flash.can_next()
                && !(self.operation.is_running() && is_start)
                && (!is_start || self.device_reachable());
            wizard_nav_generic(
                true,
                &label,
                can,
                self.t("btn_back"),
                if self.simple_flash.step == 0 {
                    // Back on the intro step returns to the Advanced grid.
                    Message::SimpleFlash(SimpleFlashMsg::SimpleFlashClose)
                } else {
                    Message::SimpleFlash(SimpleFlashMsg::SimpleFlashBack)
                },
                Message::SimpleFlash(SimpleFlashMsg::SimpleFlashNext),
            )
        } else {
            empty_wizard_nav()
        };
        column![
            wizard_action_bar(
                self.window_size_class(),
                self.t("adv_simple_flash").to_string(),
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

    fn simple_flash_step_copy(&self) -> (String, Option<String>) {
        let title_key = match self.simple_flash.step {
            0 => "adv_simple_flash",
            1 => "flash_confirm_title",
            _ => {
                let (title, _) = self.exec_status_copy();
                return (title, self.exec_app_bar_subtitle());
            }
        };
        (
            self.t(title_key).to_string(),
            (self.simple_flash.step == 0).then(|| self.t("flash_folder_desc").to_string()),
        )
    }

    /// Source step — top-aligned firmware-folder picker, matching the other
    /// wizard source pickers.
    fn simple_flash_intro_step(&self) -> Element<'_, Message> {
        scrollable(
            column![
                self.wizard_picker_row(
                    self.simple_flash.firmware_folder.as_deref(),
                    PickerPathKind::Folder,
                    Some(Message::SimpleFlash(
                        SimpleFlashMsg::SimpleFlashSelectFolder
                    )),
                    None
                ),
                self.recent_chips(
                    self.recent_paths
                        .recent(pickers::PickerKind::QfilFirmwareFolder.storage_key()),
                    |p| Message::SimpleFlash(SimpleFlashMsg::SimpleFlashFolderChosen(Some(p))),
                    "picker_recents",
                    false
                ),
            ]
            .spacing(6)
            .padding(28)
            .width(Length::Fill),
        )
        .height(Length::Fill)
        .into()
    }

    /// Confirm step — mirrors the firmware-flash confirm, but with fixed
    /// values: region edit / rollback bypass are OFF, and device region,
    /// flash target, and data-wipe outcome are all "unknown" because Simple
    /// Flash performs no detection or modification (the wipe outcome is
    /// decided solely by the firmware's own rawprogram).
    fn simple_flash_confirm_step(&self) -> Element<'_, Message> {
        let unknown = self.t("common_unknown").to_string();
        let off = self.t("flash_confirm_rb_off").to_string();
        let folder = self
            .simple_flash
            .firmware_folder
            .clone()
            .unwrap_or_else(|| "—".to_string());
        let warning = self.message_banner(
            BannerSeverity::Error,
            icon::banner_error(),
            self.t("flash_confirm_warning_title").to_string(),
            text(self.t("simple_flash_confirm_warning").to_string())
                .size(theme::text_size::BODY_SMALL)
                .style(error_container_text_style)
                .width(Length::Fill)
                .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
        );
        self.confirm_step_frame(
            vec![warning],
            vec![
                confirm_definition_row(self.t("flash_confirm_region"), &unknown),
                confirm_definition_row(self.t("flash_confirm_target"), &unknown),
                confirm_definition_row(self.t("flash_confirm_data"), &unknown),
                confirm_definition_row(self.t("flash_confirm_region_edit"), &off),
                confirm_definition_row(self.t("flash_confirm_rollback"), &off),
            ],
            vec![confirm_definition_row(
                self.t("flash_confirm_folder"),
                &folder,
            )],
        )
    }
}
