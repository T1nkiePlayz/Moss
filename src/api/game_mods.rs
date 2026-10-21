//! Mod management for Valheim, Factorio, Garry's Mod and CS2 (`/servers/{id}/game-mods`). Reading needs
//! `server.view`, changing mods `content.manage`, and the Factorio credentials `config.write`. Changes need a stopped server.
use super::{Q, server_for};
use crate::{
    auth::User, error::{ApiError, ApiResult}, games::{self, mods::{self, Kind, Providers, service, workshop}}, lifecycle, ops, server::Srv, state::St,
};
use axum::{Json, extract::{Path, Query, State}, http::StatusCode};
use serde::Deserialize;
use serde_json::{Value, json};

async fn target(st: &St, user: &User, id: &str, perm: &str) -> ApiResult<(Srv, Kind)> {
    let s = server_for(st, user, id, perm).await?;
    let kind = Kind::of(&s.runtime).ok_or_else(|| ApiError::bad("capability.unsupported", "this runtime has no game mods"))?;
    Ok((s, kind))
}

fn ensure_stopped(st: &St, s: &Srv) -> ApiResult<()> {
    if st.live.lock().unwrap().get(&s.id).is_some_and(|l| l.alive) || lifecycle::process_alive(s) {
        return Err(ApiError::conflict("server.running", "stop the server before changing its mods"));
    }
    Ok(())
}

fn ensure_installed(s: &Srv) -> ApiResult<()> {
    if s.installed { Ok(()) } else { Err(ApiError::conflict("server.not_installed", "the server is not installed yet")) }
}

pub async fn list(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let (s, kind) = target(&st, &user, &id, "server.view").await?;
    let dir = s.dir(&st);
    let cfg = games::read_settings(&dir);
    let mut v = json!({"kind": kind.as_str(), "runtime": s.runtime, "installed": [], "running": ensure_stopped(&st, &s).is_err()});
    match kind {
        Kind::Workshop => {
            let key = if s.runtime == "gmod" { workshop::COLLECTION_KEY } else { workshop::MAP_KEY };
            let cur = cfg.get(key).filter(|c| workshop::valid_id(c));
            v["workshop"] = json!({"field": if s.runtime == "gmod" { "collection" } else { "map" }, "id": cur, "url": cur.map(|c| workshop::page_url(c)), "has_api_key": cfg.get(workshop::API_KEY).is_some_and(|k| !k.is_empty())});
        }
        _ => {
            v["installed"] = Value::Array(service::installed_json(&dir, kind)?);
            if kind == Kind::Factorio {
                v["credentials"] = json!({"username": cfg.get("factorio_username").filter(|u| !u.is_empty()), "has_token": cfg.get("factorio_token").is_some_and(|t| !t.is_empty())});
            }
            if kind == Kind::Thunderstore { v["bepinex"] = json!(mods::Manifest::load(&dir)?.get(mods::thunderstore::BEPINEX_ID).is_some()); }
        }
    }
    Ok(Json(v))
}

pub async fn search(State(st): State<St>, user: User, Path(id): Path<String>, Query(q): Q) -> ApiResult<Json<Value>> {
    let (_, kind) = target(&st, &user, &id, "server.view").await?;
    if kind == Kind::Workshop { return Err(ApiError::bad("capability.unsupported", "Steam Workshop content is chosen by id; browse the Steam page")); }
    let text = q.get("q").map(|q| q.trim()).unwrap_or_default();
    if text.len() > 100 { return Err(ApiError::bad("validation", "search text is too long")); }
    let results = service::search(kind, &Providers::default(), text).await.map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, "provider.unavailable", format!("{e:#}")))?;
    Ok(Json(json!({"results": results})))
}

pub async fn updates(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let (s, kind) = target(&st, &user, &id, "server.view").await?;
    Ok(Json(json!({"updates": service::check_updates(&Providers::default(), &s.dir(&st), kind).await?})))
}

#[derive(Deserialize)]
pub struct InstallReq { pub id: String, pub version: Option<String> }

