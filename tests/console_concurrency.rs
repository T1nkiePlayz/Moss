//! Many admins (or workflows) typing into one server's console at once: every POST /command must answer promptly,
//! with 204 or, when the game's console pipe is full, 429. None may hang. On a busy disk this once stalled every
//! sender for over 10 s after about 16 commands each: SQLite's automatic WAL checkpoint (an fsync) ran on the one
//! shared connection inside a command's audit write, and every database call waited behind it.
#![cfg(target_os = "linux")]
mod suite;
use serde_json::{Value, json};
use std::{sync::{Arc, Mutex}, time::{Duration, Instant}};

const SENDERS: usize = 16;
const PER_SENDER: u32 = 200;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_console_commands_never_hang() {
    let dir = std::env::temp_dir().join(format!("moss-cmdflood-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let port = suite::free_port();
    let moss = suite::Moss::start(&dir, port, &[]);
    let c = reqwest::Client::builder().timeout(Duration::from_secs(10)).build().unwrap();
    suite::until("the controller to answer", 30, || async { c.get(format!("http://127.0.0.1:{port}/healthz")).send().await.ok().filter(|r| r.status().is_success()).map(|_| ()) }).await;
    let r = c.post(format!("{}/servers", moss.base)).json(&json!({"name": "flood", "runtime": "sim", "game_version": "sim-1.0", "memory_mb": 512, "port": suite::free_port()})).send().await.unwrap();
    assert!(r.status().is_success(), "create server: {}", r.status());
    let body: Value = r.json().await.unwrap();
    assert_eq!(suite::wait_op(&c, &moss.base, body["operation"]["id"].as_str().unwrap()).await["state"], "completed");
    let id = body["server"]["id"].as_str().unwrap().to_string();
    assert!(c.post(format!("{}/servers/{id}/actions/start", moss.base)).send().await.unwrap().status().is_success());
    suite::until("the simulated server to run", 60, || async { let s: Value = c.get(format!("{}/servers/{id}", moss.base)).send().await.ok()?.json().await.ok()?; (s["state"] == "running").then_some(()) }).await;

    let url = format!("{}/servers/{id}/command", moss.base);
    let worst = Arc::new(Mutex::new(Duration::ZERO));
    let senders: Vec<_> = (0..SENDERS).map(|w| {
        let (c, url, worst) = (c.clone(), url.clone(), worst.clone());
        tokio::spawn(async move {
            let (mut accepted, mut n) = (0u32, 0u32);
            while accepted < PER_SENDER {
                let t = Instant::now();
                let r = c.post(&url).json(&json!({"command": format!("say flood {w}-{n} {}", "x".repeat(100))})).send().await;
                let took = t.elapsed();
                { let mut m = worst.lock().unwrap(); *m = (*m).max(took); }
                match r.map(|r| r.status().as_u16()) {
                    Ok(204) => accepted += 1,
                    Ok(429) => tokio::time::sleep(Duration::from_millis(2)).await, // the game's console pipe is full: by design
                    other => panic!("sender {w}, command {n}: {other:?} after {took:?}"),
                }
                n += 1;
            }
            accepted
        })
    }).collect();
    let mut accepted = 0;
    for s in senders { accepted += s.await.unwrap(); }
    println!("{SENDERS} senders: {accepted} commands accepted, slowest answer {:?}", worst.lock().unwrap());
    assert_eq!(accepted, SENDERS as u32 * PER_SENDER);
    assert!(c.get(format!("{}/servers/{id}", moss.base)).send().await.unwrap().status().is_success(), "the controller still answers");
}
