# warp-notes

Per-repo markdown notes with an Obsidian-style live editor — the Notes app
of the [warp](https://github.com/enginkirmaci/warp-suite) family, extracted
from DevTools into its own repo. Rust/GPUI (gpui-kit 0.7), one crate.

## What it does

- One notes store on disk: `~/notes`, one subfolder per repository. A store
  an older release kept in `~/.warp/notes` (or the even older
  `~/.devtools/notes`) moves there on first run.
- The family shell: a rail (Notes, Settings), the note tree in the list pane and
  one live editor in the detail pane; Settings holds the store path, the shared
  text size, the Copper palette/typeface and reduce motion (the list width and
  the motion choice persist in `~/.warp/notes/settings.json`).
- Folder/file tree, capped full-text search, dirty-buffer-safe navigation.
- A single live editor that works like Obsidian's live preview: markdown
  renders in place while typing with its syntax hidden until the caret touches
  it, checkboxes toggle on click, Enter continues lists, links and
  `[[wikilinks]]` open on click, Ctrl+F finds; frontmatter is a Properties
  panel, Mermaid blocks render as diagrams, tables grow from their add strips
  and images sit inside sentences; colors come from the app's accent preset.
  The engine and the toolbar's pure markdown ops are shared with the family
  via [warpui-kit-markdown](../warpui-kit-markdown).
- Clipboard image paste becomes `assets/paste-<stamp>.<ext>` next to the
  note plus a relative markdown ref.

## Launch contract

```
warp-notes [--store <path>] [--repo <name>] [--note <path>]
```

- `--store` — the notes store root; default `~/notes`. DevTools passes
  its resolved `notes_store_path` setting.
- `--repo` — pin the tree to one repository's notes folder (the titlebar's
  up button climbs back to the whole store).
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
