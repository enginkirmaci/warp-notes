# warp-notes — agent instructions

Read [`../warp-suite/AGENTS.md`](../warp-suite/AGENTS.md) first. This file adds
only the app-specific facts. The user-facing description is in
[`README.md`](README.md).

Design plans live in `docs/plans/`: a plan is deleted when its work lands, and the
rules that still matter move into `README.md` or this file.

## Shape

One crate, `warp-notes`, with a real library boundary: `src/lib.rs` exposes
the modules and the binary is a thin bootstrap. The probes build the real
`NotesPage` through that boundary.

## Where things live

- `src/main.rs` — the launch parser, startup order and the one window.
- `src/notes_store.rs` — the per-repo tree, CRUD, name sanitizing, capped
  search and the `--note` path containment.
- `src/paths.rs` — `~/.warp/notes` for settings, `~/.warp/logs` for logs,
  `~/notes` for the shared store, and the legacy-layout migration.
- `src/ui/notes.rs` — the rail, the strip under the titlebar, the list head
  (counts and the All/Pinned scope), the tree and folder bands, row
  actions/prompts, the note head, live editor and find bar.
- `src/ui/settings.rs` — General and Appearance: store path, shared text size,
  palette and reduce motion.
- `src/ui_prefs.rs` — the remembered shell in the versioned snake_case
  `~/.warp/notes/settings.json`.
- `src/atomic_write.rs`, `logging.rs`, `assets.rs`, `theme.rs`.

## Launch contract

```text
warp-notes [--store <path>] [--repo <name>] [--root] [--note <path>]
```

- `--store` — the notes store root; default `~/notes`.
- `--repo` — pin the tree to one repository's notes folder; the titlebar's up
  button climbs back to the whole store.
- `--root` — open the whole store outright, beating both `--repo` and the
  scope the last window remembered (Warp Station's titlebar sends it, so the
  notes root opens whatever repo is selected there).
- `--note` — open that note right away (absolute or store-relative).
- Unknown args are ignored so a stale desktop shortcut never blocks the
  window.
- A `--note` path must stay inside the store; relative paths may not traverse
  with `..`.
- Multi-instance is free: every launch opens its own window, and the window
  outlives Warp Station.

## Store and migrations

- The store is user content, default `~/notes` (`--store` /
  `notes_store_path` can point it anywhere, an Obsidian vault for one user).
  One repository per folder under `<store>/projects/`; notes that belong to no
  repo live at the store root. It is shared with Warp Station through
  `warpui_kit_platform::notes`; keep the on-disk format compatible.
- A standalone launch uses `UiPrefs.store_path` when set; Settings edits the
  path and applies it immediately (flush the old tabs, clear them, reload the
  tree). `--store` wins and makes that row read-only, because the next Warp
  Station launch would pass its setting again.
- Trash lives at `<store>/.warp-notes/trash/<deleted-ms>-<4hex>/`: a `path`
  text file holds the store-relative original and `payload` is the moved
  file or folder. Trash is store-root-scoped even when the tree is pinned to
  one repo. The folder is hidden and skipped by every walker; entries older
  than 30 days are purged on launch.
- `paths::migrate_legacy_layout()` runs before logging. It absorbs
  `~/.warp-notes`, moves a store kept in `~/.warp/notes` or
  `~/.devtools/notes` into `~/notes/projects/`, and marks the scan one-shot. Name
  collisions keep both sides and leave the marker unwritten so the next start
  retries.
- `AppPaths::config_dir()` is the app's settings folder, **not** the store.
  The store is `paths::notes_store_root()`.

## Tests

```sh
cargo test -p warp-notes
cargo build -p warp-notes
```

### Build output

`target/` is a **symlink** to the suite's shared build cache
(`~/.cache/warp-suite/target`), so a standalone `cargo build` here and a build
through `warp-suite/apps/<app>` reuse one dependency tree instead of compiling
the gpui graph twice. Two consequences:

- `[profile.dev] debug = "line-tables-only"` in this app's `Cargo.toml` must
  stay identical to warp-suite's root `[profile.dev]`. Cargo keys a crate's
  fingerprint on the resolved profile, so a different setting makes every
  shared dependency compile a second time. Set `debug = 2` temporarily only
  when a session needs gdb/lldb variable inspection.
