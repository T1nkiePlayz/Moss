//! Server lifecycle. Game processes are launched detached (own session, stdin = a FIFO the game itself holds open,
//! stdout/stderr appended to a log file), so they are NOT children of the controller: killing or upgrading Moss never
//! stops a server, and on restart Moss re-attaches by PID + start-time and resumes tailing the log.
use crate::{error::{ApiError, ApiResult}, metrics, server::{self, Srv}, state::{Players, Sample, St}};
use std::{os::unix::fs::OpenOptionsExt, time::{Duration, Instant}};

const RUN_SCRIPT: &str = r#"echo $$ > "$MOSS_DIR/pid"; "$@" <>"$MOSS_DIR/console.fifo" >>"$MOSS_DIR/console.log" 2>&1; echo $? > "$MOSS_DIR/exit""#;

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
        (Some(p), Some(t)) => metrics::pid_start(p) == Some(t as u64),
        _ => false,
    }
}

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

pub async fn start(st: &St, s: &Srv, actor: &str) -> ApiResult<()> {
    {
        let mut live = st.live.lock().unwrap();
        let l = live.entry(s.id.clone()).or_default();
        if l.alive || l.starting { return Err(ApiError::conflict("server.running", "server is already running")); }
        l.starting = true;
    }
    let r = start_inner(st, s, actor).await;
    if let Some(l) = st.live.lock().unwrap().get_mut(&s.id) { l.starting = false; }
    r
}

async fn start_inner(st: &St, s: &Srv, actor: &str) -> ApiResult<()> {
    if !s.installed { return Err(ApiError::conflict("server.not_installed", "server is not installed yet")); }
    if s.is_mc() && s.eula_at.is_none() { return Err(ApiError::conflict("eula.required", "The Minecraft EULA must be accepted before this server can start.")); }
    if st.live.lock().unwrap().get(&s.id).is_some_and(|l| l.alive) { return Err(ApiError::conflict("server.running", "server is already running")); }
    if tokio::net::TcpListener::bind(("0.0.0.0", s.port as u16)).await.is_err() {
        return Err(ApiError::conflict("port.in_use", format!("port {} is already in use on this host", s.port)));
    }
    let argv = build_command(st, s)?;
    let (dir, mdir) = (s.dir(st), s.moss_dir(st));
    std::fs::create_dir_all(&mdir)?;
    let fifo = mdir.join("console.fifo");
    if !fifo.exists() {
        let c = std::ffi::CString::new(fifo.to_string_lossy().as_bytes()).unwrap();
        if unsafe { libc::mkfifo(c.as_ptr(), 0o600) } != 0 { return Err(std::io::Error::last_os_error().into()); }
    }
    let _ = std::fs::remove_file(mdir.join("exit"));
    let log = mdir.join("console.log");
    if std::fs::metadata(&log).map(|m| m.len() > 0).unwrap_or(false) { let _ = std::fs::rename(&log, mdir.join("console.log.1")); }
    std::fs::write(&log, format!("[moss] starting {} ({} {})\n", s.name, s.runtime, s.game_version))?;

    let mut cmd = tokio::process::Command::new("/bin/sh");
    cmd.arg("-c").arg(RUN_SCRIPT).arg("moss-run").args(&argv).current_dir(&dir).env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin").env("HOME", &dir).env("LANG", "C.UTF-8").env("MOSS_DIR", &mdir)
        .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    unsafe { cmd.pre_exec(|| { libc::setsid(); Ok(()) }); }
    let mut child = cmd.spawn().map_err(|e| ApiError::conflict("spawn.failed", format!("could not launch: {e}")))?;
    let pid = child.id().unwrap_or(0) as i64;
    let pstart = metrics::pid_start(pid).unwrap_or(0) as i64;
    tokio::spawn(async move { let _ = child.wait().await; }); // reap while the controller lives

    server::save_pid(st, &s.id, Some(pid), Some(pstart), None).await;
    {
        let mut live = st.live.lock().unwrap();
        let l = live.entry(s.id.clone()).or_default();
        l.alive = true; l.ready = false; l.stopping = false; l.crashed = false; l.detail = None;
        l.started = Some(Instant::now()); l.restart_at = None; l.players = None; l.cpu_prev = None;
    }
    server::set_desired(st, &s.id, "running").await;
    st.timeline(&s.id, "start", actor).await;
    st.audit(actor, "server.start", &s.name, "", Some(&s.id)).await;
    crate::console::start_tailer(st, s.id.clone(), log, false);
    let fresh = server::get(st, &s.id).await?;
    server::note_state(st, &fresh);
    Ok(())
}

