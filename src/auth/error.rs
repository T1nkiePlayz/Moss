//! The rejection type of the auth extractors: an `ApiError` that can also carry `Retry-After` and extra body fields.
use crate::error::ApiError;
use axum::{Json, http::{HeaderValue, StatusCode, header}, response::{IntoResponse, Response}};
use serde_json::{Value, json};

#[derive(Debug)]
pub struct AuthError { pub err: ApiError, pub retry_after: Option<u64>, pub extra: Vec<(&'static str, Value)> }

impl From<ApiError> for AuthError {
    fn from(err: ApiError) -> Self { Self { err, retry_after: None, extra: vec![] } }
}
impl From<anyhow::Error> for AuthError {
    fn from(e: anyhow::Error) -> Self { ApiError::from(e).into() }
}

impl AuthError {
    pub fn throttled(code: &'static str, retry_after: u64) -> Self {
        Self { err: ApiError::new(StatusCode::TOO_MANY_REQUESTS, code, "too many attempts, try again later"), retry_after: Some(retry_after), extra: vec![] }
    }
    pub fn mfa_required(aal: Option<&str>) -> Self {
        let err = ApiError::new(StatusCode::FORBIDDEN, "auth.mfa_required", "multi-factor authentication is required for this account");
        Self { err, retry_after: None, extra: vec![("aal", json!(aal.unwrap_or("aal1"))), ("required", json!("aal2"))] }
    }
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let mut e = json!({"code": self.err.code, "message": self.err.message, "request_id": uuid::Uuid::now_v7().to_string(),
            "fields": self.err.fields.iter().map(|(p, m)| json!({"path": p, "message": m})).collect::<Vec<_>>()});
        for (k, v) in self.extra { e[k] = v; }
        let mut r = (self.err.status, Json(json!({"error": e}))).into_response();
        if let Some(s) = self.retry_after && let Ok(v) = HeaderValue::from_str(&s.to_string()) { r.headers_mut().insert(header::RETRY_AFTER, v); }
        r
    }
}
