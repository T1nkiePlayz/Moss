//! Live player list plus kick and ban for game and Minecraft servers (`games::admin` does the work).
use super::*;
use crate::games::admin;

pub(super) async fn list(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    authz(&st, &user, &id, "server.view").await?;
    let s = server::get(&st, &id).await?;
    let l = admin::list(&st, &s).await?;
    let players: Vec<Value> = l.players.iter().map(|p| json!({"name": p.name, "id": p.id})).collect();
    Ok(Json(json!({"players": players, "online": l.count, "source": l.source, "can_kick": l.can_kick, "can_ban": l.can_ban})))
}

#[derive(Deserialize, Default)] pub(super) struct ActReq { reason: Option<String> }

async fn act(st: St, user: User, id: String, name: String, r: Option<Json<ActReq>>, ban: bool) -> ApiResult<StatusCode> {
    authz(&st, &user, &id, "server.console.write").await?;
    let s = server::get(&st, &id).await?;
    let reason = r.and_then(|Json(r)| r.reason);
    admin::act(&st, &s, &name, reason.as_deref(), ban).await?;
    let verb = if ban { "player.ban" } else { "player.kick" };
    st.audit(&user.actor(), verb, &s.name, &format!("{name}{}", reason.as_deref().map(|r| format!(" ({r})")).unwrap_or_default()), Some(&id)).await;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn kick(State(st): State<St>, user: User, Path((id, name)): Path<(String, String)>, r: Option<Json<ActReq>>) -> ApiResult<StatusCode> { act(st, user, id, name, r, false).await }
pub(super) async fn ban(State(st): State<St>, user: User, Path((id, name)): Path<(String, String)>, r: Option<Json<ActReq>>) -> ApiResult<StatusCode> { act(st, user, id, name, r, true).await }
