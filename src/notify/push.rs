//! Web Push: VAPID (RFC 8292) authentication, message encryption (RFC 8291, `aes128gcm` content coding of RFC 8188,
//! one record) and delivery. The VAPID key is generated once and kept in a 0600 file in the data directory. Subscription
//! endpoints are https URLs that must not point at private addresses (same guard as webhooks, checked again when
//! connecting). Delivery is spawned off the caller's path with a timeout; 404/410 removes the subscription.
use super::rules;
use crate::{error::ApiError, state::St};
use aes_gcm::{Aes128Gcm, KeyInit, aead::Aead};
use anyhow::{Result, anyhow, bail};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use hkdf::Hkdf;
use p256::{PublicKey, SecretKey, pkcs8::EncodePrivateKey};
use rand::{TryRng, rngs::SysRng};
use serde_json::json;
use sha2::Sha256;
use std::{net::IpAddr, path::Path, sync::Mutex, time::Duration};

const KEY_FILE: &str = "push-vapid.key";
const RECORD_SIZE: u32 = 4096;
/// Largest plaintext that fits one record (record size minus the 16-byte tag and the 1-byte delimiter) and the 86-byte header.
const MAX_PLAINTEXT: usize = RECORD_SIZE as usize - 16 - 1 - 86;
const TIMEOUT: Duration = Duration::from_secs(10);
/// VAPID tokens may live at most 24 hours; stay well below.
const JWT_TTL_SECS: u64 = 12 * 3600;
const DEFAULT_SUBJECT: &str = "https://github.com/T1nkiePlayz/Moss";

// ---- VAPID key ----

static KEY_LOCK: Mutex<()> = Mutex::new(());

fn random_key() -> Result<SecretKey> {
    loop {
        let mut b = [0u8; 32];
        SysRng.try_fill_bytes(&mut b).map_err(|e| anyhow!("random number generator failed: {e}"))?;
        if let Ok(k) = SecretKey::from_slice(&b) { return Ok(k); }
    }
}

/// The instance's VAPID key; created on first use.
pub fn load_or_create_key(data_dir: &Path) -> Result<SecretKey> {
    let _g = KEY_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let path = data_dir.join(KEY_FILE);
    if let Ok(s) = std::fs::read_to_string(&path) {
        let bytes = hex::decode(s.trim()).map_err(|_| anyhow!("{} is not valid", path.display()))?;
        return SecretKey::from_slice(&bytes).map_err(|_| anyhow!("{} does not hold a P-256 key", path.display()));
    }
    let key = random_key()?;
    std::fs::create_dir_all(data_dir)?;
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)] std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    std::io::Write::write_all(&mut o.open(&path)?, hex::encode(key.to_bytes()).as_bytes())?;
    Ok(key)
}

/// The uncompressed public point, base64url: the `applicationServerKey` browsers subscribe with.
pub fn public_key_b64(k: &SecretKey) -> String { B64.encode(k.public_key().to_sec1_bytes()) }

/// The VAPID JWT (ES256) for one push service; `aud` is the endpoint's origin.
pub fn vapid_jwt(k: &SecretKey, endpoint: &str, subject: &str, now: u64) -> Result<String> {
    let u = reqwest::Url::parse(endpoint)?;
    let aud = u.origin().ascii_serialization();
    let der = k.to_pkcs8_der().map_err(|e| anyhow!("could not encode the VAPID key: {e}"))?;
    let key = jsonwebtoken::EncodingKey::from_ec_der(der.as_bytes());
    let claims = json!({"aud": aud, "exp": now + JWT_TTL_SECS, "sub": subject});
    Ok(jsonwebtoken::encode(&jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256), &claims, &key)?)
}

// ---- RFC 8291 encryption ----

fn hkdf_expand(h: &Hkdf<Sha256>, info: &[u8], out: &mut [u8]) { h.expand(info, out).expect("HKDF output length is valid"); }

