//! Non-Minecraft game servers: a static descriptor per runtime id (install method, launch arguments, stop method,
//! readiness and player-tracking rules), plus the pieces that act on it: installation (`install`), Steam A2S
//! queries (`a2s`), console-log player tracking (`track`) and version discovery (`versions`).
//!
//! Games launch exactly like `custom` servers (detached, stdin FIFO, console log); they are never Java servers, so
//! `Srv::is_mc` is false for them and the Java, EULA (except Bedrock), content and container paths are skipped.
use crate::{error::{ApiError, ApiResult}, server::Srv};
use regex::Regex;
use serde_json::{Value, json};
use std::{collections::{HashMap, HashSet}, path::Path, sync::{LazyLock, Mutex}};

pub mod a2s;
pub mod admin;
pub mod install;
pub mod mods;
pub mod rcon;
pub mod rcon_setup;
pub mod track;
pub mod versions;

/// Moss-managed, user-editable `key=value` settings for games whose launch needs values (Valheim, Zomboid).
pub const SETTINGS_FILE: &str = "moss-game.cfg";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Proto { Udp, Tcp }
impl Proto { pub fn as_str(self) -> &'static str { match self { Proto::Udp => "udp", Proto::Tcp => "tcp" } } }

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Install { Bedrock, Terraria, Factorio, Steam(u32) }

/// How a graceful stop is requested.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stop { Command(&'static str), Signal(i32) }

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlayerSource { None, Log, A2s(u16), Rcon, Api }

/// Console sequence that makes a running world safe to copy: `pre` is sent in order, then the log is watched for `ready`
/// (re-sending `query` every couple of seconds) for a bounded time; `after` is always sent afterwards.
pub struct Flush { pub pre: &'static [&'static str], pub query: Option<&'static str>, pub ready: &'static str, pub after: Option<&'static str> }

/// Every command a flush sends, in order (the wait for `ready` happens between the two halves).
#[cfg(test)]
pub fn flush_sequence(f: &Flush) -> Vec<&'static str> { f.pre.iter().copied().chain(f.after).collect() }

static TERRARIA_FLUSH: Flush = Flush { pre: &["save"], query: None, ready: "saved", after: None };
static FACTORIO_FLUSH: Flush = Flush { pre: &["/server-save"], query: None, ready: "Saving finished", after: None };
static BEDROCK_FLUSH: Flush = Flush { pre: &["save hold"], query: Some("save query"), ready: "Data saved", after: Some("save resume") };

impl Srv {
    /// Extra ports above `port` this server occupies (0 for Minecraft and custom servers).
    pub fn port_span(&self) -> i64 { self.game().map_or(0, |g| i64::from(g.span())) }
}

/// Whether ports `[a, a + a_span]` and `[b, b + b_span]` overlap.
pub fn spans_overlap(a: i64, a_span: i64, b: i64, b_span: i64) -> bool { a <= b + b_span && b <= a + a_span }

/// Does a server on `port` with `span` extra ports collide with the port span of any other server in `all`?
pub fn clashes_with_span(port: i64, span: i64, self_id: &str, all: &[Srv]) -> bool {
    all.iter().filter(|o| o.id != self_id).any(|o| spans_overlap(port, span, o.port, o.port_span()))
}

/// First port at or after `preferred` whose whole span `[p, p + span]` avoids `used` and for which `bindable` holds.
pub fn first_free(preferred: i64, span: i64, used: &[i64], bindable: impl Fn(i64) -> bool) -> i64 {
    let mut p = preferred;
    while p <= 65000 {
        if (p..=p + span).all(|q| !used.contains(&q)) && bindable(p) { return p; }
        p += 1;
    }
    preferred
}

/// Inputs to a game's argument template.
pub struct Launch<'a> { pub port: u16, pub max_players: u32, pub name: &'a str, pub cfg: &'a HashMap<String, String> }
impl Launch<'_> { fn get(&self, k: &str) -> Option<&str> { self.cfg.get(k).map(String::as_str).filter(|v| !v.is_empty()) } }

pub struct Game {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub port: u16,
    pub proto: Proto,
    pub platforms: &'static [&'static str],
    pub install: Install,
    pub launch: fn(&Launch) -> Result<Vec<String>, String>,
    pub stop: Stop,
    /// Console line that means the server accepts players.
    pub ready: Option<&'static str>,
    /// Regexes whose first capture group is a player name.
    pub join: Option<&'static str>,
    pub leave: Option<&'static str>,
    pub players: PlayerSource,
    pub max_players: u32,
    /// Bedrock only: the Minecraft EULA applies.
    pub needs_eula: bool,
    /// Mark ready after this many seconds alive even if no readiness signal was seen.
    pub fallback_ready_s: u64,
}

fn strs<const N: usize>(a: [&str; N]) -> Vec<String> { a.iter().map(|s| s.to_string()).collect() }

