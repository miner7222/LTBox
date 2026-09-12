//! Unroot wizard view + steps. Extracted from `main.rs`.

use crate::*;
use iced::widget::{Space, button, column, container, row, scrollable, text};
use iced::{Element, Length, Theme};
use ltbox_core::tr_args;

impl App {
    pub(crate) fn view_unroot_wizard(&self) -> Element<'_, Message> {
        if self.log_popup_open && self.unroot.is_in_exec() {
            return self.log_popup_view();
        }
        let step_labels: Vec<&str> = UNROOT_STEPS.iter().map(|k| self.t(k)).collect();
        let is_exec = self.unroot.is_in_exec();
        let step_bar = if is_exec {
            empty_wizard_step_bar()
        } else {
            wizard_step_bar(&step_labels, self.unroot.step, self.window_size_class())
        };
        let body = match self.unroot.step {
            0 => self.unroot_type_step(),
            1 => self.unroot_loader_step(),
            2 => self.unroot_folder_step(),
            3 => self.unroot_confirm_step(),
            _ => self.unroot_exec_step(),
        };
        let (step_title, app_bar_subtitle) = self.unroot_step_copy();
        let body = if is_exec || self.unroot.step == 0 {
            body
        } else {
            if matches!(self.unroot.step, 1 | 2) {
                self.wizard_picker_step(step_title, body)
            } else {
                wizard_step_body(step_title, body)
            }
        };
        let nav = if self.unroot.step < 4 {
            let is_start = self.unroot.step == 3;
            let label_owned = if is_start {
                self.t("btn_start").to_string()
            } else {
                self.t("btn_next").to_string()
            };
            let can = self.unroot.can_next()
                && ltbox_core::model::capabilities(&self.device.model).unroot
                && !(self.operation.is_running() && is_start)
                && (!is_start || self.device_reachable());
            wizard_nav_generic(
                self.unroot.step > 0,
                &label_owned,
                can,
                self.t("btn_back"),
                Message::Unroot(UnrootMsg::UnrootBack),
                Message::Unroot(UnrootMsg::UnrootNext),
            )
        } else {
            empty_wizard_nav()
        };
        column![
            wizard_action_bar(
                self.window_size_class(),
                self.t("nav_unroot").to_string(),
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

    fn unroot_step_copy(&self) -> (String, Option<String>) {
        let title = match self.unroot.step {
            0 => self.t("unroot_method_title").to_string(),
            1 => self.t("edl_loader_title").to_string(),
            2 => self.t("unroot_folder_title").to_string(),
            3 => self.t("unroot_confirm_title").to_string(),
            _ => {
                let (title, _) = self.exec_status_copy();
                return (title, self.exec_app_bar_subtitle());
            }
        };
        (
            title,
            match self.unroot.step {
                1 => Some(self.loader_picker_subtitle()),
                2 => self
                    .unroot
                    .unroot_type
                    .map(|kind| self.unroot_folder_desc(kind).to_string()),
                _ => None,
            },
        )
    }

    pub(crate) fn unroot_type_step(&self) -> Element<'_, Message> {
        let size_class = self.window_size_class();
        let content_width = self.window_size.0
            - match size_class {
                WindowSizeClass::Compact => SIDEBAR_RAIL_WIDTH,
                WindowSizeClass::Expanded => SIDEBAR_EXPANDED_WIDTH,
            };
        let icon_size = self.wizard_list_icon(WIZARD_LIST_GLYPH_ICON_SIZE);
        let metrics = self.wizard_list_metrics(WIZARD_LIST_LABEL_SIZE, WIZARD_LIST_DESC_SIZE);
        let xiaoxin_pro13 = !ltbox_core::model::capabilities(&self.device.model).unroot;
        let unsupported = tr_args!("model_unsupported", model = "TB376FC / TB390FU");
        // Unroot reuses the Lucide puzzle/layers glyphs that the root
        // wizard uses for the LKM/GKI pick — context (title + label)
        // disambiguates.
        let lkm_icon = lucide_list_primary(icon::root_lkm(), icon_size);
        let gki_icon = lucide_list_primary(icon::root_gki(), icon_size);
        let lkm_card = if xiaoxin_pro13 {
            wizard_list_option_card(
                lucide_list_disabled(icon::root_lkm(), icon_size),
                self.t(UnrootType::MagiskLkm.label_key()),
                &unsupported,
                false,
                None,
                metrics,
            )
        } else {
            wizard_list_option_card(
                lkm_icon,
                self.t(UnrootType::MagiskLkm.label_key()),
                self.t(UnrootType::MagiskLkm.desc_key()),
                self.unroot.unroot_type == Some(UnrootType::MagiskLkm),
                Some(Message::Unroot(UnrootMsg::SetUnrootType(
                    UnrootType::MagiskLkm,
                ))),
                metrics,
            )
        };
        let gki_card = if xiaoxin_pro13 {
            wizard_list_option_card(
                lucide_list_disabled(icon::root_gki(), icon_size),
                self.t(UnrootType::APatchGki.label_key()),
                &unsupported,
                false,
                None,
                metrics,
            )
        } else {
            wizard_list_option_card(
                gki_icon,
                self.t(UnrootType::APatchGki.label_key()),
                self.t(UnrootType::APatchGki.desc_key()),
                self.unroot.unroot_type == Some(UnrootType::APatchGki),
                Some(Message::Unroot(UnrootMsg::SetUnrootType(
                    UnrootType::APatchGki,
                ))),
                metrics,
            )
        };
        let cards = column![lkm_card, gki_card].spacing(8.0).width(Length::Fill);
        wizard_selection_step(
            size_class,
            content_width,
            self.t("unroot_method_title").to_string(),
            cards.into(),
            Some((self.t("unroot_method_title").to_string(), vec![])),
        )
    }

    pub(crate) fn unroot_loader_step(&self) -> Element<'_, Message> {
        self.loader_picker_card(
            &self.unroot.loader_path,
            self.unroot.loader_error.as_ref(),
            Message::Unroot(UnrootMsg::UnrootSelectLoader),
            |p| Message::Unroot(UnrootMsg::UnrootLoaderChosen(Some(p))),
        )
    }

