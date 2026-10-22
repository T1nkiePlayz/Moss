//! Server blueprints: a reusable recipe (runtime, version, memory, properties, content list, schedules) that never
//! contains worlds, player data or secrets. Save from a server, export/import as JSON, create a server from one.
use crate::{api::{create_server_from_value, server_for}, auth::User, error::{ApiError, ApiResult}, ops::OpCtx, server::Srv, state::St};
use axum::{Json, extract::{Path, State}, http::StatusCode};
use serde::Deserialize;
use serde_json::{Map, Value, json};

const FORMAT: &str = "moss-blueprint";
pub mod share;

/// Upper bound for a schedule's backup retention count.
const KEEP_MAX: i64 = 365;
const MAX_JSON: usize = 256 * 1024;

/// Property keys that are never saved or applied: network binding, world identity, and anything that looks like a secret.
pub fn property_allowed(k: &str) -> bool {
    let l = k.to_ascii_lowercase();
    !(matches!(l.as_str(), "server-port" | "server-ip" | "query.port" | "rcon.port" | "level-name" | "level-seed" | "level-type" | "generator-settings")
        || ["password", "secret", "token", "key", "salt"].iter().any(|w| l.contains(w)))
}

fn plain(v: &str, max: usize) -> bool { !v.is_empty() && v.len() <= max && v.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-')) }

/// Checks an untrusted blueprint document (import) and returns a cleaned copy containing only known fields.
pub fn sanitize(v: &Value) -> ApiResult<Value> {
    let bad = |f: &str, m: &str| ApiError::bad("blueprint.invalid", format!("This is not a valid Moss blueprint: {m}")).field(f, m);
    let o = v.as_object().ok_or_else(|| bad("format", "expected a JSON object"))?;
    if o.get("format").and_then(Value::as_str) != Some(FORMAT) { return Err(bad("format", "missing the moss-blueprint marker")); }
    if o.get("version").and_then(Value::as_i64) != Some(1) { return Err(bad("version", "unsupported blueprint version")); }
    let name = o.get("name").and_then(Value::as_str).unwrap_or_default().trim().to_string();
    if name.is_empty() || name.chars().count() > 48 || name.chars().any(|c| c.is_control()) { return Err(bad("name", "name must be 1–48 characters")); }
    let runtime = o.get("runtime").and_then(Value::as_str).unwrap_or_default();
    if !plain(runtime, 32) || runtime == "custom" { return Err(bad("runtime", "unsupported runtime")); }
    let gv = o.get("game_version").and_then(Value::as_str).unwrap_or_default();
    if !plain(gv, 40) { return Err(bad("game_version", "invalid game version")); }
    let lv = match o.get("loader_version") { None | Some(Value::Null) => Value::Null, Some(Value::String(s)) if plain(s, 40) => json!(s), _ => return Err(bad("loader_version", "invalid loader version")) };
    let mem = o.get("memory_mb").and_then(Value::as_i64).unwrap_or(2048);
    if !(256..=262144).contains(&mem) { return Err(bad("memory_mb", "memory out of range")); }
    if let Some(msg) = crate::platform::memory_over(mem, crate::platform::host_memory_mb()) { return Err(bad("memory_mb", &msg)); }
    let flags = o.get("flags_profile").and_then(Value::as_str).filter(|f| matches!(*f, "balanced" | "minimal")).unwrap_or("balanced");
    let mut props = Map::new();
    if let Some(p) = o.get("properties") {
        for (k, val) in p.as_object().ok_or_else(|| bad("properties", "expected an object"))? {
            let s = match val { Value::String(s) => s.clone(), Value::Bool(b) => b.to_string(), Value::Number(n) => n.to_string(), _ => return Err(bad("properties", "values must be text, numbers or true/false")) };
            if k.is_empty() || k.len() > 64 || !k.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')) { return Err(bad("properties", "invalid property name")); }
            if s.len() > 1000 || s.contains(['\n', '\r', '\0']) { return Err(bad("properties", "invalid property value")); }
            if property_allowed(k) { props.insert(k.clone(), json!(s)); }
        }
    }
    let mut content = vec![];
    for it in o.get("content").and_then(Value::as_array).map(|a| a.as_slice()).unwrap_or_default() {
        let (prov, pid) = (it["provider"].as_str().unwrap_or("modrinth"), it["project_id"].as_str().unwrap_or_default());
        if !matches!(prov, "modrinth" | "curseforge" | "hangar") || !plain(pid, 40) { return Err(bad("content", "invalid content entry")); }
        content.push(json!({"provider": prov, "project_id": pid, "title": it["title"].as_str().unwrap_or(pid).chars().take(80).filter(|c| !c.is_control()).collect::<String>(), "kind": it["kind"].as_str().unwrap_or("").chars().take(24).collect::<String>()}));
        if content.len() > 200 { return Err(bad("content", "at most 200 content items")); }
    }
    let mut scheds = vec![];
    for sc in o.get("schedules").and_then(Value::as_array).map(|a| a.as_slice()).unwrap_or_default() {
        let kind = sc["kind"].as_str().unwrap_or_default();
        if !["backup", "restart", "start", "stop", "command"].contains(&kind) { return Err(bad("schedules", "unknown schedule kind")); }
        let mut m = Map::new();
        m.insert("kind".into(), json!(kind));
        for f in ["command", "daily_at", "tz"] { if let Some(s) = sc[f].as_str() { if s.len() > 200 || s.contains(['\n', '\r']) { return Err(bad("schedules", "invalid schedule text")); } m.insert(f.into(), json!(s)); } }
        if let Some(n) = sc["every_minutes"].as_i64() { m.insert("every_minutes".into(), json!(n.clamp(5, 525_600))); }
        if let Some(n) = sc["keep"].as_i64() { m.insert("keep".into(), json!(n.clamp(1, KEEP_MAX))); }
        m.insert("enabled".into(), json!(sc["enabled"].as_bool().unwrap_or(true)));
        scheds.push(Value::Object(m));
        if scheds.len() > 20 { return Err(bad("schedules", "at most 20 schedules")); }
    }
    Ok(json!({"format": FORMAT, "version": 1, "name": name, "runtime": runtime, "game_version": gv, "loader_version": lv, "memory_mb": mem, "flags_profile": flags, "properties": props, "content": content, "schedules": scheds}))
}

fn parse_props(raw: &str) -> Map<String, Value> {
    raw.lines().filter(|l| !l.trim_start().starts_with('#')).filter_map(|l| l.split_once('=')).filter(|(k, _)| property_allowed(k.trim())).map(|(k, v)| (k.trim().to_string(), json!(v.trim()))).collect()
}

/// Merges `props` into server.properties text: existing keys are replaced in place, new keys appended.
pub fn merge_properties(raw: &str, props: &Map<String, Value>) -> String {
    let mut done = std::collections::HashSet::new();
    let mut out: Vec<String> = raw.lines().map(|l| {
        if let Some((k, _)) = l.split_once('=') && !l.trim_start().starts_with('#') && let Some(v) = props.get(k.trim()) { done.insert(k.trim().to_string()); return format!("{}={}", k.trim(), v.as_str().unwrap_or_default()); }
        l.to_string()
    }).collect();
    for (k, v) in props { if !done.contains(k) { out.push(format!("{k}={}", v.as_str().unwrap_or_default())); } }
    out.join("\n") + "\n"
}

async fn capture(st: &St, s: &Srv, name: &str) -> ApiResult<Value> {
    if s.runtime == "custom" || s.runtime == "sim" { return Err(ApiError::bad("blueprint.unsupported", "Servers with a custom start command cannot be saved as blueprints.")); }
    let props = tokio::fs::read_to_string(s.dir(st).join("server.properties")).await.map(|r| parse_props(&r)).unwrap_or_default();
    let sid = s.id.clone();
    let (content, scheds): (Vec<Value>, Vec<Value>) = st.db.call(move |c| {
        let mut q = c.prepare("SELECT provider, project_id, title, kind FROM content_items WHERE server_id=? AND project_id IS NOT NULL AND provider IN ('modrinth','curseforge','hangar') ORDER BY title")?;
        let content = q.query_map([&sid], |r| Ok(json!({"provider": r.get::<_, String>(0)?, "project_id": r.get::<_, String>(1)?, "title": r.get::<_, String>(2)?, "kind": r.get::<_, String>(3)?})))?.collect::<Result<Vec<_>, _>>()?;
        let mut q = c.prepare("SELECT kind, command, every_minutes, daily_at, tz, keep, enabled FROM schedules WHERE server_id=? ORDER BY kind, id")?;
        let scheds = q.query_map([&sid], |r| Ok(json!({"kind": r.get::<_, String>(0)?, "command": r.get::<_, Option<String>>(1)?, "every_minutes": r.get::<_, Option<i64>>(2)?, "daily_at": r.get::<_, Option<String>>(3)?, "tz": r.get::<_, String>(4)?, "keep": r.get::<_, Option<i64>>(5)?, "enabled": r.get::<_, bool>(6)?})))?.collect::<Result<Vec<_>, _>>()?;
        Ok((content, scheds))
    }).await?;
    let scheds: Vec<Value> = scheds.into_iter().map(|v| Value::Object(v.as_object().unwrap().iter().filter(|(_, x)| !x.is_null()).map(|(k, x)| (k.clone(), x.clone())).collect())).collect();
    sanitize(&json!({"format": FORMAT, "version": 1, "name": name, "runtime": s.runtime, "game_version": s.game_version, "loader_version": s.loader_version,
        "memory_mb": s.memory_mb, "flags_profile": s.flags_profile, "properties": props, "content": content, "schedules": scheds}))
}

async fn store(st: &St, user: &User, bp: &Value) -> ApiResult<Value> {
    let (id, now) = (uuid::Uuid::now_v7().to_string(), crate::db::now());
    let (i, name, data, by, n) = (id.clone(), bp["name"].as_str().unwrap().to_string(), serde_json::to_string(bp).unwrap(), user.email.clone(), now.clone());
    if data.len() > MAX_JSON { return Err(ApiError::bad("blueprint.invalid", "This blueprint is too large.")); }
    st.db.call(move |c| c.execute("INSERT INTO blueprints(id,name,data,created_by,created_at) VALUES (?,?,?,?,?)", rusqlite::params![i, name, data, by, n])).await?;
    Ok(summary(&id, bp, &user.email, &now))
}

fn summary(id: &str, bp: &Value, by: &str, at: &str) -> Value {
    json!({"id": id, "name": bp["name"], "runtime": bp["runtime"], "game_version": bp["game_version"], "loader_version": bp["loader_version"], "memory_mb": bp["memory_mb"],
        "properties": bp["properties"].as_object().map_or(0, |m| m.len()), "content": bp["content"].as_array().map_or(0, |a| a.len()), "schedules": bp["schedules"].as_array().map_or(0, |a| a.len()), "created_by": by, "created_at": at})
}

async fn load(st: &St, id: &str) -> ApiResult<(Value, String, String)> {
    let i = id.to_string();
    let row: Option<(String, String, String)> = st.db.call(move |c| match c.query_row("SELECT data, created_by, created_at FROM blueprints WHERE id=?", [i], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))) {
        Ok(v) => Ok(Some(v)), Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None), Err(e) => Err(e) }).await?;
    let (d, by, at) = row.ok_or_else(|| ApiError::not_found("blueprint"))?;
    Ok((serde_json::from_str(&d).map_err(|e| ApiError::bad("blueprint.invalid", e.to_string()))?, by, at))
}

