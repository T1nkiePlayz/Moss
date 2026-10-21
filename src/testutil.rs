//! Test helpers: a scriptable stub HTTP server (for provider outages) and a ready application state on a temp directory.
use crate::state::St;
use axum::{Router, body::Body, extract::Request, http::{HeaderName, HeaderValue, StatusCode}, response::Response};
use std::{path::PathBuf, sync::{Arc, Mutex}, time::Duration};

/// What the stub answers for one request.
pub enum Reply {
    /// Status, extra headers, body text.
    Raw(u16, Vec<(&'static str, String)>, String),
    /// Never answers (until the caller gives up).
    Hang,
}

impl Reply {
    pub fn json(status: u16, v: serde_json::Value) -> Self { Reply::Raw(status, vec![("content-type", "application/json".into())], v.to_string()) }
    pub fn status(status: u16) -> Self { Reply::Raw(status, vec![], String::new()) }
}

pub struct Stub { pub base: String, pub hits: Arc<Mutex<Vec<String>>> }

/// Serves `f(method, path_and_query)` on a loopback port until the test process ends.
pub async fn stub<F>(f: F) -> Stub where F: Fn(&str, &str) -> Reply + Send + Sync + 'static {
    let hits = Arc::new(Mutex::new(vec![]));
    let (h2, f) = (hits.clone(), Arc::new(f));
    let app = Router::new().fallback(move |req: Request| {
        let (h2, f) = (h2.clone(), f.clone());
        async move {
            let pq = req.uri().path_and_query().map(|p| p.to_string()).unwrap_or_default();
            h2.lock().unwrap().push(format!("{} {pq}", req.method()));
            match f(req.method().as_str(), &pq) {
                Reply::Hang => { tokio::time::sleep(Duration::from_secs(120)).await; Response::new(Body::empty()) }
                Reply::Raw(s, headers, body) => {
                    let mut r = Response::new(Body::from(body));
                    *r.status_mut() = StatusCode::from_u16(s).unwrap();
                    for (k, v) in headers { r.headers_mut().insert(HeaderName::from_static(k), HeaderValue::from_str(&v).unwrap()); }
                    r
                }
            }
        }
    });
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { let _ = axum::serve(l, app).await; });
    Stub { base, hits }
}

/// A fresh data directory under the system temp dir. Tests don't clean these up, so the first call per process
/// removes ones left by runs over an hour old (recent ones may belong to a test binary running alongside).
pub fn tmp_data() -> PathBuf {
    static SWEEP: std::sync::Once = std::sync::Once::new();
    SWEEP.call_once(|| {
        let Ok(rd) = std::fs::read_dir(std::env::temp_dir()) else { return };
        for e in rd.flatten() {
            let stale = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|a| a.as_secs() > 3600);
            if stale && e.file_name().to_string_lossy().starts_with("moss-test-") { let _ = std::fs::remove_dir_all(e.path()); }
        }
    });
    let d = std::env::temp_dir().join(format!("moss-test-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Application state on a fresh data directory; `tweak` adjusts the configuration first.
pub async fn state(tweak: impl FnOnce(&mut crate::config::Config)) -> St {
    let mut cfg = crate::config::for_test(&tmp_data());
    tweak(&mut cfg);
    crate::serve::build_state(cfg).await.unwrap()
}
