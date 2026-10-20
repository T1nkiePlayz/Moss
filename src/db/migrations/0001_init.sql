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
