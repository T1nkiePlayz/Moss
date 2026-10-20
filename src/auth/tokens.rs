//! Personal API tokens (`moss_pat_…`): generation, hashing, expiry and scope rules, plus the database layer.
//! Only the SHA-256 digest and an 8 character display prefix are stored; the token itself is shown once.
use super::{User, secret};
use crate::state::St;
use axum::http::Method;
use rusqlite::{Connection, OptionalExtension, params};

pub const PREFIX: &str = "moss_pat_";
pub const MAX_PER_USER: i64 = 25;

pub struct NewToken { pub token: String, pub prefix: String, pub hash: String }

pub fn generate() -> NewToken {
    let token = format!("{PREFIX}{}", secret::random_string(40));
    NewToken { prefix: token.chars().take(8).collect(), hash: secret::sha256_hex(&token), token }
}

pub fn looks_like_token(s: &str) -> bool { s.starts_with(PREFIX) }

/// Is `expires_at` (RFC 3339) in the past at `now`? Unparseable values count as expired (fail closed).
pub fn expired(expires_at: Option<&str>, now: chrono::DateTime<chrono::Utc>) -> bool {
    match expires_at {
        None => false,
        Some(s) => chrono::DateTime::parse_from_rfc3339(s).map(|t| t <= now).unwrap_or(true),
    }
}

/// `read` tokens may only use safe methods.
pub fn scope_allows(scope: &str, method: &Method) -> bool {
    scope == "full" || (scope == "read" && (method == Method::GET || method == Method::HEAD))
}

/// Endpoints a token may never call: token management (except listing) and user/access management.
pub fn forbidden_for_tokens(method: &Method, path: &str) -> bool {
    let seg: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let seg = if seg.starts_with(&["api", "v1"]) { &seg[2..] } else { &seg[..] };
    let safe = method == Method::GET || method == Method::HEAD;
    match seg {
        ["users", ..] | ["setup", ..] => true,
        ["tokens", ..] => !safe,
        ["servers", _, "access"] => !safe,
        _ => false,
    }
}

/// Resolve a presented token to its owner with their CURRENT role. Unknown, expired or orphaned tokens return `None`.
/// `last_used` is written at most once a minute.
pub async fn authenticate(st: &St, presented: &str) -> anyhow::Result<Option<(User, String)>> {
    let hash = secret::sha256_hex(presented);
    let now = chrono::Utc::now();
    let h2 = hash.clone();
    let row = st.db.call(move |c| {
        c.query_row(
            "SELECT t.id,t.name,t.scope,t.expires_at,t.hash,u.id,u.email,u.role FROM api_tokens t JOIN users u ON u.id=t.user_id WHERE t.hash=?",
            [h2], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, Option<String>>(3)?, r.get::<_, String>(4)?,
                User { id: r.get(5)?, email: r.get(6)?, role: r.get(7)?, token: None, via_token: None, aal: None })),
        ).optional()
    }).await?;
    let Some((id, name, scope, expires, stored, mut user)) = row else { return Ok(None) };
    if !secret::ct_eq(stored.as_bytes(), hash.as_bytes()) || expired(expires.as_deref(), now) { return Ok(None); }
    let (cutoff, stamp) = ((now - chrono::Duration::seconds(60)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true), crate::db::now());
    let _ = st.db.call(move |c| c.execute("UPDATE api_tokens SET last_used=? WHERE id=? AND (last_used IS NULL OR last_used<?)", params![stamp, id, cutoff])).await;
    user.via_token = Some(name);
    Ok(Some((user, scope)))
}

pub enum Created { Ok { id: String, expires_at: Option<String> }, TooMany }