/// Encrypt `plaintext` for the subscription keys `ua_public` (65 bytes) and `auth_secret` (16 bytes). The result is the
/// complete request body: the RFC 8188 header (salt, record size, sender key) followed by the single record.
pub fn encrypt(plaintext: &[u8], ua_public: &[u8], auth_secret: &[u8], as_secret: &SecretKey, salt: &[u8; 16]) -> Result<Vec<u8>> {
    if plaintext.len() > MAX_PLAINTEXT { bail!("the push payload is too large"); }
    let ua = PublicKey::from_sec1_bytes(ua_public).map_err(|_| anyhow!("the subscription key is not a valid P-256 point"))?;
    let as_public = as_secret.public_key().to_sec1_bytes();
    let shared = p256::ecdh::diffie_hellman(as_secret.to_nonzero_scalar(), ua.as_affine());
    let prk_key = Hkdf::<Sha256>::new(Some(auth_secret), shared.raw_secret_bytes());
    let mut info = b"WebPush: info\0".to_vec();
    info.extend_from_slice(ua_public);
    info.extend_from_slice(&as_public);
    let mut ikm = [0u8; 32];
    hkdf_expand(&prk_key, &info, &mut ikm);
    let prk = Hkdf::<Sha256>::new(Some(salt), &ikm);
    let (mut cek, mut nonce) = ([0u8; 16], [0u8; 12]);
    hkdf_expand(&prk, b"Content-Encoding: aes128gcm\0", &mut cek);
    hkdf_expand(&prk, b"Content-Encoding: nonce\0", &mut nonce);
    let mut record = plaintext.to_vec();
    record.push(0x02); // last-record delimiter, no padding
    let ct = Aes128Gcm::new_from_slice(&cek).expect("16-byte key").encrypt((&nonce).into(), record.as_slice()).map_err(|_| anyhow!("encryption failed"))?;
    let mut out = Vec::with_capacity(86 + ct.len());
    out.extend_from_slice(salt);
    out.extend_from_slice(&RECORD_SIZE.to_be_bytes());
    out.push(as_public.len() as u8);
    out.extend_from_slice(&as_public);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// `encrypt` with a fresh sender key and salt.
pub fn encrypt_random(plaintext: &[u8], ua_public: &[u8], auth_secret: &[u8]) -> Result<Vec<u8>> {
    let mut salt = [0u8; 16];
    SysRng.try_fill_bytes(&mut salt).map_err(|e| anyhow!("random number generator failed: {e}"))?;
    encrypt(plaintext, ua_public, auth_secret, &random_key()?, &salt)
}

// ---- subscriptions ----

pub const MAX_ENDPOINT: usize = 1000;
pub const MAX_PER_USER: i64 = 20;

/// Check a subscription as sent by the browser. Returns the decoded `(p256dh, auth)` keys.
pub fn validate_subscription(endpoint: &str, p256dh: &str, auth: &str, allow_private: bool) -> Result<(Vec<u8>, Vec<u8>), ApiError> {
    let bad = |f: &str, m: &str| ApiError::bad("validation", m.to_string()).field(f, m);
    if endpoint.len() > MAX_ENDPOINT { return Err(bad("endpoint", "at most 1000 characters")); }
    let u = reqwest::Url::parse(endpoint).map_err(|_| bad("endpoint", "not a valid URL"))?;
    if u.scheme() != "https" { return Err(bad("endpoint", "must be an https:// URL")); }
    if !u.username().is_empty() || u.password().is_some() { return Err(bad("endpoint", "must not contain credentials")); }
    let host = u.host_str().filter(|h| !h.is_empty()).ok_or_else(|| bad("endpoint", "missing host"))?;
    let private = match host.trim_matches(['[', ']']).parse::<IpAddr>() {
        Ok(ip) => !super::webhook::is_public_ip(ip),
        Err(_) => { let d = host.trim_end_matches('.').to_ascii_lowercase(); d == "localhost" || d.ends_with(".localhost") || d.ends_with(".local") || d.ends_with(".internal") || !d.contains('.') }
    };
    if private && !allow_private { return Err(bad("endpoint", "must be a public host")); }
    let key = B64.decode(p256dh.trim_end_matches('=')).ok().filter(|k| k.len() == 65 && PublicKey::from_sec1_bytes(k).is_ok()).ok_or_else(|| bad("keys.p256dh", "not a valid P-256 public key"))?;
    let secret = B64.decode(auth.trim_end_matches('=')).ok().filter(|a| a.len() == 16).ok_or_else(|| bad("keys.auth", "must be 16 bytes, base64url"))?;
    Ok((key, secret))
}

#[derive(Clone, Debug)]
pub struct Sub { pub id: String, pub endpoint: String, pub p256dh: String, pub auth: String, pub is_admin: bool, pub visible: Option<Vec<String>> }

/// Subscribers who should receive this notification: they can see it, and the selected events include its kind.
pub fn recipients<'a>(subs: &'a [Sub], enabled: &[String], kind: &str, server_id: Option<&str>) -> Vec<&'a Sub> {
    if !enabled.iter().any(|k| k == kind) { return vec![]; }
    subs.iter().filter(|s| rules::visible_to(kind, server_id, s.is_admin, s.visible.as_deref())).collect()
}

