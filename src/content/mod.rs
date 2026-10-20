//! Typed content management on top of Modrinth and CurseForge: plugins, mods, proxy plugins and datapacks are filtered
//! by the server's runtime, loader, game version and side, installed through reviewed change plans, with undo.
//! Shared types (`Compat`, `Prov`, `Src`) and helpers live here; handlers are split by concern in the submodules.
use crate::{
    api::server_for, auth::User, providers::curseforge as cf, download::{self, Hash}, error::{ApiError, ApiResult}, files::safe_join, ops::{self, OpCtx},
    server::Srv, state::St, undo,
};
use anyhow::{Context, Result, anyhow, bail};
use axum::{Json, extract::{Path, Query, State}, http::StatusCode};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;


mod apply;
mod check;
mod identify;
mod cfpack;
mod inventory;
mod modpack;
mod modrinth;
mod mrpack;
mod plan;
mod search;
mod toggle;
mod update_all;
mod upgrade;

pub use apply::{check_updates, install, install_many, update};
pub use check::{check_get, server_findings};
pub use identify::identify;
pub use toggle::toggle;
pub use update_all::{update_all, update_all_plan};
pub use inventory::{installed, patch_item, remove, resync};
pub use modpack::{create_params as modpack_create_params, export as modpack_export, import_start as modpack_import, inspect as modpack_inspect, install_then_import};
pub use plan::plan;
pub use search::{project, providers, search};
pub use upgrade::{maybe_backup, upgrade_plan, upgrade_start};
use inventory::*; use modrinth::*; use plan::*; use search::*;

#[derive(Clone)]
pub struct Compat { pub kind: &'static str, pub project_type: &'static str, pub loaders: Vec<&'static str>, pub game_version: Option<String>, pub dir: String, pub ext: &'static str, pub cf_class: Option<u32>, pub cf_loader: u32 }

pub fn compat(st: &St, s: &Srv) -> Option<Compat> {
    let gv = Some(s.game_version.clone());
    let c = match s.runtime.as_str() {
        "paper" => Compat { kind: "plugins", project_type: "plugin", loaders: vec!["paper", "spigot", "bukkit"], game_version: gv, dir: "plugins".into(), ext: "jar", cf_class: None, cf_loader: 0 },
        "purpur" => Compat { kind: "plugins", project_type: "plugin", loaders: vec!["purpur", "paper", "spigot", "bukkit"], game_version: gv, dir: "plugins".into(), ext: "jar", cf_class: None, cf_loader: 0 },
        "folia" => Compat { kind: "plugins", project_type: "plugin", loaders: vec!["folia"], game_version: gv, dir: "plugins".into(), ext: "jar", cf_class: None, cf_loader: 0 },
        "fabric" => Compat { kind: "mods", project_type: "mod", loaders: vec!["fabric"], game_version: gv, dir: "mods".into(), ext: "jar", cf_class: None, cf_loader: 0 },
        "neoforge" => Compat { kind: "mods", project_type: "mod", loaders: vec!["neoforge"], game_version: gv, dir: "mods".into(), ext: "jar", cf_class: None, cf_loader: 0 },
        "forge" => Compat { kind: "mods", project_type: "mod", loaders: vec!["forge"], game_version: gv, dir: "mods".into(), ext: "jar", cf_class: None, cf_loader: 0 },
        "quilt" => Compat { kind: "mods", project_type: "mod", loaders: vec!["quilt", "fabric"], game_version: gv, dir: "mods".into(), ext: "jar", cf_class: None, cf_loader: 0 },
        "velocity" => Compat { kind: "proxy_plugins", project_type: "plugin", loaders: vec!["velocity"], game_version: None, dir: "plugins".into(), ext: "jar", cf_class: None, cf_loader: 0 },
        "vanilla" => {
            let level = std::fs::read_to_string(s.dir(st).join("server.properties")).ok()
                .and_then(|r| r.lines().find_map(|l| l.strip_prefix("level-name=").map(|v| v.trim().to_string()))).filter(|l| !l.is_empty() && !l.contains(['/', '\\']) && !l.starts_with('.')).unwrap_or_else(|| "world".into());
            Compat { kind: "datapacks", project_type: "datapack", loaders: vec!["datapack"], game_version: gv, dir: format!("{level}/datapacks"), ext: "zip", cf_class: None, cf_loader: 0 }
        }
        _ => return None,
    };
    let (cf_class, cf_loader) = match s.runtime.as_str() {
        "paper" | "purpur" | "folia" => (Some(cf::CLASS_BUKKIT), 0),
        "fabric" => (Some(cf::CLASS_MODS), cf::LOADER_FABRIC),
        "forge" => (Some(cf::CLASS_MODS), cf::LOADER_FORGE),
        "neoforge" => (Some(cf::CLASS_MODS), cf::LOADER_NEOFORGE),
        "quilt" => (Some(cf::CLASS_MODS), cf::LOADER_QUILT),
        "vanilla" => (Some(cf::CLASS_DATAPACKS), 0),
        _ => (None, 0),
    };
    Some(Compat { cf_class, cf_loader, ..c })
}