pub async fn list(State(st): State<St>, user: User) -> ApiResult<Json<Value>> {
    user.require_admin()?;
    let rows: Vec<(String, String, String, String)> = st.db.call(|c| { let mut q = c.prepare("SELECT id, data, created_by, created_at FROM blueprints ORDER BY created_at DESC")?; q.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect() }).await?;
    Ok(Json(Value::Array(rows.iter().filter_map(|(id, d, by, at)| serde_json::from_str::<Value>(d).ok().map(|bp| summary(id, &bp, by, at))).collect())))
}

#[derive(Deserialize)]
pub struct SaveReq { server_id: String, name: String }

pub async fn save(State(st): State<St>, user: User, Json(r): Json<SaveReq>) -> ApiResult<(StatusCode, Json<Value>)> {
    user.require_admin()?;
    let s = server_for(&st, &user, &r.server_id, "server.view").await?;
    let bp = capture(&st, &s, r.name.trim()).await?;
    let v = store(&st, &user, &bp).await?;
    st.audit(&user.actor(), "blueprint.save", bp["name"].as_str().unwrap_or(""), &format!("from {}", s.name), Some(&s.id)).await;
    Ok((StatusCode::CREATED, Json(v)))
}

pub async fn export(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    user.require_admin()?;
    Ok(Json(load(&st, &id).await?.0))
}

