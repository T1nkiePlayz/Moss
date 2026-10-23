//! What happens to a backup after it exists: uploading to the destination (with retries and verification),
//! downloading it again when only the remote copy is left, and retention of local and remote copies.
use super::{Dest, ObjInfo, Store, Xfer, check_stored, dest, key_for};
use crate::{backups::create::archive_path, ops::{self, OpCtx, Operation}, server::Srv, state::St};
use anyhow::{Result, anyhow, bail};
use std::{path::PathBuf, time::Duration};

#[derive(Clone, Copy)]
pub struct Opts { pub attempts: u32, pub backoff: Duration }
pub const DEFAULT: Opts = Opts { attempts: 3, backoff: Duration::from_secs(5) };

#[derive(Clone, Debug)]
pub struct Row { pub server_id: String, pub size: i64, pub sha: String, pub remote: String, pub dest: Option<String>, pub key: Option<String>, pub local: bool }

pub async fn row(st: &St, id: &str) -> Result<Row> {
    let i = id.to_string();
    st.db.call(move |c| c.query_row("SELECT server_id,size,sha256,remote,remote_dest,remote_key,local_present FROM backups WHERE id=?", [i], |r| Ok(Row { server_id: r.get(0)?, size: r.get(1)?, sha: r.get(2)?, remote: r.get(3)?, dest: r.get(4)?, key: r.get(5)?, local: r.get(6)? }))).await
}

async fn set_remote(st: &St, id: &str, state: &str, dest: Option<&str>, key: Option<&str>, err: Option<&str>) {
    let (i, s, d, k, e) = (id.to_string(), state.to_string(), dest.map(String::from), key.map(String::from), err.map(|e| e.chars().take(400).collect::<String>()));
    let _ = st.db.call(move |c| c.execute("UPDATE backups SET remote=?, remote_dest=COALESCE(?,remote_dest), remote_key=COALESCE(?,remote_key), remote_error=?, remote_at=CASE WHEN ?1='stored' THEN ? ELSE remote_at END WHERE id=?",
        rusqlite::params![s, d, k, e, crate::db::now(), i])).await;
}

/// Follow the byte counter into the operation's progress bar until aborted.
fn ticker(ctx: &OpCtx, x: &Xfer, total: u64) -> tokio::task::JoinHandle<()> {
    let (c, x) = (ctx.clone(), x.clone());
    tokio::spawn(async move { loop { tokio::time::sleep(Duration::from_millis(400)).await; c.progress(Some((x.done() as f64 / total.max(1) as f64).min(0.99))); } })
}

/// After a successful local backup: mark it uploading and start the upload operation. `None` when the server has no destination.
pub async fn queue_upload(st: &St, s: &Srv, backup_id: &str) -> Result<Option<Operation>> {
    let Some(d) = dest::effective(st, &s.id).await? else { return Ok(None) };
    let key = key_for(&s.id, backup_id);
    set_remote(st, backup_id, "uploading", Some(&d.id), Some(&key), None).await;
    let (s2, bid) = (s.clone(), backup_id.to_string());
    Ok(Some(ops::spawn_op(st, "backup.upload", format!("Copy {} backup to {}", s.name, d.name), Some(&s.id), true, move |ctx| async move {
        let st = ctx.st.clone();
        let r = upload(&ctx, &st, &bid, &d, DEFAULT).await;
        if let Err(e) = &r {
            let msg = format!("{e:#}");
            set_remote(&st, &bid, "failed", None, None, Some(&if ctx.cancelled() { "cancelled".to_string() } else { msg.clone() })).await;
            if !ctx.cancelled() { crate::notify::remote_backup_failed(&st, &s2, &d.name, &msg).await; }
        }
        r
    })))
}

