//! Container-mode glue for the lifecycle: start, probe (re-attach), exit facts, batched liveness and stats. The
//! engine is looked up per call, so a server launched before a controller restart is found again by its name.
use crate::{container::{self, Engine, Inspect, RunSpec, Stat, image}, error::{ApiError, ApiResult}, platform, server::{self, Srv}, state::St};
use std::{collections::HashMap, sync::Arc};

pub const DEFAULT_PIDS: u32 = 4096;

pub async fn engine(st: &St) -> ApiResult<Arc<dyn Engine>> {
    container::engine(st).await.map_err(|m| ApiError::conflict("container.engine_missing", format!("This server is set to run in a container, but {m}")))
}

/// Memory cap (MB), CPUs and process limit for the container, from the same per-server limits process mode uses.
pub fn limits(s: &Srv) -> (u64, Option<f64>, u32) {
    let req = s.requested_limits();
    let mem = req.memory_mb.map(|m| m.max(256) as u64).unwrap_or_else(|| platform::cgroup::default_memory_cap(s.memory_mb) >> 20);
    (mem, req.cpu_pct.map(|c| c as f64 / 100.0), req.pids.map_or(DEFAULT_PIDS, |p| p.clamp(64, 32_768) as u32))
}

pub fn run_spec(st: &St, s: &Srv, argv: Vec<String>) -> RunSpec {
    let (memory_mb, cpus, pids) = limits(s);
    RunSpec {
        name: container::container_name(&s.id), server_id: s.id.clone(), image: image::for_server(s), dir: s.dir(st), port: s.port as u16, memory_mb, cpus, pids,
        uid: unsafe { libc::getuid() }, gid: unsafe { libc::getgid() }, selinux: selinux_enforcing(), argv,
    }
}

pub fn selinux_enforcing() -> bool { std::fs::read_to_string("/sys/fs/selinux/enforce").is_ok_and(|v| v.trim() == "1") }

/// Create and start the container; returns the container's host PID (informational) and its start time.
pub async fn spawn(st: &St, s: &Srv, argv: Vec<String>) -> ApiResult<(i64, i64)> {
    if !container::valid_id(&s.id) { return Err(ApiError::conflict("container.bad_id", "this server cannot run in a container")); }
    let eng = engine(st).await?;
    let spec = run_spec(st, s, argv);
    let name = spec.name.clone();
    let i = tokio::task::spawn_blocking(move || -> ApiResult<Inspect> {
        if !eng.image_present(&spec.image).map_err(|e| ApiError::conflict("container.engine_error", e.to_string()))? {
            return Err(ApiError::conflict("container.image_missing", format!("The container image {} has not been downloaded yet. Open this server's settings, Isolation, and choose Download image.", spec.image)));
        }
        let _ = eng.rm(&spec.name); // a stale, stopped container from an unclean exit
        eng.run(&spec).map_err(|e| ApiError::conflict("container.start_failed", format!("could not start the container: {e}")))?;
        let i = eng.inspect(&spec.name).map_err(|e| ApiError::conflict("container.start_failed", e.to_string()))?;
        match i {
            Some(i) if i.running => Ok(i),
            other => {
                let logs = eng.logs(&spec.name, 15).unwrap_or_default();
                let _ = eng.rm(&spec.name);
                let why = other.map_or("it is gone".to_string(), |i| format!("exit code {}", i.exit_code.unwrap_or(-1)));
                Err(ApiError::conflict("container.start_failed", format!("the container stopped right after starting ({why}). {}", logs.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim())))
            }
        }
    }).await.map_err(anyhow::Error::from)??;
    container::set_alive(&name, true);
    server::set_in_container(st, &s.id, true).await;
    let pid = i.pid.max(1);
    Ok((pid, platform::pid_start(pid).unwrap_or(0) as i64))
}

/// Ask the engine about a server's container: `Some(true)` running, `Some(false)` gone or exited, `None` unknown.
pub async fn probe(st: &St, s: &Srv) -> Option<bool> {
    let eng = container::engine(st).await.ok()?;
    let name = container::container_name(&s.id);
    let n2 = name.clone();
    let r = tokio::task::spawn_blocking(move || eng.inspect(&n2)).await.ok()?;
    let alive = match r { Ok(Some(i)) => i.running, Ok(None) => false, Err(_) => return None };
    container::set_alive(&name, alive);
    Some(alive)
}

