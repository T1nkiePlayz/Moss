//! Opt-in public status pages. This is the only unauthenticated data surface besides `/healthz`: `GET /status/{slug}` (a
//! self-contained HTML page) and `GET /api/v1/public/status/{slug}` (the same data as JSON). Only what the owner opted in
//! is exposed: page title and description, the chosen servers' name, game, version and state, player counts when
//! `show_players` is on, uptime derived from the timeline, and the owner-typed public address. Ids, paths, ports, pids,
//! users and error details never leave this module. Missing and disabled pages answer with the same 404. The admin CRUD
//! under `/status-pages` is administrators only.
use super::*;
use axum::{extract::{ConnectInfo, Request}, routing::get as get_route};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use std::net::SocketAddr;

const CACHE_SECS: u64 = 10;
const CACHE_PREFIX: &str = "status_page:";
const RATE_LIMIT: u32 = 120; // requests per minute per client address, across both public routes
const MAX_SERVERS: usize = 50;

/// The public routes outside `/api/v1`.
pub fn public_html_routes() -> Router<St> { Router::new().route("/status/{slug}", get_route(public_status_html)) }

// ---- validation -------------------------------------------------------------------------------------------------

fn v(field: &str, m: &str) -> ApiError { ApiError::bad("validation", m.to_string()).field(field, m) }

/// Slugs are 3 to 40 characters of a-z, 0-9 and `-`, and neither start nor end with `-`.
pub fn valid_slug(s: &str) -> bool {
    (3..=40).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') && !s.starts_with('-') && !s.ends_with('-')
}

/// An unguessable default slug: 14 characters of the OS RNG over a-z0-9 (about 72 bits).
fn random_slug() -> String {
    use rand::{TryRng, rngs::SysRng};
    const A: &[u8; 36] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut out = String::with_capacity(14);
    let mut buf = [0u8; 32];
    while out.len() < 14 {
        SysRng.try_fill_bytes(&mut buf).expect("operating system random number generator failed");
        for b in buf { if b < 252 && out.len() < 14 { out.push(A[(b % 36) as usize] as char); } }
    }
    out
}

fn slug(s: &str) -> ApiResult<String> {
    let s = s.trim();
    if !valid_slug(s) { return Err(v("slug", "Use 3 to 40 lowercase letters, digits or dashes, not starting or ending with a dash.")); }
    Ok(s.to_string())
}
fn title(s: &str) -> ApiResult<String> {
    let s = s.trim();
    if s.is_empty() || s.chars().count() > 80 || s.chars().any(char::is_control) { return Err(v("title", "Give the page a title of up to 80 characters.")); }
    Ok(s.to_string())
}
fn description(s: &str) -> ApiResult<String> {
    let s = s.trim();
    if s.chars().count() > 500 || s.chars().any(|c| c.is_control() && c != '\n') { return Err(v("description", "At most 500 characters, without control characters.")); }
    Ok(s.to_string())
}
fn address(s: &str) -> ApiResult<Option<String>> {
    let s = s.trim();
    if s.chars().count() > 100 || s.chars().any(char::is_control) { return Err(v("show_address", "At most 100 characters on one line.")); }
    Ok((!s.is_empty()).then(|| s.to_string()))
}
async fn server_ids(st: &St, l: &Value) -> ApiResult<Vec<String>> {
    let Some(l) = l.as_array() else { return Err(v("server_ids", "Send a list of server ids.")) };
    let known: Vec<String> = server::list(st).await?.into_iter().map(|s| s.id).collect();
    let mut out: Vec<String> = vec![];
    for i in l {
        let Some(i) = i.as_str() else { return Err(v("server_ids", "Send a list of server ids.")) };
        if !known.iter().any(|k| k == i) { return Err(v("server_ids", "One of the chosen servers no longer exists.")); }
        if !out.iter().any(|o| o == i) { out.push(i.to_string()); }
    }
    if out.len() > MAX_SERVERS { return Err(v("server_ids", "Choose at most 50 servers.")); }
    Ok(out)
}

// ---- storage ----------------------------------------------------------------------------------------------------

#[derive(Clone)]
struct Page { id: String, slug: String, title: String, description: String, enabled: bool, show_players: bool, show_map: bool, address: Option<String>, created_at: String, servers: Vec<String> }

