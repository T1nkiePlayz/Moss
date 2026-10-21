//! Game mods: dependency resolution (cycles, incompatibilities), the manifest, archive safety, workshop and BepInEx
//! launch arguments, installs against stub providers, and the API handlers.
use super::{factorio::{self, DepKind}, thunderstore as ts, *};
use crate::{games, ops::OpCtx, testutil::{state, tmp_data}};
use serde_json::{Value, json};
use sha1::{Digest, Sha1};
use std::{collections::HashMap, io::Write, path::PathBuf};

// ---- Thunderstore resolution

fn pkg(ns: &str, name: &str, ver: &str, deps: &[&str]) -> ts::Pkg {
    ts::Pkg { ns: ns.into(), name: name.into(), version: ver.into(), download_url: String::new(), deps: deps.iter().map(|d| ts::parse_dep(d).unwrap()).collect() }
}
fn index(v: Vec<ts::Pkg>) -> HashMap<String, ts::Pkg> { v.into_iter().map(|p| (p.id(), p)).collect() }
fn ids(v: &[ts::Pkg]) -> Vec<String> { v.iter().map(|p| p.id()).collect() }

#[test]
fn thunderstore_dependency_strings_parse() {
    let d = ts::parse_dep("denikson-BepInExPack_Valheim-5.4.2333").unwrap();
    assert_eq!((d.ns.as_str(), d.name.as_str(), d.version.as_str()), ("denikson", "BepInExPack_Valheim", "5.4.2333"));
    assert!(ts::parse_dep("nonsense").is_none() && ts::parse_dep("a-b-notaversion").is_none() && ts::parse_dep("../x-y-1.0").is_none());
    assert_eq!(ts::split_id("RandyKnapp-EpicLoot"), Some(("RandyKnapp", "EpicLoot")));
    assert!(ts::split_id("../etc-passwd").is_none() && ts::split_id("noseparator").is_none());
}

#[test]
fn thunderstore_resolves_transitively_dependencies_first() {
    let have = index(vec![pkg("a", "Mod", "1.0.0", &["b-Lib-1.0.0", "denikson-BepInExPack_Valheim-5.4.0"]), pkg("b", "Lib", "1.0.0", &["denikson-BepInExPack_Valheim-5.4.0"]), pkg("denikson", "BepInExPack_Valheim", "5.4.0", &[])]);
    assert!(missing("a-Mod", &have).is_empty());
    assert_eq!(ids(&ts::order("a-Mod", &have).unwrap()), ["denikson-BepInExPack_Valheim", "b-Lib", "a-Mod"]);
    let partial = index(vec![pkg("a", "Mod", "1.0.0", &["b-Lib-1.0.0"])]);
    assert_eq!(missing("a-Mod", &partial).iter().map(|d| d.id()).collect::<Vec<_>>(), ["b-Lib"]);
    assert!(ts::order("a-Mod", &partial).is_err());
}

#[test]
fn thunderstore_cycles_terminate_and_newest_requirement_wins() {
    let have = index(vec![pkg("a", "A", "1.0.0", &["b-B-1.0.0"]), pkg("b", "B", "1.0.0", &["a-A-1.0.0"])]);
    assert!(missing("a-A", &have).is_empty());
    assert_eq!(ids(&ts::order("a-A", &have).unwrap()), ["b-B", "a-A"]);
    let have = index(vec![pkg("a", "A", "1.0.0", &["c-C-1.0.0", "b-B-1.0.0"]), pkg("b", "B", "1.0.0", &["c-C-2.0.0"]), pkg("c", "C", "1.0.0", &[])]);
    let m = missing("a-A", &have);
    assert_eq!((m.len(), m[0].version.as_str()), (1, "2.0.0"), "C 1.0.0 is too old for B");
}

