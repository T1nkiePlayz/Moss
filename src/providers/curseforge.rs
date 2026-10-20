//! CurseForge provider. Two ways in, chosen by configuration:
//!  * `CURSEFORGE_API_KEY` set: Moss talks to api.curseforge.com directly with that key (your own key; never shipped).
//!  * otherwise, in Supabase auth mode: through the project's `curseforge` edge function, which holds the key and
//!    is called with the signed-in user's access token (see supabase/functions/curseforge).
//!
//! Respecting the API terms: nothing from the API is cached or stored beyond what an installed item needs
//! (ids, title, version label, filename); calls are throttled and serialised through one limiter; a 429 pauses
//! every CurseForge call for its Retry-After; files whose authors disabled third-party downloads are never fetched
//! by guessing CDN URLs.
use crate::state::St;
use anyhow::{Result, anyhow};
use reqwest::{Method, header::{HeaderMap, HeaderName, HeaderValue}};
use serde_json::{Value, json};
use std::{sync::OnceLock, time::{Duration, Instant}};

/// Minimum gap between CurseForge calls from this instance (about 2.8 per second); CurseForge publishes no figure.
const MIN_GAP: Duration = Duration::from_millis(350);
/// A queued call that would wait longer than this fails fast instead of piling up.
const MAX_QUEUE: Duration = Duration::from_secs(8);
pub const CONCURRENCY: usize = 4;

#[derive(Debug)]
pub struct CfError { pub rate_limited: bool, pub msg: String }
impl std::fmt::Display for CfError { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(&self.msg) } }
impl std::error::Error for CfError {}
fn cf_err(rate_limited: bool, msg: impl Into<String>) -> anyhow::Error { CfError { rate_limited, msg: msg.into() }.into() }

#[derive(Default)]
pub struct Limiter { next: Option<Instant>, blocked_until: Option<Instant> }

pub enum Mode { Key(String), Proxy(String) }

pub fn mode(st: &St) -> Option<Mode> {
    let c = &st.cfg;
    if !c.curseforge_api_key.is_empty() { return Some(Mode::Key(c.curseforge_api_key.clone())); }
    if c.dev() || c.supabase_url.is_empty() || c.curseforge_proxy == "off" { return None; }
    let base = if c.curseforge_proxy.is_empty() { format!("{}/functions/v1/curseforge", c.supabase_url.trim_end_matches('/')) } else { c.curseforge_proxy.trim_end_matches('/').to_string() };
    Some(Mode::Proxy(base))
}

/// (available on this instance for this caller, human reason when not)
pub fn availability(st: &St, token: Option<&str>) -> (bool, Option<&'static str>) {
    match mode(st) {
        None => (false, Some("CurseForge is not configured on this Moss instance.")),
        Some(Mode::Proxy(_)) if token.is_none() => (false, Some("CurseForge needs a signed-in session.")),
        Some(_) => (true, None),
    }
}

fn client() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(|| reqwest::Client::builder()
        .user_agent(format!("moss/{}", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::none()) // the key must never follow a redirect to another host
        .connect_timeout(Duration::from_secs(10)).timeout(Duration::from_secs(30)).build().expect("curseforge client"))
}
pub fn dl_client() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(|| reqwest::Client::builder()
        .user_agent(format!("moss/{}", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(15)).read_timeout(Duration::from_secs(60)).build().expect("curseforge download client"))
}

fn hv(v: &str) -> Result<HeaderValue> { let mut h = HeaderValue::from_str(v).map_err(|_| anyhow!("invalid header value"))?; h.set_sensitive(true); Ok(h) }

fn auth_headers(st: &St, token: Option<&str>) -> Result<(String, HeaderMap)> {
    let mut h = HeaderMap::new();
    match mode(st).ok_or_else(|| cf_err(false, "CurseForge is not configured on this Moss instance."))? {
        Mode::Key(k) => { h.insert(HeaderName::from_static("x-api-key"), hv(&k)?); Ok((st.cfg.curseforge_api_url.trim_end_matches('/').to_string(), h)) }
        Mode::Proxy(base) => {
            let t = token.ok_or_else(|| cf_err(false, "CurseForge needs a signed-in session."))?;
            h.insert(reqwest::header::AUTHORIZATION, hv(&format!("Bearer {t}"))?);
            h.insert(HeaderName::from_static("apikey"), hv(&st.cfg.supabase_anon_key)?);
            Ok((base, h))
        }
    }
}

