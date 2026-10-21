//! Network and archive helpers for game mods: a per-provider host allow-list (re-checked on every redirect hop),
//! size caps, timeouts, streamed hash verification, and zip extraction that cannot write outside the server directory.
use super::Providers;
use crate::{download::Hash, files::safe_join};
use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use serde_json::Value;
use sha2::Digest;
use std::{io::{Read, Write}, path::Path, time::Duration};
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

pub const THUNDERSTORE_HOSTS: &[&str] = &["thunderstore.io"];
pub const FACTORIO_HOSTS: &[&str] = &["factorio.com"];
pub const MAX_DOWNLOAD: u64 = 512 << 20;
pub const MAX_JSON: u64 = 32 << 20;
pub const MAX_UNPACKED: u64 = 2 << 30;
pub const MAX_ENTRIES: usize = 20_000;

fn host_ok(u: &reqwest::Url, hosts: &[&str], allow_local: bool) -> bool {
    let Some(h) = u.host_str() else { return false };
    if allow_local && h == "127.0.0.1" { return true; }
    u.scheme() == "https" && hosts.iter().any(|d| h == *d || h.ends_with(&format!(".{d}")))
}

/// A client that follows redirects only to the provider's own domains.
pub fn client(hosts: &'static [&'static str], p: &Providers) -> reqwest::Client {
    let local = p.allow_local;
    reqwest::Client::builder()
        .user_agent(format!("moss/{}", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::custom(move |a| {
            if a.previous().len() > 5 { a.error("too many redirects") } else if host_ok(a.url(), hosts, local) { a.follow() } else { a.error("redirect to a host that is not the provider's") }
        }))
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(60))
        .build().expect("http client")
}

fn check_url(url: &str, hosts: &[&str], p: &Providers) -> Result<reqwest::Url> {
    let u = reqwest::Url::parse(url).context("bad url")?;
    if !host_ok(&u, hosts, p.allow_local) { bail!("refusing to contact {}", u.host_str().unwrap_or("?")); }
    Ok(u)
}

/// GETs a JSON document (30 s overall, body capped at 32 MiB). `None` for a 404.
pub async fn get_json(http: &reqwest::Client, url: &str, hosts: &[&str], p: &Providers) -> Result<Option<Value>> {
    let u = check_url(url, hosts, p)?;
    let resp = http.get(u).timeout(Duration::from_secs(30)).send().await.map_err(|e| e.without_url()).context("the mod provider could not be reached")?;
    let status = resp.status();
    if status == reqwest::StatusCode::NOT_FOUND { return Ok(None); }
    if !status.is_success() { bail!("the mod provider answered HTTP {status}"); }
    if resp.content_length().is_some_and(|n| n > MAX_JSON) { bail!("the provider's answer is too large"); }
    let mut body = Vec::new();
    let mut s = resp.bytes_stream();
    while let Some(c) = s.next().await {
        body.extend_from_slice(&c.map_err(|e| e.without_url())?);
        if body.len() as u64 > MAX_JSON { bail!("the provider's answer is too large"); }
    }
    Ok(Some(serde_json::from_slice(&body).context("the provider's answer was not JSON")?))
}

/// What to download: the URL (it may carry credentials), where to put it, the expected hash if known, and the permitted hosts.
#[derive(Clone, Copy)]
pub struct Fetch<'a> { pub url: &'a str, pub dest: &'a Path, pub expect: Option<&'a Hash>, pub hosts: &'a [&'a str] }

