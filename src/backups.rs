//! Backups: tar.zst + embedded manifest + SHA-256, written to a temp name and renamed only when complete.
//! Online backups use save-off / save-all flush / save-on. Restores go to a new server or swap in place with the
//! previous directory kept. Archives are validated entry by entry (no traversal, links, devices).
use crate::{
    api::server_for, auth::{self, User}, error::{ApiError, ApiResult}, ops::{self, OpCtx}, server::{self, Srv}, state::St,
};
use anyhow::{Result, anyhow, bail};
use axum::{Json, extract::{Path, State}, http::{HeaderValue, StatusCode, header}, response::{IntoResponse, Response}};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{io::{Read, Write}, path::{Path as FsPath, PathBuf}};

const EXCLUDE_TOP: &[&str] = &[".moss", "logs", "cache"];
const MANIFEST: &str = ".moss-manifest.json";

struct HashW<W: Write> { inner: W, h: Sha256, n: u64 }
impl<W: Write> Write for HashW<W> {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> { let n = self.inner.write(b)?; self.h.update(&b[..n]); self.n += n as u64; Ok(n) }
    fn flush(&mut self) -> std::io::Result<()> { self.inner.flush() }
}

/// Reads exactly `len` bytes, zero-padding if the file shrinks while being archived (tar needs a fixed length).
struct Fixed { f: std::io::Take<std::fs::File>, left: u64 }
impl Read for Fixed {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.left == 0 { return Ok(0); }
        let want = buf.len().min(self.left as usize);
        let n = self.f.read(&mut buf[..want])?;
        let n = if n == 0 { buf[..want].fill(0); want } else { n };
        self.left -= n as u64;
        Ok(n)
    }
}

pub fn dir_size(root: &FsPath) -> u64 {
    fn walk(p: &FsPath, top: bool) -> u64 {
        let Ok(rd) = std::fs::read_dir(p) else { return 0 };
        rd.flatten().map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if top && EXCLUDE_TOP.contains(&name.as_str()) { return 0; }
            match e.file_type() { Ok(t) if t.is_dir() => walk(&e.path(), false), Ok(t) if t.is_file() => e.metadata().map(|m| m.len()).unwrap_or(0), _ => 0 }
        }).sum()
    }
    walk(root, true)
}

fn build_archive(root: PathBuf, out: PathBuf, manifest: String, token: tokio_util::sync::CancellationToken) -> Result<(String, u64, u64)> {
    let f = std::fs::File::create(&out)?;
    let hw = HashW { inner: std::io::BufWriter::with_capacity(1 << 20, f), h: Sha256::new(), n: 0 };
    let mut enc = zstd::Encoder::new(hw, 3)?;
    enc.multithread(2).ok();
    let mut tar = tar::Builder::new(enc);
    tar.follow_symlinks(false);
    let mut h = tar::Header::new_gnu();
    h.set_size(manifest.len() as u64); h.set_mode(0o644); h.set_entry_type(tar::EntryType::Regular); h.set_cksum();
    tar.append_data(&mut h, MANIFEST, manifest.as_bytes())?;
    let mut files = 0u64;
    let mut stack = vec![(root.clone(), true)];
    while let Some((dir, top)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            if token.is_cancelled() { bail!("cancelled"); }
            let name = e.file_name().to_string_lossy().to_string();
            if top && EXCLUDE_TOP.contains(&name.as_str()) { continue; }
            let Ok(t) = e.file_type() else { continue };
            let rel = e.path().strip_prefix(&root)?.to_path_buf();
            if t.is_dir() {
                tar.append_dir(&rel, e.path()).ok();
                stack.push((e.path(), false));
            } else if t.is_file() {
                let Ok(file) = std::fs::File::open(e.path()) else { continue };
                let Ok(m) = file.metadata() else { continue };
                let mut hd = tar::Header::new_gnu();
                hd.set_metadata(&m);
                hd.set_size(m.len());
                hd.set_cksum();
                tar.append_data(&mut hd, &rel, Fixed { f: file.take(m.len()), left: m.len() })?;
                files += 1;
            } // symlinks, sockets and devices are deliberately not archived
        }
    }
    let enc = tar.into_inner()?;
    let mut hw = enc.finish()?;
    hw.flush()?;
    Ok((hex::encode(hw.h.finalize()), hw.n, files))
}

