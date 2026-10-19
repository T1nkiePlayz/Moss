//! Lightweight host and per-process metrics read straight from /proc (no heavy dependencies).
use serde_json::{Value, json};
use std::{fs, path::Path, time::Instant};

fn clk_tck() -> f64 { unsafe { libc::sysconf(libc::_SC_CLK_TCK) as f64 } }
fn page_size() -> u64 { unsafe { libc::sysconf(libc::_SC_PAGESIZE) as u64 } }

#[derive(Default)]
pub struct HostSampler {
    prev: Option<(u64, u64, Instant)>,
    pub cpu_pct: f64,
}

impl HostSampler {
    pub fn sample(&mut self) {
        let Ok(s) = fs::read_to_string("/proc/stat") else { return };
        let Some(line) = s.lines().next() else { return };
        let v: Vec<u64> = line.split_whitespace().skip(1).filter_map(|x| x.parse().ok()).collect();
        if v.len() < 5 { return; }
        let idle = v[3] + v.get(4).copied().unwrap_or(0);
        let total: u64 = v.iter().sum();
        if let Some((pi, pt, _)) = self.prev {
            let dt = total.saturating_sub(pt) as f64;
            if dt > 0.0 { self.cpu_pct = (100.0 * (1.0 - (idle.saturating_sub(pi)) as f64 / dt)).clamp(0.0, 100.0); }
        }
        self.prev = Some((idle, total, Instant::now()));
    }
}

pub fn host_json(cpu_pct: f64, data_dir: &Path) -> Value {
    let mem = fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let kb = |k: &str| mem.lines().find(|l| l.starts_with(k)).and_then(|l| l.split_whitespace().nth(1)).and_then(|x| x.parse::<u64>().ok()).unwrap_or(0) * 1024;
    let total = kb("MemTotal:");
    let avail = kb("MemAvailable:");
    let (dt, df) = disk(data_dir);
    let load1 = fs::read_to_string("/proc/loadavg").ok().and_then(|s| s.split_whitespace().next().and_then(|x| x.parse::<f64>().ok())).unwrap_or(0.0);
    let hostname = fs::read_to_string("/proc/sys/kernel/hostname").unwrap_or_default().trim().to_string();
    json!({"cpu_pct": (cpu_pct * 10.0).round() / 10.0, "mem_total": total, "mem_used": total.saturating_sub(avail), "disk_total": dt, "disk_free": df, "load1": load1, "hostname": hostname})
}

pub fn disk(path: &Path) -> (u64, u64) {
    let Ok(c) = std::ffi::CString::new(path.to_string_lossy().as_bytes()) else { return (0, 0) };
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 { return (0, 0); }
    let bs = s.f_frsize as u64;
    (s.f_blocks as u64 * bs, s.f_bavail as u64 * bs)
}

/// Total (cpu ticks, rss bytes) of every process in a process group (the shell wrapper plus the game).
pub fn group_usage(pgid: i64) -> Option<(u64, u64)> {
    let (mut ticks, mut rss, mut found) = (0u64, 0u64, false);
    for e in fs::read_dir("/proc").ok()?.flatten() {
        let name = e.file_name();
        let Some(n) = name.to_str() else { continue };
        if !n.bytes().all(|b| b.is_ascii_digit()) { continue; }
        let Ok(s) = fs::read_to_string(e.path().join("stat")) else { continue };
        // fields after the ")" of the comm: state ppid pgrp ...
        let Some(rest) = s.rfind(')').map(|i| &s[i + 2..]) else { continue };
        let f: Vec<&str> = rest.split_whitespace().collect();
        if f.len() < 22 { continue; }
        if f[2].parse::<i64>().ok() != Some(pgid) { continue; }
        found = true;
        ticks += f[11].parse::<u64>().unwrap_or(0) + f[12].parse::<u64>().unwrap_or(0);
        rss += f[21].parse::<u64>().unwrap_or(0) * page_size();
    }
    found.then_some((ticks, rss))
}

pub fn ticks_to_secs(t: u64) -> f64 { t as f64 / clk_tck() }

/// Start time (clock ticks since boot) of a pid, used to defeat PID reuse. None if the process is gone/zombie.
pub fn pid_start(pid: i64) -> Option<u64> {
    let s = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest = &s[s.rfind(')')? + 2..];
    let f: Vec<&str> = rest.split_whitespace().collect();
    if f.first() == Some(&"Z") { return None; }
    f.get(19)?.parse().ok()
}
