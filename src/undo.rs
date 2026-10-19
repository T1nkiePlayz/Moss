//! Undo history: before any Moss-driven mutation of a file, the previous version is snapshotted so the
//! change can be reverted with one click. Snapshots live in `<server>/.moss/undo/<id>/`.
use crate::{api::server_for, auth::{self, User}, error::{ApiError, ApiResult}, files::{atomic_write, safe_join}, server::Srv, state::St};
use axum::{Json, extract::{Path, State}};
use serde_json::{Value, json};

const MAX_FILE: u64 = 64 << 20;
const KEEP: i64 = 40;

/// Snapshot the given server-relative files (those that don't exist yet are recorded as "created").
pub async fn snapshot(st: &St, s: &Srv, title: &str, files: &[String]) -> ApiResult<String> {
    let id = uuid::Uuid::now_v7().to_string();
    let (root, base) = (s.dir(st), s.moss_dir(st).join("undo").join(&id));
    let rels: Vec<String> = files.to_vec();
    let entries: Vec<Value> = tokio::task::spawn_blocking(move || -> ApiResult<Vec<Value>> {
        let mut out = vec![];
        for rel in rels {
            let p = safe_join(&root, &rel)?;
            let existed = p.is_file();
            // too large to copy: record it as skipped so a revert leaves the file alone instead of deleting it
            let skipped = existed && std::fs::metadata(&p).map(|m| m.len() > MAX_FILE).unwrap_or(true);
            if existed && !skipped {
                let dst = base.join(&rel);
                if let Some(par) = dst.parent() { std::fs::create_dir_all(par)?; }
                std::fs::copy(&p, &dst)?;
            }
            out.push(json!({"path": rel, "existed": existed, "skipped": skipped}));
        }
        Ok(out)
    }).await.map_err(|e| anyhow::anyhow!(e))??;
    let (i, sid, t, f) = (id.clone(), s.id.clone(), title.to_string(), serde_json::to_string(&entries).unwrap());
    st.db.call(move |c| {
        c.execute("INSERT INTO undo(id,server_id,title,created_at,files) VALUES (?,?,?,?,?)", rusqlite::params![i, sid, t, crate::db::now(), f])?;
        c.execute("DELETE FROM undo WHERE server_id=?1 AND id NOT IN (SELECT id FROM undo WHERE server_id=?1 ORDER BY id DESC LIMIT ?2)", rusqlite::params![sid, KEEP])?;
        Ok(())
    }).await?;
    prune_dirs(st, s).await;
    Ok(id)
}

async fn prune_dirs(st: &St, s: &Srv) {
    let sid = s.id.clone();
    let keep: Vec<String> = st.db.call(move |c| { let mut q = c.prepare("SELECT id FROM undo WHERE server_id=?")?; q.query_map([sid], |r| r.get(0))?.collect() }).await.unwrap_or_default();
    let base = s.moss_dir(st).join("undo");
    if let Ok(mut rd) = tokio::fs::read_dir(&base).await {
        while let Ok(Some(e)) = rd.next_entry().await {
            if !keep.contains(&e.file_name().to_string_lossy().to_string()) { let _ = tokio::fs::remove_dir_all(e.path()).await; }
        }
    }
}

pub async fn list(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    server_for(&st, &user, &id, "server.view").await?;
    let v: Vec<Value> = st.db.call(move |c| {
        let mut q = c.prepare("SELECT id,title,created_at,files FROM undo WHERE server_id=? ORDER BY id DESC")?;
        q.query_map([id], |r| {
            let files: Vec<Value> = serde_json::from_str(&r.get::<_, String>(3)?).unwrap_or_default();
            Ok(json!({"id": r.get::<_, String>(0)?, "title": r.get::<_, String>(1)?, "created_at": r.get::<_, String>(2)?, "files": files.iter().filter_map(|f| f["path"].as_str()).collect::<Vec<_>>()}))
        })?.collect()
    }).await?;
    Ok(Json(json!(v)))
}

pub async fn revert(State(st): State<St>, user: User, Path(uid): Path<String>) -> ApiResult<Json<Value>> {
    let u = uid.clone();
    let row: Option<(String, String, String)> = st.db.call(move |c| {
        c.query_row("SELECT server_id,title,files FROM undo WHERE id=?", [u], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map(Some).or_else(|e| if matches!(e, rusqlite::Error::QueryReturnedNoRows) { Ok(None) } else { Err(e) })
    }).await?;
    let (sid, title, files) = row.ok_or_else(|| ApiError::not_found("undo entry"))?;
    let s = server_for(&st, &user, &sid, "files.write").await?;
    if !user.is_admin() { auth::authz(&st, &user, &sid, "content.manage").await?; }
    let lock = st.lock_for(&sid).try_lock_owned().map_err(|_| ApiError::conflict("server.locked", "another operation is changing this server"))?;
    let entries: Vec<Value> = serde_json::from_str(&files).unwrap_or_default();
    let (root, base) = (s.dir(&st), s.moss_dir(&st).join("undo").join(&uid));
    tokio::task::spawn_blocking(move || -> ApiResult<()> {
        for e in entries {
            let rel = e["path"].as_str().unwrap_or_default();
            let p = safe_join(&root, rel)?;
            if e["skipped"] == true { continue; }
            if e["existed"] == true {
                if let Some(par) = p.parent() { std::fs::create_dir_all(par)?; }
                atomic_write(&p, &std::fs::read(base.join(rel))?)?;
            } else if p.is_file() { std::fs::remove_file(&p)?; }
        }
        Ok(())
    }).await.map_err(|e| anyhow::anyhow!(e))??;
    // content changes also touched the inventory: resync it with disk
    crate::content::resync(&st, &s).await;
    drop(lock);
    st.timeline(&sid, "undo", &title).await;
    st.audit(&user.email, "undo.revert", &s.name, &title, Some(&sid)).await;
    Ok(Json(json!({"ok": true})))
}
