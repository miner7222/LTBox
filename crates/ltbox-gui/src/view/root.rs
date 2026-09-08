//! Root wizard view + steps + superkey/run-id/kernel-version popups. Extracted from `main.rs`.

use crate::*;
use iced::widget::{Space, button, column, container, row, text};
use iced::{Element, Length, Theme};
use ltbox_core::tr_args;
use theme::with_alpha;

impl App {
    pub(crate) fn view_root_wizard(&self) -> Element<'_, Message> {
        // Superkey / Run-ID / Kernel-version popups all render as
        // top-level M3 dialog overlays via `view()`'s layer stack —
        // do NOT early-return for any of them here, otherwise the
        // KPM step underneath would unmount and Cancel couldn't
        // restore the curated list.
        if self.log_popup_open && self.root.is_in_exec() {
            return self.log_popup_view();
        }
        let steps = self.root.active_steps();
        let step_labels: Vec<&str> = steps.iter().map(|k| self.t(k)).collect();
        let is_exec = self.root.step == 7;
        let step_bar = if is_exec {
            empty_wizard_step_bar()
        } else {
            wizard_step_bar(
                &step_labels,
                self.root.display_step(),
                self.window_size_class(),
            )
        };
        let body = match self.root.step {
            0 => self.root_family_step(),
            1 => {
                if self.root.is_skroot() {
                    self.root_skroot_flavor_step()
                } else {
                    self.root_mode_step()
                }
            }
            2 => {
                if self.root.is_gki() {
                    self.root_file_step(self.t("root_kernel_subtitle"))
                } else {
                    self.root_provider_step()
                }
            }
            3 => {
                if self.root.is_forks() {
                    self.root_file_step(self.t("root_apk_subtitle"))
                } else {
                    self.root_version_step()
                }
            }
            4 => self.root_nightly_source_step(),
            5 => self.root_folder_step(),
            6 => self.root_confirm_step(),
            8 => self.root_kpm_step(),
            _ => self.root_flash_step(),
        };
        let (step_title, app_bar_subtitle) = self.root_step_copy();
        let is_selection_step = match self.root.step {
            0 | 1 | 4 => true,
            2 => !self.root.is_gki(),
            3 => !self.root.is_forks(),
            _ => false,
        };
        let body = if is_exec || is_selection_step {
            body
        } else {
            wizard_step_body(step_title, body)
        };
        // Step 7 is in-progress — no nav. Step 8 (APatch KPM) needs
        // the normal Back/Next bar, so exclude only 7 explicitly.
        let nav = if self.root.step != 7 {
            let is_start = self.root.step == 6;
            let label_owned = if is_start {
                self.t("btn_start").to_string()
            } else {
                self.t("btn_next").to_string()
            };
            let can = self.root.can_next()
                && ltbox_core::model::capabilities(&self.device.model).root
                && !(self.operation.is_running() && is_start)
                && (!is_start || self.device_reachable());
            wizard_nav(self.root.step > 0, &label_owned, can, self.t("btn_back"))
        } else {
            empty_wizard_nav()
        };
        column![
            wizard_action_bar(
                self.window_size_class(),
                self.t("nav_root").to_string(),
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

    fn root_step_copy(&self) -> (String, Option<String>) {
        match self.root.step {
            0 => (self.t("root_type_title").to_string(), None),
            1 if self.root.is_skroot() => (self.t("root_skroot_flavor_title").to_string(), None),
            1 => {
                let family = self
                    .root
                    .family
                    .map(|f| self.t(f.label_key()))
                    .unwrap_or("?");
                (tr_args!("root_mode_title_tmpl", family = family), None)
            }
            2 if self.root.is_gki() => (self.t("root_kernel_title").to_string(), None),
            2 => {
                let family = self.root.family.unwrap_or(Family::KernelSU);
                (
                    tr_args!(
                        "root_provider_title_tmpl",
                        family = self.t(family.label_key())
                    ),
                    None,
                )
            }
            3 if self.root.is_forks() => (self.t("root_apk_title").to_string(), None),
            3 => (self.t("root_version_title").to_string(), None),
            4 => (self.t("root_source_title").to_string(), None),
            5 => (self.t("edl_loader_title").to_string(), None),
            6 => (self.t("root_confirm_title").to_string(), None),
            7 => {
                let (title, _) = self.exec_status_copy();
                (title, self.exec_app_bar_subtitle())
            }
            8 => (self.t("root_kpm_title").to_string(), None),
            _ => {
                let (title, _) = self.exec_status_copy();
                (title, self.exec_app_bar_subtitle())
            }
        }
    }

    pub(crate) fn root_kpm_step(&self) -> Element<'_, Message> {
        // No recents here — the KPM list already competes for vertical space.
        let kpm_selected = !self.root.kpm_paths.is_empty();
        let pick_btn = button(
            container(
                column![
                    text(self.t("btn_browse_kpm").to_string())
                        .size(14.0)
                        .center(),
                    text(self.t("root_kpm_desc").to_string())
                        .size(11.0)
                        .style(muted_style)
                        .center(),
                ]
                .spacing(6.0)
                .width(Length::Fill)
                .align_x(iced::Alignment::Center),
            )
            .padding([20.0, 24.0])
            .width(KPM_COLUMN_WIDTH)
            .style(move |t: &Theme| sel_card_style(t, kpm_selected)),
        )
        .on_press(Message::Root(RootMsg::RootSelectKpm))
        .padding(0)
        .style(move |t: &Theme, status| sel_card_btn_style(t, status, kpm_selected));

        // Same width as the browse card, not `Fill`. A fill-width child
        // ignores the parent column's centering and spans the whole
        // content area, which packed every row against the far left edge
        // while the card it belongs to sat centred — the two read as
        // unrelated. Matching widths makes them one column.
        let mut list = column![].spacing(4.0).width(KPM_COLUMN_WIDTH);
        for path in &self.root.kpm_paths {
            let name = std::path::Path::new(path)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| path.clone());
            let p_copy = path.clone();
            let remove = m3_icon_button(icon::kpm_remove(), 16.0, |t: &Theme, status| {
                let p = pal_of(t);
                button::Style {
                    background: Some(
                        with_alpha(p.on_surface, 0.10 + theme::state_alpha(status)).into(),
                    ),
                    text_color: p.on_surface,
                    border: iced::Border {
                        radius: theme::shape::SM.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }
            })
            .on_press(Message::Root(RootMsg::RootKpmRemove(p_copy)));
            list = list.push(
                row![
                    remove,
                    // Module filenames can be long and have no spaces, so
                    // break at glyph boundaries rather than overflowing
                    // the column the list now shares with the card.
                    text(name)
                        .size(12.0)
                        .style(on_surface_style)
                        .width(Length::Fill)
                        .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
                ]
                .spacing(10.0)
                .align_y(iced::Alignment::Center),
            );
        }

        let col = column![pick_btn, list,]
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

    pub(crate) fn root_superkey_popup(&self) -> Element<'_, Message> {
        // Two-stage flow: first-entry vs verification re-entry. The title and
        // subtitle swap so the user knows the first confirmation did not yet
        // commit the key.
        let on_verify_stage = self.root.superkey_first_entry.is_some();
        let title_key = if on_verify_stage {
            "apatch_superkey_verify_title"
        } else {
            "apatch_superkey_title"
        };
        let subtitle_key = if on_verify_stage {
            "apatch_superkey_verify_subtitle"
        } else {
            "apatch_superkey_subtitle"
        };
        let superkey = self.root.superkey_buffer.trim();
        let input_valid = (8..=63).contains(&superkey.len())
            && superkey.chars().all(|c| c.is_ascii_alphanumeric());
        let visible_error = self.error_msg.clone().filter(|_| !input_valid).or_else(|| {
            (!superkey.is_empty() && !input_valid)
                .then(|| self.t("apatch_superkey_invalid").to_string())
        });
        let input_style = if visible_error.is_some() {
            m3_text_input_error_style
        } else {
            m3_text_input_style
        };
        let mut input = iced::widget::text_input(
            self.t("apatch_superkey_placeholder"),
            &self.root.superkey_buffer,
        )
        .on_input(|__v| Message::Root(RootMsg::RootSuperkeyInput(__v)))
        .secure(true)
        .padding([8, 12])
        .line_height(iced::widget::text::LineHeight::Absolute(24.0.into()))
        .width(Length::Fill)
        .style(input_style);
        if input_valid {
            input = input.on_submit(Message::Root(RootMsg::RootSuperkeyConfirm));
        }
        // No field label: this dialog has one input and the headline already
        // names it, so a label would restate the title verbatim.
        let mut field = column![input].spacing(6);
        if let Some(error) = visible_error {
            field = field.push(dialog_field_error(error));
        }
        let header: Element<'_, Message> = column![
            text(self.t(title_key).to_string()).size(theme::text_size::TITLE_LARGE),
            text(self.t(subtitle_key).to_string())
                .size(theme::text_size::BODY_SMALL)
                .style(muted_style),
        ]
        .spacing(3)
        .into();
        let mut confirm = m3_filled_button(self.t("btn_ok").to_string());
        if input_valid {
            confirm = confirm.on_press(Message::Root(RootMsg::RootSuperkeyConfirm));
        }
        let footer: Element<'_, Message> = row![
            Space::new().width(Length::Fill),
            m3_outlined_button(self.t("btn_cancel").to_string())
                .on_press(Message::Root(RootMsg::RootSuperkeyCancel)),
            confirm,
        ]
        .spacing(10)
        .align_y(iced::Alignment::Center)
        .into();
        m3_dialog(dialog_sections(
            header,
            field.into(),
            footer,
            theme::DIALOG_WIDTH_SM,
            false,
        ))
    }

    pub(crate) fn root_run_id_popup(&self) -> Element<'_, Message> {
        let run_id = self.root.run_id_buffer.trim();
        let input_valid =
            !run_id.is_empty() && run_id.len() <= 12 && run_id.chars().all(|c| c.is_ascii_digit());
        let visible_error = self.error_msg.clone().filter(|_| !input_valid);
        let input_style = if visible_error.is_some() {
            m3_text_input_error_style
        } else {
            m3_text_input_style
        };
        let mut input = iced::widget::text_input(
            self.t("nightly_manual_placeholder"),
            &self.root.run_id_buffer,
        )
        .on_input(|__v| Message::Root(RootMsg::RootRunIdInput(__v)))
        .padding([8, 12])
        .line_height(iced::widget::text::LineHeight::Absolute(24.0.into()))
        .width(Length::Fill)
        .style(input_style);
        if input_valid {
            input = input.on_submit(Message::Root(RootMsg::RootRunIdConfirm));
        }
        let mut field = column![
            dialog_field_label(self.t("nightly_run_id_label").to_string()),
            input
        ]
        .spacing(6);
        if let Some(error) = visible_error {
            field = field.push(dialog_field_error(error));
        }
        let header: Element<'_, Message> = column![
            text(self.t("nightly_manual_title").to_string()).size(theme::text_size::TITLE_LARGE),
            text(self.t("nightly_manual_subtitle").to_string())
                .size(theme::text_size::BODY_SMALL)
                .style(muted_style),
        ]
        .spacing(3)
        .into();
        let mut confirm = m3_filled_button(self.t("btn_ok").to_string());
        if input_valid {
            confirm = confirm.on_press(Message::Root(RootMsg::RootRunIdConfirm));
        }
        let footer: Element<'_, Message> = row![
            Space::new().width(Length::Fill),
            m3_outlined_button(self.t("btn_cancel").to_string())
                .on_press(Message::Root(RootMsg::RootRunIdCancel)),
            confirm,
        ]
        .spacing(10)
        .align_y(iced::Alignment::Center)
        .into();
        m3_dialog(dialog_sections(
            header,
            field.into(),
            footer,
            theme::DIALOG_WIDTH_SM,
            false,
        ))
    }

    pub(crate) fn root_kernel_version_popup(&self) -> Element<'_, Message> {
        let kernel_version = self.root.kernel_version_buffer.trim();
        let input_valid =
            ltbox_patch::root_pipeline::normalize_ksu_kernel_version(kernel_version).is_some();
        let visible_error = self.error_msg.clone().filter(|_| !input_valid).or_else(|| {
            (!kernel_version.is_empty() && !input_valid)
                .then(|| self.t("root_kernel_version_invalid").to_string())
        });
        let input_style = if visible_error.is_some() {
            m3_text_input_error_style
        } else {
            m3_text_input_style
        };
        let mut input = iced::widget::text_input(
            self.t("root_kernel_version_placeholder"),
            &self.root.kernel_version_buffer,
        )
        .on_input(|__v| Message::Root(RootMsg::RootKernelVersionInput(__v)))
        .padding([8, 12])
        .line_height(iced::widget::text::LineHeight::Absolute(24.0.into()))
        .width(Length::Fill)
        .style(input_style);
        if input_valid {
            input = input.on_submit(Message::Root(RootMsg::RootKernelVersionConfirm));
        }
        // Single input under a headline that already names it.
        let mut field = column![input].spacing(6);
        if let Some(error) = visible_error {
            field = field.push(dialog_field_error(error));
        }
        let header: Element<'_, Message> = column![
            text(self.t("root_kernel_version_manual_title").to_string())
                .size(theme::text_size::TITLE_LARGE),
            text(self.t("root_kernel_version_manual_subtitle").to_string())
                .size(theme::text_size::BODY_SMALL)
                .style(muted_style),
        ]
        .spacing(3)
        .into();
        let mut confirm = m3_filled_button(self.t("btn_ok").to_string());
        if input_valid {
            confirm = confirm.on_press(Message::Root(RootMsg::RootKernelVersionConfirm));
        }
        let footer: Element<'_, Message> = row![
            Space::new().width(Length::Fill),
            m3_outlined_button(self.t("btn_cancel").to_string())
                .on_press(Message::Root(RootMsg::RootKernelVersionCancel)),
            confirm,
        ]
        .spacing(10)
        .align_y(iced::Alignment::Center)
        .into();
        m3_dialog(dialog_sections(
            header,
            field.into(),
            footer,
            theme::DIALOG_WIDTH_SM,
            false,
        ))
    }

    pub(crate) fn root_family_step(&self) -> Element<'_, Message> {
        let size_class = self.window_size_class();
        let content_width = self.window_size.0
            - match size_class {
                WindowSizeClass::Compact => SIDEBAR_RAIL_WIDTH,
                WindowSizeClass::Expanded => SIDEBAR_EXPANDED_WIDTH,
            };
        let xiaoxin_pro13 = !ltbox_core::model::capabilities(&self.device.model).root;
        let unsupported = tr_args!("model_unsupported", model = "TB376FC / TB390FU");
        let families = [
            Family::Magisk,
            Family::KernelSU,
            Family::APatch,
            Family::Skroot,
        ];
        let icon_size = self.wizard_list_icon(WIZARD_LIST_ICON_SIZE);
        let metrics = self.wizard_list_metrics(WIZARD_LIST_LABEL_SIZE, WIZARD_LIST_DESC_SIZE);
        let mk = |f: Family| -> Element<'_, Message> {
            if f == Family::KernelSU {
                wizard_list_option_card_recommended(
                    f.icon_sized(icon_size),
                    self.t(f.label_key()),
                    if xiaoxin_pro13 {
                        &unsupported
                    } else {
                        self.t(f.desc_key())
                    },
                    self.root.family == Some(f),
                    (!xiaoxin_pro13).then_some(Message::Root(RootMsg::RootFamily(f))),
                    metrics,
                    (
                        self.t("root_recommended_label"),
                        self.t("root_recommended_tip"),
                    ),
                )
            } else {
                wizard_list_option_card(
                    f.icon_sized(icon_size),
                    self.t(f.label_key()),
                    if xiaoxin_pro13 {
                        &unsupported
                    } else {
                        self.t(f.desc_key())
                    },
                    self.root.family == Some(f),
                    (!xiaoxin_pro13).then_some(Message::Root(RootMsg::RootFamily(f))),
                    metrics,
                )
            }
        };

        let mut cards = column![].spacing(8.0).width(Length::Fill);
        for f in families {
            cards = cards.push(mk(f));
        }

        wizard_selection_step(
            size_class,
            content_width,
            self.t("root_type_title").to_string(),
            cards.into(),
            Some((
                self.t("root_type_title").to_string(),
                vec![self.t("root_type_subtitle").to_string()],
            )),
        )
    }

    pub(crate) fn root_provider_step(&self) -> Element<'_, Message> {
        let size_class = self.window_size_class();
        let content_width = self.window_size.0
            - match size_class {
                WindowSizeClass::Compact => SIDEBAR_RAIL_WIDTH,
                WindowSizeClass::Expanded => SIDEBAR_EXPANDED_WIDTH,
            };
        let family = self.root.family.unwrap_or(Family::KernelSU);
        let providers = family.providers();
        let icon_size = self.wizard_list_icon(WIZARD_LIST_ICON_SIZE);
        let metrics = self.wizard_list_metrics(WIZARD_LIST_LABEL_SIZE, WIZARD_LIST_DESC_SIZE);
        let card = |p: Provider, selected: bool| -> Element<'static, Message> {
            let sub = p.desc_key().map(|k| self.t(k)).unwrap_or("");
            if p == Provider::KernelSU {
                wizard_list_option_card_recommended(
                    p.icon_sized(icon_size),
                    self.t(p.label_key()),
                    sub,
                    selected,
                    Some(Message::Root(RootMsg::RootProvider(p))),
                    metrics,
                    (
                        self.t("root_recommended_label"),
                        self.t("root_recommended_tip"),
                    ),
                )
            } else {
                wizard_list_option_card(
                    p.icon_sized(icon_size),
                    self.t(p.label_key()),
                    sub,
                    selected,
                    Some(Message::Root(RootMsg::RootProvider(p))),
                    metrics,
                )
            }
        };

        let mut cards = column![].spacing(8.0).width(Length::Fill);
        for &p in providers {
            cards = cards.push(card(p, self.root.provider == Some(p)));
        }
        wizard_selection_step(
            size_class,
            content_width,
            tr_args!(
                "root_provider_title_tmpl",
                family = self.t(family.label_key())
            ),
            cards.into(),
            Some((
                tr_args!(
                    "root_provider_title_tmpl",
                    family = self.t(family.label_key())
                ),
                vec![self.t("root_provider_subtitle").to_string()],
            )),
        )
    }

    pub(crate) fn root_file_step(&self, subtitle: &str) -> Element<'_, Message> {
        let selected = self.root.file_path.is_some();
        let status_text = if let Some(p) = &self.root.file_path {
            p.clone()
        } else {
            self.t("flash_folder_placeholder").to_string()
        };

        let btn_label = if self.root.is_gki() {
            self.t("btn_browse_kernel_image")
        } else {
            self.t("btn_browse_apk")
        };

        let btn = button(
            container(
                column![
                    text(btn_label.to_string()).size(14.0).center(),
                    text(subtitle.to_string())
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
        .on_press(Message::Root(RootMsg::RootSelectFile))
        .padding(0)
        .style(move |t: &Theme, status| sel_card_btn_style(t, status, selected));

        // Root OTA file picker flips between AnyKernel3 zip + raw
        // boot.img (GKI route) and provider APK (Magisk fork / APatch
        // manual) — mirror the dialog filter so recents don't surface
        // the wrong family.
        let accepted: &[&str] = if self.root.is_gki() {
            &["zip", "img"]
        } else {
            &["apk"]
        };
        let chips = self.recent_file_chips(
            accepted,
            |p| Message::RecentFilePicked(PickerTarget::RootFile, p),
            "picker_recents",
        );
        let col = column![
            btn,
            text(status_text)
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

    pub(crate) fn root_folder_step(&self) -> Element<'_, Message> {
        // Root pipeline now needs only the EDL loader (`.melf`) — the
        // full firmware folder was dropped when dump/flash stopped
        // depending on `rawprogram*.xml` and started resolving partition
        // names against the device's on-storage GPT. File-pick only.
        let selected = self.root.folder_path.is_some();
        let status = if let Some(p) = &self.root.folder_path {
            p.clone()
        } else {
            self.t("edl_loader_placeholder").to_string()
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
        .on_press(Message::Root(RootMsg::RootSelectFolder))
        .padding(0)
        .style(move |t: &Theme, status| sel_card_btn_style(t, status, selected));
        let chips = self.recent_file_chips(
            LOADER_PICKER_EXTS,
            |p| Message::Root(RootMsg::RootLoaderChosen(Some(p))),
            "picker_recents",
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

    pub(crate) fn root_mode_step(&self) -> Element<'_, Message> {
        let size_class = self.window_size_class();
        let content_width = self.window_size.0
            - match size_class {
                WindowSizeClass::Compact => SIDEBAR_RAIL_WIDTH,
                WindowSizeClass::Expanded => SIDEBAR_EXPANDED_WIDTH,
            };
        let icon_size = self.wizard_list_icon(WIZARD_LIST_GLYPH_ICON_SIZE);
        let metrics = self.wizard_list_metrics(WIZARD_LIST_LABEL_SIZE, WIZARD_LIST_DESC_SIZE);
        let tb323fu = !ltbox_core::model::capabilities(&self.device.model).gki_root;
        let unsupported_canoe = tr_args!("model_unsupported", model = self.device.model.as_str());
        let lkm_card = wizard_list_option_card_recommended(
            RootMode::Lkm.icon(icon_size),
            self.t(RootMode::Lkm.label_key()),
            self.t(RootMode::Lkm.desc_key()),
            self.root.mode == Some(RootMode::Lkm),
            Some(Message::Root(RootMsg::RootMode(RootMode::Lkm))),
            metrics,
            (
                self.t("root_recommended_label"),
                self.t("root_recommended_tip"),
            ),
        );
        // TODO(root): LTBox currently only swaps the boot.img Image for
        // GKI, which corrupts boot on TB323FU. Keep GKI disabled until
        // vbmeta handling is added.
        let gki_card: Element<'_, Message> = if tb323fu {
            wizard_list_option_card(
                RootMode::Gki.icon_disabled(icon_size),
                self.t(RootMode::Gki.label_key()),
                &unsupported_canoe,
                false,
                None,
                metrics,
            )
        } else {
            wizard_list_option_card(
                RootMode::Gki.icon(icon_size),
                self.t(RootMode::Gki.label_key()),
                self.t(RootMode::Gki.desc_key()),
                self.root.mode == Some(RootMode::Gki),
                Some(Message::Root(RootMsg::RootMode(RootMode::Gki))),
                metrics,
            )
        };
        let cards = column![lkm_card, gki_card].spacing(8.0).width(Length::Fill);
        wizard_selection_step(
            size_class,
            content_width,
            tr_args!(
                "root_mode_title_tmpl",
                family = self
                    .root
                    .family
                    .map(|family| self.t(family.label_key()))
                    .unwrap_or("?")
            ),
            cards.into(),
            Some((
                tr_args!(
                    "root_mode_title_tmpl",
                    family = self
                        .root
                        .family
                        .map(|family| self.t(family.label_key()))
                        .unwrap_or("?")
                ),
                vec![self.t("root_mode_subtitle").to_string()],
            )),
        )
    }

    pub(crate) fn root_skroot_flavor_step(&self) -> Element<'_, Message> {
        let size_class = self.window_size_class();
        let content_width = self.window_size.0
            - match size_class {
                WindowSizeClass::Compact => SIDEBAR_RAIL_WIDTH,
                WindowSizeClass::Expanded => SIDEBAR_EXPANDED_WIDTH,
            };
        let icon_size = self.wizard_list_icon(WIZARD_LIST_GLYPH_ICON_SIZE);
        let metrics = self.wizard_list_metrics(WIZARD_LIST_LABEL_SIZE, WIZARD_LIST_DESC_SIZE);
        let lite = wizard_list_option_card(
            SkrootFlavor::Lite.icon(icon_size),
            self.t(SkrootFlavor::Lite.label_key()),
            self.t(SkrootFlavor::Lite.desc_key()),
            self.root.skroot_flavor == Some(SkrootFlavor::Lite),
            Some(Message::Root(RootMsg::RootSkrootFlavor(SkrootFlavor::Lite))),
            metrics,
        );
        let pro = wizard_list_option_card(
            SkrootFlavor::Pro.icon_disabled(icon_size),
            self.t(SkrootFlavor::Pro.label_key()),
            self.t(SkrootFlavor::Pro.desc_key()),
            false,
            None,
            metrics,
        );

        let cards = column![lite, pro].spacing(8.0).width(Length::Fill);
        wizard_selection_step(
            size_class,
            content_width,
            self.t("root_skroot_flavor_title").to_string(),
            cards.into(),
            Some((
                self.t("root_skroot_flavor_title").to_string(),
                vec![self.t("root_skroot_flavor_subtitle").to_string()],
            )),
        )
    }

    pub(crate) fn root_version_step(&self) -> Element<'_, Message> {
        let size_class = self.window_size_class();
        let content_width = self.window_size.0
            - match size_class {
                WindowSizeClass::Compact => SIDEBAR_RAIL_WIDTH,
                WindowSizeClass::Expanded => SIDEBAR_EXPANDED_WIDTH,
            };
        let icon_size = self.wizard_list_icon(WIZARD_LIST_GLYPH_ICON_SIZE);
        let metrics = self.wizard_list_metrics(WIZARD_LIST_LABEL_SIZE, WIZARD_LIST_DESC_SIZE);
        let mk = |choice: VerChoice| -> Element<'_, Message> {
            if choice == VerChoice::Stable {
                wizard_list_option_card_recommended(
                    choice.icon(icon_size),
                    self.t(choice.label_key()),
                    self.t(choice.desc_key()),
                    self.root.version == Some(choice),
                    Some(Message::Root(RootMsg::RootVersion(choice))),
                    metrics,
                    (
                        self.t("root_recommended_label"),
                        self.t("root_recommended_tip"),
                    ),
                )
            } else {
                wizard_list_option_card(
                    choice.icon(icon_size),
                    self.t(choice.label_key()),
                    self.t(choice.desc_key()),
                    self.root.version == Some(choice),
                    Some(Message::Root(RootMsg::RootVersion(choice))),
                    metrics,
                )
            }
        };

        // ReSukiSU ships nightlies only — hide the Stable card so users
        // can't pick a channel that has no release assets. Other providers
        // keep both.
        let cards = if self.root.provider == Some(Provider::ReSukiSU) {
            column![mk(VerChoice::Nightly)].spacing(8.0)
        } else {
            column![mk(VerChoice::Stable), mk(VerChoice::Nightly)].spacing(8.0)
        };

        wizard_selection_step(
            size_class,
            content_width,
            self.t("root_version_title").to_string(),
            cards.width(Length::Fill).into(),
            Some((
                self.t("root_version_title").to_string(),
                vec![self.t("root_version_subtitle").to_string()],
            )),
        )
    }

    pub(crate) fn root_nightly_source_step(&self) -> Element<'_, Message> {
        let size_class = self.window_size_class();
        let content_width = self.window_size.0
            - match size_class {
                WindowSizeClass::Compact => SIDEBAR_RAIL_WIDTH,
                WindowSizeClass::Expanded => SIDEBAR_EXPANDED_WIDTH,
            };
        let icon_size = self.wizard_list_icon(WIZARD_LIST_GLYPH_ICON_SIZE);
        let metrics = self.wizard_list_metrics(WIZARD_LIST_LABEL_SIZE, WIZARD_LIST_DESC_SIZE);
        let mk = |src: NightlySource| -> Element<'_, Message> {
            wizard_list_option_card(
                src.icon(icon_size),
                self.t(src.label_key()),
                self.t(src.desc_key()),
                self.root.nightly_source == Some(src),
                Some(Message::Root(RootMsg::RootNightlySource(src))),
                metrics,
            )
        };

        // Committed ManualInput shows a chip beneath the cards; click re-opens.
        let chip: Element<'_, Message> =
            match (self.root.nightly_source, self.root.run_id.as_deref()) {
                (Some(NightlySource::ManualInput), Some(id)) if !id.is_empty() => {
                    let label = tr_args!("nightly_manual_committed", id = id);
                    button(
                        text(label)
                            .size(theme::text_size::BODY_MEDIUM)
                            .style(on_surface_style),
                    )
                    .padding([8.0, 14.0])
                    .on_press(Message::Root(RootMsg::RootNightlySource(
                        NightlySource::ManualInput,
                    )))
                    .style(|t: &Theme, status| {
                        let p = pal_of(t);
                        let bg_a = 0.10 + theme::state_alpha(status);
                        button::Style {
                            background: Some(with_alpha(p.on_surface, bg_a).into()),
                            text_color: p.on_surface,
                            border: iced::Border {
                                radius: 6.0.into(),
                                ..Default::default()
                            },
                            ..Default::default()
                        }
                    })
                    .into()
                }
                _ => Space::new().height(0).into(),
            };

        let cards = column![
            mk(NightlySource::AutoDetect),
            mk(NightlySource::ManualInput),
            chip,
        ]
        .spacing(14.0)
        .width(Length::Fill)
        .align_x(iced::Alignment::Center);
        wizard_selection_step(
            size_class,
            content_width,
            self.t("root_source_title").to_string(),
            cards.into(),
            Some((
                self.t("root_source_title").to_string(),
                vec![self.t("root_source_subtitle").to_string()],
            )),
        )
    }

    pub(crate) fn root_confirm_step(&self) -> Element<'_, Message> {
        let dash = "—".to_string();
        let fam = self
            .root
            .family
            .map(|f| self.t(f.label_key()).to_string())
            .unwrap_or_else(|| dash.clone());

        let mut grid_rows = vec![info_kv_center(self.t("root_step_type"), &fam)];
        let mut trailing_rows = Vec::new();

        if self.root.is_skroot() {
            let flavor = self
                .root
                .skroot_flavor
                .map(|f| self.t(f.label_key()).to_string())
                .unwrap_or_else(|| dash.clone());
            grid_rows.push(info_kv_center(self.t("root_step_skroot_flavor"), &flavor));
        } else {
            let mode = self
                .root
                .mode
                .map(|m| self.t(m.label_key()).to_string())
                .unwrap_or_else(|| dash.clone());
            grid_rows.push(info_kv_center(self.t("root_step_mode"), &mode));
        }

        if self.root.is_gki() {
            let path = self.root.file_path.clone().unwrap_or_else(|| dash.clone());
            trailing_rows.push(info_kv_center(self.t("root_step_kernel"), &path));
        } else if self.root.is_forks() {
            let path = self.root.file_path.clone().unwrap_or_else(|| dash.clone());
            grid_rows.push(info_kv_center(
                self.t("root_step_provider"),
                self.t("provider_magisk_forks"),
            ));
            trailing_rows.push(info_kv_center(self.t("root_step_apk"), &path));
        } else if !self.root.is_skroot() {
            let prov = self
                .root
                .provider
                .map(|p| self.t(p.label_key()).to_string())
                .unwrap_or_else(|| dash.clone());
            let ver = self
                .root
                .version
                .map(|v| self.t(v.label_key()).to_string())
                .unwrap_or_else(|| dash.clone());
            grid_rows.push(info_kv_center(self.t("root_step_provider"), &prov));
            grid_rows.push(info_kv_center(self.t("root_step_version"), &ver));
            if self.root.is_nightly() {
                let src = self
                    .root
                    .nightly_source
                    .map(|s| self.t(s.label_key()).to_string())
                    .unwrap_or_else(|| dash.clone());
                grid_rows.push(info_kv_center(self.t("root_step_source"), &src));
                if self.root.nightly_source == Some(NightlySource::ManualInput) {
                    let id = self.root.run_id.clone().unwrap_or_else(|| dash.clone());
                    grid_rows.push(info_kv_center(self.t("nightly_run_id_label"), &id));
                }
            }
        }

        if self.root.is_apatch() {
            // Count only — don't echo paths (noisy) or the superkey (secret).
            let kpm_summary = if self.root.kpm_paths.is_empty() {
                self.t("root_kpm_none").to_string()
            } else {
                tr_args!(
                    "root_kpm_count_tmpl",
                    n = self.root.kpm_paths.len().to_string()
                )
            };
            grid_rows.push(info_kv_center(self.t("root_step_kpm"), &kpm_summary));
        }

        let folder = self
            .root
            .folder_path
            .clone()
            .unwrap_or_else(|| dash.clone());
        trailing_rows.push(info_kv_center(self.t("edl_loader_label"), &folder));

        self.confirm_step_frame(vec![], grid_rows, trailing_rows)
    }

    pub(crate) fn root_flash_step(&self) -> Element<'_, Message> {
        self.exec_step_view()
    }
}
