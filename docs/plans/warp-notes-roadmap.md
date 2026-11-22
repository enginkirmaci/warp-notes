# Plan: warp-notes as a daily-driver notes workspace

Status: steps 1–9 landed (9004760, 3cad60b, a437588, 662f587, 6272a3c,
7935d19, 35f6a35, d98484a); step 10 (templates and the daily note) is open.
The plan turns the current single-note viewer into a keyboard-first
workspace: safe file management, fast retrieval, multiple open notes, and the
context a notes app needs. Steps are ordered so each one is independently
useful and probeable. The §5 entries stay as the record of what each slice
was meant to prove; trim them as the remaining step lands.

## 1. Goal

Make warp-notes the app you reach for every day, not only when Warp Station
hands it a note: one store, many notes, keyboard-first navigation, safe file
operations, and enough context (pins, recent, outline, backlinks) to work
without a file manager.

## 2. Where the app is today

### 2.1 What is already good

The editor is not the problem. The live markdown editor, autosave with
conflict detection, image paste, wikilinks, frontmatter/mermaid/table
rendering, virtualized tree, off-thread capped search, versioned preferences
and the shared palette/text-size/motion controls are all in place. The
`--store` / `--repo` / `--root` / `--note` launch contract is solid and the
store is shared with Warp Station.

### 2.2 Gaps that make it feel small

- **One note at a time.** `NotesPage::open_note` replaces the editor. There is
  no tab strip and no back/forward history, so following a wikilink or a
  search hit always loses the previous note (the text is saved; the place is
  gone).
- **The tree is mouse-only.** Rows are divs with `on_click`; there is no tree
  focus, cursor or arrow-key navigation. `F2`, `Delete` and `Ctrl+D` have no
  tree meaning. Every file operation needs a pointer.
- **File management is missing.** The store layer has `create_note`,
  `create_folder` and `delete`; `create_folder` has no caller. There is no
  rename, move, duplicate, folder delete, context menu, drag/drop, copy path
  or restore. Delete is permanent after a two-click arm.
- **Search is narrower than its label.** The title-bar field says "Search all
  notes…", but `run_search` searches `notes_root()`. A window launched with
  `--repo` searches that one repo only. The popup lists at most eight hits,
  does not rank or highlight them, and only handles Escape; Up/Down/Enter do
  nothing. There is no full results view.
- **The tree does not follow the disk.** `reload_tree` rebuilds only when the
  root path changes. A note created by another app, or removed, stays
  invisible until the scope changes or a create/delete action refreshes the
  tree. The open note's conflict detection is good, but it only runs on a
  write; the tree has no refresh at all.
- **No organization layer.** The tree is name-sorted only: no pins, no
  recent list, no sort by modified. `[[wikilinks]]` can open or create a
  note, but nothing shows what links here. There is no outline.
- **No capture flow.** A new note is a blank file. There are no templates, no
  daily note, no quick capture path.
- **Family chrome is unfinished.** `theme.rs` still passes
  `for_preset(normalize_accent(Some("blue")))`, which normalizes to Copper;
  the accent does not follow the live palette. The window title is always
  "Warp Notes". Toasts do not mirror to the desktop while the window is
  unfocused (Warp Station does). The titlebar's "Search all notes…" and its
  actual scope disagree.
- **No shortcut reference.** Shortcuts exist (Ctrl+S/N/K/,/F, Ctrl+\) but are
  discoverable only by reading the source.

## 3. What the family already proved

| Capability | Where it exists today | What warp-notes can reuse |
| --- | --- | --- |
| Repo tabs | Warp Station's tab strip (`Ctrl+T` / `Ctrl+W` / `Ctrl+Tab`) | The same interaction for open notes, with dirty marks |
| Pinned / favorite rows | warp-agent's workspace favorites and pinned chats | Pinned notes and a pinned section |
| Context menu + prompt card | Warp Station's dialogs and menus | Tree action menu, rename/move/new-folder prompts |
| Text selection Copy menu | `warpui-kit-core::text_menu` + `gpui_kit::base::TextSelectionLayer` | Copy from backlink excerpts and other read-only panel text |
| Desktop notice mirror | Warp Station's `notices` + `desktop_notifications` | Later; see the backlog |
| Explicit refresh | Warp Station's head refresh button | A refresh button plus focus refresh in the tree |
| Live accent | warp-agent's palette-driven accent | `current_ramp().accent_colors()` in `theme::apply_kit_theme` |
| Shared picker | `warpui-kit-picker` | Optional select list for move-to-folder/templates; quick open is a command palette, not this picker |
| Shared markdown + settings | `warpui-kit-markdown`, `warpui-kit-core` | Already used; keep the live editor as the only renderer |

