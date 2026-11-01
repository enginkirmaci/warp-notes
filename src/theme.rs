//! The warp-notes theme: the family palette and fonts from warpui-kit-core,
//! re-exported under the app's `theme::` paths, plus the kit mapping and the
//! scaled token getters the views read.

pub use warpui_kit_core::fonts::{code_font, resolve_fonts, ui_font};
pub use warpui_kit_core::palette;
// every shared color const stays reachable as `theme::X`
pub use warpui_kit_core::palette::*;

// ---- gpui-kit theme mapping ------------------------------------------------

/// Map the family palette onto gpui-kit's shadcn-style theme tokens. Call
/// after [`gpui_kit::init`], before opening windows.
pub fn apply_kit_theme(cx: &mut gpui::App) {
    let accent = palette::AccentColors::for_preset(palette::normalize_accent(Some("blue")));
    warpui_kit_core::apply_kit_theme(accent, &font_tokens(), palette::BACKGROUND, cx);
}

/// The tokens the kit mapping rides: the desktop text-scaling factor.
pub fn font_tokens() -> warpui_kit_core::FontTokens {
    warpui_kit_core::font_tokens(None, warpui_kit_core::system_factor())
}

// ---- token getters ---------------------------------------------------------
// The bases are the sizes the family pins; the kit rem base rides the same
// factor.

fn scaled(v: f32) -> f32 {
    (v * warpui_kit_core::font_factor() * 100.0).round() / 100.0
}

pub fn font_small() -> f32 {
    scaled(12.0)
}

pub fn font_body() -> f32 {
    scaled(13.0)
}

pub fn font_title() -> f32 {
    scaled(15.0)
}

/// The note text size; headings and code scale from it.
pub fn font_note() -> f32 {
    scaled(16.0)
}

pub fn icon_small() -> f32 {
    scaled(13.0)
}

pub fn icon_medium() -> f32 {
    scaled(15.0)
}

pub fn icon_large() -> f32 {
    scaled(17.0)
}
