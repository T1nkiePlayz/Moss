//! Soak test: runs the real binary with the simulated game server for a while and does random, hostile things to it,
//! checking invariants as it goes. Ignored by default (it takes minutes); run it with
//!
//!   MOSS_SOAK_MINUTES=2 cargo test --release --test soak -- --ignored --nocapture
//!
//! Environment: MOSS_SOAK_MINUTES (default 2), MOSS_SOAK_SEED (default: time based, printed), MOSS_SOAK_DIR (where the
//! data directory and controller log go; kept when the test fails or MOSS_SOAK_KEEP is set).
//!
//! Operations (random, weighted): start, stop, restart, kill, console spam, backup + verify, in-place restore, a
//! schedule that is made due and must fire, SIGKILL of the controller followed by re-attach, and a Modrinth stub that
//! cycles through healthy, 500, 429, hanging and connection-reset answers while content searches keep coming.
//! The data directory starts as one written by an older release (schema version 6) so the upgrade runs first.
//!
//! Invariants: no orphaned or ghost game processes, no operations stuck in queued/running, every backup verifies, the
//! database passes `PRAGMA integrity_check`, controller memory and file descriptors stay bounded, the controller
//! answers /healthz quickly all the time, and the controller log contains no panics.
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, atomic::{AtomicU8, AtomicUsize, Ordering}},
    time::{Duration, Instant},
};

const OLD: [&str; 6] = [
    include_str!("../src/db/migrations/0001_init.sql"), include_str!("../src/db/migrations/0002_limits.sql"), include_str!("../src/db/migrations/0003_ops.sql"),
    include_str!("../src/db/migrations/0004_content.sql"), include_str!("../src/db/migrations/0005_tokens.sql"), include_str!("../src/db/migrations/0006_modpacks.sql"),
];

fn free_port() -> u16 { TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port() }

/// xorshift64*: enough randomness for choosing operations, and reproducible from the printed seed.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 { self.0 ^= self.0 >> 12; self.0 ^= self.0 << 25; self.0 ^= self.0 >> 27; self.0.wrapping_mul(0x2545F4914F6CDD1D) }
    fn below(&mut self, n: usize) -> usize { (self.next() % n as u64) as usize }
}

struct Moss { child: Child, dir: PathBuf, log: PathBuf, port: u16, modrinth: u16 }
impl Moss {
    fn start(dir: &Path, log: &Path, port: u16, modrinth: u16) -> Self {
        let out = std::fs::OpenOptions::new().create(true).append(true).open(log).unwrap();
        let err = out.try_clone().unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_moss")).arg("serve")
            .env("MOSS_AUTH", "dev").env("MOSS_DEV_RUNTIMES", "true").env("MOSS_PORT", port.to_string()).env("MOSS_DATA_DIR", dir)
            .env("MOSS_MODRINTH_API_URL", format!("http://127.0.0.1:{modrinth}")).env("RUST_LOG", "moss=info").env("RUST_BACKTRACE", "1")
            .stdout(Stdio::from(out)).stderr(Stdio::from(err)).spawn().expect("spawn moss");
        Self { child, dir: dir.to_path_buf(), log: log.to_path_buf(), port, modrinth }
    }
    fn base(&self) -> String { format!("http://127.0.0.1:{}/api/v1", self.port) }
    fn pid(&self) -> u32 { self.child.id() }
    fn sigkill(&mut self) { let _ = self.child.kill(); let _ = self.child.wait(); }
    fn restart(&mut self) { self.sigkill(); *self = Self::start(&self.dir, &self.log, self.port, self.modrinth); }
}
impl Drop for Moss { fn drop(&mut self) { let _ = self.child.kill(); let _ = self.child.wait(); } }

/// Kills any game process we started when the test ends, so a failure never leaves processes behind.
struct Reaper { ports: BTreeSet<u16> }
impl Drop for Reaper { fn drop(&mut self) { for (pid, _) in sim_procs(&self.ports) { unsafe { libc::kill(pid as i32, libc::SIGKILL); } } } }

