//! Optional container isolation for game servers (Docker or Podman, driven through their CLI with argument arrays
//! only). `Engine` is the seam: `cli::CliEngine` talks to the real CLI, tests use a fake. Nothing here ever accepts
//! flags, mounts or privileges from the API: `args` builds the one fixed, locked-down command line.
use crate::{server::Srv, state::St};
use anyhow::Result;
use serde_json::{Value, json};
use std::{collections::{HashMap, HashSet}, path::PathBuf, sync::{Arc, Mutex, OnceLock}, time::{Duration, Instant}};

pub mod args;
pub mod cli;
pub mod image;
#[cfg(test)]
pub mod fake;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind { Docker, Podman }
impl Kind { pub fn name(self) -> &'static str { match self { Kind::Docker => "docker", Kind::Podman => "podman" } } }

#[derive(Clone, Debug)]
pub struct EngineInfo { pub kind: Kind, pub cli: PathBuf, pub version: String, pub rootless: bool }

/// Everything needed to run one game server container. All fields are produced by Moss, never by the API.
#[derive(Clone, Debug)]
pub struct RunSpec {
    pub name: String,
    pub server_id: String,
    pub image: String,
    /// Host server directory, the only bind mount (at `/data`).
    pub dir: PathBuf,
    pub port: u16,
    pub memory_mb: u64,
    pub cpus: Option<f64>,
    pub pids: u32,
    pub uid: u32,
    pub gid: u32,
    pub selinux: bool,
    /// The game command (argv[0] is resolved inside the image, e.g. `java`).
    pub argv: Vec<String>,
}

/// A short-lived container for an installer: same mounts and hardening, `--rm`, output streamed.
#[derive(Clone, Debug)]
pub struct OnceSpec { pub name: String, pub image: String, pub dir: PathBuf, pub memory_mb: u64, pub pids: u32, pub uid: u32, pub gid: u32, pub selinux: bool, pub argv: Vec<String> }

#[derive(Clone, Debug, PartialEq)]
pub struct Inspect { pub running: bool, pub status: String, pub exit_code: Option<i32>, pub oom_killed: bool, pub pid: i64 }

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stat { pub cpu_pct: f64, pub mem_bytes: u64 }

/// The container CLI surface Moss uses. Blocking: callers run it on `spawn_blocking`.
pub trait Engine: Send + Sync {
    fn info(&self) -> EngineInfo;
    fn image_present(&self, image: &str) -> Result<bool>;
    /// Pull an image; `on_line` receives progress lines and returns false to cancel.
    fn pull(&self, image: &str, on_line: &mut dyn FnMut(&str) -> bool) -> Result<()>;
    fn run(&self, spec: &RunSpec) -> Result<()>;
    /// `Ok(None)` = the container does not exist; `Err` = the engine could not answer (never read that as "gone").
    fn inspect(&self, name: &str) -> Result<Option<Inspect>>;
    /// Names of running Moss containers (one call for all servers).
    fn running(&self) -> Result<HashSet<String>>;
    fn stop(&self, name: &str, timeout_s: u32) -> Result<()>;
    fn kill(&self, name: &str) -> Result<()>;
    fn rm(&self, name: &str) -> Result<()>;
    fn logs(&self, name: &str, tail: u32) -> Result<String>;
    /// One batched `stats --no-stream` for all `names`.
    fn stats(&self, names: &[String]) -> Result<HashMap<String, Stat>>;
    /// Run an installer container to completion; returns its exit code. `on_line` returns false to cancel.
    fn run_once(&self, spec: &OnceSpec, on_line: &mut dyn FnMut(&str) -> bool, timeout: Duration) -> Result<i32>;
}

pub fn container_name(server_id: &str) -> String { format!("moss-{server_id}") }

/// Server ids are UUIDs; anything else never reaches a command line.
pub fn valid_id(id: &str) -> bool { !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') }

/// Can this server run in a container? Minecraft runtimes only (the image carries Java); `sim` only for development.
pub fn supported(s: &Srv, dev_runtimes: bool) -> bool { s.is_mc() || (s.runtime == "sim" && dev_runtimes) }

/// The `container` object of the server view.
pub fn view(s: &Srv) -> Value {
    json!({"image": image::for_server(s), "image_override": s.container_image, "running_in_container": s.in_container,
        "restart_needed": s.in_container != (s.isolation == "container") && s.pid.is_some()})
}

// ---- engine handle (detected lazily, cached briefly) --------------------------------------------------------------

#[derive(Default)]
pub struct Handle { cached: Mutex<Option<(Instant, Result<Arc<dyn Engine>, String>)>> }

impl Handle {
    /// Install a ready engine (tests).
    #[cfg(test)]
    pub fn with(e: Arc<dyn Engine>) -> Handle { Handle { cached: Mutex::new(Some((Instant::now() + Duration::from_secs(86_400 * 365), Ok(e)))) } }
}

/// The detected engine, or the reason there is none. Detection runs the CLI, so it is cached (20 s ok, 5 s failure).
pub async fn engine(st: &St) -> Result<Arc<dyn Engine>, String> {
    {
        let c = st.container.cached.lock().unwrap();
        if let Some((at, r)) = c.as_ref() {
            let ttl = if r.is_ok() { Duration::from_secs(20) } else { Duration::from_secs(5) };
            if at.elapsed() < ttl || *at > Instant::now() { return r.clone(); }
        }
    }
    let pref = st.cfg.container_engine.clone();
    let r = tokio::task::spawn_blocking(move || cli::detect(&pref).map(|e| Arc::new(e) as Arc<dyn Engine>)).await.unwrap_or_else(|_| Err("engine detection failed".into()));
    *st.container.cached.lock().unwrap() = Some((Instant::now(), r.clone()));
    r
}

/// Forget the cached detection (the status endpoint and Settings use this so a fresh install is noticed at once).
pub fn forget(st: &St) { *st.container.cached.lock().unwrap() = None; }

pub const NO_ENGINE_HELP: &str = "Neither Docker nor Podman was found. Install one of them (rootless Podman is the safest choice), make sure the user running Moss can use it, then check again. Servers keep running without isolation until then.";

// ---- liveness cache -----------------------------------------------------------------------------------------------

/// Liveness of container-mode servers, refreshed once per supervision pass from one `ps` call. A container that has
/// never been seen counts as alive: an unreachable engine must never turn into a "crash".
fn alive_map() -> &'static Mutex<HashMap<String, bool>> { static M: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new(); M.get_or_init(Default::default) }
pub fn set_alive(name: &str, alive: bool) { alive_map().lock().unwrap().insert(name.to_string(), alive); }
pub fn is_alive(name: &str) -> bool { alive_map().lock().unwrap().get(name).copied().unwrap_or(true) }
pub fn forget_alive(name: &str) { alive_map().lock().unwrap().remove(name); }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_and_ids() {
        assert_eq!(container_name("0190-abc"), "moss-0190-abc");
        assert!(valid_id("0190a1b2-c3d4") && !valid_id("a b") && !valid_id("../x") && !valid_id(""));
    }
    #[test]
    fn unknown_containers_count_as_alive() {
        assert!(is_alive("moss-never-seen"));
        set_alive("moss-t1", false);
        assert!(!is_alive("moss-t1"));
        forget_alive("moss-t1");
        assert!(is_alive("moss-t1"));
    }
}
