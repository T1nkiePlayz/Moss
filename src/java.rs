//! Java discovery and managed Temurin runtimes (downloaded on demand, verified by checksum).
use crate::{download, ops::OpCtx, platform, server::{self, Srv}, state::St};
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::{path::{Path, PathBuf}, sync::Mutex, time::Instant};

#[derive(Clone, Debug)]
pub struct Install { pub major: u32, pub path: PathBuf, pub managed: bool, pub version: String }

static CACHE: Mutex<Option<(Instant, Vec<Install>)>> = Mutex::new(None);

fn probe(path: &Path) -> Option<(u32, String)> {
    let out = std::process::Command::new(path).arg("-version").output().ok()?;
    let text = String::from_utf8_lossy(&out.stderr).to_string() + &String::from_utf8_lossy(&out.stdout);
    let v = text.split('"').nth(1)?.to_string();
    let major = if let Some(rest) = v.strip_prefix("1.") { rest.split('.').next()?.parse().ok()? } else { v.split(['.', '-', '+']).next()?.parse().ok()? };
    Some((major, v))
}

fn scan(st: &St) -> Vec<Install> {
    let mut found: Vec<Install> = vec![];
    let mut cands: Vec<(PathBuf, bool)> = vec![];
    if let Ok(rd) = std::fs::read_dir(st.cfg.java_dir()) { for e in rd.flatten() { cands.push((e.path().join(platform::JAVA_BIN_IN_HOME), true)); } }
    cands.extend(platform::system_java_candidates().into_iter().map(|p| (p, false)));
    for (p, managed) in cands {
        if !p.exists() { continue; }
        let real = std::fs::canonicalize(&p).unwrap_or(p.clone());
        if found.iter().any(|f| std::fs::canonicalize(&f.path).unwrap_or(f.path.clone()) == real) { continue; }
        if let Some((major, version)) = probe(&p) { found.push(Install { major, path: p, managed, version }); }
    }
    found.sort_by_key(|i| (i.major, !i.managed));
    found
}

pub fn installs(st: &St, force: bool) -> Vec<Install> {
    let mut c = CACHE.lock().unwrap();
    if !force && let Some((t, v)) = c.as_ref() && t.elapsed().as_secs() < 60 { return v.clone(); }
    let v = scan(st);
    *c = Some((Instant::now(), v.clone()));
    v
}

/// Best match: exact major first (managed preferred), otherwise the smallest newer major.
pub fn find(st: &St, major: u32) -> Option<PathBuf> {
    let all = installs(st, false);
    all.iter().find(|i| i.major == major).or_else(|| all.iter().find(|i| i.major > major)).map(|i| i.path.clone())
}

pub fn json_list(st: &St, force: bool) -> Value {
    json!({"installs": installs(st, force).iter().map(|i| json!({"major": i.major, "path": i.path, "managed": i.managed, "version": i.version})).collect::<Vec<_>>()})
}

pub async fn install_managed(ctx: OpCtx, major: u32) -> Result<()> {
    let st = ctx.st.clone();
    let Some(arch) = platform::adoptium_arch() else { bail!("no Temurin build for architecture {}", std::env::consts::ARCH) };
    ctx.stage("Looking up the latest Temurin release");
    let url = format!("https://api.adoptium.net/v3/assets/latest/{major}/hotspot?architecture={arch}&image_type=jre&os={}", platform::ADOPTIUM_OS);
    let v: Value = st.http.get(url).send().await?.error_for_status()?.json().await?;
    let pkg = &v[0]["binary"]["package"];
    let (link, sum, name) = (pkg["link"].as_str(), pkg["checksum"].as_str(), pkg["name"].as_str());
    let (Some(link), Some(sum), Some(name)) = (link, sum, name) else { bail!("Temurin {major} is not available for this platform") };
    ctx.log(name.to_string());
    let tmp = st.cfg.tmp_dir().join(format!("java-{major}-{}", uuid::Uuid::now_v7()));
    let archive = tmp.join(name);
    ctx.stage("Downloading (sha256-verified)");
    let c2 = ctx.clone();
    download::fetch(&st.http, link, &archive, Some(download::Hash::Sha256(sum.into())), &ctx.token, move |p| c2.progress(Some(p * 0.8))).await?;
    ctx.stage("Extracting");
    let (arch2, tmp2) = (archive.clone(), tmp.clone());
    let out = tokio::task::spawn_blocking(move || extract(&arch2, &tmp2.join("out"))).await??;
    ctx.stage("Checking that no running server needs the old runtime");
    let users = running_users(&st, major).await;
    if !users.is_empty() { let _ = tokio::fs::remove_dir_all(&tmp).await; bail!("java.in_use: Java {major} is used by running servers ({}); stop them first", users.join(", ")); }
    let dest = managed_dir(&st, major);
    tokio::task::spawn_blocking(move || swap_in(&out, &dest)).await??;
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    let st2 = st.clone();
    let _ = tokio::task::spawn_blocking(move || installs(&st2, true)).await;
    ctx.stage(&format!("Java {major} ready"));
    Ok(())
}

