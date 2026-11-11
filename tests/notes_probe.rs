//! Headless probe for the Notes window: the carded sidebar, opening a note
//! from the tree, the single Obsidian-style live editor, task checkboxes
//! toggling on click, Enter continuing lists, wikilinks leading to notes, and
//! the launch contract (--repo pins the scope, --note opens a note right away).

use warp_notes::theme;
use warp_notes::ui::notes::{Launch, NotesPage};
use warp_notes::ui_prefs::set_prefs_file_for_probe;

use gpui::{
    AppContext, Bounds, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, Pixels, Point,
    TestAppContext, VisualTestContext, px, size,
};
use std::path::PathBuf;

fn seed_notes(home: &std::path::Path) {
    let notes = home.join("notes/projects/alpha");
    std::fs::create_dir_all(&notes).unwrap();
    std::fs::write(notes.join("note.md"), "# Note\n\nHello **world**.\n").unwrap();
    std::fs::write(notes.join("task.md"), "- [ ] alpha task\nplain\n").unwrap();
    std::fs::write(notes.join("list.md"), "- alpha\n").unwrap();
    let beta = home.join("notes/projects/beta");
    std::fs::create_dir_all(&beta).unwrap();
    std::fs::write(beta.join("beta.md"), "# Beta\n").unwrap();
}

fn bounds(cx: &mut VisualTestContext, selector: &'static str) -> Option<Bounds<Pixels>> {
    cx.debug_bounds(selector)
}

/// Same-binary gpui tests share one process and the store root is created
/// per scenario under a unique temp dir, but the drawn frame is shared —
/// serialize the scenarios that seed their own notes tree on this lock
/// (poison-tolerant: one failure must not fail the rest).
static HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn hold_home() -> std::sync::MutexGuard<'static, ()> {
    HOME_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let b = bounds(cx, selector).unwrap_or_else(|| panic!("{selector} missing from the frame"));
    let center = Point {
        x: b.origin.x + b.size.width / 2.,
        y: b.origin.y + b.size.height / 2.,
    };
    cx.simulate_click(center, Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));
}

fn click_str(cx: &mut VisualTestContext, selector: &str) {
    let selector: &'static str = Box::leak(selector.to_string().into_boxed_str());
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

/// A double click: the platform's second press and release carry
/// `click_count: 2`, so both clicks reach the element.
fn double_click(cx: &mut VisualTestContext, selector: &str) {
    let selector: &'static str = Box::leak(selector.to_string().into_boxed_str());
    let b = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} missing from the frame"));
    let center = Point {
        x: b.origin.x + b.size.width / 2.,
        y: b.origin.y + b.size.height / 2.,
    };
    for click_count in 1..=2 {
        cx.simulate_event(MouseDownEvent {
            position: center,
            modifiers: Modifiers::default(),
            button: MouseButton::Left,
            click_count,
            first_mouse: false,
        });
        cx.simulate_event(MouseUpEvent {
            position: center,
            modifiers: Modifiers::default(),
            button: MouseButton::Left,
            click_count,
        });
        cx.run_until_parked();
        cx.update(|window, cx| drop(window.draw(cx)));
    }
}

fn middle_click(cx: &mut VisualTestContext, selector: &str) {
    let selector: &'static str = Box::leak(selector.to_string().into_boxed_str());
    let b = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} missing from the frame"));
    let center = Point {
        x: b.origin.x + b.size.width / 2.,
        y: b.origin.y + b.size.height / 2.,
    };
    cx.simulate_mouse_down(center, MouseButton::Middle, Modifiers::default());
    cx.simulate_mouse_up(center, MouseButton::Middle, Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));
}

/// Move the open note to Trash through the head's verb menu: the detail head
/// carries no Trash button of its own.
fn trash_open_note(cx: &mut VisualTestContext) {
    click(cx, "note-verbs");
    click(cx, "notes-menu-trash");
}

fn set_prompt(cx: &mut VisualTestContext, page: &gpui::Entity<NotesPage>, value: &str) {
    let input = page.read_with(cx, |page, _| page.prompt_input_handle().clone());
    cx.update(|window, cx| {
        input.update(cx, |field, cx| {
            field.set_value(value.to_string(), window, cx)
        });
    });
    cx.run_until_parked();
}

fn set_palette(cx: &mut VisualTestContext, page: &gpui::Entity<NotesPage>, value: &str) {
    let input = page.read_with(cx, |page, _| page.palette_input_handle().clone());
    cx.update(|window, cx| {
        input.update(cx, |field, cx| {
            field.set_value(value.to_string(), window, cx)
        });
    });
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));
}

/// Points HOME at the scenario while the guard lives; a standalone launch
/// (no `--store`) resolves its store from it.
struct HomeSwap {
    prior: Vec<(&'static str, Option<std::ffi::OsString>)>,
}

impl HomeSwap {
    fn set(home: &std::path::Path) -> Self {
        let prior = ["HOME", "USERPROFILE"]
            .into_iter()
            .map(|key| (key, std::env::var_os(key)))
            .collect::<Vec<_>>();
        for (key, _) in &prior {
            // SAFETY: process-global env, serialized by HOME_LOCK
            unsafe { std::env::set_var(key, home) };
        }
        Self { prior }
    }
}

impl Drop for HomeSwap {
    fn drop(&mut self) {
        for (key, value) in self.prior.drain(..) {
            match value {
                Some(value) => unsafe { std::env::set_var(key, value) },
                None => unsafe { std::env::remove_var(key) },
            }
        }
    }
}

fn temp_home(prefix: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!("{prefix}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    seed_notes(&home);
    point_prefs_at(&home);
    home
}

/// The scenario's prefs live inside its own temp home: a relaunch inside the
/// scenario reads them back, and the developer's own
/// ~/.warp/notes/settings.json is neither read nor rewritten by a test run.
/// Scenarios serialize on the home lock, so the global override never crosses
/// between them.
fn point_prefs_at(home: &std::path::Path) {
    set_prefs_file_for_probe(Some(home.join("settings.json")));
}

fn open_page(
    cx: &mut TestAppContext,
    launch: Launch,
) -> (gpui::WindowHandle<NotesPage>, gpui::Entity<NotesPage>) {
    let mut page: Option<gpui::Entity<NotesPage>> = None;
    let handle = cx.open_window(size(px(1000.), px(700.)), |window, cx| {
        page = Some(cx.entity());
        NotesPage::new(launch, window, cx)
    });
    (handle, page.unwrap())
}

#[gpui::test]
fn repo_scope_up_button_tree_and_live_editor(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);

    let home = temp_home("warpnotes-probe-scope");
    // --repo alpha: the first open pins to that repo's notes folder
    let (handle, page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    // the shell list pane at the shell's default width
    let sidebar = bounds(cx, "notes-sidebar").expect("notes sidebar must render");
    assert_eq!(
        sidebar.size.width,
        px(warpui_kit_core::shell::LIST_WIDTH),
        "the list keeps the shell's 432px default"
    );

    // the text size moved from the tree's foot to the Settings section
    assert!(
        bounds(cx, "text-size-normal").is_none(),
        "the tree no longer carries the text size",
    );
    click(cx, "rail-settings");
    assert!(
        bounds(cx, "text-size-normal").is_some(),
        "Settings carries the text size",
    );
    // the palette is the same live picker the other apps carry, so a note on
    // Plum does not stay on a hardcoded Copper row. It lives in Appearance.
    click(cx, "settings-section-Appearance");
    for key in ["copper", "plum", "pine", "blue", "gray"] {
        let selector: &'static str = Box::leak(format!("palette-{key}").into_boxed_str());
        assert!(
            bounds(cx, selector).is_some(),
            "Settings offers the {key} palette",
        );
    }
    click(cx, "rail-notes");

    // repo-scoped launch roots at that repo's notes folder
    assert!(
        bounds(cx, "note-item-note.md").is_some(),
        "--repo pins the tree to the repo's notes",
    );
    assert!(
        bounds(cx, "notes-up-dir").is_some(),
        "repo-scoped notes show the up button",
    );
    assert!(
        bounds(cx, "note-item-beta.md").is_none(),
        "other repos' notes stay hidden while scoped",
    );

    // up leaves the repo folder for the whole store: repo folders live under
    // projects/, so the root shows that one folder, not its children
    click(cx, "notes-up-dir");
    assert!(
        bounds(cx, "note-item-projects").is_some(),
        "up shows the store root, where projects/ groups the repo folders"
    );
    assert!(
        bounds(cx, "note-item-alpha").is_none(),
        "a repo folder is not a sibling of the store's own folders"
    );
    assert!(
        bounds(cx, "notes-up-dir").is_none(),
        "store root has no up button"
    );

    // open projects/, then the repo folder, then a note from the tree
    click(cx, "note-item-projects");
    assert!(
        bounds(cx, "note-item-alpha").is_some(),
        "projects/ holds every repo folder"
    );
    assert!(
        bounds(cx, "note-item-beta").is_some(),
        "projects/ holds every repo folder"
    );
    click(cx, "note-item-alpha");
    click(cx, "note-item-note.md");
    assert!(
        bounds(cx, "notes-toolbar").is_some(),
        "the note header strip appears once a note is open",
    );

    // single surface: the live editor IS the pane, the mode switcher is gone
    assert!(
        bounds(cx, "notes-live").is_some(),
        "the live editor pane renders"
    );
    for mode in [
        "notes-mode-Edit",
        "notes-mode-Live",
        "notes-mode-Split",
        "notes-mode-Preview",
    ] {
        assert!(
            bounds(cx, mode).is_none(),
            "the {mode} mode switcher is gone"
        );
    }

    // clicking a task checkbox toggles it in place — the box is the `[ ]`
    // span of the first line, toggling keeps its offsets stable. Collapse
    // first: alpha was expanded by the note-open walk above, and the click
    // would fold it instead of showing task.md. The caret moves off the line
    // first: right on the marker the box shows as source, as in Obsidian.
    click(cx, "notes-collapse-all");
    click(cx, "note-item-projects");
    click(cx, "note-item-alpha");
    click(cx, "note-item-task.md");
    cx.update(|_, cx| {
        let editor = page.read(cx).editor_handle().clone();
        editor.update(cx, |state, cx| state.set_selected_range(18..18, cx));
    });
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));
    let box_center = {
        let b = cx
            .update(|_, cx| {
                page.read(cx)
                    .editor_handle()
                    .read(cx)
                    .range_to_bounds(&(2..5))
            })
            .expect("the task box has layout bounds");
        Point::new(
            b.origin.x + b.size.width / 2.,
            b.origin.y + b.size.height / 2.,
        )
    };
    cx.simulate_click(box_center, Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));
    cx.update(|_, cx| {
        let value = page.read(cx).editor_value(cx);
        assert!(
            value.contains("- [x]"),
            "clicking the box checks the task: {value:?}"
        );
    });
    cx.simulate_click(box_center, Modifiers::default());
    cx.run_until_parked();
    cx.update(|_, cx| {
        let value = page.read(cx).editor_value(cx);
        assert!(
            value.contains("- [ ]"),
            "clicking again unchecks it: {value:?}"
        );
        assert_eq!(
            page.read(cx).editor_handle().read(cx).selected_range(),
            18..18,
            "a checkbox click leaves the caret where it was"
        );
    });

    // Enter inside a list item continues the list. The real key path is the
    // kit's Enter action (the harness's simulated-IME keystroke would also
    // inject a literal '\n' before deferred events run), so dispatch the
    // action itself.
    click(cx, "note-item-list.md");
    cx.update(|window, cx| {
        let editor = page.read(cx).editor_handle().clone();
        editor.update(cx, |state, cx| {
            state.focus(window, cx);
            state.set_selected_range(7..7, cx); // end of "- alpha"
        });
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.dispatch_action(
            Box::new(gpui_kit::base::input::Enter {
                secondary: false,
                shift: false,
            }),
            cx,
        )
    });
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));
    cx.update(|_, cx| {
        let value = page.read(cx).editor_value(cx);
        assert!(
            value.contains("- alpha\n- "),
            "Enter continues the list: {value:?}"
        );
    });

    // typing markdown on the live surface drives the live highlighter on
    // every edit (its parse must not panic and the buffer must hold)
    click(cx, "note-item-note.md");
    let live = bounds(cx, "notes-live").unwrap();
    cx.simulate_click(
        Point::new(
            live.origin.x + live.size.width / 2.,
            live.origin.y + live.size.height / 2.,
        ),
        Modifiers::default(),
    );
    cx.run_until_parked();
    cx.simulate_input("# Heading\n\n**bold** and `code`\n- [x] done\n");
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));
    assert!(
        bounds(cx, "notes-live").is_some(),
        "the live pane survives markdown typing"
    );
    cx.update(|_, cx| {
        let value = page.read(cx).editor_value(cx);
        assert!(
            value.contains("# Heading"),
            "typed markdown reached the buffer: {value:?}"
        );
        assert!(
            value.contains("**bold**"),
            "typed bold markers intact: {value:?}"
        );
    });

    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