async fn throttle(st: &St) -> Result<()> {
    let at = {
        let mut l = st.cf.lock().unwrap();
        let now = Instant::now();
        if let Some(b) = l.blocked_until.filter(|b| *b > now) { return Err(cf_err(true, format!("CurseForge asked Moss to slow down; try again in {}s.", (b - now).as_secs() + 1))); }
        let at = l.next.map_or(now, |n| n.max(now));
        if at - now > MAX_QUEUE { return Err(cf_err(true, "CurseForge requests are queued up; try again in a few seconds.")); }
        l.next = Some(at + MIN_GAP);
        at
    };
    tokio::time::sleep_until(at.into()).await;
    Ok(())
}

fn block(st: &St, secs: u64) { st.cf.lock().unwrap().blocked_until = Some(Instant::now() + Duration::from_secs(secs.clamp(1, 120))); }

/// One CurseForge API call (`path` like `/v1/mods/search`). Reads are retried once on transient failures.
pub async fn call(st: &St, token: Option<&str>, method: Method, path: &str, query: &[(&str, String)], body: Option<Value>) -> Result<Value> {
    let (base, headers) = auth_headers(st, token)?;
    let _permit = st.cf_sem.acquire().await?;
    let mut last = cf_err(false, "CurseForge is unreachable.");
    for attempt in 0..2 {
        throttle(st).await?;
        let mut rq = client().request(method.clone(), format!("{base}{path}")).headers(headers.clone()).query(query).header("accept", "application/json");
        if let Some(b) = &body { rq = rq.json(b); }
        match rq.send().await {
            Err(e) => { last = cf_err(false, format!("could not reach CurseForge: {}", e.without_url())); }
            Ok(r) => {
                let status = r.status();
                if status.is_success() { return r.json::<Value>().await.map_err(|_| cf_err(false, "CurseForge sent an unreadable response.")); }
                let retry = r.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok());
                let text = r.text().await.unwrap_or_default();
                let proxied = serde_json::from_str::<Value>(&text).ok().and_then(|v| v["error"]["message"].as_str().map(String::from));
                match status.as_u16() {
                    429 => { block(st, retry.unwrap_or(30)); return Err(cf_err(true, proxied.unwrap_or_else(|| "CurseForge is rate limiting Moss; try again shortly.".into()))); }
                    401 | 403 => return Err(cf_err(false, proxied.unwrap_or_else(|| "CurseForge rejected the request: the API key is invalid or the quota was exceeded.".into()))),
                    404 => return Err(cf_err(false, "not found on CurseForge")),
                    503 if proxied.is_some() && attempt == 0 && text.contains("not_configured") => return Err(cf_err(false, proxied.unwrap())),
                    s if (500..600).contains(&s) => { last = cf_err(false, proxied.unwrap_or_else(|| format!("CurseForge answered {s}"))); }
                    s => return Err(cf_err(false, proxied.unwrap_or_else(|| format!("CurseForge answered {s}")))),
                }
            }
        }
        if attempt == 0 { tokio::time::sleep(Duration::from_millis(600)).await; }
    }
    Err(last)
}

pub async fn get(st: &St, token: Option<&str>, path: &str, query: &[(&str, String)]) -> Result<Value> { call(st, token, Method::GET, path, query, None).await }

/// (url, headers, extra allowed host) for downloading a file. Proxy mode streams through the edge function by ids;
/// key mode uses the CDN URL CurseForge itself published for the file, with the key in a header.
pub fn download_target(st: &St, token: Option<&str>, mod_id: &str, file_id: &str, cdn_url: &str) -> Result<(String, HeaderMap, Option<String>)> {
    let (base, headers) = auth_headers(st, token)?;
    Ok(match mode(st) {
        Some(Mode::Proxy(_)) => {
            let host = reqwest::Url::parse(&base).ok().and_then(|u| u.host_str().map(String::from));
            (format!("{base}/download/{mod_id}/{file_id}"), headers, host)
        }
        _ => (cdn_url.to_string(), headers, None),
    })
}

// ---------- mapping to the shapes the rest of Moss uses ----------

pub const CLASS_BUKKIT: u32 = 5;
pub const CLASS_MODS: u32 = 6;
pub const CLASS_DATAPACKS: u32 = 6945;
pub const LOADER_FABRIC: u32 = 4;

