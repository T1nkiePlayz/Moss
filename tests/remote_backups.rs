//! End to end against the real binary: folder destination upload and verification, retention of local copies,
//! restore from a remote-only backup (including a corrupted remote copy), promote-after-compare restores and the
//! secrets rules for S3 destinations. S3 itself is covered against an in-process fake in the unit tests.
use serde_json::{Value, json};
use std::{net::TcpListener, path::{Path, PathBuf}, process::{Child, Command, Stdio}, time::{Duration, Instant}};

struct Moss { child: Child, base: String }
impl Moss {
    fn start(dir: &Path, port: u16) -> Self {
        let child = Command::new(env!("CARGO_BIN_EXE_moss")).arg("serve").env("MOSS_AUTH", "dev").env("MOSS_DEV_RUNTIMES", "true").env("MOSS_PORT", port.to_string()).env("MOSS_DATA_DIR", dir)
            .env("RUST_LOG", "moss=warn").stdout(Stdio::null()).stderr(Stdio::null()).spawn().expect("spawn moss");
        Self { child, base: format!("http://127.0.0.1:{port}/api/v1") }
    }
}
impl Drop for Moss { fn drop(&mut self) { let _ = self.child.kill(); let _ = self.child.wait(); } }

fn free_port() -> u16 { TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port() }
async fn until<F: FnMut() -> Fut, Fut: std::future::Future<Output = Option<T>>, T>(what: &str, secs: u64, mut f: F) -> T {
    let end = Instant::now() + Duration::from_secs(secs);
    loop { if let Some(v) = f().await { return v; } assert!(Instant::now() < end, "timed out waiting for {what}"); tokio::time::sleep(Duration::from_millis(200)).await; }
}

struct Api { c: reqwest::Client, base: String }
impl Api {
    async fn get(&self, p: &str) -> Value { self.c.get(format!("{}{p}", self.base)).send().await.unwrap().json().await.unwrap_or(Value::Null) }
    async fn send(&self, m: reqwest::Method, p: &str, b: Value) -> (u16, Value) {
        let r = self.c.request(m, format!("{}{p}", self.base)).json(&b).send().await.unwrap();
        (r.status().as_u16(), r.json().await.unwrap_or(Value::Null))
    }
    async fn post(&self, p: &str, b: Value) -> (u16, Value) { self.send(reqwest::Method::POST, p, b).await }
    async fn op(&self, v: &Value) -> Value {
        let id = v["operation"]["id"].as_str().unwrap_or_else(|| panic!("no operation in {v}")).to_string();
        until("operation", 60, || async { let o = self.get(&format!("/operations/{id}")).await; matches!(o["state"].as_str()?, "completed" | "failed" | "cancelled").then_some(o) }).await
    }
    async fn backup(&self, sid: &str) -> String {
        let (s, v) = self.post(&format!("/servers/{sid}/backups"), json!({})).await;
        assert_eq!(s, 202, "{v}");
        let o = self.op(&v).await;
        assert_eq!(o["state"], "completed", "{o}");
        self.get(&format!("/servers/{sid}/backups")).await.as_array().unwrap().iter().filter(|b| b["kind"] == "manual").max_by_key(|b| b["created_at"].as_str().unwrap().to_string()).unwrap()["id"].as_str().unwrap().to_string()
    }
    async fn row(&self, sid: &str, bid: &str) -> Value { self.get(&format!("/servers/{sid}/backups")).await.as_array().unwrap().iter().find(|b| b["id"] == bid).cloned().unwrap_or(Value::Null) }
    async fn wait_remote(&self, sid: &str, bid: &str, want: &str) -> Value {
        until(&format!("remote status {want}"), 40, || async { let r = self.row(sid, bid).await; if r["remote"] == "failed" && want != "failed" { panic!("upload failed: {}", r["remote_error"]); } (r["remote"] == want).then_some(r) }).await
    }
}

fn read(p: &Path) -> String { std::fs::read_to_string(p).unwrap_or_default() }

