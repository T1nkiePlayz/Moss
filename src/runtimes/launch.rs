//! Launch arguments per runtime and the Moss-owned `user_jvm_args.txt` (Forge/NeoForge read it through `@`-argument
//! files). Pure functions: `lifecycle::command` resolves Java and calls them, so the exact argv is unit-tested here.
use crate::server::Srv;
use std::path::Path;

fn aikar_flags() -> Vec<String> {
    "-XX:+UseG1GC -XX:+ParallelRefProcEnabled -XX:MaxGCPauseMillis=200 -XX:+UnlockExperimentalVMOptions -XX:+DisableExplicitGC -XX:G1NewSizePercent=30 -XX:G1MaxNewSizePercent=40 -XX:G1HeapRegionSize=8M -XX:G1ReservePercent=20 -XX:G1HeapWastePercent=5 -XX:G1MixedGCCountTarget=4 -XX:InitiatingHeapOccupancyPercent=15 -XX:G1MixedGCLiveThresholdPercent=90 -XX:G1RSetUpdatingPauseTimePercent=5 -XX:SurvivorRatio=32 -XX:+PerfDisableSharedMem -XX:MaxTenuringThreshold=1 -Dusing.aikars.flags=https://mcflags.emc.gs -Daikars.new.flags=true"
        .split(' ').map(String::from).collect()
}

/// Heap flags plus the selected flags profile.
pub fn jvm_flags(s: &Srv) -> Vec<String> {
    let mut a = vec![format!("-Xms{}M", s.memory_mb), format!("-Xmx{}M", s.memory_mb)];
    if s.flags_profile == "balanced" { a.extend(aikar_flags()); } else { a.push("-XX:+UseG1GC".into()); }
    a
}

/// Server-relative path of the `@`-argument file the Forge/NeoForge installer generates.
pub fn unix_args_path(s: &Srv) -> Option<String> {
    let v = s.loader_version.as_deref().filter(|v| !v.is_empty())?;
    match s.runtime.as_str() {
        "neoforge" => Some(format!("libraries/net/neoforged/neoforge/{v}/unix_args.txt")),
        "forge" => Some(format!("libraries/net/minecraftforge/forge/{}-{v}/unix_args.txt", s.game_version)),
        _ => None,
    }
}

/// The jar the legacy Forge installer (1.12.2 to 1.16.5) leaves in the server directory.
pub fn legacy_forge_jar(s: &Srv) -> Option<String> {
    let v = s.loader_version.as_deref().filter(|v| !v.is_empty())?;
    (s.runtime == "forge" && super::modded::legacy_forge_game(&s.game_version)).then(|| format!("forge-{}-{v}.jar", s.game_version))
}

/// The argv for a Minecraft server process, given the Java binary. `Err` explains what is missing.
pub fn argv(java: &str, s: &Srv) -> Result<Vec<String>, String> {
    let mut a = vec![java.to_string()];
    match s.runtime.as_str() {
        "forge" if super::modded::legacy_forge_game(&s.game_version) => {
            let jar = legacy_forge_jar(s).ok_or_else(|| "forge has no recorded loader version; reinstall the server".to_string())?;
            a.extend(jvm_flags(s));
            a.extend(["-Dfile.encoding=UTF-8".into(), "-jar".into(), jar, "nogui".into()]);
        }
        "forge" | "neoforge" => {
            let args = unix_args_path(s).ok_or_else(|| format!("{} has no recorded loader version; reinstall the server", s.runtime))?;
            a.extend(["@user_jvm_args.txt".into(), format!("@{args}"), "nogui".into()]);
        }
        "quilt" => {
            a.extend(jvm_flags(s));
            a.extend(["-Dfile.encoding=UTF-8".into(), "-jar".into(), "quilt-server-launch.jar".into(), "nogui".into()]);
        }
        _ => {
            a.extend(jvm_flags(s));
            a.extend(["-Dfile.encoding=UTF-8".into(), "-jar".into(), "server.jar".into()]);
            if !s.is_proxy() { a.push("nogui".into()); }
        }
    }
    Ok(a)
}

/// Contents of the managed `user_jvm_args.txt`: one flag per line.
pub fn user_jvm_args(s: &Srv) -> String {
    let mut t = String::from("# Managed by Moss: rewritten on every start from the server's memory and flags settings.\n# Change them in the Moss UI; manual edits here are overwritten.\n");
    for f in jvm_flags(s).into_iter().chain(["-Dfile.encoding=UTF-8".to_string()]) { t.push_str(&f); t.push('\n'); }
    t
}

/// Write (or refresh) `user_jvm_args.txt` in the server directory. No-op for runtimes that do not read it.
pub fn write_user_jvm_args(dir: &Path, s: &Srv) -> std::io::Result<()> {
    if !matches!(s.runtime.as_str(), "forge" | "neoforge") || (s.runtime == "forge" && super::modded::legacy_forge_game(&s.game_version)) { return Ok(()); }
    crate::files::atomic_write(&dir.join("user_jvm_args.txt"), user_jvm_args(s).as_bytes())
}

/// Test fixture: a minimal installed server record.
#[cfg(test)]
pub fn test_srv(runtime: &str, game: &str, loader: Option<&str>, profile: &str) -> Srv {
    Srv { id: "x".into(), name: "x".into(), runtime: runtime.into(), game_version: game.into(), loader_version: loader.map(String::from), java_major: 21, port: 25565, memory_mb: 4096, memory_limit_mb: None, cpu_limit_pct: None, pids_limit: None, isolation: "process".into(), container_image: None, in_container: false,
        flags_profile: profile.into(), autostart: false, restart_enabled: false, restart_max: 3, eula_at: None, desired: "stopped".into(), installed: true, install_error: None,
        pid: None, pid_start: None, tags: vec![], notes: String::new(), custom_cmd: None, created_at: String::new() }
}