#[test]
fn bepinex_pack_unpacks_into_the_server_root_and_mods_into_plugins() {
    let none = |_: &str| false;
    assert_eq!(ts::map_entry(ts::BEPINEX_ID, "BepInExPack_Valheim/BepInEx/core/BepInEx.dll", &none).as_deref(), Some("BepInEx/core/BepInEx.dll"));
    assert_eq!(ts::map_entry(ts::BEPINEX_ID, "BepInExPack_Valheim/doorstop_libs/libdoorstop_x64.so", &none).as_deref(), Some("doorstop_libs/libdoorstop_x64.so"));
    assert!(ts::map_entry(ts::BEPINEX_ID, "manifest.json", &none).is_none() && ts::map_entry(ts::BEPINEX_ID, "BepInExPack_Valheim/", &none).is_none());
    assert!(ts::map_entry(ts::BEPINEX_ID, "BepInExPack_Valheim/BepInEx/config/BepInEx.cfg", &|_| true).is_none(), "an existing config is kept");
    assert_eq!(ts::map_entry("a-Mod", "Mod.dll", &none).as_deref(), Some("BepInEx/plugins/a-Mod/Mod.dll"));
    assert_eq!(ts::map_entry("a-Mod", "plugins/Mod.dll", &none).as_deref(), Some("BepInEx/plugins/a-Mod/Mod.dll"));
    assert_eq!(ts::map_entry("a-Mod", "patchers/P.dll", &none).as_deref(), Some("BepInEx/patchers/a-Mod/P.dll"));
    assert_eq!(ts::map_entry("a-Mod", "config/a.cfg", &none).as_deref(), Some("BepInEx/config/a.cfg"));
    assert_eq!(ts::map_entry("a-Mod", "assets/x.bundle", &none).as_deref(), Some("BepInEx/plugins/a-Mod/assets/x.bundle"));
    for skip in ["manifest.json", "icon.png", "README.md", "CHANGELOG.md", "LICENSE"] { assert!(ts::map_entry("a-Mod", skip, &none).is_none(), "{skip}"); }
}

use ts::missing;

// ---- Factorio resolution

fn release(ver: &str, deps: &[&str]) -> factorio::Release {
    factorio::Release { version: ver.into(), download_url: format!("/download/m/{ver}"), sha1: "00".into(), file_name: format!("m_{ver}.zip"), deps: deps.iter().map(|d| factorio::parse_dep(d).unwrap()).collect() }
}
fn fmod(name: &str, releases: Vec<factorio::Release>) -> (String, factorio::Mod) { (name.into(), factorio::Mod { name: name.into(), title: name.into(), releases }) }
fn entry(id: &str, ver: &str, conflicts: &[&str]) -> Entry { Entry { id: id.into(), version: ver.into(), enabled: true, conflicts: conflicts.iter().map(|c| c.to_string()).collect(), ..Default::default() } }

#[test]
fn factorio_dependency_strings_parse() {
    let d = factorio::parse_dep("flib >= 0.16.2").unwrap();
    assert_eq!((d.kind, d.name.as_str(), d.req), (DepKind::Required, "flib", Some((">=".into(), "0.16.2".into()))));
    assert_eq!(factorio::parse_dep("base >= 2.0").unwrap().name, "base");
    assert_eq!(factorio::parse_dep("(?) Flow Control >= 3.0.5").unwrap().name, "Flow Control");
    assert_eq!(factorio::parse_dep("? quality").unwrap().kind, DepKind::Optional);
    assert_eq!(factorio::parse_dep("! Krastorio").unwrap().kind, DepKind::Incompatible);
    let n = factorio::parse_dep("~ other = 1.0.0").unwrap();
    assert_eq!((n.kind, n.name.as_str()), (DepKind::Required, "other"));
    assert_eq!(factorio::parse_dep("+ ChangeInserterDropLane >= 1.3.0").unwrap().kind, DepKind::Optional);
    assert!(factorio::parse_dep("  ").is_none());
    assert!(factorio::satisfies("1.10.0", &Some((">=".into(), "1.9".into()))) && !factorio::satisfies("1.2", &Some((">".into(), "1.2".into()))) && factorio::satisfies("1.2.0", &Some(("=".into(), "1.2".into()))));
}

#[test]
fn factorio_plan_installs_required_dependencies_only() {
    let have: HashMap<_, _> = [
        fmod("top", vec![release("1.0.0", &["base >= 2.0", "lib >= 1.1", "? opt", "(?) hidden", "~ soft"])]),
        fmod("lib", vec![release("1.0.0", &[]), release("1.2.0", &[]), release("2.0.0", &[])]),
        fmod("soft", vec![release("0.1.0", &[])]),
    ].into();
    let plan = factorio::plan("top", None, &have, &[]).unwrap();
    assert_eq!(plan.iter().map(|c| (c.name.as_str(), c.release.version.as_str())).collect::<Vec<_>>(), [("lib", "2.0.0"), ("soft", "0.1.0"), ("top", "1.0.0")], "optional and built-in skipped, newest satisfying release chosen");
    assert!(factorio::unfetched("top", None, &have).is_empty());
    assert_eq!(factorio::unfetched("top", None, &HashMap::new()), ["top"]);
    let skip = factorio::plan("top", None, &have, &[entry("lib", "1.2.0", &[]), entry("soft", "0.1.0", &[])]).unwrap();
    assert_eq!(skip.len(), 1, "satisfied installed dependencies are not reinstalled");
    assert!(factorio::plan("top", Some("9.9.9"), &have, &[]).is_err(), "an unknown pinned version fails");
    let strict: HashMap<_, _> = [fmod("top", vec![release("1.0.0", &["lib >= 5"])]), fmod("lib", vec![release("1.0.0", &[])])].into();
    assert!(factorio::plan("top", None, &strict, &[]).unwrap_err().to_string().contains("lib"));
}

