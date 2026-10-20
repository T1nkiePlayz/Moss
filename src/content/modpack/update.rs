//! Installed modpacks: listing, pinning a pack to its version, checking a provider for a newer version (with a diff),
//! and applying a newer version as a reviewed change: pre-change backup, undo snapshot, journaled file moves with
//! rollback, and never an overwrite of a config file the person edited.
use super::*;
use super::diff::{self, Diff, Incoming, Installed, Versions};
use super::import::{Fetched, download_one};
use futures_util::StreamExt;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path as FsPath, PathBuf};

#[derive(Clone, Debug)]
pub struct InstallRow {
    pub id: String, pub name: String, pub version: String, pub format: String, pub game_version: String, pub loader: String, pub mode: String,
    pub installed_at: String, pub files: i64, pub pinned: bool, pub provider: Option<String>, pub project_id: Option<String>, pub version_id: Option<String>,
}

impl InstallRow {
    fn json(&self, items: i64, overrides: i64) -> Value {
        json!({"id": self.id, "name": self.name, "version": self.version, "format": self.format, "game_version": self.game_version, "loader": self.loader, "mode": self.mode,
            "installed_at": self.installed_at, "files": self.files, "pinned": self.pinned, "items": items, "override_files": overrides,
            "source": self.provider.as_ref().zip(self.project_id.as_ref()).map(|(p, id)| json!({"provider": p, "project_id": id, "version_id": self.version_id})),
            "can_check": self.provider.is_some() && self.project_id.is_some()})
    }
}

const ICOLS: &str = "id,name,version,format,game_version,loader,mode,installed_at,files,pinned,provider,project_id,version_id";

fn irow(r: &rusqlite::Row) -> rusqlite::Result<InstallRow> {
    Ok(InstallRow { id: r.get(0)?, name: r.get(1)?, version: r.get(2)?, format: r.get(3)?, game_version: r.get(4)?, loader: r.get(5)?, mode: r.get(6)?, installed_at: r.get(7)?, files: r.get(8)?,
        pinned: r.get(9)?, provider: r.get(10)?, project_id: r.get(11)?, version_id: r.get(12)? })
}

pub async fn install_row(st: &St, server: &str, id: &str) -> ApiResult<InstallRow> {
    let (s, i) = (server.to_string(), id.to_string());
    st.db.call(move |c| c.query_row(&format!("SELECT {ICOLS} FROM modpack_installs WHERE id=? AND server_id=?"), [i, s], irow)).await.map_err(|_| ApiError::not_found("modpack"))
}

/// `GET /servers/{id}/modpacks`: the packs imported into this server, newest first.
pub async fn list(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    crate::api::server_for(&st, &user, &id, "server.view").await?;
    let rows: Vec<Value> = st.db.call(move |c| {
        let mut q = c.prepare(&format!("SELECT {ICOLS},(SELECT COUNT(*) FROM modpack_items WHERE install_id=modpack_installs.id),(SELECT COUNT(*) FROM modpack_files WHERE install_id=modpack_installs.id) FROM modpack_installs WHERE server_id=? ORDER BY installed_at DESC"))?;
        q.query_map([id], |r| Ok(irow(r)?.json(r.get(13)?, r.get(14)?)))?.collect()
    }).await?;
    Ok(Json(json!({"packs": rows})))
}

#[derive(Deserialize)] pub struct PinPack { pinned: bool }

/// `PATCH /servers/{id}/modpacks/{install}`: keep this pack at its current version (or release it).
pub async fn pin(State(st): State<St>, user: User, Path((id, install)): Path<(String, String)>, Json(r): Json<PinPack>) -> ApiResult<Json<Value>> {
    let s = crate::api::server_for(&st, &user, &id, "content.manage").await?;
    let n = { let (i, sid) = (install.clone(), id.clone()); st.db.call(move |c| c.execute("UPDATE modpack_installs SET pinned=? WHERE id=? AND server_id=?", rusqlite::params![r.pinned, i, sid])).await? };
    if n == 0 { return Err(ApiError::not_found("modpack")); }
    st.audit(&user.actor(), if r.pinned { "modpack.pin" } else { "modpack.unpin" }, &s.name, &install, Some(&id)).await;
    Ok(Json(json!({"id": install, "pinned": r.pinned})))
}

