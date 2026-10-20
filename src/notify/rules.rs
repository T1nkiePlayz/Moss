//! Pure notification rules: known kinds, defaults, visibility, settings validation, webhook URL handling and the
//! disk-space hysteresis. Nothing here touches the database or the network.
use crate::error::ApiError;

pub const KINDS: &[&str] = &["crash", "crash_loop", "backup_failed", "disk_low", "operation_failed", "restart_failed", "update_available", "security", "webhook_disabled"];
pub const DEFAULT_EVENTS: &[&str] = &["crash", "crash_loop", "backup_failed", "disk_low", "restart_failed"];
/// Kinds that describe the whole host or instance: only administrators may see them.
pub const ADMIN_ONLY: &[&str] = &["disk_low", "update_available", "security", "webhook_disabled"];
pub const SEVERITIES: &[&str] = &["info", "warn", "critical"];
/// Same kind + same server within this window is reported once.
pub const DEDUPE_WINDOW_MIN: i64 = 10;

pub fn is_admin_only(kind: &str) -> bool { ADMIN_ONLY.contains(&kind) }

/// Can a user who sees `visible` servers (None = all) see a notification?
pub fn visible_to(kind: &str, server_id: Option<&str>, is_admin: bool, visible: Option<&[String]>) -> bool {
    if is_admin { return true; }
    !is_admin_only(kind) && server_id.is_some_and(|s| visible.is_some_and(|v| v.iter().any(|x| x == s)))
}

pub fn parse_events(stored: Option<&str>) -> Vec<String> {
    match stored.and_then(|s| serde_json::from_str::<Vec<String>>(s).ok()) {
        Some(v) => v,
        None => DEFAULT_EVENTS.iter().map(|s| s.to_string()).collect(),
    }
}

pub fn validate_events(v: &serde_json::Value) -> Result<Vec<String>, ApiError> {
    let bad = || ApiError::bad("validation", "notify_events must be a list of known event kinds").field("notify_events", &KINDS.join(" | "));
    let mut out: Vec<String> = vec![];
    for e in v.as_array().ok_or_else(bad)? {
        let k = e.as_str().filter(|k| KINDS.contains(k)).ok_or_else(bad)?;
        if !out.iter().any(|o| o == k) { out.push(k.to_string()); }
    }
    Ok(out)
}

/// A webhook is empty (disabled) or an https URL without credentials, at most 500 characters.
pub fn validate_webhook_url(s: &str) -> Result<String, ApiError> {
    let s = s.trim();
    if s.is_empty() { return Ok(String::new()); }
    let bad = |m: &str| ApiError::bad("validation", m.to_string()).field("notify_webhook_url", m);
    if s.len() > 500 { return Err(bad("at most 500 characters")); }
    let u = reqwest::Url::parse(s).map_err(|_| bad("not a valid URL"))?;
    if u.scheme() != "https" { return Err(bad("must be an https:// URL")); }
    if u.host_str().is_none_or(str::is_empty) { return Err(bad("missing host")); }
    if !u.username().is_empty() || u.password().is_some() { return Err(bad("must not contain credentials")); }
    Ok(s.to_string())
}

pub fn validate_audit_mode(s: &str) -> Result<&'static str, ApiError> {
    match s { "redacted" => Ok("redacted"), "all" => Ok("all"), "off" => Ok("off"),
        _ => Err(ApiError::bad("validation", "audit_console_commands must be redacted, all or off").field("audit_console_commands", "redacted | all | off")) }
}

pub fn validate_access_message(s: &str) -> Result<String, ApiError> {
    if s.chars().count() > 500 { return Err(ApiError::bad("validation", "access_message is limited to 500 characters").field("access_message", "at most 500 characters")); }
    Ok(s.trim().to_string())
}

/// Host and first path segment only, for people who may see that a webhook exists but not its secret token.
pub fn redact_webhook(url: &str) -> String {
    let Ok(u) = reqwest::Url::parse(url) else { return if url.is_empty() { String::new() } else { "[redacted]".into() } };
    let first = u.path_segments().and_then(|mut s| s.next()).filter(|s| !s.is_empty());
    format!("{}://{}/{}", u.scheme(), u.host_str().unwrap_or(""), first.map_or("…".to_string(), |f| format!("{f}/…")))
}

