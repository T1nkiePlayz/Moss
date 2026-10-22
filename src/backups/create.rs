//! Creating and pruning backups: the `do_backup` operation (flush, archive, record) and archive/row removal.
use super::archive::{EXCLUDE_TOP, build_archive, dir_size, worlds_of};
use crate::{ops::OpCtx, server::Srv, state::St};
use anyhow::{Result, anyhow, bail};
use serde_json::json;
use std::path::PathBuf;

pub fn archive_path(st: &St, server_id: &str, id: &str) -> PathBuf { st.cfg.backups_dir().join(server_id).join(format!("{id}.tar.zst")) }

/// Create a backup, holding the server's operation lock. A failure raises a `backup_failed` notification.
pub async fn do_backup(ctx: &OpCtx, st: &St, s: &Srv, kind: &str, note: &str, keep: Option<i64>) -> Result<String> {
    let _g = st.lock_for(&s.id).try_lock_owned().map_err(|_| anyhow!("another operation is changing this server"))?;
    do_backup_locked(ctx, st, s, kind, note, keep).await
}

/// Like `do_backup` for callers that already hold the server's operation lock.
pub async fn do_backup_locked(ctx: &OpCtx, st: &St, s: &Srv, kind: &str, note: &str, keep: Option<i64>) -> Result<String> {
    let r = backup_inner(ctx, st, s, kind, note, keep).await;
    if let Err(e) = &r && !ctx.cancelled() { crate::notify::backup_failed(st, s, &format!("{e:#}")).await; }
    r
}

async fn backup_inner(ctx: &OpCtx, st: &St, s: &Srv, kind: &str, note: &str, keep: Option<i64>) -> Result<String> {
    let root = s.dir(st);
    ctx.stage("Checking free space");
    let (est, free) = { let (r, d) = (root.clone(), st.cfg.data_dir.clone()); tokio::task::spawn_blocking(move || (dir_size(&r), crate::platform::disk(&d).1)).await? };
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
    } else if running && let Some(f) = s.game().and_then(|g| g.flush()) {
        ctx.stage("Flushing world to disk");
        consistent = flush_game(st, s, f).await;
        if !consistent { ctx.log("world flush was not confirmed; the backup may be crash-consistent only"); }
    } else if running {
        consistent = false;
        if s.is_game() { ctx.log("this game has no console save command; the world was copied while the server was running"); }
    }
    let mut worlds: Vec<String> = vec![];
    let result: Result<(String, u64, u64)> = async {
        ctx.stage("Compressing");
        let dir = st.cfg.backups_dir().join(&s.id);
        tokio::fs::create_dir_all(&dir).await?;
        let (out, part) = (archive_path(st, &s.id, &id), dir.join(format!("{id}.part")));
        let manifest = json!({"moss": env!("CARGO_PKG_VERSION"), "backup_id": id, "server_id": s.id, "server_name": s.name, "runtime": s.runtime, "game_version": s.game_version, "created_at": crate::db::now(), "consistent": consistent, "excluded": EXCLUDE_TOP}).to_string();
        let ticker = { let c = ctx.clone(); let p = part.clone(); tokio::spawn(async move { loop { tokio::time::sleep(std::time::Duration::from_millis(500)).await; let done = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0) as f64; c.progress(Some(if est == 0 { 0.5 } else { (done / (est as f64 * 0.5 + 1.0)).min(0.95) })); } }) };
        let root3 = root.clone();
        let w = tokio::task::spawn_blocking(move || worlds_of(&root3));
        // A large file the game truncated mid-backup cannot be archived at its old length: start over a couple of times.
        let mut attempt = 0;
        let r = loop {
            let (root2, part2, token, manifest) = (root.clone(), part.clone(), ctx.token.clone(), manifest.clone());
            let r = tokio::task::spawn_blocking(move || build_archive(root2, part2, manifest, token)).await.map_err(|e| anyhow!(e)).and_then(|r| r);
            attempt += 1;
            match r {
                Err(e) if attempt < 3 && format!("{e:#}").contains(super::archive::SHRANK) => { ctx.log(format!("{e:#}; starting the archive again")); }
                r => break r.map_err(|e| if format!("{e:#}").contains(super::archive::SHRANK) { anyhow!("{e:#}; try again when the server is quieter") } else { e }),
            }
        };
        worlds = w.await.unwrap_or_default();
        ticker.abort();
        match r { Ok(v) => { tokio::fs::rename(&part, &out).await?; Ok(v) } Err(e) => { let _ = tokio::fs::remove_file(&part).await; Err(e) } }
    }.await;
    if running && s.is_mc() { let _ = crate::lifecycle::send_command(st, s, "save-on").await; }
    else if running && let Some(after) = s.game().and_then(|g| g.flush()).and_then(|f| f.after) { let _ = crate::lifecycle::send_command(st, s, after).await; }
    match result {
        Ok((sha, size, files)) => {
            let (i3, sha2, w) = (id.clone(), sha.clone(), serde_json::to_string(&worlds).unwrap_or_else(|_| "[]".into()));
            st.db.call(move |c| c.execute("UPDATE backups SET status='ok', size=?, sha256=?, files=?, consistent=?, worlds=? WHERE id=?", rusqlite::params![size as i64, sha2, files as i64, consistent, w, i3])).await?;
            ctx.log(format!("{} files, {} MiB, sha256 {}", files, size >> 20, &sha[..16]));
            st.timeline(&s.id, "backup", kind).await;
            if kind != "pre-change" { // safety backups are short-lived and stay local
                match super::remote::jobs::queue_upload(st, s, &id).await { Ok(Some(_)) => ctx.log("copying to the remote destination in a separate operation"), Ok(None) => {} Err(e) => ctx.log(format!("could not start the remote copy: {e:#}")) }
            }
            if kind == "scheduled" { prune(st, &s.id, "scheduled", keep.unwrap_or(0)).await; }
            if kind == "pre-change" { prune(st, &s.id, "pre-change", super::pre_change::KEEP).await; }
            Ok(id)
        }
        Err(e) => {
            let i3 = id.clone();
            let _ = st.db.call(move |c| c.execute("UPDATE backups SET status='failed' WHERE id=?", [i3])).await;
            Err(e)
        }
    }
}

