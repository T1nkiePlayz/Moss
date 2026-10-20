//! File manager and server.properties editor. All paths are confined to the server root; every component is
//! checked with lstat so symlinks can never lead outside the root.
use crate::{api::server_for, auth::User, error::{ApiError, ApiResult}, server::Srv, state::St, undo};
use axum::{Json, extract::{Multipart, Path, Query, State}, http::{HeaderValue, StatusCode, header}, response::{IntoResponse, Response}};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::HashMap, path::{Component, Path as FsPath, PathBuf}};

const MAX_TEXT: u64 = 2 << 20;

/// Resolve `rel` under `root`, rejecting `..`, absolute paths and any symlink component.
pub fn safe_join(root: &FsPath, rel: &str) -> ApiResult<PathBuf> {
    let esc = || ApiError::bad("path.escape", "path escapes the server directory");
    if rel.contains('\0') { return Err(esc()); }
    let mut out = root.to_path_buf();
    for c in FsPath::new(rel.trim_start_matches('/')).components() {
        match c {
            Component::Normal(p) => {
                out.push(p);
                if let Ok(m) = std::fs::symlink_metadata(&out) && m.file_type().is_symlink() { return Err(esc()); }
            }
            Component::CurDir => {}
            _ => return Err(esc()),
        }
    }
    Ok(out)
}

/// Is `rel` the controller's metadata directory or inside it? Decided on path components (so `./.moss`, `a/../` tricks
/// and `.MOSS` on case-insensitive filesystems cannot slip past a string comparison).
fn protected(rel: &str) -> bool {
    FsPath::new(rel).components().find_map(|c| match c { Component::Normal(p) => Some(p), _ => None }).is_some_and(|p| p.to_string_lossy().eq_ignore_ascii_case(".moss"))
}

/// Does `rel` name the server root itself (`""`, `/`, `.`, `./`)?
fn is_root(rel: &str) -> bool { !FsPath::new(rel).components().any(|c| matches!(c, Component::Normal(_))) }

fn rel_of(root: &FsPath, p: &FsPath) -> String { p.strip_prefix(root).map(|r| r.to_string_lossy().to_string()).unwrap_or_default() }

type Q = Query<HashMap<String, String>>;
fn qpath(q: &HashMap<String, String>) -> String { q.get("path").cloned().unwrap_or_default() }

pub async fn list(State(st): State<St>, user: User, Path(id): Path<String>, Query(q): Q) -> ApiResult<Json<Value>> {
    let s = server_for(&st, &user, &id, "files.read").await?;
    let (root, rel) = (s.dir(&st), qpath(&q));
    if protected(&rel) { return Err(ApiError::forbidden("controller metadata is not accessible")); }
    let dir = safe_join(&root, &rel)?;
    let entries = tokio::task::spawn_blocking(move || -> ApiResult<Vec<Value>> {
        let mut v = vec![];
        for e in std::fs::read_dir(&dir)?.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if is_root(&rel) && name.eq_ignore_ascii_case(".moss") { continue; }
            let Ok(m) = e.path().symlink_metadata() else { continue };
            let kind = if m.file_type().is_symlink() { "symlink" } else if m.is_dir() { "dir" } else { "file" };
            let modified = m.modified().ok().map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339());
            v.push(json!({"name": name, "kind": kind, "size": if m.is_file() { m.len() } else { 0 }, "modified": modified}));
        }
        v.sort_by(|a, b| (a["kind"] != "dir", a["name"].as_str().map(|s| s.to_lowercase())).cmp(&(b["kind"] != "dir", b["name"].as_str().map(|s| s.to_lowercase()))));
        Ok(v)
    }).await.map_err(|e| anyhow::anyhow!(e))??;
    Ok(Json(json!({"path": qpath(&q).trim_matches('/'), "entries": entries})))
}

