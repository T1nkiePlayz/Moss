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

/// Critical when a server is set to container isolation but no usable Docker or Podman is reachable.
async fn container_finding(st: &St, s: &Srv) -> Option<Value> {
    if s.isolation != "container" && !s.in_container { return None; }
    let why = crate::container::engine(st).await.err()?;
    Some(finding(format!("{}:container", s.id), "critical", Some(&s.id), "Container engine not available",
        format!("This server is set to run in a container, but it cannot start that way: {why}"), vec!["isolation: container".into(), "no usable Docker or Podman found".into()],
        Some(json!({"label": "Open settings", "kind": "open_tab", "tab": "settings"}))))
}

/// Critical when a server is set to the game-only network but the engine cannot provide it (never falls back to open).
async fn network_finding(st: &St, s: &Srv) -> Option<Value> {
    if s.isolation != "container" || s.network == "open" { return None; }
    let eng = crate::container::engine(st).await.ok()?; // a missing engine is the container finding's business
    let why = tokio::task::spawn_blocking(move || eng.ensure_gameonly_network()).await.ok()?.err()?;
    Some(finding(format!("{}:network", s.id), "critical", Some(&s.id), "Game-only network not available",
        format!("This server is set to the game-only network, so it will not start until that works: {why}"), vec!["network: game-only".into()],
        Some(json!({"label": "Open settings", "kind": "open_tab", "tab": "settings"}))))
}

/// A server that needs the Minecraft EULA (Java servers and Bedrock) but has not recorded its acceptance.
pub fn eula_missing(s: &Srv) -> bool { s.needs_eula() && s.eula_at.is_none() }

pub async fn for_server(st: &St, s: &Srv, all: &[Srv]) -> Vec<Value> {
    let mut f = vec![];
    let (state, detail) = server::state_of(s, st);
    if eula_missing(s) {
        f.push(finding(format!("{}:eula", s.id), "critical", Some(&s.id), "EULA not accepted", "This server cannot start until the Minecraft EULA is accepted. Accept it here, or set eula=true in the server's eula.txt (Minecraft Java only).".into(), vec!["eula_at is empty".into()], Some(json!({"label": "Accept EULA", "kind": "accept_eula"}))));
    }
    if s.installed && s.is_mc() && crate::java::find_async(st, s.java_major as u32).await.is_none() {
        f.push(finding(format!("{}:java", s.id), "critical", Some(&s.id), &format!("Java {} is missing", s.java_major), "Moss can download a verified Temurin runtime for you.".into(), vec![format!("required major: {}", s.java_major)], Some(json!({"label": "Install Java", "kind": "install_java", "major": s.java_major}))));
    }
    if state == "crashed" || state == "failed" {
        f.push(finding(format!("{}:crash", s.id), "critical", Some(&s.id), if state == "crashed" { "Server crashed" } else { "Install failed" }, detail.clone().unwrap_or_default(), vec![format!("state: {state}")], Some(json!({"label": "Open console", "kind": "open_tab", "tab": "console"}))));
    }
    f.extend(limits_finding(s));
    f.extend(container_finding(st, s).await);
    f.extend(network_finding(st, s).await);
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
    f.extend(content_summary(st, s).await);
    f
}

/// One summary finding when the content check has warn/critical problems; details live in the Content tab.
async fn content_summary(st: &St, s: &Srv) -> Option<Value> {
    if !s.installed { return None; }
    let c = crate::content::compat(st, s)?;
    let found = crate::content::server_findings(st, s, &c).await.ok()?;
    let bad: Vec<_> = found.iter().filter(|x| x.is_problem()).collect();
    if bad.is_empty() { return None; }
    let sev = if bad.iter().any(|x| x.severity == "critical") { "critical" } else { "warn" };
    let title = if bad.len() == 1 { "1 content problem".to_string() } else { format!("{} content problems", bad.len()) };
    Some(finding(format!("{}:content", s.id), sev, Some(&s.id), &title, "Installed mods or plugins have missing dependencies, conflicts or other compatibility problems.".into(),
        bad.iter().take(5).map(|x| x.title.clone()).collect(), Some(json!({"label": "Review content", "kind": "open_tab", "tab": "content"}))))
}

const DAY_MS: f64 = 86_400_000.0;

/// Days until free space reaches `floor`, from a least-squares line over `(t ms, free bytes)` samples.
/// None unless there are at least 24 samples spanning 24 h or more and free space is falling.
pub fn forecast_days(samples: &[(i64, i64)], floor: i64) -> Option<f64> {
    let (slope, now) = trend(samples)?;
    Some(((now - floor as f64) / -slope / DAY_MS).max(0.0))
}

