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

async fn make_sim(c: &reqwest::Client, base: &str, name: &str) -> String {
    let created: Value = c.post(format!("{base}/servers")).json(&json!({"name": name, "runtime": "sim", "game_version": "sim-1.0", "memory_mb": 512})).send().await.unwrap().json().await.unwrap();
    let op = created["operation"]["id"].as_str().unwrap().to_string();
    let end = Instant::now() + Duration::from_secs(60);
    loop {
        let o: Value = c.get(format!("{base}/operations/{op}")).send().await.unwrap().json().await.unwrap();
        if o["state"] == "completed" { break; }
        assert!(Instant::now() < end, "install did not finish");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    created["server"]["id"].as_str().unwrap().to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn file_api_cannot_reach_controller_metadata_or_delete_the_root_by_odd_spellings() {
    let dir = tmp("files");
    let m = Moss::start(&dir, &[]);
    let c = reqwest::Client::new();
    m.ready(&c).await;
    let id = make_sim(&c, &m.base, "f").await;
    let f = format!("{}/servers/{id}/files", m.base);
    std::fs::create_dir_all(dir.join("servers").join(&id).join(".moss")).unwrap();
    std::fs::write(dir.join("servers").join(&id).join(".moss/marker"), "x").unwrap();
    for p in [".moss", "./.moss", "./.moss/", ".MOSS", "/./.moss"] {
        assert_eq!(c.get(&f).query(&[("path", p)]).send().await.unwrap().status(), 403, "list {p}");
        assert_eq!(c.get(format!("{f}/content")).query(&[("path", format!("{p}/marker"))]).send().await.unwrap().status(), 403, "read {p}");
        assert_eq!(c.delete(&f).query(&[("path", p)]).send().await.unwrap().status(), 403, "delete {p}");
    }
    for p in ["", ".", "./", "//"] { assert_eq!(c.delete(&f).query(&[("path", p)]).send().await.unwrap().status(), 403, "delete root as {p:?}"); }
    assert_eq!(c.put(format!("{f}/content")).json(&json!({"path": "./.moss/marker", "content": "pwn"})).send().await.unwrap().status(), 403);
    assert_eq!(std::fs::read_to_string(dir.join("servers").join(&id).join(".moss/marker")).unwrap(), "x");
    let listing: Value = c.get(&f).query(&[("path", "./")]).send().await.unwrap().json().await.unwrap();
    assert!(!listing["entries"].as_array().unwrap().iter().any(|e| e["name"] == ".moss"), "the metadata directory is hidden from every spelling of the root");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_data_directory_and_the_scratch_directory_are_private() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tmp("perm");
    let m = Moss::start(&dir, &[]);
    let c = reqwest::Client::new();
    m.ready(&c).await;
    let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&dir), 0o700);
    assert_eq!(mode(&dir.join("tmp")), 0o700);
}
