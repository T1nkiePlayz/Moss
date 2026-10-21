//! Player analytics: who played when. A subscriber on the internal bus (like the webhook deriver) turns the "server"
//! view events into sessions (from player-name diffs) and hourly player counts, persisted in SQLite (`player_sessions`,
//! `player_counts`; both cascade with the server). `Recorder` is pure so the rules are testable. Limits, by design:
//! * Names come from the status ping (every 5 s). Minecraft lists at most 12 names; while more players are online than
//!   names listed, joins are recorded but leaves are not (they are closed once the list is complete again or the server stops).
//!   A server that hides its sample (players online, no names) records counts only.
//! * Counts are stored at most once per minute per server (peak = highest value seen in that minute).
//! * Sessions still open when the controller stops end at the last time they were seen.
//! * Rows older than 90 days are pruned daily.
use crate::state::St;
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::{collections::{HashMap, HashSet}, time::Duration};

pub const RETENTION_DAYS: i64 = 90;

#[derive(Debug, PartialEq)]
pub enum Act {
    Open { server: String, player: String, at: i64 },
    Close { server: String, player: String, at: i64 },
    CloseAll { server: String, at: i64 },
    Count { server: String, hour: i64, online: u32, peak: u32 },
    Touch { server: String, at: i64 },
}

#[derive(Default)]
struct Track { open: HashSet<String>, minute: i64, pending_max: u32 }

#[derive(Default)]
pub struct Recorder { servers: HashMap<String, Track> }

impl Recorder {
    /// `v` is a "server" view event, `now` is epoch seconds.
    pub fn on_server(&mut self, v: &Value, now: i64) -> Vec<Act> {
        let Some(id) = v["id"].as_str() else { return vec![] };
        let Some(state) = v["state"].as_str() else { self.servers.remove(id); return vec![] }; // deleted
        let mut out = vec![];
        if state != "running" {
            if let Some(t) = self.servers.remove(id) && !t.open.is_empty() { out.push(Act::CloseAll { server: id.into(), at: now }); }
            return out;
        }
        let Some(online) = v["players"]["online"].as_u64().map(|n| n.min(u32::MAX as u64) as u32) else { return out };
        let t = self.servers.entry(id.to_string()).or_default();
        t.pending_max = t.pending_max.max(online);
        let names: HashSet<String> = v["players"]["sample"].as_array().map(|a| a.iter().filter_map(|n| n.as_str()).filter(|n| !n.is_empty()).map(String::from).collect()).unwrap_or_default();
        if !(names.is_empty() && online > 0) {
            let mut joined: Vec<String> = names.iter().filter(|n| !t.open.contains(*n)).cloned().collect();
            joined.sort();
            for n in joined { t.open.insert(n.clone()); out.push(Act::Open { server: id.into(), player: n, at: now }); }
            if names.len() >= online as usize { // the list covers everyone online
                let mut left: Vec<String> = t.open.iter().filter(|n| !names.contains(*n)).cloned().collect();
                left.sort();
                for n in left { t.open.remove(&n); out.push(Act::Close { server: id.into(), player: n, at: now }); }
            }
        }
        if now / 60 != t.minute {
            t.minute = now / 60;
            out.push(Act::Count { server: id.into(), hour: now - now.rem_euclid(3600), online, peak: t.pending_max });
            t.pending_max = 0;
            if !t.open.is_empty() { out.push(Act::Touch { server: id.into(), at: now }); }
        }
        out
    }
}

pub fn apply(c: &Connection, acts: &[Act]) {
    for a in acts {
        let r = match a {
            Act::Open { server, player, at } => c.execute("INSERT INTO player_sessions(server_id,player,joined_at,last_seen) VALUES(?,?,?,?)", params![server, player, at, at]),
            Act::Close { server, player, at } => c.execute("UPDATE player_sessions SET left_at=MAX(?1,joined_at), last_seen=MAX(?1,joined_at) WHERE server_id=?2 AND player=?3 AND left_at IS NULL", params![at, server, player]),
            Act::CloseAll { server, at } => c.execute("UPDATE player_sessions SET left_at=MAX(?1,joined_at), last_seen=MAX(?1,joined_at) WHERE server_id=?2 AND left_at IS NULL", params![at, server]),
            Act::Count { server, hour, online, peak } => c.execute(
                "INSERT INTO player_counts(server_id,hour,peak,samples,sum) VALUES(?1,?2,?3,1,?4) ON CONFLICT(server_id,hour) DO UPDATE SET peak=MAX(peak,excluded.peak), samples=samples+1, sum=sum+excluded.sum",
                params![server, hour, peak.max(online), online]),
            Act::Touch { server, at } => c.execute("UPDATE player_sessions SET last_seen=? WHERE server_id=? AND left_at IS NULL", params![at, server]),
        };
        if let Err(e) = r { tracing::debug!("player analytics write skipped: {e}"); }
    }
}

