//! Hangar (PaperMC's plugin repository): the HTTP helper, search, project details, version selection/mapping and
//! change plans for Paper-family servers and Velocity. No API key is needed. Versions that only link to an external
//! download are shown but never installed: Moss only downloads files hosted by Hangar, verified by SHA-256.
use super::*;

/// A request that gives up with a clear message instead of hanging the caller (shorter under test).
#[cfg(not(test))] const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(25);
#[cfg(test)] const TIMEOUT: std::time::Duration = std::time::Duration::from_millis(700);

fn hg_err(rate_limited: bool, msg: impl Into<String>) -> anyhow::Error { cf::CfError { rate_limited, msg: msg.into() }.into() }

/// Sends one Hangar request and turns every failure into a short message a server owner can act on.
/// Errors carry `CfError` so handlers map them to 429 / 502 like the other providers.
async fn hg_send(rb: reqwest::RequestBuilder) -> Result<Value> {
    let r = rb.timeout(TIMEOUT).send().await.map_err(|e| hg_err(false, if e.is_timeout() { "Hangar did not answer in time. Try again in a moment.".to_string() } else { format!("Could not reach Hangar: {}", e.without_url()) }))?;
    let s = r.status();
    if s.is_success() { return r.json::<Value>().await.map_err(|_| hg_err(false, "Hangar sent an answer Moss could not read. Try again later.")); }
    let retry = r.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse::<u64>().ok()).map(|v| v.clamp(1, 600));
    Err(match s.as_u16() {
        429 => hg_err(true, match retry { Some(n) => format!("Hangar is rate limiting Moss. Try again in {n}s."), None => "Hangar is rate limiting Moss. Try again in a minute.".into() }),
        404 => hg_err(false, "Not found on Hangar."),
        500..=599 => hg_err(false, format!("Hangar is having problems right now (HTTP {}). Try again later.", s.as_u16())),
        n => hg_err(false, format!("Hangar refused the request (HTTP {n}).")),
    })
}

fn base(st: &St) -> String { st.cfg.hangar_api_url.trim_end_matches('/').to_string() }

pub(super) async fn hg(st: &St, path: &str, query: &[(&str, String)]) -> Result<Value> {
    let key = format!("hg:{path}:{}", query.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&"));
    let (st2, path2, q2) = (st.clone(), path.to_string(), query.iter().map(|(k, v)| (k.to_string(), v.clone())).collect::<Vec<_>>());
    st.cached(&key, 90, move || async move { hg_send(st2.http.get(format!("{}{path2}", base(&st2))).query(&q2)).await }).await
}

/// Hangar's platform name for this server, or `None` when Hangar has nothing for the runtime.
pub(super) fn hangar_platform(c: &Compat) -> Option<&'static str> {
    match c.loaders.first().copied() { Some("paper" | "purpur" | "folia") => Some("PAPER"), Some("velocity") => Some("VELOCITY"), _ => None }
}

/// Hangar project ids are `slug`s (letters, digits, `-`, `_`) or the numeric ids used by plugin dependencies.
pub(super) fn hangar_id(s: &str) -> bool { valid_id(s) }

fn require_hangar(c: &Compat) -> ApiResult<&'static str> {
    hangar_platform(c).ok_or_else(|| ApiError::bad("capability.unsupported", "Hangar has no content for this runtime"))
}

/// Moss channel name for a Hangar channel (Release, Snapshot, Alpha or an author-made one).
pub(super) fn channel(v: &Value) -> &'static str {
    match v["channel"]["name"].as_str().unwrap_or_default().to_ascii_lowercase().as_str() { "release" => "release", "snapshot" | "beta" => "beta", _ => "alpha" }
}

/// Hangar's own download hosts. The checksum comes from the same API answer as the address, so it cannot vouch for a
/// file served by anyone else.
const DOWNLOAD_HOSTS: &[&str] = &["hangarcdn.papermc.io", "hangar.papermc.io"];

/// An https address on exactly one of Hangar's download hosts (no credentials, default port).
pub(super) fn hangar_host(u: &str) -> bool {
    reqwest::Url::parse(u).is_ok_and(|u| u.scheme() == "https" && u.username().is_empty() && u.password().is_none() && u.port().is_none() && u.host_str().is_some_and(|h| DOWNLOAD_HOSTS.contains(&h)))
}

