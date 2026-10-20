//! Regression tests for findings of the internal security review (docs/security-review.md): each test pins one fix
//! against the real binary over HTTP.
use serde_json::{Value, json};
use std::{net::TcpListener, process::{Child, Command, Stdio}, time::{Duration, Instant}};

struct Moss { child: Child, base: String, port: u16 }
fn free_port() -> u16 { TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port() }
impl Moss {
    fn start(dir: &std::path::Path, extra: &[(&str, &str)]) -> Self {
        let port = free_port();
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_moss"));
        cmd.arg("serve").env("MOSS_AUTH", "dev").env("MOSS_DEV_RUNTIMES", "true").env("MOSS_PORT", port.to_string()).env("MOSS_DATA_DIR", dir).env("RUST_LOG", "moss=warn").stdout(Stdio::null()).stderr(Stdio::null());
        for (k, v) in extra { cmd.env(k, v); }
        Self { child: cmd.spawn().expect("spawn moss"), base: format!("http://127.0.0.1:{port}/api/v1"), port }
    }
    async fn ready(&self, c: &reqwest::Client) {
        let end = Instant::now() + Duration::from_secs(20);
        while c.get(format!("http://127.0.0.1:{}/healthz", self.port)).send().await.map_or(true, |r| !r.status().is_success()) { assert!(Instant::now() < end, "controller did not start"); tokio::time::sleep(Duration::from_millis(100)).await; }
    }
}
impl Drop for Moss { fn drop(&mut self) { let _ = self.child.kill(); let _ = self.child.wait(); } }

fn tmp(n: &str) -> std::path::PathBuf { let d = std::env::temp_dir().join(format!("moss-sec-{n}-{}", std::process::id())); let _ = std::fs::remove_dir_all(&d); d }
async fn ticket(c: &reqwest::Client, base: &str) -> String { c.post(format!("{base}/ticket")).send().await.unwrap().json::<Value>().await.unwrap()["ticket"].as_str().unwrap().to_string() }

#[tokio::test(flavor = "multi_thread")]
async fn streams_refuse_cross_origin_requests_and_keep_the_ticket() {
    let m = Moss::start(&tmp("origin"), &[]);
    let c = reqwest::Client::new();
    m.ready(&c).await;
    let host = format!("127.0.0.1:{}", m.port);
    // event stream: a foreign Origin is refused before the single-use ticket is spent
    let t = ticket(&c, &m.base).await;
    let r = c.get(format!("{}/events?ticket={t}", m.base)).header("Origin", "https://evil.example").send().await.unwrap();
    assert_eq!(r.status(), 403);
    let r = c.get(format!("{}/events?ticket={t}", m.base)).header("Origin", format!("http://{host}")).send().await.unwrap();
    assert_eq!(r.status(), 200, "same-origin request with the same ticket still works");
    // console WebSocket upgrade
    let created: Value = c.post(format!("{}/servers", m.base)).json(&json!({"name": "o", "runtime": "sim", "game_version": "sim-1.0", "memory_mb": 512})).send().await.unwrap().json().await.unwrap();
    let id = created["server"]["id"].as_str().unwrap();
    let upgrade = |t: &str, origin: &str| c.get(format!("{}/servers/{id}/console?ticket={t}", m.base)).header("Connection", "Upgrade").header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13").header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==").header("Origin", origin.to_string());
    let t = ticket(&c, &m.base).await;
    assert_eq!(upgrade(&t, "https://evil.example").send().await.unwrap().status(), 403);
    assert_eq!(upgrade(&t, &format!("http://{host}")).send().await.unwrap().status(), 101);
}
