//! Session-level endpoints: auth config, current user, WebSocket/SSE tickets, instance status, runtimes and Java.
use super::*;

pub(super) async fn auth_config(State(st): State<St>) -> Json<Value> {
    let claimed = st.db.call(|c| c.query_row("SELECT COUNT(*) FROM users WHERE role='owner'", [], |r| r.get::<_, i64>(0))).await.unwrap_or(0) > 0;
    Json(json!({"mode": if st.cfg.dev() { "dev" } else { "supabase" }, "supabase_url": st.cfg.supabase_url, "supabase_anon_key": st.cfg.supabase_anon_key, "owner_claimed": claimed}))
}

pub(super) async fn me(user: User) -> Json<Value> {
    let perms: Vec<&str> = if user.is_admin() { auth::ALL_PERMS.to_vec() } else { vec![] };
    Json(json!({"id": user.id, "email": user.email, "role": user.role, "permissions": perms}))
}

pub(super) async fn ticket(State(st): State<St>, user: User) -> ApiResult<Json<Value>> {
    let t = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
    {
        let mut m = st.tickets.lock().unwrap();
        m.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(30));
        if m.len() >= 2000 { return Err(ApiError::new(StatusCode::TOO_MANY_REQUESTS, "ticket.busy", "too many outstanding tickets")); }
        m.insert(t.clone(), (Instant::now(), user));
    }
    Ok(Json(json!({"ticket": t})))
}

pub(super) async fn status(State(st): State<St>, _u: User) -> Json<Value> {
    let (cpu, running, players) = {
        let cpu = st.host.lock().unwrap().cpu_pct;
        let live = st.live.lock().unwrap();
        (cpu, live.values().filter(|l| l.alive).count(), live.values().filter_map(|l| l.players.as_ref().map(|p| p.online)).sum::<u32>())
    };
    let total = server::list(&st).await.map(|s| s.len()).unwrap_or(0);
    Json(json!({"version": env!("CARGO_PKG_VERSION"), "uptime_s": st.started.elapsed().as_secs(), "host": crate::platform::host_json(cpu, &st.cfg.data_dir),
        "counts": {"servers": total, "running": running, "players": players}, "auth_mode": if st.cfg.dev() { "dev" } else { "supabase" }}))
}

pub(super) async fn runtimes_list(State(st): State<St>, _u: User) -> Json<Value> { Json(runtimes::catalogue(st.cfg.dev_runtimes)) }

pub(super) async fn runtime_versions(State(st): State<St>, _u: User, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    if id == "sim" && !st.cfg.dev_runtimes { return Err(ApiError::not_found("runtime")); }
    runtimes::versions(&st, &id).await.map(Json).map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, "upstream.unavailable", format!("could not load versions: {e:#}")))
}

pub(super) async fn java_list(State(st): State<St>, _u: User) -> Json<Value> {
    let st2 = st.clone();
    Json(tokio::task::spawn_blocking(move || crate::java::json_list(&st2, true)).await.unwrap_or(json!({"installs": []})))
}

#[derive(Deserialize)] pub(super) struct JavaReq { major: u32 }
pub(super) async fn java_install(State(st): State<St>, user: User, Json(r): Json<JavaReq>) -> ApiResult<(StatusCode, Json<Value>)> {
    user.require_admin()?;
    if ![8, 11, 17, 21, 25].contains(&r.major) { return Err(ApiError::bad("java.version", "supported majors: 8, 11, 17, 21, 25")); }
    let op = ops::spawn_op(&st, "java.install", format!("Install Java {}", r.major), None, true, move |ctx| crate::java::install_managed(ctx, r.major));
    st.audit(&user.email, "java.install", &format!("Java {}", r.major), "", None).await;
    Ok((StatusCode::ACCEPTED, Json(json!({"operation": op}))))
}
