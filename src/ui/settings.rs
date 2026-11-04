//! The Settings section: the shell's list of sections beside its detail. Every
//! screen of every app is rail + list + detail, Settings included, so the
//! section rows live in the list pane and the setting rows in the pane next to
//! it. Appearance applies at once (the design has no Save there) and writes
//! straight through to the prefs file; the store path is the launch contract's
//! answer and is shown, not edited.

use crate::theme;
use crate::ui::A11y;
use crate::ui::notes::{NotesPage, wheel_reaches_content};
use gpui::prelude::*;
use gpui::{Context, FontWeight, SharedString, Window, div, px};
use warpui_kit_core::settings as kit;
use warpui_kit_core::shell;

/// The Settings section's own sections, in list order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum SettingsSection {
    General,
    Appearance,
}

impl SettingsSection {
    fn label(self) -> &'static str {
        match self {
            SettingsSection::General => "General",
            SettingsSection::Appearance => "Appearance",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            SettingsSection::General => "Store and text size",
            SettingsSection::Appearance => "Palette, typeface and motion",
        }
    }
}

impl NotesPage {
    /// The rail's Settings section: a section list and the chosen section's
    /// rows, the same list-and-detail pair every other screen uses.
    pub(crate) fn render_settings(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let active = self.settings_section;
        let mut rows = div().flex().flex_col();
        for section in [SettingsSection::General, SettingsSection::Appearance] {
            rows = rows.child(
                shell::list_row(
                    SharedString::from(format!("settings-section-{}", section.label())),
                    section == active,
                )
                .debug_selector(move || format!("settings-section-{}", section.label()))
                .a11y_button(section.label())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings_section = section;
                    // the section is user-arranged, so it survives restarts
                    this.prefs.settings_section = match section {
                        SettingsSection::General => "general",
                        SettingsSection::Appearance => "appearance",
                    }
                    .into();
                    this.prefs.save();
                    cx.notify();
                }))
                .child(
                    div()
                        .text_size(px(theme::font_name()))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::text())
                        .child(section.label()),
                )
                .child(
                    div()
                        .mt_0p5()
                        .text_size(px(theme::font_row_meta()))
                        .text_color(theme::text_dim())
                        .child(section.hint()),
                ),
            );
        }
        let list = shell::list_pane(self.prefs.list_width)
            .child(
                div()
                    .flex_shrink_0()
                    .px_4()
                    .pt_3p5()
                    .pb_3()
                    .border_b_1()
                    .border_color(theme::hairline())
                    .text_size(px(theme::font_list_title()))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::text())
                    .child("Settings"),
            )
            .child(rows);

        let head = div()
            .flex_shrink_0()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .px(px(shell::DETAIL_PADDING))
            .py_4()
            .border_b_1()
            .border_color(theme::hairline())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(px(theme::font_screen_title()))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::text())
                            .child(active.label()),
                    )
                    .child(
                        div()
                            .mt_0p5()
                            .text_size(px(theme::font_meta()))
                            .text_color(theme::text_dim())
                            .child(active.hint()),
                    ),
            );

        let rows = match active {
            SettingsSection::General => vec![self.store_row(), self.text_row()],
            SettingsSection::Appearance => {
                vec![self.palette_row(), self.typeface_row(), self.motion_row(cx)]
            }
        };

        shell::panes()
            .child(list)
            .child(
                shell::detail_pane()
                    .debug_selector(|| "notes-settings".into())
                    .child(head)
                    .child(
                        div()
                            .id("notes-settings-scroll")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .track_scroll(&self.settings_scroll)
                            // the pane is the one scrollable of the settings
                            // screen: a wheel event that moves it is consumed
                            // here, so the tree (or anything restacked under
                            // a later frame) never reacts to the same event
                            .on_scroll_wheel(cx.listener(
                                |this, event: &gpui::ScrollWheelEvent, window, cx| {
                                    let handle = &this.settings_scroll;
                                    if wheel_reaches_content(
                                        handle.offset(),
                                        handle.max_offset(),
                                        event.delta.pixel_delta(window.line_height()).y,
                                    ) {
                                        cx.stop_propagation();
                                    }
                                },
                            ))
                            .p(px(shell::DETAIL_PADDING))
                            .child(
                                // the design's `.measure`: rows, not cards
                                div()
                                    .w_full()
                                    .min_w_0()
                                    .max_w(px(shell::READING_WIDTH))
                                    .child(kit::rows(rows)),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn store_row(&mut self) -> gpui::AnyElement {
        kit::row_with_desc(
            "settings-row-Store",
            "Notes store",
            "One subfolder per repository. Warp Station passes its notes_store_path here.",
            div()
                .min_w_0()
                .max_w(px(360.))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .font_family(SharedString::from(theme::code_font()))
                .text_size(px(theme::font_mono_meta()))
                .text_color(theme::text_dim())
                .child(SharedString::from(self.store_root.display().to_string())),
            true,
        )
    }

    fn text_row(&mut self) -> gpui::AnyElement {
        kit::row_with_desc(
            "settings-row-Text size",
            "Text size",
            "Applied right away. The other Warp apps follow when they next open.",
            warpui_kit_core::text_size_control(),
            false,
        )
    }

    fn palette_row(&mut self) -> gpui::AnyElement {
        // the live picker, the same one the other apps carry: a click applies
        // in this app at once and the others follow on their next start
        kit::row_with_desc(
            "settings-row-Palette",
            "Palette",
            "Shared by every app in the suite",
            warpui_kit_core::palette_control(),
            true,
        )
    }

    fn typeface_row(&mut self) -> gpui::AnyElement {
        kit::row_with_desc(
            "settings-row-Typeface",
            "Typeface",
            "Interface and code fonts",
            div()
                .text_size(px(theme::font_body()))
                .text_color(theme::text())
                .child("Inter + JetBrains Mono"),
            false,
        )
    }

    fn motion_row(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let reduced = self.prefs.reduce_motion;
        kit::switch_row_with_desc(
            "switch-reduce-motion",
            "Reduce motion",
            "settings-row-Reduce motion",
            "Turns off transitions and keeps spinners still. Busy labels stay",
            reduced,
            false,
            cx.listener(|this, _on: &bool, _window, cx| {
                this.set_reduce_motion(!this.prefs.reduce_motion, cx);
            }),
        )
    }

    /// The motion choice applies at once and lands in the prefs file.
    fn set_reduce_motion(&mut self, on: bool, cx: &mut Context<Self>) {
        self.prefs.reduce_motion = on;
        warpui_kit_core::motion::set_reduced(
            on || warpui_kit_core::motion::system_prefers_reduced(),
        );
        self.prefs.save();
        cx.refresh_windows();
        cx.notify();
    }
}
