//! Evidence-backed findings (never an opaque score). Deterministic rules over durable + live state.
use crate::{auth::{self, User}, error::ApiResult, lifecycle, server::{self, Srv}, state::St};
use axum::{Json, extract::{Path, State}};
use serde_json::{Value, json};

fn finding(id: String, sev: &str, server: Option<&str>, title: &str, detail: String, evidence: Vec<String>, action: Option<Value>) -> Value {
    json!({"id": id, "severity": sev, "server_id": server, "title": title, "detail": detail, "evidence": evidence, "action": action})
}

/// Info finding when limits were requested but this host cannot enforce them (no cgroup v2 delegation, macOS).
fn limits_finding(s: &Srv) -> Option<Value> {
    let req = s.requested_limits();
    if !req.any() { return None; }
    let v = crate::platform::cgroup::limits_json(&req);
    if v["enforced"] == true { return None; }
    let why = v["detail"].as_str().unwrap_or("resource limits are unavailable").to_string();
    Some(finding(format!("{}:limits", s.id), "info", Some(&s.id), "Resource limits are not enforced", format!("Limits are saved for this server but cannot be applied on this host: {why}"), vec![format!("backend: {}", v["backend"].as_str().unwrap_or("none"))], None))
}

pub async fn for_server(st: &St, s: &Srv, all: &[Srv]) -> Vec<Value> {
    let mut f = vec![];
    let (state, detail) = server::state_of(s, st);
    if s.is_mc() && s.eula_at.is_none() {
        f.push(finding(format!("{}:eula", s.id), "critical", Some(&s.id), "EULA not accepted", "This server cannot start until the Minecraft EULA is accepted.".into(), vec!["eula_at is empty".into()], Some(json!({"label": "Accept EULA", "kind": "accept_eula"}))));
    }
    if s.installed && s.is_mc() && crate::java::find(st, s.java_major as u32).is_none() {
        f.push(finding(format!("{}:java", s.id), "critical", Some(&s.id), &format!("Java {} is missing", s.java_major), "Moss can download a verified Temurin runtime for you.".into(), vec![format!("required major: {}", s.java_major)], Some(json!({"label": "Install Java", "kind": "install_java", "major": s.java_major}))));
    }
    if state == "crashed" || state == "failed" {
        f.push(finding(format!("{}:crash", s.id), "critical", Some(&s.id), if state == "crashed" { "Server crashed" } else { "Install failed" }, detail.clone().unwrap_or_default(), vec![format!("state: {state}")], Some(json!({"label": "Open console", "kind": "open_tab", "tab": "console"}))));
    }
    f.extend(limits_finding(s));
    if all.iter().any(|o| o.id != s.id && o.port == s.port) {
        f.push(finding(format!("{}:port", s.id), "warn", Some(&s.id), "Port shared with another server", format!("Port {} is assigned to more than one Moss server; only one can run at a time.", s.port), vec![format!("port {}", s.port)], Some(json!({"label": "Open settings", "kind": "open_tab", "tab": "settings"}))));
    }
    if state == "starting" {
        let slow = st.live.lock().unwrap().get(&s.id).and_then(|l| l.started).is_some_and(|t| t.elapsed().as_secs() > 180);
        if slow { f.push(finding(format!("{}:slow", s.id), "warn", Some(&s.id), "Slow to become ready", "The server has been starting for over 3 minutes without reporting readiness.".into(), vec!["no 'Done' log line or status response yet".into()], Some(json!({"label": "Open console", "kind": "open_tab", "tab": "console"})))); }
    }
    if s.installed {
        let sid = s.id.clone();
        let last: Option<String> = st.db.call(move |c| c.query_row("SELECT MAX(created_at) FROM backups WHERE server_id=? AND status='ok'", [sid], |r| r.get(0))).await.ok().flatten();
        let age_days = last.as_deref().and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok()).map(|t| (chrono::Utc::now() - t.with_timezone(&chrono::Utc)).num_days());
        let created_days = chrono::DateTime::parse_from_rfc3339(&s.created_at).map(|t| (chrono::Utc::now() - t.with_timezone(&chrono::Utc)).num_days()).unwrap_or(0);
        match age_days {
            None if created_days >= 1 => f.push(finding(format!("{}:nobackup", s.id), "warn", Some(&s.id), "No backup yet", "This server has never been backed up. A backup that exists is the only recovery point you have.".into(), vec!["0 verified backups".into()], Some(json!({"label": "Back up now", "kind": "backup"})))),
            Some(d) if d >= 7 => f.push(finding(format!("{}:oldbackup", s.id), "info", Some(&s.id), "Backup is getting old", format!("Last successful backup was {d} days ago."), vec![format!("last backup: {}", last.unwrap_or_default())], Some(json!({"label": "Back up now", "kind": "backup"})))),
            _ => {}
        }
    }
    f
}

pub async fn collect(st: &St, user: &User) -> ApiResult<Vec<Value>> {
    let vis = auth::visible_servers(st, user).await?;
    let all = server::list(st).await?;
    let mut out = vec![];
    for s in all.iter().filter(|s| vis.as_ref().is_none_or(|v| v.contains(&s.id))) { out.extend(for_server(st, s, &all).await); }
    if user.is_admin() {
        let (total, free) = crate::platform::disk(&st.cfg.data_dir);
        if total > 0 {
            let pct = free as f64 / total as f64 * 100.0;
            if pct < 5.0 || free < (2 << 30) { out.push(finding("host:disk".into(), "critical", None, "Disk almost full", format!("Only {:.1} GiB free on the data volume. Backups and world saves may fail.", free as f64 / (1u64 << 30) as f64), vec![format!("{pct:.1}% free")], None)); }
            else if pct < 15.0 { out.push(finding("host:disk".into(), "warn", None, "Disk space is getting low", format!("{:.1} GiB free ({pct:.0}%).", free as f64 / (1u64 << 30) as f64), vec![format!("{pct:.1}% free")], None)); }
        }
    }
    let rank = |v: &Value| match v["severity"].as_str() { Some("critical") => 0, Some("warn") => 1, _ => 2 };
    out.sort_by_key(rank);
    Ok(out)
}

pub async fn all(State(st): State<St>, user: User) -> ApiResult<Json<Value>> { Ok(Json(Value::Array(collect(&st, &user).await?))) }

pub async fn server_health(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let s = crate::api::server_for(&st, &user, &id, "server.view").await?;
    let all = server::list(&st).await?;
    let _ = lifecycle::process_alive(&s);
    Ok(Json(Value::Array(for_server(&st, &s, &all).await)))
}
