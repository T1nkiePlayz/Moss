//! Forge, NeoForge and Quilt: version discovery from the projects' official metadata and resolution of the
//! official installer jar (with its published checksum side file). Parsing is split from fetching so it can be
//! tested against fixture JSON/XML.
use super::{Resolved, get_json, java_for_game};
use crate::{download::{self, Hash}, state::St};
use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub const NEOFORGE_VERSIONS: &str = "https://maven.neoforged.net/api/maven/versions/releases/net/neoforged/neoforge";
pub const FORGE_PROMOS: &str = "https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json";
pub const FORGE_METADATA: &str = "https://maven.minecraftforge.net/net/minecraftforge/forge/maven-metadata.xml";
pub const QUILT_META: &str = "https://meta.quiltmc.org/v3/versions";

pub fn is_modded(runtime: &str) -> bool { matches!(runtime, "forge" | "neoforge" | "quilt") }

/// Oldest Minecraft minor version Moss offers for Forge (1.12.2); everything before 1.17 uses the legacy launch scheme.
const LEGACY_MIN_MINOR: u64 = 12;
const MODERN_MINOR: u64 = 17;

/// Forge for Minecraft 1.12.2 to 1.16.5: launched with `java -jar forge-<mc>-<build>.jar` (no `@unix_args.txt`), on Java 8.
pub fn legacy_forge_game(game: &str) -> bool { let n = vkey(game); n.len() >= 2 && n[0] == 1 && n[1] < MODERN_MINOR }

/// Java major for the legacy Forge range. 1.16.5 also runs on Java 11 and 16 with newer builds, but Java 8 is the safe choice.
pub const LEGACY_JAVA: u32 = 8;

/// Adds `legacy: true` and an explanatory `note` to the Forge versions that use the legacy scheme.
pub fn annotate_legacy(j: &mut Value) {
    for v in j["versions"].as_array_mut().into_iter().flatten() {
        let Some(id) = v["id"].as_str().map(String::from) else { continue };
        if legacy_forge_game(&id) {
            v["legacy"] = json!(true);
            v["note"] = json!(if id == "1.16.5" { "Legacy Forge, runs on Java 8 (1.16.5 builds also run on Java 11 and 16, but Moss uses Java 8)." } else { "Legacy Forge, runs on Java 8." });
        }
    }
}

/// One loader build for a Minecraft version; `recommended` marks the build Moss suggests.
#[derive(Clone, Debug, PartialEq)]
pub struct LoaderVer { pub id: String, pub recommended: bool }

/// Minecraft version -> loader builds (newest first), plus whether that Minecraft version has a stable build.
pub type Catalogue = Vec<(String, bool, Vec<LoaderVer>)>;

/// Numeric components of a dotted version; `1.20.1-47.3.0` compares as 1,20,1,47,3,0.
pub fn vkey(v: &str) -> Vec<u64> { v.split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty()).filter_map(|p| p.parse().ok()).collect() }

fn is_stable(v: &str) -> bool { !["beta", "alpha", "rc", "snapshot", "pre"].iter().any(|t| v.contains(t)) }

fn plain_mc(v: &str) -> bool { let n = vkey(v); v.chars().all(|c| c.is_ascii_digit() || c == '.') && (2..=3).contains(&n.len()) && n[0] == 1 }

fn newest_first(v: &mut [String]) { v.sort_by_key(|a| std::cmp::Reverse((vkey(a), is_stable(a)))); }

/// NeoForge `A.B.C` targets Minecraft `1.A.B` (`B == 0` means `1.A`); the year-based scheme (26+) maps `A.B[.C]`.
pub fn neoforge_mc(v: &str) -> Option<String> {
    let n = vkey(v);
    let (a, b) = (*n.first()?, *n.get(1)?);
    match a {
        21 | 20 if a == 21 || b >= 2 => Some(if b == 0 { format!("1.{a}") } else { format!("1.{a}.{b}") }),
        26..=40 => match n.get(2).filter(|c| **c != 0 && n.len() > 3) { Some(c) => Some(format!("{a}.{b}.{c}")), None => Some(format!("{a}.{b}")) },
        _ => None,
    }
}

