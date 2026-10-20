//! Console flood spike: how the tailer and the viewer fan-out behave under extreme output. The big runs are
//! `#[ignore]`d (they write hundreds of MB); numbers are in docs/spikes/console-flood.md.
//! `MOSS_FLOOD_MB=200 cargo test --release console::flood -- --ignored --nocapture --test-threads=1`
use super::*;
use crate::testutil;
use std::{io::Write, time::{Duration, Instant}};

fn mb() -> usize { std::env::var("MOSS_FLOOD_MB").ok().and_then(|v| v.parse().ok()).unwrap_or(200) }

/// (peak resident bytes, process CPU seconds) of this process.
fn proc_stats() -> (u64, f64) {
    let hwm = std::fs::read_to_string("/proc/self/status").ok().and_then(|s| s.lines().find(|l| l.starts_with("VmHWM:")).and_then(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok())).unwrap_or(0) * 1024;
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) };
    let tv = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 / 1e6;
    (hwm, tv(ru.ru_utime) + tv(ru.ru_stime))
}

fn ring_bytes(h: &Hub) -> (usize, usize) { let r = h.ring.lock().unwrap(); (r.1.len(), r.1.iter().map(|(_, l)| l.len()).sum()) }
fn seq(h: &Hub) -> u64 { h.ring.lock().unwrap().0 }

/// Starts a tailer on a fresh log with the server marked alive; returns the log path and hub.
async fn rig() -> (crate::state::St, std::path::PathBuf, Arc<Hub>) {
    let st = testutil::state(|_| {}).await;
    let log = st.cfg.data_dir.join("console.log");
    std::fs::write(&log, b"").unwrap();
    st.live.lock().unwrap().entry("flood".into()).or_default().alive = true;
    start_tailer(&st, "flood".into(), log.clone(), false);
    let h = hub(&st, "flood");
    (st, log, h)
}

async fn until_seq(h: &Hub, n: u64, secs: u64) -> bool {
    let end = Instant::now() + Duration::from_secs(secs);
    while seq(h) < n { if Instant::now() > end { return false; } tokio::time::sleep(Duration::from_millis(5)).await; }
    true
}

/// Append `total` bytes made of `line` (newline added) to the log in 1 MiB writes, from a blocking thread.
async fn write_flood(log: std::path::PathBuf, line: String, total: usize) -> usize {
    tokio::task::spawn_blocking(move || {
        let mut f = std::fs::OpenOptions::new().append(true).open(log).unwrap();
        let unit = format!("{line}\n");
        let block = unit.repeat((1 << 20) / unit.len().max(1) + 1);
        let (mut written, mut lines) = (0, 0);
        while written < total { f.write_all(block.as_bytes()).unwrap(); written += block.len(); lines += block.len() / unit.len(); }
        lines
    }).await.unwrap()
}

async fn flood(label: &str, line: String, total: usize) {
    let (st, log, h) = rig().await;
    let (rss0, cpu0) = proc_stats();
    let t = Instant::now();
    let lines = write_flood(log.clone(), line, total).await;
    let write_s = t.elapsed().as_secs_f64();
    let drained = until_seq(&h, lines as u64, 600).await;
    let total_s = t.elapsed().as_secs_f64();
    let (rss1, cpu1) = proc_stats();
    let (n, bytes) = ring_bytes(&h);
    // latency of one marker line once the backlog is gone
    let before = seq(&h);
    let m = Instant::now();
    std::fs::OpenOptions::new().append(true).open(&log).unwrap().write_all(b"marker\n").unwrap();
    assert!(until_seq(&h, before + 1, 5).await, "marker line must appear");
    let lat = m.elapsed();
    println!("FLOOD {label}: {:.0} MB, {lines} lines; write {write_s:.1}s, tailer done {total_s:.1}s ({:.0} MB/s), cpu {:.1}s, peak RSS +{} MB, ring {n} lines / {} KB, marker latency {} ms",
        total as f64 / 1e6, total as f64 / 1e6 / total_s, cpu1 - cpu0, rss1.saturating_sub(rss0) >> 20, bytes >> 10, lat.as_millis());
    assert!(drained, "{label}: tailer did not keep up");
    assert!(n <= RING && bytes <= RING * (lines::MAX_LINE + 64), "{label}: ring must stay bounded");
    drop(st);
}

