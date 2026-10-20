//! The configuration editor over HTTP against the real binary: structured edits keep comments, every write is
//! undoable, invalid input is refused, and file ids cannot escape the server directory.
use serde_json::{Value, json};
use std::{net::TcpListener, process::{Child, Command, Stdio}, time::{Duration, Instant}};

struct Moss(Child);
impl Drop for Moss { fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); } }

const PAPER_GLOBAL: &str = "# Paper global\n_version: 29\n\nmisc:\n  # keep me\n  max-joins-per-tick: 3 # default\n";

#[tokio::test(flavor = "multi_thread")]
async fn config_editor_round_trip() {
    let dir = std::env::temp_dir().join(format!("moss-cfg-it-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let moss = |args: &[&str], port: Option<u16>| {
        let mut c = Command::new(env!("CARGO_BIN_EXE_moss"));
        c.args(args).env("MOSS_DATA_DIR", &dir).env("MOSS_AUTH", "dev").env("RUST_LOG", "moss=warn").stdout(Stdio::null()).stderr(Stdio::null());
        if let Some(p) = port { c.env("MOSS_PORT", p.to_string()); }
        c
    };
    assert!(moss(&["doctor"], None).status().unwrap().success()); // creates the database
    let sid = "cfg-server-1";
    let sdir = dir.join("servers").join(sid);
    std::fs::create_dir_all(sdir.join("config")).unwrap();
    std::fs::write(sdir.join("config/paper-global.yml"), PAPER_GLOBAL).unwrap();
    std::fs::create_dir_all(sdir.join("world")).unwrap();
    std::fs::write(sdir.join("world/level.dat"), b"x").unwrap();
    {
        let c = rusqlite::Connection::open(dir.join("moss.sqlite")).unwrap();
        c.execute("INSERT INTO servers(id,name,runtime,game_version,port,memory_mb,installed,created_at) VALUES (?,?,?,?,?,?,1,'2026-01-01T00:00:00Z')", rusqlite::params![sid, "Paper test", "paper", "1.21.4", 25999, 1024]).unwrap();
    }
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let _m = Moss(moss(&["serve"], Some(port)).spawn().unwrap());
    let c = reqwest::Client::new();
    let base = format!("http://127.0.0.1:{port}/api/v1/servers/{sid}/config");
    let end = Instant::now() + Duration::from_secs(20);
    while c.get(format!("http://127.0.0.1:{port}/healthz")).send().await.is_err() { assert!(Instant::now() < end, "controller did not start"); tokio::time::sleep(Duration::from_millis(100)).await; }

    let list: Value = c.get(&base).send().await.unwrap().json().await.unwrap();
    let ids: Vec<&str> = list["files"].as_array().unwrap().iter().map(|f| f["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["paper-global", "paper-world-defaults", "spigot", "bukkit", "world--world"]);
    assert_eq!(list["files"][0]["exists"], true);
    assert_eq!(list["files"][2]["exists"], false);

    let f: Value = c.get(format!("{base}/paper-global")).send().await.unwrap().json().await.unwrap();
    assert_eq!(f["raw"], PAPER_GLOBAL);
    let joins = f["fields"].as_array().unwrap().iter().find(|x| x["key"] == "misc.max-joins-per-tick").unwrap();
    assert_eq!((joins["value"].as_i64(), joins["present"].as_bool()), (Some(3), Some(true)));

    // structured edit keeps the comments and takes a snapshot
    let r = c.put(format!("{base}/paper-global")).json(&json!({"set": {"misc.max-joins-per-tick": 6, "console.enable-brigadier-highlighting": false}})).send().await.unwrap();
    assert!(r.status().is_success(), "{}", r.text().await.unwrap());
    let on_disk = std::fs::read_to_string(sdir.join("config/paper-global.yml")).unwrap();
    assert!(on_disk.contains("# Paper global") && on_disk.contains("# keep me") && on_disk.contains("max-joins-per-tick: 6 # default") && on_disk.contains("enable-brigadier-highlighting: false"), "{on_disk}");
    let d: Value = c.get(format!("{base}/paper-global/diff")).send().await.unwrap().json().await.unwrap();
    assert_eq!(d["previous"], PAPER_GLOBAL);
    assert_eq!(d["current"], on_disk.as_str());
    let hist = c.get(format!("{base}/paper-global")).send().await.unwrap().json::<Value>().await.unwrap();
    assert_eq!(hist["history"].as_array().unwrap().len(), 1);

    // refusals: wrong type, unknown key, broken YAML, both modes at once, missing file
    for (file, body, status) in [
        ("paper-global", json!({"set": {"misc.max-joins-per-tick": "many"}}), 400),
        ("paper-global", json!({"set": {"misc.max-joins-per-tick": 0}}), 400),
        ("paper-global", json!({"set": {"nope.nothing": 1}}), 400),
        ("paper-global", json!({"raw": "a: [unclosed"}), 400),
        ("paper-global", json!({"raw": "a: 1", "set": {}}), 400),
        ("spigot", json!({"set": {"commands.log": false}}), 409),
        ("purpur", json!({"raw": "a: 1"}), 404),
        ("world--..%2F..%2Fetc", json!({"raw": "a: 1"}), 404),
    ] {
        let r = c.put(format!("{base}/{file}")).json(&body).send().await.unwrap();
        assert_eq!(r.status().as_u16(), status, "{file} {body}");
    }
    assert_eq!(std::fs::read_to_string(sdir.join("config/paper-global.yml")).unwrap(), on_disk, "refused edits change nothing");

    // raw edit, then revert through the shared undo endpoint
    let r: Value = c.put(format!("{base}/paper-global")).json(&json!({"raw": "# replaced\nmisc:\n  max-joins-per-tick: 9\n"})).send().await.unwrap().json().await.unwrap();
    let undo_id = r["undo_id"].as_str().unwrap().to_string();
    assert!(c.post(format!("http://127.0.0.1:{port}/api/v1/undo/{undo_id}/revert")).send().await.unwrap().status().is_success());
    assert_eq!(std::fs::read_to_string(sdir.join("config/paper-global.yml")).unwrap(), on_disk);
    let _ = std::fs::remove_dir_all(&dir);
}