fn bedrock(_: &Launch) -> Result<Vec<String>, String> { Ok(strs(["env", "LD_LIBRARY_PATH=.", "./bedrock_server"])) }

fn terraria(l: &Launch) -> Result<Vec<String>, String> {
    let exe = if cfg!(target_os = "macos") { "./TerrariaServer.bin.osx" } else { "./TerrariaServer.bin.x86_64" };
    Ok(vec![exe.into(), "-config".into(), "serverconfig.txt".into(), "-port".into(), l.port.to_string()])
}

fn factorio(l: &Launch) -> Result<Vec<String>, String> {
    let mut v = vec!["./bin/x64/factorio".into(), "--start-server".into(), "saves/moss.zip".into(), "--port".into(), l.port.to_string(), "--server-settings".into(), "config/server-settings.json".into()];
    if let Some(pw) = l.get("rcon_password") { v.extend(["--rcon-port".into(), l.port.to_string(), "--rcon-password".into(), pw.into()]); }
    Ok(v)
}

fn valheim(l: &Launch) -> Result<Vec<String>, String> {
    let pw = l.get("password").filter(|p| p.chars().count() >= 5).ok_or_else(|| format!("{SETTINGS_FILE} needs a password of at least 5 characters (Valheim refuses to start without one)"))?;
    let name = l.get("name").unwrap_or(l.name);
    if name.to_lowercase().contains(&pw.to_lowercase()) { return Err("the Valheim password must not be part of the server name; edit it in moss-game.cfg".into()); }
    // With BepInEx installed (`command` sets `BEPINEX_KEY`) start through Unity Doorstop, as the pack's start_server_bepinex.sh does.
    let mut v = if l.cfg.contains_key(mods::BEPINEX_KEY) {
        strs(["env", "DOORSTOP_ENABLED=1", "DOORSTOP_TARGET_ASSEMBLY=./BepInEx/core/BepInEx.Preloader.dll", "LD_LIBRARY_PATH=./doorstop_libs:./linux64", "LD_PRELOAD=libdoorstop_x64.so", "SteamAppId=892970", "./valheim_server.x86_64", "-nographics", "-batchmode", "-public", "0"])
    } else {
        strs(["env", "LD_LIBRARY_PATH=./linux64", "SteamAppId=892970", "./valheim_server.x86_64", "-nographics", "-batchmode", "-public", "0"])
    };
    v.extend(["-name".into(), name.into(), "-port".into(), l.port.to_string(), "-world".into(), l.get("world").unwrap_or("Dedicated").into(), "-password".into(), pw.into()]);
    Ok(v)
}

fn srcds(game: &str, map: &str, l: &Launch) -> Vec<String> {
    let mut v = strs(["./srcds_run", "-game", game, "-console", "-norestart", "-port"]);
    v.extend([l.port.to_string(), "+maxplayers".into(), l.max_players.to_string(), "+map".into(), map.into()]);
    if let Some(pw) = l.get("rcon_password") { v.extend(["+rcon_password".into(), pw.into()]); }
    v
}
fn gmod(l: &Launch) -> Result<Vec<String>, String> {
    let mut v = srcds("garrysmod", "gm_construct", l);
    v.extend(mods::workshop::gmod_args(l.cfg));
    Ok(v)
}
fn tf2(l: &Launch) -> Result<Vec<String>, String> { Ok(srcds("tf", "ctf_2fort", l)) }
fn l4d2(l: &Launch) -> Result<Vec<String>, String> { Ok(srcds("left4dead2", "c1m1_hotel", l)) }

fn cs2(l: &Launch) -> Result<Vec<String>, String> {
    let mut v = strs(["./game/bin/linuxsteamrt64/cs2", "-dedicated", "-console", "-usercon", "-port"]);
    v.extend([l.port.to_string(), "-maxplayers".into(), l.max_players.to_string()]);
    v.extend(mods::workshop::cs2_map_args(l.cfg));
    if let Some(pw) = l.get("rcon_password") { v.extend(["+rcon_password".into(), pw.into()]); }
    Ok(v)
}

fn rust(l: &Launch) -> Result<Vec<String>, String> {
    let mut v = strs(["env", "LD_LIBRARY_PATH=./RustDedicated_Data/Plugins/x86_64", "./RustDedicated", "-batchmode", "-nographics", "+server.port"]);
    v.extend([l.port.to_string(), "+server.queryport".into(), (u32::from(l.port) + 1).to_string(), "+server.maxplayers".into(), l.max_players.to_string(), "+server.hostname".into(), l.name.into(), "+server.identity".into(), "moss".into()]);
    if let Some(pw) = l.get("rcon_password") { v.extend(["+rcon.port".into(), l.port.to_string(), "+rcon.password".into(), pw.into(), "+rcon.web".into(), "0".into()]); }
    Ok(v)
}

