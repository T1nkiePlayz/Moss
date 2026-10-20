//! Container isolation end to end against `tests/stubs/docker` (a shell test double, NOT a real Docker or Podman):
//! drives the real binary over HTTP through enable, image pull, start, console, SIGKILLed-controller re-attach,
//! graceful stop, crash and "died while the controller was down", and asserts the exact `run` argv the engine saw.
//! No container runtime or Java exists in CI here, so this proves Moss's own logic only.
use serde_json::{Value, json};
use std::{net::TcpListener, path::{Path, PathBuf}, process::{Child, Command, Stdio}, time::{Duration, Instant}};

struct Moss { child: Child, base: String, port: u16 }

fn free_port() -> u16 { TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port() }

impl Moss {
    fn start(dir: &Path, port: u16, engine: &str) -> Self {
        let child = Command::new(env!("CARGO_BIN_EXE_moss")).arg("serve")
            .env("MOSS_AUTH", "dev").env("MOSS_DEV_RUNTIMES", "true").env("MOSS_PORT", port.to_string()).env("MOSS_DATA_DIR", dir).env("MOSS_CONTAINER_ENGINE", engine)
            .env("RUST_LOG", "moss=warn").stdout(Stdio::null()).stderr(Stdio::null()).spawn().expect("spawn moss");
        Self { child, base: format!("http://127.0.0.1:{port}/api/v1"), port }
    }
    fn sigkill(&mut self) { self.child.kill().unwrap(); self.child.wait().unwrap(); }
}
impl Drop for Moss { fn drop(&mut self) { let _ = self.child.kill(); let _ = self.child.wait(); } }

