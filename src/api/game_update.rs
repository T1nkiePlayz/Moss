//! `GET/POST /servers/{id}/game-update`: is a newer version of a game server's software available, and update it.
use super::*;
use crate::games::updates;

#[derive(Deserialize)]
pub(super) struct StatusQuery {
    /// Skip the one-hour cache of upstream lookups.
    #[serde(default)]
    refresh: bool,
}

fn game_only(s: &Srv) -> ApiResult<()> {
    if s.is_game() { Ok(()) } else { Err(ApiError::bad("game_update.not_a_game", "only game servers can be updated this way")) }
}

pub(super) async fn game_update_status(State(st): State<St>, user: User, Path(id): Path<String>, Query(q): Query<StatusQuery>) -> ApiResult<Json<Value>> {
    let s = server_for(&st, &user, &id, "server.view").await?;
    game_only(&s)?;
    let status = updates::check(&st, &s, q.refresh).await
        .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, "game_update.lookup_failed", format!("could not look up the latest version: {e:#}")))?;
    Ok(Json(json!(status)))
}

pub(super) async fn game_update_start(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<(StatusCode, Json<Value>)> {
    let s = server_for(&st, &user, &id, "config.write").await?;
    game_only(&s)?;
    match updates::begin(&st, &s, &user.actor()).await? {
        Some(op) => Ok((StatusCode::ACCEPTED, Json(json!({"operation": op})))),
        None => Err(ApiError::conflict("game_update.current", "the server is already up to date")),
    }
}
