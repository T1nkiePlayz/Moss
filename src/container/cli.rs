//! `Engine` over the `docker` / `podman` command line: argument arrays only, scrubbed environment, hard timeouts.
use super::{Engine, EngineInfo, Inspect, Kind, OnceSpec, RunSpec, Stat, args};
use anyhow::{Result, anyhow, bail};
use std::{collections::{HashMap, HashSet}, io::{BufRead, BufReader, Read}, path::{Path, PathBuf}, process::{Command, Stdio}, time::{Duration, Instant}};

pub struct CliEngine { info: EngineInfo }

/// Environment the CLI may see: where its socket lives, nothing else from the controller's environment.
fn scrubbed(cmd: &mut Command) {
    cmd.env_clear().env("PATH", std::env::var("PATH").unwrap_or_else(|_| crate::platform::GAME_PATH.into())).env("LANG", "C.UTF-8");
    for k in ["HOME", "XDG_RUNTIME_DIR", "DOCKER_HOST", "DOCKER_CONFIG", "CONTAINER_HOST", "DBUS_SESSION_BUS_ADDRESS"] {
        if let Ok(v) = std::env::var(k) { cmd.env(k, v); }
    }
}

struct Out { code: i32, stdout: String, stderr: String }

fn exec(cli: &Path, a: &[String], timeout: Duration) -> Result<Out> {
    let mut cmd = Command::new(cli);
    scrubbed(&mut cmd);
    let mut child = cmd.args(a).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| anyhow!("could not run {}: {e}", cli.display()))?;
    let (mut o, mut e) = (child.stdout.take().unwrap(), child.stderr.take().unwrap());
    let t_out = std::thread::spawn(move || { let mut s = String::new(); let _ = o.by_ref().take(4 << 20).read_to_string(&mut s); s });
    let t_err = std::thread::spawn(move || { let mut s = String::new(); let _ = e.by_ref().take(1 << 20).read_to_string(&mut s); s });
    let end = Instant::now() + timeout;
    let status = loop {
        if let Some(s) = child.try_wait()? { break s; }
        if Instant::now() > end { let _ = child.kill(); let _ = child.wait(); bail!("{} {} timed out after {}s", cli.display(), a.first().map_or("", |s| s), timeout.as_secs()); }
        std::thread::sleep(Duration::from_millis(15));
    };
    Ok(Out { code: status.code().unwrap_or(-1), stdout: t_out.join().unwrap_or_default(), stderr: t_err.join().unwrap_or_default() })
}

fn s(v: &[&str]) -> Vec<String> { v.iter().map(|x| x.to_string()).collect() }

fn first_line(t: &str) -> String { t.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim().chars().take(300).collect() }

impl CliEngine {
    pub fn new(info: EngineInfo) -> Self { CliEngine { info } }
    fn run_ok(&self, a: &[String], timeout: Duration) -> Result<String> {
        let o = exec(&self.info.cli, a, timeout)?;
        if o.code != 0 { bail!("{} {} failed: {}", self.info.kind.name(), a.first().map_or("", |s| s), first_line(&o.stderr)); }
        Ok(o.stdout)
    }
    /// Run a streaming command (pull, installer). `on_line` returns false to cancel; it is also called with "" about
    /// once a second while the process is silent, so cancellation is noticed without output.
    fn stream(&self, a: &[String], on_line: &mut dyn FnMut(&str) -> bool, timeout: Duration, cancel_cmd: Option<Vec<String>>) -> Result<i32> {
        let mut cmd = Command::new(&self.info.cli);
        scrubbed(&mut cmd);
        let mut child = cmd.args(a).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| anyhow!("could not run {}: {e}", self.info.cli.display()))?;
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        let pump = |r: Box<dyn Read + Send>, tx: std::sync::mpsc::Sender<String>| std::thread::spawn(move || {
            // progress bars redraw with \r: treat both as line ends
            for chunk in BufReader::new(r).split(b'\n').map_while(|l| l.ok()) {
                for part in String::from_utf8_lossy(&chunk).split('\r') { if !part.trim().is_empty() && tx.send(part.chars().filter(|c| !c.is_control()).take(400).collect()).is_err() { return; } }
            }
        });
        let (h1, h2) = (pump(Box::new(child.stdout.take().unwrap()), tx.clone()), pump(Box::new(child.stderr.take().unwrap()), tx));
        let end = Instant::now() + timeout;
        let mut stop = None;
        let mut quiet = Instant::now();
        let code = loop {
            while let Ok(l) = rx.try_recv() { quiet = Instant::now(); if !on_line(&l) { stop = Some("cancelled"); } }
            // A silent process must still notice cancellation: after 1 s without output, poll `on_line` with "".
            if stop.is_none() && quiet.elapsed() >= Duration::from_secs(1) { quiet = Instant::now(); if !on_line("") { stop = Some("cancelled"); } }
            if let Some(st) = child.try_wait()? { break st.code().unwrap_or(-1); }
            if stop.is_none() && Instant::now() > end { stop = Some("timed out"); }
            if let Some(why) = stop {
                if let Some(c) = &cancel_cmd { let _ = exec(&self.info.cli, c, Duration::from_secs(20)); }
                let _ = child.kill(); let _ = child.wait();
                let _ = (h1.join(), h2.join());
                bail!("{why}");
            }
            std::thread::sleep(Duration::from_millis(40));
        };
        let _ = (h1.join(), h2.join());
        while let Ok(l) = rx.try_recv() { if !on_line(&l) { stop = Some("cancelled"); } }
        if let Some(why) = stop { bail!("{why}"); }
        Ok(code)
    }
}

