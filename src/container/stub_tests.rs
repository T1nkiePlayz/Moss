//! `CliEngine` against `tests/stubs/docker`, the shell test double: checks the real command lines, parsing and
//! error handling end to end. This is NOT a test of Docker or Podman themselves (none is installed in CI here).
use super::{Engine, OnceSpec, RunSpec, cli, ports};
use std::{path::PathBuf, time::Duration};

struct Env { engine: cli::CliEngine, log: PathBuf, dir: PathBuf }

fn setup(game_cmd: &str, installer: &str) -> Env { setup_with(game_cmd, installer, "") }

fn setup_with(game_cmd: &str, installer: &str, extra_env: &str) -> Env {
    let root = std::env::temp_dir().join(format!("moss-stub-{}", uuid::Uuid::new_v4()));
    let (state, dir) = (root.join("state"), root.join("server"));
    std::fs::create_dir_all(dir.join(".moss")).unwrap();
    let log = root.join("calls.log");
    let wrapper = root.join("docker");
    std::fs::write(&wrapper, format!("#!/usr/bin/env bash\n[ \"$1\" = __probe ] && exit 0\nexport STUB_STATE='{}' STUB_LOG='{}' STUB_GAME_CMD='{game_cmd}' STUB_INSTALLER_CMD='{installer}' {extra_env}\nexec '{}/tests/stubs/docker' \"$@\"\n",
        state.display(), log.display(), env!("CARGO_MANIFEST_DIR"))).unwrap();
    std::fs::set_permissions(&wrapper, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    // A process another test forks while the file is being written inherits its write handle until that process
    // execs, and exec'ing the wrapper meanwhile fails with ETXTBSY. Wait until it runs.
    for i in 0.. {
        match std::process::Command::new(&wrapper).arg("__probe").status() {
            Err(e) if e.raw_os_error() == Some(26) && i < 100 => std::thread::sleep(std::time::Duration::from_millis(10)),
            r => { assert!(r.expect("run stub wrapper").success()); break; }
        }
    }
    Env { engine: cli::detect(wrapper.to_str().unwrap()).expect("stub engine"), log, dir }
}

fn spec(e: &Env, name: &str) -> RunSpec {
    RunSpec { name: name.into(), server_id: "x".into(), image: "docker.io/library/eclipse-temurin:21-jre".into(), dir: e.dir.clone(), port: 25599, network: ports::Network::Open, extra_ports: vec![], memory_mb: 1024, cpus: None, pids: 512, uid: 1000, gid: 1000, selinux: false, argv: vec!["java".into(), "-jar".into(), "server.jar".into()] }
}

#[test]
fn detect_reads_version_and_mode_from_info() {
    let e = setup("sleep 30", "");
    let i = e.engine.info();
    assert_eq!((i.kind.name(), i.version.as_str(), i.rootless), ("docker", "27.0.0-stub", false));
    assert!(cli::detect("/no/such/docker").is_err());
}

#[test]
fn pull_streams_progress_and_can_be_cancelled() {
    let e = setup("sleep 30", "");
    let img = "docker.io/library/eclipse-temurin:21-jre";
    assert!(!e.engine.image_present(img).unwrap());
    let mut lines = vec![];
    e.engine.pull(img, &mut |l| { lines.push(l.to_string()); true }).unwrap();
    assert!(lines.iter().any(|l| l.contains("Downloading")), "{lines:?}");
    assert!(e.engine.image_present(img).unwrap());
    assert!(e.engine.pull("docker.io/library/eclipse-temurin:17-jre", &mut |_| false).is_err(), "returning false cancels");
}

#[test]
fn container_lifecycle_run_inspect_stats_stop_kill_rm() {
    let e = setup("sleep 30", "");
    let sp = spec(&e, "moss-stubA");
    e.engine.run(&sp).unwrap();
    let i = e.engine.inspect("moss-stubA").unwrap().expect("exists");
    assert!(i.running && i.pid > 1);
    assert!(e.engine.running().unwrap().contains("moss-stubA"));
    let st = e.engine.stats(&["moss-stubA".into()]).unwrap();
    assert_eq!((st["moss-stubA"].cpu_pct, st["moss-stubA"].mem_bytes), (7.5, 256 << 20));
    assert!(e.engine.run(&sp).is_err(), "name already in use");
    e.engine.stop("moss-stubA", 5).unwrap();
    let i = e.engine.inspect("moss-stubA").unwrap().unwrap();
    assert!(!i.running && i.exit_code.is_some());
    e.engine.rm("moss-stubA").unwrap();
    assert!(e.engine.inspect("moss-stubA").unwrap().is_none(), "removed container is Ok(None), not an error");
    e.engine.rm("moss-stubA").unwrap(); // removing twice is fine

    e.engine.run(&spec(&e, "moss-stubB")).unwrap();
    e.engine.kill("moss-stubB").unwrap();
    assert_eq!(e.engine.inspect("moss-stubB").unwrap().unwrap().exit_code, Some(137));
    e.engine.kill("moss-stubB").unwrap(); // already dead is fine
    assert!(e.engine.logs("moss-stubB", 5).unwrap().contains("stub container log"));
    e.engine.rm("moss-stubB").unwrap();
}

#[test]
fn the_recorded_run_command_is_the_locked_down_one() {
    let e = setup("sleep 30", "");
    e.engine.run(&spec(&e, "moss-stubC")).unwrap();
    e.engine.rm("moss-stubC").unwrap();
    let log = std::fs::read_to_string(&e.log).unwrap();
    let run: Vec<&str> = log.lines().find(|l| l.starts_with("run\x1f")).unwrap().split('\x1f').filter(|s| !s.is_empty()).collect();
    for want in ["--cap-drop=ALL", "--security-opt=no-new-privileges", "--read-only", "--pids-limit=512", "--memory=1024m", "--restart=no", "25599:25599/tcp"] { assert!(run.contains(&want), "missing {want}: {run:?}"); }
    assert_eq!(run.iter().filter(|a| **a == "-v").count(), 1);
    assert!(!run.iter().any(|a| a.contains("privileged") || a.starts_with("--network") || a.starts_with("--cap-add") || a.starts_with("--device")));
    assert!(run.contains(&format!("{}:/data", e.dir.display()).as_str()));
}

#[test]
fn installer_container_streams_output_and_returns_the_exit_code() {
    let e = setup("sleep 30", "echo from-installer; exit 3");
    let o = OnceSpec { name: "moss-x-installer".into(), image: "docker.io/library/eclipse-temurin:21-jre".into(), dir: e.dir.clone(), memory_mb: 512, pids: 256, uid: 1000, gid: 1000, selinux: false, argv: vec!["java".into(), "-jar".into(), "/data/.moss/installer.jar".into()] };
    let mut lines = vec![];
    let code = e.engine.run_once(&o, &mut |l| { lines.push(l.to_string()); true }, Duration::from_secs(20)).unwrap();
    assert_eq!(code, 3);
    assert!(lines.iter().any(|l| l == "from-installer"), "{lines:?}");
}

#[test]
fn game_only_network_is_created_once_with_masquerading_off_and_the_run_uses_it() {
    let e = setup("sleep 30", "");
    e.engine.ensure_gameonly_network().unwrap();
    e.engine.ensure_gameonly_network().unwrap(); // idempotent: second call only inspects
    let mut sp = spec(&e, "moss-stubN");
    sp.network = ports::Network::GameOnly;
    sp.extra_ports = vec![ports::ExtraPort { port: 19132, proto: ports::Proto::Udp }];
    e.engine.run(&sp).unwrap();
    e.engine.rm("moss-stubN").unwrap();
    let log = std::fs::read_to_string(&e.log).unwrap();
    let creates: Vec<&str> = log.lines().filter(|l| l.starts_with("network\x1fcreate\x1f")).collect();
    assert_eq!(creates.len(), 1, "{log}");
    assert!(creates[0].contains("com.docker.network.bridge.enable_ip_masquerade=false") && creates[0].contains("moss-gameonly"));
    let run: Vec<&str> = log.lines().find(|l| l.starts_with("run\x1f")).unwrap().split('\x1f').filter(|s| !s.is_empty()).collect();
    assert!(run.windows(2).any(|w| w == ["--network", "moss-gameonly"]) && run.contains(&"--dns=192.0.2.1") && run.contains(&"19132:19132/udp") && run.contains(&"25599:25599/tcp"));
}

#[test]
fn game_only_refuses_a_foreign_network_and_reports_create_failures() {
    let e = setup_with("sleep 30", "", "STUB_NET_FAIL=1");
    assert!(e.engine.ensure_gameonly_network().unwrap_err().to_string().contains("could not create"));
    let e = setup_with("sleep 30", "", "STUB_NET_MASQ=true");
    // pre-existing network with masquerading on (not ours): refused, never reused
    std::fs::create_dir_all(e.dir.parent().unwrap().join("state")).unwrap();
    std::fs::write(e.dir.parent().unwrap().join("state/net-moss-gameonly"), "true\n").unwrap();
    assert!(e.engine.ensure_gameonly_network().unwrap_err().to_string().contains("not created by Moss"));
}
