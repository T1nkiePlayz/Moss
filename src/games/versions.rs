//! Version discovery and download URLs for the games that install from a direct download (Bedrock, Terraria,
//! Factorio). Parsing is separate from fetching so it can be tested against fixtures.
use super::{Game, Install};
use crate::{download::Hash, runtimes::Resolved, state::St};
use anyhow::{Result, anyhow, bail};
use regex::Regex;
use serde_json::Value;
use std::sync::LazyLock;

const BEDROCK_LINKS: &str = "https://net-secondary.web.minecraft-services.net/api/v1.0/download/links";
const TERRARIA_LIST: &str = "https://terraria.org/api/get/dedicated-servers-names";
const FACTORIO_LATEST: &str = "https://factorio.com/api/latest-releases";
/// Used when terraria.org's listing cannot be fetched. Newest first.
pub const TERRARIA_FALLBACK: &[&str] = &["1.4.5.0", "1.4.4.9"];

static TERRARIA_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"terraria-server-(\d+)\.zip").unwrap());
static BEDROCK_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"bedrock-server-(\d+(?:\.\d+)+)\.zip").unwrap());

/// "1449" -> "1.4.4.9": Terraria's file names are the version with the dots removed (every component is one digit).
pub fn terraria_dotted(digits: &str) -> String { digits.chars().map(String::from).collect::<Vec<_>>().join(".") }

pub fn version_key(v: &str) -> Vec<u64> { v.split('.').filter_map(|p| p.parse().ok()).collect() }

/// Versions found in terraria.org's listing (any string shaped like `terraria-server-1449.zip`), newest first.
pub fn terraria_versions(listing: &Value) -> Vec<String> {
    fn walk(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::String(s) => if let Some(c) = TERRARIA_NAME.captures(s) { out.push(terraria_dotted(&c[1])); },
            Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
            Value::Object(o) => o.values().for_each(|x| walk(x, out)),
            _ => {}
        }
    }
    let mut out = vec![];
    walk(listing, &mut out);
    out.sort_by_key(|v| std::cmp::Reverse(version_key(v)));
    out.dedup();
    out
}

pub fn terraria_url(version: &str) -> Result<String> {
    let digits: String = version.split('.').collect();
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) { bail!("invalid Terraria version {version}"); }
    Ok(format!("https://terraria.org/api/download/pc-dedicated-server/terraria-server-{digits}.zip"))
}

/// `(version, stable)` pairs from factorio.com's latest-releases document: the headless build of each channel.
pub fn factorio_versions(latest: &Value) -> Vec<(String, bool)> {
    let mut out: Vec<(String, bool)> = vec![];
    for (channel, stable) in [("stable", true), ("experimental", false)] {
        if let Some(v) = latest[channel]["headless"].as_str().filter(|v| !v.is_empty()) && !out.iter().any(|(x, _)| x == v) { out.push((v.to_string(), stable)); }
    }
    out
}

pub fn factorio_url(version: &str) -> String { format!("https://factorio.com/get-download/{version}/headless/linux64") }

/// The current Linux server link in Mojang's download-links document: `(version, url)`.
pub fn bedrock_link(links: &Value) -> Option<(String, String)> {
    links["result"]["links"].as_array()?.iter().find(|l| l["downloadType"] == "serverBedrockLinux").and_then(|l| {
        let url = l["downloadUrl"].as_str()?;
        Some((BEDROCK_NAME.captures(url)?[1].to_string(), url.to_string()))
    })
}

pub fn bedrock_url(version: &str) -> String { format!("https://www.minecraft.net/bedrockdedicatedserver/bin-linux/bedrock-server-{version}.zip") }

async fn bedrock_current(st: &St) -> Result<(String, String)> {
    let v = crate::runtimes::get_json(st, BEDROCK_LINKS).await.map_err(|e| anyhow!("could not read Mojang's Bedrock download links ({e:#}); try again later"))?;
    bedrock_link(&v).ok_or_else(|| anyhow!("Mojang's download-links response has no Linux Bedrock server"))
}