The app should copy the interaction patterns, not the app-specific code.

## 4. Target

### 4.1 Shell

Keep the family shell: rail, list pane, detail pane. Add:

- a per-window tab strip above the editor, with dirty dots and close buttons;
- a pinned section at the top of the tree;
- a collapsible right-hand context panel with **Outline** and **Backlinks**;
- a palette overlay for quick open and full search results.

No new color, font or size literal: every surface goes through `theme.rs` and
the kit.

The new controls need five glyphs the app does not carry today: refresh,
star (pin), more-horizontal (row actions), panel-right (context panel) and
calendar (daily note). Add them as stroked line SVGs in the family style;
reuse `icon-close.svg` for tab close and `icon-trash.svg` for Trash. No text
glyph stands in for a control.

### 4.2 Keyboard map

| Key | Action |
| --- | --- |
| `Ctrl+P` | Quick open (all notes, fuzzy path/title) |
| `Ctrl+K` | Focus the title-bar search field (existing) |
| `Ctrl+Shift+F` | Full search results for the current term |
| `Ctrl+N` | New note in the current folder |
| `Ctrl+Shift+N` | New folder |
| `F2` | Rename the selected tree entry |
| `Delete` | Move the selected tree entry to Trash |
| `Ctrl+D` | Duplicate the selected note |
| `Ctrl+Shift+E` | Focus the tree |
| `Up` / `Down` | Move the tree cursor |
| `Left` / `Right` | Collapse / expand a folder |
| `Enter` | Open the selected note or toggle the folder |
| `Ctrl+W` | Close the active tab |
| `Ctrl+Tab` / `Ctrl+Shift+Tab` | Next / previous tab |
| `Alt+Left` / `Alt+Right` | Back / forward in the note history |
| `Ctrl+\` | Toggle the list pane (existing) |
| `Ctrl+,` | Toggle Settings (existing) |
| `Ctrl+Shift+B` | Toggle the context panel |
| `Ctrl+/` | Shortcut cheat sheet |

The editor keeps its own keys. Tree keys only fire while the tree has focus;
the editor never loses typing to a tree shortcut.

### 4.3 Store and safety

- Rename, move and duplicate are path-contained and never overwrite an
  existing entry. In a repo-scoped window they stay inside that repo's
  folder; cross-repo moves are a whole-store action.
- Delete moves the entry to `<store>/.warp-notes/trash/` and offers Undo.
  There is no permanent delete outside the Trash dialog. The app's folder is
  `.warp-notes`, not Obsidian's `.trash`, so the two do not collide in a
  vault.
- Trash entries keep their store-relative path in the id folder, so a
  restore puts the note back where it came from.
- Entries older than 30 days are purged on startup.
- Hidden `.warp-notes/` (trash and templates) is skipped by the tree and by
  the shared enumerator that both warp-notes and Warp Station use, so the
  store stays compatible with the suite and with an Obsidian vault.
- Wikilinks are not rewritten on rename in v1. The rename toast says so; the
  follow-up is a link-maintenance pass.

### 4.4 Non-goals

- Cloud sync, multi-user editing, conflict merge, encryption or a lock screen.
- A second editor, rich-text/block editor or different markdown renderer.
- AI actions in this plan (a "send to warp-agent" action is a separate
  contract with the agent door).
- Drag/drop, tags, note history and export; they are in the backlog.
- Per-app build scripts or installer changes.

## 5. Steps

Each step is one commit-sized slice with its own probe. The final gate for
every step is `cargo test -p warp-notes`, `cargo fmt --all --check` and
`cargo clippy --locked --all-targets -p warp-notes -- -D warnings`; the
Windows type-check runs for anything touching paths or file moves.

The new shortcuts are actions, not a growing root `on_key_down` match.
Declare them beside `OpenFind`. Global shortcuts (quick open, full results,
tab close/cycle, the cheat sheet and the context panel) are bound in
`src/main.rs` **after** `bootstrap::init` with no focus context, the same way
the existing save/search/list toggles already behave. Tree keys (`Up`/`Down`,
`F2`, `Delete`, `Ctrl+D`, `Shift+F10`) are scoped to `"Notes > Tree"` so the
editor keeps its arrows and its Delete. Palette arrows stay in the palette's
own field; they are never global.

### Step 1 — Finish the family chrome

**Do.**

- `theme::apply_kit_theme` passes the live ramp's accent
  (`palette::current_ramp().accent_colors()`), so a palette switch moves the
  editor's primary controls with it.
- The window title follows the open note: `"note.md — Warp Notes"`, or
  `"Warp Notes"` when nothing is open. Update it on open, rename, trash and
  close.
- Add a tree-head refresh button and a focus refresh: when the window is
  activated, re-read the tree from disk without losing the expanded set, the
  cursor or the open note. On the same pass, compare the open note's
  `FileMeta`: a clean buffer reloads when the file changed, a dirty buffer
  becomes a conflict, and a deleted open note stays open as a conflict
  buffer rather than disappearing.
- No new module; this is a small, visible parity pass.

**Acceptance.**

- `cargo test -p warp-notes --test theme_probe` asserts the kit primary color
  equals `theme::accent()` after switching the ramp to Plum and back.
- `notes_probe::window_title_follows_the_open_note` reads
  `VisualTestContext::window_title()`.
- `notes_probe::refresh_sees_an_externally_created_note` writes a new `.md`
  into the store, clicks `notes-refresh`, and asserts the row appears; a
  second case removes a note and asserts the row leaves while the editor
  stays on the conflict path.
- `notes_probe::refresh_reloads_a_clean_open_note_and_conflicts_a_dirty_one`
  writes a longer body over the open file, refreshes, and asserts the new
  text for a clean buffer and the conflict bar for a dirty one.
- `notes_probe::focus_refresh_uses_the_activation_observer` writes a file,
  calls `VisualTestContext::deactivate_window()` and `Window::activate_window()`,
  and asserts the row appears. If the headless platform does not deliver the
  activation event, the same test calls the `refresh_from_disk` helper and
  asserts the observer is registered; the fallback is named in the test so a
  later platform fix can tighten it.

**Touches.** `src/theme.rs`, `src/ui/notes.rs`, `src/main.rs`,
`tests/notes_probe.rs`, new `tests/theme_probe.rs`,
warp-suite's `docs/agents/design-system.md` (the accent status).

### Step 2 — Store mutations and Trash

**Do.**

- Add a `trash` module and extend `notes_store` with `rename_entry`,
  `move_entry` and `duplicate_note`. `trash` owns `trash_entry`,
  `restore_entry`, `list_trash` and `purge_trash`, so the two modules do not
  call each other in a cycle.
- Two roots are explicit. Rename, move, duplicate, new folder and the tree
  live under the **scope root** (`<store>` or `<store>/projects/<repo>`), so
  a repo-scoped window cannot move a note into another repo. Trash lives
  under the **store root**, so a `--repo` window can still delete and restore
  an entry without the containment check rejecting the trash folder. The
  functions take the root they need; no function guesses.
- Rename keeps the `.md` extension for notes; folders keep none. Refuse an
  existing target. Handle the Windows case-only rename (`note.md` →
  `Note.md`) through a temporary name.
- Move refuses to move a folder into itself or its descendants, and refuses
  to overwrite.
- Duplicate creates `<stem> copy.md`, then `<stem> copy 2.md`; it returns the
  new path and never touches the original.
- Trash layout is deliberately bare:
  `<store>/.warp-notes/trash/<deleted-epoch-ms>-<4hex>/` with a `path` text
  file (the store-relative original, forward slashes) and a `payload` entry
  (the moved file or folder). The id is the timestamp plus entropy (retry on
  collision). There is no JSON manifest or schema to keep in sync; a missing
  payload is not listed rather than guessed at.
- Restore returns to the original relative path; if something is already
  there, restore as `<stem> (restored).md` (or `folder (restored)`) and say
  so. Missing parent folders are recreated.
- Purge entries older than 30 days on startup by parsing the id prefix; an
  unparseable id is left alone rather than deleted.
- All new functions reuse the existing containment check and have unit
  tests; the helper moves to a crate-visible function both modules use. No
  UI in this step.

**Acceptance.**

- `cargo test -p warp-notes notes_store::tests` passes, including
  `rename_refuses_overwrite_and_escape`,
  `move_refuses_folder_into_itself`, `duplicate_names_are_unique`,
  `trash_restore_round_trip` and `purge_keeps_recent_entries`.
- `cargo test -p warp-notes trash::tests` covers a repo-scoped window
  trashing into the store root (`trash_outside_scope_root_still_works`) and
  restoring a path whose parent folder was deleted.
- The suite's Windows type-check recipe (AGENTS.md → Windows type-check)
  type-checks the path handling; the real proof stays the windows-latest
  release job.

**Touches.** `src/notes_store.rs`, new `src/trash.rs`, `src/lib.rs`,
`src/ui/notes.rs` call sites only where signatures move, `README.md` and
`AGENTS.md` (the store contract and the hidden `.warp-notes` folder).

### Step 3 — Tree actions: rename, move, duplicate, new folder

**Do.**

- Add one generic prompt overlay (`PromptKind::{Rename, NewFolder, MoveTo}`)
  on the kit field, with Enter/Escape, inline validation and probe selectors.
- Every tree row gets a trailing `…` action button (visible on hover/focus)
  and a right-click menu with: Open, Rename, Duplicate, Move to folder…,
  New note here, New folder…, Copy path, Move to Trash.
- `Ctrl+N` and the `+` button keep the existing blank-note form; the new
  folder action wires the already-present `create_folder`.
- After a rename or move, update every path-bearing piece of state:
  `open_note`, `dirty`/`conflicts` keys, `prefs.open_note`, `prefs.expanded`,
  the tree cursor and the editor's `base_dir`. Put the prefix rewrite in one
  helper so Steps 6 and 8 can add pins/recent and tabs to it without a second
  copy. A folder move rewrites the prefix once. Refresh `saved_meta` from the
  new path so the next autosave compares the right baseline.
- Duplicating a note flushes/saves the source first, then opens the copy and
  expands its folder. Moving a note warns that relative image links stay in
  the old folder; moving the assets is a follow-up.
- Copy path writes the store-relative path and shows a toast.
- Projection: `note_names_stale` is set so wikilinks in the editor follow the
  new names. Links that point at the old name are not rewritten; the toast
  tells the user.
- The prompt is an occluding overlay with a scrim, Escape cancels, focus
  returns to the tree, and a click outside cancels. Validation is inline; no
  `unwrap` in the render path.

**Acceptance.**

- New `tests/notes_actions_probe.rs`:
  `rename_updates_disk_editor_and_prefs`,
  `move_note_and_folder_updates_open_path`,
  `duplicate_opens_a_unique_copy`, `new_folder_is_created_and_expanded` and
  `copy_path_writes_the_store_relative_path`.
- `repo_scope_refuses_a_move_outside_the_repo` launches with a `--repo` pin
  and asserts the target stays under `projects/<repo>`.
- The probe asserts the on-disk tree and the editor path, not only a toast.
- The context menu and prompt are reachable by `debug_selector`; the probe
  clicks them instead of calling private methods.

**Touches.** `src/ui/notes.rs` (menu/prompt), `src/notes_store.rs`,
`src/ui_prefs.rs`, `src/main.rs` (new-folder context),
`tests/notes_actions_probe.rs`, app README (the file actions and the asset
caveat).

### Step 4 — Trash UI, Undo and the Trash dialog

**Do.**

- Replace the two-click delete arm with a single **Move to Trash** action.
  The toast gains one optional action button; for trash it is **Undo** and
  lives for eight seconds.
- Closing a trashed note flushes its buffer first; if the buffer conflicts or
  cannot be saved, the recovery copy is written before the file moves, so
  the trash action never eats text.
- A folder trash closes any open note inside it.
- Add a small Trash dialog (visible from Settings → General and from the tree
  menu) listing original path, deleted age, Restore and Empty Trash. Restore
  puts the entry back, refreshes the tree and expands to it.
- Purge runs on startup; the dialog shows the count that will go.
- No permanent delete outside Empty Trash.
- The dialog lists every store entry, even when the tree is repo-scoped.
  Restoring an entry outside the current scope drops the repo pin and expands
  the whole store to it, the same rule search uses.
- Both the toast and the dialog are occluding overlays: Escape closes,
  click-outside cancels, focus returns to the tree.

**Acceptance.**

- `tests/notes_actions_probe.rs`:
  `trash_undo_restores_the_note`, `trashing_the_open_note_closes_it_and_keeps_its_text`,
  `trash_dialog_restores_and_empties`, and
  `trashing_a_folder_closes_a_note_inside_it`.
- Test seeds a dirty open note, trashes it, and asserts the untouched text is
  in the trash payload (or recovery file) after the action.

**Touches.** `src/ui/notes.rs`, `src/ui/settings.rs`, `src/trash.rs`,
`tests/notes_actions_probe.rs`, app README (Trash).

### Step 5 — Keyboard tree and search selection

**Do.**

- Give the tree a focus handle and a `tree_cursor` independent of
  `open_note`. Clicking a row focuses the tree and sets the cursor; opening a
  note still moves focus to the editor.
- `Ctrl+Shift+E` focuses the tree from anywhere. While the tree has focus:
  Up/Down move the cursor through visible rows, Left/Right collapse/expand,
  Enter opens or toggles, Home/End jump, F2 renames, Delete trashes,
  Ctrl+D duplicates and Shift+F10 opens the row menu.
- Draw a visible focus ring/cursor row; the open note keeps the accent bar.
- The tree pane is a focusable list with the existing tree-item roles; the
  cursor row reports selected/expanded state to a11y.
- The search popup gets a selected index: Up/Down move it, Enter opens,
  Escape clears, and the selected row has the same raised treatment as a
  selected list row. Reset the index when the result set changes.
- Stop the search field's input from consuming the popup keys; verify against
  the real input, not a mocked one.
- Add a small `Ctrl+/` cheat sheet: one occluding card with the keyboard map
  above, grouped by tree, tabs and editor. It is a reference, not a command
  palette.

**Acceptance.**

- `notes_probe::keyboard_tree_moves_and_opens` seeds a nested tree, focuses
  the tree, sends `down down right down enter`, and asserts the opened note
  and the cursor.
- `notes_probe::search_selects_with_arrows` types a term, sends
  `down enter`, and asserts the second hit opened.
- `notes_probe::f2_and_delete_operate_on_the_cursor`, not only on the open
  note.
- `notes_probe::shortcut_sheet_lists_the_tree_keys` opens with `Ctrl+/` and
  asserts the tree group and the F2 row.

**Touches.** `src/ui/notes.rs`, `src/main.rs` (tree context),
`tests/notes_probe.rs`.

### Step 6 — Quick open, recent and pins

**Do.**

- Add one palette overlay: filter field, virtualized rows, keyboard
  selection and a footer hint. It is a command surface, not a second
  select-style picker.
- `Ctrl+P` opens quick open over **all** notes in the store, regardless of
  the tree scope. Match on title and store-relative path with a small
  subsequence scorer (segment starts rank above mid-word matches). Empty
  query shows Recent, then Pinned.
- The note list comes from `notes_store::load_tree(&store_root)` flattened,
  not a new walker: that is the one place that already skips dot directories,
  so `.warp-notes` cannot leak into quick open.
- Persist `prefs.recent` (store-relative, newest first, cap 20) on every
  open. Persist `prefs.pinned`; add Pin/Unpin to the row menu and a star on
  the row. A pinned section sits above the tree, filtered to the current
  scope.
- Prune missing entries when loading the prefs or refreshing the tree; a
  pin/recent that was renamed or moved goes through the Step 3 path-rewrite
  helper.
- Normalize stored relative paths to forward slashes and cap the lists on
  load, so a hand-edited `settings.json` cannot make the quick-open rows
  unbounded.
- Opening a note outside the current `--repo` pin drops the pin and expands
  the whole store to that note, with a one-line toast. This keeps the tree
  and the open note agreeing.
- `Ctrl+Enter` opens the highlighted note in a new tab once Step 8 lands; in
  the meantime it is the same as Enter.
- The palette is an occluding overlay: Escape or a click outside dismisses,
  Up/Down/Enter stay in the overlay, and focus returns to the editor or tree
  that opened it.

**Acceptance.**

- New `tests/notes_palette_probe.rs`:
  `quick_open_filters_by_path_and_title`, `recent_order_survives_relaunch`,
  `pinned_notes_render_above_the_tree`, `pin_and_unpin_update_the_prefs`,
  `opening_an_outside_note_drops_the_repo_pin` and
  `quick_open_skips_the_hidden_app_folder`.
- Unit tests for the scorer: exact prefix, segment boundary, case-insensitive
  and no-match.

**Touches.** `src/ui/notes.rs` (or new `src/ui/palette.rs`), `src/ui_prefs.rs`,
`src/main.rs` (palette context), `tests/notes_palette_probe.rs`,
warp-suite's `docs/agents/settings-schema.md` (the new `pinned`/`recent` keys).

### Step 7 — Store-wide search and full results

**Do.**

- Make the title-bar search store-wide by default. Add a scope chip next to
  the field: **All notes** or the pinned repo name; clicking it restricts
  search to the tree scope. The placeholder and the chip stop disagreeing.
- Put the chip inside the field's trailing edge or the popup header, not
  beside the 460px field: the titlebar's centred search must keep its width.
- Add a full results view opened by `Ctrl+Shift+F` or the popup's
  "See all N results" footer. Reuse the palette shell from Step 6; rows show
  title, relative path and an excerpt with the term marked. Cap at the
  existing 2000-file search budget and show the count.
- Keep the title-bar popup as the glance (eight rows); the full view is the
  way to everything.
- Opening a result outside the tree scope follows the same "drop the pin,
  expand the store" rule as quick open.
- The scope choice is per window and not persisted, so two windows with
  different pins do not fight over one preference key.
- If this step has to shrink, land store-wide search + arrow-select first and
  keep the full results view as the follow-up; the popup fix is the part that
  removes the wrong-scope bug.

**Acceptance.**

- `tests/notes_palette_probe.rs`:
  `search_finds_notes_in_other_repos`, `scope_chip_narrows_to_the_pinned_repo`,
  `full_results_lists_more_than_the_popup_cap` and
  `highlight_marks_the_matched_span`.
- A 600-note fixture returns the expected hits after the awaited background
  task; the search remains debounced and off-thread.

**Touches.** `src/ui/notes.rs`, `src/ui/palette.rs` (if split),
`src/main.rs` (search context), `tests/notes_palette_probe.rs`, app README
(search scope).

### Step 8 — Tabs

**Do.**

- Add `open_notes: Vec<OpenTab>` and an active index per window. An
  `OpenTab` carries `path`, `saved_text`, `saved_meta` and `conflict`; the
  existing single-note fields move into the active tab. `open_note` becomes
  `open_in_tab(path, TabMode::{Current, New})`; the tree, quick open, search
  and wikilinks open in the current tab by default. Opening a note that is
  already open activates its tab instead of duplicating it.
- Add a back/forward history over the open sequence (`Alt+Left` /
  `Alt+Right`), the gap the old single editor left when a wikilink or a
  search hit replaced the current note. A history entry activates the note's
  tab when it is already open; otherwise it opens in the current tab.
- Render a tab strip above the editor: title, dirty dot, close button,
  middle-click close, overflow scroll. The editor head keeps the path,
  save/conflict state, format bar and the destructive action; the duplicate
  title moves to the tab.
- Switching saves the outgoing note (the existing `open_note` path); a
  failed save parks the buffer in `dirty` as it does now. Closing a tab with
  a conflict parks a recovery copy and says so.
- `Ctrl+W`, `Ctrl+Tab`, `Ctrl+Shift+Tab` and `Ctrl+1..9` work. Closing the
  last tab returns to the empty state.
- Switching to a tab compares the disk baseline: a clean tab reloads an
  external edit, a dirty tab shows the conflict bar. Rename/move/trash (Steps
  3–4) must update every affected tab through the same path-rewrite helper,
  and trashing a folder closes every tab inside it. The window title follows
  the active tab.
- Tabs are per-window and are **not** persisted (see D1); `prefs.open_note`
  keeps tracking the active tab so a bare relaunch still reopens one note.
- Window close flushes every parked buffer, including tabs that were
  switched away from; `flush_saves` already covers `dirty`, but the active
  tab's text must be captured before the window goes. Register
  `Window::on_window_should_close` for the OS close path, not only the
  titlebar button and `on_app_quit`.
- The tab strip uses the family's tab roles, a dirty dot, a close button and
  horizontal scrolling; the `+` button opens quick open.

**Acceptance.**

- New `tests/notes_tabs_probe.rs`:
  `three_tabs_keep_their_buffers`, `closing_the_active_tab_activates_a_neighbor`,
  `dirty_dots_follow_each_tab`, `window_close_saves_every_tab` and
  `relaunch_restores_only_the_active_note`.
- The probe edits each tab, switches twice, asserts each editor value, then
  waits out autosave and reads all three files from disk.
- `rename_and_move_update_every_open_tab` covers the path-bearing tab state.
- `back_forward_revisits_the_previous_note` opens A, B and C, walks back
  twice, forward once and asserts the editor paths.

**Touches.** `src/ui/notes.rs`, `src/ui_prefs.rs`, `src/main.rs` (tab
context), `tests/notes_tabs_probe.rs`.

### Step 9 — Context panel: outline and backlinks

**Do.**

- Add a collapsible right-hand panel with two tabs: **Outline** and
  **Backlinks**. `Ctrl+Shift+B` and a title-bar button toggle it; state is
  per window.
- Outline: reuse `warpui_kit_markdown::live::parse_document` — headings are
  `LiveLineKind::Heading` (ATX and setext), not a second markdown parser.
  Cache by editor revision, indent by level. Clicking a heading moves the
  editor caret to its byte offset and scrolls it into view.
- Backlinks: parse each note with the same `parse_document` and collect
  `InlineObjectKind::WikiLink` targets, normalized with the editor's own
  `note_key` (folders, `#heading`, `^block`, `.md` and case are handled).
  `note_key` is `pub(crate)` in `warpui-kit-markdown` today, so this step
  exposes it as a small public helper and uses it in both the editor and the
  app — one link normalization, not a second one. The file list comes from
  `notes_store::load_tree` (the same dot-directory-safe walk), reads happen
  off the UI thread, the index is cached by `(path, mtime)`, and it rebuilds
  after a save or a refresh. Show a count; clicking a backlink opens the
  source note.
