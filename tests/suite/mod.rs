#![allow(dead_code)]
//! Lifecycle conformance suite. One table of expectations that every runtime backend must meet; a backend plugs in by
//! implementing `Backend` (how to create the server and which console commands crash it / echo output) and calling
//! `run_suite`. The simulated runtime is the first user (see `tests/conformance.rs`).
use serde_json::{Value, json};
use std::{future::Future, net::TcpListener, pin::Pin, process::{Child, Command, Stdio}, time::{Duration, Instant}};

/// What a runtime backend has to tell the suite.
pub trait Backend {
    fn name(&self) -> &str;
    /// JSON body for `POST /servers` (the suite adds `name` and `port`).
    fn create_body(&self) -> Value;
    /// A console line that makes the process die abnormally, if the backend has one (otherwise the crash cases are skipped).
    fn crash_command(&self) -> Option<&str>;
    /// Console line the game prints back verbatim (the suite looks for `needle` in the console backlog).
    fn echo_command(&self) -> (String, String);
}

pub struct Ctx { pub c: reqwest::Client, pub base: String, pub id: String, pub port: u16, pub backend: Box<dyn Backend> }

impl Ctx {
    pub async fn state(&self) -> Option<Value> { self.c.get(format!("{}/servers/{}", self.base, self.id)).send().await.ok()?.json().await.ok() }
    pub async fn action(&self, a: &str) { assert!(self.c.post(format!("{}/servers/{}/actions/{a}", self.base, self.id)).send().await.unwrap().status().is_success(), "action {a}"); }
    pub async fn command(&self, cmd: &str) { assert_eq!(self.c.post(format!("{}/servers/{}/command", self.base, self.id)).json(&json!({"command": cmd})).send().await.unwrap().status(), 204); }
    pub async fn backlog_has(&self, needle: &str) -> bool {
        let Ok(r) = self.c.get(format!("{}/servers/{}/console/backlog?lines=300", self.base, self.id)).send().await else { return false };
        let Ok(b) = r.json::<Value>().await else { return false };
        b["lines"].as_array().is_some_and(|l| l.iter().any(|x| x.as_str().is_some_and(|x| x.contains(needle))))
    }
    pub async fn wait_state(&self, what: &str, secs: u64, want: impl Fn(&Value) -> bool) -> Value {
        until(what, secs, || async { let s = self.state().await?; want(&s).then_some(s) }).await
    }
}

