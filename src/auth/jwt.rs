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
    let mut v = Validation::new(header.alg);
    v.set_audience(&["authenticated"]);
    v.set_issuer(&[format!("{}/auth/v1", st.cfg.supabase_url.trim_end_matches('/'))]);
    v.leeway = 30;
    decode::<Claims>(token, &key, &v).map(|d| d.claims).map_err(|e| bad(&format!("invalid token: {e}")))
}
