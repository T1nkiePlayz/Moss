//! Modrinth `.mrpack` modpacks: `modrinth.index.json` (files with hashes, download URLs and per-side environment,
//! dependencies for Minecraft and the loader) plus `overrides/` and `server-overrides/`.
use super::modpack::{Pack, PackFile, Scanned, Side, Unavailable, zipsafe};
use anyhow::{Context, Result, anyhow, bail};
use serde_json::Value;
use std::io::{Read, Seek};
use zip::ZipArchive;

pub const INDEX: &str = "modrinth.index.json";
const MAX_INDEX: u64 = 16 << 20;

/// Loader dependency keys of the format and the loader id Moss uses for them.
const LOADERS: [(&str, &str); 4] = [("neoforge", "neoforge"), ("forge", "forge"), ("quilt-loader", "quilt"), ("fabric-loader", "fabric")];

/// `https://cdn.modrinth.com/data/<project>/versions/<version>/<file>` carries the ids needed to link updates.
pub fn modrinth_ids(url: &str) -> Option<(String, String)> {
    let u = reqwest::Url::parse(url).ok()?;
    if u.host_str() != Some("cdn.modrinth.com") { return None; }
    let seg: Vec<&str> = u.path_segments()?.collect();
    match seg.as_slice() { ["data", p, "versions", v, _] if !p.is_empty() && !v.is_empty() => Some((p.to_string(), v.to_string())), _ => None }
}

fn side(v: &Value) -> Side { match v.as_str() { Some("unsupported") => Side::Unsupported, Some("optional") => Side::Optional, _ => Side::Required } }

/// Parse the index JSON. Unsafe paths and malformed documents are errors; files whose hosts are not allowed are
/// reported as unavailable instead of being fetched.
pub fn parse_index(idx: &Value) -> Result<Pack> {
    if idx["formatVersion"] != 1 { bail!("unsupported .mrpack format version {}", idx["formatVersion"]); }
    if idx["game"].as_str().is_some_and(|g| g != "minecraft") { bail!("this pack is not for Minecraft"); }
    let deps = &idx["dependencies"];
    let game_version = deps["minecraft"].as_str().ok_or_else(|| anyhow!("the pack does not name a Minecraft version"))?.to_string();
    let (loader, loader_version) = LOADERS.iter().find_map(|(k, id)| deps[*k].as_str().map(|v| (id.to_string(), Some(v.to_string())))).unwrap_or_default();
    let mut pack = Pack { format: "mrpack", name: idx["name"].as_str().unwrap_or("Modpack").chars().take(100).collect(), version: idx["versionId"].as_str().unwrap_or("").chars().take(60).collect(),
        summary: idx["summary"].as_str().map(String::from), game_version, loader, loader_version, ..Default::default() };
    for f in idx["files"].as_array().into_iter().flatten() {
        let path = zipsafe::clean_rel(f["path"].as_str().unwrap_or_default())?;
        let (sha1, sha512) = (f["hashes"]["sha1"].as_str().map(String::from), f["hashes"]["sha512"].as_str().map(String::from));
        if sha1.is_none() && sha512.is_none() { bail!("{path} has no hash; refusing to import unverifiable files"); }
        let size = f["fileSize"].as_u64().unwrap_or(0);
        if size > zipsafe::MAX_ENTRY { bail!("{path} is larger than the per-file limit"); }
        let urls: Vec<String> = f["downloads"].as_array().into_iter().flatten().filter_map(|u| u.as_str()).filter(|u| url_allowed(u)).map(String::from).collect();
        if urls.is_empty() {
            let hosts: Vec<String> = f["downloads"].as_array().into_iter().flatten().filter_map(|u| u.as_str()).filter_map(|u| reqwest::Url::parse(u).ok()?.host_str().map(String::from)).collect();
            pack.unavailable.push(Unavailable { name: path, reason: format!("download host not allowed ({})", if hosts.is_empty() { "none listed".into() } else { hosts.join(", ") }), url: None });
            continue;
        }
        let provider = urls.iter().find_map(|u| modrinth_ids(u)).map(|(p, v)| ("modrinth".to_string(), p, v));
        pack.files.push(PackFile { path, urls, sha1, sha512, size, server: side(&f["env"]["server"]), provider, cf: None });
    }
    Ok(pack)
}

/// HTTPS only, and the host must be on the mrpack allow-list (or the general download allow-list).
pub fn url_allowed(u: &str) -> bool {
    reqwest::Url::parse(u).ok().is_some_and(|u| u.scheme() == "https" && u.host_str().is_some_and(crate::download::pack_host_allowed))
}

