//! `GET /modpacks/search`: find modpacks on Modrinth or CurseForge before inspecting one (the pick then goes through
//! `POST /modpacks/inspect`). Not tied to a server, so it uses the same permission as inspect.
use super::*;

const CF_MODPACK_CLASS: u32 = 4471;
const LOADERS: [&str; 4] = ["fabric", "forge", "neoforge", "quilt"];

pub(super) struct Page { pub q: String, pub offset: u32, pub limit: u32 }

pub(super) fn page(q: &HashMap<String, String>) -> Page {
    let limit = q.get("limit").and_then(|v| v.parse::<u32>().ok()).unwrap_or(20).clamp(1, 50);
    let offset = q.get("offset").and_then(|v| v.parse::<u32>().ok()).unwrap_or(0).min(10_000 - limit);
    Page { q: q.get("q").map(|t| t.trim().chars().take(100).collect()).unwrap_or_default(), offset, limit }
}

/// Query parameters for Modrinth's `/search`: modpacks that can run on a server.
pub(super) fn mr_query(p: &Page) -> Vec<(&'static str, String)> {
    let facets = vec![vec!["project_type:modpack".to_string()], vec!["server_side:required".into(), "server_side:optional".into()]];
    let index = if p.q.is_empty() { "downloads" } else { "relevance" };
    vec![("query", p.q.clone()), ("facets", serde_json::to_string(&facets).unwrap()), ("index", index.into()), ("limit", p.limit.to_string()), ("offset", p.offset.to_string())]
}

/// Query parameters for CurseForge's `/v1/mods/search`: Minecraft modpacks.
pub(super) fn cf_query(p: &Page) -> Vec<(&'static str, String)> {
    let mut q = vec![("gameId", cf::GAME_MINECRAFT.to_string()), ("classId", CF_MODPACK_CLASS.to_string()), ("sortField", cf::sort_field(if p.q.is_empty() { "downloads" } else { "" }).to_string()), ("sortOrder", "desc".into()), ("pageSize", p.limit.to_string()), ("index", p.offset.to_string())];
    if !p.q.is_empty() { q.push(("searchFilter", p.q.clone())); }
    q
}

pub(super) fn mr_items(v: &Value) -> Vec<Value> {
    v["hits"].as_array().into_iter().flatten().map(|h| {
        let cats = h["categories"].as_array().into_iter().flatten().filter_map(|c| c.as_str());
        let loaders: Vec<&str> = cats.filter(|c| LOADERS.contains(c)).collect();
        let gv: Vec<&Value> = h["versions"].as_array().into_iter().flatten().rev().take(3).collect();
        json!({"provider": "modrinth", "project_id": h["project_id"], "slug": h["slug"], "title": h["title"], "description": h["description"], "author": h["author"], "icon_url": h["icon_url"], "downloads": h["downloads"], "game_versions": gv, "loaders": loaders})
    }).collect()
}

pub(super) fn cf_items(v: &Value) -> Vec<Value> {
    v["data"].as_array().into_iter().flatten().filter(|m| cf::distributable(m)).map(|m| {
        let idx = m["latestFilesIndexes"].as_array().into_iter().flatten();
        let (mut gv, mut loaders): (Vec<String>, Vec<&str>) = (vec![], vec![]);
        for i in idx {
            if let Some(g) = i["gameVersion"].as_str() && !gv.iter().any(|x| x == g) && gv.len() < 3 { gv.push(g.to_string()); }
            let l = match i["modLoader"].as_u64() { Some(1) => "forge", Some(4) => "fabric", Some(5) => "quilt", Some(6) => "neoforge", _ => "" };
            if !l.is_empty() && !loaders.contains(&l) { loaders.push(l); }
        }
        json!({"provider": "curseforge", "project_id": m["id"].as_u64().map(|i| i.to_string()), "slug": m["slug"], "title": m["name"], "description": m["summary"], "author": m["authors"][0]["name"], "icon_url": m["logo"]["thumbnailUrl"], "downloads": m["downloadCount"], "game_versions": gv, "loaders": loaders})
    }).collect()
}