fn load_subs(c: &rusqlite::Connection) -> rusqlite::Result<Vec<Sub>> {
    let mut q = c.prepare("SELECT s.id, s.user_id, s.endpoint, s.p256dh, s.auth, u.role FROM push_subscriptions s JOIN users u ON u.id=s.user_id")?;
    let rows: Vec<(String, String, String, String, String, String)> = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?.collect::<Result<_, _>>()?;
    let mut g = c.prepare("SELECT server_id FROM grants WHERE user_id=?")?;
    rows.into_iter().map(|(id, user_id, endpoint, p256dh, auth, role)| {
        let is_admin = role == "owner" || role == "admin";
        let visible = if is_admin { None } else { Some(g.query_map([&user_id], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?) };
        Ok(Sub { id, endpoint, p256dh, auth, is_admin, visible })
    }).collect()
}

/// The JSON the service worker receives.
pub fn payload(kind: &str, severity: &str, title: &str, body: &str, server_id: Option<&str>) -> Vec<u8> {
    let cut = |s: &str, n: usize| s.chars().take(n).collect::<String>();
    json!({"kind": kind, "severity": severity, "title": cut(title, 120), "body": cut(body, 300), "server_id": server_id}).to_string().into_bytes()
}

/// Fan a stored notification out to push subscribers without blocking the caller.
pub fn dispatch(st: &St, enabled: Vec<String>, kind: &str, severity: &str, title: &str, body: &str, server_id: Option<&str>) {
    if !enabled.iter().any(|k| k == kind) { return; }
    let (st, kind, severity, msg, sid) = (st.clone(), kind.to_string(), severity.to_string(), payload(kind, severity, title, body, server_id), server_id.map(String::from));
    tokio::spawn(async move {
        let Ok(subs) = st.db.call(load_subs).await else { return };
        let list: Vec<Sub> = recipients(&subs, &enabled, &kind, sid.as_deref()).into_iter().cloned().collect();
        if list.is_empty() { return; }
        let key = match load_or_create_key(&st.cfg.data_dir) { Ok(k) => k, Err(e) => { tracing::warn!("web push unavailable: {e:#}"); return; } };
        let subject = st.setting("push_subject", DEFAULT_SUBJECT).await;
        let urgency = if severity == "critical" || kind == "crash" || kind == "crash_loop" { "high" } else { "normal" };
        for s in list {
            let (st, key, subject, msg) = (st.clone(), key.clone(), subject.clone(), msg.clone());
            tokio::spawn(async move {
                match send_one(&st, &key, &subject, &s, &msg, urgency).await {
                    Ok(Outcome::Sent) => {}
                    Ok(Outcome::Gone) => { let id = s.id.clone(); let _ = st.db.call(move |c| c.execute("DELETE FROM push_subscriptions WHERE id=?", [id])).await; }
                    Err(e) => tracing::debug!("web push delivery failed: {e:#}"),
                }
            });
        }
    });
}

enum Outcome { Sent, Gone }

async fn send_one(st: &St, key: &SecretKey, subject: &str, s: &Sub, msg: &[u8], urgency: &str) -> Result<Outcome> {
    let u = reqwest::Url::parse(&s.endpoint)?;
    if u.scheme() != "https" { bail!("push endpoints must use https"); }
    let host = u.host_str().ok_or_else(|| anyhow!("push endpoint has no host"))?.to_string();
    let addr = super::webhook::resolve_pinned(&host, u.port_or_known_default().unwrap_or(443), st.cfg.allow_private_webhooks, "MOSS_ALLOW_PRIVATE_WEBHOOKS").await?;
    let ua = B64.decode(s.p256dh.trim_end_matches('='))?;
    let auth = B64.decode(s.auth.trim_end_matches('='))?;
    let body = encrypt_random(msg, &ua, &auth)?;
    let jwt = vapid_jwt(key, &s.endpoint, subject, chrono::Utc::now().timestamp() as u64)?;
    let client = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).timeout(TIMEOUT).connect_timeout(TIMEOUT)
        .resolve(&host, addr).user_agent(concat!("Moss/", env!("CARGO_PKG_VERSION"))).build()?;
    let r = client.post(u).header("Content-Encoding", "aes128gcm").header("Content-Type", "application/octet-stream")
        .header("TTL", "86400").header("Urgency", urgency)
        .header("Authorization", format!("vapid t={jwt}, k={}", public_key_b64(key))).body(body).send().await
        .map_err(|e| anyhow!("request to {host} failed: {}", e.without_url()))?;
    match r.status().as_u16() {
        404 | 410 => Ok(Outcome::Gone),
        c if (200..300).contains(&c) => Ok(Outcome::Sent),
        c => bail!("{host} answered HTTP {c}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Vec<u8> { B64.decode(s).unwrap() }

    /// RFC 8291 Appendix A, byte for byte.
    #[test]
    fn rfc8291_appendix_a_vector() {
        let as_secret = SecretKey::from_slice(&d("yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw")).unwrap();
        assert_eq!(public_key_b64(&as_secret), "BP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8");
        let ua_public = d("BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4");
        let auth = d("BTBZMqHH6r4Tts7J_aSIgg");
        let salt: [u8; 16] = d("DGv6ra1nlYgDCS1FRnbzlw").try_into().unwrap();
        let out = encrypt(b"When I grow up, I want to be a watermelon", &ua_public, &auth, &as_secret, &salt).unwrap();
        assert_eq!(B64.encode(out), "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN");
    }

    #[test]
    fn oversized_payload_is_refused() {
        let k = SecretKey::from_slice(&d("q1dXpw3UpT5VOmu_cf_v6ih07Aems3njxI-JWgLcM94")).unwrap();
        let ua = k.public_key().to_sec1_bytes();
        let ok = encrypt_random(&vec![b'x'; MAX_PLAINTEXT], &ua, &[0; 16]).unwrap();
        assert!(ok.len() <= 4096);
        assert!(encrypt_random(&vec![b'x'; MAX_PLAINTEXT + 1], &ua, &[0; 16]).is_err());
    }

    #[test]
    fn vapid_jwt_is_es256_with_origin_audience_and_short_expiry() {
        let k = SecretKey::from_slice(&d("yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw")).unwrap();
        let now = chrono::Utc::now().timestamp() as u64;
        let jwt = vapid_jwt(&k, "https://fcm.googleapis.com/fcm/send/abc?x=1", "mailto:a@example.com", now).unwrap();
        let dk = jsonwebtoken::DecodingKey::from_ec_der(&k.public_key().to_sec1_bytes());
        let mut v = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::ES256);
        v.set_audience(&["https://fcm.googleapis.com"]);
        v.set_required_spec_claims(&["exp", "aud"]);
        let t = jsonwebtoken::decode::<serde_json::Value>(&jwt, &dk, &v).unwrap();
        let exp = t.claims["exp"].as_u64().unwrap();
        assert!(exp > now && exp <= now + 24 * 3600, "exp {exp}");
        assert_eq!(t.claims["aud"], "https://fcm.googleapis.com");
        assert_eq!(t.claims["sub"], "mailto:a@example.com");
        // a different audience is rejected
        v.set_audience(&["https://example.com"]);
        assert!(jsonwebtoken::decode::<serde_json::Value>(&jwt, &dk, &v).is_err());
    }

    #[test]
    fn vapid_key_is_created_once_with_private_permissions() {
        let dir = std::env::temp_dir().join(format!("moss-vapid-{}", uuid::Uuid::new_v4()));
        let a = load_or_create_key(&dir).unwrap();
        let b = load_or_create_key(&dir).unwrap();
        assert_eq!(public_key_b64(&a), public_key_b64(&b));
        #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; assert_eq!(std::fs::metadata(dir.join(KEY_FILE)).unwrap().permissions().mode() & 0o777, 0o600); }
        let _ = std::fs::remove_dir_all(dir);
    }

    fn keys() -> (String, String) {
        let k = SecretKey::from_slice(&d("q1dXpw3UpT5VOmu_cf_v6ih07Aems3njxI-JWgLcM94")).unwrap();
        (B64.encode(k.public_key().to_sec1_bytes()), "BTBZMqHH6r4Tts7J_aSIgg".to_string())
    }

    #[test]
    fn subscription_validation() {
        let (p, a) = keys();
        assert!(validate_subscription("https://fcm.googleapis.com/fcm/send/abc", &p, &a, false).is_ok());
        assert!(validate_subscription("https://updates.push.services.mozilla.com/wpush/v2/xyz", &p, &a, false).is_ok());
        for bad in ["http://fcm.googleapis.com/x", "ftp://x.y/z", "not a url", "https://user:pw@fcm.googleapis.com/x", "https://localhost/x", "https://127.0.0.1/x", "https://10.0.0.5/x",
            "https://192.168.1.1/x", "https://169.254.169.254/latest", "https://[::1]/x", "https://intranet/x", "https://printer.local/x", &format!("https://example.com/{}", "a".repeat(1000))] {
            assert!(validate_subscription(bad, &p, &a, false).is_err(), "{bad}");
        }
        assert!(validate_subscription("https://127.0.0.1:8443/x", &p, &a, true).is_ok());
        assert!(validate_subscription("https://example.com/x", "AAAA", &a, false).is_err());
        assert!(validate_subscription("https://example.com/x", &B64.encode([4u8; 65]), &a, false).is_err()); // not on the curve
        assert!(validate_subscription("https://example.com/x", &p, "AAAA", false).is_err());
    }

    fn sub(id: &str, admin: bool, visible: Option<&[&str]>) -> Sub {
        Sub { id: id.into(), endpoint: "https://e".into(), p256dh: String::new(), auth: String::new(), is_admin: admin, visible: visible.map(|v| v.iter().map(|s| s.to_string()).collect()) }
    }

    #[test]
    fn fan_out_respects_events_and_visibility() {
        let subs = [sub("admin", true, None), sub("a", false, Some(&["s1"])), sub("b", false, Some(&["s2"])), sub("none", false, Some(&[]))];
        let ids = |kind: &str, sid: Option<&str>, ev: &[&str]| -> Vec<String> {
            let ev: Vec<String> = ev.iter().map(|s| s.to_string()).collect();
            recipients(&subs, &ev, kind, sid).into_iter().map(|s| s.id.clone()).collect()
        };
        assert_eq!(ids("crash", Some("s1"), &["crash"]), ["admin", "a"]);
        assert_eq!(ids("crash", Some("s2"), &["crash"]), ["admin", "b"]);
        assert!(ids("crash", Some("s1"), &["backup_failed"]).is_empty()); // kind not selected
        assert_eq!(ids("disk_low", None, &["disk_low"]), ["admin"]); // host-level: admins only
        assert_eq!(ids("player_join", Some("s1"), &["player_join"]), ["admin", "a"]);
        assert!(ids("player_join", Some("s1"), &["crash", "crash_loop"]).is_empty()); // opt-in
    }

    #[test]
    fn payload_is_trimmed_json() {
        let p: serde_json::Value = serde_json::from_slice(&payload("crash", "warn", &"t".repeat(500), &"b".repeat(900), Some("s1"))).unwrap();
        assert_eq!(p["title"].as_str().unwrap().len(), 120);
        assert_eq!(p["body"].as_str().unwrap().len(), 300);
        assert_eq!(p["server_id"], "s1");
    }

    #[tokio::test]
    async fn subscriptions_load_with_role_and_grants() {
        let p = std::env::temp_dir().join(format!("moss-push-{}.sqlite", uuid::Uuid::new_v4()));
        let db = crate::db::Db::open(&p).unwrap();
        let subs = db.call(|c| {
            c.execute_batch("INSERT INTO users(id,email,role,linked,created_at) VALUES('u1','a@x.io','owner',1,'t'),('u2','b@x.io','viewer',1,'t');
                INSERT INTO servers(id,name,runtime,game_version,port,memory_mb,created_at) VALUES('s1','S','paper','1',1,1,'t');
                INSERT INTO grants(server_id,user_id,bundle) VALUES('s1','u2','operator');
                INSERT INTO push_subscriptions(id,user_id,endpoint,p256dh,auth,created_at) VALUES('p1','u1','https://a','k','a','t'),('p2','u2','https://b','k','a','t');")?;
            load_subs(c)
        }).await.unwrap();
        let by = |id: &str| subs.iter().find(|s| s.id == id).unwrap().clone();
        assert!(by("p1").is_admin && by("p1").visible.is_none());
        assert_eq!(by("p2").visible, Some(vec!["s1".to_string()]));
        let _ = std::fs::remove_file(p);
    }
}
