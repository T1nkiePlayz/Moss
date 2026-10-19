//! Authentication (Supabase JWT or dev mode) and RBAC.
//! Global roles: owner/admin/user. Per-server grants map to permission bundles.
use crate::{error::{ApiError, ApiResult}, state::St};
use axum::{extract::FromRequestParts, http::request::Parts};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Serialize)]
pub struct User {
    pub id: String,
    pub email: String,
    pub role: String,
    /// The caller's Supabase access token, kept only to forward to the CurseForge edge function.
    #[serde(skip)]
    pub token: Option<String>,
}

pub const ALL_PERMS: &[&str] = &[
    "server.view", "server.power", "server.console.read", "server.console.write", "files.read", "files.write",
    "content.manage", "backup.manage", "config.write", "server.delete", "admin.users", "admin.settings", "server.create",
];

pub fn bundle_perms(bundle: &str) -> &'static [&'static str] {
    match bundle {
        "manager" => &["server.view", "server.power", "server.console.read", "server.console.write", "files.read", "files.write", "content.manage", "backup.manage", "config.write"],
        "operator" => &["server.view", "server.power", "server.console.read", "server.console.write"],
        "viewer" => &["server.view", "server.console.read"],
        _ => &[],
    }
}

impl User {
    pub fn is_admin(&self) -> bool { self.role == "owner" || self.role == "admin" }
    pub fn require_admin(&self) -> ApiResult<()> {
        if self.is_admin() { Ok(()) } else { Err(ApiError::forbidden("administrator access required")) }
    }
}

/// Check a permission on one server. Unknown/unreadable servers return 404 rather than 403 to avoid ID probing.
pub async fn authz(st: &St, user: &User, server_id: &str, perm: &str) -> ApiResult<()> {
    if user.is_admin() { return Ok(()); }
    let (sid, uid) = (server_id.to_string(), user.id.clone());
    let bundle: Option<String> = st.db.call(move |c| {
        c.query_row("SELECT bundle FROM grants WHERE server_id=? AND user_id=?", [sid, uid], |r| r.get(0))
            .map(Some).or_else(|e| if matches!(e, rusqlite::Error::QueryReturnedNoRows) { Ok(None) } else { Err(e) })
    }).await?;
    match bundle {
        None => Err(ApiError::not_found("server")),
        Some(b) if bundle_perms(&b).contains(&perm) => Ok(()),
        Some(_) => Err(ApiError::forbidden(format!("missing permission {perm}"))),
    }
}

/// Servers visible to a user.
pub async fn visible_servers(st: &St, user: &User) -> ApiResult<Option<Vec<String>>> {
    if user.is_admin() { return Ok(None); }
    let uid = user.id.clone();
    Ok(Some(st.db.call(move |c| {
        let mut q = c.prepare("SELECT server_id FROM grants WHERE user_id=?")?;
        q.query_map([uid], |r| r.get(0))?.collect()
    }).await?))
}

#[derive(Deserialize)]
struct Claims {
    sub: String,
    email: Option<String>,
    #[serde(default)]
    user_metadata: Value,
}

async fn jwks(st: &St, force: bool) -> anyhow::Result<Value> {
    // A forced refresh (unknown key id) is rate-limited so garbage tokens cannot make us hammer Supabase.
    let ttl = if force { Duration::from_secs(30) } else { Duration::from_secs(600) };
    if let Some((t, v)) = st.jwks.lock().unwrap().as_ref()
        && t.elapsed() < ttl { return Ok(v.clone()); }
    let url = format!("{}/auth/v1/.well-known/jwks.json", st.cfg.supabase_url.trim_end_matches('/'));
    let v: Value = st.http.get(url).header("apikey", &st.cfg.supabase_anon_key).send().await?.error_for_status()?.json().await?;
    *st.jwks.lock().unwrap() = Some((Instant::now(), v.clone()));
    Ok(v)
}

