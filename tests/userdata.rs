//! Preferences, saved views and blueprints over HTTP against the real binary.
use serde_json::{Value, json};
use std::{net::TcpListener, process::{Child, Command, Stdio}, time::{Duration, Instant}};

struct Moss(Child);
impl Drop for Moss { fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); } }

#[tokio::test(flavor = "multi_thread")]
async fn prefs_views_and_blueprints() {
    let dir = std::env::temp_dir().join(format!("moss-ud-it-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let moss = |args: &[&str], port: Option<u16>| {
        let mut c = Command::new(env!("CARGO_BIN_EXE_moss"));
        c.args(args).env("MOSS_DATA_DIR", &dir).env("MOSS_AUTH", "dev").env("RUST_LOG", "moss=warn").stdout(Stdio::null()).stderr(Stdio::null());
        if let Some(p) = port { c.env("MOSS_PORT", p.to_string()); }
        c
    };
    assert!(moss(&["doctor"], None).status().unwrap().success());
    let sid = "bp-server-1";
    let sdir = dir.join("servers").join(sid);
    std::fs::create_dir_all(&sdir).unwrap();
    std::fs::create_dir_all(sdir.join("plugins/Chunky")).unwrap();
    std::fs::write(sdir.join("plugins/Chunky/chunky-settings.yml"), b"x").unwrap();
    std::fs::write(sdir.join("server.properties"), "#c\nmotd=Hello\nserver-port=25999\nrcon.password=hunter2\nlevel-seed=123\nview-distance=9\n").unwrap();
    {
        let c = rusqlite::Connection::open(dir.join("moss.sqlite")).unwrap();
        c.execute("INSERT INTO servers(id,name,runtime,game_version,port,memory_mb,installed,created_at) VALUES (?,?,?,?,?,?,1,'2026-01-01T00:00:00Z')", rusqlite::params![sid, "Source", "paper", "1.21.4", 25999, 3072]).unwrap();
        c.execute("INSERT INTO schedules(id,server_id,kind,daily_at,tz,enabled) VALUES ('s1',?,'backup','04:00','UTC',1)", [sid]).unwrap();
        c.execute("INSERT INTO content_items(id,server_id,provider,project_id,title,filename,kind,installed_at,dir) VALUES ('c1',?,'modrinth','AABBCCDD','Chunky','chunky.jar','plugins','2026-01-01T00:00:00Z','plugins')", [sid]).unwrap();
        c.execute("INSERT INTO content_items(id,server_id,provider,project_id,title,filename,kind,installed_at,dir) VALUES ('c2',?,'unknown',NULL,'Mystery','m.jar','plugins','2026-01-01T00:00:00Z','plugins')", [sid]).unwrap();
    }
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let _m = Moss(moss(&["serve"], Some(port)).spawn().unwrap());
    let c = reqwest::Client::new();
    let api = format!("http://127.0.0.1:{port}/api/v1");
    let end = Instant::now() + Duration::from_secs(20);
    while c.get(format!("http://127.0.0.1:{port}/healthz")).send().await.is_err() { assert!(Instant::now() < end, "controller did not start"); tokio::time::sleep(Duration::from_millis(100)).await; }

    // search: grouped, escaped, and empty for short queries
    let r: Value = c.get(format!("{api}/search?q=chunky")).send().await.unwrap().json().await.unwrap();
    assert_eq!(r["content"][0]["title"], "Chunky");
    assert_eq!(r["content"][0]["server_name"], "Source");
    assert_eq!(r["files"][0]["path"], "plugins/Chunky/chunky-settings.yml");
    let r: Value = c.get(format!("{api}/search?q=source")).send().await.unwrap().json().await.unwrap();
    assert_eq!(r["servers"][0]["id"], sid);
    for q in ["x", "%25%25", "zzzzzz"] {
        let r: Value = c.get(format!("{api}/search?q={q}")).send().await.unwrap().json().await.unwrap();
        assert!(["servers", "backups", "content", "files"].iter().all(|k| r[k].as_array().unwrap().is_empty()), "{q}: {r}");
    }
    assert_eq!(c.get(format!("{api}/search?q={}", "a".repeat(100))).send().await.unwrap().status(), 400);

    // preferences
    let p: Value = c.get(format!("{api}/me/preferences")).send().await.unwrap().json().await.unwrap();
    assert_eq!(p["data"], json!({}));
    let r = c.put(format!("{api}/me/preferences")).json(&json!({"data": {"theme": {"light": {"accent": "#2f6f3e"}}}})).send().await.unwrap();
    assert!(r.status().is_success());
    let p: Value = c.get(format!("{api}/me/preferences")).send().await.unwrap().json().await.unwrap();
    assert_eq!(p["data"]["theme"]["light"]["accent"], "#2f6f3e");
    assert_eq!(c.put(format!("{api}/me/preferences")).json(&json!({"data": [1]})).send().await.unwrap().status(), 400);
    assert_eq!(c.put(format!("{api}/me/preferences")).json(&json!({"data": {"x": "a".repeat(40000)}})).send().await.unwrap().status(), 400);

    // saved views
    let v: Value = c.post(format!("{api}/views")).json(&json!({"scope": "servers", "name": "Running", "filters": {"state": "running"}, "sort": "name"})).send().await.unwrap().json().await.unwrap();
    let vid = v["id"].as_str().unwrap().to_string();
    c.post(format!("{api}/views")).json(&json!({"scope": "content", "name": "Updates", "filters": {"updates": true}})).send().await.unwrap();
    let l: Value = c.get(format!("{api}/views?scope=servers")).send().await.unwrap().json().await.unwrap();
    assert_eq!(l.as_array().unwrap().len(), 1);
    assert_eq!(l[0]["filters"]["state"], "running");
    // same name replaces
    c.post(format!("{api}/views")).json(&json!({"scope": "servers", "name": "running", "filters": {"state": "stopped"}})).send().await.unwrap();
    let l: Value = c.get(format!("{api}/views?scope=servers")).send().await.unwrap().json().await.unwrap();
    assert_eq!((l.as_array().unwrap().len(), l[0]["filters"]["state"].as_str()), (1, Some("stopped")));
    for bad in [json!({"scope": "nope", "name": "x"}), json!({"scope": "servers", "name": ""}), json!({"scope": "servers", "name": "x", "filters": [1]})] {
        assert_eq!(c.post(format!("{api}/views")).json(&bad).send().await.unwrap().status(), 400, "{bad}");
    }
    assert_eq!(c.delete(format!("{api}/views/{vid}")).send().await.unwrap().status(), 404, "replaced view is gone");

    // blueprints: no secrets, seeds or ports
    let b: Value = c.post(format!("{api}/blueprints")).json(&json!({"server_id": sid, "name": "Friends template"})).send().await.unwrap().json().await.unwrap();
    let bid = b["id"].as_str().unwrap().to_string();
    assert_eq!((b["content"].as_i64(), b["schedules"].as_i64(), b["properties"].as_i64()), (Some(1), Some(1), Some(2)));
    let ex = c.get(format!("{api}/blueprints/{bid}/export")).send().await.unwrap().text().await.unwrap();
    for leak in ["hunter2", "123", "25999", "rcon", "level-seed"] { assert!(!ex.contains(leak) || leak == "123", "blueprint leaked {leak}: {ex}"); }
    let exv: Value = serde_json::from_str(&ex).unwrap();
    assert_eq!(exv["properties"]["motd"], "Hello");
    assert_eq!(exv["memory_mb"], 3072);
    // import round trip and refusals
    let imp = c.post(format!("{api}/blueprints/import")).body(ex.clone()).send().await.unwrap();
    assert_eq!(imp.status(), 201);
    assert_eq!(c.post(format!("{api}/blueprints/import")).body("not json").send().await.unwrap().status(), 400);
    assert_eq!(c.post(format!("{api}/blueprints/import")).body(ex.replace("moss-blueprint", "other")).send().await.unwrap().status(), 400);
    let list: Value = c.get(format!("{api}/blueprints")).send().await.unwrap().json().await.unwrap();
    assert_eq!(list.as_array().unwrap().len(), 2);
    assert_eq!(c.post(format!("{api}/blueprints/{bid}/create")).json(&json!({"name": "", "accept_eula": true})).send().await.unwrap().status(), 400);
    assert_eq!(c.post(format!("{api}/blueprints/{bid}/create")).json(&json!({"name": "New", "accept_eula": false})).send().await.unwrap().status(), 400, "EULA still required");
    assert_eq!(c.delete(format!("{api}/blueprints/{bid}")).send().await.unwrap().status(), 204);
    assert_eq!(c.get(format!("{api}/blueprints/{bid}/export")).send().await.unwrap().status(), 404);
    let _ = std::fs::remove_dir_all(&dir);
}
