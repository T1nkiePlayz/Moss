//! HTTP handlers for staged runs. Every route is addressed by the origin server; the staging copy itself is never a
//! route target for people (it only exists to be inspected and then discarded).
use super::run::{self, Req};
use super::*;
use crate::content::{update_all, upgrade};
use crate::content::modpack;

async fn origin_for(st: &St, user: &User, id: &str, upgrade_kind: bool) -> ApiResult<Srv> {
    let s = server_for(st, user, id, "content.manage").await?;
    if upgrade_kind { crate::auth::authz(st, user, id, "config.write").await?; }
    if server::is_staging(st, id).await { return Err(ApiError::conflict("staging.copy", "this is a test copy; manage the server it was made from")); }
    Ok(s)
}

async fn run_for(st: &St, id: &str, run: &str) -> ApiResult<Run> {
    get_run(st, run).await.filter(|r| r.origin_id == id).ok_or_else(|| ApiError::not_found("test copy"))
}

pub async fn estimate(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let s = origin_for(&st, &user, &id, false).await?;
    let (dir, servers) = (s.dir(&st), st.cfg.servers_dir());
    let (base, world) = tokio::task::spawn_blocking(move || clone::estimate(&dir)).await.map_err(anyhow::Error::from)?;
    let (lo, hi) = st.cfg.staging_ports();
    Ok(Json(json!({
        "files": base.files, "copy_bytes": base.bytes, "world_bytes": world, "free_bytes": crate::platform::disk(&servers).1, "port_range": [lo, hi],
        "defaults": {"ready_timeout_s": 180, "watch_s": 30}, "kinds": run::KINDS,
    })))
}

pub async fn list_runs(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    server_for(&st, &user, &id, "content.manage").await?;
    Ok(Json(Value::Array(runs_for(&st, &id).await.iter().map(Run::json).collect())))
}

