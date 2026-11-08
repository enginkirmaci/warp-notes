//! The per-repo notes store: one subfolder per repository under
//! `<store root>/projects` (default store `~/notes`), markdown files only.
//! Tree loading and CRUD
//! with sanitized names; the read/search half and the name sanitizer are the
//! platform kit's (`warpui_kit_platform::notes`, shared with
//! warp-station-core over the same on-disk format), the markdown editing ops
//! live in warpui-kit-markdown.

use crate::paths::default_store_root;
use std::path::{Path, PathBuf};
use std::rc::Rc;

const SEARCH_FILE_CAP: usize = 2000;

// the shared read/search layer, kept reachable under this module's names
pub use warpui_kit_platform::notes::{
    NotesSearchHit, repo_notes_root, sanitize_entry_name, search_capped,
};

/// One entry of the notes tree. Children are `Rc`s: the UI flattens the
/// visible rows on every render, and a collapsed 2000-note store must copy
/// pointers there, not every name and path.
#[derive(Clone, Debug)]
pub struct NotesTreeItem {
    pub name: String,
    pub full_path: PathBuf,
    pub is_folder: bool,
    /// The file's last write, read once while the tree is walking: the list's
    /// row stamp reads this instead of stat-ing every visible row per frame.
    pub modified: Option<std::time::SystemTime>,
    pub children: Vec<Rc<NotesTreeItem>>,
}

pub fn resolve_store_root(configured: &str) -> PathBuf {
    warpui_kit_platform::notes::resolve_store_root(configured, &default_store_root())
}

/// Resolve a `--note` launch argument (an absolute path — DevTools passes the
/// search hit's — or one relative to the store root) to a note path, which
/// must stay contained like every other store path. Relative paths may not
/// traverse with `..`: a not-yet-existing target canonicalizes nowhere, so
/// lexical containment alone cannot catch it.
pub fn resolve_note_in_store(store_root: &Path, requested: &Path) -> anyhow::Result<PathBuf> {
    if requested
        .components()
        .any(|c| c == std::path::Component::ParentDir)
    {
        anyhow::bail!("the --note path must not traverse outside the store");
    }
    let resolved = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        store_root.join(requested.to_string_lossy().trim_start_matches(['/', '\\']))
    };
    ensure_under_root(store_root, &resolved)?;
    Ok(resolved)
}

pub fn load_tree(repo_notes_root: &Path) -> Vec<Rc<NotesTreeItem>> {
    if !repo_notes_root.is_dir() {
        return Vec::new();
    }
    load_directory(repo_notes_root)
}

fn load_directory(directory: &Path) -> Vec<Rc<NotesTreeItem>> {
    let mut folders = Vec::new();
    let mut notes = Vec::new();
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut sorted: Vec<_> = entries.flatten().collect();
    sorted.sort_by_key(|e| e.file_name());
    for entry in sorted {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        match entry.file_type() {
            Ok(t) if t.is_dir() => folders.push(Rc::new(NotesTreeItem {
                children: load_directory(&entry.path()),
                name,
                full_path: entry.path(),
                is_folder: true,
                modified: entry.metadata().ok().and_then(|meta| meta.modified().ok()),
            })),
            Ok(_) if name.to_lowercase().ends_with(".md") => notes.push(Rc::new(NotesTreeItem {
                name,
                full_path: entry.path(),
                is_folder: false,
                modified: entry.metadata().ok().and_then(|meta| meta.modified().ok()),
                children: Vec::new(),
            })),
            _ => {}
        }
    }
    folders.sort_by_key(|a| a.name.to_lowercase());
    notes.sort_by_key(|a| a.name.to_lowercase());
    folders.into_iter().chain(notes).collect()
}

pub fn read_note(path: &Path) -> anyhow::Result<String> {
    Ok(std::fs::read_to_string(path)?)
}

/// The on-disk identity of a note at the last read or write: enough to tell
/// an external edit from our own write without hashing the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileMeta {
    pub modified: Option<std::time::SystemTime>,
    pub len: u64,
}

/// Current metadata for `path`; None when the file is missing or unreadable.
pub fn note_meta(path: &Path) -> Option<FileMeta> {
    let meta = std::fs::metadata(path).ok()?;
    Some(FileMeta {
        modified: meta.modified().ok(),
        len: meta.len(),
    })
}