/// The newest version the provider offers for the pack's project: (version id, label).
async fn latest_version(st: &St, user: &User, provider: &str, pid: &str) -> Result<(String, String)> {
    if provider == "curseforge" {
        let token = user.token.as_deref();
        if let (false, Some(why)) = cf::availability(st, token) { bail!("{why}"); }
        let m = cf::get(st, token, &format!("/v1/mods/{pid}"), &[]).await?["data"].clone();
        let files: Vec<Value> = m["latestFiles"].as_array().cloned().unwrap_or_default();
        let f = cf::pick_latest(&files).ok_or_else(|| anyhow!("no downloadable file found for this pack"))?;
        Ok((f["id"].as_u64().unwrap_or_default().to_string(), f["displayName"].as_str().unwrap_or_default().to_string()))
    } else {
        let vs = mr(st, &format!("/project/{pid}/version"), &[]).await?;
        let vs = vs.as_array().cloned().unwrap_or_default();
        let v = pick_latest(&vs).ok_or_else(|| anyhow!("this project has no versions"))?;
        Ok((v["id"].as_str().unwrap_or_default().to_string(), v["version_number"].as_str().unwrap_or_default().to_string()))
    }
}

async fn installed_items(st: &St, install: &str) -> Result<Vec<Installed>> {
    let i = install.to_string();
    st.db.call(move |c| {
        let mut q = c.prepare("SELECT c.id,c.provider,c.project_id,c.version_id,c.title,c.version_number,c.dir,c.filename,c.pinned,c.sha512 FROM modpack_items m JOIN content_items c ON c.id=m.content_item_id WHERE m.install_id=?")?;
        q.query_map([i], |r| Ok(Installed { item_id: r.get(0)?, provider: r.get(1)?, project: r.get(2)?, version_id: r.get(3)?, title: r.get(4)?, version: r.get(5)?,
            path: format!("{}/{}", r.get::<_, String>(6)?, r.get::<_, String>(7)?), pinned: r.get(8)?, sha512: r.get(9)? }))?.collect()
    }).await
}

async fn recorded_sums(st: &St, install: &str) -> Result<HashMap<String, String>> {
    let i = install.to_string();
    st.db.call(move |c| { let mut q = c.prepare("SELECT path,sha256 FROM modpack_files WHERE install_id=?")?; q.query_map([i], |r| Ok((r.get(0)?, r.get(1)?)))?.collect() }).await
}

/// Checksums of the override files inside the archive (what the new version would write).
fn archive_sums(pack: &FsPath, entries: &[OverrideEntry]) -> Result<BTreeMap<String, String>> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut z = zip::ZipArchive::new(std::fs::File::open(pack)?)?;
    let mut out = BTreeMap::new();
    for e in entries {
        let mut f = z.by_index(e.index)?;
        let (mut h, mut buf, mut left) = (Sha256::new(), vec![0u8; 64 * 1024], e.size + 1);
        while left > 0 {
            let n = (&mut f).take(left.min(buf.len() as u64)).read(&mut buf)?;
            if n == 0 { break; }
            h.update(&buf[..n]);
            left -= n as u64;
        }
        out.insert(e.rel.clone(), hex::encode(h.finalize()));
    }
    Ok(out)
}

fn stem(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.rsplit_once('.').map_or(name.to_string(), |(s, _)| s.to_string())
}

/// Everything a pack update works from.
pub struct Plan { pub diff: Diff, pub versions: Versions, pub to_version: String, pub pack: Pack, pub scanned_overrides: Vec<OverrideEntry>, pub new_sums: BTreeMap<String, String>,
    pub on_disk: HashMap<String, String>, pub fetch: Vec<PackFile> }

