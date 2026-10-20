//! Modrinth: the tiny HTTP helper (`mr`) and version selection/mapping for a server's runtime and game version.
use super::*;

pub(super) const API: &str = "https://api.modrinth.com/v2";

pub(super) async fn mr(st: &St, path: &str, query: &[(&str, String)]) -> Result<Value> {
    let key = format!("mr:{path}:{}", query.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&"));
    let (st2, path2, q2) = (st.clone(), path.to_string(), query.iter().map(|(k, v)| (k.to_string(), v.clone())).collect::<Vec<_>>());
    st.cached(&key, 90, move || async move {
        let r = st2.http.get(format!("{API}{path2}")).query(&q2).send().await.context("contacting Modrinth")?;
        if r.status() == 429 { bail!("Modrinth rate limit reached — try again in a minute"); }
        Ok(r.error_for_status().context("Modrinth returned an error")?.json::<Value>().await?)
    }).await
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
