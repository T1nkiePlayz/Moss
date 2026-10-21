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

fn version_key(v: &str) -> Vec<u64> { v.split('.').filter_map(|p| p.parse().ok()).collect() }

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
