//! `POST /servers/{id}/actions/reinstall`: re-run the installer for a stopped (or failed-to-install) server.
use super::*;
use crate::ops::OpCtx;
use std::path::Path as FsPath;

#[derive(Deserialize)]
pub(super) struct ReinstallReq {
    /// `true` (the default) only re-downloads the server jar/loader and keeps worlds, configuration and plugins.
    #[serde(default = "yes")]
    keep_data: bool,
}
fn yes() -> bool { true }

/// Files the installer would otherwise overwrite with defaults; preserved across a `keep_data` reinstall.
const KEEP: [&str; 2] = ["server.properties", "eula.txt"];

pub(super) async fn server_reinstall(State(st): State<St>, user: User, Path(id): Path<String>, Json(r): Json<ReinstallReq>) -> ApiResult<(StatusCode, Json<Value>)> {
    authz(&st, &user, &id, "config.write").await?;
    let s = server::get(&st, &id).await?;
    let (state, _) = server::state_of(&s, &st);
    if !matches!(state.as_str(), "stopped" | "crashed" | "failed" | "needs_attention") {
        return Err(ApiError::conflict("server.not_stopped", format!("a server that is {state} cannot be reinstalled; stop it first")));
    }
    if lifecycle::process_alive(&s) { return Err(ApiError::conflict("server.running", "stop the server before reinstalling it")); }
    let guard = st.lock_for(&id).try_lock_owned().map_err(|_| ApiError::conflict("server.locked", "another operation is changing this server"))?;
    st.audit(&user.actor(), "server.reinstall", &s.name, if r.keep_data { "keeping data" } else { "data wiped" }, Some(&id)).await;
    let (id2, keep) = (id.clone(), r.keep_data);
    let op = ops::spawn_op(&st, "server.reinstall", format!("Reinstall {}", s.name), Some(&id), true, move |ctx| async move {
        let _guard = guard;
        run(ctx, id2, keep).await
    });
    Ok((StatusCode::ACCEPTED, Json(json!({"operation": op}))))
}

async fn run(ctx: OpCtx, id: String, keep_data: bool) -> anyhow::Result<()> {
    let st = ctx.st.clone();
    let s = server::get(&st, &id).await.map_err(|e| anyhow::anyhow!(e.message))?;
    let dir = s.dir(&st);
    let kept = if keep_data { snapshot(&dir).await } else { vec![] };
    if !keep_data {
        ctx.stage("Removing the old server files");
        wipe(&dir).await?;
    }
    let i = id.clone();
    st.db.call(move |c| c.execute("UPDATE servers SET installed=0, install_error=NULL WHERE id=?", [i])).await?;
    server::emit(&st, &id).await;
    let res = crate::installer::install(ctx.clone(), id.clone()).await;
    for (name, bytes) in kept { let _ = tokio::fs::write(dir.join(name), bytes).await; }
    res
}

async fn snapshot(dir: &FsPath) -> Vec<(&'static str, Vec<u8>)> {
    let mut v = vec![];
    for n in KEEP { if let Ok(b) = tokio::fs::read(dir.join(n)).await { v.push((n, b)); } }
    v
}

/// Remove everything in the server directory except Moss's own `.moss` folder (console log, console FIFO).
async fn wipe(dir: &FsPath) -> std::io::Result<()> {
    let Ok(mut rd) = tokio::fs::read_dir(dir).await else { return Ok(()) };
    while let Some(e) = rd.next_entry().await? {
        if e.file_name() == ".moss" { continue; }
        if e.file_type().await?.is_dir() { tokio::fs::remove_dir_all(e.path()).await?; } else { tokio::fs::remove_file(e.path()).await?; }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn wipe_keeps_only_moss_dir_and_snapshot_restores_properties() {
        let d = std::env::temp_dir().join(format!("moss-reinstall-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(d.join(".moss")).unwrap();
        std::fs::create_dir_all(d.join("world")).unwrap();
        std::fs::write(d.join("world/level.dat"), "x").unwrap();
        std::fs::write(d.join("server.properties"), "motd=hi").unwrap();
        std::fs::write(d.join(".moss/console.log"), "log").unwrap();
        let kept = snapshot(&d).await;
        assert_eq!(kept.len(), 1);
        wipe(&d).await.unwrap();
        let left: Vec<_> = std::fs::read_dir(&d).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(left, vec![".moss".to_string()]);
        let _ = std::fs::remove_dir_all(&d);
    }
}