- The panel's read-only excerpts get the family right-click Copy menu
  (`warpui_kit_core::text_menu::install` + `gpui_kit::base::TextSelectionLayer`,
  rendered once at the root), with each excerpt wrapped in
  `text_menu::attach`. The live editor keeps its own cut/copy/paste menu.
- The panel has a fixed narrow width, truncates paths and never pushes the
  editor below the detail pane's minimum. Hiding it gives the width back.

**Acceptance.**

- New `tests/notes_context_probe.rs`:
  `outline_lists_headings_and_jumps_to_one`,
  `backlinks_lists_sources_for_a_wikilink`,
  `backlink_excerpt_gets_the_copy_menu` (a direct menu-open assertion, or a
  probe of the installed host when the menu cannot be driven headlessly) and
  `backlinks_skip_the_hidden_app_folder`.
- A store of 300 linked notes waits for the background index and asserts the
  expected inbound links; the panel renders its loading state before the
  index lands.

**Touches.** `src/ui/notes.rs` (or new `src/ui/context.rs`),
`src/notes_store.rs`/new `src/links.rs`, `src/main.rs` (context-panel
context), `tests/notes_context_probe.rs`,
warp-suite's `kits/warpui-kit-markdown` (the public `note_key` helper and its
test), app README/AGENTS (the panel).

