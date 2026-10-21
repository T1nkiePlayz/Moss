//! Catalogue browsing handlers: search, project details and the provider list (Modrinth, CurseForge and Hangar).
use super::*;

pub async fn search(State(st): State<St>, user: User, Query(q): Query<HashMap<String, String>>) -> ApiResult<Json<Value>> {
    let sid = q.get("server_id").ok_or_else(|| ApiError::bad("validation", "server_id is required"))?;
    let (s, c) = compat_for(&st, &user, sid, "server.view").await?;
    match Prov::parse(q.get("provider").map(|p| p.as_str()))? {
        Prov::Curseforge => return search_cf(&st, &user, &s, &c, &q).await,
        Prov::Hangar => return search_hangar(&st, &s, &c, &q).await,
        Prov::Modrinth => {}
    }
    let mut facets = vec![vec![format!("project_type:{}", c.project_type)], c.loaders.iter().map(|l| format!("categories:{l}")).collect()];
    if let Some(gv) = &c.game_version { facets.push(vec![format!("versions:{gv}")]); }
    if c.project_type == "mod" { facets.push(vec!["server_side:required".into(), "server_side:optional".into()]); }
    if let Some(cat) = q.get("category").filter(|c| valid_id(c)) { facets.push(vec![format!("categories:{cat}")]); }
    let index = match q.get("sort").map(|s| s.as_str()) { Some("downloads") => "downloads", Some("updated") => "updated", Some("follows") => "follows", _ => "relevance" };
    let limit = q.get("limit").and_then(|v| v.parse::<u32>().ok()).unwrap_or(20).clamp(1, 50);
    let offset = q.get("offset").and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
    let v = mr(&st, "/search", &[("query", q.get("q").cloned().unwrap_or_default().chars().take(100).collect()), ("facets", serde_json::to_string(&facets).unwrap()), ("index", index.into()), ("limit", limit.to_string()), ("offset", offset.to_string())]).await.map_err(upstream)?;
    let hits: Vec<Value> = v["hits"].as_array().into_iter().flatten().map(|h| json!({
        "project_id": h["project_id"], "slug": h["slug"], "title": h["title"], "description": h["description"], "author": h["author"], "icon_url": h["icon_url"],
        "downloads": h["downloads"], "follows": h["follows"], "categories": h["display_categories"].as_array().filter(|a| !a.is_empty()).map(|a| Value::Array(a.clone())).unwrap_or_else(|| h["categories"].clone()),
        "updated": h["date_modified"], "client_side": h["client_side"], "server_side": h["server_side"],
    })).collect();
    Ok(Json(json!({"hits": hits, "total": v["total_hits"], "compat": {"kind": c.kind, "loader": c.loaders[0], "game_version": s.game_version}})))
}

pub(super) async fn search_cf(st: &St, user: &User, s: &Srv, c: &Compat, q: &HashMap<String, String>) -> ApiResult<Json<Value>> {
    let class = require_cf(st, c, user.token.as_deref())?;
    let limit = q.get("limit").and_then(|v| v.parse::<u32>().ok()).unwrap_or(20).clamp(1, 50);
    let offset = q.get("offset").and_then(|v| v.parse::<u32>().ok()).unwrap_or(0).min(10_000 - limit);
    let sort = q.get("sort").map(|s| s.as_str()).unwrap_or_default();
    let mut query = vec![("classId", class.to_string()), ("sortField", cf::sort_field(sort).to_string()), ("sortOrder", "desc".into()), ("pageSize", limit.to_string()), ("index", offset.to_string()), ("gameVersion", s.game_version.clone())];
    if let Some(t) = q.get("q").map(|t| t.trim()).filter(|t| !t.is_empty()) { query.push(("searchFilter", t.chars().take(100).collect())); }
    if c.cf_loader != 0 { query.push(("modLoaderType", c.cf_loader.to_string())); }
    let v = cf::get(st, user.token.as_deref(), "/v1/mods/search", &query).await.map_err(upstream)?;
    let hits: Vec<Value> = v["data"].as_array().into_iter().flatten().filter(|m| cf::distributable(m)).map(cf::hit).collect();
    Ok(Json(json!({"hits": hits, "total": v["pagination"]["totalCount"], "compat": {"kind": c.kind, "loader": c.loaders[0], "game_version": s.game_version}})))
}

