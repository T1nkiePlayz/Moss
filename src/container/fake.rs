//! In-memory `Engine` for tests: records every call and lets a test script container state and failures.
use super::{Engine, EngineInfo, Inspect, Kind, OnceSpec, RunSpec, Stat};
use anyhow::{Result, bail};
use std::{collections::{HashMap, HashSet}, sync::Mutex, time::Duration};

#[derive(Default)]
pub struct Fake {
    pub calls: Mutex<Vec<String>>,
    pub images: Mutex<HashSet<String>>,
    pub containers: Mutex<HashMap<String, Inspect>>,
    pub runs: Mutex<Vec<RunSpec>>,
    pub onces: Mutex<Vec<OnceSpec>>,
    /// When set every call fails like an unreachable daemon.
    pub down: Mutex<bool>,
    /// A container that exits immediately after `run`.
    pub die_on_start: Mutex<bool>,
    pub stats_out: Mutex<HashMap<String, Stat>>,
}

impl Fake {
    pub fn calls_named(&self, prefix: &str) -> usize { self.calls.lock().unwrap().iter().filter(|c| c.starts_with(prefix)).count() }
    fn rec(&self, c: impl Into<String>) -> Result<()> {
        self.calls.lock().unwrap().push(c.into());
        if *self.down.lock().unwrap() { bail!("Cannot connect to the Docker daemon"); }
        Ok(())
    }
    pub fn set(&self, name: &str, i: Inspect) { self.containers.lock().unwrap().insert(name.into(), i); }
}

pub fn running(pid: i64) -> Inspect { Inspect { running: true, status: "running".into(), exit_code: Some(0), oom_killed: false, pid } }
pub fn exited(code: i32, oom: bool) -> Inspect { Inspect { running: false, status: "exited".into(), exit_code: Some(code), oom_killed: oom, pid: 0 } }

impl Engine for Fake {
    fn info(&self) -> EngineInfo { EngineInfo { kind: Kind::Docker, cli: "/fake/docker".into(), version: "99.0".into(), rootless: false } }
    fn image_present(&self, image: &str) -> Result<bool> { self.rec(format!("image_present {image}"))?; Ok(self.images.lock().unwrap().contains(image)) }
    fn pull(&self, image: &str, on_line: &mut dyn FnMut(&str) -> bool) -> Result<()> {
        self.rec(format!("pull {image}"))?;
        for l in ["Pulling fs layer", "Downloading 5MB/10MB", "Download complete"] { if !on_line(l) { bail!("cancelled"); } }
        self.images.lock().unwrap().insert(image.into());
        Ok(())
    }
    fn run(&self, spec: &RunSpec) -> Result<()> {
        self.rec(format!("run {}", spec.name))?;
        self.runs.lock().unwrap().push(spec.clone());
        let i = if *self.die_on_start.lock().unwrap() { exited(1, false) } else { running(4242) };
        self.set(&spec.name, i);
        Ok(())
    }
    fn inspect(&self, name: &str) -> Result<Option<Inspect>> { self.rec(format!("inspect {name}"))?; Ok(self.containers.lock().unwrap().get(name).cloned()) }
    fn running(&self) -> Result<HashSet<String>> {
        self.rec("ps")?;
        Ok(self.containers.lock().unwrap().iter().filter(|(_, i)| i.running).map(|(n, _)| n.clone()).collect())
    }
    fn stop(&self, name: &str, t: u32) -> Result<()> { self.rec(format!("stop {name} {t}"))?; self.set(name, exited(0, false)); Ok(()) }
    fn kill(&self, name: &str) -> Result<()> { self.rec(format!("kill {name}"))?; self.set(name, exited(137, false)); Ok(()) }
    fn rm(&self, name: &str) -> Result<()> { self.rec(format!("rm {name}"))?; self.containers.lock().unwrap().remove(name); Ok(()) }
    fn logs(&self, name: &str, _tail: u32) -> Result<String> { self.rec(format!("logs {name}"))?; Ok("Error: could not find or load main class\n".into()) }
    fn stats(&self, names: &[String]) -> Result<HashMap<String, Stat>> {
        self.rec(format!("stats {}", names.join(",")))?;
        Ok(self.stats_out.lock().unwrap().iter().filter(|(n, _)| names.contains(n)).map(|(n, s)| (n.clone(), *s)).collect())
    }
    fn run_once(&self, spec: &OnceSpec, on_line: &mut dyn FnMut(&str) -> bool, _timeout: Duration) -> Result<i32> {
        self.rec(format!("run_once {}", spec.name))?;
        self.onces.lock().unwrap().push(spec.clone());
        on_line("installing");
        Ok(0)
    }
}
