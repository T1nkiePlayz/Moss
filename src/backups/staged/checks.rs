//! Pre-promotion checks on a staged restore: is what was extracted a server Moss can start (its jar or launch files)
//! and does it contain the worlds the backup says it has. Pure filesystem reads; blocking.
use crate::{runtimes::launch::unix_args_path, server::Srv};
use serde_json::{Value, json};
use std::path::Path;

pub struct Check { pub id: &'static str, pub ok: bool, pub blocking: bool, pub message: String }

impl Check {
    pub fn json(&self) -> Value { json!({"id": self.id, "ok": self.ok, "blocking": self.blocking, "message": self.message}) }
}

/// The files a server of this runtime cannot start without (server-relative). Empty for custom and simulated servers.
pub fn required_files(s: &Srv) -> Vec<String> {
    if let Some(g) = s.game() { return vec![g.main_binary().to_string()]; }
    match s.runtime.as_str() {
        "custom" | "sim" => vec![],
        "forge" | "neoforge" => unix_args_path(s).into_iter().collect(),
        "quilt" => vec!["quilt-server-launch.jar".into()],
        _ => vec!["server.jar".into()],
    }
}

fn nonempty_dir(p: &Path) -> bool { std::fs::read_dir(p).is_ok_and(|mut d| d.next().is_some()) }

/// `only`: top-level names being restored (partial restore). `worlds`: the world folders the backup recorded.
pub fn run(stage: &Path, s: &Srv, only: Option<&[String]>, worlds: &[String]) -> Vec<Check> {
    let mut out = vec![];
    let pass = |id, message: String| Check { id, ok: true, blocking: true, message };
    let fail = |id, message: String| Check { id, ok: false, blocking: true, message };
    if !nonempty_dir(stage) { return vec![fail("content", "The restored copy is empty.".into())]; }
    match only {
        None => {
            let need = required_files(s);
            let missing: Vec<&String> = need.iter().filter(|f| !stage.join(f).is_file()).collect();
            if need.is_empty() { out.push(pass("jar", "This server type has no fixed server file to check.".into())); }
            else if missing.is_empty() { out.push(pass("jar", format!("Server file present: {}.", need.join(", ")))); }
            else { out.push(fail("jar", format!("The backup does not contain the server file {}; the server would not start after this restore.", missing.iter().map(|m| m.as_str()).collect::<Vec<_>>().join(", ")))); }
        }
        Some(only) => {
            let gone: Vec<&String> = only.iter().filter(|n| std::fs::symlink_metadata(stage.join(n)).is_err()).collect();
            if gone.is_empty() { out.push(pass("entries", format!("All {} selected item(s) were restored.", only.len()))); }
            else { out.push(fail("entries", format!("Missing from the restored copy: {}.", gone.iter().map(|m| m.as_str()).collect::<Vec<_>>().join(", ")))); }
        }
    }
    let wanted: Vec<&String> = worlds.iter().filter(|w| only.is_none_or(|o| o.contains(w))).collect();
    if wanted.is_empty() {
        out.push(Check { id: "world", ok: true, blocking: false, message: "This backup contains no world folder.".into() });
    } else {
        let bad: Vec<&&String> = wanted.iter().filter(|w| !stage.join(w.as_str()).join("level.dat").is_file()).collect();
        if bad.is_empty() { out.push(pass("world", format!("World data present: {}.", wanted.iter().map(|w| w.as_str()).collect::<Vec<_>>().join(", ")))); }
        else { out.push(fail("world", format!("The world {} has no level.dat in the restored copy; it may be damaged.", bad.iter().map(|w| w.as_str()).collect::<Vec<_>>().join(", ")))); }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtimes::launch::test_srv;

    fn stage(files: &[&str]) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("moss-chk-{}", uuid::Uuid::new_v4()));
        for f in files { let p = d.join(f); std::fs::create_dir_all(p.parent().unwrap()).unwrap(); std::fs::write(p, b"x").unwrap(); }
        std::fs::create_dir_all(&d).unwrap();
        d
    }
    fn ok(c: &[Check]) -> bool { c.iter().all(|c| c.ok || !c.blocking) }
    fn w(v: &[&str]) -> Vec<String> { v.iter().map(|s| s.to_string()).collect() }

    #[test]
    fn a_full_restore_needs_the_server_jar_and_the_worlds() {
        let paper = test_srv("paper", "1.21", None, "balanced");
        assert!(ok(&run(&stage(&["server.jar", "world/level.dat", "server.properties"]), &paper, None, &w(&["world"]))));
        let c = run(&stage(&["world/level.dat"]), &paper, None, &w(&["world"]));
        assert!(!ok(&c) && c[0].message.contains("server.jar"));
        let c = run(&stage(&["server.jar", "world/region/r.mca"]), &paper, None, &w(&["world"]));
        assert!(!ok(&c) && c.iter().any(|c| c.id == "world" && !c.ok));
        // a server that never generated a world has nothing to check there
        assert!(ok(&run(&stage(&["server.jar"]), &paper, None, &[])));
        assert!(!ok(&run(&stage(&[]), &paper, None, &[])));
        // game backups are judged on the game's own binary, not server.jar
        let bedrock = test_srv("bedrock", "1.21", None, "balanced");
        assert_eq!(required_files(&bedrock), ["bedrock_server"]);
        assert!(ok(&run(&stage(&["bedrock_server", "worlds/Bedrock level/level.dat"]), &bedrock, None, &[])));
        let c = run(&stage(&["server.jar"]), &bedrock, None, &[]);
        assert!(!ok(&c) && c[0].message.contains("bedrock_server"));
        assert!(ok(&run(&stage(&["valheim_server.x86_64"]), &test_srv("valheim", "latest", None, "balanced"), None, &[])));
    }

    #[test]
    fn runtime_specific_launch_files() {
        assert_eq!(required_files(&test_srv("quilt", "1.21", None, "x")), vec!["quilt-server-launch.jar"]);
        assert_eq!(required_files(&test_srv("neoforge", "1.21", Some("21.1.5"), "x")), vec!["libraries/net/neoforged/neoforge/21.1.5/unix_args.txt"]);
        assert!(required_files(&test_srv("sim", "1", None, "x")).is_empty() && required_files(&test_srv("custom", "1", None, "x")).is_empty());
        assert!(ok(&run(&stage(&["a.txt"]), &test_srv("custom", "1", None, "x"), None, &[])));
    }

    #[test]
    fn partial_restores_check_only_what_was_selected() {
        let paper = test_srv("paper", "1.21", None, "balanced");
        let only = w(&["world_nether"]);
        assert!(ok(&run(&stage(&["world_nether/level.dat"]), &paper, Some(&only), &w(&["world", "world_nether"]))), "no jar needed for a world-only restore");
        assert!(!ok(&run(&stage(&["world_nether/x"]), &paper, Some(&only), &w(&["world_nether"]))));
        assert!(!ok(&run(&stage(&["other"]), &paper, Some(&only), &[])));
    }
}