/// `(versions, recommended)` for the runtime picker.
pub async fn list(st: &St, g: &Game) -> Result<(Vec<(String, bool)>, Option<String>)> {
    Ok(match g.install {
        Install::Bedrock => { let (v, _) = bedrock_current(st).await?; (vec![(v.clone(), true)], Some(v)) }
        Install::Terraria => {
            let mut v = crate::runtimes::get_json(st, TERRARIA_LIST).await.map(|j| terraria_versions(&j)).unwrap_or_default();
            if v.is_empty() { v = TERRARIA_FALLBACK.iter().map(|s| s.to_string()).collect(); }
            let rec = v.first().cloned();
            (v.into_iter().map(|x| (x, true)).collect(), rec)
        }
        Install::Factorio => {
            let j = crate::runtimes::get_json(st, FACTORIO_LATEST).await.map_err(|e| anyhow!("could not read factorio.com's release list ({e:#})"))?;
            let v = factorio_versions(&j);
            if v.is_empty() { bail!("factorio.com listed no headless release"); }
            let rec = v.iter().find(|(_, s)| *s).or(v.first()).map(|(x, _)| x.clone());
            (v, rec)
        }
        Install::Steam(_) => (vec![("latest".into(), true)], Some("latest".into())),
    })
}

pub async fn resolve(st: &St, g: &Game, version: &str) -> Result<Resolved> {
    let plain = |v: &str| !v.is_empty() && v.chars().all(|c| c.is_ascii_digit() || c == '.');
    let url = match g.install {
        Install::Bedrock => {
            let (cur, url) = bedrock_current(st).await?;
            if version == "latest" || version == cur { url } else if plain(version) { bedrock_url(version) } else { bail!("invalid Bedrock version {version}") }
        }
        Install::Terraria => terraria_url(version)?,
        Install::Factorio => {
            let v = if version == "latest" || version == "stable" { factorio_versions(&crate::runtimes::get_json(st, FACTORIO_LATEST).await?).into_iter().find(|(_, s)| *s).map(|(v, _)| v).ok_or_else(|| anyhow!("no stable Factorio release"))? } else { version.to_string() };
            if !plain(&v) { bail!("invalid Factorio version {v}"); }
            factorio_url(&v)
        }
        Install::Steam(app) => format!("steam://{app}"),
    };
    Ok(Resolved { url, hash: None::<Hash>, java_major: 0, loader_version: None })
}

// ---------- latest-version lookups (update checks) ----------

