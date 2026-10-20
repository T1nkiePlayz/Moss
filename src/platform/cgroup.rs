//! Resource limits for game servers through cgroup v2 (Linux only; everywhere else the backend is `none`).
//!
//! Moss needs a *delegated* subtree (the shipped systemd unit sets `Delegate=yes`). Inside Moss's own cgroup it
//! creates `moss-servers/` and one leaf per server, writes `memory.max`, `memory.swap.max`, `cpu.max` and `pids.max`,
//! and the launch script moves the server's process group into the leaf before the game starts. Everything here
//! takes the cgroup mount point as a parameter so the tree can be faked in a temporary directory for tests.
use serde_json::{Value, json};
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::OnceLock,
};

pub const MOUNT: &str = "/sys/fs/cgroup";
const GROUP: &str = "moss-servers";
const CPU_PERIOD: u64 = 100_000;
const WANTED: [&str; 3] = ["memory", "cpu", "pids"];

/// What a server asked for (all optional).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Requested { pub memory_mb: Option<i64>, pub cpu_pct: Option<i64>, pub pids: Option<i64> }

impl Requested {
    pub fn any(&self) -> bool { self.memory_mb.is_some() || self.cpu_pct.is_some() || self.pids.is_some() }
}

/// The cgroup path of this process from the contents of `/proc/self/cgroup` (the unified `0::` line).
pub fn parse_own_cgroup(proc_self_cgroup: &str) -> Option<String> {
    proc_self_cgroup.lines().find_map(|l| l.strip_prefix("0::")).map(|p| p.trim().to_string()).filter(|p| p.starts_with('/'))
}

/// `cpu.max` value for a percentage of one core (100 = one full core, 400 = four).
pub fn cpu_max(pct: i64) -> String { format!("{} {CPU_PERIOD}", pct.max(1) as u64 * CPU_PERIOD / 100) }

/// Default memory hard cap in bytes when none is configured: heap + 25 % + 512 MiB (metaspace, threads, direct buffers).
pub fn default_memory_cap(heap_mb: i64) -> u64 { let h = heap_mb.max(0) as u64; (h + h / 4 + 512) << 20 }

/// Number of OOM kills recorded in a `memory.events` file.
pub fn parse_oom_kills(events: &str) -> u64 {
    events.lines().find_map(|l| l.strip_prefix("oom_kill ")).and_then(|v| v.trim().parse().ok()).unwrap_or(0)
}

/// Under systemd Moss must not move itself out of the unit's cgroup: on the next (re)start systemd would be refused
/// entry into a cgroup that already has controllers enabled. There `DelegateSubgroup=supervisor` (systemd 254+) is required.
fn systemd_unit(own: &str) -> bool { own.ends_with(".service") || own.ends_with(".scope") }

/// Whether `cgroup.procs` lists nothing but `pid` (Moss owns the cgroup alone; otherwise it is someone else's and must not be touched).
fn only_self(procs: &str, pid: u32) -> bool { procs.split_whitespace().all(|p| p.parse() == Ok(pid)) }

/// Whether the cgroup is not a hierarchy Moss could write into (a path that escapes the mount, for example).
fn sane(rel: &str) -> bool { !rel.split('/').any(|c| c == "..") }

fn put(path: &Path, value: &str) -> io::Result<()> {
    use io::Write;
    fs::OpenOptions::new().write(true).create(true).truncate(false).open(path)?.write_all(value.as_bytes())
}

/// A prepared `moss-servers` group.
#[derive(Debug)]
pub struct Cg { base: PathBuf, controllers: Vec<String> }

fn read_controllers(dir: &Path) -> Vec<String> {
    fs::read_to_string(dir.join("cgroup.controllers")).unwrap_or_default().split_whitespace().map(String::from).collect()
}

fn prepare_at(dir: &Path) -> io::Result<Cg> {
    let have: Vec<String> = read_controllers(dir);
    let want: Vec<&str> = WANTED.iter().copied().filter(|w| have.iter().any(|h| h == w)).collect();
    if !want.contains(&"memory") { return Err(io::Error::other(format!("the memory controller is not delegated to {}", dir.display()))); }
    let plus = want.iter().map(|c| format!("+{c}")).collect::<Vec<_>>().join(" ");
    put(&dir.join("cgroup.subtree_control"), &plus)?;
    let base = dir.join(GROUP);
    fs::create_dir_all(&base)?;
    put(&base.join("cgroup.subtree_control"), &plus)?;
    Ok(Cg { base, controllers: want.iter().map(|s| s.to_string()).collect() })
}

