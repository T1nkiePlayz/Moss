//! Installer argv, streamed execution (exit status, timeout, cancellation) and stash/restore tests.
use super::*;
use crate::runtimes::launch::test_srv;
use std::sync::{Arc, Mutex};

fn tmpdir() -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("moss-inst-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn sh(script: &str) -> Vec<String> { vec!["/bin/sh".into(), "-c".into(), script.into()] }

fn collector() -> (Arc<Mutex<Vec<String>>>, impl Fn(String) + Send + Sync + Clone + 'static) {
    let v = Arc::new(Mutex::new(vec![]));
    let v2 = v.clone();
    (v, move |l| v2.lock().unwrap().push(l))
}

#[test]
fn installer_arguments_per_runtime() {
    let n = installer_argv("/j/java", "/t/installer.jar", &test_srv("neoforge", "1.21.1", Some("21.1.172"), "minimal"));
    assert_eq!(n, ["/j/java", "-jar", "/t/installer.jar", "--installServer"]);
    let f = installer_argv("/j/java", "/t/i.jar", &test_srv("forge", "1.20.1", Some("47.2.0"), "minimal"));
    assert_eq!(f, ["/j/java", "-jar", "/t/i.jar", "--installServer"]);
    let q = installer_argv("/j/java", "/t/i.jar", &test_srv("quilt", "1.21.1", Some("0.26.4"), "minimal"));
    assert_eq!(q, ["/j/java", "-jar", "/t/i.jar", "install", "server", "1.21.1", "0.26.4", "--download-server", "--install-dir=."]);
    let q = installer_argv("/j/java", "/t/i.jar", &test_srv("quilt", "1.21.1", None, "minimal"));
    assert_eq!(&q[3..], ["install", "server", "1.21.1", "--download-server", "--install-dir=."]);
}

#[tokio::test]
async fn output_is_streamed_with_a_scrubbed_environment() {
    let dir = tmpdir();
    let (lines, log) = collector();
    run_streamed(&sh("echo out; echo err >&2; echo HOME=$HOME; echo SECRET=${MOSS_SECRET:-unset}"), &dir, Duration::from_secs(10), &CancellationToken::new(), log).await.unwrap();
    let l = lines.lock().unwrap().clone();
    assert!(l.contains(&"out".to_string()) && l.contains(&"err".to_string()));
    assert!(l.contains(&format!("HOME={}", dir.display())) && l.contains(&"SECRET=unset".to_string()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_nonzero_exit_status_is_an_error() {
    let dir = tmpdir();
    let (_, log) = collector();
    let e = run_streamed(&sh("echo boom; exit 3"), &dir, Duration::from_secs(10), &CancellationToken::new(), log).await.unwrap_err();
    assert!(format!("{e}").contains("failed"), "{e}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn timeouts_and_cancellation_stop_the_process() {
    let dir = tmpdir();
    let (_, log) = collector();
    let t = std::time::Instant::now();
    let e = run_streamed(&sh("sleep 30"), &dir, Duration::from_millis(200), &CancellationToken::new(), log.clone()).await.unwrap_err();
    assert!(format!("{e}").contains("did not finish") && t.elapsed() < Duration::from_secs(10));
    let tok = CancellationToken::new();
    let t2 = tok.clone();
    tokio::spawn(async move { tokio::time::sleep(Duration::from_millis(100)).await; t2.cancel(); });
    let e = run_streamed(&sh("sleep 30"), &dir, Duration::from_secs(20), &tok, log).await.unwrap_err();
    assert_eq!(format!("{e}"), "cancelled");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn failed_installs_are_cleaned_and_stashed_files_restored() {
    let dir = tmpdir();
    std::fs::write(dir.join("server.properties"), "x").unwrap();
    std::fs::create_dir_all(dir.join("libraries/old")).unwrap();
    std::fs::write(dir.join("libraries/old/a.jar"), "old").unwrap();
    std::fs::write(dir.join("user_jvm_args.txt"), "old").unwrap();
    let prev = dir.join(".moss/upgrade-prev");
    let moved = stash(&dir, &prev, "neoforge").unwrap();
    assert!(moved.contains(&"libraries".to_string()) && !dir.join("libraries").exists());
    let before = top_level(&dir);
    // a half-finished installer run leaves partial output behind
    std::fs::create_dir_all(dir.join("libraries/new")).unwrap();
    std::fs::write(dir.join("run.sh"), "new").unwrap();
    std::fs::write(dir.join("installer.log"), "partial").unwrap();
    cleanup_new(&dir, &before);
    assert!(!dir.join("run.sh").exists() && !dir.join("installer.log").exists() && dir.join(".moss").exists());
    unstash(&dir, &prev, &moved).unwrap();
    assert_eq!(std::fs::read_to_string(dir.join("libraries/old/a.jar")).unwrap(), "old");
    assert_eq!(std::fs::read_to_string(dir.join("user_jvm_args.txt")).unwrap(), "old");
    assert!(dir.join("server.properties").exists() && !prev.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn verification_requires_the_launch_files() {
    let dir = tmpdir();
    let s = test_srv("neoforge", "1.21.1", Some("21.1.172"), "minimal");
    assert!(verify(&dir, &s).is_err());
    let p = dir.join("libraries/net/neoforged/neoforge/21.1.172");
    std::fs::create_dir_all(&p).unwrap();
    std::fs::write(p.join("unix_args.txt"), "").unwrap();
    assert!(verify(&dir, &s).is_ok());
    assert!(verify(&dir, &test_srv("quilt", "1.21.1", None, "minimal")).is_err());
    std::fs::write(dir.join("quilt-server-launch.jar"), "").unwrap();
    assert!(verify(&dir, &test_srv("quilt", "1.21.1", None, "minimal")).is_ok());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn legacy_forge_verification_and_stash_cover_the_forge_jar() {
    let dir = tmpdir();
    let s = test_srv("forge", "1.12.2", Some("14.23.5.2860"), "minimal");
    assert!(verify(&dir, &s).is_err());
    std::fs::write(dir.join("forge-1.12.2-14.23.5.2860.jar"), "j").unwrap();
    std::fs::write(dir.join("minecraft_server.1.12.2.jar"), "v").unwrap();
    std::fs::write(dir.join("keep.txt"), "k").unwrap();
    assert!(verify(&dir, &s).is_ok());
    let prev = dir.join(".moss/upgrade-prev");
    let moved = stash(&dir, &prev, "forge").unwrap();
    assert!(moved.contains(&"forge-1.12.2-14.23.5.2860.jar".to_string()) && moved.contains(&"minecraft_server.1.12.2.jar".to_string()));
    assert!(dir.join("keep.txt").exists() && !dir.join("forge-1.12.2-14.23.5.2860.jar").exists());
    let _ = std::fs::remove_dir_all(&dir);
}