const COLS: &str = "id,slug,title,description,enabled,show_players,show_address,created_at,show_map";

fn read_page(c: &Connection, r: &rusqlite::Row) -> rusqlite::Result<Page> {
    let id: String = r.get(0)?;
    let servers = c.prepare("SELECT server_id FROM status_page_servers WHERE page_id=? ORDER BY position, server_id")?.query_map([&id], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
    Ok(Page { id, slug: r.get(1)?, title: r.get(2)?, description: r.get(3)?, enabled: r.get(4)?, show_players: r.get(5)?, address: r.get(6)?, created_at: r.get(7)?, show_map: r.get(8)?, servers })
}
fn page_by(c: &Connection, col: &str, val: &str) -> rusqlite::Result<Option<Page>> {
    c.query_row(&format!("SELECT {COLS} FROM status_pages WHERE {col}=?"), [val], |r| read_page(c, r)).optional()
}
fn set_servers(c: &Connection, id: &str, ids: &[String]) -> rusqlite::Result<()> {
    c.execute("DELETE FROM status_page_servers WHERE page_id=?", [id])?;
    for (i, s) in ids.iter().enumerate() { c.execute("INSERT INTO status_page_servers(page_id,server_id,position) VALUES (?,?,?)", params![id, s, i as i64])?; }
    Ok(())
}
fn admin_json(p: &Page) -> Value {
    json!({"id": p.id, "slug": p.slug, "path": format!("/status/{}", p.slug), "title": p.title, "description": p.description, "enabled": p.enabled,
        "show_players": p.show_players, "show_map": p.show_map, "show_address": p.address, "server_ids": p.servers, "created_at": p.created_at})
}
fn drop_cache(st: &St) { st.cache.lock().unwrap().retain(|k, _| !k.starts_with(CACHE_PREFIX)); }

// ---- admin API --------------------------------------------------------------------------------------------------

pub(super) async fn status_pages_list(State(st): State<St>, user: User) -> ApiResult<Json<Value>> {
    user.require_admin()?;
    let pages = st.db.call(|c| c.prepare(&format!("SELECT {COLS} FROM status_pages ORDER BY created_at, id"))?.query_map([], |r| read_page(c, r))?.collect::<rusqlite::Result<Vec<_>>>()).await?;
    Ok(Json(json!({"items": pages.iter().map(admin_json).collect::<Vec<_>>()})))
}

fn taken() -> ApiError { ApiError::conflict("status_page.slug_taken", "That address is already used by another status page.").field("slug", "Already in use.") }

pub(super) async fn status_pages_create(State(st): State<St>, user: User, Json(b): Json<Value>) -> ApiResult<(StatusCode, Json<Value>)> {
    user.require_admin()?;
    let custom = b["slug"].as_str().filter(|s| !s.trim().is_empty()).map(slug).transpose()?;
    let p = Page {
        id: uuid::Uuid::now_v7().to_string(), slug: custom.unwrap_or_else(random_slug), title: title(b["title"].as_str().unwrap_or(""))?,
        description: description(b["description"].as_str().unwrap_or(""))?, enabled: b["enabled"].as_bool().unwrap_or(true), show_players: b["show_players"].as_bool().unwrap_or(false), show_map: b["show_map"].as_bool().unwrap_or(false),
        address: address(b["show_address"].as_str().unwrap_or(""))?, created_at: crate::db::now(),
        servers: if b["server_ids"].is_null() { vec![] } else { server_ids(&st, &b["server_ids"]).await? },
    };
    let p2 = p.clone();
    let inserted = st.db.call(move |c| {
        let tx = c.unchecked_transaction()?;
        if page_by(&tx, "slug", &p2.slug)?.is_some() { return Ok(false); }
        tx.execute("INSERT INTO status_pages(id,slug,title,description,enabled,show_players,show_address,created_at,show_map) VALUES (?,?,?,?,?,?,?,?,?)",
            params![p2.id, p2.slug, p2.title, p2.description, p2.enabled, p2.show_players, p2.address, p2.created_at, p2.show_map])?;
        set_servers(&tx, &p2.id, &p2.servers)?;
        tx.commit()?;
        Ok(true)
    }).await?;
    if !inserted { return Err(taken()); }
    st.audit(&user.actor(), "status_page.add", &p.title, "", None).await;
    Ok((StatusCode::CREATED, Json(admin_json(&p))))
}

pub(super) async fn status_pages_patch(State(st): State<St>, user: User, Path(id): Path<String>, Json(b): Json<Value>) -> ApiResult<Json<Value>> {
    user.require_admin()?;
    let i = id.clone();
    let mut p = st.db.call(move |c| page_by(c, "id", &i)).await?.ok_or_else(|| ApiError::not_found("status page"))?;
    if let Some(s) = b["slug"].as_str() { p.slug = slug(s)?; }
    if let Some(s) = b["title"].as_str() { p.title = title(s)?; }
    if let Some(s) = b["description"].as_str() { p.description = description(s)?; }
    if let Some(x) = b["enabled"].as_bool() { p.enabled = x; }
    if let Some(x) = b["show_players"].as_bool() { p.show_players = x; }
    if let Some(x) = b["show_map"].as_bool() { p.show_map = x; }
    if b.get("show_address").is_some_and(Value::is_null) { p.address = None; } else if let Some(s) = b["show_address"].as_str() { p.address = address(s)?; }
    if !b["server_ids"].is_null() { p.servers = server_ids(&st, &b["server_ids"]).await?; }
    let p2 = p.clone();
    let ok = st.db.call(move |c| {
        let tx = c.unchecked_transaction()?;
        if tx.query_row("SELECT COUNT(*) FROM status_pages WHERE slug=? AND id!=?", params![p2.slug, p2.id], |r| r.get::<_, i64>(0))? > 0 { return Ok(false); }
        tx.execute("UPDATE status_pages SET slug=?,title=?,description=?,enabled=?,show_players=?,show_address=?,show_map=? WHERE id=?", params![p2.slug, p2.title, p2.description, p2.enabled, p2.show_players, p2.address, p2.show_map, p2.id])?;
        set_servers(&tx, &p2.id, &p2.servers)?;
        tx.commit()?;
        Ok(true)
    }).await?;
    if !ok { return Err(taken()); }
    drop_cache(&st);
    st.audit(&user.actor(), "status_page.update", &p.title, "", None).await;
    Ok(Json(admin_json(&p)))
}

pub(super) async fn status_pages_delete(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<StatusCode> {
    user.require_admin()?;
    let i = id.clone();
    let p = st.db.call(move |c| page_by(c, "id", &i)).await?.ok_or_else(|| ApiError::not_found("status page"))?;
    st.db.call(move |c| c.execute("DELETE FROM status_pages WHERE id=?", [&id])).await?;
    drop_cache(&st);
    st.audit(&user.actor(), "status_page.remove", &p.title, "", None).await;
    Ok(StatusCode::NO_CONTENT)
}

// ---- public data ------------------------------------------------------------------------------------------------

#[derive(Serialize, serde::Deserialize, Clone)]
struct Players { online: i64, max: i64 }
#[derive(Serialize, serde::Deserialize, Clone)]
struct PubServer {
    name: String, game: String, version: String, state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] players: Option<Players>,
    uptime_24h_pct: Option<f64>, uptime_7d_pct: Option<f64>,
}
#[derive(Serialize, serde::Deserialize, Clone)]
struct Public {
    title: String, description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] address: Option<String>,
    servers: Vec<PubServer>, updated_at: String,
}

