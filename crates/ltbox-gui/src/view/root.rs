//! Root wizard view + steps + superkey/run-id/kernel-version popups. Extracted from `main.rs`.

use crate::focus_button::{self as button, button};
use crate::*;
use iced::widget::{Space, column, container, row, scrollable, text};
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
        let pick_btn = self.wizard_picker_row(
            None,
            PickerPathKind::File,
            Some(Message::Root(RootMsg::RootSelectKpm)),
            None,
        );
        let mut list = column![].spacing(4.0).width(Length::Fill);
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

        let col = column![
            pick_btn,
            text(self.t("root_kpm_desc").to_string())
                .size(theme::text_size::BODY_SMALL)
                .style(muted_style),
            list,
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

    pub(crate) fn root_release_popup(&self) -> Element<'_, Message> {
        let header = column![
            text(self.t("root_release_title"))
                .size(theme::text_size::TITLE_LARGE)
                .style(on_surface_style),
            text(self.t("root_release_subtitle"))
                .size(theme::text_size::BODY_SMALL)
                .style(muted_style),
        ]
        .spacing(6)
        .into();
        let mut body = column![].spacing(8);
        if self.root.release_request.is_some() {
            body = body.push(
                text(self.t("root_release_loading"))
                    .size(theme::text_size::BODY_MEDIUM)
                    .style(muted_style),
            );
        } else if let Some(error) = &self.root.release_error {
            body = body.push(dialog_field_error(format!(
                "{}\n{error}",
                self.t("root_release_error")
            )));
        } else if self.root.releases.is_empty() {
            body = body.push(
                text(self.t("root_release_empty"))
                    .size(theme::text_size::BODY_MEDIUM)
                    .style(muted_style),
            );
        } else {
            for (index, release) in self.root.releases.iter().enumerate() {
                let label = format!(
                    "{} · {} · {}",
                    release.tag,
                    self.t(if release.prerelease {
                        "root_release_prerelease"
                    } else {
                        "root_release_stable"
                    }),
                    release
                        .published_at
                        .get(..10)
                        .unwrap_or(&release.published_at)
                );
                body = body.push(focus_button::actionable(
                    iced::widget::radio(label, index, self.root.release_selection, |index| {
                        Message::Root(RootMsg::RootReleaseSelect(index))
                    })
                    .text_size(theme::text_size::BODY_MEDIUM)
                    .size(20)
                    .spacing(12)
                    .style(|t: &Theme, status| {
                        let p = pal_of(t);
                        let selected = match status {
                            iced::widget::radio::Status::Active { is_selected }
                            | iced::widget::radio::Status::Hovered { is_selected } => is_selected,
                        };
                        iced::widget::radio::Style {
                            background: with_alpha(
                                p.primary,
                                if matches!(status, iced::widget::radio::Status::Hovered { .. }) {
                                    0.08
                                } else {
                                    0.0
                                },
                            )
                            .into(),
                            dot_color: p.primary,
                            border_width: 2.0,
                            border_color: if selected {
                                p.primary
                            } else {
                                p.on_surface_variant
                            },
                            text_color: Some(p.on_surface),
                        }
                    }),
                    Some(Message::Root(RootMsg::RootReleaseSelect(index))),
                ));
            }
        }
        let mut confirm = m3_filled_button(self.t("btn_ok").to_string());
        if self.root.release_request.is_none() && self.root.release_selection.is_some() {
            confirm = confirm.on_press(Message::Root(RootMsg::RootReleaseConfirm));
        }
        let footer = row![
            Space::new().width(Length::Fill),
            m3_outlined_button(self.t("btn_cancel").to_string())
                .on_press(Message::Root(RootMsg::RootReleaseCancel)),
            confirm,
        ]
        .spacing(10)
        .align_y(iced::Alignment::Center)
        .into();
        m3_dialog(dialog_sections(
            header,
            body.into(),
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

        let mut cards = column![].spacing(2.0).width(Length::Fill);
        for f in families {
            cards = cards.push(mk(f));
        }

        wizard_selection_step(
            size_class,
            content_width,
            self.t("root_type_title").to_string(),
            cards.into(),
            Some((self.t("root_type_title").to_string(), vec![])),
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

        let mut cards = column![].spacing(2.0).width(Length::Fill);
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
                vec![],
            )),
        )
    }

    pub(crate) fn root_file_step(&self, subtitle: &str) -> Element<'_, Message> {
        let accepted: &[&str] = if self.root.is_gki() {
            &["zip", "img"]
        } else {
            &["apk"]
        };
        scrollable(
            column![
                self.wizard_picker_row(
                    self.root.file_path.as_deref(),
                    PickerPathKind::File,
                    Some(Message::Root(RootMsg::RootSelectFile)),
                    None
                ),
                text(subtitle.to_string())
                    .size(theme::text_size::BODY_SMALL)
                    .style(muted_style),
                self.recent_file_chips(
                    accepted,
                    |p| Message::RecentFilePicked(PickerTarget::RootFile, p),
                    "picker_recents"
                ),
            ]
            .spacing(6)
            .padding(28)
            .width(Length::Fill),
        )
        .height(Length::Fill)
        .into()
    }

    pub(crate) fn root_folder_step(&self) -> Element<'_, Message> {
        self.loader_picker_card(
            &self.root.folder_path,
            None,
            Message::Root(RootMsg::RootSelectFolder),
            |p| Message::Root(RootMsg::RootLoaderChosen(Some(p))),
        )
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
        let cards = column![lkm_card, gki_card].spacing(2.0).width(Length::Fill);
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
                vec![],
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

        let cards = column![lite, pro].spacing(2.0).width(Length::Fill);
        wizard_selection_step(
            size_class,
            content_width,
            self.t("root_skroot_flavor_title").to_string(),
            cards.into(),
            Some((self.t("root_skroot_flavor_title").to_string(), vec![])),
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
            column![mk(VerChoice::Nightly)].spacing(2.0)
        } else {
            column![mk(VerChoice::Stable), mk(VerChoice::Nightly)].spacing(2.0)
        };

        wizard_selection_step(
            size_class,
            content_width,
            self.t("root_version_title").to_string(),
            cards.width(Length::Fill).into(),
            Some((self.t("root_version_title").to_string(), vec![])),
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
            column![
                mk(NightlySource::AutoDetect),
                mk(NightlySource::ManualInput)
            ]
            .spacing(2.0),
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
            Some((self.t("root_source_title").to_string(), vec![])),
        )
    }

    pub(crate) fn root_confirm_step(&self) -> Element<'_, Message> {
        let dash = "—".to_string();
        let fam = self
            .root
            .family
            .map(|f| self.t(f.label_key()).to_string())
            .unwrap_or_else(|| dash.clone());

        let mut grid_rows = vec![confirm_definition_row(self.t("root_step_type"), &fam)];
        let mut trailing_rows = Vec::new();

        if self.root.is_skroot() {
            let flavor = self
                .root
                .skroot_flavor
                .map(|f| self.t(f.label_key()).to_string())
                .unwrap_or_else(|| dash.clone());
            grid_rows.push(confirm_definition_row(
                self.t("root_step_skroot_flavor"),
                &flavor,
            ));
        } else {
            let mode = self
                .root
                .mode
                .map(|m| self.t(m.label_key()).to_string())
                .unwrap_or_else(|| dash.clone());
            grid_rows.push(confirm_definition_row(self.t("root_step_mode"), &mode));
        }

        if self.root.is_gki() {
            let path = self.root.file_path.clone().unwrap_or_else(|| dash.clone());
            trailing_rows.push(confirm_path_row(self.t("root_step_kernel"), &path));
        } else if self.root.is_forks() {
            let path = self.root.file_path.clone().unwrap_or_else(|| dash.clone());
            grid_rows.push(confirm_definition_row(
                self.t("root_step_provider"),
                self.t("provider_magisk_forks"),
            ));
            trailing_rows.push(confirm_path_row(self.t("root_step_apk"), &path));
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
            grid_rows.push(confirm_definition_row(self.t("root_step_provider"), &prov));
            grid_rows.push(confirm_definition_row(self.t("root_step_version"), &ver));
            if self.root.is_nightly() {
                let src = self
                    .root
                    .nightly_source
                    .map(|s| self.t(s.label_key()).to_string())
                    .unwrap_or_else(|| dash.clone());
                grid_rows.push(confirm_definition_row(self.t("root_step_source"), &src));
                if self.root.nightly_source == Some(NightlySource::ManualInput) {
                    let id = self.root.run_id.clone().unwrap_or_else(|| dash.clone());
                    grid_rows.push(confirm_definition_row(self.t("nightly_run_id_label"), &id));
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
            grid_rows.push(confirm_definition_row(
                self.t("root_step_kpm"),
                &kpm_summary,
            ));
        }

        let folder = self
            .root
            .folder_path
            .clone()
            .unwrap_or_else(|| dash.clone());
        trailing_rows.push(confirm_path_row(self.t("edl_loader_label"), &folder));

        grid_rows.extend(trailing_rows);
        self.confirm_step_frame(vec![], grid_rows, vec![])
    }

    pub(crate) fn root_flash_step(&self) -> Element<'_, Message> {
        self.exec_step_view()
    }
}