pub async fn until<F: FnMut() -> Fut, Fut: Future<Output = Option<T>>, T>(what: &str, secs: u64, mut f: F) -> T {
    let end = Instant::now() + Duration::from_secs(secs);
    loop {
        if let Some(v) = f().await { return v; }
        assert!(Instant::now() < end, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

pub fn free_port() -> u16 { TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port() }

pub struct Moss { pub child: Child, pub base: String, pub dir: std::path::PathBuf }
impl Moss {
    pub fn start(dir: &std::path::Path, port: u16, extra_env: &[(&str, String)]) -> Self {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_moss"));
        cmd.arg("serve").env("MOSS_AUTH", "dev").env("MOSS_DEV_RUNTIMES", "true").env("MOSS_PORT", port.to_string()).env("MOSS_DATA_DIR", dir).env("RUST_LOG", "moss=warn").stdout(Stdio::null()).stderr(Stdio::null());
        for (k, v) in extra_env { cmd.env(k, v); }
        Self { child: cmd.spawn().expect("spawn moss"), base: format!("http://127.0.0.1:{port}/api/v1"), dir: dir.to_path_buf() }
    }
}
impl Drop for Moss { fn drop(&mut self) { let _ = self.child.kill(); let _ = self.child.wait(); let _ = std::fs::remove_dir_all(&self.dir); } }

pub async fn wait_op(c: &reqwest::Client, base: &str, id: &str) -> Value {
    until("operation to finish", 90, || async { let o: Value = c.get(format!("{base}/operations/{id}")).send().await.ok()?.json().await.ok()?; matches!(o["state"].as_str()?, "completed" | "failed" | "cancelled").then_some(o) }).await
}

type Case = for<'a> fn(&'a Ctx) -> Pin<Box<dyn Future<Output = ()> + 'a>>;

macro_rules! case { ($f:expr) => { |c| Box::pin($f(c)) }; }

/// The table. Order matters: later cases start from the state the earlier ones leave (stopped).
pub const CASES: &[(&str, Case)] = &[
    ("start reaches ready", case!(start_ready)),
    ("console input is accepted and echoed", case!(console_echo)),
    ("graceful stop ends stopped with no restart", case!(graceful_stop)),
    ("kill ends stopped and stays stopped", case!(kill_stays_stopped)),
    ("crash is detected and restarted with growing backoff", case!(crash_backoff)),
    ("a stopped server rejects console input", case!(stopped_rejects_input)),
];

pub async fn run_suite(backend: impl Backend + 'static, only: Option<&str>) {
    let data = std::env::temp_dir().join(format!("moss-conf-{}-{}", backend.name(), std::process::id()));
    let _ = std::fs::remove_dir_all(&data);
    let port = free_port();
    let moss = Moss::start(&data, port, &[]);
    let c = reqwest::Client::new();
    until("controller", 20, || async { c.get(format!("http://127.0.0.1:{port}/healthz")).send().await.ok().filter(|r| r.status().is_success()) }).await;
    let game_port = free_port();
    let mut body = backend.create_body();
    body["name"] = json!(format!("conf-{}", backend.name()));
    body["port"] = json!(game_port);
    let created: Value = c.post(format!("{}/servers", moss.base)).json(&body).send().await.unwrap().json().await.unwrap();
    let id = created["server"]["id"].as_str().expect("server id").to_string();
    assert_eq!(wait_op(&c, &moss.base, created["operation"]["id"].as_str().unwrap()).await["state"], "completed");
    let ctx = Ctx { c, base: moss.base.clone(), id, port: game_port, backend: Box::new(backend) };
    for (name, case) in CASES {
        if only.is_some_and(|o| o != *name) { continue; }
        eprintln!("conformance[{}]: {name}", ctx.backend.name());
        case(&ctx).await;
    }
    drop(moss);
}

async fn start_ready(x: &Ctx) {
    x.action("start").await;
    let s = x.wait_state("ready", 30, |s| s["state"] == "running").await;
    assert!(s["pid"].is_number(), "a running server reports its pid");
    assert!(std::net::TcpStream::connect_timeout(&format!("127.0.0.1:{}", x.port).parse().unwrap(), Duration::from_secs(2)).is_ok(), "the game port accepts connections once ready");
    let again = x.c.post(format!("{}/servers/{}/actions/start", x.base, x.id)).send().await.unwrap();
    assert_eq!(again.status(), 409, "starting twice is a conflict, not a second process");
}

async fn console_echo(x: &Ctx) {
    let (cmd, needle) = x.backend.echo_command();
    x.command(&cmd).await;
    until("echo in the console backlog", 10, || async { x.backlog_has(&needle).await.then_some(()) }).await;
}

async fn graceful_stop(x: &Ctx) {
    x.action("stop").await;
    let s = x.wait_state("stopped", 30, |s| s["state"] == "stopped").await;
    assert_eq!(s["desired"], "stopped");
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(x.state().await.unwrap()["state"], "stopped", "a clean stop is never restarted");
}

async fn kill_stays_stopped(x: &Ctx) {
    x.action("start").await;
    x.wait_state("running", 30, |s| s["state"] == "running").await;
    x.action("kill").await;
    x.wait_state("stopped after kill", 20, |s| s["state"] == "stopped").await;
    tokio::time::sleep(Duration::from_secs(4)).await;
    let s = x.state().await.unwrap();
    assert_eq!((s["state"].as_str(), s["desired"].as_str()), (Some("stopped"), Some("stopped")), "kill is a deliberate stop");
}

async fn crash_backoff(x: &Ctx) {
    let Some(crash) = x.backend.crash_command().map(String::from) else { return };
    x.action("start").await;
    x.wait_state("running", 30, |s| s["state"] == "running").await;
    let pid1 = x.state().await.unwrap()["pid"].as_i64().unwrap();
    x.command(&crash).await;
    let s = x.wait_state("crashed with a scheduled restart", 15, |s| s["state"] == "crashed" && s["state_detail"].as_str().is_some_and(|d| d.contains("restarting in 2s"))).await;
    assert!(s["state_detail"].as_str().unwrap().contains("crashed"));
    let s = x.wait_state("automatic restart", 30, |s| s["state"] == "running").await;
    assert_ne!(s["pid"].as_i64().unwrap(), pid1, "the restart is a new process");
    x.command(&crash).await;
    x.wait_state("second crash backs off longer", 15, |s| s["state"] == "crashed" && s["state_detail"].as_str().is_some_and(|d| d.contains("restarting in 4s"))).await;
    x.wait_state("second automatic restart", 30, |s| s["state"] == "running").await;
    x.action("kill").await;
    x.wait_state("stopped", 20, |s| s["state"] == "stopped").await;
}

async fn stopped_rejects_input(x: &Ctx) {
    let r = x.c.post(format!("{}/servers/{}/command", x.base, x.id)).json(&json!({"command": "list"})).send().await.unwrap();
    assert_eq!(r.status(), 409);
}
