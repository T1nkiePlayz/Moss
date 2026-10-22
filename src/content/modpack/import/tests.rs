//! Import placement tests: files, overrides, replace mode, rollback, path safety and provenance rows.
use super::*;
use zipsafe::tests::build;

fn tmpdir() -> PathBuf {
    let d = std::env::temp_dir().join(format!("moss-imp-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write_zip(dir: &FsPath, entries: &[(&str, &[u8])]) -> PathBuf {
    let p = dir.join("pack.zip");
    std::fs::write(&p, build(entries).into_inner().into_inner()).unwrap();
    p
}

fn scan_entries(pack: &FsPath) -> Vec<OverrideEntry> {
    let mut z = zip::ZipArchive::new(std::fs::File::open(pack).unwrap()).unwrap();
    zipsafe::scan_overrides(&mut z, "overrides").unwrap().0
}

fn staged(dir: &FsPath, name: &str, body: &str) -> (String, PathBuf) {
    let p = dir.join(format!("{name}.bin"));
    std::fs::write(&p, body).unwrap();
    (format!("mods/{name}"), p)
}

#[test]
fn places_files_and_overrides_in_add_mode_keeping_existing_mods() {
    let (root, stage) = (tmpdir(), tmpdir());
    std::fs::create_dir_all(root.join("mods")).unwrap();
    std::fs::write(root.join("mods/keep.jar"), "keep").unwrap();
    let pack = write_zip(&stage, &[("overrides/config/a.toml", b"cfg"), ("modrinth.index.json", b"{}")]);
    let n = place(&root, "mods", &pack, &scan_entries(&pack), &[staged(&stage, "new.jar", "new")], false, &root.join(".moss/modpack-prev")).unwrap();
    assert_eq!(n, 1);
    assert_eq!(std::fs::read_to_string(root.join("mods/new.jar")).unwrap(), "new");
    assert_eq!(std::fs::read_to_string(root.join("mods/keep.jar")).unwrap(), "keep");
    assert_eq!(std::fs::read_to_string(root.join("config/a.toml")).unwrap(), "cfg");
    for d in [root, stage] { let _ = std::fs::remove_dir_all(d); }
}

#[test]
fn replace_mode_swaps_the_content_dir_and_rolls_back_on_failure() {
    let (root, stage) = (tmpdir(), tmpdir());
    std::fs::create_dir_all(root.join("mods")).unwrap();
    std::fs::write(root.join("mods/old.jar"), "old").unwrap();
    let aside = root.join(".moss/modpack-prev");
    let pack = write_zip(&stage, &[("overrides/config/a.toml", b"cfg")]);
    place(&root, "mods", &pack, &scan_entries(&pack), &[staged(&stage, "new.jar", "new")], true, &aside).unwrap();
    assert!(!root.join("mods/old.jar").exists() && root.join("mods/new.jar").exists() && aside.exists(), "the caller deletes aside after recording");
    let _ = std::fs::remove_dir_all(&aside);
    // failure after the swap: a file path that is a directory on the server
    std::fs::create_dir_all(root.join("mods/blocked.jar")).unwrap();
    let bad = [staged(&stage, "extra.jar", "x"), ("mods/blocked.jar".to_string(), stage.join("extra.jar.bin"))];
    assert!(place(&root, "mods", &pack, &[], &bad, true, &aside).is_err());
    assert!(root.join("mods/new.jar").exists() && !root.join("mods/extra.jar").exists(), "the previous directory is restored");
    for d in [root, stage] { let _ = std::fs::remove_dir_all(d); }
}

#[test]
fn merge_mode_restores_overwritten_files_on_failure() {
    let (root, stage) = (tmpdir(), tmpdir());
    std::fs::create_dir_all(root.join("mods/blocked.jar")).unwrap();
    std::fs::write(root.join("mods/a.jar"), "original").unwrap();
    let pack = write_zip(&stage, &[("modrinth.index.json", b"{}")]);
    let files = [staged(&stage, "a.jar", "replacement"), ("mods/blocked.jar".to_string(), stage.join("a.jar.bin"))];
    assert!(place(&root, "mods", &pack, &[], &files, false, &root.join(".moss/modpack-prev")).is_err());
    assert_eq!(std::fs::read_to_string(root.join("mods/a.jar")).unwrap(), "original", "the overwritten file comes back");
    for d in [root, stage] { let _ = std::fs::remove_dir_all(d); }
}

#[test]
fn files_cannot_escape_the_server_directory() {
    let (root, stage) = (tmpdir(), tmpdir());
    let outside = tmpdir();
    let pack = write_zip(&stage, &[]);
    std::os::unix::fs::symlink(&outside, root.join("mods")).unwrap();
    let r = place(&root, "mods", &pack, &[], &[staged(&stage, "x.jar", "x")], false, &root.join(".moss/p"));
    assert!(r.is_err() && !outside.join("x.jar").exists());
    let r = place(&tmpdir(), "mods", &pack, &[], &[("../escape.jar".to_string(), stage.join("x.jar.bin"))], false, &root.join(".moss/p"));
    assert!(r.is_err());
    for d in [root, stage, outside] { let _ = std::fs::remove_dir_all(d); }
}

#[test]
fn provenance_rows_link_modrinth_and_curseforge_items_and_skip_other_dirs() {
    let mk = |path: &str, prov: Option<(&str, &str, &str)>| Fetched { file: PackFile { path: path.into(), sha512: Some("s".into()), provider: prov.map(|(a, b, c)| (a.into(), b.into(), c.into())), ..Default::default() }, path: "/x".into() };
    let rows = rows_for(&[mk("mods/Alpha-1.0.jar", Some(("modrinth", "P1", "V1"))), mk("mods/b.jar", None), mk("mods/cf.jar", Some(("curseforge", "10", "100"))), mk("config/c.toml", None), mk("resourcepacks/r.zip", None)], "mods");
    assert_eq!(rows.len(), 3);
    assert_eq!((rows[0].provider.as_str(), rows[0].project.as_deref(), rows[0].version.as_deref(), rows[0].title.as_str()), ("modrinth", Some("P1"), Some("V1"), "Alpha-1.0"));
    assert_eq!((rows[1].provider.as_str(), rows[1].project.clone()), ("manual", None));
    assert_eq!(rows[2].provider, "curseforge");
}

#[test]
fn provenance_is_recorded_in_the_database() {
    let c = rusqlite::Connection::open_in_memory().unwrap();
    c.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
    crate::db::run_migrations_for_tests(&c).unwrap();
    c.execute("INSERT INTO servers(id,name,runtime,game_version,port,memory_mb,created_at) VALUES ('s','n','fabric','1.21',25565,1024,'t')", []).unwrap();
    c.execute("INSERT INTO content_items(id,server_id,provider,title,filename,kind,installed_at,dir) VALUES ('i','s','manual','t','a.jar','mods','t','mods')", []).unwrap();
    c.execute("INSERT INTO modpack_installs(id,server_id,name,format,game_version,installed_at) VALUES ('m','s','Pack','mrpack','1.21','t')", []).unwrap();
    c.execute("INSERT INTO modpack_items(content_item_id,install_id) VALUES ('i','m')", []).unwrap();
    c.execute("DELETE FROM content_items WHERE id='i'", []).unwrap();
    let left: i64 = c.query_row("SELECT COUNT(*) FROM modpack_items", [], |r| r.get(0)).unwrap();
    assert_eq!(left, 0, "provenance follows the content item");
}
