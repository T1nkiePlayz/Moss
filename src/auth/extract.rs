//! Request extractors: `User` (full policy), `MfaExempt` (only `GET /me`), `Claimant` (`POST /setup/claim`).
//! Credentials are a Supabase JWT or a `moss_pat_` API token. Failed credentials count towards the per-address throttle.
use super::{User, error::AuthError, jwt, mfa, provision, throttle, tokens};
use crate::{error::ApiError, state::St};
use axum::{extract::{ConnectInfo, FromRequestParts, OriginalUri}, http::{StatusCode, request::Parts}};
use std::{net::SocketAddr, time::Instant};

fn bearer(parts: &Parts) -> Option<&str> {
    let v = parts.headers.get("authorization")?.to_str().ok()?;
    let (scheme, rest) = v.split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then_some(rest.trim())
}

fn client(parts: &Parts, st: &St) -> String {
    throttle::client_addr(&parts.headers, parts.extensions.get::<ConnectInfo<SocketAddr>>(), st.cfg.trust_proxy)
}

fn fail(st: &St, addr: &str) { st.guard.auth_fail.lock().unwrap_or_else(|p| p.into_inner()).record(addr, Instant::now()); }

fn dev_user() -> User {
    User { id: "dev".into(), email: "dev@localhost".into(), role: "owner".into(), token: None, via_token: None, aal: Some("aal2".into()) }
}

/// Verify the bearer JWT, counting failures against `addr`. Internal errors (JWKS fetch) are not credential failures.
async fn verify_jwt(st: &St, addr: &str, token: &str) -> Result<jwt::Claims, AuthError> {
    match jwt::verify(st, token).await {
        Ok(c) => Ok(c),
        Err(e) => { if e.status == StatusCode::UNAUTHORIZED { fail(st, addr); } Err(e.into()) }
    }
}

fn check_mfa(st: &St, role: &str, aal: Option<&str>) -> Result<(), AuthError> {
    if mfa::required(st.cfg.mfa_mode(), role) && !mfa::aal2(aal) { return Err(AuthError::mfa_required(aal)); }
    Ok(())
}

async fn resolve(parts: &Parts, st: &St, enforce_mfa: bool) -> Result<User, AuthError> {
    if st.cfg.dev() { return Ok(dev_user()); }
    let presented = bearer(parts).ok_or_else(|| ApiError::unauthorized("missing bearer token"))?;
    let addr = client(parts, st);
    if let Some(retry) = st.guard.auth_fail.lock().unwrap_or_else(|p| p.into_inner()).blocked(&addr, Instant::now()) {
        return Err(AuthError::throttled("auth.throttled", retry));
    }
    if tokens::looks_like_token(presented) {
        let Some((user, scope)) = tokens::authenticate(st, presented).await? else {
            fail(st, &addr);
            return Err(ApiError::unauthorized("invalid or expired API token").into());
        };
        let (method, path) = (&parts.method, parts.extensions.get::<OriginalUri>().map(|u| u.0.path()).unwrap_or_else(|| parts.uri.path()));
        if tokens::forbidden_for_tokens(method, path) {
            return Err(ApiError::new(StatusCode::FORBIDDEN, "auth.token_forbidden", "API tokens cannot manage tokens or users").into());
        }
        if !tokens::scope_allows(&scope, method) {
            return Err(ApiError::new(StatusCode::FORBIDDEN, "auth.token_scope", "this API token is read-only").into());
        }
        return Ok(user);
    }
    let claims = verify_jwt(st, &addr, presented).await?;
    let mut user = provision::provision(st, &claims, presented).await?;
    user.aal = claims.aal;
    user.token = Some(presented.to_string());
    if enforce_mfa { check_mfa(st, &user.role, user.aal.as_deref())?; }
    Ok(user)
}

impl FromRequestParts<St> for User {
    type Rejection = AuthError;
    async fn from_request_parts(parts: &mut Parts, st: &St) -> Result<Self, AuthError> { resolve(parts, st, true).await }
}

/// Authenticated user without the MFA gate, so a signed-in user can still see *why* they are blocked (`GET /me`).
pub struct MfaExempt(pub User);
impl FromRequestParts<St> for MfaExempt {
    type Rejection = AuthError;
    async fn from_request_parts(parts: &mut Parts, st: &St) -> Result<Self, AuthError> { resolve(parts, st, false).await.map(MfaExempt) }
}

/// A verified identity that may not have a Moss account yet (it may be about to claim ownership). JWT only.
pub struct Claimant { pub id: String, pub email: String }
impl FromRequestParts<St> for Claimant {
    type Rejection = AuthError;
    async fn from_request_parts(parts: &mut Parts, st: &St) -> Result<Self, AuthError> {
        let presented = bearer(parts).ok_or_else(|| ApiError::unauthorized("missing bearer token"))?;
        if tokens::looks_like_token(presented) { return Err(ApiError::new(StatusCode::FORBIDDEN, "auth.token_forbidden", "API tokens cannot claim ownership").into()); }
        let addr = client(parts, st);
        if let Some(retry) = st.guard.auth_fail.lock().unwrap_or_else(|p| p.into_inner()).blocked(&addr, Instant::now()) {
            return Err(AuthError::throttled("auth.throttled", retry));
        }
        let claims = verify_jwt(st, &addr, presented).await?;
        // the claimant becomes an owner, so the owner-level MFA policy applies
        check_mfa(st, "owner", claims.aal.as_deref())?;
        let email = claims.email.clone().unwrap_or_default().to_lowercase();
        if email.is_empty() || (st.cfg.require_verified_email && !super::jwt::email_confirmed(st, presented, &email).await) {
            return Err(ApiError::new(StatusCode::FORBIDDEN, "auth.not_provisioned", "A verified email address is required.").into());
        }
        match provision::provision(st, &claims, presented).await {
            Ok(u) => Ok(Claimant { id: u.id, email }),
            Err(e) if e.code == "auth.setup_required" => Ok(Claimant { id: claims.sub, email }),
            Err(e) => Err(e.into()),
        }
    }
}