/// Disk-space alert with hysteresis: fires once when free space drops below the low mark, and re-arms only
/// after it has recovered above the (higher) clear mark.
#[derive(Default, Debug, PartialEq)]
pub struct DiskAlert { pub active: bool }
impl DiskAlert {
    pub const LOW_PCT: f64 = 5.0;
    pub const CLEAR_PCT: f64 = 8.0;
    pub const LOW_BYTES: u64 = 2 << 30;
    pub const CLEAR_BYTES: u64 = 3 << 30;
    /// Returns true when a notification should be sent now.
    pub fn step(&mut self, free: u64, total: u64) -> bool {
        if total == 0 { return false; }
        let pct = free as f64 / total as f64 * 100.0;
        if !self.active {
            if pct < Self::LOW_PCT || free < Self::LOW_BYTES { self.active = true; return true; }
        } else if pct > Self::CLEAR_PCT && free > Self::CLEAR_BYTES {
            self.active = false;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webhook_validation() {
        assert_eq!(validate_webhook_url("  ").unwrap(), "");
        assert!(validate_webhook_url("https://discord.com/api/webhooks/1/abc").is_ok());
        for bad in ["http://example.com/x", "ftp://x.y", "not a url", "https://user:pw@example.com/x", "https://", &format!("https://example.com/{}", "a".repeat(500))] {
            assert!(validate_webhook_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn event_validation_and_defaults() {
        assert_eq!(validate_events(&serde_json::json!(["crash", "crash", "disk_low"])).unwrap(), vec!["crash", "disk_low"]);
        assert!(validate_events(&serde_json::json!(["nope"])).is_err());
        assert!(validate_events(&serde_json::json!("crash")).is_err());
        assert_eq!(parse_events(None).len(), 5);
        assert_eq!(parse_events(Some("[\"security\"]")), vec!["security"]);
        assert_eq!(parse_events(Some("garbage")).len(), 5);
    }

    #[test]
    fn audit_and_message_validation() {
        assert!(validate_audit_mode("redacted").is_ok() && validate_audit_mode("off").is_ok() && validate_audit_mode("all").is_ok());
        assert!(validate_audit_mode("none").is_err());
        assert!(validate_access_message(&"x".repeat(500)).is_ok());
        assert!(validate_access_message(&"x".repeat(501)).is_err());
    }

    #[test]
    fn webhook_redaction_keeps_only_host_and_prefix() {
        let r = redact_webhook("https://discord.com/api/webhooks/123/SECRET");
        assert_eq!(r, "https://discord.com/api/…");
        assert!(!r.contains("SECRET") && !r.contains("123"));
        assert_eq!(redact_webhook(""), "");
        assert_eq!(redact_webhook("garbage"), "[redacted]");
    }

    #[test]
    fn visibility() {
        let vis = vec!["a".to_string()];
        assert!(visible_to("disk_low", None, true, None));
        assert!(!visible_to("disk_low", Some("a"), false, Some(&vis)));
        assert!(visible_to("crash", Some("a"), false, Some(&vis)));
        assert!(!visible_to("crash", Some("b"), false, Some(&vis)));
        assert!(!visible_to("crash", None, false, Some(&vis)));
    }

    #[test]
    fn disk_alert_fires_once_with_hysteresis() {
        let (t, mut d) = (100u64 << 30, DiskAlert::default());
        assert!(!d.step(50 << 30, t));
        assert!(d.step(4 << 30, t)); // 4% -> fire
        assert!(!d.step(3 << 30, t)); // still low: no repeat
        assert!(!d.step(6 << 30, t)); // 6% is above low but below clear: stays active
        assert!(!d.step(4 << 30, t)); // flapping around the mark does not re-fire
        assert!(!d.step(20 << 30, t) && !d.active); // recovered
        assert!(d.step(1 << 30, t)); // re-armed
        assert!(!DiskAlert::default().step(0, 0));
    }
}
