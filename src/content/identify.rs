//! Identify unmanaged files: SHA-512 each one against Modrinth (batch) and, when available, CurseForge fingerprints and Hangar (SHA-256),
//! then record matches as provider items (`source = "identified"`) so they can be updated like installed ones.
//! Only ids, title, version label and file name are stored; no CurseForge API data is cached.
use super::*;
use sha2::{Digest, Sha512};
use std::{io::Read, path::PathBuf};

/// Files above this size are skipped (jars are small; this only guards against hashing something enormous).
const MAX_HASH_SIZE: u64 = 512 << 20;

pub(super) struct Hashed { pub filename: String, pub path: PathBuf, pub sha512: String, pub fp_len: u32 }

/// SHA-512 plus the whitespace-filtered length CurseForge's fingerprint needs, in one pass.
pub(super) fn hash_file(path: &std::path::Path) -> std::io::Result<(String, u32)> {
    let mut f = std::fs::File::open(path)?;
    let (mut h, mut buf, mut fp_len) = (Sha512::new(), vec![0u8; 64 << 10], 0u64);
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 { break; }
        h.update(&buf[..n]);
        fp_len += buf[..n].iter().filter(|b| !cf::is_fp_space(**b)).count() as u64;
    }
    Ok((hex::encode(h.finalize()), fp_len.min(u32::MAX as u64) as u32))
}

pub(super) fn fingerprint_file(path: &std::path::Path, fp_len: u32) -> std::io::Result<u32> {
    let mut f = std::fs::File::open(path)?;
    let (mut m, mut buf) = (cf::Murmur2::new(fp_len), vec![0u8; 64 << 10]);
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 { break; }
        let kept: Vec<u8> = buf[..n].iter().copied().filter(|b| !cf::is_fp_space(*b)).collect();
        m.update(&kept);
    }
    Ok(m.finish())
}

