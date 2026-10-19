use crate::{download, ops::OpCtx, runtimes, server::{self, Srv}, state::St};
use anyhow::Result;

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
    if crate::java::find(st, r.java_major).is_none() {
        ctx.log(format!("Java {} is not installed — install it from Settings → Java before starting.", r.java_major));
    }
    ctx.stage(if r.hash.is_some() { "Downloading server.jar (checksum-verified)" } else { "Downloading server.jar (upstream publishes no checksum)" });
    let c2 = ctx.clone();
    let sum = download::fetch(&st.http, &r.url, &dir.join("server.jar"), r.hash, &ctx.token, move |p| c2.progress(Some(p * 0.95))).await?;
    ctx.log(format!("sha256 {sum}"));
    ctx.stage("Writing configuration");
    if s.is_mc() && !s.is_proxy() {
        if s.eula_at.is_some() {
            tokio::fs::write(dir.join("eula.txt"), "# Accepted via Moss by the server owner (https://aka.ms/MinecraftEULA)\neula=true\n").await?;
        }
        tokio::fs::write(dir.join("server.properties"), format!("server-port={}\nmotd=A Moss server\nenable-query=false\n", s.port)).await?;
    }
    Ok(())
}