/// Internal states collapse to the three a visitor cares about.
fn public_state(s: &str) -> &'static str {
    match s { "running" => "online", "starting" | "installing" => "starting", _ => "offline" }
}

/// Share of `[from, to]` (unix seconds) the server was up. `events` are (time, up) in order and inside the window;
/// `prior` is the state just before it, if known. Without a known starting state the first event begins the window;
/// with no data at all there is no figure.
fn uptime_pct(prior: Option<bool>, events: &[(i64, bool)], from: i64, to: i64) -> Option<f64> {
    let (mut t, mut up) = match prior { Some(u) => (from, u), None => *events.first()? };
    let start = t;
    let mut up_secs = 0;
    for &(et, eu) in events {
        let et = et.clamp(t, to);
        if up { up_secs += et - t; }
        t = et; up = eu;
    }
    if up { up_secs += (to - t).max(0); }
    let total = to - start;
    (total > 0).then(|| (up_secs as f64 / total as f64 * 1000.0).round() / 10.0)
}

fn secs(t: &str) -> Option<i64> { chrono::DateTime::parse_from_rfc3339(t).ok().map(|d| d.timestamp()) }

/// (24 hour, 7 day) uptime from the lifecycle marks in the timeline.
fn uptime_for(c: &Connection, id: &str, now: i64) -> rusqlite::Result<(Option<f64>, Option<f64>)> {
    const LIFECYCLE: &str = "kind IN ('start','reattach','stop','crash')";
    let iso = |t: i64| chrono::DateTime::from_timestamp(t, 0).unwrap_or_default().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let (from7, from24) = (now - 7 * 86400, now - 86400);
    let is_up = |k: &str| k == "start" || k == "reattach";
    let prior7: Option<bool> = c.query_row(&format!("SELECT kind FROM timeline WHERE server_id=? AND {LIFECYCLE} AND time<? ORDER BY id DESC LIMIT 1"), params![id, iso(from7)], |r| r.get::<_, String>(0)).optional()?.map(|k| is_up(&k));
    let events: Vec<(i64, bool)> = c.prepare(&format!("SELECT time,kind FROM timeline WHERE server_id=? AND {LIFECYCLE} AND time>=? ORDER BY id"))?
        .query_map(params![id, iso(from7)], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter().filter_map(|(t, k)| Some((secs(&t)?, is_up(&k)))).collect();
    let split = events.partition_point(|e| e.0 < from24);
    let prior24 = if split > 0 { Some(events[split - 1].1) } else { prior7 };
    Ok((uptime_pct(prior24, &events[split..], from24, now), uptime_pct(prior7, &events, from7, now)))
}

async fn build(st: &St, p: &Page) -> ApiResult<Public> {
    let cat = runtimes::catalogue(st.cfg.dev_runtimes);
    let now = chrono::Utc::now().timestamp();
    let mut servers = vec![];
    for id in &p.servers {
        let Ok(s) = server::get(st, id).await else { continue };
        let view = server::view(&s, st);
        let state = public_state(view["state"].as_str().unwrap_or(""));
        let players = (p.show_players && state == "online").then(|| (view["players"]["online"].as_i64(), view["players"]["max"].as_i64())).and_then(|(o, m)| Some(Players { online: o?, max: m? }));
        let game = cat.as_array().and_then(|a| a.iter().find(|r| r["id"] == s.runtime.as_str())).and_then(|r| r["name"].as_str()).map(String::from)
            .unwrap_or_else(|| { let mut c = s.runtime.chars(); c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default() });
        let i = id.clone();
        let (u24, u7) = st.db.call(move |c| uptime_for(c, &i, now)).await?;
        servers.push(PubServer { name: s.name.clone(), game, version: s.game_version.clone(), state: state.to_string(), players, uptime_24h_pct: u24, uptime_7d_pct: u7 });
    }
    Ok(Public { title: p.title.clone(), description: p.description.clone(), address: p.address.clone(), servers, updated_at: crate::db::now() })
}

/// Is a map link for this server still allowed: some enabled page lists it with the map switched on.
pub(super) async fn map_is_public(st: &St, server_id: &str) -> bool {
    let id = server_id.to_string();
    st.db.call(move |c| c.query_row("SELECT EXISTS(SELECT 1 FROM status_pages p JOIN status_page_servers s ON s.page_id=p.id WHERE p.enabled=1 AND p.show_map=1 AND s.server_id=?)", [id], |r| r.get::<_, bool>(0))).await.unwrap_or(false)
}

/// Map frames for a page with `show_map`: (server name, signed link) for each running server with a detected map.
async fn map_frames(st: &St, slug: &str) -> Vec<(String, String)> {
    let s = slug.to_string();
    let Ok(Some(page)) = st.db.call(move |c| page_by(c, "slug", &s)).await else { return vec![] };
    if !page.enabled || !page.show_map { return vec![]; }
    let mut out = vec![];
    for id in &page.servers {
        let Ok(srv) = server::get(st, id).await else { continue };
        if crate::maps::detect(st, &srv).is_none() || server::view(&srv, st)["state"] != "running" { continue; }
        out.push((srv.name.clone(), crate::maps::link(st, &srv.id, crate::maps::Scope::Public, crate::maps::PUBLIC_TOKEN_TTL_SECS).0));
    }
    out
}

/// The public view of an enabled page, cached briefly. `None` for missing and disabled pages alike.
async fn public_data(st: &St, slug: &str) -> ApiResult<Option<Public>> {
    if !valid_slug(slug) { return Ok(None); }
    let key = format!("{CACHE_PREFIX}{slug}");
    let cached = st.cache.lock().unwrap().get(&key).filter(|(at, _)| at.elapsed() < Duration::from_secs(CACHE_SECS)).and_then(|(_, v)| serde_json::from_value::<Public>(v.clone()).ok());
    if cached.is_some() { return Ok(cached); }
    let s = slug.to_string();
    let Some(page) = st.db.call(move |c| page_by(c, "slug", &s)).await?.filter(|p| p.enabled) else { return Ok(None) };
    let public = build(st, &page).await?;
    if let Ok(v) = serde_json::to_value(&public) { st.cache.lock().unwrap().insert(key, (Instant::now(), v)); }
    Ok(Some(public))
}

// ---- rate limit ---------------------------------------------------------------------------------------------------

const MAX_TRACKED: usize = 10_000;

/// Fixed-window counter per client address: `limit` requests per 60 seconds. Memory is bounded.
fn allow(m: &mut HashMap<String, (Instant, u32)>, key: &str, now: Instant, limit: u32) -> bool {
    if m.len() >= MAX_TRACKED && !m.contains_key(key) {
        m.retain(|_, (t, _)| now.duration_since(*t) < Duration::from_secs(60));
        // still full of live entries: drop the oldest tenth, never everyone (that would reset every limit at once)
        if m.len() >= MAX_TRACKED {
            let mut by_age: Vec<(Instant, String)> = m.iter().map(|(k, (t, _))| (*t, k.clone())).collect();
            by_age.sort_unstable();
            for (_, k) in by_age.into_iter().take(MAX_TRACKED / 10) { m.remove(&k); }
        }
    }
    let e = m.entry(key.to_string()).or_insert((now, 0));
    if now.duration_since(e.0) >= Duration::from_secs(60) { *e = (now, 0); }
    e.1 += 1;
    e.1 <= limit
}

fn client_key(req: &Request, st: &St) -> String {
    if st.cfg.trust_proxy && let Some(last) = req.headers().get("x-forwarded-for").and_then(|v| v.to_str().ok()).and_then(|v| v.rsplit(',').next()).map(str::trim).filter(|s| !s.is_empty()) { return last.to_string(); }
    req.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| c.0.ip().to_string()).unwrap_or_else(|| "unknown".into())
}

