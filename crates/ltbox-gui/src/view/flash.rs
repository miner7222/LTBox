//! Flash wizard view + steps (region, target, data, folder, confirm, exec). Extracted from `main.rs`.

use super::components::{elide_path_middle, picker_action_button, picker_path_field};
use crate::*;
use iced::widget::{Space, button, column, container, row, scrollable, text};
use iced::{Element, Length, Theme};
use ltbox_core::tr_args;
use theme::with_alpha;

const FLASH_CONFIRM_LABEL_WIDTH: f32 = 180.0;
const FLASH_CONFIRM_MAX_WIDTH: f32 = 820.0;

/// One interactive row in the flash review definition list. All values stay
/// left-aligned; destructive outcomes use the error role without turning the
/// entire editable row into an error surface.
fn flash_confirm_definition_row(
    label: String,
    value: String,
    hint: Option<String>,
    destructive: bool,
    changed: bool,
    caution: Option<String>,
    on_open: Message,
) -> Element<'static, Message> {
    let mut value_column = column![
        text(value)
            .size(
                if matches!(on_open, Message::Flash(FlashMsg::FlashSelectFolder)) {
                    theme::text_size::BODY_SMALL
                } else {
                    theme::text_size::BODY_MEDIUM
                }
            )
            .font(
                if matches!(on_open, Message::Flash(FlashMsg::FlashSelectFolder)) {
                    theme::mono_font()
                } else {
                    theme::emphasis::medium()
                }
            )
            .width(Length::Fill)
            .wrapping(
                if matches!(on_open, Message::Flash(FlashMsg::FlashSelectFolder)) {
                    iced::widget::text::Wrapping::None
                } else {
                    iced::widget::text::Wrapping::WordOrGlyph
                }
            )
            .style(move |t: &Theme| iced::widget::text::Style {
                color: Some(if destructive {
                    pal_of(t).error
                } else {
                    pal_of(t).on_surface
                }),
            })
    ]
    .spacing(2)
    .width(Length::Fill)
    .align_x(iced::Alignment::Start);
    if let Some(hint) = hint {
        value_column = value_column.push(
            text(hint)
                .size(theme::text_size::LABEL_SMALL)
                .style(muted_style)
                .width(Length::Fill)
                .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
        );
    }

    let content = row![
        container(
            text(label)
                .size(theme::text_size::BODY_SMALL)
                .style(muted_style)
                .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
        )
        .width(Length::Fixed(FLASH_CONFIRM_LABEL_WIDTH)),
        value_column,
    ]
    .spacing(16)
    .align_y(iced::Alignment::Start)
    .width(Length::Fill);

    let action = button(content)
        .on_press(on_open)
        .padding([9, 0])
        .width(Length::Fill)
        .style(move |t: &Theme, status| {
            let p = pal_of(t);
            let background = if changed {
                Some(with_alpha(p.primary, 0.12 + theme::state_alpha(status)))
            } else {
                theme::state_layer_bg(status, p.on_surface)
            };
            button::Style {
                background: background.map(Into::into),
                text_color: p.on_surface,
                border: iced::Border {
                    color: if changed {
                        p.primary
                    } else {
                        iced::Color::TRANSPARENT
                    },
                    width: if changed { 1.0 } else { 0.0 },
                    radius: theme::shape::SM.into(),
                },
                ..Default::default()
            }
        });

    let action: Element<'static, Message> = if changed {
        if let Some(caution) = caution {
            iced::widget::tooltip(
                action,
                container(
                    text(caution)
                        .size(theme::text_size::BODY_SMALL)
                        .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
                )
                .padding([6, 10])
                .max_width(360)
                .style(|t: &Theme| theme::tooltip_style(t, theme::shape::SM)),
                iced::widget::tooltip::Position::Top,
            )
            .into()
        } else {
            action.into()
        }
    } else {
        action.into()
    };

    column![
        action,
        iced::widget::rule::horizontal(1).style(shell_rule_style)
    ]
    .spacing(0)
    .width(Length::Fill)
    .into()
}

