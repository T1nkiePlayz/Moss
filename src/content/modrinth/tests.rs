//! Provider outages against stub servers: Modrinth and CurseForge timing out, answering 5xx or 429 (with Retry-After)
//! or sending malformed JSON must give short, actionable messages, the right HTTP mapping, and partial results.
use super::*;
use crate::testutil::{Reply, state, stub};
use std::{sync::Arc, time::{Duration, Instant}};

async fn mr_state(base: &str) -> St { let b = format!("{base}/v2"); state(move |c| c.modrinth_api_url = b).await }

fn err_text(e: &anyhow::Error) -> String { format!("{e:#}") }

fn rate_limited(e: &anyhow::Error) -> bool { e.downcast_ref::<cf::CfError>().is_some_and(|c| c.rate_limited) }

#[tokio::test]
async fn modrinth_server_errors_are_reported_plainly() {
    let s = stub(|_, _| Reply::status(503)).await;
    let st = mr_state(&s.base).await;
    let e = mr(&st, "/project/abc", &[]).await.unwrap_err();
    assert!(err_text(&e).contains("having problems") && err_text(&e).contains("503"), "{}", err_text(&e));
    assert!(!rate_limited(&e));
    assert_eq!(upstream(e).status, StatusCode::BAD_GATEWAY);
}

#[tokio::test]
async fn modrinth_rate_limits_surface_retry_after_and_map_to_429() {
    let s = stub(|_, _| Reply::Raw(429, vec![("retry-after", "17".into())], String::new())).await;
    let st = mr_state(&s.base).await;
    let e = mr(&st, "/project/abc", &[]).await.unwrap_err();
    assert!(rate_limited(&e) && err_text(&e).contains("17s"), "{}", err_text(&e));
    assert_eq!(upstream(e).status, StatusCode::TOO_MANY_REQUESTS);
    // no (or nonsense) Retry-After still gives a message
    let s = stub(|_, _| Reply::Raw(429, vec![("retry-after", "soon".into())], String::new())).await;
    let e = mr(&mr_state(&s.base).await, "/x", &[]).await.unwrap_err();
    assert!(rate_limited(&e) && err_text(&e).contains("a minute"));
}

#[tokio::test]
async fn modrinth_malformed_json_is_not_a_crash() {
    let s = stub(|_, _| Reply::Raw(200, vec![("content-type", "application/json".into())], "{\"hits\": [".into())).await;
    let e = mr(&mr_state(&s.base).await, "/search", &[]).await.unwrap_err();
    assert!(err_text(&e).contains("could not read"), "{}", err_text(&e));
    let s = stub(|_, _| Reply::Raw(200, vec![], "<html>maintenance</html>".into())).await;
    assert!(mr(&mr_state(&s.base).await, "/search", &[]).await.is_err());
}

#[tokio::test]
async fn modrinth_timeouts_and_unreachable_hosts_fail_fast_with_a_message() {
    let s = stub(|_, _| Reply::Hang).await;
    let st = mr_state(&s.base).await; // set up outside the timed part: opening the database can be slow under load
    let t = Instant::now();
    let e = mr(&st, "/project/abc", &[]).await.unwrap_err();
    assert!(err_text(&e).contains("did not answer in time") && t.elapsed() < Duration::from_secs(5), "{}", err_text(&e));
    // nothing listens on this port any more
    let dead = { let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap(); format!("http://{}", l.local_addr().unwrap()) };
    let e = mr(&mr_state(&dead).await, "/project/abc", &[]).await.unwrap_err();
    assert!(err_text(&e).contains("Could not reach Modrinth"), "{}", err_text(&e));
    assert!(!err_text(&e).contains("127.0.0.1"), "internal addresses stay out of messages");
}

#[tokio::test]
async fn modrinth_404_and_errors_are_not_cached() {
    let n = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let n2 = n.clone();
    let s = stub(move |_, _| if n2.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 { Reply::status(500) } else { Reply::json(200, json!({"id": "abc"})) }).await;
    let st = mr_state(&s.base).await;
    assert!(mr(&st, "/project/abc", &[]).await.is_err());
    assert_eq!(mr(&st, "/project/abc", &[]).await.unwrap()["id"], "abc", "a failure must not poison the cache");
}

fn compat_mods() -> Compat { Compat { kind: "mods", project_type: "mod", loaders: vec!["fabric"], game_version: Some("1.21.1".into()), dir: "mods".into(), ext: "jar", cf_class: None, cf_loader: 0 } }

fn item(id: &str, pid: &str) -> Value { json!({"id": id, "provider": "modrinth", "project_id": pid, "version_id": "old", "title": id, "version_number": "1.0", "filename": format!("{id}.jar"), "pinned": false}) }

