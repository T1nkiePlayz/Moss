//! The persisted delivery queue. Pending rows in `webhook_deliveries` are the queue, so queued events survive a
//! controller restart. Failed attempts are retried with exponential backoff up to `MAX_ATTEMPTS`, then the delivery is
//! marked `dead` (dead letter; it can be re-sent by hand). Consecutive failed attempts auto-disable an endpoint.
use super::{Endpoint, deliver::{self, Opts, Outcome}, secrets};
use crate::state::St;
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::{collections::HashSet, sync::{Mutex, atomic::{AtomicU64, Ordering}}, time::Duration};
use tokio::sync::{Notify, Semaphore};

pub const MAX_ATTEMPTS: i64 = 8;
/// Consecutive failed attempts (across deliveries) before an endpoint is switched off.
pub const AUTO_DISABLE_AFTER: i64 = 15;
pub const KEEP_LOG: i64 = 100;
const MAX_PENDING: i64 = 1000;
/// Seconds to wait after the nth failed attempt (1-based).
const BACKOFF_S: [u64; 7] = [5, 30, 120, 600, 1800, 3600, 7200];

static WAKE: Notify = Notify::const_new();
static IN_FLIGHT: Mutex<Option<HashSet<String>>> = Mutex::new(None);
static SCALE_PCT: AtomicU64 = AtomicU64::new(100);
static SEM: Semaphore = Semaphore::const_new(8);

pub fn wake() { WAKE.notify_one(); }
/// Tests shrink the retry delays (0 = retry immediately).
#[cfg(test)] pub fn set_backoff_scale_pct(p: u64) { SCALE_PCT.store(p, Ordering::SeqCst); }

/// The documented delay after the nth failed attempt.
pub fn backoff_base_ms(failed_attempts: i64) -> i64 { BACKOFF_S[(failed_attempts.max(1) as usize - 1).min(BACKOFF_S.len() - 1)] as i64 * 1000 }
fn backoff_ms(failed_attempts: i64) -> i64 { backoff_base_ms(failed_attempts) * SCALE_PCT.load(Ordering::Relaxed) as i64 / 100 }
fn now_ms() -> i64 { chrono::Utc::now().timestamp_millis() }

pub fn enqueue(c: &Connection, endpoint: &str, event_id: &str, kind: &str, body: &str, test: bool, time: &str) -> rusqlite::Result<String> {
    let id = uuid::Uuid::now_v7().to_string();
    if !test {
        let n: i64 = c.query_row("SELECT COUNT(*) FROM webhook_deliveries WHERE endpoint_id=? AND state='pending'", [endpoint], |r| r.get(0))?;
        if n >= MAX_PENDING {
            // the receiver has been gone for long: record the drop instead of growing the queue without bound
            c.execute("INSERT INTO webhook_deliveries(id,endpoint_id,event_id,event_type,body,state,created_at,finished_at,error) VALUES (?,?,?,?,?, 'dead',?,?, 'the queue for this endpoint was full')", params![id, endpoint, event_id, kind, "{}", time, time])?;
            return Ok(id);
        }
    }
    c.execute("INSERT INTO webhook_deliveries(id,endpoint_id,event_id,event_type,body,test,state,next_at,created_at) VALUES (?,?,?,?,?,?, 'pending', 0, ?)", params![id, endpoint, event_id, kind, body, test, time])?;
    Ok(id)
}

struct Due { id: String, endpoint: String, url: String, secret: String, kind: String, body: String, attempts: i64 }

fn in_flight<R>(f: impl FnOnce(&mut HashSet<String>) -> R) -> R { f(IN_FLIGHT.lock().unwrap_or_else(|p| p.into_inner()).get_or_insert_with(Default::default)) }