/// The hosted file of a version for a platform, ready for the plan: `None` when the version only links elsewhere.
pub(super) fn hg_file(v: &Value, platform: &str) -> Option<Value> {
    let d = &v["downloads"][platform];
    let (url, info) = (d["downloadUrl"].as_str().filter(|u| hangar_host(u))?, d["fileInfo"].as_object()?);
    let name = info.get("name")?.as_str()?;
    Some(json!({"filename": name, "size": info.get("sizeBytes"), "url": url, "hashes": {"sha256": info.get("sha256Hash").and_then(Value::as_str)}}))
}

/// Where a version that is not hosted on Hangar can be downloaded by hand.
pub(super) fn hg_external(v: &Value, platform: &str) -> Option<String> {
    http_url(&v["downloads"][platform]["externalUrl"]).as_str().map(String::from)
}

fn dep_list<'a>(v: &'a Value, platform: &str) -> impl Iterator<Item = &'a Value> { v["pluginDependencies"][platform].as_array().into_iter().flatten() }

/// A dependency's Hangar project id (they are numbers in the API), when it lives on Hangar.
fn dep_project(d: &Value) -> Option<String> {
    match &d["projectId"] { Value::Number(n) => Some(n.to_string()), Value::String(s) if hangar_id(s) => Some(s.clone()), _ => None }
}

pub(super) fn hg_map_version(v: &Value, platform: &str) -> Value {
    let file = hg_file(v, platform);
    json!({"id": v["name"], "name": v["name"], "version_number": v["name"], "channel": channel(v), "date": v["createdAt"], "downloads": v["stats"]["totalDownloads"],
        "files": file.iter().map(|f| json!({"filename": f["filename"], "size": f["size"], "primary": true})).collect::<Vec<_>>(),
        "game_versions": v["platformDependencies"][platform], "loaders": [platform.to_ascii_lowercase()],
        "dependencies": dep_list(v, platform).filter_map(|d| Some(json!({"project_id": dep_project(d)?, "kind": if d["required"] == true { "required" } else { "optional" }}))).collect::<Vec<_>>(),
        "downloadable": file.is_some(), "external_url": if file.is_some() { Value::Null } else { json!(hg_external(v, platform)) },
        "warning": if file.is_some() { Value::Null } else { json!(match hg_external(v, platform) { Some(u) => format!("Not hosted on Hangar; download it from {u} and upload it in Files."), None => "Not hosted on Hangar, so Moss cannot install it.".to_string() }) }})
}

/// Versions of a project that fit the server's platform and game version (the API filters; hosting is checked by callers).
pub(super) async fn hg_versions(st: &St, slug: &str, c: &Compat) -> Result<Vec<Value>> {
    let platform = hangar_platform(c).ok_or_else(|| anyhow!("Hangar has no content for this runtime"))?;
    let mut q = vec![("limit", "25".to_string()), ("offset", "0".to_string()), ("platform", platform.to_string())];
    if let Some(gv) = &c.game_version { q.push(("platformVersion", gv.clone())); }
    Ok(hg(st, &format!("/projects/{slug}/versions"), &q).await?["result"].as_array().cloned().unwrap_or_default())
}

/// Newest hosted version, preferring releases over snapshots over alphas.
pub(super) fn hg_pick_latest<'a>(vs: &'a [Value], platform: &str) -> Option<&'a Value> {
    let ok = |v: &&Value| hg_file(v, platform).is_some();
    ["release", "beta", "alpha"].iter().find_map(|ch| vs.iter().filter(ok).find(|v| channel(v) == *ch))
}

/// The newest installable version for an installed project, if any (update checks and upgrades).
pub(super) async fn hg_latest(st: &St, slug: &str, c: &Compat) -> Result<Option<Value>> {
    let platform = hangar_platform(c).unwrap_or("PAPER");
    Ok(hg_pick_latest(&hg_versions(st, slug, c).await?, platform).cloned())
}

pub(super) fn hg_hit(p: &Value) -> Value {
    json!({"project_id": p["namespace"]["slug"], "slug": p["namespace"]["slug"], "title": p["name"], "description": p["description"], "author": p["namespace"]["owner"], "icon_url": http_url(&p["avatarUrl"]),
        "downloads": p["stats"]["downloads"], "follows": p["stats"]["stars"], "categories": p["category"].as_str().map(|c| json!([c.replace('_', " ")])).unwrap_or(json!([])),
        "updated": p["lastUpdated"], "client_side": null, "server_side": null})
}

fn page_url(p: &Value, slug: &str) -> String { format!("https://hangar.papermc.io/{}/{slug}", p["namespace"]["owner"].as_str().filter(|o| valid_id(o)).unwrap_or("")) }