#[tokio::test(flavor = "multi_thread")]
async fn tailer_handles_a_moderate_flood_with_bounded_memory() { flood("small-lines-32MB", "[12:00:00 INFO]: spam spam spam spam spam spam spam".into(), 32 << 20).await; }

#[tokio::test(flavor = "multi_thread")]
#[ignore = "writes hundreds of MB; run in release"]
async fn flood_short_lines() { flood("short-lines", "[12:00:00 INFO]: <Steve> hello hello hello hello hello".into(), mb() << 20).await; }

#[tokio::test(flavor = "multi_thread")]
#[ignore = "writes hundreds of MB; run in release"]
async fn flood_huge_lines() { flood("1MB-lines", "y".repeat(1 << 20), mb() << 20).await; }

#[tokio::test(flavor = "multi_thread")]
#[ignore = "writes hundreds of MB; run in release"]
async fn flood_one_endless_line() {
    let (_st, log, h) = rig().await;
    let (rss0, _) = proc_stats();
    let l = log.clone();
    tokio::task::spawn_blocking(move || { let mut f = std::fs::OpenOptions::new().append(true).open(l).unwrap(); let b = vec![b'z'; 1 << 20]; for _ in 0..mb() { f.write_all(&b).unwrap(); } f.write_all(b"\nend\n").unwrap(); }).await.unwrap();
    assert!(until_seq(&h, 2, 600).await);
    let (rss1, _) = proc_stats();
    println!("FLOOD endless-line: {} MB without a newline, peak RSS +{} MB", mb(), rss1.saturating_sub(rss0) >> 20);
    assert!(ring_bytes(&h).1 < 2 * lines::MAX_LINE + 256);
}

/// A sink that accepts frames with a fixed delay (or never).
struct Slow { delay: Option<Duration>, got: usize }
impl Sink for Slow {
    async fn send(&mut self, _t: String) -> bool {
        match self.delay { Some(d) => { tokio::time::sleep(d).await; self.got += 1; true } None => std::future::pending().await }
    }
}

#[tokio::test]
async fn a_stalled_viewer_is_dropped_after_the_send_timeout_and_does_not_hold_memory() {
    let (tx, mut rx) = broadcast::channel::<(u64, String)>(512);
    let mut sink = Slow { delay: None, got: 0 };
    let mut lags = Default::default();
    tx.send((1, "a line".into())).unwrap();
    let t = Instant::now();
    let keep = relay(rx.recv().await, 0, &mut sink, &mut lags, Duration::from_millis(150)).await;
    assert!(!keep && t.elapsed() < Duration::from_secs(2), "a stalled viewer is cut loose");
    // the channel itself never grows past its capacity however much is written while a receiver is stuck
    for i in 0..100_000u64 { let _ = tx.send((i, "x".repeat(100))); }
    assert!(tx.len() <= 512);
}

#[tokio::test]
async fn a_slow_viewer_is_told_about_skipped_lines_then_dropped_if_it_keeps_falling_behind() {
    let (tx, mut rx) = broadcast::channel::<(u64, String)>(8);
    let mut sink = Slow { delay: Some(Duration::from_millis(1)), got: 0 };
    let mut lags = Default::default();
    let mut keep = true;
    for round in 0..10 {
        for i in 0..64u64 { let _ = tx.send((round * 100 + i, "l".into())); } // overrun the 8-slot buffer
        keep = relay(rx.recv().await, 0, &mut sink, &mut lags, Duration::from_secs(1)).await;
        if !keep { assert!(round >= 4, "dropped only after repeated lag, not the first time"); break; }
    }
    assert!(!keep, "a viewer that keeps lagging is disconnected");
    assert!(sink.got >= 4, "it was told about the skipped lines first");
}

#[tokio::test]
async fn a_fast_viewer_gets_every_line_while_the_slow_one_is_dropped() {
    let (tx, mut fast) = broadcast::channel::<(u64, String)>(512);
    let mut sink = Slow { delay: Some(Duration::ZERO), got: 0 };
    let mut lags = Default::default();
    for i in 1..=300u64 { tx.send((i, "x".into())).unwrap(); }
    for _ in 0..300 { assert!(relay(fast.recv().await, 0, &mut sink, &mut lags, Duration::from_secs(1)).await); }
    assert_eq!(sink.got, 300);
}
