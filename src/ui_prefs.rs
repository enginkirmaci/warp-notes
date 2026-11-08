//! The window's remembered bits, kept in `~/.warp/notes/settings.json` so
//! they survive restarts: the list width, the reduce-motion choice, and every
//! user-arranged piece of the shell — the open rail section, the list pane's
//! visibility, which note was open and which folders were expanded, and the
//! repo scope reached through the titlebar's up button. Appearance applies at
//! once (the design has no Save on it), so every change writes straight
//! through. Paths are stored relative to the notes store root, so a store
//! that moves house does not strand them.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use warpui_kit_core::shell;
use warpui_kit_platform::config::Store;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiPrefs {
    pub list_width: f32,
    pub reduce_motion: bool,
    /// Whether the list pane shows at all (Ctrl+\ and the titlebar toggle).
    pub list_visible: bool,
    /// The rail's open section: "notes" or "settings".
    pub section: String,
    /// The Settings screen's own section: "general" or "appearance".
    pub settings_section: String,
    /// The note that was open, relative to the notes store root.
    pub open_note: Option<String>,
    /// The tree's expanded folders, relative to the notes store root.
    pub expanded: Vec<String>,
    /// Recently opened notes, newest first, relative to the store root.
    pub recent: Vec<String>,
    /// Pinned notes, relative to the store root.
    pub pinned: Vec<String>,
    /// A standalone launch's chosen store root; `--store` wins over it.
    pub store_path: Option<String>,
    /// The repo scope a `--repo` launch pinned or the up button reached.
    pub notes_scope: Option<String>,
}

impl Default for UiPrefs {
    fn default() -> Self {
        Self {
            list_width: shell::LIST_WIDTH,
            reduce_motion: false,
            list_visible: true,
            section: "notes".into(),
            settings_section: "general".into(),
            open_note: None,
            expanded: Vec::new(),
            recent: Vec::new(),
            pinned: Vec::new(),
            store_path: None,
            notes_scope: None,
        }
    }
}

fn prefs_path() -> PathBuf {
    if let Some(path) = PREFS_OVERRIDE.read().unwrap().clone() {
        return path;
    }
    crate::paths::AppPaths::config_dir().join("settings.json")
}

impl UiPrefs {
    /// Reads the prefs; a missing or unreadable file is simply the defaults,
    /// and a stored width outside the shell's bounds is clamped on load. A
    /// pre-versioning file is imported once and rewritten with the version.
    pub fn load() -> Self {
        let path = prefs_path();
        let store = Store::<UiPrefs>::new(path.clone());
        match store.load() {
            Ok(prefs) => prefs.normalized(),
            Err(err) if err.needs_import() => {
                let prefs = std::fs::read_to_string(&path)
                    .ok()
                    .and_then(|text| serde_json::from_str::<UiPrefs>(&text).ok())
                    .unwrap_or_default()
                    .normalized();
                let _ = store.save(&prefs);
                prefs
            }
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) {
        let _ = Store::<UiPrefs>::new(prefs_path()).save(self);
    }

    /// Values a hand-edited file may hold that the rest of the app would
    /// otherwise have to re-check on every use.
    fn normalized(mut self) -> Self {
        self.list_width = shell::clamp_list_width(self.list_width);
        if self.section != "notes" && self.section != "settings" {
            self.section = "notes".into();
        }
        if self.settings_section != "general" && self.settings_section != "appearance" {
            self.settings_section = "general".into();
        }
        self.recent = normalize_relative_list(self.recent, 20);
        self.pinned = normalize_relative_list(self.pinned, 200);
        self.store_path = self
            .store_path
            .map(|path| path.trim().to_string())
            .filter(|path| !path.is_empty());
        self
    }
}

fn normalize_relative_list(paths: Vec<String>, cap: usize) -> Vec<String> {
    let mut normalized = Vec::new();
    for path in paths {
        let path = path.trim().replace('\\', "/");
        if path.is_empty()
            || Path::new(&path).is_absolute()
            || path.split('/').any(|part| part == "..")
        {
            continue;
        }
        if !normalized.contains(&path) {
            normalized.push(path);
        }
    }
    normalized.truncate(cap);
    normalized
}

/// Redirects the prefs file — the headless probes point it at a temp dir so a
/// test run neither reads the developer's own layout nor rewrites it. The
/// probes serialize on one lock, so the override is a plain static slot.
static PREFS_OVERRIDE: std::sync::RwLock<Option<PathBuf>> = std::sync::RwLock::new(None);

/// The headless probes' prefs-file override; see [`PREFS_OVERRIDE`].
#[doc(hidden)]
pub fn set_prefs_file_for_probe(path: Option<PathBuf>) {
    *PREFS_OVERRIDE.write().unwrap() = path;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The prefs override is process-global; these tests take turns.
    static PREFS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Every field round-trips, and an old file without the newer keys
    /// simply falls back to the defaults for them.
    #[test]
    fn prefs_round_trip_and_default_through_missing_fields() {
        let prefs = UiPrefs {
            list_width: 500.,
            reduce_motion: true,
            list_visible: false,
            section: "settings".into(),
            settings_section: "appearance".into(),
            open_note: Some("alpha/note.md".into()),
            expanded: vec!["alpha".into(), "alpha/sub".into()],
            recent: vec!["alpha/note.md".into(), "beta/note.md".into()],
            pinned: vec!["alpha/note.md".into()],
            store_path: Some("/mnt/notes".into()),
            notes_scope: Some("alpha".into()),
        };
        let text = serde_json::to_string(&prefs).unwrap();
        let parsed: UiPrefs = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed.list_width, 500.);
        assert!(parsed.reduce_motion);
        assert!(!parsed.list_visible);
        assert_eq!(parsed.section, "settings");
        assert_eq!(parsed.settings_section, "appearance");
        assert_eq!(parsed.open_note.as_deref(), Some("alpha/note.md"));
        assert_eq!(parsed.expanded, vec!["alpha", "alpha/sub"]);
        assert_eq!(parsed.recent, vec!["alpha/note.md", "beta/note.md"]);
        assert_eq!(parsed.pinned, vec!["alpha/note.md"]);
        assert_eq!(parsed.store_path.as_deref(), Some("/mnt/notes"));
        assert_eq!(parsed.notes_scope.as_deref(), Some("alpha"));

        let legacy: UiPrefs = serde_json::from_str("{\"list_width\": 400}").unwrap();
        assert_eq!(legacy.list_width, 400.);
        assert!(legacy.list_visible, "missing keys take the defaults");
        assert_eq!(legacy.section, "notes");
        assert_eq!(legacy.open_note, None);
        assert_eq!(legacy.expanded, Vec::<String>::new());
        assert!(legacy.recent.is_empty());
        assert!(legacy.pinned.is_empty());
        assert_eq!(legacy.store_path, None);
    }

