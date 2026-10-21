//! Running workflows: matching events to workflows, the safety checks (loop guard, rate limit, creator permissions),
//! sequential execution of actions, run records, and the dry run that reports what would happen.
use super::{Action, KEEP_RUNS, MAX_TOTAL_WAIT_S, MAX_WAIT_S, Workflow, eval::{self, LoopGuard, Limiter}};
use crate::{auth::{User, authz}, lifecycle, ops, server, state::St, webhooks::{self, Ev, EVENT_TYPES}};
use serde_json::{Value, json};
use std::{sync::{LazyLock, Mutex}, time::Instant};

#[derive(Default)]
struct Engine { limiter: Limiter, guard: LoopGuard }
static ENGINE: LazyLock<Mutex<Engine>> = LazyLock::new(Default::default);

fn engine<T>(f: impl FnOnce(&mut Engine) -> T) -> T { f(&mut ENGINE.lock().unwrap_or_else(|p| p.into_inner())) }
/// What one run has done so far: total time waited and the servers it acted on (for the loop guard).
#[derive(Default)]
struct RunState { waited: u64, touched: Vec<String> }

fn begin(wf: &Workflow, rs: &mut RunState, server_id: &str) {
    engine(|e| e.guard.begin(&wf.id, server_id, Instant::now()));
    if !rs.touched.iter().any(|s| s == server_id) { rs.touched.push(server_id.to_string()); }
}

/// The account a workflow runs as, as it is now (`None` when deleted).
pub async fn load_user(st: &St, id: &str) -> Option<User> {
    let i = id.to_string();
    let found = st.db.call(move |c| c.query_row("SELECT id,email,role FROM users WHERE id=?", [i], |r| Ok(User { id: r.get(0)?, email: r.get(1)?, role: r.get(2)?, token: None, via_token: None, aal: None })).map(Some)
        .or_else(|e| if matches!(e, rusqlite::Error::QueryReturnedNoRows) { Ok(None) } else { Err(e) })).await.ok().flatten();
    found.or_else(|| (id == "dev" && st.cfg.dev()).then(|| User { id: "dev".into(), email: "dev@localhost".into(), role: "owner".into(), token: None, via_token: None, aal: None }))
}

