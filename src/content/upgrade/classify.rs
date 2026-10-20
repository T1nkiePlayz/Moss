//! Pure upgrade logic: classify installed content against what the providers offer for the target version, and
//! decide the plan's blockers. Provider answers are injected so this is testable without a network.
use crate::runtimes::modded::vkey;
use serde_json::{Value, json};
use std::collections::HashMap;

/// One installed item as the upgrade assistant sees it (a linked provider item or an unidentified file).
#[derive(Clone, Debug, PartialEq)]
pub struct Linked {
    pub item: String,
    pub title: String,
    pub provider: Option<String>,
    pub project_id: Option<String>,
    pub version_id: Option<String>,
    pub current: Option<String>,
    pub filename: String,
    pub pinned: bool,
}

/// The provider's answer for one item against the target game version (and loader).
#[derive(Clone, Debug, PartialEq)]
pub enum Lookup { Build { version_id: String, label: String }, NoBuild, Failed(String) }

#[derive(Clone, Debug, PartialEq)]
pub struct Entry { pub link: Linked, pub status: &'static str, pub target: Option<String>, pub version_id: Option<String>, pub reason: Option<String> }

impl Entry {
    pub fn json(&self) -> Value {
        json!({"item": self.link.item, "title": self.link.title, "status": self.status, "current": self.link.current, "target": self.target, "version_id": self.version_id, "reason": self.reason})
    }
}

pub fn classify(items: &[Linked], found: &HashMap<String, Lookup>) -> Vec<Entry> {
    items.iter().map(|l| {
        let mk = |status, target: Option<String>, version_id: Option<String>, reason: Option<&str>| Entry { link: l.clone(), status, target, version_id, reason: reason.map(String::from) };
        if l.project_id.is_none() || l.provider.as_deref().is_none_or(|p| p == "manual") {
            return mk("unknown", None, None, Some("not linked to Modrinth or CurseForge; identify it first or check it by hand"));
        }
        match found.get(&l.item) {
            Some(Lookup::Build { version_id, label }) if l.version_id.as_deref() == Some(version_id) => mk("ok", Some(label.clone()), Some(version_id.clone()), Some("the installed build already supports the target version")),
            Some(Lookup::Build { version_id, label }) if l.pinned => mk("ok", Some(label.clone()), Some(version_id.clone()), Some("pinned: kept at the installed version")),
            Some(Lookup::Build { version_id, label }) => mk("update", Some(label.clone()), Some(version_id.clone()), None),
            Some(Lookup::NoBuild) => mk("missing", None, None, Some("no build for the target version")),
            Some(Lookup::Failed(why)) => mk("unknown", None, None, Some(why)),
            None => mk("unknown", None, None, Some("the provider did not answer for this item")),
        }
    }).collect()
}

/// `true` when `to` is an older Minecraft release than `from`. Snapshots and unparsable names never count.
pub fn is_downgrade(from: &str, to: &str) -> bool {
    let plain = |v: &str| v.chars().all(|c| c.is_ascii_digit() || c == '.') && vkey(v).len() >= 2;
    plain(from) && plain(to) && vkey(to) < vkey(from)
}

pub struct Facts<'a> { pub running: bool, pub from: &'a str, pub to: &'a str, pub from_loader: Option<&'a str>, pub to_loader: Option<&'a str>, pub confirm_downgrade: bool, pub jar_error: Option<&'a str> }

pub fn blockers(f: &Facts) -> Vec<String> {
    let mut b = vec![];
    if f.running { b.push("The server is running. Stop it before upgrading.".to_string()); }
    if let Some(e) = f.jar_error { b.push(format!("Minecraft {} cannot be installed on this runtime: {e}", f.to)); }
    if f.jar_error.is_none() && f.from == f.to && f.from_loader == f.to_loader { b.push("The server already runs this version.".into()); }
    if is_downgrade(f.from, f.to) && !f.confirm_downgrade { b.push(format!("{} is older than {}. Downgrading can corrupt worlds; confirm the downgrade to continue.", f.to, f.from)); }
    b
}

