//! Compatibility checking of the installed content. The rules (`check/rules.rs`) are pure functions over parsed jar
//! metadata plus the server's runtime and game version, so they are unit-testable without disk or network.
//! This file holds the shared types, the loader/built-in tables, the endpoint, and plan-conflict simulation.
use super::*;
use crate::jars::{Info, Meta};
use serde::Serialize;
use std::sync::Arc;

mod rules;
#[cfg(test)]
mod tests;

#[derive(Clone, Debug, Serialize)]
pub struct Action { pub label: String, pub kind: &'static str, #[serde(skip_serializing_if = "Option::is_none")] pub item: Option<String>, #[serde(skip_serializing_if = "Option::is_none")] pub project_id: Option<String> }

/// The contract's `Finding`. `related` lists further items a finding applies to (used to attach issues to every file involved).
#[derive(Clone, Debug, Serialize)]
pub struct Finding {
    pub id: String, pub severity: &'static str, pub item: Option<String>, pub title: String, pub detail: String, pub evidence: Vec<String>, pub action: Option<Action>,
    #[serde(skip)] pub related: Vec<String>,
}

impl Finding {
    pub fn new(id: String, severity: &'static str, item: Option<&str>, title: String, detail: String, evidence: Vec<String>, action: Option<Action>) -> Self {
        Finding { id, severity, item: item.map(String::from), title, detail, evidence, action, related: vec![] }
    }
    pub fn is_problem(&self) -> bool { self.severity != "info" }
}

pub fn action(label: impl Into<String>, kind: &'static str, item: Option<&str>) -> Option<Action> { Some(Action { label: label.into(), kind, item: item.map(String::from), project_id: None }) }

/// What the rules need to know about the server.
#[derive(Clone, Debug)]
pub struct Ctx { pub runtime: String, pub game_version: String, pub loader_version: Option<String>, pub java_major: i64, pub is_mods: bool }

impl Ctx {
    pub fn of(s: &Srv, c: &Compat) -> Self {
        Ctx { runtime: s.runtime.clone(), game_version: s.game_version.clone(), loader_version: s.loader_version.clone(), java_major: s.java_major, is_mods: c.kind == "mods" }
    }
    /// Mods that fail to resolve stop the server; a plugin or datapack just does not load.
    pub fn severity(&self) -> &'static str { if self.is_mods { "critical" } else { "warn" } }
    /// Loaders whose descriptors this runtime reads.
    pub fn accepted(&self) -> Vec<&'static str> {
        match self.runtime.as_str() {
            "paper" | "purpur" | "folia" => vec!["paper", "spigot", "bukkit"],
            "fabric" => vec!["fabric"],
            "quilt" => vec!["quilt", "fabric"],
            "forge" => vec!["forge"],
            "neoforge" => if crate::jars::range::cmp_str(&self.game_version, "1.20.2") == std::cmp::Ordering::Less { vec!["neoforge", "forge"] } else { vec!["neoforge"] },
            "velocity" => vec!["velocity"],
            "bungee" => vec!["bungee"],
            "vanilla" => vec!["datapack"],
            _ => vec![],
        }
    }
    /// Ids the runtime itself provides; the inner option is the version when known.
    pub fn builtin(&self, id: &str) -> Option<Option<String>> {
        let (mc, loader) = (Some(self.game_version.clone()), self.loader_version.clone());
        match (id, self.runtime.as_str()) {
            ("minecraft", _) => Some(mc),
            ("java", _) => Some(Some(self.java_major.to_string())),
            ("fabricloader" | "fabric-loader", "fabric") => Some(loader),
            ("fabricloader" | "fabric-loader", "quilt") | ("quilt_loader" | "quilt-loader", "quilt") => Some(None),
            ("forge" | "javafml" | "lowcodefml", "forge") | ("neoforge" | "javafml" | "lowcodefml", "neoforge") => Some(loader),
            ("forge", "neoforge") => Some(None),
            ("paper" | "spigot" | "bukkit" | "craftbukkit", "paper" | "purpur" | "folia") | ("purpur", "purpur") | ("folia", "folia") | ("velocity", "velocity") => Some(None),
            ("spigot" | "bukkit" | "craftbukkit", "bungee") | ("bungeecord", "bungee") => Some(None),
            _ => None,
        }
    }
}

#[derive(Clone)]
pub struct Entry { pub item: String, pub title: String, pub filename: String, pub enabled: bool, pub info: Arc<Info> }

impl Entry {
    pub fn meta<'a>(&'a self, ctx: &Ctx) -> Option<&'a Meta> { self.info.pick(&ctx.accepted()) }
}

/// Runs every rule; findings come back worst first.
pub fn run(ctx: &Ctx, entries: &[Entry]) -> Vec<Finding> {
    let mut out = rules::all(ctx, entries);
    out.sort_by_key(|f| match f.severity { "critical" => 0, "warn" => 1, _ => 2 });
    out
}

/// Findings for a server's current content (reads jars through the metadata cache).
pub async fn server_findings(st: &St, s: &Srv, c: &Compat) -> Result<Vec<Finding>> {
    let entries = inventory::entries(&inventory::list_with_info(st, s, c).await?);
    Ok(run(&Ctx::of(s, c), &entries))
}

pub async fn check_get(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let (s, c) = compat_for(&st, &user, &id, "server.view").await?;
    let findings = server_findings(&st, &s, &c).await?;
    Ok(Json(json!({"findings": findings, "checked_at": crate::db::now()})))
}

/// One step of a plan, as far as the checker is concerned.
pub struct Change { pub replaces: Option<String>, pub new_version: String }

/// Problems a plan would introduce: findings that exist after replacing versions but not before.
pub async fn plan_conflicts(st: &St, s: &Srv, c: &Compat, changes: &[Change]) -> Result<Vec<Finding>> {
    let before = inventory::entries(&inventory::list_with_info(st, s, c).await?);
    Ok(conflicts(&Ctx::of(s, c), &before, changes))
}

pub fn conflicts(ctx: &Ctx, before: &[Entry], changes: &[Change]) -> Vec<Finding> {
    let known: std::collections::HashSet<String> = run(ctx, before).into_iter().map(|f| f.id).collect();
    let after: Vec<Entry> = before.iter().map(|e| match changes.iter().find(|ch| ch.replaces.as_deref() == Some(&e.item)) {
        Some(ch) => {
            let mut info = (*e.info).clone();
            for m in info.metas.iter_mut() { m.version = ch.new_version.clone(); }
            Entry { info: Arc::new(info), ..e.clone() }
        }
        None => e.clone(),
    }).collect();
    run(ctx, &after).into_iter().filter(|f| f.is_problem() && !known.contains(&f.id)).collect()
}