/// Least-squares fit of free space over time: (bytes per ms, fitted free bytes at the newest sample). `None` unless
/// there are at least 24 samples spanning a day and free space is falling.
fn trend(samples: &[(i64, i64)]) -> Option<(f64, f64)> {
    if samples.len() < 24 { return None; }
    let (t0, t1) = (samples.iter().map(|s| s.0).min()?, samples.iter().map(|s| s.0).max()?);
    if ((t1 - t0) as f64) < DAY_MS { return None; }
    let n = samples.len() as f64;
    let (mt, mf) = (samples.iter().map(|s| (s.0 - t0) as f64).sum::<f64>() / n, samples.iter().map(|s| s.1 as f64).sum::<f64>() / n);
    let (sxy, sxx) = samples.iter().fold((0.0, 0.0), |(a, b), s| { let dx = (s.0 - t0) as f64 - mt; (a + dx * (s.1 as f64 - mf), b + dx * dx) });
    if sxx <= 0.0 { return None; }
    let slope = sxy / sxx;
    if slope >= 0.0 { return None; }
    Some((slope, mf + slope * ((t1 - t0) as f64 - mt)))
}

/// Severity for a forecast: critical within 3 days, warn within 14, otherwise none.
fn forecast_severity(days: f64) -> Option<&'static str> { if days <= 3.0 { Some("critical") } else if days <= 14.0 { Some("warn") } else { None } }

/// The "disk will be full in about N days" finding from the last 7 days of hourly host roll-ups.
async fn disk_forecast(st: &St, total: i64) -> Option<Value> {
    let since = crate::history::hour_of(chrono::Utc::now().timestamp_millis()) - 7 * 24;
    let samples: Vec<(i64, i64)> = st.db.call(move |c| c.prepare("SELECT hour, disk_free FROM host_hourly WHERE hour>=? AND disk_free>0 ORDER BY hour")?.query_map([since], |r| Ok((r.get::<_, i64>(0)? * 3_600_000, r.get::<_, i64>(1)?)))?.collect::<Result<Vec<_>, _>>()).await.ok()?;
    let floor = (2i64 << 30).max(total / 20);
    let days = forecast_days(&samples, floor)?;
    let sev = forecast_severity(days)?;
    let gib = (1u64 << 30) as f64;
    let per_day = -trend(&samples)?.0 * DAY_MS / gib; // the fitted slope the forecast uses, so always positive
    let n = days.round().max(1.0) as i64;
    Some(finding("host:disk-forecast".into(), sev, None, &format!("Disk will be full in about {n} day{}", if n == 1 { "" } else { "s" }),
        format!("Free space on the data volume has been falling by roughly {per_day:.1} GiB per day. At that pace it reaches {:.1} GiB, where backups and saves start to fail, in about {n} day{}.", floor as f64 / gib, if n == 1 { "" } else { "s" }),
        vec![format!("trend: -{per_day:.1} GiB/day over {} hourly samples", samples.len()), format!("forecast: {days:.1} days to {:.1} GiB free", floor as f64 / gib)], None))
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
            let critical = pct < 5.0 || free < (2 << 30);
            if !critical { out.extend(disk_forecast(st, total as i64).await); }
            if critical { out.push(finding("host:disk".into(), "critical", None, "Disk almost full", format!("Only {:.1} GiB free on the data volume. Backups and world saves may fail.", free as f64 / (1u64 << 30) as f64), vec![format!("{pct:.1}% free")], None)); }
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

#[cfg(test)]
mod tests {
    use super::{eula_missing, forecast_days, forecast_severity};
    use crate::runtimes::launch::test_srv;

    const GIB: i64 = 1 << 30;
    fn series(n: i64, start: i64, per_hour: i64) -> Vec<(i64, i64)> { (0..n).map(|i| (i * 3_600_000, start + per_hour * i)).collect() }

    #[test]
    fn forecast_steady_decline() {
        let s = series(48, 200 * GIB, -GIB); // 1 GiB/hour = 24 GiB/day
        let d = forecast_days(&s, 2 * GIB).unwrap(); // 153 GiB left at the last sample
        assert!((d - (153.0 - 2.0) / 24.0).abs() < 0.01, "{d}");
    }
    #[test]
    fn forecast_none_when_rising_flat_or_short() {
        assert!(forecast_days(&series(48, 50 * GIB, GIB / 10), 2 * GIB).is_none());
        assert!(forecast_days(&series(48, 50 * GIB, 0), 2 * GIB).is_none());
        assert!(forecast_days(&series(23, 50 * GIB, -GIB / 10), 2 * GIB).is_none(), "fewer than 24 samples");
        let dense: Vec<_> = (0..40).map(|i| (i * 60_000, 50 * GIB - i)).collect();
        assert!(forecast_days(&dense, 2 * GIB).is_none(), "spans under 24 h");
    }
    #[test]
    fn forecast_thresholds() {
        assert_eq!(forecast_severity(0.5), Some("critical"));
        assert_eq!(forecast_severity(3.0), Some("critical"));
        assert_eq!(forecast_severity(3.1), Some("warn"));
        assert_eq!(forecast_severity(14.0), Some("warn"));
        assert_eq!(forecast_severity(14.1), None);
    }

    #[test]
    fn bedrock_needs_the_eula_before_it_can_start() {
        let mut b = test_srv("bedrock", "1.21", None, "balanced");
        assert!(eula_missing(&b));
        b.eula_at = Some("2025-01-01T00:00:00Z".into());
        assert!(!eula_missing(&b));
        assert!(eula_missing(&test_srv("paper", "1.21", None, "balanced")));
        assert!(!eula_missing(&test_srv("terraria", "1.4", None, "balanced")) && !eula_missing(&test_srv("custom", "custom", None, "balanced")));
    }
}
