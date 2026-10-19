//! Typed content management on top of Modrinth: plugins, mods, proxy plugins and datapacks are filtered by the
//! server's runtime, loader, game version and side, installed through reviewed change plans, with undo.
use crate::{
    api::server_for, auth::User, download::{self, Hash}, error::{ApiError, ApiResult}, files::safe_join, ops::{self, OpCtx},
    server::Srv, state::St, undo,
};
use anyhow::{Context, Result, anyhow, bail};
use axum::{Json, extract::{Path, Query, State}, http::StatusCode};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;

const API: &str = "https://api.modrinth.com/v2";

pub struct Compat { pub kind: &'static str, pub project_type: &'static str, pub loaders: Vec<&'static str>, pub game_version: Option<String>, pub dir: String, pub ext: &'static str }

pub fn compat(st: &St, s: &Srv) -> Option<Compat> {
    let gv = Some(s.game_version.clone());
    let c = match s.runtime.as_str() {
        "paper" => Compat { kind: "plugins", project_type: "plugin", loaders: vec!["paper", "spigot", "bukkit"], game_version: gv, dir: "plugins".into(), ext: "jar" },
        "purpur" => Compat { kind: "plugins", project_type: "plugin", loaders: vec!["purpur", "paper", "spigot", "bukkit"], game_version: gv, dir: "plugins".into(), ext: "jar" },
        "folia" => Compat { kind: "plugins", project_type: "plugin", loaders: vec!["folia"], game_version: gv, dir: "plugins".into(), ext: "jar" },
        "fabric" => Compat { kind: "mods", project_type: "mod", loaders: vec!["fabric"], game_version: gv, dir: "mods".into(), ext: "jar" },
        "velocity" => Compat { kind: "proxy_plugins", project_type: "plugin", loaders: vec!["velocity"], game_version: None, dir: "plugins".into(), ext: "jar" },
        "vanilla" => {
            let level = std::fs::read_to_string(s.dir(st).join("server.properties")).ok()
                .and_then(|r| r.lines().find_map(|l| l.strip_prefix("level-name=").map(|v| v.trim().to_string()))).filter(|l| !l.is_empty() && !l.contains(['/', '\\']) && !l.starts_with('.')).unwrap_or_else(|| "world".into());
            Compat { kind: "datapacks", project_type: "datapack", loaders: vec!["datapack"], game_version: gv, dir: format!("{level}/datapacks"), ext: "zip" }
        }
        _ => return None,
    };
    Some(c)
}