/// `-players` only when moss-game.cfg sets `max_players`; otherwise ServerPlayerMaxNum in PalWorldSettings.ini (Config tab) applies.
fn palworld(l: &Launch) -> Result<Vec<String>, String> {
    let mut v = vec!["./PalServer.sh".into(), format!("-port={}", l.port)];
    if l.get("max_players").is_some() { v.push(format!("-players={}", l.max_players)); }
    Ok(v)
}

fn zomboid(l: &Launch) -> Result<Vec<String>, String> {
    let pw = l.get("password").ok_or_else(|| format!("{SETTINGS_FILE} needs an admin password (Project Zomboid asks for one on first start)"))?;
    Ok(vec!["./start-server.sh".into(), "-servername".into(), l.get("servername").unwrap_or("moss").into(), "-adminpassword".into(), pw.into(), "-port".into(), l.port.to_string()])
}

fn satisfactory(l: &Launch) -> Result<Vec<String>, String> { Ok(vec!["./FactoryServer.sh".into(), format!("-Port={}", l.port)]) }

const SIGINT: i32 = libc::SIGINT;
const LINUX: &[&str] = &["linux"];
const LINUX_MAC: &[&str] = &["linux", "macos"];

pub static GAMES: &[Game] = &[
    Game { id: "bedrock", name: "Minecraft Bedrock", description: "Official Bedrock Dedicated Server (Windows 10/11, consoles, phones). Linux only. Needs the Minecraft EULA.",
        port: 19132, proto: Proto::Udp, platforms: LINUX, install: Install::Bedrock, launch: bedrock, stop: Stop::Command("stop"),
        ready: Some("Server started"), join: Some(r"Player connected: ([^,]+),"), leave: Some(r"Player disconnected: ([^,]+),"),
        players: PlayerSource::Log, max_players: 10, needs_eula: true, fallback_ready_s: 120 },
    Game { id: "terraria", name: "Terraria", description: "Official Terraria dedicated server. Creates a medium world on first start. Linux and macOS.",
        port: 7777, proto: Proto::Tcp, platforms: LINUX_MAC, install: Install::Terraria, launch: terraria, stop: Stop::Command("exit"),
        ready: Some("Server started"), join: Some(r"(.+?) has joined\.\s*$"), leave: Some(r"(.+?) has left\.\s*$"),
        players: PlayerSource::Log, max_players: 8, needs_eula: false, fallback_ready_s: 120 },
    Game { id: "factorio", name: "Factorio", description: "Official headless Factorio server (stable or experimental). Creates a save on install. Linux only.",
        port: 34197, proto: Proto::Udp, platforms: LINUX, install: Install::Factorio, launch: factorio, stop: Stop::Command("/quit"),
        ready: Some(r"changing state from\(CreatingGame\) to\(InGame\)"), join: Some(r"\[JOIN\] (.+) joined the game"), leave: Some(r"\[LEAVE\] (.+) left the game"),
        players: PlayerSource::Log, max_players: 0, needs_eula: false, fallback_ready_s: 120 },
    Game { id: "valheim", name: "Valheim", description: "Valheim dedicated server via SteamCMD (anonymous, about 1 GB). Password is generated into moss-game.cfg. Linux x86_64 only.",
        port: 2456, proto: Proto::Udp, platforms: LINUX, install: Install::Steam(896660), launch: valheim, stop: Stop::Signal(SIGINT),
        ready: Some("Game server connected"), join: None, leave: None, players: PlayerSource::A2s(1), max_players: 10, needs_eula: false, fallback_ready_s: 300 },
    Game { id: "gmod", name: "Garry's Mod", description: "Garry's Mod dedicated server via SteamCMD (anonymous). Starts on gm_construct. Linux x86_64 only.",
        port: 27015, proto: Proto::Udp, platforms: LINUX, install: Install::Steam(4020), launch: gmod, stop: Stop::Command("quit"),
        ready: None, join: None, leave: None, players: PlayerSource::A2s(0), max_players: 16, needs_eula: false, fallback_ready_s: 300 },
    Game { id: "cs2", name: "Counter-Strike 2", description: "Counter-Strike 2 dedicated server via SteamCMD (anonymous, 30 GB or more). A game server login token is needed to be listed publicly. Linux x86_64 only.",
        port: 27015, proto: Proto::Udp, platforms: LINUX, install: Install::Steam(730), launch: cs2, stop: Stop::Command("quit"),
        ready: None, join: None, leave: None, players: PlayerSource::A2s(0), max_players: 10, needs_eula: false, fallback_ready_s: 300 },
    Game { id: "tf2", name: "Team Fortress 2", description: "Team Fortress 2 dedicated server via SteamCMD (anonymous). Starts on ctf_2fort. Linux x86_64 only.",
        port: 27015, proto: Proto::Udp, platforms: LINUX, install: Install::Steam(232250), launch: tf2, stop: Stop::Command("quit"),
        ready: None, join: None, leave: None, players: PlayerSource::A2s(0), max_players: 24, needs_eula: false, fallback_ready_s: 300 },
    Game { id: "l4d2", name: "Left 4 Dead 2", description: "Left 4 Dead 2 dedicated server via SteamCMD (anonymous). Linux x86_64 only.",
        port: 27015, proto: Proto::Udp, platforms: LINUX, install: Install::Steam(222860), launch: l4d2, stop: Stop::Command("quit"),
        ready: None, join: None, leave: None, players: PlayerSource::A2s(0), max_players: 8, needs_eula: false, fallback_ready_s: 300 },
    Game { id: "rust", name: "Rust", description: "Rust dedicated server via SteamCMD (anonymous, needs 8 GB or more RAM). Linux x86_64 only.",
        port: 28015, proto: Proto::Udp, platforms: LINUX, install: Install::Steam(258550), launch: rust, stop: Stop::Command("quit"),
        ready: Some("Server startup complete"), join: None, leave: None, players: PlayerSource::A2s(1), max_players: 50, needs_eula: false, fallback_ready_s: 600 },
    Game { id: "palworld", name: "Palworld", description: "Palworld dedicated server via SteamCMD (anonymous, 16 GB RAM recommended). Linux x86_64 only.",
        port: 8211, proto: Proto::Udp, platforms: LINUX, install: Install::Steam(2394010), launch: palworld, stop: Stop::Signal(SIGINT),
        ready: None, join: None, leave: None, players: PlayerSource::Rcon, max_players: 32, needs_eula: false, fallback_ready_s: 90 },
    Game { id: "zomboid", name: "Project Zomboid", description: "Project Zomboid dedicated server via SteamCMD (anonymous). Admin password is generated into moss-game.cfg. Linux x86_64 only.",
        port: 16261, proto: Proto::Udp, platforms: LINUX, install: Install::Steam(380870), launch: zomboid, stop: Stop::Command("quit"),
        ready: Some(r"SERVER STARTED"), join: None, leave: None, players: PlayerSource::Rcon, max_players: 16, needs_eula: false, fallback_ready_s: 600 },
    Game { id: "satisfactory", name: "Satisfactory", description: "Satisfactory dedicated server via SteamCMD (anonymous, 12 GB RAM recommended). Claim the server from the in-game server manager. Linux x86_64 only.",
        port: 7777, proto: Proto::Udp, platforms: LINUX, install: Install::Steam(1690800), launch: satisfactory, stop: Stop::Signal(SIGINT),
        ready: None, join: None, leave: None, players: PlayerSource::Api, max_players: 4, needs_eula: false, fallback_ready_s: 120 },
];