/// Diffs the pack file at `path` against what `row` installed on server `s`.
pub async fn plan(st: &St, utoken: Option<&str>, s: &Srv, c: &Compat, row: &InstallRow, path: &FsPath) -> Result<Plan> {
    let sc = inspect::scan(path.to_path_buf()).await?;
    let pack = sc.pack.clone();
    let mut blockers = vec![];
    if pack.runtime() != s.runtime { blockers.push(format!("This pack version needs {} but the server runs {}.", pack.runtime(), s.runtime)); }
    if pack.game_version != s.game_version { blockers.push(format!("This pack version is for Minecraft {} but the server runs {}. Use the version upgrade assistant first.", pack.game_version, s.game_version)); }
    if let (Some(a), Some(b)) = (pack.loader_version.as_deref(), s.loader_version.as_deref()) && a != b && pack.game_version == s.game_version {
        blockers.push(format!("This pack version needs {} {a} but the server has {b}. Use the version upgrade assistant first.", pack.loader));
    }
    let (files, unavailable) = if pack.format == "curseforge" { cfpack::resolve(st, utoken, &pack.cf_refs).await? } else { (pack.files.clone(), pack.unavailable.clone()) };
    let wanted: Vec<PackFile> = files.into_iter().filter(|f| f.server != Side::Unsupported && !zipsafe::is_protected(&f.path)).collect();
    let (inside, outside): (Vec<PackFile>, Vec<PackFile>) = wanted.into_iter().partition(|f| FsPath::new(&f.path).parent().and_then(|p| p.to_str()) == Some(c.dir.as_str()));
    let incoming: Vec<Incoming> = inside.iter().map(|f| Incoming { path: f.path.clone(), provider: f.provider.clone(), sha512: f.sha512.clone(), title: stem(&f.path) }).collect();
    let installed = installed_items(st, &row.id).await?;
    let files_diff = diff::diff_files(&installed, &incoming);
    let (p2, entries) = (path.to_path_buf(), sc.overrides.clone());
    let new_sums = tokio::task::spawn_blocking(move || archive_sums(&p2, &entries)).await??;
    let recorded = recorded_sums(st, &row.id).await?;
    let (root, rels): (PathBuf, Vec<String>) = (s.dir(st), new_sums.keys().chain(recorded.keys()).cloned().collect());
    let on_disk = tokio::task::spawn_blocking(move || diff::sha256_paths(&root, &rels)).await?;
    let ov = diff::diff_overrides(&new_sums, &recorded, &on_disk);
    let mut warnings = vec![];
    if !unavailable.is_empty() { warnings.push(format!("{} file(s) cannot be downloaded automatically and will be left out.", unavailable.len())); }
    if !outside.is_empty() { warnings.push(format!("{} file(s) outside the {} folder are not updated by Moss.", outside.len(), c.dir)); }
    if !ov.edited.is_empty() { warnings.push(format!("{} config file(s) you changed will be left as they are.", ov.edited.len())); }
    if !files_diff.held.is_empty() { warnings.push(format!("{} item(s) are pinned or disabled and will not change.", files_diff.held.len())); }
    if !sc.skipped.is_empty() { warnings.push(format!("{} file(s) that would replace server software are ignored.", sc.skipped.len())); }
    let to_version = pack.version.clone();
    let versions = Versions {
        from: json!({"version": row.version, "game_version": row.game_version, "loader": row.loader}),
        to: json!({"version": pack.version, "game_version": pack.game_version, "loader": pack.loader, "loader_version": pack.loader_version}),
    };
    let fetch: Vec<PackFile> = inside.iter().filter(|f| files_diff.added.iter().chain(files_diff.updated.iter()).any(|ch| ch.path == f.path)).cloned().collect();
    Ok(Plan { diff: Diff { files: files_diff, overrides: ov, blockers, warnings }, versions, to_version, pack, scanned_overrides: sc.overrides, new_sums, on_disk, fetch })
}

#[derive(Deserialize, Default)]
pub struct CheckReq { token: Option<String> }

