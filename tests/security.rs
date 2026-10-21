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

/// Public status pages are the one unauthenticated data surface: they work without credentials, expose only what the
/// owner opted in, answer missing and disabled pages identically, and nothing else became public.
#[tokio::test(flavor = "multi_thread")]
async fn public_status_pages_are_open_but_everything_else_stays_protected() {
    let dir = tmp("statuspages");
    let (page_id, off_id, server_id);
    let c = reqwest::Client::new();
    {
        // phase 1: configure as the signed-in owner (dev mode)
        let m = Moss::start(&dir, &[]);
        m.ready(&c).await;
        server_id = make_sim(&c, &m.base, "Survival <img src=x>").await;
        let sp = format!("{}/status-pages", m.base);
        let bad = c.post(&sp).json(&json!({"title": "T", "slug": "Bad Slug!"})).send().await.unwrap();
        assert_eq!(bad.status(), 400, "invalid slugs are refused");
        assert_eq!(c.post(&sp).json(&json!({"title": "", "server_ids": []})).send().await.unwrap().status(), 400);
        assert_eq!(c.post(&sp).json(&json!({"title": "T", "server_ids": ["nope"]})).send().await.unwrap().status(), 400, "unknown servers are refused");
        let made: Value = c.post(&sp).json(&json!({"title": "Our <b>servers</b>", "description": "Join us", "show_address": "play.example.com", "server_ids": [server_id]})).send().await.unwrap().json().await.unwrap();
        page_id = made["id"].as_str().unwrap().to_string();
        let slug = made["slug"].as_str().unwrap().to_string();
        assert!(slug.len() >= 10, "default slugs are unguessable: {slug}");
        assert_eq!(c.post(&sp).json(&json!({"title": "Dup", "slug": slug})).send().await.unwrap().status(), 409);
        let off: Value = c.post(&sp).json(&json!({"title": "Hidden", "slug": "hidden-page", "enabled": false})).send().await.unwrap().json().await.unwrap();
        off_id = off["id"].as_str().unwrap().to_string();
        let patched: Value = c.patch(format!("{sp}/{page_id}")).json(&json!({"slug": "our-servers", "show_players": true})).send().await.unwrap().json().await.unwrap();
        assert_eq!((patched["slug"].as_str(), patched["show_players"].as_bool()), (Some("our-servers"), Some(true)));
        assert_eq!(c.get(&sp).send().await.unwrap().json::<Value>().await.unwrap()["items"].as_array().unwrap().len(), 2);
        // the activity log records the change
        let act = c.get(format!("{}/activity", m.base)).send().await.unwrap().text().await.unwrap();
        assert!(act.contains("status_page.add") && act.contains("status_page.update"), "audited");
    }
    // phase 2: the same data directory with real authentication, and no credentials at all
    let m = Moss::start(&dir, &[("MOSS_AUTH", "supabase"), ("SUPABASE_URL", "http://127.0.0.1:1")]);
    m.ready(&c).await;
    let root = format!("http://127.0.0.1:{}", m.port);
    let json_r = c.get(format!("{}/public/status/our-servers", m.base)).send().await.unwrap();
    assert_eq!(json_r.status(), 200);
    assert_eq!(json_r.headers()["cache-control"], "public, max-age=15");
    let body = json_r.text().await.unwrap();
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!((v["title"].as_str(), v["address"].as_str()), (Some("Our <b>servers</b>"), Some("play.example.com")));
    assert_eq!(v["servers"][0]["name"], "Survival <img src=x>");
    assert_eq!(v["servers"][0]["state"], "offline");
    for hidden in [server_id.as_str(), page_id.as_str(), "\"pid\"", "\"port\"", "data_dir", "\"path\"", "memory"] { assert!(!body.contains(hidden), "{hidden} leaked: {body}"); }
    let html_r = c.get(format!("{root}/status/our-servers")).send().await.unwrap();
    assert_eq!(html_r.status(), 200);
    let csp = html_r.headers()["content-security-policy"].to_str().unwrap().to_string();
    assert!(csp.starts_with("default-src 'none'; style-src 'unsafe-inline'; img-src data:"), "{csp}");
    let html = html_r.text().await.unwrap();
    assert!(html.contains("Our &lt;b&gt;servers&lt;/b&gt;") && html.contains("Survival &lt;img src=x&gt;") && !html.contains("<img") && !html.contains("<b>"), "{html}");
    assert!(html.contains("http-equiv=\"refresh\""));
    // missing and disabled pages are indistinguishable
    let missing = c.get(format!("{root}/status/does-not-exist")).send().await.unwrap();
    let disabled = c.get(format!("{root}/status/hidden-page")).send().await.unwrap();
    assert_eq!((missing.status().as_u16(), disabled.status().as_u16()), (404, 404));
    assert_eq!(missing.text().await.unwrap(), disabled.text().await.unwrap());
    let missing = c.get(format!("{}/public/status/does-not-exist", m.base)).send().await.unwrap();
    let disabled = c.get(format!("{}/public/status/hidden-page", m.base)).send().await.unwrap();
    assert_eq!((missing.status().as_u16(), disabled.status().as_u16()), (404, 404));
    // identical apart from the per-request id
    let strip = |t: String| t.split("\"request_id\":\"").enumerate().map(|(i, s)| if i == 0 { s.to_string() } else { s.split_once('"').unwrap().1.to_string() }).collect::<String>();
    assert_eq!(strip(missing.text().await.unwrap()), strip(disabled.text().await.unwrap()));
    // the admin API and every other data route still need credentials
    let sp = format!("{}/status-pages", m.base);
    assert_eq!(c.get(&sp).send().await.unwrap().status(), 401);
    assert_eq!(c.post(&sp).json(&json!({"title": "x"})).send().await.unwrap().status(), 401);
    assert_eq!(c.patch(format!("{sp}/{page_id}")).json(&json!({"enabled": false})).send().await.unwrap().status(), 401);
    assert_eq!(c.delete(format!("{sp}/{off_id}")).send().await.unwrap().status(), 401);
    for p in ["/status", "/servers", "/settings", "/users", "/activity", "/webhooks", "/backups", "/health", "/tokens"] {
        assert_eq!(c.get(format!("{}{p}", m.base)).send().await.unwrap().status(), 401, "GET {p} must stay protected");
    }
    assert_eq!(c.get(format!("{}/servers/{server_id}", m.base)).send().await.unwrap().status(), 401);
}