pub fn sort_field(sort: &str) -> &'static str {
    match sort { "downloads" => "6", "updated" => "3", "follows" => "2", _ => "1" }
}

pub fn distributable(m: &Value) -> bool { m["allowModDistribution"] != json!(false) }

pub fn hit(m: &Value) -> Value {
    let cats: Vec<Value> = m["categories"].as_array().into_iter().flatten().filter_map(|c| c["name"].as_str()).take(3).map(|s| json!(s)).collect();
    json!({
        "project_id": m["id"].as_u64().map(|i| i.to_string()), "slug": m["slug"], "title": m["name"], "description": m["summary"],
        "author": m["authors"][0]["name"], "icon_url": m["logo"]["thumbnailUrl"], "downloads": m["downloadCount"], "follows": m["thumbsUpCount"],
        "categories": cats, "updated": m["dateModified"], "client_side": null, "server_side": null,
    })
}

const LOADER_NAMES: [(&str, &str); 6] = [("Forge", "forge"), ("Fabric", "fabric"), ("Quilt", "quilt"), ("NeoForge", "neoforge"), ("Cauldron", "cauldron"), ("LiteLoader", "liteloader")];

fn is_mc_version(s: &str) -> bool {
    let mut parts = s.split('.');
    let (a, b, c) = (parts.next(), parts.next(), parts.next());
    parts.next().is_none() && [a, b, c].iter().flatten().all(|p| !p.is_empty() && p.len() <= 3 && p.chars().all(|ch| ch.is_ascii_digit())) && b.is_some()
}

pub fn channel(f: &Value) -> &'static str { match f["releaseType"].as_u64() { Some(1) => "release", Some(2) => "beta", _ => "alpha" } }

/// The author marked this file client-only (tagged Client but not Server).
pub fn client_only(f: &Value) -> bool {
    let tags: Vec<&str> = f["gameVersions"].as_array().into_iter().flatten().filter_map(|v| v.as_str()).collect();
    tags.contains(&"Client") && !tags.contains(&"Server")
}

pub fn downloadable(f: &Value) -> bool { f["downloadUrl"].as_str().is_some_and(|u| !u.is_empty()) && f["isAvailable"] != json!(false) }

pub fn version(f: &Value) -> Value {
    let tags: Vec<&str> = f["gameVersions"].as_array().into_iter().flatten().filter_map(|v| v.as_str()).collect();
    let deps: Vec<Value> = f["dependencies"].as_array().into_iter().flatten().filter_map(|d| {
        let kind = match d["relationType"].as_u64()? { 3 => "required", 2 => "optional", 5 => "incompatible", _ => return None };
        Some(json!({"project_id": d["modId"].as_u64()?.to_string(), "kind": kind}))
    }).collect();
    let name = f["displayName"].as_str().or_else(|| f["fileName"].as_str()).unwrap_or_default();
    json!({
        "id": f["id"].as_u64().map(|i| i.to_string()), "name": name, "version_number": name, "channel": channel(f), "date": f["fileDate"], "downloads": f["downloadCount"],
        "files": [{"filename": f["fileName"], "size": f["fileLength"], "primary": true}],
        "game_versions": tags.iter().filter(|t| is_mc_version(t)).collect::<Vec<_>>(),
        "loaders": LOADER_NAMES.iter().filter(|(n, _)| tags.contains(n)).map(|(_, l)| *l).collect::<Vec<_>>(),
        "dependencies": deps, "downloadable": downloadable(f),
        "warning": if client_only(f) { json!("Marked client-only by its author; it may not work on a server.") } else if !downloadable(f) { json!("The author disabled automatic downloads for this file.") } else { Value::Null },
    })
}

/// Best installable file: release over beta over alpha, then newest.
pub fn pick_latest(files: &[Value]) -> Option<&Value> {
    files.iter().filter(|f| downloadable(f) && f["isServerPack"] != json!(true) && !client_only(f))
        .min_by(|a, b| (a["releaseType"].as_u64().unwrap_or(3), std::cmp::Reverse(a["fileDate"].as_str().unwrap_or_default())).cmp(&(b["releaseType"].as_u64().unwrap_or(3), std::cmp::Reverse(b["fileDate"].as_str().unwrap_or_default()))))
}