#[test]
fn factorio_cycles_terminate() {
    let have: HashMap<_, _> = [fmod("a", vec![release("1.0.0", &["b"])]), fmod("b", vec![release("1.0.0", &["a"])])].into();
    let plan = factorio::plan("a", None, &have, &[]).unwrap();
    assert_eq!(plan.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["b", "a"]);
}

#[test]
fn factorio_incompatible_mods_block_the_install() {
    let have: HashMap<_, _> = [fmod("a", vec![release("1.0.0", &["b", "! c"])]), fmod("b", vec![release("1.0.0", &["! a"])]), fmod("c", vec![release("1.0.0", &[])])].into();
    let both = factorio::plan("a", None, &have, &[]).unwrap_err().to_string();
    assert!(both.contains("incompatible"), "{both}: a and b exclude each other");
    let have: HashMap<_, _> = [fmod("a", vec![release("1.0.0", &["! c"])])].into();
    assert!(factorio::plan("a", None, &have, &[entry("c", "1.0.0", &[])]).unwrap_err().to_string().contains("incompatible with c"));
    assert!(factorio::plan("a", None, &have, &[entry("d", "1.0.0", &[])]).is_ok());
    let have: HashMap<_, _> = [fmod("a", vec![release("1.0.0", &[])])].into();
    assert!(factorio::plan("a", None, &have, &[entry("d", "1.0.0", &["a"])]).is_err(), "an installed mod can forbid the new one");
}

// ---- manifest and settings

#[test]
fn manifest_round_trips_and_tracks_dependents() {
    let dir = tmp_data();
    assert_eq!(Manifest::load(&dir).unwrap(), Manifest::default());
    let mut m = Manifest::default();
    m.upsert(Entry { id: "a-A".into(), title: "A".into(), version: "1.0.0".into(), enabled: true, requires: vec!["b-B".into()], files: vec!["BepInEx/plugins/a-A/A.dll".into()], installed_at: "t".into(), ..Default::default() });
    m.upsert(Entry { id: "b-B".into(), auto: true, enabled: true, ..Default::default() });
    m.upsert(Entry { id: "b-B".into(), auto: true, version: "2".into(), enabled: false, ..Default::default() });
    m.save(&dir).unwrap();
    let back = Manifest::load(&dir).unwrap();
    assert_eq!(back.mods, m.mods);
    assert_eq!((back.version, back.mods.len(), back.get("b-B").unwrap().version.as_str()), (1, 2, "2"));
    assert_eq!(back.dependents("b-B"), ["a-A"]);
    let mut back = back;
    assert!(back.remove("a-A").is_some() && back.remove("a-A").is_none());
    std::fs::write(dir.join(MANIFEST_FILE), "{broken").unwrap();
    assert_eq!(Manifest::load(&dir).unwrap_err().code, "game_mods.manifest");
}

#[test]
fn settings_edits_keep_other_lines() {
    let dir = tmp_data();
    std::fs::write(dir.join(games::SETTINGS_FILE), "# note\npassword=hunter22\nworkshop_map = 1\n").unwrap();
    set_settings(&dir, &[("workshop_map", Some("42")), ("workshop_collection", Some("7")), ("password", None)]).unwrap();
    assert_eq!(std::fs::read_to_string(dir.join(games::SETTINGS_FILE)).unwrap(), "# note\nworkshop_map=42\nworkshop_collection=7\n");
    assert!(set_settings(&dir, &[("a", Some("x\ny"))]).is_err());
}

// ---- archives

fn zip_bytes(files: &[(&str, &str)]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(vec![]));
    for (n, c) in files {
        w.start_file(*n, zip::write::SimpleFileOptions::default()).unwrap();
        w.write_all(c.as_bytes()).unwrap();
    }
    w.finish().unwrap().into_inner()
}
fn write_zip(files: &[(&str, &str)]) -> (PathBuf, PathBuf) {
    let base = tmp_data();
    let (root, z) = (base.join("root"), base.join("a.zip"));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(&z, zip_bytes(files)).unwrap();
    (root, z)
}
fn same(n: &str) -> Option<String> { Some(n.to_string()) }