    pub(crate) fn unroot_folder_step(&self) -> Element<'_, Message> {
        let mut backups = column![
            text(self.t("unroot_backups_title").to_string())
                .size(theme::text_size::TITLE_MEDIUM)
                .font(theme::emphasis::medium()),
        ]
        .spacing(8)
        .width(Length::Fill);
        if let Some(error) = &self.unroot.backup_scan_error {
            backups = backups.push(dialog_field_error(format!(
                "{}: {error}",
                self.t("unroot_backups_error")
            )));
        } else if self.unroot.backup_folders.is_empty() {
            backups = backups.push(
                container(
                    text(self.t("unroot_backups_empty").to_string())
                        .size(theme::text_size::BODY_SMALL)
                        .style(muted_style),
                )
                .padding([12, 4]),
            );
        } else {
            for entry in &self.unroot.backup_folders {
                let path = entry.path.to_string_lossy().into_owned();
                let name = entry
                    .path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.clone());
                let selected = self.unroot.folder_path.as_deref() == Some(path.as_str());
                let folder = button(
                    row![
                        selection_radio(selected, true, false),
                        text(name).size(WIZARD_LIST_LABEL_SIZE).width(Length::Fill),
                    ]
                    .spacing(12)
                    .align_y(iced::Alignment::Center),
                )
                .padding([12, 16])
                .width(Length::Fill)
                .on_press(Message::Unroot(UnrootMsg::UnrootBackupPicked(path.clone())))
                .style(move |t: &Theme, status| sel_card_btn_style_for(t, status, selected, false));
                let mut item = row![folder]
                    .spacing(8)
                    .align_y(iced::Alignment::Center)
                    .width(Length::Fill);
                if entry.has_manifest {
                    let tooltip_copy = self.t("unroot_backup_details_tooltip").to_string();
                    let detail = iced::widget::tooltip(
                        button(
                            container(text("?").size(14.0).font(theme::emphasis::medium()))
                                .width(Length::Fixed(24.0))
                                .height(Length::Fixed(24.0))
                                .align_x(iced::Alignment::Center)
                                .align_y(iced::Alignment::Center)
                                .style(|t: &Theme| container::Style {
                                    border: iced::Border {
                                        color: pal_of(t).outline,
                                        width: 1.0,
                                        radius: 12.0.into(),
                                    },
                                    ..Default::default()
                                }),
                        )
                        .padding(12)
                        .width(Length::Fixed(48.0))
                        .height(Length::Fixed(48.0))
                        .on_press(Message::Unroot(UnrootMsg::UnrootBackupManifestOpen(path)))
                        .style(|theme: &Theme, status| {
                            let palette = pal_of(theme);
                            button::Style {
                                background: theme::state_layer_bg(status, palette.primary)
                                    .map(Into::into),
                                text_color: palette.primary,
                                border: iced::Border {
                                    color: palette.outline,
                                    width: 0.0,
                                    radius: theme::shape::FULL.into(),
                                },
                                ..Default::default()
                            }
                        }),
                        container(text(tooltip_copy).size(11.0))
                            .padding([6, 10])
                            .max_width(280.0)
                            .style(|theme: &Theme| theme::tooltip_style(theme, theme::shape::SM)),
                        iced::widget::tooltip::Position::Top,
                    )
                    .gap(6.0);
                    item = item.push(detail);
                }
                backups = backups.push(item);
            }
        }

        scrollable(
            column![
                self.wizard_picker_row(
                    self.unroot.folder_path.as_deref(),
                    PickerPathKind::Folder,
                    Some(Message::Unroot(UnrootMsg::UnrootSelectFolder)),
                    None
                ),
                backups,
            ]
            .spacing(20)
            .padding(28)
            .width(Length::Fill),
        )
        .height(Length::Fill)
        .into()
    }

    pub(crate) fn unroot_backup_manifest_popup(&self) -> Element<'_, Message> {
        let Some(dialog) = self.unroot.backup_manifest_dialog.as_ref() else {
            return container(text("")).into();
        };
        let folder = dialog.folder.display().to_string();
        let header = column![
            text(self.t("unroot_backup_manifest_title").to_string())
                .size(theme::text_size::TITLE_LARGE),
            text(folder)
                .size(theme::text_size::BODY_SMALL)
                .style(muted_style),
        ]
        .spacing(12);
        let body: Element<'_, Message> = match &dialog.result {
            Ok(info) => {
                let missing = "—".to_string();
                info_key_value_table(vec![
                    (
                        self.t("unroot_backup_manifest_model").to_string(),
                        info.model.clone().unwrap_or_else(|| missing.clone()),
                    ),
                    (
                        self.t("unroot_backup_manifest_fingerprint").to_string(),
                        info.fingerprint.clone().unwrap_or_else(|| missing.clone()),
                    ),
                    (
                        self.t("unroot_backup_manifest_recorded_at").to_string(),
                        info.recorded_at.clone().unwrap_or_else(|| missing.clone()),
                    ),
                    (
                        self.t("unroot_backup_manifest_slot").to_string(),
                        info.slot.clone().unwrap_or(missing),
                    ),
                ])
            }
            Err(error) => dialog_field_error(format!(
                "{}: {error}",
                self.t("unroot_backup_manifest_error")
            )),
        };
        let close = m3_outlined_button(self.t("btn_close").to_string())
            .on_press(Message::Unroot(UnrootMsg::UnrootBackupManifestClose));
        m3_dialog(dialog_sections(
            header.into(),
            body,
            row![Space::new().width(Length::Fill), close]
                .align_y(iced::Alignment::Center)
                .into(),
            theme::DIALOG_WIDTH_MD,
            dialog.result.is_ok(),
        ))
    }

    pub(crate) fn unroot_confirm_step(&self) -> Element<'_, Message> {
        let dash = "—".to_string();
        let method = self
            .unroot
            .unroot_type
            .map(|t| self.t(t.label_key()).to_string())
            .unwrap_or_else(|| dash.clone());
        let loader = self
            .unroot
            .loader_path
            .clone()
            .unwrap_or_else(|| dash.clone());
        let folder = self
            .unroot
            .folder_path
            .clone()
            .unwrap_or_else(|| dash.clone());
        self.confirm_step_frame(
            vec![],
            vec![confirm_definition_row(
                self.t("unroot_step_method"),
                &method,
            )],
            vec![
                confirm_path_row(self.t("edl_loader_label"), &loader),
                confirm_path_row(self.t("unroot_folder_title"), &folder),
            ],
        )
    }

    pub(crate) fn unroot_exec_step(&self) -> Element<'_, Message> {
        self.exec_step_view()
    }
}
