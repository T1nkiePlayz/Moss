//! Server lifecycle. Game processes are launched detached (own session, stdin = a FIFO the game itself holds open,
//! stdout/stderr appended to a log file), so they are NOT children of the controller: killing or upgrading Moss never
//! stops a server, and on restart Moss re-attaches by PID + start-time and resumes tailing the log.
use crate::{platform, server::{self, Srv}, state::St};

mod command;
mod exit;
mod monitor;
mod reconcile;
mod start;
mod stop;

pub use monitor::spawn_background;
pub use reconcile::reconcile;
pub use start::{send_command, start};
pub use stop::{halt, kill, restart, stop};

/// Detect the resource-limit backend once at startup (creates `moss-servers` in the delegated cgroup, if any).
pub fn init_limits() {
    match platform::cgroup::backend() {
        Ok(_) => tracing::info!("resource limits: cgroup v2"),
        Err(e) => tracing::info!("resource limits are not enforced: {e}"),
    }
}

/// Called once a server's processes are gone: reads whether the kernel OOM-killed it, then removes its cgroup.
pub(crate) fn finish_cgroup(id: &str) -> bool {
    let Ok(cg) = platform::cgroup::backend() else { return false };
    let oom = cg.oom_kills(id) > 0;
    cg.remove(id);
    oom
}

/// Remove a server's cgroup (used when the server record is deleted).
pub fn cleanup_cgroup(id: &str) { finish_cgroup(id); }

pub fn looks_ready(line: &str) -> bool { line.contains("Done (") && line.contains("s)") }

pub fn mark_ready(st: &St, id: &str) {
    let changed = {
        let mut live = st.live.lock().unwrap();
        let l = live.entry(id.to_string()).or_default();
        if l.ready || !l.alive { false } else { l.ready = true; l.detail = None; true }
    };
    if changed {
        let (st, id) = (st.clone(), id.to_string());
        tokio::spawn(async move {
            st.timeline(&id, "ready", "").await;
            if let Ok(s) = server::get(&st, &id).await { server::note_state(&st, &s); }
        });
    }
}

pub fn process_alive(s: &Srv) -> bool {
    match (s.pid, s.pid_start) {
        (Some(p), Some(t)) => platform::pid_start(p) == Some(t as u64),
        _ => false,
    }
}

pub(crate) fn set_detail(st: &St, id: &str, d: &str) { st.live.lock().unwrap().entry(id.to_string()).or_default().detail = Some(d.to_string()); }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn readiness_line_detection() {
        assert!(looks_ready("[22:30:01 INFO]: Done (4.412s)! For help, type \"help\""));
        assert!(looks_ready("[10:00:00] [Server thread/INFO]: Done (1.204s)! For help"));
        assert!(!looks_ready("Done loading plugins") && !looks_ready("[INFO]: Preparing level"));
    }
}

