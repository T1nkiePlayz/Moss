-- Modpack provenance (WP-A2). content_items stays untouched: the inventory reports source "modpack" for any item
-- listed in modpack_items.
CREATE TABLE IF NOT EXISTS modpack_installs (
  id TEXT PRIMARY KEY, server_id TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE, name TEXT NOT NULL, version TEXT NOT NULL DEFAULT '',
  format TEXT NOT NULL, game_version TEXT NOT NULL, loader TEXT NOT NULL DEFAULT '', mode TEXT NOT NULL DEFAULT 'add', installed_at TEXT NOT NULL, files INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS modpack_items (
  content_item_id TEXT PRIMARY KEY REFERENCES content_items(id) ON DELETE CASCADE, install_id TEXT NOT NULL REFERENCES modpack_installs(id) ON DELETE CASCADE);
CREATE INDEX IF NOT EXISTS modpack_installs_server ON modpack_installs(server_id);
