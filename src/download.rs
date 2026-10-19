//! Hardened downloader: host allow-list (re-checked on every redirect hop), size cap, streamed hashing,
//! write-to-temp then atomic rename. Used for server jars, Java runtimes and provider files.
use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use sha2::{Digest, Sha256, Sha512};
use std::path::Path;
use tokio::io::AsyncWriteExt;

pub const MAX_BYTES: u64 = 4 << 30;

const ALLOWED: &[&str] = &[
    "mojang.com", "minecraft.net", "papermc.io", "purpurmc.org", "fabricmc.net", "modrinth.com", "adoptium.net",
    "github.com", "githubusercontent.com", "azul.com",
];

pub fn host_allowed(host: &str) -> bool {
    ALLOWED.iter().any(|d| host == *d || host.ends_with(&format!(".{d}")))
}

pub fn redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|a| {
        let ok = a.url().scheme() == "https" && a.url().host_str().is_some_and(host_allowed);
        if a.previous().len() > 5 { a.error("too many redirects") } else if ok { a.follow() } else { a.error("redirect to a host that is not on the allow-list") }
    })
}

pub fn client(contact: &str) -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(format!("moss/{} ({})", env!("CARGO_PKG_VERSION"), if contact.is_empty() { "https://github.com/moss-mc/moss" } else { contact }))
        .redirect(redirect_policy())
        .connect_timeout(std::time::Duration::from_secs(15))
        .read_timeout(std::time::Duration::from_secs(60))
        .build().expect("http client")
}

#[derive(Clone, Debug)]
pub enum Hash { Sha1(String), Sha256(String), Sha512(String) }

enum Hasher { S1(sha1::Sha1), S256(Sha256), S512(Sha512) }

impl Hasher {
    fn new(h: &Option<Hash>) -> Self { match h { Some(Hash::Sha1(_)) => Self::S1(Default::default()), Some(Hash::Sha512(_)) => Self::S512(Default::default()), _ => Self::S256(Default::default()) } }
    fn update(&mut self, b: &[u8]) { match self { Self::S1(h) => h.update(b), Self::S256(h) => h.update(b), Self::S512(h) => h.update(b) } }
    fn finish(self) -> String { match self { Self::S1(h) => hex::encode(h.finalize()), Self::S256(h) => hex::encode(h.finalize()), Self::S512(h) => hex::encode(h.finalize()) } }
}

/// Returns the hex SHA-256 (or the algorithm used for verification when one is provided) of the downloaded file.
pub async fn fetch(http: &reqwest::Client, url: &str, dest: &Path, expect: Option<Hash>, token: &tokio_util::sync::CancellationToken, progress: impl Fn(f64)) -> Result<String> {
    let u = reqwest::Url::parse(url).context("bad url")?;
    if u.scheme() != "https" || !u.host_str().is_some_and(host_allowed) { bail!("refusing to download from {}", u.host_str().unwrap_or("?")); }
    let resp = http.get(u).send().await?.error_for_status()?;
    let total = resp.content_length();
    if total.is_some_and(|t| t > MAX_BYTES) { bail!("download is larger than the safety limit"); }
    if let Some(p) = dest.parent() { tokio::fs::create_dir_all(p).await?; }
    let part = dest.with_extension("part");
    let mut f = tokio::fs::File::create(&part).await?;
    let mut hasher = Hasher::new(&expect);
    let (mut done, mut stream) = (0u64, resp.bytes_stream());
    let res: Result<()> = async {
        while let Some(chunk) = stream.next().await {
            if token.is_cancelled() { bail!("cancelled"); }
            let chunk = chunk?;
            done += chunk.len() as u64;
            if done > MAX_BYTES { bail!("download exceeded the safety limit"); }
            hasher.update(&chunk);
            f.write_all(&chunk).await?;
            if let Some(t) = total { progress(done as f64 / t as f64); }
        }
        f.flush().await?;
        Ok(())
    }.await;
    drop(f);
    if let Err(e) = res { let _ = tokio::fs::remove_file(&part).await; return Err(e); }
    let got = hasher.finish();
    if let Some(h) = expect {
        let want = match h { Hash::Sha1(s) | Hash::Sha256(s) | Hash::Sha512(s) => s };
        if !got.eq_ignore_ascii_case(&want) { let _ = tokio::fs::remove_file(&part).await; bail!("checksum mismatch (expected {want}, got {got})"); }
    }
    tokio::fs::rename(&part, dest).await?;
    Ok(got)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn allow_list_matches_domains_not_suffix_tricks() {
        assert!(host_allowed("cdn.modrinth.com") && host_allowed("fill-data.papermc.io") && host_allowed("github.com"));
        assert!(!host_allowed("evilmodrinth.com") && !host_allowed("modrinth.com.evil.io") && !host_allowed("169.254.169.254") && !host_allowed("localhost"));
    }
}
