//! Server resource handlers: list/create/get/patch/delete, power actions, console commands, stats and timeline.
use super::*;

pub(super) async fn servers_list(State(st): State<St>, user: User) -> ApiResult<Json<Value>> {
    let vis = auth::visible_servers(&st, &user).await?;
    let all = server::list(&st).await?;
    Ok(Json(Value::Array(all.iter().filter(|s| vis.as_ref().is_none_or(|v| v.contains(&s.id))).map(|s| server::view(s, &st)).collect())))
}

#[derive(Deserialize)]
struct CustomCmd { command: Vec<String> }
#[derive(Deserialize)]
pub(super) struct CreateReq {
    name: String, #[serde(default)] runtime: String, #[serde(default)] game_version: String, modpack_token: Option<String>, loader_version: Option<String>, memory_mb: Option<i64>, port: Option<i64>,
    accept_eula: Option<bool>, flags_profile: Option<String>, autostart: Option<bool>, custom: Option<CustomCmd>,
    memory_limit_mb: Option<i64>, cpu_limit_pct: Option<i64>, pids_limit: Option<i64>,
}

pub(super) async fn free_port(_st: &St, preferred: i64, used: &[i64]) -> i64 {
    let mut p = preferred;
    loop {
        if !used.contains(&p) && tokio::net::TcpListener::bind(("0.0.0.0", p as u16)).await.is_ok() { return p; }
        p += 1;
        if p > 65000 { return preferred; }
    }
}

pub(super) async fn servers_create(State(st): State<St>, user: User, Json(mut r): Json<CreateReq>) -> ApiResult<(StatusCode, Json<Value>)> {
    user.require_admin()?;
    if let Some(t) = &r.modpack_token { (r.runtime, r.game_version, r.loader_version) = crate::content::modpack_create_params(&user, t, &r.runtime, &r.game_version, r.loader_version.as_deref())?; }
    let name = r.name.trim().to_string();
    if name.is_empty() || name.chars().count() > 48 || name.chars().any(|c| c.is_control()) { return Err(ApiError::bad("validation", "invalid name").field("name", "1–48 characters, no control characters")); }
    let cat = runtimes::catalogue(st.cfg.dev_runtimes);
    let rt = cat.as_array().unwrap().iter().find(|x| x["id"] == r.runtime.as_str());
    if !rt.is_some_and(|x| x["installable"] == true) { return Err(ApiError::bad("runtime.unsupported", "unknown or not yet installable runtime").field("runtime", "choose an installable runtime")); }
    let is_mc = !matches!(r.runtime.as_str(), "custom" | "sim");
    if is_mc && r.accept_eula != Some(true) { return Err(ApiError::bad("eula.required", "You must accept the Minecraft EULA to create a Minecraft server.").field("accept_eula", "required")); }
    // versions end up in upstream URLs, so only plain version-ish strings are accepted
    let plain = |v: &str| !v.is_empty() && v.len() <= 40 && v.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-'));
    if !plain(&r.game_version) { return Err(ApiError::bad("validation", "invalid game version").field("game_version", "letters, digits and . _ + -")); }
    if r.loader_version.as_deref().is_some_and(|l| !plain(l)) { return Err(ApiError::bad("validation", "invalid loader version").field("loader_version", "letters, digits and . _ + -")); }
    let mem = r.memory_mb.unwrap_or(2048);
    if !(256..=262144).contains(&mem) { return Err(ApiError::bad("validation", "memory out of range").field("memory_mb", "256–262144")); }
    let custom = match (&r.runtime[..], &r.custom) {
        ("custom", Some(c)) if !c.command.is_empty() && c.command.iter().all(|a| !a.is_empty() && a.len() < 2048 && !a.contains('\0')) => Some(serde_json::to_string(&c.command).unwrap()),
        ("custom", _) => return Err(ApiError::bad("validation", "custom runtime needs a non-empty command array").field("custom.command", "required")),
        _ => None,
    };
    let lim = limits::LimitsPatch { memory: Some(r.memory_limit_mb), cpu: Some(r.cpu_limit_pct), pids: Some(r.pids_limit) };
    limits::validate_now(&lim)?;
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
    limits::save(&st, &id, lim).await?;
    let s = server::get(&st, &id).await?;
    tokio::fs::create_dir_all(s.dir(&st)).await?;
    if is_mc { st.audit(&user.actor(), "eula.accept", &name, "Minecraft EULA accepted at server creation", Some(&id)).await; }
    st.audit(&user.actor(), "server.create", &name, &format!("{} {}", s.runtime, s.game_version), Some(&id)).await;
    let op = ops::spawn_op(&st, "server.install", format!("Install {name}"), Some(&id), true, { let (id, tok, u) = (id.clone(), r.modpack_token.clone(), user.clone()); move |ctx| crate::content::install_then_import(ctx, id, tok, u) });
    Ok((StatusCode::ACCEPTED, Json(json!({"server": server::view(&s, &st), "operation": op}))))
}

