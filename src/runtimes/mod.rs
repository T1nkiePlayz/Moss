//! Runtime catalogue, version discovery and download resolution. Uses official metadata APIs only
//! (Mojang manifest, PaperMC Fill, Purpur, Fabric Meta, NeoForge/Forge maven, Quilt Meta) and verifies checksums
//! when upstream publishes them. Forge, NeoForge and Quilt live in `modded`; launch arguments in `launch`.
use crate::{download::Hash, state::St};
use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};

pub mod launch;
pub mod modded;

pub struct Resolved { pub url: String, pub hash: Option<Hash>, pub java_major: u32, pub loader_version: Option<String> }

pub fn catalogue(dev: bool) -> Value {
    let mut v = vec![
        json!({"id":"vanilla","name":"Vanilla","category":"official","tier":"stable","content_kind":"datapacks","description":"The official Mojang server. Datapacks only.","installable":true}),
        json!({"id":"paper","name":"Paper","category":"plugins","tier":"stable","content_kind":"plugins","description":"High-performance Spigot fork. The default choice for plugin servers.","installable":true}),
        json!({"id":"purpur","name":"Purpur","category":"plugins","tier":"stable","content_kind":"plugins","description":"Paper fork with extra gameplay configuration.","installable":true}),
        json!({"id":"folia","name":"Folia","category":"plugins","tier":"experimental","content_kind":"plugins","description":"Regionised multithreading. Most plugins are NOT compatible without explicit Folia support.","installable":true}),
        json!({"id":"fabric","name":"Fabric","category":"modded","tier":"stable","content_kind":"mods","description":"Lightweight mod loader. Server-side and both-side mods.","installable":true}),
        json!({"id":"velocity","name":"Velocity","category":"proxy","tier":"stable","content_kind":"proxy_plugins","description":"Modern, fast proxy for linking servers into a network.","installable":true}),
        json!({"id":"custom","name":"Custom command","category":"custom","tier":"experimental","content_kind":"none","description":"Run any server software from an explicit command. Reduced automatic management.","installable":true}),
        json!({"id":"neoforge","name":"NeoForge","category":"modded","tier":"stable","content_kind":"mods","description":"Modern Forge successor for Minecraft 1.20.2 and newer. Runs the official installer.","installable":true}),
        json!({"id":"forge","name":"Forge","category":"modded","tier":"experimental","content_kind":"mods","description":"Minecraft Forge for 1.17 and newer (modern installer). Runs the official installer.","installable":true}),
        json!({"id":"quilt","name":"Quilt","category":"modded","tier":"stable","content_kind":"mods","description":"Fabric-compatible mod loader with its own ecosystem.","installable":true}),
    ];
    if dev { v.push(json!({"id":"sim","name":"Simulated server","category":"custom","tier":"experimental","content_kind":"none","description":"Built-in fake Minecraft server for demos and testing. No Java needed.","installable":true})); }
    Value::Array(v)
}

pub(crate) async fn get_json(st: &St, url: &str) -> Result<Value> {
    Ok(st.http.get(url).send().await.with_context(|| format!("GET {url}"))?.error_for_status()?.json().await?)
}

async fn mojang_manifest(st: &St) -> Result<Value> {
    st.cached("mojang:manifest", 600, || get_json(st, "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json")).await
}

fn stable_name(v: &str) -> bool { !(v.contains("-rc") || v.contains("-pre") || v.contains("snapshot")) }

pub async fn versions(st: &St, runtime: &str) -> Result<Value> {
    let key = format!("versions:{runtime}");
    st.cached(&key, 600, || async {
        let (list, recommended): (Vec<(String, bool)>, Option<String>) = match runtime {
            "vanilla" => {
                let m = mojang_manifest(st).await?;
                let all = m["versions"].as_array().cloned().unwrap_or_default();
                let mut snaps = 0;
                let list = all.iter().filter_map(|v| {
                    let (id, ty) = (v["id"].as_str()?.to_string(), v["type"].as_str()?);
                    if ty == "release" { Some((id, true)) } else if ty == "snapshot" && snaps < 8 { snaps += 1; Some((id, false)) } else { None }
                }).collect();
                (list, m["latest"]["release"].as_str().map(String::from))
            }
            "paper" | "folia" | "velocity" => {
                let v = get_json(st, &format!("https://fill.papermc.io/v3/projects/{runtime}")).await?;
                let mut out = vec![];
                if let Some(groups) = v["versions"].as_object() { for (_, vs) in groups { for x in vs.as_array().into_iter().flatten() { if let Some(s) = x.as_str() { out.push((s.to_string(), runtime == "velocity" && !s.contains("rc") || stable_name(s))); } } } }
                let rec = out.iter().find(|(_, s)| *s).map(|(v, _)| v.clone());
                (out, rec)
            }
            "purpur" => {
                let v = get_json(st, "https://api.purpurmc.org/v2/purpur").await?;
                let out: Vec<(String, bool)> = v["versions"].as_array().into_iter().flatten().rev().filter_map(|x| x.as_str().map(|s| (s.to_string(), true))).collect();
                let rec = out.first().map(|(v, _)| v.clone());
                (out, rec)
            }
            "fabric" => {
                let v = get_json(st, "https://meta.fabricmc.net/v2/versions/game").await?;
                let out: Vec<(String, bool)> = v.as_array().into_iter().flatten().filter_map(|x| Some((x["version"].as_str()?.to_string(), x["stable"].as_bool().unwrap_or(false)))).collect();
                let rec = out.iter().find(|(_, s)| *s).map(|(v, _)| v.clone());
                (out, rec)
            }
            "neoforge" | "forge" | "quilt" => return Ok(modded::to_json(&modded::catalogue(st, runtime).await?)),
            "sim" => (vec![("1.0".into(), true)], Some("1.0".into())),
            "custom" => (vec![("custom".into(), true)], Some("custom".into())),
            other => return Err(anyhow!("runtime {other} has no installable versions")),
        };
        Ok(json!({"versions": list.iter().map(|(id, stable)| json!({"id": id, "stable": stable})).collect::<Vec<_>>(), "recommended": recommended}))
    }).await
}

