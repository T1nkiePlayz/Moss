//! Structured editors for the Paper-family and Velocity configuration files. Each file has a curated catalogue of
//! commonly changed keys (`catalogue`); edits change single lines of the text (`yamltext`, `tomltext`) so comments
//! and unknown keys survive, every write takes an undo snapshot, and the raw text is always available.
//! Routes: `GET /servers/{id}/config`, `GET|PUT /servers/{id}/config/{file}`, `GET /servers/{id}/config/{file}/diff`.
use crate::{api::server_for, auth::User, error::{ApiError, ApiResult}, files::{atomic_write, safe_join}, server::Srv, state::St, undo};
use axum::{Json, extract::{Path, Query, State}};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::collections::HashMap;

pub mod catalogue;
mod tomltext;
mod yamltext;
use catalogue::{Key, cmp_version, parse_version};

const MAX_BYTES: u64 = 1 << 20;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Fmt { Yaml, Toml }

struct Def { id: String, title: String, rel: String, fmt: Fmt, keys: &'static [Key] }

fn paper_family(s: &Srv) -> bool { matches!(s.runtime.as_str(), "paper" | "folia" | "purpur") }
fn at_least(version: &str, min: &str) -> bool { let v = parse_version(version); v.is_empty() || cmp_version(&v, &parse_version(min)).is_ge() }

/// A world folder name that is safe to use as a path component.
fn world_name_ok(n: &str) -> bool { !n.is_empty() && n.len() <= 64 && !n.starts_with('.') && !n.contains(['/', '\\', '\0']) }

/// Files that exist for this server's runtime and version (worlds are added by `list`).
fn defs_for(s: &Srv) -> Vec<Def> {
    let d = |id: &str, title: &str, rel: &str, fmt, keys| Def { id: id.into(), title: title.into(), rel: rel.into(), fmt, keys };
    let mut v = vec![];
    if paper_family(s) {
        if at_least(&s.game_version, "1.19") {
            v.push(d("paper-global", "Paper global", "config/paper-global.yml", Fmt::Yaml, catalogue::PAPER_GLOBAL));
            v.push(d("paper-world-defaults", "Paper world defaults", "config/paper-world-defaults.yml", Fmt::Yaml, catalogue::PAPER_WORLD));
        }
        v.push(d("spigot", "Spigot", "spigot.yml", Fmt::Yaml, catalogue::SPIGOT));
        v.push(d("bukkit", "Bukkit", "bukkit.yml", Fmt::Yaml, catalogue::BUKKIT));
        if s.runtime == "purpur" { v.push(d("purpur", "Purpur", "purpur.yml", Fmt::Yaml, catalogue::PURPUR)); }
    }
    if s.is_proxy() { v.push(d("velocity", "Velocity", "velocity.toml", Fmt::Toml, catalogue::VELOCITY)); }
    v
}

fn world_def(s: &Srv, name: &str) -> Option<Def> {
    (paper_family(s) && at_least(&s.game_version, "1.19") && world_name_ok(name))
        .then(|| Def { id: format!("world--{name}"), title: format!("Paper world: {name}"), rel: format!("{name}/paper-world.yml"), fmt: Fmt::Yaml, keys: catalogue::PAPER_WORLD })
}

fn find_def(s: &Srv, file: &str) -> ApiResult<Def> {
    let found = match file.strip_prefix("world--") { Some(w) => world_def(s, w), None => defs_for(s).into_iter().find(|d| d.id == file) };
    found.ok_or_else(|| ApiError::not_found("configuration file"))
}

/// World folders that Paper has (or will have) a `paper-world.yml` for.
fn world_dirs(root: &std::path::Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(root).into_iter().flatten().flatten().filter_map(|e| {
        let n = e.file_name().to_string_lossy().to_string();
        let t = e.file_type().ok()?;
        (t.is_dir() && world_name_ok(&n) && (e.path().join("level.dat").is_file() || e.path().join("paper-world.yml").is_file())).then_some(n)
    }).collect();
    v.sort();
    v
}

fn to_json(fmt: Fmt, raw: &str) -> Result<Value, String> {
    match fmt {
        Fmt::Yaml => { let y: serde_yaml_ng::Value = serde_yaml_ng::from_str(raw).map_err(|e| e.to_string())?; Ok(yaml_json(&y)) }
        Fmt::Toml => { let t: toml::Value = toml::from_str(raw).map_err(|e| e.to_string())?; serde_json::to_value(t).map_err(|e| e.to_string()) }
    }
}
fn yaml_json(y: &serde_yaml_ng::Value) -> Value {
    use serde_yaml_ng::Value as Y;
    match y {
        Y::Null => Value::Null,
        Y::Bool(b) => json!(b),
        Y::Number(n) => n.as_i64().map(|i| json!(i)).or_else(|| n.as_u64().map(|u| json!(u))).or_else(|| n.as_f64().map(|f| json!(f))).unwrap_or(Value::Null),
        Y::String(s) => json!(s),
        Y::Sequence(a) => Value::Array(a.iter().map(yaml_json).collect()),
        Y::Mapping(m) => Value::Object(m.iter().map(|(k, v)| (match k { Y::String(s) => s.clone(), o => serde_yaml_ng::to_string(o).unwrap_or_default().trim().to_string() }, yaml_json(v))).collect()),
        Y::Tagged(t) => yaml_json(&t.value),
    }
}
fn lookup<'a>(doc: &'a Value, path: &str) -> Option<&'a Value> { path.split('.').try_fold(doc, |v, k| v.get(k)).filter(|v| !v.is_null()) }

