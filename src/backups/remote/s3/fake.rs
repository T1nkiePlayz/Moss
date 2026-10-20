//! An in-process S3 for tests: path-style, verifies every SigV4 signature and payload hash, supports single and
//! multipart upload, get, head, list (paginated), delete, and scripted failures.
use super::{sigv4, xml};
use crate::backups::remote::md5::md5;
use axum::{Router, body::Bytes, extract::State, http::{HeaderMap, Method, StatusCode, Uri}, response::{IntoResponse, Response}};
use std::{collections::{BTreeMap, VecDeque}, sync::{Arc, Mutex}};

pub const AK: &str = "AKIAFAKEACCESSKEY000";
pub const SK: &str = "fake/secret+key/for/tests/0123456789ab";
pub const BUCKET: &str = "bkt";

#[derive(Default)]
pub struct Obj { pub data: Vec<u8>, pub etag: String, pub sha: Option<String> }
#[derive(Default)]
pub struct Up { key: String, sha: Option<String>, parts: BTreeMap<u32, (Vec<u8>, String)> }
#[derive(Default)]
pub struct Fake {
    pub objects: BTreeMap<String, Obj>,
    pub uploads: BTreeMap<String, Up>,
    pub log: Vec<String>,
    /// (method, status, code): the next matching request fails once with this error
    pub inject: VecDeque<(String, u16, String)>,
    /// every request of this method fails until cleared
    pub always_fail: Option<(String, u16, String)>,
    pub corrupt_puts: bool,
    pub drop_meta: bool,
    pub sig_failures: u32,
}
pub type Shared = Arc<Mutex<Fake>>;

fn err(status: u16, code: &str, msg: &str) -> Response {
    (StatusCode::from_u16(status).unwrap(), format!("<?xml version=\"1.0\"?><Error><Code>{code}</Code><Message>{msg}</Message></Error>")).into_response()
}

fn pdecode(s: &str) -> String {
    let b = s.as_bytes();
    let (mut o, mut i) = (vec![], 0);
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() { if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) { o.push(v); i += 3; continue; } }
        o.push(b[i]); i += 1;
    }
    String::from_utf8_lossy(&o).to_string()
}

fn verify(m: &Method, uri: &Uri, h: &HeaderMap, body: &[u8]) -> Result<(), Response> {
    let auth = h.get("authorization").and_then(|v| v.to_str().ok()).ok_or_else(|| err(403, "AccessDenied", "no authorization"))?;
    let field = |n: &str| auth.split([',', ' ']).find_map(|p| p.strip_prefix(&format!("{n}="))).unwrap_or("").to_string();
    let cred = field("Credential");
    let (ak, scope) = cred.split_once('/').unwrap_or(("", ""));
    if ak != AK { return Err(err(403, "InvalidAccessKeyId", "unknown key")); }
    let region = scope.split('/').nth(1).unwrap_or("");
    let signed = field("SignedHeaders");
    let hv = |n: &str| h.get(n).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let headers: Vec<(String, String)> = signed.split(';').map(|n| (n.to_string(), hv(n))).collect();
    let payload = hv("x-amz-content-sha256");
    if payload != sigv4::sha256_hex(body) { return Err(err(400, "XAmzContentSHA256Mismatch", "payload hash mismatch")); }
    let query: Vec<(String, String)> = uri.query().unwrap_or("").split('&').filter(|s| !s.is_empty()).map(|p| { let (k, v) = p.split_once('=').unwrap_or((p, "")); (pdecode(k), pdecode(v)) }).collect();
    let s = sigv4::sign(m.as_str(), uri.path(), &query, &headers, &payload, AK, SK, region, &hv("x-amz-date"));
    if s.signature != field("Signature") { return Err(err(403, "SignatureDoesNotMatch", "The request signature we calculated does not match")); }
    Ok(())
}

