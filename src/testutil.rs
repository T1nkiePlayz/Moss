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

/// A fresh data directory under the system temp dir.
pub fn tmp_data() -> PathBuf {
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
