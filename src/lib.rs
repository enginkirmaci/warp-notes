//! warp-notes: per-repo markdown notes with an Obsidian-style live editor.
//! The library boundary exists so the headless probes (tests/) can build the
//! real NotesPage; the binary is a thin bootstrap over it.

pub mod assets;
pub mod atomic_write;
pub mod logging;
pub mod notes_store;
pub mod paths;
pub mod theme;
pub mod trash;
pub mod ui;
pub mod ui_prefs;
