//! Shared player lists. Managing lists and entries is for administrators; a server's subscriptions are read by anyone
//! who can view the server and changed by administrators. Every change is applied to the affected servers in the
//! background (`playerlists::apply`) and recorded in the activity log.
use super::*;
use crate::playerlists::{self, valid_name};

#[derive(Deserialize, Default)] pub(super) struct ListBody { name: Option<String>, kind: Option<String> }
#[derive(Deserialize, Default)] pub(super) struct EntryBody { name: Option<String>, reason: Option<String> }
#[derive(Deserialize, Default)] pub(super) struct SubBody { list_ids: Option<Vec<String>> }

fn v(field: &str, m: &str) -> ApiError { ApiError::bad("validation", m.to_string()).field(field, m) }

fn list_name(s: &str) -> ApiResult<String> {
    let s = s.trim();
    if s.is_empty() || s.chars().count() > 60 || s.chars().any(char::is_control) { return Err(v("name", "Give the list a name of up to 60 characters.")); }
    Ok(s.to_string())
}

async fn kind_of(st: &St, id: &str) -> ApiResult<String> {
    let i = id.to_string();
    st.db.call(move |c| c.query_row("SELECT kind FROM player_lists WHERE id=?", [i], |r| r.get::<_, String>(0)).map(Some).or_else(|e| if matches!(e, rusqlite::Error::QueryReturnedNoRows) { Ok(None) } else { Err(e) })).await?.ok_or_else(|| ApiError::not_found("player list"))
}

async fn subscribers(st: &St, list_id: &str) -> ApiResult<Vec<String>> {
    let i = list_id.to_string();
    Ok(st.db.call(move |c| c.prepare("SELECT server_id FROM player_list_subs WHERE list_id=?")?.query_map([i], |r| r.get(0))?.collect()).await?)
}

