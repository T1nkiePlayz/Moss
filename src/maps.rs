//! Live web maps (BlueMap, squaremap, Pl3xMap, dynmap): detection of the plugin and its web port, the signed
//! capability tokens that authorise the map proxy, and the pure helpers the proxy handler uses (path normalisation,
//! header filtering). The proxy target is only ever `http://127.0.0.1:<detected port>`; nothing user-supplied picks a host.
use crate::{auth::secret, server::Srv, state::Inner};
use axum::http::{HeaderMap, HeaderName, HeaderValue, header};
use serde_json::{Value, json};
use std::{path::Path, time::Duration};

/// How long a freshly minted map link works.
pub const TOKEN_TTL_SECS: i64 = 12 * 3600;
/// Public status pages mint shorter links, and the page reloads before they run out.
pub const PUBLIC_TOKEN_TTL_SECS: i64 = 2 * 3600;
const DETECT_TTL: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapPlugin { pub kind: &'static str, pub port: u16 }

// ---- detection --------------------------------------------------------------------------------------------------

/// Plugins in `plugins/` as (map kind, lowercase name). A directory or a jar named like it counts.
const PLUGINS: &[(&str, &str)] = &[("bluemap", "bluemap"), ("squaremap", "squaremap"), ("pl3xmap", "pl3xmap"), ("dynmap", "dynmap")];

fn is_plugin_name(entry: &str, prefix: &str) -> bool {
    entry.strip_prefix(prefix).is_some_and(|rest| rest.is_empty() || rest == ".jar" || (rest.starts_with(['-', '_', ' ']) && rest.ends_with(".jar")))
}

/// Which map plugin the server directory holds, with the web port it is configured for. One directory listing plus at
/// most one small config read. A plugin whose own web server is switched off has no map to show.
pub fn detect_in(server_dir: &Path) -> Option<MapPlugin> {
    let plugins = server_dir.join("plugins");
    let entries: Vec<(String, bool, std::path::PathBuf)> = std::fs::read_dir(&plugins).ok()?.flatten()
        .map(|e| (e.file_name().to_string_lossy().to_lowercase(), e.file_type().is_ok_and(|t| t.is_dir()), e.path())).collect();
    for (kind, prefix) in PLUGINS {
        if !entries.iter().any(|(n, _, _)| is_plugin_name(n, prefix)) { continue; }
        let dir = entries.iter().find(|(n, is_dir, _)| *is_dir && n == prefix).map(|e| e.2.clone());
        let read = |file: &str| dir.as_ref().and_then(|d| std::fs::read_to_string(d.join(file)).ok());
        let port = match *kind {
            "bluemap" => match read("webserver.conf") { Some(t) => hocon_port(&t)?, None => 8100 },
            "squaremap" | "pl3xmap" => match read("config.yml") { Some(t) => yaml_internal_port(&t)?, None => 8080 },
            _ => match read("configuration.txt") { Some(t) => dynmap_port(&t)?, None => 8123 },
        };
        return Some(MapPlugin { kind, port });
    }
    None
}

/// Unprivileged ports only, so a typo cannot point the proxy at a system service.
fn valid_port(s: &str) -> Option<u16> { s.trim().trim_matches(['"', '\'']).parse::<u16>().ok().filter(|p| *p >= 1024) }

/// BlueMap `webserver.conf` (HOCON): `port: 8100` (or `port = 8100`). `enabled: false` means no built-in web server.
fn hocon_port(t: &str) -> Option<u16> {
    let (mut port, mut enabled) = (8100, true);
    for l in t.lines() {
        let l = l.split('#').next().unwrap_or("").trim();
        let Some((k, v)) = l.split_once([':', '=']) else { continue };
        match k.trim() { "port" => port = valid_port(v.trim_end_matches(',')).unwrap_or(port), "enabled" => enabled = v.trim() != "false", _ => {} }
    }
    enabled.then_some(port)
}

/// squaremap / Pl3xMap `config.yml`: `settings.internal-webserver.{enabled,port}`; the bind address is ignored because
/// the proxy only ever dials loopback.
fn yaml_internal_port(t: &str) -> Option<u16> {
    let (mut port, mut enabled) = (8080, true);
    let mut stack: Vec<(usize, String)> = vec![];
    for l in t.lines() {
        let body = l.split(" #").next().unwrap_or("");
        let trimmed = body.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') { continue; }
        let indent = body.len() - body.trim_start().len();
        let Some((k, v)) = trimmed.split_once(':') else { continue };
        while stack.last().is_some_and(|(i, _)| *i >= indent) { stack.pop(); }
        stack.push((indent, k.trim().to_string()));
        if stack.len() == 3 && stack[0].1 == "settings" && stack[1].1 == "internal-webserver" {
            match stack[2].1.as_str() { "port" => port = valid_port(v).unwrap_or(port), "enabled" => enabled = v.trim() != "false", _ => {} }
        }
    }
    enabled.then_some(port)
}

/// dynmap `configuration.txt`: top-level `webserver-port: 8123`; `disable-webserver: true` turns the built-in server off.
fn dynmap_port(t: &str) -> Option<u16> {
    let (mut port, mut disabled) = (8123, false);
    for l in t.lines() {
        if l.starts_with([' ', '\t', '#']) { continue; }
        let Some((k, v)) = l.split_once(':') else { continue };
        match k.trim() { "webserver-port" => port = valid_port(v).unwrap_or(port), "disable-webserver" => disabled = v.trim() == "true", _ => {} }
    }
    (!disabled).then_some(port)
}

/// `detect_in` for a server, cached for 30 seconds (the server view is built often). Only Minecraft Java servers qualify.
/// Whether the map port really belongs to this server. The port comes from files anyone with file access can edit, so
/// without this check `/map/...` could be pointed at any other loopback service. On Linux the listening socket must be
/// held by the server's process tree (`/proc/net/tcp*` inode matched against `/proc/<pid>/fd`); where `/proc` is not
/// available, the port must at least not belong to another Moss server.
pub fn port_belongs_to(s: &Srv, port: u16, others: &[Srv]) -> bool {
    match listening_inodes(port) {
        Some(inodes) => {
            let Some(root) = s.pid.filter(|p| *p > 0) else { return false };
            if inodes.is_empty() { return false; }
            let mut pids = crate::games::descendants(root, &crate::games::proc_parents());
            pids.push(root);
            pids.iter().any(|pid| holds_socket(*pid, &inodes))
        }
        None => !crate::games::clashes_with_span(i64::from(port), 0, &s.id, others),
    }
}

/// Inodes of sockets listening on `port` (any address), or `None` when `/proc/net/tcp` cannot be read (not Linux).
fn listening_inodes(port: u16) -> Option<Vec<u64>> {
    let v4 = std::fs::read_to_string("/proc/net/tcp").ok()?;
    let v6 = std::fs::read_to_string("/proc/net/tcp6").unwrap_or_default();
    Some(parse_listeners(&format!("{v4}\n{v6}"), port))
}

fn parse_listeners(table: &str, port: u16) -> Vec<u64> {
    table.lines().filter_map(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() < 10 || f[3] != "0A" { return None; } // 0A = LISTEN
        let p = u16::from_str_radix(f[1].rsplit(':').next()?, 16).ok()?;
        (p == port).then(|| f[9].parse().ok()).flatten()
    }).collect()
}

