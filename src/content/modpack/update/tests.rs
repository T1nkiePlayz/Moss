//! Pack updates against a real database and data directory. Downloads need HTTPS to a provider, which tests cannot
//! reach, so these packs change only what needs no download (removals, config files, versions) and the file
//! placement itself is covered directly.
use super::*;
use crate::{server, testutil::state};
use zipsafe::tests::build;

fn index(version: &str, mc: &str, files: &[(&str, &str, &str)]) -> Vec<u8> {
    serde_json::to_vec(&json!({"formatVersion": 1, "name": "Pack", "versionId": version, "dependencies": {"minecraft": mc, "fabric-loader": "0.16"},
        "files": files.iter().map(|(path, project, ver)| json!({"path": path, "hashes": {"sha1": "aa", "sha512": format!("{project}{ver}")}, "downloads": [format!("https://cdn.modrinth.com/data/{project}/versions/{ver}/x.jar")], "fileSize": 1})).collect::<Vec<_>>()})).unwrap()
}

fn write_pack(dir: &FsPath, idx: Vec<u8>, overrides: &[(&str, &str)]) -> PathBuf {
    let mut entries: Vec<(String, Vec<u8>)> = vec![("modrinth.index.json".into(), idx)];
    for (p, b) in overrides { entries.push((format!("overrides/{p}"), b.as_bytes().to_vec())); }
    let refs: Vec<(&str, &[u8])> = entries.iter().map(|(a, b)| (a.as_str(), b.as_slice())).collect();
    let p = dir.join(format!("{}.mrpack", uuid::Uuid::now_v7()));
    std::fs::write(&p, build(&refs).into_inner().into_inner()).unwrap();
    p
}

fn sum(s: &str) -> String { use sha2::{Digest, Sha256}; hex::encode(Sha256::digest(s.as_bytes())) }

struct Fx { st: St, s: Srv, c: Compat, install: String, dir: PathBuf }

/// A fabric server with an installed pack: mods a (kept) and b (the new version drops it), config files the pack wrote.
async fn fixture(pinned: bool) -> Fx {
    let st = state(|_| {}).await;
    let id = uuid::Uuid::now_v7().to_string();
    let (i, now) = (id.clone(), crate::db::now());
    st.db.call(move |c| c.execute("INSERT INTO servers(id,name,runtime,game_version,loader_version,port,memory_mb,installed,created_at) VALUES (?,?,?,?,?,?,?,1,?)", rusqlite::params![i, "S", "fabric", "1.21", "0.16", 25565, 2048, now])).await.unwrap();
    let s = server::get(&st, &id).await.unwrap();
    let c = compat(&st, &s).unwrap();
    let root = s.dir(&st);
    for (p, b) in [("mods/a.jar", "A"), ("mods/b.jar", "B"), ("config/changed.toml", "old"), ("config/edited.toml", "mine"), ("config/same.toml", "same")] {
        std::fs::create_dir_all(root.join(p).parent().unwrap()).unwrap();
        std::fs::write(root.join(p), b).unwrap();
    }
    let install = uuid::Uuid::now_v7().to_string();
    let (sid, inst) = (id.clone(), install.clone());
    st.db.call(move |db| {
        db.execute("INSERT INTO modpack_installs(id,server_id,name,version,format,game_version,loader,mode,installed_at,files,pinned,provider,project_id,version_id) VALUES (?,?,'Pack','1','mrpack','1.21','fabric','add','t',2,?, 'modrinth','PACK','pv1')", rusqlite::params![inst, sid, pinned])?;
        for (item, project, ver, file) in [("ia", "a", "v1", "a.jar"), ("ib", "b", "v1", "b.jar")] {
            db.execute("INSERT INTO content_items(id,server_id,provider,project_id,version_id,title,version_number,filename,kind,installed_at,pinned,dir) VALUES (?,?,'modrinth',?,?,?,?,?,'mods','t',0,'mods')", rusqlite::params![item, sid, project, ver, project, file, file])?;
            db.execute("INSERT INTO modpack_items(content_item_id,install_id) VALUES (?,?)", rusqlite::params![item, inst])?;
        }
        for (p, written) in [("config/changed.toml", "old"), ("config/edited.toml", "orig"), ("config/same.toml", "same")] {
            db.execute("INSERT INTO modpack_files(install_id,path,sha256) VALUES (?,?,?)", rusqlite::params![inst, p, hex::encode(<sha2::Sha256 as sha2::Digest>::digest(written.as_bytes()))])?;
        }
        Ok(())
    }).await.unwrap();
    Fx { st, s, c, install, dir: root }
}

fn new_pack(fx: &Fx, mc: &str) -> PathBuf {
    let tmp = fx.dir.join(".moss/tmp-test");
    std::fs::create_dir_all(&tmp).unwrap();
    write_pack(&tmp, index("2", mc, &[("mods/a.jar", "a", "v1")]), &[("config/changed.toml", "new"), ("config/edited.toml", "pack-edit"), ("config/same.toml", "same"), ("config/added.toml", "fresh")])
}

