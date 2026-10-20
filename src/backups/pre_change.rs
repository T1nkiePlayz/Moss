//! Safety backups before risky changes. Every package that is about to change a server's files calls one of these
//! (content install/update, modpack import, version upgrade, config edits through Moss); in-place restore always does.
//! Pre-change backups are pruned to the newest `KEEP` per server (locked ones are never pruned).
use super::create::{do_backup, do_backup_locked};
use crate::{ops::OpCtx, server::Srv, state::St};
use anyhow::{Result, anyhow};

pub const KEEP: i64 = 3;

/// True when the setting asks for pre-change backups (default on).
async fn enabled(st: &St) -> bool { st.setting("backup_before_change", "true").await == "true" }

/// Whether the server has anything worth backing up.
async fn has_data(st: &St, s: &Srv) -> bool {
    let root = s.dir(st);
    s.installed && tokio::task::spawn_blocking(move || super::archive::dir_size(&root) > 0).await.unwrap_or(false)
}

async fn run(st: &St, s: &Srv, reason: &str, hold_lock: bool, force: bool, ctx: Option<&OpCtx>) -> Result<Option<String>> {
    if !force && !enabled(st).await { return Ok(None); }
    if !has_data(st, s).await { return Ok(None); }
    let detached;
    let ctx = match ctx { Some(c) => c, None => { detached = OpCtx::detached(st); &detached } };
    let note = format!("before {reason}");
    let r = if hold_lock { do_backup(ctx, st, s, "pre-change", &note, None).await } else { do_backup_locked(ctx, st, s, "pre-change", &note, None).await };
    r.map(Some).map_err(|e| anyhow!("could not take the safety backup before {reason}: {e:#}. The change was not made; turn off \"back up before changes\" in Settings to proceed without one."))
}

/// Take a `pre-change` backup if the `backup_before_change` setting is on, the server is installed and has data.
/// Returns the backup id, `None` when skipped, or an error whose message is meant for the user: callers should
/// abort their operation on `Err`. Takes the server's operation lock itself, so do not call it while holding that lock
/// (use `pre_change_locked`).
#[allow(dead_code)] // entry point for other packages
pub async fn pre_change(st: &St, s: &Srv, reason: &str) -> Result<Option<String>> { run(st, s, reason, true, false, None).await }

/// `pre_change` for callers that already hold `st.lock_for(server_id)`.
#[allow(dead_code)] // entry point for other packages
pub async fn pre_change_locked(st: &St, s: &Srv, reason: &str) -> Result<Option<String>> { run(st, s, reason, false, false, None).await }

/// A safety backup regardless of the setting (used by in-place restore), reporting progress on `ctx`.
pub async fn pre_change_forced(st: &St, s: &Srv, reason: &str, ctx: &OpCtx) -> Result<Option<String>> { run(st, s, reason, true, true, Some(ctx)).await }
