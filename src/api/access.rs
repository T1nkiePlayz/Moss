//! User administration and per-server access grants.
use super::*;

pub(super) async fn users_list(State(st): State<St>, user: User) -> ApiResult<Json<Value>> {
    user.require_admin()?;
    let v: Vec<Value> = st.db.call(|c| {
        let mut s = c.prepare("SELECT id,email,role,linked,created_at FROM users ORDER BY created_at")?;
        s.query_map([], |r| Ok(json!({"id": r.get::<_, String>(0)?, "email": r.get::<_, String>(1)?, "role": r.get::<_, String>(2)?, "linked": r.get::<_, bool>(3)?, "created_at": r.get::<_, String>(4)?})))?.collect()
    }).await?;
    Ok(Json(json!(v)))
}
#[derive(Deserialize)] pub(super) struct UserReq { email: Option<String>, role: String }
pub(super) async fn users_create(State(st): State<St>, user: User, Json(r): Json<UserReq>) -> ApiResult<(StatusCode, Json<Value>)> {
    user.require_admin()?;
    let email = r.email.unwrap_or_default().trim().to_lowercase();
    if !email.contains('@') || email.len() > 200 { return Err(ApiError::bad("validation", "invalid email").field("email", "enter a valid email")); }
    if !["admin", "user"].contains(&r.role.as_str()) || (r.role == "admin" && user.role != "owner") { return Err(ApiError::forbidden("only the owner can create administrators")); }
    let (id, e2, role) = (uuid::Uuid::now_v7().to_string(), email.clone(), r.role.clone());
    let i2 = id.clone();
    st.db.call(move |c| c.execute("INSERT INTO users(id,email,role,linked,created_at) VALUES (?,?,?,0,?)", rusqlite::params![i2, e2, role, crate::db::now()])).await
        .map_err(|_| ApiError::conflict("user.exists", "that email already has access"))?;
    st.audit(&user.email, "user.invite", &email, &r.role, None).await;
    Ok((StatusCode::CREATED, Json(json!({"id": id, "email": email, "role": r.role, "linked": false}))))
}
pub(super) async fn users_patch(State(st): State<St>, user: User, Path(id): Path<String>, Json(r): Json<UserReq>) -> ApiResult<StatusCode> {
    user.require_admin()?;
    if !["admin", "user"].contains(&r.role.as_str()) || user.role != "owner" { return Err(ApiError::forbidden("only the owner can change roles")); }
    let (i, role) = (id.clone(), r.role.clone());
    let n = st.db.call(move |c| c.execute("UPDATE users SET role=? WHERE id=? AND role<>'owner'", [role, i])).await?;
    if n == 0 { return Err(ApiError::not_found("user")); }
    auth::drop_cached(&st, &id);
    st.audit(&user.email, "user.role", &id, &r.role, None).await;
    Ok(StatusCode::NO_CONTENT)
}
pub(super) async fn users_delete(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<StatusCode> {
    user.require_admin()?;
    if id == user.id { return Err(ApiError::bad("user.self", "you cannot remove yourself")); }
    let i = id.clone();
    let n = st.db.call(move |c| c.execute("DELETE FROM users WHERE id=? AND role<>'owner' AND (role<>'admin' OR ?='owner')", rusqlite::params![i, user.role.clone()])).await?;
    if n == 0 { return Err(ApiError::forbidden("cannot remove this user")); }
    auth::drop_cached(&st, &id);
    st.audit(&user.email, "user.remove", &id, "", None).await;
    Ok(StatusCode::NO_CONTENT)
}
pub(super) async fn access_get(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    user.require_admin()?;
    let i = id.clone();
    let v: Vec<Value> = st.db.call(move |c| {
        let mut s = c.prepare("SELECT g.user_id,u.email,g.bundle FROM grants g JOIN users u ON u.id=g.user_id WHERE g.server_id=?")?;
        s.query_map([i], |r| Ok(json!({"user_id": r.get::<_, String>(0)?, "email": r.get::<_, String>(1)?, "bundle": r.get::<_, String>(2)?})))?.collect()
    }).await?;
    Ok(Json(json!(v)))
}
#[derive(Deserialize)] struct Grant { user_id: String, bundle: String }
#[derive(Deserialize)] pub(super) struct GrantsReq { grants: Vec<Grant> }
pub(super) async fn access_put(State(st): State<St>, user: User, Path(id): Path<String>, Json(r): Json<GrantsReq>) -> ApiResult<StatusCode> {
    user.require_admin()?;
    if r.grants.iter().any(|g| auth::bundle_perms(&g.bundle).is_empty()) { return Err(ApiError::bad("validation", "bundle must be manager, operator or viewer")); }
    let i = id.clone();
    st.db.call(move |c| {
        c.execute("DELETE FROM grants WHERE server_id=?", [&i])?;
        for g in r.grants { c.execute("INSERT OR REPLACE INTO grants(server_id,user_id,bundle) VALUES (?,?,?)", [&i, &g.user_id, &g.bundle])?; }
        Ok(())
    }).await.map_err(|_| ApiError::bad("validation", "unknown user in grants"))?;
    st.users.lock().unwrap().clear();
    st.audit(&user.email, "access.update", &id, "", Some(&id)).await;
    Ok(StatusCode::NO_CONTENT)
}
