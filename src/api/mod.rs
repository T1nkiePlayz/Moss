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
mod java;
mod limits;
mod metrics;
mod notifications;
mod reinstall;
mod servers;
mod session;
mod settings;
mod setup;
mod support;
mod tokens;

use access::*; use activity::*; use assets::static_files; use java::*; use reinstall::*; use servers::*; use session::*; use settings::*; use setup::*; use tokens::*;
use {metrics::*, notifications::*, support::*};
pub use servers::{create_server_from_value, free_port_for};

type Q = Query<HashMap<String, String>>;

pub fn take_ticket(st: &St, t: Option<&String>) -> ApiResult<User> {
    let t = t.ok_or_else(|| ApiError::unauthorized("missing ticket"))?;
    let mut m = st.tickets.lock().unwrap();
    m.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(30));
    m.remove(t).map(|(_, u)| u).ok_or_else(|| ApiError::unauthorized("invalid or expired ticket"))
}

pub fn router(st: St) -> Router {
    use crate::{backups, blueprints, cfgfiles, console, content, files, health, importer, sched, search, undo, userdata};
    let api = Router::new()
        .route("/auth/config", get(auth_config))
        .route("/me", get(me))
        .route("/me/preferences", get(userdata::prefs_get).put(userdata::prefs_put))
        .route("/views", get(userdata::views_list).post(userdata::views_create))
        .route("/views/{id}", axum::routing::delete(userdata::views_delete))
        .route("/blueprints", get(blueprints::list).post(blueprints::save))
        .route("/blueprints/import", post(blueprints::import))
        .route("/blueprints/{id}", axum::routing::delete(blueprints::delete))
        .route("/blueprints/{id}/export", get(blueprints::export))
        .route("/blueprints/{id}/create", post(blueprints::create))
        .route("/search", get(search::search))
        .route("/ticket", post(ticket))
        .route("/status", get(status))
        .route("/runtimes", get(runtimes_list))
        .route("/runtimes/{id}/versions", get(runtime_versions))
        .route("/java", get(java_list))
        .route("/java/install", post(java_install))
        .route("/java/{major}", axum::routing::delete(java_delete))
        .route("/servers", get(servers_list).post(servers_create))
        .route("/servers/import/inspect", post(importer::inspect))
        .route("/servers/import", post(importer::start))
        .route("/servers/{id}", get(server_get).patch(server_patch).delete(server_delete))
        .route("/servers/{id}/actions/reinstall", post(server_reinstall))
        .route("/servers/{id}/actions/{action}", post(server_action))
        .route("/servers/{id}/command", post(server_command))
        .route("/servers/{id}/stats", get(server_stats))
        .route("/servers/{id}/timeline", get(server_timeline))
        .route("/servers/{id}/health", get(health::server_health))
        .route("/servers/{id}/properties", get(files::props_get).put(files::props_put))
        .route("/servers/{id}/config", get(cfgfiles::list))
        .route("/servers/{id}/config/{file}", get(cfgfiles::get).put(cfgfiles::put))
        .route("/servers/{id}/config/{file}/diff", get(cfgfiles::diff))
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
        .route("/servers/{id}/content/identify", post(content::identify))
        .route("/servers/{id}/content/check", get(content::check_get))
        .route("/servers/{id}/content/update-all/plan", post(content::update_all_plan))
        .route("/servers/{id}/content/update-all", post(content::update_all))
        .route("/servers/{id}/content/{item}/toggle", post(content::toggle))
        .route("/modpacks/inspect", post(content::modpack_inspect))
        .route("/servers/{id}/modpacks", post(content::modpack_import))
        .route("/servers/{id}/modpacks/export", get(content::modpack_export))
        .route("/servers/{id}/upgrade/plan", post(content::upgrade_plan))
        .route("/servers/{id}/upgrade", post(content::upgrade_start))
        .route("/servers/{id}/content/{item}", patch(content::patch_item).delete(content::remove))
        .route("/servers/{id}/content/{item}/update", post(content::update))
        .route("/backups", get(backups::list_all))
        .route("/servers/{id}/backups", get(backups::list).post(backups::create))
        .route("/backups/{id}", patch(backups::patch).delete(backups::delete))
        .route("/backups/{id}/preview", post(backups::preview))
        .route("/backups/{id}/verify", post(backups::verify))
        .route("/backups/{id}/restore", post(backups::restore))
        .route("/backups/{id}/download", get(backups::download))
        .route("/servers/{id}/schedules", get(sched::list).post(sched::create))
        .route("/timezones", get(sched::timezones))
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
        .route("/notifications", get(notifications_list))
        .route("/notifications/read", post(notifications_read))
        .route("/notifications/test", post(notifications_test))
        .route("/servers/{id}/metrics", get(server_metrics))
        .route("/metrics/host", get(host_metrics))
        .route("/support-bundle", get(support_bundle))
        .route("/setup/claim", post(setup_claim))
        .route("/tokens", get(tokens_list).post(tokens_create))
        .route("/tokens/{id}", axum::routing::delete(tokens_delete))
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
