//! Everything that differs between operating systems lives here, behind one small interface:
//! process identity and usage, host statistics, disk space and Java discovery. Linux reads /proc and cgroups;
//! macOS uses libproc, sysctl and a couple of tiny subprocesses. Nothing else in Moss touches /proc.
use serde_json::{Value, json};
use std::{collections::HashMap, path::{Path, PathBuf}};

#[cfg(target_os = "linux")] mod linux;
#[cfg(target_os = "linux")] use linux as imp;
#[cfg(target_os = "macos")] mod macos;
#[cfg(target_os = "macos")] use macos as imp;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
compile_error!("Moss supports Linux and macOS hosts only");

pub mod cgroup;

pub use imp::{HostSampler, group_usage_all, host_memory, load1, pid_start};

/// Name Adoptium uses for this OS in its API (`os=` parameter).
pub const ADOPTIUM_OS: &str = if cfg!(target_os = "macos") { "mac" } else { "linux" };
pub const OS_LABEL: &str = if cfg!(target_os = "macos") { "macOS" } else { "Linux" };

/// Where `bin/java` lives inside an extracted Temurin archive (macOS bundles nest it under Contents/Home).
pub const JAVA_BIN_IN_HOME: &str = if cfg!(target_os = "macos") { "Contents/Home/bin/java" } else { "bin/java" };

pub fn adoptium_arch() -> Option<&'static str> {
    match std::env::consts::ARCH { "x86_64" => Some("x64"), "aarch64" => Some("aarch64"), _ => None }
}

/// Candidate `java` executables that are not managed by Moss, most specific first.
pub fn system_java_candidates() -> Vec<PathBuf> {
    let mut c: Vec<PathBuf> = vec![];
    if let Ok(h) = std::env::var("JAVA_HOME") { c.push(PathBuf::from(h).join("bin/java")); }
    let roots: &[&str] = if cfg!(target_os = "macos") {
        &["/Library/Java/JavaVirtualMachines", "/opt/homebrew/opt", "/usr/local/opt", "/opt/homebrew/Cellar", "/usr/local/Cellar"]
    } else {
        &["/usr/lib/jvm", "/usr/lib64/jvm", "/opt/java", "/opt"]
    };
    for base in roots {
        let Ok(rd) = std::fs::read_dir(base) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if cfg!(target_os = "macos") {
                // /Library/Java/JavaVirtualMachines/<jdk>/Contents/Home/bin/java, or Homebrew's openjdk@N/libexec/openjdk.jdk/...
                c.push(p.join("Contents/Home/bin/java"));
                c.push(p.join("libexec/openjdk.jdk/Contents/Home/bin/java"));
                if let Ok(rd2) = std::fs::read_dir(&p) { for v in rd2.flatten() { c.push(v.path().join("libexec/openjdk.jdk/Contents/Home/bin/java")); } }
            } else {
                c.push(p.join("bin/java"));
            }
        }
    }
    // On macOS /usr/bin/java is a stub that pops up an "install Java" dialog when no JDK exists, so never run it there.
    if cfg!(not(target_os = "macos")) { c.push(PathBuf::from("/usr/bin/java")); }
    c
}

/// The directories game servers inherit on PATH.
pub const GAME_PATH: &str = if cfg!(target_os = "macos") { "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin" } else { "/usr/local/bin:/usr/bin:/bin" };

/// UTF-8 locale for game servers: macOS has no guaranteed `C.UTF-8`, but always has `en_US.UTF-8`.
pub const GAME_LANG: &str = if cfg!(target_os = "macos") { "en_US.UTF-8" } else { "C.UTF-8" };

pub fn disk(path: &Path) -> (u64, u64) {
    let Ok(c) = std::ffi::CString::new(path.to_string_lossy().as_bytes()) else { return (0, 0) };
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 { return (0, 0); }
    let bs = s.f_frsize as u64;
    (s.f_blocks as u64 * bs, s.f_bavail as u64 * bs)
}

pub fn hostname() -> String {
    let mut buf = [0u8; 256];
    if unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len() - 1) } != 0 { return String::new(); }
    let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

pub fn host_json(cpu_pct: f64, data_dir: &Path) -> Value {
    let (total, used) = host_memory();
    let (dt, df) = disk(data_dir);
    json!({"cpu_pct": (cpu_pct * 10.0).round() / 10.0, "mem_total": total, "mem_used": used, "disk_total": dt, "disk_free": df, "load1": load1(), "hostname": hostname(), "os": OS_LABEL})
}

/// CPU seconds and resident bytes of every process group, from a single scan of the process table.
pub type GroupUsage = HashMap<i64, (f64, u64)>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_process_is_visible() {
        let me = std::process::id() as i64;
        assert!(pid_start(me).is_some());
        let usage = group_usage_all();
        assert!(!usage.is_empty());
        let (total, used) = host_memory();
        assert!(total > 0 && used <= total);
        assert!(disk(Path::new(".")).0 > 0);
        assert!(!hostname().is_empty());
    }

    #[test]
    fn java_layout_matches_the_platform() {
        assert!(JAVA_BIN_IN_HOME.ends_with("bin/java"));
        assert!(system_java_candidates().iter().all(|p| p.ends_with("java")));
    }
}
