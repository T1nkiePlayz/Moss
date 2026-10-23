//! Server records, derived state and the JSON view shared by the API and the SSE feed.
use crate::{error::{ApiError, ApiResult}, state::{Inner, St}};
use rusqlite::{Connection, Row};
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Srv {
    pub id: String,
    pub name: String,
    pub runtime: String,
    pub game_version: String,
    pub loader_version: Option<String>,
    pub java_major: i64,
    pub port: i64,
    pub memory_mb: i64,
    pub flags_profile: String,
    pub autostart: bool,
    pub restart_enabled: bool,
    pub restart_max: i64,
    pub eula_at: Option<String>,
    pub desired: String,
    pub installed: bool,
    pub install_error: Option<String>,
    pub pid: Option<i64>,
    pub pid_start: Option<i64>,
    pub tags: Vec<String>,
    pub notes: String,
    pub custom_cmd: Option<Vec<String>>,
    pub created_at: String,
    pub memory_limit_mb: Option<i64>,
    pub cpu_limit_pct: Option<i64>,
    pub pids_limit: Option<i64>,
    /// `process` (default) or `container`; applies on the next start.
    pub isolation: String,
    pub container_image: Option<String>,
    /// The current run was launched in a container (survives a mode change until the next start).
    pub in_container: bool,
    /// Container network: `open` (default bridge) or `game-only`.
    pub network: String,
    /// JSON `[{"port":19132,"proto":"udp"}]`, published in container mode only.
    pub extra_ports: String,
}

pub const COLS: &str = "id,name,runtime,game_version,loader_version,java_major,port,memory_mb,flags_profile,autostart,restart_enabled,restart_max,eula_at,desired,installed,install_error,pid,pid_start,tags,notes,custom_cmd,created_at,memory_limit_mb,cpu_limit_pct,pids_limit,isolation,container_image,in_container,network,extra_ports";

impl Srv {
    pub fn from_row(r: &Row) -> rusqlite::Result<Srv> {
        Ok(Srv {
            id: r.get(0)?, name: r.get(1)?, runtime: r.get(2)?, game_version: r.get(3)?, loader_version: r.get(4)?, java_major: r.get(5)?,
            port: r.get(6)?, memory_mb: r.get(7)?, flags_profile: r.get(8)?, autostart: r.get(9)?, restart_enabled: r.get(10)?, restart_max: r.get(11)?,
            eula_at: r.get(12)?, desired: r.get(13)?, installed: r.get(14)?, install_error: r.get(15)?, pid: r.get(16)?, pid_start: r.get(17)?,
            tags: serde_json::from_str(&r.get::<_, String>(18)?).unwrap_or_default(), notes: r.get(19)?,
            custom_cmd: r.get::<_, Option<String>>(20)?.and_then(|s| serde_json::from_str(&s).ok()), created_at: r.get(21)?,
            memory_limit_mb: r.get(22)?, cpu_limit_pct: r.get(23)?, pids_limit: r.get(24)?,
            isolation: r.get(25)?, container_image: r.get(26)?, in_container: r.get(27)?,
            network: r.get(28)?, extra_ports: r.get(29)?,
        })
    }
    pub fn dir(&self, st: &Inner) -> PathBuf { st.cfg.servers_dir().join(&self.id) }
    pub fn moss_dir(&self, st: &Inner) -> PathBuf { self.dir(st).join(".moss") }
    pub fn requested_limits(&self) -> crate::platform::cgroup::Requested {
        crate::platform::cgroup::Requested { memory_mb: self.memory_limit_mb, cpu_pct: self.cpu_limit_pct, pids: self.pids_limit }
    }
    /// A Minecraft Java server (Java, EULA file, server.properties, content). False for custom, sim and every game.
    pub fn is_mc(&self) -> bool { !matches!(self.runtime.as_str(), "custom" | "sim") && !self.is_game() }
    /// The descriptor when this is a non-Minecraft-Java game server (Bedrock, Terraria, Factorio, Steam games).
    pub fn game(&self) -> Option<&'static crate::games::Game> { crate::games::get(&self.runtime) }
    pub fn is_game(&self) -> bool { self.game().is_some() }
    /// The Minecraft EULA must be accepted (Java servers and Bedrock).
    pub fn needs_eula(&self) -> bool { (self.is_mc() && !self.is_proxy()) || self.game().is_some_and(|g| g.needs_eula) }
    pub fn is_proxy(&self) -> bool { self.runtime == "velocity" }
    pub fn content_kind(&self) -> &'static str {
        match self.runtime.as_str() {
            "paper" | "purpur" | "folia" => "plugins",
            "fabric" | "forge" | "neoforge" | "quilt" => "mods",
            "velocity" => "proxy_plugins",
            "vanilla" => "datapacks",
            _ => "none",
        }
    }
    pub fn capabilities(&self) -> Vec<&'static str> {
        let mut c = vec!["console", "files", "backups", "schedules"];
        if self.is_mc() || self.game().is_some_and(|g| g.tracks_players()) { c.push("players"); }
        if (self.is_mc() && !self.is_proxy()) || self.runtime == "bedrock" { c.push("properties"); }
        if self.is_game() && crate::cfgfiles::has_game_config(&self.runtime) { c.push("config"); }
        if self.is_mc() { c.push("java"); }
        if self.needs_eula() { c.push("eula"); }
        if self.content_kind() != "none" { c.push("content"); }
        if matches!(self.runtime.as_str(), "valheim" | "factorio" | "gmod" | "cs2") { c.push("game_mods"); }
        c
    }
}

