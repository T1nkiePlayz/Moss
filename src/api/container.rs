//! Container isolation: engine status and the image download operation. The isolation mode itself is a field of
//! `PATCH /servers/{id}` (admin only); there is deliberately no way to pass flags, mounts or privileges.
use super::*;

/// Engine facts for the Settings page and `moss doctor`: never an error, an unavailable engine is a normal answer.
pub(super) async fn container_status(State(st): State<St>, _user: User) -> ApiResult<Json<Value>> {
    crate::container::forget(&st);
    Ok(Json(status_json(&st).await))
}

const DEFAULT_IMAGE: &str = "docker.io/library/eclipse-temurin:<java version>-jre (games: their own image, shown per server)";

async fn status_json(st: &St) -> Value {
    match crate::container::engine(st).await {
        Ok(e) => { let i = e.info(); json!({"available": true, "engine": i.kind.name(), "version": i.version, "rootless": i.rootless, "message": null, "outbound_restricted": false,
            "game_only_supported": i.kind == crate::container::Kind::Docker, "default_image": DEFAULT_IMAGE}) }
        Err(m) => json!({"available": false, "engine": null, "version": null, "rootless": null, "message": m, "outbound_restricted": false,
            "game_only_supported": false, "default_image": DEFAULT_IMAGE}),
    }
}

/// Download (or confirm) the server's container image as an operation.
pub(super) async fn container_pull(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<(StatusCode, Json<Value>)> {
    user.require_admin()?;
    let s = server_for(&st, &user, &id, "config.write").await?;
    if s.isolation != "container" { return Err(ApiError::conflict("container.not_enabled", "this server is not set to container isolation")); }
    crate::container::engine(&st).await.map_err(|m| ApiError::conflict("container.engine_missing", m))?;
    Ok((StatusCode::ACCEPTED, Json(json!({"operation": spawn_pull(&st, &s)}))))
}

pub(super) fn spawn_pull(st: &St, s: &Srv) -> ops::Operation {
    let image = crate::container::image::for_server(s);
    ops::spawn_op(st, "container.pull", format!("Download {image}"), Some(&s.id), true, { let st = st.clone(); move |ctx| async move { crate::container::pull::ensure(&st, &ctx, &image).await } })
}