pub async fn send_command(st: &St, s: &Srv, cmd: &str) -> ApiResult<()> {
    let cmd = cmd.trim();
    if cmd.is_empty() || cmd.len() > 1000 || cmd.contains(['\n', '\r', '\0']) { return Err(ApiError::bad("command.invalid", "command must be a single non-empty line")); }
    if !st.live.lock().unwrap().get(&s.id).is_some_and(|l| l.alive) { return Err(ApiError::conflict("server.not_running", "server is not running")); }
    let fifo = s.moss_dir(st).join("console.fifo");
    let line = format!("{cmd}\n");
    tokio::task::spawn_blocking(move || {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().write(true).custom_flags(libc::O_NONBLOCK).open(fifo)?;
        f.write_all(line.as_bytes())
    }).await.map_err(|e| anyhow::anyhow!(e))?.map_err(|_| ApiError::conflict("server.not_running", "console is not accepting input"))?;
    crate::console::push(&crate::console::hub(st, &s.id), format!("\x1b[36m> {cmd}\x1b[0m"));
    Ok(())
}

fn kill_group(pid: i64, sig: i32) { unsafe { libc::kill(-(pid as i32), sig); } }

fn stop_command(s: &Srv) -> &'static str { if s.is_proxy() { "end" } else { "stop" } }

/// Graceful stop → SIGTERM → SIGKILL, returns when the process is gone.
pub async fn stop_and_wait(st: &St, s: &Srv, graceful: Duration) {
    let _ = send_command(st, s, stop_command(s)).await;
    let wait = |d: Duration| async move {
        let end = Instant::now() + d;
        while Instant::now() < end { if !process_alive(s) { return true; } tokio::time::sleep(Duration::from_millis(400)).await; }
        false
    };
    if wait(graceful).await { return; }
    if let Some(p) = s.pid {
        crate::console::push(&crate::console::hub(st, &s.id), "\x1b[33m[moss] graceful stop timed out — sending SIGTERM\x1b[0m".into());
        kill_group(p, libc::SIGTERM);
        if wait(Duration::from_secs(15)).await { return; }
        crate::console::push(&crate::console::hub(st, &s.id), "\x1b[31m[moss] still running — sending SIGKILL\x1b[0m".into());
        kill_group(p, libc::SIGKILL);
        wait(Duration::from_secs(5)).await;
    }
}

pub async fn stop(st: &St, s: &Srv, actor: &str) -> ApiResult<()> {
    server::set_desired(st, &s.id, "stopped").await;
    let alive = st.live.lock().unwrap().get(&s.id).is_some_and(|l| l.alive);
    {
        let mut live = st.live.lock().unwrap();
        let l = live.entry(s.id.clone()).or_default();
        l.restart_at = None;
        if !alive { l.crashed = false; l.detail = None; }
        else { l.stopping = true; }
    }
    st.audit(actor, "server.stop", &s.name, "", Some(&s.id)).await;
    if alive {
        let (st2, s2) = (st.clone(), s.clone());
        tokio::spawn(async move {
            stop_and_wait(&st2, &s2, Duration::from_secs(if s2.is_proxy() { 30 } else { 90 })).await;
            handle_exit(&st2, &s2.id).await;
        });
    }
    let fresh = server::get(st, &s.id).await?;
    server::note_state(st, &fresh);
    Ok(())
}

pub async fn kill(st: &St, s: &Srv, actor: &str) -> ApiResult<()> {
    server::set_desired(st, &s.id, "stopped").await;
    if let Some(p) = s.pid.filter(|_| process_alive(s)) {
        st.live.lock().unwrap().entry(s.id.clone()).or_default().stopping = true;
        kill_group(p, libc::SIGKILL);
    }
    st.audit(actor, "server.kill", &s.name, "force stop", Some(&s.id)).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    handle_exit(st, &s.id).await;
    Ok(())
}

