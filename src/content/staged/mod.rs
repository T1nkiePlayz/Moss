//! Staged updates ("Try this on a copy first"): clone a server into a hidden staging copy on a free port, apply the
//! planned change there, start it, watch it, and report. Only a healthy run can be promoted, which applies the same
//! plan to the real server with the usual pre-change backup and rollback.
//!
//! A staging copy is a real `servers` row (so the lifecycle, console and monitor code works unchanged) whose id is
//! also listed in `staging_runs`; `server::list` hides every id in that table.
use super::*;
use crate::{lifecycle, server};

mod api;
mod clone;
mod health;
mod run;

pub use api::{discard_run, estimate, get_run_json, list_runs, log, promote, start};

/// How long a finished copy is kept for log inspection.
pub const KEEP_HOURS: i64 = 24;
const ACTIVE: &[&str] = &["building", "testing"];

#[derive(Clone)]
pub(super) struct Run {
    pub id: String, pub origin_id: String, pub kind: String, pub request: Value, pub include_world: bool, pub state: String,
    pub report: Value, pub port: Option<i64>, pub op_id: Option<String>, pub actor: String, pub created_at: String, pub expires_at: String,
}

const COLS: &str = "id,origin_id,kind,request,include_world,state,report,port,op_id,actor,created_at,expires_at";

impl Run {
    fn from_row(r: &rusqlite::Row) -> rusqlite::Result<Run> {
        let js = |i: usize| -> rusqlite::Result<Value> { Ok(serde_json::from_str(&r.get::<_, String>(i)?).unwrap_or(Value::Null)) };
        Ok(Run { id: r.get(0)?, origin_id: r.get(1)?, kind: r.get(2)?, request: js(3)?, include_world: r.get(4)?, state: r.get(5)?, report: js(6)?, port: r.get(7)?, op_id: r.get(8)?,
            actor: r.get(9)?, created_at: r.get(10)?, expires_at: r.get(11)? })
    }
    pub fn expired(&self) -> bool { self.expires_at.as_str() < crate::db::now().as_str() }
    pub fn active(&self) -> bool { ACTIVE.contains(&self.state.as_str()) }
    pub fn json(&self) -> Value {
        let expired = self.expired() && !self.active();
        json!({
            "id": self.id, "server_id": self.origin_id, "kind": self.kind, "state": self.state, "include_world": self.include_world, "request": self.request,
            "report": self.report, "actor": self.actor, "port": self.port, "operation_id": self.op_id, "created_at": self.created_at, "expires_at": self.expires_at, "expired": expired,
            "can_promote": self.state == "healthy" && self.kind != "none" && !expired,
        })
    }
}

pub(super) async fn get_run(st: &St, id: &str) -> Option<Run> {
    let i = id.to_string();
    st.db.call(move |c| c.query_row(&format!("SELECT {COLS} FROM staging_runs WHERE id=?"), [i], Run::from_row)).await.ok()
}

pub(super) async fn runs_for(st: &St, origin: &str) -> Vec<Run> {
    let o = origin.to_string();
    st.db.call(move |c| {
        let mut q = c.prepare(&format!("SELECT {COLS} FROM staging_runs WHERE origin_id=? ORDER BY created_at DESC"))?;
        q.query_map([o], Run::from_row)?.collect()
    }).await.unwrap_or_default()
}

async fn all_runs(st: &St) -> Vec<Run> {
    st.db.call(|c| { let mut q = c.prepare(&format!("SELECT {COLS} FROM staging_runs ORDER BY created_at"))?; q.query_map([], Run::from_row)?.collect() }).await.unwrap_or_default()
}

/// Records a state change and tells the UI (events are addressed to the origin server).
pub(super) async fn set_state(st: &St, id: &str, state: &str, report: Option<Value>) {
    let (i, s, r, exp) = (id.to_string(), state.to_string(), report.map(|r| r.to_string()), (!ACTIVE.contains(&state)).then(|| (chrono::Utc::now() + chrono::Duration::hours(KEEP_HOURS)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true)));
    let _ = st.db.call(move |c| c.execute("UPDATE staging_runs SET state=?, report=COALESCE(?,report), expires_at=COALESCE(?,expires_at) WHERE id=?", rusqlite::params![s, r, exp, i])).await;
    announce(st, id).await;
}

