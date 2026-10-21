//! The pure parts of workflows: condition evaluation, placeholder substitution, the per-workflow rate limit and the
//! loop guard. Nothing here touches the database, the network or the clock (callers pass `Instant`s).
use crate::webhooks::Ev;
use serde_json::Value;
use std::{collections::{HashMap, VecDeque}, time::{Duration, Instant}};

pub const OPS: &[&str] = &["==", "!=", ">", ">=", "<", "<=", "contains", "not_contains"];
/// Fields a condition may read, besides `data.<path>` for anything else in the event payload.
pub const FIELDS: &[&str] = &["players.online", "players.max", "player", "server", "detail"];

/// Max runs per workflow per window; more are skipped and logged.
pub const RATE_MAX: usize = 10;
pub const RATE_WINDOW: Duration = Duration::from_secs(600);
/// Events on a server within this long after a workflow acted on it are not allowed to trigger workflows.
pub const LOOP_WINDOW: Duration = Duration::from_secs(120);

/// Check a condition's shape; the message is shown to the user.
pub fn validate_condition(c: &Value) -> Result<(), String> {
    let field = c["field"].as_str().unwrap_or_default();
    if !FIELDS.contains(&field) && !field.strip_prefix("data.").is_some_and(|p| !p.is_empty() && p.len() <= 100) {
        return Err(format!("Unknown field. Use one of: {}, or data.<name>.", FIELDS.join(", ")));
    }
    let op = c["op"].as_str().unwrap_or_default();
    if !OPS.contains(&op) { return Err(format!("Unknown comparison. Use one of: {}.", OPS.join(" "))); }
    let v = &c["value"];
    if !v.is_number() && !v.as_str().is_some_and(|s| s.chars().count() <= 200) { return Err("Give the condition a number or text to compare with.".into()); }
    if matches!(op, ">" | ">=" | "<" | "<=") && num(v).is_none() { return Err("This comparison needs a number.".into()); }
    Ok(())
}

fn leaves(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::String(s) => out.push(s.clone()),
        Value::Array(a) => a.iter().for_each(|x| leaves(x, out)),
        Value::Object(m) => m.values().for_each(|x| leaves(x, out)),
        Value::Null => {}
        o => out.push(o.to_string()),
    }
}

fn lookup(field: &str, ev: &Ev) -> Option<Value> {
    let d = &ev.data;
    let v = match field {
        "players.online" => d.get("online"),
        "players.max" => d.get("max"),
        "player" => d.get("player"),
        "server" => return ev.server.as_ref().map(|(_, n)| Value::String(n.clone())),
        "detail" => { let mut l = vec![]; leaves(d, &mut l); return if l.is_empty() { None } else { Some(Value::String(l.join("\n"))) }; }
        f => f.strip_prefix("data.").and_then(|p| p.split('.').try_fold(d, |v, k| v.get(k))),
    };
    v.filter(|v| !v.is_null()).cloned()
}

fn num(v: &Value) -> Option<f64> { v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok())) }
fn text(v: &Value) -> String { v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()).to_lowercase() }

/// Does the event satisfy the condition? No condition always holds. A field the event does not carry makes every
/// comparison false except `!=` and `not_contains`.
pub fn condition_holds(cond: Option<&Value>, ev: &Ev) -> bool {
    let Some(c) = cond.filter(|c| !c.is_null()) else { return true };
    let (op, want) = (c["op"].as_str().unwrap_or_default(), &c["value"]);
    let Some(have) = lookup(c["field"].as_str().unwrap_or_default(), ev) else { return matches!(op, "!=" | "not_contains") };
    match op {
        "contains" => text(&have).contains(&text(want)),
        "not_contains" => !text(&have).contains(&text(want)),
        "==" | "!=" => {
            let eq = match (num(&have), num(want)) { (Some(a), Some(b)) => a == b, _ => text(&have) == text(want) };
            eq == (op == "==")
        }
        _ => match (num(&have), num(want)) {
            (Some(a), Some(b)) => match op { ">" => a > b, ">=" => a >= b, "<" => a < b, "<=" => a <= b, _ => false },
            _ => false,
        },
    }
}

fn clean(s: &str) -> String { s.chars().filter(|c| !c.is_control()).collect() }

/// Replace `{player}`, `{server}`, `{event}`, `{online}` and `{max}` from the event. Values lose control characters so
/// a substituted value can never turn a one-line console command into several. Other `{...}` text is left alone.
pub fn substitute(tpl: &str, ev: &Ev) -> String {
    let get = |k: &str| ev.data.get(k).filter(|v| !v.is_null()).map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())).unwrap_or_default();
    let mut out = String::new();
    let mut rest = tpl;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        let after = &rest[i..];
        let Some(j) = after.find('}') else { out.push_str(after); return out };
        let val = match &after[1..j] {
            "player" => Some(get("player")),
            "server" => Some(ev.server.as_ref().map(|(_, n)| n.clone()).unwrap_or_default()),
            "event" => Some(ev.kind.to_string()),
            "online" => Some(get("online")),
            "max" => Some(get("max")),
            _ => None,
        };
        match val { Some(v) => out.push_str(&clean(&v)), None => out.push_str(&after[..=j]) }
        rest = &after[j + 1..];
    }
    out.push_str(rest);
    out
}

/// At most `RATE_MAX` runs per workflow per `RATE_WINDOW`.
#[derive(Default)]
pub struct Limiter { runs: HashMap<String, VecDeque<Instant>> }

impl Limiter {
    /// Count a run if allowed; false means skip it.
    pub fn allow(&mut self, id: &str, now: Instant) -> bool {
        let q = self.runs.entry(id.to_string()).or_default();
        while q.front().is_some_and(|t| now.duration_since(*t) >= RATE_WINDOW) { q.pop_front(); }
        if q.len() >= RATE_MAX { return false; }
        q.push_back(now);
        true
    }
}

/// A run that never reports back (a crashed task) stops suppressing after this long.
const STUCK: Duration = Duration::from_secs(6 * 3600);

/// Remembers, per (workflow, server), when that workflow is acting or last acted on the server, so the events its own
/// actions cause cannot trigger it again. Other workflows still receive those events.
#[derive(Default)]
pub struct LoopGuard { acted: HashMap<(String, String), (Instant, Option<Instant>)> }

impl LoopGuard {
    /// The workflow starts acting on the server: suppressed until it finishes plus `LOOP_WINDOW`.
    pub fn begin(&mut self, workflow_id: &str, server_id: &str, now: Instant) {
        if self.acted.len() > 1000 { self.acted.retain(|_, (b, e)| match e { Some(e) => now.duration_since(*e) < LOOP_WINDOW, None => now.duration_since(*b) < STUCK }); }
        self.acted.insert((workflow_id.to_string(), server_id.to_string()), (now, None));
    }
    /// The workflow's run is over; the window counts from now.
    pub fn finish(&mut self, workflow_id: &str, server_id: &str, now: Instant) {
        if let Some(e) = self.acted.get_mut(&(workflow_id.to_string(), server_id.to_string())) { e.1 = Some(now); }
    }
    pub fn suppressed(&self, workflow_id: &str, server_id: &str, now: Instant) -> bool {
        self.acted.get(&(workflow_id.to_string(), server_id.to_string())).is_some_and(|(b, e)| match e { Some(e) => now.duration_since(*e) < LOOP_WINDOW, None => now.duration_since(*b) < STUCK })
    }
}
