//! Staged updates end to end against the real binary and the simulated game server: the copy is hidden from the main
//! list but listed under its origin, healthy and unhealthy copies are told apart, copies can be inspected, cancelled
//! and discarded, and a controller restart cleans up whatever was in flight.
//! (Update-all and upgrade need real provider downloads and Java, which this environment does not have; the run
//! pipeline they share is exercised here with the `none` kind, and their planning is covered by unit tests.)
mod suite;
use serde_json::{Value, json};
use std::time::Duration;
use suite::{Moss, free_port, until, wait_op};

struct T { c: reqwest::Client, base: String, origin: String, data: std::path::PathBuf }

impl T {
    async fn get(&self, path: &str) -> (u16, Value) {
        let r = self.c.get(format!("{}{path}", self.base)).send().await.unwrap();
        (r.status().as_u16(), r.json().await.unwrap_or(Value::Null))
    }
    async fn post(&self, path: &str, body: Value) -> (u16, Value) {
        let r = self.c.post(format!("{}{path}", self.base)).json(&body).send().await.unwrap();
        (r.status().as_u16(), r.json().await.unwrap_or(Value::Null))
    }
    async fn delete(&self, path: &str) -> (u16, Value) {
        let r = self.c.delete(format!("{}{path}", self.base)).send().await.unwrap();
        (r.status().as_u16(), r.json().await.unwrap_or(Value::Null))
    }
    fn behavior(&self, text: &str) { std::fs::write(self.data.join("servers").join(&self.origin).join("sim-behavior.txt"), text).unwrap(); }
    /// Starts a run and waits for its operation to end; returns (run json after the op, operation).
    async fn run_to_end(&self, body: Value) -> Value {
        let (code, r) = self.post(&format!("/servers/{}/staging", self.origin), body).await;
        assert_eq!(code, 202, "{r}");
        let op = wait_op(&self.c, &self.base, r["operation"]["id"].as_str().unwrap()).await;
        let id = r["run"]["id"].as_str().unwrap();
        let (_, run) = self.get(&format!("/servers/{}/staging/{id}", self.origin)).await;
        assert_eq!(op["state"], "completed", "{op}");
        run
    }
    async fn discard(&self, run: &str) { assert_eq!(self.delete(&format!("/servers/{}/staging/{run}", self.origin)).await.0, 200); }
}

fn port_open(p: i64) -> bool { std::net::TcpStream::connect_timeout(&format!("127.0.0.1:{p}").parse().unwrap(), Duration::from_millis(300)).is_ok() }

