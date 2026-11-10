//! The Notes window: the shell's rail, note tree list and one live editor in
//! the detail pane. Per-repo markdown notes — Markdown renders in place while
//! typing: source syntax stays hidden until the caret touches it, task
//! checkboxes toggle on click, Enter continues lists, links open, and the
//! format bar drives the pure markdown editing ops. Dirty in-memory buffers
//! survive navigation, and search is capped.

use crate::notes_store as notes_core;
use crate::theme;
use crate::ui::A11y;
use crate::ui::icon;
use crate::ui::settings::SettingsSection;
use crate::ui_prefs::UiPrefs;
use gpui::prelude::*;
use gpui::{
    App, Context, Entity, Focusable as _, FontWeight, MouseButton, Pixels, ScrollEvent,
    ScrollHandle, SharedString, UniformListScrollHandle, WeakEntity, Window, WindowControlArea,
    canvas, div, px, uniform_list,
};
use gpui_kit::component::input::{InputEvent, InputState};
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use warpui_kit_core::{motion, shell};
use warpui_kit_markdown::live::{InlineObjectKind, LiveLineKind};
use warpui_kit_markdown::{
    LiveEditor, LiveEditorEvent, LiveEditorState, apply, note_key, parse_document,
};

// Ctrl+F: open the editor's find bar (bound in main.rs).
gpui::actions!(warp_notes, [OpenFind]);

/// The splitter between the list and the detail.
const SPLITTER_WIDTH: f32 = 4.;

/// How long the editor stays idle before a note saves itself. Long enough
/// that a burst of typing writes once, short enough that a crash or a yanked
/// power cord loses at most the last second.
const AUTOSAVE_DELAY: std::time::Duration = std::time::Duration::from_secs(1);

/// The rail's sections.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Section {
    Notes,
    About,
    Settings,
}

/// What the list pane shows: the whole tree, or just the pinned notes. Per
/// window, like the tree cursor — two windows do not fight over one scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ListScope {
    All,
    Pinned,
}

/// The launch configuration the CLI parsed: the store root (Warp Station
/// passes the resolved `notes_store_path`; standalone defaults to ~/notes),
/// an optional pinned repo scope (that repo's folder under `projects/`), an
/// optional note to open right away, and whether the launch asked for the
/// whole store outright (`--root`).
#[derive(Clone, Debug, Default)]
pub struct Launch {
    pub store_root: Option<PathBuf>,
    pub repo_scope: Option<String>,
    pub open_note: Option<PathBuf>,
    /// Open the whole store, overriding both a `--repo` pin and the scope the
    /// last window remembered. Warp Station's titlebar sends this.
    pub whole_store: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NoticeKind {
    Info,
    Error,
}

#[derive(Clone)]
struct Notice {
    kind: NoticeKind,
    text: String,
    expires: std::time::Instant,
    action: Option<NoticeAction>,
    /// A counter that moves with every new toast: the entrance animation keys
    /// on it, so a second toast plays its own entrance instead of inheriting
    /// the first one's finished animation (which reads as "no animation").
    seq: u64,
}

#[derive(Clone)]
enum NoticeAction {
    UndoTrash {
        entry: crate::trash::TrashEntry,
        reopen: Option<PathBuf>,
    },
}

/// An info toast leaves on its own; an error stays longer and carries a close.
fn notice_ttl(kind: NoticeKind) -> std::time::Duration {
    std::time::Duration::from_secs(match kind {
        NoticeKind::Info => 4,
        NoticeKind::Error => 8,
    })
}

/// Tooltips always show in the standalone window (DevTools gates its own
/// embedded surfaces by a setting; this app has none).
/// One tree row: fixed, so `uniform_list` can size the whole list.
pub const TREE_ROW_HEIGHT: f32 = 44.;

/// The title bar's search field: the design's `min(460px, 36vw)` measure,
/// fixed so the popup under it can line up with its edges.
const SEARCH_W: f32 = 460.;

/// How many results the popup lists before it says how many are left. A popup
/// is a glance; the tree is the way to everything.
const SEARCH_POPUP_ROWS: usize = 8;

/// The full results view is a list, but a 2000-hit store must not lay out
/// every row at once.
const SEARCH_RESULTS_ROWS: usize = 200;

const SHOW_TOOLTIPS: bool = true;

const SHORTCUTS: &[(&str, &str)] = &[
    ("Ctrl+P", "Quick open"),
    ("Ctrl+K", "Search notes"),
    ("Ctrl+Shift+F", "Full search results"),
    ("Ctrl+Shift+B", "Outline and links"),
    ("Ctrl+Shift+E", "Focus the tree"),
    ("Up / Down", "Move the tree cursor"),
    ("Left / Right", "Collapse / expand"),
    ("Enter", "Open the selected entry"),
    ("F2", "Rename"),
    ("Delete", "Move to Trash"),
    ("Ctrl+D", "Duplicate"),
    ("Ctrl+S", "Save now"),
    ("Ctrl+N", "New note"),
    ("Ctrl+\\", "Toggle the list"),
    ("Ctrl+W", "Close the active tab"),
    ("Ctrl+Tab", "Next tab"),
    ("Alt+Left / Right", "Back / forward"),
    ("Ctrl+/", "This cheat sheet"),
];

/// What the status bar counts over the open note. Computed once per document
/// revision, not once per frame: the scan is the length of the note.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct TextStats {
    words: usize,
    chars: usize,
    lines: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PromptKind {
    Rename,
    NewFolder,
    MoveTo,
}

impl PromptKind {
    fn title(self) -> &'static str {
        match self {
            PromptKind::Rename => "Rename",
            PromptKind::NewFolder => "New folder",
            PromptKind::MoveTo => "Move to folder",
        }
    }

    fn confirm(self) -> &'static str {
        match self {
            PromptKind::Rename => "Rename",
            PromptKind::NewFolder => "Create folder",
            PromptKind::MoveTo => "Move",
        }
    }
}

#[derive(Clone)]
struct PromptState {
    kind: PromptKind,
    target: PathBuf,
}

#[derive(Clone)]
struct RowMenu {
    path: PathBuf,
    is_folder: bool,
    x: f32,
    y: f32,
}

#[derive(Clone, Debug)]
struct PaletteItem {
    path: PathBuf,
    title: String,
    relative: String,
}

#[derive(Clone, Debug)]
struct OpenTab {
    path: PathBuf,
    saved_text: String,
    saved_meta: Option<notes_core::FileMeta>,
    conflict: bool,
    /// VS Code's preview tab: a single click in the list lands here and the
    /// next single click takes the slot over. Opening the tab deliberately
    /// (double click, menu, link, launch) keeps it.
    preview: bool,
}

/// How an open lands in the strip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OpenMode {
    /// A single click in the note list: a preview tab, replaced by the next
    /// single click unless it was kept.
    Preview,
    /// Navigation — quick open, search, a link, history, a launch: it takes
    /// the active tab's slot, the app's one-tab-at-a-time behavior.
    Reuse,
    /// A deliberate new tab: appended after the others, permanently.
    New,
}