#[tokio::test(flavor = "multi_thread")]
async fn folder_destination_retention_remote_restore_and_staged_promote() {
    let root = std::env::temp_dir().join(format!("moss-remote-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (data, nas) = (root.join("data"), root.join("nas"));
    std::fs::create_dir_all(&nas).unwrap();
    let port = free_port();
    let moss = Moss::start(&data, port);
    let a = Api { c: reqwest::Client::new(), base: moss.base.clone() };
    until("controller", 20, || async { a.c.get(format!("http://127.0.0.1:{port}/healthz")).send().await.ok().filter(|r| r.status().is_success()) }).await;

    let (s, created) = a.post("/servers", json!({"name": "remote", "runtime": "sim", "game_version": "sim-1.0", "memory_mb": 512, "port": free_port()})).await;
    assert_eq!(s, 202, "{created}");
    let sid = created["server"]["id"].as_str().unwrap().to_string();
    assert_eq!(a.op(&created).await["state"], "completed");
    let live: PathBuf = data.join("servers").join(&sid);
    std::fs::create_dir_all(live.join("world")).unwrap();
    std::fs::write(live.join("world/level.dat"), "level v1").unwrap();
    std::fs::write(live.join("server.properties"), "motd=one\n").unwrap();

    // ---- destination validation: never inside the data dir, must exist
    let (s, e) = a.post("/backup-destinations", json!({"name": "bad", "kind": "folder", "path": live.to_string_lossy()})).await;
    assert_eq!((s, e["error"]["code"].as_str()), (400, Some("validation")), "{e}");
    assert_eq!(a.post("/backup-destinations", json!({"name": "bad", "kind": "folder", "path": nas.join("missing").to_string_lossy()})).await.0, 400);
    assert_eq!(a.post("/backup-destinations", json!({"name": "bad", "kind": "folder", "path": "relative"})).await.0, 400);

    // ---- add, test, list
    let (s, d) = a.post("/backup-destinations", json!({"name": "NAS", "kind": "folder", "path": nas.to_string_lossy(), "keep_local": 1, "keep_remote": 10})).await;
    assert_eq!(s, 201, "{d}");
    assert_eq!(d["is_default"], true, "the first destination becomes the default");
    let did = d["id"].as_str().unwrap().to_string();
    let (s, t) = a.post(&format!("/backup-destinations/{did}/test"), json!({})).await;
    assert_eq!((s, &t["ok"]), (200, &json!(true)), "{t}");
    assert_eq!(t["steps"].as_array().unwrap().len(), 4);
    assert!(std::fs::read_dir(nas.join("moss-backups/moss-test")).map(|d| d.count() == 0).unwrap_or(true), "test file cleaned up");

    // ---- backups are copied, verified, and local copies trimmed to the last 1
    let b1 = a.backup(&sid).await;
    let r1 = a.wait_remote(&sid, &b1, "stored").await;
    assert_eq!(r1["remote_destination"], "NAS");
    let obj1 = nas.join("moss-backups").join(&sid).join(format!("{b1}.tar.zst"));
    assert!(obj1.is_file());
    assert_eq!(std::fs::metadata(&obj1).unwrap().len() as i64, r1["size"].as_i64().unwrap());
    let b2 = a.backup(&sid).await;
    a.wait_remote(&sid, &b2, "stored").await;
    let r1 = until("b1 local copy dropped", 20, || async { let r = a.row(&sid, &b1).await; (r["local_present"] == false).then_some(r) }).await;
    assert_eq!(r1["remote"], "stored");
    assert!(!data.join("backups").join(&sid).join(format!("{b1}.tar.zst")).exists());
    assert_eq!(a.row(&sid, &b2).await["local_present"], true);

    // remote-only backups cannot be previewed or downloaded directly, but can be fetched
    let (s, e) = a.post(&format!("/backups/{b1}/preview"), json!({"mode": "in_place"})).await;
    assert_eq!((s, e["error"]["code"].as_str()), (409, Some("backup.remote_only")));
    assert_eq!(a.c.get(format!("{}/backups/{b1}/download", a.base)).send().await.unwrap().status(), 409);
    let (s, f) = a.post(&format!("/backups/{b1}/fetch"), json!({})).await;
    assert_eq!(s, 202, "{f}");
    assert_eq!(a.op(&f).await["state"], "completed");
    assert_eq!(a.row(&sid, &b1).await["local_present"], true);
    assert_eq!(a.c.get(format!("{}/backups/{b1}/download", a.base)).send().await.unwrap().status(), 200);

    // ---- restore a remote-only backup (b2 is dropped locally once b3 is stored)
    let b3 = a.backup(&sid).await;
    a.wait_remote(&sid, &b3, "stored").await;
    until("b2 local copy dropped", 20, || async { (a.row(&sid, &b2).await["local_present"] == false).then_some(()) }).await;
    std::fs::write(live.join("world/level.dat"), "level v2 (newer)").unwrap();
    let (s, r) = a.post(&format!("/backups/{b2}/restore"), json!({"mode": "in_place"})).await;
    assert_eq!(s, 202, "{r}");
    let o = a.op(&r).await;
    assert_eq!(o["state"], "completed", "{o}");
    assert_eq!(read(&live.join("world/level.dat")), "level v1", "restored from the remote copy");
    assert_eq!(a.row(&sid, &b2).await["local_present"], true, "the downloaded copy is kept");

    // ---- a corrupted remote copy is refused before anything is touched
    until("b1 local copy dropped again", 20, || async { let _ = a.backup(&sid).await; (a.row(&sid, &b1).await["local_present"] == false).then_some(()) }).await;
    let mut bytes = std::fs::read(&obj1).unwrap();
    let mid = bytes.len() / 2; bytes[mid] ^= 0xff;
    std::fs::write(&obj1, &bytes).unwrap();
    std::fs::write(live.join("world/level.dat"), "level v3 (must survive)").unwrap();
    let (_, r) = a.post(&format!("/backups/{b1}/restore"), json!({"mode": "in_place"})).await;
    let o = a.op(&r).await;
    assert_eq!(o["state"], "failed");
    assert!(o["error"].as_str().unwrap().contains("SHA-256"), "{o}");
    assert_eq!(read(&live.join("world/level.dat")), "level v3 (must survive)");
    assert_eq!(a.row(&sid, &b1).await["local_present"], false, "a corrupt download is discarded");
    assert!(std::fs::read_dir(data.join("backups").join(&sid)).unwrap().all(|e| { let n = e.unwrap().file_name().to_string_lossy().to_string(); !n.ends_with(".fetch") && !n.ends_with(".part") }));

    // ---- staged restore: compare first, promote explicitly
    let (s, r) = a.post(&format!("/backups/{b2}/restore"), json!({"mode": "in_place", "staged": true})).await;
    assert_eq!(s, 202, "{r}");
    let staged = r["staged_id"].as_str().unwrap().to_string();
    assert_eq!(a.op(&r).await["state"], "completed");
    assert_eq!(read(&live.join("world/level.dat")), "level v3 (must survive)", "staging does not touch the live server");
    let d = a.get(&format!("/staged-restores/{staged}")).await;
    assert_eq!(d["status"], "ready", "{d}");
    assert_eq!(d["can_promote"], true, "{d}");
    assert!(d["diff"]["changed"].as_i64().unwrap() >= 1 && d["diff"]["samples"]["changed"].to_string().contains("level.dat"), "{d}");
    assert!(d["checks"].as_array().unwrap().iter().all(|c| c["ok"] == true));
    assert_eq!(a.get(&format!("/servers/{sid}/staged-restores")).await.as_array().unwrap().len(), 1);
    // a failed pre-promotion check blocks the swap and changes nothing
    let stage_dir = data.join("servers").join(format!(".staged-{staged}"));
    let hidden = stage_dir.join("world/level.dat.hidden");
    std::fs::rename(stage_dir.join("world/level.dat"), &hidden).unwrap();
    let d = a.get(&format!("/staged-restores/{staged}")).await;
    assert_eq!(d["can_promote"], false);
    assert!(d["checks"].to_string().contains("level.dat"), "{d}");
    let (s, p) = a.post(&format!("/staged-restores/{staged}/promote"), json!({})).await;
    assert_eq!(s, 202);
    let o = a.op(&p).await;
    assert_eq!(o["state"], "failed", "{o}");
    assert_eq!(read(&live.join("world/level.dat")), "level v3 (must survive)");
    std::fs::rename(&hidden, stage_dir.join("world/level.dat")).unwrap();
    // now promote for real: the old directory is kept in removed/
    let (_, p) = a.post(&format!("/staged-restores/{staged}/promote"), json!({})).await;
    let o = a.op(&p).await;
    assert_eq!(o["state"], "completed", "{o}");
    assert_eq!(read(&live.join("world/level.dat")), "level v1");
    let removed: Vec<_> = std::fs::read_dir(data.join("removed")).unwrap().map(|e| e.unwrap().path()).filter(|p| p.to_string_lossy().contains("pre-restore")).collect();
    assert!(removed.iter().any(|p| read(&p.join("world/level.dat")) == "level v3 (must survive)"), "{removed:?}");
    assert!(!stage_dir.exists());
    assert_eq!(a.get(&format!("/servers/{sid}/staged-restores")).await.as_array().unwrap().len(), 0);
    // a staged restore can be discarded
    let (_, r) = a.post(&format!("/backups/{b2}/restore"), json!({"mode": "in_place", "staged": true})).await;
    let staged2 = r["staged_id"].as_str().unwrap().to_string();
    a.op(&r).await;
    assert_eq!(a.send(reqwest::Method::DELETE, &format!("/staged-restores/{staged2}"), json!({})).await.0, 204);
    assert!(!data.join("servers").join(format!(".staged-{staged2}")).exists());
    assert_eq!(a.post(&format!("/backups/{b2}/restore"), json!({"mode": "new_server", "staged": true})).await.0, 400);

    // ---- per-server override: "none" keeps backups local
    let (s, v) = a.send(reqwest::Method::PUT, &format!("/servers/{sid}/backup-destination"), json!({"destination_id": "none"})).await;
    assert_eq!((s, v["destination_id"].as_str(), v["effective"].is_null()), (200, Some("none"), true), "{v}");
    let b5 = a.backup(&sid).await;
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(a.row(&sid, &b5).await["remote"], "none");
    assert_eq!(a.send(reqwest::Method::PUT, &format!("/servers/{sid}/backup-destination"), json!({"destination_id": "default"})).await.1["effective"]["name"], "NAS");
    // a local backup can still be copied afterwards
    let (s, u) = a.post(&format!("/backups/{b5}/upload"), json!({})).await;
    assert_eq!(s, 202, "{u}");
    a.wait_remote(&sid, &b5, "stored").await;

    // ---- deleting a backup deletes its remote copy; removing a destination in use needs confirmation
    let obj5 = nas.join("moss-backups").join(&sid).join(format!("{b5}.tar.zst"));
    assert!(obj5.is_file());
    assert_eq!(a.send(reqwest::Method::DELETE, &format!("/backups/{b5}"), json!({})).await.0, 204);
    assert!(!obj5.exists());
    assert_eq!(a.send(reqwest::Method::DELETE, &format!("/backup-destinations/{did}"), json!({})).await.0, 409);

    // ---- S3 credentials: write-only, kept out of the database, readable only by the owner
    let (s, s3) = a.post("/backup-destinations", json!({"name": "B2", "kind": "s3", "endpoint": "https://s3.us-west-004.backblazeb2.com", "region": "us-west-004", "bucket": "my-moss-backups", "path_style": true, "access_key": "AKIDEXAMPLE0123456789", "secret_key": "s3cr3t-VALUE-that-must-not-leak-0123456789"})).await;
    assert_eq!(s, 201, "{s3}");
    assert_eq!((s3["access_key"].as_str(), s3["secret_key"].as_str()), (Some("set"), Some("set")));
    let listing = a.get("/backup-destinations").await.to_string();
    assert!(!listing.contains("s3cr3t") && !listing.contains("AKIDEXAMPLE"), "{listing}");
    let secrets_file = data.join("backup-secrets.json");
    assert!(read(&secrets_file).contains("s3cr3t-VALUE"));
    #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; assert_eq!(std::fs::metadata(&secrets_file).unwrap().permissions().mode() & 0o777, 0o600); }
    for f in std::fs::read_dir(&data).unwrap().map(|e| e.unwrap().path()).filter(|p| p.file_name().unwrap().to_string_lossy().starts_with("moss.sqlite")) {
        assert!(!String::from_utf8_lossy(&std::fs::read(&f).unwrap()).contains("s3cr3t-VALUE"), "secret found in {}", f.display());
    }
    let (_, bundle_preview) = (0, a.get("/support-bundle?preview=1").await);
    assert!(bundle_preview["files"].to_string().contains("backup-destinations.json"));
    let zip = a.c.get(format!("{}/support-bundle", a.base)).send().await.unwrap().bytes().await.unwrap();
    assert!(!String::from_utf8_lossy(&zip).contains("s3cr3t-VALUE"));
    // http and private endpoints are refused by default
    let (s, e) = a.post("/backup-destinations", json!({"name": "x", "kind": "s3", "endpoint": "http://example.com", "region": "us-east-1", "bucket": "bkt", "path_style": true, "access_key": "a", "secret_key": "b"})).await;
    assert_eq!((s, e["error"]["fields"][0]["path"].as_str()), (400, Some("endpoint")));
    let (_, t) = a.post(&format!("/backup-destinations/{}/test", s3["id"].as_str().unwrap()), json!({})).await;
    let _ = t; // a real endpoint is not contacted in this test environment; the unit tests cover the S3 protocol against a fake
    assert_eq!(a.send(reqwest::Method::DELETE, &format!("/backup-destinations/{}", s3["id"].as_str().unwrap()), json!({})).await.0, 204);
    assert!(!read(&secrets_file).contains("s3cr3t-VALUE"), "removing a destination removes its secrets");

    // force-removing the folder destination keeps files on the NAS but forgets them
    assert_eq!(a.send(reqwest::Method::DELETE, &format!("/backup-destinations/{did}?force=true"), json!({})).await.0, 204);
    assert!(obj1.exists());
    assert_eq!(a.row(&sid, &b1).await, Value::Null, "remote-only rows disappear");
    let _ = std::fs::remove_dir_all(&root);
}
