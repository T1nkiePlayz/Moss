//! Performance gates: runs the real binary with simulated game servers and compares what it measures with
//! `perf-budget.json`. Ignored by default (it takes about half a minute and the numbers only mean something for a
//! release build); run it with
//!
//!   cargo test --release --test perf -- --ignored --nocapture
//!
//! Measured: time from spawn until /healthz answers (startup), controller resident memory after ten idle seconds with two
//! simulated servers running, and, while one simulated server is flooded with console commands at a fixed rate
//! (`flood_commands_per_s`), the console lines per
//! second delivered over a WebSocket, the activity events per second delivered over the SSE stream, the fraction of
//! console lines that arrived (the bounded broadcast buffer may drop lines for a slow viewer, a fast one must see them
//! all) and the controller's CPU use. A Markdown table of measured against budget goes to stdout and, when set, to
//! $GITHUB_STEP_SUMMARY. Any value over budget fails the test. Linux only: memory and CPU come from /proc.
#![cfg(target_os = "linux")]
mod suite;
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::{
    sync::{Arc, atomic::{AtomicBool, AtomicUsize, Ordering}},
    time::{Duration, Instant},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const IDLE: Duration = Duration::from_secs(10);
const FLOOD: Duration = Duration::from_secs(5);

fn budget() -> Value { serde_json::from_str(include_str!("../perf-budget.json")).expect("perf-budget.json") }

fn rss_kb(pid: u32) -> u64 {
    std::fs::read_to_string(format!("/proc/{pid}/status")).ok().and_then(|s| s.lines().find(|l| l.starts_with("VmRSS:")).and_then(|l| l.split_whitespace().nth(1)?.parse().ok())).unwrap_or(0)
}

/// User plus system CPU time of the process so far, in seconds.
fn cpu_seconds(pid: u32) -> f64 {
    let s = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
    let f: Vec<&str> = s.rsplit_once(')').map(|r| r.1.split_whitespace().collect()).unwrap_or_default();
    let ticks: u64 = [11, 12].iter().filter_map(|i| f.get(*i)?.parse::<u64>().ok()).sum();
    ticks as f64 / unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as f64
}

async fn ticket(c: &reqwest::Client, base: &str) -> String {
    c.post(format!("{base}/ticket")).send().await.unwrap().json::<Value>().await.unwrap()["ticket"].as_str().unwrap().to_string()
}

/// The console WebSocket, spoken by hand (the crate has no WebSocket client): the upgrade request, then server frames,
/// which are never masked. Counts the lines that carry `token`.
async fn watch_console(port: u16, id: String, ticket: String, stop: Arc<AtomicBool>, token: &'static str, lines: Arc<AtomicUsize>) {
    let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let req = format!("GET /api/v1/servers/{id}/console?ticket={ticket} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n");
    s.write_all(req.as_bytes()).await.unwrap();
    let mut buf: Vec<u8> = vec![];
    let mut chunk = vec![0u8; 64 * 1024];
    let mut upgraded = false;
    while !stop.load(Ordering::Relaxed) {
        let Ok(Ok(n)) = tokio::time::timeout(Duration::from_millis(200), s.read(&mut chunk)).await else { continue };
        if n == 0 { return; }
        buf.extend_from_slice(&chunk[..n]);
        if !upgraded {
            let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") else { continue };
            assert!(buf.starts_with(b"HTTP/1.1 101"), "console upgrade refused: {}", String::from_utf8_lossy(&buf[..end]));
            buf.drain(..end + 4);
            upgraded = true;
        }
        while buf.len() >= 2 {
            let (opcode, mut len, mut at) = (buf[0] & 0x0f, (buf[1] & 0x7f) as usize, 2);
            if len == 126 { if buf.len() < 4 { break } len = u16::from_be_bytes([buf[2], buf[3]]) as usize; at = 4; }
            else if len == 127 { if buf.len() < 10 { break } len = u64::from_be_bytes(buf[2..10].try_into().unwrap()) as usize; at = 10; }
            if buf.len() < at + len { break }
            if opcode == 1 && String::from_utf8_lossy(&buf[at..at + len]).contains(token) { lines.fetch_add(1, Ordering::Relaxed); }
            buf.drain(..at + len);
        }
    }
}

/// The SSE stream: counts `activity` events (every console command the controller accepts is logged as one).
async fn watch_events(c: reqwest::Client, base: String, ticket: String, stop: Arc<AtomicBool>, events: Arc<AtomicUsize>) {
    let r = c.get(format!("{base}/events?ticket={ticket}")).send().await.unwrap();
    assert!(r.status().is_success(), "event stream refused: {}", r.status());
    let mut body = r.bytes_stream();
    let mut pending = String::new();
    while !stop.load(Ordering::Relaxed) {
        let Ok(Some(Ok(b))) = tokio::time::timeout(Duration::from_millis(200), body.next()).await else { continue };
        pending.push_str(&String::from_utf8_lossy(&b));
        if let Some(end) = pending.rfind("\n\n") {
            events.fetch_add(pending[..end].matches("event: activity").count(), Ordering::Relaxed);
            pending.drain(..end + 2);
        }
    }
}

struct Row { name: &'static str, measured: f64, limit: f64, unit: &'static str, max: bool }
impl Row { fn ok(&self) -> bool { if self.max { self.measured <= self.limit } else { self.measured >= self.limit } } }

fn report(rows: &[Row]) {
    let mut t = String::from("### Performance gates\n\n| Measure | Measured | Budget | Result |\n| --- | --- | --- | --- |\n");
    for r in rows {
        t.push_str(&format!("| {} | {:.2} {} | {} {:.2} {} | {} |\n", r.name, r.measured, r.unit, if r.max { "at most" } else { "at least" }, r.limit, r.unit, if r.ok() { "ok" } else { "OVER BUDGET" }));
    }
    println!("{t}");
    if let Some(p) = std::env::var_os("GITHUB_STEP_SUMMARY") {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().append(true).create(true).open(p) { let _ = writeln!(f, "{t}"); }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "performance gates: run with --ignored against a release build"]
async fn perf_budget() {
    let b = budget();
    let limit = |k: &str| b[k].as_f64().unwrap_or_else(|| panic!("perf-budget.json: {k}"));
    let dir = std::env::temp_dir().join(format!("moss-perf-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let port = suite::free_port();

    // startup
    let c = reqwest::Client::builder().timeout(Duration::from_secs(30)).build().unwrap();
    let started = Instant::now();
    let moss = suite::Moss::start(&dir, port, &[]);
    let pid = moss.child.id();
    let probe = reqwest::Client::builder().timeout(Duration::from_millis(500)).build().unwrap();
    let startup_ms = loop {
        if probe.get(format!("http://127.0.0.1:{port}/healthz")).send().await.is_ok_and(|r| r.status().is_success()) { break started.elapsed().as_secs_f64() * 1000.0; }
        assert!(started.elapsed() < Duration::from_secs(30), "controller never answered /healthz");
        tokio::time::sleep(Duration::from_millis(5)).await;
    };

    // two simulated servers, running
    let mut ids = vec![];
    for n in 0..2 {
        let r = c.post(format!("{}/servers", moss.base)).json(&json!({"name": format!("perf{n}"), "runtime": "sim", "game_version": "sim-1.0", "memory_mb": 512, "port": suite::free_port()})).send().await.unwrap();
        assert!(r.status().is_success(), "create server: {}", r.status());
        let body: Value = r.json().await.unwrap();
        assert_eq!(suite::wait_op(&c, &moss.base, body["operation"]["id"].as_str().unwrap()).await["state"], "completed");
        let id = body["server"]["id"].as_str().unwrap().to_string();
        assert!(c.post(format!("{}/servers/{id}/actions/start", moss.base)).send().await.unwrap().status().is_success());
        ids.push(id);
    }
    for id in &ids {
        suite::until("a simulated server to run", 60, || async { let s: Value = c.get(format!("{}/servers/{id}", moss.base)).send().await.ok()?.json().await.ok()?; (s["state"] == "running").then_some(()) }).await;
    }

    // idle memory
    tokio::time::sleep(IDLE).await;
    let idle_rss_mb = rss_kb(pid) as f64 / 1024.0;

    // event stream: flood one server with console commands while watching the console and the event stream
    const TOKEN: &str = "perfflood";
    let (stop, lines, events) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let target = ids[0].clone();
    let ws = tokio::spawn(watch_console(port, target.clone(), ticket(&c, &moss.base).await, stop.clone(), TOKEN, lines.clone()));
    let sse = tokio::spawn(watch_events(c.clone(), moss.base.clone(), ticket(&c, &moss.base).await, stop.clone(), events.clone()));
    tokio::time::sleep(Duration::from_millis(500)).await; // both are subscribed before the first command
    let (accepted, flooding) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicBool::new(true)));
    // Paced at a fixed rate: unpaced, a fast runner produces far more than a viewer is meant to keep up with, and the
    // broadcast buffer then skips lines and disconnects it by design (docs/spikes/console-flood.md), so the ratio
    // would measure the runner, not Moss.
    const SENDERS: u32 = 4;
    let every = Duration::from_secs_f64(f64::from(SENDERS) / limit("flood_commands_per_s"));
    let (cpu0, t0) = (cpu_seconds(pid), Instant::now());
    let senders: Vec<_> = (0..SENDERS).map(|w| {
        let (c, url, accepted, flooding) = (c.clone(), format!("{}/servers/{target}/command", moss.base), accepted.clone(), flooding.clone());
        tokio::spawn(async move {
            let mut n = 0u64;
            let mut tick = tokio::time::interval(every);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            while flooding.load(Ordering::Relaxed) {
                tick.tick().await;
                match c.post(&url).json(&json!({"command": format!("say {TOKEN} {w}-{n} {}", "x".repeat(100))})).send().await.map(|r| r.status().as_u16()) {
                    Ok(204) => { accepted.fetch_add(1, Ordering::Relaxed); }
                    Ok(429) => tokio::time::sleep(Duration::from_millis(2)).await, // the game's console pipe is full: by design
                    other => panic!("console command: {other:?}"),
                }
                n += 1;
            }
        })
    }).collect();
    tokio::time::sleep(FLOOD).await;
    flooding.store(false, Ordering::Relaxed);
    for s in senders { s.await.unwrap(); }
    let secs = t0.elapsed().as_secs_f64();
    let cpu_pct = (cpu_seconds(pid) - cpu0) / secs * 100.0;
    // Let the tailer and the viewers drain. Every accepted command shows as an echo line and as the game's output.
    let accepted = accepted.load(Ordering::Relaxed);
    let expect = accepted * 2;
    let end = Instant::now() + Duration::from_secs(10);
    while lines.load(Ordering::Relaxed) < expect && Instant::now() < end { tokio::time::sleep(Duration::from_millis(100)).await; }
    stop.store(true, Ordering::Relaxed);
    let _ = (ws.await, sse.await);
    let (got, evs) = (lines.load(Ordering::Relaxed), events.load(Ordering::Relaxed));
    println!("perf: {accepted} commands accepted in {secs:.1}s, {got} of {expect} console lines delivered, {evs} activity events, controller cpu {cpu_pct:.0}%");
    assert!(accepted > 0, "no console command was accepted");

    let rows = [
        Row { name: "Startup to /healthz", measured: startup_ms, limit: limit("startup_ms"), unit: "ms", max: true },
        Row { name: "Idle RSS (2 servers running)", measured: idle_rss_mb, limit: limit("idle_rss_mb"), unit: "MB", max: true },
        Row { name: "Console lines delivered", measured: got as f64 / secs, limit: limit("min_console_lines_per_s"), unit: "lines/s", max: false },
        Row { name: "SSE events delivered", measured: evs as f64 / secs, limit: limit("min_sse_events_per_s"), unit: "events/s", max: false },
        Row { name: "Console lines delivered / produced", measured: got as f64 / expect as f64, limit: limit("min_console_delivery_ratio"), unit: "", max: false },
        Row { name: "Controller CPU during the flood", measured: cpu_pct, limit: limit("max_flood_cpu_pct"), unit: "%", max: true },
    ];
    report(&rows);
    drop(moss);
    let over: Vec<_> = rows.iter().filter(|r| !r.ok()).map(|r| r.name).collect();
    assert!(over.is_empty(), "over budget: {over:?}");
}
