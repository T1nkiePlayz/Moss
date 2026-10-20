//! Importing an existing server over HTTP: from a host path (copied, original untouched) and from an uploaded zip.
use serde_json::{Value, json};
use std::{io::Write, net::TcpListener, path::Path, process::{Child, Command, Stdio}, time::{Duration, Instant}};

struct Moss(Child);
impl Drop for Moss { fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); } }

fn make_jar(path: &Path, files: &[(&str, &str)]) {
    let mut z = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    for (n, c) in files { z.start_file(*n, zip::write::SimpleFileOptions::default()).unwrap(); z.write_all(c.as_bytes()).unwrap(); }
    z.finish().unwrap();
}

async fn wait_op(c: &reqwest::Client, api: &str, id: &str) -> Value {
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        let ops: Value = c.get(format!("{api}/operations")).send().await.unwrap().json().await.unwrap();
        if let Some(o) = ops.as_array().unwrap().iter().find(|o| o["id"] == id) && matches!(o["state"].as_str(), Some("completed" | "failed" | "cancelled")) { return o.clone(); }
        assert!(Instant::now() < end, "operation did not finish");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn import_from_path_and_upload() {
    let base = std::env::temp_dir().join(format!("moss-import-it-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let (dir, src) = (base.join("data"), base.join("existing-server"));
    std::fs::create_dir_all(src.join("world")).unwrap();
    std::fs::create_dir_all(src.join("config")).unwrap();
    make_jar(&src.join("paper-1.21.4-100.jar"), &[("patch.properties", "version=1.21.4\n")]);
    std::fs::write(src.join("server.properties"), "motd=Imported\nserver-port=25811\n").unwrap();
    std::fs::write(src.join("config/paper-global.yml"), "_version: 29\n").unwrap();
    std::fs::write(src.join("world/level.dat"), b"world").unwrap();
    std::os::unix::fs::symlink("/etc/passwd", src.join("evil-link")).unwrap();

    let moss = |args: &[&str], port: Option<u16>| {
        let mut c = Command::new(env!("CARGO_BIN_EXE_moss"));
        c.args(args).env("MOSS_DATA_DIR", &dir).env("MOSS_AUTH", "dev").env("RUST_LOG", "moss=warn").stdout(Stdio::null()).stderr(Stdio::null());
        if let Some(p) = port { c.env("MOSS_PORT", p.to_string()); }
        c
    };
    assert!(moss(&["doctor"], None).status().unwrap().success());
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let _m = Moss(moss(&["serve"], Some(port)).spawn().unwrap());
    let c = reqwest::Client::new();
    let api = format!("http://127.0.0.1:{port}/api/v1");
    let end = Instant::now() + Duration::from_secs(20);
    while c.get(format!("http://127.0.0.1:{port}/healthz")).send().await.is_err() { assert!(Instant::now() < end, "controller did not start"); tokio::time::sleep(Duration::from_millis(100)).await; }

    // refusals: relative, traversal, missing, system, Moss's own data, not a server
    for p in ["relative/path", "/nonexistent-moss-dir", "/etc", dir.to_str().unwrap(), "/tmp/../etc", base.to_str().unwrap()] {
        let r = c.post(format!("{api}/servers/import/inspect")).json(&json!({"path": p})).send().await.unwrap();
        assert_eq!(r.status(), 400, "{p}");
    }

    let det: Value = c.post(format!("{api}/servers/import/inspect")).json(&json!({"path": src.to_str().unwrap()})).send().await.unwrap().json().await.unwrap();
    assert_eq!((det["runtime"].as_str(), det["game_version"].as_str(), det["port"].as_i64(), det["has_world"].as_bool()), (Some("paper"), Some("1.21.4"), Some(25811), Some(true)), "{det}");

    // EULA is required
    let r = c.post(format!("{api}/servers/import")).json(&json!({"name": "Imported", "path": src.to_str().unwrap()})).send().await.unwrap();
    assert_eq!(r.status(), 400);
    let r = c.post(format!("{api}/servers/import")).json(&json!({"name": "Imported", "path": src.to_str().unwrap(), "accept_eula": true})).send().await.unwrap();
    assert_eq!(r.status(), 202);
    let out: Value = r.json().await.unwrap();
    let (sid, op) = (out["server"]["id"].as_str().unwrap().to_string(), out["operation"]["id"].as_str().unwrap().to_string());
    assert_eq!(out["server"]["port"], 25811);
    let done = wait_op(&c, &api, &op).await;
    assert_eq!(done["state"], "completed", "{done}");
    let sdir = dir.join("servers").join(&sid);
    assert!(sdir.join("world/level.dat").is_file() && sdir.join("config/paper-global.yml").is_file());
    assert!(sdir.join("server.jar").is_file(), "launcher normalised");
    assert!(!sdir.join("evil-link").exists(), "symlinks are never followed");
    assert!(sdir.join("eula.txt").is_file());
    assert!(src.join("world/level.dat").is_file() && src.join("paper-1.21.4-100.jar").is_file() && !src.join("server.jar").exists(), "the original is untouched");
    let s: Value = c.get(format!("{api}/servers/{sid}")).send().await.unwrap().json().await.unwrap();
    assert_eq!((s["runtime"].as_str(), s["game_version"].as_str()), (Some("paper"), Some("1.21.4")));

    // same port again is refused with a clear message
    let r = c.post(format!("{api}/servers/import")).json(&json!({"name": "Twice", "path": src.to_str().unwrap(), "accept_eula": true})).send().await.unwrap();
    assert_eq!(r.status(), 409);

    // upload a zip (server folder inside one top-level folder)
    let zp = base.join("upload.zip");
    {
        let mut z = zip::ZipWriter::new(std::fs::File::create(&zp).unwrap());
        let o = zip::write::SimpleFileOptions::default();
        z.start_file("mypack/server.properties", o).unwrap(); z.write_all(b"server-port=25812\n").unwrap();
        z.start_file("mypack/world/level.dat", o).unwrap(); z.write_all(b"w").unwrap();
        let mut jar = std::io::Cursor::new(vec![]);
        { let mut j = zip::ZipWriter::new(&mut jar); j.start_file("version.json", o).unwrap(); j.write_all(br#"{"id":"1.20.4"}"#).unwrap(); j.start_file("net/minecraft/bundler/Main.class", o).unwrap(); j.write_all(b"x").unwrap(); j.finish().unwrap(); }
        z.start_file("mypack/server.jar", o).unwrap(); z.write_all(jar.get_ref()).unwrap();
        z.finish().unwrap();
    }
    let multipart = |name: &str, bytes: Vec<u8>| {
        let mut body = format!("--XB\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\nContent-Type: application/zip\r\n\r\n").into_bytes();
        body.extend(bytes);
        body.extend(b"\r\n--XB--\r\n");
        c.post(format!("{api}/servers/import/inspect")).header("content-type", "multipart/form-data; boundary=XB").body(body)
    };
    let det: Value = multipart("mypack.zip", std::fs::read(&zp).unwrap()).send().await.unwrap().json().await.unwrap();
    assert_eq!((det["runtime"].as_str(), det["game_version"].as_str(), det["port"].as_i64()), (Some("vanilla"), Some("1.20.4"), Some(25812)), "{det}");
    let token = det["token"].as_str().unwrap().to_string();
    let r = c.post(format!("{api}/servers/import")).json(&json!({"name": "From zip", "token": token, "accept_eula": true})).send().await.unwrap();
    assert_eq!(r.status(), 202);
    let out: Value = r.json().await.unwrap();
    let done = wait_op(&c, &api, out["operation"]["id"].as_str().unwrap()).await;
    assert_eq!(done["state"], "completed", "{done}");
    assert!(dir.join("servers").join(out["server"]["id"].as_str().unwrap()).join("world/level.dat").is_file());
    // the token is single use
    let r = c.post(format!("{api}/servers/import")).json(&json!({"name": "Again", "token": token, "accept_eula": true})).send().await.unwrap();
    assert_eq!(r.status(), 400);

    // a zip that tries to escape is refused and leaves nothing behind
    let evil = base.join("evil.zip");
    { let mut z = zip::ZipWriter::new(std::fs::File::create(&evil).unwrap()); z.start_file("../../escape.txt", zip::write::SimpleFileOptions::default()).unwrap(); z.write_all(b"x").unwrap(); z.finish().unwrap(); }
    assert_eq!(multipart("evil.zip", std::fs::read(&evil).unwrap()).send().await.unwrap().status(), 400);
    assert!(!dir.join("escape.txt").exists() && !base.join("escape.txt").exists());
    assert_eq!(multipart("junk.zip", b"not an archive".to_vec()).send().await.unwrap().status(), 400);
    let _ = std::fs::remove_dir_all(&base);
}
