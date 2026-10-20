//! Managed Java runtimes: install (refused while a running server needs the version) and removal.
use super::*;

#[derive(Deserialize)]
pub(super) struct JavaReq { major: u32 }

const MAJORS: [u32; 5] = [8, 11, 17, 21, 25];

fn in_use(major: u32, users: &[String]) -> ApiError {
    ApiError::conflict("java.in_use", format!("Java {major} is in use by running servers: {}. Stop them first.", users.join(", ")))
}

pub(super) async fn java_install(State(st): State<St>, user: User, Json(r): Json<JavaReq>) -> ApiResult<(StatusCode, Json<Value>)> {
    user.require_admin()?;
    if !MAJORS.contains(&r.major) { return Err(ApiError::bad("java.version", "supported majors: 8, 11, 17, 21, 25")); }
    let users = crate::java::running_users(&st, r.major).await;
    if !users.is_empty() { return Err(in_use(r.major, &users)); }
    let op = ops::spawn_op(&st, "java.install", format!("Install Java {}", r.major), None, true, move |ctx| crate::java::install_managed(ctx, r.major));
    st.audit(&user.email, "java.install", &format!("Java {}", r.major), "", None).await;
    Ok((StatusCode::ACCEPTED, Json(json!({"operation": op}))))
}

pub(super) async fn java_delete(State(st): State<St>, user: User, Path(major): Path<u32>) -> ApiResult<StatusCode> {
    user.require_admin()?;
    let dir = crate::java::managed_dir(&st, major);
    if !dir.is_dir() {
        let st2 = st.clone();
        let other = tokio::task::spawn_blocking(move || crate::java::installs(&st2, false).iter().any(|i| i.major == major)).await.unwrap_or(false);
        return Err(if other { ApiError::bad("java.not_managed", format!("Java {major} was not installed by Moss, so Moss will not remove it")) } else { ApiError::not_found("managed Java") });
    }
    let users = crate::java::running_users(&st, major).await;
    if !users.is_empty() { return Err(in_use(major, &users)); }
    // Move aside first so a server starting at this very moment never sees a half-deleted runtime.
    let aside = st.cfg.tmp_dir().join(format!("java-removed-{}", uuid::Uuid::new_v4().simple()));
    tokio::fs::create_dir_all(st.cfg.tmp_dir()).await?;
    tokio::fs::rename(&dir, &aside).await?;
    let _ = tokio::fs::remove_dir_all(&aside).await;
    let st2 = st.clone();
    let _ = tokio::task::spawn_blocking(move || crate::java::installs(&st2, true)).await;
    st.audit(&user.email, "java.remove", &format!("Java {major}"), "", None).await;
    Ok(StatusCode::NO_CONTENT)
}
