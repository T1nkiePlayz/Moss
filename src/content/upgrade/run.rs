//! The upgrade operation. Order matters for atomicity: download the target builds, apply the content change
//! (journaled renames), then swap the runtime files; any failure rolls the journal back and the installer path
//! restores the previous server jar/loader files. The database is only touched after everything succeeded.
use super::*;
use std::path::PathBuf;

/// A downloaded replacement for an installed item.
struct Prepared { entry: Entry, filename: String, path: PathBuf, label: String, sha512: Option<String> }

/// Reversible filesystem changes made to the content directory.
#[derive(Default)]
struct Journal { created: Vec<PathBuf>, renamed: Vec<(PathBuf, PathBuf)> }

impl Journal {
    fn rename(&mut self, from: PathBuf, to: PathBuf) -> std::io::Result<()> {
        if let Some(p) = to.parent() { std::fs::create_dir_all(p)?; }
        std::fs::rename(&from, &to)?;
        self.renamed.push((from, to));
        Ok(())
    }
    fn rollback(&mut self) {
        for p in self.created.drain(..) { let _ = std::fs::remove_file(p); }
        for (from, to) in self.renamed.drain(..).rev() { let _ = std::fs::rename(to, from); }
    }
}

async fn fetch_build(ctx: &OpCtx, st: &St, token: Option<&str>, ct: &Compat, e: &Entry, tmp: &std::path::Path) -> Result<Option<Prepared>> {
    let (Some(pid), Some(vid)) = (e.link.project_id.as_deref(), e.version_id.as_deref()) else { return Ok(None) };
    let label = e.target.clone().unwrap_or_default();
    if e.link.provider.as_deref() == Some("curseforge") {
        let f = cf::get(st, token, &format!("/v1/mods/{pid}/files/{vid}"), &[]).await?["data"].clone();
        if !cf::downloadable(&f) { ctx.log(format!("{}: the author disabled automatic downloads; left unchanged. Download it from {}", e.link.title, cf::project_url(pid.parse().unwrap_or(0)))); return Ok(None); }
        let fname = safe_filename(f["fileName"].as_str().unwrap_or_default(), ct.ext)?;
        let sha1 = f["hashes"].as_array().into_iter().flatten().find(|h| h["algo"] == 1).and_then(|h| h["value"].as_str()).map(|h| Hash::Sha1(h.into()));
        let (u, headers, extra_host) = cf::download_target(st, token, pid, vid, f["downloadUrl"].as_str().unwrap_or_default())?;
        let _permit = st.cf_sem.acquire().await?;
        let path = tmp.join(&fname);
        download::fetch_ext(cf::dl_client(), &u, &path, sha1, &ctx.token, |_| {}, download::Opts { headers, extra_host }).await?;
        return Ok(Some(Prepared { entry: e.clone(), filename: fname, path, label, sha512: None }));
    }
    let v = mr(st, &format!("/version/{vid}"), &[]).await?;
    let f = primary_file(&v).ok_or_else(|| anyhow!("{} has no files", e.link.title))?;
    let fname = safe_filename(f["filename"].as_str().unwrap_or_default(), ct.ext)?;
    let sha512 = f["hashes"]["sha512"].as_str().map(String::from);
    let hash = sha512.clone().map(Hash::Sha512).or_else(|| f["hashes"]["sha1"].as_str().map(|h| Hash::Sha1(h.into())));
    let path = tmp.join(&fname);
    download::fetch(&st.http, f["url"].as_str().ok_or_else(|| anyhow!("no download url"))?, &path, hash, &ctx.token, |_| {}).await?;
    Ok(Some(Prepared { entry: e.clone(), filename: fname, path, label, sha512 }))
}

fn apply_content(dir: &std::path::Path, aside: &std::path::Path, choice: &str, entries: &[Entry], prepared: &[Prepared], j: &mut Journal) -> std::io::Result<()> {
    if choice == "update" {
        for p in prepared {
            let old = dir.join(&p.entry.link.filename);
            if old.exists() { j.rename(old, aside.join(&p.entry.link.filename))?; }
            let dest = dir.join(&p.filename);
            if dest.exists() { j.rename(dest.clone(), aside.join(format!("replaced-{}", p.filename)))?; }
            std::fs::create_dir_all(dir)?;
            if std::fs::rename(&p.path, &dest).is_err() { std::fs::copy(&p.path, &dest)?; }
            j.created.push(dest);
        }
    } else if choice == "disable_incompatible" {
        for e in entries.iter().filter(|e| e.status == "missing") {
            let from = dir.join(&e.link.filename);
            if from.is_file() { j.rename(from.clone(), PathBuf::from(format!("{}.disabled", from.display())))?; }
        }
    }
    Ok(())
}

