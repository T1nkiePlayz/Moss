//! Staged-run bookkeeping against a real database and data directory: hiding, discarding, boot recovery, the failure
//! path of a run, plan signatures and the "can promote" rule.
use super::*;
use crate::testutil::{Reply, state, stub};

async fn origin(st: &St) -> Srv {
    let id = uuid::Uuid::now_v7().to_string();
    let i = id.clone();
    st.db.call(move |c| c.execute(
        "INSERT INTO servers(id,name,runtime,game_version,port,memory_mb,eula_at,installed,created_at) VALUES (?,?,?,?,?,?,?,1,?)",
        rusqlite::params![i, "Survival", "paper", "1.21.4", 25565, 2048, crate::db::now(), crate::db::now()])).await.unwrap();
    let s = server::get(st, &id).await.unwrap();
    std::fs::create_dir_all(s.dir(st).join("plugins")).unwrap();
    std::fs::write(s.dir(st).join("plugins/a.jar"), "jar").unwrap();
    s
}

fn req(kind: &str) -> run::Req { run::Req { kind: kind.into(), include_world: false, ready_timeout_s: None, watch_s: None, upgrade: None, pack: None } }

async fn copy(st: &St, o: &Srv, kind: &str) -> String {
    let id = uuid::Uuid::now_v7().to_string();
    run::create_copy(st, o, &id, &req(kind), 30100, "tester").await.unwrap();
    std::fs::create_dir_all(st.cfg.servers_dir().join(&id)).unwrap();
    id
}

#[tokio::test]
async fn copies_are_hidden_from_the_list_but_not_from_supervision() {
    let st = state(|_| {}).await;
    let o = origin(&st).await;
    let id = copy(&st, &o, "none").await;
    let names: Vec<String> = server::list(&st).await.unwrap().into_iter().map(|s| s.name).collect();
    assert_eq!(names, vec!["Survival".to_string()]);
    let all = server::list_all(&st).await.unwrap();
    let c = all.iter().find(|s| s.id == id).expect("the supervisor still sees the copy");
    assert_eq!((c.name.as_str(), c.port, c.autostart, c.restart_enabled, c.installed), ("Survival (staging)", 30100, false, false, true));
    assert!(c.eula_at.is_some(), "the copy inherits the accepted EULA so it can start");
    assert!(server::is_staging(&st, &id).await && !server::is_staging(&st, &o.id).await);
}

#[tokio::test]
async fn the_copy_gets_its_own_inventory_rows() {
    let st = state(|_| {}).await;
    let o = origin(&st).await;
    let oid = o.id.clone();
    st.db.call(move |c| c.execute("INSERT INTO content_items(id,server_id,provider,project_id,version_id,title,version_number,filename,kind,installed_at,pinned,dir) VALUES ('i1',?,'modrinth','p','v1','A','1','a.jar','plugins','t',1,'plugins')", [oid])).await.unwrap();
    let id = copy(&st, &o, "update_all").await;
    let items = db_items(&st, &id).await.unwrap();
    assert_eq!(items.len(), 1);
    assert_ne!(items[0]["id"], "i1", "a new row, so changing the copy never edits the original's inventory");
    assert_eq!((items[0]["pinned"].as_bool(), items[0]["project_id"].as_str()), (Some(true), Some("p")));
    assert_eq!(db_items(&st, &o.id).await.unwrap().len(), 1);
}

#[tokio::test]
async fn discard_removes_files_rows_and_live_state() {
    let st = state(|_| {}).await;
    let o = origin(&st).await;
    let id = copy(&st, &o, "none").await;
    std::fs::write(st.cfg.servers_dir().join(&id).join("x"), "1").unwrap();
    st.timeline(&id, "start", "").await;
    st.live.lock().unwrap().entry(id.clone()).or_default().crashed = true;
    discard(&st, &id).await.unwrap();
    assert!(get_run(&st, &id).await.is_none());
    assert!(!st.cfg.servers_dir().join(&id).exists());
    assert!(server::get(&st, &id).await.is_err());
    assert!(st.live.lock().unwrap().get(&id).is_none());
    let n: i64 = st.db.call({ let i = id.clone(); move |c| c.query_row("SELECT COUNT(*) FROM timeline WHERE server_id=?", [i], |r| r.get(0)) }).await.unwrap();
    assert_eq!(n, 0);
    assert!(o.dir(&st).join("plugins/a.jar").is_file(), "the real server is untouched");
    discard(&st, &id).await.unwrap(); // discarding twice is fine
}