pub fn get(runtime: &str) -> Option<&'static Game> { GAMES.iter().find(|g| g.id == runtime) }

const OS: &str = if cfg!(target_os = "macos") { "macos" } else { "linux" };

impl Game {
    /// Whether this host can install and run the game (Steam dedicated servers are Linux x86_64 only).
    pub fn supported_here(&self) -> bool {
        self.platforms.contains(&OS) && (!matches!(self.install, Install::Steam(_)) || cfg!(target_arch = "x86_64"))
    }
    /// Extra ports above `port` the game also occupies (A2S query, IPv6 twin, ...).
    pub fn span(&self) -> u16 { match self.id { "valheim" => 2, "bedrock" | "rust" | "zomboid" => 1, _ => 0 } }
    /// The server-relative executable a restored copy cannot start without.
    pub fn main_binary(&self) -> &'static str {
        match self.id {
            "bedrock" => "bedrock_server",
            "terraria" => if cfg!(target_os = "macos") { "TerrariaServer.bin.osx" } else { "TerrariaServer.bin.x86_64" },
            "factorio" => "bin/x64/factorio",
            "valheim" => "valheim_server.x86_64",
            "gmod" | "tf2" | "l4d2" => "srcds_run",
            "cs2" => "game/bin/linuxsteamrt64/cs2",
            "rust" => "RustDedicated",
            "palworld" => "PalServer.sh",
            "zomboid" => "start-server.sh",
            _ => "FactoryServer.sh",
        }
    }
    /// Console commands that flush the world before a live backup; `None` when the game has no console save.
    pub fn flush(&self) -> Option<&'static Flush> { match self.id { "terraria" => Some(&TERRARIA_FLUSH), "factorio" => Some(&FACTORIO_FLUSH), "bedrock" => Some(&BEDROCK_FLUSH), _ => None } }
    pub fn tracks_players(&self) -> bool { self.players != PlayerSource::None }
    pub fn a2s_port(&self, port: i64) -> Option<u16> { match self.players { PlayerSource::A2s(off) => u16::try_from(port + i64::from(off)).ok(), _ => None } }
    /// Whether the console log carries anything the monitor wants (readiness or joins and leaves).
    pub fn reads_log(&self) -> bool { self.ready.is_some() || self.players == PlayerSource::Log }
    pub fn view(&self) -> Value {
        let tracking = match self.players { PlayerSource::None => "none", PlayerSource::Log => "log", PlayerSource::A2s(_) => "a2s", PlayerSource::Rcon => "rcon", PlayerSource::Api => "api" };
        let installable = self.supported_here();
        json!({
            "id": self.id, "name": self.name, "category": "game", "tier": if matches!(self.install, Install::Steam(_)) { "experimental" } else { "stable" },
            "content_kind": "none", "description": self.description, "installable": installable,
            "default_port": self.port, "protocol": self.proto.as_str(), "platforms": self.platforms, "player_tracking": tracking,
            "needs_eula": self.needs_eula, "max_players": self.max_players,
        })
    }
}

