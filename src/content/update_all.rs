//! Update everything at once: collect updates for non-pinned provider items (including identified manual files), resolve
//! them into ONE change plan, and apply it as one operation with one undo snapshot, dependencies first.
use super::*;

/// Resolves every available update into one ordered list of steps (dependencies before dependents, no project twice).
pub(super) async fn build_all(st: &St, s: &Srv, c: &Compat, token: Option<&str>) -> Result<(Vec<Step>, Vec<String>)> {
    let items = db_items(st, &s.id).await?;
    let found = apply::collect_updates(st, token, c, &items).await;
    let updates = found.list;
    let mut steps: Vec<Step> = vec![];
    let mut warnings: Vec<String> = found.problems;
    for u in updates {
        let (Some(pid), Some(prov)) = (u["project_id"].as_str(), u["provider"].as_str()) else { continue };
        let src = Src { prov: Prov::parse(Some(prov)).unwrap_or(Prov::Modrinth), token: token.map(String::from) };
        match build_plan(st, &src, s, c, pid, None).await {
            Ok((new_steps, w)) => {
                merge_steps(&mut steps, new_steps);
                for w in w { if !warnings.contains(&w) { warnings.push(w); } }
            }
            Err(e) => warnings.push(format!("{}: skipped — {e:#}", u["title"].as_str().unwrap_or("item"))),
        }
    }
    Ok((steps, warnings))
}

/// Appends steps that are not already planned (the same project can be reached as an update and as a dependency).
pub(super) fn merge_steps(steps: &mut Vec<Step>, new: Vec<Step>) {
    for ns in new { if !steps.iter().any(|x| x.prov == ns.prov && x.project_id == ns.project_id) { steps.push(ns); } }
}

pub async fn update_all_plan(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let (s, c) = compat_for(&st, &user, &id, "content.manage").await?;
    let (steps, warnings) = build_all(&st, &s, &c, user.token.as_deref()).await.map_err(|e| match e.downcast_ref::<cf::CfError>() { Some(_) => upstream(e), None => ApiError::bad("plan.failed", format!("{e:#}")) })?;
    Ok(Json(plan_json(&st, &s, &c, &steps, warnings).await?))
}

/// Identifies a resolved plan (provider, project and version of every step, order-independent) so a staged
/// run can prove that promoting applies exactly what was tested.
pub(super) fn plan_signature(steps: &[Step]) -> String {
    let mut v: Vec<String> = steps.iter().map(|s| format!("{}:{}:{}", s.prov.as_str(), s.project_id, s.version["id"].as_str().unwrap_or_default())).collect();
    v.sort();
    v.join(",")
}

/// What an update-all run does. `expect` is the signature a staged test was made against; `backup` takes the
/// pre-change backup. Takes the server lock itself; returns the signature of what was applied.
pub(super) async fn run_all(ctx: &OpCtx, s: &Srv, c: &Compat, token: Option<&str>, actor: &str, backup: bool, expect: Option<&str>) -> Result<String> {
    let st = ctx.st.clone();
    let _guard = st.lock_for(&s.id).try_lock_owned().map_err(|_| anyhow!("another operation is changing this server"))?;
    ctx.stage("Checking for updates");
    let (steps, _warnings) = build_all(&st, s, c, token).await?;
    ctx.check()?;
    let sig = plan_signature(&steps);
    if let Some(e) = expect && e != sig { bail!("the available updates changed since the test copy was made; run the test again"); }
    if steps.is_empty() { ctx.log("Everything is up to date."); return Ok(sig); }
    if backup { crate::backups::pre_change_locked(&st, s, "updating content").await?; }
    let label = format!("Update all ({} files)", steps.len());
    apply::execute(ctx, s, c, &steps, token, actor, apply::Labels { undo: Some(label), audit: "content.update_all" }).await?;
    Ok(sig)
}

pub async fn update_all(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<(StatusCode, Json<Value>)> {
    let (s, c) = compat_for(&st, &user, &id, "content.manage").await?;
    let (token, actor) = (user.token.clone(), user.actor());
    let op = ops::spawn_op(&st, "content.update_all", format!("Update all content on {}", s.name), Some(&id), true, move |ctx| async move {
        run_all(&ctx, &s, &c, token.as_deref(), &actor, true, None).await.map(|_| ())
    });
    Ok((StatusCode::ACCEPTED, Json(json!({"operation": op}))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(prov: Prov, pid: &str) -> Step { Step { prov, action: "replace", project_id: pid.into(), title: pid.into(), version: json!({}), file: json!({}), replaces: None, reason: String::new() } }

    #[test]
    fn merging_keeps_dependency_order_and_drops_repeats_per_provider() {
        let mut steps = vec![step(Prov::Modrinth, "lib"), step(Prov::Modrinth, "a")];
        merge_steps(&mut steps, vec![step(Prov::Modrinth, "lib"), step(Prov::Modrinth, "b"), step(Prov::Curseforge, "lib")]);
        let ids: Vec<(bool, &str)> = steps.iter().map(|s| (s.prov == Prov::Curseforge, s.project_id.as_str())).collect();
        assert_eq!(ids, vec![(false, "lib"), (false, "a"), (false, "b"), (true, "lib")]);
    }
}