async fn handle(State(f): State<Shared>, m: Method, uri: Uri, h: HeaderMap, body: Bytes) -> Response {
    let mut f = f.lock().unwrap();
    let q: BTreeMap<String, String> = uri.query().unwrap_or("").split('&').filter(|s| !s.is_empty()).map(|p| { let (k, v) = p.split_once('=').unwrap_or((p, "")); (pdecode(k), pdecode(v)) }).collect();
    f.log.push(format!("{m} {}{}", uri.path(), uri.query().map(|q| format!("?{q}")).unwrap_or_default()));
    if let Err(r) = verify(&m, &uri, &h, &body) { f.sig_failures += 1; return r; }
    if let Some((fm, st, code)) = &f.always_fail && *fm == m.as_str() && !(q.contains_key("uploadId") && m == Method::DELETE) { return err(*st, code, "scripted failure"); }
    if let Some(i) = f.inject.iter().position(|(fm, ..)| *fm == m.as_str()) { let (_, st, code) = f.inject.remove(i).unwrap(); return err(st, &code, "scripted failure"); }
    let path = pdecode(uri.path());
    let Some(rest) = path.strip_prefix(&format!("/{BUCKET}")) else { return err(404, "NoSuchBucket", "no such bucket") };
    let key = rest.trim_start_matches('/').to_string();
    let etag = |d: &[u8]| hex::encode(md5(d));
    match (m.as_str(), key.is_empty()) {
        ("GET", true) => {
            let prefix = q.get("prefix").cloned().unwrap_or_default();
            let max: usize = q.get("max-keys").and_then(|s| s.parse().ok()).unwrap_or(1000);
            let start: usize = q.get("continuation-token").and_then(|s| s.parse().ok()).unwrap_or(0);
            let all: Vec<_> = f.objects.iter().filter(|(k, _)| k.starts_with(&prefix)).collect();
            let page: Vec<_> = all.iter().skip(start).take(max).collect();
            let trunc = start + page.len() < all.len();
            let mut x = String::from("<ListBucketResult>");
            for (k, o) in &page { x.push_str(&format!("<Contents><Key>{}</Key><Size>{}</Size><ETag>&quot;{}&quot;</ETag></Contents>", xml::escape(k), o.data.len(), o.etag)); }
            x.push_str(&format!("<IsTruncated>{trunc}</IsTruncated>"));
            if trunc { x.push_str(&format!("<NextContinuationToken>{}</NextContinuationToken>", start + page.len())); }
            x.push_str("</ListBucketResult>");
            x.into_response()
        }
        ("POST", false) if q.contains_key("uploads") => {
            let id = format!("up-{}", f.uploads.len() + 1 + f.log.len());
            let sha = h.get("x-amz-meta-sha256").and_then(|v| v.to_str().ok()).map(String::from);
            f.uploads.insert(id.clone(), Up { key, sha, parts: BTreeMap::new() });
            format!("<InitiateMultipartUploadResult><UploadId>{id}</UploadId></InitiateMultipartUploadResult>").into_response()
        }
        ("PUT", false) if q.contains_key("partNumber") => {
            let Some(u) = f.uploads.get_mut(q.get("uploadId").map(String::as_str).unwrap_or("")) else { return err(404, "NoSuchUpload", "no such upload") };
            let n: u32 = q["partNumber"].parse().unwrap_or(0);
            let e = etag(&body);
            u.parts.insert(n, (body.to_vec(), e.clone()));
            ([("etag", format!("\"{e}\""))], "").into_response()
        }
        ("POST", false) if q.contains_key("uploadId") => {
            let Some(u) = f.uploads.remove(&q["uploadId"]) else { return err(404, "NoSuchUpload", "no such upload") };
            let body = String::from_utf8_lossy(&body).to_string();
            let (nums, tags) = (xml::all(&body, "PartNumber"), xml::all(&body, "ETag"));
            let (mut data, mut cat) = (vec![], vec![]);
            for (n, t) in nums.iter().zip(&tags) {
                let Some((d, e)) = u.parts.get(&n.parse().unwrap_or(0)) else { return err(400, "InvalidPart", "missing part") };
                if xml::unescape(t).trim_matches('"') != e { return err(400, "InvalidPart", "etag mismatch"); }
                data.extend_from_slice(d); cat.extend_from_slice(&md5(d));
            }
            let e = format!("{}-{}", etag(&cat), nums.len());
            f.objects.insert(u.key, Obj { data, etag: e.clone(), sha: u.sha });
            format!("<CompleteMultipartUploadResult><ETag>&quot;{e}&quot;</ETag></CompleteMultipartUploadResult>").into_response()
        }
        ("DELETE", false) if q.contains_key("uploadId") => { f.uploads.remove(&q["uploadId"]); StatusCode::NO_CONTENT.into_response() }
        ("PUT", false) => {
            let sha = h.get("x-amz-meta-sha256").and_then(|v| v.to_str().ok()).map(String::from).filter(|_| !f.drop_meta);
            let e = etag(&body);
            let mut data = body.to_vec();
            if f.corrupt_puts && !data.is_empty() { data[0] ^= 0xff; }
            f.objects.insert(key, Obj { data, etag: e.clone(), sha });
            ([("etag", format!("\"{e}\""))], "").into_response()
        }
        ("GET", false) => match f.objects.get(&key) { Some(o) => ([("etag", format!("\"{}\"", o.etag))], o.data.clone()).into_response(), None => err(404, "NoSuchKey", "no such key") },
        ("HEAD", false) => match f.objects.get(&key) {
            Some(o) => { let mut r = ([("etag", format!("\"{}\"", o.etag)), ("content-length", o.data.len().to_string())], "").into_response(); if let Some(s) = &o.sha { r.headers_mut().insert("x-amz-meta-sha256", s.parse().unwrap()); } r }
            None => StatusCode::NOT_FOUND.into_response(),
        },
        ("DELETE", false) => { f.objects.remove(&key); StatusCode::NO_CONTENT.into_response() }
        _ => err(400, "InvalidRequest", "unsupported"),
    }
}

/// Start the fake on a loopback port; returns its endpoint URL and shared state.
pub async fn start() -> (String, Shared) {
    let st: Shared = Arc::default();
    let app = Router::new().fallback(axum::routing::any(handle)).layer(axum::extract::DefaultBodyLimit::disable()).with_state(st.clone());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (url, st)
}
