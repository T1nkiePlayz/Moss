//! The test operation: copy, apply the change to the copy, start it, wait for readiness, watch the log, report.
use super::*;
use super::{clone, health};
use crate::content::{update_all, upgrade};
use std::time::{Duration, Instant};

/// What the person asked for; stored with the run so promote and the UI can see exactly what was tested.
#[derive(Clone, Deserialize, serde::Serialize)]
pub struct Req {
    pub kind: String,
    #[serde(default)] pub include_world: bool,
    #[serde(default)] pub ready_timeout_s: Option<u64>,
    #[serde(default)] pub watch_s: Option<u64>,
    #[serde(default)] pub upgrade: Option<upgrade::Target>,
}

impl Req {
    pub fn ready_timeout(&self) -> u64 { self.ready_timeout_s.unwrap_or(180).clamp(10, 900) }
    pub fn watch(&self) -> u64 { self.watch_s.unwrap_or(30).min(300) }
}

pub(super) const KINDS: &[&str] = &["none", "update_all", "upgrade"];

/// Cloned servers carry their own tests' side effects, so everything below works on the copy's own row and directory.
pub(super) async fn create_copy(st: &St, origin: &Srv, run_id: &str, req: &Req, port: i64, actor: &str) -> anyhow::Result<()> {
    let (o, i, r, a, now) = (origin.id.clone(), run_id.to_string(), serde_json::to_string(req)?, actor.to_string(), crate::db::now());
    let (name, world, kind) = (format!("{} (staging)", origin.name), req.include_world, req.kind.clone());
    let exp = (chrono::Utc::now() + chrono::Duration::hours(KEEP_HOURS)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    st.db.call(move |c| {
        let tx = c.unchecked_transaction()?;
        tx.execute("INSERT INTO staging_runs(id,origin_id,kind,request,include_world,state,report,port,actor,created_at,expires_at) VALUES (?,?,?,?,?,'building','{}',?,?,?,?)",
            rusqlite::params![i, o, kind, r, world, port, a, now, exp])?;
        tx.execute(
            "INSERT INTO servers(id,node_id,name,runtime,game_version,loader_version,java_major,port,memory_mb,flags_profile,autostart,restart_enabled,restart_max,eula_at,eula_by,desired,installed,tags,notes,custom_cmd,created_at,memory_limit_mb,cpu_limit_pct,pids_limit)
             SELECT ?,node_id,?,runtime,game_version,loader_version,java_major,?,memory_mb,flags_profile,0,0,restart_max,eula_at,eula_by,'stopped',1,'[]','Temporary copy used to test a change. Moss removes it automatically.',custom_cmd,?,memory_limit_mb,cpu_limit_pct,pids_limit FROM servers WHERE id=?",
            rusqlite::params![i, name, port, now, o])?;
        let ids: Vec<String> = { let mut q = tx.prepare("SELECT id FROM content_items WHERE server_id=?")?; q.query_map([&o], |r| r.get(0))?.collect::<Result<_, _>>()? };
        for old in ids {
            tx.execute("INSERT INTO content_items(id,server_id,provider,project_id,version_id,title,version_number,filename,kind,sha512,installed_at,pinned,dir,source)
                        SELECT ?,?,provider,project_id,version_id,title,version_number,filename,kind,sha512,installed_at,pinned,dir,source FROM content_items WHERE id=?",
                rusqlite::params![uuid::Uuid::now_v7().to_string(), i, old])?;
        }
        tx.commit()
    }).await?;
    Ok(())
}

struct Watched { ready_secs: Option<f64>, crashed: bool, exited_early: bool }

async fn watch(ctx: &OpCtx, st: &St, id: &str, ready_timeout: u64, watch_s: u64) -> anyhow::Result<Watched> {
    let t0 = Instant::now();
    let mut ready_at: Option<Instant> = None;
    loop {
        ctx.check()?;
        let (alive, ready, crashed) = { let l = st.live.lock().unwrap(); l.get(id).map_or((false, false, false), |l| (l.alive, l.ready, l.crashed)) };
        if ready && ready_at.is_none() { ready_at = Some(Instant::now()); ctx.stage("Watching the copy for problems"); }
        if !alive && t0.elapsed() > Duration::from_millis(800) {
            return Ok(Watched { ready_secs: ready_at.map(|t| (t - t0).as_secs_f64()), crashed, exited_early: true });
        }
        match ready_at {
            Some(t) if t.elapsed() >= Duration::from_secs(watch_s) => return Ok(Watched { ready_secs: Some((t - t0).as_secs_f64()), crashed: false, exited_early: false }),
            None if t0.elapsed() >= Duration::from_secs(ready_timeout) => return Ok(Watched { ready_secs: None, crashed: false, exited_early: false }),
            _ => {}
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
}

pub(super) fn console_text(st: &St, s: &Srv) -> String {
    let mdir = s.moss_dir(st);
    let tail = |p: std::path::PathBuf| -> String {
        let Ok(b) = std::fs::read(p) else { return String::new() };
        let from = b.len().saturating_sub(2 << 20);
        String::from_utf8_lossy(&b[from..]).into_owned()
    };
    tail(mdir.join("console.log"))
}

/// What the plan would change, as shown in the report.
async fn plan_for(st: &St, origin: &Srv, req: &Req, token: Option<&str>) -> anyhow::Result<(Option<String>, Vec<Value>)> {
    match req.kind.as_str() {
        "update_all" => {
            let c = compat(st, origin).ok_or_else(|| anyhow!("this runtime has no managed content"))?;
            let (steps, _w) = update_all::build_all(st, origin, &c, token).await?;
            if steps.is_empty() { bail!("There are no content updates to try."); }
            let changes = steps.iter().map(|s| json!({"title": s.title, "action": s.action, "to": s.version["version_number"]})).collect();
            Ok((Some(update_all::plan_signature(&steps)), changes))
        }
        "upgrade" => {
            let t = req.upgrade.as_ref().ok_or_else(|| anyhow!("upgrade target missing"))?;
            let c = compat(st, origin).ok_or_else(|| anyhow!("this runtime has no managed content"))?;
            let sig = upgrade::upgrade_signature(st, token, origin, &c, t).await?;
            Ok((Some(sig), vec![json!({"title": format!("{} {}", origin.runtime, origin.game_version), "action": "upgrade", "to": t.game_version})]))
        }
        _ => Ok((None, vec![])),
    }
}

pub(super) async fn test(ctx: OpCtx, run: String, origin: Srv, req: Req, token: Option<String>, actor: String) -> anyhow::Result<()> {
    let st = ctx.st.clone();
    let res = test_inner(&ctx, &run, &origin, &req, token.as_deref(), &actor).await;
    if let Err(e) = res {
        if let Ok(s) = server::get(&st, &run).await { lifecycle::halt(&st, &s).await; }
        if ctx.cancelled() { let _ = discard(&st, &run).await; return Err(e); }
        let msg = format!("{e:#}");
        set_state(&st, &run, "failed", Some(json!({"healthy": false, "error": msg, "reasons": [msg]}))).await;
        return Err(e);
    }
    Ok(())
}

async fn test_inner(ctx: &OpCtx, run: &str, origin: &Srv, req: &Req, token: Option<&str>, actor: &str) -> anyhow::Result<()> {
    let st = ctx.st.clone();
    ctx.stage("Working out what would change");
    let (signature, changes) = plan_for(&st, origin, req, token).await?;
    ctx.check()?;
    ctx.stage("Copying the server");
    let (src, dst, world, cancel) = (origin.dir(&st), st.cfg.servers_dir().join(run), req.include_world, ctx.token.clone());
    let copied = tokio::task::spawn_blocking(move || clone::copy_tree(&src, &dst, world, &|| cancel.is_cancelled())).await??;
    ctx.log(format!("Copied {} files ({} MB).", copied.files, copied.bytes >> 20));
    let mut sv = server::get(&st, run).await.map_err(|e| anyhow!(e.message))?;
    sandbox_properties(&sv.dir(&st));
    crate::files::patch_port(&st, &sv, sv.port).await;
    ctx.check()?;
    let mut notes: Vec<String> = vec![];
    if lifecycle::process_alive(origin) && req.include_world { notes.push("The world was copied while the server was running, so it may be slightly out of date.".into()); }
    if !req.include_world { notes.push("The copy starts with a fresh world; world-specific problems will not show up.".into()); }
    match req.kind.as_str() {
        "update_all" => {
            let c = compat(&st, &sv).ok_or_else(|| anyhow!("this runtime has no managed content"))?;
            ctx.stage("Applying the updates to the copy");
            update_all::run_all(ctx, &sv, &c, token, actor, false, None).await?;
        }
        "upgrade" => {
            let (c, t) = (compat(&st, &sv).ok_or_else(|| anyhow!("this runtime has no managed content"))?, req.upgrade.clone().ok_or_else(|| anyhow!("upgrade target missing"))?);
            ctx.stage("Upgrading the copy");
            upgrade::run_upgrade(ctx.clone(), sv.clone(), c, token.map(String::from), &t, false, actor.to_string()).await?;
            sv = server::get(&st, run).await.map_err(|e| anyhow!(e.message))?;
        }
        _ => {}
    }
    ctx.check()?;
    set_state(&st, run, "testing", None).await;
    ctx.stage(&format!("Starting the copy on port {}", sv.port));
    lifecycle::start(&st, &sv, actor).await.map_err(|e| anyhow!(e.message))?;
    let w = watch(ctx, &st, run, req.ready_timeout(), req.watch()).await?;
    let findings = health::analyze(&console_text(&st, &sv));
    let reasons = health::verdict(&health::Outcome { ready_secs: w.ready_secs.map(|s| s as u64), ready_timeout_s: req.ready_timeout(), crashed: w.crashed, exited_early: w.exited_early, findings: &findings });
    ctx.stage("Stopping the copy");
    lifecycle::halt(&st, &sv).await;
    let healthy = reasons.is_empty();
    let report = json!({
        "healthy": healthy, "reasons": reasons, "ready_secs": w.ready_secs.map(|s| (s * 10.0).round() / 10.0), "ready_timeout_s": req.ready_timeout(), "watch_s": req.watch(),
        "crashed": w.crashed, "errors": findings.errors, "warnings": findings.warnings, "samples": findings.samples, "fatal": findings.fatal,
        "changes": changes, "signature": signature, "notes": notes, "copy": {"files": copied.files, "bytes": copied.bytes, "world": req.include_world},
        "port": sv.port, "tested_at": crate::db::now(),
    });
    set_state(&st, run, if healthy { "healthy" } else { "unhealthy" }, Some(report)).await;
    st.timeline(&origin.id, "staging", &format!("test copy {}", if healthy { "healthy" } else { "unhealthy" })).await;
    ctx.stage(if healthy { "The copy is healthy" } else { "The copy has problems" });
    Ok(())
}
