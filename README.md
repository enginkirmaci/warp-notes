# warp-notes

Per-repo markdown notes with an Obsidian-style live editor: one notes store on
disk, a folder per repository, and a single editor that renders markdown in
place while you type.

Part of the [warp suite](https://github.com/enginkirmaci/warp-suite). Rust and
GPUI.

## Install

The normal way is the suite bundle, which installs every warp app at once:

- **Linux** — download `warp-suite-<version>-linux.tar.gz` from the
  [suite release](https://github.com/enginkirmaci/warp-suite/releases), extract
  it, and run `./install.sh`. Binaries go to `~/.local/bin`, desktop entries
  and icons to `~/.local/share`; everything is per-user, no root needed.
- **Windows** — download `warp-suite-<version>-windows-x64.zip`, extract it,
  and run `install.cmd`. It installs under
  `%LOCALAPPDATA%\Programs\warp-suite` with a Start Menu folder. The same ZIP
  is portable: run the executables under `apps\` without installing.

warp-notes also ships on its own: the Linux tarball and the Windows portable
zip and installer are on
[this repository's releases](https://github.com/enginkirmaci/warp-notes/releases).

## Run

```sh
cargo run            # debug build
cargo test           # unit tests and headless UI probes
```

The first build fetches GPUI from the zed git repository and takes a while. It
needs the usual Wayland/Vulkan development packages.

## What it does

- **One notes store on disk**: `~/notes`, one repository per folder under
  `projects/`. A standalone window can point it anywhere from Settings; a
  `--store` launch, including Warp Station's, wins.
- **A live editor** that works like Obsidian's live preview: markdown renders
  in place with its syntax hidden until the caret touches it, checkboxes
  toggle on click, Enter continues lists, links and `[[wikilinks]]` open on
  click, and `Ctrl+F` finds. Frontmatter is a Properties panel, tables grow
  from their add strips, and images sit inside sentences.
- **A folder and file tree** with capped full-text search and dirty-buffer-safe
  navigation. A row's action menu renames, moves, duplicates, creates folders
  and copies store-relative paths.
- **Trash with Undo**: moving a note or folder to Trash is one action with an
  Undo toast. Entries stay for 30 days, and there is no permanent delete
  outside Empty Trash.
- **Keyboard-first**: `Ctrl+Shift+E` focuses the tree, `Ctrl+P` quick-opens any
  note, `Ctrl+Shift+F` opens the full search results, `Ctrl+Shift+B` toggles
  the outline and backlinks panel, and `Ctrl+/` opens the shortcut sheet.
- **Tabs** under the titlebar: a single click previews, a double click keeps,
  and a middle click opens a note in a tab of its own.

## Launch contract

```
warp-notes [--store <path>] [--repo <name>] [--root] [--note <path>]
```

| Flag | What it does |
| --- | --- |
| `--store` | the notes store root; defaults to `~/notes` or the path saved in Settings. Warp Station passes its resolved `notes_store_path` and wins over the local field. |
| `--repo` | pins the tree to one repository's folder under `projects/` |
| `--root` | opens the whole store outright, beating a `--repo` pin |
| `--note` | opens that note right away (absolute, or store-relative) |

Multi-instance is free: every launch opens its own window.

## Where it keeps its files

| Path | What |
| --- | --- |
| `~/notes` | the notes store — your content |
| `~/.warp/notes/settings.json` | the versioned settings |
| `~/.warp/logs/` | the suite's shared rolling logs |

An older store kept in `~/.warp/notes` (or the even older `~/.devtools/notes`)
moves to `~/notes` on first run.

## Siblings

| App | What it adds |
| --- | --- |
| [warp-station](https://github.com/enginkirmaci/warp-station) | the workbench that launches this app with `--store`/`--repo`/`--root`/`--note` |
| [warp-agent](https://github.com/enginkirmaci/warp-agent) | the chat client |
| [warp-pomodoro](https://github.com/enginkirmaci/warp-pomodoro) | the focus timer |
| [warp-flow](https://github.com/enginkirmaci/warp-flow) | the workflow engine |
| [warp-snapit](https://github.com/enginkirmaci/warp-snapit) | the Windows zone snapper |

## Build and package

```sh
cargo build --release
```

The `release` workflow packages this repository alone: on Linux it stages the
binary, the desktop entry and the hicolor icons into a tarball; on Windows it
writes the portable zip and the Inno Setup installer. The assets stay here
under `packaging/`.

## License

GPL-3.0. See [LICENSE](LICENSE).