/// What to do before writing an open note, from the editor's dirty flag and
/// the metadata captured when the note was last read or written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveDecision {
    /// Nothing changed since the last save.
    NothingToDo,
    /// The disk still matches the baseline: the write is safe.
    Write,
    /// The file changed (or vanished) underneath us: never overwrite blind.
    ChangedOnDisk,
}

/// The conflict policy, isolated so it is unit-testable: a note we have no
/// baseline for is written (we cannot know better), a clean buffer never
/// writes, and a dirty buffer writes only while the file still matches the
/// metadata our text was based on.
pub fn save_decision(dirty: bool, disk: Option<FileMeta>, saved: Option<FileMeta>) -> SaveDecision {
    if !dirty {
        return SaveDecision::NothingToDo;
    }
    match saved {
        Some(saved) if disk != Some(saved) => SaveDecision::ChangedOnDisk,
        _ => SaveDecision::Write,
    }
}

/// BOM-less UTF-8 — a BOM shows up as stray bytes in git diffs.
pub fn write_note(path: &Path, content: &str) -> anyhow::Result<()> {
    crate::atomic_write::write_atomically(path, content)
}

pub fn create_note(repo_notes_root: &Path, parent: &Path, name: &str) -> anyhow::Result<PathBuf> {
    let file_name = sanitize_entry_name(name, true)
        .ok_or_else(|| anyhow::anyhow!("The note name is empty."))?;
    ensure_under_root(repo_notes_root, parent)?;
    std::fs::create_dir_all(parent)?;
    let path = parent.join(&file_name);
    std::fs::File::create_new(&path)?;
    Ok(path)
}

/// Saves pasted clipboard image bytes as `<folder>/assets/paste-<local
/// datetime>-<4 hex>.<ext>` and returns the path relative to the note's
/// folder for the markdown ref (forward slashes, so the link works on every
/// platform). `File::create_new` makes the write exclusive: a same-second
/// second paste retries with fresh randomness instead of clobbering.
pub fn insert_image_asset(folder: &Path, ext: &str, bytes: &[u8]) -> std::io::Result<String> {
    if ext.is_empty() || !ext.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid image extension",
        ));
    }
    use std::io::Write as _;
    let assets = folder.join("assets");
    std::fs::create_dir_all(&assets)?;
    for _ in 0..8 {
        let rand = paste_rand16();
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let name = format!("paste-{stamp}-{rand:04x}.{ext}");
        match std::fs::File::create_new(assets.join(&name)) {
            Ok(mut file) => {
                file.write_all(bytes)?;
                return Ok(format!("assets/{name}"));
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "no unique asset name found",
    ))
}

/// Four hex digits of per-call entropy — `RandomState` seeds are random per
/// instance, so no `rand` dependency and same-second pastes still differ.
pub(crate) fn paste_rand16() -> u16 {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u32(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0),
    );
    hasher.finish() as u16
}

pub fn create_folder(repo_notes_root: &Path, parent: &Path, name: &str) -> anyhow::Result<PathBuf> {
    let folder_name = sanitize_entry_name(name, false)
        .ok_or_else(|| anyhow::anyhow!("The folder name is empty."))?;
    ensure_under_root(repo_notes_root, parent)?;
    let path = parent.join(&folder_name);
    if path.exists() {
        return Err(anyhow::anyhow!("'{folder_name}' already exists."));
    }
    std::fs::create_dir_all(&path)?;
    Ok(path)
}

