//! Operations (list/get/cancel), the activity feed and the server-sent events stream.
use super::*;

pub(super) async fn operations_list(State(st): State<St>, user: User, Query(q): Q) -> ApiResult<Json<Value>> {
    let limit = q.get("limit").and_then(|v| v.parse().ok()).unwrap_or(50usize).min(200);
    let vis = auth::visible_servers(&st, &user).await?;
    let list: Vec<_> = ops::list(&st, q.get("server_id").map(|s| s.as_str()), 300).into_iter()
        .filter(|o| vis.as_ref().is_none_or(|v| o.server_id.as_ref().is_some_and(|s| v.contains(s)))).take(limit).collect();
    Ok(Json(json!(list)))
}
pub(super) async fn operation_get(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let op = ops::get(&st, &id).ok_or_else(|| ApiError::not_found("operation"))?;
    match &op.server_id { Some(s) => authz(&st, &user, s, "server.view").await?, None => user.require_admin()? }
    Ok(Json(json!(op)))
}
pub(super) async fn operation_cancel(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let op = ops::get(&st, &id).ok_or_else(|| ApiError::not_found("operation"))?;
    match &op.server_id { Some(s) => authz(&st, &user, s, "server.power").await?, None => user.require_admin()? }
    if !ops::cancel(&st, &id) { return Err(ApiError::conflict("operation.not_cancellable", "this operation cannot be cancelled right now")); }
    Ok(Json(json!({"ok": true})))
}

pub(super) async fn activity(State(st): State<St>, user: User, Query(q): Q) -> ApiResult<Json<Value>> {
    let limit: i64 = q.get("limit").and_then(|v| v.parse().ok()).unwrap_or(100).clamp(1, 500);
    let vis = auth::visible_servers(&st, &user).await?;
    let sid = q.get("server_id").cloned();
    if let Some(s) = &sid { authz(&st, &user, s, "server.view").await?; }
    let rows: Vec<(Value, Option<String>)> = st.db.call(move |c| {
        let mut s = c.prepare("SELECT id,time,actor,action,target,detail,server_id FROM activity WHERE (?1 IS NULL OR server_id=?1) ORDER BY id DESC LIMIT ?2")?;
        s.query_map(rusqlite::params![sid, limit * 4], |r| Ok((json!({"id": r.get::<_, i64>(0)?, "time": r.get::<_, String>(1)?, "actor": r.get::<_, String>(2)?, "action": r.get::<_, String>(3)?, "target": r.get::<_, String>(4)?, "detail": r.get::<_, String>(5)?}), r.get::<_, Option<String>>(6)?)))?.collect()
    }).await?;
    Ok(Json(Value::Array(rows.into_iter().filter(|(_, s)| vis.as_ref().is_none_or(|v| s.as_ref().is_some_and(|s| v.contains(s)))).map(|(v, _)| v).take(limit as usize).collect())))
}

/// Server-sent events. A subscriber that fell behind the broadcast buffer gets `resync` (it must refetch its lists).
pub(super) async fn events(State(st): State<St>, headers: axum::http::HeaderMap, Query(q): Q) -> ApiResult<Response> {
    super::origin::check(&st, &headers)?;
    let user = take_ticket(&st, q.get("ticket"))?;
    let epoch = auth::epoch();
    let vis = auth::visible_servers(&st, &user).await?;
    let admin = user.is_admin();
    let rx = st.bus.subscribe();
    let hello = tokio_stream::once(Ok::<_, std::convert::Infallible>(sse::Event::default().event("hello").data(json!({"server_time": crate::db::now()}).to_string())));
    // `vis` and `admin` are a snapshot: once any user or grant changes, end the stream so the client reconnects
    // through authorization again rather than keep receiving events it may no longer be allowed to see.
    let stream = BroadcastStream::new(rx).take_while(move |_| auth::epoch() == epoch).filter_map(move |e| {
        let e = match e {
            Ok(e) => e,
            Err(_lagged) => return Some(Ok(sse::Event::default().event("resync").data("{}"))),
        };
        if e.kind == "notification" && !admin && crate::notify::rules::is_admin_only(e.data["kind"].as_str().unwrap_or_default()) { return None; }
        let sid = e.data.get("server_id").and_then(|v| v.as_str()).or_else(|| (e.kind == "server").then(|| e.data["id"].as_str()).flatten()).map(String::from);
        if let Some(v) = &vis && !sid.as_ref().is_some_and(|s| v.contains(s)) { return None; }
        Some(Ok(sse::Event::default().event(e.kind).data(e.data.to_string())))
    });
    Ok(Sse::new(hello.chain(stream)).keep_alive(sse::KeepAlive::new().interval(Duration::from_secs(15))).into_response())
}
