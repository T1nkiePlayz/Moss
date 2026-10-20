//! Destination credentials. They are not stored in SQLite (so a copy or export of the database never contains them)
//! but in `<data>/backup-secrets.json`, created with mode 0600 and replaced atomically. The API only ever reports
//! whether a secret is set.
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io::Write, path::{Path, PathBuf}, sync::Mutex};

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Creds { pub access_key: String, pub secret_key: String }

static LOCK: Mutex<()> = Mutex::new(());

pub fn file(data_dir: &Path) -> PathBuf { data_dir.join("backup-secrets.json") }

fn read(data_dir: &Path) -> BTreeMap<String, Creds> {
    std::fs::read_to_string(file(data_dir)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn write(data_dir: &Path, m: &BTreeMap<String, Creds>) -> Result<()> {
    let (path, tmp) = (file(data_dir), data_dir.join(format!("backup-secrets.{}.tmp", uuid::Uuid::new_v4())));
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)] std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    let mut f = o.open(&tmp)?;
    let r = f.write_all(serde_json::to_string(m)?.as_bytes()).and_then(|_| f.sync_all());
    drop(f);
    if let Err(e) = r.and_then(|_| std::fs::rename(&tmp, &path)) { let _ = std::fs::remove_file(&tmp); return Err(e.into()); }
    Ok(())
}

pub fn get(data_dir: &Path, id: &str) -> Option<Creds> { let _g = LOCK.lock().unwrap_or_else(|p| p.into_inner()); read(data_dir).remove(id) }
pub fn is_set(data_dir: &Path, id: &str) -> bool { get(data_dir, id).is_some_and(|c| !c.access_key.is_empty() && !c.secret_key.is_empty()) }
pub fn set(data_dir: &Path, id: &str, c: Creds) -> Result<()> { let _g = LOCK.lock().unwrap_or_else(|p| p.into_inner()); let mut m = read(data_dir); m.insert(id.into(), c); write(data_dir, &m) }
pub fn remove(data_dir: &Path, id: &str) -> Result<()> { let _g = LOCK.lock().unwrap_or_else(|p| p.into_inner()); let mut m = read(data_dir); if m.remove(id).is_some() { write(data_dir, &m)?; } Ok(()) }
/// Every secret value, for redaction.
pub fn all_values(data_dir: &Path) -> Vec<String> {
    let _g = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    read(data_dir).into_values().flat_map(|c| [c.access_key, c.secret_key]).filter(|s| !s.is_empty()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stored_privately_and_removable() {
        let d = std::env::temp_dir().join(format!("moss-sec-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        assert!(!is_set(&d, "a"));
        set(&d, "a", Creds { access_key: "AK".into(), secret_key: "SK".into() }).unwrap();
        set(&d, "b", Creds { access_key: "AK2".into(), secret_key: "SK2".into() }).unwrap();
        assert!(is_set(&d, "a"));
        #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; assert_eq!(std::fs::metadata(file(&d)).unwrap().permissions().mode() & 0o777, 0o600); }
        assert_eq!(get(&d, "b").unwrap().secret_key, "SK2");
        assert_eq!(all_values(&d).len(), 4);
        remove(&d, "a").unwrap();
        assert!(!is_set(&d, "a") && is_set(&d, "b"));
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1, "no temp files left behind");
        let _ = std::fs::remove_dir_all(&d);
    }
}