/// Renames one note or folder in place. Notes keep their `.md` extension;
/// folders keep none. The target directory is the scope root, so a repo-scoped
/// window cannot rename a note into another repo. A case-only rename on
/// Windows goes through a temporary name because the target already "exists".
pub fn rename_entry(scope_root: &Path, target: &Path, new_name: &str) -> anyhow::Result<PathBuf> {
    ensure_under_root(scope_root, target)?;
    if target == scope_root {
        anyhow::bail!("the notes root cannot be renamed");
    }
    let is_folder = target.is_dir();
    let file_name = sanitize_entry_name(new_name, !is_folder)
        .ok_or_else(|| anyhow::anyhow!("The name is empty."))?;
    let parent = target
        .parent()
        .ok_or_else(|| anyhow::anyhow!("the entry has no parent"))?;
    let destination = parent.join(&file_name);
    ensure_under_root(scope_root, &destination)?;
    if destination == target {
        return Ok(target.to_path_buf());
    }
    if destination.exists() {
        // Windows treats a case-only change as an existing target; allow it
        // only when both paths resolve to the same file, through a temp hop.
        let same = target
            .canonicalize()
            .ok()
            .zip(destination.canonicalize().ok())
            .is_some_and(|(left, right)| left == right);
        if !same {
            anyhow::bail!("'{file_name}' already exists");
        }
        let temporary = parent.join(format!(".warp-rename-{:04x}", paste_rand16()));
        std::fs::rename(target, &temporary)?;
        if let Err(err) = std::fs::rename(&temporary, &destination) {
            let _ = std::fs::rename(&temporary, target);
            return Err(err.into());
        }
    } else {
        std::fs::rename(target, &destination)?;
    }
    Ok(destination)
}

/// Moves one note or folder under another folder in the same scope root.
/// Moving a folder into itself or a descendant is refused before the rename,
/// as is every existing target.
pub fn move_entry(scope_root: &Path, target: &Path, new_parent: &Path) -> anyhow::Result<PathBuf> {
    ensure_under_root(scope_root, target)?;
    ensure_under_root(scope_root, new_parent)?;
    if !new_parent.is_dir() {
        anyhow::bail!("the destination folder does not exist");
    }
    if target.is_dir() {
        let target_canon = target
            .canonicalize()
            .map_err(|err| anyhow::anyhow!("cannot resolve the folder: {err}"))?;
        let parent_canon = new_parent
            .canonicalize()
            .map_err(|err| anyhow::anyhow!("cannot resolve the destination: {err}"))?;
        if parent_canon.starts_with(&target_canon) {
            anyhow::bail!("a folder cannot be moved into itself");
        }
    }
    let file_name = target
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("the entry has no name"))?;
    let destination = new_parent.join(file_name);
    ensure_under_root(scope_root, &destination)?;
    if destination == target {
        return Ok(target.to_path_buf());
    }
    if destination.exists() {
        anyhow::bail!("'{}' already exists", destination.display());
    }
    std::fs::rename(target, &destination)?;
    Ok(destination)
}

/// Copies one note next to itself as `<stem> copy.md`, then `copy 2`, and so
/// on. The write is exclusive, so another process cannot claim the same name
/// between the probe and the copy.
pub fn duplicate_note(scope_root: &Path, source: &Path) -> anyhow::Result<PathBuf> {
    ensure_under_root(scope_root, source)?;
    if !source.is_file() {
        anyhow::bail!("only a note can be duplicated");
    }
    let parent = source
        .parent()
        .ok_or_else(|| anyhow::anyhow!("the note has no parent"))?;
    let stem = source
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .ok_or_else(|| anyhow::anyhow!("the note has no usable name"))?;
    let bytes = std::fs::read(source)?;
    use std::io::Write as _;
    for version in 1..=1000 {
        let name = if version == 1 {
            format!("{stem} copy.md")
        } else {
            format!("{stem} copy {version}.md")
        };
        let candidate = parent.join(name);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(mut file) => {
                file.write_all(&bytes)?;
                return Ok(candidate);
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err.into()),
        }
    }
    anyhow::bail!("no unique copy name was found")
}

pub fn search(repo_notes_root: &Path, term: &str) -> Vec<NotesSearchHit> {
    search_capped(repo_notes_root, term, SEARCH_FILE_CAP)
}

/// canonicalize() fails for the not-yet-created parent chains create_note /
/// create_folder validate, and on Windows it returns `\\?\`-prefixed verbatim
/// paths — normalize both sides to plain paths and compare components
/// (case-insensitively on Windows) so mixed forms still pass containment.
pub(crate) fn ensure_under_root(root: &Path, target: &Path) -> anyhow::Result<()> {
    let root = plain_path(root.canonicalize().unwrap_or_else(|_| root.to_path_buf()));
    let target = plain_path(
        target
            .canonicalize()
            .unwrap_or_else(|_| target.to_path_buf()),
    );
    let contained = if cfg!(windows) {
        let fold = |p: &Path| -> Vec<String> {
            p.components()
                .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
                .collect()
        };
        fold(&target).starts_with(&fold(&root))
    } else {
        target.starts_with(&root)
    };
    if contained {
        Ok(())
    } else {
        Err(anyhow::anyhow!("the path escapes the notes store"))
    }
}

