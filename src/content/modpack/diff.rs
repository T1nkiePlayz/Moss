//! What changes when an installed modpack moves to another version: files added, removed and updated, config
//! overrides changed, and which override files the person edited (those are never overwritten). Pure functions over
//! plain values so every rule is testable without a server.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};

/// A file the pack installed earlier, as recorded in the inventory.
#[derive(Clone, Debug, PartialEq)]
pub struct Installed {
    pub item_id: String, pub provider: Option<String>, pub project: Option<String>, pub version_id: Option<String>, pub title: String,
    pub version: Option<String>, pub path: String, pub pinned: bool, pub sha512: Option<String>,
}

/// A file the new pack version lists.
#[derive(Clone, Debug, PartialEq)]
pub struct Incoming { pub path: String, pub provider: Option<(String, String, String)>, pub sha512: Option<String>, pub title: String }

#[derive(Clone, Debug, PartialEq)]
pub struct Change { pub title: String, pub path: String, pub from: Option<String>, pub to: Option<String>, pub reason: Option<&'static str>, pub item_id: Option<String> }

impl Change {
    pub fn json(&self) -> Value { json!({"title": self.title, "path": self.path, "from": self.from, "to": self.to, "reason": self.reason}) }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FileDiff { pub added: Vec<Change>, pub removed: Vec<Change>, pub updated: Vec<Change>, pub held: Vec<Change>, pub unchanged: usize }

/// Identity of a file across versions: its project when the provider is known, otherwise its path.
fn key(provider: &Option<String>, project: &Option<String>, path: &str) -> String {
    match (provider, project) { (Some(p), Some(id)) if p != "manual" => format!("{p}:{id}"), _ => format!("path:{path}") }
}

pub fn diff_files(installed: &[Installed], incoming: &[Incoming]) -> FileDiff {
    let mut d = FileDiff::default();
    let by_key: HashMap<String, &Installed> = installed.iter().map(|i| (key(&i.provider, &i.project, &i.path), i)).collect();
    let mut seen: Vec<String> = vec![];
    for n in incoming {
        let k = key(&n.provider.as_ref().map(|p| p.0.clone()), &n.provider.as_ref().map(|p| p.1.clone()), &n.path);
        seen.push(k.clone());
        let new_ver = n.provider.as_ref().map(|p| p.2.clone());
        let Some(old) = by_key.get(&k) else { d.added.push(Change { title: n.title.clone(), path: n.path.clone(), from: None, to: new_ver, reason: None, item_id: None }); continue };
        let same = match (&old.version_id, &new_ver) { (Some(a), Some(b)) => a == b, _ => old.sha512.is_some() && old.sha512 == n.sha512 };
        if same && old.path == n.path { d.unchanged += 1; continue; }
        let ch = Change { title: old.title.clone(), path: n.path.clone(), from: old.version.clone().or(old.version_id.clone()), to: new_ver, reason: None, item_id: Some(old.item_id.clone()) };
        if old.pinned { d.held.push(Change { reason: Some("pinned"), ..ch }); }
        else if old.path.to_lowercase().ends_with(".disabled") { d.held.push(Change { reason: Some("disabled"), ..ch }); }
        else { d.updated.push(ch); }
    }
    for i in installed {
        if seen.contains(&key(&i.provider, &i.project, &i.path)) { continue; }
        let ch = Change { title: i.title.clone(), path: i.path.clone(), from: i.version.clone().or(i.version_id.clone()), to: None, reason: None, item_id: Some(i.item_id.clone()) };
        if i.pinned { d.held.push(Change { reason: Some("pinned"), ..ch }); } else { d.removed.push(ch); }
    }
    d
}

#[derive(Clone, Debug, PartialEq)]
pub enum Ov { Unchanged, Added, Updated, Edited(&'static str) }

/// What to do with one override file. `recorded` is the checksum Moss stored when the pack wrote it, `on_disk` the
/// checksum now, `new` the checksum in the new pack version.
pub fn classify_override(recorded: Option<&str>, on_disk: Option<&str>, new: &str) -> Ov {
    if on_disk == Some(new) || recorded == Some(new) { return Ov::Unchanged; }
    match (recorded, on_disk) {
        (Some(r), Some(d)) if r == d => Ov::Updated,
        (Some(_), Some(_)) => Ov::Edited("edited since the pack wrote it"),
        (Some(_), None) => Ov::Edited("deleted since the pack wrote it"),
        (None, Some(_)) => Ov::Edited("already exists and Moss has no record of what the pack wrote"),
        (None, None) => Ov::Added,
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct OverrideDiff { pub added: Vec<String>, pub updated: Vec<String>, pub edited: Vec<(String, &'static str)>, pub removed_upstream: Vec<String>, pub unchanged: usize }

impl OverrideDiff {
    /// The override files that may be written: new ones and unedited ones the pack changed. Never an edited file.
    pub fn writable(&self) -> Vec<&str> { self.added.iter().chain(self.updated.iter()).map(String::as_str).collect() }
}

/// `new`, `recorded` and `on_disk` map relative path to sha256 (`on_disk` only holds files that exist).
pub fn diff_overrides(new: &BTreeMap<String, String>, recorded: &HashMap<String, String>, on_disk: &HashMap<String, String>) -> OverrideDiff {
    let mut d = OverrideDiff::default();
    for (path, sum) in new {
        match classify_override(recorded.get(path).map(String::as_str), on_disk.get(path).map(String::as_str), sum) {
            Ov::Unchanged => d.unchanged += 1,
            Ov::Added => d.added.push(path.clone()),
            Ov::Updated => d.updated.push(path.clone()),
            Ov::Edited(why) => d.edited.push((path.clone(), why)),
        }
    }
    let mut gone: Vec<String> = recorded.keys().filter(|p| !new.contains_key(*p)).cloned().collect();
    gone.sort();
    d.removed_upstream = gone;
    d
}

/// SHA-256 of a file, hex. None when it cannot be read.
pub fn sha256_file(p: &std::path::Path) -> Option<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(p).ok()?;
    let (mut h, mut buf) = (Sha256::new(), vec![0u8; 64 * 1024]);
    loop { let n = f.read(&mut buf).ok()?; if n == 0 { break; } h.update(&buf[..n]); }
    Some(hex::encode(h.finalize()))
}

/// Checksums of server files given as relative paths (each through `safe_join`; missing or unreadable ones are left out).
pub fn sha256_paths(root: &std::path::Path, rels: &[String]) -> HashMap<String, String> {
    rels.iter().filter_map(|r| Some((r.clone(), sha256_file(&crate::files::safe_join(root, r).ok()?)?))).collect()
}

pub struct Versions { pub from: Value, pub to: Value }

pub struct Diff { pub files: FileDiff, pub overrides: OverrideDiff, pub blockers: Vec<String>, pub warnings: Vec<String> }

impl Diff {
    pub fn changes_anything(&self) -> bool {
        let f = &self.files;
        !(f.added.is_empty() && f.removed.is_empty() && f.updated.is_empty() && self.overrides.added.is_empty() && self.overrides.updated.is_empty())
    }

    /// Identifies exactly what applying would do, so a tested change can be promoted only if nothing moved since.
    pub fn signature(&self, to_version: &str) -> String {
        let mut lines: Vec<String> = vec![format!("to:{to_version}")];
        for (tag, list) in [("add", &self.files.added), ("upd", &self.files.updated), ("del", &self.files.removed)] {
            for c in list { lines.push(format!("{tag}:{}:{}", c.path, c.to.clone().unwrap_or_default())); }
        }
        for p in &self.overrides.added { lines.push(format!("ova:{p}")); }
        for p in &self.overrides.updated { lines.push(format!("ovu:{p}")); }
        lines.sort();
        hex::encode(Sha256::digest(lines.join("\n").as_bytes()))
    }

    pub fn json(&self, v: &Versions, to_version: &str) -> Value {
        let l = |c: &[Change]| c.iter().map(Change::json).collect::<Vec<_>>();
        json!({
            "versions": {"from": v.from, "to": v.to},
            "files": {"added": l(&self.files.added), "removed": l(&self.files.removed), "updated": l(&self.files.updated), "held": l(&self.files.held), "unchanged": self.files.unchanged},
            "overrides": {
                "added": self.overrides.added, "updated": self.overrides.updated,
                "user_modified": self.overrides.edited.iter().map(|(p, w)| json!({"path": p, "reason": w})).collect::<Vec<_>>(),
                "removed_upstream": self.overrides.removed_upstream, "unchanged": self.overrides.unchanged,
            },
            "blockers": self.blockers, "warnings": self.warnings, "can_apply": self.blockers.is_empty(), "changes": self.changes_anything(), "signature": self.signature(to_version),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inst(item: &str, project: &str, ver: &str, path: &str) -> Installed {
        Installed { item_id: item.into(), provider: Some("modrinth".into()), project: Some(project.into()), version_id: Some(ver.into()), title: project.to_uppercase(), version: Some(format!("v{ver}")), path: path.into(), pinned: false, sha512: None }
    }
    fn inc(project: &str, ver: &str, path: &str) -> Incoming { Incoming { path: path.into(), provider: Some(("modrinth".into(), project.into(), ver.into())), sha512: None, title: project.to_uppercase() } }

    #[test]
    fn files_are_added_removed_updated_and_unchanged_by_project() {
        let old = [inst("1", "a", "1", "mods/a-1.jar"), inst("2", "b", "1", "mods/b-1.jar"), inst("3", "c", "1", "mods/c-1.jar")];
        let new = [inc("a", "1", "mods/a-1.jar"), inc("b", "2", "mods/b-2.jar"), inc("d", "1", "mods/d-1.jar")];
        let d = diff_files(&old, &new);
        assert_eq!(d.unchanged, 1);
        assert_eq!((d.updated.len(), d.updated[0].title.as_str(), d.updated[0].from.as_deref(), d.updated[0].to.as_deref(), d.updated[0].path.as_str()), (1, "B", Some("v1"), Some("2"), "mods/b-2.jar"));
        assert_eq!((d.added.len(), d.added[0].path.as_str()), (1, "mods/d-1.jar"));
        assert_eq!((d.removed.len(), d.removed[0].title.as_str()), (1, "C"));
        assert!(d.held.is_empty());
    }

    #[test]
    fn pinned_and_disabled_items_are_held_not_changed() {
        let mut pinned = inst("1", "a", "1", "mods/a-1.jar");
        pinned.pinned = true;
        let disabled = inst("2", "b", "1", "mods/b-1.jar.disabled");
        let mut pinned_gone = inst("3", "c", "1", "mods/c-1.jar");
        pinned_gone.pinned = true;
        let d = diff_files(&[pinned, disabled, pinned_gone], &[inc("a", "2", "mods/a-2.jar"), inc("b", "2", "mods/b-2.jar")]);
        assert!(d.updated.is_empty() && d.removed.is_empty() && d.added.is_empty());
        let why: Vec<(&str, Option<&str>)> = d.held.iter().map(|h| (h.title.as_str(), h.reason)).collect();
        assert_eq!(why, vec![("A", Some("pinned")), ("B", Some("disabled")), ("C", Some("pinned"))]);
    }

    #[test]
    fn files_without_a_provider_match_by_path_and_compare_hashes() {
        let manual = Installed { provider: Some("manual".into()), project: None, version_id: None, sha512: Some("h1".into()), ..inst("1", "x", "1", "mods/x.jar") };
        let same = Incoming { path: "mods/x.jar".into(), provider: None, sha512: Some("h1".into()), title: "x".into() };
        let changed = Incoming { sha512: Some("h2".into()), ..same.clone() };
        assert_eq!(diff_files(std::slice::from_ref(&manual), &[same]).unchanged, 1);
        assert_eq!(diff_files(std::slice::from_ref(&manual), &[changed]).updated.len(), 1);
    }

    #[test]
    fn override_rules_never_overwrite_what_the_person_changed() {
        use Ov::*;
        assert_eq!(classify_override(Some("a"), Some("a"), "b"), Updated, "untouched since the pack wrote it: safe to update");
        assert!(matches!(classify_override(Some("a"), Some("x"), "b"), Edited(_)), "edited by the person");
        assert!(matches!(classify_override(Some("a"), None, "b"), Edited(w) if w.contains("deleted")));
        assert!(matches!(classify_override(None, Some("x"), "b"), Edited(_)), "no baseline: keep the existing file");
        assert_eq!(classify_override(None, None, "b"), Added);
        assert_eq!(classify_override(Some("a"), Some("x"), "a"), Unchanged, "the pack did not change it; the person's edit stays");
        assert_eq!(classify_override(None, Some("b"), "b"), Unchanged);
        assert_eq!(classify_override(Some("a"), Some("b"), "b"), Unchanged);
    }

    #[test]
    fn override_diff_lists_every_class_and_only_unedited_files_are_writable() {
        let new: BTreeMap<String, String> = [("config/new.toml", "n"), ("config/changed.toml", "c2"), ("config/edited.toml", "e2"), ("config/same.toml", "s")].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        let rec: HashMap<String, String> = [("config/changed.toml", "c1"), ("config/edited.toml", "e1"), ("config/same.toml", "s"), ("config/dropped.toml", "d")].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        let disk: HashMap<String, String> = [("config/changed.toml", "c1"), ("config/edited.toml", "mine"), ("config/same.toml", "s"), ("config/dropped.toml", "d")].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        let d = diff_overrides(&new, &rec, &disk);
        assert_eq!((d.added.as_slice(), d.updated.as_slice(), d.unchanged), (&["config/new.toml".to_string()][..], &["config/changed.toml".to_string()][..], 1));
        assert_eq!(d.edited.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>(), vec!["config/edited.toml"]);
        assert_eq!(d.removed_upstream, vec!["config/dropped.toml".to_string()]);
        let w = d.writable();
        assert!(w.contains(&"config/new.toml") && w.contains(&"config/changed.toml") && !w.contains(&"config/edited.toml"));
    }

    #[test]
    fn signature_changes_with_the_plan_and_ignores_order() {
        let mk = |adds: &[&str]| Diff { files: FileDiff { added: adds.iter().map(|p| Change { title: p.to_string(), path: format!("mods/{p}.jar"), from: None, to: Some("1".into()), reason: None, item_id: None }).collect(), ..Default::default() }, overrides: OverrideDiff::default(), blockers: vec![], warnings: vec![] };
        assert_eq!(mk(&["a", "b"]).signature("2"), mk(&["b", "a"]).signature("2"));
        assert_ne!(mk(&["a"]).signature("2"), mk(&["a", "b"]).signature("2"));
        assert_ne!(mk(&["a"]).signature("2"), mk(&["a"]).signature("3"));
        assert!(!mk(&[]).changes_anything() && mk(&["a"]).changes_anything());
    }
}
