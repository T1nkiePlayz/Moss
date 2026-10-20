//! Authentication (Supabase JWT or dev mode) and RBAC: the `User` extractor, permission bundles, `authz` and `visible_servers`.
//! Global roles: owner/admin/user. Per-server grants map to permission bundles. Token checks are in `jwt`, account provisioning in `provision`.
use crate::{error::{ApiError, ApiResult}, state::St};
use serde::Serialize;

mod error;
mod extract;
mod jwt;
mod mfa;
mod provision;
pub mod secret;
pub mod setup;
mod throttle;
pub mod tokens;

pub use error::AuthError;
pub use mfa::Mode as MfaMode;
pub use extract::{Claimant, MfaExempt};
pub use provision::{drop_all, drop_cached};
pub use throttle::Guard;

#[derive(Clone, Debug, Serialize)]
pub struct User {
    pub id: String,
    pub email: String,
    pub role: String,
    /// The caller's Supabase access token, kept only to forward to the CurseForge edge function.
    #[serde(skip)]
    pub token: Option<String>,
    /// Name of the API token that authenticated this request (`None` for interactive sessions).
    #[serde(skip)]
    pub via_token: Option<String>,
    /// The `aal` claim of the session token (`None` for API tokens and absent claims).
    #[serde(skip)]
    pub aal: Option<String>,
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
    /// Name recorded in the activity log: the e-mail, plus the token name for API-token requests.
    pub fn actor(&self) -> String {
        match &self.via_token { Some(t) => format!("{} (token {t})", self.email), None => self.email.clone() }
    }
    /// Is this caller subject to the MFA policy, and have they met it? API tokens are non-interactive and count as satisfied.
    pub fn mfa_required(&self, mode: mfa::Mode) -> bool { mfa::required(mode, &self.role) }
    pub fn mfa_satisfied(&self) -> bool { self.via_token.is_some() || mfa::aal2(self.aal.as_deref()) }
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
    #[test]
    fn actor_names_the_token() {
        let mut u = User { id: "1".into(), email: "a@b.c".into(), role: "user".into(), token: None, via_token: None, aal: None };
        assert_eq!(u.actor(), "a@b.c");
        u.via_token = Some("ci".into());
        assert_eq!(u.actor(), "a@b.c (token ci)");
        assert!(u.mfa_satisfied());
        u.via_token = None;
        assert!(!u.mfa_satisfied());
        u.aal = Some("aal2".into());
        assert!(u.mfa_satisfied());
    }
}
