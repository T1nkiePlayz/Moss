//! `POST /backups/{id}/preview`: what a restore would change. The archive is streamed once and never extracted;
//! entries are compared by name, size and modification time with the current server directory. Memory is bounded:
//! counters, at most 50 sample names per category and one 8-byte fingerprint per archive file.
use super::{archive::{EXCLUDE_TOP, MANIFEST, safe_entry_path, top_name}, create::archive_path, find, swap::validate_only};
use crate::{auth::User, error::{ApiError, ApiResult}, server, state::St};
use anyhow::{Result, bail};
use axum::{Json, extract::{Path, State}};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::{BTreeSet, HashSet}, hash::{Hash, Hasher}, path::Path as FsPath};

const SAMPLES: usize = 50;

#[derive(Default, Debug)]
pub struct Compare {
    pub files: u64, pub size: u64, pub added: u64, pub removed: u64, pub changed: u64, pub unchanged: u64,
    pub cur_files: u64, pub cur_size: u64, pub names: BTreeSet<String>,
    pub added_s: Vec<String>, pub removed_s: Vec<String>, pub changed_s: Vec<String>,
}

fn sample(v: &mut Vec<String>, s: &str) { if v.len() < SAMPLES { v.push(s.to_string()); } }
fn fingerprint(s: &str) -> u64 { let mut h = std::collections::hash_map::DefaultHasher::new(); s.hash(&mut h); h.finish() }
fn mtime_secs(m: &std::fs::Metadata) -> Option<u64> { m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_secs()) }

/// Blocking: compare `archive` with `current` (when given), limited to the top-level names in `only`.
pub fn compare(archive: &FsPath, current: Option<&FsPath>, only: Option<&[String]>) -> Result<Compare> {
    let mut c = Compare::default();
    let mut seen: HashSet<u64> = HashSet::new();
    let wanted = |top: &str| only.is_none_or(|o| o.iter().any(|n| n == top));
    let mut ar = tar::Archive::new(zstd::Decoder::new(std::fs::File::open(archive)?)?);
    for e in ar.entries()? {
        let e = e?;
        let p = e.path()?.into_owned();
        if p == FsPath::new(MANIFEST) { continue; }
        if !safe_entry_path(&p) { bail!("archive contains an unsafe path: {}", p.display()); }
        let Some(top) = top_name(&p) else { continue };
        c.names.insert(top.clone());
        if e.header().entry_type() != tar::EntryType::Regular || !wanted(&top) { continue; }
        let rel = p.to_string_lossy().to_string();
        c.files += 1;
        c.size += e.size();
        let Some(cur) = current else { continue };
        seen.insert(fingerprint(&rel));
        match std::fs::symlink_metadata(cur.join(&p)) {
            Ok(m) if m.is_file() => {
                if m.len() == e.size() && mtime_secs(&m) == e.header().mtime().ok() { c.unchanged += 1; } else { c.changed += 1; sample(&mut c.changed_s, &rel); }
            }
            _ => { c.added += 1; sample(&mut c.added_s, &rel); }
        }
    }
    if let Some(cur) = current {
        let mut stack = vec![(cur.to_path_buf(), String::new())];
        while let Some((dir, prefix)) = stack.pop() {
            for e in std::fs::read_dir(&dir)?.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if prefix.is_empty() && (EXCLUDE_TOP.contains(&name.as_str()) || !wanted(&name)) { continue; }
                let rel = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
                let Ok(t) = e.file_type() else { continue };
                if t.is_dir() { stack.push((e.path(), rel)); }
                else if t.is_file() {
                    c.cur_files += 1;
                    c.cur_size += e.metadata().map(|m| m.len()).unwrap_or(0);
                    if !seen.contains(&fingerprint(&rel)) { c.removed += 1; sample(&mut c.removed_s, &rel); }
                }
            }
        }
    }
    Ok(c)
}

#[derive(Deserialize)] pub struct PreviewReq { mode: String, only: Option<Vec<String>> }

