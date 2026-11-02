//! The window's remembered bits: the list width and the reduce-motion
//! choice, kept in `~/.warp-notes/settings.json` so they survive restarts.
//! Appearance applies at once (the design has no Save on it), so every
//! change writes straight through.

use crate::atomic_write::write_atomically;
use std::path::PathBuf;
use warpui_kit_core::shell;

#[derive(Debug, Clone)]
pub struct UiPrefs {
    pub list_width: f32,
    pub reduce_motion: bool,
}

impl Default for UiPrefs {
    fn default() -> Self {
        Self {
            list_width: shell::LIST_WIDTH,
            reduce_motion: false,
        }
    }
}

fn prefs_path() -> PathBuf {
    crate::paths::AppPaths::config_dir().join("settings.json")
}

impl UiPrefs {
    /// Reads the prefs; a missing or unreadable file is simply the defaults,
    /// and a stored width outside the shell's bounds is clamped on load.
    pub fn load() -> Self {
        let Ok(text) = std::fs::read_to_string(prefs_path()) else {
            return Self::default();
        };
        let value: serde_json::Value = match serde_json::from_str(&text) {
            Ok(value) => value,
            Err(err) => {
                tracing::warn!("could not parse {}: {err}", prefs_path().display());
                return Self::default();
            }
        };
        Self {
            list_width: value
                .get("list_width")
                .and_then(|v| v.as_f64())
                .map(|v| shell::clamp_list_width(v as f32))
                .unwrap_or(shell::LIST_WIDTH),
            reduce_motion: value
                .get("reduce_motion")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        }
    }

    pub fn save(&self) {
        let path = prefs_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let text = serde_json::json!({
            "list_width": self.list_width,
            "reduce_motion": self.reduce_motion,
        })
        .to_string();
        if let Err(err) = write_atomically(&path, &text) {
            tracing::warn!("could not write {}: {err}", path.display());
        }
    }
}