#[derive(Clone, Copy, PartialEq)]
pub enum Prov { Modrinth, Curseforge }
impl Prov {
    pub fn parse(v: Option<&str>) -> ApiResult<Self> {
        match v { None | Some("") | Some("modrinth") => Ok(Self::Modrinth), Some("curseforge") => Ok(Self::Curseforge), _ => Err(ApiError::bad("validation", "provider must be modrinth or curseforge").field("provider", "modrinth | curseforge")) }
    }
    pub fn as_str(self) -> &'static str { if self == Self::Curseforge { "curseforge" } else { "modrinth" } }
}

/// Which catalogue a request targets, plus the caller's token (forwarded to the CurseForge edge function).
#[derive(Clone)]
pub struct Src { pub prov: Prov, pub token: Option<String> }

fn require_cf(st: &St, c: &Compat, token: Option<&str>) -> ApiResult<u32> {
    let class = c.cf_class.ok_or_else(|| ApiError::bad("capability.unsupported", "CurseForge has no content for this runtime"))?;
    if let (false, Some(why)) = cf::availability(st, token) { return Err(ApiError::new(StatusCode::CONFLICT, "provider.unavailable", why)); }
    Ok(class)
}

/// Links come from project authors: only http(s) URLs are passed on, never javascript: or data: ones.
fn http_url(v: &Value) -> Value { match v.as_str() { Some(u) if u.starts_with("https://") || u.starts_with("http://") => v.clone(), _ => Value::Null } }

fn valid_id(s: &str) -> bool { !s.is_empty() && s.len() <= 64 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') }

fn upstream(e: anyhow::Error) -> ApiError {
    match e.downcast_ref::<cf::CfError>() {
        Some(c) if c.rate_limited => ApiError::new(StatusCode::TOO_MANY_REQUESTS, "provider.rate_limited", c.msg.clone()),
        _ => ApiError::new(StatusCode::BAD_GATEWAY, "provider.unavailable", format!("{e:#}")),
    }
}

async fn compat_for(st: &St, user: &User, id: &str, perm: &str) -> ApiResult<(Srv, Compat)> {
    let s = server_for(st, user, id, perm).await?;
    let c = compat(st, &s).ok_or_else(|| ApiError::bad("capability.unsupported", "this runtime has no managed content"))?;
    Ok((s, c))
}

fn lifecycle_running(st: &St, s: &Srv) -> bool { st.live.lock().unwrap().get(&s.id).is_some_and(|l| l.alive) }

#[derive(Deserialize)] pub struct InstallReq { project_id: String, version_id: Option<String>, provider: Option<String> }

fn src_for(st: &St, user: &User, c: &Compat, provider: Option<&str>) -> ApiResult<Src> {
    let prov = Prov::parse(provider)?;
    if prov == Prov::Curseforge { require_cf(st, c, user.token.as_deref())?; }
    Ok(Src { prov, token: user.token.clone() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_http_links_pass_through() {
        assert_eq!(http_url(&json!("https://example.com/x")), json!("https://example.com/x"));
        for bad in ["javascript:alert(1)", "data:text/html,x", "//evil.example", ""] { assert!(http_url(&json!(bad)).is_null(), "{bad}"); }
        assert!(http_url(&Value::Null).is_null());
    }

    #[test]
    fn provider_parsing_defaults_to_modrinth() {
        assert!(Prov::parse(None).unwrap() == Prov::Modrinth && Prov::parse(Some("")).unwrap() == Prov::Modrinth);
        assert!(Prov::parse(Some("curseforge")).unwrap() == Prov::Curseforge);
        assert!(Prov::parse(Some("steam")).is_err());
    }
}
