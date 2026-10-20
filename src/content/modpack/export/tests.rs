//! Export tests: index shape, which directories are included, and a round trip through the importer's parser.
use super::*;
use crate::runtimes::launch::test_srv;

fn tmpdir() -> PathBuf {
    let d = std::env::temp_dir().join(format!("moss-exp-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn exported() -> Vec<Exported> {
    vec![Exported { path: "mods/a.jar".into(), url: "https://cdn.modrinth.com/data/P/versions/V/a.jar".into(), sha1: Some("aa".into()), sha512: "bb".into(), size: 12 }]
}

#[test]
fn index_carries_hashes_dependencies_and_loader() {
    let s = test_srv("neoforge", "1.21.1", Some("21.1.172"), "minimal");
    let i = build_index(&s, &exported(), "2026.10.20");
    assert_eq!(i["formatVersion"], 1);
    assert_eq!(i["dependencies"], json!({"minecraft": "1.21.1", "neoforge": "21.1.172"}));
    assert_eq!(i["files"][0]["hashes"], json!({"sha1": "aa", "sha512": "bb"}));
    assert_eq!(i["files"][0]["downloads"][0], "https://cdn.modrinth.com/data/P/versions/V/a.jar");
    let f = build_index(&test_srv("fabric", "1.20.1", Some("0.15.11"), "minimal"), &[], "1");
    assert_eq!(f["dependencies"]["fabric-loader"], "0.15.11");
    assert_eq!(loader_key("paper"), None);
}

#[test]
fn overrides_include_config_dirs_only_and_never_follow_links() {
    let root = tmpdir();
    for (p, c) in [("config/a.toml", "1"), ("config/sub/b.json", "2"), ("defaultconfigs/c.toml", "3"), ("kubejs/server_scripts/x.js", "4"), ("world/level.dat", "w"), ("logs/latest.log", "l"), (".moss/secret", "s"), ("mods/m.jar", "j"), ("config/.hidden", "h")] {
        let f = root.join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, c).unwrap();
    }
    let outside = tmpdir();
    std::fs::write(outside.join("secret.txt"), "top secret").unwrap();
    std::os::unix::fs::symlink(&outside, root.join("config/link")).unwrap();
    std::os::unix::fs::symlink(outside.join("secret.txt"), root.join("config/file-link")).unwrap();
    let rels: Vec<String> = collect_overrides(&root).into_iter().map(|(r, _)| r).collect();
    assert_eq!(rels, ["config/a.toml", "config/sub/b.json", "defaultconfigs/c.toml", "kubejs/server_scripts/x.js"]);
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&outside);
}

#[test]
fn exported_pack_parses_with_the_importers_parser() {
    let root = tmpdir();
    std::fs::create_dir_all(root.join("config")).unwrap();
    std::fs::write(root.join("config/a.toml"), "x=1").unwrap();
    let dest = root.join("out.mrpack");
    let s = test_srv("quilt", "1.21.1", Some("0.26.4"), "minimal");
    write_pack(&dest, &build_index(&s, &exported(), "v1"), &collect_overrides(&root)).unwrap();
    let mut z = zip::ZipArchive::new(std::fs::File::open(&dest).unwrap()).unwrap();
    let sc = mrpack::parse(&mut z).unwrap();
    assert_eq!((sc.pack.runtime(), sc.pack.loader_version.as_deref(), sc.pack.files.len()), ("quilt", Some("0.26.4"), 1));
    assert_eq!(sc.pack.files[0].provider, Some(("modrinth".into(), "P".into(), "V".into())));
    assert_eq!(sc.overrides.iter().map(|o| o.rel.as_str()).collect::<Vec<_>>(), ["config/a.toml"]);
    let _ = std::fs::remove_dir_all(&root);
}
