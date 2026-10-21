//! Web Push endpoints: the VAPID public key and the caller's browser subscriptions.
use super::*;
use crate::notify::push;
use rusqlite::params;

pub(super) async fn push_key(State(st): State<St>, _user: User) -> ApiResult<Json<Value>> {
    let dir = st.cfg.data_dir.clone();
    let key = tokio::task::spawn_blocking(move || push::load_or_create_key(&dir)).await.map_err(|e| anyhow::anyhow!("{e}"))?.map_err(ApiError::from)?;
    Ok(Json(json!({"key": push::public_key_b64(&key)})))
}

#[derive(Deserialize)]
pub(super) struct Keys { p256dh: String, auth: String }
#[derive(Deserialize)]
pub(super) struct SubReq { endpoint: String, keys: Keys }

pub(super) async fn push_subscribe(State(st): State<St>, user: User, Json(r): Json<SubReq>) -> ApiResult<StatusCode> {
    push::validate_subscription(&r.endpoint, &r.keys.p256dh, &r.keys.auth, st.cfg.allow_private_webhooks)?;
    let (uid, id, now) = (user.id.clone(), uuid::Uuid::now_v7().to_string(), crate::db::now());
    let refused = st.db.call(move |c| {
        let known: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM push_subscriptions WHERE endpoint=?1 AND user_id=?2)", params![r.endpoint, uid], |x| x.get(0))?;
        let n: i64 = c.query_row("SELECT COUNT(*) FROM push_subscriptions WHERE user_id=?", [&uid], |x| x.get(0))?;
        if !known && n >= push::MAX_PER_USER { return Ok(Some("limit")); }
        // another user's subscription moves only when the caller also holds its auth secret (the same browser after a
        // sign-in switch); knowing the endpoint URL alone is not enough
        let n = c.execute("INSERT INTO push_subscriptions(id,user_id,endpoint,p256dh,auth,created_at) VALUES(?,?,?,?,?,?)
            ON CONFLICT(endpoint) DO UPDATE SET user_id=excluded.user_id, p256dh=excluded.p256dh, auth=excluded.auth
            WHERE push_subscriptions.user_id=excluded.user_id OR push_subscriptions.auth=excluded.auth",
            params![id, uid, r.endpoint, r.keys.p256dh, r.keys.auth, now])?;
        Ok((n == 0).then_some("taken"))
    }).await?;
    match refused {
        Some("limit") => return Err(ApiError::conflict("limit", format!("at most {} push subscriptions per user", push::MAX_PER_USER))),
        Some(_) => return Err(ApiError::conflict("push.subscription_taken", "this push subscription belongs to another account")),
        None => {}
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub(super) struct UnsubReq { endpoint: String }

pub(super) async fn push_unsubscribe(State(st): State<St>, user: User, Json(r): Json<UnsubReq>) -> ApiResult<StatusCode> {
    let uid = user.id.clone();
    st.db.call(move |c| c.execute("DELETE FROM push_subscriptions WHERE endpoint=? AND user_id=?", params![r.endpoint, uid])).await?;
    Ok(StatusCode::NO_CONTENT)
}