pub(super) async fn server_get(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    authz(&st, &user, &id, "server.view").await?;
    Ok(Json(server::view(&server::get(&st, &id).await?, &st)))
}

#[derive(Deserialize)]
pub(super) struct PatchReq {
    name: Option<String>, memory_mb: Option<i64>, port: Option<i64>, flags_profile: Option<String>, autostart: Option<bool>,
    restart_policy: Option<RestartPolicy>, tags: Option<Vec<String>>, notes: Option<String>, accept_eula: Option<bool>,
    #[serde(default, deserialize_with = "limits::double_option")] memory_limit_mb: Option<Option<i64>>,
    #[serde(default, deserialize_with = "limits::double_option")] cpu_limit_pct: Option<Option<i64>>,
    #[serde(default, deserialize_with = "limits::double_option")] pids_limit: Option<Option<i64>>,
    isolation: Option<String>,
    #[serde(default, deserialize_with = "limits::double_option")] container_image: Option<Option<String>>,
}
#[derive(Deserialize)] struct RestartPolicy { enabled: bool, max_per_10m: i64 }

pub(super) async fn server_patch(State(st): State<St>, user: User, Path(id): Path<String>, Json(p): Json<PatchReq>) -> ApiResult<Json<Value>> {
    authz(&st, &user, &id, "config.write").await?;
    let s = server::get(&st, &id).await?;
    if let Some(n) = &p.name && (n.trim().is_empty() || n.chars().count() > 48) { return Err(ApiError::bad("validation", "invalid name").field("name", "1–48 characters")); }
    if let Some(m) = p.memory_mb && !(256..=262144).contains(&m) { return Err(ApiError::bad("validation", "memory out of range").field("memory_mb", "256–262144")); }
    let lim = limits::LimitsPatch { memory: p.memory_limit_mb, cpu: p.cpu_limit_pct, pids: p.pids_limit };
    limits::validate_now(&lim)?;
    if let Some(f) = &p.flags_profile && f != "balanced" && f != "minimal" { return Err(ApiError::bad("validation", "unknown flags profile").field("flags_profile", "balanced | minimal")); }
    if let Some(port) = p.port {
        if !(1..=65535).contains(&port) { return Err(ApiError::bad("validation", "invalid port").field("port", "1–65535")); }
        if server::list(&st).await?.iter().any(|o| o.id != id && o.port == port) { return Err(ApiError::conflict("port.taken", "port already assigned to another Moss server").field("port", "already assigned")); }
    }
    let iso = isolation_patch(&st, &user, &s, &p).await?;
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
    limits::save(&st, &id, lim).await?;
    if let Some((mode, image)) = iso { apply_isolation(&st, &user, &s, mode, image).await?; }
    if p.accept_eula == Some(true) && s.eula_at.is_none() {
        st.audit(&user.actor(), "eula.accept", &s.name, "Minecraft EULA accepted", Some(&id)).await;
        if s.installed { let _ = tokio::fs::write(s.dir(&st).join("eula.txt"), "# Accepted via Moss by the server owner (https://aka.ms/MinecraftEULA)\neula=true\n").await; }
    }
    if let Some(port) = p.port.filter(|port| *port != s.port) { crate::files::patch_port(&st, &s, port).await; }
    st.audit(&user.actor(), "server.update", &s.name, "settings changed", Some(&id)).await;
    let fresh = server::get(&st, &id).await?;
    let v = server::view(&fresh, &st);
    st.emit("server", v.clone());
    Ok(Json(v))
}