impl Engine for CliEngine {
    fn info(&self) -> EngineInfo { self.info.clone() }
    fn image_present(&self, image: &str) -> Result<bool> {
        let o = exec(&self.info.cli, &s(&["image", "inspect", "--format", "{{.Id}}", image]), Duration::from_secs(30))?;
        match o.code { 0 => Ok(true), _ if not_found(&o.stderr) => Ok(false), _ => bail!("could not check the image: {}", first_line(&o.stderr)) }
    }
    fn pull(&self, image: &str, on_line: &mut dyn FnMut(&str) -> bool) -> Result<()> {
        let code = self.stream(&s(&["pull", image]), on_line, Duration::from_secs(30 * 60), None)?;
        if code != 0 { bail!("pulling {image} failed"); }
        Ok(())
    }
    fn run(&self, spec: &RunSpec) -> Result<()> {
        if !args::mount_path_ok(&spec.dir) { bail!("the server directory path cannot be mounted safely"); }
        let a = args::run_args(&self.info, spec);
        self.run_ok(&a, Duration::from_secs(120)).map(|_| ())
    }
    fn ensure_gameonly_network(&self) -> Result<()> {
        if self.info.kind == Kind::Podman { bail!("{PODMAN_NO_GAMEONLY}"); }
        const FMT: &str = "{{index .Options \"com.docker.network.bridge.enable_ip_masquerade\"}}";
        let net = super::ports::GAMEONLY_NET;
        let check = || -> Result<Option<String>> {
            let o = exec(&self.info.cli, &s(&["network", "inspect", "--format", FMT, net]), Duration::from_secs(20))?;
            if o.code == 0 { return Ok(Some(o.stdout.trim().to_string())); }
            if not_found(&o.stderr) { return Ok(None); }
            bail!("could not inspect the network {net}: {}", first_line(&o.stderr))
        };
        let verify = |v: String| if v == "false" { Ok(()) } else { Err(anyhow!("the network {net} exists but was not created by Moss (outbound masquerading is on); remove it with `docker network rm {net}` and try again")) };
        if let Some(v) = check()? { return verify(v); }
        let o = exec(&self.info.cli, &args::gameonly_network_args(), Duration::from_secs(30))?;
        if o.code != 0 {
            // lost a race with another creator: fine if it now exists and is right
            return match check() { Ok(Some(v)) => verify(v), _ => bail!("could not create the network {net}: {}", first_line(&o.stderr)) };
        }
        verify(check()?.unwrap_or_default())
    }
    fn inspect(&self, name: &str) -> Result<Option<Inspect>> {
        let o = exec(&self.info.cli, &s(&["inspect", "--type", "container", "--format", INSPECT_FMT, name]), Duration::from_secs(20))?;
        if o.code == 0 { return parse_inspect(&o.stdout).map(Some); }
        if not_found(&o.stderr) { return Ok(None); }
        bail!("could not inspect {name}: {}", first_line(&o.stderr))
    }
    fn running(&self) -> Result<HashSet<String>> {
        let out = self.run_ok(&s(&["ps", "--filter", &format!("label={}", args::LABEL), "--format", "{{.Names}}"]), Duration::from_secs(20))?;
        Ok(out.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect())
    }
    fn stop(&self, name: &str, timeout_s: u32) -> Result<()> { self.run_ok(&s(&["stop", "--time", &timeout_s.to_string(), name]), Duration::from_secs(timeout_s as u64 + 30)).map(|_| ()) }
    fn kill(&self, name: &str) -> Result<()> {
        let o = exec(&self.info.cli, &s(&["kill", name]), Duration::from_secs(20))?;
        // a container that is already gone or already stopped is what the caller wanted
        if o.code != 0 && !not_found(&o.stderr) && !o.stderr.to_lowercase().contains("not running") { bail!("kill failed: {}", first_line(&o.stderr)); }
        Ok(())
    }
    fn rm(&self, name: &str) -> Result<()> {
        let o = exec(&self.info.cli, &s(&["rm", "--force", name]), Duration::from_secs(60))?;
        if o.code != 0 && !not_found(&o.stderr) { bail!("remove failed: {}", first_line(&o.stderr)); }
        Ok(())
    }
    fn logs(&self, name: &str, tail: u32) -> Result<String> {
        let o = exec(&self.info.cli, &s(&["logs", "--tail", &tail.to_string(), name]), Duration::from_secs(20))?;
        Ok(format!("{}{}", o.stdout, o.stderr))
    }
    fn stats(&self, names: &[String]) -> Result<HashMap<String, Stat>> {
        if names.is_empty() { return Ok(HashMap::new()); }
        let mut a = s(&["stats", "--no-stream", "--format", "{{.Name}}|{{.CPUPerc}}|{{.MemUsage}}"]);
        a.extend(names.iter().cloned());
        Ok(parse_stats(&self.run_ok(&a, Duration::from_secs(30))?))
    }
    fn run_once(&self, spec: &OnceSpec, on_line: &mut dyn FnMut(&str) -> bool, timeout: Duration) -> Result<i32> {
        if !args::mount_path_ok(&spec.dir) { bail!("the server directory path cannot be mounted safely"); }
        let a = args::once_args(&self.info, spec);
        self.stream(&a, on_line, timeout, Some(s(&["rm", "--force", &spec.name])))
    }
}

