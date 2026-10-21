//! Remote admin for game servers: a live player list plus kick and ban. Source, Rust, Factorio, Palworld and Zomboid
//! are driven over RCON (`rcon`); Satisfactory's HTTPS API only yields a player count; Terraria, Bedrock and Minecraft
//! Java use the console FIFO. Names are never interpolated into a command unless they are free of quotes and control
//! characters.
use super::{Game, PlayerSource, SETTINGS_FILE, rcon, read_settings};
use crate::{error::{ApiError, ApiResult}, lifecycle, server::Srv, state::{Players, St}};
use serde_json::{Value, json};
use std::{collections::HashMap, net::{Ipv4Addr, SocketAddr}, time::Duration};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Flavor { Source, Rust, Factorio, Palworld, Zomboid }

pub fn flavor(g: &Game) -> Option<Flavor> {
    Some(match g.id { "gmod" | "tf2" | "l4d2" | "cs2" | "l4d" | "css" | "dods" | "hl2dm" | "nmrih" => Flavor::Source, "rust" => Flavor::Rust, "factorio" => Flavor::Factorio, "palworld" => Flavor::Palworld, "zomboid" => Flavor::Zomboid, _ => return None })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Online { pub name: String, pub id: Option<String> }
fn online(name: &str, id: Option<&str>) -> Online { Online { name: name.into(), id: id.map(String::from) } }

fn list_command(f: Flavor) -> &'static str {
    match f { Flavor::Source => "status", Flavor::Rust => "playerlist", Flavor::Factorio => "/players online", Flavor::Palworld => "ShowPlayers", Flavor::Zomboid => "players" }
}

// ---------- parsers ----------

/// Source `status` (`# userid "name" uniqueid ...`) and CS2 `status` (`id time ping loss state rate adr 'name'`). Bots are skipped.
pub fn parse_source_status(text: &str) -> Vec<Online> {
    use regex::Regex;
    use std::sync::LazyLock;
    static SRC: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"^#\s*(\d+)\s+(?:\d+\s+)?"(.*)"\s+(\S+)"#).unwrap());
    static CS2: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(\d+)\s+(\S+)\s+\d+\s+\d+\s+\S+\s+\d+\s+(\S+)\s+'(.*)'\s*$").unwrap());
    text.lines().filter_map(|l| {
        if let Some(c) = SRC.captures(l) { return (&c[3] != "BOT").then(|| online(&c[2], Some(&c[1]))); }
        let c = CS2.captures(l)?;
        (!matches!(&c[2], "BOT" | "[NoChan]") && &c[3] != "[NoChan]").then(|| online(&c[4], Some(&c[1])))
    }).filter(|p| !p.name.is_empty()).collect()
}

/// Rust `playerlist`: a JSON array of `{SteamID, DisplayName, ...}`.
pub fn parse_rust_playerlist(text: &str) -> Vec<Online> {
    let Some(a) = text.find('[').and_then(|s| serde_json::from_str::<Vec<Value>>(&text[s..]).ok()) else { return vec![] };
    a.iter().filter_map(|p| {
        let n = p["DisplayName"].as_str().filter(|n| !n.is_empty())?;
        Some(online(n, p["SteamID"].as_str().filter(|s| !s.is_empty())))
    }).collect()
}

/// Factorio `/players online`: `Online players (2):` then `  name (online)` lines.
pub fn parse_factorio_players(text: &str) -> Vec<Online> {
    text.lines().skip_while(|l| !l.trim_start().starts_with("Online players")).skip(1)
        .filter_map(|l| { let n = l.trim(); let n = n.strip_suffix("(online)").unwrap_or(n).trim(); (!n.is_empty()).then(|| online(n, None)) }).collect()
}

/// Palworld `ShowPlayers`: CSV `name,playeruid,steamid` (names may contain commas, so split from the right).
pub fn parse_palworld_players(text: &str) -> Vec<Online> {
    text.lines().filter_map(|l| {
        let mut p = l.trim().rsplitn(3, ',');
        let (steam, _uid, name) = (p.next()?.trim(), p.next()?, p.next()?.trim());
        (!name.is_empty() && name != "name" && !steam.is_empty()).then(|| online(name, Some(steam)))
    }).collect()
}

