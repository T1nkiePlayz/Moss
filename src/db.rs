use anyhow::{Context, Result};
use rusqlite::Connection;
use std::{path::Path, sync::{Arc, Mutex}};

/// One SQLite connection (WAL) behind a mutex, used via `spawn_blocking`.
/// Moss keeps high-rate data (console, metrics) out of the database, so a single writer is plenty.
#[derive(Clone)]
pub struct Db(Arc<Mutex<Connection>>);

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta (k TEXT PRIMARY KEY, v TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS users (
  id TEXT PRIMARY KEY, email TEXT NOT NULL UNIQUE COLLATE NOCASE, role TEXT NOT NULL, linked INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS servers (
  id TEXT PRIMARY KEY, node_id TEXT NOT NULL DEFAULT 'local', name TEXT NOT NULL, runtime TEXT NOT NULL,
  game_version TEXT NOT NULL, loader_version TEXT, java_major INTEGER NOT NULL DEFAULT 21, port INTEGER NOT NULL,
  memory_mb INTEGER NOT NULL, flags_profile TEXT NOT NULL DEFAULT 'balanced', autostart INTEGER NOT NULL DEFAULT 0,
  restart_enabled INTEGER NOT NULL DEFAULT 1, restart_max INTEGER NOT NULL DEFAULT 3,
  eula_at TEXT, eula_by TEXT, desired TEXT NOT NULL DEFAULT 'stopped', installed INTEGER NOT NULL DEFAULT 0,
  install_error TEXT, pid INTEGER, pid_start INTEGER, started_at TEXT, last_exit TEXT,
  tags TEXT NOT NULL DEFAULT '[]', notes TEXT NOT NULL DEFAULT '', custom_cmd TEXT, created_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS grants (
  server_id TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE, user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  bundle TEXT NOT NULL, PRIMARY KEY (server_id, user_id));
CREATE TABLE IF NOT EXISTS operations (
  id TEXT PRIMARY KEY, kind TEXT NOT NULL, title TEXT NOT NULL, server_id TEXT, state TEXT NOT NULL, stage TEXT NOT NULL DEFAULT '',
  progress REAL, log TEXT NOT NULL DEFAULT '[]', error TEXT, cancellable INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL, started_at TEXT, ended_at TEXT);
CREATE INDEX IF NOT EXISTS ops_created ON operations(created_at DESC);
CREATE TABLE IF NOT EXISTS activity (
  id INTEGER PRIMARY KEY AUTOINCREMENT, time TEXT NOT NULL, actor TEXT NOT NULL, action TEXT NOT NULL, target TEXT NOT NULL, detail TEXT NOT NULL DEFAULT '', server_id TEXT);
CREATE INDEX IF NOT EXISTS act_server ON activity(server_id, id DESC);
CREATE TABLE IF NOT EXISTS timeline (
  id INTEGER PRIMARY KEY AUTOINCREMENT, server_id TEXT NOT NULL, time TEXT NOT NULL, kind TEXT NOT NULL, detail TEXT NOT NULL DEFAULT '');
CREATE INDEX IF NOT EXISTS tl_server ON timeline(server_id, id DESC);
CREATE TABLE IF NOT EXISTS content_items (
  id TEXT PRIMARY KEY, server_id TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE, provider TEXT NOT NULL,
  project_id TEXT, version_id TEXT, title TEXT NOT NULL, version_number TEXT, filename TEXT NOT NULL, kind TEXT NOT NULL,
  sha512 TEXT, installed_at TEXT NOT NULL, pinned INTEGER NOT NULL DEFAULT 0, dir TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS backups (
  id TEXT PRIMARY KEY, server_id TEXT NOT NULL, created_at TEXT NOT NULL, size INTEGER NOT NULL DEFAULT 0, sha256 TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL, verified_at TEXT, note TEXT NOT NULL DEFAULT '', kind TEXT NOT NULL, consistent INTEGER NOT NULL DEFAULT 0,
  files INTEGER NOT NULL DEFAULT 0, server_name TEXT NOT NULL DEFAULT '', runtime TEXT NOT NULL DEFAULT '', game_version TEXT NOT NULL DEFAULT '');
CREATE TABLE IF NOT EXISTS schedules (
  id TEXT PRIMARY KEY, server_id TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE, kind TEXT NOT NULL, command TEXT,
  every_minutes INTEGER, daily_at TEXT, tz_offset_min INTEGER NOT NULL DEFAULT 0, enabled INTEGER NOT NULL DEFAULT 1, keep INTEGER,
  last_run TEXT, next_run TEXT, last_status TEXT);
CREATE TABLE IF NOT EXISTS undo (
  id TEXT PRIMARY KEY, server_id TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE, title TEXT NOT NULL, created_at TEXT NOT NULL, files TEXT NOT NULL);
"#;

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let c = Connection::open(path).with_context(|| format!("opening database {}", path.display()))?;
        c.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;")?;
        c.execute_batch(SCHEMA)?;
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
    )
}

pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