fn holds_socket(pid: i64, inodes: &[u64]) -> bool {
    let Ok(rd) = std::fs::read_dir(format!("/proc/{pid}/fd")) else { return false };
    rd.flatten().any(|e| std::fs::read_link(e.path()).ok().and_then(|t| t.to_str()?.strip_prefix("socket:[")?.strip_suffix(']')?.parse::<u64>().ok()).is_some_and(|i| inodes.contains(&i)))
}

pub fn detect(st: &Inner, s: &Srv) -> Option<MapPlugin> {
    if !s.is_mc() || s.is_proxy() { return None; }
    let key = format!("map:{}", s.id);
    if let Some((at, v)) = st.cache.lock().unwrap().get(&key) && at.elapsed() < DETECT_TTL { return from_json(v); }
    let found = detect_in(&s.dir(st));
    st.cache.lock().unwrap().insert(key, (std::time::Instant::now(), found.map_or(Value::Null, |m| json!({"kind": m.kind, "port": m.port}))));
    found
}

fn from_json(v: &Value) -> Option<MapPlugin> {
    let kind = PLUGINS.iter().map(|p| p.0).find(|k| v["kind"] == *k)?;
    Some(MapPlugin { kind, port: u16::try_from(v["port"].as_u64()?).ok()? })
}

// ---- capability tokens ------------------------------------------------------------------------------------------

