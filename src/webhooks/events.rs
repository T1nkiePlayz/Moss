//! Turns what the controller already tracks into webhook events, without touching the code that produces it: a
//! subscriber on the internal bus (server state and player list, operations, notifications) plus a slow health poll.
//! `Deriver` is pure so the rules are testable. Limits, by design:
//! * Player events come from the status ping (polled every 5 s): a join and leave inside one poll is missed, and
//!   Minecraft only lists up to 12 names, so with more players online some joins/leaves are not reported.
//!   Servers that hide their player sample report no player events.
//! * A server that was already running when the controller started does not produce `server.started`.
use super::{Ev, publish};
use crate::state::St;
use serde_json::{Value, json};
use std::{collections::{HashMap, HashSet, VecDeque}, time::{Duration, Instant}};

/// A start within this long after a stop or crash is reported as a restart.
const RESTART_WINDOW: Duration = Duration::from_secs(120);

#[derive(Default)]
struct Track { state: String, down_at: Option<Instant>, players: Option<Vec<String>> }

#[derive(Default)]
pub struct Deriver { servers: HashMap<String, Track>, seen_ops: HashSet<String>, seen_order: VecDeque<String> }

fn ev(kind: &'static str, id: &str, name: &str, data: Value) -> Ev { Ev { kind, server: Some((id.into(), name.into())), data } }

impl Deriver {
    pub fn on_server(&mut self, v: &Value, now: Instant) -> Vec<Ev> {
        let (Some(id), Some(state)) = (v["id"].as_str(), v["state"].as_str()) else { return vec![] }; // deletions carry no state
        let name = v["name"].as_str().unwrap_or("");
        let t = self.servers.entry(id.to_string()).or_default();
        let mut out = vec![];
        let prev = std::mem::replace(&mut t.state, state.to_string());
        if !prev.is_empty() && prev != state {
            match state {
                "running" => {
                    let restarted = t.down_at.is_some_and(|d| now.duration_since(d) < RESTART_WINDOW);
                    t.down_at = None;
                    out.push(ev(if restarted { "server.restarted" } else { "server.started" }, id, name, json!({"previous_state": prev})));
                }
                "stopped" if matches!(prev.as_str(), "stopping" | "running" | "starting") => { t.down_at = Some(now); out.push(ev("server.stopped", id, name, json!({}))); }
                "crashed" => { t.down_at = Some(now); out.push(ev("server.crashed", id, name, json!({"detail": v["state_detail"]}))); }
                "stopping" => t.down_at = Some(now),
                _ => {}
            }
        }
        // players: only while the server reports a list; a gap (stopped, starting) resets the baseline silently
        match v["players"].as_object().map(|_| &v["players"]) {
            Some(p) => {
                let names: Vec<String> = p["sample"].as_array().map(|a| a.iter().filter_map(|x| x.as_str()).map(|s| s.chars().take(32).collect()).collect()).unwrap_or_default();
                if let Some(old) = &t.players {
                    let data = |n: &String| json!({"player": n, "online": p["online"], "max": p["max"]});
                    for n in names.iter().filter(|n| !old.contains(n)) { out.push(ev("player.joined", id, name, data(n))); }
                    for n in old.iter().filter(|n| !names.contains(n)) { out.push(ev("player.left", id, name, data(n))); }
                }
                t.players = Some(names);
            }
            None => t.players = None,
        }
        out
    }

    fn first_time(&mut self, id: &str) -> bool {
        if !self.seen_ops.insert(id.to_string()) { return false; }
        self.seen_order.push_back(id.to_string());
        while self.seen_order.len() > 2000 { if let Some(o) = self.seen_order.pop_front() { self.seen_ops.remove(&o); } }
        true
    }

    pub fn on_op(&mut self, op: &Value, server_name: impl Fn(&str) -> String) -> Vec<Ev> {
        let (Some(id), Some(state), Some(kind)) = (op["id"].as_str(), op["state"].as_str(), op["kind"].as_str()) else { return vec![] };
        if !matches!(state, "completed" | "failed") || !self.first_time(id) { return vec![]; }
        let ok = state == "completed";
        let sid = op["server_id"].as_str();
        let server = sid.map(|s| (s.to_string(), server_name(s)));
        let data = json!({"operation": {"id": id, "kind": kind, "title": op["title"], "state": state, "error": op["error"]}});
        let mut out = vec![Ev { kind: if ok { "operation.completed" } else { "operation.failed" }, server: server.clone(), data: data.clone() }];
        let specific = match (kind, ok) {
            ("backup.create", true) => Some("backup.completed"),
            ("backup.create" | "backup.upload", false) => Some("backup.failed"),
            ("content.install" | "modpack.import", true) => Some("content.installed"),
            _ => None,
        };
        if let Some(k) = specific { out.push(Ev { kind: k, server, data }); }
        out
    }
}

/// Backup facts for `backup.completed`, best effort.
async fn enrich_backup(st: &St, e: &mut Ev) {
    let Some((sid, _)) = e.server.clone() else { return };
    let row: Option<Value> = st.db.call(move |c| c.query_row("SELECT id,size,created_at,kind,sha256 FROM backups WHERE server_id=? AND status='ok' ORDER BY created_at DESC LIMIT 1", [sid],
        |r| Ok(json!({"id": r.get::<_, String>(0)?, "size": r.get::<_, i64>(1)?, "created_at": r.get::<_, String>(2)?, "kind": r.get::<_, String>(3)?, "sha256": r.get::<_, String>(4)?}))).map(Some).or_else(|e| if matches!(e, rusqlite::Error::QueryReturnedNoRows) { Ok(None) } else { Err(e) })).await.ok().flatten();
    if let Some(b) = row { e.data["backup"] = b; }
}

fn system_admin() -> crate::auth::User { crate::auth::User { id: "system".into(), email: "system".into(), role: "owner".into(), token: None, via_token: None, aal: None } }

async fn wanted(st: &St, kind: &str) -> bool {
    super::list(st).await.is_ok_and(|l| l.iter().any(|e| e.enabled && e.events.iter().any(|k| k == kind))) || crate::workflows::wants(st, kind).await
}

pub fn spawn(st: &St) {
    let st1 = st.clone();
    tokio::spawn(async move {
        let mut rx = st1.bus.subscribe();
        let mut d = Deriver::default();
        loop {
            let e = match rx.recv().await {
                Ok(e) => e,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            };
            let mut out = match e.kind {
                "server" => { if e.data["id"].as_str().is_some() && crate::server::is_staging(&st1, e.data["id"].as_str().unwrap_or("")).await { vec![] } else { d.on_server(&e.data, Instant::now()) } }
                "op" => {
                    // Only finished operations produce events, and only those need the server's name.
                    let name = match (e.data["state"].as_str(), e.data["server_id"].as_str()) {
                        (Some("completed" | "failed"), Some(sid)) => crate::server::get(&st1, sid).await.map(|s| s.name).unwrap_or_default(),
                        _ => String::new(),
                    };
                    d.on_op(&e.data, |_| name.clone())
                }
                "notification" if e.data["kind"] == "update_available" => vec![Ev { kind: "update.available", server: None, data: json!({"title": e.data["title"], "body": e.data["body"]}) }],
                _ => vec![],
            };
            for ev in &mut out { if ev.kind == "backup.completed" { enrich_backup(&st1, ev).await; } }
            for ev in out { publish(&st1, ev).await; }
        }
    });
    let st2 = st.clone();
    tokio::spawn(async move {
        let mut known: HashSet<String> = HashSet::new();
        let mut first = true;
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            if !wanted(&st2, "health.critical").await { known.clear(); continue; }
            let Ok(findings) = crate::health::collect(&st2, &system_admin()).await else { continue };
            let crit: Vec<&Value> = findings.iter().filter(|f| f["severity"] == "critical").collect();
            let now: HashSet<String> = crit.iter().filter_map(|f| f["id"].as_str().map(String::from)).collect();
            if !first {
                for f in crit.iter().filter(|f| f["id"].as_str().is_some_and(|i| !known.contains(i))) {
                    let server = match f["server_id"].as_str() { Some(id) => crate::server::get(&st2, id).await.ok().map(|s| (s.id, s.name)), None => None };
                    publish(&st2, Ev { kind: "health.critical", server, data: json!({"finding": {"id": f["id"], "title": f["title"], "detail": f["detail"], "evidence": f["evidence"]}}) }).await;
                }
            }
            known = now;
            first = false; // what was already critical when the poll began is not news
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sv(state: &str, players: Option<(u32, &[&str])>) -> Value {
        json!({"id": "s1", "name": "Survival", "state": state, "state_detail": null,
            "players": players.map(|(n, s)| json!({"online": n, "max": 20, "sample": s}))})
    }
    fn kinds(v: Vec<Ev>) -> Vec<&'static str> { v.into_iter().map(|e| e.kind).collect() }

    #[test]
    fn lifecycle_transitions_become_events_and_the_first_sight_is_silent() {
        let (mut d, t0) = (Deriver::default(), Instant::now());
        assert!(d.on_server(&sv("running", None), t0).is_empty(), "already running at boot");
        assert_eq!(kinds(d.on_server(&sv("stopping", None), t0)), Vec::<&str>::new());
        assert_eq!(kinds(d.on_server(&sv("stopped", None), t0)), ["server.stopped"]);
        // a start long after the stop is a plain start
        d.on_server(&sv("starting", None), t0 + Duration::from_secs(600));
        assert_eq!(kinds(d.on_server(&sv("running", None), t0 + Duration::from_secs(620))), ["server.started"]);
        assert_eq!(kinds(d.on_server(&sv("crashed", None), t0 + Duration::from_secs(700))), ["server.crashed"]);
        d.on_server(&sv("starting", None), t0 + Duration::from_secs(710));
        assert_eq!(kinds(d.on_server(&sv("running", None), t0 + Duration::from_secs(730))), ["server.restarted"]);
        // the same state repeated (the 5 s sampler re-emits the server) is not a transition
        assert!(d.on_server(&sv("running", None), t0 + Duration::from_secs(740)).is_empty());
        // stop then start within two minutes = restart
        d.on_server(&sv("stopping", None), t0 + Duration::from_secs(800));
        d.on_server(&sv("stopped", None), t0 + Duration::from_secs(805));
        d.on_server(&sv("starting", None), t0 + Duration::from_secs(806));
        assert_eq!(kinds(d.on_server(&sv("running", None), t0 + Duration::from_secs(830))), ["server.restarted"]);
    }

    #[test]
    fn players_are_diffed_from_the_sample_with_a_silent_baseline() {
        let (mut d, t) = (Deriver::default(), Instant::now());
        d.on_server(&sv("running", Some((1, &["Alex"]))), t);
        let e = d.on_server(&sv("running", Some((2, &["Alex", "Steve"]))), t);
        assert_eq!(kinds(e.iter().map(|e| Ev { kind: e.kind, server: None, data: Value::Null }).collect()), ["player.joined"]);
        assert_eq!(e[0].data["player"], "Steve");
        assert_eq!(e[0].data["online"], 2);
        let e = d.on_server(&sv("running", Some((1, &["Steve"]))), t);
        assert_eq!((e[0].kind, e[0].data["player"].as_str()), ("player.left", Some("Alex")));
        // while the server is not reporting players the baseline resets: nobody "joins" when it comes back
        d.on_server(&sv("stopping", None), t);
        assert!(d.on_server(&sv("running", Some((1, &["Alex"]))), t).iter().all(|e| !e.kind.starts_with("player")));
    }

    #[test]
    fn operations_fire_once_with_the_specific_event() {
        let mut d = Deriver::default();
        let name = |_: &str| "Survival".to_string();
        let op = |kind: &str, state: &str| json!({"id": format!("{kind}-{state}"), "kind": kind, "title": "t", "state": state, "server_id": "s1", "error": null});
        assert!(d.on_op(&op("backup.create", "running"), name).is_empty());
        assert_eq!(kinds(d.on_op(&op("backup.create", "completed"), name)), ["operation.completed", "backup.completed"]);
        assert!(d.on_op(&op("backup.create", "completed"), name).is_empty(), "the same operation is reported once");
        assert_eq!(kinds(d.on_op(&op("backup.create", "failed"), name)), ["operation.failed", "backup.failed"]);
        assert_eq!(kinds(d.on_op(&op("content.install", "completed"), name)), ["operation.completed", "content.installed"]);
        assert_eq!(kinds(d.on_op(&op("java.install", "failed"), name)), ["operation.failed"]);
        assert!(d.on_op(&op("backup.create", "cancelled"), name).is_empty());
    }
}
