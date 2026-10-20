//! Console hub: one log tailer per running server fans lines out to any number of WebSocket viewers.
use crate::{auth::{self, User}, error::ApiError, state::St};
use axum::{extract::{Path, Query, State, ws::{Message, WebSocket, WebSocketUpgrade}}, response::Response};
use std::{collections::VecDeque, io::{Read, Seek, SeekFrom}, path::Path as FsPath, sync::{Arc, Mutex}};
use tokio::sync::broadcast;

mod lines;
#[cfg(test)] mod flood_tests;

const RING: usize = 1500;

pub struct Hub {
    pub ring: Mutex<(u64, VecDeque<(u64, String)>)>,
    pub tx: broadcast::Sender<(u64, String)>,
    pub tailing: std::sync::atomic::AtomicBool,
}

pub fn hub(st: &St, id: &str) -> Arc<Hub> {
    st.hubs.lock().unwrap().entry(id.to_string()).or_insert_with(|| {
        let (tx, _) = broadcast::channel(512);
        Arc::new(Hub { ring: Mutex::new((0, VecDeque::new())), tx, tailing: Default::default() })
    }).clone()
}

pub fn push(h: &Hub, line: String) {
    let seq = {
        let mut r = h.ring.lock().unwrap();
        r.0 += 1;
        let seq = r.0;
        r.1.push_back((seq, line.clone()));
        if r.1.len() > RING { r.1.pop_front(); }
        seq
    };
    let _ = h.tx.send((seq, line));
}

pub fn read_tail(path: &FsPath, max_bytes: u64) -> Vec<String> {
    let Ok(mut f) = std::fs::File::open(path) else { return vec![] };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let start = len.saturating_sub(max_bytes);
    let _ = f.seek(SeekFrom::Start(start));
    let mut buf = Vec::new();
    let _ = f.take(max_bytes).read_to_end(&mut buf);
    let text = String::from_utf8_lossy(&buf);
    let mut lines: Vec<String> = text.lines().map(|l| l.trim_end_matches('\r').to_string()).collect();
    if start > 0 && !lines.is_empty() { lines.remove(0); }
    lines
}

/// Ensure the ring is primed from disk (used on first access after a controller restart).
pub fn prime(st: &St, id: &str, log: &FsPath) -> Arc<Hub> {
    let h = hub(st, id);
    let empty = h.ring.lock().unwrap().1.is_empty();
    if empty {
        for l in read_tail(log, 256 * 1024).into_iter().rev().take(RING).collect::<Vec<_>>().into_iter().rev() { push(&h, l); }
    }
    h
}

/// How long to wait before the next poll: fast while output flows or the server runs, backing off to about a
/// second when idle and the server is not running.
pub fn next_poll_ms(alive: bool, idle_polls: u32) -> u64 {
    if alive { return if idle_polls < 100 { 120 } else { 250 }; }
    match idle_polls { 0 | 1 => 120, 2 => 250, 3 => 500, _ => 1000 }
}

/// What the activity log records for a console command. `redacted` (default) hides the arguments of commands that
/// carry credentials, `all` logs everything and `off` logs nothing. `None` means: do not write an entry.
pub fn audit_command_detail(mode: &str, cmd: &str) -> Option<String> {
    const SENSITIVE: &[&str] = &["login", "l", "register", "reg", "password", "changepassword", "changepass", "authme", "2fa", "rcon", "auth", "setpassword", "unregister", "forcelogin"];
    match mode {
        "off" => None,
        "all" => Some(cmd.to_string()),
        _ => {
            let first = cmd.split_whitespace().next().unwrap_or("").trim_start_matches('/').to_lowercase();
            if SENSITIVE.contains(&first.as_str()) { Some(format!("[redacted: {first}]")) } else { Some(cmd.to_string()) }
        }
    }
}

