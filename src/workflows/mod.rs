//! Event workflows: "when <event> happens (on these servers, if <condition>), do <actions>". Triggers are exactly the
//! webhook event kinds, received from the same derived stream (`webhooks::subscribe`). A workflow runs with the
//! permissions of its creator, re-checked on every run (a creator who lost access gets the workflow disabled with a
//! recorded reason). Actions run one after another in their own task; the first failure stops the rest.
//! Safety: events on a server shortly after a workflow acted on it never trigger workflows (loop protection), and each
//! workflow runs at most `eval::RATE_MAX` times per `eval::RATE_WINDOW`.
use crate::state::St;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub mod eval;
pub mod run;
#[cfg(test)] mod tests;

pub const MAX_ACTIONS: usize = 20;
pub const MAX_WAIT_S: u64 = 600;
pub const KEEP_RUNS: i64 = 50;

fn trigger_target() -> String { "trigger".into() }

/// One step. `server` is `"trigger"` (the server the event is about) or a server id.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Start { #[serde(default = "trigger_target")] server: String },
    Stop { #[serde(default = "trigger_target")] server: String },
    Restart { #[serde(default = "trigger_target")] server: String },
    Command { #[serde(default = "trigger_target")] server: String, command: String },
    Backup { #[serde(default = "trigger_target")] server: String },
    Wait { seconds: u64 },
    Notify { title: String, #[serde(default)] body: String },
    /// A plain JSON POST to `url`, or a signed delivery to the existing event webhook `endpoint_id` (administrators only).
    Webhook { #[serde(default)] url: Option<String>, #[serde(default)] endpoint_id: Option<String> },
}

impl Action {
    /// The server this step acts on and the permission it needs there.
    pub fn target(&self) -> Option<(&str, &'static str)> {
        match self {
            Action::Start { server } | Action::Stop { server } | Action::Restart { server } => Some((server, "server.power")),
            Action::Command { server, .. } => Some((server, "server.console.write")),
            Action::Backup { server } => Some((server, "backup.manage")),
            _ => None,
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Action::Start { .. } => "start", Action::Stop { .. } => "stop", Action::Restart { .. } => "restart", Action::Command { .. } => "command",
            Action::Backup { .. } => "backup", Action::Wait { .. } => "wait", Action::Notify { .. } => "notify", Action::Webhook { .. } => "webhook",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Workflow {
    pub id: String, pub name: String, pub enabled: bool, pub trigger: String, pub server_ids: Vec<String>, pub condition: Option<Value>,
    pub actions: Vec<Action>, pub created_by: String, pub created_at: String, pub last_run_at: Option<String>, pub last_result: Option<String>,
    pub disabled_reason: Option<String>,
}

pub const COLS: &str = "id,name,enabled,\"trigger\",server_ids,condition,actions,created_by,created_at,last_run_at,last_result,disabled_reason";

impl Workflow {
    pub fn from_row(r: &rusqlite::Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: r.get(0)?, name: r.get(1)?, enabled: r.get(2)?, trigger: r.get(3)?,
            server_ids: serde_json::from_str(&r.get::<_, String>(4)?).unwrap_or_default(),
            condition: r.get::<_, Option<String>>(5)?.and_then(|s| serde_json::from_str(&s).ok()),
            actions: serde_json::from_str(&r.get::<_, String>(6)?).unwrap_or_default(),
            created_by: r.get(7)?, created_at: r.get(8)?, last_run_at: r.get(9)?, last_result: r.get(10)?, disabled_reason: r.get(11)?,
        })
    }
    pub fn to_json(&self) -> Value {
        json!({"id": self.id, "name": self.name, "enabled": self.enabled, "trigger": self.trigger, "server_ids": self.server_ids, "condition": self.condition,
            "actions": self.actions, "created_by": self.created_by, "created_at": self.created_at, "last_run_at": self.last_run_at,
            "last_result": self.last_result, "disabled_reason": self.disabled_reason})
    }
    /// Is the event about a server this workflow covers? (No server list = any server; the creator's permissions are
    /// checked when it runs.) Events without a server only match workflows without a server list.
    pub fn covers(&self, server_id: Option<&str>) -> bool {
        self.server_ids.is_empty() || server_id.is_some_and(|s| self.server_ids.iter().any(|x| x == s))
    }
}

fn opt<T>(r: rusqlite::Result<T>) -> rusqlite::Result<Option<T>> {
    match r { Ok(v) => Ok(Some(v)), Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None), Err(e) => Err(e) }
}

pub async fn list(st: &St) -> anyhow::Result<Vec<Workflow>> {
    st.db.call(|c| c.prepare(&format!("SELECT {COLS} FROM workflows ORDER BY created_at, id"))?.query_map([], Workflow::from_row)?.collect()).await
}
pub async fn get(st: &St, id: &str) -> anyhow::Result<Option<Workflow>> {
    let id = id.to_string();
    st.db.call(move |c| opt(c.query_row(&format!("SELECT {COLS} FROM workflows WHERE id=?"), [id], Workflow::from_row))).await
}
pub async fn insert(st: &St, w: &Workflow) -> anyhow::Result<()> {
    let w = w.clone();
    st.db.call(move |c| c.execute("INSERT INTO workflows(id,name,enabled,\"trigger\",server_ids,condition,actions,created_by,created_at,last_run_at,last_result,disabled_reason) VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
        rusqlite::params![w.id, w.name, w.enabled, w.trigger, json!(w.server_ids).to_string(), w.condition.as_ref().map(|c| c.to_string()), json!(w.actions).to_string(), w.created_by, w.created_at, w.last_run_at, w.last_result, w.disabled_reason]).map(|_| ())).await
}
/// Rewrites the editable fields (not the run bookkeeping).
pub async fn update(st: &St, w: &Workflow) -> anyhow::Result<()> {
    let w = w.clone();
    st.db.call(move |c| c.execute("UPDATE workflows SET name=?,enabled=?,\"trigger\"=?,server_ids=?,condition=?,actions=?,disabled_reason=? WHERE id=?",
        rusqlite::params![w.name, w.enabled, w.trigger, json!(w.server_ids).to_string(), w.condition.as_ref().map(|c| c.to_string()), json!(w.actions).to_string(), w.disabled_reason, w.id]).map(|_| ())).await
}
pub async fn delete(st: &St, id: &str) -> anyhow::Result<()> {
    let id = id.to_string();
    st.db.call(move |c| c.execute("DELETE FROM workflows WHERE id=?", [id]).map(|_| ())).await
}
pub async fn runs(st: &St, id: &str) -> anyhow::Result<Vec<Value>> {
    let id = id.to_string();
    st.db.call(move |c| c.prepare("SELECT id,at,event,ok,log FROM workflow_runs WHERE workflow_id=? ORDER BY at DESC, id DESC LIMIT 50")?.query_map([id],
        |r| Ok(json!({"id": r.get::<_, String>(0)?, "at": r.get::<_, String>(1)?, "event": serde_json::from_str::<Value>(&r.get::<_, String>(2)?).unwrap_or(Value::Null),
            "ok": r.get::<_, bool>(3)?, "log": r.get::<_, String>(4)?.lines().collect::<Vec<_>>()})))?.collect()).await
}

/// Is any enabled workflow triggered by this event kind? (Lets the health poll run only when someone listens.)
pub async fn wants(st: &St, kind: &str) -> bool {
    let k = kind.to_string();
    st.db.call(move |c| c.query_row("SELECT EXISTS(SELECT 1 FROM workflows WHERE enabled=1 AND \"trigger\"=?)", [k], |r| r.get(0))).await.unwrap_or(false)
}

/// A URL the "call webhook" action may POST to: https, a host, no credentials, not a private literal address.
pub fn validate_url(s: &str, allow_private: bool) -> Result<(), String> {
    let s = s.trim();
    let u = reqwest::Url::parse(s).map_err(|_| "Enter a full address such as https://example.com/hook.".to_string())?;
    if s.len() > 500 { return Err("At most 500 characters.".into()); }
    if u.scheme() != "https" { return Err("The address must start with https://.".into()); }
    let Some(host) = u.host_str().filter(|h| !h.is_empty()) else { return Err("The address has no host name.".into()) };
    if !u.username().is_empty() || u.password().is_some() { return Err("Do not put a user name or password in the address.".into()); }
    if let Ok(ip) = host.trim_matches(['[', ']']).parse::<std::net::IpAddr>() && !allow_private && !crate::notify::webhook::is_public_ip(ip) {
        return Err("That is a private or local address. Moss only calls public addresses unless the administrator allows private webhooks.".into());
    }
    Ok(())
}

/// Shape-check a definition. Errors are `(field, message)`. `servers` are the ids that must exist for explicit targets.
pub fn validate(trigger: &str, condition: Option<&Value>, actions: &[Action], allow_private: bool) -> Result<(), (String, String)> {
    let bad = |f: &str, m: &str| Err((f.to_string(), m.to_string()));
    if !crate::webhooks::is_event(trigger) { return bad("trigger", "Choose one of the known events."); }
    if let Some(c) = condition.filter(|c| !c.is_null()) && let Err(m) = eval::validate_condition(c) { return bad("condition", &m); }
    if actions.is_empty() { return bad("actions", "Add at least one action."); }
    if actions.len() > MAX_ACTIONS { return bad("actions", &format!("At most {MAX_ACTIONS} actions.")); }
    for (i, a) in actions.iter().enumerate() {
        let f = format!("actions[{i}]");
        if let Some((s, _)) = a.target() {
            if s.is_empty() || s.len() > 100 { return bad(&f, "Choose a server."); }
            if s == "trigger" && trigger == "update.available" { return bad(&f, "This event is not about one server; choose a specific server."); }
        }
        match a {
            Action::Command { command, .. } => {
                let c = command.trim();
                if c.is_empty() || c.len() > 200 || c.contains(['\n', '\r', '\0']) { return bad(&f, "A command must be one line of up to 200 characters."); }
            }
            Action::Wait { seconds } if !(1..=MAX_WAIT_S).contains(seconds) => return bad(&f, &format!("Wait between 1 and {MAX_WAIT_S} seconds.")),
            Action::Notify { title, body } => {
                if title.trim().is_empty() || title.chars().count() > 100 { return bad(&f, "Give the notification a title of up to 100 characters."); }
                if body.chars().count() > 500 { return bad(&f, "The notification text is at most 500 characters."); }
            }
            Action::Webhook { url, endpoint_id } => match (url, endpoint_id) {
                (Some(u), None) => if let Err(m) = validate_url(u, allow_private) { return Err((f, m)); },
                (None, Some(e)) if !e.is_empty() => {}
                _ => return bad(&f, "Give either an https address or an existing webhook."),
            },
            _ => {}
        }
    }
    Ok(())
}

pub fn spawn_background(st: &St) {
    let st = st.clone();
    tokio::spawn(async move {
        let mut rx = crate::webhooks::subscribe();
        loop {
            match rx.recv().await {
                Ok(ev) => run::handle(&st, &ev).await,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => tracing::warn!("workflows: missed {n} events"),
                Err(_) => break,
            }
        }
    });
}