/// `POST /servers/{id}/modpacks/{install}/check-update`: is there a newer pack version, and what would it change?
/// With a `token` (a newer pack the person uploaded and inspected) the diff is made against that file instead.
pub async fn check_update(State(st): State<St>, user: User, Path((id, install)): Path<(String, String)>, body: Option<Json<CheckReq>>) -> ApiResult<Json<Value>> {
    let (s, c) = compat_for(&st, &user, &id, "content.manage").await?;
    let row = install_row(&st, &id, &install).await?;
    let req = body.map(|b| b.0).unwrap_or_default();
    let current = json!({"version": row.version, "version_id": row.version_id});
    let (path, ident) = if let Some(t) = &req.token {
        let e = store::get(t, &user.id).ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "modpack.token_expired", "this modpack token is unknown or has expired (tokens last 30 minutes); inspect the pack again"))?;
        (e.path.clone(), e.source.clone())
    } else {
        let (Some(prov), Some(pid)) = (row.provider.clone(), row.project_id.clone()) else {
            return Err(ApiError::conflict("modpack.no_source", "This pack was imported from a file, so Moss cannot look for a newer version by itself. Upload the newer pack file to compare it."));
        };
        if row.pinned { return Ok(Json(json!({"pinned": true, "up_to_date": null, "current": current, "message": "This pack is pinned to its current version. Unpin it to check for updates."}))); }
        let (vid, label) = latest_version(&st, &user, &prov, &pid).await.map_err(upstream)?;
        if Some(&vid) == row.version_id.as_ref() { return Ok(Json(json!({"pinned": false, "up_to_date": true, "current": current, "latest": {"version_id": vid, "version": label}}))); }
        let (path, _size, ident) = remote::fetch_remote(&st, &user, &json!({"provider": prov, "project_id": pid, "version_id": vid})).await?;
        (path, Some(ident))
    };
    let p = plan(&st, user.token.as_deref(), &s, &c, &row, &path).await.map_err(|e| ApiError::bad("modpack.invalid", format!("{e:#}")))?;
    // a pack that was fetched here is staged under a token so applying it (or testing it on a copy) needs no second download
    let token = match &req.token {
        Some(t) => t.clone(),
        None => store::put(&st, store::Entry { token: String::new(), path: path.clone(), user: user.id.clone(), created: std::time::Instant::now(), runtime: p.pack.runtime().to_string(),
            game_version: p.pack.game_version.clone(), loader_version: p.pack.loader_version.clone(), name: p.pack.name.clone(), source: ident.clone() }),
    };
    let up_to_date = !p.diff.changes_anything() && p.versions.from["version"] == p.versions.to["version"];
    Ok(Json(json!({
        "pinned": row.pinned, "up_to_date": up_to_date, "current": current, "latest": {"version_id": ident.as_ref().map(|i| i.version_id.clone()), "version": p.to_version},
        "token": token, "diff": p.diff.json(&p.versions, &p.to_version),
    })))
}

/// Journaled file changes: everything set aside or created, so a failure puts the server back exactly as it was.
#[derive(Default)]
pub struct Journal { created: Vec<PathBuf>, aside: Vec<(PathBuf, PathBuf)> }

impl Journal {
    fn set_aside(&mut self, from: &FsPath, aside: &FsPath, copy: bool) -> std::io::Result<()> {
        let to = aside.join(self.aside.len().to_string());
        std::fs::create_dir_all(aside)?;
        if copy { std::fs::copy(from, &to)?; } else { std::fs::rename(from, &to)?; }
        self.aside.push((to, from.to_path_buf()));
        Ok(())
    }
    pub fn rollback(&mut self) {
        for p in self.created.drain(..) { let _ = std::fs::remove_file(p); }
        for (from, orig) in self.aside.drain(..).rev() { if let Some(par) = orig.parent() { let _ = std::fs::create_dir_all(par); } let _ = std::fs::rename(from, orig); }
    }
}

/// What to put where. Files are (relative target, downloaded temp file, relative path of the old file it replaces).
pub struct Placement { pub removes: Vec<String>, pub files: Vec<(String, PathBuf, Option<String>)>, pub overrides: Vec<OverrideEntry> }

pub fn place(root: &FsPath, aside: &FsPath, p: &Placement, pack: &FsPath, j: &mut Journal) -> Result<usize> {
    let _ = std::fs::remove_dir_all(aside);
    let res = (|| -> Result<usize> {
        for rel in &p.removes {
            let path = safe_join(root, rel).map_err(|e| anyhow!(e.message))?;
            if path.is_file() { j.set_aside(&path, aside, false)?; }
        }
        for (rel, tmp, old) in &p.files {
            let target = safe_join(root, rel).map_err(|e| anyhow!(e.message))?;
            if target.is_dir() { bail!("{rel} is a directory on the server"); }
            if let Some(old) = old {
                let o = safe_join(root, old).map_err(|e| anyhow!(e.message))?;
                if o.is_file() { j.set_aside(&o, aside, false)?; }
            }
            if target.is_file() { j.set_aside(&target, aside, false)?; }
            if let Some(par) = target.parent() { std::fs::create_dir_all(par)?; }
            let target = safe_join(root, rel).map_err(|e| anyhow!(e.message))?;
            if std::fs::rename(tmp, &target).is_err() { std::fs::copy(tmp, &target)?; }
            j.created.push(target);
        }
        for e in &p.overrides {
            let target = safe_join(root, &e.rel).map_err(|x| anyhow!(x.message))?;
            if target.is_file() { j.set_aside(&target, aside, true)?; } else { j.created.push(target); }
        }
        let mut z = zip::ZipArchive::new(std::fs::File::open(pack)?)?;
        Ok(zipsafe::extract(&mut z, &p.overrides, root)?.len())
    })();
    if res.is_err() { j.rollback(); }
    res
}

