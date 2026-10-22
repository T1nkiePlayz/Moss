//! `GET/POST /system/update`: is a newer Moss available, and install it (the controller restarts afterwards).
//! Administrators only. See `crate::selfupdate`.
use super::*;
use crate::selfupdate;

#[derive(Deserialize, Default)]
pub(super) struct StartBody {
    /// A newer release to install instead of the latest, e.g. "v0.2.0".
    #[serde(default)]
    version: Option<String>,
}

pub(super) async fn status(State(st): State<St>, user: User) -> ApiResult<Json<Value>> {
    user.require_admin()?;
    Ok(Json(json!(selfupdate::info(&st))))
}

pub(super) async fn check(State(st): State<St>, user: User) -> ApiResult<Json<Value>> {
    user.require_admin()?;
    let i = selfupdate::check(&st).await
        .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, "update.lookup_failed", format!("could not look up the latest release: {e:#}")))?;
    Ok(Json(json!(i)))
}

pub(super) async fn start(State(st): State<St>, user: User, body: Option<Json<StartBody>>) -> ApiResult<(StatusCode, Json<Value>)> {
    user.require_admin()?;
    let body = body.map(|b| b.0).unwrap_or_default();
    let op = selfupdate::begin(&st, &user.actor(), body.version.as_deref()).await?;
    Ok((StatusCode::ACCEPTED, Json(json!({"operation": op}))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::update::Method;

    fn user(role: &str) -> User { User { id: "u".into(), email: "u@x.y".into(), role: role.into(), token: None, via_token: None, aal: None } }
    fn pin(v: &str) -> Option<Json<StartBody>> { Some(Json(StartBody { version: Some(v.into()) })) }

    // one test: the latest-release and method overrides are process-wide
    #[tokio::test]
    async fn updates_are_admin_only_forward_only_and_handed_to_the_helper() {
        let st = crate::testutil::state(|_| {}).await;
        assert!(status(State(st.clone()), user("viewer")).await.is_err());
        assert!(start(State(st.clone()), user("operator"), pin("v99.0.0")).await.is_err());

        selfupdate::set_test_state(Some("v99.0.0"), Some(Method::Manual { reason: "managed by Homebrew".into(), command: "brew upgrade moss".into() }));
        let i = status(State(st.clone()), user("admin")).await.unwrap().0;
        assert_eq!((i["available"].as_bool(), i["method"].as_str(), i["command"].as_str()), (Some(true), Some("manual"), Some("brew upgrade moss")));
        assert_eq!(i["release_url"], "https://github.com/T1nkiePlayz/Moss/releases/tag/v99.0.0");
        assert_eq!(start(State(st.clone()), user("admin"), pin("v99.0.0")).await.unwrap_err().code, "update.unavailable");

        selfupdate::set_test_state(Some("v99.0.0"), Some(Method::Helper));
        assert_eq!(start(State(st.clone()), user("admin"), pin("v0.0.1")).await.unwrap_err().code, "update.none", "no downgrades from the panel");
        assert_eq!(start(State(st.clone()), user("admin"), pin("v1;reboot")).await.unwrap_err().code, "validation");
        let (code, _) = start(State(st.clone()), user("admin"), pin("99.0.0")).await.unwrap();
        assert_eq!(code, StatusCode::ACCEPTED);
        let req = st.cfg.data_dir.join(selfupdate::REQUEST_FILE);
        for _ in 0..100 { if req.exists() { break; } tokio::time::sleep(std::time::Duration::from_millis(20)).await; }
        assert_eq!(std::fs::read_to_string(&req).unwrap(), "v99.0.0\n");
        selfupdate::set_test_state(None, None);
    }
}