/// One tab's strip state, read out before the borrow of `self` ends.
struct TabChip {
    title: String,
    active: bool,
    dirty: bool,
    preview: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ContextTab {
    Outline,
    Links,
}

#[derive(Clone, Debug)]
struct Heading {
    level: u8,
    label: String,
    byte: usize,
}

#[derive(Clone, Debug)]
struct Backlink {
    path: PathBuf,
    title: String,
    relative: String,
}

pub struct NotesPage {
    /// The notes store root: --store when given, else ~/notes.
    pub(crate) store_root: PathBuf,
    /// True when the launch passed `--store` (Warp Station or CLI); the
    /// settings row is read-only then because the next launch wins.
    pub(crate) store_pinned: bool,
    pub(crate) store_path_input: Entity<InputState>,
    store_path_generation: u32,
    /// A debounced store-path edit waiting for the next render, where the
    /// window is available to rebuild the shell.
    store_path_pending: Option<PathBuf>,
    /// The tree's pinned scope: None roots at the whole notes store, Some at
    /// one repo's folder under `projects/`. Fixed after launch except through
    /// the titlebar's up button.
    notes_scope: Option<String>,
    /// The rail's open section: the tree + editor, About or Settings.
    section: Section,
    /// About: which suite member the detail shows; Notes until another is
    /// picked.
    pub(crate) about_selected: warpui_kit_core::about::AppId,
    /// The license card is up over the About section.
    pub(crate) about_license_open: bool,
    /// The license document, built the first time the card opens: read-only
    /// markdown, so the GPL reads as prose and its text is selectable.
    pub(crate) about_license: Option<gpui::Entity<warpui_kit_markdown::LiveEditorState>>,
    /// The Settings section's own section: General or Appearance.
    pub(crate) settings_section: SettingsSection,
    /// The remembered window bits: list width and reduce motion.
    pub(crate) prefs: UiPrefs,
    /// Ctrl+\ and the title bar's toggle hide the list pane.
    list_visible: bool,
    /// The list pane's scope: the tree, or the pinned notes.
    list_scope: ListScope,
    /// active list resize: (start pointer x, start list width); shared with
    /// the paint-phase listener registration because window.on_mouse_event
    /// may only be called during paint in this gpui rev
    splitter_drag: Rc<Cell<Option<(Pixels, f32)>>>,
    /// The tree list's scroll handle. The wheel barrier reads the offsets so
    /// it consumes only the events the list actually answers — the outer
    /// scrollable must not also scroll on the same event.
    tree_scroll: UniformListScrollHandle,
    /// The tree's keyboard focus and selected row, separate from the open
    /// note: moving the cursor does not replace the editor.
    tree_focus: gpui::FocusHandle,
    tree_cursor: Option<PathBuf>,
    /// The window root's focus, so global keys work before a note is open.
    page_focus: gpui::FocusHandle,
    /// The settings pane's scroll handle, same job as `tree_scroll`.
    pub(crate) settings_scroll: ScrollHandle,
    /// The loaded tree behind an Rc: every editor keystroke re-renders the
    /// page, and the rebuild must not deep-clone the whole tree per frame.
    tree: Rc<Vec<Rc<notes_core::NotesTreeItem>>>,
    tree_loaded_for: Option<PathBuf>,
    /// The tree changed: the editor's wikilinks need the new note names.
    note_names_stale: bool,
    expanded: std::collections::HashSet<PathBuf>,
    open_note: Option<PathBuf>,
    /// Per-window tabs. The active tab mirrors `open_note`, `saved_text`,
    /// `saved_meta` and `conflict`; the others keep their own baseline.
    open_tabs: Vec<OpenTab>,
    active_tab: usize,
    /// Back/forward history over the open sequence (Alt+Left/Right).
    history: Vec<PathBuf>,
    history_index: usize,
    /// Unsaved note contents by path — navigation never loses an edit.
    dirty: HashMap<PathBuf, String>,
    /// The on-disk contents of the open note; comparing against the editor
    /// makes the unsaved marker follow every keystroke.
    saved_text: String,
    /// The on-disk identity at the last read or write. A write whose file no
    /// longer matches this is a conflict, not a save.
    saved_meta: Option<notes_core::FileMeta>,
    /// The open note changed on disk: autosave is paused and the editor head
    /// offers Reload / Overwrite.
    conflict: bool,
    /// Every path whose parked buffer conflicted with the disk, so reopening
    /// it keeps the conflict instead of autosaving the parked text.
    conflicts: HashSet<PathBuf>,
    /// The status bar's word/char/line counts and the editor revision they
    /// were computed from: the scan is the length of the note, and the bar
    /// re-renders on every caret move.
    text_stats: TextStats,
    stats_revision: Option<u64>,
    /// The head's "Edited 5m" label and when it was read: a `stat` syscall
    /// per frame is not worth a string that changes once a minute.
    age_cache: Option<(PathBuf, String, std::time::Instant)>,
    /// The title bar search field's painted box (x, width), captured during
    /// paint: the results popup hangs off it, and the field is not at the
    /// window's centre (the brand and app links sit to its left).
    search_field_box: Rc<Cell<Option<(f32, f32)>>>,
    search: Entity<InputState>,
    search_hits: Vec<notes_core::NotesSearchHit>,
    /// The search popup's keyboard selection.
    search_selected: usize,
    /// Search every note in the store, or only the pinned repo's tree.
    search_all_notes: bool,
    /// The full results view opened from the popup or Ctrl+Shift+F.
    search_results_open: bool,
    /// The right-hand Outline/Backlinks panel.
    context_open: bool,
    context_tab: ContextTab,
    outline_cache: Option<(u64, Vec<Heading>)>,
    backlinks: Vec<Backlink>,
    backlinks_for: Option<String>,
    backlinks_generation: u32,
    backlinks_building: bool,
    /// Debounce sequence; a keystroke invalidates any in-flight search.
    search_generation: u32,
    /// Ctrl+F in the editor: the find bar, its byte ranges and the current hit.
    find_open: bool,
    find: Entity<InputState>,
    find_hits: Vec<std::ops::Range<usize>>,
    find_index: usize,
    /// Ctrl+/ shortcut cheat sheet.
    shortcut_open: bool,
    /// The live markdown editor.
    editor: Entity<LiveEditorState>,
    /// The debounced autosave: every edit bumps this, and only the task from
    /// the last edit still holding the epoch writes the note.
    save_epoch: u64,
    /// The "+ note" create form (name + optional folder).
    create_open: bool,
    create_name: Entity<InputState>,
    create_folder: Entity<InputState>,
    /// The rename / new-folder / move prompt's field and target.
    prompt: Option<PromptState>,
    prompt_input: Entity<InputState>,
    /// The row action menu, anchored where the pointer opened it.
    row_menu: Option<RowMenu>,
    /// The Trash dialog: list, restore, empty.
    trash_open: bool,
    /// Ctrl+P quick open over the whole store.
    palette_open: bool,
    palette_input: Entity<InputState>,
    palette_items: Vec<PaletteItem>,
    palette_selected: usize,
    /// The transient toast: save/create/delete outcomes, bottom right.
    notice: Option<Notice>,
    /// The next toast's sequence number.
    notice_seq: u64,
    /// A left press landed on the titlebar background; the first move turns
    /// it into a window drag (Linux/macOS — Windows uses the HTCAPTION path).
    titlebar_should_move: bool,
}

impl NotesPage {
    pub fn new(launch: Launch, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut prefs = UiPrefs::load();
        let store_pinned = launch.store_root.is_some();
        let store_root = match launch.store_root {
            Some(root) => root,
            None => prefs
                .store_path
                .as_deref()
                .map(PathBuf::from)
                .unwrap_or_else(crate::paths::default_store_root),
        };
        let store_root = if store_root.is_absolute() {
            store_root
        } else {
            std::path::absolute(&store_root).unwrap_or(store_root)
        };
        let _ = std::fs::create_dir_all(&store_root);
        crate::trash::purge_trash(&store_root, crate::trash::MAX_AGE);
        let store_path_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(store_root.display().to_string())
                .placeholder(crate::paths::default_store_root().display().to_string())
        });
        let notes_scope = launch.repo_scope;
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search all notes…"));
        let editor = cx.new(|cx| LiveEditorState::new(window, cx).placeholder("# Title"));
        let find = cx.new(|cx| InputState::new(window, cx).placeholder("Find in note…"));
        cx.subscribe(&find, |this, field, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.run_find(field.read(cx).value().to_string(), cx);
            }
        })
        .detach();
        let create_name = cx.new(|cx| InputState::new(window, cx).placeholder("New note name"));
        let create_folder =
            cx.new(|cx| InputState::new(window, cx).placeholder("Folder (optional)"));
        let prompt_input = cx.new(|cx| InputState::new(window, cx).placeholder("Name"));
        let palette_input = cx.new(|cx| InputState::new(window, cx).placeholder("Open a note…"));
        cx.subscribe(&palette_input, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.palette_selected = 0;
                cx.notify();
            }
        })
        .detach();
        cx.subscribe(&store_path_input, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.schedule_store_path(cx);
            }
        })
        .detach();
        cx.subscribe(&search, |this, field, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.run_search(field.read(cx).value().to_string(), cx);
            }
        })
        .detach();
        // rendered links lead out of the editor: the browser, or another note;
        // an edit re-arms the autosave debounce
        cx.subscribe_in(
            &editor,
            window,
            |this, _, event: &LiveEditorEvent, window, cx| match event {
                LiveEditorEvent::OpenUrl(url) => cx.open_url(url),
                LiveEditorEvent::OpenNote(target) => this.follow_link(target, window, cx),
                LiveEditorEvent::Change => this.schedule_autosave(cx),
            },
        )
        .detach();
        // the status strip and unsaved marker follow every keystroke and
        // every caret move
        cx.observe(&editor, |_, _, cx| cx.notify()).detach();
        // the remembered bits: the layout, and the motion choice (applied
        // before the first frame so nothing animates that should not)
        // --root asks for the whole store outright: it beats a --repo pin and
        // the remembered one, and dropping the pin keeps the next bare launch
        // at the root too
        if launch.whole_store && prefs.notes_scope.take().is_some() {
            prefs.save();
        }
        // the setting or the desktop's own preference, whichever asks first
        motion::set_reduced(prefs.reduce_motion || motion::system_prefers_reduced());
        // E2E hook: WARP_NOTES_PAGE=settings opens the rail's Settings section
        // on launch (the probes drive surfaces the pointer cannot reach
        // without taking the human's screen); otherwise the rail opens where
        // the last session left it
        let section = match std::env::var("WARP_NOTES_PAGE").as_deref() {
            Ok("settings") => Section::Settings,
            Ok("about") => Section::About,
            _ => match prefs.section.as_str() {
                "settings" => Section::Settings,
                "about" => Section::About,
                _ => Section::Notes,
            },
        };
        let settings_section = match prefs.settings_section.as_str() {
            "appearance" => SettingsSection::Appearance,
            _ => SettingsSection::General,
        };
        // a --root launch opens the whole store; otherwise a --repo pin wins
        // over the scope the last session reached, and the tree reopens the
        // folders it had, still under this store root
        let notes_scope = if launch.whole_store {
            None
        } else {
            match notes_scope {
                Some(repo) => Some(repo),
                None => prefs.notes_scope.clone(),
            }
        };
        let expanded = prefs
            .expanded
            .iter()
            .map(|relative| store_root.join(relative.replace('\\', "/")))
            .collect();
        let list_visible = prefs.list_visible;
        let mut page = Self {
            store_root,
            store_pinned,
            store_path_input,
            store_path_generation: 0,
            store_path_pending: None,
            notes_scope,
            section,
            about_selected: warpui_kit_core::about::AppId::Notes,
            about_license_open: false,
            about_license: None,
            settings_section,
            prefs,
            list_visible,
            list_scope: ListScope::All,
            splitter_drag: Rc::new(Cell::new(None)),
            tree_scroll: UniformListScrollHandle::default(),
            tree_focus: cx.focus_handle(),
            tree_cursor: None,
            page_focus: cx.focus_handle(),
            settings_scroll: ScrollHandle::default(),
            tree: Rc::new(Vec::new()),
            tree_loaded_for: None,
            note_names_stale: true,
            expanded,
            open_note: None,
            open_tabs: Vec::new(),
            active_tab: 0,
            history: Vec::new(),
            history_index: 0,
            dirty: HashMap::new(),
            saved_text: String::new(),
            saved_meta: None,
            conflict: false,
            conflicts: HashSet::new(),
            text_stats: TextStats::default(),
            stats_revision: None,
            age_cache: None,
            search_field_box: Rc::new(Cell::new(None)),
            search,
            search_hits: Vec::new(),
            search_selected: 0,
            search_all_notes: true,
            search_results_open: false,
            context_open: false,
            context_tab: ContextTab::Outline,
            outline_cache: None,
            backlinks: Vec::new(),
            backlinks_for: None,
            backlinks_generation: 0,
            backlinks_building: false,
            search_generation: 0,
            find_open: false,
            find,
            find_hits: Vec::new(),
            find_index: 0,
            shortcut_open: false,
            editor,
            save_epoch: 0,
            create_open: false,
            create_name,
            create_folder,
            prompt: None,
            prompt_input,
            row_menu: None,
            trash_open: false,
            palette_open: false,
            palette_input,
            palette_items: Vec::new(),
            palette_selected: 0,
            notice: None,
            notice_seq: 0,
            titlebar_should_move: false,
        };
        // A store that moved or lost files must not leave dead quick-open
        // rows; the lists are store-relative, so prune before the first open.
        let recent_before = page.prefs.recent.clone();
        let pinned_before = page.prefs.pinned.clone();
        page.prefs
            .recent
            .retain(|relative| page.store_root.join(relative).is_file());
        page.prefs
            .pinned
            .retain(|relative| page.store_root.join(relative).is_file());
        if page.prefs.recent != recent_before || page.prefs.pinned != pinned_before {
            page.prefs.save();
        }
        // --note: the requested note opens right away, tree expanded to it.
        // Contained like every other store path — a stray --note must not
        // reach (or on save, overwrite) a file outside the store.
        if let Some(path) = launch.open_note {
            // A relative --note is store-relative by contract; with --repo
            // pinned, the natural reading is inside that repo's folder, so
            // try it first when the file is there (`--repo demo --note
            // hello.md` used to fail because it resolved at the store root).
            let path = if path.is_relative() {
                page.notes_scope
                    .as_deref()
                    .map(|repo| notes_core::repo_notes_root(&page.store_root, repo).join(&path))
                    .filter(|candidate| candidate.is_file())
                    .unwrap_or(path)
            } else {
                path
            };
            match notes_core::resolve_note_in_store(&page.store_root, &path) {
                Ok(path) => {
                    page.expand_to(&path);
                    page.open_note(path, window, cx);
                }
                Err(err) => page.notice(
                    cx,
                    NoticeKind::Error,
                    &format!("cannot open {}: {err}", path.display()),
                ),
            }
        } else if let Some(relative) = page.prefs.open_note.clone() {
            // no note named on the command line: the last session's open note
            // comes back, so a restart continues where the user left off. A
            // note deleted since is simply skipped.
            if let Ok(path) =
                notes_core::resolve_note_in_store(&page.store_root, Path::new(&relative))
                && path.is_file()
            {
                page.expand_to(&path);
                page.open_note(path, window, cx);
            }
        }
        // quitting or closing the window must not wait out the autosave
        // debounce: the buffers land on disk before the app goes away
        cx.on_app_quit(|this, cx| {
            this.flush_saves(cx);
            async {}
        })
        .detach();
        // a window that comes back from another app must see notes created or
        // removed outside it, and an externally edited open note must not be
        // overwritten by the next autosave
        cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.refresh_from_disk(window, cx);
            }
        })
        .detach();
        page.set_note_window_title(window);
        if page.open_note.is_none() {
            window.focus(&page.page_focus, cx);
        }
        let weak = cx.weak_entity();
        window.on_window_should_close(cx, move |_, cx| {
            let _ = weak.update(cx, |page, cx| page.flush_saves(cx));
            true
        });
        page
    }

    fn notes_root(&self) -> Option<PathBuf> {
        // the pinned scope decides — repo-scoped when launch or the up button
        // set one, whole-store otherwise
        match &self.notes_scope {
            Some(repo) => Some(notes_core::repo_notes_root(&self.store_root, repo)),
            None => Some(self.store_root.clone()),
        }
    }

    /// Expands the tree folders down to `path`, so a note reached without the
    /// tree (a launch argument, a link) still shows where it lives.
    fn expand_to(&mut self, path: &Path) {
        let root = self.notes_root();
        for parent in path.ancestors().skip(1) {
            if let Some(root) = &root
                && parent.starts_with(root)
            {
                self.expanded.insert(parent.to_path_buf());
            }
            if parent.parent().is_none() {
                break;
            }
        }
    }

    /// Stores the tree's expanded folders in the prefs, relative to the store
    /// root so a moved store keeps them.
    fn remember_expanded(&mut self) {
        self.prefs.expanded = self
            .expanded
            .iter()
            .filter_map(|path| path.strip_prefix(&self.store_root).ok())
            .map(|relative| relative.to_string_lossy().replace('\\', "/"))
            .collect();
        self.prefs.save();
    }

    fn reload_tree(&mut self) {
        if let Some(root) = self.notes_root() {
            // keyed by PathBuf: to_str() collapses non-UTF-8 roots to "" and
            // would re-walk the tree every frame
            if self.tree_loaded_for.as_ref() != Some(&root) {
                self.tree = Rc::new(notes_core::load_tree(&root));
                self.tree_loaded_for = Some(root);
                self.note_names_stale = true;
            }
        } else {
            self.tree = Rc::new(Vec::new());
            self.tree_loaded_for = None;
        }
    }

    /// Reloads the tree after a create or delete, whatever the scope.
    fn refresh_tree(&mut self, root: &Path) {
        self.tree = Rc::new(notes_core::load_tree(root));
        self.note_names_stale = true;
    }

    /// Re-reads the tree and reconciles the open note with the disk. Called by
    /// the titlebar refresh button and by window activation, so a note another
    /// app created or removed never waits for a scope change. A clean buffer
    /// follows the disk; a dirty one becomes a conflict instead of being
    /// overwritten by the next autosave.
    pub(crate) fn refresh_from_disk(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.notes_root() else {
            return;
        };
        self.refresh_tree(&root);

        let Some(path) = self.open_note.clone() else {
            self.set_note_window_title(window);
            cx.notify();
            return;
        };
        let disk = notes_core::note_meta(&path);
        let clean = !self.conflict
            && !self.conflicts.contains(&path)
            && self.editor.read(cx).value().as_ref() == self.saved_text.as_str();
        match (disk, self.saved_meta) {
            (None, _) => {
                if !self.conflict {
                    self.conflict = true;
                    self.conflicts.insert(path.clone());
                    self.notice(
                        cx,
                        NoticeKind::Error,
                        &format!("{} was deleted on disk", path.display()),
                    );
                }
            }
            (Some(meta), saved) if Some(meta) != saved => {
                if clean {
                    match notes_core::read_note(&path) {
                        Ok(text) => {
                            self.saved_text = text.clone();
                            self.saved_meta = Some(meta);
                            self.dirty.remove(&path);
                            self.conflicts.remove(&path);
                            self.conflict = false;
                            self.editor
                                .update(cx, |state, cx| state.set_value(text, window, cx));
                            self.notice(cx, NoticeKind::Info, "note reloaded from disk");
                        }
                        Err(err) => self.notice(
                            cx,
                            NoticeKind::Error,
                            &format!("cannot reload {}: {err}", path.display()),
                        ),
                    }
                } else if !self.conflict {
                    self.conflict = true;
                    self.conflicts.insert(path.clone());
                    self.notice(
                        cx,
                        NoticeKind::Error,
                        &format!(
                            "{} changed on disk; the unsaved buffer was kept",
                            path.display()
                        ),
                    );
                }
            }
            _ => {}
        }
        self.sync_active_tab();
        self.set_note_window_title(window);
        cx.notify();
    }

    fn set_note_window_title(&self, window: &mut Window) {
        let title = self
            .open_note
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| format!("{} — Warp Notes", name.to_string_lossy()))
            .unwrap_or_else(|| "Warp Notes".to_string());
        window.set_window_title(&title);
    }

    pub(crate) fn open_note(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.open_note_inner(path, window, cx, OpenMode::Reuse, true);
    }

    /// A single click in the note list: the tab is a preview the next click
    /// may take over.
    fn open_note_preview(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.open_note_inner(path, window, cx, OpenMode::Preview, true);
    }

    fn open_note_new_tab(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.open_note_inner(path, window, cx, OpenMode::New, true);
    }

    /// Opens a note in the current tab (or in a new one), records history and
    /// keeps the tab list coherent. A path already open activates its tab.
    fn open_note_inner(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
        mode: OpenMode,
        record_history: bool,
    ) {
        if let Some(index) = self.open_tabs.iter().position(|tab| tab.path == path) {
            // a deliberate open keeps the tab it lands on
            let mut kept = false;
            if mode != OpenMode::Preview
                && let Some(tab) = self.open_tabs.get_mut(index)
                && tab.preview
            {
                tab.preview = false;
                kept = true;
            }
            if index != self.active_tab {
                self.activate_tab(index, window, cx);
            } else {
                self.editor
                    .update(cx, |editor, cx| editor.focus(window, cx));
            }
            if record_history {
                self.push_history(path);
            }
            if kept {
                cx.notify();
            }
            return;
        }
        self.save_for_navigation(cx);
        let baseline = match notes_core::read_note(&path) {
            Ok(text) => text,
            Err(err) => {
                self.notice(
                    cx,
                    NoticeKind::Error,
                    &format!("cannot open {}: {err}", path.display()),
                );
                return;
            }
        };
        let content = self
            .dirty
            .get(&path)
            .cloned()
            .unwrap_or_else(|| baseline.clone());
        let meta = notes_core::note_meta(&path);
        let conflict = self.conflicts.contains(&path);
        let tab = OpenTab {
            path: path.clone(),
            saved_text: baseline.clone(),
            saved_meta: meta,
            conflict,
            preview: mode == OpenMode::Preview,
        };
        match mode {
            OpenMode::New => {
                self.open_tabs.push(tab);
                self.active_tab = self.open_tabs.len() - 1;
            }
            OpenMode::Reuse => match self.open_tabs.get_mut(self.active_tab) {
                Some(slot) => *slot = tab,
                None => {
                    self.open_tabs.push(tab);
                    self.active_tab = self.open_tabs.len() - 1;
                }
            },
            OpenMode::Preview => {
                // one preview slot: the active tab when it is a preview, else
                // the tab that already is one, else a fresh tab beside the
                // active one
                let slot = if self
                    .open_tabs
                    .get(self.active_tab)
                    .is_some_and(|tab| tab.preview)
                {
                    Some(self.active_tab)
                } else {
                    self.open_tabs.iter().position(|tab| tab.preview)
                };
                if let Some(index) = slot {
                    self.open_tabs[index] = tab;
                    self.active_tab = index;
                } else if self.open_tabs.is_empty() {
                    self.open_tabs.push(tab);
                    self.active_tab = 0;
                } else {
                    // plant it right after the kept tab, as VS Code does
                    let at = (self.active_tab + 1).min(self.open_tabs.len());
                    self.open_tabs.insert(at, tab);
                    self.active_tab = at;
                }
            }
        }
        self.remember_open_state(path, baseline, content, meta, conflict, window, cx);
        if record_history {
            self.push_history(self.open_note.clone().unwrap_or_default());
        }
        cx.notify();
    }

    /// A double click on a tab keeps it: it stops being the slot a single
    /// click takes over.
    fn keep_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.open_tabs.get_mut(index) else {
            return;
        };
        tab.preview = false;
        self.activate_tab(index, window, cx);
        cx.notify();
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the tab baseline, content and metadata travel together"
    )]
    fn remember_open_state(
        &mut self,
        path: PathBuf,
        baseline: String,
        content: String,
        meta: Option<notes_core::FileMeta>,
        conflict: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.saved_text = baseline;
        self.saved_meta = meta;
        self.conflict = conflict;
        self.find_open = false;
        self.find_hits.clear();
        self.find_index = 0;
        self.age_cache = None;
        self.outline_cache = None;
        self.tree_cursor = Some(path.clone());
        self.open_note = Some(path.clone());
        let folder = path.parent().map(Path::to_path_buf);
        let relative = store_relative(&self.store_root, &path);
        self.prefs.open_note = Some(relative.clone());
        self.prefs.recent.retain(|recent| recent != &relative);
        self.prefs.recent.insert(0, relative);
        self.prefs.recent.truncate(20);
        self.prefs.save();
        self.editor.update(cx, |state, cx| {
            state.set_base_dir(folder, cx);
            state.set_value(content, window, cx);
            state.focus(window, cx);
        });
        self.sync_active_tab();
        if self.context_open && self.context_tab == ContextTab::Links {
            self.refresh_backlinks(cx);
        }
        self.set_note_window_title(window);
    }

    /// Saves the outgoing active tab before navigation. A failed write is
    /// parked; a changed-on-disk write keeps the conflict so the tab can
    /// offer Reload/Overwrite when it comes back.
    fn save_for_navigation(&mut self, cx: &mut Context<Self>) {
        let Some(current) = self.open_note.clone() else {
            return;
        };
        let text = self.editor.read(cx).value().to_string();
        if text != self.saved_text {
            match Self::write_checked(&current, &text, self.saved_meta) {
                Ok(true) => {
                    self.saved_text = text;
                    self.saved_meta = notes_core::note_meta(&current);
                    self.dirty.remove(&current);
                    self.conflicts.remove(&current);
                    tracing::info!("saved {}", current.display());
                }
                Ok(false) => {
                    self.park_conflict(current.clone(), text);
                    self.notice(
                        cx,
                        NoticeKind::Error,
                        &format!(
                            "{} changed on disk; the unsaved buffer was kept",
                            current.display()
                        ),
                    );
                }
                Err(err) => {
                    self.dirty.insert(current, text);
                    self.notice(cx, NoticeKind::Error, &format!("note save failed: {err}"));
                }
            }
        } else {
            self.dirty.remove(&current);
        }
        self.sync_active_tab();
    }

    fn sync_active_tab(&mut self) {
        let Some(path) = self.open_note.clone() else {
            return;
        };
        if let Some(tab) = self.open_tabs.get_mut(self.active_tab) {
            tab.path = path;
            tab.saved_text = self.saved_text.clone();
            tab.saved_meta = self.saved_meta;
            tab.conflict = self.conflict;
        }
    }

    fn activate_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.open_tabs.len() || index == self.active_tab {
            return;
        }
        self.save_for_navigation(cx);
        self.active_tab = index;
        self.load_active_tab(window, cx);
        cx.notify();
    }

    fn load_active_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.open_tabs.get(self.active_tab).cloned() else {
            return;
        };
        let disk = notes_core::note_meta(&tab.path);
        let mut baseline = tab.saved_text.clone();
        let mut meta = tab.saved_meta;
        let mut conflict = tab.conflict;
        let mut content = self
            .dirty
            .get(&tab.path)
            .cloned()
            .unwrap_or_else(|| baseline.clone());
        // A clean inactive tab follows an external edit when it comes back.
        if !self.dirty.contains_key(&tab.path)
            && disk != meta
            && let Ok(text) = notes_core::read_note(&tab.path)
        {
            baseline = text.clone();
            meta = disk;
            conflict = false;
            content = text;
        }
        self.remember_open_state(tab.path, baseline, content, meta, conflict, window, cx);
    }

    fn push_history(&mut self, path: PathBuf) {
        if self.history.get(self.history_index) == Some(&path) {
            return;
        }
        self.history.truncate(self.history_index + 1);
        self.history.push(path);
        if self.history.len() > 50 {
            self.history.remove(0);
        }
        self.history_index = self.history.len().saturating_sub(1);
    }

    fn clear_active_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_note = None;
        self.saved_text.clear();
        self.saved_meta = None;
        self.conflict = false;
        self.find_open = false;
        self.prefs.open_note = None;
        self.prefs.save();
        self.editor
            .update(cx, |editor, cx| editor.set_value("", window, cx));
        self.set_note_window_title(window);
        cx.notify();
    }

    fn close_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.open_tabs.len() {
            return;
        }
        if index == self.active_tab {
            self.save_for_navigation(cx);
            self.open_tabs.remove(index);
            if self.open_tabs.is_empty() {
                self.clear_active_note(window, cx);
            } else {
                self.active_tab = index.min(self.open_tabs.len() - 1);
                self.load_active_tab(window, cx);
            }
        } else {
            self.open_tabs.remove(index);
            if index < self.active_tab {
                self.active_tab -= 1;
            }
        }
        cx.notify();
    }

    fn go_history(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        if self.history.is_empty() {
            return;
        }
        let next = (self.history_index as isize)
            .saturating_add(delta)
            .clamp(0, self.history.len() as isize - 1) as usize;
        if next == self.history_index {
            return;
        }
        let path = self.history[next].clone();
        self.history_index = next;
        self.open_note_inner(path, window, cx, OpenMode::Reuse, false);
    }

    fn schedule_store_path(&mut self, cx: &mut Context<Self>) {
        if self.store_pinned {
            return;
        }
        self.store_path_generation += 1;
        let generation = self.store_path_generation;
        let requested = resolve_store_path(self.store_path_input.read(cx).value().trim());
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(600))
                .await;
            let _ = this.update(cx, |this: &mut NotesPage, cx| {
                if this.store_path_generation == generation {
                    this.store_path_pending = Some(requested);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(crate) fn apply_store_path(
        &mut self,
        requested: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if requested == self.store_root {
            return;
        }
        if let Err(err) = std::fs::create_dir_all(&requested) {
            let current = self.store_root.display().to_string();
            self.store_path_input
                .update(cx, |field, cx| field.set_value(current, window, cx));
            self.notice(
                cx,
                NoticeKind::Error,
                &format!("cannot use that store: {err}"),
            );
            return;
        }
        self.flush_saves(cx);
        self.store_root = requested.clone();
        self.notes_scope = None;
        self.prefs.notes_scope = None;
        self.prefs.store_path = Some(requested.display().to_string());
        self.prefs.open_note = None;
        self.prefs
            .recent
            .retain(|relative| requested.join(relative).is_file());
        self.prefs
            .pinned
            .retain(|relative| requested.join(relative).is_file());
        self.prefs.save();
        self.open_tabs.clear();
        self.active_tab = 0;
        self.history.clear();
        self.history_index = 0;
        self.open_note = None;
        self.saved_text.clear();
        self.saved_meta = None;
        self.conflict = false;
        self.dirty.clear();
        self.conflicts.clear();
        self.tree_cursor = None;
        self.expanded.clear();
        self.tree_loaded_for = None;
        self.tree = Rc::new(Vec::new());
        self.search_hits.clear();
        self.search
            .update(cx, |field, cx| field.set_value(String::new(), window, cx));
        self.search_results_open = false;
        self.palette_open = false;
        self.trash_open = false;
        self.shortcut_open = false;
        self.context_open = false;
        self.find_open = false;
        self.editor
            .update(cx, |editor, cx| editor.set_value("", window, cx));
        let display = requested.display().to_string();
        self.store_path_input
            .update(cx, |field, cx| field.set_value(display, window, cx));
        self.refresh_tree(&requested);
        self.set_note_window_title(window);
        self.notice(cx, NoticeKind::Info, "notes store changed");
        cx.notify();
    }

    fn toggle_context_panel(&mut self, cx: &mut Context<Self>) {
        self.context_open = !self.context_open;
        if self.context_open && self.context_tab == ContextTab::Links {
            self.refresh_backlinks(cx);
        }
        cx.notify();
    }

    fn set_context_tab(&mut self, tab: ContextTab, cx: &mut Context<Self>) {
        self.context_tab = tab;
        if tab == ContextTab::Links {
            self.refresh_backlinks(cx);
        }
        cx.notify();
    }

    fn refresh_backlinks(&mut self, cx: &mut Context<Self>) {
        let Some(open) = self.open_note.clone() else {
            self.backlinks.clear();
            self.backlinks_for = None;
            self.backlinks_building = false;
            return;
        };
        let Some(scope) = self.notes_root() else {
            return;
        };
        let Some(name) = open.file_name() else {
            return;
        };
        let target = note_key(&name.to_string_lossy());
        let store_root = self.store_root.clone();
        self.backlinks_generation += 1;
        let generation = self.backlinks_generation;
        self.backlinks_building = true;
        self.backlinks_for = Some(target.clone());
        self.backlinks.clear();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let links = cx
                .background_executor()
                .spawn(async move { backlinks_for(&scope, &store_root, &target, &open) })
                .await;
            let _ = this.update(cx, |this: &mut NotesPage, cx| {
                if this.backlinks_generation == generation {
                    this.backlinks = links;
                    this.backlinks_building = false;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn outline_headings(&mut self, cx: &mut Context<Self>) -> Vec<Heading> {
        let revision = self.editor.read(cx).revision();
        if let Some((cached, headings)) = &self.outline_cache
            && *cached == revision
        {
            return headings.clone();
        }
        let text = self.editor.read(cx).value();
        let headings = headings_for(&text);
        self.outline_cache = Some((revision, headings.clone()));
        headings
    }

    fn open_note_anywhere(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let outside_scope = self
            .notes_root()
            .is_some_and(|scope| !path.starts_with(&scope));
        if outside_scope {
            self.notes_scope = None;
            self.prefs.notes_scope = None;
            self.prefs.save();
            self.tree_loaded_for = None;
        }
        if let Some(scope) = self.notes_root() {
            self.refresh_tree(&scope);
        }
        self.expand_to(&path);
        self.remember_expanded();
        self.open_note(path, window, cx);
    }

    fn is_pinned(&self, path: &Path) -> bool {
        let relative = store_relative(&self.store_root, path);
        self.prefs.pinned.iter().any(|pinned| pinned == &relative)
    }

    fn toggle_pin(&mut self, path: &Path, cx: &mut Context<Self>) {
        let relative = store_relative(&self.store_root, path);
        if let Some(index) = self
            .prefs
            .pinned
            .iter()
            .position(|pinned| pinned == &relative)
        {
            self.prefs.pinned.remove(index);
            self.notice(cx, NoticeKind::Info, "note unpinned");
        } else {
            self.prefs.pinned.insert(0, relative);
            self.prefs.pinned.truncate(200);
            self.notice(cx, NoticeKind::Info, "note pinned");
        }
        self.prefs.save();
        cx.notify();
    }

    fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut items = Vec::new();
        let tree = notes_core::load_tree(&self.store_root);
        collect_palette_items(&tree, &self.store_root, &mut items);
        items.sort_by(|left, right| left.relative.cmp(&right.relative));
        self.palette_items = items;
        self.palette_open = true;
        self.palette_selected = 0;
        self.prompt = None;
        self.row_menu = None;
        self.trash_open = false;
        self.shortcut_open = false;
        self.search_results_open = false;
        self.palette_input.update(cx, |field, cx| {
            field.set_value(String::new(), window, cx);
            field.focus(window, cx);
        });
        cx.notify();
    }

    fn close_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette_open = false;
        self.palette_items.clear();
        if self.open_note.is_some() {
            self.editor
                .update(cx, |editor, cx| editor.focus(window, cx));
        } else {
            window.focus(&self.tree_focus, cx);
        }
        cx.notify();
    }

    fn palette_rows(&self, cx: &Context<Self>) -> Vec<PaletteItem> {
        let query = self.palette_input.read(cx).value().trim().to_lowercase();
        let mut rows: Vec<(i32, PaletteItem)> = Vec::new();
        if query.is_empty() {
            let mut seen = HashSet::new();
            for relative in &self.prefs.pinned {
                if let Some(item) = self
                    .palette_items
                    .iter()
                    .find(|item| &item.relative == relative)
                    && seen.insert(item.path.clone())
                {
                    rows.push((2, item.clone()));
                }
            }
            for relative in &self.prefs.recent {
                if let Some(item) = self
                    .palette_items
                    .iter()
                    .find(|item| &item.relative == relative)
                    && seen.insert(item.path.clone())
                {
                    rows.push((1, item.clone()));
                }
            }
            for item in &self.palette_items {
                if seen.insert(item.path.clone()) {
                    rows.push((0, item.clone()));
                }
            }
        } else {
            for item in &self.palette_items {
                let score = fuzzy_score(&query, &item.title)
                    .into_iter()
                    .chain(fuzzy_score(&query, &item.relative))
                    .max();
                if let Some(score) = score {
                    rows.push((score, item.clone()));
                }
            }
            rows.sort_by(|left, right| {
                right
                    .0
                    .cmp(&left.0)
                    .then_with(|| left.1.relative.cmp(&right.1.relative))
            });
        }
        rows.into_iter().map(|(_, item)| item).take(12).collect()
    }

    fn open_palette_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rows = self.palette_rows(cx);
        let index = self.palette_selected.min(rows.len().saturating_sub(1));
        let Some(item) = rows.get(index).cloned() else {
            return;
        };
        self.palette_open = false;
        self.open_note_anywhere(item.path, window, cx);
    }

    fn open_palette_selected_in_new_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rows = self.palette_rows(cx);
        let index = self.palette_selected.min(rows.len().saturating_sub(1));
        let Some(item) = rows.get(index).cloned() else {
            return;
        };
        self.palette_open = false;
        self.open_note_new_tab(item.path, window, cx);
    }

    /// Writes `text` to `path` only while the file still matches `expected`;
    /// `Ok(false)` means skipped because it changed on disk. No baseline
    /// (metadata unreadable) keeps the old write-through behavior.
    fn write_checked(
        path: &Path,
        text: &str,
        expected: Option<notes_core::FileMeta>,
    ) -> Result<bool, String> {
        let disk = notes_core::note_meta(path);
        if notes_core::save_decision(true, disk, expected)
            == notes_core::SaveDecision::ChangedOnDisk
        {
            return Ok(false);
        }
        notes_core::write_note(path, text).map_err(|err| err.to_string())?;
        Ok(true)
    }

    /// Parks a buffer whose note moved on disk: the text survives navigation
    /// and the quit-time flush, but autosave never writes it over the note.
    fn park_conflict(&mut self, path: PathBuf, text: String) {
        self.dirty.insert(path.clone(), text);
        self.conflicts.insert(path.clone());
        if self.open_note.as_ref() == Some(&path) {
            self.conflict = true;
        }
        self.sync_active_tab();
    }

    /// Adopts a successful write as the new baseline.
    fn adopt_saved(&mut self, path: &Path, text: String) {
        self.saved_text = text;
        self.saved_meta = notes_core::note_meta(path);
        self.dirty.remove(path);
        self.conflicts.remove(path);
        self.conflict = false;
        self.sync_active_tab();
    }

    /// Where a conflicted buffer is kept instead of over its note: the app's
    /// own data folder, not the notes store (a stray recovery file must not
    /// show up as a note).
    fn recovery_path(path: &Path) -> PathBuf {
        let dir = crate::paths::AppPaths::config_dir().join("recovery");
        let _ = std::fs::create_dir_all(&dir);
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "note.md".into());
        let rand = notes_core::paste_rand16();
        dir.join(format!("{stamp}-{rand:04x}-{name}"))
    }

    fn park_recovery(path: &Path, text: &str) {
        let target = Self::recovery_path(path);
        match notes_core::write_note(&target, text) {
            Ok(()) => tracing::warn!(
                "kept the conflicted buffer for {} at {}",
                path.display(),
                target.display()
            ),
            Err(err) => tracing::warn!("could not keep a recovery copy: {err}"),
        }
    }

    /// A clicked `[[wikilink]]`: the note with that name, next to the open
    /// note first, then anywhere under the notes root. Like Obsidian, a link
    /// to a note that does not exist yet creates it beside the open one.
    fn follow_link(&mut self, target: &str, window: &mut Window, cx: &mut Context<Self>) {
        let name = target.split(['#', '^']).next().unwrap_or(target).trim();
        if name.is_empty() {
            return;
        }
        let file = if name.ends_with(".md") {
            name.to_string()
        } else {
            format!("{name}.md")
        };
        let sibling = self
            .open_note
            .as_ref()
            .and_then(|note| note.parent())
            .map(|folder| folder.join(&file))
            .filter(|path| path.is_file());
        let found = sibling.or_else(|| {
            let root = self.notes_root()?;
            let items = notes_core::load_tree(&root);
            find_note(&items, &file)
        });
        if let Some(path) = found {
            self.open_note(path, window, cx);
            return;
        }
        let Some(root) = self.notes_root() else {
            return;
        };
        let folder = self
            .open_note
            .as_ref()
            .and_then(|note| note.parent())
            .map_or_else(|| root.clone(), Path::to_path_buf);
        let Some(clean) = notes_core::sanitize_entry_name(name, true) else {
            self.notice(cx, NoticeKind::Info, &format!("Not a usable name: {name}"));
            return;
        };
        match notes_core::create_note(&root, &folder, &clean) {
            Ok(path) => {
                self.refresh_tree(&root);
                self.open_note(path, window, cx);
            }
            Err(err) => self.notice(cx, NoticeKind::Error, &format!("note create failed: {err}")),
        }
    }

    /// Writes the open note now. The autosave debounce calls this a second
    /// after the last edit; Ctrl+S (kept for muscle memory) calls it at once.
    /// An already-saved buffer is not rewritten, and a buffer whose file
    /// changed on disk is parked for Reload / Overwrite instead (audit A1).
    fn save_current(&mut self, cx: &mut Context<Self>) {
        if self.conflict {
            return;
        }
        if !self.is_dirty(cx) {
            return;
        }
        let Some(path) = self.open_note.clone() else {
            return;
        };
        let text = self.editor.read(cx).value().to_string();
        match Self::write_checked(&path, &text, self.saved_meta) {
            Ok(true) => {
                self.adopt_saved(&path, text);
                tracing::info!("saved {}", path.display());
            }
            Ok(false) => {
                self.park_conflict(path.clone(), text);
                self.notice(
                    cx,
                    NoticeKind::Error,
                    &format!(
                        "{} changed on disk; choose Reload or Overwrite",
                        path.display()
                    ),
                );
            }
            Err(err) => self.notice(cx, NoticeKind::Error, &format!("note save failed: {err}")),
        }
        self.sync_active_tab();
        cx.notify();
    }

    /// Re-arms the autosave: the editor's Change event bumps the epoch, and
    /// only the task from the last edit still holding it writes the note —
    /// a burst of typing saves once, a second after it stops. `set_value`
    /// emits Change too, but an untouched buffer has nothing to write.
    fn schedule_autosave(&mut self, cx: &mut Context<Self>) {
        // an unresolved conflict pauses autosave until the user picks a side
        if self.conflict || !self.is_dirty(cx) {
            return;
        }
        self.save_epoch += 1;
        let epoch = self.save_epoch;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(AUTOSAVE_DELAY).await;
            let _ = this.update(cx, |this, cx| {
                if this.save_epoch == epoch {
                    this.save_current(cx);
                }
            });
        })
        .detach();
    }

    /// Writes every unsaved buffer now: the open note's, then any parked by
    /// an earlier failed save. A conflicted buffer goes to a recovery copy,
    /// never over the note it no longer matches. The close button and app
    /// quit call this, so the debounce never eats an edit.
    fn flush_saves(&mut self, cx: &mut Context<Self>) {
        let open = self.open_note.clone();
        if let Some(path) = open.clone() {
            let text = self.editor.read(cx).value().to_string();
            if self.conflict || self.conflicts.contains(&path) {
                Self::park_recovery(&path, &text);
            } else if text != self.saved_text {
                // the quit flush is a write like any other: an external edit
                // that landed since the last autosave must not be clobbered
                match Self::write_checked(&path, &text, self.saved_meta) {
                    Ok(true) => tracing::info!("saved {}", path.display()),
                    Ok(false) => Self::park_recovery(&path, &text),
                    Err(err) => tracing::warn!("could not save {} on exit: {err}", path.display()),
                }
            }
        }
        for (path, text) in std::mem::take(&mut self.dirty) {
            if open.as_ref() == Some(&path) {
                // the open note (and its conflict copy) was handled above
                continue;
            }
            if self.conflicts.contains(&path) {
                Self::park_recovery(&path, &text);
            } else if let Err(err) = notes_core::write_note(&path, &text) {
                tracing::warn!("could not save {} on exit: {err}", path.display());
            }
        }
        self.sync_active_tab();
    }

    /// The conflict bar's Reload: drop the local buffer and re-read the note.
    fn reload_conflicted(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.open_note.clone() else {
            return;
        };
        match notes_core::read_note(&path) {
            Ok(text) => {
                self.saved_text = text.clone();
                self.saved_meta = notes_core::note_meta(&path);
                self.dirty.remove(&path);
                self.conflicts.remove(&path);
                self.conflict = false;
                self.save_epoch += 1;
                self.editor
                    .update(cx, |state, cx| state.set_value(text.as_str(), window, cx));
                self.notice(cx, NoticeKind::Info, "note reloaded from disk");
            }
            Err(err) => self.notice(
                cx,
                NoticeKind::Error,
                &format!("cannot reload {}: {err}", path.display()),
            ),
        }
        self.sync_active_tab();
        cx.notify();
    }

    /// The conflict bar's Overwrite: the local buffer wins, deliberately.
    fn overwrite_conflicted(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.open_note.clone() else {
            return;
        };
        let text = self.editor.read(cx).value().to_string();
        match notes_core::write_note(&path, &text) {
            Ok(()) => {
                self.adopt_saved(&path, text);
                self.notice(cx, NoticeKind::Info, "note overwritten on disk");
            }
            Err(err) => self.notice(cx, NoticeKind::Error, &format!("note save failed: {err}")),
        }
        self.sync_active_tab();
        cx.notify();
    }

    /// The sidebar search: the walk reads up to 2000 note files, so it is
    /// debounced and pushed off the UI thread — a newer keystroke invalidates
    /// whatever is still in flight.
    /// Ctrl+F: every match of the term, case-insensitive when the term is
    /// ASCII (the lowercase copy keeps every byte offset in place), and the
    /// first one selected in the editor.
    fn run_find(&mut self, term: String, cx: &mut Context<Self>) {
        self.find_hits.clear();
        self.find_index = 0;
        let text = self.editor.read(cx).value().to_string();
        let term = term.trim().to_string();
        if !term.is_empty() && self.open_note.is_some() {
            let (hay, needle) = if term.is_ascii() {
                (text.to_ascii_lowercase(), term.to_ascii_lowercase())
            } else {
                (text.clone(), term.clone())
            };
            let mut from = 0;
            while let Some(offset) = hay[from..].find(&needle) {
                let start = from + offset;
                self.find_hits.push(start..start + needle.len());
                from = start + needle.len().max(1);
            }
        }
        self.select_find_hit(cx);
        cx.notify();
    }

    /// Puts the editor's selection on the current match, so the note shows
    /// where it is even while the find field keeps the keyboard.
    fn select_find_hit(&mut self, cx: &mut Context<Self>) {
        if let Some(range) = self.find_hits.get(self.find_index).cloned() {
            self.editor
                .update(cx, |editor, cx| editor.set_selected_range(range, cx));
        }
    }

    /// Enter and Shift+Enter walk the matches, wrapping at either end.
    fn step_find(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.find_hits.is_empty() {
            return;
        }
        let len = self.find_hits.len() as isize;
        self.find_index = (self.find_index as isize + delta).rem_euclid(len) as usize;
        self.select_find_hit(cx);
        cx.notify();
    }

    fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.find_open = false;
        cx.notify();
        let editor = self.editor.clone();
        editor.update(cx, |editor, cx| editor.focus(window, cx));
    }

    fn run_search(&mut self, term: String, cx: &mut Context<Self>) {
        self.search_generation += 1;
        let generation = self.search_generation;
        let term = term.trim().to_string();
        self.search_selected = 0;
        if term.is_empty() {
            self.search_hits.clear();
            cx.notify();
            return;
        }
        let root = if self.search_all_notes {
            self.store_root.clone()
        } else {
            let Some(root) = self.notes_root() else {
                return;
            };
            root
        };
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(180))
                .await;
            let hits = cx
                .background_executor()
                .spawn(async move { notes_core::search(&root, &term) })
                .await;
            let _ = this.update(cx, |this: &mut NotesPage, cx| {
                if this.search_generation == generation {
                    this.search_hits = hits;
                    this.search_selected = 0;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn open_search_hit(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.open_search_hit_inner(index, false, window, cx);
    }

    fn open_search_hit_in_new_tab(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_search_hit_inner(index, true, window, cx);
    }

    fn open_search_hit_inner(
        &mut self,
        index: usize,
        new_tab: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(hit) = self.search_hits.get(index).cloned() else {
            return;
        };
        self.search
            .update(cx, |field, cx| field.set_value(String::new(), window, cx));
        self.search_hits.clear();
        self.search_selected = 0;
        self.search_results_open = false;
        if new_tab {
            self.open_note_new_tab(hit.path, window, cx);
        } else {
            self.open_note_anywhere(hit.path, window, cx);
        }
    }

    fn toggle_search_scope(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.notes_scope.is_none() {
            return;
        }
        self.search_all_notes = !self.search_all_notes;
        let term = self.search.read(cx).value().to_string();
        self.run_search(term, cx);
        self.search.update(cx, |field, cx| field.focus(window, cx));
        cx.notify();
    }

    fn open_search_results(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.search.read(cx).value().trim().is_empty() {
            return;
        }
        self.search_results_open = true;
        self.search_selected = 0;
        self.palette_open = false;
        self.trash_open = false;
        self.shortcut_open = false;
        self.search.update(cx, |field, cx| field.focus(window, cx));
        cx.notify();
    }

    fn close_search_results(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_results_open = false;
        if self.open_note.is_some() {
            self.editor
                .update(cx, |editor, cx| editor.focus(window, cx));
        } else {
            window.focus(&self.tree_focus, cx);
        }
        cx.notify();
    }

    fn create_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.notes_root() else {
            return;
        };
        let raw_name = self.create_name.read(cx).value().to_string();
        let raw_folder = self.create_folder.read(cx).value().to_string();
        let Some(name) = notes_core::sanitize_entry_name(raw_name.trim(), true) else {
            self.notice(cx, NoticeKind::Info, "Enter a note name");
            return;
        };
        // optional subfolder under the root (sanitized per segment)
        let mut target = root.clone();
        if !raw_folder.trim().is_empty() {
            for segment in raw_folder.trim().split(['/', '\\']) {
                match notes_core::sanitize_entry_name(segment, false) {
                    Some(clean) => target.push(clean),
                    None => {
                        self.notice(cx, NoticeKind::Info, "Folder name is not usable");
                        return;
                    }
                }
            }
        }
        match notes_core::create_note(&root, &target, &name) {
            Ok(path) => {
                self.refresh_tree(&root);
                self.create_open = false;
                self.create_name
                    .update(cx, |f, cx| f.set_value(String::new(), window, cx));
                self.create_folder
                    .update(cx, |f, cx| f.set_value(String::new(), window, cx));
                if let Some(parent) = path.parent() {
                    self.expanded.insert(parent.to_path_buf());
                    self.remember_expanded();
                }
                self.open_note(path, window, cx);
            }
            Err(err) => self.notice(cx, NoticeKind::Error, &format!("note create failed: {err}")),
        }
    }

    /// The bottom-right toast: an error stays longer and carries a close
    /// button; a spawned timer hides it so no user action is needed.
    pub(crate) fn notice(&mut self, cx: &mut Context<Self>, kind: NoticeKind, text: &str) {
        self.notice_with_action(cx, kind, text, None);
    }

    fn notice_with_action(
        &mut self,
        cx: &mut Context<Self>,
        kind: NoticeKind,
        text: &str,
        action: Option<NoticeAction>,
    ) {
        let ttl = if action.is_some() {
            std::time::Duration::from_secs(8)
        } else {
            notice_ttl(kind)
        };
        let expires = std::time::Instant::now() + ttl;
        self.notice_seq += 1;
        self.notice = Some(Notice {
            kind,
            text: text.to_string(),
            expires,
            action,
            seq: self.notice_seq,
        });
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(ttl).await;
            let _ = this.update(cx, |this: &mut NotesPage, cx| {
                // only clear our own toast; a newer one may have replaced it
                if this.notice.as_ref().is_some_and(|n| n.expires == expires) {
                    this.notice = None;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    /// Moves a note or folder to Trash. The open buffer (or a buffer inside a
    /// trashed folder) is written or parked as a recovery copy first; a Trash
    /// failure leaves the editor exactly where it was.
    fn trash_path(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let Some(scope) = self.notes_root() else {
            return;
        };
        let open_before = self.open_note.clone();
        let affected = open_before
            .as_ref()
            .is_some_and(|open| open == &path || open.starts_with(&path));
        if affected && let Some(open) = open_before.clone() {
            let text = self.editor.read(cx).value().to_string();
            if self.conflict || self.conflicts.contains(&open) {
                Self::park_recovery(&open, &text);
            } else if text != self.saved_text {
                match Self::write_checked(&open, &text, self.saved_meta) {
                    Ok(true) => {
                        self.saved_text = text;
                        self.saved_meta = notes_core::note_meta(&open);
                        self.dirty.remove(&open);
                    }
                    Ok(false) => Self::park_recovery(&open, &text),
                    Err(err) => {
                        Self::park_recovery(&open, &text);
                        tracing::warn!("could not save {} before Trash: {err}", open.display());
                    }
                }
            }
        }

        match crate::trash::trash_entry(&self.store_root, &path) {
            Ok(entry) => {
                let removed_active = affected
                    || self
                        .open_tabs
                        .get(self.active_tab)
                        .is_some_and(|tab| tab.path == path || tab.path.starts_with(&path));
                let removed_before = self.open_tabs[..self.active_tab.min(self.open_tabs.len())]
                    .iter()
                    .filter(|tab| tab.path == path || tab.path.starts_with(&path))
                    .count();
                self.open_tabs
                    .retain(|tab| tab.path != path && !tab.path.starts_with(&path));
                self.dirty
                    .retain(|entry, _| entry != &path && !entry.starts_with(&path));
                self.conflicts
                    .retain(|entry| entry != &path && !entry.starts_with(&path));
                if removed_active {
                    if self.open_tabs.is_empty() {
                        self.clear_active_note(window, cx);
                    } else {
                        self.active_tab = self
                            .active_tab
                            .saturating_sub(removed_before)
                            .min(self.open_tabs.len() - 1);
                        self.load_active_tab(window, cx);
                    }
                } else {
                    self.active_tab = self.active_tab.saturating_sub(removed_before);
                    self.set_note_window_title(window);
                }
                self.expanded.retain(|folder| !folder.starts_with(&path));
                if self
                    .tree_cursor
                    .as_ref()
                    .is_some_and(|cursor| cursor == &path || cursor.starts_with(&path))
                {
                    self.tree_cursor = path.parent().map(Path::to_path_buf);
                }
                let relative = store_relative(&self.store_root, &path);
                let under = |entry: &String| {
                    entry == &relative || entry.starts_with(&format!("{relative}/"))
                };
                self.prefs.pinned.retain(|entry| !under(entry));
                self.prefs.recent.retain(|entry| !under(entry));
                self.prefs.save();
                self.remember_expanded();
                self.refresh_tree(&scope);
                self.notice_with_action(
                    cx,
                    NoticeKind::Info,
                    "moved to Trash",
                    Some(NoticeAction::UndoTrash {
                        entry,
                        reopen: affected.then_some(open_before).flatten(),
                    }),
                );
            }
            Err(err) => self.notice(cx, NoticeKind::Error, &format!("Trash failed: {err}")),
        }
        cx.notify();
    }

    fn undo_trash(
        &mut self,
        entry: crate::trash::TrashEntry,
        reopen: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match crate::trash::restore_entry(&self.store_root, &entry) {
            Ok(path) => {
                self.reveal_restored(&path, window, cx);
                if let Some(open) = reopen
                    && open.is_file()
                {
                    self.open_note(open, window, cx);
                }
                self.notice(cx, NoticeKind::Info, "restored from Trash");
            }
            Err(err) => self.notice(cx, NoticeKind::Error, &format!("restore failed: {err}")),
        }
        cx.notify();
    }

    fn restore_trash_entry(
        &mut self,
        entry: crate::trash::TrashEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match crate::trash::restore_entry(&self.store_root, &entry) {
            Ok(path) => {
                self.reveal_restored(&path, window, cx);
                self.notice(cx, NoticeKind::Info, "restored from Trash");
            }
            Err(err) => self.notice(cx, NoticeKind::Error, &format!("restore failed: {err}")),
        }
        cx.notify();
    }

    /// Shows a restored path in the tree even when it came back outside the
    /// current repo pin; the pin is dropped so the tree and the path agree.
    fn reveal_restored(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let outside_scope = self
            .notes_root()
            .is_some_and(|scope| !path.starts_with(&scope));
        if outside_scope {
            self.notes_scope = None;
            self.prefs.notes_scope = None;
            self.prefs.save();
            self.tree_loaded_for = None;
        }
        if let Some(scope) = self.notes_root() {
            self.refresh_tree(&scope);
        }
        self.expand_to(path);
        self.remember_expanded();
        self.set_note_window_title(window);
        cx.notify();
    }

    pub(crate) fn open_trash_dialog(&mut self, cx: &mut Context<Self>) {
        self.row_menu = None;
        self.prompt = None;
        self.palette_open = false;
        self.shortcut_open = false;
        self.search_results_open = false;
        self.trash_open = true;
        cx.notify();
    }

    fn open_row_menu(
        &mut self,
        path: PathBuf,
        is_folder: bool,
        at: gpui::Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.row_menu = Some(RowMenu {
            path,
            is_folder,
            x: at.x.as_f32(),
            y: at.y.as_f32(),
        });
        self.prompt = None;
        cx.notify();
    }

    fn open_tree_entry(
        &mut self,
        path: PathBuf,
        is_folder: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.row_menu = None;
        self.tree_cursor = Some(path.clone());
        window.focus(&self.tree_focus, cx);
        if is_folder {
            if self.expanded.contains(&path) {
                self.expanded.remove(&path);
            } else {
                self.expanded.insert(path);
            }
            self.remember_expanded();
            cx.notify();
        } else {
            self.open_note(path, window, cx);
        }
    }

    fn start_prompt(
        &mut self,
        kind: PromptKind,
        target: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(scope) = self.notes_root() else {
            return;
        };
        let prefill = match kind {
            PromptKind::Rename => {
                if target.is_dir() {
                    target
                        .file_name()
                        .map(|name| name.to_string_lossy().to_string())
                        .unwrap_or_default()
                } else {
                    target
                        .file_stem()
                        .map(|name| name.to_string_lossy().to_string())
                        .unwrap_or_default()
                }
            }
            PromptKind::NewFolder => String::new(),
            PromptKind::MoveTo => target
                .parent()
                .and_then(|parent| parent.strip_prefix(&scope).ok())
                .map(|relative| relative.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default(),
        };
        self.prompt_input.update(cx, |field, cx| {
            field.set_value(prefill, window, cx);
            field.focus(window, cx);
        });
        self.prompt = Some(PromptState { kind, target });
        self.row_menu = None;
        self.search_results_open = false;
        cx.notify();
    }

    fn confirm_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(prompt) = self.prompt.clone() else {
            return;
        };
        let Some(scope) = self.notes_root() else {
            return;
        };
        let value = self.prompt_input.read(cx).value().trim().to_string();
        let outcome = match prompt.kind {
            PromptKind::Rename => {
                notes_core::rename_entry(&scope, &prompt.target, &value).map(|new| (new, None))
            }
            PromptKind::NewFolder => {
                let parent = if prompt.target.is_dir() {
                    prompt.target.clone()
                } else {
                    prompt
                        .target
                        .parent()
                        .map(Path::to_path_buf)
                        .unwrap_or_else(|| scope.clone())
                };
                notes_core::create_folder(&scope, &parent, &value).map(|new| (new, Some(parent)))
            }
            PromptKind::MoveTo => {
                let parent = match resolve_folder(&scope, &value) {
                    Ok(parent) => parent,
                    Err(err) => {
                        self.notice(cx, NoticeKind::Error, &err.to_string());
                        return;
                    }
                };
                notes_core::move_entry(&scope, &prompt.target, &parent).map(|new| (new, None))
            }
        };
        match outcome {
            Ok((new_path, new_parent)) => {
                self.prompt = None;
                if prompt.kind == PromptKind::NewFolder {
                    if let Some(parent) = new_parent {
                        self.expanded.insert(parent);
                        self.remember_expanded();
                    }
                    self.refresh_tree(&scope);
                } else {
                    self.after_path_change(&prompt.target, &new_path, window, cx);
                    if prompt.kind == PromptKind::Rename {
                        self.refresh_tree(&scope);
                    }
                }
                self.notice(
                    cx,
                    NoticeKind::Info,
                    match prompt.kind {
                        PromptKind::Rename => "note renamed",
                        PromptKind::NewFolder => "folder created",
                        PromptKind::MoveTo => "note moved",
                    },
                );
            }
            Err(err) => self.notice(cx, NoticeKind::Error, &err.to_string()),
        }
        cx.notify();
    }

    fn after_path_change(
        &mut self,
        old: &Path,
        new: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(open) = self.open_note.clone()
            && let Ok(relative) = open.strip_prefix(old)
        {
            let updated = new.join(relative);
            self.open_note = Some(updated.clone());
            self.saved_meta = notes_core::note_meta(&updated);
            self.prefs.open_note = Some(store_relative(&self.store_root, &updated));
            self.editor.update(cx, |state, cx| {
                state.set_base_dir(updated.parent().map(Path::to_path_buf), cx);
            });
            self.set_note_window_title(window);
        }
        for tab in &mut self.open_tabs {
            tab.path = rewrite_path(&tab.path, old, new);
            tab.saved_meta = notes_core::note_meta(&tab.path);
        }
        self.saved_meta = self
            .open_note
            .as_ref()
            .and_then(|path| notes_core::note_meta(path));
        self.dirty = self
            .dirty
            .drain()
            .map(|(path, text)| (rewrite_path(&path, old, new), text))
            .collect();
        self.conflicts = self
            .conflicts
            .drain()
            .map(|path| rewrite_path(&path, old, new))
            .collect();
        self.expanded = self
            .expanded
            .drain()
            .map(|path| rewrite_path(&path, old, new))
            .collect();
        self.tree_cursor = self
            .tree_cursor
            .as_ref()
            .map(|path| rewrite_path(path, old, new));
        rewrite_pref_list(&mut self.prefs.recent, &self.store_root, old, new);
        rewrite_pref_list(&mut self.prefs.pinned, &self.store_root, old, new);
        self.prefs.save();
        self.remember_expanded();
        self.age_cache = None;
        self.note_names_stale = true;
        if let Some(scope) = self.notes_root() {
            self.refresh_tree(&scope);
        }
        cx.notify();
    }

    fn duplicate_path(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.open_note.as_ref() == Some(&path) && self.is_dirty(cx) {
            self.save_current(cx);
            if self.conflict {
                self.notice(cx, NoticeKind::Error, "fix the conflict before duplicating");
                return;
            }
        }
        let Some(scope) = self.notes_root() else {
            return;
        };
        match notes_core::duplicate_note(&scope, &path) {
            Ok(new_path) => {
                if let Some(parent) = new_path.parent() {
                    self.expanded.insert(parent.to_path_buf());
                    self.remember_expanded();
                }
                self.refresh_tree(&scope);
                self.open_note(new_path, window, cx);
                self.notice(cx, NoticeKind::Info, "note duplicated");
            }
            Err(err) => self.notice(cx, NoticeKind::Error, &err.to_string()),
        }
    }

    fn copy_entry_path(&mut self, path: &Path, cx: &mut Context<Self>) {
        let relative = store_relative(&self.store_root, path);
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(relative.clone()));
        self.notice(cx, NoticeKind::Info, &format!("copied {relative}"));
    }

    fn new_note_here(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let Some(scope) = self.notes_root() else {
            return;
        };
        let folder = if path.is_dir() {
            path
        } else {
            path.parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| scope.clone())
        };
        let relative = folder
            .strip_prefix(&scope)
            .map(|relative| relative.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        self.create_open = true;
        self.create_folder
            .update(cx, |field, cx| field.set_value(relative, window, cx));
        self.create_name
            .update(cx, |field, cx| field.focus(window, cx));
        self.row_menu = None;
        cx.notify();
    }

    fn apply_format(&mut self, format: &str, window: &mut Window, cx: &mut Context<Self>) {
        // the kit editor owns the selection: read it, apply over the exact
        // range, and restore the op's returned selection
        let text = self.editor.read(cx).value().to_string();
        let range = self.editor.read(cx).selected_range();
        let start_ch = byte_to_char(&text, range.start);
        let end_ch = byte_to_char(&text, range.end);
        let result = apply::markdown_apply(format, &text, start_ch, end_ch - start_ch);
        self.editor.update(cx, |state, cx| {
            // replace_all keeps the undo stack, unlike set_value
            state.replace_all(result.text.as_str(), window, cx);
            let start_b = char_to_byte(&result.text, result.start);
            let end_b = char_to_byte(&result.text, result.start + result.length);
            state.set_selected_range(start_b..end_b, cx);
        });
        cx.notify();
    }

    /// The paste hook on the live editor: a clipboard IMAGE becomes a file
    /// saved next to the open note plus a relative markdown ref inserted at
    /// the caret (the dirty marker flows from the usual buffer comparison).
    /// Text entries win — Windows delivers text+image as one mixed item and
    /// Wayland offers text alongside images — so a real text paste is never
    /// hijacked, and returning `false` lets the editor's own text insertion
    /// run (a no-op for image-only clipboards). A failed save still consumes
    /// the paste: the user asked to paste an image, not to spill its bytes.
    fn paste_from_clipboard(
        page: &WeakEntity<Self>,
        item: &gpui::ClipboardItem,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        if item.entries.iter().any(
            |entry| matches!(entry, gpui::ClipboardEntry::String(text) if !text.text.is_empty()),
        ) {
            return false;
        }
        let Some(image) = item.entries.iter().find_map(|entry| match entry {
            gpui::ClipboardEntry::Image(image) => Some(image),
            _ => None,
        }) else {
            return false;
        };
        let Some(page) = page.upgrade() else {
            return false;
        };
        let Some(folder) = page
            .read(cx)
            .open_note
            .as_ref()
            .and_then(|path| path.parent())
            .map(Path::to_path_buf)
        else {
            return false;
        };
        match notes_core::insert_image_asset(&folder, image.format.extension(), &image.bytes) {
            Ok(relative) => {
                let editor = page.read(cx).editor.clone();
                editor.update(cx, |state, cx| {
                    state.insert(format!("![image]({relative})\n"), window, cx);
                });
                true
            }
            Err(err) => {
                page.update(cx, |page, cx| {
                    page.notice(cx, NoticeKind::Error, &format!("image paste failed: {err}"))
                });
                true
            }
        }
    }

    /// The editor's current buffer — for headless probes.
    #[doc(hidden)]
    pub fn editor_value(&self, cx: &gpui::App) -> String {
        self.editor.read(cx).value().to_string()
    }

    /// The editor's selection — for headless probes (find).
    #[doc(hidden)]
    pub fn editor_selected_range(&self, cx: &gpui::App) -> std::ops::Range<usize> {
        self.editor.read(cx).selected_range()
    }

    /// The dirty state of the open note — for headless probes.
    #[doc(hidden)]
    pub fn is_dirty(&self, cx: &gpui::App) -> bool {
        self.open_note.is_some()
            && self.editor.read(cx).value().as_ref() != self.saved_text.as_str()
    }

    /// The search field — for headless probes (typing a term).
    #[doc(hidden)]
    pub fn search_handle(&self) -> &Entity<InputState> {
        &self.search
    }

    /// Runs the page's search synchronously — for headless probes, which
    /// cannot wait out the field's 180 ms debounce on a real timer.
    #[doc(hidden)]
    pub fn run_search_for_probe(&mut self, term: String, cx: &mut Context<Self>) {
        let root = if self.search_all_notes {
            Some(self.store_root.clone())
        } else {
            self.notes_root()
        };
        self.search_hits = root
            .map(|root| notes_core::search(&root, &term))
            .unwrap_or_default();
        cx.notify();
    }

    /// How many results the last search produced — for headless probes.
    #[doc(hidden)]
    pub fn search_hit_count(&self, _cx: &gpui::App) -> usize {
        self.search_hits.len()
    }

    /// The status bar's counts and the revision they were computed from — for
    /// headless probes (the cache must not recompute per frame).
    #[doc(hidden)]
    pub fn text_stats(&self) -> (usize, usize, usize, Option<u64>) {
        (
            self.text_stats.words,
            self.text_stats.chars,
            self.text_stats.lines,
            self.stats_revision,
        )
    }

    /// The editor entity — for headless probes (caret, layout hit-testing).
    #[doc(hidden)]
    pub fn editor_handle(&self) -> &Entity<LiveEditorState> {
        &self.editor
    }

    /// The prompt's field — for headless probes (rename/move/new-folder).
    #[doc(hidden)]
    pub fn prompt_input_handle(&self) -> &Entity<InputState> {
        &self.prompt_input
    }

    /// The path the editor is showing — for headless probes.
    #[doc(hidden)]
    pub fn open_note_path(&self) -> Option<PathBuf> {
        self.open_note.clone()
    }

    /// The tree's focus handle — for headless keyboard probes.
    #[doc(hidden)]
    pub fn tree_focus_handle(&self) -> gpui::FocusHandle {
        self.tree_focus.clone()
    }

    /// The selected tree row — for headless keyboard probes.
    #[doc(hidden)]
    pub fn tree_cursor_for_probe(&self) -> Option<PathBuf> {
        self.tree_cursor.clone()
    }

    /// The list head's live counts — for headless head probes.
    #[doc(hidden)]
    pub fn list_summary_for_probe(&self) -> String {
        self.summary_text()
    }

    /// Which list scope is showing: "all" or "pinned".
    #[doc(hidden)]
    pub fn list_scope_for_probe(&self) -> &'static str {
        match self.list_scope {
            ListScope::All => "all",
            ListScope::Pinned => "pinned",
        }
    }

    /// How many notes a folder in the tree holds — for headless band probes.
    #[doc(hidden)]
    pub fn band_count_for_probe(&self, folder: &str) -> Option<usize> {
        fn find<'a>(
            items: &'a [Rc<notes_core::NotesTreeItem>],
            folder: &str,
        ) -> Option<&'a notes_core::NotesTreeItem> {
            for item in items {
                if item.is_folder && item.name == folder {
                    return Some(item);
                }
                if let Some(found) = find(&item.children, folder) {
                    return Some(found);
                }
            }
            None
        }
        find(&self.tree, folder).map(note_count)
    }

    /// The search popup's selected row — for headless arrow-key probes.
    #[doc(hidden)]
    pub fn search_selected_for_probe(&self) -> usize {
        self.search_selected
    }

    /// The quick-open filter field — for headless palette probes.
    #[doc(hidden)]
    pub fn palette_input_handle(&self) -> &Entity<InputState> {
        &self.palette_input
    }

    /// The settled repo scope — for headless scope-drop probes.
    #[doc(hidden)]
    pub fn notes_scope_for_probe(&self) -> Option<String> {
        self.notes_scope.clone()
    }

    /// The open tab paths — for headless tab/history probes.
    #[doc(hidden)]
    pub fn tab_paths_for_probe(&self) -> Vec<PathBuf> {
        self.open_tabs.iter().map(|tab| tab.path.clone()).collect()
    }

    #[doc(hidden)]
    pub fn active_tab_for_probe(&self) -> usize {
        self.active_tab
    }

    /// Which open tabs are previews — for headless tab probes.
    #[doc(hidden)]
    pub fn preview_tabs_for_probe(&self) -> Vec<bool> {
        self.open_tabs.iter().map(|tab| tab.preview).collect()
    }

    /// The store root the page is currently using.
    #[doc(hidden)]
    pub fn store_root_for_probe(&self) -> PathBuf {
        self.store_root.clone()
    }

    /// The editable store field.
    #[doc(hidden)]
    pub fn store_path_input_handle(&self) -> &Entity<InputState> {
        &self.store_path_input
    }
}

fn byte_to_char(text: &str, byte: usize) -> usize {
    text[..byte.min(text.len())].chars().count()
}

/// True when a vertical scroller at `offset` (≤ 0, clamped to `max_offset`)
/// has room to move for `delta` — the "the inner one handled it" test. A
/// wheel event routed to an inner scrollable must not also scroll an outer
/// one, but a scroller already hard against the edge in the wheel's
/// direction lets the event pass, as browsers chain it.
pub(crate) fn wheel_reaches_content(
    offset: gpui::Point<Pixels>,
    max_offset: gpui::Point<Pixels>,
    delta: gpui::Pixels,
) -> bool {
    if delta < px(0.) {
        // scrolling toward the bottom: room is whatever is left below
        -offset.y < max_offset.y
    } else {
        // scrolling back toward the top: any offset off the top moves
        offset.y < px(0.)
    }
}

/// Every note's file name in the tree.
fn collect_note_names(items: &[Rc<notes_core::NotesTreeItem>], names: &mut Vec<String>) {
    for item in items {
        if item.is_folder {
            collect_note_names(&item.children, names);
        } else {
            names.push(item.name.clone());
        }
    }
}

fn headings_for(text: &str) -> Vec<Heading> {
    let document = parse_document(text);
    let mut headings = Vec::new();
    for line in &document.lines {
        let LiveLineKind::Heading { level, marker } = &line.kind else {
            continue;
        };
        let start = marker.end;
        let Some(label) = text.get(start..line.range.end) else {
            continue;
        };
        let label = label.trim().trim_end_matches('#').trim();
        if label.is_empty() {
            continue;
        }
        headings.push(Heading {
            level: *level,
            label: label.to_string(),
            byte: start,
        });
    }
    headings
}

fn collect_note_paths(items: &[Rc<notes_core::NotesTreeItem>], out: &mut Vec<PathBuf>) {
    for item in items {
        if item.is_folder {
            collect_note_paths(&item.children, out);
        } else {
            out.push(item.full_path.clone());
        }
    }
}

/// The count's noun: "1 note", "7 notes".
fn note_word(count: usize) -> &'static str {
    if count == 1 { "note" } else { "notes" }
}