/// Validate the isolation fields of a PATCH (admin only; engine must exist to enable). Returns the new (mode, image override).
async fn isolation_patch(st: &St, user: &User, s: &Srv, p: &PatchReq) -> ApiResult<Option<(String, Option<String>)>> {
    if p.isolation.is_none() && p.container_image.is_none() { return Ok(None); }
    user.require_admin()?;
    let mode = p.isolation.clone().unwrap_or_else(|| s.isolation.clone());
    if mode != "process" && mode != "container" { return Err(ApiError::bad("validation", "unknown isolation mode").field("isolation", "process | container")); }
    let image = match &p.container_image {
        Some(Some(i)) if !i.trim().is_empty() => Some(crate::container::image::validate(i).map_err(|m| ApiError::bad("validation", m.clone()).field("container_image", &m))?),
        Some(_) => None,
        None => s.container_image.clone(),
    };
    if mode == "container" {
        if !crate::container::supported(s, st.cfg.dev_runtimes) { return Err(ApiError::bad("container.unsupported", "only Minecraft servers can run in a container").field("isolation", "not available for this server type")); }
        if s.isolation != "container" || p.container_image.is_some() {
            crate::container::engine(st).await.map_err(|m| ApiError::conflict("container.engine_missing", m).field("isolation", "no container engine"))?;
        }
    }
    Ok(Some((mode, image)))
}

async fn apply_isolation(st: &St, user: &User, s: &Srv, mode: String, image: Option<String>) -> ApiResult<()> {
    let (i, m, img) = (s.id.clone(), mode.clone(), image.clone());
    st.db.call(move |c| c.execute("UPDATE servers SET isolation=?, container_image=? WHERE id=?", rusqlite::params![m, img, i])).await?;
    if mode != s.isolation || image != s.container_image {
        st.audit(&user.actor(), "server.isolation", &s.name, &format!("{mode}{}", image.as_deref().map(|i| format!(" ({i})")).unwrap_or_default()), Some(&s.id)).await;
        if mode == "container" {
            let fresh = server::get(st, &s.id).await?;
            super::container::spawn_pull(st, &fresh); // download the image now so the next start is quick
        }
    }
    Ok(())
}

#[derive(Deserialize)] pub(super) struct DelQ { #[serde(default)] delete_data: bool }
pub(super) async fn server_delete(State(st): State<St>, user: User, Path(id): Path<String>, Query(q): Query<DelQ>) -> ApiResult<StatusCode> {
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
    lifecycle::cleanup_cgroup(&id);
    if s.isolation == "container" || s.in_container { lifecycle::ctr::remove(&st, &id).await; }
    st.live.lock().unwrap().remove(&id);
    st.hubs.lock().unwrap().remove(&id);
    st.stats.lock().unwrap().remove(&id);
    drop(guard);
    st.audit(&user.actor(), "server.delete", &s.name, if q.delete_data { "data deleted" } else { "data kept in data/removed" }, None).await;
    st.emit("server", json!({"id": id, "deleted": true}));
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn server_action(State(st): State<St>, user: User, Path((id, action)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    authz(&st, &user, &id, "server.power").await?;
    let s = server::get(&st, &id).await?;
    match action.as_str() {
        "start" => lifecycle::start(&st, &s, &user.actor()).await?,
        "stop" => lifecycle::stop(&st, &s, &user.actor()).await?,
        "restart" => lifecycle::restart(&st, &s, &user.actor()).await?,
        "kill" => lifecycle::kill(&st, &s, &user.actor()).await?,
        _ => return Err(ApiError::bad("action.unknown", "unknown action")),
    }
    let fresh = server::get(&st, &id).await?;
    Ok(Json(json!({"server": server::view(&fresh, &st)})))
}

#[derive(Deserialize)] pub(super) struct CmdReq { command: String }
pub(super) async fn server_command(State(st): State<St>, user: User, Path(id): Path<String>, Json(r): Json<CmdReq>) -> ApiResult<StatusCode> {
    authz(&st, &user, &id, "server.console.write").await?;
    let s = server::get(&st, &id).await?;
    lifecycle::send_command(&st, &s, &r.command).await?;
    st.audit_command(&user.actor(), &s.name, &r.command, &id).await;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn server_stats(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    authz(&st, &user, &id, "server.view").await?;
    let v: Vec<_> = st.stats.lock().unwrap().get(&id).map(|q| q.iter().cloned().collect()).unwrap_or_default();
    Ok(Json(json!({"samples": v})))
}

pub(super) async fn server_timeline(State(st): State<St>, user: User, Path(id): Path<String>, Query(q): Q) -> ApiResult<Json<Value>> {
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