/// Who a token was minted for. A user link names its holder, so the proxy re-checks that user's access on every request
/// (removing a grant or the user ends the link at once); public tokens go through the public rate limiter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope { User(String), Public }

/// The signing key lives in `<data dir>/map.key` (64 random characters, owner-only), created on first use.
fn key(st: &Inner) -> Vec<u8> {
    let path = st.cfg.data_dir.join("map.key");
    let stored = |p: &Path| std::fs::read_to_string(p).ok().filter(|t| t.trim().len() == 64).map(|t| t.trim().as_bytes().to_vec());
    if let Some(k) = stored(&path) { return k; }
    let fresh = secret::random_string(64);
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    { use std::os::unix::fs::OpenOptionsExt; o.mode(0o600); }
    if let Ok(mut f) = o.open(&path) { use std::io::Write; let _ = f.write_all(fresh.as_bytes()); return fresh.into_bytes(); }
    // lost a creation race (or the disk is read-only): prefer whatever is on disk, else a key that lives for this call only
    stored(&path).unwrap_or_else(|| fresh.into_bytes())
}

fn mac(key: &[u8], server_id: &str, exp: i64, scope: &Scope) -> String {
    let msg = match scope {
        Scope::Public => format!("moss-map-v1\n{server_id}\n{exp}\np"),
        // v2: user links issued before holders were bound (`<exp>.u.<mac>`) no longer verify
        Scope::User(uid) => format!("moss-map-v2\n{server_id}\n{exp}\nu\n{uid}"),
    };
    hex::encode(crate::webhooks::sign::hmac_sha256(key, msg.as_bytes()))
}

/// `<expiry>.p.<mac>` or `<expiry>.u.<hex user id>.<mac>` (HMAC-SHA256 under the server key over server id, expiry,
/// scope and holder), safe inside a URL path.
pub fn mint(st: &Inner, server_id: &str, scope: Scope, ttl_secs: i64) -> (String, i64) {
    let exp = chrono::Utc::now().timestamp() + ttl_secs;
    (mint_with(&key(st), server_id, &scope, exp), exp)
}

fn mint_with(key: &[u8], server_id: &str, scope: &Scope, exp: i64) -> String {
    let m = mac(key, server_id, exp, scope);
    match scope { Scope::Public => format!("{exp}.p.{m}"), Scope::User(uid) => format!("{exp}.u.{}.{m}", hex::encode(uid)) }
}

/// The scope of a valid, unexpired token for exactly this server. A user scope still has to pass that user's access check.
pub fn verify(st: &Inner, server_id: &str, token: &str) -> Option<Scope> { verify_with(&key(st), server_id, token, chrono::Utc::now().timestamp()) }

fn verify_with(key: &[u8], server_id: &str, token: &str, now: i64) -> Option<Scope> {
    let p: Vec<&str> = token.split('.').collect();
    let (exp, scope, given) = match p.as_slice() {
        [exp, "p", given] => (exp, Scope::Public, given),
        [exp, "u", holder, given] => (exp, Scope::User(String::from_utf8(hex::decode(holder).ok()?).ok().filter(|u| !u.is_empty())?), given),
        _ => return None,
    };
    let exp: i64 = exp.parse().ok()?;
    if !secret::ct_eq(mac(key, server_id, exp, &scope).as_bytes(), given.as_bytes()) || exp < now { return None; }
    Some(scope)
}

// ---- request shaping --------------------------------------------------------------------------------------------

/// Turn the part of the request path after `/map/{id}/{token}` into a safe upstream path (always starting with `/`,
/// keeping a trailing slash). Empty segments collapse; `.`/`..`, encoded separators, backslashes and control characters
/// are refused, so the result can never leave the map's own web root or name another host.
pub fn normalise_path(rest: &str) -> Option<String> {
    let mut out = String::from("/");
    for s in rest.split('/').filter(|s| !s.is_empty()) {
        let dec = percent_decode(s)?;
        if dec == "." || dec == ".." || dec.contains(['/', '\\']) || dec.chars().any(|c| c.is_control()) { return None; }
        if !s.bytes().all(|b| b.is_ascii_alphanumeric() || b"-._~%!$&'()*+,;=:@".contains(&b)) { return None; }
        out.push_str(s);
        out.push('/');
    }
    if !rest.ends_with('/') && out.len() > 1 { out.pop(); }
    Some(out)
}

fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let (mut out, mut i) = (Vec::with_capacity(b.len()), 0);
    while i < b.len() {
        if b[i] == b'%' {
            let h = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(h, 16).ok()?);
            i += 3;
        } else { out.push(b[i]); i += 1; }
    }
    String::from_utf8(out).ok()
}

/// The only upstream URL the proxy ever builds.
pub fn upstream_url(port: u16, path: &str, query: Option<&str>) -> String {
    match query { Some(q) if !q.is_empty() => format!("http://127.0.0.1:{port}{path}?{q}"), _ => format!("http://127.0.0.1:{port}{path}") }
}

/// The request headers passed on to the map: a small allow-list. Cookies, Authorization, Host, Origin and the rest stay behind.
pub fn forward_request_headers(h: &HeaderMap) -> Vec<(HeaderName, HeaderValue)> {
    const ALLOW: [HeaderName; 5] = [header::ACCEPT, header::ACCEPT_ENCODING, header::RANGE, header::IF_NONE_MATCH, header::IF_MODIFIED_SINCE];
    ALLOW.iter().filter_map(|n| h.get(n).map(|v| (n.clone(), v.clone()))).collect()
}

/// Upstream response headers worth keeping. Set-Cookie, CSP, CORS, framing and hop-by-hop headers are dropped; the
/// proxy then adds its own (see `harden`). A relative or same-host `Location` is kept under the proxy prefix.
pub fn filter_response_headers(h: &HeaderMap, prefix: &str) -> HeaderMap {
    let mut out = HeaderMap::new();
    for (n, v) in h {
        let name = n.as_str();
        let drop = matches!(name, "set-cookie" | "set-cookie2" | "content-security-policy" | "content-security-policy-report-only" | "x-frame-options" | "x-content-type-options"
            | "referrer-policy" | "permissions-policy" | "clear-site-data" | "strict-transport-security" | "public-key-pins" | "alt-svc" | "link" | "refresh"
            | "connection" | "keep-alive" | "proxy-authenticate" | "proxy-authorization" | "te" | "trailer" | "transfer-encoding" | "upgrade" | "server" | "location" | "content-location"
            | "x-powered-by" | "cross-origin-resource-policy" | "cross-origin-opener-policy" | "cross-origin-embedder-policy")
            || name.starts_with("access-control-");
        if !drop { out.append(n.clone(), v.clone()); }
    }
    if let Some(l) = h.get(header::LOCATION).and_then(|v| v.to_str().ok()).and_then(|l| rewrite_location(l, prefix)) && let Ok(v) = HeaderValue::from_str(&l) { out.insert(header::LOCATION, v); }
    out
}

/// Redirects stay inside the proxy: `/x` and `http://127.0.0.1:<any>/x` become `<prefix>/x`; relative stays relative;
/// anything pointing elsewhere is dropped.
fn rewrite_location(l: &str, prefix: &str) -> Option<String> {
    if l.chars().any(|c| c.is_control()) { return None; }
    if let Some(r) = l.strip_prefix("http://127.0.0.1").or_else(|| l.strip_prefix("http://localhost")) {
        let r = r.trim_start_matches(|c: char| c == ':' || c.is_ascii_digit());
        return (r.is_empty() || (r.starts_with('/') && !r.starts_with("//"))).then(|| format!("{prefix}{}", if r.is_empty() { "/" } else { r }));
    }
    if l.starts_with("//") || l.contains("://") || l.starts_with('\\') { return None; }
    if l.starts_with('/') { return Some(format!("{prefix}{l}")); }
    Some(l.to_string())
}

