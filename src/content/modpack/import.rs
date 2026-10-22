//! The modpack import operation: resolve and download hash-verified files, set the current content aside (replace
//! mode), place the files and overrides through `safe_join`, record provenance and undo what it can on failure.
use super::*;
use futures_util::StreamExt;
use std::path::{Path as FsPath, PathBuf};

pub(super) struct Fetched { pub(super) file: PackFile, pub(super) path: PathBuf }

pub(super) async fn download_one(ctx: OpCtx, token: Option<String>, f: PackFile, tmp: PathBuf, idx: usize) -> Result<Fetched> {
    let st = ctx.st.clone();
    let hash = f.sha512.clone().map(Hash::Sha512).or_else(|| f.sha1.clone().map(Hash::Sha1)).ok_or_else(|| anyhow!("{} has no hash", f.path))?;
    let dest = tmp.join(format!("{idx}.bin"));
    if let Some((pid, fid)) = f.cf {
        let url = f.urls.first().cloned().unwrap_or_default();
        let (u, headers, extra_host) = cf::download_target(&st, token.as_deref(), &pid.to_string(), &fid.to_string(), &url)?;
        let _permit = st.cf_sem.acquire().await?;
        download::fetch_ext(cf::dl_client(), &u, &dest, Some(hash), &ctx.token, |_| {}, download::Opts { headers, extra_host }).await?;
        return Ok(Fetched { file: f, path: dest });
    }
    let mut last = anyhow!("{} has no download URL", f.path);
    for url in &f.urls {
        let Some(host) = reqwest::Url::parse(url).ok().and_then(|u| u.host_str().map(String::from)).filter(|h| download::pack_host_allowed(h)) else { continue };
        match download::fetch_ext(&st.http, url, &dest, Some(hash.clone()), &ctx.token, |_| {}, download::Opts { extra_host: Some(host), ..Default::default() }).await {
            Ok(_) => return Ok(Fetched { file: f, path: dest }),
            Err(e) if ctx.cancelled() => return Err(e),
            Err(e) => last = e.context(format!("downloading {}", f.path)),
        }
    }
    Err(last)
}

/// On success `aside` is left in place: the caller deletes it once the install is recorded.
/// Replace mode sets the content directory aside first. Files and overrides are then written; on failure the
/// files created here are removed and the original directory is put back. (Overrides that replaced existing files
/// cannot be restored individually: that is what the pre-change backup is for.)
fn place(root: &FsPath, content_dir: &str, pack: &FsPath, overrides: &[OverrideEntry], files: &[(String, PathBuf)], replace: bool, aside: &FsPath) -> Result<usize> {
    let dir = safe_join(root, content_dir).map_err(|e| anyhow!(e.message))?;
    let stashed = replace && dir.exists();
    let _ = std::fs::remove_dir_all(aside);
    if stashed {
        if let Some(p) = aside.parent() { std::fs::create_dir_all(p)?; }
        std::fs::rename(&dir, aside)?;
    }
    let displaced = aside.join("displaced");
    let mut moved: Vec<(PathBuf, PathBuf)> = vec![];
    let mut created: Vec<PathBuf> = vec![];
    let res = (|| -> Result<usize> {
        for (rel, tmp) in files {
            let target = safe_join(root, rel).map_err(|e| anyhow!(e.message))?;
            if target.is_dir() { bail!("{rel} is a directory on the server"); }
            if let Some(p) = target.parent() { std::fs::create_dir_all(p)?; }
            let target = safe_join(root, rel).map_err(|e| anyhow!(e.message))?;
            if !stashed && target.is_file() {
                let keep = displaced.join(rel);
                if let Some(p) = keep.parent() { std::fs::create_dir_all(p)?; }
                std::fs::rename(&target, &keep)?;
                moved.push((target.clone(), keep));
            }
            if std::fs::rename(tmp, &target).is_err() { std::fs::copy(tmp, &target)?; }
            created.push(target);
        }
        let mut z = zip::ZipArchive::new(std::fs::File::open(pack)?)?;
        Ok(zipsafe::extract(&mut z, overrides, root)?.len())
    })();
    match res {
        Ok(n) => Ok(n),
        Err(e) => {
            for p in created { let _ = std::fs::remove_file(p); }
            for (target, keep) in moved { let _ = std::fs::rename(keep, target); }
            if !stashed { let _ = std::fs::remove_dir_all(aside); }
            if stashed { let _ = std::fs::remove_dir_all(&dir); let _ = std::fs::rename(aside, &dir); }
            Err(e)
        }
    }
}

