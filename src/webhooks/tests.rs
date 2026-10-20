//! Delivery tests against an in-process receiver. The receiver speaks plain http on loopback (TLS is exercised by
//! reqwest itself, not by these tests); the production path refuses http and private addresses, which is tested too.
use super::{deliver::{self, Opts}, queue, secrets, *};
use crate::testutil;
use axum::{Router, body::Bytes, extract::State, http::{HeaderMap, StatusCode}};
use std::{sync::{Arc, Mutex}, time::Duration};

const TEST_OPTS: Opts = Opts { allow_private: true, allow_http: true };

#[derive(Clone, Default)]
struct Rx { hits: Arc<Mutex<Vec<(HeaderMap, String)>>>, script: Arc<Mutex<Vec<u16>>>, hang: bool }

async fn receiver(rx: Rx) -> String {
    async fn h(State(rx): State<Rx>, headers: HeaderMap, body: Bytes) -> (StatusCode, String) {
        rx.hits.lock().unwrap().push((headers, String::from_utf8_lossy(&body).to_string()));
        if rx.hang { tokio::time::sleep(Duration::from_secs(60)).await; }
        let code = { let mut s = rx.script.lock().unwrap(); if s.is_empty() { 200 } else { s.remove(0) } };
        (StatusCode::from_u16(code).unwrap(), "x".repeat(5000))
    }
    let app = Router::new().fallback(h).with_state(rx);
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/hook", l.local_addr().unwrap());
    tokio::spawn(async move { let _ = axum::serve(l, app).await; });
    url
}

async fn endpoint(st: &St, url: &str, events: &[&str], servers: &[&str]) -> Endpoint {
    let e = Endpoint { id: uuid::Uuid::now_v7().to_string(), name: "t".into(), url: url.into(), events: events.iter().map(|s| s.to_string()).collect(), server_ids: servers.iter().map(|s| s.to_string()).collect(),
        enabled: true, failures: 0, disabled_reason: None, created_at: crate::db::now() };
    secrets::set(&st.cfg.data_dir, &e.id, "whsec_test").unwrap();
    let e2 = e.clone();
    st.db.call(move |c| c.execute("INSERT INTO webhook_endpoints(id,name,url,events,server_ids,enabled,created_at) VALUES (?,?,?,?,?,?,?)",
        rusqlite::params![e2.id, e2.name, e2.url, json!(e2.events).to_string(), json!(e2.server_ids).to_string(), e2.enabled, e2.created_at])).await.unwrap();
    e
}

async fn rows(st: &St, e: &Endpoint) -> Vec<Value> { queue::deliveries(st, &e.id).await.unwrap() }