/// Headers every proxied response carries, whatever the plugin sent. The CSP sandbox gives the page an opaque origin
/// even when opened as a top-level document, so plugin JavaScript never runs with Moss's origin.
pub fn harden(h: &mut HeaderMap) {
    let set = |h: &mut HeaderMap, n: HeaderName, v: &'static str| { h.insert(n, HeaderValue::from_static(v)); };
    set(h, header::CONTENT_SECURITY_POLICY, "sandbox allow-scripts allow-popups");
    set(h, header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    set(h, header::REFERRER_POLICY, "no-referrer");
    set(h, header::ACCESS_CONTROL_ALLOW_ORIGIN, "*");
    set(h, HeaderName::from_static("cross-origin-resource-policy"), "cross-origin");
    set(h, header::CACHE_CONTROL, "private, no-cache");
}

/// `/map/{id}/{token}/`, the base path of a server's proxied map, and when it stops working (unix seconds).
pub fn link(st: &Inner, server_id: &str, scope: Scope, ttl_secs: i64) -> (String, i64) {
    let (token, exp) = mint(st, server_id, scope, ttl_secs);
    (format!("/map/{server_id}/{token}/"), exp)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir_with(files: &[(&str, &str)]) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("moss-maps-{}", uuid::Uuid::now_v7()));
        for (p, c) in files { let f = d.join(p); std::fs::create_dir_all(f.parent().unwrap()).unwrap(); std::fs::write(f, c).unwrap(); }
        d
    }

    #[test]
    fn bluemap_port_comes_from_webserver_conf_or_defaults() {
        let d = dir_with(&[("plugins/BlueMap/webserver.conf", "enabled: true\n# port: 1\nip: \"0.0.0.0\"\nport: 8200\n")]);
        assert_eq!(detect_in(&d), Some(MapPlugin { kind: "bluemap", port: 8200 }));
        let d = dir_with(&[("plugins/BlueMap/core.conf", "x")]);
        assert_eq!(detect_in(&d), Some(MapPlugin { kind: "bluemap", port: 8100 }));
        let d = dir_with(&[("plugins/BlueMap/webserver.conf", "enabled: false\nport: 8200\n")]);
        assert_eq!(detect_in(&d), None, "built-in web server off");
    }

    #[test]
    fn squaremap_and_pl3xmap_read_the_internal_webserver_port() {
        let cfg = "settings:\n  web-address: http://localhost:8080\n  internal-webserver:\n    enabled: true\n    bind: 0.0.0.0\n    port: 8085\n  other:\n    port: 9999\n";
        let d = dir_with(&[("plugins/squaremap/config.yml", cfg)]);
        assert_eq!(detect_in(&d), Some(MapPlugin { kind: "squaremap", port: 8085 }));
        let d = dir_with(&[("plugins/Pl3xMap/config.yml", cfg)]);
        assert_eq!(detect_in(&d), Some(MapPlugin { kind: "pl3xmap", port: 8085 }));
        let d = dir_with(&[("plugins/squaremap/config.yml", "settings:\n  internal-webserver:\n    enabled: false\n    port: 8085\n")]);
        assert_eq!(detect_in(&d), None);
        let d = dir_with(&[("plugins/Pl3xMap/placeholder", "")]);
        assert_eq!(detect_in(&d), Some(MapPlugin { kind: "pl3xmap", port: 8080 }));
    }

    #[test]
    fn dynmap_port_comes_from_configuration_txt() {
        let d = dir_with(&[("plugins/dynmap/configuration.txt", "# webserver-port: 1\nweb:\n  webserver-port: 3000\nwebserver-port: 8124\ndisable-webserver: false\n")]);
        assert_eq!(detect_in(&d), Some(MapPlugin { kind: "dynmap", port: 8124 }));
        let d = dir_with(&[("plugins/dynmap/configuration.txt", "disable-webserver: true\n")]);
        assert_eq!(detect_in(&d), None);
        let d = dir_with(&[("plugins/dynmap-3.7.jar", "")]);
        assert_eq!(detect_in(&d), Some(MapPlugin { kind: "dynmap", port: 8123 }), "a jar counts, defaults apply");
    }

    #[test]
    fn no_plugin_no_map_and_privileged_ports_are_ignored() {
        assert_eq!(detect_in(&dir_with(&[("plugins/Essentials/config.yml", "")])), None);
        assert_eq!(detect_in(&dir_with(&[("server.properties", "")])), None);
        assert_eq!(detect_in(&dir_with(&[("plugins/BlueMap/webserver.conf", "port: 80\n")])), Some(MapPlugin { kind: "bluemap", port: 8100 }));
        assert_eq!(detect_in(&dir_with(&[("plugins/NotBlueMapAddon/x", "")])), None);
        assert_eq!(detect_in(&dir_with(&[("plugins/bluemapaddon.jar", "")])), None);
    }

    #[test]
    fn tokens_expire_and_bind_to_the_server() {
        let k = b"k".repeat(32);
        let u = || Scope::User("user-1".into());
        let t = mint_with(&k, "srv-a", &u(), 1_000);
        assert_eq!(verify_with(&k, "srv-a", &t, 999), Some(u()));
        assert_eq!(verify_with(&k, "srv-a", &t, 1_000), Some(u()));
        assert_eq!(verify_with(&k, "srv-a", &t, 1_001), None, "expired");
        assert_eq!(verify_with(&k, "srv-b", &t, 0), None, "wrong server");
        assert_eq!(verify_with(&b"x".repeat(32), "srv-a", &t, 0), None, "wrong key");
        let p = mint_with(&k, "srv-a", &Scope::Public, 1_000);
        assert_eq!(verify_with(&k, "srv-a", &p, 0), Some(Scope::Public));
    }

    #[test]
    fn tampered_tokens_are_refused() {
        let k = b"k".repeat(32);
        let t = mint_with(&k, "s", &Scope::User("user-1".into()), 1_000);
        let (_, rest) = t.split_once('.').unwrap();
        assert_eq!(verify_with(&k, "s", &format!("9999999999.{rest}"), 0), None, "extended expiry");
        let m = t.rsplit('.').next().unwrap();
        assert_eq!(verify_with(&k, "s", &format!("1000.p.{m}"), 0), None, "escalated scope");
        assert_eq!(verify_with(&k, "s", &t.replace(&hex::encode("user-1"), &hex::encode("user-2")), 0), None, "another holder");
        // links minted before holders were bound stop working
        let v1 = format!("1000.u.{}", hex::encode(crate::webhooks::sign::hmac_sha256(&k, b"moss-map-v1\ns\n1000\nu")));
        assert_eq!(verify_with(&k, "s", &v1, 0), None, "unbound v1 user link");
        let mut flipped = t.clone();
        flipped.pop();
        flipped.push(if t.ends_with('0') { '1' } else { '0' });
        assert_eq!(verify_with(&k, "s", &flipped, 0), None);
        for bad in ["", ".", "..", "1.u", "1.u.", "x.u.ab", &format!("{t}.extra"), "-1.u.00", "1.u..00", "1.u.zz.00"] { assert_eq!(verify_with(&k, "s", bad, 0), None, "{bad}"); }
    }

    #[tokio::test]
    async fn the_key_is_stable_across_calls_and_private() {
        let st = crate::testutil::state(|_| {}).await;
        let (t, _) = mint(&st, "s1", Scope::User("u".into()), 60);
        assert_eq!(verify(&st, "s1", &t), Some(Scope::User("u".into())));
        assert_eq!(verify(&st, "s2", &t), None);
        let (expired, _) = mint(&st, "s1", Scope::User("u".into()), -5);
        assert_eq!(verify(&st, "s1", &expired), None);
        #[cfg(unix)]
        { use std::os::unix::fs::PermissionsExt; assert_eq!(std::fs::metadata(st.cfg.data_dir.join("map.key")).unwrap().permissions().mode() & 0o777, 0o600); }
    }

    #[test]
    fn paths_are_normalised_and_traversal_refused() {
        assert_eq!(normalise_path("").as_deref(), Some("/"));
        assert_eq!(normalise_path("/").as_deref(), Some("/"));
        assert_eq!(normalise_path("tiles/0/1.png").as_deref(), Some("/tiles/0/1.png"));
        assert_eq!(normalise_path("//a///b/").as_deref(), Some("/a/b/"));
        assert_eq!(normalise_path("maps/my%20world/x.json").as_deref(), Some("/maps/my%20world/x.json"));
        assert_eq!(normalise_path("//evil.com").as_deref(), Some("/evil.com"), "a host-looking path is just a path on the map server");
        for bad in ["..", "a/../b", "a/./b", "%2e%2e/x", "a/%2E%2e/b", "a%2fb", "a%5Cb", "a\\b", "a/%00", "%zz", "a/%0a", "a b", "a/é", "a?b", "a#b"] {
            assert_eq!(normalise_path(bad), None, "{bad}");
        }
    }

    #[test]
    fn the_upstream_url_is_always_loopback_on_the_detected_port() {
        for p in ["/", "/a/b", "/evil.com", "/@evil.com", "/127.0.0.1:9", "/http:/evil"] {
            let u = reqwest::Url::parse(&upstream_url(8100, p, Some("x=1"))).unwrap();
            assert_eq!((u.scheme(), u.host_str(), u.port()), ("http", Some("127.0.0.1"), Some(8100)), "{p}");
        }
        assert_eq!(upstream_url(1, "/a", None), "http://127.0.0.1:1/a");
    }

    #[test]
    fn only_allow_listed_request_headers_are_forwarded() {
        let mut h = HeaderMap::new();
        for (k, v) in [("cookie", "s=1"), ("authorization", "Bearer x"), ("host", "moss"), ("origin", "null"), ("range", "bytes=0-9"), ("accept", "*/*"), ("accept-encoding", "gzip"), ("if-none-match", "\"a\""), ("if-modified-since", "x"), ("x-forwarded-for", "1.2.3.4")] {
            h.insert(HeaderName::from_static(k), HeaderValue::from_static(v));
        }
        let mut names: Vec<_> = forward_request_headers(&h).into_iter().map(|(n, _)| n.to_string()).collect();
        names.sort();
        assert_eq!(names, ["accept", "accept-encoding", "if-modified-since", "if-none-match", "range"]);
    }

    #[test]
    fn upstream_security_headers_are_replaced() {
        let mut up = HeaderMap::new();
        for (k, v) in [("set-cookie", "a=b"), ("content-security-policy", "default-src *"), ("access-control-allow-origin", "https://x"), ("access-control-allow-credentials", "true"),
            ("x-frame-options", "SAMEORIGIN"), ("content-type", "text/html"), ("etag", "\"1\""), ("content-encoding", "gzip"), ("location", "http://127.0.0.1:8100/x"), ("transfer-encoding", "chunked")] {
            up.insert(HeaderName::from_static(k), HeaderValue::from_static(v));
        }
        let mut h = filter_response_headers(&up, "/map/s/t");
        harden(&mut h);
        assert!(h.get("set-cookie").is_none() && h.get("access-control-allow-credentials").is_none() && h.get("transfer-encoding").is_none() && h.get("x-frame-options").is_none());
        assert_eq!(h["content-security-policy"], "sandbox allow-scripts allow-popups");
        assert_eq!(h["access-control-allow-origin"], "*");
        assert_eq!(h["x-content-type-options"], "nosniff");
        assert_eq!((h["content-type"].to_str().unwrap(), h["etag"].to_str().unwrap(), h["content-encoding"].to_str().unwrap()), ("text/html", "\"1\"", "gzip"));
        assert_eq!(h["location"], "/map/s/t/x");
    }

    #[test]
    fn redirects_cannot_leave_the_proxy() {
        let p = "/map/s/t";
        assert_eq!(rewrite_location("/a", p).as_deref(), Some("/map/s/t/a"));
        assert_eq!(rewrite_location("http://127.0.0.1:8100/a?b=1", p).as_deref(), Some("/map/s/t/a?b=1"));
        assert_eq!(rewrite_location("index.html", p).as_deref(), Some("index.html"));
        for bad in ["//evil.com/x", "https://evil.com/", "http://127.0.0.1.evil.com/x", "http://127.0.0.1@evil.com/", "\\\\evil", "/a\r\nSet-Cookie: x"] { assert_eq!(rewrite_location(bad, p), None, "{bad}"); }
    }
}