### Step 10 — Templates and the daily note

**Do.**

- Templates live in `<store>/.warp-notes/templates/*.md` (hidden, skipped by
  the tree and search). "New from template…" in the tree menu lists them; the
  new note is created in the current folder and starts with the template
  body. Substitute `{{date}}`, `{{time}}` and `{{title}}`.
- "Today's note" in the `+` menu opens or creates
  `<current scope>/journal/YYYY-MM-DD.md` (`<store>/journal` at the whole
  store, `<store>/projects/<repo>/journal` when pinned). If
  `.warp-notes/templates/daily.md` exists, use it; otherwise start with
  `# YYYY-MM-DD`. Opening an existing daily note never overwrites it.
- Keep the path logic pure (`daily_path(date)` / `render_template`) so it is
  unit-tested without the clock.
- Document the two hidden folders in the app README and AGENTS; no launch
  argument changes.

**Acceptance.**

- `templates` unit tests:
  `daily_path_is_stable_and_date_based`, `render_template_replaces_placeholders`.
- `notes_probe::new_from_template_uses_the_template_body` and
  `todays_note_creates_then_reopens`; the second click must not rewrite the
  file.
- `cargo test -p warp-notes` plus the suite's Windows type-check recipe for
  the path work.

**Touches.** `src/notes_store.rs`/new `src/templates.rs`,
`src/ui/notes.rs`, `src/ui_prefs.rs` if a folder setting is added,
`README.md`, `AGENTS.md`, warp-suite's `docs/agents/settings-schema.md`.