/// Upload, verify, record, and apply retention. Retries the whole copy with growing pauses; a copy that fails
/// verification is deleted from the destination before the next attempt.
pub async fn upload(ctx: &OpCtx, st: &St, backup_id: &str, d: &Dest, o: Opts) -> Result<()> {
    let r = row(st, backup_id).await?;
    let (path, size, key) = (archive_path(st, &r.server_id, backup_id), r.size.max(0) as u64, key_for(&r.server_id, backup_id));
    if !path.is_file() { bail!("the local archive is gone, so there is nothing to upload"); }
    let store = dest::open(st, d).await?;
    let x = Xfer::with_token(ctx.token.clone());
    let tick = ticker(ctx, &x, size);
    let mut last = anyhow!("no attempt was made");
    let mut done = false;
    for n in 1..=o.attempts.max(1) {
        ctx.check()?;
        ctx.stage(&format!("Uploading to {} (attempt {n} of {})", d.name, o.attempts));
        x.set(0);
        let res: Result<()> = async {
            store.put(&path, &key, &r.sha, &x).await?;
            ctx.stage("Verifying the copy");
            let info: ObjInfo = store.head(&key).await?.ok_or_else(|| anyhow!("the destination does not have the file after uploading"))?;
            let (p2, sha, part) = (path.clone(), r.sha.clone(), part_of(&store, size));
            tokio::task::spawn_blocking(move || check_stored(&info, size, &sha, &p2, part)).await?
        }.await;
        match res {
            Ok(()) => { done = true; break; }
            Err(e) => {
                if ctx.cancelled() { let _ = store.delete(&key).await; tick.abort(); return Err(e); }
                ctx.log(format!("attempt {n} failed: {e:#}"));
                let _ = store.delete(&key).await;
                last = e;
                if n < o.attempts { tokio::time::sleep(o.backoff * n).await; }
            }
        }
    }
    tick.abort();
    if !done { return Err(anyhow!("upload failed after {} attempts: {last:#}", o.attempts)); }
    set_remote(st, backup_id, "stored", Some(&d.id), Some(&key), None).await;
    ctx.log(format!("stored on {} and verified (size and SHA-256)", d.name));
    if let Err(e) = retention(st, &r.server_id, d).await { ctx.log(format!("retention could not finish: {e:#}")); }
    Ok(())
}

fn part_of(store: &Store, size: u64) -> Option<u64> { match store { Store::S3(s) => Some(s.part_size.unwrap_or_else(|| super::s3::part_size_for(size))), Store::Folder(_) => None } }

/// Make sure the archive is on local disk, downloading and verifying it from the destination when only the remote copy is left.
pub async fn ensure_local(ctx: &OpCtx, st: &St, backup_id: &str) -> Result<PathBuf> {
    let r = row(st, backup_id).await?;
    let path = archive_path(st, &r.server_id, backup_id);
    if r.local && path.is_file() { return Ok(path); }
    if r.remote != "stored" { bail!("this backup's file is missing and there is no remote copy to restore it from"); }
    let d = match &r.dest { Some(id) => dest::get(st, id).await?, None => None }.ok_or_else(|| anyhow!("the destination this backup was stored on has been removed"))?;
    let key = r.key.clone().unwrap_or_else(|| key_for(&r.server_id, backup_id));
    let store = dest::open(st, &d).await?;
    ctx.stage(&format!("Downloading from {}", d.name));
    if let Some(p) = path.parent() { tokio::fs::create_dir_all(p).await?; }
    let x = Xfer::with_token(ctx.token.clone());
    let tick = ticker(ctx, &x, r.size.max(0) as u64);
    let res = store.get(&key, &path, Some((&r.sha, r.size.max(0) as u64)), &x).await;
    tick.abort();
    res.map_err(|e| anyhow!("could not download the backup from {}: {e:#}", d.name))?;
    let i = backup_id.to_string();
    st.db.call(move |c| c.execute("UPDATE backups SET local_present=1 WHERE id=?", [i])).await?;
    ctx.log("downloaded and checksum verified");
    Ok(path)
}

/// Delete the remote object of a stored backup (the destination being gone counts as done).
pub async fn delete_remote(st: &St, backup_id: &str) -> Result<()> {
    let r = row(st, backup_id).await?;
    if r.remote == "uploading" { bail!("backup is being uploaded; wait for the upload to finish"); }
    if r.remote != "stored" { return Ok(()); }
    let Some(d) = (match &r.dest { Some(id) => dest::get(st, id).await?, None => None }) else { return Ok(()) };
    let store = dest::open(st, &d).await?;
    store.delete(&r.key.unwrap_or_else(|| key_for(&r.server_id, backup_id))).await
}

/// Drop a local archive whose remote copy is safe.
pub async fn drop_local(st: &St, server_id: &str, id: &str) {
    let _ = tokio::fs::remove_file(archive_path(st, server_id, id)).await;
    let i = id.to_string();
    let _ = st.db.call(move |c| c.execute("UPDATE backups SET local_present=0 WHERE id=? AND remote='stored'", [i])).await;
}

