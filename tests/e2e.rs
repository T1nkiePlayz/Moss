//! End to end: drives the real binary over HTTP with the simulated game server.
//! Covers the promises Moss is built on: servers survive a SIGKILLed controller and are re-attached,
//! the console works, and a backup can be created and verified.
mod suite;
use serde_json::{Value, json};
use std::{net::TcpListener, process::{Child, Command, Stdio}, time::{Duration, Instant}};

struct Moss { child: Child, base: String, dir: std::path::PathBuf, port: u16 }

fn free_port() -> u16 { TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port() }

impl Moss {
    fn start(dir: &std::path::Path, port: u16) -> Self {
        let child = Command::new(env!("CARGO_BIN_EXE_moss")).arg("serve")
            .env("MOSS_AUTH", "dev").env("MOSS_DEV_RUNTIMES", "true").env("MOSS_PORT", port.to_string()).env("MOSS_DATA_DIR", dir)
            .env("RUST_LOG", "moss=warn").stdout(Stdio::null()).stderr(Stdio::null()).spawn().expect("spawn moss");
        Self { child, base: format!("http://127.0.0.1:{port}/api/v1"), dir: dir.to_path_buf(), port }
    }
    fn sigkill(&mut self) { self.child.kill().unwrap(); self.child.wait().unwrap(); }
}
impl Drop for Moss { fn drop(&mut self) { let _ = self.child.kill(); let _ = self.child.wait(); suite::reap_servers(&self.dir); } }

