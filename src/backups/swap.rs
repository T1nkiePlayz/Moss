//! Selective in-place restore helpers: validating `only` names and swapping just those top-level entries from a
//! staging directory into the server directory, keeping what was replaced under `data/removed`.
use super::archive::MANIFEST;
use crate::error::ApiError;
use anyhow::{Result, bail};
use std::path::Path;

/// `only` must be 1-100 plain top-level names: no separators, no `..`, nothing Moss keeps for itself.
pub fn validate_only(only: &[String]) -> Result<(), ApiError> {
    let bad = |m: &str| ApiError::bad("validation", m.to_string()).field("only", m);
    if only.is_empty() || only.len() > 100 { return Err(bad("list 1 to 100 top-level names")); }
    for n in only {
        if n.is_empty() || n.len() > 255 || n == "." || n.contains("..") || n.contains(['/', '\\', '\0']) { return Err(bad("names must be plain top-level entries (no separators or ..)")); }
        if n == ".moss" || n == MANIFEST { return Err(bad("that entry cannot be restored")); }
    }
    Ok(())
}

/// Move each `only` entry of `stage` into `dir`, first moving the existing one into `kept`. Every step is a
/// rename on the same filesystem; if any step fails everything done so far is rolled back.
pub fn swap_selected(stage: &Path, dir: &Path, kept: &Path, only: &[String]) -> Result<()> {
    std::fs::create_dir_all(kept)?;
    let mut done: Vec<(&String, bool)> = vec![]; // (name, had_previous)
    let result: Result<()> = (|| {
        for n in only {
            let (cur, new) = (dir.join(n), stage.join(n));
            if std::fs::symlink_metadata(&new).is_err() { bail!("{n} is not in the archive"); }
            let had = std::fs::symlink_metadata(&cur).is_ok();
            if had { std::fs::rename(&cur, kept.join(n))?; }
            done.push((n, had));
            std::fs::rename(&new, &cur)?;
        }
        Ok(())
    })();
    if result.is_err() {
        for (n, had) in done.into_iter().rev() {
            let (cur, new) = (dir.join(n), stage.join(n));
            if cur.exists() && !stage.join(n).exists() { let _ = std::fs::rename(&cur, &new); }
            if had { let _ = std::fs::rename(kept.join(n), &cur); }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(v: &[&str]) -> Vec<String> { v.iter().map(|s| s.to_string()).collect() }

    #[test]
    fn only_names_are_validated() {
        assert!(validate_only(&names(&["world", "world_nether", "server.properties"])).is_ok());
        for bad in [vec![], names(&[""]), names(&["a/b"]), names(&["../x"]), names(&[".."]), names(&["a\\b"]), names(&["/etc"]), names(&[".moss"]), names(&[".moss-manifest.json"]), names(&["x..y"]), names(&["a\0b"])] {
            assert!(validate_only(&bad).is_err(), "{bad:?}");
        }
        assert!(validate_only(&vec!["w".to_string(); 101]).is_err());
    }

    #[test]
    fn swap_replaces_only_selected_entries_and_keeps_the_old_ones() {
        let base = std::env::temp_dir().join(format!("moss-swap-{}", uuid::Uuid::new_v4()));
        let (stage, dir, kept) = (base.join("stage"), base.join("dir"), base.join("kept"));
        for d in [stage.join("world"), dir.join("world"), dir.join("plugins"), dir.join(".moss")] { std::fs::create_dir_all(d).unwrap(); }
        std::fs::write(stage.join("world/level.dat"), b"old-backup").unwrap();
        std::fs::write(dir.join("world/level.dat"), b"current").unwrap();
        std::fs::write(dir.join("plugins/p.jar"), b"keep me").unwrap();
        std::fs::write(stage.join("new.txt"), b"added").unwrap();
        swap_selected(&stage, &dir, &kept, &names(&["world", "new.txt"])).unwrap();
        assert_eq!(std::fs::read(dir.join("world/level.dat")).unwrap(), b"old-backup");
        assert_eq!(std::fs::read(dir.join("new.txt")).unwrap(), b"added");
        assert_eq!(std::fs::read(dir.join("plugins/p.jar")).unwrap(), b"keep me");
        assert_eq!(std::fs::read(kept.join("world/level.dat")).unwrap(), b"current");
        assert!(dir.join(".moss").is_dir());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_failed_swap_rolls_back() {
        let base = std::env::temp_dir().join(format!("moss-swap-{}", uuid::Uuid::new_v4()));
        let (stage, dir, kept) = (base.join("stage"), base.join("dir"), base.join("kept"));
        for d in [stage.join("world"), dir.join("world")] { std::fs::create_dir_all(d).unwrap(); }
        std::fs::write(stage.join("world/level.dat"), b"backup").unwrap();
        std::fs::write(dir.join("world/level.dat"), b"current").unwrap();
        // second name is missing from the stage: the first swap must be undone
        assert!(swap_selected(&stage, &dir, &kept, &names(&["world", "ghost"])).is_err());
        assert_eq!(std::fs::read(dir.join("world/level.dat")).unwrap(), b"current");
        assert_eq!(std::fs::read(stage.join("world/level.dat")).unwrap(), b"backup");
        let _ = std::fs::remove_dir_all(&base);
    }
}
