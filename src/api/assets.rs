//! The embedded web UI (`web/dist`): static file serving, SPA fallback, cache headers and the CSP.
use super::*;

#[derive(RustEmbed)]
#[folder = "web/dist/"]
struct Assets;

pub(super) async fn static_files(State(st): State<St>, uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let (file, is_index) = match Assets::get(path).filter(|_| !path.is_empty()) {
        Some(f) => (f, false),
        None => match Assets::get("index.html") { Some(f) => (f, true), None => return (StatusCode::NOT_FOUND, "web UI is not built: run `npm run build` in web/").into_response() },
    };
    let mime = if is_index { "text/html".to_string() } else { mime_guess::from_path(path).first_or_octet_stream().to_string() };
    let mut r = Response::new(axum::body::Body::from(file.data.into_owned()));
    let h = r.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_str(&mime).unwrap());
    if is_index {
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        let sb = st.cfg.supabase_url.trim_end_matches('/');
        let csp = format!("default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: https://cdn.modrinth.com https://*.forgecdn.net https://*.supabase.co https://avatars.githubusercontent.com; connect-src 'self' {sb} ws: wss:; frame-ancestors 'none'; base-uri 'none'; form-action 'self'");
        if let Ok(v) = HeaderValue::from_str(&csp) { h.insert(header::CONTENT_SECURITY_POLICY, v); }
    } else if path.starts_with("assets/") {
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=31536000, immutable"));
    }
    r
}
