//! Personal API token management. Creation and revocation need an interactive session (the extractor refuses tokens here).
use super::*;
use auth::tokens::{self as pat, Created};

#[derive(Deserialize)]
pub(super) struct TokenReq { name: String, scope: String, expires_in_days: Option<i64> }

pub(super) async fn tokens_create(State(st): State<St>, user: User, Json(r): Json<TokenReq>) -> ApiResult<(StatusCode, Json<Value>)> {
    let name = r.name.trim().to_string();
    if name.is_empty() || name.chars().count() > 60 { return Err(ApiError::bad("validation", "invalid name").field("name", "1 to 60 characters")); }
    if !["read", "full"].contains(&r.scope.as_str()) { return Err(ApiError::bad("validation", "invalid scope").field("scope", "read or full")); }
    if r.expires_in_days.is_some_and(|d| !(1..=3650).contains(&d)) { return Err(ApiError::bad("validation", "invalid expiry").field("expires_in_days", "1 to 3650 days")); }
    let tok = pat::generate();
    let (uid, n2, scope, days, shown) = (user.id.clone(), name.clone(), r.scope.clone(), r.expires_in_days, tok.token.clone());
    match st.db.call(move |c| pat::insert(c, &uid, &n2, &scope, &tok, days)).await? {
        Created::TooMany => Err(ApiError::new(StatusCode::CONFLICT, "token.limit", format!("you can have at most {} active tokens", pat::MAX_PER_USER))),
        Created::Ok { id, expires_at } => {
            st.audit(&user.actor(), "token.create", &name, &r.scope, None).await;
            Ok((StatusCode::CREATED, Json(json!({"id": id, "token": shown, "name": name, "scope": r.scope, "expires_at": expires_at}))))
        }
    }
}

pub(super) async fn tokens_list(State(st): State<St>, user: User, Query(q): Q) -> ApiResult<Json<Value>> {
    let all = q.get("all").is_some_and(|v| v == "1" || v == "true");
    if all { user.require_admin()?; }
    let uid = user.id.clone();
    let v: Vec<Value> = st.db.call(move |c| {
        let mut s = c.prepare("SELECT t.id,t.name,t.scope,t.prefix,t.created_at,t.last_used,t.expires_at,u.email FROM api_tokens t JOIN users u ON u.id=t.user_id WHERE (?1 OR t.user_id=?2) ORDER BY t.created_at")?;
        s.query_map(rusqlite::params![all, uid], |r| {
            let mut o = json!({"id": r.get::<_, String>(0)?, "name": r.get::<_, String>(1)?, "scope": r.get::<_, String>(2)?, "prefix": r.get::<_, String>(3)?,
                "created_at": r.get::<_, String>(4)?, "last_used": r.get::<_, Option<String>>(5)?, "expires_at": r.get::<_, Option<String>>(6)?});
            if all { o["owner"] = json!(r.get::<_, String>(7)?); }
            Ok(o)
        })?.collect()
    }).await?;
    Ok(Json(json!(v)))
}

pub(super) async fn tokens_delete(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<StatusCode> {
    let (i, uid, admin) = (id.clone(), user.id.clone(), user.is_admin());
    let name: Option<String> = st.db.call(move |c| {
        let name: Option<String> = c.query_row("SELECT name FROM api_tokens WHERE id=? AND (user_id=? OR ?)", rusqlite::params![i, uid, admin], |r| r.get(0)).ok();
        if name.is_some() { c.execute("DELETE FROM api_tokens WHERE id=?", [&i])?; }
        Ok(name)
    }).await?;
    let name = name.ok_or_else(|| ApiError::not_found("token"))?;
    st.audit(&user.actor(), "token.revoke", &name, "", None).await;
    Ok(StatusCode::NO_CONTENT)
}