fn valid_id(s: &str) -> bool { !s.is_empty() && s.len() <= 64 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') }

async fn mr(st: &St, path: &str, query: &[(&str, String)]) -> Result<Value> {
    let key = format!("mr:{path}:{}", query.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&"));
    let (st2, path2, q2) = (st.clone(), path.to_string(), query.iter().map(|(k, v)| (k.to_string(), v.clone())).collect::<Vec<_>>());
    st.cached(&key, 90, move || async move {
        let r = st2.http.get(format!("{API}{path2}")).query(&q2).send().await.context("contacting Modrinth")?;
        if r.status() == 429 { bail!("Modrinth rate limit reached — try again in a minute"); }
        Ok(r.error_for_status().context("Modrinth returned an error")?.json::<Value>().await?)
    }).await
}

fn upstream(e: anyhow::Error) -> ApiError { ApiError::new(StatusCode::BAD_GATEWAY, "provider.unavailable", format!("{e:#}")) }

async fn compat_for(st: &St, user: &User, id: &str, perm: &str) -> ApiResult<(Srv, Compat)> {
    let s = server_for(st, user, id, perm).await?;
    let c = compat(st, &s).ok_or_else(|| ApiError::bad("capability.unsupported", "this runtime has no managed content"))?;
    Ok((s, c))
}

pub async fn search(State(st): State<St>, user: User, Query(q): Query<HashMap<String, String>>) -> ApiResult<Json<Value>> {
    let sid = q.get("server_id").ok_or_else(|| ApiError::bad("validation", "server_id is required"))?;
    let (s, c) = compat_for(&st, &user, sid, "server.view").await?;
    let mut facets = vec![vec![format!("project_type:{}", c.project_type)], c.loaders.iter().map(|l| format!("categories:{l}")).collect()];
    if let Some(gv) = &c.game_version { facets.push(vec![format!("versions:{gv}")]); }
    if c.project_type == "mod" { facets.push(vec!["server_side:required".into(), "server_side:optional".into()]); }
    if let Some(cat) = q.get("category").filter(|c| valid_id(c)) { facets.push(vec![format!("categories:{cat}")]); }
    let index = match q.get("sort").map(|s| s.as_str()) { Some("downloads") => "downloads", Some("updated") => "updated", Some("follows") => "follows", _ => "relevance" };
    let limit = q.get("limit").and_then(|v| v.parse::<u32>().ok()).unwrap_or(20).clamp(1, 50);
    let offset = q.get("offset").and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
    let v = mr(&st, "/search", &[("query", q.get("q").cloned().unwrap_or_default().chars().take(100).collect()), ("facets", serde_json::to_string(&facets).unwrap()), ("index", index.into()), ("limit", limit.to_string()), ("offset", offset.to_string())]).await.map_err(upstream)?;
    let hits: Vec<Value> = v["hits"].as_array().into_iter().flatten().map(|h| json!({
        "project_id": h["project_id"], "slug": h["slug"], "title": h["title"], "description": h["description"], "author": h["author"], "icon_url": h["icon_url"],
        "downloads": h["downloads"], "follows": h["follows"], "categories": h["display_categories"].as_array().filter(|a| !a.is_empty()).map(|a| Value::Array(a.clone())).unwrap_or_else(|| h["categories"].clone()),
        "updated": h["date_modified"], "client_side": h["client_side"], "server_side": h["server_side"],
    })).collect();
    Ok(Json(json!({"hits": hits, "total": v["total_hits"], "compat": {"kind": c.kind, "loader": c.loaders[0], "game_version": s.game_version}})))
}

fn map_version(v: &Value) -> Value {
    json!({"id": v["id"], "name": v["name"], "version_number": v["version_number"], "channel": v["version_type"], "date": v["date_published"], "downloads": v["downloads"],
        "files": v["files"].as_array().into_iter().flatten().map(|f| json!({"filename": f["filename"], "size": f["size"], "primary": f["primary"]})).collect::<Vec<_>>(),
        "game_versions": v["game_versions"], "loaders": v["loaders"],
        "dependencies": v["dependencies"].as_array().into_iter().flatten().filter(|d| d["project_id"].is_string()).map(|d| json!({"project_id": d["project_id"], "kind": d["dependency_type"]})).collect::<Vec<_>>()})
}

async fn compatible_versions(st: &St, pid: &str, c: &Compat) -> Result<Vec<Value>> {
    let mut q = vec![("loaders", serde_json::to_string(&c.loaders)?)];
    if let Some(gv) = &c.game_version { q.push(("game_versions", serde_json::to_string(&[gv])?)); }
    Ok(mr(st, &format!("/project/{pid}/version"), &q).await?.as_array().cloned().unwrap_or_default())
}

fn pick_latest(vs: &[Value]) -> Option<&Value> {
    ["release", "beta", "alpha"].iter().find_map(|ch| vs.iter().find(|v| v["version_type"] == *ch))
}

pub async fn project(State(st): State<St>, user: User, Path(pid): Path<String>, Query(q): Query<HashMap<String, String>>) -> ApiResult<Json<Value>> {
    if !valid_id(&pid) { return Err(ApiError::bad("validation", "invalid project id")); }
    let sid = q.get("server_id").ok_or_else(|| ApiError::bad("validation", "server_id is required"))?;
    let (_s, c) = compat_for(&st, &user, sid, "server.view").await?;
    let p = mr(&st, &format!("/project/{pid}"), &[]).await.map_err(upstream)?;
    let vs = compatible_versions(&st, &pid, &c).await.map_err(upstream)?;
    Ok(Json(json!({
        "project_id": p["id"], "title": p["title"], "description": p["description"], "body": p["body"], "icon_url": p["icon_url"],
        "links": {"source": p["source_url"], "issues": p["issues_url"], "wiki": p["wiki_url"], "page": format!("https://modrinth.com/project/{}", p["slug"].as_str().unwrap_or_default())},
        "license": p["license"]["name"], "gallery": p["gallery"].as_array().into_iter().flatten().filter_map(|g| g["url"].as_str()).take(6).collect::<Vec<_>>(),
        "versions": vs.iter().take(40).map(map_version).collect::<Vec<_>>(),
    })))
}

fn item_json(r: &rusqlite::Row) -> rusqlite::Result<Value> {
    Ok(json!({"id": r.get::<_, String>(0)?, "provider": r.get::<_, String>(1)?, "project_id": r.get::<_, Option<String>>(2)?, "version_id": r.get::<_, Option<String>>(3)?, "title": r.get::<_, String>(4)?,
        "version_number": r.get::<_, Option<String>>(5)?, "filename": r.get::<_, String>(6)?, "kind": r.get::<_, String>(7)?, "sha512": r.get::<_, Option<String>>(8)?,
        "installed_at": r.get::<_, String>(9)?, "pinned": r.get::<_, bool>(10)?, "source": "moss", "dir": r.get::<_, String>(11)?}))
}

async fn db_items(st: &St, sid: &str) -> Result<Vec<Value>> {
    let sid = sid.to_string();
    st.db.call(move |c| {
        let mut q = c.prepare("SELECT id,provider,project_id,version_id,title,version_number,filename,kind,sha512,installed_at,pinned,dir FROM content_items WHERE server_id=? ORDER BY title COLLATE NOCASE")?;
        q.query_map([sid], item_json)?.collect()
    }).await
}

pub async fn installed(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let (s, c) = compat_for(&st, &user, &id, "server.view").await?;
    let mut items = db_items(&st, &id).await?;
    let known: Vec<(String, String)> = items.iter().map(|i| (i["dir"].as_str().unwrap_or_default().to_string(), i["filename"].as_str().unwrap_or_default().to_string())).collect();
    let dir = safe_join(&s.dir(&st), &c.dir)?;
    let (ext, cdir) = (c.ext, c.dir.clone());
    let manual: Vec<Value> = tokio::task::spawn_blocking(move || {
        let mut v = vec![];
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if !name.ends_with(&format!(".{ext}")) || known.iter().any(|(d, f)| *d == cdir && *f == name) { continue; }
                let when = e.metadata().ok().and_then(|m| m.modified().ok()).map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339()).unwrap_or_default();
                v.push(json!({"id": format!("manual:{name}"), "provider": "manual", "project_id": null, "version_id": null, "title": name.trim_end_matches(&format!(".{ext}")), "version_number": null, "filename": name, "kind": "", "sha512": null, "installed_at": when, "pinned": false, "source": "manual", "dir": cdir}));
            }
        }
        v
    }).await.map_err(|e| anyhow!(e))?;
    for m in manual { items.push(m); }
    for i in items.iter_mut() { i["kind"] = json!(c.kind); i.as_object_mut().map(|o| o.remove("dir")); }
    Ok(Json(json!({"items": items})))
}

