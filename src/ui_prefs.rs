//! The window's remembered bits, kept in `~/.warp/notes/settings.json` so
//! they survive restarts: the list width, the reduce-motion choice, and every
//! user-arranged piece of the shell — the open rail section, the list pane's
//! visibility, which note was open and which folders were expanded, and the
//! repo scope reached through the titlebar's up button. Appearance applies at
//! once (the design has no Save on it), so every change writes straight
//! through. Paths are stored relative to the notes store root, so a store
//! that moves house does not strand them.

use crate::atomic_write::write_atomically;
use std::path::PathBuf;
use warpui_kit_core::shell;

#[derive(Debug, Clone, PartialEq)]
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
    /// and a stored width outside the shell's bounds is clamped on load.
    pub fn load() -> Self {
        let text = match std::fs::read_to_string(prefs_path()) {
            Ok(text) => text,
            Err(_) => return Self::default(),
        };
        let mut prefs = Self::from_json(&text);
        prefs.list_width = shell::clamp_list_width(prefs.list_width);
        prefs
    }

    pub fn save(&self) {
        let path = prefs_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(err) = write_atomically(&path, &self.to_json()) {
            tracing::warn!("could not write {}: {err}", path.display());
        }
    }

    fn to_json(&self) -> String {
        serde_json::json!({
            "list_width": self.list_width,
            "reduce_motion": self.reduce_motion,
            "list_visible": self.list_visible,
            "section": self.section,
            "settings_section": self.settings_section,
            "open_note": self.open_note,
            "expanded": self.expanded,
            "notes_scope": self.notes_scope,
        })
        .to_string()
    }

    /// Every field stands alone: an unknown value falls back to that field's
    /// default, so a hand-edited or half-written file costs nothing.
    fn from_json(text: &str) -> Self {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
            tracing::warn!("could not parse {}", prefs_path().display());
            return Self::default();
        };
        let section = |key: &str, one: &str, other: &str, fallback: &str| -> String {
            match value.get(key).and_then(|v| v.as_str()) {
                Some(s) if s == one || s == other => s.to_string(),
                _ => fallback.to_string(),
            }
        };
        Self {
            list_width: value
                .get("list_width")
                .and_then(|v| v.as_f64())
                .unwrap_or(shell::LIST_WIDTH as f64) as f32,
            reduce_motion: value
                .get("reduce_motion")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            list_visible: value
                .get("list_visible")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            section: section("section", "notes", "settings", "notes"),
            settings_section: section("settings_section", "general", "appearance", "general"),
            open_note: value
                .get("open_note")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            expanded: value
                .get("expanded")
                .and_then(|v| v.as_array())
                .map(|list| {
                    list.iter()
                        .filter_map(|v| v.as_str())
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            notes_scope: value
                .get("notes_scope")
                .and_then(|v| v.as_str())
                .map(str::to_string),
        }
    }
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
            notes_scope: Some("alpha".into()),
        };
        let parsed = UiPrefs::from_json(&prefs.to_json());
        assert_eq!(parsed.list_width, 500.);
        assert!(parsed.reduce_motion);
        assert!(!parsed.list_visible);
        assert_eq!(parsed.section, "settings");
        assert_eq!(parsed.settings_section, "appearance");
        assert_eq!(parsed.open_note.as_deref(), Some("alpha/note.md"));
        assert_eq!(parsed.expanded, vec!["alpha", "alpha/sub"]);
        assert_eq!(parsed.notes_scope.as_deref(), Some("alpha"));

        let legacy = UiPrefs::from_json("{\"list_width\": 400}");
        assert_eq!(legacy.list_width, 400.);
        assert!(legacy.list_visible, "missing keys take the defaults");
        assert_eq!(legacy.section, "notes");
        assert_eq!(legacy.open_note, None);
        assert_eq!(legacy.expanded, Vec::<String>::new());
    }

    /// A mangled file is the defaults, and a saved width outside the shell's
    /// bounds is clamped on load — with the override pointing at a temp file
    /// so the developer's own prefs are never touched.
    #[test]
    fn load_defaults_on_garbage_and_clamps_width() {
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
        let loaded = UiPrefs::load();
        assert_eq!(
            loaded.list_width,
            shell::clamp_list_width(5000.),
            "the saved width is clamped back into the shell's bounds"
        );
        set_prefs_file_for_probe(None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