/// Find and prepare a writable, delegated cgroup under `root` for the process whose own cgroup is `own`.
/// When that cgroup holds Moss itself (the kernel's "no internal processes" rule makes enabling controllers fail
/// with EBUSY) and Moss runs outside systemd (a container, a hand-made cgroup), it first moves itself into a `controller` leaf. Under `DelegateSubgroup=supervisor` the delegated
/// root is the parent and nothing needs moving.
pub fn prepare(root: &Path, own: &str, own_pid: u32) -> Result<Cg, String> {
    if !root.join("cgroup.controllers").exists() { return Err("cgroup v2 is not mounted (the host uses cgroup v1 or none)".into()); }
    if !sane(own) { return Err(format!("unexpected cgroup path {own}")); }
    let own_dir = root.join(own.trim_start_matches('/'));
    let mut tries = vec![own_dir.clone()];
    if let Some(p) = own_dir.parent().filter(|_| own.ends_with("/supervisor") && own != "/supervisor") { tries.insert(0, p.to_path_buf()); }
    let mut last = String::from("no candidate cgroup");
    for dir in &tries {
        match prepare_at(dir) {
            Ok(cg) => return Ok(cg),
            Err(e) if e.raw_os_error() == Some(libc::EBUSY) && dir == &own_dir && !systemd_unit(own) && only_self(&fs::read_to_string(dir.join("cgroup.procs")).unwrap_or_default(), own_pid) => {
                let leaf = own_dir.join("controller");
                let moved = fs::create_dir_all(&leaf).and_then(|_| put(&leaf.join("cgroup.procs"), &own_pid.to_string()));
                match moved.and_then(|_| prepare_at(&own_dir)) {
                    Ok(cg) => return Ok(cg),
                    Err(e2) => last = format!("{}: {e2}", own_dir.display()),
                }
            }
            Err(e) => last = format!("{}: {e}", dir.display()),
        }
    }
    Err(format!("no delegated cgroup is writable ({last}); run Moss from a systemd unit with Delegate=yes and DelegateSubgroup=supervisor (systemd 254+), see packaging/moss.service"))
}

impl Cg {
    pub fn dir(&self, id: &str) -> PathBuf { self.base.join(id) }
    pub fn has(&self, controller: &str) -> bool { self.controllers.iter().any(|c| c == controller) }

    /// Create the server's leaf and write the limits. Memory is mandatory (an error is returned if it cannot be set);
    /// swap, OOM grouping and the optional controllers are best effort.
    pub fn apply(&self, id: &str, memory_bytes: Option<u64>, req: &Requested) -> io::Result<PathBuf> {
        let dir = self.dir(id);
        fs::create_dir_all(&dir)?;
        if let Some(m) = memory_bytes {
            put(&dir.join("memory.max"), &m.to_string())?;
            let _ = put(&dir.join("memory.swap.max"), "0");
            let _ = put(&dir.join("memory.oom.group"), "1");
        }
        if let (Some(p), true) = (req.cpu_pct, self.has("cpu")) { put(&dir.join("cpu.max"), &cpu_max(p))?; }
        if let (Some(n), true) = (req.pids, self.has("pids")) { put(&dir.join("pids.max"), &n.to_string())?; }
        Ok(dir)
    }

    pub fn oom_kills(&self, id: &str) -> u64 { parse_oom_kills(&fs::read_to_string(self.dir(id).join("memory.events")).unwrap_or_default()) }

    pub fn memory_current(&self, id: &str) -> Option<u64> {
        fs::read_to_string(self.dir(id).join("memory.current")).ok()?.trim().parse().ok()
    }

    /// Remove the server's leaf (fails harmlessly while processes remain in it).
    pub fn remove(&self, id: &str) { let _ = fs::remove_dir(self.dir(id)); }
}

static GLOBAL: OnceLock<Result<Cg, String>> = OnceLock::new();

/// The process-wide backend, detected once (creating `moss-servers` as a side effect).
pub fn backend() -> Result<&'static Cg, &'static str> {
    let r = GLOBAL.get_or_init(|| {
        if !cfg!(target_os = "linux") { return Err("resource limits need cgroup v2, which only Linux has; on macOS Moss cannot cap memory or CPU".into()); }
        let own = fs::read_to_string("/proc/self/cgroup").ok().and_then(|s| parse_own_cgroup(&s)).ok_or_else(|| "cgroup v2 is not available".to_string())?;
        prepare(Path::new(MOUNT), &own, std::process::id())
    });
    r.as_ref().map_err(|e| e.as_str())
}

/// The `limits` object of the server JSON.
pub fn limits_json(req: &Requested) -> Value {
    let (enforced, detail) = match backend() { Ok(_) => (true, None), Err(e) => (false, Some(e.to_string())) };
    json!({"memory_mb": req.memory_mb, "cpu_pct": req.cpu_pct, "pids": req.pids, "enforced": enforced,
        "backend": if enforced { "cgroup2" } else { "none" }, "detail": detail})
}