fn limited(st: &St, req: &Request) -> bool { limited_for(st, req, "", RATE_LIMIT) }

/// The shared per-client counter table; `bucket` keeps another route's traffic (the public map proxy) from eating
/// the status page's own budget.
pub(super) fn limited_for(st: &St, req: &Request, bucket: &str, limit: u32) -> bool {
    static HITS: std::sync::OnceLock<std::sync::Mutex<HashMap<String, (Instant, u32)>>> = std::sync::OnceLock::new();
    let key = format!("{bucket}{}", client_key(req, st));
    !allow(&mut HITS.get_or_init(Default::default).lock().unwrap_or_else(|p| p.into_inner()), &key, Instant::now(), limit)
}

pub(super) fn too_many() -> Response {
    let mut r = (StatusCode::TOO_MANY_REQUESTS, "Too many requests. Try again in a minute.").into_response();
    r.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from_static("60"));
    r
}

// ---- public handlers ----------------------------------------------------------------------------------------------

pub(super) async fn public_status_json(State(st): State<St>, Path(slug): Path<String>, req: Request) -> Response {
    if limited(&st, &req) { return too_many(); }
    match public_data(&st, &slug).await {
        Ok(Some(p)) => {
            let mut r = Json(p).into_response();
            r.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=15"));
            r
        }
        Ok(None) => ApiError::not_found("status page").into_response(),
        Err(e) => e.into_response(),
    }
}