/// Tail the log from `offset` until the process is gone and the file is drained. Calls `on_line` for readiness detection.
pub fn start_tailer(st: &St, id: String, log: std::path::PathBuf, from_end: bool) {
    let h = prime(st, &id, &log);
    if h.tailing.swap(true, std::sync::atomic::Ordering::SeqCst) { return; }
    let st = st.clone();
    tokio::spawn(async move {
        let mut off = if from_end { std::fs::metadata(&log).map(|m| m.len()).unwrap_or(0) } else { 0 };
        let mut split = lines::LineSplitter::default();
        let mut idle_after_exit = 0;
        let mut idle_polls = 0u32;
        loop {
            let (new_off, chunk) = tokio::task::spawn_blocking({
                let (log, off) = (log.clone(), off);
                move || {
                    let Ok(mut f) = std::fs::File::open(&log) else { return (off, Vec::new()) };
                    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
                    let off = if len < off { 0 } else { off }; // log truncated/rotated
                    let _ = f.seek(SeekFrom::Start(off));
                    let mut buf = Vec::new();
                    let _ = f.take(256 * 1024).read_to_end(&mut buf);
                    (off + buf.len() as u64, buf)
                }
            }).await.unwrap_or((off, Vec::new()));
            off = new_off;
            if !chunk.is_empty() {
                idle_after_exit = 0;
                idle_polls = 0;
                split.feed(&chunk, |line| {
                    if crate::lifecycle::looks_ready(&line) { crate::lifecycle::mark_ready(&st, &id); }
                    push(&h, line);
                });
                continue;
            }
            let alive = st.live.lock().unwrap().get(&id).is_some_and(|l| l.alive);
            if !alive {
                idle_after_exit += 1;
                if idle_after_exit > 5 { split.flush(|l| push(&h, l)); break; }
            }
            idle_polls = idle_polls.saturating_add(1);
            tokio::time::sleep(std::time::Duration::from_millis(next_poll_ms(alive, idle_polls))).await;
        }
        h.tailing.store(false, std::sync::atomic::Ordering::SeqCst);
    });
}

pub async fn backlog(State(st): State<St>, user: User, Path(id): Path<String>, Query(q): Query<std::collections::HashMap<String, String>>) -> Result<axum::Json<serde_json::Value>, ApiError> {
    auth::authz(&st, &user, &id, "server.console.read").await?;
    let s = crate::server::get(&st, &id).await?;
    let n: usize = q.get("lines").and_then(|v| v.parse().ok()).unwrap_or(500).min(RING);
    let h = prime(&st, &id, &s.moss_dir(&st).join("console.log"));
    let r = h.ring.lock().unwrap();
    let lines: Vec<&String> = r.1.iter().rev().take(n).map(|(_, l)| l).collect::<Vec<_>>().into_iter().rev().collect();
    Ok(axum::Json(serde_json::json!({"lines": lines})))
}

pub async fn ws(State(st): State<St>, Path(id): Path<String>, Query(q): Query<std::collections::HashMap<String, String>>, up: WebSocketUpgrade) -> Result<Response, ApiError> {
    let user = crate::api::take_ticket(&st, q.get("ticket"))?;
    auth::authz(&st, &user, &id, "server.console.read").await?;
    let s = crate::server::get(&st, &id).await?;
    let can_write = auth::authz(&st, &user, &id, "server.console.write").await.is_ok();
    Ok(up.max_message_size(16 * 1024).on_upgrade(move |sock| handle(st, s, user, can_write, sock)))
}

/// Where a viewer's frames go. A WebSocket in production; tests use sinks that stall.
pub trait Sink { fn send(&mut self, text: String) -> impl std::future::Future<Output = bool> + Send; }
impl Sink for WebSocket { async fn send(&mut self, text: String) -> bool { WebSocket::send(self, Message::Text(text.into())).await.is_ok() } }

/// A viewer that cannot take a frame within this long is dropped (its socket buffer is full: the browser is gone or frozen).
pub const SEND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
/// Falling behind the broadcast buffer this many times within `LAG_WINDOW` also drops the viewer.
const MAX_LAGS: usize = 5;
const LAG_WINDOW: std::time::Duration = std::time::Duration::from_secs(30);

pub async fn send_frame<S: Sink>(sink: &mut S, text: String, timeout: std::time::Duration) -> bool {
    tokio::time::timeout(timeout, sink.send(text)).await.unwrap_or(false)
}

fn line_frame(l: &str) -> String { serde_json::json!({"t":"line","d":l}).to_string() }

