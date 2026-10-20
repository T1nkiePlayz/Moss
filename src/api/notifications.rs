//! Notification endpoints: list, mark read, webhook test. Non-admins only see notifications of servers they can
//! see, and never the host-level kinds.
use super::*;
use crate::notify::{self, rules};

async fn visible_rows(st: &St, user: &User, unread_only: bool, limit: i64) -> ApiResult<Vec<Value>> {
    let vis = auth::visible_servers(st, user).await?;
    let sql = format!("{} {} ORDER BY time DESC LIMIT ?", notify::SEL, if unread_only { "WHERE read=0" } else { "" });
    let rows: Vec<Value> = st.db.call(move |c| { let mut q = c.prepare(&sql)?; q.query_map([limit], notify::row_json)?.collect() }).await?;
    let admin = user.is_admin();
    Ok(rows.into_iter().filter(|n| rules::visible_to(n["kind"].as_str().unwrap_or(""), n["server_id"].as_str(), admin, vis.as_deref())).collect())
}

pub(super) async fn notifications_list(State(st): State<St>, user: User, Query(q): Q) -> ApiResult<Json<Value>> {
    let limit = q.get("limit").and_then(|v| v.parse::<usize>().ok()).unwrap_or(50).clamp(1, 200);
    let unread_only = matches!(q.get("unread").map(String::as_str), Some("1" | "true"));
    let items: Vec<Value> = visible_rows(&st, &user, unread_only, 1000).await?;
    let unread = if unread_only { items.len() } else { items.iter().filter(|n| n["read"] == false).count() };
    Ok(Json(json!({"items": items.into_iter().take(limit).collect::<Vec<_>>(), "unread": unread})))
}

#[derive(Deserialize)]
pub(super) struct ReadReq { ids: Option<Vec<String>>, all: Option<bool> }

pub(super) async fn notifications_read(State(st): State<St>, user: User, Json(r): Json<ReadReq>) -> ApiResult<StatusCode> {
    let all = r.all.unwrap_or(false);
    if !all && r.ids.is_none() { return Err(ApiError::bad("validation", "pass ids or all:true")); }
    let wanted = r.ids.unwrap_or_default();
    let ids: Vec<String> = visible_rows(&st, &user, true, 1000).await?.into_iter().filter_map(|n| n["id"].as_str().map(String::from)).filter(|id| all || wanted.contains(id)).collect();
    st.db.call(move |c| { let mut q = c.prepare("UPDATE notifications SET read=1 WHERE id=?")?; for id in &ids { q.execute([id])?; } Ok(()) }).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn notifications_test(State(st): State<St>, user: User) -> ApiResult<Json<Value>> {
    user.require_admin()?;
    let (delivered, detail) = notify::send_test(&st).await;
    st.audit(&user.email, "notifications.test", "webhook", if delivered { "delivered" } else { "failed" }, None).await;
    Ok(Json(json!({"delivered": delivered, "detail": detail})))
}