#[cfg(test)]
mod tests {
    use super::{test_srv as srv, *};

    #[test]
    fn neoforge_and_forge_use_argument_files() {
        let n = argv("/j/java", &srv("neoforge", "1.21.1", Some("21.1.172"), "balanced")).unwrap();
        assert_eq!(n, ["/j/java", "@user_jvm_args.txt", "@libraries/net/neoforged/neoforge/21.1.172/unix_args.txt", "nogui"]);
        let f = argv("/j/java", &srv("forge", "1.20.1", Some("47.2.0"), "balanced")).unwrap();
        assert_eq!(f, ["/j/java", "@user_jvm_args.txt", "@libraries/net/minecraftforge/forge/1.20.1-47.2.0/unix_args.txt", "nogui"]);
        assert!(argv("/j/java", &srv("forge", "1.20.1", None, "minimal")).is_err());
    }

    #[test]
    fn legacy_forge_launches_the_forge_jar_with_plain_jvm_flags() {
        let f = argv("/j/java8", &srv("forge", "1.12.2", Some("14.23.5.2860"), "minimal")).unwrap();
        assert_eq!(f, ["/j/java8", "-Xms4096M", "-Xmx4096M", "-XX:+UseG1GC", "-Dfile.encoding=UTF-8", "-jar", "forge-1.12.2-14.23.5.2860.jar", "nogui"]);
        let g = argv("/j/java8", &srv("forge", "1.16.5", Some("36.2.39"), "balanced")).unwrap();
        assert!(g.contains(&"-Daikars.new.flags=true".to_string()) && g[g.len() - 3..] == ["-jar", "forge-1.16.5-36.2.39.jar", "nogui"]);
        assert!(!g.iter().any(|a| a.starts_with('@')), "legacy Forge does not read argument files");
        assert!(argv("/j/java8", &srv("forge", "1.12.2", None, "minimal")).is_err());
        // 1.17 is the first modern version
        assert!(argv("/j/java", &srv("forge", "1.17.1", Some("37.1.1"), "minimal")).unwrap().contains(&"@user_jvm_args.txt".to_string()));
        assert_eq!(legacy_forge_jar(&srv("neoforge", "1.16.5", Some("1"), "minimal")), None);
    }

    #[test]
    fn legacy_forge_does_not_get_a_user_jvm_args_file() {
        let dir = std::env::temp_dir().join(format!("moss-jvm-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        write_user_jvm_args(&dir, &srv("forge", "1.16.5", Some("36.2.39"), "minimal")).unwrap();
        assert!(!dir.join("user_jvm_args.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn quilt_launches_the_quilt_server_launch_jar() {
        let q = argv("/j/java", &srv("quilt", "1.21.1", Some("0.26.4"), "minimal")).unwrap();
        assert_eq!(q, ["/j/java", "-Xms4096M", "-Xmx4096M", "-XX:+UseG1GC", "-Dfile.encoding=UTF-8", "-jar", "quilt-server-launch.jar", "nogui"]);
    }

    #[test]
    fn existing_runtimes_keep_their_exact_argv() {
        let p = argv("/j/java", &srv("paper", "1.21.1", None, "minimal")).unwrap();
        assert_eq!(p, ["/j/java", "-Xms4096M", "-Xmx4096M", "-XX:+UseG1GC", "-Dfile.encoding=UTF-8", "-jar", "server.jar", "nogui"]);
        let v = argv("/j/java", &srv("velocity", "3.4", None, "minimal")).unwrap();
        assert_eq!(v.last().unwrap(), "server.jar");
        let b = argv("/j/java", &srv("fabric", "1.21.1", Some("0.16"), "balanced")).unwrap();
        assert!(b.contains(&"-Daikars.new.flags=true".to_string()) && b[b.len() - 3..] == ["-jar", "server.jar", "nogui"]);
    }

    #[test]
    fn user_jvm_args_hold_memory_and_profile_one_flag_per_line() {
        let t = user_jvm_args(&srv("neoforge", "1.21.1", Some("21.1.172"), "minimal"));
        let flags: Vec<&str> = t.lines().filter(|l| !l.starts_with('#')).collect();
        assert_eq!(flags, ["-Xms4096M", "-Xmx4096M", "-XX:+UseG1GC", "-Dfile.encoding=UTF-8"]);
        assert!(user_jvm_args(&srv("forge", "1.20.1", Some("47.2.0"), "balanced")).contains("-XX:MaxGCPauseMillis=200\n"));
    }

    #[test]
    fn user_jvm_args_file_is_refreshed_only_for_forge_family() {
        let dir = std::env::temp_dir().join(format!("moss-jvm-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        write_user_jvm_args(&dir, &srv("paper", "1.21", None, "minimal")).unwrap();
        assert!(!dir.join("user_jvm_args.txt").exists());
        std::fs::write(dir.join("user_jvm_args.txt"), "-Xmx1M\n").unwrap();
        write_user_jvm_args(&dir, &srv("neoforge", "1.21.1", Some("21.1.1"), "minimal")).unwrap();
        assert!(std::fs::read_to_string(dir.join("user_jvm_args.txt")).unwrap().contains("-Xmx4096M"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
