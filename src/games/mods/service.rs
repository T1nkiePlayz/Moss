//! Install, remove, enable/disable and update-check for game mods, on top of the provider modules. These functions
//! work on a server directory only; permission, "server stopped" and locking checks belong to the API handlers.
use super::{Entry, Kind, Manifest, Providers, cmp_version, factorio, net, thunderstore as ts};
use crate::{db::now, download::Hash, error::{ApiError, ApiResult}, files::safe_join, games::read_settings, ops::OpCtx};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::path::Path;

/// The Factorio portal credentials from moss-game.cfg, when both are set.
pub fn factorio_credentials(dir: &Path) -> Option<(String, String)> {
    let cfg = read_settings(dir);
    let get = |k: &str| cfg.get(k).filter(|v| !v.is_empty()).cloned();
    Some((get("factorio_username")?, get("factorio_token")?))
}

pub fn valid_id(kind: Kind, id: &str) -> bool {
    match kind { Kind::Thunderstore => ts::split_id(id).is_some(), Kind::Factorio => factorio::valid_name(id) && !factorio::builtin(id), Kind::Workshop => false }
}

pub fn installed_json(dir: &Path, kind: Kind) -> ApiResult<Vec<Value>> {
    Ok(Manifest::load(dir)?.mods.into_iter().map(|e| {
        let url = match (kind, ts::split_id(&e.id)) {
            (Kind::Thunderstore, Some((ns, n))) => format!("https://thunderstore.io/c/valheim/p/{ns}/{n}/"),
            _ => format!("https://mods.factorio.com/mod/{}", e.id.replace(' ', "%20")),
        };
        json!({"id": e.id, "title": e.title, "version": e.version, "enabled": e.enabled, "auto": e.auto, "requires": e.requires, "url": url, "installed_at": e.installed_at})
    }).collect())
}

pub async fn search(kind: Kind, p: &Providers, q: &str) -> Result<Vec<Value>> {
    match kind {
        Kind::Thunderstore => ts::search(&net::client(net::THUNDERSTORE_HOSTS, p), p, q).await,
        Kind::Factorio => factorio::search(&net::client(net::FACTORIO_HOSTS, p), p, q).await,
        Kind::Workshop => bail!("Steam Workshop content is chosen by id, not searched"),
    }
}

pub async fn install(ctx: &OpCtx, p: &Providers, dir: &Path, tmp: &Path, kind: Kind, id: &str, version: Option<&str>) -> Result<Vec<String>> {
    if !valid_id(kind, id) { bail!("invalid mod id"); }
    tokio::fs::create_dir_all(dir).await?;
    match kind {
        Kind::Thunderstore => install_ts(ctx, p, dir, tmp, id, version).await,
        Kind::Factorio => install_factorio(ctx, p, dir, id, version).await,
        Kind::Workshop => bail!("Steam Workshop content is not installed by Moss"),
    }
}

fn remove_files(dir: &Path, files: &[String]) {
    for f in files {
        let Ok(path) = safe_join(dir, f) else { continue };
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}.old", path.display()));
        let mut rel = Path::new(f).parent();
        while let Some(r) = rel.filter(|r| r.components().count() >= 3) {
            if std::fs::remove_dir(dir.join(r)).is_err() { break; }
            rel = r.parent();
        }
    }
}

async fn install_ts(ctx: &OpCtx, p: &Providers, dir: &Path, tmp: &Path, id: &str, version: Option<&str>) -> Result<Vec<String>> {
    let (ns, name) = ts::split_id(id).context("invalid mod id")?;
    let http = net::client(net::THUNDERSTORE_HOSTS, p);
    ctx.stage("Resolving dependencies");
    let root = ts::fetch_version(&http, p, ns, name, version).await?;
    let mut pkgs = ts::resolve(&http, p, root.clone()).await?;
    let mut manifest = am(Manifest::load(dir))?;
    if manifest.get(ts::BEPINEX_ID).is_none() && !pkgs.iter().any(|x| x.id() == ts::BEPINEX_ID) {
        pkgs.insert(0, ts::fetch_version(&http, p, "denikson", "BepInExPack_Valheim", None).await?);
    }
    let root_id = root.id();
    let mut done = vec![];
    tokio::fs::create_dir_all(tmp).await?;
    for pkg in pkgs {
        let pid = pkg.id();
        let is_root = pid == root_id;
        if !is_root && manifest.get(&pid).is_some_and(|e| cmp_version(&e.version, &pkg.version).is_ge()) { continue; }
        ctx.check()?;
        ctx.stage(&format!("Installing {pid} {}", pkg.version));
        let zip = tmp.join(format!("{}.zip", uuid::Uuid::now_v7()));
        net::download(&http, &net::Fetch { url: &pkg.download_url, dest: &zip, expect: None, hosts: net::THUNDERSTORE_HOSTS }, p, &ctx.token, |f| ctx.progress(Some(f))).await?;
        let old = manifest.get(&pid).cloned();
        if let Some(o) = &old { remove_files(dir, &o.files); }
        let (z, d, pid2) = (zip.clone(), dir.to_path_buf(), pid.clone());
        let files = tokio::task::spawn_blocking(move || net::extract_zip(&z, &d, &|e| ts::map_entry(&pid2, e, &|r| d.join(r).exists()))).await?;
        let _ = tokio::fs::remove_file(&zip).await;
        let files = files.with_context(|| format!("could not unpack {pid}"))?;
        let mut requires: Vec<String> = pkg.deps.iter().map(|d| d.id()).collect();
        if pid != ts::BEPINEX_ID && !requires.iter().any(|r| r == ts::BEPINEX_ID) { requires.push(ts::BEPINEX_ID.into()); }
        manifest.upsert(Entry {
            id: pid.clone(), title: pkg.name.replace('_', " "), version: pkg.version.clone(), enabled: true, auto: !is_root && old.as_ref().is_none_or(|o| o.auto),
            requires, conflicts: vec![], files, installed_at: now(),
        });
        am(manifest.save(dir))?;
        done.push(pid);
    }
    Ok(done)
}