pub(super) async fn run(ctx: OpCtx, s: Srv, c: Compat, token: Option<String>, req: UpgradeReq, actor: String) -> Result<()> {
    let st = ctx.st.clone();
    let _guard = st.lock_for(&s.id).try_lock_owned().map_err(|_| anyhow!("another operation is changing this server"))?;
    ctx.stage("Planning the upgrade");
    let b = build(&st, token.as_deref(), &s, &c, &req.game_version, req.loader_version.as_deref(), req.confirm_downgrade).await?;
    if !b.blockers.is_empty() { bail!("{}", b.blockers.join(" ")); }
    let resolved = b.resolved.ok_or_else(|| anyhow!("the server software could not be resolved"))?;
    ctx.check()?;
    if req.backup {
        ctx.stage("Taking a pre-change backup");
        match crate::backups::pre_change_forced_locked(&st, &s, "a version upgrade", &ctx).await? { Some(id) => ctx.log(format!("Backup {id} holds the previous state; restore it to roll back.")), None => ctx.log("No pre-change backup was taken.") }
    }
    let (root, ct) = (s.dir(&st), for_game(&c, &req.game_version));
    let dir = safe_join(&root, &c.dir).map_err(|e| anyhow!(e.message))?;
    let tmp = st.cfg.tmp_dir().join(format!("upgrade-{}", uuid::Uuid::now_v7()));
    let aside = root.join(".moss").join("upgrade-content-prev");
    let res = async {
        let mut prepared = vec![];
        if req.content == "update" {
            let todo: Vec<&Entry> = b.entries.iter().filter(|e| e.status == "update").collect();
            for (i, e) in todo.iter().enumerate() {
                ctx.stage(&format!("Downloading {} ({}/{})", e.link.title, i + 1, todo.len()));
                ctx.check()?;
                if let Some(p) = fetch_build(&ctx, &st, token.as_deref(), &ct, e, &tmp).await? { prepared.push(p); }
            }
        }
        ctx.stage("Applying the content change");
        let _ = tokio::fs::remove_dir_all(&aside).await;
        let mut journal = Journal::default();
        let (d, a, choice, entries, prep) = (dir.clone(), aside.clone(), req.content.clone(), b.entries.clone(), std::mem::take(&mut prepared));
        let applied = tokio::task::spawn_blocking(move || { let r = apply_content(&d, &a, &choice, &entries, &prep, &mut journal); (r, journal, prep) }).await?;
        let (r, mut journal, prep) = applied;
        if let Err(e) = r { journal.rollback(); return Err(anyhow!("could not change the content files: {e}")); }
        let swap = async {
            ctx.check()?;
            ctx.stage(&format!("Installing {} {}", s.runtime, req.game_version));
            let mut target = s.clone();
            target.game_version = req.game_version.clone();
            target.loader_version = resolved.loader_version.clone().or_else(|| req.loader_version.clone());
            crate::installer::replace_runtime(&ctx, &st, &target, &resolved, &root).await
        }.await;
        if let Err(e) = swap { journal.rollback(); ctx.log("Rolled back the content change."); return Err(e); }
        Ok::<_, anyhow::Error>(prep)
    }.await;
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    let prep = match res { Ok(p) => p, Err(e) => { let _ = tokio::fs::remove_dir_all(&aside).await; return Err(e.context("upgrade failed; the server was left on its previous version")); } };
    let _ = tokio::fs::remove_dir_all(&aside).await;
    ctx.stage("Recording the new version");
    let (sid, gv, lv, java) = (s.id.clone(), req.game_version.clone(), resolved.loader_version.clone().or_else(|| req.loader_version.clone()), resolved.java_major as i64);
    let rows: Vec<(String, String, String, String, Option<String>)> = prep.iter().map(|p| (p.entry.link.item.clone(), p.entry.version_id.clone().unwrap_or_default(), p.label.clone(), p.filename.clone(), p.sha512.clone())).collect();
    st.db.call(move |db| {
        let tx = db.unchecked_transaction()?;
        tx.execute("UPDATE servers SET game_version=?, loader_version=COALESCE(?,loader_version), java_major=? WHERE id=?", rusqlite::params![gv, lv, java, sid])?;
        for (item, vid, label, fname, sha) in rows { tx.execute("UPDATE content_items SET version_id=?, version_number=?, filename=?, sha512=?, installed_at=? WHERE id=?", rusqlite::params![vid, label, fname, sha, crate::db::now(), item])?; }
        tx.commit()
    }).await?;
    for p in &prep { ctx.log(format!("Updated {} to {}", p.entry.link.title, p.label)); }
    st.timeline(&s.id, "upgrade", &format!("{} {} → {}", s.runtime, s.game_version, req.game_version)).await;
    st.audit(&actor, "server.upgrade", &s.name, &format!("{} → {} (content: {})", s.game_version, req.game_version, req.content), Some(&s.id)).await;
    crate::server::emit(&st, &s.id).await;
    ctx.stage("Done — start the server to finish the upgrade");
    Ok(())
}

#[cfg(test)]
mod tests;