fn row_json(r: &rusqlite::Row) -> rusqlite::Result<Value> {
    Ok(json!({"id": r.get::<_, String>(0)?, "server_id": r.get::<_, String>(1)?, "created_at": r.get::<_, String>(2)?, "size": r.get::<_, i64>(3)?, "sha256": r.get::<_, String>(4)?,
        "status": r.get::<_, String>(5)?, "verified_at": r.get::<_, Option<String>>(6)?, "note": r.get::<_, String>(7)?, "kind": r.get::<_, String>(8)?,
        "consistent": r.get::<_, bool>(9)?, "files": r.get::<_, i64>(10)?, "server_name": r.get::<_, String>(11)?}))
}
const SEL: &str = "SELECT id,server_id,created_at,size,sha256,status,verified_at,note,kind,consistent,files,server_name FROM backups";

pub fn archive_path(st: &St, server_id: &str, id: &str) -> PathBuf { st.cfg.backups_dir().join(server_id).join(format!("{id}.tar.zst")) }

pub async fn do_backup(ctx: &OpCtx, st: &St, s: &Srv, kind: &str, note: &str, keep: Option<i64>) -> Result<String> {
    let _g = st.lock_for(&s.id).try_lock_owned().map_err(|_| anyhow!("another operation is changing this server"))?;
    let root = s.dir(st);
    ctx.stage("Checking free space");
    let (est, (_, free)) = (dir_size(&root), crate::metrics::disk(&st.cfg.data_dir));
    if free > 0 && free < est / 2 + (256 << 20) { bail!("not enough free disk space: need about {} MiB, have {} MiB", (est / 2 + (256 << 20)) >> 20, free >> 20); }
    let id = uuid::Uuid::now_v7().to_string();
    let (sid, sname, rt, gv, k, n) = (s.id.clone(), s.name.clone(), s.runtime.clone(), s.game_version.clone(), kind.to_string(), note.to_string());
    let i2 = id.clone();
    st.db.call(move |c| c.execute("INSERT INTO backups(id,server_id,created_at,status,note,kind,server_name,runtime,game_version) VALUES (?,?,?,'running',?,?,?,?,?)", rusqlite::params![i2, sid, crate::db::now(), n, k, sname, rt, gv])).await?;
    let running = st.live.lock().unwrap().get(&s.id).is_some_and(|l| l.alive && l.ready);
    let mut consistent = !running;
    if running && s.is_mc() {
        ctx.stage("Flushing world to disk (save-off, save-all flush)");
        let hub = crate::console::hub(st, &s.id);
        let before = hub.ring.lock().unwrap().0;
        if crate::lifecycle::send_command(st, s, "save-off").await.is_ok() && crate::lifecycle::send_command(st, s, "save-all flush").await.is_ok() {
            for _ in 0..75 {
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                let seen = hub.ring.lock().unwrap().1.iter().any(|(seq, l)| *seq > before && (l.contains("Saved the game") || l.contains("Saved the world")));
                if seen { consistent = true; break; }
            }
        }
        if !consistent { ctx.log("world flush was not confirmed; the backup may be crash-consistent only"); }
    } else if running {
        consistent = false;
    }
    let result: Result<(String, u64, u64)> = async {
        ctx.stage("Compressing");
        let dir = st.cfg.backups_dir().join(&s.id);
        tokio::fs::create_dir_all(&dir).await?;
        let (out, part) = (archive_path(st, &s.id, &id), dir.join(format!("{id}.part")));
        let manifest = json!({"moss": env!("CARGO_PKG_VERSION"), "backup_id": id, "server_id": s.id, "server_name": s.name, "runtime": s.runtime, "game_version": s.game_version, "created_at": crate::db::now(), "consistent": consistent, "excluded": EXCLUDE_TOP}).to_string();
        let (root2, part2, token) = (root.clone(), part.clone(), ctx.token.clone());
        let ticker = { let c = ctx.clone(); let p = part.clone(); tokio::spawn(async move { loop { tokio::time::sleep(std::time::Duration::from_millis(500)).await; let done = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0) as f64; c.progress(Some(if est == 0 { 0.5 } else { (done / (est as f64 * 0.5 + 1.0)).min(0.95) })); } }) };
        let r = tokio::task::spawn_blocking(move || build_archive(root2, part2, manifest, token)).await;
        ticker.abort();
        let r = r.map_err(|e| anyhow!(e)).and_then(|r| r);
        match r { Ok(v) => { tokio::fs::rename(&part, &out).await?; Ok(v) } Err(e) => { let _ = tokio::fs::remove_file(&part).await; Err(e) } }
    }.await;
    if running && s.is_mc() { let _ = crate::lifecycle::send_command(st, s, "save-on").await; }
    match result {
        Ok((sha, size, files)) => {
            let (i3, sha2) = (id.clone(), sha.clone());
            st.db.call(move |c| c.execute("UPDATE backups SET status='ok', size=?, sha256=?, files=?, consistent=? WHERE id=?", rusqlite::params![size as i64, sha2, files as i64, consistent, i3])).await?;
            ctx.log(format!("{} files, {} MiB, sha256 {}", files, size >> 20, &sha[..16]));
            st.timeline(&s.id, "backup", kind).await;
            if kind == "scheduled" { prune(st, &s.id, keep).await; }
            Ok(id)
        }
        Err(e) => {
            let i3 = id.clone();
            let _ = st.db.call(move |c| c.execute("UPDATE backups SET status='failed' WHERE id=?", [i3])).await;
            Err(e)
        }
    }
}

