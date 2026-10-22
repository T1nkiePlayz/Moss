//! Notification endpoints: list, mark read, webhook test. Non-admins only see notifications of servers they can
//! see, and never the host-level kinds.
use super::*;
use crate::notify::{self, rules};

/// Newest `limit` rows the caller may see. Rows are read in batches and filtered as they come, so hidden newer rows
/// (other servers, host-level kinds) cannot push visible older ones out of the window.
async fn visible_rows(st: &St, user: &User, unread_only: bool, limit: i64) -> ApiResult<Vec<Value>> {
    const BATCH: i64 = 500;
    const SCAN_CAP: i64 = 50_000;
    let vis = auth::visible_servers(st, user).await?;
    let admin = user.is_admin();
    let sql = format!("{} {} ORDER BY time DESC, id DESC LIMIT ? OFFSET ?", notify::SEL, if unread_only { "WHERE read=0" } else { "" });
    let (mut out, mut offset) = (Vec::<Value>::new(), 0i64);
    while (out.len() as i64) < limit && offset < SCAN_CAP {
        let sql = sql.clone();
        let rows: Vec<Value> = st.db.call(move |c| { let mut q = c.prepare(&sql)?; q.query_map([BATCH, offset], notify::row_json)?.collect() }).await?;
        let n = rows.len() as i64;
        out.extend(rows.into_iter().filter(|n| rules::visible_to(n["kind"].as_str().unwrap_or(""), n["server_id"].as_str(), admin, vis.as_deref())));
        if n < BATCH { break; }
        offset += BATCH;
    }
    out.truncate(limit as usize);
    Ok(out)
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
    // One commit (one fsync) for the whole batch rather than one per row under the shared DB lock.
    st.db.call(move |c| { let tx = c.unchecked_transaction()?; { let mut q = tx.prepare("UPDATE notifications SET read=1 WHERE id=?")?; for id in &ids { q.execute([id])?; } } tx.commit() }).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn notifications_test(State(st): State<St>, user: User) -> ApiResult<Json<Value>> {
    user.require_admin()?;
    let (delivered, detail) = notify::send_test(&st).await;
    st.audit(&user.actor(), "notifications.test", "webhook", if delivered { "delivered" } else { "failed" }, None).await;
    Ok(Json(json!({"delivered": delivered, "detail": detail})))
}
