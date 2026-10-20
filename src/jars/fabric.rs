//! `fabric.mod.json` and `quilt.mod.json`.
use super::{Dep, Meta, json_entry, str_of, strings};
use serde_json::Value;
use std::io::{Read, Seek};

/// A version constraint that is a string or an array of alternatives.
fn range_of(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Array(a) => { let r: Vec<String> = a.iter().filter_map(str_of).collect(); (!r.is_empty()).then(|| r.join(" || ")) }
        _ => None,
    }
}

fn fabric_map(v: &Value, required: bool) -> Vec<Dep> {
    v.as_object().into_iter().flatten().map(|(id, r)| Dep { id: id.clone(), required, range: range_of(r) }).collect()
}

pub fn fabric<R: Read + Seek>(z: &mut zip::ZipArchive<R>) -> Option<Meta> {
    let j = json_entry(z, "fabric.mod.json")?;
    let mut m = Meta::new("fabric", str_of(&j["id"])?);
    m.name = str_of(&j["name"]).unwrap_or_else(|| m.id.clone());
    m.version = str_of(&j["version"]).unwrap_or_default();
    m.description = str_of(&j["description"]).unwrap_or_default();
    m.authors = strings(&j["authors"]);
    m.environment = match j["environment"].as_str() { Some("client") => "client", Some("server") => "server", Some("*") | None => "both", _ => "unknown" };
    m.depends = fabric_map(&j["depends"], true);
    m.depends.extend(fabric_map(&j["recommends"], false));
    m.breaks = j["breaks"].as_object().into_iter().flatten().map(|(id, r)| match range_of(r).filter(|r| r != "*") { Some(r) => format!("{id} {r}"), None => id.clone() }).collect();
    m.provides = strings(&j["provides"]);
    Some(m)
}

fn quilt_dep(v: &Value, required: bool) -> Option<(Dep, bool)> {
    match v {
        Value::String(s) => Some((Dep { id: s.clone(), required, range: None }, true)),
        Value::Object(_) => {
            let id = str_of(&v["id"])?;
            let optional = v["optional"].as_bool().unwrap_or(false);
            Some((Dep { id, required: required && !optional, range: range_of(&v["versions"]) }, true))
        }
        _ => None,
    }
}

pub fn quilt<R: Read + Seek>(z: &mut zip::ZipArchive<R>) -> Option<Meta> {
    let j = json_entry(z, "quilt.mod.json")?;
    let q = &j["quilt_loader"];
    let mut m = Meta::new("quilt", str_of(&q["id"])?);
    let md = &q["metadata"];
    m.name = str_of(&md["name"]).unwrap_or_else(|| m.id.clone());
    m.version = str_of(&q["version"]).unwrap_or_default();
    m.description = str_of(&md["description"]).unwrap_or_default();
    m.authors = md["contributors"].as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();
    m.environment = match j["minecraft"]["environment"].as_str() { Some("client") => "client", Some("dedicated_server") => "server", Some("*") | None => "both", _ => "unknown" };
    m.depends = q["depends"].as_array().into_iter().flatten().filter_map(|d| quilt_dep(d, true)).map(|(d, _)| d).collect();
    m.breaks = q["breaks"].as_array().into_iter().flatten().filter_map(|d| quilt_dep(d, true)).map(|(d, _)| match d.range.filter(|r| r != "*") { Some(r) => format!("{} {r}", d.id), None => d.id }).collect();
    m.provides = q["provides"].as_array().into_iter().flatten().filter_map(|p| str_of(p).or_else(|| str_of(&p["id"]))).collect();
    Some(m)
}