/// Applies the pack file at `pack_path` to the install `row_id` on `s`. Takes the server lock. Returns the signature of
/// what was applied. `expect` is the signature a staged test was made against; `identity` is where the pack came from.
#[allow(clippy::too_many_arguments)]
pub async fn apply_update(ctx: &OpCtx, s: &Srv, c: &Compat, row_id: &str, pack_path: &FsPath, utoken: Option<&str>, actor: &str, expect: Option<&str>, backup: bool, identity: Option<store::Identity>) -> Result<String> {
    let st = ctx.st.clone();
    let _guard = st.lock_for(&s.id).try_lock_owned().map_err(|_| anyhow!("another operation is changing this server"))?;
    let row = install_row(&st, &s.id, row_id).await.map_err(|e| anyhow!(e.message))?;
    if row.pinned { bail!("this pack is pinned to its current version; unpin it first"); }
    ctx.stage("Comparing the new pack version with what is installed");
    let p = plan(&st, utoken, s, c, &row, pack_path).await?;
    if !p.diff.blockers.is_empty() { bail!("{}", p.diff.blockers.join(" ")); }
    let sig = p.diff.signature(&p.to_version);
    if let Some(e) = expect && e != sig { bail!("what the pack update would change differs from what was tested; run the test again"); }
    ctx.check()?;
    if backup {
        ctx.stage("Taking a pre-change backup");
        match crate::content::maybe_backup(&st, s).await? { Some(id) => ctx.log(format!("Backup {id} holds the previous state; restore it to roll back.")), None => ctx.log("No pre-change backup was taken.") }
    }
    let tmp = st.cfg.tmp_dir().join(format!("packupdate-{}", uuid::Uuid::now_v7()));
    tokio::fs::create_dir_all(&tmp).await?;
    let res = apply_inner(ctx, s, c, &row, pack_path, utoken, actor, &p, identity, &tmp).await;
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    res?;
    Ok(sig)
}

#[allow(clippy::too_many_arguments)]
async fn apply_inner(ctx: &OpCtx, s: &Srv, c: &Compat, row: &InstallRow, pack_path: &FsPath, utoken: Option<&str>, actor: &str, p: &Plan, identity: Option<store::Identity>, tmp: &FsPath) -> Result<()> {
    let st = ctx.st.clone();
    let total = p.fetch.len();
    let mut stream = futures_util::stream::iter(p.fetch.iter().cloned().enumerate().map(|(i, f)| download_one(ctx.clone(), utoken.map(String::from), f, tmp.to_path_buf(), i))).buffer_unordered(4);
    let mut fetched: Vec<Fetched> = vec![];
    while let Some(r) = stream.next().await {
        fetched.push(r?);
        ctx.stage(&format!("Downloading files ({}/{total})", fetched.len()));
        ctx.progress(Some(fetched.len() as f64 / total.max(1) as f64 * 0.8));
        ctx.check()?;
    }
    drop(stream);
    let d = &p.diff;
    let writable: Vec<String> = d.overrides.writable().into_iter().map(String::from).collect();
    let ov_entries: Vec<OverrideEntry> = p.scanned_overrides.iter().filter(|e| writable.contains(&e.rel)).cloned().collect();
    // installed path of every updated item (the file the new one replaces)
    let installed = installed_items(&st, &row.id).await?;
    let old_path = |item: &Option<String>| item.as_ref().and_then(|id| installed.iter().find(|i| &i.item_id == id)).map(|i| i.path.clone());
    let files: Vec<(String, PathBuf, Option<String>)> = fetched.iter().map(|f| {
        let upd = d.files.updated.iter().find(|u| u.path == f.file.path);
        (f.file.path.clone(), f.path.clone(), upd.and_then(|u| old_path(&u.item_id)))
    }).collect();
    let removes: Vec<String> = d.files.removed.iter().map(|r| r.path.clone()).collect();
    ctx.stage("Snapshotting files for undo");
    let mut touched: Vec<String> = removes.clone();
    for (rel, _, old) in &files { touched.push(rel.clone()); if let Some(o) = old { touched.push(o.clone()); } }
    touched.extend(ov_entries.iter().map(|e| e.rel.clone()));
    touched.sort(); touched.dedup();
    undo::snapshot(&st, s, &format!("Update {} to {}", p.pack.name, p.to_version), &touched).await.map_err(|e| anyhow!(e.message))?;
    ctx.check()?;
    ctx.stage("Applying the pack update");
    let (root, aside, pack, placement) = (s.dir(&st), s.moss_dir(&st).join("pack-update-prev"), pack_path.to_path_buf(), Placement { removes, files, overrides: ov_entries });
    let (wrote, mut journal) = tokio::task::spawn_blocking(move || { let mut j = Journal::default(); let r = place(&root, &aside, &placement, &pack, &mut j); r.map(|n| (n, j)) }).await??;
    let recorded = record_update(&st, s, c, row, p, &fetched, identity, &writable).await;
    if let Err(e) = recorded { journal.rollback(); return Err(e.context("recording the update failed; the files were put back")); }
    let _ = tokio::fs::remove_dir_all(s.moss_dir(&st).join("pack-update-prev")).await;
    for ch in &d.files.updated { ctx.log(format!("Updated {}", ch.title)); }
    for ch in &d.files.added { ctx.log(format!("Added {}", ch.title)); }
    for ch in &d.files.removed { ctx.log(format!("Removed {}", ch.title)); }
    ctx.log(format!("Wrote {wrote} config file(s); left {} you changed alone.", d.overrides.edited.len()));
    st.timeline(&s.id, "modpack", &format!("{} → {}", p.pack.name, p.to_version)).await;
    st.audit(actor, "modpack.update", &s.name, &format!("{} {} → {}", p.pack.name, row.version, p.to_version), Some(&s.id)).await;
    ctx.stage("Done — restart the server to load the update");
    Ok(())
}