/// Send a game's flush commands and wait up to about 10 s for its "saved" line. The caller sends `after` (resume) itself.
async fn flush_game(st: &St, s: &Srv, f: &crate::games::Flush) -> bool {
    let hub = crate::console::hub(st, &s.id);
    let before = hub.ring.lock().unwrap().0;
    for c in f.pre { if crate::lifecycle::send_command(st, s, c).await.is_err() { return false; } }
    for i in 0..50 {
        if i > 0 && i % 10 == 0 && let Some(q) = f.query { let _ = crate::lifecycle::send_command(st, s, q).await; } else if i == 0 && let Some(q) = f.query { let _ = crate::lifecycle::send_command(st, s, q).await; }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        if hub.ring.lock().unwrap().1.iter().any(|(seq, l)| *seq > before && l.contains(f.ready)) { return true; }
    }
    false
}

/// Keep the newest `keep` backups of `kind` (0 = the `backup_keep` setting), never touching locked ones nor the last verified-good one.
async fn prune(st: &St, sid: &str, kind: &str, keep: i64) {
    let keep = if keep > 0 { keep } else { st.setting("backup_keep", "7").await.parse().unwrap_or(7) }.max(1);
    let (s2, k2) = (sid.to_string(), kind.to_string());
    let victims: Vec<String> = st.db.call(move |c| prune_victims(c, &s2, &k2, keep)).await.unwrap_or_default();
    for v in victims { retire_local(st, sid, &v).await; }
}

/// The ok backups beyond the newest `keep` ok ones, plus failed rows beyond the newest few (failed rows never count toward `keep`).
fn prune_victims(c: &rusqlite::Connection, sid: &str, kind: &str, keep: i64) -> rusqlite::Result<Vec<String>> {
    let mut q = c.prepare("SELECT id FROM (SELECT id, locked FROM backups WHERE server_id=?1 AND kind=?3 AND local_present=1 AND status='ok' ORDER BY created_at DESC LIMIT -1 OFFSET ?2) WHERE locked=0 AND id <> COALESCE((SELECT id FROM backups WHERE server_id=?1 AND status='ok' ORDER BY created_at DESC LIMIT 1), '')")?;
    let mut v: Vec<String> = q.query_map(rusqlite::params![sid, keep, kind], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    let mut f = c.prepare("SELECT id FROM backups WHERE server_id=?1 AND kind=?3 AND locked=0 AND local_present=1 AND status='failed' ORDER BY created_at DESC LIMIT -1 OFFSET ?2")?;
    v.extend(f.query_map(rusqlite::params![sid, 3, kind], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?);
    Ok(v)
}

/// Retention for local copies: a backup that is safely on a remote destination only loses its local file.
async fn retire_local(st: &St, sid: &str, id: &str) {
    let i = id.to_string();
    let stored = st.db.call(move |c| c.query_row("SELECT remote='stored' FROM backups WHERE id=?", [i], |r| r.get::<_, bool>(0))).await.unwrap_or(false);
    if stored { super::remote::jobs::drop_local(st, sid, id).await; } else { let _ = remove_backup(st, sid, id).await; }
}

/// Delete an archive and its row. Locked backups are refused.
pub async fn remove_backup(st: &St, sid: &str, id: &str) -> Result<()> {
    let i = id.to_string();
    if st.db.call(move |c| c.query_row("SELECT locked FROM backups WHERE id=?", [i], |r| r.get::<_, bool>(0))).await.unwrap_or(false) { bail!("backup is locked"); }
    let i = id.to_string();
    if st.db.call(move |c| c.query_row("SELECT remote='uploading' FROM backups WHERE id=?", [i], |r| r.get::<_, bool>(0))).await.unwrap_or(false) { bail!("backup is being uploaded; wait for the upload to finish"); }
    let _ = tokio::fs::remove_file(archive_path(st, sid, id)).await;
    let i = id.to_string();
    st.db.call(move |c| c.execute("DELETE FROM backups WHERE id=?", [i])).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_rows_do_not_push_ok_backups_out_of_the_keep_window() {
        let c = rusqlite::Connection::open_in_memory().unwrap();
        crate::db::migrate_for_tests(&c);
        c.execute("INSERT INTO servers(id,name,runtime,game_version,port,created_at) VALUES ('s','s','vanilla','1.21',25565,'t')", []).ok();
        let ins = |id: &str, at: &str, status: &str| c.execute("INSERT INTO backups(id,server_id,created_at,status,kind) VALUES (?,'s',?,?,'scheduled')", rusqlite::params![id, at, status]).unwrap();
        ins("ok1", "2024-01-01", "ok"); ins("ok2", "2024-01-02", "ok"); ins("ok3", "2024-01-03", "ok");
        for i in 0..5 { ins(&format!("f{i}"), &format!("2024-01-1{i}"), "failed"); }
        let v = prune_victims(&c, "s", "scheduled", 2).unwrap();
        assert!(v.contains(&"ok1".to_string()), "oldest ok beyond keep=2 goes");
        assert!(!v.contains(&"ok2".to_string()) && !v.contains(&"ok3".to_string()), "newest 2 ok are kept");
        assert_eq!(v.iter().filter(|x| x.starts_with('f')).count(), 2, "failed rows beyond the newest 3 go");
    }
}