async fn public_status_html(State(st): State<St>, Path(slug): Path<String>, req: Request) -> Response {
    if limited(&st, &req) { return too_many(); }
    let mut frames = vec![];
    let (status, body) = match public_data(&st, &slug).await {
        Ok(Some(p)) => { frames = map_frames(&st, &slug).await; (StatusCode::OK, render(&p, &frames)) }
        Ok(None) => (StatusCode::NOT_FOUND, not_found_html()),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, shell("Status unavailable", "<h1>Status unavailable</h1><p class=\"desc\">Please try again in a moment.</p>")),
    };
    let mut r = (status, body).into_response();
    let h = r.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
    h.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(if frames.is_empty() { "default-src 'none'; style-src 'unsafe-inline'; img-src data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'" }
        else { "default-src 'none'; style-src 'unsafe-inline'; img-src data:; frame-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'" }));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static(if status == StatusCode::OK { "public, max-age=15" } else { "no-store" }));
    h.insert("x-robots-tag", HeaderValue::from_static("noindex, nofollow"));
    r
}

// ---- HTML ---------------------------------------------------------------------------------------------------------

/// Escape text for HTML element content and quoted attribute values.
fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() { match c { '&' => o.push_str("&amp;"), '<' => o.push_str("&lt;"), '>' => o.push_str("&gt;"), '"' => o.push_str("&quot;"), '\'' => o.push_str("&#39;"), c => o.push(c) } }
    o
}

