//! Content-change journal tests: update, disable and full rollback.
use super::*;
use super::classify::Linked;

fn tmpdir() -> PathBuf {
    let d = std::env::temp_dir().join(format!("moss-upg-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn entry(file: &str, status: &'static str) -> Entry {
    let link = Linked { item: file.into(), title: file.into(), provider: Some("modrinth".into()), project_id: Some("p".into()), version_id: Some("v1".into()), current: None, filename: file.into(), pinned: false };
    Entry { link, status, target: Some("v2".into()), version_id: Some("v2".into()), reason: None }
}

fn prepared(dir: &std::path::Path, old: &str, new: &str, body: &str) -> Prepared {
    let path = dir.join(format!("dl-{new}"));
    std::fs::write(&path, body).unwrap();
    Prepared { entry: entry(old, "update"), filename: new.into(), path, label: "v2".into(), sha512: None }
}

#[test]
fn update_replaces_files_and_rollback_restores_them() {
    let (dir, tmp) = (tmpdir(), tmpdir());
    let aside = tmp.join("aside");
    std::fs::write(dir.join("a-1.jar"), "old").unwrap();
    std::fs::write(dir.join("b.jar"), "keep").unwrap();
    let p = [prepared(&tmp, "a-1.jar", "a-2.jar", "new")];
    let mut j = Journal::default();
    apply_content(&dir, &aside, "update", &[entry("a-1.jar", "update")], &p, &mut j).unwrap();
    assert!(!dir.join("a-1.jar").exists());
    assert_eq!(std::fs::read_to_string(dir.join("a-2.jar")).unwrap(), "new");
    j.rollback();
    assert_eq!(std::fs::read_to_string(dir.join("a-1.jar")).unwrap(), "old");
    assert!(!dir.join("a-2.jar").exists());
    assert_eq!(std::fs::read_to_string(dir.join("b.jar")).unwrap(), "keep");
    for d in [dir, tmp] { let _ = std::fs::remove_dir_all(d); }
}

#[test]
fn disable_incompatible_renames_only_missing_items_and_rolls_back() {
    let (dir, tmp) = (tmpdir(), tmpdir());
    for f in ["gone.jar", "fine.jar", "newer.jar"] { std::fs::write(dir.join(f), f).unwrap(); }
    let entries = [entry("gone.jar", "missing"), entry("fine.jar", "ok"), entry("newer.jar", "update")];
    let mut j = Journal::default();
    apply_content(&dir, &tmp.join("aside"), "disable_incompatible", &entries, &[], &mut j).unwrap();
    assert!(dir.join("gone.jar.disabled").exists() && !dir.join("gone.jar").exists());
    assert!(dir.join("fine.jar").exists() && dir.join("newer.jar").exists());
    j.rollback();
    assert!(dir.join("gone.jar").exists() && !dir.join("gone.jar.disabled").exists());
    for d in [dir, tmp] { let _ = std::fs::remove_dir_all(d); }
}

#[test]
fn keep_changes_nothing() {
    let (dir, tmp) = (tmpdir(), tmpdir());
    std::fs::write(dir.join("a.jar"), "a").unwrap();
    let mut j = Journal::default();
    apply_content(&dir, &tmp.join("aside"), "keep", &[entry("a.jar", "missing")], &[], &mut j).unwrap();
    assert!(dir.join("a.jar").exists() && j.renamed.is_empty() && j.created.is_empty());
    for d in [dir, tmp] { let _ = std::fs::remove_dir_all(d); }
}
