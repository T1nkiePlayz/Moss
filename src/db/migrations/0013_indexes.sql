-- Backup lists, pruning and the content inventory all filter by server.
CREATE INDEX IF NOT EXISTS backups_server_time ON backups(server_id, created_at DESC);
CREATE INDEX IF NOT EXISTS content_items_server ON content_items(server_id);