pub async fn read(State(st): State<St>, user: User, Path(id): Path<String>, Query(q): Q) -> ApiResult<Json<Value>> {
    let s = server_for(&st, &user, &id, "files.read").await?;
    let rel = qpath(&q);
    if protected(&rel) { return Err(ApiError::forbidden("controller metadata is not accessible")); }
    let p = safe_join(&s.dir(&st), &rel)?;
    let r = tokio::task::spawn_blocking(move || -> ApiResult<Value> {
        use std::io::Read;
        let m = std::fs::metadata(&p)?;
        if !m.is_file() { return Err(ApiError::bad("path.not_file", "not a file")); }
        let mut buf = Vec::new();
        std::fs::File::open(&p)?.take(MAX_TEXT).read_to_end(&mut buf)?;
        let binary = buf.contains(&0) || std::str::from_utf8(&buf).is_err() && m.len() <= MAX_TEXT;
        if binary { return Ok(json!({"content": "", "size": m.len(), "truncated": false, "binary": true})); }
        Ok(json!({"content": String::from_utf8_lossy(&buf), "size": m.len(), "truncated": m.len() > MAX_TEXT, "binary": false}))
    }).await.map_err(|e| anyhow::anyhow!(e))??;
    Ok(Json(r))
}

pub fn atomic_write(path: &FsPath, data: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension(format!("moss-tmp-{}", std::process::id()));
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)
}

#[derive(Deserialize)] pub struct WriteReq { path: String, content: String }
pub async fn write(State(st): State<St>, user: User, Path(id): Path<String>, Json(r): Json<WriteReq>) -> ApiResult<Json<Value>> {
    let s = server_for(&st, &user, &id, "files.write").await?;
    if protected(&r.path) { return Err(ApiError::forbidden("controller metadata is not writable")); }
    if r.content.len() as u64 > MAX_TEXT * 4 { return Err(ApiError::bad("file.too_large", "file too large for the editor")); }
    let p = safe_join(&s.dir(&st), &r.path)?;
    let undo_id = undo::snapshot(&st, &s, &format!("Edit {}", r.path), std::slice::from_ref(&r.path)).await?;
    let c = r.content.clone();
    tokio::task::spawn_blocking(move || atomic_write(&p, c.as_bytes())).await.map_err(|e| anyhow::anyhow!(e))??;
    st.audit(&user.actor(), "file.write", &s.name, &r.path, Some(&id)).await;
    Ok(Json(json!({"undo_id": undo_id})))
}

#[derive(Deserialize)] pub struct PathReq { path: String }
pub async fn mkdir(State(st): State<St>, user: User, Path(id): Path<String>, Json(r): Json<PathReq>) -> ApiResult<StatusCode> {
    let s = server_for(&st, &user, &id, "files.write").await?;
    if protected(&r.path) { return Err(ApiError::forbidden("reserved path")); }
    tokio::fs::create_dir_all(safe_join(&s.dir(&st), &r.path)?).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)] pub struct RenameReq { from: String, to: String }
