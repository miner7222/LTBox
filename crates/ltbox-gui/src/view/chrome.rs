//! App shell: root view dispatcher, titlebar, sidebar, content frame, banners, toast, dialogs. Extracted from `main.rs`.

use crate::focus_button::{self as button, button};
use crate::*;
use iced::widget::{self, Space, column, container, row, scrollable, text};
use iced::{Element, Length, Theme};

/// M3 single-line snackbar height.
const TOAST_HEIGHT: f32 = 48.0;

impl App {
    pub(crate) fn view(&self) -> Element<'_, Message> {
        self.sync_runtime_theme();
        let mut main = column![];
        if !crate::SYSTEM_WINDOW_CHROME {
            // Custom borderless title bar (Windows / Linux). macOS uses the
            // native system title bar, so skip the in-app one.
            main = main.push(self.title_bar());
            main = main.push(widget::rule::horizontal(1).style(shell_rule_style));
        }
        let row_area: Element<'_, Message> = match self.window_size_class() {
            WindowSizeClass::Expanded => row![self.sidebar(), self.content()]
                .width(Length::Fill)
                .height(Length::Fill)
                .into(),
            WindowSizeClass::Compact => {
                // Compact keeps a fixed icon-rail placeholder while the
                // hovered drawer floats over content, avoiding reflow during
                // the width spring.
                let rail_placeholder = container(iced::widget::Space::new())
                    .width(Length::Fixed(SIDEBAR_RAIL_WIDTH))
                    .height(Length::Fill);
                let row_base = row![rail_placeholder, self.content()].height(Length::Fill);
                iced::widget::Stack::with_children(vec![row_base.into(), self.sidebar()])
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into()
            }
        };
        main = main.push(row_area);
        main = main.push(self.status_bar());

        // Custom chrome draws a 1-px outline + 1-px inset so children don't
        // overpaint the borderless window's edge. With native macOS decorations
        // the system frame bounds the window, so skip the inset/outline.
        let framed: Element<'_, Message> = if crate::SYSTEM_WINDOW_CHROME {
            container(main)
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else {
            container(main)
                .width(Length::Fill)
                .height(Length::Fill)
                .padding(1)
                .style(|t: &Theme| container::Style {
                    border: iced::Border {
                        color: pal_of(t).outline_variant,
                        width: 1.0,
                        radius: 0.0.into(),
                    },
                    ..Default::default()
                })
                .into()
        };

        // Error banner below popups so the scrim dims the banner too.
        let mut layers: Vec<Element<'_, Message>> = vec![framed];

        if self.should_show_error_banner()
            && let Some(err) = &self.error_msg
        {
            layers.push(self.error_banner(err));
        }
        let dialog_layer_start = layers.len();
        let mut modeless_layers = Vec::new();
        if self.software_fix.confirm_open {
            layers.push(self.software_fix_confirm_dialog());
        }
        if self.country_popup_open {
            layers.push(self.country_popup_view());
        }
        if self.region_target_popup_open {
            layers.push(self.region_target_popup_view());
        }
        if self.konabess.target_popup_open {
            layers.push(self.konabess_target_popup_view());
        }
        if let Some(field) = self.confirm_edit_field {
            layers.push(self.flash_confirm_edit_popup(field));
        }
        if self.flash.firmware_identity_dialog.is_some() {
            layers.push(self.flash_firmware_identity_popup());
        }
        if self.manual_rollback_editor.is_some() {
            layers.push(self.manual_rollback_popup_view());
        }
        if let Some(t) = self.reboot_confirm_target {
            layers.push(self.reboot_confirm_popup(t));
        }
        if self.reboot_wait_transition
            && self.reboot_wait_dialog_open
            && self.operation.is_running()
            && self.operation.view() == Some(View::Reboot)
        {
            modeless_layers.push(layers.len());
            layers.push(self.reboot_wait_popup());
        }
        if self.root.run_id_popup_open {
            layers.push(self.root_run_id_popup());
        }
        if self.root.release_popup_open {
            layers.push(self.root_release_popup());
        }
        if self.root.kernel_version_popup_open {
            layers.push(self.root_kernel_version_popup());
        }
        if self.root.superkey_popup_open {
            layers.push(self.root_superkey_popup());
        }
        if self.should_show_busy_progress_dialog() {
            modeless_layers.push(layers.len());
            layers.push(self.busy_progress_dialog());
        }
        if self.device_info_popup.is_some() {
            layers.push(self.device_info_popup_view());
        }
        if self.unroot.backup_manifest_dialog.is_some() {
            layers.push(self.unroot_backup_manifest_popup());
        }
        if self.ota_popup.is_some() {
            layers.push(self.ota_popup_view());
        }
        if self.qfil_popup.is_some() {
            layers.push(self.qfil_popup_view());
        }
        if self.flash_serial_prompt.is_some() {
            layers.push(self.flash_serial_prompt_view());
        }
        if self.rollback_popup_open {
            layers.push(self.rollback_detail_popup_view());
        }
        if self.arb_index_popup_open {
            layers.push(self.arb_index_popup_view());
        }
        if self.update_dialog_source.is_some() {
            layers.push(self.update_dialog_view());
        }
        if self.about_licenses_open {
            layers.push(self.about_licenses_dialog());
        }
        if self.dual_usb_help_open {
            layers.push(self.dual_usb_help_dialog());
        }
        let toast_layer_count = usize::from(self.toast_msg.is_some());
        if self.toast_msg.is_some() {
            layers.push(self.toast_view());
        }
        if self.startup_disclaimer_open {
            layers.push(self.startup_disclaimer_dialog());
        }
        let any_dialog_open = layers.len() > dialog_layer_start + toast_layer_count;
        let top_dialog = if any_dialog_open {
            Some(if self.startup_disclaimer_open {
                layers.len() - 1
            } else {
                layers.len() - 1 - toast_layer_count
            })
        } else {
            None
        }
        .filter(|index| !modeless_layers.contains(index));
        // Keep the wrapper mounted so opening a dialog preserves underlying
        // scroll positions and text-input state while removing keyboard access.
        for (index, layer) in layers.iter_mut().enumerate() {
            let previous = std::mem::replace(layer, Space::new().into());
            *layer = focus_button::scope(previous, top_dialog.is_none_or(|top| index == top));
        }

        // Resize handles last so the edge/corner hit areas at the window
        // edges and corners sit above every popup/toast — the user can
        // still grab the border while a dialog is open. Events outside
        // each handle's bounding box pass through to the layers below
        // so normal UI clicks aren't intercepted. macOS uses native resize
        // edges, so the overlay handles are omitted there.
        if !crate::SYSTEM_WINDOW_CHROME && !self.window_maximized {
            layers.push(self.resize_handles());
        }

        // Keep custom-chrome hit areas above every scrim. This layer is only
        // as tall as the title bar, so the Stack passes events below it to the
        // dialog or shell underneath.
        if !crate::SYSTEM_WINDOW_CHROME && any_dialog_open {
            layers.push(self.title_bar_overlay());
        }

        iced::widget::Stack::with_children(layers).into()
    }

    pub(crate) fn startup_disclaimer_dialog(&self) -> Element<'_, Message> {
        let acknowledgement = focus_button::actionable(
            widget::checkbox(self.startup_disclaimer_checked)
                .label(self.t("startup_disclaimer_accept").to_string())
                .on_toggle(Message::StartupDisclaimerToggled)
                .size(20.0)
                .spacing(12.0)
                .text_size(theme::text_size::BODY_MEDIUM)
                .style(m3_checkbox_style),
            Some(Message::StartupDisclaimerToggled(
                !self.startup_disclaimer_checked,
            )),
        );

        let mut continue_button =
            m3_filled_button(self.t("startup_disclaimer_continue").to_string());
        if self.startup_disclaimer_checked {
            continue_button = continue_button.on_press(Message::StartupDisclaimerConfirm);
        }

        let header: Element<'_, Message> = text(self.t("startup_disclaimer_title").to_string())
            .size(theme::text_size::TITLE_LARGE)
            .font(theme::emphasis::bold())
            .style(on_surface_style)
            .into();
        let body: Element<'_, Message> = column![
            text(self.t("startup_disclaimer_body").to_string())
                .size(theme::text_size::BODY_MEDIUM)
                .style(muted_style)
                .wrapping(iced::widget::text::Wrapping::WordOrGlyph)
                .width(Length::Fill),
            acknowledgement,
        ]
        .spacing(16)
        .into();
        let footer: Element<'_, Message> = row![
            Space::new().width(Length::Fill),
            m3_outlined_button(self.t("startup_disclaimer_exit").to_string())
                .on_press(Message::StartupDisclaimerExit),
            continue_button,
        ]
        .spacing(10)
        .align_y(iced::Alignment::Center)
        .into();

        m3_dialog(dialog_sections(
            header,
            body,
            footer,
            theme::DIALOG_WIDTH_SM,
            false,
        ))
    }

    pub(crate) fn title_bar(&self) -> Element<'_, Message> {
        self.title_bar_layer(false)
    }

    fn title_bar_overlay(&self) -> Element<'_, Message> {
        container(self.title_bar_layer(true))
            .padding(1)
            .width(Length::Fill)
            .into()
    }

    fn title_bar_layer(&self, hit_areas_only: bool) -> Element<'_, Message> {
        let title_content: Element<'_, Message> = if hit_areas_only {
            container(Space::new().height(16))
                .padding([8, 12])
                .width(Length::Fill)
                .into()
        } else {
            container(
                row![
                    iced::widget::image(TITLE_BAR_ICON_HANDLE.clone())
                        .width(16)
                        .height(16),
                    text("LTBox").size(12).style(muted_style),
                ]
                .spacing(6)
                .align_y(iced::Alignment::Center),
            )
            .padding([8, 12])
            .width(Length::Fill)
            .into()
        };

        let drag_area = iced::widget::mouse_area(title_content)
            .on_press(Message::Window(WindowMsg::WindowDrag))
            .on_double_click(Message::Window(WindowMsg::WindowToggleMaximize));

        let btn_w = 46;
        let btn_h = 32;

        let minimize_content: Element<'_, Message> =
            container(lucide_icon(icon::win_minimize(), 12.0, |t: &Theme| {
                pal_of(t).on_surface
            }))
            .width(btn_w)
            .height(btn_h)
            .center_x(btn_w)
            .center_y(btn_h)
            .into();
        let minimize_btn = button(minimize_content)
            .on_press(Message::Window(WindowMsg::WindowMinimize))
            .padding(0)
            .style(|t: &Theme, status| {
                // Palette-driven state layer. The old flat grey at 15% ignored
                // the theme entirely and did not distinguish hover from press.
                let p = pal_of(t);
                button::Style {
                    background: theme::state_layer_bg(status, p.on_surface).map(Into::into),
                    text_color: p.on_surface,
                    ..Default::default()
                }
            });

        let maximize_icon = if self.window_maximized {
            icon::win_restore()
        } else {
            icon::win_maximize()
        };
        let maximize_content: Element<'_, Message> =
            container(lucide_icon(maximize_icon, 12.0, |t: &Theme| {
                pal_of(t).on_surface
            }))
            .width(btn_w)
            .height(btn_h)
            .center_x(btn_w)
            .center_y(btn_h)
            .into();
        let maximize_btn = button(maximize_content)
            .on_press(Message::Window(WindowMsg::WindowToggleMaximize))
            .padding(0)
            .style(|t: &Theme, status| {
                // Palette-driven state layer. The old flat grey at 15% ignored
                // the theme entirely and did not distinguish hover from press.
                let p = pal_of(t);
                button::Style {
                    background: theme::state_layer_bg(status, p.on_surface).map(Into::into),
                    text_color: p.on_surface,
                    ..Default::default()
                }
            });

        let close_content: Element<'_, Message> =
            // No explicit glyph colour: the icon inherits the button's
            // `text_color`, so it flips to `on_error` the moment the red
            // wash lands instead of staying dark on red.
            container(icon::win_close().size(12))
                .width(btn_w)
                .height(btn_h)
                .center_x(btn_w)
                .center_y(btn_h)
                .into();
        let close_btn = button(close_content)
            .on_press(Message::Window(WindowMsg::WindowClose))
            .padding(0)
            .style(|t: &Theme, status| {
                // Close keeps its distinct red wash, but on the `error` role
                // rather than a hardcoded `rgb(0.9, 0.2, 0.2)` that stayed put
                // across every theme seed and both modes.
                let p = pal_of(t);
                let hot = matches!(status, button::Status::Hovered | button::Status::Pressed);
                button::Style {
                    background: hot.then(|| {
                        theme::mix_color(p.error, p.on_error, theme::state_alpha(status)).into()
                    }),
                    text_color: if hot { p.on_error } else { p.on_surface },
                    ..Default::default()
                }
            });

        container(
            row![drag_area, minimize_btn, maximize_btn, close_btn,]
                .align_y(iced::Alignment::Center)
                .height(btn_h),
        )
        .width(Length::Fill)
        .style(move |t: &Theme| {
            if hit_areas_only {
                container::Style::default()
            } else {
                container::Style {
                    background: Some(pal_of(t).surface_container_low.into()),
                    ..Default::default()
                }
            }
        })
        .into()
    }

    /// Invisible edge/corner handles for the borderless window.
    pub(crate) fn resize_handles(&self) -> Element<'_, Message> {
        const EDGE: f32 = 8.0;
        const CORNER: f32 = 14.0;

        // Build one positioned, transparent handle.
        // `dir`: which window edge / corner this handle resizes.
        // `w` / `h`: handle hit-area size.
        // `x` / `y`: alignment of the handle inside the Fill outer.
        // `interaction`: cursor to show on hover.
        let handle = |dir: iced::window::Direction,
                      w: Length,
                      h: Length,
                      x: iced::alignment::Horizontal,
                      y: iced::alignment::Vertical,
                      interaction: iced::mouse::Interaction|
         -> Element<'_, Message> {
            let hit = container(iced::widget::Space::new()).width(w).height(h);
            let area = iced::widget::mouse_area(hit)
                .on_press(Message::Window(WindowMsg::WindowResize(dir)))
                .interaction(interaction);
            container(area)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(x)
                .align_y(y)
                .into()
        };

        use iced::alignment::{Horizontal, Vertical};
        use iced::mouse::Interaction;
        use iced::window::Direction;
        let edges: Vec<Element<'_, Message>> = vec![
            // Edges first (lower z) so corners can overlap them.
            handle(
                Direction::North,
                Length::Fill,
                Length::Fixed(EDGE),
                Horizontal::Center,
                Vertical::Top,
                Interaction::ResizingVertically,
            ),
            handle(
                Direction::South,
                Length::Fill,
                Length::Fixed(EDGE),
                Horizontal::Center,
                Vertical::Bottom,
                Interaction::ResizingVertically,
            ),
            handle(
                Direction::West,
                Length::Fixed(EDGE),
                Length::Fill,
                Horizontal::Left,
                Vertical::Center,
                Interaction::ResizingHorizontally,
            ),
            handle(
                Direction::East,
                Length::Fixed(EDGE),
                Length::Fill,
                Horizontal::Right,
                Vertical::Center,
                Interaction::ResizingHorizontally,
            ),
            // Corners on top so the diagonal cursor + diagonal resize
            // win at the actual corner pixels.
            handle(
                Direction::NorthWest,
                Length::Fixed(CORNER),
                Length::Fixed(CORNER),
                Horizontal::Left,
                Vertical::Top,
                Interaction::ResizingDiagonallyDown,
            ),
            handle(
                Direction::NorthEast,
                Length::Fixed(CORNER),
                Length::Fixed(CORNER),
                Horizontal::Right,
                Vertical::Top,
                Interaction::ResizingDiagonallyUp,
            ),
            handle(
                Direction::SouthWest,
                Length::Fixed(CORNER),
                Length::Fixed(CORNER),
                Horizontal::Left,
                Vertical::Bottom,
                Interaction::ResizingDiagonallyUp,
            ),
            handle(
                Direction::SouthEast,
                Length::Fixed(CORNER),
                Length::Fixed(CORNER),
                Horizontal::Right,
                Vertical::Bottom,
                Interaction::ResizingDiagonallyDown,
            ),
        ];
        iced::widget::Stack::with_children(edges)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    pub(crate) fn sidebar(&self) -> Element<'_, Message> {
        // Opacity uses its own non-overshooting effects spring.
        // Expanded navigation keeps labels fully visible.
        let size_class = self.window_size_class();
        let label_alpha = self.sidebar_visual_progress();
        let collapsed = size_class == WindowSizeClass::Compact && !self.sidebar_expanded;
        // Center the 56px indicator inside the 80px rail.
        let list_padding = if collapsed { [12, 12] } else { [12, 8] };
        let mut col = column![]
            .spacing(1)
            .padding(list_padding)
            // Keep compact items anchored while the panel shrinks on exit.
            .align_x(iced::Alignment::Start);
        for &v in NAV_MAIN {
            col = col.push(nav_btn(
                v,
                self.t(v.sidebar_label_key()),
                self.current_view == v,
                self.is_nav_enabled(v),
                label_alpha,
                collapsed,
            ));
        }
        col = col.push(sec_hdr(self.t("nav_section_tools"), label_alpha, collapsed));
        for &v in NAV_TOOLS {
            col = col.push(nav_btn(
                v,
                self.t(v.sidebar_label_key()),
                self.current_view == v,
                self.is_nav_enabled(v),
                label_alpha,
                collapsed,
            ));
        }
        // About sits at the very bottom of the nav list.
        col = col.push(nav_btn(
            View::About,
            self.t(View::About.sidebar_label_key()),
            self.current_view == View::About,
            self.is_nav_enabled(View::About),
            label_alpha,
            collapsed,
        ));

        // M3: when the destination list is longer than the drawer, it scrolls
        // inside the drawer, including when keyboard focus reveals a destination.
        let body: Element<'_, Message> = widget::scrollable(col)
            .style(m3_scrollable_style)
            .width(Length::Fill)
            .height(Length::Fill)
            .into();

        let width = match size_class {
            WindowSizeClass::Compact => {
                SIDEBAR_RAIL_WIDTH
                    + (SIDEBAR_EXPANDED_WIDTH - SIDEBAR_RAIL_WIDTH) * self.sidebar_anim
            }
            WindowSizeClass::Expanded => SIDEBAR_EXPANDED_WIDTH,
        };
        let floats_over_content = size_class == WindowSizeClass::Compact && self.sidebar_anim > 0.0;
        let anim = self.sidebar_anim;
        let panel = container(body)
            .width(width)
            .height(Length::Fill)
            .style(panel_bg);
        let mut shell =
            row![panel, widget::rule::vertical(1).style(shell_rule_style)].height(Length::Fill);
        if floats_over_content {
            // Cast beside the panel rather than around it — see
            // `theme::drawer_edge_gradient`.
            shell = shell.push(
                container(Space::new())
                    .width(Length::Fixed(theme::DRAWER_EDGE_SHADOW_WIDTH * anim))
                    .height(Length::Fill)
                    .style(move |t: &Theme| container::Style {
                        background: Some(theme::drawer_edge_gradient(theme::is_dark(t), anim)),
                        ..Default::default()
                    }),
            );
        }
        match size_class {
            WindowSizeClass::Compact => {
                // Idle interaction prevents click-through to wizard cards
                // under the Stack (Stack levitates the cursor for lower
                // layers when top reports a non-None interaction).
                iced::widget::mouse_area(shell)
                    .on_enter(Message::SidebarHoverEnter)
                    .on_exit(Message::SidebarHoverExit)
                    .on_press(Message::Noop)
                    .interaction(iced::mouse::Interaction::Idle)
                    .into()
            }
            // Expanded participates in the row layout, so it needs neither
            // overlay click-through protection nor hover messages.
            WindowSizeClass::Expanded => shell.into(),
        }
    }

    pub(crate) fn content(&self) -> Element<'_, Message> {
        if self.current_view == View::Root {
            return self.view_root_wizard();
        }
        if self.current_view == View::Flash {
            return self.view_flash_wizard();
        }
        if self.current_view == View::SystemUpdate {
            return self.view_sysupdate_wizard();
        }
        if self.current_view == View::Unroot {
            return self.view_unroot_wizard();
        }
        if self.current_view == View::KonaBess {
            return self.view_konabess_wizard();
        }
        // Advanced owns its scroll/padding so both the landing screen and
        // wizards can use a full-width top app bar without being pinched by
        // the generic content wrapper.
        if self.current_view == View::Advanced {
            return self.view_advanced();
        }

        // Settings uses the same full-width top app bar pattern as Advanced.
        if self.current_view == View::Settings {
            return self.view_settings();
        }

        // Reboot cards need Fill height; scrollable would force Shrink
        // and collapse them.
        if self.current_view == View::Reboot {
            return self.view_reboot();
        }
        let inner = match self.current_view {
            View::Dashboard => self.view_dashboard(),
            View::Advanced => self.view_advanced(),
            View::Settings => self.view_settings(),
            View::About => self.view_about(),
            _ => self.view_placeholder(),
        };
        // Dashboard wants the log card to fill the leftover vertical space
        // so the inner top + bottom margins stay symmetric. A `scrollable`
        // gives its child unbounded height, which collapses every
        // `Length::Fill` inside the dashboard tree to zero — so the
        // dashboard skips the scrollable wrapper and lets its own
        // `column.height(Fill)` claim the bounded viewport directly.
        // Other views (Advanced, Settings, …) keep the scrollable wrapper
        // because their content can legitimately grow past the viewport.
        let body: Element<'_, Message> = if self.current_view == View::Dashboard {
            container(inner)
                .padding(24)
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else if self.current_view == View::About {
            // About (like Dashboard) centers via `Length::Fill`; a scrollable
            // would give its child unbounded height and collapse that to zero.
            container(inner)
                .padding(24)
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else {
            scrollable(container(inner).padding(24).width(Length::Fill))
                .style(m3_scrollable_style)
                .into()
        };
        container(body)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    pub(crate) fn error_banner(&self, msg: &str) -> Element<'_, Message> {
        // Floating overlay via `view()`'s stack so the layout below
        // doesn't shift. Inset card (not an edge-to-edge strip) so the
        // alert reads as a discrete surface above content.
        const INSET: f32 = 12.0;
        const DISMISS_TARGET: f32 = 40.0;

        let dismiss = button(
            container(lucide_icon(icon::win_close(), 16.0, |t: &Theme| {
                pal_of(t).on_error_container
            }))
            .width(DISMISS_TARGET)
            .height(DISMISS_TARGET)
            .center_x(DISMISS_TARGET)
            .center_y(DISMISS_TARGET),
        )
        .on_press(Message::DismissError)
        .padding(0)
        .style(|t: &Theme, status| {
            let p = pal_of(t);
            let a = theme::state_alpha(status);
            button::Style {
                background: if a > 0.0 {
                    Some(with_alpha(p.on_error_container, a).into())
                } else {
                    None
                },
                text_color: p.on_error_container,
                border: iced::Border {
                    radius: theme::shape::SM.into(),
                    ..Default::default()
                },
                ..Default::default()
            }
        });

        let body = text(msg.to_string())
            .size(theme::text_size::BODY_SMALL)
            .style(error_container_text_style)
            .width(Length::Fill);
        let card = self.message_banner_with_trailing(
            BannerSeverity::Error,
            icon::banner_error(),
            self.t("banner_error_title").to_string(),
            body,
            Some(dismiss.into()),
        );

        // Top/side inset + Fill-height spacer below keeps the overlay
        // non-layout-shifting and floating instead of edge-flush.
        column![
            container(card)
                .padding(iced::Padding {
                    top: INSET,
                    right: INSET,
                    bottom: 0.0,
                    left: INSET,
                })
                .width(Length::Fill),
            Space::new().width(Length::Fill).height(Length::Fill),
        ]
        .width(Length::Fill)
        .into()
    }

    pub(crate) fn status_bar(&self) -> Element<'_, Message> {
        let p = self.pal();
        let status_color = self.device.connection.color(&p);
        let status_label = self.t(self.connection_label_key());
        let model_text = if self.device.model.is_empty() {
            ""
        } else {
            &self.device.model
        };
        let mut connection = row![
            text(format!("●  {status_label}"))
                .size(12)
                .color(status_color),
        ]
        .spacing(6)
        .align_y(iced::Alignment::Center);
        if !model_text.is_empty() {
            connection = connection
                .push(text("-").size(12).style(muted_style))
                .push(text(model_text).size(12).style(muted_style));
        }
        let mut status_row = row![connection]
            .spacing(12)
            .align_y(iced::Alignment::Center);
        if self.operation.is_running() {
            status_row = status_row.push(
                text(self.t("status_working").to_string())
                    .size(12)
                    .style(accent_style),
            );
        }
        status_row = status_row.push(Space::new().width(Length::Fill));
        // Debug builds show "debug" instead of the version so a dev build is
        // never mistaken for a released one in screenshots/bug reports.
        let version = if cfg!(debug_assertions) {
            "debug"
        } else {
            concat!("v", env!("CARGO_PKG_VERSION"))
        };
        if self.update_available.is_some() {
            status_row = status_row.push(
                row![
                    self.status_update_available_button(),
                    text(version).size(12).style(muted_style),
                ]
                .spacing(6)
                .align_y(iced::Alignment::Center),
            );
        } else {
            // Preserve the original child and metrics byte-for-byte when no
            // update is available.
            status_row = status_row.push(text(version).size(12).style(muted_style));
        }
        // Top divider via an explicit `horizontal_rule` (1 px) so
        // the meeting point with the sidebar's right divider lands
        // as a single line per direction (M3 bottom-app-bar
        // guidance: one divider per shared edge).
        column![
            widget::rule::horizontal(1).style(shell_rule_style),
            container(status_row)
                .height(32)
                .padding([4, 20])
                .align_y(iced::Alignment::Center)
                .width(Length::Fill)
                .style(|t: &Theme| panel_bg(t)),
        ]
        .into()
    }

    /// Shared severity banner. Every warning and error uses this anatomy:
    /// semantic icon, medium-weight title, compact body, a full-strength
    /// role outline, and a dedicated 3 px leading accent.
    pub(crate) fn message_banner<'a>(
        &self,
        severity: BannerSeverity,
        icon_glyph: iced::widget::Text<'static, Theme, iced::Renderer>,
        title: impl Into<String>,
        body: impl Into<Element<'a, Message>>,
    ) -> Element<'a, Message> {
        self.message_banner_with_trailing(severity, icon_glyph, title, body, None)
    }

    fn message_banner_with_trailing<'a>(
        &self,
        severity: BannerSeverity,
        icon_glyph: iced::widget::Text<'static, Theme, iced::Renderer>,
        title: impl Into<String>,
        body: impl Into<Element<'a, Message>>,
        trailing: Option<Element<'a, Message>>,
    ) -> Element<'a, Message> {
        const ICON_SIZE: f32 = 20.0;
        const ACCENT_WIDTH: f32 = 3.0;
        const BORDER_WIDTH: f32 = 1.0;

        let foreground = move |t: &Theme| {
            let p = pal_of(t);
            match severity {
                BannerSeverity::Warning => p.on_warning_container,
                BannerSeverity::Error => p.on_error_container,
            }
        };
        let icon = container(lucide_icon(icon_glyph, ICON_SIZE, foreground))
            .width(ICON_SIZE)
            .height(ICON_SIZE)
            .align_x(iced::alignment::Horizontal::Center)
            .align_y(iced::alignment::Vertical::Center);
        let title = text(title.into())
            .size(theme::text_size::BODY_MEDIUM)
            .font(theme::emphasis::medium())
            .style(move |t: &Theme| iced::widget::text::Style {
                color: Some(foreground(t)),
            });
        let copy = column![title, body.into()]
            .spacing(2)
            .width(Length::Fill)
            .align_x(iced::Alignment::Start);
        // A dismiss target belongs beside the complete title/body block.
        // Putting it in the body made that line 40px tall below the title,
        // leaving excess space below short messages and lowering the close icon.
        let has_trailing = trailing.is_some();
        let mut content = row![icon, copy]
            .spacing(13)
            .padding([13, 16])
            .width(Length::Fill)
            .align_y(if has_trailing {
                iced::Alignment::Center
            } else {
                iced::Alignment::Start
            });
        if let Some(trailing) = trailing {
            content = content.push(trailing);
        }

        // CSS draws this as one rounded box whose left border is simply
        // thicker, so the heavy edge follows the corner curve. iced has no
        // per-side border width, and a 3px strip cannot render a 12px radius —
        // it came out as a straight bar butted against a rounded box. Nest
        // instead: an accent-filled outer box, and the container tone inset
        // over it by the border widths. What shows through IS the border.
        let inner = container(content)
            .width(Length::Fill)
            .style(move |t: &Theme| {
                let p = pal_of(t);
                let background = match severity {
                    BannerSeverity::Warning => p.warning_container,
                    BannerSeverity::Error => p.error_container,
                };
                container::Style {
                    background: Some(background.into()),
                    text_color: Some(foreground(t)),
                    border: iced::Border {
                        radius: (theme::shape::MD - BORDER_WIDTH).into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }
            });

        container(inner)
            .width(Length::Fill)
            .padding(iced::Padding {
                top: BORDER_WIDTH,
                right: BORDER_WIDTH,
                bottom: BORDER_WIDTH,
                left: ACCENT_WIDTH,
            })
            .style(move |t: &Theme| {
                let p = pal_of(t);
                let accent = match severity {
                    BannerSeverity::Warning => p.warning,
                    BannerSeverity::Error => p.error,
                };
                container::Style {
                    background: Some(accent.into()),
                    border: iced::Border {
                        radius: theme::shape::MD.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }
            })
            .into()
    }

    /// Wrap a (disabled) driver button in a hover tooltip explaining the
    /// download needs an internet connection — shown while offline so the
    /// greyed-out button isn't a dead end with no explanation.
    fn needs_internet_tooltip<'a>(
        &self,
        btn: impl Into<Element<'a, Message>>,
    ) -> Element<'a, Message> {
        widget::tooltip(
            btn,
            container(text(self.t("driver_needs_internet_tip").to_string()).size(11))
                .padding([6, 10])
                .max_width(240)
                .style(|t: &Theme| theme::tooltip_style(t, theme::shape::SM)),
            widget::tooltip::Position::Top,
        )
        .into()
    }

    /// The active driver banner for the current driver state, or `None` when
    /// drivers are fine. A missing-driver warning takes priority over the
    /// post-install restart reminder, which in turn takes priority over an
    /// available update. Shared by the dashboard and Settings so switching
    /// driver mode surfaces the relevant prompt in both places.
    pub(crate) fn driver_install_banner(&self) -> Option<Element<'_, Message>> {
        use ltbox_device::driver::DriverStatus;
        if matches!(
            self.driver_status,
            Some(
                DriverStatus::Missing(_)
                    | DriverStatus::UdevRulesMissing
                    | DriverStatus::UdevRulesStale
                    | DriverStatus::UdevRulesNoPermission
                    | DriverStatus::KernelDriverMissing
                    | DriverStatus::KernelDriverUnsupported
            )
        ) {
            Some(self.driver_warning_banner())
        } else if self.driver_restart_recommended {
            Some(self.driver_restart_recommended_banner())
        } else if self.driver_update.is_some() {
            Some(self.driver_update_banner())
        } else {
            None
        }
    }

    /// Session-only reminder shown after a successful Qualcomm driver install
    /// or update. Closing it does not persist across app sessions.
    pub(crate) fn driver_restart_recommended_banner(&self) -> Element<'_, Message> {
        let close = button(
            text(self.t("btn_close").to_string())
                .size(theme::text_size::BODY_MEDIUM)
                .wrapping(iced::widget::text::Wrapping::None),
        )
        .padding([10, 18])
        .height(40)
        .style(banner_filled_btn_style)
        .on_press(Message::CloseDriverRestartRecommended);

        let body = row![
            text(self.t("driver_restart_recommended_desc").to_string())
                .size(theme::text_size::BODY_SMALL)
                .style(warning_container_text_style)
                .width(Length::Fill),
            close,
        ]
        .spacing(8)
        .width(Length::Fill)
        .align_y(iced::Alignment::Center);

        self.message_banner(
            BannerSeverity::Warning,
            icon::banner_warning(),
            self.t("driver_restart_recommended_title").to_string(),
            body,
        )
    }

    pub(crate) fn driver_warning_banner(&self) -> Element<'_, Message> {
        use ltbox_device::driver::DriverStatus;
        let installing = self.installing_drivers;
        // Per-state copy. Windows drivers and Linux kernel-driver packages
        // download from GitHub (network required); Linux udev-rules install is
        // a local pkexec call. Unsupported states show copy only, no action.
        let (title_key, desc_key, install_key, needs_network, can_install) =
            match self.driver_status {
                Some(DriverStatus::UdevRulesMissing) => (
                    "driver_udev_missing_title",
                    "driver_udev_missing_desc",
                    "driver_udev_install_btn",
                    false,
                    true,
                ),
                Some(DriverStatus::UdevRulesStale) => (
                    "driver_udev_stale_title",
                    "driver_udev_stale_desc",
                    "driver_udev_install_btn",
                    false,
                    true,
                ),
                Some(DriverStatus::UdevRulesNoPermission) => (
                    "driver_udev_noperm_title",
                    "driver_udev_noperm_desc",
                    "driver_udev_install_btn",
                    false,
                    true,
                ),
                Some(DriverStatus::KernelDriverMissing) => (
                    "driver_kernel_missing_title",
                    "driver_kernel_missing_desc",
                    "driver_install_btn",
                    true,
                    true,
                ),
                Some(DriverStatus::KernelDriverUnsupported) => (
                    "driver_kernel_unsupported_title",
                    "driver_kernel_unsupported_desc",
                    "driver_install_btn",
                    false,
                    false,
                ),
                _ => (
                    "driver_missing_title",
                    "driver_missing_desc",
                    "driver_install_btn",
                    true,
                    true,
                ),
            };
        let offline = needs_network && self.online == Some(false);
        let btn_label = if installing {
            self.t("driver_installing_btn").to_string()
        } else {
            self.t(install_key).to_string()
        };
        // `Length::Shrink` width on the inner text + `wrapping::None` so
        // a long localized label (e.g. Korean "다운로드 & 설치") never
        // collapses into a per-grapheme vertical column when the parent
        // row decides the button's natural width is wider than the slot
        // it has — let the button overflow its slot instead of shredding
        // the label.
        let btn_label_text = text(btn_label)
            .size(theme::text_size::BODY_MEDIUM)
            .wrapping(iced::widget::text::Wrapping::None);
        let action: Element<'_, Message> = if can_install {
            let mut btn = button(btn_label_text)
                .padding([10, 18])
                .height(40)
                .style(banner_filled_btn_style);
            // Offline → the fetch can only fail, so disable + explain on hover.
            if !installing && !offline {
                btn = btn.on_press(Message::InstallDrivers);
            }
            if offline {
                self.needs_internet_tooltip(btn)
            } else {
                btn.into()
            }
        } else {
            Space::new().width(0).into()
        };

        // `body` fills the remainder via `Length::Fill` so the button
        // sits flush right with its natural width — the previous
        // `Space::new().width(Fill)` between two `Shrink` siblings made
        // the row's total width depend on each text's natural width,
        // which under a long desc string overflowed the banner and left
        // the button only a sliver — collapsing its label into a
        // vertical glyph stack.
        let body = row![
            text(self.t(desc_key).to_string())
                .size(theme::text_size::BODY_SMALL)
                .style(warning_container_text_style)
                .width(Length::Fill),
            action,
        ]
        .spacing(12)
        .width(Length::Fill)
        .align_y(iced::Alignment::Center);

        self.message_banner(
            BannerSeverity::Warning,
            icon::banner_warning(),
            self.t(title_key).to_string(),
            body,
        )
    }

    /// Optional "driver update available" banner — shown when the installed
    /// Qualcomm driver is older than the latest release and the user has
    /// not dismissed it. [Update] reuses the install flow; [Don't show
    /// again] persists the dismissal and drops the banner.
    pub(crate) fn driver_update_banner(&self) -> Element<'_, Message> {
        let installing = self.installing_drivers;
        let offline = self.online == Some(false);
        let (current, latest) = self
            .driver_update
            .as_ref()
            .map(|u| (u.current.clone(), u.latest.clone()))
            .unwrap_or_default();

        let update_label = if installing {
            self.t("driver_installing_btn").to_string()
        } else {
            self.t("driver_update_btn").to_string()
        };
        let mut update_btn = button(
            text(update_label)
                .size(theme::text_size::BODY_MEDIUM)
                .wrapping(iced::widget::text::Wrapping::None),
        )
        .padding([10, 18])
        .height(40)
        .style(banner_filled_btn_style);
        if !installing && !offline {
            update_btn = update_btn.on_press(Message::InstallDrivers);
        }
        let update_action: Element<'_, Message> = if offline {
            self.needs_internet_tooltip(update_btn)
        } else {
            update_btn.into()
        };

        let mut dismiss_btn = button(
            text(self.t("driver_dont_show_again").to_string())
                .size(theme::text_size::BODY_MEDIUM)
                .wrapping(iced::widget::text::Wrapping::None),
        )
        .padding([10, 18])
        .height(40)
        .style(banner_text_btn_style);
        // Dismiss needs no network — only gate it on an in-flight install.
        if !installing {
            dismiss_btn = dismiss_btn.on_press(Message::DismissDriverUpdate);
        }

        let body = row![
            text(tr_args!(
                "driver_update_desc",
                current = current,
                latest = latest
            ))
            .size(theme::text_size::BODY_SMALL)
            .style(warning_container_text_style)
            .width(Length::Fill),
            update_action,
            dismiss_btn,
        ]
        .spacing(8)
        .width(Length::Fill)
        .align_y(iced::Alignment::Center);

        self.message_banner(
            BannerSeverity::Warning,
            icon::banner_warning(),
            self.t("driver_update_title").to_string(),
            body,
        )
    }

    /// Bottom-of-screen transient toast. Renders a low-attention pill
    /// over a transparent passthrough container so the rest of the
    /// view keeps responding to clicks while the toast is on screen.
    pub(crate) fn toast_view(&self) -> Element<'_, Message> {
        let Some(msg) = self.toast_msg.clone() else {
            return container(text("")).into();
        };
        // Snackbars use the inverse surface role pair in both schemes.
        let pill = container(
            text(msg)
                .size(12)
                .style(|t: &Theme| iced::widget::text::Style {
                    color: Some(pal_of(t).inverse_on_surface),
                }),
        )
        .padding([0, 16])
        .height(Length::Fixed(TOAST_HEIGHT))
        .align_y(iced::alignment::Vertical::Center)
        .style(|t: &Theme| -> container::Style {
            let p = pal_of(t);
            container::Style {
                background: Some(p.inverse_surface.into()),
                border: iced::Border {
                    // M3 snackbars sit at the extra-small step and carry a
                    // shadow; the pill radius this used to draw belongs to
                    // chips and the nav indicator.
                    radius: theme::shape::XS.into(),
                    ..Default::default()
                },
                shadow: theme::elevation(3, theme::is_dark(t)),
                ..Default::default()
            }
        });
        container(pill)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(iced::Padding {
                top: 0.0,
                right: 0.0,
                // Sits ~100 px above the viewport floor — clears the
                // device-info popup's close-button row without
                // crowding the table content.
                bottom: 100.0,
                left: 0.0,
            })
            .center_x(Length::Fill)
            .align_y(iced::Alignment::End)
            .into()
    }

    pub(crate) fn busy_progress_dialog(&self) -> Element<'_, Message> {
        let op_name = self.busy_operation_label();
        let body = self
            .busy_body_override()
            .unwrap_or_else(|| tr_args!("progress_dialog_body", operation = op_name));

        let indicator = material_circular_progress(MaterialProgressSize::Standard);
        let indicator_box = container(indicator)
            .width(56)
            .height(56)
            .center_x(56)
            .center_y(56)
            .style(|t: &Theme| {
                let p = pal_of(t);
                container::Style {
                    text_color: Some(p.primary),
                    ..Default::default()
                }
            });

        let title_col = column![
            text(self.t("progress_dialog_title").to_string())
                .size(theme::text_size::TITLE_MEDIUM)
                .font(theme::emphasis::medium())
                .style(on_surface_style),
            text(body)
                .size(theme::text_size::BODY_SMALL)
                .style(muted_style),
        ]
        .spacing(6)
        .width(Length::Fill);

        let content = column![
            row![indicator_box, title_col]
                .spacing(18)
                .align_y(iced::Alignment::Center),
        ]
        .spacing(16)
        .padding([16, 20])
        .width(Length::Fixed(theme::DIALOG_WIDTH_MD));

        // Modeless: a flash can run for minutes, so the busy dialog must NOT
        // trap the user — the sidebar stays clickable to navigate back to the
        // running op's progress screen. (Confirm dialogs use the modal
        // `m3_dialog`.)
        m3_dialog_modeless(content.into())
    }

    /// Shared loading-state body for any `_popup_view` that fetches
    /// upstream data. 48 px tall slim box with a centred progress ring —
    /// every popup uses the same shape, so consolidate here instead
    /// of duplicating the container chain in each call site.
    pub(crate) fn popup_loading_view(&self) -> Element<'_, Message> {
        // These popups wait on a single upstream fetch — squarely the
        // 200 ms to 5 s band where M3 specifies the loading indicator in
        // place of an indeterminate progress ring. The busy dialog keeps
        // the ring, since a flash is not a short wait.
        container(material_loading_indicator())
            .width(Length::Fill)
            .height(48)
            .center_x(Length::Fill)
            .center_y(48)
            .into()
    }

    pub(crate) fn view_placeholder(&self) -> Element<'_, Message> {
        column![
            text(self.t(self.current_view.label_key()).to_string())
                .size(theme::text_size::TITLE_LARGE),
            widget::rule::horizontal(1),
            container(text("").size(14).style(muted_style))
                .padding(48)
                .width(Length::Fill)
                .center_x(Length::Fill),
        ]
        .spacing(14)
        .width(Length::Fill)
        .into()
    }
}
