//! Promote-after-compare restores. `start` extracts a backup into a staging directory next to the servers (the live
//! server is not touched); the UI then shows the difference against the live files and the pre-promotion checks, and
//! only an explicit `promote` swaps the directories (the old one is kept under `removed/`) after a fresh safety backup.
use super::{create::archive_path, preview::compare_dir, remote::jobs::ensure_local, swap::{swap_full, swap_selected}};
use crate::{api::server_for, auth::{self, User}, error::{ApiError, ApiResult}, ops, server::{self, Srv}, state::St};
use anyhow::anyhow;
use axum::{Json, extract::{Path, State}, http::StatusCode};
use serde_json::{Value, json};

mod checks;

fn stage_dir(st: &St, id: &str) -> std::path::PathBuf { st.cfg.servers_dir().join(format!(".staged-{id}")) }

struct Row { id: String, backup_id: String, server_id: String, only: Option<Vec<String>>, status: String, error: Option<String>, created_at: String }
impl Row {
    fn json(&self) -> Value { json!({"id": self.id, "backup_id": self.backup_id, "server_id": self.server_id, "only": self.only, "status": self.status, "error": self.error, "created_at": self.created_at}) }
}
const SEL: &str = "SELECT id,backup_id,server_id,only,status,error,created_at FROM staged_restores";
fn row(r: &rusqlite::Row) -> rusqlite::Result<Row> {
    let only: Vec<String> = serde_json::from_str(&r.get::<_, String>(3)?).unwrap_or_default();
    Ok(Row { id: r.get(0)?, backup_id: r.get(1)?, server_id: r.get(2)?, only: (!only.is_empty()).then_some(only), status: r.get(4)?, error: r.get(5)?, created_at: r.get(6)? })
}

async fn load(st: &St, user: &User, id: &str) -> ApiResult<Row> {
    let i = id.to_string();
    let r = st.db.call(move |c| c.query_row(&format!("{SEL} WHERE id=?"), [i], row)).await.map_err(|_| ApiError::not_found("staged restore"))?;
    if server::get(st, &r.server_id).await.is_ok() { auth::authz(st, user, &r.server_id, "backup.manage").await?; } else { user.require_admin()?; }
    Ok(r)
}

async fn set_status(st: &St, id: &str, status: &str, err: Option<&str>) {
    let (i, s, e) = (id.to_string(), status.to_string(), err.map(|e| e.chars().take(400).collect::<String>()));
    let _ = st.db.call(move |c| c.execute("UPDATE staged_restores SET status=?, error=? WHERE id=?", rusqlite::params![s, e, i])).await;
}

async fn discard_dir(st: &St, id: &str) { let d = stage_dir(st, id); let _ = tokio::task::spawn_blocking(move || std::fs::remove_dir_all(d)).await; }