/// Catalogue entries for every game (installable only where the host can run them).
pub fn catalogue() -> Vec<Value> { GAMES.iter().map(Game::view).collect() }

/// Parses `key=value` lines (blank lines and `#` comments ignored).
pub fn parse_settings(text: &str) -> HashMap<String, String> {
    text.lines().filter_map(|l| { let l = l.trim(); if l.is_empty() || l.starts_with('#') { return None; } let (k, v) = l.split_once('=')?; Some((k.trim().to_string(), v.trim().to_string())) }).collect()
}

pub fn read_settings(dir: &Path) -> HashMap<String, String> {
    std::fs::read_to_string(dir.join(SETTINGS_FILE)).map(|t| parse_settings(&t)).unwrap_or_default()
}

/// The argument vector for a game server (never a shell string). Called from `lifecycle::command::build_command`.
pub fn command(dir: &Path, s: &Srv) -> ApiResult<Vec<String>> {
    let g = get(&s.runtime).ok_or_else(|| ApiError::bad("runtime.unsupported", "unknown game"))?;
    if !g.supported_here() { return Err(ApiError::conflict("platform.unsupported", format!("{} is not supported on this host ({})", g.name, g.platforms.join(" / ")))); }
    let mut cfg = read_settings(dir);
    let port16 = u16::try_from(s.port).map_err(|_| ApiError::bad("validation", "invalid port"))?;
    rcon_setup::prepare(dir, g, port16, &mut cfg); // remote admin: generated password, Palworld and Zomboid settings files
    if dir.join("BepInEx/core/BepInEx.Preloader.dll").exists() { cfg.insert(mods::BEPINEX_KEY.into(), "1".into()); }
    let max = cfg.get("max_players").and_then(|v| v.parse().ok()).unwrap_or(g.max_players);
    let port = port16;
    let argv = (g.launch)(&Launch { port, max_players: max, name: &s.name, cfg: &cfg }).map_err(|m| ApiError::conflict("game.config", m))?;
    if let Some(exe) = argv.iter().find(|a| a.starts_with("./")) && !dir.join(exe).exists() {
        return Err(ApiError::conflict("server.not_installed", format!("{exe} is missing; reinstall the server")));
    }
    Ok(argv)
}

/// The executables in an argument vector (entries like `./bedrock_server`), relative to the server directory.
pub fn executables(argv: &[String]) -> Vec<&str> { argv.iter().filter(|a| a.starts_with("./")).map(|a| &a[2..]).collect() }

/// Console-line classification for readiness and player tracking.
#[derive(Debug, PartialEq, Eq)]
pub enum Event { Ready, Join(String), Leave(String) }