/// Zomboid `players`: `Players connected (2):` then `-name` lines.
pub fn parse_zomboid_players(text: &str) -> Vec<Online> {
    text.lines().filter_map(|l| { let n = l.trim().strip_prefix('-')?.trim(); (!n.is_empty()).then(|| online(n, None)) }).collect()
}

pub fn parse_list(f: Flavor, text: &str) -> Vec<Online> {
    match f { Flavor::Source => parse_source_status(text), Flavor::Rust => parse_rust_playerlist(text), Flavor::Factorio => parse_factorio_players(text), Flavor::Palworld => parse_palworld_players(text), Flavor::Zomboid => parse_zomboid_players(text) }
}

// ---------- commands ----------

/// A name or reason that is safe inside a console argument: no quotes, no `;` (Source and Factorio chain commands
/// with it), no control characters, and no leading `@` (a Minecraft target selector such as `@a` would hit everyone).
pub fn safe_text(s: &str, max: usize) -> bool { !s.is_empty() && s.len() <= max && !s.starts_with('@') && !s.contains(['"', '\\', ';']) && !s.chars().any(char::is_control) }

fn need_safe(s: &str, what: &str) -> ApiResult<()> {
    if safe_text(s, 128) { Ok(()) } else { Err(ApiError::bad("player.name", format!("{what} is empty, too long or contains quotes, `;`, a leading `@` or control characters"))) }
}

/// The RCON command to kick or ban `p`.
pub fn rcon_action(f: Flavor, ban: bool, p: &Online, reason: Option<&str>) -> ApiResult<String> {
    let (name, id) = (p.name.as_str(), p.id.as_deref().unwrap_or(""));
    let r = reason.map(str::trim).filter(|r| !r.is_empty());
    if let Some(r) = r { need_safe(r, "the reason")?; }
    if f != Flavor::Source && f != Flavor::Palworld { need_safe(name, "the player name")?; }
    let numeric = |v: &str| if !v.is_empty() && v.chars().all(|c| c.is_ascii_alphanumeric()) { Ok(()) } else { Err(ApiError::bad("player.id", "the server reported an unusable player id")) };
    Ok(match f {
        Flavor::Source => { numeric(id)?; if ban { format!("banid 0 {id} kick; writeid") } else { format!("kickid {id}{}", r.map(|r| format!(" {r}")).unwrap_or_default()) } }
        Flavor::Rust => format!("{} \"{name}\"{}", if ban { "ban" } else { "kick" }, r.map(|r| format!(" \"{r}\"")).unwrap_or_default()),
        Flavor::Factorio => format!("/{} {name}{}", if ban { "ban" } else { "kick" }, r.map(|r| format!(" {r}")).unwrap_or_default()),
        Flavor::Palworld => { numeric(id)?; format!("{} {id}", if ban { "BanPlayer" } else { "KickPlayer" }) }
        Flavor::Zomboid => format!("{} \"{name}\"{}", if ban { "banuser" } else { "kickuser" }, r.map(|r| format!(" -r \"{r}\"")).unwrap_or_default()),
    })
}

/// The console command for games without RCON (Minecraft Java, Terraria, Bedrock).
pub fn console_action(runtime: &str, mc: bool, ban: bool, name: &str, reason: Option<&str>) -> ApiResult<String> {
    need_safe(name, "the player name")?;
    let r = reason.map(str::trim).filter(|r| !r.is_empty());
    if let Some(r) = r { need_safe(r, "the reason")?; }
    let verb = if ban { "ban" } else { "kick" };
    Ok(match runtime {
        "terraria" => format!("{verb} {name}"),
        "bedrock" if ban => return Err(ApiError::bad("ban.unsupported", "Bedrock Dedicated Server has no ban command; use the allowlist instead")),
        "bedrock" => format!("kick \"{name}\""),
        _ if mc => format!("{verb} {name}{}", r.map(|r| format!(" {r}")).unwrap_or_default()),
        _ => return Err(ApiError::bad("admin.unsupported", "this server has no kick or ban support")),
    })
}

// ---------- Satisfactory HTTPS API ----------

