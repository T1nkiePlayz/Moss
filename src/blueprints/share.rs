//! Blueprint share codes.
//!
//! A share code is `moss1_` + base64url (no padding) of: one version byte, four checksum bytes (a SHA-256 prefix of the
//! packed payload) and the deflated, cleaned blueprint JSON. Decoding is strict: the checksum must match, the unpacked
//! size is capped (no decompression bombs) and the document must pass `sanitize` plus `strict_check`.

use super::*;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use std::io::{Read, Write};

const CODE_PREFIX: &str = "moss1_";
const CODE_VERSION: u8 = 1;
/// Longest accepted share code, in characters.
pub const MAX_CODE: usize = 48 * 1024;

fn checksum(payload: &[u8]) -> [u8; 4] {
    use sha2::{Digest, Sha256};
    let d = Sha256::digest(payload);
    [d[0], d[1], d[2], d[3]]
}

pub fn encode(bp: &Value) -> ApiResult<String> {
    let fail = |e: std::io::Error| ApiError::bad("blueprint.invalid", e.to_string());
    let bp = sanitize(bp)?;
    let json = serde_json::to_vec(&bp).map_err(|e| ApiError::bad("blueprint.invalid", e.to_string()))?;
    let mut z = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
    z.write_all(&json).map_err(fail)?;
    let packed = z.finish().map_err(fail)?;
    let mut raw = vec![CODE_VERSION];
    raw.extend_from_slice(&checksum(&packed));
    raw.extend_from_slice(&packed);
    let code = format!("{CODE_PREFIX}{}", URL_SAFE_NO_PAD.encode(raw));
    if code.len() > MAX_CODE {
        return Err(ApiError::bad("blueprint.too_large", "This blueprint is too large to share as a code. Export it as a file instead."));
    }
    Ok(code)
}

fn unsafe_text(s: &str) -> Option<&'static str> {
    let t = s.trim();
    let b = t.as_bytes();
    let drive = b.len() > 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && matches!(b[2], b'\\' | b'/');
    if t.starts_with(['/', '\\', '~']) || drive || t.contains("../") || t.contains("..\\") {
        return Some("absolute or relative file paths are not allowed");
    }
    let l = t.to_ascii_lowercase();
    if l.contains("<script") || l.starts_with("#!") || l.contains("javascript:") {
        return Some("scripts are not allowed");
    }
    None
}

fn walk(v: &Value) -> Option<&'static str> {
    match v {
        Value::String(s) => unsafe_text(s),
        Value::Array(a) => a.iter().find_map(walk),
        Value::Object(o) => o.values().find_map(walk),
        _ => None,
    }
}

/// Extra rules for codes that arrive from other people: content must be real project ids (Modrinth ids are 8 letters
/// and digits, CurseForge ids are numbers) and no text may look like a path or a script.
fn strict_check(bp: &Value) -> ApiResult<()> {
    let bad = |m: &str| ApiError::bad("blueprint.invalid", format!("This share code is not safe to import: {m}"));
    for it in bp["content"].as_array().map(|a| a.as_slice()).unwrap_or_default() {
        let (prov, id) = (it["provider"].as_str().unwrap_or_default(), it["project_id"].as_str().unwrap_or_default());
        let ok = match prov {
            "modrinth" => id.len() == 8 && id.chars().all(|c| c.is_ascii_alphanumeric()),
            "curseforge" => (1..=10).contains(&id.len()) && id.chars().all(|c| c.is_ascii_digit()),
            _ => false,
        };
        if !ok { return Err(bad("content items must be Modrinth or CurseForge project ids")); }
    }
    match walk(bp) { Some(m) => Err(bad(m)), None => Ok(()) }
}

pub fn decode(code: &str) -> ApiResult<Value> {
    let bad = |m: &str| ApiError::bad("blueprint.invalid", format!("This share code is not valid: {m}"));
    let code: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    if code.len() > MAX_CODE { return Err(bad("it is too long")); }
    let body = code.strip_prefix(CODE_PREFIX).ok_or_else(|| bad("it should start with moss1_"))?;
    let raw = URL_SAFE_NO_PAD.decode(body).map_err(|_| bad("it contains characters that are not allowed, so it was probably cut off or changed"))?;
    if raw.len() < 6 { return Err(bad("it is too short")); }
    if raw[0] != CODE_VERSION { return Err(bad("it was made by a newer version of Moss")); }
    let (sum, packed) = (&raw[1..5], &raw[5..]);
    if sum != checksum(packed) { return Err(bad("the checksum does not match, so it was probably cut off or changed")); }
    let mut json = Vec::new();
    flate2::read::DeflateDecoder::new(packed).take(MAX_JSON as u64 + 1).read_to_end(&mut json).map_err(|_| bad("it could not be unpacked"))?;
    if json.len() > MAX_JSON { return Err(bad("it unpacks to something too large")); }
    let v: Value = serde_json::from_slice(&json).map_err(|_| bad("it does not contain a blueprint"))?;
    let bp = sanitize(&v)?;
    strict_check(&bp)?;
    Ok(bp)
}