pub fn get_sync(c: &Connection, id: &str) -> rusqlite::Result<Option<Srv>> {
    match c.query_row(&format!("SELECT {COLS} FROM servers WHERE id=?"), [id], Srv::from_row) {
        Ok(s) => Ok(Some(s)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e),
    }
}

pub async fn get(st: &St, id: &str) -> ApiResult<Srv> {
    let i = id.to_string();
    st.db.read(move |c| get_sync(c, &i)).await?.ok_or_else(|| ApiError::not_found("server"))
}

/// Servers the operator manages. Staging copies are real server rows but never appear here.
pub async fn list(st: &St) -> anyhow::Result<Vec<Srv>> {
    st.db.call(|c| {
        let mut q = c.prepare(&format!("SELECT {COLS} FROM servers WHERE id NOT IN (SELECT id FROM staging_runs) ORDER BY created_at"))?;
        q.query_map([], Srv::from_row)?.collect()
    }).await
}

/// Every server row including staging copies (for port allocation, supervision and boot reconciliation).
pub async fn list_all(st: &St) -> anyhow::Result<Vec<Srv>> {
    st.db.call(|c| {
        let mut q = c.prepare(&format!("SELECT {COLS} FROM servers ORDER BY created_at"))?;
        q.query_map([], Srv::from_row)?.collect()
    }).await
}

pub async fn is_staging(st: &St, id: &str) -> bool {
    let i = id.to_string();
    st.db.call(move |c| c.query_row("SELECT 1 FROM staging_runs WHERE id=?", [i], |_| Ok(())).map(|_| true)).await.unwrap_or(false)
}

pub async fn save_pid(st: &St, id: &str, pid: Option<i64>, start: Option<i64>, last_exit: Option<String>) {
    let (i, now) = (id.to_string(), crate::db::now());
    let _ = st.db.call(move |c| c.execute(
        "UPDATE servers SET pid=?, pid_start=?, started_at=CASE WHEN ? IS NOT NULL THEN ? ELSE started_at END, last_exit=COALESCE(?,last_exit) WHERE id=?",
        rusqlite::params![pid, start, pid, now, last_exit, i])).await;
}