fn re(src: &'static str) -> &'static Regex {
    static CACHE: LazyLock<Mutex<HashMap<&'static str, &'static Regex>>> = LazyLock::new(Default::default);
    let mut c = CACHE.lock().unwrap();
    c.entry(src).or_insert_with(|| Box::leak(Box::new(Regex::new(src).expect("valid game regex"))))
}

fn clean_name(n: &str) -> Option<String> {
    let n = n.trim().trim_start_matches(|c: char| c == ':' || c.is_whitespace());
    (!n.is_empty() && n.len() <= 64).then(|| n.to_string())
}

pub fn classify(g: &Game, line: &str) -> Option<Event> {
    if let Some(n) = g.join.and_then(|j| re(j).captures(line)).and_then(|c| clean_name(&c[1])) { return Some(Event::Join(n)); }
    if let Some(n) = g.leave.and_then(|j| re(j).captures(line)).and_then(|c| clean_name(&c[1])) { return Some(Event::Leave(n)); }
    if g.ready.is_some_and(|r| re(r).is_match(line)) { return Some(Event::Ready); }
    None
}

/// `pid -> ppid` for every process (Linux `/proc`); empty elsewhere.
pub fn proc_parents() -> HashMap<i64, i64> {
    let mut m = HashMap::new();
    let Ok(rd) = std::fs::read_dir("/proc") else { return m };
    for e in rd.flatten() {
        let Some(pid) = e.file_name().to_str().and_then(|n| n.parse::<i64>().ok()) else { continue };
        if let Some(ppid) = std::fs::read_to_string(e.path().join("stat")).ok().and_then(|s| parse_ppid(&s)) { m.insert(pid, ppid); }
    }
    m
}

/// The parent PID from a `/proc/<pid>/stat` line (the command name may itself contain spaces and parentheses).
pub fn parse_ppid(stat: &str) -> Option<i64> { stat.rsplit_once(')')?.1.split_whitespace().nth(1)?.parse().ok() }

/// Every descendant of `root` (not `root` itself), given a pid -> ppid map.
pub fn descendants(root: i64, parents: &HashMap<i64, i64>) -> Vec<i64> {
    let mut out = vec![];
    let mut seen: HashSet<i64> = HashSet::from([root]);
    let mut frontier = vec![root];
    while let Some(p) = frontier.pop() {
        for (&c, &pp) in parents { if pp == p && seen.insert(c) { out.push(c); frontier.push(c); } }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(id: &str, port: u16, cfg: &[(&str, &str)]) -> Vec<String> {
        let cfg: HashMap<String, String> = cfg.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let g = get(id).unwrap();
        (g.launch)(&Launch { port, max_players: g.max_players, name: "My Server", cfg: &cfg }).unwrap()
    }

    #[test]
    fn descriptors_are_consistent() {
        let mut ids = HashSet::new();
        for g in GAMES {
            assert!(ids.insert(g.id), "duplicate {}", g.id);
            assert!(!g.platforms.is_empty() && !g.description.is_empty() && g.port > 0);
            for r in [g.ready, g.join, g.leave].into_iter().flatten() { Regex::new(r).unwrap(); }
            if g.players == PlayerSource::Log { assert!(g.join.is_some() && g.leave.is_some(), "{}", g.id); }
            if let Install::Steam(_) = g.install { assert_eq!(g.platforms, LINUX); }
        }
        assert_eq!(GAMES.len(), 12);
        let b = get("bedrock").unwrap();
        assert_eq!((b.port, b.proto, b.needs_eula), (19132, Proto::Udp, true));
        assert_eq!((get("terraria").unwrap().port, get("terraria").unwrap().proto), (7777, Proto::Tcp));
        assert_eq!(get("factorio").unwrap().port, 34197);
        assert_eq!(get("valheim").unwrap().a2s_port(2456), Some(2457));
        assert_eq!(get("cs2").unwrap().a2s_port(27015), Some(27015));
        assert_eq!(get("bedrock").unwrap().a2s_port(19132), None);
        assert!(get("vanilla").is_none() && get("custom").is_none());
    }

    #[test]
    fn flush_sequences_and_spans() {
        let seq = |id: &str| get(id).unwrap().flush().map(flush_sequence);
        assert_eq!(seq("bedrock").unwrap(), ["save hold", "save resume"]);
        assert_eq!(get("bedrock").unwrap().flush().unwrap().query, Some("save query"));
        assert_eq!(seq("terraria").unwrap(), ["save"]);
        assert_eq!(seq("factorio").unwrap(), ["/server-save"]);
        assert!(seq("valheim").is_none() && seq("rust").is_none() && seq("palworld").is_none() && seq("gmod").is_none());
        assert_eq!((get("valheim").unwrap().span(), get("rust").unwrap().span(), get("terraria").unwrap().span()), (2, 1, 0));
    }

    #[test]
    fn two_valheim_servers_get_disjoint_spans() {
        use crate::runtimes::launch::test_srv;
        let mut a = test_srv("valheim", "latest", None, "balanced");
        a.id = "a".into(); a.port = 2456;
        let first: Vec<i64> = (a.port..=a.port + a.port_span()).collect();
        assert_eq!(first, [2456, 2457, 2458]);
        assert_eq!(first_free(2456, 2, &first, |_| true), 2459);
        assert_eq!(first_free(2456, 2, &[], |_| true), 2456);
        assert_eq!(first_free(2456, 2, &[2458], |p| p != 2459), 2460);
        assert!(clashes_with_span(2457, 2, "b", std::slice::from_ref(&a)));
        assert!(clashes_with_span(2454, 2, "b", std::slice::from_ref(&a)));
        assert!(!clashes_with_span(2459, 2, "b", std::slice::from_ref(&a)));
        assert!(!clashes_with_span(2457, 2, "a", std::slice::from_ref(&a)));
        // reverse direction: a Minecraft server sitting on the first one's query port
        let mut mc = test_srv("paper", "1.21", None, "balanced");
        mc.id = "m".into(); mc.port = 2457;
        assert!(clashes_with_span(2456, 2, "a", std::slice::from_ref(&mc)));
        assert!(clashes_with_span(2458, 0, "m", std::slice::from_ref(&a)));
    }

    #[test]
    fn catalogue_entries_are_games() {
        let c = catalogue();
        assert_eq!(c.len(), GAMES.len());
        assert!(c.iter().all(|e| e["category"] == "game" && e["content_kind"] == "none" && e["default_port"].is_number()));
        let t = c.iter().find(|e| e["id"] == "terraria").unwrap();
        assert_eq!((t["protocol"].as_str(), t["player_tracking"].as_str()), (Some("tcp"), Some("log")));
    }

    #[test]
    fn argv_per_game() {
        assert_eq!(argv("bedrock", 19132, &[]), ["env", "LD_LIBRARY_PATH=.", "./bedrock_server"]);
        let t = argv("terraria", 7778, &[]);
        assert!(t[0].starts_with("./TerrariaServer.bin.") && t[1..] == ["-config", "serverconfig.txt", "-port", "7778"]);
        assert_eq!(argv("factorio", 34198, &[]), ["./bin/x64/factorio", "--start-server", "saves/moss.zip", "--port", "34198", "--server-settings", "config/server-settings.json"]);
        let v = argv("valheim", 2456, &[("password", "hunter22"), ("world", "Midgard")]);
        assert_eq!(&v[..4], ["env", "LD_LIBRARY_PATH=./linux64", "SteamAppId=892970", "./valheim_server.x86_64"]);
        assert!(v.windows(2).any(|w| w == ["-port", "2456"]) && v.windows(2).any(|w| w == ["-world", "Midgard"]) && v.windows(2).any(|w| w == ["-password", "hunter22"]) && v.windows(2).any(|w| w == ["-name", "My Server"]));
        assert_eq!(argv("gmod", 27016, &[]), ["./srcds_run", "-game", "garrysmod", "-console", "-norestart", "-port", "27016", "+maxplayers", "16", "+map", "gm_construct"]);
        assert_eq!(&argv("tf2", 27015, &[])[..3], ["./srcds_run", "-game", "tf"]);
        assert!(argv("tf2", 27015, &[]).ends_with(&["+map".to_string(), "ctf_2fort".to_string()]));
        assert!(argv("l4d2", 27015, &[]).contains(&"left4dead2".to_string()));
        assert_eq!(argv("cs2", 27015, &[])[0], "./game/bin/linuxsteamrt64/cs2");
        let r = argv("rust", 28015, &[]);
        assert!(r.windows(2).any(|w| w == ["+server.port", "28015"]) && r.windows(2).any(|w| w == ["+server.queryport", "28016"]));
        assert_eq!(argv("palworld", 8211, &[]), ["./PalServer.sh", "-port=8211"]);
        assert_eq!(argv("zomboid", 16261, &[("password", "pw12345")]), ["./start-server.sh", "-servername", "moss", "-adminpassword", "pw12345", "-port", "16261"]);
        assert_eq!(argv("satisfactory", 7777, &[]), ["./FactoryServer.sh", "-Port=7777"]);
    }

    #[test]
    fn rcon_arguments_follow_the_password() {
        let a = |id: &str, port| argv(id, port, &[("rcon_password", "s3cret")]);
        assert!(a("gmod", 27015).ends_with(&["+rcon_password".to_string(), "s3cret".to_string()]));
        assert!(a("cs2", 27015).windows(2).any(|w| w == ["+rcon_password", "s3cret"]));
        assert!(a("rust", 28015).windows(2).any(|w| w == ["+rcon.port", "28015"]) && a("rust", 28015).ends_with(&["+rcon.web".to_string(), "0".to_string()]));
        assert!(a("factorio", 34197).ends_with(&["--rcon-port".to_string(), "34197".to_string(), "--rcon-password".to_string(), "s3cret".to_string()]));
    }

    #[test]
    fn valheim_and_zomboid_demand_passwords() {
        let none = HashMap::new();
        let l = |cfg| Launch { port: 2456, max_players: 10, name: "x", cfg };
        assert!(valheim(&l(&none)).is_err() && zomboid(&l(&none)).is_err());
        let short: HashMap<String, String> = [("password".to_string(), "abc".to_string())].into();
        assert!(valheim(&l(&short)).is_err());
        let has_name: HashMap<String, String> = [("password".to_string(), "Server".to_string())].into();
        let named = Launch { port: 2456, max_players: 10, name: "My server", cfg: &has_name };
        assert!(valheim(&named).is_err());
    }

    #[test]
    fn executables_are_found() { assert_eq!(executables(&argv("valheim", 1, &[("password", "abcdef")])), ["valheim_server.x86_64"]); }

    #[test]
    fn log_regexes() {
        let (b, t, f) = (get("bedrock").unwrap(), get("terraria").unwrap(), get("factorio").unwrap());
        assert_eq!(classify(b, "[2025-01-01 10:00:00:123 INFO] Server started."), Some(Event::Ready));
        assert_eq!(classify(b, "[2025-01-01 10:00:00:123 INFO] Player connected: Steve Smith, xuid: 2535"), Some(Event::Join("Steve Smith".into())));
        assert_eq!(classify(b, "[2025-01-01 10:05:00:123 INFO] Player disconnected: Steve Smith, xuid: 2535"), Some(Event::Leave("Steve Smith".into())));
        assert_eq!(classify(b, "[INFO] Running AutoCompaction..."), None);
        assert_eq!(classify(t, "Server started"), Some(Event::Ready));
        assert_eq!(classify(t, ": Alice has joined."), Some(Event::Join("Alice".into())));
        assert_eq!(classify(t, "Alice has left."), Some(Event::Leave("Alice".into())));
        assert_eq!(classify(t, "127.0.0.1:5000 is connecting..."), None);
        assert_eq!(classify(f, "  12.345 Info ServerMultiplayerManager.cpp:808: changing state from(CreatingGame) to(InGame)"), Some(Event::Ready));
        assert_eq!(classify(f, "2025-01-01 10:00:00 [JOIN] bob joined the game"), Some(Event::Join("bob".into())));
        assert_eq!(classify(f, "2025-01-01 10:00:09 [LEAVE] bob left the game"), Some(Event::Leave("bob".into())));
        assert_eq!(classify(f, "2025-01-01 10:00:09 [CHAT] bob: [JOIN] hi"), None);
        assert_eq!(classify(get("rust").unwrap(), "Server startup complete"), Some(Event::Ready));
        assert_eq!(classify(get("zomboid").unwrap(), "*** SERVER STARTED ****"), Some(Event::Ready));
        assert_eq!(classify(get("valheim").unwrap(), "02/02/2025 10:00:00: Game server connected"), Some(Event::Ready));
    }

    #[test]
    fn settings_parse() {
        let m = parse_settings("# comment\npassword = abc12345\n\nworld=Dedicated\nbad line\n");
        assert_eq!((m.get("password").map(String::as_str), m.get("world").map(String::as_str), m.len()), (Some("abc12345"), Some("Dedicated"), 2));
    }

    #[test]
    fn process_tree() {
        assert_eq!(parse_ppid("123 (my (game) x) S 45 123 123 0 -1"), Some(45));
        assert_eq!(parse_ppid("garbage"), None);
        let parents: HashMap<i64, i64> = [(10, 1), (11, 10), (12, 11), (13, 10), (20, 1)].into();
        let mut d = descendants(10, &parents);
        d.sort();
        assert_eq!(d, vec![11, 12, 13]);
        assert!(descendants(99, &parents).is_empty());
    }

    #[test]
    fn capabilities_follow_the_game() {
        use crate::runtimes::launch::test_srv;
        let caps = |rt: &str| test_srv(rt, "latest", None, "balanced").capabilities();
        let paper = caps("paper");
        assert!(["java", "eula", "properties", "players", "content"].iter().all(|c| paper.contains(c)));
        let bedrock = caps("bedrock");
        assert!(["players", "properties", "eula", "console", "backups"].iter().all(|c| bedrock.contains(c)) && !bedrock.contains(&"java") && !bedrock.contains(&"content"));
        let terraria = caps("terraria");
        assert!(terraria.contains(&"players") && !terraria.contains(&"properties") && !terraria.contains(&"eula") && !terraria.contains(&"java"));
        assert!(caps("palworld").contains(&"players") && caps("palworld").contains(&"files") && caps("satisfactory").contains(&"players") && !caps("valheim").is_empty());
        let v = test_srv("valheim", "latest", None, "balanced");
        assert!(v.is_game() && !v.is_mc() && !v.needs_eula() && test_srv("bedrock", "x", None, "balanced").needs_eula());
        assert!(!test_srv("custom", "custom", None, "balanced").is_game());
    }

    #[test]
    fn command_needs_installed_files_and_runs_in_a_container_too() {
        use crate::runtimes::launch::test_srv;
        let dir = std::env::temp_dir().join(format!("moss-games-cmd-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = test_srv("bedrock", "1.21.0", None, "balanced");
        if !cfg!(target_os = "linux") { // Bedrock is Linux-only: elsewhere it is refused before anything else
            assert_eq!(command(&dir, &s).unwrap_err().code, "platform.unsupported");
            return;
        }
        let missing = command(&dir, &s).unwrap_err();
        assert_eq!(missing.code, "server.not_installed");
        std::fs::write(dir.join("bedrock_server"), "").unwrap();
        assert_eq!(command(&dir, &s).unwrap().last().map(String::as_str), Some("./bedrock_server"));
        s.isolation = "container".into();
        assert_eq!(command(&dir, &s).unwrap().last().map(String::as_str), Some("./bedrock_server"), "same argv inside the container");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
