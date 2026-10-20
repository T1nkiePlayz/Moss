-- WP-C: schedule time zones, backup locks/worlds, notifications, long-term metrics.
ALTER TABLE schedules ADD COLUMN tz TEXT NOT NULL DEFAULT 'UTC';
ALTER TABLE backups ADD COLUMN locked INTEGER NOT NULL DEFAULT 0;
ALTER TABLE backups ADD COLUMN worlds TEXT NOT NULL DEFAULT '[]';
CREATE TABLE IF NOT EXISTS notifications (
  id TEXT PRIMARY KEY, time TEXT NOT NULL, kind TEXT NOT NULL, severity TEXT NOT NULL, title TEXT NOT NULL,
  body TEXT NOT NULL DEFAULT '', server_id TEXT, read INTEGER NOT NULL DEFAULT 0);
CREATE INDEX IF NOT EXISTS notif_time ON notifications(time DESC);
CREATE INDEX IF NOT EXISTS notif_dedupe ON notifications(kind, server_id, time);
CREATE TABLE IF NOT EXISTS metrics_hourly (
  server_id TEXT NOT NULL, hour INTEGER NOT NULL, n INTEGER NOT NULL, cpu_avg REAL NOT NULL, cpu_max REAL NOT NULL,
  mem_avg REAL NOT NULL, mem_max INTEGER NOT NULL, players_max INTEGER NOT NULL DEFAULT 0, PRIMARY KEY (server_id, hour));
CREATE TABLE IF NOT EXISTS host_hourly (
  hour INTEGER PRIMARY KEY, n INTEGER NOT NULL, cpu_avg REAL NOT NULL, cpu_max REAL NOT NULL,
  mem_avg REAL NOT NULL, mem_max INTEGER NOT NULL, disk_free INTEGER NOT NULL DEFAULT 0);
UPDATE schedules SET tz = printf('UTC%s%02d:%02d', CASE WHEN tz_offset_min < 0 THEN '-' ELSE '+' END, abs(tz_offset_min) / 60, abs(tz_offset_min) % 60) WHERE tz_offset_min <> 0;