/// Insert a token unless the owner already has [`MAX_PER_USER`] live ones (expired tokens do not count).
pub fn insert(c: &Connection, user_id: &str, name: &str, scope: &str, tok: &NewToken, expires_in_days: Option<i64>) -> rusqlite::Result<Created> {
    let tx = c.unchecked_transaction()?;
    let now = chrono::Utc::now();
    let stamp = now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let live: i64 = tx.query_row("SELECT COUNT(*) FROM api_tokens WHERE user_id=? AND (expires_at IS NULL OR expires_at>?)", params![user_id, stamp], |r| r.get(0))?;
    if live >= MAX_PER_USER { return Ok(Created::TooMany); }
    let id = uuid::Uuid::now_v7().to_string();
    let expires_at = expires_in_days.map(|d| (now + chrono::Duration::days(d)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
    tx.execute("INSERT INTO api_tokens(id,user_id,name,scope,hash,prefix,created_at,expires_at) VALUES (?,?,?,?,?,?,?,?)",
        params![id, user_id, name, scope, tok.hash, tok.prefix, stamp, expires_at])?;
    tx.commit()?;
    Ok(Created::Ok { id, expires_at })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};

    fn db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        crate::db::migrate_for_tests(&c);
        c.execute("INSERT INTO users(id,email,role,linked,created_at) VALUES ('u1','a@b.c','user',1,'x')", []).unwrap();
        c
    }

    #[test]
    fn token_format_and_hash() {
        let t = generate();
        assert!(t.token.starts_with("moss_pat_") && t.token.len() == 9 + 40);
        assert_eq!(t.prefix, "moss_pat");
        assert_eq!(t.hash, secret::sha256_hex(&t.token));
        assert_eq!(t.hash.len(), 64);
        assert_ne!(generate().token, t.token);
        assert!(looks_like_token(&t.token) && !looks_like_token("eyJhbGciOi"));
    }
    #[test]
    fn expiry() {
        let now = Utc::now();
        assert!(!expired(None, now));
        assert!(!expired(Some(&(now + Duration::days(1)).to_rfc3339()), now));
        assert!(expired(Some(&(now - Duration::seconds(1)).to_rfc3339()), now));
        assert!(expired(Some("garbage"), now));
    }
    #[test]
    fn scope_matrix() {
        for (scope, m, ok) in [("read", Method::GET, true), ("read", Method::HEAD, true), ("read", Method::POST, false), ("read", Method::DELETE, false),
            ("full", Method::POST, true), ("full", Method::GET, true), ("weird", Method::GET, false)] {
            assert_eq!(scope_allows(scope, &m), ok, "{scope} {m}");
        }
    }
    #[test]
    fn forbidden_endpoints() {
        assert!(forbidden_for_tokens(&Method::GET, "/api/v1/users"));
        assert!(forbidden_for_tokens(&Method::PATCH, "/api/v1/users/x"));
        assert!(forbidden_for_tokens(&Method::POST, "/api/v1/tokens"));
        assert!(forbidden_for_tokens(&Method::DELETE, "/api/v1/tokens/abc"));
        assert!(forbidden_for_tokens(&Method::PUT, "/api/v1/servers/s1/access"));
        assert!(forbidden_for_tokens(&Method::POST, "//api/v1//tokens/"));
        assert!(!forbidden_for_tokens(&Method::GET, "/api/v1/tokens"));
        assert!(!forbidden_for_tokens(&Method::GET, "/api/v1/servers/s1/access"));
        assert!(!forbidden_for_tokens(&Method::POST, "/api/v1/servers/s1/actions/start"));
    }
    #[test]
    fn cap_counts_only_live_tokens() {
        let c = db();
        for i in 0..MAX_PER_USER { assert!(matches!(insert(&c, "u1", &format!("t{i}"), "full", &generate(), None).unwrap(), Created::Ok { .. })); }
        assert!(matches!(insert(&c, "u1", "extra", "full", &generate(), None).unwrap(), Created::TooMany));
        c.execute("UPDATE api_tokens SET expires_at='2000-01-01T00:00:00Z' WHERE name='t0'", []).unwrap();
        assert!(matches!(insert(&c, "u1", "extra", "read", &generate(), Some(7)).unwrap(), Created::Ok { expires_at: Some(_), .. }));
    }
    #[test]
    fn revocation_and_user_delete_cascade() {
        let c = db();
        let t = generate();
        let Created::Ok { id, .. } = insert(&c, "u1", "ci", "read", &t, None).unwrap() else { panic!() };
        let found = |c: &Connection| c.query_row("SELECT COUNT(*) FROM api_tokens WHERE hash=?", [&t.hash], |r| r.get::<_, i64>(0)).unwrap();
        assert_eq!(found(&c), 1);
        c.execute("DELETE FROM api_tokens WHERE id=?", [&id]).unwrap();
        assert_eq!(found(&c), 0);
        insert(&c, "u1", "ci", "read", &t, None).unwrap();
        c.execute("DELETE FROM users WHERE id='u1'", []).unwrap();
        assert_eq!(found(&c), 0);
    }
}
