//! Instance settings. Written by admins only; the webhook URL is a secret, so other users see only its host and prefix.
use super::*;
use crate::notify::rules;

pub(super) async fn settings_json(st: &Inner, is_admin: bool) -> Value {
    let webhook = st.setting("notify_webhook_url", "").await;
    let stored = st.setting("notify_events", "").await;
    let events = rules::parse_events(Some(stored.as_str()).filter(|s| !s.is_empty()));
    json!({
        "default_memory_mb": st.setting("default_memory_mb", "2048").await.parse::<i64>().unwrap_or(2048),
        "backup_keep": st.setting("backup_keep", "7").await.parse::<i64>().unwrap_or(7),
        "backup_before_change": st.setting("backup_before_change", "true").await == "true",
        "modrinth_contact": st.setting("modrinth_contact", "").await,
        "public_base_url": st.setting("public_base_url", "").await,
        "notify_webhook_url": if is_admin { webhook } else { rules::redact_webhook(&webhook) },
        "notify_events": events,
        "audit_console_commands": st.setting("audit_console_commands", "redacted").await,
        "access_message": st.setting("access_message", "").await,
        "data_dir": st.cfg.data_dir.to_string_lossy(), "bind": format!("{}:{}", st.cfg.bind, st.cfg.port), "version": env!("CARGO_PKG_VERSION"),
    })
}
pub(super) async fn settings_get(State(st): State<St>, user: User) -> Json<Value> { Json(settings_json(&st, user.is_admin()).await) }
pub(super) async fn settings_put(State(st): State<St>, user: User, Json(v): Json<Value>) -> ApiResult<Json<Value>> {
    user.require_admin()?;
    // validate everything first so a bad field never leaves the request half-applied
    let webhook = v["notify_webhook_url"].as_str().map(rules::validate_webhook_url).transpose()?;
    let events = (!v["notify_events"].is_null()).then(|| rules::validate_events(&v["notify_events"])).transpose()?;
    let audit = v["audit_console_commands"].as_str().map(rules::validate_audit_mode).transpose()?;
    let message = v["access_message"].as_str().map(rules::validate_access_message).transpose()?;
    if let Some(n) = v["default_memory_mb"].as_i64() && (256..=262144).contains(&n) { st.set_setting("default_memory_mb", &n.to_string()).await; }
    if let Some(n) = v["backup_keep"].as_i64() { st.set_setting("backup_keep", &n.clamp(1, 365).to_string()).await; }
    if let Some(b) = v["backup_before_change"].as_bool() { st.set_setting("backup_before_change", &b.to_string()).await; }
    if let Some(s) = v["modrinth_contact"].as_str() { st.set_setting("modrinth_contact", &s.chars().take(200).collect::<String>()).await; }
    if let Some(s) = v["public_base_url"].as_str() { st.set_setting("public_base_url", &s.chars().take(200).collect::<String>()).await; }
    if let Some(u) = &webhook { st.set_setting("notify_webhook_url", u).await; }
    if let Some(e) = &events { st.set_setting("notify_events", &serde_json::to_string(e).unwrap_or_default()).await; }
    if let Some(a) = audit { st.set_setting("audit_console_commands", a).await; }
    if let Some(m) = &message { st.set_setting("access_message", m).await; }
    st.audit(&user.email, "settings.update", "instance", "", None).await;
    Ok(Json(settings_json(&st, true).await))
}
