//! ui module: the Notes window and the shared titlebar control.

pub mod notes;
pub mod settings;

pub use warpui_kit_core::tooltip::tooltip_text;
pub use warpui_kit_core::{A11y, icon};

use gpui::prelude::*;
use gpui::{App, Styled, Window, div, px};

/// One floating-window titlebar button (minimize/zoom/close, the notes up
/// button): a full-height strip of the 46 px bar, so its hover reads as the
/// bar itself lighting up. It occludes the drag region underneath and stops
/// the press so a click stays a click.
pub fn window_button(
    id: &'static str,
    icon_path: &'static str,
    hover_bg: gpui::Hsla,
    tooltip: &'static str,
    show_tooltip: bool,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> impl gpui::IntoElement {
    window_button_glyph(
        id,
        hover_bg,
        tooltip,
        show_tooltip,
        on_click,
        icon(
            icon_path,
            px(crate::theme::icon_medium()),
            crate::theme::text_muted(),
        )
        .into_any_element(),
    )
}

/// [`window_button`] carrying a prepared glyph: the zoom button passes the
/// kit's window-state glyph so the family's maximize/restore reads alike.
pub fn window_button_glyph(
    id: &'static str,
    hover_bg: gpui::Hsla,
    tooltip: &'static str,
    show_tooltip: bool,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
    glyph: gpui::AnyElement,
) -> impl gpui::IntoElement {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .a11y_button(tooltip)
        .cursor_pointer()
        // the titlebar underneath is one big drag region; a control both
        // occludes it from the Windows hit test and stops the press from
        // reaching the drag handler, so a click stays a click
        .occlude()
        .flex_shrink_0()
        .w(px(36.))
        .h(px(warpui_kit_core::shell::scaled(crate::theme::TITLEBAR_H)))
        .flex()
        .items_center()
        .justify_center()
        .hover(move |s| s.bg(hover_bg))
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(on_click)
        .when(show_tooltip, |d| d.tooltip(tooltip_text(tooltip)))
        .child(glyph)
}
