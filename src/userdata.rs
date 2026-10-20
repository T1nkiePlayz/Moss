//! Per-user data: synced browser preferences (`/me/preferences`) and saved list views (`/views`). Every query is
//! scoped to the caller's own user id, so one person can never read or change another person's data.
use crate::{auth::User, error::{ApiError, ApiResult}, state::St};
use axum::{Json, extract::{Path, Query, State}, http::StatusCode};
use serde::Deserialize;
use serde_json::{Value, json};

const PREFS_MAX: usize = 32 * 1024;
const VIEWS_PER_SCOPE: i64 = 50;
const SCOPES: [&str; 2] = ["servers", "content"];

pub async fn prefs_get(State(st): State<St>, user: User) -> ApiResult<Json<Value>> {
    let uid = user.id.clone();
    let row: Option<(String, String)> = st.db.call(move |c| {
        match c.query_row("SELECT data, updated_at FROM user_prefs WHERE user_id=?", [uid], |r| Ok((r.get(0)?, r.get(1)?))) {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }).await?;
    Ok(Json(match row {
        Some((d, at)) => json!({"data": serde_json::from_str::<Value>(&d).unwrap_or(json!({})), "updated_at": at}),
        None => json!({"data": {}, "updated_at": null}),
    }))
}

#[derive(Deserialize)]
pub struct PrefsReq { data: Value }

pub async fn prefs_put(State(st): State<St>, user: User, Json(r): Json<PrefsReq>) -> ApiResult<Json<Value>> {
    if !r.data.is_object() { return Err(ApiError::bad("validation", "preferences must be an object").field("data", "object required")); }
    let text = serde_json::to_string(&r.data).map_err(|e| ApiError::bad("validation", e.to_string()))?;
    if text.len() > PREFS_MAX { return Err(ApiError::bad("validation", "preferences are too large").field("data", "at most 32 KB")); }
    let (uid, now) = (user.id.clone(), crate::db::now());
    let n = now.clone();
    st.db.call(move |c| c.execute("INSERT INTO user_prefs(user_id,data,updated_at) VALUES (?,?,?) ON CONFLICT(user_id) DO UPDATE SET data=excluded.data, updated_at=excluded.updated_at", rusqlite::params![uid, text, n])).await?;
    Ok(Json(json!({"data": r.data, "updated_at": now})))
}

#[derive(Deserialize)]
pub struct ViewsQ { scope: Option<String> }

pub async fn views_list(State(st): State<St>, user: User, Query(q): Query<ViewsQ>) -> ApiResult<Json<Value>> {
    if let Some(s) = &q.scope && !SCOPES.contains(&s.as_str()) { return Err(ApiError::bad("validation", "unknown scope").field("scope", "servers | content")); }
    let (uid, scope) = (user.id.clone(), q.scope.clone());
    let v: Vec<Value> = st.db.call(move |c| {
        let mut s = c.prepare("SELECT id,scope,name,filters,sort,created_at FROM saved_views WHERE user_id=?1 AND (?2 IS NULL OR scope=?2) ORDER BY created_at, id")?;
        s.query_map(rusqlite::params![uid, scope], |r| Ok(json!({"id": r.get::<_, String>(0)?, "scope": r.get::<_, String>(1)?, "name": r.get::<_, String>(2)?,
            "filters": serde_json::from_str::<Value>(&r.get::<_, String>(3)?).unwrap_or(json!({})), "sort": r.get::<_, String>(4)?, "created_at": r.get::<_, String>(5)?})))?.collect()
    }).await?;
    Ok(Json(Value::Array(v)))
}

#[derive(Deserialize)]
pub struct ViewReq { scope: String, name: String, #[serde(default)] filters: Value, #[serde(default)] sort: String }

pub async fn views_create(State(st): State<St>, user: User, Json(r): Json<ViewReq>) -> ApiResult<(StatusCode, Json<Value>)> {
    let name = r.name.trim().to_string();
    if !SCOPES.contains(&r.scope.as_str()) { return Err(ApiError::bad("validation", "unknown scope").field("scope", "servers | content")); }
    if name.is_empty() || name.chars().count() > 48 || name.chars().any(|c| c.is_control()) { return Err(ApiError::bad("validation", "invalid name").field("name", "1–48 characters")); }
    let filters = if r.filters.is_null() { json!({}) } else { r.filters };
    if !filters.is_object() { return Err(ApiError::bad("validation", "filters must be an object").field("filters", "object required")); }
    let ftext = serde_json::to_string(&filters).unwrap();
    if ftext.len() > 4096 { return Err(ApiError::bad("validation", "filters are too large").field("filters", "at most 4 KB")); }
    if r.sort.len() > 64 || r.sort.chars().any(|c| c.is_control()) { return Err(ApiError::bad("validation", "invalid sort").field("sort", "at most 64 characters")); }
    let (id, now) = (uuid::Uuid::now_v7().to_string(), crate::db::now());
    let (i, uid, scope, nm, sort, n) = (id.clone(), user.id.clone(), r.scope.clone(), name.clone(), r.sort.clone(), now.clone());
    let made = st.db.call(move |c| {
        let used: i64 = c.query_row("SELECT COUNT(*) FROM saved_views WHERE user_id=? AND scope=?", rusqlite::params![uid, scope], |r| r.get(0))?;
        if used >= VIEWS_PER_SCOPE { return Ok(false); }
        // saving under an existing name replaces that view
        c.execute("DELETE FROM saved_views WHERE user_id=? AND scope=? AND name=? COLLATE NOCASE", rusqlite::params![uid, scope, nm])?;
        c.execute("INSERT INTO saved_views(id,user_id,scope,name,filters,sort,created_at) VALUES (?,?,?,?,?,?,?)", rusqlite::params![i, uid, scope, nm, ftext, sort, n])?;
        Ok(true)
    }).await?;
    if !made { return Err(ApiError::conflict("views.limit", "You can keep up to 50 saved views per list. Delete one first.")); }
    Ok((StatusCode::CREATED, Json(json!({"id": id, "scope": r.scope, "name": name, "filters": filters, "sort": r.sort, "created_at": now}))))
}

pub async fn views_delete(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<StatusCode> {
    let uid = user.id.clone();
    let n = st.db.call(move |c| c.execute("DELETE FROM saved_views WHERE id=? AND user_id=?", rusqlite::params![id, uid])).await?;
    if n == 0 { return Err(ApiError::not_found("view")); }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn migration_creates_the_tables() {
        let c = rusqlite::Connection::open_in_memory().unwrap();
        crate::db::migrations::run(&c).unwrap();
        for t in ["user_prefs", "saved_views", "blueprints"] { c.prepare(&format!("SELECT * FROM {t}")).unwrap(); }
        assert_eq!(SCOPES.len(), 2);
    }
}
