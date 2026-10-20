//! Start the bundled `moss serve` as a detached process, or attach to the one a previous launch left running.
//! Real sign-in is always on (`--auth supabase`), the controller listens on loopback only, and nothing goes through a shell.
use crate::{health, state, state::Controller};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

pub struct Paths {
    /// Per-user app data directory: holds `data/`, `config/`, `moss.log` and `controller.json`.
    pub root: PathBuf,
    /// The bundled `moss` executable.
    pub binary: PathBuf,
    /// The `.env` shipped with the app (public sign-in settings), copied into `config/` on every start.
    pub shipped_env: Option<PathBuf>,
}

impl Paths {
    pub fn data_dir(&self) -> PathBuf { self.root.join("data") }
    /// Working directory of the controller: it reads `.env` and `.env.local` from here, so people can add their own settings.
    pub fn config_dir(&self) -> PathBuf { self.root.join("config") }
    pub fn log_file(&self) -> PathBuf { self.root.join("moss.log") }
}

/// A controller this app started or found, and the child handle when this launch started it.
pub struct Handle {
    pub controller: Controller,
    child: Option<Child>,
}

impl Handle {
    pub fn started_here(&self) -> bool { self.child.is_some() }
    pub fn url(&self) -> String { format!("http://127.0.0.1:{}/", self.controller.port) }
}

/// Command line for the controller. Explicit flags beat any MOSS_* variables in the person's environment.
pub fn serve_args(port: u16, data_dir: &Path) -> Vec<OsString> {
    ["serve", "--bind", "127.0.0.1", "--port", &port.to_string(), "--auth", "supabase", "--data-dir"]
        .iter().map(OsString::from).chain([data_dir.as_os_str().to_owned()]).collect()
}

fn alive(h: &mut Option<Child>, pid: u32) -> bool {
    match h { Some(c) => matches!(c.try_wait(), Ok(None)), None => state::pid_alive(pid) }
}

fn spawn(p: &Paths, port: u16) -> Result<Child, String> {
    let io = |e: std::io::Error| format!("Could not start Moss: {e}");
    std::fs::create_dir_all(p.data_dir()).map_err(io)?;
    std::fs::create_dir_all(p.config_dir()).map_err(io)?;
    if let Some(env) = &p.shipped_env { let _ = std::fs::copy(env, p.config_dir().join(".env")); }
    let log = std::fs::OpenOptions::new().create(true).append(true).open(p.log_file()).map_err(io)?;
    let mut cmd = Command::new(&p.binary);
    cmd.args(serve_args(port, &p.data_dir())).current_dir(p.config_dir())
        .env_remove("MOSS_AUTH").env_remove("MOSS_BIND").env_remove("MOSS_PORT").env_remove("MOSS_DATA_DIR")
        .stdin(Stdio::null()).stdout(log.try_clone().map_err(io)?).stderr(log);
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0); // its own group: closing the app or a terminal never signals it
    cmd.spawn().map_err(|e| format!("Could not start Moss ({}): {e}", p.binary.display()))
}

/// Reuse a healthy controller from a previous launch, otherwise start one on a free port and wait for it.
pub fn attach_or_start(p: &Paths) -> Result<Handle, String> {
    if let Some(c) = state::read(&p.root) {
        if state::pid_alive(c.pid) && health::healthy(c.port) { return Ok(Handle { controller: c, child: None }); }
        state::clear(&p.root);
    }
    let mut last = String::new();
    for _ in 0..3 {
        let port = health::free_port().map_err(|e| format!("No free port on this computer: {e}"))?;
        let mut child = Some(spawn(p, port)?);
        let pid = child.as_ref().map_or(0, Child::id);
        match health::wait_ready(port, Duration::from_secs(30), || alive(&mut child, pid)) {
            Ok(()) => {
                let controller = Controller { pid, port };
                state::write(&p.root, controller).map_err(|e| format!("Could not save the controller record: {e}"))?;
                return Ok(Handle { controller, child });
            }
            Err(e) => { last = e; if let Some(mut c) = child { let _ = c.kill(); let _ = c.wait(); } }
        }
    }
    Err(last)
}

