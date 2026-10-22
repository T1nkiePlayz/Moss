//! Starting a server detached and talking to its console.
use super::command::build_command;
use crate::{error::{ApiError, ApiResult}, platform, server::{self, Srv}, state::St};
use std::time::Instant;

// When MOSS_CGROUP is set the wrapper shell moves itself into that cgroup first, so the game and everything it forks
// start inside the limits (no window in which they run unconfined).
const RUN_SCRIPT: &str = r#"[ -n "$MOSS_CGROUP" ] && echo $$ > "$MOSS_CGROUP/cgroup.procs" 2>/dev/null; echo $$ > "$MOSS_DIR/pid"; "$@" <>"$MOSS_DIR/console.fifo" >>"$MOSS_DIR/console.log" 2>&1; echo $? > "$MOSS_DIR/exit""#;

pub async fn start(st: &St, s: &Srv, actor: &str) -> ApiResult<()> {
    {
        let mut live = st.live.lock().unwrap();
        let l = live.entry(s.id.clone()).or_default();
        if l.alive || l.starting { return Err(ApiError::conflict("server.running", "server is already running")); }
        l.starting = true;
        super::take_stop_request(&s.id); // a leftover from a start that failed
    }
    let r = start_inner(st, s, actor).await;
    if let Some(l) = st.live.lock().unwrap().get_mut(&s.id) { l.starting = false; }
    r
}

/// Java Properties semantics: `#`/`!` comment lines are ignored and the last `eula=` value wins.
pub(crate) fn eula_txt_accepted(text: &str) -> bool {
    text.lines().map(str::trim).filter(|l| !l.starts_with('#') && !l.starts_with('!'))
        .filter_map(|l| l.split_once('=')).rfind(|(k, _)| k.trim().eq_ignore_ascii_case("eula"))
        .is_some_and(|(_, v)| v.trim().eq_ignore_ascii_case("true"))
}

/// Read eula.txt only if it is a regular file (not a link, FIFO or device) of sane size; blocking work, so call from spawn_blocking.
pub(crate) fn read_eula_txt(dir: &std::path::Path) -> Option<String> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    // O_NOFOLLOW + O_NONBLOCK, then check the opened file itself, so a swap after any earlier check cannot block or redirect.
    let f = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(dir.join("eula.txt")).ok()?;
    if !f.metadata().ok()?.file_type().is_file() { return None; }
    let mut s = String::new();
    f.take(64 << 10).read_to_string(&mut s).ok()?;
    Some(s)
}

/// A Minecraft Java server whose owner set `eula=true` in eula.txt by hand counts as accepted: record it (once) and allow the start.
async fn accept_from_eula_txt(st: &St, s: &Srv, actor: &str) -> ApiResult<bool> {
    if !s.is_mc() || s.is_proxy() { return Ok(false); }
    let dir = s.dir(st);
    let text = tokio::task::spawn_blocking(move || read_eula_txt(&dir)).await.map_err(anyhow::Error::from)?;
    if !text.is_some_and(|t| eula_txt_accepted(&t)) { return Ok(false); }
    let (i, now) = (s.id.clone(), crate::db::now());
    let changed = st.db.call(move |c| c.execute("UPDATE servers SET eula_at=?, eula_by='eula.txt' WHERE id=? AND eula_at IS NULL", rusqlite::params![now, i])).await?;
    if changed > 0 { st.audit(actor, "eula.accept", &s.name, "Minecraft EULA accepted via eula.txt", Some(&s.id)).await; }
    Ok(true)
}

