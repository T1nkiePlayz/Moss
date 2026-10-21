use super::{eval::*, run, *};
use crate::workflows;
use crate::{testutil, webhooks::Ev};
use std::time::{Duration, Instant};

fn ev(kind: &'static str, data: Value) -> Ev { Ev { kind, server: Some(("s1".into(), "Survival".into())), data } }
fn joined(player: &str, online: u32) -> Ev { ev("player.joined", json!({"player": player, "online": online, "max": 20})) }
fn cond(field: &str, op: &str, value: Value) -> Value { json!({"field": field, "op": op, "value": value}) }

#[test]
fn conditions_compare_numbers_and_text() {
    let e = joined("Steve", 5);
    let holds = |c: Value| condition_holds(Some(&c), &e);
    assert!(condition_holds(None, &e) && condition_holds(Some(&Value::Null), &e), "no condition always holds");
    assert!(holds(cond("players.online", ">=", json!(5))) && !holds(cond("players.online", ">", json!(5))));
    assert!(holds(cond("players.online", "<", json!("6"))), "numbers may be given as text");
    assert!(holds(cond("players.max", "==", json!(20))) && holds(cond("players.max", "!=", json!(19))));
    assert!(holds(cond("player", "==", json!("steve"))), "text is compared without case");
    assert!(holds(cond("player", "contains", json!("tev"))) && holds(cond("player", "not_contains", json!("xyz"))));
    assert!(holds(cond("server", "==", json!("Survival"))));
    assert!(holds(cond("data.online", "==", json!(5))), "any payload field by path");
    // a field the event does not carry
    assert!(!holds(cond("data.nothing", "==", json!(1))) && !holds(cond("data.nothing", "contains", json!("a"))));
    assert!(holds(cond("data.nothing", "!=", json!(1))) && holds(cond("data.nothing", "not_contains", json!("a"))));
    // detail searches every text in the payload
    let crash = ev("server.crashed", json!({"detail": "java.lang.OutOfMemoryError: heap"}));
    assert!(condition_holds(Some(&cond("detail", "contains", json!("outofmemory"))), &crash));
    assert!(!condition_holds(Some(&cond("detail", "contains", json!("watchdog"))), &crash));
    // a non-numeric value never satisfies an ordering
    assert!(!holds(cond("player", ">", json!(1))));
}

#[test]
fn condition_shapes_are_validated() {
    assert!(validate_condition(&cond("players.online", ">=", json!(3))).is_ok());
    assert!(validate_condition(&cond("data.x.y", "contains", json!("a"))).is_ok());
    assert!(validate_condition(&cond("nope", "==", json!(1))).is_err());
    assert!(validate_condition(&cond("player", "~=", json!(1))).is_err());
    assert!(validate_condition(&cond("players.online", ">=", json!("many"))).is_err());
    assert!(validate_condition(&cond("player", "==", json!(["a"]))).is_err());
}

#[test]
fn placeholders_are_replaced_and_cleaned() {
    let e = joined("Steve", 3);
    assert_eq!(substitute("say Welcome {player} to {server} ({online}/{max}) [{event}]", &e), "say Welcome Steve to Survival (3/20) [player.joined]");
    assert_eq!(substitute("{unknown} {player", &e), "{unknown} {player", "unknown or unclosed placeholders stay as written");
    assert_eq!(substitute("hi {player}", &ev("server.started", json!({}))), "hi ", "a known placeholder the event lacks becomes empty");
    let evil = ev("player.joined", json!({"player": "a\nstop\r\u{0}b"}));
    assert_eq!(substitute("say {player}", &evil), "say astopb", "a value can never add a second console line");
    assert_eq!(substitute("no braces", &e), "no braces");
}

#[test]
fn rate_limit_allows_ten_runs_per_ten_minutes() {
    let (mut l, t0) = (Limiter::default(), Instant::now());
    for i in 0..RATE_MAX { assert!(l.allow("w", t0 + Duration::from_secs(i as u64)), "run {i}"); }
    assert!(!l.allow("w", t0 + Duration::from_secs(30)), "the 11th inside the window is skipped");
    assert!(l.allow("other", t0 + Duration::from_secs(30)), "limits are per workflow");
    assert!(l.allow("w", t0 + RATE_WINDOW + Duration::from_secs(1)), "old runs age out");
}

#[test]
fn events_right_after_a_workflow_acted_are_suppressed() {
    let (mut g, t0) = (LoopGuard::default(), Instant::now());
    assert!(!g.suppressed("w1", "s1", t0));
    g.begin("w1", "s1", t0);
    assert!(g.suppressed("w1", "s1", t0 + Duration::from_secs(5)));
    assert!(!g.suppressed("w1", "s2", t0 + Duration::from_secs(5)), "only the server acted on");
    assert!(!g.suppressed("w2", "s1", t0 + Duration::from_secs(5)), "only the acting workflow is suppressed");
    // a long run keeps suppressing while it is running; the window starts when it finishes
    let late = t0 + Duration::from_secs(1000);
    assert!(g.suppressed("w1", "s1", late), "still running");
    g.finish("w1", "s1", late);
    assert!(g.suppressed("w1", "s1", late + Duration::from_secs(100)));
    assert!(!g.suppressed("w1", "s1", late + LOOP_WINDOW + Duration::from_secs(1)));
}

#[test]
fn definitions_are_validated() {
    let ok = |t: &str, c: Option<&Value>, a: Vec<Action>| validate(t, c, &a, false);
    let cmd = |c: &str| Action::Command { server: "trigger".into(), command: c.into() };
    assert!(ok("server.started", None, vec![cmd("say hi {server}")]).is_ok());
    assert_eq!(ok("nope", None, vec![cmd("x")]).unwrap_err().0, "trigger");
    assert_eq!(ok("server.started", None, vec![]).unwrap_err().0, "actions");
    assert_eq!(ok("server.started", None, vec![cmd("a\nb")]).unwrap_err().0, "actions[0]");
    assert!(ok("server.started", None, vec![Action::Wait { seconds: 601 }]).is_err() && ok("server.started", None, vec![Action::Wait { seconds: 600 }]).is_ok());
    assert!(ok("update.available", None, vec![Action::Restart { server: "trigger".into() }]).is_err(), "no trigger server for that event");
    assert!(ok("server.started", None, vec![Action::Webhook { url: Some("http://example.com/x".into()), endpoint_id: None }]).is_err());
    assert!(ok("server.started", None, vec![Action::Webhook { url: Some("https://127.0.0.1/x".into()), endpoint_id: None }]).is_err());
    assert!(ok("server.started", None, vec![Action::Webhook { url: Some("https://example.com/x".into()), endpoint_id: None }]).is_ok());
    assert!(ok("server.started", Some(&cond("bad", "==", json!(1))), vec![cmd("x")]).is_err());
    assert!(serde_json::from_value::<Action>(json!({"type": "explode"})).is_err());
    assert_eq!(serde_json::from_value::<Action>(json!({"type": "restart"})).unwrap(), Action::Restart { server: "trigger".into() });
}

#[test]
fn requirements_follow_the_actions() {
    let acts = vec![Action::Restart { server: "trigger".into() }, Action::Backup { server: "s2".into() }, Action::Notify { title: "t".into(), body: String::new() }];
    let r = run::requirements(&acts, &["s1".into()]);
    assert!(r.contains(&("s1".into(), "server.power")) && r.contains(&("s2".into(), "backup.manage")) && r.contains(&("s2".into(), "server.view")));
    assert!(!r.contains(&("s2".into(), "server.power")));
}

async fn add_user(st: &St, id: &str, role: &str) {
    let (id, role) = (id.to_string(), role.to_string());
    st.db.call(move |c| c.execute("INSERT INTO users(id,email,role,linked,created_at) VALUES (?,?,?,1,?)", rusqlite::params![id, format!("{id}@x.test"), role, crate::db::now()]).map(|_| ())).await.unwrap();
}
async fn add_server(st: &St, id: &str) {
    let id = id.to_string();
    st.db.call(move |c| c.execute("INSERT INTO servers(id,name,runtime,game_version,port,memory_mb,created_at) VALUES (?,?,?,?,?,?,?)", rusqlite::params![id, format!("srv-{id}"), "sim", "sim-1.0", 25565, 512, crate::db::now()]).map(|_| ())).await.unwrap();
}
fn wf(by: &str, trigger: &str, servers: &[&str], actions: Vec<Action>) -> Workflow {
    Workflow { id: uuid::Uuid::now_v7().to_string(), name: "t".into(), enabled: true, trigger: trigger.into(), server_ids: servers.iter().map(|s| s.to_string()).collect(), condition: None, actions,
        created_by: by.into(), created_at: crate::db::now(), last_run_at: None, last_result: None, disabled_reason: None }
}
async fn wait_runs(st: &St, id: &str, n: usize) -> Vec<Value> {
    for _ in 0..100 {
        let r = workflows::runs(st, id).await.unwrap();
        if r.len() >= n { return r; }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("no run recorded");
}

#[tokio::test]
async fn permissions_are_rechecked_at_run_time_and_the_workflow_is_disabled() {
    let st = testutil::state(|_| {}).await;
    add_user(&st, "u1", "user").await;
    add_server(&st, "s1").await;
    add_server(&st, "s2").await;
    st.db.call(|c| c.execute("INSERT INTO grants(server_id,user_id,bundle) VALUES ('s1','u1','manager')", []).map(|_| ())).await.unwrap();
    let u = run::load_user(&st, "u1").await.unwrap();
    let acts = vec![Action::Restart { server: "trigger".into() }];
    assert!(run::permission_problem(&st, &u, &acts, &["s1".into()]).await.is_none());
    assert!(run::permission_problem(&st, &u, &acts, &["s2".into()]).await.is_some(), "no grant on s2");
    // a viewer cannot restart
    st.db.call(|c| c.execute("UPDATE grants SET bundle='viewer' WHERE user_id='u1'", []).map(|_| ())).await.unwrap();
    assert!(run::permission_problem(&st, &u, &acts, &["s1".into()]).await.unwrap().contains("server.power"));
    st.db.call(|c| c.execute("UPDATE grants SET bundle='manager' WHERE user_id='u1'", []).map(|_| ())).await.unwrap();

    let w = wf("u1", "server.crashed", &["s1"], vec![Action::Notify { title: "crashed".into(), body: String::new() }, Action::Restart { server: "trigger".into() }]);
    workflows::insert(&st, &w).await.unwrap();
    // access revoked after the workflow was saved
    st.db.call(|c| c.execute("DELETE FROM grants WHERE user_id='u1'", []).map(|_| ())).await.unwrap();
    run::run_one(st.clone(), w.clone(), ev("server.crashed", json!({}))).await;
    let after = workflows::get(&st, &w.id).await.unwrap().unwrap();
    assert!(!after.enabled, "disabled");
    assert!(after.disabled_reason.as_deref().is_some_and(|r| r.contains("no longer has the access")), "{:?}", after.disabled_reason);
    let runs = workflows::runs(&st, &w.id).await.unwrap();
    assert_eq!((runs.len(), runs[0]["ok"].clone()), (1, json!(false)));
    assert!(!runs[0]["log"].to_string().contains("step 1 ok"), "nothing ran: {runs:?}");
    // a deleted creator disables too
    let w2 = wf("ghost", "server.crashed", &[], vec![Action::Notify { title: "x".into(), body: String::new() }]);
    workflows::insert(&st, &w2).await.unwrap();
    run::run_one(st.clone(), w2.clone(), ev("server.crashed", json!({}))).await;
    assert!(!workflows::get(&st, &w2.id).await.unwrap().unwrap().enabled);
}

#[tokio::test]
async fn matching_events_run_actions_and_loops_and_floods_are_stopped() {
    let st = testutil::state(|_| {}).await;
    add_user(&st, "adm", "admin").await;
    add_server(&st, "L1").await;
    add_server(&st, "L2").await;
    add_server(&st, "R1").await;
    let note = |t: &str| vec![Action::Notify { title: t.into(), body: "{player} on {server}".into() }];
    // condition + scope
    let mut w = wf("adm", "player.joined", &["L1"], note("hello {player}"));
    w.condition = Some(cond("players.online", ">=", json!(3)));
    workflows::insert(&st, &w).await.unwrap();
    let on = |sid: &str, name: &str, online: u32| Ev { kind: "player.joined", server: Some((sid.into(), name.into())), data: json!({"player": "Steve", "online": online, "max": 20}) };
    run::handle(&st, &on("L1", "srv-L1", 2)).await; // condition false
    run::handle(&st, &on("L2", "srv-L2", 9)).await; // other server
    run::handle(&st, &on("L1", "srv-L1", 3)).await;
    let r = wait_runs(&st, &w.id, 1).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(workflows::runs(&st, &w.id).await.unwrap().len(), 1, "only the matching event ran");
    assert_eq!(r[0]["ok"], json!(true));
    let n: String = st.db.call(|c| c.query_row("SELECT title FROM notifications WHERE kind='workflow'", [], |r| r.get(0))).await.unwrap();
    assert_eq!(n, "Workflow \u{201c}t\u{201d} (adm@x.test): hello Steve", "attributed to the workflow and its creator");

    // loop protection: a workflow just acted on L2, so events about L2 trigger nothing
    let w2 = wf("adm", "player.joined", &["L2"], note("loop"));
    workflows::insert(&st, &w2).await.unwrap();
    run::handle(&st, &on("L2", "srv-L2", 1)).await;
    assert_eq!(wait_runs(&st, &w2.id, 1).await.len(), 1);
    // simulate the workflow's own action on L2 (what exec_one does), then the event it causes
    run::mark_for_test(&w2.id, "L2");
    // another workflow on the same server still gets the event (e.g. a crash alert)
    let w2b = wf("adm", "player.joined", &["L2"], note("other"));
    workflows::insert(&st, &w2b).await.unwrap();
    run::handle(&st, &on("L2", "srv-L2", 1)).await;
    assert_eq!(wait_runs(&st, &w2b.id, 1).await.len(), 1, "other workflows are not suppressed");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(workflows::runs(&st, &w2.id).await.unwrap().len(), 1, "the event caused by a workflow did not retrigger");

    // rate limit: 10 runs, the rest are skipped and logged
    let w3 = wf("adm", "player.joined", &["R1"], note("flood"));
    workflows::insert(&st, &w3).await.unwrap();
    for _ in 0..12 { run::handle(&st, &on("R1", "srv-R1", 1)).await; }
    let runs = wait_runs(&st, &w3.id, 12).await;
    let skipped = runs.iter().filter(|r| r["log"].to_string().contains("rate limit")).count();
    assert_eq!((runs.len(), skipped), (12, 2), "{runs:?}");
    // the allowed runs are spawned and may finish after the skips, so check last_result on a skip made after they all landed
    run::handle(&st, &on("R1", "srv-R1", 1)).await;
    let last = workflows::get(&st, &w3.id).await.unwrap().unwrap().last_result.unwrap();
    assert!(last.contains("rate limit"), "{last}");
}

#[tokio::test]
async fn dry_run_describes_without_executing() {
    let st = testutil::state(|_| {}).await;
    add_user(&st, "adm", "admin").await;
    add_server(&st, "d1").await;
    let w = wf("adm", "player.joined", &["d1"], vec![Action::Command { server: "trigger".into(), command: "say hi {player}".into() }, Action::Restart { server: "gone".into() }]);
    let p = run::plan(&st, &w, &run::sample_event("player.joined", Some(("d1".into(), "srv-d1".into())))).await;
    assert_eq!((p["executed"].clone(), p["would_run"].clone()), (json!(false), json!(false)), "second step targets a missing server");
    assert_eq!(p["steps"][0]["description"], "Send to srv-d1: say hi Steve");
    assert_eq!(p["steps"][0]["allowed"], json!(true));
    assert_eq!(p["steps"][1]["problem"], "server not found");
    assert!(workflows::runs(&st, &w.id).await.unwrap().is_empty());
}

#[tokio::test]
async fn only_the_last_fifty_runs_are_kept() {
    let st = testutil::state(|_| {}).await;
    add_user(&st, "adm", "admin").await;
    let w = wf("adm", "server.started", &[], vec![Action::Notify { title: "x".into(), body: String::new() }]);
    workflows::insert(&st, &w).await.unwrap();
    for _ in 0..55 { run::run_one(st.clone(), w.clone(), ev("server.started", json!({}))).await; }
    assert_eq!(workflows::runs(&st, &w.id).await.unwrap().len(), 50);
}

#[tokio::test]
async fn all_servers_means_servers_the_creator_can_view() {
    let st = testutil::state(|_| {}).await;
    add_user(&st, "u1", "user").await;
    add_server(&st, "mine").await;
    add_server(&st, "secret").await;
    st.db.call(|c| c.execute("INSERT INTO grants(server_id,user_id,bundle) VALUES ('mine','u1','manager')", []).map(|_| ())).await.unwrap();
    let w = wf("u1", "player.joined", &[], vec![Action::Notify { title: "hi".into(), body: String::new() }]);
    workflows::insert(&st, &w).await.unwrap();
    let on = |sid: &str, name: &str| Ev { kind: "player.joined", server: Some((sid.into(), name.into())), data: json!({"player": "Alex"}) };
    run::handle(&st, &on("secret", "srv-secret")).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(workflows::runs(&st, &w.id).await.unwrap().is_empty(), "no run record for an invisible server");
    let after = workflows::get(&st, &w.id).await.unwrap().unwrap();
    assert!(after.enabled && after.disabled_reason.is_none() && after.last_result.is_none(), "not disabled, nothing stored");
    run::run_one(st.clone(), w.clone(), on("secret", "srv-secret")).await;
    assert!(workflows::runs(&st, &w.id).await.unwrap().is_empty(), "run_one also skips it silently");
    run::handle(&st, &on("mine", "srv-mine")).await;
    assert_eq!(wait_runs(&st, &w.id, 1).await.len(), 1, "own servers still match");
    // an explicitly listed server that is no longer permitted does disable
    let w2 = wf("u1", "player.joined", &["secret"], vec![Action::Notify { title: "x".into(), body: String::new() }]);
    workflows::insert(&st, &w2).await.unwrap();
    run::run_one(st.clone(), w2.clone(), on("secret", "srv-secret")).await;
    assert!(!workflows::get(&st, &w2.id).await.unwrap().unwrap().enabled);
}

#[tokio::test]
async fn dry_run_never_names_servers_the_creator_cannot_view() {
    let st = testutil::state(|_| {}).await;
    add_user(&st, "u1", "user").await;
    add_server(&st, "mine").await;
    add_server(&st, "secret").await;
    st.db.call(|c| c.execute("INSERT INTO grants(server_id,user_id,bundle) VALUES ('mine','u1','manager')", []).map(|_| ())).await.unwrap();
    let w = wf("u1", "player.joined", &[], vec![Action::Restart { server: "secret".into() }, Action::Restart { server: "trigger".into() }]);
    let p = run::plan(&st, &w, &run::sample_event("player.joined", Some(("mine".into(), "srv-mine".into())))).await;
    assert!(!p.to_string().contains("srv-secret"), "{p}");
    assert_eq!(p["steps"][0]["allowed"], json!(false));
    assert_eq!(p["steps"][1]["server"], "srv-mine");
}

#[tokio::test]
async fn rights_are_rechecked_before_every_step() {
    let st = testutil::state(|_| {}).await;
    add_user(&st, "u1", "user").await;
    add_server(&st, "s1").await;
    st.db.call(|c| c.execute("INSERT INTO grants(server_id,user_id,bundle) VALUES ('s1','u1','manager')", []).map(|_| ())).await.unwrap();
    let w = wf("u1", "server.crashed", &["s1"], vec![Action::Notify { title: "a".into(), body: String::new() }, Action::Wait { seconds: 1 }, Action::Command { server: "trigger".into(), command: "say x".into() }]);
    workflows::insert(&st, &w).await.unwrap();
    let (st2, w2) = (st.clone(), w.clone());
    let h = tokio::spawn(async move { run::run_one(st2, w2, ev("server.crashed", json!({}))).await });
    tokio::time::sleep(Duration::from_millis(300)).await; // inside the wait
    st.db.call(|c| c.execute("DELETE FROM grants WHERE user_id='u1'", []).map(|_| ())).await.unwrap();
    h.await.unwrap();
    let runs = workflows::runs(&st, &w.id).await.unwrap();
    assert_eq!(runs[0]["ok"], json!(false));
    assert!(runs[0]["log"].to_string().contains("lost access"), "{runs:?}");
    assert!(!workflows::get(&st, &w.id).await.unwrap().unwrap().enabled);
}

#[test]
fn total_waiting_is_capped() {
    let w = |s: u64| Action::Wait { seconds: s };
    assert!(validate("server.started", None, &[w(600), w(600), w(600)], false).is_ok());
    assert_eq!(validate("server.started", None, &[w(600), w(600), w(600), w(1)], false).unwrap_err().0, "actions");
}

#[tokio::test]
async fn the_per_user_cap_holds_under_parallel_creates() {
    let st = testutil::state(|_| {}).await;
    add_user(&st, "u1", "user").await;
    let tasks: Vec<_> = (0..8).map(|_| { let st = st.clone(); tokio::spawn(async move { workflows::insert_capped(&st, &wf("u1", "server.crashed", &[], vec![]), 5).await.unwrap() }) }).collect();
    let mut ok = 0;
    for t in tasks { if t.await.unwrap() { ok += 1; } }
    assert_eq!(ok, 5);
    let n: i64 = st.db.call(|c| c.query_row("SELECT COUNT(*) FROM workflows WHERE created_by='u1'", [], |r| r.get(0))).await.unwrap();
    assert_eq!(n, 5);
}
