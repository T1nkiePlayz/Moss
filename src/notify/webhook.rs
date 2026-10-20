//! Webhook delivery. HTTPS only, no redirects, 5 s timeouts, and the destination address is resolved once, checked
//! against private/loopback/link-local/CGNAT ranges (unless explicitly allowed) and connected to by IP, so a DNS
//! answer cannot change between the check and the connection. The URL is a secret: it is never logged.
use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(5);
/// Delay before attempt 2 and 3.
const BACKOFF: [Duration; 2] = [Duration::from_secs(2), Duration::from_secs(6)];

fn public_v4(a: Ipv4Addr) -> bool {
    let o = a.octets();
    !(a.is_unspecified() || a.is_loopback() || a.is_private() || a.is_link_local() || a.is_broadcast() || a.is_multicast() || a.is_documentation()
        || o[0] == 0                                        // 0.0.0.0/8
        || (o[0] == 100 && (64..128).contains(&o[1]))       // 100.64.0.0/10 carrier-grade NAT
        || (o[0] == 192 && o[1] == 0 && o[2] == 0)          // 192.0.0.0/24 IETF protocol assignments
        || (o[0] == 198 && (o[1] == 18 || o[1] == 19))      // 198.18.0.0/15 benchmarking
        || o[0] >= 240)                                     // 240.0.0.0/4 reserved
}

/// True when the address is a globally routable unicast address.
pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(a) => public_v4(a),
        IpAddr::V6(a) => {
            if let Some(v4) = a.to_ipv4_mapped() { return public_v4(v4); }
            let s = a.segments();
            if a.is_unspecified() || a.is_loopback() || a.is_multicast() { return false; }
            if s[0] & 0xfe00 == 0xfc00 || s[0] & 0xffc0 == 0xfe80 || s[0] & 0xffc0 == 0xfec0 { return false; } // ULA, link-local, site-local
            if s[0] == 0x2001 && s[1] == 0x0db8 { return false; } // documentation
            let embedded = |hi: u16, lo: u16| Ipv4Addr::new((hi >> 8) as u8, hi as u8, (lo >> 8) as u8, lo as u8);
            if s[0] == 0x0064 && s[1] == 0xff9b && s[2..6] == [0; 4] { return public_v4(embedded(s[6], s[7])); } // NAT64
            if s[0] == 0x0064 && s[1] == 0xff9b && s[2] == 1 { return false; } // 64:ff9b:1::/48 local-use NAT64 (RFC 8215)
            if s[0] == 0x2002 { return public_v4(embedded(s[1], s[2])); } // 6to4
            if s[0..5] == [0; 5] && s[5] == 0 { return false; } // deprecated IPv4-compatible
            true
        }
    }
}

pub fn is_discord(u: &reqwest::Url) -> bool {
    let h = u.host_str().unwrap_or("");
    (h == "discord.com" || h == "discordapp.com" || h.ends_with(".discord.com") || h.ends_with(".discordapp.com")) && u.path().starts_with("/api/webhooks/")
}

/// The JSON sent for an event: a Discord embed for Discord webhooks, a generic object otherwise.
pub fn build_payload(url: &reqwest::Url, event: &str, severity: &str, title: &str, body: &str, server: Option<&str>, time: &str) -> Value {
    if is_discord(url) {
        let color = match severity { "critical" => 0xE5484D, "warn" => 0xF5A524, _ => 0x3E9B5A };
        let mut e = json!({"title": title.chars().take(256).collect::<String>(), "description": body.chars().take(2000).collect::<String>(), "color": color, "timestamp": time,
            "footer": {"text": format!("Moss {}", env!("CARGO_PKG_VERSION"))}});
        if let Some(s) = server { e["fields"] = json!([{"name": "Server", "value": s.chars().take(200).collect::<String>(), "inline": true}]); }
        json!({"username": "Moss", "embeds": [e]})
    } else {
        json!({"event": event, "severity": severity, "title": title, "body": body, "server": server, "time": time, "moss": env!("CARGO_PKG_VERSION")})
    }
}

/// Resolve the host once and return the single address to connect to. `opt_in` names the setting that allows private targets.
pub async fn resolve_pinned(host: &str, port: u16, allow_private: bool, opt_in: &str) -> Result<SocketAddr> {
    let addrs: Vec<SocketAddr> = match host.trim_matches(['[', ']']).parse::<IpAddr>() {
        Ok(ip) => vec![SocketAddr::new(ip, port)],
        Err(_) => tokio::time::timeout(TIMEOUT, tokio::net::lookup_host((host, port))).await.map_err(|_| anyhow!("DNS lookup for {host} timed out"))?.map_err(|_| anyhow!("could not resolve {host}"))?.collect(),
    };
    if addrs.is_empty() { bail!("could not resolve {host}"); }
    if !allow_private && let Some(bad) = addrs.iter().find(|a| !is_public_ip(a.ip())) {
        bail!("{host} resolves to a non-public address ({}); set {opt_in}=true to allow this", bad.ip());
    }
    Ok(addrs[0])
}

async fn pinned_addr(u: &reqwest::Url, allow_private: bool) -> Result<SocketAddr> {
    let host = u.host_str().ok_or_else(|| anyhow!("webhook URL has no host"))?;
    resolve_pinned(host, u.port_or_known_default().unwrap_or(443), allow_private, "MOSS_ALLOW_PRIVATE_WEBHOOKS").await
}