pub async fn import(State(st): State<St>, user: User, body: axum::body::Bytes) -> ApiResult<(StatusCode, Json<Value>)> {
    user.require_admin()?;
    if body.len() > MAX_JSON { return Err(ApiError::bad("blueprint.invalid", "This file is too large to be a blueprint.")); }
    let v: Value = serde_json::from_slice(&body).map_err(|_| ApiError::bad("blueprint.invalid", "This file is not valid JSON."))?;
    let bp = sanitize(&v)?;
    let out = store(&st, &user, &bp).await?;
    st.audit(&user.actor(), "blueprint.import", bp["name"].as_str().unwrap_or(""), "imported from JSON", None).await;
    Ok((StatusCode::CREATED, Json(out)))
}

pub async fn delete(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<StatusCode> {
    user.require_admin()?;
    let (bp, ..) = load(&st, &id).await?;
    let i = id.clone();
    st.db.call(move |c| c.execute("DELETE FROM blueprints WHERE id=?", [i])).await?;
    st.audit(&user.actor(), "blueprint.delete", bp["name"].as_str().unwrap_or(""), "", None).await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct CreateReq { name: String, port: Option<i64>, accept_eula: Option<bool> }

pub async fn create(State(st): State<St>, user: User, Path(id): Path<String>, Json(r): Json<CreateReq>) -> ApiResult<(StatusCode, Json<Value>)> {
    user.require_admin()?;
    let (bp, ..) = load(&st, &id).await?;
    let bp = sanitize(&bp)?;
    let mut req = json!({"name": r.name, "runtime": bp["runtime"], "game_version": bp["game_version"], "loader_version": bp["loader_version"], "memory_mb": bp["memory_mb"], "flags_profile": bp["flags_profile"], "accept_eula": r.accept_eula, "autostart": false});
    if let Some(p) = r.port { req["port"] = json!(p); }
    let (code, Json(mut out)) = create_server_from_value(st.clone(), user.clone(), req, Some(bp.clone())).await?;
    let sid = out["server"]["id"].as_str().unwrap_or_default().to_string();
    let mut skipped = vec![];
    for sc in bp["schedules"].as_array().map(|a| a.as_slice()).unwrap_or_default() {
        let Ok(req) = serde_json::from_value::<crate::sched::Req>(sc.clone()) else { continue };
        if crate::sched::create(State(st.clone()), user.clone(), Path(sid.clone()), Json(req)).await.is_err() { skipped.push(sc["kind"].as_str().unwrap_or("schedule").to_string()); }
    }
    out["blueprint"] = json!({"id": id, "skipped_schedules": skipped});
    Ok((code, Json(out)))
}

/// Runs inside the install operation, after the server files exist: writes the saved properties and installs the content.
pub async fn apply_after_install(ctx: OpCtx, id: String, bp: Value, user: User) -> anyhow::Result<()> {
    let st = ctx.st.clone();
    let s = crate::server::get(&st, &id).await.map_err(|e| anyhow::anyhow!(e.message))?;
    if let Some(p) = bp["properties"].as_object().filter(|p| !p.is_empty()) && s.capabilities().contains(&"properties") {
        ctx.stage("Applying saved settings");
        let path = s.dir(&st).join("server.properties");
        let raw = tokio::fs::read_to_string(&path).await.unwrap_or_default();
        let p: Map<String, Value> = p.iter().filter(|(k, _)| property_allowed(k)).map(|(k, v)| (k.clone(), v.clone())).collect();
        tokio::fs::write(&path, merge_properties(&raw, &p)).await?;
    }
    let items: Vec<(String, String)> = bp["content"].as_array().map(|a| a.iter().filter_map(|c| Some((c["provider"].as_str()?.to_string(), c["project_id"].as_str()?.to_string()))).collect()).unwrap_or_default();
    if !items.is_empty() {
        match crate::content::install_many(ctx.clone(), &id, &items, &user).await {
            Ok(n) => ctx.log(format!("Installed {n} content item(s) from the blueprint")),
            Err(e) => ctx.log(format!("Some blueprint content could not be installed: {e}")),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn doc() -> Value { json!({"format": FORMAT, "version": 1, "name": "Friends SMP", "runtime": "paper", "game_version": "1.21.4", "memory_mb": 4096,
        "properties": {"motd": "hi", "server-port": "1", "rcon.password": "x", "level-seed": "42", "view-distance": 8, "pvp": false},
        "content": [{"provider": "modrinth", "project_id": "AABBCC", "title": "Chunky"}], "schedules": [{"kind": "backup", "daily_at": "04:00", "tz": "UTC", "junk": 1}]}) }
    #[test]
    fn sanitize_drops_secrets_world_and_network_keys() {
        let b = sanitize(&doc()).unwrap();
        let keys: Vec<&String> = b["properties"].as_object().unwrap().keys().collect();
        assert_eq!(keys.len(), 3, "{keys:?}");
        assert_eq!(b["properties"]["view-distance"], "8");
        assert!(b["schedules"][0].get("junk").is_none());
    }
    #[test]
    fn sanitize_rejects_bad_documents() {
        for (k, v) in [("format", json!("x")), ("runtime", json!("custom")), ("game_version", json!("1.21;rm")), ("memory_mb", json!(1)), ("name", json!(""))] {
            let mut d = doc(); d[k] = v;
            assert!(sanitize(&d).is_err(), "{k}");
        }
        let mut d = doc(); d["content"][0]["project_id"] = json!("../x");
        assert!(sanitize(&d).is_err());
        let mut d = doc(); d["properties"]["motd"] = json!("a\nb=c");
        assert!(sanitize(&d).is_err(), "newline injection");
        assert!(sanitize(&json!([])).is_err());
    }
    #[test]
    fn merge_replaces_and_appends() {
        let p = parse_props("#c\nmotd=old\nserver-port=1\nrcon.password=s\npvp=true\n");
        assert_eq!(p.len(), 2);
        let mut m = Map::new(); m.insert("motd".into(), json!("new")); m.insert("view-distance".into(), json!("8"));
        assert_eq!(merge_properties("#c\nmotd=old\nserver-port=25565\n", &m), "#c\nmotd=new\nserver-port=25565\nview-distance=8\n");
    }
}