#[tokio::test]
async fn the_plan_lists_every_kind_of_change_and_protects_edited_configs() {
    let fx = fixture(false).await;
    let row = install_row(&fx.st, &fx.s.id, &fx.install).await.unwrap();
    let p = plan(&fx.st, None, &fx.s, &fx.c, &row, &new_pack(&fx, "1.21")).await.unwrap();
    assert!(p.diff.blockers.is_empty());
    assert_eq!(p.diff.files.unchanged, 1, "a.jar is the same version");
    assert_eq!(p.diff.files.removed.iter().map(|c| c.path.as_str()).collect::<Vec<_>>(), vec!["mods/b.jar"]);
    assert_eq!(p.diff.overrides.updated, vec!["config/changed.toml".to_string()]);
    assert_eq!(p.diff.overrides.added, vec!["config/added.toml".to_string()]);
    assert_eq!(p.diff.overrides.edited.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>(), vec!["config/edited.toml"]);
    assert_eq!(p.diff.overrides.unchanged, 1);
    let j = p.diff.json(&p.versions, &p.to_version);
    assert_eq!(j["versions"]["to"]["version"], "2");
    assert_eq!(j["overrides"]["user_modified"][0]["path"], "config/edited.toml");
    assert_eq!(j["can_apply"], true);
}

#[tokio::test]
async fn applying_removes_dropped_mods_writes_only_safe_configs_and_records_everything() {
    let fx = fixture(false).await;
    let pack = new_pack(&fx, "1.21");
    let sig = apply_update(&OpCtx::detached(&fx.st), &fx.s, &fx.c, &fx.install, &pack, None, "tester", None, false, None).await.unwrap();
    let read = |p: &str| std::fs::read_to_string(fx.dir.join(p)).ok();
    assert_eq!(read("config/changed.toml").as_deref(), Some("new"), "untouched config updated");
    assert_eq!(read("config/edited.toml").as_deref(), Some("mine"), "the person's edit is never overwritten");
    assert_eq!(read("config/added.toml").as_deref(), Some("fresh"));
    assert!(read("mods/b.jar").is_none() && read("mods/a.jar").is_some());
    let items: Vec<String> = db_items(&fx.st, &fx.s.id).await.unwrap().iter().map(|i| i["filename"].as_str().unwrap().to_string()).collect();
    assert_eq!(items, vec!["a.jar".to_string()]);
    let row = install_row(&fx.st, &fx.s.id, &fx.install).await.unwrap();
    assert_eq!(row.version, "2");
    let sums = recorded_sums(&fx.st, &fx.install).await.unwrap();
    assert_eq!(sums["config/changed.toml"], sum("new"));
    assert_eq!(sums["config/added.toml"], sum("fresh"));
    assert_eq!(sums["config/edited.toml"], sum("orig"), "the baseline for an edited file stays what the pack last wrote");
    let undo: i64 = fx.st.db.call({ let i = fx.s.id.clone(); move |c| c.query_row("SELECT COUNT(*) FROM undo WHERE server_id=?", [i], |r| r.get(0)) }).await.unwrap();
    assert_eq!(undo, 1, "one undo entry covers the whole update");
    assert!(!fx.dir.join(".moss/pack-update-prev").exists());
    // applying the same version again changes nothing and has the same signature
    let again = apply_update(&OpCtx::detached(&fx.st), &fx.s, &fx.c, &fx.install, &pack, None, "tester", None, false, None).await.unwrap();
    assert_ne!(again, sig, "the plan differs now that the work is done");
    assert_eq!(read("config/edited.toml").as_deref(), Some("mine"));
}

#[tokio::test]
async fn undo_restores_what_the_update_changed() {
    let fx = fixture(false).await;
    apply_update(&OpCtx::detached(&fx.st), &fx.s, &fx.c, &fx.install, &new_pack(&fx, "1.21"), None, "tester", None, false, None).await.unwrap();
    let (uid, files): (String, String) = fx.st.db.call({ let i = fx.s.id.clone(); move |c| c.query_row("SELECT id,files FROM undo WHERE server_id=?", [i], |r| Ok((r.get(0)?, r.get(1)?))) }).await.unwrap();
    assert!(files.contains("mods/b.jar") && files.contains("config/changed.toml") && !files.contains("config/edited.toml"), "{files}");
    let base = fx.dir.join(".moss/undo").join(&uid);
    assert_eq!(std::fs::read_to_string(base.join("mods/b.jar")).unwrap(), "B", "the removed mod is in the snapshot");
    assert_eq!(std::fs::read_to_string(base.join("config/changed.toml")).unwrap(), "old");
}

