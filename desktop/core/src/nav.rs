//! Which addresses the window may show. Only the controller's own loopback origin is allowed inside the window;
//! everything else is handed to the system browser, and only web links at that.

/// True for `http://127.0.0.1:<port>` and paths below it. `@`, extra digits or another host never match.
pub fn is_panel_url(url: &str, port: u16) -> bool {
    let origin = format!("http://127.0.0.1:{port}");
    match url.strip_prefix(origin.as_str()) {
        Some(rest) => rest.is_empty() || rest.starts_with(['/', '?', '#']),
        None => false,
    }
}

/// Links the app may open in the system browser: plain web addresses, nothing that runs code or reads files.
pub fn may_open_externally(url: &str) -> bool {
    let l = url.trim().to_ascii_lowercase();
    (l.starts_with("https://") || l.starts_with("http://")) && !l.contains(char::is_whitespace)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn panel_origin_is_exact() {
        assert!(is_panel_url("http://127.0.0.1:8420", 8420));
        assert!(is_panel_url("http://127.0.0.1:8420/", 8420));
        assert!(is_panel_url("http://127.0.0.1:8420/#/servers", 8420));
        assert!(is_panel_url("http://127.0.0.1:8420/api/v1/status?x=1", 8420));
        assert!(!is_panel_url("http://127.0.0.1:84201/", 8420));
        assert!(!is_panel_url("http://127.0.0.1:8420@evil.example/", 8420));
        assert!(!is_panel_url("http://127.0.0.1.evil.example:8420/", 8420));
        assert!(!is_panel_url("https://127.0.0.1:8420/", 8420));
        assert!(!is_panel_url("http://localhost:8420/", 8420));
        assert!(!is_panel_url("http://127.0.0.1:9000/", 8420));
        assert!(!is_panel_url("file:///etc/passwd", 8420));
    }
    #[test]
    fn only_web_links_leave_the_app() {
        assert!(may_open_externally("https://modrinth.com/mod/sodium"));
        assert!(may_open_externally("http://example.com"));
        assert!(!may_open_externally("javascript:alert(1)"));
        assert!(!may_open_externally("file:///etc/passwd"));
        assert!(!may_open_externally("tel:123"));
        assert!(!may_open_externally("https://a.example/ x"));
    }
}