pub async fn install(State(st): State<St>, user: User, Path(id): Path<String>, Json(r): Json<InstallReq>) -> ApiResult<(StatusCode, Json<Value>)> {
    let (s, kind) = target(&st, &user, &id, "content.manage").await?;
    if kind == Kind::Workshop { return Err(ApiError::bad("capability.unsupported", "Steam Workshop content is set by id, not installed")); }
    if !service::valid_id(kind, &r.id) { return Err(ApiError::bad("validation", "invalid mod id").field("id", "not a valid mod id for this game")); }
    if r.version.as_deref().is_some_and(|v| v.is_empty() || v.len() > 32 || !v.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')) { return Err(ApiError::bad("validation", "invalid version").field("version", "invalid version")); }
    ensure_installed(&s)?;
    ensure_stopped(&st, &s)?;
    let dir = s.dir(&st);
    if kind == Kind::Factorio && service::factorio_credentials(&dir).is_none() {
        return Err(ApiError::conflict("game_mods.credentials", "Factorio downloads need your mod portal username and token (factorio.com/profile); add them in the Mods tab"));
    }
    let (tmp, actor, sid) = (st.cfg.tmp_dir().join(format!("gamemods-{}", uuid::Uuid::now_v7())), user.actor(), s.id.clone());
    let title = format!("Install {} on {}", r.id, s.name);
    let op = ops::spawn_op(&st, "gamemods.install", title, Some(&id), true, move |ctx| async move {
        let _guard = ctx.st.lock_for(&sid).try_lock_owned().map_err(|_| anyhow::anyhow!("another operation is changing this server"))?;
        let res = service::install(&ctx, &Providers::default(), &dir, &tmp, kind, &r.id, r.version.as_deref()).await;
        let _ = tokio::fs::remove_dir_all(&tmp).await;
        let done = res?;
        ctx.st.audit(&actor, "gamemods.install", &r.id, &done.join(", "), Some(&sid)).await;
        Ok(())
    });
    Ok((StatusCode::ACCEPTED, Json(json!({"operation": op}))))
}

fn busy(st: &St, s: &Srv) -> ApiResult<tokio::sync::OwnedMutexGuard<()>> {
    st.lock_for(&s.id).try_lock_owned().map_err(|_| ApiError::conflict("server.busy", "another operation is changing this server"))
}

#[derive(Deserialize)]
pub struct PatchReq { enabled: bool }

pub async fn patch(State(st): State<St>, user: User, Path((id, m)): Path<(String, String)>, Json(r): Json<PatchReq>) -> ApiResult<Json<Value>> {
    let (s, kind) = target(&st, &user, &id, "content.manage").await?;
    if kind == Kind::Workshop { return Err(ApiError::bad("capability.unsupported", "no installed mods to toggle")); }
    ensure_stopped(&st, &s)?;
    let _g = busy(&st, &s)?;
    service::set_enabled(&s.dir(&st), kind, &m, r.enabled)?;
    st.audit(&user.actor(), if r.enabled { "gamemods.enable" } else { "gamemods.disable" }, &m, "", Some(&s.id)).await;
    Ok(Json(json!({"ok": true})))
}

pub async fn delete(State(st): State<St>, user: User, Path((id, m)): Path<(String, String)>) -> ApiResult<StatusCode> {
    let (s, kind) = target(&st, &user, &id, "content.manage").await?;
    if kind == Kind::Workshop { return Err(ApiError::bad("capability.unsupported", "no installed mods to remove")); }
    ensure_stopped(&st, &s)?;
    let _g = busy(&st, &s)?;
    service::remove(&s.dir(&st), kind, &m)?;
    st.audit(&user.actor(), "gamemods.remove", &m, "", Some(&s.id)).await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct WorkshopReq { pub collection: Option<String>, pub map: Option<String> }

/// Sets (or, with an empty value, clears) the Workshop collection (Garry's Mod) or map (CS2) the server mounts.
pub async fn workshop_put(State(st): State<St>, user: User, Path(id): Path<String>, Json(r): Json<WorkshopReq>) -> ApiResult<Json<Value>> {
    let (s, kind) = target(&st, &user, &id, "content.manage").await?;
    let (field, key, val) = match (s.runtime.as_str(), r.collection, r.map) {
        ("gmod", Some(c), None) => ("collection", workshop::COLLECTION_KEY, c),
        ("cs2", None, Some(m)) => ("map", workshop::MAP_KEY, m),
        _ if kind == Kind::Workshop => return Err(ApiError::bad("validation", "send `collection` for Garry's Mod or `map` for CS2")),
        _ => return Err(ApiError::bad("capability.unsupported", "this runtime has no Steam Workshop")),
    };
    let val = val.trim();
    if !val.is_empty() && !workshop::valid_id(val) { return Err(ApiError::bad("validation", "Workshop ids are numbers").field(field, "must be digits only")); }
    ensure_stopped(&st, &s)?;
    let _g = busy(&st, &s)?;
    mods::set_settings(&s.dir(&st), &[(key, (!val.is_empty()).then_some(val))])?;
    st.audit(&user.actor(), "gamemods.workshop", key, val, Some(&s.id)).await;
    Ok(Json(json!({"ok": true, "id": (!val.is_empty()).then_some(val)})))
}

#[derive(Deserialize)]
pub struct CredsReq { pub username: String, pub token: Option<String> }

/// Stores the Factorio mod portal username and token in moss-game.cfg. The token is never returned or logged.
pub async fn credentials_put(State(st): State<St>, user: User, Path(id): Path<String>, Json(r): Json<CredsReq>) -> ApiResult<Json<Value>> {
    let (s, kind) = target(&st, &user, &id, "config.write").await?;
    if kind != Kind::Factorio { return Err(ApiError::bad("capability.unsupported", "only Factorio needs mod portal credentials")); }
    let ok = |v: &str| v.len() <= 200 && v.chars().all(|c| c.is_ascii_graphic());
    let (name, token) = (r.username.trim(), r.token.as_deref().map(str::trim));
    if !ok(name) || token.is_some_and(|t| !ok(t)) { return Err(ApiError::bad("validation", "credentials contain invalid characters")); }
    let _g = busy(&st, &s)?;
    let dir = s.dir(&st);
    if name.is_empty() {
        mods::set_settings(&dir, &[("factorio_username", None), ("factorio_token", None)])?;
    } else {
        mods::set_settings(&dir, &[("factorio_username", Some(name))])?;
        if let Some(t) = token.filter(|t| !t.is_empty()) { mods::set_settings(&dir, &[("factorio_token", Some(t))])?; }
    }
    st.audit(&user.actor(), "gamemods.credentials", "factorio", "", Some(&s.id)).await;
    Ok(Json(json!({"ok": true})))
}
