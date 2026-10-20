//! Install pipeline for server records: resolve the runtime, download the server jar (or run the official
//! installer for Forge, NeoForge and Quilt), write first-run configuration. `replace_runtime` is the keep-data
//! flavour used by the version upgrade assistant.
use crate::{download, ops::OpCtx, runtimes::{self, Resolved, modded::is_modded}, server::{self, Srv}, state::St};
use anyhow::{Result, anyhow};
use std::path::Path;

mod modded;

/// Install pipeline for a freshly created server record. Same pipeline for the wizard, API and CLI.
pub async fn install(ctx: OpCtx, id: String) -> Result<()> {
    let st: St = ctx.st.clone();
    let s = server::get(&st, &id).await.map_err(|e| anyhow::anyhow!(e.message))?;
    let res = run(&ctx, &st, &s).await;
    let (i, err) = (id.clone(), res.as_ref().err().map(|e| format!("{e:#}")));
    let ok = res.is_ok();
    st.db.call(move |c| c.execute("UPDATE servers SET installed=?, install_error=? WHERE id=?", rusqlite::params![ok, err, i])).await?;
    if ok { st.timeline(&id, "install", &format!("{} {}", s.runtime, s.game_version)).await; }
    if let Ok(fresh) = server::get(&st, &id).await { server::note_state(&st, &fresh); }
    res
}

async fn run(ctx: &OpCtx, st: &St, s: &Srv) -> Result<()> {
    let dir = s.dir(st);
    tokio::fs::create_dir_all(dir.join(".moss")).await?;
    if matches!(s.runtime.as_str(), "sim" | "custom") {
        ctx.stage("Preparing directory");
        return Ok(());
    }
    ctx.stage(&format!("Resolving {} {}", s.runtime, s.game_version));
    let r = runtimes::resolve(st, &s.runtime, &s.game_version, s.loader_version.as_deref()).await?;
    ctx.check()?;
    {
        let (i, java, lv) = (s.id.clone(), r.java_major as i64, r.loader_version.clone());
        st.db.call(move |c| c.execute("UPDATE servers SET java_major=?, loader_version=COALESCE(?,loader_version) WHERE id=?", rusqlite::params![java, lv, i])).await?;
    }
    if crate::java::find(st, r.java_major).is_none() && !is_modded(&s.runtime) {
        ctx.log(format!("Java {} is not installed — install it from Settings → Java before starting.", r.java_major));
    }
    install_files(ctx, st, s, &r, &dir).await?;
    ctx.stage("Writing configuration");
    if s.is_mc() && !s.is_proxy() {
        if s.eula_at.is_some() {
            tokio::fs::write(dir.join("eula.txt"), "# Accepted via Moss by the server owner (https://aka.ms/MinecraftEULA)\neula=true\n").await?;
        }
        tokio::fs::write(dir.join("server.properties"), format!("server-port={}\nmotd=A Moss server\nenable-query=false\n", s.port)).await?;
    }
    Ok(())
}

/// Put the runtime's files into `dir`: the server jar, or the output of the official installer for modded loaders.
/// `s` carries the target game version and loader; `r` is what `runtimes::resolve` returned for it.
pub async fn install_files(ctx: &OpCtx, st: &St, s: &Srv, r: &Resolved, dir: &Path) -> Result<()> {
    if is_modded(&s.runtime) {
        let mut target = s.clone();
        target.loader_version = r.loader_version.clone().or_else(|| s.loader_version.clone());
        return modded::install(ctx, st, &target, r, dir).await;
    }
    ctx.stage(if r.hash.is_some() { "Downloading server.jar (checksum-verified)" } else { "Downloading server.jar (upstream publishes no checksum)" });
    let c2 = ctx.clone();
    let sum = download::fetch(&st.http, &r.url, &dir.join("server.jar"), r.hash.clone(), &ctx.token, move |p| c2.progress(Some(p * 0.95))).await?;
    ctx.log(format!("sha256 {sum}"));
    Ok(())
}

/// Keep-data reinstall for the upgrade assistant: the previous runtime files are moved to `.moss/upgrade-prev`,
/// the new ones installed, and on any failure the previous files are put back exactly as they were.
pub async fn replace_runtime(ctx: &OpCtx, st: &St, target: &Srv, r: &Resolved, dir: &Path) -> Result<()> {
    let prev = dir.join(".moss").join("upgrade-prev");
    let (d, p, rt) = (dir.to_path_buf(), prev.clone(), target.runtime.clone());
    let moved = tokio::task::spawn_blocking(move || modded::stash(&d, &p, &rt)).await?.map_err(|e| anyhow!("could not set the current server files aside: {e}"))?;
    match install_files(ctx, st, target, r, dir).await {
        Ok(()) => { let _ = tokio::fs::remove_dir_all(&prev).await; Ok(()) }
        Err(e) => {
            let (d, p) = (dir.to_path_buf(), prev.clone());
            let restored = tokio::task::spawn_blocking(move || modded::unstash(&d, &p, &moved)).await;
            ctx.log(match restored { Ok(Ok(())) => "Restored the previous server files.".to_string(), _ => format!("Could not restore every previous file; they are kept in {}", prev.display()) });
            Err(e)
        }
    }
}