/// How many notes a folder holds, at any depth.
fn note_count(item: &notes_core::NotesTreeItem) -> usize {
    if item.is_folder {
        item.children.iter().map(|child| note_count(child)).sum()
    } else {
        1
    }
}

fn backlinks_for(scope: &Path, store_root: &Path, target: &str, current: &Path) -> Vec<Backlink> {
    let mut paths = Vec::new();
    collect_note_paths(&notes_core::load_tree(scope), &mut paths);
    let mut seen = HashSet::new();
    let mut links = Vec::new();
    for path in paths {
        if path == current || !seen.insert(path.clone()) {
            continue;
        }
        let Ok(text) = notes_core::read_note(&path) else {
            continue;
        };
        let document = parse_document(&text);
        let linked = document.lines.iter().any(|line| {
            line.objects.iter().any(|object| {
                matches!(&object.kind, InlineObjectKind::WikiLink { target: link }
                    if note_key(link) == target)
            })
        });
        if !linked {
            continue;
        }
        let title = path
            .file_stem()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string_lossy().to_string());
        links.push(Backlink {
            path: path.clone(),
            title,
            relative: store_relative(store_root, &path),
        });
    }
    links.sort_by(|left, right| left.title.cmp(&right.title));
    links.truncate(200);
    links
}

