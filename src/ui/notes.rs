//! The Notes page: per-repo markdown notes — folder/file tree on the left,
//! a single Obsidian-style live editor on the right. Markdown renders in
//! place while typing: source syntax stays hidden until the caret touches it,
//! task checkboxes toggle on click, Enter continues lists, links open, and
//! the toolbar drives the pure markdown editing ops. Dirty in-memory buffers
//! survive navigation, and search is capped.

use crate::notes_store as notes_core;
use crate::theme;
use crate::ui::A11y;
use crate::ui::icon;
use gpui::prelude::*;
use gpui::{
    App, Context, Entity, FontWeight, MouseButton, SharedString, WeakEntity, Window,
    WindowControlArea, div, px,
};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use warpui_kit_markdown::{LiveEditor, LiveEditorEvent, LiveEditorState, apply};

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

struct Notice {
    kind: NoticeKind,
    text: String,
    expires: std::time::Instant,
}

const NOTICE_TTL: std::time::Duration = std::time::Duration::from_secs(4);
/// Tooltips always show in the standalone window (DevTools gates its own
/// embedded surfaces by a setting; this app has none).
const SHOW_TOOLTIPS: bool = true;

pub struct NotesPage {
    /// The notes store root: --store when given, else ~/.warp/notes.
    store_root: PathBuf,
    /// The tree's pinned scope: None roots at the whole notes store (one
    /// folder per repo), Some at one repo's notes folder. Fixed after launch
    /// except through the titlebar's up button.
    notes_scope: Option<String>,
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
    /// The live markdown editor.
    editor: Entity<LiveEditorState>,
    /// Delete arming: (path, expire-instant) — a second click within 3s fires.
    armed_delete: Option<(PathBuf, std::time::Instant)>,
    /// The "+ note" create form (name + optional folder).
    create_open: bool,
    create_name: Entity<InputState>,
    create_folder: Entity<InputState>,
    /// The transient toast strip: save/create/delete outcomes.
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
        let mut page = Self {
            store_root,
            notes_scope,
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

    /// The transient toast: shows in the status strip until it expires; a
    /// spawned timer hides it so no user action is needed.
    fn notice(&mut self, cx: &mut Context<Self>, kind: NoticeKind, text: &str) {
        let expires = std::time::Instant::now() + NOTICE_TTL;
        self.notice = Some(Notice {
            kind,
            text: text.to_string(),
            expires,
        });
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(NOTICE_TTL).await;
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

        div()
            .size_full()
            .flex()
            .flex_col()
            .min_h_0()
            .bg(theme::BACKGROUND)
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
                } else if event.keystroke.key == "escape" && this.create_open {
                    this.create_open = false;
                    cx.notify();
                    cx.stop_propagation();
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
                    .gap_2()
                    .px_2()
                    .pb_2()
                    .child(self.render_tree(cx))
                    .child(
                        div()
                            .id("notes-editor-column")
                            .debug_selector(|| "notes-editor-column".into())
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .flex()
                            .flex_col()
                            .min_h_0()
                            .rounded_lg()
                            .border_1()
                            .border_color(theme::HAIRLINE)
                            .bg(theme::CARD)
                            .child(self.render_toolbar(note_title, dirty, armed, window, cx))
                            .child(self.render_editor(cx))
                            .when(self.open_note.is_some(), |column| {
                                column.child(self.render_status_bar(cx))
                            }),
                    ),
            )
    }
}

