//! Live map endpoints. `GET /api/v1/servers/{id}/map` (same permission as viewing the server) tells the UI whether a map
//! plugin was found, whether it answers, and hands out a signed link `/map/{id}/{token}/`. That link is served by
//! `proxy` below, outside `/api/v1`, because the map page runs in a sandboxed iframe with an opaque origin and so carries
//! no Moss cookies: the token in the path is the credential. The proxy only dials `127.0.0.1:<detected port>`, only
//! passes GET and HEAD, and replaces every security-relevant header (see `crate::maps`).
use super::*;
use crate::maps::{self, Scope};
use axum::{body::Body, extract::Request};
use futures_util::TryStreamExt;
use std::sync::OnceLock;

const MAX_BODY: u64 = 128 << 20;
const MAX_QUERY: usize = 2048;
/// Public map viewers load many tiles, so the budget is far above the status page's own, but still bounded per client.
const PUBLIC_RATE_LIMIT: u32 = 3000;

/// Routes outside `/api/v1`, added after the router's global header layers so the map's responses keep their own
/// headers (the global `X-Frame-Options: DENY` would otherwise stop the UI framing them).
pub fn public_routes() -> Router<St> {
    Router::new().route("/map/{id}/{token}", get(proxy)).route("/map/{id}/{token}/", get(proxy)).route("/map/{id}/{token}/{*rest}", get(proxy).options(proxy))
}

async fn reachable(port: u16) -> bool {
    tokio::time::timeout(Duration::from_millis(500), tokio::net::TcpStream::connect(("127.0.0.1", port))).await.is_ok_and(|r| r.is_ok())
}

pub(super) async fn map_info(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let s = server_for(&st, &user, &id, "server.view").await?;
    let Some(m) = maps::detect(&st, &s) else { return Ok(Json(json!({"available": false}))) };
    let running = server::view(&s, &st)["state"] == "running";
    let up = running && reachable(m.port).await;
    let (url, exp) = maps::link(&st, &s.id, Scope::User, maps::TOKEN_TTL_SECS);
    Ok(Json(json!({"available": true, "kind": m.kind, "running": running, "reachable": up, "url": url, "expires_at": exp})))
}

fn client() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(|| reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none()).connect_timeout(Duration::from_secs(3)).timeout(Duration::from_secs(60)).build().expect("map proxy client"))
}

/// A plain-text answer that still carries the hardening headers.
fn plain(status: StatusCode, msg: &'static str) -> Response {
    let mut r = (status, msg).into_response();
    r.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain; charset=utf-8"));
    maps::harden(r.headers_mut());
    r
}