#[cfg(test)]
mod port_owner_tests {
    use super::*;
    #[test]
    fn listener_table_parsing() {
        let t = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n   0: 0100007F:1FA4 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 4242 1 0\n   1: 0100007F:1FA4 0100007F:9C40 01 00000000:00000000 00:00000000 00000000  1000        0 4343 1 0\n   2: 00000000:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 99 1 0";
        assert_eq!(parse_listeners(t, 8100), vec![4242]);
        assert_eq!(parse_listeners(t, 8080), vec![99]);
        assert!(parse_listeners(t, 1234).is_empty());
    }
    #[test]
    fn a_port_held_by_another_process_is_refused() {
        if listening_inodes(1).is_none() { return; }
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let mut s = crate::runtimes::launch::test_srv("paper", "1.21.1", None, "balanced");
        s.pid = None;
        assert!(!port_belongs_to(&s, port, &[]));
        let mut other = std::process::Command::new("sleep").arg("5").spawn().unwrap(); // Rust opens sockets close-on-exec, so it holds none
        s.pid = Some(i64::from(other.id()));
        assert!(!port_belongs_to(&s, port, &[]));
        let _ = other.kill();
        let _ = other.wait();
        s.pid = Some(i64::from(std::process::id()));
        assert!(port_belongs_to(&s, port, &[]));
    }
}
