//! `GET /servers/{id}/modpacks/export`: a `.mrpack` of the server's Modrinth-sourced content (with hashes and
//! download URLs) plus config overrides. Never `world*`, `logs` or `.moss`; symlinks are never followed.
use super::*;
use axum::{http::{HeaderValue, header}, response::{IntoResponse, Response}};
use futures_util::StreamExt;
use std::io::Write;
use std::path::{Path as FsPath, PathBuf};

/// Server directories exported as overrides, when present.
const OVERRIDE_DIRS: [&str; 3] = ["config", "defaultconfigs", "kubejs"];
const MAX_FILE: u64 = 16 << 20;
const MAX_TOTAL: u64 = 256 << 20;
const MAX_FILES: usize = 20_000;

/// An installed Modrinth item together with the version data the index needs.
#[derive(Clone, Debug, PartialEq)]
pub struct Exported { pub path: String, pub url: String, pub sha1: Option<String>, pub sha512: String, pub size: u64 }

/// Loader dependency key of the `.mrpack` format for a Moss runtime.
pub fn loader_key(runtime: &str) -> Option<&'static str> {
    match runtime { "fabric" => Some("fabric-loader"), "quilt" => Some("quilt-loader"), "forge" => Some("forge"), "neoforge" => Some("neoforge"), _ => None }
}

pub fn build_index(s: &Srv, files: &[Exported], version_id: &str) -> Value {
    let mut deps = serde_json::Map::new();
    deps.insert("minecraft".into(), json!(s.game_version));
    if let (Some(k), Some(v)) = (loader_key(&s.runtime), s.loader_version.as_deref()) { deps.insert(k.into(), json!(v)); }
    json!({"formatVersion": 1, "game": "minecraft", "versionId": version_id, "name": s.name, "summary": "Exported from Moss",
        "files": files.iter().map(|f| json!({"path": f.path, "hashes": {"sha1": f.sha1, "sha512": f.sha512}, "env": {"client": "required", "server": "required"}, "downloads": [f.url], "fileSize": f.size})).collect::<Vec<_>>(),
        "dependencies": deps})
}

/// Regular files under the override directories (lstat only: links are skipped), as (relative path, absolute path).
pub fn collect_overrides(root: &FsPath) -> Vec<(String, PathBuf)> {
    let (mut out, mut total) = (vec![], 0u64);
    let mut stack: Vec<(String, PathBuf)> = OVERRIDE_DIRS.iter().map(|d| (d.to_string(), root.join(d))).collect();
    while let Some((rel, dir)) = stack.pop() {
        if !std::fs::symlink_metadata(&dir).is_ok_and(|m| m.is_dir()) { continue; }
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let mut entries: Vec<_> = rd.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let name = e.file_name().to_string_lossy().to_string();
            let (r, p) = (format!("{rel}/{name}"), e.path());
            let Ok(m) = std::fs::symlink_metadata(&p) else { continue };
            if m.is_dir() { stack.push((r, p)); }
            else if m.is_file() && m.len() <= MAX_FILE && !name.starts_with('.') && out.len() < MAX_FILES && total + m.len() <= MAX_TOTAL { total += m.len(); out.push((r, p)); }
        }
    }
    out.sort();
    out
}

fn write_pack(dest: &FsPath, index: &Value, overrides: &[(String, PathBuf)]) -> Result<()> {
    let mut w = zip::ZipWriter::new(std::fs::File::create(dest)?);
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    w.start_file("modrinth.index.json", opts)?;
    w.write_all(&serde_json::to_vec_pretty(index)?)?;
    for (rel, p) in overrides {
        w.start_file(format!("overrides/{rel}"), opts)?;
        std::io::copy(&mut std::fs::File::open(p)?, &mut w)?;
    }
    w.finish()?;
    Ok(())
}

async fn lookup(st: St, item: Value, dir: String) -> Option<Exported> {
    let (vid, filename, sha) = (item["version_id"].as_str()?, item["filename"].as_str()?, item["sha512"].as_str());
    let v = mr(&st, &format!("/version/{vid}"), &[]).await.ok()?;
    let files = v["files"].as_array()?;
    let f = files.iter().find(|f| sha.is_some() && f["hashes"]["sha512"].as_str() == sha).or_else(|| files.iter().find(|f| f["filename"] == filename)).or_else(|| files.first())?;
    let url = f["url"].as_str()?;
    mrpack::url_allowed(url).then_some(())?;
    Some(Exported { path: format!("{dir}/{}", f["filename"].as_str()?), url: url.into(), sha1: f["hashes"]["sha1"].as_str().map(String::from), sha512: f["hashes"]["sha512"].as_str()?.into(), size: f["size"].as_u64().unwrap_or(0) })
}

pub async fn export(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<Response> {
    let (s, c) = compat_for(&st, &user, &id, "files.read").await?;
    let items: Vec<Value> = db_items(&st, &id).await?.into_iter().filter(|i| i["provider"] == "modrinth" && i["version_id"].is_string() && i["dir"] == c.dir.as_str()).take(500).collect();
    let total = items.len();
    let mut files: Vec<Exported> = futures_util::stream::iter(items.into_iter().map(|i| lookup(st.clone(), i, c.dir.clone()))).buffer_unordered(8).filter_map(|x| async move { x }).collect().await;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let skipped = total - files.len();
    let index = build_index(&s, &files, &chrono::Utc::now().format("%Y.%m.%d").to_string());
    let root = s.dir(&st);
    let overrides = tokio::task::spawn_blocking(move || collect_overrides(&root)).await.map_err(|e| anyhow!(e))?;
    tokio::fs::create_dir_all(st.cfg.tmp_dir()).await?;
    let tmp = st.cfg.tmp_dir().join(format!("export-{}.mrpack", uuid::Uuid::now_v7()));
    let t2 = tmp.clone();
    let built = tokio::task::spawn_blocking(move || write_pack(&t2, &index, &overrides)).await.map_err(|e| anyhow!(e))?;
    if let Err(e) = built { let _ = tokio::fs::remove_file(&tmp).await; return Err(e.into()); }
    let f = tokio::fs::File::open(&tmp).await?;
    let len = f.metadata().await?.len();
    let _ = tokio::fs::remove_file(&tmp).await; // the open handle keeps the data readable until the stream ends
    let name: String = s.name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    let mut r = axum::body::Body::from_stream(tokio_util::io::ReaderStream::new(f)).into_response();
    let h = r.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/x-modrinth-modpack+zip"));
    h.insert(header::CONTENT_DISPOSITION, HeaderValue::from_str(&format!("attachment; filename=\"{name}.mrpack\"")).unwrap_or_else(|_| HeaderValue::from_static("attachment")));
    h.insert(header::CONTENT_LENGTH, HeaderValue::from(len));
    h.insert("x-moss-skipped", HeaderValue::from(skipped as u64));
    Ok(r)
}

#[cfg(test)]
mod tests;
