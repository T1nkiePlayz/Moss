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
    let sql = format!("{} {} ORDER BY n.time DESC, n.id DESC LIMIT ?2 OFFSET ?3", notify::SEL, if unread_only { notify::UNREAD } else { "" });
    let (mut out, mut offset) = (Vec::<Value>::new(), 0i64);
    while (out.len() as i64) < limit && offset < SCAN_CAP {
        let (sql, uid) = (sql.clone(), user.id.clone());
        let rows: Vec<Value> = st.db.call(move |c| { let mut q = c.prepare(&sql)?; q.query_map(rusqlite::params![uid, BATCH, offset], notify::row_json)?.collect() }).await?;
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
    // Read state is the caller's own. One commit (one fsync) for the whole batch rather than one per row.
    let uid = user.id.clone();
    st.db.call(move |c| { let tx = c.unchecked_transaction()?; { let mut q = tx.prepare("INSERT OR IGNORE INTO notification_reads(user_id,notification_id) VALUES (?,?)")?; for id in &ids { q.execute([&uid, id])?; } } tx.commit() }).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn notifications_test(State(st): State<St>, user: User) -> ApiResult<Json<Value>> {
    user.require_admin()?;
    let (delivered, detail) = notify::send_test(&st).await;
    st.audit(&user.actor(), "notifications.test", "webhook", if delivered { "delivered" } else { "failed" }, None).await;
    Ok(Json(json!({"delivered": delivered, "detail": detail})))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(id: &str) -> User { User { id: id.into(), email: format!("{id}@x.y"), role: "admin".into(), token: None, via_token: None, aal: None } }

    async fn unread(st: &St, u: &str) -> Vec<String> {
        visible_rows(st, &user(u), true, 100).await.unwrap().into_iter().map(|n| n["id"].as_str().unwrap().to_string()).collect()
    }

    #[tokio::test]
    async fn read_state_is_per_user() {
        let st = crate::testutil::state(|_| {}).await;
        st.db.call(|c| {
            for (i, u) in ["a", "b"].iter().enumerate() { c.execute("INSERT INTO users(id,email,role,created_at) VALUES (?1,?2,'admin','x')", rusqlite::params![u, format!("{i}@x.y")])?; }
            for n in ["n1", "n2"] { c.execute("INSERT INTO notifications(id,time,kind,severity,title) VALUES (?1,'2026-01-01T00:00:00Z','crash','warn','Crashed')", [n])?; }
            Ok(())
        }).await.unwrap();
        notifications_read(State(st.clone()), user("a"), Json(ReadReq { ids: Some(vec!["n1".into()]), all: None })).await.unwrap();
        assert_eq!(unread(&st, "a").await, ["n2"]);
        assert_eq!(unread(&st, "b").await.len(), 2, "another admin's notifications stay unread");
        notifications_read(State(st.clone()), user("a"), Json(ReadReq { ids: None, all: Some(true) })).await.unwrap();
        assert!(unread(&st, "a").await.is_empty());
        assert_eq!(unread(&st, "b").await.len(), 2);
        let all = visible_rows(&st, &user("a"), false, 100).await.unwrap();
        assert!(all.iter().all(|n| n["read"] == true) && all.len() == 2);
        // a deleted user's read rows go with them
        st.db.call(|c| c.execute("DELETE FROM users WHERE id='a'", [])).await.unwrap();
        assert_eq!(st.db.call(|c| c.query_row("SELECT COUNT(*) FROM notification_reads", [], |r| r.get::<_, i64>(0))).await.unwrap(), 0);
    }
}
