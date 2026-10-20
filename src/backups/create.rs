//! Creating and pruning backups: the `do_backup` operation (flush, archive, record) and archive/row removal.
use super::archive::{EXCLUDE_TOP, build_archive, dir_size};
use crate::{ops::OpCtx, server::Srv, state::St};
use anyhow::{Result, anyhow, bail};
use serde_json::json;
use std::path::PathBuf;

pub fn archive_path(st: &St, server_id: &str, id: &str) -> PathBuf { st.cfg.backups_dir().join(server_id).join(format!("{id}.tar.zst")) }

pub async fn do_backup(ctx: &OpCtx, st: &St, s: &Srv, kind: &str, note: &str, keep: Option<i64>) -> Result<String> {
    let _g = st.lock_for(&s.id).try_lock_owned().map_err(|_| anyhow!("another operation is changing this server"))?;
    let root = s.dir(st);
    ctx.stage("Checking free space");
    let (est, (_, free)) = (dir_size(&root), crate::platform::disk(&st.cfg.data_dir));
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

pub async fn remove_backup(st: &St, sid: &str, id: &str) -> Result<()> {
    let _ = tokio::fs::remove_file(archive_path(st, sid, id)).await;
    let i = id.to_string();
    st.db.call(move |c| c.execute("DELETE FROM backups WHERE id=?", [i])).await?;
    Ok(())
}