pub(super) async fn lists(State(st): State<St>, user: User) -> ApiResult<Json<Value>> {
    user.require_admin()?;
    let items = st.db.call(|c| {
        c.prepare("SELECT l.id, l.name, l.kind, l.created_at, (SELECT COUNT(*) FROM player_list_entries WHERE list_id=l.id), (SELECT COUNT(*) FROM player_list_subs WHERE list_id=l.id)
            FROM player_lists l ORDER BY l.created_at, l.id")?
            .query_map([], |r| Ok(json!({"id": r.get::<_, String>(0)?, "name": r.get::<_, String>(1)?, "kind": r.get::<_, String>(2)?, "created_at": r.get::<_, String>(3)?, "entries": r.get::<_, i64>(4)?, "servers": r.get::<_, i64>(5)?})))?.collect::<rusqlite::Result<Vec<_>>>()
    }).await?;
    Ok(Json(json!({"items": items})))
}

pub(super) async fn list_create(State(st): State<St>, user: User, Json(b): Json<ListBody>) -> ApiResult<(StatusCode, Json<Value>)> {
    user.require_admin()?;
    let name = list_name(b.name.as_deref().unwrap_or(""))?;
    let kind = match b.kind.as_deref() { Some(k @ ("allow" | "ban")) => k.to_string(), _ => return Err(v("kind", "Choose allow (a whitelist) or ban.")) };
    let (id, now) = (uuid::Uuid::now_v7().to_string(), crate::db::now());
    let (i2, n2, k2, t2) = (id.clone(), name.clone(), kind.clone(), now.clone());
    st.db.call(move |c| c.execute("INSERT INTO player_lists(id,name,kind,created_at) VALUES (?,?,?,?)", rusqlite::params![i2, n2, k2, t2])).await?;
    st.audit(&user.actor(), "playerlist.create", &name, &kind, None).await;
    Ok((StatusCode::CREATED, Json(json!({"id": id, "name": name, "kind": kind, "created_at": now, "entries": 0, "servers": 0}))))
}

pub(super) async fn list_patch(State(st): State<St>, user: User, Path(id): Path<String>, Json(b): Json<ListBody>) -> ApiResult<Json<Value>> {
    user.require_admin()?;
    let kind = kind_of(&st, &id).await?;
    let name = list_name(b.name.as_deref().unwrap_or(""))?;
    let (i2, n2) = (id.clone(), name.clone());
    st.db.call(move |c| c.execute("UPDATE player_lists SET name=? WHERE id=?", rusqlite::params![n2, i2])).await?;
    st.audit(&user.actor(), "playerlist.rename", &name, &kind, None).await;
    Ok(Json(json!({"id": id, "name": name, "kind": kind})))
}

pub(super) async fn list_delete(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<StatusCode> {
    user.require_admin()?;
    let kind = kind_of(&st, &id).await?;
    let subs = subscribers(&st, &id).await?;
    let i2 = id.clone();
    let name: String = st.db.call(move |c| { let n = c.query_row("SELECT name FROM player_lists WHERE id=?", [&i2], |r| r.get(0))?; c.execute("DELETE FROM player_lists WHERE id=?", [&i2])?; Ok(n) }).await?;
    playerlists::apply_many_bg(&st, subs);
    st.audit(&user.actor(), "playerlist.delete", &name, &kind, None).await;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn entries(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    user.require_admin()?;
    kind_of(&st, &id).await?;
    let items = st.db.call(move |c| {
        c.prepare("SELECT name, uuid, reason, added_by, added_at FROM player_list_entries WHERE list_id=? ORDER BY lower(name)")?
            .query_map([id], |r| Ok(json!({"name": r.get::<_, String>(0)?, "uuid": r.get::<_, Option<String>>(1)?, "reason": r.get::<_, String>(2)?, "added_by": r.get::<_, String>(3)?, "added_at": r.get::<_, String>(4)?})))?.collect::<rusqlite::Result<Vec<_>>>()
    }).await?;
    Ok(Json(json!({"items": items})))
}

pub(super) async fn entry_add(State(st): State<St>, user: User, Path(id): Path<String>, Json(b): Json<EntryBody>) -> ApiResult<(StatusCode, Json<Value>)> {
    user.require_admin()?;
    let kind = kind_of(&st, &id).await?;
    let name = b.name.unwrap_or_default().trim().to_string();
    if !valid_name(&name) { return Err(v("name", "Enter a player name of up to 64 characters without quotes or control characters.")); }
    let reason = if kind == "ban" { b.reason.unwrap_or_default().trim().to_string() } else { String::new() };
    if !reason.is_empty() && !crate::games::admin::safe_text(&reason, 128) { return Err(v("reason", "The reason is up to 128 characters without quotes or control characters.")); }
    let (i2, n2, r2, a2, t2) = (id.clone(), name.clone(), reason.clone(), user.actor(), crate::db::now());
    let added = st.db.call(move |c| c.execute("INSERT OR IGNORE INTO player_list_entries(list_id,name,reason,added_by,added_at) VALUES (?,?,?,?,?)", rusqlite::params![i2, n2, r2, a2, t2])).await?;
    if added == 0 { return Err(ApiError::conflict("player.exists", "That player is already on the list.")); }
    playerlists::apply_many_bg(&st, subscribers(&st, &id).await?);
    st.audit(&user.actor(), "playerlist.add", &name, &format!("{kind} list {id}{}", if reason.is_empty() { String::new() } else { format!(" ({reason})") }), None).await;
    Ok((StatusCode::CREATED, Json(json!({"name": name, "uuid": null, "reason": reason, "added_by": user.actor()}))))
}

pub(super) async fn entry_remove(State(st): State<St>, user: User, Path((id, name)): Path<(String, String)>) -> ApiResult<StatusCode> {
    user.require_admin()?;
    let kind = kind_of(&st, &id).await?;
    let (i2, n2) = (id.clone(), name.clone());
    let n = st.db.call(move |c| c.execute("DELETE FROM player_list_entries WHERE list_id=? AND lower(name)=lower(?)", rusqlite::params![i2, n2])).await?;
    if n == 0 { return Err(ApiError::not_found("player")); }
    playerlists::apply_many_bg(&st, subscribers(&st, &id).await?);
    st.audit(&user.actor(), "playerlist.remove", &name, &format!("{kind} list {id}"), None).await;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn server_get(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    authz(&st, &user, &id, "server.view").await?;
    let s = server::get(&st, &id).await?;
    let sup = playerlists::support(&s);
    let sid = id.clone();
    let (items, (pa, pb)) = st.db.call(move |c| {
        let items = c.prepare("SELECT l.id, l.name, l.kind, (SELECT COUNT(*) FROM player_list_entries WHERE list_id=l.id), EXISTS(SELECT 1 FROM player_list_subs WHERE list_id=l.id AND server_id=?)
            FROM player_lists l ORDER BY l.created_at, l.id")?
            .query_map([&sid], |r| Ok(json!({"id": r.get::<_, String>(0)?, "name": r.get::<_, String>(1)?, "kind": r.get::<_, String>(2)?, "entries": r.get::<_, i64>(3)?, "subscribed": r.get::<_, bool>(4)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let p = playerlists::pending(c, &server::get_sync(c, &sid)?.ok_or(rusqlite::Error::QueryReturnedNoRows)?)?;
        Ok((items, p))
    }).await?;
    let note = if s.is_proxy() { Some("Velocity has no whitelist. Subscribe the servers behind the proxy instead.") }
        else if s.runtime == "bedrock" { Some("Bedrock has no ban command: only allowlists apply here.") }
        else if !sup.allow && sup.ban { Some("This game supports ban lists only; banning works while the server runs, and removing a name does not unban it.") }
        else if !sup.allow && !sup.ban { Some("This game has no admin channel Moss can use for player lists.") } else { None };
    Ok(Json(json!({"lists": items, "support": {"allow": sup.allow, "ban": sup.ban, "note": note}, "pending": {"allow": pa, "ban": pb}, "allowlist_enabled": playerlists::allow_enabled(&st, &s)})))
}

pub(super) async fn server_put(State(st): State<St>, user: User, Path(id): Path<String>, Json(b): Json<SubBody>) -> ApiResult<Json<Value>> {
    user.require_admin()?;
    let s = server::get(&st, &id).await?;
    let sup = playerlists::support(&s);
    let mut ids: Vec<String> = vec![];
    for l in b.list_ids.unwrap_or_default() {
        let kind = kind_of(&st, &l).await.map_err(|_| v("list_ids", "One of the chosen lists no longer exists."))?;
        if !(if kind == "allow" { sup.allow } else { sup.ban }) { return Err(v("list_ids", &format!("This server cannot use {} lists.", if kind == "allow" { "allow" } else { "ban" }))); }
        if !ids.contains(&l) { ids.push(l); }
    }
    let (sid, i2) = (id.clone(), ids.clone());
    st.db.call(move |c| {
        let tx = c.unchecked_transaction()?;
        tx.execute("DELETE FROM player_list_subs WHERE server_id=?", [&sid])?;
        for l in &i2 { tx.execute("INSERT OR IGNORE INTO player_list_subs(list_id,server_id) VALUES (?,?)", [l, &sid])?; }
        tx.commit()
    }).await?;
    playerlists::apply_bg(&st, &id);
    st.audit(&user.actor(), "playerlist.subscribe", &s.name, &format!("{} list(s)", ids.len()), Some(&id)).await;
    Ok(Json(json!({"list_ids": ids})))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(role: &str) -> User { User { id: "9".into(), email: "u@example.com".into(), role: role.into(), token: None, via_token: None, aal: None } }

    #[tokio::test]
    async fn managing_lists_is_admin_only() {
        let st = crate::testutil::state(|_| {}).await;
        let u = || user("user");
        assert_eq!(lists(State(st.clone()), u()).await.unwrap_err().status, StatusCode::FORBIDDEN);
        let body = || Json(ListBody { name: Some("Friends".into()), kind: Some("allow".into()) });
        assert_eq!(list_create(State(st.clone()), u(), body()).await.unwrap_err().status, StatusCode::FORBIDDEN);
        let (code, Json(made)) = list_create(State(st.clone()), user("admin"), body()).await.unwrap();
        assert_eq!(code, StatusCode::CREATED);
        let id = made["id"].as_str().unwrap().to_string();
        assert_eq!(list_patch(State(st.clone()), u(), Path(id.clone()), Json(ListBody { name: Some("x".into()), kind: None })).await.unwrap_err().status, StatusCode::FORBIDDEN);
        assert_eq!(list_delete(State(st.clone()), u(), Path(id.clone())).await.unwrap_err().status, StatusCode::FORBIDDEN);
        assert_eq!(entries(State(st.clone()), u(), Path(id.clone())).await.unwrap_err().status, StatusCode::FORBIDDEN);
        assert_eq!(entry_add(State(st.clone()), u(), Path(id.clone()), Json(EntryBody { name: Some("Alice".into()), reason: None })).await.unwrap_err().status, StatusCode::FORBIDDEN);
        assert_eq!(entry_remove(State(st.clone()), u(), Path((id.clone(), "Alice".into()))).await.unwrap_err().status, StatusCode::FORBIDDEN);
        assert_eq!(server_put(State(st.clone()), u(), Path("s".into()), Json(SubBody::default())).await.unwrap_err().status, StatusCode::FORBIDDEN);
        // an administrator can add, sees a duplicate refused (any case), and can remove
        let add = |n: &str| entry_add(State(st.clone()), user("owner"), Path(id.clone()), Json(EntryBody { name: Some(n.into()), reason: None }));
        let _ = add("Alice").await.unwrap();
        assert_eq!(add("alice").await.unwrap_err().status, StatusCode::CONFLICT);
        assert_eq!(add("bad\"name").await.unwrap_err().status, StatusCode::BAD_REQUEST);
        assert_eq!(entry_remove(State(st.clone()), user("owner"), Path((id.clone(), "ALICE".into()))).await.unwrap(), StatusCode::NO_CONTENT);
    }
}