fn collect_palette_items(
    items: &[Rc<notes_core::NotesTreeItem>],
    store_root: &Path,
    out: &mut Vec<PaletteItem>,
) {
    for item in items {
        if item.is_folder {
            collect_palette_items(&item.children, store_root, out);
        } else {
            out.push(PaletteItem {
                path: item.full_path.clone(),
                title: item
                    .name
                    .trim_end_matches(".md")
                    .trim_end_matches(".MD")
                    .to_string(),
                relative: store_relative(store_root, &item.full_path),
            });
        }
    }
}

fn fuzzy_score(query: &str, candidate: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let query = query.to_lowercase();
    let candidate = candidate.to_lowercase();
    let mut query_chars = query.chars();
    let mut wanted = query_chars.next()?;
    let mut score = 0i32;
    let mut streak = 0i32;
    let mut matched = 0usize;
    for (index, ch) in candidate.chars().enumerate() {
        if ch != wanted {
            streak = 0;
            continue;
        }
        score += 10 + streak * 6;
        let boundary = index == 0
            || matches!(
                candidate.chars().nth(index.saturating_sub(1)),
                Some('/' | '\\' | ' ' | '_' | '-' | '.')
            );
        if boundary {
            score += 14;
        }
        streak += 1;
        matched += 1;
        let Some(next) = query_chars.next() else {
            break;
        };
        wanted = next;
    }
    if matched == query.chars().count() {
        Some(score - candidate.len() as i32 / 8)
    } else {
        None
    }
}

fn find_tree_item(
    items: &[Rc<notes_core::NotesTreeItem>],
    path: &Path,
) -> Option<Rc<notes_core::NotesTreeItem>> {
    for item in items {
        if item.full_path == path {
            return Some(item.clone());
        }
        if item.is_folder
            && let Some(found) = find_tree_item(&item.children, path)
        {
            return Some(found);
        }
    }
    None
}

/// Depth-first search of the note tree for a file name, case-insensitive like
/// Obsidian's link resolution.
fn find_note(items: &[Rc<notes_core::NotesTreeItem>], file: &str) -> Option<PathBuf> {
    items.iter().find_map(|item| {
        if item.is_folder {
            find_note(&item.children, file)
        } else {
            item.name
                .eq_ignore_ascii_case(file)
                .then(|| item.full_path.clone())
        }
    })
}

/// The toolbar glyph buttons' spoken names — the visible labels ("B", "H1")
/// are shorthand, not names.
fn format_label(format: &str) -> &'static str {
    match format {
        "bold" => "Bold",
        "italic" => "Italic",
        "strike" => "Strikethrough",
        "code" => "Inline code",
        "link" => "Insert link",
        "h1" => "Heading 1",
        "h2" => "Heading 2",
        "h3" => "Heading 3",
        "bullet" => "Bullet list",
        "numbered" => "Numbered list",
        "task" => "Task list",
        "quote" => "Quote",
        "codeblock" => "Code block",
        "hr" => "Horizontal rule",
        _ => "Format",
    }
}

fn char_to_byte(text: &str, char_ix: usize) -> usize {
    text.char_indices()
        .nth(char_ix)
        .map(|(i, _)| i)
        .unwrap_or(text.len())
}

/// The note's folder under the store root, compacted to `~/…` when it lives
/// in the home folder. The head shows where the note sits, not its file name
/// again.
fn compact_path(store_root: &Path, path: &Path) -> String {
    let folder = path.parent().unwrap_or(path);
    let display = folder.display().to_string();
    let shown = match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && display.starts_with(&home) => {
            display.replacen(&home, "~", 1)
        }
        _ => display,
    };
    // a store root the app was handed: show it whole, not as the home prefix
    if shown.starts_with('~') || store_root.starts_with(std::env::var("HOME").unwrap_or_default()) {
        shown
    } else {
        folder.display().to_string()
    }
}

pub(crate) fn resolve_store_path(raw: &str) -> PathBuf {
    let raw = raw.trim();
    if raw.is_empty() {
        return crate::paths::default_store_root();
    }
    let expanded = if raw == "~" {
        warpui_kit_platform::paths::home()
    } else if let Some(rest) = raw.strip_prefix("~/") {
        warpui_kit_platform::paths::home().join(rest)
    } else {
        PathBuf::from(raw)
    };
    if expanded.is_absolute() {
        expanded
    } else {
        std::path::absolute(&expanded).unwrap_or(expanded)
    }
}

fn store_relative(store_root: &Path, path: &Path) -> String {
    path.strip_prefix(store_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|age| age.as_millis())
        .unwrap_or(0)
}

fn rewrite_path(path: &Path, old: &Path, new: &Path) -> PathBuf {
    if path == old {
        return new.to_path_buf();
    }
    match path.strip_prefix(old) {
        Ok(relative) => new.join(relative),
        Err(_) => path.to_path_buf(),
    }
}

fn match_ranges(text: &str, term: &str) -> Vec<std::ops::Range<usize>> {
    if term.is_empty() {
        return Vec::new();
    }
    let lower = text.to_lowercase();
    let term = term.to_lowercase();
    if lower.len() != text.len() {
        // Lowercasing changed the byte length (a non-ASCII case pair), so the
        // offsets would not map back to the original string.
        return Vec::new();
    }
    let mut ranges = Vec::new();
    let mut start = 0;
    while let Some(found) = lower[start..].find(&term) {
        let begin = start + found;
        let end = begin + term.len();
        ranges.push(begin..end);
        start = end.max(begin + 1);
    }
    ranges
}

fn highlighted_excerpt(window: &mut Window, text: &str, term: &str) -> gpui::AnyElement {
    let mut style = window.text_style();
    style.font_size = gpui::AbsoluteLength::Pixels(px(theme::font_meta()));
    style.color = theme::text_faint().into();
    let highlight = gpui::HighlightStyle {
        background_color: Some(theme::accent_soft().into()),
        font_weight: Some(FontWeight::SEMIBOLD),
        ..Default::default()
    };
    let highlights: Vec<_> = match_ranges(text, term)
        .into_iter()
        .map(|range| (range, highlight))
        .collect();
    gpui::StyledText::new(text.to_string())
        .with_default_highlights(&style, highlights)
        .into_any_element()
}

fn rewrite_pref_list(list: &mut [String], store_root: &Path, old: &Path, new: &Path) {
    for entry in list {
        let absolute = store_root.join(&*entry);
        let rewritten = rewrite_path(&absolute, old, new);
        if rewritten != absolute {
            *entry = store_relative(store_root, &rewritten);
        }
    }
}

fn resolve_folder(scope: &Path, value: &str) -> anyhow::Result<PathBuf> {
    let mut target = scope.to_path_buf();
    if !value.trim().is_empty() {
        for segment in value.trim().split(['/', '\\']) {
            if segment.is_empty() {
                continue;
            }
            let name = notes_core::sanitize_entry_name(segment, false)
                .ok_or_else(|| anyhow::anyhow!("folder name is not usable"))?;
            target.push(name);
        }
    }
    if !target.is_dir() {
        anyhow::bail!("folder does not exist");
    }
    Ok(target)
}

fn menu_item(
    id: &'static str,
    label: impl Into<SharedString>,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> gpui::AnyElement {
    let label: SharedString = label.into();
    div()
        .id(id)
        .debug_selector(move || id.into())
        .a11y_button(label.clone())
        .cursor_pointer()
        .rounded(px(6.))
        .px_2()
        .py_1p5()
        .text_size(px(theme::font_body()))
        .text_color(theme::text_dim())
        .hover(|style| style.bg(theme::raised()).text_color(theme::text()))
        .on_click(on_click)
        .child(label)
        .into_any_element()
}

fn menu_item_danger(
    id: &'static str,
    label: impl Into<SharedString>,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> gpui::AnyElement {
    let label: SharedString = label.into();
    div()
        .id(id)
        .debug_selector(move || id.into())
        .a11y_button(label.clone())
        .cursor_pointer()
        .rounded(px(6.))
        .px_2()
        .py_1p5()
        .text_size(px(theme::font_body()))
        .text_color(theme::DANGER)
        .hover(|style| style.bg(theme::tint(theme::DANGER)))
        .on_click(on_click)
        .child(label)
        .into_any_element()
}

/// The note's age for the head's right side: now, 5m, 2h, 3d.
///
/// `stat` is a syscall, and the head re-renders on every keystroke and every
/// caret move — so the answer is cached per path and recomputed only once the
/// bucket it prints could have changed (a minute for "now", an hour for "5m",
/// a day for "2h"). A miss on a path we just asked about is cached too: the
/// old version re-stat'ed a missing file on every frame.
fn edited_age(path: &Path, cache: &mut Option<(PathBuf, String, std::time::Instant)>) -> String {
    const TTL: std::time::Duration = std::time::Duration::from_secs(60);
    if let Some((cached_path, label, at)) = cache
        && cached_path == path
        && at.elapsed() < TTL
    {
        return label.clone();
    }
    let now = std::time::Instant::now();
    let label = age_label(
        std::fs::metadata(path)
            .ok()
            .and_then(|meta| meta.modified().ok()),
    );
    *cache = Some((path.to_path_buf(), label.clone(), now));
    label
}

/// A note's age as the list and the head print it: now, 5m, 2h, 3d.
fn age_label(modified: Option<std::time::SystemTime>) -> String {
    modified
        .and_then(|modified| std::time::SystemTime::now().duration_since(modified).ok())
        .map(|age| {
            let elapsed = age.as_secs();
            match elapsed {
                0..=59 => "now".to_string(),
                60..=3599 => format!("{}m", elapsed / 60),
                3600..=86_399 => format!("{}h", elapsed / 3600),
                _ => format!("{}d", elapsed / 86_400),
            }
        })
        .unwrap_or_else(|| "now".to_string())
}

impl Render for NotesPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        if let Some(requested) = self.store_path_pending.take() {
            self.apply_store_path(requested, window, cx);
        }
        self.reload_tree();
        if std::mem::take(&mut self.note_names_stale) {
            let mut names = Vec::new();
            collect_note_names(&self.tree, &mut names);
            self.editor
                .update(cx, |state, cx| state.set_note_names(names, cx));
        }
        // value() hands out a SharedString (a refcount bump); the per-frame
        // work below stays allocation-free on &str so a long note does not
        // re-copy its whole text every keystroke
        let editor_text = self.editor.read(cx).value();
        let dirty = self.open_note.is_some() && editor_text.as_ref() != self.saved_text.as_str();
        let page_focus = self.page_focus.clone();
        let mut root = div()
            .size_full()
            .flex()
            .flex_col()
            .min_h_0()
            .bg(theme::background())
            .track_focus(&page_focus)
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                let control = event.keystroke.modifiers.control;
                if control && event.keystroke.key == "s" {
                    this.save_current(cx);
                    cx.stop_propagation();
                } else if control && event.keystroke.key == "n" {
                    this.create_open = true;
                    this.create_name
                        .update(cx, |field, cx| field.focus(window, cx));
                    cx.notify();
                    cx.stop_propagation();
                } else if control && event.keystroke.key == "\\" {
                    this.toggle_list(cx);
                    cx.stop_propagation();
                } else if control && event.keystroke.key == "k" {
                    // the design's Ctrl+K: the title bar's search field
                    this.search.update(cx, |field, cx| field.focus(window, cx));
                    cx.stop_propagation();
                } else if control && event.keystroke.key == "p" {
                    this.open_palette(window, cx);
                    cx.stop_propagation();
                } else if control && event.keystroke.modifiers.shift && event.keystroke.key == "f" {
                    this.open_search_results(window, cx);
                    cx.stop_propagation();
                } else if control && event.keystroke.modifiers.shift && event.keystroke.key == "e" {
                    window.focus(&this.tree_focus, cx);
                    cx.stop_propagation();
                } else if control && event.keystroke.modifiers.shift && event.keystroke.key == "b" {
                    this.toggle_context_panel(cx);
                    cx.stop_propagation();
                } else if control && event.keystroke.key == "w" {
                    if !this.open_tabs.is_empty() {
                        this.close_tab(this.active_tab, window, cx);
                    }
                    cx.stop_propagation();
                } else if control && event.keystroke.key == "tab" {
                    let len = this.open_tabs.len();
                    if len > 1 {
                        let step = if event.keystroke.modifiers.shift {
                            len - 1
                        } else {
                            1
                        };
                        this.activate_tab((this.active_tab + step) % len, window, cx);
                    }
                    cx.stop_propagation();
                } else if control
                    && event
                        .keystroke
                        .key
                        .parse::<usize>()
                        .is_ok_and(|digit| (1..=9).contains(&digit))
                {
                    if let Ok(digit) = event.keystroke.key.parse::<usize>()
                        && digit <= this.open_tabs.len()
                    {
                        this.activate_tab(digit - 1, window, cx);
                    }
                    cx.stop_propagation();
                } else if event.keystroke.modifiers.alt
                    && matches!(event.keystroke.key.as_str(), "left" | "right")
                {
                    let delta = if event.keystroke.key == "left" { -1 } else { 1 };
                    this.go_history(delta, window, cx);
                    cx.stop_propagation();
                } else if control && event.keystroke.key == "/" {
                    this.shortcut_open = !this.shortcut_open;
                    cx.notify();
                    cx.stop_propagation();
                } else if control && event.keystroke.key == "," {
                    this.set_section(
                        match this.section {
                            Section::Settings => Section::Notes,
                            Section::Notes | Section::About => Section::Settings,
                        },
                        cx,
                    );
                    cx.stop_propagation();
                } else if control && event.keystroke.key == "f" && this.open_note.is_some() {
                    this.find_open = true;
                    this.find.update(cx, |field, cx| field.focus(window, cx));
                    let term = this.find.read(cx).value().to_string();
                    this.run_find(term, cx);
                    cx.notify();
                    cx.stop_propagation();
                } else if event.keystroke.key == "escape" && this.about_license_open {
                    this.about_license_open = false;
                    cx.notify();
                    cx.stop_propagation();
                } else if event.keystroke.key == "escape" && this.find_open {
                    this.close_find(window, cx);
                    cx.stop_propagation();
                } else if event.keystroke.key == "escape" && this.create_open {
                    this.create_open = false;
                    cx.notify();
                    cx.stop_propagation();
                } else if event.keystroke.key == "escape" && this.shortcut_open {
                    this.shortcut_open = false;
                    cx.notify();
                    cx.stop_propagation();
                }
            }))
            // the keybinding path: the action fires even where a focused
            // editor swallows the key event itself
            .on_action(cx.listener(|this, _: &OpenFind, window, cx| {
                if this.open_note.is_some() {
                    this.find_open = true;
                    this.find.update(cx, |field, cx| field.focus(window, cx));
                    let term = this.find.read(cx).value().to_string();
                    this.run_find(term, cx);
                    cx.notify();
                }
            }))
            .child(self.render_titlebar(window, cx))
            .child(
                div()
                    .id("notes-body")
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .overflow_hidden()
                    .flex()
                    .flex_row()
                    .child(self.render_rail(cx))
                    .child(self.render_body(dirty, window, cx)),
            )
            .relative();
        // the search results float over the panes, under the title bar field
        if !self.search_results_open
            && let Some(popup) = self.render_search_popup(window, cx)
        {
            root = root.child(popup);
        }
        if let Some(toasts) = self.render_toasts(cx) {
            root = root.child(toasts);
        }
        if let Some(menu) = self.row_menu.clone() {
            root = root.child(self.render_row_menu(window, cx, menu));
        }
        if let Some(prompt) = self.prompt.clone() {
            root = root.child(self.render_prompt(window, cx, prompt));
        }
        if self.trash_open {
            root = root.child(self.render_trash_dialog(window, cx));
        }
        if self.shortcut_open {
            root = root.child(self.render_shortcuts(cx));
        }
        if self.palette_open {
            root = root.child(self.render_palette(window, cx));
        }
        if self.search_results_open {
            root = root.child(self.render_search_results(window, cx));
        }
        if self.about_license_open {
            let editor = self.about_license.get_or_insert_with(|| {
                warpui_kit_markdown::read_only_document(
                    crate::ui::about::LICENSE.into(),
                    window,
                    cx,
                )
            });
            let body = warpui_kit_markdown::render_read_only_document_in_card(editor, cx);
            root = root.child(warpui_kit_core::about::license_dialog(
                "about-license",
                "GNU General Public License v3.0",
                body,
                cx.listener(|this, _, _, cx| {
                    this.about_license_open = false;
                    cx.notify();
                }),
            ));
        }
        root
    }
}