/// Running `sim-server` processes whose `--port` is one of ours: (pid, port). The API reports the pid of the wrapper
/// shell, which has the same arguments, so the shell and the game both show up here.
fn sim_procs(ports: &BTreeSet<u16>) -> Vec<(u32, u16)> {
    let mut v = vec![];
    let Ok(rd) = std::fs::read_dir("/proc") else { return v };
    for e in rd.flatten() {
        let Some(pid) = e.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
        let Ok(raw) = std::fs::read(format!("/proc/{pid}/cmdline")) else { continue };
        let args: Vec<String> = raw.split(|b| *b == 0).map(|a| String::from_utf8_lossy(a).into_owned()).collect();
        if !args.iter().any(|a| a == "sim-server") { continue }
        let Some(port) = args.iter().position(|a| a == "--port").and_then(|i| args.get(i + 1)).and_then(|p| p.parse::<u16>().ok()) else { continue };
        if ports.contains(&port) && !std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|s| s.contains(") Z ")) { v.push((pid, port)); }
    }
    v
}

fn ppid(pid: u32) -> Option<u32> {
    let s = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    s.rsplit_once(')')?.1.split_whitespace().nth(1)?.parse().ok()
}

fn rss_kb(pid: u32) -> u64 {
    std::fs::read_to_string(format!("/proc/{pid}/status")).ok().and_then(|s| s.lines().find(|l| l.starts_with("VmRSS:")).and_then(|l| l.split_whitespace().nth(1)?.parse().ok())).unwrap_or(0)
}
fn fd_count(pid: u32) -> usize { std::fs::read_dir(format!("/proc/{pid}/fd")).map(|d| d.count()).unwrap_or(0) }

