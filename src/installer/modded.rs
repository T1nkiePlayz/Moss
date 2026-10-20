//! Running the official Forge/NeoForge/Quilt installer jar in a server directory, plus the stash/restore
//! helpers that make a keep-data reinstall atomic. The installer process is sandboxed as far as a plain child
//! can be: environment cleared, HOME = the server directory, hard timeout, cancellable, output streamed.
use crate::{download, ops::OpCtx, platform, runtimes::{Resolved, launch}, server::Srv, state::St};
use anyhow::{Result, anyhow, bail};
use std::{collections::HashSet, path::Path, time::Duration};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio_util::sync::CancellationToken;

pub const TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// argv of the installer for each runtime (`jar` is the downloaded installer, the working directory the server dir).
pub fn installer_argv(java: &str, jar: &str, s: &Srv) -> Vec<String> {
    let mut a = vec![java.to_string(), "-jar".into(), jar.to_string()];
    if s.runtime == "quilt" {
        a.extend(["install".into(), "server".into(), s.game_version.clone()]);
        if let Some(l) = s.loader_version.as_deref().filter(|l| !l.is_empty()) { a.push(l.to_string()); }
        a.extend(["--download-server".into(), "--install-dir=.".into()]);
    } else {
        a.push("--installServer".into());
    }
    a
}

/// Run `argv` in `dir` with a scrubbed environment, streaming every output line to `log`.
pub async fn run_streamed(argv: &[String], dir: &Path, timeout: Duration, token: &CancellationToken, log: impl Fn(String) + Send + Sync + Clone + 'static) -> Result<()> {
    let mut cmd = tokio::process::Command::new(&argv[0]);
    cmd.args(&argv[1..]).current_dir(dir).env_clear().env("PATH", platform::GAME_PATH).env("HOME", dir).env("LANG", "C.UTF-8")
        .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).kill_on_drop(true).process_group(0);
    let mut child = cmd.spawn().map_err(|e| anyhow!("could not start {}: {e}", argv[0]))?;
    let pump = |r: Option<Box<dyn tokio::io::AsyncRead + Unpin + Send>>| {
        let log = log.clone();
        tokio::spawn(async move {
            if let Some(r) = r {
                let mut lines = BufReader::new(r).lines();
                while let Ok(Some(l)) = lines.next_line().await { log(l.chars().filter(|c| !c.is_control()).take(400).collect()); }
            }
        })
    };
    let pumps = [pump(child.stdout.take().map(|o| Box::new(o) as _)), pump(child.stderr.take().map(|o| Box::new(o) as _))];
    let status = tokio::select! {
        s = child.wait() => Some(s?),
        _ = tokio::time::sleep(timeout) => None,
        _ = token.cancelled() => None,
    };
    let Some(status) = status else {
        let _ = child.kill().await;
        for p in pumps { p.abort(); }
        if token.is_cancelled() { bail!("cancelled"); }
        bail!("the installer did not finish within {} minutes and was stopped", timeout.as_secs() / 60);
    };
    for p in pumps { let _ = p.await; }
    if !status.success() { bail!("the installer failed ({status})"); }
    Ok(())
}

fn top_level(dir: &Path) -> HashSet<String> {
    std::fs::read_dir(dir).map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect()).unwrap_or_default()
}

/// Remove everything the installer added to the top level of `dir` (never `.moss`).
fn cleanup_new(dir: &Path, before: &HashSet<String>) {
    for n in top_level(dir).difference(before).filter(|n| *n != ".moss") {
        let p = dir.join(n);
        let _ = if p.is_dir() && !p.is_symlink() { std::fs::remove_dir_all(&p) } else { std::fs::remove_file(&p) };
    }
}

/// Download the official installer, run it and verify it produced a launchable server. Cleans up on failure.
pub async fn install(ctx: &OpCtx, st: &St, s: &Srv, r: &Resolved, dir: &Path) -> Result<()> {
    let java = crate::java::find(st, r.java_major).ok_or_else(|| anyhow!("Java {} is needed to run the {} installer. Install it from Settings → Java, then retry.", r.java_major, s.runtime))?;
    let tmp = st.cfg.tmp_dir().join(format!("installer-{}", uuid::Uuid::now_v7()));
    let jar = tmp.join("installer.jar");
    ctx.stage(if r.hash.is_some() { "Downloading the installer (checksum-verified)" } else { "Downloading the installer (upstream publishes no checksum)" });
    let c2 = ctx.clone();
    let res = async {
        download::fetch(&st.http, &r.url, &jar, r.hash.clone(), &ctx.token, move |p| c2.progress(Some(p * 0.1))).await?;
        ctx.check()?;
        ctx.stage(&format!("Running the {} installer (this can take several minutes)", s.runtime));
        let before = top_level(dir);
        let argv = installer_argv(&java.to_string_lossy(), &jar.to_string_lossy(), s);
        let c3 = ctx.clone();
        let out = run_streamed(&argv, dir, TIMEOUT, &ctx.token, move |l| c3.log(l)).await.and_then(|()| verify(dir, s));
        if out.is_err() { cleanup_new(dir, &before); }
        out
    }.await;
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    res?;
    launch::write_user_jvm_args(dir, s)?;
    Ok(())
}

fn verify(dir: &Path, s: &Srv) -> Result<()> {
    let want = if s.runtime == "quilt" { Some("quilt-server-launch.jar".to_string()) } else { launch::unix_args_path(s) };
    match want { Some(w) if dir.join(&w).is_file() => Ok(()), Some(w) => bail!("the installer finished but did not create {w}"), None => bail!("no loader version recorded") }
}

/// Top-level names a runtime's install owns, so a keep-data reinstall can set them aside.
fn owned_names(dir: &Path, runtime: &str) -> Vec<String> {
    let mut v: Vec<String> = match runtime {
        "forge" | "neoforge" => ["libraries", "run.sh", "run.bat", "user_jvm_args.txt", "installer.log"].map(String::from).into(),
        "quilt" => ["quilt-server-launch.jar", "libraries", "server", "installer.log"].map(String::from).into(),
        _ => vec!["server.jar".into()],
    };
    v.extend(top_level(dir).into_iter().filter(|n| n.starts_with("minecraft_server.") && n.ends_with(".jar")));
    v
}

/// Move the runtime's current files into `prev`; returns the names moved.
pub fn stash(dir: &Path, prev: &Path, runtime: &str) -> std::io::Result<Vec<String>> {
    let _ = std::fs::remove_dir_all(prev);
    std::fs::create_dir_all(prev)?;
    let mut moved = vec![];
    for n in owned_names(dir, runtime) {
        let from = dir.join(&n);
        if from.symlink_metadata().is_ok() { std::fs::rename(&from, prev.join(&n))?; moved.push(n); }
    }
    Ok(moved)
}

/// Undo `stash`: whatever a failed install left under those names is removed, the originals move back.
pub fn unstash(dir: &Path, prev: &Path, moved: &[String]) -> std::io::Result<()> {
    for n in moved {
        let to = dir.join(n);
        if let Ok(m) = to.symlink_metadata() { if m.is_dir() { std::fs::remove_dir_all(&to)?; } else { std::fs::remove_file(&to)?; } }
        std::fs::rename(prev.join(n), &to)?;
    }
    let _ = std::fs::remove_dir_all(prev);
    Ok(())
}

#[cfg(test)]
mod tests;