pub fn fallback_java(v: &str) -> u32 {
    let nums: Vec<u32> = v.split(|c: char| !c.is_ascii_digit()).filter_map(|x| x.parse().ok()).collect();
    match (nums.first().copied().unwrap_or(1), nums.get(1).copied().unwrap_or(0), nums.get(2).copied().unwrap_or(0)) {
        (1, m, p) if m > 20 || (m == 20 && p >= 5) => 21,
        (1, m, _) if m >= 17 => 17,
        (1, _, _) => 8,
        _ => 25,
    }
}

/// Java major required by a Minecraft version, from Mojang's per-version metadata.
pub async fn java_for_game(st: &St, game: &str) -> u32 {
    let r: Result<u32> = async {
        let m = mojang_manifest(st).await?;
        let url = m["versions"].as_array().and_then(|a| a.iter().find(|v| v["id"] == game)).and_then(|v| v["url"].as_str().map(String::from)).ok_or_else(|| anyhow!("unknown version"))?;
        let v = st.cached(&format!("mojang:v:{game}"), 3600, || get_json(st, &url)).await?;
        v["javaVersion"]["majorVersion"].as_u64().map(|x| x as u32).ok_or_else(|| anyhow!("no javaVersion"))
    }.await;
    r.unwrap_or_else(|_| fallback_java(game))
}

pub async fn resolve(st: &St, runtime: &str, game: &str, loader: Option<&str>) -> Result<Resolved> {
    match runtime {
        "vanilla" => {
            let m = mojang_manifest(st).await?;
            let url = m["versions"].as_array().and_then(|a| a.iter().find(|v| v["id"] == game)).and_then(|v| v["url"].as_str().map(String::from)).ok_or_else(|| anyhow!("Minecraft {game} was not found in Mojang's manifest"))?;
            let v = get_json(st, &url).await?;
            let d = &v["downloads"]["server"];
            Ok(Resolved { url: d["url"].as_str().ok_or_else(|| anyhow!("{game} has no dedicated server download"))?.into(), hash: d["sha1"].as_str().map(|s| Hash::Sha1(s.into())), java_major: v["javaVersion"]["majorVersion"].as_u64().map(|x| x as u32).unwrap_or_else(|| fallback_java(game)), loader_version: None })
        }
        "paper" | "folia" | "velocity" => {
            let v = get_json(st, &format!("https://fill.papermc.io/v3/projects/{runtime}/versions/{game}/builds/latest")).await?;
            let d = &v["downloads"]["server:default"];
            let java = if runtime == "velocity" { 21 } else { java_for_game(st, game).await };
            Ok(Resolved { url: d["url"].as_str().ok_or_else(|| anyhow!("no build available for {runtime} {game}"))?.into(), hash: d["checksums"]["sha256"].as_str().map(|s| Hash::Sha256(s.into())), java_major: java, loader_version: v["id"].as_i64().map(|b| format!("build {b}")) })
        }
        "purpur" => Ok(Resolved { url: format!("https://api.purpurmc.org/v2/purpur/{game}/latest/download"), hash: None, java_major: java_for_game(st, game).await, loader_version: None }),
        "fabric" => {
            let loader = match loader { Some(l) => l.to_string(), None => {
                let v = get_json(st, "https://meta.fabricmc.net/v2/versions/loader").await?;
                v.as_array().and_then(|a| a.iter().find(|x| x["stable"] == true)).and_then(|x| x["version"].as_str()).ok_or_else(|| anyhow!("no stable Fabric loader"))?.to_string()
            } };
            let inst = get_json(st, "https://meta.fabricmc.net/v2/versions/installer").await?;
            let inst = inst.as_array().and_then(|a| a.iter().find(|x| x["stable"] == true)).and_then(|x| x["version"].as_str()).ok_or_else(|| anyhow!("no stable Fabric installer"))?.to_string();
            Ok(Resolved { url: format!("https://meta.fabricmc.net/v2/versions/loader/{game}/{loader}/{inst}/server/jar"), hash: None, java_major: java_for_game(st, game).await, loader_version: Some(loader) })
        }
        "neoforge" | "forge" | "quilt" => modded::resolve(st, runtime, game, loader).await,
        other => Err(anyhow!("runtime {other} cannot be installed")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn java_fallback_follows_mojang_requirements() {
        assert_eq!(fallback_java("1.16.5"), 8);
        assert_eq!(fallback_java("1.18.2"), 17);
        assert_eq!(fallback_java("1.20.4"), 17);
        assert_eq!(fallback_java("1.20.5"), 21);
        assert_eq!(fallback_java("1.21.8"), 21);
    }
    #[test]
    fn stable_names() { assert!(stable_name("1.21.8") && !stable_name("1.21.9-rc1") && !stable_name("1.21.9-pre2")); }
    #[test]
    fn catalogue_lists_modded_runtimes_as_installable() {
        let c = catalogue(false);
        for (id, tier) in [("neoforge", "stable"), ("forge", "experimental"), ("quilt", "stable")] {
            let r = c.as_array().unwrap().iter().find(|r| r["id"] == id).unwrap();
            assert_eq!((r["installable"].clone(), r["tier"].as_str(), r["content_kind"].as_str()), (json!(true), Some(tier), Some("mods")));
        }
        assert!(c.as_array().unwrap().iter().all(|r| r["id"] != "sim"));
    }
}