#[test]
fn extraction_writes_mapped_files() {
    let (root, z) = write_zip(&[("a/b.txt", "hi"), ("skip.txt", "no")]);
    let got = net::extract_zip(&z, &root, &|n| n.strip_prefix("a/").map(|r| format!("out/{r}"))).unwrap();
    assert_eq!(got, ["out/b.txt"]);
    assert_eq!(std::fs::read_to_string(root.join("out/b.txt")).unwrap(), "hi");
    assert!(!root.join("skip.txt").exists());
}

#[test]
fn zip_slip_is_rejected_before_anything_is_written() {
    for evil in ["../evil.txt", "a/../../evil.txt"] {
        let (root, z) = write_zip(&[("good.txt", "ok"), (evil, "pwned")]);
        let e = net::extract_zip(&z, &root, &same).unwrap_err().to_string();
        assert!(e.contains("unsafe path"), "{evil}: {e}");
        assert!(!root.join("good.txt").exists() && !root.parent().unwrap().join("evil.txt").exists(), "{evil} must leave no files");
    }
}

#[test]
fn absolute_entry_names_are_confined_to_the_root() {
    let (root, z) = write_zip(&[("/abs/evil.txt", "x")]);
    assert_eq!(net::extract_zip(&z, &root, &same).unwrap(), ["abs/evil.txt"]);
    assert!(root.join("abs/evil.txt").exists());
}

#[test]
fn symlink_entries_are_rejected() {
    let base = tmp_data();
    let root = base.join("root");
    std::fs::create_dir_all(&root).unwrap();
    let mut w = zip::ZipWriter::new(std::fs::File::create(base.join("s.zip")).unwrap());
    w.add_symlink("link", "/etc/passwd", zip::write::SimpleFileOptions::default()).unwrap();
    w.finish().unwrap();
    assert!(net::extract_zip(&base.join("s.zip"), &root, &same).unwrap_err().to_string().contains("symlink"));
}

#[test]
fn extraction_will_not_write_through_a_symlinked_directory() {
    let (root, z) = write_zip(&[("d/x.txt", "x")]);
    let outside = tmp_data();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside, root.join("d")).unwrap();
        assert!(net::extract_zip(&z, &root, &same).is_err());
        assert!(!outside.join("x.txt").exists());
    }
    let _ = (root, z, outside);
}

// ---- launch arguments

fn cfg(kv: &[(&str, &str)]) -> HashMap<String, String> { kv.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() }
fn launch(id: &str, c: &HashMap<String, String>) -> Vec<String> { (games::get(id).unwrap().launch)(&games::Launch { port: 27015, max_players: 16, name: "n", cfg: c }).unwrap() }

#[test]
fn workshop_ids_are_digits_only() {
    assert!(workshop::valid_id("123456789") && !workshop::valid_id("") && !workshop::valid_id("12a") && !workshop::valid_id("1 2") && !workshop::valid_id("-1") && !workshop::valid_id(&"1".repeat(21)));
}

#[test]
fn gmod_mounts_the_workshop_collection() {
    assert!(!launch("gmod", &cfg(&[])).iter().any(|a| a.contains("workshop")));
    let v = launch("gmod", &cfg(&[("workshop_collection", "123456")]));
    assert_eq!(&v[v.len() - 2..], ["+host_workshop_collection", "123456"]);
    assert!(!launch("gmod", &cfg(&[("workshop_collection", "1;quit")])).iter().any(|a| a.contains("workshop")), "a bad id in the file is ignored");
    let v = launch("gmod", &cfg(&[("workshop_collection", "5"), ("steam_api_key", "ABCDEF123")]));
    assert_eq!(&v[v.len() - 4..], ["+host_workshop_collection", "5", "-authkey", "ABCDEF123"]);
}

#[test]
fn cs2_replaces_the_default_map_with_a_workshop_map() {
    let v = launch("cs2", &cfg(&[]));
    assert_eq!(&v[v.len() - 2..], ["+map", "de_dust2"]);
    let v = launch("cs2", &cfg(&[("workshop_map", "3070284539")]));
    assert_eq!(&v[v.len() - 2..], ["+host_workshop_map", "3070284539"]);
    assert!(!v.contains(&"de_dust2".to_string()));
    let v = launch("cs2", &cfg(&[("workshop_map", "x")]));
    assert_eq!(&v[v.len() - 2..], ["+map", "de_dust2"]);
}

