//! The store's Trash: one hidden folder under the store root, no manifest.
//!
//! Layout:
//! `<store>/.warp-notes/trash/<deleted-epoch-ms>-<4hex>/path` holds the
//! original store-relative path as text, and `payload` is the moved file or
//! folder. The id carries the deletion time, so purge is a prefix parse and a
//! restore never guesses where the entry came from. The folder is hidden and
//! every walker skips dot directories, including Warp Station's search.
//!
//! Trash is intentionally store-root-scoped even when the window is pinned to
//! one repo: the containment check must accept `<store>/.warp-notes/trash`,
//! which is outside `<store>/projects/<repo>`.

use crate::notes_store::{ensure_under_root, paste_rand16};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const APP_DIR: &str = ".warp-notes";
const TRASH_DIR: &str = "trash";
const PATH_FILE: &str = "path";
const PAYLOAD: &str = "payload";

/// The app's Trash retention: an entry older than this is purged on launch.
pub const MAX_AGE: Duration = Duration::from_secs(30 * 24 * 60 * 60);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrashEntry {
    pub id: String,
    pub path: PathBuf,
    pub deleted_at_ms: u128,
}

impl TrashEntry {
    /// The store-relative path as the UI and the path file spell it.
    pub fn relative(&self) -> String {
        self.path
            .components()
            .map(|part| part.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/")
    }
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|age| age.as_millis())
        .unwrap_or(0)
}

fn trash_root(store_root: &Path) -> PathBuf {
    store_root.join(APP_DIR).join(TRASH_DIR)
}

fn path_from_text(text: &str) -> PathBuf {
    text.split('/').filter(|part| !part.is_empty()).collect()
}

/// Moves `target` into the store's Trash. The store root is deliberately a
/// separate argument from the tree's scope root: a repo-scoped window can
/// still delete a note without the trash folder failing containment.
pub fn trash_entry(store_root: &Path, target: &Path) -> anyhow::Result<TrashEntry> {
    ensure_under_root(store_root, target)?;
    if target == store_root {
        anyhow::bail!("the notes root cannot be moved to Trash");
    }
    let relative = relative_to_store(store_root, target)?;
    if relative
        .components()
        .next()
        .is_some_and(|part| part.as_os_str() == APP_DIR)
    {
        anyhow::bail!("the app's own folder cannot be moved to Trash");
    }

    let now = now_millis();
    let root = trash_root(store_root);
    std::fs::create_dir_all(&root)?;
    let id = loop {
        let candidate = format!("{now}-{:04x}", paste_rand16());
        if !root.join(&candidate).exists() {
            break candidate;
        }
    };
    let dir = root.join(&id);
    std::fs::create_dir_all(&dir)?;
    let path_text = relative.to_string_lossy().replace('\\', "/");
    if let Err(err) = std::fs::write(dir.join(PATH_FILE), &path_text) {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(err.into());
    }
    if let Err(err) = std::fs::rename(target, dir.join(PAYLOAD)) {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(err.into());
    }
    Ok(TrashEntry {
        id,
        path: relative,
        deleted_at_ms: now,
    })
}

/// Every complete entry, newest first. An id whose path file or payload is
/// missing is not listed; purge leaves it alone rather than deleting by guess.
pub fn list_trash(store_root: &Path) -> Vec<TrashEntry> {
    let Ok(entries) = std::fs::read_dir(trash_root(store_root)) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let id = entry.file_name().to_string_lossy().to_string();
        let Some(deleted_at_ms) = id
            .split_once('-')
            .and_then(|(stamp, _)| stamp.parse::<u128>().ok())
        else {
            continue;
        };
        let dir = entry.path();
        let Ok(path_text) = std::fs::read_to_string(dir.join(PATH_FILE)) else {
            continue;
        };
        if !dir.join(PAYLOAD).exists() {
            continue;
        }
        found.push(TrashEntry {
            id,
            path: path_from_text(&path_text),
            deleted_at_ms,
        });
    }
    found.sort_by_key(|entry| std::cmp::Reverse(entry.deleted_at_ms));
    found
}

/// Puts the entry back where it came from. A collision keeps both sides: the
/// restored copy takes a ` (restored)` suffix instead of replacing the live
/// file.
pub fn restore_entry(store_root: &Path, entry: &TrashEntry) -> anyhow::Result<PathBuf> {
    let dir = trash_root(store_root).join(&entry.id);
    let payload = dir.join(PAYLOAD);
    if !payload.exists() {
        anyhow::bail!("the Trash entry is missing its payload");
    }
    let relative = &entry.path;
    if relative.is_absolute()
        || relative
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        anyhow::bail!("the Trash entry points outside the store");
    }
    let target = store_root.join(relative);
    ensure_under_root(store_root, &target)?;
    let target = unique_restore_path(target);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(&payload, &target)?;
    let _ = std::fs::remove_dir_all(&dir);
    Ok(target)
}

/// Removes entries older than `max_age`. The caller decides the policy (the
/// app uses 30 days); the function is pure apart from the filesystem so the
/// clock can be pinned in a probe.
pub fn purge_trash(store_root: &Path, max_age: Duration) {
    purge_trash_at(store_root, now_millis(), max_age);
}

