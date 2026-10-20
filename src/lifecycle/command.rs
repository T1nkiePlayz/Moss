//! The exact argument vector used to launch a server (never a shell string).
use crate::{error::{ApiError, ApiResult}, server::Srv, state::St};

fn aikar_flags() -> Vec<String> {
    "-XX:+UseG1GC -XX:+ParallelRefProcEnabled -XX:MaxGCPauseMillis=200 -XX:+UnlockExperimentalVMOptions -XX:+DisableExplicitGC -XX:G1NewSizePercent=30 -XX:G1MaxNewSizePercent=40 -XX:G1HeapRegionSize=8M -XX:G1ReservePercent=20 -XX:G1HeapWastePercent=5 -XX:G1MixedGCCountTarget=4 -XX:InitiatingHeapOccupancyPercent=15 -XX:G1MixedGCLiveThresholdPercent=90 -XX:G1RSetUpdatingPauseTimePercent=5 -XX:SurvivorRatio=32 -XX:+PerfDisableSharedMem -XX:MaxTenuringThreshold=1 -Dusing.aikars.flags=https://mcflags.emc.gs -Daikars.new.flags=true"
        .split(' ').map(String::from).collect()
}

/// The exact argument vector (never a shell string) used to launch a server.
pub fn build_command(st: &St, s: &Srv) -> ApiResult<Vec<String>> {
    match s.runtime.as_str() {
        "sim" => Ok(vec![std::env::current_exe()?.to_string_lossy().into(), "sim-server".into(), "--port".into(), s.port.to_string()]),
        "custom" => s.custom_cmd.clone().filter(|c| !c.is_empty()).ok_or_else(|| ApiError::bad("custom.command", "custom server has no command")),
        _ => {
            let java = crate::java::find(st, s.java_major as u32).ok_or_else(|| ApiError::conflict("java.missing", format!("Java {} is not installed. Install it from Settings → Java (Moss can download a managed Temurin runtime).", s.java_major)))?;
            let mut a = vec![java.to_string_lossy().to_string(), format!("-Xms{}M", s.memory_mb), format!("-Xmx{}M", s.memory_mb)];
            if s.flags_profile == "balanced" { a.extend(aikar_flags()); } else { a.push("-XX:+UseG1GC".into()); }
            a.extend(["-Dfile.encoding=UTF-8".into(), "-jar".into(), "server.jar".into()]);
            if !s.is_proxy() { a.push("nogui".into()); }
            Ok(a)
        }
    }
}