async fn due(st: &St) -> Vec<Due> {
    let now = now_ms();
    let rows: Vec<(String, String, String, String, String, i64)> = st.db.call(move |c| {
        c.prepare("SELECT d.id,d.endpoint_id,e.url,d.event_type,d.body,d.attempts FROM webhook_deliveries d JOIN webhook_endpoints e ON e.id=d.endpoint_id
                   WHERE d.state='pending' AND d.test=0 AND e.enabled=1 AND d.next_at<=? ORDER BY d.next_at, d.id LIMIT 64")?
            .query_map([now], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?.collect()
    }).await.unwrap_or_default();
    rows.into_iter().filter(|r| in_flight(|s| s.insert(r.0.clone()))).map(|(id, endpoint, url, kind, body, attempts)| {
        let secret = secrets::get(&st.cfg.data_dir, &endpoint).unwrap_or_default();
        Due { id, endpoint, url, secret, kind, body, attempts }
    }).collect()
}

/// Store the outcome of an attempt; returns whether the endpoint was just switched off.
pub fn record(c: &Connection, delivery: &str, endpoint: &str, attempts_before: i64, o: &Outcome, test: bool) -> rusqlite::Result<bool> {
    let attempts = attempts_before + 1;
    let now = crate::db::now();
    let (state, next, fin) = if o.ok { ("delivered", 0, Some(now.clone())) }
        else if test || attempts >= MAX_ATTEMPTS { ("dead", 0, Some(now.clone())) }
        else { ("pending", now_ms() + backoff_ms(attempts), None) };
    c.execute("UPDATE webhook_deliveries SET state=?,attempts=?,next_at=?,finished_at=?,status_code=?,latency_ms=?,response=?,error=? WHERE id=?",
        params![state, attempts, next, fin, o.status, o.latency_ms, o.response, o.error, delivery])?;
    let mut disabled = false;
    if !test {
        if o.ok { c.execute("UPDATE webhook_endpoints SET failures=0 WHERE id=?", [endpoint])?; }
        else {
            c.execute("UPDATE webhook_endpoints SET failures=failures+1 WHERE id=?", [endpoint])?;
            let (f, en): (i64, bool) = c.query_row("SELECT failures,enabled FROM webhook_endpoints WHERE id=?", [endpoint], |r| Ok((r.get(0)?, r.get(1)?)))?;
            if en && f >= AUTO_DISABLE_AFTER {
                let why = format!("Switched off automatically after {f} failed delivery attempts in a row.");
                c.execute("UPDATE webhook_endpoints SET enabled=0, disabled_reason=? WHERE id=?", params![why, endpoint])?;
                c.execute("UPDATE webhook_deliveries SET state='dead', finished_at=?, error=COALESCE(error,'')||' (endpoint switched off)' WHERE endpoint_id=? AND state='pending'", params![now, endpoint])?;
                disabled = true;
            }
        }
    }
    if state != "pending" {
        c.execute("DELETE FROM webhook_deliveries WHERE endpoint_id=?1 AND state!='pending' AND id NOT IN (SELECT id FROM webhook_deliveries WHERE endpoint_id=?1 AND state!='pending' ORDER BY created_at DESC, id DESC LIMIT ?2)", params![endpoint, KEEP_LOG])?;
    }
    Ok(disabled)
}

async fn run_one(st: St, d: Due, opts: Opts) {
    let _permit = SEM.acquire().await;
    let o = deliver::attempt(&d.url, &d.secret, &d.kind, &d.id, &d.body, opts).await;
    let (id, ep, att, o2) = (d.id.clone(), d.endpoint.clone(), d.attempts, o.clone());
    match st.db.call(move |c| record(c, &id, &ep, att, &o2, false)).await {
        Ok(true) => disabled_notice(&st, &d.endpoint).await,
        Ok(false) => {}
        Err(e) => tracing::warn!("webhooks: could not record a delivery result: {e:#}"),
    }
    in_flight(|s| s.remove(&d.id));
    if !o.ok { tracing::debug!("webhook delivery {} failed (attempt {})", d.id, d.attempts + 1); }
}

pub async fn disabled_notice(st: &St, endpoint: &str) {
    let name = super::get(st, endpoint).await.ok().flatten().map(|e| e.name).unwrap_or_default();
    crate::notify::send(st, "webhook_disabled", "warn", &format!("Webhook {name} was switched off"),
        "It failed too many times in a row. Fix the receiver, then turn it back on in Settings, Webhooks.", None).await;
    st.audit("system", "webhook.disable", &name, "too many consecutive failures", None).await;
}

/// Deliver everything that is due. With `wait` the call returns when all attempts are finished (tests).
pub async fn process_due(st: &St, opts: Opts, wait: bool) -> usize {
    let items = due(st).await;
    let n = items.len();
    let mut set = tokio::task::JoinSet::new();
    for d in items { set.spawn(run_one(st.clone(), d, opts)); }
    if wait { while set.join_next().await.is_some() {} } else { set.detach_all(); }
    n
}

/// One synchronous attempt for "send test event": recorded in the log, never retried, never counted towards auto-disable.
pub async fn send_test(st: &St, e: &Endpoint, opts: Opts, by: &str) -> anyhow::Result<Value> {
    let (event_id, time) = (uuid::Uuid::now_v7().to_string(), crate::db::now());
    let data = json!({"message": "This is a test event from Moss.", "sent_by": by});
    let body = super::body(&event_id, "test", &time, None, &data);
    let (ep, b, t2, ev2) = (e.id.clone(), body.clone(), time.clone(), event_id.clone());
    let id = st.db.call(move |c| enqueue(c, &ep, &ev2, "test", &b, true, &t2)).await?;
    let secret = secrets::get(&st.cfg.data_dir, &e.id).unwrap_or_default();
    let o = deliver::attempt(&e.url, &secret, "test", &id, &body, opts).await;
    let (id2, ep, o2) = (id.clone(), e.id.clone(), o.clone());
    st.db.call(move |c| record(c, &id2, &ep, 0, &o2, true)).await?;
    Ok(delivery_json(st, &id).await?.unwrap_or(Value::Null))
}

/// Put a dead delivery back in the queue.
pub async fn retry(st: &St, endpoint: &str, delivery: &str) -> anyhow::Result<bool> {
    let (e, d) = (endpoint.to_string(), delivery.to_string());
    let n = st.db.call(move |c| c.execute("UPDATE webhook_deliveries SET state='pending',attempts=0,next_at=0,finished_at=NULL WHERE id=? AND endpoint_id=? AND state='dead' AND test=0", params![d, e])).await?;
    if n > 0 { wake(); }
    Ok(n > 0)
}

pub fn row_json(r: &rusqlite::Row) -> rusqlite::Result<Value> {
    let next: i64 = r.get(11)?;
    Ok(json!({"id": r.get::<_, String>(0)?, "event_id": r.get::<_, String>(1)?, "type": r.get::<_, String>(2)?, "test": r.get::<_, bool>(3)?, "state": r.get::<_, String>(4)?,
        "attempts": r.get::<_, i64>(5)?, "max_attempts": MAX_ATTEMPTS, "created_at": r.get::<_, String>(6)?, "finished_at": r.get::<_, Option<String>>(7)?,
        "status_code": r.get::<_, Option<i64>>(8)?, "latency_ms": r.get::<_, Option<i64>>(9)?, "response": r.get::<_, Option<String>>(10)?,
        "next_attempt_at": (next > 0).then(|| chrono::DateTime::from_timestamp_millis(next).map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))), "error": r.get::<_, Option<String>>(12)?}))
}
const SEL: &str = "SELECT id,event_id,event_type,test,state,attempts,created_at,finished_at,status_code,latency_ms,response,next_at,error FROM webhook_deliveries";

