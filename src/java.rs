//! Java discovery and managed Temurin runtimes (downloaded on demand, verified by checksum).
use crate::{download, ops::OpCtx, platform, state::St};
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
    let (dest, arch2, tmp2) = (st.cfg.java_dir().join(major.to_string()), archive.clone(), tmp.clone());
    tokio::task::spawn_blocking(move || -> Result<()> {
        let f = std::fs::File::open(&arch2)?;
        let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(f));
        let out = tmp2.join("out");
        std::fs::create_dir_all(&out)?;
        for e in tar.entries()? {
            let mut e = e?;
            let p = e.path()?.into_owned();
            let stripped: PathBuf = p.components().skip(1).collect();
            if stripped.as_os_str().is_empty() { continue; }
            if !matches!(e.header().entry_type(), tar::EntryType::Regular | tar::EntryType::Directory | tar::EntryType::Symlink) { continue; }
            let target = out.join(&stripped);
            if !target.starts_with(&out) || stripped.components().any(|c| matches!(c, std::path::Component::ParentDir)) { bail!("unsafe path in archive"); }
            if let Some(par) = target.parent() { std::fs::create_dir_all(par)?; }
            if e.header().entry_type() == tar::EntryType::Symlink {
                let l = e.link_name()?.map(|l| l.into_owned()).unwrap_or_default();
                if l.is_absolute() || l.components().any(|c| matches!(c, std::path::Component::ParentDir)) { continue; }
            }
            e.unpack(&target)?;
        }
        if dest.exists() { std::fs::remove_dir_all(&dest)?; }
        std::fs::create_dir_all(dest.parent().unwrap())?;
        std::fs::rename(&out, &dest)?;
        Ok(())
    }).await??;
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    installs(&st, true);
    ctx.stage(&format!("Java {major} ready"));
    Ok(())
}