pub async fn search(State(st): State<St>, user: User, Query(q): Query<HashMap<String, String>>) -> ApiResult<Json<Value>> {
    modpack::may_stage(&st, &user).await?;
    let p = page(&q);
    let (items, total) = match q.get("provider").map(|s| s.as_str()).unwrap_or("modrinth") {
        "modrinth" => {
            let v = mr(&st, "/search", &mr_query(&p)).await.map_err(upstream)?;
            (mr_items(&v), v["total_hits"].clone())
        }
        "curseforge" => {
            if let (false, Some(why)) = cf::availability(&st, user.token.as_deref()) { return Err(ApiError::new(StatusCode::CONFLICT, "provider.unavailable", why)); }
            let v = cf::get(&st, user.token.as_deref(), "/v1/mods/search", &cf_query(&p)).await.map_err(upstream)?;
            (cf_items(&v), v["pagination"]["totalCount"].clone())
        }
        _ => return Err(ApiError::bad("validation", "provider must be modrinth or curseforge").field("provider", "modrinth | curseforge")),
    };
    Ok(Json(json!({"items": items, "next_offset": next_offset(&p, total.as_u64().unwrap_or(0)), "total": total})))
}

/// Where the next page starts, or None on the last one. Pages are counted upstream, so CurseForge packs that are
/// dropped from a page (no third-party distribution) never shift the next page. Upstream stops at 10,000 results.
fn next_offset(p: &Page, total: u64) -> Option<u32> {
    let n = p.offset + p.limit;
    (u64::from(n) < total.min(10_000)).then_some(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(pairs: &[(&str, &str)]) -> HashMap<String, String> { pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() }

    #[test]
    fn page_clamps_and_trims() {
        let at = |o: &str, total| next_offset(&page(&q(&[("offset", o)])), total);
        assert_eq!((at("0", 45), at("20", 45), at("40", 45), at("9980", 50_000)), (Some(20), Some(40), None, None));
        let p = page(&q(&[("q", "  fabulous  "), ("limit", "999"), ("offset", "99999")]));
        assert_eq!((p.q.as_str(), p.limit, p.offset), ("fabulous", 50, 9950));
        let d = page(&q(&[]));
        assert_eq!((d.q.as_str(), d.limit, d.offset), ("", 20, 0));
    }

    #[test]
    fn modrinth_query_filters_modpacks() {
        let k = mr_query(&page(&q(&[("q", "opti")])));
        let get = |n: &str| k.iter().find(|(a, _)| *a == n).unwrap().1.clone();
        assert!(get("facets").contains("project_type:modpack"));
        assert_eq!(get("index"), "relevance");
        assert_eq!(get("query"), "opti");
        assert_eq!(mr_query(&page(&q(&[]))).iter().find(|(a, _)| *a == "index").unwrap().1, "downloads");
    }

    #[test]
    fn curseforge_query_targets_modpack_class() {
        let k = cf_query(&page(&q(&[("q", "rlcraft"), ("offset", "40")])));
        let get = |n: &str| k.iter().find(|(a, _)| *a == n).map(|x| x.1.clone());
        assert_eq!((get("gameId").as_deref(), get("classId").as_deref(), get("index").as_deref(), get("searchFilter").as_deref()), (Some("432"), Some("4471"), Some("40"), Some("rlcraft")));
        assert!(cf_query(&page(&q(&[]))).iter().all(|(a, _)| *a != "searchFilter"));
    }

    #[test]
    fn parses_modrinth_hits() {
        let v = json!({"total_hits": 1, "hits": [{"project_id": "P1", "slug": "fo", "title": "Fab", "description": "d", "author": "a", "icon_url": null, "downloads": 5, "categories": ["fabric", "adventure"], "versions": ["1.19", "1.20", "1.21"]}]});
        let i = mr_items(&v);
        assert_eq!(i[0]["provider"], "modrinth");
        assert_eq!(i[0]["project_id"], "P1");
        assert_eq!(i[0]["loaders"], json!(["fabric"]));
        assert_eq!(i[0]["game_versions"], json!(["1.21", "1.20", "1.19"]));
    }

    #[test]
    fn parses_curseforge_hits_and_drops_undistributable() {
        let v = json!({"data": [
            {"id": 9, "slug": "pack", "name": "Pack", "summary": "s", "authors": [{"name": "bob"}], "logo": {"thumbnailUrl": "https://x/i.png"}, "downloadCount": 12.0,
             "latestFilesIndexes": [{"gameVersion": "1.20.1", "modLoader": 1}, {"gameVersion": "1.20.1", "modLoader": 4}, {"gameVersion": "1.19.2", "modLoader": 0}]},
            {"id": 10, "name": "Hidden", "allowModDistribution": false}]});
        let i = cf_items(&v);
        assert_eq!(i.len(), 1);
        assert_eq!((&i[0]["project_id"], &i[0]["author"], &i[0]["loaders"], &i[0]["game_versions"]), (&json!("9"), &json!("bob"), &json!(["forge", "fabric"]), &json!(["1.20.1", "1.19.2"])));
    }
}