/// Record whether the current run lives in a container (set at start, cleared when it exits).
pub async fn set_in_container(st: &St, id: &str, on: bool) {
    let i = id.to_string();
    let _ = st.db.call(move |c| c.execute("UPDATE servers SET in_container=? WHERE id=?", rusqlite::params![on, i])).await;
}

pub async fn set_desired(st: &St, id: &str, desired: &str) {
    let (i, d) = (id.to_string(), desired.to_string());
    let _ = st.db.call(move |c| c.execute("UPDATE servers SET desired=? WHERE id=?", [d, i])).await;
}

/// Derived observed state: (state, detail).
pub fn state_of(s: &Srv, st: &Inner) -> (String, Option<String>) {
    if !s.installed {
        return match &s.install_error { Some(e) => ("failed".into(), Some(e.clone())), None => ("installing".into(), None) };
    }
    let live = st.live.lock().unwrap();
    let l = live.get(&s.id);
    let (alive, ready, stopping, crashed) = l.map_or((false, false, false, false), |l| (l.alive, l.ready, l.stopping, l.crashed));
    let detail = l.and_then(|l| l.detail.clone());
    if alive {
        let state = if stopping { "stopping" } else if ready { "running" } else { "starting" };
        return (state.into(), detail);
    }
    if crashed { return ("crashed".into(), detail); }
    if !s.dir(st).exists() { return ("needs_attention".into(), Some("server directory is missing".into())); }
    ("stopped".into(), detail)
}

pub fn view(s: &Srv, st: &Inner) -> Value {
    let (state, detail) = state_of(s, st);
    let has_map = crate::maps::detect(st, s).is_some(); // may read the disk: not under the global `live` lock
    let live = st.live.lock().unwrap();
    let l = live.get(&s.id);
    let uptime = l.filter(|l| l.alive).and_then(|l| l.started).map(|t| t.elapsed().as_secs());
    let players = l.and_then(|l| l.players.as_ref()).filter(|_| state == "running").map(|p| json!({"online": p.online, "max": p.max, "sample": p.sample}));
    let mut capabilities = s.capabilities();
    if has_map { capabilities.push("map"); }
    json!({
        "id": s.id, "name": s.name, "runtime": s.runtime, "game_version": s.game_version, "loader_version": s.loader_version,
        "java_major": s.java_major, "port": s.port, "memory_mb": s.memory_mb, "flags_profile": s.flags_profile, "autostart": s.autostart,
        "restart_policy": {"enabled": s.restart_enabled, "max_per_10m": s.restart_max}, "eula_accepted": s.eula_at.is_some() || !s.needs_eula(),
        "desired": s.desired, "state": state, "state_detail": detail, "state_since": l.and_then(|l| l.state_since.clone()),
        "uptime_s": uptime, "pid": s.pid.filter(|_| l.is_some_and(|l| l.alive)), "players": players,
        "motd": l.and_then(|l| l.motd.clone()), "cpu_pct": l.filter(|l| l.alive).and_then(|l| l.cpu), "mem_bytes": l.filter(|l| l.alive).and_then(|l| l.mem),
        "content_kind": s.content_kind(), "tags": s.tags, "notes": s.notes, "created_at": s.created_at, "capabilities": capabilities,
        "limits": crate::platform::cgroup::limits_json(&s.requested_limits()),
        "isolation": s.isolation, "container": crate::container::view(s),
    })
}

pub async fn emit(st: &St, id: &str) {
    if let Ok(s) = get(st, id).await { st.emit("server", view(&s, st)); }
}

/// Compare derived state with the last emitted one and push an event + timeline mark on change.
pub fn note_state(st: &St, s: &Srv) {
    let (state, _) = state_of(s, st);
    let changed = {
        let mut live = st.live.lock().unwrap();
        let l = live.entry(s.id.clone()).or_default();
        if l.last_state != state { l.last_state = state.clone(); l.state_since = Some(crate::db::now()); true } else { false }
    };
    if changed { st.emit("server", view(s, st)); }
}