pub fn warnings(from: &str, to: &str, java: (i64, u32), entries: &[Entry]) -> Vec<String> {
    let mut w = vec![];
    if java.0 != java.1 as i64 { w.push(format!("Java changes from {} to {}.", java.0, java.1)); }
    if is_downgrade(from, to) { w.push("Worlds saved by a newer Minecraft version usually cannot be loaded by an older one. Restore the backup if the world fails to load.".into()); }
    let count = |s: &str| entries.iter().filter(|e| e.status == s).count();
    if count("missing") > 0 { w.push(format!("{} item(s) have no build for {to}; they will stop working unless you disable or replace them.", count("missing"))); }
    if count("unknown") > 0 { w.push(format!("{} item(s) could not be checked and are left as they are.", count("unknown"))); }
    w
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l(item: &str, prov: Option<&str>, pid: Option<&str>, vid: Option<&str>) -> Linked {
        Linked { item: item.into(), title: item.into(), provider: prov.map(String::from), project_id: pid.map(String::from), version_id: vid.map(String::from), current: vid.map(|v| format!("v-{v}")), filename: format!("{item}.jar"), pinned: false }
    }
    fn build(v: &str) -> Lookup { Lookup::Build { version_id: v.into(), label: format!("v-{v}") } }

    #[test]
    fn classifies_every_status() {
        let mut pinned = l("pin", Some("modrinth"), Some("p4"), Some("old"));
        pinned.pinned = true;
        let items = [l("same", Some("modrinth"), Some("p1"), Some("v1")), l("newer", Some("curseforge"), Some("p2"), Some("v1")), l("gone", Some("modrinth"), Some("p3"), Some("v1")),
            l("manual:a.jar", None, None, None), l("flaky", Some("modrinth"), Some("p5"), Some("v1")), pinned, l("silent", Some("modrinth"), Some("p6"), Some("v1"))];
        let found = HashMap::from([("same".into(), build("v1")), ("newer".into(), build("v2")), ("gone".into(), Lookup::NoBuild), ("flaky".into(), Lookup::Failed("Modrinth is down".into())), ("pin".into(), build("new"))]);
        let out = classify(&items, &found);
        let s: Vec<&str> = out.iter().map(|e| e.status).collect();
        assert_eq!(s, ["ok", "update", "missing", "unknown", "unknown", "ok", "unknown"]);
        assert_eq!(out[1].version_id.as_deref(), Some("v2"));
        assert_eq!(out[1].json()["current"], "v-v1");
        assert_eq!(out[1].json()["target"], "v-v2");
        assert!(out[2].reason.as_deref().unwrap().contains("no build"));
        assert_eq!(out[4].reason.as_deref(), Some("Modrinth is down"));
        assert!(out[5].reason.as_deref().unwrap().contains("pinned"));
    }

    #[test]
    fn downgrade_detection_compares_numeric_versions() {
        assert!(is_downgrade("1.21.1", "1.20.6") && is_downgrade("1.21", "1.20.4") && is_downgrade("1.21.1", "1.21"));
        assert!(!is_downgrade("1.20.4", "1.21") && !is_downgrade("1.21", "1.21") && !is_downgrade("1.21", "25w14a") && !is_downgrade("custom", "1.0"));
        assert!(!is_downgrade("1.9", "1.10"));
    }

    #[test]
    fn blockers_cover_running_missing_jar_downgrade_and_noop() {
        let base = Facts { running: false, from: "1.20.4", to: "1.21", from_loader: Some("a"), to_loader: Some("a"), confirm_downgrade: false, jar_error: None };
        assert!(blockers(&base).is_empty());
        assert_eq!(blockers(&Facts { running: true, ..base }).len(), 1);
        assert!(blockers(&Facts { jar_error: Some("loader 9 is not available"), ..base })[0].contains("loader 9"));
        assert_eq!(blockers(&Facts { from: "1.21", to: "1.20.4", ..base }).len(), 1);
        assert!(blockers(&Facts { from: "1.21", to: "1.20.4", confirm_downgrade: true, ..base }).is_empty());
        assert!(blockers(&Facts { to: "1.20.4", ..base })[0].contains("already"));
        assert!(blockers(&Facts { to: "1.20.4", to_loader: Some("b"), ..base }).is_empty());
    }

    #[test]
    fn warnings_mention_java_changes_and_missing_items() {
        let e = classify(&[l("gone", Some("modrinth"), Some("p"), Some("v"))], &HashMap::from([("gone".into(), Lookup::NoBuild)]));
        let w = warnings("1.20.4", "1.21", (17, 21), &e);
        assert!(w.iter().any(|x| x.contains("Java changes from 17 to 21")) && w.iter().any(|x| x.contains("1 item(s) have no build")));
        assert!(warnings("1.21", "1.21.1", (21, 21), &[]).is_empty());
    }
}