pub async fn get_run_json(State(st): State<St>, user: User, Path((id, run)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    server_for(&st, &user, &id, "content.manage").await?;
    Ok(Json(run_for(&st, &id, &run).await?.json()))
}

pub async fn log(State(st): State<St>, user: User, Path((id, run)): Path<(String, String)>, Query(q): Query<HashMap<String, String>>) -> ApiResult<Json<Value>> {
    server_for(&st, &user, &id, "content.manage").await?;
    run_for(&st, &id, &run).await?;
    let n = q.get("lines").and_then(|v| v.parse::<usize>().ok()).unwrap_or(200).clamp(1, 1000);
    let s = server::get(&st, &run).await?;
    let text = health::strip_ansi(&run::console_text(&st, &s));
    let all: Vec<&str> = text.lines().collect();
    let from = all.len().saturating_sub(n);
    Ok(Json(json!({"lines": &all[from..], "truncated": from > 0})))
}

pub async fn start(State(st): State<St>, user: User, Path(id): Path<String>, Json(mut req): Json<Req>) -> ApiResult<(StatusCode, Json<Value>)> {
    let s = origin_for(&st, &user, &id, req.kind == "upgrade").await?;
    if !run::KINDS.contains(&req.kind.as_str()) { return Err(ApiError::bad("validation", "unknown kind").field("kind", "none | update_all | upgrade | pack_update")); }
    if s.runtime == "custom" { return Err(ApiError::bad("capability.unsupported", "custom servers cannot be copied for a test")); }
    if !s.installed { return Err(ApiError::conflict("server.not_installed", "server is not installed yet")); }
    match req.kind.as_str() {
        "update_all" => { compat(&st, &s).ok_or_else(|| ApiError::bad("capability.unsupported", "this runtime has no managed content"))?; }
        "upgrade" => {
            compat(&st, &s).ok_or_else(|| ApiError::bad("capability.unsupported", "this runtime has no managed content"))?;
            let t = req.upgrade.as_ref().ok_or_else(|| ApiError::bad("validation", "upgrade target required").field("upgrade", "required for kind upgrade"))?;
            upgrade::validate_target(&s, t)?;
        }
        "pack_update" => { compat(&st, &s).ok_or_else(|| ApiError::bad("capability.unsupported", "this runtime has no managed content"))?; }
        _ => {}
    }
    let mut kept_pack: Option<std::path::PathBuf> = None;
    if req.kind == "pack_update" {
        let pr = req.pack.as_mut().ok_or_else(|| ApiError::bad("validation", "pack details required").field("pack", "required for kind pack_update"))?;
        let row = modpack::install_row(&st, &id, &pr.install_id).await?;
        if row.pinned { return Err(ApiError::conflict("modpack.pinned", "this pack is pinned to its current version; unpin it to update")); }
        let tok = pr.token.take().ok_or_else(|| ApiError::bad("validation", "a checked pack version is required").field("pack.token", "required"))?;
        let entry = modpack::entry(&tok, &user.id).ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "modpack.token_expired", "this modpack token is unknown or has expired (tokens last 30 minutes); check for the update again"))?;
        if entry.runtime != s.runtime { return Err(ApiError::conflict("modpack.runtime_mismatch", format!("this pack needs {} but the server runs {}", entry.runtime, s.runtime))); }
        (pr.provider, pr.project_id, pr.version_id) = match entry.source { Some(i) => (Some(i.provider), Some(i.project_id), Some(i.version_id)), None => (None, None, None) };
        kept_pack = Some(entry.path);
    }
    let existing = runs_for(&st, &id).await;
    if existing.iter().any(Run::active) { return Err(ApiError::conflict("staging.busy", "a test copy is already being built for this server; wait for it or cancel it")); }
    let (dir, servers) = (s.dir(&st), st.cfg.servers_dir());
    let (base, world) = tokio::task::spawn_blocking(move || clone::estimate(&dir)).await.map_err(anyhow::Error::from)?;
    let need = base.bytes + if req.include_world { world } else { 0 };
    let free = crate::platform::disk(&servers).1;
    if free > 0 && free < need + (64 << 20) {
        return Err(ApiError::conflict("disk.low", format!("Not enough free disk space for a copy: it needs about {} MB and {} MB is free.", need >> 20, free >> 20)));
    }
    let used: Vec<i64> = server::list_all(&st).await?.iter().map(|x| x.port).collect();
    let range = st.cfg.staging_ports();
    let port = clone::pick_port(range, &used).await.ok_or_else(|| ApiError::conflict("staging.no_port", format!("No free port in {}-{} for the copy. Change MOSS_STAGING_PORT_RANGE or discard old copies.", range.0, range.1)))?;
    for r in existing { let _ = discard(&st, &r.id).await; }
    let (run_id, actor) = (uuid::Uuid::now_v7().to_string(), user.actor());
    if let Some(src) = kept_pack {
        let dst = run::pack_file(&st, &run_id);
        if let Some(d) = dst.parent() { tokio::fs::create_dir_all(d).await.map_err(anyhow::Error::from)?; }
        tokio::fs::copy(&src, &dst).await.map_err(anyhow::Error::from)?;
    }
    if let Err(e) = run::create_copy(&st, &s, &run_id, &req, port, &actor).await { let _ = tokio::fs::remove_file(run::pack_file(&st, &run_id)).await; return Err(e.into()); }
    let (title, token) = (format!("Test a copy of {}", s.name), user.token.clone());
    st.audit(&actor, "staging.start", &s.name, &req.kind, Some(&id)).await;
    let op = ops::spawn_op(&st, "staging.test", title, Some(&id), true, { let (r, o, rq, t, a) = (run_id.clone(), s.clone(), req.clone(), token, actor.clone()); move |ctx| run::test(ctx, r, o, rq, t, a) });
    let (rid, oid) = (run_id.clone(), op.id.clone());
    st.db.call(move |c| c.execute("UPDATE staging_runs SET op_id=? WHERE id=?", [oid, rid])).await?;
    announce(&st, &run_id).await;
    let run = get_run(&st, &run_id).await.map(|r| r.json());
    Ok((StatusCode::ACCEPTED, Json(json!({"run": run, "operation": op}))))
}