#[test]
fn valheim_uses_doorstop_when_bepinex_is_present() {
    let base = &[("password", "hunter22")];
    let plain = launch("valheim", &cfg(base));
    assert_eq!(&plain[..4], ["env", "LD_LIBRARY_PATH=./linux64", "SteamAppId=892970", "./valheim_server.x86_64"]);
    let mut c = cfg(base);
    c.insert(BEPINEX_KEY.into(), "1".into());
    let v = launch("valheim", &c);
    for want in ["DOORSTOP_ENABLED=1", "DOORSTOP_TARGET_ASSEMBLY=./BepInEx/core/BepInEx.Preloader.dll", "LD_LIBRARY_PATH=./doorstop_libs:./linux64", "LD_PRELOAD=libdoorstop_x64.so"] { assert!(v.contains(&want.to_string()), "{want}"); }
    assert!(games::executables(&v).contains(&"valheim_server.x86_64"));
}

// ---- stub providers and installs

async fn serve(f: impl Fn(&str) -> (u16, Vec<u8>) + Send + Sync + 'static) -> String {
    let f = std::sync::Arc::new(f);
    let app = axum::Router::new().fallback(move |req: axum::extract::Request| {
        let f = f.clone();
        async move {
            let (s, b) = f(&req.uri().path_and_query().map(|p| p.to_string()).unwrap_or_default());
            (axum::http::StatusCode::from_u16(s).unwrap(), b)
        }
    });
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { let _ = axum::serve(l, app).await; });
    base
}
fn js(v: Value) -> (u16, Vec<u8>) { (200, v.to_string().into_bytes()) }
fn local(base: &str) -> Providers { Providers { thunderstore: base.into(), factorio: base.into(), allow_local: true } }

fn ts_version(base: &str, ns: &str, name: &str, ver: &str, deps: &[&str]) -> Value {
    json!({"namespace": ns, "name": name, "version_number": ver, "dependencies": deps, "download_url": format!("{base}/dl/{ns}-{name}-{ver}.zip")})
}

#[tokio::test]
async fn thunderstore_install_toggle_and_remove() {
    let bep = zip_bytes(&[("manifest.json", "{}"), ("BepInExPack_Valheim/BepInEx/core/BepInEx.Preloader.dll", "core"), ("BepInExPack_Valheim/BepInEx/config/BepInEx.cfg", "cfg"), ("BepInExPack_Valheim/doorstop_libs/libdoorstop_x64.so", "so")]);
    let modz = zip_bytes(&[("manifest.json", "{}"), ("icon.png", "x"), ("Epic.dll", "dll"), ("assets/a.bundle", "b")]);
    let base = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let b2 = base.clone();
    let url = serve(move |p| {
        let base = b2.lock().unwrap().clone();
        match p {
            "/api/experimental/package/a/Epic/" => js(json!({"latest": ts_version(&base, "a", "Epic", "1.2.0", &["denikson-BepInExPack_Valheim-5.4.2202"])})),
            "/api/experimental/package/denikson/BepInExPack_Valheim/5.4.2202/" => js(ts_version(&base, "denikson", "BepInExPack_Valheim", "5.4.2202", &[])),
            "/dl/a-Epic-1.2.0.zip" => (200, modz.clone()),
            "/dl/denikson-BepInExPack_Valheim-5.4.2202.zip" => (200, bep.clone()),
            _ => (404, vec![]),
        }
    }).await;
    *base.lock().unwrap() = url.clone();
    let (st, dir, tmp) = (state(|_| {}).await, tmp_data(), tmp_data().join("tmp"));
    let ctx = OpCtx::detached(&st);
    let done = service::install(&ctx, &local(&url), &dir, &tmp, Kind::Thunderstore, "a-Epic", None).await.unwrap();
    assert_eq!(done, ["denikson-BepInExPack_Valheim", "a-Epic"]);
    assert!(dir.join("BepInEx/core/BepInEx.Preloader.dll").exists() && dir.join("doorstop_libs/libdoorstop_x64.so").exists());
    assert!(dir.join("BepInEx/plugins/a-Epic/Epic.dll").exists() && dir.join("BepInEx/plugins/a-Epic/assets/a.bundle").exists() && !dir.join("manifest.json").exists());
    let m = Manifest::load(&dir).unwrap();
    assert_eq!((m.get("a-Epic").unwrap().auto, m.get("denikson-BepInExPack_Valheim").unwrap().auto), (false, true));
    assert_eq!(service::installed_json(&dir, Kind::Thunderstore).unwrap().len(), 2);

    service::set_enabled(&dir, Kind::Thunderstore, "a-Epic", false).unwrap();
    assert!(!dir.join("BepInEx/plugins/a-Epic/Epic.dll").exists() && dir.join("BepInEx/plugins/a-Epic/Epic.dll.old").exists());
    assert!(!Manifest::load(&dir).unwrap().get("a-Epic").unwrap().enabled);
    service::set_enabled(&dir, Kind::Thunderstore, "a-Epic", true).unwrap();
    assert!(dir.join("BepInEx/plugins/a-Epic/Epic.dll").exists());
    assert_eq!(service::set_enabled(&dir, Kind::Thunderstore, ts::BEPINEX_ID, false).unwrap_err().code, "game_mods.core");

    assert_eq!(service::remove(&dir, Kind::Thunderstore, ts::BEPINEX_ID).unwrap_err().code, "game_mods.required");
    service::remove(&dir, Kind::Thunderstore, "a-Epic").unwrap();
    assert!(!dir.join("BepInEx/plugins/a-Epic").exists(), "the mod's folder is pruned");
    assert_eq!(service::remove(&dir, Kind::Thunderstore, "a-Epic").unwrap_err().status, 404);
    assert!(dir.join("BepInEx/config/BepInEx.cfg").exists());
}