pub async fn resync(st: &St, s: &Srv) {
    let Some(c) = compat(st, s) else { return };
    let root = s.dir(st);
    let Ok(items) = db_items(st, &s.id).await else { return };
    for i in items {
        let (dir, f) = (i["dir"].as_str().unwrap_or(&c.dir).to_string(), i["filename"].as_str().unwrap_or_default().to_string());
        if !root.join(&dir).join(&f).exists() {
            let id = i["id"].as_str().unwrap_or_default().to_string();
            let _ = st.db.call(move |c| c.execute("DELETE FROM content_items WHERE id=?", [id])).await;
        }
    }
}

// ---------- planning ----------
struct Step { action: &'static str, project_id: String, title: String, version: Value, file: Value, replaces: Option<(String, String)>, reason: String }

fn primary_file(v: &Value) -> Option<Value> {
    let files = v["files"].as_array()?;
    files.iter().find(|f| f["primary"] == true).or_else(|| files.first()).cloned()
}

fn safe_filename(n: &str, ext: &str) -> Result<String> {
    let base = std::path::Path::new(n).file_name().and_then(|f| f.to_str()).unwrap_or_default();
    if base.is_empty() || base.starts_with('.') || base != n || !base.to_lowercase().ends_with(&format!(".{ext}")) { bail!("provider file name {n:?} is not an acceptable .{ext} file"); }
    Ok(base.to_string())
}

async fn build_plan(st: &St, s: &Srv, c: &Compat, project_id: &str, version_id: Option<&str>) -> Result<(Vec<Step>, Vec<String>)> {
    if !valid_id(project_id) { bail!("invalid project id"); }
    let mut warnings = vec![];
    if s.runtime == "folia" { warnings.push("Folia needs plugins that explicitly declare Folia support; Modrinth lists only those tagged for Folia.".into()); }
    if lifecycle_running(st, s) { warnings.push("The server is running. Restart it to load the change.".into()); }
    let installed = db_items(st, &s.id).await?;
    let mut steps: Vec<Step> = vec![];
    let mut queue: Vec<(String, Option<String>)> = vec![(project_id.to_string(), version_id.map(String::from))];
    while let Some((pid, vid)) = queue.pop() {
        if steps.iter().any(|x| x.project_id == pid) || steps.len() >= 12 { continue; }
        let proj = mr(st, &format!("/project/{pid}"), &[]).await?;
        let pid_real = proj["id"].as_str().unwrap_or(&pid).to_string();
        let version = match vid {
            Some(v) => {
                let v = mr(st, &format!("/version/{v}"), &[]).await?;
                if v["project_id"] != pid_real.as_str() { bail!("version does not belong to project"); }
                let loaders_ok = v["loaders"].as_array().is_some_and(|l| l.iter().any(|x| c.loaders.iter().any(|y| x == y)));
                let gv_ok = c.game_version.as_ref().is_none_or(|g| v["game_versions"].as_array().is_some_and(|a| a.iter().any(|x| x == g.as_str())));
                if !loaders_ok || !gv_ok { bail!("{} {} is not compatible with {} {}", proj["title"].as_str().unwrap_or("project"), v["version_number"].as_str().unwrap_or(""), s.runtime, s.game_version); }
                v
            }
            None => {
                let vs = compatible_versions(st, &pid_real, c).await?;
                pick_latest(&vs).cloned().ok_or_else(|| anyhow!("no version of {} is compatible with {} {}", proj["title"].as_str().unwrap_or("this project"), s.runtime, s.game_version))?
            }
        };
        if version["version_type"] != "release" { warnings.push(format!("{} {} is a {} build.", proj["title"].as_str().unwrap_or(""), version["version_number"].as_str().unwrap_or(""), version["version_type"].as_str().unwrap_or("pre-release"))); }
        let file = primary_file(&version).ok_or_else(|| anyhow!("version has no files"))?;
        safe_filename(file["filename"].as_str().unwrap_or_default(), c.ext)?;
        let existing = installed.iter().find(|i| i["project_id"] == pid_real.as_str());
        if let Some(e) = existing && e["version_id"] == version["id"] && pid_real != project_id { continue; }
        let is_root = pid_real == project_id || pid == project_id;
        let replaces = existing.map(|e| (e["id"].as_str().unwrap_or_default().to_string(), e["filename"].as_str().unwrap_or_default().to_string()));
        let (action, reason) = match &replaces { Some(_) => ("replace", "replaces the installed version".to_string()), None if is_root => ("install", "requested".to_string()), None => ("install", "required dependency".to_string()) };
        for d in version["dependencies"].as_array().into_iter().flatten() {
            if d["dependency_type"] == "required"
                && let Some(dp) = d["project_id"].as_str()
                    && !installed.iter().any(|i| i["project_id"] == dp) { queue.push((dp.to_string(), d["version_id"].as_str().map(String::from))); }
        }
        steps.push(Step { action, project_id: pid_real, title: proj["title"].as_str().unwrap_or("project").into(), version, file, replaces, reason });
    }
    steps.reverse();
    Ok((steps, warnings))
}

fn lifecycle_running(st: &St, s: &Srv) -> bool { st.live.lock().unwrap().get(&s.id).is_some_and(|l| l.alive) }

#[derive(Deserialize)] pub struct InstallReq { project_id: String, version_id: Option<String> }

pub async fn plan(State(st): State<St>, user: User, Path(id): Path<String>, Json(r): Json<InstallReq>) -> ApiResult<Json<Value>> {
    let (s, c) = compat_for(&st, &user, &id, "content.manage").await?;
    let (steps, warnings) = build_plan(&st, &s, &c, &r.project_id, r.version_id.as_deref()).await.map_err(|e| ApiError::bad("plan.failed", format!("{e:#}")))?;
    Ok(Json(json!({"items": steps.iter().map(|x| json!({"action": x.action, "title": x.title, "filename": x.file["filename"], "size": x.file["size"], "reason": x.reason})).collect::<Vec<_>>(), "warnings": warnings, "requires_restart": true})))
}

pub async fn install(State(st): State<St>, user: User, Path(id): Path<String>, Json(r): Json<InstallReq>) -> ApiResult<(StatusCode, Json<Value>)> {
    let (s, c) = compat_for(&st, &user, &id, "content.manage").await?;
    let title = format!("Install content on {}", s.name);
    let actor = user.email.clone();
    let op = ops::spawn_op(&st, "content.install", title, Some(&id), true, move |ctx| apply(ctx, s, c, r.project_id, r.version_id, actor));
    Ok((StatusCode::ACCEPTED, Json(json!({"operation": op}))))
}

async fn apply(ctx: OpCtx, s: Srv, c: Compat, project_id: String, version_id: Option<String>, actor: String) -> Result<()> {
    let st = ctx.st.clone();
    let _guard = st.lock_for(&s.id).try_lock_owned().map_err(|_| anyhow!("another operation is changing this server"))?;
    ctx.stage("Resolving versions and dependencies");
    let (steps, _w) = build_plan(&st, &s, &c, &project_id, version_id.as_deref()).await?;
    ctx.check()?;
    let root = s.dir(&st);
    let target_dir = safe_join(&root, &c.dir).map_err(|e| anyhow!(e.message))?;
    tokio::fs::create_dir_all(&target_dir).await?;
    let tmp = st.cfg.tmp_dir().join(format!("content-{}", uuid::Uuid::now_v7()));
    let n = steps.len() as f64;
    let mut downloaded = vec![];
    for (i, step) in steps.iter().enumerate() {
        ctx.stage(&format!("Downloading {} ({}/{})", step.title, i + 1, steps.len()));
        let fname = safe_filename(step.file["filename"].as_str().unwrap_or_default(), c.ext)?;
        let url = step.file["url"].as_str().ok_or_else(|| anyhow!("no download url"))?;
        let hash = step.file["hashes"]["sha512"].as_str().map(|h| Hash::Sha512(h.into()));
        let dest = tmp.join(&fname);
        let c2 = ctx.clone();
        let base = i as f64 / n;
        download::fetch(&st.http, url, &dest, hash, &ctx.token, move |p| c2.progress(Some((base + p / n) * 0.8))).await?;
        downloaded.push((fname, dest));
    }
    ctx.check()?;
    ctx.stage("Snapshotting files for undo");
    let mut touched: Vec<String> = downloaded.iter().map(|(f, _)| format!("{}/{}", c.dir, f)).collect();
    for step in &steps { if let Some((_, old)) = &step.replaces { touched.push(format!("{}/{}", c.dir, old)); } }
    touched.sort(); touched.dedup();
    undo::snapshot(&st, &s, &format!("Install {}", steps.iter().map(|x| x.title.as_str()).collect::<Vec<_>>().join(", ")), &touched).await.map_err(|e| anyhow!(e.message))?;
    ctx.stage("Applying changes");
    for (step, (fname, src)) in steps.iter().zip(downloaded.iter()) {
        if let Some((old_id, old_name)) = &step.replaces {
            let _ = tokio::fs::remove_file(target_dir.join(old_name)).await;
            let oid = old_id.clone();
            st.db.call(move |c| c.execute("DELETE FROM content_items WHERE id=?", [oid])).await?;
        }
        let dest = target_dir.join(fname);
        if tokio::fs::rename(src, &dest).await.is_err() { tokio::fs::copy(src, &dest).await?; }
        let (sid, pid, vid, title, vn, fnm, kind, sha, dir) = (s.id.clone(), step.project_id.clone(), step.version["id"].as_str().unwrap_or_default().to_string(), step.title.clone(), step.version["version_number"].as_str().unwrap_or_default().to_string(), fname.clone(), c.kind.to_string(), step.file["hashes"]["sha512"].as_str().map(String::from), c.dir.clone());
        st.db.call(move |db| db.execute("INSERT INTO content_items(id,server_id,provider,project_id,version_id,title,version_number,filename,kind,sha512,installed_at,pinned,dir) VALUES (?,?,?,?,?,?,?,?,?,?,?,0,?)",
            rusqlite::params![uuid::Uuid::now_v7().to_string(), sid, "modrinth", pid, vid, title, vn, fnm, kind, sha, crate::db::now(), dir])).await?;
        ctx.log(format!("{} {} → {}/{}", step.action, step.title, c.dir, fname));
    }
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    st.timeline(&s.id, "update", &steps.iter().map(|x| x.title.as_str()).collect::<Vec<_>>().join(", ")).await;
    st.audit(&actor, "content.install", &s.name, &steps.iter().map(|x| format!("{} {}", x.title, x.version["version_number"].as_str().unwrap_or(""))).collect::<Vec<_>>().join(", "), Some(&s.id)).await;
    ctx.stage("Done — restart the server to load the change");
    Ok(())
}

pub async fn remove(State(st): State<St>, user: User, Path((id, item)): Path<(String, String)>) -> ApiResult<(StatusCode, Json<Value>)> {
    let (s, c) = compat_for(&st, &user, &id, "content.manage").await?;
    let (filename, row_id) = if let Some(f) = item.strip_prefix("manual:") { (safe_filename(f, c.ext).map_err(|e| ApiError::bad("validation", e.to_string()))?, None) } else {
        let i = item.clone();
        let f: Option<(String, String)> = st.db.call(move |db| db.query_row("SELECT filename,dir FROM content_items WHERE id=?", [i], |r| Ok((r.get(0)?, r.get(1)?))).map(Some).or_else(|e| if matches!(e, rusqlite::Error::QueryReturnedNoRows) { Ok(None) } else { Err(e) })).await?;
        (f.ok_or_else(|| ApiError::not_found("content item"))?.0, Some(item.clone()))
    };
    let actor = user.email.clone();
    let op = ops::spawn_op(&st, "content.remove", format!("Remove {filename}"), Some(&id), false, move |ctx| async move {
        let st = ctx.st.clone();
        let _g = st.lock_for(&s.id).try_lock_owned().map_err(|_| anyhow!("another operation is changing this server"))?;
        ctx.stage("Snapshotting for undo");
        let rel = format!("{}/{}", c.dir, filename);
        undo::snapshot(&st, &s, &format!("Remove {filename}"), std::slice::from_ref(&rel)).await.map_err(|e| anyhow!(e.message))?;
        ctx.stage("Removing file (plugin data and configuration are kept)");
        let p = safe_join(&s.dir(&st), &rel).map_err(|e| anyhow!(e.message))?;
        match tokio::fs::remove_file(&p).await { Ok(()) => {}, Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}, Err(e) => return Err(e.into()) }
        if let Some(rid) = row_id { st.db.call(move |db| db.execute("DELETE FROM content_items WHERE id=?", [rid])).await?; }
        st.audit(&actor, "content.remove", &s.name, &filename, Some(&s.id)).await;
        Ok(())
    });
    Ok((StatusCode::ACCEPTED, Json(json!({"operation": op}))))
}

