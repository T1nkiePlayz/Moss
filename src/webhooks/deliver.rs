//! One signed delivery attempt. HTTPS only, no redirects, 10 s for the whole exchange, the destination resolved once,
//! checked and connected to by IP (reusing the notification guard), at most `RESPONSE_CAP` bytes of the answer kept.
//! Errors and logs never contain the URL or the secret.
use super::sign;
use crate::notify::webhook::resolve_pinned;
use anyhow::{Result, anyhow, bail};
use std::time::{Duration, Instant};

pub const TIMEOUT: Duration = Duration::from_secs(10);
pub const RESPONSE_CAP: usize = 2048;

#[derive(Clone, Copy)]
pub struct Opts {
    pub allow_private: bool,
    /// Plain http. Never set outside tests: it lets an in-process receiver stand in for a TLS one.
    pub allow_http: bool,
}

#[derive(Debug, Default, Clone)]
pub struct Outcome {
    pub ok: bool,
    pub status: Option<u16>,
    pub latency_ms: i64,
    pub response: String,
    pub error: Option<String>,
}

fn clean(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).chars().map(|c| if c.is_control() && c != '\n' { ' ' } else { c }).collect::<String>().trim().to_string()
}

async fn send(url: &str, secret: &str, event: &str, delivery: &str, body: &str, o: Opts) -> Result<(u16, String)> {
    let u = reqwest::Url::parse(url).map_err(|_| anyhow!("the webhook URL is not valid"))?;
    if u.scheme() != "https" && !(o.allow_http && u.scheme() == "http") { bail!("webhooks must use https"); }
    let host = u.host_str().ok_or_else(|| anyhow!("the webhook URL has no host"))?.to_string();
    let addr = resolve_pinned(&host, u.port_or_known_default().unwrap_or(443), o.allow_private, "MOSS_ALLOW_PRIVATE_WEBHOOKS").await?;
    let client = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).timeout(TIMEOUT).connect_timeout(Duration::from_secs(5))
        .resolve(&host, addr).user_agent(concat!("Moss/", env!("CARGO_PKG_VERSION"))).build()?;
    let ts = chrono::Utc::now().timestamp();
    let mut r = client.post(u).header("content-type", "application/json").header("x-moss-event", event).header("x-moss-delivery", delivery)
        .header("x-moss-timestamp", ts.to_string()).header("x-moss-signature", sign::signature(secret, ts, body)).body(body.to_string())
        .send().await.map_err(|e| anyhow!("request to {host} failed: {}", if e.is_timeout() { "timed out".to_string() } else { e.without_url().to_string() }))?;
    let status = r.status().as_u16();
    let mut got: Vec<u8> = vec![];
    while got.len() < RESPONSE_CAP {
        match r.chunk().await { Ok(Some(c)) => got.extend_from_slice(&c[..c.len().min(RESPONSE_CAP - got.len())]), _ => break }
    }
    Ok((status, clean(&got)))
}

pub async fn attempt(url: &str, secret: &str, event: &str, delivery: &str, body: &str, o: Opts) -> Outcome {
    let t = Instant::now();
    let res = send(url, secret, event, delivery, body, o).await;
    let latency_ms = t.elapsed().as_millis() as i64;
    match res {
        Ok((s, response)) if (200..300).contains(&s) => Outcome { ok: true, status: Some(s), latency_ms, response, error: None },
        Ok((s, response)) => Outcome { ok: false, status: Some(s), latency_ms, response,
            error: Some(if (300..400).contains(&s) { format!("the receiver answered HTTP {s}; redirects are not followed") } else { format!("the receiver answered HTTP {s}") }) },
        Err(e) => Outcome { ok: false, status: None, latency_ms, response: String::new(), error: Some(e.to_string()) },
    }
}
