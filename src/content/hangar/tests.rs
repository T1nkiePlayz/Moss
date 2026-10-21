//! Hangar: parsing real-shaped payloads, version selection, dependency mapping, external-only handling and outages
//! against stub servers.
use super::*;
use crate::testutil::{Reply, state, stub};
use std::time::{Duration, Instant};

async fn hg_state(base: &str) -> St { let b = format!("{base}/api/v1"); state(move |c| c.hangar_api_url = b).await }

fn err_text(e: &anyhow::Error) -> String { format!("{e:#}") }

fn compat_paper() -> Compat { Compat { kind: "plugins", project_type: "plugin", loaders: vec!["paper", "spigot", "bukkit"], game_version: Some("1.21.4".into()), dir: "plugins".into(), ext: "jar", cf_class: None, cf_loader: 0 } }
fn compat_velocity() -> Compat { Compat { kind: "proxy_plugins", loaders: vec!["velocity"], game_version: None, ..compat_paper() } }

fn version(name: &str, channel: &str, hosted: bool, deps: Value) -> Value {
    let dl = if hosted {
        json!({"fileInfo": {"name": format!("Plug-{name}.jar"), "sizeBytes": 1234, "sha256Hash": "ab".repeat(32)}, "externalUrl": null, "downloadUrl": format!("https://hangarcdn.papermc.io/plugins/o/Plug/versions/{name}/PAPER/Plug-{name}.jar")})
    } else {
        json!({"fileInfo": null, "externalUrl": "https://example.com/plug", "downloadUrl": null})
    };
    json!({"name": name, "channel": {"name": channel}, "createdAt": "2025-01-01T00:00:00Z", "stats": {"totalDownloads": 5},
        "downloads": {"PAPER": dl}, "platformDependencies": {"PAPER": ["1.21.4"]}, "pluginDependencies": {"PAPER": deps}})
}

#[test]
fn platform_mapping_covers_paper_family_and_velocity_only() {
    assert_eq!(hangar_platform(&compat_paper()), Some("PAPER"));
    assert_eq!(hangar_platform(&Compat { loaders: vec!["folia"], ..compat_paper() }), Some("PAPER"));
    assert_eq!(hangar_platform(&Compat { loaders: vec!["purpur", "paper"], ..compat_paper() }), Some("PAPER"));
    assert_eq!(hangar_platform(&compat_velocity()), Some("VELOCITY"));
    assert_eq!(hangar_platform(&Compat { loaders: vec!["fabric"], ..compat_paper() }), None);
}

#[test]
fn search_hits_are_parsed_from_hangar_shape() {
    let p = json!({"id": 7, "name": "ViaVersion", "namespace": {"owner": "ViaVersion", "slug": "ViaVersion"}, "stats": {"downloads": 900, "stars": 12}, "category": "admin_tools",
        "description": "Allows newer clients", "lastUpdated": "2025-02-02T00:00:00Z", "avatarUrl": "https://hangarcdn.papermc.io/avatars/project/7.webp", "supportedPlatforms": {"PAPER": ["1.21.4"]}});
    let h = hg_hit(&p);
    assert_eq!(h["project_id"], "ViaVersion");
    assert_eq!((h["title"].as_str(), h["author"].as_str(), h["downloads"].as_u64(), h["follows"].as_u64()), (Some("ViaVersion"), Some("ViaVersion"), Some(900), Some(12)));
    assert_eq!(h["categories"], json!(["admin tools"]));
    assert_eq!(h["icon_url"], "https://hangarcdn.papermc.io/avatars/project/7.webp");
    let mut bad = p.clone();
    bad["avatarUrl"] = json!("javascript:alert(1)");
    assert!(hg_hit(&bad)["icon_url"].is_null());
}

#[test]
fn version_selection_prefers_hosted_releases() {
    let vs = vec![version("3.0-SNAP", "Snapshot", true, json!([])), version("2.1", "Release", false, json!([])), version("2.0", "Release", true, json!([])), version("1.0", "Alpha", true, json!([]))];
    assert_eq!(hg_pick_latest(&vs, "PAPER").unwrap()["name"], "2.0", "the external-only 2.1 is skipped, releases beat snapshots");
    assert_eq!(hg_pick_latest(&vs[..1], "PAPER").unwrap()["name"], "3.0-SNAP");
    assert!(hg_pick_latest(&vs[1..2], "PAPER").is_none());
    assert!(hg_pick_latest(&vs, "VELOCITY").is_none());
    assert_eq!(channel(&vs[0]), "beta");
    assert_eq!(channel(&vs[3]), "alpha");
}