pub fn parse<R: Read + Seek>(z: &mut ZipArchive<R>) -> Result<Scanned> {
    let idx: Value = {
        let f = z.by_name(INDEX).context("the archive has no modrinth.index.json")?;
        if f.size() > MAX_INDEX { bail!("modrinth.index.json is too large"); }
        serde_json::from_reader(f.take(MAX_INDEX)).context("modrinth.index.json is not valid JSON")?
    };
    let pack = parse_index(&idx)?;
    let declared: u64 = pack.files.iter().map(|f| f.size).sum();
    if declared > zipsafe::MAX_TOTAL { bail!("the pack's files exceed the total size limit"); }
    let (overrides, skipped) = zipsafe::scan_overrides(z, "overrides")?;
    Ok(Scanned { pack, overrides, skipped })
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::modpack::zipsafe::tests::build;
    use serde_json::json;

    pub fn index() -> Value {
        json!({"formatVersion": 1, "game": "minecraft", "versionId": "1.2.0", "name": "Cool Pack", "summary": "A pack",
            "dependencies": {"minecraft": "1.21.1", "neoforge": "21.1.172"},
            "files": [
                {"path": "mods/a.jar", "hashes": {"sha1": "aa", "sha512": "bb"}, "env": {"client": "required", "server": "required"}, "downloads": ["https://cdn.modrinth.com/data/PROJ1/versions/VER1/a.jar"], "fileSize": 10},
                {"path": "mods/client.jar", "hashes": {"sha1": "aa"}, "env": {"client": "required", "server": "unsupported"}, "downloads": ["https://cdn.modrinth.com/data/P2/versions/V2/client.jar"], "fileSize": 5},
                {"path": "mods/gh.jar", "hashes": {"sha512": "cc"}, "downloads": ["https://github.com/x/y/releases/download/1/gh.jar"], "fileSize": 7},
                {"path": "mods/evil.jar", "hashes": {"sha1": "dd"}, "downloads": ["https://evil.example/e.jar", "http://cdn.modrinth.com/data/a/versions/b/e.jar"], "fileSize": 7}]})
    }

    #[test]
    fn parses_dependencies_files_and_sides() {
        let p = parse_index(&index()).unwrap();
        assert_eq!((p.name.as_str(), p.version.as_str(), p.game_version.as_str(), p.loader.as_str(), p.loader_version.as_deref()), ("Cool Pack", "1.2.0", "1.21.1", "neoforge", Some("21.1.172")));
        assert_eq!(p.files.len(), 3);
        assert_eq!(p.files[0].server, Side::Required);
        assert_eq!(p.files[0].provider, Some(("modrinth".into(), "PROJ1".into(), "VER1".into())));
        assert_eq!(p.files[1].server, Side::Unsupported);
        assert!(p.files[2].provider.is_none() && p.files[2].sha512.as_deref() == Some("cc"));
        assert_eq!(p.unavailable.len(), 1);
        assert!(p.unavailable[0].reason.contains("evil.example"));
        assert_eq!(p.runtime(), "neoforge");
    }

    #[test]
    fn other_loaders_and_vanilla_packs() {
        for (key, id, rt) in [("fabric-loader", "fabric", "fabric"), ("quilt-loader", "quilt", "quilt"), ("forge", "forge", "forge")] {
            let p = parse_index(&json!({"formatVersion": 1, "name": "x", "dependencies": {"minecraft": "1.20.1", key: "1.0"}, "files": []})).unwrap();
            assert_eq!((p.loader.as_str(), p.runtime()), (id, rt));
        }
        let v = parse_index(&json!({"formatVersion": 1, "name": "x", "dependencies": {"minecraft": "1.20.1"}, "files": []})).unwrap();
        assert_eq!((v.loader.as_str(), v.runtime(), v.loader_version.as_deref()), ("", "vanilla", None));
    }

    #[test]
    fn malformed_and_hostile_indexes_are_rejected() {
        let mut bad = index();
        bad["formatVersion"] = json!(2);
        assert!(parse_index(&bad).is_err());
        assert!(parse_index(&json!({"formatVersion": 1, "dependencies": {}, "files": []})).is_err());
        for path in ["../x.jar", "/etc/x", "mods/../../x", ".moss/x", "a\\b"] {
            let mut i = index();
            i["files"][0]["path"] = json!(path);
            assert!(parse_index(&i).is_err(), "{path}");
        }
        let mut i = index();
        i["files"][0]["hashes"] = json!({});
        assert!(parse_index(&i).unwrap_err().to_string().contains("no hash"));
    }

    #[test]
    fn host_allow_list_and_modrinth_urls() {
        assert!(url_allowed("https://cdn.modrinth.com/data/a/versions/b/c.jar") && url_allowed("https://raw.githubusercontent.com/x") && url_allowed("https://gitlab.com/x"));
        assert!(!url_allowed("http://cdn.modrinth.com/x") && !url_allowed("https://modrinth.com.evil.io/x") && !url_allowed("https://169.254.169.254/") && !url_allowed("file:///etc/passwd"));
        assert_eq!(modrinth_ids("https://cdn.modrinth.com/data/AABBCC/versions/1234/file.jar"), Some(("AABBCC".into(), "1234".into())));
        assert!(modrinth_ids("https://github.com/data/a/versions/b/c.jar").is_none() && modrinth_ids("https://cdn.modrinth.com/other").is_none());
    }

    #[test]
    fn parses_a_whole_archive_including_overrides() {
        let idx = serde_json::to_vec(&index()).unwrap();
        let mut z = build(&[(INDEX, &idx), ("overrides/config/x.toml", b"abc"), ("server-overrides/config/y.toml", b"de"), ("overrides/server.jar", b"no")]);
        let s = parse(&mut z).unwrap();
        assert_eq!((s.overrides.len(), s.overrides.iter().map(|o| o.size).sum::<u64>(), s.skipped.len()), (2, 5, 1));
        assert!(parse(&mut build(&[("overrides/a", b"x")])).is_err());
        let mut hostile = build(&[(INDEX, &idx), ("overrides/../../escape.txt", b"x")]);
        assert!(parse(&mut hostile).is_err());
    }
}
