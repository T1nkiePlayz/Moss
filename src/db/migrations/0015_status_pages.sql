-- Opt-in public status pages: a page anyone can open without signing in, showing only the servers the owner chose.
CREATE TABLE IF NOT EXISTS status_pages (
  id TEXT PRIMARY KEY, slug TEXT NOT NULL UNIQUE, title TEXT NOT NULL, description TEXT NOT NULL DEFAULT '',
  enabled INTEGER NOT NULL DEFAULT 1, show_players INTEGER NOT NULL DEFAULT 0,
  show_address TEXT,                      -- owner-entered public address, never the bind address
  created_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS status_page_servers (
  page_id TEXT NOT NULL REFERENCES status_pages(id) ON DELETE CASCADE,
  server_id TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
  position INTEGER NOT NULL DEFAULT 0, PRIMARY KEY (page_id, server_id));