const STEAMCMD_INFO: &str = "https://api.steamcmd.net/v1/info";
const LOOKUP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// The public-branch build id in a steamcmd.net `info/<appid>` document.
pub fn steam_buildid(doc: &Value, app: u32) -> Option<String> {
    match &doc["data"][app.to_string()]["depots"]["branches"]["public"]["buildid"] {
        Value::String(s) if !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// Newest version in terraria.org's listing document.
pub fn terraria_latest(listing: &Value) -> Option<String> { terraria_versions(listing).into_iter().next() }

/// Which Factorio release to compare with: the experimental one when the server already runs a version newer than
/// the stable release, otherwise the stable one.
pub fn factorio_target(installed: &str, versions: &[(String, bool)]) -> Option<String> {
    let stable = versions.iter().find(|(_, s)| *s).map(|(v, _)| v.clone());
    let exp = versions.iter().find(|(_, s)| !*s).map(|(v, _)| v.clone());
    match (stable, exp) {
        (Some(s), Some(e)) if version_key(installed) > version_key(&s) => Some(e),
        (Some(s), _) => Some(s),
        (None, e) => e,
    }
}

async fn get_json_timed(st: &St, url: &str) -> Result<Value> {
    Ok(st.http.get(url).timeout(LOOKUP_TIMEOUT).send().await.map_err(|e| anyhow!("GET {url}: {e}"))?.error_for_status()?.json().await?)
}

/// The latest public build id for a Steam app, from steamcmd.net (10 s timeout).
pub async fn steam_latest_buildid(st: &St, app: u32) -> Result<String> {
    let doc = get_json_timed(st, &format!("{STEAMCMD_INFO}/{app}")).await?;
    steam_buildid(&doc, app).ok_or_else(|| anyhow!("steamcmd.net has no public build id for app {app}"))
}

/// The raw release document of a direct-download game (Bedrock links, Terraria listing, Factorio releases).
pub async fn release_doc(st: &St, g: &Game) -> Result<Value> {
    let url = match g.install { Install::Bedrock => BEDROCK_LINKS, Install::Terraria => TERRARIA_LIST, Install::Factorio => FACTORIO_LATEST, Install::Steam(_) => bail!("Steam games have no release document") };
    get_json_timed(st, url).await
}

/// The latest version in a release document, as `release_doc` returned it. `installed` picks the Factorio channel.
pub fn latest_in_doc(g: &Game, doc: &Value, installed: &str) -> Option<String> {
    match g.install {
        Install::Bedrock => bedrock_link(doc).map(|(v, _)| v),
        Install::Terraria => terraria_latest(doc),
        Install::Factorio => factorio_target(installed, &factorio_versions(doc)),
        Install::Steam(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn terraria_listing_is_parsed_and_sorted() {
        let j = json!(["terraria-server-1449.zip", "terraria-server-14481.zip", "terraria-server-1450.zip", "junk", "terraria-server-1449.zip"]);
        assert_eq!(terraria_versions(&j), vec!["1.4.5.0", "1.4.4.9", "1.4.4.8.1"]);
        assert!(terraria_versions(&json!({"x": 1})).is_empty());
        assert_eq!(terraria_url("1.4.4.9").unwrap(), "https://terraria.org/api/download/pc-dedicated-server/terraria-server-1449.zip");
        assert!(terraria_url("1.4/../x").is_err());
    }

    #[test]
    fn factorio_latest_releases() {
        let j = json!({"stable": {"alpha": "2.0.55", "headless": "2.0.55"}, "experimental": {"alpha": "2.1.1", "headless": "2.1.1"}});
        assert_eq!(factorio_versions(&j), vec![("2.0.55".to_string(), true), ("2.1.1".to_string(), false)]);
        let same = json!({"stable": {"headless": "2.0.55"}, "experimental": {"headless": "2.0.55"}});
        assert_eq!(factorio_versions(&same).len(), 1);
        assert_eq!(factorio_url("2.0.55"), "https://factorio.com/get-download/2.0.55/headless/linux64");
    }

    #[test]
    fn steamcmd_info_buildid() {
        let j = json!({"data": {"896660": {"depots": {"branches": {"public": {"buildid": "25909431", "timeupdated": "1"}, "beta": {"buildid": "1"}}}}}, "status": "success"});
        assert_eq!(steam_buildid(&j, 896660).as_deref(), Some("25909431"));
        assert_eq!(steam_buildid(&j, 4020), None);
        assert_eq!(steam_buildid(&json!({"data": {"7": {"depots": {"branches": {"public": {"buildid": 42}}}}}}), 7).as_deref(), Some("42"));
        assert_eq!(steam_buildid(&json!({"data": {"7": {"depots": {"branches": {"public": {"buildid": "x"}}}}}}), 7), None);
        assert_eq!(steam_buildid(&json!({"data": []}), 7), None);
    }

    #[test]
    fn factorio_channel_follows_the_installed_version() {
        let v = vec![("2.0.55".to_string(), true), ("2.1.1".to_string(), false)];
        assert_eq!(factorio_target("2.0.50", &v).as_deref(), Some("2.0.55"));
        assert_eq!(factorio_target("2.0.55", &v).as_deref(), Some("2.0.55"));
        assert_eq!(factorio_target("2.1.0", &v).as_deref(), Some("2.1.1"));
        assert_eq!(factorio_target("2.0.55", &[("2.1.1".to_string(), false)]).as_deref(), Some("2.1.1"));
        assert_eq!(factorio_target("1", &[]), None);
    }

    #[test]
    fn bedrock_links_document() {
        let j = json!({"result": {"links": [
            {"downloadType": "serverBedrockWindows", "downloadUrl": "https://www.minecraft.net/bedrockdedicatedserver/bin-win/bedrock-server-1.21.124.2.zip"},
            {"downloadType": "serverBedrockLinux", "downloadUrl": "https://www.minecraft.net/bedrockdedicatedserver/bin-linux/bedrock-server-1.21.124.2.zip"}]}});
        let (v, u) = bedrock_link(&j).unwrap();
        assert_eq!(v, "1.21.124.2");
        assert!(u.contains("bin-linux"));
        assert!(bedrock_link(&json!({"result": {"links": []}})).is_none());
    }
}
