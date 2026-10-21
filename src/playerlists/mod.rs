//! Shared player lists: named allowlists and ban lists managed once and applied to every subscribed server.
//! Applying is idempotent: the desired names (union of the subscribed lists) are compared with the names Moss recorded
//! as applied (`player_list_applied`) and only the difference is acted on, so rows that change while an apply runs are
//! picked up by the next apply (every change triggers one, and applies for one server never overlap).
//!  - Minecraft Java: stopped, whitelist.json / banned-players.json are merged (hand-added entries survive); running,
//!    `whitelist add|remove`, `ban`, `pardon` go through the console.
//!  - Bedrock: allowlist.json or `allowlist add|remove`; bans do not exist.
//!  - Games with an admin channel: bans only, through `games::admin::act` (no unban command exists there).
//!  - Velocity proxies have no whitelist; subscribe the backend servers instead.
use crate::{games::admin, lifecycle, server::{self, Srv}, state::St};
use files::{Item, key};
use rusqlite::{Connection, params};
use std::{collections::{HashMap, HashSet}, sync::{Arc, Mutex, OnceLock}};

pub mod files;

pub const MOJANG: &str = "https://api.mojang.com";

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Support { pub allow: bool, pub ban: bool }

pub fn support(s: &Srv) -> Support {
    if s.is_proxy() { return Support { allow: false, ban: false }; }
    if s.is_mc() { return Support { allow: true, ban: true }; }
    if s.runtime == "bedrock" { return Support { allow: true, ban: false }; }
    Support { allow: false, ban: s.game().is_some_and(|g| admin::flavor(g).is_some() || g.id == "terraria") }
}

fn supports(s: &Srv, kind: &str) -> bool { let u = support(s); if kind == "allow" { u.allow } else { u.ban } }

/// Names that are safe to hand to a console command (and to store): the admin channel's rules.
pub fn valid_name(n: &str) -> bool { admin::safe_text(n, 64) && n == n.trim() }

pub fn running(st: &St, id: &str) -> bool { st.live.lock().unwrap().get(id).is_some_and(|l| l.alive) }

