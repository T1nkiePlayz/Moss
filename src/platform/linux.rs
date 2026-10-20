//! Linux: /proc based process and host statistics.
use super::GroupUsage;
use std::{fs, time::Instant};

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

/// (total, used) physical memory in bytes.
pub fn host_memory() -> (u64, u64) {
    let mem = fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let kb = |k: &str| mem.lines().find(|l| l.starts_with(k)).and_then(|l| l.split_whitespace().nth(1)).and_then(|x| x.parse::<u64>().ok()).unwrap_or(0) * 1024;
    let (total, avail) = (kb("MemTotal:"), kb("MemAvailable:"));
    (total, total.saturating_sub(avail))
}

pub fn load1() -> f64 {
    fs::read_to_string("/proc/loadavg").ok().and_then(|s| s.split_whitespace().next().and_then(|x| x.parse::<f64>().ok())).unwrap_or(0.0)
}

/// Start time (clock ticks since boot) of a pid, used to defeat PID reuse. None if the process is gone or a zombie.
pub fn pid_start(pid: i64) -> Option<u64> {
    let s = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest = &s[s.rfind(')')? + 2..];
    let f: Vec<&str> = rest.split_whitespace().collect();
    if f.first() == Some(&"Z") { return None; }
    f.get(19)?.parse().ok()
}

/// One pass over /proc, summing CPU seconds and RSS per process group (the shell wrapper plus the game share one).
pub fn group_usage_all() -> GroupUsage {
    let (tck, page) = (clk_tck(), page_size());
    let mut out = GroupUsage::new();
    let Ok(rd) = fs::read_dir("/proc") else { return out };
    for e in rd.flatten() {
        let name = e.file_name();
        let Some(n) = name.to_str() else { continue };
        if !n.bytes().all(|b| b.is_ascii_digit()) { continue; }
        let Ok(s) = fs::read_to_string(e.path().join("stat")) else { continue };
        let Some(rest) = s.rfind(')').map(|i| &s[i + 2..]) else { continue };
        let f: Vec<&str> = rest.split_whitespace().collect();
        if f.len() < 22 { continue; }
        let Some(pgid) = f[2].parse::<i64>().ok() else { continue };
        let ticks = f[11].parse::<u64>().unwrap_or(0) + f[12].parse::<u64>().unwrap_or(0);
        let rss = f[21].parse::<u64>().unwrap_or(0) * page;
        let slot = out.entry(pgid).or_insert((0.0, 0));
        slot.0 += ticks as f64 / tck;
        slot.1 += rss;
    }
    out
}