/// (item id, file name, new version id, title, sha512)
type UpdRow = (String, String, Option<String>, String, Option<String>);
/// (new item id, provider, project, version, title, file name, sha512)
type AddRow = (String, String, Option<String>, Option<String>, String, String, Option<String>);

#[allow(clippy::too_many_arguments)]
async fn record_update(st: &St, s: &Srv, c: &Compat, row: &InstallRow, p: &Plan, fetched: &[Fetched], identity: Option<store::Identity>, written: &[String]) -> Result<()> {
    let d = &p.diff;
    let now = crate::db::now();
    let (sid, dir, kind, install) = (s.id.clone(), c.dir.clone(), c.kind.to_string(), row.id.clone());
    let version_of = |path: &str| fetched.iter().find(|f| f.file.path == path).map(|f| (f.file.provider.clone(), f.file.sha512.clone()));
    let upd: Vec<UpdRow> = d.files.updated.iter().filter_map(|u| {
        let (prov, sha) = version_of(&u.path)?;
        let fname = u.path.rsplit('/').next()?.to_string();
        Some((u.item_id.clone()?, fname.clone(), prov.map(|x| x.2), fname, sha))
    }).collect();
    let add: Vec<AddRow> = d.files.added.iter().filter_map(|a| {
        let f = fetched.iter().find(|f| f.file.path == a.path)?;
        let fname = a.path.rsplit('/').next()?.to_string();
        let (prov, project, version) = match &f.file.provider { Some((x, y, z)) => (x.clone(), Some(y.clone()), Some(z.clone())), None => ("manual".into(), None, None) };
        Some((uuid::Uuid::now_v7().to_string(), prov, project, version, stem(&fname), fname, f.file.sha512.clone()))
    }).collect();
    let del: Vec<String> = d.files.removed.iter().filter_map(|r| r.item_id.clone()).collect();
    let baseline: Vec<(String, String)> = p.new_sums.iter().filter(|(k, v)| written.contains(k) || p.on_disk.get(*k) == Some(*v)).map(|(k, v)| (k.clone(), v.clone())).collect();
    let (name, version, gv, loader, n_files) = (p.pack.name.clone(), p.pack.version.clone(), p.pack.game_version.clone(), p.pack.loader.clone(), (d.files.unchanged + d.files.added.len() + d.files.updated.len() + d.files.held.len()) as i64);
    st.db.call(move |db| {
        let tx = db.unchecked_transaction()?;
        for id in del { tx.execute("DELETE FROM content_items WHERE id=?", [id])?; }
        for (id, fname, vid, label, sha) in upd {
            tx.execute("UPDATE content_items SET version_id=COALESCE(?,version_id), version_number=?, filename=?, sha512=?, installed_at=? WHERE id=?", rusqlite::params![vid, label, fname, sha, now, id])?;
        }
        for (id, prov, project, version, title, fname, sha) in add {
            tx.execute("INSERT INTO content_items(id,server_id,provider,project_id,version_id,title,version_number,filename,kind,sha512,installed_at,pinned,dir) VALUES (?,?,?,?,?,?,?,?,?,?,?,0,?)",
                rusqlite::params![id, sid, prov, project, version, title, fname, fname, kind, sha, now, dir])?;
            tx.execute("INSERT INTO modpack_items(content_item_id, install_id) VALUES (?,?)", rusqlite::params![id, install])?;
        }
        for (path, sum) in baseline { tx.execute("INSERT OR REPLACE INTO modpack_files(install_id,path,sha256) VALUES (?,?,?)", rusqlite::params![install, path, sum])?; }
        tx.execute("UPDATE modpack_installs SET name=?, version=?, game_version=?, loader=?, installed_at=?, files=? WHERE id=?", rusqlite::params![name, version, gv, loader, now, n_files, install])?;
        if let Some(i) = identity { tx.execute("UPDATE modpack_installs SET provider=?, project_id=?, version_id=? WHERE id=?", rusqlite::params![i.provider, i.project_id, i.version_id, install])?; }
        tx.commit()
    }).await?;
    Ok(())
}

