//! In-memory store for staged modpacks: a token names a temp file under `tmp/` for 30 minutes, bound to the user
//! who staged it. Expired entries are swept (and their files deleted) on every access.
use crate::state::St;
use std::{collections::HashMap, path::PathBuf, sync::Mutex, time::{Duration, Instant}};

pub const TTL: Duration = Duration::from_secs(30 * 60);
const MAX_PER_USER: usize = 6;

#[derive(Clone, Debug)]
pub struct Entry {
    pub token: String,
    pub path: PathBuf,
    pub user: String,
    pub created: Instant,
    pub runtime: String,
    pub game_version: String,
    pub loader_version: Option<String>,
    pub name: String,
}

static STORE: Mutex<Option<HashMap<String, Entry>>> = Mutex::new(None);

/// Drop entries older than `ttl`, returning the files that should be deleted.
pub fn sweep_map(m: &mut HashMap<String, Entry>, ttl: Duration) -> Vec<PathBuf> {
    let dead: Vec<String> = m.iter().filter(|(_, e)| e.created.elapsed() >= ttl).map(|(k, _)| k.clone()).collect();
    dead.into_iter().filter_map(|k| m.remove(&k)).map(|e| e.path).collect()
}

fn with<T>(f: impl FnOnce(&mut HashMap<String, Entry>) -> T) -> T {
    let (out, dead) = {
        let mut g = STORE.lock().unwrap();
        let m = g.get_or_insert_with(HashMap::new);
        let dead = sweep_map(m, TTL);
        (f(m), dead)
    };
    for p in dead { let _ = std::fs::remove_file(p); }
    out
}

/// Remove temp files left behind by a previous run (older than the token lifetime). Called when the first pack is staged.
fn sweep_stale_files(st: &St) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        if let Ok(rd) = std::fs::read_dir(st.cfg.tmp_dir()) {
            for e in rd.flatten().filter(|e| e.file_name().to_string_lossy().starts_with("modpack-")) {
                if e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|a| a >= TTL) { let _ = std::fs::remove_file(e.path()); }
            }
        }
    });
}

/// Store an entry and return its token. A user's oldest staged packs are dropped beyond a small cap.
pub fn put(st: &St, mut e: Entry) -> String {
    sweep_stale_files(st);
    let token = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
    let t2 = token.clone();
    e.token = token.clone();
    let dropped: Vec<PathBuf> = with(move |m| {
        let mut mine: Vec<(String, Instant)> = m.iter().filter(|(_, x)| x.user == e.user).map(|(k, x)| (k.clone(), x.created)).collect();
        mine.sort_by_key(|(_, t)| *t);
        let extra = (mine.len() + 1).saturating_sub(MAX_PER_USER);
        let dropped = mine.into_iter().take(extra).filter_map(|(k, _)| m.remove(&k)).map(|x| x.path).collect();
        m.insert(t2, e);
        dropped
    });
    for p in dropped { let _ = std::fs::remove_file(p); }
    token
}

/// The entry for a token, only for the user who staged it.
pub fn get(token: &str, user_id: &str) -> Option<Entry> {
    with(|m| m.get(token).filter(|e| e.user == user_id).cloned())
}

/// Forget a token and delete its file (after a successful import).
pub fn consume(token: &str) {
    if let Some(e) = with(|m| m.remove(token)) { let _ = std::fs::remove_file(e.path); }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(user: &str, age: Duration, path: &str) -> Entry {
        Entry { token: String::new(), path: path.into(), user: user.into(), created: Instant::now() - age, runtime: "fabric".into(), game_version: "1.21".into(), loader_version: None, name: "n".into() }
    }

    #[test]
    fn sweep_removes_only_expired_entries_and_reports_their_files() {
        let mut m = HashMap::from([("old".to_string(), entry("u", TTL + Duration::from_secs(1), "/tmp/old")), ("new".to_string(), entry("u", Duration::from_secs(5), "/tmp/new"))]);
        assert_eq!(sweep_map(&mut m, TTL), [PathBuf::from("/tmp/old")]);
        assert!(m.contains_key("new") && !m.contains_key("old"));
    }

    #[test]
    fn tokens_are_bound_to_their_user_and_expire() {
        let dir = std::env::temp_dir().join(format!("moss-store-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        let (live, stale) = (dir.join("live.pack"), dir.join("stale.pack"));
        std::fs::write(&live, "x").unwrap();
        std::fs::write(&stale, "x").unwrap();
        let (a, b) = ("test-user-a", "test-user-b");
        with(|m| { m.insert("tok-live".into(), entry(a, Duration::from_secs(1), live.to_str().unwrap())); m.insert("tok-stale".into(), entry(a, TTL + Duration::from_secs(5), stale.to_str().unwrap())); });
        assert!(get("tok-live", a).is_some());
        assert!(get("tok-live", b).is_none(), "another user must not use the token");
        assert!(get("tok-stale", a).is_none());
        assert!(!stale.exists(), "expired temp files are deleted");
        consume("tok-live");
        assert!(get("tok-live", a).is_none() && !live.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