pub async fn restart(st: &St, s: &Srv, actor: &str) -> ApiResult<()> {
    let alive = st.live.lock().unwrap().get(&s.id).is_some_and(|l| l.alive);
    if !alive { return start(st, s, actor).await; }
    st.live.lock().unwrap().entry(s.id.clone()).or_default().stopping = true;
    st.audit(actor, "server.restart", &s.name, "", Some(&s.id)).await;
    let (st2, s2, actor2) = (st.clone(), s.clone(), actor.to_string());
    tokio::spawn(async move {
        stop_and_wait(&st2, &s2, Duration::from_secs(if s2.is_proxy() { 30 } else { 90 })).await;
        handle_exit(&st2, &s2.id).await;
        tokio::time::sleep(Duration::from_millis(800)).await;
        st2.timeline(&s2.id, "restart", &actor2).await;
        if let Ok(fresh) = server::get(&st2, &s2.id).await
            && let Err(e) = start(&st2, &fresh, &actor2).await { set_detail(&st2, &s2.id, &format!("restart failed: {}", e.message)); }
    });
    let fresh = server::get(st, &s.id).await?;
    server::note_state(st, &fresh);
    Ok(())
}

fn set_detail(st: &St, id: &str, d: &str) { st.live.lock().unwrap().entry(id.to_string()).or_default().detail = Some(d.to_string()); }

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
            let why = code.map_or("killed".to_string(), |c| format!("exit code {c}"));
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

pub fn spawn_background(st: &St) {
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
    let st2 = st.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        loop {
            tick.tick().await;
            st2.host.lock().unwrap().sample();
            let Ok(servers) = server::list(&st2).await else { continue };
            for s in servers.iter().filter(|s| s.installed && s.pid.is_some()) {
                let Some(pid) = s.pid else { continue };
                let Some((ticks, rss)) = metrics::group_usage(pid) else { continue };
                let (ready, cpu) = {
                    let mut live = st2.live.lock().unwrap();
                    let l = live.entry(s.id.clone()).or_default();
                    let cpu = l.cpu_prev.map(|(pt, pi)| 100.0 * metrics::ticks_to_secs(ticks.saturating_sub(pt)) / pi.elapsed().as_secs_f64().max(0.1));
                    l.cpu_prev = Some((ticks, Instant::now()));
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
                server::emit(&st2, &s.id).await;
            }
        }
    });
}

/// Called once at startup: re-attach to servers that survived, account for ones that died while we were down, autostart.
pub async fn reconcile(st: &St) -> anyhow::Result<()> {
    let servers = server::list(st).await?;
    let mut to_start = vec![];
    for s in servers.iter().filter(|s| s.installed) {
        if process_alive(s) {
            let started = st.db.call({ let id = s.id.clone(); move |c| c.query_row("SELECT started_at FROM servers WHERE id=?", [id], |r| r.get::<_, Option<String>>(0)) }).await.ok().flatten();
            let age = started.and_then(|t| chrono::DateTime::parse_from_rfc3339(&t).ok()).map(|t| (chrono::Utc::now() - t.with_timezone(&chrono::Utc)).num_seconds().max(0) as u64).unwrap_or(60);
            let ready = crate::mcping::ping(s.port as u16).await.is_ok() || !s.is_mc() && s.runtime != "sim";
            {
                let mut live = st.live.lock().unwrap();
                let l = live.entry(s.id.clone()).or_default();
                l.alive = true; l.started = Some(Instant::now() - Duration::from_secs(age));
                l.ready = ready;
            }
            crate::console::start_tailer(st, s.id.clone(), s.moss_dir(st).join("console.log"), true);
            st.timeline(&s.id, "reattach", "controller restarted; server kept running").await;
            tracing::info!(server = %s.name, pid = ?s.pid, "re-attached to running server");
        } else {
            if s.pid.is_some() { handle_exit(st, &s.id).await; }
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