async fn verify(st: &St, token: &str) -> ApiResult<Claims> {
    let bad = |m: &str| ApiError::unauthorized(m.to_string());
    let header = decode_header(token).map_err(|_| bad("malformed token"))?;
    let key = match header.alg {
        Algorithm::HS256 => {
            if st.cfg.supabase_jwt_secret.is_empty() { return Err(bad("HS256 token but SUPABASE_JWT_SECRET is not configured")); }
            DecodingKey::from_secret(st.cfg.supabase_jwt_secret.as_bytes())
        }
        Algorithm::ES256 | Algorithm::RS256 | Algorithm::EdDSA => {
            let kid = header.kid.clone().ok_or_else(|| bad("token has no key id"))?;
            let find = |set: &Value| -> Option<Value> { set["keys"].as_array()?.iter().find(|k| k["kid"] == kid.as_str()).cloned() };
            let mut jwk = find(&jwks(st, false).await?);
            if jwk.is_none() { jwk = find(&jwks(st, true).await.map_err(ApiError::from)?); }
            let jwk: jsonwebtoken::jwk::Jwk = serde_json::from_value(jwk.ok_or_else(|| bad("unknown signing key"))?).map_err(|_| bad("bad signing key"))?;
            DecodingKey::from_jwk(&jwk).map_err(|_| bad("unsupported signing key"))?
        }
        _ => return Err(bad("unsupported token algorithm")),
    };
    let mut v = Validation::new(header.alg);
    v.set_audience(&["authenticated"]);
    v.set_issuer(&[format!("{}/auth/v1", st.cfg.supabase_url.trim_end_matches('/'))]);
    v.leeway = 30;
    decode::<Claims>(token, &key, &v).map(|d| d.claims).map_err(|e| bad(&format!("invalid token: {e}")))
}

async fn provision(st: &St, c: Claims) -> ApiResult<User> {
    if let Some((t, u)) = st.users.lock().unwrap().get(&c.sub)
        && t.elapsed() < Duration::from_secs(30) { return Ok(u.clone()); }
    let email = c.email.clone().unwrap_or_default().to_lowercase();
    let verified = c.user_metadata["email_verified"].as_bool().unwrap_or(false);
    let owners = st.cfg.owner_email_list();
    let require_verified = st.cfg.require_verified_email;
    let sub = c.sub.clone();
    let user = st.db.call(move |db| {
        let by_id = db.query_row("SELECT id,email,role FROM users WHERE id=?", [&sub], |r| Ok(User { id: r.get(0)?, email: r.get(1)?, role: r.get(2)?, token: None }));
        if let Ok(u) = by_id { return Ok(Some(u)); }
        if email.is_empty() || (require_verified && !verified) { return Ok(None); }
        // invited (pre-created, unlinked) account?
        if let Ok((old_id, role)) = db.query_row("SELECT id,role FROM users WHERE email=? AND linked=0", [&email], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))) {
            // grants cascade on user delete, so free the unique email, add the real row, move grants, then drop the placeholder
            db.execute("UPDATE users SET email=email||'#invite' WHERE id=?", [&old_id])?;
            db.execute("INSERT INTO users(id,email,role,linked,created_at) VALUES (?,?,?,1,?)", rusqlite::params![sub, email, role, crate::db::now()])?;
            db.execute("UPDATE grants SET user_id=? WHERE user_id=?", [&sub, &old_id])?;
            db.execute("DELETE FROM users WHERE id=?", [&old_id])?;
            return Ok(Some(User { id: sub, email, role, token: None }));
        }
        let owner_count: i64 = db.query_row("SELECT COUNT(*) FROM users WHERE role='owner'", [], |r| r.get(0))?;
        let role = (owners.contains(&email) || (owners.is_empty() && owner_count == 0)).then_some("owner");
        match role {
            Some(role) => {
                db.execute("INSERT INTO users(id,email,role,linked,created_at) VALUES (?,?,?,1,?)", rusqlite::params![sub, email, role, crate::db::now()])?;
                Ok(Some(User { id: sub, email, role: role.into(), token: None }))
            }
            None => Ok(None),
        }
    }).await?;
    match user {
        Some(u) => {
            st.users.lock().unwrap().insert(c.sub, (Instant::now(), u.clone()));
            Ok(u)
        }
        None => Err(ApiError::new(axum::http::StatusCode::FORBIDDEN, "auth.not_provisioned", "This account has not been invited to this Moss instance (or its email is not verified yet).")),
    }
}

impl FromRequestParts<St> for User {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, st: &St) -> Result<Self, ApiError> {
        if st.cfg.dev() {
            return Ok(User { id: "dev".into(), email: "dev@localhost".into(), role: "owner".into(), token: None });
        }
        let token = parts.headers.get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).ok_or_else(|| ApiError::unauthorized("missing bearer token"))?;
        let claims = verify(st, token).await?;
        let mut user = provision(st, claims).await?;
        user.token = Some(token.to_string());
        Ok(user)
    }
}

pub fn drop_cached(st: &St, id: &str) { st.users.lock().unwrap().remove(id); }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bundles_are_least_privilege() {
        assert!(!bundle_perms("viewer").contains(&"server.power"));
        assert!(bundle_perms("operator").contains(&"server.power") && !bundle_perms("operator").contains(&"files.write"));
        assert!(!bundle_perms("manager").contains(&"server.delete") && !bundle_perms("manager").contains(&"admin.users"));
        assert!(bundle_perms("nonsense").is_empty());
    }
}
