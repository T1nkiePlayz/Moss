//! Fixture tests for the Forge, NeoForge and Quilt version parsers.
use super::*;

fn ids(l: &[LoaderVer]) -> Vec<&str> { l.iter().map(|x| x.id.as_str()).collect() }

#[test]
fn neoforge_versions_map_to_minecraft() {
    for (v, mc) in [("20.2.3-beta", "1.20.2"), ("20.4.237", "1.20.4"), ("21.0.167", "1.21"), ("21.1.172", "1.21.1"), ("21.11.5", "1.21.11"), ("26.1.0.4-beta", "26.1"), ("26.1.2.7", "26.1.2")] {
        assert_eq!(neoforge_mc(v).as_deref(), Some(mc), "{v}");
    }
    for old in ["20.1.0", "47.1.106", "19.4.0"] { assert!(neoforge_mc(old).is_none(), "{old}"); }
}

#[test]
fn neoforge_catalogue_orders_and_recommends_stable() {
    let c = parse_neoforge(&json!({"isSnapshot": false, "versions": ["20.2.3-beta", "20.2.86", "21.1.9", "21.1.172", "21.1.100", "21.2.0-beta", "47.1.106", "bad version"]}));
    assert_eq!(c.iter().map(|(m, _, _)| m.as_str()).collect::<Vec<_>>(), ["1.21.2", "1.21.1", "1.20.2"]);
    let (_, stable, l) = &c[0];
    assert!(!stable && l.iter().all(|x| !x.recommended));
    let (_, stable, l) = &c[1];
    assert!(stable);
    assert_eq!(ids(l), ["21.1.172", "21.1.100", "21.1.9"]);
    assert_eq!(l.iter().filter(|x| x.recommended).map(|x| x.id.as_str()).collect::<Vec<_>>(), ["21.1.172"]);
    assert_eq!(ids(&c[2].2), ["20.2.86", "20.2.3-beta"]);
}

const FORGE_XML: &str = "<metadata><versioning><versions><version>1.7.10-10.13.4.1614</version><version>1.16.5-36.2.42</version><version>1.17.1-37.1.1</version><version>1.20.1-47.2.0</version><version>1.20.1-47.3.0</version><version>1.20.1-47.3.10</version><version>1.12.2-14.23.5.2860-1.12.2</version></versions></versioning></metadata>";

#[test]
fn forge_catalogue_filters_old_versions_and_uses_promotions() {
    let promos = json!({"homepage": "x", "promos": {"1.20.1-latest": "47.3.10", "1.20.1-recommended": "47.2.0", "1.17.1-latest": "37.1.1", "1.16.5-recommended": "36.2.39", "1.21-latest": "51.0.1"}});
    let c = parse_forge(&promos, FORGE_XML);
    assert_eq!(c.iter().map(|(m, _, _)| m.as_str()).collect::<Vec<_>>(), ["1.21", "1.20.1", "1.17.1"]);
    let (_, _, l) = &c[1];
    assert_eq!(ids(l), ["47.3.10", "47.3.0", "47.2.0"]);
    assert_eq!(l.iter().find(|x| x.recommended).unwrap().id, "47.2.0");
    // no recommended promotion: the "latest" one is recommended
    assert_eq!(c[2].2.iter().find(|x| x.recommended).unwrap().id, "37.1.1");
    assert_eq!(c[0].2[0].id, "51.0.1");
}

#[test]
fn quilt_catalogue_uses_stable_loaders_for_stable_games() {
    let games = json!([{"version": "1.21.1", "stable": true}, {"version": "25w01a", "stable": false}, {"version": "1.20.4", "stable": true}]);
    let loaders = json!([{"version": "0.27.0-beta.1"}, {"version": "0.26.4"}, {"version": "0.26.3"}]);
    let c = parse_quilt(&games, &loaders);
    assert_eq!(c.len(), 2);
    assert_eq!(ids(&c[0].2), ["0.26.4", "0.26.3"]);
    assert!(c[0].2[0].recommended && !c[0].2[1].recommended);
}

#[test]
fn json_shape_matches_the_contract() {
    let j = to_json(&parse_neoforge(&json!({"versions": ["21.1.172"]})));
    assert_eq!(j["recommended"], "1.21.1");
    assert_eq!(j["versions"][0], json!({"id": "1.21.1", "stable": true}));
    assert_eq!(j["loader_versions"]["1.21.1"][0], json!({"id": "21.1.172", "recommended": true}));
}

#[test]
fn installer_urls_and_side_hashes() {
    assert_eq!(installer_url("neoforge", "1.21.1", "21.1.172").unwrap(), "https://maven.neoforged.net/releases/net/neoforged/neoforge/21.1.172/neoforge-21.1.172-installer.jar");
    assert_eq!(installer_url("forge", "1.20.1", "47.2.0").unwrap(), "https://maven.minecraftforge.net/net/minecraftforge/forge/1.20.1-47.2.0/forge-1.20.1-47.2.0-installer.jar");
    assert!(installer_url("quilt", "1.21", "0.26.4").is_none());
    assert!(matches!(parse_side_hash(&format!("{}\n", "ab".repeat(20))), Some(Hash::Sha1(_))));
    assert!(matches!(parse_side_hash(&format!("{}  file.jar", "cd".repeat(32))), Some(Hash::Sha256(_))));
    assert!(parse_side_hash("<html>not found</html>").is_none() && parse_side_hash("").is_none());
    for u in ["https://maven.neoforged.net/x", "https://maven.minecraftforge.net/x", "https://maven.quiltmc.org/x", "https://files.minecraftforge.net/x", "https://meta.quiltmc.org/x"] {
        assert!(download::host_allowed(reqwest::Url::parse(u).unwrap().host_str().unwrap()), "{u}");
    }
}