/// Keep the newest `keep` scheduled backups, but never remove the last verified-good one.
async fn prune(st: &St, sid: &str, keep: Option<i64>) {
    let keep = match keep { Some(k) => k, None => st.setting("backup_keep", "7").await.parse().unwrap_or(7) }.max(1);
    let s2 = sid.to_string();
    let victims: Vec<String> = st.db.call(move |c| {
        let mut q = c.prepare("SELECT id FROM backups WHERE server_id=?1 AND kind='scheduled' AND status IN ('ok','failed') AND id <> COALESCE((SELECT id FROM backups WHERE server_id=?1 AND status='ok' ORDER BY created_at DESC LIMIT 1), '') ORDER BY created_at DESC LIMIT -1 OFFSET ?2")?;
        q.query_map(rusqlite::params![s2, keep], |r| r.get(0))?.collect()
    }).await.unwrap_or_default();
    for v in victims { let _ = remove_backup(st, sid, &v).await; }
}

async fn remove_backup(st: &St, sid: &str, id: &str) -> Result<()> {
    let _ = tokio::fs::remove_file(archive_path(st, sid, id)).await;
    let i = id.to_string();
    st.db.call(move |c| c.execute("DELETE FROM backups WHERE id=?", [i])).await?;
    Ok(())
}

fn safe_entry_path(p: &FsPath) -> bool { p.components().all(|c| matches!(c, std::path::Component::Normal(_))) && p.components().count() > 0 }

fn verify_archive(path: &FsPath, want: &str) -> Result<u64> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop { let n = f.read(&mut buf)?; if n == 0 { break; } h.update(&buf[..n]); }
    let got = hex::encode(h.finalize());
    if !got.eq_ignore_ascii_case(want) { bail!("checksum mismatch: archive is corrupt (expected {}…, got {}…)", &want[..12.min(want.len())], &got[..12]); }
    let mut ar = tar::Archive::new(zstd::Decoder::new(std::fs::File::open(path)?)?);
    let mut entries = 0;
    for e in ar.entries()? {
        let e = e?;
        let p = e.path()?.into_owned();
        if !safe_entry_path(&p) { bail!("archive contains an unsafe path: {}", p.display()); }
        if !matches!(e.header().entry_type(), tar::EntryType::Regular | tar::EntryType::Directory) { bail!("archive contains a non-regular entry: {}", p.display()); }
        entries += 1;
    }
    Ok(entries)
}

