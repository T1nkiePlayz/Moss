//! Scheduler: interval or daily-at schedules with an explicit UTC offset. After downtime a missed schedule runs
//! once and is rescheduled from now (a backlog of runs is never replayed).
use crate::{api::server_for, auth::User, error::{ApiError, ApiResult}, lifecycle, ops, server, state::St};
use axum::{Json, extract::{Path, State}, http::StatusCode};
use chrono::{DateTime, Duration as CD, NaiveTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};

fn parse_hhmm(s: &str) -> Option<NaiveTime> { NaiveTime::parse_from_str(s, "%H:%M").ok() }

pub fn next_run(every: Option<i64>, daily: Option<&str>, tz_min: i64, from: DateTime<Utc>) -> Option<DateTime<Utc>> {
    if let Some(m) = every { return Some(from + CD::minutes(m.max(1))); }
    let t = parse_hhmm(daily?)?;
    let local = from + CD::minutes(tz_min);
    let mut cand = local.date_naive().and_time(t);
    if cand <= local.naive_utc() { cand += CD::days(1); }
    Some(DateTime::<Utc>::from_naive_utc_and_offset(cand, Utc) - CD::minutes(tz_min))
}

const SEL: &str = "SELECT id,server_id,kind,command,every_minutes,daily_at,tz_offset_min,enabled,keep,last_run,next_run,last_status FROM schedules";
fn row_json(r: &rusqlite::Row) -> rusqlite::Result<Value> {
    Ok(json!({"id": r.get::<_, String>(0)?, "server_id": r.get::<_, String>(1)?, "kind": r.get::<_, String>(2)?, "command": r.get::<_, Option<String>>(3)?, "every_minutes": r.get::<_, Option<i64>>(4)?,
        "daily_at": r.get::<_, Option<String>>(5)?, "tz_offset_min": r.get::<_, i64>(6)?, "enabled": r.get::<_, bool>(7)?, "keep": r.get::<_, Option<i64>>(8)?,
        "last_run": r.get::<_, Option<String>>(9)?, "next_run": r.get::<_, Option<String>>(10)?, "last_status": r.get::<_, Option<String>>(11)?}))
}

pub async fn list(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    server_for(&st, &user, &id, "server.view").await?;
    let v: Vec<Value> = st.db.call(move |c| { let mut q = c.prepare(&format!("{SEL} WHERE server_id=? ORDER BY kind,id"))?; q.query_map([id], row_json)?.collect() }).await?;
    Ok(Json(json!(v)))
}

#[derive(Deserialize)]
pub struct Req { kind: Option<String>, command: Option<String>, every_minutes: Option<i64>, daily_at: Option<String>, tz_offset_min: Option<i64>, enabled: Option<bool>, keep: Option<i64> }

fn validate(r: &Req, creating: bool) -> ApiResult<()> {
    if creating {
        let kind = r.kind.as_deref().unwrap_or_default();
        if !["backup", "restart", "start", "stop", "command"].contains(&kind) { return Err(ApiError::bad("validation", "unknown kind").field("kind", "backup | restart | start | stop | command")); }
        if kind == "command" && r.command.as_deref().is_none_or(|c| c.trim().is_empty()) { return Err(ApiError::bad("validation", "command required").field("command", "required")); }
        if r.every_minutes.is_none() == r.daily_at.is_none() { return Err(ApiError::bad("validation", "set exactly one of every_minutes or daily_at")); }
    }
    if let Some(c) = &r.command && (c.len() > 200 || c.contains(['\n', '\r'])) { return Err(ApiError::bad("validation", "command must be a single line up to 200 chars").field("command", "invalid")); }
    if let Some(m) = r.every_minutes && !(5..=525_600).contains(&m) { return Err(ApiError::bad("validation", "interval must be 5 minutes to 1 year").field("every_minutes", "5–525600")); }
    if let Some(d) = &r.daily_at && parse_hhmm(d).is_none() { return Err(ApiError::bad("validation", "use HH:MM").field("daily_at", "HH:MM")); }
    if let Some(t) = r.tz_offset_min && !(-840..=840).contains(&t) { return Err(ApiError::bad("validation", "offset out of range").field("tz_offset_min", "-840–840")); }
    Ok(())
}