async fn until<F: FnMut() -> Fut, Fut: std::future::Future<Output = Option<T>>, T>(what: &str, secs: u64, mut f: F) -> T {
    let end = Instant::now() + Duration::from_secs(secs);
    loop {
        if let Some(v) = f().await { return v; }
        assert!(Instant::now() < end, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

async fn get(c: &reqwest::Client, url: String) -> Option<Value> { c.get(url).send().await.ok()?.json().await.ok() }

async fn server_state(c: &reqwest::Client, base: &str, id: &str) -> Option<Value> { get(c, format!("{base}/servers/{id}")).await }

async fn wait_op(c: &reqwest::Client, base: &str, id: &str) -> Value {
    until("operation to finish", 60, || async { let o = get(c, format!("{base}/operations/{id}")).await?; matches!(o["state"].as_str()?, "completed" | "failed" | "cancelled").then_some(o) }).await
}

#[tokio::test(flavor = "multi_thread")]
async fn servers_survive_the_controller_and_backups_verify() {
    let data = std::env::temp_dir().join(format!("moss-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data);
    let port = free_port();
    let game_port = free_port();
    let c = reqwest::Client::new();
    let mut moss = Moss::start(&data, port);
    until("controller to answer", 20, || async { c.get(format!("http://127.0.0.1:{port}/healthz")).send().await.ok().filter(|r| r.status().is_success()) }).await;
    let base = moss.base.clone();

    // create + install the simulated server
    let created: Value = c.post(format!("{base}/servers")).json(&json!({"name": "e2e", "runtime": "sim", "game_version": "sim-1.0", "memory_mb": 512, "port": game_port}))
        .send().await.unwrap().json().await.unwrap();
    let id = created["server"]["id"].as_str().expect("server id").to_string();
    let op = created["operation"]["id"].as_str().unwrap().to_string();
    assert_eq!(wait_op(&c, &base, &op).await["state"], "completed");

    // start and wait for readiness
    assert!(c.post(format!("{base}/servers/{id}/actions/start")).send().await.unwrap().status().is_success());
    let running = until("server to be running", 30, || async { let s = server_state(&c, &base, &id).await?; (s["state"] == "running").then_some(s) }).await;
    let pid = running["pid"].as_i64().expect("pid");

    // console round trip
    assert_eq!(c.post(format!("{base}/servers/{id}/command")).json(&json!({"command": "say hello from e2e"})).send().await.unwrap().status(), 204);
    until("command echo in the console", 10, || async {
        let b = get(&c, format!("{base}/servers/{id}/console/backlog?lines=200")).await?;
        b["lines"].as_array()?.iter().any(|l| l.as_str().is_some_and(|l| l.contains("say hello from e2e"))).then_some(())
    }).await;

    // SIGKILL the controller: the game must keep running
    moss.sigkill();
    assert!(std::net::TcpStream::connect_timeout(&format!("127.0.0.1:{game_port}").parse().unwrap(), Duration::from_secs(2)).is_ok(), "game server died with the controller");

    // a fresh controller re-attaches to the same process
    let mut moss = Moss::start(&data, moss.port);
    let again = until("re-attach", 30, || async { let s = server_state(&c, &moss.base, &id).await?; (s["state"] == "running").then_some(s) }).await;
    assert_eq!(again["pid"].as_i64(), Some(pid), "controller must re-attach to the original process");
    assert_eq!(c.post(format!("{}/servers/{id}/command", moss.base)).json(&json!({"command": "say after restart"})).send().await.unwrap().status(), 204);

    // backup -> verify -> listed as ok and verified
    let bk: Value = c.post(format!("{}/servers/{id}/backups", moss.base)).json(&json!({"note": "e2e"})).send().await.unwrap().json().await.unwrap();
    assert_eq!(wait_op(&c, &moss.base, bk["operation"]["id"].as_str().unwrap()).await["state"], "completed");
    let list = get(&c, format!("{}/servers/{id}/backups", moss.base)).await.unwrap();
    let b = &list.as_array().unwrap()[0];
    assert_eq!(b["status"], "ok");
    let vf: Value = c.post(format!("{}/backups/{}/verify", moss.base, b["id"].as_str().unwrap())).send().await.unwrap().json().await.unwrap();
    assert_eq!(wait_op(&c, &moss.base, vf["operation"]["id"].as_str().unwrap()).await["state"], "completed");

    // graceful stop
    assert!(c.post(format!("{}/servers/{id}/actions/kill", moss.base)).send().await.unwrap().status().is_success());
    until("server to stop", 20, || async { let s = server_state(&c, &moss.base, &id).await?; (s["state"] == "stopped").then_some(()) }).await;
    moss.sigkill();
    let _ = std::fs::remove_dir_all(&moss.dir);
}

/// A stop is recorded first and delivered from a background task. A controller that dies in between leaves a live
/// process whose desired state is already "stopped": the next controller must finish the stop (found by the soak test).
#[tokio::test(flavor = "multi_thread")]
async fn a_stop_lost_to_a_controller_crash_is_finished_after_restart() {
    let data = std::env::temp_dir().join(format!("moss-e2e-stop-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data);
    let (port, game_port) = (free_port(), free_port());
    let c = reqwest::Client::new();
    let mut moss = Moss::start(&data, port);
    until("controller to answer", 20, || async { c.get(format!("http://127.0.0.1:{port}/healthz")).send().await.ok().filter(|r| r.status().is_success()) }).await;
    let created: Value = c.post(format!("{}/servers", moss.base)).json(&json!({"name": "lost-stop", "runtime": "sim", "game_version": "sim-1.0", "memory_mb": 512, "port": game_port}))
        .send().await.unwrap().json().await.unwrap();
    let id = created["server"]["id"].as_str().unwrap().to_string();
    assert_eq!(wait_op(&c, &moss.base, created["operation"]["id"].as_str().unwrap()).await["state"], "completed");
    assert!(c.post(format!("{}/servers/{id}/actions/start", moss.base)).send().await.unwrap().status().is_success());
    until("server to be running", 30, || async { (server_state(&c, &moss.base, &id).await?["state"] == "running").then_some(()) }).await;
    // the controller recorded the stop wish and died before the background task delivered it
    moss.sigkill();
    rusqlite::Connection::open(data.join("moss.sqlite")).unwrap().execute("UPDATE servers SET desired='stopped' WHERE id=?", [&id]).unwrap();
    let mut moss = Moss::start(&data, moss.port);
    until("the pending stop to be finished", 40, || async { (server_state(&c, &moss.base, &id).await?["state"] == "stopped").then_some(()) }).await;
    assert!(std::net::TcpStream::connect_timeout(&format!("127.0.0.1:{game_port}").parse().unwrap(), Duration::from_millis(500)).is_err(), "the game is still listening");
    moss.sigkill();
    let _ = std::fs::remove_dir_all(&data);
}

/// Pressing Stop or Kill while a restart is under way must win: the restart must not bring the server back
/// (found by the soak test).
#[tokio::test(flavor = "multi_thread")]
async fn stop_during_a_restart_wins() {
    let data = std::env::temp_dir().join(format!("moss-e2e-restart-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data);
    let (port, game_port) = (free_port(), free_port());
    let c = reqwest::Client::new();
    let mut moss = Moss::start(&data, port);
    until("controller to answer", 20, || async { c.get(format!("http://127.0.0.1:{port}/healthz")).send().await.ok().filter(|r| r.status().is_success()) }).await;
    let created: Value = c.post(format!("{}/servers", moss.base)).json(&json!({"name": "restart-stop", "runtime": "sim", "game_version": "sim-1.0", "memory_mb": 512, "port": game_port}))
        .send().await.unwrap().json().await.unwrap();
    let id = created["server"]["id"].as_str().unwrap().to_string();
    assert_eq!(wait_op(&c, &moss.base, created["operation"]["id"].as_str().unwrap()).await["state"], "completed");
    for action in ["stop", "kill"] {
        assert!(c.post(format!("{}/servers/{id}/actions/start", moss.base)).send().await.unwrap().status().is_success());
        until("server to be running", 30, || async { (server_state(&c, &moss.base, &id).await?["state"] == "running").then_some(()) }).await;
        assert!(c.post(format!("{}/servers/{id}/actions/restart", moss.base)).send().await.unwrap().status().is_success());
        assert!(c.post(format!("{}/servers/{id}/actions/{action}", moss.base)).send().await.unwrap().status().is_success());
        until("server to be stopped", 30, || async { (server_state(&c, &moss.base, &id).await?["state"] == "stopped").then_some(()) }).await;
        // the restart's background task would start it again about a second later
        tokio::time::sleep(Duration::from_millis(3500)).await;
        let s = server_state(&c, &moss.base, &id).await.unwrap();
        assert_eq!(s["state"], "stopped", "{action} during a restart was overridden: {s}");
    }
    moss.sigkill();
    let _ = std::fs::remove_dir_all(&data);
}