fn plain_path(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    let plain = text
        .strip_prefix(r"\\?\UNC\")
        .map(|rest| format!(r"\\{rest}"))
        .or_else(|| text.strip_prefix(r"\\?\").map(str::to_string));
    match plain {
        Some(plain) => PathBuf::from(plain),
        None => path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(prefix: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "{prefix}-{}-{:04x}",
            std::process::id(),
            paste_rand16()
        ))
    }

    #[test]
    fn save_decision_never_overwrites_a_changed_file() {
        let before = FileMeta {
            modified: None,
            len: 10,
        };
        let after = FileMeta {
            modified: None,
            len: 11,
        };
        assert_eq!(
            save_decision(false, Some(before), Some(before)),
            SaveDecision::NothingToDo
        );
        assert_eq!(
            save_decision(true, Some(before), Some(before)),
            SaveDecision::Write
        );
        assert_eq!(
            save_decision(true, Some(after), Some(before)),
            SaveDecision::ChangedOnDisk
        );
        // deleted underneath the editor: do not recreate it silently
        assert_eq!(
            save_decision(true, None, Some(before)),
            SaveDecision::ChangedOnDisk
        );
        // no baseline (metadata unreadable) leaves the old write-through
        assert_eq!(save_decision(true, Some(before), None), SaveDecision::Write);
    }

    #[test]
    fn tree_load_and_search_roundtrip() {
        let tmp = std::env::temp_dir().join(format!("dt-notes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let root = tmp.join("My Repo");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("alpha.md"), "# Title\n\nhello needle here\n").unwrap();
        std::fs::write(root.join("sub/beta.md"), "nothing relevant\n").unwrap();
        std::fs::write(root.join(".hidden.md"), "needle").unwrap();

        let tree = load_tree(&root);
        assert_eq!(tree.len(), 2);
        assert!(tree[0].is_folder && tree[0].name == "sub");
        assert_eq!(tree[1].name, "alpha.md");

        // hidden FILES match by name (only dot-DIRECTORIES are skipped)
        let hits = search(&root, "needle");
        assert_eq!(hits.len(), 2);
        assert!(hits.iter().any(|h| h.excerpt.contains("needle")));

        let note = create_note(&root, &root, "New/Note").unwrap();
        assert!(note.to_string_lossy().ends_with("New-Note.md"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn plain_path_strips_windows_verbatim_prefixes() {
        assert_eq!(
            plain_path(PathBuf::from(r"\\?\C:\Users\e\notes")),
            PathBuf::from(r"C:\Users\e\notes")
        );
        assert_eq!(
            plain_path(PathBuf::from(r"\\?\UNC\server\share\notes")),
            PathBuf::from(r"\\server\share\notes")
        );
        assert_eq!(
            plain_path(PathBuf::from("/home/e/notes")),
            PathBuf::from("/home/e/notes")
        );
    }

    #[test]
    fn insert_image_asset_writes_distinct_files() {
        const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";
        let tmp = std::env::temp_dir().join(format!("dt-notes-asset-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let folder = tmp.join("alpha");
        std::fs::create_dir_all(&folder).unwrap();

        let first = insert_image_asset(&folder, "png", PNG_MAGIC).unwrap();
        // the ref is relative with forward slashes — a markdown link
        assert!(
            first.starts_with("assets/paste-") && first.ends_with(".png"),
            "{first}"
        );
        assert!(!first.contains('\\'), "{first}");
        let name = first
            .trim_start_matches("assets/paste-")
            .trim_end_matches(".png");
        let parts: Vec<&str> = name.split('-').collect();
        assert_eq!(parts.len(), 3, "date-time-rand: {first}");
        assert_eq!(
            (parts[0].len(), parts[1].len(), parts[2].len()),
            (8, 6, 4),
            "{first}"
        );
        assert!(parts[2].chars().all(|c| c.is_ascii_hexdigit()), "{first}");

        let written = std::fs::read(folder.join(&first)).unwrap();
        assert_eq!(written, PNG_MAGIC, "bytes land on disk unchanged");

        let second = insert_image_asset(&folder, "png", b"other").unwrap();
        assert_ne!(first, second, "same-second pastes must not collide");

        assert!(insert_image_asset(&folder, "bad/ext", b"").is_err());
        assert!(insert_image_asset(&folder, "", b"").is_err());
        assert_eq!(std::fs::read_dir(folder.join("assets")).unwrap().count(), 2);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn create_note_accepts_verbatim_root_with_new_parent() {
        // root canonicalized (verbatim on Windows), parent not yet on disk
        let tmp = std::env::temp_dir().join(format!("dt-notes-verbatim-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let root = tmp.join("Repo");
        std::fs::create_dir_all(&root).unwrap();
        let canonical_root = root.canonicalize().unwrap();
        let parent = root.join("New Folder");
        create_note(&canonical_root, &parent, "note").unwrap();
        assert!(parent.join("note.md").is_file());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn containment_rejects_escapes() {
        let tmp = std::env::temp_dir().join(format!("dt-notes-contain-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let root = tmp.join("store");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(tmp.join("outside")).unwrap();
        assert!(ensure_under_root(&root, &tmp.join("outside")).is_err());
        assert!(ensure_under_root(&root, root.parent().unwrap()).is_err());
        assert!(ensure_under_root(&root, &root).is_ok());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn rename_entry_keeps_note_extension_and_refuses_overwrite() {
        let root = scratch("warp-notes-rename");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("alpha.md"), "alpha").unwrap();
        std::fs::write(root.join("beta.md"), "beta").unwrap();
        std::fs::create_dir_all(root.join("folder")).unwrap();

        let renamed = rename_entry(&root, &root.join("alpha.md"), "Gamma").unwrap();
        assert_eq!(renamed, root.join("Gamma.md"));
        assert_eq!(std::fs::read_to_string(&renamed).unwrap(), "alpha");
        assert!(!root.join("alpha.md").exists());
        assert!(rename_entry(&root, &renamed, "beta").is_err());

        let folder = rename_entry(&root, &root.join("folder"), "Renamed").unwrap();
        assert_eq!(folder, root.join("Renamed"));
        assert!(folder.is_dir());

        let outside = scratch("warp-notes-rename-outside");
        assert!(rename_entry(&root, &outside, "escape").is_err());
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[test]
    fn move_entry_refuses_a_folder_into_itself() {
        let root = scratch("warp-notes-move");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("a/sub")).unwrap();
        std::fs::write(root.join("a/note.md"), "note").unwrap();

        assert!(move_entry(&root, &root.join("a"), &root.join("a/sub")).is_err());
        let moved = move_entry(&root, &root.join("a/note.md"), &root.join("a/sub")).unwrap();
        assert_eq!(moved, root.join("a/sub/note.md"));
        assert_eq!(std::fs::read_to_string(&moved).unwrap(), "note");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn duplicate_note_names_are_unique_and_leave_the_original() {
        let root = scratch("warp-notes-copy");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("a")).unwrap();
        std::fs::write(root.join("a/note.md"), "body").unwrap();

        let first = duplicate_note(&root, &root.join("a/note.md")).unwrap();
        let second = duplicate_note(&root, &root.join("a/note.md")).unwrap();
        assert_eq!(first, root.join("a/note copy.md"));
        assert_eq!(second, root.join("a/note copy 2.md"));
        assert_eq!(std::fs::read_to_string(first).unwrap(), "body");
        assert_eq!(std::fs::read_to_string(second).unwrap(), "body");
        assert_eq!(
            std::fs::read_to_string(root.join("a/note.md")).unwrap(),
            "body"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn resolve_note_rejects_traversal_and_joins_relative() {
        let root = std::env::temp_dir().join(format!("dt-notes-resolve-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        assert!(resolve_note_in_store(&root, Path::new("../escape.md")).is_err());
        // a not-yet-existing absolute path with .. must not pass lexically
        assert!(
            resolve_note_in_store(&root, &root.join("..").join("escape.md")).is_err(),
            "lexical containment missed a .. escape"
        );
        assert_eq!(
            resolve_note_in_store(&root, Path::new("a/b.md")).unwrap(),
            root.join("a/b.md")
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