async fn read_raw(path: std::path::PathBuf) -> ApiResult<Option<String>> {
    tokio::task::spawn_blocking(move || -> ApiResult<Option<String>> {
        let m = match std::fs::metadata(&path) { Ok(m) => m, Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None), Err(e) => return Err(e.into()) };
        if !m.is_file() { return Err(ApiError::bad("path.not_file", "not a file")); }
        if m.len() > MAX_BYTES { return Err(ApiError::bad("file.too_large", "this file is too large for the editor")); }
        Ok(Some(String::from_utf8_lossy(&std::fs::read(&path)?).into_owned()))
    }).await.map_err(|e| anyhow::anyhow!(e))?
}

/// Undo snapshots that touched `rel`, newest first: (id, title, created_at, existed).
async fn history(st: &St, s: &Srv, rel: &str) -> ApiResult<Vec<(String, String, String, bool)>> {
    let (sid, rel) = (s.id.clone(), rel.to_string());
    Ok(st.db.call(move |c| {
        let mut q = c.prepare("SELECT id,title,created_at,files FROM undo WHERE server_id=? ORDER BY id DESC")?;
        let rows = q.query_map([sid], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?)))?;
        let mut out = vec![];
        for r in rows {
            let (id, title, at, files) = r?;
            let files: Vec<Value> = serde_json::from_str(&files).unwrap_or_default();
            if let Some(f) = files.iter().find(|f| f["path"] == rel.as_str()) { out.push((id, title, at, f["existed"] == true && f["skipped"] != true)); }
        }
        Ok(out)
    }).await?)
}

pub async fn list(State(st): State<St>, user: User, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let s = server_for(&st, &user, &id, "files.read").await?;
    let root = s.dir(&st);
    let mut defs = defs_for(&s);
    let r2 = root.clone();
    if world_def(&s, "w").is_some() {
        for w in tokio::task::spawn_blocking(move || world_dirs(&r2)).await.map_err(|e| anyhow::anyhow!(e))? { defs.extend(world_def(&s, &w)); }
    }
    let files: Vec<Value> = defs.iter().map(|d| json!({"id": d.id, "title": d.title, "path": d.rel, "format": if d.fmt == Fmt::Yaml { "yaml" } else { "toml" }, "exists": root.join(&d.rel).is_file()})).collect();
    Ok(Json(json!({"files": files})))
}

