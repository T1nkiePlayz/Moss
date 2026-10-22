//! Notifications: durable in-app records (bell + SSE `notification` event) and optional webhook delivery.
//! `send` is the single entry point for producers; it de-duplicates (same kind + server within 10 minutes is
//! reported once), stores, pushes to subscribers and, when the kind is enabled and a webhook is configured,
//! delivers off the request path. Producers: crash handling, backups, operations, the disk check and restarts.
use crate::{platform, server::Srv, state::St};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::time::Duration;

pub mod push;
pub mod rules;
pub mod webhook;

pub fn row_json(r: &rusqlite::Row) -> rusqlite::Result<Value> {
    Ok(json!({"id": r.get::<_, String>(0)?, "time": r.get::<_, String>(1)?, "kind": r.get::<_, String>(2)?, "severity": r.get::<_, String>(3)?,
        "title": r.get::<_, String>(4)?, "body": r.get::<_, String>(5)?, "server_id": r.get::<_, Option<String>>(6)?, "read": r.get::<_, bool>(7)?}))
}
/// Rows with `read` as seen by the user bound to `?1` (read state is per user, in `notification_reads`).
pub const SEL: &str = "SELECT n.id,n.time,n.kind,n.severity,n.title,n.body,n.server_id,EXISTS(SELECT 1 FROM notification_reads r WHERE r.user_id=?1 AND r.notification_id=n.id) FROM notifications n";
/// Filter for `SEL`: only rows the user bound to `?1` has not read.
pub const UNREAD: &str = "WHERE NOT EXISTS(SELECT 1 FROM notification_reads r WHERE r.user_id=?1 AND r.notification_id=n.id)";

/// Insert unless the same kind + server was already recorded within the de-dup window. True when inserted.
fn insert_if_new(c: &Connection, n: &Value, now: chrono::DateTime<chrono::Utc>) -> rusqlite::Result<bool> {
    let cutoff = (now - chrono::Duration::minutes(rules::DEDUPE_WINDOW_MIN)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let (kind, sid) = (n["kind"].as_str(), n["server_id"].as_str());
    // Player joins are de-duplicated per player (the body names them), so two players joining together both notify.
    let dup: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM notifications WHERE kind=?1 AND server_id IS ?2 AND time>=?3 AND (kind<>'player_join' OR body=?4))", params![kind, sid, cutoff, n["body"].as_str()], |r| r.get(0))?;
    if dup { return Ok(false); }
    c.execute("INSERT INTO notifications(id,time,kind,severity,title,body,server_id) VALUES (?,?,?,?,?,?,?)",
        params![n["id"].as_str(), n["time"].as_str(), kind, n["severity"].as_str(), n["title"].as_str(), n["body"].as_str(), sid])?;
    Ok(true)
}

/// Record a notification and fan it out. Never fails the caller; duplicates are dropped silently.
pub async fn send(st: &St, kind: &str, severity: &str, title: &str, body: &str, server_id: Option<&str>) {
    if !rules::KINDS.contains(&kind) { tracing::warn!("unknown notification kind {kind}"); return; }
    let severity = if rules::SEVERITIES.contains(&severity) { severity } else { "info" };
    let n = json!({"id": uuid::Uuid::now_v7().to_string(), "time": crate::db::now(), "kind": kind, "severity": severity,
        "title": title.chars().take(200).collect::<String>(), "body": body.chars().take(1000).collect::<String>(), "server_id": server_id, "read": false});
    let n2 = n.clone();
    match st.db.call(move |c| insert_if_new(c, &n2, chrono::Utc::now())).await {
        Ok(true) => {}
        Ok(false) => return,
        Err(e) => { tracing::warn!("could not store notification: {e:#}"); return; }
    }
    st.emit("notification", n.clone());
    let enabled = rules::parse_events(st.db.call(|c| c.query_row("SELECT v FROM meta WHERE k='notify_events'", [], |r| r.get::<_, String>(0)).map(Some).or_else(|e| if matches!(e, rusqlite::Error::QueryReturnedNoRows) { Ok(None) } else { Err(e) })).await.ok().flatten().as_deref());
    if !enabled.iter().any(|k| k == kind) { return; }
    push::dispatch(st, enabled, kind, severity, title, body, server_id);
    let url = st.setting("notify_webhook_url", "").await;
    if url.is_empty() { return; }
    let st = st.clone();
    tokio::spawn(async move {
        let server = match n["server_id"].as_str() { Some(id) => crate::server::get(&st, id).await.ok().map(|s| s.name), None => None };
        let Ok(u) = reqwest::Url::parse(&url) else { return };
        let payload = webhook::build_payload(&u, n["kind"].as_str().unwrap_or(""), n["severity"].as_str().unwrap_or("info"), n["title"].as_str().unwrap_or(""), n["body"].as_str().unwrap_or(""), server.as_deref(), n["time"].as_str().unwrap_or(""));
        let (ok, detail) = webhook::deliver(&url, &payload, st.cfg.allow_private_webhooks, 3).await;
        if !ok { tracing::warn!("notification webhook delivery failed: {detail}"); }
    });
}

/// `POST /notifications/test`: one attempt, reported to the caller.
pub async fn send_test(st: &St) -> (bool, String) {
    let url = st.setting("notify_webhook_url", "").await;
    if url.is_empty() { return (false, "no webhook URL is configured".into()); }
    let Ok(u) = reqwest::Url::parse(&url) else { return (false, "the webhook URL is not valid".into()) };
    let payload = webhook::build_payload(&u, "test", "info", "Moss test notification", "If you can read this, webhook delivery works.", None, &crate::db::now());
    webhook::deliver(&url, &payload, st.cfg.allow_private_webhooks, 1).await
}

