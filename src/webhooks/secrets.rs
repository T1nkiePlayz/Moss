//! Endpoint signing secrets. Kept out of SQLite (a copy of the database never contains them) in `<data>/webhook-secrets.json`,
//! created with mode 0600 and replaced atomically. The API shows a secret once, when it is generated.
use anyhow::Result;
use std::{collections::BTreeMap, io::Write, path::{Path, PathBuf}, sync::Mutex};

static LOCK: Mutex<()> = Mutex::new(());

pub fn file(data_dir: &Path) -> PathBuf { data_dir.join("webhook-secrets.json") }

fn read(data_dir: &Path) -> BTreeMap<String, String> {
    std::fs::read_to_string(file(data_dir)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn write(data_dir: &Path, m: &BTreeMap<String, String>) -> Result<()> {
    let (path, tmp) = (file(data_dir), data_dir.join(format!("webhook-secrets.{}.tmp", uuid::Uuid::new_v4())));
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)] std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    let mut f = o.open(&tmp)?;
    let r = f.write_all(serde_json::to_string(m)?.as_bytes()).and_then(|_| f.sync_all());
    drop(f);
    if let Err(e) = r.and_then(|_| std::fs::rename(&tmp, &path)) { let _ = std::fs::remove_file(&tmp); return Err(e.into()); }
    Ok(())
}

/// A fresh random secret from the OS RNG, e.g. `whsec_Qx…` (48 alphanumeric characters, about 285 bits).
pub fn generate() -> String { format!("whsec_{}", crate::auth::secret::random_string(48)) }

pub fn get(data_dir: &Path, id: &str) -> Option<String> { let _g = LOCK.lock().unwrap_or_else(|p| p.into_inner()); read(data_dir).remove(id) }
pub fn set(data_dir: &Path, id: &str, secret: &str) -> Result<()> { let _g = LOCK.lock().unwrap_or_else(|p| p.into_inner()); let mut m = read(data_dir); m.insert(id.into(), secret.into()); write(data_dir, &m) }
pub fn remove(data_dir: &Path, id: &str) -> Result<()> { let _g = LOCK.lock().unwrap_or_else(|p| p.into_inner()); let mut m = read(data_dir); if m.remove(id).is_some() { write(data_dir, &m)?; } Ok(()) }
/// Every secret, for redaction in the support bundle.
pub fn all_values(data_dir: &Path) -> Vec<String> { let _g = LOCK.lock().unwrap_or_else(|p| p.into_inner()); read(data_dir).into_values().filter(|s| !s.is_empty()).collect() }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stored_privately_and_removable() {
        let d = std::env::temp_dir().join(format!("moss-whsec-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        let (a, b) = (generate(), generate());
        assert!(a.starts_with("whsec_") && a.len() == 54 && a != b);
        set(&d, "a", &a).unwrap();
        set(&d, "b", &b).unwrap();
        #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; assert_eq!(std::fs::metadata(file(&d)).unwrap().permissions().mode() & 0o777, 0o600); }
        assert_eq!(get(&d, "a").as_deref(), Some(a.as_str()));
        assert_eq!(all_values(&d).len(), 2);
        remove(&d, "a").unwrap();
        assert!(get(&d, "a").is_none() && get(&d, "b").is_some());
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1, "no temp files left behind");
        let _ = std::fs::remove_dir_all(&d);
    }
}
