//! Server lifecycle. Game processes are launched detached (own session, stdin = a FIFO the game itself holds open,
//! stdout/stderr appended to a log file), so they are NOT children of the controller: killing or upgrading Moss never
//! stops a server, and on restart Moss re-attaches by PID + start-time and resumes tailing the log. Servers set to
//! container isolation run the same way inside a container instead and are re-attached by container name (`ctr`).
use crate::{platform, server::{self, Srv}, state::St};

mod command;
pub(crate) mod ctr;
mod exit;
mod monitor;
mod reconcile;
mod start;
mod stop;

pub use monitor::spawn_background;
pub use reconcile::reconcile;
pub use start::{send_command, start};
pub use stop::{halt, kill, restart, stop};

/// Detect the resource-limit backend once at startup (creates `moss-servers` in the delegated cgroup, if any).
pub fn init_limits() {
    match platform::cgroup::backend() {
        Ok(_) => tracing::info!("resource limits: cgroup v2"),
        Err(e) => tracing::info!("resource limits are not enforced: {e}"),
    }
}

/// Called once a server's processes are gone: reads whether the kernel OOM-killed it, then removes its cgroup.
pub(crate) fn finish_cgroup(id: &str) -> bool {
    let Ok(cg) = platform::cgroup::backend() else { return false };
    let oom = cg.oom_kills(id) > 0;
    cg.remove(id);
    oom
}

/// Remove a server's cgroup (used when the server record is deleted).
pub fn cleanup_cgroup(id: &str) { finish_cgroup(id); }

pub fn looks_ready(line: &str) -> bool { line.contains("Done (") && line.contains("s)") }

pub fn mark_ready(st: &St, id: &str) {
    let changed = {
        let mut live = st.live.lock().unwrap();
        let l = live.entry(id.to_string()).or_default();
        if l.ready || !l.alive { false } else { l.ready = true; l.detail = None; true }
    };
    if changed {
        let (st, id) = (st.clone(), id.to_string());
        tokio::spawn(async move {
            st.timeline(&id, "ready", "").await;
            if let Ok(s) = server::get(&st, &id).await { server::note_state(&st, &s); }
            crate::playerlists::apply(&st, &id).await;
        });
    }
}

/// Running, or a start is under way (its pid may not be saved yet). Callers re-read `s` after taking the server lock.
pub fn busy(st: &St, s: &Srv) -> bool {
    process_alive(s) || st.live.lock().unwrap().get(&s.id).is_some_and(|l| l.starting)
}

pub fn process_alive(s: &Srv) -> bool {
    if s.in_container { return crate::container::is_alive(&crate::container::container_name(&s.id)); }
    match (s.pid, s.pid_start) {
        (Some(p), Some(t)) => platform::pid_start(p) == Some(t as u64),
        _ => false,
    }
}

/// Open a file in a server's `.moss` directory without following a symlink, and require the opened file itself to be a
/// FIFO (`fifo`) or a regular file. The server directory is writable by the game (bind-mounted in container mode), so
/// it can swap these for links to host files. Always non-blocking, so a planted FIFO cannot stall the caller.
pub(crate) fn open_moss_file(path: &std::path::Path, opts: &mut std::fs::OpenOptions, fifo: bool) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::{FileTypeExt, OpenOptionsExt};
    let f = opts.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path)?;
    let t = f.metadata()?.file_type();
    if if fifo { t.is_fifo() } else { t.is_file() } { Ok(f) } else { Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "not the expected file type")) }
}

/// Stop or Kill pressed while a start was still under way (nothing to signal yet): `finish_start` acts on it once the
/// process is up. The value is true for Kill. Only touched while the `live` lock is held, so check and set are atomic.
static STOP_WHILE_STARTING: std::sync::Mutex<Option<std::collections::HashMap<String, bool>>> = std::sync::Mutex::new(None);

/// Under the `live` lock: when a start is in progress, remember the stop request and return true.
pub(crate) fn request_stop_if_starting(l: &crate::state::Live, id: &str, kill: bool) -> bool {
    if l.alive || !l.starting { return false; }
    let mut m = STOP_WHILE_STARTING.lock().unwrap();
    let e = m.get_or_insert_with(Default::default).entry(id.to_string()).or_insert(false);
    *e |= kill;
    true
}

/// Under the `live` lock: take a pending stop request (Some(true) = kill).
pub(crate) fn take_stop_request(id: &str) -> Option<bool> { STOP_WHILE_STARTING.lock().unwrap().as_mut()?.remove(id) }

pub(crate) fn set_detail(st: &St, id: &str, d: &str) { st.live.lock().unwrap().entry(id.to_string()).or_default().detail = Some(d.to_string()); }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn readiness_line_detection() {
        assert!(looks_ready("[22:30:01 INFO]: Done (4.412s)! For help, type \"help\""));
        assert!(looks_ready("[10:00:00] [Server thread/INFO]: Done (1.204s)! For help"));
        assert!(!looks_ready("Done loading plugins") && !looks_ready("[INFO]: Preparing level"));
    }

    #[test]
    fn stop_during_a_start_is_remembered_once() {
        let id = format!("stop-req-{}", uuid::Uuid::new_v4());
        let mut l = crate::state::Live::default();
        assert!(!request_stop_if_starting(&l, &id, false), "nothing in progress");
        l.starting = true;
        assert!(request_stop_if_starting(&l, &id, false));
        assert!(request_stop_if_starting(&l, &id, true));
        assert!(request_stop_if_starting(&l, &id, false));
        assert_eq!(take_stop_request(&id), Some(true), "a Kill among the requests wins");
        assert_eq!(take_stop_request(&id), None);
        l.alive = true;
        assert!(!request_stop_if_starting(&l, &id, true), "a running server is stopped directly");
    }

    #[test]
    fn moss_files_refuse_symlinks_and_wrong_types() {
        let d = std::env::temp_dir().join(format!("moss-mfile-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        let host = d.join("host.txt");
        std::fs::write(&host, "secret").unwrap();
        std::os::unix::fs::symlink(&host, d.join("console.fifo")).unwrap();
        std::os::unix::fs::symlink(&host, d.join("console.log")).unwrap();
        assert!(open_moss_file(&d.join("console.fifo"), std::fs::OpenOptions::new().write(true), true).is_err(), "symlinked fifo refused");
        assert!(open_moss_file(&d.join("console.log"), std::fs::OpenOptions::new().read(true), false).is_err(), "symlinked log refused");
        assert_eq!(std::fs::read_to_string(&host).unwrap(), "secret");
        let c = std::ffi::CString::new(d.join("pipe").to_string_lossy().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        assert!(open_moss_file(&d.join("pipe"), std::fs::OpenOptions::new().read(true), false).is_err(), "a FIFO is not a log");
        assert!(open_moss_file(&host, std::fs::OpenOptions::new().read(true), false).is_ok());
        let _ = std::fs::remove_dir_all(&d);
    }
}

