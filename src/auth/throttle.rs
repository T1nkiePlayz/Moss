//! In-memory sliding-window limiter (failed credentials per client address, setup-claim attempts per user)
//! and client-address resolution. Time is passed in so the logic is testable.
use axum::{extract::ConnectInfo, http::HeaderMap};
use std::{collections::{HashMap, VecDeque}, net::SocketAddr, sync::Mutex, time::{Duration, Instant}};

pub const FAIL_LIMIT: usize = 30;
pub const CLAIM_LIMIT: usize = 5;
const WINDOW: Duration = Duration::from_secs(60);
const MAX_KEYS: usize = 50_000;

pub struct Limiter {
    max: usize,
    window: Duration,
    hits: HashMap<String, VecDeque<Instant>>,
    swept: Option<Instant>,
}

impl Limiter {
    pub fn new(max: usize, window: Duration) -> Self { Self { max, window, hits: HashMap::new(), swept: None } }

    fn prune(q: &mut VecDeque<Instant>, now: Instant, window: Duration) {
        while q.front().is_some_and(|t| now.duration_since(*t) >= window) { q.pop_front(); }
    }

    /// Seconds to wait if `key` is already at its limit, else `None`.
    pub fn blocked(&mut self, key: &str, now: Instant) -> Option<u64> {
        let q = self.hits.get_mut(key)?;
        Self::prune(q, now, self.window);
        if q.len() < self.max { return None; }
        let oldest = *q.front()?;
        Some((self.window.saturating_sub(now.duration_since(oldest))).as_secs() + 1)
    }

    /// Record one event for `key`; also performs the periodic sweep of stale keys.
    pub fn record(&mut self, key: &str, now: Instant) {
        if self.swept.is_none_or(|s| now.duration_since(s) >= self.window) { self.sweep(now); }
        if self.hits.len() >= MAX_KEYS && !self.hits.contains_key(key) { self.sweep(now); if self.hits.len() >= MAX_KEYS { self.hits.clear(); } }
        let q = self.hits.entry(key.to_string()).or_default();
        Self::prune(q, now, self.window);
        q.push_back(now);
    }

    /// Count an attempt and refuse it (returning the retry delay) when over the limit.
    pub fn hit(&mut self, key: &str, now: Instant) -> Result<(), u64> {
        if let Some(r) = self.blocked(key, now) { return Err(r); }
        self.record(key, now);
        Ok(())
    }

    pub fn sweep(&mut self, now: Instant) {
        let w = self.window;
        self.hits.retain(|_, q| { Self::prune(q, now, w); !q.is_empty() });
        self.swept = Some(now);
    }
    #[cfg(test)]
    fn len(&self) -> usize { self.hits.len() }
}

/// All limiters, kept in the shared state.
pub struct Guard {
    pub auth_fail: Mutex<Limiter>,
    pub claim: Mutex<Limiter>,
}
impl Default for Guard {
    fn default() -> Self {
        Self { auth_fail: Mutex::new(Limiter::new(FAIL_LIMIT, WINDOW)), claim: Mutex::new(Limiter::new(CLAIM_LIMIT, WINDOW)) }
    }
}

/// Client address: the right-most `X-Forwarded-For` entry (the one our own proxy appended) when
/// `trust_proxy` is set, otherwise the socket peer. Unknown peers share one bucket.
pub fn client_addr(headers: &HeaderMap, peer: Option<&ConnectInfo<SocketAddr>>, trust_proxy: bool) -> String {
    if trust_proxy
        && let Some(v) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok())
        && let Some(last) = v.rsplit(',').next().map(str::trim).filter(|s| !s.is_empty()) {
        return last.to_string();
    }
    peer.map(|c| c.0.ip().to_string()).unwrap_or_else(|| "unknown".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;
    #[test]
    fn blocks_after_limit_and_recovers() {
        let mut l = Limiter::new(3, Duration::from_secs(60));
        let t0 = Instant::now();
        for _ in 0..3 { assert!(l.blocked("a", t0).is_none()); l.record("a", t0); }
        assert!(l.blocked("a", t0 + Duration::from_secs(1)).is_some());
        assert!(l.blocked("b", t0).is_none());
        let r = l.blocked("a", t0 + Duration::from_secs(10)).unwrap();
        assert!((50..=51).contains(&r));
        assert!(l.blocked("a", t0 + Duration::from_secs(61)).is_none());
    }
    #[test]
    fn hit_counts_and_refuses() {
        let mut l = Limiter::new(2, Duration::from_secs(60));
        let t = Instant::now();
        assert!(l.hit("u", t).is_ok() && l.hit("u", t).is_ok());
        assert!(l.hit("u", t).is_err());
        assert!(l.hit("u", t + Duration::from_secs(61)).is_ok());
    }
    #[test]
    fn sweep_drops_stale_keys() {
        let mut l = Limiter::new(2, Duration::from_secs(60));
        let t = Instant::now();
        l.record("a", t); l.record("b", t);
        assert_eq!(l.len(), 2);
        l.record("c", t + Duration::from_secs(120));
        assert_eq!(l.len(), 1);
    }
    #[test]
    fn forwarded_header_only_when_trusted() {
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", HeaderValue::from_static("1.1.1.1, 9.9.9.9"));
        let peer = ConnectInfo(SocketAddr::from(([10, 0, 0, 1], 5555)));
        assert_eq!(client_addr(&h, Some(&peer), false), "10.0.0.1");
        assert_eq!(client_addr(&h, Some(&peer), true), "9.9.9.9");
        assert_eq!(client_addr(&HeaderMap::new(), Some(&peer), true), "10.0.0.1");
        assert_eq!(client_addr(&HeaderMap::new(), None, false), "unknown");
    }
}
