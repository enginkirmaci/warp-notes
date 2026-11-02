//! The warp-notes theme: the family palette, type and scale from
//! warpui-kit-core, re-exported under the app's `theme::` paths, plus the kit
//! mapping.

// every shared font step and color const stays reachable as `theme::X`
pub use warpui_kit_core::fonts::*;
// the fixed semantic colors stay constants (they never follow the palette),
// and the accent preset system rides along
pub use warpui_kit_core::palette::{
    AccentColors, CHANGES, DANGER, current_ramp, normalize_accent, tint,
};
// the module itself names `palette::…` when it maps the kit theme
use warpui_kit_core::palette;
pub use warpui_kit_core::shell::scaled;

// ---- shell metrics ----------------------------------------------------------

/// The title bar's height, from the shared shell.
pub const TITLEBAR_H: f32 = warpui_kit_core::shell::TITLE_BAR_HEIGHT;

// ---- gpui-kit theme mapping ------------------------------------------------

/// Map the family palette onto gpui-kit's shadcn-style theme tokens. Call
/// after [`gpui_kit::init`], before opening windows.
pub fn apply_kit_theme(cx: &mut gpui::App) {
    let accent = palette::AccentColors::for_preset(palette::normalize_accent(Some("blue")));
    warpui_kit_core::apply_kit_theme(accent, &active_tokens(), palette::current_ramp().base, cx);
}

// ---- the live palette ------------------------------------------------------
// Every surface comes through here, so switching the palette in one settings
// page moves all of them. These were constants; they are functions now for
// exactly that reason, and the names stay the apps' own vocabulary.

/// The primary fill.
pub fn accent() -> gpui::Rgba {
    current_ramp().accent
}
/// The accent as ink: links, focus rings, active pills.
pub fn accent_ink() -> gpui::Rgba {
    current_ramp().accent_ink
}
/// The tinted tile behind the brand mark.
pub fn accent_soft() -> gpui::Rgba {
    current_ramp().accent_soft
}
/// Quiet dividers.
pub fn border_light() -> gpui::Rgba {
    current_ramp().line_soft
}
/// The default separator.
pub fn hairline() -> gpui::Rgba {
    current_ramp().line
}
/// Control borders.
pub fn line_strong() -> gpui::Rgba {
    current_ramp().line_strong
}
/// Text on a filled `accent()`.
pub fn on_accent() -> gpui::Rgba {
    current_ramp().on_accent
}
/// A card, a tile, the detail pane.
pub fn panel() -> gpui::Rgba {
    current_ramp().panel
}
/// Menus, dialogs and tooltips.
pub fn popup() -> gpui::Rgba {
    current_ramp().overlay
}
/// The selected row, hover fills, active segments.
pub fn raised() -> gpui::Rgba {
    current_ramp().raised
}
/// The window canvas.
pub fn surface() -> gpui::Rgba {
    current_ramp().base
}
/// Names and values.
pub fn text() -> gpui::Rgba {
    current_ramp().text
}
/// Secondary copy.
pub fn text_dim() -> gpui::Rgba {
    current_ramp().text_dim
}
/// Timestamps and hints.
pub fn text_muted() -> gpui::Rgba {
    current_ramp().text_muted
}
/// Fields, diffs and code: the recessed step.
pub fn well() -> gpui::Rgba {
    current_ramp().well
}

// the app's older spellings, one-liners over the readers above
pub fn background() -> gpui::Rgba {
    surface()
}
pub fn overlay() -> gpui::Rgba {
    popup()
}
pub fn text_faint() -> gpui::Rgba {
    text_muted()
}