fn opening_a_note_puts_the_keyboard_in_the_editor(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-focus");
    let (handle, page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));
    click(cx, "note-item-note.md");
    cx.simulate_input("x");
    cx.run_until_parked();
    cx.update(|_, cx| {
        let value = page.read(cx).editor_value(cx);
        assert!(
            value.starts_with('x'),
            "typing goes straight into the opened note: {value:?}"
        );
    });
    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
fn following_a_wikilink_opens_or_creates_the_note(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-wiki");
    let notes = home.join("notes/projects/alpha");
    std::fs::write(
        notes.join("links.md"),
        "see [[list]] and [[Brand New]]\n\nend",
    )
    .unwrap();
    let (handle, page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: Some(PathBuf::from("projects/alpha/links.md")),
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    // the existing note: focused caret parked below, so the link renders
    let caret_away = |cx: &mut VisualTestContext| {
        cx.update(|_, cx| {
            let editor = page.read(cx).editor_handle().clone();
            let end = editor.read(cx).value().len();
            editor.update(cx, |state, cx| state.set_selected_range(end..end, cx));
        });
        cx.run_until_parked();
        cx.update(|window, cx| drop(window.draw(cx)));
    };
    caret_away(cx);
    let link_at = |cx: &mut VisualTestContext, needle: &str| {
        cx.update(|_, cx| {
            let editor = page.read(cx).editor_handle().read(cx);
            let start = editor.value().find(needle).unwrap();
            editor
                .range_to_bounds(&(start..start + needle.len()))
                .expect("the link has bounds")
        })
    };
    let bounds = link_at(cx, "list");
    cx.simulate_click(
        Point::new(
            bounds.origin.x + px(2.),
            bounds.origin.y + bounds.size.height / 2.,
        ),
        Modifiers::default(),
    );
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));
    cx.update(|_, cx| {
        let value = page.read(cx).editor_value(cx);
        assert!(
            value.starts_with("- alpha"),
            "the link opened list.md: {value:?}"
        );
    });

    // a link to a note that does not exist creates it, as Obsidian does
    click(cx, "note-item-links.md");
    caret_away(cx);
    let bounds = link_at(cx, "Brand New");
    cx.simulate_click(
        Point::new(
            bounds.origin.x + px(2.),
            bounds.origin.y + bounds.size.height / 2.,
        ),
        Modifiers::default(),
    );
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));
    assert!(
        notes.join("Brand New.md").is_file(),
        "the missing note was created beside the open one"
    );
    cx.update(|_, cx| {
        let value = page.read(cx).editor_value(cx);
        assert_eq!(value, "", "and opened empty");
    });
    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
fn note_arg_opens_the_requested_note(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);

    let home = temp_home("warpnotes-probe-note");
    // --note projects/beta/beta.md (store-relative): opens right away, no
    // scope pin
    let (handle, page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: None,
            open_note: Some(PathBuf::from("projects/beta/beta.md")),
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    cx.update(|_, cx| {
        let value = page.read(cx).editor_value(cx);
        assert!(
            value.contains("# Beta"),
            "--note opens the requested note: {value:?}"
        );
    });
    // whole-store root: projects/ is the one folder holding the repo folders
    assert!(
        bounds(cx, "note-item-projects").is_some(),
        "a --note launch without --repo roots at the whole store",
    );
    assert!(
        bounds(cx, "notes-toolbar").is_some(),
        "the note header strip is up"
    );

    let _ = std::fs::remove_dir_all(&home);
}

/// The titlebar's launch shape: `--root` opens the whole store even when the
/// prefs remember a repo pin, and drops that pin for the next bare launch.
#[gpui::test]
fn root_launch_overrides_a_remembered_repo_scope(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);

    let home = temp_home("warpnotes-probe-root");
    let prefs_path = home.join("settings.json");
    std::fs::write(
        &prefs_path,
        r#"{"schema_version": 1, "notes_scope": "alpha"}"#,
    )
    .unwrap();
    let (handle, _page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: None,
            open_note: None,
            whole_store: true,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    // projects/ is at the root (the repo folders are inside it), nothing is
    // pinned
    assert!(
        bounds(cx, "note-item-projects").is_some(),
        "--root shows every repo folder",
    );
    assert!(
        bounds(cx, "notes-up-dir").is_none(),
        "the root has no scope chip or up button",
    );
    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&prefs_path).unwrap()).unwrap();
    assert!(
        saved["notes_scope"].is_null(),
        "the remembered pin is dropped: {saved}",
    );

    let _ = std::fs::remove_dir_all(&home);
}

/// With `--repo` pinned, a relative `--note` is read inside that repo's
/// folder: `--repo alpha --note note.md` opens `alpha/note.md` (the smoke
/// test's launch shape, which used to land on "No note open").
#[gpui::test]
fn a_relative_note_opens_inside_the_pinned_repo(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-repo-note");
    let (handle, page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: Some(PathBuf::from("note.md")),
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));
    cx.update(|_, cx| {
        let value = page.read(cx).editor_value(cx);
        assert!(
            value.contains("# Note"),
            "the repo-relative note opened: {value:?}"
        );
    });
    let _ = std::fs::remove_dir_all(&home);
}

// ---- image paste ----------------------------------------------------------

/// A valid 1x1 RGBA PNG (magic + IHDR + IDAT + IEND).
const PNG_BYTES: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0xfc, 0xcf, 0xc0, 0x50,
    0x0f, 0x00, 0x04, 0x85, 0x01, 0x80, 0x84, 0xa9, 0x8c, 0x21, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
    0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