#[test]
fn external_only_versions_are_not_installable_and_link_out() {
    let v = version("2.1", "Release", false, json!([]));
    assert!(hg_file(&v, "PAPER").is_none());
    assert_eq!(hg_external(&v, "PAPER").as_deref(), Some("https://example.com/plug"));
    let m = hg_map_version(&v, "PAPER");
    assert_eq!((m["downloadable"].clone(), m["external_url"].clone(), m["files"].clone()), (json!(false), json!("https://example.com/plug"), json!([])));
    assert!(m["warning"].as_str().unwrap().contains("https://example.com/plug"));
    let mut js = v.clone();
    js["downloads"]["PAPER"]["externalUrl"] = json!("javascript:alert(1)");
    assert!(hg_external(&js, "PAPER").is_none());
    // a plain http download is never accepted either
    let mut http = version("1", "Release", true, json!([]));
    http["downloads"]["PAPER"]["downloadUrl"] = json!("http://hangarcdn.papermc.io/x.jar");
    assert!(hg_file(&http, "PAPER").is_none());
}

#[test]
fn hosted_versions_carry_filename_checksum_and_dependencies() {
    let deps = json!([{"name": "Vault", "projectId": 55, "required": true, "externalUrl": null, "platform": "PAPER"}, {"name": "PAPI", "projectId": null, "required": false, "externalUrl": "https://spigotmc.org/x", "platform": "PAPER"}]);
    let v = version("2.0", "Release", true, deps);
    let f = hg_file(&v, "PAPER").unwrap();
    assert_eq!((f["filename"].as_str(), f["hashes"]["sha256"].as_str().map(str::len), f["size"].as_u64()), (Some("Plug-2.0.jar"), Some(64), Some(1234)));
    let m = hg_map_version(&v, "PAPER");
    assert_eq!(m["id"], "2.0");
    assert_eq!(m["game_versions"], json!(["1.21.4"]));
    assert_eq!(m["dependencies"], json!([{"project_id": "55", "kind": "required"}]), "only dependencies that live on Hangar are listed");
}

async fn add_server(st: &St, installed: bool) -> Srv {
    let id = "srv1";
    let i = id.to_string();
    st.db.call(move |c| {
        c.execute("INSERT INTO servers(id,name,runtime,game_version,port,memory_mb,created_at) VALUES (?,?,?,?,?,?,?)", rusqlite::params![i, "t", "paper", "1.21.4", 25565, 1024, "now"])?;
        if installed { c.execute("INSERT INTO content_items(id,server_id,provider,project_id,version_id,title,version_number,filename,kind,installed_at,dir) VALUES ('i1',?,'hangar','Plug','1.0','Plug','1.0','Plug-1.0.jar','plugins','now','plugins')", [i])?; }
        Ok(())
    }).await.unwrap();
    crate::server::get(st, id).await.unwrap()
}

#[tokio::test]
async fn search_sends_platform_version_and_text() {
    let s = stub(|_, _| Reply::json(200, json!({"pagination": {"count": 1, "limit": 20, "offset": 0}, "result": [{"name": "ViaVersion", "namespace": {"owner": "o", "slug": "ViaVersion"}, "stats": {}, "category": "misc"}]}))).await;
    let st = hg_state(&s.base).await;
    let srv = add_server(&st, false).await;
    let q: HashMap<String, String> = [("q", "via"), ("sort", "downloads")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    let Json(r) = search_hangar(&st, &srv, &compat_paper(), &q).await.unwrap();
    assert_eq!((r["hits"][0]["project_id"].as_str(), r["total"].as_u64()), (Some("ViaVersion"), Some(1)));
    let hit = s.hits.lock().unwrap()[0].clone();
    assert!(hit.contains("/api/v1/projects?") && hit.contains("platform=PAPER") && hit.contains("version=1.21.4") && hit.contains("q=via") && hit.contains("sort=-downloads"), "{hit}");
}

fn hangar_stub(plug_versions: Value) -> impl Fn(&str, &str) -> Reply + Send + Sync + 'static {
    move |_, p| {
        if p.starts_with("/api/v1/projects/Plug/versions") { Reply::json(200, json!({"result": plug_versions})) }
        else if p.starts_with("/api/v1/projects/Plug") { Reply::json(200, json!({"name": "Plug", "namespace": {"owner": "o", "slug": "Plug"}})) }
        else if p.starts_with("/api/v1/projects/Vault/versions") { Reply::json(200, json!({"result": [version("9", "Release", true, json!([]))]})) }
        else if p.starts_with("/api/v1/projects/55") { Reply::json(200, json!({"name": "Vault", "namespace": {"owner": "v", "slug": "Vault"}})) }
        else { Reply::status(404) }
    }
}

