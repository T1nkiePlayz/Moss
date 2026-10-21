-- Player analytics. Times are UTC epoch seconds. Sessions come from player-name diffs in the status ping (only when names
-- are known); counts are per-server hourly buckets (peak, and sum/samples for the average). Both go with the server.
CREATE TABLE IF NOT EXISTS player_sessions (
  id INTEGER PRIMARY KEY AUTOINCREMENT, server_id TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
  player TEXT NOT NULL, joined_at INTEGER NOT NULL, left_at INTEGER, last_seen INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS player_sessions_server ON player_sessions(server_id, joined_at);
CREATE INDEX IF NOT EXISTS player_sessions_open ON player_sessions(server_id) WHERE left_at IS NULL;
CREATE TABLE IF NOT EXISTS player_counts (
  server_id TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE, hour INTEGER NOT NULL,
  peak INTEGER NOT NULL, samples INTEGER NOT NULL, sum INTEGER NOT NULL, PRIMARY KEY (server_id, hour)) WITHOUT ROWID;
