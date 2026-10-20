//! Per-backup remote actions: copy (or retry copying) to the destination, and bring a remote-only backup back to disk.
use super::jobs;
use crate::{auth::User, backups::find, error::{ApiError, ApiResult}, ops, server, state::St};
use axum::{Json, extract::{Path, State}, http::StatusCode};
use serde_json::{Value, json};

/// `POST /backups/{id}/upload`: copy a local backup to the server's destination (also the retry after a failure).
pub async fn upload(State(st): State<St>, user: User, Path(bid): Path<String>) -> ApiResult<(StatusCode, Json<Value>)> {
    let (sid, ..) = find(&st, &user, &bid, "backup.manage").await?;
    let r = jobs::row(&st, &bid).await?;
    let status: String = { let b = bid.clone(); st.db.call(move |c| c.query_row("SELECT status FROM backups WHERE id=?", [b], |r| r.get(0))).await? };
    if status != "ok" { return Err(ApiError::conflict("backup.unusable", "this backup did not complete")); }
    if r.remote == "uploading" { return Err(ApiError::conflict("backup.uploading", "this backup is already being copied")); }
    if r.remote == "stored" { return Err(ApiError::conflict("backup.stored", "this backup already has a verified remote copy")); }
    if !r.local { return Err(ApiError::conflict("backup.no_local", "the local file is gone, so there is nothing to copy")); }
    let s = server::get(&st, &sid).await.map_err(|_| ApiError::conflict("server.gone", "the original server no longer exists"))?;
    match jobs::queue_upload(&st, &s, &bid).await? {
        Some(op) => Ok((StatusCode::ACCEPTED, Json(json!({"operation": op})))),
        None => Err(ApiError::conflict("backup.no_destination", "this server has no backup destination; add one in Settings, Backups")),
    }
}

/// `POST /backups/{id}/fetch`: download the remote copy and verify its checksum.
pub async fn fetch(State(st): State<St>, user: User, Path(bid): Path<String>) -> ApiResult<(StatusCode, Json<Value>)> {
    let (sid, ..) = find(&st, &user, &bid, "backup.manage").await?;
    let r = jobs::row(&st, &bid).await?;
    if r.local { return Err(ApiError::conflict("backup.local_present", "this backup is already on this machine")); }
    if r.remote != "stored" { return Err(ApiError::conflict("backup.no_remote", "this backup has no remote copy")); }
    let op = ops::spawn_op(&st, "backup.fetch", "Download backup from remote", Some(&sid), true, move |ctx| async move {
        let st = ctx.st.clone();
        jobs::ensure_local(&ctx, &st, &bid).await.map(|_| ())
    });
    Ok((StatusCode::ACCEPTED, Json(json!({"operation": op}))))
}
