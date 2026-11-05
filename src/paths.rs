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

/// The layout this build writes, adopted from the previous one: the logs move
/// into the shared folder, the legacy `~/.warp-notes` folder becomes the
/// app's settings folder, and note content still in `~/.warp/notes` or
/// `~/.devtools/notes` moves to `~/notes`. Every step is idempotent, so this
/// runs on every start.
pub fn migrate_legacy_layout() {
    // the store moves first: it frees the `~/.warp/notes` name for the app's
    // own folder adopted right after
    migrate_legacy_store();
    paths::adopt_legacy_logs(&legacy_dir().join("logs"));
    paths::adopt_legacy_dir(&legacy_dir(), &AppPaths::config_dir());
}

/// Move note content older builds kept in `~/.warp/notes` (and the even older
/// `~/.devtools/notes`) into the store at `~/notes`. A name collision keeps
/// both sides — notes are never overwritten or deleted.
pub fn migrate_legacy_store() {
    paths::adopt_notes_store(&paths::home().join(".devtools").join("notes"));
    paths::adopt_notes_store(&paths::suite_root().join("notes"));
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