#[tokio::test]
async fn thunderstore_search_maps_listing_results() {
    let url = serve(|p| if p.starts_with("/api/cyberstorm/listing/valheim/?q=epic") {
        js(json!({"count": 1, "results": [{"namespace": "RandyKnapp", "name": "Epic_Loot", "description": "loot", "download_count": 5, "icon_url": "https://ccdn.thunderstore.io/i.png"}, {"namespace": "../x", "name": "bad"}]}))
    } else { (404, vec![]) }).await;
    let http = net::client(net::THUNDERSTORE_HOSTS, &local(&url));
    let r = ts::search(&http, &local(&url), "epic").await.unwrap();
    assert_eq!(r.len(), 1);
    assert_eq!((r[0]["id"].as_str(), r[0]["title"].as_str()), (Some("RandyKnapp-Epic_Loot"), Some("Epic Loot")));
}

type Hits = std::sync::Arc<std::sync::Mutex<Vec<String>>>;

fn factorio_stub_state(zip: Vec<u8>, sha: String) -> (Hits, impl Fn(&str) -> (u16, Vec<u8>) + Send + Sync + 'static) {
    let hits = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
    let h = hits.clone();
    (hits, move |p: &str| {
        h.lock().unwrap().push(p.to_string());
        if p == "/api/mods/foo/full" {
            js(json!({"name": "foo", "title": "Foo", "releases": [{"version": "1.0.0", "file_name": "foo_1.0.0.zip", "download_url": "/download/foo/abc", "sha1": sha, "info_json": {"dependencies": ["base >= 2.0", "lib >= 1.0", "? opt"]}}]}))
        } else if p == "/api/mods/lib/full" {
            js(json!({"name": "lib", "title": "Lib", "releases": [{"version": "1.1.0", "file_name": "lib_1.1.0.zip", "download_url": "/download/lib/def", "sha1": hex::encode(Sha1::digest(b"libzip")), "info_json": {"dependencies": []}}]}))
        } else if p.starts_with("/download/foo/abc?") && p.contains("username=me") && p.contains("token=sekret") { (200, zip.clone()) }
        else if p.starts_with("/download/lib/def?") { (200, b"libzip".to_vec()) }
        else { (404, vec![]) }
    })
}