pub async fn check_updates(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let (_s, c) = compat_for(&st, &user, &id, "content.manage").await?;
    let items = db_items(&st, &id).await?;
    let checks = items.iter().filter(|i| i["project_id"].is_string() && i["pinned"] != true).take(60).map(|i| {
        let (st, pid, cur, item_id, title, cur_num) = (st.clone(), i["project_id"].as_str().unwrap().to_string(), i["version_id"].as_str().unwrap_or_default().to_string(), i["id"].as_str().unwrap_or_default().to_string(), i["title"].as_str().unwrap_or_default().to_string(), i["version_number"].clone());
        let c = Compat { kind: c.kind, project_type: c.project_type, loaders: c.loaders.clone(), game_version: c.game_version.clone(), dir: c.dir.clone(), ext: c.ext };
        async move {
            let vs = compatible_versions(&st, &pid, &c).await.ok()?;
            let latest = pick_latest(&vs)?;
            (latest["id"].as_str()? != cur).then(|| json!({"item_id": item_id, "title": title, "current": cur_num, "latest": latest["version_number"], "version_id": latest["id"]}))
        }
    });
    let updates: Vec<Value> = futures_util::future::join_all(checks).await.into_iter().flatten().collect();
    Ok(Json(json!({"updates": updates})))
}

