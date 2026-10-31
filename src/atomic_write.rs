//! The atomic file replace the note writer rides: unique temp file + rename
//! in the target directory, so a crash mid-write never leaves a truncated
//! note behind.

use std::path::Path;

/// Replace `path` with `contents` atomically: write a uniquely named temp file
/// next to the target, then rename over it (POSIX rename / Win32 replace —
/// atomic on the same volume).
pub fn write_atomically(path: &Path, contents: &str) -> anyhow::Result<()> {
    use anyhow::Context;
    if let Some(dir) = path.parent()
        && !dir.as_os_str().is_empty()
    {
        std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    }
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".into());
    let tmp = path.with_file_name(format!("{file_name}.{}.tmp", uuid_suffix()));
    std::fs::write(&tmp, contents).with_context(|| format!("write {}", tmp.display()))?;
    if let Err(err) = std::fs::rename(&tmp, path) {
        // a destination held open without FILE_SHARE_DELETE (an editor) —
        // drop it and retry once; if the retry also fails, keep the temp
        // file: with the old file delete-pending it is the only surviving
        // copy of the new contents, so the error names it instead of
        // deleting it
        if std::fs::remove_file(path).is_ok() && std::fs::rename(&tmp, path).is_ok() {
            return Ok(());
        }
        return Err(err).with_context(|| {
            format!(
                "rename {} -> {} (new contents kept at {})",
                tmp.display(),
                path.display(),
                tmp.display()
            )
        });
    }
    Ok(())
}

fn uuid_suffix() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos:x}-{}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_replaces_and_cleans_up() {
        let dir = std::env::temp_dir().join(format!("warp-notes-atomic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("note.md");
        write_atomically(&target, "first").unwrap();
        write_atomically(&target, "second").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "second");
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "temp files left behind: {leftovers:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
