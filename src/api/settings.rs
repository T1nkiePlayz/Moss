//! Instance settings (admin-only read/write).
use super::*;

pub(super) async fn settings_json(st: &Inner) -> Value {
    json!({
        "default_memory_mb": st.setting("default_memory_mb", "2048").await.parse::<i64>().unwrap_or(2048),
        "backup_keep": st.setting("backup_keep", "7").await.parse::<i64>().unwrap_or(7),
        "backup_before_change": st.setting("backup_before_change", "true").await == "true",
        "modrinth_contact": st.setting("modrinth_contact", "").await,
        "public_base_url": st.setting("public_base_url", "").await,
        "data_dir": st.cfg.data_dir.to_string_lossy(), "bind": format!("{}:{}", st.cfg.bind, st.cfg.port), "version": env!("CARGO_PKG_VERSION"),
    })
}
pub(super) async fn settings_get(State(st): State<St>, _u: User) -> Json<Value> { Json(settings_json(&st).await) }
pub(super) async fn settings_put(State(st): State<St>, user: User, Json(v): Json<Value>) -> ApiResult<Json<Value>> {
    user.require_admin()?;
    if let Some(n) = v["default_memory_mb"].as_i64() && (256..=262144).contains(&n) { st.set_setting("default_memory_mb", &n.to_string()).await; }
    if let Some(n) = v["backup_keep"].as_i64() { st.set_setting("backup_keep", &n.clamp(1, 365).to_string()).await; }
    if let Some(b) = v["backup_before_change"].as_bool() { st.set_setting("backup_before_change", &b.to_string()).await; }
    if let Some(s) = v["modrinth_contact"].as_str() { st.set_setting("modrinth_contact", &s.chars().take(200).collect::<String>()).await; }
    if let Some(s) = v["public_base_url"].as_str() { st.set_setting("public_base_url", &s.chars().take(200).collect::<String>()).await; }
    st.audit(&user.email, "settings.update", "instance", "", None).await;
    Ok(Json(settings_json(&st).await))
}
