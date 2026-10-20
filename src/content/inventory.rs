//! The per-server inventory of installed content: listing, resync with disk, removal and pinning.
use super::*;

pub(super) fn item_json(r: &rusqlite::Row) -> rusqlite::Result<Value> {
    Ok(json!({"id": r.get::<_, String>(0)?, "provider": r.get::<_, String>(1)?, "project_id": r.get::<_, Option<String>>(2)?, "version_id": r.get::<_, Option<String>>(3)?, "title": r.get::<_, String>(4)?,
        "version_number": r.get::<_, Option<String>>(5)?, "filename": r.get::<_, String>(6)?, "kind": r.get::<_, String>(7)?, "sha512": r.get::<_, Option<String>>(8)?,
        "installed_at": r.get::<_, String>(9)?, "pinned": r.get::<_, bool>(10)?, "source": "moss", "dir": r.get::<_, String>(11)?}))
}

pub(super) async fn db_items(st: &St, sid: &str) -> Result<Vec<Value>> {
    let sid = sid.to_string();
    st.db.call(move |c| {
        let mut q = c.prepare("SELECT id,provider,project_id,version_id,title,version_number,filename,kind,sha512,installed_at,pinned,dir FROM content_items WHERE server_id=? ORDER BY title COLLATE NOCASE")?;
        q.query_map([sid], item_json)?.collect()
    }).await
}

pub async fn installed(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let (s, c) = compat_for(&st, &user, &id, "server.view").await?;
    let mut items = db_items(&st, &id).await?;
    let known: Vec<(String, String)> = items.iter().map(|i| (i["dir"].as_str().unwrap_or_default().to_string(), i["filename"].as_str().unwrap_or_default().to_string())).collect();
    let dir = safe_join(&s.dir(&st), &c.dir)?;
    let (ext, cdir) = (c.ext, c.dir.clone());
    let manual: Vec<Value> = tokio::task::spawn_blocking(move || {
        let mut v = vec![];
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if !name.ends_with(&format!(".{ext}")) || known.iter().any(|(d, f)| *d == cdir && *f == name) { continue; }
                let when = e.metadata().ok().and_then(|m| m.modified().ok()).map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339()).unwrap_or_default();
                v.push(json!({"id": format!("manual:{name}"), "provider": "manual", "project_id": null, "version_id": null, "title": name.trim_end_matches(&format!(".{ext}")), "version_number": null, "filename": name, "kind": "", "sha512": null, "installed_at": when, "pinned": false, "source": "manual", "dir": cdir}));
            }
        }
        v
    }).await.map_err(|e| anyhow!(e))?;
    for m in manual { items.push(m); }
    for i in items.iter_mut() { i["kind"] = json!(c.kind); i.as_object_mut().map(|o| o.remove("dir")); }
    Ok(Json(json!({"items": items})))
}

pub async fn resync(st: &St, s: &Srv) {
    let Some(c) = compat(st, s) else { return };
    let root = s.dir(st);
    let Ok(items) = db_items(st, &s.id).await else { return };
    for i in items {
        let (dir, f) = (i["dir"].as_str().unwrap_or(&c.dir).to_string(), i["filename"].as_str().unwrap_or_default().to_string());
        if !root.join(&dir).join(&f).exists() {
            let id = i["id"].as_str().unwrap_or_default().to_string();
            let _ = st.db.call(move |c| c.execute("DELETE FROM content_items WHERE id=?", [id])).await;
        }
    }
}

pub async fn remove(State(st): State<St>, user: User, Path((id, item)): Path<(String, String)>) -> ApiResult<(StatusCode, Json<Value>)> {
    let (s, c) = compat_for(&st, &user, &id, "content.manage").await?;
    let (filename, row_id) = if let Some(f) = item.strip_prefix("manual:") { (safe_filename(f, c.ext).map_err(|e| ApiError::bad("validation", e.to_string()))?, None) } else {
        let i = item.clone();
        let f: Option<(String, String)> = st.db.call(move |db| db.query_row("SELECT filename,dir FROM content_items WHERE id=?", [i], |r| Ok((r.get(0)?, r.get(1)?))).map(Some).or_else(|e| if matches!(e, rusqlite::Error::QueryReturnedNoRows) { Ok(None) } else { Err(e) })).await?;
        (f.ok_or_else(|| ApiError::not_found("content item"))?.0, Some(item.clone()))
    };
    let actor = user.email.clone();
    let op = ops::spawn_op(&st, "content.remove", format!("Remove {filename}"), Some(&id), false, move |ctx| async move {
        let st = ctx.st.clone();
        let _g = st.lock_for(&s.id).try_lock_owned().map_err(|_| anyhow!("another operation is changing this server"))?;
        ctx.stage("Snapshotting for undo");
        let rel = format!("{}/{}", c.dir, filename);
        undo::snapshot(&st, &s, &format!("Remove {filename}"), std::slice::from_ref(&rel)).await.map_err(|e| anyhow!(e.message))?;
        ctx.stage("Removing file (plugin data and configuration are kept)");
        let p = safe_join(&s.dir(&st), &rel).map_err(|e| anyhow!(e.message))?;
        match tokio::fs::remove_file(&p).await { Ok(()) => {}, Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}, Err(e) => return Err(e.into()) }
        if let Some(rid) = row_id { st.db.call(move |db| db.execute("DELETE FROM content_items WHERE id=?", [rid])).await?; }
        st.audit(&actor, "content.remove", &s.name, &filename, Some(&s.id)).await;
        Ok(())
    });
    Ok((StatusCode::ACCEPTED, Json(json!({"operation": op}))))
}

#[derive(Deserialize)] pub struct PinReq { pinned: bool }
pub async fn patch_item(State(st): State<St>, user: User, Path((id, item)): Path<(String, String)>, Json(r): Json<PinReq>) -> ApiResult<StatusCode> {
    server_for(&st, &user, &id, "content.manage").await?;
    let n = st.db.call(move |db| db.execute("UPDATE content_items SET pinned=? WHERE id=? AND server_id=?", rusqlite::params![r.pinned, item, id])).await?;
    if n == 0 { return Err(ApiError::not_found("content item")); }
    Ok(StatusCode::NO_CONTENT)
}
