//! Plugin descriptors: `plugin.yml`, `paper-plugin.yml`, `bungee.yml`, `velocity-plugin.json`, and datapack `pack.mcmeta`.
use super::{Dep, Meta, json_entry, str_of, strings, text_entry};
use serde_json::Value;
use serde_yaml_ng::Value as Y;
use std::io::{Read, Seek};

fn y_str(v: &Y) -> Option<String> {
    match v { Y::String(s) => Some(s.clone()), Y::Number(n) => Some(n.to_string()), Y::Bool(b) => Some(b.to_string()), _ => None }
}

fn y_list(v: &Y) -> Vec<String> {
    match v { Y::Sequence(s) => s.iter().filter_map(y_str).collect(), other => y_str(other).into_iter().collect() }
}

fn yaml<R: Read + Seek>(z: &mut zip::ZipArchive<R>, name: &str) -> Option<Y> {
    let y: Y = serde_yaml_ng::from_str(&text_entry(z, name)?).ok()?;
    y.is_mapping().then_some(y)
}

fn base(y: &Y, loader: &'static str) -> Option<Meta> {
    let mut m = Meta::new(loader, y.get("name").and_then(y_str)?);
    m.version = y.get("version").and_then(y_str).unwrap_or_default();
    m.description = y.get("description").and_then(y_str).map(|d| d.trim().to_string()).unwrap_or_default();
    m.authors = y.get("authors").map(y_list).unwrap_or_default();
    if let Some(a) = y.get("author").and_then(y_str) { m.authors.insert(0, a); }
    m.environment = "server";
    m.api_version = y.get("api-version").and_then(y_str);
    Some(m)
}

pub fn plugin_yml<R: Read + Seek>(z: &mut zip::ZipArchive<R>) -> Option<Meta> {
    let y = yaml(z, "plugin.yml")?;
    let mut m = base(&y, "bukkit")?;
    m.depends = y.get("depend").map(y_list).unwrap_or_default().into_iter().map(|id| Dep { id, required: true, range: None }).collect();
    m.depends.extend(y.get("softdepend").map(y_list).unwrap_or_default().into_iter().map(|id| Dep { id, required: false, range: None }));
    Some(m)
}

pub fn paper<R: Read + Seek>(z: &mut zip::ZipArchive<R>) -> Option<Meta> {
    let y = yaml(z, "paper-plugin.yml")?;
    let mut m = base(&y, "paper")?;
    for (name, d) in y.get("dependencies").and_then(|d| d.get("server")).and_then(|s| s.as_mapping()).into_iter().flatten() {
        let Some(id) = y_str(name) else { continue };
        m.depends.push(Dep { id, required: d.get("required").and_then(|r| r.as_bool()).unwrap_or(true), range: None });
    }
    Some(m)
}

pub fn bungee<R: Read + Seek>(z: &mut zip::ZipArchive<R>) -> Option<Meta> {
    let y = yaml(z, "bungee.yml")?;
    let mut m = base(&y, "bungee")?;
    m.depends = y.get("depends").map(y_list).unwrap_or_default().into_iter().map(|id| Dep { id, required: true, range: None }).collect();
    m.depends.extend(y.get("softDepends").map(y_list).unwrap_or_default().into_iter().map(|id| Dep { id, required: false, range: None }));
    Some(m)
}

pub fn velocity<R: Read + Seek>(z: &mut zip::ZipArchive<R>) -> Option<Meta> {
    let j = json_entry(z, "velocity-plugin.json")?;
    let mut m = Meta::new("velocity", str_of(&j["id"])?);
    m.name = str_of(&j["name"]).unwrap_or_else(|| m.id.clone());
    m.version = str_of(&j["version"]).unwrap_or_default();
    m.description = str_of(&j["description"]).unwrap_or_default();
    m.authors = strings(&j["authors"]);
    m.environment = "server";
    m.depends = j["dependencies"].as_array().into_iter().flatten().filter_map(|d| Some(Dep { id: str_of(&d["id"])?, required: !d["optional"].as_bool().unwrap_or(false), range: None })).collect();
    Some(m)
}

/// A datapack is a zip with `pack.mcmeta`; the file stem stands in for its id.
pub fn datapack<R: Read + Seek>(z: &mut zip::ZipArchive<R>, stem: &str) -> Option<Meta> {
    let j = json_entry(z, "pack.mcmeta")?;
    let p = &j["pack"];
    let mut m = Meta::new("datapack", stem);
    m.description = match &p["description"] { Value::String(s) => s.clone(), other => str_of(&other["text"]).unwrap_or_default() };
    m.version = str_of(&p["pack_format"]).map(|f| format!("pack format {f}")).unwrap_or_default();
    m.environment = "both";
    Some(m)
}
