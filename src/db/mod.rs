//! The SQLite handle: one WAL writer behind a mutex, a read-only connection and a background checkpointer, history
//! pruning, and the timestamp helper.
//! Schema creation and upgrades live in `migrations`.
pub(crate) mod migrations;

/// Applies the schema to an in-memory connection (tests of other modules).
#[cfg(test)]
pub fn run_migrations_for_tests(c: &Connection) -> rusqlite::Result<()> { migrations::run(c) }

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::{path::Path, sync::{Arc, Mutex}, time::Duration};

/// One SQLite writer connection (WAL) behind a mutex, used via `spawn_blocking`, plus a read-only connection for hot
/// lookups (`read`). Moss keeps high-rate data (console, metrics) out of the database, so a single writer is plenty.
/// WAL checkpoints run on a third connection in a background thread (`checkpointer`), never on the writer.
#[derive(Clone)]
pub struct Db { w: Arc<Mutex<Connection>>, r: Arc<Mutex<Connection>> }

/// How often the checkpointer copies committed WAL frames into the database file.
const CHECKPOINT_EVERY: Duration = Duration::from_secs(1);

/// SQLite's automatic checkpoint runs inside the commit that crosses 1000 WAL pages, on the committing connection, and
/// fsyncs the WAL and the database file. On the shared writer, under its mutex, that fsync took over 10 s on a busy
/// rotational disk and every database call (each console command's audit entry among them) waited behind it. A
/// passive checkpoint on its own connection never holds the writer's mutex and does nothing when nothing is new.
/// btrfs still blocks WAL appends for the length of the WAL fsync (it holds the inode lock), which is why lookups
/// use the reader and console-command audit entries are queued (`state::AuditQueue`). Ends once the `Db` is dropped.
fn checkpointer(path: &Path, db: std::sync::Weak<Mutex<Connection>>) -> Result<()> {
    let c = Connection::open(path).with_context(|| format!("opening database {}", path.display()))?;
    c.execute_batch("PRAGMA synchronous=NORMAL; PRAGMA busy_timeout=5000; PRAGMA wal_autocheckpoint=0;")?;
    std::thread::Builder::new().name("moss-db-checkpoint".into()).spawn(move || {
        loop {
            std::thread::sleep(CHECKPOINT_EVERY);
            if db.strong_count() == 0 { break; }
            if let Err(e) = c.query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |_| Ok(())) { tracing::debug!("wal checkpoint: {e}"); }
        }
    })?;
    Ok(())
}

async fn run<T, F>(conn: &Arc<Mutex<Connection>>, f: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
{
    let conn = conn.clone();
    tokio::task::spawn_blocking(move || {
        let c = conn.lock().unwrap_or_else(|p| p.into_inner());
        f(&c).map_err(anyhow::Error::from)
    })
    .await?
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let c = Connection::open(path).with_context(|| format!("opening database {}", path.display()))?;
        // WAL + synchronous=NORMAL: no corruption on power loss and nothing lost on a crash, only the last commits on a
        // power cut, without an fsync on every commit. Temp tables and sorts stay in memory.
        c.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000; PRAGMA temp_store=MEMORY; PRAGMA wal_autocheckpoint=0;")?;
        migrations::run(&c)?;
        let r = Connection::open(path).with_context(|| format!("opening database {}", path.display()))?;
        r.execute_batch("PRAGMA query_only=ON; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000; PRAGMA temp_store=MEMORY; PRAGMA wal_autocheckpoint=0;")?;
        let w = Arc::new(Mutex::new(c));
        checkpointer(path, Arc::downgrade(&w))?;
        Ok(Self { w, r: Arc::new(Mutex::new(r)) })
    }

    /// Run `f` on the writer. Everything that writes, and anything that must see its own transaction, goes here.
    pub async fn call<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
    {
        run(&self.w, f).await
    }

    /// Run a read-only `f` on the reader: it sees every committed write and never waits for the writer (WAL readers
    /// run alongside it), so a slow commit cannot hold up lookups on hot paths such as every API request's server.
    pub async fn read<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
    {
        run(&self.r, f).await
    }
}

/// Bound the history tables so a long-lived instance does not grow its database forever.
pub fn prune_history(c: &Connection) -> rusqlite::Result<()> {
    c.execute_batch(
        "DELETE FROM operations WHERE state NOT IN ('queued','running') AND id NOT IN (SELECT id FROM operations ORDER BY created_at DESC LIMIT 1000);
         DELETE FROM activity WHERE id <= (SELECT COALESCE(MAX(id),0) - 20000 FROM activity);
         DELETE FROM timeline WHERE id <= (SELECT COALESCE(MAX(id),0) - 20000 FROM timeline);",
    )?;
    crate::history::prune(c)
}

/// Open the database without migrating or creating it (offline recovery tooling).
pub fn open_existing(path: &Path) -> Result<Connection> {
    if !path.is_file() { anyhow::bail!("no database at {}", path.display()); }
    let c = Connection::open(path).with_context(|| format!("opening database {}", path.display()))?;
    c.execute_batch("PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;")?;
    Ok(c)
}
pub use migrations::latest as latest_migration;

pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Run all migrations on a scratch connection (unit tests of other modules).
#[cfg(test)]
pub fn migrate_for_tests(c: &Connection) { migrations::run(c).unwrap() }

#[cfg(test)]
mod tests {
    use super::*;

    /// Commits on the shared connection never checkpoint (that fsync once stalled every request behind it); the
    /// background connection copies the WAL into the database file instead.
    #[test]
    fn checkpoints_happen_off_the_shared_connection() {
        let dir = std::env::temp_dir().join(format!("moss-db-ckpt-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("moss.sqlite");
        let db = Db::open(&path).unwrap();
        let shared = db.w.lock().unwrap();
        assert_eq!(shared.query_row("PRAGMA wal_autocheckpoint", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
        let before = std::fs::metadata(&path).unwrap().len();
        // well over SQLite's 1000-page automatic checkpoint threshold
        shared.execute_batch("CREATE TABLE ckpt(b BLOB); WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i < 2000) INSERT INTO ckpt SELECT zeroblob(4096) FROM n;").unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), before, "the committing connection checkpointed");
        drop(shared);
        let start = std::time::Instant::now();
        while std::fs::metadata(&path).unwrap().len() < 2000 * 4096 {
            assert!(start.elapsed() < Duration::from_secs(10), "the background checkpointer never ran");
            std::thread::sleep(Duration::from_millis(50));
        }
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