- `cargo clean` here removes the **symlink**, not the cache: the next build
  then creates a real `target/` on the slow disk. Re-link with
  `ln -s ~/.cache/warp-suite/target target` (or run
  `warp-suite/scripts/dev/link-apps.sh`, which manages these links).

`.vscode/` carries the F5 setup for this checkout: a `cargo build` task and a
gdb (Linux) / cppvsdbg (Windows) launch config that reads the binary from
`target/debug/`.

`.cargo/config.toml` (gitignored, machine-local) points the warpui-kit crates
at the sibling `warp-suite/kits/` checkout with cargo's `paths` override, so a
build here uses the same kit sources as a build through
`warp-suite/apps/<app>` and the two share the compiled kit artifacts. Kit
breakpoints resolve to `kits/`, not to a `~/.cargo/git/checkouts` copy.
`Cargo.toml` and `Cargo.lock` keep the pinned tag, so CI's `--locked` build is
unaffected; delete the file to build against the tag again.

- `tests/notes_probe.rs` covers the real window: tree, opening a note, the
  live editor, row actions, keyboard tree/search selection, Trash/Undo and
  the Trash dialog, quick open, recent/pinned notes, store-wide and full
  search, tabs/back-forward, the Outline/Backlinks panel, task checkboxes,
  list continuation, wikilinks, and the `--repo`/`--root`/`--note` launch
  contract.
- `tests/layout_migration.rs` drives the scratch-HOME migration end to end.
  HOME is process-global, so scenarios take turns behind its lock.
- The tree's cursor belongs to `tree_focus`: arrows scroll it into view and
  its raised emphasis only shows while the tree owns the keys. A field or the
  editor keeps its own arrows; do not add a window-level tree key handler.
- The keyboard sheet is `warpui_kit_core::shortcuts::dialog`; `Ctrl+/` opens
  it and `?` is the alias while no text field or the editor owns the keys. The
  card takes the focus and every close returns it to `page_focus`.
- `warpui_kit_core::pane_nav` carries the two panes: the tree's `Tab` focuses
  the editor, `Shift+Tab` is a no-op at the row start, and the editor keeps
  `Tab`/`Shift+Tab` for indent/outdent — `Ctrl+Shift+E` stays the way back.
- The capture hooks are `WARP_NOTES_PAGE=settings|about` and
  `WARP_NOTES_SHORTCUTS=1`, which opens the keyboard sheet on launch.

## App-specific contracts

- Warp Station launches this app with `--store` (its `notes_store_path`
  setting) plus `--root`/`--repo`/`--note`; its titlebar asks for the root and
  a repo's launcher pins that folder. A customized store must follow.
- A missing binary makes Warp Station's notes surfaces show a toast; nothing
  else breaks.
- Text size and palette come from `~/.warp/ui.json` through
  `warpui-kit-core`; this app stores neither. The accent follows the live
  ramp through `current_ramp().accent_colors()`; do not add a second accent
  picker.
- Reduce motion is local to this app in `UiPrefs`, not in `ui.json`.

## Do not

- Make the app's settings folder the notes store, or point the store at
  `~/.warp/notes`; that folder is the app's data now.
- Add a single-instance guard; multi-instance is the contract.
- Add a second editor or markdown renderer; the live editor is
  `warpui-kit-markdown`.
- Add a permanent delete path outside the Trash dialog; removal goes through
  `<store>/.warp-notes/trash` first.
- Add a per-app build script or installer; the suite's `scripts/` owns those.

## Public history and secrets

- Never commit a token, PAT, password, private IP, Wi-Fi SSID, home path or
  personal email in file content. Use placeholders in docs and examples.
- Pass credentials through GitHub Actions secrets only; never echo or log
  them.
- Before publishing, force-pushing or adding a remote, run this over the
  current tree and every ref:

  ```sh
  git grep -n -i -E '192\.168\.[0-9]+\.[0-9]+|10\.[0-9]+\.[0-9]+\.[0-9]+|/home/[a-z]+|ghp_|gho_|github_pat_|ghs_' -- .
  ```
