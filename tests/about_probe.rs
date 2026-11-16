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

    // The list holds only the members installed on this machine, and the app
    // you are in is always one of them. The kit's own probe covers the
    // multi-member screen.
    cx.debug_bounds("about-row-notes")
        .expect("the app you are in is always listed");
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

/// A sibling's `--version` is the only fact source the About list has for a
/// binary this process did not compile, so the probe drives a real executable
/// through `PATH` instead of trusting the detection path by inspection.
#[cfg(unix)]
fn fake_binary(dir: &std::path::Path, name: &str, version_line: &str) {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(name);
    std::fs::write(
        &path,
        format!("#!/bin/sh\nprintf '%s\\n' '{version_line}'\n"),
    )
    .expect("write the fake sibling");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod the fake sibling");
}

/// Restores `PATH` when the probe leaves, panic included, so a sibling test
/// cannot leak its scratch bin dir into the other probes in this binary.
#[cfg(unix)]
struct PathGuard(Option<std::ffi::OsString>);

#[cfg(unix)]
impl Drop for PathGuard {
    fn drop(&mut self) {
        // SAFETY: process-global env, restored inside the same test body and
        // serialized by HOME_LOCK
        unsafe {
            match self.0.take() {
                Some(path) => std::env::set_var("PATH", path),
                None => std::env::remove_var("PATH"),
            }
        }
    }
}

/// Only the members whose binary is installed are listed: an installed
/// sibling answers `--version` and gets a row, an absent one does not.
#[cfg(unix)]
#[gpui::test]
fn only_installed_siblings_are_listed(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-about-installed");
    let bin = home.join("bin");
    std::fs::create_dir_all(&bin).expect("scratch bin dir");
    fake_binary(&bin, "warp-station", "Warp Station 9.9.9 (abc1234) · dev");

    let original = std::env::var_os("PATH");
    let mut entries = vec![bin];
    if let Some(rest) = &original {
        entries.extend(std::env::split_paths(rest));
    }
    // SAFETY: process-global env, restored by the guard when the test ends
    unsafe {
        std::env::set_var("PATH", std::env::join_paths(entries).expect("join PATH"));
    }
    let _path = PathGuard(original);

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

    cx.debug_bounds("about-row-notes")
        .expect("the app you are in is listed");
    cx.debug_bounds("about-row-station")
        .expect("an installed sibling is listed");
    for key in ["about-row-agent", "about-row-snapit"] {
        assert!(
            cx.debug_bounds(key).is_none(),
            "{key} is not installed and must not be listed"
        );
    }

    // The row's version comes from the sibling's own `--version` line, and the
    // copy verb writes exactly that member's line.
    click(cx, "about-row-station");
    click(cx, "about-copy-version");
    assert_eq!(
        clipboard_text(cx).as_deref(),
        Some("Warp Station 9.9.9 (abc1234) · dev"),
        "the copy line is the sibling's own version line"
    );
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