async fn start_inner(st: &St, s: &Srv, actor: &str) -> ApiResult<()> {
    if !s.installed { return Err(ApiError::conflict("server.not_installed", "server is not installed yet")); }
    if crate::health::eula_missing(s) && !accept_from_eula_txt(st, s, actor).await? { return Err(ApiError::conflict("eula.required", "The Minecraft EULA must be accepted before this server can start.")); }
    if st.live.lock().unwrap().get(&s.id).is_some_and(|l| l.alive) { return Err(ApiError::conflict("server.running", "server is already running")); }
    if tokio::net::TcpListener::bind(("0.0.0.0", s.port as u16)).await.is_err() {
        return Err(ApiError::conflict("port.in_use", format!("port {} is already in use on this host", s.port)));
    }
    crate::playerlists::apply_before_start(st, &s.id).await; // shared allow/ban lists into the files the game reads at start
    // Java discovery may run `java -version`, so keep it off the async worker threads.
    let argv = tokio::task::spawn_blocking({ let (st, s) = (st.clone(), s.clone()); move || build_command(&st, &s) }).await.map_err(anyhow::Error::from)??;
    let (dir, mdir) = (s.dir(st), s.moss_dir(st));
    tokio::fs::create_dir_all(&mdir).await?;
    let fifo = mdir.join("console.fifo");
    // lstat: the game can write `.moss`, so anything but a real FIFO here (a link to a host file, say) is replaced.
    let is_fifo = tokio::fs::symlink_metadata(&fifo).await.map(|m| { use std::os::unix::fs::FileTypeExt; m.file_type().is_fifo() });
    if is_fifo.as_ref().is_ok_and(|f| !f) { let _ = tokio::fs::remove_file(&fifo).await; }
    if !is_fifo.unwrap_or(false) {
        let c = std::ffi::CString::new(fifo.to_string_lossy().as_bytes()).unwrap();
        if unsafe { libc::mkfifo(c.as_ptr(), 0o600) } != 0 { return Err(std::io::Error::last_os_error().into()); }
    }
    let _ = tokio::fs::remove_file(mdir.join("exit")).await;
    let log = mdir.join("console.log");
    if tokio::fs::symlink_metadata(&log).await.is_ok_and(|m| m.is_file() && m.len() > 0) { let _ = tokio::fs::rename(&log, mdir.join("console.log.1")).await; }
    // Whatever is left (an empty log, or a link or FIFO the game planted) goes; create_new never follows a link.
    let _ = tokio::fs::remove_file(&log).await;
    {
        use tokio::io::AsyncWriteExt;
        let mut f = tokio::fs::OpenOptions::new().write(true).create_new(true).open(&log).await?;
        f.write_all(format!("[moss] starting {} ({} {})\n", s.name, s.runtime, s.game_version).as_bytes()).await?;
    }

    let ctr = s.isolation == "container";
    let (pid, pstart) = if ctr {
        super::ctr::spawn(st, s, argv).await.inspect_err(|e| crate::console::push(&crate::console::hub(st, &s.id), format!("\x1b[31m[moss] {}\x1b[0m", e.message)))?
    } else {
        spawn_process(st, s, &argv, &dir, &mdir)?
    };
    server::save_pid(st, &s.id, Some(pid), Some(pstart), None).await;
    if !ctr { server::set_in_container(st, &s.id, false).await; }
    finish_start(st, s, actor, log).await
}

/// Launch the detached wrapper shell (process mode); returns the PID and its start time.
fn spawn_process(st: &St, s: &Srv, argv: &[String], dir: &std::path::Path, mdir: &std::path::Path) -> ApiResult<(i64, i64)> {
    let cg_dir = limit_cgroup(st, s);
    let mut cmd = tokio::process::Command::new("/bin/sh");
    cmd.arg("-c").arg(RUN_SCRIPT).arg("moss-run").args(argv).current_dir(dir).env_clear()
        .env("PATH", platform::GAME_PATH).env("HOME", dir).env("LANG", platform::GAME_LANG).env("MOSS_DIR", mdir)
        .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    if let Some(d) = &cg_dir { cmd.env("MOSS_CGROUP", d); }
    unsafe { cmd.pre_exec(|| { libc::setsid(); Ok(()) }); }
    let mut child = cmd.spawn().map_err(|e| ApiError::conflict("spawn.failed", format!("could not launch: {e}")))?;
    let pid = child.id().unwrap_or(0) as i64;
    let pstart = platform::pid_start(pid).unwrap_or(0) as i64;
    tokio::spawn(async move { let _ = child.wait().await; }); // reap while the controller lives
    Ok((pid, pstart))
}

async fn finish_start(st: &St, s: &Srv, actor: &str, log: std::path::PathBuf) -> ApiResult<()> {
    let (stop_req, stop_gen) = {
        let mut live = st.live.lock().unwrap();
        let l = live.entry(s.id.clone()).or_default();
        l.alive = true; l.ready = false; l.stopping = false; l.crashed = false; l.detail = None;
        l.started = Some(Instant::now()); l.restart_at = None; l.players = None; l.cpu_prev = None;
        // Taken under the same lock in which Stop/Kill saw `starting`, so a request is either seen here or Stop saw `alive`.
        let r = super::take_stop_request(&s.id);
        if r.is_some() { l.stopping = true; }
        (r, l.stop_gen)
    };
    if stop_req.is_none() {
        server::set_desired(st, &s.id, "running").await;
        // a Stop that saw `alive` between the lock above and this write must still win
        if st.live.lock().unwrap().get(&s.id).is_some_and(|l| l.stop_gen != stop_gen) { server::set_desired(st, &s.id, "stopped").await; }
    }
    st.timeline(&s.id, "start", actor).await;
    st.audit(actor, "server.start", &s.name, "", Some(&s.id)).await;
    crate::console::start_tailer(st, s.id.clone(), log, false);
    if let Some(kill) = stop_req {
        // Stop or Kill was pressed while this start was under way: that wish wins (desired is already "stopped").
        let fresh = server::get(st, &s.id).await?;
        super::stop::abort_started(st, &fresh, kill).await;
        return Ok(());
    }
    let fresh = server::get(st, &s.id).await?;
    server::note_state(st, &fresh);
    Ok(())
}

