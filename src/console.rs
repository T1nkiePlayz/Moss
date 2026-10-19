//! Console hub: one log tailer per running server fans lines out to any number of WebSocket viewers.
use crate::{auth::{self, User}, error::ApiError, state::St};
use axum::{extract::{Path, Query, State, ws::{Message, WebSocket, WebSocketUpgrade}}, response::Response};
use std::{collections::VecDeque, io::{Read, Seek, SeekFrom}, path::Path as FsPath, sync::{Arc, Mutex}};
use tokio::sync::broadcast;

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

/// Tail the log from `offset` until the process is gone and the file is drained. Calls `on_line` for readiness detection.
pub fn start_tailer(st: &St, id: String, log: std::path::PathBuf, from_end: bool) {
    let h = prime(st, &id, &log);
    if h.tailing.swap(true, std::sync::atomic::Ordering::SeqCst) { return; }
    let st = st.clone();
    tokio::spawn(async move {
        let mut off = if from_end { std::fs::metadata(&log).map(|m| m.len()).unwrap_or(0) } else { 0 };
        let mut partial = String::new();
        let mut idle_after_exit = 0;
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
                partial.push_str(&String::from_utf8_lossy(&chunk));
                while let Some(i) = partial.find('\n') {
                    let line: String = partial.drain(..=i).collect();
                    let line = line.trim_end_matches(['\n', '\r']).to_string();
                    if crate::lifecycle::looks_ready(&line) { crate::lifecycle::mark_ready(&st, &id); }
                    push(&h, line);
                }
                if partial.len() > 64 * 1024 { let l = std::mem::take(&mut partial); push(&h, l); }
                continue;
            }
            let alive = st.live.lock().unwrap().get(&id).is_some_and(|l| l.alive);
            if !alive {
                idle_after_exit += 1;
                if idle_after_exit > 5 { if !partial.is_empty() { push(&h, std::mem::take(&mut partial)); } break; }
            }
            tokio::time::sleep(std::time::Duration::from_millis(120)).await;
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

async fn handle(st: St, s: crate::server::Srv, user: User, can_write: bool, mut sock: WebSocket) {
    let h = prime(&st, &s.id, &s.moss_dir(&st).join("console.log"));
    // The UI loads history from /console/backlog; the socket only carries lines produced after it connects.
    let (mut rx, last) = { let r = h.ring.lock().unwrap(); (h.tx.subscribe(), r.0) };
    let mut bus = st.bus.subscribe();
    loop {
        tokio::select! {
            m = rx.recv() => match m {
                Ok((seq, l)) => { if seq > last && sock.send(Message::Text(serde_json::json!({"t":"line","d":l}).to_string().into())).await.is_err() { break; } }
                Err(broadcast::error::RecvError::Lagged(n)) => { let _ = sock.send(Message::Text(serde_json::json!({"t":"line","d":format!("\x1b[33m[moss] {n} lines skipped (slow connection)\x1b[0m")}).to_string().into())).await; }
                Err(_) => break,
            },
            e = bus.recv() => if let Ok(e) = e && e.kind == "server" && e.data["id"] == s.id.as_str()
                && sock.send(Message::Text(serde_json::json!({"t":"state","d":e.data["state"]}).to_string().into())).await.is_err() { break; },
            m = sock.recv() => match m {
                Some(Ok(Message::Text(t))) => {
                    let Ok(v) = serde_json::from_str::<serde_json::Value>(&t) else { continue };
                    if v["t"] == "cmd" && can_write
                        && let Some(cmd) = v["d"].as_str() {
                            if let Err(e) = crate::lifecycle::send_command(&st, &s, cmd).await {
                                let _ = sock.send(Message::Text(serde_json::json!({"t":"line","d":format!("\x1b[31m[moss] {}\x1b[0m", e.message)}).to_string().into())).await;
                            } else { st.audit(&user.email, "console.command", &s.name, cmd, Some(&s.id)).await; }
                        }
                }
                Some(Ok(Message::Ping(p))) => { let _ = sock.send(Message::Pong(p)).await; }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                _ => {}
            },
        }
    }
}
