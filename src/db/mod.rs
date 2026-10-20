//! The SQLite handle: one WAL connection behind a mutex, history pruning, and the timestamp helper.
//! Schema creation and upgrades live in `migrations`.
mod migrations;

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::{path::Path, sync::{Arc, Mutex}};

/// One SQLite connection (WAL) behind a mutex, used via `spawn_blocking`.
/// Moss keeps high-rate data (console, metrics) out of the database, so a single writer is plenty.
#[derive(Clone)]
pub struct Db(Arc<Mutex<Connection>>);


impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let c = Connection::open(path).with_context(|| format!("opening database {}", path.display()))?;
        c.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;")?;
        migrations::run(&c)?;
        Ok(Self(Arc::new(Mutex::new(c))))
    }
    pub async fn call<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
    {
        let db = self.0.clone();
        tokio::task::spawn_blocking(move || {
            let c = db.lock().unwrap_or_else(|p| p.into_inner());
            f(&c).map_err(anyhow::Error::from)
        })
        .await?
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

pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Run all migrations on a scratch connection (unit tests of other modules).
#[cfg(test)]
pub fn migrate_for_tests(c: &Connection) { migrations::run(c).unwrap() }
