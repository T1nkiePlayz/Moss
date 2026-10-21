//! Workflow endpoints. Administrators manage every workflow; other users manage their own, and only for servers where
//! they hold the permissions the actions need (the same permissions are re-checked every time a workflow runs).
use super::*;
use crate::{webhooks::EVENT_TYPES, workflows::{self, Action, Workflow, run}};

fn v(field: &str, m: &str) -> ApiError { ApiError::bad("validation", m.to_string()).field(field, m) }

fn can_manage(user: &User, w: &Workflow) -> bool { user.is_admin() || w.created_by == user.id }

async fn find(st: &St, user: &User, id: &str) -> ApiResult<Workflow> {
    workflows::get(st, id).await?.filter(|w| can_manage(user, w)).ok_or_else(|| ApiError::not_found("workflow"))
}

/// Applies the fields present in `b` on top of `w` and validates the result.
async fn apply(st: &St, user: &User, mut w: Workflow, b: &Value) -> ApiResult<Workflow> {
    if !b.is_object() { return Err(ApiError::bad("validation", "expected a JSON object")); }
    if let Some(n) = b.get("name") {
        let n = n.as_str().unwrap_or_default().trim();
        if n.is_empty() || n.chars().count() > 60 || n.chars().any(char::is_control) { return Err(v("name", "Give the workflow a name of up to 60 characters.")); }
        w.name = n.to_string();
    }
    if let Some(t) = b.get("trigger") { w.trigger = t.as_str().unwrap_or_default().to_string(); }
    if let Some(l) = b.get("server_ids") {
        let l: Vec<String> = serde_json::from_value(l.clone()).map_err(|_| v("server_ids", "Expected a list of server ids."))?;
        let known: Vec<String> = server::list(st).await?.into_iter().map(|s| s.id).collect();
        let mut out: Vec<String> = vec![];
        for i in l {
            if !known.contains(&i) { return Err(v("server_ids", "One of the chosen servers no longer exists.")); }
            if !out.contains(&i) { out.push(i); }
        }
        w.server_ids = out;
    }
    if let Some(c) = b.get("condition") { w.condition = if c.is_null() { None } else { Some(c.clone()) }; }
    if let Some(a) = b.get("actions") { w.actions = serde_json::from_value::<Vec<Action>>(a.clone()).map_err(|e| v("actions", &format!("Not a valid list of actions: {e}")))?; }
    if let Some(e) = b.get("enabled") {
        w.enabled = e.as_bool().ok_or_else(|| v("enabled", "Expected true or false."))?;
        if w.enabled { w.disabled_reason = None; }
    }
    workflows::validate(&w.trigger, w.condition.as_ref(), &w.actions, st.cfg.allow_private_webhooks).map_err(|(f, m)| v(&f, &m))?;
    // explicit action targets must exist
    for (i, a) in w.actions.iter().enumerate() {
        if let Some((t, _)) = a.target() && t != "trigger" && server::get(st, t).await.is_err() { return Err(v(&format!("actions[{i}]"), "That server no longer exists.")); }
    }
    if let Some(p) = run::permission_problem(st, user, &w.actions, &w.server_ids).await { return Err(ApiError::forbidden(format!("You cannot create this workflow: {p}"))); }
    Ok(w)
}

fn blank(user: &User) -> Workflow {
    Workflow { id: uuid::Uuid::now_v7().to_string(), name: String::new(), enabled: true, trigger: String::new(), server_ids: vec![], condition: None, actions: vec![],
        created_by: user.id.clone(), created_at: crate::db::now(), last_run_at: None, last_result: None, disabled_reason: None }
}

pub(super) async fn workflows_list(State(st): State<St>, user: User) -> ApiResult<Json<Value>> {
    let items: Vec<Value> = workflows::list(&st).await?.iter().filter(|w| can_manage(&user, w)).map(Workflow::to_json).collect();
    Ok(Json(json!({"items": items, "event_types": EVENT_TYPES.iter().map(|(t, d)| json!({"type": t, "description": d})).collect::<Vec<_>>()})))
}

pub(super) async fn workflows_create(State(st): State<St>, user: User, Json(b): Json<Value>) -> ApiResult<(StatusCode, Json<Value>)> {
    if b.get("name").is_none() { return Err(v("name", "Give the workflow a name of up to 60 characters.")); }
    let w = apply(&st, &user, blank(&user), &b).await?;
    workflows::insert(&st, &w).await?;
    st.audit(&user.actor(), "workflow.create", &w.name, &w.trigger, None).await;
    Ok((StatusCode::CREATED, Json(w.to_json())))
}

pub(super) async fn workflows_get(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    Ok(Json(find(&st, &user, &id).await?.to_json()))
}

pub(super) async fn workflows_patch(State(st): State<St>, user: User, Path(id): Path<String>, Json(b): Json<Value>) -> ApiResult<Json<Value>> {
    let w = find(&st, &user, &id).await?;
    let w = apply(&st, &user, w, &b).await?;
    workflows::update(&st, &w).await?;
    st.audit(&user.actor(), "workflow.update", &w.name, "", None).await;
    Ok(Json(find(&st, &user, &id).await?.to_json()))
}

pub(super) async fn workflows_delete(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<StatusCode> {
    let w = find(&st, &user, &id).await?;
    workflows::delete(&st, &id).await?;
    st.audit(&user.actor(), "workflow.delete", &w.name, "", None).await;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn workflows_runs(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    find(&st, &user, &id).await?;
    Ok(Json(json!({"items": workflows::runs(&st, &id).await?})))
}

/// Dry run with a sample event; nothing is executed. Optional body: `{"server_id": "...", "data": {...}}`.
pub(super) async fn workflows_test(State(st): State<St>, user: User, Path(id): Path<String>, body: Option<Json<Value>>) -> ApiResult<Json<Value>> {
    let w = find(&st, &user, &id).await?;
    let b = body.map(|b| b.0).unwrap_or(Value::Null);
    let all = server::list(&st).await?;
    let pick = b["server_id"].as_str().map(String::from).or_else(|| w.server_ids.first().cloned()).or_else(|| all.first().map(|s| s.id.clone()));
    let server = pick.and_then(|p| all.iter().find(|s| s.id == p).map(|s| (s.id.clone(), s.name.clone())));
    let mut ev = run::sample_event(&w.trigger, server.or_else(|| Some(("sample".into(), "Sample server".into()))).filter(|_| w.trigger != "update.available"));
    if let (Some(extra), Some(d)) = (b["data"].as_object(), ev.data.as_object_mut()) { for (k, x) in extra { d.insert(k.clone(), x.clone()); } }
    Ok(Json(run::plan(&st, &w, &ev).await))
}
