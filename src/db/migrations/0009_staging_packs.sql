-- Staged update copies (a staging copy is a servers row hidden from lists, tracked here) and modpack identity,
-- pinning and override checksums (used to diff a pack against a newer version without overwriting edited configs).
CREATE TABLE IF NOT EXISTS staging_runs (
  id TEXT PRIMARY KEY, origin_id TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE, kind TEXT NOT NULL, request TEXT NOT NULL DEFAULT '{}',
  include_world INTEGER NOT NULL DEFAULT 0, state TEXT NOT NULL, report TEXT NOT NULL DEFAULT '{}', port INTEGER, op_id TEXT,
  actor TEXT NOT NULL DEFAULT '', created_at TEXT NOT NULL, expires_at TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS staging_origin ON staging_runs(origin_id);
ALTER TABLE modpack_installs ADD COLUMN provider TEXT;
ALTER TABLE modpack_installs ADD COLUMN project_id TEXT;
ALTER TABLE modpack_installs ADD COLUMN version_id TEXT;
ALTER TABLE modpack_installs ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0;
CREATE TABLE IF NOT EXISTS modpack_files (
  install_id TEXT NOT NULL REFERENCES modpack_installs(id) ON DELETE CASCADE, path TEXT NOT NULL, sha256 TEXT NOT NULL, PRIMARY KEY (install_id, path));