pub(super) async fn search_hangar(st: &St, s: &Srv, c: &Compat, q: &HashMap<String, String>) -> ApiResult<Json<Value>> {
    let platform = require_hangar(c)?;
    let limit = q.get("limit").and_then(|v| v.parse::<u32>().ok()).unwrap_or(20).clamp(1, 25);
    let offset = q.get("offset").and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
    let text: String = q.get("q").map(|t| t.trim().chars().take(100).collect()).unwrap_or_default();
    let sort = match q.get("sort").map(|s| s.as_str()) { Some("updated") => "-updated", Some("follows") => "-stars", _ => "-downloads" };
    let mut query = vec![("limit", limit.to_string()), ("offset", offset.to_string()), ("platform", platform.to_string()), ("sort", sort.to_string())];
    if platform == "PAPER" { query.push(("version", s.game_version.clone())); }
    if !text.is_empty() { query.push(("q", text)); }
    let v = hg(st, "/projects", &query).await.map_err(upstream)?;
    let hits: Vec<Value> = v["result"].as_array().into_iter().flatten().filter(|p| p["namespace"]["slug"].as_str().is_some_and(hangar_id)).map(hg_hit).collect();
    Ok(Json(json!({"hits": hits, "total": v["pagination"]["count"], "compat": {"kind": c.kind, "loader": c.loaders[0], "game_version": s.game_version}})))
}

pub(super) async fn project_hangar(st: &St, pid: &str, c: &Compat) -> ApiResult<Json<Value>> {
    let platform = require_hangar(c)?;
    let p = hg(st, &format!("/projects/{pid}"), &[]).await.map_err(upstream)?;
    let slug = p["namespace"]["slug"].as_str().unwrap_or(pid).to_string();
    let vs = hg_versions(st, &slug, c).await.map_err(upstream)?;
    let link = |needle: &str| p["settings"]["links"].as_array().into_iter().flatten().flat_map(|g| g["links"].as_array().into_iter().flatten()).find(|l| l["name"].as_str().is_some_and(|n| n.to_ascii_lowercase().contains(needle))).map(|l| http_url(&l["url"])).unwrap_or(Value::Null);
    Ok(Json(json!({
        "project_id": slug, "title": p["name"], "description": p["description"], "body": p["mainPageContent"].as_str().filter(|b| !b.is_empty()).map(|b| json!(b)).unwrap_or_else(|| p["description"].clone()), "icon_url": http_url(&p["avatarUrl"]),
        "links": {"source": link("source"), "issues": link("issue"), "wiki": link("wiki"), "page": page_url(&p, &slug)},
        "license": p["settings"]["license"]["name"], "gallery": [],
        "versions": vs.iter().take(25).map(|v| hg_map_version(v, platform)).collect::<Vec<_>>(),
    })))
}