impl NotesPage {
    /// The floating window paints its own title bar: the window is created
    /// with appears_transparent, so without this strip there is no title,
    /// drag region, or window controls.
    fn render_titlebar(&mut self, window: &Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        div()
            .h(px(38.))
            .flex()
            .items_center()
            .px_2()
            .gap_1()
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
            .child(icon(
                "icons/icon-note.svg",
                px(theme::icon_medium()),
                theme::TEXT_MUTED,
            ))
            .child(
                div()
                    .text_size(px(theme::font_title()))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Notes"),
            )
            .when_some(self.notes_scope.clone(), |row, repo| {
                row.child(
                    div()
                        .rounded_md()
                        .bg(theme::CHIP)
                        .px_1p5()
                        .py_0p5()
                        .text_size(px(theme::font_small() - 1.))
                        .text_color(theme::TEXT_MUTED)
                        .child(repo),
                )
                // up out of the repo's notes folder to the whole store
                // (one folder per repo)
                .child(crate::ui::window_button(
                    "notes-up-dir",
                    "icons/icon-arrow-up.svg",
                    theme::HOVER,
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
            .child(crate::ui::window_button(
                "notes-minimize",
                "icons/icon-window-minimize.svg",
                theme::HOVER,
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
                theme::HOVER,
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

    /// The status strip: "N words · N chars · N lines", "Ln x, Col y", and
    /// the transient notice when one is live.
    fn render_status_bar(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        if let Some(notice) = &self.notice
            && notice.expires <= std::time::Instant::now()
        {
            self.notice = None;
        }
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
            .px_2()
            .py_1()
            .border_t_1()
            .border_color(theme::HAIRLINE)
            .text_size(px(theme::font_small() - 1.))
            .text_color(theme::TEXT_FAINT)
            .child(format!("{words} words · {chars} chars · {lines} lines"))
            .child(div().flex_1());
        strip = match &self.notice {
            Some(notice) => strip.child(
                div()
                    .text_color(match notice.kind {
                        NoticeKind::Info => theme::TEXT_DIM,
                        NoticeKind::Error => theme::DANGER,
                    })
                    .child(notice.text.clone()),
            ),
            None if dirty => strip.child(
                div()
                    .text_color(theme::AMBER_BRIGHT)
                    .child("Unsaved · Ctrl+S to save"),
            ),
            None => strip,
        };
        strip
            .child(format!("Ln {line}, Col {col}"))
            .into_any_element()
    }

    fn render_tree(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let term = self.search.read(cx).value().to_string();
        let can_create = self.notes_scope.is_some() || !self.tree.is_empty();
        let mut tree =
            div()
                .id("notes-sidebar")
                .debug_selector(|| "notes-sidebar".into())
                .w(px(300.))
                .h_full()
                .flex()
                .flex_col()
                .min_h_0()
                .gap_2()
                .p_2()
                .rounded_lg()
                .border_1()
                .border_color(theme::HAIRLINE)
                .bg(theme::CARD)
                // search row: field, "+" new note, collapse-all
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_1()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .on_key_down(cx.listener(
                                    |this, event: &gpui::KeyDownEvent, window, cx| {
                                        if event.keystroke.key == "escape" {
                                            this.search.update(cx, |field, cx| {
                                                field.set_value(String::new(), window, cx)
                                            });
                                            this.search_hits.clear();
                                            cx.notify();
                                        }
                                    },
                                ))
                                .child(Input::new(&self.search).small().cleanable(true).prefix(
                                    icon(
                                        "icons/icon-search.svg",
                                        px(theme::icon_small() - 1.),
                                        theme::TEXT_FAINT,
                                    ),
                                )),
                        )
                        .when(can_create, |row| {
                            row.child(
                                div()
                                    .id("note-new")
                                    .a11y_toggle("New note", self.create_open)
                                    .cursor_pointer()
                                    .rounded_md()
                                    .size(px(22.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .hover(|s| s.bg(theme::HOVER))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.create_open = !this.create_open;
                                        cx.notify();
                                    }))
                                    .child(icon(
                                        "icons/icon-plus.svg",
                                        px(theme::icon_small() - 1.),
                                        theme::TEXT_MUTED,
                                    )),
                            )
                        })
                        .child(
                            div()
                                .id("notes-collapse-all")
                                .debug_selector(|| "notes-collapse-all".into())
                                .a11y_button("Collapse all folders")
                                .cursor_pointer()
                                .rounded_md()
                                .size(px(22.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .hover(|s| s.bg(theme::HOVER))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.expanded.clear();
                                    cx.notify();
                                }))
                                .child(icon(
                                    "icons/icon-list-bullet.svg",
                                    px(theme::icon_small() - 1.),
                                    theme::TEXT_MUTED,
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
                            .child(Input::new(&self.create_name).small())
                            .child(Input::new(&self.create_folder).small())
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_1()
                                    .child(
                                        div()
                                            .id("note-create-confirm")
                                            .a11y_button("Create note")
                                            .cursor_pointer()
                                            .rounded_md()
                                            .px_2()
                                            .py_0p5()
                                            .text_size(px(theme::font_small()))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .bg(theme::ACCENT)
                                            .text_color(theme::ON_ACCENT)
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
                                            .rounded_md()
                                            .px_2()
                                            .py_0p5()
                                            .text_size(px(theme::font_small()))
                                            .text_color(theme::TEXT_FAINT)
                                            .hover(|s| s.bg(theme::HOVER))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.create_open = false;
                                                cx.notify();
                                            }))
                                            .child("×"),
                                    ),
                            ),
                    )
                });

        if !term.trim().is_empty() {
            let mut hits = div()
                .id("notes-hits")
                .flex()
                .flex_col()
                .gap_1()
                .overflow_y_scroll();
            for (index, hit) in self.search_hits.iter().enumerate() {
                hits = hits.child(
                    div()
                        .id(("search-hit", index as u64))
                        .a11y_button(format!("{} — {}", hit.title, hit.excerpt))
                        .cursor_pointer()
                        .rounded_md()
                        .px_2()
                        .py_1()
                        .hover(|s| s.bg(theme::HOVER))
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
                                        .text_size(px(theme::font_small()))
                                        .text_color(theme::TEXT_DIM)
                                        .child(hit.title.clone()),
                                )
                                .child(
                                    div()
                                        .text_size(px(theme::font_small() - 2.))
                                        .text_color(theme::TEXT_FAINT)
                                        .child(hit.excerpt.clone()),
                                ),
                        ),
                );
            }
            tree = tree.child(hits);
        } else {
            // Rc clone — the tree is shared with reload sites, never copied
            let items = self.tree.clone();
            tree = tree.child(
                div()
                    .id("notes-tree-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(self.render_tree_items(&items, 0, cx)),
            );
        }
        tree.into_any_element()
    }

    fn render_tree_items(
        &mut self,
        items: &[notes_core::NotesTreeItem],
        depth: usize,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let mut list = div().flex().flex_col().gap_0p5();
        for (index, item) in items.iter().enumerate() {
            let expanded = self.expanded.contains(&item.full_path);
            let selected = self.open_note.as_ref() == Some(&item.full_path);
            let id = gpui::ElementId::NamedInteger(
                SharedString::from(format!("note-item-{}-{}", depth, item.name)),
                index as u64,
            );
            let selector_name = item.name.clone();
            let row = div()
                .id(id)
                .debug_selector(move || format!("note-item-{}", selector_name))
                .a11y_tree_item(item.name.clone(), item.is_folder.then_some(expanded))
                .cursor_pointer()
                .rounded_md()
                .px_1p5()
                .py_0p5()
                .text_size(px(theme::font_small()))
                .when(selected, |d| {
                    d.bg(theme::tint(theme::ACCENT)).text_color(theme::ACCENT)
                })
                .when(!selected, |d| d.text_color(theme::TEXT_DIM))
                .hover(|s| s.bg(theme::HOVER))
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
                        .pl(px(depth as f32 * 10.0))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_1()
                        .when(item.is_folder, |row| {
                            row.child(icon(
                                if expanded {
                                    "icons/icon-chevron-down.svg"
                                } else {
                                    "icons/icon-chevron-right.svg"
                                },
                                px(theme::icon_small() - 4.),
                                theme::TEXT_FAINT,
                            ))
                        })
                        .child(if item.is_folder {
                            icon(
                                "icons/icon-folder.svg",
                                px(theme::icon_small() - 2.),
                                theme::TEXT_MUTED,
                            )
                        } else {
                            icon(
                                "icons/icon-note.svg",
                                px(theme::icon_small() - 2.),
                                theme::TEXT_MUTED,
                            )
                        })
                        .child(item.name.clone()),
                );
            list = list.child(row);
            if item.is_folder && expanded {
                list = list.child(self.render_tree_items(&item.children, depth + 1, cx));
            }
        }
        list.into_any_element()
    }

    fn render_toolbar(
        &mut self,
        note_title: String,
        dirty: bool,
        armed: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let _ = window;

        // formatting row: text styles, headings, lists and inserts, separated
        // into groups; icon buttons where the asset set has a glyph and short
        // text labels for the rest (B/I/S/H1–H3)
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
                formats = formats.child(div().mx_1().h(px(14.)).w(px(1.)).bg(theme::HAIRLINE));
            }
            let mut button = div()
                .id(SharedString::from(format!("fmt-{format}")))
                .a11y_button(format_label(format))
                .cursor_pointer()
                .rounded_md()
                .size(px(24.))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(theme::font_small() - 1.))
                .text_color(theme::TEXT_MUTED)
                .hover(|s| s.bg(theme::HOVER).text_color(theme::TEXT))
                .on_click(
                    cx.listener(move |this, _, window, cx| this.apply_format(format, window, cx)),
                );
            if let Some(path) = icon_path {
                button = button.child(icon(path, px(theme::icon_small() - 1.), theme::TEXT_MUTED));
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
            .gap_1()
            .px_2()
            .py_1p5()
            .border_b_1()
            .border_color(theme::HAIRLINE)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    // filename header (mono, like a document title strip)
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .font_family(SharedString::from(theme::code_font()))
                            .text_size(px(theme::font_small()))
                            .text_color(theme::TEXT_MUTED)
                            .child(note_title.clone()),
                    )
                    .when(dirty, |row| {
                        row.child(
                            div()
                                .text_size(px(theme::font_small() - 1.))
                                .text_color(theme::AMBER_BRIGHT)
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
                                .rounded_md()
                                .px_2()
                                .py_0p5()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap_1()
                                .text_size(px(theme::font_small()))
                                .text_color(theme::TEXT_DIM)
                                .border_1()
                                .border_color(theme::HAIRLINE)
                                .hover(|s| s.bg(theme::HOVER))
                                .on_click(
                                    cx.listener(|this, _, window, cx| {
                                        this.save_current(window, cx)
                                    }),
                                )
                                .child(icon(
                                    "icons/icon-save.svg",
                                    px(theme::icon_small() - 1.),
                                    theme::TEXT_MUTED,
                                ))
                                .child("Save"),
                        )
                        .child(
                            div()
                                .id("note-delete")
                                .a11y_button(if armed {
                                    "Confirm delete note"
                                } else {
                                    "Delete note"
                                })
                                .cursor_pointer()
                                .size(px(24.))
                                .rounded_md()
                                .flex()
                                .items_center()
                                .justify_center()
                                .when(armed, |d| {
                                    d.border_1().border_color(theme::tint(theme::DANGER))
                                })
                                .when(!armed, |d| d.hover(|s| s.bg(theme::HOVER)))
                                .on_click(cx.listener(|this, _, _, cx| this.delete_current(cx)))
                                .child(icon(
                                    "icons/icon-trash.svg",
                                    px(theme::icon_small() - 1.),
                                    if armed {
                                        theme::DANGER
                                    } else {
                                        theme::TEXT_FAINT
                                    },
                                )),
                        )
                    }),
            )
            .when(self.open_note.is_some(), |toolbar| toolbar.child(formats))
            .into_any_element()
    }

    /// The live editor at note size; everything else scales from it.
    fn editor_element(&self, cx: &Context<Self>) -> LiveEditor {
        let page = cx.weak_entity();
        LiveEditor::new(&self.editor)
            .aria_label("Note body")
            .text_size(px(theme::font_note()))
            .surface(theme::CARD)
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
                    theme::TEXT_FAINT,
                ))
                .child(
                    div()
                        .text_size(px(theme::font_body() + 1.))
                        .text_color(theme::TEXT_MUTED)
                        .child("No note open"),
                )
                .child(
                    div()
                        .text_size(px(theme::font_small()))
                        .text_color(theme::TEXT_FAINT)
                        .child("Pick one from the tree, or start a new note."),
                )
                .when(can_create, |column| {
                    column.child(
                        div()
                            .id("notes-empty-new")
                            .a11y_button("New note")
                            .cursor_pointer()
                            .rounded_md()
                            .border_1()
                            .border_color(theme::ACCENT)
                            .px_3()
                            .py_1p5()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_1p5()
                            .text_size(px(theme::font_small()))
                            .text_color(theme::ACCENT)
                            .hover(|s| s.bg(theme::tint(theme::ACCENT)))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.create_open = true;
                                this.create_name
                                    .update(cx, |field, cx| field.focus(window, cx));
                                cx.notify();
                            }))
                            .child(icon(
                                "icons/icon-plus.svg",
                                px(theme::icon_small()),
                                theme::ACCENT,
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