pub async fn create(State(st): State<St>, user: User, Path(id): Path<String>, Json(r): Json<Req>) -> ApiResult<(StatusCode, Json<Value>)> {
    validate(&r, true)?;
    let s = server_for(&st, &user, &id, if r.kind.as_deref() == Some("backup") { "backup.manage" } else { "server.power" }).await?;
    if r.kind.as_deref() == Some("command") { crate::auth::authz(&st, &user, &id, "server.console.write").await?; }
    let sid = uuid::Uuid::now_v7().to_string();
    let tz = r.tz_offset_min.unwrap_or(0);
    let next = next_run(r.every_minutes, r.daily_at.as_deref(), tz, Utc::now()).map(|t| t.to_rfc3339());
    let (i, srv, kind, cmd, ev, da, en, keep) = (sid.clone(), id.clone(), r.kind.clone().unwrap(), r.command.clone(), r.every_minutes, r.daily_at.clone(), r.enabled.unwrap_or(true), r.keep.map(|k| k.clamp(1, 365)));
    st.db.call(move |c| c.execute("INSERT INTO schedules(id,server_id,kind,command,every_minutes,daily_at,tz_offset_min,enabled,keep,next_run) VALUES (?,?,?,?,?,?,?,?,?,?)", rusqlite::params![i, srv, kind, cmd, ev, da, tz, en, keep, next])).await?;
    st.audit(&user.email, "schedule.create", &s.name, r.kind.as_deref().unwrap_or(""), Some(&id)).await;
    let v = get_one(&st, &sid).await?;
    Ok((StatusCode::CREATED, Json(v)))
}

async fn get_one(st: &St, id: &str) -> ApiResult<Value> {
    let i = id.to_string();
    st.db.call(move |c| c.query_row(&format!("{SEL} WHERE id=?"), [i], row_json)).await.map_err(|_| ApiError::not_found("schedule"))
}

async fn authorize(st: &St, user: &User, sched: &Value) -> ApiResult<()> {
    let sid = sched["server_id"].as_str().unwrap_or_default();
    let perm = if sched["kind"] == "backup" { "backup.manage" } else if sched["kind"] == "command" { "server.console.write" } else { "server.power" };
    crate::auth::authz(st, user, sid, perm).await
}

pub async fn patch(State(st): State<St>, user: User, Path(id): Path<String>, Json(r): Json<Req>) -> ApiResult<Json<Value>> {
    let cur = get_one(&st, &id).await?;
    authorize(&st, &user, &cur).await?;
    validate(&r, false)?;
    let every = r.every_minutes.or(cur["every_minutes"].as_i64()).filter(|_| r.daily_at.is_none());
    let daily = r.daily_at.clone().or(cur["daily_at"].as_str().map(String::from)).filter(|_| r.every_minutes.is_none());
    let tz = r.tz_offset_min.or(cur["tz_offset_min"].as_i64()).unwrap_or(0);
    let next = next_run(every, daily.as_deref(), tz, Utc::now()).map(|t| t.to_rfc3339());
    let i = id.clone();
    st.db.call(move |c| c.execute("UPDATE schedules SET command=COALESCE(?,command), every_minutes=?, daily_at=?, tz_offset_min=?, enabled=COALESCE(?,enabled), keep=COALESCE(?,keep), next_run=? WHERE id=?",
        rusqlite::params![r.command, every, daily, tz, r.enabled, r.keep.map(|k| k.clamp(1, 365)), next, i])).await?;
    Ok(Json(get_one(&st, &id).await?))
}

