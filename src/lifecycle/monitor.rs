//! Background loops: liveness/readiness/restart supervision every second, resource sampling every five.
use super::{exit::handle_exit, mark_ready, process_alive, set_detail, start::start};
use crate::{platform, server, state::{Players, Sample, St}};
use std::{collections::HashMap, time::{Duration, Instant}};

pub fn spawn_background(st: &St) {
    super::init_limits();
    supervise(st);
    sample(st);
}

/// Servers whose automatic restart task is still running.
static PENDING_RESTARTS: std::sync::Mutex<Option<std::collections::HashSet<String>>> = std::sync::Mutex::new(None);

struct Pending(String);
impl Drop for Pending {
    fn drop(&mut self) { if let Some(p) = PENDING_RESTARTS.lock().unwrap_or_else(|e| e.into_inner()).as_mut() { p.remove(&self.0); } }
}

fn supervise(st: &St) {
    let st1 = st.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut n = 0u64;
        loop {
            tick.tick().await;
            n += 1;
            let Ok(servers) = server::list_all(&st1).await else { continue };
            if n.is_multiple_of(2) { super::ctr::refresh(&st1, &servers).await; } // one `ps` for every container server
            let mut probe = vec![];
            let mut game_ticks = vec![];
            for s in servers.iter().filter(|s| s.installed) {
                let alive = process_alive(s);
                let (was, ready, restart_due, started) = {
                    let live = st1.live.lock().unwrap();
                    let l = live.get(&s.id);
                    (l.is_some_and(|l| l.alive), l.is_some_and(|l| l.ready), l.and_then(|l| l.restart_at).is_some_and(|t| t <= Instant::now()), l.and_then(|l| l.started))
                };
                if !alive && (s.pid.is_some() || was) { handle_exit(&st1, &s.id).await; }
                if alive && !was { st1.live.lock().unwrap().entry(s.id.clone()).or_default().alive = true; }
                if let Some(g) = s.game() {
                    // Games: readiness from the console log, A2S or a fallback timer; players from the log or A2S.
                    if alive { game_ticks.push((s, g)); } else { crate::games::track::forget(&s.id); }
                } else if alive && !ready && n.is_multiple_of(3) && started.is_none_or(|t| t.elapsed().as_secs() > 4) {
                    if s.is_mc() || s.runtime == "sim" { probe.push(s); } else { mark_ready(&st1, &s.id); }
                }
                // Run the restart on its own task so a slow start does not stall supervising every other server; at most
                // one per server at a time (a due restart waits for the next tick while one is still pending).
                if restart_due && PENDING_RESTARTS.lock().unwrap().get_or_insert_with(Default::default).insert(s.id.clone()) {
                    st1.live.lock().unwrap().entry(s.id.clone()).or_default().restart_at = None;
                    let (st2, s2) = (st1.clone(), s.clone());
                    tokio::spawn(async move {
                        let _pending = Pending(s2.id.clone()); // cleared even if the task panics
                        st2.timeline(&s2.id, "restart", "automatic restart after crash").await;
                        // Stop pressed while this task was queued (before `starting` was set) cancels the restart
                        if server::get(&st2, &s2.id).await.is_ok_and(|f| f.desired != "running") { return; }
                        if let Err(e) = start(&st2, &s2, "system").await { set_detail(&st2, &s2.id, &format!("automatic restart failed: {}", e.message)); crate::notify::restart_failed(&st2, &s2, &e.message).await; }
                    });
                }
                server::note_state(&st1, s);
            }
            futures_util::future::join_all(game_ticks.iter().map(|(s, g)| crate::games::track::tick(&st1, s, g, n))).await;
            // Readiness pings take up to 1.5s each; run them together so booting servers don't delay supervising the rest.
            let pings = futures_util::future::join_all(probe.iter().map(|s| crate::mcping::ping(s.port as u16))).await;
            for (s, p) in probe.into_iter().zip(pings) {
                if p.is_ok() { mark_ready(&st1, &s.id); server::note_state(&st1, s); }
            }
        }
    });
}

