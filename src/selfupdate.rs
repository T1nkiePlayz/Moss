//! Updating Moss itself from the panel: a background check for new releases (announced as a `moss_update`
//! notification and an `update` event), and the `moss.update` operation that installs one and restarts the
//! controller. Game servers are detached and keep running across the restart.
//!
//! How an update is installed depends on who may write the binary (see `cli::update::method`):
//! - self: Moss replaces its own binary (same checks as `moss update`), then re-executes itself in place, so the PID
//!   stays the same under systemd and launchd;
//! - helper: under the hardened systemd unit Moss cannot write /usr/local/bin, so it leaves a request file that the
//!   root `moss-update.path` unit picks up; that runs `moss update --request … --restart-service moss.service`;
//! - manual: a package manager owns the binary, or nothing above applies; the panel shows the command to run.
use crate::cli::update::{self as cli, Method};
use crate::error::{ApiError, ApiResult};
use crate::ops::{self, Operation};
use crate::state::St;
use serde::Serialize;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const CURRENT: &str = env!("CARGO_PKG_VERSION");
/// Meta key: the release already announced, so a restart does not announce it again.
const NOTIFIED: &str = "moss_update_notified";
pub const REQUEST_FILE: &str = "update-request";

/// The executable and arguments this controller started with. Captured at startup: on Linux, once the binary has been
/// replaced, `current_exe()` names the deleted file.
static LAUNCH: std::sync::OnceLock<(PathBuf, Vec<std::ffi::OsString>)> = std::sync::OnceLock::new();
/// The latest release seen and when (RFC 3339).
static LATEST: Mutex<Option<(String, String)>> = Mutex::new(None);
static RUNNING: AtomicBool = AtomicBool::new(false);
#[cfg(test)]
static TEST_METHOD: Mutex<Option<Method>> = Mutex::new(None);

/// Children this process already had when it started. Only an exec restart leaves any: the game-server wrappers
/// the previous image launched, which nothing in this image waits for. Collected before Moss spawns anything, so
/// reaping them never takes a child that tokio is waiting on.
static INHERITED: Mutex<Vec<i32>> = Mutex::new(Vec::new());

pub fn capture_launch() {
    *INHERITED.lock().unwrap() = inherited_children();
    let exe = std::env::current_exe().ok().map(|e| e.canonicalize().unwrap_or(e));
    if let Some(exe) = exe { let _ = LAUNCH.set((exe, std::env::args_os().skip(1).collect())); }
}

fn exe() -> Option<PathBuf> {
    LAUNCH.get().map(|l| l.0.clone()).or_else(|| std::env::current_exe().ok().map(|e| e.canonicalize().unwrap_or(e)))
}

fn method(st: &St) -> Method {
    #[cfg(test)]
    if let Some(m) = TEST_METHOD.lock().unwrap().clone() { return m; }
    match exe() { Some(e) => cli::method(&e, &st.cfg.data_dir), None => Method::Manual { reason: "Moss cannot find its own executable.".into(), command: String::new() } }
}

#[derive(Debug, Clone, Serialize)]
pub struct Info {
    pub current: String,
    pub latest: Option<String>,
    pub available: bool,
    pub checked_at: Option<String>,
    pub method: &'static str,
    pub reason: Option<String>,
    pub command: Option<String>,
    pub release_url: Option<String>,
    pub running: bool,
}

pub fn info(st: &St) -> Info {
    let latest = LATEST.lock().unwrap().clone();
    let (method, reason, command) = match method(st) {
        Method::SelfUpdate => ("self", None, None),
        Method::Helper => ("helper", None, None),
        Method::Manual { reason, command } => ("manual", Some(reason), Some(command).filter(|c| !c.is_empty())),
    };
    let repo = cli::server_args(None).repo;
    Info {
        current: CURRENT.into(),
        available: latest.as_ref().is_some_and(|(t, _)| cli::is_newer(t, CURRENT)),
        release_url: latest.as_ref().map(|(t, _)| format!("https://github.com/{repo}/releases/tag/{t}")),
        checked_at: latest.as_ref().map(|(_, at)| at.clone()),
        latest: latest.map(|(t, _)| t),
        method, reason, command,
        running: RUNNING.load(Ordering::SeqCst),
    }
}