pub async fn delete(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<StatusCode> {
    let cur = get_one(&st, &id).await?;
    authorize(&st, &user, &cur).await?;
    st.db.call(move |c| c.execute("DELETE FROM schedules WHERE id=?", [id])).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn run_now(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let cur = get_one(&st, &id).await?;
    authorize(&st, &user, &cur).await?;
    execute(&st, &cur, &user.email).await;
    Ok(Json(get_one(&st, &id).await?))
}

async fn execute(st: &St, sc: &Value, actor: &str) {
    let (id, sid, kind) = (sc["id"].as_str().unwrap_or_default().to_string(), sc["server_id"].as_str().unwrap_or_default().to_string(), sc["kind"].as_str().unwrap_or_default().to_string());
    let status = match server::get(st, &sid).await {
        Err(_) => "server missing".to_string(),
        Ok(s) => {
            let res: Result<String, String> = match kind.as_str() {
                "backup" => {
                    let keep = sc["keep"].as_i64();
                    let s2 = s.clone();
                    ops::spawn_op(st, "backup.create", format!("Scheduled backup of {}", s.name), Some(&sid), true, move |ctx| async move { let st = ctx.st.clone(); crate::backups::do_backup(&ctx, &st, &s2, "scheduled", "scheduled", keep).await.map(|_| ()) });
                    Ok("backup started".into())
                }
                "start" => lifecycle::start(st, &s, actor).await.map(|_| "started".into()).map_err(|e| e.message),
                "stop" => lifecycle::stop(st, &s, actor).await.map(|_| "stopping".into()).map_err(|e| e.message),
                "restart" => lifecycle::restart(st, &s, actor).await.map(|_| "restarting".into()).map_err(|e| e.message),
                "command" => lifecycle::send_command(st, &s, sc["command"].as_str().unwrap_or_default()).await.map(|_| "command sent".into()).map_err(|e| e.message),
                _ => Err("unknown kind".into()),
            };
            match res { Ok(m) => m, Err(e) => format!("skipped: {e}") }
        }
    };
    let (every, daily, tz) = (sc["every_minutes"].as_i64(), sc["daily_at"].as_str().map(String::from), sc["tz_offset_min"].as_i64().unwrap_or(0));
    let next = next_run(every, daily.as_deref(), tz, Utc::now()).map(|t| t.to_rfc3339());
    let _ = st.db.call(move |c| c.execute("UPDATE schedules SET last_run=?, last_status=?, next_run=? WHERE id=?", rusqlite::params![crate::db::now(), status, next, id])).await;
}

pub fn spawn_background(st: &St) {
    let st = st.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(15)).await;
            let now = Utc::now().to_rfc3339();
            let due: Vec<Value> = st.db.call(move |c| { let mut q = c.prepare(&format!("{SEL} WHERE enabled=1 AND next_run IS NOT NULL AND next_run<=?"))?; q.query_map([now], row_json)?.collect() }).await.unwrap_or_default();
            for sc in due { execute(&st, &sc, "schedule").await; }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn interval_schedules_add_minutes() {
        let t = Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap();
        assert_eq!(next_run(Some(30), None, 0, t).unwrap(), t + CD::minutes(30));
    }

    #[test]
    fn daily_respects_utc_offset() {
        // 03:00 at UTC+8 is 19:00 UTC the previous day
        let t = Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap();
        let n = next_run(None, Some("03:00"), 480, t).unwrap();
        assert_eq!(n, Utc.with_ymd_and_hms(2026, 1, 1, 19, 0, 0).unwrap());
        // already past today's slot -> tomorrow
        let t2 = Utc.with_ymd_and_hms(2026, 1, 1, 20, 0, 0).unwrap();
        assert_eq!(next_run(None, Some("03:00"), 480, t2).unwrap(), Utc.with_ymd_and_hms(2026, 1, 2, 19, 0, 0).unwrap());
    }

    #[test]
    fn invalid_time_is_rejected() { assert!(next_run(None, Some("25:99"), 0, Utc::now()).is_none()); }
}