fn sample(st: &St) {
    let st2 = st.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut n = 0u64;
        loop {
            tick.tick().await;
            n += 1;
            let Ok(servers) = server::list_all(&st2).await else { continue };
            // One scan of the process table for every server, plus host CPU and cgroup memory. On macOS these spawn
            // `ps`, so none of it runs on an async worker thread.
            let (st3, ids) = (st2.clone(), servers.iter().filter(|s| s.pid.is_some() && !s.in_container).map(|s| s.id.clone()).collect::<Vec<_>>());
            // Container servers: one batched `stats --no-stream` for all of them.
            let cstats = super::ctr::stats(&st2, servers.iter().filter(|s| s.in_container && process_alive(s)).map(|s| crate::container::container_name(&s.id)).collect()).await;
            let Ok((usage, cg_mem)) = tokio::task::spawn_blocking(move || {
                st3.host.lock().unwrap().sample();
                let cg = platform::cgroup::backend().ok();
                let mem: HashMap<String, u64> = ids.into_iter().filter_map(|id| Some((id.clone(), cg?.memory_current(&id)?))).collect();
                (platform::group_usage_all(), mem)
            }).await else { continue };
            // A server deleted after `list_all` above may have had a sample re-added; drop history for servers that are gone.
            st2.stats.lock().unwrap().retain(|id, _| servers.iter().any(|s| &s.id == id));
            // Player pings for every ready game server at once rather than up to 1.5s each in turn.
            let pinged: Vec<&crate::server::Srv> = {
                let live = st2.live.lock().unwrap();
                servers.iter().filter(|s| s.installed && s.pid.is_some() && (s.is_mc() || s.runtime == "sim") && live.get(&s.id).is_some_and(|l| l.ready)).collect()
            };
            let a2s_games: Vec<&crate::server::Srv> = {
                let live = st2.live.lock().unwrap();
                servers.iter().filter(|s| s.installed && s.pid.is_some() && s.game().is_some_and(|g| matches!(g.players, crate::games::PlayerSource::A2s(_))) && live.get(&s.id).is_some_and(|l| l.ready)).collect()
            };
            let mut game_players = crate::games::track::a2s_players(&a2s_games).await;
            // RCON and HTTP-API games (Palworld, Zomboid, Satisfactory) are polled about every 30 s.
            if n.is_multiple_of(6) {
                let admin_games: Vec<&crate::server::Srv> = {
                    let live = st2.live.lock().unwrap();
                    servers.iter().filter(|s| s.installed && s.pid.is_some() && s.game().is_some_and(|g| matches!(g.players, crate::games::PlayerSource::Rcon | crate::games::PlayerSource::Api)) && live.get(&s.id).is_some_and(|l| l.ready)).collect()
                };
                game_players.extend(crate::games::admin::poll_players(&st2, &admin_games).await);
            }
            // a list ban on an admin-channel game can only land while that player is connected
            for (id, p) in &game_players { crate::playerlists::players_seen(&st2, id, &p.sample); }
            let mut pings: HashMap<String, _> = pinged.iter().map(|s| s.id.clone()).zip(futures_util::future::join_all(pinged.iter().map(|s| crate::mcping::ping(s.port as u16))).await).collect();
            for s in servers.iter().filter(|s| s.installed && s.pid.is_some()) {
                let Some(pid) = s.pid else { continue };
                let from_engine = s.in_container.then(|| cstats.get(&crate::container::container_name(&s.id)).copied());
                let (cpu_secs, rss) = match from_engine {
                    Some(Some(c)) => (0.0, c.mem_bytes),
                    Some(None) => continue,
                    None => {
                        let Some(&(cpu_secs, rss)) = usage.get(&pid) else { continue };
                        (cpu_secs, cg_mem.get(&s.id).copied().unwrap_or(rss)) // cgroup memory (incl. page cache of the game) when present
                    }
                };
                let (ready, cpu) = {
                    let mut live = st2.live.lock().unwrap();
                    let l = live.entry(s.id.clone()).or_default();
                    let cpu = match from_engine {
                        Some(Some(c)) => Some(c.cpu_pct), // the engine already reports a percentage
                        _ => l.cpu_prev.map(|(prev, at)| 100.0 * (cpu_secs - prev).max(0.0) / at.elapsed().as_secs_f64().max(0.1)),
                    };
                    l.cpu_prev = Some((cpu_secs, Instant::now()));
                    if let Some(c) = cpu { l.cpu = Some((c * 10.0).round() / 10.0); }
                    l.mem = Some(rss);
                    (l.ready, l.cpu)
                };
                let mut players = None;
                if ready && (s.is_mc() || s.runtime == "sim")
                    && let Some(Ok(p)) = pings.remove(&s.id) {
                        players = Some(p.online);
                        let mut live = st2.live.lock().unwrap();
                        let l = live.entry(s.id.clone()).or_default();
                        l.players = Some(Players { online: p.online, max: p.max, sample: p.sample });
                        l.motd = Some(p.motd);
                    }
                if s.game().is_some_and(|g| g.tracks_players()) {
                    let mut live = st2.live.lock().unwrap();
                    let l = live.entry(s.id.clone()).or_default();
                    if let Some(p) = game_players.remove(&s.id) { l.players = Some(p); }
                    if l.ready { players = l.players.as_ref().map(|p| p.online); }
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

