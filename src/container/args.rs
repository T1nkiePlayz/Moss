//! The one fixed container command line, as pure functions so tests can assert the exact argv. There is no
//! parameter for extra flags, mounts, devices, capabilities or privileged mode anywhere in this module.
use super::{EngineInfo, Kind, OnceSpec, RunSpec, ports::{self, Network}};

pub const DATA: &str = "/data";
pub const LABEL: &str = "moss.managed=true";

/// Inside the container: stdin is the FIFO (opened read-write so it never sees EOF, like process mode), output is
/// appended to the same console log the controller tails. `exec` makes the game PID 1 so `stop` signals reach it.
const RUN_SCRIPT: &str = r#"exec "$@" <>/data/.moss/console.fifo >>/data/.moss/console.log 2>&1"#;

/// A bind-mount source must survive `-v src:dst[:opts]` parsing untouched.
pub fn mount_path_ok(p: &std::path::Path) -> bool {
    p.is_absolute() && p.to_str().is_some_and(|s| !s.contains([':', ',', '\n', '\0', '"', '\'']))
}

/// `--user`/`--userns` for the engine: the files in the server directory must stay owned by the Moss user.
fn user_args(info: &EngineInfo, uid: u32, gid: u32) -> Vec<String> {
    match (info.kind, info.rootless) {
        // rootless Docker maps the invoking user to root inside the container
        (Kind::Docker, true) => vec!["--user".into(), "0:0".into()],
        // rootless Podman: keep-id maps the host user to the same id inside
        (Kind::Podman, true) => vec!["--userns=keep-id".into(), "--user".into(), format!("{uid}:{gid}")],
        _ => vec!["--user".into(), format!("{uid}:{gid}")],
    }
}

fn mount(dir: &std::path::Path, selinux: bool) -> String { format!("{}:{DATA}{}", dir.display(), if selinux { ":Z" } else { "" }) }

fn hardening(info: &EngineInfo, uid: u32, gid: u32, memory_mb: u64, pids: u32) -> Vec<String> {
    let mut a: Vec<String> = user_args(info, uid, gid);
    a.extend(["--cap-drop=ALL", "--security-opt=no-new-privileges", "--read-only", "--tmpfs", "/tmp:rw,nosuid,nodev,size=512m"].map(String::from));
    a.extend([format!("--pids-limit={pids}"), format!("--memory={memory_mb}m"), format!("--memory-swap={memory_mb}m")]);
    a
}

/// Moss's own bridge with masquerading off: published ports still work (DNAT, replies via conntrack) but a new
/// outbound connection leaves with a private source address nothing routes back. DNS is pointed at an unroutable
/// address so the embedded resolver does not forward to the host's resolvers.
fn game_only(info: &EngineInfo) -> Vec<String> {
    let dns = match info.kind { Kind::Podman => "--dns=none".to_string(), Kind::Docker => "--dns=192.0.2.1".to_string() };
    vec!["--network".into(), ports::GAMEONLY_NET.into(), dns]
}

/// `docker network create` arguments for the game-only network.
pub fn gameonly_network_args() -> Vec<String> {
    ["network", "create", "--driver", "bridge", "--opt", "com.docker.network.bridge.enable_ip_masquerade=false", ports::GAMEONLY_NET].map(String::from).into()
}

/// `-p` values for the game port span: `port..=port+span` with the game's protocol; a UDP game also gets TCP on the
/// base port (remote console).
fn game_ports(s: &RunSpec) -> Vec<String> {
    let range = if s.span == 0 { format!("{0}:{0}", s.port) } else { let end = s.port.saturating_add(s.span); format!("{0}-{1}:{0}-{1}", s.port, end) };
    let mut v = vec![format!("{range}/{}", s.proto.name())];
    if s.proto == ports::Proto::Udp { v.push(format!("{0}:{0}/tcp", s.port)); }
    v
}

/// `docker run` arguments (after the CLI name) for a long-running game server.
pub fn run_args(info: &EngineInfo, s: &RunSpec) -> Vec<String> {
    let mut a: Vec<String> = ["run", "--detach", "--name"].map(String::from).into();
    a.push(s.name.clone());
    a.extend(["--label".into(), LABEL.into(), "--label".into(), format!("moss.server={}", s.server_id), "--restart=no".into()]);
    a.extend(hardening(info, s.uid, s.gid, s.memory_mb, s.pids));
    if let Some(c) = s.cpus { a.push(format!("--cpus={c:.2}")); }
    a.extend(["-v".into(), mount(&s.dir, s.selinux), "-w".into(), DATA.into()]);
    // the game's whole port span, published on the same numbers inside and out so the port the UI shows is the real one
    for p in game_ports(s) { a.extend(["-p".into(), p]); }
    for e in &s.extra_ports { a.extend(["-p".into(), format!("{0}:{0}/{1}", e.port, e.proto.name())]); }
    if s.network == Network::GameOnly { a.extend(game_only(info)); }
    a.extend(["-e".into(), "HOME=/data".into(), "-e".into(), "LANG=C.UTF-8".into()]);
    a.extend(["--entrypoint".into(), "/bin/sh".into(), s.image.clone(), "-c".into(), RUN_SCRIPT.into(), "moss-run".into()]);
    a.extend(s.argv.iter().cloned());
    a
}

