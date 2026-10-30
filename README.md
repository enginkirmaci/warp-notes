# warp-notes

Per-repo markdown notes with an Obsidian-style live editor — the Notes app
of the [warp](https://github.com/enginkirmaci/warp-suite) family, extracted
from DevTools into its own repo. Rust/GPUI (gpui-kit 0.7), one crate.

## What it does

- One notes store on disk: `~/.warp/notes`, one subfolder per repository.
  A legacy `~/.devtools/notes` store is renamed over on first run.
- Folder/file tree, capped full-text search, dirty-buffer-safe navigation.
- A single live editor: markdown renders in place while typing, source
  markers fade off the caret line, task checkboxes toggle on click, Enter
  continues lists; the toolbar drives the pure markdown ops (shared with the
  family via [warpui-kit-markdown](../warpui-kit-markdown)).
- Clipboard image paste becomes `assets/paste-<stamp>.<ext>` next to the
  note plus a relative markdown ref.

## Launch contract

```
warp-notes [--store <path>] [--repo <name>] [--note <path>]
```

- `--store` — the notes store root; default `~/.warp/notes`. DevTools passes
  its resolved `notes_store_path` setting.
- `--repo` — pin the tree to one repository's notes folder (the titlebar's
  up button climbs back to the whole store).
- `--note` — open that note right away (absolute, or store-relative).

Multi-instance is free: every launch opens its own window.

## Build

```sh
cargo build --release
packaging/linux/install.sh        # build + desktop entry + hicolor icons
```

Windows: `packaging/windows/build.ps1` (add `-Installer` for the Inno Setup
installer). The suite ships both through
[warp-suite](https://github.com/enginkirmaci/warp-suite).

## Sibling deps

`warpui-kit-core` (palette/fonts/icons/a11y) and `warpui-kit-markdown` (the
live engine + toolbar ops) resolve as path deps next to this repo — clone
them into the same parent directory.