impl NotesPage {
    /// The list pane's visibility, wherever it is toggled from — the state
    /// and its remembered copy move together.
    fn toggle_list(&mut self, cx: &mut Context<Self>) {
        self.list_visible = !self.list_visible;
        self.prefs.list_visible = self.list_visible;
        self.prefs.save();
        cx.notify();
    }

    /// The rail's open section, remembered so a restart reopens it.
    fn set_section(&mut self, section: Section, cx: &mut Context<Self>) {
        self.section = section;
        self.prefs.section = match section {
            Section::Notes => "notes",
            Section::About => "about",
            Section::Settings => "settings",
        }
        .into();
        self.prefs.save();
        cx.notify();
    }

    /// The rail: the tree and editor's section, with Settings pinned to the
    /// bottom.
    fn render_rail(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let section = self.section;
        shell::rail()
            .debug_selector(|| "notes-rail".into())
            .child(
                shell::rail_item(
                    "rail-notes",
                    "icons/icon-note.svg",
                    "Notes",
                    section == Section::Notes,
                )
                .debug_selector(|| "rail-notes".into())
                .on_click(cx.listener(|this, _, _, cx| {
                    this.set_section(Section::Notes, cx);
                })),
            )
            .child(div().flex_1())
            .child(
                shell::rail_item(
                    "rail-about",
                    "icons/icon-about.svg",
                    "About",
                    section == Section::About,
                )
                .debug_selector(|| "rail-about".into())
                .on_click(cx.listener(|this, _, _, cx| {
                    this.set_section(Section::About, cx);
                })),
            )
            .child(
                shell::rail_item(
                    "rail-settings",
                    "icons/icon-cog.svg",
                    "Settings",
                    section == Section::Settings,
                )
                .debug_selector(|| "rail-settings".into())
                .on_click(cx.listener(|this, _, _, cx| {
                    this.set_section(Section::Settings, cx);
                })),
            )
            .into_any_element()
    }

