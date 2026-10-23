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
    /// Bumped by every Stop/Kill, so a start finishing concurrently can tell its "running" write lost the race.
    pub stop_gen: u64,
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
    pub audit_q: AuditQueue,
}

pub type St = Arc<Inner>;

/// One activity-log entry waiting for `AuditQueue`'s writer.
struct AuditRow { time: String, actor: String, action: String, target: String, detail: String, server_id: Option<String> }

/// Console-command audit entries, written in batches by one background task so a command never waits for a commit.
/// Each command used to insert its own row on the shared writer before answering: with many admins or workflows
/// typing at once, a slow commit (a WAL fsync on a busy disk) held every POST /command for its whole length.
/// Rows reach the log and the event stream in order, a moment later. The queue holds 16k entries (a few MB at most),
/// far more than people or workflows type during a long stall; a scripted flood beyond that waits rather than lose
/// entries.
pub struct AuditQueue(tokio::sync::mpsc::Sender<AuditRow>);

impl AuditQueue {
    const CAPACITY: usize = 16_384;
    const BATCH: usize = 256;

    pub fn spawn(db: Db, bus: broadcast::Sender<Event>) -> Self {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<AuditRow>(Self::CAPACITY);
        tokio::spawn(async move {
            let mut batch = Vec::with_capacity(Self::BATCH);
            while rx.recv_many(&mut batch, Self::BATCH).await > 0 {
                let rows: Vec<_> = batch.iter().map(|r| (r.time.clone(), r.actor.clone(), r.action.clone(), r.target.clone(), r.detail.clone(), r.server_id.clone())).collect();
                let ids = db.call(move |c| {
                    let tx = c.unchecked_transaction()?;
                    let mut ids = Vec::with_capacity(rows.len());
                    {
                        let mut q = tx.prepare_cached("INSERT INTO activity(time,actor,action,target,detail,server_id) VALUES (?,?,?,?,?,?)")?;
                        for r in &rows { q.execute(rusqlite::params![r.0, r.1, r.2, r.3, r.4, r.5])?; ids.push(tx.last_insert_rowid()); }
                    }
                    tx.commit()?;
                    Ok(ids)
                }).await.unwrap_or_else(|e| { tracing::warn!("activity log: {} console command entries not written: {e}", batch.len()); vec![] });
                for (i, r) in batch.drain(..).enumerate() {
                    let _ = bus.send(Event { kind: "activity", data: serde_json::json!({"id": ids.get(i).copied().unwrap_or(0), "time": r.time, "actor": r.actor, "action": r.action, "target": r.target, "detail": r.detail, "server_id": r.server_id}) });
                }
            }
        });
        Self(tx)
    }
}

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
            let row = AuditRow { time: crate::db::now(), actor: actor.into(), action: "console.command".into(), target: server_name.into(), detail, server_id: Some(server_id.into()) };
            let _ = self.audit_q.0.send(row).await;
        }
    }

    pub async fn timeline(&self, server_id: &str, kind: &str, detail: &str) {
        let (s, k, d, t) = (server_id.to_string(), kind.to_string(), detail.to_string(), crate::db::now());
        let _ = self.db.call(move |c| c.execute("INSERT INTO timeline(server_id,time,kind,detail) VALUES (?,?,?,?)", rusqlite::params![s, t, k, d])).await;
    }

    pub async fn setting(&self, key: &str, default: &str) -> String {
        let k = key.to_string();
        let v = self.db.read(move |c| c.query_row("SELECT v FROM meta WHERE k=?", [k], |r| r.get::<_, String>(0)).map(Some).or_else(|e| if matches!(e, rusqlite::Error::QueryReturnedNoRows) { Ok(None) } else { Err(e) })).await;
        v.ok().flatten().unwrap_or_else(|| default.to_string())
    }

    pub async fn set_setting(&self, key: &str, value: &str) -> anyhow::Result<()> {
        let (k, v) = (key.to_string(), value.to_string());
        self.db.call(move |c| c.execute("INSERT INTO meta(k,v) VALUES(?,?) ON CONFLICT(k) DO UPDATE SET v=excluded.v", [k, v])).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn console_command_audit_entries_are_written_in_order_and_announced() {
        let st = crate::testutil::state(|_| {}).await;
        let mut rx = st.bus.subscribe();
        for i in 0..300 { st.audit_command("alice", "Survival", &format!("say {i}"), "s1").await; }
        st.audit_command("alice", "Survival", "login hunter2", "s1").await;
        let mut seen = vec![];
        while seen.len() < 301 {
            let e = tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv()).await.expect("activity event").unwrap();
            if e.kind == "activity" { seen.push((e.data["id"].as_i64().unwrap(), e.data["detail"].as_str().unwrap().to_string())); }
        }
        assert!(seen.windows(2).all(|w| w[0].0 < w[1].0), "ids increase in send order");
        assert_eq!(seen[7].1, "say 7");
        assert_eq!(seen[300].1, "[redacted: login]");
        let rows: Vec<(i64, String)> = st.db.read(|c| c.prepare("SELECT id, detail FROM activity WHERE action='console.command' ORDER BY id")?.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect()).await.unwrap();
        assert_eq!(rows, seen);
    }
}
