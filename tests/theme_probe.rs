//! The kit theme must carry the live palette, accent included. A fixed accent
//! here was the last place warp-notes ignored a palette switch.

use gpui::TestAppContext;
use warp_notes::theme;

#[gpui_kit::gpui::test]
fn accent_follows_the_live_ramp(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    cx.update(|cx| {
        warpui_kit_core::palette::set_ramp("plum");
        theme::apply_kit_theme(cx);
    });
    cx.update(|cx| {
        use gpui_kit::component::Theme;
        let t = Theme::global(cx);
        assert_eq!(
            t.primary,
            gpui::Hsla::from(theme::accent()),
            "the kit primary follows the live accent"
        );
        assert_eq!(t.background, gpui::Hsla::from(theme::background()));
    });

    // Leave the process on the default ramp for any later test in this binary.
    cx.update(|cx| {
        warpui_kit_core::palette::set_ramp("copper");
        theme::apply_kit_theme(cx);
    });
    cx.update(|cx| {
        use gpui_kit::component::Theme;
        assert_eq!(
            Theme::global(cx).primary,
            gpui::Hsla::from(theme::accent()),
            "restoring Copper restores the accent"
        );
    });
}