fn assemble(map: BTreeMap<String, Vec<String>>, recommended: impl Fn(&str, &str) -> bool) -> Catalogue {
    let mut mcs: Vec<String> = map.keys().cloned().collect();
    newest_first(&mut mcs);
    mcs.into_iter().map(|mc| {
        let mut vs = map[&mc].clone();
        newest_first(&mut vs);
        vs.dedup();
        let flagged: Vec<bool> = vs.iter().map(|v| recommended(&mc, v)).collect();
        // when upstream flags nothing, the newest stable build is the recommendation
        let pick = flagged.iter().position(|f| *f).or_else(|| vs.iter().position(|v| is_stable(v)));
        let stable = vs.iter().any(|v| is_stable(v));
        let list = vs.into_iter().enumerate().map(|(i, id)| LoaderVer { id, recommended: Some(i) == pick }).collect();
        (mc, stable, list)
    }).collect()
}

pub fn parse_neoforge(v: &Value) -> Catalogue {
    let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for s in v["versions"].as_array().into_iter().flatten().filter_map(|x| x.as_str()) {
        if s.chars().any(|c| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+' | '_'))) { continue; }
        if let Some(mc) = neoforge_mc(s) { map.entry(mc).or_default().push(s.to_string()); }
    }
    assemble(map, |_, _| false)
}

/// `<version>1.20.1-47.3.0</version>` entries from Forge's maven-metadata.xml.
pub fn metadata_versions(xml: &str) -> Vec<String> {
    xml.split("<version>").skip(1).filter_map(|p| p.split("</version>").next()).map(|s| s.trim().to_string()).collect()
}

pub fn parse_forge(promos: &Value, metadata_xml: &str) -> Catalogue {
    let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let promo = |suffix: &str| -> BTreeMap<String, String> {
        promos["promos"].as_object().into_iter().flatten().filter_map(|(k, v)| Some((k.strip_suffix(suffix)?.to_string(), v.as_str()?.to_string()))).collect()
    };
    let (rec, latest) = (promo("-recommended"), promo("-latest"));
    let mut add = |mc: &str, fv: &str| {
        let ok = fv.chars().all(|c| c.is_ascii_digit() || c == '.') && vkey(fv).len() >= 3;
        if plain_mc(mc) && vkey(mc)[1] >= LEGACY_MIN_MINOR && ok { map.entry(mc.to_string()).or_default().push(fv.to_string()); }
    };
    for v in metadata_versions(metadata_xml) { if let Some((mc, fv)) = v.split_once('-') { add(mc, fv); } }
    for (mc, fv) in rec.iter().chain(latest.iter()) { add(mc, fv); }
    for fvs in map.values_mut() { fvs.sort(); fvs.dedup(); }
    assemble(map, |mc, v| rec.get(mc).is_some_and(|r| r == v) || (!rec.contains_key(mc) && latest.get(mc).is_some_and(|l| l == v)))
}

/// Quilt's loader list is global; every stable game version gets the newest stable loaders.
pub fn parse_quilt(games: &Value, loaders: &Value) -> Catalogue {
    let mut ls: Vec<String> = loaders.as_array().into_iter().flatten().filter_map(|x| x["version"].as_str().map(String::from)).collect();
    newest_first(&mut ls);
    let stable: Vec<&String> = ls.iter().filter(|l| is_stable(l)).take(10).collect();
    games.as_array().into_iter().flatten().filter(|g| g["stable"] == true).filter_map(|g| g["version"].as_str()).map(|mc| {
        (mc.to_string(), true, stable.iter().enumerate().map(|(i, l)| LoaderVer { id: (*l).clone(), recommended: i == 0 }).collect())
    }).collect()
}

/// The `/runtimes/{id}/versions` body: Minecraft versions, the recommended one, and `loader_versions`.
pub fn to_json(c: &Catalogue) -> Value {
    let lv: serde_json::Map<String, Value> = c.iter().map(|(mc, _, l)| (mc.clone(), json!(l.iter().map(|x| json!({"id": x.id, "recommended": x.recommended})).collect::<Vec<_>>()))).collect();
    json!({"versions": c.iter().map(|(mc, s, _)| json!({"id": mc, "stable": s})).collect::<Vec<_>>(),
        "recommended": c.iter().find(|(_, s, _)| *s).map(|(mc, _, _)| mc), "loader_versions": lv})
}

pub async fn catalogue(st: &St, runtime: &str) -> Result<Catalogue> {
    Ok(match runtime {
        "neoforge" => parse_neoforge(&get_json(st, NEOFORGE_VERSIONS).await?),
        "forge" => {
            let promos = get_json(st, FORGE_PROMOS).await?;
            let xml = match st.http.get(FORGE_METADATA).send().await { Ok(r) if r.status().is_success() => r.text().await.unwrap_or_default(), _ => String::new() };
            parse_forge(&promos, &xml)
        }
        "quilt" => parse_quilt(&get_json(st, &format!("{QUILT_META}/game")).await?, &get_json(st, &format!("{QUILT_META}/loader")).await?),
        other => bail!("{other} is not a modded runtime"),
    })
}

fn plain(v: &str) -> bool { !v.is_empty() && v.len() <= 40 && v.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-')) }

/// Official installer download URL for a loader build.
pub fn installer_url(runtime: &str, game: &str, loader: &str) -> Option<String> {
    Some(match runtime {
        "neoforge" => format!("https://maven.neoforged.net/releases/net/neoforged/neoforge/{loader}/neoforge-{loader}-installer.jar"),
        "forge" => format!("https://maven.minecraftforge.net/net/minecraftforge/forge/{game}-{loader}/forge-{game}-{loader}-installer.jar"),
        _ => return None,
    })
}

/// First whitespace token of a `.sha1`/`.sha256` side file, as a verifiable hash.
pub fn parse_side_hash(text: &str) -> Option<Hash> {
    let t = text.split_whitespace().next()?.to_ascii_lowercase();
    if !t.chars().all(|c| c.is_ascii_hexdigit()) { return None; }
    match t.len() { 64 => Some(Hash::Sha256(t)), 40 => Some(Hash::Sha1(t)), _ => None }
}

async fn side_hash(st: &St, url: &str) -> Option<Hash> {
    for ext in ["sha256", "sha1"] {
        if let Ok(r) = st.http.get(format!("{url}.{ext}")).send().await && r.status().is_success()
            && let Some(h) = parse_side_hash(&r.text().await.unwrap_or_default()) { return Some(h); }
    }
    None
}

/// Resolve the installer for `runtime`/`game` (and the loader build: the given one or the recommended one).
pub async fn resolve(st: &St, runtime: &str, game: &str, loader: Option<&str>) -> Result<Resolved> {
    if !plain(game) || loader.is_some_and(|l| !plain(l)) { bail!("invalid version"); }
    let cat = catalogue(st, runtime).await?;
    let (_, _, builds) = cat.iter().find(|(mc, _, _)| mc == game).ok_or_else(|| anyhow!("{runtime} does not support Minecraft {game}"))?;
    let lv = match loader {
        Some(l) => builds.iter().find(|b| b.id == l).map(|b| b.id.clone()).ok_or_else(|| anyhow!("{runtime} {l} is not available for Minecraft {game}"))?,
        None => builds.iter().find(|b| b.recommended).or(builds.first()).map(|b| b.id.clone()).ok_or_else(|| anyhow!("no {runtime} build for Minecraft {game}"))?,
    };
    let url = match runtime {
        "quilt" => {
            let inst = get_json(st, &format!("{QUILT_META}/installer")).await?;
            let u = inst.as_array().and_then(|a| a.first()).and_then(|x| x["url"].as_str()).ok_or_else(|| anyhow!("no Quilt installer published"))?.to_string();
            if !reqwest::Url::parse(&u).ok().and_then(|u| u.host_str().map(download::host_allowed)).unwrap_or(false) { bail!("the Quilt installer is hosted outside the allow-list"); }
            u
        }
        _ => installer_url(runtime, game, &lv).ok_or_else(|| anyhow!("unknown runtime"))?,
    };
    let hash = side_hash(st, &url).await;
    let java_major = if runtime == "forge" && legacy_forge_game(game) { LEGACY_JAVA } else { java_for_game(st, game).await };
    Ok(Resolved { url, hash, java_major, loader_version: Some(lv) })
}

#[cfg(test)]
mod tests;
