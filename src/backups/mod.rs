//! Backups API: list/create/verify/delete/download handlers over the `backups` table. Online backups use
//! save-off / save-all flush / save-on. Archive format is in `archive`, creation and pruning in `create`, restores in `restore`.
use crate::{api::server_for, auth::{self, User}, error::{ApiError, ApiResult}, ops, state::St};
use axum::{Json, extract::{Path, State}, http::{HeaderValue, StatusCode, header}, response::{IntoResponse, Response}};
use serde::Deserialize;
use serde_json::{Value, json};

mod archive;
mod create;
mod restore;

pub use create::do_backup;
pub use restore::restore;
use archive::verify_archive;
use create::{archive_path, remove_backup};

fn row_json(r: &rusqlite::Row) -> rusqlite::Result<Value> {
    Ok(json!({"id": r.get::<_, String>(0)?, "server_id": r.get::<_, String>(1)?, "created_at": r.get::<_, String>(2)?, "size": r.get::<_, i64>(3)?, "sha256": r.get::<_, String>(4)?,
        "status": r.get::<_, String>(5)?, "verified_at": r.get::<_, Option<String>>(6)?, "note": r.get::<_, String>(7)?, "kind": r.get::<_, String>(8)?,
        "consistent": r.get::<_, bool>(9)?, "files": r.get::<_, i64>(10)?, "server_name": r.get::<_, String>(11)?}))
}
const SEL: &str = "SELECT id,server_id,created_at,size,sha256,status,verified_at,note,kind,consistent,files,server_name FROM backups";

// ---------- API ----------
pub async fn list(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    server_for(&st, &user, &id, "server.view").await?;
    let v: Vec<Value> = st.db.call(move |c| { let mut q = c.prepare(&format!("{SEL} WHERE server_id=? ORDER BY created_at DESC"))?; q.query_map([id], row_json)?.collect() }).await?;
    Ok(Json(json!(v)))
}

pub async fn list_all(State(st): State<St>, user: User) -> ApiResult<Json<Value>> {
    let vis = auth::visible_servers(&st, &user).await?;
    let v: Vec<Value> = st.db.call(|c| { let mut q = c.prepare(&format!("{SEL} ORDER BY created_at DESC LIMIT 500"))?; q.query_map([], row_json)?.collect() }).await?;
    Ok(Json(Value::Array(v.into_iter().filter(|b| vis.as_ref().is_none_or(|v| v.iter().any(|s| b["server_id"] == s.as_str()))).collect())))
}

#[derive(Deserialize, Default)] pub struct CreateReq { note: Option<String> }
pub async fn create(State(st): State<St>, user: User, Path(id): Path<String>, body: Option<Json<CreateReq>>) -> ApiResult<(StatusCode, Json<Value>)> {
    let s = server_for(&st, &user, &id, "backup.manage").await?;
    if !s.installed { return Err(ApiError::conflict("server.not_installed", "server is not installed yet")); }
    let note = body.and_then(|b| b.0.note).unwrap_or_default().chars().take(200).collect::<String>();
    let actor = user.email.clone();
    let op = ops::spawn_op(&st, "backup.create", format!("Back up {}", s.name), Some(&id), true, move |ctx| async move {
        let st = ctx.st.clone();
        do_backup(&ctx, &st, &s, "manual", &note, None).await?;
        st.audit(&actor, "backup.create", &s.name, &note, Some(&s.id)).await;
        Ok(())
    });
    Ok((StatusCode::ACCEPTED, Json(json!({"operation": op}))))
}

async fn find(st: &St, user: &User, bid: &str, perm: &str) -> ApiResult<(String, String, String, String, String)> {
    let b = bid.to_string();
    let r: Option<(String, String, String, String, String)> = st.db.call(move |c| c.query_row("SELECT server_id,sha256,server_name,runtime,game_version FROM backups WHERE id=?", [b], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))).map(Some).or_else(|e| if matches!(e, rusqlite::Error::QueryReturnedNoRows) { Ok(None) } else { Err(e) })).await?;
    let r = r.ok_or_else(|| ApiError::not_found("backup"))?;
    if st.db.call({ let s = r.0.clone(); move |c| c.query_row("SELECT 1 FROM servers WHERE id=?", [s], |_| Ok(())) }).await.is_ok() { auth::authz(st, user, &r.0, perm).await?; } else { user.require_admin()?; }
    Ok(r)
}

pub async fn verify(State(st): State<St>, user: User, Path(bid): Path<String>) -> ApiResult<(StatusCode, Json<Value>)> {
    let (sid, sha, ..) = find(&st, &user, &bid, "backup.manage").await?;
    let b = bid.clone();
    let op = ops::spawn_op(&st, "backup.verify", "Verify backup", Some(&sid.clone()), false, move |ctx| async move {
        ctx.stage("Reading archive and checking SHA-256");
        let p = archive_path(&ctx.st, &sid, &b);
        let n = tokio::task::spawn_blocking(move || verify_archive(&p, &sha)).await??;
        ctx.log(format!("{n} entries verified, all paths safe"));
        let b2 = b.clone();
        ctx.st.db.call(move |c| c.execute("UPDATE backups SET verified_at=? WHERE id=?", [crate::db::now(), b2])).await?;
        Ok(())
    });
    Ok((StatusCode::ACCEPTED, Json(json!({"operation": op}))))
}


pub async fn delete(State(st): State<St>, user: User, Path(bid): Path<String>) -> ApiResult<StatusCode> {
    let (sid, ..) = find(&st, &user, &bid, "backup.manage").await?;
    remove_backup(&st, &sid, &bid).await?;
    st.audit(&user.email, "backup.delete", &bid, "", Some(&sid)).await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn download(State(st): State<St>, user: User, Path(bid): Path<String>) -> ApiResult<Response> {
    let (sid, ..) = find(&st, &user, &bid, "backup.manage").await?;
    let f = tokio::fs::File::open(archive_path(&st, &sid, &bid)).await?;
    let len = f.metadata().await?.len();
    let mut r = axum::body::Body::from_stream(tokio_util::io::ReaderStream::new(f)).into_response();
    let h = r.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/zstd"));
    h.insert(header::CONTENT_DISPOSITION, HeaderValue::from_str(&format!("attachment; filename=\"moss-backup-{bid}.tar.zst\"")).unwrap());
    h.insert(header::CONTENT_LENGTH, HeaderValue::from(len));
    Ok(r)
}
