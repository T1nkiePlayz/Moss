//! A small S3 client (AWS, Backblaze B2, Cloudflare R2, MinIO, Wasabi…) over reqwest/rustls with SigV4 signing:
//! single and multipart upload streamed from the file, streamed download, head, list and delete. Every request
//! pins the resolved address (SSRF guard), refuses redirects, and retries transient failures with backoff.
//! Error text never contains credentials or signatures.
use super::{ObjInfo, Xfer};
use anyhow::{Result, anyhow, bail};
use futures_util::StreamExt;
use reqwest::{Body, Method, StatusCode};
use sha2::{Digest, Sha256};
use std::{io::{Read, Seek, SeekFrom}, path::{Path, PathBuf}, time::Duration};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

pub mod sigv4;
pub mod xml;
#[cfg(test)] mod fake;
#[cfg(test)] mod tests;

use sigv4::{sha256_hex, uri_encode};

const MIN_PART: u64 = 16 << 20;
const MAX_PARTS: u64 = 9000;

#[derive(Clone, Debug)]
pub struct S3Cfg { pub endpoint: String, pub region: String, pub bucket: String, pub prefix: String, pub path_style: bool }

pub struct S3 {
    cfg: S3Cfg, access: String, secret: String, client: reqwest::Client, base: reqwest::Url,
    /// Upload part size; production uses `part_size_for`, tests shrink it.
    pub part_size: Option<u64>,
    /// Pause before retry n (0-based); tests make it tiny.
    pub backoff: Duration,
    pub attempts: u32,
}

#[derive(Debug)]
struct Fail { status: Option<StatusCode>, msg: String, retry: bool }
impl std::fmt::Display for Fail { fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { f.write_str(&self.msg) } }
impl std::error::Error for Fail {}

pub fn part_size_for(total: u64) -> u64 { MIN_PART.max(total.div_ceil(MAX_PARTS).div_ceil(1 << 20) << 20) }

/// Friendly text for the S3 error codes people actually hit.
fn explain(status: StatusCode, code: &str, msg: &str) -> String {
    let h = match code {
        "InvalidAccessKeyId" => "the access key is not recognised by this endpoint",
        "SignatureDoesNotMatch" => "the secret key is wrong, or the region or endpoint does not match the account",
        "NoSuchBucket" => "the bucket does not exist (check the bucket name, region and endpoint)",
        "AccessDenied" | "AllAccessDisabled" => "the key is not allowed to do this; it needs read, write, list and delete on the bucket",
        "RequestTimeTooSkewed" => "this computer's clock is too far off; fix the system time",
        "AuthorizationHeaderMalformed" | "PermanentRedirect" | "IllegalLocationConstraintException" => "the region does not match the bucket",
        "SlowDown" | "ServiceUnavailable" | "InternalError" => "the service is busy, try again later",
        "" if status == StatusCode::FORBIDDEN || status == StatusCode::UNAUTHORIZED => "the keys were rejected or are not allowed to do this; they need read, write, list and delete on the bucket",
        _ => "",
    };
    let m: String = msg.chars().take(160).collect();
    if h.is_empty() { format!("HTTP {}{}{}", status.as_u16(), if code.is_empty() { String::new() } else { format!(" {code}") }, if m.is_empty() { String::new() } else { format!(": {m}") }) } else { format!("HTTP {}{}: {h}", status.as_u16(), if code.is_empty() { String::new() } else { format!(" {code}") }) }
}