pub async fn update(State(st): State<St>, user: User, Path((id, item)): Path<(String, String)>) -> ApiResult<(StatusCode, Json<Value>)> {
    let (s, c) = compat_for(&st, &user, &id, "content.manage").await?;
    let i = item.clone();
    let pid: Option<String> = st.db.call(move |db| db.query_row("SELECT project_id FROM content_items WHERE id=?", [i], |r| r.get(0))).await.map_err(|_| ApiError::not_found("content item"))?;
    let pid = pid.ok_or_else(|| ApiError::bad("content.manual", "manually added files have no provider to update from"))?;
    let actor = user.email.clone();
    let op = ops::spawn_op(&st, "content.update", format!("Update on {}", s.name), Some(&id), true, move |ctx| apply(ctx, s, c, pid, None, actor));
    Ok((StatusCode::ACCEPTED, Json(json!({"operation": op}))))
}

#[derive(Deserialize)] pub struct PinReq { pinned: bool }
pub async fn patch_item(State(st): State<St>, user: User, Path((id, item)): Path<(String, String)>, Json(r): Json<PinReq>) -> ApiResult<StatusCode> {
    server_for(&st, &user, &id, "content.manage").await?;
    let n = st.db.call(move |db| db.execute("UPDATE content_items SET pinned=? WHERE id=? AND server_id=?", rusqlite::params![r.pinned, item, id])).await?;
    if n == 0 { return Err(ApiError::not_found("content item")); }
    Ok(StatusCode::NO_CONTENT)
}