async fn until<F: FnMut() -> Fut, Fut: std::future::Future<Output = Option<T>>, T>(what: &str, secs: u64, mut f: F) -> T {
    let end = Instant::now() + Duration::from_secs(secs);
    loop {
        if let Some(v) = f().await { return v; }
        assert!(Instant::now() < end, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}
async fn get(c: &reqwest::Client, url: String) -> Option<Value> { c.get(url).send().await.ok()?.json().await.ok() }
async fn state(c: &reqwest::Client, base: &str, id: &str) -> Option<Value> { get(c, format!("{base}/servers/{id}")).await }
async fn wait_state(c: &reqwest::Client, base: &str, id: &str, want: &'static str, secs: u64) -> Value {
    until(&format!("server to be {want}"), secs, || async { let s = state(c, base, id).await?; (s["state"] == want).then_some(s) }).await
}
async fn timeline_has(c: &reqwest::Client, base: &str, id: &str, kind: &str) -> bool {
    get(c, format!("{base}/servers/{id}/timeline")).await.is_some_and(|t| t.as_array().is_some_and(|a| a.iter().any(|e| e["kind"] == kind)))
}
async fn wait_op(c: &reqwest::Client, base: &str, id: &str) -> Value {
    until("operation to finish", 60, || async { let o = get(c, format!("{base}/operations/{id}")).await?; matches!(o["state"].as_str()?, "completed" | "failed" | "cancelled").then_some(o) }).await
}
async fn cmd(c: &reqwest::Client, base: &str, id: &str, line: &str) { assert_eq!(c.post(format!("{base}/servers/{id}/command")).json(&json!({"command": line})).send().await.unwrap().status(), 204); }

/// A wrapper script that bakes the stub's state paths in (the controller scrubs the CLI's environment, as it should).
fn write_engine(root: &Path) -> (PathBuf, PathBuf, PathBuf) {
    std::fs::create_dir_all(root).unwrap();
    let (state, log, wrapper) = (root.join("stub-state"), root.join("calls.log"), root.join("docker"));
    std::fs::write(&wrapper, format!("#!/usr/bin/env bash\nexport STUB_STATE='{}' STUB_LOG='{}' STUB_GAME_CMD='{} sim-server --port {{PORT}}'\nexec '{}/tests/stubs/docker' \"$@\"\n",
        state.display(), log.display(), env!("CARGO_BIN_EXE_moss"), env!("CARGO_MANIFEST_DIR"))).unwrap();
    std::fs::set_permissions(&wrapper, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    (wrapper, state, log)
}

fn runs(log: &Path) -> Vec<Vec<String>> {
    std::fs::read_to_string(log).unwrap_or_default().lines().filter(|l| l.starts_with("run\x1f"))
        .map(|l| l.split('\x1f').filter(|s| !s.is_empty()).map(String::from).collect()).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn container_servers_start_stop_reattach_and_recover() {
    let root = std::env::temp_dir().join(format!("moss-ctr-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let data = root.join("data");
    let (engine, stub_state, log) = write_engine(&root);
    let (port, game_port) = (free_port(), free_port());
    let c = reqwest::Client::new();
    let mut moss = Moss::start(&data, port, engine.to_str().unwrap());
    until("controller", 20, || async { c.get(format!("http://127.0.0.1:{port}/healthz")).send().await.ok().filter(|r| r.status().is_success()) }).await;
    let base = moss.base.clone();

    let created: Value = c.post(format!("{base}/servers")).json(&json!({"name": "ctr", "runtime": "sim", "game_version": "sim-1.0", "memory_mb": 512, "port": game_port, "memory_limit_mb": 900, "cpu_limit_pct": 150, "pids_limit": 300}))
        .send().await.unwrap().json().await.unwrap();
    let id = created["server"]["id"].as_str().unwrap().to_string();
    assert_eq!(wait_op(&c, &base, created["operation"]["id"].as_str().unwrap()).await["state"], "completed");
    let name = format!("moss-{id}");

    // engine status and input validation: nothing but the allow-listed fields is accepted
    let status = get(&c, format!("{base}/container/status")).await.unwrap();
    assert_eq!((status["available"].clone(), status["engine"].clone(), status["rootless"].clone()), (json!(true), json!("docker"), json!(false)));
    for bad in [json!({"isolation": "privileged"}), json!({"container_image": "evil.example.com/x:1"}), json!({"container_image": "ubuntu:24.04"}), json!({"container_image": "ghcr.io/o/i:1 --privileged"})] {
        let r = c.patch(format!("{base}/servers/{id}")).json(&bad).send().await.unwrap();
        assert_eq!(r.status(), 400, "{bad}");
    }
    assert_eq!(c.patch(format!("{base}/servers/{id}")).json(&json!({"container_image": "ghcr.io/someone/java:21"})).send().await.unwrap().status(), 200);
    assert_eq!(c.patch(format!("{base}/servers/{id}")).json(&json!({"container_image": null})).send().await.unwrap().status(), 200);

    // enabling container mode downloads the image in an operation; the mode only applies on the next start
    let v: Value = c.patch(format!("{base}/servers/{id}")).json(&json!({"isolation": "container"})).send().await.unwrap().json().await.unwrap();
    assert_eq!(v["isolation"], "container");
    until("image pull operation", 30, || async {
        let ops = get(&c, format!("{base}/operations")).await?;
        ops.as_array()?.iter().find(|o| o["kind"] == "container.pull" && o["state"] == "completed").cloned()
    }).await;

    // start: running inside the (stub) container, with the engine's numbers in the usual stats pipeline
    assert!(c.post(format!("{base}/servers/{id}/actions/start")).send().await.unwrap().status().is_success());
    let running = wait_state(&c, &base, &id, "running", 30).await;
    let pid = running["pid"].as_i64().expect("container pid");
    assert_eq!(running["container"]["running_in_container"], true);
    until("stats from the engine", 20, || async { let s = state(&c, &base, &id).await?; (s["mem_bytes"] == 256u64 << 20 && s["cpu_pct"] == 7.5).then_some(()) }).await;
    cmd(&c, &base, &id, "say hello container").await;
    until("console echo", 10, || async {
        let b = get(&c, format!("{base}/servers/{id}/console/backlog?lines=200")).await?;
        b["lines"].as_array()?.iter().any(|l| l.as_str().is_some_and(|l| l.contains("say hello container"))).then_some(())
    }).await;

    // the exact command line the engine received
    let all = runs(&log);
    assert_eq!(all.len(), 1);
    let a = &all[0];
    let has = |x: &str| a.iter().any(|v| v == x);
    for want in ["--cap-drop=ALL", "--security-opt=no-new-privileges", "--read-only", "--restart=no", "--pids-limit=300", "--memory=900m", "--memory-swap=900m", "--cpus=1.50", "--detach"] { assert!(has(want), "missing {want}: {a:?}"); }
    assert!(has(&name) && has(&format!("{game_port}:{game_port}/tcp")) && has("docker.io/library/eclipse-temurin:21-jre"));
    let mounts: Vec<&String> = a.iter().zip(a.iter().skip(1)).filter(|(f, _)| *f == "-v").map(|(_, v)| v).collect();
    assert_eq!(mounts, [&format!("{}/servers/{id}:/data", data.display())], "exactly the server directory, nothing else");
    assert_eq!(a.iter().filter(|v| *v == "-p").count(), 1);
    assert!(!a.iter().any(|v| v.contains("privileged") || v.starts_with("--network") || v.starts_with("--cap-add") || v.starts_with("--device") || v.starts_with("--pid=") || v.starts_with("--volumes-from")), "{a:?}");
    assert!(!a.iter().any(|v| v == "-i" || v == "-t"), "no TTY or stdin attach: input goes through the FIFO");

    // SIGKILL the controller: the "container" keeps running; a new controller re-attaches by name + inspect
    moss.sigkill();
    assert!(std::net::TcpStream::connect_timeout(&format!("127.0.0.1:{game_port}").parse().unwrap(), Duration::from_secs(2)).is_ok(), "game died with the controller");
    let mut moss = Moss::start(&data, moss.port, engine.to_str().unwrap());
    let again = wait_state(&c, &moss.base, &id, "running", 30).await;
    assert_eq!(again["pid"].as_i64(), Some(pid));
    until("reattach on the timeline", 10, || async { timeline_has(&c, &moss.base, &id, "reattach").await.then_some(()) }).await;
    cmd(&c, &moss.base, &id, "say after restart").await;
    until("console echo after restart", 10, || async {
        let b = get(&c, format!("{}/servers/{id}/console/backlog?lines=200", moss.base)).await?;
        b["lines"].as_array()?.iter().any(|l| l.as_str().is_some_and(|l| l.contains("[Server] after restart"))).then_some(())
    }).await;

    eprintln!("step: graceful stop:");
    // graceful stop: `stop` over the console FIFO, then the container is removed and nothing is left behind
    assert!(c.post(format!("{}/servers/{id}/actions/stop", moss.base)).send().await.unwrap().status().is_success());
    wait_state(&c, &moss.base, &id, "stopped", 30).await;
    until("container removal", 10, || async { (!stub_state.join("c").join(&name).exists()).then_some(()) }).await;
    assert!(timeline_has(&c, &moss.base, &id, "stop").await);

    eprintln!("step: crash inside");
    // crash inside the container is detected from the engine's exit code
    assert!(c.post(format!("{}/servers/{id}/actions/start", moss.base)).send().await.unwrap().status().is_success());
    wait_state(&c, &moss.base, &id, "running", 30).await;
    cmd(&c, &moss.base, &id, "crash").await;
    until("crash detected", 20, || async { timeline_has(&c, &moss.base, &id, "crash").await.then_some(()) }).await;
    // the default restart policy brings it back inside a new container
    wait_state(&c, &moss.base, &id, "running", 30).await;
    assert!(timeline_has(&c, &moss.base, &id, "restart").await);
    assert!(c.post(format!("{}/servers/{id}/actions/kill", moss.base)).send().await.unwrap().status().is_success());
    wait_state(&c, &moss.base, &id, "stopped", 30).await;

    eprintln!("step: died while");
    // died while the controller was down: reconcile on boot accounts for it
    let before = get(&c, format!("{}/servers/{id}/timeline", moss.base)).await.unwrap().as_array().unwrap().iter().filter(|e| e["kind"] == "crash").count();
    assert!(c.post(format!("{}/servers/{id}/actions/start", moss.base)).send().await.unwrap().status().is_success());
    let s = wait_state(&c, &moss.base, &id, "running", 30).await;
    moss.sigkill();
    unsafe { libc::kill(s["pid"].as_i64().unwrap() as i32, libc::SIGKILL); }
    let moss = Moss::start(&data, moss.port, engine.to_str().unwrap());
    until("crash accounted for after boot", 30, || async {
        let n = get(&c, format!("{}/servers/{id}/timeline", moss.base)).await?.as_array()?.iter().filter(|e| e["kind"] == "crash").count();
        (n > before).then_some(())
    }).await;
    wait_state(&c, &moss.base, &id, "running", 30).await; // restart policy again
    assert!(c.post(format!("{}/servers/{id}/actions/kill", moss.base)).send().await.unwrap().status().is_success());
    wait_state(&c, &moss.base, &id, "stopped", 30).await;
    assert!(runs(&log).len() >= 4, "every start was a new container");
    drop(moss);
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test(flavor = "multi_thread")]
async fn without_an_engine_container_mode_is_refused_with_a_clear_message() {
    let root = std::env::temp_dir().join(format!("moss-ctr-e2e-none-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (port, game_port) = (free_port(), free_port());
    let c = reqwest::Client::new();
    let moss = Moss::start(&root.join("data"), port, "/definitely/not/docker");
    until("controller", 20, || async { c.get(format!("http://127.0.0.1:{port}/healthz")).send().await.ok().filter(|r| r.status().is_success()) }).await;
    let base = moss.base.clone();
    let status = get(&c, format!("{base}/container/status")).await.unwrap();
    assert_eq!(status["available"], false);
    assert!(status["message"].as_str().unwrap().contains("not found"));
    let created: Value = c.post(format!("{base}/servers")).json(&json!({"name": "n", "runtime": "sim", "game_version": "sim-1.0", "memory_mb": 512, "port": game_port})).send().await.unwrap().json().await.unwrap();
    let id = created["server"]["id"].as_str().unwrap();
    wait_op(&c, &base, created["operation"]["id"].as_str().unwrap()).await;
    let r = c.patch(format!("{base}/servers/{id}")).json(&json!({"isolation": "container"})).send().await.unwrap();
    assert_eq!(r.status(), 409);
    assert_eq!(r.json::<Value>().await.unwrap()["error"]["code"], "container.engine_missing");
    let s = state(&c, &base, id).await.unwrap();
    assert_eq!(s["isolation"], "process", "the mode did not change");
    drop(moss);
    let _ = std::fs::remove_dir_all(&root);
}
