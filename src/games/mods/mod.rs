//! Mod management for games other than Minecraft: Valheim (BepInEx mods from Thunderstore), Factorio (the official
//! mod portal) and Steam Workshop content for Garry's Mod and CS2. State lives in the server directory
//! (`moss-mods.json`, `mods/mod-list.json`, `moss-game.cfg`), never in the database, and the Minecraft content
//! pipeline (`crate::content`) is not involved.
use crate::{error::{ApiError, ApiResult}, games::SETTINGS_FILE};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub mod factorio;
pub mod net;
pub mod service;
pub mod thunderstore;
pub mod workshop;
#[cfg(test)]
mod tests;

pub const MANIFEST_FILE: &str = "moss-mods.json";
/// Synthetic setting `games::command` sets when BepInEx is present in the server directory (never read from the file).
pub const BEPINEX_KEY: &str = "_bepinex";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind { Thunderstore, Factorio, Workshop }

impl Kind {
    pub fn of(runtime: &str) -> Option<Self> {
        match runtime { "valheim" => Some(Self::Thunderstore), "factorio" => Some(Self::Factorio), "gmod" | "cs2" => Some(Self::Workshop), _ => None }
    }
    pub fn as_str(self) -> &'static str { match self { Self::Thunderstore => "thunderstore", Self::Factorio => "factorio", Self::Workshop => "workshop" } }
}

/// Base URLs of the mod catalogues. Tests point these at loopback stubs (`allow_local`).
#[derive(Clone)]
pub struct Providers { pub thunderstore: String, pub factorio: String, pub allow_local: bool }

impl Default for Providers {
    fn default() -> Self { Self { thunderstore: "https://thunderstore.io".into(), factorio: "https://mods.factorio.com".into(), allow_local: false } }
}

/// One installed mod. `files` are server-relative paths Moss created (removed again on uninstall).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Entry {
    pub id: String,
    pub title: String,
    pub version: String,
    pub enabled: bool,
    /// Installed only because another mod needs it.
    #[serde(default)]
    pub auto: bool,
    /// Ids of the mods this one needs.
    #[serde(default)]
    pub requires: Vec<String>,
    /// Ids (Factorio names) this one declares itself incompatible with.
    #[serde(default)]
    pub conflicts: Vec<String>,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub installed_at: String,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Default)]
pub struct Manifest {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub mods: Vec<Entry>,
}

impl Manifest {
    pub fn load(dir: &Path) -> ApiResult<Self> {
        match std::fs::read_to_string(dir.join(MANIFEST_FILE)) {
            Ok(t) => serde_json::from_str(&t).map_err(|_| ApiError::conflict("game_mods.manifest", format!("{MANIFEST_FILE} is not valid JSON; fix or delete it from the Files tab"))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }
    pub fn save(&mut self, dir: &Path) -> ApiResult<()> {
        self.version = 1;
        let body = serde_json::to_vec_pretty(self).map_err(anyhow::Error::from)?;
        write_atomic(&dir.join(MANIFEST_FILE), &body)?;
        Ok(())
    }
    pub fn get(&self, id: &str) -> Option<&Entry> { self.mods.iter().find(|m| m.id == id) }
    pub fn upsert(&mut self, e: Entry) {
        match self.mods.iter_mut().find(|m| m.id == e.id) { Some(m) => *m = e, None => self.mods.push(e) }
    }
    pub fn remove(&mut self, id: &str) -> Option<Entry> {
        let i = self.mods.iter().position(|m| m.id == id)?;
        Some(self.mods.remove(i))
    }
    /// Installed mods that list `id` as a requirement.
    pub fn dependents(&self, id: &str) -> Vec<&str> { self.mods.iter().filter(|m| m.id != id && m.requires.iter().any(|r| r == id)).map(|m| m.id.as_str()).collect() }
}

pub fn write_atomic(path: &Path, body: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension(format!("tmp-{}", uuid::Uuid::now_v7().simple()));
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, path).inspect_err(|_| { let _ = std::fs::remove_file(&tmp); })
}

/// Rewrites `key=value` lines in moss-game.cfg: `Some` sets or appends, `None` removes. Other lines are kept as they are.
/// Callers validate values first; a value with a line break is refused here as well.
pub fn set_settings(dir: &Path, changes: &[(&str, Option<&str>)]) -> ApiResult<()> {
    if changes.iter().any(|(k, v)| k.contains(['=', '\n', '\r']) || v.is_some_and(|v| v.contains(['\n', '\r']))) { return Err(ApiError::bad("validation", "settings values cannot contain line breaks")); }
    let path = dir.join(SETTINGS_FILE);
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    for (k, v) in changes {
        let at = lines.iter().position(|l| { let l = l.trim(); !l.starts_with('#') && l.split_once('=').is_some_and(|(lk, _)| lk.trim() == *k) });
        match (at, v) {
            (Some(i), Some(v)) => lines[i] = format!("{k}={v}"),
            (Some(i), None) => { lines.remove(i); }
            (None, Some(v)) => lines.push(format!("{k}={v}")),
            (None, None) => {}
        }
    }
    let mut out = lines.join("\n");
    if !out.is_empty() { out.push('\n'); }
    write_atomic(&path, out.as_bytes())?;
    Ok(())
}

/// Compares dotted numeric versions ("1.10.2" > "1.9"); non-numeric parts count as 0.
pub fn cmp_version(a: &str, b: &str) -> std::cmp::Ordering {
    let parts = |v: &str| v.split(['.', '-', '+']).map(|p| p.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    let (mut x, mut y) = (parts(a), parts(b));
    let n = x.len().max(y.len());
    x.resize(n, 0);
    y.resize(n, 0);
    x.cmp(&y)
}
