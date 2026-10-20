//! Authentication (Supabase JWT or dev mode) and RBAC: the `User` extractor, permission bundles, `authz` and `visible_servers`.
//! Global roles: owner/admin/user. Per-server grants map to permission bundles. Token checks are in `jwt`, account provisioning in `provision`.
use crate::{error::{ApiError, ApiResult}, state::St};
use axum::{extract::FromRequestParts, http::request::Parts};
use serde::Serialize;

mod jwt;
mod provision;

pub use provision::drop_cached;

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

impl FromRequestParts<St> for User {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, st: &St) -> Result<Self, ApiError> {
        if st.cfg.dev() {
            return Ok(User { id: "dev".into(), email: "dev@localhost".into(), role: "owner".into(), token: None });
        }
        let token = parts.headers.get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).ok_or_else(|| ApiError::unauthorized("missing bearer token"))?;
        let claims = jwt::verify(st, token).await?;
        let mut user = provision::provision(st, claims).await?;
        user.token = Some(token.to_string());
        Ok(user)
    }
}

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
