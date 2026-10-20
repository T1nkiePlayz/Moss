//! Restoring a backup, either in place (safety backup, staged extraction, then a directory swap or, with `only`,
//! a swap of just the named top-level entries; the previous contents are kept) or as a brand-new server on a free port.
use super::{archive::{extract_archive, verify_archive_scan}, find, pre_change_forced, remote::jobs::ensure_local, swap::{swap_full, swap_selected, validate_only}};
use crate::{auth::User, error::{ApiError, ApiResult}, ops, server, state::St};
use anyhow::anyhow;
use axum::{Json, extract::{Path, State}, http::StatusCode};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)] pub struct RestoreReq { mode: String, only: Option<Vec<String>>, /// in-place only: extract to a staging directory and wait for an explicit promote
    staged: Option<bool> }
pub async fn restore(State(st): State<St>, user: User, Path(bid): Path<String>, Json(r): Json<RestoreReq>) -> ApiResult<(StatusCode, Json<Value>)> {
    let (sid, sha, sname, rt, gv) = find(&st, &user, &bid, "backup.manage").await?;
    let existing = server::get(&st, &sid).await.ok();
    let in_place = match r.mode.as_str() { "in_place" => true, "new_server" => false, _ => return Err(ApiError::bad("validation", "mode must be new_server or in_place")) };
    if let Some(only) = &r.only {
        if !in_place { return Err(ApiError::bad("validation", "only is for in-place restores").field("only", "not allowed with new_server")); }
        validate_only(only)?;
    }
    if in_place {
        let s = existing.as_ref().ok_or_else(|| ApiError::conflict("server.gone", "the original server no longer exists; restore as a new server"))?;
        if crate::lifecycle::process_alive(s) { return Err(ApiError::conflict("server.running", "stop the server before restoring in place")); }
    } else { user.require_admin()?; }
    if r.staged == Some(true) {
        if !in_place { return Err(ApiError::bad("validation", "staged restores are for in-place restores").field("staged", "not allowed with new_server")); }
        return super::staged::start(&st, &user, &bid, existing.unwrap(), sha, r.only).await;
    }
    let actor = user.actor();
    let b = bid.clone();
    let only = r.only.clone();
    let op = ops::spawn_op(&st, "backup.restore", format!("Restore {sname}"), Some(&sid.clone()), true, move |ctx| async move {
        let st = ctx.st.clone();
        let p = ensure_local(&ctx, &st, &b).await?;
        ctx.stage("Verifying archive before touching anything");
        let (p2, sha2) = (p.clone(), sha.clone());
        let (_, names) = tokio::task::spawn_blocking(move || verify_archive_scan(&p2, &sha2)).await??;
        if let Some(missing) = only.iter().flatten().find(|n| !names.contains(*n)) { return Err(anyhow!("\"{missing}\" is not in this backup")); }
        ctx.check()?;
        if in_place {
            let s = existing.unwrap();
            ctx.stage("Taking a safety backup of the current state");
            pre_change_forced(&st, &s, "restore", &ctx).await?;
            let _g = st.lock_for(&s.id).try_lock_owned().map_err(|_| anyhow!("another operation is changing this server"))?;
            ctx.stage("Extracting to a staging directory");
            let stage = st.cfg.servers_dir().join(format!(".restore-{}", uuid::Uuid::now_v7()));
            let (p3, stage2, tok, only2) = (p.clone(), stage.clone(), ctx.token.clone(), only.clone());
            if let Err(e) = tokio::task::spawn_blocking(move || extract_archive(&p3, &stage2, &tok, only2.as_deref())).await? { let _ = tokio::fs::remove_dir_all(&stage).await; return Err(e); }
            ctx.stage("Swapping in the restored files (the previous ones are kept)");
            let (dir, kept) = (s.dir(&st), st.cfg.data_dir.join("removed").join(format!("{}-pre-restore-{}", s.id, chrono::Utc::now().format("%Y%m%d-%H%M%S"))));
            if let Some(only) = &only {
                let (stage2, dir2, kept2, only2) = (stage.clone(), dir.clone(), kept.clone(), only.clone());
                let r = tokio::task::spawn_blocking(move || swap_selected(&stage2, &dir2, &kept2, &only2)).await?;
                let _ = tokio::fs::remove_dir_all(&stage).await;
                r?;
            } else {
                let (stage2, dir2, kept2) = (stage.clone(), dir.clone(), kept.clone());
                tokio::task::spawn_blocking(move || swap_full(&stage2, &dir2, &kept2)).await??;
            }
            ctx.log(format!("previous contents kept at {}", kept.display()));
            st.audit(&actor, "backup.restore", &s.name, &only.as_ref().map_or("in place".to_string(), |o| format!("in place: {}", o.join(", "))), Some(&s.id)).await;
        } else {
            ctx.stage("Creating a new server from the backup");
            let nid = uuid::Uuid::now_v7().to_string();
            let used: Vec<i64> = server::list_all(&st).await?.iter().map(|x| x.port).collect();
            let mut port = existing.as_ref().map_or(25565, |e| e.port);
            while used.contains(&port) || tokio::net::TcpListener::bind(("0.0.0.0", port as u16)).await.is_err() { port += 1; }
            let dir = st.cfg.servers_dir().join(&nid);
            let (p3, d2, tok) = (p.clone(), dir.clone(), ctx.token.clone());
            ctx.stage("Extracting");
            if let Err(e) = tokio::task::spawn_blocking(move || extract_archive(&p3, &d2, &tok, None)).await? { let _ = tokio::fs::remove_dir_all(&dir).await; return Err(e); }
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