/// Arguments for an installer container: `--rm`, same mount and hardening, default network (installers download).
pub fn once_args(info: &EngineInfo, s: &OnceSpec) -> Vec<String> {
    let mut a: Vec<String> = ["run", "--rm", "--name"].map(String::from).into();
    a.push(s.name.clone());
    a.extend(["--label".into(), LABEL.into()]);
    a.extend(hardening(info, s.uid, s.gid, s.memory_mb, s.pids));
    a.extend(["-v".into(), mount(&s.dir, s.selinux), "-w".into(), DATA.into(), "-e".into(), "HOME=/data".into(), "-e".into(), "LANG=C.UTF-8".into()]);
    a.push(s.image.clone());
    a.extend(s.argv.iter().cloned());
    a
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    pub fn info(kind: Kind, rootless: bool) -> EngineInfo { EngineInfo { kind, cli: PathBuf::from("/usr/bin/docker"), version: "27".into(), rootless } }
    pub fn spec() -> RunSpec {
        RunSpec { name: "moss-abc".into(), server_id: "abc".into(), image: "docker.io/library/eclipse-temurin:21-jre".into(), dir: PathBuf::from("/srv/moss/servers/abc"), port: 25570, proto: ports::Proto::Tcp, span: 0, network: Network::Open, extra_ports: vec![],
            memory_mb: 3072, cpus: Some(1.5), pids: 4096, uid: 1000, gid: 1001, selinux: false, argv: vec!["java".into(), "-Xmx2048M".into(), "-jar".into(), "server.jar".into(), "nogui".into()] }
    }

    #[test]
    fn exact_argv_for_a_rootful_docker_server() {
        let a = run_args(&info(Kind::Docker, false), &spec());
        let want = ["run", "--detach", "--name", "moss-abc", "--label", "moss.managed=true", "--label", "moss.server=abc", "--restart=no", "--user", "1000:1001",
            "--cap-drop=ALL", "--security-opt=no-new-privileges", "--read-only", "--tmpfs", "/tmp:rw,nosuid,nodev,size=512m", "--pids-limit=4096", "--memory=3072m", "--memory-swap=3072m",
            "--cpus=1.50", "-v", "/srv/moss/servers/abc:/data", "-w", "/data", "-p", "25570:25570/tcp", "-e", "HOME=/data", "-e", "LANG=C.UTF-8",
            "--entrypoint", "/bin/sh", "docker.io/library/eclipse-temurin:21-jre", "-c", RUN_SCRIPT, "moss-run", "java", "-Xmx2048M", "-jar", "server.jar", "nogui"];
        assert_eq!(a, want);
    }

    #[test]
    fn a_udp_game_publishes_its_span_as_udp_and_tcp_on_the_base_port() {
        let published = |s: &RunSpec| -> Vec<String> { let a = run_args(&info(Kind::Docker, false), s); a.iter().zip(a.iter().skip(1)).filter(|(f, _)| *f == "-p").map(|(_, v)| v.clone()).collect() };
        let mut s = spec();
        s.port = 2456; s.proto = ports::Proto::Udp; s.span = 2;
        assert_eq!(published(&s), ["2456-2458:2456-2458/udp", "2456:2456/tcp"]);
        s.span = 0;
        assert_eq!(published(&s), ["2456:2456/udp", "2456:2456/tcp"]);
        s.proto = ports::Proto::Tcp; s.span = 1; s.port = 27015;
        assert_eq!(published(&s), ["27015-27016:27015-27016/tcp"]);
    }

    #[test]
    fn never_privileged_never_extra_mounts_or_networks() {
        for (k, r) in [(Kind::Docker, false), (Kind::Docker, true), (Kind::Podman, false), (Kind::Podman, true)] {
            let a = run_args(&info(k, r), &spec());
            assert_eq!(a.iter().filter(|x| *x == "-v").count(), 1, "exactly one bind mount");
            assert_eq!(a.iter().filter(|x| *x == "-p").count(), 1, "exactly one published port");
            for bad in ["--privileged", "--network=host", "--net=host", "--pid=host", "--device", "--cap-add", "--volumes-from", "--ipc=host", "--userns=host"] {
                assert!(!a.iter().any(|x| x.starts_with(bad)), "{bad} must never appear");
            }
            assert!(a.contains(&"--cap-drop=ALL".to_string()) && a.contains(&"--security-opt=no-new-privileges".to_string()) && a.contains(&"--read-only".to_string()));
            assert!(a.iter().any(|x| x.starts_with("--pids-limit=")) && a.iter().any(|x| x.starts_with("--memory=")));
            let mounts: Vec<&String> = a.iter().zip(a.iter().skip(1)).filter(|(f, _)| *f == "-v").map(|(_, v)| v).collect();
            assert_eq!(mounts, ["/srv/moss/servers/abc:/data"]);
        }
    }

    #[test]
    fn game_only_adds_the_network_dns_and_extra_ports_only() {
        let mut s = spec();
        s.network = Network::GameOnly;
        s.extra_ports = vec![ports::ExtraPort { port: 19132, proto: ports::Proto::Udp }, ports::ExtraPort { port: 24454, proto: ports::Proto::Tcp }];
        let a = run_args(&info(Kind::Docker, false), &s);
        let at = |f: &str| a.iter().position(|x| x == f).map(|i| a[i + 1].clone());
        assert_eq!(at("--network").as_deref(), Some("moss-gameonly"));
        assert!(a.contains(&"--dns=192.0.2.1".to_string()));
        let published: Vec<&str> = a.iter().zip(a.iter().skip(1)).filter(|(f, _)| *f == "-p").map(|(_, v)| v.as_str()).collect();
        assert_eq!(published, ["25570:25570/tcp", "19132:19132/udp", "24454:24454/tcp"]);
        assert!(run_args(&info(Kind::Podman, true), &s).contains(&"--dns=none".to_string()));
        let open = run_args(&info(Kind::Docker, false), &spec());
        assert!(!open.iter().any(|x| x == "--network" || x.starts_with("--dns")), "open mode keeps the default bridge");
        assert!(!a.iter().any(|x| x == "--network=host" || x == "host"));
    }

    #[test]
    fn gameonly_network_has_masquerading_off() {
        let a = gameonly_network_args();
        assert_eq!(a, ["network", "create", "--driver", "bridge", "--opt", "com.docker.network.bridge.enable_ip_masquerade=false", "moss-gameonly"]);
    }

    #[test]
    fn user_mapping_per_engine_mode() {
        let u = |k, r| { let a = run_args(&info(k, r), &spec()); a.iter().position(|x| x == "--user").map(|i| a[i + 1].clone()) };
        assert_eq!(u(Kind::Docker, false).as_deref(), Some("1000:1001"));
        assert_eq!(u(Kind::Docker, true).as_deref(), Some("0:0"));
        assert_eq!(u(Kind::Podman, true).as_deref(), Some("1000:1001"));
        assert!(run_args(&info(Kind::Podman, true), &spec()).contains(&"--userns=keep-id".to_string()));
        assert!(!run_args(&info(Kind::Docker, false), &spec()).iter().any(|x| x.starts_with("--userns")));
    }

    #[test]
    fn selinux_relabels_the_one_mount_and_no_cpus_flag_when_unset() {
        let mut s = spec();
        s.selinux = true; s.cpus = None;
        let a = run_args(&info(Kind::Podman, false), &s);
        assert!(a.contains(&"/srv/moss/servers/abc:/data:Z".to_string()));
        assert!(!a.iter().any(|x| x.starts_with("--cpus")));
    }

    #[test]
    fn installer_container_is_removed_afterwards_and_hardened() {
        let o = OnceSpec { name: "moss-abc-install".into(), image: "docker.io/library/eclipse-temurin:21-jre".into(), dir: PathBuf::from("/srv/moss/servers/abc"), memory_mb: 2048, pids: 2048, uid: 1000, gid: 1000, selinux: false,
            argv: vec!["java".into(), "-jar".into(), "/data/.moss/installer.jar".into(), "--installServer".into()] };
        let a = once_args(&info(Kind::Docker, false), &o);
        assert_eq!(&a[..3], ["run", "--rm", "--name"]);
        assert!(a.contains(&"--cap-drop=ALL".to_string()) && a.contains(&"--read-only".to_string()) && !a.iter().any(|x| x == "--privileged"));
        assert_eq!(a.iter().filter(|x| *x == "-v").count(), 1);
        assert_eq!(&a[a.len() - 5..], ["docker.io/library/eclipse-temurin:21-jre", "java", "-jar", "/data/.moss/installer.jar", "--installServer"]);
    }

    #[test]
    fn mount_paths_with_separators_are_refused() {
        assert!(mount_path_ok(std::path::Path::new("/srv/moss/servers/abc")));
        assert!(!mount_path_ok(std::path::Path::new("/srv/a:b")) && !mount_path_ok(std::path::Path::new("/srv/a,b")) && !mount_path_ok(std::path::Path::new("rel/dir")));
    }
}
