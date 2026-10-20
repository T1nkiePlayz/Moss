//! Starting a server detached and talking to its console.
use super::command::build_command;
use crate::{error::{ApiError, ApiResult}, platform, server::{self, Srv}, state::St};
use std::{os::unix::fs::OpenOptionsExt, time::Instant};

// When MOSS_CGROUP is set the wrapper shell moves itself into that cgroup first, so the game and everything it forks
// start inside the limits (no window in which they run unconfined).
const RUN_SCRIPT: &str = r#"[ -n "$MOSS_CGROUP" ] && echo $$ > "$MOSS_CGROUP/cgroup.procs" 2>/dev/null; echo $$ > "$MOSS_DIR/pid"; "$@" <>"$MOSS_DIR/console.fifo" >>"$MOSS_DIR/console.log" 2>&1; echo $? > "$MOSS_DIR/exit""#;

pub async fn start(st: &St, s: &Srv, actor: &str) -> ApiResult<()> {
    {
        let mut live = st.live.lock().unwrap();
        let l = live.entry(s.id.clone()).or_default();
        if l.alive || l.starting { return Err(ApiError::conflict("server.running", "server is already running")); }
        l.starting = true;
    }
    let r = start_inner(st, s, actor).await;
    if let Some(l) = st.live.lock().unwrap().get_mut(&s.id) { l.starting = false; }
    r
}

async fn start_inner(st: &St, s: &Srv, actor: &str) -> ApiResult<()> {
    if !s.installed { return Err(ApiError::conflict("server.not_installed", "server is not installed yet")); }
    if s.is_mc() && s.eula_at.is_none() { return Err(ApiError::conflict("eula.required", "The Minecraft EULA must be accepted before this server can start.")); }
    if st.live.lock().unwrap().get(&s.id).is_some_and(|l| l.alive) { return Err(ApiError::conflict("server.running", "server is already running")); }
    if tokio::net::TcpListener::bind(("0.0.0.0", s.port as u16)).await.is_err() {
        return Err(ApiError::conflict("port.in_use", format!("port {} is already in use on this host", s.port)));
    }
    // Java discovery may run `java -version`, so keep it off the async worker threads.
    let argv = tokio::task::spawn_blocking({ let (st, s) = (st.clone(), s.clone()); move || build_command(&st, &s) }).await.map_err(anyhow::Error::from)??;
    let (dir, mdir) = (s.dir(st), s.moss_dir(st));
    std::fs::create_dir_all(&mdir)?;
    let fifo = mdir.join("console.fifo");
    if !fifo.exists() {
        let c = std::ffi::CString::new(fifo.to_string_lossy().as_bytes()).unwrap();
        if unsafe { libc::mkfifo(c.as_ptr(), 0o600) } != 0 { return Err(std::io::Error::last_os_error().into()); }
    }
    let _ = std::fs::remove_file(mdir.join("exit"));
    let log = mdir.join("console.log");
    if std::fs::metadata(&log).map(|m| m.len() > 0).unwrap_or(false) { let _ = std::fs::rename(&log, mdir.join("console.log.1")); }
    std::fs::write(&log, format!("[moss] starting {} ({} {})\n", s.name, s.runtime, s.game_version))?;

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
        .env("PATH", platform::GAME_PATH).env("HOME", &dir).env("LANG", platform::GAME_LANG).env("MOSS_DIR", mdir)
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
    {
        let mut live = st.live.lock().unwrap();
        let l = live.entry(s.id.clone()).or_default();
        l.alive = true; l.ready = false; l.stopping = false; l.crashed = false; l.detail = None;
        l.started = Some(Instant::now()); l.restart_at = None; l.players = None; l.cpu_prev = None;
    }
    server::set_desired(st, &s.id, "running").await;
    st.timeline(&s.id, "start", actor).await;
    st.audit(actor, "server.start", &s.name, "", Some(&s.id)).await;
    crate::console::start_tailer(st, s.id.clone(), log, false);
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
        let mut f = std::fs::OpenOptions::new().write(true).custom_flags(libc::O_NONBLOCK).open(fifo)?;
        f.write_all(line.as_bytes())
    }).await.map_err(|e| anyhow::anyhow!(e))?.map_err(|_| ApiError::conflict("server.not_running", "console is not accepting input"))?;
    crate::console::push(&crate::console::hub(st, &s.id), format!("\x1b[36m> {cmd}\x1b[0m"));
    Ok(())
}