pub async fn project(State(st): State<St>, user: User, Path(pid): Path<String>, Query(q): Query<HashMap<String, String>>) -> ApiResult<Json<Value>> {
    if !valid_id(&pid) { return Err(ApiError::bad("validation", "invalid project id")); }
    let sid = q.get("server_id").ok_or_else(|| ApiError::bad("validation", "server_id is required"))?;
    let (_s, c) = compat_for(&st, &user, sid, "server.view").await?;
    match Prov::parse(q.get("provider").map(|p| p.as_str()))? {
        Prov::Curseforge => return project_cf(&st, &user, &pid, &c).await,
        Prov::Hangar => return project_hangar(&st, &pid, &c).await,
        Prov::Modrinth => {}
    }
    let p = mr(&st, &format!("/project/{pid}"), &[]).await.map_err(upstream)?;
    let vs = compatible_versions(&st, &pid, &c).await.map_err(upstream)?;
    Ok(Json(json!({
        "project_id": p["id"], "title": p["title"], "description": p["description"], "body": p["body"], "icon_url": p["icon_url"],
        "links": {"source": http_url(&p["source_url"]), "issues": http_url(&p["issues_url"]), "wiki": http_url(&p["wiki_url"]), "page": format!("https://modrinth.com/project/{}", p["slug"].as_str().unwrap_or_default())},
        "license": p["license"]["name"], "gallery": p["gallery"].as_array().into_iter().flatten().filter_map(|g| g["url"].as_str()).take(6).collect::<Vec<_>>(),
        "versions": vs.iter().take(40).map(map_version).collect::<Vec<_>>(),
    })))
}

pub(super) async fn project_cf(st: &St, user: &User, pid: &str, c: &Compat) -> ApiResult<Json<Value>> {
    require_cf(st, c, user.token.as_deref())?;
    if pid.parse::<u32>().is_err() { return Err(ApiError::bad("validation", "invalid project id")); }
    let tok = user.token.as_deref();
    let m = cf::get(st, tok, &format!("/v1/mods/{pid}"), &[]).await.map_err(upstream)?;
    let m = &m["data"];
    let files = cf_files(st, tok, pid, c).await.map_err(upstream)?;
    let slug = m["slug"].as_str().unwrap_or_default();
    Ok(Json(json!({
        "project_id": pid, "title": m["name"], "description": m["summary"], "body": m["summary"], "icon_url": m["logo"]["thumbnailUrl"],
        "links": {"source": http_url(&m["links"]["sourceUrl"]), "issues": http_url(&m["links"]["issuesUrl"]), "wiki": http_url(&m["links"]["wikiUrl"]), "page": match http_url(&m["links"]["websiteUrl"]) { Value::Null => json!(format!("https://www.curseforge.com/minecraft/{slug}")), u => u }},
        "license": null, "gallery": m["screenshots"].as_array().into_iter().flatten().filter_map(|g| g["thumbnailUrl"].as_str()).take(6).collect::<Vec<_>>(),
        "versions": files.iter().filter(|f| f["isServerPack"] != json!(true)).take(40).map(cf::version).collect::<Vec<_>>(),
    })))
}

/// Files of a mod that fit the server's game version and loader (the API filters; availability is checked here).
pub(super) async fn cf_files(st: &St, token: Option<&str>, pid: &str, c: &Compat) -> Result<Vec<Value>> {
    let mut q = vec![("pageSize", "50".to_string())];
    if let Some(gv) = &c.game_version { q.push(("gameVersion", gv.clone())); if c.cf_loader != 0 { q.push(("modLoaderType", c.cf_loader.to_string())); } }
    let v = cf::get(st, token, &format!("/v1/mods/{pid}/files"), &q).await?;
    Ok(v["data"].as_array().into_iter().flatten().filter(|f| f["isAvailable"] != json!(false)).cloned().collect())
}

#[derive(Deserialize)] pub struct ProvQ { server_id: Option<String> }
/// Which catalogues exist on this instance and, for a server, which of them have content for its runtime.
pub async fn providers(State(st): State<St>, user: User, Query(q): Query<ProvQ>) -> ApiResult<Json<Value>> {
    let compat = match &q.server_id { Some(id) => Some(compat_for(&st, &user, id, "server.view").await.map(|x| x.1).ok()), None => None };
    let (cf_ok, cf_why) = cf::availability(&st, user.token.as_deref());
    let mr_supported = compat.as_ref().is_none_or(|c| c.is_some());
    let cf_supported = compat.as_ref().is_none_or(|c| c.as_ref().is_some_and(|c| c.cf_class.is_some()));
    let reason = |ok: bool, supported: bool, why: Option<&str>, none: &str| if !ok { why.map(String::from) } else if !supported { Some(none.to_string()) } else { None };
    let hg_supported = compat.as_ref().is_none_or(|c| c.as_ref().is_some_and(|c| hangar_platform(c).is_some()));
    Ok(Json(json!({"providers": [
        {"id": "modrinth", "name": "Modrinth", "available": true, "supported": mr_supported, "reason": reason(true, mr_supported, None, "Modrinth has no content for this runtime.")},
        {"id": "curseforge", "name": "CurseForge", "available": cf_ok, "supported": cf_supported, "reason": reason(cf_ok, cf_supported, cf_why, "CurseForge has no content for this runtime.")},
        {"id": "hangar", "name": "Hangar", "available": true, "supported": hg_supported, "reason": reason(true, hg_supported, None, "Hangar only has plugins for Paper, Purpur, Folia and Velocity.")},
    ]})))
}