/// Ask the controller to stop (SIGTERM, which it handles as a graceful shutdown) and wait up to `grace`.
/// Game servers are separate processes and keep running. Returns whether the controller is gone.
pub fn stop(p: &Paths, h: &mut Handle, grace: Duration) -> bool {
    let pid = h.controller.pid;
    if pid > 1 && state::pid_alive(pid) {
        // SAFETY: plain signal to the one pid we recorded after seeing it answer on our port.
        unsafe { libc::kill(pid as i32, libc::SIGTERM) };
    }
    let end = std::time::Instant::now() + grace;
    while std::time::Instant::now() < end {
        if !alive(&mut h.child, pid) { state::clear(&p.root); return true; }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn paths(root: &Path, binary: PathBuf) -> Paths { Paths { root: root.into(), binary, shipped_env: None } }

    #[test]
    fn args_force_real_sign_in_and_loopback() {
        let a: Vec<String> = serve_args(5123, Path::new("/d")).iter().map(|s| s.to_string_lossy().into()).collect();
        assert_eq!(a, ["serve", "--bind", "127.0.0.1", "--port", "5123", "--auth", "supabase", "--data-dir", "/d"]);
    }

    #[test]
    fn attaches_to_a_recorded_healthy_controller() {
        let d = tempfile::tempdir().unwrap();
        let port = health::tests::stub("200 OK", "ok");
        state::write(d.path(), Controller { pid: std::process::id(), port }).unwrap();
        let h = attach_or_start(&paths(d.path(), "/nonexistent/moss".into())).unwrap();
        assert!(!h.started_here());
        assert_eq!(h.controller.port, port);
        assert_eq!(h.url(), format!("http://127.0.0.1:{port}/"));
    }

    #[test]
    fn a_stale_record_is_dropped_and_a_missing_binary_is_reported() {
        let d = tempfile::tempdir().unwrap();
        state::write(d.path(), Controller { pid: 0, port: 1 }).unwrap();
        let err = attach_or_start(&paths(d.path(), "/nonexistent/moss".into())).err().unwrap();
        assert!(err.contains("Could not start Moss"), "{err}");
        assert_eq!(state::read(d.path()), None);
    }

    /// A stand-in `moss` that exits at once: starting fails quickly and the log keeps its output.
    #[test]
    fn a_controller_that_exits_early_fails_fast_with_the_log_hint() {
        let d = tempfile::tempdir().unwrap();
        let bin = d.path().join("moss");
        std::fs::write(&bin, "#!/bin/sh\necho 'boom' >&2\nexit 3\n").unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let t = std::time::Instant::now();
        let err = attach_or_start(&paths(&d.path().join("root"), bin)).err().unwrap();
        assert!(err.contains("moss.log"), "{err}");
        assert!(t.elapsed() < Duration::from_secs(20));
        assert!(std::fs::read_to_string(d.path().join("root/moss.log")).unwrap().contains("boom"));
    }

    #[test]
    fn stop_terminates_a_detached_process_and_clears_the_record() {
        let d = tempfile::tempdir().unwrap();
        let mut child = Command::new("sleep").arg("60").spawn().unwrap();
        let pid = child.id();
        state::write(d.path(), Controller { pid, port: 1 }).unwrap();
        let p = paths(d.path(), "x".into());
        let mut h = Handle { controller: Controller { pid, port: 1 }, child: None };
        // reap in the background the way init would for a real detached controller
        let reaper = std::thread::spawn(move || { let _ = child.wait(); });
        assert!(stop(&p, &mut h, Duration::from_secs(5)));
        reaper.join().unwrap();
        assert_eq!(state::read(d.path()), None);
    }
}