const CSS: &str = ":root{color-scheme:light dark;--bg:#f5f6f8;--card:#fff;--fg:#14171c;--mut:#5b6572;--line:#dde1e7;--ok:#13804a;--warn:#a15c00;--bad:#b3261e}\
@media(prefers-color-scheme:dark){:root{--bg:#0f1216;--card:#181c22;--fg:#e8ebef;--mut:#99a3b0;--line:#2a313a;--ok:#47c383;--warn:#e0a03a;--bad:#f07a72}}\
*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--fg);font:16px/1.5 system-ui,-apple-system,Segoe UI,Roboto,sans-serif}\
main{max-width:720px;margin:0 auto;padding:28px 16px 40px}h1{font-size:1.6rem;margin:0 0 6px;overflow-wrap:anywhere}.desc{color:var(--mut);margin:0 0 14px;white-space:pre-line;overflow-wrap:anywhere}\
.addr{display:inline-block;margin:0 0 16px;padding:4px 10px;border:1px solid var(--line);border-radius:8px;background:var(--card);font-family:ui-monospace,Menlo,Consolas,monospace;overflow-wrap:anywhere}\
ul{list-style:none;margin:0;padding:0;display:grid;gap:10px}li{background:var(--card);border:1px solid var(--line);border-radius:12px;padding:14px 16px}\
.top{display:flex;gap:10px;align-items:center;justify-content:space-between;flex-wrap:wrap}.name{font-weight:600;overflow-wrap:anywhere}\
.st{display:inline-flex;align-items:center;gap:6px;font-weight:600;font-size:.9rem}.st:before{content:'';width:10px;height:10px;border-radius:50%;background:currentColor}\
.online{color:var(--ok)}.starting{color:var(--warn)}.offline{color:var(--bad)}\
iframe.map{width:100%;height:60vh;min-height:320px;border:1px solid var(--line);border-radius:12px;background:var(--card)}h2{font-size:1.1rem;margin:22px 0 8px}.meta{display:flex;gap:6px 18px;flex-wrap:wrap;margin-top:6px;color:var(--mut);font-size:.9rem}footer{margin-top:20px;color:var(--mut);font-size:.8rem}";

fn shell(title: &str, body: &str) -> String {
    format!("<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta http-equiv=\"refresh\" content=\"30\"><meta name=\"robots\" content=\"noindex,nofollow\"><title>{}</title><style>{CSS}</style></head><body><main>{body}</main></body></html>\n", esc(title))
}

fn not_found_html() -> String { shell("Status page not found", "<h1>Status page not found</h1><p class=\"desc\">This page does not exist or is not public.</p>") }