/// The union of the entries of every list of `kind` the server subscribes to (first entry per name wins, a known UUID is kept).
pub fn desired(c: &Connection, server_id: &str, kind: &str) -> rusqlite::Result<Vec<Item>> {
    let mut q = c.prepare("SELECT e.name, e.uuid, e.reason FROM player_list_entries e JOIN player_lists l ON l.id=e.list_id JOIN player_list_subs s ON s.list_id=e.list_id
        WHERE s.server_id=? AND l.kind=? ORDER BY e.added_at, e.name")?;
    let rows = q.query_map(params![server_id, kind], |r| Ok(Item { name: r.get(0)?, uuid: r.get(1)?, reason: r.get(2)? }))?;
    let mut out: Vec<Item> = vec![];
    let mut at: HashMap<String, usize> = HashMap::new();
    for it in rows {
        let it = it?;
        match at.get(&key(&it.name)) {
            Some(&i) => if out[i].uuid.is_none() { out[i].uuid = it.uuid; },
            None => { at.insert(key(&it.name), out.len()); out.push(it); }
        }
    }
    Ok(out)
}

/// Applied names that were on the server before any list asked for them (Moss forgets them instead of removing them).
pub fn hand_keys(c: &Connection, server_id: &str, kind: &str) -> rusqlite::Result<HashSet<String>> {
    c.prepare("SELECT key FROM player_list_applied WHERE server_id=? AND kind=? AND owned=0")?.query_map(params![server_id, kind], |r| r.get(0))?.collect()
}

pub fn applied(c: &Connection, server_id: &str, kind: &str) -> rusqlite::Result<Vec<String>> {
    c.prepare("SELECT name FROM player_list_applied WHERE server_id=? AND kind=? ORDER BY key")?.query_map(params![server_id, kind], |r| r.get(0))?.collect()
}

/// How many desired names are not on the server yet, per kind (what the UI shows as pending).
pub fn pending(c: &Connection, s: &Srv) -> rusqlite::Result<(usize, usize)> {
    let one = |kind: &str| -> rusqlite::Result<usize> {
        if !supports(s, kind) { return Ok(0); }
        Ok(files::diff(&desired(c, &s.id, kind)?, &applied(c, &s.id, kind)?).add.len())
    };
    Ok((one("allow")?, one("ban")?))
}

fn lock_for(id: &str) -> Arc<tokio::sync::Mutex<()>> {
    static L: OnceLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    L.get_or_init(Default::default).lock().unwrap().entry(id.to_string()).or_default().clone()
}

fn note(st: &St, id: &str, msg: String) { crate::console::push(&crate::console::hub(st, id), format!("\x1b[33m[moss] player lists: {msg}\x1b[0m")); }

fn read_prop(s: &Srv, st: &St, k: &str) -> Option<String> {
    let raw = std::fs::read_to_string(s.dir(st).join("server.properties")).ok()?;
    raw.lines().filter(|l| !l.trim_start().starts_with('#')).filter_map(|l| l.split_once('=')).find(|(a, _)| a.trim() == k).map(|(_, v)| v.trim().to_string())
}

/// `white-list` (Java) or `allow-list` (Bedrock) in server.properties; None when unknown.
pub fn allow_enabled(st: &St, s: &Srv) -> Option<bool> {
    if s.runtime == "bedrock" { return read_prop(s, st, "allow-list").map(|v| v == "true"); }
    if s.is_mc() && !s.is_proxy() { return Some(read_prop(s, st, "white-list").is_some_and(|v| v == "true")); }
    None
}

/// Apply in the background (never blocks the caller, errors are logged).
pub fn apply_bg(st: &St, id: &str) {
    let (st, id) = (st.clone(), id.to_string());
    tokio::spawn(async move { apply(&st, &id).await; });
}

/// Apply in the background to each of `servers`, one after the other.
pub fn apply_many_bg(st: &St, servers: Vec<String>) {
    let st = st.clone();
    tokio::spawn(async move { for id in servers { apply(&st, &id).await; } });
}

/// Admin-channel games: when a player with a pending list ban is seen online, apply again so the ban lands now.
pub fn players_seen(st: &St, id: &str, names: &[String]) {
    static LAST: OnceLock<Mutex<HashMap<String, std::time::Instant>>> = OnceLock::new();
    if names.is_empty() { return; }
    {
        // at most one check a minute per server, so a ban that keeps failing is not retried on every poll
        let mut last = LAST.get_or_init(Default::default).lock().unwrap();
        if last.get(id).is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(60)) { return; }
        last.insert(id.to_string(), std::time::Instant::now());
    }
    let (st, id, seen) = (st.clone(), id.to_string(), names.iter().map(|n| key(n)).collect::<HashSet<String>>());
    tokio::spawn(async move {
        let sid = id.clone();
        let hit = st.db.call(move |c| Ok(files::diff(&desired(c, &sid, "ban")?, &applied(c, &sid, "ban")?).add.iter().any(|i| seen.contains(&key(&i.name))))).await;
        if hit.unwrap_or(false) { apply(&st, &id).await; }
    });
}

pub async fn apply(st: &St, id: &str) { apply_with(st, id, MOJANG, std::time::Duration::from_secs(15)).await }

/// Before a start: Mojang lookups get at most 3 s in total; names still unresolved are added once the server is ready.
pub async fn apply_before_start(st: &St, id: &str) { apply_with(st, id, MOJANG, std::time::Duration::from_secs(3)).await }

/// `budget` bounds the Mojang lookups of the whole apply (both kinds).
pub async fn apply_with(st: &St, id: &str, mojang: &str, budget: std::time::Duration) {
    let lock = lock_for(id);
    let _g = lock.lock().await;
    let deadline = std::time::Instant::now() + budget;
    let Ok(s) = server::get(st, id).await else { return };
    for kind in ["allow", "ban"] {
        if !supports(&s, kind) { continue; }
        if let Err(e) = apply_kind(st, &s, kind, mojang, deadline).await { tracing::warn!("player lists on {}: {e}", s.name); note(st, id, e); }
    }
}

