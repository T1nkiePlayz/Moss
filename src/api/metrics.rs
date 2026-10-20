//! Long-term metrics endpoints (see `crate::history`).
use super::*;
use crate::history::{self, Range};

fn range_of(q: &HashMap<String, String>) -> ApiResult<(String, Range)> {
    let s = q.get("range").map_or("1h", String::as_str);
    Range::parse(s).map(|r| (s.to_string(), r)).ok_or_else(|| ApiError::bad("validation", "range must be 1h, 24h, 7d or 30d").field("range", "1h | 24h | 7d | 30d"))
}

pub(super) async fn server_metrics(State(st): State<St>, user: User, Path(id): Path<String>, Query(q): Q) -> ApiResult<Json<Value>> {
    authz(&st, &user, &id, "server.view").await?;
    let (name, r) = range_of(&q)?;
    Ok(Json(history::server_series(&st, &id, &name, r).await?))
}

pub(super) async fn host_metrics(State(st): State<St>, user: User, Query(q): Q) -> ApiResult<Json<Value>> {
    user.require_admin()?;
    let (name, r) = range_of(&q)?;
    Ok(Json(history::host_series(&st, &name, r).await?))
}
