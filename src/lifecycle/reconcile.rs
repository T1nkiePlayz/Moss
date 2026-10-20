//! Startup reconciliation: re-attach to servers that survived, account for ones that died, autostart.
use super::{exit::handle_exit, process_alive, set_detail, start::start};
use crate::{server, state::St};
use std::time::{Duration, Instant};

/// Called once at startup: re-attach to servers that survived, account for ones that died while we were down, autostart.
pub async fn reconcile(st: &St) -> anyhow::Result<()> {
    let servers = server::list(st).await?;
    let mut to_start = vec![];
    for s in servers.iter().filter(|s| s.installed) {
        // A container server is re-attached by name + inspect (the PID of a container means nothing to a new controller).
        let (alive, unknown) = if s.in_container {
            let p = super::ctr::probe(st, s).await;
            (p.unwrap_or(true), p.is_none())
        } else { (process_alive(s), false) };
        if alive {
            let started = st.db.call({ let id = s.id.clone(); move |c| c.query_row("SELECT started_at FROM servers WHERE id=?", [id], |r| r.get::<_, Option<String>>(0)) }).await.ok().flatten();
            let age = started.and_then(|t| chrono::DateTime::parse_from_rfc3339(&t).ok()).map(|t| (chrono::Utc::now() - t.with_timezone(&chrono::Utc)).num_seconds().max(0) as u64).unwrap_or(60);
            let ready = crate::mcping::ping(s.port as u16).await.is_ok() || !s.is_mc() && s.runtime != "sim";
            {
                let mut live = st.live.lock().unwrap();
                let l = live.entry(s.id.clone()).or_default();
                l.alive = true; l.started = Some(Instant::now() - Duration::from_secs(age));
                l.ready = ready;
            }
            if unknown { set_detail(st, &s.id, "the container engine is not reachable right now; Moss will pick the server up when it is"); }
            crate::console::start_tailer(st, s.id.clone(), s.moss_dir(st).join("console.log"), true);
            st.timeline(&s.id, "reattach", "controller restarted; server kept running").await;
            tracing::info!(server = %s.name, pid = ?s.pid, "re-attached to running server");
        } else {
            if s.pid.is_some() { handle_exit(st, &s.id).await; } else { super::cleanup_cgroup(&s.id); }
            if s.desired == "running" || s.autostart { to_start.push(s.clone()); }
        }
    }
    let st2 = st.clone();
    tokio::spawn(async move {
        for s in to_start {
            let crashed = st2.live.lock().unwrap().get(&s.id).is_some_and(|l| l.crashed);
            if crashed { continue; } // crash policy already scheduled/declined this
            if let Err(e) = start(&st2, &s, "system").await { tracing::warn!(server = %s.name, "autostart failed: {}", e.message); set_detail(&st2, &s.id, &format!("autostart failed: {}", e.message)); }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });
    Ok(())
}

