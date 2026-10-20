//! Origin check for the browser-initiated stream endpoints (console WebSocket, event stream). A browser always sends
//! `Origin` on a WebSocket upgrade, so a page on another site cannot open a stream even if it somehow held a ticket.
//! Requests without `Origin` (curl, scripts, same-origin GET) are not browser cross-site requests and pass.
use crate::{error::ApiError, state::St};
use axum::http::{HeaderMap, header};

/// `scheme://host[:port]` -> `host[:port]`, lower-cased; anything else (including `null`) -> None.
fn authority(origin: &str) -> Option<String> {
    let rest = origin.strip_prefix("https://").or_else(|| origin.strip_prefix("http://"))?;
    (!rest.is_empty() && !rest.contains(['/', '@', ' '])).then(|| rest.to_ascii_lowercase())
}

pub fn same_host(headers: &HeaderMap, trust_proxy: bool) -> bool {
    let Some(o) = headers.get(header::ORIGIN) else { return true };
    let Some(o) = o.to_str().ok().and_then(authority) else { return false };
    let hv = |name: &str| headers.get(name).and_then(|v| v.to_str().ok()).map(|v| v.split(',').next().unwrap_or("").trim().to_ascii_lowercase()).filter(|v| !v.is_empty());
    if hv("host").is_some_and(|h| h == o) { return true; }
    trust_proxy && hv("x-forwarded-host").is_some_and(|h| h == o)
}

pub fn check(st: &St, headers: &HeaderMap) -> Result<(), ApiError> {
    if same_host(headers, st.cfg.trust_proxy) { Ok(()) } else { Err(ApiError::forbidden("cross-origin streams are not allowed")) }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn h(pairs: &[(&str, &str)]) -> HeaderMap { let mut m = HeaderMap::new(); for (k, v) in pairs { m.insert(axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(), v.parse().unwrap()); } m }
    #[test]
    fn same_host_passes_cross_origin_fails() {
        assert!(same_host(&h(&[("host", "moss.example.com")]), false), "no Origin: not a browser cross-site request");
        assert!(same_host(&h(&[("host", "moss.example.com"), ("origin", "https://moss.example.com")]), false));
        assert!(same_host(&h(&[("host", "127.0.0.1:8080"), ("origin", "http://127.0.0.1:8080")]), false));
        assert!(same_host(&h(&[("host", "Moss.Example.com"), ("origin", "https://moss.example.COM")]), false));
        assert!(!same_host(&h(&[("host", "moss.example.com"), ("origin", "https://evil.example")]), false));
        assert!(!same_host(&h(&[("host", "moss.example.com"), ("origin", "https://moss.example.com.evil.example")]), false));
        assert!(!same_host(&h(&[("host", "moss.example.com"), ("origin", "https://moss.example.com:8443")]), false), "port must match");
        assert!(!same_host(&h(&[("host", "moss.example.com"), ("origin", "null")]), false));
        assert!(!same_host(&h(&[("host", "moss.example.com"), ("origin", "https://moss.example.com@evil.example")]), false));
    }
    #[test]
    fn forwarded_host_counts_only_behind_a_trusted_proxy() {
        let hs = h(&[("host", "127.0.0.1:8080"), ("x-forwarded-host", "moss.example.com"), ("origin", "https://moss.example.com")]);
        assert!(same_host(&hs, true));
        assert!(!same_host(&hs, false), "an untrusted client can set X-Forwarded-Host itself");
        let evil = h(&[("host", "127.0.0.1:8080"), ("x-forwarded-host", "moss.example.com"), ("origin", "https://evil.example")]);
        assert!(!same_host(&evil, true));
    }
}