/// What the actions need: view access on every server involved, plus the permission each action asks for.
/// `trigger_servers` are the servers a `"trigger"` target can mean.
pub fn requirements(actions: &[Action], trigger_servers: &[String]) -> Vec<(String, &'static str)> {
    let mut out: Vec<(String, &'static str)> = vec![];
    let mut add = |s: &str, p: &'static str| { if !out.iter().any(|(x, q)| x == s && *q == p) { out.push((s.to_string(), p)); } };
    for s in trigger_servers { add(s, "server.view"); }
    for a in actions {
        let Some((t, perm)) = a.target() else { continue };
        let servers: Vec<String> = if t == "trigger" { trigger_servers.to_vec() } else { vec![t.to_string()] };
        for s in &servers { add(s, "server.view"); add(s, perm); }
    }
    out
}

/// The first reason `user` may not run these actions, if any.
pub async fn permission_problem(st: &St, user: &User, actions: &[Action], trigger_servers: &[String]) -> Option<String> {
    if !user.is_admin() && actions.iter().any(|a| matches!(a, Action::Webhook { endpoint_id: Some(_), .. })) { return Some("only administrators can use a saved webhook".into()); }
    for (s, p) in requirements(actions, trigger_servers) {
        if let Err(e) = authz(st, user, &s, p).await { return Some(format!("{} ({p} on server {s})", e.message)); }
    }
    None
}

fn event_json(ev: &Ev) -> Value {
    json!({"type": ev.kind, "server": ev.server.as_ref().map(|(i, n)| json!({"id": i, "name": n})), "data": ev.data})
}

/// An example event of a kind, for the dry run.
pub fn sample_event(kind: &str, server: Option<(String, String)>) -> Ev {
    let kind = EVENT_TYPES.iter().map(|(k, _)| *k).find(|k| *k == kind).unwrap_or("server.started");
    let data = match kind {
        "player.joined" | "player.left" => json!({"player": "Steve", "online": 3, "max": 20}),
        "server.crashed" => json!({"detail": "exit code 1"}),
        "server.started" | "server.restarted" => json!({"previous_state": "starting"}),
        "backup.completed" | "backup.failed" | "operation.completed" | "operation.failed" | "content.installed" => json!({"operation": {"id": "sample", "kind": "backup.create", "title": "Backup", "state": "completed", "error": null}}),
        "update.available" => json!({"title": "Updates available", "body": "2 updates"}),
        "health.critical" => json!({"finding": {"id": "sample", "title": "Disk almost full", "detail": "98% used"}}),
        _ => json!({}),
    };
    Ev { kind, server, data }
}

/// Start the runs this event triggers. Called for every derived event.
pub async fn handle(st: &St, ev: &Ev) {
    let wfs = match super::list(st).await { Ok(l) => l, Err(e) => { tracing::warn!("workflows: could not read workflows: {e:#}"); return; } };
    let sid = ev.server.as_ref().map(|s| s.0.as_str());
    for wf in wfs.into_iter().filter(|w| w.enabled && w.trigger == ev.kind && w.covers(sid)) {
        if let Some(s) = sid && engine(|e| e.guard.suppressed(&wf.id, s, Instant::now())) { tracing::debug!("workflows: {} skipped, it just changed {}", wf.name, s); continue; }
        if !creator_can_see(st, &wf, sid).await { continue; }
        if !eval::condition_holds(wf.condition.as_ref(), ev) { continue; }
        if !engine(|e| e.limiter.allow(&wf.id, Instant::now())) {
            let msg = format!("skipped: rate limit ({} runs per {} minutes)", eval::RATE_MAX, eval::RATE_WINDOW.as_secs() / 60);
            tracing::warn!("workflows: {} {msg}", wf.name);
            record(st, &wf, ev, false, &[msg], true).await;
            continue;
        }
        tokio::spawn(run_one(st.clone(), wf, ev.clone()));
    }
}

/// "All servers" means the servers the creator can currently view: events about others are skipped without a trace.
/// (A deleted creator still passes, so the run can disable the workflow with a reason that names no server.)
async fn creator_can_see(st: &St, wf: &Workflow, sid: Option<&str>) -> bool {
    let Some(s) = sid.filter(|_| wf.server_ids.is_empty()) else { return true };
    match load_user(st, &wf.created_by).await { Some(u) => authz(st, &u, s, "server.view").await.is_ok(), None => true }
}

/// Run one workflow for one event and record the result.
pub async fn run_one(st: St, wf: Workflow, ev: Ev) {
    if !creator_can_see(&st, &wf, ev.server.as_ref().map(|s| s.0.as_str())).await { return; }
    let (mut log, mut rs) = (vec![], RunState::default());
    let res = execute(&st, &wf, &ev, &mut log, &mut rs).await;
    let now = Instant::now();
    engine(|e| for s in &rs.touched { e.guard.finish(&wf.id, s, now); });
    if let Err(e) = &res { log.push(format!("failed: {e}")); }
    record(&st, &wf, &ev, res.is_ok(), &log, false).await;
}

async fn disable(st: &St, wf: &Workflow, reason: &str) {
    let (id, r) = (wf.id.clone(), reason.to_string());
    let _ = st.db.call(move |c| c.execute("UPDATE workflows SET enabled=0, disabled_reason=? WHERE id=?", rusqlite::params![r, id])).await;
    st.audit(&format!("workflow:{}", wf.id), "workflow.disabled", &wf.name, reason, None).await;
}

async fn record(st: &St, wf: &Workflow, ev: &Ev, ok: bool, log: &[String], skipped: bool) {
    let (id, wid, at, event, text) = (uuid::Uuid::now_v7().to_string(), wf.id.clone(), crate::db::now(), event_json(ev).to_string(), log.join("\n"));
    let result = match (skipped, ok) { (true, _) => log.last().cloned().unwrap_or_else(|| "skipped".into()), (_, true) => "ok".to_string(), _ => log.last().cloned().unwrap_or_else(|| "failed".into()) };
    let result: String = result.chars().take(300).collect();
    let (a2, w2) = (at.clone(), wid.clone());
    let _ = st.db.call(move |c| {
        c.execute("INSERT INTO workflow_runs(id,workflow_id,at,event,ok,log) VALUES (?,?,?,?,?,?)", rusqlite::params![id, w2, a2, event, ok, text])?;
        c.execute("DELETE FROM workflow_runs WHERE workflow_id=?1 AND id NOT IN (SELECT id FROM workflow_runs WHERE workflow_id=?1 ORDER BY at DESC, id DESC LIMIT ?2)", rusqlite::params![w2, KEEP_RUNS])?;
        c.execute("UPDATE workflows SET last_run_at=?, last_result=? WHERE id=?", rusqlite::params![at, result, wid])
    }).await;
}

async fn execute(st: &St, wf: &Workflow, ev: &Ev, log: &mut Vec<String>, rs: &mut RunState) -> Result<(), String> {
    let trigger_servers: Vec<String> = ev.server.iter().map(|s| s.0.clone()).collect();
    let Some(user) = load_user(st, &wf.created_by).await else {
        disable(st, wf, "The account that created this workflow no longer exists.").await;
        return Err("creator no longer exists; workflow disabled".into());
    };
    if let Some(p) = permission_problem(st, &user, &wf.actions, &trigger_servers).await {
        disable(st, wf, &format!("The creator no longer has the access this workflow needs: {p}")).await;
        return Err(format!("creator lost access ({p}); workflow disabled"));
    }
    for (i, a) in wf.actions.iter().enumerate() {
        let mut user = user.clone();
        if i > 0 {
            if !super::get(st, &wf.id).await.ok().flatten().is_some_and(|w| w.enabled) { return Err("stopped: the workflow was disabled or deleted".into()); }
            // rights can change during a wait or a long step: look at them again before every step
            let Some(now_user) = load_user(st, &wf.created_by).await else {
                disable(st, wf, "The account that created this workflow no longer exists.").await;
                return Err("creator no longer exists; workflow disabled".into());
            };
            if let Some(p) = permission_problem(st, &now_user, std::slice::from_ref(a), &trigger_servers).await {
                disable(st, wf, &format!("The creator no longer has the access this workflow needs: {p}")).await;
                return Err(format!("creator lost access ({p}); workflow disabled"));
            }
            user = now_user;
        }
        let line = exec_one(st, wf, ev, &user, a, rs).await.map_err(|e| format!("step {} ({}): {e}", i + 1, a.label()))?;
        log.push(format!("step {} ok: {line}", i + 1));
    }
    Ok(())
}

async fn target(st: &St, ev: &Ev, t: &str) -> Result<server::Srv, String> {
    let id = if t == "trigger" { ev.server.as_ref().map(|s| s.0.clone()).ok_or("the event has no server")? } else { t.to_string() };
    server::get(st, &id).await.map_err(|_| "server not found".to_string())
}

async fn exec_one(st: &St, wf: &Workflow, ev: &Ev, user: &User, a: &Action, rs: &mut RunState) -> Result<String, String> {
    let actor = format!("workflow:{}", wf.id);
    match a {
        Action::Start { server } | Action::Stop { server } | Action::Restart { server } => {
            let s = target(st, ev, server).await?;
            begin(wf, rs, &s.id);
            let (r, word) = match a { Action::Start { .. } => (lifecycle::start(st, &s, &actor).await, "started"), Action::Stop { .. } => (lifecycle::stop(st, &s, &actor).await, "stopping"), _ => (lifecycle::restart(st, &s, &actor).await, "restarting") };
            r.map(|_| format!("{word} {}", s.name)).map_err(|e| e.message)
        }
        Action::Command { server, command } => {
            let s = target(st, ev, server).await?;
            let cmd = eval::substitute(command, ev);
            begin(wf, rs, &s.id);
            lifecycle::send_command(st, &s, &cmd).await.map_err(|e| e.message)?;
            st.audit_command(&actor, &s.name, &cmd, &s.id).await;
            Ok(format!("sent a command to {}", s.name))
        }
        Action::Backup { server } => {
            let s = target(st, ev, server).await?;
            begin(wf, rs, &s.id);
            let (s2, note) = (s.clone(), format!("Workflow: {}", wf.name));
            let op = ops::spawn_op(st, "backup.create", format!("Workflow backup of {}", s.name), Some(&s.id), true, move |ctx| async move { let st = ctx.st.clone(); crate::backups::do_backup(&ctx, &st, &s2, "scheduled", &note, None).await.map(|_| ()) });
            let end = Instant::now() + std::time::Duration::from_secs(4 * 3600);
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                let Some(o) = ops::get(st, &op.id) else { break };
                match o.state.as_str() { "completed" => break, "failed" | "cancelled" => { return Err(o.error.unwrap_or_else(|| format!("backup {}", o.state))); } _ => {} }
                if Instant::now() > end { return Err("the backup is taking too long".into()); }
            }
            Ok(format!("backed up {}", s.name))
        }
        Action::Wait { seconds } => {
            let secs = (*seconds).min(MAX_WAIT_S);
            if rs.waited + secs > MAX_TOTAL_WAIT_S { return Err(format!("this run would wait more than {} minutes in total", MAX_TOTAL_WAIT_S / 60)); }
            rs.waited += secs;
            tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
            Ok(format!("waited {seconds}s"))
        }
        Action::Notify { title, body } => {
            // Anyone with the right permissions can write a workflow, so say whose it is: no spoofed system notices.
            let who = if user.email.is_empty() { user.id.clone() } else { user.email.clone() };
            let title = format!("Workflow \u{201c}{}\u{201d} ({who}): {}", wf.name, eval::substitute(title, ev));
            let n = json!({"id": uuid::Uuid::now_v7().to_string(), "time": crate::db::now(), "kind": "workflow", "severity": "info",
                "title": title.chars().take(120).collect::<String>(), "body": eval::substitute(body, ev).chars().take(1000).collect::<String>(),
                "server_id": ev.server.as_ref().map(|s| s.0.clone()), "read": false});
            let n2 = n.clone();
            st.db.call(move |c| c.execute("INSERT INTO notifications(id,time,kind,severity,title,body,server_id) VALUES (?,?,?,?,?,?,?)",
                rusqlite::params![n2["id"].as_str(), n2["time"].as_str(), "workflow", "info", n2["title"].as_str(), n2["body"].as_str(), n2["server_id"].as_str()]).map(|_| ())).await.map_err(|e| e.to_string())?;
            st.emit("notification", n);
            Ok("notification sent".into())
        }
        Action::Webhook { url, endpoint_id } => {
            let payload = json!({"workflow": {"id": wf.id, "name": wf.name}, "event": event_json(ev), "time": crate::db::now()});
            match (url, endpoint_id) {
                (Some(u), _) => { post_json(st, u, &payload).await?; Ok("webhook called".into()) }
                (None, Some(e)) => {
                    if !user.is_admin() { return Err("only administrators can use a saved webhook".into()); }
                    let ep = webhooks::get(st, e).await.map_err(|e| e.to_string())?.ok_or("that webhook no longer exists")?;
                    let (event_id, time) = (uuid::Uuid::now_v7().to_string(), crate::db::now());
                    let body = webhooks::body(&event_id, ev.kind, &time, ev.server.as_ref(), &json!({"workflow": {"id": wf.id, "name": wf.name}, "event": ev.data}));
                    let (kind, eid) = (ev.kind.to_string(), ep.id.clone());
                    st.db.call(move |c| webhooks::queue::enqueue(c, &eid, &event_id, &kind, &body, false, &time).map(|_| ())).await.map_err(|e| e.to_string())?;
                    webhooks::queue::wake();
                    Ok(format!("queued a delivery to {}", ep.name))
                }
                _ => Err("no address".into()),
            }
        }
    }
}

