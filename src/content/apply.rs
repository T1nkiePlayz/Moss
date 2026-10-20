//! Applying plans: install/update operations (download, verify, place, record, undo snapshot) and update checks.
use super::*;

pub async fn install(State(st): State<St>, user: User, Path(id): Path<String>, Json(r): Json<InstallReq>) -> ApiResult<(StatusCode, Json<Value>)> {
    let (s, c) = compat_for(&st, &user, &id, "content.manage").await?;
    let title = format!("Install content on {}", s.name);
    let actor = user.email.clone();
    let src = src_for(&st, &user, &c, r.provider.as_deref())?;
    let op = ops::spawn_op(&st, "content.install", title, Some(&id), true, move |ctx| apply(ctx, s, c, src, r.project_id, r.version_id, actor));
    Ok((StatusCode::ACCEPTED, Json(json!({"operation": op}))))
}

pub(super) async fn apply(ctx: OpCtx, s: Srv, c: Compat, src: Src, project_id: String, version_id: Option<String>, actor: String) -> Result<()> {
    let st = ctx.st.clone();
    let _guard = st.lock_for(&s.id).try_lock_owned().map_err(|_| anyhow!("another operation is changing this server"))?;
    ctx.stage("Resolving versions and dependencies");
    let (steps, _w) = build_plan(&st, &src, &s, &c, &project_id, version_id.as_deref()).await?;
    ctx.check()?;
    let root = s.dir(&st);
    let target_dir = safe_join(&root, &c.dir).map_err(|e| anyhow!(e.message))?;
    tokio::fs::create_dir_all(&target_dir).await?;
    let tmp = st.cfg.tmp_dir().join(format!("content-{}", uuid::Uuid::now_v7()));
    let n = steps.len() as f64;
    let mut downloaded = vec![];
    for (i, step) in steps.iter().enumerate() {
        ctx.stage(&format!("Downloading {} ({}/{})", step.title, i + 1, steps.len()));
        let fname = safe_filename(step.file["filename"].as_str().unwrap_or_default(), c.ext)?;
        let url = step.file["url"].as_str().ok_or_else(|| anyhow!("no download url"))?;
        let hash = step.file["hashes"]["sha512"].as_str().map(|h| Hash::Sha512(h.into())).or_else(|| step.file["hashes"]["sha1"].as_str().map(|h| Hash::Sha1(h.into())));
        let dest = tmp.join(&fname);
        let c2 = ctx.clone();
        let base = i as f64 / n;
        let progress = move |p: f64| c2.progress(Some((base + p / n) * 0.8));
        if src.prov == Prov::Curseforge {
            // CurseForge downloads are rate limited like any other call and carry the key (directly or via the edge function)
            let fid = step.version["id"].as_str().unwrap_or_default();
            let (u, headers, extra_host) = cf::download_target(&st, src.token.as_deref(), &step.project_id, fid, url)?;
            let _permit = st.cf_sem.acquire().await?;
            download::fetch_ext(cf::dl_client(), &u, &dest, hash, &ctx.token, progress, download::Opts { headers, extra_host }).await?;
        } else {
            download::fetch(&st.http, url, &dest, hash, &ctx.token, progress).await?;
        }
        downloaded.push((fname, dest));
    }
    ctx.check()?;
    ctx.stage("Snapshotting files for undo");
    let mut touched: Vec<String> = downloaded.iter().map(|(f, _)| format!("{}/{}", c.dir, f)).collect();
    for step in &steps { if let Some((_, old)) = &step.replaces { touched.push(format!("{}/{}", c.dir, old)); } }
    touched.sort(); touched.dedup();
    undo::snapshot(&st, &s, &format!("Install {}", steps.iter().map(|x| x.title.as_str()).collect::<Vec<_>>().join(", ")), &touched).await.map_err(|e| anyhow!(e.message))?;
    ctx.stage("Applying changes");
    for (step, (fname, tmp_file)) in steps.iter().zip(downloaded.iter()) {
        if let Some((old_id, old_name)) = &step.replaces {
            let _ = tokio::fs::remove_file(target_dir.join(old_name)).await;
            let oid = old_id.clone();
            st.db.call(move |c| c.execute("DELETE FROM content_items WHERE id=?", [oid])).await?;
        }
        let dest = target_dir.join(fname);
        if tokio::fs::rename(tmp_file, &dest).await.is_err() { tokio::fs::copy(tmp_file, &dest).await?; }
        let prov = src.prov.as_str();
        let (sid, pid, vid, title, vn, fnm, kind, sha, dir) = (s.id.clone(), step.project_id.clone(), step.version["id"].as_str().unwrap_or_default().to_string(), step.title.clone(), step.version["version_number"].as_str().unwrap_or_default().to_string(), fname.clone(), c.kind.to_string(), step.file["hashes"]["sha512"].as_str().map(String::from), c.dir.clone());
        st.db.call(move |db| db.execute("INSERT INTO content_items(id,server_id,provider,project_id,version_id,title,version_number,filename,kind,sha512,installed_at,pinned,dir) VALUES (?,?,?,?,?,?,?,?,?,?,?,0,?)",
            rusqlite::params![uuid::Uuid::now_v7().to_string(), sid, prov, pid, vid, title, vn, fnm, kind, sha, crate::db::now(), dir])).await?;
        ctx.log(format!("{} {} → {}/{}", step.action, step.title, c.dir, fname));
    }
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    st.timeline(&s.id, "update", &steps.iter().map(|x| x.title.as_str()).collect::<Vec<_>>().join(", ")).await;
    st.audit(&actor, "content.install", &s.name, &steps.iter().map(|x| format!("{} {}", x.title, x.version["version_number"].as_str().unwrap_or(""))).collect::<Vec<_>>().join(", "), Some(&s.id)).await;
    ctx.stage("Done — restart the server to load the change");
    Ok(())
}