#[tokio::test]
async fn boot_recovery_removes_interrupted_and_expired_copies_only() {
    let st = state(|_| {}).await;
    let o = origin(&st).await;
    let (building, testing, fresh, old) = (copy(&st, &o, "none").await, copy(&st, &o, "none").await, copy(&st, &o, "none").await, copy(&st, &o, "none").await);
    set_state(&st, &testing, "testing", None).await;
    set_state(&st, &fresh, "healthy", Some(json!({"healthy": true}))).await;
    set_state(&st, &old, "unhealthy", None).await;
    let past = (chrono::Utc::now() - chrono::Duration::hours(1)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let i = old.clone();
    st.db.call(move |c| c.execute("UPDATE staging_runs SET expires_at=? WHERE id=?", rusqlite::params![past, i])).await.unwrap();
    recover(&st).await.unwrap();
    let left: Vec<String> = runs_for(&st, &o.id).await.into_iter().map(|r| r.id).collect();
    assert_eq!(left, vec![fresh.clone()], "building ({building}), testing and expired runs are gone; the fresh finished one stays for inspection");
    assert!(st.cfg.servers_dir().join(&fresh).is_dir() && !st.cfg.servers_dir().join(&building).exists());
}

#[tokio::test]
async fn only_a_fresh_healthy_run_with_a_change_can_be_promoted() {
    let st = state(|_| {}).await;
    let o = origin(&st).await;
    let id = copy(&st, &o, "update_all").await;
    assert_eq!(get_run(&st, &id).await.unwrap().json()["can_promote"], false, "still building");
    set_state(&st, &id, "healthy", Some(json!({"healthy": true}))).await;
    let r = get_run(&st, &id).await.unwrap().json();
    assert_eq!((r["can_promote"].as_bool(), r["expired"].as_bool()), (Some(true), Some(false)));
    let past = (chrono::Utc::now() - chrono::Duration::minutes(5)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let i = id.clone();
    st.db.call(move |c| c.execute("UPDATE staging_runs SET expires_at=? WHERE id=?", rusqlite::params![past, i])).await.unwrap();
    let r = get_run(&st, &id).await.unwrap().json();
    assert_eq!((r["can_promote"].as_bool(), r["expired"].as_bool()), (Some(false), Some(true)));
    let none = copy(&st, &o, "none").await;
    set_state(&st, &none, "healthy", None).await;
    assert_eq!(get_run(&st, &none).await.unwrap().json()["can_promote"], false, "a plain test has nothing to apply");
}

#[tokio::test]
async fn a_run_with_nothing_to_update_fails_cleanly_without_copying() {
    let s = stub(|_, _| Reply::json(200, json!([]))).await;
    let b = format!("{}/v2", s.base);
    let st = state(move |c| c.modrinth_api_url = b).await;
    let o = origin(&st).await;
    let oid = o.id.clone();
    st.db.call(move |c| c.execute("INSERT INTO content_items(id,server_id,provider,project_id,version_id,title,version_number,filename,kind,installed_at,pinned,dir) VALUES ('i1',?,'modrinth','p','v1','A','1','a.jar','plugins','t',0,'plugins')", [oid])).await.unwrap();
    let id = uuid::Uuid::now_v7().to_string();
    let rq = req("update_all");
    run::create_copy(&st, &o, &id, &rq, 30101, "tester").await.unwrap();
    let e = run::test(OpCtx::detached(&st), id.clone(), o.clone(), rq, None, "tester".into()).await.unwrap_err();
    assert!(format!("{e:#}").contains("no content updates"), "{e:#}");
    let r = get_run(&st, &id).await.unwrap();
    assert_eq!(r.state, "failed");
    assert!(r.report["reasons"][0].as_str().unwrap().contains("no content updates"));
    assert!(!st.cfg.servers_dir().join(&id).exists(), "nothing was copied for a run that had nothing to do");
    discard(&st, &id).await.unwrap();
}

#[test]
fn plan_signatures_ignore_order_and_distinguish_versions() {
    use crate::content::update_all::plan_signature;
    let step = |p: &str, v: &str| Step { prov: Prov::Modrinth, action: "replace", project_id: p.into(), title: p.into(), version: json!({"id": v}), file: json!({}), replaces: None, reason: String::new() };
    assert_eq!(plan_signature(&[step("a", "1"), step("b", "2")]), plan_signature(&[step("b", "2"), step("a", "1")]));
    assert_ne!(plan_signature(&[step("a", "1")]), plan_signature(&[step("a", "2")]));
}

#[test]
fn test_copies_never_expose_rcon_or_query() {
    let d = std::env::temp_dir().join(format!("moss-sandbox-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("server.properties"), "server-port=25565\nenable-rcon=true\nrcon.password=secret\nmotd=hi\n").unwrap();
    sandbox_properties(&d);
    let p = std::fs::read_to_string(d.join("server.properties")).unwrap();
    assert!(p.contains("enable-rcon=false") && p.contains("enable-query=false") && p.contains("motd=hi") && !p.contains("enable-rcon=true"), "{p}");
    let _ = std::fs::remove_dir_all(&d);
}

#[tokio::test]
async fn the_copy_gets_its_own_pack_records_and_a_discarded_copy_removes_the_kept_pack_file() {
    let st = state(|_| {}).await;
    let o = origin(&st).await;
    let (sid, now) = (o.id.clone(), crate::db::now());
    st.db.call(move |c| {
        c.execute("INSERT INTO content_items(id,server_id,provider,project_id,version_id,title,version_number,filename,kind,installed_at,pinned,dir) VALUES ('i1',?,'modrinth','p','v','T','1','a.jar','plugins',?,0,'plugins')", rusqlite::params![sid, now])?;
        c.execute("INSERT INTO modpack_installs(id,server_id,name,format,game_version,installed_at,pinned,provider,project_id,version_id) VALUES ('k1',?,'Pack','mrpack','1.21',?,0,'modrinth','P','pv')", rusqlite::params![sid, now])?;
        c.execute("INSERT INTO modpack_items(content_item_id,install_id) VALUES ('i1','k1')", [])?;
        c.execute("INSERT INTO modpack_files(install_id,path,sha256) VALUES ('k1','config/x.toml','abc')", [])
    }).await.unwrap();
    let id = copy(&st, &o, "pack_update").await;
    let (n, linked, sums): (String, String, i64) = st.db.call({ let i = id.clone(); move |c| {
        let n: String = c.query_row("SELECT id FROM modpack_installs WHERE server_id=?", [&i], |r| r.get(0))?;
        let l: String = c.query_row("SELECT content_item_id FROM modpack_items WHERE install_id=?", [&n], |r| r.get(0))?;
        let s: i64 = c.query_row("SELECT COUNT(*) FROM modpack_files WHERE install_id=?", [&n], |r| r.get(0))?;
        Ok((n, l, s))
    } }).await.unwrap();
    assert_eq!(n, format!("{id}:k1"));
    assert_eq!(linked, format!("{id}:i1"));
    assert_eq!(sums, 1);
    let kept = run::pack_file(&st, &id);
    std::fs::create_dir_all(kept.parent().unwrap()).unwrap();
    std::fs::write(&kept, "pack").unwrap();
    discard(&st, &id).await.unwrap();
    assert!(!kept.exists());
    let left: i64 = st.db.call(|c| c.query_row("SELECT COUNT(*) FROM modpack_installs WHERE id LIKE '%:k1'", [], |r| r.get(0))).await.unwrap();
    assert_eq!(left, 0, "the copy's pack rows go with its server row");
}