/// Applies the tested change to the real server (same plan, with the pre-change backup and rollback).
pub async fn promote(State(st): State<St>, user: User, Path((id, run)): Path<(String, String)>) -> ApiResult<(StatusCode, Json<Value>)> {
    let r = run_for(&st, &id, &run).await?;
    let s = origin_for(&st, &user, &id, r.kind == "upgrade").await?;
    if r.kind == "none" { return Err(ApiError::conflict("staging.nothing_to_promote", "this run only tested a copy of the server; there is no change to apply")); }
    if r.state != "healthy" { return Err(ApiError::conflict("staging.not_healthy", "only a copy that passed its checks can be applied to the real server")); }
    if r.expired() { return Err(ApiError::conflict("staging.expired", "this test is more than 24 hours old; run it again")); }
    let req: Req = serde_json::from_value(r.request.clone()).map_err(|_| ApiError::bad("staging.invalid", "the stored request could not be read"))?;
    let c = compat(&st, &s).ok_or_else(|| ApiError::bad("capability.unsupported", "this runtime has no managed content"))?;
    if r.kind == "upgrade" && (st.live.lock().unwrap().get(&id).is_some_and(|l| l.alive) || crate::lifecycle::process_alive(&s)) {
        return Err(ApiError::conflict("server.running", "stop the server before applying an upgrade"));
    }
    let (token, actor, sig, run_id) = (user.token.clone(), user.actor(), r.report["signature"].as_str().map(String::from), r.id.clone());
    st.audit(&actor, "staging.promote", &s.name, &r.kind, Some(&id)).await;
    let op = ops::spawn_op(&st, "staging.promote", format!("Apply the tested change to {}", s.name), Some(&id), true, move |ctx| async move {
        match req.kind.as_str() {
            "update_all" => { update_all::run_all(&ctx, &s, &c, token.as_deref(), &actor, true, sig.as_deref()).await?; }
            "upgrade" => {
                let t = req.upgrade.clone().ok_or_else(|| anyhow!("upgrade target missing"))?;
                ctx.stage("Checking that nothing changed since the test");
                if Some(upgrade::upgrade_signature(&ctx.st, token.as_deref(), &s, &c, &t).await?) != sig { bail!("what the upgrade would change differs from what was tested; run the test again"); }
                upgrade::run_upgrade(ctx.clone(), s.clone(), c, token, &t, true, actor).await?;
            }
            "pack_update" => {
                let pr = req.pack.clone().ok_or_else(|| anyhow!("pack update details missing"))?;
                modpack::apply_update(&ctx, &s, &c, &pr.install_id, &run::pack_file(&ctx.st, &run_id), token.as_deref(), &actor, sig.as_deref(), true, pr.identity()).await?;
            }
            _ => bail!("nothing to apply"),
        }
        ctx.stage("Removing the test copy");
        discard(&ctx.st, &run_id).await
    });
    Ok((StatusCode::ACCEPTED, Json(json!({"operation": op}))))
}

/// Discards a copy. A copy that is still being built is cancelled instead (its operation cleans up after itself).
pub async fn discard_run(State(st): State<St>, user: User, Path((id, run)): Path<(String, String)>) -> ApiResult<(StatusCode, Json<Value>)> {
    let r = run_for(&st, &id, &run).await?;
    let s = origin_for(&st, &user, &id, false).await?;
    st.audit(&user.actor(), "staging.discard", &s.name, &r.kind, Some(&id)).await;
    if r.active() && let Some(op) = &r.op_id && ops::cancel(&st, op) { return Ok((StatusCode::ACCEPTED, Json(json!({"state": "cancelling"})))); }
    discard(&st, &run).await?;
    Ok((StatusCode::OK, Json(json!({"state": "discarded"}))))
}