/// Handles one message from the hub for one viewer. False = stop serving this viewer (it is told why when it can still listen).
pub async fn relay<S: Sink>(m: Result<(u64, String), broadcast::error::RecvError>, last: u64, sink: &mut S, lags: &mut std::collections::VecDeque<std::time::Instant>, timeout: std::time::Duration) -> bool {
    match m {
        Ok((seq, l)) => seq <= last || send_frame(sink, line_frame(&l), timeout).await,
        Err(broadcast::error::RecvError::Lagged(n)) => {
            let now = std::time::Instant::now();
            lags.push_back(now);
            while lags.front().is_some_and(|t| now.duration_since(*t) > LAG_WINDOW) { lags.pop_front(); }
            if lags.len() >= MAX_LAGS {
                let _ = send_frame(sink, line_frame("\x1b[31m[moss] disconnected: this connection cannot keep up with the console output. Reload the page to reconnect.\x1b[0m"), std::time::Duration::from_millis(500)).await;
                return false;
            }
            send_frame(sink, line_frame(&format!("\x1b[33m[moss] {n} lines skipped (slow connection)\x1b[0m")), timeout).await
        }
        Err(_) => false,
    }
}

async fn handle(st: St, s: crate::server::Srv, user: User, can_write: bool, mut sock: WebSocket) {
    let h = prime(&st, &s.id, &s.moss_dir(&st).join("console.log"));
    // The UI loads history from /console/backlog; the socket only carries lines produced after it connects.
    let (mut rx, last) = { let r = h.ring.lock().unwrap(); (h.tx.subscribe(), r.0) };
    let mut bus = st.bus.subscribe();
    let mut lags = std::collections::VecDeque::new();
    loop {
        tokio::select! {
            m = rx.recv() => if !relay(m, last, &mut sock, &mut lags, SEND_TIMEOUT).await { break; },
            e = bus.recv() => if let Ok(e) = e && e.kind == "server" && e.data["id"] == s.id.as_str()
                && !send_frame(&mut sock, serde_json::json!({"t":"state","d":e.data["state"]}).to_string(), SEND_TIMEOUT).await { break; },
            m = sock.recv() => match m {
                Some(Ok(Message::Text(t))) => {
                    let Ok(v) = serde_json::from_str::<serde_json::Value>(&t) else { continue };
                    if v["t"] == "cmd" && can_write
                        && let Some(cmd) = v["d"].as_str() {
                            if let Err(e) = crate::lifecycle::send_command(&st, &s, cmd).await {
                                let _ = sock.send(Message::Text(serde_json::json!({"t":"line","d":format!("\x1b[31m[moss] {}\x1b[0m", e.message)}).to_string().into())).await;
                            } else { st.audit_command(&user.actor(), &s.name, cmd, &s.id).await; }
                        }
                }
                Some(Ok(Message::Ping(p))) => { let _ = sock.send(Message::Pong(p)).await; }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                _ => {}
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polling_is_fast_while_busy_and_backs_off_when_idle_and_stopped() {
        assert_eq!(next_poll_ms(true, 0), 120);
        assert_eq!(next_poll_ms(true, 99), 120);
        assert_eq!(next_poll_ms(true, 500), 250);
        assert_eq!(next_poll_ms(false, 0), 120);
        let mut prev = 0;
        for i in 0..8 { let d = next_poll_ms(false, i); assert!(d >= prev && d <= 1000); prev = d; }
        assert_eq!(next_poll_ms(false, 50), 1000);
    }

    #[test]
    fn sensitive_commands_are_redacted_by_default() {
        for c in ["login hunter2", "/login hunter2", "LOGIN x", "/Register a a", "l pw", "2fa 123456", "authme reload", "rcon pw", "forcelogin Bob", "unregister pw"] {
            let got = audit_command_detail("redacted", c).unwrap();
            assert!(got.starts_with("[redacted: ") && !got.contains("hunter2") && !got.contains("123456") && !got.contains("pw"), "{c} -> {got}");
        }
        assert_eq!(audit_command_detail("redacted", "rcon-pass").unwrap(), "rcon-pass");
        assert_eq!(audit_command_detail("redacted", "/login pw").unwrap(), "[redacted: login]");
        assert_eq!(audit_command_detail("redacted", "  Register   a b").unwrap(), "[redacted: register]");
        assert_eq!(audit_command_detail("redacted", "say hello").unwrap(), "say hello");
        assert_eq!(audit_command_detail("redacted", "").unwrap(), "");
    }

    #[test]
    fn all_and_off_modes() {
        assert_eq!(audit_command_detail("all", "login pw").unwrap(), "login pw");
        assert!(audit_command_detail("off", "say hi").is_none());
        // an unknown stored value behaves like the safe default
        assert_eq!(audit_command_detail("garbage", "login pw").unwrap(), "[redacted: login]");
    }
}
