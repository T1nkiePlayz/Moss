//! Cross-product search for the command palette: servers, backups, installed content and file names in one call,
//! filtered by what the caller may see. `GET /api/v1/search?q=…&limit=…` returns results grouped by kind.
use crate::{auth::{self, User}, error::{ApiError, ApiResult}, server, state::St};
use axum::{Json, extract::{Query, State}};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::Path;

const SKIP_DIRS: [&str; 12] = [".moss", "libraries", "versions", "cache", "region", "entities", "poi", "playerdata", "stats", "advancements", "DIM-1", "DIM1"];
const MAX_WALK: usize = 6000;

#[derive(Deserialize)]
pub struct Q { q: Option<String>, limit: Option<usize> }

/// File paths under `root` (depth limited, no symlinks) whose name contains `needle` (lower case).
pub fn find_files(root: &Path, needle: &str, max: usize) -> Vec<String> {
    let mut out = vec![];
    let mut seen = 0usize;
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((d, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        let mut entries: Vec<_> = rd.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            seen += 1;
            if seen > MAX_WALK || out.len() >= max { return out; }
            let Ok(t) = e.file_type() else { continue };
            let name = e.file_name().to_string_lossy().to_string();
            if t.is_symlink() { continue; }
            if t.is_dir() {
                if depth < 4 && !SKIP_DIRS.contains(&name.as_str()) { stack.push((e.path(), depth + 1)); }
            } else if t.is_file() && name.to_lowercase().contains(needle) && let Ok(rel) = e.path().strip_prefix(root) {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    out
}

fn like(q: &str) -> String { format!("%{}%", q.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")) }

pub async fn search(State(st): State<St>, user: User, Query(p): Query<Q>) -> ApiResult<Json<Value>> {
    let q = p.q.unwrap_or_default().trim().to_lowercase();
    if q.chars().count() > 80 { return Err(ApiError::bad("validation", "search text is too long").field("q", "at most 80 characters")); }
    let limit = p.limit.unwrap_or(6).clamp(1, 20);
    if q.chars().count() < 2 { return Ok(Json(json!({"q": q, "servers": [], "backups": [], "content": [], "files": []}))); }
    let vis = auth::visible_servers(&st, &user).await?;
    let all = server::list(&st).await?;
    let mine: Vec<&server::Srv> = all.iter().filter(|s| vis.as_ref().is_none_or(|v| v.contains(&s.id))).collect();
    let name_of = |id: &str| mine.iter().find(|s| s.id == id).map(|s| s.name.clone());
    let servers: Vec<Value> = mine.iter().filter(|s| s.name.to_lowercase().contains(&q) || s.tags.iter().any(|t| t.to_lowercase().contains(&q)) || s.runtime.contains(&q) || s.game_version.contains(&q))
        .take(limit).map(|s| json!({"id": s.id, "name": s.name, "runtime": s.runtime, "game_version": s.game_version})).collect();
    let ids: Vec<String> = mine.iter().map(|s| s.id.clone()).collect();
    let (pat, lim, ids2, admin) = (like(&q), limit as i64, ids.clone(), vis.is_none());
    let (backups, content): (Vec<Value>, Vec<Value>) = st.db.call(move |c| {
        let ok = |sid: &str| admin || ids2.iter().any(|i| i == sid);
        let mut b = c.prepare("SELECT id, server_id, server_name, created_at, note, kind FROM backups WHERE (note LIKE ?1 ESCAPE '\\' OR server_name LIKE ?1 ESCAPE '\\' OR kind LIKE ?1 ESCAPE '\\') ORDER BY created_at DESC LIMIT 200")?;
        let backups = b.query_map([&pat], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, String>(4)?, r.get::<_, String>(5)?)))?
            .filter_map(Result::ok).filter(|r| ok(&r.1)).take(lim as usize).map(|r| json!({"id": r.0, "server_id": r.1, "server_name": r.2, "created_at": r.3, "note": r.4, "kind": r.5})).collect();
        let mut i = c.prepare("SELECT id, server_id, title, filename, kind FROM content_items WHERE (title LIKE ?1 ESCAPE '\\' OR filename LIKE ?1 ESCAPE '\\') ORDER BY title LIMIT 200")?;
        let content = i.query_map([&pat], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, String>(4)?)))?
            .filter_map(Result::ok).filter(|r| ok(&r.1)).take(lim as usize).map(|r| json!({"id": r.0, "server_id": r.1, "title": r.2, "filename": r.3, "kind": r.4})).collect();
        Ok((backups, content))
    }).await?;
    let backups: Vec<Value> = backups.into_iter().map(|mut b| { if let Some(n) = name_of(b["server_id"].as_str().unwrap_or_default()) { b["server_name"] = json!(n); } b }).collect();
    let content: Vec<Value> = content.into_iter().map(|mut c| { c["server_name"] = json!(name_of(c["server_id"].as_str().unwrap_or_default())); c }).collect();
    // file names: only on servers where the caller may read files
    let mut files: Vec<Value> = vec![];
    for s in &mine {
        if files.len() >= limit { break; }
        if auth::authz(&st, &user, &s.id, "files.read").await.is_err() { continue; }
        let (root, needle, n) = (s.dir(&st), q.clone(), limit - files.len());
        let found = tokio::task::spawn_blocking(move || find_files(&root, &needle, n)).await.unwrap_or_default();
        files.extend(found.into_iter().map(|p| json!({"server_id": s.id, "server_name": s.name, "path": p})));
    }
    Ok(Json(json!({"q": q, "servers": servers, "backups": backups, "content": content, "files": files})))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finds_names_but_skips_world_data_and_links() {
        let d = std::env::temp_dir().join(format!("moss-search-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for f in ["plugins/Essentials/config.yml", "world/region/r.0.0.mca", "config/paper-global.yml", ".moss/undo/config.yml", "libraries/x/config.jar"] {
            std::fs::create_dir_all(d.join(f).parent().unwrap()).unwrap();
            std::fs::write(d.join(f), b"x").unwrap();
        }
        std::os::unix::fs::symlink("/etc", d.join("etc-link")).unwrap();
        let mut r = find_files(&d, "config", 10);
        r.sort();
        assert_eq!(r, vec!["plugins/Essentials/config.yml"]);
        assert_eq!(find_files(&d, "paper", 10), vec!["config/paper-global.yml"]);
        assert_eq!(find_files(&d, "r.0", 10), Vec::<String>::new(), "region files are skipped");
        assert_eq!(find_files(&d, "passwd", 10), Vec::<String>::new(), "links are not followed");
        assert_eq!(like("a%b_c"), "%a\\%b\\_c%");
    }
}