pub async fn share(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    user.require_admin()?;
    let (bp, ..) = load(&st, &id).await?;
    let code = encode(&bp)?;
    Ok(Json(json!({"code": code, "length": code.len()})))
}

#[derive(Deserialize)]
pub struct CodeReq { code: String }

pub async fn import_code(State(st): State<St>, user: User, Json(r): Json<CodeReq>) -> ApiResult<(StatusCode, Json<Value>)> {
    user.require_admin()?;
    let bp = decode(&r.code)?;
    let out = store(&st, &user, &bp).await?;
    st.audit(&user.actor(), "blueprint.import", bp["name"].as_str().unwrap_or(""), "imported from a share code", None).await;
    Ok((StatusCode::CREATED, Json(out)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc() -> Value {
        json!({"format": FORMAT, "version": 1, "name": "Friends SMP", "runtime": "paper", "game_version": "1.21.4", "memory_mb": 4096,
            "properties": {"motd": "hi", "view-distance": 8, "pvp": false},
            "content": [{"provider": "modrinth", "project_id": "AABBCCDD", "title": "Chunky"}], "schedules": [{"kind": "backup", "daily_at": "04:00", "tz": "UTC", "command": "say hi"}]})
    }
    fn mk(ver: u8, payload: &[u8]) -> String {
        let mut z = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
        z.write_all(payload).unwrap();
        let p = z.finish().unwrap();
        let mut raw = vec![ver];
        raw.extend_from_slice(&checksum(&p));
        raw.extend_from_slice(&p);
        format!("moss1_{}", URL_SAFE_NO_PAD.encode(raw))
    }

    #[test]
    fn round_trips_and_ignores_pasted_whitespace() {
        let code = encode(&doc()).unwrap();
        assert!(code.starts_with("moss1_") && code.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-')));
        let back = decode(&code).unwrap();
        assert_eq!(back, sanitize(&doc()).unwrap());
        assert_eq!(decode(&format!("  {}\n", code)).unwrap(), back);
    }

    #[test]
    fn detects_damage() {
        let code = encode(&doc()).unwrap();
        assert!(decode(&code[..code.len() - 5]).is_err(), "truncated");
        let mut flipped = code.clone().into_bytes();
        let i = flipped.len() / 2;
        flipped[i] = if flipped[i] == b'A' { b'B' } else { b'A' };
        assert!(decode(&String::from_utf8(flipped).unwrap()).is_err(), "bit flip");
        assert!(decode("hello").is_err());
        assert!(decode("moss1_!!!").is_err());
        assert!(decode(&"moss1_".repeat(MAX_CODE)).is_err(), "too long");
    }

    #[test]
    fn rejects_bombs_and_future_versions() {
        let big = vec![b' '; MAX_JSON * 8];
        assert!(decode(&mk(1, &big)).unwrap_err().message.contains("too large"), "inflate bomb");
        assert!(decode(&mk(2, b"{}")).is_err(), "future version");
        assert!(decode(&mk(1, &serde_json::to_vec(&doc()).unwrap())).is_ok());
    }

    #[test]
    fn strict_rules_reject_unsafe_content() {
        let cases: [(&str, Value); 8] = [
            ("/content/0/project_id", json!("../etc/x")),
            ("/content/0/project_id", json!("short")),
            ("/content/0/provider", json!("evil")),
            ("/properties/motd", json!("/etc/passwd")),
            ("/properties/motd", json!("C:\\Windows")),
            ("/properties/motd", json!("<script>x</script>")),
            ("/schedules/0/command", json!("~/bin/x")),
            ("/properties/motd", json!("#!/bin/sh")),
        ];
        for (ptr, v) in cases {
            let mut d = doc();
            *d.pointer_mut(ptr).unwrap() = v;
            assert!(decode(&mk(1, &serde_json::to_vec(&d).unwrap())).is_err(), "{ptr}");
        }
        let mut d = doc();
        d["content"][0] = json!({"provider": "curseforge", "project_id": "238222"});
        assert!(decode(&mk(1, &serde_json::to_vec(&d).unwrap())).is_ok());
    }
}