    #[test]
    fn recent_and_pinned_lists_normalize_and_dedupe() {
        let prefs = UiPrefs {
            recent: vec![
                "alpha\\note.md".into(),
                "alpha/note.md".into(),
                "../escape.md".into(),
                String::new(),
            ],
            pinned: vec!["beta/note.md".into(), "beta/note.md".into()],
            ..UiPrefs::default()
        }
        .normalized();
        assert_eq!(prefs.recent, vec!["alpha/note.md"]);
        assert_eq!(prefs.pinned, vec!["beta/note.md"]);
    }

    /// A mangled file is the defaults, and a saved width outside the shell's
    /// bounds is clamped on load — with the override pointing at a temp file
    /// so the developer's own prefs are never touched.
    #[test]
    fn load_defaults_on_garbage_and_clamps_width() {
        let _turn = PREFS_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let dir = std::env::temp_dir().join(format!("warp-notes-prefs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        std::fs::write(&path, "{ not json").unwrap();
        set_prefs_file_for_probe(Some(path.clone()));
        let prefs = UiPrefs::load();
        assert_eq!(prefs, UiPrefs::default());

        let wide = UiPrefs {
            list_width: 5000.,
            ..UiPrefs::default()
        };
        wide.save();
        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved["schema_version"], 1);
        assert_eq!(saved["list_width"], 5000.0);
        let loaded = UiPrefs::load();
        assert_eq!(
            loaded.list_width,
            shell::clamp_list_width(5000.),
            "the saved width is clamped back into the shell's bounds"
        );
        set_prefs_file_for_probe(None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A pre-versioning file is read, rewritten with `schema_version`, and
    /// keeps its values.
    #[test]
    fn a_legacy_file_is_imported_once() {
        let _turn = PREFS_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let dir = std::env::temp_dir().join(format!("warp-notes-prefs-old-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        std::fs::write(&path, r#"{"list_width": 400, "section": "settings"}"#).unwrap();
        set_prefs_file_for_probe(Some(path.clone()));

        let loaded = UiPrefs::load();
        assert_eq!(loaded.list_width, 400.);
        assert_eq!(loaded.section, "settings");

        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved["schema_version"], 1);
        assert_eq!(saved["list_width"], 400.0);
        set_prefs_file_for_probe(None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