/// Exit code and whether the OOM killer fired, then remove the container.
pub async fn collect_exit(st: &St, s: &Srv) -> (Option<i32>, bool) {
    let Ok(eng) = container::engine(st).await else { return (None, false) };
    let name = container::container_name(&s.id);
    let r = tokio::task::spawn_blocking(move || {
        let i = eng.inspect(&name).ok().flatten();
        if i.as_ref().is_none_or(|i| !i.running) { let _ = eng.rm(&name); }
        i
    }).await.ok().flatten();
    r.map_or((None, false), |i| (i.exit_code, i.oom_killed))
}

/// Refresh liveness of every container-mode server with one `ps`. An unreachable engine changes nothing.
pub async fn refresh(st: &St, servers: &[Srv]) {
    let mine: Vec<String> = servers.iter().filter(|s| s.in_container).map(|s| container::container_name(&s.id)).collect();
    if mine.is_empty() { return; }
    let Ok(eng) = container::engine(st).await else { return };
    let Ok(Ok(up)) = tokio::task::spawn_blocking(move || eng.running()).await else { return };
    for n in mine { let alive = up.contains(&n); container::set_alive(&n, alive); }
}

/// One batched `stats --no-stream` for all running container servers.
pub async fn stats(st: &St, names: Vec<String>) -> HashMap<String, Stat> {
    if names.is_empty() { return HashMap::new(); }
    let Ok(eng) = container::engine(st).await else { return HashMap::new() };
    tokio::task::spawn_blocking(move || eng.stats(&names).unwrap_or_default()).await.unwrap_or_default()
}

/// Is the container still running (direct inspect, used while waiting for a stop)? Unknown counts as running.
pub async fn running_now(st: &St, s: &Srv) -> bool { probe(st, s).await.unwrap_or(true) }

pub async fn stop_container(st: &St, s: &Srv, timeout_s: u32) {
    if let Ok(eng) = container::engine(st).await {
        let n = container::container_name(&s.id);
        let _ = tokio::task::spawn_blocking(move || eng.stop(&n, timeout_s)).await;
    }
}

pub async fn kill_container(st: &St, s: &Srv) {
    if let Ok(eng) = container::engine(st).await {
        let n = container::container_name(&s.id);
        let _ = tokio::task::spawn_blocking(move || eng.kill(&n)).await;
    }
}

