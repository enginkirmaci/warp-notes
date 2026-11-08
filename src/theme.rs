//! The warp-notes theme: the family palette, type and scale from
//! warpui-kit-core, re-exported under the app's `theme::` paths, plus the kit
//! mapping.

// every shared font step and color const stays reachable as `theme::X`
pub use warpui_kit_core::fonts::*;
// the fixed semantic colors stay constants (they never follow the palette),
// and the accent preset system rides along
pub use warpui_kit_core::palette::{AccentColors, CHANGES, DANGER, SCRIM, current_ramp, tint};
// the module itself names `palette::…` when it maps the kit theme
use warpui_kit_core::palette;
pub use warpui_kit_core::shell::scaled;

// ---- shell metrics ----------------------------------------------------------

/// The title bar's height, from the shared shell.
pub const TITLEBAR_H: f32 = warpui_kit_core::shell::TITLE_BAR_HEIGHT;

/// The cap on the store-relative path in the note head. The detail pane's
/// minimum width must not follow the path: a deeper store (repo notes under
/// `projects/`) made the pane wider than the window and pushed the head's
/// controls past its edge.
pub const NOTE_PATH_MAX_W: f32 = 260.;

/// The tree's row-action menu.
pub const MENU_WIDTH: f32 = 224.;

/// The rename/move/new-folder prompt card.
pub const PROMPT_WIDTH: f32 = 420.;

/// The Trash dialog card.
pub const TRASH_WIDTH: f32 = 620.;

// ---- gpui-kit theme mapping ------------------------------------------------

/// Map the family palette onto gpui-kit's shadcn-style theme tokens. Call
/// after [`gpui_kit::init`], before opening windows.
pub fn apply_kit_theme(cx: &mut gpui::App) {
    // the accent rides the live ramp, read at every retheme so a palette
    // switch moves the kit's primary controls with the rest of the app
    let accent = palette::current_ramp().accent_colors();
    warpui_kit_core::apply_kit_theme(accent, &active_tokens(), palette::current_ramp().base, cx);
}

// ---- the live palette ------------------------------------------------------
// Every surface comes through here, so switching the palette in one settings
// page moves all of them. These were constants; they are functions now for
// exactly that reason, and the names stay the apps' own vocabulary — the
// readers themselves are the kit's, re-exported.

pub use warpui_kit_core::palette::{
    accent, accent_ink, accent_soft, border_light, hairline, line_strong, on_accent, panel, popup,
    raised, surface, text, text_dim, text_muted, well,
};

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