struct PasteProbe {
    cx: &'static mut VisualTestContext,
    page: gpui::Entity<NotesPage>,
    home: PathBuf,
    /// holds the scenario lock for the whole scenario, not just the setup
    _guard: std::sync::MutexGuard<'static, ()>,
}

/// A window with the seeded alpha note open in the live editor, the editor
/// focused. Each caller passes its own temp-dir prefix — same-binary tests
/// share a process id, and one caller's cleanup must not wipe another's tree.
fn paste_probe(tcx: &mut TestAppContext, prefix: &str) -> PasteProbe {
    let guard = hold_home();
    tcx.update(gpui_kit::init);
    tcx.update(theme::apply_kit_theme);

    let home = temp_home(prefix);
    let (handle, page) = open_page(
        tcx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), tcx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));
    click(cx, "note-item-note.md");
    cx.update(|window, cx| {
        let editor = page.read(cx).editor_handle().clone();
        editor.update(cx, |state, cx| state.focus(window, cx));
    });
    cx.run_until_parked();
    // into_mut hands back a &'static mut — deref-copy into the probe struct
    PasteProbe {
        cx,
        page,
        home,
        _guard: guard,
    }
}

fn dispatch_paste(probe: &mut PasteProbe) {
    probe
        .cx
        .update(|window, cx| window.dispatch_action(Box::new(gpui_kit::base::input::Paste), cx));
    probe.cx.run_until_parked();
    probe.cx.update(|window, cx| drop(window.draw(cx)));
}

#[gpui::test]
fn pasting_a_clipboard_image_saves_an_asset_and_inserts_a_ref(tcx: &mut TestAppContext) {
    let mut probe = paste_probe(tcx, "warpnotes-paste-img");
    probe.cx.update(|_, cx| {
        cx.write_to_clipboard(gpui::ClipboardItem::new_image(&gpui::Image::from_bytes(
            gpui::ImageFormat::Png,
            PNG_BYTES.to_vec(),
        )))
    });
    dispatch_paste(&mut probe);

    probe.cx.update(|_, cx| {
        let page = probe.page.read(cx);
        let value = page.editor_value(cx);
        assert!(
            value.contains("![image](assets/paste-"),
            "image paste inserts a relative markdown ref: {value:?}"
        );
        assert!(
            value.starts_with("![image](assets/paste-"),
            "the ref lands at the caret and is complete: {value:?}"
        );
        assert!(page.is_dirty(cx), "pasting marks the note dirty");
    });

    let assets = probe.home.join("notes/projects/alpha/assets");
    let mut entries: Vec<PathBuf> = std::fs::read_dir(&assets)
        .expect("the assets folder is created next to the note")
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(entries.len(), 1, "exactly one asset per paste: {entries:?}");
    let path = entries.remove(0);
    assert_eq!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("png"),
        "{}",
        path.display()
    );
    assert!(
        path.file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("paste-"),
        "{}",
        path.display()
    );
    assert_eq!(
        std::fs::read(&path).unwrap(),
        PNG_BYTES,
        "bytes land on disk unchanged"
    );

    let _ = std::fs::remove_dir_all(&probe.home);
}

#[gpui::test]
fn pasting_text_never_writes_an_asset(tcx: &mut TestAppContext) {
    let mut probe = paste_probe(tcx, "warpnotes-paste-text");
    probe.cx.update(|_, cx| {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string("pasted text".into()))
    });
    dispatch_paste(&mut probe);

    probe.cx.update(|_, cx| {
        let value = probe.page.read(cx).editor_value(cx);
        assert!(
            value.contains("pasted text"),
            "text paste inserts as text: {value:?}"
        );
    });
    assert!(
        !probe.home.join("notes/projects/alpha/assets").exists(),
        "text paste writes no asset folder"
    );

    let _ = std::fs::remove_dir_all(&probe.home);
}

#[gpui::test]
fn a_mixed_clipboard_pastes_text_not_image(tcx: &mut TestAppContext) {
    // Windows delivers text+image as ONE item with multiple entries; the
    // text entry must win and no asset may be written
    let mut probe = paste_probe(tcx, "warpnotes-paste-mix");
    probe.cx.update(|_, cx| {
        let mut item = gpui::ClipboardItem::new_string("mixed text wins".into());
        item.entries
            .push(gpui::ClipboardEntry::Image(gpui::Image::from_bytes(
                gpui::ImageFormat::Png,
                PNG_BYTES.to_vec(),
            )));
        cx.write_to_clipboard(item)
    });
    dispatch_paste(&mut probe);

    probe.cx.update(|_, cx| {
        let value = probe.page.read(cx).editor_value(cx);
        assert!(
            value.contains("mixed text wins"),
            "the text entry is pasted: {value:?}"
        );
        assert!(
            !value.contains("![image]"),
            "the image entry is ignored: {value:?}"
        );
    });
    assert!(
        !probe.home.join("notes/projects/alpha/assets").exists(),
        "mixed paste writes no asset folder"
    );

    let _ = std::fs::remove_dir_all(&probe.home);
}

#[gpui::test]
fn ctrl_f_opens_find_and_selects_the_first_match(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    // main.rs registers this; the probe builds the window directly
    cx.update(|cx| {
        cx.bind_keys([gpui::KeyBinding::new(
            "ctrl-f",
            warp_notes::ui::notes::OpenFind,
            None,
        )]);
    });

    let home = temp_home("warpnotes-probe-find");
    let (handle, page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: None,
            open_note: Some(home.join("notes/projects/alpha/note.md")),
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    // the design's Ctrl+F opens the bar on the open note
    cx.simulate_keystrokes("ctrl-f");
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));
    assert!(
        bounds(cx, "notes-find-bar").is_some(),
        "Ctrl+F opens the find bar"
    );

    // typing walks the editor's text: the first "world" is selected (byte 16)
    cx.simulate_input("world");
    cx.run_until_parked();
    let selected = cx.read_entity(&page, |page, cx| page.editor_selected_range(cx));
    assert_eq!(
        selected,
        16..21,
        "the first match is selected in the editor"
    );

    // Escape closes it and gives the editor the keyboard back
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));
    assert!(
        bounds(cx, "notes-find-bar").is_none(),
        "Escape closes the find bar"
    );
    let _ = std::fs::remove_dir_all(&home);
}