async fn apply_kind(st: &St, s: &Srv, kind: &'static str, mojang: &str, deadline: std::time::Instant) -> Result<(), String> {
    let (sid, k) = (s.id.clone(), kind);
    let (want, have, hand) = st.db.call(move |c| Ok((desired(c, &sid, k)?, applied(c, &sid, k)?, hand_keys(c, &sid, k)?))).await.map_err(|e| e.to_string())?;
    if want.is_empty() && have.is_empty() { return Ok(()); }
    let live = running(st, &s.id);
    if s.game().is_some_and(|_| s.runtime != "bedrock") {
        // admin-channel games: bans only, and only while the server runs
        if !live { return Ok(()); }
        let d = files::diff(&want, &have);
        for it in &d.add {
            let reason = (!it.reason.is_empty()).then_some(it.reason.as_str());
            match admin::act(st, s, &it.name, reason, true).await {
                Ok(()) => mark(st, &s.id, kind, &it.name, true).await,
                Err(e) if e.status == axum::http::StatusCode::NOT_FOUND || e.status == axum::http::StatusCode::BAD_REQUEST => {} // not connected (Source/Palworld need ids) or unusable name: stays pending
                Err(e) => return Err(e.message),
            }
        }
        for n in &d.remove { mark(st, &s.id, kind, n, false).await; } // there is no unban command to send; forget it
        return Ok(());
    }
    if live { apply_running(st, s, kind, &want, &have, &hand).await } else { apply_files(st, s, kind, want, have, hand, mojang, deadline).await }
}

/// Records (`on`) or forgets a name on the server; `mark_hand` records one that was already there by hand.
async fn mark(st: &St, sid: &str, kind: &str, name: &str, on: bool) { mark_owned(st, sid, kind, name, on, true).await }
async fn mark_hand(st: &St, sid: &str, kind: &str, name: &str) { mark_owned(st, sid, kind, name, true, false).await }

async fn mark_owned(st: &St, sid: &str, kind: &str, name: &str, on: bool, owned: bool) {
    let (sid, kind, name) = (sid.to_string(), kind.to_string(), name.to_string());
    let _ = st.db.call(move |c| {
        if on { c.execute("INSERT OR REPLACE INTO player_list_applied(server_id,kind,key,name,owned) VALUES (?,?,?,?,?)", params![sid, kind, key(&name), name, owned]) }
        else { c.execute("DELETE FROM player_list_applied WHERE server_id=? AND kind=? AND key=?", params![sid, kind, key(&name)]) }
    }).await;
}

fn cmd_name_ok(s: &Srv, n: &str) -> bool { s.runtime == "bedrock" || !n.contains(char::is_whitespace) }

fn list_file(bedrock: bool, kind: &str) -> &'static str { match (bedrock, kind) { (true, _) => "allowlist.json", (_, "allow") => "whitelist.json", _ => "banned-players.json" } }

/// Running server: send the difference through the console. Names already in the server's list file by hand are
/// recorded as hand entries instead (no command), and removing one of those only forgets it.
async fn apply_running(st: &St, s: &Srv, kind: &str, want: &[Item], have: &[String], hand: &HashSet<String>) -> Result<(), String> {
    let d = files::diff(want, have);
    let bedrock = s.runtime == "bedrock";
    let on_disk = files::names_in(&tokio::fs::read_to_string(s.dir(st).join(list_file(bedrock, kind))).await.unwrap_or_default());
    let q = |n: &str| if bedrock { format!("\"{n}\"") } else { n.to_string() };
    let mut changed = false;
    let send = |c: String| async move { lifecycle::send_command(st, s, &c).await.map_err(|e| e.message) };
    for it in d.add.iter().filter(|i| cmd_name_ok(s, &i.name)) {
        if on_disk.contains_key(&key(&it.name)) { mark_hand(st, &s.id, kind, &it.name).await; continue; }
        let c = match (kind, bedrock) {
            ("allow", false) => format!("whitelist add {}", it.name),
            ("allow", true) => format!("allowlist add {}", q(&it.name)),
            _ if it.reason.is_empty() => format!("ban {}", it.name),
            _ => format!("ban {} {}", it.name, it.reason),
        };
        send(c).await?;
        mark(st, &s.id, kind, &it.name, true).await;
        changed = true;
    }
    for n in d.remove.iter() {
        if !cmd_name_ok(s, n) || hand.contains(&key(n)) { mark(st, &s.id, kind, n, false).await; continue; }
        send(match (kind, bedrock) { ("allow", false) => format!("whitelist remove {n}"), ("allow", true) => format!("allowlist remove {}", q(n)), _ => format!("pardon {n}") }).await?;
        mark(st, &s.id, kind, n, false).await;
        changed = true;
    }
    if changed && kind == "allow" { send(if bedrock { "allowlist reload" } else { "whitelist reload" }.to_string()).await?; }
    Ok(())
}

