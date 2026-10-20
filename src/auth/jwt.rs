//! Supabase JWT verification: signing-key (JWKS) cache and token validation into `Claims`.
use crate::{error::{ApiError, ApiResult}, state::St};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use serde::Deserialize;
use serde_json::Value;
use std::time::{Duration, Instant};

#[derive(Deserialize)]
pub struct Claims {
    pub sub: String,
    pub email: Option<String>,
    #[serde(default)]
    pub user_metadata: Value,
    /// Authenticator assurance level: `aal1` (password/OAuth) or `aal2` (second factor verified).
    #[serde(default)]
    pub aal: Option<String>,
}

impl Claims {
    pub fn email_verified(&self) -> bool { self.user_metadata["email_verified"].as_bool().unwrap_or(false) }
}

const FRESH: Duration = Duration::from_secs(600);
/// A forced refresh (unknown key id) is rate-limited so garbage tokens cannot make us hammer Supabase.
const FORCE_EVERY: Duration = Duration::from_secs(30);
/// Keys older than this are not used even when Supabase is unreachable.
const STALE_MAX: Duration = Duration::from_secs(24 * 3600);

async fn jwks(st: &St, force: bool) -> anyhow::Result<Value> {
    let ttl = if force { FORCE_EVERY } else { FRESH };
    let cached = st.jwks.lock().unwrap_or_else(|p| p.into_inner()).clone();
    if let Some((t, v)) = &cached && t.elapsed() < ttl { return Ok(v.clone()); }
    let url = format!("{}/auth/v1/.well-known/jwks.json", st.cfg.supabase_url.trim_end_matches('/'));
    let fetched: anyhow::Result<Value> = async {
        Ok(st.http.get(url).header("apikey", &st.cfg.supabase_anon_key).timeout(Duration::from_secs(10)).send().await?.error_for_status()?.json().await?)
    }.await;
    match fetched {
        Ok(v) => { *st.jwks.lock().unwrap_or_else(|p| p.into_inner()) = Some((Instant::now(), v.clone())); Ok(v) }
        // keep serving with the last known keys through a short Supabase outage
        Err(e) => match cached { Some((t, v)) if t.elapsed() < STALE_MAX => { tracing::warn!("JWKS refresh failed, using cached keys: {e:#}"); Ok(v) } _ => Err(e) },
    }
}

/// Validation rules shared by all algorithms: audience and issuer pinned, `exp`/`sub` mandatory, small clock leeway.
pub fn validation(alg: Algorithm, supabase_url: &str) -> Validation {
    let mut v = Validation::new(alg);
    v.set_audience(&["authenticated"]);
    v.set_issuer(&[format!("{}/auth/v1", supabase_url.trim_end_matches('/'))]);
    v.set_required_spec_claims(&["exp", "sub", "aud", "iss"]);
    v.leeway = 30;
    v
}

pub async fn verify(st: &St, token: &str) -> ApiResult<Claims> {
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
    let v = validation(header.alg, &st.cfg.supabase_url);
    decode::<Claims>(token, &key, &v).map(|d| d.claims).map_err(|e| bad(&format!("invalid token: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{EncodingKey, Header, encode};
    use serde_json::json;

    fn mint(claims: Value, secret: &str) -> String { encode(&Header::new(Algorithm::HS256), &claims, &EncodingKey::from_secret(secret.as_bytes())).unwrap() }
    fn check(token: &str) -> Result<Claims, jsonwebtoken::errors::Error> {
        decode::<Claims>(token, &DecodingKey::from_secret(b"s3cret"), &validation(Algorithm::HS256, "https://x.supabase.co/")).map(|d| d.claims)
    }
    fn good() -> Value { json!({"sub": "u1", "aud": "authenticated", "iss": "https://x.supabase.co/auth/v1", "exp": chrono::Utc::now().timestamp() + 600, "aal": "aal2", "email": "a@b.c"}) }

    #[test]
    fn accepts_valid_and_parses_aal() {
        let c = check(&mint(good(), "s3cret")).unwrap();
        assert_eq!(c.aal.as_deref(), Some("aal2"));
        assert!(!c.email_verified());
    }
    #[test]
    fn missing_aal_is_none() {
        let mut g = good(); g.as_object_mut().unwrap().remove("aal");
        assert_eq!(check(&mint(g, "s3cret")).unwrap().aal, None);
    }
    #[test]
    fn rejects_wrong_audience_issuer_expiry_and_signature() {
        let mut g = good(); g["aud"] = json!("anon"); assert!(check(&mint(g, "s3cret")).is_err());
        let mut g = good(); g["iss"] = json!("https://evil.example/auth/v1"); assert!(check(&mint(g, "s3cret")).is_err());
        let mut g = good(); g["exp"] = json!(chrono::Utc::now().timestamp() - 3600); assert!(check(&mint(g, "s3cret")).is_err());
        let mut g = good(); g.as_object_mut().unwrap().remove("sub"); assert!(check(&mint(g, "s3cret")).is_err());
        assert!(check(&mint(good(), "other")).is_err());
    }
}