pub fn managed_dir(st: &St, major: u32) -> PathBuf { st.cfg.java_dir().join(major.to_string()) }
pub fn managed_bin(st: &St, major: u32) -> PathBuf { managed_dir(st, major).join(platform::JAVA_BIN_IN_HOME) }

/// Names of running Minecraft servers whose Java resolves to `bin` (the managed install being replaced or removed).
pub fn users_of(st: &St, servers: &[Srv], bin: &Path) -> Vec<String> {
    let Ok(target) = std::fs::canonicalize(bin) else { return vec![] };
    servers.iter().filter(|s| s.is_mc() && s.installed && crate::lifecycle::process_alive(s))
        .filter(|s| find(st, s.java_major as u32).and_then(|p| std::fs::canonicalize(p).ok()).is_some_and(|p| p == target))
        .map(|s| s.name.clone()).collect()
}

pub async fn running_users(st: &St, major: u32) -> Vec<String> {
    let servers = server::list_all(st).await.unwrap_or_default();
    let (st2, bin) = (st.clone(), managed_bin(st, major));
    tokio::task::spawn_blocking(move || users_of(&st2, &servers, &bin)).await.unwrap_or_default()
}

/// True when a symlink at `entry` (relative to the extraction root) pointing at `link` stays inside the root.
/// macOS JRE bundles use relative links such as `Contents/MacOS/libjli.dylib -> ../Home/lib/libjli.dylib`.
fn link_stays_inside(entry: &Path, link: &Path) -> bool {
    use std::path::Component::*;
    if link.is_absolute() { return false; }
    let mut depth = entry.parent().map_or(0, |p| p.components().count()) as i64;
    for c in link.components() {
        match c { ParentDir => { depth -= 1; if depth < 0 { return false; } } Normal(_) => depth += 1, CurDir => {}, _ => return false }
    }
    true
}

/// Unpack a Temurin tar.gz (dropping its single top-level directory) into `out`, refusing anything that escapes it.
fn extract(archive: &Path, out: &Path) -> Result<PathBuf> {
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(std::fs::File::open(archive)?));
    std::fs::create_dir_all(out)?;
    for e in tar.entries()? {
        let mut e = e?;
        let p = e.path()?.into_owned();
        let stripped: PathBuf = p.components().skip(1).collect();
        if stripped.as_os_str().is_empty() { continue; }
        if !matches!(e.header().entry_type(), tar::EntryType::Regular | tar::EntryType::Directory | tar::EntryType::Symlink) { continue; }
        if stripped.components().any(|c| !matches!(c, std::path::Component::Normal(_))) { bail!("unsafe path in archive"); }
        let target = out.join(&stripped);
        if let Some(par) = target.parent() { std::fs::create_dir_all(par)?; }
        if e.header().entry_type() == tar::EntryType::Symlink {
            let l = e.link_name()?.map(|l| l.into_owned()).unwrap_or_default();
            if !link_stays_inside(&stripped, &l) { continue; }
        }
        e.unpack(&target)?;
    }
    Ok(out.to_path_buf())
}

/// Replace `dest` with `fresh` by renames: the old tree is moved aside first and restored if the swap fails.
fn swap_in(fresh: &Path, dest: &Path) -> Result<()> {
    if let Some(par) = dest.parent() { std::fs::create_dir_all(par)?; }
    let old = dest.with_extension(format!("old-{}", uuid::Uuid::new_v4().simple()));
    let had = dest.exists();
    if had { std::fs::rename(dest, &old)?; }
    if let Err(e) = std::fs::rename(fresh, dest) {
        if had { let _ = std::fs::rename(&old, dest); }
        return Err(e.into());
    }
    if had { let _ = std::fs::remove_dir_all(&old); }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_safety() {
        let p = Path::new;
        assert!(link_stays_inside(p("Contents/MacOS/libjli.dylib"), p("../Home/lib/libjli.dylib")));
        assert!(link_stays_inside(p("bin/java"), p("java17")));
        assert!(!link_stays_inside(p("bin/java"), p("../../etc/passwd")));
        assert!(!link_stays_inside(p("a"), p("/etc/passwd")));
        assert!(!link_stays_inside(p("a"), p("..")));
    }

    #[test]
    fn swap_replaces_atomically_and_keeps_nothing_behind() {
        let root = std::env::temp_dir().join(format!("moss-java-{}", uuid::Uuid::new_v4()));
        let (dest, fresh) = (root.join("21"), root.join("tmp/out"));
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(dest.join("old"), "1").unwrap();
        std::fs::create_dir_all(&fresh).unwrap();
        std::fs::write(fresh.join("new"), "2").unwrap();
        swap_in(&fresh, &dest).unwrap();
        assert!(dest.join("new").exists() && !dest.join("old").exists());
        let names: Vec<_> = std::fs::read_dir(&root).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert!(names.iter().all(|n| !n.contains("old-")), "{names:?}");
        let _ = std::fs::remove_dir_all(&root);
    }
}