/// Stopped server: merge the JSON file Minecraft or Bedrock reads at start. A desired name the file already holds and
/// Moss did not write is left as it is and recorded as a hand entry, so removing it from a list never drops it.
#[allow(clippy::too_many_arguments)]
async fn apply_files(st: &St, s: &Srv, kind: &str, want: Vec<Item>, have: Vec<String>, hand: HashSet<String>, mojang: &str, deadline: std::time::Instant) -> Result<(), String> {
    let bedrock = s.runtime == "bedrock";
    let online = read_prop(s, st, "online-mode").is_none_or(|v| v != "false");
    let file = list_file(bedrock, kind);
    let path = s.dir(st).join(file);
    let existing = tokio::fs::read_to_string(&path).await.unwrap_or_default();
    let on_disk = files::names_in(&existing);
    let owned: HashSet<String> = have.iter().map(|n| key(n)).filter(|k| !hand.contains(k)).collect();
    let (mut items, mut adopted): (Vec<Item>, Vec<String>) = (vec![], vec![]);
    for mut it in want {
        let k = key(&it.name);
        if on_disk.contains_key(&k) && !owned.contains(&k) { adopted.push(it.name); continue; }
        if !bedrock {
            if !online { it.uuid = Some(files::offline_uuid(&it.name)); }
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if it.uuid.is_none() && !left.is_zero() {
                it.uuid = files::mojang_uuid(&st.http, mojang, &it.name, left.min(std::time::Duration::from_secs(5))).await;
                if let Some(u) = it.uuid.clone() {
                    let (n, u) = (it.name.clone(), u);
                    let _ = st.db.call(move |c| c.execute("UPDATE player_list_entries SET uuid=? WHERE lower(name)=lower(?) AND uuid IS NULL", params![u, n])).await;
                }
            }
            // the UUID Moss wrote earlier keeps a name in place when Mojang cannot be asked now
            if it.uuid.is_none() && owned.contains(&k) { it.uuid = on_disk.get(&k).cloned().flatten(); }
            if it.uuid.is_none() { continue; } // unknown to Mojang or no answer: stays pending
        }
        items.push(it);
    }
    let entry: fn(&Item) -> serde_json::Value = if bedrock { files::bedrock_entry } else if kind == "allow" { files::whitelist_entry } else { files::ban_entry };
    let out = files::merge(&existing, &items, &owned, entry).map_err(|e| format!("{file} left alone ({e})"))?;
    if out != existing {
        let tmp = path.with_extension("json.moss-tmp");
        tokio::fs::write(&tmp, &out).await.map_err(|e| e.to_string())?;
        tokio::fs::rename(&tmp, &path).await.map_err(|e| e.to_string())?;
    }
    let (sid, kind, names): (String, String, Vec<String>) = (s.id.clone(), kind.to_string(), items.iter().map(|i| i.name.clone()).collect());
    st.db.call(move |c| {
        let keep: HashSet<String> = names.iter().chain(&adopted).map(|n| key(n)).collect();
        for n in applied(c, &sid, &kind)? { if !keep.contains(&key(&n)) { c.execute("DELETE FROM player_list_applied WHERE server_id=? AND kind=? AND key=?", params![sid, kind, key(&n)])?; } }
        for n in &names { c.execute("INSERT OR REPLACE INTO player_list_applied(server_id,kind,key,name,owned) VALUES (?,?,?,?,1)", params![sid, kind, key(n), n])?; }
        for n in &adopted { c.execute("INSERT OR REPLACE INTO player_list_applied(server_id,kind,key,name,owned) VALUES (?,?,?,?,0)", params![sid, kind, key(n), n])?; }
        Ok(())
    }).await.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{Reply, state, stub};

    async fn setup(st: &St, props: &str) -> (String, String, std::path::PathBuf) {
        let sid = uuid::Uuid::now_v7().to_string();
        let (s2, lid, lid2) = (sid.clone(), "l1".to_string(), "l2".to_string());
        st.db.call(move |c| {
            c.execute("INSERT INTO servers(id,name,runtime,game_version,port,memory_mb,created_at,installed) VALUES (?, 'Survival','paper','1.21.1',25565,1024,'2026-01-01T00:00:00Z',1)", [&s2])?;
            c.execute("INSERT INTO player_lists(id,name,kind,created_at) VALUES (?,?,?,'t')", [&lid, "Friends", "allow"])?;
            c.execute("INSERT INTO player_lists(id,name,kind,created_at) VALUES (?,?,?,'t')", [&lid2, "Griefers", "ban"])?;
            for (l, n, r) in [("l1", "Alice", ""), ("l1", "Bob", ""), ("l2", "Grief", "griefing")] {
                c.execute("INSERT INTO player_list_entries(list_id,name,reason,added_at) VALUES (?,?,?,'2026-01-01')", [l, n, r])?;
            }
            for l in ["l1", "l2"] { c.execute("INSERT INTO player_list_subs(list_id,server_id) VALUES (?,?)", [l, &s2])?; }
            Ok(())
        }).await.unwrap();
        let dir = st.cfg.servers_dir().join(&sid);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("server.properties"), props).unwrap();
        (sid, "l1".into(), dir)
    }

    #[tokio::test]
    async fn an_offline_mode_server_gets_merged_files_and_applies_are_idempotent() {
        let st = state(|_| {}).await;
        let (sid, _, dir) = setup(&st, "online-mode=false\n").await;
        std::fs::write(dir.join("whitelist.json"), r#"[{"uuid":"hand-uuid","name":"Hand"}]"#).unwrap();
        apply(&st, &sid).await;
        let wl: Vec<serde_json::Value> = serde_json::from_str(&std::fs::read_to_string(dir.join("whitelist.json")).unwrap()).unwrap();
        assert_eq!(wl.iter().map(|e| e["name"].as_str().unwrap()).collect::<Vec<_>>(), ["Hand", "Alice", "Bob"]);
        assert_eq!(wl[1]["uuid"], files::offline_uuid("Alice"));
        let bans: Vec<serde_json::Value> = serde_json::from_str(&std::fs::read_to_string(dir.join("banned-players.json")).unwrap()).unwrap();
        assert_eq!((bans[0]["name"].as_str(), bans[0]["reason"].as_str()), (Some("Grief"), Some("griefing")));
        let before = std::fs::read_to_string(dir.join("whitelist.json")).unwrap();
        apply(&st, &sid).await;
        assert_eq!(std::fs::read_to_string(dir.join("whitelist.json")).unwrap(), before);
        // unsubscribe from the allow list: Moss's names go, the hand-added one stays
        let s2 = sid.clone();
        st.db.call(move |c| c.execute("DELETE FROM player_list_subs WHERE list_id='l1' AND server_id=?", [&s2])).await.unwrap();
        apply(&st, &sid).await;
        let wl: Vec<serde_json::Value> = serde_json::from_str(&std::fs::read_to_string(dir.join("whitelist.json")).unwrap()).unwrap();
        assert_eq!(wl.iter().map(|e| e["name"].as_str().unwrap()).collect::<Vec<_>>(), ["Hand"]);
        let s3 = sid.clone();
        let (p, a) = st.db.call(move |c| Ok((pending(c, &server::get_sync(c, &s3)?.unwrap())?, applied(c, &s3, "allow")?))).await.unwrap();
        assert_eq!((p, a.len()), ((0, 0), 0));
    }

    #[tokio::test]
    async fn an_online_mode_server_resolves_uuids_with_mojang_and_caches_them() {
        let st = state(|_| {}).await;
        let (sid, _, dir) = setup(&st, "online-mode=true\n").await;
        let mj = stub(|_, p| if p.ends_with("/Alice") { Reply::json(200, serde_json::json!({"id": "069a79f444e94726a5befca90e38aaf5", "name": "Alice"})) } else { Reply::status(404) }).await;
        apply_with(&st, &sid, &mj.base, std::time::Duration::from_secs(15)).await;
        let wl: Vec<serde_json::Value> = serde_json::from_str(&std::fs::read_to_string(dir.join("whitelist.json")).unwrap()).unwrap();
        // Bob and the banned Grief are unknown to Mojang: not written, still pending
        assert_eq!(wl.len(), 1);
        assert_eq!(wl[0]["uuid"], "069a79f4-44e9-4726-a5be-fca90e38aaf5");
        let s2 = sid.clone();
        let (u, p) = st.db.call(move |c| Ok((c.query_row("SELECT uuid FROM player_list_entries WHERE name='Alice'", [], |r| r.get::<_, Option<String>>(0))?, pending(c, &server::get_sync(c, &s2)?.unwrap())?))).await.unwrap();
        assert_eq!((u.as_deref(), p), (Some("069a79f4-44e9-4726-a5be-fca90e38aaf5"), (1, 1)));
    }

    #[tokio::test]
    async fn an_unreadable_whitelist_file_is_left_alone() {
        let st = state(|_| {}).await;
        let (sid, _, dir) = setup(&st, "online-mode=false\n").await;
        std::fs::write(dir.join("whitelist.json"), "{not json").unwrap();
        apply(&st, &sid).await;
        assert_eq!(std::fs::read_to_string(dir.join("whitelist.json")).unwrap(), "{not json");
    }

    #[tokio::test]
    async fn a_hand_ban_is_adopted_and_survives_removal_from_the_list() {
        let st = state(|_| {}).await;
        let (sid, _, dir) = setup(&st, "online-mode=false\n").await;
        let hand = r#"[{"uuid":"hand-uuid","name":"grief","source":"Console","reason":"by hand"}]"#;
        std::fs::write(dir.join("banned-players.json"), hand).unwrap();
        apply(&st, &sid).await;
        assert_eq!(serde_json::from_str::<serde_json::Value>(&std::fs::read_to_string(dir.join("banned-players.json")).unwrap()).unwrap(), serde_json::from_str::<serde_json::Value>(hand).unwrap()); // kept as it is
        let s2 = sid.clone();
        let p = st.db.call(move |c| pending(c, &server::get_sync(c, &s2)?.unwrap())).await.unwrap();
        assert_eq!(p.1, 0);
        st.db.call(|c| c.execute("DELETE FROM player_list_entries WHERE name='Grief'", [])).await.unwrap();
        apply(&st, &sid).await;
        assert_eq!(serde_json::from_str::<serde_json::Value>(&std::fs::read_to_string(dir.join("banned-players.json")).unwrap()).unwrap(), serde_json::from_str::<serde_json::Value>(hand).unwrap());
        let s3 = sid.clone();
        assert!(st.db.call(move |c| applied(c, &s3, "ban")).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn names_moss_wrote_stay_when_mojang_is_down() {
        let st = state(|_| {}).await;
        let (sid, _, dir) = setup(&st, "online-mode=true\n").await;
        let up = stub(|_, p| Reply::json(200, serde_json::json!({"id": if p.ends_with("/Alice") { "069a79f444e94726a5befca90e38aaf5" } else { "169a79f444e94726a5befca90e38aaf5" }}))).await;
        apply_with(&st, &sid, &up.base, std::time::Duration::from_secs(15)).await;
        st.db.call(|c| c.execute("UPDATE player_list_entries SET uuid=NULL", [])).await.unwrap(); // e.g. after an online-mode switch
        let before = std::fs::read_to_string(dir.join("whitelist.json")).unwrap();
        let down = stub(|_, _| Reply::status(503)).await;
        apply_with(&st, &sid, &down.base, std::time::Duration::from_secs(15)).await;
        assert_eq!(std::fs::read_to_string(dir.join("whitelist.json")).unwrap(), before);
        // a zero budget makes no lookups at all
        apply_with(&st, &sid, "http://127.0.0.1:9", std::time::Duration::ZERO).await;
        assert_eq!(std::fs::read_to_string(dir.join("whitelist.json")).unwrap(), before);
    }

    #[tokio::test]
    async fn switching_to_online_mode_replaces_offline_uuids_with_mojang_ones() {
        let st = state(|_| {}).await;
        let (sid, _, dir) = setup(&st, "online-mode=false\n").await;
        apply(&st, &sid).await;
        std::fs::write(dir.join("server.properties"), "online-mode=true\n").unwrap();
        let mj = stub(|_, p| if p.ends_with("/Alice") { Reply::json(200, serde_json::json!({"id": "069a79f444e94726a5befca90e38aaf5"})) } else { Reply::status(503) }).await;
        apply_with(&st, &sid, &mj.base, std::time::Duration::from_secs(15)).await;
        let wl: Vec<serde_json::Value> = serde_json::from_str(&std::fs::read_to_string(dir.join("whitelist.json")).unwrap()).unwrap();
        let uuid = |n: &str| wl.iter().find(|e| e["name"] == n).map(|e| e["uuid"].as_str().unwrap().to_string());
        assert_eq!(uuid("Alice").as_deref(), Some("069a79f4-44e9-4726-a5be-fca90e38aaf5"));
        assert_eq!(uuid("Bob"), Some(files::offline_uuid("Bob"))); // Mojang failed: the earlier entry stays
    }
}