async fn install_factorio(ctx: &OpCtx, p: &Providers, dir: &Path, id: &str, version: Option<&str>) -> Result<Vec<String>> {
    let (user, token) = factorio_credentials(dir).context("set factorio_username and factorio_token first (see the Mods tab)")?;
    let http = net::client(net::FACTORIO_HOSTS, p);
    let mut manifest = am(Manifest::load(dir))?;
    ctx.stage("Resolving dependencies");
    let chosen = factorio::resolve(&http, p, id, version, &manifest.mods).await?;
    let mut done = vec![];
    for c in chosen {
        ctx.check()?;
        ctx.stage(&format!("Installing {} {}", c.name, c.release.version));
        let rel = format!("mods/{}", c.release.file_name);
        let dest = safe_join(dir, &rel).map_err(|e| anyhow::anyhow!(e.message))?;
        let url = factorio::download_url(p, &c.release, &user, &token)?;
        let sha = Hash::Sha1(c.release.sha1.clone());
        net::download(&http, &net::Fetch { url: &url, dest: &dest, expect: Some(&sha), hosts: net::FACTORIO_HOSTS }, p, &ctx.token, |f| ctx.progress(Some(f))).await?;
        let old = manifest.get(&c.name).cloned();
        if let Some(o) = old.as_ref() { remove_files(dir, &o.files.iter().filter(|f| **f != rel).cloned().collect::<Vec<_>>()); }
        let dep_names = |k: factorio::DepKind| c.release.deps.iter().filter(|d| d.kind == k && !factorio::builtin(&d.name)).map(|d| d.name.clone()).collect::<Vec<_>>();
        factorio::modlist_set(dir, &c.name, true)?;
        manifest.upsert(Entry {
            id: c.name.clone(), title: c.title.clone(), version: c.release.version.clone(), enabled: true, auto: c.name != id && old.as_ref().is_none_or(|o| o.auto),
            requires: dep_names(factorio::DepKind::Required), conflicts: dep_names(factorio::DepKind::Incompatible), files: vec![rel], installed_at: now(),
        });
        am(manifest.save(dir))?;
        done.push(c.name);
    }
    Ok(done)
}

fn modlist_err(e: anyhow::Error) -> ApiError { ApiError::conflict("game_mods.modlist", format!("{e:#}")) }

pub fn remove(dir: &Path, kind: Kind, id: &str) -> ApiResult<()> {
    let mut m = Manifest::load(dir)?;
    let e = m.get(id).cloned().ok_or_else(|| ApiError::not_found("mod"))?;
    let by = m.dependents(id);
    if !by.is_empty() { return Err(ApiError::conflict("game_mods.required", format!("{} is required by {}; remove those first", e.title, by.join(", ")))); }
    remove_files(dir, &e.files);
    if kind == Kind::Factorio { factorio::modlist_remove(dir, id).map_err(modlist_err)?; }
    m.remove(id);
    m.save(dir)
}

pub fn set_enabled(dir: &Path, kind: Kind, id: &str, enabled: bool) -> ApiResult<()> {
    let mut m = Manifest::load(dir)?;
    let e = m.get(id).cloned().ok_or_else(|| ApiError::not_found("mod"))?;
    if id == ts::BEPINEX_ID { return Err(ApiError::bad("game_mods.core", "BepInEx is what loads every other mod; remove it instead of disabling it")); }
    if e.enabled == enabled { return Ok(()); }
    match kind {
        Kind::Factorio => factorio::modlist_set(dir, id, enabled).map_err(modlist_err)?,
        _ => for f in e.files.iter().filter(|f| ts::disables(f)) {
            let Ok(path) = safe_join(dir, f) else { continue };
            let old = format!("{}.old", path.display());
            let (from, to) = if enabled { (old, path.display().to_string()) } else { (path.display().to_string(), old) };
            match std::fs::rename(&from, &to) { Ok(()) => {} Err(e) if e.kind() == std::io::ErrorKind::NotFound => {} Err(e) => return Err(e.into()) }
        },
    }
    if let Some(x) = m.mods.iter_mut().find(|x| x.id == id) { x.enabled = enabled; }
    m.save(dir)
}

/// Installed mods with a newer release on the provider: `{id, installed, latest}`. Mods whose lookup fails are left out.
pub async fn check_updates(p: &Providers, dir: &Path, kind: Kind) -> ApiResult<Vec<Value>> {
    let m = Manifest::load(dir)?;
    let mut out = vec![];
    for e in &m.mods {
        let latest = match kind {
            Kind::Thunderstore => {
                let Some((ns, n)) = ts::split_id(&e.id) else { continue };
                ts::fetch_version(&net::client(net::THUNDERSTORE_HOSTS, p), p, ns, n, None).await.ok().map(|x| x.version)
            }
            Kind::Factorio => factorio::fetch_mod(&net::client(net::FACTORIO_HOSTS, p), p, &e.id).await.ok().flatten()
                .and_then(|x| x.releases.into_iter().map(|r| r.version).max_by(|a, b| cmp_version(a, b))),
            Kind::Workshop => None,
        };
        if let Some(l) = latest.filter(|l| cmp_version(l, &e.version).is_gt()) { out.push(json!({"id": e.id, "installed": e.version, "latest": l})); }
    }
    Ok(out)
}

fn am<T>(r: ApiResult<T>) -> Result<T> { r.map_err(|e| anyhow::anyhow!(e.message)) }