fn extract_archive(path: &FsPath, dest: &FsPath, token: &tokio_util::sync::CancellationToken) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    let mut ar = tar::Archive::new(zstd::Decoder::new(std::fs::File::open(path)?)?);
    ar.set_preserve_permissions(false);
    ar.set_unpack_xattrs(false);
    let mut total = 0u64;
    for e in ar.entries()? {
        if token.is_cancelled() { bail!("cancelled"); }
        let mut e = e?;
        let p = e.path()?.into_owned();
        if p == FsPath::new(MANIFEST) { continue; }
        if !safe_entry_path(&p) || !matches!(e.header().entry_type(), tar::EntryType::Regular | tar::EntryType::Directory) { bail!("refusing unsafe archive entry {}", p.display()); }
        total += e.size();
        if total > (2u64 << 40) { bail!("archive expands beyond the safety limit"); }
        if !e.unpack_in(dest)? { bail!("entry {} would escape the destination", p.display()); }
    }
    Ok(())
}

// ---------- API ----------
pub async fn list(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    server_for(&st, &user, &id, "server.view").await?;
    let v: Vec<Value> = st.db.call(move |c| { let mut q = c.prepare(&format!("{SEL} WHERE server_id=? ORDER BY created_at DESC"))?; q.query_map([id], row_json)?.collect() }).await?;
    Ok(Json(json!(v)))
}

pub async fn list_all(State(st): State<St>, user: User) -> ApiResult<Json<Value>> {
    let vis = auth::visible_servers(&st, &user).await?;
    let v: Vec<Value> = st.db.call(|c| { let mut q = c.prepare(&format!("{SEL} ORDER BY created_at DESC LIMIT 500"))?; q.query_map([], row_json)?.collect() }).await?;
    Ok(Json(Value::Array(v.into_iter().filter(|b| vis.as_ref().is_none_or(|v| v.iter().any(|s| b["server_id"] == s.as_str()))).collect())))
}

#[derive(Deserialize, Default)] pub struct CreateReq { note: Option<String> }
pub async fn create(State(st): State<St>, user: User, Path(id): Path<String>, body: Option<Json<CreateReq>>) -> ApiResult<(StatusCode, Json<Value>)> {
    let s = server_for(&st, &user, &id, "backup.manage").await?;
    if !s.installed { return Err(ApiError::conflict("server.not_installed", "server is not installed yet")); }
    let note = body.and_then(|b| b.0.note).unwrap_or_default().chars().take(200).collect::<String>();
    let actor = user.email.clone();
    let op = ops::spawn_op(&st, "backup.create", format!("Back up {}", s.name), Some(&id), true, move |ctx| async move {
        let st = ctx.st.clone();
        do_backup(&ctx, &st, &s, "manual", &note, None).await?;
        st.audit(&actor, "backup.create", &s.name, &note, Some(&s.id)).await;
        Ok(())
    });
    Ok((StatusCode::ACCEPTED, Json(json!({"operation": op}))))
}

async fn find(st: &St, user: &User, bid: &str, perm: &str) -> ApiResult<(String, String, String, String, String)> {
    let b = bid.to_string();
    let r: Option<(String, String, String, String, String)> = st.db.call(move |c| c.query_row("SELECT server_id,sha256,server_name,runtime,game_version FROM backups WHERE id=?", [b], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))).map(Some).or_else(|e| if matches!(e, rusqlite::Error::QueryReturnedNoRows) { Ok(None) } else { Err(e) })).await?;
    let r = r.ok_or_else(|| ApiError::not_found("backup"))?;
    if st.db.call({ let s = r.0.clone(); move |c| c.query_row("SELECT 1 FROM servers WHERE id=?", [s], |_| Ok(())) }).await.is_ok() { auth::authz(st, user, &r.0, perm).await?; } else { user.require_admin()?; }
    Ok(r)
}

pub async fn verify(State(st): State<St>, user: User, Path(bid): Path<String>) -> ApiResult<(StatusCode, Json<Value>)> {
    let (sid, sha, ..) = find(&st, &user, &bid, "backup.manage").await?;
    let b = bid.clone();
    let op = ops::spawn_op(&st, "backup.verify", "Verify backup", Some(&sid.clone()), false, move |ctx| async move {
        ctx.stage("Reading archive and checking SHA-256");
        let p = archive_path(&ctx.st, &sid, &b);
        let n = tokio::task::spawn_blocking(move || verify_archive(&p, &sha)).await??;
        ctx.log(format!("{n} entries verified, all paths safe"));
        let b2 = b.clone();
        ctx.st.db.call(move |c| c.execute("UPDATE backups SET verified_at=? WHERE id=?", [crate::db::now(), b2])).await?;
        Ok(())
    });
    Ok((StatusCode::ACCEPTED, Json(json!({"operation": op}))))
}