/// Ends sessions left open by a previous controller run at the time they were last seen.
pub fn close_stale(c: &Connection) -> rusqlite::Result<usize> {
    c.execute("UPDATE player_sessions SET left_at=MAX(last_seen,joined_at) WHERE left_at IS NULL", [])
}

pub fn prune(c: &Connection, now: i64) -> rusqlite::Result<()> {
    let cutoff = now - RETENTION_DAYS * 86400;
    c.execute("DELETE FROM player_sessions WHERE left_at IS NOT NULL AND left_at < ?", [cutoff])?;
    c.execute("DELETE FROM player_counts WHERE hour < ?", [cutoff])?;
    Ok(())
}

fn iso(t: i64) -> String { chrono::DateTime::from_timestamp(t, 0).unwrap_or_default().to_rfc3339_opts(chrono::SecondsFormat::Secs, true) }

/// The analytics document for one server over the last `days` days; aggregation happens in SQL.
pub fn report(c: &Connection, id: &str, days: i64, now: i64) -> rusqlite::Result<Value> {
    let since = now - days * 86400;
    let first_hour = since - since.rem_euclid(3600);
    let hourly: Vec<Value> = c.prepare("SELECT hour, peak, CAST(sum AS REAL)/samples FROM player_counts WHERE server_id=? AND hour>=? ORDER BY hour")?
        .query_map(params![id, first_hour], |r| Ok(json!({"hour": iso(r.get(0)?), "peak": r.get::<_, i64>(1)?, "avg": (r.get::<_, f64>(2)? * 100.0).round() / 100.0})))?.collect::<Result<_, _>>()?;
    let (unique, sessions, total): (i64, i64, i64) = c.query_row(
        "SELECT COUNT(DISTINCT player), COUNT(*), COALESCE(SUM(COALESCE(left_at,?1) - MAX(joined_at,?2)),0) FROM player_sessions WHERE server_id=?3 AND COALESCE(left_at,?1) > ?2",
        params![now, since, id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    let peak = c.query_row("SELECT peak, hour FROM player_counts WHERE server_id=? AND hour>=? ORDER BY peak DESC, hour DESC LIMIT 1", params![id, first_hour],
        |r| Ok(json!({"count": r.get::<_, i64>(0)?, "at": iso(r.get(1)?)}))).or_else(|e| if matches!(e, rusqlite::Error::QueryReturnedNoRows) { Ok(json!({"count": 0, "at": null})) } else { Err(e) })?;
    let top: Vec<Value> = c.prepare(
        "SELECT player, SUM(COALESCE(left_at,?1) - MAX(joined_at,?2)), COUNT(*), MAX(COALESCE(left_at,?1)), MAX(left_at IS NULL) FROM player_sessions
         WHERE server_id=?3 AND COALESCE(left_at,?1) > ?2 GROUP BY player ORDER BY 2 DESC, player LIMIT 25")?
        .query_map(params![now, since, id], |r| Ok(json!({"name": r.get::<_, String>(0)?, "play_seconds": r.get::<_, i64>(1)?, "sessions": r.get::<_, i64>(2)?, "last_seen": iso(r.get(3)?), "online": r.get::<_, i64>(4)? != 0})))?.collect::<Result<_, _>>()?;
    let recent: Vec<Value> = c.prepare("SELECT player, joined_at, left_at FROM player_sessions WHERE server_id=?1 AND COALESCE(left_at,?2) > ?3 ORDER BY joined_at DESC, id DESC LIMIT 20")?
        .query_map(params![id, now, since], |r| { let (j, l): (i64, Option<i64>) = (r.get(1)?, r.get(2)?);
            Ok(json!({"name": r.get::<_, String>(0)?, "joined_at": iso(j), "left_at": l.map(iso), "seconds": l.unwrap_or(now) - j})) })?.collect::<Result<_, _>>()?;
    Ok(json!({"days": days, "hourly": hourly, "unique_players": unique, "sessions": sessions, "total_play_seconds": total,
        "avg_session_seconds": if sessions > 0 { total / sessions } else { 0 }, "peak": peak, "top_players": top, "recent_sessions": recent}))
}

pub async fn spawn_background(st: &St) {
    let now = chrono::Utc::now().timestamp();
    let _ = st.db.call(move |c| { close_stale(c)?; prune(c, now) }).await;
    let st1 = st.clone();
    tokio::spawn(async move {
        let mut rx = st1.bus.subscribe();
        let mut rec = Recorder::default();
        loop {
            let e = match rx.recv().await {
                Ok(e) => e,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            };
            if e.kind != "server" { continue; }
            let acts = rec.on_server(&e.data, chrono::Utc::now().timestamp());
            if acts.is_empty() { continue; }
            // staging runs are not servers (no row to attach the data to)
            if let Some(i) = e.data["id"].as_str() && crate::server::is_staging(&st1, i).await { continue; }
            let joins: Vec<(String, String)> = acts.iter().filter_map(|a| if let Act::Open { server, player, .. } = a { Some((server.clone(), player.clone())) } else { None }).collect();
            let _ = st1.db.call(move |c| { apply(c, &acts); Ok(()) }).await;
            for (server, player) in joins { crate::notify::player_joined(&st1, &server, &player).await; }
        }
    });
    let st2 = st.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(24 * 3600)).await;
            let now = chrono::Utc::now().timestamp();
            let _ = st2.db.call(move |c| prune(c, now)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sv(state: &str, online: u32, names: &[&str]) -> Value { json!({"id": "s1", "name": "S", "state": state, "players": {"online": online, "max": 20, "sample": names}}) }
    fn open(p: &str, at: i64) -> Act { Act::Open { server: "s1".into(), player: p.into(), at } }
    fn close(p: &str, at: i64) -> Act { Act::Close { server: "s1".into(), player: p.into(), at } }
    fn db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("PRAGMA foreign_keys=ON").unwrap();
        crate::db::migrate_for_tests(&c);
        c.execute("INSERT INTO servers(id,name,runtime,game_version,port,memory_mb,created_at) VALUES('s1','S','paper','1.21',25565,1024,'2026-01-01T00:00:00Z')", []).unwrap();
        c
    }
    fn sessions(acts: Vec<Act>) -> Vec<Act> { acts.into_iter().filter(|a| matches!(a, Act::Open { .. } | Act::Close { .. } | Act::CloseAll { .. })).collect() }
    fn counts(a: Vec<Act>) -> Vec<(i64, u32, u32)> { a.into_iter().filter_map(|a| if let Act::Count { hour, online, peak, .. } = a { Some((hour, online, peak)) } else { None }).collect() }

    #[test]
    fn sessions_open_and_close_from_name_diffs() {
        let mut r = Recorder::default();
        assert_eq!(sessions(r.on_server(&sv("running", 1, &["Alex"]), 1000)), [open("Alex", 1000)]);
        assert_eq!(sessions(r.on_server(&sv("running", 2, &["Alex", "Steve"]), 1005)), [open("Steve", 1005)]);
        assert_eq!(sessions(r.on_server(&sv("running", 1, &["Steve"]), 1010)), [close("Alex", 1010)]);
        assert_eq!(sessions(r.on_server(&sv("stopping", 1, &["Steve"]), 1015)), [Act::CloseAll { server: "s1".into(), at: 1015 }]);
        assert!(sessions(r.on_server(&sv("stopped", 0, &[]), 1020)).is_empty());
    }

    #[test]
    fn truncated_or_hidden_name_lists_do_not_invent_leaves() {
        let mut r = Recorder::default();
        r.on_server(&sv("running", 1, &["A"]), 0);
        // 3 online but only B listed: B joins, A is not closed because the list is incomplete
        assert_eq!(sessions(r.on_server(&sv("running", 3, &["B"]), 5)), [open("B", 5)]);
        // sample hidden: nothing changes
        assert!(sessions(r.on_server(&sv("running", 3, &[]), 10)).is_empty());
        // complete again: A and B gone
        assert_eq!(sessions(r.on_server(&sv("running", 0, &[]), 15)), [close("A", 15), close("B", 15)]);
    }

    #[test]
    fn counts_are_throttled_to_once_a_minute_and_keep_the_peak() {
        let mut r = Recorder::default();
        assert_eq!(counts(r.on_server(&sv("running", 2, &["A", "B"]), 7200 + 61)), [(7200, 2, 2)]);
        assert!(counts(r.on_server(&sv("running", 5, &[]), 7200 + 70)).is_empty());
        assert!(counts(r.on_server(&sv("running", 1, &[]), 7200 + 100)).is_empty());
        assert_eq!(counts(r.on_server(&sv("running", 1, &[]), 7200 + 121)), [(7200, 1, 5)]);
        // a new hour lands in a new bucket
        assert_eq!(counts(r.on_server(&sv("running", 0, &[]), 10800 + 5)), [(10800, 0, 0)]);
    }

    #[test]
    fn writes_buckets_sessions_and_cleans_up() {
        let c = db();
        let mut r = Recorder::default();
        for (t, n, names) in [(3600 + 60, 2u32, vec!["A", "B"]), (3600 + 130, 4, vec!["A", "B"]), (3600 + 200, 1, vec!["B"])] {
            apply(&c, &r.on_server(&sv("running", n, &names), t));
        }
        // minute 1 stores 2; minute 2 stores the first event (4); the third (minute 3) stores 1 with the pending peak
        let (peak, samples, sum): (i64, i64, i64) = c.query_row("SELECT peak,samples,sum FROM player_counts WHERE hour=3600", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
        assert_eq!((peak, samples, sum), (4, 3, 7));
        let open: i64 = c.query_row("SELECT COUNT(*) FROM player_sessions WHERE left_at IS NULL", [], |r| r.get(0)).unwrap();
        assert_eq!(open, 1);
        // a crash leaves B open: it ends at its last_seen
        assert_eq!(close_stale(&c).unwrap(), 1);
        let left: i64 = c.query_row("SELECT left_at FROM player_sessions WHERE player='B'", [], |r| r.get(0)).unwrap();
        assert_eq!(left, 3600 + 200);
        // the data goes with the server
        c.execute("DELETE FROM servers WHERE id='s1'", []).unwrap();
        assert_eq!(c.query_row::<i64, _, _>("SELECT (SELECT COUNT(*) FROM player_sessions)+(SELECT COUNT(*) FROM player_counts)", [], |r| r.get(0)).unwrap(), 0);
    }

    #[test]
    fn prune_drops_only_rows_past_retention() {
        let c = db();
        let now = 200 * 86400;
        let old = now - 91 * 86400;
        for (p, j, l) in [("Old", old, Some(old + 10)), ("New", now - 100, Some(now - 50)), ("Open", old, None)] {
            c.execute("INSERT INTO player_sessions(server_id,player,joined_at,left_at,last_seen) VALUES('s1',?,?,?,?)", params![p, j, l, j]).unwrap();
        }
        c.execute("INSERT INTO player_counts VALUES('s1',?,1,1,1),('s1',?,1,1,1)", params![old, now - 3600]).unwrap();
        prune(&c, now).unwrap();
        let names: Vec<String> = c.prepare("SELECT player FROM player_sessions ORDER BY player").unwrap().query_map([], |r| r.get(0)).unwrap().collect::<Result<_, _>>().unwrap();
        assert_eq!(names, ["New", "Open"]);
        assert_eq!(c.query_row::<i64, _, _>("SELECT COUNT(*) FROM player_counts", [], |r| r.get(0)).unwrap(), 1);
    }

    #[test]
    fn report_aggregates_in_sql() {
        let c = db();
        let now = 10 * 86400;
        for (p, j, l) in [("Alex", now - 3 * 86400, Some(now - 3 * 86400 + 3600)), ("Alex", now - 7200, Some(now - 5400)), ("Steve", now - 7200, Some(now - 3600)), ("Live", now - 600, None)] {
            c.execute("INSERT INTO player_sessions(server_id,player,joined_at,left_at,last_seen) VALUES('s1',?,?,?,0)", params![p, j, l]).unwrap();
        }
        c.execute("INSERT INTO player_counts VALUES('s1',?,3,4,8),('s1',?,5,2,3)", params![now - 7200, now - 3600]).unwrap();
        let r = report(&c, "s1", 7, now).unwrap();
        let total = 3600 + 1800 + 3600 + 600;
        assert_eq!((r["unique_players"].clone(), r["sessions"].clone(), r["total_play_seconds"].clone()), (json!(3), json!(4), json!(total)));
        assert_eq!(r["avg_session_seconds"], total / 4);
        assert_eq!(r["peak"]["count"], 5);
        assert_eq!(r["hourly"].as_array().unwrap().len(), 2);
        assert_eq!(r["hourly"][0]["avg"], 2.0);
        assert_eq!(r["top_players"][0]["name"], "Alex");
        assert_eq!(r["top_players"][0]["play_seconds"], 5400);
        assert_eq!(r["top_players"][0]["sessions"], 2);
        let live = r["top_players"].as_array().unwrap().iter().find(|p| p["name"] == "Live").unwrap();
        assert_eq!(live["online"], true);
        assert_eq!(r["recent_sessions"][0]["name"], "Live");
        assert_eq!(r["recent_sessions"][0]["left_at"], Value::Null);
        // a 1-day window drops the old Alex session
        let r1 = report(&c, "s1", 1, now).unwrap();
        assert_eq!(r1["sessions"], 3);
        assert_eq!(r1["unique_players"], 3);
        assert_eq!((r1["top_players"][0]["name"].clone(), r1["top_players"][0]["play_seconds"].clone()), (json!("Steve"), json!(3600)));
    }
}
