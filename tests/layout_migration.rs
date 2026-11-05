//! The per-user layout migration, end to end: one scratch HOME carrying the
//! folders an older build wrote, then the tree this build reads. HOME is
//! process-global, so the two scenarios take turns behind a lock.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

static HOME_LOCK: Mutex<()> = Mutex::new(());

fn hold_home() -> MutexGuard<'static, ()> {
    HOME_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A fresh scratch home; the caller holds it through [`HomeGuard`].
fn scratch_home(tag: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!("warp-notes-layout-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    home
}

/// Points HOME (and USERPROFILE, which Windows checks first) at `home` for as
/// long as the guard lives, then puts the developer's own values back.
struct HomeGuard {
    prior: Vec<(&'static str, Option<OsString>)>,
}

impl HomeGuard {
    fn set(home: &Path) -> Self {
        let prior = ["HOME", "USERPROFILE"]
            .into_iter()
            .map(|key| (key, std::env::var_os(key)))
            .collect::<Vec<_>>();
        for (key, _) in &prior {
            // SAFETY: process-global env, serialized by HOME_LOCK
            unsafe { std::env::set_var(key, home) };
        }
        Self { prior }
    }
}

impl Drop for HomeGuard {
    fn drop(&mut self) {
        for (key, value) in self.prior.drain(..) {
            match value {
                Some(value) => unsafe { std::env::set_var(key, value) },
                None => unsafe { std::env::remove_var(key) },
            }
        }
    }
}

fn write(path: impl AsRef<Path>, body: &str) {
    let path = path.as_ref();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

fn read(path: impl AsRef<Path>) -> String {
    std::fs::read_to_string(path.as_ref()).unwrap()
}

/// The old layout: the app's folder at `~/.warp-notes`, the notes store at
/// `~/.warp/notes`, the shared `ui.json` at the suite root.
fn seed_old_layout(home: &Path) {
    write(
        home.join(".warp-notes/settings.json"),
        r#"{"list_width":400}"#,
    );
    write(
        home.join(".warp-notes/logs/warpnotes-.log.2026-11-04"),
        "old log line",
    );
    write(home.join(".warp/notes/alpha/note.md"), "# alpha");
    write(home.join(".warp/notes/beta/beta.md"), "# beta");
    write(home.join(".warp/notes/flat.md"), "# flat");
    write(home.join(".warp/ui.json"), r#"{"text_size":"small"}"#);
}

/// The store moves to `~/notes`, the app's settings take the freed
/// `~/.warp/notes` name, the logs join the shared folder and the legacy
/// app folder is retired.
#[test]
fn the_old_layout_moves_under_the_suite_root_and_the_store_to_home_notes() {
    let _guard = hold_home();
    let home = scratch_home("full");
    seed_old_layout(&home);
    let _home = HomeGuard::set(&home);

    warp_notes::paths::migrate_legacy_layout();

    assert_eq!(read(home.join("notes/alpha/note.md")), "# alpha");
    assert_eq!(read(home.join("notes/beta/beta.md")), "# beta");
    assert_eq!(read(home.join("notes/flat.md")), "# flat");
    assert_eq!(
        read(home.join(".warp/notes/settings.json")),
        r#"{"list_width":400}"#,
        "the app's settings folder took the old store's name"
    );
    assert_eq!(
        read(home.join(".warp/logs/warpnotes-.log.2026-11-04")),
        "old log line"
    );
    assert!(
        !home.join(".warp-notes").exists(),
        "the legacy app folder is retired"
    );
    assert_eq!(
        read(home.join(".warp/ui.json")),
        r#"{"text_size":"small"}"#,
        "the shared file is not the app's to move"
    );
    let _ = std::fs::remove_dir_all(&home);
}

/// Note content that only ever lived inside the old `~/.warp-notes` folder
/// (the pre-suite store) reaches `~/notes` on the same run: the legacy folder
/// is absorbed before the store pass, not after it.
#[test]
fn notes_inside_the_old_app_folder_reach_the_store() {
    let _guard = hold_home();
    let home = scratch_home("old-app-store");
    write(home.join(".warp-notes/alpha/note.md"), "# alpha");
    write(
        home.join(".warp-notes/settings.json"),
        r#"{"list_width":400}"#,
    );
    let _home = HomeGuard::set(&home);

    warp_notes::paths::migrate_legacy_layout();

    assert_eq!(read(home.join("notes/alpha/note.md")), "# alpha");
    assert_eq!(
        read(home.join(".warp/notes/settings.json")),
        r#"{"list_width":400}"#
    );
    assert!(!home.join(".warp-notes").exists());
    let _ = std::fs::remove_dir_all(&home);
}

/// Once the one-shot store migration ran, `~/.warp/notes` is the app's own
/// folder: a later entry there must stay put instead of being moved into the
/// user's notes (audit C2).
#[test]
fn the_store_migration_is_one_shot() {
    let _guard = hold_home();
    let home = scratch_home("one-shot");
    seed_old_layout(&home);
    let _home = HomeGuard::set(&home);

    warp_notes::paths::migrate_legacy_layout();
    assert!(
        home.join(".warp/.notes-store-v1").is_file(),
        "the completed migration leaves its marker"
    );

    // app data (or a future release's cache) appears in the app folder
    write(home.join(".warp/notes/cache/blob.bin"), "x");
    warp_notes::paths::migrate_legacy_layout();

    assert!(
        home.join(".warp/notes/cache/blob.bin").is_file(),
        "the repurposed app folder is not scanned after the marker"
    );
    assert!(!home.join("notes/cache").exists());
    let _ = std::fs::remove_dir_all(&home);
}

/// A second launch finds nothing left to move and keeps every path.
#[test]
fn a_second_pass_over_a_migrated_home_changes_nothing() {
    let _guard = hold_home();
    let home = scratch_home("second");
    seed_old_layout(&home);
    let _home = HomeGuard::set(&home);

    warp_notes::paths::migrate_legacy_layout();
    warp_notes::paths::migrate_legacy_layout();

    assert!(home.join("notes/alpha/note.md").is_file());
    assert!(home.join(".warp/notes/settings.json").is_file());
    assert!(home.join(".warp/logs/warpnotes-.log.2026-11-04").is_file());
    assert!(!home.join(".warp-notes").exists());
    let _ = std::fs::remove_dir_all(&home);
}
