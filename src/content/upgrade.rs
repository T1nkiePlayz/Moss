//! Version upgrade assistant: `POST /servers/{id}/upgrade/plan` (what would change) and `POST /servers/{id}/upgrade`
//! (the operation). Classification and blockers are pure (`classify`); the operation lives in `run`.
use super::*;
use crate::runtimes::{self, Resolved};
use classify::{Entry, Facts, Linked, Lookup};
use futures_util::StreamExt;

mod classify;
mod run;

/// Setting-driven safety backup for callers holding the server lock (modpack import).
pub async fn maybe_backup(st: &St, s: &Srv) -> Result<Option<String>> { crate::backups::pre_change_locked(st, s, "importing a modpack").await }

#[derive(Deserialize)]
pub struct PlanReq { game_version: String, loader_version: Option<String>, #[serde(default)] confirm_downgrade: bool }

#[derive(Deserialize)]
pub struct UpgradeReq {
    game_version: String, loader_version: Option<String>, #[serde(default = "default_content")] content: String,
    #[serde(default = "yes")] backup: bool, #[serde(default)] confirm_downgrade: bool,
}
fn default_content() -> String { "update".into() }
fn yes() -> bool { true }

fn plain(v: &str) -> bool { !v.is_empty() && v.len() <= 40 && v.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-')) }

fn check_versions(game: &str, loader: Option<&str>) -> ApiResult<()> {
    if !plain(game) { return Err(ApiError::bad("validation", "invalid game version").field("game_version", "letters, digits and . _ + -")); }
    if loader.is_some_and(|l| !plain(l)) { return Err(ApiError::bad("validation", "invalid loader version").field("loader_version", "letters, digits and . _ + -")); }
    Ok(())
}

/// The same compatibility rules, pointed at another game version.
pub(super) fn for_game(c: &Compat, gv: &str) -> Compat {
    Compat { kind: c.kind, project_type: c.project_type, loaders: c.loaders.clone(), game_version: c.game_version.as_ref().map(|_| gv.to_string()), dir: c.dir.clone(), ext: c.ext, cf_class: c.cf_class, cf_loader: c.cf_loader }
}

/// Everything the plan endpoint reports, plus what the operation needs (the resolved runtime and the entries).
pub(super) struct Built { pub json: Value, pub blockers: Vec<String>, pub resolved: Option<Resolved>, pub entries: Vec<Entry> }

async fn linked_items(st: &St, s: &Srv, c: &Compat) -> Result<Vec<Linked>> {
    let rows = db_items(st, &s.id).await?;
    let mut out: Vec<Linked> = rows.iter().map(|i| {
        let g = |k: &str| i[k].as_str().map(String::from);
        Linked { item: g("id").unwrap_or_default(), title: g("title").unwrap_or_default(), provider: g("provider"), project_id: g("project_id"), version_id: g("version_id"), current: g("version_number"), filename: g("filename").unwrap_or_default(), pinned: i["pinned"] == true }
    }).collect();
    if let Ok(mut rd) = tokio::fs::read_dir(safe_join(&s.dir(st), &c.dir).map_err(|e| anyhow!(e.message))?).await {
        while let Ok(Some(e)) = rd.next_entry().await {
            let name = e.file_name().to_string_lossy().to_string();
            if name.ends_with(&format!(".{}", c.ext)) && !out.iter().any(|l| l.filename == name) {
                out.push(Linked { item: format!("manual:{name}"), title: name.clone(), provider: None, project_id: None, version_id: None, current: None, filename: name, pinned: false });
            }
        }
    }
    Ok(out)
}

async fn modrinth_lookup(st: &St, ct: &Compat, l: Linked) -> (String, Lookup) {
    let r = match compatible_versions(st, l.project_id.as_deref().unwrap_or_default(), ct).await {
        Ok(vs) => pick_latest(&vs).map_or(Lookup::NoBuild, |v| Lookup::Build { version_id: v["id"].as_str().unwrap_or_default().into(), label: v["version_number"].as_str().unwrap_or_default().into() }),
        Err(e) => Lookup::Failed(format!("Modrinth: {e:#}")),
    };
    (l.item, r)
}

/// Ask each provider for the newest build compatible with the target. Modrinth is one call per item (bounded
/// concurrency); CurseForge is batched.
async fn lookup_all(st: &St, token: Option<&str>, ct: &Compat, items: &[Linked]) -> HashMap<String, Lookup> {
    let mut found = HashMap::new();
    let mr_items: Vec<Linked> = items.iter().filter(|l| l.provider.as_deref() == Some("modrinth") && l.project_id.is_some()).cloned().collect();
    let results: Vec<(String, Lookup)> = futures_util::stream::iter(mr_items.into_iter().map(|l| modrinth_lookup(st, ct, l))).buffer_unordered(6).collect().await;
    found.extend(results);
    let cf_items: Vec<&Linked> = items.iter().filter(|l| l.provider.as_deref() == Some("curseforge") && l.project_id.is_some()).collect();
    if !cf_items.is_empty() {
        if !cf::availability(st, token).0 {
            for l in &cf_items { found.insert(l.item.clone(), Lookup::Failed("CurseForge is not available on this instance".into())); }
        } else {
            let ids: Vec<u64> = cf_items.iter().filter_map(|l| l.project_id.as_deref()?.parse().ok()).collect();
            let mut mods: Vec<Value> = vec![];
            let mut err = None;
            for chunk in ids.chunks(50) {
                match cf::call(st, token, reqwest::Method::POST, "/v1/mods", &[], Some(json!({"modIds": chunk}))).await {
                    Ok(v) => mods.extend(v["data"].as_array().cloned().unwrap_or_default()),
                    Err(e) => { err = Some(format!("CurseForge: {e:#}")); break; }
                }
            }
            for l in cf_items {
                let m = mods.iter().find(|m| m["id"].as_u64().map(|n| n.to_string()) == l.project_id);
                let r = match (m, &err) {
                    (Some(m), _) => cf::latest_index(m, ct.game_version.as_deref(), ct.cf_loader).map_or(Lookup::NoBuild, |(fid, name)| Lookup::Build { version_id: fid, label: name }),
                    (None, Some(e)) => Lookup::Failed(e.clone()),
                    (None, None) => Lookup::Failed("project not found on CurseForge".into()),
                };
                found.insert(l.item.clone(), r);
            }
        }
    }
    found
}

pub(super) async fn build(st: &St, token: Option<&str>, s: &Srv, c: &Compat, game: &str, loader: Option<&str>, confirm_downgrade: bool) -> Result<Built> {
    let resolved = runtimes::resolve(st, &s.runtime, game, loader).await;
    let (jar_error, java_to) = match &resolved {
        Ok(r) => (None, r.java_major),
        Err(e) => (Some(format!("{e:#}")), runtimes::java_for_game(st, game).await),
    };
    let to_loader = resolved.as_ref().ok().and_then(|r| r.loader_version.clone()).or_else(|| loader.map(String::from));
    let running = lifecycle_running(st, s) || crate::lifecycle::process_alive(s);
    let blockers = classify::blockers(&Facts { running, from: &s.game_version, to: game, from_loader: s.loader_version.as_deref(), to_loader: to_loader.as_deref(), confirm_downgrade, jar_error: jar_error.as_deref() });
    let items = linked_items(st, s, c).await?;
    let found = lookup_all(st, token, &for_game(c, game), &items).await;
    let entries = classify::classify(&items, &found);
    let mut warnings = classify::warnings(&s.game_version, game, (s.java_major, java_to), &entries);
    if s.runtime == "forge" { warnings.push("Forge support is experimental.".into()); }
    let note = resolved.as_ref().ok().and_then(|r| r.loader_version.clone()).map(|l| format!("{} {l}", s.runtime));
    let json = json!({
        "from": {"game_version": s.game_version, "loader_version": s.loader_version}, "to": {"game_version": game, "loader_version": to_loader},
        "java": {"from": s.java_major, "to": java_to, "installed": crate::java::find(st, java_to).is_some()},
        "server_jar": {"available": resolved.is_ok(), "note": jar_error.clone().or(note)},
        "content": entries.iter().map(Entry::json).collect::<Vec<_>>(), "blockers": blockers, "warnings": warnings, "can_proceed": blockers.is_empty(),
    });
    Ok(Built { json, blockers, resolved: resolved.ok(), entries })
}

fn upgrade_target(s: &Srv) -> ApiResult<()> {
    if matches!(s.runtime.as_str(), "custom" | "sim") { return Err(ApiError::bad("capability.unsupported", "this runtime cannot be upgraded by Moss")); }
    if !s.installed { return Err(ApiError::conflict("server.not_installed", "server is not installed yet")); }
    Ok(())
}

pub async fn upgrade_plan(State(st): State<St>, user: User, Path(id): Path<String>, Json(r): Json<PlanReq>) -> ApiResult<Json<Value>> {
    let s = server_for(&st, &user, &id, "config.write").await?;
    crate::auth::authz(&st, &user, &id, "content.manage").await?;
    upgrade_target(&s)?;
    check_versions(&r.game_version, r.loader_version.as_deref())?;
    let c = compat(&st, &s).ok_or_else(|| ApiError::bad("capability.unsupported", "this runtime has no managed content"))?;
    let b = build(&st, user.token.as_deref(), &s, &c, &r.game_version, r.loader_version.as_deref(), r.confirm_downgrade).await?;
    Ok(Json(b.json))
}

pub async fn upgrade_start(State(st): State<St>, user: User, Path(id): Path<String>, Json(r): Json<UpgradeReq>) -> ApiResult<(StatusCode, Json<Value>)> {
    let s = server_for(&st, &user, &id, "config.write").await?;
    crate::auth::authz(&st, &user, &id, "content.manage").await?;
    upgrade_target(&s)?;
    check_versions(&r.game_version, r.loader_version.as_deref())?;
    if !matches!(r.content.as_str(), "update" | "disable_incompatible" | "keep") { return Err(ApiError::bad("validation", "unknown content choice").field("content", "update | disable_incompatible | keep")); }
    if lifecycle_running(&st, &s) || crate::lifecycle::process_alive(&s) { return Err(ApiError::conflict("server.running", "stop the server before upgrading it")); }
    if classify::is_downgrade(&s.game_version, &r.game_version) && !r.confirm_downgrade { return Err(ApiError::bad("upgrade.downgrade", "this is a downgrade; send confirm_downgrade:true to proceed").field("confirm_downgrade", "required for downgrades")); }
    let c = compat(&st, &s).ok_or_else(|| ApiError::bad("capability.unsupported", "this runtime has no managed content"))?;
    let (title, token, actor) = (format!("Upgrade {} to {}", s.name, r.game_version), user.token.clone(), user.actor());
    let op = ops::spawn_op(&st, "server.upgrade", title, Some(&id), true, move |ctx| run::run(ctx, s, c, token, r, actor));
    Ok((StatusCode::ACCEPTED, Json(json!({"operation": op}))))
}
