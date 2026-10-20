//! HTTP API: the router (every route is listed here) plus helpers shared by the handler modules and other modules
//! (`server_for`, `take_ticket`). Handlers live in `session`, `servers`, `activity`, `access` and `settings`;
//! the embedded web UI in `assets`.
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

mod access;
mod activity;
mod assets;
mod servers;
mod session;
mod settings;

use access::*; use activity::*; use assets::static_files; use servers::*; use session::*; use settings::*;

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
        .route("/content/providers", get(content::providers))
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

pub async fn server_for(st: &St, user: &User, id: &str, perm: &str) -> ApiResult<Srv> {
    authz(st, user, id, perm).await?;
    server::get(st, id).await
}