/// A store with thousands of notes must cost what a small one costs: the tree
/// is virtualized, so only the rows in the viewport are laid out. Before the
/// `uniform_list` change every note became an element on every frame.
#[gpui::test]
fn a_large_store_lays_out_only_the_visible_rows(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);

    let home = std::env::temp_dir().join(format!("warpnotes-probe-large-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let notes = home.join("notes/projects/alpha");
    std::fs::create_dir_all(&notes).unwrap();
    point_prefs_at(&home);
    // 2000 notes: comfortably more than a viewport can show
    for ix in 0..2000 {
        std::fs::write(notes.join(format!("note-{ix:04}.md")), "# t\n").unwrap();
    }

    let (handle, _page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    // the first rows are on screen
    assert!(
        bounds(cx, "note-item-note-0000.md").is_some(),
        "the top of the tree renders",
    );
    // and the far end of the list is not laid out at all: with 2000 rows and a
    // 700px window, the virtualized list stops long before the last note
    assert!(
        bounds(cx, "note-item-note-1999.md").is_none(),
        "rows past the viewport are never built",
    );
    let visible = (0..2000)
        .filter(|ix| {
            let selector: &'static str =
                Box::leak(format!("note-item-note-{ix:04}.md").into_boxed_str());
            bounds(cx, selector).is_some()
        })
        .count();
    assert!(
        visible < 60,
        "a 700px pane lays out a screenful, not the store ({visible} rows)",
    );

    let _ = std::fs::remove_dir_all(&home);
}

/// Search lives in the title bar, not the list: the field is the design's one
/// search control, and its results are a popup under it — the tree keeps its
/// scroll and selection while you type. The popup is capped, so a broad term
/// matching the whole store never builds hundreds of rows.
#[gpui::test]
fn search_sits_in_the_titlebar_and_caps_its_popup(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);

    let home = std::env::temp_dir().join(format!("warpnotes-probe-hits-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let notes = home.join("notes/projects/alpha");
    std::fs::create_dir_all(&notes).unwrap();
    point_prefs_at(&home);
    // every file matches the term, by name and by body
    for ix in 0..600 {
        std::fs::write(
            notes.join(format!("shared-{ix:04}.md")),
            format!("# shared {ix}\n\nshared body {ix}\n"),
        )
        .unwrap();
    }

    let (handle, page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    // the search field is in the title bar, and the list pane has none
    let field = bounds(cx, "notes-search-field").expect("the search field is in the title bar");
    // the design puts it centred: two flexible columns, the field between them
    let window_centre = cx.update(|window, _| window.viewport_size().width) / 2.;
    let field_centre = field.origin.x + field.size.width / 2.;
    assert!(
        (field_centre - window_centre).abs() < px(2.),
        "the search field sits on the window's centre \
         (field centre {field_centre:?}, window centre {window_centre:?})",
    );
    assert!(
        bounds(cx, "field-search").is_none(),
        "the list pane no longer carries a search field",
    );

    // with nothing typed there is no popup over the tree
    assert!(
        bounds(cx, "notes-search-popup").is_none(),
        "an empty field draws no popup",
    );

    // the search the page runs on a keystroke, run straight: the field's
    // Change path debounces on a real timer, which a headless probe has no
    // reason to wait out for a result set
    cx.update(|window, cx| {
        let field = page.read(cx).search_handle().clone();
        field.update(cx, |field, cx| field.set_value("shared", window, cx));
        page.update(cx, |page, cx| {
            page.run_search_for_probe("shared".into(), cx)
        });
        drop(window.draw(cx));
    });
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    let hits = cx.read_entity(&page, |page, cx| page.search_hit_count(cx));
    assert_eq!(hits, 600, "the search matched every seeded note");

    // the results are the title bar's popup, capped: a glance, not a list
    let popup = bounds(cx, "notes-search-popup").expect("a term with matches opens the popup");
    let field = bounds(cx, "notes-search-field").expect("the field");
    assert!(
        (popup.origin.x - field.origin.x).abs() < px(2.),
        "the popup hangs off the field's left edge ({popup:?} vs {field:?})",
    );
    assert!(
        (popup.size.width - field.size.width).abs() < px(2.),
        "the popup is the field's width ({popup:?} vs {field:?})",
    );
    assert!(
        popup.origin.y >= field.origin.y + field.size.height - px(1.),
        "the popup sits under the field ({popup:?} vs {field:?})",
    );
    // the tree is still the list underneath, so the pane keeps working
    assert!(
        bounds(cx, "notes-tree-rows").is_some(),
        "the tree stays the list while the popup shows",
    );
    // The store enumerates files in `readdir` order, not by name, so count
    // what the popup laid out rather than naming a row.
    let built = (0..600)
        .filter(|ix| {
            let selector: &'static str =
                Box::leak(format!("notes-search-hit-shared-{ix:04}").into_boxed_str());
            bounds(cx, selector).is_some()
        })
        .count();
    assert!(built > 0, "the popup lists its matches");
    assert!(
        built <= 8,
        "the popup caps its rows rather than building 600 ({built})",
    );

    let _ = std::fs::remove_dir_all(&home);
}

/// The status bar's counts scan the whole note, and the bar re-renders on
/// every caret move — so they are cached against the editor's revision. This
/// pins that: a redraw with no edit reuses the cached counts, and an edit
/// recomputes them.
#[gpui::test]
fn status_counts_are_cached_until_the_text_changes(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-stats");
    let (handle, page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));
    click(cx, "note-item-note.md");

    let (words, chars, lines, revision) = cx.read_entity(&page, |page, _| page.text_stats());
    assert!(words > 0, "the open note was counted");
    assert!(chars >= words, "chars cover the words");
    assert!(lines > 0, "the note has lines");
    let revision = revision.expect("the counts carry the revision they came from");

    // a redraw with nothing typed keeps the cached numbers and revision
    cx.update(|window, cx| drop(window.draw(cx)));
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));
    let (w2, c2, l2, r2) = cx.read_entity(&page, |page, _| page.text_stats());
    assert_eq!((w2, c2, l2), (words, chars, lines), "no edit, no recount");
    assert_eq!(r2, Some(revision), "the cached revision still stands");

    // typing moves the revision, so the next frame recounts
    click(cx, "notes-editor-column");
    cx.simulate_input("one two three four");
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));
    let (w3, _, _, r3) = cx.read_entity(&page, |page, _| page.text_stats());
    assert!(w3 > words, "typed words reach the count ({w3} vs {words})",);
    assert_ne!(r3, Some(revision), "the revision followed the edit");

    let _ = std::fs::remove_dir_all(&home);
}

/// Saving is automatic: a second of typing silence writes the note, and no
/// surface carries a save step. The debounce rides the test clock.
#[gpui::test]
fn edits_autosave_after_the_debounce(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-autosave");
    let (handle, _page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));
    click(cx, "note-item-note.md");

    let note = home.join("notes/projects/alpha/note.md");
    let on_disk = std::fs::read_to_string(&note).unwrap();
    let live = bounds(cx, "notes-live").expect("the live editor renders");
    cx.simulate_click(
        Point::new(
            live.origin.x + live.size.width / 2.,
            live.origin.y + live.size.height / 2.,
        ),
        Modifiers::default(),
    );
    cx.run_until_parked();
    cx.simulate_input("typed without saving");
    cx.run_until_parked();

    // the write waits out the idle second, not the keystroke
    assert_eq!(
        std::fs::read_to_string(&note).unwrap(),
        on_disk,
        "the debounce has not run yet"
    );
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(2));
    cx.run_until_parked();
    let saved = std::fs::read_to_string(&note).unwrap();
    assert!(
        saved.contains("typed without saving"),
        "the idle second wrote the note: {saved:?}"
    );

    let _ = std::fs::remove_dir_all(&home);
}

/// A note changed externally is never overwritten by autosave: the conflict
/// bar offers Reload / Overwrite, and only Overwrite replaces the disk copy
/// (audit A1).
#[gpui::test]
fn an_external_edit_becomes_a_conflict_not_a_silent_overwrite(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-conflict");
    let note = home.join("notes/projects/alpha/note.md");
    let (handle, page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: Some(note.clone()),
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    let live = bounds(cx, "notes-live").expect("the live editor renders");
    cx.simulate_click(
        Point::new(
            live.origin.x + live.size.width / 2.,
            live.origin.y + live.size.height / 2.,
        ),
        Modifiers::default(),
    );
    cx.run_until_parked();
    cx.simulate_input("typed locally");
    cx.run_until_parked();

    // an editor outside the app writes the same note
    std::fs::write(&note, "# Note\n\nEXTERNAL\n").unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(2));
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    assert_eq!(
        std::fs::read_to_string(&note).unwrap(),
        "# Note\n\nEXTERNAL\n",
        "autosave must not overwrite the external copy"
    );
    assert!(
        bounds(cx, "note-conflict").is_some(),
        "the conflict bar must offer a choice"
    );
    assert!(
        page.read_with(cx, |page, cx| page.is_dirty(cx)),
        "the local buffer stays unsaved"
    );

    eprintln!(
        "before overwrite: bounds={:?} disk={:?}",
        bounds(cx, "conflict-overwrite"),
        std::fs::read_to_string(&note)
    );
    click(cx, "conflict-overwrite");
    eprintln!("after overwrite: disk={:?}", std::fs::read_to_string(&note));
    assert!(
        std::fs::read_to_string(&note)
            .unwrap()
            .contains("typed locally"),
        "Overwrite is the deliberate path that replaces the disk copy"
    );
    assert!(bounds(cx, "note-conflict").is_none());
    assert!(!page.read_with(cx, |page, cx| page.is_dirty(cx)));

    // ...and Reload drops the local buffer instead
    let live = bounds(cx, "notes-live").expect("the live editor still renders");
    cx.simulate_click(
        Point::new(
            live.origin.x + live.size.width / 2.,
            live.origin.y + live.size.height / 2.,
        ),
        Modifiers::default(),
    );
    cx.simulate_input(" again");
    cx.run_until_parked();
    std::fs::write(&note, "# Note\n\nEXTERNAL2\n").unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(2));
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));
    assert!(bounds(cx, "note-conflict").is_some());
    click(cx, "conflict-reload");
    assert_eq!(
        std::fs::read_to_string(&note).unwrap(),
        "# Note\n\nEXTERNAL2\n",
        "Reload keeps the disk copy"
    );
    assert!(!page.read_with(cx, |page, cx| page.is_dirty(cx)));
    assert!(bounds(cx, "note-conflict").is_none());

    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