/// What a staged test of a pack update would change on `s`, for the run report: (signature, changes). Refuses when
/// there is nothing to change or the update needs the upgrade assistant first.
pub async fn staged_plan(st: &St, utoken: Option<&str>, s: &Srv, c: &Compat, install: &str, path: &FsPath) -> Result<(String, Vec<Value>)> {
    let row = install_row(st, &s.id, install).await.map_err(|e| anyhow!(e.message))?;
    if row.pinned { bail!("this pack is pinned to its current version; unpin it first"); }
    let p = plan(st, utoken, s, c, &row, path).await?;
    if !p.diff.blockers.is_empty() { bail!("{}", p.diff.blockers.join(" ")); }
    if !p.diff.changes_anything() { bail!("There are no pack changes to try."); }
    let f = &p.diff.files;
    let mut changes: Vec<Value> = vec![];
    for (action, list) in [("add", &f.added), ("update", &f.updated), ("remove", &f.removed)] {
        for ch in list { changes.push(json!({"title": ch.title, "action": action, "to": ch.to})); }
    }
    for path in p.diff.overrides.added.iter().chain(p.diff.overrides.updated.iter()) { changes.push(json!({"title": path, "action": "config", "to": null})); }
    Ok((p.diff.signature(&p.to_version), changes))
}

#[derive(Deserialize)] pub struct UpdateReq { token: String }

/// `POST /servers/{id}/modpacks/{install}/update`: apply the staged newer pack version.
pub async fn update_start(State(st): State<St>, user: User, Path((id, install)): Path<(String, String)>, Json(r): Json<UpdateReq>) -> ApiResult<(StatusCode, Json<Value>)> {
    let (s, c) = compat_for(&st, &user, &id, "content.manage").await?;
    let row = install_row(&st, &id, &install).await?;
    if row.pinned { return Err(ApiError::conflict("modpack.pinned", "this pack is pinned to its current version; unpin it to update")); }
    let entry = store::get(&r.token, &user.id).ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "modpack.token_expired", "this modpack token is unknown or has expired (tokens last 30 minutes); check for the update again"))?;
    if entry.runtime != s.runtime { return Err(ApiError::conflict("modpack.runtime_mismatch", format!("this pack needs {} but the server runs {}", entry.runtime, s.runtime))); }
    let (utoken, actor, title) = (user.token.clone(), user.actor(), format!("Update {} on {}", row.name, s.name));
    let op = ops::spawn_op(&st, "modpack.update", title, Some(&id), true, move |ctx| async move {
        apply_update(&ctx, &s, &c, &row.id, &entry.path, utoken.as_deref(), &actor, None, true, entry.source.clone()).await?;
        store::consume(&entry.token);
        Ok(())
    });
    Ok((StatusCode::ACCEPTED, Json(json!({"operation": op}))))
}

#[cfg(test)]
mod tests;