    /// The strip under the titlebar: the list's own tab, then one per open
    /// note. The family's shape — `shell::tab_strip()` on the window chrome,
    /// the selected tab joining the surface under it.
    fn render_tab_strip(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let active_text = self.editor.read(cx).value().to_string();
        let tabs: Vec<TabChip> = self
            .open_tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| {
                let dirty = if index == self.active_tab {
                    active_text != self.saved_text
                } else {
                    self.dirty.contains_key(&tab.path) || tab.conflict
                };
                let title = tab
                    .path
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| "untitled".into());
                TabChip {
                    title,
                    active: index == self.active_tab,
                    dirty,
                    preview: tab.preview,
                }
            })
            .collect();
        let mut strip = shell::tab_strip()
            .id("notes-tab-strip")
            .debug_selector(|| "notes-tab-strip".into())
            .overflow_x_scroll()
            // the list's own tab, as station's "Repositories": it names the
            // pane below and gives the strip a resting selection
            .child(
                shell::strip_tab(
                    "notes-strip-home",
                    "Notes",
                    tabs.is_empty(),
                    theme::surface(),
                )
                .debug_selector(|| "notes-strip-home".into())
                .flex_shrink_0()
                .child(icon(
                    "icons/icon-note.svg",
                    px(theme::icon_small()),
                    theme::text_dim(),
                ))
                .child("Notes")
                .on_click(cx.listener(|this, _, window, cx| {
                    window.focus(&this.tree_focus, cx);
                    cx.notify();
                })),
            );
        for (index, chip) in tabs.iter().enumerate() {
            strip = strip.child(
                shell::strip_tab(
                    SharedString::from(format!("note-tab-{index}")),
                    chip.title.clone(),
                    chip.active,
                    theme::panel(),
                )
                .debug_selector(move || format!("note-tab-{index}"))
                .flex_shrink_0()
                .on_click(
                    cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
                        if event.click_count() > 1 {
                            this.keep_tab(index, window, cx);
                        } else {
                            this.activate_tab(index, window, cx);
                        }
                    }),
                )
                .on_mouse_down(
                    MouseButton::Middle,
                    cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.close_tab(index, window, cx);
                    }),
                )
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        // a preview tab is italic, as in VS Code
                        .when(chip.preview, |title| title.italic())
                        .child(chip.title.clone()),
                )
                .when(chip.dirty, |tab| {
                    tab.child(
                        div()
                            .flex_shrink_0()
                            .size(px(6.))
                            .rounded_full()
                            .bg(theme::CHANGES),
                    )
                })
                .child(
                    div()
                        .id(("note-tab-close", index as u64))
                        .debug_selector(move || format!("note-tab-close-{index}"))
                        .a11y_button("Close tab")
                        .cursor_pointer()
                        .flex_shrink_0()
                        .rounded(px(4.))
                        .p_0p5()
                        .hover(|close| close.bg(theme::raised()))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.close_tab(index, window, cx);
                        }))
                        .child(icon(
                            "icons/icon-close.svg",
                            px(theme::icon_tiny() - 2.),
                            theme::text_faint(),
                        )),
                ),
            );
        }
        strip = strip.child(
            div()
                .id("notes-tab-new")
                .debug_selector(|| "notes-tab-new".into())
                .a11y_button("New tab")
                .cursor_pointer()
                .flex_shrink_0()
                .size(px(26.))
                .ml_1()
                .rounded(px(6.))
                .flex()
                .items_center()
                .justify_center()
                .hover(|button| button.bg(theme::raised()))
                .on_click(cx.listener(|this, _, window, cx| this.open_palette(window, cx)))
                .child(icon(
                    "icons/icon-plus.svg",
                    px(theme::icon_small() - 2.),
                    theme::text_muted(),
                )),
        );
        strip.into_any_element()
    }

    fn render_context_panel(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let tab_button = |id: &'static str, label: &'static str, active: bool| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .a11y_tab(label, active)
                .relative()
                .cursor_pointer()
                .h(px(theme::scaled(32.)))
                .px_2()
                .flex()
                .items_center()
                .text_size(px(theme::font_row_meta()))
                .when(active, |button| {
                    button
                        .text_color(theme::text())
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(
                            div()
                                .absolute()
                                .bottom_0()
                                .left_0()
                                .right_0()
                                .h(px(2.))
                                .child(motion::grow_mark(
                                    SharedString::from(format!("context-tab-mark-{id}")),
                                    theme::accent(),
                                )),
                        )
                })
                .when(!active, |button| {
                    button
                        .text_color(theme::text_dim())
                        .hover(|button| button.text_color(theme::text()))
                })
                .child(label)
        };
        let mut header = div()
            .flex()
            .flex_row()
            .items_center()
            .gap_1()
            .px_2()
            .h(px(theme::scaled(36.)))
            .border_b_1()
            .border_color(theme::hairline());
        for tab in [ContextTab::Outline, ContextTab::Links] {
            let (id, label) = match tab {
                ContextTab::Outline => ("context-tab-outline", "Outline"),
                ContextTab::Links => ("context-tab-links", "Links"),
            };
            header = header.child(tab_button(id, label, self.context_tab == tab).on_click(
                cx.listener(move |this, _, _, cx| {
                    this.set_context_tab(tab, cx);
                }),
            ));
        }
        header = header
            .child(div().flex_1())
            .child(
                div()
                    .id("context-refresh")
                    .debug_selector(|| "context-refresh".into())
                    .a11y_button("Refresh panel")
                    .cursor_pointer()
                    .rounded(px(4.))
                    .p_0p5()
                    .hover(|button| button.bg(theme::raised()))
                    .on_click(cx.listener(|this, _, _, cx| {
                        if this.context_tab == ContextTab::Links {
                            this.refresh_backlinks(cx);
                        } else {
                            this.outline_cache = None;
                            cx.notify();
                        }
                    }))
                    .child(icon(
                        "icons/icon-refresh.svg",
                        px(theme::icon_tiny()),
                        theme::text_faint(),
                    )),
            )
            .child(
                div()
                    .id("context-close")
                    .debug_selector(|| "context-close".into())
                    .a11y_button("Close panel")
                    .cursor_pointer()
                    .rounded(px(4.))
                    .p_0p5()
                    .hover(|button| button.bg(theme::raised()))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.context_open = false;
                        cx.notify();
                    }))
                    .child(icon(
                        "icons/icon-close.svg",
                        px(theme::icon_tiny()),
                        theme::text_faint(),
                    )),
            );

        let mut list = div()
            .id("context-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_0p5()
            .p_2();
        match self.context_tab {
            ContextTab::Outline => {
                let headings = self.outline_headings(cx);
                if headings.is_empty() {
                    list = list.child(
                        div()
                            .px_2()
                            .py_2()
                            .text_size(px(theme::font_meta()))
                            .text_color(theme::text_faint())
                            .child("No headings."),
                    );
                }
                for (index, heading) in headings.iter().enumerate() {
                    let byte = heading.byte;
                    list = list.child(
                        div()
                            .id(("context-heading", index as u64))
                            .debug_selector(move || format!("context-heading-{index}"))
                            .a11y_button(format!("Jump to {}", heading.label))
                            .cursor_pointer()
                            .rounded(px(4.))
                            .py_0p5()
                            .pl(px(6. + heading.level.saturating_sub(1) as f32 * 10.))
                            .pr_2()
                            .truncate()
                            .text_size(px(theme::font_meta()))
                            .text_color(theme::text_dim())
                            .hover(|row| row.bg(theme::raised()))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.editor.update(cx, |editor, cx| {
                                    editor.focus(window, cx);
                                    editor.set_selected_range(byte..byte, cx);
                                });
                                cx.notify();
                            }))
                            .child(heading.label.clone()),
                    );
                }
            }
            ContextTab::Links => {
                if self.backlinks_building {
                    list = list.child(
                        div()
                            .px_2()
                            .py_2()
                            .text_size(px(theme::font_meta()))
                            .text_color(theme::text_faint())
                            .child("Scanning notes…"),
                    );
                } else if self.backlinks.is_empty() {
                    list = list.child(
                        div()
                            .px_2()
                            .py_2()
                            .text_size(px(theme::font_meta()))
                            .text_color(theme::text_faint())
                            .child("No backlinks."),
                    );
                }
                for (index, link) in self.backlinks.iter().enumerate() {
                    let path = link.path.clone();
                    list = list.child(
                        div()
                            .id(("context-link", index as u64))
                            .debug_selector(move || format!("context-link-{index}"))
                            .a11y_button(format!("Open {}", link.title))
                            .cursor_pointer()
                            .rounded(px(4.))
                            .px_2()
                            .py_1()
                            .flex()
                            .flex_col()
                            .hover(|row| row.bg(theme::raised()))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_note_anywhere(path.clone(), window, cx);
                            }))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(theme::font_body()))
                                    .text_color(theme::text())
                                    .child(link.title.clone()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(theme::font_meta()))
                                    .text_color(theme::text_faint())
                                    .child(link.relative.clone()),
                            ),
                    );
                }
            }
        }

        div()
            .id("notes-context-panel")
            .debug_selector(|| "notes-context-panel".into())
            .w(px(theme::CONTEXT_WIDTH))
            .h_full()
            .flex_shrink_0()
            .border_l_1()
            .border_color(theme::border_light())
            .bg(theme::panel())
            .flex()
            .flex_col()
            .child(header)
            .child(list)
            .into_any_element()
    }

    /// Everything right of the rail: the list and the editor (or Settings).
    fn render_body(
        &mut self,
        dirty: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        match self.section {
            Section::About => self.render_about(cx),
            Section::Settings => self.render_settings(window, cx),
            Section::Notes => {
                let mut panes = shell::panes().min_w_0();
                if self.list_visible {
                    panes = panes
                        .child(self.render_tree(window, cx))
                        .child(self.render_splitter(cx));
                }
                // the head spans the pane; the text keeps its reading measure
                let head = self
                    .open_note
                    .is_some()
                    .then(|| self.render_editor_head(dirty, cx));
                let body = div()
                    .id("notes-editor-body")
                    .debug_selector(|| "notes-editor-body".into())
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .max_w(px(theme::scaled(shell::READING_WIDTH)))
                    .mx_auto()
                    .flex()
                    .flex_col()
                    .when(self.open_note.is_some() && self.conflict, |column| {
                        column.child(self.render_conflict_bar(cx))
                    })
                    .when(self.find_open, |column| {
                        column.child(self.render_find_bar(window, cx))
                    })
                    .child(self.render_editor(cx))
                    .when(self.open_note.is_some(), |column| {
                        column.child(self.render_status_bar(cx))
                    });
                let body_key = SharedString::from(match self.open_note.as_ref() {
                    Some(path) => format!("note-body-{}", path.display()),
                    None => "note-body-empty".to_string(),
                });
                let mut detail = div()
                    .id("notes-detail")
                    .debug_selector(|| "notes-detail".into())
                    .flex()
                    .flex_row()
                    .flex_1()
                    .w_full()
                    .h_full()
                    .min_h_0()
                    .min_w_0()
                    .overflow_hidden()
                    .child(
                        div()
                            .id("notes-editor-column")
                            .debug_selector(|| "notes-editor-column".into())
                            .flex_1()
                            .flex_shrink(1.)
                            .min_w_0()
                            .h_full()
                            .flex()
                            .flex_col()
                            .min_h_0()
                            .children(head)
                            .child(motion::enter(body_key, body)),
                    );
                if self.context_open {
                    detail = detail.child(self.render_context_panel(window, cx));
                }
                let panes = panes.child(shell::detail_pane().child(detail));
                // titlebar, then the strip over the whole stage, then the two
                // panes — station's stack
                shell::stage()
                    .child(self.render_tab_strip(cx))
                    .child(panes)
                    .into_any_element()
            }
        }
    }

    /// The drag edge between the list and the detail.
    fn render_splitter(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let drag_state = self.splitter_drag.clone();
        let drag_for_paint = self.splitter_drag.clone();
        let weak = cx.weak_entity();
        let width_now = self.prefs.list_width;
        div()
            .id("notes-splitter")
            .relative()
            .w(px(SPLITTER_WIDTH))
            .h_full()
            .flex_shrink_0()
            .bg(theme::well())
            .cursor_col_resize()
            .hover(|s| s.bg(theme::line_strong()))
            .on_mouse_down(
                MouseButton::Left,
                move |event: &gpui::MouseDownEvent, _, cx| {
                    drag_state.set(Some((event.position.x, width_now)));
                    cx.stop_propagation();
                },
            )
            .child(
                canvas(
                    // this gpui rev: canvas(prepaint, paint) — on_mouse_event
                    // must register from the second closure, prepaint panics
                    // on it
                    |_, _, _| {},
                    move |_, _, window, _| {
                        // registered during paint, as this gpui rev requires
                        if drag_for_paint.get().is_some() {
                            let drag = drag_for_paint.clone();
                            window.on_mouse_event(
                                move |event: &gpui::MouseMoveEvent, phase, _window, cx| {
                                    if phase != gpui::DispatchPhase::Bubble {
                                        return;
                                    }
                                    let Some((start_x, start_width)) = drag.get() else {
                                        return;
                                    };
                                    match event.pressed_button {
                                        Some(MouseButton::Left) => {
                                            let delta = (event.position.x - start_x).as_f32();
                                            if let Some(page) = weak.upgrade() {
                                                page.update(cx, |page, cx| {
                                                    page.prefs.list_width = shell::clamp_list_width(
                                                        start_width + delta,
                                                    );
                                                    cx.notify();
                                                });
                                            }
                                        }
                                        _ => {
                                            drag.set(None);
                                            if let Some(page) = weak.upgrade() {
                                                page.update(cx, |page, _| page.prefs.save());
                                            }
                                            cx.stop_propagation();
                                        }
                                    }
                                },
                            );
                        }
                    },
                )
                .absolute()
                .size_full(),
            )
            .into_any_element()
    }

    /// The toast stack, bottom right: errors carry a close button and a
    /// danger border; everything leaves on its own. Painted last so it rides
    /// above the tree and the editor.
    fn render_toasts(&mut self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if let Some(notice) = &self.notice
            && notice.expires <= std::time::Instant::now()
        {
            self.notice = None;
        }
        let notice = self.notice.clone()?;
        let error = notice.kind == NoticeKind::Error;
        let mut toast = div()
            .id("notes-toast")
            .debug_selector(|| "notes-toast".into())
            .role(gpui::Role::Alert)
            .aria_label(notice.text.clone())
            .occlude()
            .max_w(px(420.))
            .flex()
            .flex_row()
            .items_start()
            .gap_2()
            .rounded(px(8.))
            .border_1()
            .border_color(if error {
                theme::tint(theme::DANGER)
            } else {
                gpui::Hsla::from(theme::hairline())
            })
            .bg(theme::overlay())
            .shadow_lg()
            .p_2p5()
            .child(icon(
                if error {
                    "icons/icon-alert.svg"
                } else {
                    "icons/icon-info.svg"
                },
                px(theme::icon_small()),
                if error {
                    theme::DANGER
                } else {
                    theme::text_dim()
                },
            ))
            .child(
                div()
                    .min_w_0()
                    .text_size(px(theme::font_body()))
                    .text_color(if error {
                        theme::text()
                    } else {
                        theme::text_dim()
                    })
                    .child(notice.text.clone()),
            );
        if let Some(action) = notice.action.clone() {
            toast = toast.child(
                div()
                    .id("notes-toast-action")
                    .debug_selector(|| "notes-toast-action".into())
                    .a11y_button("Undo")
                    .cursor_pointer()
                    .flex_shrink_0()
                    .rounded(px(6.))
                    .px_2()
                    .py_0p5()
                    .text_size(px(theme::font_control()))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::accent_ink())
                    .hover(|style| style.bg(theme::raised()))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.notice = None;
                        match action.clone() {
                            NoticeAction::UndoTrash { entry, reopen } => {
                                this.undo_trash(entry, reopen, window, cx);
                            }
                        }
                    }))
                    .child("Undo"),
            );
        }
        if error {
            toast = toast.child(
                div()
                    .id("notes-toast-close")
                    .a11y_button("Dismiss notification")
                    .cursor_pointer()
                    .flex_shrink_0()
                    .rounded(px(6.))
                    .p_0p5()
                    .hover(|s| s.bg(theme::raised()))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.notice = None;
                        cx.notify();
                    }))
                    .child(icon(
                        "icons/icon-close.svg",
                        px(theme::icon_small() - 4.),
                        theme::text_faint(),
                    )),
            );
        }
        Some(
            div()
                .absolute()
                .bottom_3()
                .right_3()
                .flex()
                .flex_col()
                .items_end()
                // The animation sets `position: relative` for its rise; it must
                // live INSIDE the absolute layer or the toast joins the root
                // flex column and pushes the whole page up.
                .child(motion::enter(("toast-enter", notice.seq), toast))
                .into_any_element(),
        )
    }

    fn render_row_menu(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
        menu: RowMenu,
    ) -> gpui::AnyElement {
        let max_x = (window.viewport_size().width.as_f32() - theme::MENU_WIDTH - 8.).max(8.);
        let max_y = (window.viewport_size().height.as_f32() - 320.).max(8.);
        let x = menu.x.clamp(8., max_x);
        let y = menu.y.clamp(8., max_y);
        let path = menu.path.clone();
        let is_folder = menu.is_folder;
        let pinned = self.is_pinned(&path);
        let card = warpui_kit_core::controls::popover_card()
            .id("notes-row-menu")
            .debug_selector(|| "notes-row-menu".into())
            .absolute()
            .left(px(x))
            .top(px(y))
            .w(px(theme::MENU_WIDTH))
            .occlude()
            .flex()
            .flex_col()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(menu_item(
                "notes-menu-open",
                "Open",
                cx.listener({
                    let path = path.clone();
                    move |this, _, window, cx| {
                        this.open_tree_entry(path.clone(), is_folder, window, cx);
                    }
                }),
            ))
            .child(menu_item(
                "notes-menu-open-tab",
                "Open in new tab",
                cx.listener({
                    let path = path.clone();
                    move |this, _, window, cx| {
                        this.row_menu = None;
                        this.tree_cursor = Some(path.clone());
                        this.open_note_new_tab(path.clone(), window, cx);
                    }
                }),
            ))
            .child(menu_item(
                "notes-menu-rename",
                "Rename…",
                cx.listener({
                    let path = path.clone();
                    move |this, _, window, cx| {
                        this.row_menu = None;
                        this.start_prompt(PromptKind::Rename, path.clone(), window, cx);
                    }
                }),
            ))
            .when(!is_folder, |card| {
                card.child(menu_item(
                    "notes-menu-duplicate",
                    "Duplicate",
                    cx.listener({
                        let path = path.clone();
                        move |this, _, window, cx| {
                            this.row_menu = None;
                            this.duplicate_path(path.clone(), window, cx);
                        }
                    }),
                ))
            })
            .child(menu_item(
                "notes-menu-move",
                "Move to folder…",
                cx.listener({
                    let path = path.clone();
                    move |this, _, window, cx| {
                        this.row_menu = None;
                        this.start_prompt(PromptKind::MoveTo, path.clone(), window, cx);
                    }
                }),
            ))
            .child(menu_item(
                "notes-menu-new-note",
                "New note here",
                cx.listener({
                    let path = path.clone();
                    move |this, _, window, cx| {
                        this.row_menu = None;
                        this.new_note_here(path.clone(), window, cx);
                    }
                }),
            ))
            .child(menu_item(
                "notes-menu-new-folder",
                "New folder…",
                cx.listener({
                    let path = path.clone();
                    move |this, _, window, cx| {
                        this.row_menu = None;
                        let target = if is_folder {
                            path.clone()
                        } else {
                            path.parent().map(Path::to_path_buf).unwrap_or(path.clone())
                        };
                        this.start_prompt(PromptKind::NewFolder, target, window, cx);
                    }
                }),
            ))
            .child(div().h(px(1.)).my_1().bg(theme::hairline()))
            .child(menu_item(
                "notes-menu-copy-path",
                "Copy path",
                cx.listener({
                    let path = path.clone();
                    move |this, _, _, cx| {
                        this.row_menu = None;
                        this.copy_entry_path(&path, cx);
                    }
                }),
            ))
            .child(menu_item(
                "notes-menu-pin",
                if pinned { "Unpin note" } else { "Pin note" },
                cx.listener({
                    let path = path.clone();
                    move |this, _, _, cx| {
                        this.row_menu = None;
                        this.toggle_pin(&path, cx);
                    }
                }),
            ))
            .child(menu_item(
                "notes-menu-trash-dialog",
                "Trash…",
                cx.listener(|this, _, _, cx| {
                    this.open_trash_dialog(cx);
                }),
            ))
            .child(menu_item_danger(
                "notes-menu-trash",
                "Move to Trash",
                cx.listener({
                    let path = path.clone();
                    move |this, _, window, cx| {
                        this.row_menu = None;
                        this.trash_path(path.clone(), window, cx);
                    }
                }),
            ));
        div()
            .id("notes-row-menu-scrim")
            .absolute()
            .inset_0()
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.row_menu = None;
                    cx.notify();
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, _, _, cx| {
                    this.row_menu = None;
                    cx.notify();
                }),
            )
            .child(card)
            .into_any_element()
    }

    fn render_prompt(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        prompt: PromptState,
    ) -> gpui::AnyElement {
        let kind = prompt.kind;
        let hint = match kind {
            PromptKind::Rename => "The name becomes the file name. Notes keep their .md extension.",
            PromptKind::NewFolder => "The folder is created under the selected folder.",
            PromptKind::MoveTo => "The folder path is relative to the current tree scope.",
        };
        let card = div()
            .id("notes-prompt-card")
            .debug_selector(|| "notes-prompt-card".into())
            .role(gpui::Role::Dialog)
            .occlude()
            .w(px(theme::PROMPT_WIDTH))
            .rounded(px(8.))
            .border_1()
            .border_color(theme::line_strong())
            .bg(theme::panel())
            .shadow_lg()
            .p_4()
            .flex()
            .flex_col()
            .gap_2()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                if event.keystroke.key == "enter" {
                    this.confirm_prompt(window, cx);
                    cx.stop_propagation();
                } else if event.keystroke.key == "escape" {
                    this.prompt = None;
                    cx.notify();
                    cx.stop_propagation();
                }
            }))
            .child(
                div()
                    .text_size(px(theme::font_screen_title()))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::text())
                    .child(kind.title()),
            )
            .child(
                div()
                    .text_size(px(theme::font_meta()))
                    .text_color(theme::text_muted())
                    .child(hint),
            )
            .child(warpui_kit_core::field::text(
                "notes-prompt-input",
                &self.prompt_input,
                window,
                cx,
            ))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_end()
                    .gap_2()
                    .pt_1()
                    .child(
                        div()
                            .id("notes-prompt-cancel")
                            .debug_selector(|| "notes-prompt-cancel".into())
                            .a11y_button("Cancel")
                            .cursor_pointer()
                            .rounded(px(6.))
                            .h(px(theme::scaled(shell::CONTROL_HEIGHT) - 8.))
                            .px_3()
                            .flex()
                            .items_center()
                            .text_size(px(theme::font_control()))
                            .text_color(theme::text_muted())
                            .hover(|style| style.bg(theme::raised()))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.prompt = None;
                                cx.notify();
                            }))
                            .child("Cancel"),
                    )
                    .child(
                        div()
                            .id("notes-prompt-confirm")
                            .debug_selector(|| "notes-prompt-confirm".into())
                            .a11y_button(kind.confirm())
                            .cursor_pointer()
                            .rounded(px(6.))
                            .h(px(theme::scaled(shell::CONTROL_HEIGHT) - 8.))
                            .px_3()
                            .flex()
                            .items_center()
                            .bg(theme::accent())
                            .text_color(theme::on_accent())
                            .text_size(px(theme::font_control()))
                            .font_weight(FontWeight::MEDIUM)
                            .hover(|style| style.opacity(0.9))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.confirm_prompt(window, cx);
                            }))
                            .child(kind.confirm()),
                    ),
            );
        div()
            .id("notes-prompt-scrim")
            .absolute()
            .inset_0()
            .occlude()
            .bg(theme::SCRIM)
            .flex()
            .items_center()
            .justify_center()
            .p_4()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.prompt = None;
                    cx.notify();
                }),
            )
            .child(card)
            .into_any_element()
    }

    fn render_trash_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let entries = crate::trash::list_trash(&self.store_root);
        let count = entries.len();
        let now = now_ms();
        let mut list = div()
            .id("trash-list")
            .debug_selector(|| "trash-list".into())
            .flex()
            .flex_col()
            .gap_1()
            .max_h(px(360.))
            .overflow_y_scroll();
        if entries.is_empty() {
            list = list.child(
                div()
                    .py_4()
                    .text_size(px(theme::font_body()))
                    .text_color(theme::text_faint())
                    .child("Trash is empty."),
            );
        }
        for (index, entry) in entries.iter().enumerate() {
            let entry = entry.clone();
            let label = entry.relative();
            let age = crate::trash::age_label(entry.deleted_at_ms, now);
            list = list.child(
                div()
                    .id(("trash-row", index as u64))
                    .debug_selector(move || format!("trash-row-{index}"))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .rounded(px(6.))
                    .px_2()
                    .py_1p5()
                    .hover(|style| style.bg(theme::raised()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family(SharedString::from(theme::code_font()))
                            .text_size(px(theme::font_mono_meta()))
                            .text_color(theme::text_dim())
                            .child(label),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(theme::font_meta()))
                            .text_color(theme::text_faint())
                            .child(age),
                    )
                    .child(
                        div()
                            .id(("trash-restore", index as u64))
                            .debug_selector(move || format!("trash-restore-{index}"))
                            .a11y_button("Restore")
                            .cursor_pointer()
                            .flex_shrink_0()
                            .rounded(px(6.))
                            .px_2()
                            .py_0p5()
                            .text_size(px(theme::font_control()))
                            .text_color(theme::accent_ink())
                            .hover(|style| style.bg(theme::raised()))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.restore_trash_entry(entry.clone(), window, cx);
                            }))
                            .child("Restore"),
                    ),
            );
        }

        let card = div()
            .id("notes-trash-dialog")
            .debug_selector(|| "notes-trash-dialog".into())
            .role(gpui::Role::Dialog)
            .occlude()
            .w(px(theme::TRASH_WIDTH))
            .rounded(px(8.))
            .border_1()
            .border_color(theme::line_strong())
            .bg(theme::panel())
            .shadow_lg()
            .p_4()
            .flex()
            .flex_col()
            .gap_3()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    this.trash_open = false;
                    cx.notify();
                    cx.stop_propagation();
                }
            }))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(theme::font_screen_title()))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::text())
                            .child("Trash"),
                    )
                    .child(
                        div()
                            .text_size(px(theme::font_meta()))
                            .text_color(theme::text_muted())
                            .child(format!("{count} item(s)")),
                    ),
            )
            .child(
                div()
                    .text_size(px(theme::font_meta()))
                    .text_color(theme::text_muted())
                    .child("Deleted notes stay here for 30 days, then they are purged."),
            )
            .child(list)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_end()
                    .gap_2()
                    .child(
                        div()
                            .id("trash-empty")
                            .debug_selector(|| "trash-empty".into())
                            .a11y_button("Empty Trash")
                            .cursor_pointer()
                            .rounded(px(6.))
                            .h(px(theme::scaled(shell::CONTROL_HEIGHT) - 8.))
                            .px_3()
                            .flex()
                            .items_center()
                            .text_size(px(theme::font_control()))
                            .text_color(theme::DANGER)
                            .hover(|style| style.bg(theme::tint(theme::DANGER)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                crate::trash::empty_trash(&this.store_root);
                                this.notice(cx, NoticeKind::Info, "Trash emptied");
                                cx.notify();
                            }))
                            .child("Empty Trash"),
                    )
                    .child(
                        div()
                            .id("trash-close")
                            .debug_selector(|| "trash-close".into())
                            .a11y_button("Close Trash")
                            .cursor_pointer()
                            .rounded(px(6.))
                            .h(px(theme::scaled(shell::CONTROL_HEIGHT) - 8.))
                            .px_3()
                            .flex()
                            .items_center()
                            .bg(theme::accent())
                            .text_color(theme::on_accent())
                            .text_size(px(theme::font_control()))
                            .font_weight(FontWeight::MEDIUM)
                            .hover(|style| style.opacity(0.9))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.trash_open = false;
                                cx.notify();
                            }))
                            .child("Close"),
                    ),
            );
        let _ = window;
        div()
            .id("notes-trash-scrim")
            .absolute()
            .inset_0()
            .occlude()
            .bg(theme::SCRIM)
            .flex()
            .items_center()
            .justify_center()
            .p_4()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.trash_open = false;
                    cx.notify();
                }),
            )
            .child(card)
            .into_any_element()
    }

    fn render_shortcuts(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let mut rows = div().flex().flex_col().gap_1();
        for (keys, label) in SHORTCUTS {
            rows = rows.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .rounded(px(4.))
                    .px_2()
                    .py_1()
                    .child(
                        div()
                            .w(px(118.))
                            .flex_shrink_0()
                            .font_family(SharedString::from(theme::code_font()))
                            .text_size(px(theme::font_mono_meta()))
                            .text_color(theme::text_dim())
                            .child(*keys),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(theme::font_body()))
                            .text_color(theme::text())
                            .child(*label),
                    ),
            );
        }
        let card = div()
            .id("shortcut-card")
            .debug_selector(|| "shortcut-card".into())
            .role(gpui::Role::Dialog)
            .occlude()
            .w(px(theme::PROMPT_WIDTH + 60.))
            .max_h(px(600.))
            .rounded(px(8.))
            .border_1()
            .border_color(theme::line_strong())
            .bg(theme::panel())
            .shadow_lg()
            .p_4()
            .flex()
            .flex_col()
            .gap_2()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .text_size(px(theme::font_screen_title()))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::text())
                    .child("Keyboard"),
            )
            .child(
                div()
                    .text_size(px(theme::font_meta()))
                    .text_color(theme::text_muted())
                    .child("The editor keeps its own arrow keys while it has focus."),
            )
            .child(
                div()
                    .id("shortcut-list")
                    .max_h(px(470.))
                    .overflow_y_scroll()
                    .child(rows),
            )
            .child(
                div().flex().flex_row().justify_end().child(
                    div()
                        .id("shortcut-close")
                        .debug_selector(|| "shortcut-close".into())
                        .a11y_button("Close keyboard help")
                        .cursor_pointer()
                        .rounded(px(6.))
                        .h(px(theme::scaled(shell::CONTROL_HEIGHT) - 8.))
                        .px_3()
                        .flex()
                        .items_center()
                        .bg(theme::accent())
                        .text_color(theme::on_accent())
                        .text_size(px(theme::font_control()))
                        .font_weight(FontWeight::MEDIUM)
                        .hover(|style| style.opacity(0.9))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.shortcut_open = false;
                            cx.notify();
                        }))
                        .child("Close"),
                ),
            );
        div()
            .id("shortcut-scrim")
            .absolute()
            .inset_0()
            .occlude()
            .bg(theme::SCRIM)
            .flex()
            .items_center()
            .justify_center()
            .p_4()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.shortcut_open = false;
                    cx.notify();
                }),
            )
            .child(card)
            .into_any_element()
    }

    fn render_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        let rows = self.palette_rows(cx);
        let row_count = rows.len();
        let selected = self.palette_selected.min(row_count.saturating_sub(1));
        let mut list = div().flex().flex_col().gap_0p5();
        if rows.is_empty() {
            list = list.child(
                div()
                    .px_2()
                    .py_4()
                    .text_size(px(theme::font_body()))
                    .text_color(theme::text_faint())
                    .child("No notes."),
            );
        }
        for (index, item) in rows.iter().enumerate() {
            let path = item.path.clone();
            list = list.child(
                div()
                    .id(("palette-row", index as u64))
                    .debug_selector(move || format!("palette-row-{index}"))
                    .a11y_button(format!("{} — {}", item.title, item.relative))
                    .cursor_pointer()
                    .rounded(px(6.))
                    .px_2()
                    .py_1p5()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .when(index == selected, |row| row.bg(theme::raised()))
                    .hover(|row| row.bg(theme::raised()))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.palette_selected = index;
                        this.open_note_anywhere(path.clone(), window, cx);
                        this.palette_open = false;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(theme::font_body()))
                            .text_color(theme::text())
                            .child(item.title.clone()),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(theme::font_meta()))
                            .text_color(theme::text_faint())
                            .child(item.relative.clone()),
                    ),
            );
        }
        let card = div()
            .id("palette-card")
            .debug_selector(|| "palette-card".into())
            .role(gpui::Role::Dialog)
            .occlude()
            .w(px(theme::PALETTE_WIDTH))
            .rounded(px(8.))
            .border_1()
            .border_color(theme::line_strong())
            .bg(theme::panel())
            .shadow_lg()
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .on_key_down(cx.listener(
                        move |this, event: &gpui::KeyDownEvent, window, cx| {
                            match event.keystroke.key.as_str() {
                                "escape" => this.close_palette(window, cx),
                                "down" => {
                                    if row_count > 0 {
                                        this.palette_selected =
                                            (this.palette_selected + 1).min(row_count - 1);
                                        cx.notify();
                                    }
                                }
                                "up" => {
                                    this.palette_selected = this.palette_selected.saturating_sub(1);
                                    cx.notify();
                                }
                                "enter" => {
                                    if event.keystroke.modifiers.control {
                                        this.open_palette_selected_in_new_tab(window, cx);
                                    } else {
                                        this.open_palette_selected(window, cx);
                                    }
                                }
                                _ => return,
                            }
                            cx.stop_propagation();
                        },
                    ))
                    .child(warpui_kit_core::field::text(
                        "field-palette",
                        &self.palette_input,
                        window,
                        cx,
                    )),
            )
            .child(
                div()
                    .id("palette-list")
                    .max_h(px(420.))
                    .overflow_y_scroll()
                    .child(list),
            )
            .child(
                div()
                    .text_size(px(theme::font_meta()))
                    .text_color(theme::text_faint())
                    .child("Enter to open · Esc to close · pinned and recent first"),
            );
        div()
            .id("palette-scrim")
            .debug_selector(|| "palette-scrim".into())
            .absolute()
            .inset_0()
            .occlude()
            .bg(theme::SCRIM)
            .flex()
            .items_start()
            .justify_center()
            .pt(px(96.))
            .px_4()
            .pb_4()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.close_palette(window, cx);
                }),
            )
            .child(card)
            .into_any_element()
    }

    fn render_search_results(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let term = self.search.read(cx).value().trim().to_string();
        let hits = self.search_hits.clone();
        let shown = hits.len().min(SEARCH_RESULTS_ROWS);
        let selected = self.search_selected.min(shown.saturating_sub(1));
        let mut list = div()
            .id("search-results-list")
            .debug_selector(|| "search-results-list".into())
            .flex()
            .flex_col()
            .gap_1()
            .max_h(px(470.))
            .overflow_y_scroll();
        if hits.is_empty() {
            list = list.child(
                div()
                    .py_4()
                    .text_size(px(theme::font_body()))
                    .text_color(theme::text_faint())
                    .child("No matches."),
            );
        }
        for (index, hit) in hits.iter().take(shown).enumerate() {
            let path = hit.path.clone();
            let title = hit.title.clone();
            let relative = hit.relative.clone();
            let excerpt = hit.excerpt.clone();
            list = list.child(
                div()
                    .id(("search-result", index as u64))
                    .debug_selector(move || format!("search-result-{index}"))
                    .a11y_button(format!("{} — {}", hit.title, hit.excerpt))
                    .cursor_pointer()
                    .rounded(px(6.))
                    .px_2()
                    .py_1p5()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .when(index == selected, |row| row.bg(theme::raised()))
                    .hover(|row| row.bg(theme::raised()))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.search
                            .update(cx, |field, cx| field.set_value(String::new(), window, cx));
                        this.search_hits.clear();
                        this.search_selected = 0;
                        this.open_note_anywhere(path.clone(), window, cx);
                        this.search_results_open = false;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(theme::font_body()))
                                    .text_color(theme::text())
                                    .child(title),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(theme::font_meta()))
                                    .text_color(theme::text_faint())
                                    .child(relative),
                            ),
                    )
                    .child(highlighted_excerpt(window, &excerpt, &term)),
            );
        }
        let scope_label = if self.search_all_notes {
            "All notes".to_string()
        } else {
            self.notes_scope.clone().unwrap_or_else(|| "Repo".into())
        };
        let card = div()
            .id("search-results-card")
            .debug_selector(|| "search-results-card".into())
            .role(gpui::Role::Dialog)
            .occlude()
            .w(px(760.))
            .rounded(px(8.))
            .border_1()
            .border_color(theme::line_strong())
            .bg(theme::panel())
            .shadow_lg()
            .p_4()
            .flex()
            .flex_col()
            .gap_3()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    this.close_search_results(window, cx);
                    cx.stop_propagation();
                }
            }))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(theme::font_screen_title()))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::text())
                            .child(format!("Search: {term}")),
                    )
                    .child(
                        div()
                            .id("search-results-scope")
                            .debug_selector(|| "search-results-scope".into())
                            .a11y_button("Search scope")
                            .cursor_pointer()
                            .rounded(px(4.))
                            .px_1p5()
                            .py_0p5()
                            .bg(theme::raised())
                            .text_size(px(theme::font_micro()))
                            .text_color(theme::text_muted())
                            .hover(|chip| chip.bg(theme::accent_soft()))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_search_scope(window, cx);
                            }))
                            .child(scope_label),
                    )
                    .child(
                        div()
                            .text_size(px(theme::font_meta()))
                            .text_color(theme::text_faint())
                            .child(format!("{shown} of {}", hits.len())),
                    ),
            )
            .child(list)
            .child(
                div().flex().flex_row().justify_end().child(
                    div()
                        .id("search-results-close")
                        .debug_selector(|| "search-results-close".into())
                        .a11y_button("Close results")
                        .cursor_pointer()
                        .rounded(px(6.))
                        .h(px(theme::scaled(shell::CONTROL_HEIGHT) - 8.))
                        .px_3()
                        .flex()
                        .items_center()
                        .bg(theme::accent())
                        .text_color(theme::on_accent())
                        .text_size(px(theme::font_control()))
                        .font_weight(FontWeight::MEDIUM)
                        .hover(|button| button.opacity(0.9))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.close_search_results(window, cx);
                        }))
                        .child("Close"),
                ),
            );
        div()
            .id("search-results-scrim")
            .debug_selector(|| "search-results-scrim".into())
            .absolute()
            .inset_0()
            .occlude()
            .bg(theme::SCRIM)
            .flex()
            .items_center()
            .justify_center()
            .p_4()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.close_search_results(window, cx);
                }),
            )
            .child(card)
            .into_any_element()
    }
}
impl NotesPage {
    /// The floating window paints its own title bar: the window is created
    /// with appears_transparent, so without this strip there is no title,
    /// drag region, or window controls. The shell's bar: the brand mark and
    /// name, the repo-scope chip when launch pinned one, the list toggle,
    /// then the window controls.
    fn render_titlebar(&mut self, window: &Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        shell::title_bar()
            .px(px(14.))
            // the whole bar drags; the window buttons occlude it from the
            // Windows hit test and stop the press so they stay clickable
            .window_control_area(WindowControlArea::Drag)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    gpui_kit::base::GlobalState::suppress_text_selection(cx);
                    this.titlebar_should_move = true;
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    this.titlebar_should_move = false;
                }),
            )
            .on_mouse_move(
                cx.listener(|this, event: &gpui::MouseMoveEvent, window, _| {
                    if this.titlebar_should_move {
                        this.titlebar_should_move = false;
                        // a missed mouse-up (the OS move loop swallowed it)
                        // must not turn a later hover into a drag
                        if event.pressed_button == Some(MouseButton::Left) {
                            window.start_window_move();
                        }
                    }
                }),
            )
            .on_mouse_down_out(cx.listener(|this, _, _, _| {
                this.titlebar_should_move = false;
            }))
            // the left column: the app's own mark and name, then the family's
            // link and the scope chip. It is ONE flexible column so the search
            // field between it and the window controls lands on the window's
            // centre; the right side's flexible spacer does the balancing.
            .child(
                div()
                    .debug_selector(|| "notes-titlebar-left".into())
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(28.))
                            .rounded(px(6.))
                            .bg(theme::accent_soft())
                            .child(icon(
                                "icons/logo.svg",
                                px(theme::icon_small()),
                                theme::accent_ink(),
                            )),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(theme::font_brand()))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::text())
                            .whitespace_nowrap()
                            .child("Warp Notes"),
                    )
                    // the family: the sibling apps this window can open, right
                    // after its own mark and name, behind one separator
                    .child(
                        shell::app_links().child(
                            shell::app_link(
                                "notes-launch-station",
                                "icons/logo-station.svg",
                                "Open Warp Station",
                            )
                            .on_click(|_, _, _| {
                                // the window outlives this one: reap the child
                                // on a thread instead of leaving a zombie
                                match std::process::Command::new("warp-station").spawn() {
                                    Ok(mut child) => {
                                        std::thread::spawn(move || {
                                            let _ = child.wait();
                                        });
                                    }
                                    Err(err) => {
                                        tracing::warn!("could not launch warp-station: {err}");
                                    }
                                }
                            })
                            .when(SHOW_TOOLTIPS, |d| {
                                d.tooltip(crate::ui::tooltip_text("Open Warp Station"))
                            }),
                        ),
                    )
                    .when_some(self.notes_scope.clone(), |row, repo| {
                        row.child(
                            div()
                                .id("notes-scope-chip")
                                .h(px(24.))
                                .px_2p5()
                                .rounded(px(6.))
                                .bg(theme::raised())
                                .flex()
                                .items_center()
                                .text_size(px(theme::font_meta()))
                                .text_color(theme::text_dim())
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .max_w(px(220.))
                                .child(repo),
                        )
                        // up out of the repo's notes folder to the whole store
                        .child(crate::ui::window_button(
                            "notes-up-dir",
                            "icons/icon-arrow-up.svg",
                            theme::raised().into(),
                            "Up to all repositories' notes",
                            SHOW_TOOLTIPS,
                            cx.listener(|this, _, _, cx| {
                                this.notes_scope = None;
                                this.expanded.clear();
                                // the scope the user reached is the one the
                                // next launch without a --repo pin reopens
                                this.prefs.notes_scope = None;
                                this.remember_expanded();
                                cx.notify();
                            }),
                        ))
                    }),
            )
            // search: the app's one search field, in the title bar over the
            // list it searches. The left column is flexible and the spacer on
            // its right balances it, so the field lands on the window's centre.
            .child({
                let box_slot = self.search_field_box.clone();
                div()
                    .occlude()
                    .flex_shrink_0()
                    .w(px(SEARCH_W))
                    .relative()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    // paint-phase capture of the field's box: the results popup
                    // hangs off the real one, not a guessed centre
                    .child(
                        canvas(
                            |_, _, _| {},
                            move |bounds, _, _, _| {
                                box_slot.set(Some((
                                    bounds.origin.x.as_f32(),
                                    bounds.size.width.as_f32(),
                                )));
                            },
                        )
                        .absolute()
                        .size_full(),
                    )
                    .child(
                        warpui_kit_core::field::boxed(
                            "notes-search-field",
                            &self.search.read(cx).focus_handle(cx),
                            window,
                            warpui_kit_core::field::title_height(),
                        )
                        .child(icon(
                            "icons/icon-search.svg",
                            px(theme::icon_small()),
                            theme::text_muted(),
                        ))
                        .child(warpui_kit_core::field::input(&self.search).cleanable(true))
                        .child(warpui_kit_core::field::kbd("Ctrl K"))
                        // the results hang under the field, so the keyboard
                        // belongs to the search while it has text
                        .on_key_down(cx.listener(
                            |this, event: &gpui::KeyDownEvent, window, cx| {
                                if this.search_results_open {
                                    match event.keystroke.key.as_str() {
                                        "escape" => this.close_search_results(window, cx),
                                        "down" => {
                                            if !this.search_hits.is_empty() {
                                                let last = this
                                                    .search_hits
                                                    .len()
                                                    .min(SEARCH_RESULTS_ROWS)
                                                    .saturating_sub(1);
                                                this.search_selected =
                                                    (this.search_selected + 1).min(last);
                                                cx.notify();
                                            }
                                        }
                                        "up" => {
                                            this.search_selected =
                                                this.search_selected.saturating_sub(1);
                                            cx.notify();
                                        }
                                        "enter" => {
                                            if event.keystroke.modifiers.control {
                                                this.open_search_hit_in_new_tab(
                                                    this.search_selected,
                                                    window,
                                                    cx,
                                                );
                                            } else {
                                                this.open_search_hit(
                                                    this.search_selected,
                                                    window,
                                                    cx,
                                                );
                                            }
                                        }
                                        _ => return,
                                    }
                                    cx.stop_propagation();
                                } else if event.keystroke.key == "escape" {
                                    this.search.update(cx, |field, cx| {
                                        field.set_value(String::new(), window, cx)
                                    });
                                    this.search_hits.clear();
                                    this.search_selected = 0;
                                    cx.notify();
                                    cx.stop_propagation();
                                } else if event.keystroke.key == "down" {
                                    if !this.search_hits.is_empty() {
                                        let last = this
                                            .search_hits
                                            .len()
                                            .min(SEARCH_POPUP_ROWS)
                                            .saturating_sub(1);
                                        this.search_selected = (this.search_selected + 1).min(last);
                                        cx.notify();
                                    }
                                    cx.stop_propagation();
                                } else if event.keystroke.key == "up" {
                                    this.search_selected = this.search_selected.saturating_sub(1);
                                    cx.notify();
                                    cx.stop_propagation();
                                } else if event.keystroke.key == "enter" {
                                    if event.keystroke.modifiers.control {
                                        this.open_search_hit_in_new_tab(
                                            this.search_selected,
                                            window,
                                            cx,
                                        );
                                    } else {
                                        this.open_search_hit(this.search_selected, window, cx);
                                    }
                                    cx.stop_propagation();
                                }
                            },
                        )),
                    )
            })
            // the right column: the list toggle, then the window controls. It
            // is flexible and right-aligned so it balances the left column and
            // the search field between them sits on the window's centre.
            .child(
                div()
                    .debug_selector(|| "notes-titlebar-right".into())
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap_1()
                    .when(self.section == Section::Notes, |row| {
                        row.child(crate::ui::window_button(
                            "notes-back",
                            "icons/icon-arrow-left.svg",
                            theme::raised().into(),
                            "Back",
                            SHOW_TOOLTIPS,
                            cx.listener(|this, _, window, cx| this.go_history(-1, window, cx)),
                        ))
                        .child(crate::ui::window_button(
                            "notes-forward",
                            "icons/icon-arrow-right.svg",
                            theme::raised().into(),
                            "Forward",
                            SHOW_TOOLTIPS,
                            cx.listener(|this, _, window, cx| this.go_history(1, window, cx)),
                        ))
                        .child(crate::ui::window_button(
                            "notes-context-toggle",
                            "icons/icon-panel-right.svg",
                            theme::raised().into(),
                            "Outline and links",
                            SHOW_TOOLTIPS,
                            cx.listener(|this, _, _, cx| this.toggle_context_panel(cx)),
                        ))
                        .child(crate::ui::window_button(
                            "notes-refresh",
                            "icons/icon-refresh.svg",
                            theme::raised().into(),
                            "Refresh notes from disk",
                            SHOW_TOOLTIPS,
                            cx.listener(|this, _, window, cx| this.refresh_from_disk(window, cx)),
                        ))
                        .child(crate::ui::window_button(
                            "notes-list-toggle",
                            "icons/icon-panel-left.svg",
                            theme::raised().into(),
                            if self.list_visible {
                                "Hide list"
                            } else {
                                "Show list"
                            },
                            SHOW_TOOLTIPS,
                            cx.listener(|this, _, _, cx| this.toggle_list(cx)),
                        ))
                    })
                    .child(div().w(px(1.)).h(px(20.)).mx_1().bg(theme::hairline()))
                    .child(crate::ui::window_button(
                        "notes-minimize",
                        "icons/icon-window-minimize.svg",
                        theme::raised().into(),
                        "Minimize",
                        SHOW_TOOLTIPS,
                        |_, window, _| window.minimize_window(),
                    ))
                    .child(crate::ui::window_button_glyph(
                        "notes-zoom",
                        theme::raised().into(),
                        if window.is_maximized() {
                            "Restore"
                        } else {
                            "Maximize"
                        },
                        SHOW_TOOLTIPS,
                        cx.listener(|_, _, window, cx| {
                            window.zoom_window();
                            // the compositor toggles asynchronously — nudge a
                            // re-render so the maximize/restore glyph keeps up
                            cx.notify();
                        }),
                        shell::zoom_glyph(window.is_maximized(), theme::text_muted()),
                    ))
                    .child(crate::ui::window_button(
                        "notes-close",
                        "icons/icon-window-close.svg",
                        theme::tint(theme::DANGER),
                        "Close",
                        SHOW_TOOLTIPS,
                        // the buffers land on disk before the window goes —
                        // closing is a save point, not a data-loss risk
                        cx.listener(|this, _, window, cx| {
                            this.flush_saves(cx);
                            window.remove_window();
                        }),
                    )),
            )
            .into_any_element()
    }

    /// The find bar: the term, the match count, Enter/Shift+Enter to walk the
    /// matches, Escape or × to close. It keeps the keyboard, so the editor's
    /// selection marks the match without stealing the typing.
    fn render_find_bar(&mut self, window: &Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        let count = self.find_hits.len();
        let counter = if count == 0 {
            "No matches".to_string()
        } else {
            format!("{}/{}", self.find_index + 1, count)
        };
        div()
            .id("notes-find-bar")
            .debug_selector(|| "notes-find-bar".into())
            .flex_shrink_0()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .px_4()
            .pb_2()
            .border_b_1()
            .border_color(theme::border_light())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                        match event.keystroke.key.as_str() {
                            "enter" => {
                                let delta = if event.keystroke.modifiers.shift {
                                    -1
                                } else {
                                    1
                                };
                                this.step_find(delta, cx);
                                cx.stop_propagation();
                            }
                            "escape" => {
                                this.close_find(window, cx);
                                cx.stop_propagation();
                            }
                            _ => {}
                        }
                    }))
                    .child(warpui_kit_core::field::text(
                        "field-find",
                        &self.find,
                        window,
                        cx,
                    )),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(theme::font_meta()))
                    .text_color(theme::text_faint())
                    .child(counter),
            )
            .child(
                div()
                    .id("notes-find-close")
                    .a11y_button("Close find")
                    .cursor_pointer()
                    .rounded(px(6.))
                    .size(px(theme::scaled(26.)))
                    .flex()
                    .items_center()
                    .justify_center()
                    .hover(|s| s.bg(theme::raised()))
                    .on_click(cx.listener(|this, _, window, cx| this.close_find(window, cx)))
                    .child(icon(
                        "icons/icon-close.svg",
                        px(theme::icon_small() - 2.),
                        theme::text_muted(),
                    )),
            )
            .into_any_element()
    }

    /// The changed-on-disk bar: autosave is paused until the user picks which
    /// copy wins. Reload drops the buffer; Overwrite writes it deliberately.
    fn render_conflict_bar(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let button = |id: &'static str, label: &'static str, color: gpui::Rgba| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .a11y_button(label)
                .cursor_pointer()
                .h(px(theme::scaled(shell::CONTROL_HEIGHT) - 8.))
                .px_3()
                .rounded(px(6.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .bg(theme::raised())
                .text_size(px(theme::font_control()))
                .text_color(color)
                .hover(|s| s.bg(theme::accent_soft()))
                .child(label)
        };
        div()
            .id("note-conflict")
            .debug_selector(|| "note-conflict".into())
            .w_full()
            .flex()
            .flex_col()
            .gap_2()
            .px_4()
            .py_2()
            .bg(theme::tint(theme::DANGER))
            .border_b_1()
            .border_color(theme::tint(theme::DANGER))
            .child(
                div()
                    .w_full()
                    .text_size(px(theme::font_meta()))
                    .text_color(theme::DANGER)
                    .child("This note changed on disk. Autosave is paused."),
            )
            .child(
                div()
                    .w_full()
                    .flex()
                    .flex_row()
                    .justify_end()
                    .gap_2()
                    .child(button("conflict-reload", "Reload", theme::text()).on_click(
                        cx.listener(|this, _, window, cx| this.reload_conflicted(window, cx)),
                    ))
                    .child(
                        button("conflict-overwrite", "Overwrite", theme::DANGER)
                            .on_click(cx.listener(|this, _, _, cx| this.overwrite_conflicted(cx))),
                    ),
            )
            .into_any_element()
    }

    /// The status strip: "N words · N chars · N lines", the unsaved hint,
    /// and "Ln x, Col y".
    ///
    /// The counts scan the whole document, and this bar re-renders on every
    /// caret move and every editor notify — so they are cached against the
    /// editor's revision, which moves only when the text does. Without that,
    /// a long note paid a full pass over its bytes per keystroke.
    fn render_status_bar(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        // `stats_revision` is None exactly while no note is open, so opening
        // and closing both invalidate it without a separate flag
        let (line, col) = if self.open_note.is_some() {
            let byte = self.editor.read(cx).selected_range().start;
            self.editor.read(cx).line_col(byte)
        } else {
            (1, 1)
        };
        let revision = self
            .open_note
            .is_some()
            .then(|| self.editor.read(cx).revision());
        if self.stats_revision != revision {
            self.stats_revision = revision;
            self.text_stats = match revision {
                Some(_) => {
                    let text = self.editor.read(cx).value();
                    TextStats {
                        words: text.split_whitespace().count(),
                        chars: text.chars().count(),
                        lines: text.lines().count(),
                    }
                }
                None => TextStats::default(),
            };
        }
        let TextStats {
            words,
            chars,
            lines,
        } = self.text_stats;
        let dirty = self.open_note.is_some() && self.is_dirty(cx);
        let mut strip = div()
            .id("notes-status")
            .flex()
            .flex_row()
            .items_center()
            .gap_3()
            .py_1p5()
            .border_t_1()
            .border_color(theme::border_light())
            .text_size(px(theme::font_meta()))
            .text_color(theme::text_faint())
            .child(format!("{words} words · {chars} chars · {lines} lines"))
            .child(div().flex_1());
        if self.conflict {
            strip = strip.child(div().text_color(theme::DANGER).child("Changed on disk"));
        } else if dirty {
            // saving is automatic: the mark only says the write is still a
            // second of typing silence away
            strip = strip.child(div().text_color(theme::CHANGES).child("Unsaved"));
        }
        strip
            .child(format!("Ln {line}, Col {col}"))
            .into_any_element()
    }

    /// The list pane: the tree tools over the tree, on the shell's list
    /// surface. Search does not replace the tree — its results are the title
    /// bar's popup — so the list keeps its scroll and selection while typing.
    fn render_tree(&mut self, window: &Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        let can_create = self.notes_scope.is_some() || !self.tree.is_empty();
        let (notes_count, pinned_count) = self.list_counts();
        let tree_focus = self.tree_focus.clone();
        let tree = shell::list_pane(self.prefs.list_width)
            .id("notes-sidebar")
            .debug_selector(|| "notes-sidebar".into())
            .track_focus(&tree_focus)
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                this.handle_tree_key(event, window, cx);
            }))
            // the head: the live counts, the list's tools, then the scope
            .child(self.render_list_head(notes_count, pinned_count, can_create, cx))
            // inline "new note" form, stacked for the narrow column
            .when(self.create_open, |card| {
                card.child(
                    div()
                        .id("note-create-form")
                        .debug_selector(|| "note-create-form".into())
                        .flex()
                        .flex_col()
                        .gap_1()
                        .pt_2()
                        .px_3()
                        .child(warpui_kit_core::field::text(
                            "field-create_name",
                            &self.create_name,
                            window,
                            cx,
                        ))
                        .child(warpui_kit_core::field::text(
                            "field-create_folder",
                            &self.create_folder,
                            window,
                            cx,
                        ))
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap_1()
                                .py_1()
                                .child(
                                    div()
                                        .id("note-create-confirm")
                                        .a11y_button("Create note")
                                        .cursor_pointer()
                                        .rounded(px(6.))
                                        .h(px(theme::scaled(shell::CONTROL_HEIGHT) - 8.))
                                        .px_3()
                                        .flex()
                                        .items_center()
                                        .text_size(px(theme::font_control()))
                                        .font_weight(FontWeight::MEDIUM)
                                        .bg(theme::accent())
                                        .text_color(theme::on_accent())
                                        .hover(|s| s.opacity(0.9))
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.create_note(window, cx)
                                        }))
                                        .child("Create"),
                                )
                                .child(
                                    div()
                                        .id("note-create-cancel")
                                        .a11y_button("Cancel new note")
                                        .cursor_pointer()
                                        .rounded(px(6.))
                                        .h(px(theme::scaled(shell::CONTROL_HEIGHT) - 8.))
                                        .px_2p5()
                                        .flex()
                                        .items_center()
                                        .text_size(px(theme::font_control()))
                                        .text_color(theme::text_muted())
                                        .hover(|s| s.bg(theme::raised()))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.create_open = false;
                                            cx.notify();
                                        }))
                                        .child("Cancel"),
                                ),
                        ),
                )
            });

        let (list, key) = match self.list_scope {
            ListScope::All => (self.render_tree_list(cx), "notes-list-all"),
            ListScope::Pinned => (self.render_pinned_list(cx), "notes-list-pinned"),
        };
        let list = div()
            .flex_1()
            .min_h_0()
            .w_full()
            .flex()
            .flex_col()
            .child(list);
        tree.child(motion::enter(key, list)).into_any_element()
    }

    /// The list head: the live counts over the list's tools, then the scope —
    /// Warp Station's list-head shape.
    fn render_list_head(
        &mut self,
        notes: usize,
        pinned: usize,
        can_create: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let page = cx.entity();
        let choices = vec![
            warpui_kit_core::settings::Choice::new(
                "all",
                format!("All {notes}"),
                self.list_scope == ListScope::All,
            ),
            warpui_kit_core::settings::Choice::new(
                "pinned",
                format!("Pinned {pinned}"),
                self.list_scope == ListScope::Pinned,
            ),
        ];
        div()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_2p5()
            .px_4()
            .pt_3p5()
            .pb_2()
            .border_b_1()
            .border_color(theme::hairline())
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .min_h(px(theme::scaled(30.)))
                    .child(
                        div()
                            .id("notes-list-summary")
                            .debug_selector(|| "notes-list-summary".into())
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .flex()
                            .flex_row()
                            .items_center()
                            .text_size(px(theme::font_sub()))
                            .text_color(theme::text_muted())
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme::text())
                                    .child(format!("{notes} {}", note_word(notes))),
                            )
                            .child(format!(" · {pinned} pinned")),
                    )
                    .when(can_create, |row| {
                        row.child(
                            div()
                                .id("note-new")
                                .a11y_toggle("New note", self.create_open)
                                .cursor_pointer()
                                .rounded(px(6.))
                                .size(px(theme::scaled(26.)))
                                .flex()
                                .items_center()
                                .justify_center()
                                .hover(|s| s.bg(theme::raised()))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.create_open = !this.create_open;
                                    cx.notify();
                                }))
                                .child(icon(
                                    "icons/icon-plus.svg",
                                    px(theme::icon_small() - 1.),
                                    theme::text_muted(),
                                )),
                        )
                    })
                    .child(
                        div()
                            .id("notes-collapse-all")
                            .debug_selector(|| "notes-collapse-all".into())
                            .a11y_button("Collapse all folders")
                            .cursor_pointer()
                            .rounded(px(6.))
                            .size(px(theme::scaled(26.)))
                            .flex()
                            .items_center()
                            .justify_center()
                            .hover(|s| s.bg(theme::raised()))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.expanded.clear();
                                cx.notify();
                            }))
                            .child(icon(
                                "icons/icon-list-bullet.svg",
                                px(theme::icon_small() - 1.),
                                theme::text_muted(),
                            )),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .child(warpui_kit_core::settings::segmented(
                        "notes-list-scope",
                        choices,
                        move |key, _, cx| {
                            let scope = if key.as_ref() == "pinned" {
                                ListScope::Pinned
                            } else {
                                ListScope::All
                            };
                            page.update(cx, |this, cx| this.set_list_scope(scope, cx));
                        },
                    )),
            )
            .into_any_element()
    }

    /// The pinned scope: the same rows as the tree, flat, newest pin first.
    fn render_pinned_list(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let pinned = self.pinned_paths();
        if pinned.is_empty() {
            return div()
                .id("notes-pinned-empty")
                .debug_selector(|| "notes-pinned-empty".into())
                .flex_1()
                .min_h_0()
                .w_full()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(theme::font_meta()))
                .text_color(theme::text_muted())
                .child("No pinned notes yet")
                .into_any_element();
        }
        let editor_text = self.editor.read(cx).value();
        let mut list = div()
            .id("notes-pinned-list")
            .debug_selector(|| "notes-pinned-list".into())
            .flex_1()
            .min_h_0()
            .w_full()
            .overflow_y_scroll()
            .flex()
            .flex_col();
        let mut shown = 0usize;
        for path in &pinned {
            if let Some(item) = self.tree_item_for(path) {
                list = list.child(self.render_tree_row(&item, 0, shown, editor_text.as_ref(), cx));
                shown += 1;
            }
        }
        list.into_any_element()
    }

    /// What the head prints: "7 notes · 2 pinned".
    fn summary_text(&self) -> String {
        let (notes, pinned) = self.list_counts();
        format!("{notes} {} · {pinned} pinned", note_word(notes))
    }

    /// The list's live counts: every note in the tree, and the pins in scope.
    fn list_counts(&self) -> (usize, usize) {
        let mut notes = Vec::new();
        collect_note_paths(&self.tree, &mut notes);
        (notes.len(), self.pinned_paths().len())
    }

    /// The pinned notes in the current tree scope, in pin order.
    fn pinned_paths(&self) -> Vec<PathBuf> {
        let scope = self.notes_root();
        self.prefs
            .pinned
            .iter()
            .map(|relative| self.store_root.join(relative))
            .filter(|path| path.is_file())
            .filter(|path| scope.as_ref().is_none_or(|scope| path.starts_with(scope)))
            .collect()
    }

    /// Switch the list's scope; the tree cursor belonged to the old list.
    fn set_list_scope(&mut self, scope: ListScope, cx: &mut Context<Self>) {
        if self.list_scope == scope {
            return;
        }
        self.list_scope = scope;
        self.tree_cursor = None;
        cx.notify();
    }

    /// The search results, as the design's popup under the title bar field.
    /// Absent while the field is empty; the popup is the only thing a search
    /// draws, so the tree underneath keeps its scroll and selection.
    fn render_search_popup(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        // The field is not at the window's centre — the brand, the app links
        // and the scope chip all sit to its left — so the popup hangs off the
        // field's own painted box rather than a guessed centre. Before the
        // first paint there is no box to hang it from.
        let _ = window;
        let (left, width) = self.search_field_box.get()?;
        let term = self.search.read(cx).value().trim().to_string();
        if term.is_empty() {
            return None;
        }
        let hits = &self.search_hits;
        let scope_label = if self.search_all_notes {
            "All notes".to_string()
        } else {
            self.notes_scope.clone().unwrap_or_else(|| "Repo".into())
        };
        let mut rows = div().flex().flex_col().gap_0p5().child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .px_2()
                .py_1()
                .child(
                    div()
                        .id("search-scope-chip")
                        .debug_selector(|| "search-scope-chip".into())
                        .a11y_button("Search scope")
                        .cursor_pointer()
                        .rounded(px(4.))
                        .px_1p5()
                        .py_0p5()
                        .bg(theme::raised())
                        .text_size(px(theme::font_micro()))
                        .text_color(theme::text_muted())
                        .hover(|chip| chip.bg(theme::accent_soft()))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.toggle_search_scope(window, cx);
                        }))
                        .child(scope_label),
                )
                .child(
                    div()
                        .text_size(px(theme::font_meta()))
                        .text_color(theme::text_faint())
                        .child(format!("{} hit(s)", hits.len())),
                ),
        );
        if hits.is_empty() {
            rows = rows.child(
                div()
                    .px_2()
                    .py_2()
                    .text_size(px(theme::font_body()))
                    .text_color(theme::text_faint())
                    .child("No matches."),
            );
        }
        // a popup is a glance, not a list: the design caps it and the tree
        // stays the way to reach everything
        for (index, hit) in hits.iter().take(SEARCH_POPUP_ROWS).enumerate() {
            let path = hit.path.clone();
            let selected = index == self.search_selected;
            rows = rows.child(
                div()
                    .id(("notes-search-hit", index as u64))
                    .debug_selector({
                        let title = hit.title.clone();
                        move || format!("notes-search-hit-{title}")
                    })
                    .a11y_button(format!("{} — {}", hit.title, hit.excerpt))
                    .cursor_pointer()
                    .rounded(px(6.))
                    .px_2()
                    .py_1p5()
                    .flex()
                    .flex_col()
                    .when(selected, |row| row.bg(theme::raised()))
                    .hover(|s| s.bg(theme::raised()))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.search
                            .update(cx, |field, cx| field.set_value(String::new(), window, cx));
                        this.search_hits.clear();
                        this.search_results_open = false;
                        this.open_note_anywhere(path.clone(), window, cx);
                    }))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(theme::font_body()))
                            .text_color(theme::text_dim())
                            .child(hit.title.clone()),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(theme::font_meta()))
                            .text_color(theme::text_faint())
                            .child(hit.excerpt.clone()),
                    ),
            );
        }
        if hits.len() > SEARCH_POPUP_ROWS {
            rows = rows.child(
                div()
                    .id("notes-search-all")
                    .debug_selector(|| "notes-search-all".into())
                    .a11y_button("See all results")
                    .cursor_pointer()
                    .rounded(px(4.))
                    .px_2()
                    .py_1()
                    .text_size(px(theme::font_meta()))
                    .text_color(theme::accent_ink())
                    .hover(|row| row.bg(theme::raised()))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_search_results(window, cx);
                    }))
                    .child(format!("See all {} results", hits.len())),
            );
        }
        Some(
            div()
                .id("notes-search-popup")
                .debug_selector(|| "notes-search-popup".into())
                .absolute()
                .top(px(theme::scaled(shell::TITLE_BAR_HEIGHT)))
                // under the field it belongs to, from its painted box
                .left(px(left))
                .w(px(width))
                .occlude()
                .rounded_lg()
                .border_1()
                .border_color(theme::hairline())
                .bg(theme::overlay())
                .shadow_lg()
                .p_1()
                .child(rows)
                .into_any_element(),
        )
    }

    /// The tree flattened to what is on screen: one entry per visible row,
    /// with its depth. A note store can hold thousands of notes, and the old
    /// recursion built an element for every one of them on every frame; the
    /// flattened form feeds `uniform_list`, which lays out only the rows in
    /// the viewport.
    fn visible_tree_rows(
        &self,
        items: &[Rc<notes_core::NotesTreeItem>],
        depth: usize,
        out: &mut Vec<(Rc<notes_core::NotesTreeItem>, usize)>,
    ) {
        for item in items {
            let expanded = self.expanded.contains(&item.full_path);
            out.push((item.clone(), depth));
            if item.is_folder && expanded {
                self.visible_tree_rows(&item.children, depth + 1, out);
            }
        }
    }

    fn visible_row_paths(&self) -> Vec<(PathBuf, bool)> {
        let mut rows = Vec::new();
        self.visible_tree_rows(&self.tree, 0, &mut rows);
        rows.into_iter()
            .map(|(item, _)| (item.full_path.clone(), item.is_folder))
            .collect()
    }

    fn tree_item_for(&self, path: &Path) -> Option<Rc<notes_core::NotesTreeItem>> {
        find_tree_item(&self.tree, path)
    }

    fn move_tree_cursor(&mut self, delta: isize, cx: &mut Context<Self>) {
        let rows = self.visible_row_paths();
        if rows.is_empty() {
            return;
        }
        let Some(current) = self
            .tree_cursor
            .as_ref()
            .and_then(|cursor| rows.iter().position(|(path, _)| path == cursor))
        else {
            let index = if delta < 0 { rows.len() - 1 } else { 0 };
            self.tree_cursor = Some(rows[index].0.clone());
            cx.notify();
            return;
        };
        let next = (current as isize)
            .saturating_add(delta)
            .clamp(0, rows.len() as isize - 1) as usize;
        self.tree_cursor = Some(rows[next].0.clone());
        cx.notify();
    }

    fn jump_tree_cursor(&mut self, last: bool, cx: &mut Context<Self>) {
        let rows = self.visible_row_paths();
        let Some((path, _)) = rows.get(if last {
            rows.len().saturating_sub(1)
        } else {
            0
        }) else {
            return;
        };
        self.tree_cursor = Some(path.clone());
        cx.notify();
    }

    fn collapse_or_parent(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.tree_cursor.clone() else {
            return;
        };
        let is_folder = self.tree_item_for(&path).is_some_and(|item| item.is_folder);
        if is_folder && self.expanded.contains(&path) {
            self.expanded.remove(&path);
            self.remember_expanded();
        } else if let Some(parent) = path.parent()
            && self
                .visible_row_paths()
                .iter()
                .any(|(candidate, _)| candidate == parent)
        {
            self.tree_cursor = Some(parent.to_path_buf());
        }
        cx.notify();
    }

    fn expand_or_child(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.tree_cursor.clone() else {
            return;
        };
        let Some(item) = self.tree_item_for(&path) else {
            return;
        };
        if !item.is_folder {
            return;
        }
        if !self.expanded.contains(&path) {
            self.expanded.insert(path);
            self.remember_expanded();
        } else if let Some(child) = item.children.first() {
            self.tree_cursor = Some(child.full_path.clone());
        }
        cx.notify();
    }

    fn activate_tree_cursor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.tree_cursor.clone() else {
            return;
        };
        let is_folder = self.tree_item_for(&path).is_some_and(|item| item.is_folder);
        self.open_tree_entry(path, is_folder, window, cx);
    }

    fn handle_tree_key(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();
        if event.keystroke.modifiers.alt && matches!(key, "left" | "right") {
            let delta = if key == "left" { -1 } else { 1 };
            self.go_history(delta, window, cx);
            cx.stop_propagation();
            return;
        }
        // the pinned scope is a flat list with no cursor: the tree's keys would
        // move a selection the user cannot see
        if self.list_scope == ListScope::Pinned {
            return;
        }
        if event.keystroke.modifiers.control && key == "d" {
            if let Some(path) = self.tree_cursor.clone() {
                self.duplicate_path(path, window, cx);
            }
            cx.stop_propagation();
            return;
        }
        if event.keystroke.modifiers.shift && key == "f10" {
            if let Some(path) = self.tree_cursor.clone() {
                let is_folder = self.tree_item_for(&path).is_some_and(|item| item.is_folder);
                self.open_row_menu(path, is_folder, window.mouse_position(), cx);
            }
            cx.stop_propagation();
            return;
        }
        match key {
            "down" => self.move_tree_cursor(1, cx),
            "up" => self.move_tree_cursor(-1, cx),
            "home" => self.jump_tree_cursor(false, cx),
            "end" => self.jump_tree_cursor(true, cx),
            "left" => self.collapse_or_parent(cx),
            "right" => self.expand_or_child(cx),
            "enter" => self.activate_tree_cursor(window, cx),
            "f2" => {
                if let Some(path) = self.tree_cursor.clone() {
                    self.start_prompt(PromptKind::Rename, path, window, cx);
                }
            }
            "delete" | "backspace" => {
                if let Some(path) = self.tree_cursor.clone() {
                    self.trash_path(path, window, cx);
                }
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    /// One row of the file tree: 34 px, the selected one lifted with the
    /// accent down its left edge, and a `changes` dot on notes with unsaved
    /// text.
    fn render_tree_row(
        &mut self,
        item: &notes_core::NotesTreeItem,
        depth: usize,
        index: usize,
        editor_text: &str,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        if item.is_folder {
            self.render_folder_band(item, depth, index, cx)
        } else {
            self.render_note_row(item, depth, index, editor_text, cx)
        }
    }

    /// A folder's band: chevron, glyph, quiet name, then how many notes it
    /// holds — the tree's version of station's group band.
    fn render_folder_band(
        &mut self,
        item: &notes_core::NotesTreeItem,
        depth: usize,
        index: usize,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let expanded = self.expanded.contains(&item.full_path);
        let cursor = self.tree_cursor.as_ref() == Some(&item.full_path);
        let id = gpui::ElementId::NamedInteger(
            SharedString::from(format!("note-item-{}-{}", depth, item.name)),
            index as u64,
        );
        let selector_name = item.name.clone();
        let actions = self.row_actions(item, index, cx);
        div()
            .id(id)
            .debug_selector(move || format!("note-item-{}", selector_name))
            .a11y_tree_item(item.name.clone(), Some(expanded))
            .cursor_pointer()
            .flex()
            .items_center()
            .gap_1p5()
            .w_full()
            .h(px(theme::scaled(TREE_ROW_HEIGHT)))
            .pl(px(10. + depth as f32 * 14.))
            .pr_3()
            .when(cursor, |d| d.bg(theme::raised()))
            .hover(|s| s.bg(theme::panel()))
            .on_mouse_down(MouseButton::Right, {
                let path = item.full_path.clone();
                let is_folder = item.is_folder;
                cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                    this.open_row_menu(path.clone(), is_folder, event.position, cx);
                    cx.stop_propagation();
                })
            })
            .on_click({
                let item = item.clone();
                cx.listener(move |this, _, window, cx| {
                    this.tree_cursor = Some(item.full_path.clone());
                    window.focus(&this.tree_focus, cx);
                    if this.expanded.contains(&item.full_path) {
                        this.expanded.remove(&item.full_path);
                    } else {
                        this.expanded.insert(item.full_path.clone());
                    }
                    this.remember_expanded();
                    cx.notify();
                })
            })
            .child(icon(
                if expanded {
                    "icons/icon-chevron-down.svg"
                } else {
                    "icons/icon-chevron-right.svg"
                },
                px(theme::icon_tiny()),
                theme::text_faint(),
            ))
            .child(icon(
                "icons/icon-folder.svg",
                px(theme::icon_tiny()),
                theme::text_muted(),
            ))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(theme::font_sub()))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::text_dim())
                    .child(item.name.clone()),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(theme::font_mono_meta()))
                    .text_color(theme::text_muted())
                    .child(note_count(item).to_string()),
            )
            .child(actions)
            .into_any_element()
    }

    /// A note's row: the name at name size, its age on the right, the pin when
    /// it has one, and the row's verb — a full-width band.
    fn render_note_row(
        &mut self,
        item: &notes_core::NotesTreeItem,
        depth: usize,
        index: usize,
        editor_text: &str,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let selected = self.open_note.as_ref() == Some(&item.full_path);
        let cursor = self.tree_cursor.as_ref() == Some(&item.full_path);
        let pinned = self.is_pinned(&item.full_path);
        let unsaved = self.dirty.contains_key(&item.full_path)
            || (selected && editor_text != self.saved_text.as_str());
        let age = age_label(item.modified);
        let id = gpui::ElementId::NamedInteger(
            SharedString::from(format!("note-item-{}-{}", depth, item.name)),
            index as u64,
        );
        let selector_name = item.name.clone();
        let pin_selector = format!("note-pin-{}", item.name);
        let pin_path = item.full_path.clone();
        let actions = self.row_actions(item, index, cx);
        div()
            .id(id)
            .debug_selector(move || format!("note-item-{}", selector_name))
            .a11y_tree_item(item.name.clone(), None)
            .relative()
            .cursor_pointer()
            .flex()
            .items_center()
            .gap_1p5()
            .w_full()
            .h(px(theme::scaled(TREE_ROW_HEIGHT)))
            .pl(px(10. + depth as f32 * 14.))
            .pr_3()
            .when(selected, |d| {
                d.bg(theme::raised()).child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left_0()
                        .w(px(3.))
                        .bg(theme::accent()),
                )
            })
            .when(!selected, |d| d.hover(|s| s.bg(theme::panel())))
            .when(cursor && !selected, |d| d.bg(theme::raised()))
            .on_mouse_down(MouseButton::Right, {
                let path = item.full_path.clone();
                let is_folder = item.is_folder;
                cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                    this.open_row_menu(path.clone(), is_folder, event.position, cx);
                    cx.stop_propagation();
                })
            })
            .on_click({
                let item = item.clone();
                cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
                    this.tree_cursor = Some(item.full_path.clone());
                    window.focus(&this.tree_focus, cx);
                    if event.click_count() > 1 {
                        // a second click keeps the note, as VS Code's
                        // double click in the explorer does
                        this.open_note_new_tab(item.full_path.clone(), window, cx);
                    } else {
                        this.open_note_preview(item.full_path.clone(), window, cx);
                    }
                    cx.notify();
                })
            })
            .on_mouse_down(MouseButton::Middle, {
                let path = item.full_path.clone();
                cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.tree_cursor = Some(path.clone());
                    this.open_note_new_tab(path.clone(), window, cx);
                })
            })
            .child(icon(
                "icons/icon-note.svg",
                px(theme::icon_tiny()),
                theme::text_muted(),
            ))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(theme::font_name()))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::text())
                    .child(item.name.clone()),
            )
            .when(unsaved, |row| {
                row.child(
                    div()
                        .flex_shrink_0()
                        .size(px(6.))
                        .rounded_full()
                        .bg(theme::CHANGES),
                )
            })
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(theme::font_mono_meta()))
                    .text_color(theme::text_muted())
                    .child(age),
            )
            .when(pinned, |row| {
                row.child(
                    div()
                        .id(("note-pin", index as u64))
                        .debug_selector(move || pin_selector.clone())
                        .a11y_button("Unpin note")
                        .cursor_pointer()
                        .flex_shrink_0()
                        .rounded(px(4.))
                        .p_0p5()
                        .hover(|s| s.bg(theme::panel()))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.toggle_pin(&pin_path, cx);
                        }))
                        .child(icon(
                            "icons/icon-star-filled.svg",
                            px(theme::icon_tiny()),
                            theme::accent_ink(),
                        )),
                )
            })
            .child(actions)
            .into_any_element()
    }

    /// The row's trailing verb: the row menu, quiet until hovered.
    fn row_actions(
        &self,
        item: &notes_core::NotesTreeItem,
        index: usize,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let action_selector = format!("note-actions-{}", item.name);
        div()
            .id(("note-actions", index as u64))
            .debug_selector(move || action_selector.clone())
            .a11y_button("Note actions")
            .cursor_pointer()
            .flex_shrink_0()
            .ml_1()
            .rounded(px(6.))
            .p_0p5()
            .opacity(0.55)
            .hover(|style| style.bg(theme::raised()).opacity(1.))
            .on_mouse_down(MouseButton::Left, {
                let path = item.full_path.clone();
                let is_folder = item.is_folder;
                cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                    this.open_row_menu(path.clone(), is_folder, event.position, cx);
                    cx.stop_propagation();
                })
            })
            .child(icon(
                "icons/icon-more.svg",
                px(theme::icon_small() - 2.),
                theme::text_faint(),
            ))
            .into_any_element()
    }

    /// The virtualized tree: `uniform_list` renders only the rows the viewport
    /// shows, so a large store costs the same as a small one.
    fn render_tree_list(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let mut rows = Vec::new();
        let tree = self.tree.clone();
        self.visible_tree_rows(&tree, 0, &mut rows);
        // the editor's own handle: comparing a row's unsaved dot needs the
        // text, and a clone here is a refcount bump, not a copy
        let editor_text = self.editor.read(cx).value();
        let count = rows.len();
        uniform_list(
            SharedString::from("notes-tree-rows"),
            count,
            cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                range
                    .map(|index| {
                        let (item, depth) = &rows[index];
                        this.render_tree_row(item, *depth, index, &editor_text, cx)
                    })
                    .collect()
            }),
        )
        .track_scroll(&self.tree_scroll)
        // the tree is the inner scrollable of the list pane: when a wheel
        // event moves it, it is consumed here, so nothing outside — today or
        // after a later restack — reacts to the same event
        .on_scroll(cx.listener(|this, event: &ScrollEvent, window, cx| {
            let state = this.tree_scroll.0.borrow();
            let (offset, max) = (state.base_handle.offset(), state.base_handle.max_offset());
            if wheel_reaches_content(offset, max, event.delta.pixel_delta(window.line_height()).y) {
                cx.stop_propagation();
            }
        }))
        .flex_1()
        .min_h_0()
        .w_full()
        .debug_selector(|| "notes-tree-rows".into())
        .into_any_element()
    }

    /// The editor's head: the file name in mono, the unsaved mark, Save, and
    /// Move to Trash, over the format bar.
    /// The editor's head: the note's identity, its facts, then the tools —
    /// Warp Station's detail head.
    fn render_editor_head(&mut self, dirty: bool, cx: &mut Context<Self>) -> gpui::AnyElement {
        // formatting row: fourteen buttons in four groups — text styles and
        // links; headings; lists and quote; block inserts. Icon buttons where
        // the asset set has a glyph and short text labels for the rest.
        let mut formats = div().flex().flex_row().items_center().gap_0p5();
        let spec: [(&str, &str, Option<&str>, bool); 14] = [
            ("bold", "B", None, false),
            ("italic", "I", None, false),
            ("strike", "S", None, false),
            ("code", "", Some("icons/icon-code.svg"), false),
            ("link", "", Some("icons/icon-link.svg"), false),
            ("h1", "H1", None, true),
            ("h2", "H2", None, false),
            ("h3", "H3", None, false),
            ("bullet", "", Some("icons/icon-list-bullet.svg"), true),
            ("numbered", "", Some("icons/icon-list-ordered.svg"), false),
            ("task", "", Some("icons/icon-task.svg"), false),
            ("quote", "", Some("icons/icon-quote.svg"), false),
            ("codeblock", "", Some("icons/icon-code-block.svg"), true),
            ("hr", "—", None, false),
        ];
        for (format, label, icon_path, separator) in spec {
            if separator {
                formats = formats.child(
                    div()
                        .mx_1()
                        .h(px(theme::scaled(14.)))
                        .w(px(1.))
                        .bg(theme::line_strong()),
                );
            }
            let mut button = div()
                .id(SharedString::from(format!("fmt-{format}")))
                .a11y_button(format_label(format))
                .cursor_pointer()
                .rounded(px(6.))
                .size(px(theme::scaled(26.)))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(theme::font_meta()))
                .text_color(theme::text_muted())
                .hover(|s| s.bg(theme::raised()).text_color(theme::text()))
                .on_click(
                    cx.listener(move |this, _, window, cx| this.apply_format(format, window, cx)),
                );
            if let Some(path) = icon_path {
                button = button.child(icon(
                    path,
                    px(theme::icon_small() - 2.),
                    theme::text_muted(),
                ));
            } else {
                button = button.child(label);
            }
            formats = formats.child(button);
        }

        let title = self
            .open_note
            .as_ref()
            .and_then(|path| path.file_stem())
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "Untitled".into());
        let pin_path = self.open_note.clone();
        let pinned = pin_path.as_deref().is_some_and(|path| self.is_pinned(path));
        let toggle_path = pin_path.clone();
        let verbs_path = pin_path.clone();

        div()
            .id("notes-toolbar")
            .debug_selector(|| "notes-toolbar".into())
            .flex()
            .flex_col()
            .flex_shrink_0()
            .pt_4()
            .px(px(theme::scaled(shell::DETAIL_PADDING)))
            .pb_2()
            .border_b_1()
            .border_color(theme::hairline())
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(theme::scaled(10.)))
                    .min_h(px(theme::scaled(36.)))
                    .child(
                        div()
                            .id("note-pin-toggle")
                            .debug_selector(|| "note-pin-toggle".into())
                            .a11y_toggle(if pinned { "Unpin note" } else { "Pin note" }, pinned)
                            .cursor_pointer()
                            .flex_shrink_0()
                            .size(px(theme::scaled(24.)))
                            .rounded(px(4.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .hover(|d| d.bg(theme::raised()))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(path) = toggle_path.clone() {
                                    this.toggle_pin(&path, cx);
                                }
                            }))
                            .child(icon(
                                if pinned {
                                    "icons/icon-star-filled.svg"
                                } else {
                                    "icons/icon-star.svg"
                                },
                                px(theme::icon_medium()),
                                if pinned {
                                    theme::accent_ink()
                                } else {
                                    theme::text_muted()
                                },
                            )),
                    )
                    .child(
                        div()
                            .id("note-title")
                            .debug_selector(|| "note-title".into())
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(theme::font_screen_title()))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::text())
                            .child(title),
                    )
                    .when(self.conflict, |row| {
                        row.child(
                            div()
                                .flex_shrink_0()
                                .text_size(px(theme::font_meta()))
                                .text_color(theme::DANGER)
                                .child("● changed on disk"),
                        )
                    })
                    .when(dirty && !self.conflict, |row| {
                        row.child(
                            div()
                                .flex_shrink_0()
                                .text_size(px(theme::font_meta()))
                                .text_color(theme::CHANGES)
                                .child("● unsaved"),
                        )
                    })
                    .child(
                        warpui_kit_core::controls::icon_button("note-verbs", "Note menu")
                            .debug_selector(|| "note-verbs".into())
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                                    if let Some(path) = verbs_path.clone() {
                                        this.open_row_menu(path, false, event.position, cx);
                                    }
                                    cx.stop_propagation();
                                }),
                            )
                            .child(icon(
                                "icons/icon-more.svg",
                                px(theme::icon_small()),
                                theme::text_dim(),
                            )),
                    ),
            )
            .when_some(self.open_note.clone(), |toolbar, path| {
                let age = edited_age(&path, &mut self.age_cache);
                toolbar.child(
                    div()
                        .mt_1()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .gap(px(theme::scaled(14.)))
                        .text_size(px(theme::font_meta()))
                        .text_color(theme::text_dim())
                        .child(
                            div()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .font_family(SharedString::from(theme::code_font()))
                                .text_size(px(theme::font_mono_meta()))
                                .max_w(px(theme::scaled(theme::NOTE_PATH_MAX_W)))
                                .child(compact_path(&self.store_root, &path)),
                        )
                        .child(format!("Edited {age}")),
                )
            })
            .child(formats)
            .into_any_element()
    }

    /// The live editor at note size; everything else scales from it.
    fn editor_element(&self, cx: &Context<Self>) -> LiveEditor {
        let page = cx.weak_entity();
        LiveEditor::new(&self.editor)
            .aria_label("Note body")
            .text_size(px(theme::font_note()))
            .surface(theme::panel())
            .on_paste(move |item, window, cx| Self::paste_from_clipboard(&page, item, window, cx))
    }

    fn render_editor(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        if self.open_note.is_none() {
            let can_create = self.notes_scope.is_some() || !self.tree.is_empty();
            return div()
                .id("notes-empty")
                .debug_selector(|| "notes-empty".into())
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_2()
                .child(icon(
                    "icons/icon-note.svg",
                    px(theme::icon_large() + 8.),
                    theme::text_faint(),
                ))
                .child(
                    div()
                        .text_size(px(theme::font_heading()))
                        .text_color(theme::text_muted())
                        .child("No note open"),
                )
                .child(
                    div()
                        .text_size(px(theme::font_body()))
                        .text_color(theme::text_faint())
                        .child("Pick one from the tree, or start a new note."),
                )
                .when(can_create, |column| {
                    column.child(
                        div()
                            .id("notes-empty-new")
                            .debug_selector(|| "notes-empty-new".into())
                            .a11y_button("New note")
                            .cursor_pointer()
                            .rounded(px(6.))
                            .h(px(theme::scaled(shell::CONTROL_HEIGHT)))
                            .px_4()
                            .mt_2()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_1p5()
                            .text_size(px(theme::font_control()))
                            .font_weight(FontWeight::MEDIUM)
                            // the design's plain button: panel fill, control
                            // border — the accent outline was this app's own
                            // invention, and the empty state's one primary is
                            // the New note control in the tree above it
                            .border_1()
                            .border_color(theme::line_strong())
                            .bg(theme::panel())
                            .text_color(theme::text())
                            .hover(|s| s.bg(theme::raised()))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.create_open = true;
                                this.create_name
                                    .update(cx, |field, cx| field.focus(window, cx));
                                cx.notify();
                            }))
                            .child(icon(
                                "icons/icon-plus.svg",
                                px(theme::icon_small() - 2.),
                                theme::text_dim(),
                            ))
                            .child("New note"),
                    )
                })
                .into_any_element();
        }
        div()
            .id("notes-live")
            .debug_selector(|| "notes-live".into())
            .flex_1()
            .min_h_0()
            // the editor scrolls on every wheel over its surface (its own
            // handler, scrollbar included) and never chains past its edge —
            // so the event ends here instead of reaching anything outside
            .on_scroll(|_, _, cx| cx.stop_propagation())
            .child(self.editor_element(cx))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{fuzzy_score, match_ranges};

    #[test]
    fn fuzzy_score_prefers_prefix_and_boundary_matches() {
        let exact = fuzzy_score("note", "note.md").unwrap();
        let mid = fuzzy_score("note", "denote.md").unwrap();
        assert!(exact > mid, "a prefix outranks a mid-word hit");
        assert!(fuzzy_score("pn", "projects/note.md").is_some());
        assert!(fuzzy_score("NOTE", "projects/note.md").is_some());
        assert!(fuzzy_score("zzz", "projects/note.md").is_none());
    }

    #[test]
    fn outline_headings_carry_their_byte_offsets() {
        let headings = super::headings_for("# Note\n\n## Section\n");
        assert_eq!(headings[0].label, "Note");
        assert_eq!(headings[0].byte, 2);
        assert_eq!(headings[1].level, 2);
        assert_eq!(
            &"# Note\n\n## Section\n"[headings[1].byte..][..7],
            "Section"
        );
    }

    #[test]
    fn search_highlight_ranges_cover_every_match() {
        assert_eq!(
            match_ranges("a needle and needle", "needle"),
            vec![2..8, 13..19]
        );
        assert!(match_ranges("no match", "needle").is_empty());
        assert!(
            match_ranges("İstanbul", "i").is_empty(),
            "unsafe offsets are dropped"
        );
    }
}
