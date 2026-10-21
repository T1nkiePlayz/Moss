-- Shared player lists: named allowlists and ban lists, managed once and applied to every subscribed server.
CREATE TABLE IF NOT EXISTS player_lists (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  kind TEXT NOT NULL CHECK (kind IN ('allow','ban')),
  created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS player_list_entries (
  list_id TEXT NOT NULL REFERENCES player_lists(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  uuid TEXT,
  reason TEXT NOT NULL DEFAULT '',
  added_by TEXT NOT NULL DEFAULT '',
  added_at TEXT NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS player_list_entries_name ON player_list_entries(list_id, lower(name));
CREATE TABLE IF NOT EXISTS player_list_subs (
  list_id TEXT NOT NULL REFERENCES player_lists(id) ON DELETE CASCADE,
  server_id TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
  PRIMARY KEY (list_id, server_id)
);
-- Names Moss has put on a server (the diff base, and how Moss tells its own entries from ones an operator added by hand).
CREATE TABLE IF NOT EXISTS player_list_applied (
  server_id TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
  kind TEXT NOT NULL CHECK (kind IN ('allow','ban')),
  key TEXT NOT NULL,
  name TEXT NOT NULL,
  PRIMARY KEY (server_id, kind, key)
);
