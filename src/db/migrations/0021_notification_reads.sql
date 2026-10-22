-- Read state per user: marking notifications read (or "Mark all read") no longer clears them for every other admin.
-- No foreign key to users (the dev-mode user has no row); a trigger drops a deleted user's rows instead.
CREATE TABLE IF NOT EXISTS notification_reads (
  user_id TEXT NOT NULL,
  notification_id TEXT NOT NULL REFERENCES notifications(id) ON DELETE CASCADE,
  PRIMARY KEY (user_id, notification_id)) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS notif_reads_by_notification ON notification_reads(notification_id);
CREATE TRIGGER IF NOT EXISTS notif_reads_user_gone AFTER DELETE ON users BEGIN
  DELETE FROM notification_reads WHERE user_id = OLD.id;
END;
-- What was read under the old shared flag stays read for everyone who could see it then.
INSERT OR IGNORE INTO notification_reads(user_id, notification_id) SELECT u.id, n.id FROM notifications n, users u WHERE n.read = 1;