## 6. Test and gate matrix

| Area | Test |
| --- | --- |
| Store CRUD, rename/move/duplicate, containment | unit tests in `notes_store.rs` |
| Trash layout, restore, purge | unit tests in `trash.rs` |
| Chrome, refresh, title | `tests/notes_probe.rs`, `tests/theme_probe.rs` |
| Tree actions and prompts | `tests/notes_actions_probe.rs` |
| Keyboard tree and search popup | `tests/notes_probe.rs` |
| Quick open, recent, pins, full search | `tests/notes_palette_probe.rs` + scorer unit tests |
| Tabs and save-on-close | `tests/notes_tabs_probe.rs` |
| Outline, backlinks, text menu | `tests/notes_context_probe.rs` |
| Templates, daily note | unit tests + `tests/notes_probe.rs` |

Every step also runs the package gate:

```sh
cargo fmt --all --check
cargo clippy --locked --all-targets -p warp-notes -- -D warnings
cargo test -p warp-notes
```

The backlink step also runs `cargo test -p warpui-kit-markdown` and the kit
clippy selector, because it exposes one helper from the editor's private
module.

For Steps 2–4 and 10, also run the suite's Windows type-check recipe
(AGENTS.md → Windows type-check). The real proof stays the windows-latest
release job.

The probes use a scratch `HOME` and serialize on the existing process-global
locks; the palette ramp test restores Copper and takes the same turn as the
other global-state probes.