/// Remove a server's container (used when its record is deleted).
pub async fn remove(st: &St, id: &str) {
    if !container::valid_id(id) { return; }
    if let Ok(eng) = container::engine(st).await {
        let n = container::container_name(id);
        let _ = tokio::task::spawn_blocking({ let n = n.clone(); move || eng.rm(&n) }).await;
        container::forget_alive(&n);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{container::{Handle, fake::{self, Fake}}, runtimes::launch::test_srv};
    use clap::Parser;

    #[derive(Parser)] struct W { #[command(flatten)] c: crate::config::Config }

    async fn state(fake: &Arc<Fake>) -> St {
        let dir = std::env::temp_dir().join(format!("moss-ctr-{}", uuid::Uuid::new_v4()));
        let cfg = W::parse_from(["x", "--auth", "dev", "--data-dir", dir.to_str().unwrap()]).c;
        let st = crate::serve::build_state(cfg).await.unwrap();
        // the Arc<Inner> is not shared yet, so the handle can be swapped in place
        let mut st = st;
        Arc::get_mut(&mut st).unwrap().container = Handle::with(fake.clone());
        st
    }
    fn srv(id: &str) -> Srv { let mut s = test_srv("paper", "1.21.1", None, "minimal"); s.id = id.into(); s.isolation = "container".into(); s.port = 25570; s }

    #[tokio::test]
    async fn start_needs_the_image_then_runs_one_locked_down_container() {
        let f = Arc::new(Fake::default());
        let st = state(&f).await;
        let mut s = srv("0190-aaaa");
        s.memory_limit_mb = Some(3000); s.cpu_limit_pct = Some(150); s.pids_limit = Some(500);
        let e = spawn(&st, &s, vec!["java".into(), "-jar".into(), "server.jar".into()]).await.unwrap_err();
        assert_eq!(e.code, "container.image_missing");
        assert_eq!(f.calls_named("run "), 0);
        f.images.lock().unwrap().insert("docker.io/library/eclipse-temurin:21-jre".into());
        let (pid, _) = spawn(&st, &s, vec!["java".into(), "-jar".into(), "server.jar".into()]).await.unwrap();
        assert_eq!(pid, 4242);
        let spec = f.runs.lock().unwrap()[0].clone();
        assert_eq!((spec.name.as_str(), spec.port, spec.memory_mb, spec.cpus, spec.pids), ("moss-0190-aaaa", 25570, 3000, Some(1.5), 500));
        assert_eq!(spec.dir, s.dir(&st));
        assert!(crate::container::is_alive("moss-0190-aaaa"));
    }

    #[tokio::test]
    async fn a_container_that_dies_at_once_is_reported_and_removed() {
        let f = Arc::new(Fake::default());
        let st = state(&f).await;
        f.images.lock().unwrap().insert("docker.io/library/eclipse-temurin:21-jre".into());
        *f.die_on_start.lock().unwrap() = true;
        let e = spawn(&st, &srv("0190-bbbb"), vec!["java".into()]).await.unwrap_err();
        assert_eq!(e.code, "container.start_failed");
        assert!(e.message.contains("exit code 1") && e.message.contains("main class"), "{}", e.message);
        assert!(f.containers.lock().unwrap().is_empty(), "the dead container is cleaned up");
    }

    #[tokio::test]
    async fn probe_reattaches_by_name_and_never_reads_an_unreachable_engine_as_gone() {
        let f = Arc::new(Fake::default());
        let st = state(&f).await;
        let s = srv("0190-cccc");
        assert_eq!(probe(&st, &s).await, Some(false), "no such container");
        f.set("moss-0190-cccc", fake::running(77));
        assert_eq!(probe(&st, &s).await, Some(true));
        assert!(crate::container::is_alive("moss-0190-cccc"));
        *f.down.lock().unwrap() = true;
        assert_eq!(probe(&st, &s).await, None, "unknown, not dead");
        assert!(crate::container::is_alive("moss-0190-cccc"), "liveness unchanged while the engine is down");
    }

    #[tokio::test]
    async fn exit_facts_are_collected_and_the_container_removed() {
        let f = Arc::new(Fake::default());
        let st = state(&f).await;
        f.set("moss-0190-dddd", fake::exited(137, true));
        assert_eq!(collect_exit(&st, &srv("0190-dddd")).await, (Some(137), true));
        assert!(f.containers.lock().unwrap().is_empty());
        assert_eq!(collect_exit(&st, &srv("0190-none")).await, (None, false));
    }

    #[tokio::test]
    async fn liveness_and_stats_use_one_engine_call_for_all_servers() {
        let f = Arc::new(Fake::default());
        let st = state(&f).await;
        let mut all = vec![];
        for id in ["0190-e1", "0190-e2", "0190-e3"] { let mut s = srv(id); s.in_container = true; all.push(s); }
        f.set("moss-0190-e1", fake::running(1)); f.set("moss-0190-e2", fake::exited(1, false));
        refresh(&st, &all).await;
        assert_eq!(f.calls_named("ps"), 1);
        assert!(crate::container::is_alive("moss-0190-e1") && !crate::container::is_alive("moss-0190-e2") && !crate::container::is_alive("moss-0190-e3"));
        f.stats_out.lock().unwrap().insert("moss-0190-e1".into(), Stat { cpu_pct: 12.0, mem_bytes: 1 << 30 });
        let m = stats(&st, vec!["moss-0190-e1".into(), "moss-0190-e2".into()]).await;
        assert_eq!(f.calls_named("stats"), 1);
        assert_eq!(m["moss-0190-e1"].mem_bytes, 1 << 30);
    }

    #[test]
    fn default_limits_follow_the_process_mode_defaults() {
        let mut s = srv("x");
        assert_eq!(limits(&s), ((4096 + 1024 + 512) as u64, None, DEFAULT_PIDS));
        s.cpu_limit_pct = Some(50); s.memory_limit_mb = Some(100);
        assert_eq!(limits(&s), (256, Some(0.5), DEFAULT_PIDS));
    }
}
