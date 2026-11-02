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
    App, Context, Entity, FontWeight, MouseButton, Pixels, SharedString, WeakEntity, Window,
    WindowControlArea, canvas, div, px, uniform_list,
};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use std::cell::Cell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use warpui_kit_core::{motion, shell};
use warpui_kit_markdown::{LiveEditor, LiveEditorEvent, LiveEditorState, apply};

// Ctrl+F: open the editor's find bar (bound in main.rs).
gpui::actions!(warp_notes, [OpenFind]);

/// The splitter between the list and the detail.
const SPLITTER_WIDTH: f32 = 4.;

/// The rail's sections.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Section {
    Notes,
    Settings,
}

/// The launch configuration the CLI parsed: the store root (DevTools passes
/// the resolved `notes_store_path`; standalone defaults to ~/.warp/notes),
/// an optional pinned repo scope and an optional note to open right away.
#[derive(Clone, Debug, Default)]
pub struct Launch {
    pub store_root: Option<PathBuf>,
    pub repo_scope: Option<String>,
    pub open_note: Option<PathBuf>,
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
const TREE_ROW_HEIGHT: f32 = 34.;

const SHOW_TOOLTIPS: bool = true;

pub struct NotesPage {
    /// The notes store root: --store when given, else ~/.warp/notes.
    pub(crate) store_root: PathBuf,
    /// The tree's pinned scope: None roots at the whole notes store (one
    /// folder per repo), Some at one repo's notes folder. Fixed after launch
    /// except through the titlebar's up button.
    notes_scope: Option<String>,
    /// The rail's open section: the tree + editor, or Settings.
    section: Section,
    /// The Settings section's own section: General or Appearance.
    pub(crate) settings_section: SettingsSection,
    /// The remembered window bits: list width and reduce motion.
    pub(crate) prefs: UiPrefs,
    /// Ctrl+\ and the title bar's toggle hide the list pane.
    list_visible: bool,
    /// active list resize: (start pointer x, start list width); shared with
    /// the paint-phase listener registration because window.on_mouse_event
    /// may only be called during paint in this gpui rev
    splitter_drag: Rc<Cell<Option<(Pixels, f32)>>>,
    /// The loaded tree behind an Rc: every editor keystroke re-renders the
    /// page, and the rebuild must not deep-clone the whole tree per frame.
    tree: Rc<Vec<notes_core::NotesTreeItem>>,
    tree_loaded_for: Option<PathBuf>,
    /// The tree changed: the editor's wikilinks need the new note names.
    note_names_stale: bool,
    expanded: std::collections::HashSet<PathBuf>,
    open_note: Option<PathBuf>,
    /// Unsaved note contents by path — navigation never loses an edit.
    dirty: HashMap<PathBuf, String>,
    /// The on-disk contents of the open note; comparing against the editor
    /// makes the unsaved marker follow every keystroke.
    saved_text: String,
    search: Entity<InputState>,
    search_hits: Vec<notes_core::NotesSearchHit>,
    /// Debounce sequence; a keystroke invalidates any in-flight search.
    search_generation: u32,
    /// Ctrl+F in the editor: the find bar, its byte ranges and the current hit.
    find_open: bool,
    find: Entity<InputState>,
    find_hits: Vec<std::ops::Range<usize>>,
    find_index: usize,
    /// The live markdown editor.
    editor: Entity<LiveEditorState>,
    /// Delete arming: (path, expire-instant) — a second click within 3s fires.
    armed_delete: Option<(PathBuf, std::time::Instant)>,
    /// The "+ note" create form (name + optional folder).
    create_open: bool,
    create_name: Entity<InputState>,
    create_folder: Entity<InputState>,
    /// The transient toast: save/create/delete outcomes, bottom right.
    notice: Option<Notice>,
    /// A left press landed on the titlebar background; the first move turns
    /// it into a window drag (Linux/macOS — Windows uses the HTCAPTION path).
    titlebar_should_move: bool,
}

impl NotesPage {
    pub fn new(launch: Launch, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store_root = match launch.store_root {
            Some(root) => root,
            None => crate::paths::default_store_root(),
        };
        let _ = std::fs::create_dir_all(&store_root);
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
        cx.subscribe(&search, |this, field, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.run_search(field.read(cx).value().to_string(), cx);
            }
        })
        .detach();
        // rendered links lead out of the editor: the browser, or another note
        cx.subscribe_in(
            &editor,
            window,
            |this, _, event: &LiveEditorEvent, window, cx| match event {
                LiveEditorEvent::OpenUrl(url) => cx.open_url(url),
                LiveEditorEvent::OpenNote(target) => this.follow_link(target, window, cx),
                LiveEditorEvent::Change => {}
            },
        )
        .detach();
        // the status strip and unsaved marker follow every keystroke and
        // every caret move
        cx.observe(&editor, |_, _, cx| cx.notify()).detach();
        // the remembered bits: list width, and the motion choice (applied
        // before the first frame so nothing animates that should not)
        let prefs = UiPrefs::load();
        // the setting or the desktop's own preference, whichever asks first
        motion::set_reduced(prefs.reduce_motion || motion::system_prefers_reduced());
        // E2E hook: WARP_NOTES_PAGE=settings opens the rail's Settings section
        // on launch (the probes drive surfaces the pointer cannot reach
        // without taking the human's screen)
        let section = match std::env::var("WARP_NOTES_PAGE").as_deref() {
            Ok("settings") => Section::Settings,
            _ => Section::Notes,
        };
        let mut page = Self {
            store_root,
            notes_scope,
            section,
            settings_section: SettingsSection::General,
            prefs,
            list_visible: true,
            splitter_drag: Rc::new(Cell::new(None)),
            tree: Rc::new(Vec::new()),
            tree_loaded_for: None,
            note_names_stale: true,
            expanded: std::collections::HashSet::new(),
            open_note: None,
            dirty: HashMap::new(),
            saved_text: String::new(),
            search,
            search_hits: Vec::new(),
            search_generation: 0,
            find_open: false,
            find,
            find_hits: Vec::new(),
            find_index: 0,
            editor,
            armed_delete: None,
            create_open: false,
            create_name,
            create_folder,
            notice: None,
            titlebar_should_move: false,
        };
        // --note: the requested note opens right away, tree expanded to it.
        // Contained like every other store path — a stray --note must not
        // reach (or on save, overwrite) a file outside the store.
        if let Some(path) = launch.open_note {
            match notes_core::resolve_note_in_store(&page.store_root, &path) {
                Ok(path) => {
                    let root = page.notes_root();
                    for parent in path.ancestors().skip(1) {
                        if let Some(root) = &root
                            && parent.starts_with(root)
                        {
                            page.expanded.insert(parent.to_path_buf());
                        }
                        if parent.parent().is_none() {
                            break;
                        }
                    }
                    page.open_note(path, window, cx);
                }
                Err(err) => page.notice(
                    cx,
                    NoticeKind::Error,
                    &format!("cannot open {}: {err}", path.display()),
                ),
            }
        }
        page
    }

    fn notes_root(&self) -> Option<PathBuf> {
        // the pinned scope decides — repo-scoped when launch or the up button
        // set one, whole-store otherwise (one folder per repo)
        match &self.notes_scope {
            Some(repo) => Some(notes_core::repo_notes_root(&self.store_root, repo)),
            None => Some(self.store_root.clone()),
        }
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

    pub(crate) fn open_note(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        // park the current buffer before switching
        if let Some(current) = self.open_note.clone() {
            let text = self.editor.read(cx).value().to_string();
            self.dirty.insert(current, text);
        }
        // a note that cannot be read (non-UTF-8, permissions, deleted
        // underneath us) must not open as an empty-looking editor — the first
        // save would then replace the original file with the typed fragment
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
        self.saved_text = baseline;
        // the bar belongs to the note it was opened on
        self.find_open = false;
        self.find_hits.clear();
        self.find_index = 0;
        let folder = path.parent().map(Path::to_path_buf);
        self.open_note = Some(path);
        self.editor.update(cx, |state, cx| {
            // relative image paths resolve next to the note
            state.set_base_dir(folder, cx);
            state.set_value(content, window, cx);
            state.focus(window, cx);
        });
        cx.notify();
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

    fn save_current(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.open_note.clone() else {
            return;
        };
        let text = self.editor.read(cx).value().to_string();
        match notes_core::write_note(&path, &text) {
            Ok(()) => {
                self.dirty.remove(&path);
                self.saved_text = text;
                tracing::info!("saved {}", path.display());
            }
            Err(err) => self.notice(cx, NoticeKind::Error, &format!("note save failed: {err}")),
        }
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
        if term.is_empty() {
            self.search_hits.clear();
            cx.notify();
            return;
        }
        let Some(root) = self.notes_root() else {
            return;
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
                    cx.notify();
                }
            });
        })
        .detach();
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
                }
                self.open_note(path, window, cx);
            }
            Err(err) => self.notice(cx, NoticeKind::Error, &format!("note create failed: {err}")),
        }
    }

    /// The bottom-right toast: an error stays longer and carries a close
    /// button; a spawned timer hides it so no user action is needed.
    fn notice(&mut self, cx: &mut Context<Self>, kind: NoticeKind, text: &str) {
        let ttl = notice_ttl(kind);
        let expires = std::time::Instant::now() + ttl;
        self.notice = Some(Notice {
            kind,
            text: text.to_string(),
            expires,
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

    fn delete_current(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.open_note.clone() else {
            return;
        };
        let Some(root) = self.notes_root() else {
            return;
        };
        match &self.armed_delete {
            Some((armed, _)) if *armed == path => {
                match notes_core::delete(&root, &path) {
                    Ok(()) => {
                        self.dirty.remove(&path);
                        self.open_note = None;
                        self.saved_text.clear();
                        self.refresh_tree(&root);
                    }
                    Err(err) => {
                        self.notice(cx, NoticeKind::Error, &format!("note delete failed: {err}"))
                    }
                }
                self.armed_delete = None;
            }
            _ => {
                self.armed_delete = Some((
                    path,
                    std::time::Instant::now() + std::time::Duration::from_secs(3),
                ));
            }
        }
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

    /// The editor entity — for headless probes (caret, layout hit-testing).
    #[doc(hidden)]
    pub fn editor_handle(&self) -> &Entity<LiveEditorState> {
        &self.editor
    }
}

fn byte_to_char(text: &str, byte: usize) -> usize {
    text[..byte.min(text.len())].chars().count()
}

/// Every note's file name in the tree.
fn collect_note_names(items: &[notes_core::NotesTreeItem], names: &mut Vec<String>) {
    for item in items {
        if item.is_folder {
            collect_note_names(&item.children, names);
        } else {
            names.push(item.name.clone());
        }
    }
}

/// Depth-first search of the note tree for a file name, case-insensitive like
/// Obsidian's link resolution.
fn find_note(items: &[notes_core::NotesTreeItem], file: &str) -> Option<PathBuf> {
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

/// The note's age for the head's right side: now, 5m, 2h, 3d.
fn edited_age(path: &Path) -> Option<String> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    let elapsed = std::time::SystemTime::now()
        .duration_since(modified)
        .ok()?
        .as_secs();
    Some(match elapsed {
        0..=59 => "now".to_string(),
        60..=3599 => format!("{}m", elapsed / 60),
        3600..=86_399 => format!("{}h", elapsed / 3600),
        _ => format!("{}d", elapsed / 86_400),
    })
}

impl Render for NotesPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
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
        let armed = self
            .armed_delete
            .as_ref()
            .map(|(path, deadline)| {
                Some(path) == self.open_note.as_ref() && *deadline > std::time::Instant::now()
            })
            .unwrap_or(false);
        let note_title = self
            .open_note
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "No note selected".into());

        let mut root = div()
            .size_full()
            .flex()
            .flex_col()
            .min_h_0()
            .bg(theme::background())
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                let control = event.keystroke.modifiers.control;
                if control && event.keystroke.key == "s" {
                    this.save_current(window, cx);
                    cx.stop_propagation();
                } else if control && event.keystroke.key == "n" {
                    this.create_open = true;
                    this.create_name
                        .update(cx, |field, cx| field.focus(window, cx));
                    cx.notify();
                    cx.stop_propagation();
                } else if control && event.keystroke.key == "\\" {
                    this.list_visible = !this.list_visible;
                    cx.notify();
                    cx.stop_propagation();
                } else if control && event.keystroke.key == "," {
                    this.section = match this.section {
                        Section::Settings => Section::Notes,
                        Section::Notes => Section::Settings,
                    };
                    cx.notify();
                    cx.stop_propagation();
                } else if control && event.keystroke.key == "f" && this.open_note.is_some() {
                    this.find_open = true;
                    this.find.update(cx, |field, cx| field.focus(window, cx));
                    let term = this.find.read(cx).value().to_string();
                    this.run_find(term, cx);
                    cx.notify();
                    cx.stop_propagation();
                } else if event.keystroke.key == "escape" && this.find_open {
                    this.close_find(window, cx);
                    cx.stop_propagation();
                } else if event.keystroke.key == "escape" && this.create_open {
                    this.create_open = false;
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
                    .flex()
                    .flex_row()
                    .child(self.render_rail(cx))
                    .child(self.render_body(note_title, dirty, armed, window, cx)),
            )
            .relative();
        if let Some(toasts) = self.render_toasts(cx) {
            root = root.child(toasts);
        }
        root
    }
}

