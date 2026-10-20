//! Copying a server directory for a test run. Everything is a real copy (never a hard link: an installer writing
//! into the copy must not be able to touch the original), symlinks are skipped, and the live-only state
//! (.moss, logs, crash reports, caches, the session lock) is left behind. The world is optional because it can be huge.
use std::{io, path::{Path, PathBuf}};

/// Top-level entries that never belong in a copy.
const SKIP: &[&str] = &[".moss", "logs", "crash-reports", "cache", "session.lock", "hs_err_pid"];

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Stats { pub files: u64, pub bytes: u64 }

/// The world folders (the configured level, plus its nether and end folders) as top-level names.
pub fn world_dirs(server_dir: &Path) -> Vec<String> {
    let level = std::fs::read_to_string(server_dir.join("server.properties")).ok()
        .and_then(|r| r.lines().find_map(|l| l.strip_prefix("level-name=").map(|v| v.trim().to_string())))
        .filter(|l| !l.is_empty() && !l.contains(['/', '\\']) && !l.starts_with('.')).unwrap_or_else(|| "world".into());
    vec![format!("{level}_nether"), format!("{level}_the_end"), level]
}

fn skipped(name: &str, world: &[String], include_world: bool) -> bool {
    SKIP.iter().any(|s| name == *s || (*s == "hs_err_pid" && name.starts_with(s))) || (!include_world && world.iter().any(|w| w == name))
}