/// From a mod's `latestFilesIndexes`: the newest file id for a game version and loader (0 = any), preferring releases.
pub fn latest_index(m: &Value, game_version: Option<&str>, loader: u32) -> Option<(String, String)> {
    m["latestFilesIndexes"].as_array()?.iter()
        .filter(|i| game_version.is_none_or(|g| i["gameVersion"] == g) && (loader == 0 || i["modLoader"].as_u64().is_none_or(|l| l == loader as u64)))
        .min_by_key(|i| (i["releaseType"].as_u64().unwrap_or(3), std::cmp::Reverse(i["fileId"].as_u64().unwrap_or(0))))
        .and_then(|i| Some((i["fileId"].as_u64()?.to_string(), i["filename"].as_str().unwrap_or_default().to_string())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(id: u64, rt: u64, date: &str, url: Option<&str>, tags: &[&str]) -> Value {
        json!({"id": id, "releaseType": rt, "fileDate": date, "downloadUrl": url, "gameVersions": tags, "fileName": format!("a-{id}.jar"), "displayName": format!("A {id}"), "fileLength": 10})
    }

    #[test]
    fn picks_release_over_newer_beta_and_skips_undownloadable() {
        let fs = vec![
            file(1, 1, "2026-01-01", Some("https://edge.forgecdn.net/a"), &["1.21.1"]),
            file(2, 2, "2026-03-01", Some("https://edge.forgecdn.net/b"), &["1.21.1"]),
            file(3, 1, "2026-05-01", None, &["1.21.1"]),
        ];
        assert_eq!(pick_latest(&fs).unwrap()["id"], 1);
        assert!(pick_latest(&fs[2..]).is_none());
    }

    #[test]
    fn client_only_files_are_never_picked() {
        let fs = vec![file(1, 1, "2026-01-01", Some("https://edge.forgecdn.net/a"), &["1.21.1", "Client"]), file(2, 1, "2025-01-01", Some("https://edge.forgecdn.net/b"), &["1.21.1", "Client", "Server"])];
        assert_eq!(pick_latest(&fs).unwrap()["id"], 2);
    }

    #[test]
    fn version_mapping_exposes_loader_game_versions_and_dependencies() {
        let mut f = file(9, 3, "2026-01-01", None, &["1.21.1", "Fabric", "Server", "Java 21"]);
        f["dependencies"] = json!([{"modId": 306612, "relationType": 3}, {"modId": 5, "relationType": 1}, {"modId": 7, "relationType": 2}]);
        let v = version(&f);
        assert_eq!(v["channel"], "alpha");
        assert_eq!(v["game_versions"], json!(["1.21.1"]));
        assert_eq!(v["loaders"], json!(["fabric"]));
        assert_eq!(v["dependencies"].as_array().unwrap().len(), 2);
        assert_eq!(v["downloadable"], false);
    }

    #[test]
    fn hit_mapping_and_distribution_flag() {
        let m = json!({"id": 123, "slug": "x", "name": "X", "summary": "s", "authors": [{"name": "a"}], "logo": {"thumbnailUrl": "u"}, "downloadCount": 5, "thumbsUpCount": 2, "categories": [{"name": "Tools"}], "dateModified": "d", "allowModDistribution": false});
        assert_eq!(hit(&m)["project_id"], "123");
        assert!(!distributable(&m));
        assert!(distributable(&json!({"allowModDistribution": null})));
    }

    #[test]
    fn latest_index_matches_version_and_loader() {
        let m = json!({"latestFilesIndexes": [
            {"gameVersion": "1.21.1", "fileId": 10, "filename": "a.jar", "releaseType": 1, "modLoader": 1},
            {"gameVersion": "1.21.1", "fileId": 11, "filename": "b.jar", "releaseType": 1, "modLoader": 4},
            {"gameVersion": "1.21.1", "fileId": 12, "filename": "c.jar", "releaseType": 2, "modLoader": 4},
            {"gameVersion": "1.20.1", "fileId": 13, "filename": "d.jar", "releaseType": 1, "modLoader": 4}]});
        assert_eq!(latest_index(&m, Some("1.21.1"), 4).unwrap().0, "11");
        assert_eq!(latest_index(&m, Some("1.21.1"), 0).unwrap().0, "11"); // any loader: newest release
        assert_eq!(latest_index(&m, Some("1.21.1"), 1).unwrap().0, "10");
        assert!(latest_index(&m, Some("1.19"), 0).is_none());
    }

    #[test]
    fn sort_mapping_has_a_default() { assert_eq!(sort_field("downloads"), "6"); assert_eq!(sort_field("nonsense"), "1"); }
}