#[derive(Deserialize)] pub struct RestoreReq { mode: String }
pub async fn restore(State(st): State<St>, user: User, Path(bid): Path<String>, Json(r): Json<RestoreReq>) -> ApiResult<(StatusCode, Json<Value>)> {
    let (sid, sha, sname, rt, gv) = find(&st, &user, &bid, "backup.manage").await?;
    let existing = server::get(&st, &sid).await.ok();
    let in_place = match r.mode.as_str() { "in_place" => true, "new_server" => false, _ => return Err(ApiError::bad("validation", "mode must be new_server or in_place")) };
    if in_place {
        let s = existing.as_ref().ok_or_else(|| ApiError::conflict("server.gone", "the original server no longer exists; restore as a new server"))?;
        if crate::lifecycle::process_alive(s) { return Err(ApiError::conflict("server.running", "stop the server before restoring in place")); }
    } else { user.require_admin()?; }
    let actor = user.email.clone();
    let b = bid.clone();
    let op = ops::spawn_op(&st, "backup.restore", format!("Restore {sname}"), Some(&sid.clone()), true, move |ctx| async move {
        let st = ctx.st.clone();
        ctx.stage("Verifying archive before touching anything");
        let p = archive_path(&st, &sid, &b);
        let (p2, sha2) = (p.clone(), sha.clone());
        tokio::task::spawn_blocking(move || verify_archive(&p2, &sha2)).await??;
        ctx.check()?;
        if in_place {
            let s = existing.unwrap();
            ctx.stage("Taking a safety backup of the current state");
            do_backup(&ctx, &st, &s, "pre-change", "before restore", None).await?;
            let _g = st.lock_for(&s.id).try_lock_owned().map_err(|_| anyhow!("another operation is changing this server"))?;
            ctx.stage("Extracting to a staging directory");
            let stage = st.cfg.servers_dir().join(format!(".restore-{}", uuid::Uuid::now_v7()));
            let (p3, stage2, tok) = (p.clone(), stage.clone(), ctx.token.clone());
            if let Err(e) = tokio::task::spawn_blocking(move || extract_archive(&p3, &stage2, &tok)).await? { let _ = tokio::fs::remove_dir_all(&stage).await; return Err(e); }
            ctx.stage("Swapping directories (the previous one is kept)");
            let (dir, kept) = (s.dir(&st), st.cfg.data_dir.join("removed").join(format!("{}-pre-restore-{}", s.id, chrono::Utc::now().format("%Y%m%d-%H%M%S"))));
            tokio::fs::create_dir_all(kept.parent().unwrap()).await?;
            tokio::fs::rename(&dir, &kept).await?;
            if let Err(e) = tokio::fs::rename(&stage, &dir).await { let _ = tokio::fs::rename(&kept, &dir).await; return Err(e.into()); }
            let _ = tokio::fs::rename(kept.join(".moss"), dir.join(".moss")).await;
            ctx.log(format!("previous contents kept at {}", kept.display()));
            st.audit(&actor, "backup.restore", &s.name, "in place", Some(&s.id)).await;
        } else {
            ctx.stage("Creating a new server from the backup");
            let nid = uuid::Uuid::now_v7().to_string();
            let used: Vec<i64> = server::list(&st).await?.iter().map(|x| x.port).collect();
            let mut port = existing.as_ref().map_or(25565, |e| e.port);
            while used.contains(&port) || tokio::net::TcpListener::bind(("0.0.0.0", port as u16)).await.is_err() { port += 1; }
            let dir = st.cfg.servers_dir().join(&nid);
            let (p3, d2, tok) = (p.clone(), dir.clone(), ctx.token.clone());
            ctx.stage("Extracting");
            if let Err(e) = tokio::task::spawn_blocking(move || extract_archive(&p3, &d2, &tok)).await? { let _ = tokio::fs::remove_dir_all(&dir).await; return Err(e); }
            let _ = tokio::fs::create_dir_all(dir.join(".moss")).await;
            let (n2, e) = (format!("{sname} (restored)"), existing.clone());
            let (rt2, gv2, nid2) = (rt.clone(), gv.clone(), nid.clone());
            st.db.call(move |c| c.execute(
                "INSERT INTO servers(id,name,runtime,game_version,loader_version,java_major,port,memory_mb,flags_profile,eula_at,eula_by,custom_cmd,installed,created_at) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,1,?)",
                rusqlite::params![nid2, n2, rt2, gv2, e.as_ref().and_then(|e| e.loader_version.clone()), e.as_ref().map_or(21, |e| e.java_major), port, e.as_ref().map_or(2048, |e| e.memory_mb), e.as_ref().map_or("balanced".to_string(), |e| e.flags_profile.clone()),
                    e.as_ref().and_then(|e| e.eula_at.clone()), e.as_ref().map(|_| "restored"), e.as_ref().and_then(|e| e.custom_cmd.as_ref().map(|c| serde_json::to_string(c).unwrap())), crate::db::now()])).await?;
            crate::files::patch_port(&st, &server::get(&st, &nid).await.map_err(|e| anyhow!(e.message))?, port).await;
            ctx.log(format!("new server created on port {port}"));
            st.audit(&actor, "backup.restore", &sname, "as new server", Some(&nid)).await;
            server::emit(&st, &nid).await;
        }
        Ok(())
    });
    Ok((StatusCode::ACCEPTED, Json(json!({"operation": op}))))
}