pub async fn check_updates(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let (_s, c) = compat_for(&st, &user, &id, "content.manage").await?;
    let items = db_items(&st, &id).await?;
    let watched = |prov: &'static str| items.iter().filter(move |i| i["provider"] == prov && i["project_id"].is_string() && i["pinned"] != true).take(60).cloned().collect::<Vec<_>>();
    let checks = watched("modrinth").into_iter().map(|i| {
        let (st, pid, cur, item_id, title, cur_num) = (st.clone(), i["project_id"].as_str().unwrap().to_string(), i["version_id"].as_str().unwrap_or_default().to_string(), i["id"].as_str().unwrap_or_default().to_string(), i["title"].as_str().unwrap_or_default().to_string(), i["version_number"].clone());
        let c = Compat { kind: c.kind, project_type: c.project_type, loaders: c.loaders.clone(), game_version: c.game_version.clone(), dir: c.dir.clone(), ext: c.ext, cf_class: c.cf_class, cf_loader: c.cf_loader };
        async move {
            let vs = compatible_versions(&st, &pid, &c).await.ok()?;
            let latest = pick_latest(&vs)?;
            (latest["id"].as_str()? != cur).then(|| json!({"item_id": item_id, "title": title, "current": cur_num, "latest": latest["version_number"], "version_id": latest["id"]}))
        }
    });
    let mut updates: Vec<Value> = futures_util::future::join_all(checks).await.into_iter().flatten().collect();
    // CurseForge: one batch call for every installed CurseForge item instead of one per item
    let cfi = watched("curseforge");
    if !cfi.is_empty() && cf::availability(&st, user.token.as_deref()).0 {
        let ids: Vec<u64> = cfi.iter().filter_map(|i| i["project_id"].as_str()?.parse().ok()).collect();
        match cf::call(&st, user.token.as_deref(), reqwest::Method::POST, "/v1/mods", &[], Some(json!({"modIds": ids}))).await {
            Ok(v) => for m in v["data"].as_array().into_iter().flatten() {
                let Some(item) = cfi.iter().find(|i| i["project_id"].as_str() == m["id"].as_u64().map(|n| n.to_string()).as_deref()) else { continue };
                let Some((fid, fname)) = cf::latest_index(m, c.game_version.as_deref(), c.cf_loader) else { continue };
                if item["version_id"].as_str() != Some(fid.as_str()) {
                    updates.push(json!({"item_id": item["id"], "title": item["title"], "current": item["version_number"], "latest": fname, "version_id": fid}));
                }
            },
            Err(e) => return Err(upstream(e)),
        }
    }
    Ok(Json(json!({"updates": updates})))
}

pub async fn update(State(st): State<St>, user: User, Path((id, item)): Path<(String, String)>) -> ApiResult<(StatusCode, Json<Value>)> {
    let (s, c) = compat_for(&st, &user, &id, "content.manage").await?;
    let i = item.clone();
    let (pid, provider): (Option<String>, String) = st.db.call(move |db| db.query_row("SELECT project_id,provider FROM content_items WHERE id=?", [i], |r| Ok((r.get(0)?, r.get(1)?)))).await.map_err(|_| ApiError::not_found("content item"))?;
    let src = src_for(&st, &user, &c, Some(&provider))?;
    let pid = pid.ok_or_else(|| ApiError::bad("content.manual", "manually added files have no provider to update from"))?;
    let actor = user.email.clone();
    let op = ops::spawn_op(&st, "content.update", format!("Update on {}", s.name), Some(&id), true, move |ctx| apply(ctx, s, c, src, pid, None, actor));
    Ok((StatusCode::ACCEPTED, Json(json!({"operation": op}))))
}
