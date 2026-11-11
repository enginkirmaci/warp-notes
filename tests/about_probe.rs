//! The About section: the rail opens it on Notes, the list moves the detail,
//! Copy version writes the one-line contract and the license card stays in the
//! window.

use gpui::{Modifiers, Point, TestAppContext, VisualTestContext, px, size};
use std::path::PathBuf;
use warp_notes::theme;
use warp_notes::ui::notes::{Launch, NotesPage};
use warp_notes::ui_prefs::set_prefs_file_for_probe;

/// Same-binary tests share a process; the drawn frame is shared too, so the
/// scenario takes its turn and keeps its prefs inside its own temp home.
static HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn hold_home() -> std::sync::MutexGuard<'static, ()> {
    HOME_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn temp_home(prefix: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!("{prefix}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join("notes")).unwrap();
    set_prefs_file_for_probe(Some(home.join("settings.json")));
    home
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let b = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} missing from the frame"));
    let center = Point {
        x: b.origin.x + b.size.width / 2.,
        y: b.origin.y + b.size.height / 2.,
    };
    cx.simulate_click(center, Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));
}

fn clipboard_text(cx: &VisualTestContext) -> Option<String> {
    cx.read_from_clipboard().and_then(|item| item.text())
}

#[gpui::test]
fn about_opens_on_notes_and_copies_the_selected_app(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-about");

    let handle = cx.open_window(size(px(1400.), px(1000.)), |window, cx| {
        NotesPage::new(
            Launch {
                store_root: Some(home.join("notes")),
                repo_scope: None,
                open_note: None,
                whole_store: true,
            },
            window,
            cx,
        )
    });
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    click(cx, "rail-about");

    for key in [
        "about-row-station",
        "about-row-agent",
        "about-row-notes",
        "about-row-snapit",
    ] {
        cx.debug_bounds(key)
            .unwrap_or_else(|| panic!("{key} renders"));
    }
    // Notes is the app you are in, so it owns the primary and the copy line.
    cx.debug_bounds("about-primary").expect("the primary");

    click(cx, "about-copy-version");
    let line = clipboard_text(cx).expect("copy version writes the clipboard");
    let version = env!("CARGO_PKG_VERSION");
    assert!(
        line.starts_with(&format!("Warp Notes {version} (")),
        "the copy line names Notes, got {line:?}"
    );
    assert!(line.ends_with("· dev"), "a plain build says dev: {line:?}");
}

#[gpui::test]
fn the_license_card_opens_and_closes(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-about-license");

    let handle = cx.open_window(size(px(1400.), px(1000.)), |window, cx| {
        NotesPage::new(
            Launch {
                store_root: Some(home.join("notes")),
                repo_scope: None,
                open_note: None,
                whole_store: true,
            },
            window,
            cx,
        )
    });
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    click(cx, "rail-about");
    assert!(cx.debug_bounds("about-license-dialog").is_none());

    click(cx, "about-link-license");
    cx.debug_bounds("about-license-dialog")
        .expect("View license opens the card");

    click(cx, "about-license-close");
    assert!(
        cx.debug_bounds("about-license-dialog").is_none(),
        "Close dismisses the card"
    );
    cx.debug_bounds("about-page").expect("the page stays");
}
