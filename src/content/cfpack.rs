//! CurseForge modpack `.zip` (`manifest.json`): file references resolved through the CurseForge batch endpoint.
//! Files whose authors disabled third-party downloads are reported as unavailable with their project page; CDN URLs
//! are never guessed. Nothing resolved is cached: the references are re-resolved when an import runs.
use super::modpack::{CfRef, Pack, PackFile, Scanned, Side, Unavailable, zipsafe};
use crate::{providers::curseforge as cf, state::St};
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::io::{Read, Seek};
use zip::ZipArchive;

const MAX_MANIFEST: u64 = 16 << 20;

/// `forge-47.2.0` -> ("forge", "47.2.0"); only loaders Moss can install are recognised.
pub fn parse_loader(id: &str) -> Option<(String, String)> {
    let (name, ver) = id.split_once('-')?;
    matches!(name, "forge" | "neoforge" | "fabric" | "quilt").then(|| (name.to_string(), ver.to_string()))
}

pub fn parse_manifest(m: &Value) -> Result<(Pack, String)> {
    if m["manifestType"].as_str().is_some_and(|t| t != "minecraftModpack") { bail!("this CurseForge archive is not a modpack"); }
    let game = m["minecraft"]["version"].as_str().context("the manifest does not name a Minecraft version")?.to_string();
    let loaders = m["minecraft"]["modLoaders"].as_array().cloned().unwrap_or_default();
    let chosen = loaders.iter().find(|l| l["primary"] == true).or(loaders.first()).and_then(|l| l["id"].as_str());
    let (loader, loader_version) = match chosen.and_then(parse_loader) { Some((l, v)) => (l, Some(v)), None if chosen.is_some() => bail!("unsupported mod loader {:?}", chosen.unwrap_or_default()), None => Default::default() };
    let mut refs = vec![];
    for f in m["files"].as_array().into_iter().flatten() {
        let (Some(p), Some(i)) = (f["projectID"].as_u64(), f["fileID"].as_u64()) else { bail!("the manifest has a file entry without projectID/fileID") };
        refs.push(CfRef { project_id: p, file_id: i, required: f["required"] != false });
    }
    let dir = zipsafe::clean_rel(m["overrides"].as_str().unwrap_or("overrides"))?;
    if dir.contains('/') { bail!("the overrides folder name must be a single directory"); }
    let pack = Pack { format: "curseforge", name: m["name"].as_str().unwrap_or("Modpack").chars().take(100).collect(), version: m["version"].as_str().unwrap_or("").chars().take(60).collect(),
        summary: None, game_version: game, loader, loader_version, cf_refs: refs, ..Default::default() };
    Ok((pack, dir))
}

pub fn parse<R: Read + Seek>(z: &mut ZipArchive<R>) -> Result<Scanned> {
    let m: Value = {
        let f = z.by_name("manifest.json").context("the archive has no manifest.json")?;
        if f.size() > MAX_MANIFEST { bail!("manifest.json is too large"); }
        serde_json::from_reader(f.take(MAX_MANIFEST)).context("manifest.json is not valid JSON")?
    };
    let (pack, dir) = parse_manifest(&m)?;
    let (overrides, skipped) = zipsafe::scan_overrides(z, &dir)?;
    Ok(Scanned { pack, overrides, skipped })
}

/// Match references to the API's file objects. Pure, so it is tested without the network.
pub fn classify(refs: &[CfRef], files: &[Value]) -> (Vec<PackFile>, Vec<Unavailable>) {
    let (mut ok, mut gone) = (vec![], vec![]);
    for r in refs {
        let page = Some(cf::project_url(r.project_id));
        let Some(f) = files.iter().find(|f| f["id"].as_u64() == Some(r.file_id) && f["modId"].as_u64().is_none_or(|m| m == r.project_id)) else {
            gone.push(Unavailable { name: format!("project {} file {}", r.project_id, r.file_id), reason: "this file no longer exists on CurseForge".into(), url: page });
            continue;
        };
        let name = f["fileName"].as_str().unwrap_or_default();
        if !cf::downloadable(f) {
            gone.push(Unavailable { name: name.into(), reason: "the author disabled third-party downloads; download it from CurseForge and add it in Files".into(), url: page });
            continue;
        }
        if name.is_empty() || name.contains(['/', '\\', '\0']) || name.starts_with('.') {
            gone.push(Unavailable { name: name.into(), reason: "CurseForge returned an unusable file name".into(), url: page });
            continue;
        }
        let is_jar = name.to_ascii_lowercase().ends_with(".jar");
        if !is_jar && !cf::client_only(f) {
            gone.push(Unavailable { name: name.into(), reason: "not a .jar file, so it was not installed; add it manually if it is needed".into(), url: page });
            continue;
        }
        let server = if cf::client_only(f) || !is_jar { Side::Unsupported } else if r.required { Side::Required } else { Side::Optional };
        let sha1 = f["hashes"].as_array().into_iter().flatten().find(|h| h["algo"] == 1).and_then(|h| h["value"].as_str()).map(String::from);
        ok.push(PackFile { path: format!("mods/{name}"), urls: f["downloadUrl"].as_str().map(|u| vec![u.to_string()]).unwrap_or_default(), sha1, sha512: None, size: f["fileLength"].as_u64().unwrap_or(0), server,
            provider: Some(("curseforge".into(), r.project_id.to_string(), r.file_id.to_string())), cf: Some((r.project_id, r.file_id)) });
    }
    (ok, gone)
}

