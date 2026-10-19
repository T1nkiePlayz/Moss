use axum::{Json, http::StatusCode, response::{IntoResponse, Response}};
use serde_json::json;

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
    pub fields: Vec<(String, String)>,
}

pub type ApiResult<T> = Result<T, ApiError>;

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self { status, code, message: message.into(), fields: vec![] }
    }
    pub fn bad(code: &'static str, m: impl Into<String>) -> Self { Self::new(StatusCode::BAD_REQUEST, code, m) }
    pub fn not_found(what: &str) -> Self { Self::new(StatusCode::NOT_FOUND, "not_found", format!("{what} not found")) }
    pub fn forbidden(m: impl Into<String>) -> Self { Self::new(StatusCode::FORBIDDEN, "forbidden", m) }
    pub fn unauthorized(m: impl Into<String>) -> Self { Self::new(StatusCode::UNAUTHORIZED, "auth.unauthorized", m) }
    pub fn conflict(code: &'static str, m: impl Into<String>) -> Self { Self::new(StatusCode::CONFLICT, code, m) }
    pub fn field(mut self, path: &str, msg: &str) -> Self { self.fields.push((path.into(), msg.into())); self }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        tracing::error!("internal error: {e:#}");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", "internal error")
    }
}
impl From<std::io::Error> for ApiError {
    fn from(e: std::io::Error) -> Self {
        match e.kind() {
            std::io::ErrorKind::NotFound => Self::not_found("path"),
            std::io::ErrorKind::PermissionDenied => Self::forbidden("permission denied on host filesystem"),
            _ => anyhow::Error::from(e).into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let fields: Vec<_> = self.fields.iter().map(|(p, m)| json!({"path": p, "message": m})).collect();
        let body = json!({"error": {"code": self.code, "message": self.message, "request_id": uuid::Uuid::now_v7().to_string(), "fields": fields}});
        (self.status, Json(body)).into_response()
    }
}
