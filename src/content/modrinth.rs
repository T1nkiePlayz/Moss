//! Modrinth: the tiny HTTP helper (`mr`) and version selection/mapping for a server's runtime and game version.
use super::*;

/// A request that gives up with a clear message instead of hanging the caller (shorter under test).
#[cfg(not(test))] const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(25);
#[cfg(test)] const TIMEOUT: std::time::Duration = std::time::Duration::from_millis(700);

fn mr_err(rate_limited: bool, msg: impl Into<String>) -> anyhow::Error { cf::CfError { rate_limited, msg: msg.into() }.into() }

/// Sends one Modrinth request and turns every failure (unreachable, timeout, 429 with Retry-After, 5xx, bad JSON)
/// into a short message a server owner can act on. Errors carry `CfError` so handlers map them to 429 / 502.
async fn mr_send(rb: reqwest::RequestBuilder) -> Result<Value> {
    let r = rb.timeout(TIMEOUT).send().await.map_err(|e| mr_err(false, if e.is_timeout() { "Modrinth did not answer in time. Try again in a moment.".to_string() } else { format!("Could not reach Modrinth: {}", e.without_url()) }))?;
    let s = r.status();
    if s.is_success() { return r.json::<Value>().await.map_err(|_| mr_err(false, "Modrinth sent an answer Moss could not read. Try again later.")); }
    let retry = r.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse::<u64>().ok()).map(|v| v.clamp(1, 600));
    Err(match s.as_u16() {
        429 => mr_err(true, match retry { Some(n) => format!("Modrinth is rate limiting Moss. Try again in {n}s."), None => "Modrinth is rate limiting Moss. Try again in a minute.".into() }),
        404 => mr_err(false, "Not found on Modrinth."),
        500..=599 => mr_err(false, format!("Modrinth is having problems right now (HTTP {}). Try again later.", s.as_u16())),
        n => mr_err(false, format!("Modrinth refused the request (HTTP {n}).")),
    })
}

fn base(st: &St) -> String { st.cfg.modrinth_api_url.trim_end_matches('/').to_string() }

pub(super) async fn mr(st: &St, path: &str, query: &[(&str, String)]) -> Result<Value> {
    let key = format!("mr:{path}:{}", query.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&"));
    let (st2, path2, q2) = (st.clone(), path.to_string(), query.iter().map(|(k, v)| (k.to_string(), v.clone())).collect::<Vec<_>>());
    st.cached(&key, 90, move || async move { mr_send(st2.http.get(format!("{}{path2}", base(&st2))).query(&q2)).await }).await
}

pub(super) fn map_version(v: &Value) -> Value {
    json!({"id": v["id"], "name": v["name"], "version_number": v["version_number"], "channel": v["version_type"], "date": v["date_published"], "downloads": v["downloads"],
        "files": v["files"].as_array().into_iter().flatten().map(|f| json!({"filename": f["filename"], "size": f["size"], "primary": f["primary"]})).collect::<Vec<_>>(),
        "game_versions": v["game_versions"], "loaders": v["loaders"],
        "dependencies": v["dependencies"].as_array().into_iter().flatten().filter(|d| d["project_id"].is_string()).map(|d| json!({"project_id": d["project_id"], "kind": d["dependency_type"]})).collect::<Vec<_>>()})
}

pub(super) async fn compatible_versions(st: &St, pid: &str, c: &Compat) -> Result<Vec<Value>> {
    let mut q = vec![("loaders", serde_json::to_string(&c.loaders)?)];
    if let Some(gv) = &c.game_version { q.push(("game_versions", serde_json::to_string(&[gv])?)); }
    Ok(mr(st, &format!("/project/{pid}/version"), &q).await?.as_array().cloned().unwrap_or_default())
}

pub(super) fn pick_latest(vs: &[Value]) -> Option<&Value> {
    ["release", "beta", "alpha"].iter().find_map(|ch| vs.iter().find(|v| v["version_type"] == *ch))
}

/// Identifies files by SHA-512 in one batch: `{hash: version}` for every file Modrinth knows. Not cached (the key set varies).
pub(super) async fn version_files(st: &St, hashes: &[String]) -> Result<Value> {
    mr_send(st.http.post(format!("{}/version_files", base(st))).json(&json!({"hashes": hashes, "algorithm": "sha512"}))).await
}

/// Titles for project ids in one request.
pub(super) async fn project_titles(st: &St, ids: &[String]) -> Result<HashMap<String, String>> {
    let v = mr(st, "/projects", &[("ids", serde_json::to_string(ids)?)]).await?;
    Ok(v.as_array().into_iter().flatten().filter_map(|p| Some((p["id"].as_str()?.to_string(), p["title"].as_str()?.to_string()))).collect())
}

#[cfg(test)]
mod tests;