pub fn parse_satisfactory_state(v: &Value) -> Option<(u32, u32)> {
    let s = &v["data"]["serverGameState"];
    Some((u32::try_from(s["numConnectedPlayers"].as_u64()?).ok()?, s["playerLimit"].as_u64().and_then(|n| u32::try_from(n).ok()).unwrap_or(0)))
}

/// Connected player count from the local dedicated-server API (self-signed certificate, accepted for 127.0.0.1 only).
pub async fn satisfactory_count(port: u16, admin_password: Option<&str>) -> Result<(u32, u32), String> {
    let c = reqwest::Client::builder().danger_accept_invalid_certs(true).timeout(Duration::from_secs(5)).build().map_err(|e| e.to_string())?;
    let url = format!("https://127.0.0.1:{port}/api/v1");
    let login = match admin_password { Some(p) => json!({"function": "PasswordLogin", "data": {"password": p, "minimumPrivilegeLevel": "Administrator"}}), None => json!({"function": "PasswordlessLogin", "data": {"minimumPrivilegeLevel": "Administrator"}}) };
    let r: Value = c.post(&url).json(&login).send().await.map_err(|e| e.to_string())?.json().await.map_err(|e| e.to_string())?;
    let token = r["data"]["authenticationToken"].as_str().ok_or("login refused (set admin_password in moss-game.cfg if the server is claimed)")?;
    let r: Value = c.post(&url).bearer_auth(token).json(&json!({"function": "QueryServerState"})).send().await.map_err(|e| e.to_string())?.json().await.map_err(|e| e.to_string())?;
    parse_satisfactory_state(&r).ok_or_else(|| "unexpected QueryServerState reply".into())
}

// ---------- live listing ----------

fn addr(s: &Srv) -> ApiResult<SocketAddr> { Ok(SocketAddr::from((Ipv4Addr::LOCALHOST, u16::try_from(s.port).map_err(|_| ApiError::bad("validation", "invalid port"))?))) }

fn rcon_password(st: &St, s: &Srv) -> ApiResult<String> {
    read_settings(&s.dir(st)).remove("rcon_password").filter(|p| !p.is_empty())
        .ok_or_else(|| ApiError::conflict("rcon.not_configured", format!("{SETTINGS_FILE} has no rcon_password; restart the server so Moss can set up remote admin")))
}

fn rcon_err(e: rcon::RconError) -> ApiError { ApiError::conflict("rcon.failed", e.to_string()) }