/// Downloads `url` to `dest` (via a `.part` file), verifying `expect` when the provider gives a hash. The URL may carry
/// credentials, so no error text here ever includes it.
pub async fn download(http: &reqwest::Client, f: &Fetch<'_>, p: &Providers, cancel: &CancellationToken, progress: impl Fn(f64)) -> Result<()> {
    let Fetch { url, dest, expect, hosts } = *f;
    let u = check_url(url, hosts, p)?;
    let resp = http.get(u).send().await.map_err(|e| e.without_url()).context("the download could not start")?;
    if !resp.status().is_success() { bail!("the download failed (HTTP {})", resp.status()); }
    let total = resp.content_length();
    if total.is_some_and(|t| t > MAX_DOWNLOAD) { bail!("download is larger than the safety limit"); }
    if let Some(d) = dest.parent() { tokio::fs::create_dir_all(d).await?; }
    let part = dest.with_extension("part");
    let mut file = tokio::fs::File::create(&part).await?;
    let (mut sha1, mut sha256) = (sha1::Sha1::new(), sha2::Sha256::new());
    let (mut done, mut stream) = (0u64, resp.bytes_stream());
    let res: Result<()> = async {
        while let Some(chunk) = stream.next().await {
            if cancel.is_cancelled() { bail!("cancelled"); }
            let chunk = chunk.map_err(|e| e.without_url())?;
            done += chunk.len() as u64;
            if done > MAX_DOWNLOAD { bail!("download exceeded the safety limit"); }
            sha1.update(&chunk);
            sha256.update(&chunk);
            file.write_all(&chunk).await?;
            if let Some(t) = total { progress(done as f64 / t as f64); }
        }
        file.flush().await?;
        Ok(())
    }.await;
    drop(file);
    let verdict = res.and_then(|()| match expect {
        None => Ok(()),
        Some(Hash::Sha1(w)) => if hex::encode(sha1.finalize()).eq_ignore_ascii_case(w) { Ok(()) } else { bail!("checksum mismatch (SHA-1)") },
        Some(Hash::Sha256(w)) => if hex::encode(sha256.finalize()).eq_ignore_ascii_case(w) { Ok(()) } else { bail!("checksum mismatch (SHA-256)") },
        Some(Hash::Sha512(_)) => bail!("unsupported checksum type"),
    });
    if let Err(e) = verdict { let _ = tokio::fs::remove_file(&part).await; return Err(e); }
    tokio::fs::rename(&part, dest).await?;
    Ok(())
}

/// Extracts `zip_path` under `root`. `map` turns an archive entry name (forward slashes) into the server-relative
/// destination, or `None` to skip it. The whole archive is vetted before anything is written: an entry that is absolute,
/// contains `..`, is a symlink, or an archive that is too big or has too many entries is refused outright. Returns the
/// destination paths written; if writing fails part way the files already written are removed again.
pub fn extract_zip(zip_path: &Path, root: &Path, map: &dyn Fn(&str) -> Option<String>) -> Result<Vec<String>> {
    let mut z = zip::ZipArchive::new(std::fs::File::open(zip_path)?).context("not a valid zip archive")?;
    if z.len() > MAX_ENTRIES { bail!("the archive has too many entries"); }
    let mut plan: Vec<(usize, String)> = vec![];
    let mut total = 0u64;
    for i in 0..z.len() {
        let e = z.by_index(i)?;
        let Some(name) = e.enclosed_name() else { bail!("the archive contains an unsafe path ({}); refusing to extract it", e.name().map(|n| n.escape_default().to_string()).unwrap_or_default()) };
        if e.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) { bail!("the archive contains a symlink ({}); refusing to extract it", e.name().map(|n| n.escape_default().to_string()).unwrap_or_default()); }
        total += e.size();
        if total > MAX_UNPACKED { bail!("the archive unpacks to more than the safety limit"); }
        if e.is_dir() { continue; }
        let name = name.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/");
        let Some(dest) = map(&name) else { continue };
        safe_join(root, &dest).map_err(|e| anyhow::anyhow!(e.message))?;
        plan.push((i, dest));
    }
    // files the archive replaces are moved aside first, so a failed extraction restores them instead of deleting them
    let mut written: Vec<String> = vec![];
    let mut saved: Vec<(std::path::PathBuf, std::path::PathBuf)> = vec![];
    let res: Result<()> = (|| {
        let mut budget = MAX_UNPACKED;
        for (i, dest) in &plan {
            let path = safe_join(root, dest).map_err(|e| anyhow::anyhow!(e.message))?;
            if let Some(d) = path.parent() { std::fs::create_dir_all(d)?; }
            if path.is_file() && !written.contains(dest) {
                let aside = path.with_file_name(format!(".{}.moss-old", path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()));
                std::fs::rename(&path, &aside)?;
                saved.push((path.clone(), aside));
            }
            let mut out = std::fs::File::create(&path)?;
            written.push(dest.clone());
            let n = std::io::copy(&mut z.by_index(*i)?.take(budget + 1), &mut out)?;
            if n > budget { bail!("the archive unpacks to more than the safety limit"); }
            budget -= n;
            out.flush()?;
        }
        Ok(())
    })();
    if let Err(e) = res {
        for w in &written { let _ = std::fs::remove_file(root.join(w)); }
        for (orig, aside) in &saved { let _ = std::fs::rename(aside, orig); }
        return Err(e);
    }
    for (_, aside) in &saved { let _ = std::fs::remove_file(aside); }
    Ok(written)
}
