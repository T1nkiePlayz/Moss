//! Event webhooks: outbound, signed, retried subscriptions to what happens on this Moss instance (distinct from the
//! notification webhook, which is a human-readable alert). `publish` turns an event into queued deliveries (one per
//! matching endpoint); `queue` delivers them with persisted retries; `events` derives events from the internal bus.
use crate::state::St;
use serde_json::{Value, json};

pub mod deliver;
pub mod events;
pub mod queue;
pub mod secrets;
pub mod sign;
#[cfg(test)] mod tests;

pub const EVENT_TYPES: &[(&str, &str)] = &[
    ("server.started", "A server finished starting and is ready"),
    ("server.stopped", "A server stopped"),
    ("server.crashed", "A server's process died unexpectedly"),
    ("server.restarted", "A server came back up shortly after going down"),
    ("backup.completed", "A backup finished"),
    ("backup.failed", "A backup or a remote copy of one failed"),
    ("operation.completed", "Any long-running job finished"),
    ("operation.failed", "Any long-running job failed"),
    ("player.joined", "A player joined (seen in the server's player list, polled every few seconds)"),
    ("player.left", "A player left (same limits as player.joined)"),
    ("content.installed", "A mod, plugin or modpack was installed"),
    ("update.available", "Updates are available for installed content"),
    ("health.critical", "A critical finding appeared in the health checks"),
];

pub fn is_event(t: &str) -> bool { EVENT_TYPES.iter().any(|(k, _)| *k == t) }

#[derive(Clone, Debug)]
pub struct Endpoint {
    pub id: String, pub name: String, pub url: String, pub events: Vec<String>, pub server_ids: Vec<String>,
    pub enabled: bool, pub failures: i64, pub disabled_reason: Option<String>, pub created_at: String,
}

impl Endpoint {
    pub fn from_row(r: &rusqlite::Row) -> rusqlite::Result<Self> {
        let list = |i: usize| -> rusqlite::Result<Vec<String>> { Ok(serde_json::from_str(&r.get::<_, String>(i)?).unwrap_or_default()) };
        Ok(Self { id: r.get(0)?, name: r.get(1)?, url: r.get(2)?, events: list(3)?, server_ids: list(4)?, enabled: r.get(5)?, failures: r.get(6)?, disabled_reason: r.get(7)?, created_at: r.get(8)? })
    }
    pub fn wants(&self, kind: &str, server_id: Option<&str>) -> bool {
        self.events.iter().any(|e| e == kind) && (self.server_ids.is_empty() || server_id.is_some_and(|s| self.server_ids.iter().any(|x| x == s)))
    }
}
pub const COLS: &str = "id,name,url,events,server_ids,enabled,failures,disabled_reason,created_at";

pub async fn list(st: &St) -> anyhow::Result<Vec<Endpoint>> {
    st.db.call(|c| c.prepare(&format!("SELECT {COLS} FROM webhook_endpoints ORDER BY created_at"))?.query_map([], Endpoint::from_row)?.collect()).await
}
pub async fn get(st: &St, id: &str) -> anyhow::Result<Option<Endpoint>> {
    let id = id.to_string();
    st.db.call(move |c| c.query_row(&format!("SELECT {COLS} FROM webhook_endpoints WHERE id=?"), [id], Endpoint::from_row).map(Some).or_else(|e| if matches!(e, rusqlite::Error::QueryReturnedNoRows) { Ok(None) } else { Err(e) })).await
}

/// An event ready to be queued.
pub struct Ev { pub kind: &'static str, pub server: Option<(String, String)>, pub data: Value }

/// The JSON delivered for an event. Oversized `data` is replaced so one body never exceeds 64 KiB.
pub fn body(event_id: &str, kind: &str, time: &str, server: Option<&(String, String)>, data: &Value) -> String {
    let mk = |d: &Value| json!({"id": event_id, "type": kind, "time": time, "server": server.map(|(i, n)| json!({"id": i, "name": n})), "data": d}).to_string();
    let b = mk(data);
    if b.len() > 64 << 10 { mk(&json!({"truncated": true})) } else { b }
}

/// Queue the event for every enabled endpoint that subscribes to it. Never fails the caller.
pub async fn publish(st: &St, ev: Ev) {
    let eps = match list(st).await { Ok(v) => v, Err(e) => { tracing::warn!("webhooks: could not read endpoints: {e:#}"); return; } };
    let sid = ev.server.as_ref().map(|s| s.0.as_str());
    let targets: Vec<String> = eps.iter().filter(|e| e.enabled && e.wants(ev.kind, sid)).map(|e| e.id.clone()).collect();
    if targets.is_empty() { return; }
    let (event_id, time) = (uuid::Uuid::now_v7().to_string(), crate::db::now());
    let b = body(&event_id, ev.kind, &time, ev.server.as_ref(), &ev.data);
    let kind = ev.kind.to_string();
    if let Err(e) = st.db.call(move |c| { for t in &targets { queue::enqueue(c, t, &event_id, &kind, &b, false, &time)?; } Ok(()) }).await { tracing::warn!("webhooks: could not queue event: {e:#}"); return; }
    queue::wake();
}

pub fn spawn_background(st: &St) {
    queue::spawn(st);
    events::spawn(st);
}