impl App {
    pub(crate) fn view_flash_wizard(&self) -> Element<'_, Message> {
        let current_step = self.flash.current_step();
        let step_labels: Vec<&str> = self
            .flash
            .visible_steps()
            .iter()
            .map(|step| self.t(step.label_key()))
            .collect();
        let step_bar = if current_step == FlashStep::Flash {
            empty_wizard_step_bar()
        } else {
            wizard_step_bar(&step_labels, self.flash.step, self.window_size_class())
        };
        let body = match current_step {
            FlashStep::Region => self.flash_region_step(),
            FlashStep::Target => self.flash_target_step(),
            FlashStep::Data => self.flash_data_step(),
            FlashStep::Folder => self.flash_folder_step(),
            FlashStep::Bootloader => self.flash_bootloader_step(),
            FlashStep::Confirm => self.flash_confirm_step(),
            FlashStep::Flash => self.flash_exec_step(),
        };
        let (step_title, app_bar_subtitle) = self.flash_step_copy();
        let is_selection_step = matches!(
            current_step,
            FlashStep::Region | FlashStep::Target | FlashStep::Data
        );
        let body = if current_step == FlashStep::Flash || is_selection_step {
            body
        } else if matches!(current_step, FlashStep::Folder | FlashStep::Bootloader) {
            self.wizard_picker_step(step_title, body)
        } else {
            wizard_step_body(step_title, body)
        };
        let nav = if current_step != FlashStep::Flash {
            let is_start = current_step == FlashStep::Confirm;
            let label_owned = if is_start {
                self.t("btn_start").to_string()
            } else {
                self.t("btn_next").to_string()
            };
            let can = self.flash.can_next()
                && (current_step != FlashStep::Region
                    || (self.queries.region_pending.is_none()
                        && self.flash_serial_prompt.is_none()))
                && !(self.operation.is_running() && is_start)
                && (!is_start || self.device_reachable());
            wizard_nav_generic(
                self.flash.step > 0,
                &label_owned,
                can,
                self.t("btn_back"),
                Message::Flash(FlashMsg::FlashBack),
                Message::Flash(FlashMsg::FlashNext),
            )
        } else {
            empty_wizard_nav()
        };
        column![
            wizard_action_bar(
                self.window_size_class(),
                self.t("nav_flash").to_string(),
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

    fn flash_step_copy(&self) -> (String, Option<String>) {
        let (title_key, subtitle_key) = match self.flash.current_step() {
            FlashStep::Region => ("flash_region_title", Some("flash_region_subtitle")),
            FlashStep::Target => ("flash_target_title", Some("flash_target_subtitle")),
            FlashStep::Data => ("flash_data_title", Some("flash_data_subtitle")),
            FlashStep::Folder => ("flash_folder_title", Some("flash_folder_desc")),
            FlashStep::Bootloader => (
                "flash_bootloader_title",
                Some(if self.flash.uses_gbl() {
                    "flash_bootloader_efisp_desc"
                } else {
                    "flash_bootloader_pick_subtitle"
                }),
            ),
            FlashStep::Confirm => ("flash_confirm_title", None),
            FlashStep::Flash => {
                let (title, _) = self.exec_status_copy();
                return (title, self.exec_app_bar_subtitle());
            }
        };
        (
            self.t(title_key).to_string(),
            subtitle_key.map(|key| self.t(key).to_string()),
        )
    }

    pub(crate) fn flash_region_step(&self) -> Element<'_, Message> {
        let size_class = self.window_size_class();
        let content_width = self.window_size.0
            - match size_class {
                WindowSizeClass::Compact => SIDEBAR_RAIL_WIDTH,
                WindowSizeClass::Expanded => SIDEBAR_EXPANDED_WIDTH,
            };
        let icon_size = self.wizard_list_icon(WIZARD_LIST_GLYPH_ICON_SIZE);
        let metrics = self.wizard_list_metrics(WIZARD_LIST_LABEL_SIZE, WIZARD_LIST_DESC_SIZE);
        let prc_icon = lucide_list_primary(icon::region_prc(), icon_size);
        let auto_card = wizard_list_option_card_recommended(
            lucide_list_primary(icon::nightly_auto(), icon_size),
            self.t("flash_region_auto"),
            self.t("flash_region_auto_pick_desc"),
            self.flash.region_selection == Some(FlashRegionSelection::Auto),
            Some(Message::Flash(FlashMsg::FlashRegionAuto)),
            metrics,
            (
                self.t("root_recommended_label"),
                self.t("flash_region_auto_recommended_tip"),
            ),
        );
        // TB322FC is a PRC-only SKU. Render ROW as a disabled card with
        // a grayed icon so the constraint is visible — silent skip
        // would confuse users who expect both options.
        let tb322fc = self.model_capabilities().prc_only;
        let unsupported_tb322fc = tr_args!("model_unsupported", model = self.device.model.as_str());
        let row_card: Element<'_, Message> = if tb322fc {
            wizard_list_option_card(
                lucide_list_disabled(icon::region_row(), icon_size),
                self.t("region_row"),
                &unsupported_tb322fc,
                false,
                None,
                metrics,
            )
        } else {
            wizard_list_option_card(
                lucide_list_primary(icon::region_row(), icon_size),
                self.t("region_row"),
                self.t("region_row_name"),
                self.flash.region_selection
                    == Some(FlashRegionSelection::Manual(DeviceRegion::Row)),
                Some(Message::Flash(FlashMsg::FlashRegion(DeviceRegion::Row))),
                metrics,
            )
        };
        let mut cards = column![
            auto_card,
            wizard_list_option_card(
                prc_icon,
                self.t("region_prc"),
                self.t("region_prc_name"),
                self.flash.region_selection
                    == Some(FlashRegionSelection::Manual(DeviceRegion::Prc)),
                Some(Message::Flash(FlashMsg::FlashRegion(DeviceRegion::Prc))),
                metrics,
            ),
            row_card,
        ]
        .spacing(2.0)
        .width(Length::Fill);
        if self.queries.region_pending.is_some() {
            cards = cards.push(
                container(
                    row![
                        material_circular_progress(MaterialProgressSize::Standard),
                        text(self.t("flash_region_detecting").to_string())
                            .size(theme::text_size::BODY_SMALL)
                            .style(muted_style),
                    ]
                    .spacing(12.0)
                    .align_y(iced::Alignment::Center),
                )
                .padding([8, 12]),
            );
        } else if self.flash.region_auto_unknown {
            // Keep the conditional result below all choices, like the data
            // step's wipe warning, so selecting Auto does not move the rows.
            cards = cards.push(
                self.message_banner(
                    BannerSeverity::Warning,
                    icon::banner_warning(),
                    self.t("flash_region_auto_unknown").to_string(),
                    text(self.t("flash_region_auto_unknown_body").to_string())
                        .size(theme::text_size::BODY_SMALL)
                        .style(warning_container_text_style)
                        .width(Length::Fill)
                        .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
                ),
            );
        }
        wizard_selection_step(
            size_class,
            content_width,
            self.t("flash_region_title").to_string(),
            cards.into(),
            Some((
                self.t("flash_region_help_title").to_string(),
                vec![
                    self.t("flash_region_help_body").to_string(),
                    self.t("flash_region_help_hardware").to_string(),
                ],
            )),
        )
    }