struct Row { id: String, provider: String, project: Option<String>, version: Option<String>, title: String, label: String, filename: String, sha512: Option<String> }

fn rows_for(fetched: &[Fetched], content_dir: &str) -> Vec<Row> {
    fetched.iter().filter_map(|f| {
        let p = FsPath::new(&f.file.path);
        if p.parent().and_then(|x| x.to_str()) != Some(content_dir) { return None; }
        let filename = p.file_name()?.to_str()?.to_string();
        let (provider, project, version) = match &f.file.provider { Some((a, b, c)) => (a.clone(), Some(b.clone()), Some(c.clone())), None => ("manual".into(), None, None) };
        let title = filename.rsplit_once('.').map_or(filename.clone(), |(s, _)| s.to_string());
        Some(Row { id: uuid::Uuid::now_v7().to_string(), provider, project, version, title, label: filename.clone(), filename, sha512: f.file.sha512.clone() })
    }).collect()
}

pub(super) async fn import(ctx: OpCtx, s: Srv, c: Compat, entry: store::Entry, mode: String, utoken: Option<String>, actor: String) -> Result<()> {
    let st = ctx.st.clone();
    let _guard = st.lock_for(&s.id).try_lock_owned().map_err(|_| anyhow!("another operation is changing this server"))?;
    let replace = mode == "replace";
    ctx.stage("Reading the pack");
    let sc = inspect::scan(entry.path.clone()).await?;
    let pack = sc.pack;
    if pack.runtime() != s.runtime { bail!("this pack needs {} but the server runs {}", pack.runtime(), s.runtime); }
    if replace && (lifecycle_running(&st, &s) || crate::lifecycle::process_alive(&s)) { bail!("stop the server before replacing its content"); }
    let (files, unavailable) = if pack.format == "curseforge" {
        ctx.stage("Resolving files on CurseForge");
        cfpack::resolve(&st, utoken.as_deref(), &pack.cf_refs).await?
    } else { (pack.files.clone(), pack.unavailable.clone()) };
    for u in &unavailable { ctx.log(format!("Skipped {}: {}{}", u.name, u.reason, u.url.as_ref().map(|x| format!(" ({x})")).unwrap_or_default())); }
    let todo: Vec<PackFile> = files.into_iter().filter(|f| f.server != Side::Unsupported && !zipsafe::is_protected(&f.path)).collect();
    ctx.check()?;
    ctx.stage("Taking a pre-change backup");
    match maybe_backup(&st, &s).await? { Some(id) => ctx.log(format!("Backup {id} holds the previous state; restore it to roll back.")), None => ctx.log("No pre-change backup was taken.") }
    let tmp = st.cfg.tmp_dir().join(format!("modpack-{}", uuid::Uuid::now_v7()));
    tokio::fs::create_dir_all(&tmp).await?;
    let res: Result<usize> = async {
        let total = todo.len();
        let mut stream = futures_util::stream::iter(todo.into_iter().enumerate().map(|(i, f)| download_one(ctx.clone(), utoken.clone(), f, tmp.clone(), i))).buffer_unordered(4);
        let mut fetched: Vec<Fetched> = vec![];
        while let Some(r) = stream.next().await {
            fetched.push(r?);
            ctx.stage(&format!("Downloading files ({}/{total})", fetched.len()));
            ctx.progress(Some(fetched.len() as f64 / total.max(1) as f64 * 0.85));
            ctx.check()?;
        }
        drop(stream);
        fetched.sort_by(|a, b| a.file.path.cmp(&b.file.path));
        ctx.stage("Placing files and overrides");
        let placed: Vec<(String, PathBuf)> = fetched.iter().map(|f| (f.file.path.clone(), f.path.clone())).collect();
        let (root, cdir, ppath, ov, aside) = (s.dir(&st), c.dir.clone(), entry.path.clone(), sc.overrides.clone(), s.moss_dir(&st).join("modpack-prev"));
        let aside2 = aside.clone();
        let wrote = tokio::task::spawn_blocking(move || place(&root, &cdir, &ppath, &ov, &placed, replace, &aside2)).await??;
        ctx.log(format!("Installed {} files and {wrote} override files.", fetched.len()));
        let (root, rels) = (s.dir(&st), sc.overrides.iter().map(|o| o.rel.clone()).collect::<Vec<_>>());
        let sums = tokio::task::spawn_blocking(move || diff::sha256_paths(&root, &rels)).await?;
        if let Err(e) = record(&st, &s, &c, &pack, &mode, &fetched, entry.source.as_ref(), &sums).await {
            ctx.log(format!("Recording the install failed; the previous content remains in {}.", aside.display()));
            return Err(e);
        }
        let _ = tokio::fs::remove_dir_all(&aside).await;
        Ok(fetched.len())
    }.await;
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    let n = res.map_err(|e| e.context("modpack import failed; the server's files were left as they were where possible"))?;
    store::consume(&entry.token);
    st.timeline(&s.id, "modpack", &format!("{} {} ({n} files)", pack.name, pack.version)).await;
    st.audit(&actor, "modpack.import", &s.name, &format!("{} {} ({mode})", pack.name, pack.version), Some(&s.id)).await;
    ctx.stage("Done — start the server to load the content");
    Ok(())
}