impl NotesPage {
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
                    this.section = Section::Notes;
                    cx.notify();
                })),
            )
            .child(div().flex_1())
            .child(
                shell::rail_item(
                    "rail-settings",
                    "icons/icon-cog.svg",
                    "Settings",
                    section == Section::Settings,
                )
                .debug_selector(|| "rail-settings".into())
                .on_click(cx.listener(|this, _, _, cx| {
                    this.section = Section::Settings;
                    cx.notify();
                })),
            )
            .into_any_element()
    }

    /// Everything right of the rail: the list and the editor (or Settings).
    fn render_body(
        &mut self,
        note_title: String,
        dirty: bool,
        armed: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        match self.section {
            Section::Settings => self.render_settings(window, cx),
            Section::Notes => {
                let mut panes = shell::panes();
                if self.list_visible {
                    panes = panes
                        .child(self.render_tree(cx))
                        .child(self.render_splitter(cx));
                }
                panes
                    .child(
                        shell::detail_pane().child(
                            div()
                                .id("notes-editor-column")
                                .debug_selector(|| "notes-editor-column".into())
                                .flex_1()
                                .min_w_0()
                                .w_full()
                                .max_w(px(theme::scaled(shell::READING_WIDTH)))
                                .mx_auto()
                                .h_full()
                                .flex()
                                .flex_col()
                                .min_h_0()
                                .when(self.open_note.is_some(), |column| {
                                    // the head is the open file's name; with
                                    // nothing open the empty state speaks
                                    column.child(
                                        self.render_editor_head(note_title, dirty, armed, cx),
                                    )
                                })
                                .when(self.find_open, |column| {
                                    column.child(self.render_find_bar(cx))
                                })
                                .child(self.render_editor(cx))
                                .when(self.open_note.is_some(), |column| {
                                    column.child(self.render_status_bar(cx))
                                }),
                        ),
                    )
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
            motion::enter(
                "toast-enter",
                div()
                    .absolute()
                    .bottom_3()
                    .right_3()
                    .flex()
                    .flex_col()
                    .items_end()
                    .child(toast),
            )
            .into_any_element(),
        )
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
            .pl(px(14.))
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
                // (one folder per repo)
                .child(crate::ui::window_button(
                    "notes-up-dir",
                    "icons/icon-arrow-up.svg",
                    theme::raised().into(),
                    "Up to all repositories' notes",
                    SHOW_TOOLTIPS,
                    cx.listener(|this, _, _, cx| {
                        this.notes_scope = None;
                        this.expanded.clear();
                        cx.notify();
                    }),
                ))
            })
            .child(div().flex_1().h_full())
            .when(self.section == Section::Notes, |row| {
                row.child(crate::ui::window_button(
                    "notes-list-toggle",
                    "icons/icon-panel-left.svg",
                    theme::raised().into(),
                    if self.list_visible {
                        "Hide list"
                    } else {
                        "Show list"
                    },
                    SHOW_TOOLTIPS,
                    cx.listener(|this, _, _, cx| {
                        this.list_visible = !this.list_visible;
                        cx.notify();
                    }),
                ))
            })
            .child(crate::ui::window_button(
                "notes-launch-station",
                "icons/logo-station.svg",
                theme::raised().into(),
                "Open Warp Station",
                SHOW_TOOLTIPS,
                |_, _, _| {
                    if let Err(err) = std::process::Command::new("warp-station").spawn() {
                        tracing::warn!("could not launch warp-station: {err}");
                    }
                },
            ))
            .child(div().w(px(1.)).h(px(20.)).mx_1().bg(theme::hairline()))
            .child(crate::ui::window_button(
                "notes-minimize",
                "icons/icon-window-minimize.svg",
                theme::raised().into(),
                "Minimize",
                SHOW_TOOLTIPS,
                |_, window, _| window.minimize_window(),
            ))
            .child(crate::ui::window_button(
                "notes-zoom",
                if window.is_maximized() {
                    "icons/icon-window-restore.svg"
                } else {
                    "icons/icon-window-maximize.svg"
                },
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
            ))
            .child(crate::ui::window_button(
                "notes-close",
                "icons/icon-window-close.svg",
                theme::tint(theme::DANGER),
                "Close",
                SHOW_TOOLTIPS,
                |_, window, _| window.remove_window(),
            ))
            .into_any_element()
    }

    /// The find bar: the term, the match count, Enter/Shift+Enter to walk the
    /// matches, Escape or × to close. It keeps the keyboard, so the editor's
    /// selection marks the match without stealing the typing.
    fn render_find_bar(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
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
                    .child(warpui_kit_core::shell::field(
                        "field-find",
                        Input::new(&self.find).with_size(theme::field_size()),
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

    /// The status strip: "N words · N chars · N lines", the unsaved hint,
    /// and "Ln x, Col y".
    fn render_status_bar(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let text = if self.open_note.is_some() {
            self.editor.read(cx).value()
        } else {
            SharedString::new("")
        };
        let words = text.split_whitespace().count();
        let chars = text.chars().count();
        let lines = text.lines().count();
        let dirty = self.open_note.is_some() && text.as_ref() != self.saved_text.as_str();
        let (line, col) = if self.open_note.is_some() {
            let byte = self.editor.read(cx).selected_range().start;
            let prefix = &text[..byte.min(text.len())];
            let line = prefix.matches('\n').count() + 1;
            let col = prefix
                .rsplit('\n')
                .next()
                .map(|l| l.chars().count())
                .unwrap_or(0)
                + 1;
            (line, col)
        } else {
            (1, 1)
        };
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
        if dirty {
            strip = strip.child(
                div()
                    .text_color(theme::CHANGES)
                    .child("Unsaved · Ctrl+S to save"),
            );
        }
        strip
            .child(format!("Ln {line}, Col {col}"))
            .into_any_element()
    }

    /// The list pane: the tree tools over the tree (or the search results),
    /// on the shell's list surface.
    fn render_tree(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let term = self.search.read(cx).value().to_string();
        let can_create = self.notes_scope.is_some() || !self.tree.is_empty();
        let mut tree = shell::list_pane(self.prefs.list_width)
            .id("notes-sidebar")
            .debug_selector(|| "notes-sidebar".into())
            // the head: the list's title, then the two tree tools, exactly the
            // shell's list-head shape
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .px_4()
                    .pt_3p5()
                    .pb_2()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(theme::font_list_title()))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::text())
                            .child("Notes"),
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
            // search: the whole store, on the line under the title
            .child(
                div().flex_shrink_0().px_4().pb_3().child(
                    div()
                        .w_full()
                        .min_w_0()
                        .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                            if event.keystroke.key == "escape" {
                                this.search.update(cx, |field, cx| {
                                    field.set_value(String::new(), window, cx)
                                });
                                this.search_hits.clear();
                                cx.notify();
                            }
                        }))
                        .child(warpui_kit_core::shell::field(
                            "field-search",
                            Input::new(&self.search)
                                .with_size(theme::field_size())
                                .cleanable(true)
                                .prefix(icon(
                                    "icons/icon-search.svg",
                                    px(theme::icon_small() - 1.),
                                    theme::text_faint(),
                                )),
                        )),
                ),
            )
            // inline "new note" form, stacked for the narrow column
            .when(self.create_open, |card| {
                card.child(
                    div()
                        .id("note-create-form")
                        .flex()
                        .flex_col()
                        .gap_1()
                        .pt_2()
                        .px_3()
                        .child(warpui_kit_core::shell::field(
                            "field-create_name",
                            Input::new(&self.create_name).with_size(theme::field_size()),
                        ))
                        .child(warpui_kit_core::shell::field(
                            "field-create_folder",
                            Input::new(&self.create_folder).with_size(theme::field_size()),
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

        if !term.trim().is_empty() {
            let count = self.search_hits.len();
            let mut hits = div()
                .id("notes-hits")
                .flex()
                .flex_col()
                .pt_1()
                .px_2()
                .gap_0p5()
                .overflow_y_scroll()
                // the count heads the results, or the pane says there are none
                .child(
                    div()
                        .px_2()
                        .py_1p5()
                        .text_size(px(theme::font_meta()))
                        .text_color(theme::text_faint())
                        .child(if count == 0 {
                            "No matches.".to_string()
                        } else {
                            format!("{count} {}", if count == 1 { "note" } else { "notes" })
                        }),
                );
            for (index, hit) in self.search_hits.iter().enumerate() {
                hits = hits.child(
                    div()
                        .id(("search-hit", index as u64))
                        .a11y_button(format!("{} — {}", hit.title, hit.excerpt))
                        .cursor_pointer()
                        .rounded(px(6.))
                        .px_2()
                        .py_1p5()
                        .hover(|s| s.bg(theme::raised()))
                        .on_click({
                            let path = hit.path.clone();
                            cx.listener(move |this, _, window, cx| {
                                this.open_note(path.clone(), window, cx)
                            })
                        })
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .text_size(px(theme::font_body()))
                                        .text_color(theme::text_dim())
                                        .child(hit.title.clone()),
                                )
                                .child(
                                    div()
                                        .text_size(px(theme::font_meta()))
                                        .text_color(theme::text_faint())
                                        .child(hit.excerpt.clone()),
                                ),
                        ),
                );
            }
            tree = tree.child(hits);
        } else {
            // uniform_list owns the scrolling: only the rows in the viewport
            // are laid out, however many notes the store holds
            tree = tree.child(self.render_tree_list(cx));
        }
        tree.into_any_element()
    }

    /// The tree flattened to what is on screen: one entry per visible row,
    /// with its depth. A note store can hold thousands of notes, and the old
    /// recursion built an element for every one of them on every frame; the
    /// flattened form feeds `uniform_list`, which lays out only the rows in
    /// the viewport.
    fn visible_tree_rows(
        &self,
        items: &[notes_core::NotesTreeItem],
        depth: usize,
        out: &mut Vec<(notes_core::NotesTreeItem, usize)>,
    ) {
        for item in items {
            let expanded = self.expanded.contains(&item.full_path);
            out.push((item.clone(), depth));
            if item.is_folder && expanded {
                self.visible_tree_rows(&item.children, depth + 1, out);
            }
        }
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
        let expanded = self.expanded.contains(&item.full_path);
        let selected = self.open_note.as_ref() == Some(&item.full_path);
        let unsaved = self.dirty.contains_key(&item.full_path)
            || (selected && !item.is_folder && editor_text != self.saved_text.as_str());
        let id = gpui::ElementId::NamedInteger(
            SharedString::from(format!("note-item-{}-{}", depth, item.name)),
            index as u64,
        );
        let selector_name = item.name.clone();
        div()
            .id(id)
            .debug_selector(move || format!("note-item-{}", selector_name))
            .a11y_tree_item(item.name.clone(), item.is_folder.then_some(expanded))
            .relative()
            .cursor_pointer()
            .flex()
            .items_center()
            .h(px(theme::scaled(TREE_ROW_HEIGHT)))
            .pl(px(10. + depth as f32 * 14.))
            .pr_3()
            .text_size(px(theme::font_row_meta()))
            .when(selected, |d| {
                d.bg(theme::raised()).text_color(theme::text()).child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left_0()
                        .w(px(3.))
                        .bg(theme::accent()),
                )
            })
            .when(!selected, |d| d.text_color(theme::text_dim()))
            .when(!selected, |d| d.hover(|s| s.bg(theme::panel())))
            .on_click({
                let item = item.clone();
                cx.listener(move |this, _, window, cx| {
                    if item.is_folder {
                        if this.expanded.contains(&item.full_path) {
                            this.expanded.remove(&item.full_path);
                        } else {
                            this.expanded.insert(item.full_path.clone());
                        }
                    } else {
                        this.open_note(item.full_path.clone(), window, cx);
                    }
                    cx.notify();
                })
            })
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .min_w_0()
                    .flex_1()
                    .when(item.is_folder, |row| {
                        row.child(icon(
                            if expanded {
                                "icons/icon-chevron-down.svg"
                            } else {
                                "icons/icon-chevron-right.svg"
                            },
                            px(theme::icon_tiny()),
                            theme::text_faint(),
                        ))
                    })
                    .child(if item.is_folder {
                        icon(
                            "icons/icon-folder.svg",
                            px(theme::icon_tiny()),
                            theme::text_muted(),
                        )
                    } else {
                        icon(
                            "icons/icon-note.svg",
                            px(theme::icon_tiny()),
                            theme::text_muted(),
                        )
                    })
                    .child(div().truncate().min_w_0().child(item.name.clone())),
            )
            .when(unsaved, |row| {
                row.child(
                    div()
                        .flex_shrink_0()
                        .ml_1()
                        .size(px(6.))
                        .rounded_full()
                        .bg(theme::CHANGES),
                )
            })
            .into_any_element()
    }

    /// The virtualized tree: `uniform_list` renders only the rows the viewport
    /// shows, so a large store costs the same as a small one.
    fn render_tree_list(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let mut rows = Vec::new();
        let tree = self.tree.clone();
        self.visible_tree_rows(&tree, 0, &mut rows);
        let editor_text = self.editor.read(cx).value().to_string();
        let count = rows.len();
        uniform_list(
            SharedString::from("notes-tree-rows"),
            count,
            cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                range
                    .map(|index| {
                        let (item, depth) = rows[index].clone();
                        this.render_tree_row(&item, depth, index, &editor_text, cx)
                    })
                    .collect()
            }),
        )
        .flex_1()
        .min_h_0()
        .w_full()
        .debug_selector(|| "notes-tree-rows".into())
        .into_any_element()
    }

    /// The editor's head: the file name in mono, the unsaved mark, Save, and
    /// Delete (which needs a second click), over the format bar.
    fn render_editor_head(
        &mut self,
        note_title: String,
        dirty: bool,
        armed: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
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

        div()
            .id("notes-toolbar")
            .debug_selector(|| "notes-toolbar".into())
            .flex()
            .flex_col()
            .pt_3p5()
            .px_4()
            .pb_2()
            .border_b_1()
            .border_color(theme::border_light())
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .pb_2()
                    // the file name, mono like the path it is
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .font_family(SharedString::from(theme::code_font()))
                            .text_size(px(theme::font_mono_meta()))
                            .text_color(theme::text_dim())
                            .child(note_title.clone()),
                    )
                    .when(dirty, |row| {
                        row.child(
                            div()
                                .text_size(px(theme::font_meta()))
                                .text_color(theme::CHANGES)
                                .child("● unsaved"),
                        )
                    })
                    .child(div().flex_1())
                    .when(self.open_note.is_some(), |row| {
                        row.child(
                            div()
                                .id("note-save")
                                .a11y_button("Save note")
                                .cursor_pointer()
                                .rounded(px(6.))
                                .h(px(theme::scaled(shell::CONTROL_HEIGHT) - 8.))
                                .px_3()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap_1p5()
                                .text_size(px(theme::font_control()))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(theme::text())
                                .border_1()
                                .border_color(theme::line_strong())
                                .bg(theme::panel())
                                .hover(|s| s.bg(theme::raised()))
                                .on_click(
                                    cx.listener(|this, _, window, cx| {
                                        this.save_current(window, cx)
                                    }),
                                )
                                .child(icon(
                                    "icons/icon-save.svg",
                                    px(theme::icon_small() - 2.),
                                    theme::text_dim(),
                                ))
                                .child("Save"),
                        )
                        .child(
                            // destructive: the first click arms it, the
                            // second within 3s fires; anything else disarms
                            div()
                                .id("note-delete")
                                .a11y_button(if armed {
                                    "Confirm delete note"
                                } else {
                                    "Delete note"
                                })
                                .cursor_pointer()
                                .rounded(px(6.))
                                .h(px(theme::scaled(shell::CONTROL_HEIGHT) - 8.))
                                .px_3()
                                .flex()
                                .items_center()
                                .justify_center()
                                .gap_1p5()
                                .text_size(px(theme::font_control()))
                                .when(armed, |d| {
                                    d.border_1()
                                        .border_color(theme::tint(theme::DANGER))
                                        .text_color(theme::DANGER)
                                        .child("Confirm")
                                })
                                .when(!armed, |d| {
                                    d.text_color(theme::text_faint())
                                        .hover(|s| s.bg(theme::raised()))
                                })
                                .on_click(cx.listener(|this, _, _, cx| this.delete_current(cx)))
                                .when(!armed, |d| {
                                    d.child(icon(
                                        "icons/icon-trash.svg",
                                        px(theme::icon_small() - 2.),
                                        theme::text_faint(),
                                    ))
                                }),
                        )
                    }),
            )
            .when_some(self.open_note.clone(), |toolbar, path| {
                toolbar.child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .pb_1()
                        .text_size(px(theme::font_meta()))
                        .text_color(theme::text_faint())
                        .child(
                            div()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .font_family(SharedString::from(theme::code_font()))
                                .child(compact_path(&self.store_root, &path)),
                        )
                        .child(div().flex_1())
                        .when_some(edited_age(&path), |row, age| {
                            row.child(format!("Edited {age}"))
                        }),
                )
            })
            .when(self.open_note.is_some(), |toolbar| toolbar.child(formats))
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
                            .border_1()
                            .border_color(theme::accent_ink())
                            .text_color(theme::accent_ink())
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
                                theme::accent_ink(),
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
            .child(self.editor_element(cx))
            .into_any_element()
    }
}