async fn proxy(State(st): State<St>, req: Request) -> Response {
    let method = req.method().clone();
    if method == axum::http::Method::OPTIONS {
        // CORS preflight from the map's opaque origin: answered here, never forwarded
        let mut r = Response::new(Body::empty());
        *r.status_mut() = StatusCode::NO_CONTENT;
        maps::harden(r.headers_mut());
        let h = r.headers_mut();
        h.insert(header::ACCESS_CONTROL_ALLOW_METHODS, HeaderValue::from_static("GET, HEAD"));
        h.insert(header::ACCESS_CONTROL_ALLOW_HEADERS, HeaderValue::from_static("accept, range, if-none-match, if-modified-since"));
        h.insert(header::ACCESS_CONTROL_MAX_AGE, HeaderValue::from_static("600"));
        return r;
    }
    if method != axum::http::Method::GET && method != axum::http::Method::HEAD {
        let mut r = plain(StatusCode::METHOD_NOT_ALLOWED, "Only GET and HEAD are allowed.");
        r.headers_mut().insert(header::ALLOW, HeaderValue::from_static("GET, HEAD"));
        return r;
    }
    // /map/{id}/{token}[/rest], parsed from the raw path so percent-encoding is judged by `normalise_path`, not by the router
    let path = req.uri().path().to_string();
    let mut parts = path.splitn(5, '/');
    let (_, _, Some(id), Some(token)) = (parts.next(), parts.next(), parts.next(), parts.next()) else { return plain(StatusCode::NOT_FOUND, "Not found.") };
    let rest = parts.next();
    let prefix = format!("/map/{id}/{token}");
    let not_found = || plain(StatusCode::NOT_FOUND, "Not found.");
    let Some(scope) = maps::verify(&st, id, token) else { return not_found() };
    let Some(rest) = rest else {
        // without the trailing slash the page's relative links would resolve one level too high
        let mut r = Response::new(Body::empty());
        *r.status_mut() = StatusCode::PERMANENT_REDIRECT;
        r.headers_mut().insert(header::LOCATION, HeaderValue::from_str(&format!("{prefix}/{}", req.uri().query().map(|q| format!("?{q}")).unwrap_or_default())).unwrap_or_else(|_| HeaderValue::from_static("/")));
        maps::harden(r.headers_mut());
        return r;
    };
    if scope == Scope::Public {
        if status_pages::limited_for(&st, &req, "map:", PUBLIC_RATE_LIMIT) { let mut r = status_pages::too_many(); maps::harden(r.headers_mut()); return r; }
        // a public link stops working as soon as the owner switches the map off the status page
        if !status_pages::map_is_public(&st, id).await { return not_found(); }
    }
    let Ok(s) = server::get(&st, id).await else { return not_found() };
    let Some(m) = maps::detect(&st, &s) else { return not_found() };
    let Some(up_path) = maps::normalise_path(rest) else { return plain(StatusCode::BAD_REQUEST, "Bad path.") };
    let query = req.uri().query();
    if query.is_some_and(|q| q.len() > MAX_QUERY) { return plain(StatusCode::URI_TOO_LONG, "Query too long.") }
    let url = maps::upstream_url(m.port, &up_path, query);
    let mut up = if method == axum::http::Method::HEAD { client().head(&url) } else { client().get(&url) };
    for (n, v) in maps::forward_request_headers(req.headers()) { up = up.header(n, v); }
    let resp = match up.send().await {
        Ok(r) => r,
        Err(_) => return plain(StatusCode::BAD_GATEWAY, "The map is not reachable. Is the server running?"),
    };
    if resp.content_length().is_some_and(|n| n > MAX_BODY) { return plain(StatusCode::BAD_GATEWAY, "The map answered with too much data.") }
    let status = resp.status();
    let mut headers = maps::filter_response_headers(resp.headers(), &prefix);
    maps::harden(&mut headers);
    let mut seen = 0u64;
    let stream = resp.bytes_stream().map_err(std::io::Error::other).and_then(move |b| {
        seen += b.len() as u64;
        std::future::ready(if seen > MAX_BODY { Err(std::io::Error::other("map response too large")) } else { Ok(b) })
    });
    let mut r = Response::new(Body::from_stream(stream));
    *r.status_mut() = status;
    *r.headers_mut() = headers;
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{Reply, stub};

    async fn serve(st: &St) -> String {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        let app = router(st.clone());
        tokio::spawn(async move { let _ = axum::serve(l, app).await; });
        base
    }

    /// A Paper server whose BlueMap web server is configured for `port`.
    async fn server_with_map(st: &St, id: &str, port: u16) {
        let i = id.to_string();
        st.db.call(move |c| c.execute("INSERT INTO servers(id,name,runtime,game_version,port,memory_mb,created_at,installed) VALUES (?,'Survival','paper','1.21.1',25599,512,'2026-01-01T00:00:00Z',1)", [&i])).await.unwrap();
        let d = st.cfg.servers_dir().join(id).join("plugins/BlueMap");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("webserver.conf"), format!("enabled: true\nport: {port}\n")).unwrap();
    }

    fn http() -> reqwest::Client { reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none()).build().unwrap() }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_proxy_talks_only_to_the_detected_loopback_port_and_replaces_security_headers() {
        let st = crate::testutil::state(|_| {}).await;
        let upstream = stub(|_, p| if p.starts_with("/redir") {
            Reply::Raw(302, vec![("location", "http://evil.example/x".into())], String::new())
        } else {
            Reply::Raw(200, vec![("content-type", "text/html".into()), ("content-security-policy", "default-src *".into()), ("set-cookie", "sid=1".into()),
                ("access-control-allow-origin", "https://evil.example".into()), ("access-control-allow-credentials", "true".into()), ("x-frame-options", "SAMEORIGIN".into()), ("etag", "\"v1\"".into())], "<h1>map</h1>".into())
        }).await;
        let decoy = stub(|_, _| Reply::status(200)).await;
        let port: u16 = upstream.base.rsplit(':').next().unwrap().parse().unwrap();
        server_with_map(&st, "srv1", port).await;
        let base = serve(&st).await;
        let (token, _) = maps::mint(&st, "srv1", Scope::User, 600);
        let c = http();

        let r = c.get(format!("{base}/map/srv1/{token}/tiles/a.png?x=1")).header("cookie", "moss=secret").header("authorization", "Bearer t").header("range", "bytes=0-3").send().await.unwrap();
        assert_eq!(r.status(), 200);
        let h = r.headers().clone();
        assert_eq!(h["content-security-policy"], "sandbox allow-scripts allow-popups");
        assert_eq!(h["access-control-allow-origin"], "*");
        assert_eq!(h["x-content-type-options"], "nosniff");
        assert!(h.get("set-cookie").is_none() && h.get("access-control-allow-credentials").is_none() && h.get("x-frame-options").is_none());
        assert_eq!(h["etag"], "\"v1\"");
        assert_eq!(r.text().await.unwrap(), "<h1>map</h1>");
        assert_eq!(*upstream.hits.lock().unwrap(), ["GET /tiles/a.png?x=1"]);

        // host tricks in the path stay paths on the detected port; the decoy is never contacted
        let decoy_port = decoy.base.rsplit(':').next().unwrap();
        let r = c.get(format!("{base}/map/srv1/{token}//127.0.0.1:{decoy_port}/x")).send().await.unwrap();
        assert_eq!(r.status(), 200);
        assert_eq!(upstream.hits.lock().unwrap().last().unwrap(), &format!("GET /127.0.0.1:{decoy_port}/x"));
        // traversal is refused before anything is dialled
        let before = upstream.hits.lock().unwrap().len();
        for p in ["a%2fb", "a%00b"] { assert_eq!(c.get(format!("{base}/map/srv1/{token}/{p}")).send().await.unwrap().status(), 400, "{p}"); }
        assert_eq!(upstream.hits.lock().unwrap().len(), before);
        // redirects to other hosts are dropped, not followed
        let r = c.get(format!("{base}/map/srv1/{token}/redir")).send().await.unwrap();
        assert_eq!(r.status(), 302);
        assert!(r.headers().get("location").is_none());
        // missing slash redirects within the proxy
        let r = c.get(format!("{base}/map/srv1/{token}")).send().await.unwrap();
        assert_eq!((r.status().as_u16(), r.headers()["location"].to_str().unwrap()), (308, format!("/map/srv1/{token}/").as_str()));
        // other methods are not proxied
        let before = upstream.hits.lock().unwrap().len();
        assert_eq!(c.post(format!("{base}/map/srv1/{token}/x")).send().await.unwrap().status(), 405);
        assert_eq!(c.delete(format!("{base}/map/srv1/{token}/x")).send().await.unwrap().status(), 405);
        let r = c.request(reqwest::Method::OPTIONS, format!("{base}/map/srv1/{token}/x")).send().await.unwrap();
        assert_eq!((r.status().as_u16(), r.headers()["access-control-allow-origin"].to_str().unwrap()), (204, "*"));
        assert_eq!(upstream.hits.lock().unwrap().len(), before);
        // HEAD works
        assert_eq!(c.head(format!("{base}/map/srv1/{token}/h")).send().await.unwrap().status(), 200);
        assert!(decoy.hits.lock().unwrap().is_empty(), "nothing but 127.0.0.1:<detected port> is contacted");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn bad_tokens_unknown_servers_and_dead_maps_are_refused_cleanly() {
        let st = crate::testutil::state(|_| {}).await;
        let upstream = stub(|_, _| Reply::status(200)).await;
        let port: u16 = upstream.base.rsplit(':').next().unwrap().parse().unwrap();
        server_with_map(&st, "srv1", port).await;
        let closed = { let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap(); l.local_addr().unwrap().port() };
        server_with_map(&st, "srv2", closed).await;
        let base = serve(&st).await;
        let c = http();
        let (good, _) = maps::mint(&st, "srv1", Scope::User, 600);
        let (other, _) = maps::mint(&st, "srv2", Scope::User, 600);
        let (expired, _) = maps::mint(&st, "srv1", Scope::User, -1);
        let (public, _) = maps::mint(&st, "srv1", Scope::Public, 600);
        let tampered = format!("{good}0");
        for (id, t) in [("srv1", other.as_str()), ("srv1", expired.as_str()), ("srv1", "junk"), ("srv1", tampered.as_str()), ("nope", good.as_str())] {
            assert_eq!(c.get(format!("{base}/map/{id}/{t}/x")).send().await.unwrap().status(), 404, "{id} {t}");
        }
        // a public link needs a status page that shows the map
        assert_eq!(c.get(format!("{base}/map/srv1/{public}/x")).send().await.unwrap().status(), 404);
        assert!(upstream.hits.lock().unwrap().is_empty());
        st.db.call(|c| {
            c.execute("INSERT INTO status_pages(id,slug,title,description,enabled,show_players,show_address,created_at,show_map) VALUES ('p1','my-page','T','D',1,0,NULL,'x',1)", [])?;
            c.execute("INSERT INTO status_page_servers(page_id,server_id,position) VALUES ('p1','srv1',0)", [])
        }).await.unwrap();
        assert_eq!(c.get(format!("{base}/map/srv1/{public}/x")).send().await.unwrap().status(), 200);
        st.db.call(|c| c.execute("UPDATE status_pages SET show_map=0", [])).await.unwrap();
        assert_eq!(c.get(format!("{base}/map/srv1/{public}/x")).send().await.unwrap().status(), 404, "switching the map off revokes public links");
        // a map that does not answer is a 502 with the hardening headers, not a hang or a leak
        let r = c.get(format!("{base}/map/srv2/{other}/")).send().await.unwrap();
        assert_eq!(r.status(), 502);
        assert_eq!(r.headers()["content-security-policy"], "sandbox allow-scripts allow-popups");
    }
}