// ---- producers (all best effort) ----

/// A server's process died unexpectedly. `gave_up`: automatic restart is exhausted (crash loop).
pub async fn server_crashed(st: &St, s: &Srv, detail: &str, gave_up: bool) {
    if gave_up { send(st, "crash_loop", "critical", &format!("{} keeps crashing", s.name), detail, Some(&s.id)).await; }
    else { send(st, "crash", "warn", &format!("{} crashed", s.name), detail, Some(&s.id)).await; }
}
pub async fn restart_failed(st: &St, s: &Srv, why: &str) {
    send(st, "restart_failed", "critical", &format!("{} could not be restarted", s.name), why, Some(&s.id)).await;
}
pub async fn backup_failed(st: &St, s: &Srv, why: &str) {
    send(st, "backup_failed", "warn", &format!("Backup of {} failed", s.name), why, Some(&s.id)).await;
}
pub async fn remote_backup_failed(st: &St, s: &Srv, destination: &str, why: &str) {
    send(st, "backup_failed", "warn", &format!("Remote copy of {} failed", s.name), &format!("Could not copy the backup to {destination}: {why}"), Some(&s.id)).await;
}
/// A player joined a server (opt-in kind; de-duplicated per player and server by `send`).
pub async fn player_joined(st: &St, server_id: &str, player: &str) {
    let name = match crate::server::get(st, server_id).await { Ok(s) => s.name, Err(_) => return };
    send(st, "player_join", "info", &format!("{player} joined {name}"), &format!("{player} joined {name}."), Some(server_id)).await;
}
pub async fn operation_failed(st: &St, title: &str, why: &str, server_id: Option<&str>) {
    send(st, "operation_failed", "warn", &format!("{title} failed"), why, server_id).await;
}

/// Every five minutes: warn once when the data volume runs low (hysteresis in `rules::DiskAlert`).
pub fn spawn_background(st: &St) {
    let st = st.clone();
    tokio::spawn(async move {
        let mut alert = rules::DiskAlert::default();
        loop {
            let (total, free) = platform::disk(&st.cfg.data_dir);
            if alert.step(free, total) {
                let body = format!("{:.1} GiB free of {:.1} GiB on the data volume. Backups and world saves may fail.", free as f64 / (1u64 << 30) as f64, total as f64 / (1u64 << 30) as f64);
                send(&st, "disk_low", "critical", "Disk space is running low", &body, None).await;
            }
            tokio::time::sleep(Duration::from_secs(300)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(kind: &str, sid: Option<&str>) -> Value {
        json!({"id": uuid::Uuid::now_v7().to_string(), "time": crate::db::now(), "kind": kind, "severity": "warn", "title": "t", "body": "b", "server_id": sid})
    }

    fn db() -> (crate::db::Db, std::path::PathBuf) {
        let p = std::env::temp_dir().join(format!("moss-notify-{}.sqlite", uuid::Uuid::new_v4()));
        (crate::db::Db::open(&p).unwrap(), p)
    }

    #[tokio::test]
    async fn same_kind_and_server_is_deduplicated_within_the_window() {
        let (d, p) = db();
        let now = chrono::Utc::now();
        let r = d.call(move |c| {
            let a = insert_if_new(c, &note("crash", Some("s1")), now)?;
            let dup = insert_if_new(c, &note("crash", Some("s1")), now)?;
            let other_server = insert_if_new(c, &note("crash", Some("s2")), now)?;
            let other_kind = insert_if_new(c, &note("backup_failed", Some("s1")), now)?;
            let global_a = insert_if_new(c, &note("disk_low", None), now)?;
            let global_b = insert_if_new(c, &note("disk_low", None), now)?;
            // eleven minutes later the same notification is allowed again
            let later = insert_if_new(c, &note("crash", Some("s1")), now + chrono::Duration::minutes(11))?;
            Ok((a, dup, other_server, other_kind, global_a, global_b, later))
        }).await.unwrap();
        assert_eq!(r, (true, false, true, true, true, false, true));
        let _ = std::fs::remove_file(p);
    }

    #[tokio::test]
    async fn player_join_is_deduplicated_per_player_and_server() {
        let (d, p) = db();
        let now = chrono::Utc::now();
        let join = |player: &str, sid: &str| json!({"id": uuid::Uuid::now_v7().to_string(), "time": crate::db::now(), "kind": "player_join", "severity": "info", "title": "t", "body": format!("{player} joined."), "server_id": sid});
        let r = d.call(move |c| {
            let alex = insert_if_new(c, &join("Alex", "s1"), now)?;
            let alex_again = insert_if_new(c, &join("Alex", "s1"), now)?;
            let steve = insert_if_new(c, &join("Steve", "s1"), now)?;
            let alex_elsewhere = insert_if_new(c, &join("Alex", "s2"), now)?;
            let alex_later = insert_if_new(c, &join("Alex", "s1"), now + chrono::Duration::minutes(11))?;
            Ok((alex, alex_again, steve, alex_elsewhere, alex_later))
        }).await.unwrap();
        assert_eq!(r, (true, false, true, true, true));
        assert!(rules::KINDS.contains(&"player_join") && !rules::DEFAULT_EVENTS.contains(&"player_join") && !rules::is_admin_only("player_join"));
        let _ = std::fs::remove_file(p);
    }
}
