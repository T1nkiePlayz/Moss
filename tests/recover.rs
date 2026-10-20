//! `moss recover` against the real binary: works offline, refuses while a controller runs, honours --force.
use std::{net::TcpListener, process::{Command, Output, Stdio}, time::{Duration, Instant}};

fn moss(dir: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_moss")).args(args).env("MOSS_DATA_DIR", dir).env("MOSS_AUTH", "dev").env("RUST_LOG", "moss=warn").output().expect("run moss")
}
fn text(o: &Output) -> String { format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)) }

#[test]
fn recover_works_offline_and_refuses_while_running() {
    let dir = std::env::temp_dir().join(format!("moss-recover-it-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    // no database yet: a clear failure, not a panic
    let o = moss(&dir, &["recover", "db", "check"]);
    assert!(!o.status.success() && text(&o).contains("no database"), "{}", text(&o));
    // `doctor` creates the database
    assert!(moss(&dir, &["doctor"]).status.success());
    let o = moss(&dir, &["recover", "db", "check"]);
    assert!(o.status.success() && text(&o).contains("integrity check: ok"), "{}", text(&o));
    let o = moss(&dir, &["recover", "owner", "set", "me@example.com"]);
    assert!(o.status.success() && text(&o).contains("me@example.com is now the owner"), "{}", text(&o));
    assert!(!moss(&dir, &["recover", "owner", "set", "nonsense"]).status.success());
    let o = moss(&dir, &["recover", "export-config"]);
    assert!(text(&o).contains("me@example.com") && !text(&o).contains("\"hash\""));
    assert!(moss(&dir, &["recover", "setup-token", "new", "--unclaim"]).status.success());
    assert!(dir.join("setup-token").is_file());

    // a running controller holds the lock
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let mut child = Command::new(env!("CARGO_BIN_EXE_moss")).arg("serve").env("MOSS_DATA_DIR", &dir).env("MOSS_AUTH", "dev").env("MOSS_PORT", port.to_string())
        .stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    let end = Instant::now() + Duration::from_secs(20);
    while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() { assert!(Instant::now() < end, "controller did not start"); std::thread::sleep(Duration::from_millis(100)); }
    let o = moss(&dir, &["recover", "servers", "list"]);
    assert!(!o.status.success() && text(&o).contains("appears to be running"), "{}", text(&o));
    let o = moss(&dir, &["recover", "--force", "servers", "list"]);
    assert!(o.status.success() && text(&o).contains("No servers"), "{}", text(&o));
    // a second controller on the same data directory is refused
    let o = moss(&dir, &["serve", "--port", &(port + 1).to_string()]);
    assert!(!o.status.success() && text(&o).contains("already using"), "{}", text(&o));
    child.kill().unwrap();
    child.wait().unwrap();
    // the kernel released the lock with the process
    assert!(moss(&dir, &["recover", "servers", "list"]).status.success());
    let _ = std::fs::remove_dir_all(&dir);
}
