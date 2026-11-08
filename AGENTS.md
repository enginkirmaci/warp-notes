# warp-notes — agent instructions

Read [`../../AGENTS.md`](../../AGENTS.md) first. This file adds only the
app-specific facts. The user-facing description is in
[`README.md`](README.md).

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
- `src/ui/notes.rs` — the rail, tree, live editor and find bar.
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

- `tests/notes_probe.rs` covers the real window: tree, opening a note, the
  live editor, task checkboxes, list continuation, wikilinks, and the
  `--repo`/`--root`/`--note` launch contract.
- `tests/layout_migration.rs` drives the scratch-HOME migration end to end.
  HOME is process-global, so scenarios take turns behind its lock.
- The capture hook is `WARP_NOTES_PAGE=settings`.

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
- Add a per-app build script or installer; the suite's `scripts/` owns those.