pub(super) async fn build_plan_hg(st: &St, s: &Srv, c: &Compat, project_id: &str, version_id: Option<&str>) -> Result<(Vec<Step>, Vec<String>)> {
    if !hangar_id(project_id) { bail!("invalid project id"); }
    let platform = hangar_platform(c).ok_or_else(|| anyhow!("Hangar has no content for this runtime"))?;
    let mut warnings: Vec<String> = vec![];
    if s.runtime == "folia" { warnings.push("Folia needs plugins that explicitly declare Folia support; Hangar does not filter for it, so check the project page.".into()); }
    if lifecycle_running(st, s) { warnings.push("The server is running. Restart it to load the change.".into()); }
    let installed = db_items(st, &s.id).await?;
    let mut steps: Vec<Step> = vec![];
    let mut queue: Vec<(String, Option<String>)> = vec![(project_id.to_string(), version_id.map(String::from))];
    let mut seen: Vec<String> = vec![];
    while let Some((pid, vid)) = queue.pop() {
        if seen.contains(&pid) || steps.len() >= 12 { continue; }
        seen.push(pid.clone());
        let proj = hg(st, &format!("/projects/{pid}"), &[]).await?;
        let slug = proj["namespace"]["slug"].as_str().filter(|s| hangar_id(s)).unwrap_or(&pid).to_string();
        if steps.iter().any(|x| x.project_id == slug) { continue; }
        let title = proj["name"].as_str().unwrap_or("project").to_string();
        let is_root = pid == project_id || slug == project_id;
        let existing = installed.iter().find(|i| i["provider"] == "hangar" && i["project_id"] == slug.as_str());
        if !is_root && existing.is_some() { continue; }
        let vs = hg_versions(st, &slug, c).await?;
        let version = match vid {
            Some(v) => vs.iter().find(|x| x["name"] == v.as_str()).cloned().ok_or_else(|| anyhow!("{title} {v} is not compatible with {} {}", s.runtime, s.game_version))?,
            None => match hg_pick_latest(&vs, platform) {
                Some(v) => v.clone(),
                None if vs.is_empty() => bail!("no version of {title} is compatible with {} {}", s.runtime, s.game_version),
                None => { if !is_root { warnings.push(format!("{title} is required but is only available outside Hangar{}. Install it manually.", hg_external(&vs[0], platform).map(|u| format!(" ({u})")).unwrap_or_default())); continue; } vs[0].clone() }
            },
        };
        let vname = version["name"].as_str().unwrap_or_default().to_string();
        let Some(file) = hg_file(&version, platform) else {
            let hint = hg_external(&version, platform).map(|u| format!(" Download it from {u} and upload it in Files.")).unwrap_or_default();
            if is_root { bail!("{title} {vname} is not hosted on Hangar, so Moss cannot install it.{hint}"); }
            warnings.push(format!("{title} {vname} is required but not hosted on Hangar. Install it manually.{hint}"));
            continue;
        };
        safe_filename(file["filename"].as_str().unwrap_or_default(), c.ext)?;
        if file["hashes"]["sha256"].as_str().is_none() { bail!("{title} {vname} has no checksum on Hangar, so Moss will not install it"); }
        if channel(&version) != "release" { warnings.push(format!("{title} {vname} is a {} build.", channel(&version))); }
        if let Some(e) = existing && e["version_id"] == vname.as_str() && !is_root { continue; }
        let replaces = existing.map(|e| (e["id"].as_str().unwrap_or_default().to_string(), e["filename"].as_str().unwrap_or_default().to_string()));
        let (action, reason) = match &replaces { Some(_) => ("replace", "replaces the installed version".to_string()), None if is_root => ("install", "requested".to_string()), None => ("install", "required dependency".to_string()) };
        for d in dep_list(&version, platform).filter(|d| d["required"] == true) {
            match dep_project(d) {
                Some(dp) => queue.push((dp, None)),
                None => {
                    let (name, link) = (d["name"].as_str().unwrap_or("a plugin"), hg_external(&json!({"downloads": {platform: {"externalUrl": d["externalUrl"]}}}), platform));
                    let w = format!("{title} needs {name}, which is not on Hangar. Install it manually{}.", link.map(|u| format!(" ({u})")).unwrap_or_default());
                    if !warnings.contains(&w) { warnings.push(w); }
                }
            }
        }
        steps.push(Step { prov: Prov::Hangar, action, project_id: slug, title, version: json!({"id": vname, "version_number": vname, "version_type": channel(&version)}), file, replaces, reason });
    }
    steps.reverse();
    Ok((steps, warnings))
}

/// Matches an unmanaged file to a Hangar version by SHA-256 (`/versions/hash/{sha256}` answers with the project).
/// Returns `(slug, title, version name)`; the version name is empty when it is not among the project's newest versions.
pub(super) async fn hg_identify(st: &St, c: &Compat, sha256: &str) -> Result<Option<(String, String, String)>> {
    let Some(platform) = hangar_platform(c) else { return Ok(None) };
    if sha256.len() != 64 || !sha256.chars().all(|ch| ch.is_ascii_hexdigit()) { return Ok(None); }
    let p = match hg_send(st.http.get(format!("{}/versions/hash/{sha256}", base(st)))).await {
        Ok(p) => p,
        Err(e) if e.downcast_ref::<cf::CfError>().is_some_and(|c| !c.rate_limited && c.msg == "Not found on Hangar.") => return Ok(None),
        Err(e) => return Err(e),
    };
    let Some(slug) = p["namespace"]["slug"].as_str().filter(|s| hangar_id(s)) else { return Ok(None) };
    let vs = hg(st, &format!("/projects/{slug}/versions"), &[("limit", "25".into()), ("platform", platform.into())]).await?;
    let name = vs["result"].as_array().into_iter().flatten()
        .find(|v| v["downloads"][platform]["fileInfo"]["sha256Hash"].as_str().is_some_and(|h| h.eq_ignore_ascii_case(sha256)))
        .and_then(|v| v["name"].as_str()).unwrap_or_default().to_string();
    Ok(Some((slug.to_string(), p["name"].as_str().unwrap_or(slug).to_string(), name)))
}

#[cfg(test)]
mod tests;