fn purge_trash_at(store_root: &Path, now_ms: u128, max_age: Duration) {
    let Ok(entries) = std::fs::read_dir(trash_root(store_root)) else {
        return;
    };
    let max_age_ms = max_age.as_millis();
    for entry in entries.flatten() {
        let id = entry.file_name().to_string_lossy().to_string();
        let Some(deleted_at_ms) = id
            .split_once('-')
            .and_then(|(stamp, _)| stamp.parse::<u128>().ok())
        else {
            continue;
        };
        if now_ms.saturating_sub(deleted_at_ms) > max_age_ms {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// Empty Trash: permanent by request, with no path outside the trash root.
pub fn empty_trash(store_root: &Path) {
    let Ok(entries) = std::fs::read_dir(trash_root(store_root)) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let _ = std::fs::remove_dir_all(path);
        } else {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn relative_to_store(store_root: &Path, target: &Path) -> anyhow::Result<PathBuf> {
    if let Ok(relative) = target.strip_prefix(store_root)
        && !relative.as_os_str().is_empty()
    {
        return Ok(relative.to_path_buf());
    }
    let root = store_root.canonicalize()?;
    let target = target.canonicalize()?;
    let relative = target
        .strip_prefix(&root)
        .map_err(|_| anyhow::anyhow!("the path escapes the notes store"))?;
    Ok(relative.to_path_buf())
}

fn unique_restore_path(target: PathBuf) -> PathBuf {
    if !target.exists() {
        return target;
    }
    let parent = target.parent().unwrap_or_else(|| Path::new(""));
    let stem = target
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_string())
        .unwrap_or_else(|| "restored".into());
    let extension = target
        .extension()
        .map(|extension| format!(".{}", extension.to_string_lossy()))
        .unwrap_or_default();
    for version in 1..=1000 {
        let suffix = if version == 1 {
            " (restored)".to_string()
        } else {
            format!(" (restored {version})")
        };
        let candidate = parent.join(format!("{stem}{suffix}{extension}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    parent.join(format!("{stem} (restored){extension}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scratch(prefix: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "{prefix}-{}-{:04x}",
            std::process::id(),
            paste_rand16()
        ))
    }

    fn seed_note(root: &Path) -> PathBuf {
        let note = root.join("projects/alpha/note.md");
        std::fs::create_dir_all(note.parent().unwrap()).unwrap();
        std::fs::write(&note, "body").unwrap();
        note
    }

    #[test]
    fn trash_restore_round_trip() {
        let root = scratch("warp-notes-trash");
        let _ = std::fs::remove_dir_all(&root);
        let note = seed_note(&root);

        let entry = trash_entry(&root, &note).unwrap();
        assert!(!note.exists(), "the live copy is gone");
        assert_eq!(entry.relative(), "projects/alpha/note.md");
        let listed = list_trash(&root);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].path, entry.path);

        let restored = restore_entry(&root, &entry).unwrap();
        assert_eq!(restored, note);
        assert_eq!(std::fs::read_to_string(&note).unwrap(), "body");
        assert!(list_trash(&root).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn trash_accepts_a_note_below_projects_while_the_store_is_root() {
        // The UI passes the store root even when the tree is repo-scoped; the
        // nested target must still pass containment and land in Trash.
        let root = scratch("warp-notes-trash-scope");
        let _ = std::fs::remove_dir_all(&root);
        let note = seed_note(&root);
        assert!(trash_entry(&root, &note).is_ok());
        assert!(list_trash(&root).len() == 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn restore_keeps_both_sides_on_a_collision() {
        let root = scratch("warp-notes-trash-collision");
        let _ = std::fs::remove_dir_all(&root);
        let note = seed_note(&root);
        let entry = trash_entry(&root, &note).unwrap();
        std::fs::write(&note, "new live body").unwrap();

        let restored = restore_entry(&root, &entry).unwrap();
        assert_eq!(restored, root.join("projects/alpha/note (restored).md"));
        assert_eq!(std::fs::read_to_string(note).unwrap(), "new live body");
        assert_eq!(std::fs::read_to_string(restored).unwrap(), "body");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn purge_keeps_recent_entries_and_drops_old_ones() {
        let root = scratch("warp-notes-trash-purge");
        let _ = std::fs::remove_dir_all(&root);
        let note = seed_note(&root);
        let entry = trash_entry(&root, &note).unwrap();
        let now = now_millis();
        let day = 24 * 60 * 60 * 1000;

        purge_trash_at(&root, now + day, Duration::from_secs(30 * 24 * 60 * 60));
        assert_eq!(list_trash(&root).len(), 1, "a recent entry survives");

        purge_trash_at(
            &root,
            now + 31 * day,
            Duration::from_secs(30 * 24 * 60 * 60),
        );
        assert!(list_trash(&root).is_empty(), "a 31-day entry is gone");
        assert!(!root.join(APP_DIR).join(TRASH_DIR).join(entry.id).exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn trash_refuses_the_store_root_and_the_app_folder() {
        let root = scratch("warp-notes-trash-refuse");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(APP_DIR)).unwrap();
        assert!(trash_entry(&root, &root).is_err());
        assert!(trash_entry(&root, &root.join(APP_DIR)).is_err());
        let outside = scratch("warp-notes-trash-outside");
        std::fs::write(&outside, "x").unwrap();
        assert!(trash_entry(&root, &outside).is_err());
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_file(&outside);
    }

    #[test]
    fn empty_trash_removes_every_entry() {
        let root = scratch("warp-notes-trash-empty");
        let _ = std::fs::remove_dir_all(&root);
        let note = seed_note(&root);
        let _ = trash_entry(&root, &note).unwrap();
        empty_trash(&root);
        assert!(list_trash(&root).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }
}