/// Writes the install row (with the pack's provider identity when it has one, so it can be checked for updates later),
/// the item rows, and the checksums of the override files the pack wrote (the baseline for later pack updates).
#[allow(clippy::too_many_arguments)]
async fn record(st: &St, s: &Srv, c: &Compat, pack: &Pack, mode: &str, fetched: &[Fetched], source: Option<&store::Identity>, sums: &HashMap<String, String>) -> Result<()> {
    let rows = rows_for(fetched, &c.dir);
    let (sid, kind, dir, replace) = (s.id.clone(), c.kind.to_string(), c.dir.clone(), mode == "replace");
    let (src, sums) = (source.cloned(), sums.clone());
    let (install, name, version, format, gv, loader, mode, n) = (uuid::Uuid::now_v7().to_string(), pack.name.clone(), pack.version.clone(), pack.format.to_string(), pack.game_version.clone(), pack.loader.clone(), mode.to_string(), fetched.len() as i64);
    st.db.call(move |db| {
        let tx = db.unchecked_transaction()?;
        if replace { tx.execute("DELETE FROM content_items WHERE server_id=? AND dir=?", rusqlite::params![sid, dir])?; }
        tx.execute("INSERT INTO modpack_installs(id,server_id,name,version,format,game_version,loader,mode,installed_at,files) VALUES (?,?,?,?,?,?,?,?,?,?)", rusqlite::params![install, sid, name, version, format, gv, loader, mode, crate::db::now(), n])?;
        if let Some(i) = &src { tx.execute("UPDATE modpack_installs SET provider=?, project_id=?, version_id=? WHERE id=?", rusqlite::params![i.provider, i.project_id, i.version_id, install])?; }
        for (path, sum) in &sums { tx.execute("INSERT OR REPLACE INTO modpack_files(install_id,path,sha256) VALUES (?,?,?)", rusqlite::params![install, path, sum])?; }
        for r in rows {
            tx.execute("DELETE FROM content_items WHERE server_id=? AND dir=? AND filename=?", rusqlite::params![sid, dir, r.filename])?;
            tx.execute("INSERT INTO content_items(id,server_id,provider,project_id,version_id,title,version_number,filename,kind,sha512,installed_at,pinned,dir) VALUES (?,?,?,?,?,?,?,?,?,?,?,0,?)",
                rusqlite::params![r.id, sid, r.provider, r.project, r.version, r.title, r.label, r.filename, kind, r.sha512, crate::db::now(), dir])?;
            tx.execute("INSERT INTO modpack_items(content_item_id, install_id) VALUES (?,?)", rusqlite::params![r.id, install])?;
        }
        tx.commit()
    }).await
}

#[cfg(test)]
mod tests;