#[tokio::test]
async fn factorio_install_verifies_sha1_keeps_mod_list_and_hides_the_token() {
    let zipb = zip_bytes(&[("foo/info.json", "{}")]);
    let (hits, f) = factorio_stub_state(zipb.clone(), hex::encode(Sha1::digest(&zipb)));
    let url = serve(f).await;
    let (st, dir, tmp) = (state(|_| {}).await, tmp_data(), tmp_data());
    let ctx = OpCtx::detached(&st);
    let p = local(&url);
    let e = service::install(&ctx, &p, &dir, &tmp, Kind::Factorio, "foo", None).await.unwrap_err();
    assert!(format!("{e:#}").contains("factorio_username"), "missing credentials are reported: {e:#}");
    std::fs::write(dir.join(games::SETTINGS_FILE), "factorio_username=me\nfactorio_token=sekret\n").unwrap();
    assert_eq!(service::factorio_credentials(&dir), Some(("me".into(), "sekret".into())));
    let done = service::install(&ctx, &p, &dir, &tmp, Kind::Factorio, "foo", None).await.unwrap();
    assert_eq!(done, ["lib", "foo"]);
    assert!(dir.join("mods/foo_1.0.0.zip").exists() && dir.join("mods/lib_1.1.0.zip").exists());
    let list: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("mods/mod-list.json")).unwrap()).unwrap();
    assert_eq!(list["mods"], json!([{"name": "base", "enabled": true}, {"name": "lib", "enabled": true}, {"name": "foo", "enabled": true}]));
    assert!(hits.lock().unwrap().iter().all(|h| !h.contains("/opt/")), "optional dependencies are not fetched");

    service::set_enabled(&dir, Kind::Factorio, "foo", false).unwrap();
    let list: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("mods/mod-list.json")).unwrap()).unwrap();
    assert_eq!(list["mods"][2], json!({"name": "foo", "enabled": false}));
    assert_eq!(service::remove(&dir, Kind::Factorio, "lib").unwrap_err().code, "game_mods.required");
    service::remove(&dir, Kind::Factorio, "foo").unwrap();
    assert!(!dir.join("mods/foo_1.0.0.zip").exists());
    let list: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("mods/mod-list.json")).unwrap()).unwrap();
    assert_eq!(list["mods"].as_array().unwrap().len(), 2);

    // a wrong checksum is refused and leaves nothing behind; the error never carries the token
    let (_, f) = factorio_stub_state(zipb, "00".repeat(20));
    let url = serve(f).await;
    let dir2 = tmp_data();
    std::fs::write(dir2.join(games::SETTINGS_FILE), "factorio_username=me\nfactorio_token=sekret\n").unwrap();
    let e = format!("{:#}", service::install(&ctx, &local(&url), &dir2, &tmp, Kind::Factorio, "foo", None).await.unwrap_err());
    assert!(e.contains("checksum") && !e.contains("sekret"), "{e}");
    assert!(!dir2.join("mods/foo_1.0.0.zip").exists() && !dir2.join("mods/foo_1.0.0.part").exists());
}

#[tokio::test]
async fn downloads_refuse_other_hosts() {
    let st = state(|_| {}).await;
    let ctx = OpCtx::detached(&st);
    let p = Providers::default();
    let http = net::client(net::THUNDERSTORE_HOSTS, &p);
    for bad in ["https://evil.example/x.zip", "https://thunderstore.io.evil.example/x.zip", "http://thunderstore.io/x.zip", "https://169.254.169.254/x"] {
        let e = net::download(&http, &net::Fetch { url: bad, dest: &tmp_data().join("x.zip"), expect: None, hosts: net::THUNDERSTORE_HOSTS }, &p, &ctx.token, |_| {}).await;
        assert!(e.is_err(), "{bad}");
    }
}

#[tokio::test]
async fn factorio_search_filters_the_cached_listing() {
    let url = serve(|p| if p.starts_with("/api/mods?") {
        js(json!({"results": [{"name": "Krastorio2", "title": "Krastorio 2", "owner": "raiguard", "summary": "overhaul", "downloads_count": 9, "latest_release": {"version": "2.1.4"}}, {"name": "flib", "title": "Factorio Library", "owner": "raiguard", "summary": "lib", "downloads_count": 1}]}))
    } else { (404, vec![]) }).await;
    let p = local(&url);
    let http = net::client(net::FACTORIO_HOSTS, &p);
    let r = factorio::search(&http, &p, "krast").await.unwrap();
    assert_eq!((r.len(), r[0]["id"].as_str(), r[0]["version"].as_str()), (1, Some("Krastorio2"), Some("2.1.4")));
    assert_eq!(factorio::search(&http, &p, "").await.unwrap().len(), 2);
}

// ---- API handlers

mod api {
    use super::*;
    use crate::{api::game_mods as h, auth::User, server};
    use axum::{Json, extract::{Path, State}};

    fn owner() -> User { User { id: "u".into(), email: "o@x".into(), role: "owner".into(), token: None, via_token: None, aal: None } }