/// Ask the release source for the newest version, remember it and announce it if it is new.
pub async fn check(st: &St) -> anyhow::Result<Info> {
    let tag = cli::latest(&cli::server_args(None)).await?;
    *LATEST.lock().unwrap() = Some((tag, crate::db::now()));
    let i = info(st);
    st.emit("update", json!(i));
    if i.available { announce(st, &i).await; }
    Ok(i)
}

async fn announce(st: &St, i: &Info) {
    let Some(latest) = i.latest.as_deref() else { return };
    if st.setting(NOTIFIED, "").await == latest { return; }
    let how = match i.method { "manual" => "See Settings → Updates for how to install it.", _ => "Install it from Settings → Updates; Moss restarts afterwards and game servers keep running." };
    crate::notify::send(st, "moss_update", "info", &format!("Moss {latest} is available"), &format!("This controller runs v{CURRENT}. {how}"), None).await;
    if let Err(e) = st.set_setting(NOTIFIED, latest).await { tracing::warn!("could not record the update notification: {e:#}"); }
}

#[cfg(target_os = "linux")]
fn inherited_children() -> Vec<i32> {
    let me = std::process::id().to_string();
    let Ok(dir) = std::fs::read_dir("/proc") else { return vec![] };
    dir.flatten().filter_map(|e| e.file_name().to_str()?.parse::<i32>().ok()).filter(|pid| {
        // the parent pid is the second field after the parenthesised command name
        std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()
            .and_then(|s| s.rfind(')').map(|i| s[i + 1..].split_whitespace().nth(1) == Some(me.as_str())))
            .unwrap_or(false)
    }).collect()
}
#[cfg(not(target_os = "linux"))]
fn inherited_children() -> Vec<i32> { vec![] }

/// Reap the inherited children as they exit (see INHERITED).
fn spawn_reaper() {
    if INHERITED.lock().unwrap().is_empty() { return; }
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            let mut left = INHERITED.lock().unwrap();
            left.retain(|&pid| unsafe { libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG) } == 0);
            if left.is_empty() { return; }
        }
    });
}

/// Check a minute after start, then every six hours. Off with MOSS_UPDATE_CHECK=false, and in dev mode.
pub fn spawn_background(st: &St) {
    spawn_reaper();
    let off = std::env::var("MOSS_UPDATE_CHECK").is_ok_and(|v| matches!(v.to_ascii_lowercase().as_str(), "0" | "false" | "no" | "off"));
    if off || st.cfg.dev() { return; }
    let st = st.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(60)).await;
        loop {
            if let Err(e) = check(&st).await { tracing::debug!("update check failed: {e:#}"); }
            tokio::time::sleep(Duration::from_secs(6 * 3600)).await;
        }
    });
}

/// Clears RUNNING unless the update ends in a restart.
struct Running(bool);
impl Drop for Running { fn drop(&mut self) { if !self.0 { RUNNING.store(false, Ordering::SeqCst); } } }