/// Local copies beyond `keep_local` go (the remote copy stays); remote copies beyond `keep_remote` go. Locked backups
/// are untouched, and the newest stored copy is never removed.
pub async fn retention(st: &St, server_id: &str, d: &Dest) -> Result<()> {
    let (sid, did, kl, kr) = (server_id.to_string(), d.id.clone(), d.keep_local, d.keep_remote.max(1));
    let (drops, victims) = st.db.call(move |c| retention_victims(c, &sid, &did, kl, kr)).await?;
    for id in drops { if !victims.iter().any(|v| v.0 == id) { drop_local(st, server_id, &id).await; } }
    if victims.is_empty() { return Ok(()); }
    let store = dest::open(st, d).await?;
    for (id, key, local) in victims {
        let key = if key.is_empty() { key_for(server_id, &id) } else { key };
        if store.delete(&key).await.is_err() { continue; } // keep the record while the object may still exist
        let i = id.clone();
        if local { let _ = st.db.call(move |c| c.execute("UPDATE backups SET remote='none', remote_dest=NULL, remote_key=NULL, remote_error=NULL, remote_at=NULL WHERE id=?", [i])).await; }
        else { let _ = st.db.call(move |c| c.execute("DELETE FROM backups WHERE id=?", [i])).await; }
    }
    Ok(())
}

/// Local drops and remote victims. Safety (`pre-change`) copies are counted apart so they never displace scheduled/manual ones.
type Victims = (Vec<String>, Vec<(String, String, bool)>);
fn retention_victims(c: &rusqlite::Connection, sid: &str, did: &str, kl: i64, kr: i64) -> rusqlite::Result<Victims> {
    let drops = if kl > 0 {
        c.prepare("SELECT id FROM backups WHERE server_id=?1 AND remote_dest=?2 AND remote='stored' AND local_present=1 AND locked=0 AND status='ok' AND kind<>'pre-change' ORDER BY created_at DESC LIMIT -1 OFFSET ?3")?
            .query_map(rusqlite::params![sid, did, kl], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?
    } else { vec![] };
    let sql = |cmp: &str| format!("SELECT id,COALESCE(remote_key,''),local_present FROM backups WHERE server_id=?1 AND remote_dest=?2 AND remote='stored' AND locked=0 AND status='ok' AND kind{cmp}'pre-change' ORDER BY created_at DESC LIMIT -1 OFFSET ?3");
    let mut victims = vec![];
    for (cmp, keep) in [("<>", kr), ("=", crate::backups::pre_change::KEEP)] {
        victims.extend(c.prepare(&sql(cmp))?.query_map(rusqlite::params![sid, did, keep], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<Vec<_>, _>>()?);
    }
    Ok((drops, victims))
}

/// At startup: uploads that were running when the controller stopped did not finish.
pub async fn recover(st: &St) {
    let _ = st.db.call(|c| {
        c.execute("UPDATE backups SET remote='failed', remote_error='the upload was interrupted by a controller restart; retry it from the backup menu' WHERE remote='uploading'", [])?;
        c.execute("UPDATE staged_restores SET status='failed', error='interrupted by a controller restart' WHERE status='preparing'", [])
    }).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> rusqlite::Connection {
        let c = rusqlite::Connection::open_in_memory().unwrap();
        crate::db::migrate_for_tests(&c);
        c.execute("INSERT INTO servers(id,name,runtime,game_version,port,created_at) VALUES ('s','s','vanilla','1.21',25565,'t')", []).ok();
        c
    }
    fn ins(c: &rusqlite::Connection, id: &str, at: &str, kind: &str) {
        c.execute("INSERT INTO backups(id,server_id,created_at,status,kind,remote,remote_dest,local_present) VALUES (?,'s',?,'ok',?,'stored','d',1)", rusqlite::params![id, at, kind]).unwrap();
    }

    #[test]
    fn safety_copies_beyond_keep_are_victims() {
        let c = db();
        for i in 0..5 { ins(&c, &format!("p{i}"), &format!("2024-02-0{i}"), "pre-change"); }
        let (_, v) = retention_victims(&c, "s", "d", 0, 10).unwrap();
        let mut ids: Vec<_> = v.into_iter().map(|x| x.0).collect(); ids.sort();
        assert_eq!(ids, vec!["p0".to_string(), "p1".to_string()], "only the oldest beyond KEEP=3");
    }

    #[test]
    fn safety_copies_do_not_displace_scheduled_ones() {
        let c = db();
        ins(&c, "s1", "2024-01-01", "scheduled"); ins(&c, "s2", "2024-01-02", "manual");
        for i in 0..3 { ins(&c, &format!("p{i}"), &format!("2024-03-0{i}"), "pre-change"); }
        let (drops, v) = retention_victims(&c, "s", "d", 2, 2).unwrap();
        assert!(v.is_empty(), "newer pre-change copies must not push the older scheduled/manual copies out: {v:?}");
        assert!(drops.is_empty());
        ins(&c, "s3", "2024-01-03", "scheduled");
        let (drops, v) = retention_victims(&c, "s", "d", 2, 2).unwrap();
        assert_eq!(v.len(), 1); assert_eq!(v[0].0, "s1"); assert_eq!(drops, vec!["s1".to_string()]);
    }
}
