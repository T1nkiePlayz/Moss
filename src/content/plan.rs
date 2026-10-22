//! Change plans: resolves a project (and its required dependencies) into reviewed install steps per provider.
use super::*;

// ---------- planning ----------
pub(super) struct Step { pub prov: Prov, pub action: &'static str, pub project_id: String, pub title: String, pub version: Value, pub file: Value, pub replaces: Option<(String, String)>, pub reason: String }

pub(super) fn primary_file(v: &Value) -> Option<Value> {
    let files = v["files"].as_array()?;
    files.iter().find(|f| f["primary"] == true).or_else(|| files.first()).cloned()
}

pub(super) fn safe_filename(n: &str, ext: &str) -> Result<String> {
    let base = std::path::Path::new(n).file_name().and_then(|f| f.to_str()).unwrap_or_default();
    if base.is_empty() || base.starts_with('.') || base != n || !base.to_lowercase().ends_with(&format!(".{ext}")) { bail!("provider file name {n:?} is not an acceptable .{ext} file"); }
    Ok(base.to_string())
}

pub(super) async fn build_plan(st: &St, src: &Src, s: &Srv, c: &Compat, project_id: &str, version_id: Option<&str>) -> Result<(Vec<Step>, Vec<String>)> {
    match src.prov {
        Prov::Modrinth => build_plan_mr(st, s, c, project_id, version_id).await,
        Prov::Curseforge => build_plan_cf(st, src.token.as_deref(), s, c, project_id, version_id).await,
        Prov::Hangar => build_plan_hg(st, s, c, project_id, version_id).await,
    }
}

pub(super) async fn build_plan_cf(st: &St, token: Option<&str>, s: &Srv, c: &Compat, project_id: &str, version_id: Option<&str>) -> Result<(Vec<Step>, Vec<String>)> {
    if project_id.parse::<u32>().is_err() { bail!("invalid project id"); }
    let mut warnings = vec!["CurseForge does not say whether a mod runs on servers; Moss skips files their authors mark client-only.".to_string()];
    if s.runtime == "folia" { warnings.push("Folia needs plugins that explicitly declare Folia support; CurseForge does not filter for it.".into()); }
    if lifecycle_running(st, s) { warnings.push("The server is running. Restart it to load the change.".into()); }
    let installed = db_items(st, &s.id).await?;
    let mut steps: Vec<Step> = vec![];
    let mut queue: Vec<(String, Option<String>)> = vec![(project_id.to_string(), version_id.map(String::from))];
    let mut capped = false;
    while let Some((pid, vid)) = queue.pop() {
        if steps.iter().any(|x| x.project_id == pid) { continue; }
        if steps.len() >= 12 {
            if !capped { capped = true; warnings.push("This install pulls in more than 12 items; the remaining dependencies were not planned. Install them separately.".into()); }
            continue;
        }
        let m = cf::get(st, token, &format!("/v1/mods/{pid}"), &[]).await?;
        let m = &m["data"];
        let title = m["name"].as_str().unwrap_or("project").to_string();
        if !cf::distributable(m) { bail!("{title}: the author disabled third-party downloads on CurseForge. Download it from the project page and upload it in Files."); }
        let file = match vid {
            Some(v) => {
                if v.parse::<u32>().is_err() { bail!("invalid version id"); }
                let f = cf::get(st, token, &format!("/v1/mods/{pid}/files/{v}"), &[]).await?["data"].clone();
                let tags: Vec<&str> = f["gameVersions"].as_array().into_iter().flatten().filter_map(|t| t.as_str()).collect();
                let gv_ok = c.game_version.as_ref().is_none_or(|g| tags.contains(&g.as_str()));
                let loader_ok = c.cf_loader == 0 || tags.contains(&cf::loader_tag(c.cf_loader));
                if !gv_ok || !loader_ok { bail!("{title} {} is not compatible with {} {}", f["displayName"].as_str().unwrap_or(""), s.runtime, s.game_version); }
                if !cf::downloadable(&f) { bail!("{title}: the author disabled automatic downloads for this file. Download it from the project page and upload it in Files."); }
                f
            }
            None => {
                let files = cf_files(st, token, &pid, c).await?;
                cf::pick_latest(&files).cloned().ok_or_else(|| anyhow!("no downloadable version of {title} is compatible with {} {}", s.runtime, s.game_version))?
            }
        };
        let fname = file["fileName"].as_str().unwrap_or_default();
        safe_filename(fname, c.ext)?;
        if file["releaseType"].as_u64() != Some(1) { warnings.push(format!("{title} {} is a {} build.", file["displayName"].as_str().unwrap_or(""), cf::channel(&file))); }
        let existing = installed.iter().find(|i| i["provider"] == "curseforge" && i["project_id"] == pid.as_str());
        let fid = file["id"].as_u64().unwrap_or_default().to_string();
        if let Some(e) = existing && e["version_id"] == fid.as_str() && pid != project_id { continue; }
        let replaces = existing.map(|e| (e["id"].as_str().unwrap_or_default().to_string(), e["filename"].as_str().unwrap_or_default().to_string()));
        let (action, reason) = match &replaces { Some(_) => ("replace", "replaces the installed version".to_string()), None if pid == project_id => ("install", "requested".to_string()), None => ("install", "required dependency".to_string()) };
        for d in file["dependencies"].as_array().into_iter().flatten() {
            if d["relationType"] == 3 && let Some(dp) = d["modId"].as_u64() {
                let dp = dp.to_string();
                if !installed.iter().any(|i| i["provider"] == "curseforge" && i["project_id"] == dp.as_str()) { queue.push((dp, None)); }
            }
        }
        let sha1 = file["hashes"].as_array().into_iter().flatten().find(|h| h["algo"] == 1).and_then(|h| h["value"].as_str()).map(String::from);
        let version = json!({"id": fid, "version_number": file["displayName"], "version_type": cf::channel(&file)});
        let file_j = json!({"filename": fname, "size": file["fileLength"], "url": file["downloadUrl"], "hashes": {"sha1": sha1}, "cf_mod": pid});
        steps.push(Step { prov: Prov::Curseforge, action, project_id: pid, title, version, file: file_j, replaces, reason });
    }
    steps.reverse();
    Ok((steps, warnings))
}

pub(super) async fn build_plan_mr(st: &St, s: &Srv, c: &Compat, project_id: &str, version_id: Option<&str>) -> Result<(Vec<Step>, Vec<String>)> {
    if !valid_id(project_id) { bail!("invalid project id"); }
    let mut warnings = vec![];
    if s.runtime == "folia" { warnings.push("Folia needs plugins that explicitly declare Folia support; Modrinth lists only those tagged for Folia.".into()); }
    if lifecycle_running(st, s) { warnings.push("The server is running. Restart it to load the change.".into()); }
    let installed = db_items(st, &s.id).await?;
    let mut steps: Vec<Step> = vec![];
    let mut queue: Vec<(String, Option<String>)> = vec![(project_id.to_string(), version_id.map(String::from))];
    let mut capped = false;
    while let Some((pid, vid)) = queue.pop() {
        if steps.iter().any(|x| x.project_id == pid) { continue; }
        if steps.len() >= 12 {
            if !capped { capped = true; warnings.push("This install pulls in more than 12 items; the remaining dependencies were not planned. Install them separately.".into()); }
            continue;
        }
        let proj = mr(st, &format!("/project/{pid}"), &[]).await?;
        let pid_real = proj["id"].as_str().unwrap_or(&pid).to_string();
        let version = match vid {
            Some(v) => {
                let v = mr(st, &format!("/version/{v}"), &[]).await?;
                if v["project_id"] != pid_real.as_str() { bail!("version does not belong to project"); }
                let loaders_ok = v["loaders"].as_array().is_some_and(|l| l.iter().any(|x| c.loaders.iter().any(|y| x == y)));
                let gv_ok = c.game_version.as_ref().is_none_or(|g| v["game_versions"].as_array().is_some_and(|a| a.iter().any(|x| x == g.as_str())));
                if !loaders_ok || !gv_ok { bail!("{} {} is not compatible with {} {}", proj["title"].as_str().unwrap_or("project"), v["version_number"].as_str().unwrap_or(""), s.runtime, s.game_version); }
                v
            }
            None => {
                let vs = compatible_versions(st, &pid_real, c).await?;
                pick_latest(&vs).cloned().ok_or_else(|| anyhow!("no version of {} is compatible with {} {}", proj["title"].as_str().unwrap_or("this project"), s.runtime, s.game_version))?
            }
        };
        if version["version_type"] != "release" { warnings.push(format!("{} {} is a {} build.", proj["title"].as_str().unwrap_or(""), version["version_number"].as_str().unwrap_or(""), version["version_type"].as_str().unwrap_or("pre-release"))); }
        let file = primary_file(&version).ok_or_else(|| anyhow!("version has no files"))?;
        safe_filename(file["filename"].as_str().unwrap_or_default(), c.ext)?;
        let existing = installed.iter().find(|i| i["provider"] == "modrinth" && i["project_id"] == pid_real.as_str());
        if let Some(e) = existing && e["version_id"] == version["id"] && pid_real != project_id { continue; }
        let is_root = pid_real == project_id || pid == project_id;
        let replaces = existing.map(|e| (e["id"].as_str().unwrap_or_default().to_string(), e["filename"].as_str().unwrap_or_default().to_string()));
        let (action, reason) = match &replaces { Some(_) => ("replace", "replaces the installed version".to_string()), None if is_root => ("install", "requested".to_string()), None => ("install", "required dependency".to_string()) };
        for d in version["dependencies"].as_array().into_iter().flatten() {
            if d["dependency_type"] == "required"
                && let Some(dp) = d["project_id"].as_str()
                    && !installed.iter().any(|i| i["provider"] == "modrinth" && i["project_id"] == dp) { queue.push((dp.to_string(), d["version_id"].as_str().map(String::from))); }
        }
        steps.push(Step { prov: Prov::Modrinth, action, project_id: pid_real, title: proj["title"].as_str().unwrap_or("project").into(), version, file, replaces, reason });
    }
    steps.reverse();
    Ok((steps, warnings))
}

pub async fn plan(State(st): State<St>, user: User, Path(id): Path<String>, Json(r): Json<InstallReq>) -> ApiResult<Json<Value>> {
    let (s, c) = compat_for(&st, &user, &id, "content.manage").await?;
    let src = src_for(&st, &user, &c, r.provider.as_deref())?;
    let (steps, warnings) = build_plan(&st, &src, &s, &c, &r.project_id, r.version_id.as_deref()).await.map_err(|e| match e.downcast_ref::<cf::CfError>() { Some(_) => upstream(e), None => ApiError::bad("plan.failed", format!("{e:#}")) })?;
    Ok(Json(plan_json(&st, &s, &c, &steps, warnings).await?))
}

/// The ChangePlan response for resolved steps: items with versions, warnings, and the problems the plan would cause.
pub(super) async fn plan_json(st: &St, s: &Srv, c: &Compat, steps: &[Step], mut warnings: Vec<String>) -> Result<Value> {
    let installed = db_items(st, &s.id).await?;
    let from = |x: &Step| x.replaces.as_ref().and_then(|(id, _)| installed.iter().find(|i| i["id"] == id.as_str())).and_then(|i| i["version_number"].as_str().map(String::from));
    let changes: Vec<check::Change> = steps.iter().map(|x| check::Change { replaces: x.replaces.as_ref().map(|(id, _)| id.clone()), new_version: x.version["version_number"].as_str().unwrap_or_default().to_string() }).collect();
    let conflicts = check::plan_conflicts(st, s, c, &changes).await.unwrap_or_default();
    for f in &conflicts { warnings.push(format!("{}: {}", f.title, f.detail)); }
    let items: Vec<Value> = steps.iter().map(|x| json!({"action": x.action, "title": x.title, "filename": x.file["filename"], "size": x.file["size"], "reason": x.reason, "version": x.version["version_number"], "from_version": from(x)})).collect();
    Ok(json!({"items": items, "warnings": warnings, "conflicts": conflicts, "requires_restart": true}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_filename_rejects_paths_and_wrong_extensions() {
        assert!(safe_filename("Plugin-1.0.jar", "jar").is_ok());
        for bad in ["../x.jar", "a/b.jar", ".hidden.jar", "x.exe", ""] { assert!(safe_filename(bad, "jar").is_err(), "{bad}"); }
    }
}