pub struct Listing { pub players: Vec<Online>, pub source: &'static str, pub count: u32, pub can_kick: bool, pub can_ban: bool }

fn alive(st: &St, s: &Srv) -> bool { st.live.lock().unwrap().get(&s.id).is_some_and(|l| l.alive) }

/// Whether Moss can kick or ban on this server at all.
fn capability(s: &Srv) -> (bool, bool) {
    match s.game() { Some(g) if flavor(g).is_some() => (true, true), Some(g) if g.id == "terraria" => (true, true), Some(g) if g.id == "bedrock" => (true, false), Some(_) => (false, false), None => (s.is_mc() && !s.is_proxy(), s.is_mc() && !s.is_proxy()) }
}

/// The players online right now, from the most direct source the game offers.
pub async fn list(st: &St, s: &Srv) -> ApiResult<Listing> {
    let (can_kick, can_ban) = capability(s);
    if let Some(g) = s.game().filter(|_| alive(st, s)) {
        if let Some(f) = flavor(g) {
            let out = rcon::exec(addr(s)?, &rcon_password(st, s)?, list_command(f)).await.map_err(rcon_err)?;
            let players = parse_list(f, &out);
            return Ok(Listing { count: players.len() as u32, players, source: "rcon", can_kick, can_ban });
        }
        if g.players == PlayerSource::Api {
            let pw = read_settings(&s.dir(st)).remove("admin_password").filter(|p| !p.is_empty());
            let (n, _) = satisfactory_count(addr(s)?.port(), pw.as_deref()).await.map_err(|e| ApiError::conflict("api.failed", e))?;
            return Ok(Listing { players: vec![], source: "api", count: n, can_kick, can_ban });
        }
    }
    let names: Vec<String> = st.live.lock().unwrap().get(&s.id).and_then(|l| l.players.as_ref().map(|p| p.sample.clone())).unwrap_or_default();
    let players: Vec<Online> = names.iter().map(|n| online(n, None)).collect();
    Ok(Listing { count: players.len() as u32, players, source: "tracked", can_kick, can_ban })
}

/// Kicks or bans `name`. RCON games look the player up in the live list first (Source and Palworld act on ids).
pub async fn act(st: &St, s: &Srv, name: &str, reason: Option<&str>, ban: bool) -> ApiResult<()> {
    let (can_kick, can_ban) = capability(s);
    if !(if ban { can_ban } else { can_kick }) { return Err(ApiError::bad("admin.unsupported", "this server does not support that action")); }
    if !alive(st, s) { return Err(ApiError::conflict("server.not_running", "server is not running")); }
    match s.game().and_then(|g| flavor(g).map(|f| (g, f))) {
        Some((_, f)) => {
            let (a, pw) = (addr(s)?, rcon_password(st, s)?);
            let players = parse_list(f, &rcon::exec(a, &pw, list_command(f)).await.map_err(rcon_err)?);
            let p = players.into_iter().find(|p| p.name == name).ok_or_else(|| ApiError::not_found("player"))?;
            let cmd = rcon_action(f, ban, &p, reason)?;
            rcon::exec(a, &pw, &cmd).await.map_err(rcon_err)?;
            Ok(())
        }
        None => lifecycle::send_command(st, s, &console_action(&s.runtime, s.is_mc(), ban, name, reason)?).await,
    }
}

/// Player counts for every ready RCON or API game in `servers`, queried together. Failures are left out.
pub async fn poll_players(st: &St, servers: &[&Srv]) -> HashMap<String, Players> {
    // each query is bounded so one misbehaving endpoint never holds up the monitor loop
    let res = futures_util::future::join_all(servers.iter().map(|s| async move { tokio::time::timeout(std::time::Duration::from_secs(10), async move {
        let g = s.game()?;
        let cfg = read_settings(&s.dir(st));
        let max = cfg.get("max_players").and_then(|v| v.parse().ok()).unwrap_or(g.max_players);
        let a = addr(s).ok()?;
        if let Some(f) = flavor(g).filter(|_| g.players == PlayerSource::Rcon) {
            let out = rcon::exec(a, cfg.get("rcon_password").filter(|p| !p.is_empty())?, list_command(f)).await.ok()?;
            let names: Vec<String> = parse_list(f, &out).into_iter().map(|p| p.name).collect();
            return Some((s.id.clone(), Players { online: names.len() as u32, max, sample: names }));
        }
        if g.players == PlayerSource::Api {
            let (n, limit) = satisfactory_count(a.port(), cfg.get("admin_password").map(String::as_str).filter(|p| !p.is_empty())).await.ok()?;
            return Some((s.id.clone(), Players { online: n, max: if limit > 0 { limit } else { max }, sample: vec![] }));
        }
        None
    }).await.ok().flatten() })).await;
    res.into_iter().flatten().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_status_both_formats() {
        let src = "hostname: x\n# userid name                uniqueid            connected ping loss state\n#      2 \"Alice B\" STEAM_1:0:123  00:12   45    0 active\n#      3 \"Bot Bob\" BOT                          active\n#      4 1 \"Carl\" [U:1:99] 01:00 30 0 active\n";
        assert_eq!(parse_source_status(src), vec![online("Alice B", Some("2")), online("Carl", Some("4"))]);
        let cs2 = "---------players--------\n  id    time ping loss      state   rate adr name\n65280   00:12   15    0     active 786432 192.168.1.5:27005 'Alice'\n65535     BOT    0    0     active      0 loopback 'Bot'\n    2   01:00   20    0     active  80000 10.0.0.2:27005 'O\\'Hara'\n#end\n";
        let p = parse_source_status(cs2);
        assert_eq!(p[0], online("Alice", Some("65280")));
        assert_eq!(p.len(), 2);
        assert!(parse_source_status("no players here").is_empty());
    }

    #[test]
    fn rust_factorio_palworld_zomboid_lists() {
        let rust = r#"[{"SteamID":"76561198000000001","OwnerSteamID":"0","DisplayName":"Alice","Ping":12,"Address":"1.2.3.4:5","ConnectedSeconds":30},{"SteamID":"76561198000000002","DisplayName":"Bob"}]"#;
        assert_eq!(parse_rust_playerlist(rust), vec![online("Alice", Some("76561198000000001")), online("Bob", Some("76561198000000002"))]);
        assert!(parse_rust_playerlist("[]").is_empty() && parse_rust_playerlist("garbage").is_empty());
        assert_eq!(parse_factorio_players("Online players (2):\n  alice (online)\n  bob (online)\n"), vec![online("alice", None), online("bob", None)]);
        assert!(parse_factorio_players("Online players (0):\n").is_empty());
        let pal = "name,playeruid,steamid\nAlice,1234567,76561198000000001\nSmith, J,99,76561198000000002\n";
        assert_eq!(parse_palworld_players(pal), vec![online("Alice", Some("76561198000000001")), online("Smith, J", Some("76561198000000002"))]);
        assert_eq!(parse_zomboid_players("Players connected (2): \n-alice\n-Bob Smith\n"), vec![online("alice", None), online("Bob Smith", None)]);
        assert!(parse_zomboid_players("Players connected (0): \n").is_empty());
    }

    #[test]
    fn commands_per_game() {
        let (a, pal) = (online("Alice", Some("2")), online("Pal", Some("7656119")));
        assert_eq!(rcon_action(Flavor::Source, false, &a, Some("afk")).unwrap(), "kickid 2 afk");
        assert_eq!(rcon_action(Flavor::Source, true, &a, None).unwrap(), "banid 0 2 kick; writeid");
        assert_eq!(rcon_action(Flavor::Rust, false, &a, None).unwrap(), "kick \"Alice\"");
        assert_eq!(rcon_action(Flavor::Rust, true, &a, Some("cheating")).unwrap(), "ban \"Alice\" \"cheating\"");
        assert_eq!(rcon_action(Flavor::Factorio, false, &a, None).unwrap(), "/kick Alice");
        assert_eq!(rcon_action(Flavor::Palworld, true, &pal, None).unwrap(), "BanPlayer 7656119");
        assert_eq!(rcon_action(Flavor::Zomboid, false, &a, None).unwrap(), "kickuser \"Alice\"");
        assert_eq!(rcon_action(Flavor::Zomboid, true, &a, Some("x")).unwrap(), "banuser \"Alice\" -r \"x\"");
        for bad in ["a\"b", "a\nb", "a\\b", "", "a\u{7}b"] {
            assert!(rcon_action(Flavor::Rust, false, &online(bad, None), None).is_err(), "{bad:?}");
            assert!(console_action("terraria", false, false, bad, None).is_err());
        }
        assert!(rcon_action(Flavor::Rust, false, &a, Some("x\" ; quit")).is_err());
        assert!(rcon_action(Flavor::Source, false, &online("x", Some("2; quit")), None).is_err());
        assert_eq!(console_action("terraria", false, true, "Al", None).unwrap(), "ban Al");
        assert_eq!(console_action("bedrock", false, false, "Al B", None).unwrap(), "kick \"Al B\"");
        assert!(console_action("bedrock", false, true, "Al", None).is_err());
        assert_eq!(console_action("paper", true, false, "Steve", Some("bye")).unwrap(), "kick Steve bye");
        assert!(console_action("valheim", false, false, "x", None).is_err());
    }

    #[test]
    fn satisfactory_state() {
        let v = json!({"data": {"serverGameState": {"numConnectedPlayers": 3, "playerLimit": 4, "isGameRunning": true}}});
        assert_eq!(parse_satisfactory_state(&v), Some((3, 4)));
        assert_eq!(parse_satisfactory_state(&json!({"errorCode": "x"})), None);
    }

    #[tokio::test]
    async fn rcon_round_trip_with_a_fake_server() {
        let a = rcon::tests::fake("pw", |c| if c == "ShowPlayers" { vec!["name,playeruid,steamid\nAl,1,2\n".into()] } else { vec![] }, true).await;
        let out = rcon::exec(a, "pw", list_command(Flavor::Palworld)).await.unwrap();
        assert_eq!(parse_list(Flavor::Palworld, &out), vec![online("Al", Some("2"))]);
    }
}
