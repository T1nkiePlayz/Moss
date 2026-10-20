//! macOS: libproc for processes, sysctl/vm_stat/ps for host numbers.
use super::GroupUsage;
use std::{mem, process::Command, time::Instant};

/// Nanoseconds per mach tick (1/1 on Intel, 125/3 on Apple silicon); libproc reports CPU time in mach ticks.
fn timebase() -> (u64, u64) {
    let mut t = libc::mach_timebase_info { numer: 0, denom: 0 };
    if unsafe { libc::mach_timebase_info(&mut t) } != 0 || t.denom == 0 { return (1, 1); }
    (t.numer as u64, t.denom as u64)
}

fn bsd_info(pid: i32) -> Option<libc::proc_bsdinfo> {
    let mut info: libc::proc_bsdinfo = unsafe { mem::zeroed() };
    let size = mem::size_of::<libc::proc_bsdinfo>() as i32;
    let n = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, (&mut info as *mut libc::proc_bsdinfo).cast(), size) };
    (n == size).then_some(info)
}

fn task_info(pid: i32) -> Option<libc::proc_taskinfo> {
    let mut info: libc::proc_taskinfo = unsafe { mem::zeroed() };
    let size = mem::size_of::<libc::proc_taskinfo>() as i32;
    let n = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDTASKINFO, 0, (&mut info as *mut libc::proc_taskinfo).cast(), size) };
    (n == size).then_some(info)
}

/// Start time in microseconds since the epoch; stable for the life of the process, so it defeats PID reuse.
/// None if the process is gone or a zombie.
pub fn pid_start(pid: i64) -> Option<u64> {
    let i = bsd_info(i32::try_from(pid).ok()?)?;
    if i.pbi_status == libc::SZOMB { return None; }
    Some(i.pbi_start_tvsec * 1_000_000 + i.pbi_start_tvusec)
}

fn all_pids() -> Vec<i32> {
    let need = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if need <= 0 { return vec![]; }
    let mut buf = vec![0i32; need as usize + 64];
    let got = unsafe { libc::proc_listallpids(buf.as_mut_ptr().cast(), (buf.len() * mem::size_of::<i32>()) as i32) };
    if got <= 0 { return vec![]; }
    buf.truncate(got as usize);
    buf.retain(|p| *p > 0);
    buf
}

/// One pass over the process table, summing CPU seconds and resident bytes per process group.
pub fn group_usage_all() -> GroupUsage {
    let (numer, denom) = timebase();
    let mut out = GroupUsage::new();
    for pid in all_pids() {
        let (Some(b), Some(t)) = (bsd_info(pid), task_info(pid)) else { continue };
        let ns = (t.pti_total_user + t.pti_total_system) as u128 * numer as u128 / denom as u128;
        let slot = out.entry(b.pbi_pgid as i64).or_insert((0.0, 0));
        slot.0 += ns as f64 / 1e9;
        slot.1 += t.pti_resident_size;
    }
    out
}

fn sysctl_u64(name: &str) -> Option<u64> {
    let c = std::ffi::CString::new(name).ok()?;
    let mut v: u64 = 0;
    let mut len = mem::size_of::<u64>();
    let r = unsafe { libc::sysctlbyname(c.as_ptr(), (&mut v as *mut u64).cast(), &mut len, std::ptr::null_mut(), 0) };
    (r == 0).then_some(v)
}

/// (total, used) physical memory. "Used" follows Activity Monitor: everything except free, inactive and speculative pages.
pub fn host_memory() -> (u64, u64) {
    let total = sysctl_u64("hw.memsize").unwrap_or(0);
    let Ok(out) = Command::new("vm_stat").output() else { return (total, 0) };
    let text = String::from_utf8_lossy(&out.stdout);
    let page = text.lines().next().and_then(|l| l.split("page size of ").nth(1)).and_then(|r| r.split_whitespace().next()).and_then(|n| n.parse::<u64>().ok()).unwrap_or(16384);
    let pages = |key: &str| text.lines().find(|l| l.starts_with(key)).and_then(|l| l.split(':').nth(1)).and_then(|v| v.trim().trim_end_matches('.').parse::<u64>().ok()).unwrap_or(0);
    let reclaimable = (pages("Pages free") + pages("Pages inactive") + pages("Pages speculative")) * page;
    (total, total.saturating_sub(reclaimable))
}

pub fn load1() -> f64 {
    let mut l = [0f64; 3];
    if unsafe { libc::getloadavg(l.as_mut_ptr(), 3) } < 1 { 0.0 } else { l[0] }
}

#[derive(Default)]
pub struct HostSampler {
    prev: Option<Instant>,
    pub cpu_pct: f64,
}

impl HostSampler {
    /// Sum of per-process %cpu from `ps`, divided by the core count: close to Activity Monitor's total without mach APIs.
    pub fn sample(&mut self) {
        self.prev = Some(Instant::now());
        let Ok(out) = Command::new("ps").args(["-A", "-o", "%cpu="]).output() else { return };
        let sum: f64 = String::from_utf8_lossy(&out.stdout).lines().filter_map(|l| l.trim().parse::<f64>().ok()).sum();
        let cores = sysctl_u64("hw.logicalcpu").unwrap_or(1).max(1) as f64;
        self.cpu_pct = (sum / cores).clamp(0.0, 100.0);
    }
}