#[tokio::test]
async fn plan_resolves_required_dependencies_and_reports_external_ones() {
    let deps = json!([{"name": "Vault", "projectId": 55, "required": true}, {"name": "Legacy", "projectId": null, "required": true, "externalUrl": "https://example.com/legacy"}, {"name": "Opt", "projectId": 77, "required": false}]);
    let s = stub(hangar_stub(json!([version("2.0", "Release", true, deps)]))).await;
    let st = hg_state(&s.base).await;
    let srv = add_server(&st, false).await;
    let (steps, warnings) = build_plan_hg(&st, &srv, &compat_paper(), "Plug", None).await.unwrap();
    let got: Vec<(&str, &str, &str)> = steps.iter().map(|x| (x.project_id.as_str(), x.reason.as_str(), x.version["id"].as_str().unwrap())).collect();
    assert_eq!(got, vec![("Vault", "required dependency", "9"), ("Plug", "requested", "2.0")]);
    assert!(steps.iter().all(|x| x.prov == Prov::Hangar && x.file["hashes"]["sha256"].as_str().is_some()));
    assert!(warnings.iter().any(|w| w.contains("Legacy") && w.contains("Install it manually") && w.contains("https://example.com/legacy")), "{warnings:?}");
    let hits = s.hits.lock().unwrap().join("\n");
    assert!(hits.contains("platform=PAPER") && hits.contains("platformVersion=1.21.4"), "{hits}");
    assert!(!hits.contains("projects/77"), "optional dependencies are not resolved");
}

#[tokio::test]
async fn plan_replaces_an_installed_version_and_pins_to_a_requested_one() {
    let s = stub(hangar_stub(json!([version("2.0", "Release", true, json!([])), version("1.5", "Release", true, json!([]))]))).await;
    let st = hg_state(&s.base).await;
    let srv = add_server(&st, true).await;
    let (steps, _) = build_plan_hg(&st, &srv, &compat_paper(), "Plug", Some("1.5")).await.unwrap();
    assert_eq!((steps[0].action, steps[0].version["id"].as_str(), steps[0].replaces.as_ref().map(|r| r.1.as_str())), ("replace", Some("1.5"), Some("Plug-1.0.jar")));
    let e = build_plan_hg(&st, &srv, &compat_paper(), "Plug", Some("0.1")).await.err().unwrap();
    assert!(err_text(&e).contains("not compatible"), "{}", err_text(&e));
}

#[tokio::test]
async fn plan_refuses_external_only_versions_with_a_link() {
    let s = stub(hangar_stub(json!([version("2.0", "Release", false, json!([]))]))).await;
    let st = hg_state(&s.base).await;
    let srv = add_server(&st, false).await;
    let e = build_plan_hg(&st, &srv, &compat_paper(), "Plug", None).await.err().unwrap();
    assert!(err_text(&e).contains("not hosted on Hangar") && err_text(&e).contains("https://example.com/plug"), "{}", err_text(&e));
    let e = build_plan_hg(&st, &srv, &compat_paper(), "Plug", Some("2.0")).await.err().unwrap();
    assert!(err_text(&e).contains("not hosted on Hangar"), "{}", err_text(&e));
}

#[tokio::test]
async fn update_checks_find_newer_hosted_versions_and_survive_an_outage() {
    let s = stub(hangar_stub(json!([version("2.0", "Release", true, json!([])), version("1.0", "Release", true, json!([]))]))).await;
    let st = hg_state(&s.base).await;
    let item = json!({"id": "i1", "provider": "hangar", "project_id": "Plug", "version_id": "1.0", "title": "Plug", "version_number": "1.0", "filename": "Plug-1.0.jar", "pinned": false});
    let u = apply::collect_updates(&st, None, &compat_paper(), std::slice::from_ref(&item)).await;
    assert_eq!((u.list.len(), u.list[0]["version_id"].as_str(), u.list[0]["provider"].as_str()), (1, Some("2.0"), Some("hangar")), "{:?}", u.problems);
    let mut pinned = item.clone();
    pinned["pinned"] = json!(true);
    assert!(apply::collect_updates(&st, None, &compat_paper(), &[pinned]).await.list.is_empty());
    // current version already newest: no update
    let mut cur = item.clone();
    cur["version_id"] = json!("2.0");
    assert!(apply::collect_updates(&st, None, &compat_paper(), &[cur]).await.list.is_empty());
    // outage: reported, not silently "up to date"
    let down = stub(|_, _| Reply::status(503)).await;
    let u = apply::collect_updates(&hg_state(&down.base).await, None, &compat_paper(), &[item]).await;
    assert!(u.list.is_empty() && u.problems.iter().any(|p| p.contains("1 Hangar item(s) could not be checked")), "{:?}", u.problems);
}

// ---------- outages ----------

