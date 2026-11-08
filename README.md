# warp-notes

Per-repo markdown notes with an Obsidian-style live editor — the Notes app
of the [warp](https://github.com/enginkirmaci/warp-suite) family, extracted
from DevTools into its own repo. Rust/GPUI (gpui-kit 0.9), one crate.

## What it does

- One notes store on disk: `~/notes`, one repository per folder under
  `projects/`. A standalone window can point it anywhere from Settings
  (a `--store` launch, including Warp Station's, wins); an older store kept in
  `~/.warp/notes` (or the even older `~/.devtools/notes`) moves there on first
  run.
- The family shell, Warp Station's stack: a rail (Notes, Settings), the strip
  under the titlebar (a Notes tab for the list, then one per open note), the
  tree in the list pane and one live editor in the detail pane. Settings holds
  the store path (editable when the window was not launched with `--store`),
  the shared text size, the live palette and reduce motion (the list width and
  the motion choice persist in the versioned snake_case
  `~/.warp/notes/settings.json`).
- The list head reads the store: "N notes · M pinned" with the new-note and
  collapse controls, then an `All` / `Pinned` scope. The tree's folders are
  bands with their note counts; note rows carry the name at name size, the
  edited age, the pin and the row menu.
- The note head is the note's identity: the pin, the title at screen-title
  size, the store-relative path with the edit age, and a `…` menu for rename,
  move, duplicate, copy path and Trash. The text keeps its 760px measure under
  a full-width head.
- Folder/file tree, capped full-text search, dirty-buffer-safe navigation.
  A row's action menu (also on right-click) renames, moves, duplicates,
  creates folders and copies store-relative paths; the tree and the open note
  follow changes made outside the window on refresh or activation.
- Moving a note or folder to Trash is one action with an Undo toast; Settings
  → General opens the Trash dialog to restore or empty it. Entries stay for
  30 days, and there is no permanent delete outside Empty Trash.
- Keyboard-first tree: `Ctrl+Shift+E` focuses it, arrows move/collapse/expand,
  Enter opens, `F2` renames, `Delete` trashes and `Ctrl+D` duplicates. The
  title-bar search selects hits with Up/Down/Enter, and `Ctrl+/` opens the
  shortcut sheet.
- `Ctrl+P` quick open searches every note in the store by title and path;
  pinned and recent notes come first. The row menu and the note head's star
  pin/unpin, and the head's `Pinned` scope lists them flat. Opening a note
  from another repo drops the current repo pin so the tree can show it.
- The title-bar search is store-wide by default; its popup scope chip narrows
  to the pinned repo. `Ctrl+Shift+F` (or "See all results") opens the full
  result list with highlighted excerpts.
- Open notes appear in the strip under the titlebar: `Ctrl+Enter` in quick
  open or the row menu's "Open in new tab" adds one, `Ctrl+W` closes,
  `Ctrl+Tab` cycles, and `Alt+Left`/`Right` (or the titlebar arrows) walk the
  open history.
- The right-hand context panel toggles with `Ctrl+Shift+B`: Outline lists the
  current headings and jumps the caret, Backlinks lists the notes that link
  here and opens their source.
- A single live editor that works like Obsidian's live preview: markdown
  renders in place while typing with its syntax hidden until the caret touches
  it, checkboxes toggle on click, Enter continues lists, links and
  `[[wikilinks]]` open on click, Ctrl+F finds; frontmatter is a Properties
  panel, Mermaid blocks render as diagrams, tables grow from their add strips
  and images sit inside sentences; colors come from the app's accent preset.
  The engine and the toolbar's pure markdown ops are shared with the family
  via [warpui-kit-markdown](../warpui-kit-markdown).
- Clipboard image paste becomes `assets/paste-<stamp>.<ext>` next to the
  note plus a relative markdown ref. Moving a note leaves those relative
  assets in the old folder; move the assets with it when that matters.

## Launch contract

```
warp-notes [--store <path>] [--repo <name>] [--root] [--note <path>]
```

- `--store` — the notes store root; default `~/notes` or the path saved in
  Settings. Warp Station passes its resolved `notes_store_path` setting and
  wins over the local field.
- `--repo` — pin the tree to one repository's folder under `projects/` (the
  titlebar's up button climbs back to the whole store).
- `--root` — open the whole store outright, beating a `--repo` pin and the
  scope the last window remembered. Warp Station's titlebar sends it, so the
  notes root opens whatever repository is selected there.
- `--note` — open that note right away (absolute, or store-relative).

Multi-instance is free: every launch opens its own window.

## Build

```sh
cargo build --release
```

Packaging lives in the suite: `scripts/build.sh --app warp-notes` (Linux) and
`scripts/build.ps1 -App warp-notes` (Windows) in
[warp-suite](https://github.com/enginkirmaci/warp-suite). This directory only
carries the assets under `packaging/`.

## Sibling deps

`warpui-kit-core` (palette/fonts/icons/a11y), `warpui-kit-markdown` (the live
editor + toolbar ops) and `warpui-kit-diff` (code-block highlighting, through
the markdown crate) resolve as path deps next to this repo — clone them into
the same parent directory.