impl S3 {
    /// Resolve and pin the endpoint (or `<bucket>.<endpoint>` for virtual-hosted style). Fails for non-public targets unless allowed.
    pub async fn connect(cfg: S3Cfg, access: String, secret: String, allow_private: bool) -> Result<S3> {
        let base = reqwest::Url::parse(&cfg.endpoint).map_err(|_| anyhow!("the endpoint is not a valid URL"))?;
        let mut host = base.host_str().ok_or_else(|| anyhow!("the endpoint has no host"))?.to_string();
        if !cfg.path_style { host = format!("{}.{}", cfg.bucket, host); }
        let port = base.port_or_known_default().unwrap_or(443);
        let addr = crate::notify::webhook::resolve_pinned(&host, port, allow_private, "MOSS_ALLOW_PRIVATE_BACKUP_HOSTS").await?;
        let client = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).connect_timeout(Duration::from_secs(15)).read_timeout(Duration::from_secs(120))
            .resolve(&host, addr).user_agent(concat!("Moss/", env!("CARGO_PKG_VERSION"))).build()?;
        Ok(S3 { cfg, access, secret, client, base, part_size: None, backoff: Duration::from_secs(2), attempts: 4 })
    }

    fn full_key(&self, key: &str) -> String { if self.cfg.prefix.is_empty() { key.to_string() } else { format!("{}/{key}", self.cfg.prefix) } }

    /// (url, host header value, encoded path) for an object key (or the bucket when `key` is None).
    fn target(&self, key: Option<&str>) -> (reqwest::Url, String, String) {
        let mut path = String::new();
        let mut u = self.base.clone();
        if self.cfg.path_style { path.push('/'); path.push_str(&uri_encode(&self.cfg.bucket, true)); }
        else { let _ = u.set_host(Some(&format!("{}.{}", self.cfg.bucket, self.base.host_str().unwrap_or_default()))); }
        match key { Some(k) => { path.push('/'); path.push_str(&uri_encode(&self.full_key(k), false)); } None if path.is_empty() => path.push('/'), None => {} }
        u.set_path(&path);
        u.set_query(None);
        let host = match u.port() { Some(p) => format!("{}:{p}", u.host_str().unwrap_or_default()), None => u.host_str().unwrap_or_default().to_string() };
        (u, host, path)
    }

    /// One signed request. `body`: (stream or bytes, payload sha256, length).
    async fn once(&self, method: Method, key: Option<&str>, query: &[(&str, &str)], headers: &[(&str, String)], body: Option<(Body, String, u64)>) -> Result<reqwest::Response, Fail> {
        let (mut url, host, path) = self.target(key);
        let q: Vec<(String, String)> = query.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        if !q.is_empty() {
            let mut s: Vec<(String, String)> = q.iter().map(|(k, v)| (uri_encode(k, true), uri_encode(v, true))).collect();
            s.sort();
            url.set_query(Some(&s.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&")));
        }
        let amz_date = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
        let hash = body.as_ref().map(|b| b.1.clone()).unwrap_or_else(|| sha256_hex(b""));
        let mut h: Vec<(String, String)> = vec![("host".into(), host), ("x-amz-content-sha256".into(), hash), ("x-amz-date".into(), amz_date.clone())];
        for (k, v) in headers { h.push((k.to_string(), v.clone())); }
        let s = sigv4::sign(method.as_str(), &path, &q, &h, &h[1].1, &self.access, &self.secret, &self.cfg.region, &amz_date);
        let mut rb = self.client.request(method, url);
        for (k, v) in &h { if k != "host" { rb = rb.header(k.as_str(), v.as_str()); } }
        rb = rb.header("authorization", s.authorization);
        if let Some((b, _, len)) = body { rb = rb.header("content-length", len).body(b); }
        let r = rb.send().await.map_err(|e| Fail { status: None, msg: format!("could not reach the endpoint: {}", e.without_url()), retry: true })?;
        if r.status().is_success() { return Ok(r); }
        let st = r.status();
        let text = r.text().await.unwrap_or_default();
        let (code, msg) = (xml::first(&text, "Code").unwrap_or_default(), xml::first(&text, "Message").unwrap_or_default());
        let retry = st.is_server_error() || matches!(st.as_u16(), 408 | 429) || matches!(code.as_str(), "RequestTimeout" | "SlowDown");
        Err(Fail { status: Some(st), msg: explain(st, &code, &msg), retry })
    }

    /// `once` with retries; `mk` builds a fresh body for each attempt.
    async fn call<F, Fut>(&self, x: &Xfer, mut mk: F) -> Result<reqwest::Response>
    where F: FnMut() -> Fut, Fut: std::future::Future<Output = Result<reqwest::Response, Fail>> {
        let mut last = None;
        for i in 0..self.attempts {
            if x.token.is_cancelled() { bail!("cancelled"); }
            if i > 0 { tokio::select! { _ = tokio::time::sleep(self.backoff * 4u32.pow(i - 1)) => {}, _ = x.token.cancelled() => bail!("cancelled") } }
            match mk().await { Ok(r) => return Ok(r), Err(f) if f.retry => last = Some(f), Err(f) => return Err(anyhow::Error::new(f)) }
        }
        Err(anyhow::Error::new(last.unwrap_or(Fail { status: None, msg: "no attempt was made".into(), retry: false })))
    }

    async fn simple(&self, x: &Xfer, method: Method, key: Option<&str>, query: &[(&str, &str)], headers: &[(&str, String)], payload: Vec<u8>) -> Result<reqwest::Response> {
        let (hash, len) = (sha256_hex(&payload), payload.len() as u64);
        self.call(x, || self.once(method.clone(), key, query, headers, Some((Body::from(payload.clone()), hash.clone(), len)))).await
    }

    fn file_body(path: &Path, offset: u64, len: u64, x: &Xfer) -> impl std::future::Future<Output = std::io::Result<Body>> + use<> {
        let (path, x) = (path.to_path_buf(), x.clone());
        async move {
            let mut f = tokio::fs::File::open(path).await?;
            f.seek(SeekFrom::Start(offset)).await?;
            let s = tokio_util::io::ReaderStream::with_capacity(f.take(len), 256 << 10).map(move |c| {
                if x.token.is_cancelled() { return Err(std::io::Error::other("cancelled")); }
                if let Ok(b) = &c { x.add(b.len() as u64); }
                c
            });
            Ok(Body::wrap_stream(s))
        }
    }

    async fn range_hash(path: &Path, offset: u64, len: u64) -> Result<String> {
        let path = path.to_path_buf();
        tokio::task::spawn_blocking(move || {
            let mut f = std::fs::File::open(path)?;
            f.seek(SeekFrom::Start(offset))?;
            let (mut h, mut b, mut left) = (Sha256::new(), vec![0u8; 1 << 20], len);
            while left > 0 { let n = f.read(&mut b[..left.min(1 << 20) as usize])?; if n == 0 { bail!("the local archive changed while it was being uploaded"); } h.update(&b[..n]); left -= n as u64; }
            Ok(hex::encode(h.finalize()))
        }).await?
    }

    /// Upload `src` as `key`, with the archive's SHA-256 stored as object metadata.
    pub async fn put(&self, src: &Path, key: &str, sha: &str, x: &Xfer) -> Result<()> {
        let size = tokio::fs::metadata(src).await?.len();
        let part = self.part_size.unwrap_or_else(|| part_size_for(size));
        let meta = [("x-amz-meta-sha256", sha.to_string()), ("content-type", "application/zstd".to_string())];
        if size <= part {
            let hash = Self::range_hash(src, 0, size).await?;
            let base = x.done();
            self.call(x, || async {
                x.set(base);
                let b = Self::file_body(src, 0, size, x).await.map_err(|e| Fail { status: None, msg: format!("could not read the local archive: {e}"), retry: false })?;
                self.once(Method::PUT, Some(key), &[], &meta, Some((b, hash.clone(), size))).await
            }).await?;
            return Ok(());
        }
        let r = self.simple(x, Method::POST, Some(key), &[("uploads", "")], &meta, vec![]).await?;
        let id = xml::first(&r.text().await?, "UploadId").ok_or_else(|| anyhow!("the service did not return an upload id"))?;
        let res = self.parts(src, key, &id, size, part, x).await;
        if res.is_err() { let _ = self.simple(&Xfer::new(), Method::DELETE, Some(key), &[("uploadId", id.as_str())], &[], vec![]).await; }
        res
    }

    async fn parts(&self, src: &Path, key: &str, id: &str, size: u64, part: u64, x: &Xfer) -> Result<()> {
        let mut etags: Vec<(u32, String)> = vec![];
        let (mut off, mut n) = (0u64, 1u32);
        while off < size {
            let len = part.min(size - off);
            let hash = Self::range_hash(src, off, len).await?;
            let base = x.done();
            let ns = n.to_string();
            let r = self.call(x, || async {
                x.set(base);
                let b = Self::file_body(src, off, len, x).await.map_err(|e| Fail { status: None, msg: format!("could not read the local archive: {e}"), retry: false })?;
                self.once(Method::PUT, Some(key), &[("partNumber", ns.as_str()), ("uploadId", id)], &[], Some((b, hash.clone(), len))).await
            }).await?;
            let etag = r.headers().get("etag").and_then(|v| v.to_str().ok()).ok_or_else(|| anyhow!("the service did not return an ETag for part {n}"))?.to_string();
            etags.push((n, etag));
            off += len; n += 1;
        }
        self.simple(x, Method::POST, Some(key), &[("uploadId", id)], &[("content-type", "application/xml".into())], xml::complete_body(&etags).into_bytes()).await?;
        Ok(())
    }

    pub async fn head(&self, key: &str) -> Result<Option<ObjInfo>> {
        match self.call(&Xfer::new(), || self.once(Method::HEAD, Some(key), &[], &[], None)).await {
            Ok(r) => {
                let hv = |n: &str| r.headers().get(n).and_then(|v| v.to_str().ok()).map(String::from);
                Ok(Some(ObjInfo { key: key.into(), size: hv("content-length").and_then(|v| v.parse().ok()).unwrap_or(0), etag: hv("etag").map(|e| e.trim_matches('"').to_string()), sha256: hv("x-amz-meta-sha256"),
                    kms: hv("x-amz-server-side-encryption").is_some_and(|v| v.starts_with("aws:kms")) }))
            }
            Err(e) if e.downcast_ref::<Fail>().is_some_and(|f| f.status == Some(StatusCode::NOT_FOUND)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Stream the object into `dest`; returns the SHA-256 and size received.
    pub async fn get(&self, key: &str, dest: &Path, x: &Xfer) -> Result<(String, u64)> {
        let base = x.done();
        let mut last = None;
        for i in 0..self.attempts {
            if i > 0 { tokio::time::sleep(self.backoff * 4u32.pow(i - 1)).await; }
            x.set(base);
            match self.get_once(key, dest, x).await { Ok(v) => return Ok(v), Err(e) if x.token.is_cancelled() || e.downcast_ref::<Fail>().is_some_and(|f| !f.retry) => return Err(e), Err(e) => last = Some(e) }
        }
        Err(last.unwrap_or_else(|| anyhow!("download failed")))
    }

    async fn get_once(&self, key: &str, dest: &Path, x: &Xfer) -> Result<(String, u64)> {
        let r = self.once(Method::GET, Some(key), &[], &[], None).await.map_err(anyhow::Error::new)?; // a Fail keeps its retry flag for `get`
        let want = r.content_length();
        let part: PathBuf = dest.with_file_name(format!("{}.dl", dest.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()));
        let res: Result<(String, u64)> = async {
            let mut f = tokio::fs::File::create(&part).await?;
            let (mut h, mut n) = (Sha256::new(), 0u64);
            let mut s = r.bytes_stream();
            while let Some(c) = s.next().await {
                if x.token.is_cancelled() { bail!("cancelled"); }
                let c = c.map_err(|e| anyhow!("download interrupted: {}", e.without_url()))?;
                f.write_all(&c).await?; h.update(&c); n += c.len() as u64; x.add(c.len() as u64);
            }
            f.sync_all().await?;
            if want.is_some_and(|w| w != n) { bail!("download was cut short ({n} of {} bytes)", want.unwrap_or(0)); }
            Ok((hex::encode(h.finalize()), n))
        }.await;
        match res { Ok(v) => { tokio::fs::rename(&part, dest).await?; Ok(v) } Err(e) => { let _ = tokio::fs::remove_file(&part).await; Err(e) } }
    }

    pub async fn delete(&self, key: &str) -> Result<()> {
        match self.call(&Xfer::new(), || self.once(Method::DELETE, Some(key), &[], &[], None)).await {
            Ok(_) => Ok(()),
            Err(e) if e.downcast_ref::<Fail>().is_some_and(|f| f.status == Some(StatusCode::NOT_FOUND)) => Ok(()),
            Err(e) => Err(e),
        }
    }

    pub async fn list(&self, prefix: &str, max: usize) -> Result<Vec<ObjInfo>> {
        let (mut out, mut token) = (vec![], None::<String>);
        let full = self.full_key(prefix);
        let strip = if self.cfg.prefix.is_empty() { 0 } else { self.cfg.prefix.len() + 1 };
        loop {
            let mut q = vec![("list-type", "2"), ("prefix", full.as_str())];
            let mk = max.min(1000).to_string();
            q.push(("max-keys", mk.as_str()));
            if let Some(t) = &token { q.push(("continuation-token", t.as_str())); }
            let body = self.call(&Xfer::new(), || self.once(Method::GET, None, &q, &[], None)).await?.text().await?;
            for c in xml::all(&body, "Contents") {
                let k = xml::first(c, "Key").unwrap_or_default();
                if k.len() >= strip { out.push(ObjInfo { key: k[strip..].to_string(), size: xml::first(c, "Size").and_then(|s| s.parse().ok()).unwrap_or(0), etag: xml::first(c, "ETag").map(|e| e.trim_matches('"').to_string()), sha256: None, kms: false }); }
            }
            token = xml::first(&body, "NextContinuationToken").filter(|_| xml::first(&body, "IsTruncated").as_deref() == Some("true"));
            if token.is_none() || out.len() >= max { break; }
        }
        out.truncate(max);
        Ok(out)
    }
}
