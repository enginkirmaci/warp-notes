//! The Settings section: the shell's list of sections beside its detail. Every
//! screen of every app is rail + list + detail, Settings included, so the
//! section rows live in the list pane and the setting rows in the pane next to
//! it. Appearance applies at once (the design has no Save there) and writes
//! straight through to the prefs file; the store path is the launch contract's
//! answer and is shown, not edited.

use crate::theme;
use crate::ui::A11y;
use crate::ui::notes::NotesPage;
use gpui::prelude::*;
use gpui::{Context, FontWeight, SharedString, Window, div, px};
use gpui_kit::component::switch::Switch;
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

        let cards = match active {
            SettingsSection::General => vec![self.store_card(), self.text_card()],
            SettingsSection::Appearance => {
                vec![
                    self.palette_card(),
                    self.typeface_card(),
                    self.motion_card(cx),
                ]
            }
        };
        let mut column = div()
            .w_full()
            .max_w(px(shell::READING_WIDTH))
            .flex()
            .flex_col()
            .gap_3();
        for card in cards {
            column = column.child(card);
        }

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
                            .p(px(shell::DETAIL_PADDING))
                            .child(column),
                    ),
            )
            .into_any_element()
    }

    fn store_card(&mut self) -> gpui::AnyElement {
        card("Store")
            .child(setting_row(
                "Notes store",
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
            ))
            .child(caption(
                "One subfolder per repository. Warp Station passes its notes_store_path here.",
            ))
            .into_any_element()
    }

    fn text_card(&mut self) -> gpui::AnyElement {
        card("Text")
            .child(setting_row(
                "Text size",
                warpui_kit_core::text_size_control(),
            ))
            .child(caption(
                "Applied right away. The other Warp apps follow when they next open.",
            ))
            .into_any_element()
    }

    fn palette_card(&mut self) -> gpui::AnyElement {
        // the live picker, the same one the other apps carry: a click applies
        // in this app at once and the others follow on their next start
        card("Palette")
            .child(setting_row("Palette", warpui_kit_core::palette_control()))
            .child(caption(
                "Applied right away. The other Warp apps follow when they next open.",
            ))
            .into_any_element()
    }

    fn typeface_card(&mut self) -> gpui::AnyElement {
        card("Typeface")
            .child(setting_row(
                "Typeface",
                div()
                    .text_size(px(theme::font_body()))
                    .text_color(theme::text())
                    .child("Inter + JetBrains Mono"),
            ))
            .child(caption(
                "The family's two faces, bundled with the app. Code, paths and note names set in mono.",
            ))
            .into_any_element()
    }

    fn motion_card(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let reduced = self.prefs.reduce_motion;
        card("Motion")
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .min_h(px(shell::scaled(60.)))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(theme::font_body()))
                            .text_color(theme::text())
                            .child("Reduce motion"),
                    )
                    .child(
                        Switch::new(SharedString::from("switch-reduce-motion"))
                            .checked(reduced)
                            .accessibility_label("Reduce motion")
                            .on_click(cx.listener(|this, _on: &bool, _window, cx| {
                                this.set_reduce_motion(!this.prefs.reduce_motion, cx);
                            })),
                    ),
            )
            .child(caption(
                "Turns animation off, spinners included. The busy words stay.",
            ))
            .into_any_element()
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

/// One settings card: a base card on the detail pane, its heading and rows.
fn card(title: &str) -> gpui::Div {
    div()
        .rounded(px(8.))
        .border_1()
        .border_color(theme::hairline())
        .bg(theme::surface())
        .p_4()
        .flex()
        .flex_col()
        .gap_2p5()
        .child(
            div()
                .text_size(px(theme::font_heading()))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::text())
                .child(SharedString::from(title)),
        )
}

/// One setting: the label left, the control right, 60 high.
fn setting_row(label: &str, control: impl gpui::IntoElement) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap_3()
        .min_h(px(shell::scaled(60.)))
        .child(
            div()
                .flex_1()
                .text_size(px(theme::font_body()))
                .text_color(theme::text())
                .child(SharedString::from(label)),
        )
        .child(control)
}

fn caption(text: &str) -> gpui::Div {
    div()
        .text_size(px(theme::font_meta()))
        .text_color(theme::text_faint())
        .child(SharedString::from(text))
}