pub(super) async fn announce(st: &St, id: &str) {
    if let Some(r) = get_run(st, id).await { st.emit("staging", json!({"server_id": r.origin_id, "run": r.json()})); }
}

/// Keeps a test copy off the network features that could clash with the real server or the outside world.
pub(super) fn sandbox_properties(dir: &std::path::Path) {
    let p = dir.join("server.properties");
    let Ok(raw) = std::fs::read_to_string(&p) else { return };
    let (mut rcon, mut query) = (false, false);
    let mut out: Vec<String> = raw.lines().map(|l| {
        let k = l.split('=').next().unwrap_or_default().trim();
        match k { "enable-rcon" => { rcon = true; "enable-rcon=false".into() }, "enable-query" => { query = true; "enable-query=false".into() }, _ => l.to_string() }
    }).collect();
    if !rcon { out.push("enable-rcon=false".into()); }
    if !query { out.push("enable-query=false".into()); }
    let _ = std::fs::write(&p, out.join("\n") + "\n");
}

/// Stops the copy if it runs, removes its files and every row that belongs to it.
pub async fn discard(st: &St, id: &str) -> anyhow::Result<()> {
    let Some(run) = get_run(st, id).await else { return Ok(()) };
    if let Ok(s) = server::get(st, id).await { lifecycle::halt(st, &s).await; }
    let lock = st.lock_for(id);
    let _g = lock.lock().await;
    let dir = st.cfg.servers_dir().join(id);
    if dir.starts_with(st.cfg.servers_dir()) && dir != st.cfg.servers_dir() { let _ = tokio::fs::remove_dir_all(&dir).await; }
    let i = id.to_string();
    st.db.call(move |c| {
        c.execute("DELETE FROM timeline WHERE server_id=?", [&i])?;
        c.execute("DELETE FROM servers WHERE id=?", [&i])?;
        c.execute("DELETE FROM staging_runs WHERE id=?", [&i])?;
        Ok(())
    }).await?;
    lifecycle::cleanup_cgroup(id);
    st.live.lock().unwrap().remove(id);
    st.hubs.lock().unwrap().remove(id);
    st.stats.lock().unwrap().remove(id);
    st.emit("staging", json!({"server_id": run.origin_id, "run": {"id": id, "server_id": run.origin_id, "state": "discarded", "deleted": true}}));
    st.emit("server", json!({"id": id, "deleted": true}));
    Ok(())
}

/// Discards every copy made from `origin` (the origin server is being deleted).
pub async fn discard_all_for(st: &St, origin: &str) {
    for r in runs_for(st, origin).await {
        if r.active() && let Some(op) = &r.op_id { ops::cancel(&st.clone(), op); }
        if let Err(e) = discard(st, &r.id).await { tracing::warn!(run = %r.id, "could not discard a staging copy: {e:#}"); }
    }
}

/// Startup: a copy that was still building or being tested was interrupted by the restart; stop whatever it left
/// running and remove it. Must run before the lifecycle reconciliation so no copy is re-attached or autostarted.
pub async fn recover(st: &St) -> anyhow::Result<()> {
    for r in all_runs(st).await {
        if !r.active() && !r.expired() { continue; }
        if let Ok(s) = server::get(st, &r.id).await && lifecycle::process_alive(&s) { let _ = lifecycle::kill(st, &s, "system").await; }
        tracing::info!(run = %r.id, state = %r.state, "removing a staging copy left over from before the restart");
        discard(st, &r.id).await?;
    }
    Ok(())
}

/// Removes copies whose 24 hours are up.
pub fn spawn_background(st: &St) {
    let st = st.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(300)).await;
            for r in all_runs(&st).await { if r.expired() && !r.active() { let _ = discard(&st, &r.id).await; } }
        }
    });
}

#[cfg(test)]
mod tests;