fn window_title_follows_the_open_note(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-title");
    let (handle, _page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    assert_eq!(
        cx.window_title().as_deref(),
        Some("Warp Notes"),
        "a window with no note keeps the app title"
    );
    click(cx, "note-item-note.md");
    assert_eq!(
        cx.window_title().as_deref(),
        Some("note.md — Warp Notes"),
        "the title follows the open note"
    );

    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
fn refresh_sees_an_externally_created_note(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-refresh");
    let (handle, _page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    let notes = home.join("notes/projects/alpha");
    std::fs::write(notes.join("fresh.md"), "# Fresh\n").unwrap();
    assert!(
        bounds(cx, "note-item-fresh.md").is_none(),
        "the tree is stale until refresh"
    );
    click(cx, "notes-refresh");
    assert!(
        bounds(cx, "note-item-fresh.md").is_some(),
        "refresh picks up a note created outside the app"
    );

    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
fn refresh_reloads_a_clean_open_note_and_conflicts_a_dirty_one(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-refresh-conflict");
    let note = home.join("notes/projects/alpha/note.md");
    let (handle, page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: Some(note.clone()),
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    std::fs::write(&note, "# Note\n\nchanged outside\n").unwrap();
    click(cx, "notes-refresh");
    page.read_with(cx, |page, cx| {
        assert!(
            page.editor_value(cx).contains("changed outside"),
            "a clean buffer follows the disk: {:?}",
            page.editor_value(cx)
        );
    });
    assert!(
        bounds(cx, "note-conflict").is_none(),
        "a clean reload is not a conflict"
    );

    let live = bounds(cx, "notes-live").expect("the live editor renders");
    cx.simulate_click(
        Point::new(
            live.origin.x + live.size.width / 2.,
            live.origin.y + live.size.height / 2.,
        ),
        Modifiers::default(),
    );
    cx.run_until_parked();
    cx.simulate_input("typed locally");
    cx.run_until_parked();
    std::fs::write(&note, "# Note\n\nchanged again outside\n").unwrap();
    click(cx, "notes-refresh");
    assert!(
        bounds(cx, "note-conflict").is_some(),
        "a dirty buffer becomes a conflict instead of being overwritten"
    );
    page.read_with(cx, |page, cx| {
        assert!(page.editor_value(cx).contains("typed locally"));
    });

    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
fn rename_updates_disk_editor_and_prefs(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-rename");
    let (handle, page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    click(cx, "note-item-note.md");
    click_str(cx, "note-actions-note.md");
    click(cx, "notes-menu-rename");
    set_prompt(cx, &page, "renamed");
    click(cx, "notes-prompt-confirm");

    let alpha = home.join("notes/projects/alpha");
    assert!(!alpha.join("note.md").exists(), "the old name is gone");
    assert_eq!(
        std::fs::read_to_string(alpha.join("renamed.md")).unwrap(),
        "# Note\n\nHello **world**.\n"
    );
    page.read_with(cx, |page, _| {
        assert_eq!(page.open_note_path(), Some(alpha.join("renamed.md")));
    });
    assert_eq!(
        cx.window_title().as_deref(),
        Some("renamed.md — Warp Notes"),
        "the title follows the renamed note"
    );
    let prefs: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join("settings.json")).unwrap())
            .unwrap();
    assert_eq!(
        prefs["open_note"], "projects/alpha/renamed.md",
        "the remembered note follows the rename"
    );

    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
fn move_note_updates_the_open_path(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-move");
    let alpha = home.join("notes/projects/alpha");
    std::fs::create_dir_all(alpha.join("archive")).unwrap();
    let (handle, page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    click(cx, "note-item-note.md");
    click_str(cx, "note-actions-note.md");
    click(cx, "notes-menu-move");
    set_prompt(cx, &page, "archive");
    click(cx, "notes-prompt-confirm");

    assert!(!alpha.join("note.md").exists());
    assert_eq!(
        std::fs::read_to_string(alpha.join("archive/note.md")).unwrap(),
        "# Note\n\nHello **world**.\n"
    );
    page.read_with(cx, |page, _| {
        assert_eq!(page.open_note_path(), Some(alpha.join("archive/note.md")));
    });
    assert_eq!(cx.window_title().as_deref(), Some("note.md — Warp Notes"));

    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
fn duplicate_opens_a_unique_copy_and_new_folder_expands(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-copy-folder");
    let (handle, page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    click(cx, "note-item-note.md");
    click_str(cx, "note-actions-note.md");
    click(cx, "notes-menu-duplicate");
    let alpha = home.join("notes/projects/alpha");
    assert_eq!(
        std::fs::read_to_string(alpha.join("note copy.md")).unwrap(),
        "# Note\n\nHello **world**.\n"
    );
    page.read_with(cx, |page, _| {
        assert_eq!(page.open_note_path(), Some(alpha.join("note copy.md")));
    });

    click_str(cx, "note-actions-note.md");
    click(cx, "notes-menu-new-folder");
    set_prompt(cx, &page, "ideas");
    click(cx, "notes-prompt-confirm");
    assert!(alpha.join("ideas").is_dir());
    assert!(
        bounds(cx, "note-item-ideas").is_some(),
        "the new folder is visible under the open repo"
    );

    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
fn copy_path_writes_the_store_relative_path(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-copy-path");
    let (handle, _page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    click(cx, "note-item-note.md");
    click_str(cx, "note-actions-note.md");
    click(cx, "notes-menu-copy-path");
    let item = cx.update(|_, cx| cx.read_from_clipboard()).unwrap();
    let text = item.entries.iter().find_map(|entry| match entry {
        gpui::ClipboardEntry::String(text) => Some(text.text.clone()),
        _ => None,
    });
    assert_eq!(text.as_deref(), Some("projects/alpha/note.md"));

    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
fn trash_undo_restores_the_note_and_reopens_it(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-trash-undo");
    let note = home.join("notes/projects/alpha/note.md");
    let (handle, page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    click(cx, "note-item-note.md");
    trash_open_note(cx);
    assert!(!note.exists(), "the live copy is gone");
    page.read_with(cx, |page, _| {
        assert_eq!(page.open_note_path(), None);
    });
    assert_eq!(cx.window_title().as_deref(), Some("Warp Notes"));
    assert!(
        bounds(cx, "notes-toast-action").is_some(),
        "the toast offers Undo"
    );

    click(cx, "notes-toast-action");
    assert_eq!(
        std::fs::read_to_string(&note).unwrap(),
        "# Note\n\nHello **world**.\n"
    );
    page.read_with(cx, |page, _| {
        assert_eq!(page.open_note_path(), Some(note.clone()));
    });
    assert_eq!(cx.window_title().as_deref(), Some("note.md — Warp Notes"));

    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
fn trashing_a_dirty_note_keeps_its_text_for_undo(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-trash-dirty");
    let note = home.join("notes/projects/alpha/note.md");
    let (handle, page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    click(cx, "note-item-note.md");
    let live = bounds(cx, "notes-live").expect("the live editor renders");
    cx.simulate_click(
        Point::new(
            live.origin.x + live.size.width / 2.,
            live.origin.y + live.size.height / 2.,
        ),
        Modifiers::default(),
    );
    cx.run_until_parked();
    cx.simulate_input("typed before trash");
    cx.run_until_parked();
    trash_open_note(cx);
    assert!(!note.exists());

    click(cx, "notes-toast-action");
    page.read_with(cx, |page, cx| {
        assert!(page.editor_value(cx).contains("typed before trash"));
    });
    assert!(
        std::fs::read_to_string(&note)
            .unwrap()
            .contains("typed before trash")
    );

    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
fn trash_dialog_restores_and_empties(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-trash-dialog");
    let alpha = home.join("notes/projects/alpha");
    let (handle, _page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    click(cx, "note-item-note.md");
    trash_open_note(cx);
    click_str(cx, "note-actions-task.md");
    click(cx, "notes-menu-trash");
    assert!(!alpha.join("note.md").exists());
    assert!(!alpha.join("task.md").exists());

    click(cx, "rail-settings");
    click(cx, "settings-trash-open");
    assert!(bounds(cx, "notes-trash-dialog").is_some());
    click_str(cx, "trash-restore-0");
    assert!(alpha.join("task.md").is_file(), "the newest entry is back");

    click(cx, "trash-empty");
    assert!(
        warp_notes::trash::list_trash(&home.join("notes")).is_empty(),
        "Empty Trash clears the rest"
    );
    click(cx, "trash-close");
    assert!(bounds(cx, "notes-trash-dialog").is_none());

    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
fn trashing_a_folder_closes_the_note_inside(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-trash-folder");
    let alpha = home.join("notes/projects/alpha");
    std::fs::create_dir_all(alpha.join("archive")).unwrap();
    std::fs::write(alpha.join("archive/inside.md"), "inside").unwrap();
    let (handle, page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: Some(alpha.join("archive/inside.md")),
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    click_str(cx, "note-actions-archive");
    click(cx, "notes-menu-trash");
    assert!(!alpha.join("archive").exists());
    page.read_with(cx, |page, _| {
        assert_eq!(page.open_note_path(), None);
    });

    click(cx, "notes-toast-action");
    assert!(alpha.join("archive/inside.md").is_file());
    page.read_with(cx, |page, _| {
        assert_eq!(page.open_note_path(), Some(alpha.join("archive/inside.md")));
    });

    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
fn keyboard_tree_moves_and_opens(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-tree-keys");
    let alpha = home.join("notes/projects/alpha");
    let (handle, page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    let focus = page.read_with(cx, |page, _| page.tree_focus_handle());
    cx.update(|window, cx| window.focus(&focus, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("down down enter");
    page.read_with(cx, |page, _| {
        assert_eq!(page.tree_cursor_for_probe(), Some(alpha.join("note.md")));
        assert_eq!(page.open_note_path(), Some(alpha.join("note.md")));
    });

    // Ctrl+Shift+E leaves the editor and returns to the tree, where F2 acts
    // on the cursor row.
    cx.simulate_keystrokes("ctrl-shift-e f2");
    assert!(
        bounds(cx, "notes-prompt-card").is_some(),
        "Ctrl+Shift+E moved focus to the tree"
    );
    click(cx, "notes-prompt-cancel");

    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
fn keyboard_tree_keeps_the_cursor_in_view(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-tree-scroll");
    let alpha = home.join("notes/projects/alpha");
    for index in 0..40 {
        std::fs::write(
            alpha.join(format!("row-{index:02}.md")),
            format!("# Row {index}\n"),
        )
        .unwrap();
    }
    let (handle, page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    let focus = page.read_with(cx, |page, _| page.tree_focus_handle());
    cx.update(|window, cx| window.focus(&focus, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("end");
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    assert!(
        page.read_with(cx, |page, _| page.tree_scroll_offset_for_probe())
            .y
            < px(0.),
        "End scrolls the tree to the cursor row"
    );
    page.read_with(cx, |page, _| {
        assert!(
            page.tree_cursor_for_probe().is_some(),
            "End leaves the cursor on the last row"
        );
    });

    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
fn search_selects_with_arrows(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-search-keys");
    let (handle, page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    cx.update(|window, cx| {
        let field = page.read(cx).search_handle().clone();
        field.update(cx, |field, cx| {
            field.set_value("alpha", window, cx);
            field.focus(window, cx);
        });
        page.update(cx, |page, cx| page.run_search_for_probe("alpha".into(), cx));
        drop(window.draw(cx));
    });
    cx.run_until_parked();
    assert!(
        cx.read_entity(&page, |page, app| page.search_hit_count(app) >= 2),
        "the fixture has two alpha hits"
    );
    cx.simulate_keystrokes("down enter");
    page.read_with(cx, |page, _| {
        assert_eq!(
            page.search_selected_for_probe(),
            0,
            "opening the hit resets the selection"
        );
        assert!(page.open_note_path().is_some(), "Enter opens the hit");
    });

    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
fn shortcut_sheet_opens_and_closes(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-shortcuts");
    let (handle, _page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    click(cx, "note-item-note.md");
    cx.simulate_keystrokes("ctrl-/");
    assert!(bounds(cx, "shortcut-card").is_some());
    click(cx, "shortcut-close");
    assert!(bounds(cx, "shortcut-card").is_none());

    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
fn f2_and_delete_operate_on_the_cursor(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-tree-edit");
    let alpha = home.join("notes/projects/alpha");
    let (handle, page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    let focus = page.read_with(cx, |page, _| page.tree_focus_handle());
    cx.update(|window, cx| window.focus(&focus, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("down down f2");
    assert!(bounds(cx, "notes-prompt-card").is_some(), "F2 opens Rename");
    click(cx, "notes-prompt-cancel");
    cx.update(|window, cx| window.focus(&focus, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("delete");
    assert!(
        !alpha.join("note.md").exists(),
        "Delete trashes the cursor note"
    );
    assert!(bounds(cx, "notes-toast-action").is_some());

    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
fn quick_open_filters_and_opens_a_note(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-palette");
    let alpha = home.join("notes/projects/alpha");
    let (handle, page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    cx.simulate_keystrokes("ctrl-p");
    assert!(
        bounds(cx, "palette-card").is_some(),
        "Ctrl+P opens quick open"
    );
    set_palette(cx, &page, "task");
    click(cx, "palette-row-0");
    page.read_with(cx, |page, _| {
        assert_eq!(page.open_note_path(), Some(alpha.join("task.md")));
    });
    assert!(bounds(cx, "palette-card").is_none());

    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
fn recent_order_survives_a_relaunch(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-recent");
    let (handle, _page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    click(cx, "note-item-note.md");
    click(cx, "note-item-task.md");
    let prefs: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join("settings.json")).unwrap())
            .unwrap();
    assert_eq!(prefs["recent"][0], "projects/alpha/task.md");
    assert_eq!(prefs["recent"][1], "projects/alpha/note.md");

    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
fn pinned_notes_render_and_unpin(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-pin");
    let (handle, page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    click(cx, "note-item-note.md");
    click_str(cx, "note-actions-note.md");
    click(cx, "notes-menu-pin");
    let prefs: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join("settings.json")).unwrap())
            .unwrap();
    assert_eq!(prefs["pinned"][0], "projects/alpha/note.md");

    // the pin shows up as the list head's Pinned scope, not a band over the tree
    click(cx, "notes-list-scope-pinned");
    assert_eq!(
        page.read_with(cx, |page, _| page.list_scope_for_probe()),
        "pinned"
    );
    assert!(bounds(cx, "note-item-note.md").is_some(), "the pinned row");
    assert!(bounds(cx, "notes-pinned-empty").is_none());

    click_str(cx, "note-pin-note.md");
    let prefs: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join("settings.json")).unwrap())
            .unwrap();
    assert_eq!(prefs["pinned"].as_array().map(Vec::len), Some(0));
    assert!(
        bounds(cx, "notes-pinned-empty").is_some(),
        "the empty pinned scope says so"
    );

    click(cx, "notes-list-scope-all");
    assert_eq!(
        page.read_with(cx, |page, _| page.list_scope_for_probe()),
        "all"
    );

    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
fn quick_open_arrows_select_the_next_note(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-palette-arrows");
    let alpha = home.join("notes/projects/alpha");
    let (handle, _page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: None,
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    cx.simulate_keystrokes("ctrl-p down enter");
    assert!(
        bounds(cx, "palette-card").is_none(),
        "Enter closes quick open"
    );
    let opened = cx.read_entity(&_page, |page, _| page.open_note_path());
    assert_eq!(opened, Some(alpha.join("note.md")));

    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
fn quick_open_drops_a_repo_pin_for_another_repo(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-palette-scope");
    let beta = home.join("notes/projects/beta/beta.md");
    let (handle, page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    cx.simulate_keystrokes("ctrl-p");
    set_palette(cx, &page, "beta");
    click(cx, "palette-row-0");
    page.read_with(cx, |page, _| {
        assert_eq!(page.open_note_path(), Some(beta.clone()));
        assert_eq!(
            page.notes_scope_for_probe(),
            None,
            "the repo pin is dropped"
        );
    });
    assert!(
        bounds(cx, "note-item-projects").is_some(),
        "the tree reopened at the whole store"
    );

    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
fn quick_open_skips_the_hidden_app_folder(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-palette-hidden");
    let notes = home.join("notes");
    std::fs::create_dir_all(notes.join(".warp-notes")).unwrap();
    std::fs::write(notes.join(".warp-notes/hidden.md"), "hidden").unwrap();
    let (handle, page) = open_page(
        cx,
        Launch {
            store_root: Some(notes.clone()),
            repo_scope: None,
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    cx.simulate_keystrokes("ctrl-p");
    set_palette(cx, &page, "hidden");
    assert!(
        bounds(cx, "palette-row-0").is_none(),
        "the hidden app folder never reaches quick open"
    );
    click(cx, "palette-scrim");

    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
fn search_finds_notes_in_other_repos_and_drops_the_pin(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-search-all");
    let beta = home.join("notes/projects/beta/beta.md");
    let (handle, page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    cx.update(|window, cx| {
        let field = page.read(cx).search_handle().clone();
        field.update(cx, |field, cx| field.set_value("beta", window, cx));
        page.update(cx, |page, cx| page.run_search_for_probe("beta".into(), cx));
        drop(window.draw(cx));
    });
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));
    assert!(
        cx.read_entity(&page, |page, app| page.search_hit_count(app)) >= 1,
        "a repo-pinned window still searches the whole store"
    );
    click_str(cx, "notes-search-hit-beta");
    page.read_with(cx, |page, _| {
        assert_eq!(page.open_note_path(), Some(beta.clone()));
        assert_eq!(page.notes_scope_for_probe(), None);
    });

    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
fn search_scope_chip_narrows_to_the_pinned_repo(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-search-scope");
    let (handle, _page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    cx.update(|window, cx| {
        let field = _page.read(cx).search_handle().clone();
        field.update(cx, |field, cx| field.set_value("beta", window, cx));
        _page.update(cx, |page, cx| page.run_search_for_probe("beta".into(), cx));
        drop(window.draw(cx));
    });
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));
    assert!(
        bounds(cx, "search-scope-chip").is_some(),
        "the scope chip is in the popup"
    );
    click(cx, "search-scope-chip");
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(300));
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));
    assert_eq!(
        cx.read_entity(&_page, |page, app| page.search_hit_count(app)),
        0,
        "the pinned repo has no beta note"
    );

    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
fn ctrl_shift_f_opens_the_full_results(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-search-shortcut");
    let (handle, page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    cx.update(|window, cx| {
        let field = page.read(cx).search_handle().clone();
        field.update(cx, |field, cx| field.set_value("alpha", window, cx));
        page.update(cx, |page, cx| page.run_search_for_probe("alpha".into(), cx));
        drop(window.draw(cx));
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("ctrl-shift-f");
    assert!(bounds(cx, "search-results-card").is_some());
    click(cx, "search-results-close");
    assert!(bounds(cx, "search-results-card").is_none());

    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
fn full_results_open_beyond_the_popup_cap(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-search-full");
    let notes = home.join("notes/projects/alpha");
    for index in 0..20 {
        std::fs::write(notes.join(format!("shared-{index:02}.md")), "shared body\n").unwrap();
    }
    let (handle, _page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    cx.update(|window, cx| {
        let field = _page.read(cx).search_handle().clone();
        field.update(cx, |field, cx| field.set_value("shared", window, cx));
        _page.update(cx, |page, cx| {
            page.run_search_for_probe("shared".into(), cx)
        });
        drop(window.draw(cx));
    });
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));
    assert!(
        bounds(cx, "notes-search-all").is_some(),
        "the popup offers the full view"
    );
    click(cx, "notes-search-all");
    assert!(bounds(cx, "search-results-card").is_some());
    assert!(
        bounds(cx, "search-result-3").is_some(),
        "the list goes past the popup cap"
    );
    click(cx, "search-results-close");
    assert!(bounds(cx, "search-results-card").is_none());

    let _ = std::fs::remove_dir_all(&home);
}

fn set_store_path(cx: &mut VisualTestContext, page: &gpui::Entity<NotesPage>, value: &str) {
    let input = page.read_with(cx, |page, _| page.store_path_input_handle().clone());
    cx.update(|window, cx| {
        input.update(cx, |field, cx| {
            field.set_value(value.to_string(), window, cx);
            field.focus(window, cx);
        });
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(800));
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));
    cx.update(|window, cx| drop(window.draw(cx)));
}

fn type_editor(cx: &mut VisualTestContext, text: &str) {
    let live = bounds(cx, "notes-live").expect("the live editor renders");
    cx.simulate_click(
        Point::new(
            live.origin.x + live.size.width / 2.,
            live.origin.y + live.size.height / 2.,
        ),
        Modifiers::default(),
    );
    cx.run_until_parked();
    cx.simulate_input(text);
    cx.run_until_parked();
}

#[gpui::test]
fn tabs_keep_their_buffers_and_close(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-tabs");
    let alpha = home.join("notes/projects/alpha");
    let (handle, page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    click(cx, "note-item-note.md");
    type_editor(cx, " first buffer");
    click_str(cx, "note-actions-task.md");
    click(cx, "notes-menu-open-tab");
    type_editor(cx, " second buffer");
    click_str(cx, "note-actions-list.md");
    click(cx, "notes-menu-open-tab");
    assert_eq!(
        page.read_with(cx, |page, _| page.tab_paths_for_probe()),
        vec![
            alpha.join("note.md"),
            alpha.join("task.md"),
            alpha.join("list.md")
        ]
    );
    assert!(bounds(cx, "notes-tab-strip").is_some());

    click(cx, "note-tab-0");
    page.read_with(cx, |page, cx| {
        assert!(page.editor_value(cx).contains("first buffer"));
    });
    click(cx, "note-tab-1");
    page.read_with(cx, |page, cx| {
        assert!(page.editor_value(cx).contains("second buffer"));
    });
    assert_eq!(page.read_with(cx, |page, _| page.active_tab_for_probe()), 1);

    click(cx, "note-tab-close-1");
    assert_eq!(
        page.read_with(cx, |page, _| page.tab_paths_for_probe())
            .len(),
        2
    );
    assert_eq!(
        page.read_with(cx, |page, _| page.open_note_path()),
        Some(alpha.join("list.md"))
    );
    cx.simulate_keystrokes("ctrl-tab");
    assert_eq!(page.read_with(cx, |page, _| page.active_tab_for_probe()), 0);
    cx.simulate_keystrokes("ctrl-shift-tab");
    assert_eq!(page.read_with(cx, |page, _| page.active_tab_for_probe()), 1);

    cx.simulate_keystrokes("ctrl-w ctrl-w");
    assert!(
        page.read_with(cx, |page, _| page.tab_paths_for_probe())
            .is_empty()
    );
    // the strip stays: its home tab names the list
    assert!(bounds(cx, "notes-tab-strip").is_some());
    assert!(bounds(cx, "notes-strip-home").is_some());
    assert!(bounds(cx, "note-tab-0").is_none());
    assert_eq!(cx.window_title().as_deref(), Some("Warp Notes"));

    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
fn a_single_click_previews_and_a_double_click_keeps_the_tab(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-preview-tabs");
    let alpha = home.join("notes/projects/alpha");
    let (handle, page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    // one click: a preview tab, and the next click takes its slot over
    click(cx, "note-item-note.md");
    assert_eq!(
        page.read_with(cx, |page, _| page.tab_paths_for_probe()),
        vec![alpha.join("note.md")]
    );
    assert_eq!(
        page.read_with(cx, |page, _| page.preview_tabs_for_probe()),
        vec![true]
    );
    click(cx, "note-item-task.md");
    assert_eq!(
        page.read_with(cx, |page, _| page.tab_paths_for_probe()),
        vec![alpha.join("task.md")]
    );
    assert_eq!(
        page.read_with(cx, |page, _| page.preview_tabs_for_probe()),
        vec![true]
    );

    // a double click in the list keeps the note it lands on
    double_click(cx, "note-item-list.md");
    assert_eq!(
        page.read_with(cx, |page, _| page.tab_paths_for_probe()),
        vec![alpha.join("list.md")]
    );
    assert_eq!(
        page.read_with(cx, |page, _| page.preview_tabs_for_probe()),
        vec![false]
    );

    // the kept tab stays put; the click plants a preview beside it
    click(cx, "note-item-note.md");
    assert_eq!(
        page.read_with(cx, |page, _| page.tab_paths_for_probe()),
        vec![alpha.join("list.md"), alpha.join("note.md")]
    );
    assert_eq!(
        page.read_with(cx, |page, _| page.preview_tabs_for_probe()),
        vec![false, true]
    );
    assert_eq!(page.read_with(cx, |page, _| page.active_tab_for_probe()), 1);

    // the app keeps one preview slot: with the kept tab active, the click
    // takes over the preview tab that is already there
    click(cx, "note-tab-0");
    click(cx, "note-item-task.md");
    assert_eq!(
        page.read_with(cx, |page, _| page.tab_paths_for_probe()),
        vec![alpha.join("list.md"), alpha.join("task.md")]
    );
    assert_eq!(
        page.read_with(cx, |page, _| page.preview_tabs_for_probe()),
        vec![false, true]
    );
    assert_eq!(page.read_with(cx, |page, _| page.active_tab_for_probe()), 1);

    // double-clicking the tab keeps it too
    double_click(cx, "note-tab-1");
    assert_eq!(
        page.read_with(cx, |page, _| page.preview_tabs_for_probe()),
        vec![false, false]
    );

    // the middle button always opens a tab of its own
    middle_click(cx, "note-item-note.md");
    assert_eq!(
        page.read_with(cx, |page, _| page.tab_paths_for_probe()),
        vec![
            alpha.join("list.md"),
            alpha.join("task.md"),
            alpha.join("note.md")
        ]
    );
    assert_eq!(
        page.read_with(cx, |page, _| page.preview_tabs_for_probe()),
        vec![false, false, false]
    );
    assert_eq!(
        page.read_with(cx, |page, _| page.open_note_path()),
        Some(alpha.join("note.md"))
    );

    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
fn ctrl_enter_opens_quick_open_in_a_new_tab(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-tabs-palette");
    let alpha = home.join("notes/projects/alpha");
    let (handle, page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    click(cx, "note-item-note.md");
    cx.simulate_keystrokes("ctrl-p");
    set_palette(cx, &page, "task");
    cx.simulate_keystrokes("ctrl-enter");
    assert_eq!(
        page.read_with(cx, |page, _| page.tab_paths_for_probe()),
        vec![alpha.join("note.md"), alpha.join("task.md")]
    );
    assert_eq!(
        page.read_with(cx, |page, _| page.open_note_path()),
        Some(alpha.join("task.md"))
    );

    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
fn back_forward_walks_the_open_history(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-history");
    let alpha = home.join("notes/projects/alpha");
    let (handle, page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    click(cx, "note-item-note.md");
    click(cx, "note-item-task.md");
    click(cx, "note-item-list.md");
    cx.simulate_keystrokes("alt-left");
    assert_eq!(
        page.read_with(cx, |page, _| page.open_note_path()),
        Some(alpha.join("task.md"))
    );
    cx.simulate_keystrokes("alt-left");
    assert_eq!(
        page.read_with(cx, |page, _| page.open_note_path()),
        Some(alpha.join("note.md"))
    );
    cx.simulate_keystrokes("alt-right");
    assert_eq!(
        page.read_with(cx, |page, _| page.open_note_path()),
        Some(alpha.join("task.md"))
    );

    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
fn a_toast_floats_without_shrinking_the_page(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-toast-float");
    let (handle, _page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    click(cx, "note-item-note.md");
    let before = bounds(cx, "notes-editor-column").expect("the editor column");
    click_str(cx, "note-actions-note.md");
    click(cx, "notes-menu-pin");
    let after = bounds(cx, "notes-editor-column").expect("the editor column");
    assert_eq!(
        before, after,
        "the floating toast must not join the root flex column"
    );
    assert!(bounds(cx, "notes-toast").is_some());

    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
fn store_path_setting_is_editable_and_applies(cx: &mut TestAppContext) {
    let _guard = hold_home();
    let home = temp_home("warpnotes-probe-store-path");
    let _home = HomeSwap::set(&home);
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let (handle, page) = open_page(
        cx,
        Launch {
            store_root: None,
            repo_scope: None,
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    click(cx, "rail-settings");
    assert!(
        bounds(cx, "field-notes-store").is_some(),
        "the store is editable"
    );
    let vault = home.join("vault");
    set_store_path(cx, &page, vault.to_string_lossy().as_ref());
    page.read_with(cx, |page, _| {
        assert_eq!(page.store_root_for_probe(), vault);
    });
    assert!(vault.is_dir(), "the chosen store is created");
    let prefs: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join("settings.json")).unwrap())
            .unwrap();
    assert_eq!(prefs["store_path"], vault.to_string_lossy().as_ref());

    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
fn a_store_launched_with_store_is_read_only(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-store-pinned");
    let (handle, _page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: None,
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    click(cx, "rail-settings");
    assert!(
        bounds(cx, "field-notes-store").is_none(),
        "--store wins; the path is shown read-only"
    );

    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
fn outline_lists_headings_and_jumps(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-outline");
    let (handle, page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    click(cx, "note-item-note.md");
    click(cx, "notes-context-toggle");
    assert!(bounds(cx, "notes-context-panel").is_some());
    assert!(bounds(cx, "context-heading-0").is_some());
    click(cx, "context-heading-0");
    page.read_with(cx, |page, cx| {
        assert_eq!(page.editor_selected_range(cx).start, 2);
    });
    click(cx, "context-close");
    assert!(bounds(cx, "notes-context-panel").is_none());

    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
fn backlinks_list_the_linking_note(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-backlinks");
    let alpha = home.join("notes/projects/alpha");
    let beta = home.join("notes/projects/beta/beta.md");
    std::fs::write(alpha.join("note.md"), "# Note\n\nsee [[beta]]\n").unwrap();
    let (handle, page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: None,
            open_note: Some(beta.clone()),
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    click(cx, "notes-context-toggle");
    click(cx, "context-tab-links");
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));
    assert!(
        bounds(cx, "context-link-0").is_some(),
        "the alpha note links to beta"
    );
    click(cx, "context-link-0");
    page.read_with(cx, |page, _| {
        assert_eq!(page.open_note_path(), Some(alpha.join("note.md")));
    });

    let _ = std::fs::remove_dir_all(&home);
}

/// Everything the user arranges comes back on relaunch: the open note, the
/// tree's expanded folders, the hidden list pane, the rail's section and the
/// scope the up button reached.
#[gpui::test]
fn a_relaunch_restores_the_arranged_ui(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-restore");
    let launch = Launch {
        store_root: Some(home.join("notes")),
        repo_scope: Some("alpha".into()),
        open_note: None,
        whole_store: false,
    };

    // arrange: up to the whole store, expand alpha, open a note, hide the
    // list, move the rail to Settings
    let (handle, _page) = open_page(cx, launch.clone());
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));
    click(cx, "notes-up-dir");
    click(cx, "note-item-projects");
    click(cx, "note-item-alpha");
    click(cx, "note-item-note.md");
    click(cx, "notes-list-toggle");
    click(cx, "rail-settings");
    cx.update(|window, cx| drop(window.draw(cx)));

    // relaunch: no --note and no --repo pin, and the arranged state comes
    // back — a --repo launch would pin the scope again by contract
    let (handle, _page) = open_page(
        cx,
        Launch {
            store_root: launch.store_root.clone(),
            repo_scope: None,
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    // the Settings screen came back, so the note's chrome is not on screen
    assert!(
        bounds(cx, "text-size-normal").is_some(),
        "the rail reopens on Settings"
    );

    // back on Notes: the open note, the whole-store scope and the hidden
    // list pane all came back with it
    click(cx, "rail-notes");
    assert!(
        bounds(cx, "notes-toolbar").is_some(),
        "the last open note is open again"
    );
    assert!(
        bounds(cx, "notes-sidebar").is_none(),
        "the hidden list pane stays hidden"
    );
    assert!(
        bounds(cx, "notes-up-dir").is_none(),
        "the restored scope is the store root, so there is no up button"
    );

    // showing the list again reveals the whole-store scope and the
    // remembered expansion
    click(cx, "notes-list-toggle");
    assert!(
        bounds(cx, "notes-sidebar").is_some(),
        "the toggle brings the list back"
    );
    assert!(
        bounds(cx, "note-item-beta").is_some(),
        "the up button's whole-store scope comes back"
    );
    assert!(
        bounds(cx, "note-item-task.md").is_some(),
        "the expanded folder stayed expanded"
    );

    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
fn tree_rows_fill_the_pane(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-row-width");
    seed_notes(&home);
    let (handle, _page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    let sidebar = bounds(cx, "notes-sidebar").expect("the list pane");
    let row = bounds(cx, "note-item-note.md").expect("the note row");
    // the list's hairline border is the one pixel the row does not own
    let inner = sidebar.size.width - px(1.);
    assert!(
        (row.size.width - inner).abs() <= px(2.),
        "the row must span the pane: row {} vs inner {}",
        row.size.width,
        inner
    );
    assert_eq!(
        row.size.height,
        px(theme::scaled(warp_notes::ui::notes::TREE_ROW_HEIGHT)),
        "the family's row height"
    );

    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
fn folder_bands_carry_their_note_count(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-band-count");
    seed_notes(&home);
    std::fs::create_dir_all(home.join("notes/projects/alpha/sub")).unwrap();
    std::fs::write(home.join("notes/projects/alpha/sub/one.md"), "# One\n").unwrap();
    std::fs::write(home.join("notes/projects/alpha/sub/two.md"), "# Two\n").unwrap();
    let (handle, page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    assert_eq!(
        page.read_with(cx, |page, _| page.band_count_for_probe("sub")),
        Some(2)
    );
    assert!(
        bounds(cx, "note-item-sub").is_some(),
        "the folder band renders"
    );

    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
fn list_head_counts_notes_and_pins(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-head-counts");
    seed_notes(&home);
    let (handle, page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    assert!(bounds(cx, "notes-list-summary").is_some());
    assert_eq!(
        page.read_with(cx, |page, _| page.list_summary_for_probe()),
        "3 notes · 0 pinned"
    );

    click(cx, "note-item-note.md");
    click_str(cx, "note-actions-note.md");
    click(cx, "notes-menu-pin");
    assert_eq!(
        page.read_with(cx, |page, _| page.list_summary_for_probe()),
        "3 notes · 1 pinned"
    );

    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
fn the_strip_sits_above_both_panes(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-strip");
    seed_notes(&home);
    let (handle, _page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    let strip = bounds(cx, "notes-tab-strip").expect("the strip is always there");
    let sidebar = bounds(cx, "notes-sidebar").expect("the list pane");
    let detail = bounds(cx, "notes-detail").expect("the detail pane");
    assert!(
        strip.origin.y + strip.size.height <= sidebar.origin.y + px(1.),
        "the strip sits above the panes"
    );
    assert!(
        (strip.origin.x - sidebar.origin.x).abs() <= px(1.),
        "the strip starts with the list"
    );
    assert!(
        strip.origin.x + strip.size.width >= detail.origin.x + detail.size.width - px(1.),
        "the strip spans the stage"
    );

    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
fn the_home_tab_focuses_the_tree(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-home-tab");
    seed_notes(&home);
    let (handle, page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    click(cx, "note-item-note.md");
    click(cx, "notes-strip-home");
    let tree_focus = page.read_with(cx, |page, _| page.tree_focus_handle());
    assert_eq!(
        cx.update(|window, cx| window.focused(cx)),
        Some(tree_focus),
        "the home tab hands the keyboard to the tree"
    );

    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
fn the_head_leads_with_the_note_title(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-head");
    seed_notes(&home);
    let (handle, _page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    click(cx, "note-item-note.md");
    let pane = bounds(cx, "notes-detail").expect("the detail pane");
    let title = bounds(cx, "note-title").expect("the note title");
    assert!(
        title.origin.x >= pane.origin.x + px(24.),
        "the head starts at the detail padding"
    );
    assert!(
        title.size.height > px(20.),
        "the title is set at the screen-title step"
    );
    assert!(bounds(cx, "note-verbs").is_some(), "the verb cluster");
    assert!(
        bounds(cx, "note-trash").is_none(),
        "Trash moved into the verb menu"
    );
    assert!(bounds(cx, "note-pin-toggle").is_some());

    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
fn the_verb_menu_trashes_the_note(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-verbs");
    seed_notes(&home);
    let note = home.join("notes/projects/alpha/note.md");
    let (handle, page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    click(cx, "note-item-note.md");
    click(cx, "note-verbs");
    assert!(
        bounds(cx, "notes-row-menu").is_some(),
        "the verb menu opens"
    );
    click(cx, "notes-menu-trash");
    assert!(!note.exists(), "the note went to Trash");
    assert_eq!(page.read_with(cx, |page, _| page.open_note_path()), None);
    assert!(
        bounds(cx, "notes-toast-action").is_some(),
        "Undo is offered"
    );

    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
fn empty_state_offers_a_new_note(cx: &mut TestAppContext) {
    let _guard = hold_home();
    cx.update(gpui_kit::init);
    cx.update(theme::apply_kit_theme);
    let home = temp_home("warpnotes-probe-empty");
    seed_notes(&home);
    let (handle, _page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join("notes")),
            repo_scope: Some("alpha".into()),
            open_note: None,
            whole_store: false,
        },
    );
    let cx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    cx.run_until_parked();
    cx.update(|window, cx| drop(window.draw(cx)));

    assert!(bounds(cx, "notes-empty").is_some(), "nothing is open");
    click(cx, "notes-empty-new");
    assert!(
        bounds(cx, "note-create-form").is_some(),
        "the empty state starts the new-note form"
    );

    let _ = std::fs::remove_dir_all(&home);
}