/// Walks the top level and everything below, calling `f(path, is_dir, len, rel)` for regular files and directories.
fn walk(root: &Path, include_world: bool, cancelled: &dyn Fn() -> bool, f: &mut dyn FnMut(&Path, bool, u64, &Path) -> io::Result<()>) -> io::Result<()> {
    let world = world_dirs(root);
    let mut stack: Vec<(PathBuf, PathBuf)> = vec![(root.to_path_buf(), PathBuf::new())];
    while let Some((dir, rel)) = stack.pop() {
        for e in std::fs::read_dir(&dir)? {
            let e = e?;
            if cancelled() { return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled")); }
            let name = e.file_name().to_string_lossy().to_string();
            if rel.as_os_str().is_empty() && skipped(&name, &world, include_world) { continue; }
            let t = e.file_type()?;
            if t.is_symlink() { continue; }
            let r = rel.join(&name);
            if t.is_dir() { f(&e.path(), true, 0, &r)?; stack.push((e.path(), r)); }
            else if t.is_file() { f(&e.path(), false, e.metadata()?.len(), &r)?; }
        }
    }
    Ok(())
}

/// Bytes a copy would take, with and without the world.
pub fn estimate(root: &Path) -> (Stats, u64) {
    let (mut base, mut with_world) = (Stats::default(), Stats::default());
    let never = || false;
    let _ = walk(root, false, &never, &mut |_, dir, len, _| { if !dir { base.files += 1; base.bytes += len; } Ok(()) });
    let _ = walk(root, true, &never, &mut |_, dir, len, _| { if !dir { with_world.files += 1; with_world.bytes += len; } Ok(()) });
    let extra = with_world.bytes.saturating_sub(base.bytes);
    (base, extra)
}

/// Copies `src` into the (new) directory `dst`.
pub fn copy_tree(src: &Path, dst: &Path, include_world: bool, cancelled: &dyn Fn() -> bool) -> io::Result<Stats> {
    std::fs::create_dir_all(dst)?;
    let mut st = Stats::default();
    walk(src, include_world, cancelled, &mut |path, dir, len, rel| {
        let to = dst.join(rel);
        if dir { std::fs::create_dir_all(&to) } else {
            if let Some(p) = to.parent() { std::fs::create_dir_all(p)?; }
            std::fs::copy(path, &to)?;
            st.files += 1; st.bytes += len;
            Ok(())
        }
    })?;
    Ok(st)
}

/// Picks a port in `[lo, hi]` that no Moss server uses and that can be bound right now.
pub async fn pick_port(range: (u16, u16), used: &[i64]) -> Option<i64> {
    for p in range.0..=range.1 {
        if used.contains(&(p as i64)) { continue; }
        if tokio::net::TcpListener::bind(("0.0.0.0", p)).await.is_ok() { return Some(p as i64); }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> PathBuf {
        let d = std::env::temp_dir().join(format!("moss-clone-{}", uuid::Uuid::now_v7()));
        for sub in ["mods", "config", "logs", "crash-reports", ".moss", "world/region", "world_nether", "myworld"] { std::fs::create_dir_all(d.join(sub)).unwrap(); }
        for (f, body) in [("server.jar", "jar"), ("mods/a.jar", "aaaa"), ("config/a.toml", "x=1"), ("logs/latest.log", "log"), ("crash-reports/c.txt", "c"), (".moss/pid", "1"),
                          ("world/region/r.mca", "0123456789"), ("world_nether/n.dat", "nn"), ("session.lock", "l"), ("hs_err_pid12.log", "e"), ("myworld/x", "w")] {
            std::fs::write(d.join(f), body).unwrap();
        }
        d
    }

    #[test]
    fn the_copy_leaves_live_state_and_the_world_behind_by_default() {
        let (src, dst) = (tree(), std::env::temp_dir().join(format!("moss-clone-dst-{}", uuid::Uuid::now_v7())));
        let s = copy_tree(&src, &dst, false, &|| false).unwrap();
        assert_eq!(s.files, 4, "server.jar, mods/a.jar, config/a.toml and myworld/x (only `world` is the level here)");
        assert!(dst.join("mods/a.jar").is_file() && dst.join("config/a.toml").is_file() && dst.join("server.jar").is_file());
        for gone in ["logs", "crash-reports", ".moss", "world", "world_nether", "session.lock", "hs_err_pid12.log"] { assert!(!dst.join(gone).exists(), "{gone} must not be copied"); }
        let _ = (std::fs::remove_dir_all(&src), std::fs::remove_dir_all(&dst));
    }

    #[test]
    fn including_the_world_copies_it() {
        let (src, dst) = (tree(), std::env::temp_dir().join(format!("moss-clone-dst-{}", uuid::Uuid::now_v7())));
        copy_tree(&src, &dst, true, &|| false).unwrap();
        assert_eq!(std::fs::read_to_string(dst.join("world/region/r.mca")).unwrap(), "0123456789");
        assert!(dst.join("world_nether/n.dat").is_file());
        assert!(!dst.join("logs").exists());
        let _ = (std::fs::remove_dir_all(&src), std::fs::remove_dir_all(&dst));
    }

    #[test]
    fn the_configured_level_name_decides_what_the_world_is() {
        let src = tree();
        std::fs::write(src.join("server.properties"), "level-name=myworld\n").unwrap();
        let dst = std::env::temp_dir().join(format!("moss-clone-dst-{}", uuid::Uuid::now_v7()));
        copy_tree(&src, &dst, false, &|| false).unwrap();
        assert!(!dst.join("myworld").exists() && dst.join("world/region/r.mca").is_file(), "`world` is an ordinary folder when the level is myworld");
        let (base, world) = estimate(&src);
        assert!(world >= 1 && base.files >= 4);
        let _ = (std::fs::remove_dir_all(&src), std::fs::remove_dir_all(&dst));
    }

    #[test]
    fn symlinks_are_not_followed() {
        let src = tree();
        let outside = std::env::temp_dir().join(format!("moss-clone-outside-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret"), "s").unwrap();
        std::os::unix::fs::symlink(&outside, src.join("linked")).unwrap();
        std::os::unix::fs::symlink(outside.join("secret"), src.join("secret-link")).unwrap();
        let dst = std::env::temp_dir().join(format!("moss-clone-dst-{}", uuid::Uuid::now_v7()));
        copy_tree(&src, &dst, true, &|| false).unwrap();
        assert!(!dst.join("linked").exists() && !dst.join("secret-link").exists());
        let _ = (std::fs::remove_dir_all(&src), std::fs::remove_dir_all(&dst), std::fs::remove_dir_all(&outside));
    }

    #[test]
    fn cancellation_stops_the_copy() {
        let (src, dst) = (tree(), std::env::temp_dir().join(format!("moss-clone-dst-{}", uuid::Uuid::now_v7())));
        let e = copy_tree(&src, &dst, false, &|| true).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::Interrupted);
        let _ = (std::fs::remove_dir_all(&src), std::fs::remove_dir_all(&dst));
    }

    #[tokio::test]
    async fn port_pick_skips_used_and_bound_ports() {
        let l = tokio::net::TcpListener::bind(("0.0.0.0", 0)).await.unwrap();
        let busy = l.local_addr().unwrap().port();
        let p = pick_port((busy, busy + 2), &[busy as i64 + 1]).await.unwrap();
        assert_eq!(p, busy as i64 + 2);
        assert!(pick_port((busy, busy), &[]).await.is_none());
    }
}