/// Begin a staged in-place restore of backup `bid` (the caller has validated mode, permissions and that the server is stopped).
pub async fn start(st: &St, user: &User, bid: &str, s: Srv, sha: String, only: Option<Vec<String>>) -> ApiResult<(StatusCode, Json<Value>)> {
    let sid = s.id.clone();
    let (s2, b2) = (sid.clone(), bid.to_string());
    let old: Vec<Row> = st.db.call(move |c| c.prepare(&format!("{SEL} WHERE server_id=?"))?.query_map([s2], row)?.collect()).await?;
    if old.iter().any(|o| o.status == "preparing") { return Err(ApiError::conflict("staged.busy", "a restore for this server is already being prepared")); }
    for o in &old { discard_dir(st, &o.id).await; let i = o.id.clone(); st.db.call(move |c| c.execute("DELETE FROM staged_restores WHERE id=?", [i])).await?; } // replaced by the new one
    let id = uuid::Uuid::now_v7().to_string();
    let (i2, only_json, now) = (id.clone(), serde_json::to_string(&only.clone().unwrap_or_default()).unwrap_or_default(), crate::db::now());
    st.db.call(move |c| c.execute("INSERT INTO staged_restores(id,backup_id,server_id,only,status,created_at) VALUES (?,?,?,?, 'preparing', ?)", rusqlite::params![i2, b2, sid, only_json, now])).await?;
    let (sid_id, actor) = (id.clone(), user.actor());
    let (bid, sid2) = (bid.to_string(), s.id.clone());
    let op = ops::spawn_op(st, "backup.stage", format!("Prepare restore of {}", s.name), Some(&sid2), true, move |ctx| async move {
        let st = ctx.st.clone();
        let r: anyhow::Result<()> = async {
            let p = ensure_local(&ctx, &st, &bid).await?;
            ctx.stage("Verifying archive before touching anything");
            let (p2, sha2) = (p.clone(), sha.clone());
            let (_, names) = tokio::task::spawn_blocking(move || super::archive::verify_archive_scan(&p2, &sha2)).await??;
            if let Some(missing) = only.iter().flatten().find(|n| !names.contains(*n)) { return Err(anyhow!("\"{missing}\" is not in this backup")); }
            ctx.check()?;
            let size = std::fs::metadata(archive_path(&st, &s.id, &bid)).map(|m| m.len()).unwrap_or(0);
            let free = crate::platform::disk(&st.cfg.data_dir).1;
            if free > 0 && free < size.saturating_mul(2) + (256 << 20) { return Err(anyhow!("not enough free disk space to stage this restore: need about {} MiB, have {} MiB", (size * 2 + (256 << 20)) >> 20, free >> 20)); }
            ctx.stage("Extracting to a staging directory (the live server is not touched)");
            let stage = stage_dir(&st, &sid_id);
            let (tok, only2) = (ctx.token.clone(), only.clone());
            let res = tokio::task::spawn_blocking(move || super::archive::extract_archive(&p, &stage, &tok, only2.as_deref())).await?;
            if let Err(e) = res { discard_dir(&st, &sid_id).await; return Err(e); }
            Ok(())
        }.await;
        match &r {
            Ok(()) => { set_status(&st, &sid_id, "ready", None).await; ctx.log("ready: review the changes, then promote"); st.audit(&actor, "backup.stage", &s.name, "restore staged", Some(&s.id)).await; }
            Err(e) => set_status(&st, &sid_id, "failed", Some(&if ctx.cancelled() { "cancelled".to_string() } else { format!("{e:#}") })).await,
        }
        r
    });
    Ok((StatusCode::ACCEPTED, Json(json!({"operation": op, "staged_id": id}))))
}

pub async fn list(State(st): State<St>, user: User, Path(sid): Path<String>) -> ApiResult<Json<Value>> {
    server_for(&st, &user, &sid, "backup.manage").await?;
    let v: Vec<Row> = st.db.call(move |c| c.prepare(&format!("{SEL} WHERE server_id=? ORDER BY created_at DESC"))?.query_map([sid], row)?.collect()).await?;
    Ok(Json(Value::Array(v.iter().map(Row::json).collect())))
}