pub async fn delete(State(st): State<St>, user: User, Path(bid): Path<String>) -> ApiResult<StatusCode> {
    let (sid, ..) = find(&st, &user, &bid, "backup.manage").await?;
    remove_backup(&st, &sid, &bid).await?;
    st.audit(&user.email, "backup.delete", &bid, "", Some(&sid)).await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn download(State(st): State<St>, user: User, Path(bid): Path<String>) -> ApiResult<Response> {
    let (sid, ..) = find(&st, &user, &bid, "backup.manage").await?;
    let f = tokio::fs::File::open(archive_path(&st, &sid, &bid)).await?;
    let len = f.metadata().await?.len();
    let mut r = axum::body::Body::from_stream(tokio_util::io::ReaderStream::new(f)).into_response();
    let h = r.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/zstd"));
    h.insert(header::CONTENT_DISPOSITION, HeaderValue::from_str(&format!("attachment; filename=\"moss-backup-{bid}.tar.zst\"")).unwrap());
    h.insert(header::CONTENT_LENGTH, HeaderValue::from(len));
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_paths_must_be_plain_relative() {
        assert!(safe_entry_path(FsPath::new("world/level.dat")));
        for bad in ["../x", "/etc/passwd", "a/../b", ""] { assert!(!safe_entry_path(FsPath::new(bad)), "{bad}"); }
    }

    #[test]
    fn archive_round_trip_verifies_and_extracts() {
        let base = std::env::temp_dir().join(format!("moss-bk-{}", std::process::id()));
        let (src, out, dst) = (base.join("src"), base.join("a.tar.zst"), base.join("dst"));
        std::fs::create_dir_all(src.join("world")).unwrap();
        std::fs::create_dir_all(src.join(".moss")).unwrap();
        std::fs::write(src.join("world/level.dat"), b"data").unwrap();
        std::fs::write(src.join(".moss/secret"), b"x").unwrap();
        let tok = tokio_util::sync::CancellationToken::new();
        let (sha, _size, files) = build_archive(src.clone(), out.clone(), "{}".into(), tok.clone()).unwrap();
        assert_eq!(files, 1);
        assert!(verify_archive(&out, &sha).is_ok());
        assert!(verify_archive(&out, &"0".repeat(64)).is_err());
        extract_archive(&out, &dst, &tok).unwrap();
        assert_eq!(std::fs::read(dst.join("world/level.dat")).unwrap(), b"data");
        assert!(!dst.join(".moss").exists());
        let _ = std::fs::remove_dir_all(&base);
    }
}
