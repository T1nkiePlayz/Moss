//! The per-server inventory of installed content: listing, resync with disk, removal and pinning.
use super::*;

pub(super) fn item_json(r: &rusqlite::Row) -> rusqlite::Result<Value> {
    Ok(json!({"id": r.get::<_, String>(0)?, "provider": r.get::<_, String>(1)?, "project_id": r.get::<_, Option<String>>(2)?, "version_id": r.get::<_, Option<String>>(3)?, "title": r.get::<_, String>(4)?,
        "version_number": r.get::<_, Option<String>>(5)?, "filename": r.get::<_, String>(6)?, "kind": r.get::<_, String>(7)?, "sha512": r.get::<_, Option<String>>(8)?,
        "installed_at": r.get::<_, String>(9)?, "pinned": r.get::<_, bool>(10)?, "source": r.get::<_, Option<String>>(12)?.unwrap_or_else(|| "moss".into()), "dir": r.get::<_, String>(11)?, "pack_pinned": r.get::<_, bool>(13)?}))
}

pub(super) async fn db_items(st: &St, sid: &str) -> Result<Vec<Value>> {
    let sid = sid.to_string();
    st.db.call(move |c| {
        let mut q = c.prepare("SELECT id,provider,project_id,version_id,title,version_number,filename,kind,sha512,installed_at,pinned,dir,COALESCE(source, (SELECT 'modpack' FROM modpack_items m WHERE m.content_item_id=content_items.id)), EXISTS(SELECT 1 FROM modpack_items m JOIN modpack_installs i ON i.id=m.install_id WHERE m.content_item_id=content_items.id AND i.pinned=1) FROM content_items WHERE server_id=? ORDER BY title COLLATE NOCASE")?;
        q.query_map([sid], item_json)?.collect()
    }).await
}

/// `<name>.<ext>` or `<name>.<ext>.disabled`.
pub(super) fn is_content_file(name: &str, ext: &str) -> bool {
    let lower = name.to_lowercase();
    lower.ends_with(&format!(".{ext}")) || lower.ends_with(&format!(".{ext}.disabled"))
}

/// A bare file name (no path parts, not hidden) that is `<x>.<ext>` or `<x>.<ext>.disabled`.
pub(super) fn content_filename(n: &str, ext: &str) -> Result<String> {
    let base = std::path::Path::new(n).file_name().and_then(|f| f.to_str()).unwrap_or_default();
    if base.is_empty() || base.starts_with('.') || base != n || !is_content_file(base, ext) { bail!("{n:?} is not an acceptable .{ext} file name"); }
    Ok(base.to_string())
}

/// The inventory's base items (managed rows plus manual files, with `enabled`/`identified`) and each file's path.
pub(super) async fn list_files(st: &St, s: &Srv, c: &Compat) -> Result<Vec<(Value, std::path::PathBuf)>> {
    let mut items = db_items(st, &s.id).await?;
    let known: Vec<(String, String)> = items.iter().map(|i| (i["dir"].as_str().unwrap_or_default().to_string(), i["filename"].as_str().unwrap_or_default().to_string())).collect();
    let root = s.dir(st);
    let dir = safe_join(&root, &c.dir).map_err(|e| anyhow!(e.message))?;
    let (ext, cdir) = (c.ext, c.dir.clone());
    let manual: Vec<Value> = tokio::task::spawn_blocking(move || {
        let mut v = vec![];
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if !is_content_file(&name, ext) || known.iter().any(|(d, f)| *d == cdir && *f == name) || !e.file_type().is_ok_and(|t| t.is_file()) { continue; }
                let when = e.metadata().ok().and_then(|m| m.modified().ok()).map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339()).unwrap_or_default();
                let title = name.trim_end_matches(".disabled").trim_end_matches(&format!(".{ext}")).to_string();
                v.push(json!({"id": format!("manual:{name}"), "provider": "manual", "project_id": null, "version_id": null, "title": title, "version_number": null, "filename": name, "kind": "", "sha512": null, "installed_at": when, "pinned": false, "pack_pinned": false, "source": "manual", "dir": cdir}));
            }
        }
        v.sort_by(|a, b| a["filename"].as_str().cmp(&b["filename"].as_str()));
        v
    }).await.map_err(|e| anyhow!(e))?;
    items.extend(manual);
    Ok(items.into_iter().map(|mut i| {
        let (d, f) = (i["dir"].as_str().unwrap_or(&c.dir).to_string(), i["filename"].as_str().unwrap_or_default().to_string());
        i["enabled"] = json!(!f.to_lowercase().ends_with(".disabled"));
        i["identified"] = json!(i["provider"] != "manual");
        i["kind"] = json!(c.kind);
        let path = safe_join(&root, &format!("{d}/{f}")).unwrap_or_else(|_| root.join(".invalid"));
        (i, path)
    }).collect())
}

pub(super) async fn list_with_info(st: &St, s: &Srv, c: &Compat) -> Result<Vec<(Value, std::sync::Arc<crate::jars::Info>)>> {
    let files = list_files(st, s, c).await?;
    let infos = crate::jars::read_many(files.iter().map(|(_, p)| p.clone()).collect()).await;
    Ok(files.into_iter().zip(infos).map(|((i, _), info)| (i, info)).collect())
}

pub(super) fn entries(list: &[(Value, std::sync::Arc<crate::jars::Info>)]) -> Vec<check::Entry> {
    list.iter().map(|(i, info)| check::Entry {
        item: i["id"].as_str().unwrap_or_default().to_string(), title: i["title"].as_str().unwrap_or_default().to_string(),
        filename: i["filename"].as_str().unwrap_or_default().to_string(), enabled: i["enabled"] == true, info: info.clone(),
    }).collect()
}

/// The full inventory (contract v2): base item plus `meta` and `issues`; internal `dir` removed.
pub(super) async fn items_json(st: &St, s: &Srv, c: &Compat) -> Result<Vec<Value>> {
    let list = list_with_info(st, s, c).await?;
    let ctx = check::Ctx::of(s, c);
    let findings = check::run(&ctx, &entries(&list));
    let acc = ctx.accepted();
    Ok(list.into_iter().map(|(mut i, info)| {
        let id = i["id"].as_str().unwrap_or_default().to_string();
        i["meta"] = info.pick(&acc).map(|m| json!(m)).unwrap_or(Value::Null);
        i["issues"] = json!(findings.iter().filter(|f| f.item.as_deref() == Some(&id) || f.related.contains(&id)).collect::<Vec<_>>());
        i.as_object_mut().map(|o| o.remove("dir"));
        i
    }).collect())
}

pub async fn installed(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let (s, c) = compat_for(&st, &user, &id, "server.view").await?;
    Ok(Json(json!({"items": items_json(&st, &s, &c).await?})))
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
    let (filename, row_id) = if let Some(f) = item.strip_prefix("manual:") { (content_filename(f, c.ext).map_err(|e| ApiError::bad("validation", e.to_string()))?, None) } else {
        let i = item.clone();
        let f: Option<(String, String)> = st.db.call(move |db| db.query_row("SELECT filename,dir FROM content_items WHERE id=?", [i], |r| Ok((r.get(0)?, r.get(1)?))).map(Some).or_else(|e| if matches!(e, rusqlite::Error::QueryReturnedNoRows) { Ok(None) } else { Err(e) })).await?;
        (f.ok_or_else(|| ApiError::not_found("content item"))?.0, Some(item.clone()))
    };
    let actor = user.actor();
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