/// Resolve a pack's references with the CurseForge API (one batched call per 100 files, through the shared limiter).
pub async fn resolve(st: &St, token: Option<&str>, refs: &[CfRef]) -> Result<(Vec<PackFile>, Vec<Unavailable>)> {
    if refs.is_empty() { return Ok((vec![], vec![])); }
    let ids: Vec<u64> = refs.iter().map(|r| r.file_id).collect();
    Ok(classify(refs, &cf::files_batch(st, token, &ids).await?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::modpack::zipsafe::tests::build;
    use serde_json::json;

    fn manifest() -> Value {
        json!({"manifestType": "minecraftModpack", "manifestVersion": 1, "name": "CF Pack", "version": "3.1", "author": "me", "overrides": "overrides",
            "minecraft": {"version": "1.20.1", "modLoaders": [{"id": "forge-47.2.0", "primary": true}]},
            "files": [{"projectID": 10, "fileID": 100, "required": true}, {"projectID": 11, "fileID": 110, "required": false}, {"projectID": 12, "fileID": 120, "required": true}, {"projectID": 13, "fileID": 130}, {"projectID": 14, "fileID": 140}]})
    }

    #[test]
    fn parses_manifest_loader_and_references() {
        let (p, dir) = parse_manifest(&manifest()).unwrap();
        assert_eq!((p.name.as_str(), p.game_version.as_str(), p.loader.as_str(), p.loader_version.as_deref(), dir.as_str()), ("CF Pack", "1.20.1", "forge", Some("47.2.0"), "overrides"));
        assert_eq!(p.cf_refs.len(), 5);
        assert!(p.cf_refs[0].required && !p.cf_refs[1].required);
        assert_eq!(p.runtime(), "forge");
        for (id, l, v) in [("neoforge-21.1.172", "neoforge", "21.1.172"), ("fabric-0.15.11", "fabric", "0.15.11"), ("quilt-0.26.4", "quilt", "0.26.4")] { assert_eq!(parse_loader(id), Some((l.into(), v.into()))); }
        assert!(parse_loader("cauldron-1.0").is_none());
    }

    #[test]
    fn rejects_bad_manifests() {
        let mut m = manifest();
        m["manifestType"] = json!("minecraftModList");
        assert!(parse_manifest(&m).is_err());
        let mut m = manifest();
        m["minecraft"]["modLoaders"] = json!([{"id": "liteloader-1.0", "primary": true}]);
        assert!(parse_manifest(&m).is_err());
        let mut m = manifest();
        m["overrides"] = json!("../outside");
        assert!(parse_manifest(&m).is_err());
        let mut m = manifest();
        m["files"] = json!([{"projectID": 1}]);
        assert!(parse_manifest(&m).is_err());
    }

    #[test]
    fn unavailable_files_are_listed_with_their_project_page_and_urls_are_never_guessed() {
        let (p, _) = parse_manifest(&manifest()).unwrap();
        let files = vec![
            json!({"id": 100, "modId": 10, "fileName": "a.jar", "downloadUrl": "https://edge.forgecdn.net/files/1/a.jar", "fileLength": 5, "hashes": [{"algo": 1, "value": "abc"}], "gameVersions": ["1.20.1", "Forge"]}),
            json!({"id": 110, "modId": 11, "fileName": "client.jar", "downloadUrl": "https://edge.forgecdn.net/files/1/c.jar", "gameVersions": ["1.20.1", "Client"]}),
            json!({"id": 120, "modId": 12, "fileName": "closed.jar", "downloadUrl": null, "gameVersions": []}),
            json!({"id": 130, "modId": 13, "fileName": "../x.jar", "downloadUrl": "https://edge.forgecdn.net/x", "gameVersions": []})];
        let (ok, gone) = classify(&p.cf_refs, &files);
        assert_eq!(ok.len(), 2);
        assert_eq!((ok[0].path.as_str(), ok[0].server, ok[0].sha1.as_deref(), ok[0].cf), ("mods/a.jar", Side::Required, Some("abc"), Some((10, 100))));
        assert_eq!(ok[1].server, Side::Unsupported);
        assert_eq!(gone.len(), 3);
        assert_eq!((gone[0].name.as_str(), gone[0].url.as_deref()), ("closed.jar", Some("https://www.curseforge.com/projects/12")));
        assert!(gone[0].reason.contains("disabled third-party downloads"));
        assert!(gone[1].reason.contains("unusable"));
        assert!(gone[2].reason.contains("no longer exists") && gone[2].name.contains("140"));
    }

    #[test]
    fn parses_a_whole_archive_with_a_custom_overrides_folder() {
        let mut m = manifest();
        m["overrides"] = json!("custom");
        let raw = serde_json::to_vec(&m).unwrap();
        let mut z = build(&[("manifest.json", &raw), ("custom/config/a.cfg", b"12"), ("overrides/ignored.txt", b"x")]);
        let s = parse(&mut z).unwrap();
        assert_eq!(s.overrides.len(), 1);
        assert_eq!(s.overrides[0].rel, "config/a.cfg");
        assert!(parse(&mut build(&[("overrides/a", b"x")])).is_err());
    }
}
