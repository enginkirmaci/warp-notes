//! Headless probe for the Notes window: the carded sidebar, opening a note
//! from the tree, the single Obsidian-style live editor, task checkboxes
//! toggling on click, Enter continuing lists, wikilinks leading to notes, and
//! the launch contract (--repo pins the scope, --note opens a note right away).

use warp_notes::theme;
use warp_notes::ui::notes::{Launch, NotesPage};

use gpui::{
    AppContext, Bounds, Modifiers, Pixels, Point, TestAppContext, VisualTestContext, px, size,
};
use std::path::PathBuf;

fn seed_notes(home: &std::path::Path) {
    let notes = home.join(".warp/notes/alpha");
    std::fs::create_dir_all(&notes).unwrap();
    std::fs::write(notes.join("note.md"), "# Note\n\nHello **world**.\n").unwrap();
    std::fs::write(notes.join("task.md"), "- [ ] alpha task\nplain\n").unwrap();
    std::fs::write(notes.join("list.md"), "- alpha\n").unwrap();
    let beta = home.join(".warp/notes/beta");
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

fn temp_home(prefix: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!("{prefix}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    seed_notes(&home);
    home
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
            store_root: Some(home.join(".warp/notes")),
            repo_scope: Some("alpha".into()),
            open_note: None,
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

    // up leaves the repo folder for the whole store — one folder per repo
    click(cx, "notes-up-dir");
    assert!(
        bounds(cx, "note-item-alpha").is_some(),
        "up shows the store's repo folders"
    );
    assert!(
        bounds(cx, "note-item-beta").is_some(),
        "up shows every repo folder"
    );
    assert!(
        bounds(cx, "notes-up-dir").is_none(),
        "store root has no up button"
    );

    // open the folder and then a note from the tree
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
            store_root: Some(home.join(".warp/notes")),
            repo_scope: Some("alpha".into()),
            open_note: None,
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
    let notes = home.join(".warp/notes/alpha");
    std::fs::write(
        notes.join("links.md"),
        "see [[list]] and [[Brand New]]\n\nend",
    )
    .unwrap();
    let (handle, page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join(".warp/notes")),
            repo_scope: Some("alpha".into()),
            open_note: Some(PathBuf::from("alpha/links.md")),
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
    // --note beta/beta.md (store-relative): opens right away, no scope pin
    let (handle, page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join(".warp/notes")),
            repo_scope: None,
            open_note: Some(PathBuf::from("beta/beta.md")),
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
    // whole-store root: both repo folders are siblings in the tree
    assert!(
        bounds(cx, "note-item-alpha").is_some(),
        "a --note launch without --repo roots at the whole store",
    );
    assert!(
        bounds(cx, "notes-toolbar").is_some(),
        "the note header strip is up"
    );

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
            store_root: Some(home.join(".warp/notes")),
            repo_scope: Some("alpha".into()),
            open_note: None,
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

    let assets = probe.home.join(".warp/notes/alpha/assets");
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
        !probe.home.join(".warp/notes/alpha/assets").exists(),
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
        !probe.home.join(".warp/notes/alpha/assets").exists(),
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
            store_root: Some(home.join(".warp/notes")),
            repo_scope: None,
            open_note: Some(home.join(".warp/notes/alpha/note.md")),
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
    let notes = home.join(".warp/notes/alpha");
    std::fs::create_dir_all(&notes).unwrap();
    // 2000 notes: comfortably more than a viewport can show
    for ix in 0..2000 {
        std::fs::write(notes.join(format!("note-{ix:04}.md")), "# t\n").unwrap();
    }

    let (handle, _page) = open_page(
        cx,
        Launch {
            store_root: Some(home.join(".warp/notes")),
            repo_scope: Some("alpha".into()),
            open_note: None,
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