    pub(crate) fn flash_target_step(&self) -> Element<'_, Message> {
        let size_class = self.window_size_class();
        let content_width = self.window_size.0
            - match size_class {
                WindowSizeClass::Compact => SIDEBAR_RAIL_WIDTH,
                WindowSizeClass::Expanded => SIDEBAR_EXPANDED_WIDTH,
            };
        let icon_size = self.wizard_list_icon(WIZARD_LIST_GLYPH_ICON_SIZE);
        let metrics = self.wizard_list_metrics(WIZARD_LIST_LABEL_SIZE, WIZARD_LIST_DESC_SIZE);
        let device = lucide_list_primary(icon::tile_device(), icon_size);
        // TB322FC ships only in PRC, so cross-region (OtherRegion) is
        // never a valid target. Disable the card with a grayed icon to
        // keep the constraint visible on the picker.
        let tb322fc = self.model_capabilities().prc_only;
        let unsupported_tb322fc = tr_args!("model_unsupported", model = self.device.model.as_str());
        // Region-aware target descriptions spell out the hardware market and
        // the ROM being installed so users don't conflate the two (the most
        // common point of confusion in this wizard). device_region is chosen
        // in step 0, so it is Some here; the None arm is a defensive fallback.
        let (same_desc, other_desc) = match self.flash.device_region {
            Some(DeviceRegion::Prc) => ("flashtarget_same_desc_prc", "flashtarget_other_desc_prc"),
            Some(DeviceRegion::Row) => ("flashtarget_same_desc_row", "flashtarget_other_desc_row"),
            None => ("flashtarget_same_desc", "flashtarget_other_desc"),
        };
        let other_card: Element<'_, Message> = if tb322fc {
            wizard_list_option_card(
                lucide_list_disabled(icon::tile_globe(), icon_size),
                self.t(FlashTarget::OtherRegion.label_key()),
                &unsupported_tb322fc,
                false,
                None,
                metrics,
            )
        } else {
            wizard_list_option_card(
                lucide_list_primary(icon::tile_globe(), icon_size),
                self.t(FlashTarget::OtherRegion.label_key()),
                self.t(other_desc),
                self.flash.target == Some(FlashTarget::OtherRegion),
                Some(Message::Flash(FlashMsg::FlashTarget(
                    FlashTarget::OtherRegion,
                ))),
                metrics,
            )
        };
        let cards = column![
            other_card,
            wizard_list_option_card(
                device,
                self.t(FlashTarget::SameRegion.label_key()),
                self.t(same_desc),
                self.flash.target == Some(FlashTarget::SameRegion),
                Some(Message::Flash(FlashMsg::FlashTarget(
                    FlashTarget::SameRegion
                ))),
                metrics,
            ),
        ]
        .spacing(2.0)
        .width(Length::Fill);
        wizard_selection_step(
            size_class,
            content_width,
            self.t("flash_target_title").to_string(),
            cards.into(),
            Some((String::new(), vec![])),
        )
    }

    pub(crate) fn flash_data_step(&self) -> Element<'_, Message> {
        let size_class = self.window_size_class();
        let content_width = self.window_size.0
            - match size_class {
                WindowSizeClass::Compact => SIDEBAR_RAIL_WIDTH,
                WindowSizeClass::Expanded => SIDEBAR_EXPANDED_WIDTH,
            };
        let icon_size = self.wizard_list_icon(WIZARD_LIST_GLYPH_ICON_SIZE);
        let metrics = self.wizard_list_metrics(WIZARD_LIST_LABEL_SIZE, WIZARD_LIST_DESC_SIZE);
        let shield = lucide_list_primary(icon::tile_shield(), icon_size);
        // Erasing `metadata` + `userdata` is the one irreversible choice
        // on this step, so it carries the error role rather than looking
        // like the sibling it is not.
        let wipe = lucide_error(icon::tile_wipe(), icon_size);
        let mut cards = column![].spacing(2.0).width(Length::Fill);
        cards = cards
            .push(wizard_list_option_card(
                shield,
                self.t(DataMode::Keep.label_key()),
                self.t("datamode_keep_desc"),
                self.flash.data_mode == Some(DataMode::Keep),
                Some(Message::Flash(FlashMsg::FlashDataMode(DataMode::Keep))),
                metrics,
            ))
            .push(wizard_list_option_card_destructive(
                wipe,
                self.t(DataMode::Wipe.label_key()),
                self.t("datamode_wipe_desc"),
                self.flash.data_mode == Some(DataMode::Wipe),
                Some(Message::Flash(FlashMsg::FlashDataMode(DataMode::Wipe))),
                metrics,
            ));
        // Below the options, not above: this banner appears only when erase
        // is chosen, and placing it first shoved both rows down on selection.
        if self.flash.data_mode == Some(DataMode::Wipe) {
            cards = cards.push(
                self.message_banner(
                    BannerSeverity::Error,
                    icon::banner_error(),
                    self.t("flash_data_wipe_warning_title").to_string(),
                    text(self.t("flash_data_wipe_warning_body").to_string())
                        .size(theme::text_size::BODY_SMALL)
                        .style(error_container_text_style)
                        .width(Length::Fill)
                        .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
                ),
            );
        }
        wizard_selection_step(
            size_class,
            content_width,
            self.t("flash_data_title").to_string(),
            cards.into(),
            Some((String::new(), vec![])),
        )
    }

    pub(crate) fn flash_folder_step(&self) -> Element<'_, Message> {
        let selected_path = self.flash.firmware_folder.as_deref();
        let path_row = self.wizard_picker_row(
            selected_path,
            PickerPathKind::Folder,
            Some(Message::Flash(FlashMsg::FlashSelectFolder)),
            selected_path.map(|_| Message::Flash(FlashMsg::FlashClearFolder)),
        );

        let mut content = column![path_row,]
            .spacing(6)
            .padding(iced::Padding {
                top: 18.0,
                right: 28.0,
                bottom: 28.0,
                left: 28.0,
            })
            .width(Length::Fill)
            .align_x(iced::Alignment::Start);

        // The picked firmware folder ships no EDL loader — keep the warning
        // and its corrective action together as one shared message banner.
        if selected_path.is_some() && self.flash.loader_required {
            let loader_path_row = row![
                picker_path_field(
                    self.flash.loader_override.as_deref(),
                    self.t("picker_no_file_selected").to_string(),
                    true,
                    self.picker_text_width(1),
                ),
                picker_action_button(
                    self.t("btn_pick").to_string(),
                    Some(Message::Flash(FlashMsg::FlashSelectLoader)),
                    true,
                ),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center)
            .width(Length::Fill);
            let mut banner_body = column![
                text(self.t("flash_loader_missing_body").to_string())
                    .size(theme::text_size::BODY_SMALL)
                    .style(warning_container_text_style)
                    .width(Length::Fill)
                    .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
                loader_path_row,
            ]
            .spacing(10)
            .width(Length::Fill);
            if let Some(error) = &self.flash.loader_error {
                banner_body = banner_body.push(
                    text(error.clone())
                        .size(theme::text_size::LABEL_SMALL)
                        .width(Length::Fill)
                        .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
                );
            }
            content =
                content
                    .push(Space::new().height(Length::Fixed(16.0)))
                    .push(self.message_banner(
                        BannerSeverity::Warning,
                        icon::banner_warning(),
                        self.t("flash_loader_missing_title").to_string(),
                        banner_body,
                    ));
        }

        content = content.push(
            self.recent_chips(
                self.recent_paths
                    .recent(PickerTarget::FlashFolder.kind().storage_key()),
                |path| Message::RecentFolderPicked(PickerTarget::FlashFolder, path),
                "picker_recents",
                false,
            ),
        );

        scrollable(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(m3_scrollable_style)
            .into()
    }

    pub(crate) fn flash_bootloader_step(&self) -> Element<'_, Message> {
        let selected = self.flash.user_abl_path.is_some();
        let analyzing = self.flash.user_abl_analyzing;
        let picker_row = self.wizard_picker_row(
            self.flash.user_abl_path.as_deref(),
            PickerPathKind::File,
            (!analyzing).then_some(Message::Flash(FlashMsg::FlashSelectBootloader)),
            selected.then_some(Message::Flash(FlashMsg::FlashClearBootloader)),
        );

        let verdict_key = if !selected {
            "flash_bootloader_empty"
        } else if analyzing {
            "flash_bootloader_analyzing"
        } else if self.flash.uses_gbl() {
            match self.flash.user_abl_efisp_load {
                ltbox_patch::efisp_load::EfispLoad::Yes => "common_yes",
                ltbox_patch::efisp_load::EfispLoad::No => "err_abl_efisp_not_loaded",
                ltbox_patch::efisp_load::EfispLoad::Undetermined => "err_abl_efisp_undetermined",
            }
        } else {
            match self.flash.user_abl_key_class {
                Some(ltbox_patch::key_map::KeyClass::Testkey) => "flash_key_testkey",
                Some(ltbox_patch::key_map::KeyClass::Lenovo) => "flash_key_lenovo",
                Some(ltbox_patch::key_map::KeyClass::Unknown) | None => "flash_key_unknown",
            }
        };
        let valid = selected && self.flash.bootloader_can_next();
        let verdict_text = if self.flash.uses_gbl() && selected && !analyzing && valid {
            format!("{}: {}", self.t("efisp_load_label"), self.t(verdict_key))
        } else {
            self.t(verdict_key).to_string()
        };
        let verdict = text(verdict_text)
            .size(theme::text_size::BODY_MEDIUM)
            .style(move |theme: &Theme| iced::widget::text::Style {
                color: Some(if valid {
                    pal_of(theme).success
                } else if selected && !analyzing {
                    pal_of(theme).error
                } else {
                    pal_of(theme).outline
                }),
            });

        let mut content = column![
            picker_row,
            verdict,
            self.recent_file_chips(
                &["elf"],
                |path| Message::Flash(FlashMsg::FlashBootloaderChosen(Some(path))),
                "picker_recents"
            )
        ]
        .spacing(10.0)
        .width(Length::Fill)
        .align_x(iced::Alignment::Start);
        if self.flash.uses_gbl() && !selected && !self.flash.bootloader_can_next() {
            content = content.push(
                text(self.t("err_abl_efisp_undetermined").to_string())
                    .size(13.0)
                    .center(),
            );
        }

        container(content)
            .padding(28.0)
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .align_y(iced::alignment::Vertical::Top)
            .into()
    }

    pub(crate) fn flash_confirm_step(&self) -> Element<'_, Message> {
        let dash = "—".to_string();
        // `wf_config` is the worker's only input, so the summary derives every
        // editable row from it (not the wizard cards). The values match the
        // card selections in the normal flow, so the rendered rows are
        // unchanged until a confirm-step override diverges from the baseline.
        let cfg = &self.wf_config;
        let base = self.confirm_baseline.as_ref();
        let caution = self.t("flash_confirm_override_warning").to_string();
        let open = |f: ConfirmField| Message::Flash(FlashMsg::FlashConfirmOpen(f));

        let device_name = if self.device.market_name.is_empty() {
            dash.as_str()
        } else {
            self.device.market_name.as_str()
        };
        let device_model = if self.device.model.is_empty() {
            dash.as_str()
        } else {
            self.device.model.as_str()
        };
        let device_value = format!("{device_name} · {device_model}");
        let device_row = confirm_definition_row(self.t("dash_device"), &device_value);

        let region = cfg
            .device_region
            .map(|r| self.t(r.label_key()).to_string())
            .unwrap_or_else(|| dash.clone());
        let region_changed = base.is_some_and(|b| b.device_region != cfg.device_region);

        // Target ↔ Region-edit both reflect `modify_region`, so they always
        // agree and highlight together.
        let target_kind = if cfg.modify_region {
            FlashTarget::OtherRegion
        } else {
            FlashTarget::SameRegion
        };
        let target = self.t(target_kind.label_key()).to_string();
        let modify_changed = base.is_some_and(|b| b.modify_region != cfg.modify_region);

        let data = self
            .t(if cfg.wipe {
                "flash_confirm_data_wipe"
            } else {
                "flash_confirm_data_keep"
            })
            .to_string();
        let data_changed = base.is_some_and(|b| b.wipe != cfg.wipe);

        // Confirm rows use short value labels (Modify / Auto / Ignore)
        // instead of the verbose "… rollback index" strings shown in
        // logs — the review summary is tighter to read that way.
        let modify_region = self
            .t(if cfg.modify_region {
                "flash_confirm_rb_on"
            } else {
                "flash_confirm_rb_off"
            })
            .to_string();
        let rollback = self
            .t(match cfg.modify_rollback {
                RollbackSetting::On => "flash_confirm_rb_on",
                RollbackSetting::Auto => "flash_confirm_rb_auto",
                RollbackSetting::Manual => "flash_confirm_rb_manual",
                RollbackSetting::Off => "flash_confirm_rb_off",
            })
            .to_string();
        let rollback_changed = base.is_some_and(|b| b.modify_rollback != cfg.modify_rollback);

        let rollback_caution = if cfg.modify_rollback == RollbackSetting::Manual
            && self.manual_rollback_downgrade_warning().is_some()
        {
            self.t("flash_confirm_rb_manual_downgrade").to_string()
        } else {
            caution.clone()
        };

        // The operation can destroy data or leave a kept-data downgrade
        // unbootable, so this is an error-role safety banner rather than an
        // amber inline note. The icon is separate from localized copy.
        let (warning_title_key, warning_key) = if cfg.wipe {
            (
                "flash_data_wipe_warning_title",
                "flash_confirm_warning_wipe",
            )
        } else {
            ("flash_confirm_warning_title", "flash_confirm_warning")
        };
        let warning = self.message_banner(
            BannerSeverity::Error,
            icon::banner_error(),
            self.t(warning_title_key).to_string(),
            text(self.t(warning_key).to_string())
                .size(theme::text_size::BODY_SMALL)
                .style(error_container_text_style)
                .width(Length::Fill)
                .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
        );

        let region_row = flash_confirm_definition_row(
            self.t("flash_confirm_region").to_string(),
            region,
            None,
            false,
            region_changed,
            Some(caution.clone()),
            open(ConfirmField::Region),
        );
        let target_row = flash_confirm_definition_row(
            self.t("flash_confirm_target").to_string(),
            target,
            None,
            false,
            modify_changed,
            Some(caution.clone()),
            open(ConfirmField::Target),
        );
        let data_row = flash_confirm_definition_row(
            self.t("flash_confirm_data").to_string(),
            data,
            None,
            cfg.wipe,
            data_changed,
            Some(caution.clone()),
            open(ConfirmField::Data),
        );
        let region_edit_row = flash_confirm_definition_row(
            self.t("flash_confirm_region_edit").to_string(),
            modify_region,
            None,
            false,
            modify_changed,
            Some(caution.clone()),
            open(ConfirmField::RegionEdit),
        );
        let rollback_hint_key = if ["TB520FU", "TB321FU"].contains(&self.device.model.as_str()) {
            "flash_confirm_rb_fastboot_hint"
        } else {
            "flash_confirm_rb_edl_hint"
        };
        let rollback_hint = (cfg.modify_rollback == RollbackSetting::Auto)
            .then(|| self.t(rollback_hint_key).to_string());
        let rollback_row = flash_confirm_definition_row(
            self.t("flash_confirm_rollback").to_string(),
            rollback,
            rollback_hint,
            cfg.modify_rollback != RollbackSetting::Off,
            rollback_changed,
            Some(rollback_caution),
            // Always the setting list, Manual included: the row is how the user
            // gets back to On/Auto/Off, and picking Manual there opens the
            // editor anyway.
            open(ConfirmField::Rollback),
        );

        let country_changed = base.is_some_and(|b| b.country_action != cfg.country_action);
        let country_label = if let Some(cc) = cfg.country_action.target() {
            COUNTRY_CODES
                .iter()
                .find(|e| e.code == cc)
                .map(|e| format!("{} — {}", e.code, e.name))
                .unwrap_or_else(|| cc.to_string())
        } else {
            self.t("flash_confirm_country_skip").to_string()
        };
        let country_row = flash_confirm_definition_row(
            self.t("flash_confirm_country").to_string(),
            country_label,
            None,
            false,
            country_changed,
            Some(caution),
            open(ConfirmField::Country),
        );
        let folder_owned = self
            .flash
            .firmware_folder
            .clone()
            .unwrap_or_else(|| dash.clone());
        let folder_row = flash_confirm_definition_row(
            self.t("flash_confirm_folder").to_string(),
            elide_path_middle(
                &folder_owned,
                ((self.window_size.0
                    - if self.window_size_class() == WindowSizeClass::Expanded {
                        SIDEBAR_EXPANDED_WIDTH
                    } else {
                        SIDEBAR_RAIL_WIDTH
                    })
                .min(FLASH_CONFIRM_MAX_WIDTH)
                    - 56.0
                    - FLASH_CONFIRM_LABEL_WIDTH
                    - 16.0)
                    .max(60.0),
            ),
            None,
            false,
            false,
            None,
            Message::Flash(FlashMsg::FlashSelectFolder),
        );

        let definitions = column![
            device_row,
            region_row,
            target_row,
            data_row,
            region_edit_row,
            rollback_row,
            country_row,
            folder_row,
        ]
        .spacing(0)
        .width(Length::Fill)
        .align_x(iced::Alignment::Start);
        let content = column![warning, definitions]
            .spacing(16.0)
            .padding([18.0, 28.0])
            .width(Length::Fill)
            .align_x(iced::Alignment::Start);
        let summary = iced::widget::scrollable(content)
            .style(m3_scrollable_style)
            .height(Length::Shrink)
            .width(Length::Fill);

        container(
            container(summary)
                .width(Length::Fill)
                .max_width(FLASH_CONFIRM_MAX_WIDTH),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .align_y(iced::alignment::Vertical::Top)
        .into()
    }

    pub(crate) fn flash_exec_step(&self) -> Element<'_, Message> {
        self.exec_step_view_with_inline_log()
    }
}
