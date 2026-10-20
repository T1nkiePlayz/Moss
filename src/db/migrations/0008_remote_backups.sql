-- Remote backup destinations (folder or S3-compatible). Secrets never live here: they are kept in a 0600 file in the data directory.
CREATE TABLE IF NOT EXISTS backup_destinations (
  id TEXT PRIMARY KEY, name TEXT NOT NULL, kind TEXT NOT NULL CHECK (kind IN ('folder','s3')), config TEXT NOT NULL DEFAULT '{}',
  is_default INTEGER NOT NULL DEFAULT 0, keep_local INTEGER NOT NULL DEFAULT 0, keep_remote INTEGER NOT NULL DEFAULT 14, created_at TEXT NOT NULL);
-- Per-server override of the default destination. destination_id '' = this server is never copied remotely.
CREATE TABLE IF NOT EXISTS backup_dest_overrides (
  server_id TEXT PRIMARY KEY REFERENCES servers(id) ON DELETE CASCADE, destination_id TEXT NOT NULL);
ALTER TABLE backups ADD COLUMN remote TEXT NOT NULL DEFAULT 'none'; -- none | uploading | stored | failed
ALTER TABLE backups ADD COLUMN remote_dest TEXT;
ALTER TABLE backups ADD COLUMN remote_key TEXT;
ALTER TABLE backups ADD COLUMN remote_error TEXT;
ALTER TABLE backups ADD COLUMN remote_at TEXT;
ALTER TABLE backups ADD COLUMN local_present INTEGER NOT NULL DEFAULT 1;
-- Restores staged for comparison before promotion.
CREATE TABLE IF NOT EXISTS staged_restores (
  id TEXT PRIMARY KEY, backup_id TEXT NOT NULL, server_id TEXT NOT NULL, only TEXT NOT NULL DEFAULT '[]',
  status TEXT NOT NULL, error TEXT, created_at TEXT NOT NULL);