/// A plain JSON POST: https only, no redirects, 10 s, the destination resolved once and checked like webhook deliveries.
async fn post_json(st: &St, url: &str, payload: &Value) -> Result<(), String> {
    super::validate_url(url, st.cfg.allow_private_webhooks)?;
    let u = reqwest::Url::parse(url).map_err(|_| "the address is not valid".to_string())?;
    let host = u.host_str().ok_or("the address has no host")?.to_string();
    let addr = crate::notify::webhook::resolve_pinned(&host, u.port_or_known_default().unwrap_or(443), st.cfg.allow_private_webhooks, "MOSS_ALLOW_PRIVATE_WEBHOOKS").await.map_err(|e| e.to_string())?;
    let client = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).timeout(std::time::Duration::from_secs(10)).resolve(&host, addr)
        .user_agent(concat!("Moss/", env!("CARGO_PKG_VERSION"))).build().map_err(|e| e.to_string())?;
    let r = client.post(u).json(payload).send().await.map_err(|_| "the request failed".to_string())?;
    if r.status().is_success() { Ok(()) } else { Err(format!("the server answered {}", r.status().as_u16())) }
}

/// The dry run: what `wf` would do for `ev` right now, with its creator's permissions. Executes nothing.
pub async fn plan(st: &St, wf: &Workflow, ev: &Ev) -> Value {
    let sid = ev.server.as_ref().map(|s| s.0.as_str());
    let user = load_user(st, &wf.created_by).await;
    let viewable = |id: String| { let user = user.clone(); async move { match &user { Some(u) => authz(st, u, &id, "server.view").await.is_ok(), None => false } } };
    let trigger_ok = wf.trigger == ev.kind && wf.covers(sid);
    let cond_ok = eval::condition_holds(wf.condition.as_ref(), ev);
    let mut steps = vec![];
    for (i, a) in wf.actions.iter().enumerate() {
        let (server_name, problem) = match a.target() {
            Some((t, perm)) => {
                let id = if t == "trigger" { sid.map(String::from) } else { Some(t.to_string()) };
                match id {
                    None => (None, Some("the event has no server".to_string())),
                    Some(id) => {
                        let exists = server::get(st, &id).await.ok();
                        let name = match &exists { Some(s) if viewable(id.clone()).await => Some(s.name.clone()), _ => None };
                        let p = match (&user, &exists) {
                            (_, None) => Some("server not found".to_string()),
                            (None, _) => Some("the workflow's creator no longer exists".to_string()),
                            (Some(u), _) => authz(st, u, &id, perm).await.err().map(|e| e.message),
                        };
                        (name, p)
                    }
                }
            }
            None => (None, match (a, &user) { (Action::Webhook { endpoint_id: Some(_), .. }, Some(u)) if !u.is_admin() => Some("only administrators can use a saved webhook".to_string()), (_, None) => Some("the workflow's creator no longer exists".to_string()), _ => None }),
        };
        let description = match a {
            Action::Start { .. } => format!("Start {}", server_name.clone().unwrap_or_default()),
            Action::Stop { .. } => format!("Stop {}", server_name.clone().unwrap_or_default()),
            Action::Restart { .. } => format!("Restart {}", server_name.clone().unwrap_or_default()),
            Action::Backup { .. } => format!("Take a backup of {}", server_name.clone().unwrap_or_default()),
            Action::Command { command, .. } => format!("Send to {}: {}", server_name.clone().unwrap_or_default(), eval::substitute(command, ev)),
            Action::Wait { seconds } => format!("Wait {seconds} seconds"),
            Action::Notify { title, .. } => format!("Notify: {}", eval::substitute(title, ev)),
            Action::Webhook { url, endpoint_id } => if url.is_some() { "Call a webhook address".to_string() } else { format!("Deliver to saved webhook {}", endpoint_id.clone().unwrap_or_default()) },
        };
        steps.push(json!({"index": i + 1, "action": a.label(), "description": description, "server": server_name, "allowed": problem.is_none(), "problem": problem}));
    }
    let would_run = trigger_ok && cond_ok && steps.iter().all(|s| s["allowed"] == true);
    json!({"event": event_json(ev), "trigger_matches": trigger_ok, "condition_holds": cond_ok, "would_run": would_run, "steps": steps, "executed": false})
}

#[cfg(test)]
pub fn mark_for_test(workflow_id: &str, server_id: &str) { engine(|e| { let now = Instant::now(); e.guard.begin(workflow_id, server_id, now); e.guard.finish(workflow_id, server_id, now); }); }
