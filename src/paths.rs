//! Global per-user locations: rolling logs under ~/.warp-notes, the notes
//! store at ~/.warp/notes — the shared store every warp app reads. On first
//! run a legacy ~/.devtools/notes store is renamed over, so existing notes
//! survive the move.

use std::path::PathBuf;

pub struct AppPaths;

impl AppPaths {
    pub fn config_dir() -> PathBuf {
        home().join(".warp-notes")
    }

    pub fn logs_dir() -> PathBuf {
        Self::config_dir().join("logs")
    }
}

/// The shared notes store: one subfolder per repository.
pub fn default_store_root() -> PathBuf {
    home().join(".warp").join("notes")
}

/// One-shot migration of the legacy DevTools store: `~/.devtools/notes` →
/// `~/.warp/notes` by rename. "Landed" means the target holds content; an
/// empty target is a leftover from a failed attempt (either app creates the
/// empty store after a failed rename), so it is dropped and the rename
/// retried — otherwise one early failure would strand the notes in
/// ~/.devtools forever. A rename that still fails leaves both sides intact.
pub fn migrate_legacy_store() {
    let legacy = home().join(".devtools").join("notes");
    if !legacy.is_dir() {
        return;
    }
    let target = default_store_root();
    if target.is_dir()
        && std::fs::read_dir(&target).is_ok_and(|mut entries| entries.next().is_some())
    {
        return;
    }
    if target.is_dir()
        && let Err(err) = std::fs::remove_dir(&target)
    {
        tracing::warn!(
            "could not clear the empty store {}: {err}",
            target.display()
        );
        return;
    }
    if let Some(parent) = target.parent()
        && let Err(err) = std::fs::create_dir_all(parent)
    {
        tracing::warn!("could not create {}: {err}", parent.display());
        return;
    }
    match std::fs::rename(&legacy, &target) {
        Ok(()) => tracing::info!(
            "migrated the notes store {} → {}",
            legacy.display(),
            target.display()
        ),
        Err(err) => tracing::warn!(
            "could not migrate the legacy notes store {}: {err}",
            legacy.display()
        ),
    }
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_store_is_under_home_warp() {
        // shape only: the shared store stays $HOME/.warp/notes on every OS
        assert!(
            default_store_root().ends_with(".warp/notes")
                || default_store_root().ends_with(r".warp\notes")
        );
    }
}