/// One delivery attempt. Errors never contain the URL.
pub async fn attempt(url: &str, payload: &Value, allow_private: bool) -> Result<String> {
    let u = reqwest::Url::parse(url).map_err(|_| anyhow!("the webhook URL is not valid"))?;
    if u.scheme() != "https" { bail!("webhooks must use https"); }
    let addr = pinned_addr(&u, allow_private).await?;
    let host = u.host_str().unwrap_or_default().to_string();
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none()).timeout(TIMEOUT).connect_timeout(TIMEOUT)
        .resolve(&host, addr).user_agent(concat!("Moss/", env!("CARGO_PKG_VERSION"))).build()?;
    let r = client.post(u).json(payload).send().await.map_err(|e| anyhow!("request to {host} failed: {}", e.without_url()))?;
    let status = r.status();
    if status.is_success() { Ok(format!("delivered to {host} (HTTP {})", status.as_u16())) }
    else if status.is_redirection() { bail!("{host} answered with a redirect (HTTP {}); redirects are not followed", status.as_u16()) }
    else { bail!("{host} answered HTTP {}", status.as_u16()) }
}

/// Up to `attempts` tries with backoff; returns the last outcome as `(delivered, detail)`.
pub async fn deliver(url: &str, payload: &Value, allow_private: bool, attempts: usize) -> (bool, String) {
    let mut last = String::new();
    for i in 0..attempts.clamp(1, 3) {
        if i > 0 { tokio::time::sleep(BACKOFF[i - 1]).await; }
        match attempt(url, payload, allow_private).await {
            Ok(m) => return (true, m),
            Err(e) => last = e.to_string(),
        }
    }
    (false, last)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_use_nat64_prefix_is_not_public() {
        assert!(!is_public_ip("64:ff9b:1::7f00:1".parse().unwrap()));
        assert!(is_public_ip("64:ff9b::808:808".parse().unwrap()));
        assert!(!is_public_ip("64:ff9b::7f00:1".parse().unwrap()));
    }

    fn ip(s: &str) -> IpAddr { s.parse().unwrap() }

    #[test]
    fn ssrf_ranges_are_refused() {
        for bad in ["127.0.0.1", "10.1.2.3", "172.16.0.1", "172.31.255.255", "192.168.1.1", "169.254.169.254", "100.64.0.1", "100.127.255.255", "0.0.0.0", "0.1.2.3",
            "255.255.255.255", "224.0.0.1", "240.0.0.1", "198.18.0.1", "192.0.0.8", "192.0.2.1", "::1", "::", "fe80::1", "fc00::1", "fd12:3456::1", "ff02::1", "::ffff:127.0.0.1",
            "::ffff:10.0.0.1", "::ffff:169.254.169.254", "64:ff9b::7f00:1", "2002:7f00:1::1", "2001:db8::1", "fec0::1"] {
            assert!(!is_public_ip(ip(bad)), "{bad} must be refused");
        }
    }

    #[test]
    fn public_addresses_are_allowed() {
        for good in ["8.8.8.8", "1.1.1.1", "172.15.0.1", "172.32.0.1", "100.63.255.255", "100.128.0.1", "93.184.216.34", "2606:4700:4700::1111", "::ffff:8.8.8.8", "2002:808:808::1"] {
            assert!(is_public_ip(ip(good)), "{good} must be allowed");
        }
    }

    #[test]
    fn discord_gets_an_embed_and_others_the_generic_shape() {
        let d = reqwest::Url::parse("https://discord.com/api/webhooks/1/tok").unwrap();
        let p = build_payload(&d, "crash", "critical", "Survival crashed", "exit code 1", Some("Survival"), "2026-01-01T00:00:00Z");
        assert_eq!(p["embeds"][0]["title"], "Survival crashed");
        assert_eq!(p["embeds"][0]["color"], 0xE5484D);
        assert_eq!(p["embeds"][0]["fields"][0]["value"], "Survival");
        assert!(p.get("event").is_none());
        let g = reqwest::Url::parse("https://example.com/hook").unwrap();
        let p = build_payload(&g, "backup_failed", "warn", "T", "B", None, "2026-01-01T00:00:00Z");
        assert_eq!(p, json!({"event": "backup_failed", "severity": "warn", "title": "T", "body": "B", "server": null, "time": "2026-01-01T00:00:00Z", "moss": env!("CARGO_PKG_VERSION")}));
        assert!(!is_discord(&reqwest::Url::parse("https://discord.com.evil.io/api/webhooks/1/x").unwrap()));
        assert!(!is_discord(&reqwest::Url::parse("https://discord.com/other").unwrap()));
    }

    #[tokio::test]
    async fn delivery_refuses_http_and_private_targets_without_connecting() {
        let p = json!({});
        let e = attempt("http://example.com/x", &p, false).await.unwrap_err().to_string();
        assert!(e.contains("https"));
        let e = attempt("https://127.0.0.1:9/secret-path", &p, false).await.unwrap_err().to_string();
        assert!(e.contains("non-public") && !e.contains("secret-path"));
        let e = attempt("https://[::1]/x", &p, false).await.unwrap_err().to_string();
        assert!(e.contains("non-public"));
        // with the override the address is accepted (the connection itself then fails: nothing listens on port 9)
        let e = attempt("https://127.0.0.1:9/secret-path", &p, true).await.unwrap_err().to_string();
        assert!(!e.contains("non-public") && !e.contains("secret-path"), "{e}");
    }
}
