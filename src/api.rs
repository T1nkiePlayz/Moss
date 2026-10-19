use crate::{
    auth::{self, User, authz}, error::{ApiError, ApiResult}, lifecycle, ops, runtimes,
    server::{self, Srv}, state::{Inner, St},
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderValue, StatusCode, Uri, header},
    response::{IntoResponse, Response, Sse, sse},
    routing::{get, patch, post},
};
use rust_embed::RustEmbed;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::HashMap, time::{Duration, Instant}};
use tokio_stream::{StreamExt, wrappers::BroadcastStream};

#[derive(RustEmbed)]
#[folder = "web/dist/"]
struct Assets;

type Q = Query<HashMap<String, String>>;

pub fn take_ticket(st: &St, t: Option<&String>) -> ApiResult<User> {
    let t = t.ok_or_else(|| ApiError::unauthorized("missing ticket"))?;
    let mut m = st.tickets.lock().unwrap();
    m.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(30));
    m.remove(t).map(|(_, u)| u).ok_or_else(|| ApiError::unauthorized("invalid or expired ticket"))
}

pub fn router(st: St) -> Router {
    use crate::{backups, console, content, files, health, sched, undo};
    let api = Router::new()
        .route("/auth/config", get(auth_config))
        .route("/me", get(me))
        .route("/ticket", post(ticket))
        .route("/status", get(status))
        .route("/runtimes", get(runtimes_list))
        .route("/runtimes/{id}/versions", get(runtime_versions))
        .route("/java", get(java_list))
        .route("/java/install", post(java_install))
        .route("/servers", get(servers_list).post(servers_create))
        .route("/servers/{id}", get(server_get).patch(server_patch).delete(server_delete))
        .route("/servers/{id}/actions/{action}", post(server_action))
        .route("/servers/{id}/command", post(server_command))
        .route("/servers/{id}/stats", get(server_stats))
        .route("/servers/{id}/timeline", get(server_timeline))
        .route("/servers/{id}/health", get(health::server_health))
        .route("/servers/{id}/properties", get(files::props_get).put(files::props_put))
        .route("/servers/{id}/console/backlog", get(console::backlog))
        .route("/servers/{id}/console", get(console::ws))
        .route("/servers/{id}/files", get(files::list).delete(files::delete))
        .route("/servers/{id}/files/content", get(files::read).put(files::write))
        .route("/servers/{id}/files/mkdir", post(files::mkdir))
        .route("/servers/{id}/files/rename", post(files::rename))
        .route("/servers/{id}/files/upload", post(files::upload))
        .route("/servers/{id}/files/download", get(files::download))
        .route("/content/search", get(content::search))
        .route("/content/projects/{pid}", get(content::project))
        .route("/servers/{id}/content", get(content::installed).post(content::install))
        .route("/servers/{id}/content/plan", post(content::plan))
        .route("/servers/{id}/content/check-updates", post(content::check_updates))
        .route("/servers/{id}/content/{item}", patch(content::patch_item).delete(content::remove))
        .route("/servers/{id}/content/{item}/update", post(content::update))
        .route("/backups", get(backups::list_all))
        .route("/servers/{id}/backups", get(backups::list).post(backups::create))
        .route("/backups/{id}", axum::routing::delete(backups::delete))
        .route("/backups/{id}/verify", post(backups::verify))
        .route("/backups/{id}/restore", post(backups::restore))
        .route("/backups/{id}/download", get(backups::download))
        .route("/servers/{id}/schedules", get(sched::list).post(sched::create))
        .route("/schedules/{id}", patch(sched::patch).delete(sched::delete))
        .route("/schedules/{id}/run", post(sched::run_now))
        .route("/operations", get(operations_list))
        .route("/operations/{id}", get(operation_get))
        .route("/operations/{id}/cancel", post(operation_cancel))
        .route("/servers/{id}/undo", get(undo::list))
        .route("/undo/{id}/revert", post(undo::revert))
        .route("/activity", get(activity))
        .route("/health", get(health::all))
        .route("/users", get(users_list).post(users_create))
        .route("/users/{id}", patch(users_patch).delete(users_delete))
        .route("/servers/{id}/access", get(access_get).put(access_put))
        .route("/settings", get(settings_get).put(settings_put))
        .route("/events", get(events))
        .fallback(|| async { ApiError::not_found("endpoint") });
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .nest("/api/v1", api)
        .fallback(static_files)
        .layer(tower_http::compression::CompressionLayer::new())
        .layer(tower_http::set_header::SetResponseHeaderLayer::overriding(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")))
        .layer(tower_http::set_header::SetResponseHeaderLayer::overriding(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer")))
        .layer(tower_http::set_header::SetResponseHeaderLayer::overriding(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY")))
        .layer(axum::extract::DefaultBodyLimit::max(512 << 20))
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(st)
}

async fn static_files(State(st): State<St>, uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let (file, is_index) = match Assets::get(path).filter(|_| !path.is_empty()) {
        Some(f) => (f, false),
        None => match Assets::get("index.html") { Some(f) => (f, true), None => return (StatusCode::NOT_FOUND, "web UI is not built: run `npm run build` in web/").into_response() },
    };
    let mime = if is_index { "text/html".to_string() } else { mime_guess::from_path(path).first_or_octet_stream().to_string() };
    let mut r = Response::new(axum::body::Body::from(file.data.into_owned()));
    let h = r.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_str(&mime).unwrap());
    if is_index {
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        let sb = st.cfg.supabase_url.trim_end_matches('/');
        let csp = format!("default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: https://cdn.modrinth.com https://*.supabase.co https://avatars.githubusercontent.com; connect-src 'self' {sb} ws: wss:; frame-ancestors 'none'; base-uri 'none'; form-action 'self'");
        if let Ok(v) = HeaderValue::from_str(&csp) { h.insert(header::CONTENT_SECURITY_POLICY, v); }
    } else if path.starts_with("assets/") {
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=31536000, immutable"));
    }
    r
}

async fn auth_config(State(st): State<St>) -> Json<Value> {
    let claimed = st.db.call(|c| c.query_row("SELECT COUNT(*) FROM users WHERE role='owner'", [], |r| r.get::<_, i64>(0))).await.unwrap_or(0) > 0;
    Json(json!({"mode": if st.cfg.dev() { "dev" } else { "supabase" }, "supabase_url": st.cfg.supabase_url, "supabase_anon_key": st.cfg.supabase_anon_key, "owner_claimed": claimed}))
}

async fn me(user: User) -> Json<Value> {
    let perms: Vec<&str> = if user.is_admin() { auth::ALL_PERMS.to_vec() } else { vec![] };
    Json(json!({"id": user.id, "email": user.email, "role": user.role, "permissions": perms}))
}

async fn ticket(State(st): State<St>, user: User) -> ApiResult<Json<Value>> {
    let t = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
    {
        let mut m = st.tickets.lock().unwrap();
        m.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(30));
        if m.len() >= 2000 { return Err(ApiError::new(StatusCode::TOO_MANY_REQUESTS, "ticket.busy", "too many outstanding tickets")); }
        m.insert(t.clone(), (Instant::now(), user));
    }
    Ok(Json(json!({"ticket": t})))
}

async fn status(State(st): State<St>, _u: User) -> Json<Value> {
    let (cpu, running, players) = {
        let cpu = st.host.lock().unwrap().cpu_pct;
        let live = st.live.lock().unwrap();
        (cpu, live.values().filter(|l| l.alive).count(), live.values().filter_map(|l| l.players.as_ref().map(|p| p.online)).sum::<u32>())
    };
    let total = server::list(&st).await.map(|s| s.len()).unwrap_or(0);
    Json(json!({"version": env!("CARGO_PKG_VERSION"), "uptime_s": st.started.elapsed().as_secs(), "host": crate::metrics::host_json(cpu, &st.cfg.data_dir),
        "counts": {"servers": total, "running": running, "players": players}, "auth_mode": if st.cfg.dev() { "dev" } else { "supabase" }}))
}

async fn runtimes_list(State(st): State<St>, _u: User) -> Json<Value> { Json(runtimes::catalogue(st.cfg.dev_runtimes)) }

async fn runtime_versions(State(st): State<St>, _u: User, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    if id == "sim" && !st.cfg.dev_runtimes { return Err(ApiError::not_found("runtime")); }
    runtimes::versions(&st, &id).await.map(Json).map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, "upstream.unavailable", format!("could not load versions: {e:#}")))
}

async fn java_list(State(st): State<St>, _u: User) -> Json<Value> {
    let st2 = st.clone();
    Json(tokio::task::spawn_blocking(move || crate::java::json_list(&st2, true)).await.unwrap_or(json!({"installs": []})))
}

#[derive(Deserialize)] struct JavaReq { major: u32 }
async fn java_install(State(st): State<St>, user: User, Json(r): Json<JavaReq>) -> ApiResult<(StatusCode, Json<Value>)> {
    user.require_admin()?;
    if ![8, 11, 17, 21, 25].contains(&r.major) { return Err(ApiError::bad("java.version", "supported majors: 8, 11, 17, 21, 25")); }
    let op = ops::spawn_op(&st, "java.install", format!("Install Java {}", r.major), None, true, move |ctx| crate::java::install_managed(ctx, r.major));
    st.audit(&user.email, "java.install", &format!("Java {}", r.major), "", None).await;
    Ok((StatusCode::ACCEPTED, Json(json!({"operation": op}))))
}

// ---------- servers ----------
async fn servers_list(State(st): State<St>, user: User) -> ApiResult<Json<Value>> {
    let vis = auth::visible_servers(&st, &user).await?;
    let all = server::list(&st).await?;
    Ok(Json(Value::Array(all.iter().filter(|s| vis.as_ref().is_none_or(|v| v.contains(&s.id))).map(|s| server::view(s, &st)).collect())))
}

#[derive(Deserialize)]
struct CustomCmd { command: Vec<String> }
#[derive(Deserialize)]
struct CreateReq {
    name: String, runtime: String, game_version: String, loader_version: Option<String>, memory_mb: Option<i64>, port: Option<i64>,
    accept_eula: Option<bool>, flags_profile: Option<String>, autostart: Option<bool>, custom: Option<CustomCmd>,
}

async fn free_port(_st: &St, preferred: i64, used: &[i64]) -> i64 {
    let mut p = preferred;
    loop {
        if !used.contains(&p) && tokio::net::TcpListener::bind(("0.0.0.0", p as u16)).await.is_ok() { return p; }
        p += 1;
        if p > 65000 { return preferred; }
    }
}

async fn servers_create(State(st): State<St>, user: User, Json(r): Json<CreateReq>) -> ApiResult<(StatusCode, Json<Value>)> {
    user.require_admin()?;
    let name = r.name.trim().to_string();
    if name.is_empty() || name.chars().count() > 48 || name.chars().any(|c| c.is_control()) { return Err(ApiError::bad("validation", "invalid name").field("name", "1–48 characters, no control characters")); }
    let cat = runtimes::catalogue(st.cfg.dev_runtimes);
    let rt = cat.as_array().unwrap().iter().find(|x| x["id"] == r.runtime.as_str());
    if !rt.is_some_and(|x| x["installable"] == true) { return Err(ApiError::bad("runtime.unsupported", "unknown or not yet installable runtime").field("runtime", "choose an installable runtime")); }
    let is_mc = !matches!(r.runtime.as_str(), "custom" | "sim");
    if is_mc && r.accept_eula != Some(true) { return Err(ApiError::bad("eula.required", "You must accept the Minecraft EULA to create a Minecraft server.").field("accept_eula", "required")); }
    let mem = r.memory_mb.unwrap_or(2048);
    if !(256..=262144).contains(&mem) { return Err(ApiError::bad("validation", "memory out of range").field("memory_mb", "256–262144")); }
    let custom = match (&r.runtime[..], &r.custom) {
        ("custom", Some(c)) if !c.command.is_empty() && c.command.iter().all(|a| !a.is_empty() && a.len() < 2048 && !a.contains('\0')) => Some(serde_json::to_string(&c.command).unwrap()),
        ("custom", _) => return Err(ApiError::bad("validation", "custom runtime needs a non-empty command array").field("custom.command", "required")),
        _ => None,
    };
    let flags = r.flags_profile.filter(|f| f == "balanced" || f == "minimal").unwrap_or_else(|| "balanced".into());
    let existing: Vec<i64> = server::list(&st).await?.iter().map(|s| s.port).collect();
    let default_port = if r.runtime == "velocity" { 25577 } else { 25565 };
    let port = match r.port {
        Some(p) if !(1..=65535).contains(&p) => return Err(ApiError::bad("validation", "invalid port").field("port", "1–65535")),
        Some(p) if existing.contains(&p) => return Err(ApiError::conflict("port.taken", format!("port {p} is already assigned to another Moss server")).field("port", "already assigned")),
        Some(p) => p,
        None => free_port(&st, default_port, &existing).await,
    };
    let (id, now) = (uuid::Uuid::now_v7().to_string(), crate::db::now());
    let eula = (is_mc && r.accept_eula == Some(true)).then(|| now.clone());
    let row = (id.clone(), name.clone(), r.runtime.clone(), r.game_version.clone(), r.loader_version.clone(), port, mem, flags, r.autostart.unwrap_or(false), eula, user.email.clone(), custom, now);
    st.db.call(move |c| c.execute(
        "INSERT INTO servers(id,name,runtime,game_version,loader_version,port,memory_mb,flags_profile,autostart,eula_at,eula_by,custom_cmd,created_at) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)",
        rusqlite::params![row.0, row.1, row.2, row.3, row.4, row.5, row.6, row.7, row.8, row.9, row.10, row.11, row.12])).await?;
    let s = server::get(&st, &id).await?;
    tokio::fs::create_dir_all(s.dir(&st)).await?;
    if is_mc { st.audit(&user.email, "eula.accept", &name, "Minecraft EULA accepted at server creation", Some(&id)).await; }
    st.audit(&user.email, "server.create", &name, &format!("{} {}", s.runtime, s.game_version), Some(&id)).await;
    let op = ops::spawn_op(&st, "server.install", format!("Install {name}"), Some(&id), true, { let id = id.clone(); move |ctx| crate::installer::install(ctx, id) });
    Ok((StatusCode::ACCEPTED, Json(json!({"server": server::view(&s, &st), "operation": op}))))
}

async fn server_get(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    authz(&st, &user, &id, "server.view").await?;
    Ok(Json(server::view(&server::get(&st, &id).await?, &st)))
}

#[derive(Deserialize)]
struct PatchReq {
    name: Option<String>, memory_mb: Option<i64>, port: Option<i64>, flags_profile: Option<String>, autostart: Option<bool>,
    restart_policy: Option<RestartPolicy>, tags: Option<Vec<String>>, notes: Option<String>, accept_eula: Option<bool>,
}
#[derive(Deserialize)] struct RestartPolicy { enabled: bool, max_per_10m: i64 }

async fn server_patch(State(st): State<St>, user: User, Path(id): Path<String>, Json(p): Json<PatchReq>) -> ApiResult<Json<Value>> {
    authz(&st, &user, &id, "config.write").await?;
    let s = server::get(&st, &id).await?;
    if let Some(n) = &p.name && (n.trim().is_empty() || n.chars().count() > 48) { return Err(ApiError::bad("validation", "invalid name").field("name", "1–48 characters")); }
    if let Some(m) = p.memory_mb && !(256..=262144).contains(&m) { return Err(ApiError::bad("validation", "memory out of range").field("memory_mb", "256–262144")); }
    if let Some(f) = &p.flags_profile && f != "balanced" && f != "minimal" { return Err(ApiError::bad("validation", "unknown flags profile").field("flags_profile", "balanced | minimal")); }
    if let Some(port) = p.port {
        if !(1..=65535).contains(&port) { return Err(ApiError::bad("validation", "invalid port").field("port", "1–65535")); }
        if server::list(&st).await?.iter().any(|o| o.id != id && o.port == port) { return Err(ApiError::conflict("port.taken", "port already assigned to another Moss server").field("port", "already assigned")); }
    }
    let now = crate::db::now();
    let (i, p2, now2) = (id.clone(), (p.name.clone(), p.memory_mb, p.port, p.flags_profile.clone(), p.autostart, p.restart_policy.as_ref().map(|r| (r.enabled, r.max_per_10m.clamp(0, 20))), p.tags.clone().map(|t| serde_json::to_string(&t.into_iter().map(|x| x.trim().chars().take(32).collect::<String>()).filter(|x| !x.is_empty()).take(12).collect::<Vec<_>>()).unwrap()), p.notes.clone(), p.accept_eula), now);
    st.db.call(move |c| {
        let (name, mem, port, flags, auto, rp, tags, notes, eula) = p2;
        if let Some(v) = name { c.execute("UPDATE servers SET name=? WHERE id=?", rusqlite::params![v.trim(), i])?; }
        if let Some(v) = mem { c.execute("UPDATE servers SET memory_mb=? WHERE id=?", rusqlite::params![v, i])?; }
        if let Some(v) = port { c.execute("UPDATE servers SET port=? WHERE id=?", rusqlite::params![v, i])?; }
        if let Some(v) = flags { c.execute("UPDATE servers SET flags_profile=? WHERE id=?", rusqlite::params![v, i])?; }
        if let Some(v) = auto { c.execute("UPDATE servers SET autostart=? WHERE id=?", rusqlite::params![v, i])?; }
        if let Some((e, m)) = rp { c.execute("UPDATE servers SET restart_enabled=?, restart_max=? WHERE id=?", rusqlite::params![e, m, i])?; }
        if let Some(v) = tags { c.execute("UPDATE servers SET tags=? WHERE id=?", rusqlite::params![v, i])?; }
        if let Some(v) = notes { c.execute("UPDATE servers SET notes=? WHERE id=?", rusqlite::params![v.chars().take(4000).collect::<String>(), i])?; }
        if eula == Some(true) { c.execute("UPDATE servers SET eula_at=COALESCE(eula_at,?), eula_by=COALESCE(eula_by,'owner') WHERE id=?", rusqlite::params![now2, i])?; }
        Ok(())
    }).await?;
    if p.accept_eula == Some(true) && s.eula_at.is_none() {
        st.audit(&user.email, "eula.accept", &s.name, "Minecraft EULA accepted", Some(&id)).await;
        if s.installed { let _ = tokio::fs::write(s.dir(&st).join("eula.txt"), "# Accepted via Moss by the server owner (https://aka.ms/MinecraftEULA)\neula=true\n").await; }
    }
    if let Some(port) = p.port.filter(|port| *port != s.port) { crate::files::patch_port(&st, &s, port).await; }
    st.audit(&user.email, "server.update", &s.name, "settings changed", Some(&id)).await;
    let fresh = server::get(&st, &id).await?;
    let v = server::view(&fresh, &st);
    st.emit("server", v.clone());
    Ok(Json(v))
}

#[derive(Deserialize)] struct DelQ { #[serde(default)] delete_data: bool }
async fn server_delete(State(st): State<St>, user: User, Path(id): Path<String>, Query(q): Query<DelQ>) -> ApiResult<StatusCode> {
    authz(&st, &user, &id, "server.delete").await?;
    let s = server::get(&st, &id).await?;
    if lifecycle::process_alive(&s) { return Err(ApiError::conflict("server.running", "stop the server before deleting it")); }
    let guard = st.lock_for(&id).try_lock_owned().map_err(|_| ApiError::conflict("server.locked", "another operation is changing this server"))?;
    let dir = s.dir(&st);
    if q.delete_data {
        let _ = tokio::fs::remove_dir_all(&dir).await;
    } else if dir.exists() {
        let kept = st.cfg.data_dir.join("removed");
        tokio::fs::create_dir_all(&kept).await?;
        tokio::fs::rename(&dir, kept.join(format!("{}-{}", s.id, s.name.chars().filter(|c| c.is_alphanumeric()).collect::<String>()))).await?;
    }
    let i = id.clone();
    // Backups outlive the server unless its data is deleted too, so a removed server can still be restored as a new one.
    if q.delete_data { let _ = tokio::fs::remove_dir_all(st.cfg.backups_dir().join(&id)).await; }
    let wipe = q.delete_data;
    st.db.call(move |c| {
        c.execute("DELETE FROM servers WHERE id=?", [&i])?;
        c.execute("DELETE FROM timeline WHERE server_id=?", [&i])?;
        if wipe { c.execute("DELETE FROM backups WHERE server_id=?", [&i])?; }
        Ok(())
    }).await?;
    st.live.lock().unwrap().remove(&id);
    st.hubs.lock().unwrap().remove(&id);
    st.stats.lock().unwrap().remove(&id);
    drop(guard);
    st.audit(&user.email, "server.delete", &s.name, if q.delete_data { "data deleted" } else { "data kept in data/removed" }, None).await;
    st.emit("server", json!({"id": id, "deleted": true}));
    Ok(StatusCode::NO_CONTENT)
}

async fn server_action(State(st): State<St>, user: User, Path((id, action)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    authz(&st, &user, &id, "server.power").await?;
    let s = server::get(&st, &id).await?;
    match action.as_str() {
        "start" => lifecycle::start(&st, &s, &user.email).await?,
        "stop" => lifecycle::stop(&st, &s, &user.email).await?,
        "restart" => lifecycle::restart(&st, &s, &user.email).await?,
        "kill" => lifecycle::kill(&st, &s, &user.email).await?,
        _ => return Err(ApiError::bad("action.unknown", "unknown action")),
    }
    let fresh = server::get(&st, &id).await?;
    Ok(Json(json!({"server": server::view(&fresh, &st)})))
}

#[derive(Deserialize)] struct CmdReq { command: String }
async fn server_command(State(st): State<St>, user: User, Path(id): Path<String>, Json(r): Json<CmdReq>) -> ApiResult<StatusCode> {
    authz(&st, &user, &id, "server.console.write").await?;
    let s = server::get(&st, &id).await?;
    lifecycle::send_command(&st, &s, &r.command).await?;
    st.audit(&user.email, "console.command", &s.name, &r.command, Some(&id)).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn server_stats(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    authz(&st, &user, &id, "server.view").await?;
    let v: Vec<_> = st.stats.lock().unwrap().get(&id).map(|q| q.iter().cloned().collect()).unwrap_or_default();
    Ok(Json(json!({"samples": v})))
}

async fn server_timeline(State(st): State<St>, user: User, Path(id): Path<String>, Query(q): Q) -> ApiResult<Json<Value>> {
    authz(&st, &user, &id, "server.view").await?;
    let limit: i64 = q.get("limit").and_then(|v| v.parse().ok()).unwrap_or(100).clamp(1, 500);
    let i = id.clone();
    let mut rows: Vec<Value> = st.db.call(move |c| {
        let mut s = c.prepare("SELECT time,kind,detail FROM timeline WHERE server_id=? ORDER BY id DESC LIMIT ?")?;
        s.query_map(rusqlite::params![i, limit], |r| Ok(json!({"time": r.get::<_, String>(0)?, "kind": r.get::<_, String>(1)?, "detail": r.get::<_, String>(2)?})))?.collect()
    }).await?;
    rows.reverse();
    Ok(Json(Value::Array(rows)))
}

// ---------- operations & activity ----------
async fn operations_list(State(st): State<St>, user: User, Query(q): Q) -> ApiResult<Json<Value>> {
    let limit = q.get("limit").and_then(|v| v.parse().ok()).unwrap_or(50usize).min(200);
    let vis = auth::visible_servers(&st, &user).await?;
    let list: Vec<_> = ops::list(&st, q.get("server_id").map(|s| s.as_str()), 300).into_iter()
        .filter(|o| vis.as_ref().is_none_or(|v| o.server_id.as_ref().is_some_and(|s| v.contains(s)))).take(limit).collect();
    Ok(Json(json!(list)))
}
async fn operation_get(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let op = ops::get(&st, &id).ok_or_else(|| ApiError::not_found("operation"))?;
    match &op.server_id { Some(s) => authz(&st, &user, s, "server.view").await?, None => user.require_admin()? }
    Ok(Json(json!(op)))
}
async fn operation_cancel(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let op = ops::get(&st, &id).ok_or_else(|| ApiError::not_found("operation"))?;
    match &op.server_id { Some(s) => authz(&st, &user, s, "server.power").await?, None => user.require_admin()? }
    if !ops::cancel(&st, &id) { return Err(ApiError::conflict("operation.not_cancellable", "this operation cannot be cancelled right now")); }
    Ok(Json(json!({"ok": true})))
}

async fn activity(State(st): State<St>, user: User, Query(q): Q) -> ApiResult<Json<Value>> {
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

// ---------- events ----------
async fn events(State(st): State<St>, Query(q): Q) -> ApiResult<Response> {
    let user = take_ticket(&st, q.get("ticket"))?;
    let vis = auth::visible_servers(&st, &user).await?;
    let rx = st.bus.subscribe();
    let hello = tokio_stream::once(Ok::<_, std::convert::Infallible>(sse::Event::default().event("hello").data("{}")));
    let stream = BroadcastStream::new(rx).filter_map(move |e| {
        let e = e.ok()?;
        let sid = e.data.get("server_id").and_then(|v| v.as_str()).or_else(|| (e.kind == "server").then(|| e.data["id"].as_str()).flatten()).map(String::from);
        if let Some(v) = &vis && !sid.as_ref().is_some_and(|s| v.contains(s)) { return None; }
        Some(Ok(sse::Event::default().event(e.kind).data(e.data.to_string())))
    });
    Ok(Sse::new(hello.chain(stream)).keep_alive(sse::KeepAlive::new().interval(Duration::from_secs(15))).into_response())
}

// ---------- users & access ----------
async fn users_list(State(st): State<St>, user: User) -> ApiResult<Json<Value>> {
    user.require_admin()?;
    let v: Vec<Value> = st.db.call(|c| {
        let mut s = c.prepare("SELECT id,email,role,linked,created_at FROM users ORDER BY created_at")?;
        s.query_map([], |r| Ok(json!({"id": r.get::<_, String>(0)?, "email": r.get::<_, String>(1)?, "role": r.get::<_, String>(2)?, "linked": r.get::<_, bool>(3)?, "created_at": r.get::<_, String>(4)?})))?.collect()
    }).await?;
    Ok(Json(json!(v)))
}
#[derive(Deserialize)] struct UserReq { email: Option<String>, role: String }
async fn users_create(State(st): State<St>, user: User, Json(r): Json<UserReq>) -> ApiResult<(StatusCode, Json<Value>)> {
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
async fn users_patch(State(st): State<St>, user: User, Path(id): Path<String>, Json(r): Json<UserReq>) -> ApiResult<StatusCode> {
    user.require_admin()?;
    if !["admin", "user"].contains(&r.role.as_str()) || user.role != "owner" { return Err(ApiError::forbidden("only the owner can change roles")); }
    let (i, role) = (id.clone(), r.role.clone());
    let n = st.db.call(move |c| c.execute("UPDATE users SET role=? WHERE id=? AND role<>'owner'", [role, i])).await?;
    if n == 0 { return Err(ApiError::not_found("user")); }
    auth::drop_cached(&st, &id);
    st.audit(&user.email, "user.role", &id, &r.role, None).await;
    Ok(StatusCode::NO_CONTENT)
}
async fn users_delete(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<StatusCode> {
    user.require_admin()?;
    if id == user.id { return Err(ApiError::bad("user.self", "you cannot remove yourself")); }
    let i = id.clone();
    let n = st.db.call(move |c| c.execute("DELETE FROM users WHERE id=? AND role<>'owner' AND (role<>'admin' OR ?='owner')", rusqlite::params![i, user.role.clone()])).await?;
    if n == 0 { return Err(ApiError::forbidden("cannot remove this user")); }
    auth::drop_cached(&st, &id);
    st.audit(&user.email, "user.remove", &id, "", None).await;
    Ok(StatusCode::NO_CONTENT)
}
async fn access_get(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    user.require_admin()?;
    let i = id.clone();
    let v: Vec<Value> = st.db.call(move |c| {
        let mut s = c.prepare("SELECT g.user_id,u.email,g.bundle FROM grants g JOIN users u ON u.id=g.user_id WHERE g.server_id=?")?;
        s.query_map([i], |r| Ok(json!({"user_id": r.get::<_, String>(0)?, "email": r.get::<_, String>(1)?, "bundle": r.get::<_, String>(2)?})))?.collect()
    }).await?;
    Ok(Json(json!(v)))
}
#[derive(Deserialize)] struct Grant { user_id: String, bundle: String }
#[derive(Deserialize)] struct GrantsReq { grants: Vec<Grant> }
async fn access_put(State(st): State<St>, user: User, Path(id): Path<String>, Json(r): Json<GrantsReq>) -> ApiResult<StatusCode> {
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

// ---------- settings ----------
async fn settings_json(st: &Inner) -> Value {
    json!({
        "default_memory_mb": st.setting("default_memory_mb", "2048").await.parse::<i64>().unwrap_or(2048),
        "backup_keep": st.setting("backup_keep", "7").await.parse::<i64>().unwrap_or(7),
        "backup_before_change": st.setting("backup_before_change", "true").await == "true",
        "modrinth_contact": st.setting("modrinth_contact", "").await,
        "public_base_url": st.setting("public_base_url", "").await,
        "data_dir": st.cfg.data_dir.to_string_lossy(), "bind": format!("{}:{}", st.cfg.bind, st.cfg.port), "version": env!("CARGO_PKG_VERSION"),
    })
}
async fn settings_get(State(st): State<St>, _u: User) -> Json<Value> { Json(settings_json(&st).await) }
async fn settings_put(State(st): State<St>, user: User, Json(v): Json<Value>) -> ApiResult<Json<Value>> {
    user.require_admin()?;
    if let Some(n) = v["default_memory_mb"].as_i64() && (256..=262144).contains(&n) { st.set_setting("default_memory_mb", &n.to_string()).await; }
    if let Some(n) = v["backup_keep"].as_i64() { st.set_setting("backup_keep", &n.clamp(1, 365).to_string()).await; }
    if let Some(b) = v["backup_before_change"].as_bool() { st.set_setting("backup_before_change", &b.to_string()).await; }
    if let Some(s) = v["modrinth_contact"].as_str() { st.set_setting("modrinth_contact", &s.chars().take(200).collect::<String>()).await; }
    if let Some(s) = v["public_base_url"].as_str() { st.set_setting("public_base_url", &s.chars().take(200).collect::<String>()).await; }
    st.audit(&user.email, "settings.update", "instance", "", None).await;
    Ok(Json(settings_json(&st).await))
}

pub async fn server_for(st: &St, user: &User, id: &str, perm: &str) -> ApiResult<Srv> {
    authz(st, user, id, perm).await?;
    server::get(st, id).await
}
