//! Factorio mods from the official mod portal (mods.factorio.com). The portal gives a SHA-1 per release, which is
//! verified; downloads need the player's portal username and token (`factorio_username` / `factorio_token` in moss-game.cfg).
use super::{Entry, Providers, cmp_version, net};
use crate::files::safe_join;
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{cmp::Ordering, collections::{HashMap, HashSet}, path::Path, sync::Mutex, time::{Duration, Instant}};

const MAX_MODS: usize = 100;
const LIST_PAGES: u32 = 5;
const LIST_TTL: Duration = Duration::from_secs(3600);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DepKind { Required, Optional, Incompatible }

#[derive(Clone, Debug, PartialEq)]
pub struct Dep { pub kind: DepKind, pub name: String, pub req: Option<(String, String)> }

/// Parses "name", "name >= 1.2", "? name", "(?) name", "! name", "~ name" (required, no load order). Unknown
/// prefixes (such as `+`) are treated as optional, so they are never installed automatically.
pub fn parse_dep(s: &str) -> Option<Dep> {
    let s = s.trim();
    let (kind, rest) = if let Some(r) = s.strip_prefix("(?)") { (DepKind::Optional, r) }
        else if let Some(r) = s.strip_prefix('?') { (DepKind::Optional, r) }
        else if let Some(r) = s.strip_prefix('!') { (DepKind::Incompatible, r) }
        else if let Some(r) = s.strip_prefix('~') { (DepKind::Required, r) }
        else if let Some(r) = s.strip_prefix('+') { (DepKind::Optional, r) }
        else { (DepKind::Required, s) };
    let rest = rest.trim();
    let (name, req) = match rest.find(['<', '>', '=']) {
        Some(i) => {
            let tail = &rest[i..];
            let op: String = tail.chars().take_while(|c| "<>=".contains(*c)).collect();
            let ver = tail[op.len()..].trim().to_string();
            (rest[..i].trim(), Some((op, ver)))
        }
        None => (rest, None),
    };
    (!name.is_empty()).then(|| Dep { kind, name: name.into(), req })
}

pub fn satisfies(version: &str, req: &Option<(String, String)>) -> bool {
    let Some((op, v)) = req else { return true };
    let c = cmp_version(version, v);
    match op.as_str() { ">=" => c.is_ge(), ">" => c.is_gt(), "<=" => c.is_le(), "<" => c.is_lt(), "=" | "==" => c == Ordering::Equal, _ => true }
}

/// Parts of the game itself, not portal mods.
pub fn builtin(name: &str) -> bool { matches!(name, "base" | "core" | "space-age" | "quality" | "elevated-rails") }

pub fn valid_name(n: &str) -> bool { !n.is_empty() && n.len() <= 100 && n.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '_' | '-' | '.')) && !n.starts_with('.') }

#[derive(Clone, Debug, PartialEq)]
pub struct Release { pub version: String, pub download_url: String, pub sha1: String, pub file_name: String, pub deps: Vec<Dep> }

#[derive(Clone, Debug, PartialEq)]
pub struct Mod { pub name: String, pub title: String, pub releases: Vec<Release> }

pub fn parse_mod(v: &Value) -> Option<Mod> {
    let name = v["name"].as_str().filter(|n| valid_name(n))?;
    let releases = v["releases"].as_array()?.iter().filter_map(|r| {
        let file_name = r["file_name"].as_str().filter(|f| f.ends_with(".zip") && !f.contains(['/', '\\']))?;
        let deps = r["info_json"]["dependencies"].as_array().map(|a| a.iter().filter_map(|d| d.as_str().and_then(parse_dep)).collect()).unwrap_or_default();
        Some(Release { version: r["version"].as_str()?.into(), download_url: r["download_url"].as_str()?.into(), sha1: r["sha1"].as_str()?.into(), file_name: file_name.into(), deps })
    }).collect();
    Some(Mod { name: name.into(), title: v["title"].as_str().unwrap_or(name).trim().into(), releases })
}