pub async fn rename(State(st): State<St>, user: User, Path(id): Path<String>, Json(r): Json<RenameReq>) -> ApiResult<StatusCode> {
    let s = server_for(&st, &user, &id, "files.write").await?;
    if protected(&r.from) || protected(&r.to) { return Err(ApiError::forbidden("reserved path")); }
    let root = s.dir(&st);
    let (a, b) = (safe_join(&root, &r.from)?, safe_join(&root, &r.to)?);
    if b.exists() { return Err(ApiError::conflict("path.exists", "destination already exists")); }
    tokio::fs::rename(a, b).await?;
    st.audit(&user.actor(), "file.rename", &s.name, &format!("{} → {}", r.from, r.to), Some(&id)).await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete(State(st): State<St>, user: User, Path(id): Path<String>, Query(q): Q) -> ApiResult<StatusCode> {
    let s = server_for(&st, &user, &id, "files.write").await?;
    let rel = qpath(&q);
    if is_root(&rel) || protected(&rel) { return Err(ApiError::forbidden("refusing to delete this path")); }
    let p = safe_join(&s.dir(&st), &rel)?;
    let m = tokio::fs::symlink_metadata(&p).await?;
    if m.is_dir() { tokio::fs::remove_dir_all(&p).await?; } else { tokio::fs::remove_file(&p).await?; }
    st.audit(&user.actor(), "file.delete", &s.name, &rel, Some(&id)).await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn upload(State(st): State<St>, user: User, Path(id): Path<String>, Query(q): Q, mut mp: Multipart) -> ApiResult<Json<Value>> {
    let s = server_for(&st, &user, &id, "files.write").await?;
    let rel = qpath(&q);
    if protected(&rel) { return Err(ApiError::forbidden("reserved path")); }
    let dir = safe_join(&s.dir(&st), &rel)?;
    tokio::fs::create_dir_all(&dir).await?;
    let mut saved = vec![];
    while let Some(mut f) = mp.next_field().await.map_err(|e| ApiError::bad("upload.invalid", e.to_string()))? {
        let Some(name) = f.file_name().map(|n| n.to_string()) else { continue };
        let name = FsPath::new(&name).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if name.is_empty() || (is_root(&rel) && name.eq_ignore_ascii_case(".moss")) { continue; }
        let target = safe_join(&dir, &name)?;
        let tmp = target.with_extension("moss-upload");
        let mut out = tokio::fs::File::create(&tmp).await?;
        use tokio::io::AsyncWriteExt;
        let res: ApiResult<()> = async {
            while let Some(chunk) = f.chunk().await.map_err(|e| ApiError::bad("upload.invalid", e.to_string()))? { out.write_all(&chunk).await?; }
            out.flush().await?;
            Ok(())
        }.await;
        drop(out);
        if let Err(e) = res { let _ = tokio::fs::remove_file(&tmp).await; return Err(e); }
        tokio::fs::rename(&tmp, &target).await?;
        saved.push(rel_of(&s.dir(&st), &target));
    }
    st.audit(&user.actor(), "file.upload", &s.name, &saved.join(", "), Some(&id)).await;
    Ok(Json(json!({"saved": saved})))
}

pub async fn download(State(st): State<St>, user: User, Path(id): Path<String>, Query(q): Q) -> ApiResult<Response> {
    let s = server_for(&st, &user, &id, "files.read").await?;
    let rel = qpath(&q);
    if protected(&rel) { return Err(ApiError::forbidden("reserved path")); }
    let p = safe_join(&s.dir(&st), &rel)?;
    let f = tokio::fs::File::open(&p).await?;
    let m = f.metadata().await?;
    if !m.is_file() { return Err(ApiError::bad("path.not_file", "not a file")); }
    let name: String = p.file_name().map(|n| n.to_string_lossy().chars().map(|c| if c.is_ascii_graphic() && c != '"' && c != '\\' || c == ' ' { c } else { '_' }).collect()).unwrap_or_default();
    let mut r = axum::body::Body::from_stream(tokio_util::io::ReaderStream::new(f)).into_response();
    let h = r.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/octet-stream"));
    h.insert(header::CONTENT_DISPOSITION, HeaderValue::from_str(&format!("attachment; filename=\"{name}\"")).unwrap_or_else(|_| HeaderValue::from_static("attachment")));
    h.insert(header::CONTENT_LENGTH, HeaderValue::from(m.len()));
    Ok(r)
}

// ---------- server.properties ----------
fn parse_props(raw: &str) -> Vec<Value> {
    raw.lines().filter(|l| !l.trim_start().starts_with('#') && l.contains('=')).filter_map(|l| {
        let (k, v) = l.split_once('=')?;
        Some(json!({"key": k.trim(), "value": v.trim_end_matches('\r')}))
    }).collect()
}

pub async fn props_get(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let s = server_for(&st, &user, &id, "files.read").await?;
    if !s.capabilities().contains(&"properties") { return Err(ApiError::bad("capability.unsupported", "this runtime has no server.properties")); }
    let raw = tokio::fs::read_to_string(s.dir(&st).join("server.properties")).await.unwrap_or_default();
    Ok(Json(json!({"raw": raw, "entries": parse_props(&raw)})))
}

#[derive(Deserialize)] pub struct PropsReq { raw: String }
pub async fn props_put(State(st): State<St>, user: User, Path(id): Path<String>, Json(r): Json<PropsReq>) -> ApiResult<Json<Value>> {
    let s = server_for(&st, &user, &id, "config.write").await?;
    if r.raw.len() > 256 * 1024 { return Err(ApiError::bad("file.too_large", "too large")); }
    let entries = parse_props(&r.raw);
    let port = entries.iter().find(|e| e["key"] == "server-port").and_then(|e| e["value"].as_str()).and_then(|v| v.trim().parse::<i64>().ok());
    if let Some(p) = port {
        if !(1..=65535).contains(&p) { return Err(ApiError::bad("validation", "invalid server-port").field("server-port", "1–65535")); }
        if crate::server::list_all(&st).await?.iter().any(|o| o.id != id && o.port == p) { return Err(ApiError::conflict("port.taken", "server-port is assigned to another Moss server")); }
    }
    let undo_id = undo::snapshot(&st, &s, "Edit server.properties", &["server.properties".into()]).await?;
    let (p, raw) = (s.dir(&st).join("server.properties"), r.raw.clone());
    tokio::task::spawn_blocking(move || atomic_write(&p, raw.as_bytes())).await.map_err(|e| anyhow::anyhow!(e))??;
    if let Some(p) = port.filter(|p| *p != s.port) {
        let i = id.clone();
        st.db.call(move |c| c.execute("UPDATE servers SET port=? WHERE id=?", rusqlite::params![p, i])).await?;
    }
    st.audit(&user.actor(), "config.write", &s.name, "server.properties", Some(&id)).await;
    Ok(Json(json!({"undo_id": undo_id})))
}

/// Keep server.properties / velocity.toml in sync when the port is changed from the settings page.
pub async fn patch_port(st: &St, s: &Srv, port: i64) {
    let dir = s.dir(st);
    for (file, key, fmt) in [("server.properties", "server-port=", format!("server-port={port}")), ("velocity.toml", "bind = ", format!("bind = \"0.0.0.0:{port}\""))] {
        let p = dir.join(file);
        if let Ok(raw) = tokio::fs::read_to_string(&p).await {
            let mut found = false;
            let mut out: Vec<String> = raw.lines().map(|l| if l.trim_start().starts_with(key) { found = true; fmt.clone() } else { l.to_string() }).collect();
            if !found && file == "server.properties" { out.push(fmt.clone()); }
            let _ = atomic_write(&p, (out.join("\n") + "\n").as_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_join_accepts_normal_paths() {
        let root = std::env::temp_dir().join("moss-test-root");
        assert!(safe_join(&root, "plugins/a.jar").unwrap().starts_with(&root));
        assert_eq!(safe_join(&root, "/").unwrap(), root);
        assert_eq!(safe_join(&root, "./a//b").unwrap(), root.join("a").join("b"));
    }

    #[test]
    fn safe_join_rejects_escapes() {
        let root = std::env::temp_dir().join("moss-test-root");
        for bad in ["../x", "a/../../x", "a/..", "x\0y"] { assert!(safe_join(&root, bad).is_err(), "{bad}"); }
    }

    #[test]
    fn safe_join_rejects_symlink_components() {
        let root = std::env::temp_dir().join(format!("moss-symlink-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let _ = std::fs::remove_file(root.join("evil"));
        std::os::unix::fs::symlink("/etc", root.join("evil")).unwrap();
        assert!(safe_join(&root, "evil/passwd").is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn controller_metadata_is_protected() {
        assert!(protected(".moss") && protected("/.moss/undo/x") && !protected("plugins/.mossy"));
        // spellings that resolve to the same directory through safe_join
        for p in ["./.moss", "./.moss/undo", ".//.moss/x", ".MOSS", ".Moss/undo", "/./.moss"] { assert!(protected(p), "{p}"); }
        assert!(is_root("") && is_root("/") && is_root(".") && is_root("./") && is_root("//./") && !is_root("a") && !is_root("./a"));
    }

    #[test]
    fn properties_parse_ignores_comments() {
        let e = parse_props("#c\nserver-port=25565\nmotd=hi = there\n\n");
        assert_eq!(e.len(), 2);
        assert_eq!(e[1]["value"], "hi = there");
    }
}