/// Start installing `version` (or the latest release). 409 when there is nothing newer or the panel cannot do it.
pub async fn begin(st: &St, actor: &str, version: Option<&str>) -> ApiResult<Operation> {
    let pinned = version.map(cli::valid_tag).transpose().map_err(|e| ApiError::bad("validation", "not a release version").field("version", &format!("{e:#}")))?;
    let m = method(st);
    if let Method::Manual { reason, .. } = &m { return Err(ApiError::conflict("update.unavailable", reason)); }
    let tag = match pinned {
        Some(t) => t,
        None => {
            let i = check(st).await.map_err(|e| ApiError::new(axum::http::StatusCode::BAD_GATEWAY, "update.lookup_failed", format!("could not look up the latest release: {e:#}")))?;
            i.latest.filter(|_| i.available).ok_or_else(|| ApiError::conflict("update.none", "Moss is up to date"))?
        }
    };
    if !cli::is_newer(&tag, CURRENT) { return Err(ApiError::conflict("update.none", format!("v{CURRENT} is installed; the panel only installs newer releases (use `moss update --version` to go back)"))); }
    if RUNNING.swap(true, Ordering::SeqCst) { return Err(ApiError::conflict("update.running", "an update is already running")); }
    let guard = Running(false);
    st.audit(actor, "moss.update", &tag, &format!("v{CURRENT} -> {tag}"), None).await;
    Ok(ops::spawn_op(st, "moss.update", format!("Update Moss to {tag}"), None, false, move |ctx| async move {
        let mut guard = guard;
        match m {
            Method::Helper => {
                ctx.stage("Handing the update to the moss-update helper");
                write_request(&ctx.st.cfg.data_dir, &tag)?;
                ctx.log(format!("The helper installs {tag} as root and then restarts Moss. If Moss is still on v{CURRENT} in a few minutes, check `journalctl -u moss-update`."));
            }
            _ => {
                let exe = exe().ok_or_else(|| anyhow::anyhow!("cannot find this executable"))?;
                ctx.stage(&format!("Downloading and verifying {tag}"));
                let say = { let ctx = ctx.clone(); move |l: String| ctx.log(l) };
                let a = cli::server_args(Some(tag.clone()));
                match cli::install(&a, &exe, &say).await? {
                    Some(t) => {
                        ctx.stage(&format!("Installed {t}; restarting Moss"));
                        guard.0 = true;
                        let st = ctx.st.clone();
                        tokio::spawn(async move {
                            // let the completed operation reach the database and the panel first
                            tokio::time::sleep(Duration::from_millis(1500)).await;
                            restart(&st).await;
                        });
                    }
                    None => ctx.log(format!("{tag} is already installed.")),
                }
            }
        }
        Ok(())
    }))
}

/// Leave the version for the root helper: written to a temporary name and renamed, so it never sees a partial file.
fn write_request(data_dir: &Path, tag: &str) -> anyhow::Result<()> {
    use std::io::Write;
    let tmp = data_dir.join(format!(".{REQUEST_FILE}.{}", std::process::id()));
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
    let res = f.write_all(format!("{tag}\n").as_bytes()).and_then(|_| f.sync_all()).and_then(|_| std::fs::rename(&tmp, data_dir.join(REQUEST_FILE)));
    if res.is_err() { let _ = std::fs::remove_file(&tmp); }
    Ok(res?)
}

/// Replace this process with the new binary: same PID, same arguments and environment. Open files are close-on-exec,
/// so the data-directory lock and the listening socket are released for the new process. Returns only on failure.
async fn restart(st: &St) {
    use std::os::unix::process::CommandExt;
    crate::history::flush(st, true).await;
    let Some((exe, args)) = LAUNCH.get() else { RUNNING.store(false, Ordering::SeqCst); tracing::error!("cannot restart: the launch command was not recorded; restart Moss by hand"); return; };
    tracing::info!("restarting into the updated binary {}", exe.display());
    let err = std::process::Command::new(exe).args(args).exec();
    RUNNING.store(false, Ordering::SeqCst);
    tracing::error!("could not restart into the updated binary ({err}); restart Moss by hand to finish the update");
    crate::notify::send(st, "operation_failed", "warn", "Moss was updated but could not restart", &format!("Restart the controller by hand to use the new version ({err})."), None).await;
}

#[cfg(test)]
pub(crate) fn set_test_state(latest: Option<&str>, m: Option<Method>) {
    *LATEST.lock().unwrap() = latest.map(|t| (t.to_string(), crate::db::now()));
    *TEST_METHOD.lock().unwrap() = m;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_are_written_whole() {
        let d = std::env::temp_dir().join(format!("moss-selfupdate-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        write_request(&d, "v9.9.9").unwrap();
        assert_eq!(std::fs::read_to_string(d.join(REQUEST_FILE)).unwrap(), "v9.9.9\n");
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1, "no temporary file left behind");
        let _ = std::fs::remove_dir_all(&d);
    }
}
