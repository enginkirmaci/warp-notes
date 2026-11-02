//! The warp-notes theme: the family palette, type and scale from
//! warpui-kit-core, re-exported under the app's `theme::` paths, plus the kit
//! mapping.

// every shared font step and color const stays reachable as `theme::X`
pub use warpui_kit_core::fonts::*;
pub use warpui_kit_core::palette;
pub use warpui_kit_core::palette::*;
pub use warpui_kit_core::shell::scaled;

// ---- shell metrics ----------------------------------------------------------

/// The title bar's height, from the shared shell.
pub const TITLEBAR_H: f32 = warpui_kit_core::shell::TITLE_BAR_HEIGHT;

// ---- gpui-kit theme mapping ------------------------------------------------

/// Map the family palette onto gpui-kit's shadcn-style theme tokens. Call
/// after [`gpui_kit::init`], before opening windows.
pub fn apply_kit_theme(cx: &mut gpui::App) {
    let accent = palette::AccentColors::for_preset(palette::normalize_accent(Some("blue")));
    warpui_kit_core::apply_kit_theme(accent, &active_tokens(), palette::BACKGROUND, cx);
}
