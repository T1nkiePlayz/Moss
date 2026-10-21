use crate::{config::Config, db::Db};
use serde_json::Value;
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::Instant,
};
use tokio::sync::broadcast;

#[derive(Clone, Debug)]
pub struct Event {
    pub kind: &'static str,
    pub data: Value,
}

#[derive(Default, Clone, Debug)]
pub struct Players {
    pub online: u32,
    pub max: u32,
    pub sample: Vec<String>,
}

/// Volatile per-server runtime facts. Durable facts live in the database.
#[derive(Default)]
pub struct Live {
    pub alive: bool,
    pub ready: bool,
    pub stopping: bool,
    pub crashed: bool,
    pub detail: Option<String>,
    pub state_since: Option<String>,
    pub started: Option<Instant>,
    pub players: Option<Players>,
    pub motd: Option<String>,
    pub cpu: Option<f64>,
    pub mem: Option<u64>,
    pub crashes: Vec<Instant>,
    pub restart_at: Option<Instant>,
    pub last_state: String,
    pub cpu_prev: Option<(f64, Instant)>,
    pub exiting: bool,
    /// A start is in progress; stops two quick Start clicks (or an autostart racing a click) from launching twice.
    pub starting: bool,
}

#[derive(Clone, Copy, serde::Serialize)]
pub struct Sample {
    pub t: i64,
    pub cpu: f64,
    pub mem: u64,
    pub players: Option<u32>,
}

pub struct Inner {
    pub cfg: Config,
    pub db: Db,
    pub bus: broadcast::Sender<Event>,
    pub http: reqwest::Client,
    pub started: Instant,
    pub live: Mutex<HashMap<String, Live>>,
    pub hubs: Mutex<HashMap<String, Arc<crate::console::Hub>>>,
    pub stats: Mutex<HashMap<String, VecDeque<Sample>>>,
    pub tickets: Mutex<HashMap<String, (Instant, crate::auth::User, u64)>>, // minted at, user snapshot, auth epoch
    pub ops: Mutex<crate::ops::Registry>,
    pub locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    pub cache: Mutex<HashMap<String, (Instant, Value)>>,
    pub users: Mutex<HashMap<String, (Instant, crate::auth::User)>>,
    pub jwks: Mutex<Option<(Instant, Value)>>,
    pub guard: crate::auth::Guard,
    pub host: Mutex<crate::platform::HostSampler>,
    pub cf: Mutex<crate::providers::curseforge::Limiter>,
    pub cf_sem: tokio::sync::Semaphore,
    pub container: crate::container::Handle,
}

pub type St = Arc<Inner>;

impl Inner {
    pub fn emit(&self, kind: &'static str, data: Value) {
        let _ = self.bus.send(Event { kind, data });
    }

    pub fn lock_for(&self, server_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.locks.lock().unwrap().entry(server_id.to_string()).or_default().clone()
    }

    pub async fn cached<F, Fut>(&self, key: &str, ttl_s: u64, f: F) -> anyhow::Result<Value>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = anyhow::Result<Value>>,
    {
        if let Some((t, v)) = self.cache.lock().unwrap().get(key)
            && t.elapsed().as_secs() < ttl_s {
                return Ok(v.clone());
            }
        let v = f().await?;
        let mut c = self.cache.lock().unwrap();
        if c.len() > 400 {
            c.retain(|_, (t, _)| t.elapsed().as_secs() < 600);
        }
        c.insert(key.to_string(), (Instant::now(), v.clone()));
        Ok(v)
    }

    /// Append to the activity log (also pushed to subscribers).
    pub async fn audit(&self, actor: &str, action: &str, target: &str, detail: &str, server_id: Option<&str>) {
        let (a, ac, t, d, s, time) = (actor.to_string(), action.to_string(), target.to_string(), detail.to_string(), server_id.map(String::from), crate::db::now());
        let (a2, ac2, t2, d2, s2, tm2) = (a.clone(), ac.clone(), t.clone(), d.clone(), s.clone(), time.clone());
        let id = self
            .db
            .call(move |c| {
                c.execute("INSERT INTO activity(time,actor,action,target,detail,server_id) VALUES (?,?,?,?,?,?)", rusqlite::params![tm2, a2, ac2, t2, d2, s2])?;
                Ok(c.last_insert_rowid())
            })
            .await
            .unwrap_or(0);
        self.emit("activity", serde_json::json!({"id": id, "time": time, "actor": a, "action": ac, "target": t, "detail": d, "server_id": s}));
    }

    /// Audit a console command according to the `audit_console_commands` setting (redacted by default).
    pub async fn audit_command(&self, actor: &str, server_name: &str, cmd: &str, server_id: &str) {
        let mode = self.setting("audit_console_commands", "redacted").await;
        if let Some(detail) = crate::console::audit_command_detail(&mode, cmd) {
            self.audit(actor, "console.command", server_name, &detail, Some(server_id)).await;
        }
    }

    pub async fn timeline(&self, server_id: &str, kind: &str, detail: &str) {
        let (s, k, d, t) = (server_id.to_string(), kind.to_string(), detail.to_string(), crate::db::now());
        let _ = self.db.call(move |c| c.execute("INSERT INTO timeline(server_id,time,kind,detail) VALUES (?,?,?,?)", rusqlite::params![s, t, k, d])).await;
    }

    pub async fn setting(&self, key: &str, default: &str) -> String {
        let k = key.to_string();
        let v = self.db.call(move |c| c.query_row("SELECT v FROM meta WHERE k=?", [k], |r| r.get::<_, String>(0)).map(Some).or_else(|e| if matches!(e, rusqlite::Error::QueryReturnedNoRows) { Ok(None) } else { Err(e) })).await;
        v.ok().flatten().unwrap_or_else(|| default.to_string())
    }

    pub async fn set_setting(&self, key: &str, value: &str) {
        let (k, v) = (key.to_string(), value.to_string());
        let _ = self.db.call(move |c| c.execute("INSERT INTO meta(k,v) VALUES(?,?) ON CONFLICT(k) DO UPDATE SET v=excluded.v", [k, v])).await;
    }
}
