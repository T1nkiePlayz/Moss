//! A workflow end to end against the real binary and the simulated game server: starting the server derives a
//! `server.started` event, which runs the workflow's command action on that server (placeholders filled in), is recorded
//! in the run history, and a dry run reports the same plan without executing anything.
mod suite;
use serde_json::{Value, json};
use suite::{Moss, free_port, until, wait_op};

#[tokio::test(flavor = "multi_thread")]
async fn server_started_event_runs_a_command_action() {
    let data = std::env::temp_dir().join(format!("moss-workflow-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data);
    let port = free_port();
    let moss = Moss::start(&data, port, &[]);
    let c = reqwest::Client::new();
    until("controller", 20, || async { c.get(format!("http://127.0.0.1:{port}/healthz")).send().await.ok().filter(|r| r.status().is_success()) }).await;
    let created: Value = c.post(format!("{}/servers", moss.base)).json(&json!({"name": "wf-sim", "runtime": "sim", "game_version": "sim-1.0", "memory_mb": 512, "port": free_port()})).send().await.unwrap().json().await.unwrap();
    let sid = created["server"]["id"].as_str().unwrap().to_string();
    assert_eq!(wait_op(&c, &moss.base, created["operation"]["id"].as_str().unwrap()).await["state"], "completed");

    // validation
    let bad = c.post(format!("{}/workflows", moss.base)).json(&json!({"name": "x", "trigger": "nope", "actions": []})).send().await.unwrap();
    assert_eq!(bad.status(), 400);
    let w: Value = c.post(format!("{}/workflows", moss.base)).json(&json!({"name": "greet", "trigger": "server.started", "server_ids": [sid],
        "actions": [{"type": "command", "command": "say workflow-hello {server}"}]})).send().await.unwrap().json().await.unwrap();
    let wid = w["id"].as_str().unwrap().to_string();
    assert_eq!(w["enabled"], true);

    // dry run: describes the command, executes nothing
    let plan: Value = c.post(format!("{}/workflows/{wid}/test", moss.base)).send().await.unwrap().json().await.unwrap();
    assert_eq!((plan["executed"].clone(), plan["would_run"].clone()), (json!(false), json!(true)), "{plan}");
    assert_eq!(plan["steps"][0]["description"], "Send to wf-sim: say workflow-hello wf-sim");
    let runs: Value = c.get(format!("{}/workflows/{wid}/runs", moss.base)).send().await.unwrap().json().await.unwrap();
    assert!(runs["items"].as_array().unwrap().is_empty(), "a dry run records nothing");

    // the real event
    assert!(c.post(format!("{}/servers/{sid}/actions/start", moss.base)).send().await.unwrap().status().is_success());
    let run = until("workflow run", 60, || async {
        let r: Value = c.get(format!("{}/workflows/{wid}/runs", moss.base)).send().await.ok()?.json().await.ok()?;
        r["items"].as_array()?.first().cloned()
    }).await;
    assert_eq!(run["ok"], true, "{run}");
    assert_eq!(run["event"]["type"], "server.started");
    until("command in the console", 20, || async {
        let b: Value = c.get(format!("{}/servers/{sid}/console/backlog?lines=300", moss.base)).send().await.ok()?.json().await.ok()?;
        b["lines"].as_array()?.iter().any(|l| l.as_str().is_some_and(|l| l.contains("say workflow-hello wf-sim"))).then_some(())
    }).await;
    let got: Value = c.get(format!("{}/workflows/{wid}", moss.base)).send().await.unwrap().json().await.unwrap();
    assert_eq!(got["last_result"], "ok");
    let list: Value = c.get(format!("{}/workflows", moss.base)).send().await.unwrap().json().await.unwrap();
    assert_eq!(list["items"].as_array().unwrap().len(), 1);

    assert_eq!(c.delete(format!("{}/workflows/{wid}", moss.base)).send().await.unwrap().status(), 204);
    assert_eq!(c.get(format!("{}/workflows/{wid}", moss.base)).send().await.unwrap().status(), 404);
    let _ = c.post(format!("{}/servers/{sid}/actions/stop", moss.base)).send().await;
}
