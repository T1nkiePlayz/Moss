//! `META-INF/mods.toml` (Forge) and `META-INF/neoforge.mods.toml` (NeoForge).
use super::{Dep, Meta, text_entry};
use std::io::{Read, Seek};

/// `${file.jarVersion}` placeholders resolve from the jar manifest's Implementation-Version.
fn manifest_version<R: Read + Seek>(z: &mut zip::ZipArchive<R>) -> Option<String> {
    text_entry(z, "META-INF/MANIFEST.MF")?.lines().find_map(|l| l.strip_prefix("Implementation-Version:").map(|v| v.trim().to_string()))
}

fn s(t: &toml::Value, k: &str) -> Option<String> {
    match t.get(k)? { toml::Value::String(x) => Some(x.clone()), toml::Value::Integer(i) => Some(i.to_string()), toml::Value::Float(f) => Some(f.to_string()), _ => None }
}

pub fn parse<R: Read + Seek>(z: &mut zip::ZipArchive<R>, path: &str, loader: &'static str) -> Option<Meta> {
    let doc: toml::Value = toml::from_str(&text_entry(z, path)?).ok()?;
    let first = doc.get("mods")?.as_array()?.first()?;
    let mut m = Meta::new(loader, s(first, "modId")?);
    m.name = s(first, "displayName").unwrap_or_else(|| m.id.clone());
    m.description = s(first, "description").map(|d| d.trim().to_string()).unwrap_or_default();
    m.authors = s(&doc, "authors").or_else(|| s(first, "authors")).map(|a| a.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()).unwrap_or_default();
    let mut version = s(first, "version").unwrap_or_default();
    if version.contains("${") { version = manifest_version(z).unwrap_or_default(); }
    m.version = version;
    let mut client_only = false;
    let deps = doc.get("dependencies").and_then(|d| d.get(&m.id)).and_then(|d| d.as_array());
    for d in deps.into_iter().flatten() {
        let Some(id) = s(d, "modId") else { continue };
        let side = s(d, "side").unwrap_or_else(|| "BOTH".into()).to_uppercase();
        if side == "CLIENT" {
            if matches!(id.as_str(), "minecraft" | "forge" | "neoforge") { client_only = true; }
            continue; // a client-side dependency is not needed on a server
        }
        // a bare version is a Maven "soft" requirement: anything is accepted
        let range = s(d, "versionRange").map(|r| r.trim().to_string()).filter(|r| r.starts_with(['[', '(']));
        let kind = s(d, "type").map(|t| t.to_lowercase());
        match kind.as_deref() {
            Some("incompatible") => m.breaks.push(match &range { Some(r) => format!("{id} {r}"), None => id }),
            Some("discouraged") => {}
            Some("optional") => m.depends.push(Dep { id, required: false, range }),
            Some(_) => m.depends.push(Dep { id, required: true, range }),
            None => m.depends.push(Dep { id, required: d.get("mandatory").and_then(|v| v.as_bool()).unwrap_or(true), range }),
        }
    }
    m.environment = if client_only { "client" } else { "both" };
    Some(m)
}
