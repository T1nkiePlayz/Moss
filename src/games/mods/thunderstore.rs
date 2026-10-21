//! Valheim mods from Thunderstore. Search uses the community listing endpoint the Thunderstore site itself uses
//! (`/api/cyberstorm/listing/valheim/?q=`); package metadata and versions use the documented experimental package API.
//! Thunderstore publishes no checksums, so downloads are not hash-verified (they are restricted to its own hosts).
use super::{Providers, cmp_version, net};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

pub const BEPINEX_ID: &str = "denikson-BepInExPack_Valheim";
const BEPINEX_DIR: &str = "BepInExPack_Valheim/";
const MAX_PACKAGES: usize = 100;

#[derive(Clone, Debug, PartialEq)]
pub struct DepRef { pub ns: String, pub name: String, pub version: String }
impl DepRef { pub fn id(&self) -> String { id(&self.ns, &self.name) } }

#[derive(Clone, Debug, PartialEq)]
pub struct Pkg { pub ns: String, pub name: String, pub version: String, pub download_url: String, pub deps: Vec<DepRef> }
impl Pkg { pub fn id(&self) -> String { id(&self.ns, &self.name) } }

pub fn id(ns: &str, name: &str) -> String { format!("{ns}-{name}") }

fn part_ok(s: &str) -> bool { !s.is_empty() && s.len() <= 100 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') }

/// "Namespace-Name" as used for mod ids.
pub fn split_id(s: &str) -> Option<(&str, &str)> { s.split_once('-').filter(|(a, b)| part_ok(a) && part_ok(b)) }

pub fn valid_version(v: &str) -> bool { !v.is_empty() && v.len() <= 32 && v.chars().all(|c| c.is_ascii_digit() || c == '.') }

/// "Namespace-Name-1.2.3" as listed in a package's dependencies.
pub fn parse_dep(s: &str) -> Option<DepRef> {
    let mut it = s.rsplitn(3, '-');
    let (version, name, ns) = (it.next()?, it.next()?, it.next()?);
    (part_ok(ns) && part_ok(name) && valid_version(version)).then(|| DepRef { ns: ns.into(), name: name.into(), version: version.into() })
}

/// A version object from the package API.
pub fn parse_version(v: &Value) -> Option<Pkg> {
    let (ns, name, version) = (v["namespace"].as_str()?, v["name"].as_str()?, v["version_number"].as_str()?);
    if !part_ok(ns) || !part_ok(name) || !valid_version(version) { return None; }
    let deps = v["dependencies"].as_array()?.iter().filter_map(|d| d.as_str().and_then(parse_dep)).collect();
    Some(Pkg { ns: ns.into(), name: name.into(), version: version.into(), download_url: v["download_url"].as_str()?.into(), deps })
}

pub async fn fetch_version(http: &reqwest::Client, p: &Providers, ns: &str, name: &str, version: Option<&str>) -> Result<Pkg> {
    if !part_ok(ns) || !part_ok(name) || version.is_some_and(|v| !valid_version(v)) { bail!("invalid package id"); }
    let url = match version { Some(v) => format!("{}/api/experimental/package/{ns}/{name}/{v}/", p.thunderstore), None => format!("{}/api/experimental/package/{ns}/{name}/", p.thunderstore) };
    let v = net::get_json(http, &url, net::THUNDERSTORE_HOSTS, p).await?.with_context(|| format!("{ns}-{name}{} was not found on Thunderstore", version.map(|v| format!("-{v}")).unwrap_or_default()))?;
    let obj = if version.is_some() { &v } else { &v["latest"] };
    parse_version(obj).with_context(|| format!("Thunderstore returned an unreadable record for {ns}-{name}"))
}

/// Dependencies of the packages reachable from `root` that `have` lacks (or has only at an older version than
/// some package asks for), at the highest version anyone asks for.
pub fn missing(root: &str, have: &HashMap<String, Pkg>) -> Vec<DepRef> {
    let (mut seen, mut stack, mut want): (HashSet<String>, Vec<&Pkg>, HashMap<String, DepRef>) = (HashSet::new(), vec![], HashMap::new());
    if let Some(r) = have.get(root) { stack.push(r); seen.insert(root.to_string()); }
    while let Some(p) = stack.pop() {
        for d in &p.deps {
            let key = d.id();
            match have.get(&key) {
                Some(h) if cmp_version(&h.version, &d.version).is_ge() => { if seen.insert(key.clone()) { stack.push(h); } }
                _ => { if want.get(&key).is_none_or(|w| cmp_version(&w.version, &d.version).is_lt()) { want.insert(key, d.clone()); } }
            }
        }
    }
    let mut v: Vec<_> = want.into_values().collect();
    v.sort_by_key(DepRef::id);
    v
}

/// Install order for `root`: dependencies first, the root last, each package once. Dependency cycles do not loop
/// (each package is visited once, so the second visit of a cycle is simply skipped).
pub fn order(root: &str, have: &HashMap<String, Pkg>) -> Result<Vec<Pkg>> {
    fn visit(id: &str, have: &HashMap<String, Pkg>, seen: &mut HashSet<String>, out: &mut Vec<Pkg>) -> Result<()> {
        if !seen.insert(id.to_string()) { return Ok(()); }
        let p = have.get(id).with_context(|| format!("{id} could not be resolved"))?;
        for d in &p.deps { visit(&d.id(), have, seen, out)?; }
        out.push(p.clone());
        Ok(())
    }
    let mut out = vec![];
    visit(root, have, &mut HashSet::new(), &mut out)?;
    Ok(out)
}

/// Fetches everything `root` needs (transitively) and returns the install order.
pub async fn resolve(http: &reqwest::Client, p: &Providers, root: Pkg) -> Result<Vec<Pkg>> {
    let root_id = root.id();
    let mut have = HashMap::from([(root_id.clone(), root)]);
    loop {
        let need = missing(&root_id, &have);
        if need.is_empty() { break; }
        for d in need {
            if have.len() >= MAX_PACKAGES { bail!("this mod needs more than {MAX_PACKAGES} packages; refusing to continue"); }
            have.insert(d.id(), fetch_version(http, p, &d.ns, &d.name, Some(&d.version)).await?);
        }
    }
    order(&root_id, &have)
}

/// Where an archive entry goes for a package. BepInEx itself unpacks into the server root; other mods go under
/// `BepInEx/plugins/<id>/` (or the matching patchers, monomod and core folders), and their config files into
/// `BepInEx/config/` (never overwriting an existing one). Packaging files are dropped. `exists` tells whether a path is taken.
pub fn map_entry(pkg_id: &str, entry: &str, exists: &dyn Fn(&str) -> bool) -> Option<String> {
    if pkg_id == BEPINEX_ID {
        let rest = entry.strip_prefix(BEPINEX_DIR)?;
        if rest.is_empty() || (rest.starts_with("BepInEx/config/") && exists(rest)) { return None; }
        return Some(rest.to_string());
    }
    let entry = entry.strip_prefix("BepInEx/").unwrap_or(entry);
    let (first, rest) = match entry.split_once('/') { Some((f, r)) => (f.to_ascii_lowercase(), Some(r)), None => (entry.to_ascii_lowercase(), None) };
    let Some(rest) = rest else {
        if matches!(first.as_str(), "manifest.json" | "icon.png" | "readme.md" | "changelog.md" | "license" | "license.txt" | "license.md") { return None; }
        return Some(format!("BepInEx/plugins/{pkg_id}/{entry}"));
    };
    if rest.is_empty() { return None; }
    match first.as_str() {
        "plugins" | "patchers" | "monomod" | "core" => Some(format!("BepInEx/{first}/{pkg_id}/{rest}")),
        "config" => { let d = format!("BepInEx/config/{rest}"); if exists(&d) { None } else { Some(d) } }
        _ => Some(format!("BepInEx/plugins/{pkg_id}/{entry}")),
    }
}

/// Is the file one a disabled mod has renamed away (`x.dll` becomes `x.dll.old`)?
pub fn disables(file: &str) -> bool { file.ends_with(".dll") }

pub async fn search(http: &reqwest::Client, p: &Providers, q: &str) -> Result<Vec<Value>> {
    let mut u = reqwest::Url::parse(&format!("{}/api/cyberstorm/listing/valheim/", p.thunderstore)).context("bad provider url")?;
    u.query_pairs_mut().append_pair("q", q).append_pair("ordering", if q.is_empty() { "most-downloaded" } else { "last-updated" }).append_pair("deprecated", "false").append_pair("nsfw", "false");
    let v = net::get_json(http, u.as_str(), net::THUNDERSTORE_HOSTS, p).await?.unwrap_or_default();
    let hits = v["results"].as_array().map(|a| a.as_slice()).unwrap_or_default();
    Ok(hits.iter().filter_map(|r| {
        let (ns, name) = (r["namespace"].as_str()?, r["name"].as_str()?);
        if !part_ok(ns) || !part_ok(name) { return None; }
        let icon = r["icon_url"].as_str().filter(|u| u.starts_with("https://")).map(Value::from).unwrap_or(Value::Null);
        Some(json!({"id": id(ns, name), "title": name.replace('_', " "), "author": ns, "summary": r["description"], "downloads": r["download_count"], "icon_url": icon,
            "url": format!("https://thunderstore.io/c/valheim/p/{ns}/{name}/")}))
    }).collect())
}