pub async fn get(State(st): State<St>, user: User, Path((id, file)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    let s = server_for(&st, &user, &id, "files.read").await?;
    let def = find_def(&s, &file)?;
    let raw = read_raw(safe_join(&s.dir(&st), &def.rel)?).await?;
    let hist = history(&st, &s, &def.rel).await?;
    let (doc, parse_error) = match &raw { Some(r) => match to_json(def.fmt, r) { Ok(d) => (d, None), Err(e) => (Value::Null, Some(e)) }, None => (Value::Null, None) };
    let fields: Vec<Value> = def.keys.iter().filter(|k| k.exists_in(&s.game_version)).map(|k| {
        let v = lookup(&doc, k.path);
        json!({"key": k.path, "type": k.ty.name(), "default": k.default_json(), "group": k.group, "description": k.desc, "min": k.min, "max": k.max, "options": k.options,
            "restart": k.restart, "secret": k.secret, "danger": k.danger, "mixed": k.mixed, "present": v.is_some(), "value": if k.secret { Value::Null } else { v.cloned().unwrap_or(Value::Null) }})
    }).collect();
    let history: Vec<Value> = hist.iter().map(|(i, t, a, _)| json!({"id": i, "title": t, "created_at": a})).collect();
    Ok(Json(json!({"id": def.id, "title": def.title, "path": def.rel, "format": if def.fmt == Fmt::Yaml { "yaml" } else { "toml" }, "exists": raw.is_some(),
        "raw": raw.unwrap_or_default(), "parse_error": parse_error, "fields": fields, "history": history})))
}

#[derive(Deserialize)]
pub struct PutReq { raw: Option<String>, set: Option<Map<String, Value>>, unset: Option<Vec<String>> }

fn apply_changes(def: &Def, version: &str, mut text: String, r: &PutReq) -> ApiResult<String> {
    for (k, v) in r.set.iter().flatten() {
        let key = def.keys.iter().find(|x| x.path == k && x.exists_in(version)).ok_or_else(|| ApiError::bad("config.unknown_key", format!("{k} is not an editable setting here")).field(k, "unknown setting"))?;
        key.validate(v).map_err(|m| ApiError::bad("validation", format!("{k}: {m}")).field(k, &m))?;
        let path: Vec<&str> = k.split('.').collect();
        let float = key.ty == catalogue::Ty::Float;
        let bad = |m: String| ApiError::bad("config.unsupported_edit", m).field(k, "use the raw editor");
        text = match def.fmt {
            Fmt::Yaml => yamltext::literal(v, float).and_then(|l| yamltext::set(&text, &path, &l)),
            Fmt::Toml => tomltext::literal(v, float).and_then(|l| tomltext::set(&text, &path, &l)),
        }.map_err(bad)?;
    }
    for k in r.unset.iter().flatten() {
        if !def.keys.iter().any(|x| x.path == k) { return Err(ApiError::bad("config.unknown_key", format!("{k} is not an editable setting here")).field(k, "unknown setting")); }
        let path: Vec<&str> = k.split('.').collect();
        text = match def.fmt { Fmt::Yaml => yamltext::unset(&text, &path), Fmt::Toml => tomltext::unset(&text, &path) }.map_err(|m| ApiError::bad("config.unsupported_edit", m).field(k, "use the raw editor"))?;
    }
    Ok(text)
}

pub async fn put(State(st): State<St>, user: User, Path((id, file)): Path<(String, String)>, Json(r): Json<PutReq>) -> ApiResult<Json<Value>> {
    let s = server_for(&st, &user, &id, "config.write").await?;
    let def = find_def(&s, &file)?;
    if r.raw.is_some() == (r.set.is_some() || r.unset.is_some()) { return Err(ApiError::bad("validation", "send either raw, or set/unset")); }
    let path = safe_join(&s.dir(&st), &def.rel)?;
    let old = read_raw(path.clone()).await?.ok_or_else(|| ApiError::conflict("config.missing", format!("{} does not exist yet. Start the server once so it creates the file.", def.rel)))?;
    let new = match &r.raw { Some(raw) => raw.clone(), None => apply_changes(&def, &s.game_version, old.clone(), &r)? };
    if new.len() as u64 > MAX_BYTES || new.contains('\0') { return Err(ApiError::bad("file.too_large", "the file is too large or not text")); }
    if new == old { return Ok(Json(json!({"undo_id": null, "changed": false}))); }
    let doc = to_json(def.fmt, &new).map_err(|e| ApiError::bad("config.invalid", format!("the result is not valid {}: {e}", if def.fmt == Fmt::Yaml { "YAML" } else { "TOML" })))?;
    let port = (def.id == "velocity").then(|| doc.get("bind").and_then(Value::as_str).and_then(|b| b.rsplit_once(':')).and_then(|(_, p)| p.parse::<i64>().ok())).flatten();
    if let Some(p) = port {
        if !(1..=65535).contains(&p) { return Err(ApiError::bad("validation", "invalid port in bind").field("bind", "1–65535")); }
        if crate::server::list(&st).await?.iter().any(|o| o.id != id && o.port == p) { return Err(ApiError::conflict("port.taken", "that port is assigned to another Moss server").field("bind", "already assigned")); }
    }
    let undo_id = undo::snapshot(&st, &s, &format!("Edit {}", def.rel), std::slice::from_ref(&def.rel)).await?;
    let (p2, n2) = (path, new);
    tokio::task::spawn_blocking(move || atomic_write(&p2, n2.as_bytes())).await.map_err(|e| anyhow::anyhow!(e))??;
    if let Some(p) = port.filter(|p| *p != s.port) {
        let i = id.clone();
        st.db.call(move |c| c.execute("UPDATE servers SET port=? WHERE id=?", rusqlite::params![p, i])).await?;
    }
    st.audit(&user.actor(), "config.write", &s.name, &def.rel, Some(&id)).await;
    Ok(Json(json!({"undo_id": undo_id, "changed": true})))
}

/// Current text against a previous snapshot (the latest by default), for the diff view.
pub async fn diff(State(st): State<St>, user: User, Path((id, file)): Path<(String, String)>, Query(q): Query<HashMap<String, String>>) -> ApiResult<Json<Value>> {
    let s = server_for(&st, &user, &id, "files.read").await?;
    let def = find_def(&s, &file)?;
    let current = read_raw(safe_join(&s.dir(&st), &def.rel)?).await?.unwrap_or_default();
    let hist = history(&st, &s, &def.rel).await?;
    // only ids from this file's own history are ever turned into a path
    let pick = match q.get("snapshot") { Some(w) => hist.iter().find(|h| &h.0 == w).ok_or_else(|| ApiError::not_found("snapshot"))?, None => match hist.first() { Some(h) => h, None => return Ok(Json(json!({"current": current, "previous": null, "snapshot": null}))) } };
    let previous = if pick.3 { read_raw(s.moss_dir(&st).join("undo").join(&pick.0).join(&def.rel)).await?.unwrap_or_default() } else { String::new() };
    Ok(Json(json!({"current": current, "previous": previous, "snapshot": {"id": pick.0, "title": pick.1, "created_at": pick.2}})))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn srv(runtime: &str, version: &str) -> Srv {
        Srv { id: "s".into(), name: "n".into(), runtime: runtime.into(), game_version: version.into(), loader_version: None, java_major: 21, port: 25565, memory_mb: 1024, isolation: "process".into(), container_image: None, in_container: false, network: "open".into(), extra_ports: "[]".into(), flags_profile: "balanced".into(),
            autostart: false, restart_enabled: true, restart_max: 3, eula_at: None, desired: "stopped".into(), installed: true, install_error: None, pid: None, pid_start: None,
            tags: vec![], notes: String::new(), custom_cmd: None, created_at: String::new(), memory_limit_mb: None, cpu_limit_pct: None, pids_limit: None }
    }
    #[test]
    fn files_depend_on_runtime_and_version() {
        let ids = |s: &Srv| defs_for(s).into_iter().map(|d| d.id).collect::<Vec<_>>();
        assert_eq!(ids(&srv("paper", "1.21.4")), ["paper-global", "paper-world-defaults", "spigot", "bukkit"]);
        assert_eq!(ids(&srv("paper", "1.18.2")), ["spigot", "bukkit"], "paper-global.yml starts with 1.19");
        assert!(ids(&srv("purpur", "1.21")).contains(&"purpur".to_string()));
        assert_eq!(ids(&srv("velocity", "3.4.0-SNAPSHOT")), ["velocity"]);
        assert!(ids(&srv("vanilla", "1.21")).is_empty() && ids(&srv("fabric", "1.21")).is_empty());
        assert!(find_def(&srv("paper", "1.21"), "purpur").is_err());
        assert!(find_def(&srv("paper", "1.21"), "world--world").is_ok());
        for bad in ["world--../etc", "world--.hidden", "world--a/b", "world--"] { assert!(find_def(&srv("paper", "1.21"), bad).is_err(), "{bad}"); }
        assert!(find_def(&srv("velocity", "3.4"), "world--world").is_err());
    }
    #[test]
    fn world_dirs_need_a_level_or_config() {
        let d = std::env::temp_dir().join(format!("moss-cfg-{}", uuid::Uuid::new_v4()));
        for (n, f) in [("world", "level.dat"), ("custom", "paper-world.yml"), ("plugins", "x.jar")] { std::fs::create_dir_all(d.join(n)).unwrap(); std::fs::write(d.join(n).join(f), b"").unwrap(); }
        assert_eq!(world_dirs(&d), ["custom", "world"]);
        let _ = std::fs::remove_dir_all(d);
    }
    #[test]
    fn structured_changes_validate_and_keep_text() {
        let def = Def { id: "bukkit".into(), title: String::new(), rel: "bukkit.yml".into(), fmt: Fmt::Yaml, keys: catalogue::BUKKIT };
        let doc = "# Bukkit\nsettings:\n  allow-end: true # keep\nspawn-limits:\n  monsters: 70\n";
        let req = |set: Value, unset: Vec<&str>| PutReq { raw: None, set: set.as_object().cloned(), unset: Some(unset.into_iter().map(String::from).collect()) };
        let out = apply_changes(&def, "1.21", doc.into(), &req(json!({"spawn-limits.monsters": 30, "settings.allow-end": false, "spawn-limits.axolotls": 2}), vec![])).unwrap();
        let j = to_json(Fmt::Yaml, &out).unwrap();
        assert_eq!((j["spawn-limits"]["monsters"].as_i64(), j["settings"]["allow-end"].as_bool(), j["spawn-limits"]["axolotls"].as_i64()), (Some(30), Some(false), Some(2)));
        assert!(out.contains("# Bukkit") && out.contains("allow-end: false # keep"));
        assert!(apply_changes(&def, "1.21", doc.into(), &req(json!({"spawn-limits.monsters": -5}), vec![])).is_err());
        assert!(apply_changes(&def, "1.21", doc.into(), &req(json!({"settings.not-real": 1}), vec![])).is_err());
        assert!(apply_changes(&def, "1.16.5", doc.into(), &req(json!({"spawn-limits.axolotls": 2}), vec![])).is_err(), "key does not exist before 1.17");
        let out = apply_changes(&def, "1.21", doc.into(), &req(json!({}), vec!["spawn-limits.monsters"])).unwrap();
        assert!(!out.contains("monsters"));
    }
    #[test]
    fn lookup_and_yaml_conversion() {
        let doc = to_json(Fmt::Yaml, "a:\n  b: 1\n  c: ~\n5: x\n").unwrap();
        assert_eq!(lookup(&doc, "a.b"), Some(&json!(1)));
        assert_eq!(lookup(&doc, "a.c"), None);
        assert_eq!(lookup(&doc, "5"), Some(&json!("x")));
        assert!(to_json(Fmt::Yaml, "a: [").is_err() && to_json(Fmt::Toml, "a = ").is_err());
    }
}
