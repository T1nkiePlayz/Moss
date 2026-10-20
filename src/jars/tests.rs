//! Fixtures are zips built in memory; nothing touches the disk except the cache test.
use super::*;
use std::io::{Cursor, Write};

pub(crate) fn make_zip(files: &[(&str, &str)]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(Cursor::new(vec![]));
    for (n, c) in files {
        w.start_file(*n, zip::write::SimpleFileOptions::default()).unwrap();
        w.write_all(c.as_bytes()).unwrap();
    }
    w.finish().unwrap().into_inner()
}

fn meta(files: &[(&str, &str)]) -> Info { parse(Cursor::new(make_zip(files)), "pack") }

#[test]
fn fabric_mod_json() {
    let i = meta(&[("fabric.mod.json", r#"{"schemaVersion":1,"id":"lithium","version":"0.12.1","name":"Lithium","authors":["JellySquid",{"name":"2No2Name"}],"environment":"*",
        "depends":{"fabricloader":">=0.15.0","minecraft":["1.20.x","1.21"],"java":">=17"},"recommends":{"modmenu":"*"},"breaks":{"optifabric":"*","sodium":"<0.1"},"provides":["lithium-alias"]}"#)]);
    let m = &i.metas[0];
    assert_eq!((m.id.as_str(), m.loader, m.environment, m.version.as_str()), ("lithium", "fabric", "both", "0.12.1"));
    assert_eq!(m.authors, vec!["JellySquid", "2No2Name"]);
    assert!(m.depends.contains(&Dep { id: "minecraft".into(), required: true, range: Some("1.20.x || 1.21".into()) }));
    assert!(m.depends.iter().any(|d| d.id == "modmenu" && !d.required));
    assert_eq!(m.breaks, vec!["optifabric", "sodium <0.1"]);
    assert_eq!(m.provides, vec!["lithium-alias"]);
}

#[test]
fn quilt_mod_json() {
    let i = meta(&[("quilt.mod.json", r#"{"schema_version":1,"quilt_loader":{"id":"qmod","version":"1.0","metadata":{"name":"Q","description":"d","contributors":{"Ann":"Owner"}},
        "depends":["quilt_loader",{"id":"qsl","versions":">=2","optional":true}],"breaks":[{"id":"old","versions":"<1"}]},"minecraft":{"environment":"dedicated_server"}}"#)]);
    let m = &i.metas[0];
    assert_eq!((m.loader, m.environment), ("quilt", "server"));
    assert_eq!(m.authors, vec!["Ann"]);
    assert!(m.depends.iter().any(|d| d.id == "qsl" && !d.required && d.range.as_deref() == Some(">=2")));
    assert_eq!(m.breaks, vec!["old <1"]);
}

#[test]
fn forge_and_neoforge_toml() {
    let toml = r#"modLoader="javafml"
authors="A, B"
[[mods]]
modId="jei"
version="${file.jarVersion}"
displayName="Just Enough Items"
description="  items  "
[[dependencies.jei]]
modId="minecraft"
mandatory=true
versionRange="[1.20,1.21)"
side="BOTH"
[[dependencies.jei]]
modId="forge"
mandatory=false
versionRange="47"
[[dependencies.jei]]
modId="clientlib"
mandatory=true
side="CLIENT""#;
    let i = meta(&[("META-INF/mods.toml", toml), ("META-INF/MANIFEST.MF", "Manifest-Version: 1.0\nImplementation-Version: 15.2.0\n")]);
    let m = &i.metas[0];
    assert_eq!((m.id.as_str(), m.loader, m.version.as_str(), m.description.as_str()), ("jei", "forge", "15.2.0", "items"));
    assert_eq!(m.authors, vec!["A", "B"]);
    assert_eq!(m.depends.len(), 2); // the CLIENT-side dependency is dropped on a server
    assert_eq!(m.depends[0].range.as_deref(), Some("[1.20,1.21)"));
    assert_eq!(m.depends[1], Dep { id: "forge".into(), required: false, range: None });
    let neo = r#"[[mods]]
modId="n"
version="2.0"
[[dependencies.n]]
modId="neoforge"
type="required"
versionRange="[21,)"
side="CLIENT"
[[dependencies.n]]
modId="bad"
type="incompatible"
versionRange="[1,2)""#;
    let i = meta(&[("META-INF/neoforge.mods.toml", neo)]);
    let m = &i.metas[0];
    assert_eq!((m.loader, m.environment), ("neoforge", "client"));
    assert_eq!(m.breaks, vec!["bad [1,2)"]);
}

#[test]
fn bukkit_family() {
    let i = meta(&[("plugin.yml", "name: Vault\nversion: 1.7.3\nmain: a.B\nauthor: cereal\nauthors: [x, y]\napi-version: '1.13'\ndepend: [Core]\nsoftdepend: [Essentials]\ndescription: Economy\n")]);
    let m = &i.metas[0];
    assert_eq!((m.id.as_str(), m.loader, m.api_version.as_deref()), ("Vault", "bukkit", Some("1.13")));
    assert_eq!(m.authors, vec!["cereal", "x", "y"]);
    assert_eq!(m.depends, vec![Dep { id: "Core".into(), required: true, range: None }, Dep { id: "Essentials".into(), required: false, range: None }]);
    let i = meta(&[("paper-plugin.yml", "name: P\nversion: 2\nmain: a.B\napi-version: '1.21'\ndependencies:\n  server:\n    Vault:\n      load: BEFORE\n      required: false\n    Other:\n      load: AFTER\n"), ("plugin.yml", "name: P\nversion: 2\n")]);
    assert_eq!(i.metas.iter().map(|m| m.loader).collect::<Vec<_>>(), vec!["paper", "bukkit"]);
    assert_eq!(i.metas[0].depends, vec![Dep { id: "Vault".into(), required: false, range: None }, Dep { id: "Other".into(), required: true, range: None }]);
    assert_eq!(i.pick(&["paper"]).unwrap().loader, "paper");
    let i = meta(&[("bungee.yml", "name: B\nversion: 1\nmain: x\ndepends: [A]\nsoftDepends: [C]")]);
    assert_eq!((i.metas[0].loader, i.metas[0].depends.len()), ("bungee", 2));
    let i = meta(&[("velocity-plugin.json", r#"{"id":"vp","name":"VP","version":"3","authors":["z"],"dependencies":[{"id":"luckperms","optional":true},{"id":"x"}]}"#)]);
    let m = &i.metas[0];
    assert_eq!((m.loader, m.depends.len(), m.depends[0].required, m.depends[1].required), ("velocity", 2, false, true));
}

#[test]
fn datapack_and_unknown_and_corrupt() {
    let i = meta(&[("pack.mcmeta", r#"{"pack":{"pack_format":48,"description":{"text":"Cool pack"}}}"#), ("data/x.json", "{}")]);
    assert_eq!((i.metas[0].loader, i.metas[0].id.as_str(), i.metas[0].description.as_str()), ("datapack", "pack", "Cool pack"));
    let i = meta(&[("a.class", "x")]);
    assert!(i.metas.is_empty() && i.error.is_none());
    let i = parse(Cursor::new(b"not a zip at all".to_vec()), "x");
    assert!(i.metas.is_empty() && i.error.is_some());
}

#[test]
fn malformed_descriptors_never_panic() {
    for (n, c) in [("fabric.mod.json", "{"), ("fabric.mod.json", "[]"), ("quilt.mod.json", "{\"quilt_loader\":5}"), ("META-INF/mods.toml", "[[mods"), ("META-INF/mods.toml", "mods = 3"),
        ("plugin.yml", ": : :\n\t- ["), ("plugin.yml", "- a\n- b"), ("paper-plugin.yml", "name: [1]"), ("velocity-plugin.json", "null"), ("pack.mcmeta", "\u{0}")] {
        let i = meta(&[(n, c)]);
        assert!(i.metas.is_empty(), "{n}: {c}");
    }
}

#[test]
fn oversized_entries_are_ignored() {
    let big = format!("{{\"id\":\"big\",\"pad\":\"{}\"}}", "a".repeat(MAX_ENTRY as usize + 10));
    assert!(meta(&[("fabric.mod.json", &big)]).metas.is_empty());
}

#[test]
fn cache_follows_size_and_mtime() {
    let dir = std::env::temp_dir().join(format!("moss-jars-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("a.jar");
    std::fs::write(&p, make_zip(&[("fabric.mod.json", r#"{"id":"one","version":"1"}"#)])).unwrap();
    let a = read(&p);
    assert!(Arc::ptr_eq(&a, &read(&p)), "unchanged file is served from cache");
    std::fs::write(&p, make_zip(&[("fabric.mod.json", r#"{"id":"two-two","version":"1"}"#)])).unwrap();
    assert_eq!(read(&p).metas[0].id, "two-two");
    assert!(read(&dir.join("missing.jar")).error.is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ranges() {
    use range::satisfies as s;
    for (r, v, want) in [
        (">=0.15.0", "0.15.11", true), (">=0.15.0", "0.14.9", false), ("1.20.x", "1.20.4", true), ("1.20.x", "1.21", false), ("1.20.x || 1.21", "1.21", true),
        ("^1.2.3", "1.9.0", true), ("^1.2.3", "2.0.0", false), ("^0.2.3", "0.3.0", false), ("~1.2", "1.2.9", true), ("~1.2", "1.3.0", false),
        (">=1.20 <1.21", "1.20.1", true), (">=1.20 <1.21", "1.21", false), (">= 1.20", "1.20", true), ("1.20", "1.20.0", true), ("1.20", "1.20.1", false),
        ("*", "weird", true), (">=1.0.0", "1.0.0-beta.1", false), ("<2", "2.0.0+build3", false),
        ("[1.20,1.21)", "1.20.1", true), ("[1.20,1.21)", "1.21", false), ("(,3]", "3", true), ("[21,)", "20.4", false), ("[1.5]", "1.5.0", true), ("[1,2),[4,5)", "4.1", true), ("[1,2),[4,5)", "3", false),
    ] { assert_eq!(s(r, v), Some(want), "{r} vs {v}"); }
    assert_eq!(s(">=1.0", "${version}"), None);
    assert_eq!(s("[oops", "1.0"), None);
    assert_eq!(s("???", "1.0"), None);
}
