//! Unroot wizard view + steps. Extracted from `main.rs`.

use crate::*;
use iced::widget::{button, column, container, text};
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
            wizard_step_body(step_title, body)
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
        (title, None)
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
            Some((
                self.t("unroot_method_title").to_string(),
                vec![self.t("unroot_method_subtitle").to_string()],
            )),
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
        let selected = self.unroot.folder_path.is_some();
        let desc_owned = self
            .unroot
            .unroot_type
            .map(|t| self.unroot_folder_desc(t).to_string())
            .unwrap_or_else(|| self.t("unroot_folder_placeholder").to_string());
        let status = if let Some(p) = &self.unroot.folder_path {
            p.clone()
        } else {
            self.t("flash_folder_placeholder").to_string()
        };
        let btn = button(
            container(
                column![
                    text(self.t("btn_browse_folder").to_string())
                        .size(14.0)
                        .center(),
                    text(desc_owned).size(11.0).style(muted_style).center(),
                ]
                .spacing(6.0)
                .width(Length::Fill)
                .align_x(iced::Alignment::Center),
            )
            .padding([20.0, 24.0])
            .width(Length::Fixed(280.0))
            .style(move |t: &Theme| sel_card_style(t, selected)),
        )
        .on_press(Message::Unroot(UnrootMsg::UnrootSelectFolder))
        .padding(0)
        .style(move |t: &Theme, status| sel_card_btn_style(t, status, selected));
        let chips = self.recent_chips(
            self.recent_paths
                .recent(PickerTarget::UnrootFolder.kind().storage_key()),
            |p| Message::RecentFolderPicked(PickerTarget::UnrootFolder, p),
            "picker_recents",
            false,
        );
        let col = column![
            btn,
            text(status)
                .size(12.0)
                .width(Length::Fill)
                .style(move |t: &Theme| {
                    let p = pal_of(t);
                    iced::widget::text::Style {
                        color: Some(if selected { p.success } else { p.outline }),
                    }
                })
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
            vec![info_kv_center(self.t("unroot_step_method"), &method)],
            vec![
                info_kv_center(self.t("edl_loader_label"), &loader),
                info_kv_center(self.t("unroot_folder_title"), &folder),
            ],
        )
    }

    pub(crate) fn unroot_exec_step(&self) -> Element<'_, Message> {
        self.exec_step_view()
    }
}
