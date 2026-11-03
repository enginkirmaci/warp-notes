//! The per-repo notes store: one subfolder per repository under the store
//! root (default ~/.warp/notes), markdown files only. Tree loading, CRUD with
//! sanitized names and capped content search; the pure markdown editing ops
//! live in warpui-kit-markdown.

use crate::paths::default_store_root;
use std::path::{Path, PathBuf};
use std::rc::Rc;

const SEARCH_CONTENT_CAP_BYTES: u64 = 2 * 1024 * 1024;
const SEARCH_FILE_CAP: usize = 2000;

/// One entry of the notes tree. Children are `Rc`s: the UI flattens the
/// visible rows on every render, and a collapsed 2000-note store must copy
/// pointers there, not every name and path.
#[derive(Clone, Debug)]
pub struct NotesTreeItem {
    pub name: String,
    pub full_path: PathBuf,
    pub is_folder: bool,
    pub children: Vec<Rc<NotesTreeItem>>,
}

#[derive(Clone, Debug)]
pub struct NotesSearchHit {
    pub path: PathBuf,
    pub title: String,
    pub relative: String,
    pub excerpt: String,
}

pub fn resolve_store_root(configured: &str) -> PathBuf {
    let trimmed = configured.trim();
    if trimmed.is_empty() {
        default_store_root()
    } else {
        PathBuf::from(trimmed)
    }
}

pub fn repo_notes_root(store_root: &Path, repo_name: &str) -> PathBuf {
    store_root.join(sanitize_entry_name(repo_name, false).unwrap_or_else(|| "notes".into()))
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
            })),
            Ok(_) if name.to_lowercase().ends_with(".md") => notes.push(Rc::new(NotesTreeItem {
                name,
                full_path: entry.path(),
                is_folder: false,
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
fn paste_rand16() -> u16 {
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

pub fn delete(repo_notes_root: &Path, target: &Path) -> anyhow::Result<()> {
    ensure_under_root(repo_notes_root, target)?;
    if target.is_dir() {
        std::fs::remove_dir_all(target)?;
    } else if target.is_file() {
        std::fs::remove_file(target)?;
    }
    Ok(())
}

pub fn search(repo_notes_root: &Path, term: &str) -> Vec<NotesSearchHit> {
    search_capped(repo_notes_root, term, SEARCH_FILE_CAP)
}

/// The notes-page search scans up to [`SEARCH_FILE_CAP`] files; the global
/// search passes a smaller cap (40 files per repo).
pub fn search_capped(repo_notes_root: &Path, term: &str, file_cap: usize) -> Vec<NotesSearchHit> {
    if !repo_notes_root.is_dir() || term.trim().is_empty() {
        return Vec::new();
    }
    let term = term.trim();
    let mut hits = Vec::new();
    for path in enumerate_note_files(repo_notes_root) {
        if hits.len() >= file_cap {
            break;
        }
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let name_match = file_name.to_lowercase().contains(&term.to_lowercase());
        let excerpt = if name_match {
            String::new()
        } else {
            match read_excerpt(&path, term) {
                Some(e) => e,
                None => continue,
            }
        };
        let relative = path
            .strip_prefix(repo_notes_root)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        hits.push(NotesSearchHit {
            title: file_name
                .trim_end_matches(".md")
                .trim_end_matches(".MD")
                .to_string(),
            excerpt: if excerpt.is_empty() {
                first_content_line(&path).unwrap_or_default()
            } else {
                excerpt
            },
            path,
            relative,
        });
    }
    hits
}

/// Path-name sanitizer: invalid filename chars and separators become '-',
/// edges trimmed; `None` when nothing survives.
pub fn sanitize_entry_name(name: &str, enforce_markdown_extension: bool) -> Option<String> {
    let mut cleaned: String = name
        .trim()
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0'..='\u{1f}' => '-',
            other => other,
        })
        .collect();
    cleaned = cleaned.trim_matches([' ', '.']).to_string();
    if cleaned.is_empty() {
        return None;
    }
    if enforce_markdown_extension && !cleaned.to_lowercase().ends_with(".md") {
        cleaned.push_str(".md");
    }
    Some(cleaned)
}

/// canonicalize() fails for the not-yet-created parent chains create_note /
/// create_folder validate, and on Windows it returns `\\?\`-prefixed verbatim
/// paths — normalize both sides to plain paths and compare components
/// (case-insensitively on Windows) so mixed forms still pass containment.
fn ensure_under_root(root: &Path, target: &Path) -> anyhow::Result<()> {
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

fn enumerate_note_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut queue = std::collections::VecDeque::new();
    queue.push_back(root.to_path_buf());
    while let Some(current) = queue.pop_front() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            match entry.file_type() {
                Ok(t) if t.is_dir() => {
                    if !name.starts_with('.') {
                        queue.push_back(entry.path());
                    }
                }
                Ok(_) if name.to_lowercase().ends_with(".md") => files.push(entry.path()),
                _ => {}
            }
        }
    }
    files
}

fn read_excerpt(path: &Path, term: &str) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > SEARCH_CONTENT_CAP_BYTES {
        return None;
    }
    let content = std::fs::read_to_string(path).ok()?;
    for raw_line in content.lines() {
        let line: String = raw_line.split_whitespace().collect::<Vec<_>>().join(" ");
        let Some(index) = line.to_lowercase().find(&term.to_lowercase()) else {
            continue;
        };
        // the offset was found in the lowercased copy, whose byte length can
        // differ (İ U+0130 lowercases to 3 bytes) — walk back onto the
        // original line's char boundaries before slicing
        let mut start = index.saturating_sub(40);
        while start > 0 && !line.is_char_boundary(start) {
            start += 1;
        }
        let mut end = (start + 160).min(line.len());
        while end < line.len() && !line.is_char_boundary(end) {
            end += 1;
        }
        let mut excerpt = String::new();
        if start > 0 {
            excerpt.push('…');
        }
        excerpt.push_str(&line[start..end]);
        if end < line.len() {
            excerpt.push('…');
        }
        return Some(excerpt);
    }
    None
}

fn first_content_line(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    content
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_replaces_invalid_and_enforces_md() {
        assert_eq!(
            sanitize_entry_name("a/b\\c:d", false).as_deref(),
            Some("a-b-c-d")
        );
        assert_eq!(
            sanitize_entry_name("  ..note.. ", false).as_deref(),
            Some("note")
        );
        assert_eq!(sanitize_entry_name("   ", false), None);
        assert_eq!(
            sanitize_entry_name("todo", true).as_deref(),
            Some("todo.md")
        );
        assert_eq!(
            sanitize_entry_name("TODO.MD", true).as_deref(),
            Some("TODO.MD")
        );
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