pub async fn preview(State(st): State<St>, user: User, Path(bid): Path<String>, Json(r): Json<PreviewReq>) -> ApiResult<Json<Value>> {
    let (sid, ..) = find(&st, &user, &bid, "backup.manage").await?;
    let in_place = match r.mode.as_str() { "in_place" => true, "new_server" => false, _ => return Err(ApiError::bad("validation", "mode must be new_server or in_place")) };
    if let Some(only) = &r.only {
        if !in_place { return Err(ApiError::bad("validation", "only is for in-place restores").field("only", "not allowed with new_server")); }
        validate_only(only)?;
    }
    let existing = server::get(&st, &sid).await.ok();
    if in_place && existing.is_none() { return Err(ApiError::conflict("server.gone", "the original server no longer exists; restore as a new server")); }
    if !in_place { user.require_admin()?; }
    let b = bid.clone();
    let (created_at, status, consistent, worlds): (String, String, bool, String) = st.db.call(move |c| c.query_row("SELECT created_at,status,consistent,worlds FROM backups WHERE id=?", [b], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))).await?;
    if status != "ok" { return Err(ApiError::conflict("backup.unusable", "this backup did not complete")); }
    let (path, cur, only) = (archive_path(&st, &sid, &bid), existing.as_ref().filter(|_| in_place).map(|s| s.dir(&st)), r.only.clone());
    let c = tokio::task::spawn_blocking(move || compare(&path, cur.as_deref(), only.as_deref())).await.map_err(|e| anyhow::anyhow!(e))?.map_err(|e| ApiError::conflict("backup.unreadable", format!("could not read the archive: {e:#}")))?;
    if let Some(missing) = r.only.iter().flatten().find(|n| !c.names.contains(*n)) { return Err(ApiError::bad("validation", format!("\"{missing}\" is not in this backup")).field("only", "not in the archive")); }
    let mut worlds: Vec<String> = serde_json::from_str(&worlds).unwrap_or_default();
    if let Some(o) = &r.only { worlds.retain(|w| o.contains(w)); }
    let mut warnings = vec![];
    if !consistent { warnings.push("This backup was taken while the server was running and may only be crash-consistent.".to_string()); }
    if existing.as_ref().is_some_and(|s| in_place && crate::lifecycle::process_alive(s)) { warnings.push("The server is running; stop it before restoring in place.".to_string()); }
    if in_place { warnings.push("A safety backup of the current state is taken first, and replaced files are kept under data/removed.".to_string()); }
    Ok(Json(json!({
        "mode": r.mode, "archive": {"files": c.files, "size": c.size, "created_at": created_at},
        "current": if in_place { json!({"files": c.cur_files, "size": c.cur_size}) } else { Value::Null },
        "added": if in_place { c.added } else { c.files }, "removed": c.removed, "changed": c.changed, "unchanged": c.unchanged,
        "size_delta": c.size as i64 - if in_place { c.cur_size as i64 } else { 0 },
        "samples": {"added": c.added_s, "removed": c.removed_s, "changed": c.changed_s}, "worlds": worlds, "warnings": warnings,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backups::archive::build_archive;

    #[test]
    fn compares_added_removed_changed_and_unchanged() {
        let base = std::env::temp_dir().join(format!("moss-prev-{}", uuid::Uuid::new_v4()));
        let (src, out) = (base.join("src"), base.join("a.tar.zst"));
        std::fs::create_dir_all(src.join("world")).unwrap();
        for (f, d) in [("world/level.dat", "aaaa"), ("world/region.mca", "bbbb"), ("server.properties", "cccc"), ("old.txt", "dddd")] { std::fs::write(src.join(f), d).unwrap(); }
        build_archive(src.clone(), out.clone(), "{}".into(), tokio_util::sync::CancellationToken::new()).unwrap();
        // now diverge: change one file, delete one, add one, leave two alone
        std::fs::write(src.join("server.properties"), "longer content").unwrap();
        std::fs::remove_file(src.join("old.txt")).unwrap();
        std::fs::write(src.join("extra.txt"), "new").unwrap();
        std::fs::create_dir_all(src.join("logs")).unwrap();
        std::fs::write(src.join("logs/latest.log"), "ignored").unwrap();
        let c = compare(&out, Some(&src), None).unwrap();
        assert_eq!((c.files, c.added, c.changed, c.unchanged, c.removed), (4, 1, 1, 2, 1));
        assert_eq!(c.added_s, vec!["old.txt"]);
        assert_eq!(c.changed_s, vec!["server.properties"]);
        assert_eq!(c.removed_s, vec!["extra.txt"]);
        assert_eq!((c.cur_files, c.names.len()), (4, 3));
        // limited to one world
        let w = compare(&out, Some(&src), Some(&["world".to_string()])).unwrap();
        assert_eq!((w.files, w.added, w.changed, w.unchanged, w.removed, w.cur_files), (2, 0, 0, 2, 0, 2));
        // new-server mode has no current directory
        let n = compare(&out, None, None).unwrap();
        assert_eq!((n.files, n.cur_files, n.removed), (4, 0, 0));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn samples_are_capped() {
        let mut v = vec![];
        for i in 0..200 { sample(&mut v, &format!("f{i}")); }
        assert_eq!(v.len(), SAMPLES);
    }
}
