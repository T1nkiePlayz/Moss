//! Steam Workshop content for Garry's Mod (a collection) and CS2 (a map). Steam downloads the content itself when the
//! server starts; Moss only validates the id and adds the launch arguments. `steam_api_key` (optional) is passed as `-authkey`.
use std::collections::HashMap;

pub const COLLECTION_KEY: &str = "workshop_collection";
pub const MAP_KEY: &str = "workshop_map";
pub const API_KEY: &str = "steam_api_key";

/// Workshop ids are plain decimal numbers.
pub fn valid_id(s: &str) -> bool { !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit()) }

fn id<'a>(cfg: &'a HashMap<String, String>, key: &str) -> Option<&'a str> { cfg.get(key).map(String::as_str).filter(|v| valid_id(v)) }

fn authkey(cfg: &HashMap<String, String>, v: &mut Vec<String>) {
    if let Some(k) = cfg.get(API_KEY).filter(|k| !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric())) { v.extend(["-authkey".into(), k.clone()]); }
}

/// Extra Garry's Mod arguments: the workshop collection to mount.
pub fn gmod_args(cfg: &HashMap<String, String>) -> Vec<String> {
    let Some(c) = id(cfg, COLLECTION_KEY) else { return vec![] };
    let mut v = vec!["+host_workshop_collection".to_string(), c.to_string()];
    authkey(cfg, &mut v);
    v
}

/// The CS2 map arguments: a workshop map when configured, otherwise `de_dust2`.
pub fn cs2_map_args(cfg: &HashMap<String, String>) -> Vec<String> {
    let mut v = match id(cfg, MAP_KEY) { Some(m) => vec!["+host_workshop_map".to_string(), m.to_string()], None => vec!["+map".to_string(), "de_dust2".to_string()] };
    if id(cfg, MAP_KEY).is_some() { authkey(cfg, &mut v); }
    v
}

/// The Steam page to browse for an id.
pub fn page_url(id: &str) -> String { format!("https://steamcommunity.com/sharedfiles/filedetails/?id={id}") }