async fn backup_worlds(st: &St, bid: &str) -> Vec<String> {
    let b = bid.to_string();
    st.db.call(move |c| c.query_row("SELECT worlds FROM backups WHERE id=?", [b], |r| r.get::<_, String>(0))).await.ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/// `GET /staged-restores/{id}`: the staged restore with its pre-promotion checks and the diff against the live server.
pub async fn get(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let r = load(&st, &user, &id).await?;
    let mut v = r.json();
    if r.status != "ready" { return Ok(Json(v)); }
    let s = server::get(&st, &r.server_id).await.map_err(|_| ApiError::conflict("server.gone", "the server no longer exists"))?;
    let (stage, live, worlds) = (stage_dir(&st, &id), s.dir(&st), backup_worlds(&st, &r.backup_id).await);
    let (s2, only) = (s.clone(), r.only.clone());
    let (checks, cmp) = tokio::task::spawn_blocking(move || (checks::run(&stage, &s2, only.as_deref(), &worlds), compare_dir(&stage, &live, only.as_deref()))).await.map_err(|e| anyhow!(e))?;
    let c = cmp.map_err(|e| ApiError::conflict("staged.unreadable", format!("could not compare the staged copy: {e:#}")))?;
    let mut warnings = vec!["A safety backup of the current state is taken first, and the replaced files are kept under data/removed.".to_string()];
    let running = crate::lifecycle::process_alive(&s);
    if running { warnings.push("The server is running; stop it before promoting.".into()); }
    v["can_promote"] = json!(checks.iter().all(|c| c.ok || !c.blocking) && !running);
    v["checks"] = Value::Array(checks.iter().map(checks::Check::json).collect());
    v["diff"] = json!({"archive": {"files": c.files, "size": c.size}, "current": {"files": c.cur_files, "size": c.cur_size}, "added": c.added, "removed": c.removed, "changed": c.changed, "unchanged": c.unchanged,
        "size_delta": c.size as i64 - c.cur_size as i64, "samples": {"added": c.added_s, "removed": c.removed_s, "changed": c.changed_s}});
    v["warnings"] = json!(warnings);
    Ok(Json(v))
}

pub async fn discard(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<StatusCode> {
    let r = load(&st, &user, &id).await?;
    if r.status == "preparing" { return Err(ApiError::conflict("staged.busy", "this restore is still being prepared; cancel its operation first")); }
    discard_dir(&st, &id).await;
    let i = id.clone();
    st.db.call(move |c| c.execute("DELETE FROM staged_restores WHERE id=?", [i])).await?;
    st.audit(&user.actor(), "backup.stage.discard", &r.backup_id, "", Some(&r.server_id)).await;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /staged-restores/{id}/promote`: swap the staged files in. The previous directory is kept under `removed/`.
pub async fn promote(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<(StatusCode, Json<Value>)> {
    let r = load(&st, &user, &id).await?;
    if r.status != "ready" { return Err(ApiError::conflict("staged.not_ready", "this restore is not ready to promote")); }
    let s = server::get(&st, &r.server_id).await.map_err(|_| ApiError::conflict("server.gone", "the original server no longer exists"))?;
    if crate::lifecycle::process_alive(&s) { return Err(ApiError::conflict("server.running", "stop the server before promoting")); }
    let (actor, worlds) = (user.actor(), backup_worlds(&st, &r.backup_id).await);
    let sid2 = s.id.clone();
    let op = ops::spawn_op(&st, "backup.promote", format!("Restore {}", s.name), Some(&sid2), false, move |ctx| async move {
        let st = ctx.st.clone();
        ctx.stage("Checking the staged copy");
        let stage = stage_dir(&st, &id);
        if !stage.is_dir() { return Err(anyhow!("the staged files are gone; prepare the restore again")); }
        let (stage2, s2, only, w2) = (stage.clone(), s.clone(), r.only.clone(), worlds);
        let cs = tokio::task::spawn_blocking(move || checks::run(&stage2, &s2, only.as_deref(), &w2)).await?;
        if let Some(c) = cs.iter().find(|c| !c.ok && c.blocking) { return Err(anyhow!("{} Nothing was changed.", c.message)); }
        ctx.stage("Taking a safety backup of the current state");
        super::pre_change_forced(&st, &s, "restore", &ctx).await?;
        let _g = st.lock_for(&s.id).try_lock_owned().map_err(|_| anyhow!("another operation is changing this server"))?;
        if crate::lifecycle::process_alive(&s) { return Err(anyhow!("the server was started in the meantime; stop it and promote again")); }
        ctx.stage("Swapping in the restored files (the previous ones are kept)");
        let (dir, kept) = (s.dir(&st), st.cfg.data_dir.join("removed").join(format!("{}-pre-restore-{}", s.id, chrono::Utc::now().format("%Y%m%d-%H%M%S"))));
        let (st2, d2, k2, only) = (stage.clone(), dir.clone(), kept.clone(), r.only.clone());
        tokio::task::spawn_blocking(move || match &only { Some(o) => { let r = swap_selected(&st2, &d2, &k2, o); let _ = std::fs::remove_dir_all(&st2); r } None => swap_full(&st2, &d2, &k2) }).await??;
        let i = id.clone();
        st.db.call(move |c| c.execute("DELETE FROM staged_restores WHERE id=?", [i])).await?;
        ctx.log(format!("previous contents kept at {}", kept.display()));
        st.audit(&actor, "backup.restore", &s.name, &r.only.as_ref().map_or("promoted after compare".to_string(), |o| format!("promoted after compare: {}", o.join(", "))), Some(&s.id)).await;
        Ok(())
    });
    Ok((StatusCode::ACCEPTED, Json(json!({"operation": op}))))
}