#[tokio::test]
async fn hangar_server_errors_and_rate_limits_are_reported_plainly() {
    let s = stub(|_, _| Reply::status(503)).await;
    let e = hg(&hg_state(&s.base).await, "/projects/x", &[]).await.err().unwrap();
    assert!(err_text(&e).contains("having problems") && err_text(&e).contains("503"), "{}", err_text(&e));
    assert_eq!(upstream(e).status, StatusCode::BAD_GATEWAY);
    let s = stub(|_, _| Reply::Raw(429, vec![("retry-after", "17".into())], String::new())).await;
    let e = hg(&hg_state(&s.base).await, "/projects/x", &[]).await.err().unwrap();
    assert!(err_text(&e).contains("17s"), "{}", err_text(&e));
    assert_eq!(upstream(e).status, StatusCode::TOO_MANY_REQUESTS);
    let s = stub(|_, _| Reply::status(404)).await;
    assert!(err_text(&hg(&hg_state(&s.base).await, "/projects/LuckPerms", &[]).await.unwrap_err()).contains("Not found on Hangar"));
}

#[tokio::test]
async fn hangar_malformed_json_timeouts_and_unreachable_hosts_fail_fast() {
    let s = stub(|_, _| Reply::Raw(200, vec![("content-type", "application/json".into())], "{\"result\": [".into())).await;
    assert!(err_text(&hg(&hg_state(&s.base).await, "/projects", &[]).await.unwrap_err()).contains("could not read"));
    let s = stub(|_, _| Reply::Hang).await;
    let st = hg_state(&s.base).await; // set up outside the timed part: opening the database can be slow under load
    let t = Instant::now();
    let e = hg(&st, "/projects", &[]).await.err().unwrap();
    assert!(err_text(&e).contains("did not answer in time") && t.elapsed() < Duration::from_secs(5), "{}", err_text(&e));
    let dead = { let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap(); format!("http://{}", l.local_addr().unwrap()) };
    let e = hg(&hg_state(&dead).await, "/projects", &[]).await.err().unwrap();
    assert!(err_text(&e).contains("Could not reach Hangar") && !err_text(&e).contains("127.0.0.1"), "{}", err_text(&e));
}

#[tokio::test]
async fn hangar_down_does_not_hide_modrinth_updates() {
    let ms = stub(|_, p| if p.contains("/version") { Reply::json(200, json!([{"id": "new", "version_number": "2.0", "version_type": "release"}])) } else { Reply::status(500) }).await;
    let hs = stub(|_, _| Reply::status(503)).await;
    let (b, hb) = (format!("{}/v2", ms.base), format!("{}/api/v1", hs.base));
    let st = state(move |c| { c.modrinth_api_url = b; c.hangar_api_url = hb; }).await;
    let mr_item = json!({"id": "a", "provider": "modrinth", "project_id": "good", "version_id": "old", "title": "a", "version_number": "1.0", "filename": "a.jar", "pinned": false});
    let hg_item = json!({"id": "h", "provider": "hangar", "project_id": "Plug", "version_id": "1.0", "title": "h", "version_number": "1.0", "filename": "h.jar", "pinned": false});
    let u = apply::collect_updates(&st, None, &compat_paper(), &[mr_item, hg_item]).await;
    assert_eq!(u.list.len(), 1, "{:?}", u.problems);
    assert!(u.problems.iter().any(|p| p.contains("Hangar")), "{:?}", u.problems);
}

#[tokio::test]
async fn hangar_failures_are_not_cached() {
    let n = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let n2 = n.clone();
    let s = stub(move |_, _| if n2.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 { Reply::status(500) } else { Reply::json(200, json!({"name": "ok"})) }).await;
    let st = hg_state(&s.base).await;
    assert!(hg(&st, "/projects/a", &[]).await.is_err());
    assert_eq!(hg(&st, "/projects/a", &[]).await.unwrap()["name"], "ok");
}

#[test]
fn only_hangars_own_hosts_are_downloadable() {
    let v = |url: &str| json!({"downloads": {"PAPER": {"fileInfo": {"name": "P.jar", "sizeBytes": 1, "sha256Hash": "ab".repeat(32)}, "downloadUrl": url}}});
    assert!(hg_file(&v("https://hangarcdn.papermc.io/plugins/a.jar"), "PAPER").is_some());
    assert!(hg_file(&v("https://hangar.papermc.io/api/v1/projects/a/versions/1/PAPER/download"), "PAPER").is_some());
    for bad in ["https://evil.example.com/a.jar", "https://hangarcdn.papermc.io.evil.io/a.jar", "https://evil.io/@hangarcdn.papermc.io/a.jar", "https://hangarcdn.papermc.io@evil.io/a.jar",
        "https://cdn.papermc.io/a.jar", "http://hangarcdn.papermc.io/a.jar", "https://hangarcdn.papermc.io:8443/a.jar"] {
        assert!(hg_file(&v(bad), "PAPER").is_none(), "{bad}");
    }
    assert_eq!(hg_map_version(&v("https://evil.example.com/a.jar"), "PAPER")["downloadable"], json!(false));
}
