//! Background loops: liveness/readiness/restart supervision every second, resource sampling every five.
use super::{exit::handle_exit, mark_ready, process_alive, set_detail, start::start};
use crate::{platform, server, state::{Players, Sample, St}};
use std::{collections::HashMap, time::{Duration, Instant}};

pub fn spawn_background(st: &St) {
    super::init_limits();
    supervise(st);
    sample(st);
}

fn supervise(st: &St) {
    let st1 = st.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        let mut n = 0u64;
        loop {
            tick.tick().await;
            n += 1;
            let Ok(servers) = server::list(&st1).await else { continue };
            for s in servers.iter().filter(|s| s.installed) {
                let alive = process_alive(s);
                let (was, ready, restart_due, started) = {
                    let live = st1.live.lock().unwrap();
                    let l = live.get(&s.id);
                    (l.is_some_and(|l| l.alive), l.is_some_and(|l| l.ready), l.and_then(|l| l.restart_at).is_some_and(|t| t <= Instant::now()), l.and_then(|l| l.started))
                };
                if !alive && (s.pid.is_some() || was) { handle_exit(&st1, &s.id).await; }
                if alive && !was { st1.live.lock().unwrap().entry(s.id.clone()).or_default().alive = true; }
                if alive && !ready && n.is_multiple_of(3) && started.is_none_or(|t| t.elapsed().as_secs() > 4) {
                    let ok = if s.is_mc() || s.runtime == "sim" { crate::mcping::ping(s.port as u16).await.is_ok() } else { true };
                    if ok { mark_ready(&st1, &s.id); }
                }
                if restart_due {
                    st1.live.lock().unwrap().entry(s.id.clone()).or_default().restart_at = None;
                    st1.timeline(&s.id, "restart", "automatic restart after crash").await;
                    if let Err(e) = start(&st1, s, "system").await { set_detail(&st1, &s.id, &format!("automatic restart failed: {}", e.message)); }
                }
                server::note_state(&st1, s);
            }
        }
    });
}

fn sample(st: &St) {
    let st2 = st.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        loop {
            tick.tick().await;
            let Ok(servers) = server::list(&st2).await else { continue };
            // One scan of the process table for every server, plus host CPU and cgroup memory. On macOS these spawn
            // `ps`, so none of it runs on an async worker thread.
            let (st3, ids) = (st2.clone(), servers.iter().filter(|s| s.pid.is_some()).map(|s| s.id.clone()).collect::<Vec<_>>());
            let Ok((usage, cg_mem)) = tokio::task::spawn_blocking(move || {
                st3.host.lock().unwrap().sample();
                let cg = platform::cgroup::backend().ok();
                let mem: HashMap<String, u64> = ids.into_iter().filter_map(|id| Some((id.clone(), cg?.memory_current(&id)?))).collect();
                (platform::group_usage_all(), mem)
            }).await else { continue };
            for s in servers.iter().filter(|s| s.installed && s.pid.is_some()) {
                let Some(pid) = s.pid else { continue };
                let Some(&(cpu_secs, rss)) = usage.get(&pid) else { continue };
                let rss = cg_mem.get(&s.id).copied().unwrap_or(rss); // cgroup memory (incl. page cache of the game) when present
                let (ready, cpu) = {
                    let mut live = st2.live.lock().unwrap();
                    let l = live.entry(s.id.clone()).or_default();
                    let cpu = l.cpu_prev.map(|(prev, at)| 100.0 * (cpu_secs - prev).max(0.0) / at.elapsed().as_secs_f64().max(0.1));
                    l.cpu_prev = Some((cpu_secs, Instant::now()));
                    if let Some(c) = cpu { l.cpu = Some((c * 10.0).round() / 10.0); }
                    l.mem = Some(rss);
                    (l.ready, l.cpu)
                };
                let mut players = None;
                if ready && (s.is_mc() || s.runtime == "sim")
                    && let Ok(p) = crate::mcping::ping(s.port as u16).await {
                        players = Some(p.online);
                        let mut live = st2.live.lock().unwrap();
                        let l = live.entry(s.id.clone()).or_default();
                        l.players = Some(Players { online: p.online, max: p.max, sample: p.sample });
                        l.motd = Some(p.motd);
                    }
                {
                    let mut stats = st2.stats.lock().unwrap();
                    let q = stats.entry(s.id.clone()).or_default();
                    q.push_back(Sample { t: chrono::Utc::now().timestamp_millis(), cpu: cpu.unwrap_or(0.0), mem: rss, players });
                    while q.len() > 720 { q.pop_front(); }
                }
                st2.emit("server", server::view(s, &st2)); // `s` is this iteration's fresh row: no second query
            }
        }
    });
}

