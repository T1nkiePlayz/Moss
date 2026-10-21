//! Graceful stop, escalation to SIGTERM/SIGKILL, force kill and restart.
use super::{ctr, exit::handle_exit, process_alive, set_detail, start::{send_command, start}};
use crate::{error::ApiResult, games::Stop, server::{self, Srv}, state::St};
use std::time::{Duration, Instant};

fn kill_group(pid: i64, sig: i32) { unsafe { libc::kill(-(pid as i32), sig); } }

fn stop_command(s: &Srv) -> &'static str {
    if s.is_proxy() { return "end"; }
    match s.game().map(|g| g.stop) { Some(Stop::Command(c)) => c, _ => "stop" }
}

/// Signal-stopped games (Valheim, Palworld, Satisfactory) get the signal on the game process itself, not on the
/// wrapper shell: the shell must stay alive until the game has finished saving, because its PID is what Moss watches.
fn signal_game(pid: i64, sig: i32) {
    let kids = crate::games::descendants(pid, &crate::games::proc_parents());
    if kids.is_empty() { kill_group(pid, sig); return; }
    for k in kids { unsafe { libc::kill(k as i32, sig); } }
}

/// Graceful stop → SIGTERM → SIGKILL, returns when the process is gone.
pub async fn stop_and_wait(st: &St, s: &Srv, graceful: Duration) {
    match (s.game().map(|g| g.stop), s.pid) {
        (Some(Stop::Signal(sig)), Some(pid)) if !s.in_container => signal_game(pid, sig),
        (Some(Stop::Signal(sig)), _) if s.in_container => ctr::signal_container(st, s, sig).await,
        _ => { let _ = send_command(st, s, stop_command(s)).await; }
    }
    let wait = |d: Duration| async move {
        let end = Instant::now() + d;
        while Instant::now() < end {
            let gone = if s.in_container { !ctr::running_now(st, s).await } else { !process_alive(s) };
            if gone { if s.in_container { crate::container::set_alive(&crate::container::container_name(&s.id), false); } return true; }
            tokio::time::sleep(Duration::from_millis(400)).await;
        }
        false
    };
    if wait(graceful).await { return; }
    if s.in_container {
        // `docker stop`: SIGTERM, then SIGKILL after its own grace period
        crate::console::push(&crate::console::hub(st, &s.id), "\x1b[33m[moss] graceful stop timed out — stopping the container\x1b[0m".into());
        ctr::stop_container(st, s, 15).await;
        if wait(Duration::from_secs(25)).await { return; }
        crate::console::push(&crate::console::hub(st, &s.id), "\x1b[31m[moss] still running — killing the container\x1b[0m".into());
        ctr::kill_container(st, s).await;
        wait(Duration::from_secs(5)).await;
        return;
    }
    if let Some(p) = s.pid {
        crate::console::push(&crate::console::hub(st, &s.id), "\x1b[33m[moss] graceful stop timed out — sending SIGTERM\x1b[0m".into());
        kill_group(p, libc::SIGTERM);
        if wait(Duration::from_secs(15)).await { return; }
        crate::console::push(&crate::console::hub(st, &s.id), "\x1b[31m[moss] still running — sending SIGKILL\x1b[0m".into());
        kill_group(p, libc::SIGKILL);
        wait(Duration::from_secs(5)).await;
    }
}

/// Stops a server and waits until its process is gone (test copies: nobody watches them, so no audit entry).
pub async fn halt(st: &St, s: &Srv) {
    server::set_desired(st, &s.id, "stopped").await;
    {
        let mut live = st.live.lock().unwrap();
        let l = live.entry(s.id.clone()).or_default();
        l.restart_at = None;
        if l.alive { l.stopping = true; }
    }
    let fresh = server::get(st, &s.id).await.unwrap_or_else(|_| s.clone());
    if process_alive(&fresh) { stop_and_wait(st, &fresh, Duration::from_secs(30)).await; }
    handle_exit(st, &s.id).await;
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
    if s.in_container && process_alive(s) {
        st.live.lock().unwrap().entry(s.id.clone()).or_default().stopping = true;
        ctr::kill_container(st, s).await;
        crate::container::set_alive(&crate::container::container_name(&s.id), false);
    } else if let Some(p) = s.pid.filter(|_| process_alive(s)) {
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
        // Someone may have pressed Stop or Kill while the restart was under way: that wish wins.
        let Ok(fresh) = server::get(&st2, &s2.id).await else { return };
        if fresh.desired == "stopped" { return; }
        st2.timeline(&s2.id, "restart", &actor2).await;
        if let Err(e) = start(&st2, &fresh, &actor2).await { set_detail(&st2, &s2.id, &format!("restart failed: {}", e.message)); }
    });
    let fresh = server::get(st, &s.id).await?;
    server::note_state(st, &fresh);
    Ok(())
}