/// Create the server's cgroup with its limits (default memory cap for Java servers). None when limits cannot be enforced.
fn limit_cgroup(st: &St, s: &Srv) -> Option<std::path::PathBuf> {
    let cg = platform::cgroup::backend().ok()?;
    let req = s.requested_limits();
    let mem = req.memory_mb.map(|m| (m.max(0) as u64) << 20).or_else(|| s.is_mc().then(|| platform::cgroup::default_memory_cap(s.memory_mb)));
    cg.remove(&s.id); // a leftover leaf from an unclean exit
    match cg.apply(&s.id, mem, &req) {
        Ok(d) => Some(d),
        Err(e) => {
            crate::console::push(&crate::console::hub(st, &s.id), format!("\x1b[33m[moss] resource limits could not be applied: {e}\x1b[0m"));
            None
        }
    }
}

pub async fn send_command(st: &St, s: &Srv, cmd: &str) -> ApiResult<()> {
    let cmd = cmd.trim();
    if cmd.is_empty() || cmd.len() > 1000 || cmd.contains(['\n', '\r', '\0']) { return Err(ApiError::bad("command.invalid", "command must be a single non-empty line")); }
    if !st.live.lock().unwrap().get(&s.id).is_some_and(|l| l.alive) { return Err(ApiError::conflict("server.not_running", "server is not running")); }
    let fifo = s.moss_dir(st).join("console.fifo");
    let line = format!("{cmd}\n");
    tokio::task::spawn_blocking(move || {
        use std::io::Write;
        let mut f = super::open_moss_file(&fifo, std::fs::OpenOptions::new().write(true), true)?;
        f.write_all(line.as_bytes())
    }).await.map_err(|e| anyhow::anyhow!(e))?.map_err(|e| if e.kind() == std::io::ErrorKind::WouldBlock { ApiError::new(axum::http::StatusCode::TOO_MANY_REQUESTS, "console.busy", "the server is not reading its console fast enough; try again in a moment") } else { ApiError::conflict("server.not_running", "console is not accepting input") })?;
    crate::console::push(&crate::console::hub(st, &s.id), format!("\x1b[36m> {cmd}\x1b[0m"));
    Ok(())
}

#[cfg(test)]
mod eula_tests {
    use super::eula_txt_accepted;
    #[test]
    fn eula_txt_needs_an_uncommented_true() {
        assert!(eula_txt_accepted("# by Mojang\neula=true\n"));
        assert!(eula_txt_accepted("eula = TRUE"));
        assert!(!eula_txt_accepted("#eula=true\n"));
        assert!(!eula_txt_accepted("# x\neula=false\n"));
        assert!(!eula_txt_accepted(""));
        assert!(!eula_txt_accepted("eula=true\neula=false\n"), "last value wins");
        assert!(eula_txt_accepted("eula=false\n! c\neula=true\n"));
        assert!(!eula_txt_accepted("!eula=true\n"));
    }

    #[test]
    fn eula_txt_must_be_a_regular_file() {
        let d = std::env::temp_dir().join(format!("moss-eula-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        assert!(super::read_eula_txt(&d).is_none());
        std::fs::write(d.join("real.txt"), "eula=true\n").unwrap();
        std::os::unix::fs::symlink(d.join("real.txt"), d.join("eula.txt")).unwrap();
        assert!(super::read_eula_txt(&d).is_none(), "symlink refused");
        std::fs::remove_file(d.join("eula.txt")).unwrap();
        let c = std::ffi::CString::new(d.join("eula.txt").to_string_lossy().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        assert!(super::read_eula_txt(&d).is_none(), "fifo refused without blocking");
        std::fs::remove_file(d.join("eula.txt")).unwrap();
        std::fs::write(d.join("eula.txt"), "eula=true\n").unwrap();
        assert_eq!(super::read_eula_txt(&d).as_deref(), Some("eula=true\n"));
        let _ = std::fs::remove_dir_all(&d);
    }
}
