// To change the schema add migrations/NNNN_name.sql and one line to MIGRATIONS; never edit an applied migration.
//! Numbered schema migrations, tracked with SQLite's `PRAGMA user_version`. Each migration is applied once, in a
//! transaction. Migration 0001 is entirely `IF NOT EXISTS`, so databases created before the runner existed
//! (schema present, `user_version` 0) are adopted without change.
use rusqlite::Connection;

const MIGRATIONS: &[(&str, &str)] = &[
    ("0001_init", include_str!("migrations/0001_init.sql")),
    ("0002_limits", include_str!("migrations/0002_limits.sql")),
    ("0003_ops", include_str!("migrations/0003_ops.sql")),
    ("0004_content", include_str!("migrations/0004_content.sql")),
    ("0005_tokens", include_str!("migrations/0005_tokens.sql")),
    ("0006_modpacks", include_str!("migrations/0006_modpacks.sql")),
    ("0007_container", include_str!("migrations/0007_container.sql")),
    ("0008_remote_backups", include_str!("migrations/0008_remote_backups.sql")),
    ("0009_staging_packs", include_str!("migrations/0009_staging_packs.sql")),
    ("0010_userdata", include_str!("migrations/0010_userdata.sql")),
    ("0012_event_webhooks", include_str!("migrations/0012_event_webhooks.sql")),
];

/// Number of migrations this build knows (the `user_version` of an up-to-date database).
pub fn latest() -> usize { MIGRATIONS.len() }

pub fn run(c: &Connection) -> rusqlite::Result<()> { run_with(c, MIGRATIONS) }

fn run_with(c: &Connection, migrations: &[(&str, &str)]) -> rusqlite::Result<()> {
    let applied: usize = c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?.max(0) as usize;
    for (i, (_, sql)) in migrations.iter().enumerate().skip(applied) {
        c.execute_batch(&format!("BEGIN;\n{sql}\nPRAGMA user_version = {};\nCOMMIT;", i + 1))
            .inspect_err(|_| { let _ = c.execute_batch("ROLLBACK"); })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn migrations_are_idempotent_and_versioned() {
        let c = Connection::open_in_memory().unwrap();
        run(&c).unwrap();
        run(&c).unwrap();
        let v: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(v as usize, MIGRATIONS.len());
    }

    const V6_ROWS: &[&str] = &[
        "INSERT INTO meta(k,v) VALUES('backup_keep','5')",
        "INSERT INTO users(id,email,role,linked,created_at) VALUES('u1','a@example.com','owner',1,'2026-01-01T00:00:00Z')",
        "INSERT INTO servers(id,name,runtime,game_version,port,memory_mb,created_at,installed,memory_limit_mb) VALUES('s1','Survival','paper','1.21.1',25565,2048,'2026-01-01T00:00:00Z',1,4096)",
        "INSERT INTO grants(server_id,user_id,bundle) VALUES('s1','u1','operator')",
        "INSERT INTO operations(id,kind,title,server_id,state,created_at) VALUES('o1','backup.create','Back up','s1','completed','2026-01-01T00:00:00Z')",
        "INSERT INTO activity(time,actor,action,target) VALUES('2026-01-01T00:00:00Z','a@example.com','server.create','Survival')",
        "INSERT INTO timeline(server_id,time,kind) VALUES('s1','2026-01-01T00:00:00Z','start')",
        "INSERT INTO content_items(id,server_id,provider,title,filename,kind,installed_at,dir,source) VALUES('c1','s1','modrinth','Lithium','lithium.jar','mod','2026-01-01T00:00:00Z','mods','modpack')",
        "INSERT INTO backups(id,server_id,created_at,size,sha256,status,kind,server_name,locked,worlds) VALUES('b1','s1','2026-01-01T00:00:00Z',1234,'abc','ok','manual','Survival',1,'[\"world\"]')",
        "INSERT INTO schedules(id,server_id,kind,every_minutes,tz) VALUES('sc1','s1','backup',60,'Europe/Berlin')",
        "INSERT INTO undo(id,server_id,title,created_at,files) VALUES('un1','s1','Edit','2026-01-01T00:00:00Z','[]')",
        "INSERT INTO notifications(id,time,kind,severity,title) VALUES('n1','2026-01-01T00:00:00Z','crash','warn','Crashed')",
        "INSERT INTO metrics_hourly(server_id,hour,n,cpu_avg,cpu_max,mem_avg,mem_max) VALUES('s1',1,2,1.5,3.0,100.0,200)",
        "INSERT INTO host_hourly(hour,n,cpu_avg,cpu_max,mem_avg,mem_max) VALUES(1,2,1.5,3.0,100.0,200)",
        "INSERT INTO api_tokens(id,user_id,name,scope,hash,prefix,created_at) VALUES('t1','u1','ci','read','h','moss_pat_ab','2026-01-01T00:00:00Z')",
        "INSERT INTO modpack_installs(id,server_id,name,format,game_version,installed_at) VALUES('m1','s1','Pack','mrpack','1.21.1','2026-01-01T00:00:00Z')",
        "INSERT INTO modpack_items(content_item_id,install_id) VALUES('c1','m1')",
    ];

    /// Every (table, column) pair with all rows, using only the columns that exist at the old version.
    fn snapshot(c: &Connection, cols: &std::collections::BTreeMap<String, Vec<String>>) -> std::collections::BTreeMap<String, Vec<Vec<String>>> {
        cols.iter().map(|(t, cs)| {
            let q = format!("SELECT {} FROM {t} ORDER BY 1", cs.iter().map(|c| format!("CAST({c} AS TEXT)")).collect::<Vec<_>>().join(","));
            let mut st = c.prepare(&q).unwrap();
            let rows = st.query_map([], |r| (0..cs.len()).map(|i| Ok(r.get::<_, Option<String>>(i)?.unwrap_or_default())).collect::<rusqlite::Result<Vec<_>>>()).unwrap().map(|r| r.unwrap()).collect();
            (t.clone(), rows)
        }).collect()
    }

    #[test]
    fn a_version_6_database_keeps_all_its_data_through_the_current_migrations() {
        let path = std::env::temp_dir().join(format!("moss-upgrade-{}.sqlite", uuid::Uuid::new_v4()));
        let c = Connection::open(&path).unwrap();
        c.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        run_with(&c, &MIGRATIONS[..6]).unwrap();
        assert_eq!(c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0)).unwrap(), 6);
        for sql in V6_ROWS { c.execute(sql, []).unwrap(); }
        let tables: Vec<String> = c.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'").unwrap().query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect();
        let mut cols = std::collections::BTreeMap::new();
        for t in &tables {
            let n: i64 = c.query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| r.get(0)).unwrap();
            assert!(n > 0, "fixture has no data in table {t}");
            let cs: Vec<String> = c.prepare(&format!("PRAGMA table_info({t})")).unwrap().query_map([], |r| r.get(1)).unwrap().map(|r| r.unwrap()).collect();
            cols.insert(t.clone(), cs);
        }
        let before = snapshot(&c, &cols);
        drop(c);
        // the real startup path: open runs every migration that exists
        let db = crate::db::Db::open(&path).unwrap();
        drop(db);
        let c = Connection::open(&path).unwrap();
        assert_eq!(c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0)).unwrap() as usize, MIGRATIONS.len());
        assert_eq!(snapshot(&c, &cols), before);
        // old backup rows stay local copies
        let (remote, present): (String, bool) = c.query_row("SELECT remote, local_present FROM backups WHERE id='b1'", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!((remote.as_str(), present), ("none", true));
        drop(c);
        let _ = std::fs::remove_file(&path);
    }
}