/// A file that was recognised, ready to become a content_items row.
pub(super) struct Found { pub filename: String, pub sha512: String, pub provider: &'static str, pub project_id: String, pub version_id: String, pub title: String, pub version_number: String }

pub async fn identify(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<(StatusCode, Json<Value>)> {
    let (s, c) = compat_for(&st, &user, &id, "content.manage").await?;
    let (token, actor) = (user.token.clone(), user.actor());
    let op = ops::spawn_op(&st, "content.identify", format!("Identify content on {}", s.name), Some(&id), true, move |ctx| run(ctx, s, c, token, actor));
    Ok((StatusCode::ACCEPTED, Json(json!({"operation": op}))))
}

async fn run(ctx: OpCtx, s: Srv, c: Compat, token: Option<String>, actor: String) -> Result<()> {
    let st = ctx.st.clone();
    let _guard = st.lock_for(&s.id).try_lock_owned().map_err(|_| anyhow!("another operation is changing this server"))?;
    run_locked(ctx, s, c, token, actor).await.map(|_| ())
}

/// Identify a freshly imported server's files. The caller already holds the server lock. Returns (recognised, total).
pub(crate) async fn identify_imported(ctx: OpCtx, s: Srv, token: Option<String>, actor: String) -> Result<(usize, usize)> {
    let c = compat(&ctx.st, &s).ok_or_else(|| anyhow!("this runtime has no managed content"))?;
    run_locked(ctx, s, c, token, actor).await
}

async fn run_locked(ctx: OpCtx, s: Srv, c: Compat, token: Option<String>, actor: String) -> Result<(usize, usize)> {
    let st = ctx.st.clone();
    ctx.stage("Hashing files");
    let paths: Vec<(String, PathBuf)> = inventory::list_files(&st, &s, &c).await?.into_iter().filter(|(i, _)| i["provider"] == "manual").map(|(i, p)| (i["filename"].as_str().unwrap_or_default().to_string(), p)).collect();
    let total = paths.len();
    if total == 0 { ctx.log("No unmanaged files to identify."); return Ok((0, 0)); }
    let cx = ctx.clone();
    let hashed: Vec<Hashed> = tokio::task::spawn_blocking(move || {
        let mut v = vec![];
        for (i, (filename, path)) in paths.into_iter().enumerate() {
            cx.progress(Some(i as f64 / total as f64 * 0.4));
            if cx.cancelled() { break; }
            if std::fs::metadata(&path).map(|m| m.len() > MAX_HASH_SIZE).unwrap_or(true) { cx.log(format!("skipped {filename}: unreadable or too large")); continue; }
            match hash_file(&path) { Ok((sha512, fp_len)) => v.push(Hashed { filename, path, sha512, fp_len }), Err(e) => cx.log(format!("skipped {filename}: {e}")) }
        }
        v
    }).await?;
    ctx.check()?;
    ctx.stage("Asking Modrinth");
    let mut found: Vec<Found> = vec![];
    for chunk in hashed.chunks(200) {
        let res = version_files(&st, &chunk.iter().map(|h| h.sha512.clone()).collect::<Vec<_>>()).await?;
        let titles = project_titles(&st, &res.as_object().into_iter().flatten().filter_map(|(_, v)| v["project_id"].as_str().map(String::from)).collect::<std::collections::BTreeSet<_>>().into_iter().collect::<Vec<_>>()).await.unwrap_or_default();
        for h in chunk {
            let Some(v) = res.get(&h.sha512).filter(|v| v["id"].is_string()) else { continue };
            let pid = v["project_id"].as_str().unwrap_or_default().to_string();
            found.push(Found { filename: h.filename.clone(), sha512: h.sha512.clone(), provider: "modrinth", title: titles.get(&pid).cloned().or_else(|| v["name"].as_str().map(String::from)).unwrap_or_else(|| h.filename.clone()), project_id: pid, version_id: v["id"].as_str().unwrap_or_default().to_string(), version_number: v["version_number"].as_str().unwrap_or_default().to_string() });
        }
    }
    let by_modrinth = found.len();
    ctx.progress(Some(0.6));
    let rest: Vec<&Hashed> = hashed.iter().filter(|h| !found.iter().any(|f| f.filename == h.filename)).collect();
    if !rest.is_empty() && c.cf_class.is_some() && cf::availability(&st, token.as_deref()).0 {
        ctx.stage("Asking CurseForge");
        match cf_lookup(&st, token.as_deref(), &rest, &ctx).await { Ok(v) => found.extend(v), Err(e) => ctx.log(format!("CurseForge lookup skipped: {e:#}")) }
    }
    let before_hg = found.len();
    let rest: Vec<&Hashed> = hashed.iter().filter(|h| !found.iter().any(|f| f.filename == h.filename)).collect();
    if !rest.is_empty() && hangar_platform(&c).is_some() {
        ctx.stage("Asking Hangar");
        match hangar_lookup(&st, &c, &rest, &ctx).await { Ok(v) => found.extend(v), Err(e) => ctx.log(format!("Hangar lookup skipped: {e:#}")) }
    }
    ctx.check()?;
    ctx.stage("Recording matches");
    let (n, cfn, hgn) = (found.len(), before_hg - by_modrinth, found.len() - before_hg);
    record(&st, &s, &c, found).await?;
    ctx.log(format!("Identified {n} of {total} files ({by_modrinth} on Modrinth, {cfn} on CurseForge, {hgn} on Hangar); {} could not be identified and stay manual.", total - n));
    st.audit(&actor, "content.identify", &s.name, &format!("{n} of {total} files"), Some(&s.id)).await;
    Ok((n, total))
}

/// SHA-256 of each remaining file, then one Hangar hash lookup per file (Hangar has no batch endpoint).
async fn hangar_lookup(st: &St, c: &Compat, rest: &[&Hashed], ctx: &OpCtx) -> Result<Vec<Found>> {
    let jobs: Vec<(String, String, PathBuf)> = rest.iter().take(100).map(|h| (h.filename.clone(), h.sha512.clone(), h.path.clone())).collect();
    let sums: Vec<(String, String, String)> = tokio::task::spawn_blocking(move || jobs.into_iter().filter_map(|(f, s512, p)| {
        let mut file = std::fs::File::open(p).ok()?;
        let mut h = sha2::Sha256::new();
        let mut buf = vec![0u8; 64 << 10];
        loop { let n = file.read(&mut buf).ok()?; if n == 0 { break; } h.update(&buf[..n]); }
        Some((f, s512, hex::encode(h.finalize())))
    }).collect()).await?;
    let mut out = vec![];
    for (filename, sha512, sha256) in sums {
        ctx.check()?;
        let Some((project_id, title, version)) = hg_identify(st, c, &sha256).await? else { continue };
        out.push(Found { filename, sha512, provider: "hangar", project_id, version_id: version.clone(), title, version_number: version });
    }
    Ok(out)
}

async fn cf_lookup(st: &St, token: Option<&str>, rest: &[&Hashed], ctx: &OpCtx) -> Result<Vec<Found>> {
    let jobs: Vec<(String, PathBuf, u32)> = rest.iter().map(|h| (h.filename.clone(), h.path.clone(), h.fp_len)).collect();
    let fps: Vec<(String, u32)> = tokio::task::spawn_blocking(move || jobs.into_iter().filter_map(|(f, p, l)| fingerprint_file(&p, l).ok().map(|fp| (f, fp))).collect()).await?;
    let mut matches = vec![];
    for chunk in fps.chunks(500) {
        ctx.check()?;
        matches.extend(cf::fingerprint_matches(st, token, &chunk.iter().map(|(_, fp)| *fp).collect::<Vec<_>>()).await?);
    }
    let ids: Vec<u64> = { let mut v: Vec<u64> = matches.iter().map(|m| m.mod_id).collect(); v.sort(); v.dedup(); v };
    let names = if ids.is_empty() { Default::default() } else { cf::mod_names(st, token, &ids).await.unwrap_or_default() };
    let mut out = vec![];
    for (filename, fp) in &fps {
        let Some(m) = matches.iter().find(|m| m.fingerprint == *fp) else { continue };
        let sha512 = rest.iter().find(|h| h.filename == *filename).map(|h| h.sha512.clone()).unwrap_or_default();
        out.push(Found { filename: filename.clone(), sha512, provider: "curseforge", project_id: m.mod_id.to_string(), version_id: m.file_id.to_string(), title: names.get(&m.mod_id).cloned().unwrap_or_else(|| m.file_name.clone()), version_number: m.display_name.clone() });
    }
    Ok(out)
}

async fn record(st: &St, s: &Srv, c: &Compat, found: Vec<Found>) -> Result<()> {
    let (sid, kind, dir) = (s.id.clone(), c.kind.to_string(), c.dir.clone());
    st.db.call(move |db| {
        let tx = db.unchecked_transaction()?;
        for f in found {
            tx.execute("INSERT INTO content_items(id,server_id,provider,project_id,version_id,title,version_number,filename,kind,sha512,installed_at,pinned,dir,source) VALUES (?,?,?,?,?,?,?,?,?,?,?,0,?,'identified')",
                rusqlite::params![uuid::Uuid::now_v7().to_string(), sid, f.provider, f.project_id, f.version_id, f.title, f.version_number, f.filename, kind, f.sha512, crate::db::now(), dir])?;
        }
        tx.commit()
    }).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn hangar_identifies_by_sha256_and_ignores_unknown_files() {
        use crate::testutil::{Reply, state, stub};
        let known = "ab".repeat(32);
        let k = known.clone();
        let s = stub(move |_, p| {
            if p.contains(&format!("/versions/hash/{k}")) { Reply::json(200, json!({"name": "ViaVersion", "namespace": {"owner": "o", "slug": "ViaVersion"}})) }
            else if p.starts_with("/api/v1/projects/ViaVersion/versions") { Reply::json(200, json!({"result": [{"name": "5.0", "downloads": {"PAPER": {"fileInfo": {"sha256Hash": k}}}}]})) }
            else { Reply::status(404) }
        }).await;
        let b = format!("{}/api/v1", s.base);
        let st = state(move |c| c.hangar_api_url = b).await;
        let c = Compat { kind: "plugins", project_type: "plugin", loaders: vec!["paper"], game_version: Some("1.21.4".into()), dir: "plugins".into(), ext: "jar", cf_class: None, cf_loader: 0 };
        assert_eq!(hg_identify(&st, &c, &known).await.unwrap(), Some(("ViaVersion".into(), "ViaVersion".into(), "5.0".into())));
        assert_eq!(hg_identify(&st, &c, &"cd".repeat(32)).await.unwrap(), None, "404 means unknown, not an error");
        assert_eq!(hg_identify(&st, &c, "../x").await.unwrap(), None);
        let down = stub(|_, _| Reply::status(503)).await;
        let b = format!("{}/api/v1", down.base);
        assert!(hg_identify(&state(move |c| c.hangar_api_url = b).await, &c, &known).await.is_err(), "outages are errors");
    }

    #[test]
    fn hash_and_fingerprint_agree_with_in_memory_versions() {
        let p = std::env::temp_dir().join(format!("moss-identify-{}", uuid::Uuid::now_v7()));
        let data: Vec<u8> = (0..200_000u32).map(|i| (i * 31 % 253) as u8).collect();
        std::fs::write(&p, &data).unwrap();
        let (sha, len) = hash_file(&p).unwrap();
        assert_eq!(sha, hex::encode(Sha512::digest(&data)));
        assert_eq!(fingerprint_file(&p, len).unwrap(), cf::fingerprint(&data));
        let _ = std::fs::remove_file(&p);
    }
}
