-- Per-user browser preferences (theme editor sync), saved list views, and server blueprints (WP ops-misc).
CREATE TABLE IF NOT EXISTS user_prefs (user_id TEXT PRIMARY KEY, data TEXT NOT NULL, updated_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS saved_views (
  id TEXT PRIMARY KEY, user_id TEXT NOT NULL, scope TEXT NOT NULL, name TEXT NOT NULL, filters TEXT NOT NULL DEFAULT '{}',
  sort TEXT NOT NULL DEFAULT '', created_at TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS saved_views_user ON saved_views(user_id, scope);
CREATE TABLE IF NOT EXISTS blueprints (
  id TEXT PRIMARY KEY, name TEXT NOT NULL, data TEXT NOT NULL, created_by TEXT NOT NULL, created_at TEXT NOT NULL);