pub const PODMAN_NO_GAMEONLY: &str = "Game-only networking is not available with Podman: Moss only knows how to restrict outbound traffic safely with Docker's bridge options. Use the open network, or run this server under Docker.";

const INSPECT_FMT: &str = "{{.State.Status}}|{{.State.Running}}|{{.State.ExitCode}}|{{.State.OOMKilled}}|{{.State.Pid}}";

fn not_found(stderr: &str) -> bool {
    let e = stderr.to_lowercase();
    e.contains("no such") || e.contains("not found") || e.contains("no container with name") || e.contains("no such image")
}

pub fn parse_inspect(out: &str) -> Result<Inspect> {
    let p: Vec<&str> = out.trim().split('|').collect();
    if p.len() < 5 { bail!("unexpected inspect output"); }
    Ok(Inspect { status: p[0].to_string(), running: p[1] == "true", exit_code: p[2].parse().ok(), oom_killed: p[3] == "true", pid: p[4].parse().unwrap_or(0) })
}

/// `1.5GiB`, `512MiB`, `12.3kB` → bytes.
pub fn parse_size(t: &str) -> Option<u64> {
    let t = t.trim();
    let i = t.find(|c: char| c.is_ascii_alphabetic())?;
    let n: f64 = t[..i].trim().parse().ok()?;
    let mul = match t[i..].to_ascii_lowercase().as_str() {
        "b" => 1.0, "kb" => 1e3, "kib" => 1024.0, "mb" => 1e6, "mib" => 1048576.0, "gb" => 1e9, "gib" => 1073741824.0, "tb" => 1e12, "tib" => 1099511627776.0, _ => return None,
    };
    Some((n * mul) as u64)
}

pub fn parse_stats(out: &str) -> HashMap<String, Stat> {
    out.lines().filter_map(|l| {
        let mut p = l.trim().split('|');
        let (name, cpu, mem) = (p.next()?, p.next()?, p.next()?);
        Some((name.to_string(), Stat { cpu_pct: cpu.trim().trim_end_matches('%').parse().unwrap_or(0.0), mem_bytes: parse_size(mem.split('/').next()?)? }))
    }).collect()
}