#[tokio::test]
async fn update_checks_return_partial_results_when_part_of_modrinth_fails() {
    let s = stub(|_, p| if p.starts_with("/v2/project/good/version") { Reply::json(200, json!([{"id": "new", "version_number": "2.0", "version_type": "release"}])) } else { Reply::status(500) }).await;
    let st = mr_state(&s.base).await;
    let u = apply::collect_updates(&st, None, &compat_mods(), &[item("a", "good"), item("b", "bad1"), item("c", "bad2")]).await;
    assert_eq!(u.list.len(), 1);
    assert_eq!(u.list[0]["title"], "a");
    assert!(u.problems.iter().any(|p| p.contains("having problems")) && u.problems.iter().any(|p| p.contains("2 Modrinth item(s) could not be checked")), "{:?}", u.problems);
}

#[tokio::test]
async fn update_checks_with_modrinth_fully_down_report_why_instead_of_saying_up_to_date() {
    let s = stub(|_, _| Reply::status(502)).await;
    let st = mr_state(&s.base).await;
    let u = apply::collect_updates(&st, None, &compat_mods(), &[item("a", "x")]).await;
    assert!(u.list.is_empty() && !u.problems.is_empty());
}

#[tokio::test]
async fn update_all_planning_degrades_to_warnings() {
    let s = stub(|_, _| Reply::status(500)).await;
    let st = mr_state(&s.base).await;
    let id = "srv1";
    let i = id.to_string();
    st.db.call(move |c| {
        c.execute("INSERT INTO servers(id,name,runtime,game_version,port,memory_mb,created_at) VALUES (?,?,?,?,?,?,?)", rusqlite::params![i, "t", "fabric", "1.21.1", 25565, 1024, "now"])?;
        c.execute("INSERT INTO content_items(id,server_id,provider,project_id,version_id,title,version_number,filename,kind,installed_at,dir) VALUES ('i1',?,'modrinth','p1','old','P1','1','p1.jar','mods','now','mods')", [i])
    }).await.unwrap();
    let srv = crate::server::get(&st, id).await.unwrap();
    let (steps, warnings) = update_all::build_all(&st, &srv, &compat_mods(), None).await.unwrap();
    assert!(steps.is_empty() && !warnings.is_empty(), "{warnings:?}");
}

// ---------- CurseForge ----------

async fn cf_state(base: &str) -> St { let b = base.to_string(); state(move |c| { c.curseforge_api_key = "test-key".into(); c.curseforge_api_url = b; }).await }

#[tokio::test]
async fn curseforge_429_pauses_calls_and_reports_rate_limiting() {
    let s = stub(|_, _| Reply::Raw(429, vec![("retry-after", "5".into())], String::new())).await;
    let st = cf_state(&s.base).await;
    let e = cf::get(&st, None, "/v1/mods/1", &[]).await.unwrap_err();
    assert!(rate_limited(&e), "{e:#}");
    let hits = s.hits.lock().unwrap().len();
    let e2 = cf::get(&st, None, "/v1/mods/1", &[]).await.unwrap_err();
    assert!(rate_limited(&e2) && err_text(&e2).contains("slow down"), "{e2:#}");
    assert_eq!(s.hits.lock().unwrap().len(), hits, "the pause must not hit the provider again");
    assert_eq!(upstream(e2).status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn curseforge_5xx_is_retried_once_then_reported() {
    let s = stub(|_, _| Reply::status(500)).await;
    let st = cf_state(&s.base).await;
    let e = cf::get(&st, None, "/v1/mods/1", &[]).await.unwrap_err();
    assert!(err_text(&e).contains("500") && !rate_limited(&e));
    assert_eq!(s.hits.lock().unwrap().len(), 2, "one retry for transient failures");
    assert_eq!(upstream(e).status, StatusCode::BAD_GATEWAY);
}

#[tokio::test]
async fn curseforge_malformed_json_and_bad_keys_are_clear() {
    let s = stub(|_, p| if p.contains("/1") { Reply::Raw(200, vec![], "{oops".into()) } else { Reply::status(403) }).await;
    let st = cf_state(&s.base).await;
    assert!(err_text(&cf::get(&st, None, "/v1/mods/1", &[]).await.unwrap_err()).contains("unreadable"));
    let e = cf::get(&st, None, "/v1/mods/2", &[]).await.unwrap_err();
    assert!(err_text(&e).contains("API key is invalid"));
}

#[tokio::test]
async fn curseforge_down_does_not_hide_modrinth_updates() {
    let ms = stub(|_, p| if p.contains("/version") { Reply::json(200, json!([{"id": "new", "version_number": "2.0", "version_type": "release"}])) } else { Reply::status(500) }).await;
    let cs = stub(|_, _| Reply::status(503)).await;
    let (b, cb) = (format!("{}/v2", ms.base), cs.base.clone());
    let st = state(move |c| { c.modrinth_api_url = b; c.curseforge_api_key = "k".into(); c.curseforge_api_url = cb; }).await;
    let mut cf_item = item("cfmod", "99");
    cf_item["provider"] = json!("curseforge");
    let u = apply::collect_updates(&st, None, &compat_mods(), &[item("a", "good"), cf_item]).await;
    assert_eq!(u.list.len(), 1, "{:?}", u.problems);
    assert!(u.problems.iter().any(|p| p.contains("CurseForge items were not checked")), "{:?}", u.problems);
}
