//! Book-keeping after a process is gone.
use super::process_alive;
use crate::{server, state::St};
use std::time::{Duration, Instant};


/// Book-keeping after a process is gone: clean stop vs crash, restart policy with backoff and a crash-loop breaker.
pub async fn handle_exit(st: &St, id: &str) {
    {
        let mut live = st.live.lock().unwrap();
        let l = live.entry(id.to_string()).or_default();
        if l.exiting { return; }
        l.exiting = true;
    }
    let Ok(s) = server::get(st, id).await else { return };
    if s.pid.is_none() || process_alive(&s) { st.live.lock().unwrap().entry(id.to_string()).or_default().exiting = false; return; }
    let oom = super::finish_cgroup(id);
    if oom { crate::console::push(&crate::console::hub(st, id), "\x1b[31m[moss] the server was killed by the memory limit\x1b[0m".into()); }
    let code = std::fs::read_to_string(s.moss_dir(st).join("exit")).ok().and_then(|c| c.trim().parse::<i32>().ok());
    server::save_pid(st, id, None, None, Some(code.map_or("unknown (killed)".into(), |c| c.to_string()))).await;
    let stopping = { let l = st.live.lock().unwrap(); l.get(id).is_some_and(|l| l.stopping) };
    let clean = stopping || s.desired == "stopped";
    let mut restart_in = None;
    let mut audit_msg = None;
    {
        let mut live = st.live.lock().unwrap();
        let l = live.entry(id.to_string()).or_default();
        l.alive = false; l.ready = false; l.stopping = false; l.players = None; l.cpu = None; l.mem = None; l.started = None;
        if clean { l.crashed = false; l.detail = None; }
        else {
            l.crashed = true;
            l.crashes.retain(|t| t.elapsed() < Duration::from_secs(600));
            l.crashes.push(Instant::now());
            let n = l.crashes.len() as i64;
            let why = if oom { "killed by the memory limit".to_string() } else { code.map_or("killed".to_string(), |c| format!("exit code {c}")) };
            if s.restart_enabled && n <= s.restart_max {
                let delay = (2u64.pow(n as u32)).min(60);
                l.restart_at = Some(Instant::now() + Duration::from_secs(delay));
                l.detail = Some(format!("crashed ({why}); restarting in {delay}s"));
                restart_in = Some(delay);
            } else {
                l.detail = Some(format!("crashed ({why}); {n} crashes in 10 min — automatic restart disabled"));
            }
            audit_msg = Some(l.detail.clone().unwrap());
        }
        l.exiting = false;
    }
    if clean {
        st.timeline(id, "stop", &code.map_or(String::new(), |c| format!("exit {c}"))).await;
    } else {
        st.timeline(id, "crash", audit_msg.as_deref().unwrap_or("")).await;
        st.audit("system", "server.crash", &s.name, audit_msg.as_deref().unwrap_or(""), Some(id)).await;
        if restart_in.is_none() { server::set_desired(st, id, "stopped").await; }
    }
    if let Ok(fresh) = server::get(st, id).await { server::note_state(st, &fresh); }
}