    async fn add(st: &crate::state::St, id: &str, runtime: &str) -> PathBuf {
        let (i, r) = (id.to_string(), runtime.to_string());
        st.db.call(move |c| { c.execute("INSERT INTO servers(id,name,runtime,game_version,port,memory_mb,installed,created_at) VALUES (?,?,?,?,?,?,1,'t')", rusqlite::params![i, "S", r, "latest", 27015, 1024])?; Ok(()) }).await.unwrap();
        let d = server::get(st, id).await.unwrap().dir(st);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[tokio::test]
    async fn only_game_servers_have_mods_and_the_capability_says_so() {
        let st = state(|_| {}).await;
        add(&st, "p", "paper").await;
        let e = h::list(State(st.clone()), owner(), Path("p".into())).await.unwrap_err();
        assert_eq!(e.code, "capability.unsupported");
        for (rt, want) in [("valheim", true), ("factorio", true), ("gmod", true), ("cs2", true), ("rust", false), ("paper", false)] {
            assert_eq!(crate::runtimes::launch::test_srv(rt, "x", None, "balanced").capabilities().contains(&"game_mods"), want, "{rt}");
        }
    }

    #[tokio::test]
    async fn factorio_install_needs_credentials_and_they_are_never_echoed() {
        let st = state(|_| {}).await;
        let dir = add(&st, "f", "factorio").await;
        let req = || Json(h::InstallReq { id: "foo".into(), version: None });
        let e = h::install(State(st.clone()), owner(), Path("f".into()), req()).await.unwrap_err();
        assert_eq!(e.code, "game_mods.credentials");
        let bad = h::install(State(st.clone()), owner(), Path("f".into()), Json(h::InstallReq { id: "../x".into(), version: None })).await.unwrap_err();
        assert_eq!(bad.code, "validation");
        let _ = h::credentials_put(State(st.clone()), owner(), Path("f".into()), Json(h::CredsReq { username: "me".into(), token: Some("sekret".into()) })).await.unwrap();
        let v = h::list(State(st.clone()), owner(), Path("f".into())).await.unwrap().0;
        assert_eq!(v["credentials"], json!({"username": "me", "has_token": true}));
        assert!(!v.to_string().contains("sekret"));
        assert!(std::fs::read_to_string(dir.join(games::SETTINGS_FILE)).unwrap().contains("factorio_token=sekret"));
        let _ = h::credentials_put(State(st.clone()), owner(), Path("f".into()), Json(h::CredsReq { username: "".into(), token: None })).await.unwrap();
        assert!(service::factorio_credentials(&dir).is_none());
    }

    #[tokio::test]
    async fn workshop_ids_are_validated_and_stored() {
        let st = state(|_| {}).await;
        let dir = add(&st, "g", "gmod").await;
        let put = |c: &str| h::workshop_put(State(st.clone()), owner(), Path("g".into()), Json(h::WorkshopReq { collection: Some(c.into()), map: None }));
        assert_eq!(put("12x").await.unwrap_err().code, "validation");
        let _ = put("987654").await.unwrap();
        assert!(std::fs::read_to_string(dir.join(games::SETTINGS_FILE)).unwrap().contains("workshop_collection=987654"));
        let v = h::list(State(st.clone()), owner(), Path("g".into())).await.unwrap().0;
        assert_eq!((v["workshop"]["field"].as_str(), v["workshop"]["id"].as_str()), (Some("collection"), Some("987654")));
        assert!(v["workshop"]["url"].as_str().unwrap().contains("987654"));
        let wrong = h::workshop_put(State(st.clone()), owner(), Path("g".into()), Json(h::WorkshopReq { collection: None, map: Some("1".into()) })).await.unwrap_err();
        assert_eq!(wrong.code, "validation", "a map is not a gmod setting");
        let _ = put("").await.unwrap();
        assert!(!std::fs::read_to_string(dir.join(games::SETTINGS_FILE)).unwrap().contains("workshop_collection"));
        let e = h::search(State(st.clone()), owner(), Path("g".into()), axum::extract::Query(HashMap::new())).await.unwrap_err();
        assert_eq!(e.code, "capability.unsupported");
    }

    #[tokio::test]
    async fn changes_are_refused_while_the_server_runs_and_unprivileged_users_are_refused() {
        let st = state(|_| {}).await;
        let dir = add(&st, "v", "valheim").await;
        let mut m = Manifest::default();
        m.upsert(Entry { id: "a-A".into(), enabled: true, ..Default::default() });
        m.save(&dir).unwrap();
        let viewer = User { id: "nobody".into(), email: "n@x".into(), role: "user".into(), token: None, via_token: None, aal: None };
        let e = h::delete(State(st.clone()), viewer, Path(("v".into(), "a-A".into()))).await.unwrap_err();
        assert_eq!(e.status, axum::http::StatusCode::NOT_FOUND, "no access looks like no server");
        h::delete(State(st.clone()), owner(), Path(("v".into(), "a-A".into()))).await.unwrap();
        assert!(Manifest::load(&dir).unwrap().mods.is_empty());
    }
}
