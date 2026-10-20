//! Plugin and mod metadata read straight from jar/zip files, without extracting anything to disk.
//! One parser per descriptor (`fabric`, `forge`, `bukkit`); `read` caches results in memory by (path, size, mtime) so
//! listing hundreds of mods stays fast. Everything is defensive: capped entry reads, capped entry counts, no path
//! extraction, and malformed descriptors simply yield no metadata instead of an error or a panic.
use serde::Serialize;
use serde_json::Value;
use std::{collections::HashMap, io::Read, path::{Path, PathBuf}, sync::{Arc, Mutex, OnceLock}, time::SystemTime};

mod bukkit;
mod fabric;
mod forge;
pub mod range;
#[cfg(test)]
pub(crate) mod tests;

/// Largest descriptor entry we will read (1 MiB); anything bigger is treated as absent.
pub const MAX_ENTRY: u64 = 1 << 20;
/// Archives with more entries than this are refused as unreadable.
pub const MAX_ENTRIES: usize = 50_000;

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Dep { pub id: String, pub required: bool, pub range: Option<String> }

/// The contract's `meta` object. `provides` and `api_version` are internal extras for the checker.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Meta {
    pub id: String,
    pub name: String,
    pub version: String,
    pub authors: Vec<String>,
    pub description: String,
    pub loader: &'static str,
    pub environment: &'static str,
    pub depends: Vec<Dep>,
    /// Declared incompatibilities as `"id"` or `"id <range>"`.
    pub breaks: Vec<String>,
    #[serde(skip)] pub provides: Vec<String>,
    #[serde(skip)] pub api_version: Option<String>,
}

impl Meta {
    pub fn new(loader: &'static str, id: impl Into<String>) -> Self {
        let id = id.into();
        Meta { name: id.clone(), id, version: String::new(), authors: vec![], description: String::new(), loader, environment: "unknown", depends: vec![], breaks: vec![], provides: vec![], api_version: None }
    }
}

/// Everything learned from one file: every descriptor found (a jar can support several loaders) or why it is unreadable.
#[derive(Clone, Debug, Default)]
pub struct Info { pub metas: Vec<Meta>, pub error: Option<String> }

impl Info {
    /// The descriptor for the first accepted loader, else the first one found.
    pub fn pick(&self, accepted: &[&str]) -> Option<&Meta> {
        accepted.iter().find_map(|l| self.metas.iter().find(|m| m.loader == *l)).or(self.metas.first())
    }
}

/// Reads a capped entry; `None` when absent, too large or unreadable.
pub(crate) fn entry<R: Read + std::io::Seek>(z: &mut zip::ZipArchive<R>, name: &str) -> Option<Vec<u8>> {
    let f = z.by_name(name).ok()?;
    if f.is_dir() || f.size() > MAX_ENTRY { return None; }
    let mut buf = Vec::with_capacity(f.size() as usize);
    f.take(MAX_ENTRY + 1).read_to_end(&mut buf).ok()?;
    (buf.len() as u64 <= MAX_ENTRY).then_some(buf)
}

pub(crate) fn json_entry<R: Read + std::io::Seek>(z: &mut zip::ZipArchive<R>, name: &str) -> Option<Value> {
    let b = entry(z, name)?;
    serde_json::from_slice(b.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&b)).ok()
}

pub(crate) fn text_entry<R: Read + std::io::Seek>(z: &mut zip::ZipArchive<R>, name: &str) -> Option<String> {
    String::from_utf8(entry(z, name)?).ok().map(|s| s.trim_start_matches('\u{feff}').to_string())
}

/// Parses an already-open archive. `stem` names datapacks, which carry no id of their own.
pub fn parse<R: Read + std::io::Seek>(reader: R, stem: &str) -> Info {
    let mut z = match zip::ZipArchive::new(reader) {
        Ok(z) if z.len() <= MAX_ENTRIES => z,
        Ok(_) => return Info { metas: vec![], error: Some("archive has too many entries".into()) },
        Err(e) => return Info { metas: vec![], error: Some(format!("not a readable zip/jar file: {e}")) },
    };
    let mut metas: Vec<Meta> = vec![];
    metas.extend(fabric::fabric(&mut z));
    metas.extend(fabric::quilt(&mut z));
    metas.extend(forge::parse(&mut z, "META-INF/neoforge.mods.toml", "neoforge"));
    metas.extend(forge::parse(&mut z, "META-INF/mods.toml", "forge"));
    metas.extend(bukkit::paper(&mut z));
    metas.extend(bukkit::plugin_yml(&mut z));
    metas.extend(bukkit::velocity(&mut z));
    metas.extend(bukkit::bungee(&mut z));
    if metas.is_empty() { metas.extend(bukkit::datapack(&mut z, stem)); }
    Info { metas, error: None }
}

type Cache = HashMap<PathBuf, (u64, Option<SystemTime>, Arc<Info>)>;

/// Blocking read of one file through the (path, size, mtime) cache.
pub fn read(path: &Path) -> Arc<Info> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let md = std::fs::metadata(path).ok();
    let key = md.as_ref().map(|m| (m.len(), m.modified().ok()));
    if let Some((size, mtime)) = key && let Some((s, t, i)) = cache.lock().unwrap_or_else(|p| p.into_inner()).get(path) && *s == size && *t == mtime { return i.clone(); }
    let stem = path.file_name().and_then(|f| f.to_str()).unwrap_or_default().trim_end_matches(".disabled").trim_end_matches(".zip").trim_end_matches(".jar").to_string();
    let info = Arc::new(match std::fs::File::open(path) {
        Ok(f) => parse(std::io::BufReader::new(f), &stem),
        Err(e) => Info { metas: vec![], error: Some(format!("cannot open file: {e}")) },
    });
    if let Some((size, mtime)) = key {
        let mut c = cache.lock().unwrap_or_else(|p| p.into_inner());
        if c.len() > 4096 { c.clear(); }
        c.insert(path.to_path_buf(), (size, mtime, info.clone()));
    }
    info
}

/// Reads many files on the blocking pool, in order.
pub async fn read_many(paths: Vec<PathBuf>) -> Vec<Arc<Info>> {
    let n = paths.len();
    tokio::task::spawn_blocking(move || paths.iter().map(|p| read(p)).collect())
        .await.unwrap_or_else(|_| (0..n).map(|_| Arc::new(Info { metas: vec![], error: Some("internal error while reading files".into()) })).collect())
}

pub(crate) fn str_of(v: &Value) -> Option<String> {
    match v { Value::String(s) => Some(s.clone()), Value::Number(n) => Some(n.to_string()), _ => None }
}

pub(crate) fn strings(v: &Value) -> Vec<String> {
    match v {
        Value::Array(a) => a.iter().filter_map(|x| str_of(x).or_else(|| str_of(&x["name"]))).collect(),
        Value::String(s) => vec![s.clone()],
        _ => vec![],
    }
}