fn render(p: &Public, maps: &[(String, String)]) -> String {
    let mut b = format!("<h1>{}</h1>", esc(&p.title));
    if !p.description.is_empty() { b += &format!("<p class=\"desc\">{}</p>", esc(&p.description)); }
    if let Some(a) = &p.address { b += &format!("<div class=\"addr\">{}</div>", esc(a)); }
    if p.servers.is_empty() { b += "<p class=\"desc\">No servers are listed on this page.</p>"; }
    b += "<ul>";
    for s in &p.servers {
        let label = match s.state.as_str() { "online" => "Online", "starting" => "Starting", _ => "Offline" };
        b += &format!("<li><div class=\"top\"><span class=\"name\">{}</span><span class=\"st {}\">{label}</span></div><div class=\"meta\"><span>{} {}</span>", esc(&s.name), esc(&s.state), esc(&s.game), esc(&s.version));
        if let Some(pl) = &s.players { b += &format!("<span>{} / {} players</span>", pl.online, pl.max); }
        if let Some(u) = s.uptime_24h_pct { b += &format!("<span>{u:.1}% uptime (24 h)</span>"); }
        if let Some(u) = s.uptime_7d_pct { b += &format!("<span>{u:.1}% uptime (7 d)</span>"); }
        b += "</div></li>";
    }
    for (name, url) in maps {
        b += &format!("<h2>{} map</h2><iframe class=\"map\" src=\"{}\" title=\"{} live map\" sandbox=\"allow-scripts allow-popups\" loading=\"lazy\" referrerpolicy=\"no-referrer\"></iframe>", esc(name), esc(url), esc(name));
    }
    let when = p.updated_at.get(..19).unwrap_or(&p.updated_at).replace('T', " ");
    b += &format!("</ul><footer>Updated {} UTC. This page refreshes every 30 seconds.</footer>", esc(&when));
    // a map in the page must not be reloaded every 30 seconds; its links last two hours, so refresh well before that
    if maps.is_empty() { shell(&p.title, &b) } else { shell(&p.title, &b).replacen("content=\"30\"", "content=\"1500\"", 1).replace("refreshes every 30 seconds", "refreshes every 25 minutes") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_are_validated() {
        for ok in ["abc", "my-server-1", "a-b", &"a".repeat(40), "123"] { assert!(valid_slug(ok), "{ok}"); }
        for bad in ["", "ab", &"a".repeat(41), "Abc", "-abc", "abc-", "a_b", "a b", "a/b", "../x", "ünï", "a.b"] { assert!(!valid_slug(bad), "{bad}"); }
        assert!(slug(" my-page ").is_ok());
        assert!(slug("No!").is_err());
    }

    #[test]
    fn random_slugs_are_valid_and_long_enough() {
        let (a, b) = (random_slug(), random_slug());
        assert!(valid_slug(&a) && a.len() >= 10);
        assert_ne!(a, b);
    }

    #[test]
    fn html_escapes_every_user_string() {
        let evil = "<script>alert(1)</script>\"'&";
        let p = Public {
            title: evil.into(), description: evil.into(), address: Some(evil.into()), updated_at: "2026-01-01T00:00:00.000Z".into(),
            servers: vec![PubServer { name: evil.into(), game: evil.into(), version: evil.into(), state: "online".into(), players: Some(Players { online: 1, max: 2 }), uptime_24h_pct: Some(99.5), uptime_7d_pct: None }],
        };
        let h = render(&p, &[]);
        assert!(!h.contains("<script"), "{h}");
        assert!(h.contains("&lt;script&gt;alert(1)&lt;/script&gt;&quot;&#39;&amp;"));
        assert!(h.contains("1 / 2 players") && h.contains("99.5% uptime (24 h)") && !h.contains("(7 d)"));
        assert!(h.contains("http-equiv=\"refresh\" content=\"30\"") && h.contains("prefers-color-scheme:dark"));
    }

    #[test]
    fn a_map_is_framed_in_a_sandbox_and_the_page_stops_reloading_every_30_seconds() {
        let p = Public { title: "T".into(), description: String::new(), address: None, updated_at: "2026-01-01T00:00:00.000Z".into(), servers: vec![] };
        let h = render(&p, &[("Survival <b>".into(), "/map/s1/123.p.abc/".into())]);
        assert!(h.contains("<iframe class=\"map\" src=\"/map/s1/123.p.abc/\"") && h.contains("sandbox=\"allow-scripts allow-popups\"") && !h.contains("allow-same-origin"));
        assert!(h.contains("Survival &lt;b&gt; map") && h.contains("content=\"1500\"") && !h.contains("content=\"30\""));
        assert!(!render(&p, &[]).contains("<iframe"));
    }

    #[test]
    fn states_collapse_to_three() {
        for (i, o) in [("running", "online"), ("starting", "starting"), ("installing", "starting"), ("stopped", "offline"), ("crashed", "offline"), ("failed", "offline"), ("stopping", "offline"), ("needs_attention", "offline")] { assert_eq!(public_state(i), o); }
    }

    #[test]
    fn uptime_is_the_share_of_the_window_spent_up() {
        assert_eq!(uptime_pct(Some(true), &[], 0, 100), Some(100.0));
        assert_eq!(uptime_pct(Some(false), &[], 0, 100), Some(0.0));
        assert_eq!(uptime_pct(Some(true), &[(50, false)], 0, 100), Some(50.0));
        assert_eq!(uptime_pct(Some(false), &[(25, true), (75, false)], 0, 100), Some(50.0));
        assert_eq!(uptime_pct(None, &[(50, true)], 0, 100), Some(100.0), "the window starts at the first known event");
        assert_eq!(uptime_pct(None, &[], 0, 100), None);
    }

    #[test]
    fn rate_limit_is_per_client_and_resets() {
        let mut m = HashMap::new();
        let t = Instant::now();
        for _ in 0..3 { assert!(allow(&mut m, "a", t, 3)); }
        assert!(!allow(&mut m, "a", t, 3));
        assert!(allow(&mut m, "b", t, 3));
        assert!(allow(&mut m, "a", t + Duration::from_secs(61), 3));
    }

    #[test]
    fn a_full_rate_limit_table_evicts_the_oldest_not_everyone() {
        let mut m = HashMap::new();
        let t = Instant::now();
        for i in 0..MAX_TRACKED - 1 { allow(&mut m, &format!("ip{i}"), t, 3); }
        for _ in 0..4 { allow(&mut m, "attacker", t + Duration::from_secs(1), 3); }
        assert_eq!(m.len(), MAX_TRACKED);
        assert!(allow(&mut m, "newcomer", t + Duration::from_secs(2), 3));
        assert!(m.len() <= MAX_TRACKED, "memory stays bounded: {}", m.len());
        assert!(!allow(&mut m, "attacker", t + Duration::from_secs(3), 3), "a full table does not reset limits that are still running");
    }

    #[tokio::test]
    async fn only_enabled_pages_are_public_and_hidden_fields_never_appear() {
        let st = crate::testutil::state(|c| c.dev_runtimes = true).await;
        st.db.call(|c| {
            c.execute("INSERT INTO servers(id,name,runtime,game_version,port,memory_mb,created_at,installed) VALUES ('srv-secret-id','Survival <b>','sim','sim-1.0',25599,512,'2026-01-01T00:00:00Z',1)", [])?;
            c.execute("INSERT INTO status_pages(id,slug,title,description,enabled,show_players,show_address,created_at) VALUES ('p1','my-page','T','D',1,0,'play.example.com','x')", [])?;
            set_servers(c, "p1", &["srv-secret-id".to_string()])
        }).await.unwrap();
        let public = public_data(&st, "my-page").await.unwrap().expect("enabled page is public");
        let json = serde_json::to_string(&public).unwrap();
        assert!(json.contains("play.example.com") && json.contains("Survival <b>") && json.contains("Simulated server"), "{json}");
        assert!(!json.contains("players\""), "players hidden unless the owner opted in: {json}");
        for hidden in ["srv-secret-id", "25599", "\"port\"", "\"pid\"", "\"id\"", "path", "memory", "user"] { assert!(!json.contains(hidden), "{hidden} leaked: {json}"); }
        assert!(public_data(&st, "missing-page").await.unwrap().is_none());
        assert!(public_data(&st, "Bad Slug!").await.unwrap().is_none());
        // disabling the page makes it vanish even though a cached copy exists
        st.db.call(|c| c.execute("UPDATE status_pages SET enabled=0 WHERE id='p1'", [])).await.unwrap();
        drop_cache(&st);
        assert!(public_data(&st, "my-page").await.unwrap().is_none());
    }
}