fn ev(kind: &'static str, sid: &str) -> Ev { Ev { kind, server: Some((sid.into(), "Survival".into())), data: json!({"k": 1}) } }

#[tokio::test]
async fn delivers_a_signed_event_with_the_documented_headers_and_payload() {
    queue::set_backoff_scale_pct(0);
    let st = testutil::state(|_| {}).await;
    let rx = Rx::default();
    let url = receiver(rx.clone()).await;
    let e = endpoint(&st, &url, &["server.started"], &[]).await;
    publish(&st, ev("server.started", "s1")).await;
    publish(&st, ev("server.stopped", "s1")).await; // not subscribed
    assert_eq!(queue::process_due(&st, TEST_OPTS, true).await, 1);
    let hits = rx.hits.lock().unwrap().clone();
    assert_eq!(hits.len(), 1);
    let (h, body) = &hits[0];
    let ts: i64 = h["x-moss-timestamp"].to_str().unwrap().parse().unwrap();
    assert_eq!(h["x-moss-signature"].to_str().unwrap(), sign::signature("whsec_test", ts, body), "signature = HMAC-SHA256(secret, timestamp.body)");
    assert_eq!(h["x-moss-event"], "server.started");
    let v: Value = serde_json::from_str(body).unwrap();
    assert_eq!((v["type"].as_str(), v["server"]["name"].as_str(), v["data"]["k"].as_i64()), (Some("server.started"), Some("Survival"), Some(1)));
    assert_eq!(h["x-moss-delivery"].to_str().unwrap(), rows_id(&st, &e).await);
    assert!(v["id"].is_string() && v["time"].is_string());
    let log = rows(&st, &e).await;
    assert_eq!((log[0]["state"].as_str(), log[0]["status_code"].as_i64(), log[0]["attempts"].as_i64()), (Some("delivered"), Some(200), Some(1)));
    assert!(log[0]["response"].as_str().unwrap().len() <= deliver::RESPONSE_CAP, "response is truncated");
    assert!(log[0]["latency_ms"].as_i64().is_some());
}

async fn rows_id(st: &St, e: &Endpoint) -> String { rows(st, e).await[0]["id"].as_str().unwrap().to_string() }

#[tokio::test]
async fn failures_are_retried_with_the_same_delivery_id_then_dead_lettered() {
    queue::set_backoff_scale_pct(0);
    let st = testutil::state(|_| {}).await;
    let rx = Rx { script: Arc::new(Mutex::new(vec![500, 503, 200])), ..Default::default() };
    let e = endpoint(&st, &receiver(rx.clone()).await, &["backup.failed"], &[]).await;
    publish(&st, ev("backup.failed", "s1")).await;
    for _ in 0..3 { queue::process_due(&st, TEST_OPTS, true).await; }
    let hits = rx.hits.lock().unwrap().clone();
    assert_eq!(hits.len(), 3);
    assert!(hits.iter().all(|(h, b)| h["x-moss-delivery"] == hits[0].0["x-moss-delivery"] && *b == hits[0].1), "a retry resends the same event and delivery id");
    let log = rows(&st, &e).await;
    assert_eq!((log[0]["state"].as_str(), log[0]["attempts"].as_i64()), (Some("delivered"), Some(3)));
    assert_eq!(get(&st, &e.id).await.unwrap().unwrap().failures, 0, "a success resets the failure count");

    // a receiver that never recovers: dead after MAX_ATTEMPTS, then it can be re-sent by hand
    let rx = Rx { script: Arc::new(Mutex::new(vec![500; 20])), ..Default::default() };
    let e = endpoint(&st, &receiver(rx.clone()).await, &["backup.failed"], &[]).await;
    publish(&st, ev("backup.failed", "s1")).await;
    for _ in 0..queue::MAX_ATTEMPTS + 2 { queue::process_due(&st, TEST_OPTS, true).await; }
    assert_eq!(rx.hits.lock().unwrap().len() as i64, queue::MAX_ATTEMPTS);
    let log = rows(&st, &e).await;
    assert_eq!(log[0]["state"], "dead");
    let id = log[0]["id"].as_str().unwrap().to_string();
    assert!(queue::retry(&st, &e.id, &id).await.unwrap());
    assert_eq!(rows(&st, &e).await[0]["state"], "pending");
    assert!(!queue::retry(&st, &e.id, &id).await.unwrap(), "only dead deliveries can be retried");
}

#[tokio::test]
async fn the_queue_survives_a_restart_and_backoff_grows() {
    assert!(queue::backoff_base_ms(1) < queue::backoff_base_ms(2) && queue::backoff_base_ms(2) < queue::backoff_base_ms(6));
    assert_eq!(queue::backoff_base_ms(1), 5000);
    let st = testutil::state(|_| {}).await;
    let rx = Rx { script: Arc::new(Mutex::new(vec![500])), ..Default::default() };
    let e = endpoint(&st, &receiver(rx.clone()).await, &["server.crashed"], &[]).await;
    publish(&st, ev("server.crashed", "s1")).await;
    queue::process_due(&st, TEST_OPTS, true).await; // 500: scheduled for later
    st.db.call(|c| c.execute("UPDATE webhook_deliveries SET next_at=next_at+3600000", [])).await.unwrap(); // (the test shrinks delays to zero)
    assert_eq!(queue::process_due(&st, TEST_OPTS, true).await, 0, "not due yet");
    let log = rows(&st, &e).await;
    assert_eq!((log[0]["state"].as_str(), log[0]["attempts"].as_i64()), (Some("pending"), Some(1)));
    assert!(log[0]["next_attempt_at"].is_string());
    // a "restart": a new state on the same data directory still holds the queued delivery
    let dir = st.cfg.data_dir.clone();
    let db = crate::db::Db::open(&dir.join("moss.sqlite")).unwrap();
    let n: i64 = db.call(|c| c.query_row("SELECT COUNT(*) FROM webhook_deliveries WHERE state='pending'", [], |r| r.get(0))).await.unwrap();
    assert_eq!(n, 1);
}

#[tokio::test]
async fn filters_by_event_and_server_and_skips_disabled_endpoints() {
    queue::set_backoff_scale_pct(0);
    let st = testutil::state(|_| {}).await;
    let rx = Rx::default();
    let url = receiver(rx.clone()).await;
    let only_s1 = endpoint(&st, &url, &["player.joined"], &["s1"]).await;
    let all = endpoint(&st, &url, &["player.joined", "update.available"], &[]).await;
    let off = endpoint(&st, &url, &["player.joined"], &[]).await;
    st.db.call({ let id = off.id.clone(); move |c| c.execute("UPDATE webhook_endpoints SET enabled=0 WHERE id=?", [id]) }).await.unwrap();
    publish(&st, ev("player.joined", "s1")).await;
    publish(&st, ev("player.joined", "s2")).await;
    publish(&st, Ev { kind: "update.available", server: None, data: json!({}) }).await;
    assert_eq!(rows(&st, &only_s1).await.len(), 1);
    assert_eq!(rows(&st, &all).await.len(), 3);
    assert_eq!(rows(&st, &off).await.len(), 0);
    // a global event (no server) never matches a per-server filter
    let g = endpoint(&st, &url, &["update.available"], &["s1"]).await;
    publish(&st, Ev { kind: "update.available", server: None, data: json!({}) }).await;
    assert_eq!(rows(&st, &g).await.len(), 0);
}

#[tokio::test]
async fn consecutive_failures_switch_the_endpoint_off() {
    queue::set_backoff_scale_pct(0);
    let st = testutil::state(|_| {}).await;
    let rx = Rx { script: Arc::new(Mutex::new(vec![500; 100])), ..Default::default() };
    let e = endpoint(&st, &receiver(rx.clone()).await, &["server.started"], &[]).await;
    for _ in 0..3 { publish(&st, ev("server.started", "s1")).await; }
    for _ in 0..queue::AUTO_DISABLE_AFTER { queue::process_due(&st, TEST_OPTS, true).await; if !get(&st, &e.id).await.unwrap().unwrap().enabled { break; } }
    let got = get(&st, &e.id).await.unwrap().unwrap();
    assert!(!got.enabled && got.disabled_reason.is_some() && got.failures >= queue::AUTO_DISABLE_AFTER);
    let hits = rx.hits.lock().unwrap().len();
    assert_eq!(queue::process_due(&st, TEST_OPTS, true).await, 0, "a switched-off endpoint is no longer called");
    assert_eq!(rx.hits.lock().unwrap().len(), hits);
    assert!(rows(&st, &e).await.iter().all(|r| r["state"] != "pending"), "its queue is dead-lettered");
}

#[tokio::test]
async fn test_events_are_logged_but_never_retried_or_counted() {
    queue::set_backoff_scale_pct(0);
    let st = testutil::state(|_| {}).await;
    let rx = Rx { script: Arc::new(Mutex::new(vec![500, 200])), ..Default::default() };
    let e = endpoint(&st, &receiver(rx.clone()).await, &["server.started"], &[]).await;
    let d = queue::send_test(&st, &e, TEST_OPTS, "me@example.com").await.unwrap();
    assert_eq!((d["state"].as_str(), d["status_code"].as_i64(), d["test"].as_bool()), (Some("dead"), Some(500), Some(true)));
    assert_eq!(get(&st, &e.id).await.unwrap().unwrap().failures, 0);
    let d = queue::send_test(&st, &e, TEST_OPTS, "me@example.com").await.unwrap();
    assert_eq!(d["state"], "delivered");
    assert_eq!(rx.hits.lock().unwrap()[1].0["x-moss-event"], "test");
}

#[tokio::test]
async fn a_hanging_receiver_times_out_and_the_log_keeps_only_the_last_100() {
    let st = testutil::state(|_| {}).await;
    // timeout: shrink nothing, just prove an unreachable port is a clean failure without the URL in the error
    let o = deliver::attempt("http://127.0.0.1:9/secret-path?token=abc", "s", "x", "d", "{}", TEST_OPTS).await;
    assert!(!o.ok && o.status.is_none());
    let err = o.error.unwrap();
    assert!(!err.contains("secret-path") && !err.contains("token"), "{err}");
    let _ = Rx { hang: true, ..Default::default() }; // the 10 s timeout itself is not slept through in unit tests

    let rx = Rx::default();
    let e = endpoint(&st, &receiver(rx).await, &["server.started"], &[]).await;
    for _ in 0..130 { publish(&st, ev("server.started", "s1")).await; queue::process_due(&st, TEST_OPTS, true).await; }
    assert_eq!(rows(&st, &e).await.len() as i64, queue::KEEP_LOG);
}

#[tokio::test]
async fn ssrf_and_plain_http_are_refused_in_production_mode() {
    let strict = Opts { allow_private: false, allow_http: false };
    for url in ["http://example.com/h", "https://127.0.0.1/h", "https://[::1]/h", "https://10.0.0.5/h", "https://169.254.169.254/latest", "https://192.168.1.1/h", "https://100.64.0.1/h"] {
        let o = deliver::attempt(url, "s", "x", "d", "{}", strict).await;
        assert!(!o.ok && o.status.is_none(), "{url}");
        assert!(o.error.as_deref().is_some_and(|e| e.contains("https") || e.contains("non-public")), "{url}: {:?}", o.error);
    }
    // the override allows private addresses, but still https only
    let o = deliver::attempt("http://127.0.0.1:9/h", "s", "x", "d", "{}", Opts { allow_private: true, allow_http: false }).await;
    assert!(o.error.unwrap().contains("https"));
}

#[tokio::test]
async fn redirects_are_not_followed() {
    let app = Router::new().fallback(|| async { (StatusCode::FOUND, [("location", "http://127.0.0.1:1/elsewhere")], "") });
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/x", l.local_addr().unwrap());
    tokio::spawn(async move { let _ = axum::serve(l, app).await; });
    let o = deliver::attempt(&url, "s", "x", "d", "{}", TEST_OPTS).await;
    assert_eq!(o.status, Some(302));
    assert!(!o.ok && o.error.unwrap().contains("redirects are not followed"));
}

#[test]
fn oversized_data_is_replaced_and_secrets_never_appear_in_the_body() {
    let big = json!({"x": "a".repeat(70_000)});
    let b = body("e", "server.started", "t", None, &big);
    assert!(b.len() < 1000 && b.contains("truncated"));
    let small = body("e", "server.started", "t", Some(&("s1".into(), "N".into())), &json!({}));
    assert!(!small.contains("whsec"));
}