async fn until<F: FnMut() -> Fut, Fut: std::future::Future<Output = Option<T>>, T>(what: &str, secs: u64, mut f: F) -> T {
    let end = Instant::now() + Duration::from_secs(secs);
    loop {
        if let Some(v) = f().await { return v; }
        assert!(Instant::now() < end, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

struct Api { c: reqwest::Client, base: String }
impl Api {
    async fn get(&self, path: &str) -> Option<Value> { self.c.get(format!("{}{path}", self.base)).send().await.ok()?.json().await.ok() }
    async fn post(&self, path: &str, body: Value) -> (u16, Value) {
        match self.c.post(format!("{}{path}", self.base)).json(&body).send().await {
            Ok(r) => { let s = r.status().as_u16(); (s, r.json().await.unwrap_or(Value::Null)) }
            Err(e) => panic!("POST {path} failed: {e}"),
        }
    }
    /// Waits for a server state; a timeout reports what the API says about the server.
    async fn wait_state(&self, id: &str, label: &str, secs: u64, why: &str) -> Value {
        let end = Instant::now() + Duration::from_secs(secs);
        loop {
            let s = self.server(id).await;
            if s["state"] == label { return s; }
            assert!(Instant::now() < end, "timed out waiting for server {id} to be {label} ({why}); API says: state={} detail={} desired={} pid={} since={}", s["state"], s["state_detail"], s["desired"], s["pid"], s["state_since"]);
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }
    async fn server(&self, id: &str) -> Value { self.get(&format!("/servers/{id}")).await.unwrap_or(Value::Null) }
    async fn wait_op(&self, id: &str) -> Value {
        until(&format!("operation {id} to finish"), 120, || async { let o = self.get(&format!("/operations/{id}")).await?; matches!(o["state"].as_str()?, "completed" | "failed" | "cancelled" | "interrupted").then_some(o) }).await
    }
    async fn ok_op(&self, what: &str, status_body: (u16, Value)) -> Value {
        let (s, b) = status_body;
        assert!(s < 300, "{what}: HTTP {s}: {b}");
        let id = b["operation"]["id"].as_str().unwrap_or_else(|| panic!("{what}: no operation in {b}")).to_string();
        let op = self.wait_op(&id).await;
        assert_eq!(op["state"], "completed", "{what} did not complete: {op}");
        op
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Want { Running, Stopped }

struct Soak {
    moss: Moss,
    api: Api,
    rng: Rng,
    dir: PathBuf,
    servers: Vec<(String, u16, Want)>,
    ports: BTreeSet<u16>,
    base_rss: Option<(u64, usize)>,
    stats: BTreeMap<&'static str, usize>,
    provider: Arc<ProviderStats>,
}

#[derive(Default)]
struct ProviderStats { ok: AtomicUsize, failed: AtomicUsize, hung: AtomicUsize }

impl Soak {
    fn count(&mut self, k: &'static str) { *self.stats.entry(k).or_default() += 1; }

    async fn healthz(&self) {
        let t = Instant::now();
        let r = self.api.c.get(format!("http://127.0.0.1:{}/healthz", self.moss.port)).timeout(Duration::from_secs(5)).send().await;
        assert!(r.as_ref().is_ok_and(|r| r.status().is_success()), "controller stopped answering /healthz: {r:?}");
        assert!(t.elapsed() < Duration::from_secs(3), "/healthz took {:?}", t.elapsed());
    }

    async fn wait_ready(&self) {
        until("controller to answer", 30, || async { self.api.c.get(format!("http://127.0.0.1:{}/healthz", self.moss.port)).send().await.ok().filter(|r| r.status().is_success()) }).await;
    }

    /// Every server should be in the state we last asked for, and the game processes must match the API exactly.
    async fn settle_and_check(&mut self) {
        for (id, _, want) in self.servers.clone() {
            let (label, secs) = match want { Want::Running => ("running", 45), Want::Stopped => ("stopped", 45) };
            self.api.wait_state(&id, label, secs, "settle check").await;
        }
        self.check_processes().await;
        self.check_ops().await;
        self.healthz().await;
        self.check_resources();
        let db = self.dir.join("moss.sqlite");
        let c = Connection::open_with_flags(&db, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let r: String = c.query_row("PRAGMA integrity_check", [], |r| r.get(0)).unwrap();
        assert_eq!(r, "ok", "database integrity_check");
        self.check_log();
    }

    async fn check_processes(&self) {
        let end = Instant::now() + Duration::from_secs(10);
        loop {
            let mut api_pids = BTreeSet::new();
            for (id, ..) in &self.servers { if let Some(p) = self.api.server(id).await["pid"].as_u64() { api_pids.insert(p as u32); } }
            let procs = sim_procs(&self.ports);
            // the API pid is the wrapper shell; the game is its child
            let orphans: Vec<_> = procs.iter().filter(|(p, _)| !api_pids.contains(p) && !ppid(*p).is_some_and(|pp| api_pids.contains(&pp))).collect();
            let ghosts: Vec<_> = api_pids.iter().filter(|p| !procs.iter().any(|(q, _)| q == *p)).collect();
            if orphans.is_empty() && ghosts.is_empty() { return; }
            assert!(Instant::now() < end, "orphaned game processes {orphans:?} / API pids without a process {ghosts:?} (API says {api_pids:?})");
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    }

    async fn check_ops(&self) {
        let ops = self.api.get("/operations").await.unwrap_or(Value::Null);
        for o in ops.as_array().into_iter().flatten() {
            if !matches!(o["state"].as_str(), Some("queued" | "running")) { continue }
            let age = o["created_at"].as_str().and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok()).map(|t| chrono::Utc::now().signed_duration_since(t).num_seconds()).unwrap_or(0);
            assert!(age < 180, "operation stuck for {age}s: {o}");
        }
    }

    fn check_resources(&mut self) {
        let (rss, fds) = (rss_kb(self.moss.pid()), fd_count(self.moss.pid()));
        match self.base_rss {
            None => self.base_rss = Some((rss, fds)),
            Some((r0, f0)) => {
                assert!(rss < r0 * 2 + 64 * 1024, "controller memory grew from {r0} kB to {rss} kB");
                assert!(fds < f0 + 120, "controller file descriptors grew from {f0} to {fds}");
            }
        }
    }

    fn check_log(&self) {
        let log = std::fs::read_to_string(&self.moss.log).unwrap_or_default();
        if let Some(l) = log.lines().find(|l| l.contains("panicked at")) { panic!("panic in the controller log: {l}"); }
    }

    // ---- operations -----------------------------------------------------------------------------------------

    async fn action(&mut self, i: usize, a: &'static str) {
        let id = self.servers[i].0.clone();
        if a == "start" { self.api.wait_state(&id, "stopped", 60, "before start").await; }
        let (s, b) = self.api.post(&format!("/servers/{id}/actions/{a}"), json!({})).await;
        assert!(s < 300, "{a} on {id}: HTTP {s}: {b}");
        self.servers[i].2 = if matches!(a, "start" | "restart") { Want::Running } else { Want::Stopped };
        self.count(a);
    }

    async fn spam(&mut self, i: usize) {
        let id = self.servers[i].0.clone();
        self.api.wait_state(&id, "running", 60, "before console spam").await;
        let mut bad = vec![];
        for chunk in 0..6 {
            let reqs = (0..50).map(|n| { let (c, url) = (self.api.c.clone(), format!("{}/servers/{id}/command", self.api.base)); async move {
                let r = c.post(url).json(&json!({"command": format!("say soak spam {chunk}-{n} {}", "x".repeat(200))})).send().await?;
                let s = r.status().as_u16();
                Ok::<_, reqwest::Error>((s, r.text().await.unwrap_or_default()))
            } });
            for r in futures_util::future::join_all(reqs).await { match r { Ok((204 | 429, _)) => {}, other => bad.push(format!("{other:?}")) } }
        }
        if !bad.is_empty() {
            let sv = self.api.server(&id).await;
            let procs = sim_procs(&self.ports);
            panic!("console spam on {id} got {} unexpected answers, first: {:?}; API says state={} detail={} pid={}; game processes {procs:?}", bad.len(), bad[0], sv["state"], sv["state_detail"], sv["pid"]);
        }
        let backlog = self.api.get(&format!("/servers/{id}/console/backlog?lines=100")).await.expect("backlog after spam");
        assert!(backlog["lines"].as_array().is_some_and(|l| !l.is_empty()), "console backlog empty after spam");
        self.count("console_spam");
    }

    async fn backup_and_verify(&mut self, i: usize) {
        let id = self.servers[i].0.clone();
        let b = self.api.post(&format!("/servers/{id}/backups"), json!({"note": "soak"})).await;
        self.api.ok_op("backup", b).await;
        let list = self.api.get(&format!("/servers/{id}/backups")).await.unwrap();
        let newest = list.as_array().unwrap().first().expect("a backup").clone();
        assert_eq!(newest["status"], "ok", "new backup: {newest}");
        let v = self.api.post(&format!("/backups/{}/verify", newest["id"].as_str().unwrap()), json!({})).await;
        self.api.ok_op("verify", v).await;
        self.count("backup_verify");
    }

    async fn restore(&mut self, i: usize) {
        let id = self.servers[i].0.clone();
        if self.servers[i].2 == Want::Running { self.action(i, "stop").await; }
        self.api.wait_state(&id, "stopped", 60, "before restore").await; // an earlier stop may still be finishing
        let mut list = self.api.get(&format!("/servers/{id}/backups")).await.unwrap();
        if list.as_array().is_none_or(|l| l.is_empty()) { self.backup_and_verify(i).await; list = self.api.get(&format!("/servers/{id}/backups")).await.unwrap(); }
        let ok: Vec<_> = list.as_array().unwrap().iter().filter(|b| b["status"] == "ok").collect();
        let pick = ok[self.rng.below(ok.len())]["id"].as_str().unwrap().to_string();
        let r = self.api.post(&format!("/backups/{pick}/restore"), json!({"mode": "in_place"})).await;
        self.api.ok_op("restore", r).await;
        self.count("restore");
    }

    /// Makes a schedule due by editing its next_run in the database; the scheduler must run it on its own.
    async fn schedule(&mut self, i: usize) {
        let id = self.servers[i].0.clone();
        let (s, sc) = self.api.post(&format!("/servers/{id}/schedules"), json!({"kind": "command", "command": "say scheduled soak", "every_minutes": 60})).await;
        assert_eq!(s, 201, "{sc}");
        let sid = sc["id"].as_str().unwrap().to_string();
        let was_running = self.servers[i].2 == Want::Running;
        {
            let c = Connection::open(self.dir.join("moss.sqlite")).unwrap();
            c.busy_timeout(Duration::from_secs(10)).unwrap();
            c.execute("UPDATE schedules SET next_run=? WHERE id=?", rusqlite::params![(chrono::Utc::now() - chrono::Duration::minutes(1)).to_rfc3339(), sid]).unwrap();
        }
        let last = until("the schedule to fire", 40, || async { let l = self.api.get(&format!("/servers/{id}/schedules")).await?; l.as_array()?.iter().find(|s| s["id"] == sid.as_str() && !s["last_run"].is_null()).cloned() }).await;
        assert!(last["last_status"].as_str().is_some(), "schedule ran without a status: {last}");
        let _ = was_running;
        let d = self.api.c.delete(format!("{}/schedules/{sid}", self.api.base)).send().await.unwrap();
        assert!(d.status().is_success(), "delete schedule");
        self.count("schedule_fired");
    }

    /// SIGKILL the controller; running games must survive and be re-attached by the new controller with the same pid.
    async fn kill_controller(&mut self) {
        let before: BTreeMap<String, u64> = {
            let mut m = BTreeMap::new();
            for (id, ..) in &self.servers { let s = self.api.server(id).await; if s["state"] == "running" { m.insert(id.clone(), s["pid"].as_u64().unwrap_or(0)); } }
            m
        };
        self.moss.sigkill();
        for (id, port, _) in &self.servers {
            if before.contains_key(id) { assert!(std::net::TcpStream::connect_timeout(&format!("127.0.0.1:{port}").parse().unwrap(), Duration::from_secs(2)).is_ok(), "game {id} died with the controller"); }
        }
        self.moss.restart();
        self.wait_ready().await;
        for (id, pid) in before {
            let s = self.api.wait_state(&id, "running", 60, "re-attach after controller SIGKILL").await;
            assert_eq!(s["pid"].as_u64(), Some(pid), "controller must re-attach to the original process of {id}");
        }
        // a restart or stop that the old controller never finished must be completed by the new one
        for (id, _, w) in self.servers.clone() { self.api.wait_state(&id, if w == Want::Running { "running" } else { "stopped" }, 90, "after controller SIGKILL").await; }
        self.base_rss = None; // a fresh process has a fresh baseline
        self.count("controller_sigkill");
    }

    async fn step(&mut self, n: usize) {
        let i = self.rng.below(self.servers.len());
        let want = self.servers[i].2;
        let roll = self.rng.below(100);
        match (roll, want) {
            (0..=19, Want::Stopped) => self.action(i, "start").await,
            (0..=9, Want::Running) => self.action(i, "stop").await,
            (10..=19, Want::Running) => self.action(i, "restart").await,
            (20..=23, Want::Running) => self.action(i, "kill").await,
            (24..=39, Want::Running) => self.spam(i).await,
            (40..=54, _) => self.backup_and_verify(i).await,
            (55..=62, _) => self.restore(i).await,
            (63..=72, _) => self.schedule(i).await,
            (73..=80, _) => self.kill_controller().await,
            _ => self.count("idle"),
        }
        println!("soak step {n}: server {i} roll {roll}");
        self.healthz().await;
        if n % 6 == 5 { self.settle_and_check().await; }
    }
}

/// A Modrinth stand-in that cycles through failure modes, driven by a mode flag.
async fn provider_stub(listener: tokio::net::TcpListener, mode: Arc<AtomicU8>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    loop {
        let Ok((mut sock, _)) = listener.accept().await else { return };
        let m = mode.load(Ordering::Relaxed);
        tokio::spawn(async move {
            let mut buf = [0u8; 2048];
            let _ = tokio::time::timeout(Duration::from_secs(2), sock.read(&mut buf)).await;
            let reply = |status: &str, extra: &str, body: &str| format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n{body}", body.len());
            match m {
                0 => { let _ = sock.write_all(reply("200 OK", "", r#"{"hits":[],"total_hits":0}"#).as_bytes()).await; }
                1 => { let _ = sock.write_all(reply("503 Service Unavailable", "", "{}").as_bytes()).await; }
                2 => { let _ = sock.write_all(reply("429 Too Many Requests", "Retry-After: 2\r\n", "{}").as_bytes()).await; }
                3 => { tokio::time::sleep(Duration::from_secs(60)).await; } // hang: never answer
                _ => { let _ = sock.write_all(b"garbage that is not http").await; } // protocol error, then close
            }
        });
    }
}

fn seed_old_release(dir: &Path) {
    std::fs::create_dir_all(dir.join("servers/s1")).unwrap();
    std::fs::write(dir.join("servers/s1/server.properties"), "motd=hi\n").unwrap();
    let c = Connection::open(dir.join("moss.sqlite")).unwrap();
    for (i, sql) in OLD.iter().enumerate() { c.execute_batch(&format!("BEGIN;\n{sql}\nPRAGMA user_version = {};\nCOMMIT;", i + 1)).unwrap(); }
    for q in [
        // a paper server that is never started: it only exists so content searches have a server to ask about
        "INSERT INTO servers(id,name,runtime,game_version,port,memory_mb,created_at,installed) VALUES('s1','Old survival','paper','1.21.1',25570,2048,'2026-01-01T00:00:00Z',1)",
        "INSERT INTO backups(id,server_id,created_at,size,sha256,status,kind,server_name,locked,worlds) VALUES('b1','s1','2026-01-01T00:00:00Z',10,'abc','ok','manual','Old survival',1,'[\"world\"]')",
        "INSERT INTO schedules(id,server_id,kind,every_minutes,tz) VALUES('sc1','s1','backup',60,'UTC')",
        "INSERT INTO meta(k,v) VALUES('backup_keep','4')",
    ] { c.execute(q, []).unwrap(); }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "soak test: run with --ignored (MOSS_SOAK_MINUTES sets the duration)"]
async fn soak() {
    let minutes: f64 = std::env::var("MOSS_SOAK_MINUTES").ok().and_then(|v| v.parse().ok()).unwrap_or(2.0);
    let seed: u64 = std::env::var("MOSS_SOAK_SEED").ok().and_then(|v| v.parse().ok()).unwrap_or_else(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as u64 | 1);
    let root = std::env::var("MOSS_SOAK_DIR").map(PathBuf::from).unwrap_or_else(|_| std::env::temp_dir().join(format!("moss-soak-{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&root);
    let data = root.join("data");
    std::fs::create_dir_all(&data).unwrap();
    let log = root.join("controller.log");
    println!("soak: {minutes} min, seed {seed}, dir {}", root.display());

    seed_old_release(&data);
    let provider_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let provider_port = provider_listener.local_addr().unwrap().port();
    let mode = Arc::new(AtomicU8::new(0));
    tokio::spawn(provider_stub(provider_listener, mode.clone()));

    let port = free_port();
    let ports: BTreeSet<u16> = (0..3).map(|_| free_port()).collect();
    let _reaper = Reaper { ports: ports.clone() };
    let moss = Moss::start(&data, &log, port, provider_port);
    let api = Api { c: reqwest::Client::builder().timeout(Duration::from_secs(60)).build().unwrap(), base: moss.base() };
    let mut soak = Soak { moss, api, rng: Rng(seed), dir: data.clone(), servers: vec![], ports: ports.clone(), base_rss: None, stats: BTreeMap::new(), provider: Arc::new(ProviderStats::default()) };
    soak.wait_ready().await;

    // The upgrade: the old data must be served, and the schema brought up to date.
    let old = soak.api.get("/servers").await.unwrap();
    assert_eq!(old[0]["name"], "Old survival");
    assert_eq!(soak.api.get("/backups").await.unwrap()[0]["id"], "b1");
    let v: i64 = Connection::open_with_flags(data.join("moss.sqlite"), OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap().query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
    assert!(v > 6, "schema version after upgrade: {v}");

    for (n, port) in ports.iter().enumerate() {
        let (s, b) = soak.api.post("/servers", json!({"name": format!("soak{n}"), "runtime": "sim", "game_version": "sim-1.0", "memory_mb": 512, "port": port})).await;
        assert!(s < 300, "{b}");
        soak.api.ok_op("install", (s, b.clone())).await;
        soak.servers.push((b["server"]["id"].as_str().unwrap().to_string(), *port, Want::Stopped));
    }

    // Provider outages: the mode changes every few seconds while content searches keep coming.
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let ps = soak.provider.clone();
    let (c2, base2, stop2, mode2) = (reqwest::Client::builder().timeout(Duration::from_secs(4)).build().unwrap(), soak.api.base.clone(), stop.clone(), mode.clone());
    let probe = tokio::spawn(async move {
        let mut n = 0u64;
        while !stop2.load(Ordering::Relaxed) {
            mode2.store(((n / 4) % 5) as u8, Ordering::Relaxed);
            match c2.get(format!("{base2}/content/search?server_id=s1&q=soak{n}")).send().await {
                Ok(r) if r.status().is_success() => { ps.ok.fetch_add(1, Ordering::Relaxed); }
                Ok(r) => { assert!(r.status().as_u16() < 600); ps.failed.fetch_add(1, Ordering::Relaxed); }
                Err(e) if e.is_timeout() => { ps.hung.fetch_add(1, Ordering::Relaxed); }
                Err(_) => { ps.failed.fetch_add(1, Ordering::Relaxed); } // the controller restarting under a SIGKILL
            }
            n += 1;
            tokio::time::sleep(Duration::from_millis(700)).await;
        }
    });

    let deadline = Instant::now() + Duration::from_secs_f64(minutes * 60.0);
    // warm up: everything running so the first checks have something to compare
    for i in 0..soak.servers.len() { soak.action(i, "start").await; }
    soak.settle_and_check().await;
    let mut n = 0;
    while Instant::now() < deadline {
        soak.step(n).await;
        n += 1;
    }

    // wind down: stop everything, then every backup must verify and nothing may be left over
    stop.store(true, Ordering::Relaxed);
    let _ = probe.await;
    soak.settle_and_check().await;
    for i in 0..soak.servers.len() { if soak.servers[i].2 == Want::Running { soak.action(i, "stop").await; } }
    soak.settle_and_check().await;
    for (id, ..) in soak.servers.clone() {
        let list = soak.api.get(&format!("/servers/{id}/backups")).await.unwrap();
        for b in list.as_array().unwrap() {
            assert_eq!(b["status"], "ok", "backup {b}");
            let v = soak.api.post(&format!("/backups/{}/verify", b["id"].as_str().unwrap()), json!({})).await;
            soak.api.ok_op("final verify", v).await;
        }
    }
    assert!(sim_procs(&ports).is_empty(), "game processes left after stopping everything");
    soak.check_log();
    println!("soak ok after {n} steps: {:?}; provider searches ok={} failed={} hung={}; controller rss {} kB, {} fds",
        soak.stats, soak.provider.ok.load(Ordering::Relaxed), soak.provider.failed.load(Ordering::Relaxed), soak.provider.hung.load(Ordering::Relaxed), rss_kb(soak.moss.pid()), fd_count(soak.moss.pid()));
    drop(soak);
    if std::env::var_os("MOSS_SOAK_KEEP").is_none() { let _ = std::fs::remove_dir_all(&root); }
}