/// Number of CPU cores, the upper bound of `cpu_limit_pct / 100`.
pub fn cores() -> i64 { std::thread::available_parallelism().map_or(1, |n| n.get() as i64) }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_own_cgroup() {
        assert_eq!(parse_own_cgroup("0::/system.slice/moss.service\n").as_deref(), Some("/system.slice/moss.service"));
        assert_eq!(parse_own_cgroup("12:memory:/x\n0::/\n").as_deref(), Some("/"));
        assert_eq!(parse_own_cgroup("12:memory:/x\n"), None);
    }

    #[test]
    fn systemd_units_are_recognised() {
        assert!(systemd_unit("/system.slice/moss.service") && systemd_unit("/user.slice/app.scope"));
        assert!(!systemd_unit("/docker/abc") && !systemd_unit("/"));
    }

    #[test]
    fn only_self_check() {
        assert!(only_self("12\n", 12) && only_self("", 12));
        assert!(!only_self("12\n13\n", 12));
    }

    #[test]
    fn formats_cpu_and_memory() {
        assert_eq!(cpu_max(100), "100000 100000");
        assert_eq!(cpu_max(250), "250000 100000");
        assert_eq!(cpu_max(10), "10000 100000");
        assert_eq!(default_memory_cap(2048), (2048 + 512 + 512) << 20);
        assert_eq!(parse_oom_kills("low 0\nhigh 0\nmax 3\noom 2\noom_kill 2\noom_group_kill 0\n"), 2);
        assert_eq!(parse_oom_kills(""), 0);
    }

    fn fake_root(controllers: &str) -> tempdir::T {
        let t = tempdir::T::new();
        fs::write(t.0.join("cgroup.controllers"), "cpuset cpu io memory pids").unwrap();
        let own = t.0.join("system.slice/moss.service");
        fs::create_dir_all(&own).unwrap();
        fs::write(own.join("cgroup.controllers"), controllers).unwrap();
        t
    }

    #[test]
    fn prepares_and_applies_on_a_fake_tree() {
        let t = fake_root("cpu memory pids");
        let cg = prepare(&t.0, "/system.slice/moss.service", 4242).unwrap();
        let own = t.0.join("system.slice/moss.service");
        assert_eq!(fs::read_to_string(own.join("cgroup.subtree_control")).unwrap(), "+memory +cpu +pids");
        let req = Requested { memory_mb: Some(1024), cpu_pct: Some(150), pids: Some(512) };
        let dir = cg.apply("srv1", Some(1024 << 20), &req).unwrap();
        assert_eq!(dir, own.join("moss-servers/srv1"));
        assert_eq!(fs::read_to_string(dir.join("memory.max")).unwrap(), (1024u64 << 20).to_string());
        assert_eq!(fs::read_to_string(dir.join("memory.swap.max")).unwrap(), "0");
        assert_eq!(fs::read_to_string(dir.join("cpu.max")).unwrap(), "150000 100000");
        assert_eq!(fs::read_to_string(dir.join("pids.max")).unwrap(), "512");
        fs::write(dir.join("memory.events"), "oom_kill 1\n").unwrap();
        fs::write(dir.join("memory.current"), "12345\n").unwrap();
        assert_eq!(cg.oom_kills("srv1"), 1);
        assert_eq!(cg.memory_current("srv1"), Some(12345));
        assert_eq!(cg.memory_current("other"), None);
        for f in ["memory.events", "memory.current", "memory.max", "memory.swap.max", "memory.oom.group", "cpu.max", "pids.max"] { fs::remove_file(dir.join(f)).unwrap(); }
        cg.remove("srv1");
        assert!(!dir.exists());
    }

    #[test]
    fn refuses_without_the_memory_controller_or_with_v1() {
        let t = fake_root("cpu pids");
        assert!(prepare(&t.0, "/system.slice/moss.service", 1).unwrap_err().contains("memory"));
        let empty = tempdir::T::new();
        assert!(prepare(&empty.0, "/x", 1).unwrap_err().contains("cgroup v2"));
        assert!(prepare(&t.0, "/../etc", 1).is_err());
    }

    #[test]
    fn delegate_subgroup_uses_the_parent() {
        let t = fake_root("cpu memory pids");
        fs::create_dir_all(t.0.join("system.slice/moss.service/supervisor")).unwrap();
        // the supervisor leaf has no controllers; the parent does
        let cg = prepare(&t.0, "/system.slice/moss.service/supervisor", 1).unwrap();
        assert_eq!(cg.dir("a"), t.0.join("system.slice/moss.service/moss-servers/a"));
    }

    #[test]
    fn json_without_backend_is_explicit() {
        let v = limits_json(&Requested { memory_mb: Some(512), ..Default::default() });
        assert_eq!(v["memory_mb"], 512);
        assert!(v["cpu_pct"].is_null());
        assert!(v["backend"] == "cgroup2" || v["detail"].is_string());
    }

    mod tempdir {
        use std::path::PathBuf;
        pub struct T(pub PathBuf);
        impl T {
            pub fn new() -> T {
                let p = std::env::temp_dir().join(format!("moss-cg-{}", uuid::Uuid::new_v4()));
                std::fs::create_dir_all(&p).unwrap();
                T(p)
            }
        }
        impl Drop for T { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
    }
}