pub async fn delivery_json(st: &St, id: &str) -> anyhow::Result<Option<Value>> {
    let id = id.to_string();
    st.db.call(move |c| c.query_row(&format!("{SEL} WHERE id=?"), [id], row_json).map(Some).or_else(|e| if matches!(e, rusqlite::Error::QueryReturnedNoRows) { Ok(None) } else { Err(e) })).await
}
pub async fn deliveries(st: &St, endpoint: &str) -> anyhow::Result<Vec<Value>> {
    let e = endpoint.to_string();
    st.db.call(move |c| c.prepare(&format!("{SEL} WHERE endpoint_id=? ORDER BY created_at DESC, id DESC LIMIT ?"))?.query_map(params![e, KEEP_LOG + 50], row_json)?.collect()).await
}

pub fn spawn(st: &St) {
    let st = st.clone();
    tokio::spawn(async move {
        let opts = Opts { allow_private: st.cfg.allow_private_webhooks, allow_http: false };
        loop {
            process_due(&st, opts, false).await;
            let wait = st.db.call(|c| c.query_row("SELECT MIN(d.next_at) FROM webhook_deliveries d JOIN webhook_endpoints e ON e.id=d.endpoint_id WHERE d.state='pending' AND e.enabled=1", [], |r| r.get::<_, Option<i64>>(0))).await.ok().flatten();
            let sleep = wait.map_or(30_000, |t| (t - now_ms()).clamp(250, 30_000)) as u64;
            tokio::select! { _ = WAKE.notified() => {}, _ = tokio::time::sleep(Duration::from_millis(sleep)) => {} }
        }
    });
}