#[tokio::test(flavor = "multi_thread")]
async fn staged_runs_are_hidden_inspectable_and_cleaned_up() {
    let data = std::env::temp_dir().join(format!("moss-staging-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data);
    let (port, base_port) = (free_port(), free_port());
    let env = [("MOSS_STAGING_PORT_RANGE", format!("{}-{}", base_port + 1, base_port + 60))];
    let moss = Moss::start(&data, port, &env);
    let c = reqwest::Client::new();
    until("controller", 20, || async { c.get(format!("http://127.0.0.1:{port}/healthz")).send().await.ok().filter(|r| r.status().is_success()) }).await;
    let game_port = free_port();
    let created: Value = c.post(format!("{}/servers", moss.base)).json(&json!({"name": "origin", "runtime": "sim", "game_version": "sim-1.0", "memory_mb": 512, "port": game_port})).send().await.unwrap().json().await.unwrap();
    let origin = created["server"]["id"].as_str().unwrap().to_string();
    assert_eq!(wait_op(&c, &moss.base, created["operation"]["id"].as_str().unwrap()).await["state"], "completed");
    let t = T { c: c.clone(), base: moss.base.clone(), origin: origin.clone(), data: data.clone() };
    // the real server keeps running through every test below
    assert!(c.post(format!("{}/servers/{origin}/actions/start", t.base)).send().await.unwrap().status().is_success());
    until("origin running", 30, || async { t.get(&format!("/servers/{origin}")).await.1["state"].as_str().filter(|s| *s == "running").map(|_| ()) }).await;
    std::fs::write(data.join("servers").join(&origin).join("keepme.txt"), "x").unwrap();

    // estimate and validation
    let (code, est) = t.get(&format!("/servers/{origin}/staging/estimate")).await;
    assert_eq!(code, 200);
    assert_eq!(est["port_range"], json!([base_port + 1, base_port + 60]));
    assert!(est["copy_bytes"].as_u64().unwrap() > 0 && est["free_bytes"].as_u64().unwrap() > 0, "{est}");
    assert_eq!(t.post(&format!("/servers/{origin}/staging"), json!({"kind": "bogus"})).await.0, 400);
    let (code, e) = t.post(&format!("/servers/{origin}/staging"), json!({"kind": "update_all"})).await;
    assert_eq!((code, e["error"]["code"].as_str()), (400, Some("capability.unsupported")), "the sim runtime has no managed content: {e}");

    // a healthy copy
    let run = t.run_to_end(json!({"kind": "none", "ready_timeout_s": 20, "watch_s": 2})).await;
    let rid = run["id"].as_str().unwrap().to_string();
    assert_eq!((run["state"].as_str(), run["report"]["healthy"].as_bool()), (Some("healthy"), Some(true)), "{run}");
    assert!(run["report"]["ready_secs"].as_f64().unwrap() > 0.0);
    assert_eq!(run["report"]["errors"], 0);
    assert!(run["report"]["warnings"].as_u64().unwrap() >= 1, "the sim prints a WARN line: {run}");
    let copy_port = run["port"].as_i64().unwrap();
    assert!((base_port as i64 + 1..=base_port as i64 + 60).contains(&copy_port) && copy_port != game_port as i64, "port {copy_port} comes from the staging range");
    assert!(!port_open(copy_port), "the copy is stopped once the test is over");
    assert_eq!(t.get(&format!("/servers/{origin}")).await.1["state"], "running", "the real server was never touched");
    assert!(data.join("servers").join(&rid).join("keepme.txt").is_file(), "the copy has the server's files");
    assert!(!data.join("servers").join(&rid).join("world").exists(), "no world unless asked for");

    // hidden from the main list, listed under the origin, flagged when asked for
    let (_, all) = t.get("/servers").await;
    assert!(all.as_array().unwrap().iter().all(|s| s["id"] != rid.as_str()), "staging copies are not in the main list");
    let (_, with) = t.get("/servers?include_staging=true").await;
    let copy = with.as_array().unwrap().iter().find(|s| s["id"] == rid.as_str()).expect("listed on request");
    assert_eq!(copy["staging_of"], origin.as_str());
    let (_, runs) = t.get(&format!("/servers/{origin}/staging")).await;
    assert_eq!(runs.as_array().unwrap().len(), 1);

    // inspection works, hand-driving the copy does not
    let (code, log) = t.get(&format!("/servers/{origin}/staging/{rid}/log")).await;
    assert_eq!(code, 200);
    assert!(log["lines"].as_array().unwrap().iter().any(|l| l.as_str().unwrap().contains("Done (")), "{log}");
    assert_eq!(t.post(&format!("/servers/{rid}/actions/start"), json!({})).await.1["error"]["code"], "staging.managed");
    assert_eq!(t.delete(&format!("/servers/{rid}")).await.0, 409);
    assert_eq!(t.get(&format!("/servers/other/staging/{rid}")).await.0, 404);
    // a run that only tested a copy has nothing to promote
    assert_eq!(t.post(&format!("/servers/{origin}/staging/{rid}/promote"), json!({})).await.1["error"]["code"], "staging.nothing_to_promote");

    // discarding removes every trace
    t.discard(&rid).await;
    assert_eq!(t.get(&format!("/servers/{origin}/staging")).await.1, json!([]));
    assert_eq!(t.get(&format!("/servers/{rid}")).await.0, 404);
    assert!(!data.join("servers").join(&rid).exists());

    // unhealthy: a fatal error in the log, found and reported; the copy is kept for inspection
    t.behavior("fatal\nerror-lines=2\n");
    let run = t.run_to_end(json!({"kind": "none", "ready_timeout_s": 20, "watch_s": 2})).await;
    let rid = run["id"].as_str().unwrap().to_string();
    assert_eq!(run["state"], "unhealthy", "{run}");
    assert!(run["report"]["fatal"].as_str().unwrap().contains("NoClassDefFoundError"));
    assert!(run["report"]["errors"].as_u64().unwrap() >= 3);
    assert!(run["report"]["reasons"][0].as_str().unwrap().contains("fatal error"));
    assert_eq!(run["can_promote"], false);
    assert_eq!(t.get(&format!("/servers/{origin}/staging/{rid}/log")).await.0, 200, "an unhealthy copy stays inspectable");
    assert!(data.join("servers").join(&rid).is_dir());
    t.discard(&rid).await;

    // unhealthy: crash after startup
    t.behavior("crash-after-ready=1\n");
    let run = t.run_to_end(json!({"kind": "none", "ready_timeout_s": 20, "watch_s": 20})).await;
    assert_eq!((run["state"].as_str(), run["report"]["crashed"].as_bool()), (Some("unhealthy"), Some(true)), "{run}");
    t.discard(run["id"].as_str().unwrap()).await;
    let (_, tl) = t.get(&format!("/servers/{origin}/timeline")).await;
    assert!(!tl.to_string().contains("crash"), "a crashing copy is a finding, not an incident on the real server: {tl}");

    // unhealthy: never becomes ready
    t.behavior("never-ready\n");
    let run = t.run_to_end(json!({"kind": "none", "ready_timeout_s": 10, "watch_s": 1})).await;
    assert_eq!(run["state"], "unhealthy", "{run}");
    assert!(run["report"]["reasons"][0].as_str().unwrap().contains("within 10 seconds"), "{run}");
    t.discard(run["id"].as_str().unwrap()).await;

    // cancel while it is being watched; a second run is refused meanwhile
    t.behavior("");
    let (code, r) = t.post(&format!("/servers/{origin}/staging"), json!({"kind": "none", "ready_timeout_s": 20, "watch_s": 120})).await;
    assert_eq!(code, 202);
    let (rid, op) = (r["run"]["id"].as_str().unwrap().to_string(), r["operation"]["id"].as_str().unwrap().to_string());
    until("testing", 30, || async { (t.get(&format!("/servers/{origin}/staging/{rid}")).await.1["state"] == "testing").then_some(()) }).await;
    assert_eq!(t.post(&format!("/servers/{origin}/staging"), json!({"kind": "none"})).await.1["error"]["code"], "staging.busy");
    let copy_port = t.get(&format!("/servers/{origin}/staging/{rid}")).await.1["port"].as_i64().unwrap();
    until("the copy to be running", 20, || async { port_open(copy_port).then_some(()) }).await;
    assert_eq!(t.delete(&format!("/servers/{origin}/staging/{rid}")).await.0, 202);
    assert_eq!(wait_op(&c, &t.base, &op).await["state"], "cancelled");
    until("copy gone", 20, || async { (t.get(&format!("/servers/{rid}")).await.0 == 404 && !port_open(copy_port)).then_some(()) }).await;
    assert!(!data.join("servers").join(&rid).exists());

    // a controller restart in the middle of a run: the copy is stopped and removed on boot
    let (code, r) = t.post(&format!("/servers/{origin}/staging"), json!({"kind": "none", "ready_timeout_s": 20, "watch_s": 120})).await;
    assert_eq!(code, 202);
    let rid = r["run"]["id"].as_str().unwrap().to_string();
    until("testing", 30, || async { (t.get(&format!("/servers/{origin}/staging/{rid}")).await.1["state"] == "testing").then_some(()) }).await;
    let copy_port = t.get(&format!("/servers/{origin}/staging/{rid}")).await.1["port"].as_i64().unwrap();
    until("the copy to be running", 20, || async { port_open(copy_port).then_some(()) }).await;
    let mut moss = moss;
    moss.child.kill().unwrap();
    moss.child.wait().unwrap();
    assert!(port_open(copy_port), "game servers outlive the controller, so the copy is still running");
    let moss2 = Moss::start(&data, port, &env);
    until("controller back", 20, || async { c.get(format!("http://127.0.0.1:{port}/healthz")).send().await.ok().filter(|r| r.status().is_success()) }).await;
    until("interrupted copy cleaned up", 20, || async { (t.get(&format!("/servers/{origin}/staging")).await.1 == json!([]) && !port_open(copy_port)).then_some(()) }).await;
    assert!(!data.join("servers").join(&rid).exists());
    assert_eq!(t.get(&format!("/servers/{origin}")).await.1["state"], "running", "the real server was re-attached, not disturbed");
    drop(moss2);
    drop(moss);
}