/// First candidates found on PATH or in the usual install directories.
fn find_cli(name: &str) -> Option<PathBuf> {
    if name.contains('/') { return Path::new(name).is_file().then(|| PathBuf::from(name)); }
    let path = std::env::var("PATH").unwrap_or_default();
    let dirs = path.split(':').map(String::from).chain(["/usr/local/bin", "/usr/bin", "/opt/homebrew/bin", "/bin"].map(String::from));
    dirs.filter(|d| !d.is_empty()).map(|d| Path::new(&d).join(name)).find(|p| p.is_file())
}

fn probe(kind: Kind, cli: PathBuf) -> Result<EngineInfo, String> {
    let fmt = match kind { Kind::Docker => "{{.ServerVersion}}|{{.SecurityOptions}}", Kind::Podman => "{{.Version.Version}}|{{.Host.Security.Rootless}}" };
    let o = exec(&cli, &s(&["info", "--format", fmt]), Duration::from_secs(20)).map_err(|e| e.to_string())?;
    if o.code != 0 { return Err(format!("{} is installed but not usable by this user: {}", kind.name(), first_line(&o.stderr))); }
    let (ver, rest) = o.stdout.trim().split_once('|').ok_or("unexpected `info` output")?;
    let rootless = match kind { Kind::Docker => rest.contains("rootless"), Kind::Podman => rest.trim() == "true" };
    Ok(EngineInfo { kind, cli, version: ver.trim().to_string(), rootless })
}

/// Pick an engine: `MOSS_CONTAINER_ENGINE` (a CLI name or path) wins; otherwise Docker then Podman, except that
/// rootless Podman is preferred over a rootful Docker when both work. `Err` explains what is missing.
pub fn detect(preferred: &str) -> Result<CliEngine, String> {
    let kind_of = |p: &Path| if p.file_name().is_some_and(|n| n.to_string_lossy().contains("podman")) { Kind::Podman } else { Kind::Docker };
    if !preferred.trim().is_empty() {
        let cli = find_cli(preferred.trim()).ok_or_else(|| format!("the configured container engine {} was not found", preferred.trim()))?;
        return probe(kind_of(&cli), cli).map(CliEngine::new);
    }
    let docker = find_cli("docker").map(|c| probe(Kind::Docker, c));
    let podman = find_cli("podman").map(|c| probe(Kind::Podman, c));
    match (docker, podman) {
        (_, Some(Ok(p))) if p.rootless => Ok(CliEngine::new(p)),
        (Some(Ok(d)), _) => Ok(CliEngine::new(d)),
        (_, Some(Ok(p))) => Ok(CliEngine::new(p)),
        (None, None) => Err(super::NO_ENGINE_HELP.into()),
        (d, p) => Err(d.and_then(|r| r.err()).or_else(|| p.and_then(|r| r.err())).unwrap_or_else(|| super::NO_ENGINE_HELP.into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sizes_and_stats() {
        assert_eq!(parse_size("1.5GiB"), Some(1610612736));
        assert_eq!(parse_size("512MiB"), Some(536870912));
        assert_eq!(parse_size("12.3kB"), Some(12300));
        assert_eq!(parse_size("0B"), Some(0));
        assert_eq!(parse_size("nonsense"), None);
        let m = parse_stats("moss-a|12.50%|1.2GiB / 3GiB\nmoss-b|--|0B / 0B\ngarbage\n");
        assert_eq!(m["moss-a"], Stat { cpu_pct: 12.5, mem_bytes: (1.2 * 1073741824.0) as u64 });
        assert_eq!(m["moss-b"].cpu_pct, 0.0);
        assert!(!m.contains_key("garbage"));
    }

    #[test]
    fn parses_inspect() {
        let i = parse_inspect("running|true|0|false|4242\n").unwrap();
        assert_eq!(i, Inspect { running: true, status: "running".into(), exit_code: Some(0), oom_killed: false, pid: 4242 });
        let x = parse_inspect("exited|false|137|true|0").unwrap();
        assert!(!x.running && x.oom_killed && x.exit_code == Some(137));
        assert!(parse_inspect("bad").is_err());
    }

    #[test]
    fn missing_container_messages_are_recognised() {
        assert!(not_found("Error response from daemon: No such container: moss-x"));
        assert!(not_found("Error: no container with name or ID \"moss-x\" found: no such container"));
        assert!(!not_found("Cannot connect to the Docker daemon at unix:///var/run/docker.sock. Is the docker daemon running?"));
    }
}