/// The release to install: the exact `want`ed version, or the newest that satisfies `req`.
pub fn pick<'a>(m: &'a Mod, req: &Option<(String, String)>, want: Option<&str>) -> Option<&'a Release> {
    match want {
        Some(w) => m.releases.iter().find(|r| r.version == w),
        None => m.releases.iter().filter(|r| satisfies(&r.version, req)).max_by(|a, b| cmp_version(&a.version, &b.version)),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Chosen { pub name: String, pub title: String, pub release: Release }

/// Names of mods reachable from `root` through required dependencies whose records are not yet in `have`.
pub fn unfetched(root: &str, want: Option<&str>, have: &HashMap<String, Mod>) -> Vec<String> {
    let (mut seen, mut out) = (HashSet::new(), vec![]);
    let mut stack = vec![(root.to_string(), None::<(String, String)>, want.map(String::from))];
    while let Some((n, req, w)) = stack.pop() {
        if builtin(&n) || !seen.insert(n.clone()) { continue; }
        let Some(m) = have.get(&n) else { out.push(n); continue };
        let Some(r) = pick(m, &req, w.as_deref()) else { continue };
        for d in r.deps.iter().filter(|d| d.kind == DepKind::Required) { stack.push((d.name.clone(), d.req.clone(), None)); }
    }
    out.sort();
    out
}

/// Install order (dependencies first, root last) for `root`, skipping required mods already installed at an acceptable
/// version. Fails on a missing release or when a chosen mod is incompatible (`!`) with another chosen or installed one.
/// Dependency cycles are fine: each mod is visited once.
pub fn plan(root: &str, want: Option<&str>, have: &HashMap<String, Mod>, installed: &[Entry]) -> Result<Vec<Chosen>> {
    struct Walk<'a> { have: &'a HashMap<String, Mod>, installed: &'a [Entry], seen: HashSet<String>, out: Vec<Chosen> }
    fn visit(n: &str, req: &Option<(String, String)>, w: Option<&str>, is_root: bool, c: &mut Walk) -> Result<()> {
        if builtin(n) || !c.seen.insert(n.to_string()) { return Ok(()); }
        if !is_root && c.installed.iter().any(|e| e.id == n && satisfies(&e.version, req)) { return Ok(()); }
        let m = c.have.get(n).with_context(|| format!("the required mod {n} was not found on the mod portal"))?;
        let r = pick(m, req, w).with_context(|| match w { Some(w) => format!("{n} has no release {w}"), None => format!("no release of {n} satisfies its requirement") })?;
        for d in r.deps.iter().filter(|d| d.kind == DepKind::Required) { visit(&d.name, &d.req, None, false, c)?; }
        c.out.push(Chosen { name: n.into(), title: m.title.clone(), release: r.clone() });
        Ok(())
    }
    let mut walk = Walk { have, installed, seen: HashSet::new(), out: vec![] };
    visit(root, &None, want, true, &mut walk)?;
    let out = walk.out;
    let names: HashSet<&str> = out.iter().map(|c| c.name.as_str()).chain(installed.iter().map(|e| e.id.as_str())).collect();
    for c in &out {
        for d in c.release.deps.iter().filter(|d| d.kind == DepKind::Incompatible) {
            if names.contains(d.name.as_str()) && d.name != c.name { bail!("{} is incompatible with {}", c.name, d.name); }
        }
    }
    for e in installed {
        for bad in &e.conflicts {
            if out.iter().any(|c| &c.name == bad && c.name != e.id) { bail!("the installed mod {} is incompatible with {bad}", e.id); }
        }
    }
    Ok(out)
}

fn mod_url(p: &Providers, tail: &[&str]) -> Result<reqwest::Url> {
    let mut u = reqwest::Url::parse(&p.factorio).context("bad provider url")?;
    u.path_segments_mut().map_err(|()| anyhow::anyhow!("bad provider url"))?.extend(["api", "mods"]).extend(tail);
    Ok(u)
}

pub async fn fetch_mod(http: &reqwest::Client, p: &Providers, name: &str) -> Result<Option<Mod>> {
    if !valid_name(name) { bail!("invalid mod name"); }
    let u = mod_url(p, &[name, "full"])?;
    Ok(net::get_json(http, u.as_str(), net::FACTORIO_HOSTS, p).await?.and_then(|v| parse_mod(&v)))
}

/// Fetches `root` and everything it requires, then plans the install.
pub async fn resolve(http: &reqwest::Client, p: &Providers, root: &str, want: Option<&str>, installed: &[Entry]) -> Result<Vec<Chosen>> {
    let mut have: HashMap<String, Mod> = HashMap::new();
    loop {
        let need = unfetched(root, want, &have);
        if need.is_empty() { break; }
        for n in need {
            if have.len() >= MAX_MODS { bail!("this mod needs more than {MAX_MODS} mods; refusing to continue"); }
            let m = fetch_mod(http, p, &n).await?.with_context(|| format!("{n} was not found on the mod portal"))?;
            have.insert(n, m);
        }
    }
    plan(root, want, &have, installed)
}

/// Download URL with the player's credentials. Never log this.
pub fn download_url(p: &Providers, r: &Release, user: &str, token: &str) -> Result<String> {
    if !r.download_url.starts_with("/download/") { bail!("unexpected download path from the portal"); }
    let mut u = reqwest::Url::parse(&p.factorio).context("bad provider url")?.join(&r.download_url).context("bad download path")?;
    u.query_pairs_mut().append_pair("username", user).append_pair("token", token);
    Ok(u.into())
}

static LIST: Mutex<Vec<(String, Instant, Vec<Value>)>> = Mutex::new(Vec::new());

/// The portal has no text search, so the most downloaded mods are listed (cached an hour) and filtered here;
/// a query that is an exact mod name is also looked up directly.
pub async fn search(http: &reqwest::Client, p: &Providers, q: &str) -> Result<Vec<Value>> {
    let cached = LIST.lock().unwrap().iter().find(|(b, at, _)| *b == p.factorio && at.elapsed() < LIST_TTL).map(|(_, _, v)| v.clone());
    let list = match cached {
        Some(l) => l,
        None => {
            let mut all = vec![];
            for page in 1..=LIST_PAGES {
                let mut u = mod_url(p, &[])?;
                u.query_pairs_mut().append_pair("page_size", "200").append_pair("page", &page.to_string()).append_pair("sort", "downloads_count").append_pair("sort_order", "desc").append_pair("hide_deprecated", "true");
                let Some(v) = net::get_json(http, u.as_str(), net::FACTORIO_HOSTS, p).await? else { break };
                let rows = v["results"].as_array().cloned().unwrap_or_default();
                let last = rows.len() < 200;
                all.extend(rows);
                if last { break; }
            }
            let mut l = LIST.lock().unwrap();
            l.retain(|(b, _, _)| *b != p.factorio);
            l.push((p.factorio.clone(), Instant::now(), all.clone()));
            all
        }
    };
    let ql = q.to_lowercase();
    let mut rows: Vec<Value> = list.into_iter().filter(|r| ql.is_empty() || ["name", "title", "summary"].iter().any(|k| r[*k].as_str().is_some_and(|s| s.to_lowercase().contains(&ql)))).take(25).collect();
    if !q.is_empty() && valid_name(q) && !rows.iter().any(|r| r["name"] == q) {
        let u = mod_url(p, &[q])?;
        if let Ok(Some(v)) = net::get_json(http, u.as_str(), net::FACTORIO_HOSTS, p).await && v["name"] == q { rows.insert(0, v); }
    }
    Ok(rows.iter().filter_map(|r| {
        let name = r["name"].as_str().filter(|n| valid_name(n))?;
        let version = r["latest_release"]["version"].as_str().or_else(|| r["releases"].as_array().and_then(|a| a.last()).and_then(|x| x["version"].as_str()));
        let mut u = reqwest::Url::parse("https://mods.factorio.com/mod/").ok()?;
        u.path_segments_mut().ok()?.pop_if_empty().push(name);
        Some(json!({"id": name, "title": r["title"].as_str().unwrap_or(name).trim(), "author": r["owner"], "summary": r["summary"].as_str().map(str::trim), "downloads": r["downloads_count"], "icon_url": null, "version": version, "url": u.as_str()}))
    }).collect())
}

fn modlist_path(dir: &Path) -> Result<std::path::PathBuf> { safe_join(dir, "mods/mod-list.json").map_err(|e| anyhow::anyhow!(e.message)) }

fn modlist_read(dir: &Path) -> Result<Vec<Value>> {
    match std::fs::read_to_string(modlist_path(dir)?) {
        Ok(t) => Ok(serde_json::from_str::<Value>(&t).context("mods/mod-list.json is not valid JSON")?["mods"].as_array().cloned().unwrap_or_default()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![json!({"name": "base", "enabled": true})]),
        Err(e) => Err(e.into()),
    }
}

fn modlist_write(dir: &Path, mods: Vec<Value>) -> Result<()> {
    std::fs::create_dir_all(dir.join("mods"))?;
    super::write_atomic(&modlist_path(dir)?, &serde_json::to_vec_pretty(&json!({"mods": mods}))?)?;
    Ok(())
}

/// Sets (adding if needed) a mod's `enabled` flag in mods/mod-list.json, keeping every other entry.
pub fn modlist_set(dir: &Path, name: &str, enabled: bool) -> Result<()> {
    let mut mods = modlist_read(dir)?;
    match mods.iter_mut().find(|m| m["name"] == name) { Some(m) => m["enabled"] = json!(enabled), None => mods.push(json!({"name": name, "enabled": enabled})) }
    modlist_write(dir, mods)
}

pub fn modlist_remove(dir: &Path, name: &str) -> Result<()> {
    let mut mods = modlist_read(dir)?;
    mods.retain(|m| m["name"] != name);
    modlist_write(dir, mods)
}
