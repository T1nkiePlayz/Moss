//! Remote backup destinations. A destination is a folder (mounted NAS/USB) or any S3-compatible bucket. This module
//! owns the storage drivers (`folder`, `s3`), configuration and validation (`validate`), credentials (`secrets`),
//! the destination table (`dest`), the upload/fetch/retention jobs (`jobs`) and the self-test (`probe`).
use anyhow::{Result, bail};
use std::{path::Path, sync::{Arc, atomic::{AtomicU64, Ordering}}};
use tokio_util::sync::CancellationToken;

pub mod api;
pub mod dest;
pub mod folder;
pub mod jobs;
mod md5;
pub mod probe;
pub mod s3;
pub mod secrets;
pub mod validate;

pub use dest::Dest;

/// Shared progress and cancellation for one transfer.
#[derive(Clone)]
pub struct Xfer { pub token: CancellationToken, bytes: Arc<AtomicU64> }
impl Xfer {
    pub fn new() -> Self { Self { token: CancellationToken::new(), bytes: Arc::default() } }
    pub fn with_token(token: CancellationToken) -> Self { Self { token, bytes: Arc::default() } }
    pub fn add(&self, n: u64) { self.bytes.fetch_add(n, Ordering::Relaxed); }
    pub fn set(&self, n: u64) { self.bytes.store(n, Ordering::Relaxed); }
    pub fn done(&self) -> u64 { self.bytes.load(Ordering::Relaxed) }
}
impl Default for Xfer { fn default() -> Self { Self::new() } }

#[derive(Clone, Debug, PartialEq)]
pub struct ObjInfo { pub key: String, pub size: u64, pub etag: Option<String>, pub sha256: Option<String>, pub kms: bool }

/// The object name of a backup on any destination.
pub fn key_for(server_id: &str, backup_id: &str) -> String { format!("{server_id}/{backup_id}.tar.zst") }

pub enum Store { Folder(folder::Folder), S3(Box<s3::S3>) }

impl Store {
    pub async fn put(&self, src: &Path, key: &str, sha: &str, x: &Xfer) -> Result<()> {
        match self { Store::Folder(f) => f.put(src, key, x).await, Store::S3(s) => s.put(src, key, sha, x).await }
    }
    pub async fn head(&self, key: &str) -> Result<Option<ObjInfo>> {
        match self { Store::Folder(f) => f.head(key).await, Store::S3(s) => s.head(key).await }
    }
    pub async fn delete(&self, key: &str) -> Result<()> {
        match self { Store::Folder(f) => f.delete(key).await, Store::S3(s) => s.delete(key).await }
    }
    pub async fn list(&self, prefix: &str, max: usize) -> Result<Vec<ObjInfo>> {
        match self { Store::Folder(f) => f.list(prefix).await.map(|mut v| { v.truncate(max); v }), Store::S3(s) => s.list(prefix, max).await }
    }
    /// Download to `dest`, which only appears once the size and (when given) SHA-256 match; otherwise nothing is left behind.
    pub async fn get(&self, key: &str, dest: &Path, expect: Option<(&str, u64)>, x: &Xfer) -> Result<()> {
        let tmp = dest.with_file_name(format!("{}.fetch", dest.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()));
        let r = match self { Store::Folder(f) => f.get(key, &tmp, x).await, Store::S3(s) => s.get(key, &tmp, x).await };
        let (sha, size) = match r { Ok(v) => v, Err(e) => { let _ = tokio::fs::remove_file(&tmp).await; return Err(e); } };
        if let Some((want, wsize)) = expect {
            if size != wsize { let _ = tokio::fs::remove_file(&tmp).await; bail!("the downloaded copy has the wrong size ({size} bytes, expected {wsize})"); }
            if !sha.eq_ignore_ascii_case(want) { let _ = tokio::fs::remove_file(&tmp).await; bail!("the downloaded copy failed its SHA-256 check; it was discarded"); }
        }
        tokio::fs::rename(&tmp, dest).await?;
        Ok(())
    }
}

/// Compare what the destination reports with what we sent. Size and SHA-256 metadata must match; the ETag is checked
/// when it is an MD5 (single upload) or MD5-of-MD5s (multipart) and the object is not KMS-encrypted.
pub fn check_stored(info: &ObjInfo, size: u64, sha: &str, local: &Path, part: Option<u64>) -> Result<()> {
    if info.size != size { bail!("the destination holds {} bytes but {} were sent", info.size, size); }
    if let Some(s) = &info.sha256 && !s.eq_ignore_ascii_case(sha) { bail!("the destination's SHA-256 does not match the backup"); }
    if info.sha256.is_none() && info.etag.is_none() { bail!("the destination returned neither a checksum nor an ETag to verify against"); }
    if let Some(e) = &info.etag && !info.kms && !e.is_empty() {
        let want = etag_for(local, size, part)?;
        let plain = e.trim_matches('"');
        let looks_md5 = plain.len() >= 32 && plain.chars().take(32).all(|c| c.is_ascii_hexdigit());
        if looks_md5 && !plain.eq_ignore_ascii_case(&want) { bail!("the destination's ETag does not match the uploaded data"); }
    }
    Ok(())
}

/// The ETag S3 would report: MD5 of the file, or `md5(concat(md5(part)))-N` for multipart uploads.
fn etag_for(local: &Path, size: u64, part: Option<u64>) -> Result<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(local)?;
    let mut buf = vec![0u8; 1 << 20];
    match part.filter(|p| size > *p) {
        None => { let mut m = md5::Md5::new(); loop { let n = f.read(&mut buf)?; if n == 0 { break; } m.update(&buf[..n]); } Ok(hex::encode(m.finish())) }
        Some(p) => {
            let (mut all, mut left, mut n) = (md5::Md5::new(), size, 0);
            while left > 0 {
                let mut m = md5::Md5::new();
                let mut want = p.min(left);
                left -= want;
                while want > 0 { let k = f.read(&mut buf[..want.min(1 << 20) as usize])?; if k == 0 { bail!("the local archive changed during upload"); } m.update(&buf[..k]); want -= k as u64; }
                all.update(&m.finish());
                n += 1;
            }
            Ok(format!("{}-{n}", hex::encode(all.finish())))
        }
    }
}