## 7. Decisions

- **D1 — Tabs are per window, not persisted.** Only the active note is
  remembered (today's `prefs.open_note`), so a launch with `--note` or
  `--repo` does not restore a stale tab set into another window. Pins and
  recent are app-level and last-writer-wins across windows, like the existing
  layout fields; tabs are not, which avoids making that worse. A
  session-restore feature can come later with per-window IDs.
- **D2 — Trash lives inside the store** at
  `<store>/.warp-notes/trash/<deleted-epoch-ms>-<4hex>/{path,payload}`,
  30-day purge. No JSON manifest: the id is the deletion time and `path` is
  the original. It travels with the vault and is invisible to the tree, the
  shared search and Obsidian's own `.trash`. The alternative
  (`~/.warp/notes/trash`) was rejected because a store on another machine
  would lose its restores; a bare `<store>/.trash` was rejected because
  Obsidian owns that name.
- **D3 — Delete is immediate + Undo, not a confirmation dialog.** The note
  goes to Trash, the toast offers Undo for eight seconds, and the Trash
  dialog is the recovery surface. The old two-click arm disappears.
- **D4 — Search is store-wide by default.** The scope chip restricts it. A
  hit outside the tree scope drops the repo pin so the tree and the open
  note agree. This fixes the "Search all notes…" label for good.
- **D5 — One editor, tabs park buffers.** Switching tabs saves or parks the
  outgoing buffer. `LiveEditorState::set_value` resets the undo stack and the
  scroll position, so v1 promises the text and the path, not the undo/scroll
  state; a later multiple-editor change can restore both.
- **D6 — No new external crate.** Reuse `warpui-kit-core` field/menu/
  text-menu, the workspace's `warpui-kit-picker` where a select list helps,
  and a small local fuzzy scorer. Do not use the picker for quick open or
  full results: it handles only Escape and its data source is synchronous,
  so it cannot carry the keyboard-first palette; it is only an option for a
  mouse Move-to-folder/template list. The tree, search and tabs stay in the
  app.
- **D7 — Do not rewrite wikilinks on rename in v1.** The rename toast says
  links are untouched. A link-maintenance pass (find `[[old]]`, offer a
  preview, rewrite exact matches) is the follow-up once backlinks exist.
- **D8 — Deferred:** desktop notification mirror (needs the station service
  in a kit crate), tags + tag filtering (the live parser already has tag
  spans), sort by modified/recent, drag/drop, note history, export/print,
  AI actions, store-path editing, open-in-external-editor, and orphan-asset
  cleanup.

## 8. Sequence

- **First slice (Steps 1–4):** the app becomes safe to manage — rename,
  move, duplicate, folders, trash, undo, refresh, live accent. This is the
  smallest release that removes the "I have to open a file manager" feeling.
- **Second slice (Steps 5–7):** the app becomes fast to navigate — keyboard
  tree, quick open, recent, pins, store-wide search and full results.
- **Third slice (Steps 8–10):** the app becomes a workspace — tabs, context
  panel, templates and daily note.

Do not start Step 8 before Steps 1–4 land: tabs multiply the path-bearing
state that rename/move/trash must keep coherent. Do not start Step 9 before
Step 5: the panel's keyboard surfaces reuse the tree cursor and palette
selection model.

When the steps land, fold the user-facing changes into the app README and
AGENTS, update the settings schema, and delete this plan.
