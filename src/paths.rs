//! Global per-user locations: the notes store is user content at `~/notes`
//! (one folder per repository, shared with Warp Station), the app's own
//! settings live with the other apps' data in `~/.warp/notes`, and the
//! rolling logs go to the suite's shared `~/.warp/logs`. On first run the
//! legacy `~/.warp-notes` folder moves in and a store still at
//! `~/.warp/notes` (or the older `~/.devtools/notes`) moves to `~/notes`, so
//! existing notes survive the move.

use std::path::PathBuf;
use warpui_kit_platform::paths;

pub struct AppPaths;

impl AppPaths {
    /// The app's own folder, `~/.warp/notes` — settings only; the store
    /// itself is `~/notes`.
    pub fn config_dir() -> PathBuf {
        paths::app_dir("notes")
    }

    /// The suite's shared logs folder, `~/.warp/logs`.
    pub fn logs_dir() -> PathBuf {
        paths::logs_dir()
    }
}

/// The shared notes store: one subfolder per repository, `~/notes`.
pub fn default_store_root() -> PathBuf {
    paths::notes_store_root()
}

/// The layout this build writes, adopted from the previous one: the legacy
/// `~/.warp-notes` folder becomes the app's settings folder (its old store
/// content included), the logs join the shared folder, and note content moves
/// to `~/notes`. Every step is idempotent; the store scan itself is one-shot
/// behind a marker, because `~/.warp/notes` now holds the app's settings.
pub fn migrate_legacy_layout() {
    // absorb the old app folder first: it may still hold the pre-suite store,
    // and the store pass must see those notes (in the app folder, or left in
    // ~/.warp-notes for name collisions) before it marks the job done
    paths::adopt_legacy_logs(&legacy_dir().join("logs"));
    paths::adopt_legacy_dir(&legacy_dir(), &AppPaths::config_dir());
    migrate_legacy_store();
}

/// Move note content older builds kept in `~/.warp/notes`, `~/.warp-notes` and
/// `~/.devtools/notes` into the store at `~/notes`. The old app folder and the
/// devtools folder are true store locations and are always absorbed; the suite
/// folder is the app's own folder now, so it is only scanned until the
/// one-shot marker says the migration ran (audit C2). A name collision keeps
/// both sides and leaves the marker unwritten so the next start retries.
pub fn migrate_legacy_store() {
    let devtools = paths::adopt_notes_store(&paths::home().join(".devtools").join("notes"));
    let legacy_app = paths::adopt_notes_store(&legacy_dir());
    if paths::notes_store_adopted() {
        return;
    }
    let suite = paths::adopt_notes_store(&paths::suite_root().join("notes"));
    if devtools && legacy_app && suite {
        paths::mark_notes_store_adopted();
    }
}

/// The folder this app owned before the suite layout: `~/.warp-notes`.
fn legacy_dir() -> PathBuf {
    paths::home().join(".warp-notes")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_store_is_home_notes_and_not_the_app_folder() {
        assert_eq!(default_store_root(), paths::home().join("notes"));
        assert_ne!(default_store_root(), AppPaths::config_dir());
        assert_eq!(AppPaths::config_dir(), paths::suite_root().join("notes"));
        assert_eq!(AppPaths::logs_dir(), paths::suite_root().join("logs"));
    }

    #[test]
    fn the_legacy_folder_keeps_its_old_name() {
        assert_eq!(legacy_dir(), paths::home().join(".warp-notes"));
    }
}
