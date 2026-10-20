//! A data directory written by the previous release (database at schema version 6, built here from the old
//! migration files) must open with the current binary: the data survives, the schema is brought up to date, and the
//! API serves it.
use rusqlite::Connection;
use serde_json::Value;
use std::{net::TcpListener, process::{Command, Stdio}, time::{Duration, Instant}};

const OLD: [&str; 6] = [
    include_str!("../src/db/migrations/0001_init.sql"), include_str!("../src/db/migrations/0002_limits.sql"), include_str!("../src/db/migrations/0003_ops.sql"),
    include_str!("../src/db/migrations/0004_content.sql"), include_str!("../src/db/migrations/0005_tokens.sql"), include_str!("../src/db/migrations/0006_modpacks.sql"),
];

#[tokio::test(flavor = "multi_thread")]
async fn a_release_0_2_data_directory_upgrades_and_boots() {
    let data = std::env::temp_dir().join(format!("moss-upgrade-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data);
    std::fs::create_dir_all(data.join("servers/s1")).unwrap();
    std::fs::write(data.join("servers/s1/server.properties"), "motd=hi\n").unwrap();
    let db = data.join("moss.sqlite");
    {
        let c = Connection::open(&db).unwrap();
        for (i, sql) in OLD.iter().enumerate() { c.execute_batch(&format!("BEGIN;\n{sql}\nPRAGMA user_version = {};\nCOMMIT;", i + 1)).unwrap(); }
        for q in [
            "INSERT INTO servers(id,name,runtime,game_version,port,memory_mb,created_at,installed) VALUES('s1','Old survival','paper','1.21.1',25570,2048,'2026-01-01T00:00:00Z',1)",
            "INSERT INTO backups(id,server_id,created_at,size,sha256,status,kind,server_name,locked,worlds) VALUES('b1','s1','2026-01-01T00:00:00Z',10,'abc','ok','manual','Old survival',1,'[\"world\"]')",
            "INSERT INTO schedules(id,server_id,kind,every_minutes,tz) VALUES('sc1','s1','backup',60,'UTC')",
            "INSERT INTO meta(k,v) VALUES('backup_keep','4')",
            "INSERT INTO notifications(id,time,kind,severity,title,server_id) VALUES('n1',strftime('%Y-%m-%dT%H:%M:%fZ','now'),'crash','warn','Old crash','s1')", // recent: old ones are pruned
        ] { c.execute(q, []).unwrap(); }
    }
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let mut child = Command::new(env!("CARGO_BIN_EXE_moss")).arg("serve").env("MOSS_AUTH", "dev").env("MOSS_PORT", port.to_string()).env("MOSS_DATA_DIR", &data)
        .env("RUST_LOG", "moss=warn").stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    let c = reqwest::Client::new();
    let base = format!("http://127.0.0.1:{port}/api/v1");
    let end = Instant::now() + Duration::from_secs(30);
    while c.get(format!("http://127.0.0.1:{port}/healthz")).send().await.map(|r| !r.status().is_success()).unwrap_or(true) { assert!(Instant::now() < end, "the upgraded data directory did not boot"); tokio::time::sleep(Duration::from_millis(200)).await; }
    let servers: Value = c.get(format!("{base}/servers")).send().await.unwrap().json().await.unwrap();
    assert_eq!(servers.as_array().unwrap()[0]["name"], "Old survival");
    let backups: Value = c.get(format!("{base}/backups")).send().await.unwrap().json().await.unwrap();
    let b = &backups.as_array().unwrap()[0];
    assert_eq!((b["id"].as_str(), b["locked"].as_bool(), b["remote"].as_str(), b["local_present"].as_bool()), (Some("b1"), Some(true), Some("none"), Some(true)));
    let settings: Value = c.get(format!("{base}/settings")).send().await.unwrap().json().await.unwrap();
    assert_eq!(settings["backup_keep"], 4);
    assert!(c.get(format!("{base}/backup-destinations")).send().await.unwrap().status().is_success());
    let n: Value = c.get(format!("{base}/notifications")).send().await.unwrap().json().await.unwrap();
    assert!(n.to_string().contains("Old crash"), "{n}");
    let _ = child.kill(); let _ = child.wait();
    let v: i64 = Connection::open(&db).unwrap().query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
    assert!(v > 6, "schema version was {v}");
    let _ = std::fs::remove_dir_all(&data);
}
