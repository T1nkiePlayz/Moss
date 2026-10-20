//! Applying plans: install/update operations (download, verify, place, record, undo snapshot) and update checks.
use super::*;

pub async fn install(State(st): State<St>, user: User, Path(id): Path<String>, Json(r): Json<InstallReq>) -> ApiResult<(StatusCode, Json<Value>)> {
    let (s, c) = compat_for(&st, &user, &id, "content.manage").await?;
    let title = format!("Install content on {}", s.name);
    let actor = user.actor();
    let src = src_for(&st, &user, &c, r.provider.as_deref())?;
    let op = ops::spawn_op(&st, "content.install", title, Some(&id), true, move |ctx| apply(ctx, s, c, src, r.project_id, r.version_id, actor));
    Ok((StatusCode::ACCEPTED, Json(json!({"operation": op}))))
}

/// How a batch of steps is labelled in undo history, the timeline and the activity log.
pub(super) struct Labels { pub undo: Option<String>, pub audit: &'static str }

pub(super) async fn apply(ctx: OpCtx, s: Srv, c: Compat, src: Src, project_id: String, version_id: Option<String>, actor: String) -> Result<()> {
    let st = ctx.st.clone();
    let _guard = st.lock_for(&s.id).try_lock_owned().map_err(|_| anyhow!("another operation is changing this server"))?;
    ctx.stage("Resolving versions and dependencies");
    let (steps, _w) = build_plan(&st, &src, &s, &c, &project_id, version_id.as_deref()).await?;
    ctx.check()?;
    crate::backups::pre_change_locked(&st, &s, "changing content").await?;
    execute(&ctx, &s, &c, &steps, src.token.as_deref(), &actor, Labels { undo: None, audit: "content.install" }).await
}

/// Downloads every step, snapshots for undo, then applies all file moves and the database changes as one unit:
/// if anything fails while applying, files already replaced are put back. The caller holds the server lock.
pub(super) async fn execute(ctx: &OpCtx, s: &Srv, c: &Compat, steps: &[Step], token: Option<&str>, actor: &str, labels: Labels) -> Result<()> {
    let st = ctx.st.clone();
    let root = s.dir(&st);
    let target_dir = safe_join(&root, &c.dir).map_err(|e| anyhow!(e.message))?;
    tokio::fs::create_dir_all(&target_dir).await?;
    let tmp = st.cfg.tmp_dir().join(format!("content-{}", uuid::Uuid::now_v7()));
    let res = download_and_apply(ctx, s, c, steps, token, actor, &labels, &target_dir, &tmp).await;
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    res
}

#[allow(clippy::too_many_arguments)]
async fn download_and_apply(ctx: &OpCtx, s: &Srv, c: &Compat, steps: &[Step], token: Option<&str>, actor: &str, labels: &Labels, target_dir: &std::path::Path, tmp: &std::path::Path) -> Result<()> {
    let st = ctx.st.clone();
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
        if step.prov == Prov::Curseforge {
            // CurseForge downloads are rate limited like any other call and carry the key (directly or via the edge function)
            let fid = step.version["id"].as_str().unwrap_or_default();
            let (u, headers, extra_host) = cf::download_target(&st, token, &step.project_id, fid, url)?;
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
    for step in steps { if let Some((_, old)) = &step.replaces { touched.push(format!("{}/{}", c.dir, old)); } }
    touched.sort(); touched.dedup();
    let names = steps.iter().map(|x| x.title.as_str()).collect::<Vec<_>>().join(", ");
    undo::snapshot(&st, s, &labels.undo.clone().unwrap_or_else(|| format!("Install {names}")), &touched).await.map_err(|e| anyhow!(e.message))?;
    ctx.stage("Applying changes");
    let mut applied: Vec<Applied> = vec![];
    let placed = async { place_files(steps, &downloaded, target_dir, &tmp.join("replaced"), &mut applied).await?; record_rows(&st, s, c, steps, &downloaded).await }.await;
    if let Err(e) = placed {
        rollback(&applied).await;
        return Err(e.context("applying the change failed; files that were already replaced have been restored"));
    }
    for step in steps { ctx.log(format!("{} {} → {}/{}", step.action, step.title, c.dir, step.file["filename"].as_str().unwrap_or_default())); }
    st.timeline(&s.id, "update", &names).await;
    st.audit(actor, labels.audit, &s.name, &steps.iter().map(|x| format!("{} {}", x.title, x.version["version_number"].as_str().unwrap_or(""))).collect::<Vec<_>>().join(", "), Some(&s.id)).await;
    ctx.stage("Done — restart the server to load the change");
    Ok(())
}

/// A file placed in the server directory, with whatever it displaced (moved aside, never deleted until everything succeeds).
struct Applied { new: std::path::PathBuf, backups: Vec<(std::path::PathBuf, std::path::PathBuf)> }

/// Moves each downloaded file into place, setting aside whatever it replaces. Records progress in `applied` so a failure can be undone.
async fn place_files(steps: &[Step], downloaded: &[(String, std::path::PathBuf)], target_dir: &std::path::Path, aside: &std::path::Path, applied: &mut Vec<Applied>) -> Result<()> {
    for (i, (step, (fname, tmp_file))) in steps.iter().zip(downloaded.iter()).enumerate() {
        let dest = target_dir.join(fname);
        let mut a = Applied { new: dest.clone(), backups: vec![] };
        let mut displaced: Vec<std::path::PathBuf> = step.replaces.iter().map(|(_, old)| target_dir.join(old)).collect();
        if !displaced.contains(&dest) { displaced.push(dest.clone()); }
        for (k, path) in displaced.into_iter().enumerate() {
            if !path.is_file() { continue; }
            tokio::fs::create_dir_all(aside).await?;
            let b = aside.join(format!("{i}-{k}"));
            tokio::fs::rename(&path, &b).await?;
            a.backups.push((b, path));
        }
        applied.push(a);
        if tokio::fs::rename(tmp_file, &dest).await.is_err() { tokio::fs::copy(tmp_file, &dest).await?; }
    }
    Ok(())
}

/// Removes what was placed and puts displaced files back, newest first.
async fn rollback(applied: &[Applied]) {
    for a in applied.iter().rev() {
        let _ = tokio::fs::remove_file(&a.new).await;
        for (backup, orig) in &a.backups { let _ = tokio::fs::rename(backup, orig).await; }
    }
}

/// Swaps the inventory rows for the replaced items and adds the new ones, in one transaction.
async fn record_rows(st: &St, s: &Srv, c: &Compat, steps: &[Step], downloaded: &[(String, std::path::PathBuf)]) -> Result<()> {
    let rows: Vec<(Option<String>, [String; 8])> = steps.iter().zip(downloaded.iter()).map(|(step, (fname, _))| (step.replaces.as_ref().map(|(id, _)| id.clone()), [
        step.prov.as_str().to_string(), step.project_id.clone(), step.version["id"].as_str().unwrap_or_default().to_string(), step.title.clone(),
        step.version["version_number"].as_str().unwrap_or_default().to_string(), fname.clone(), c.kind.to_string(), step.file["hashes"]["sha512"].as_str().unwrap_or_default().to_string()])).collect();
    let (sid, dir) = (s.id.clone(), c.dir.clone());
    st.db.call(move |db| {
        let tx = db.unchecked_transaction()?;
        for (old, r) in rows {
            if let Some(o) = old { tx.execute("DELETE FROM content_items WHERE id=?", [o])?; }
            let sha = (!r[7].is_empty()).then(|| r[7].clone());
            tx.execute("INSERT INTO content_items(id,server_id,provider,project_id,version_id,title,version_number,filename,kind,sha512,installed_at,pinned,dir) VALUES (?,?,?,?,?,?,?,?,?,?,?,0,?)",
                rusqlite::params![uuid::Uuid::now_v7().to_string(), sid, r[0], r[1], r[2], r[3], r[4], r[5], r[6], sha, crate::db::now(), dir])?;
        }
        tx.commit()
    }).await
}

/// Updates found plus the problems met on the way: one provider being down must not hide what the other found.
pub(super) struct Updates { pub list: Vec<Value>, pub problems: Vec<String> }

/// Updates available for the non-pinned, enabled provider items in `items` (Modrinth per project, CurseForge in one batch call).
pub(super) async fn collect_updates(st: &St, token: Option<&str>, c: &Compat, items: &[Value]) -> Updates {
    let watched = |prov: &'static str| items.iter().filter(move |i| i["provider"] == prov && i["project_id"].is_string() && i["pinned"] != true && i["pack_pinned"] != true && !i["filename"].as_str().unwrap_or_default().ends_with(".disabled")).take(60).cloned().collect::<Vec<_>>();
    let checks = watched("modrinth").into_iter().map(|i| {
        let (st, pid, cur, item_id, title, cur_num) = (st.clone(), i["project_id"].as_str().unwrap().to_string(), i["version_id"].as_str().unwrap_or_default().to_string(), i["id"].as_str().unwrap_or_default().to_string(), i["title"].as_str().unwrap_or_default().to_string(), i["version_number"].clone());
        let c = c.clone();
        async move {
            let vs = match compatible_versions(&st, &pid, &c).await { Ok(v) => v, Err(e) => return Err(format!("{e:#}")) };
            let Some(latest) = pick_latest(&vs) else { return Ok(None) };
            Ok((latest["id"].as_str() != Some(cur.as_str())).then(|| json!({"item_id": item_id, "title": title, "current": cur_num, "latest": latest["version_number"], "version_id": latest["id"], "project_id": pid, "provider": "modrinth"})))
        }
    });
    let mut updates: Vec<Value> = vec![];
    let mut problems: Vec<String> = vec![];
    let mut failed = 0;
    for r in futures_util::future::join_all(checks).await {
        match r { Ok(Some(u)) => updates.push(u), Ok(None) => {}, Err(e) => { failed += 1; if !problems.contains(&e) { problems.push(e); } } }
    }
    if failed > 0 { problems.push(format!("{failed} Modrinth item(s) could not be checked, so the list may be incomplete.")); }
    // CurseForge: one batch call for every installed CurseForge item instead of one per item
    let cfi = watched("curseforge");
    if !cfi.is_empty() && cf::availability(st, token).0 {
        let ids: Vec<u64> = cfi.iter().filter_map(|i| i["project_id"].as_str()?.parse().ok()).collect();
        match cf::call(st, token, reqwest::Method::POST, "/v1/mods", &[], Some(json!({"modIds": ids}))).await {
            Ok(v) => for m in v["data"].as_array().into_iter().flatten() {
                let Some(item) = cfi.iter().find(|i| i["project_id"].as_str() == m["id"].as_u64().map(|n| n.to_string()).as_deref()) else { continue };
                let Some((fid, fname)) = cf::latest_index(m, c.game_version.as_deref(), c.cf_loader) else { continue };
                if item["version_id"].as_str() != Some(fid.as_str()) {
                    updates.push(json!({"item_id": item["id"], "title": item["title"], "current": item["version_number"], "latest": fname, "version_id": fid, "project_id": item["project_id"], "provider": "curseforge"}));
                }
            },
            Err(e) => problems.push(format!("CurseForge items were not checked: {e:#}")),
        }
    }
    Updates { list: updates, problems }
}

pub async fn check_updates(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let (_s, c) = compat_for(&st, &user, &id, "content.manage").await?;
    let items = db_items(&st, &id).await?;
    let u = collect_updates(&st, user.token.as_deref(), &c, &items).await;
    Ok(Json(json!({"updates": u.list, "warnings": u.problems})))
}

pub async fn update(State(st): State<St>, user: User, Path((id, item)): Path<(String, String)>) -> ApiResult<(StatusCode, Json<Value>)> {
    let (s, c) = compat_for(&st, &user, &id, "content.manage").await?;
    let i = item.clone();
    let (pid, provider): (Option<String>, String) = st.db.call(move |db| db.query_row("SELECT project_id,provider FROM content_items WHERE id=?", [i], |r| Ok((r.get(0)?, r.get(1)?)))).await.map_err(|_| ApiError::not_found("content item"))?;
    let src = src_for(&st, &user, &c, Some(&provider))?;
    let pid = pid.ok_or_else(|| ApiError::bad("content.manual", "manually added files have no provider to update from"))?;
    let actor = user.actor();
    let op = ops::spawn_op(&st, "content.update", format!("Update on {}", s.name), Some(&id), true, move |ctx| apply(ctx, s, c, src, pid, None, actor));
    Ok((StatusCode::ACCEPTED, Json(json!({"operation": op}))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(replaces: Option<&str>) -> Step {
        Step { prov: Prov::Modrinth, action: "replace", project_id: "p".into(), title: "P".into(), version: json!({"id": "v"}), file: json!({"filename": "x.jar"}), replaces: replaces.map(|o| ("row".to_string(), o.to_string())), reason: String::new() }
    }

    #[tokio::test]
    async fn a_failure_part_way_restores_every_replaced_file() {
        let d = std::env::temp_dir().join(format!("moss-apply-{}", uuid::Uuid::now_v7()));
        let (target, tmp) = (d.join("mods"), d.join("tmp"));
        std::fs::create_dir_all(&target).unwrap();
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(target.join("a-1.jar"), "old a").unwrap();
        std::fs::write(target.join("b-1.jar"), "old b").unwrap();
        std::fs::write(tmp.join("a-2.jar"), "new a").unwrap();
        // b-2.jar was never downloaded: placing the second step fails
        let (steps, downloaded) = (vec![step(Some("a-1.jar")), step(Some("b-1.jar"))], vec![("a-2.jar".to_string(), tmp.join("a-2.jar")), ("b-2.jar".to_string(), tmp.join("b-2.jar"))]);
        let mut applied = vec![];
        let r = place_files(&steps, &downloaded, &target, &tmp.join("replaced"), &mut applied).await;
        assert!(r.is_err());
        assert_eq!(std::fs::read_to_string(target.join("a-2.jar")).unwrap(), "new a", "first step was applied before the failure");
        rollback(&applied).await;
        assert_eq!(std::fs::read_to_string(target.join("a-1.jar")).unwrap(), "old a");
        assert_eq!(std::fs::read_to_string(target.join("b-1.jar")).unwrap(), "old b");
        assert!(!target.join("a-2.jar").exists() && !target.join("b-2.jar").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[tokio::test]
    async fn success_replaces_old_files_and_handles_same_name_updates() {
        let d = std::env::temp_dir().join(format!("moss-apply-{}", uuid::Uuid::now_v7()));
        let (target, tmp) = (d.join("mods"), d.join("tmp"));
        std::fs::create_dir_all(&target).unwrap();
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(target.join("same.jar"), "old").unwrap();
        std::fs::write(tmp.join("same.jar"), "new").unwrap();
        let mut applied = vec![];
        place_files(&[step(Some("same.jar"))], &[("same.jar".to_string(), tmp.join("same.jar"))], &target, &tmp.join("replaced"), &mut applied).await.unwrap();
        assert_eq!(std::fs::read_to_string(target.join("same.jar")).unwrap(), "new");
        rollback(&applied).await;
        assert_eq!(std::fs::read_to_string(target.join("same.jar")).unwrap(), "old");
        let _ = std::fs::remove_dir_all(&d);
    }
}