#[tokio::test]
async fn pinned_packs_wrong_versions_and_stale_tests_are_refused() {
    let fx = fixture(true).await;
    let e = apply_update(&OpCtx::detached(&fx.st), &fx.s, &fx.c, &fx.install, &new_pack(&fx, "1.21"), None, "t", None, false, None).await.unwrap_err();
    assert!(format!("{e:#}").contains("pinned"), "{e:#}");
    assert!(fx.dir.join("mods/b.jar").is_file(), "nothing changed");
    let fx = fixture(false).await;
    let e = apply_update(&OpCtx::detached(&fx.st), &fx.s, &fx.c, &fx.install, &new_pack(&fx, "1.22"), None, "t", None, false, None).await.unwrap_err();
    assert!(format!("{e:#}").contains("upgrade assistant"), "{e:#}");
    let e = apply_update(&OpCtx::detached(&fx.st), &fx.s, &fx.c, &fx.install, &new_pack(&fx, "1.21"), None, "t", Some("stale-signature"), false, None).await.unwrap_err();
    assert!(format!("{e:#}").contains("differs from what was tested"), "{e:#}");
    assert!(fx.dir.join("mods/b.jar").is_file() && std::fs::read_to_string(fx.dir.join("config/changed.toml")).unwrap() == "old");
}

#[tokio::test]
async fn placement_replaces_files_and_a_failure_puts_everything_back() {
    let fx = fixture(false).await;
    let stage = fx.dir.join(".moss/stage");
    std::fs::create_dir_all(&stage).unwrap();
    std::fs::write(stage.join("a2"), "A2").unwrap();
    std::fs::write(stage.join("c"), "C").unwrap();
    let pack = write_pack(&stage, index("2", "1.21", &[]), &[("config/changed.toml", "new")]);
    let entries = { let mut z = zip::ZipArchive::new(std::fs::File::open(&pack).unwrap()).unwrap(); zipsafe::scan_overrides(&mut z, "overrides").unwrap().0 };
    let aside = fx.dir.join(".moss/aside");
    let good = Placement { removes: vec!["mods/b.jar".into()], files: vec![("mods/a-2.jar".into(), stage.join("a2"), Some("mods/a.jar".into()))], overrides: entries.clone() };
    let mut j = Journal::default();
    place(&fx.dir, &aside, &good, &pack, &mut j).unwrap();
    assert!(!fx.dir.join("mods/a.jar").exists() && !fx.dir.join("mods/b.jar").exists());
    assert_eq!(std::fs::read_to_string(fx.dir.join("mods/a-2.jar")).unwrap(), "A2");
    assert_eq!(std::fs::read_to_string(fx.dir.join("config/changed.toml")).unwrap(), "new");
    j.rollback();
    assert_eq!(std::fs::read_to_string(fx.dir.join("mods/a.jar")).unwrap(), "A");
    assert_eq!(std::fs::read_to_string(fx.dir.join("mods/b.jar")).unwrap(), "B");
    assert_eq!(std::fs::read_to_string(fx.dir.join("config/changed.toml")).unwrap(), "old", "overwritten configs come back too");
    assert!(!fx.dir.join("mods/a-2.jar").exists());
    // failure part way: the second file's target is a directory
    std::fs::create_dir_all(fx.dir.join("mods/blocked.jar")).unwrap();
    let bad = Placement { removes: vec!["mods/b.jar".into()], files: vec![("mods/a-2.jar".into(), stage.join("a2"), Some("mods/a.jar".into())), ("mods/blocked.jar".into(), stage.join("c"), None)], overrides: entries };
    let mut j = Journal::default();
    assert!(place(&fx.dir, &aside, &bad, &pack, &mut j).is_err());
    assert_eq!(std::fs::read_to_string(fx.dir.join("mods/a.jar")).unwrap(), "A");
    assert_eq!(std::fs::read_to_string(fx.dir.join("mods/b.jar")).unwrap(), "B");
    assert!(!fx.dir.join("mods/a-2.jar").exists());
    assert_eq!(std::fs::read_to_string(fx.dir.join("config/changed.toml")).unwrap(), "old");
}

#[tokio::test]
async fn placement_cannot_write_outside_the_server() {
    let fx = fixture(false).await;
    let stage = fx.dir.join(".moss/stage");
    std::fs::create_dir_all(&stage).unwrap();
    std::fs::write(stage.join("x"), "x").unwrap();
    let pack = write_pack(&stage, index("2", "1.21", &[]), &[]);
    let p = Placement { removes: vec![], files: vec![("../escape.jar".into(), stage.join("x"), None)], overrides: vec![] };
    let mut j = Journal::default();
    assert!(place(&fx.dir, &fx.dir.join(".moss/aside"), &p, &pack, &mut j).is_err());
    assert!(!fx.dir.parent().unwrap().join("escape.jar").exists());
}

#[tokio::test]
async fn pinning_a_pack_is_stored_and_listed() {
    let fx = fixture(false).await;
    let (sid, inst) = (fx.s.id.clone(), fx.install.clone());
    fx.st.db.call(move |c| c.execute("UPDATE modpack_installs SET pinned=1 WHERE id=? AND server_id=?", [inst, sid])).await.unwrap();
    assert!(install_row(&fx.st, &fx.s.id, &fx.install).await.unwrap().pinned);
    let j = install_row(&fx.st, &fx.s.id, &fx.install).await.unwrap().json(2, 3);
    assert_eq!((j["pinned"].as_bool(), j["can_check"].as_bool(), j["source"]["provider"].as_str()), (Some(true), Some(true), Some("modrinth")));
    assert!(install_row(&fx.st, "other", &fx.install).await.is_err(), "scoped to its server");
}
