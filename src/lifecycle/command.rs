//! The exact argument vector used to launch a server (never a shell string).
use crate::{error::{ApiError, ApiResult}, runtimes::launch, server::Srv, state::St};

/// The exact argument vector (never a shell string) used to launch a server. Forge and NeoForge also get their
/// Moss-managed `user_jvm_args.txt` refreshed here, so every start picks up the current memory and flags settings.
pub fn build_command(st: &St, s: &Srv) -> ApiResult<Vec<String>> {
    match s.runtime.as_str() {
        "sim" => Ok(vec![std::env::current_exe()?.to_string_lossy().into(), "sim-server".into(), "--port".into(), s.port.to_string()]),
        "custom" => s.custom_cmd.clone().filter(|c| !c.is_empty()).ok_or_else(|| ApiError::bad("custom.command", "custom server has no command")),
        _ => {
            let java = crate::java::find(st, s.java_major as u32).ok_or_else(|| ApiError::conflict("java.missing", format!("Java {} is not installed. Install it from Settings → Java (Moss can download a managed Temurin runtime).", s.java_major)))?;
            let a = launch::argv(&java.to_string_lossy(), s).map_err(|m| ApiError::conflict("server.not_installed", m))?;
            launch::write_user_jvm_args(&s.dir(st), s)?;
            Ok(a)
        }
    }
}
