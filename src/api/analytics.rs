//! Per-server player analytics (see `crate::analytics`).
use super::*;

pub(super) async fn server_analytics(State(st): State<St>, user: User, Path(id): Path<String>, Query(q): Q) -> ApiResult<Json<Value>> {
    let days = match q.get("days").map_or("7", String::as_str) {
        "7" => 7,
        "30" => 30,
        "90" => 90,
        _ => return Err(ApiError::bad("validation", "days must be 7, 30 or 90").field("days", "7 | 30 | 90")),
    };
    server_for(&st, &user, &id, "server.view").await?;
    let now = chrono::Utc::now().timestamp();
    Ok(Json(st.db.call(move |c| crate::analytics::report(c, &id, days, now)).await?))
}
